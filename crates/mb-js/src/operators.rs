//! Operatoren aus Abschnitt 13, die mehr als eine Umwandlung brauchen:
//! ApplyStringOrNumericBinaryOperator (Abschnitt 13.15.3), die einstelligen
//! Operatoren auf Zahlen und BigInts (Abschnitte 13.4 und 13.5) und
//! `typeof` (Abschnitt 13.5.3).

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::bigint::{self, BigInt, Bitwise};
use crate::conversion::{Numeric, to_numeric, to_primitive, to_string};
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
    let (left, right) = if op == BinaryOp::Add {
        let left = to_primitive(agent, scope, left, None)?;
        let right = to_primitive(agent, scope, right, None)?;
        let heap = agent.heap();
        if is_string(heap.get(left)) || is_string(heap.get(right)) {
            let left = to_string(agent, scope, left)?;
            let right = to_string(agent, scope, right)?;
            return concatenate(agent, scope, left, right);
        }
        (left, right)
    } else {
        let heap = agent.heap();
        (
            heap.local(scope, heap.get(left)),
            heap.local(scope, heap.get(right)),
        )
    };
    let left = to_numeric(agent, scope, left)?;
    let right = to_numeric(agent, scope, right)?;
    match (left, right) {
        (Numeric::Number(x), Numeric::Number(y)) => Ok(agent
            .heap()
            .local(scope, Value::number(number_operation(x, op, y)))),
        (Numeric::BigInt(x), Numeric::BigInt(y)) => bigint_operation(agent, scope, x, op, y),
        // SameType gilt nicht.
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// Die Rechenarten der Zahlen aus Tabelle 2 in Abschnitt 6.1.6.1.
fn number_operation(left: f64, op: BinaryOp, right: f64) -> f64 {
    // Number::multiply, divide, add und subtract (Abschnitte 6.1.6.1.4,
    // 6.1.6.1.5, 6.1.6.1.7 und 6.1.6.1.8) sind die Rechenarten von
    // IEEE 754, die Rust genauso ausführt.
    match op {
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
    }
}

/// Die Rechenarten der BigInts aus Tabelle 2 in Abschnitt 6.1.6.2.
fn bigint_operation<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    left: Local<'_>,
    op: BinaryOp,
    right: Local<'_>,
) -> Completion<Local<'s>> {
    let (Some(x), Some(y)) = (
        bigint::value_of(agent, left),
        bigint::value_of(agent, right),
    ) else {
        // ToNumeric gab BigInts.
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let result = match op {
        // BigInt::exponentiate (Abschnitt 6.1.6.2.3).
        BinaryOp::Exponentiate if y.is_negative() => {
            return Err(agent.throw_error(ErrorKind::RangeError));
        }
        BinaryOp::Exponentiate => bigint::compute(agent, |budget| x.exponentiate(&y, budget))?,
        BinaryOp::Multiply => bigint::compute(agent, |budget| x.multiply(&y, budget))?,
        // BigInt::divide und BigInt::remainder (Abschnitte 6.1.6.2.5 und
        // 6.1.6.2.6) werfen für den Divisor 0 einen RangeError.
        BinaryOp::Divide | BinaryOp::Remainder => {
            match bigint::compute(agent, |budget| x.divide(&y, budget))? {
                None => return Err(agent.throw_error(ErrorKind::RangeError)),
                Some((quotient, _)) if op == BinaryOp::Divide => quotient,
                Some((_, remainder)) => remainder,
            }
        }
        BinaryOp::Add => bigint::compute(agent, |budget| x.add(&y, budget))?,
        BinaryOp::Subtract => bigint::compute(agent, |budget| x.subtract(&y, budget))?,
        BinaryOp::LeftShift => bigint::compute(agent, |budget| x.shift_left(&y, budget))?,
        BinaryOp::SignedRightShift => bigint::compute(agent, |budget| x.shift_right(&y, budget))?,
        // BigInt::unsignedRightShift (Abschnitt 6.1.6.2.11).
        BinaryOp::UnsignedRightShift => return Err(agent.throw_error(ErrorKind::TypeError)),
        BinaryOp::BitwiseAnd | BinaryOp::BitwiseXor | BinaryOp::BitwiseOr => {
            let bitwise = match op {
                BinaryOp::BitwiseAnd => Bitwise::And,
                BinaryOp::BitwiseXor => Bitwise::Xor,
                _ => Bitwise::Or,
            };
            bigint::compute(agent, |budget| x.bitwise(bitwise, &y, budget))?
        }
    };
    bigint::allocate(agent, scope, &result)
}

/// Die einstelligen Operatoren auf dem Ergebnis von ToNumeric.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    /// `-` (Abschnitt 13.5.5)
    Negate,
    /// `~` (Abschnitt 13.5.6)
    BitwiseNot,
    /// `++` (Abschnitte 13.4.2 bis 13.4.5)
    Increment,
    /// `--`
    Decrement,
    /// Nur ToNumeric, für den alten Wert der Postfix-Operatoren.
    ToNumeric,
}

/// Wendet einen einstelligen Operator nach ToNumeric an, für Zahlen mit
/// Number::unaryMinus, Number::bitwiseNOT und Number::add, für BigInts mit
/// den Gegenstücken aus Abschnitt 6.1.6.2.
pub fn apply_unary_operator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    op: UnaryOp,
    value: Local<'_>,
) -> Completion<Local<'s>> {
    let bigint = match to_numeric(agent, scope, value)? {
        Numeric::Number(number) => {
            let result = match op {
                UnaryOp::Negate => number::unary_minus(number),
                UnaryOp::BitwiseNot => number::bitwise_not(number),
                UnaryOp::Increment => number + 1.0,
                UnaryOp::Decrement => number - 1.0,
                UnaryOp::ToNumeric => number,
            };
            return Ok(agent.heap().local(scope, Value::number(result)));
        }
        Numeric::BigInt(bigint) => bigint,
    };
    if op == UnaryOp::ToNumeric {
        return Ok(bigint);
    }
    let Some(x) = bigint::value_of(agent, bigint) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let one = BigInt::from_u64(1);
    let result = match op {
        UnaryOp::Negate => bigint::compute(agent, |budget| x.negate_within(budget))?,
        UnaryOp::BitwiseNot => bigint::compute(agent, |budget| x.not(budget))?,
        UnaryOp::Increment => bigint::compute(agent, |budget| x.add(&one, budget))?,
        _ => bigint::compute(agent, |budget| x.subtract(&one, budget))?,
    };
    bigint::allocate(agent, scope, &result)
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
