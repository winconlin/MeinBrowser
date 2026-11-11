//! ECMAScript-Funktionsobjekte (Abschnitt 10.2).
//!
//! Ein Funktionsobjekt ist ein Objekt der Art [`ObjectClass::Function`] mit
//! sechs internen Feldern vor seinen Eigenschaften: der Nummer seiner
//! gemeinsamen Daten im Agent, der Nummer seines Realms ([[Realm]]), dem
//! Kontext, in dem es entstand ([[Environment]]; *undefined* für Funktionen,
//! die nur die globale Umgebung sehen), dem Objekt, dessen Prototyp
//! `super` liest ([[HomeObject]]; *undefined* ohne), und beim Konstruktor
//! einer Klasse [[Fields]] und [[PrivateMethods]]. [[Fields]] ist eine
//! interne Zelle: vorn die Zahl der Felder, dann je Feld Name und
//! Initialisierer (*undefined* ohne), mit Platz für weitere. Die privaten
//! Methoden und Accessoren liegen als private Elemente an einem eigenen
//! Objekt ohne Prototyp, das Code der Sprache nie sieht; beide Felder sind
//! *undefined*, solange die Klasse nichts davon hat.
//! Die gemeinsamen Daten ([`SharedFunction`]) gehören zu der Stelle im
//! Quelltext, an der die Funktionsobjekte entstehen: der Syntaxbaum, die
//! Funktion darin und, sobald sie das erste Mal aufgerufen wurde, ihr
//! Bytecode mit den Konstanten als Werten. Wie die Atome bleiben sie
//! vorerst, solange der Agent lebt. Seine internen Methoden setzt ein
//! Funktionsobjekt gewöhnlich um, [[Call]] und [[Construct]] stehen im
//! Interpreter.
//!
//! [[ThisMode]], [[Strict]], [[FormalParameters]], [[ECMAScriptCode]],
//! [[ConstructorKind]], [[IsClassConstructor]] und ob es [[Construct]]
//! gibt, folgen aus der Funktion im Syntaxbaum ([`Construction`]).
//! Generatorfunktionen haben %GeneratorFunction.prototype% als Prototyp und
//! ein `prototype` aus %GeneratorPrototype%, aber kein [[Construct]].
//! async-Funktionen haben %AsyncFunction.prototype% als Prototyp, weder
//! `prototype` noch [[Construct]] (Abschnitt 27.10.4).
//!
//! Klassen: Der Konstruktor einer Klasse ohne `constructor` ist in der
//! Spezifikation eine eingebaute Funktion (ClassDefinitionEvaluation,
//! Abschnitt 15.7.14, Schritt 14.a). Hier hat er gemeinsame Daten mit
//! festem Bytecode ([`Origin::DefaultConstructor`]), damit `super()` wie
//! jeder Aufruf ohne Rekursion in Rust läuft. Beobachten lässt sich das
//! nicht: Beide haben `length` 0 und den Namen der Klasse, und keiner hat
//! `prototype` außer dem aus MakeConstructor.

#![forbid(unsafe_code)]

use std::rc::Rc;

use crate::agent::{Agent, Completion, ErrorKind};
use crate::ast::{Ast, Function, FunctionKind, Node};
use crate::builtin::{builtin_entry, builtin_parts};
use crate::bytecode::{Code, FunctionTemplate};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Heap, Local, ObjectClass, OutOfMemory, Persistent, Scope};
use crate::key::{PropertyKey, symbol_description};
use crate::object::{INLINE, ordinary_object_create};
use crate::operations::define_property_or_throw;
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Ref, Value};

/// Das interne Feld mit der Nummer der gemeinsamen Daten.
const SHARED: usize = 0;
/// Das interne Feld mit der Nummer des Realms.
const REALM: usize = 1;
/// Das interne Feld mit dem Kontext.
const ENVIRONMENT: usize = 2;
/// Das interne Feld mit [[HomeObject]].
const HOME: usize = 3;
/// Das interne Feld mit [[Fields]].
const FIELDS: usize = 4;
/// Das interne Feld mit [[PrivateMethods]].
const PRIVATE_METHODS: usize = 5;
/// In der Zelle von [[Fields]]: die Zahl der Felder, dann je Feld zwei
/// Werte.
const FIELD_COUNT: usize = 0;
const FIELD_ENTRY: usize = 2;

/// Die gemeinsamen Daten der Funktionsobjekte einer Stelle im Quelltext.
pub(crate) struct SharedFunction {
    /// Der Syntaxbaum des Scripts, aus dem die Funktion stammt.
    pub(crate) unit: Rc<Ast>,
    pub(crate) origin: Origin,
    /// Bytecode und Konstanten nach dem ersten Aufruf.
    pub(crate) compiled: Option<Compiled>,
}

/// Woraus der Code einer Funktion entsteht.
pub(crate) enum Origin {
    /// Aus einer Funktion im Syntaxbaum.
    Function(FunctionTemplate),
    /// Der Konstruktor einer Klasse ohne `constructor`, mit `derived` für
    /// eine abgeleitete Klasse.
    DefaultConstructor { derived: bool },
}

/// Ob ein Funktionsobjekt [[Construct]] hat und von welcher Art.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Construction {
    /// Kein [[Construct]].
    None,
    /// Eine gewöhnliche Funktion aus einer Deklaration oder einem Ausdruck,
    /// [[ConstructorKind]] ~base~.
    Function,
    /// Der Konstruktor einer Klasse ohne `extends` (MakeClassConstructor,
    /// Abschnitt 10.2.6): [[Call]] wirft einen TypeError.
    Base,
    /// Der Konstruktor einer abgeleiteten Klasse, [[ConstructorKind]]
    /// ~derived~.
    Derived,
}

impl Construction {
    /// Ob [[IsClassConstructor]] gilt.
    pub(crate) fn is_class(self) -> bool {
        matches!(self, Construction::Base | Construction::Derived)
    }
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
        self.add_shared(unit, Origin::Function(template))
    }

    /// Dasselbe für den Konstruktor einer Klasse ohne `constructor` im
    /// Syntaxbaum `unit`.
    pub(crate) fn add_default_constructor(
        &mut self,
        unit: Rc<Ast>,
        derived: bool,
    ) -> Result<i32, OutOfMemory> {
        self.add_shared(unit, Origin::DefaultConstructor { derived })
    }

    fn add_shared(&mut self, unit: Rc<Ast>, origin: Origin) -> Result<i32, OutOfMemory> {
        let index = i32::try_from(self.functions.len()).map_err(|_| OutOfMemory)?;
        self.functions.push(SharedFunction {
            unit,
            origin,
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

/// GetFunctionRealm (Abschnitt 7.3.24): der Realm eines Funktionsobjekts
/// oder einer eingebauten Funktion, für andere Objekte `current`, der
/// aktuelle. Gebundene Funktionen und Proxys kommen mit den eingebauten
/// Objekten.
pub(crate) fn get_function_realm(heap: &Heap, value: Value<'_>, current: RealmId) -> RealmId {
    function_parts(heap, value)
        .or_else(|| builtin_parts(heap, value))
        .map_or(current, |(_, realm)| realm)
}

/// [[HomeObject]] des Funktionsobjekts `value`, `None` ohne und für andere
/// Werte.
pub(crate) fn home_object<'h>(heap: &'h Heap, value: Value<'_>) -> Option<Value<'h>> {
    let Kind::Object(object) = value.kind() else {
        return None;
    };
    heap.function_field(object, HOME)
        .filter(|home| !home.is_undefined())
}

/// MakeMethod (Abschnitt 10.2.7): `home` wird [[HomeObject]] des
/// Funktionsobjekts `function`. Für andere Werte geschieht nichts.
pub(crate) fn make_method(heap: &Heap, function: Value<'_>, home: Value<'_>) {
    if let Kind::Object(object) = function.kind() {
        heap.set_function_field(object, HOME, home);
    }
}

/// Hängt an [[Fields]] des Funktionsobjekts `function` ein Feld mit dem
/// Namen `name` und dem Initialisierer `initializer` an
/// (ClassDefinitionEvaluation, Abschnitt 15.7.14, Schritte 31.f und 33).
/// Ist die Zelle voll, kommt eine doppelt so große. Für andere Werte
/// geschieht nichts.
pub(crate) fn add_field(
    agent: &mut Agent,
    scope: &Scope,
    function: Local<'_>,
    name: Local<'_>,
    initializer: Local<'_>,
) -> Result<(), OutOfMemory> {
    let (count, capacity) = {
        let heap = agent.heap();
        match fields_cell(heap, heap.get(function)) {
            Some(cell) => (
                heap.slot(cell, FIELD_COUNT)
                    .and_then(Value::as_int32)
                    .and_then(|count| usize::try_from(count).ok())
                    .unwrap_or(0),
                heap.slot_count(cell),
            ),
            None => (0, 0),
        }
    };
    let next = count.checked_add(1).ok_or(OutOfMemory)?;
    let stored = i32::try_from(next).map_err(|_| OutOfMemory)?;
    let entry = count
        .checked_mul(FIELD_ENTRY)
        .and_then(|values| values.checked_add(1))
        .ok_or(OutOfMemory)?;
    let needed = entry.checked_add(FIELD_ENTRY).ok_or(OutOfMemory)?;
    if needed > capacity {
        let size = needed
            .max(capacity.checked_mul(2).ok_or(OutOfMemory)?)
            .max(1 + 4 * FIELD_ENTRY);
        let cell = agent.heap_mut().alloc_slots(scope, size)?;
        let heap = agent.heap();
        let (Kind::Object(object), Kind::Internal(new)) =
            (heap.get(function).kind(), heap.get(cell).kind())
        else {
            return Ok(());
        };
        if let Some(old) = fields_cell(heap, heap.get(function)) {
            for index in 0..capacity {
                heap.set_slot(
                    new,
                    index,
                    heap.slot(old, index).unwrap_or(Value::UNDEFINED),
                );
            }
        }
        heap.set_function_field(object, FIELDS, Value::internal(new));
    }
    let heap = agent.heap();
    if let Some(cell) = fields_cell(heap, heap.get(function)) {
        heap.set_slot(cell, entry, heap.get(name));
        heap.set_slot(cell, entry.saturating_add(1), heap.get(initializer));
        heap.set_slot(cell, FIELD_COUNT, Value::int32(stored));
    }
    Ok(())
}

/// Das Feld mit dem Index `index` in [[Fields]] des Funktionsobjekts
/// `function`: Name und Initialisierer, `None` dahinter und für andere
/// Werte.
pub(crate) fn field_entry<'h>(
    heap: &'h Heap,
    function: Value<'_>,
    index: usize,
) -> Option<(Value<'h>, Value<'h>)> {
    let cell = fields_cell(heap, function)?;
    let count = heap.slot(cell, FIELD_COUNT)?.as_int32()?;
    if index >= usize::try_from(count).ok()? {
        return None;
    }
    let entry = index.checked_mul(FIELD_ENTRY)?.checked_add(1)?;
    Some((
        heap.slot(cell, entry)?,
        heap.slot(cell, entry.checked_add(1)?)?,
    ))
}

fn fields_cell<'h>(heap: &'h Heap, function: Value<'_>) -> Option<Ref<'h>> {
    let Kind::Object(object) = function.kind() else {
        return None;
    };
    match heap.function_field(object, FIELDS)?.kind() {
        Kind::Internal(cell) => Some(cell),
        _ => None,
    }
}

/// Das Objekt mit den privaten Methoden und Accessoren ([[PrivateMethods]])
/// des Funktionsobjekts `function`, `None` ohne und für andere Werte.
pub(crate) fn private_methods<'h>(heap: &'h Heap, function: Value<'_>) -> Option<Value<'h>> {
    let Kind::Object(object) = function.kind() else {
        return None;
    };
    heap.function_field(object, PRIVATE_METHODS)
        .filter(|methods| matches!(methods.kind(), Kind::Object(_)))
}

/// Legt das Objekt für [[PrivateMethods]] des Funktionsobjekts `function`
/// an und gibt es zurück. Für andere Werte bleibt es ungenutzt.
pub(crate) fn create_private_methods<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    function: Local<'_>,
) -> Result<Local<'s>, OutOfMemory> {
    let methods = ordinary_object_create(agent, scope, None)?;
    let heap = agent.heap();
    if let Kind::Object(object) = heap.get(function).kind() {
        heap.set_function_field(object, PRIVATE_METHODS, heap.get(methods));
    }
    Ok(methods)
}

/// IsConstructor (Abschnitt 7.2.4): Funktionsobjekte, die MakeConstructor
/// bekamen, und eingebaute Funktionen, deren Eintrag [[Construct]] nennt.
pub(crate) fn is_constructor(agent: &Agent, value: Value<'_>) -> bool {
    if let Some(builtin) = builtin_entry(agent, value) {
        return builtin.constructor;
    }
    construction(agent, value) != Construction::None
}

/// Ob und wie das Funktionsobjekt `value` [[Construct]] hat;
/// [`Construction::None`] für andere Werte.
pub(crate) fn construction(agent: &Agent, value: Value<'_>) -> Construction {
    function_parts(agent.heap(), value).map_or(Construction::None, |(shared, _)| {
        shared_construction(agent, shared)
    })
}

/// Dasselbe für die gemeinsamen Daten `shared`.
pub(crate) fn shared_construction(agent: &Agent, shared: usize) -> Construction {
    let Some(entry) = agent.functions.get(shared) else {
        return Construction::None;
    };
    let template = match &entry.origin {
        Origin::Function(template) => template,
        Origin::DefaultConstructor { derived: true } => return Construction::Derived,
        Origin::DefaultConstructor { derived: false } => return Construction::Base,
    };
    match entry.unit.function(template.function) {
        Some(function) if function.kind == FunctionKind::Constructor => Construction::Base,
        Some(function) if function.kind == FunctionKind::DerivedConstructor => {
            Construction::Derived
        }
        Some(function) if constructs(function) => Construction::Function,
        _ => Construction::None,
    }
}

/// Ob Funktionsobjekte der Funktion MakeConstructor bekommen: gewöhnliche
/// Funktionen aus Deklarationen und Ausdrücken, keine Generatoren und keine
/// mit `async` (Abschnitte 15.2.4 und 15.2.5). Pfeilfunktionen (Abschnitt
/// 15.3.4), Methoden, Getter und Setter (Abschnitt 15.4) nicht. Die
/// Konstruktoren von Klassen entstehen nur mit
/// [`instantiate_class_constructor`].
fn constructs(function: &Function) -> bool {
    function.kind == FunctionKind::Normal && !function.generator && !function.is_async
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
/// (Abschnitte 15.4.4 und 15.4.5) ohne MakeMethod; das setzt danach
/// [`make_method`], wenn der Code `super` liest. Pfeilfunktionen
/// (Abschnitt 15.3.4) haben wie Methoden kein `prototype`.
///
/// Generatoren und Generator-Methoden (InstantiateGeneratorFunctionObject
/// und InstantiateGeneratorFunctionExpression, Abschnitte 15.5.3 und
/// 15.5.4, und MethodDefinitionEvaluation) haben stattdessen
/// %GeneratorFunction.prototype% als Prototyp und ein neues Objekt aus
/// %GeneratorPrototype% als `prototype`, beschreibbar, aber weder aufzählbar
/// noch konfigurierbar und ohne `constructor`. async-Funktionen, async-Methoden
/// und async-Pfeilfunktionen (InstantiateAsyncFunctionObject,
/// InstantiateAsyncFunctionExpression und
/// InstantiateAsyncArrowFunctionExpression, Abschnitte 15.8.2, 15.8.3 und
/// 15.9.4) haben %AsyncFunction.prototype% als Prototyp und kein
/// `prototype`. async-Generatoren und ihre Methoden
/// (InstantiateAsyncGeneratorFunctionObject und
/// InstantiateAsyncGeneratorFunctionExpression, Abschnitte 15.6.3 und
/// 15.6.4) haben wie Generatoren ein `prototype`, mit
/// %AsyncGeneratorFunction.prototype% und %AsyncGeneratorPrototype%.
pub(crate) fn instantiate_closure<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    shared: i32,
    name: PropertyKey<'_>,
    environment: Local<'_>,
) -> Completion<Local<'s>> {
    let found = shared_entry(agent, shared).and_then(|entry| {
        let Origin::Function(template) = &entry.origin else {
            return None;
        };
        let function = entry.unit.function(template.function)?;
        Some((
            function.kind,
            function.generator,
            function.is_async,
            constructs(function),
            expected_argument_count(&entry.unit, function),
        ))
    });
    // Die Nummer stammt aus den Konstanten des Codes, der die Funktion
    // anlegt; es gibt sie immer, und sie gehört zu einer Funktion.
    let Some((kind, generator, is_async, constructor, length)) = found else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let intrinsic = match (generator, is_async) {
        (false, false) => Intrinsic::FunctionPrototype,
        (true, false) => Intrinsic::GeneratorFunctionPrototype,
        (false, true) => Intrinsic::AsyncFunctionPrototype,
        (true, true) => Intrinsic::AsyncGeneratorFunctionPrototype,
    };
    let prototype = agent.intrinsic(scope, realm, intrinsic);
    let function = allocate_function(agent, scope, realm, shared, environment, prototype)?;
    set_function_length(agent, scope, function, length)?;
    let prefix = match kind {
        FunctionKind::Getter => Some("get"),
        FunctionKind::Setter => Some("set"),
        _ => None,
    };
    set_function_name(agent, scope, function, name, prefix)?;
    if constructor {
        make_constructor(agent, scope, realm, function)?;
    }
    if generator {
        let intrinsic = if is_async {
            Intrinsic::AsyncGeneratorPrototype
        } else {
            Intrinsic::GeneratorPrototype
        };
        let generator_prototype = agent.intrinsic(scope, realm, intrinsic);
        let prototype = ordinary_object_create(agent, scope, Some(generator_prototype))?;
        let key = agent.key_from_units(scope, &units("prototype"))?;
        let descriptor = PropertyDescriptor::data(prototype, true, false, false);
        define_property_or_throw(agent, scope, function, key, &descriptor)?;
    }
    Ok(function)
}

/// Der Konstruktor einer Klasse für die gemeinsamen Daten `shared` im Realm
/// `realm` mit dem Namen `name` (className), dem Kontext `environment`, dem
/// Prototyp `parent` (ctorParent) und `prototype` (proto) als
/// [[HomeObject]] und als `prototype` (ClassDefinitionEvaluation,
/// Abschnitt 15.7.14, Schritte 14 bis 18): DefineMethod mit MakeMethod und
/// MakeClassConstructor, SetFunctionName, MakeConstructor ohne
/// Schreibrecht und DefineMethodProperty für `constructor` auf dem
/// Prototyp. Ohne `constructor` in der Klasse ist `length` 0 wie bei
/// CreateBuiltinFunction.
#[allow(clippy::too_many_arguments)]
pub(crate) fn instantiate_class_constructor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    shared: i32,
    name: PropertyKey<'_>,
    environment: Local<'_>,
    parent: Local<'_>,
    prototype: Local<'_>,
) -> Completion<Local<'s>> {
    let found = shared_entry(agent, shared).and_then(|entry| match &entry.origin {
        Origin::Function(template) => {
            let function = entry.unit.function(template.function)?;
            Some(expected_argument_count(&entry.unit, function))
        }
        Origin::DefaultConstructor { .. } => Some(0),
    });
    let class =
        usize::try_from(shared).is_ok_and(|index| shared_construction(agent, index).is_class());
    // Die Konstante stammt aus einer Klasse; es gibt sie immer.
    let (Some(length), true) = (found, class) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let function = allocate_function(agent, scope, realm, shared, environment, parent)?;
    let heap = agent.heap();
    make_method(heap, heap.get(function), heap.get(prototype));
    set_function_length(agent, scope, function, length)?;
    set_function_name(agent, scope, function, name, None)?;
    let key = agent.key_from_units(scope, &units("prototype"))?;
    let descriptor = PropertyDescriptor::data(prototype, false, false, false);
    define_property_or_throw(agent, scope, function, key, &descriptor)?;
    let key = agent.key_from_units(scope, &units("constructor"))?;
    let descriptor = PropertyDescriptor::data(function, true, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    Ok(function)
}

/// Die gemeinsamen Daten mit der Nummer `shared`.
fn shared_entry(agent: &Agent, shared: i32) -> Option<&SharedFunction> {
    usize::try_from(shared)
        .ok()
        .and_then(|index| agent.functions.get(index))
}

/// Ein neues Funktionsobjekt mit `prototype` als [[Prototype]], noch ohne
/// Eigenschaften und ohne [[HomeObject]].
fn allocate_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    shared: i32,
    environment: Local<'_>,
    prototype: Local<'_>,
) -> Completion<Local<'s>> {
    let realm_number = i32::try_from(realm.index()).map_err(|_| OutOfMemory)?;
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
pub(crate) fn set_function_length(
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
/// in eckigen Klammern, ohne Beschreibung die leere Zeichenkette, ein
/// privater Name seine Beschreibung.
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
                if heap.is_private_symbol(symbol) {
                    text.extend(heap.string_units(description));
                } else {
                    text.push(u16::from(b'['));
                    text.extend(heap.string_units(description));
                    text.push(u16::from(b']'));
                }
            }
        }
    }
    let value = agent.heap_mut().alloc_string(scope, &text)?;
    let key = agent.key_from_units(scope, &units("name"))?;
    let descriptor = PropertyDescriptor::data(value, false, false, true);
    define_property_or_throw(agent, scope, function, key, &descriptor)
}

/// MakeConstructor (Abschnitt 10.2.5) ohne die Fälle für Klassen: ein
/// neues Objekt mit `constructor` als `prototype`. [[Construct]] folgt wie
/// in [`is_constructor`] aus der Funktion.
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
