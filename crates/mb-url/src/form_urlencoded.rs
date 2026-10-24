//! Das Format `application/x-www-form-urlencoded` (URL Standard,
//! Abschnitt 5), etwa für `URLSearchParams` und Formulare.

use mb_encoding::{Encoding, utf8_decode_without_bom};

use crate::percent::{FORM_URLENCODED, percent_decode, percent_encode_after_encoding};

/// Der Parser (`#concept-urlencoded-parser`): Name-Wert-Paare aus Bytes.
pub fn parse(input: &[u8]) -> Vec<(String, String)> {
    let mut output = Vec::new();
    for bytes in input.split(|&byte| byte == b'&') {
        if bytes.is_empty() {
            continue;
        }
        let (name, value) = match bytes.iter().position(|&byte| byte == b'=') {
            Some(position) => (
                bytes.get(..position).unwrap_or_default(),
                bytes.get(position + 1..).unwrap_or_default(),
            ),
            None => (bytes, &[][..]),
        };
        let decode = |part: &[u8]| {
            let replaced: Vec<u8> = part
                .iter()
                .map(|&byte| if byte == b'+' { b' ' } else { byte })
                .collect();
            utf8_decode_without_bom(&percent_decode(&replaced))
        };
        output.push((decode(name), decode(value)));
    }
    output
}

/// Der Parser für Zeichenketten (`#concept-urlencoded-string-parser`).
pub fn parse_str(input: &str) -> Vec<(String, String)> {
    parse(input.as_bytes())
}

/// Der Serialisierer (`#concept-urlencoded-serializer`).
pub fn serialize<'a>(
    tuples: impl IntoIterator<Item = (&'a str, &'a str)>,
    encoding: Encoding,
) -> String {
    let encoding = encoding.output_encoding();
    let mut output = String::new();
    for (name, value) in tuples {
        if !output.is_empty() {
            output.push('&');
        }
        output.push_str(&percent_encode_after_encoding(
            encoding,
            name,
            FORM_URLENCODED,
        ));
        output.push('=');
        output.push_str(&percent_encode_after_encoding(
            encoding,
            value,
            FORM_URLENCODED,
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let pairs = parse(b"a=b&&=c&d&e=f=g&h+i=%26%zz&%C3%BC=%FF");
        let expected = [
            ("a", "b"),
            ("", "c"),
            ("d", ""),
            ("e", "f=g"),
            ("h i", "&%zz"),
            ("ü", "\u{FFFD}"),
        ];
        assert_eq!(pairs.len(), expected.len());
        for ((name, value), (expected_name, expected_value)) in pairs.iter().zip(expected) {
            assert_eq!(
                (name.as_str(), value.as_str()),
                (expected_name, expected_value)
            );
        }
        let serialized = serialize(
            pairs
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
            Encoding::Utf8,
        );
        assert_eq!(
            serialized,
            "a=b&=c&d=&e=f%3Dg&h+i=%26%25zz&%C3%BC=%EF%BF%BD"
        );
    }
}
