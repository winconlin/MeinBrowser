//! Das Stylesheet der Benutzeragenten und die *presentational hints* in CSS
//! aus dem Abschnitt „Rendering“ des HTML Standard. Die Blöcke erzeugt
//! `tools/datagen` aus `data/html-rendering/rendering.json`.
//!
//! Jeder Block ist ein eigenes Stylesheet, denn jeder beginnt mit seiner
//! eigenen `@namespace`-Regel. Bisher gelten die Blöcke für alle Dokumente,
//! die für den *quirks mode*, für HTML-Dokumente und für die Kodierung
//! ISO-8859-8 sowie die *presentational hints*. Es fehlen die Stile im
//! Shadow Tree von `details` (es gibt noch keine Shadow Trees) und die für
//! die Darstellungsarten von `select`. Die Zuordnung von Attributen zu
//! Eigenschaften, die der Standard in Prosa beschreibt, fehlt ebenfalls.

mod tables;

use std::sync::OnceLock;

use mb_dom::QuirksMode;

use crate::cascade::Origin;
use crate::stylesheet::StyleSheet;
use crate::values::ParseContext;

/// Wann die Regeln eines Blocks gelten, wie der Absatz davor es sagt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Condition {
    Always,
    /// Als *presentational hints*: im Ursprung *author presentational hint
    /// origin* mit der Spezifität null.
    PresentationalHint,
    Quirks,
    HtmlDocument,
    /// Wenn das Dokument als ISO-8859-8 kodiert ist.
    Iso8859_8,
    /// Im Shadow Tree eines `details`.
    DetailsShadowTree,
    SelectDropDown,
    SelectNative,
    SelectDropDownNative,
    SelectBase,
    SelectDropDownBase,
    SelectListBoxBase,
}

/// Ein CSS-Block aus dem Standard; die Überschrift seines Abschnitts steht
/// als Kommentar davor.
pub(crate) struct Block {
    pub condition: Condition,
    pub css: &'static str,
}

/// Was die Auswahl der Blöcke über das Dokument wissen muss.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DocumentInfo {
    pub quirks_mode: QuirksMode,
    /// Ein HTML-Dokument, kein XML-Dokument.
    pub html_document: bool,
    /// Ob die Kodierung des Dokuments ISO-8859-8 ist.
    pub iso_8859_8: bool,
}

/// Alle Blöcke, einmal gelesen.
fn sheets() -> &'static [(Condition, StyleSheet)] {
    static SHEETS: OnceLock<Vec<(Condition, StyleSheet)>> = OnceLock::new();
    SHEETS.get_or_init(|| {
        tables::BLOCKS
            .iter()
            .map(|block| {
                let sheet = StyleSheet::parse(block.css, ParseContext::default());
                (block.condition, sheet)
            })
            .collect()
    })
}

/// Die Stylesheets, die für ein Dokument gelten, in der Reihenfolge des
/// Standards, mit ihrem Ursprung.
pub(crate) fn sheets_for(
    info: DocumentInfo,
) -> impl Iterator<Item = (Origin, &'static StyleSheet)> {
    sheets().iter().filter_map(move |(condition, sheet)| {
        let origin = match condition {
            Condition::Always => Origin::UserAgent,
            Condition::PresentationalHint => Origin::PresentationalHint,
            Condition::Quirks if info.quirks_mode == QuirksMode::Quirks => Origin::UserAgent,
            Condition::HtmlDocument if info.html_document => Origin::UserAgent,
            Condition::Iso8859_8 if info.iso_8859_8 => Origin::UserAgent,
            _ => return None,
        };
        Some((origin, sheet))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_have_rules() {
        // Jeder Block, der gilt, ergibt Regeln. Was eine Regel an
        // Unbekanntem enthält, fällt still weg; dass gar nichts übrig
        // bleibt, wäre ein Zeichen für einen Fehler beim Ausziehen oder
        // Lesen.
        assert_eq!(sheets().len(), tables::BLOCKS.len());
        let used = [
            Condition::Always,
            Condition::PresentationalHint,
            Condition::Quirks,
            Condition::HtmlDocument,
            Condition::Iso8859_8,
        ];
        for (block, (condition, sheet)) in tables::BLOCKS.iter().zip(sheets()) {
            if used.contains(condition) {
                assert!(!sheet.rules.is_empty(), "{}", block.css);
            }
        }
    }

    #[test]
    fn selection() {
        let count = |info: DocumentInfo| sheets_for(info).count();
        let plain = count(DocumentInfo::default());
        let html = count(DocumentInfo {
            html_document: true,
            ..DocumentInfo::default()
        });
        let quirks = count(DocumentInfo {
            quirks_mode: QuirksMode::Quirks,
            ..DocumentInfo::default()
        });
        let limited = count(DocumentInfo {
            quirks_mode: QuirksMode::LimitedQuirks,
            ..DocumentInfo::default()
        });
        assert_eq!(html, plain + 1);
        assert_eq!(quirks, plain + 5);
        assert_eq!(limited, plain);
        let hints = sheets_for(DocumentInfo::default())
            .filter(|(origin, _)| *origin == Origin::PresentationalHint)
            .count();
        assert_eq!(hints, 6);
    }
}
