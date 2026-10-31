//! Arrays (Abschnitt 10.4.2). Ein Array ist ein gewöhnliches Objekt mit
//! eigener Art im Kopfwort und eigenem [[DefineOwnProperty]]. Seine
//! Eigenschaft *"length"* liegt wie jede andere benannte Eigenschaft in der
//! Gestalt, als erste und mit den Attributen, die ArrayCreate vergibt.

use super::{INLINE, delete, ordinary_define_own_property, ordinary_get_own_property, storage};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::conversion::{to_number, to_uint32};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, ObjectClass, OutOfMemory, Scope};
use crate::key::{PropertyKey, same_key};
use crate::value::Value;

/// Die Codeeinheiten von *"length"*.
const LENGTH: [u16; 6] = [0x6C, 0x65, 0x6E, 0x67, 0x74, 0x68];

fn length_key<'s>(agent: &mut Agent, scope: &'s Scope) -> Result<PropertyKey<'s>, OutOfMemory> {
    Ok(PropertyKey::String(agent.atom(scope, &LENGTH)?))
}

/// Der Wert einer Länge: eine ganze Zahl von 0 bis 2³² − 1.
fn length_value(value: Value<'_>) -> Option<u32> {
    let number = value.as_number()?;
    if number.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&number) {
        // Schneidet nicht ab: Bereich und Ganzzahligkeit sind geprüft.
        Some(number as u32)
    } else {
        None
    }
}

/// [[Value]] und [[Writable]] der Eigenschaft *"length"* mit ihrer
/// Beschreibung.
fn current_length<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    array: Local<'_>,
    key: PropertyKey<'_>,
) -> Option<(u32, bool, PropertyDescriptor<'s>)> {
    let heap = agent.heap();
    let descriptor = ordinary_get_own_property(heap, scope, array, key)?;
    let length = length_value(heap.get(descriptor.value?))?;
    Some((length, descriptor.writable == Some(true), descriptor))
}

/// ArrayCreate (Abschnitt 10.4.2.2). Bis es Realms gibt, nennt der Aufrufer
/// den Prototyp statt %Array.prototype%; `None` steht für *null*.
pub fn array_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    length: u64,
    prototype: Option<Local<'_>>,
) -> Completion<Local<'s>> {
    let Ok(length) = u32::try_from(length) else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    let shape = agent.empty_shape(scope);
    let array =
        agent
            .heap_mut()
            .alloc_object(scope, ObjectClass::Array, INLINE, shape, prototype)?;
    let key = length_key(agent, scope)?;
    let value = agent.heap().local(scope, Value::number(f64::from(length)));
    let descriptor = PropertyDescriptor::data(value, true, false, false);
    ordinary_define_own_property(agent, scope, array, key, &descriptor)?;
    Ok(array)
}

/// [[DefineOwnProperty]] eines Arrays (Abschnitt 10.4.2.1).
pub(super) fn define_own_property(
    agent: &mut Agent,
    scope: &Scope,
    array: Local<'_>,
    key: PropertyKey<'_>,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<bool> {
    let length_key = length_key(agent, scope)?;
    if same_key(agent.heap(), key, length_key) {
        return array_set_length(agent, scope, array, length_key, descriptor);
    }
    let PropertyKey::Index(index) = key else {
        return ordinary_define_own_property(agent, scope, array, key, descriptor);
    };
    let Some((length, writable, mut length_descriptor)) =
        current_length(agent, scope, array, length_key)
    else {
        // ArrayCreate legt die Länge immer an.
        return ordinary_define_own_property(agent, scope, array, key, descriptor);
    };
    if index >= length && !writable {
        return Ok(false);
    }
    if !ordinary_define_own_property(agent, scope, array, key, descriptor)? {
        return Ok(false);
    }
    if index >= length {
        // Ein Index ist höchstens 2³² − 2.
        let next = f64::from(index) + 1.0;
        length_descriptor.value = Some(agent.heap().local(scope, Value::number(next)));
        ordinary_define_own_property(agent, scope, array, length_key, &length_descriptor)?;
    }
    Ok(true)
}

/// ArraySetLength (Abschnitt 10.4.2.4).
fn array_set_length(
    agent: &mut Agent,
    scope: &Scope,
    array: Local<'_>,
    key: PropertyKey<'_>,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<bool> {
    let Some(value) = descriptor.value else {
        return ordinary_define_own_property(agent, scope, array, key, descriptor);
    };
    // Beide Umwandlungen rufen valueOf, wie es die Spezifikation verlangt.
    let new_length = to_uint32(agent, scope, value)?;
    let number_length = to_number(agent, scope, value)?;
    // SameValueZero: NaN ist ungleich jeder Länge, -0 gleich 0.
    if f64::from(new_length) != number_length {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let mut new_descriptor = PropertyDescriptor {
        value: Some(
            agent
                .heap()
                .local(scope, Value::number(f64::from(new_length))),
        ),
        ..*descriptor
    };
    let Some((old_length, old_writable, _)) = current_length(agent, scope, array, key) else {
        return ordinary_define_own_property(agent, scope, array, key, &new_descriptor);
    };
    if new_length >= old_length {
        return ordinary_define_own_property(agent, scope, array, key, &new_descriptor);
    }
    if !old_writable {
        return Ok(false);
    }
    // Schreibschutz erst am Ende setzen, falls ein Element bleibt.
    let new_writable = new_descriptor.writable != Some(false);
    if !new_writable {
        new_descriptor.writable = Some(true);
    }
    if !ordinary_define_own_property(agent, scope, array, key, &new_descriptor)? {
        return Ok(false);
    }
    let doomed: Vec<u32> = {
        let heap = agent.heap();
        storage::object_of(heap, array)
            .map(|object| storage::indices(heap, object))
            .unwrap_or_default()
            .into_iter()
            .filter(|&index| index >= new_length)
            .collect()
    };
    for &index in doomed.iter().rev() {
        if !delete(agent, scope, array, PropertyKey::Index(index))? {
            let kept = f64::from(index) + 1.0;
            new_descriptor.value = Some(agent.heap().local(scope, Value::number(kept)));
            if !new_writable {
                new_descriptor.writable = Some(false);
            }
            ordinary_define_own_property(agent, scope, array, key, &new_descriptor)?;
            return Ok(false);
        }
    }
    if !new_writable {
        let read_only = PropertyDescriptor {
            writable: Some(false),
            ..PropertyDescriptor::default()
        };
        ordinary_define_own_property(agent, scope, array, key, &read_only)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
