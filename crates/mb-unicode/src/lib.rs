//! Aus der Unicode-Zeichendatenbank generierte Tabellen und die Unicode-Algorithmen (Normalisierung, Bidi, Zeilenumbruch, Segmentierung, IDNA).
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.3.
//!
//! Die Tabellen in `tables.rs` erzeugt `cargo run -p datagen` aus den
//! Rohdaten unter `data/unicode/`. Codepunkte sind hier `u32` statt `char`,
//! damit auch einzelne Surrogate aus JavaScript-Zeichenketten abgefragt
//! werden können (RS-9).

#![forbid(unsafe_code)]

pub mod normalization;
#[rustfmt::skip]
mod tables;

pub use tables::{BidiClass, GeneralCategory, JoiningType, UNICODE_VERSION};

/// Der größte Codepunkt.
pub const MAX_CODE_POINT: u32 = 0x10_FFFF;

/// Die allgemeine Kategorie eines Codepunkts. Werte über U+10FFFF sind keine
/// Codepunkte und gelten als nicht vergeben.
pub fn general_category(code_point: u32) -> GeneralCategory {
    lookup(&tables::GENERAL_CATEGORY, code_point).unwrap_or(GeneralCategory::Unassigned)
}

/// Die kanonische Kombinationsklasse eines Codepunkts, 0 für alle Zeichen,
/// die bei der Normalisierung nicht umsortiert werden.
pub fn canonical_combining_class(code_point: u32) -> u8 {
    lookup(&tables::CANONICAL_COMBINING_CLASS, code_point).unwrap_or(0)
}

/// Die Bidi-Klasse eines Codepunkts. Nicht vergebene Codepunkte haben die
/// Vorgaben aus `DerivedBidiClass.txt`, Werte über U+10FFFF gelten als `L`.
pub fn bidi_class(code_point: u32) -> BidiClass {
    lookup(&tables::BIDI_CLASS, code_point).unwrap_or(BidiClass::LeftToRight)
}

/// Der Joining-Type eines Codepunkts, `U` (nicht verbindend) für alle nicht
/// genannten.
pub fn joining_type(code_point: u32) -> JoiningType {
    lookup(&tables::JOINING_TYPE, code_point).unwrap_or(JoiningType::NonJoining)
}

/// Sucht den Abschnitt, in dem der Codepunkt liegt. Die Tabellen beginnen
/// bei 0 und sind nach Anfang sortiert.
fn lookup<T: Copy>(table: &[(u32, T)], code_point: u32) -> Option<T> {
    if code_point > MAX_CODE_POINT {
        return None;
    }
    let after = table.partition_point(|&(start, _)| start <= code_point);
    table.get(after.checked_sub(1)?).map(|&(_, value)| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version() {
        assert_eq!(UNICODE_VERSION, (18, 0, 0));
    }

    #[test]
    fn known_categories() {
        use GeneralCategory as G;
        let cases = [
            (0x0000, G::Control),
            (0x0020, G::SpaceSeparator),
            (0x0030, G::DecimalNumber),
            (0x0041, G::UppercaseLetter),
            (0x0061, G::LowercaseLetter),
            (0x00AD, G::Format),
            (0x01C5, G::TitlecaseLetter),
            (0x0300, G::NonspacingMark),
            (0x0378, G::Unassigned),
            (0x20AC, G::CurrencySymbol),
            (0x4E00, G::OtherLetter),
            (0xD800, G::Surrogate),
            (0xE000, G::PrivateUse),
            (0x1F600, G::OtherSymbol),
            (0x10_FFFD, G::PrivateUse),
            (0x10_FFFF, G::Unassigned),
            (0x11_0000, G::Unassigned),
            (u32::MAX, G::Unassigned),
        ];
        for (code_point, category) in cases {
            assert_eq!(general_category(code_point), category, "{code_point:#X}");
        }
    }

    #[test]
    fn names_and_groups() {
        let lu = general_category(0x41);
        assert_eq!(lu.short_name(), "Lu");
        assert_eq!(lu.long_name(), "Uppercase_Letter");
        assert!(lu.is_letter() && lu.is_cased_letter());
        assert!(!lu.is_mark());
        let mn = general_category(0x300);
        assert!(mn.is_mark() && !mn.is_letter());
        assert!(general_category(0x378).is_other());
        assert!(general_category(0x2028).is_separator());
    }

    #[test]
    fn known_combining_classes() {
        let cases = [
            (0x0041, 0),
            (0x0300, 230),
            (0x0316, 220),
            (0x0334, 1),
            (0x05B0, 10),
            (0x093C, 7),
            (0x094D, 9),
            (0x10_FFFF, 0),
            (0x11_0000, 0),
        ];
        for (code_point, class) in cases {
            assert_eq!(
                canonical_combining_class(code_point),
                class,
                "{code_point:#X}"
            );
        }
    }

    #[test]
    fn known_bidi_classes() {
        use BidiClass as B;
        let cases = [
            (0x0041, B::LeftToRight),
            (0x05D0, B::RightToLeft),
            (0x0627, B::ArabicLetter),
            (0x0030, B::EuropeanNumber),
            (0x0660, B::ArabicNumber),
            (0x0300, B::NonspacingMark),
            (0x200B, B::BoundaryNeutral),
            (0x0020, B::WhiteSpace),
            (0x0021, B::OtherNeutral),
            (0x0024, B::EuropeanTerminator),
            (0x002C, B::CommonSeparator),
            (0x002B, B::EuropeanSeparator),
            (0x000A, B::ParagraphSeparator),
            (0x0009, B::SegmentSeparator),
            (0x202A, B::LeftToRightEmbedding),
            (0x2066, B::LeftToRightIsolate),
            // Vorgaben für nicht vergebene Codepunkte.
            (0x05FF, B::RightToLeft),
            (0x07BF, B::ArabicLetter),
            (0x20CF, B::EuropeanTerminator),
            (0xFDD0, B::BoundaryNeutral),
            (0xE0080, B::BoundaryNeutral),
            (0x10_FFFF, B::BoundaryNeutral),
            (0x11_0000, B::LeftToRight),
        ];
        for (code_point, class) in cases {
            assert_eq!(bidi_class(code_point), class, "{code_point:#X}");
        }
        assert_eq!(B::ArabicLetter.short_name(), "AL");
        assert_eq!(B::ArabicLetter.long_name(), "Arabic_Letter");
    }

    #[test]
    fn known_joining_types() {
        use JoiningType as J;
        let cases = [
            (0x0627, J::RightJoining),
            (0x0628, J::DualJoining),
            (0x0640, J::JoinCausing),
            (0x200D, J::JoinCausing),
            (0x200C, J::NonJoining),
            (0x0300, J::Transparent),
            (0x00AD, J::Transparent),
            (0xA872, J::LeftJoining),
            (0x0041, J::NonJoining),
            (0x11_0000, J::NonJoining),
        ];
        for (code_point, joining) in cases {
            assert_eq!(joining_type(code_point), joining, "{code_point:#X}");
        }
        assert_eq!(J::DualJoining.short_name(), "D");
    }

    /// Vergleicht jeden Codepunkt mit `UnicodeData.txt`, gelesen unabhängig
    /// vom Generator.
    #[test]
    fn tables_match_unicode_data() {
        let data = include_str!("../../../data/unicode/UnicodeData.txt");
        let mut expected = vec![("Cn", 0u8); 0x11_0000];
        let mut range_start = None;
        for line in data.lines() {
            let fields: Vec<&str> = line.split(';').collect();
            let code_point = u32::from_str_radix(fields[0], 16).expect("Codepunkt");
            let name = fields[1];
            if name.ends_with(", First>") {
                range_start = Some(code_point);
                continue;
            }
            let first = if name.ends_with(", Last>") {
                range_start.take().expect("Bereichsanfang")
            } else {
                code_point
            };
            let class: u8 = fields[3].parse().expect("Kombinationsklasse");
            for slot in &mut expected[first as usize..=code_point as usize] {
                *slot = (fields[2], class);
            }
        }
        for (code_point, &(category, class)) in (0u32..).zip(&expected) {
            assert_eq!(
                general_category(code_point).short_name(),
                category,
                "{code_point:#X}"
            );
            assert_eq!(
                canonical_combining_class(code_point),
                class,
                "{code_point:#X}"
            );
        }
    }
}
