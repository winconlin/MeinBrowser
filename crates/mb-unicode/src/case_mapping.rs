//! Umwandlung in Klein- und Großbuchstaben (Unicode-Standard, Abschnitt
//! 3.13, Default Case Conversion): toLowercase und toUppercase mit den
//! vollständigen Abbildungen aus `SpecialCasing.txt` und sonst den einfachen
//! aus `UnicodeData.txt`.
//!
//! Von den Abbildungen mit Bedingung gilt nur Final_Sigma: U+03A3 wird am
//! Ende eines Worts zu U+03C2 statt zu U+03C3. Die Zeilen für einzelne
//! Sprachen (Litauisch, Türkisch, Aserbaidschanisch) bleiben weg; so
//! verlangt es ECMA-262 für `toLowerCase` und `toUpperCase`. Die Funktionen
//! nehmen auch einzelne Surrogate an und lassen sie unverändert (RS-9).

use crate::lookup;
use crate::tables::{CASE_IGNORABLE, CASED, LOWERCASE, UPPERCASE};

/// U+03A3 GREEK CAPITAL LETTER SIGMA.
const CAPITAL_SIGMA: u32 = 0x03A3;
/// U+03C2 GREEK SMALL LETTER FINAL SIGMA.
const FINAL_SIGMA: u32 = 0x03C2;

/// Ob ein Codepunkt Cased ist (Unicode-Standard, D135).
pub fn is_cased(code_point: u32) -> bool {
    lookup(&CASED, code_point).unwrap_or(false)
}

/// Ob ein Codepunkt Case_Ignorable ist (Unicode-Standard, D136).
pub fn is_case_ignorable(code_point: u32) -> bool {
    lookup(&CASE_IGNORABLE, code_point).unwrap_or(false)
}

/// toLowercase(X) für eine Folge von Codepunkten.
pub fn to_lowercase(input: &[u32]) -> Vec<u32> {
    let mut output = Vec::with_capacity(input.len());
    for (index, &code_point) in input.iter().enumerate() {
        if code_point == CAPITAL_SIGMA && is_final_sigma(input, index) {
            output.push(FINAL_SIGMA);
            continue;
        }
        match mapping(&LOWERCASE, code_point) {
            Some(mapping) => output.extend_from_slice(mapping),
            None => output.push(code_point),
        }
    }
    output
}

/// toUppercase(X) für eine Folge von Codepunkten.
pub fn to_uppercase(input: &[u32]) -> Vec<u32> {
    let mut output = Vec::with_capacity(input.len());
    for &code_point in input {
        match mapping(&UPPERCASE, code_point) {
            Some(mapping) => output.extend_from_slice(mapping),
            None => output.push(code_point),
        }
    }
    output
}

/// Die Bedingung Final_Sigma (Unicode-Standard, Tabelle 3-17) für das
/// Zeichen an der Stelle `index`: Davor steht ein Zeichen mit Cased, von
/// ihm höchstens durch Zeichen mit Case_Ignorable getrennt, und danach
/// folgt keines auf dieselbe Weise.
fn is_final_sigma(input: &[u32], index: usize) -> bool {
    let (before, after) = input.split_at(index);
    next_is_cased(before.iter().rev()) && !next_is_cased(after.iter().skip(1))
}

/// Ob das erste Zeichen ohne Case_Ignorable die Eigenschaft Cased hat.
fn next_is_cased<'a>(mut code_points: impl Iterator<Item = &'a u32>) -> bool {
    code_points
        .find(|&&code_point| !is_case_ignorable(code_point))
        .is_some_and(|&code_point| is_cased(code_point))
}

/// Die Abbildung eines Codepunkts, `None` für alle, die bleiben, wie sie
/// sind.
fn mapping(table: &'static [(u32, &'static [u32])], code_point: u32) -> Option<&'static [u32]> {
    let index = table
        .binary_search_by_key(&code_point, |&(source, _)| source)
        .ok()?;
    table.get(index).map(|&(_, mapping)| mapping)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_points(text: &str) -> Vec<u32> {
        text.chars().map(u32::from).collect()
    }

    fn lower(text: &str) -> String {
        to_lowercase(&code_points(text))
            .into_iter()
            .filter_map(char::from_u32)
            .collect()
    }

    fn upper(text: &str) -> String {
        to_uppercase(&code_points(text))
            .into_iter()
            .filter_map(char::from_u32)
            .collect()
    }

    #[test]
    fn simple_and_full_mappings() {
        assert_eq!(lower("Straße ÄÖÜ"), "straße äöü");
        assert_eq!(upper("Straße äöü"), "STRASSE ÄÖÜ");
        // Vollständige Abbildungen aus SpecialCasing.txt.
        assert_eq!(upper("\u{FB01}"), "FI");
        assert_eq!(upper("\u{149}"), "\u{2BC}N");
        assert_eq!(lower("\u{130}"), "i\u{307}");
        // Ohne Sprache bleibt das türkische I, wie es überall ist.
        assert_eq!(lower("I"), "i");
        assert_eq!(upper("i"), "I");
        // Zeichen ohne Groß- und Kleinschreibung und über der BMP.
        assert_eq!(lower("123 \u{10400}"), "123 \u{10428}");
        assert_eq!(upper("\u{10428}"), "\u{10400}");
        // Titelbuchstaben.
        assert_eq!(lower("\u{1C5}"), "\u{1C6}");
        assert_eq!(upper("\u{1C5}"), "\u{1C4}");
    }

    #[test]
    fn final_sigma() {
        assert_eq!(lower("ΟΔΟΣ"), "οδος");
        assert_eq!(lower("ΟΔΟΣ ΣΟΦΟΣ"), "οδος σοφος");
        // Allein und am Wortanfang steht kein Zeichen mit Cased davor.
        assert_eq!(lower("Σ"), "σ");
        assert_eq!(lower("ΣΑ"), "σα");
        // Zeichen mit Case_Ignorable zählen auf beiden Seiten nicht.
        assert_eq!(lower("Α\u{301}Σ\u{301}"), "α\u{301}ς\u{301}");
        assert_eq!(lower("ΑΣ.Β"), "ασ.β");
        assert_eq!(lower("ΑΣ. Β"), "ας. β");
        assert_eq!(lower("Α.Σ"), "α.ς");
        assert_eq!(lower("Α Σ"), "α σ");
        assert_eq!(upper("ος"), "ΟΣ");
    }

    #[test]
    fn lone_surrogates_stay() {
        assert_eq!(to_lowercase(&[0xD800, 0x41]), [0xD800, 0x61]);
        assert_eq!(to_uppercase(&[0x61, 0xDFFF]), [0x41, 0xDFFF]);
        assert_eq!(to_lowercase(&[0x11_0000]), [0x11_0000]);
    }

    #[test]
    fn properties() {
        assert!(is_cased(u32::from('a')));
        assert!(is_cased(0x1C5));
        assert!(!is_cased(u32::from('1')));
        assert!(is_case_ignorable(u32::from('.')));
        assert!(is_case_ignorable(0x301));
        assert!(!is_case_ignorable(u32::from(' ')));
        assert!(!is_cased(0x11_0000));
    }
}
