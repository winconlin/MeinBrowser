//! Läufer für die WPT (siehe `docs/architektur.md`, Abschnitt 10).
//!
//! Bisher gibt es die Liste der Referenztests für M1, ihren Leser und den
//! Läufer für Referenztests: Test und Referenz werden in einem Viewport von
//! 800 × 600 CSS-Pixeln mit einem Gerätepixel je CSS-Pixel gezeichnet und
//! Pixel für Pixel verglichen, ohne Toleranz. `cargo run -p testrunner
//! --release` lässt die Liste für M1 laufen; die Schriften sind die des
//! Systems.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::Path;

use mb_base::Budget;
use mb_layout::Fonts;
use mb_raster::Canvas;

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

/// Breite und Höhe des Viewports der WPT.
pub const VIEWPORT: (f64, f64) = (800.0, 600.0);

/// Zeichnet eine Datei mit dem Budget, das auch das Programm hat; `None`,
/// wenn sie sich nicht laden lässt oder das Budget nicht reicht.
pub fn render(path: &Path, fonts: &Fonts<'_>) -> Option<Canvas> {
    let url = mb_web::file_url(path)?;
    let page = mb_web::Page::load(url, Box::new(mb_web::read_file_within))?;
    let budget = Budget::new(mb_web::RENDER_STEPS);
    let scene = page
        .render_within(fonts, VIEWPORT.0, VIEWPORT.1, &budget)
        .ok()?;
    mb_raster::render(&scene).ok()
}

/// Wie viele Pixel verschieden sind; Flächen verschiedener Größe sind ganz
/// verschieden.
pub fn differing_pixels(first: &Canvas, second: &Canvas) -> usize {
    if (first.width(), first.height()) != (second.width(), second.height()) {
        return first.pixels().len().max(second.pixels().len());
    }
    first
        .pixels()
        .iter()
        .zip(second.pixels())
        .filter(|(a, b)| a != b)
        .count()
}

/// Die Zeichenfläche als Bild im Format PPM (P6), etwa zum Ansehen von
/// Unterschieden. Die Pixel werden über Weiß gelegt.
pub fn to_ppm(canvas: &Canvas) -> Vec<u8> {
    let mut output = format!("P6\n{} {}\n255\n", canvas.width(), canvas.height()).into_bytes();
    for &pixel in canvas.pixels() {
        let [alpha, red, green, blue] = pixel.to_be_bytes();
        // Vormultipliziert: Weiß scheint mit 255 − Alpha durch.
        let white = 255 - alpha;
        output.extend([red, green, blue].map(|channel| channel.saturating_add(white)));
    }
    output
}
