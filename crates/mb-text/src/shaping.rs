//! Einfache Textformung: Jedes Zeichen bekommt die Glyphe aus `cmap` und
//! die Vorschubbreite aus `hmtx`, ohne GSUB und GPOS, also ohne Ligaturen,
//! Unterschneidung und Positionierung von Akzenten.
//!
//! Welche Schrift ein Zeichen bekommt, entscheidet die Schriftauswahl nach
//! CSS Fonts 4 mit den Regeln für ganze Cluster (Abschnitt „Cluster
//! matching“). Als Cluster gilt hier ein Zeichen mit den unmittelbar
//! folgenden Zeichen der Kategorien Mn, Mc und Me; die Grenzen nach UAX #29
//! fehlen noch. Nach Ersatzschriften für ein Zeichen mit Variantenselektor
//! wird noch nicht gesucht (dafür fehlt `cmap` im Format 14); hat keine
//! Schrift der Liste beide Zeichen, bleibt der Selektor ohne Glyphe, wie es
//! der Abschnitt für den Fall verlangt, dass keine Schrift die Sequenz hat.

use std::sync::Arc;

use mb_unicode::normalization;

use crate::library::{FaceId, FontLibrary, FontQuery};
use crate::opentype::{Face, GlyphId};

/// Eine Glyphe im geformten Text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub face: FaceId,
    pub id: GlyphId,
    /// Der Index des ersten Codepunkts im Text, für den die Glyphe steht.
    pub cluster: usize,
    /// Die Vorschubbreite in CSS-Pixeln.
    pub advance: f32,
}

/// Formt `text` in der Schrift, die `query` beschreibt, bei `size` CSS-Pixeln.
/// Codepunkte, die keine Unicode-Skalarwerte sind (einzelne Surrogate),
/// gelten als U+FFFD. Zeichen, die keine Schrift hat, bekommen die Glyphe 0
/// (*.notdef*) der ersten verfügbaren Schrift; ohne jede Schrift fehlen sie.
pub fn shape(library: &FontLibrary, query: &FontQuery<'_>, size: f32, text: &[u32]) -> Vec<Glyph> {
    let list: Vec<(FaceId, Arc<Face>)> = library
        .select(query)
        .into_iter()
        .filter_map(|id| Some((id, library.face(id)?)))
        .collect();
    let first = list
        .first()
        .map(|(id, _)| *id)
        .or_else(|| library.default_face(query));
    let shaper = Shaper {
        library,
        query,
        size,
        list,
        first,
    };
    let characters: Vec<char> = text
        .iter()
        .map(|&code_point| char::from_u32(code_point).unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    let mut glyphs = Vec::with_capacity(characters.len());
    let mut start = 0;
    while let Some(rest) = characters.get(start..)
        && let Some((_, marks)) = rest.split_first()
    {
        let length = marks.iter().take_while(|&&mark| is_mark(mark)).count();
        let Some(cluster) = rest.get(..=length) else {
            break;
        };
        shaper.cluster(cluster, start, &mut glyphs);
        start = start.saturating_add(cluster.len());
    }
    glyphs
}

/// Mn, Mc und Me.
fn is_mark(character: char) -> bool {
    mb_unicode::general_category(u32::from(character)).is_mark()
}

/// Variation_Selector aus `PropList.txt`.
fn is_variation_selector(character: char) -> bool {
    matches!(
        u32::from(character),
        0x180B..=0x180D | 0x180F | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF
    )
}

fn supports(face: &Face, characters: &[char]) -> bool {
    characters
        .iter()
        .all(|&character| face.glyph_index(character).is_some())
}

struct Shaper<'a> {
    library: &'a FontLibrary,
    query: &'a FontQuery<'a>,
    size: f32,
    /// Die Schriftliste mit geladenen Schnitten.
    list: Vec<(FaceId, Arc<Face>)>,
    /// Die erste verfügbare Schrift (*first available font*).
    first: Option<FaceId>,
}

impl Shaper<'_> {
    /// Ein Cluster ab dem Codepunkt `start`.
    fn cluster(&self, cluster: &[char], start: usize, glyphs: &mut Vec<Glyph>) {
        let [base, marks @ ..] = cluster else {
            return;
        };
        if marks.is_empty() {
            self.single(*base, start, glyphs);
            return;
        }
        // Schritt 1, siehe `match_in_list`.
        if self.match_in_list(cluster, start, glyphs) {
            return;
        }
        // Schritt 2: Mit Variantenselektor nur das Grundzeichen; weitere
        // Selektoren sind ein Kodierungsfehler und fallen weg. Sonst eine
        // Ersatzschrift für den ganzen Cluster.
        if let [selector, rest @ ..] = marks
            && is_variation_selector(*selector)
        {
            self.single(*base, start, glyphs);
            let after = start.saturating_add(2);
            for (offset, &mark) in rest.iter().enumerate() {
                if !is_variation_selector(mark) {
                    self.single(mark, after.saturating_add(offset), glyphs);
                }
            }
            return;
        }
        if let Some(id) = self.library.fallback(self.query, cluster)
            && let Some(face) = self.library.face(id)
        {
            self.emit(id, &face, cluster, start, glyphs);
            return;
        }
        // Schritt 3: der längste Anfang, den eine Schrift der Liste nach
        // Schritt 1 darstellt, die übrigen Zeichen einzeln.
        for length in (1..cluster.len()).rev() {
            let Some(prefix) = cluster.get(..length) else {
                continue;
            };
            if self.match_in_list(prefix, start, glyphs) {
                let rest = cluster.get(length..).unwrap_or_default();
                self.marks_separately(rest, start.saturating_add(length), glyphs);
                return;
            }
        }
        self.single(*base, start, glyphs);
        self.marks_separately(marks, start.saturating_add(1), glyphs);
    }

    /// Schritt 1 für eine Folge ab dem Codepunkt `start`: die erste Schrift
    /// der Liste, die alle Zeichen hat oder das eine Zeichen, das kanonisch
    /// gleich ist. `false`, wenn es keine gibt.
    fn match_in_list(&self, sequence: &[char], start: usize, glyphs: &mut Vec<Glyph>) -> bool {
        let composed = normalization::compose(sequence.iter().map(|&c| u32::from(c)));
        let single = match composed.as_slice() {
            [single] => char::from_u32(*single),
            _ => None,
        };
        for (id, face) in &self.list {
            if supports(face, sequence) {
                self.emit(*id, face, sequence, start, glyphs);
                return true;
            }
            if let Some(single) = single
                && let Some(glyph) = face.glyph_index(single)
            {
                glyphs.push(self.glyph(*id, face, glyph, start));
                return true;
            }
        }
        false
    }

    fn marks_separately(&self, marks: &[char], start: usize, glyphs: &mut Vec<Glyph>) {
        for (offset, &mark) in marks.iter().enumerate() {
            self.single(mark, start.saturating_add(offset), glyphs);
        }
    }

    /// Ein einzelnes Zeichen: die erste Schrift der Liste mit Glyphe, sonst
    /// eine Ersatzschrift, sonst *.notdef* der ersten verfügbaren Schrift.
    fn single(&self, character: char, start: usize, glyphs: &mut Vec<Glyph>) {
        let characters = [character];
        if let Some((id, face)) = self
            .list
            .iter()
            .find(|(_, face)| supports(face, &characters))
        {
            self.emit(*id, face, &characters, start, glyphs);
        } else if let Some(id) = self.library.fallback(self.query, &characters)
            && let Some(face) = self.library.face(id)
        {
            self.emit(id, &face, &characters, start, glyphs);
        } else if let Some(id) = self.first
            && let Some(face) = self.library.face(id)
        {
            glyphs.push(self.glyph(id, &face, GlyphId(0), start));
        }
    }

    /// Die Glyphen für Zeichen, die die Schrift alle hat.
    fn emit(
        &self,
        id: FaceId,
        face: &Face,
        characters: &[char],
        start: usize,
        glyphs: &mut Vec<Glyph>,
    ) {
        for (offset, &character) in characters.iter().enumerate() {
            let glyph = face.glyph_index(character).unwrap_or(GlyphId(0));
            glyphs.push(self.glyph(id, face, glyph, start.saturating_add(offset)));
        }
    }

    fn glyph(&self, id: FaceId, face: &Face, glyph: GlyphId, cluster: usize) -> Glyph {
        let units = f32::from(face.advance_width(glyph).unwrap_or(0));
        let per_em = f32::from(face.metrics().units_per_em);
        Glyph {
            face: id,
            id: glyph,
            cluster,
            advance: units * self.size / per_em,
        }
    }
}

#[cfg(test)]
mod tests;
