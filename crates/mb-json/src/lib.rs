//! Ein kleiner JSON-Leser (RFC 8259) für Rohdaten wie `encodings.json`, für
//! Testdaten und später für Manifeste.
//!
//! Die Verschachtelungstiefe ist begrenzt (RS-8), Fehler nennen die Stelle.
//! Einzelne Surrogate in `\u`-Escapes werden zu U+FFFD, wie bei der
//! Umwandlung in einen `USVString` in WebIDL.

#![forbid(unsafe_code)]

/// Ein JSON-Wert. Zahlen bleiben Text, so wie sie dastehen.
#[derive(Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    /// Der Wert zu einem Schlüssel, falls `self` ein Objekt ist. Kommt der
    /// Schlüssel mehrfach vor, gilt der erste.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// Ein Fehler beim Lesen, mit der Stelle in Bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub position: usize,
    pub message: &'static str,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON, Byte {}: {}", self.position, self.message)
    }
}

impl std::error::Error for Error {}

/// Die größte erlaubte Verschachtelungstiefe.
const MAX_DEPTH: usize = 64;

/// Liest einen JSON-Text.
pub fn parse(text: &str) -> Result<Value, Error> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        position: 0,
    };
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.position != parser.bytes.len() {
        return Err(parser.error("Zeichen nach dem Ende des Werts"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn error(&self, message: &'static str) -> Error {
        Error {
            position: self.position,
            message,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    fn starts_with(&self, expected: &[u8]) -> bool {
        self.bytes
            .get(self.position..)
            .is_some_and(|rest| rest.starts_with(expected))
    }

    fn expect(&mut self, expected: &[u8]) -> Result<(), Error> {
        if self.starts_with(expected) {
            self.position += expected.len();
            Ok(())
        } else {
            Err(self.error("unerwartetes Zeichen"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(self.error("zu tief verschachtelt"));
        }
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.expect(b"true").map(|()| Value::Bool(true)),
            Some(b'f') => self.expect(b"false").map(|()| Value::Bool(false)),
            Some(b'n') => self.expect(b"null").map(|()| Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number().map(Value::Number),
            Some(_) => Err(self.error("unerwartetes Zeichen")),
            None => Err(self.error("Text endet zu früh")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, Error> {
        self.expect(b"{")?;
        let mut members = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("Schlüssel erwartet"));
            }
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b":")?;
            let value = self.value(depth + 1)?;
            members.push((key, value));
            self.skip_whitespace();
            match self.next() {
                Some(b',') => {}
                Some(b'}') => return Ok(Value::Object(members)),
                _ => return Err(self.error("`,` oder `}` erwartet")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, Error> {
        self.expect(b"[")?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.next() {
                Some(b',') => {}
                Some(b']') => return Ok(Value::Array(items)),
                _ => return Err(self.error("`,` oder `]` erwartet")),
            }
        }
    }

    fn number(&mut self) -> Result<String, Error> {
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        match self.next() {
            Some(b'0') => {}
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.error("Ziffer erwartet")),
        }
        if self.peek() == Some(b'.') {
            self.position += 1;
            self.digit()?;
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            self.digit()?;
            self.digits();
        }
        let text = self.bytes.get(start..self.position).unwrap_or_default();
        Ok(String::from_utf8_lossy(text).into_owned())
    }

    fn digit(&mut self) -> Result<(), Error> {
        match self.next() {
            Some(b'0'..=b'9') => Ok(()),
            _ => Err(self.error("Ziffer erwartet")),
        }
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
    }

    fn string(&mut self) -> Result<String, Error> {
        self.expect(b"\"")?;
        let mut text = String::new();
        loop {
            let start = self.position;
            while matches!(self.peek(), Some(byte) if byte != b'"' && byte != b'\\' && byte >= 0x20)
            {
                self.position += 1;
            }
            // Die Eingabe ist ein &str, die Grenzen liegen vor ASCII-Zeichen.
            let run = self.bytes.get(start..self.position).unwrap_or_default();
            text.push_str(&String::from_utf8_lossy(run));
            match self.next() {
                Some(b'"') => return Ok(text),
                Some(b'\\') => text.push(self.escape()?),
                Some(_) => return Err(self.error("Steuerzeichen in Zeichenkette")),
                None => return Err(self.error("Zeichenkette ohne Ende")),
            }
        }
    }

    fn escape(&mut self) -> Result<char, Error> {
        Ok(match self.next() {
            Some(b'"') => '"',
            Some(b'\\') => '\\',
            Some(b'/') => '/',
            Some(b'b') => '\u{8}',
            Some(b'f') => '\u{c}',
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'u') => {
                let unit = self.hex4()?;
                if (0xD800..0xDC00).contains(&unit) && self.starts_with(b"\\u") {
                    // Nur ein folgendes niedriges Surrogat gehört dazu, sonst
                    // wird das nächste Escape für sich gelesen.
                    let saved = self.position;
                    self.position += 2;
                    let low = self.hex4()?;
                    if (0xDC00..0xE000).contains(&low) {
                        let pair = char::decode_utf16([unit, low]).next();
                        return Ok(pair.and_then(Result::ok).unwrap_or('\u{FFFD}'));
                    }
                    self.position = saved;
                }
                char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}')
            }
            _ => return Err(self.error("ungültige Escape-Sequenz")),
        })
    }

    fn hex4(&mut self) -> Result<u16, Error> {
        let digits = self
            .bytes
            .get(self.position..self.position + 4)
            .filter(|digits| digits.iter().all(u8::is_ascii_hexdigit))
            .ok_or_else(|| self.error("vier Hexziffern erwartet"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.error("Hexziffern"))?;
        let value = u16::from_str_radix(text, 16).map_err(|_| self.error("Hexziffern"))?;
        self.position += 4;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_values() {
        let value =
            parse(r#" { "a": [1, -2.5e3, true, false, null], "b": "x\"\u00e4\ud83d\ude00\n" } "#)
                .expect("gültiges JSON");
        assert_eq!(
            value.get("a").and_then(Value::as_array),
            Some(
                &[
                    Value::Number("1".into()),
                    Value::Number("-2.5e3".into()),
                    Value::Bool(true),
                    Value::Bool(false),
                    Value::Null
                ][..]
            )
        );
        assert_eq!(value.get("b").and_then(Value::as_str), Some("x\"ä😀\n"));
        assert_eq!(value.get("c"), None);
        assert_eq!(parse("[]"), Ok(Value::Array(Vec::new())));
        assert_eq!(parse("{}"), Ok(Value::Object(Vec::new())));
        assert_eq!(parse("\"äöü\""), Ok(Value::String("äöü".into())));
    }

    #[test]
    fn lone_surrogates_become_replacement_characters() {
        let string = |text: &str| {
            parse(text)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
        };
        assert_eq!(string(r#""\ud800""#).as_deref(), Some("\u{FFFD}"));
        assert_eq!(string(r#""\udc00a""#).as_deref(), Some("\u{FFFD}a"));
        assert_eq!(string(r#""\ud800\u0041""#).as_deref(), Some("\u{FFFD}A"));
        assert_eq!(
            string(r#""\ud800\ud83d\ude00""#).as_deref(),
            Some("\u{FFFD}😀")
        );
    }

    #[test]
    fn rejects_broken_json() {
        for text in [
            "",
            "[",
            "[1,]",
            "{\"a\" 1}",
            "{a: 1}",
            "01",
            "1.",
            "-",
            "\"a",
            "\"\\x\"",
            "\"\\ud800\\u12\"",
            "\"\t\"",
            "[1] 2",
            "tru",
            "\"\\u12\"",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
        let deep = "[".repeat(100) + &"]".repeat(100);
        assert!(parse(&deep).is_err());
        assert_eq!(
            parse("[1,]").map_err(|error| error.to_string()),
            Err("JSON, Byte 3: unerwartetes Zeichen".to_owned())
        );
    }
}
