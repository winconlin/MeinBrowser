//! Normalisierung nach UAX #15: kanonische Zerlegung (NFD) und kanonische
//! Komposition (NFC).
//!
//! Die Kompatibilitätsformen NFKD und NFKC fehlen noch; IDNA bringt seine
//! Abbildungen selbst mit. Die Funktionen auf Codepunkten nehmen auch
//! einzelne Surrogate an und lassen sie unverändert (RS-9).

use crate::canonical_combining_class;
use crate::tables::{CANONICAL_COMPOSITION, CANONICAL_DECOMPOSITION};

// Hangul-Silben werden berechnet (Unicode-Standard, Abschnitt 3.12).
const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

/// Die kanonische Zerlegung (NFD) einer Zeichenkette.
pub fn nfd(text: &str) -> String {
    to_string(decompose(text.chars().map(u32::from)))
}

/// Die kanonische Komposition (NFC) einer Zeichenkette.
pub fn nfc(text: &str) -> String {
    to_string(compose(text.chars().map(u32::from)))
}

/// Ob die Zeichenkette schon in NFC vorliegt.
pub fn is_nfc(text: &str) -> bool {
    compose(text.chars().map(u32::from))
        .into_iter()
        .eq(text.chars().map(u32::from))
}

/// Die Tabellen und Hangul liefern nur Skalarwerte.
fn to_string(code_points: Vec<u32>) -> String {
    code_points.into_iter().filter_map(char::from_u32).collect()
}

/// Kanonische Zerlegung und Sortierung nach Kombinationsklasse.
pub fn decompose(input: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut output = Vec::new();
    for code_point in input {
        decompose_one(code_point, &mut output);
    }
    // Canonical Ordering: Jede Folge von Zeichen mit Klasse ungleich 0 wird
    // stabil nach Klasse sortiert.
    for run in output.split_mut(|&code_point| canonical_combining_class(code_point) == 0) {
        run.sort_by_key(|&code_point| canonical_combining_class(code_point));
    }
    output
}

/// Hängt die vollständige kanonische Zerlegung eines Codepunkts an.
fn decompose_one(code_point: u32, output: &mut Vec<u32>) {
    if let Some(index) = code_point
        .checked_sub(S_BASE)
        .filter(|&index| index < S_COUNT)
    {
        output.push(L_BASE + index / N_COUNT);
        output.push(V_BASE + (index % N_COUNT) / T_COUNT);
        let trailing = index % T_COUNT;
        if trailing != 0 {
            output.push(T_BASE + trailing);
        }
        return;
    }
    let found = CANONICAL_DECOMPOSITION.binary_search_by_key(&code_point, |&(key, _)| key);
    match found
        .ok()
        .and_then(|found| CANONICAL_DECOMPOSITION.get(found))
    {
        Some(&(_, parts)) => output.extend_from_slice(parts),
        None => output.push(code_point),
    }
}

/// Kanonische Zerlegung, dann Komposition (UAX #15, Abschnitt 1.3).
pub fn compose(input: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let decomposed = decompose(input);
    let mut output: Vec<u32> = Vec::with_capacity(decomposed.len());
    // Position des letzten Starters und Klasse des letzten Zeichens danach.
    let mut starter: Option<usize> = None;
    let mut last_class: Option<u8> = None;
    for code_point in decomposed {
        let class = canonical_combining_class(code_point);
        // Nicht blockiert ist ein Zeichen direkt nach dem Starter oder nach
        // Zeichen mit kleinerer Klasse.
        let blocked = last_class.is_some_and(|last| last >= class);
        if !blocked
            && let Some(slot) = starter.and_then(|index| output.get_mut(index))
            && let Some(composite) = compose_pair(*slot, code_point)
        {
            *slot = composite;
            continue;
        }
        if class == 0 {
            starter = Some(output.len());
            last_class = None;
        } else {
            last_class = Some(class);
        }
        output.push(code_point);
    }
    output
}

/// Das primäre Kompositum aus zwei Codepunkten.
fn compose_pair(first: u32, second: u32) -> Option<u32> {
    let leading = first.checked_sub(L_BASE).filter(|&l| l < L_COUNT);
    let vowel = second.checked_sub(V_BASE).filter(|&v| v < V_COUNT);
    if let (Some(leading), Some(vowel)) = (leading, vowel) {
        return Some(S_BASE + (leading * V_COUNT + vowel) * T_COUNT);
    }
    let syllable = first
        .checked_sub(S_BASE)
        .filter(|&s| s < S_COUNT && s % T_COUNT == 0);
    let trailing = second.checked_sub(T_BASE).filter(|&t| t > 0 && t < T_COUNT);
    if let (Some(_), Some(trailing)) = (syllable, trailing) {
        return Some(first + trailing);
    }
    let found = CANONICAL_COMPOSITION
        .binary_search_by_key(&(first, second), |&(a, b, _)| (a, b))
        .ok()?;
    CANONICAL_COMPOSITION
        .get(found)
        .map(|&(_, _, composite)| composite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples() {
        assert_eq!(nfd("À"), "A\u{300}");
        assert_eq!(nfc("A\u{300}"), "À");
        // Ångström-Zeichen ist eine Einzelzerlegung und kommt nicht zurück.
        assert_eq!(nfc("\u{212B}"), "Å");
        // Sortierung nach Klasse: Unterpunkt (220) vor Akut (230).
        assert_eq!(nfd("q\u{301}\u{323}"), "q\u{323}\u{301}");
        assert_eq!(nfc("\u{1E0B}\u{323}"), "\u{1E0D}\u{307}");
        // Ausnahme der Komposition.
        assert_eq!(nfc("\u{915}\u{93C}"), "\u{915}\u{93C}");
        assert_eq!(nfc("\u{958}"), "\u{915}\u{93C}");
        // Hangul in beide Richtungen.
        assert_eq!(nfd("\u{D4DB}"), "\u{1111}\u{1171}\u{11B6}");
        assert_eq!(nfc("\u{1111}\u{1171}\u{11B6}"), "\u{D4DB}");
        assert_eq!(nfc("\u{1100}\u{1161}"), "\u{AC00}");
        assert!(is_nfc("Straße"));
        assert!(!is_nfc("A\u{300}"));
        // Einzelne Surrogate bleiben, wie sie sind.
        assert_eq!(compose([0xD800, 0x41, 0x300]), [0xD800, 0xC0]);
    }

    /// Ein Feld der Testdatei: Codepunkte, durch Leerzeichen getrennt.
    fn field(text: &str) -> Vec<u32> {
        text.split_whitespace()
            .map(|code_point| u32::from_str_radix(code_point, 16).expect("Codepunkt"))
            .collect()
    }

    #[test]
    fn normalization_test_suite() {
        let text = testdata::read(
            "unicodetools",
            "unicodetools/data/ucd/18.0.0/NormalizationTest.txt",
        );
        let mut part = "";
        let mut listed = std::collections::BTreeSet::new();
        let mut cases = 0;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix('@') {
                part = rest.split_whitespace().next().unwrap_or_default();
                continue;
            }
            let data = line.split('#').next().unwrap_or_default();
            if data.trim().is_empty() {
                continue;
            }
            let columns: Vec<Vec<u32>> = data.split(';').take(5).map(field).collect();
            let [source, nfc, nfd, nfkc, nfkd] = columns.as_slice() else {
                panic!("Zeile ohne fünf Spalten: {line}");
            };
            // Die Invarianten aus dem Kopf der Datei, soweit sie NFC und NFD
            // betreffen.
            for input in [source, nfc, nfd] {
                assert_eq!(&compose(input.iter().copied()), nfc, "NFC: {line}");
                assert_eq!(&decompose(input.iter().copied()), nfd, "NFD: {line}");
            }
            for input in [nfkc, nfkd] {
                assert_eq!(&compose(input.iter().copied()), nfkc, "NFC: {line}");
                assert_eq!(&decompose(input.iter().copied()), nfkd, "NFD: {line}");
            }
            if part == "Part1"
                && let [code_point] = source.as_slice()
            {
                listed.insert(*code_point);
            }
            cases += 1;
        }
        assert!(cases > 10_000, "nur {cases} Fälle gelesen");
        // Alle übrigen Zeichen bleiben unverändert.
        for code_point in (0..=0x10_FFFF).filter(|code_point| !listed.contains(code_point)) {
            if char::from_u32(code_point).is_some() {
                assert_eq!(compose([code_point]), [code_point], "U+{code_point:04X}");
                assert_eq!(decompose([code_point]), [code_point], "U+{code_point:04X}");
            }
        }
    }
}
