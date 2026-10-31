//! Die Operationen des Typs Number (Abschnitt 6.1.6.1). Addition,
//! Subtraktion, Multiplikation und Division sind die von IEEE 754, die Rust
//! mit `+`, `-`, `*` und `/` schon so rechnet.

#![forbid(unsafe_code)]

/// Number::unaryMinus (Abschnitt 6.1.6.1.1). NaN bleibt NaN, sonst wechselt
/// das Vorzeichen.
pub fn unary_minus(number: f64) -> f64 {
    -number
}

/// Number::bitwiseNOT (Abschnitt 6.1.6.1.2).
pub fn bitwise_not(number: f64) -> f64 {
    f64::from(!to_int32(number))
}

/// Number::exponentiate (Abschnitt 6.1.6.1.3). Anders als IEEE 754 ergibt
/// ±1 hoch ±∞ und 1 hoch NaN den Wert NaN.
pub fn exponentiate(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() {
        return f64::NAN;
    }
    if exponent == 0.0 {
        return 1.0;
    }
    if base.is_nan() {
        return f64::NAN;
    }
    if exponent.is_infinite() && base.abs() == 1.0 {
        return f64::NAN;
    }
    // Für alle übrigen Sonderfälle der Spezifikation (Basis ±∞ oder ±0,
    // Exponent ±∞, negative Basis mit gebrochenem Exponenten) liefert pow
    // nach IEEE 754 dasselbe Ergebnis.
    base.powf(exponent)
}

/// Number::remainder (Abschnitt 6.1.6.1.6). Der Rest von Rust ist fmod aus
/// IEEE 754: exakt, mit dem Vorzeichen des Dividenden, und mit denselben
/// Sonderfällen.
pub fn remainder(numerator: f64, denominator: f64) -> f64 {
    numerator % denominator
}

/// Number::leftShift (Abschnitt 6.1.6.1.9).
pub fn left_shift(x: f64, y: f64) -> f64 {
    f64::from(to_int32(x).wrapping_shl(to_uint32(y) % 32))
}

/// Number::signedRightShift (Abschnitt 6.1.6.1.10).
pub fn signed_right_shift(x: f64, y: f64) -> f64 {
    f64::from(to_int32(x).wrapping_shr(to_uint32(y) % 32))
}

/// Number::unsignedRightShift (Abschnitt 6.1.6.1.11).
pub fn unsigned_right_shift(x: f64, y: f64) -> f64 {
    f64::from(to_uint32(x).wrapping_shr(to_uint32(y) % 32))
}

/// Number::lessThan (Abschnitt 6.1.6.1.12): `None` steht für *undefined*,
/// wenn ein Operand NaN ist.
pub fn less_than(x: f64, y: f64) -> Option<bool> {
    if x.is_nan() || y.is_nan() {
        return None;
    }
    // Der Vergleich von IEEE 754 behandelt ±0 als gleich und ±∞ wie die
    // Spezifikation.
    Some(x < y)
}

/// Number::equal (Abschnitt 6.1.6.1.13).
pub fn equal(x: f64, y: f64) -> bool {
    x == y
}

/// Die bitweisen Operatoren aus NumberBitwiseOp (Abschnitt 6.1.6.1.16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitwiseOp {
    And,
    Xor,
    Or,
}

/// NumberBitwiseOp (Abschnitt 6.1.6.1.16).
pub fn bitwise(op: BitwiseOp, x: f64, y: f64) -> f64 {
    let (left, right) = (to_int32(x), to_int32(y));
    f64::from(match op {
        BitwiseOp::And => left & right,
        BitwiseOp::Xor => left ^ right,
        BitwiseOp::Or => left | right,
    })
}

const TWO_TO_THE_32: f64 = 4_294_967_296.0;

/// Ganze Zahl oder ±∞ wie ToIntegerOrInfinity (Abschnitt 7.1.5), nie -0.
pub(crate) fn integer_or_infinity(number: f64) -> f64 {
    let truncated = number.trunc();
    if truncated.is_nan() || truncated == 0.0 {
        0.0
    } else {
        truncated
    }
}

/// ToFixedSizeInteger (Abschnitt 7.1.7) mit ~unsigned~ und 32 Bit für eine
/// Zahl, die zuerst wie in ToIntegerOrInfinity abgeschnitten wird. Das ist
/// ToUint32 (Abschnitt 7.1.9) für Zahlen.
pub fn to_uint32(number: f64) -> u32 {
    let integer = integer_or_infinity(number);
    if !integer.is_finite() {
        return 0;
    }
    // Der Rest ganzer Zahlen ist exakt und liegt in [0, 2³²).
    let fixed = integer.rem_euclid(TWO_TO_THE_32);
    if fixed.fract() == 0.0 && (0.0..TWO_TO_THE_32).contains(&fixed) {
        // Schneidet nicht ab: Bereich und Ganzzahligkeit sind geprüft.
        fixed as u32
    } else {
        0
    }
}

/// ToInt32 (Abschnitt 7.1.8) für Zahlen: dieselben 32 Bit wie ToUint32 als
/// Zweierkomplement gelesen.
pub fn to_int32(number: f64) -> i32 {
    i32::from_ne_bytes(to_uint32(number).to_ne_bytes())
}

/// Die Ziffern aus Schritt 5 von Number::toString für eine endliche Zahl
/// über null und der Exponent der ersten Ziffer. Die Anmerkung zu
/// Number::toString empfiehlt die kürzesten Ziffern, die zur selben Zahl
/// zurückführen, davon die nächsten und bei Gleichstand die gerade Folge;
/// so rechnet auch V8. Rust liefert mit `{:e}` die kürzesten und nächsten
/// Ziffern, bei Gleichstand aber die obere Folge.
fn shortest_digits(x: f64) -> (Vec<u8>, i64) {
    let (mut digits, exponent) = scientific_digits(&format!("{x:e}"));
    if let Some(even) = even_neighbour(x, &digits, exponent) {
        digits = even;
    }
    (digits, exponent)
}

/// Ziffern und Exponent aus der Schreibweise `d.ddde±x` von Rust.
fn scientific_digits(text: &str) -> (Vec<u8>, i64) {
    let (mantissa, exponent) = text.split_once('e').unwrap_or((text, "0"));
    let digits = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    // Der Exponent von f64 liegt weit innerhalb von i64.
    (digits, exponent.parse().unwrap_or(0))
}

/// Die gerade Ziffernfolge gleicher Länge, wenn `x` genau in der Mitte
/// zwischen ihr und der ungeraden Folge `digits` liegt und ebenfalls zu `x`
/// zurückführt.
fn even_neighbour(x: f64, digits: &[u8], exponent: i64) -> Option<Vec<u8>> {
    let (&last, _) = digits.split_last()?;
    // `b'0'` ist gerade, das Byte hat also die Parität der Ziffer.
    if last % 2 == 0 {
        return None;
    }
    // In der Mitte liegt `x` nur, wenn sein genauer Dezimalwert eine Ziffer
    // mehr hat und auf 5 endet. Erst eine Ziffer mehr runden, dann den
    // ganzen Wert prüfen: Ein f64 hat höchstens 767 gültige Dezimalziffern.
    let length = digits.len();
    let (rounded, rounded_exponent) = scientific_digits(&format!("{x:.length$e}"));
    if rounded_exponent != exponent || rounded.last() != Some(&b'5') {
        return None;
    }
    let (exact, _) = scientific_digits(&format!("{x:.800e}"));
    let (head, tail) = exact.split_at_checked(length.checked_add(1)?)?;
    if head != rounded.as_slice() || tail.iter().any(|&digit| digit != b'0') {
        return None;
    }
    // Die beiden Nachbarn sind die abgeschnittenen Ziffern und die Folge
    // eins darüber; `digits` ist einer von ihnen.
    let mut lower = rounded;
    lower.pop();
    let neighbour = if lower.as_slice() == digits {
        increment(&lower)?
    } else {
        lower
    };
    if neighbour.last() == Some(&b'0') || neighbour.len() != length {
        return None;
    }
    let mut text = String::new();
    for (index, &digit) in neighbour.iter().enumerate() {
        if index == 1 {
            text.push('.');
        }
        text.push(char::from(digit));
    }
    let back: f64 = format!("{text}e{exponent}").parse().ok()?;
    if back.to_bits() != x.to_bits() {
        return None;
    }
    Some(neighbour)
}

/// Die Dezimalziffern `digits` plus eins, ohne neue Stelle.
fn increment(digits: &[u8]) -> Option<Vec<u8>> {
    let mut result = digits.to_vec();
    for digit in result.iter_mut().rev() {
        if *digit == b'9' {
            *digit = b'0';
        } else {
            *digit = digit.checked_add(1)?;
            return Some(result);
        }
    }
    None
}

/// Number::toString (Abschnitt 6.1.6.1.20) zur Basis 10, mit den Ziffern
/// aus [`shortest_digits`]; hier wird nur die Form zusammengesetzt.
pub fn to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x == 0.0 {
        return "0".to_owned();
    }
    if x < 0.0 {
        return format!("-{}", to_string(-x));
    }
    if x.is_infinite() {
        return "Infinity".to_owned();
    }
    let (digits, exponent) = shortest_digits(x);
    // Ziffern sind ASCII.
    let digits = String::from_utf8(digits).unwrap_or_default();
    let k = i64::try_from(digits.len()).unwrap_or(i64::MAX);
    let n = exponent.saturating_add(1);
    let zeros = |count: i64| "0".repeat(usize::try_from(count).unwrap_or(0));
    if (-5..=21).contains(&n) {
        if n >= k {
            return format!("{digits}{}", zeros(n.saturating_sub(k)));
        }
        if n > 0 {
            let (head, tail) = digits.split_at(usize::try_from(n).unwrap_or(0));
            return format!("{head}.{tail}");
        }
        return format!("0.{}{digits}", zeros(n.saturating_neg()));
    }
    let sign = if n < 0 { '-' } else { '+' };
    let magnitude = n.saturating_sub(1).unsigned_abs();
    match digits.split_at_checked(1) {
        Some((first, rest)) if !rest.is_empty() => format!("{first}.{rest}e{sign}{magnitude}"),
        _ => format!("{digits}e{sign}{magnitude}"),
    }
}

#[cfg(test)]
mod tests;
