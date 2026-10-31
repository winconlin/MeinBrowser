//! Referenzen auf Eigenschaften (Abschnitt 6.2.5) mit einem beliebigen
//! Wert als Basis: GetValue, PutValue und `delete` (Abschnitte 6.2.5.5,
//! 6.2.5.6 und 13.5.1.2), dazu CopyDataProperties (Abschnitt 7.3.25) und
//! InstanceofOperator (Abschnitt 13.10.2).
//!
//! Für einen primitiven Wert verlangen diese Schritte ToObject (Abschnitt
//! 7.1.20). Das Objekt daraus bleibt unsichtbar, die Anmerkungen zu
//! GetValue, PutValue und `delete` erlauben, es nicht anzulegen. Hier wird
//! es darum nur nachgebildet: Ein String-Objekt hat als eigene
//! Eigenschaften `length` und seine Codeeinheiten (StringCreate und
//! StringGetOwnProperty, Abschnitte 10.4.3.4 und 10.4.3.5), die übrigen
//! Wrapper haben keine, und alles andere liegt auf dem Prototyp des Realms.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::compare::same_value;
use crate::conversion::to_boolean;
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, WellKnownSymbol, same_key};
use crate::object::{self, get_prototype_of};
use crate::operations::{call, create_data_property_or_throw, get, get_method, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Wohin ToObject die Basis einer Referenz bringt.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Base<'s> {
    Object(Local<'s>),
    /// Ein primitiver Wert. Gehalten wird der Prototyp seines Wrappers.
    Primitive(Local<'s>),
}

/// Eine eigene Eigenschaft eines String-Objekts.
enum StringOwn {
    Length(usize),
    Unit(u16),
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

/// Der Schritt ToObject (Abschnitt 7.1.20) für die Basis `value`: ein
/// TypeError für *undefined* und *null*.
pub(crate) fn to_object_base<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'_>,
) -> Completion<Base<'s>> {
    let intrinsic = match agent.heap().get(value).kind() {
        Kind::Object(_) => {
            let heap = agent.heap();
            return Ok(Base::Object(heap.local(scope, heap.get(value))));
        }
        Kind::Boolean(_) => Intrinsic::BooleanPrototype,
        Kind::Number(_) => Intrinsic::NumberPrototype,
        Kind::String(_) => Intrinsic::StringPrototype,
        Kind::Symbol(_) => Intrinsic::SymbolPrototype,
        // BigInt-Werte gibt es noch nicht, Lücken und interne Zellen sehen
        // Skripte nie.
        Kind::Undefined | Kind::Null | Kind::BigInt(_) | Kind::Hole | Kind::Internal(_) => {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
    };
    Ok(Base::Primitive(agent.intrinsic(scope, realm, intrinsic)))
}

/// StringGetOwnProperty (Abschnitt 10.4.3.5) und die Eigenschaft `length`
/// aus StringCreate für das String-Objekt zu `value`; `None`, wenn `value`
/// keine Zeichenkette ist oder die Eigenschaft nicht hat.
fn string_own(
    agent: &mut Agent,
    scope: &Scope,
    value: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Option<StringOwn>> {
    let length = match agent.heap().get(value).kind() {
        Kind::String(string) => agent.heap().string_len(string),
        _ => return Ok(None),
    };
    Ok(match key {
        PropertyKey::Index(index) => {
            let heap = agent.heap();
            match (heap.get(value).kind(), usize::try_from(index)) {
                (Kind::String(string), Ok(index)) => {
                    heap.string_unit(string, index).map(StringOwn::Unit)
                }
                _ => None,
            }
        }
        PropertyKey::String(_) => {
            let name = agent.atom(scope, &units("length"))?;
            same_key(agent.heap(), key, PropertyKey::String(name))
                .then_some(StringOwn::Length(length))
        }
        PropertyKey::Symbol(_) => None,
    })
}

fn string_own_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    own: StringOwn,
) -> Completion<Local<'s>> {
    Ok(match own {
        StringOwn::Length(length) => {
            // Zeichenketten haben höchstens `u32::MAX` Codeeinheiten.
            let length = u32::try_from(length).unwrap_or(u32::MAX);
            agent.heap().local(scope, Value::number(f64::from(length)))
        }
        StringOwn::Unit(unit) => agent.heap_mut().alloc_string(scope, &[unit])?,
    })
}

/// GetValue für eine Eigenschaftsreferenz, nachdem ToObject die Basis
/// `this` zu `base` gemacht und ToPropertyKey den Schlüssel bestimmt hat:
/// [[Get]] mit `this` als Empfänger.
pub(crate) fn get_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    base: Base<'_>,
    this: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Local<'s>> {
    match base {
        Base::Object(object) => object::get(agent, scope, object, key, this),
        Base::Primitive(prototype) => {
            if let Some(own) = string_own(agent, scope, this, key)? {
                return string_own_value(agent, scope, own);
            }
            object::get(agent, scope, prototype, key, this)
        }
    }
}

/// PutValue für eine Eigenschaftsreferenz: [[Set]] mit `this` als
/// Empfänger, ein TypeError im strikten Modus, wenn das scheitert.
/// Eigene Eigenschaften eines String-Objekts sind nicht beschreibbar, und
/// ohne eigene Eigenschaft geht OrdinarySet zum Prototyp.
#[allow(clippy::too_many_arguments)]
pub(crate) fn put_value(
    agent: &mut Agent,
    scope: &Scope,
    base: Base<'_>,
    this: Local<'_>,
    key: PropertyKey<'_>,
    value: Local<'_>,
    strict: bool,
) -> Completion<()> {
    let succeeded = match base {
        Base::Object(object) => object::set(agent, scope, object, key, value, this)?,
        Base::Primitive(prototype) => {
            if string_own(agent, scope, this, key)?.is_some() {
                false
            } else {
                object::set(agent, scope, prototype, key, value, this)?
            }
        }
    };
    if !succeeded && strict {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(())
}

/// `delete` für eine Eigenschaftsreferenz. Das Objekt aus ToObject hat nur
/// die eigenen Eigenschaften eines String-Objekts, und die sind nicht
/// konfigurierbar.
pub(crate) fn delete_property(
    agent: &mut Agent,
    scope: &Scope,
    base: Base<'_>,
    this: Local<'_>,
    key: PropertyKey<'_>,
    strict: bool,
) -> Completion<bool> {
    let deleted = match base {
        Base::Object(object) => object::delete(agent, scope, object, key)?,
        Base::Primitive(_) => string_own(agent, scope, this, key)?.is_none(),
    };
    if !deleted && strict {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(deleted)
}

/// CopyDataProperties (Abschnitt 7.3.25) ohne ausgenommene Schlüssel, wie
/// es die Ausbreitung in Objektliteralen braucht.
pub(crate) fn copy_data_properties(
    agent: &mut Agent,
    scope: &Scope,
    target: Local<'_>,
    source: Local<'_>,
) -> Completion<()> {
    match agent.heap().get(source).kind() {
        Kind::Object(_) => {}
        Kind::String(string) => {
            // Die aufzählbaren eigenen Eigenschaften eines String-Objekts
            // sind seine Codeeinheiten, `length` ist nicht aufzählbar.
            let length = agent.heap().string_len(string);
            for index in 0..length {
                agent.scope(|agent, inner| -> Completion<()> {
                    // Zeichenketten haben höchstens `u32::MAX` Codeeinheiten,
                    // jede Stelle ist also ein Array-Index.
                    let Ok(key) = u32::try_from(index) else {
                        return Ok(());
                    };
                    let unit = match agent.heap().get(source).kind() {
                        Kind::String(string) => agent.heap().string_unit(string, index),
                        _ => None,
                    };
                    let Some(unit) = unit else {
                        return Ok(());
                    };
                    let value = agent.heap_mut().alloc_string(inner, &[unit])?;
                    create_data_property_or_throw(
                        agent,
                        inner,
                        target,
                        PropertyKey::Index(key),
                        value,
                    )
                })?;
            }
            return Ok(());
        }
        // *undefined* und *null* werden übergangen, die übrigen Wrapper
        // haben keine eigenen Eigenschaften.
        _ => return Ok(()),
    }
    let keys = object::own_property_keys(agent, scope, source)?;
    for key in keys {
        agent.scope(|agent, inner| -> Completion<()> {
            let Some(property) = object::get_own_property(agent, inner, source, key)? else {
                return Ok(());
            };
            if property.enumerable != Some(true) {
                return Ok(());
            }
            let value = get(agent, inner, source, key)?;
            create_data_property_or_throw(agent, inner, target, key, value)
        })?;
    }
    Ok(())
}

/// InstanceofOperator (Abschnitt 13.10.2).
pub(crate) fn instanceof_operator(
    agent: &mut Agent,
    scope: &Scope,
    value: Local<'_>,
    target: Local<'_>,
) -> Completion<bool> {
    if !is_object(agent, target) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::HasInstance);
    if let Some(handler) = get_method(agent, scope, target, PropertyKey::Symbol(symbol))? {
        let result = call(agent, scope, handler, target, &[value])?;
        let heap = agent.heap();
        return Ok(to_boolean(heap, heap.get(result)));
    }
    if !is_callable(agent.heap(), agent.heap().get(target)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    ordinary_has_instance(agent, scope, target, value)
}

/// OrdinaryHasInstance (Abschnitt 7.3.21). Gebundene Funktionen gibt es
/// noch nicht.
fn ordinary_has_instance(
    agent: &mut Agent,
    scope: &Scope,
    constructor: Local<'_>,
    instance: Local<'_>,
) -> Completion<bool> {
    if !is_callable(agent.heap(), agent.heap().get(constructor)) {
        return Ok(false);
    }
    if !is_object(agent, instance) {
        return Ok(false);
    }
    let key = agent.key_from_units(scope, &units("prototype"))?;
    let prototype = get(agent, scope, constructor, key)?;
    if !is_object(agent, prototype) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    // Ohne Rekursion und mit einem Handle für die ganze Kette (RS-8).
    let heap = agent.heap();
    let cursor = heap.local(scope, heap.get(instance));
    loop {
        let found = agent.scope(|agent, inner| -> Completion<Option<bool>> {
            let Some(next) = get_prototype_of(agent, inner, cursor)? else {
                return Ok(Some(false));
            };
            let heap = agent.heap();
            if same_value(heap, heap.get(prototype), heap.get(next)) {
                return Ok(Some(true));
            }
            heap.set(cursor, heap.get(next));
            Ok(None)
        })?;
        if let Some(found) = found {
            return Ok(found);
        }
    }
}

#[cfg(test)]
mod tests;
