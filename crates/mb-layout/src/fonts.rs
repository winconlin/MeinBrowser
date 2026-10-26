//! Die Schriften für Stil und Layout: die Anfrage an die Bibliothek aus
//! einer berechneten Schrift, die Maße der ersten verfügbaren Schrift und
//! das Formen von Text.
//!
//! Aufstieg und Abstieg kommen aus `sTypoAscender` und `sTypoDescender` der
//! Tabelle OS/2, ohne sie aus `hhea`, wie CSS Inline 3 es empfiehlt; der
//! Zeilenabstand der Schrift (*line gap metric*) aus derselben Quelle, nie
//! unter null.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use mb_style::properties::{FontFamily, FontStyle, GenericFamily, ScriptGenericFamily};
use mb_style::{Advance, Font, FontMetrics, FontMetricsSource};
use mb_text::{Face, Family, FontLibrary, FontQuery, Glyph, Slope};

/// Die Maße der ersten verfügbaren Schrift in `px`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Metrics {
    pub(crate) ascent: f64,
    pub(crate) descent: f64,
    pub(crate) line_gap: f64,
    pub(crate) x_height: Option<f64>,
    pub(crate) cap_height: Option<f64>,
    pub(crate) zero: Option<f64>,
    pub(crate) water: Option<f64>,
}

impl Metrics {
    /// Ohne jede Schrift: Aufstieg 0,8 em und Abstieg 0,2 em. Das ist eine
    /// Festlegung dieses Browsers; keine Spezifikation nennt Werte für den
    /// Fall, dass es gar keine Schrift gibt.
    fn fallback(size: f64) -> Metrics {
        Metrics {
            ascent: size * 0.8,
            descent: size * 0.2,
            line_gap: 0.0,
            x_height: None,
            cap_height: None,
            zero: None,
            water: None,
        }
    }

    /// Die Zeilenhöhe bei `line-height: normal`.
    pub(crate) fn normal_line_height(&self) -> f64 {
        self.ascent + self.descent + self.line_gap
    }
}

/// Der Schlüssel einer Anfrage im Zwischenspeicher.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct QueryKey {
    families: Vec<FamilyKey>,
    weight: u64,
    width: u64,
    slope: (u8, u64),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FamilyKey {
    Name(String),
    Generic(u8),
}

/// Die Schriftbibliothek mit Zwischenspeichern für Stil und Layout.
pub struct Fonts<'a> {
    library: &'a FontLibrary,
    /// Je Anfrage die Maße der ersten verfügbaren Schrift bei 1 px.
    metrics: Mutex<BTreeMap<QueryKey, Option<Metrics>>>,
}

impl std::fmt::Debug for Fonts<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Fonts")
            .field("library", &self.library)
            .finish_non_exhaustive()
    }
}

/// Eine Anfrage an die Bibliothek mit ihren Familien.
pub(crate) struct Query {
    families: Vec<Family>,
    weight: f32,
    width: f32,
    slope: Slope,
}

impl Query {
    pub(crate) fn new(font: &Font<'_>) -> Query {
        let families = font
            .family
            .0
            .iter()
            .map(|family| match family {
                FontFamily::Name(name) => Family::Name(name.clone()),
                FontFamily::Generic(generic) => Family::Generic(generic_family(*generic)),
                FontFamily::ScriptGeneric(generic) => Family::Generic(match generic {
                    ScriptGenericFamily::Fangsong => mb_text::GenericFamily::Fangsong,
                    ScriptGenericFamily::Kai => mb_text::GenericFamily::Kai,
                    ScriptGenericFamily::KhmerMul => mb_text::GenericFamily::KhmerMul,
                    ScriptGenericFamily::Nastaliq => mb_text::GenericFamily::Nastaliq,
                }),
            })
            .collect();
        let slope = match font.style {
            FontStyle::Normal => Slope::Normal,
            FontStyle::Italic => Slope::Italic,
            // `left` und `right` gehören zu Fonts 5 und werden noch wie
            // `oblique` gesucht.
            FontStyle::Left | FontStyle::Right | FontStyle::Oblique(None) => Slope::Oblique(14.0),
            FontStyle::Oblique(Some(angle)) => Slope::Oblique(to_f32(angle.degrees())),
        };
        Query {
            families,
            weight: to_f32(font.weight),
            width: to_f32(font.width),
            slope,
        }
    }

    pub(crate) fn as_query(&self) -> FontQuery<'_> {
        FontQuery {
            families: &self.families,
            weight: self.weight,
            width: self.width,
            slope: self.slope,
        }
    }

    fn key(&self) -> QueryKey {
        QueryKey {
            families: self
                .families
                .iter()
                .map(|family| match family {
                    Family::Name(name) => FamilyKey::Name(name.clone()),
                    Family::Generic(generic) => FamilyKey::Generic(generic_number(*generic)),
                })
                .collect(),
            weight: self.weight.to_bits().into(),
            width: self.width.to_bits().into(),
            slope: match self.slope {
                Slope::Normal => (0, 0),
                Slope::Italic => (1, 0),
                Slope::Oblique(angle) => (2, angle.to_bits().into()),
            },
        }
    }
}

fn generic_family(generic: GenericFamily) -> mb_text::GenericFamily {
    use mb_text::GenericFamily as G;
    match generic {
        GenericFamily::Serif => G::Serif,
        GenericFamily::SansSerif => G::SansSerif,
        GenericFamily::SystemUi => G::SystemUi,
        GenericFamily::Cursive => G::Cursive,
        GenericFamily::Fantasy => G::Fantasy,
        GenericFamily::Math => G::Math,
        GenericFamily::Monospace => G::Monospace,
        GenericFamily::UiSerif => G::UiSerif,
        GenericFamily::UiSansSerif => G::UiSansSerif,
        GenericFamily::UiMonospace => G::UiMonospace,
        GenericFamily::UiRounded => G::UiRounded,
    }
}

fn generic_number(generic: mb_text::GenericFamily) -> u8 {
    use mb_text::GenericFamily as G;
    match generic {
        G::Serif => 0,
        G::SansSerif => 1,
        G::Monospace => 2,
        G::Cursive => 3,
        G::Fantasy => 4,
        G::SystemUi => 5,
        G::Math => 6,
        G::UiSerif => 7,
        G::UiSansSerif => 8,
        G::UiMonospace => 9,
        G::UiRounded => 10,
        G::Fangsong => 11,
        G::Kai => 12,
        G::KhmerMul => 13,
        G::Nastaliq => 14,
    }
}

/// Eine Zahl für die Schnittstellen von `mb-text`, die mit `f32` rechnen.
pub(crate) fn to_f32(value: f64) -> f32 {
    // Werte außerhalb des Bereichs von f32 werden unendlich; die Größen,
    // um die es geht, sind auf ±f32::MAX begrenzt.
    value as f32
}

impl<'a> Fonts<'a> {
    pub fn new(library: &'a FontLibrary) -> Fonts<'a> {
        Fonts {
            library,
            metrics: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn library(&self) -> &FontLibrary {
        self.library
    }

    /// Die erste verfügbare Schrift (*first available font*): die erste
    /// Schrift der Liste, sonst die des Browsers. Ohne `unicode-range` ist
    /// das die erste, die es gibt.
    pub(crate) fn first_available(&self, query: &Query) -> Option<Arc<Face>> {
        let request = query.as_query();
        let id = self
            .library
            .select(&request)
            .into_iter()
            .find(|&id| self.library.face(id).is_some())
            .or_else(|| self.library.default_face(&request))?;
        self.library.face(id)
    }

    /// Die Maße der ersten verfügbaren Schrift bei `size` px.
    pub(crate) fn metrics(&self, query: &Query, size: f64) -> Metrics {
        let key = query.key();
        let unit = {
            let mut cache = self.metrics.lock().unwrap_or_else(PoisonError::into_inner);
            match cache.get(&key) {
                Some(found) => *found,
                None => {
                    let found = self.first_available(query).map(|face| unit_metrics(&face));
                    cache.insert(key, found);
                    found
                }
            }
        };
        match unit {
            Some(unit) => Metrics {
                ascent: unit.ascent * size,
                descent: unit.descent * size,
                line_gap: unit.line_gap * size,
                x_height: unit.x_height.map(|value| value * size),
                cap_height: unit.cap_height.map(|value| value * size),
                zero: unit.zero.map(|value| value * size),
                water: unit.water.map(|value| value * size),
            },
            None => Metrics::fallback(size),
        }
    }

    /// Formt Text; die Vorschubbreiten sind in `px`.
    pub(crate) fn shape(&self, query: &Query, size: f64, text: &[u32]) -> Vec<Glyph> {
        mb_text::shape(self.library, &query.as_query(), to_f32(size), text)
    }
}

/// Die Maße einer Schrift bei 1 px.
fn unit_metrics(face: &Face) -> Metrics {
    let metrics = face.metrics();
    let units = f64::from(metrics.units_per_em.max(1));
    let scaled = |value: i16| f64::from(value) / units;
    let (ascent, descent, line_gap) = match (
        metrics.typo_ascender,
        metrics.typo_descender,
        metrics.typo_line_gap,
    ) {
        (Some(ascent), Some(descent), line_gap) => (
            scaled(ascent),
            -scaled(descent),
            line_gap.map_or(0.0, scaled),
        ),
        _ => (
            scaled(metrics.ascender),
            -scaled(metrics.descender),
            scaled(metrics.line_gap),
        ),
    };
    let advance = |character: char| {
        let glyph = face.glyph_index(character)?;
        face.advance_width(glyph)
            .map(|width| f64::from(width) / units)
    };
    Metrics {
        ascent,
        descent,
        line_gap: line_gap.max(0.0),
        x_height: metrics.x_height.filter(|&value| value > 0).map(scaled),
        cap_height: metrics.cap_height.filter(|&value| value > 0).map(scaled),
        zero: advance('0'),
        water: advance('\u{6C34}'),
    }
}

impl FontMetricsSource for Fonts<'_> {
    fn metrics(&self, font: &Font<'_>) -> FontMetrics {
        let query = Query::new(font);
        let metrics = Fonts::metrics(self, &query, font.size);
        // Ohne Schrift fehlen alle Maße; die Annahmen aus Values 4 trifft
        // dann mb-style.
        if self.first_available(&query).is_none() {
            return FontMetrics::default();
        }
        let advance = |value: Option<f64>| {
            value.map(|horizontal| Advance {
                horizontal,
                // Senkrecht gesetzt ist der Vorschub ein Geviert, solange
                // die Tabellen `vmtx` und `vhea` fehlen.
                vertical: font.size,
            })
        };
        FontMetrics {
            x_height: metrics.x_height,
            cap_height: metrics.cap_height,
            ascent: Some(metrics.ascent),
            normal_line_height: Some(metrics.normal_line_height()),
            zero: advance(metrics.zero),
            water: advance(metrics.water),
        }
    }
}
