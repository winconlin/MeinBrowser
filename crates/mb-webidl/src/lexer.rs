//! Web IDL, Anhang „IDL grammar“: längster Treffer auf Unicode-Skalaren.
//! Quellstand: whatwg/webidl 296213a1d9c8293850295c6e77d3b13dce8681a9.
//! Tokenwerte bleiben Scheiben der UTF-8-Quelle; Zahlen werden nicht gerundet.

use std::iter::FusedIterator;
use std::ops::Range;

/// Benannte lexikalische Terminals sowie die festen Grammatikterminals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Integer,
    Decimal,
    Identifier,
    String,
    Whitespace,
    Comment,
    /// Reserviertes Wort oder festes Satzzeichen, einschließlich `...`.
    Terminal,
    /// Genau ein sonstiger Unicode-Skalar. Kein Nachweis gültiger IDL.
    Other,
    End,
}

impl Kind {
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Comment)
    }
}

/// Ein unveränderter Ausschnitt der Quelle; `span` zählt UTF-8-Bytes.
/// Ein String enthält seine Anführungszeichen, Trivia seine Originalbytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: Kind,
    pub span: Range<usize>,
    pub text: &'a str,
}

/// Ein vorwärts laufender Tokenizer mit konstantem Zusatzspeicher.
/// Kommentare und Leerraum werden geliefert, damit spätere Werkzeuge die
/// Quelle vollständig zuordnen können. Ein Parser darf Trivia überspringen.
pub struct Lexer<'a> {
    source: &'a str,
    position: usize,
    /// Nach erfolgloser Suche kann kein späterer Blockkommentar schließen.
    /// Verhindert wiederholte Suche bei vielen offenen Kommentar-Anfängen.
    no_block_end: bool,
}

impl<'a> Lexer<'a> {
    pub const fn new(source: &'a str) -> Self {
        Self {
            source,
            position: 0,
            no_block_end: false,
        }
    }

    /// Liefert auch nach dem Ende immer `End` am Ende der Quelle.
    /// Nicht geschlossene Strings/Blockkommentare passen nicht auf deren
    /// Regex und fallen wie in der Grammatik auf `other` zurück. Ob der
    /// Tokenstrom gültige IDL ist, entscheidet erst der spätere Parser.
    pub fn next_token(&mut self) -> Token<'a> {
        let start = self.position;
        let rest = &self.source[start..];
        if rest.is_empty() {
            return self.finish(Kind::End, 0);
        }
        let bytes = rest.as_bytes();
        if whitespace(bytes[0]) {
            let end = bytes.iter().take_while(|&&byte| whitespace(byte)).count();
            return self.finish(Kind::Whitespace, end);
        }
        if rest.starts_with("//") {
            // Perl 5 `.` ohne /s schließt nur LF aus, nicht CR oder U+2028.
            let end = rest.find('\n').unwrap_or(rest.len());
            return self.finish(Kind::Comment, end);
        }
        if rest.starts_with("/*") && !self.no_block_end {
            if let Some(end) = rest[2..].find("*/") {
                return self.finish(Kind::Comment, end + 4);
            }
            self.no_block_end = true;
        }
        if bytes[0] == b'"'
            && let Some(end) = rest[1..].find('"')
        {
            // WebIDLs String-Regel kennt keine Backslash-Escapes und
            // erlaubt auch Zeilenumbrüche zwischen den Anführungszeichen.
            return self.finish(Kind::String, end + 2);
        }
        if matches!(bytes[0], b'0'..=b'9' | b'-' | b'.')
            && let Some((kind, length)) = number(bytes)
        {
            return self.finish(kind, length);
        }
        let prefix = usize::from(matches!(bytes[0], b'_' | b'-'));
        if bytes.get(prefix).is_some_and(u8::is_ascii_alphabetic) {
            let mut end = prefix + 1;
            while bytes
                .get(end)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                end += 1;
            }
            let kind = if KEYWORDS.binary_search(&&rest[..end]).is_ok() {
                Kind::Terminal
            } else {
                Kind::Identifier
            };
            return self.finish(kind, end);
        }
        if rest.starts_with("...") {
            return self.finish(Kind::Terminal, 3);
        }
        let scalar = rest.chars().next().expect("nicht leere Quelle");
        let kind = if matches!(
            scalar,
            '(' | ')'
                | '*'
                | ','
                | '-'
                | '.'
                | ':'
                | ';'
                | '<'
                | '='
                | '>'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
        ) {
            Kind::Terminal
        } else {
            Kind::Other
        };
        self.finish(kind, scalar.len_utf8())
    }

    fn finish(&mut self, kind: Kind, length: usize) -> Token<'a> {
        let start = self.position;
        self.position += length;
        Token {
            kind,
            span: start..self.position,
            text: &self.source[start..self.position],
        }
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        (self.position < self.source.len()).then(|| self.next_token())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.source.len() - self.position))
    }
}

impl FusedIterator for Lexer<'_> {}

const fn whitespace(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | b'\r' | b' ')
}

fn digits(bytes: &[u8], mut at: usize) -> usize {
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    at
}

fn exponent(bytes: &[u8], at: usize) -> Option<usize> {
    if !matches!(bytes.get(at), Some(b'E' | b'e')) {
        return None;
    }
    let sign = usize::from(matches!(bytes.get(at + 1), Some(b'+' | b'-')));
    let start = at + 1 + sign;
    let end = digits(bytes, start);
    (end > start).then_some(end)
}

/// Beide Zahlenregeln werden berücksichtigt, dann der längste Treffer.
/// So ist `08` zwei Integer-Tokens, `08.5` aber ein Decimal-Token.
fn number(bytes: &[u8]) -> Option<(Kind, usize)> {
    let start = usize::from(bytes.first() == Some(&b'-'));
    let digit_end = digits(bytes, start);
    let integer_end = match bytes.get(start) {
        Some(b'1'..=b'9') => Some(digit_end),
        Some(b'0') => {
            if matches!(bytes.get(start + 1), Some(b'X' | b'x')) {
                let mut end = start + 2;
                while bytes.get(end).is_some_and(u8::is_ascii_hexdigit) {
                    end += 1;
                }
                Some(if end > start + 2 { end } else { start + 1 })
            } else {
                let mut end = start + 1;
                while matches!(bytes.get(end), Some(b'0'..=b'7')) {
                    end += 1;
                }
                Some(end)
            }
        }
        _ => None,
    };
    let decimal_end = if bytes.get(digit_end) == Some(&b'.') {
        let end = digits(bytes, digit_end + 1);
        if digit_end > start || end > digit_end + 1 {
            Some(exponent(bytes, end).unwrap_or(end))
        } else {
            None
        }
    } else if digit_end > start {
        exponent(bytes, digit_end)
    } else {
        None
    };
    match (integer_end, decimal_end) {
        (Some(integer), Some(decimal)) if decimal > integer => Some((Kind::Decimal, decimal)),
        (Some(integer), _) => Some((Kind::Integer, integer)),
        (None, Some(decimal)) => Some((Kind::Decimal, decimal)),
        _ => None,
    }
}

// Feste Grammatikterminals des oben genannten Standards, case-sensitive
// und sortiert. Extended-Attribute-Namen sind gewöhnliche Identifier.
const KEYWORDS: &[&str] = &[
    "-Infinity",
    "ArrayBuffer",
    "BigInt64Array",
    "BigUint64Array",
    "ByteString",
    "DOMString",
    "DataView",
    "Float16Array",
    "Float32Array",
    "Float64Array",
    "FrozenArray",
    "Infinity",
    "Int16Array",
    "Int32Array",
    "Int8Array",
    "NaN",
    "ObservableArray",
    "Promise",
    "SharedArrayBuffer",
    "USVString",
    "Uint16Array",
    "Uint32Array",
    "Uint8Array",
    "Uint8ClampedArray",
    "any",
    "async_iterable",
    "async_sequence",
    "attribute",
    "bigint",
    "boolean",
    "byte",
    "callback",
    "const",
    "constructor",
    "deleter",
    "dictionary",
    "double",
    "enum",
    "false",
    "float",
    "getter",
    "includes",
    "inherit",
    "interface",
    "iterable",
    "long",
    "maplike",
    "mixin",
    "namespace",
    "null",
    "object",
    "octet",
    "optional",
    "or",
    "partial",
    "readonly",
    "record",
    "required",
    "sequence",
    "setlike",
    "setter",
    "short",
    "static",
    "stringifier",
    "symbol",
    "true",
    "typedef",
    "undefined",
    "unrestricted",
    "unsigned",
];

#[cfg(test)]
mod tests;

#[cfg(test)]
mod webref;
