//! BigInt-Werte (Abschnitt 6.1.6.2): ganze Zahlen beliebiger Größe als
//! Vorzeichen und Betrag. Der Betrag besteht aus 64-Bit-Wörtern, das
//! niedrigste zuerst, ohne führende Nullwörter; Null hat kein Wort und
//! ist nie negativ. So hat jeder Wert genau eine Darstellung, und zwei
//! Werte sind gleich, wenn ihre Darstellungen gleich sind.
//!
//! Die Rechenarten folgen den Schulverfahren: Addition und Subtraktion
//! linear, Multiplikation und Division (Knuth, Algorithmus D) quadratisch
//! in der Zahl der Wörter, die Potenz durch Quadrieren. Bitweise Operatoren
//! rechnen im unendlichen Zweierkomplement, wie Abschnitt 6.1.6.2 es
//! verlangt. Alles kommt ohne Rekursion aus.
//!
//! Die Spezifikation kennt keine Obergrenze. Wie andere Engines wirft
//! diese einen RangeError für Ergebnisse über [`MAX_BITS`] Bits. Vor jeder
//! aufwendigen Rechnung fragt sie ihr [`Budget`]: ob der Speicher für das
//! Ergebnis frei ist und ob die Arbeit bezahlt werden kann. Arbeit zählt in
//! Einheiten von ungefähr einer Multiplikation zweier Wörter mit Addition.

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::fmt;

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::heap::{Local, Scope};
use crate::value::Kind;

/// Höchstens so viele Bits hat ein BigInt, 2³⁰ wie in V8. Ein solcher
/// Wert belegt 128 MiB.
pub(crate) const MAX_BITS: u64 = 1 << 30;
/// Höchstens so viele Wörter hat ein Betrag.
const MAX_LIMBS: usize = 1 << 24;

/// Eine ganze Zahl.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BigInt {
    negative: bool,
    magnitude: Vec<u64>,
}

/// Warum eine Rechnung nicht zu Ende kam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Das Ergebnis hätte mehr als [`MAX_BITS`] Bits.
    TooLarge,
    /// Das Ergebnis passt nicht in den freien Speicher.
    OutOfMemory,
    /// Das Budget hat die Arbeit nicht bezahlt.
    Stopped,
}

/// Was eine Rechnung verbrauchen darf: Bytes außerhalb des Heaps und
/// Arbeit, die `spend` bezahlt.
pub(crate) struct Budget<'a> {
    pub(crate) bytes: usize,
    pub(crate) spend: &'a mut dyn FnMut(u64) -> bool,
}

/// So viele Einheiten kostet jedes Wort des Ergebnisses zusätzlich zur
/// Rechnung: Werte aus dem Heap lesen und das Ergebnis dort anlegen kostet
/// bei den Rechnungen mit linearem Aufwand mehr als die Rechnung selbst
/// (Messung bei [`compute`]).
const COPY_WORK: u64 = 8;

impl Budget<'_> {
    /// Fragt vor einer Rechnung mit `work` Einheiten, deren Ergebnis bis zu
    /// `limbs` Wörter hat; jedes Wort kostet dazu [`COPY_WORK`] Einheiten.
    fn take(&mut self, work: u64, limbs: usize) -> Result<(), Failure> {
        if limbs > MAX_LIMBS {
            return Err(Failure::TooLarge);
        }
        if limbs.saturating_mul(8) > self.bytes {
            return Err(Failure::OutOfMemory);
        }
        if (self.spend)(work.saturating_add(count(limbs).saturating_mul(COPY_WORK))) {
            Ok(())
        } else {
            Err(Failure::Stopped)
        }
    }
}

fn low(value: u128) -> u64 {
    // Die Maske lässt 64 Bit übrig; die Umwandlung verliert nichts.
    (value & u128::from(u64::MAX)) as u64
}

fn high(value: u128) -> u64 {
    // Nach der Verschiebung bleiben 64 Bit.
    (value >> 64) as u64
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn work(a: usize, b: usize) -> u64 {
    count(a).saturating_mul(count(b))
}

/// Entfernt führende Nullwörter.
fn trim(mut words: Vec<u64>) -> Vec<u64> {
    while words.last() == Some(&0) {
        words.pop();
    }
    words
}

fn bit_length_of(words: &[u64]) -> u64 {
    match words.last() {
        None => 0,
        Some(&top) => count(words.len() - 1)
            .saturating_mul(64)
            .saturating_add(u64::from(64 - top.leading_zeros())),
    }
}

/// Ob der Betrag eine Zweierpotenz ist: genau ein gesetztes Bit.
fn is_power_of_two(words: &[u64]) -> bool {
    match words.split_last() {
        Some((top, rest)) => top.is_power_of_two() && rest.iter().all(|&word| word == 0),
        None => false,
    }
}

/// Vergleicht zwei Beträge.
fn compare_magnitudes(a: &[u64], b: &[u64]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn add_magnitudes(a: &[u64], b: &[u64]) -> Vec<u64> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut sum = Vec::with_capacity(long.len() + 1);
    let mut carry = false;
    for (index, &x) in long.iter().enumerate() {
        let y = short.get(index).copied().unwrap_or(0);
        let (partial, first) = x.overflowing_add(y);
        let (partial, second) = partial.overflowing_add(u64::from(carry));
        sum.push(partial);
        carry = first || second;
    }
    if carry {
        sum.push(1);
    }
    sum
}

/// `a - b` für `a >= b`.
fn subtract_magnitudes(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut difference = Vec::with_capacity(a.len());
    let mut borrow = false;
    for (index, &x) in a.iter().enumerate() {
        let y = b.get(index).copied().unwrap_or(0);
        let (partial, first) = x.overflowing_sub(y);
        let (partial, second) = partial.overflowing_sub(u64::from(borrow));
        difference.push(partial);
        borrow = first || second;
    }
    trim(difference)
}

fn multiply_magnitudes(a: &[u64], b: &[u64]) -> Vec<u64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut product = vec![0u64; a.len() + b.len()];
    for (index, &x) in a.iter().enumerate() {
        let Some(row) = product.get_mut(index..) else {
            continue;
        };
        let mut carry = 0u128;
        for (slot, &y) in row.iter_mut().zip(b) {
            let term = u128::from(x) * u128::from(y) + u128::from(*slot) + carry;
            *slot = low(term);
            carry = u128::from(high(term));
        }
        // Dieses Wort hat noch keine Zeile beschrieben.
        if let Some(slot) = row.get_mut(b.len()) {
            *slot = low(carry);
        }
    }
    trim(product)
}

/// Teilt durch ein einzelnes Wort ungleich null.
fn divide_by_word(a: &[u64], divisor: u64) -> (Vec<u64>, u64) {
    let mut quotient = vec![0u64; a.len()];
    let mut remainder = 0u64;
    for (slot, &word) in quotient.iter_mut().zip(a).rev() {
        let value = (u128::from(remainder) << 64) | u128::from(word);
        *slot = low(value / u128::from(divisor));
        remainder = low(value % u128::from(divisor));
    }
    (trim(quotient), remainder)
}

/// Verschiebt um `shift` Bit (unter 64) nach links, mit einem Wort mehr.
fn shift_words_left(words: &[u64], shift: u32) -> Vec<u64> {
    let mut shifted = Vec::with_capacity(words.len() + 1);
    let mut carry = 0u64;
    for &word in words {
        if shift == 0 {
            shifted.push(word);
        } else {
            shifted.push((word << shift) | carry);
            carry = word >> (64 - shift);
        }
    }
    shifted.push(carry);
    shifted
}

/// Quotient und Rest zweier Beträge, der Divisor ungleich null (Knuth,
/// The Art of Computer Programming, Band 2, Abschnitt 4.3.1,
/// Algorithmus D).
fn divide_magnitudes(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    if compare_magnitudes(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if let [divisor] = b {
        let (quotient, remainder) = divide_by_word(a, *divisor);
        return (quotient, trim(vec![remainder]));
    }
    let n = b.len();
    let shift = b.last().map_or(0, |top| top.leading_zeros());
    // D1: Der Divisor bekommt ein gesetztes oberstes Bit; das Wort, das
    // er dabei abgibt, ist null.
    let mut divisor = shift_words_left(b, shift);
    divisor.pop();
    let mut rest = shift_words_left(a, shift);
    let top = divisor.last().copied().unwrap_or(1);
    let second = divisor.get(n - 2).copied().unwrap_or(0);
    let mut quotient = vec![0u64; a.len() - n + 1];
    for j in (0..=a.len() - n).rev() {
        // D3: den Quotienten schätzen.
        let upper = rest.get(j + n).copied().unwrap_or(0);
        let lower = rest.get(j + n - 1).copied().unwrap_or(0);
        let next = rest.get(j + n - 2).copied().unwrap_or(0);
        let numerator = (u128::from(upper) << 64) | u128::from(lower);
        let mut estimate = numerator / u128::from(top);
        let mut remainder = numerator % u128::from(top);
        while estimate > u128::from(u64::MAX)
            || estimate * u128::from(second) > ((remainder << 64) | u128::from(next))
        {
            estimate -= 1;
            remainder += u128::from(top);
            if remainder > u128::from(u64::MAX) {
                break;
            }
        }
        // D4: multiplizieren und abziehen.
        let mut carry = 0u128;
        let mut borrow = false;
        if let Some(window) = rest.get_mut(j..=j + n) {
            for (index, slot) in window.iter_mut().enumerate() {
                let product = match divisor.get(index) {
                    Some(&digit) => estimate * u128::from(digit) + carry,
                    None => carry,
                };
                carry = u128::from(high(product));
                let (partial, first) = slot.overflowing_sub(low(product));
                let (partial, second) = partial.overflowing_sub(u64::from(borrow));
                *slot = partial;
                borrow = first || second;
            }
            // D6: Die Schätzung war um eins zu groß; einmal zurück addieren.
            if borrow {
                estimate -= 1;
                let mut carry = false;
                for (index, slot) in window.iter_mut().enumerate() {
                    let digit = divisor.get(index).copied().unwrap_or(0);
                    let (partial, first) = slot.overflowing_add(digit);
                    let (partial, second) = partial.overflowing_add(u64::from(carry));
                    *slot = partial;
                    carry = first || second;
                }
            }
        }
        if let Some(slot) = quotient.get_mut(j) {
            *slot = low(estimate);
        }
    }
    // D8: Der Rest steht in den unteren n Wörtern, noch verschoben.
    rest.truncate(n);
    let remainder = if shift == 0 {
        rest
    } else {
        let mut shifted = Vec::with_capacity(n);
        for index in 0..n {
            let word = rest.get(index).copied().unwrap_or(0);
            let above = rest.get(index + 1).copied().unwrap_or(0);
            shifted.push((word >> shift) | (above << (64 - shift)));
        }
        shifted
    };
    (trim(quotient), trim(remainder))
}

/// Das unendliche Zweierkomplement eines Werts in `length` Wörtern.
fn twos_complement(value: &BigInt, length: usize) -> Vec<u64> {
    let mut words: Vec<u64> = (0..length)
        .map(|index| value.magnitude.get(index).copied().unwrap_or(0))
        .collect();
    if value.negative {
        negate_words(&mut words);
    }
    words
}

/// Bildet das Zweierkomplement in place: alle Bits umkehren, eins dazu.
fn negate_words(words: &mut [u64]) {
    let mut carry = true;
    for word in words {
        let (partial, overflow) = (!*word).overflowing_add(u64::from(carry));
        *word = partial;
        carry = overflow;
    }
}

impl BigInt {
    pub(crate) fn zero() -> BigInt {
        BigInt::default()
    }

    pub(crate) fn from_u64(value: u64) -> BigInt {
        BigInt {
            negative: false,
            magnitude: trim(vec![value]),
        }
    }

    pub(crate) fn from_i64(value: i64) -> BigInt {
        let mut result = BigInt::from_u64(value.unsigned_abs());
        result.negative = value < 0;
        result
    }

    /// Ein Wert aus Vorzeichen und Betrag, der führende Nullwörter haben
    /// darf.
    pub(crate) fn from_parts(negative: bool, magnitude: Vec<u64>) -> Result<BigInt, Failure> {
        let magnitude = trim(magnitude);
        if magnitude.len() > MAX_LIMBS {
            return Err(Failure::TooLarge);
        }
        Ok(BigInt {
            negative: negative && !magnitude.is_empty(),
            magnitude,
        })
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.magnitude.is_empty()
    }

    pub(crate) fn is_negative(&self) -> bool {
        self.negative
    }

    /// Die Wörter des Betrags, das niedrigste zuerst.
    pub(crate) fn magnitude(&self) -> &[u64] {
        &self.magnitude
    }

    /// Die Zahl der Bits des Betrags.
    pub(crate) fn bit_length(&self) -> u64 {
        bit_length_of(&self.magnitude)
    }

    /// Der Wert, wenn er in ein `u64` passt und nicht negativ ist.
    pub(crate) fn to_u64(&self) -> Option<u64> {
        match (self.negative, self.magnitude.as_slice()) {
            (false, []) => Some(0),
            (false, [word]) => Some(*word),
            _ => None,
        }
    }

    /// BigInt::unaryMinus (Abschnitt 6.1.6.2.1).
    pub(crate) fn negate(&self) -> BigInt {
        BigInt {
            negative: !self.negative && !self.is_zero(),
            magnitude: self.magnitude.clone(),
        }
    }

    /// BigInt::add (Abschnitt 6.1.6.2.7).
    pub(crate) fn add(&self, other: &BigInt, budget: &mut Budget<'_>) -> Result<BigInt, Failure> {
        let longest = self.magnitude.len().max(other.magnitude.len());
        budget.take(count(longest), longest.saturating_add(1))?;
        if self.negative == other.negative {
            return BigInt::from_parts(
                self.negative,
                add_magnitudes(&self.magnitude, &other.magnitude),
            );
        }
        match compare_magnitudes(&self.magnitude, &other.magnitude) {
            Ordering::Less => BigInt::from_parts(
                other.negative,
                subtract_magnitudes(&other.magnitude, &self.magnitude),
            ),
            _ => BigInt::from_parts(
                self.negative,
                subtract_magnitudes(&self.magnitude, &other.magnitude),
            ),
        }
    }

    /// BigInt::subtract (Abschnitt 6.1.6.2.8).
    pub(crate) fn subtract(
        &self,
        other: &BigInt,
        budget: &mut Budget<'_>,
    ) -> Result<BigInt, Failure> {
        self.add(&other.negate(), budget)
    }

    /// BigInt::multiply (Abschnitt 6.1.6.2.4).
    pub(crate) fn multiply(
        &self,
        other: &BigInt,
        budget: &mut Budget<'_>,
    ) -> Result<BigInt, Failure> {
        let (a, b) = (self.magnitude.len(), other.magnitude.len());
        if self.bit_length().saturating_add(other.bit_length()) > MAX_BITS.saturating_add(1) {
            return Err(Failure::TooLarge);
        }
        budget.take(work(a, b), a.saturating_add(b))?;
        BigInt::from_parts(
            self.negative != other.negative,
            multiply_magnitudes(&self.magnitude, &other.magnitude),
        )
    }

    /// Quotient nach BigInt::divide (Abschnitt 6.1.6.2.5, zur Null hin
    /// gerundet) und Rest nach BigInt::remainder (Abschnitt 6.1.6.2.6, mit
    /// dem Vorzeichen des Dividenden); `None` für den Divisor 0.
    pub(crate) fn divide(
        &self,
        other: &BigInt,
        budget: &mut Budget<'_>,
    ) -> Result<Option<(BigInt, BigInt)>, Failure> {
        if other.is_zero() {
            return Ok(None);
        }
        let (a, b) = (self.magnitude.len(), other.magnitude.len());
        let rows = a.saturating_sub(b).saturating_add(1);
        budget.take(work(rows, b).saturating_add(count(a)), a.saturating_add(2))?;
        let (quotient, remainder) = divide_magnitudes(&self.magnitude, &other.magnitude);
        Ok(Some((
            BigInt::from_parts(self.negative != other.negative, quotient)?,
            BigInt::from_parts(self.negative, remainder)?,
        )))
    }

    /// BigInt::exponentiate (Abschnitt 6.1.6.2.3) für einen Exponenten,
    /// der nicht negativ ist.
    pub(crate) fn exponentiate(
        &self,
        exponent: &BigInt,
        budget: &mut Budget<'_>,
    ) -> Result<BigInt, Failure> {
        if exponent.is_zero() {
            return Ok(BigInt::from_u64(1));
        }
        if self.is_zero() {
            return Ok(BigInt::zero());
        }
        let odd = exponent.magnitude.first().is_some_and(|word| word & 1 == 1);
        if self.magnitude.as_slice() == [1] {
            return Ok(BigInt {
                negative: self.negative && odd,
                magnitude: vec![1],
            });
        }
        // Jedes andere Ergebnis hat mindestens so viele Bits wie der
        // Exponent angibt.
        let Some(power) = exponent.to_u64() else {
            return Err(Failure::TooLarge);
        };
        let least = self
            .bit_length()
            .saturating_sub(1)
            .saturating_mul(power)
            .saturating_add(1);
        if least > MAX_BITS {
            return Err(Failure::TooLarge);
        }
        // Passt schon das Ergebnis nicht in den Speicher, beginnt die
        // Rechnung gar nicht erst.
        budget.take(0, usize::try_from(least.div_ceil(64)).unwrap_or(usize::MAX))?;
        let mut result = BigInt::from_u64(1);
        for bit in (0..64 - power.leading_zeros()).rev() {
            result = result.multiply(&result, budget)?;
            if power >> bit & 1 == 1 {
                result = result.multiply(self, budget)?;
            }
        }
        Ok(result)
    }

    /// BigInt::leftShift (Abschnitt 6.1.6.2.9) um `bits` Bit nach links,
    /// für negative Weiten nach rechts mit Abrunden. Die Weite ist ein
    /// BigInt, damit auch sehr große Weiten richtig wirken.
    pub(crate) fn shift_left(
        &self,
        bits: &BigInt,
        budget: &mut Budget<'_>,
    ) -> Result<BigInt, Failure> {
        if bits.negative {
            return self.shift_right(&bits.negate(), budget);
        }
        if self.is_zero() {
            return Ok(BigInt::zero());
        }
        let Some(bits) = bits.to_u64() else {
            return Err(Failure::TooLarge);
        };
        if self.bit_length().saturating_add(bits) > MAX_BITS {
            return Err(Failure::TooLarge);
        }
        let words = usize::try_from(bits / 64).map_err(|_| Failure::TooLarge)?;
        let length = usize::try_from(self.bit_length().saturating_add(bits).div_ceil(64))
            .map_err(|_| Failure::TooLarge)?;
        budget.take(count(length), length)?;
        let shift = u32::try_from(bits % 64).unwrap_or(0);
        let mut magnitude = vec![0u64; words];
        magnitude.extend(shift_words_left(&self.magnitude, shift));
        BigInt::from_parts(self.negative, magnitude)
    }

    /// Nach rechts um `bits` Bit, zur negativen Unendlichkeit hin
    /// gerundet; der Kern von BigInt::signedRightShift (Abschnitt
    /// 6.1.6.2.10).
    pub(crate) fn shift_right(
        &self,
        bits: &BigInt,
        budget: &mut Budget<'_>,
    ) -> Result<BigInt, Failure> {
        if bits.negative {
            return self.shift_left(&bits.negate(), budget);
        }
        let bits = bits.to_u64().unwrap_or(u64::MAX);
        if bits >= self.bit_length() {
            return Ok(if self.negative {
                BigInt::from_i64(-1)
            } else {
                BigInt::zero()
            });
        }
        budget.take(count(self.magnitude.len()), self.magnitude.len())?;
        // Unter der Bitlänge, also auch unter der Zahl der Wörter.
        let words = usize::try_from(bits / 64).unwrap_or(0);
        let shift = u32::try_from(bits % 64).unwrap_or(0);
        let rest = self.magnitude.get(words..).unwrap_or_default();
        let mut magnitude = Vec::with_capacity(rest.len());
        for (index, &word) in rest.iter().enumerate() {
            let above = rest.get(index + 1).copied().unwrap_or(0);
            magnitude.push(if shift == 0 {
                word
            } else {
                (word >> shift) | (above << (64 - shift))
            });
        }
        let dropped = self
            .magnitude
            .get(..words)
            .unwrap_or_default()
            .iter()
            .any(|&word| word != 0)
            || (shift > 0 && rest.first().is_some_and(|&word| word << (64 - shift) != 0));
        let magnitude = trim(magnitude);
        if self.negative && dropped {
            // Abrunden: Für negative Werte wird der Betrag größer.
            return BigInt::from_parts(true, add_magnitudes(&magnitude, &[1]));
        }
        BigInt::from_parts(self.negative, magnitude)
    }

    /// BigInt::unaryMinus für den Operator `-`, mit Budget für die Kopie.
    pub(crate) fn negate_within(&self, budget: &mut Budget<'_>) -> Result<BigInt, Failure> {
        let length = self.magnitude.len();
        budget.take(count(length), length)?;
        Ok(self.negate())
    }

    /// BigInt::bitwiseNOT (Abschnitt 6.1.6.2.2): -x - 1.
    pub(crate) fn not(&self, budget: &mut Budget<'_>) -> Result<BigInt, Failure> {
        let length = self.magnitude.len();
        budget.take(count(length), length.saturating_add(1))?;
        if self.negative {
            BigInt::from_parts(false, subtract_magnitudes(&self.magnitude, &[1]))
        } else {
            BigInt::from_parts(true, add_magnitudes(&self.magnitude, &[1]))
        }
    }

    /// BigInt::bitwiseAND, bitwiseXOR und bitwiseOR (Abschnitte 6.1.6.2.18
    /// bis 6.1.6.2.20) im unendlichen Zweierkomplement.
    pub(crate) fn bitwise(
        &self,
        op: Bitwise,
        other: &BigInt,
        budget: &mut Budget<'_>,
    ) -> Result<BigInt, Failure> {
        let length = self
            .magnitude
            .len()
            .max(other.magnitude.len())
            .saturating_add(1);
        budget.take(count(length), length)?;
        let a = twos_complement(self, length);
        let b = twos_complement(other, length);
        let mut words: Vec<u64> = a
            .iter()
            .zip(&b)
            .map(|(&x, &y)| match op {
                Bitwise::And => x & y,
                Bitwise::Xor => x ^ y,
                Bitwise::Or => x | y,
            })
            .collect();
        let negative = words.last().is_some_and(|&top| top >> 63 == 1);
        if negative {
            negate_words(&mut words);
        }
        BigInt::from_parts(negative, words)
    }

    /// BigInt.asUintN (Abschnitt 21.2.2.2) ab Schritt 3: der Wert modulo
    /// 2^`bits`.
    pub(crate) fn as_uint_n(&self, bits: u64, budget: &mut Budget<'_>) -> Result<BigInt, Failure> {
        if bits == 0 {
            return Ok(BigInt::zero());
        }
        // Gelesen wird der ganze Wert, auch wenn er bleibt; das Ergebnis
        // kostet seine Wörter wie bei den anderen Rechnungen.
        let read = self.magnitude.len();
        if !self.negative && self.bit_length() <= bits {
            budget.take(count(read), read)?;
            return Ok(self.clone());
        }
        // Die Wörter, die der Rest braucht; die Bits darüber fallen weg.
        let length = bits.div_ceil(64);
        let kept = usize::try_from(length).map_or(read, |length| length.min(read));
        budget.take(count(read), kept)?;
        let mut words = self.magnitude.get(..kept).unwrap_or_default().to_vec();
        mask(&mut words, bits);
        if !self.negative || words.iter().all(|&word| word == 0) {
            return BigInt::from_parts(false, words);
        }
        // Für negative Werte 2^bits - (|x| mod 2^bits).
        if bits > MAX_BITS {
            return Err(Failure::TooLarge);
        }
        let length = usize::try_from(length).map_err(|_| Failure::TooLarge)?;
        budget.take(count(length), length)?;
        words.resize(length, 0);
        negate_words(&mut words);
        mask(&mut words, bits);
        BigInt::from_parts(false, words)
    }

    /// BigInt.asIntN (Abschnitt 21.2.2.1) ab Schritt 3: der Wert modulo
    /// 2^`bits`, ab 2^(`bits` - 1) negativ.
    pub(crate) fn as_int_n(&self, bits: u64, budget: &mut Budget<'_>) -> Result<BigInt, Failure> {
        if bits == 0 {
            return Ok(BigInt::zero());
        }
        // Gelesen wird der ganze Wert, auch wenn er bleibt.
        let read = self.magnitude.len();
        budget.take(count(read), 0)?;
        // Passt der Wert schon, bleibt er: -2^(bits-1) <= x < 2^(bits-1).
        // Für negative Werte zählt die Bitlänge von |x| - 1; sie ist nur
        // für Zweierpotenzen kleiner als die von |x|.
        let needed = if self.negative && is_power_of_two(&self.magnitude) {
            self.bit_length().saturating_sub(1)
        } else {
            self.bit_length()
        };
        if needed < bits {
            budget.take(0, read)?;
            return Ok(self.clone());
        }
        // Sonst ist `bits` höchstens so groß wie die Bitlänge.
        let modulo = self.as_uint_n(bits, budget)?;
        let top = bits - 1;
        let set = usize::try_from(top / 64)
            .ok()
            .and_then(|word| modulo.magnitude.get(word))
            .is_some_and(|&word| word >> (top % 64) & 1 == 1);
        if !set {
            return Ok(modulo);
        }
        // modulo - 2^bits = -(2^bits - modulo); der Betrag ist das
        // Zweierkomplement in `bits` Bit und hat höchstens `bits` Bit.
        let length = usize::try_from(bits.div_ceil(64)).map_err(|_| Failure::TooLarge)?;
        budget.take(count(length), length)?;
        let mut words = modulo.magnitude;
        words.resize(length, 0);
        negate_words(&mut words);
        mask(&mut words, bits);
        BigInt::from_parts(true, words)
    }

    /// Der Wert als nächste Zahl, bei Gleichstand die mit gerader Mantisse
    /// (𝔽(ℝ(x)), Abschnitt 6.1.6.1).
    pub(crate) fn to_f64(&self) -> f64 {
        let length = self.bit_length();
        let magnitude = if length <= 64 {
            // Die Umwandlung von u64 nach f64 rundet so.
            self.magnitude.first().copied().unwrap_or(0) as f64
        } else {
            // Die obersten 64 Bit, das unterste davon gesetzt, wenn darunter
            // ein Bit gesetzt ist. Es liegt unter der Rundungsstelle einer
            // 53-Bit-Mantisse und entscheidet so nur Gleichstände.
            let shift = length - 64;
            let words = usize::try_from(shift / 64).unwrap_or(usize::MAX);
            let bit = u32::try_from(shift % 64).unwrap_or(0);
            let word = self.magnitude.get(words).copied().unwrap_or(0);
            let above = self
                .magnitude
                .get(words.saturating_add(1))
                .copied()
                .unwrap_or(0);
            let top = if bit == 0 {
                word
            } else {
                (word >> bit) | (above << (64 - bit))
            };
            let sticky = self
                .magnitude
                .get(..words)
                .unwrap_or_default()
                .iter()
                .any(|&word| word != 0)
                || (bit > 0 && word << (64 - bit) != 0);
            let top = top | u64::from(sticky);
            // Ab 2^1024 ist das Ergebnis unendlich; die Exponenten bis
            // dorthin liegen im Bereich von powi.
            match i32::try_from(shift) {
                Ok(exponent) if exponent <= 1024 => (top as f64) * 2f64.powi(exponent),
                _ => f64::INFINITY,
            }
        };
        if self.negative { -magnitude } else { magnitude }
    }

    /// Die ganze Zahl `number` als BigInt, `None`, wenn sie keine ganze
    /// Zahl ist (Abschnitt 21.2.1.1.1, NumberToBigInt, ohne den Fehler).
    pub(crate) fn from_f64(number: f64) -> Option<BigInt> {
        if !number.is_finite() || number.fract() != 0.0 {
            return None;
        }
        let bits = number.to_bits();
        let exponent = i64::try_from((bits >> 52) & 0x7FF).unwrap_or(0);
        if exponent == 0 {
            // Unter 1 ist nur die Null eine ganze Zahl.
            return Some(BigInt::zero());
        }
        let mantissa = (bits & ((1 << 52) - 1)) | (1 << 52);
        let shift = exponent - 1075;
        let mut magnitude = if shift >= 0 {
            let shift = u32::try_from(shift).unwrap_or(0);
            let words = usize::try_from(shift / 64).unwrap_or(0);
            let mut magnitude = vec![0u64; words];
            magnitude.extend(shift_words_left(&[mantissa], shift % 64));
            magnitude
        } else {
            // Ganzzahlig: Die Bits unter der Stelle sind null.
            vec![mantissa >> shift.unsigned_abs().min(63)]
        };
        magnitude = trim(magnitude);
        Some(BigInt {
            negative: number < 0.0 && !magnitude.is_empty(),
            magnitude,
        })
    }

    /// Vergleicht mit einer Zahl, `None` für NaN.
    pub(crate) fn compare_number(&self, number: f64) -> Option<Ordering> {
        if number.is_nan() {
            return None;
        }
        if number == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if number == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        let floor = number.floor();
        let order = self.cmp(&BigInt::from_f64(floor)?);
        // Liegt die Zahl über ihrer Abrundung, ist ein gleicher Wert kleiner.
        Some(if order == Ordering::Equal && number > floor {
            Ordering::Less
        } else {
            order
        })
    }

    /// BigInt::toString (Abschnitt 6.1.6.2.23) zur Basis `radix` (2 bis
    /// 36), mit kleinen Buchstaben.
    pub(crate) fn to_radix_string(
        &self,
        radix: u32,
        budget: &mut Budget<'_>,
    ) -> Result<Vec<u8>, Failure> {
        let radix = radix.clamp(2, 36);
        // Ziffern je Wort: die größte Potenz der Basis unter 2^64.
        let mut chunk = u64::from(radix);
        let mut per_chunk = 1usize;
        while let Some(next) = chunk.checked_mul(u64::from(radix)) {
            chunk = next;
            per_chunk += 1;
        }
        let length = self.magnitude.len();
        let digits = usize::try_from(self.bit_length())
            .unwrap_or(usize::MAX)
            .saturating_add(1);
        budget.take(
            work(length, length).saturating_add(count(digits)),
            digits.div_ceil(8),
        )?;
        let mut text = Vec::new();
        let mut rest = self.magnitude.clone();
        while !rest.is_empty() {
            let (quotient, mut remainder) = divide_by_word(&rest, chunk);
            rest = quotient;
            for _ in 0..per_chunk {
                if rest.is_empty() && remainder == 0 {
                    break;
                }
                text.push(digit(remainder % u64::from(radix)));
                remainder /= u64::from(radix);
            }
        }
        if text.is_empty() {
            text.push(b'0');
        }
        if self.negative {
            text.push(b'-');
        }
        text.reverse();
        Ok(text)
    }

    /// Der Wert eines Literals: Ziffern ohne Präfix und Trennzeichen zur
    /// Basis 2, 8, 10 oder 16. Ohne Grenzen für Arbeit und Speicher, denn
    /// den Umfang begrenzt der Quelltext; `None` für ungültige Ziffern oder
    /// mehr als [`MAX_BITS`] Bits.
    pub fn from_literal(radix: u32, digits: &[u16]) -> Option<BigInt> {
        let digits: Vec<u8> = digits
            .iter()
            .map(|&unit| {
                u8::try_from(unit)
                    .ok()
                    .filter(|&byte| char::from(byte).is_digit(radix))
            })
            .collect::<Option<_>>()?;
        let mut spend = |_| true;
        let mut budget = Budget {
            bytes: usize::MAX,
            spend: &mut spend,
        };
        BigInt::from_digits(radix, &digits, &mut budget).ok()
    }

    /// Der Wert der Ziffern `digits` (ASCII, alle gültig) zur Basis
    /// `radix` (2 bis 36).
    pub(crate) fn from_digits(
        radix: u32,
        digits: &[u8],
        budget: &mut Budget<'_>,
    ) -> Result<BigInt, Failure> {
        let radix = radix.clamp(2, 36);
        let digits = digits
            .iter()
            .position(|&digit| digit != b'0')
            .and_then(|start| digits.get(start..))
            .unwrap_or_default();
        // Jede Ziffer trägt höchstens log2(36) < 6 Bit bei, mindestens
        // log2(radix) ab der zweiten.
        let least = count(digits.len().saturating_sub(1)).saturating_mul(u64::from(radix.ilog2()));
        if least > MAX_BITS {
            return Err(Failure::TooLarge);
        }
        let words = digits
            .len()
            .saturating_mul(6)
            .div_ceil(64)
            .saturating_add(1);
        budget.take(
            work(words, words).saturating_add(count(digits.len())),
            words,
        )?;
        let mut chunk = u64::from(radix);
        let mut per_chunk = 1usize;
        while let Some(next) = chunk.checked_mul(u64::from(radix)) {
            chunk = next;
            per_chunk += 1;
        }
        let mut magnitude: Vec<u64> = Vec::new();
        for group in digits.chunks(per_chunk) {
            let mut scale = 1u64;
            let mut value = 0u64;
            for &byte in group {
                // Höchstens per_chunk Ziffern passen in ein Wort.
                scale = scale.saturating_mul(u64::from(radix));
                value = value
                    .saturating_mul(u64::from(radix))
                    .saturating_add(u64::from(digit_value(byte)));
            }
            let mut carry = u128::from(value);
            for word in &mut magnitude {
                let term = u128::from(*word) * u128::from(scale) + carry;
                *word = low(term);
                carry = u128::from(high(term));
            }
            if carry != 0 {
                magnitude.push(low(carry));
            }
        }
        BigInt::from_parts(false, magnitude)
    }
}

/// Löscht alle Bits ab `bits`.
fn mask(words: &mut [u64], bits: u64) {
    for (index, word) in words.iter_mut().enumerate() {
        let start = count(index).saturating_mul(64);
        if start >= bits {
            *word = 0;
        } else if bits - start < 64 {
            *word &= (1u64 << (bits - start)) - 1;
        }
    }
}

fn digit(value: u64) -> u8 {
    b"0123456789abcdefghijklmnopqrstuvwxyz"
        .get(usize::try_from(value).unwrap_or(0))
        .copied()
        .unwrap_or(b'0')
}

fn digit_value(byte: u8) -> u32 {
    char::from(byte).to_digit(36).unwrap_or(0)
}

/// Die bitweisen Operatoren.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bitwise {
    And,
    Xor,
    Or,
}

impl Ord for BigInt {
    /// BigInt::lessThan (Abschnitt 6.1.6.2.12) als Ordnung.
    fn cmp(&self, other: &BigInt) -> Ordering {
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => compare_magnitudes(&self.magnitude, &other.magnitude),
            (true, true) => compare_magnitudes(&other.magnitude, &self.magnitude),
        }
    }
}

impl fmt::Display for BigInt {
    /// Die Dezimalziffern, ohne Grenzen für Arbeit und Speicher.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut spend = |_| true;
        let mut budget = Budget {
            bytes: usize::MAX,
            spend: &mut spend,
        };
        let text = self
            .to_radix_string(10, &mut budget)
            .map_err(|_| fmt::Error)?;
        formatter.write_str(&String::from_utf8_lossy(&text))
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &BigInt) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Anbindung an den Agent

/// So viele Einheiten Arbeit kosten einen Arbeitsschritt. Eine Einheit ist
/// ungefähr ein Produkt zweier Wörter beim Multiplizieren.
const WORK_PER_STEP: u64 = 64;

/// Rechnet mit dem Budget des Agents: Arbeit bezahlen die Arbeitsquoten,
/// [`WORK_PER_STEP`] Einheiten je Schritt, und Rust darf belegen, was vom
/// Speicherbudget des Heaps frei ist. Ohne laufende Ausführung, etwa wenn
/// der Host ToString aufruft, gibt es keine Quote und die Arbeit ist frei.
/// Ein Ergebnis über [`MAX_BITS`] Bits wird ein RangeError.
///
/// [`WORK_PER_STEP`] und [`COPY_WORK`] sind gewählt und danach einmal
/// gemessen (Release, ein Kern eines Xeon mit 2,1 GHz, 20.11.2026), mit
/// Schleifen, die eine Quote von 20.000.000 Schritten aufbrauchen: Ein
/// Schritt des Interpreters dauerte 21 ns in einer leeren Schleife und
/// 115 ns mit einer Addition von Zahlen. Ein so bezahlter Schritt dauerte
/// beim Multiplizieren von Werten mit 100 und 1.000 Wörtern 69 bis 74 ns,
/// beim Dividieren 127 ns, beim Addieren, Verschieben und bei `&` mit
/// 200 Wörtern 64 bis 86 ns, bei `toString()` 138 ns und beim Lesen aus
/// Text mit `BigInt` 39 ns. Ohne [`COPY_WORK`] dauerte ein Schritt bei den
/// Rechnungen mit linearem Aufwand 246 bis 349 ns, denn das Lesen der
/// Werte aus dem Heap und das Anlegen des Ergebnisses kosteten mehr als die
/// Rechnung.
pub(crate) fn compute<T>(
    agent: &mut Agent,
    f: impl FnOnce(&mut Budget<'_>) -> Result<T, Failure>,
) -> Completion<T> {
    let metered = agent.has_execution_budget();
    let heap = agent.heap();
    let bytes = heap.limit().saturating_sub(heap.used_bytes());
    let mut stopped = None;
    let result = {
        let agent: &Agent = agent;
        let mut carry = 0u64;
        let mut spend = |work: u64| {
            if !metered {
                return true;
            }
            let total = carry.saturating_add(work);
            carry = total % WORK_PER_STEP;
            match agent.spend_execution_steps(total / WORK_PER_STEP) {
                Ok(()) => true,
                Err(abrupt) => {
                    stopped = Some(abrupt);
                    false
                }
            }
        };
        let mut budget = Budget {
            bytes,
            spend: &mut spend,
        };
        f(&mut budget)
    };
    match result {
        Ok(value) => Ok(value),
        Err(Failure::TooLarge) => Err(agent.throw_error(ErrorKind::RangeError)),
        Err(Failure::OutOfMemory) => Err(Abrupt::OutOfMemory),
        Err(Failure::Stopped) => Err(stopped.unwrap_or(Abrupt::OutOfMemory)),
    }
}

/// Der Wert eines Handles, wenn es ein BigInt hält.
pub(crate) fn value_of(agent: &Agent, value: Local<'_>) -> Option<BigInt> {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::BigInt(cell) => heap.bigint(cell),
        _ => None,
    }
}

/// Legt einen Wert im Heap an.
pub(crate) fn allocate<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: &BigInt,
) -> Completion<Local<'s>> {
    Ok(agent.heap_mut().alloc_bigint(scope, value)?)
}

/// BigInt::toString (Abschnitt 6.1.6.2.23) als Codeeinheiten.
pub(crate) fn to_units(agent: &mut Agent, value: &BigInt, radix: u32) -> Completion<Vec<u16>> {
    let text = compute(agent, |budget| value.to_radix_string(radix, budget))?;
    Ok(text.into_iter().map(u16::from).collect())
}

#[cfg(test)]
mod tests;
