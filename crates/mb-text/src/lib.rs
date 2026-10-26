//! Schriften, Textformung und Zeilenumbruch.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.3. Bisher gibt es den Leser für
//! Schriftdateien im sfnt-Format (TrueType und OpenType, auch Sammlungen):
//! Kopfdaten, Metriken, Namen, die Zuordnung von Zeichen zu Glyphen und die
//! Umrisse aus `glyf`. Umrisse aus `CFF ` und `CFF2`, variable Schriften,
//! Farbschriften und WOFF fehlen noch.
//!
//! Dazu kommen die Bibliothek der installierten Schriften mit der
//! Schriftauswahl nach CSS Fonts 4 ([`FontLibrary`]; welche Dateien
//! installiert sind, weiß die Plattformschicht) und eine einfache
//! Textformung ohne GSUB und GPOS ([`shape`]).

#![forbid(unsafe_code)]

mod library;
mod matching;
mod opentype;
mod outline;
mod shaping;

pub use library::{FaceId, Family, FontLibrary, FontQuery, GenericFamily, Loader};
pub use matching::Slope;
pub use opentype::{
    Face, FaceMetrics, FaceStyle, FontData, FontError, GlyphId, NameRecord, Tag, face_count,
};
pub use outline::{Outline, PathCommand, Point};
pub use shaping::{Glyph, shape};
