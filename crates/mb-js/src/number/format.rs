//! Die Zahlformate von %Number.prototype%: toExponential, toFixed und
//! toPrecision (Abschnitte 21.1.3.2, 21.1.3.3 und 21.1.3.5) und
//! Number::toString zu einer anderen Basis als 10 (Abschnitt 6.1.6.1.20).
//!
//! Die drei Formate runden den genauen Wert einer Zahl. Ihn liefert Rust
//! mit `{:.800e}`: Ein f64 hat höchstens 767 gültige Dezimalziffern, 801
//! Ziffern sind also genau. Bei Gleichstand verlangt die Spezifikation die
//! größere Zahl; deshalb runden die Formate hier selbst und nicht über die
//! Genauigkeit von `format!`, die bei Gleichstand zur geraden Ziffer rundet.

use super::{scientific_digits, shortest_digits};

/// Die genauen Dezimalziffern einer endlichen Zahl über null ohne Nullen am
/// Ende und der Exponent der ersten Ziffer.
fn exact_digits(x: f64) -> (Vec<u8>, i64) {
    let (mut digits, exponent) = scientific_digits(&format!("{x:.800e}"));
    while digits.len() > 1 && digits.last() == Some(&b'0') {
        digits.pop();
    }
    (digits, exponent)
}

/// Die Ziffern der ganzen Zahl, die aus den ersten `count` Ziffern von
/// `digits` entsteht, wenn der Rest dahinter gerundet wird: aufwärts ab der
/// Ziffer 5, also bei Gleichstand zur größeren Zahl. Ein Übertrag über alle
/// Stellen ergibt eine Ziffer mehr, `count` null die Ziffer 0 oder 1.
fn round_half_up(digits: &[u8], count: usize) -> Vec<u8> {
    let mut result: Vec<u8> = (0..count)
        .map(|index| digits.get(index).copied().unwrap_or(b'0'))
        .collect();
    if digits.get(count).is_some_and(|&next| next >= b'5') {
        let mut carry = true;
        for digit in result.iter_mut().rev() {
            if *digit == b'9' {
                *digit = b'0';
            } else {
                *digit = digit.saturating_add(1);
                carry = false;
                break;
            }
        }
        if carry {
            result.insert(0, b'1');
        }
    }
    if result.is_empty() {
        result.push(b'0');
    }
    result
}

/// `count` gültige Ziffern einer endlichen Zahl über null, nach
/// [`round_half_up`] gerundet, und der Exponent der ersten Ziffer.
fn significant_digits(x: f64, count: usize) -> (Vec<u8>, i64) {
    let (digits, exponent) = exact_digits(x);
    let mut rounded = round_half_up(&digits, count);
    if rounded.len() > count {
        // Aus 99…9 wurde 100…0: eine Null weniger, eine Stelle höher.
        rounded.pop();
        return (rounded, exponent.saturating_add(1));
    }
    (rounded, exponent)
}

fn text(digits: &[u8]) -> String {
    digits.iter().map(|&digit| char::from(digit)).collect()
}

fn zeros(count: i64) -> String {
    "0".repeat(usize::try_from(count).unwrap_or(0))
}

/// Das Vorzeichen und der Betrag. -0 zählt wie +0 als nicht negativ.
fn split_sign(x: f64) -> (&'static str, f64) {
    if x < 0.0 { ("-", -x) } else { ("", x) }
}

/// `e+d` oder `e-d` für den Exponenten `exponent`.
fn exponent_suffix(exponent: i64) -> String {
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("e{sign}{}", exponent.unsigned_abs())
}

/// Die erste Ziffer, dahinter mit Punkt die übrigen, wenn es welche gibt.
fn mantissa(digits: &[u8]) -> String {
    match digits.split_first() {
        Some((first, rest)) if !rest.is_empty() => {
            format!("{}.{}", char::from(*first), text(rest))
        }
        _ => text(digits),
    }
}

/// Die Schritte 10 bis 12 von Number.prototype.toFixed für eine endliche
/// Zahl unter 10²¹ und `fraction_digits` Nachkommastellen.
pub(crate) fn to_fixed(x: f64, fraction_digits: usize) -> String {
    let (sign, x) = split_sign(x);
    let f = i64::try_from(fraction_digits).unwrap_or(i64::MAX);
    let n = if x == 0.0 {
        vec![b'0']
    } else {
        // x · 10^f hat dieselben Ziffern mit dem Exponenten exponent + f;
        // vor dem Komma stehen davon exponent + f + 1.
        let (digits, exponent) = exact_digits(x);
        let integral = exponent.saturating_add(f).saturating_add(1);
        match usize::try_from(integral) {
            Ok(count) => round_half_up(&digits, count),
            // Unter 0,1 · 10^-f wird n null.
            Err(_) => vec![b'0'],
        }
    };
    if fraction_digits == 0 {
        return format!("{sign}{}", text(&n));
    }
    let mut m = text(&n);
    let k = i64::try_from(n.len()).unwrap_or(i64::MAX);
    if k <= f {
        m = format!("{}{m}", zeros(f.saturating_add(1).saturating_sub(k)));
    }
    let point = m.len().saturating_sub(fraction_digits);
    let (a, b) = m.split_at(point);
    format!("{sign}{a}.{b}")
}

/// Die Schritte 9 bis 15 von Number.prototype.toExponential für eine
/// endliche Zahl. Ohne `fraction_digits` sind es so viele Ziffern, wie die
/// Zahl eindeutig bestimmen, also die kürzesten wie bei Number::toString.
pub(crate) fn to_exponential(x: f64, fraction_digits: Option<usize>) -> String {
    let (sign, x) = split_sign(x);
    let (digits, exponent) = if x == 0.0 {
        let count = fraction_digits.unwrap_or(0).saturating_add(1);
        (vec![b'0'; count], 0)
    } else {
        match fraction_digits {
            None => shortest_digits(x),
            Some(f) => significant_digits(x, f.saturating_add(1)),
        }
    };
    format!("{sign}{}{}", mantissa(&digits), exponent_suffix(exponent))
}

/// Die Schritte 8 bis 13 von Number.prototype.toPrecision für eine
/// endliche Zahl und `precision` gültige Ziffern von 1 bis 100.
pub(crate) fn to_precision(x: f64, precision: usize) -> String {
    let (sign, x) = split_sign(x);
    let (digits, e) = if x == 0.0 {
        (vec![b'0'; precision], 0)
    } else {
        significant_digits(x, precision)
    };
    let p = i64::try_from(precision).unwrap_or(i64::MAX);
    if e < -6 || e >= p {
        return format!("{sign}{}{}", mantissa(&digits), exponent_suffix(e));
    }
    let m = text(&digits);
    if e == p.saturating_sub(1) {
        return format!("{sign}{m}");
    }
    // e ≥ 0: e + 1 Stellen vor dem Komma; sonst „0.“, −(e + 1) Nullen und
    // die Ziffern, auch für e = −1.
    if e >= 0
        && let Ok(integral) = usize::try_from(e.saturating_add(1))
    {
        let (a, b) = m.split_at(integral.min(m.len()));
        return format!("{sign}{a}.{b}");
    }
    format!("{sign}0.{}{m}", zeros(e.saturating_add(1).saturating_neg()))
}

/// Die Ziffern 0 bis 9 und a bis z.
fn digit_char(digit: u32, radix: u32) -> char {
    char::from_digit(digit, radix).unwrap_or('0')
}

/// Number::toString (Abschnitt 6.1.6.1.20) zur Basis `radix` von 2 bis 36.
///
/// Die Spezifikation lässt die Ziffern zu anderen Basen als 10 offen und
/// verlangt nur eine Verallgemeinerung der Basis 10. Wie dort gibt es
/// gerade so viele Nachkommastellen, wie die Zahl von ihren Nachbarn
/// unterscheiden: Sie enden, sobald der Rest unter die halbe Lücke zur
/// nächsten Zahl fällt, und die letzte Stelle wird gerundet, wenn das
/// Ergebnis dabei in dieser Lücke bleibt. Stellen vor dem Komma jenseits
/// von 2⁵³ sind nicht mehr genau bestimmt und werden Nullen.
pub(crate) fn to_radix_string(x: f64, radix: u32) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x == 0.0 {
        return "0".to_owned();
    }
    if x < 0.0 {
        return format!("-{}", to_radix_string(-x, radix));
    }
    if x.is_infinite() {
        return "Infinity".to_owned();
    }
    let base = f64::from(radix);
    let mut integer = x.floor();
    let mut fraction = x - integer;
    // Die halbe Lücke zur nächstgrößeren Zahl, mindestens die kleinste
    // Zahl über null.
    let next = f64::from_bits(x.to_bits().saturating_add(1));
    let mut delta = (0.5 * (next - x)).max(f64::from_bits(1));
    let mut fraction_digits: Vec<u32> = Vec::new();
    if fraction >= delta {
        loop {
            fraction *= base;
            delta *= base;
            // Schneidet nur die Nachkommastellen ab: 0 ≤ fraction < radix.
            let digit = fraction as u32;
            fraction_digits.push(digit);
            fraction -= f64::from(digit);
            let round_up = fraction > 0.5 || (fraction == 0.5 && digit & 1 == 1);
            if round_up && fraction + delta > 1.0 {
                // Aufrunden mit Übertrag, notfalls bis vor das Komma.
                loop {
                    match fraction_digits.pop() {
                        None => {
                            integer += 1.0;
                            break;
                        }
                        Some(last) if last.saturating_add(1) < radix => {
                            fraction_digits.push(last.saturating_add(1));
                            break;
                        }
                        Some(_) => {}
                    }
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }
    // Die Stellen vor dem Komma von hinten: Ab 2⁵³ trägt der Rest der
    // Division keine Information mehr, dort stehen Nullen.
    const EXACT: f64 = 9_007_199_254_740_992.0;
    let mut reversed = String::new();
    while integer / base >= EXACT {
        integer /= base;
        reversed.push('0');
    }
    loop {
        let remainder = integer % base;
        // Der Rest einer ganzen Zahl ist ganz und liegt in [0, radix).
        let digit = remainder as u32;
        reversed.push(digit_char(digit, radix));
        integer = (integer - remainder) / base;
        if integer <= 0.0 {
            break;
        }
    }
    let mut result: String = reversed.chars().rev().collect();
    if !fraction_digits.is_empty() {
        result.push('.');
        result.extend(
            fraction_digits
                .iter()
                .map(|&digit| digit_char(digit, radix)),
        );
    }
    result
}
