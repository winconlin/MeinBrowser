//! Ein kleiner JSON-Leser (RFC 8259) für Rohdaten wie `encodings.json`, für
//! Testdaten und später für Manifeste.
//!
//! Den Text zerlegt [`tree`], derselbe Zerleger, den `JSON.parse` der
//! JavaScript-Engine nutzt (architektur.md 4.4). Er kommt ohne Rekursion
//! aus; die Werte hier sind aber verschachtelt, darum ist die Tiefe
//! begrenzt (RS-8). Fehler nennen die Stelle in Bytes. Einzelne Surrogate
//! in `\u`-Escapes werden zu U+FFFD, wie bei der Umwandlung in einen
//! `USVString` in WebIDL.

#![forbid(unsafe_code)]

pub mod tree;

use tree::{Limits, Node, Rejected, Tree};

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

/// Liest einen JSON-Text mit dem Zerleger aus [`tree`], den auch
/// `JSON.parse` der JavaScript-Engine nutzt.
pub fn parse(text: &str) -> Result<Value, Error> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let limits = Limits {
        nodes: usize::MAX,
        depth: MAX_DEPTH,
    };
    let tree = tree::parse(&units, limits).map_err(|rejected| {
        let (position, message) = match rejected {
            Rejected::Invalid { position, message } => (position, message),
            Rejected::TooDeep { position } => (position, "zu tief verschachtelt"),
            Rejected::TooLarge => (units.len(), "zu viele Werte"),
        };
        Error {
            position: byte_position(text, position),
            message,
        }
    })?;
    Ok(build(&tree, &units))
}

/// Die Stelle in Bytes zur Stelle `unit` in UTF-16-Codeeinheiten.
fn byte_position(text: &str, unit: usize) -> usize {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        if units >= unit {
            return index;
        }
        units += character.len_utf16();
    }
    text.len()
}

/// Baut die Werte aus dem Baum, von hinten nach vorn: Die Kinder eines
/// Knotens stehen hinter ihm und sind also schon fertig. Einzelne Surrogate
/// werden zu U+FFFD, wie bei der Umwandlung in einen `USVString`.
fn build(tree: &Tree, units: &[u16]) -> Value {
    let mut values: Vec<Option<Value>> = Vec::new();
    values.resize_with(tree.nodes.len(), || None);
    for (index, node) in tree.nodes.iter().enumerate().rev() {
        let value = match node {
            Node::Null => Value::Null,
            Node::Boolean(boolean) => Value::Bool(*boolean),
            Node::Number(_) => {
                let (start, end) = tree.ranges.get(index).copied().unwrap_or_default();
                let text = units.get(start..end).unwrap_or_default();
                Value::Number(String::from_utf16_lossy(text))
            }
            Node::String(text) => Value::String(String::from_utf16_lossy(text)),
            Node::Array(elements) => Value::Array(
                elements
                    .iter()
                    .map(|&child| take(&mut values, child))
                    .collect(),
            ),
            Node::Object(members) => Value::Object(
                members
                    .iter()
                    .map(|(key, child)| (String::from_utf16_lossy(key), take(&mut values, *child)))
                    .collect(),
            ),
        };
        if let Some(slot) = values.get_mut(index) {
            *slot = Some(value);
        }
    }
    take(&mut values, 0)
}

/// Nimmt den fertigen Wert des Knotens `index` heraus.
fn take(values: &mut [Option<Value>], index: usize) -> Value {
    values
        .get_mut(index)
        .and_then(Option::take)
        .unwrap_or(Value::Null)
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
    fn depth_is_limited_and_positions_are_bytes() {
        let deepest = "[".repeat(65) + &"]".repeat(65);
        assert!(parse(&deepest).is_ok());
        let deeper = "[".repeat(66) + &"]".repeat(66);
        assert_eq!(
            parse(&deeper).map_err(|error| (error.position, error.message)),
            Err((65, "zu tief verschachtelt"))
        );
        assert_eq!(parse("[\"ä\", ]").map_err(|error| error.position), Err(7));
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
