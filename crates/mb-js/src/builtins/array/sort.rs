//! SortIndexedProperties (Abschnitt 23.1.3.30.1) und CompareArrayElements
//! (Abschnitt 23.1.3.30.2) für `sort` und `toSorted`.
//!
//! Die Sortierung ist eine stabile Mischsortierung von unten nach oben ohne
//! Rekursion. Sie hört beim ersten Fehler eines Vergleichs auf, wie Schritt
//! 6 von SortIndexedProperties verlangt, und zählt jeden Vergleich als
//! Arbeitsschritt. Ohne Vergleichsfunktion wandelt jeder Vergleich beide
//! Werte mit ToString um, wie die Spezifikation es beschreibt.

use super::{index_key, step};
use crate::agent::{Agent, Completion};
use crate::compare::is_less_than;
use crate::conversion::{to_number, to_string};
use crate::heap::{Local, Scope};
use crate::operations::{call, get, has_property};
use crate::value::Value;

/// Wie SortIndexedProperties mit Lücken umgeht.
#[derive(Clone, Copy)]
pub(super) enum Holes {
    /// ~skip-holes~: Nur vorhandene Eigenschaften kommen in die Liste.
    Skip,
    /// ~read-through-holes~: Jeder Index wird gelesen, eine Lücke liefert
    /// den Wert aus dem Prototyp oder *undefined*.
    ReadThrough,
}

/// SortIndexedProperties (Abschnitt 23.1.3.30.1): die Elemente von
/// `object` bis `length`, sortiert mit `comparator` (*undefined* für die
/// Reihenfolge nach ToString).
pub(super) fn sort_indexed_properties<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    length: f64,
    comparator: Local<'_>,
    holes: Holes,
) -> Completion<Vec<Local<'s>>> {
    let mut items = Vec::new();
    let mut k = 0.0;
    while k < length {
        step(agent)?;
        let key = index_key(agent, scope, k)?;
        let read = match holes {
            Holes::Skip => has_property(agent, object, key)?,
            Holes::ReadThrough => true,
        };
        if read {
            items.push(get(agent, scope, object, key)?);
        }
        k += 1.0;
    }
    merge_sort(agent, &mut items, comparator)?;
    Ok(items)
}

/// Eine stabile Mischsortierung von unten nach oben: erst Läufe der Länge
/// 1, dann 2, 4 und so fort, jede Runde aus `items` in `buffer` und
/// zurück.
fn merge_sort<'s>(
    agent: &mut Agent,
    items: &mut Vec<Local<'s>>,
    comparator: Local<'_>,
) -> Completion<()> {
    let count = items.len();
    let mut buffer = items.clone();
    let mut width = 1;
    while width < count {
        let mut start = 0;
        while start < count {
            let middle = start.saturating_add(width).min(count);
            let end = middle.saturating_add(width).min(count);
            let (left, right) = (
                items.get(start..middle).unwrap_or_default(),
                items.get(middle..end).unwrap_or_default(),
            );
            let merged = merge(agent, left, right, comparator)?;
            if let Some(target) = buffer.get_mut(start..end) {
                target.copy_from_slice(&merged);
            }
            start = end;
        }
        std::mem::swap(items, &mut buffer);
        width = width.checked_mul(2).unwrap_or(count);
    }
    Ok(())
}

/// Mischt zwei sortierte Läufe. Bei Gleichstand kommt das linke Element
/// zuerst, das hält die Sortierung stabil.
fn merge<'s>(
    agent: &mut Agent,
    left: &[Local<'s>],
    right: &[Local<'s>],
    comparator: Local<'_>,
) -> Completion<Vec<Local<'s>>> {
    let mut merged = Vec::with_capacity(left.len().saturating_add(right.len()));
    let (mut i, mut j) = (0, 0);
    while let (Some(&x), Some(&y)) = (left.get(i), right.get(j)) {
        if compare(agent, y, x, comparator)? < 0.0 {
            merged.push(y);
            j = j.saturating_add(1);
        } else {
            merged.push(x);
            i = i.saturating_add(1);
        }
    }
    merged.extend_from_slice(left.get(i..).unwrap_or_default());
    merged.extend_from_slice(right.get(j..).unwrap_or_default());
    Ok(merged)
}

/// CompareArrayElements (Abschnitt 23.1.3.30.2): negativ, wenn `x` vor `y`
/// gehört. *undefined* kommt ans Ende; sonst entscheidet `comparator` oder
/// der Vergleich der Zeichenketten aus ToString.
fn compare(
    agent: &mut Agent,
    x: Local<'_>,
    y: Local<'_>,
    comparator: Local<'_>,
) -> Completion<f64> {
    step(agent)?;
    let (x_undefined, y_undefined) = {
        let heap = agent.heap();
        (heap.get(x).is_undefined(), heap.get(y).is_undefined())
    };
    if x_undefined && y_undefined {
        return Ok(0.0);
    }
    if x_undefined {
        return Ok(1.0);
    }
    if y_undefined {
        return Ok(-1.0);
    }
    agent.scope(|agent, inner| -> Completion<f64> {
        if !agent.heap().get(comparator).is_undefined() {
            let undefined = agent.heap().local(inner, Value::UNDEFINED);
            let result = call(agent, inner, comparator, undefined, &[x, y])?;
            let order = to_number(agent, inner, result)?;
            return Ok(if order.is_nan() { 0.0 } else { order });
        }
        let x_string = to_string(agent, inner, x)?;
        let y_string = to_string(agent, inner, y)?;
        if is_less_than(agent, inner, x_string, y_string, true)? == Some(true) {
            return Ok(-1.0);
        }
        if is_less_than(agent, inner, y_string, x_string, true)? == Some(true) {
            return Ok(1.0);
        }
        Ok(0.0)
    })
}
