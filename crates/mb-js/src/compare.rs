//! Vergleiche aus Abschnitt 7.2.

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::mem::discriminant;

use crate::agent::{Agent, Completion};
use crate::bigint::{self, BigInt};
use crate::conversion::{
    Numeric, PreferredType, string_to_bigint, string_to_number, to_numeric, to_primitive,
};
use crate::heap::{Heap, Local, Scope};
use crate::number;
use crate::value::{Kind, Ref, Value};

/// SameType (Abschnitt 7.2.8).
fn same_type(x: Value<'_>, y: Value<'_>) -> bool {
    discriminant(&x.kind()) == discriminant(&y.kind())
}

/// SameValue (Abschnitt 7.2.9).
pub fn same_value(heap: &Heap, x: Value<'_>, y: Value<'_>) -> bool {
    match (x.as_number(), y.as_number()) {
        // Number::sameValue: alle NaN sind gleich, +0 und -0 nicht.
        (Some(x), Some(y)) => {
            (x.is_nan() && y.is_nan()) || (x == y && x.is_sign_negative() == y.is_sign_negative())
        }
        (None, None) => same_value_non_number(heap, x, y),
        _ => false,
    }
}

/// SameValueZero (Abschnitt 7.2.10).
pub fn same_value_zero(heap: &Heap, x: Value<'_>, y: Value<'_>) -> bool {
    match (x.as_number(), y.as_number()) {
        // Number::sameValueZero: alle NaN sind gleich, +0 und -0 auch.
        (Some(x), Some(y)) => (x.is_nan() && y.is_nan()) || x == y,
        (None, None) => same_value_non_number(heap, x, y),
        _ => false,
    }
}

/// SameValueNonNumber (Abschnitt 7.2.11) für zwei Werte, die keine Zahlen
/// sind; verschiedene Arten sind nie gleich (SameType).
fn same_value_non_number(heap: &Heap, x: Value<'_>, y: Value<'_>) -> bool {
    let (x, y) = (x.kind(), y.kind());
    if discriminant(&x) != discriminant(&y) {
        return false;
    }
    match (x, y) {
        (Kind::String(x), Kind::String(y)) => heap.strings_equal(x, y),
        // BigInt::equal (Abschnitt 6.1.6.2.13) vergleicht die Werte.
        (Kind::BigInt(x), Kind::BigInt(y)) => x == y || heap.bigints_equal(x, y),
        // Alles andere vergleicht die Identität.
        _ => x == y,
    }
}

/// IsLessThan (Abschnitt 7.2.12). `None` steht für *undefined*.
pub fn is_less_than(
    agent: &mut Agent,
    scope: &Scope,
    x: Local<'_>,
    y: Local<'_>,
    left_first: bool,
) -> Completion<Option<bool>> {
    let hint = Some(PreferredType::Number);
    let (px, py) = if left_first {
        let px = to_primitive(agent, scope, x, hint)?;
        (px, to_primitive(agent, scope, y, hint)?)
    } else {
        // Umgekehrt, damit die Auswertung von links nach rechts bleibt.
        let py = to_primitive(agent, scope, y, hint)?;
        (to_primitive(agent, scope, x, hint)?, py)
    };
    let heap = agent.heap();
    match (heap.get(px).kind(), heap.get(py).kind()) {
        (Kind::String(a), Kind::String(b)) => {
            // Lexikographisch nach Codeeinheiten, ein echtes Präfix ist
            // kleiner.
            return Ok(Some(code_units(heap, a).lt(code_units(heap, b))));
        }
        (Kind::BigInt(a), Kind::String(b)) => {
            let a = heap.bigint(a).unwrap_or_default();
            let b = heap.string_units(b);
            return Ok(string_to_bigint(agent, &b)?.map(|b| a < b));
        }
        (Kind::String(a), Kind::BigInt(b)) => {
            let a = heap.string_units(a);
            let b = heap.bigint(b).unwrap_or_default();
            return Ok(string_to_bigint(agent, &a)?.map(|a| a < b));
        }
        _ => {}
    }
    let nx = to_numeric(agent, scope, px)?;
    let ny = to_numeric(agent, scope, py)?;
    let value = |numeric| bigint::value_of(agent, numeric).unwrap_or_default();
    Ok(match (nx, ny) {
        (Numeric::Number(x), Numeric::Number(y)) => number::less_than(x, y),
        (Numeric::BigInt(x), Numeric::BigInt(y)) => Some(value(x) < value(y)),
        // Ein BigInt und eine Zahl vergleichen ihre mathematischen Werte;
        // NaN ergibt *undefined*, die Unendlichkeiten liegen außen.
        (Numeric::BigInt(x), Numeric::Number(y)) => value(x)
            .compare_number(y)
            .map(|order| order == Ordering::Less),
        (Numeric::Number(x), Numeric::BigInt(y)) => value(y)
            .compare_number(x)
            .map(|order| order == Ordering::Greater),
    })
}

fn code_units<'h>(heap: &'h Heap, string: Ref<'h>) -> impl Iterator<Item = u16> + 'h {
    (0..heap.string_len(string)).map_while(move |index| heap.string_unit(string, index))
}

/// Was IsLooselyEqual mit einem der beiden Werte tut, bevor es weiter
/// vergleicht.
enum Step {
    Left(f64),
    Right(f64),
    LeftToPrimitive,
    RightToPrimitive,
    /// Ein BigInt mit einer Zeichenkette: StringToBigInt der Zeichenkette,
    /// dann der Vergleich der Werte.
    StringToBigInt(BigInt, Vec<u16>),
}

/// IsLooselyEqual (Abschnitt 7.2.13), der Operator `==`. Statt sich selbst
/// aufzurufen wie in der Spezifikation, wandelt eine Schleife einen der
/// Werte um, bis ein Schritt das Ergebnis liefert, spätestens in der
/// vierten Runde. Objekte mit [[IsHTMLDDA]] (Anhang B.3.6) kommen mit dem
/// Host.
pub fn is_loosely_equal(
    agent: &mut Agent,
    scope: &Scope,
    x: Local<'_>,
    y: Local<'_>,
) -> Completion<bool> {
    let heap = agent.heap();
    let mut x = heap.local(scope, heap.get(x));
    let mut y = heap.local(scope, heap.get(y));
    loop {
        let heap = agent.heap();
        let (left, right) = (heap.get(x), heap.get(y));
        if same_type(left, right) {
            return Ok(is_strictly_equal(heap, left, right));
        }
        let string_number = |string| string_to_number(&heap.string_units(string));
        let boolean_number = |boolean| if boolean { 1.0 } else { 0.0 };
        let step = match (left.kind(), right.kind()) {
            (Kind::Null, Kind::Undefined) | (Kind::Undefined, Kind::Null) => return Ok(true),
            (Kind::Number(_), Kind::String(string)) => Step::Right(string_number(string)),
            (Kind::String(string), Kind::Number(_)) => Step::Left(string_number(string)),
            (Kind::Boolean(boolean), _) => Step::Left(boolean_number(boolean)),
            (_, Kind::Boolean(boolean)) => Step::Right(boolean_number(boolean)),
            (
                Kind::String(_) | Kind::Number(_) | Kind::BigInt(_) | Kind::Symbol(_),
                Kind::Object(_),
            ) => Step::RightToPrimitive,
            (
                Kind::Object(_),
                Kind::String(_) | Kind::Number(_) | Kind::BigInt(_) | Kind::Symbol(_),
            ) => Step::LeftToPrimitive,
            (Kind::BigInt(bigint), Kind::String(string))
            | (Kind::String(string), Kind::BigInt(bigint)) => Step::StringToBigInt(
                heap.bigint(bigint).unwrap_or_default(),
                heap.string_units(string),
            ),
            // Ein BigInt und eine Zahl sind gleich, wenn die Zahl endlich
            // ist und denselben mathematischen Wert hat.
            (Kind::BigInt(bigint), Kind::Number(number))
            | (Kind::Number(number), Kind::BigInt(bigint)) => {
                let value = heap.bigint(bigint).unwrap_or_default();
                return Ok(
                    number.is_finite() && value.compare_number(number) == Some(Ordering::Equal)
                );
            }
            _ => return Ok(false),
        };
        match step {
            Step::StringToBigInt(bigint, units) => {
                return Ok(string_to_bigint(agent, &units)?.is_some_and(|value| value == bigint));
            }
            Step::Left(number) => x = agent.heap().local(scope, Value::number(number)),
            Step::Right(number) => y = agent.heap().local(scope, Value::number(number)),
            Step::LeftToPrimitive => x = to_primitive(agent, scope, x, None)?,
            Step::RightToPrimitive => y = to_primitive(agent, scope, y, None)?,
        }
    }
}

/// IsStrictlyEqual (Abschnitt 7.2.14), der Operator `===`.
pub fn is_strictly_equal(heap: &Heap, x: Value<'_>, y: Value<'_>) -> bool {
    match (x.as_number(), y.as_number()) {
        (Some(x), Some(y)) => number::equal(x, y),
        (None, None) => same_value_non_number(heap, x, y),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
