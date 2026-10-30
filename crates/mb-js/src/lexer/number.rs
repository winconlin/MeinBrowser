//! Der Number-Wert von Ganzzahlen zur Basis 2, 8 und 16 (𝔽 des MV):
//! gerundet auf die nächste darstellbare Zahl, bei Gleichstand auf die mit
//! gerader Mantisse, zu große Werte werden Unendlich.

/// Bits der Mantisse einschließlich der führenden Eins.
const PRECISION: u32 = 53;

/// Der Wert der Ziffern `digits` (ASCII, ohne Präfix und Trennzeichen) zur
/// Basis `radix`, die eine Zweierpotenz sein muss.
pub(super) fn from_power_of_two_radix(digits: &str, radix: u32) -> f64 {
    let bits_per_digit = radix.trailing_zeros();
    // Die ersten bis zu 64 signifikanten Bits, dahinter nur, wie viele
    // Bits es noch sind und ob eines davon gesetzt ist.
    let mut leading: u64 = 0;
    let mut used: u32 = 0;
    let mut dropped: u64 = 0;
    let mut sticky = false;
    for digit in digits.chars() {
        let Some(value) = digit.to_digit(radix) else {
            continue;
        };
        for shift in (0..bits_per_digit).rev() {
            let bit = (value >> shift) & 1 == 1;
            if used == 0 && !bit {
                continue;
            }
            if used < u64::BITS {
                leading = (leading << 1) | u64::from(bit);
                used += 1;
            } else {
                dropped = dropped.saturating_add(1);
                sticky |= bit;
            }
        }
    }
    if used == 0 {
        return 0.0;
    }
    // Auf PRECISION Bits runden: Der Wert ist leading · 2^dropped plus
    // einen Rest unter 2^dropped, der nur über `sticky` zählt.
    let extra = used.saturating_sub(PRECISION);
    let mut mantissa = leading >> extra;
    if extra > 0 {
        let rest = leading & ((1 << extra) - 1);
        let half = 1 << (extra - 1);
        if rest > half || (rest == half && (sticky || mantissa & 1 == 1)) {
            mantissa += 1;
        }
    }
    let mut exponent = dropped.saturating_add(u64::from(extra));
    if mantissa >> PRECISION != 0 {
        mantissa >>= 1;
        exponent = exponent.saturating_add(1);
    }
    // mantissa · 2^exponent mit höchstens PRECISION Bits; normalisiert hat
    // die Mantisse genau PRECISION Bits und der Exponent wird kleiner.
    let width = u64::BITS - mantissa.leading_zeros();
    let shift = PRECISION - width;
    let mantissa = mantissa << shift;
    // Der Exponent der führenden Eins ist exponent + width - 1 ≥ 0.
    let Some(power) = exponent.checked_add(u64::from(width) - 1) else {
        return f64::INFINITY;
    };
    let biased = power.saturating_add(1023);
    if biased >= 0x7FF {
        return f64::INFINITY;
    }
    let fraction = mantissa & ((1 << (PRECISION - 1)) - 1);
    f64::from_bits((biased << (PRECISION - 1)) | fraction)
}
