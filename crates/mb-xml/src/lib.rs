//! Parser für XML, XHTML und SVG.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.4. Grundlage sind XML 1.0
//! (Fifth Edition, REC-xml-20081126), Namespaces in XML 1.0 (Third Edition,
//! REC-xml-names-20091208) und der Abschnitt „Parsing XML documents“ im HTML
//! Standard. Der Parser prüft nicht auf Gültigkeit und liest keine externen
//! Entitäten (Abschnitt 5.1); für XHTML kennt er die Entitäten des HTML
//! Standard. Beim ersten fatalen Fehler hört er auf. Was er bis dahin
//! gelesen hat, steht im Dokument; was ein Browser dann zeigt, lässt der
//! HTML Standard offen.
//!
//! Skripte führt der Parser nicht aus (*XML scripting support disabled*).

#![forbid(unsafe_code)]

mod chars;
mod encoding;
mod namespaces;
mod parser;

#[cfg(test)]
mod conformance;
#[cfg(test)]
mod tests;

use mb_dom::Document;
use mb_encoding::Encoding;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Namensräume auflösen und ihre Bedingungen prüfen. Ohne heißen
    /// Elemente und Attribute so, wie sie dastehen, und haben keinen
    /// Namensraum. Browser lesen immer mit Namensräumen.
    pub namespaces: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { namespaces: true }
    }
}

/// Ein fataler Fehler mit seiner Stelle im Dokument, gezählt ab 1 in
/// Zeichen nach dem Vereinheitlichen der Zeilenenden. Liegt der Fehler im
/// Ersetzungstext einer Entität, ist die Stelle die nach dem Verweis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XmlError {
    pub line: usize,
    pub column: usize,
    pub message: &'static str,
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Zeile {}, Spalte {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for XmlError {}

/// Das Ergebnis von [`parse`].
pub struct Parsed {
    /// Ein XML-Dokument (DOM: *type* „xml“).
    pub document: Document,
    /// Die Zeichenkodierung; keine, wenn sich der Text nicht dekodieren ließ.
    pub encoding: Option<Encoding>,
    pub error: Option<XmlError>,
}

/// Liest ein Dokument aus Bytes.
pub fn parse(bytes: &[u8], options: &Options) -> Parsed {
    let mut document = Document::new();
    document.set_html_document(false);
    match encoding::decode(bytes) {
        Ok((text, encoding)) => {
            let (document, error) = parser::run(document, &text, options.namespaces);
            Parsed {
                document,
                encoding: Some(encoding),
                error,
            }
        }
        Err(message) => Parsed {
            document,
            encoding: None,
            error: Some(XmlError {
                line: 1,
                column: 1,
                message,
            }),
        },
    }
}
