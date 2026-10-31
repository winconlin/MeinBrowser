//! Vergleiche aus Abschnitt 7.2.

#![forbid(unsafe_code)]

use std::mem::discriminant;

use crate::heap::Heap;
use crate::value::{Kind, Value};

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

#[cfg(test)]
mod tests;
