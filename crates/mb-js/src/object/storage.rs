//! Plätze für benannte Eigenschaften und Elemente eines Objekts.
//!
//! Benannte Eigenschaften liegen in den eingebetteten Plätzen und danach in
//! einem Zusatzfeld, das bei Bedarf doppelt so groß neu angelegt wird.
//!
//! Elemente sind dicht oder dünn besetzt. Dichte Elemente sind ein Feld
//! von Werten, in dem die Marke „Loch“ fehlende Indizes kennzeichnet; sie
//! haben alle die Attribute, die CreateDataProperty vergibt. Dünn besetzte
//! Elemente sind eine nach Index sortierte Liste mit Attributen je
//! Eintrag. Ein Element mit anderen Attributen oder ein Index weit hinter
//! dem Ende macht die Elemente dünn besetzt; zurück geht es nicht.

use super::shape::internal;
use crate::heap::{CellKind, Heap, Local, OutOfMemory, Scope};
use crate::value::{Kind, Ref, Value};

pub(crate) const WRITABLE: u32 = 1;
pub(crate) const ENUMERABLE: u32 = 2;
pub(crate) const CONFIGURABLE: u32 = 4;
/// Eine Zugriffseigenschaft. Ihr Platz hält ein Paar aus Getter und Setter.
pub(crate) const ACCESSOR: u32 = 8;
/// Die Attribute dichter Elemente und der Eigenschaften aus
/// CreateDataProperty.
pub(crate) const DEFAULT: u32 = WRITABLE | ENUMERABLE | CONFIGURABLE;

/// So viele Löcher darf ein Schreiben hinter dichten Elementen höchstens
/// lassen. Ein Index weiter hinten macht die Elemente dünn besetzt.
const DENSE_GAP: usize = 1024;

/// Dünn besetzte Elemente: im ersten Feld die Zahl der Einträge, dann je
/// Eintrag Index, Attribute und Wert.
const SPARSE_COUNT: usize = 0;
const ENTRY: usize = 3;

pub(crate) fn object_of<'h>(heap: &'h Heap, local: Local<'_>) -> Option<Ref<'h>> {
    match heap.get(local).kind() {
        Kind::Object(object) => Some(object),
        _ => None,
    }
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

// Benannte Eigenschaften

pub(crate) fn slot<'h>(heap: &'h Heap, object: Ref<'h>, position: usize) -> Option<Value<'h>> {
    let inline = heap.inline_count(object);
    match position.checked_sub(inline) {
        None => heap.inline_slot(object, position),
        Some(index) => heap.field(heap.object_overflow(object)?, index),
    }
}

pub(crate) fn set_slot(heap: &Heap, object: Ref<'_>, position: usize, value: Value<'_>) -> bool {
    let inline = heap.inline_count(object);
    match position.checked_sub(inline) {
        None => heap.set_inline_slot(object, position, value),
        Some(index) => heap
            .object_overflow(object)
            .is_some_and(|overflow| heap.set_field(overflow, index, value)),
    }
}

/// Sorgt für Plätze für `needed` benannte Eigenschaften.
pub(crate) fn reserve(
    heap: &mut Heap,
    scope: &Scope,
    object: Local<'_>,
    needed: usize,
) -> Result<(), OutOfMemory> {
    let (inline, capacity) = {
        let object = object_of(heap, object).ok_or(OutOfMemory)?;
        let capacity = heap
            .object_overflow(object)
            .map_or(0, |overflow| heap.field_count(overflow));
        (heap.inline_count(object), capacity)
    };
    if needed <= inline.saturating_add(capacity) {
        return Ok(());
    }
    let size = (needed - inline)
        .max(capacity.checked_mul(2).ok_or(OutOfMemory)?)
        .max(4);
    let overflow = heap.alloc_cell(scope, CellKind::Slots, size)?;
    let (Some(object), Some(new)) = (object_of(heap, object), internal(heap.get(overflow))) else {
        return Err(OutOfMemory);
    };
    if let Some(old) = heap.object_overflow(object) {
        for index in 0..heap.field_count(old) {
            heap.set_field(
                new,
                index,
                heap.field(old, index).unwrap_or(Value::UNDEFINED),
            );
        }
    }
    heap.set_object_overflow(object, Some(new));
    Ok(())
}

/// Entfernt den Platz `position` von `count` belegten; die Plätze dahinter
/// rücken nach.
pub(crate) fn remove_slot(heap: &Heap, object: Ref<'_>, position: usize, count: usize) {
    for index in position..count.saturating_sub(1) {
        let next = slot(heap, object, index + 1).unwrap_or(Value::UNDEFINED);
        set_slot(heap, object, index, next);
    }
    if let Some(last) = count.checked_sub(1) {
        set_slot(heap, object, last, Value::UNDEFINED);
    }
}

/// Legt das Paar aus Getter und Setter einer Zugriffseigenschaft an.
pub(crate) fn alloc_pair<'s>(
    heap: &mut Heap,
    scope: &'s Scope,
    getter: Local<'_>,
    setter: Local<'_>,
) -> Result<Local<'s>, OutOfMemory> {
    let pair = heap.alloc_cell(scope, CellKind::Slots, 2)?;
    if let Some(cell) = internal(heap.get(pair)) {
        heap.set_field(cell, 0, heap.get(getter));
        heap.set_field(cell, 1, heap.get(setter));
    }
    Ok(pair)
}

/// Getter und Setter aus einem Paar.
pub(crate) fn pair<'h>(heap: &'h Heap, pair: Value<'h>) -> (Value<'h>, Value<'h>) {
    let field = |index| {
        internal(pair)
            .and_then(|cell| heap.field(cell, index))
            .unwrap_or(Value::UNDEFINED)
    };
    (field(0), field(1))
}

// Elemente

/// Der gespeicherte Wert (bei Zugriffseigenschaften das Paar) und die
/// Attribute des Elements `index`.
pub(crate) fn element<'h>(heap: &'h Heap, object: Ref<'h>, index: u32) -> Option<(Value<'h>, u32)> {
    let elements = heap.object_elements(object)?;
    match heap.cell_kind(elements)? {
        CellKind::Slots => {
            let value = heap.field(elements, to_usize(index))?;
            (!value.is_hole()).then_some((value, DEFAULT))
        }
        CellKind::Sparse => {
            let entry = search(heap, elements, index).ok()?;
            let base = 1 + entry * ENTRY;
            let attributes = heap.field(elements, base + 1).and_then(Value::as_int32)?;
            let value = heap.field(elements, base + 2)?;
            Some((value, u32::try_from(attributes).ok()?))
        }
        _ => None,
    }
}

/// Die Indizes aller Elemente, aufsteigend.
pub(crate) fn indices(heap: &Heap, object: Ref<'_>) -> Vec<u32> {
    let Some(elements) = heap.object_elements(object) else {
        return Vec::new();
    };
    match heap.cell_kind(elements) {
        Some(CellKind::Slots) => (0..heap.field_count(elements))
            .filter(|&index| {
                heap.field(elements, index)
                    .is_some_and(|value| !value.is_hole())
            })
            .filter_map(|index| u32::try_from(index).ok())
            .collect(),
        Some(CellKind::Sparse) => (0..sparse_count(heap, elements))
            .filter_map(|entry| entry_index(heap, elements, entry))
            .collect(),
        _ => Vec::new(),
    }
}

/// Index im Eintrag: das Bitmuster des `u32` als Int32. Skripte sehen
/// diese Liste nie.
fn index_value(index: u32) -> Value<'static> {
    Value::int32(i32::from_ne_bytes(index.to_ne_bytes()))
}

fn entry_index(heap: &Heap, elements: Ref<'_>, entry: usize) -> Option<u32> {
    let value = heap.field(elements, 1 + entry * ENTRY)?.as_int32()?;
    Some(u32::from_ne_bytes(value.to_ne_bytes()))
}

fn sparse_count(heap: &Heap, elements: Ref<'_>) -> usize {
    heap.field(elements, SPARSE_COUNT)
        .and_then(Value::as_int32)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(0)
}

fn sparse_capacity(heap: &Heap, elements: Ref<'_>) -> usize {
    heap.field_count(elements).saturating_sub(1) / ENTRY
}

fn set_sparse_count(heap: &Heap, elements: Ref<'_>, count: usize) -> Result<(), OutOfMemory> {
    let count = i32::try_from(count).map_err(|_| OutOfMemory)?;
    heap.set_field(elements, SPARSE_COUNT, Value::int32(count));
    Ok(())
}

/// Binäre Suche nach `index`: der Eintrag oder die Stelle, an die er
/// gehört.
fn search(heap: &Heap, elements: Ref<'_>, index: u32) -> Result<usize, usize> {
    let (mut low, mut high) = (0, sparse_count(heap, elements));
    while low < high {
        let middle = low + (high - low) / 2;
        match entry_index(heap, elements, middle) {
            Some(found) if found == index => return Ok(middle),
            Some(found) if found < index => low = middle + 1,
            _ => high = middle,
        }
    }
    Err(low)
}

/// Schreibt das Element `index` mit dem gespeicherten Wert `stored` (bei
/// Zugriffseigenschaften dem Paar) und den Attributen `attributes`.
pub(crate) fn put_element(
    heap: &mut Heap,
    scope: &Scope,
    object: Local<'_>,
    index: u32,
    stored: Local<'_>,
    attributes: u32,
) -> Result<(), OutOfMemory> {
    let (kind, length) = {
        let object = object_of(heap, object).ok_or(OutOfMemory)?;
        match heap.object_elements(object) {
            None => (None, 0),
            Some(elements) => (heap.cell_kind(elements), heap.field_count(elements)),
        }
    };
    let position = to_usize(index);
    if kind != Some(CellKind::Sparse) && attributes == DEFAULT {
        if position >= length && position < length.saturating_add(DENSE_GAP) {
            let grown = length
                .saturating_add(length / 2)
                .saturating_add(8)
                .max(position.saturating_add(1));
            grow_dense(heap, scope, object, grown)?;
        }
        let elements = object_of(heap, object).and_then(|object| heap.object_elements(object));
        if let Some(elements) = elements
            && heap.set_field(elements, position, heap.get(stored))
        {
            return Ok(());
        }
    }
    if kind != Some(CellKind::Sparse) {
        make_sparse(heap, scope, object)?;
    }
    put_sparse(heap, scope, object, index, stored, attributes)
}

fn attributes_value(attributes: u32) -> Result<Value<'static>, OutOfMemory> {
    i32::try_from(attributes)
        .map(Value::int32)
        .map_err(|_| OutOfMemory)
}

/// Vergrößert dichte Elemente auf `length` Plätze.
fn grow_dense(
    heap: &mut Heap,
    scope: &Scope,
    object: Local<'_>,
    length: usize,
) -> Result<(), OutOfMemory> {
    let elements = heap.alloc_cell(scope, CellKind::Slots, length)?;
    let (Some(object), Some(new)) = (object_of(heap, object), internal(heap.get(elements))) else {
        return Err(OutOfMemory);
    };
    let old = heap.object_elements(object);
    for index in 0..length {
        let value = old
            .and_then(|old| heap.field(old, index))
            .unwrap_or(Value::HOLE);
        heap.set_field(new, index, value);
    }
    heap.set_object_elements(object, Some(new));
    Ok(())
}

/// Macht dichte Elemente dünn besetzt.
fn make_sparse(heap: &mut Heap, scope: &Scope, object: Local<'_>) -> Result<(), OutOfMemory> {
    let present = object_of(heap, object).map_or(0, |object| indices(heap, object).len());
    let capacity = present.checked_mul(2).ok_or(OutOfMemory)?.max(4);
    let fields = capacity
        .checked_mul(ENTRY)
        .and_then(|fields| fields.checked_add(1))
        .ok_or(OutOfMemory)?;
    let elements = heap.alloc_cell(scope, CellKind::Sparse, fields)?;
    let (Some(object), Some(new)) = (object_of(heap, object), internal(heap.get(elements))) else {
        return Err(OutOfMemory);
    };
    let mut count = 0;
    if let Some(old) = heap.object_elements(object) {
        for index in indices(heap, object) {
            let value = heap.field(old, to_usize(index)).unwrap_or(Value::UNDEFINED);
            let base = 1 + count * ENTRY;
            heap.set_field(new, base, index_value(index));
            heap.set_field(new, base + 1, attributes_value(DEFAULT)?);
            heap.set_field(new, base + 2, value);
            count += 1;
        }
    }
    set_sparse_count(heap, new, count)?;
    heap.set_object_elements(object, Some(new));
    Ok(())
}

fn put_sparse(
    heap: &mut Heap,
    scope: &Scope,
    object: Local<'_>,
    index: u32,
    stored: Local<'_>,
    attributes: u32,
) -> Result<(), OutOfMemory> {
    let attributes = attributes_value(attributes)?;
    let (found, count, capacity) = {
        let object = object_of(heap, object).ok_or(OutOfMemory)?;
        let elements = heap.object_elements(object).ok_or(OutOfMemory)?;
        (
            search(heap, elements, index),
            sparse_count(heap, elements),
            sparse_capacity(heap, elements),
        )
    };
    let position = match found {
        Ok(entry) => {
            let elements = object_of(heap, object)
                .and_then(|object| heap.object_elements(object))
                .ok_or(OutOfMemory)?;
            let base = 1 + entry * ENTRY;
            heap.set_field(elements, base + 1, attributes);
            heap.set_field(elements, base + 2, heap.get(stored));
            return Ok(());
        }
        Err(position) => position,
    };
    if count >= capacity {
        let fields = capacity
            .checked_mul(2)
            .and_then(|capacity| capacity.max(4).checked_mul(ENTRY))
            .and_then(|fields| fields.checked_add(1))
            .ok_or(OutOfMemory)?;
        let elements = heap.alloc_cell(scope, CellKind::Sparse, fields)?;
        let (Some(object), Some(new)) = (object_of(heap, object), internal(heap.get(elements)))
        else {
            return Err(OutOfMemory);
        };
        if let Some(old) = heap.object_elements(object) {
            for field in 0..heap.field_count(old) {
                heap.set_field(
                    new,
                    field,
                    heap.field(old, field).unwrap_or(Value::UNDEFINED),
                );
            }
        }
        heap.set_object_elements(object, Some(new));
    }
    let elements = object_of(heap, object)
        .and_then(|object| heap.object_elements(object))
        .ok_or(OutOfMemory)?;
    for entry in (position..count).rev() {
        for offset in 0..ENTRY {
            let value = heap
                .field(elements, 1 + entry * ENTRY + offset)
                .unwrap_or(Value::UNDEFINED);
            heap.set_field(elements, 1 + (entry + 1) * ENTRY + offset, value);
        }
    }
    let base = 1 + position * ENTRY;
    heap.set_field(elements, base, index_value(index));
    heap.set_field(elements, base + 1, attributes);
    heap.set_field(elements, base + 2, heap.get(stored));
    set_sparse_count(heap, elements, count + 1)
}

/// Entfernt das Element `index`, falls es das gibt.
pub(crate) fn remove_element(heap: &Heap, object: Ref<'_>, index: u32) {
    let Some(elements) = heap.object_elements(object) else {
        return;
    };
    match heap.cell_kind(elements) {
        Some(CellKind::Slots) => {
            heap.set_field(elements, to_usize(index), Value::HOLE);
        }
        Some(CellKind::Sparse) => {
            let Ok(entry) = search(heap, elements, index) else {
                return;
            };
            let count = sparse_count(heap, elements);
            for next in entry + 1..count {
                for offset in 0..ENTRY {
                    let value = heap
                        .field(elements, 1 + next * ENTRY + offset)
                        .unwrap_or(Value::UNDEFINED);
                    heap.set_field(elements, 1 + (next - 1) * ENTRY + offset, value);
                }
            }
            for offset in 0..ENTRY {
                heap.set_field(elements, 1 + (count - 1) * ENTRY + offset, Value::UNDEFINED);
            }
            // Weniger Einträge passen immer.
            let _ = set_sparse_count(heap, elements, count - 1);
        }
        _ => {}
    }
}
