//! Proxy-Objekte (Abschnitt 10.5).
//!
//! Ein Proxy ist ein Objekt der Art [`ObjectClass::Proxy`] mit drei
//! internen Feldern: [[ProxyTarget]], [[ProxyHandler]] und ob er [[Call]]
//! und [[Construct]] hat. Das entscheidet ProxyCreate einmal am Ziel; ein
//! widerrufener Proxy behält es, hat aber *null* in beiden anderen Feldern.
//! Gestalt und Plätze hat er wie jedes Objekt, aber nur für private
//! Elemente, die ein Konstruktor einer Basisklasse ihm geben kann; sein
//! Prototyp bleibt leer.
//!
//! Jede interne Methode in `object` fragt bei einem Proxy dieses Modul. Hat
//! der Handler keine Falle, geht die Methode an das Ziel: Das Modul schreibt
//! es in den Handle des Aufrufers und meldet [`Step::Forward`], und der
//! Aufrufer fährt in seiner Schleife fort. So braucht eine Kette von
//! Proxys ohne Fallen keinen Rust-Stapel (RS-8). Sonst ruft das Modul die
//! Falle und prüft ihr Ergebnis gegen die Invarianten des Ziels.
//!
//! Jeder Schritt zählt wie ein Lauf des Interpreters
//! ([`Agent::enter_interpreter`]): Er kostet einen Arbeitsschritt, und der
//! Rust-Stapel darf dabei nur bis [`crate::agent::STACK_BUDGET`] wachsen.
//! Das begrenzt auch die Wege, auf denen Rust-Code ohne JavaScript tiefer
//! geht: ein Handler, der selbst ein Proxy ist, dessen Handler wieder einer
//! ist, und die Prüfungen der Invarianten, die interne Methoden des Ziels
//! aufrufen. Die Werte eines Schritts liegen in einem eigenen Bereich; nur
//! das Ergebnis kommt in den des Aufrufers.
//!
//! Neue Objekte für den Handler, die Beschreibung für `defineProperty` und
//! das Array der Argumente für `apply` und `construct`, entstehen im
//! aktuellen Realm ([`Agent::current_realm`]), wie die Spezifikation es
//! verlangt; für `construct` kennt ihn schon der Aufrufer.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use crate::agent::{Agent, Completion, ErrorKind};
use crate::compare::same_value;
use crate::conversion::to_boolean;
use crate::descriptor::{PropertyDescriptor, from_property_descriptor, to_property_descriptor};
use crate::function::{get_function_realm, is_constructor};
use crate::heap::{Heap, Local, ObjectClass, Scope};
use crate::key::PropertyKey;
use crate::operations::{
    call, create_array_from_list, get_method, is_callable, length_of_array_like,
};
use crate::realm::RealmId;
use crate::value::{Kind, Value};

/// Das interne Feld mit [[ProxyTarget]].
const TARGET: usize = 0;
/// Das interne Feld mit [[ProxyHandler]].
const HANDLER: usize = 1;
/// Das interne Feld mit den Kennzeichen [`CALL`] und [`CONSTRUCT`].
const FLAGS: usize = 2;
/// Der Proxy hat [[Call]].
const CALL: i32 = 1;
/// Der Proxy hat [[Construct]].
const CONSTRUCT: i32 = 2;

/// Was ein Schritt eines Proxys ergibt.
pub(crate) enum Step<T> {
    /// Der Handler hat keine Falle; im Handle des Aufrufers steht jetzt das
    /// Ziel, an das die interne Methode weitergeht.
    Forward,
    /// Das Ergebnis der Falle, geprüft.
    Done(T),
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn type_error<T>(agent: &mut Agent) -> Completion<T> {
    Err(agent.throw_error(ErrorKind::TypeError))
}

fn is_object(value: Value<'_>) -> bool {
    matches!(value.kind(), Kind::Object(_))
}

/// Ob `value` ein Proxy ist.
pub(crate) fn is_proxy(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        Kind::Object(object) => heap.object_class(object) == Some(ObjectClass::Proxy),
        _ => false,
    }
}

/// [[ProxyTarget]] des Proxys `value`, *null*, wenn er widerrufen ist;
/// `None` für andere Werte.
pub(crate) fn proxy_target<'h>(heap: &'h Heap, value: Value<'_>) -> Option<Value<'h>> {
    let Kind::Object(object) = value.kind() else {
        return None;
    };
    heap.proxy_field(object, TARGET)
}

fn flags(heap: &Heap, value: Value<'_>) -> i32 {
    let Kind::Object(object) = value.kind() else {
        return 0;
    };
    heap.proxy_field(object, FLAGS)
        .and_then(Value::as_int32)
        .unwrap_or(0)
}

/// Ob der Proxy `value` [[Call]] hat; *false* für andere Werte.
pub(crate) fn has_call(heap: &Heap, value: Value<'_>) -> bool {
    flags(heap, value) & CALL != 0
}

/// Ob der Proxy `value` [[Construct]] hat; *false* für andere Werte.
pub(crate) fn has_construct(heap: &Heap, value: Value<'_>) -> bool {
    flags(heap, value) & CONSTRUCT != 0
}

/// ProxyCreate (Abschnitt 10.5.15).
pub(crate) fn proxy_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    target: Local<'_>,
    handler: Local<'_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    if !is_object(heap.get(target)) || !is_object(heap.get(handler)) {
        return type_error(agent);
    }
    let mut kinds = 0;
    if is_callable(heap, heap.get(target)) {
        kinds |= CALL;
    }
    if is_constructor(agent, agent.heap().get(target)) {
        kinds |= CONSTRUCT;
    }
    let shape = agent.empty_shape(scope);
    let proxy = agent
        .heap_mut()
        .alloc_object(scope, ObjectClass::Proxy, 0, shape, None)?;
    let heap = agent.heap();
    if let Kind::Object(cell) = heap.get(proxy).kind() {
        heap.set_proxy_field(cell, TARGET, heap.get(target));
        heap.set_proxy_field(cell, HANDLER, heap.get(handler));
        heap.set_proxy_field(cell, FLAGS, Value::int32(kinds));
    }
    Ok(proxy)
}

/// Widerruft den Proxy `value` (Schritte 4 bis 7 der Funktion aus
/// Proxy.revocable, Abschnitt 28.2.2.1.1).
pub(crate) fn revoke(heap: &Heap, value: Value<'_>) {
    if let Kind::Object(object) = value.kind() {
        heap.set_proxy_field(object, TARGET, Value::NULL);
        heap.set_proxy_field(object, HANDLER, Value::NULL);
    }
}

/// Ein Schritt eines Proxys: ein Lauf mit eigenem Bereich für die Werte,
/// die er braucht.
fn step<T>(
    agent: &mut Agent,
    f: impl FnOnce(&mut Agent, &Scope) -> Completion<T>,
) -> Completion<T> {
    agent.with_default_execution_budget(|agent| {
        agent.enter_interpreter()?;
        let result = agent.scope(f);
        agent.leave_interpreter();
        result
    })
}

/// Ziel, Handler und Falle eines Proxys.
struct Trap<'s> {
    target: Local<'s>,
    handler: Local<'s>,
    function: Local<'s>,
}

/// ValidateNonRevokedProxy (Abschnitt 10.5.14), [[ProxyTarget]],
/// [[ProxyHandler]] und GetMethod(handler, `name`) für den Proxy in
/// `cursor`. Ohne Falle schreibt es das Ziel in `cursor` und gibt `None`.
fn trap<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    cursor: Local<'_>,
    name: &str,
) -> Completion<Option<Trap<'s>>> {
    let fields = {
        let heap = agent.heap();
        match heap.get(cursor).kind() {
            Kind::Object(object) => heap
                .proxy_field(object, TARGET)
                .zip(heap.proxy_field(object, HANDLER))
                .map(|(target, handler)| (heap.local(scope, target), heap.local(scope, handler))),
            _ => None,
        }
    };
    // Andere Werte als Proxys kommen nicht hierher.
    let Some((target, handler)) = fields else {
        return type_error(agent);
    };
    if agent.heap().get(handler).is_null() {
        return type_error(agent);
    }
    let key = agent.key_from_units(scope, &units(name))?;
    match get_method(agent, scope, handler, key)? {
        Some(function) => Ok(Some(Trap {
            target,
            handler,
            function,
        })),
        None => {
            let heap = agent.heap();
            heap.set(cursor, heap.get(target));
            Ok(None)
        }
    }
}

/// Der aktuelle Realm für neue Objekte. Läuft nichts, etwa wenn der Host
/// eine interne Methode direkt aufruft, ist es der Realm der Falle.
fn current_realm(agent: &Agent, trap: &Trap<'_>) -> RealmId {
    agent.current_realm().unwrap_or_else(|| {
        let heap = agent.heap();
        let fallback = RealmId::from_index(0);
        get_function_realm(heap, heap.get(trap.function), fallback).unwrap_or(fallback)
    })
}

/// Die Prototypen `a` und `b` nach SameValue; `None` steht für *null*.
fn same_prototype(heap: &Heap, a: Option<Local<'_>>, b: Option<Local<'_>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_value(heap, heap.get(a), heap.get(b)),
        _ => false,
    }
}

/// Dieselbe Beschreibung mit Handles in `scope`.
fn rehome<'s>(
    heap: &Heap,
    scope: &'s Scope,
    descriptor: &PropertyDescriptor<'_>,
) -> PropertyDescriptor<'s> {
    let local = |field: Option<Local<'_>>| field.map(|field| heap.local(scope, heap.get(field)));
    PropertyDescriptor {
        value: local(descriptor.value),
        writable: descriptor.writable,
        get: local(descriptor.get),
        set: local(descriptor.set),
        enumerable: descriptor.enumerable,
        configurable: descriptor.configurable,
    }
}

/// Derselbe Schlüssel mit einem Handle in `scope`.
fn rehome_key<'s>(heap: &Heap, scope: &'s Scope, key: PropertyKey<'_>) -> PropertyKey<'s> {
    match key {
        PropertyKey::Index(index) => PropertyKey::Index(index),
        PropertyKey::String(name) => PropertyKey::String(heap.local(scope, heap.get(name))),
        PropertyKey::Symbol(name) => PropertyKey::Symbol(heap.local(scope, heap.get(name))),
    }
}

/// CompletePropertyDescriptor (Abschnitt 6.2.6.6).
fn complete_property_descriptor<'s>(
    heap: &Heap,
    scope: &'s Scope,
    descriptor: &mut PropertyDescriptor<'s>,
) {
    let undefined = || Some(heap.local(scope, Value::UNDEFINED));
    if descriptor.is_accessor() {
        descriptor.get = descriptor.get.or_else(undefined);
        descriptor.set = descriptor.set.or_else(undefined);
    } else {
        descriptor.value = descriptor.value.or_else(undefined);
        descriptor.writable = descriptor.writable.or(Some(false));
    }
    descriptor.enumerable = descriptor.enumerable.or(Some(false));
    descriptor.configurable = descriptor.configurable.or(Some(false));
}

/// Ob die Eigenschaft eine Dateneigenschaft ist, die weder konfigurierbar
/// noch beschreibbar ist; ihr Wert, wenn ja.
fn frozen_value<'s>(descriptor: &PropertyDescriptor<'s>) -> Option<Local<'s>> {
    (descriptor.configurable == Some(false)
        && descriptor.is_data()
        && descriptor.writable == Some(false))
    .then_some(descriptor.value)
    .flatten()
}

/// Ob die Eigenschaft ein nicht konfigurierbarer Accessor ist, dessen
/// Funktion `field` *undefined* ist.
fn missing_accessor(
    heap: &Heap,
    descriptor: &PropertyDescriptor<'_>,
    field: Option<Local<'_>>,
) -> bool {
    descriptor.configurable == Some(false)
        && descriptor.is_accessor()
        && field.is_none_or(|field| heap.get(field).is_undefined())
}

/// [[GetPrototypeOf]] (Abschnitt 10.5.1).
pub(super) fn get_prototype_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    cursor: Local<'_>,
) -> Completion<Step<Option<Local<'s>>>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "getPrototypeOf")? else {
            return Ok(Step::Forward);
        };
        let result = call(agent, inner, trap.function, trap.handler, &[trap.target])?;
        let prototype = match agent.heap().get(result).kind() {
            Kind::Object(_) => Some(result),
            Kind::Null => None,
            _ => return type_error(agent),
        };
        if !super::is_extensible(agent, trap.target)? {
            let target_prototype = super::get_prototype_of(agent, inner, trap.target)?;
            if !same_prototype(agent.heap(), prototype, target_prototype) {
                return type_error(agent);
            }
        }
        let heap = agent.heap();
        Ok(Step::Done(
            prototype.map(|prototype| heap.local(scope, heap.get(prototype))),
        ))
    })
}

/// [[SetPrototypeOf]] (Abschnitt 10.5.2).
pub(super) fn set_prototype_of(
    agent: &mut Agent,
    cursor: Local<'_>,
    prototype: Option<Local<'_>>,
) -> Completion<Step<bool>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "setPrototypeOf")? else {
            return Ok(Step::Forward);
        };
        let heap = agent.heap();
        let value = heap.local(
            inner,
            prototype.map_or(Value::NULL, |prototype| heap.get(prototype)),
        );
        let result = call(
            agent,
            inner,
            trap.function,
            trap.handler,
            &[trap.target, value],
        )?;
        if !to_boolean(agent.heap(), agent.heap().get(result)) {
            return Ok(Step::Done(false));
        }
        if super::is_extensible(agent, trap.target)? {
            return Ok(Step::Done(true));
        }
        let target_prototype = super::get_prototype_of(agent, inner, trap.target)?;
        if !same_prototype(agent.heap(), prototype, target_prototype) {
            return type_error(agent);
        }
        Ok(Step::Done(true))
    })
}

/// [[IsExtensible]] (Abschnitt 10.5.3).
pub(super) fn is_extensible(agent: &mut Agent, cursor: Local<'_>) -> Completion<Step<bool>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "isExtensible")? else {
            return Ok(Step::Forward);
        };
        let result = call(agent, inner, trap.function, trap.handler, &[trap.target])?;
        let result = to_boolean(agent.heap(), agent.heap().get(result));
        if result != super::is_extensible(agent, trap.target)? {
            return type_error(agent);
        }
        Ok(Step::Done(result))
    })
}

/// [[PreventExtensions]] (Abschnitt 10.5.4).
pub(super) fn prevent_extensions(agent: &mut Agent, cursor: Local<'_>) -> Completion<Step<bool>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "preventExtensions")? else {
            return Ok(Step::Forward);
        };
        let result = call(agent, inner, trap.function, trap.handler, &[trap.target])?;
        let result = to_boolean(agent.heap(), agent.heap().get(result));
        if result && super::is_extensible(agent, trap.target)? {
            return type_error(agent);
        }
        Ok(Step::Done(result))
    })
}

/// [[GetOwnProperty]] (Abschnitt 10.5.5).
pub(super) fn get_own_property<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    cursor: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Step<Option<PropertyDescriptor<'s>>>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "getOwnPropertyDescriptor")? else {
            return Ok(Step::Forward);
        };
        let name = agent.key_as_value(inner, key)?;
        let result = call(
            agent,
            inner,
            trap.function,
            trap.handler,
            &[trap.target, name],
        )?;
        let found = agent.heap().get(result);
        let undefined = found.is_undefined();
        if !is_object(found) && !undefined {
            return type_error(agent);
        }
        let target_descriptor = super::get_own_property(agent, inner, trap.target, key)?;
        if undefined {
            let Some(target_descriptor) = target_descriptor else {
                return Ok(Step::Done(None));
            };
            if target_descriptor.configurable == Some(false) {
                return type_error(agent);
            }
            if !super::is_extensible(agent, trap.target)? {
                return type_error(agent);
            }
            return Ok(Step::Done(None));
        }
        let extensible = super::is_extensible(agent, trap.target)?;
        let mut descriptor = to_property_descriptor(agent, inner, result)?;
        complete_property_descriptor(agent.heap(), inner, &mut descriptor);
        if !super::is_compatible_property_descriptor(
            agent,
            inner,
            extensible,
            &descriptor,
            target_descriptor.as_ref(),
        )? {
            return type_error(agent);
        }
        if descriptor.configurable == Some(false) {
            let Some(target_descriptor) = target_descriptor else {
                return type_error(agent);
            };
            if target_descriptor.configurable == Some(true) {
                return type_error(agent);
            }
            if descriptor.writable == Some(false) && target_descriptor.writable == Some(true) {
                return type_error(agent);
            }
        }
        Ok(Step::Done(Some(rehome(agent.heap(), scope, &descriptor))))
    })
}

/// [[DefineOwnProperty]] (Abschnitt 10.5.6).
pub(super) fn define_own_property(
    agent: &mut Agent,
    cursor: Local<'_>,
    key: PropertyKey<'_>,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<Step<bool>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "defineProperty")? else {
            return Ok(Step::Forward);
        };
        let realm = current_realm(agent, &trap);
        let object = from_property_descriptor(agent, inner, realm, Some(descriptor))?;
        let name = agent.key_as_value(inner, key)?;
        let result = call(
            agent,
            inner,
            trap.function,
            trap.handler,
            &[trap.target, name, object],
        )?;
        if !to_boolean(agent.heap(), agent.heap().get(result)) {
            return Ok(Step::Done(false));
        }
        let target_descriptor = super::get_own_property(agent, inner, trap.target, key)?;
        let extensible = super::is_extensible(agent, trap.target)?;
        let setting_config_false = descriptor.configurable == Some(false);
        let Some(target_descriptor) = target_descriptor else {
            if !extensible || setting_config_false {
                return type_error(agent);
            }
            return Ok(Step::Done(true));
        };
        if !super::is_compatible_property_descriptor(
            agent,
            inner,
            extensible,
            descriptor,
            Some(&target_descriptor),
        )? {
            return type_error(agent);
        }
        if setting_config_false && target_descriptor.configurable == Some(true) {
            return type_error(agent);
        }
        if target_descriptor.is_data()
            && target_descriptor.configurable == Some(false)
            && target_descriptor.writable == Some(true)
            && descriptor.writable == Some(false)
        {
            return type_error(agent);
        }
        Ok(Step::Done(true))
    })
}

/// [[HasProperty]] (Abschnitt 10.5.7).
pub(super) fn has_property(
    agent: &mut Agent,
    cursor: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Step<bool>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "has")? else {
            return Ok(Step::Forward);
        };
        let name = agent.key_as_value(inner, key)?;
        let result = call(
            agent,
            inner,
            trap.function,
            trap.handler,
            &[trap.target, name],
        )?;
        let result = to_boolean(agent.heap(), agent.heap().get(result));
        if !result
            && let Some(target_descriptor) =
                super::get_own_property(agent, inner, trap.target, key)?
        {
            if target_descriptor.configurable == Some(false) {
                return type_error(agent);
            }
            if !super::is_extensible(agent, trap.target)? {
                return type_error(agent);
            }
        }
        Ok(Step::Done(result))
    })
}

/// [[Get]] (Abschnitt 10.5.8).
pub(super) fn get<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    cursor: Local<'_>,
    key: PropertyKey<'_>,
    receiver: Local<'_>,
) -> Completion<Step<Local<'s>>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "get")? else {
            return Ok(Step::Forward);
        };
        let name = agent.key_as_value(inner, key)?;
        let result = call(
            agent,
            inner,
            trap.function,
            trap.handler,
            &[trap.target, name, receiver],
        )?;
        if let Some(target_descriptor) = super::get_own_property(agent, inner, trap.target, key)? {
            let heap = agent.heap();
            if let Some(value) = frozen_value(&target_descriptor)
                && !same_value(heap, heap.get(result), heap.get(value))
            {
                return type_error(agent);
            }
            if missing_accessor(heap, &target_descriptor, target_descriptor.get)
                && !heap.get(result).is_undefined()
            {
                return type_error(agent);
            }
        }
        let heap = agent.heap();
        Ok(Step::Done(heap.local(scope, heap.get(result))))
    })
}

/// [[Set]] (Abschnitt 10.5.9).
pub(super) fn set(
    agent: &mut Agent,
    cursor: Local<'_>,
    key: PropertyKey<'_>,
    value: Local<'_>,
    receiver: Local<'_>,
) -> Completion<Step<bool>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "set")? else {
            return Ok(Step::Forward);
        };
        let name = agent.key_as_value(inner, key)?;
        let result = call(
            agent,
            inner,
            trap.function,
            trap.handler,
            &[trap.target, name, value, receiver],
        )?;
        if !to_boolean(agent.heap(), agent.heap().get(result)) {
            return Ok(Step::Done(false));
        }
        if let Some(target_descriptor) = super::get_own_property(agent, inner, trap.target, key)? {
            let heap = agent.heap();
            if let Some(stored) = frozen_value(&target_descriptor)
                && !same_value(heap, heap.get(value), heap.get(stored))
            {
                return type_error(agent);
            }
            if missing_accessor(heap, &target_descriptor, target_descriptor.set) {
                return type_error(agent);
            }
        }
        Ok(Step::Done(true))
    })
}

/// [[Delete]] (Abschnitt 10.5.10).
pub(super) fn delete(
    agent: &mut Agent,
    cursor: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Step<bool>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "deleteProperty")? else {
            return Ok(Step::Forward);
        };
        let name = agent.key_as_value(inner, key)?;
        let result = call(
            agent,
            inner,
            trap.function,
            trap.handler,
            &[trap.target, name],
        )?;
        if !to_boolean(agent.heap(), agent.heap().get(result)) {
            return Ok(Step::Done(false));
        }
        let Some(target_descriptor) = super::get_own_property(agent, inner, trap.target, key)?
        else {
            return Ok(Step::Done(true));
        };
        if target_descriptor.configurable == Some(false) {
            return type_error(agent);
        }
        if !super::is_extensible(agent, trap.target)? {
            return type_error(agent);
        }
        Ok(Step::Done(true))
    })
}

/// Ein Schlüssel zum Vergleichen: ein Index oder die Bits seines Atoms
/// oder Symbols. Gültig, solange nichts sammelt.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KeyId {
    Index(u32),
    Named(u64),
}

fn key_id(heap: &Heap, key: PropertyKey<'_>) -> KeyId {
    match key {
        PropertyKey::Index(index) => KeyId::Index(index),
        PropertyKey::String(name) | PropertyKey::Symbol(name) => {
            KeyId::Named(heap.get(name).bits())
        }
    }
}

/// [[OwnPropertyKeys]] (Abschnitt 10.5.11) mit CreateListFromArrayLike
/// für Schlüssel (Abschnitt 7.3.19). Jedes Element des Ergebnisses und
/// jeder Schlüssel des Ziels kostet einen Arbeitsschritt; Doppelte und
/// fehlende Schlüssel findet eine geordnete Menge, nicht der Vergleich jedes
/// Paares.
pub(super) fn own_property_keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    cursor: Local<'_>,
) -> Completion<Step<Vec<PropertyKey<'s>>>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "ownKeys")? else {
            return Ok(Step::Forward);
        };
        let array = call(agent, inner, trap.function, trap.handler, &[trap.target])?;
        if !is_object(agent.heap().get(array)) {
            return type_error(agent);
        }
        let length = length_of_array_like(agent, inner, array)?;
        let mut keys = Vec::new();
        let mut index: u32 = 0;
        while f64::from(index) < length {
            agent.spend_execution_steps(1)?;
            let element = crate::operations::get(agent, inner, array, PropertyKey::Index(index))?;
            let Some(key) = agent.key_from_value(inner, element) else {
                return type_error(agent);
            };
            keys.push(key);
            // Mehr Elemente als Indizes kann kein Array halten.
            index = index
                .checked_add(1)
                .ok_or_else(|| agent.throw_error(ErrorKind::RangeError))?;
        }
        {
            let heap = agent.heap();
            let mut seen = BTreeSet::new();
            if !keys.iter().all(|&key| seen.insert(key_id(heap, key))) {
                return type_error(agent);
            }
        }
        let extensible = super::is_extensible(agent, trap.target)?;
        let target_keys = super::own_property_keys(agent, inner, trap.target)?;
        let mut configurable = Vec::new();
        let mut nonconfigurable = Vec::new();
        for &key in &target_keys {
            agent.spend_execution_steps(1)?;
            let descriptor = super::get_own_property(agent, inner, trap.target, key)?;
            if descriptor.is_some_and(|descriptor| descriptor.configurable == Some(false)) {
                nonconfigurable.push(key);
            } else {
                configurable.push(key);
            }
        }
        let heap = agent.heap();
        let result = |keys: &[PropertyKey<'_>]| {
            keys.iter()
                .map(|&key| rehome_key(heap, scope, key))
                .collect::<Vec<_>>()
        };
        if extensible && nonconfigurable.is_empty() {
            return Ok(Step::Done(result(&keys)));
        }
        let mut unchecked: BTreeSet<KeyId> = keys.iter().map(|&key| key_id(heap, key)).collect();
        for &key in &nonconfigurable {
            if !unchecked.remove(&key_id(heap, key)) {
                return type_error(agent);
            }
        }
        if extensible {
            return Ok(Step::Done(result(&keys)));
        }
        for &key in &configurable {
            if !unchecked.remove(&key_id(heap, key)) {
                return type_error(agent);
            }
        }
        if !unchecked.is_empty() {
            return type_error(agent);
        }
        Ok(Step::Done(result(&keys)))
    })
}

/// [[Call]] (Abschnitt 10.5.12).
pub(crate) fn proxy_call<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    cursor: Local<'_>,
    this: Local<'_>,
    arguments: &[Local<'_>],
) -> Completion<Step<Local<'s>>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "apply")? else {
            return Ok(Step::Forward);
        };
        let realm = current_realm(agent, &trap);
        let array = create_array_from_list(agent, inner, realm, arguments)?;
        let result = call(
            agent,
            inner,
            trap.function,
            trap.handler,
            &[trap.target, this, array],
        )?;
        let heap = agent.heap();
        Ok(Step::Done(heap.local(scope, heap.get(result))))
    })
}

/// [[Construct]] (Abschnitt 10.5.13); `current` ist der aktuelle Realm.
pub(crate) fn proxy_construct<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    current: RealmId,
    cursor: Local<'_>,
    arguments: &[Local<'_>],
    new_target: Local<'_>,
) -> Completion<Step<Local<'s>>> {
    step(agent, |agent, inner| {
        let Some(trap) = trap(agent, inner, cursor, "construct")? else {
            return Ok(Step::Forward);
        };
        let array = create_array_from_list(agent, inner, current, arguments)?;
        let result = call(
            agent,
            inner,
            trap.function,
            trap.handler,
            &[trap.target, array, new_target],
        )?;
        let heap = agent.heap();
        if !is_object(heap.get(result)) {
            return type_error(agent);
        }
        Ok(Step::Done(heap.local(scope, heap.get(result))))
    })
}
