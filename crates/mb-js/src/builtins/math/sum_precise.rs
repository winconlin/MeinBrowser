//! Die genaue Summe für Math.sumPrecise (Abschnitt 21.3.2.34): Jede
//! endliche Zahl ist ein ganzzahliges Vielfaches von 2⁻¹⁰⁷⁴ unter 2¹⁰²⁴,
//! also eine ganze Zahl unter 2²⁰⁹⁸ in Einheiten von 2⁻¹⁰⁷⁴. Weniger als 2⁵³
//! Summanden (mehr lässt Math.sumPrecise nicht zu) bleiben unter 2²¹⁵¹.
//! Die Summe ist daher eine Festkommazahl mit 34 Wörtern zu 64 Bit im
//! Zweierkomplement, und erst das Ergebnis wird einmal gerundet.

/// Wörter der Festkommazahl: 2176 Bit für Betrag und Vorzeichen.
const WORDS: usize = 34;
/// Bits der Mantisse einer Zahl, mit der führenden Eins.
const PRECISION: usize = 53;
/// Die Mantisse ohne die führende Eins.
const FRACTION_MASK: u64 = (1 << 52) - 1;

/// Eine Summe endlicher Zahlen ohne Rundung.
pub(super) struct ExactSum {
    /// Das niedrigste Wort zuerst.
    words: [u64; WORDS],
}

impl ExactSum {
    pub(super) fn new() -> ExactSum {
        ExactSum { words: [0; WORDS] }
    }

    /// Addiert die endliche Zahl `x`.
    pub(super) fn add(&mut self, x: f64) {
        let bits = x.to_bits();
        let biased = (bits >> 52) & 0x7FF;
        let fraction = bits & FRACTION_MASK;
        // x = mantissa · 2^(shift − 1074); subnormale Zahlen haben keine
        // führende Eins und denselben Abstand wie die kleinsten normalen.
        let (mantissa, shift) = if biased == 0 {
            (fraction, 0)
        } else {
            (fraction | (1 << 52), biased.saturating_sub(1))
        };
        if mantissa == 0 {
            return;
        }
        // shift ≤ 2045: Beide Wörter liegen innerhalb der Summe.
        let Ok(shift) = usize::try_from(shift) else {
            return;
        };
        let word = shift / 64;
        let offset = shift % 64;
        let low = mantissa << offset;
        let high = if offset == 0 {
            0
        } else {
            mantissa >> (64 - offset)
        };
        if bits >> 63 == 0 {
            self.add_at(word, low, high);
        } else {
            self.subtract_at(word, low, high);
        }
    }

    /// Addiert `low` und `high` ab dem Wort `word` mit Übertrag bis zum
    /// obersten Wort; ein Übertrag darüber hinaus fällt wie im
    /// Zweierkomplement weg.
    fn add_at(&mut self, word: usize, low: u64, high: u64) {
        let mut carry = false;
        for (index, slot) in self.words.iter_mut().enumerate().skip(word) {
            let part = match index.saturating_sub(word) {
                0 => low,
                1 => high,
                _ if !carry => break,
                _ => 0,
            };
            let (sum, first) = slot.overflowing_add(part);
            let (sum, second) = sum.overflowing_add(u64::from(carry));
            *slot = sum;
            carry = first || second;
        }
    }

    /// Wie [`ExactSum::add_at`], aber subtrahiert.
    fn subtract_at(&mut self, word: usize, low: u64, high: u64) {
        let mut borrow = false;
        for (index, slot) in self.words.iter_mut().enumerate().skip(word) {
            let part = match index.saturating_sub(word) {
                0 => low,
                1 => high,
                _ if !borrow => break,
                _ => 0,
            };
            let (difference, first) = slot.overflowing_sub(part);
            let (difference, second) = difference.overflowing_sub(u64::from(borrow));
            *slot = difference;
            borrow = first || second;
        }
    }

    /// Die Summe auf die nächste Zahl gerundet, bei Gleichstand zur geraden
    /// Mantisse, jenseits der größten endlichen Zahl ±∞ (𝔽 aus Abschnitt
    /// 5.2.5). Null ist +0.
    pub(super) fn round(&self) -> f64 {
        let negative = self.words.last().is_some_and(|&top| top >> 63 == 1);
        let magnitude = if negative {
            negate(&self.words)
        } else {
            self.words
        };
        let Some(length) = bit_length(&magnitude) else {
            return 0.0;
        };
        let sign = if negative { 1u64 << 63 } else { 0 };
        if length <= PRECISION {
            // Höchstens 53 Bit: genau darstellbar als m · 2⁻¹⁰⁷⁴, auch
            // subnormal, und die Multiplikation mit 2⁻¹⁰⁷⁴ rundet nicht.
            let mantissa = magnitude.first().copied().unwrap_or(0);
            // Unter 2⁵³ ist die Umwandlung genau.
            let value = (mantissa as f64) * f64::from_bits(1);
            return f64::from_bits(value.to_bits() | sign);
        }
        // Die Bits unter der Mantisse.
        let mut shift = length.saturating_sub(PRECISION);
        let mut mantissa = bits_at(&magnitude, shift);
        let half = bit(&magnitude, shift.saturating_sub(1));
        let sticky = any_below(&magnitude, shift.saturating_sub(1));
        if half && (sticky || mantissa & 1 == 1) {
            mantissa = mantissa.saturating_add(1);
            if mantissa == 1 << PRECISION {
                mantissa >>= 1;
                shift = shift.saturating_add(1);
            }
        }
        // Die führende Eins steht bei 2^(shift + 52 − 1074), der Exponent
        // mit Versatz 1023 ist also shift + 1.
        let biased = u64::try_from(shift.saturating_add(1)).unwrap_or(u64::MAX);
        if biased >= 0x7FF {
            return f64::from_bits(f64::INFINITY.to_bits() | sign);
        }
        f64::from_bits(sign | (biased << 52) | (mantissa & FRACTION_MASK))
    }
}

/// Das Zweierkomplement von `words`.
fn negate(words: &[u64; WORDS]) -> [u64; WORDS] {
    let mut result = [0; WORDS];
    let mut carry = true;
    for (slot, &word) in result.iter_mut().zip(words) {
        let (sum, overflow) = (!word).overflowing_add(u64::from(carry));
        *slot = sum;
        carry = overflow;
    }
    result
}

/// Die Anzahl der Bits bis zum höchsten gesetzten, `None` für null.
fn bit_length(words: &[u64; WORDS]) -> Option<usize> {
    let (index, word) = words
        .iter()
        .enumerate()
        .rev()
        .find(|(_, word)| **word != 0)?;
    let below = usize::try_from(word.leading_zeros()).unwrap_or(0);
    Some(index.saturating_mul(64).saturating_add(64 - below))
}

/// Das Bit Nummer `position`, gezählt ab 0.
fn bit(words: &[u64; WORDS], position: usize) -> bool {
    words
        .get(position / 64)
        .is_some_and(|word| (word >> (position % 64)) & 1 == 1)
}

/// Die 53 Bits ab Bit Nummer `position`.
fn bits_at(words: &[u64; WORDS], position: usize) -> u64 {
    let index = position / 64;
    let offset = position % 64;
    let low = words.get(index).copied().unwrap_or(0) >> offset;
    let high = if offset == 0 {
        0
    } else {
        words.get(index.saturating_add(1)).copied().unwrap_or(0) << (64 - offset)
    };
    (low | high) & ((1 << PRECISION) - 1)
}

/// Ob ein Bit unter Bit Nummer `position` gesetzt ist.
fn any_below(words: &[u64; WORDS], position: usize) -> bool {
    let index = position / 64;
    let offset = position % 64;
    let whole = words.iter().take(index).any(|&word| word != 0);
    let part = offset != 0
        && words
            .get(index)
            .is_some_and(|&word| word & ((1 << offset) - 1) != 0);
    whole || part
}

#[cfg(test)]
mod tests {
    use super::ExactSum;

    fn sum(values: &[f64]) -> f64 {
        let mut sum = ExactSum::new();
        for &value in values {
            sum.add(value);
        }
        sum.round()
    }

    #[test]
    fn sums_are_rounded_once() {
        assert_eq!(sum(&[]).to_bits(), 0.0f64.to_bits());
        assert_eq!(sum(&[0.1, 0.2]), 0.30000000000000004);
        assert_eq!(sum(&[1e20, 0.1, -1e20]), 0.1);
        assert_eq!(sum(&[1.0, 1e100, 1.0, -1e100]), 2.0);
        assert_eq!(sum(&[f64::MAX, f64::MAX, -f64::MAX]), f64::MAX);
        assert_eq!(sum(&[f64::MAX, f64::MAX]), f64::INFINITY);
        assert_eq!(sum(&[-f64::MAX, -f64::MAX]), f64::NEG_INFINITY);
        assert_eq!(sum(&[5e-324, 5e-324]), 1e-323);
        assert_eq!(sum(&[-5e-324]), -5e-324);
        assert_eq!(sum(&[3.0, -3.0]).to_bits(), 0.0f64.to_bits());
        // 2⁵³ + 1 liegt genau zwischen zwei Zahlen: zur geraden Mantisse.
        assert_eq!(
            sum(&[9_007_199_254_740_992.0, 1.0]),
            9_007_199_254_740_992.0
        );
        assert_eq!(
            sum(&[9_007_199_254_740_992.0, 1.0, 1e-300]),
            9_007_199_254_740_994.0
        );
        assert_eq!(
            sum(&[-9_007_199_254_740_992.0, -3.0]),
            -9_007_199_254_740_996.0
        );
    }

    #[test]
    fn the_largest_sum_rounds_to_infinity_only_past_the_last_half_step() {
        // MAX + 2⁹⁷⁰ − ε bleibt MAX, MAX + 2⁹⁷⁰ ist Gleichstand und wird ∞.
        let half_step = 2f64.powi(970);
        let below = 2f64.powi(969);
        assert_eq!(sum(&[f64::MAX, below]), f64::MAX);
        assert_eq!(sum(&[f64::MAX, half_step]), f64::INFINITY);
    }
}
