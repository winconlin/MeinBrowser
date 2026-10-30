//! Schriftdateien im sfnt-Format: TrueType, OpenType und Sammlungen
//! (`ttcf`).
//!
//! Grundlage sind das TrueType Reference Manual von Apple (Kapitel 6, die
//! Tabellen) und für die Teile, die nur OpenType kennt (Sammlungen, die
//! Felder von `OS/2` ab Version 1, die Flags für den Versatz von
//! Komponenten), fontTools und FreeType als Referenzimplementierungen; die
//! OpenType-Spezifikation von Microsoft war aus der Arbeitsumgebung nicht
//! erreichbar. Prüfsummen werden nicht geprüft.

mod cmap;
mod glyf;
mod names;
mod reader;
#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::Arc;

pub use names::NameRecord;

use cmap::Subtable;
use glyf::Glyphs;
use reader::{Reader, offset, u16_at, u32_at};

use crate::outline::Outline;

/// Der Inhalt einer Schriftdatei, geteilt zwischen ihren Schnitten.
pub type FontData = Arc<[u8]>;

/// Die Kennung einer Tabelle, etwa `glyf`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(pub [u8; 4]);

impl fmt::Debug for Tag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "'{}'", String::from_utf8_lossy(&self.0))
    }
}

/// Warum sich eine Schrift oder eine Glyphe nicht lesen lässt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontError {
    /// Die Daten enden mitten in einer Struktur.
    Truncated,
    /// Weder sfnt noch eine Sammlung.
    UnknownFormat,
    /// Eine nötige Tabelle fehlt.
    MissingTable(Tag),
    /// Ein Wert ist ungültig.
    Malformed(&'static str),
    /// Gültig, aber noch nicht unterstützt.
    Unsupported(&'static str),
}

/// Die Nummer einer Glyphe in ihrer Schrift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphId(pub u16);

/// Maße der ganzen Schrift in Einheiten des Geviertquadrats; nach unten
/// negativ. Was aus `OS/2` kommt, fehlt, wenn die Tabelle fehlt oder zu
/// alt ist.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceMetrics {
    pub units_per_em: u16,
    /// Das Rechteck um alle Glyphen aus `head`: `xMin`, `yMin`, `xMax` und
    /// `yMax`.
    pub bounds: (i16, i16, i16, i16),
    /// Ober- und Unterlänge und Zeilenabstand aus `hhea`.
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    /// Dieselben Maße aus `OS/2` (*typographic*).
    pub typo_ascender: Option<i16>,
    pub typo_descender: Option<i16>,
    pub typo_line_gap: Option<i16>,
    /// Bit 7 von `fsSelection` (USE_TYPO_METRICS): Die Maße aus `OS/2`
    /// sollen gelten.
    pub use_typo_metrics: bool,
    /// Die Maße für Windows aus `OS/2`, beide nach unten positiv
    /// gespeichert; hier ist die Unterlänge negativ.
    pub win_ascent: Option<i32>,
    pub win_descent: Option<i32>,
    pub x_height: Option<i16>,
    pub cap_height: Option<i16>,
    pub strikeout_position: Option<i16>,
    pub strikeout_size: Option<i16>,
    /// Aus `post`.
    pub underline_position: Option<i16>,
    pub underline_thickness: Option<i16>,
    /// Neigung in Grad gegen den Uhrzeigersinn von der Senkrechten, für
    /// kursive Schriften negativ.
    pub italic_angle: f64,
    pub is_fixed_pitch: bool,
}

/// Die Einordnung eines Schnitts für die Schriftauswahl.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceStyle {
    /// `usWeightClass` aus `OS/2`, sonst 700 bei Bit 0 von `macStyle`
    /// (fett), sonst 400.
    pub weight: u16,
    /// `usWidthClass` aus `OS/2` (1 bis 9), sonst 5.
    pub width: u16,
    /// Bit 0 von `fsSelection`, ohne `OS/2` Bit 1 von `macStyle`.
    pub italic: bool,
    /// Bit 9 von `fsSelection` (OBLIQUE, ab Version 4).
    pub oblique: bool,
}

/// Ein Eintrag im Tabellenverzeichnis, in den Grenzen der Datei.
#[derive(Clone, Copy, Debug)]
struct TableRecord {
    tag: Tag,
    start: usize,
    end: usize,
}

/// Wie die Umrisse gespeichert sind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutlineFormat {
    TrueType { long_offsets: bool },
    Cff,
    None,
}

/// Ein Schnitt einer Schriftdatei.
#[derive(Clone, Debug)]
pub struct Face {
    data: FontData,
    tables: Vec<TableRecord>,
    glyph_count: u16,
    horizontal_metrics: u16,
    metrics: FaceMetrics,
    style: FaceStyle,
    names: Vec<NameRecord>,
    cmap: Option<Subtable>,
    outlines: OutlineFormat,
}

const TTCF: u32 = u32::from_be_bytes(*b"ttcf");
const TRUE: u32 = u32::from_be_bytes(*b"true");
const OTTO: u32 = u32::from_be_bytes(*b"OTTO");
const TYP1: u32 = u32::from_be_bytes(*b"typ1");
const WOFF: u32 = u32::from_be_bytes(*b"wOFF");
const WOFF2: u32 = u32::from_be_bytes(*b"wOF2");

/// Wie viele Schnitte eine Datei enthält: einen, oder bei einer Sammlung
/// die Zahl aus ihrem Kopf.
pub fn face_count(data: &[u8]) -> Result<u32, FontError> {
    let mut reader = Reader::at(data, 0)?;
    match reader.u32()? {
        TTCF => {
            reader.skip(4)?;
            reader.u32()
        }
        0x0001_0000 | TRUE | OTTO => Ok(1),
        version => Err(unknown_version(version)),
    }
}

fn unknown_version(version: u32) -> FontError {
    match version {
        TYP1 => FontError::Unsupported("Type 1 in sfnt"),
        WOFF | WOFF2 => FontError::Unsupported("WOFF"),
        _ => FontError::UnknownFormat,
    }
}

impl Face {
    /// Liest den Schnitt `index` einer Datei (0 außer bei Sammlungen).
    pub fn parse(data: FontData, index: u32) -> Result<Face, FontError> {
        let directory = directory_offset(&data, index)?;
        let mut reader = Reader::at(&data, directory)?;
        let version = reader.u32()?;
        if !matches!(version, 0x0001_0000 | TRUE | OTTO) {
            return Err(unknown_version(version));
        }
        let count = reader.u16()?;
        reader.skip(6)?;
        let mut tables: Vec<TableRecord> = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let tag = Tag(<[u8; 4]>::try_from(reader.bytes(4)?).map_err(|_| FontError::Truncated)?);
            let _checksum = reader.u32()?;
            let start = usize::try_from(reader.u32()?).map_err(|_| FontError::Truncated)?;
            let length = usize::try_from(reader.u32()?).map_err(|_| FontError::Truncated)?;
            let end = start.checked_add(length).ok_or(FontError::Truncated)?;
            if end > data.len() {
                return Err(FontError::Truncated);
            }
            // Doppelte Einträge: Der erste gilt.
            if !tables.iter().any(|table| table.tag == tag) {
                tables.push(TableRecord { tag, start, end });
            }
        }
        let mut face = Face {
            data,
            tables,
            glyph_count: 0,
            horizontal_metrics: 0,
            metrics: FaceMetrics {
                units_per_em: 0,
                bounds: (0, 0, 0, 0),
                ascender: 0,
                descender: 0,
                line_gap: 0,
                typo_ascender: None,
                typo_descender: None,
                typo_line_gap: None,
                use_typo_metrics: false,
                win_ascent: None,
                win_descent: None,
                x_height: None,
                cap_height: None,
                strikeout_position: None,
                strikeout_size: None,
                underline_position: None,
                underline_thickness: None,
                italic_angle: 0.0,
                is_fixed_pitch: false,
            },
            style: FaceStyle {
                weight: 400,
                width: 5,
                italic: false,
                oblique: false,
            },
            names: Vec::new(),
            cmap: None,
            outlines: OutlineFormat::None,
        };
        face.read_tables()?;
        Ok(face)
    }

    fn read_tables(&mut self) -> Result<(), FontError> {
        // head
        let head = self.required(b"head")?;
        let units_per_em = u16_at(head, 18)?;
        if !(16..=16384).contains(&units_per_em) {
            return Err(FontError::Malformed("head: unitsPerEm"));
        }
        let mut reader = Reader::at(head, 36)?;
        let bounds = (reader.i16()?, reader.i16()?, reader.i16()?, reader.i16()?);
        let mac_style = u16_at(head, 44)?;
        let long_offsets = match u16_at(head, 50)? {
            0 => false,
            1 => true,
            _ => return Err(FontError::Malformed("head: indexToLocFormat")),
        };
        // maxp
        let glyph_count = u16_at(self.required(b"maxp")?, 4)?;
        // hhea
        let hhea = self.required(b"hhea")?;
        let mut reader = Reader::at(hhea, 4)?;
        let ascender = reader.i16()?;
        let descender = reader.i16()?;
        let line_gap = reader.i16()?;
        let horizontal_metrics = u16_at(hhea, 34)?;
        if horizontal_metrics == 0 {
            return Err(FontError::Malformed("hhea: numberOfHMetrics"));
        }
        let hmtx = self.required(b"hmtx")?;
        if hmtx.len() < usize::from(horizontal_metrics).saturating_mul(4) {
            return Err(FontError::Truncated);
        }
        // cmap
        let cmap = cmap::choose(self.required(b"cmap")?)?;
        let names = match self.table(b"name") {
            Some(table) => names::read(table)?,
            None => Vec::new(),
        };
        let outlines = if self.table(b"glyf").is_some() {
            self.required(b"loca")?;
            OutlineFormat::TrueType { long_offsets }
        } else if self.table(b"CFF ").is_some() || self.table(b"CFF2").is_some() {
            OutlineFormat::Cff
        } else {
            OutlineFormat::None
        };

        let mut metrics = FaceMetrics {
            units_per_em,
            bounds,
            ascender,
            descender,
            line_gap,
            ..self.metrics
        };
        let mut style = FaceStyle {
            weight: if mac_style & 1 != 0 { 700 } else { 400 },
            width: 5,
            italic: mac_style & 2 != 0,
            oblique: false,
        };
        if let Some(os2) = self.table(b"OS/2") {
            read_os2(os2, &mut metrics, &mut style)?;
        }
        if let Some(post) = self.table(b"post") {
            let mut reader = Reader::at(post, 4)?;
            metrics.italic_angle = f64::from(reader.i32()?) / 65536.0;
            metrics.underline_position = Some(reader.i16()?);
            metrics.underline_thickness = Some(reader.i16()?);
            metrics.is_fixed_pitch = reader.u32()? != 0;
        }
        self.glyph_count = glyph_count;
        self.horizontal_metrics = horizontal_metrics;
        self.metrics = metrics;
        self.style = style;
        self.names = names;
        self.cmap = cmap;
        self.outlines = outlines;
        Ok(())
    }

    /// Die Daten einer Tabelle.
    fn table(&self, tag: &[u8; 4]) -> Option<&[u8]> {
        let record = self.tables.iter().find(|table| table.tag.0 == *tag)?;
        self.data.get(record.start..record.end)
    }

    fn required(&self, tag: &[u8; 4]) -> Result<&[u8], FontError> {
        self.table(tag).ok_or(FontError::MissingTable(Tag(*tag)))
    }

    /// Ob die Schrift eine Tabelle hat.
    pub fn has_table(&self, tag: Tag) -> bool {
        self.tables.iter().any(|table| table.tag == tag)
    }

    pub fn glyph_count(&self) -> u16 {
        self.glyph_count
    }

    pub fn metrics(&self) -> &FaceMetrics {
        &self.metrics
    }

    pub fn style(&self) -> FaceStyle {
        self.style
    }

    /// Die Namen aus der Tabelle `name`.
    pub fn names(&self) -> &[NameRecord] {
        &self.names
    }

    /// Die Glyphe zu einem Zeichen; `None`, wenn die Schrift keine hat.
    pub fn glyph_index(&self, character: char) -> Option<GlyphId> {
        let subtable = self.cmap?;
        let table = self.table(b"cmap")?;
        cmap::lookup(table, subtable, u32::from(character))
            .filter(|&glyph| glyph < self.glyph_count)
            .map(GlyphId)
    }

    /// Die Vorschubbreite einer Glyphe.
    pub fn advance_width(&self, glyph: GlyphId) -> Option<u16> {
        if glyph.0 >= self.glyph_count {
            return None;
        }
        // Glyphen hinter den langen Einträgen haben die letzte Breite.
        let index = glyph.0.min(self.horizontal_metrics.saturating_sub(1));
        let hmtx = self.table(b"hmtx")?;
        u16_at(hmtx, offset(0, usize::from(index), 4).ok()?).ok()
    }

    /// Der Abstand vom Ursprung zum linken Rand des Umrisses.
    pub fn left_side_bearing(&self, glyph: GlyphId) -> Option<i16> {
        if glyph.0 >= self.glyph_count {
            return None;
        }
        let hmtx = self.table(b"hmtx")?;
        let position = match glyph.0.checked_sub(self.horizontal_metrics) {
            None => offset(2, usize::from(glyph.0), 4),
            // Nach den langen Einträgen folgen nur noch Abstände.
            Some(rest) => offset(
                usize::from(self.horizontal_metrics).checked_mul(4)?,
                usize::from(rest),
                2,
            ),
        };
        Reader::at(hmtx, position.ok()?).ok()?.i16().ok()
    }

    /// Die Tabellen für TrueType-Umrisse.
    fn glyphs(&self) -> Result<Glyphs<'_>, FontError> {
        let OutlineFormat::TrueType { long_offsets } = self.outlines else {
            return Err(FontError::Unsupported("Schrift ohne glyf"));
        };
        Ok(Glyphs {
            glyf: self.required(b"glyf")?,
            loca: self.required(b"loca")?,
            long_offsets,
            glyph_count: self.glyph_count,
        })
    }

    /// Der Umriss einer Glyphe.
    pub fn outline(&self, glyph: GlyphId) -> Result<Outline, FontError> {
        match self.outlines {
            OutlineFormat::TrueType { .. } => self.glyphs()?.outline(glyph.0),
            OutlineFormat::Cff => Err(FontError::Unsupported("CFF-Umrisse")),
            OutlineFormat::None => Err(FontError::Unsupported("Schrift ohne Umrisse")),
        }
    }
}

/// Wo das Tabellenverzeichnis eines Schnitts beginnt.
fn directory_offset(data: &[u8], index: u32) -> Result<usize, FontError> {
    if u32_at(data, 0)? != TTCF {
        return if index == 0 {
            Ok(0)
        } else {
            Err(FontError::Malformed("kein Schnitt mit dieser Nummer"))
        };
    }
    // ttcf, Version, Zahl der Schnitte, dann ihre Versätze.
    let count = u32_at(data, 8)?;
    if index >= count {
        return Err(FontError::Malformed("kein Schnitt mit dieser Nummer"));
    }
    let index = usize::try_from(index).map_err(|_| FontError::Truncated)?;
    let directory = u32_at(data, offset(12, index, 4)?)?;
    usize::try_from(directory).map_err(|_| FontError::Truncated)
}

/// Die Felder aus `OS/2`, so weit die Version und die Länge der Tabelle
/// reichen. Version 0 hatte bei Apple ursprünglich nur 68 Bytes, ohne die
/// typografischen Maße.
fn read_os2(os2: &[u8], metrics: &mut FaceMetrics, style: &mut FaceStyle) -> Result<(), FontError> {
    let version = u16_at(os2, 0)?;
    let weight = u16_at(os2, 4)?;
    let width = u16_at(os2, 6)?;
    let selection = u16_at(os2, 62)?;
    // Werte außerhalb des Bereichs gelten als nicht angegeben.
    if (1..=1000).contains(&weight) {
        style.weight = weight;
    }
    if (1..=9).contains(&width) {
        style.width = width;
    }
    style.italic = selection & 0x0001 != 0;
    metrics.strikeout_size = Some(Reader::at(os2, 26)?.i16()?);
    metrics.strikeout_position = Some(Reader::at(os2, 28)?.i16()?);
    if os2.len() >= 78 {
        let mut reader = Reader::at(os2, 68)?;
        metrics.typo_ascender = Some(reader.i16()?);
        metrics.typo_descender = Some(reader.i16()?);
        metrics.typo_line_gap = Some(reader.i16()?);
        metrics.win_ascent = Some(i32::from(reader.u16()?));
        metrics.win_descent = Some(-i32::from(reader.u16()?));
    }
    if version >= 2 && os2.len() >= 90 {
        let mut reader = Reader::at(os2, 86)?;
        metrics.x_height = Some(reader.i16()?);
        metrics.cap_height = Some(reader.i16()?);
    }
    // Bit 7 (USE_TYPO_METRICS) und Bit 9 (OBLIQUE) sind nach fontTools erst
    // ab Version 4 definiert; die Bedeutung der Bits steht so in FreeType,
    // das Bit 7 allerdings auch bei älteren Versionen auswertet.
    if version >= 4 {
        metrics.use_typo_metrics = selection & 0x0080 != 0;
        style.oblique = selection & 0x0200 != 0;
    }
    Ok(())
}
