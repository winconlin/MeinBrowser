//! Eigene kleine GIF-Dateien, ohne fremde Bilder oder Testbibliotheken.
//!
//! Die LZW-Daten verwenden meist nur Literale. Ihre Codebreiten ergeben
//! sich aus der Zahl der ausgegebenen Literale, unabhängig vom Dekoder.

use super::*;

const LIMIT: u64 = 1 << 20;
const BLACK_WHITE: [[u8; 3]; 2] = [[0, 0, 0], [255, 255, 255]];

fn table_bits(palette: &[[u8; 3]]) -> u8 {
    assert!((2..=256).contains(&palette.len()) && palette.len().is_power_of_two());
    u8::try_from(palette.len().trailing_zeros() - 1).expect("kleine Palette")
}

fn file(
    screen: [u16; 2],
    palette: Option<&[[u8; 3]]>,
    background: u8,
    blocks: &[Vec<u8>],
) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    for size in screen {
        out.extend_from_slice(&size.to_le_bytes());
    }
    out.push(palette.map_or(0, |palette| 0x80 | table_bits(palette)));
    out.extend_from_slice(&[background, 0]);
    if let Some(palette) = palette {
        out.extend(palette.iter().flatten().copied());
    }
    for block in blocks {
        out.extend_from_slice(block);
    }
    out.push(0x3B);
    out
}

fn image(
    rect: [u16; 4],
    palette: Option<&[[u8; 3]]>,
    interlaced: bool,
    minimum: u8,
    compressed: &[u8],
    chunk_size: usize,
) -> Vec<u8> {
    let mut out = vec![0x2C];
    for value in rect {
        out.extend_from_slice(&value.to_le_bytes());
    }
    let packed = palette.map_or(0, |palette| 0x80 | table_bits(palette));
    out.push(packed | if interlaced { 0x40 } else { 0 });
    if let Some(palette) = palette {
        out.extend(palette.iter().flatten().copied());
    }
    out.push(minimum);
    for chunk in compressed.chunks(chunk_size) {
        out.push(u8::try_from(chunk.len()).expect("Unterblock bis 255 Bytes"));
        out.extend_from_slice(chunk);
    }
    out.push(0);
    out
}

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    position: usize,
}

impl Bits {
    fn code(&mut self, code: u16, width: u8) {
        for bit in 0..usize::from(width) {
            if self.position / 8 == self.bytes.len() {
                self.bytes.push(0);
            }
            let value = u8::try_from((code >> bit) & 1).expect("ein Bit");
            self.bytes[self.position / 8] |= value << (self.position % 8);
            self.position += 1;
        }
    }
}

/// Vor dem n-ten Literal sind höchstens n−1 neue Wörter angelegt. Das
/// Wörterbuch enthält dabei keine ausgegebenen zusammengesetzten Wörter.
fn literal_width(minimum: u8, previous_literals: usize) -> u8 {
    let next = ((1_usize << minimum) + 2 + previous_literals.saturating_sub(1)).min(4096);
    let mut width = minimum + 1;
    while width < 12 && next >= 1_usize << width {
        width += 1;
    }
    width
}

fn literals(indices: &[u8], minimum: u8, reset_after: usize) -> Vec<u8> {
    literal_stream(indices, minimum, reset_after, true)
}

fn literal_stream(indices: &[u8], minimum: u8, reset_after: usize, initial_clear: bool) -> Vec<u8> {
    let clear = 1_u16 << minimum;
    let mut bits = Bits::default();
    if initial_clear {
        bits.code(clear, minimum + 1);
    }
    let mut count = 0;
    for &index in indices {
        if count == reset_after {
            bits.code(clear, literal_width(minimum, count));
            count = 0;
        }
        bits.code(u16::from(index), literal_width(minimum, count));
        count += 1;
    }
    bits.code(clear + 1, literal_width(minimum, count));
    bits.bytes
}

fn gce(transparent: u8) -> Vec<u8> {
    vec![0x21, 0xF9, 4, 1, 0, 0, transparent, 0]
}

fn rgba(indices: &[u8], palette: &[[u8; 3]]) -> Vec<u8> {
    indices
        .iter()
        .flat_map(|&index| {
            let [r, g, b] = palette[usize::from(index)];
            [r, g, b, 255]
        })
        .collect()
}

#[test]
fn gif87a_one_pixel_and_owned_pixels() {
    // Ein schwarzes Pixel: Clear 4, Literal 0, Ende 5, je drei Bits.
    let bytes =
        b"GIF87a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff,\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0;";
    let decoded = decode(bytes, LIMIT).expect("GIF87a");
    assert_eq!((decoded.width(), decoded.height()), (1, 1));
    assert_eq!(decoded.pixels(), [0, 0, 0, 255]);
    assert_eq!(decoded.into_rgba8(), [0, 0, 0, 255]);
}

#[test]
fn initial_literal_without_clear_is_allowed() {
    // Appendix F empfiehlt Encodern einen anfänglichen Clear, verlangt
    // ihn aber nicht. Literal 0 und Ende 5 ergeben das Datenbyte 0x28.
    let frame = image([0, 0, 1, 1], None, false, 2, &[0x28], 255);
    let bytes = file([1, 1], Some(&BLACK_WHITE), 0, &[frame]);
    assert_eq!(
        decode(&bytes, LIMIT).expect("Literal ohne Clear").pixels(),
        [0, 0, 0, 255]
    );
}

#[test]
fn dictionary_words_without_initial_clear_keep_pixel_order() {
    let mut bits = Bits::default();
    for (code, width) in [(0, 3), (1, 3), (6, 3), (8, 4), (5, 4)] {
        bits.code(code, width);
    }
    let frame = image([0, 0, 7, 1], None, false, 2, &bits.bytes, 1);
    let bytes = file([7, 1], Some(&BLACK_WHITE), 0, &[frame]);
    assert_eq!(
        decode(&bytes, LIMIT)
            .expect("Wörterbuch ohne anfänglichen Clear")
            .pixels(),
        rgba(&[0, 1, 0, 1, 0, 1, 0], &BLACK_WHITE)
    );
}

#[test]
fn local_palette_takes_precedence_and_works_without_global_palette() {
    let local = [[255, 0, 0], [0, 255, 0]];
    let frame = image(
        [0, 0, 2, 1],
        Some(&local),
        false,
        2,
        &literals(&[0, 1], 2, usize::MAX),
        255,
    );
    for global in [None, Some(BLACK_WHITE.as_slice())] {
        let bytes = file([2, 1], global, 0, std::slice::from_ref(&frame));
        assert_eq!(
            decode(&bytes, LIMIT).expect("lokale Palette").pixels(),
            rgba(&[0, 1], &local)
        );
    }
}

#[test]
fn offset_frame_uses_logical_screen_background() {
    let frame = image(
        [1, 1, 1, 1],
        None,
        false,
        2,
        &literals(&[0], 2, usize::MAX),
        255,
    );
    let bytes = file([3, 3], Some(&BLACK_WHITE), 1, &[frame]);
    let decoded = decode(&bytes, LIMIT).expect("versetztes Bild");
    assert_eq!((decoded.width(), decoded.height()), (3, 3));
    assert_eq!(
        decoded.pixels(),
        rgba(&[1, 1, 1, 1, 0, 1, 1, 1, 1], &BLACK_WHITE)
    );
}

#[test]
fn transparency_applies_after_comments_and_application_extensions() {
    let frame = image(
        [1, 0, 2, 1],
        None,
        false,
        2,
        &literals(&[0, 1], 2, usize::MAX),
        255,
    );
    let comment = b"\x21\xfe\x03abc\0".to_vec();
    let application = b"\x21\xff\x0bNETSCAPE2.0\x03\x01\0\0\0".to_vec();
    let bytes = file(
        [4, 1],
        Some(&BLACK_WHITE),
        1,
        &[gce(0), comment, application, frame],
    );
    assert_eq!(
        decode(&bytes, LIMIT).expect("Transparenz").pixels(),
        [0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0]
    );
}

#[test]
fn plain_text_consumes_graphics_control_extension() {
    let mut text = vec![0x21, 0x01, 12];
    text.extend_from_slice(&[0; 12]);
    text.extend_from_slice(&[1, b'x', 0]);
    let frame = image(
        [1, 0, 1, 1],
        None,
        false,
        2,
        &literals(&[0], 2, usize::MAX),
        255,
    );
    let bytes = file([3, 1], Some(&BLACK_WHITE), 1, &[gce(0), text, frame]);
    assert_eq!(
        decode(&bytes, LIMIT).expect("Plain Text").pixels(),
        rgba(&[1, 0, 1], &BLACK_WHITE)
    );
}

#[test]
fn interlaced_rows_with_frame_offset() {
    let palette: Vec<[u8; 3]> = (0..16).map(|n| [n * 16, 255 - n * 16, n * 8]).collect();
    let mut indices = Vec::new();
    for row in [0, 8, 4, 2, 6, 1, 3, 5, 7, 9] {
        indices.extend_from_slice(&[row; 3]);
    }
    let frame = image(
        [1, 2, 3, 10],
        None,
        true,
        4,
        &literals(&indices, 4, usize::MAX),
        2,
    );
    let bytes = file([5, 14], Some(&palette), 15, &[frame]);
    let mut expected = vec![15; 5 * 14];
    for row in 0..10 {
        for column in 1..4 {
            expected[(row + 2) * 5 + column] = u8::try_from(row).expect("Zeile");
        }
    }
    assert_eq!(
        decode(&bytes, LIMIT).expect("Interlace").pixels(),
        rgba(&expected, &palette)
    );
}

#[test]
fn dictionary_special_case_reuses_previous_word() {
    let mut bits = Bits::default();
    for code in [4, 0, 6, 5] {
        bits.code(code, 3);
    }
    let frame = image([0, 0, 3, 1], None, false, 2, &bits.bytes, 1);
    let bytes = file([3, 1], Some(&BLACK_WHITE), 0, &[frame]);
    assert_eq!(
        decode(&bytes, LIMIT).expect("KwKwK").pixels(),
        rgba(&[0, 0, 0], &BLACK_WHITE)
    );
}

#[test]
fn dictionary_words_and_longer_special_case_keep_pixel_order() {
    let mut bits = Bits::default();
    // Nach 0, 1 enthält Eintrag 6 das Wort 01. Eintrag 8 ist beim
    // folgenden Code noch nicht angelegt: Er wird als 01 + 0 gelesen.
    for (code, width) in [(4, 3), (0, 3), (1, 3), (6, 3), (8, 4), (5, 4)] {
        bits.code(code, width);
    }
    let frame = image([0, 0, 7, 1], None, false, 2, &bits.bytes, 1);
    let bytes = file([7, 1], Some(&BLACK_WHITE), 0, &[frame]);
    assert_eq!(
        decode(&bytes, LIMIT).expect("Wörterbuchwörter").pixels(),
        rgba(&[0, 1, 0, 1, 0, 1, 0], &BLACK_WHITE)
    );
}

#[test]
fn code_widths_grow_to_twelve_bits_dictionary_fills_and_clear_resets_it() {
    let palette: Vec<[u8; 3]> = (0..=255).map(|n| [n, 255 - n, n / 2]).collect();
    let indices: Vec<u8> = (0..10_000)
        .map(|n| u8::try_from(n % 256).expect("Index"))
        .collect();
    // 5.000 Literale füllen das Wörterbuch bis 4.096 Einträge, ohne einen
    // Clear zu erzwingen. Der anschließende Clear wird noch mit 12 Bits
    // geschrieben, danach beginnen die Codebreiten wieder bei neun.
    let compressed = literals(&indices, 8, 5000);
    let frame = image([0, 0, 100, 100], None, false, 8, &compressed, 1);
    let bytes = file([100, 100], Some(&palette), 0, &[frame]);
    assert_eq!(
        decode(&bytes, LIMIT).expect("alle Codebreiten").pixels(),
        rgba(&indices, &palette)
    );
}

#[test]
fn code_width_growth_and_later_clear_work_without_initial_clear() {
    let palette: Vec<[u8; 3]> = (0..=255).map(|n| [n, 255 - n, n / 2]).collect();
    let indices: Vec<u8> = (0..10_000)
        .map(|n| u8::try_from(n % 256).expect("Index"))
        .collect();
    let compressed = literal_stream(&indices, 8, 5000, false);
    let frame = image([0, 0, 100, 100], None, false, 8, &compressed, 1);
    let bytes = file([100, 100], Some(&palette), 0, &[frame]);
    assert_eq!(
        decode(&bytes, LIMIT)
            .expect("Codebreiten ohne anfänglichen Clear")
            .pixels(),
        rgba(&indices, &palette)
    );
}

#[test]
fn consecutive_clear_codes_are_allowed() {
    let mut bits = Bits::default();
    for code in [4, 4, 0, 4, 1, 5] {
        bits.code(code, 3);
    }
    let frame = image([0, 0, 2, 1], None, false, 2, &bits.bytes, 255);
    let bytes = file([2, 1], Some(&BLACK_WHITE), 0, &[frame]);
    assert_eq!(
        decode(&bytes, LIMIT).expect("mehrfache Clears").pixels(),
        rgba(&[0, 1], &BLACK_WHITE)
    );
}

#[test]
fn first_frame_does_not_decode_later_frames() {
    let frame = image(
        [0, 0, 1, 1],
        None,
        false,
        2,
        &literals(&[1], 2, usize::MAX),
        255,
    );
    let mut bytes = file([1, 1], Some(&BLACK_WHITE), 0, &[frame]);
    bytes.pop();
    bytes.extend_from_slice(b"\x2cnot a valid second image");
    assert_eq!(
        decode(&bytes, LIMIT).expect("nur erstes Bild").pixels(),
        rgba(&[1], &BLACK_WHITE)
    );
}

#[test]
fn every_truncation_before_first_frame_terminator_fails() {
    let frame = image(
        [0, 0, 1, 1],
        None,
        false,
        2,
        &literals(&[0], 2, usize::MAX),
        1,
    );
    let bytes = file([1, 1], Some(&BLACK_WHITE), 0, &[gce(0), frame]);
    for length in 0..bytes.len() - 1 {
        assert!(decode(&bytes[..length], LIMIT).is_err(), "Länge {length}");
    }
    assert!(
        decode(&bytes[..bytes.len() - 1], LIMIT).is_ok(),
        "Trailer nicht nötig"
    );
}

#[test]
fn dimensions_and_pixel_limit_are_checked_before_output() {
    assert!(matches!(
        decode(&file([0, 1], None, 0, &[]), LIMIT),
        Err(Error::Dimensions)
    ));
    assert!(matches!(
        decode(&file([u16::MAX, u16::MAX], None, 0, &[]), 1024),
        Err(Error::TooLarge)
    ));
    let frame = image(
        [1, 0, 1, 1],
        None,
        false,
        2,
        &literals(&[0], 2, usize::MAX),
        255,
    );
    assert!(matches!(
        decode(&file([1, 1], Some(&BLACK_WHITE), 0, &[frame]), LIMIT),
        Err(Error::Dimensions)
    ));
    let frame = image(
        [0, 0, 0, 1],
        None,
        false,
        2,
        &literals(&[], 2, usize::MAX),
        255,
    );
    assert!(matches!(
        decode(&file([1, 1], Some(&BLACK_WHITE), 0, &[frame]), LIMIT),
        Err(Error::Dimensions)
    ));
}

#[test]
fn palettes_and_indices_are_validated() {
    let frame = image(
        [0, 0, 1, 1],
        None,
        false,
        2,
        &literals(&[0], 2, usize::MAX),
        255,
    );
    assert!(matches!(
        decode(&file([1, 1], None, 0, &[frame]), LIMIT),
        Err(Error::MissingPalette)
    ));
    let frame = image(
        [0, 0, 1, 1],
        None,
        false,
        2,
        &literals(&[2], 2, usize::MAX),
        255,
    );
    assert!(matches!(
        decode(&file([1, 1], Some(&BLACK_WHITE), 0, &[frame]), LIMIT),
        Err(Error::PaletteIndex)
    ));
    assert!(matches!(
        decode(&file([1, 1], Some(&BLACK_WHITE), 2, &[]), LIMIT),
        Err(Error::PaletteIndex)
    ));
}

#[test]
fn short_and_excess_pixel_streams_are_rejected() {
    for indices in [&[0][..], &[0, 1, 0][..]] {
        let frame = image(
            [0, 0, 2, 1],
            None,
            false,
            2,
            &literals(indices, 2, usize::MAX),
            255,
        );
        assert!(matches!(
            decode(&file([2, 1], Some(&BLACK_WHITE), 0, &[frame]), LIMIT),
            Err(Error::PixelCount)
        ));
    }
}

#[test]
fn invalid_lzw_codes_and_code_sizes_are_rejected() {
    for minimum in [0, 1, 9, 255] {
        let frame = image([0, 0, 1, 1], None, false, minimum, &[0], 255);
        assert!(matches!(
            decode(&file([1, 1], Some(&BLACK_WHITE), 0, &[frame]), LIMIT),
            Err(Error::LzwCodeSize)
        ));
    }
    for codes in [&[6, 5][..], &[4, 0, 7, 5][..]] {
        let mut bits = Bits::default();
        for &code in codes {
            bits.code(code, 3);
        }
        let frame = image([0, 0, 1, 1], None, false, 2, &bits.bytes, 255);
        assert!(matches!(
            decode(&file([1, 1], Some(&BLACK_WHITE), 0, &[frame]), LIMIT),
            Err(Error::Lzw)
        ));
    }
}

#[test]
fn invalid_blocks_and_graphics_control_terminators_are_rejected() {
    assert!(matches!(decode(b"notGIF", LIMIT), Err(Error::Signature)));
    assert!(matches!(
        decode(&file([1, 1], Some(&BLACK_WHITE), 0, &[]), LIMIT),
        Err(Error::Block)
    ));
    let mut control = gce(0);
    control[7] = 1;
    assert!(matches!(
        decode(&file([1, 1], Some(&BLACK_WHITE), 0, &[control]), LIMIT),
        Err(Error::Block)
    ));
}
