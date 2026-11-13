//! Operationen auf Objekten (Abschnitt 7.3), IsArray (Abschnitt 7.2.2) und
//! IsCallable (Abschnitt 7.2.3).
//!
//! Call und Construct aus Rust-Code beginnen je einen neuen Lauf des
//! Interpreters.
//!
//! HostEnsureCanAddPrivateElement (Abschnitt 7.3.29) lehnt nur für
//! Exoten des Hosts ab, wie `WindowProxy` und `Location` im HTML Standard;
//! solche Objekte gibt es noch nicht, darum fehlt der Schritt.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::conversion::to_length;
use crate::descriptor::PropertyDescriptor;
use crate::function::is_constructor;
use crate::heap::{Heap, Local, ObjectClass, Scope};
use crate::interpreter::{call_function, construct_function};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{
    self, PrivateElement, array_create, has_call, private_element_find, private_element_write,
    proxy_target,
};
use crate::realm::{Intrinsic, RealmId};
use crate::reference::{get_value, to_object_base};
use crate::value::{Kind, Value};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// IsArray (Abschnitt 7.2.2): ein Array oder ein Proxy, dessen Ziel eines
/// ist, über eine Kette in einer Schleife. Ein widerrufener Proxy darin
/// wirft einen TypeError.
pub fn is_array(agent: &mut Agent, value: Local<'_>) -> Completion<bool> {
    match array_status(agent.heap(), agent.heap().get(value)) {
        Some(found) => Ok(found),
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// [`is_array`] ohne den Fehler: `None` für einen widerrufenen Proxy.
fn array_status(heap: &Heap, value: Value<'_>) -> Option<bool> {
    let mut current = match value.kind() {
        Kind::Object(object) => object,
        _ => return Some(false),
    };
    loop {
        match heap.object_class(current) {
            Some(ObjectClass::Array) => return Some(true),
            Some(ObjectClass::Proxy) => match proxy_target(heap, Value::object(current))?.kind() {
                Kind::Object(target) => current = target,
                _ => return None,
            },
            _ => return Some(false),
        }
    }
}

/// IsCallable (Abschnitt 7.2.3): ECMAScript-Funktionsobjekte, eingebaute
/// und gebundene Funktionen und Proxys um eine aufrufbare Funktion.
pub fn is_callable(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        Kind::Object(object) => match heap.object_class(object) {
            Some(ObjectClass::Function | ObjectClass::Builtin | ObjectClass::BoundFunction) => true,
            Some(ObjectClass::Proxy) => has_call(heap, value),
            Some(
                ObjectClass::Ordinary
                | ObjectClass::Array
                | ObjectClass::Arguments
                | ObjectClass::Error
                | ObjectClass::Primitive
                | ObjectClass::Iterator
                | ObjectClass::Generator
                | ObjectClass::Promise
                | ObjectClass::AsyncFromSyncIterator
                | ObjectClass::AsyncGenerator
                | ObjectClass::Collection,
            )
            | None => false,
        },
        _ => false,
    }
}

/// Call (Abschnitt 7.3.13) aus Rust-Code: ein neuer Lauf des Interpreters.
pub fn call<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    function: Local<'_>,
    this: Local<'_>,
    arguments: &[Local<'_>],
) -> Completion<Local<'s>> {
    if !is_callable(agent.heap(), agent.heap().get(function)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    call_function(agent, scope, function, this, arguments)
}

/// Construct (Abschnitt 7.3.14) aus Rust-Code; ohne `new_target` gilt
/// `constructor` selbst. `current` ist der aktuelle Realm. Die
/// Spezifikation setzt voraus, dass `constructor` ein Konstruktor ist;
/// sonst wirft das einen TypeError.
pub(crate) fn construct<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    current: RealmId,
    constructor: Local<'_>,
    arguments: &[Local<'_>],
    new_target: Option<Local<'_>>,
) -> Completion<Local<'s>> {
    if !is_constructor(agent, agent.heap().get(constructor)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let new_target = new_target.unwrap_or(constructor);
    construct_function(agent, scope, current, constructor, arguments, new_target)
}

/// Invoke (Abschnitt 7.3.20) mit GetV (Abschnitt 7.3.3): Ein primitiver
/// Wert liest über den Prototyp seines Wrappers aus `realm`, dem
/// aktuellen Realm.
pub(crate) fn invoke<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'_>,
    key: PropertyKey<'_>,
    arguments: &[Local<'_>],
) -> Completion<Local<'s>> {
    let base = to_object_base(agent, scope, realm, value)?;
    let function = get_value(agent, scope, base, value, key)?;
    call(agent, scope, function, value, arguments)
}

/// SpeciesConstructor (Abschnitt 7.3.22) für das Objekt `object`.
pub(crate) fn species_constructor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    default: Local<'s>,
) -> Completion<Local<'s>> {
    let units: Vec<u16> = "constructor".encode_utf16().collect();
    let key = agent.key_from_units(scope, &units)?;
    let constructor = get(agent, scope, object, key)?;
    let value = agent.heap().get(constructor);
    if value.is_undefined() {
        return Ok(default);
    }
    if !matches!(value.kind(), Kind::Object(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::Species);
    let species = get(agent, scope, constructor, PropertyKey::Symbol(symbol))?;
    let value = agent.heap().get(species);
    if value.is_undefined() || value.is_null() {
        return Ok(default);
    }
    if is_constructor(agent, value) {
        return Ok(species);
    }
    Err(agent.throw_error(ErrorKind::TypeError))
}

/// CreateArrayFromList (Abschnitt 7.3.17) mit %Array.prototype% aus
/// `realm`, dem aktuellen Realm.
pub(crate) fn create_array_from_list<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    elements: &[Local<'_>],
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    let array = array_create(agent, scope, 0, Some(prototype))?;
    for (index, &element) in elements.iter().enumerate() {
        // Mehr Elemente als Indizes bis 2³² − 2 kann kein Array halten.
        let Some(index) = u32::try_from(index).ok().filter(|&index| index < u32::MAX) else {
            return Err(agent.throw_error(ErrorKind::RangeError));
        };
        create_data_property_or_throw(agent, scope, array, PropertyKey::Index(index), element)?;
    }
    Ok(array)
}

/// Get (Abschnitt 7.3.2).
pub fn get<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Local<'s>> {
    object::get(agent, scope, object, key, object)
}

/// GetMethod (Abschnitt 7.3.10) für ein Objekt: `None` steht für
/// *undefined*. Für andere Werte bräuchte GetV (Abschnitt 7.3.3) ToObject;
/// alle Aufrufer übergeben Objekte, für andere Werte wirft es einen
/// TypeError.
pub fn get_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Option<Local<'s>>> {
    let function = get(agent, scope, object, key)?;
    let value = agent.heap().get(function);
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    if !is_callable(agent.heap(), value) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(Some(function))
}

/// Set (Abschnitt 7.3.4).
pub fn set(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    value: Local<'_>,
    throw: bool,
) -> Completion<()> {
    let success = object::set(agent, scope, object, key, value, object)?;
    if !success && throw {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(())
}

/// CreateDataProperty (Abschnitt 7.3.5).
pub fn create_data_property(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    value: Local<'_>,
) -> Completion<bool> {
    let descriptor = PropertyDescriptor::data(value, true, true, true);
    object::define_own_property(agent, scope, object, key, &descriptor)
}

/// CreateDataPropertyOrThrow (Abschnitt 7.3.6).
pub fn create_data_property_or_throw(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    value: Local<'_>,
) -> Completion<()> {
    if !create_data_property(agent, scope, object, key, value)? {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(())
}

/// CreateNonEnumerableDataPropertyOrThrow (Abschnitt 7.3.7). Der Aufrufer
/// hat ein gewöhnliches, erweiterbares Objekt ohne nicht konfigurierbare
/// Eigenschaften; dann gelingt das Anlegen immer.
pub fn create_non_enumerable_data_property_or_throw(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    value: Local<'_>,
) -> Completion<()> {
    let descriptor = PropertyDescriptor::data(value, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// DefinePropertyOrThrow (Abschnitt 7.3.8).
pub fn define_property_or_throw(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<()> {
    if !object::define_own_property(agent, scope, object, key, descriptor)? {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(())
}

/// DeletePropertyOrThrow (Abschnitt 7.3.9).
pub fn delete_property_or_throw(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<()> {
    if !object::delete(agent, scope, object, key)? {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(())
}

/// HasProperty (Abschnitt 7.3.11).
pub fn has_property(
    agent: &mut Agent,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<bool> {
    object::has_property(agent, object, key)
}

/// HasOwnProperty (Abschnitt 7.3.12).
pub fn has_own_property(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<bool> {
    Ok(object::get_own_property(agent, scope, object, key)?.is_some())
}

/// LengthOfArrayLike (Abschnitt 7.3.19).
pub(crate) fn length_of_array_like(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
) -> Completion<f64> {
    let key = agent.key_from_units(scope, &units("length"))?;
    let length = get(agent, scope, object, key)?;
    to_length(agent, scope, length)
}

/// CreateListFromArrayLike (Abschnitt 7.3.20) mit beliebigen Elementen.
/// Jedes Element kostet einen Arbeitsschritt, damit eine riesige `length`
/// am Budget scheitert.
pub(crate) fn create_list_from_array_like<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
) -> Completion<Vec<Local<'s>>> {
    if !matches!(agent.heap().get(value).kind(), Kind::Object(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let length = length_of_array_like(agent, scope, value)?;
    let mut list = Vec::new();
    let mut index: u32 = 0;
    while f64::from(index) < length {
        agent.spend_execution_steps(1)?;
        list.push(get(agent, scope, value, PropertyKey::Index(index))?);
        // Mehr Elemente als Indizes kann kein Array halten.
        index = index
            .checked_add(1)
            .ok_or_else(|| agent.throw_error(ErrorKind::RangeError))?;
    }
    Ok(list)
}

/// Die Stufen von SetIntegrityLevel und TestIntegrityLevel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntegrityLevel {
    Sealed,
    Frozen,
}

/// SetIntegrityLevel (Abschnitt 7.3.15).
pub(crate) fn set_integrity_level(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    level: IntegrityLevel,
) -> Completion<bool> {
    if !object::prevent_extensions(agent, object)? {
        return Ok(false);
    }
    let keys = object::own_property_keys(agent, scope, object)?;
    for key in keys {
        agent.scope(|agent, inner| -> Completion<()> {
            let descriptor = match level {
                IntegrityLevel::Sealed => PropertyDescriptor {
                    configurable: Some(false),
                    ..PropertyDescriptor::default()
                },
                IntegrityLevel::Frozen => {
                    let Some(current) = object::get_own_property(agent, inner, object, key)? else {
                        return Ok(());
                    };
                    PropertyDescriptor {
                        writable: if current.is_accessor() {
                            None
                        } else {
                            Some(false)
                        },
                        configurable: Some(false),
                        ..PropertyDescriptor::default()
                    }
                }
            };
            define_property_or_throw(agent, inner, object, key, &descriptor)
        })?;
    }
    Ok(true)
}

/// TestIntegrityLevel (Abschnitt 7.3.16).
pub(crate) fn test_integrity_level(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    level: IntegrityLevel,
) -> Completion<bool> {
    if object::is_extensible(agent, object)? {
        return Ok(false);
    }
    let keys = object::own_property_keys(agent, scope, object)?;
    for key in keys {
        let fixed = agent.scope(|agent, inner| -> Completion<bool> {
            let Some(current) = object::get_own_property(agent, inner, object, key)? else {
                return Ok(true);
            };
            if current.configurable == Some(true) {
                return Ok(false);
            }
            if level == IntegrityLevel::Frozen
                && current.is_data()
                && current.writable == Some(true)
            {
                return Ok(false);
            }
            Ok(true)
        })?;
        if !fixed {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Was EnumerableOwnProperties liefert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnumerableKind {
    Key,
    Value,
    KeyValue,
}

/// EnumerableOwnProperties (Abschnitt 7.3.24); die Paare für
/// [`EnumerableKind::KeyValue`] sind Arrays aus `realm`, dem aktuellen
/// Realm. Schlüssel kommen als Zeichenketten.
pub(crate) fn enumerable_own_properties<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    object: Local<'_>,
    kind: EnumerableKind,
) -> Completion<Vec<Local<'s>>> {
    let keys = object::own_property_keys(agent, scope, object)?;
    let mut results = Vec::new();
    for key in keys {
        if matches!(key, PropertyKey::Symbol(_)) {
            continue;
        }
        let Some(descriptor) = object::get_own_property(agent, scope, object, key)? else {
            continue;
        };
        if descriptor.enumerable != Some(true) {
            continue;
        }
        match kind {
            EnumerableKind::Key => results.push(agent.key_as_value(scope, key)?),
            EnumerableKind::Value => results.push(get(agent, scope, object, key)?),
            EnumerableKind::KeyValue => {
                let name = agent.key_as_value(scope, key)?;
                let value = get(agent, scope, object, key)?;
                results.push(create_array_from_list(agent, scope, realm, &[name, value])?);
            }
        }
    }
    Ok(results)
}

/// PrivateFieldAdd (Abschnitt 7.3.27).
pub(crate) fn private_field_add(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: Local<'_>,
    value: Local<'_>,
) -> Completion<()> {
    private_method_or_accessor_add(agent, scope, object, name, PrivateElement::Field(value))
}

/// PrivateMethodOrAccessorAdd (Abschnitt 7.3.28), mit einem Feld auch
/// PrivateFieldAdd: Gibt es den Namen am Objekt schon, ist das ein
/// TypeError.
pub(crate) fn private_method_or_accessor_add(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: Local<'_>,
    element: PrivateElement<'_>,
) -> Completion<()> {
    if private_element_find(agent.heap(), scope, object, name).is_some() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    private_element_write(agent, scope, object, name, element)?;
    Ok(())
}

/// PrivateGet (Abschnitt 7.3.30).
pub(crate) fn private_get<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    name: Local<'_>,
) -> Completion<Local<'s>> {
    match private_element_find(agent.heap(), scope, object, name) {
        Some(PrivateElement::Field(value) | PrivateElement::Method(value)) => Ok(value),
        Some(PrivateElement::Accessor { getter, .. })
            if !agent.heap().get(getter).is_undefined() =>
        {
            call(agent, scope, getter, object, &[])
        }
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// PrivateSet (Abschnitt 7.3.31).
pub(crate) fn private_set(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: Local<'_>,
    value: Local<'_>,
) -> Completion<()> {
    match private_element_find(agent.heap(), scope, object, name) {
        Some(PrivateElement::Field(_)) => {
            private_element_write(agent, scope, object, name, PrivateElement::Field(value))?;
            Ok(())
        }
        Some(PrivateElement::Accessor { setter, .. })
            if !agent.heap().get(setter).is_undefined() =>
        {
            call(agent, scope, setter, object, &[value])?;
            Ok(())
        }
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}
