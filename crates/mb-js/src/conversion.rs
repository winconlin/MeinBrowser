//! Typumwandlungen (Abschnitt 7.1): ToPrimitive, ToNumber mit
//! StringToNumber, ToIntegerOrInfinity und ToUint32. Die übrigen kommen mit
//! den Schritten, die sie brauchen.
//!
//! Solange es keine Funktionsobjekte gibt, ist keine Methode aufrufbar.
//! ToPrimitive wirft für Objekte darum vorerst immer einen TypeError.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::lexer::number::from_power_of_two_radix;
use crate::lexer::{is_line_terminator, is_white_space};
use crate::operations::{call, get, get_method, is_callable};
use crate::value::{Kind, Value};

/// Der Typ, den ToPrimitive bevorzugen soll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferredType {
    String,
    Number,
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn is_object(value: Value<'_>) -> bool {
    matches!(value.kind(), Kind::Object(_))
}

/// ToPrimitive (Abschnitt 7.1.1). `None` heißt, dass kein Typ bevorzugt
/// wird.
pub fn to_primitive<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    input: Local<'_>,
    preferred: Option<PreferredType>,
) -> Completion<Local<'s>> {
    let value = agent.heap().get(input);
    if !is_object(value) {
        return Ok(agent.heap().local(scope, value));
    }
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::ToPrimitive);
    if let Some(exotic) = get_method(agent, scope, input, PropertyKey::Symbol(symbol))? {
        let hint = match preferred {
            None => "default",
            Some(PreferredType::String) => "string",
            Some(PreferredType::Number) => "number",
        };
        let hint = agent.heap_mut().alloc_string(scope, &units(hint))?;
        let result = call(agent, scope, exotic, input, &[hint])?;
        if !is_object(agent.heap().get(result)) {
            return Ok(result);
        }
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    ordinary_to_primitive(
        agent,
        scope,
        input,
        preferred.unwrap_or(PreferredType::Number),
    )
}

/// OrdinaryToPrimitive (Abschnitt 7.1.1.1).
pub fn ordinary_to_primitive<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    hint: PreferredType,
) -> Completion<Local<'s>> {
    let names = match hint {
        PreferredType::String => ["toString", "valueOf"],
        PreferredType::Number => ["valueOf", "toString"],
    };
    for name in names {
        let key = agent.key_from_units(scope, &units(name))?;
        let method = get(agent, scope, object, key)?;
        if is_callable(agent.heap(), agent.heap().get(method)) {
            let result = call(agent, scope, method, object, &[])?;
            if !is_object(agent.heap().get(result)) {
                return Ok(result);
            }
        }
    }
    Err(agent.throw_error(ErrorKind::TypeError))
}

/// ToNumber (Abschnitt 7.1.4).
pub fn to_number(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<f64> {
    let primitive = if is_object(agent.heap().get(value)) {
        to_primitive(agent, scope, value, Some(PreferredType::Number))?
    } else {
        value
    };
    let heap = agent.heap();
    let number = match heap.get(primitive).kind() {
        Kind::Number(number) => Some(number),
        Kind::Undefined => Some(f64::NAN),
        Kind::Null | Kind::Boolean(false) => Some(0.0),
        Kind::Boolean(true) => Some(1.0),
        Kind::String(string) => Some(string_to_number(&heap.string_units(string))),
        // Symbole und BigInts werfen. ToPrimitive gibt kein Objekt zurück,
        // Lücken und interne Zellen sind keine Werte der Sprache.
        Kind::Symbol(_) | Kind::BigInt(_) | Kind::Object(_) | Kind::Hole | Kind::Internal(_) => {
            None
        }
    };
    number.ok_or_else(|| agent.throw_error(ErrorKind::TypeError))
}

/// StringToNumber (Abschnitt 7.1.4.1.1) für die Codeeinheiten einer
/// Zeichenkette. Einzelne Surrogate passen zu keiner Regel der Grammatik
/// und ergeben NaN wie jedes andere Zeichen außerhalb von ASCII, das kein
/// Leerraum ist.
pub fn string_to_number(units: &[u16]) -> f64 {
    let space = |unit: &u16| {
        let c = u32::from(*unit);
        is_white_space(c) || is_line_terminator(c)
    };
    let start = units.iter().position(|unit| !space(unit));
    let end = units.iter().rposition(|unit| !space(unit));
    let (Some(start), Some(end)) = (start, end) else {
        // Leer oder nur Leerraum.
        return 0.0;
    };
    let mut text = String::new();
    for &unit in units.get(start..=end).unwrap_or_default() {
        match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii() => text.push(char::from(byte)),
            _ => return f64::NAN,
        }
    }
    non_decimal(&text)
        .or_else(|| decimal(&text))
        .unwrap_or(f64::NAN)
}

/// *NonDecimalIntegerLiteral* ohne Trennzeichen.
fn non_decimal(text: &str) -> Option<f64> {
    let radix = match text.get(..2)? {
        "0b" | "0B" => 2,
        "0o" | "0O" => 8,
        "0x" | "0X" => 16,
        _ => return None,
    };
    let digits = text.get(2..)?;
    let valid = !digits.is_empty() && digits.chars().all(|digit| digit.is_digit(radix));
    valid.then(|| from_power_of_two_radix(digits, radix))
}

/// *StrDecimalLiteral*.
fn decimal(text: &str) -> Option<f64> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let magnitude = if unsigned == "Infinity" {
        f64::INFINITY
    } else {
        unsigned_decimal(unsigned)?
    };
    // Aus +0 wird -0.
    Some(if negative { -magnitude } else { magnitude })
}

/// *StrUnsignedDecimalLiteral* ohne `Infinity`.
fn unsigned_decimal(text: &str) -> Option<f64> {
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(at) => (text.get(..at)?, Some(text.get(at + 1..)?)),
        None => (text, None),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if !digits(integer) || !digits(fraction) || (integer.is_empty() && fraction.is_empty()) {
        return None;
    }
    let (negative, exponent) = match exponent {
        None => (false, "0"),
        Some(exponent) => match exponent.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, exponent.strip_prefix('+').unwrap_or(exponent)),
        },
    };
    if exponent.is_empty() || !digits(exponent) {
        return None;
    }
    // Wie im Lexer: Die Form Ziffern.Ziffern0eZiffern rundet Rust exakt,
    // zu große Exponenten ergeben Unendlich, zu kleine Null.
    let mut normalized = String::with_capacity(text.len() + 4);
    normalized.push_str(if integer.is_empty() { "0" } else { integer });
    normalized.push('.');
    normalized.push_str(fraction);
    normalized.push_str("0e");
    if negative {
        normalized.push('-');
    }
    normalized.push_str(exponent);
    normalized.parse::<f64>().ok()
}

/// ToIntegerOrInfinity (Abschnitt 7.1.5): eine ganze Zahl oder ±∞, nie -0.
pub fn to_integer_or_infinity(
    agent: &mut Agent,
    scope: &Scope,
    value: Local<'_>,
) -> Completion<f64> {
    let number = to_number(agent, scope, value)?;
    Ok(integer_or_infinity(number))
}

fn integer_or_infinity(number: f64) -> f64 {
    let truncated = number.trunc();
    if truncated.is_nan() || truncated == 0.0 {
        0.0
    } else {
        truncated
    }
}

/// ToUint32 (Abschnitt 7.1.9).
pub fn to_uint32(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<u32> {
    let integer = to_integer_or_infinity(agent, scope, value)?;
    Ok(unsigned_32(integer))
}

/// ToFixedSizeInteger (Abschnitt 7.1.7) mit ~unsigned~ und 32 Bit für eine
/// ganze Zahl oder ±∞.
fn unsigned_32(integer: f64) -> u32 {
    const MODULUS: f64 = 4_294_967_296.0;
    if !integer.is_finite() {
        return 0;
    }
    // Der Rest ganzer Zahlen ist exakt und liegt in [0, 2³²).
    let fixed = integer.rem_euclid(MODULUS);
    if fixed.fract() == 0.0 && (0.0..MODULUS).contains(&fixed) {
        // Schneidet nicht ab: Bereich und Ganzzahligkeit sind geprüft.
        fixed as u32
    } else {
        0
    }
}

#[cfg(test)]
mod tests;
