//! Operationen auf Objekten (Abschnitt 7.3), IsArray (Abschnitt 7.2.2) und
//! IsCallable (Abschnitt 7.2.3).

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Heap, Local, ObjectClass, Scope};
use crate::key::PropertyKey;
use crate::object;
use crate::value::{Kind, Value};

/// IsArray (Abschnitt 7.2.2). Proxys, die das Ergebnis ihres Ziels
/// liefern, kommen mit den Proxy-Objekten.
pub fn is_array(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        Kind::Object(object) => heap.object_class(object) == Some(ObjectClass::Array),
        _ => false,
    }
}

/// IsCallable (Abschnitt 7.2.3).
pub fn is_callable(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        // Gewöhnliche Objekte haben kein [[Call]]. Funktionsobjekte kommen
        // mit Schritt 4 des Entwurfs.
        Kind::Object(object) => match heap.object_class(object) {
            Some(ObjectClass::Ordinary | ObjectClass::Array) | None => false,
        },
        _ => false,
    }
}

/// Call (Abschnitt 7.3.13).
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
    // Bis es Funktionsobjekte gibt, ist nichts aufrufbar.
    let _ = (scope, this, arguments);
    Err(agent.throw_error(ErrorKind::TypeError))
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
/// *undefined*. Für andere Werte braucht GetV (Abschnitt 7.3.3) ToObject,
/// das mit den Realms kommt; bis dahin wirft es einen TypeError.
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
