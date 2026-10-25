//! Hosts (URL Standard, Abschnitt 3): Domains, IP-Adressen, opake und leere
//! Hosts, mit Parser und Serialisierung.

use std::fmt::Write as _;

use mb_encoding::utf8_decode_without_bom;
use mb_unicode::idna;

use crate::error::{Report, ValidationError};
use crate::percent::{
    C0_CONTROL, contains_percent_encoded_byte, percent_decode, utf8_percent_encode,
};

/// Ein Host (`#concept-host`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Host {
    /// Eine Domain: nicht leer und nur ASCII.
    Domain(String),
    /// Eine IPv4-Adresse.
    Ipv4(u32),
    /// Eine IPv6-Adresse aus acht Teilen.
    Ipv6([u16; 8]),
    /// Ein opaker Host einer URL ohne speziellen Schema: nicht leer und nur
    /// ASCII.
    Opaque(String),
    /// Der leere Host.
    Empty,
}

impl Host {
    /// Der Host-Parser (`#concept-host-parser`). Mit `is_opaque` wird ein
    /// opaker Host gelesen, wie bei URLs ohne speziellen Schema.
    pub fn parse(input: &str, is_opaque: bool) -> Result<Host, ValidationError> {
        parse(input, is_opaque, &mut Report::new(None))
    }

    /// Wie [`Host::parse`], dazu alle Validierungsfehler in `errors`.
    pub fn parse_with_errors(
        input: &str,
        is_opaque: bool,
        errors: &mut Vec<ValidationError>,
    ) -> Result<Host, ValidationError> {
        parse(input, is_opaque, &mut Report::new(Some(errors)))
    }
}

/// Der Host-Serialisierer (`#concept-host-serializer`).
impl std::fmt::Display for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Host::Domain(text) | Host::Opaque(text) => f.write_str(text),
            Host::Ipv4(address) => f.write_str(&serialize_ipv4(*address)),
            Host::Ipv6(address) => write!(f, "[{}]", serialize_ipv6(address)),
            Host::Empty => Ok(()),
        }
    }
}

pub(crate) fn parse(
    input: &str,
    is_opaque: bool,
    report: &mut Report,
) -> Result<Host, ValidationError> {
    if let Some(rest) = input.strip_prefix('[') {
        let Some(address) = rest.strip_suffix(']') else {
            return report.fail(ValidationError::Ipv6Unclosed);
        };
        return ipv6_parse(address, report).map(Host::Ipv6);
    }
    if is_opaque {
        return opaque_host_parse(input, report);
    }
    if report.is_wanted() && contains_percent_encoded_byte(input) {
        report.error(ValidationError::DomainPercentEncoded);
    }
    let domain = utf8_decode_without_bom(&percent_decode(input.as_bytes()));
    let ascii_domain = domain_parser(&domain, report)?;
    if ends_in_a_number(&ascii_domain) {
        if !domain.is_ascii() {
            report.error(ValidationError::Ipv4NonAsciiInput);
        }
        return ipv4_parse(&ascii_domain, report).map(Host::Ipv4);
    }
    Ok(Host::Domain(ascii_domain))
}

/// `#forbidden-host-code-point`
pub(crate) fn is_forbidden_host_code_point(c: char) -> bool {
    matches!(
        c,
        '\0' | '\t'
            | '\n'
            | '\r'
            | ' '
            | '#'
            | '/'
            | ':'
            | '<'
            | '>'
            | '?'
            | '@'
            | '['
            | '\\'
            | ']'
            | '^'
            | '|'
    )
}

/// `#forbidden-domain-code-point`
fn is_forbidden_domain_code_point(c: char) -> bool {
    is_forbidden_host_code_point(c) || c.is_ascii_control() || c == '%'
}

/// Die Schalter für Unicode ToASCII im *domain parser ToASCII*. Der Standard
/// ruft es auch streng auf, aber nur, um einen Validierungsfehler zu melden.
fn to_ascii_options(be_strict: bool) -> idna::Options {
    idna::Options {
        check_hyphens: be_strict,
        check_bidi: true,
        check_joiners: true,
        use_std3_ascii_rules: be_strict,
        transitional_processing: false,
        verify_dns_length: be_strict,
    }
}

/// Der *domain parser* (`#concept-domain-to-ascii`) mit *beStrict* aus, so
/// wie ihn der Host-Parser aufruft.
fn domain_parser(domain: &str, report: &mut Report) -> Result<String, ValidationError> {
    // Das strenge Ergebnis zählt nur für den Validierungsfehler.
    if report.is_wanted() && idna::to_ascii(domain, to_ascii_options(true)).is_err() {
        report.error(ValidationError::DomainToAscii);
    }
    // Auch ohne Meldung oben ist das strenge ToASCII in jedem der folgenden
    // Abbruchfälle gescheitert, denn es prüft mehr als das lockere.
    let result = if domain.is_ascii() {
        domain.to_ascii_lowercase()
    } else {
        idna::to_ascii(domain, to_ascii_options(false))
            .map_err(|_| ValidationError::DomainToAscii)?
    };
    if result.is_empty() || result.chars().any(is_forbidden_domain_code_point) {
        return Err(ValidationError::DomainToAscii);
    }
    Ok(result)
}

/// *Domain to Unicode* (`#concept-domain-to-unicode`): die Domain mit
/// dekodierten Punycode-Labels, oder unverändert, wenn dabei ein Fehler
/// auftritt.
pub fn domain_to_unicode(domain: &str) -> String {
    let options = idna::Options {
        check_hyphens: false,
        check_bidi: true,
        check_joiners: true,
        use_std3_ascii_rules: false,
        transitional_processing: false,
        verify_dns_length: false,
    };
    let (result, errors) = idna::to_unicode(domain, options);
    if errors.is_empty() {
        result
    } else {
        domain.to_owned()
    }
}

/// Der *ends in a number checker* (`#ends-in-a-number-checker`).
fn ends_in_a_number(input: &str) -> bool {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    let Some(last) = parts.last() else {
        return false;
    };
    if !last.is_empty() && last.bytes().all(|byte| byte.is_ascii_digit()) {
        return true;
    }
    ipv4_number_parse(last).is_some()
}

/// Der IPv4-Parser (`#concept-ipv4-parser`).
fn ipv4_parse(input: &str, report: &mut Report) -> Result<u32, ValidationError> {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") {
        report.error(ValidationError::Ipv4EmptyPart);
        if parts.len() > 1 {
            parts.pop();
        }
    }
    if parts.len() < 4 {
        report.error(ValidationError::Ipv4TooFewParts);
    }
    if parts.len() > 4 {
        return report.fail(ValidationError::Ipv4TooManyParts);
    }
    let mut numbers = Vec::with_capacity(parts.len());
    for part in parts {
        let Some((number, non_decimal)) = ipv4_number_parse(part) else {
            return report.fail(ValidationError::Ipv4NonNumericPart);
        };
        if non_decimal {
            report.error(ValidationError::Ipv4NonDecimalPart);
        }
        numbers.push(number);
    }
    if numbers.iter().any(|&number| number > 255) {
        report.error(ValidationError::Ipv4OutOfRangePart);
    }
    let Some((&last, rest)) = numbers.split_last() else {
        return Err(ValidationError::Ipv4NonNumericPart);
    };
    // Beide Abbrüche sind oben schon als IPv4-out-of-range-part gemeldet.
    if rest.iter().any(|&number| number > 255) {
        return Err(ValidationError::Ipv4OutOfRangePart);
    }
    // 256 hoch (5 − Anzahl der Zahlen); es sind ein bis vier.
    let limit = (numbers.len()..4).fold(256u64, |limit, _| limit.saturating_mul(256));
    if last >= limit {
        return Err(ValidationError::Ipv4OutOfRangePart);
    }
    // Alle Zahlen sind jetzt beschränkt, die Summe passt in 32 Bit.
    let mut ipv4 = last;
    let mut weight: u64 = 1 << 24;
    for &number in rest {
        ipv4 = ipv4.saturating_add(number.saturating_mul(weight));
        weight >>= 8;
    }
    u32::try_from(ipv4).map_err(|_| ValidationError::Ipv4OutOfRangePart)
}

/// Der *IPv4 number parser* (`#ipv4-number-parser`): die Zahl und ob sie
/// nicht dezimal geschrieben ist. Der Standard rechnet mit beliebig großen
/// Zahlen; hier bleibt eine zu große Zahl bei `u64::MAX` stehen, was an den
/// Vergleichen danach nichts ändert.
fn ipv4_number_parse(input: &str) -> Option<(u64, bool)> {
    if input.is_empty() {
        return None;
    }
    let mut validation_error = false;
    let mut radix = 10;
    let mut digits = input;
    if let Some(rest) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        validation_error = true;
        digits = rest;
        radix = 16;
    } else if input.chars().nth(1).is_some()
        && let Some(rest) = input.strip_prefix('0')
    {
        validation_error = true;
        digits = rest;
        radix = 8;
    }
    if digits.is_empty() {
        return Some((0, true));
    }
    let mut output: u64 = 0;
    for c in digits.chars() {
        let digit = c.to_digit(radix)?;
        output = output
            .saturating_mul(u64::from(radix))
            .saturating_add(u64::from(digit));
    }
    Some((output, validation_error))
}

/// Der IPv6-Parser (`#concept-ipv6-parser`).
fn ipv6_parse(input: &str, report: &mut Report) -> Result<[u16; 8], ValidationError> {
    let input: Vec<char> = input.chars().collect();
    let c = |pointer: usize| input.get(pointer).copied();
    let mut address = [0u16; 8];
    let mut piece_index: usize = 0;
    let mut compress: Option<usize> = None;
    let mut pointer: usize = 0;
    if c(pointer) == Some(':') {
        if c(pointer + 1) != Some(':') {
            return report.fail(ValidationError::Ipv6InvalidCompression);
        }
        pointer += 2;
        piece_index += 1;
        compress = Some(piece_index);
    }
    while let Some(current) = c(pointer) {
        if piece_index == 8 {
            return report.fail(ValidationError::Ipv6TooManyPieces);
        }
        if current == ':' {
            if compress.is_some() {
                return report.fail(ValidationError::Ipv6MultipleCompression);
            }
            pointer += 1;
            piece_index += 1;
            compress = Some(piece_index);
            continue;
        }
        let mut value: u32 = 0;
        let mut length: usize = 0;
        while length < 4
            && let Some(digit) = c(pointer).and_then(|c| c.to_digit(16))
        {
            value = value * 0x10 + digit;
            pointer += 1;
            length += 1;
        }
        match c(pointer) {
            Some('.') => {
                if length == 0 {
                    return report.fail(ValidationError::Ipv4InIpv6InvalidCodePoint);
                }
                pointer -= length;
                if piece_index > 6 {
                    return report.fail(ValidationError::Ipv4InIpv6TooManyPieces);
                }
                let mut numbers_seen = 0;
                while let Some(current) = c(pointer) {
                    if numbers_seen > 0 {
                        if current == '.' && numbers_seen < 4 {
                            pointer += 1;
                        } else {
                            return report.fail(ValidationError::Ipv4InIpv6InvalidCodePoint);
                        }
                    }
                    if !c(pointer).is_some_and(|c| c.is_ascii_digit()) {
                        return report.fail(ValidationError::Ipv4InIpv6InvalidCodePoint);
                    }
                    let mut ipv4_piece: Option<u32> = None;
                    while let Some(number) = c(pointer).and_then(|c| c.to_digit(10)) {
                        let piece = match ipv4_piece {
                            None => number,
                            Some(0) => {
                                return report.fail(ValidationError::Ipv4InIpv6InvalidCodePoint);
                            }
                            Some(piece) => piece * 10 + number,
                        };
                        if piece > 255 {
                            return report.fail(ValidationError::Ipv4InIpv6OutOfRangePart);
                        }
                        ipv4_piece = Some(piece);
                        pointer += 1;
                    }
                    // Höchstens 255 · 256 + 255, und `piece_index` ist höchstens 7.
                    if let Some(slot) = address.get_mut(piece_index) {
                        let combined = u32::from(*slot) * 0x100 + ipv4_piece.unwrap_or(0);
                        *slot = u16::try_from(combined).unwrap_or(u16::MAX);
                    }
                    numbers_seen += 1;
                    if numbers_seen == 2 || numbers_seen == 4 {
                        piece_index += 1;
                    }
                }
                if numbers_seen != 4 {
                    return report.fail(ValidationError::Ipv4InIpv6TooFewParts);
                }
                break;
            }
            Some(':') => {
                pointer += 1;
                if c(pointer).is_none() {
                    return report.fail(ValidationError::Ipv6InvalidCodePoint);
                }
            }
            Some(_) => return report.fail(ValidationError::Ipv6InvalidCodePoint),
            None => {}
        }
        // Eine führende Null: Der Wert ist kleiner als 0x10 hoch (Länge − 1).
        if length > 1 && value < 1 << (4 * (length - 1)) {
            report.error(ValidationError::Ipv6PieceLeadingZero);
        }
        // Höchstens vier Hexziffern; `piece_index` ist kleiner als 8.
        if let Some(slot) = address.get_mut(piece_index) {
            *slot = u16::try_from(value).unwrap_or(u16::MAX);
        }
        piece_index += 1;
    }
    if let Some(compress) = compress {
        let mut swaps = piece_index - compress;
        piece_index = 7;
        while piece_index != 0 && swaps > 0 {
            address.swap(piece_index, compress + swaps - 1);
            piece_index -= 1;
            swaps -= 1;
        }
    } else if piece_index != 8 {
        return report.fail(ValidationError::Ipv6TooFewPieces);
    }
    Ok(address)
}

/// Der Parser für opake Hosts (`#concept-opaque-host-parser`). Eine leere
/// Eingabe ergibt den leeren Host.
fn opaque_host_parse(input: &str, report: &mut Report) -> Result<Host, ValidationError> {
    if input.chars().any(is_forbidden_host_code_point) {
        return report.fail(ValidationError::HostInvalidCodePoint);
    }
    if report.is_wanted()
        && (input.chars().any(|c| !is_url_code_point(c) && c != '%')
            || has_invalid_percent_sign(input))
    {
        report.error(ValidationError::InvalidUrlUnit);
    }
    let host = utf8_percent_encode(input, C0_CONTROL);
    Ok(if host.is_empty() {
        Host::Empty
    } else {
        Host::Opaque(host)
    })
}

/// Ob irgendwo ein `%` steht, auf das keine zwei Hexziffern folgen.
fn has_invalid_percent_sign(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.iter().enumerate().any(|(index, &byte)| {
        byte == b'%'
            && !(bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit))
    })
}

/// `#url-code-points`
pub(crate) fn is_url_code_point(c: char) -> bool {
    match c {
        'a'..='z' | 'A'..='Z' | '0'..='9' => true,
        '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | '-' | '.' | '/' | ':' | ';'
        | '=' | '?' | '@' | '_' | '~' => true,
        '\u{A0}'..='\u{10FFFD}' => !is_noncharacter(c),
        _ => false,
    }
}

/// Nichtzeichen: U+FDD0 bis U+FDEF und die letzten beiden Codepunkte jeder
/// Ebene. Surrogate sind keine `char`.
fn is_noncharacter(c: char) -> bool {
    let code_point = u32::from(c);
    (0xFDD0..=0xFDEF).contains(&code_point) || code_point & 0xFFFE == 0xFFFE
}

/// Der IPv4-Serialisierer (`#concept-ipv4-serializer`).
fn serialize_ipv4(address: u32) -> String {
    let [a, b, c, d] = address.to_be_bytes();
    format!("{a}.{b}.{c}.{d}")
}

/// Der IPv6-Serialisierer (`#concept-ipv6-serializer`), ohne Klammern.
fn serialize_ipv6(address: &[u16; 8]) -> String {
    let mut output = String::new();
    let compress = compressed_piece_index(address);
    let mut ignore0 = false;
    for (piece_index, &piece) in address.iter().enumerate() {
        if ignore0 && piece == 0 {
            continue;
        }
        ignore0 = false;
        if compress == Some(piece_index) {
            output.push_str(if piece_index == 0 { "::" } else { ":" });
            ignore0 = true;
            continue;
        }
        // Schreiben in einen String gelingt immer.
        let _ = write!(output, "{piece:x}");
        if piece_index != 7 {
            output.push(':');
        }
    }
    output
}

/// *Find the IPv6 address compressed piece index*: der Anfang der ersten
/// längsten Folge von mindestens zwei Nullen.
fn compressed_piece_index(address: &[u16; 8]) -> Option<usize> {
    let mut longest_index = None;
    let mut longest_size = 1;
    let mut found_index = None;
    let mut found_size = 0;
    for (piece_index, &piece) in address.iter().enumerate() {
        if piece != 0 {
            if found_size > longest_size {
                longest_index = found_index;
                longest_size = found_size;
            }
            found_index = None;
            found_size = 0;
        } else {
            if found_index.is_none() {
                found_index = Some(piece_index);
            }
            found_size += 1;
        }
    }
    if found_size > longest_size {
        return found_index;
    }
    longest_index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_serialization() {
        // Das Beispiel aus dem Standard: Die Verkürzung beginnt bei der zweiten Null.
        assert_eq!(
            compressed_piece_index(&[0, 0xF, 0, 0, 0xF, 0xF, 0, 0]),
            Some(2)
        );
        assert_eq!(
            serialize_ipv6(&[0, 0xF, 0, 0, 0xF, 0xF, 0, 0]),
            "0:f::f:f:0:0"
        );
        assert_eq!(serialize_ipv6(&[0; 8]), "::");
        assert_eq!(serialize_ipv6(&[0, 0, 0, 0, 0, 0, 0, 1]), "::1");
        assert_eq!(serialize_ipv6(&[1, 0, 0, 0, 0, 0, 0, 0]), "1::");
        // Eine einzelne Null wird nicht verkürzt.
        assert_eq!(serialize_ipv6(&[1, 0, 2, 3, 4, 5, 6, 7]), "1:0:2:3:4:5:6:7");
        assert_eq!(serialize_ipv4(0x7F00_0001), "127.0.0.1");
    }

    #[test]
    fn host_parser() {
        let parse = |input| Host::parse(input, false);
        assert_eq!(parse("EXAMPLE.org"), Ok(Host::Domain("example.org".into())));
        assert_eq!(parse("0x7f.1"), Ok(Host::Ipv4(0x7F00_0001)));
        assert_eq!(parse("1.2.3.4."), Ok(Host::Ipv4(0x0102_0304)));
        assert_eq!(parse("4294967295"), Ok(Host::Ipv4(u32::MAX)));
        assert_eq!(
            parse("4294967296"),
            Err(ValidationError::Ipv4OutOfRangePart)
        );
        assert_eq!(parse("09"), Err(ValidationError::Ipv4NonNumericPart));
        assert_eq!(
            parse("[::ffff:1.2.3.4]"),
            Ok(Host::Ipv6([0, 0, 0, 0, 0, 0xFFFF, 0x0102, 0x0304]))
        );
        assert_eq!(
            parse("b%C3%BCcher"),
            Ok(Host::Domain("xn--bcher-kva".into()))
        );
        assert_eq!(
            Host::parse("a b", true),
            Err(ValidationError::HostInvalidCodePoint)
        );
        assert_eq!(Host::parse("", true), Ok(Host::Empty));
        assert_eq!(
            Host::parse("ä%41", true),
            Ok(Host::Opaque("%C3%A4%41".into()))
        );
        assert_eq!(domain_to_unicode("xn--bcher-kva.example"), "bücher.example");
        // Ein Fehler bei ToUnicode lässt die Domain, wie sie ist.
        assert_eq!(domain_to_unicode("xn--8i7caa"), "xn--8i7caa");
    }
}
