//! Typumwandlungen (Abschnitt 7.1): ToPrimitive, ToBoolean, ToNumeric,
//! ToNumber mit StringToNumber, ToIntegerOrInfinity, ToInt32, ToUint32,
//! ToString, ToObject und ToPropertyKey. Die übrigen kommen mit den
//! Schritten, die sie brauchen. BigInt-Werte gibt es noch nicht; bis sie
//! kommen, ergibt ToNumeric immer eine Zahl.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::heap::{Heap, Local, Scope};
use crate::key::{MAX_ARRAY_INDEX, PropertyKey, WellKnownSymbol};
use crate::lexer::number::from_power_of_two_radix;
use crate::lexer::{is_line_terminator, is_white_space};
use crate::number::{self, integer_or_infinity};
use crate::object::{primitive_object_create, string_create};
use crate::operations::{call, get, get_method, is_callable};
use crate::realm::{Intrinsic, RealmId};
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

/// ToBoolean (Abschnitt 7.1.2). Objekte mit [[IsHTMLDDA]] (Anhang B.3.6)
/// kommen mit dem Host, bis dahin ist jedes Objekt *true*.
pub fn to_boolean(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        Kind::Boolean(boolean) => boolean,
        Kind::Undefined | Kind::Null => false,
        Kind::Number(number) => number != 0.0 && !number.is_nan(),
        Kind::String(string) => heap.string_len(string) != 0,
        // 0n wäre *false*, aber BigInt-Werte gibt es noch nicht. Lücken und
        // interne Zellen sind keine Werte der Sprache.
        Kind::Symbol(_) | Kind::BigInt(_) | Kind::Object(_) | Kind::Hole | Kind::Internal(_) => {
            true
        }
    }
}

/// ToNumeric (Abschnitt 7.1.3). Ein BigInt käme unverändert zurück; bis es
/// BigInt-Werte gibt, ist das Ergebnis immer eine Zahl.
pub fn to_numeric(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<f64> {
    let primitive = to_primitive(agent, scope, value, Some(PreferredType::Number))?;
    to_number(agent, scope, primitive)
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

/// Leerraum für TrimString (Abschnitt 22.1.3.32.1): *WhiteSpace* und
/// *LineTerminator*.
fn is_trimmed(unit: u16) -> bool {
    let c = u32::from(unit);
    is_white_space(c) || is_line_terminator(c)
}

/// TrimString(string, ~start~): `units` ohne führenden Leerraum.
fn trim_start(units: &[u16]) -> &[u16] {
    let start = units
        .iter()
        .position(|&unit| !is_trimmed(unit))
        .unwrap_or(units.len());
    units.get(start..).unwrap_or_default()
}

/// Die Schritte 2 bis 5 von parseFloat (Abschnitt 19.2.4): der längste
/// Anfang der Zeichenkette ohne führenden Leerraum, der ein
/// *StrDecimalLiteral* ist, als Zahl, sonst NaN.
pub(crate) fn parse_float(units: &[u16]) -> f64 {
    // Zum Literal gehören nur Zeichen aus ASCII.
    let text: String = trim_start(units)
        .iter()
        .map_while(|&unit| u8::try_from(unit).ok().filter(u8::is_ascii))
        .map(char::from)
        .collect();
    let length = decimal_prefix(text.as_bytes());
    if length == 0 {
        return f64::NAN;
    }
    text.get(..length).and_then(decimal).unwrap_or(f64::NAN)
}

/// Die Länge des längsten Anfangs von `bytes`, der ein *StrDecimalLiteral*
/// ist, 0, wenn keiner es ist.
fn decimal_prefix(bytes: &[u8]) -> usize {
    let digits_from = |start: usize| {
        let count = bytes
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        start.saturating_add(count)
    };
    let mut at = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    if bytes
        .get(at..)
        .is_some_and(|rest| rest.starts_with(b"Infinity"))
    {
        return at.saturating_add(8);
    }
    let integer_end = digits_from(at);
    let mut has_digits = integer_end > at;
    at = integer_end;
    if bytes.get(at) == Some(&b'.') {
        let fraction_end = digits_from(at.saturating_add(1));
        has_digits |= fraction_end > at.saturating_add(1);
        at = fraction_end;
    }
    if !has_digits {
        return 0;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        let mut exponent = at.saturating_add(1);
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent = exponent.saturating_add(1);
        }
        let exponent_end = digits_from(exponent);
        if exponent_end > exponent {
            at = exponent_end;
        }
    }
    at
}

/// Die Schritte 2 bis 16 von parseInt (Abschnitt 19.2.5) mit `radix` als
/// Ergebnis von ToInt32. Zur Basis 10 und zu Basen, die Potenzen von 2
/// sind, ist das Ergebnis genau gerundet, zu den übrigen Basen eine Näherung,
/// wie Schritt 14 sie erlaubt.
pub(crate) fn parse_int(units: &[u16], radix: i32) -> f64 {
    let mut rest = trim_start(units);
    let negative = rest.first() == Some(&u16::from(b'-'));
    if matches!(rest.first(), Some(&unit) if unit == u16::from(b'-') || unit == u16::from(b'+')) {
        rest = rest.get(1..).unwrap_or_default();
    }
    let (mut radix, strip_prefix) = match radix {
        0 => (10, true),
        16 => (16, true),
        2..=36 => (radix.unsigned_abs(), false),
        _ => return f64::NAN,
    };
    if strip_prefix
        && rest.first() == Some(&u16::from(b'0'))
        && matches!(rest.get(1), Some(&unit) if unit == u16::from(b'x') || unit == u16::from(b'X'))
    {
        rest = rest.get(2..).unwrap_or_default();
        radix = 16;
    }
    let digits: String = rest
        .iter()
        .map_while(|&unit| {
            let c = char::from_u32(u32::from(unit))?;
            c.is_digit(radix).then_some(c)
        })
        .collect();
    if digits.is_empty() {
        return f64::NAN;
    }
    let magnitude = match radix {
        10 => digits.parse::<f64>().unwrap_or(f64::NAN),
        2 | 4 | 8 | 16 | 32 => from_power_of_two_radix(&digits, radix),
        _ => digits.chars().fold(0.0, |value, c| {
            value * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
        }),
    };
    // Null mit Minus ergibt -0.
    if negative { -magnitude } else { magnitude }
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

/// ToLength (Abschnitt 7.1.22): eine ganze Zahl von 0 bis 2⁵³ − 1.
pub fn to_length(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<f64> {
    /// 2⁵³ − 1, die größte ganze Zahl, die eine Gleitkommazahl exakt hält.
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    // ToIntegerOrInfinity liefert nie NaN und nie -0, `clamp` also eine
    // ganze Zahl zwischen den Grenzen.
    let length = to_integer_or_infinity(agent, scope, value)?;
    Ok(length.clamp(0.0, MAX_SAFE_INTEGER))
}

/// ToInt32 (Abschnitt 7.1.8).
pub fn to_int32(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<i32> {
    let integer = to_integer_or_infinity(agent, scope, value)?;
    Ok(number::to_int32(integer))
}

/// ToUint32 (Abschnitt 7.1.9).
pub fn to_uint32(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<u32> {
    let integer = to_integer_or_infinity(agent, scope, value)?;
    Ok(number::to_uint32(integer))
}

/// ToString (Abschnitt 7.1.19). Das Ergebnis ist immer eine Zeichenkette.
pub fn to_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
) -> Completion<Local<'s>> {
    let primitive = if is_object(agent.heap().get(value)) {
        to_primitive(agent, scope, value, Some(PreferredType::String))?
    } else {
        agent.heap().local(scope, agent.heap().get(value))
    };
    let text = match agent.heap().get(primitive).kind() {
        Kind::String(_) => return Ok(primitive),
        Kind::Undefined => Some("undefined".to_owned()),
        Kind::Null => Some("null".to_owned()),
        Kind::Boolean(true) => Some("true".to_owned()),
        Kind::Boolean(false) => Some("false".to_owned()),
        Kind::Number(number) => Some(number::to_string(number)),
        // Symbole werfen, BigInt::toString kommt mit den BigInt-Werten.
        // ToPrimitive gibt kein Objekt zurück, Lücken und interne Zellen
        // sind keine Werte der Sprache.
        Kind::Symbol(_) | Kind::BigInt(_) | Kind::Object(_) | Kind::Hole | Kind::Internal(_) => {
            None
        }
    };
    let Some(text) = text else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    Ok(agent.heap_mut().alloc_string(scope, &units(&text))?)
}

/// ToObject (Abschnitt 7.1.20). Wrapper-Objekte bekommen ihren Prototyp
/// aus `realm`, dem aktuellen Realm.
pub fn to_object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'_>,
) -> Completion<Local<'s>> {
    let intrinsic = match agent.heap().get(value).kind() {
        Kind::Object(_) => {
            let heap = agent.heap();
            return Ok(heap.local(scope, heap.get(value)));
        }
        Kind::Boolean(_) => Intrinsic::BooleanPrototype,
        Kind::Number(_) => Intrinsic::NumberPrototype,
        Kind::String(_) => {
            let prototype = agent.intrinsic(scope, realm, Intrinsic::StringPrototype);
            return string_create(agent, scope, value, prototype);
        }
        Kind::Symbol(_) => Intrinsic::SymbolPrototype,
        // BigInt-Werte gibt es noch nicht, Lücken und interne Zellen sind
        // keine Werte der Sprache.
        Kind::Undefined | Kind::Null | Kind::BigInt(_) | Kind::Hole | Kind::Internal(_) => {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
    };
    let prototype = agent.intrinsic(scope, realm, intrinsic);
    Ok(primitive_object_create(agent, scope, value, prototype)?)
}

/// ToPropertyKey (Abschnitt 7.1.21). Eine Zahl, deren Zeichenkette ein
/// Array-Index ist, wird ohne die Zeichenkette zum Index.
pub fn to_property_key<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
) -> Completion<PropertyKey<'s>> {
    if let Some(index) = agent.heap().get(value).as_number().and_then(number_index) {
        return Ok(PropertyKey::Index(index));
    }
    let key = to_primitive(agent, scope, value, Some(PreferredType::String))?;
    if matches!(agent.heap().get(key).kind(), Kind::Symbol(_)) {
        return Ok(PropertyKey::Symbol(key));
    }
    let string = to_string(agent, scope, key)?;
    match agent.key_from_value(scope, string) {
        Some(key) => Ok(key),
        // ToString liefert immer eine Zeichenkette.
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// Der Array-Index, den ToString aus der Zahl machen würde. Ganze Zahlen
/// unter 10²¹ schreibt Number::toString als Dezimalziffern ohne führende
/// Null, -0 als "0".
fn number_index(number: f64) -> Option<u32> {
    if number.fract() == 0.0 && (0.0..=f64::from(MAX_ARRAY_INDEX)).contains(&number) {
        // Schneidet nicht ab: Bereich und Ganzzahligkeit sind geprüft.
        Some(number as u32)
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
