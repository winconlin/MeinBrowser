//! ECMAScript-Funktionsobjekte (Abschnitt 10.2).
//!
//! Ein Funktionsobjekt ist ein Objekt der Art [`ObjectClass::Function`] mit
//! drei internen Feldern vor seinen Eigenschaften: der Nummer seiner
//! gemeinsamen Daten im Agent, der Nummer seines Realms ([[Realm]]) und dem
//! Kontext, in dem es entstand ([[Environment]]; *undefined* für Funktionen,
//! die nur die globale Umgebung sehen).
//! Die gemeinsamen Daten ([`SharedFunction`]) gehören zu der Stelle im
//! Quelltext, an der die Funktionsobjekte entstehen: der Syntaxbaum, die
//! Funktion darin und, sobald sie das erste Mal aufgerufen wurde, ihr
//! Bytecode mit den Konstanten als Werten. Wie die Atome bleiben sie
//! vorerst, solange der Agent lebt. Seine internen Methoden setzt ein
//! Funktionsobjekt gewöhnlich um, [[Call]] steht im Interpreter.
//!
//! [[ThisMode]], [[Strict]], [[FormalParameters]] und [[ECMAScriptCode]]
//! folgen aus der Funktion im Syntaxbaum. Es fehlen noch [[HomeObject]] und
//! alles für Klassen.

#![forbid(unsafe_code)]

use std::rc::Rc;

use crate::agent::{Agent, Completion, ErrorKind};
use crate::ast::{Ast, Function, FunctionKind, Node};
use crate::bytecode::{Code, FunctionTemplate};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Heap, Local, ObjectClass, OutOfMemory, Persistent, Scope};
use crate::key::{PropertyKey, symbol_description};
use crate::object::{INLINE, ordinary_object_create};
use crate::operations::define_property_or_throw;
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Das interne Feld mit der Nummer der gemeinsamen Daten.
const SHARED: usize = 0;
/// Das interne Feld mit der Nummer des Realms.
const REALM: usize = 1;
/// Das interne Feld mit dem Kontext.
const ENVIRONMENT: usize = 2;

/// Die gemeinsamen Daten der Funktionsobjekte einer Stelle im Quelltext.
pub(crate) struct SharedFunction {
    /// Der Syntaxbaum des Scripts, aus dem die Funktion stammt.
    pub(crate) unit: Rc<Ast>,
    pub(crate) template: FunctionTemplate,
    /// Bytecode und Konstanten nach dem ersten Aufruf.
    pub(crate) compiled: Option<Compiled>,
}

/// Eine übersetzte Funktion.
pub(crate) struct Compiled {
    pub(crate) code: Rc<Code>,
    /// Die Konstanten als Werte in einem internen Feld.
    pub(crate) pool: Persistent,
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl Agent {
    /// Legt gemeinsame Daten für die Funktion `template` aus dem
    /// Syntaxbaum `unit` an und gibt ihre Nummer zurück.
    pub(crate) fn add_shared_function(
        &mut self,
        unit: Rc<Ast>,
        template: FunctionTemplate,
    ) -> Result<i32, OutOfMemory> {
        let index = i32::try_from(self.functions.len()).map_err(|_| OutOfMemory)?;
        self.functions.push(SharedFunction {
            unit,
            template,
            compiled: None,
        });
        Ok(index)
    }
}

/// Die Nummer der gemeinsamen Daten und der Realm des Funktionsobjekts
/// `value`, `None` für andere Werte.
pub(crate) fn function_parts(heap: &Heap, value: Value<'_>) -> Option<(usize, RealmId)> {
    let Kind::Object(object) = value.kind() else {
        return None;
    };
    let shared = heap.function_field(object, SHARED)?.as_int32()?;
    let realm = heap.function_field(object, REALM)?.as_int32()?;
    Some((
        usize::try_from(shared).ok()?,
        RealmId::from_index(usize::try_from(realm).ok()?),
    ))
}

/// Der Kontext ([[Environment]]) des Funktionsobjekts `value`, `None` für
/// andere Werte.
pub(crate) fn function_environment<'h>(heap: &'h Heap, value: Value<'_>) -> Option<Value<'h>> {
    let Kind::Object(object) = value.kind() else {
        return None;
    };
    heap.function_field(object, ENVIRONMENT)
}

/// Ein neues Funktionsobjekt für die gemeinsamen Daten `shared` im Realm
/// `realm` mit dem Namen `name`, das nur die globale Umgebung sieht.
pub(crate) fn instantiate_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    shared: i32,
    name: PropertyKey<'_>,
) -> Completion<Local<'s>> {
    let environment = agent.heap().local(scope, Value::UNDEFINED);
    instantiate_closure(agent, scope, realm, shared, name, environment)
}

/// Ein neues Funktionsobjekt für die gemeinsamen Daten `shared` im Realm
/// `realm` mit dem Namen `name` und dem Kontext `environment`:
/// OrdinaryFunctionCreate mit
/// %Function.prototype% (Abschnitt 10.2.3), SetFunctionName mit dem Präfix
/// für Getter und Setter und für gewöhnliche Funktionen MakeConstructor.
/// Das sind InstantiateOrdinaryFunctionObject und
/// InstantiateOrdinaryFunctionExpression (Abschnitte 15.2.4 und 15.2.5)
/// sowie die Schritte aus DefineMethod und MethodDefinitionEvaluation
/// (Abschnitte 15.4.2 und 15.4.4) ohne MakeMethod. Pfeilfunktionen
/// (Abschnitt 15.3.4) haben wie Methoden kein `prototype`.
pub(crate) fn instantiate_closure<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    shared: i32,
    name: PropertyKey<'_>,
    environment: Local<'_>,
) -> Completion<Local<'s>> {
    let found = usize::try_from(shared)
        .ok()
        .and_then(|index| agent.functions.get(index))
        .and_then(|entry| {
            let function = entry.unit.function(entry.template.function)?;
            Some((
                function.kind,
                function.generator || function.is_async,
                expected_argument_count(&entry.unit, function),
            ))
        });
    // Die Nummer stammt aus den Konstanten des Codes, der die Funktion
    // anlegt; es gibt sie immer.
    let Some((kind, special, length)) = found else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let realm_number = i32::try_from(realm.index()).map_err(|_| OutOfMemory)?;
    let prototype = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
    let shape = agent.empty_shape(scope);
    let function = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::Function,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    if let Kind::Object(object) = heap.get(function).kind() {
        heap.set_function_field(object, SHARED, Value::int32(shared));
        heap.set_function_field(object, REALM, Value::int32(realm_number));
        heap.set_function_field(object, ENVIRONMENT, heap.get(environment));
    }
    set_function_length(agent, scope, function, length)?;
    let prefix = match kind {
        FunctionKind::Getter => Some("get"),
        FunctionKind::Setter => Some("set"),
        _ => None,
    };
    set_function_name(agent, scope, function, name, prefix)?;
    if kind == FunctionKind::Normal && !special {
        make_constructor(agent, scope, realm, function)?;
    }
    Ok(function)
}

/// ExpectedArgumentCount (Abschnitt 15.1.5): die Parameter vor dem ersten
/// mit Vorgabewert oder dem Rest.
fn expected_argument_count(ast: &Ast, function: &Function) -> u32 {
    let count = ast
        .list(function.parameters)
        .iter()
        .take_while(|&&parameter| {
            !matches!(
                ast.node(parameter),
                Node::WithDefault { .. } | Node::Rest(_)
            )
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// SetFunctionLength (Abschnitt 10.2.10).
fn set_function_length(
    agent: &mut Agent,
    scope: &Scope,
    function: Local<'_>,
    length: u32,
) -> Completion<()> {
    let value = agent.heap().local(scope, Value::number(f64::from(length)));
    let key = agent.key_from_units(scope, &units("length"))?;
    let descriptor = PropertyDescriptor::data(value, false, false, true);
    define_property_or_throw(agent, scope, function, key, &descriptor)
}

/// SetFunctionName (Abschnitt 10.2.9). Ein Symbol gibt seine Beschreibung
/// in eckigen Klammern, ohne Beschreibung die leere Zeichenkette.
pub(crate) fn set_function_name(
    agent: &mut Agent,
    scope: &Scope,
    function: Local<'_>,
    name: PropertyKey<'_>,
    prefix: Option<&str>,
) -> Completion<()> {
    let mut text: Vec<u16> = prefix
        .map(|prefix| prefix.encode_utf16().chain([u16::from(b' ')]).collect())
        .unwrap_or_default();
    let heap = agent.heap();
    match name {
        PropertyKey::Index(index) => text.extend(index.to_string().encode_utf16()),
        PropertyKey::String(string) => {
            if let Kind::String(string) = heap.get(string).kind() {
                text.extend(heap.string_units(string));
            }
        }
        PropertyKey::Symbol(symbol) => {
            if let Kind::Symbol(symbol) = heap.get(symbol).kind()
                && let Kind::String(description) = symbol_description(heap, symbol).kind()
            {
                text.push(u16::from(b'['));
                text.extend(heap.string_units(description));
                text.push(u16::from(b']'));
            }
        }
    }
    let value = agent.heap_mut().alloc_string(scope, &text)?;
    let key = agent.key_from_units(scope, &units("name"))?;
    let descriptor = PropertyDescriptor::data(value, false, false, true);
    define_property_or_throw(agent, scope, function, key, &descriptor)
}

/// MakeConstructor (Abschnitt 10.2.5) ohne die Fälle für Klassen: ein
/// neues Objekt mit `constructor` als `prototype`. [[Construct]] kommt mit
/// `new`.
fn make_constructor(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    function: Local<'_>,
) -> Completion<()> {
    let object_prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(object_prototype))?;
    let constructor = agent.key_from_units(scope, &units("constructor"))?;
    let descriptor = PropertyDescriptor::data(function, true, false, true);
    define_property_or_throw(agent, scope, prototype, constructor, &descriptor)?;
    let key = agent.key_from_units(scope, &units("prototype"))?;
    let descriptor = PropertyDescriptor::data(prototype, true, false, false);
    define_property_or_throw(agent, scope, function, key, &descriptor)
}

#[cfg(test)]
mod tests;
