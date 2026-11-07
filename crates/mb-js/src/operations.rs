//! Operationen auf Objekten (Abschnitt 7.3), IsArray (Abschnitt 7.2.2) und
//! IsCallable (Abschnitt 7.2.3).
//!
//! HostEnsureCanAddPrivateElement (Abschnitt 7.3.29) lehnt nur für
//! Exoten des Hosts ab, wie `WindowProxy` und `Location` im HTML Standard;
//! solche Objekte gibt es noch nicht, darum fehlt der Schritt.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Heap, Local, ObjectClass, Scope};
use crate::interpreter::call_function;
use crate::key::PropertyKey;
use crate::object::{self, PrivateElement, private_element_find, private_element_write};
use crate::value::{Kind, Value};

/// IsArray (Abschnitt 7.2.2). Proxys, die das Ergebnis ihres Ziels
/// liefern, kommen mit den Proxy-Objekten.
pub fn is_array(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        Kind::Object(object) => heap.object_class(object) == Some(ObjectClass::Array),
        _ => false,
    }
}

/// IsCallable (Abschnitt 7.2.3). Bisher haben ECMAScript-Funktionsobjekte
/// und eingebaute Funktionen [[Call]].
pub fn is_callable(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        Kind::Object(object) => match heap.object_class(object) {
            Some(ObjectClass::Function | ObjectClass::Builtin) => true,
            Some(
                ObjectClass::Ordinary
                | ObjectClass::Array
                | ObjectClass::Arguments
                | ObjectClass::Error
                | ObjectClass::Primitive
                | ObjectClass::Iterator
                | ObjectClass::Generator,
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
