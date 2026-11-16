//! Faltung der Groß- und Kleinschreibung (Unicode-Standard, Abschnitt 3.13):
//! die vollständige Faltung aus `CaseFolding.txt` (Status C und F) ohne die
//! türkischen Sonderfälle (T). Damit vergleicht CSS etwa Schriftnamen
//! (Default Caseless Matching, D144).
//!
//! Die Faltung erhält keine Normalform; wer kanonisch gleiche Texte als gleich
//! ansehen will, muss zusätzlich normalisieren (D145). Die Funktionen auf
//! Codepunkten nehmen auch einzelne Surrogate an und lassen sie unverändert
//! (RS-9).
//!
//! Dazu die einfache Faltung (scf, Status C und S), die jeden Codepunkt auf
//! genau einen abbildet. Mit ihr vergleichen reguläre Ausdrücke mit `i` und
//! `u` oder `v`.

use crate::tables::{CASE_FOLDING, SIMPLE_CASE_FOLDING};

/// Die gefaltete Zeichenkette (toCasefold).
pub fn fold(text: &str) -> String {
    folded(text).filter_map(char::from_u32).collect()
}

/// Faltet eine Folge von Codepunkten.
pub fn fold_code_points(input: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut output = Vec::new();
    for code_point in input {
        match folding(code_point) {
            Some(mapping) => output.extend_from_slice(mapping),
            None => output.push(code_point),
        }
    }
    output
}

/// Ob zwei Zeichenketten nach Default Caseless Matching gleich sind:
/// toCasefold(a) = toCasefold(b), ohne Normalisierung.
pub fn caseless_eq(a: &str, b: &str) -> bool {
    folded(a).eq(folded(b))
}

fn folded(text: &str) -> impl Iterator<Item = u32> + '_ {
    text.chars().flat_map(|c| {
        let code_point = u32::from(c);
        let mapping = folding(code_point).unwrap_or_default();
        let unchanged = mapping.is_empty().then_some(code_point);
        mapping.iter().copied().chain(unchanged)
    })
}

/// Die einfache Faltung eines Codepunkts (scf): der Eintrag mit Status S,
/// sonst der mit Status C, sonst der Codepunkt selbst.
pub fn simple_fold(code_point: u32) -> u32 {
    simple(code_point, folding(code_point)).unwrap_or(code_point)
}

/// Alle Codepunkte, die die einfache Faltung ändert, mit ihrem Ziel,
/// aufsteigend nach Codepunkt.
pub fn simple_foldings() -> impl Iterator<Item = (u32, u32)> {
    CASE_FOLDING.iter().filter_map(|&(code_point, mapping)| {
        simple(code_point, Some(mapping)).map(|target| (code_point, target))
    })
}

/// Das Ziel der einfachen Faltung zu einem Eintrag der vollständigen. Einen
/// Eintrag mit Status S gibt es nur neben einem mit Status F, und einer mit
/// Status C hat genau einen Codepunkt; beides prüft `tools/datagen`.
fn simple(code_point: u32, full: Option<&'static [u32]>) -> Option<u32> {
    match full? {
        [target] => Some(*target),
        _ => {
            let index = SIMPLE_CASE_FOLDING
                .binary_search_by_key(&code_point, |&(source, _)| source)
                .ok()?;
            SIMPLE_CASE_FOLDING.get(index).map(|&(_, target)| target)
        }
    }
}

/// Die Faltung eines Codepunkts, `None` für alle, die bleiben, wie sie sind.
fn folding(code_point: u32) -> Option<&'static [u32]> {
    let index = CASE_FOLDING
        .binary_search_by_key(&code_point, |&(source, _)| source)
        .ok()?;
    CASE_FOLDING.get(index).map(|&(_, mapping)| mapping)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_foldings() {
        assert_eq!(fold("Arial"), "arial");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(fold("STRASSE"), "strasse");
        assert_eq!(fold("ΣΑΣ"), "σασ");
        assert_eq!(fold("ς"), "σ");
        assert_eq!(fold("ﬀ"), "ff");
        // Ohne die türkische Sonderform: İ wird i mit Punkt darüber.
        assert_eq!(fold("İ"), "i\u{307}");
        assert_eq!(fold("I"), "i");
        assert_eq!(fold("字"), "字");
        assert!(caseless_eq("DejaVu Sans", "dejavu sans"));
        assert!(caseless_eq("Fuß", "FUSS"));
        assert!(!caseless_eq("Arial", "Arial "));
        // Ohne Normalisierung bleiben kanonisch gleiche Texte verschieden.
        assert!(!caseless_eq("\u{C5}", "A\u{30A}"));
        // Einzelne Surrogate bleiben, wie sie sind.
        assert_eq!(
            fold_code_points([0x41, 0xD800, 0xDF]),
            [0x61, 0xD800, 0x73, 0x73]
        );
    }

    #[test]
    fn simple_foldings_map_to_one_code_point() {
        assert_eq!(simple_fold(0x41), 0x61);
        assert_eq!(simple_fold(0x61), 0x61);
        // ß hat nur eine vollständige Faltung, ẞ auch eine einfache.
        assert_eq!(simple_fold(0xDF), 0xDF);
        assert_eq!(simple_fold(0x1E9E), 0xDF);
        // ſ und das Kelvin-Zeichen falten in den ASCII-Bereich.
        assert_eq!(simple_fold(0x17F), 0x73);
        assert_eq!(simple_fold(0x212A), 0x6B);
        // Ohne die türkische Sonderform.
        assert_eq!(simple_fold(0x130), 0x130);
        assert_eq!(simple_fold(0xD800), 0xD800);
        assert_eq!(simple_fold(0x11_0000), 0x11_0000);
        // Die Liste ist aufsteigend, ändert jeden ihrer Codepunkte und ist
        // idempotent: Ein Ziel wird nicht weiter gefaltet.
        let all: Vec<(u32, u32)> = simple_foldings().collect();
        assert!(all.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(all.contains(&(0x1E9E, 0xDF)));
        for &(source, target) in &all {
            assert_ne!(source, target);
            assert_eq!(simple_fold(source), target);
            assert_eq!(simple_fold(target), target, "U+{source:04X}");
        }
    }

    /// Vergleicht jeden Codepunkt mit `CaseFolding.txt`, gelesen unabhängig
    /// vom Generator.
    #[test]
    fn matches_case_folding_file() {
        let text = include_str!("../../../data/unicode/CaseFolding.txt");
        let mut expected = std::collections::BTreeMap::new();
        for line in text.lines() {
            let data = line.split('#').next().unwrap_or_default();
            let fields: Vec<&str> = data.split(';').map(str::trim).collect();
            let [code_point, status, mapping, ..] = fields.as_slice() else {
                continue;
            };
            if matches!(*status, "C" | "F") {
                let code_point = u32::from_str_radix(code_point, 16).expect("Codepunkt");
                let mapping: Vec<u32> = mapping
                    .split_whitespace()
                    .map(|part| u32::from_str_radix(part, 16).expect("Codepunkt"))
                    .collect();
                expected.insert(code_point, mapping);
            }
        }
        assert!(expected.len() > 1500, "nur {} Einträge", expected.len());
        for code_point in 0..=crate::MAX_CODE_POINT {
            let folded = fold_code_points([code_point]);
            match expected.get(&code_point) {
                Some(mapping) => assert_eq!(&folded, mapping, "U+{code_point:04X}"),
                None => assert_eq!(folded, [code_point], "U+{code_point:04X}"),
            }
        }
    }
}
