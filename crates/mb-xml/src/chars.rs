//! Zeichenklassen aus XML 1.0 (Fifth Edition), Abschnitte 2.2 und 2.3.

/// `Char`: die Zeichen, die in einem Dokument stehen dürfen.
pub(crate) fn is_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}')
        || c >= '\u{10000}'
}

/// `S`: Leerraum.
pub(crate) fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// `NameStartChar`
pub(crate) fn is_name_start_char(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}'
        | '\u{D8}'..='\u{F6}'
        | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}'
        | '\u{37F}'..='\u{1FFF}'
        | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}'
        | '\u{2C00}'..='\u{2FEF}'
        | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}'
        | '\u{FDF0}'..='\u{FFFD}'
        | '\u{10000}'..='\u{EFFFF}'
    )
}

/// `NameChar`
pub(crate) fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c,
            '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}'
        )
}

/// `PubidChar`
pub(crate) fn is_pubid_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || " \r\n-'()+,./:=?;!*#@$_%".contains(c)
}

/// `QName` aus Namespaces in XML 1.0 für einen Namen, der schon `Name`
/// ist: höchstens ein Doppelpunkt, und vor wie nach ihm ein `NCName`.
pub(crate) fn is_qname(name: &str) -> bool {
    match name.split_once(':') {
        None => true,
        Some((prefix, local)) => {
            !prefix.is_empty()
                && !local.contains(':')
                && local.chars().next().is_some_and(is_name_start_char)
        }
    }
}

/// `NCName` für einen Namen, der schon `Name` ist.
pub(crate) fn is_ncname(name: &str) -> bool {
    !name.contains(':')
}
