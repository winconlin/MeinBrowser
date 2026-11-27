//! SHA-256 aus `mb-crypto` gegen ein unabhängiges Modell.
//!
//! Das erste Byte der Eingabe wählt, wie die Nachricht in Stücke geteilt an
//! `update` geht; der Rest ist die Nachricht. Bit 0 und 1 wählen die Art:
//! alles in einem Aufruf, Stücke fester Länge (Bit 2 bis 7 plus eins, also
//! 1 bis 64 Bytes), Stücke zufälliger Länge von 0 bis 130 Bytes aus einem
//! xorshift-Generator mit dem ersten Byte als Startwert, oder einzelne
//! Bytes. Geprüft wird:
//!
//! - `Provider::digest` und der Hash in Stücken über `Provider::hash`
//!   ergeben dasselbe wie das Modell.
//! - Der Hash jedes Anfangsstücks, an dem die Teilung eine Grenze setzt,
//!   ergibt dasselbe wie das Modell für dieses Anfangsstück, solange die
//!   Nachricht höchstens [`MAX_PREFIXES`] Grenzen hat.
//!
//! Das Modell folgt FIPS 180-4 auf einem anderen Weg als `mb-crypto`: Es
//! füllt die ganze Nachricht vorab auf, rechnet die Konstanten aus den
//! Primzahlen aus, statt sie abzuschreiben, schreibt Rotationen als
//! Verschiebungen und Ch und Maj in einer gleichwertigen anderen Form.

use std::sync::OnceLock;

use mb_crypto::{Builtin, HashAlgorithm, Provider};

/// Höchstens so viele Anfangsstücke prüft eine Eingabe einzeln.
const MAX_PREFIXES: usize = 64;

pub fn sha256(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let Some((&control, message)) = data.split_first() else {
        return;
    };
    let expected = model(message);
    assert_eq!(
        Builtin.digest(HashAlgorithm::Sha256, message).as_bytes(),
        expected
    );
    let pieces = pieces(control, message.len());
    let mut context = Builtin.hash(HashAlgorithm::Sha256);
    let mut start = 0;
    for &length in &pieces {
        context.update(&message[start..start + length]);
        start += length;
    }
    assert_eq!(start, message.len());
    assert_eq!(context.finish().as_bytes(), expected);
    if pieces.len() <= MAX_PREFIXES {
        let mut end = 0;
        for &length in &pieces {
            end += length;
            let prefix = &message[..end];
            let mut context = Builtin.hash(HashAlgorithm::Sha256);
            context.update(prefix);
            assert_eq!(context.finish().as_bytes(), model(prefix));
        }
    }
}

/// Die Längen der Stücke, in denen `length` Bytes an `update` gehen.
fn pieces(control: u8, length: usize) -> Vec<usize> {
    let mut pieces = Vec::new();
    let mut rest = length;
    let mut state = u64::from(control) | 0x100;
    while rest > 0 {
        let piece = match control & 3 {
            0 => rest,
            1 => usize::from(control >> 2) + 1,
            2 => {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % 131) as usize
            }
            _ => 1,
        }
        .min(rest);
        pieces.push(piece);
        rest -= piece;
    }
    pieces
}

/// Die Konstanten K und H(0) aus den Primzahlen (FIPS 180-4, 4.2.2 und
/// 5.3.3): die ersten 32 Bit der Nachkommastellen ihrer Kubik- und
/// Quadratwurzeln.
fn constants() -> &'static ([u32; 64], [u32; 8]) {
    static CONSTANTS: OnceLock<([u32; 64], [u32; 8])> = OnceLock::new();
    CONSTANTS.get_or_init(|| {
        let mut primes = Vec::new();
        let mut candidate = 2_u128;
        while primes.len() < 64 {
            if primes.iter().all(|&prime| !candidate.is_multiple_of(prime)) {
                primes.push(candidate);
            }
            candidate += 1;
        }
        let mut k = [0; 64];
        for (slot, &prime) in k.iter_mut().zip(&primes) {
            *slot = root(prime << 96, 3) as u32;
        }
        let mut h = [0; 8];
        for (slot, &prime) in h.iter_mut().zip(&primes) {
            *slot = root(prime << 64, 2) as u32;
        }
        (k, h)
    })
}

/// Die größte ganze Zahl r mit rᵖ ≤ n, Bit für Bit von oben bestimmt.
fn root(n: u128, power: u32) -> u128 {
    let mut result = 0_u128;
    for bit in (0..64).rev() {
        let candidate = result | 1 << bit;
        if candidate.checked_pow(power).is_some_and(|value| value <= n) {
            result = candidate;
        }
    }
    result
}

/// ROTR nach FIPS 180-4, 3.2, bewusst als Verschiebungen geschrieben und
/// nicht mit `rotate_right` wie in `mb-crypto`.
#[allow(clippy::manual_rotate)]
fn rotate(x: u32, n: u32) -> u32 {
    (x >> n) | (x << (32 - n))
}

/// SHA-256 der ganzen Nachricht.
fn model(message: &[u8]) -> Vec<u8> {
    let (k, h0) = constants();
    let mut padded = message.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&(message.len() as u64 * 8).to_be_bytes());
    let mut h = *h0;
    for block in padded.chunks(64) {
        let mut w = Vec::with_capacity(64);
        for t in 0..64 {
            let word = if t < 16 {
                u32::from_be_bytes([
                    block[4 * t],
                    block[4 * t + 1],
                    block[4 * t + 2],
                    block[4 * t + 3],
                ])
            } else {
                let x: u32 = w[t - 2];
                let y: u32 = w[t - 15];
                let sigma1 = rotate(x, 17) ^ rotate(x, 19) ^ (x >> 10);
                let sigma0 = rotate(y, 7) ^ rotate(y, 18) ^ (y >> 3);
                sigma1
                    .wrapping_add(w[t - 7])
                    .wrapping_add(sigma0)
                    .wrapping_add(w[t - 16])
            };
            w.push(word);
        }
        let mut v = h;
        for t in 0..64 {
            let [a, b, c, d, e, f, g, hh] = v;
            let choose = (e & f) | (!e & g);
            let majority = (a & b) | (c & (a | b));
            let t1 = hh
                .wrapping_add(rotate(e, 6) ^ rotate(e, 11) ^ rotate(e, 25))
                .wrapping_add(choose)
                .wrapping_add(k[t])
                .wrapping_add(w[t]);
            let t2 = (rotate(a, 2) ^ rotate(a, 13) ^ rotate(a, 22)).wrapping_add(majority);
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (word, value) in h.iter_mut().zip(v) {
            *word = word.wrapping_add(value);
        }
    }
    h.iter().flat_map(|word| word.to_be_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_matches_the_standard() {
        // Die Beispiele des NIST zu FIPS 180-4.
        let hex = |bytes: Vec<u8>| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        assert_eq!(
            hex(model(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(model(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(model(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn pieces_cover_the_message() {
        for control in 0..=255 {
            for length in [0, 1, 55, 56, 63, 64, 65, 1000, crate::MAX_INPUT - 1] {
                let pieces = pieces(control, length);
                assert_eq!(pieces.iter().sum::<usize>(), length);
                match control & 3 {
                    0 => assert!(pieces.len() <= 1),
                    1 => assert!(
                        pieces
                            .iter()
                            .all(|&piece| piece <= usize::from(control >> 2) + 1)
                    ),
                    2 => assert!(pieces.iter().all(|&piece| piece <= 130)),
                    _ => assert_eq!(pieces.len(), length),
                }
            }
        }
        // Die zufällige Teilung bringt auch leere Stücke.
        assert!(
            (0..=255)
                .filter(|control| control & 3 == 2)
                .any(|control| pieces(control, 4000).contains(&0))
        );
    }

    #[test]
    fn inputs_of_every_kind_pass() {
        let message: Vec<u8> = (0..300_u32).map(|index| (index * 7) as u8).collect();
        for control in [0, 1, 1 | 63 << 2, 2, 2 | 0xFC, 3] {
            let mut data = vec![control];
            data.extend_from_slice(&message);
            sha256(&data);
        }
        // Jede Länge bis über zwei Blöcke, also jeder Fall beim Auffüllen.
        for length in 0..=130 {
            let mut data = vec![0];
            data.extend_from_slice(&message[..length]);
            sha256(&data);
        }
        sha256(&[]);
    }
}
