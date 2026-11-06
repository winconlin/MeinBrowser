//! Wrapper-Objekte für primitive Werte: Boolean-, Number-, String- und
//! Symbol-Objekte (Abschnitte 20.3.4, 21.1.4, 22.1.4 und 20.4.4). Sie sind
//! gewöhnliche Objekte mit einem internen Feld für den primitiven Wert,
//! also [[BooleanData]], [[NumberData]], [[StringData]] oder [[SymbolData]].
//!
//! String-Objekte sind exotisch (Abschnitt 10.4.3): Ihre Codeeinheiten sind
//! eigene Eigenschaften, aufzählbar, aber weder beschreibbar noch
//! konfigurierbar. StringCreate legt sie hier als gewöhnliche Elemente an,
//! statt sie wie StringGetOwnProperty bei jedem Zugriff zu bilden. Von außen
//! ist das nicht zu unterscheiden: [[GetOwnProperty]] findet dieselbe
//! Beschreibung; [[DefineOwnProperty]] kommt zum selben Ergebnis wie
//! IsCompatiblePropertyDescriptor und ändert nichts, denn die Eigenschaft
//! ist weder beschreibbar noch konfigurierbar; [[OwnPropertyKeys]] nennt
//! Indizes ohnehin aufsteigend vor allen anderen Schlüsseln. Es kostet aber
//! Speicher für jede Codeeinheit.

use super::INLINE;
use crate::agent::{Agent, Completion};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, ObjectClass, OutOfMemory, Scope};
use crate::key::PropertyKey;
use crate::operations::define_property_or_throw;
use crate::value::{Kind, Value};

/// Ein Wrapper-Objekt für den primitiven Wert `value` mit dem Prototyp
/// `prototype`. Für Zeichenketten ist es [`string_create`].
pub(crate) fn primitive_object_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
    prototype: Local<'_>,
) -> Result<Local<'s>, OutOfMemory> {
    let shape = agent.empty_shape(scope);
    let object = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::Primitive,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    if let Kind::Object(reference) = heap.get(object).kind() {
        heap.set_primitive_data(reference, heap.get(value));
    }
    Ok(object)
}

/// StringCreate (Abschnitt 10.4.3.4) mit den Codeeinheiten als Elementen.
pub(crate) fn string_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
    prototype: Local<'_>,
) -> Completion<Local<'s>> {
    let object = primitive_object_create(agent, scope, value, prototype)?;
    let length = match agent.heap().get(value).kind() {
        Kind::String(string) => agent.heap().string_len(string),
        _ => 0,
    };
    // Zeichenketten haben höchstens `u32::MAX` Codeeinheiten.
    let count = u32::try_from(length).unwrap_or(u32::MAX);
    let length_value = agent.heap().local(scope, Value::number(f64::from(count)));
    let units: Vec<u16> = "length".encode_utf16().collect();
    let key = agent.key_from_units(scope, &units)?;
    let descriptor = PropertyDescriptor::data(length_value, false, false, false);
    define_property_or_throw(agent, scope, object, key, &descriptor)?;
    for index in 0..count {
        agent.scope(|agent, inner| -> Completion<()> {
            let unit = match (agent.heap().get(value).kind(), usize::try_from(index)) {
                (Kind::String(string), Ok(position)) => agent.heap().string_unit(string, position),
                _ => None,
            };
            let Some(unit) = unit else {
                return Ok(());
            };
            let text = agent.heap_mut().alloc_string(inner, &[unit])?;
            let descriptor = PropertyDescriptor::data(text, false, true, false);
            define_property_or_throw(agent, inner, object, PropertyKey::Index(index), &descriptor)
        })?;
    }
    Ok(object)
}
