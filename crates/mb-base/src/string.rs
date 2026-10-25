//! Zeichenketten, wie DOM und JavaScript sie sehen.
//!
//! Rusts `String` und `char` können keine einzelnen Surrogate enthalten,
//! DOM- und JavaScript-Zeichenketten schon (RS-9 im Vault). [`DomString`]
//! ist deshalb eine Folge von UTF-16-Codeeinheiten wie WebIDL `DOMString`,
//! gespeichert kompakt als Latin-1, solange jede Codeeinheit unter 0x100
//! liegt (siehe `docs/architektur.md`, Abschnitt 4.1).

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Ein Codepunkt im Sinn des Infra Standard: ein Unicode-Skalarwert oder ein
/// Surrogat (U+D800 bis U+DFFF). Der Eingabestrom des HTML-Parsers und
/// JavaScript-Zeichenketten bestehen aus solchen Codepunkten.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodePoint(u32);

impl CodePoint {
    /// U+FFFD REPLACEMENT CHARACTER
    pub const REPLACEMENT: CodePoint = CodePoint::from_char(char::REPLACEMENT_CHARACTER);

    pub const fn from_char(c: char) -> CodePoint {
        CodePoint(c as u32)
    }

    /// Der Codepunkt mit diesem Wert, falls er höchstens 0x10FFFF ist.
    pub const fn from_u32(value: u32) -> Option<CodePoint> {
        if value <= 0x10_FFFF {
            Some(CodePoint(value))
        } else {
            None
        }
    }

    pub const fn value(self) -> u32 {
        self.0
    }

    /// Das Zeichen, falls der Codepunkt kein Surrogat ist.
    pub const fn to_char(self) -> Option<char> {
        char::from_u32(self.0)
    }

    pub const fn is_surrogate(self) -> bool {
        matches!(self.0, 0xD800..=0xDFFF)
    }
}

impl From<char> for CodePoint {
    fn from(c: char) -> CodePoint {
        CodePoint::from_char(c)
    }
}

/// Eine Zeichenkette aus UTF-16-Codeeinheiten, die auch einzelne Surrogate
/// enthalten darf.
///
/// Gleichheit, Ordnung und Hash richten sich nur nach den Codeeinheiten,
/// nicht nach der Speicherform.
#[derive(Clone, Default)]
pub struct DomString {
    repr: Repr,
}

#[derive(Clone)]
enum Repr {
    /// Jede Codeeinheit liegt unter 0x100 und steht in einem Byte.
    Latin1(Vec<u8>),
    Utf16(Vec<u16>),
}

impl Default for Repr {
    fn default() -> Repr {
        Repr::Latin1(Vec::new())
    }
}

impl DomString {
    pub const fn new() -> DomString {
        DomString {
            repr: Repr::Latin1(Vec::new()),
        }
    }

    pub fn from_code_units(units: &[u16]) -> DomString {
        let mut string = DomString::new();
        for &unit in units {
            string.push_code_unit(unit);
        }
        string
    }

    /// Die Länge in Codeeinheiten.
    pub fn len(&self) -> usize {
        match &self.repr {
            Repr::Latin1(bytes) => bytes.len(),
            Repr::Utf16(units) => units.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn code_unit(&self, index: usize) -> Option<u16> {
        match &self.repr {
            Repr::Latin1(bytes) => bytes.get(index).copied().map(u16::from),
            Repr::Utf16(units) => units.get(index).copied(),
        }
    }

    pub fn code_units(&self) -> CodeUnits<'_> {
        match &self.repr {
            Repr::Latin1(bytes) => CodeUnits::Latin1(bytes.iter()),
            Repr::Utf16(units) => CodeUnits::Utf16(units.iter()),
        }
    }

    /// Die Codepunkte: Paare aus hohem und tiefem Surrogat werden zu einem
    /// Codepunkt, einzelne Surrogate bleiben, was sie sind.
    pub fn code_points(&self) -> CodePoints<'_> {
        CodePoints {
            units: self.code_units(),
            pending: None,
        }
    }

    pub fn push(&mut self, c: char) {
        let mut buffer = [0; 2];
        for &unit in c.encode_utf16(&mut buffer).iter() {
            self.push_code_unit(unit);
        }
    }

    pub fn push_str(&mut self, text: &str) {
        for c in text.chars() {
            self.push(c);
        }
    }

    /// Hängt einen Codepunkt an. Ein Surrogat wird zu einer Codeeinheit; folgt
    /// ein tiefes auf ein hohes, bilden beide ein Paar.
    pub fn push_code_point(&mut self, code_point: CodePoint) {
        match code_point.to_char() {
            Some(c) => self.push(c),
            None => {
                if let Ok(unit) = u16::try_from(code_point.value()) {
                    self.push_code_unit(unit);
                }
            }
        }
    }

    pub fn push_code_unit(&mut self, unit: u16) {
        match &mut self.repr {
            Repr::Latin1(bytes) => match u8::try_from(unit) {
                Ok(byte) => bytes.push(byte),
                Err(_) => {
                    let mut units: Vec<u16> = bytes.iter().copied().map(u16::from).collect();
                    units.push(unit);
                    self.repr = Repr::Utf16(units);
                }
            },
            Repr::Utf16(units) => units.push(unit),
        }
    }

    pub fn push_dom_string(&mut self, other: &DomString) {
        match (&mut self.repr, &other.repr) {
            (Repr::Latin1(bytes), Repr::Latin1(more)) => bytes.extend_from_slice(more),
            _ => {
                for unit in other.code_units() {
                    self.push_code_unit(unit);
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.repr = Repr::Latin1(Vec::new());
    }

    /// Ob jede Codeeinheit ein ASCII-Zeichen ist.
    pub fn is_ascii(&self) -> bool {
        match &self.repr {
            Repr::Latin1(bytes) => bytes.is_ascii(),
            Repr::Utf16(_) => false,
        }
    }

    /// Die Zeichenkette als `&str`, falls sie nur aus ASCII besteht.
    pub fn as_ascii(&self) -> Option<&str> {
        match &self.repr {
            // Bytes über 0x7F wären Latin-1, nicht UTF-8.
            Repr::Latin1(bytes) if bytes.is_ascii() => std::str::from_utf8(bytes).ok(),
            _ => None,
        }
    }

    /// Als `String`; einzelne Surrogate werden dabei zu U+FFFD. Nur für
    /// Ausgaben gedacht, an denen das nichts verfälscht.
    pub fn to_string_lossy(&self) -> String {
        self.code_points()
            .map(|code_point| code_point.to_char().unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    }

    /// *ASCII lowercase*: nur A bis Z werden zu Kleinbuchstaben.
    pub fn to_ascii_lowercase(&self) -> DomString {
        match &self.repr {
            Repr::Latin1(bytes) => DomString {
                repr: Repr::Latin1(bytes.to_ascii_lowercase()),
            },
            Repr::Utf16(units) => DomString {
                repr: Repr::Utf16(
                    units
                        .iter()
                        .map(|&unit| match u8::try_from(unit) {
                            Ok(byte) => u16::from(byte.to_ascii_lowercase()),
                            Err(_) => unit,
                        })
                        .collect(),
                ),
            },
        }
    }

    /// Gleichheit ohne Unterschied zwischen ASCII-Groß- und -Kleinbuchstaben
    /// (*ASCII case-insensitive match*).
    pub fn eq_ignore_ascii_case(&self, other: &str) -> bool {
        let lower = |unit: u16| match u8::try_from(unit) {
            Ok(byte) => u16::from(byte.to_ascii_lowercase()),
            Err(_) => unit,
        };
        self.code_units()
            .map(lower)
            .eq(other.encode_utf16().map(lower))
    }

    /// Ob die Zeichenkette mit `prefix` beginnt, ohne Unterschied zwischen
    /// ASCII-Groß- und -Kleinbuchstaben.
    pub fn starts_with_ignore_ascii_case(&self, prefix: &str) -> bool {
        let lower = |unit: u16| match u8::try_from(unit) {
            Ok(byte) => u16::from(byte.to_ascii_lowercase()),
            Err(_) => unit,
        };
        let mut units = self.code_units().map(lower);
        prefix
            .encode_utf16()
            .map(lower)
            .all(|expected| units.next() == Some(expected))
    }
}

impl From<&str> for DomString {
    fn from(text: &str) -> DomString {
        let mut string = DomString::new();
        string.push_str(text);
        string
    }
}

impl From<char> for DomString {
    fn from(c: char) -> DomString {
        let mut string = DomString::new();
        string.push(c);
        string
    }
}

impl PartialEq for DomString {
    fn eq(&self, other: &DomString) -> bool {
        match (&self.repr, &other.repr) {
            (Repr::Latin1(a), Repr::Latin1(b)) => a == b,
            (Repr::Utf16(a), Repr::Utf16(b)) => a == b,
            _ => self.len() == other.len() && self.code_units().eq(other.code_units()),
        }
    }
}

impl Eq for DomString {}

impl PartialEq<str> for DomString {
    fn eq(&self, other: &str) -> bool {
        match &self.repr {
            Repr::Latin1(bytes) if other.is_ascii() => bytes.as_slice() == other.as_bytes(),
            _ => self.code_units().eq(other.encode_utf16()),
        }
    }
}

impl PartialEq<&str> for DomString {
    fn eq(&self, other: &&str) -> bool {
        *self == **other
    }
}

impl PartialOrd for DomString {
    fn partial_cmp(&self, other: &DomString) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DomString {
    /// Nach Codeeinheiten, wie der Vergleich von Zeichenketten in JavaScript.
    fn cmp(&self, other: &DomString) -> Ordering {
        self.code_units().cmp(other.code_units())
    }
}

impl Hash for DomString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.len());
        for unit in self.code_units() {
            state.write_u16(unit);
        }
    }
}

impl fmt::Debug for DomString {
    /// Wie bei `str`, einzelne Surrogate erscheinen als `\u{D800}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"")?;
        for code_point in self.code_points() {
            match code_point.to_char() {
                Some(c) => write!(f, "{}", c.escape_debug())?,
                None => write!(f, "\\u{{{:X}}}", code_point.value())?,
            }
        }
        f.write_str("\"")
    }
}

/// Die Codeeinheiten einer [`DomString`].
#[derive(Clone)]
pub enum CodeUnits<'a> {
    Latin1(std::slice::Iter<'a, u8>),
    Utf16(std::slice::Iter<'a, u16>),
}

impl Iterator for CodeUnits<'_> {
    type Item = u16;

    fn next(&mut self) -> Option<u16> {
        match self {
            CodeUnits::Latin1(bytes) => bytes.next().copied().map(u16::from),
            CodeUnits::Utf16(units) => units.next().copied(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            CodeUnits::Latin1(bytes) => bytes.size_hint(),
            CodeUnits::Utf16(units) => units.size_hint(),
        }
    }
}

/// Die Codepunkte einer [`DomString`].
#[derive(Clone)]
pub struct CodePoints<'a> {
    units: CodeUnits<'a>,
    /// Eine schon gelesene Codeeinheit, die nicht zum vorigen Paar gehörte.
    pending: Option<u16>,
}

impl Iterator for CodePoints<'_> {
    type Item = CodePoint;

    fn next(&mut self) -> Option<CodePoint> {
        let unit = self.pending.take().or_else(|| self.units.next())?;
        if (0xD800..0xDC00).contains(&unit) {
            match self.units.next() {
                Some(low @ 0xDC00..0xE000) => {
                    let high = u32::from(unit) - 0xD800;
                    let low = u32::from(low) - 0xDC00;
                    return Some(CodePoint(0x1_0000 + (high << 10) + low));
                }
                other => self.pending = other,
            }
        }
        Some(CodePoint(u32::from(unit)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_switches_to_utf16_when_needed() {
        let mut string = DomString::from("Grüße");
        assert!(matches!(string.repr, Repr::Latin1(_)));
        assert_eq!(string.len(), 5);
        string.push('€');
        assert!(matches!(string.repr, Repr::Utf16(_)));
        string.push('😀');
        assert_eq!(string.len(), 8);
        assert_eq!(string, "Grüße€😀");
        assert_eq!(string.to_string_lossy(), "Grüße€😀");
        assert_eq!(string.code_unit(6), Some(0xD83D));
        assert_eq!(string.code_unit(8), None);
        string.clear();
        assert!(string.is_empty() && matches!(string.repr, Repr::Latin1(_)));
    }

    #[test]
    fn equality_ignores_storage() {
        let latin1 = DomString::from("abc");
        let utf16 = DomString {
            repr: Repr::Utf16(vec![0x61, 0x62, 0x63]),
        };
        assert_eq!(latin1, utf16);
        let hash = |string: &DomString| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            string.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash(&latin1), hash(&utf16));
        assert_eq!(latin1.cmp(&utf16), Ordering::Equal);
        let (short, long) = (DomString::from("ab"), DomString::from("abc"));
        assert!(short < long);
        // Nach Codeeinheiten liegt U+FF61 hinter U+1F600 (0xD83D…).
        let (emoji, halfwidth) = (DomString::from("\u{1F600}"), DomString::from("\u{FF61}"));
        assert!(emoji < halfwidth);
    }

    #[test]
    fn lone_surrogates() {
        let mut string = DomString::from("a");
        string.push_code_point(CodePoint::from_u32(0xDFFF).expect("gültig"));
        string.push('b');
        assert_eq!(string.len(), 3);
        assert_eq!(string.to_string_lossy(), "a\u{FFFD}b");
        assert_eq!(format!("{string:?}"), "\"a\\u{DFFF}b\"");
        let points: Vec<u32> = string.code_points().map(CodePoint::value).collect();
        assert_eq!(points, [0x61, 0xDFFF, 0x62]);
        // Hohes Surrogat vor einem Zeichen, das kein tiefes ist.
        let string = DomString::from_code_units(&[0xD800, 0x41, 0xD800]);
        let points: Vec<u32> = string.code_points().map(CodePoint::value).collect();
        assert_eq!(points, [0xD800, 0x41, 0xD800]);
        // Ein tiefes nach einem hohen bildet ein Paar.
        let mut string = DomString::new();
        string.push_code_point(CodePoint::from_u32(0xD83D).expect("gültig"));
        string.push_code_point(CodePoint::from_u32(0xDE00).expect("gültig"));
        assert_eq!(string, "😀");
        assert!(CodePoint::from_u32(0x11_0000).is_none());
    }

    #[test]
    fn ascii_helpers() {
        let string = DomString::from("Text/HTML");
        assert_eq!(string.as_ascii(), Some("Text/HTML"));
        assert!(string.eq_ignore_ascii_case("text/html"));
        assert!(!string.eq_ignore_ascii_case("text/htm"));
        assert!(string.starts_with_ignore_ascii_case("TEXT/"));
        assert!(!string.starts_with_ignore_ascii_case("text/html+"));
        assert_eq!(string.to_ascii_lowercase(), "text/html");
        // Latin-1 über 0x7F ist kein ASCII, auch wenn die Bytes UTF-8 wären.
        assert_eq!(DomString::from("Ã©").as_ascii(), None);
        assert_eq!(DomString::from("ÄÖ€").to_ascii_lowercase(), "ÄÖ€");
        assert!(!DomString::from("Ä").eq_ignore_ascii_case("ä"));
    }
}
