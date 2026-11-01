//! arguments-Objekte (Abschnitt 10.4.4).
//!
//! Beide Fassungen sind Objekte der Art [`ObjectClass::Arguments`]. Ohne
//! Verknüpfung (CreateUnmappedArgumentsObject, Abschnitt 10.4.4.6) ist ihr
//! internes Feld [[ParameterMap]] *undefined*. Ein verknüpftes Objekt
//! (CreateMappedArgumentsObject, Abschnitt 10.4.4.7) hält dort eine interne
//! Zelle: im ersten Feld den Kontext mit den Bindungen der Parameter,
//! danach für jeden Index den Platz seiner Bindung in diesem Kontext oder
//! *undefined*, wenn der Index nicht verknüpft ist. Statt der Getter und
//! Setter aus MakeArgGetter und MakeArgSetter liest und schreibt das Objekt
//! den Platz selbst, und statt einen Index aus der Zuordnung zu löschen,
//! setzt es ihn auf *undefined*.
//!
//! Ein verknüpfter Index ist immer eine beschreibbare Dateneigenschaft:
//! Wird er eine Zugriffseigenschaft oder schreibgeschützt, endet die
//! Verknüpfung. Darum genügt es für [[GetOwnProperty]], [[Get]] und [[Set]]
//! (Abschnitte 10.4.4.1, 10.4.4.3 und 10.4.4.4), dass die Suche nach einer
//! eigenen Eigenschaft den Wert aus der Bindung nimmt; [[Set]] mit dem
//! Objekt als Empfänger endet in [[DefineOwnProperty]] mit einem Wert, und
//! das schreibt die Bindung. [[DefineOwnProperty]] und [[Delete]]
//! (Abschnitte 10.4.4.2 und 10.4.4.5) stehen hier und in `object`.
//!
//! Abweichungen, bis es eingebaute Funktionen gibt: Die Eigenschaft
//! @@iterator mit %Array.prototype.values% fehlt, und `callee` eines
//! Objekts ohne Verknüpfung hat statt %ThrowTypeError% *undefined* als
//! Getter und Setter. Lesen ergibt darum *undefined* statt eines TypeError;
//! Schreiben im strikten Modus wirft ihn, wie es soll.

use super::{INLINE, ordinary_define_own_property, storage::object_of};
use crate::agent::{Agent, Completion};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Heap, Local, ObjectClass, OutOfMemory, Scope};
use crate::key::PropertyKey;
use crate::operations::{create_data_property_or_throw, define_property_or_throw};
use crate::value::{Kind, Ref, Value};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Die Zahl der Argumente in der internen Zelle `arguments`.
fn argument_count(heap: &Heap, arguments: Local<'_>) -> usize {
    match heap.get(arguments).kind() {
        Kind::Internal(cell) => heap.slot_count(cell),
        _ => 0,
    }
}

/// Ein neues arguments-Objekt ohne Eigenschaften, mit `prototype` als
/// %Object.prototype%.
fn alloc<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
) -> Result<Local<'s>, OutOfMemory> {
    let shape = agent.empty_shape(scope);
    agent.heap_mut().alloc_object(
        scope,
        ObjectClass::Arguments,
        INLINE,
        shape,
        Some(prototype),
    )
}

/// Die Argumente als Eigenschaften mit ihren Indizes.
fn define_indices(agent: &mut Agent, object: Local<'_>, arguments: Local<'_>) -> Completion<()> {
    let count = argument_count(agent.heap(), arguments);
    for index in 0..count {
        agent.scope(|agent, inner| -> Completion<()> {
            let heap = agent.heap();
            let value = match heap.get(arguments).kind() {
                Kind::Internal(cell) => heap.slot(cell, index),
                _ => None,
            };
            let value = heap.local(inner, value.unwrap_or(Value::UNDEFINED));
            // Es gibt höchstens so viele Argumente wie Register.
            let key = PropertyKey::Index(u32::try_from(index).map_err(|_| OutOfMemory)?);
            create_data_property_or_throw(agent, inner, object, key, value)
        })?;
    }
    Ok(())
}

/// *"length"* mit der Zahl der Argumente, beschreibbar und konfigurierbar.
fn define_length(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    arguments: Local<'_>,
) -> Completion<()> {
    let count = argument_count(agent.heap(), arguments);
    // Die Zahl ist klein genug für eine exakte Gleitkommazahl.
    let count = u32::try_from(count).map_err(|_| OutOfMemory)?;
    let value = agent.heap().local(scope, Value::number(f64::from(count)));
    let key = agent.key_from_units(scope, &units("length"))?;
    let descriptor = PropertyDescriptor::data(value, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// CreateUnmappedArgumentsObject (Abschnitt 10.4.4.6) für die Argumente in
/// der internen Zelle `arguments`, mit `prototype` als %Object.prototype%.
pub(crate) fn create_unmapped_arguments_object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
    arguments: Local<'_>,
) -> Completion<Local<'s>> {
    let object = alloc(agent, scope, prototype)?;
    define_length(agent, scope, object, arguments)?;
    define_indices(agent, object, arguments)?;
    // Abweichung: *undefined* statt %ThrowTypeError%.
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let key = agent.key_from_units(scope, &units("callee"))?;
    let descriptor = PropertyDescriptor::accessor(undefined, undefined, false, false);
    define_property_or_throw(agent, scope, object, key, &descriptor)?;
    Ok(object)
}

/// CreateMappedArgumentsObject (Abschnitt 10.4.4.7) für die Funktion
/// `function` und die Argumente in der internen Zelle `arguments`. Die
/// Bindungen der Parameter liegen im Kontext `context`, `map` nennt für
/// jeden Parameter ihren Platz darin, `None` für einen, dessen Name ein
/// späterer Parameter wiederholt.
pub(crate) fn create_mapped_arguments_object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
    function: Local<'_>,
    arguments: Local<'_>,
    context: Local<'_>,
    map: &[Option<u32>],
) -> Completion<Local<'s>> {
    let object = alloc(agent, scope, prototype)?;
    define_indices(agent, object, arguments)?;
    define_length(agent, scope, object, arguments)?;
    let count = argument_count(agent.heap(), arguments).min(map.len());
    if map.iter().take(count).any(Option::is_some) {
        let length = count.checked_add(1).ok_or(OutOfMemory)?;
        let cell = agent.heap_mut().alloc_slots(scope, length)?;
        let heap = agent.heap();
        if let (Kind::Internal(cell), Some(target)) =
            (heap.get(cell).kind(), object_of(heap, object))
        {
            heap.set_slot(cell, 0, heap.get(context));
            for (index, slot) in map.iter().take(count).enumerate() {
                if let Some(slot) = slot.and_then(|slot| i32::try_from(slot).ok()) {
                    heap.set_slot(cell, index.saturating_add(1), Value::int32(slot));
                }
            }
            heap.set_arguments_map(target, Value::internal(cell));
        }
    }
    let key = agent.key_from_units(scope, &units("callee"))?;
    let descriptor = PropertyDescriptor::data(function, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)?;
    Ok(object)
}

/// Der Kontext und der Platz der Bindung, mit der der Index `index` des
/// Objekts verknüpft ist.
fn mapped_place<'h>(heap: &'h Heap, object: Ref<'_>, index: u32) -> Option<(Ref<'h>, usize)> {
    let Kind::Internal(map) = heap.arguments_map(object)?.kind() else {
        return None;
    };
    let entry = usize::try_from(index).ok()?.checked_add(1)?;
    let slot = usize::try_from(heap.slot(map, entry)?.as_int32()?).ok()?;
    let Kind::Internal(context) = heap.slot(map, 0)?.kind() else {
        return None;
    };
    Some((context, slot))
}

/// Der Wert der Bindung, mit der der Index verknüpft ist, `None` ohne
/// Verknüpfung und für andere Objekte.
pub(super) fn mapped_value<'h>(heap: &'h Heap, object: Ref<'_>, index: u32) -> Option<Value<'h>> {
    let (context, slot) = mapped_place(heap, object, index)?;
    heap.slot(context, slot)
}

/// Beendet die Verknüpfung des Index, wenn es eine gibt.
pub(super) fn unmap(heap: &Heap, object: Ref<'_>, index: u32) {
    let Some(Kind::Internal(map)) = heap.arguments_map(object).map(|map| map.kind()) else {
        return;
    };
    if let Some(entry) = usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
    {
        heap.set_slot(map, entry, Value::UNDEFINED);
    }
}

/// [[DefineOwnProperty]] eines arguments-Objekts (Abschnitt 10.4.4.2).
pub(super) fn define_own_property(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<bool> {
    let PropertyKey::Index(index) = key else {
        return ordinary_define_own_property(agent, scope, object, key, descriptor);
    };
    let current = {
        let heap = agent.heap();
        object_of(heap, object)
            .and_then(|target| mapped_value(heap, target, index))
            .map(|value| heap.local(scope, value))
    };
    let Some(current) = current else {
        return ordinary_define_own_property(agent, scope, object, key, descriptor);
    };
    // Wird ein verknüpfter Index schreibgeschützt, behält er den Wert der
    // Bindung.
    let mut new_descriptor = *descriptor;
    if descriptor.is_data() && descriptor.value.is_none() && descriptor.writable == Some(false) {
        new_descriptor.value = Some(current);
    }
    if !ordinary_define_own_property(agent, scope, object, key, &new_descriptor)? {
        return Ok(false);
    }
    let heap = agent.heap();
    let Some(target) = object_of(heap, object) else {
        return Ok(true);
    };
    if descriptor.is_accessor() {
        unmap(heap, target, index);
        return Ok(true);
    }
    if let Some(value) = descriptor.value
        && let Some((context, slot)) = mapped_place(heap, target, index)
    {
        heap.set_slot(context, slot, heap.get(value));
    }
    if descriptor.writable == Some(false) {
        unmap(heap, target, index);
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
