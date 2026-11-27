//! Kryptografie hinter einer schmalen Anbieter-Schnittstelle.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.11. Wer Kryptografie braucht,
//! spricht nur mit einem [`Provider`]. Der eigene Anbieter ist [`Builtin`];
//! ein anderer lässt sich einsetzen, ohne die Aufrufer zu ändern.
//!
//! Bisher gibt es ein Verfahren, SHA-256 nach FIPS 180-4.
//!
//! # Konstante Laufzeit
//!
//! SHA-256 rechnet nur mit Addition modulo 2³², Rotation, Verschiebung und
//! bitweisen Operatoren auf 32-Bit-Wörtern. Es verzweigt und liest Tabellen
//! nur nach öffentlichen Größen, dem Rundenzähler und der Länge der Daten,
//! nie nach ihrem Inhalt. Die Laufzeit hängt darum nur von der Länge ab.
//! Ein Vergleich von [`Digest`] mit `==` hat dagegen keine konstante
//! Laufzeit; für geheime Werte wie MACs ist er nicht gedacht.

#![forbid(unsafe_code)]

mod sha256;

#[cfg(test)]
mod tests;

use std::fmt;

/// Ein Hashverfahren.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HashAlgorithm {
    /// SHA-256 nach FIPS 180-4, Abschnitt 6.2.
    Sha256,
}

impl HashAlgorithm {
    /// Die Länge des Hashwerts in Bytes.
    pub const fn output_len(self) -> usize {
        match self {
            HashAlgorithm::Sha256 => 32,
        }
    }
}

/// Ein Hashwert mit seinem Verfahren.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest {
    algorithm: HashAlgorithm,
    /// Die Bytes des Hashwerts, dahinter Nullen.
    bytes: [u8; Digest::MAX_LEN],
}

impl Digest {
    /// Die größte Länge eines Hashwerts in Bytes.
    pub const MAX_LEN: usize = 32;

    fn new(algorithm: HashAlgorithm, bytes: [u8; Digest::MAX_LEN]) -> Digest {
        Digest { algorithm, bytes }
    }

    /// Ein Hashwert aus seinen Bytes, etwa für einen anderen Anbieter, der
    /// selbst rechnet. `None`, wenn die Länge nicht zum Verfahren passt.
    pub fn from_bytes(algorithm: HashAlgorithm, bytes: &[u8]) -> Option<Digest> {
        if bytes.len() != algorithm.output_len() {
            return None;
        }
        let mut buffer = [0; Digest::MAX_LEN];
        buffer.get_mut(..bytes.len())?.copy_from_slice(bytes);
        Some(Digest::new(algorithm, buffer))
    }

    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    /// Die Bytes des Hashwerts, so viele, wie das Verfahren liefert.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes
            .get(..self.algorithm.output_len())
            .unwrap_or(&self.bytes)
    }

    /// Der Hashwert als Hexziffern in Kleinbuchstaben.
    pub fn to_hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut text = String::with_capacity(self.as_bytes().len() * 2);
        for &byte in self.as_bytes() {
            for nibble in [byte >> 4, byte & 0x0F] {
                let digit = DIGITS.get(usize::from(nibble)).copied().unwrap_or(b'0');
                text.push(char::from(digit));
            }
        }
        text
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Digest({:?}, {})", self.algorithm, self.to_hex())
    }
}

/// Ein laufender Hash: Daten in beliebigen Stücken hineingeben, dann
/// abschließen. Das Ergebnis hängt nicht davon ab, wie die Daten geteilt
/// sind.
pub trait HashContext: Send {
    fn update(&mut self, data: &[u8]);

    fn finish(self: Box<Self>) -> Digest;
}

/// Die Schnittstelle zu einem Anbieter der Kryptografie.
pub trait Provider: Send + Sync {
    /// Beginnt einen Hash mit dem Verfahren `algorithm`.
    fn hash(&self, algorithm: HashAlgorithm) -> Box<dyn HashContext>;

    /// Der Hash eines ganzen Stücks.
    fn digest(&self, algorithm: HashAlgorithm, data: &[u8]) -> Digest {
        let mut context = self.hash(algorithm);
        context.update(data);
        context.finish()
    }
}

/// Der eigene Anbieter mit den Verfahren dieses Crates.
#[derive(Clone, Copy, Debug, Default)]
pub struct Builtin;

impl Provider for Builtin {
    fn hash(&self, algorithm: HashAlgorithm) -> Box<dyn HashContext> {
        match algorithm {
            HashAlgorithm::Sha256 => Box::new(sha256::Sha256::new()),
        }
    }

    fn digest(&self, algorithm: HashAlgorithm, data: &[u8]) -> Digest {
        match algorithm {
            HashAlgorithm::Sha256 => {
                let mut context = sha256::Sha256::new();
                context.update(data);
                context.finalize()
            }
        }
    }
}
