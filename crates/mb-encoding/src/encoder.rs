//! Die Kodierer, nach den Algorithmen des Encoding Standard.
//!
//! *replacement*, UTF-16BE und UTF-16LE haben keinen Kodierer.

use crate::Encoding;
use crate::index::{
    big5_pointer, code_point as index_code_point, euc_kr_pointer, gb18030_pointer,
    gb18030_ranges_pointer, jis0208_pointer, shift_jis_pointer, single_byte_pointer,
};
use crate::tables::INDEX_ISO_2022_JP_KATAKANA;

/// Ein Zeichen, das die Kodierung nicht darstellen kann (Modus „fatal“).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unencodable {
    /// Das Zeichen. ISO-2022-JP meldet für U+000E, U+000F und U+001B
    /// absichtlich U+FFFD.
    pub character: char,
    /// Wie viele Bytes der Eingabe bis einschließlich dieses Zeichens
    /// verarbeitet sind. Hier geht es mit demselben Kodierer weiter.
    pub consumed: usize,
}

impl std::fmt::Display for Unencodable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "U+{:04X} lässt sich nicht kodieren",
            u32::from(self.character)
        )
    }
}

impl std::error::Error for Unencodable {}

/// Ein Kodierer, der Eingabe auch in Stücken verarbeitet.
#[derive(Clone, Debug)]
pub struct Encoder {
    encoding: Encoding,
    state: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Iso2022JpState {
    Ascii,
    Roman,
    Jis0208,
}

#[derive(Clone, Debug)]
enum State {
    Utf8,
    SingleByte(&'static [u16; 128]),
    Gb18030 { gbk: bool },
    Big5,
    EucJp,
    Iso2022Jp(Iso2022JpState),
    ShiftJis,
    EucKr,
    XUserDefined,
}

enum Item {
    Char(char),
    End,
}

enum Step {
    Finished,
    Bytes([u8; 4], usize),
    Error(char),
}

fn one(byte: u8) -> Step {
    Step::Bytes([byte, 0, 0, 0], 1)
}

fn two(first: u8, second: u8) -> Step {
    Step::Bytes([first, second, 0, 0], 2)
}

fn three(bytes: [u8; 3]) -> Step {
    let [a, b, c] = bytes;
    Step::Bytes([a, b, c, 0], 3)
}

/// Ein Zeiger als Byte. Die Algorithmen garantieren den Bereich; ein Wert
/// darüber würde als Fehler gemeldet statt abgeschnitten.
fn byte(value: u32) -> Option<u8> {
    u8::try_from(value).ok()
}

/// Das Byte eines ASCII-Zeichens.
fn ascii(c: char) -> Option<u8> {
    u8::try_from(c).ok().filter(u8::is_ascii)
}

struct Queue<'a> {
    input: &'a str,
    position: usize,
    restored: Option<char>,
    last: bool,
}

impl Queue<'_> {
    fn read(&mut self) -> Option<Item> {
        if let Some(c) = self.restored.take() {
            return Some(Item::Char(c));
        }
        if let Some(c) = self
            .input
            .get(self.position..)
            .and_then(|rest| rest.chars().next())
        {
            self.position += c.len_utf8();
            return Some(Item::Char(c));
        }
        self.last.then_some(Item::End)
    }
}

impl Encoder {
    /// Ein neuer Kodierer, oder `None` für Kodierungen ohne Kodierer
    /// (*replacement*, UTF-16BE, UTF-16LE).
    pub fn new(encoding: Encoding) -> Option<Encoder> {
        let state = match encoding {
            Encoding::Utf8 => State::Utf8,
            Encoding::Gbk => State::Gb18030 { gbk: true },
            Encoding::Gb18030 => State::Gb18030 { gbk: false },
            Encoding::Big5 => State::Big5,
            Encoding::EucJp => State::EucJp,
            Encoding::Iso2022Jp => State::Iso2022Jp(Iso2022JpState::Ascii),
            Encoding::ShiftJis => State::ShiftJis,
            Encoding::EucKr => State::EucKr,
            Encoding::XUserDefined => State::XUserDefined,
            Encoding::Replacement | Encoding::Utf16Be | Encoding::Utf16Le => return None,
            _ => State::SingleByte(crate::tables::single_byte_index(encoding)?),
        };
        Some(Encoder { encoding, state })
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Kodiert ein Stück im Modus „html“: Nicht darstellbare Zeichen werden zu
    /// `&#…;` mit dem Codepunkt in Dezimalziffern. Mit `last` endet die Eingabe
    /// nach diesem Stück.
    pub fn encode(&mut self, input: &str, last: bool, output: &mut Vec<u8>) {
        let mut position = 0;
        while let Err(unencodable) = self.run(input.get(position..).unwrap_or(""), last, output) {
            output.extend_from_slice(b"&#");
            output.extend_from_slice(u32::from(unencodable.character).to_string().as_bytes());
            output.push(b';');
            position += unencodable.consumed;
        }
    }

    /// Kodiert ein Stück im Modus „fatal“ (*encode or fail*). Beim ersten
    /// nicht darstellbaren Zeichen kommt ein Fehler; danach geht es mit dem
    /// Rest ab [`Unencodable::consumed`] weiter.
    pub fn encode_or_fail(
        &mut self,
        input: &str,
        last: bool,
        output: &mut Vec<u8>,
    ) -> Result<(), Unencodable> {
        self.run(input, last, output)
    }

    /// *Process a queue* bis zum Ende des Stücks oder zum ersten Fehler.
    fn run(&mut self, input: &str, last: bool, output: &mut Vec<u8>) -> Result<(), Unencodable> {
        let mut queue = Queue {
            input,
            position: 0,
            restored: None,
            last,
        };
        while let Some(item) = queue.read() {
            match self.state.handle(&mut queue, item) {
                Step::Finished => return Ok(()),
                Step::Bytes(bytes, length) => {
                    output.extend_from_slice(bytes.get(..length).unwrap_or_default());
                }
                Step::Error(character) => {
                    return Err(Unencodable {
                        character,
                        consumed: queue.position,
                    });
                }
            }
        }
        Ok(())
    }
}

impl State {
    fn handle(&mut self, queue: &mut Queue<'_>, item: Item) -> Step {
        let Item::Char(c) = item else {
            if let State::Iso2022Jp(state) = self
                && *state != Iso2022JpState::Ascii
            {
                *state = Iso2022JpState::Ascii;
                return three([0x1B, 0x28, 0x42]);
            }
            return Step::Finished;
        };
        let code_point = u32::from(c);
        if let State::Iso2022Jp(state) = self {
            return iso_2022_jp(queue, state, c);
        }
        if let Some(ascii) = ascii(c) {
            return one(ascii);
        }
        let result = match self {
            State::Utf8 => {
                let mut bytes = [0; 4];
                let length = c.encode_utf8(&mut bytes).len();
                return Step::Bytes(bytes, length);
            }
            State::SingleByte(index) => single_byte_pointer(index, code_point)
                .and_then(|pointer| byte(pointer + 0x80))
                .map(one),
            State::XUserDefined => (0xF780..=0xF7FF)
                .contains(&code_point)
                .then(|| byte(code_point - 0xF780 + 0x80))
                .flatten()
                .map(one),
            State::Gb18030 { gbk } => gb18030(*gbk, code_point),
            State::Big5 => big5_pointer(code_point).and_then(|pointer| {
                let trailing = pointer % 157;
                let offset = if trailing < 0x3F { 0x40 } else { 0x62 };
                Some(two(byte(pointer / 157 + 0x81)?, byte(trailing + offset)?))
            }),
            State::EucJp => euc_jp(code_point),
            State::ShiftJis => shift_jis(code_point),
            State::EucKr => euc_kr_pointer(code_point).and_then(|pointer| {
                Some(two(
                    byte(pointer / 190 + 0x81)?,
                    byte(pointer % 190 + 0x41)?,
                ))
            }),
            // Oben schon behandelt.
            State::Iso2022Jp(_) => None,
        };
        result.unwrap_or(Step::Error(c))
    }
}

/// Die Codepunkte, die der gb18030-Kodierer abweichend vom Index auf zwei
/// Bytes abbildet (Tabelle im Abschnitt „gb18030 encoder“).
const GB18030_ENCODER_TABLE: [(u32, [u8; 2]); 18] = [
    (0xE78D, [0xA6, 0xD9]),
    (0xE78E, [0xA6, 0xDA]),
    (0xE78F, [0xA6, 0xDB]),
    (0xE790, [0xA6, 0xDC]),
    (0xE791, [0xA6, 0xDD]),
    (0xE792, [0xA6, 0xDE]),
    (0xE793, [0xA6, 0xDF]),
    (0xE794, [0xA6, 0xEC]),
    (0xE795, [0xA6, 0xED]),
    (0xE796, [0xA6, 0xF3]),
    (0xE81E, [0xFE, 0x59]),
    (0xE826, [0xFE, 0x61]),
    (0xE82B, [0xFE, 0x66]),
    (0xE82C, [0xFE, 0x67]),
    (0xE832, [0xFE, 0x6D]),
    (0xE843, [0xFE, 0x7E]),
    (0xE854, [0xFE, 0x90]),
    (0xE864, [0xFE, 0xA0]),
];

fn gb18030(gbk: bool, code_point: u32) -> Option<Step> {
    // Index gb18030 bildet 0xA3 0xA0 auf U+3000 ab, U+E5E5 geht nicht zurück.
    if code_point == 0xE5E5 {
        return None;
    }
    if gbk && code_point == 0x20AC {
        return Some(one(0x80));
    }
    if let Some((_, [first, second])) = GB18030_ENCODER_TABLE
        .iter()
        .find(|&&(table_code_point, _)| table_code_point == code_point)
    {
        return Some(two(*first, *second));
    }
    if let Some(pointer) = gb18030_pointer(code_point) {
        let trailing = pointer % 190;
        let offset = if trailing < 0x3F { 0x40 } else { 0x41 };
        return Some(two(byte(pointer / 190 + 0x81)?, byte(trailing + offset)?));
    }
    if gbk {
        return None;
    }
    let mut pointer = gb18030_ranges_pointer(code_point)?;
    let byte1 = pointer / (10 * 126 * 10);
    pointer %= 10 * 126 * 10;
    let byte2 = pointer / (10 * 126);
    pointer %= 10 * 126;
    let byte3 = pointer / 10;
    let byte4 = pointer % 10;
    Some(Step::Bytes(
        [
            byte(byte1 + 0x81)?,
            byte(byte2 + 0x30)?,
            byte(byte3 + 0x81)?,
            byte(byte4 + 0x30)?,
        ],
        4,
    ))
}

fn euc_jp(code_point: u32) -> Option<Step> {
    match code_point {
        0x00A5 => return Some(one(0x5C)),
        0x203E => return Some(one(0x7E)),
        0xFF61..=0xFF9F => return Some(two(0x8E, byte(code_point - 0xFF61 + 0xA1)?)),
        _ => {}
    }
    let code_point = if code_point == 0x2212 {
        0xFF0D
    } else {
        code_point
    };
    let pointer = jis0208_pointer(code_point)?;
    Some(two(byte(pointer / 94 + 0xA1)?, byte(pointer % 94 + 0xA1)?))
}

fn shift_jis(code_point: u32) -> Option<Step> {
    match code_point {
        0x0080 => return Some(one(0x80)),
        0x00A5 => return Some(one(0x5C)),
        0x203E => return Some(one(0x7E)),
        0xFF61..=0xFF9F => return Some(one(byte(code_point - 0xFF61 + 0xA1)?)),
        _ => {}
    }
    let code_point = if code_point == 0x2212 {
        0xFF0D
    } else {
        code_point
    };
    let pointer = shift_jis_pointer(code_point)?;
    let leading = pointer / 188;
    let leading_offset = if leading < 0x1F { 0x81 } else { 0xC1 };
    let trailing = pointer % 188;
    let offset = if trailing < 0x3F { 0x40 } else { 0x41 };
    Some(two(
        byte(leading + leading_offset)?,
        byte(trailing + offset)?,
    ))
}

/// Der Kodierer für ISO-2022-JP mit seinem Zustand.
fn iso_2022_jp(queue: &mut Queue<'_>, state: &mut Iso2022JpState, c: char) -> Step {
    use Iso2022JpState as S;
    let mut code_point = u32::from(c);
    if matches!(*state, S::Ascii | S::Roman) && matches!(code_point, 0x0E | 0x0F | 0x1B) {
        // U+FFFD statt des Zeichens, um Angriffe zu verhindern.
        return Step::Error('\u{FFFD}');
    }
    if *state == S::Ascii
        && let Some(ascii) = ascii(c)
    {
        return one(ascii);
    }
    if *state == S::Roman {
        match (code_point, ascii(c)) {
            (0x00A5, _) => return one(0x5C),
            (0x203E, _) => return one(0x7E),
            (0x5C | 0x7E, _) => {}
            (_, Some(ascii)) => return one(ascii),
            (_, None) => {}
        }
    }
    if c.is_ascii() && *state != S::Ascii {
        queue.restored = Some(c);
        *state = S::Ascii;
        return three([0x1B, 0x28, 0x42]);
    }
    if (code_point == 0x00A5 || code_point == 0x203E) && *state != S::Roman {
        queue.restored = Some(c);
        *state = S::Roman;
        return three([0x1B, 0x28, 0x4A]);
    }
    if code_point == 0x2212 {
        code_point = 0xFF0D;
    }
    if (0xFF61..=0xFF9F).contains(&code_point) {
        match index_code_point(&INDEX_ISO_2022_JP_KATAKANA, code_point - 0xFF61) {
            Some(full_width) => code_point = full_width,
            None => return Step::Error(c),
        }
    }
    // Ab hier ist `code_point` immer ein Skalarwert aus den Tabellen.
    let current = char::from_u32(code_point).unwrap_or(c);
    let Some(pointer) = jis0208_pointer(code_point) else {
        if *state == S::Jis0208 {
            queue.restored = Some(current);
            *state = S::Ascii;
            return three([0x1B, 0x28, 0x42]);
        }
        return Step::Error(current);
    };
    if *state != S::Jis0208 {
        queue.restored = Some(current);
        *state = S::Jis0208;
        return three([0x1B, 0x24, 0x42]);
    }
    match (byte(pointer / 94 + 0x21), byte(pointer % 94 + 0x21)) {
        (Some(leading), Some(trailing)) => two(leading, trailing),
        _ => Step::Error(current),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Decoder;
    use crate::tables::single_byte_index;

    fn encode(encoding: Encoding, text: &str) -> Vec<u8> {
        let mut output = Vec::new();
        Encoder::new(encoding)
            .expect("Kodierung mit Kodierer")
            .encode(text, true, &mut output);
        output
    }

    fn decode(encoding: Encoding, bytes: &[u8], output: &mut String) {
        output.clear();
        Decoder::new(encoding).decode(bytes, true, output);
    }

    /// Kodiert ein einzelnes Zeichen im Modus „fatal“.
    fn encode_char(encoding: Encoding, c: char, output: &mut Vec<u8>) -> bool {
        output.clear();
        let Some(mut encoder) = Encoder::new(encoding) else {
            return false;
        };
        let mut buffer = [0; 4];
        encoder
            .encode_or_fail(c.encode_utf8(&mut buffer), true, output)
            .is_ok()
    }

    /// Was beim Dekodieren eines kodierten Zeichens herauskommt, wenn es laut
    /// Standard nicht das Zeichen selbst ist.
    fn one_way(encoding: Encoding, code_point: u32) -> Option<u32> {
        let japanese = matches!(
            encoding,
            Encoding::EucJp | Encoding::ShiftJis | Encoding::Iso2022Jp
        );
        let single_byte_yen = matches!(encoding, Encoding::EucJp | Encoding::ShiftJis);
        match code_point {
            0x2212 if japanese => Some(0xFF0D),
            0x00A5 if single_byte_yen => Some(0x5C),
            0x203E if single_byte_yen => Some(0x7E),
            0xFF61..=0xFF9F if encoding == Encoding::Iso2022Jp => {
                index_code_point(&INDEX_ISO_2022_JP_KATAKANA, code_point - 0xFF61)
            }
            _ => None,
        }
    }

    #[test]
    fn multi_byte_round_trips() {
        let mut bytes = Vec::new();
        let mut text = String::new();
        let mut again = Vec::new();
        for encoding in [
            Encoding::Gb18030,
            Encoding::Gbk,
            Encoding::Big5,
            Encoding::EucJp,
            Encoding::Iso2022Jp,
            Encoding::ShiftJis,
            Encoding::EucKr,
        ] {
            let mut encodable = 0u32;
            for c in (0..=0x10_FFFF).filter_map(char::from_u32) {
                if !encode_char(encoding, c, &mut bytes) {
                    continue;
                }
                encodable += 1;
                decode(encoding, &bytes, &mut text);
                let code_point = u32::from(c);
                let gb18030 = matches!(encoding, Encoding::Gb18030 | Encoding::Gbk);
                if gb18030
                    && GB18030_ENCODER_TABLE
                        .iter()
                        .any(|&(table_code_point, _)| table_code_point == code_point)
                {
                    // Die Tabelle gilt nur in eine Richtung. Das dekodierte
                    // Zeichen ergibt wieder dieselben Bytes.
                    let decoded = text.chars().next().expect("ein Zeichen");
                    assert_ne!(decoded, c);
                    assert!(encode_char(encoding, decoded, &mut again));
                    assert_eq!(again, bytes);
                    continue;
                }
                let expected = one_way(encoding, code_point)
                    .and_then(char::from_u32)
                    .unwrap_or(c);
                assert_eq!(
                    text.chars().collect::<Vec<_>>(),
                    [expected],
                    "{} U+{code_point:04X} {bytes:02X?}",
                    encoding.name()
                );
            }
            if encoding == Encoding::Gb18030 {
                // Alle Skalarwerte außer U+E5E5.
                assert_eq!(encodable, 0x11_0000 - 0x800 - 1);
            }
        }
    }

    #[test]
    fn single_byte_round_trips() {
        let mut bytes = Vec::new();
        let mut code_points: Vec<u32> = Encoding::ALL
            .into_iter()
            .filter_map(single_byte_index)
            .flat_map(|index| index.iter().map(|&value| u32::from(value)))
            .filter(|&value| value != 0)
            .collect();
        code_points.sort_unstable();
        code_points.dedup();
        for encoding in Encoding::ALL {
            let Some(index) = single_byte_index(encoding) else {
                continue;
            };
            for byte in 0..0x80u8 {
                assert!(encode_char(encoding, char::from(byte), &mut bytes));
                assert_eq!(bytes, [byte]);
            }
            for &code_point in &code_points {
                let c = char::from_u32(code_point).expect("Skalarwert");
                let pointer = (0u8..)
                    .zip(index)
                    .find(|&(_, &value)| u32::from(value) == code_point);
                match pointer {
                    Some((pointer, _)) => {
                        assert!(encode_char(encoding, c, &mut bytes));
                        assert_eq!(bytes, [pointer + 0x80], "{} {c}", encoding.name());
                    }
                    None => assert!(!encode_char(encoding, c, &mut bytes)),
                }
            }
        }
    }

    #[test]
    fn utf8_and_x_user_defined() {
        let mut bytes = Vec::new();
        for c in (0..=0x10_FFFF).filter_map(char::from_u32) {
            assert!(encode_char(Encoding::Utf8, c, &mut bytes));
            assert_eq!(bytes, c.to_string().as_bytes());
        }
        for byte in 0x80..=0xFFu8 {
            let c = char::from_u32(0xF780 + u32::from(byte) - 0x80).expect("Skalarwert");
            assert!(encode_char(Encoding::XUserDefined, c, &mut bytes));
            assert_eq!(bytes, [byte]);
        }
        assert!(!encode_char(Encoding::XUserDefined, '\u{F77F}', &mut bytes));
        assert!(!encode_char(Encoding::XUserDefined, '\u{F800}', &mut bytes));
        assert!(!encode_char(Encoding::XUserDefined, 'ä', &mut bytes));
    }

    #[test]
    fn known_bytes() {
        assert_eq!(
            encode(Encoding::EucJp, "¥‾\u{FF61}あ−"),
            [0x5C, 0x7E, 0x8E, 0xA1, 0xA4, 0xA2, 0xA1, 0xDD]
        );
        assert_eq!(
            encode(Encoding::ShiftJis, "\u{80}¥‾\u{FF61}あ"),
            [0x80, 0x5C, 0x7E, 0xA1, 0x82, 0xA0]
        );
        // NEC-Auswahl ausgelassen: die IBM-Erweiterung bei Zeiger 10717.
        assert_eq!(encode(Encoding::ShiftJis, "ⅱ"), [0xFA, 0x41]);
        // Big5: der letzte Zeiger für U+2550 und U+5341, ohne Hongkong-Zeichen.
        assert_eq!(encode(Encoding::Big5, "═十"), [0xF9, 0xF9, 0xA4, 0x51]);
        assert_eq!(encode(Encoding::Big5, "\u{4425}"), b"&#17445;");
        assert_eq!(encode(Encoding::EucKr, "가"), [0xB0, 0xA1]);
        assert_eq!(encode(Encoding::Gb18030, "\u{3000}"), [0xA1, 0xA1]);
        assert_eq!(
            encode(Encoding::Gb18030, "\u{80}"),
            [0x81, 0x30, 0x81, 0x30]
        );
        assert_eq!(
            encode(Encoding::Gb18030, "\u{E7C7}"),
            [0x81, 0x35, 0xF4, 0x37]
        );
        assert_eq!(encode(Encoding::Gb18030, "\u{E78D}"), [0xA6, 0xD9]);
        assert_eq!(
            encode(Encoding::Gb18030, "\u{10000}"),
            [0x90, 0x30, 0x81, 0x30]
        );
        assert_eq!(
            encode(Encoding::Gb18030, "\u{10FFFF}"),
            [0xE3, 0x32, 0x9A, 0x35]
        );
        assert_eq!(encode(Encoding::Gbk, "\u{10000}"), b"&#65536;");
    }

    #[test]
    fn iso_2022_jp() {
        // Beispiel aus dem Standard.
        assert_eq!(encode(Encoding::Iso2022Jp, "¥"), b"\x1B(J\x5C\x1B(B");
        // Im Zustand Roman bleibt ASCII außer 0x5C und 0x7E unverändert, diese
        // beiden stehen dort für ¥ und ‾.
        assert_eq!(encode(Encoding::Iso2022Jp, "a¥b"), b"a\x1B(J\x5Cb\x1B(B");
        assert_eq!(encode(Encoding::Iso2022Jp, "¥~"), b"\x1B(J\x5C\x1B(B~");
        assert_eq!(encode(Encoding::Iso2022Jp, "¥‾a"), b"\x1B(J\x5C\x7Ea\x1B(B");
        assert_eq!(
            encode(Encoding::Iso2022Jp, "\u{FF61}−"),
            b"\x1B$B\x21\x23\x21\x5D\x1B(B"
        );
        assert_eq!(
            encode(Encoding::Iso2022Jp, "あ😀"),
            b"\x1B$B\x24\x22\x1B(B&#128512;"
        );
        assert_eq!(encode(Encoding::Iso2022Jp, "a\u{1B}"), b"a&#65533;");
        let mut encoder = Encoder::new(Encoding::Iso2022Jp).expect("Kodierer");
        let mut output = Vec::new();
        assert_eq!(
            encoder.encode_or_fail("¥\u{E}x", true, &mut output),
            Err(Unencodable {
                character: '\u{FFFD}',
                consumed: 3
            })
        );
        // Der Kodierer bleibt im Zustand Roman.
        assert_eq!(encoder.encode_or_fail("x", true, &mut output), Ok(()));
        assert_eq!(output, b"\x1B(J\x5Cx\x1B(B");
    }

    #[test]
    fn fatal_mode_reports_position() {
        let mut encoder = Encoder::new(Encoding::Windows1252).expect("Kodierer");
        let mut output = Vec::new();
        let input = "aä☃b";
        let error = encoder.encode_or_fail(input, true, &mut output);
        assert_eq!(
            error,
            Err(Unencodable {
                character: '☃',
                consumed: 6
            })
        );
        assert_eq!(output, [b'a', 0xE4]);
        assert_eq!(
            encoder.encode_or_fail(input.get(6..).unwrap_or_default(), true, &mut output),
            Ok(())
        );
        assert_eq!(output, [b'a', 0xE4, b'b']);
    }

    #[test]
    fn chunks_give_the_same_result() {
        let text = "a¥b‾c\u{FF61}あ−😀\u{1B}~\\☃十가€\u{E5E5}\u{10000}z¥¥あa";
        for encoding in Encoding::ALL {
            let Some(mut encoder) = Encoder::new(encoding) else {
                continue;
            };
            let whole = encode(encoding, text);
            let mut output = Vec::new();
            for c in text.chars() {
                encoder.encode(c.encode_utf8(&mut [0; 4]), false, &mut output);
            }
            encoder.encode("", true, &mut output);
            assert_eq!(output, whole, "{}", encoding.name());
        }
    }
}
