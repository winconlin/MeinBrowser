//! Deflate nach RFC 1951. Was der Text offenlässt, entscheidet der Parser
//! wie zlib und dessen Referenzdekoder `puff`: unvollständige Codes sind nur
//! als einzelner Code der Länge 1 erlaubt, HLIT über 286 und HDIST über 30
//! sind Fehler, ebenso ein Abstand vor den Anfang der Ausgabe.

use crate::Error;

/// Liest Bits vom niederwertigsten an (RFC 1951, Abschnitt 3.1.1).
struct Bits<'a> {
    input: &'a [u8],
    /// Das nächste Byte, das noch nicht im Puffer ist.
    pos: usize,
    buffer: u64,
    count: u32,
}

impl Bits<'_> {
    fn refill(&mut self) {
        while self.count <= 56 {
            let Some(&byte) = self.input.get(self.pos) else {
                return;
            };
            self.buffer |= u64::from(byte) << self.count;
            self.count += 8;
            self.pos += 1;
        }
    }

    /// Die nächsten `n` Bits (höchstens 32), am Ende mit Nullen aufgefüllt.
    fn peek(&mut self, n: u32) -> u32 {
        if self.count < n {
            self.refill();
        }
        let mask = (1_u64 << n) - 1;
        u32::try_from(self.buffer & mask).unwrap_or(0)
    }

    fn consume(&mut self, n: u32) -> Result<(), Error> {
        if n > self.count {
            return Err(Error::UnexpectedEnd);
        }
        self.buffer >>= n;
        self.count -= n;
        Ok(())
    }

    fn bits(&mut self, n: u32) -> Result<u32, Error> {
        let value = self.peek(n);
        self.consume(n)?;
        Ok(value)
    }

    /// Wie viele Bits noch übrig sind.
    fn remaining(&self) -> usize {
        let unread = self.input.len().saturating_sub(self.pos);
        unread
            .saturating_mul(8)
            .saturating_add(usize::try_from(self.count).unwrap_or(0))
    }

    /// Die Stelle des nächsten ganzen Bytes; angebrochene Bytes zählen als
    /// gelesen.
    fn byte_position(&self) -> usize {
        let whole = usize::try_from(self.count / 8).unwrap_or(0);
        self.pos.saturating_sub(whole)
    }

    /// Verwirft die Bits bis zur Bytegrenze und leert den Puffer, sodass
    /// `pos` auf das nächste ganze Byte zeigt.
    fn align(&mut self) {
        self.pos = self.byte_position();
        self.buffer = 0;
        self.count = 0;
    }
}

/// Ein kanonischer Huffman-Code als Tabelle über `bits` Bits: Jeder
/// Eintrag ist `(Symbol << 4) | Länge`, die Länge 0 steht für kein Code.
struct Huffman {
    table: Vec<u16>,
    bits: u32,
}

/// Was die Längen eines Codes ergeben (RFC 1951, Abschnitt 3.2.2).
enum Code {
    Complete(Huffman),
    /// Unvollständig; ob das erlaubt ist, entscheidet der Aufrufer.
    Incomplete(Huffman),
    OverSubscribed,
}

const MAX_BITS: usize = 15;

fn build(lengths: &[u8]) -> Code {
    let mut count = [0_u16; MAX_BITS + 1];
    for &length in lengths {
        if let Some(entry) = count.get_mut(usize::from(length)) {
            *entry += 1;
        }
    }
    count[0] = 0;
    // Überbelegt, wenn mehr Codes einer Länge vergeben werden, als frei sind.
    let mut left: i32 = 1;
    for &number in count.iter().skip(1) {
        left = left * 2 - i32::from(number);
        if left < 0 {
            return Code::OverSubscribed;
        }
    }
    let max = count.iter().rposition(|&number| number > 0).unwrap_or(0);
    let bits = u32::try_from(max).unwrap_or(0);
    let mut next = [0_u32; MAX_BITS + 1];
    let mut code = 0_u32;
    for length in 1..=MAX_BITS {
        code = (code + u32::from(count[length - 1])) << 1;
        next[length] = code;
    }
    let mut table = vec![0_u16; 1 << max];
    for (symbol, &length) in lengths.iter().enumerate() {
        let length = usize::from(length);
        if length == 0 {
            continue;
        }
        let Some(slot) = next.get_mut(length) else {
            continue;
        };
        // Im Strom kommt das höchstwertige Bit des Codes zuerst.
        let reversed = slot.reverse_bits() >> (32 - length);
        *slot += 1;
        let entry = u16::try_from(symbol << 4 | length).unwrap_or(0);
        let mut index = usize::try_from(reversed).unwrap_or(usize::MAX);
        while let Some(cell) = table.get_mut(index) {
            *cell = entry;
            index += 1 << length;
        }
    }
    let huffman = Huffman { table, bits };
    if left > 0 && max > 0 {
        Code::Incomplete(huffman)
    } else {
        Code::Complete(huffman)
    }
}

impl Huffman {
    /// Ein Code für Literale und Längen oder für Abstände: unvollständig
    /// nur als ein einziger Code der Länge 1.
    fn for_data(lengths: &[u8], error: Error) -> Result<Huffman, Error> {
        match build(lengths) {
            Code::Complete(huffman) => Ok(huffman),
            Code::Incomplete(huffman)
                if lengths.iter().filter(|&&length| length != 0).count() == 1
                    && huffman.bits == 1 =>
            {
                Ok(huffman)
            }
            Code::Incomplete(_) | Code::OverSubscribed => Err(error),
        }
    }

    fn decode(&self, bits: &mut Bits<'_>, invalid: Error) -> Result<usize, Error> {
        let index = usize::try_from(bits.peek(self.bits)).unwrap_or(usize::MAX);
        let entry = self.table.get(index).copied().unwrap_or(0);
        let length = u32::from(entry & 0xF);
        if length == 0 {
            // Ohne genug Eingabe ist das Ende erreicht, sonst ist der Code
            // ungültig.
            return Err(if bits.count < self.bits {
                Error::UnexpectedEnd
            } else {
                invalid
            });
        }
        bits.consume(length)?;
        Ok(usize::from(entry >> 4))
    }
}

/// Grundwerte und Zusatzbits der Längen 257 bis 285 und der Abstände 0 bis
/// 29 (RFC 1951, Abschnitt 3.2.5).
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Die Reihenfolge der Längen für den Code der Codelängen.
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Das Ergebnis von [`inflate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inflated {
    pub data: Vec<u8>,
    /// Die Bytes der Eingabe bis zum Ende des letzten Blocks, ein
    /// angebrochenes Byte mitgezählt.
    pub consumed: usize,
    /// Ob ein Block mit `BFINAL` kam. Sonst endete die Eingabe nach einem
    /// ganzen Block, und es könnte mehr folgen.
    pub finished: bool,
}

/// Dekodiert Deflate-Daten. Mehr als `limit` Bytes Ausgabe sind ein
/// Fehler, damit kleine Eingaben nicht beliebig viel Speicher belegen.
pub fn inflate(input: &[u8], limit: usize) -> Result<Inflated, Error> {
    let mut bits = Bits {
        input,
        pos: 0,
        buffer: 0,
        count: 0,
    };
    let mut out = Vec::new();
    let incomplete = |data| Inflated {
        data,
        consumed: input.len(),
        finished: false,
    };
    loop {
        // Wie zlib: Endet die Eingabe vor dem Kopf eines Blocks oder direkt
        // nach dem eines Blocks ohne Kompression (so endet ein Kodierer, der
        // mit Nullen auffüllt), könnte noch mehr kommen.
        if bits.remaining() < 3 {
            return Ok(incomplete(out));
        }
        let last = bits.bits(1)? == 1;
        match bits.bits(2)? {
            0 => {
                bits.align();
                if !last && bits.pos >= input.len() {
                    return Ok(incomplete(out));
                }
                stored(&mut bits, &mut out, limit)?;
            }
            1 => {
                let (literals, distances) = fixed_codes();
                compressed(&mut bits, &mut out, limit, &literals, &distances)?;
            }
            2 => {
                let (literals, distances) = dynamic_codes(&mut bits)?;
                compressed(&mut bits, &mut out, limit, &literals, &distances)?;
            }
            _ => return Err(Error::BlockType),
        }
        if last {
            return Ok(Inflated {
                data: out,
                consumed: bits.byte_position(),
                finished: true,
            });
        }
    }
}

/// Ein Block ohne Kompression (Abschnitt 3.2.4).
fn stored(bits: &mut Bits<'_>, out: &mut Vec<u8>, limit: usize) -> Result<(), Error> {
    bits.align();
    let start = bits.pos.saturating_add(4);
    let Some(&[low, high, complement_low, complement_high]) = bits.input.get(bits.pos..start)
    else {
        return Err(Error::UnexpectedEnd);
    };
    let length = u16::from_le_bytes([low, high]);
    if length != !u16::from_le_bytes([complement_low, complement_high]) {
        return Err(Error::StoredLength);
    }
    let end = start.saturating_add(usize::from(length));
    let data = bits.input.get(start..end).ok_or(Error::UnexpectedEnd)?;
    if out.len().saturating_add(data.len()) > limit {
        return Err(Error::TooLarge);
    }
    out.extend_from_slice(data);
    bits.pos = end;
    Ok(())
}

/// Die festen Codes (Abschnitt 3.2.6). Die Symbole 286, 287, 30 und 31
/// haben Codes, kommen aber nicht vor.
fn fixed_codes() -> (Huffman, Huffman) {
    let mut lengths = [0_u8; 288];
    for (symbol, length) in lengths.iter_mut().enumerate() {
        *length = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let literals = match build(&lengths) {
        Code::Complete(huffman) | Code::Incomplete(huffman) => huffman,
        Code::OverSubscribed => unreachable!("die festen Codes sind vollständig"),
    };
    let distances = match build(&[5; 32]) {
        Code::Complete(huffman) | Code::Incomplete(huffman) => huffman,
        Code::OverSubscribed => unreachable!("die festen Codes sind vollständig"),
    };
    (literals, distances)
}

/// Die Codes eines Blocks mit eigenen Codes (Abschnitt 3.2.7).
fn dynamic_codes(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), Error> {
    let literal_count = usize::try_from(bits.bits(5)?).unwrap_or(usize::MAX) + 257;
    let distance_count = usize::try_from(bits.bits(5)?).unwrap_or(usize::MAX) + 1;
    let code_length_count = usize::try_from(bits.bits(4)?).unwrap_or(usize::MAX) + 4;
    if literal_count > 286 || distance_count > 30 {
        return Err(Error::TooManySymbols);
    }
    let mut code_lengths = [0_u8; 19];
    for &symbol in CODE_LENGTH_ORDER.iter().take(code_length_count) {
        if let Some(length) = code_lengths.get_mut(symbol) {
            *length = u8::try_from(bits.bits(3)?).unwrap_or(0);
        }
    }
    // Der Code der Codelängen muss vollständig sein.
    let code_length_code = match build(&code_lengths) {
        Code::Complete(huffman) if huffman.bits > 0 => huffman,
        _ => return Err(Error::CodeLengthCode),
    };
    let total = literal_count + distance_count;
    let mut lengths: Vec<u8> = Vec::with_capacity(total);
    while lengths.len() < total {
        let symbol = code_length_code.decode(bits, Error::CodeLengthCode)?;
        let (value, repeat) = match symbol {
            0..=15 => (u8::try_from(symbol).unwrap_or(0), 1),
            16 => {
                let previous = *lengths.last().ok_or(Error::Repeat)?;
                (previous, 3 + bits.bits(2)?)
            }
            17 => (0, 3 + bits.bits(3)?),
            _ => (0, 11 + bits.bits(7)?),
        };
        let repeat = usize::try_from(repeat).unwrap_or(usize::MAX);
        if lengths.len() + repeat > total {
            return Err(Error::Repeat);
        }
        lengths.extend(std::iter::repeat_n(value, repeat));
    }
    let (literal_lengths, distance_lengths) = lengths.split_at(literal_count);
    if literal_lengths.get(256).is_none_or(|&length| length == 0) {
        return Err(Error::MissingEndOfBlock);
    }
    let literals = Huffman::for_data(literal_lengths, Error::LiteralLengthCode)?;
    let distances = Huffman::for_data(distance_lengths, Error::DistanceCode)?;
    Ok((literals, distances))
}

/// Die Daten eines komprimierten Blocks bis zum Symbol 256.
fn compressed(
    bits: &mut Bits<'_>,
    out: &mut Vec<u8>,
    limit: usize,
    literals: &Huffman,
    distances: &Huffman,
) -> Result<(), Error> {
    loop {
        let symbol = literals.decode(bits, Error::InvalidLiteralLength)?;
        if symbol < 256 {
            if out.len() >= limit {
                return Err(Error::TooLarge);
            }
            out.push(u8::try_from(symbol).unwrap_or(0));
            continue;
        }
        if symbol == 256 {
            return Ok(());
        }
        let index = symbol - 257;
        let (Some(&base), Some(&extra)) = (LENGTH_BASE.get(index), LENGTH_EXTRA.get(index)) else {
            return Err(Error::InvalidLiteralLength);
        };
        let length = usize::from(base) + usize::try_from(bits.bits(u32::from(extra))?).unwrap_or(0);
        let index = distances.decode(bits, Error::InvalidDistance)?;
        let (Some(&base), Some(&extra)) = (DISTANCE_BASE.get(index), DISTANCE_EXTRA.get(index))
        else {
            return Err(Error::InvalidDistance);
        };
        let distance =
            usize::from(base) + usize::try_from(bits.bits(u32::from(extra))?).unwrap_or(0);
        if distance > out.len() {
            return Err(Error::TooFarBack);
        }
        if out.len() + length > limit {
            return Err(Error::TooLarge);
        }
        // Die Kopie darf sich mit dem überschneiden, was sie schreibt.
        let start = out.len() - distance;
        for offset in 0..length {
            let byte = out.get(start + offset).copied().unwrap_or(0);
            out.push(byte);
        }
    }
}
