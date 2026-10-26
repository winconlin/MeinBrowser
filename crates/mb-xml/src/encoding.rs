//! Die Zeichenkodierung eines Dokuments nach XML 1.0, Abschnitt 4.3.3 und
//! Anhang F, mit den Kodierungen und Bezeichnungen des WHATWG Encoding
//! Standard.

use mb_encoding::{Decoder, Encoding, bom_sniff};

const UNKNOWN: &str = "unbekannte Zeichenkodierung";
const MISMATCH: &str = "die Zeichenkodierung passt nicht zur Kodierungsdeklaration";
const INVALID: &str = "ungültige Bytefolge für die Zeichenkodierung";
const NAME: &str = "ungültiger Name der Zeichenkodierung";

/// Die Bezeichnungen für UTF-16 aus dem Encoding Standard, die keine
/// Bytereihenfolge nennen. `utf-16le`, `utf-16be`, `unicodefeff` und
/// `unicodefffe` nennen eine.
const UTF16_WITHOUT_BYTE_ORDER: [&[u8]; 5] = [
    b"utf-16",
    b"unicode",
    b"ucs-2",
    b"csunicode",
    b"iso-10646-ucs-2",
];

/// `EncName`
pub(crate) fn is_encoding_name(name: &[u8]) -> bool {
    name.first().is_some_and(u8::is_ascii_alphabetic)
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// *Get an encoding* für einen Namen aus der Deklaration.
fn declared_encoding(label: &[u8]) -> Result<Encoding, &'static str> {
    if !is_encoding_name(label) {
        return Err(NAME);
    }
    Encoding::for_label(label).ok_or(UNKNOWN)
}

/// Dekodiert das Dokument. Eine Byte-Order-Mark hat Vorrang und wird
/// entfernt; sonst gilt die Kodierungsdeklaration, ohne sie UTF-8. Fatal
/// sind eine unbekannte Kodierung, eine Deklaration, die nicht zur
/// Byte-Order-Mark oder zur Anordnung der ersten Zeichen passt, und jede
/// ungültige Bytefolge.
pub(crate) fn decode(bytes: &[u8]) -> Result<(String, Encoding), &'static str> {
    let (bom, rest) = match bom_sniff(bytes) {
        Some(Encoding::Utf8) => (Some(Encoding::Utf8), bytes.get(3..).unwrap_or_default()),
        Some(encoding) => (Some(encoding), bytes.get(2..).unwrap_or_default()),
        None => (None, bytes),
    };
    // Ohne Byte-Order-Mark erkennt man UTF-16 an `<?` am Anfang (Anhang
    // F.1); dann muss die Deklaration die Bytereihenfolge nennen.
    let utf16 = match bom {
        Some(encoding @ (Encoding::Utf16Le | Encoding::Utf16Be)) => Some(encoding),
        Some(_) => None,
        None if rest.starts_with(&[0x3C, 0, 0x3F, 0]) => Some(Encoding::Utf16Le),
        None if rest.starts_with(&[0, 0x3C, 0, 0x3F]) => Some(Encoding::Utf16Be),
        None => None,
    };
    if let Some(encoding) = utf16 {
        let text = decode_as(encoding, rest)?;
        match declared_label(text.as_bytes()) {
            Some(label) => {
                let declared = declared_encoding(label)?;
                // Mit Byte-Order-Mark passt auch eine Bezeichnung ohne
                // Bytereihenfolge, die der Encoding Standard UTF-16LE zuordnet.
                let fits = declared == encoding
                    || (bom.is_some()
                        && UTF16_WITHOUT_BYTE_ORDER
                            .iter()
                            .any(|name| label.eq_ignore_ascii_case(name)));
                if !fits {
                    return Err(MISMATCH);
                }
            }
            None if bom.is_none() => return Err(MISMATCH),
            None => {}
        }
        return Ok((text, encoding));
    }
    let encoding = match declared_label(rest) {
        Some(label) => {
            let declared = declared_encoding(label)?;
            let fits = match bom {
                Some(bom) => declared == bom,
                None => !matches!(declared, Encoding::Utf16Le | Encoding::Utf16Be),
            };
            if !fits {
                return Err(MISMATCH);
            }
            declared
        }
        None => Encoding::Utf8,
    };
    Ok((decode_as(encoding, rest)?, encoding))
}

fn decode_as(encoding: Encoding, bytes: &[u8]) -> Result<String, &'static str> {
    let mut text = String::new();
    Decoder::new(encoding)
        .decode_or_fail(bytes, true, &mut text)
        .map_err(|_| INVALID)?;
    Ok(text)
}

/// Der Wert von `encoding` in der XML-Deklaration am Anfang, gelesen wie
/// ASCII. Ob die Deklaration sonst stimmt, prüft der Parser.
fn declared_label(bytes: &[u8]) -> Option<&[u8]> {
    let rest = bytes.strip_prefix(b"<?xml")?;
    let rest = skip_space(rest).filter(|after| after.len() < rest.len())?;
    let rest = rest.strip_prefix(b"version")?;
    let (_, rest) = pseudo_attribute(rest)?;
    let rest = skip_space(rest).filter(|after| after.len() < rest.len())?;
    let rest = rest.strip_prefix(b"encoding")?;
    pseudo_attribute(rest).map(|(value, _)| value)
}

fn skip_space(bytes: &[u8]) -> Option<&[u8]> {
    let start = bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        .unwrap_or(bytes.len());
    bytes.get(start..)
}

/// `Eq` und ein Wert in Anführungszeichen; liefert den Wert und den Rest.
fn pseudo_attribute(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = skip_space(bytes)?.strip_prefix(b"=")?;
    let (&quote, rest) = skip_space(rest)?.split_first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let end = rest.iter().position(|&byte| byte == quote)?;
    Some((rest.get(..end)?, rest.get(end.checked_add(1)?..)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        let units = (if bom { "\u{FEFF}" } else { "" }).encode_utf16();
        for unit in units.chain(text.encode_utf16()) {
            if big_endian {
                bytes.extend(unit.to_be_bytes());
            } else {
                bytes.extend(unit.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn detection() {
        assert_eq!(decode(b"<a/>"), Ok(("<a/>".to_owned(), Encoding::Utf8)));
        assert_eq!(
            decode(b"\xEF\xBB\xBF<a/>"),
            Ok(("<a/>".to_owned(), Encoding::Utf8))
        );
        let latin = b"<?xml version='1.0' encoding='ISO-8859-1'?><a>\xE4</a>";
        assert_eq!(
            decode(latin).map(|(text, encoding)| (text.ends_with("<a>\u{E4}</a>"), encoding)),
            Ok((true, Encoding::Windows1252))
        );
        let declared = "<?xml version='1.0' encoding='UTF-16'?><a/>";
        assert_eq!(
            decode(&utf16(declared, true, true)).map(|(_, encoding)| encoding),
            Ok(Encoding::Utf16Be)
        );
        assert_eq!(
            decode(&utf16("<a/>", false, true)).map(|(_, encoding)| encoding),
            Ok(Encoding::Utf16Le)
        );
        let declared = "<?xml version='1.0' encoding='UTF-16BE'?><a/>";
        assert_eq!(
            decode(&utf16(declared, true, false)).map(|(_, encoding)| encoding),
            Ok(Encoding::Utf16Be)
        );
    }

    #[test]
    fn fatal_errors() {
        assert_eq!(decode(b"<a>\xFF</a>"), Err(INVALID));
        assert_eq!(
            decode(b"<?xml version='1.0' encoding='no-such-thing'?><a/>"),
            Err(UNKNOWN)
        );
        assert_eq!(
            decode(b"<?xml version='1.0' encoding='utf:8'?><a/>"),
            Err(NAME)
        );
        assert_eq!(
            decode(b"\xEF\xBB\xBF<?xml version='1.0' encoding='iso-8859-1'?><a/>"),
            Err(MISMATCH)
        );
        assert_eq!(
            decode(b"<?xml version='1.0' encoding='utf-16'?><a/>"),
            Err(MISMATCH)
        );
        let declared = "<?xml version='1.0' encoding='UTF-8'?><a/>";
        assert_eq!(decode(&utf16(declared, false, true)), Err(MISMATCH));
        let undeclared = "<?xml version='1.0'?><a/>";
        assert_eq!(decode(&utf16(undeclared, false, false)), Err(MISMATCH));
    }
}
