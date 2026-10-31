//! Gestalten. Eine Gestalt nennt die benannten Eigenschaften eines Objekts
//! in der Reihenfolge, in der sie entstanden sind, mit ihren Attributen.
//! Sie ist eine Kette: Jede Gestalt fügt ihrer Elterngestalt genau einen
//! Schlüssel hinzu, die leere Gestalt ist die Wurzel. Die Eigenschaft an
//! Stelle `n` der Kette liegt im Platz `n` des Objekts.
//!
//! Objekte, die dieselben Schlüssel mit denselben Attributen in derselben
//! Reihenfolge bekommen, teilen ihre Gestalt, denn jede Gestalt merkt sich
//! ihre Übergänge in einer schwachen Tabelle. Eine Gestalt, die kein Objekt
//! mehr nutzt, fällt dadurch beim Sammeln weg.

use crate::heap::{CellKind, Heap, Local, OutOfMemory, Scope};
use crate::value::{Kind, Ref, Value};

const PARENT: usize = 0;
const KEY: usize = 1;
const ATTRIBUTES: usize = 2;
/// Die Länge der Kette bis hier, 0 für die leere Gestalt.
const COUNT: usize = 3;
const TRANSITIONS: usize = 4;
const FIELDS: usize = 5;

/// Ein Eintrag der Übergangstabelle: Schlüssel, Attribute, Folgegestalt.
const ENTRY: usize = 3;
const ENTRY_CHILD: usize = 2;

/// Der Verweis auf eine interne Zelle.
pub(crate) fn internal(value: Value<'_>) -> Option<Ref<'_>> {
    match value.kind() {
        Kind::Internal(cell) => Some(cell),
        _ => None,
    }
}

fn int32(heap: &Heap, cell: Ref<'_>, index: usize) -> usize {
    heap.field(cell, index)
        .and_then(Value::as_int32)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn int32_value(value: usize) -> Result<Value<'static>, OutOfMemory> {
    // Mehr Eigenschaften fasst das Speicherbudget nicht.
    i32::try_from(value)
        .map(Value::int32)
        .map_err(|_| OutOfMemory)
}

pub(crate) fn alloc_empty<'s>(heap: &mut Heap, scope: &'s Scope) -> Result<Local<'s>, OutOfMemory> {
    let shape = heap.alloc_cell(scope, CellKind::Shape, FIELDS)?;
    if let Some(cell) = internal(heap.get(shape)) {
        heap.set_field(cell, ATTRIBUTES, Value::int32(0));
        heap.set_field(cell, COUNT, Value::int32(0));
    }
    Ok(shape)
}

/// Die Zahl der Eigenschaften.
pub(crate) fn count(heap: &Heap, shape: Ref<'_>) -> usize {
    int32(heap, shape, COUNT)
}

fn attributes(heap: &Heap, shape: Ref<'_>) -> u32 {
    u32::try_from(int32(heap, shape, ATTRIBUTES)).unwrap_or(0)
}

fn parent<'h>(heap: &'h Heap, shape: Ref<'h>) -> Option<Ref<'h>> {
    internal(heap.field(shape, PARENT)?)
}

/// Stelle und Attribute der Eigenschaft mit dem Schlüssel `key`, einem
/// Atom oder Symbol.
pub(crate) fn find(heap: &Heap, shape: Ref<'_>, key: Value<'_>) -> Option<(usize, u32)> {
    let mut current = shape;
    loop {
        let count = count(heap, current);
        if count == 0 {
            return None;
        }
        if heap.field(current, KEY)?.bits() == key.bits() {
            return Some((count - 1, attributes(heap, current)));
        }
        current = parent(heap, current)?;
    }
}

/// Schlüssel und Attribute aller Eigenschaften in der Reihenfolge der
/// Kette.
pub(crate) fn entries<'h>(heap: &'h Heap, shape: Ref<'h>) -> Vec<(Value<'h>, u32)> {
    let mut entries = Vec::with_capacity(count(heap, shape));
    let mut current = Some(shape);
    while let Some(shape) = current.filter(|&shape| count(heap, shape) > 0) {
        let key = heap.field(shape, KEY).unwrap_or(Value::UNDEFINED);
        entries.push((key, attributes(heap, shape)));
        current = parent(heap, shape);
    }
    entries.reverse();
    entries
}

/// Die Gestalt, die zu `shape` den Schlüssel `key` mit den Attributen
/// `attributes` hinzufügt.
pub(crate) fn transition<'s>(
    heap: &mut Heap,
    scope: &'s Scope,
    shape: Local<'_>,
    key: Local<'_>,
    attributes: u32,
) -> Result<Local<'s>, OutOfMemory> {
    if let Some(child) = find_transition(heap, shape, key, attributes) {
        return Ok(heap.local(scope, child));
    }
    let child = heap.alloc_cell(scope, CellKind::Shape, FIELDS)?;
    let attributes_value = i32::try_from(attributes)
        .map(Value::int32)
        .map_err(|_| OutOfMemory)?;
    {
        let (Some(parent), Some(cell)) = (internal(heap.get(shape)), internal(heap.get(child)))
        else {
            return Err(OutOfMemory);
        };
        let count = int32_value(count(heap, parent).checked_add(1).ok_or(OutOfMemory)?)?;
        heap.set_field(cell, PARENT, heap.get(shape));
        heap.set_field(cell, KEY, heap.get(key));
        heap.set_field(cell, ATTRIBUTES, attributes_value);
        heap.set_field(cell, COUNT, count);
    }
    record_transition(heap, scope, shape, key, attributes_value, child)?;
    Ok(child)
}

fn find_transition<'h>(
    heap: &'h Heap,
    shape: Local<'_>,
    key: Local<'_>,
    attributes: u32,
) -> Option<Value<'h>> {
    let table = internal(heap.field(internal(heap.get(shape))?, TRANSITIONS)?)?;
    let key = heap.get(key).bits();
    let attributes = i32::try_from(attributes).ok()?;
    (0..heap.field_count(table) / ENTRY).find_map(|entry| {
        let base = entry * ENTRY;
        let child = heap.field(table, base + ENTRY_CHILD)?;
        let matches = internal(child).is_some()
            && heap.field(table, base)?.bits() == key
            && heap.field(table, base + 1)?.as_int32() == Some(attributes);
        matches.then_some(child)
    })
}

/// Trägt den Übergang in die Tabelle von `shape` ein, in einen freien
/// Eintrag oder in eine doppelt so große neue Tabelle.
fn record_transition(
    heap: &mut Heap,
    scope: &Scope,
    shape: Local<'_>,
    key: Local<'_>,
    attributes: Value<'static>,
    child: Local<'_>,
) -> Result<(), OutOfMemory> {
    let entry = |heap: &Heap, table: Ref<'_>, index: usize| {
        let base = index * ENTRY;
        heap.set_field(table, base, heap.get(key));
        heap.set_field(table, base + 1, attributes);
        heap.set_field(table, base + ENTRY_CHILD, heap.get(child));
    };
    let old_value = internal(heap.get(shape)).and_then(|shape| heap.field(shape, TRANSITIONS));
    let old = old_value.and_then(internal);
    if let Some(table) = old {
        let free = (0..heap.field_count(table) / ENTRY).find(|&index| {
            heap.field(table, index * ENTRY + ENTRY_CHILD)
                .and_then(internal)
                .is_none()
        });
        if let Some(index) = free {
            entry(heap, table, index);
            return Ok(());
        }
    }
    let old_entries = old.map_or(0, |table| heap.field_count(table) / ENTRY);
    let old = old_value
        .filter(|&value| internal(value).is_some())
        .map(|value| heap.local(scope, value));
    let capacity = old_entries.checked_mul(2).ok_or(OutOfMemory)?.max(2);
    let fields = capacity.checked_mul(ENTRY).ok_or(OutOfMemory)?;
    let table = heap.alloc_cell(scope, CellKind::Weak, fields)?;
    let Some(new) = internal(heap.get(table)) else {
        return Err(OutOfMemory);
    };
    let mut used = 0;
    if let Some(old) = old.and_then(|old| internal(heap.get(old))) {
        // Die Anforderung kann gesammelt und Einträge geleert haben.
        for index in 0..heap.field_count(old) / ENTRY {
            let base = index * ENTRY;
            if heap
                .field(old, base + ENTRY_CHILD)
                .and_then(internal)
                .is_none()
            {
                continue;
            }
            for offset in 0..ENTRY {
                let value = heap.field(old, base + offset).unwrap_or(Value::UNDEFINED);
                heap.set_field(new, used * ENTRY + offset, value);
            }
            used += 1;
        }
    }
    entry(heap, new, used);
    if let Some(shape) = internal(heap.get(shape)) {
        heap.set_field(shape, TRANSITIONS, heap.get(table));
    }
    Ok(())
}

/// Die Gestalt nach dem Löschen der Eigenschaft an Stelle `position`
/// (`attributes` ist `None`) oder mit neuen Attributen für sie. Die Kette
/// wird ab dieser Stelle neu aufgebaut, die übrigen Eigenschaften behalten
/// Reihenfolge und Attribute.
pub(crate) fn rebuild<'s>(
    heap: &mut Heap,
    scope: &'s Scope,
    shape: Local<'_>,
    position: usize,
    attributes: Option<u32>,
) -> Result<Local<'s>, OutOfMemory> {
    heap.scope(|heap, inner| {
        let (base, later) = {
            let Some(start) = internal(heap.get(shape)) else {
                return Err(OutOfMemory);
            };
            let mut later = Vec::new();
            let mut current = start;
            while count(heap, current) > position + 1 {
                let key = heap.field(current, KEY).unwrap_or(Value::UNDEFINED);
                later.push((heap.local(inner, key), self::attributes(heap, current)));
                current = parent(heap, current).ok_or(OutOfMemory)?;
            }
            if let Some(attributes) = attributes {
                let key = heap.field(current, KEY).unwrap_or(Value::UNDEFINED);
                later.push((heap.local(inner, key), attributes));
            }
            let base = heap.field(current, PARENT).unwrap_or(Value::UNDEFINED);
            (heap.local(inner, base), later)
        };
        let mut current = base;
        for &(key, attributes) in later.iter().rev() {
            current = transition(heap, inner, current, key, attributes)?;
        }
        Ok(heap.local(scope, heap.get(current)))
    })
}
