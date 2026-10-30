use super::*;

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("Hexadezimalzahl"))
        .collect()
}

/// Die Fälle für Deflate ohne Kopf aus `test/infcover.c` von zlib
/// (`cover_inflate`), mit dem Fehler, den zlib meldet.
const ZLIB_ERRORS: [(&str, Error); 14] = [
    ("0 0 0 0 0", Error::StoredLength),
    ("6", Error::BlockType),
    ("fc 0 0", Error::TooManySymbols),
    ("4 0 fe ff", Error::CodeLengthCode),
    ("4 0 24 49 0", Error::Repeat),
    ("4 0 24 e9 ff ff", Error::Repeat),
    ("4 0 24 e9 ff 6d", Error::MissingEndOfBlock),
    (
        "4 80 49 92 24 49 92 24 71 ff ff 93 11 0",
        Error::LiteralLengthCode,
    ),
    (
        "4 80 49 92 24 49 92 24 f b4 ff ff c3 84",
        Error::DistanceCode,
    ),
    (
        "4 c0 81 8 0 0 0 0 20 7f eb b 0 0",
        Error::InvalidLiteralLength,
    ),
    ("2 7e ff ff", Error::InvalidDistance),
    ("c c0 81 0 0 0 0 0 90 ff 6b 4 0", Error::TooFarBack),
    // Aus `cover_fast`: ein ungültiger Abstand und ein ungültiges Literal
    // im festen Code.
    ("3 7e 0 0 0 0 0", Error::InvalidDistance),
    ("1b 7 0 0 0 0 0", Error::InvalidLiteralLength),
];

/// Die Fälle aus `cover_inflate`, bei denen zlib keinen Fehler meldet.
const ZLIB_VALID: [&str; 7] = [
    "3 0",
    "1 1 0 fe ff 0",
    "5 c0 21 d 0 0 0 80 b0 fe 6d 2f 91 6c",
    "5 e0 81 91 24 cb b2 2c 49 e2 f 2e 8b 9a 47 56 9f fb fe ec d2 ff 1f",
    "ed c0 1 1 0 0 0 40 20 ff 57 1b 42 2c 4f",
    "ed cf c1 b1 2c 47 10 c4 30 fa 6f 35 1d 1 82 59 3d fb be 2e 2a fc f c",
    "ed c0 81 0 0 0 0 80 a0 fd a9 17 a9 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 \
     0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 6",
];

#[test]
fn zlib_cases() {
    for (input, error) in ZLIB_ERRORS {
        assert_eq!(inflate(&hex(input), 1 << 20), Err(error), "{input}");
    }
    for input in ZLIB_VALID {
        assert!(inflate(&hex(input), 1 << 20).is_ok(), "{input}");
    }
    let fixed = inflate(&hex("3 0"), 0).expect("leerer fester Block");
    assert_eq!(
        fixed,
        Inflated {
            data: Vec::new(),
            consumed: 2,
            finished: true,
        }
    );
    let stored = inflate(&hex("1 1 0 fe ff 0 aa"), 1).expect("Block ohne Kompression");
    assert_eq!((stored.data, stored.consumed), (vec![0], 6));
}

#[test]
fn zlib_format() {
    // Aus `cover_wrap` in `infcover.c`.
    assert_eq!(
        zlib_decompress(&hex("78 9c 63 0 0 0 1 0 1"), 1),
        Ok(vec![0])
    );
    assert_eq!(zlib_decompress(&hex("78 90"), 1), Err(Error::ZlibHeader));
    assert_eq!(zlib_decompress(&hex("77 85"), 1), Err(Error::ZlibHeader));
    assert_eq!(
        zlib_decompress(&hex("8 b8 0 0 0 1"), 1),
        Err(Error::Dictionary)
    );
    assert_eq!(
        zlib_decompress(&hex("78 9c 63 0 0 0 1 0 2"), 1),
        Err(Error::Checksum)
    );
    assert_eq!(
        zlib_decompress(&hex("78 9c 63 0 0 0 1 0"), 1),
        Err(Error::UnexpectedEnd)
    );
    assert_eq!(
        zlib_decompress(&hex("78 9c 63 0 0 0 1 0 1"), 0),
        Err(Error::TooLarge)
    );
}

#[test]
fn sized_output_is_allocated_once() {
    // Ein Block mit festen Codes (Abschnitt 3.2.6): das Literal 0, dann
    // viermal 258 Bytes ab dem Abstand 1, die Byte für Byte dazukommen.
    let mut writer = BitWriter::default();
    writer.value(1, 1); // BFINAL
    writer.value(1, 2); // feste Codes
    writer.code(0x30, 8); // Literal 0
    for _ in 0..4 {
        writer.code(0xC5, 8); // Symbol 285: Länge 258
        writer.code(0, 5); // Abstand 1
    }
    writer.code(0, 7); // Blockende
    let expected = vec![0_u8; 1033];
    let mut input = vec![0x78, 0x01];
    input.extend_from_slice(&writer.bytes());
    input.extend_from_slice(&adler32(&expected).to_be_bytes());
    assert_eq!(zlib_decompress(&input, 1033).as_ref(), Ok(&expected));
    let data = zlib_decompress_sized(&input, 1033).expect("1033 Bytes");
    assert_eq!(data, expected);
    // Wäre der Puffer gewachsen, hätte er mehr Platz.
    assert_eq!(data.capacity(), Vec::<u8>::with_capacity(1033).capacity());
    assert_eq!(zlib_decompress_sized(&input, 1032), Err(Error::TooLarge));
}

/// Schreibt Bits wie ein Kodierer: Zahlen vom niederwertigsten Bit an,
/// Huffman-Codes vom höchstwertigen an.
#[derive(Default)]
struct BitWriter {
    bits: Vec<bool>,
}

impl BitWriter {
    fn value(&mut self, value: u32, count: u32) {
        for bit in 0..count {
            self.bits.push(value >> bit & 1 == 1);
        }
    }

    fn code(&mut self, code: u32, length: u32) {
        for bit in (0..length).rev() {
            self.bits.push(code >> bit & 1 == 1);
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0_u8; self.bits.len().div_ceil(8)];
        for (index, &bit) in self.bits.iter().enumerate() {
            if let Some(byte) = bytes.get_mut(index / 8)
                && bit
            {
                *byte |= 1 << (index % 8);
            }
        }
        bytes
    }
}

/// Ein Block mit eigenen Codes, der nur das Blockende enthält: Literale
/// und Längen haben nur den Code für 256 (Länge 1), der eine Abstand die
/// Länge `distance_length`.
fn single_code_block(distance_length: u32) -> Vec<u8> {
    let mut writer = BitWriter::default();
    writer.value(1, 1); // BFINAL
    writer.value(2, 2); // eigene Codes
    writer.value(0, 5); // HLIT: 257 Literale und Längen
    writer.value(0, 5); // HDIST: ein Abstand
    writer.value(14, 4); // HCLEN: 18 Codelängen
    // Der Code der Codelängen: 18 mit Länge 1, 1 und 2 mit Länge 2, also
    // kanonisch 18 → 0, 1 → 10, 2 → 11.
    for symbol in [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1] {
        let length = match symbol {
            18 => 1,
            1 | 2 => 2,
            _ => 0,
        };
        writer.value(length, 3);
    }
    // 256 Nullen: zweimal 18, mit 138 und 118 Wiederholungen.
    writer.code(0, 1);
    writer.value(138 - 11, 7);
    writer.code(0, 1);
    writer.value(118 - 11, 7);
    // Länge 1 für das Symbol 256, dann die des Abstands.
    writer.code(0b10, 2);
    writer.code(if distance_length == 1 { 0b10 } else { 0b11 }, 2);
    // Das Blockende hat den Code 0.
    writer.code(0, 1);
    writer.bytes()
}

#[test]
fn incomplete_codes() {
    // Ein einzelner Code der Länge 1 darf unvollständig sein, einer der
    // Länge 2 nicht.
    assert_eq!(
        inflate(&single_code_block(1), 0).map(|result| result.finished),
        Ok(true)
    );
    assert_eq!(inflate(&single_code_block(2), 0), Err(Error::DistanceCode));
}

/// Die Daten aus `src/compress/flate/testdata` von Go: Jede Datei ohne
/// `.in` sind Deflate-Daten aus Gos Kodierer für die Datei mit `.in`, ohne
/// letzten Block. zlib dekodiert sie genauso.
#[test]
fn go_test_vectors() {
    let files = testdata::files("go", "src/compress/flate/testdata", "in");
    assert_eq!(files.len(), 9);
    let mut checked = 0;
    for input in &files {
        let stem = input.trim_end_matches(".in");
        let expected = testdata::read_bytes("go", &format!("src/compress/flate/testdata/{input}"));
        let mut variants: Vec<String> = vec![format!("{stem}.golden")];
        for writer in ["dyn", "sync", "wb"] {
            variants.push(format!("{stem}.{writer}.expect"));
            variants.push(format!("{stem}.{writer}.expect-noinput"));
        }
        for variant in variants {
            let path = format!("src/compress/flate/testdata/{variant}");
            if !testdata::root().join("go").join(&path).exists() {
                continue;
            }
            let compressed = testdata::read_bytes("go", &path);
            let inflated =
                inflate(&compressed, 1 << 20).unwrap_or_else(|error| panic!("{variant}: {error}"));
            assert_eq!(inflated.data, expected, "{variant}");
            assert!(!inflated.finished, "{variant}");
            checked += 1;
        }
    }
    assert_eq!(checked, 57);
}
