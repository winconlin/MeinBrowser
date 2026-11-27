//! SHA-256 nach FIPS 180-4 (August 2015): die Funktionen aus 4.1.2, die
//! Konstanten aus 4.2.2, das Auffüllen aus 5.1.1, der Anfangswert aus
//! 5.3.3 und die Berechnung aus 6.2.
//!
//! Die Nachricht besteht aus ganzen Bytes; Nachrichten mit einer Länge in
//! Bit, die kein Vielfaches von acht ist, kommen im Web nicht vor.

use crate::{Digest, HashAlgorithm, HashContext};

#[cfg(test)]
mod tests;

/// Die Länge eines Blocks in Bytes (512 Bit).
const BLOCK: usize = 64;

/// Die längste erlaubte Nachricht in Bytes: Ihre Länge in Bit muss in
/// 64 Bit passen (5.1.1).
const MAX_BYTES: u64 = u64::MAX >> 3;

/// Die Konstanten K aus 4.2.2: die ersten 32 Bit der Nachkommastellen der
/// Kubikwurzeln der ersten 64 Primzahlen.
#[rustfmt::skip]
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Der Anfangswert H(0) aus 5.3.3: die ersten 32 Bit der Nachkommastellen
/// der Quadratwurzeln der ersten acht Primzahlen.
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// Ein laufender SHA-256-Hash.
#[derive(Clone)]
pub(crate) struct Sha256 {
    /// Der Zwischenwert H(i).
    state: [u32; 8],
    /// Der angefangene Block.
    buffer: [u8; BLOCK],
    /// Wie viele Bytes von `buffer` belegt sind, immer weniger als ein
    /// Block.
    filled: usize,
    /// Die Länge der Nachricht bisher in Bytes, höchstens [`MAX_BYTES`].
    length: u64,
}

impl Sha256 {
    pub(crate) fn new() -> Sha256 {
        Sha256 {
            state: H0,
            buffer: [0; BLOCK],
            filled: 0,
            length: 0,
        }
    }

    /// Hängt `data` an die Nachricht an.
    ///
    /// # Panics
    ///
    /// Wenn die Nachricht damit länger als 2⁶¹ − 1 Bytes würde, denn ihre
    /// Länge in Bit muss in 64 Bit passen (5.1.1). Das sind zwei Exbibyte;
    /// so viel Daten fallen nicht an.
    pub(crate) fn update(&mut self, data: &[u8]) {
        let length = u64::try_from(data.len())
            .ok()
            .and_then(|added| self.length.checked_add(added))
            .filter(|&length| length <= MAX_BYTES);
        match length {
            Some(length) => self.length = length,
            None => {
                panic!("SHA-256: Die Nachricht ist länger als 2^64 - 1 Bit (FIPS 180-4, 5.1.1).")
            }
        }
        let mut data = data;
        if self.filled > 0 {
            let take = BLOCK.saturating_sub(self.filled).min(data.len());
            let (head, rest) = data.split_at_checked(take).unwrap_or((data, &[]));
            let end = self.filled.saturating_add(take);
            if let Some(slot) = self.buffer.get_mut(self.filled..end) {
                slot.copy_from_slice(head);
            }
            self.filled = end;
            data = rest;
            if self.filled < BLOCK {
                return;
            }
            compress(&mut self.state, &self.buffer);
            self.filled = 0;
        }
        let (blocks, rest) = data.as_chunks::<BLOCK>();
        for block in blocks {
            compress(&mut self.state, block);
        }
        if let Some(slot) = self.buffer.get_mut(..rest.len()) {
            slot.copy_from_slice(rest);
        }
        self.filled = rest.len();
    }

    /// Füllt die Nachricht nach 5.1.1 auf und liefert H(N) als Bytes mit
    /// dem höchsten zuerst (6.2.2, Schritt 4 und 3.1).
    pub(crate) fn finalize(mut self) -> Digest {
        // Eine 1, Nullen bis 448 modulo 512 Bit, dann die Länge in Bit als
        // 64-Bit-Zahl. Das passt in den angefangenen Block, wenn er höchstens
        // 55 Bytes hat, sonst braucht es einen zweiten. Die Länge ist
        // höchstens `MAX_BYTES`, in Bit passt sie also in 64 Bit.
        let bits = self.length << 3;
        let mut tail = [0_u8; 2 * BLOCK];
        if let Some(slot) = tail.get_mut(..self.filled) {
            slot.copy_from_slice(self.buffer.get(..self.filled).unwrap_or(&[]));
        }
        if let Some(byte) = tail.get_mut(self.filled) {
            *byte = 0x80;
        }
        let end = if self.filled < BLOCK - 8 {
            BLOCK
        } else {
            2 * BLOCK
        };
        if let Some(slot) = tail.get_mut(end.saturating_sub(8)..end) {
            slot.copy_from_slice(&bits.to_be_bytes());
        }
        let (blocks, _) = tail.get(..end).unwrap_or(&[]).as_chunks::<BLOCK>();
        for block in blocks {
            compress(&mut self.state, block);
        }
        let mut bytes = [0_u8; Digest::MAX_LEN];
        let (words, _) = bytes.as_chunks_mut::<4>();
        for (slot, word) in words.iter_mut().zip(self.state) {
            *slot = word.to_be_bytes();
        }
        Digest::new(HashAlgorithm::Sha256, bytes)
    }
}

impl HashContext for Sha256 {
    fn update(&mut self, data: &[u8]) {
        Sha256::update(self, data);
    }

    fn finish(self: Box<Self>) -> Digest {
        Sha256::finalize(*self)
    }
}

/// Verarbeitet einen Block nach 6.2.2.
fn compress(state: &mut [u32; 8], block: &[u8; BLOCK]) {
    // Schritt 1: der Nachrichtenplan W.
    let mut w = [0_u32; 64];
    let (words, _) = block.as_chunks::<4>();
    for (slot, word) in w.iter_mut().zip(words) {
        *slot = u32::from_be_bytes(*word);
    }
    for t in 16..64 {
        let value = match (w.get(t - 2), w.get(t - 7), w.get(t - 15), w.get(t - 16)) {
            (Some(&w2), Some(&w7), Some(&w15), Some(&w16)) => small_sigma1(w2)
                .wrapping_add(w7)
                .wrapping_add(small_sigma0(w15))
                .wrapping_add(w16),
            _ => 0,
        };
        if let Some(slot) = w.get_mut(t) {
            *slot = value;
        }
    }
    // Schritte 2 und 3: die Arbeitsvariablen und die 64 Runden.
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (&k, &wt) in K.iter().zip(w.iter()) {
        let t1 = h
            .wrapping_add(big_sigma1(e))
            .wrapping_add(ch(e, f, g))
            .wrapping_add(k)
            .wrapping_add(wt);
        let t2 = big_sigma0(a).wrapping_add(maj(a, b, c));
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    // Schritt 4: der nächste Zwischenwert.
    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}

// Die Funktionen aus 4.1.2.

fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

fn big_sigma0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}

fn big_sigma1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}

fn small_sigma0(x: u32) -> u32 {
    x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
}

fn small_sigma1(x: u32) -> u32 {
    x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
}
