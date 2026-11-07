//! Eingebaute Funktionsobjekte (Abschnitt 10.3).
//!
//! Eine eingebaute Funktion ist ein Objekt der Art [`ObjectClass::Builtin`]
//! mit drei internen Feldern vor seinen Eigenschaften: der Nummer ihres
//! Eintrags im Agent, der Nummer ihres Realms ([[Realm]]) und dem, was sie
//! einfängt. Legt die Spezifikation eine Funktion mit CreateBuiltinFunction
//! aus einem Abstract Closure an, etwa die Funktionen `resolve` und
//! `reject` eines Promise, ist das ein Eintrag ohne Namen, und die Werte,
//! die das Closure einfängt, kommen in das dritte Feld
//! ([`create_builtin_closure`]); sonst ist es *undefined*. Ein Eintrag
//! ([`Builtin`]) nennt Name, `length`, ob es [[Construct]] gibt, und die
//! Rust-Funktion mit dem Verhalten. Die Einträge stehen in festen Tabellen
//! der Engine und des Hosts; der Agent nummeriert sie, sobald das erste
//! Funktionsobjekt zu einem Eintrag entsteht. So lassen sich die Tabellen
//! gegen die Eigenschaften in der Spezifikation prüfen.
//!
//! Die Rust-Funktion bekommt den Agent, `this`, die Argumente und
//! new.target (BuiltinCallOrConstruct, Abschnitt 10.3.3). Die Argumente sind
//! Handles und damit Wurzeln, auch wenn die Funktion Speicher anfordert.
//! [[Realm]] der Funktion ist während des Aufrufs der aktuelle Realm: Dort
//! entstehen neue Objekte, und ein Fehler der Engine aus dem Aufruf bekommt
//! ihn. Funktionen mit [[Async]] gibt es noch nicht.
//!
//! Ruft JavaScript eine eingebaute Funktion auf, läuft sie im Befehl des
//! Aufrufs, ohne Rahmen. Ruft Rust-Code sie auf, zählt das wie ein neuer
//! Lauf des Interpreters ([`crate::interpreter::call_function`]): Er kostet
//! einen Schritt und darf den Rust-Stapel nur bis
//! [`crate::agent::STACK_BUDGET`] belegen (RS-8).

#![forbid(unsafe_code)]

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::descriptor::PropertyDescriptor;
use crate::function::{set_function_length, set_function_name};
use crate::heap::{Heap, Local, ObjectClass, OutOfMemory, Scope};
use crate::object::INLINE;
use crate::operations::define_property_or_throw;
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Das interne Feld mit der Nummer des Eintrags.
const BEHAVIOUR: usize = 0;
/// Das interne Feld mit der Nummer des Realms.
const REALM: usize = 1;
/// Das interne Feld mit dem, was die Funktion einfängt.
const CAPTURES: usize = 2;

/// Das Verhalten einer eingebauten Funktion. Das Ergebnis liegt im Bereich
/// `scope`; für [[Construct]] ist es immer ein Objekt.
pub type Behaviour =
    for<'s> fn(&mut Agent, &'s Scope, &BuiltinCall<'_, '_>) -> Completion<Local<'s>>;

/// Ein Eintrag in der Tabelle der eingebauten Funktionen.
pub struct Builtin {
    /// Der Wert von `name` (SetFunctionName ohne Präfix).
    pub name: &'static str,
    /// Der Wert von `length` (SetFunctionLength).
    pub length: u32,
    /// Ob die Funktion [[Construct]] hat.
    pub constructor: bool,
    pub behaviour: Behaviour,
}

/// Was eine eingebaute Funktion bei ihrem Aufruf bekommt.
pub struct BuiltinCall<'a, 'l> {
    /// Die Funktion selbst, das *active function object*.
    pub function: Local<'l>,
    /// Ihr [[Realm]], der aktuelle Realm des Aufrufs.
    pub realm: RealmId,
    /// Der Wert von `this`; bei [[Construct]] nicht initialisiert und
    /// *undefined*.
    pub this: Local<'l>,
    pub arguments: &'a [Local<'l>],
    /// NewTarget, `None` für [[Call]].
    pub new_target: Option<Local<'l>>,
}

impl BuiltinCall<'_, '_> {
    /// Das Argument `index`, *undefined*, wenn es fehlt.
    pub fn argument<'s>(&self, agent: &Agent, scope: &'s Scope, index: usize) -> Local<'s> {
        let heap = agent.heap();
        let value = self
            .arguments
            .get(index)
            .map_or(Value::UNDEFINED, |&argument| heap.get(argument));
        heap.local(scope, value)
    }

    /// Was die Funktion einfängt ([`create_builtin_closure`]), sonst
    /// *undefined*.
    pub(crate) fn captures<'s>(&self, agent: &Agent, scope: &'s Scope) -> Local<'s> {
        let heap = agent.heap();
        let value = match heap.get(self.function).kind() {
            Kind::Object(object) => heap
                .builtin_field(object, CAPTURES)
                .unwrap_or(Value::UNDEFINED),
            _ => Value::UNDEFINED,
        };
        heap.local(scope, value)
    }
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl Agent {
    /// Die Nummer des Eintrags `builtin`, beim ersten Mal neu vergeben.
    fn builtin_number(&mut self, builtin: &'static Builtin) -> Result<i32, OutOfMemory> {
        let address = std::ptr::from_ref(builtin).addr();
        if let Some(&number) = self.builtin_numbers.get(&address) {
            return Ok(number);
        }
        let number = i32::try_from(self.builtins.len()).map_err(|_| OutOfMemory)?;
        self.builtins.push(builtin);
        self.builtin_numbers.insert(address, number);
        Ok(number)
    }
}

/// Die Nummer des Eintrags und der Realm der eingebauten Funktion `value`,
/// `None` für andere Werte.
pub(crate) fn builtin_parts(heap: &Heap, value: Value<'_>) -> Option<(usize, RealmId)> {
    let Kind::Object(object) = value.kind() else {
        return None;
    };
    let number = heap.builtin_field(object, BEHAVIOUR)?.as_int32()?;
    let realm = heap.builtin_field(object, REALM)?.as_int32()?;
    Some((
        usize::try_from(number).ok()?,
        RealmId::from_index(usize::try_from(realm).ok()?),
    ))
}

/// Der Eintrag der eingebauten Funktion `value`.
pub(crate) fn builtin_entry(agent: &Agent, value: Value<'_>) -> Option<&'static Builtin> {
    let (number, _) = builtin_parts(agent.heap(), value)?;
    agent.builtins.get(number).copied()
}

/// CreateBuiltinFunction (Abschnitt 10.3.4) mit %Function.prototype% aus
/// `realm` als [[Prototype]] und ohne Präfix.
pub fn create_builtin_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    builtin: &'static Builtin,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
    builtin_function_with_prototype(agent, scope, realm, builtin, prototype)
}

/// CreateBuiltinFunction (Abschnitt 10.3.4) für ein Abstract Closure, das
/// `captures` einfängt, sonst wie [`create_builtin_function`]. Meist ist
/// `captures` eine interne Zelle, die mehrere solche Funktionen teilen
/// können, wie den Record aus CreateResolvingFunctions (Abschnitt
/// 27.5.1.3).
pub(crate) fn create_builtin_closure<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    builtin: &'static Builtin,
    captures: Local<'_>,
) -> Completion<Local<'s>> {
    let function = create_builtin_function(agent, scope, realm, builtin)?;
    let heap = agent.heap();
    if let Kind::Object(object) = heap.get(function).kind() {
        heap.set_builtin_field(object, CAPTURES, heap.get(captures));
    }
    Ok(function)
}

/// CreateBuiltinFunction mit dem Prototyp `prototype`.
pub(crate) fn builtin_function_with_prototype<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    builtin: &'static Builtin,
    prototype: Local<'_>,
) -> Completion<Local<'s>> {
    let number = agent.builtin_number(builtin)?;
    let realm_number = i32::try_from(realm.index()).map_err(|_| OutOfMemory)?;
    let shape = agent.empty_shape(scope);
    let function = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::Builtin,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    if let Kind::Object(object) = heap.get(function).kind() {
        heap.set_builtin_field(object, BEHAVIOUR, Value::int32(number));
        heap.set_builtin_field(object, REALM, Value::int32(realm_number));
    }
    set_function_length(agent, scope, function, builtin.length)?;
    let name = agent.key_from_units(scope, &units(builtin.name))?;
    set_function_name(agent, scope, function, name, None)?;
    Ok(function)
}

/// Legt eine eingebaute Funktion an und gibt sie `object` als Eigenschaft
/// mit ihrem Namen: beschreibbar, konfigurierbar und nicht aufzählbar wie
/// die Funktionen der Abschnitte 19 bis 28 (Abschnitt 18).
pub fn define_builtin_function(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    object: Local<'_>,
    builtin: &'static Builtin,
) -> Completion<()> {
    let function = create_builtin_function(agent, scope, realm, builtin)?;
    let key = agent.key_from_units(scope, &units(builtin.name))?;
    let descriptor = PropertyDescriptor::data(function, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// [[Call]] und [[Construct]] einer eingebauten Funktion (Abschnitte
/// 10.3.1 und 10.3.2) mit BuiltinCallOrConstruct (Abschnitt 10.3.3). Ein
/// Fehler der Engine aus dem Aufruf gehört zu ihrem Realm.
pub(crate) fn call_builtin<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    function: Local<'_>,
    this: Local<'_>,
    arguments: &[Local<'_>],
    new_target: Option<Local<'_>>,
) -> Completion<Local<'s>> {
    let found = builtin_parts(agent.heap(), agent.heap().get(function))
        .and_then(|(number, realm)| Some((*agent.builtins.get(number)?, realm)));
    // Nur Aufrufer, die eine eingebaute Funktion vor sich haben, kommen
    // hierher.
    let Some((builtin, realm)) = found else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let call = BuiltinCall {
        function,
        realm,
        this,
        arguments,
        new_target,
    };
    let result = (builtin.behaviour)(agent, scope, &call);
    if matches!(result, Err(Abrupt::Throw)) {
        agent.set_error_realm(realm);
    }
    result
}

#[cfg(test)]
mod tests;
