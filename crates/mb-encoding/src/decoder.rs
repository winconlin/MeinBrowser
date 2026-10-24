//! Die Dekodierer aller Kodierungen, Schritt für Schritt nach den Algorithmen
//! des Encoding Standard.
//!
//! Jeder Dekodierer ist ein *handler*, der ein Byte oder das Ende der Eingabe
//! (*end-of-queue*) bekommt. Das *restore* der Spezifikation legt Bytes auf
//! einen kleinen Stapel, der vor der übrigen Eingabe gelesen wird.

use crate::Encoding;
use crate::index::{code_point, gb18030_ranges_code_point};
use crate::tables::{INDEX_BIG5, INDEX_EUC_KR, INDEX_GB18030, INDEX_JIS0208, INDEX_JIS0212};

/// Ein Fehler beim Dekodieren im Modus „fatal“.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeError {
    pub encoding: Encoding,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ungültige Bytefolge für {}", self.encoding.name())
    }
}

impl std::error::Error for DecodeError {}

/// Ein Dekodierer, der Eingabe auch in Stücken verarbeitet.
#[derive(Clone, Debug)]
pub struct Decoder {
    encoding: Encoding,
    state: State,
    /// Zurückgelegte Bytes, das oberste wird zuerst gelesen.
    restored: Vec<u8>,
}

enum Item {
    Byte(u8),
    End,
}

enum Step {
    Finished,
    Output(u32),
    /// Big5 liefert für vier Zeiger zwei Codepunkte.
    Output2(u32, u32),
    Error,
    Continue,
}

struct Queue<'a> {
    restored: &'a mut Vec<u8>,
    input: &'a [u8],
    position: usize,
    last: bool,
}

impl Queue<'_> {
    /// Das nächste Element, oder `None`, wenn das Stück zu Ende ist und noch
    /// mehr Eingabe folgt.
    fn read(&mut self) -> Option<Item> {
        if let Some(byte) = self.restored.pop() {
            return Some(Item::Byte(byte));
        }
        if let Some(&byte) = self.input.get(self.position) {
            self.position += 1;
            return Some(Item::Byte(byte));
        }
        self.last.then_some(Item::End)
    }

    /// *Restore*: Die Bytes kommen in dieser Reihenfolge als Nächstes.
    fn restore(&mut self, bytes: &[u8]) {
        self.restored.extend(bytes.iter().rev());
    }
}

impl Decoder {
    /// Ein neuer Dekodierer im Anfangszustand.
    pub fn new(encoding: Encoding) -> Decoder {
        Decoder {
            encoding,
            state: State::new(encoding),
            restored: Vec::new(),
        }
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Dekodiert ein Stück im Modus „replacement“: Fehler werden zu U+FFFD.
    /// Mit `last` endet die Eingabe nach diesem Stück.
    pub fn decode(&mut self, input: &[u8], last: bool, output: &mut String) {
        // Im Modus „replacement“ gibt es keinen Fehler.
        let _ = self.run(input, last, output, false);
    }

    /// Dekodiert ein Stück im Modus „fatal“: Beim ersten Fehler ist Schluss.
    /// Danach ist der Dekodierer nicht mehr zu gebrauchen.
    pub fn decode_or_fail(
        &mut self,
        input: &[u8],
        last: bool,
        output: &mut String,
    ) -> Result<(), DecodeError> {
        self.run(input, last, output, true)
    }

    /// *Process a queue* für ein Stück der Eingabe.
    fn run(
        &mut self,
        input: &[u8],
        last: bool,
        output: &mut String,
        fatal: bool,
    ) -> Result<(), DecodeError> {
        let mut queue = Queue {
            restored: &mut self.restored,
            input,
            position: 0,
            last,
        };
        let error = DecodeError {
            encoding: self.encoding,
        };
        while let Some(item) = queue.read() {
            let result = match self.state.handle(&mut queue, item) {
                Step::Finished => return Ok(()),
                Step::Continue => Ok(()),
                Step::Output(code_point) => push(output, code_point),
                Step::Output2(first, second) => {
                    push(output, first).and_then(|()| push(output, second))
                }
                Step::Error => Err(()),
            };
            if result.is_err() {
                if fatal {
                    return Err(error);
                }
                output.push('\u{FFFD}');
            }
        }
        Ok(())
    }
}

/// Hängt einen Codepunkt an. Die Dekodierer liefern laut Spezifikation nur
/// Skalarwerte; sonst gilt es als Fehler.
fn push(output: &mut String, code_point: u32) -> Result<(), ()> {
    output.push(char::from_u32(code_point).ok_or(())?);
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Iso2022JpState {
    Ascii,
    Roman,
    Katakana,
    LeadingByte,
    TrailingByte,
    EscapeStart,
    Escape,
}

#[derive(Clone, Debug)]
enum State {
    Utf8 {
        code_point: u32,
        bytes_seen: u8,
        bytes_needed: u8,
        lower_boundary: u8,
        upper_boundary: u8,
    },
    Utf16 {
        big_endian: bool,
        leading_byte: Option<u8>,
        leading_surrogate: Option<u16>,
    },
    SingleByte(&'static [u16; 128]),
    Gb18030 {
        first: u8,
        second: u8,
        third: u8,
    },
    Big5 {
        leading: u8,
    },
    EucJp {
        jis0212: bool,
        leading: u8,
    },
    Iso2022Jp {
        state: Iso2022JpState,
        output_state: Iso2022JpState,
        leading: u8,
        output: bool,
    },
    ShiftJis {
        leading: u8,
    },
    EucKr {
        leading: u8,
    },
    Replacement {
        error_returned: bool,
    },
    XUserDefined,
}

impl State {
    fn new(encoding: Encoding) -> State {
        match encoding {
            Encoding::Utf8 => State::Utf8 {
                code_point: 0,
                bytes_seen: 0,
                bytes_needed: 0,
                lower_boundary: 0x80,
                upper_boundary: 0xBF,
            },
            Encoding::Utf16Be | Encoding::Utf16Le => State::Utf16 {
                big_endian: encoding == Encoding::Utf16Be,
                leading_byte: None,
                leading_surrogate: None,
            },
            Encoding::Gbk | Encoding::Gb18030 => State::Gb18030 {
                first: 0,
                second: 0,
                third: 0,
            },
            Encoding::Big5 => State::Big5 { leading: 0 },
            Encoding::EucJp => State::EucJp {
                jis0212: false,
                leading: 0,
            },
            Encoding::Iso2022Jp => State::Iso2022Jp {
                state: Iso2022JpState::Ascii,
                output_state: Iso2022JpState::Ascii,
                leading: 0,
                output: false,
            },
            Encoding::ShiftJis => State::ShiftJis { leading: 0 },
            Encoding::EucKr => State::EucKr { leading: 0 },
            Encoding::Replacement => State::Replacement {
                error_returned: false,
            },
            Encoding::XUserDefined => State::XUserDefined,
            // Alle übrigen sind Einzelbyte-Kodierungen (ein Test prüft das).
            _ => match crate::tables::single_byte_index(encoding) {
                Some(index) => State::SingleByte(index),
                None => State::Replacement {
                    error_returned: false,
                },
            },
        }
    }

    fn handle(&mut self, queue: &mut Queue<'_>, item: Item) -> Step {
        match self {
            State::Utf8 {
                code_point,
                bytes_seen,
                bytes_needed,
                lower_boundary,
                upper_boundary,
            } => {
                let Item::Byte(byte) = item else {
                    if *bytes_needed != 0 {
                        // Die Spezifikation setzt nur die Zahl der fehlenden
                        // Bytes zurück; danach kommt nichts mehr, der Rest
                        // ist also gleichgültig.
                        *self = State::new(Encoding::Utf8);
                        return Step::Error;
                    }
                    return Step::Finished;
                };
                if *bytes_needed == 0 {
                    match byte {
                        0x00..=0x7F => return Step::Output(u32::from(byte)),
                        0xC2..=0xDF => {
                            *bytes_needed = 1;
                            *code_point = u32::from(byte & 0x1F);
                        }
                        0xE0..=0xEF => {
                            if byte == 0xE0 {
                                *lower_boundary = 0xA0;
                            }
                            if byte == 0xED {
                                *upper_boundary = 0x9F;
                            }
                            *bytes_needed = 2;
                            *code_point = u32::from(byte & 0xF);
                        }
                        0xF0..=0xF4 => {
                            if byte == 0xF0 {
                                *lower_boundary = 0x90;
                            }
                            if byte == 0xF4 {
                                *upper_boundary = 0x8F;
                            }
                            *bytes_needed = 3;
                            *code_point = u32::from(byte & 0x7);
                        }
                        _ => return Step::Error,
                    }
                    return Step::Continue;
                }
                if !(*lower_boundary..=*upper_boundary).contains(&byte) {
                    *self = State::new(Encoding::Utf8);
                    queue.restore(&[byte]);
                    return Step::Error;
                }
                *lower_boundary = 0x80;
                *upper_boundary = 0xBF;
                *code_point = (*code_point << 6) | u32::from(byte & 0x3F);
                *bytes_seen += 1;
                if bytes_seen != bytes_needed {
                    return Step::Continue;
                }
                let result = *code_point;
                *code_point = 0;
                *bytes_needed = 0;
                *bytes_seen = 0;
                Step::Output(result)
            }
            State::Utf16 {
                big_endian,
                leading_byte,
                leading_surrogate,
            } => {
                let Item::Byte(byte) = item else {
                    if leading_byte.is_some() || leading_surrogate.is_some() {
                        *leading_byte = None;
                        *leading_surrogate = None;
                        return Step::Error;
                    }
                    return Step::Finished;
                };
                let Some(leading) = leading_byte.take() else {
                    *leading_byte = Some(byte);
                    return Step::Continue;
                };
                let code_unit = if *big_endian {
                    u16::from_be_bytes([leading, byte])
                } else {
                    u16::from_le_bytes([leading, byte])
                };
                if let Some(leading_surrogate) = leading_surrogate.take() {
                    if (0xDC00..=0xDFFF).contains(&code_unit) {
                        return Step::Output(
                            0x10000
                                + ((u32::from(leading_surrogate) - 0xD800) << 10)
                                + (u32::from(code_unit) - 0xDC00),
                        );
                    }
                    let bytes = if *big_endian {
                        code_unit.to_be_bytes()
                    } else {
                        code_unit.to_le_bytes()
                    };
                    queue.restore(&bytes);
                    return Step::Error;
                }
                if (0xD800..=0xDBFF).contains(&code_unit) {
                    *leading_surrogate = Some(code_unit);
                    return Step::Continue;
                }
                if (0xDC00..=0xDFFF).contains(&code_unit) {
                    return Step::Error;
                }
                Step::Output(u32::from(code_unit))
            }
            State::SingleByte(index) => match item {
                Item::End => Step::Finished,
                Item::Byte(byte @ 0x00..=0x7F) => Step::Output(u32::from(byte)),
                Item::Byte(byte) => {
                    code_point(&index[..], u32::from(byte) - 0x80).map_or(Step::Error, Step::Output)
                }
            },
            State::XUserDefined => match item {
                Item::End => Step::Finished,
                Item::Byte(byte @ 0x00..=0x7F) => Step::Output(u32::from(byte)),
                Item::Byte(byte) => Step::Output(0xF780 + u32::from(byte) - 0x80),
            },
            State::Replacement { error_returned } => match item {
                Item::End => Step::Finished,
                Item::Byte(_) if !*error_returned => {
                    *error_returned = true;
                    Step::Error
                }
                Item::Byte(_) => Step::Finished,
            },
            State::Gb18030 {
                first,
                second,
                third,
            } => {
                let Item::Byte(byte) = item else {
                    if (*first, *second, *third) == (0, 0, 0) {
                        return Step::Finished;
                    }
                    (*first, *second, *third) = (0, 0, 0);
                    return Step::Error;
                };
                if *third != 0 {
                    if !(0x30..=0x39).contains(&byte) {
                        queue.restore(&[*second, *third, byte]);
                        (*first, *second, *third) = (0, 0, 0);
                        return Step::Error;
                    }
                    let pointer = (u32::from(*first) - 0x81) * (10 * 126 * 10)
                        + (u32::from(*second) - 0x30) * (10 * 126)
                        + (u32::from(*third) - 0x81) * 10
                        + u32::from(byte)
                        - 0x30;
                    (*first, *second, *third) = (0, 0, 0);
                    return gb18030_ranges_code_point(pointer).map_or(Step::Error, Step::Output);
                }
                if *second != 0 {
                    if (0x81..=0xFE).contains(&byte) {
                        *third = byte;
                        return Step::Continue;
                    }
                    queue.restore(&[*second, byte]);
                    (*first, *second) = (0, 0);
                    return Step::Error;
                }
                if *first != 0 {
                    if (0x30..=0x39).contains(&byte) {
                        *second = byte;
                        return Step::Continue;
                    }
                    let leading = u32::from(*first);
                    *first = 0;
                    let offset = if byte < 0x7F { 0x40 } else { 0x41 };
                    let pointer = (matches!(byte, 0x40..=0x7E | 0x80..=0xFE))
                        .then(|| (leading - 0x81) * 190 + (u32::from(byte) - offset));
                    if let Some(code_point) =
                        pointer.and_then(|pointer| code_point(&INDEX_GB18030, pointer))
                    {
                        return Step::Output(code_point);
                    }
                    if byte.is_ascii() {
                        queue.restore(&[byte]);
                    }
                    return Step::Error;
                }
                match byte {
                    0x00..=0x7F => Step::Output(u32::from(byte)),
                    0x80 => Step::Output(0x20AC),
                    0x81..=0xFE => {
                        *first = byte;
                        Step::Continue
                    }
                    0xFF => Step::Error,
                }
            }
            State::Big5 { leading } => {
                let Item::Byte(byte) = item else {
                    if *leading != 0 {
                        *leading = 0;
                        return Step::Error;
                    }
                    return Step::Finished;
                };
                if *leading != 0 {
                    let lead = u32::from(*leading);
                    *leading = 0;
                    let offset = if byte < 0x7F { 0x40 } else { 0x62 };
                    let pointer = (matches!(byte, 0x40..=0x7E | 0xA1..=0xFE))
                        .then(|| (lead - 0x81) * 157 + (u32::from(byte) - offset));
                    match pointer {
                        Some(1133) => return Step::Output2(0x00CA, 0x0304),
                        Some(1135) => return Step::Output2(0x00CA, 0x030C),
                        Some(1164) => return Step::Output2(0x00EA, 0x0304),
                        Some(1166) => return Step::Output2(0x00EA, 0x030C),
                        _ => {}
                    }
                    if let Some(code_point) =
                        pointer.and_then(|pointer| code_point(&INDEX_BIG5, pointer))
                    {
                        return Step::Output(code_point);
                    }
                    if byte.is_ascii() {
                        queue.restore(&[byte]);
                    }
                    return Step::Error;
                }
                match byte {
                    0x00..=0x7F => Step::Output(u32::from(byte)),
                    0x81..=0xFE => {
                        *leading = byte;
                        Step::Continue
                    }
                    _ => Step::Error,
                }
            }
            State::EucJp { jis0212, leading } => {
                let Item::Byte(byte) = item else {
                    if *leading != 0 {
                        *leading = 0;
                        *jis0212 = false;
                        return Step::Error;
                    }
                    return Step::Finished;
                };
                if *leading == 0x8E && (0xA1..=0xDF).contains(&byte) {
                    *leading = 0;
                    return Step::Output(0xFF61 - 0xA1 + u32::from(byte));
                }
                if *leading == 0x8F && (0xA1..=0xFE).contains(&byte) {
                    *jis0212 = true;
                    *leading = byte;
                    return Step::Continue;
                }
                if *leading != 0 {
                    let lead = *leading;
                    *leading = 0;
                    let mut result = None;
                    if (0xA1..=0xFE).contains(&lead) && (0xA1..=0xFE).contains(&byte) {
                        let pointer = (u32::from(lead) - 0xA1) * 94 + u32::from(byte) - 0xA1;
                        result = if *jis0212 {
                            code_point(&INDEX_JIS0212, pointer)
                        } else {
                            code_point(&INDEX_JIS0208, pointer)
                        };
                    }
                    *jis0212 = false;
                    if let Some(code_point) = result {
                        return Step::Output(code_point);
                    }
                    if byte.is_ascii() {
                        queue.restore(&[byte]);
                    }
                    return Step::Error;
                }
                match byte {
                    0x00..=0x7F => Step::Output(u32::from(byte)),
                    0x8E | 0x8F | 0xA1..=0xFE => {
                        *leading = byte;
                        Step::Continue
                    }
                    _ => Step::Error,
                }
            }
            State::Iso2022Jp {
                state,
                output_state,
                leading,
                output,
            } => iso_2022_jp(queue, item, state, output_state, leading, output),
            State::ShiftJis { leading } => {
                let Item::Byte(byte) = item else {
                    if *leading != 0 {
                        *leading = 0;
                        return Step::Error;
                    }
                    return Step::Finished;
                };
                if *leading != 0 {
                    let lead = u32::from(*leading);
                    *leading = 0;
                    let offset = if byte < 0x7F { 0x40 } else { 0x41 };
                    let leading_offset = if lead < 0xA0 { 0x81 } else { 0xC1 };
                    let pointer = (matches!(byte, 0x40..=0x7E | 0x80..=0xFC))
                        .then(|| (lead - leading_offset) * 188 + u32::from(byte) - offset);
                    if let Some(pointer) = pointer
                        && (8836..=10715).contains(&pointer)
                    {
                        return Step::Output(0xE000 - 8836 + pointer);
                    }
                    if let Some(code_point) =
                        pointer.and_then(|pointer| code_point(&INDEX_JIS0208, pointer))
                    {
                        return Step::Output(code_point);
                    }
                    if byte.is_ascii() {
                        queue.restore(&[byte]);
                    }
                    return Step::Error;
                }
                match byte {
                    0x00..=0x80 => Step::Output(u32::from(byte)),
                    0xA1..=0xDF => Step::Output(0xFF61 - 0xA1 + u32::from(byte)),
                    0x81..=0x9F | 0xE0..=0xFC => {
                        *leading = byte;
                        Step::Continue
                    }
                    _ => Step::Error,
                }
            }
            State::EucKr { leading } => {
                let Item::Byte(byte) = item else {
                    if *leading != 0 {
                        *leading = 0;
                        return Step::Error;
                    }
                    return Step::Finished;
                };
                if *leading != 0 {
                    let lead = u32::from(*leading);
                    *leading = 0;
                    let pointer = (0x41..=0xFE)
                        .contains(&byte)
                        .then(|| (lead - 0x81) * 190 + (u32::from(byte) - 0x41));
                    if let Some(code_point) =
                        pointer.and_then(|pointer| code_point(&INDEX_EUC_KR, pointer))
                    {
                        return Step::Output(code_point);
                    }
                    if byte.is_ascii() {
                        queue.restore(&[byte]);
                    }
                    return Step::Error;
                }
                match byte {
                    0x00..=0x7F => Step::Output(u32::from(byte)),
                    0x81..=0xFE => {
                        *leading = byte;
                        Step::Continue
                    }
                    _ => Step::Error,
                }
            }
        }
    }
}

/// Der Dekodierer für ISO-2022-JP mit seinen Zuständen.
fn iso_2022_jp(
    queue: &mut Queue<'_>,
    item: Item,
    state: &mut Iso2022JpState,
    output_state: &mut Iso2022JpState,
    leading: &mut u8,
    output: &mut bool,
) -> Step {
    use Iso2022JpState as S;
    match *state {
        S::Ascii => match item {
            Item::Byte(0x1B) => {
                *state = S::EscapeStart;
                Step::Continue
            }
            Item::Byte(byte @ 0x00..=0x7F) if byte != 0x0E && byte != 0x0F => {
                *output = false;
                Step::Output(u32::from(byte))
            }
            Item::End => Step::Finished,
            Item::Byte(_) => {
                *output = false;
                Step::Error
            }
        },
        S::Roman => match item {
            Item::Byte(0x1B) => {
                *state = S::EscapeStart;
                Step::Continue
            }
            Item::Byte(0x5C) => {
                *output = false;
                Step::Output(0x00A5)
            }
            Item::Byte(0x7E) => {
                *output = false;
                Step::Output(0x203E)
            }
            Item::Byte(byte @ 0x00..=0x7F) if byte != 0x0E && byte != 0x0F => {
                *output = false;
                Step::Output(u32::from(byte))
            }
            Item::End => Step::Finished,
            Item::Byte(_) => {
                *output = false;
                Step::Error
            }
        },
        S::Katakana => match item {
            Item::Byte(0x1B) => {
                *state = S::EscapeStart;
                Step::Continue
            }
            Item::Byte(byte @ 0x21..=0x5F) => {
                *output = false;
                Step::Output(0xFF61 - 0x21 + u32::from(byte))
            }
            Item::End => Step::Finished,
            Item::Byte(_) => {
                *output = false;
                Step::Error
            }
        },
        S::LeadingByte => match item {
            Item::Byte(0x1B) => {
                *state = S::EscapeStart;
                Step::Continue
            }
            Item::Byte(byte @ 0x21..=0x7E) => {
                *output = false;
                *leading = byte;
                *state = S::TrailingByte;
                Step::Continue
            }
            Item::End => Step::Finished,
            Item::Byte(_) => {
                *output = false;
                Step::Error
            }
        },
        S::TrailingByte => match item {
            Item::Byte(0x1B) => {
                *state = S::EscapeStart;
                Step::Error
            }
            Item::Byte(byte @ 0x21..=0x7E) => {
                *state = S::LeadingByte;
                let pointer = (u32::from(*leading) - 0x21) * 94 + u32::from(byte) - 0x21;
                code_point(&INDEX_JIS0208, pointer).map_or(Step::Error, Step::Output)
            }
            Item::End | Item::Byte(_) => {
                *state = S::LeadingByte;
                Step::Error
            }
        },
        S::EscapeStart => {
            if let Item::Byte(byte @ (0x24 | 0x28)) = item {
                *leading = byte;
                *state = S::Escape;
                return Step::Continue;
            }
            if let Item::Byte(byte) = item {
                queue.restore(&[byte]);
            }
            *output = false;
            *state = *output_state;
            Step::Error
        }
        S::Escape => {
            let lead = *leading;
            *leading = 0;
            let next = match (lead, &item) {
                (0x28, Item::Byte(0x42)) => Some(S::Ascii),
                (0x28, Item::Byte(0x4A)) => Some(S::Roman),
                (0x28, Item::Byte(0x49)) => Some(S::Katakana),
                (0x24, Item::Byte(0x40 | 0x42)) => Some(S::LeadingByte),
                _ => None,
            };
            if let Some(next) = next {
                *state = next;
                *output_state = next;
                let previous = std::mem::replace(output, true);
                return if previous {
                    Step::Error
                } else {
                    Step::Continue
                };
            }
            match item {
                Item::End => queue.restore(&[lead]),
                Item::Byte(byte) => queue.restore(&[lead, byte]),
            }
            *output = false;
            *state = *output_state;
            Step::Error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(encoding: Encoding, bytes: &[u8]) -> String {
        let mut output = String::new();
        Decoder::new(encoding).decode(bytes, true, &mut output);
        output
    }

    /// Byte für Byte, damit Zustand und zurückgelegte Bytes über die Grenzen
    /// der Stücke gehen müssen.
    fn decode_bytewise(encoding: Encoding, bytes: &[u8]) -> String {
        let mut decoder = Decoder::new(encoding);
        let mut output = String::new();
        for byte in bytes {
            decoder.decode(std::slice::from_ref(byte), false, &mut output);
        }
        decoder.decode(&[], true, &mut output);
        output
    }

    /// Bytes, die oft Grenzen der Algorithmen treffen.
    const BOUNDARIES: [u8; 25] = [
        0x00, 0x41, 0x7F, 0x80, 0x8F, 0x90, 0x9F, 0xA0, 0xBF, 0xC0, 0xC1, 0xC2, 0xDF, 0xE0, 0xE1,
        0xEC, 0xED, 0xEE, 0xEF, 0xF0, 0xF1, 0xF3, 0xF4, 0xF5, 0xFF,
    ];

    #[test]
    fn utf8_matches_std() {
        // Rusts verlustbehaftete Umwandlung ersetzt wie der Standard jeden
        // größten ungültigen Teil (*maximal subpart*) durch ein U+FFFD.
        let mut inputs: Vec<Vec<u8>> = Vec::new();
        for first in 0..=255u8 {
            inputs.push(vec![first]);
            for second in 0..=255u8 {
                inputs.push(vec![first, second]);
            }
        }
        for &first in &BOUNDARIES {
            for &second in &BOUNDARIES {
                for &third in &BOUNDARIES {
                    inputs.push(vec![first, second, third]);
                    for &fourth in &BOUNDARIES {
                        inputs.push(vec![first, second, third, fourth]);
                    }
                }
            }
        }
        for input in &inputs {
            let expected = String::from_utf8_lossy(input);
            assert_eq!(decode(Encoding::Utf8, input), expected, "{input:02X?}");
            if input.len() <= 3 {
                assert_eq!(
                    decode_bytewise(Encoding::Utf8, input),
                    expected,
                    "{input:02X?}"
                );
            }
        }
    }

    #[test]
    fn utf16_matches_std() {
        const UNITS: [u16; 7] = [0x0041, 0xD800, 0xDBFF, 0xDC00, 0xDFFF, 0xE000, 0xFFFD];
        // Alle Folgen aus bis zu vier dieser Code-Units.
        let mut sequences: Vec<Vec<u16>> = Vec::new();
        for length in 1..=4 {
            for number in 0..UNITS.len().pow(length) {
                let mut rest = number;
                let mut units = Vec::new();
                for _ in 0..length {
                    units.extend(UNITS.get(rest % UNITS.len()));
                    rest /= UNITS.len();
                }
                sequences.push(units);
            }
        }
        for units in &sequences {
            let expected: String = char::decode_utf16(units.iter().copied())
                .map(|result| result.unwrap_or('\u{FFFD}'))
                .collect();
            let big: Vec<u8> = units.iter().flat_map(|unit| unit.to_be_bytes()).collect();
            let little: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
            assert_eq!(decode(Encoding::Utf16Be, &big), expected, "{units:04X?}");
            assert_eq!(decode(Encoding::Utf16Le, &little), expected, "{units:04X?}");
            assert_eq!(decode_bytewise(Encoding::Utf16Le, &little), expected);
        }
        // Ein übrig gebliebenes Byte am Ende.
        assert_eq!(decode(Encoding::Utf16Le, b"a\x00b"), "a\u{FFFD}");
    }

    #[test]
    fn single_byte() {
        assert_eq!(
            decode(Encoding::Windows1252, b"a\x80\x81\x9F\xE0"),
            "a€\u{81}Ÿà"
        );
        // ISO-8859-8-I teilt den Index mit ISO-8859-8.
        assert_eq!(decode(Encoding::Iso8859_8I, b"\xE0"), "א");
        assert_eq!(decode(Encoding::Iso8859_8, b"\xE0"), "א");
        // Lücke im Index.
        assert_eq!(decode(Encoding::Iso8859_3, b"\xA5"), "\u{FFFD}");
        assert_eq!(
            decode(Encoding::XUserDefined, b"A\x80\xFF"),
            "A\u{F780}\u{F7FF}"
        );
    }

    #[test]
    fn replacement() {
        assert_eq!(decode(Encoding::Replacement, b""), "");
        assert_eq!(decode(Encoding::Replacement, b"abc"), "\u{FFFD}");
        assert_eq!(decode_bytewise(Encoding::Replacement, b"abc"), "\u{FFFD}");
    }

    #[test]
    fn gb18030() {
        assert_eq!(decode(Encoding::Gb18030, b"\x80\xFF"), "€\u{FFFD}");
        assert_eq!(decode(Encoding::Gbk, b"\xA2\xE3"), "€");
        assert_eq!(decode(Encoding::Gb18030, b"\xA3\xA0"), "\u{3000}");
        assert_eq!(decode(Encoding::Gb18030, b"\xA6\xD9"), "\u{FE10}");
        assert_eq!(decode(Encoding::Gb18030, b"\x81\x30\x81\x30"), "\u{80}");
        assert_eq!(decode(Encoding::Gb18030, b"\x81\x35\xF4\x37"), "\u{E7C7}");
        assert_eq!(decode(Encoding::Gb18030, b"\x84\x31\xA4\x39"), "\u{FFFF}");
        assert_eq!(decode(Encoding::Gb18030, b"\x84\x31\xA5\x30"), "\u{FFFD}");
        assert_eq!(decode(Encoding::Gb18030, b"\x90\x30\x81\x30"), "\u{10000}");
        assert_eq!(decode(Encoding::Gb18030, b"\xE3\x32\x9A\x35"), "\u{10FFFF}");
        assert_eq!(decode(Encoding::Gb18030, b"\xE3\x32\x9A\x36"), "\u{FFFD}");
        // Zurückgelegte Bytes werden neu gelesen.
        assert_eq!(decode(Encoding::Gb18030, b"\x81 "), "\u{FFFD} ");
        assert_eq!(decode(Encoding::Gb18030, b"\x810 "), "\u{FFFD}0 ");
        assert_eq!(
            decode(Encoding::Gb18030, b"\x810\x81 "),
            "\u{FFFD}0\u{FFFD} "
        );
        assert_eq!(decode(Encoding::Gb18030, b"\x810\x81"), "\u{FFFD}");
    }

    #[test]
    fn big5() {
        assert_eq!(
            decode(Encoding::Big5, b"\x88\x62\x88\x64"),
            "\u{CA}\u{304}\u{CA}\u{30C}"
        );
        assert_eq!(
            decode(Encoding::Big5, b"\x88\xA3\x88\xA5"),
            "\u{EA}\u{304}\u{EA}\u{30C}"
        );
        assert_eq!(
            decode(Encoding::Big5, b"\x81 \xFE\xFF\x80"),
            "\u{FFFD} \u{FFFD}\u{FFFD}"
        );
        assert_eq!(decode(Encoding::Big5, b"\x81"), "\u{FFFD}");
    }

    #[test]
    fn euc_jp() {
        assert_eq!(decode(Encoding::EucJp, b"\xA4\xA2\x8E\xA1"), "あ\u{FF61}");
        assert_eq!(decode(Encoding::EucJp, b"\x8F\xA2\xAF"), "\u{2D8}");
        assert_eq!(decode(Encoding::EucJp, b"\xA4 "), "\u{FFFD} ");
        assert_eq!(decode(Encoding::EucJp, b"\x8F\xA2"), "\u{FFFD}");
    }

    #[test]
    fn iso_2022_jp() {
        assert_eq!(
            decode(Encoding::Iso2022Jp, b"a\x1B$B\x24\x22\x1B(Bb"),
            "aあb"
        );
        assert_eq!(decode(Encoding::Iso2022Jp, b"\x1B(J\x5C\x7E"), "¥‾");
        assert_eq!(decode(Encoding::Iso2022Jp, b"\x1B(I\x21"), "\u{FF61}");
        assert_eq!(decode(Encoding::Iso2022Jp, b"\x0E"), "\u{FFFD}");
        // Ungültige oder abgebrochene Escape-Folgen.
        assert_eq!(decode(Encoding::Iso2022Jp, b"\x1B(Xa"), "\u{FFFD}(Xa");
        assert_eq!(decode(Encoding::Iso2022Jp, b"\x1B("), "\u{FFFD}(");
        assert_eq!(decode(Encoding::Iso2022Jp, b"\x1B"), "\u{FFFD}");
        // Zwei Escape-Folgen ohne Zeichen dazwischen sind ein Fehler.
        assert_eq!(decode(Encoding::Iso2022Jp, b"\x1B(B\x1B(B"), "\u{FFFD}");
        // Beispiel aus dem Standard: zweimal „¥“ kodiert und aneinandergehängt.
        let twice = b"\x1B(J\x5C\x1B(B\x1B(J\x5C\x1B(B";
        assert_eq!(decode(Encoding::Iso2022Jp, twice), "¥\u{FFFD}¥");
        assert_eq!(decode_bytewise(Encoding::Iso2022Jp, twice), "¥\u{FFFD}¥");
    }

    #[test]
    fn shift_jis() {
        assert_eq!(
            decode(Encoding::ShiftJis, b"\x82\xA0\xA1\x80"),
            "あ\u{FF61}\u{80}"
        );
        assert_eq!(decode(Encoding::ShiftJis, b"\xF0\x40"), "\u{E000}");
        assert_eq!(
            decode(Encoding::ShiftJis, b"\xA0\x81 "),
            "\u{FFFD}\u{FFFD} "
        );
    }

    #[test]
    fn euc_kr() {
        assert_eq!(decode(Encoding::EucKr, b"\xB0\xA1\x81\x41"), "가갂");
        assert_eq!(decode(Encoding::EucKr, b"\x81 \x80"), "\u{FFFD} \u{FFFD}");
    }

    #[test]
    fn chunks_give_the_same_result() {
        // Pseudozufällige Bytes, dazu viel ASCII und Escape-Folgen.
        let mut state: u32 = 0x1234_5678;
        let bytes: Vec<u8> = (0..20_000)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let [value, kind, ..] = state.to_be_bytes();
                match kind % 8 {
                    0 => 0x1B,
                    1 => b"$(BJI@".get(usize::from(value % 6)).copied().unwrap_or(0),
                    2 => value % 0x80,
                    _ => value,
                }
            })
            .collect();
        for encoding in Encoding::ALL {
            let whole = decode(encoding, &bytes);
            assert_eq!(
                decode_bytewise(encoding, &bytes),
                whole,
                "{}",
                encoding.name()
            );
            let mut decoder = Decoder::new(encoding);
            let mut output = String::new();
            for chunk in bytes.chunks(7) {
                decoder.decode(chunk, false, &mut output);
            }
            decoder.decode(&[], true, &mut output);
            assert_eq!(output, whole, "{}", encoding.name());
        }
    }

    #[test]
    fn fatal_mode() {
        let mut output = String::new();
        let result = Decoder::new(Encoding::ShiftJis).decode_or_fail(b"a\xA0b", true, &mut output);
        assert_eq!(
            result,
            Err(DecodeError {
                encoding: Encoding::ShiftJis
            })
        );
        assert_eq!(output, "a");
        let mut output = String::new();
        let result = Decoder::new(Encoding::EucKr).decode_or_fail(b"a\xB0\xA1", true, &mut output);
        assert_eq!(result, Ok(()));
        assert_eq!(output, "a가");
        // Ein unvollständiges Zeichen am Ende ist ein Fehler.
        let mut decoder = Decoder::new(Encoding::Utf8);
        let mut output = String::new();
        assert_eq!(decoder.decode_or_fail(b"\xC3", false, &mut output), Ok(()));
        assert!(decoder.decode_or_fail(b"", true, &mut output).is_err());
    }
}
