//! EnumerateObjectProperties (Abschnitt 14.7.5.9) für `for`-`in`.
//!
//! Die Spezifikation beschreibt den Aufzähler als Generator. Hier ist er
//! eine interne Zelle ([`Heap::alloc_slots`](crate::heap::Heap::alloc_slots))
//! mit sechs Plätzen: das Objekt, dessen Schlüssel gerade an der Reihe
//! sind, am Ende *undefined*; die Zelle mit dessen Schlüsseln aus
//! [[OwnPropertyKeys]] ohne die Symbole; der Index darin; die Zelle mit den
//! schon besuchten Schlüsseln; deren Zahl; und die Tiefe in der
//! Prototypkette. Die Schlüssel eines Objekts holt er erst, wenn es an der
//! Reihe ist, und prüft jeden beim Besuch mit [[GetOwnProperty]]: Eine
//! inzwischen gelöschte Eigenschaft fällt weg, eine nicht aufzählbare zählt
//! als besucht und verdeckt so gleichnamige Eigenschaften der Prototypen.
//!
//! Indizes liegen in den Zellen als Zahlen, die übrigen Schlüssel als die
//! Atome aus der Gestalt des Objekts; so ist der Vergleich mit den besuchten
//! Schlüsseln ein Vergleich der Bits. Er ist linear in der Zahl der
//! besuchten Schlüssel, entfällt aber beim ersten Objekt, dessen eigene
//! Schlüssel ohnehin verschieden sind, und kostet Arbeitsschritte.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion};
use crate::conversion::to_object;
use crate::heap::{Local, OutOfMemory, Scope};
use crate::key::{MAX_ARRAY_INDEX, PropertyKey};
use crate::object::{get_own_property, get_prototype_of, own_property_keys};
use crate::realm::RealmId;
use crate::value::{Kind, Value};

/// Das Objekt, dessen Schlüssel an der Reihe sind, oder *undefined*.
const OBJECT: usize = 0;
/// Die Zelle mit seinen Schlüsseln, *undefined*, bis sie geholt sind.
const KEYS: usize = 1;
/// Der Index des nächsten Schlüssels in dieser Zelle.
const INDEX: usize = 2;
/// Die Zelle mit den besuchten Schlüsseln, *undefined* bis zum ersten.
const VISITED: usize = 3;
/// Wie viele davon belegt sind.
const VISITED_COUNT: usize = 4;
/// Wie viele Prototypen schon hinter dem Objekt liegen.
const DEPTH: usize = 5;
const FIELDS: usize = 6;
/// Die Größe der ersten Zelle für besuchte Schlüssel.
const FIRST_CAPACITY: usize = 8;
/// So viele Vergleiche mit besuchten Schlüsseln kosten einen Arbeitsschritt.
const COMPARISONS_PER_STEP: u64 = 64;

/// Ein neuer Aufzähler für `value`. *undefined* und *null* ergeben einen
/// fertigen (ForIn/OfHeadEvaluation, Abschnitt 14.7.5.6, Schritt 6.b),
/// alles andere zählt die Eigenschaften von ToObject auf.
pub(crate) fn for_in_prepare<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'_>,
) -> Completion<Local<'s>> {
    let enumerator = agent.heap_mut().alloc_slots(scope, FIELDS)?;
    let found = agent.heap().get(value);
    if found.is_undefined() || found.is_null() {
        return Ok(enumerator);
    }
    let object = to_object(agent, scope, realm, value)?;
    let heap = agent.heap();
    write(agent, enumerator, OBJECT, heap.get(object));
    write(agent, enumerator, INDEX, Value::number(0.0));
    write(agent, enumerator, VISITED_COUNT, Value::number(0.0));
    write(agent, enumerator, DEPTH, Value::number(0.0));
    Ok(enumerator)
}

/// Der nächste Schlüssel des Aufzählers als Zeichenkette, `None` am Ende.
pub(crate) fn for_in_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    enumerator: Local<'_>,
) -> Completion<Option<Local<'s>>> {
    loop {
        let object = {
            let heap = agent.heap();
            heap.local(scope, read(agent, enumerator, OBJECT))
        };
        if !matches!(agent.heap().get(object).kind(), Kind::Object(_)) {
            return Ok(None);
        }
        if read(agent, enumerator, KEYS).is_undefined() {
            load_keys(agent, enumerator, object)?;
        }
        let index = count(agent, enumerator, INDEX);
        let Some(key) = key_at(agent, enumerator, index) else {
            // Die Schlüssel dieses Objekts sind durch: weiter zum Prototyp.
            let prototype = get_prototype_of(agent, scope, object)?;
            let depth = count(agent, enumerator, DEPTH);
            let heap = agent.heap();
            let next = prototype.map_or(Value::UNDEFINED, |prototype| heap.get(prototype));
            write(agent, enumerator, OBJECT, next);
            write(agent, enumerator, KEYS, Value::UNDEFINED);
            write(agent, enumerator, INDEX, Value::number(0.0));
            write(agent, enumerator, DEPTH, as_number(depth.saturating_add(1)));
            continue;
        };
        write(agent, enumerator, INDEX, as_number(index.saturating_add(1)));
        agent.spend_execution_steps(1)?;
        if count(agent, enumerator, DEPTH) > 0 && visited(agent, enumerator, key)? {
            continue;
        }
        // Von hier an fordern Aufrufe Speicher an; der Schlüssel steht
        // danach noch an seinem Index in der Zelle.
        let key = agent.heap().local(scope, key);
        let property_key = match agent.heap().get(key).kind() {
            Kind::Number(number) => PropertyKey::Index(index_of(number)),
            Kind::String(_) => PropertyKey::String(key),
            _ => continue,
        };
        let Some(descriptor) = get_own_property(agent, scope, object, property_key)? else {
            continue;
        };
        push_visited(agent, scope, enumerator, index)?;
        if descriptor.enumerable == Some(true) {
            return Ok(Some(agent.key_as_value(scope, property_key)?));
        }
    }
}

/// Holt die Schlüssel von `object` ohne Symbole in eine neue Zelle.
fn load_keys(agent: &mut Agent, enumerator: Local<'_>, object: Local<'_>) -> Completion<()> {
    agent.scope(|agent, inner| -> Completion<()> {
        let keys = own_property_keys(agent, inner, object)?;
        let count = keys
            .iter()
            .filter(|key| !matches!(key, PropertyKey::Symbol(_)))
            .count();
        let cell = agent.heap_mut().alloc_slots(inner, count)?;
        let heap = agent.heap();
        if let Kind::Internal(target) = heap.get(cell).kind() {
            let mut position = 0;
            for key in &keys {
                let value = match key {
                    PropertyKey::Index(index) => Value::number(f64::from(*index)),
                    PropertyKey::String(local) => heap.get(*local),
                    PropertyKey::Symbol(_) => continue,
                };
                heap.set_slot(target, position, value);
                position = position.saturating_add(1);
            }
        }
        write(agent, enumerator, KEYS, heap.get(cell));
        write(agent, enumerator, INDEX, Value::number(0.0));
        Ok(())
    })
}

/// Ob `key` schon besucht wurde.
fn visited(agent: &Agent, enumerator: Local<'_>, key: Value<'_>) -> Completion<bool> {
    let count = count(agent, enumerator, VISITED_COUNT);
    let steps = u64::try_from(count)
        .unwrap_or(u64::MAX)
        .div_ceil(COMPARISONS_PER_STEP);
    agent.spend_execution_steps(steps)?;
    let heap = agent.heap();
    let Kind::Internal(cell) = read(agent, enumerator, VISITED).kind() else {
        return Ok(false);
    };
    Ok((0..count).any(|index| {
        heap.slot(cell, index)
            .is_some_and(|visited| visited.bits() == key.bits())
    }))
}

/// Merkt den Schlüssel am Index `index` als besucht; die Zelle dafür
/// wächst bei Bedarf aufs Doppelte.
fn push_visited(
    agent: &mut Agent,
    scope: &Scope,
    enumerator: Local<'_>,
    index: usize,
) -> Completion<()> {
    let count = count(agent, enumerator, VISITED_COUNT);
    let capacity = match read(agent, enumerator, VISITED).kind() {
        Kind::Internal(cell) => agent.heap().slot_count(cell),
        _ => 0,
    };
    if count >= capacity {
        let grown = capacity
            .checked_mul(2)
            .ok_or(OutOfMemory)?
            .max(FIRST_CAPACITY);
        let cell = agent.heap_mut().alloc_slots(scope, grown)?;
        let heap = agent.heap();
        if let (Kind::Internal(target), Kind::Internal(source)) = (
            heap.get(cell).kind(),
            read(agent, enumerator, VISITED).kind(),
        ) {
            for position in 0..count {
                if let Some(value) = heap.slot(source, position) {
                    heap.set_slot(target, position, value);
                }
            }
        }
        write(agent, enumerator, VISITED, heap.get(cell));
    }
    let heap = agent.heap();
    if let (Some(key), Kind::Internal(cell)) = (
        key_at(agent, enumerator, index),
        read(agent, enumerator, VISITED).kind(),
    ) {
        heap.set_slot(cell, count, key);
    }
    write(
        agent,
        enumerator,
        VISITED_COUNT,
        as_number(count.saturating_add(1)),
    );
    Ok(())
}

/// Der Schlüssel am Index `index` der Zelle mit den Schlüsseln.
fn key_at<'h>(agent: &'h Agent, enumerator: Local<'_>, index: usize) -> Option<Value<'h>> {
    match read(agent, enumerator, KEYS).kind() {
        Kind::Internal(cell) => agent.heap().slot(cell, index),
        _ => None,
    }
}

/// Der Platz `field` des Aufzählers, *undefined*, wenn es ihn nicht gibt.
/// Gilt bis zur nächsten Anforderung von Speicher.
fn read<'h>(agent: &'h Agent, enumerator: Local<'_>, field: usize) -> Value<'h> {
    let heap = agent.heap();
    match heap.get(enumerator).kind() {
        Kind::Internal(cell) => heap.slot(cell, field).unwrap_or(Value::UNDEFINED),
        _ => Value::UNDEFINED,
    }
}

fn write(agent: &Agent, enumerator: Local<'_>, field: usize, value: Value<'_>) {
    let heap = agent.heap();
    if let Kind::Internal(cell) = heap.get(enumerator).kind() {
        heap.set_slot(cell, field, value);
    }
}

/// Der Zähler im Platz `field`.
fn count(agent: &Agent, enumerator: Local<'_>, field: usize) -> usize {
    let number = read(agent, enumerator, field).as_number().unwrap_or(0.0);
    // Zähler entstehen aus `as_number` und bleiben ganz und klein.
    if number >= 0.0 && number.fract() == 0.0 {
        usize::try_from(number as u64).unwrap_or(usize::MAX)
    } else {
        0
    }
}

fn as_number(count: usize) -> Value<'static> {
    // Zähler bleiben weit unter 2⁵³.
    Value::number(u32::try_from(count).map_or(f64::INFINITY, f64::from))
}

/// Der Index aus einer Zahl in der Zelle mit den Schlüsseln. Dort stehen
/// nur Indizes aus [[OwnPropertyKeys]].
fn index_of(number: f64) -> u32 {
    if number >= 0.0 && number <= f64::from(MAX_ARRAY_INDEX) && number.fract() == 0.0 {
        // Bereich und Ganzzahligkeit sind geprüft.
        number as u32
    } else {
        0
    }
}
