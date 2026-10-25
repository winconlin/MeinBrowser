//! CSS-Parser nach CSS Syntax Level 3 und die Grammatiken der Eigenschaften.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.4. Grundlage ist der Editor's
//! Draft von CSS Syntax Level 3 (Stand 23.10.2026). Bisher gibt es das
//! Dekodieren von Stylesheets, den Tokenizer, den Parser mit allen
//! Einstiegen und das An+B-Format. Es fehlen noch das Serialisieren und die
//! Grammatiken der Eigenschaften. Parse-Fehler ändern das Ergebnis nicht und
//! werden nicht gemeldet.

#![forbid(unsafe_code)]

mod anb;
mod parser;
mod tokenizer;

#[cfg(test)]
mod tests;

pub use anb::{AnPlusB, parse_an_plus_b};
pub use parser::{
    AtRule, BlockKind, ComponentValue, ComponentValues, Context, Declaration, Iter, QualifiedRule,
    Rule, RuleId, RuleTree, Syntax, SyntaxError, Values,
    parse_comma_separated_component_value_list, parse_component_value, parse_component_value_list,
    parse_declaration, parse_declaration_list, parse_rule, parse_stylesheet,
};
pub use tokenizer::{Sign, Token};

use mb_encoding::Encoding;

/// Die Eingabe der Einstiege: Text nach *filter code points*, also ohne CR,
/// FF, NUL und Surrogate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    chars: Vec<char>,
}

impl Source {
    pub fn new(text: &str) -> Source {
        Source {
            chars: filter_code_points(text.chars()),
        }
    }

    /// Aus UTF-16-Code-Units, etwa einem DOMString. Einzelne Surrogate
    /// werden dabei zu U+FFFD, wie das Filtern es verlangt.
    pub fn from_utf16(units: &[u16]) -> Source {
        let chars = char::decode_utf16(units.iter().copied())
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER));
        Source {
            chars: filter_code_points(chars),
        }
    }

    pub(crate) fn chars(&self) -> &[char] {
        &self.chars
    }
}

/// *Filter code points*: CR, FF und CR LF werden zu LF, NUL wird zu U+FFFD.
/// Surrogate kann ein `char` nicht enthalten.
fn filter_code_points(input: impl Iterator<Item = char>) -> Vec<char> {
    let mut output = Vec::new();
    let mut input = input.peekable();
    while let Some(c) = input.next() {
        match c {
            '\r' => {
                input.next_if_eq(&'\n');
                output.push('\n');
            }
            '\u{C}' => output.push('\n'),
            '\0' => output.push(char::REPLACEMENT_CHARACTER),
            _ => output.push(c),
        }
    }
    output
}

/// *Decode* der Bytes eines Stylesheets („The input byte stream“), mit der
/// Bezeichnung aus dem Protokoll (etwa `charset` in `Content-Type`) und der
/// Kodierung des verweisenden Dokuments. Liefert den Text und die verwendete
/// Kodierung.
pub fn decode_stylesheet(
    bytes: &[u8],
    protocol_encoding: Option<&[u8]>,
    environment_encoding: Option<Encoding>,
) -> (String, Encoding) {
    let fallback = fallback_encoding(bytes, protocol_encoding, environment_encoding);
    mb_encoding::decode(bytes, fallback)
}

/// *Determine the fallback encoding*
fn fallback_encoding(
    bytes: &[u8],
    protocol_encoding: Option<&[u8]>,
    environment_encoding: Option<Encoding>,
) -> Encoding {
    if let Some(encoding) = protocol_encoding.and_then(Encoding::for_label) {
        return encoding;
    }
    if let Some(label) = charset_label(bytes) {
        match Encoding::for_label(label) {
            Some(Encoding::Utf16Be | Encoding::Utf16Le) => return Encoding::Utf8,
            Some(encoding) => return encoding,
            None => {}
        }
    }
    environment_encoding.unwrap_or(Encoding::Utf8)
}

/// Die Bezeichnung aus `@charset "…";`, wenn die ersten 1024 Bytes genau so
/// beginnen. Dazwischen sind alle ASCII-Bytes außer `"` erlaubt.
fn charset_label(bytes: &[u8]) -> Option<&[u8]> {
    let head = bytes.get(..1024).unwrap_or(bytes);
    let rest = head.strip_prefix(b"@charset \"")?;
    let end = rest.iter().position(|&byte| byte == b'"')?;
    let (label, tail) = rest.split_at_checked(end)?;
    (label.is_ascii() && tail.starts_with(b"\";")).then_some(label)
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn filters_code_points() {
        let chars = |source: Source| source.chars().iter().collect::<String>();
        assert_eq!(
            chars(Source::new("a\r\nb\rc\u{C}d\0")),
            "a\nb\nc\nd\u{FFFD}"
        );
        let units = [0x61, 0xD800, 0x62, 0xDC00, 0xD83D, 0xDE00];
        assert_eq!(chars(Source::from_utf16(&units)), "a\u{FFFD}b\u{FFFD}😀");
    }

    #[test]
    fn charset_needs_the_exact_bytes() {
        assert_eq!(charset_label(b"@charset \"x\"; a"), Some(&b"x"[..]));
        assert_eq!(charset_label(b"@charset \"x\" ;"), None);
        assert_eq!(charset_label(b"@charset \"\xE9\";"), None);
        let mut long = b"@charset \"".to_vec();
        long.extend(std::iter::repeat_n(b'a', 1020));
        long.extend(b"\";");
        assert_eq!(charset_label(&long), None);
    }
}
