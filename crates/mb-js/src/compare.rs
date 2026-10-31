//! Vergleiche aus Abschnitt 7.2. BigInt-Werte gibt es noch nicht; die
//! Schritte für sie kommen mit ihnen.

#![forbid(unsafe_code)]

use std::mem::discriminant;

use crate::agent::{Agent, Completion};
use crate::conversion::{PreferredType, string_to_number, to_numeric, to_primitive};
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
        // BigInt::equal kommt mit den BigInt-Zellen; bis dahin gibt es
        // keine BigInt-Werte.
        (Kind::BigInt(x), Kind::BigInt(y)) => x == y,
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
    if let (Kind::String(a), Kind::String(b)) = (heap.get(px).kind(), heap.get(py).kind()) {
        // Lexikographisch nach Codeeinheiten, ein echtes Präfix ist kleiner.
        return Ok(Some(code_units(heap, a).lt(code_units(heap, b))));
    }
    let nx = to_numeric(agent, scope, px)?;
    let ny = to_numeric(agent, scope, py)?;
    Ok(number::less_than(nx, ny))
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
            // StringToBigInt und der Vergleich von BigInt und Zahl kommen
            // mit den BigInt-Werten.
            _ => return Ok(false),
        };
        match step {
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
