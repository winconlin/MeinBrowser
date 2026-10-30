//! Die Zeichenkodierung eines Dokuments aus seinen Bytes (HTML Standard,
//! Abschnitte „Determining the character encoding“ und „Changing the
//! encoding while parsing“).
//!
//! Vom Algorithmus *encoding sniffing* gibt es die Schritte, für die es
//! Angaben gibt: die Byte-Order-Mark, das Vorab-Lesen (*prescan*) der ersten
//! [`PRESCAN_BYTES`] Bytes und zuletzt die Ersatzkodierung des Aufrufers. Es
//! fehlen die Kodierung aus der Transportschicht (`Content-Type`, kommt mit
//! Fetch), die Wahl des Benutzers, die Kodierung des einbettenden Dokuments
//! und eine Erkennung aus dem Inhalt.

use std::collections::BTreeSet;

use mb_base::DomString;
use mb_encoding::{Encoding, bom_sniff};

#[cfg(test)]
mod tests;

/// Wie viele Bytes das Vorab-Lesen ansieht; der Standard empfiehlt 1024.
pub const PRESCAN_BYTES: usize = 1024;

/// Wie sicher die Kodierung ist (*confidence*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// Ein `meta` im Dokument kann sie noch ändern.
    Tentative,
    Certain,
}

/// *Encoding sniffing algorithm* mit `fallback` als Ersatzkodierung.
pub fn sniff(bytes: &[u8], fallback: Encoding) -> (Encoding, Confidence) {
    if let Some(encoding) = bom_sniff(bytes) {
        return (encoding, Confidence::Certain);
    }
    let start = bytes.get(..PRESCAN_BYTES).unwrap_or(bytes);
    let encoding = prescan(start).unwrap_or(fallback);
    (encoding, Confidence::Tentative)
}

/// *Prescan a byte stream to determine its encoding* über alle `bytes`;
/// das Ende der Bytes ist die Endbedingung.
pub fn prescan(bytes: &[u8]) -> Option<Encoding> {
    // Prescan for UTF-16 XML declarations
    if bytes.starts_with(&[0x3C, 0, 0x3F, 0, 0x78, 0]) {
        return Some(Encoding::Utf16Le);
    }
    if bytes.starts_with(&[0, 0x3C, 0, 0x3F, 0, 0x78]) {
        return Some(Encoding::Utf16Be);
    }
    let mut scanner = Scanner { bytes, position: 0 };
    // Gehen die Bytes aus, endet der Algorithmus mit *get an XML encoding*.
    scanner.run().or_else(|| xml_encoding(bytes))
}

/// *Get an encoding* für einen Attributwert. Zeichen außerhalb von ASCII
/// gehören zu keiner Bezeichnung.
pub(crate) fn encoding_for_label(value: &DomString) -> Option<Encoding> {
    Encoding::for_label(&ascii_bytes(value))
}

/// *Algorithm for extracting a character encoding from a meta element* für
/// den Wert von `content`.
pub(crate) fn content_encoding(value: &DomString) -> Option<Encoding> {
    extract_from_content(&ascii_bytes(value))
}

/// Die Codeeinheiten als Bytes; was nicht ASCII ist, wird 0xFF. Das ist
/// kein ASCII-Leerraum und kein Zeichen, auf das die Algorithmen achten,
/// und keine Bezeichnung enthält es.
fn ascii_bytes(value: &DomString) -> Vec<u8> {
    value
        .code_units()
        .map(|unit| u8::try_from(unit).ok().filter(u8::is_ascii).unwrap_or(0xFF))
        .collect()
}

/// *Change the encoding*: `None`, wenn es bei `current` bleibt und die
/// Kodierung nur sicher wird, sonst die Kodierung, mit der das Dokument neu
/// zu parsen ist. Den Wechsel des Dekodierers mitten im Parsen, den der
/// Standard erlaubt, gibt es nicht; es wird immer neu geparst.
pub(crate) fn change(current: Encoding, new: Encoding) -> Option<Encoding> {
    if matches!(current, Encoding::Utf16Be | Encoding::Utf16Le) {
        return None;
    }
    let new = match new {
        Encoding::Utf16Be | Encoding::Utf16Le => Encoding::Utf8,
        Encoding::XUserDefined => Encoding::Windows1252,
        other => other,
    };
    (new != current).then_some(new)
}

fn is_space(byte: u8) -> bool {
    matches!(byte, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

/// Die Bytes ab einer Stelle. `None` heißt immer, dass die Bytes ausgegangen
/// sind; das Vorab-Lesen bricht dann ab.
struct Scanner<'a> {
    bytes: &'a [u8],
    position: usize,
}

/// Ein Attribut, wie es *get an attribute* liest, mit Kleinbuchstaben in
/// Name und Wert.
struct SniffedAttribute {
    name: Vec<u8>,
    value: Vec<u8>,
}

impl Scanner<'_> {
    fn byte(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn rest(&self) -> &[u8] {
        self.bytes.get(self.position..).unwrap_or_default()
    }

    fn advance(&mut self) {
        self.position = self.position.saturating_add(1);
    }

    /// Setzt die Stelle auf das nächste Byte, für das `found` gilt, ab
    /// `from` Bytes hinter der Stelle; `None`, wenn keines mehr kommt.
    fn advance_to(&mut self, from: usize, found: impl Fn(u8) -> bool) -> Option<()> {
        let start = self.position.checked_add(from)?;
        let offset = self.bytes.get(start..)?.iter().position(|&b| found(b))?;
        self.position = start.checked_add(offset)?;
        Some(())
    }

    /// Die Schleife des Vorab-Lesens: die Kodierung oder `None`, wenn die
    /// Bytes ausgehen.
    fn run(&mut self) -> Option<Encoding> {
        loop {
            let rest = self.rest();
            if rest.starts_with(b"<!--") {
                // Auf das `>` eines `-->`; die Striche dürfen die von `<!--`
                // sein.
                let end = rest.get(2..)?.windows(3).position(|w| w == b"-->")?;
                self.position = self.position.checked_add(end)?.checked_add(4)?;
            } else if is_meta_start(rest) {
                self.position = self.position.checked_add(5)?;
                if let Some(encoding) = self.meta()? {
                    return Some(encoding);
                }
            } else if is_tag_start(rest) {
                self.advance_to(0, |b| is_space(b) || b == b'>')?;
                while self.attribute()?.is_some() {}
            } else if rest.starts_with(b"<!") || rest.starts_with(b"</") || rest.starts_with(b"<?")
            {
                self.advance_to(1, |b| b == b'>')?;
            } else {
                // Jedes andere Byte bleibt unbeachtet; das Ende der Bytes
                // bricht ab.
                self.byte()?;
            }
            // Next byte
            self.advance();
        }
    }

    /// Die Schritte für `<meta`, die Stelle auf dem Leerraum oder `/` danach.
    /// `Some(None)`, wenn das Element keine Kodierung angibt.
    fn meta(&mut self) -> Option<Option<Encoding>> {
        let mut names = BTreeSet::new();
        let mut got_pragma = false;
        let mut need_pragma = None;
        // `None` ist *null*, `Some(None)` *failure*.
        let mut charset: Option<Option<Encoding>> = None;
        while let Some(attribute) = self.attribute()? {
            if names.contains(&attribute.name) {
                continue;
            }
            match attribute.name.as_slice() {
                b"http-equiv" => {
                    if attribute.value == b"content-type" {
                        got_pragma = true;
                    }
                }
                b"content" => {
                    if let Some(encoding) = extract_from_content(&attribute.value)
                        && charset.is_none()
                    {
                        charset = Some(Some(encoding));
                        need_pragma = Some(true);
                    }
                }
                b"charset" => {
                    charset = Some(Encoding::for_label(&attribute.value));
                    need_pragma = Some(false);
                }
                _ => {}
            }
            names.insert(attribute.name);
        }
        // Processing
        let Some(need_pragma) = need_pragma else {
            return Some(None);
        };
        if need_pragma && !got_pragma {
            return Some(None);
        }
        let encoding = match charset.flatten() {
            None => return Some(None),
            Some(Encoding::Utf16Be | Encoding::Utf16Le) => Encoding::Utf8,
            Some(Encoding::XUserDefined) => Encoding::Windows1252,
            Some(other) => other,
        };
        Some(Some(encoding))
    }

    /// *Get an attribute*: `Some(None)`, wenn es keins mehr gibt, die Stelle
    /// dann auf dem `>`.
    fn attribute(&mut self) -> Option<Option<SniffedAttribute>> {
        while matches!(self.byte()?, b if is_space(b) || b == b'/') {
            self.advance();
        }
        if self.byte()? == b'>' {
            return Some(None);
        }
        let mut name = Vec::new();
        let without_value = |name| {
            Some(Some(SniffedAttribute {
                name,
                value: Vec::new(),
            }))
        };
        loop {
            match self.byte()? {
                b'=' if !name.is_empty() => {
                    self.advance();
                    return self.value(name);
                }
                b if is_space(b) => break,
                b'/' | b'>' => return without_value(name),
                b => name.push(b.to_ascii_lowercase()),
            }
            self.advance();
        }
        // Spaces
        while is_space(self.byte()?) {
            self.advance();
        }
        if self.byte()? != b'=' {
            return without_value(name);
        }
        self.advance();
        self.value(name)
    }

    /// Der Schritt „Value“ von *get an attribute*.
    fn value(&mut self, name: Vec<u8>) -> Option<Option<SniffedAttribute>> {
        let mut value = Vec::new();
        while is_space(self.byte()?) {
            self.advance();
        }
        match self.byte()? {
            quote @ (b'"' | b'\'') => loop {
                // Quote loop
                self.advance();
                let b = self.byte()?;
                if b == quote {
                    self.advance();
                    return Some(Some(SniffedAttribute { name, value }));
                }
                value.push(b.to_ascii_lowercase());
            },
            b'>' => return Some(Some(SniffedAttribute { name, value })),
            b => {
                value.push(b.to_ascii_lowercase());
                self.advance();
            }
        }
        loop {
            match self.byte()? {
                b if is_space(b) || b == b'>' => {
                    return Some(Some(SniffedAttribute { name, value }));
                }
                b => value.push(b.to_ascii_lowercase()),
            }
            self.advance();
        }
    }
}

/// `<meta` ohne Rücksicht auf Groß- und Kleinschreibung, dann Leerraum
/// oder `/`.
fn is_meta_start(rest: &[u8]) -> bool {
    rest.get(..5)
        .is_some_and(|start| start.eq_ignore_ascii_case(b"<meta"))
        && rest.get(5).is_some_and(|&b| is_space(b) || b == b'/')
}

/// `<`, vielleicht `/`, dann ein ASCII-Buchstabe.
fn is_tag_start(rest: &[u8]) -> bool {
    let letter = match rest {
        [b'<', b'/', letter, ..] => letter,
        [b'<', letter, ..] => letter,
        _ => return false,
    };
    letter.is_ascii_alphabetic()
}

/// *Get an XML encoding*.
fn xml_encoding(bytes: &[u8]) -> Option<Encoding> {
    if !bytes.starts_with(b"<?xml") {
        return None;
    }
    // Nur `encoding` muss vor dem ersten `>` stehen, die Schritte danach
    // gehen darüber hinaus.
    let declaration_end = bytes.iter().position(|&b| b == b'>')?;
    let found = bytes
        .get(..declaration_end)?
        .windows(8)
        .position(|w| w == b"encoding")?;
    // Hinter Leerraum und Steuerzeichen.
    let skip = |position: usize| {
        let count = bytes
            .get(position..)?
            .iter()
            .take_while(|&&b| b <= 0x20)
            .count();
        position.checked_add(count)
    };
    let mut position = skip(found.checked_add(8)?)?;
    if bytes.get(position) != Some(&b'=') {
        return None;
    }
    position = skip(position.checked_add(1)?)?;
    let quote = *bytes.get(position)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let rest = bytes.get(position.checked_add(1)?..)?;
    let label = rest.get(..rest.iter().position(|&b| b == quote)?)?;
    if label.iter().any(|&b| b <= 0x20) {
        return None;
    }
    match Encoding::for_label(label)? {
        Encoding::Utf16Be | Encoding::Utf16Le => Some(Encoding::Utf8),
        other => Some(other),
    }
}

/// *Algorithm for extracting a character encoding from a meta element*.
fn extract_from_content(s: &[u8]) -> Option<Encoding> {
    let mut position = 0;
    loop {
        // Loop
        let rest = s.get(position..)?;
        let found = rest
            .windows(7)
            .position(|w| w.eq_ignore_ascii_case(b"charset"))?;
        position = position.checked_add(found)?.checked_add(7)?;
        position = position.checked_add(skip_ascii_whitespace(s.get(position..)?))?;
        if s.get(position) != Some(&b'=') {
            continue;
        }
        position = position.checked_add(1)?;
        position = position.checked_add(skip_ascii_whitespace(s.get(position..)?))?;
        let rest = s.get(position..)?;
        return match *rest.first()? {
            quote @ (b'"' | b'\'') => {
                let inner = rest.get(1..)?;
                let end = inner.iter().position(|&b| b == quote)?;
                Encoding::for_label(inner.get(..end)?)
            }
            _ => {
                let end = rest
                    .iter()
                    .position(|&b| b.is_ascii_whitespace() || b == b';')
                    .unwrap_or(rest.len());
                Encoding::for_label(rest.get(..end)?)
            }
        };
    }
}

fn skip_ascii_whitespace(bytes: &[u8]) -> usize {
    bytes.iter().take_while(|b| b.is_ascii_whitespace()).count()
}
