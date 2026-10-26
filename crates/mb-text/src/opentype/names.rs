//! Die Tabelle `name`: Namen der Schrift in verschiedenen Sprachen.

use mb_encoding::{Decoder, Encoding};

use super::FontError;
use super::reader::{Reader, offset};

/// Ein Name aus der Tabelle `name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameRecord {
    /// Die Art des Namens (*name ID*), etwa 1 für die Familie, 2 für den
    /// Schnitt, 4 für den vollen Namen, 6 für den PostScript-Namen, 16 und
    /// 17 für die typografische Familie und den typografischen Schnitt.
    pub id: u16,
    pub platform: u16,
    /// Die Sprache, je nach Plattform kodiert.
    pub language: u16,
    pub value: String,
}

/// Die Namen, die gelesen werden; die übrigen (Urheberrecht, Lizenz,
/// Beschreibungen) braucht der Browser nicht.
const READ_IDS: [u16; 6] = [1, 2, 4, 6, 16, 17];

/// Liest die Namen in den Kodierungen, die sich lesen lassen: Unicode
/// (Plattform 0) und Microsoft (Plattform 3, Kodierungen 0, 1 und 10) als
/// UTF-16BE, Macintosh Roman (Plattform 1, Kodierung 0). Andere Einträge
/// fallen weg.
pub(crate) fn read(table: &[u8]) -> Result<Vec<NameRecord>, FontError> {
    let mut reader = Reader::at(table, 0)?;
    let _format = reader.u16()?;
    let count = reader.u16()?;
    let storage = usize::from(reader.u16()?);
    let mut names = Vec::new();
    for _ in 0..count {
        let platform = reader.u16()?;
        let encoding = reader.u16()?;
        let language = reader.u16()?;
        let id = reader.u16()?;
        let length = usize::from(reader.u16()?);
        let start = usize::from(reader.u16()?);
        if !READ_IDS.contains(&id) {
            continue;
        }
        let Ok(bytes) = Reader::at(table, offset(storage, start, 1)?)
            .and_then(|mut string| string.bytes(length))
        else {
            // Ein Name außerhalb der Tabelle fällt weg, die übrigen bleiben.
            continue;
        };
        let value = match (platform, encoding) {
            (0, _) | (3, 0 | 1 | 10) => utf16_be(bytes),
            (1, 0) => mac_roman(bytes),
            _ => continue,
        };
        names.push(NameRecord {
            id,
            platform,
            language,
            value,
        });
    }
    Ok(names)
}

fn utf16_be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_be_bytes(pair))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Macintosh Roman, wie die Kodierung `macintosh` des WHATWG Encoding
/// Standard.
fn mac_roman(bytes: &[u8]) -> String {
    let mut value = String::new();
    Decoder::new(Encoding::Macintosh).decode(bytes, true, &mut value);
    value
}
