//! Schriften, Textformung und Zeilenumbruch.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.3. Bisher gibt es den Leser für
//! Schriftdateien im sfnt-Format (TrueType und OpenType, auch Sammlungen):
//! Kopfdaten, Metriken, Namen, die Zuordnung von Zeichen zu Glyphen und die
//! Umrisse aus `glyf`. Umrisse aus `CFF ` und `CFF2`, variable Schriften,
//! Farbschriften und WOFF fehlen noch.

#![forbid(unsafe_code)]

mod opentype;
mod outline;

pub use opentype::{
    Face, FaceMetrics, FaceStyle, FontData, FontError, GlyphId, NameRecord, Tag, face_count,
};
pub use outline::{Outline, PathCommand, Point};
