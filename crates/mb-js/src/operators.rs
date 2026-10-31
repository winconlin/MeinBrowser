//! Operatoren aus Abschnitt 13, die mehr als eine Umwandlung brauchen:
//! ApplyStringOrNumericBinaryOperator (Abschnitt 13.15.3) und `typeof`
//! (Abschnitt 13.5.3). BigInt-Werte gibt es noch nicht; ihre Rechenarten
//! kommen mit ihnen.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::conversion::{to_numeric, to_primitive, to_string};
use crate::heap::{Heap, Local, Scope};
use crate::number::{self, BitwiseOp};
use crate::operations::is_callable;
use crate::value::{Kind, Value};

/// Die Operatoren von ApplyStringOrNumericBinaryOperator, dort *opText*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    /// `**`
    Exponentiate,
    /// `*`
    Multiply,
    /// `/`
    Divide,
    /// `%`
    Remainder,
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `<<`
    LeftShift,
    /// `>>`
    SignedRightShift,
    /// `>>>`
    UnsignedRightShift,
    /// `&`
    BitwiseAnd,
    /// `^`
    BitwiseXor,
    /// `|`
    BitwiseOr,
}

fn is_string(value: Value<'_>) -> bool {
    matches!(value.kind(), Kind::String(_))
}

/// ApplyStringOrNumericBinaryOperator (Abschnitt 13.15.3).
pub fn apply_string_or_numeric_binary_operator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    left: Local<'_>,
    op: BinaryOp,
    right: Local<'_>,
) -> Completion<Local<'s>> {
    let result = if op == BinaryOp::Add {
        let left = to_primitive(agent, scope, left, None)?;
        let right = to_primitive(agent, scope, right, None)?;
        let heap = agent.heap();
        if is_string(heap.get(left)) || is_string(heap.get(right)) {
            let left = to_string(agent, scope, left)?;
            let right = to_string(agent, scope, right)?;
            return concatenate(agent, scope, left, right);
        }
        numeric(agent, scope, left, op, right)?
    } else {
        numeric(agent, scope, left, op, right)?
    };
    Ok(agent.heap().local(scope, Value::number(result)))
}

/// Die Schritte ab ToNumeric. Bis es BigInt-Werte gibt, sind beide
/// Operanden danach Zahlen und SameType gilt immer.
fn numeric(
    agent: &mut Agent,
    scope: &Scope,
    left: Local<'_>,
    op: BinaryOp,
    right: Local<'_>,
) -> Completion<f64> {
    let left = to_numeric(agent, scope, left)?;
    let right = to_numeric(agent, scope, right)?;
    // Number::multiply, divide, add und subtract (Abschnitte 6.1.6.1.4,
    // 6.1.6.1.5, 6.1.6.1.7 und 6.1.6.1.8) sind die Rechenarten von
    // IEEE 754, die Rust genauso ausführt.
    Ok(match op {
        BinaryOp::Exponentiate => number::exponentiate(left, right),
        BinaryOp::Multiply => left * right,
        BinaryOp::Divide => left / right,
        BinaryOp::Remainder => number::remainder(left, right),
        BinaryOp::Add => left + right,
        BinaryOp::Subtract => left - right,
        BinaryOp::LeftShift => number::left_shift(left, right),
        BinaryOp::SignedRightShift => number::signed_right_shift(left, right),
        BinaryOp::UnsignedRightShift => number::unsigned_right_shift(left, right),
        BinaryOp::BitwiseAnd => number::bitwise(BitwiseOp::And, left, right),
        BinaryOp::BitwiseXor => number::bitwise(BitwiseOp::Xor, left, right),
        BinaryOp::BitwiseOr => number::bitwise(BitwiseOp::Or, left, right),
    })
}

/// Die Verkettung zweier Zeichenketten. Zeichenketten sind bisher flach,
/// also werden beide kopiert. Mehr als 2³² − 1 Codeeinheiten fasst keine;
/// darüber wirft die Verkettung einen RangeError, wie andere Engines an
/// ihrer Grenze.
fn concatenate<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    left: Local<'s>,
    right: Local<'s>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let (Kind::String(first), Kind::String(second)) =
        (heap.get(left).kind(), heap.get(right).kind())
    else {
        // ToString liefert immer Zeichenketten.
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let (first_length, second_length) = (heap.string_len(first), heap.string_len(second));
    if first_length == 0 {
        return Ok(right);
    }
    if second_length == 0 {
        return Ok(left);
    }
    let fits = first_length
        .checked_add(second_length)
        .is_some_and(|length| u32::try_from(length).is_ok());
    if !fits {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let mut units = heap.string_units(first);
    units.extend(heap.string_units(second));
    Ok(agent.heap_mut().alloc_string(scope, &units)?)
}

/// Das Ergebnis von `typeof` für einen Wert, die Schritte ab GetValue in
/// Abschnitt 13.5.3.1. Nicht auflösbare Referenzen behandelt der
/// Interpreter, Objekte mit [[IsHTMLDDA]] (Anhang B.3.6) kommen mit dem
/// Host.
pub fn type_of(heap: &Heap, value: Value<'_>) -> &'static str {
    match value.kind() {
        Kind::Undefined => "undefined",
        Kind::Null => "object",
        Kind::String(_) => "string",
        Kind::Symbol(_) => "symbol",
        Kind::Boolean(_) => "boolean",
        Kind::Number(_) => "number",
        Kind::BigInt(_) => "bigint",
        Kind::Object(_) if is_callable(heap, value) => "function",
        Kind::Object(_) => "object",
        // Lücken und interne Zellen sind keine Werte der Sprache. Eine
        // Lücke liest [[Get]] als *undefined*.
        Kind::Hole | Kind::Internal(_) => "undefined",
    }
}

#[cfg(test)]
mod tests;
