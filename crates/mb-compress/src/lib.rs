//! Dekoder für Deflate, Brotli und zstd sowie ZIP.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.11. Bisher gibt es Deflate (RFC
//! 1951) und das zlib-Format (RFC 1950), wie PNG sie braucht, dazu CRC-32
//! und Adler-32. Dekodiert wird in einem Zug aus dem ganzen Eingabepuffer.

#![forbid(unsafe_code)]

mod checksum;
mod inflate;

#[cfg(test)]
mod tests;

pub use checksum::{Crc32, adler32, crc32};
pub use inflate::{Inflated, inflate};

/// Fehler beim Dekodieren. Die Fälle entsprechen den Meldungen von zlib.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Die Eingabe endet mitten in den Daten.
    UnexpectedEnd,
    /// Die Ausgabe wäre größer als die Grenze des Aufrufers.
    TooLarge,
    /// „invalid block type“
    BlockType,
    /// „invalid stored block lengths“
    StoredLength,
    /// „too many length or distance symbols“
    TooManySymbols,
    /// „invalid code lengths set“
    CodeLengthCode,
    /// „invalid bit length repeat“
    Repeat,
    /// „invalid code -- missing end-of-block“
    MissingEndOfBlock,
    /// „invalid literal/lengths set“
    LiteralLengthCode,
    /// „invalid distances set“
    DistanceCode,
    /// „invalid literal/length code“
    InvalidLiteralLength,
    /// „invalid distance code“
    InvalidDistance,
    /// „invalid distance too far back“
    TooFarBack,
    /// Methode, Fenstergröße oder Prüfbits im zlib-Kopf stimmen nicht.
    ZlibHeader,
    /// Der zlib-Strom braucht ein voreingestelltes Wörterbuch.
    Dictionary,
    /// Die Prüfsumme am Ende passt nicht zu den Daten.
    Checksum,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::UnexpectedEnd => "Daten enden zu früh",
            Error::TooLarge => "Ausgabe zu groß",
            Error::BlockType => "ungültiger Blocktyp",
            Error::StoredLength => "Länge eines unkomprimierten Blocks ungültig",
            Error::TooManySymbols => "zu viele Symbole für Längen oder Abstände",
            Error::CodeLengthCode => "ungültiger Code für die Codelängen",
            Error::Repeat => "ungültige Wiederholung einer Codelänge",
            Error::MissingEndOfBlock => "kein Code für das Blockende",
            Error::LiteralLengthCode => "ungültiger Code für Literale und Längen",
            Error::DistanceCode => "ungültiger Code für Abstände",
            Error::InvalidLiteralLength => "ungültiges Literal oder ungültige Länge",
            Error::InvalidDistance => "ungültiger Abstand",
            Error::TooFarBack => "Abstand reicht vor den Anfang der Daten",
            Error::ZlibHeader => "ungültiger zlib-Kopf",
            Error::Dictionary => "zlib-Strom mit Wörterbuch",
            Error::Checksum => "Prüfsumme passt nicht",
        })
    }
}

impl std::error::Error for Error {}

/// Dekodiert einen zlib-Strom (RFC 1950) mit höchstens `limit` Bytes
/// Ausgabe. Was nach der Prüfsumme kommt, zählt nicht.
pub fn zlib_decompress(input: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let Some(&[method, flags]) = input.get(..2) else {
        return Err(Error::UnexpectedEnd);
    };
    // Methode 8 (Deflate) mit einem Fenster bis 32 KiB, und die beiden
    // Bytes als Zahl durch 31 teilbar.
    let check = (u16::from(method) << 8 | u16::from(flags)) % 31;
    if method & 0x0F != 8 || method >> 4 > 7 || check != 0 {
        return Err(Error::ZlibHeader);
    }
    if flags & 0x20 != 0 {
        return Err(Error::Dictionary);
    }
    let body = input.get(2..).unwrap_or_default();
    let inflated = inflate(body, limit)?;
    if !inflated.finished {
        return Err(Error::UnexpectedEnd);
    }
    let end = inflated.consumed.saturating_add(4);
    let Some(&[a, b, c, d]) = body.get(inflated.consumed..end) else {
        return Err(Error::UnexpectedEnd);
    };
    if adler32(&inflated.data) != u32::from_be_bytes([a, b, c, d]) {
        return Err(Error::Checksum);
    }
    Ok(inflated.data)
}
