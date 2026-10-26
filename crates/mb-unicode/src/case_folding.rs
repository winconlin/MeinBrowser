//! Faltung der Groß- und Kleinschreibung (Unicode-Standard, Abschnitt 3.13):
//! die vollständige Faltung aus `CaseFolding.txt` (Status C und F) ohne die
//! türkischen Sonderfälle (T). Damit vergleicht CSS etwa Schriftnamen
//! (Default Caseless Matching, D144).
//!
//! Die Faltung erhält keine Normalform; wer kanonisch gleiche Texte als gleich
//! ansehen will, muss zusätzlich normalisieren (D145). Die Funktionen auf
//! Codepunkten nehmen auch einzelne Surrogate an und lassen sie unverändert
//! (RS-9).

use crate::tables::CASE_FOLDING;

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
