//! Aus der Unicode-Zeichendatenbank generierte Tabellen und die Unicode-Algorithmen (Normalisierung, Bidi, Zeilenumbruch, Segmentierung, IDNA).
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.3.
//!
//! Die Tabellen in `tables.rs` und `idna/tables.rs` erzeugt
//! `cargo run -p datagen` aus den Rohdaten unter `data/unicode/`. Codepunkte sind hier `u32` statt `char`,
//! damit auch einzelne Surrogate aus JavaScript-Zeichenketten abgefragt
//! werden können (RS-9).

#![forbid(unsafe_code)]

pub mod case_folding;
pub mod idna;
pub mod line_breaking;
pub mod normalization;
#[rustfmt::skip]
mod tables;

pub use tables::{
    BidiClass, EastAsianWidth, GeneralCategory, JoiningType, LineBreak, UNICODE_VERSION,
};

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

/// Die Zeilenumbruchklasse eines Codepunkts, wie sie in `LineBreak.txt`
/// steht, also vor der Auflösung nach LB1 (siehe [`line_breaking`]). Nicht
/// vergebene Codepunkte haben die Vorgaben der Datei, Werte über U+10FFFF
/// gelten als `XX`.
pub fn line_break(code_point: u32) -> LineBreak {
    lookup(&tables::LINE_BREAK, code_point).unwrap_or(LineBreak::Unknown)
}

/// Die East_Asian_Width eines Codepunkts. Nicht vergebene Codepunkte haben
/// die Vorgaben aus `EastAsianWidth.txt`, Werte über U+10FFFF gelten als `N`.
pub fn east_asian_width(code_point: u32) -> EastAsianWidth {
    lookup(&tables::EAST_ASIAN_WIDTH, code_point).unwrap_or(EastAsianWidth::Neutral)
}

/// Ob ein Codepunkt Extended_Pictographic ist (UTS #51). Das gilt auch für
/// manche noch nicht vergebene Codepunkte.
pub fn is_extended_pictographic(code_point: u32) -> bool {
    lookup(&tables::EXTENDED_PICTOGRAPHIC, code_point).unwrap_or(false)
}

/// Ob ein Codepunkt ein Bezeichner beginnen kann (ID_Start, UAX #31).
/// ECMAScript ergänzt für Bezeichner `$` und `_` selbst.
pub fn is_id_start(code_point: u32) -> bool {
    lookup(&tables::ID_START, code_point).unwrap_or(false)
}

/// Ob ein Codepunkt in einem Bezeichner nach dem ersten Zeichen stehen kann
/// (ID_Continue, UAX #31). Jeder Codepunkt mit ID_Start hat auch diese
/// Eigenschaft.
pub fn is_id_continue(code_point: u32) -> bool {
    lookup(&tables::ID_CONTINUE, code_point).unwrap_or(false)
}

/// Der Kurzname des Werts von General_Category zu einem seiner Namen aus
/// `PropertyValueAliases.txt`, auch für die Gruppen: `Lu` für
/// `Uppercase_Letter`, `L` für `Letter`, `Cc` für `cntrl`. Verglichen wird
/// genau, ohne die losen Regeln aus UAX #44, so wie JavaScript es für
/// `\p{…}` verlangt.
pub fn general_category_value(name: &str) -> Option<&'static str> {
    value(&tables::GENERAL_CATEGORY_NAMES, name)
}

/// Der Kurzname des Werts von Script zu einem seiner Namen, verglichen wie
/// bei [`general_category_value`]: `Latn` für `Latin`, `Zinh` für `Qaai`.
pub fn script_value(name: &str) -> Option<&'static str> {
    value(&tables::SCRIPT_NAMES, name)
}

/// Sucht einen Namen in einer nach Namen sortierten Tabelle.
fn value(table: &[(&str, &'static str)], name: &str) -> Option<&'static str> {
    let index = table.binary_search_by(|&(entry, _)| entry.cmp(name)).ok()?;
    table.get(index).map(|&(_, short)| short)
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
    fn value_names() {
        let cases = [
            ("Lu", Some("Lu")),
            ("Uppercase_Letter", Some("Lu")),
            ("L", Some("L")),
            ("Letter", Some("L")),
            ("LC", Some("LC")),
            ("Cased_Letter", Some("LC")),
            ("cntrl", Some("Cc")),
            ("digit", Some("Nd")),
            ("punct", Some("P")),
            ("Combining_Mark", Some("M")),
            ("uppercase_letter", None),
            ("Uppercase Letter", None),
            ("L&", None),
            ("", None),
        ];
        for (name, short) in cases {
            assert_eq!(general_category_value(name), short, "{name}");
        }
        let cases = [
            ("Latin", Some("Latn")),
            ("Latn", Some("Latn")),
            ("Zinh", Some("Zinh")),
            ("Inherited", Some("Zinh")),
            ("Qaai", Some("Zinh")),
            ("Qaac", Some("Copt")),
            ("Zzzz", Some("Zzzz")),
            ("Unknown", Some("Zzzz")),
            ("latin", None),
            ("Lu", None),
        ];
        for (name, short) in cases {
            assert_eq!(script_value(name), short, "{name}");
        }
        for table in [
            &tables::GENERAL_CATEGORY_NAMES[..],
            &tables::SCRIPT_NAMES[..],
        ] {
            assert!(table.windows(2).all(|pair| pair[0].0 < pair[1].0));
        }
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

    #[test]
    fn known_line_break_classes() {
        use LineBreak as L;
        let cases = [
            (0x0009, L::BreakAfter),
            (0x000A, L::LineFeed),
            (0x0020, L::Space),
            (0x0021, L::Exclamation),
            (0x0022, L::Quotation),
            (0x0028, L::OpenPunctuation),
            (0x0029, L::CloseParenthesis),
            (0x002D, L::Hyphen),
            (0x0030, L::Numeric),
            (0x0041, L::Alphabetic),
            (0x00A0, L::Glue),
            (0x00A7, L::Ambiguous),
            (0x05D0, L::HebrewLetter),
            (0x0E01, L::ComplexContext),
            (0x200B, L::ZWSpace),
            (0x200D, L::ZWJ),
            (0x2060, L::WordJoiner),
            (0x3041, L::ConditionalJapaneseStarter),
            (0x4E00, L::Ideographic),
            (0xAC00, L::H2),
            (0xD800, L::Surrogate),
            (0x1F1E6, L::RegionalIndicator),
            (0x1F466, L::EBase),
            (0x1F3FB, L::EModifier),
            // Vorgaben für nicht vergebene Codepunkte.
            (0x0378, L::Unknown),
            (0x20CF, L::PrefixNumeric),
            (0x2FFFD, L::Ideographic),
            (0x11_0000, L::Unknown),
        ];
        for (code_point, class) in cases {
            assert_eq!(line_break(code_point), class, "{code_point:#X}");
        }
        assert_eq!(L::ZWSpace.short_name(), "ZW");
        assert_eq!(L::CloseParenthesis.long_name(), "Close_Parenthesis");
    }

    #[test]
    fn known_east_asian_widths() {
        use EastAsianWidth as E;
        let cases = [
            (0x0000, E::Neutral),
            (0x0041, E::Narrow),
            (0x00A1, E::Ambiguous),
            (0x1100, E::Wide),
            (0x20A9, E::Halfwidth),
            (0x3000, E::Fullwidth),
            (0x4E00, E::Wide),
            (0xFF61, E::Halfwidth),
            (0x1F600, E::Wide),
            // Vorgaben für nicht vergebene Codepunkte.
            (0x0378, E::Neutral),
            (0x3FFFD, E::Wide),
            (0x11_0000, E::Neutral),
        ];
        for (code_point, width) in cases {
            assert_eq!(east_asian_width(code_point), width, "{code_point:#X}");
        }
    }

    #[test]
    fn known_extended_pictographic() {
        assert!(is_extended_pictographic(0x00A9));
        assert!(is_extended_pictographic(0x1F600));
        assert!(!is_extended_pictographic(0x0023));
        assert!(!is_extended_pictographic(0x0041));
        assert!(!is_extended_pictographic(0x1F1E6));
        assert!(!is_extended_pictographic(0x11_0000));
    }

    #[test]
    fn known_identifier_properties() {
        // (Codepunkt, ID_Start, ID_Continue)
        let cases = [
            (0x0024, false, false),
            (0x0030, false, true),
            (0x0041, true, true),
            (0x005F, false, true),
            // Other_ID_Continue.
            (0x00B7, false, true),
            (0x0387, false, true),
            (0x1369, false, true),
            (0x19DA, false, true),
            // ZWNJ, ZWJ und KATAKANA MIDDLE DOT seit Unicode 15.1.
            (0x200C, false, true),
            (0x200D, false, true),
            (0x30FB, false, true),
            // Other_ID_Start.
            (0x1885, true, true),
            (0x2118, true, true),
            (0x212E, true, true),
            (0x309B, true, true),
            // Lm, aber Pattern_Syntax.
            (0x2E2F, false, false),
            (0x0345, false, true),
            (0xFF10, false, true),
            (0xE0100, false, true),
            (0x1F600, false, false),
            (0x11_0000, false, false),
        ];
        for (code_point, start, part) in cases {
            assert_eq!(is_id_start(code_point), start, "{code_point:#X}");
            assert_eq!(is_id_continue(code_point), part, "{code_point:#X}");
        }
    }

    /// Liest eine Eigenschaftsdatei unabhängig vom Generator: `default` für
    /// alle Codepunkte, darüber die Werte aus den Datenzeilen.
    fn property_file(text: &str, default: &'static str) -> Vec<String> {
        let mut values = vec![default.to_owned(); 0x11_0000];
        for line in text.lines() {
            let data = line.split('#').next().unwrap_or_default();
            if data.trim().is_empty() {
                continue;
            }
            let (range, value) = data.split_once(';').expect("zwei Felder");
            let (first, last) = range
                .trim()
                .split_once("..")
                .unwrap_or((range.trim(), range.trim()));
            let first = usize::from_str_radix(first, 16).expect("Codepunkt");
            let last = usize::from_str_radix(last, 16).expect("Codepunkt");
            for slot in &mut values[first..=last] {
                value.trim().clone_into(slot);
            }
        }
        values
    }

    /// Vergleicht jeden Codepunkt mit `LineBreak.txt`, `EastAsianWidth.txt`,
    /// `emoji-data.txt` und `DerivedCoreProperties.txt`, gelesen unabhängig
    /// vom Generator. `LineBreak.txt` und `EastAsianWidth.txt` haben nur eine
    /// `@missing`-Zeile für den ganzen Coderaum, die übrigen Vorgaben stehen
    /// als Datenzeilen darin.
    #[test]
    fn tables_match_property_files() {
        let line_breaks = include_str!("../../../data/unicode/LineBreak.txt");
        assert!(line_breaks.contains("\n# @missing: 0000..10FFFF; XX\n"));
        assert_eq!(line_breaks.matches("@missing").count(), 1);
        let widths = include_str!("../../../data/unicode/EastAsianWidth.txt");
        assert!(widths.contains("\n# @missing: 0000..10FFFF; N\n"));
        assert_eq!(widths.matches("@missing").count(), 1);
        let line_breaks = property_file(line_breaks, "XX");
        let widths = property_file(widths, "N");
        let emoji = include_str!("../../../data/unicode/emoji-data.txt");
        let pictographic: String = emoji
            .lines()
            .filter(|line| line.contains("; Extended_Pictographic"))
            .map(|line| format!("{line}\n"))
            .collect();
        let pictographic = property_file(&pictographic, "");
        let derived = include_str!("../../../data/unicode/DerivedCoreProperties.txt");
        let only = |property: &str| -> String {
            derived
                .lines()
                .filter(|line| line.contains(&format!("; {property} ")))
                .map(|line| format!("{line}\n"))
                .collect()
        };
        let id_start = property_file(&only("ID_Start"), "");
        let id_continue = property_file(&only("ID_Continue"), "");
        for code_point in 0..=MAX_CODE_POINT {
            let index = code_point as usize;
            assert_eq!(
                line_break(code_point).short_name(),
                line_breaks[index],
                "{code_point:#X}"
            );
            assert_eq!(
                east_asian_width(code_point).short_name(),
                widths[index],
                "{code_point:#X}"
            );
            assert_eq!(
                is_extended_pictographic(code_point),
                !pictographic[index].is_empty(),
                "{code_point:#X}"
            );
            assert_eq!(
                is_id_start(code_point),
                !id_start[index].is_empty(),
                "{code_point:#X}"
            );
            assert_eq!(
                is_id_continue(code_point),
                !id_continue[index].is_empty(),
                "{code_point:#X}"
            );
        }
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
