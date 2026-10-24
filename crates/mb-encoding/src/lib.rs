//! Zeichenkodierungen nach dem WHATWG Encoding Standard.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.4. Alle 40 Kodierungen des
//! Standards mit Dekodierer, alle außer *replacement*, UTF-16BE und UTF-16LE
//! auch mit Kodierer. Die Tabellen erzeugt `cargo run -p datagen` aus
//! `data/encoding`.
//!
//! Neben [`Decoder`] und [`Encoder`] gibt es die Einstiege, die andere
//! Standards benutzen (*hooks for standards*). *UTF-8 encode* entspricht in
//! Rust `str::as_bytes`.

#![forbid(unsafe_code)]

mod decoder;
mod encoder;
mod index;
#[rustfmt::skip]
mod tables;

pub use decoder::{DecodeError, Decoder};
pub use encoder::{Encoder, Unencodable};
pub use tables::Encoding;

use tables::LABELS;

impl Encoding {
    /// *Get an encoding*: die Kodierung zu einer Bezeichnung wie `latin1`
    /// oder `utf-8`. ASCII-Leerraum am Rand zählt nicht, Groß- und
    /// Kleinschreibung (nur ASCII) auch nicht.
    pub fn for_label(label: &[u8]) -> Option<Encoding> {
        let label = label.trim_ascii();
        // `LABELS` ist nach Bytes sortiert und enthält nur Kleinbuchstaben.
        let found = LABELS
            .binary_search_by(|(entry, _)| {
                entry.bytes().cmp(label.iter().map(u8::to_ascii_lowercase))
            })
            .ok()?;
        LABELS.get(found).map(|&(_, encoding)| encoding)
    }

    /// *Get an output encoding*: UTF-8 statt *replacement*, UTF-16BE und
    /// UTF-16LE, sonst die Kodierung selbst.
    pub fn output_encoding(self) -> Encoding {
        match self {
            Encoding::Replacement | Encoding::Utf16Be | Encoding::Utf16Le => Encoding::Utf8,
            _ => self,
        }
    }

    /// Ein neuer Dekodierer für diese Kodierung.
    pub fn new_decoder(self) -> Decoder {
        Decoder::new(self)
    }

    /// *Get an encoder*: ein neuer Kodierer, oder `None` für Kodierungen ohne
    /// Kodierer.
    pub fn new_encoder(self) -> Option<Encoder> {
        Encoder::new(self)
    }
}

/// *BOM sniff*: die Kodierung, die eine Byte-Order-Mark am Anfang angibt.
pub fn bom_sniff(bytes: &[u8]) -> Option<Encoding> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some(Encoding::Utf8)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some(Encoding::Utf16Be)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some(Encoding::Utf16Le)
    } else {
        None
    }
}

/// *Decode*: Eine Byte-Order-Mark hat Vorrang vor `fallback` und wird
/// entfernt. Liefert den Text und die verwendete Kodierung.
pub fn decode(bytes: &[u8], fallback: Encoding) -> (String, Encoding) {
    let (encoding, rest) = match bom_sniff(bytes) {
        Some(Encoding::Utf8) => (Encoding::Utf8, bytes.get(3..)),
        Some(encoding) => (encoding, bytes.get(2..)),
        None => (fallback, Some(bytes)),
    };
    let mut output = String::new();
    Decoder::new(encoding).decode(rest.unwrap_or_default(), true, &mut output);
    (output, encoding)
}

/// *UTF-8 decode*: Eine UTF-8-Byte-Order-Mark am Anfang wird entfernt.
pub fn utf8_decode(bytes: &[u8]) -> String {
    let rest = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    utf8_decode_without_bom(rest)
}

/// *UTF-8 decode without BOM*: Fehler werden zu U+FFFD.
pub fn utf8_decode_without_bom(bytes: &[u8]) -> String {
    let mut output = String::new();
    Decoder::new(Encoding::Utf8).decode(bytes, true, &mut output);
    output
}

/// *UTF-8 decode without BOM or fail*: Beim ersten Fehler kommt ein Fehler.
pub fn utf8_decode_without_bom_or_fail(bytes: &[u8]) -> Result<String, DecodeError> {
    let mut output = String::new();
    Decoder::new(Encoding::Utf8).decode_or_fail(bytes, true, &mut output)?;
    Ok(output)
}

/// *Encode* im Modus „html“, für Formulare: Nicht darstellbare Zeichen werden
/// zu `&#…;`. Kodierungen ohne Kodierer werden wie ihre Ausgabekodierung
/// UTF-8 behandelt.
pub fn encode(input: &str, encoding: Encoding) -> Vec<u8> {
    let Some(mut encoder) = Encoder::new(encoding) else {
        return input.as_bytes().to_vec();
    };
    let mut output = Vec::new();
    encoder.encode(input, true, &mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Die Kodierungen, die keine Einzelbyte-Kodierungen sind.
    const NOT_SINGLE_BYTE: [Encoding; 12] = [
        Encoding::Utf8,
        Encoding::Gbk,
        Encoding::Gb18030,
        Encoding::Big5,
        Encoding::EucJp,
        Encoding::Iso2022Jp,
        Encoding::ShiftJis,
        Encoding::EucKr,
        Encoding::Replacement,
        Encoding::Utf16Be,
        Encoding::Utf16Le,
        Encoding::XUserDefined,
    ];

    #[test]
    fn labels() {
        assert_eq!(Encoding::for_label(b"utf-8"), Some(Encoding::Utf8));
        assert_eq!(
            Encoding::for_label(b" \t\n\x0C\rUTF8 \r\n"),
            Some(Encoding::Utf8)
        );
        assert_eq!(Encoding::for_label(b"latin1"), Some(Encoding::Windows1252));
        assert_eq!(Encoding::for_label(b"ASCII"), Some(Encoding::Windows1252));
        assert_eq!(
            Encoding::for_label(b"iso-8859-1"),
            Some(Encoding::Windows1252)
        );
        assert_eq!(Encoding::for_label(b"utf-16"), Some(Encoding::Utf16Le));
        assert_eq!(Encoding::for_label(b"gb2312"), Some(Encoding::Gbk));
        assert_eq!(Encoding::for_label(b"x-sjis"), Some(Encoding::ShiftJis));
        assert_eq!(
            Encoding::for_label(b"iso-2022-kr"),
            Some(Encoding::Replacement)
        );
        assert_eq!(
            Encoding::for_label(b"x-user-defined"),
            Some(Encoding::XUserDefined)
        );
        // Vertikaler Tabulator ist kein ASCII-Leerraum.
        assert_eq!(Encoding::for_label(b"\x0Butf-8"), None);
        assert_eq!(Encoding::for_label(b"utf-8\0"), None);
        assert_eq!(Encoding::for_label(b"utf 8"), None);
        assert_eq!(Encoding::for_label(b""), None);
        assert_eq!(
            Encoding::for_label(b"replacement"),
            Some(Encoding::Replacement)
        );
        // Nur ASCII zählt: U+212A KELVIN SIGN ist kein „k“.
        assert_eq!(Encoding::for_label("\u{212A}oi8-r".as_bytes()), None);
        assert_eq!(Encoding::for_label(b"KOI8-R"), Some(Encoding::Koi8R));
    }

    #[test]
    fn every_label_and_name_is_found() {
        assert!(LABELS.windows(2).all(|pair| match pair {
            [(first, _), (second, _)] => first < second,
            _ => false,
        }));
        for &(label, encoding) in &LABELS {
            assert_eq!(Encoding::for_label(label.as_bytes()), Some(encoding));
            let upper = label.to_ascii_uppercase();
            assert_eq!(Encoding::for_label(upper.as_bytes()), Some(encoding));
        }
        // Jeder Name ist auch eine Bezeichnung.
        for encoding in Encoding::ALL {
            assert_eq!(
                Encoding::for_label(encoding.name().as_bytes()),
                Some(encoding)
            );
        }
    }

    #[test]
    fn single_byte_encodings() {
        let single_byte: Vec<Encoding> = Encoding::ALL
            .into_iter()
            .filter(|encoding| !NOT_SINGLE_BYTE.contains(encoding))
            .collect();
        assert_eq!(single_byte.len(), 28);
        for encoding in Encoding::ALL {
            assert_eq!(
                tables::single_byte_index(encoding).is_some(),
                single_byte.contains(&encoding),
                "{}",
                encoding.name()
            );
        }
    }

    #[test]
    fn output_encodings_and_encoders() {
        for encoding in Encoding::ALL {
            let without_encoder = matches!(
                encoding,
                Encoding::Replacement | Encoding::Utf16Be | Encoding::Utf16Le
            );
            assert_eq!(encoding.new_encoder().is_none(), without_encoder);
            let expected = if without_encoder {
                Encoding::Utf8
            } else {
                encoding
            };
            assert_eq!(encoding.output_encoding(), expected);
        }
    }

    #[test]
    fn byte_order_marks() {
        assert_eq!(bom_sniff(b"\xEF\xBB\xBFa"), Some(Encoding::Utf8));
        assert_eq!(bom_sniff(b"\xEF\xBB"), None);
        assert_eq!(bom_sniff(b"\xFE\xFF"), Some(Encoding::Utf16Be));
        assert_eq!(bom_sniff(b"\xFF\xFE\x00"), Some(Encoding::Utf16Le));
        assert_eq!(bom_sniff(b""), None);
        assert_eq!(
            decode(b"\xEF\xBB\xBFabc", Encoding::Windows1252),
            ("abc".to_owned(), Encoding::Utf8)
        );
        assert_eq!(
            decode(b"\xFE\xFF\x00a", Encoding::Utf8),
            ("a".to_owned(), Encoding::Utf16Be)
        );
        assert_eq!(
            decode(b"\xFF\xFEa\x00", Encoding::Utf8),
            ("a".to_owned(), Encoding::Utf16Le)
        );
        assert_eq!(
            decode(b"\xE4", Encoding::Windows1252),
            ("ä".to_owned(), Encoding::Windows1252)
        );
        // Nur eine Byte-Order-Mark wird entfernt.
        assert_eq!(utf8_decode(b"\xEF\xBB\xBF\xEF\xBB\xBFa"), "\u{FEFF}a");
        assert_eq!(utf8_decode_without_bom(b"\xEF\xBB\xBFa"), "\u{FEFF}a");
        assert_eq!(utf8_decode_without_bom(b"a\xFFb"), "a\u{FFFD}b");
        assert_eq!(
            utf8_decode_without_bom_or_fail(b"a\xC3\xA4").as_deref(),
            Ok("aä")
        );
        assert_eq!(
            utf8_decode_without_bom_or_fail(b"a\xFF"),
            Err(DecodeError {
                encoding: Encoding::Utf8
            })
        );
    }

    #[test]
    fn encode_for_forms() {
        assert_eq!(encode("a☃", Encoding::Windows1252), b"a&#9731;");
        assert_eq!(encode("€", Encoding::Windows1252), [0x80]);
        assert_eq!(encode("€", Encoding::Gbk), [0x80]);
        // index gb18030: € bei Zeiger 6432.
        assert_eq!(encode("€", Encoding::Gb18030), [0xA2, 0xE3]);
        assert_eq!(encode("\u{E5E5}", Encoding::Gb18030), b"&#58853;");
        assert_eq!(encode("ä", Encoding::Utf16Le), "ä".as_bytes());
        assert_eq!(encode("ä", Encoding::Replacement), "ä".as_bytes());
        assert_eq!(encode("😀", Encoding::Utf8), "😀".as_bytes());
    }
}
