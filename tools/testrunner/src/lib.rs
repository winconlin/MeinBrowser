//! Läufer für die WPT (siehe `docs/architektur.md`, Abschnitt 10).
//!
//! Bisher gibt es nur die Liste der Referenztests für M1 und ihren Leser;
//! der Läufer selbst kommt mit Layout und CPU-Renderer.

#![forbid(unsafe_code)]

use std::fmt;

#[cfg(test)]
mod tests;

/// Die Referenztests für M1. Wie sie ausgewählt sind, steht im Kopf der
/// Datei.
pub const M1_REFTESTS: &str = include_str!("../m1-referenztests.txt");

/// Wie Test und Referenz zueinander stehen sollen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    /// `==`: Sie müssen gleich aussehen (`<link rel="match">`).
    Match,
    /// `!=`: Sie müssen verschieden aussehen (`<link rel="mismatch">`).
    Mismatch,
}

/// Ein Eintrag einer Liste von Referenztests. Die Pfade sind relativ zur
/// Wurzel der WPT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefTest {
    pub relation: Relation,
    pub test: String,
    pub reference: String,
}

/// Eine Zeile einer Liste, die sich nicht lesen lässt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListError {
    /// Die Nummer der Zeile, ab 1.
    pub line: usize,
    pub message: &'static str,
}

impl fmt::Display for ListError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Zeile {}: {}", self.line, self.message)
    }
}

/// Liest eine Liste von Referenztests: je Zeile `==` oder `!=`, den Test und
/// die Referenz, getrennt durch Leerraum. Leere Zeilen und Zeilen, die mit
/// `#` beginnen, zählen nicht.
pub fn parse_list(text: &str) -> Result<Vec<RefTest>, ListError> {
    let mut tests = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let error = |message| ListError {
            line: index.saturating_add(1),
            message,
        };
        let mut fields = line.split_whitespace();
        let relation = match fields.next() {
            Some("==") => Relation::Match,
            Some("!=") => Relation::Mismatch,
            _ => return Err(error("`==` oder `!=` erwartet")),
        };
        let (Some(test), Some(reference), None) = (fields.next(), fields.next(), fields.next())
        else {
            return Err(error("Test und Referenz erwartet"));
        };
        tests.push(RefTest {
            relation,
            test: test.to_owned(),
            reference: reference.to_owned(),
        });
    }
    Ok(tests)
}
