//! Prozentkodierung (URL Standard, Abschnitt 1.3 „Percent-encoded bytes“).

use mb_encoding::Encoding;

/// Eine Menge von Zeichen, die prozentkodiert werden
/// (`#percent-encode-set`). Zeichen außerhalb von ASCII gehören zu jeder
/// Menge des Standards; für ASCII hält ein Bit je Zeichen fest, ob es dazu
/// gehört.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeSet {
    ascii: u128,
}

impl EncodeSet {
    /// Die Menge mit den zusätzlichen ASCII-Zeichen aus `chars`.
    const fn with(self, chars: &[u8]) -> EncodeSet {
        let mut ascii = self.ascii;
        let mut index = 0;
        while index < chars.len() {
            ascii |= 1 << chars[index];
            index += 1;
        }
        EncodeSet { ascii }
    }

    pub fn contains(self, c: char) -> bool {
        match u8::try_from(c) {
            Ok(byte) if byte.is_ascii() => self.ascii & (1 << byte) != 0,
            _ => true,
        }
    }
}

/// `#c0-control-percent-encode-set`: C0-Steuerzeichen und alles über U+007E.
pub const C0_CONTROL: EncodeSet = EncodeSet {
    // U+0000 bis U+001F und U+007F.
    ascii: 0xFFFF_FFFF | (1 << 0x7F),
};

/// `#fragment-percent-encode-set`
pub const FRAGMENT: EncodeSet = C0_CONTROL.with(b" \"<>`");

/// `#query-percent-encode-set`
pub const QUERY: EncodeSet = C0_CONTROL.with(b" \"#<>");

/// `#special-query-percent-encode-set`
pub const SPECIAL_QUERY: EncodeSet = QUERY.with(b"'");

/// `#path-percent-encode-set`
pub const PATH: EncodeSet = QUERY.with(b"?^`{}");

/// `#userinfo-percent-encode-set`
pub const USERINFO: EncodeSet = PATH.with(b"/:;=@[\\]|");

/// `#component-percent-encode-set`
pub const COMPONENT: EncodeSet = USERINFO.with(b"$%&+,");

/// `#application-x-www-form-urlencoded-percent-encode-set`
pub const FORM_URLENCODED: EncodeSet = COMPONENT.with(b"!'()~");

/// Hängt `%` und zwei große Hexziffern an (`#percent-encode`).
fn push_percent_encoded(byte: u8, output: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    output.push('%');
    for nibble in [byte >> 4, byte & 0x0F] {
        output.push(char::from(HEX[usize::from(nibble)]));
    }
}

/// `#percent-decode` für Bytes. `%` ohne zwei Hexziffern bleibt stehen.
pub fn percent_decode(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut bytes = input.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let mut lookahead = bytes.clone();
            let high = lookahead.next().and_then(hex_value);
            let low = lookahead.next().and_then(hex_value);
            if let (Some(high), Some(low)) = (high, low) {
                output.push(high << 4 | low);
                bytes = lookahead;
                continue;
            }
        }
        output.push(byte);
    }
    output
}

fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

/// Ob `text` ein prozentkodiertes Byte enthält (`#percent-encoded-byte`).
pub fn contains_percent_encoded_byte(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(3).any(|window| {
        matches!(window, [b'%', high, low] if high.is_ascii_hexdigit() && low.is_ascii_hexdigit())
    })
}

/// `#string-percent-encode-after-encoding`: kodiert `input` mit `encoding`
/// und prozentkodiert jedes Byte aus `set`. Was die Kodierung nicht darstellen
/// kann, wird zu `%26%23…%3B`. Mit `space_as_plus` (*spaceAsPlus*) wird das
/// Leerzeichen zu `+`; das will nur der Serialisierer für
/// `application/x-www-form-urlencoded`.
pub fn percent_encode_after_encoding(
    encoding: Encoding,
    input: &str,
    set: EncodeSet,
    space_as_plus: bool,
) -> String {
    let mut output = String::with_capacity(input.len());
    // UTF-8 kann alles darstellen und braucht keinen Kodierer.
    if encoding == Encoding::Utf8 {
        push_encoded_bytes(input.as_bytes(), set, space_as_plus, &mut output);
        return output;
    }
    // Die Ausgabekodierungen haben alle einen Kodierer; sonst gilt UTF-8.
    let Some(mut encoder) = encoding.output_encoding().new_encoder() else {
        push_encoded_bytes(input.as_bytes(), set, space_as_plus, &mut output);
        return output;
    };
    let mut position = 0;
    loop {
        let mut bytes = Vec::new();
        let rest = input.get(position..).unwrap_or_default();
        let result = encoder.encode_or_fail(rest, true, &mut bytes);
        push_encoded_bytes(&bytes, set, space_as_plus, &mut output);
        match result {
            Ok(()) => return output,
            Err(unencodable) => {
                output.push_str("%26%23");
                output.push_str(&u32::from(unencodable.character).to_string());
                output.push_str("%3B");
                position += unencodable.consumed;
            }
        }
    }
}

fn push_encoded_bytes(bytes: &[u8], set: EncodeSet, space_as_plus: bool, output: &mut String) {
    for &byte in bytes {
        if space_as_plus && byte == b' ' {
            output.push('+');
        } else if set.contains(char::from(byte)) {
            push_percent_encoded(byte, output);
        } else {
            output.push(char::from(byte));
        }
    }
}

/// `#string-utf-8-percent-encode`
pub fn utf8_percent_encode(input: &str, set: EncodeSet) -> String {
    let mut output = String::with_capacity(input.len());
    for c in input.chars() {
        push_utf8_percent_encoded(c, set, &mut output);
    }
    output
}

/// `#utf-8-percent-encode` für ein Zeichen, an `output` angehängt.
pub fn push_utf8_percent_encoded(c: char, set: EncodeSet, output: &mut String) {
    if set.contains(c) {
        let mut buffer = [0; 4];
        for &byte in c.encode_utf8(&mut buffer).as_bytes() {
            push_percent_encoded(byte, output);
        }
    } else {
        output.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Die Beispiele aus dem Standard (`#example-percent-encode-operations`).
    #[test]
    fn examples_from_the_standard() {
        let mut encoded = String::new();
        push_percent_encoded(0x23, &mut encoded);
        push_percent_encoded(0x7F, &mut encoded);
        assert_eq!(encoded, "%23%7F");
        assert_eq!(percent_decode(b"%25%s%1G"), b"%%s%1G");
        assert_eq!(
            percent_decode("‽%25%2E".as_bytes()),
            [0xE2, 0x80, 0xBD, 0x25, 0x2E]
        );
        let shift_jis =
            |input| percent_encode_after_encoding(Encoding::ShiftJis, input, SPECIAL_QUERY, false);
        assert_eq!(shift_jis(" "), "%20");
        assert_eq!(shift_jis("≡"), "%81%DF");
        assert_eq!(shift_jis("‽"), "%26%238253%3B");
        assert_eq!(
            percent_encode_after_encoding(Encoding::Iso2022Jp, "¥", SPECIAL_QUERY, false),
            "%1B(J\\%1B(B"
        );
        assert_eq!(
            percent_encode_after_encoding(Encoding::ShiftJis, "1+1 ≡ 2%20‽", FORM_URLENCODED, true),
            "1%2B1+%81%DF+2%2520%26%238253%3B"
        );
        assert_eq!(utf8_percent_encode("≡", USERINFO), "%E2%89%A1");
        assert_eq!(utf8_percent_encode("‽", USERINFO), "%E2%80%BD");
        assert_eq!(
            utf8_percent_encode("Say what‽", USERINFO),
            "Say%20what%E2%80%BD"
        );
        // UTF-8 percent-encode ruft percent-encode after encoding ohne
        // spaceAsPlus auf, also bleibt es auch bei dieser Menge bei `%20`.
        assert_eq!(utf8_percent_encode("a b", FORM_URLENCODED), "a%20b");
        assert_eq!(
            percent_encode_after_encoding(Encoding::Utf8, "a b", FORM_URLENCODED, false),
            "a%20b"
        );
        assert_eq!(
            percent_encode_after_encoding(Encoding::Utf8, "a b", FORM_URLENCODED, true),
            "a+b"
        );
    }

    #[test]
    fn sets() {
        // Laut Standard bleiben bei FORM_URLENCODED nur alphanumerische
        // Zeichen und `*-._` übrig.
        let kept: String = (0u8..0x80)
            .map(char::from)
            .filter(|&c| !FORM_URLENCODED.contains(c))
            .filter(|c| !c.is_ascii_alphanumeric())
            .collect();
        assert_eq!(kept, "*-._");
        // Der Unterschied zwischen QUERY und FRAGMENT.
        assert!(QUERY.contains('#') && !FRAGMENT.contains('#'));
        assert!(FRAGMENT.contains('`') && !QUERY.contains('`'));
        assert!(C0_CONTROL.contains('\u{7F}') && C0_CONTROL.contains('ä'));
        assert!(!C0_CONTROL.contains('~') && !C0_CONTROL.contains(' '));
        assert!(contains_percent_encoded_byte("a%2Eb") && !contains_percent_encoded_byte("a%2"));
    }
}
