//! Die Tabelle `cmap`: von Unicode-Codepunkten zu Glyphen.
//!
//! Gelesen werden nur Untertabellen für Unicode, in den Formaten 0, 4, 6,
//! 10, 12 und 13. Die Formate 2 und 8 sind für gemischte Kodierungen mit
//! einem und zwei Bytes gedacht, die Untertabellen für Symbole (3, 0) und
//! die anderer Kodierungen des Microsoft-Systems zeigen keine
//! Unicode-Codepunkte; Format 14 (Variantenfolgen) fehlt noch.

use super::FontError;
use super::reader::{Reader, offset, u16_at, u32_at};

/// Eine gewählte Untertabelle: ihre Lage in der Tabelle `cmap` und ihr
/// Format. Gelesen wird bis zum Ende der Tabelle, nicht bis zum Ende, das
/// die Untertabelle angibt, denn das ist bei Format 4 oft zu klein (die
/// Länge hat nur 16 Bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Subtable {
    pub(crate) platform: u16,
    pub(crate) encoding: u16,
    pub(crate) offset: usize,
    pub(crate) format: u16,
}

const SUPPORTED_FORMATS: [u16; 6] = [0, 4, 6, 10, 12, 13];

/// Die Untertabellen für Unicode, die bevorzugte zuerst: erst die für alle
/// Ebenen, dann die nur für die BMP. Apple bevorzugt in seinem Handbuch
/// ebenso die Untertabellen, die über die BMP hinausgehen.
const PREFERENCE: [(u16, u16); 8] = [
    (3, 10),
    (0, 6),
    (0, 4),
    (3, 1),
    (0, 3),
    (0, 2),
    (0, 1),
    (0, 0),
];

/// Alle Untertabellen der Tabelle in der Reihenfolge der Einträge.
pub(crate) fn subtables(cmap: &[u8]) -> Result<Vec<Subtable>, FontError> {
    let mut reader = Reader::at(cmap, 0)?;
    let _version = reader.u16()?;
    let count = reader.u16()?;
    let mut subtables = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let platform = reader.u16()?;
        let encoding = reader.u16()?;
        let start = usize::try_from(reader.u32()?).map_err(|_| FontError::Truncated)?;
        let format = u16_at(cmap, start)?;
        subtables.push(Subtable {
            platform,
            encoding,
            offset: start,
            format,
        });
    }
    Ok(subtables)
}

/// Die Untertabelle, nach der Glyphen gesucht werden; `None`, wenn keine
/// passende da ist.
pub(crate) fn choose(cmap: &[u8]) -> Result<Option<Subtable>, FontError> {
    let subtables = subtables(cmap)?;
    Ok(PREFERENCE.iter().find_map(|&(platform, encoding)| {
        subtables.iter().copied().find(|subtable| {
            subtable.platform == platform
                && subtable.encoding == encoding
                && SUPPORTED_FORMATS.contains(&subtable.format)
        })
    }))
}

/// Die Glyphe zu einem Codepunkt; `None`, wenn die Untertabelle keine
/// nennt oder Glyphe 0 (die fehlende Glyphe).
pub(crate) fn lookup(cmap: &[u8], subtable: Subtable, code: u32) -> Option<u16> {
    let glyph = match subtable.format {
        0 => format_0(cmap, subtable.offset, code),
        4 => format_4(cmap, subtable.offset, code),
        6 => format_6(cmap, subtable.offset, code),
        10 => format_10(cmap, subtable.offset, code),
        12 => format_12(cmap, subtable.offset, code, false),
        13 => format_12(cmap, subtable.offset, code, true),
        _ => Ok(None),
    };
    glyph.ok().flatten().filter(|&glyph| glyph != 0)
}

/// Format 0: 256 Glyphen mit je einem Byte.
fn format_0(cmap: &[u8], start: usize, code: u32) -> Result<Option<u16>, FontError> {
    let Ok(code) = u8::try_from(code) else {
        return Ok(None);
    };
    let position = offset(start, 6, 1)?.checked_add(usize::from(code));
    let position = position.ok_or(FontError::Truncated)?;
    Ok(Some(u16::from(Reader::at(cmap, position)?.u8()?)))
}

/// Format 4: Abschnitte mit 16-Bit-Codepunkten.
fn format_4(cmap: &[u8], start: usize, code: u32) -> Result<Option<u16>, FontError> {
    let Ok(code) = u16::try_from(code) else {
        return Ok(None);
    };
    let segments_x2 = usize::from(u16_at(cmap, offset(start, 6, 1)?)?);
    let segments = segments_x2 / 2;
    if segments == 0 || segments_x2 % 2 != 0 {
        return Err(FontError::Malformed("cmap 4: Zahl der Abschnitte"));
    }
    let ends = offset(start, 14, 1)?;
    // Hinter den Enden steht ein reserviertes Feld.
    let starts = offset(
        ends,
        1,
        segments_x2.checked_add(2).ok_or(FontError::Truncated)?,
    )?;
    let deltas = offset(starts, 1, segments_x2)?;
    let range_offsets = offset(deltas, 1, segments_x2)?;
    // Der erste Abschnitt, dessen Ende nicht kleiner ist als der Codepunkt.
    let (mut low, mut high) = (0, segments);
    while low < high {
        let middle = low + (high - low) / 2;
        if u16_at(cmap, offset(ends, middle, 2)?)? < code {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low == segments {
        return Ok(None);
    }
    let segment_start = u16_at(cmap, offset(starts, low, 2)?)?;
    if segment_start > code {
        return Ok(None);
    }
    let delta = u16_at(cmap, offset(deltas, low, 2)?)?;
    let range_offset_position = offset(range_offsets, low, 2)?;
    let range_offset = u16_at(cmap, range_offset_position)?;
    if range_offset == 0 {
        return Ok(Some(code.wrapping_add(delta)));
    }
    // Die Stelle ist relativ zum Feld `idRangeOffset` selbst.
    let position = range_offset_position
        .checked_add(usize::from(range_offset))
        .and_then(|position| {
            position.checked_add(usize::from(code - segment_start).checked_mul(2)?)
        })
        .ok_or(FontError::Truncated)?;
    let glyph = u16_at(cmap, position)?;
    Ok(Some(if glyph == 0 {
        0
    } else {
        glyph.wrapping_add(delta)
    }))
}

/// Format 6: ein zusammenhängender Bereich mit 16-Bit-Codepunkten.
fn format_6(cmap: &[u8], start: usize, code: u32) -> Result<Option<u16>, FontError> {
    let first = u32::from(u16_at(cmap, offset(start, 6, 1)?)?);
    let count = u32::from(u16_at(cmap, offset(start, 8, 1)?)?);
    trimmed_array(cmap, offset(start, 10, 1)?, first, count, code)
}

/// Format 10: ein zusammenhängender Bereich mit 32-Bit-Codepunkten.
fn format_10(cmap: &[u8], start: usize, code: u32) -> Result<Option<u16>, FontError> {
    let first = u32_at(cmap, offset(start, 12, 1)?)?;
    let count = u32_at(cmap, offset(start, 16, 1)?)?;
    trimmed_array(cmap, offset(start, 20, 1)?, first, count, code)
}

/// Die Glyphe aus einer Liste für `count` Codepunkte ab `first`.
fn trimmed_array(
    cmap: &[u8],
    glyphs: usize,
    first: u32,
    count: u32,
    code: u32,
) -> Result<Option<u16>, FontError> {
    let Some(index) = code.checked_sub(first).filter(|&index| index < count) else {
        return Ok(None);
    };
    let index = usize::try_from(index).map_err(|_| FontError::Truncated)?;
    u16_at(cmap, offset(glyphs, index, 2)?).map(Some)
}

/// Format 12 (Glyphen fortlaufend) und 13 (eine Glyphe je Gruppe):
/// Gruppen mit 32-Bit-Codepunkten.
fn format_12(
    cmap: &[u8],
    start: usize,
    code: u32,
    many_to_one: bool,
) -> Result<Option<u16>, FontError> {
    let count = u32_at(cmap, offset(start, 12, 1)?)?;
    let count = usize::try_from(count).map_err(|_| FontError::Truncated)?;
    let groups = offset(start, 16, 1)?;
    // Die erste Gruppe, deren Ende nicht kleiner ist als der Codepunkt.
    let (mut low, mut high) = (0, count);
    while low < high {
        let middle = low + (high - low) / 2;
        let end = u32_at(cmap, offset(groups, middle, 12)?.saturating_add(4))?;
        if end < code {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low == count {
        return Ok(None);
    }
    let group = offset(groups, low, 12)?;
    let mut reader = Reader::at(cmap, group)?;
    let first = reader.u32()?;
    let _end = reader.u32()?;
    let glyph = reader.u32()?;
    let Some(index) = code.checked_sub(first) else {
        return Ok(None);
    };
    let glyph = if many_to_one {
        Some(glyph)
    } else {
        glyph.checked_add(index)
    };
    Ok(glyph.and_then(|glyph| u16::try_from(glyph).ok()))
}
