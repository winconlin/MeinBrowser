//! Die Tabelle hinter [[MapData]] und [[SetData]] (Abschnitte 24.1 und
//! 24.2) und was Maps und Sets teilen: das Objekt mit RequireInternalSlot,
//! die Iteratoren und `forEach`. Konstruktoren und Prototypen stehen in
//! [`super::map`] und [`super::set`].
//!
//! Die Spezifikation beschreibt [[MapData]] als Liste: Gelöschte Einträge
//! werden ~empty~, neue kommen hinten dazu, und Iteratoren und `forEach`
//! laufen mit einem Index darüber. Verlangt ist ein Zugriff, der im Mittel
//! unter linearer Zeit bleibt. Die Tabelle ist darum eine Hashtabelle mit
//! Ketten nach dem Muster der deterministischen Hashtabellen: Die Einträge
//! liegen in der Reihenfolge, in der sie dazukamen, in einer Zelle aus
//! Werten, je Eintrag Schlüssel, Wert, Hashwert und der nächste Eintrag
//! seiner Kette; eine zweite Zelle hält die Anfänge der Ketten. Ein
//! gelöschter Eintrag wird zur Lücke ([`Value::HOLE`]) und bleibt in seiner
//! Kette. Erst wenn hinten kein Platz mehr ist, ziehen alle Einträge ohne
//! die Lücken in neue Zellen um, so groß, dass danach mindestens die Hälfte
//! frei ist.
//!
//! Wer über die Tabelle läuft, hält seine Stelle in einer eigenen kleinen
//! Zelle ([`cursor`]). Die Tabelle kennt diese Zellen über eine schwache
//! Zelle und rückt sie beim Umziehen um die weggefallenen Lücken nach vorn,
//! nach `clear` auf den Anfang. So sehen Iteratoren und `forEach` dieselben
//! Einträge wie mit dem Index der Spezifikation.
//!
//! Zahlen, Zeichenketten und die übrigen primitiven Werte haben einen
//! Hashwert aus ihrem Inhalt. Objekte und Symbole vergleicht SameValueZero
//! nach ihrer Identität, ihr Hashwert kommt aus ihrer Lage im Heap. Der
//! Collector verschiebt aber Zellen. Die Tabelle merkt sich darum, nach wie
//! vielen Sammlungen ihre Hashwerte berechnet wurden. Hat sie Schlüssel
//! dieser Art und wurde seitdem gesammelt, berechnet sie deren Hashwerte
//! und alle Ketten neu, bevor sie das nächste Mal sucht. Das kostet nicht
//! mehr als das Kopieren der Tabelle beim Sammeln. Die Hashwerte kommen aus
//! dem [`RandomState`] des Agents, die Reihenfolge der Einträge hängt nicht
//! davon ab.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use super::iterator::{
    INDEX, KIND, MAP_ENTRIES, MAP_KEYS, MAP_VALUES, SET_ENTRIES, SET_VALUES, TARGET,
    iterator_create,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::BuiltinCall;
use crate::compare::same_value_zero;
use crate::heap::{CellKind, Heap, Local, ObjectClass, OutOfMemory, Scope};
use crate::iterator::create_iterator_result_object;
use crate::object::INLINE;
use crate::operations::{call, create_array_from_list, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Ref, Value};

/// Die Art einer Sammlung im ersten internen Feld.
pub(super) const MAP: i32 = 0;
pub(super) const SET: i32 = 1;

/// Die internen Felder des Objekts: die Art und die Tabelle.
const COLLECTION_KIND: usize = 0;
const DATA: usize = 1;

/// Die Felder der Tabelle. Die Zellen für Einträge und Ketten fehlen
/// (*undefined*), solange die Tabelle leer ist.
const ENTRIES: usize = 0;
const BUCKETS: usize = 1;
/// Wie viele Einträge belegt sind, Lücken mitgezählt.
const USED: usize = 2;
/// Wie viele Einträge keine Lücken sind.
const SIZE: usize = 3;
/// Nach wie vielen Sammlungen die Hashwerte berechnet wurden.
const EPOCH: usize = 4;
/// Wie viele Schlüssel Objekte oder Symbole sind.
const IDENTITIES: usize = 5;
/// Die schwache Zelle mit den Zeigern und wie viele Plätze darin belegt
/// sind.
const CURSORS: usize = 6;
const CURSOR_COUNT: usize = 7;
const TABLE_FIELDS: usize = 8;

/// Ein Eintrag besteht aus vier Werten.
const WIDTH: usize = 4;
const KEY: usize = 0;
const VALUE: usize = 1;
/// Der Hashwert als Int32 mit denselben Bits.
const HASH: usize = 2;
/// Der nächste Eintrag der Kette, -1 am Ende.
const NEXT: usize = 3;

/// So viele Einträge fasst die Tabelle mindestens, wenn sie nicht leer ist.
/// Die Zahl der Ketten ist die Hälfte davon; beides sind Zweierpotenzen.
const MIN_CAPACITY: usize = 8;
const END: i32 = -1;

/// Die Felder eines Zeigers: die Stelle des nächsten Eintrags, -1, wenn
/// der Zeiger nicht mehr gebraucht wird, und für einen Iterator der Realm,
/// in dem seine Closure läuft.
const CURSOR_INDEX: usize = 0;
const CURSOR_REALM: usize = 1;
const CURSOR_FIELDS: usize = 2;

fn internal<'h>(value: Option<Value<'h>>) -> Option<Ref<'h>> {
    match value?.kind() {
        Kind::Internal(cell) => Some(cell),
        _ => None,
    }
}

/// Ein Feld mit einer Zahl ab 0, sonst `None`.
fn index_in(heap: &Heap, cell: Ref<'_>, index: usize) -> Option<usize> {
    heap.field(cell, index)
        .and_then(Value::as_int32)
        .and_then(|value| usize::try_from(value).ok())
}

fn count(heap: &Heap, cell: Ref<'_>, index: usize) -> usize {
    index_in(heap, cell, index).unwrap_or(0)
}

/// Eine Stelle oder Zahl als Wert. Die Tabelle hat höchstens so viele
/// Einträge, wie ihre Zelle mit `i32::MAX` Werten fasst ([`allocate`]).
fn int(value: usize) -> Value<'static> {
    Value::int32(i32::try_from(value).unwrap_or(i32::MAX))
}

/// Die Zahl der Sammlungen als Wert, genau bis 2⁵³.
fn epoch(heap: &Heap) -> Value<'static> {
    Value::number(heap.collections() as f64)
}

fn hash_value(hash: u32) -> Value<'static> {
    Value::int32(i32::from_ne_bytes(hash.to_ne_bytes()))
}

fn stored_hash(heap: &Heap, entries: Ref<'_>, base: usize) -> u32 {
    let bits = heap
        .field(entries, base + HASH)
        .and_then(Value::as_int32)
        .unwrap_or(0);
    u32::from_ne_bytes(bits.to_ne_bytes())
}

/// Ob der Hashwert des Schlüssels an seiner Lage im Heap hängt.
fn is_identity(key: Value<'_>) -> bool {
    matches!(
        key.kind(),
        Kind::Object(_) | Kind::Symbol(_) | Kind::BigInt(_)
    )
}

/// Der Hashwert eines Schlüssels. -0 und +0 haben denselben, ebenso alle
/// NaN und jede Zahl, ob als Int32 oder als Double gespeichert.
fn hash_of(heap: &Heap, hasher: &RandomState, key: Value<'_>) -> u32 {
    let mut state = hasher.build_hasher();
    match key.kind() {
        Kind::Undefined => state.write_u8(0),
        Kind::Null => state.write_u8(1),
        Kind::Boolean(b) => {
            state.write_u8(2);
            state.write_u8(u8::from(b));
        }
        Kind::Number(number) => {
            state.write_u8(3);
            let bits = if number.is_nan() {
                f64::NAN.to_bits()
            } else if number == 0.0 {
                0
            } else {
                number.to_bits()
            };
            state.write_u64(bits);
        }
        Kind::String(string) => {
            state.write_u8(4);
            for index in 0..heap.string_len(string) {
                state.write_u16(heap.string_unit(string, index).unwrap_or(0));
            }
        }
        Kind::Symbol(cell) | Kind::Object(cell) => {
            state.write_u8(5);
            state.write_u32(cell.offset());
        }
        // BigInt-Werte gibt es noch nicht. Mit ihnen kommt ein Hashwert aus
        // den Ziffern, denn SameValueZero vergleicht sie nach dem Wert.
        Kind::BigInt(cell) => {
            state.write_u8(6);
            state.write_u32(cell.offset());
        }
        // Lücken und interne Zellen sind nie Schlüssel.
        Kind::Hole | Kind::Internal(_) => state.write_u8(7),
    }
    let hash = state.finish();
    u32::try_from((hash ^ (hash >> 32)) & u64::from(u32::MAX)).unwrap_or(0)
}

/// CanonicalizeKeyedCollectionKey (Abschnitt 24.5.1): -0 wird +0.
pub(super) fn canonicalize<'s>(agent: &Agent, scope: &'s Scope, key: Local<'_>) -> Local<'s> {
    let heap = agent.heap();
    let value = heap.get(key);
    if value.as_number() == Some(0.0) {
        return heap.local(scope, Value::int32(0));
    }
    heap.local(scope, value)
}

/// Eine neue, leere Tabelle.
pub(super) fn table_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
) -> Result<Local<'s>, OutOfMemory> {
    let table = agent.heap_mut().alloc_slots(scope, TABLE_FIELDS)?;
    let heap = agent.heap();
    if let Some(cell) = internal(Some(heap.get(table))) {
        for index in [USED, SIZE, IDENTITIES, CURSOR_COUNT] {
            heap.set_field(cell, index, Value::int32(0));
        }
        heap.set_field(cell, EPOCH, epoch(heap));
    }
    Ok(table)
}

fn cell_of<'h>(heap: &'h Heap, local: Local<'_>) -> Option<Ref<'h>> {
    internal(Some(heap.get(local)))
}

/// SetDataSize (Abschnitt 24.2.1.5), auch für [[MapData]].
pub(super) fn size(agent: &Agent, table: Local<'_>) -> usize {
    let heap = agent.heap();
    cell_of(heap, table).map_or(0, |cell| count(heap, cell, SIZE))
}

/// Verteilt alle Einträge neu auf die Ketten, mit neuen Hashwerten für
/// Objekte und Symbole.
fn rechain(heap: &Heap, hasher: &RandomState, table: Ref<'_>) {
    let (Some(entries), Some(buckets)) = (
        internal(heap.field(table, ENTRIES)),
        internal(heap.field(table, BUCKETS)),
    ) else {
        return;
    };
    let chains = heap.field_count(buckets);
    let mask = chains.saturating_sub(1);
    for bucket in 0..chains {
        heap.set_field(buckets, bucket, Value::int32(END));
    }
    for index in 0..count(heap, table, USED) {
        let base = index * WIDTH;
        let Some(key) = heap.field(entries, base + KEY) else {
            break;
        };
        if key.is_hole() {
            continue;
        }
        let hash = if is_identity(key) {
            let hash = hash_of(heap, hasher, key);
            heap.set_field(entries, base + HASH, hash_value(hash));
            hash
        } else {
            stored_hash(heap, entries, base)
        };
        let bucket = usize::try_from(hash).unwrap_or(0) & mask;
        let head = heap.field(buckets, bucket).unwrap_or(Value::int32(END));
        heap.set_field(entries, base + NEXT, head);
        heap.set_field(buckets, bucket, int(index));
    }
    heap.set_field(table, EPOCH, epoch(heap));
}

/// Berechnet die Hashwerte von Objekten und Symbolen neu, wenn seit der
/// letzten Berechnung gesammelt wurde.
fn refresh(heap: &Heap, hasher: &RandomState, table: Ref<'_>) {
    if count(heap, table, IDENTITIES) == 0 {
        return;
    }
    let current = epoch(heap);
    if heap.field(table, EPOCH).map(Value::bits) != Some(current.bits()) {
        rechain(heap, hasher, table);
    }
}

/// Die Stelle des Eintrags mit dem Schlüssel `key` in der Kette zu `hash`.
fn lookup(heap: &Heap, table: Ref<'_>, hash: u32, key: Value<'_>) -> Option<usize> {
    let entries = internal(heap.field(table, ENTRIES))?;
    let buckets = internal(heap.field(table, BUCKETS))?;
    let mask = heap.field_count(buckets).checked_sub(1)?;
    let mut index = index_in(heap, buckets, usize::try_from(hash).ok()? & mask);
    while let Some(current) = index {
        let base = current.checked_mul(WIDTH)?;
        let stored = heap.field(entries, base + KEY)?;
        if stored_hash(heap, entries, base) == hash && same_value_zero(heap, stored, key) {
            return Some(current);
        }
        index = index_in(heap, entries, base + NEXT);
    }
    None
}

/// Die Stelle des Eintrags mit dem Schlüssel `key`, der schon kanonisch
/// ist ([`canonicalize`]), `None`, wenn es ihn nicht gibt.
pub(super) fn find(agent: &Agent, table: Local<'_>, key: Local<'_>) -> Option<usize> {
    let heap = agent.heap();
    let hasher = agent.collection_hasher();
    let cell = cell_of(heap, table)?;
    refresh(heap, hasher, cell);
    let key = heap.get(key);
    lookup(heap, cell, hash_of(heap, hasher, key), key)
}

fn entry_part<'s>(
    agent: &Agent,
    scope: &'s Scope,
    table: Local<'_>,
    index: usize,
    part: usize,
) -> Local<'s> {
    let heap = agent.heap();
    let value = cell_of(heap, table)
        .and_then(|cell| internal(heap.field(cell, ENTRIES)))
        .zip(
            index
                .checked_mul(WIDTH)
                .and_then(|base| base.checked_add(part)),
        )
        .and_then(|(entries, field)| heap.field(entries, field))
        .unwrap_or(Value::UNDEFINED);
    heap.local(scope, value)
}

/// Der Schlüssel des Eintrags an der Stelle `index`.
pub(super) fn key_at<'s>(
    agent: &Agent,
    scope: &'s Scope,
    table: Local<'_>,
    index: usize,
) -> Local<'s> {
    entry_part(agent, scope, table, index, KEY)
}

/// Der Wert des Eintrags an der Stelle `index`.
pub(super) fn value_at<'s>(
    agent: &Agent,
    scope: &'s Scope,
    table: Local<'_>,
    index: usize,
) -> Local<'s> {
    entry_part(agent, scope, table, index, VALUE)
}

/// Ersetzt den Wert des Eintrags an der Stelle `index`.
pub(super) fn set_value_at(agent: &Agent, table: Local<'_>, index: usize, value: Local<'_>) {
    let heap = agent.heap();
    let entries = cell_of(heap, table).and_then(|cell| internal(heap.field(cell, ENTRIES)));
    let field = index
        .checked_mul(WIDTH)
        .and_then(|base| base.checked_add(VALUE));
    if let (Some(entries), Some(field)) = (entries, field) {
        heap.set_field(entries, field, heap.get(value));
    }
}

/// Zellen für `capacity` Einträge und halb so viele Ketten.
fn allocate<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    capacity: usize,
) -> Result<(Local<'s>, Local<'s>), OutOfMemory> {
    let length = capacity
        .checked_mul(WIDTH)
        .filter(|&length| i32::try_from(length).is_ok())
        .ok_or(OutOfMemory)?;
    let entries = agent.heap_mut().alloc_slots(scope, length)?;
    let buckets = agent.heap_mut().alloc_slots(scope, capacity / 2)?;
    Ok((entries, buckets))
}

/// Wie viele Einträge eine neue Zelle für `size` Einträge fassen soll.
fn capacity_for(size: usize) -> Result<usize, OutOfMemory> {
    size.checked_mul(2)
        .and_then(usize::checked_next_power_of_two)
        .map(|capacity| capacity.max(MIN_CAPACITY))
        .ok_or(OutOfMemory)
}

/// Kopiert die Einträge von `source` ohne Lücken in die neuen Zellen
/// `entries` und `buckets` und macht sie zu denen von `target`. Mit `remap`
/// (`source` ist `target`) rücken die Zeiger nach.
fn transfer(
    heap: &Heap,
    hasher: &RandomState,
    source: Ref<'_>,
    target: Ref<'_>,
    (entries, buckets): (Ref<'_>, Ref<'_>),
    remap: bool,
) {
    let used = count(heap, source, USED);
    let mut before = Vec::new();
    let mut live = 0;
    let mut identities = 0;
    if let Some(old) = internal(heap.field(source, ENTRIES)) {
        for index in 0..used {
            if remap {
                before.push(live);
            }
            let base = index * WIDTH;
            let key = heap.field(old, base + KEY).unwrap_or(Value::HOLE);
            if key.is_hole() {
                continue;
            }
            let to = live * WIDTH;
            heap.set_field(entries, to + KEY, key);
            for part in [VALUE, HASH] {
                let value = heap.field(old, base + part).unwrap_or(Value::UNDEFINED);
                heap.set_field(entries, to + part, value);
            }
            if is_identity(key) {
                identities += 1;
            }
            live += 1;
        }
    }
    heap.set_field(target, ENTRIES, Value::internal(entries));
    heap.set_field(target, BUCKETS, Value::internal(buckets));
    heap.set_field(target, USED, int(live));
    heap.set_field(target, SIZE, int(live));
    heap.set_field(target, IDENTITIES, int(identities));
    rechain(heap, hasher, target);
    if remap {
        before.push(live);
        move_cursors(heap, target, |index| {
            before.get(index).copied().unwrap_or(live)
        });
    }
}

/// Zieht die Einträge in Zellen für `capacity` Einträge um.
fn rebuild(
    agent: &mut Agent,
    scope: &Scope,
    table: Local<'_>,
    capacity: usize,
) -> Result<(), OutOfMemory> {
    let (entries, buckets) = allocate(agent, scope, capacity)?;
    // Erst nach den Anforderungen lesen, sie können verschoben haben.
    let heap = agent.heap();
    let hasher = agent.collection_hasher();
    let (Some(cell), Some(entries), Some(buckets)) = (
        cell_of(heap, table),
        cell_of(heap, entries),
        cell_of(heap, buckets),
    ) else {
        return Err(OutOfMemory);
    };
    transfer(heap, hasher, cell, cell, (entries, buckets), true);
    Ok(())
}

/// Hängt einen Eintrag an. Der Schlüssel ist kanonisch und noch nicht in
/// der Tabelle.
pub(super) fn append(
    agent: &mut Agent,
    scope: &Scope,
    table: Local<'_>,
    key: Local<'_>,
    value: Local<'_>,
) -> Result<(), OutOfMemory> {
    let (used, capacity, size) = {
        let heap = agent.heap();
        let cell = cell_of(heap, table).ok_or(OutOfMemory)?;
        let capacity = internal(heap.field(cell, ENTRIES))
            .map_or(0, |entries| heap.field_count(entries) / WIDTH);
        (count(heap, cell, USED), capacity, count(heap, cell, SIZE))
    };
    if used >= capacity {
        rebuild(agent, scope, table, capacity_for(size)?)?;
    }
    let heap = agent.heap();
    let hasher = agent.collection_hasher();
    let cell = cell_of(heap, table).ok_or(OutOfMemory)?;
    refresh(heap, hasher, cell);
    let (Some(entries), Some(buckets)) = (
        internal(heap.field(cell, ENTRIES)),
        internal(heap.field(cell, BUCKETS)),
    ) else {
        return Err(OutOfMemory);
    };
    let key = heap.get(key);
    let hash = hash_of(heap, hasher, key);
    let index = count(heap, cell, USED);
    let base = index * WIDTH;
    let bucket = usize::try_from(hash).unwrap_or(0) & heap.field_count(buckets).saturating_sub(1);
    let head = heap.field(buckets, bucket).unwrap_or(Value::int32(END));
    heap.set_field(entries, base + KEY, key);
    heap.set_field(entries, base + VALUE, heap.get(value));
    heap.set_field(entries, base + HASH, hash_value(hash));
    heap.set_field(entries, base + NEXT, head);
    heap.set_field(buckets, bucket, int(index));
    heap.set_field(cell, USED, int(index + 1));
    heap.set_field(cell, SIZE, int(count(heap, cell, SIZE) + 1));
    if is_identity(key) {
        let identities = count(heap, cell, IDENTITIES);
        if identities == 0 {
            // Die übrigen Hashwerte hängen an keiner Lage; ab jetzt gilt
            // diese Sammlung.
            heap.set_field(cell, EPOCH, epoch(heap));
        }
        heap.set_field(cell, IDENTITIES, int(identities + 1));
    }
    Ok(())
}

/// Macht den Eintrag mit dem kanonischen Schlüssel `key` zur Lücke.
/// `false`, wenn es ihn nicht gibt.
pub(super) fn delete(agent: &Agent, table: Local<'_>, key: Local<'_>) -> bool {
    let Some(index) = find(agent, table, key) else {
        return false;
    };
    let heap = agent.heap();
    let Some(cell) = cell_of(heap, table) else {
        return false;
    };
    let Some(entries) = internal(heap.field(cell, ENTRIES)) else {
        return false;
    };
    let base = index * WIDTH;
    let identity = heap.field(entries, base + KEY).is_some_and(is_identity);
    heap.set_field(entries, base + KEY, Value::HOLE);
    heap.set_field(entries, base + VALUE, Value::UNDEFINED);
    heap.set_field(cell, SIZE, int(count(heap, cell, SIZE).saturating_sub(1)));
    if identity {
        let identities = count(heap, cell, IDENTITIES).saturating_sub(1);
        heap.set_field(cell, IDENTITIES, int(identities));
    }
    true
}

/// Leert die Tabelle. Alle Zeiger stehen danach am Anfang.
pub(super) fn clear(agent: &Agent, table: Local<'_>) {
    let heap = agent.heap();
    let Some(cell) = cell_of(heap, table) else {
        return;
    };
    heap.set_field(cell, ENTRIES, Value::UNDEFINED);
    heap.set_field(cell, BUCKETS, Value::UNDEFINED);
    for index in [USED, SIZE, IDENTITIES] {
        heap.set_field(cell, index, Value::int32(0));
    }
    move_cursors(heap, cell, |_| 0);
}

/// Eine neue Tabelle mit denselben Einträgen ohne Lücken, für „a copy of
/// [[SetData]]“.
pub(super) fn copy<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    table: Local<'_>,
) -> Result<Local<'s>, OutOfMemory> {
    let result = table_create(agent, scope)?;
    let size = size(agent, table);
    if size == 0 {
        return Ok(result);
    }
    let (entries, buckets) = allocate(agent, scope, capacity_for(size)?)?;
    let heap = agent.heap();
    let hasher = agent.collection_hasher();
    let (Some(source), Some(target), Some(entries), Some(buckets)) = (
        cell_of(heap, table),
        cell_of(heap, result),
        cell_of(heap, entries),
        cell_of(heap, buckets),
    ) else {
        return Err(OutOfMemory);
    };
    transfer(heap, hasher, source, target, (entries, buckets), false);
    Ok(result)
}

// Zeiger

/// Ob der Wert ein Zeiger ist, der noch gebraucht wird.
fn attached(heap: &Heap, value: Value<'_>) -> bool {
    internal(Some(value)).is_some_and(|cursor| index_in(heap, cursor, CURSOR_INDEX).is_some())
}

/// Setzt jeden Zeiger, der noch gebraucht wird, auf `to(Stelle)`.
fn move_cursors(heap: &Heap, table: Ref<'_>, to: impl Fn(usize) -> usize) {
    let Some(list) = internal(heap.field(table, CURSORS)) else {
        return;
    };
    for slot in 0..count(heap, table, CURSOR_COUNT) {
        let Some(cursor) = internal(heap.field(list, slot)) else {
            continue;
        };
        if let Some(index) = index_in(heap, cursor, CURSOR_INDEX) {
            heap.set_field(cursor, CURSOR_INDEX, int(to(index)));
        }
    }
}

/// Ein neuer Zeiger am Anfang der Tabelle. `realm` ist für einen Iterator
/// der Realm seiner Closure.
pub(super) fn cursor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    table: Local<'_>,
    realm: RealmId,
) -> Completion<Local<'s>> {
    // So viele Realms passen nicht in den Speicher.
    let realm =
        i32::try_from(realm.index()).map_err(|_| agent.throw_error(ErrorKind::RangeError))?;
    let cursor = agent.heap_mut().alloc_slots(scope, CURSOR_FIELDS)?;
    {
        let heap = agent.heap();
        if let Some(cell) = cell_of(heap, cursor) {
            heap.set_field(cell, CURSOR_INDEX, Value::int32(0));
            heap.set_field(cell, CURSOR_REALM, Value::int32(realm));
        }
    }
    register(agent, scope, table, cursor)?;
    Ok(cursor)
}

/// Trägt `cursor` in die schwache Zelle der Tabelle ein. Ist sie voll,
/// fallen erst Zeiger weg, die nicht mehr gebraucht werden; reicht das
/// nicht, kommt eine doppelt so große.
fn register(
    agent: &mut Agent,
    scope: &Scope,
    table: Local<'_>,
    cursor: Local<'_>,
) -> Result<(), OutOfMemory> {
    let length = {
        let heap = agent.heap();
        let cell = cell_of(heap, table).ok_or(OutOfMemory)?;
        let mut used = count(heap, cell, CURSOR_COUNT);
        let list = internal(heap.field(cell, CURSORS));
        let length = list.map_or(0, |list| heap.field_count(list));
        if let Some(list) = list {
            if used >= length {
                used = prune(heap, list, used);
            }
            if used < length {
                heap.set_field(list, used, heap.get(cursor));
                heap.set_field(cell, CURSOR_COUNT, int(used + 1));
                return Ok(());
            }
        }
        heap.set_field(cell, CURSOR_COUNT, int(used));
        length
    };
    let length = length.checked_mul(2).ok_or(OutOfMemory)?.max(4);
    let fresh = agent.heap_mut().alloc_cell(scope, CellKind::Weak, length)?;
    let heap = agent.heap();
    let (Some(cell), Some(fresh)) = (cell_of(heap, table), cell_of(heap, fresh)) else {
        return Err(OutOfMemory);
    };
    let mut kept = 0;
    if let Some(list) = internal(heap.field(cell, CURSORS)) {
        for slot in 0..count(heap, cell, CURSOR_COUNT) {
            let value = heap.field(list, slot).unwrap_or(Value::UNDEFINED);
            if attached(heap, value) {
                heap.set_field(fresh, kept, value);
                kept += 1;
            }
        }
    }
    heap.set_field(fresh, kept, heap.get(cursor));
    heap.set_field(cell, CURSORS, Value::internal(fresh));
    heap.set_field(cell, CURSOR_COUNT, int(kept + 1));
    Ok(())
}

/// Rückt die Zeiger, die noch gebraucht werden, an den Anfang der Zelle.
/// Die Zahl der übrigen.
fn prune(heap: &Heap, list: Ref<'_>, used: usize) -> usize {
    let mut kept = 0;
    for slot in 0..used {
        let value = heap.field(list, slot).unwrap_or(Value::UNDEFINED);
        if attached(heap, value) {
            heap.set_field(list, kept, value);
            kept += 1;
        }
    }
    for slot in kept..used {
        heap.set_field(list, slot, Value::UNDEFINED);
    }
    kept
}

/// Die Stelle des nächsten Eintrags ab dem Zeiger, der dahinter rückt;
/// `None` am Ende. Kommen später Einträge dazu, findet ihn der nächste
/// Aufruf.
pub(super) fn step(agent: &Agent, table: Local<'_>, cursor: Local<'_>) -> Option<usize> {
    let heap = agent.heap();
    let cell = cell_of(heap, table)?;
    let cursor = cell_of(heap, cursor)?;
    let start = index_in(heap, cursor, CURSOR_INDEX)?;
    let used = count(heap, cell, USED);
    let entries = internal(heap.field(cell, ENTRIES));
    for index in start..used {
        let key = entries.and_then(|entries| heap.field(entries, index * WIDTH + KEY));
        if key.is_some_and(|key| !key.is_hole()) {
            heap.set_field(cursor, CURSOR_INDEX, int(index + 1));
            return Some(index);
        }
    }
    heap.set_field(cursor, CURSOR_INDEX, int(used.max(start)));
    None
}

/// Gibt einen Zeiger auf; die Tabelle rückt ihn nicht mehr nach.
pub(super) fn detach(agent: &Agent, cursor: Local<'_>) {
    let heap = agent.heap();
    if let Some(cursor) = cell_of(heap, cursor) {
        heap.set_field(cursor, CURSOR_INDEX, Value::int32(END));
    }
}

fn cursor_realm(agent: &Agent, cursor: Local<'_>) -> RealmId {
    let heap = agent.heap();
    let index = cell_of(heap, cursor).map_or(0, |cell| count(heap, cell, CURSOR_REALM));
    RealmId::from_index(index)
}

// Objekte

/// Eine Map oder ein Set der Art `kind` mit `prototype` und der Tabelle
/// `table`, für OrdinaryCreateFromConstructor und OrdinaryObjectCreate mit
/// « [[MapData]] » beziehungsweise « [[SetData]] ».
pub(super) fn collection_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
    kind: i32,
    table: Local<'_>,
) -> Result<Local<'s>, OutOfMemory> {
    let shape = agent.empty_shape(scope);
    let object = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::Collection,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    if let Kind::Object(cell) = heap.get(object).kind() {
        heap.set_collection_field(cell, COLLECTION_KIND, Value::int32(kind));
        heap.set_collection_field(cell, DATA, heap.get(table));
    }
    Ok(object)
}

/// RequireInternalSlot (Abschnitt 7.3.32) mit [[MapData]] für `kind`
/// [`MAP`], sonst [[SetData]]: die Tabelle.
pub(super) fn this_table<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
    kind: i32,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let table = match heap.get(value).kind() {
        Kind::Object(object) => {
            let found = heap.collection_field(object, COLLECTION_KIND);
            if found.and_then(Value::as_int32) == Some(kind) {
                heap.collection_field(object, DATA)
            } else {
                None
            }
        }
        _ => None,
    };
    match table {
        Some(table) => Ok(heap.local(scope, table)),
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// CreateMapIterator (Abschnitt 24.1.5.1) und CreateSetIterator (Abschnitt
/// 24.2.6.1) mit der Art `kind` aus [`super::iterator`]. Die Closure der
/// Spezifikation ist ein Iterator mit der Tabelle und einem Zeiger; ihr
/// Realm ist der aktuelle, `realm`.
pub(super) fn create_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'_>,
    kind: i32,
) -> Completion<Local<'s>> {
    let (collection, prototype) = if matches!(kind, MAP_KEYS | MAP_VALUES | MAP_ENTRIES) {
        (MAP, Intrinsic::MapIteratorPrototype)
    } else {
        (SET, Intrinsic::SetIteratorPrototype)
    };
    let table = this_table(agent, scope, value, collection)?;
    let prototype = agent.intrinsic(scope, realm, prototype);
    let iterator = iterator_create(agent, scope, prototype, table, kind)?;
    let cursor = cursor(agent, scope, table, realm)?;
    let heap = agent.heap();
    if let Kind::Object(object) = heap.get(iterator).kind() {
        heap.set_iterator_field(object, INDEX, heap.get(cursor));
    }
    Ok(iterator)
}

/// %MapIteratorPrototype%.next (Abschnitt 24.1.5.2.1) und
/// %SetIteratorPrototype%.next (Abschnitt 24.2.6.2.1): GeneratorResume mit
/// der Marke der Art `collection`. Die Closure ruft keinen Code der Sprache
/// auf, ~executing~ kommt also nicht vor.
pub(super) fn iterator_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    collection: i32,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let fields = match heap.get(arguments.this).kind() {
        Kind::Object(object) => heap.iterator_field(object, KIND).and_then(|kind| {
            let kind = kind.as_int32()?;
            let wanted = match collection {
                MAP => matches!(kind, MAP_KEYS | MAP_VALUES | MAP_ENTRIES),
                _ => matches!(kind, SET_VALUES | SET_ENTRIES),
            };
            wanted.then_some((
                kind,
                heap.iterator_field(object, TARGET)?,
                heap.iterator_field(object, INDEX)?,
            ))
        }),
        _ => None,
    };
    let Some((kind, table, cursor)) = fields else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let table = heap.local(scope, table);
    let cursor = heap.local(scope, cursor);
    // Der Realm der Closure; ein fertiger Iterator hat keinen mehr.
    let realm = if heap.get(table).is_undefined() {
        arguments.realm
    } else {
        cursor_realm(agent, cursor)
    };
    let index = if agent.heap().get(table).is_undefined() {
        None
    } else {
        step(agent, table, cursor)
    };
    let Some(index) = index else {
        detach(agent, cursor);
        let heap = agent.heap();
        if let Kind::Object(object) = heap.get(arguments.this).kind() {
            heap.set_iterator_field(object, TARGET, Value::UNDEFINED);
            heap.set_iterator_field(object, INDEX, Value::UNDEFINED);
        }
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        return create_iterator_result_object(agent, scope, realm, undefined, true);
    };
    let key = key_at(agent, scope, table, index);
    let result = match kind {
        MAP_KEYS | SET_VALUES => key,
        MAP_VALUES => value_at(agent, scope, table, index),
        SET_ENTRIES => create_array_from_list(agent, scope, realm, &[key, key])?,
        _ => {
            let value = value_at(agent, scope, table, index);
            create_array_from_list(agent, scope, realm, &[key, value])?
        }
    };
    create_iterator_result_object(agent, scope, realm, result, false)
}

/// Map.prototype.forEach (Abschnitt 24.1.3.5) und Set.prototype.forEach
/// (Abschnitt 24.2.4.7). Ein Arbeitsschritt je Eintrag.
pub(super) fn for_each<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    kind: i32,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    let table = this_table(agent, scope, this, kind)?;
    let callback = arguments.argument(agent, scope, 0);
    if !is_callable(agent.heap(), agent.heap().get(callback)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let this_argument = arguments.argument(agent, scope, 1);
    let cursor = cursor(agent, scope, table, arguments.realm)?;
    let result = loop {
        let done = agent.scope(|agent, inner| -> Completion<bool> {
            agent.spend_execution_steps(1)?;
            let Some(index) = step(agent, table, cursor) else {
                return Ok(true);
            };
            let key = key_at(agent, inner, table, index);
            let value = if kind == MAP {
                value_at(agent, inner, table, index)
            } else {
                key
            };
            call(agent, inner, callback, this_argument, &[value, key, this])?;
            Ok(false)
        });
        match done {
            Ok(false) => {}
            Ok(true) => break Ok(()),
            Err(abrupt) => break Err(abrupt),
        }
    };
    detach(agent, cursor);
    result?;
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}
