//! Normalisierung nach UAX #15: kanonische Zerlegung (NFD) und kanonische
//! Komposition (NFC), dazu die Kompatibilitätsformen NFKD und NFKC.
//!
//! IDNA bringt seine Abbildungen selbst mit. Die Funktionen auf Codepunkten
//! nehmen auch einzelne Surrogate an und lassen sie unverändert (RS-9).

use crate::canonical_combining_class;
use crate::tables::{CANONICAL_COMPOSITION, CANONICAL_DECOMPOSITION, COMPATIBILITY_DECOMPOSITION};

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

/// Die Kompatibilitätszerlegung (NFKD) einer Zeichenkette.
pub fn nfkd(text: &str) -> String {
    to_string(decompose_compatible(text.chars().map(u32::from)))
}

/// Die Kompatibilitätszerlegung mit kanonischer Komposition (NFKC) einer
/// Zeichenkette.
pub fn nfkc(text: &str) -> String {
    to_string(compose_compatible(text.chars().map(u32::from)))
}

/// Kanonische Zerlegung und Sortierung nach Kombinationsklasse.
pub fn decompose(input: impl IntoIterator<Item = u32>) -> Vec<u32> {
    decompose_with(input, false)
}

/// Kompatibilitätszerlegung und Sortierung nach Kombinationsklasse (NFKD).
pub fn decompose_compatible(input: impl IntoIterator<Item = u32>) -> Vec<u32> {
    decompose_with(input, true)
}

fn decompose_with(input: impl IntoIterator<Item = u32>, compatible: bool) -> Vec<u32> {
    let mut output = Vec::new();
    for code_point in input {
        decompose_one(code_point, compatible, &mut output);
    }
    // Canonical Ordering: Jede Folge von Zeichen mit Klasse ungleich 0 wird
    // stabil nach Klasse sortiert.
    for run in output.split_mut(|&code_point| canonical_combining_class(code_point) == 0) {
        run.sort_by_key(|&code_point| canonical_combining_class(code_point));
    }
    output
}

/// Hängt die vollständige Zerlegung eines Codepunkts an, die kanonische
/// oder mit `compatible` die für NFKD.
fn decompose_one(code_point: u32, compatible: bool, output: &mut Vec<u32>) {
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
    // Die Tabelle für NFKD nennt nur die Zeichen, deren Zerlegung von der
    // kanonischen abweicht.
    let compatible = compatible
        .then(|| decomposition(&COMPATIBILITY_DECOMPOSITION, code_point))
        .flatten();
    match compatible.or_else(|| decomposition(&CANONICAL_DECOMPOSITION, code_point)) {
        Some(parts) => output.extend_from_slice(parts),
        None => output.push(code_point),
    }
}

/// Die Zerlegung eines Codepunkts aus einer nach Codepunkt sortierten
/// Tabelle.
fn decomposition(
    table: &'static [(u32, &'static [u32])],
    code_point: u32,
) -> Option<&'static [u32]> {
    let found = table
        .binary_search_by_key(&code_point, |&(key, _)| key)
        .ok()?;
    table.get(found).map(|&(_, parts)| parts)
}

/// Kanonische Zerlegung, dann Komposition (UAX #15, Abschnitt 1.3).
pub fn compose(input: impl IntoIterator<Item = u32>) -> Vec<u32> {
    compose_decomposed(decompose(input))
}

/// Kompatibilitätszerlegung, dann kanonische Komposition (NFKC).
pub fn compose_compatible(input: impl IntoIterator<Item = u32>) -> Vec<u32> {
    compose_decomposed(decompose_compatible(input))
}

/// Die kanonische Komposition einer zerlegten und sortierten Folge.
fn compose_decomposed(decomposed: Vec<u32>) -> Vec<u32> {
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
        assert_eq!(compose_compatible([0xD800, 0xFB01]), [0xD800, 0x66, 0x69]);
    }

    #[test]
    fn compatibility_forms() {
        // Ligatur, Kreiszahl und hochgestellte Ziffer.
        assert_eq!(nfkd("\u{FB01}\u{2460}\u{B2}"), "fi12");
        assert_eq!(nfkc("\u{FB01}\u{2460}\u{B2}"), "fi12");
        // Die kanonischen Formen lassen sie stehen.
        assert_eq!(nfd("\u{FB01}"), "\u{FB01}");
        assert_eq!(nfc("\u{FB01}"), "\u{FB01}");
        // U+1E9B hat eine kanonische Zerlegung mit einem Zeichen, das selbst
        // eine Kompatibilitätszerlegung hat.
        assert_eq!(nfd("\u{1E9B}"), "\u{17F}\u{307}");
        assert_eq!(nfkd("\u{1E9B}"), "s\u{307}");
        assert_eq!(nfkc("\u{1E9B}"), "\u{1E61}");
        // Hangul bleibt berechnet.
        assert_eq!(nfkd("\u{D4DB}"), "\u{1111}\u{1171}\u{11B6}");
        assert_eq!(nfkc("\u{1111}\u{1171}\u{11B6}"), "\u{D4DB}");
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
            // Die Invarianten aus dem Kopf der Datei.
            for input in [source, nfc, nfd] {
                assert_eq!(&compose(input.iter().copied()), nfc, "NFC: {line}");
                assert_eq!(&decompose(input.iter().copied()), nfd, "NFD: {line}");
            }
            for input in [nfkc, nfkd] {
                assert_eq!(&compose(input.iter().copied()), nfkc, "NFC: {line}");
                assert_eq!(&decompose(input.iter().copied()), nfkd, "NFD: {line}");
            }
            for input in [source, nfc, nfd, nfkc, nfkd] {
                let composed = compose_compatible(input.iter().copied());
                assert_eq!(&composed, nfkc, "NFKC: {line}");
                let decomposed = decompose_compatible(input.iter().copied());
                assert_eq!(&decomposed, nfkd, "NFKD: {line}");
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
                let composed = compose_compatible([code_point]);
                assert_eq!(composed, [code_point], "U+{code_point:04X}");
                let decomposed = decompose_compatible([code_point]);
                assert_eq!(decomposed, [code_point], "U+{code_point:04X}");
            }
        }
    }
}
