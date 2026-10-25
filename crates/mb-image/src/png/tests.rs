use std::collections::BTreeMap;

use mb_compress::{adler32, crc32};

use super::*;

const LIMIT: u64 = 1 << 24;

/// Die Ordner mit Testbildern in image-png.
const DIRECTORIES: [&str; 3] = ["tests/pngsuite", "tests/pngsuite-extra", "tests/bugfixes"];

/// Woran die Bilder scheitern, deren Name mit x beginnt, nach ihren Chunks.
const FAILURES: [(&str, Error); 22] = [
    // Farbtyp 1 und 9.
    ("xc1n0g08.png", Error::Header),
    ("xc9n2c08.png", Error::Header),
    // Bittiefe 0, 3 und 99.
    ("xd0n2c08.png", Error::Header),
    ("xd3n2c08.png", Error::Header),
    ("xd9n2c08.png", Error::Header),
    // Die Signatur ist verändert, auch durch eingefügte oder umgewandelte
    // Zeilenenden.
    ("xcrn0g04.png", Error::Signature),
    ("xlfn0g04.png", Error::Signature),
    ("xs1n0g01.png", Error::Signature),
    ("xs2n0g01.png", Error::Signature),
    ("xs4n0g01.png", Error::Signature),
    ("xs7n0g01.png", Error::Signature),
    // Falsche CRC bei IDAT und IHDR.
    ("xcsn0g01.png", Error::Checksum),
    ("xhdn0g08.png", Error::Checksum),
    ("x_interlaced_chck.png", Error::Checksum),
    ("xdtn0g01.png", Error::MissingImageData),
    // gAMA zwischen IDAT-Chunks.
    ("x_consecutive_idat.png", Error::Order),
    // Der erste Chunk ist IDAT, bei der anderen Datei ein abgeschnittener
    // Chunk mit einem Typ aus Nullen und Buchstaben.
    ("x_ihdr_missing.png", Error::MissingHeader),
    ("x_unexpected_eof.png", Error::MissingHeader),
    // Breite oder Höhe 0, und eine Breite über 2³¹ − 1.
    ("x_issue#214.png", Error::Header),
    ("x_issue#230.0.png", Error::Header),
    ("x_issue#230.1.png", Error::Header),
    // Ein leerer IDAT.
    (
        "x_image-issue#1075.png",
        Error::Compression(mb_compress::Error::UnexpectedEnd),
    ),
];

/// Eine Ergebnisliste von image-png: Dateiname und CRC-32, keine bei
/// „Expected failure“. Die Listen führen `x_interlaced_chck.png` unter
/// `tests/pngsuite/`, die Datei liegt aber in `tests/bugfixes/`; image-png
/// vergleicht bei den erwarteten Fehlern nur die Anzahl. Deshalb wird hier
/// über den Dateinamen zugeordnet.
fn results(path: &str) -> BTreeMap<String, Option<u32>> {
    testdata::read("image-png", path)
        .lines()
        .map(|line| {
            let (file, value) = line.split_once(": ").expect("Zeile mit „: “");
            let name = file.rsplit('/').next().unwrap_or(file).to_owned();
            let crc = (value != "Expected failure").then(|| value.parse().expect("Prüfsumme"));
            (name, crc)
        })
        .collect()
}

/// Die Ausgabe von image-png mit `EXPAND | STRIP_16`, aus unserer
/// RGBA-Ausgabe gewonnen: Grau bleibt ein Kanal, eine Palette wird RGB, und
/// tRNS wird zum Alphakanal.
fn expanded(png: &Png) -> Vec<u8> {
    let alpha = *png.transparency() != Transparency::None;
    let channels: &[usize] = match (png.color_type(), alpha) {
        (ColorType::Grey, false) => &[0],
        (ColorType::Grey, true) | (ColorType::GreyAlpha, _) => &[0, 3],
        (ColorType::Truecolor | ColorType::Indexed, false) => &[0, 1, 2],
        (ColorType::Truecolor | ColorType::Indexed, true) | (ColorType::TruecolorAlpha, _) => {
            &[0, 1, 2, 3]
        }
    };
    let rgba = png.to_rgba8();
    let (pixels, _) = rgba.as_chunks::<4>();
    pixels
        .iter()
        .flat_map(|pixel| {
            channels
                .iter()
                .filter_map(move |&channel| pixel.get(channel).copied())
        })
        .collect()
}

/// Die Bilder aus den Tests von image-png mit dessen Prüfsummen:
/// `results_identity.txt` für [`Png::data`] (dort `Transformations::IDENTITY`)
/// und `results.txt` für [`Png::to_rgba8`].
#[test]
fn image_png_test_images() {
    let identity = results("tests/results_identity.txt");
    let normalized = results("tests/results.txt");
    let failures: BTreeMap<&str, Error> = FAILURES.into_iter().collect();
    let mut files = 0;
    for directory in DIRECTORIES {
        for name in testdata::files("image-png", directory, "png") {
            files += 1;
            let bytes = testdata::read_bytes("image-png", &format!("{directory}/{name}"));
            let decoded = decode(&bytes, LIMIT);
            match (identity.get(&name), normalized.get(&name)) {
                (Some(Some(raw)), Some(Some(rgba))) => {
                    let png = decoded.unwrap_or_else(|error| panic!("{name}: {error}"));
                    assert_eq!(crc32(png.data()), *raw, "{name}");
                    assert_eq!(crc32(&expanded(&png)), *rgba, "{name}, RGBA");
                }
                (Some(None), Some(None)) => {
                    assert_eq!(
                        decoded.err(),
                        failures.get(name.as_str()).copied(),
                        "{name}"
                    );
                }
                other => panic!("{name}: {other:?}"),
            }
        }
    }
    assert_eq!(
        (files, identity.len(), normalized.len(), failures.len()),
        (195, 195, 195, 22)
    );
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let length = u32::try_from(data.len()).expect("kurzer Chunk");
    let mut out = length.to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.value().to_be_bytes());
    out
}

/// Ein Chunk, dessen CRC nicht passt.
fn damaged(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = chunk(kind, data);
    if let Some(last) = out.last_mut() {
        *last ^= 1;
    }
    out
}

fn ihdr(width: u32, height: u32, bit_depth: u8, color_type: u8, interlace: u8) -> Vec<u8> {
    let mut data = width.to_be_bytes().to_vec();
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&[bit_depth, color_type, 0, 0, interlace]);
    chunk(b"IHDR", &data)
}

/// Ein zlib-Strom aus einem Block ohne Kompression.
fn zlib(data: &[u8]) -> Vec<u8> {
    let length = u16::try_from(data.len()).expect("kurze Daten");
    let mut out = vec![0x78, 0x01, 0x01];
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&(!length).to_le_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn idat(raw: &[u8]) -> Vec<u8> {
    chunk(b"IDAT", &zlib(raw))
}

fn iend() -> Vec<u8> {
    chunk(b"IEND", &[])
}

fn file(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut out = SIGNATURE.to_vec();
    for chunk in chunks {
        out.extend_from_slice(chunk);
    }
    out
}

fn rgba(chunks: &[Vec<u8>]) -> Vec<u8> {
    decode(&file(chunks), LIMIT)
        .expect("gültiges Bild")
        .to_rgba8()
}

#[test]
fn conversion_to_rgba() {
    // Grau mit 2 Bit: 0, 1, 2, 3 werden 0, 85, 170, 255. tRNS nennt 0x0102,
    // davon zählen die unteren 2 Bit, also der Wert 2.
    let grey = [ihdr(4, 1, 2, 0, 0), idat(&[0, 0b0001_1011])];
    assert_eq!(
        rgba(&[grey[0].clone(), grey[1].clone(), iend()]),
        [
            0, 0, 0, 255, 85, 85, 85, 255, 170, 170, 170, 255, 255, 255, 255, 255
        ]
    );
    let png = decode(
        &file(&[
            grey[0].clone(),
            chunk(b"tRNS", &[1, 2]),
            grey[1].clone(),
            iend(),
        ]),
        LIMIT,
    )
    .expect("Grau mit tRNS");
    assert_eq!(*png.transparency(), Transparency::Grey(2));
    assert_eq!(
        png.to_rgba8(),
        [
            0, 0, 0, 255, 85, 85, 85, 255, 170, 170, 170, 0, 255, 255, 255, 255
        ]
    );

    // Grau mit 16 Bit: Das obere Byte bleibt, tRNS vergleicht alle 16 Bit.
    assert_eq!(
        rgba(&[
            ihdr(3, 1, 16, 0, 0),
            chunk(b"tRNS", &[0x12, 0x34]),
            idat(&[0, 0x12, 0x34, 0x12, 0x35, 0xFF, 0x00]),
            iend(),
        ]),
        [
            0x12, 0x12, 0x12, 0, 0x12, 0x12, 0x12, 255, 0xFF, 0xFF, 0xFF, 255
        ]
    );

    // Truecolor mit tRNS.
    assert_eq!(
        rgba(&[
            ihdr(2, 1, 8, 2, 0),
            chunk(b"tRNS", &[0, 1, 0, 2, 0, 3]),
            idat(&[0, 1, 2, 3, 1, 2, 4]),
            iend(),
        ]),
        [1, 2, 3, 0, 1, 2, 4, 255]
    );

    // Palette mit kürzerem tRNS und einem Index außerhalb der Palette.
    assert_eq!(
        rgba(&[
            ihdr(4, 1, 8, 3, 0),
            chunk(b"PLTE", &[10, 11, 12, 20, 21, 22, 30, 31, 32]),
            chunk(b"tRNS", &[0, 128]),
            idat(&[0, 0, 1, 2, 3]),
            iend(),
        ]),
        [
            10, 11, 12, 0, 20, 21, 22, 128, 30, 31, 32, 255, 0, 0, 0, 255
        ]
    );

    // Grau mit Alpha in 16 Bit und RGBA in 8 Bit.
    assert_eq!(
        rgba(&[
            ihdr(1, 1, 16, 4, 0),
            idat(&[0, 0xAB, 0xCD, 0x80, 0x01]),
            iend()
        ]),
        [0xAB, 0xAB, 0xAB, 0x80]
    );
    assert_eq!(
        rgba(&[ihdr(1, 1, 8, 6, 0), idat(&[0, 1, 2, 3, 4]), iend()]),
        [1, 2, 3, 4]
    );
}

#[test]
fn unused_bits_are_cleared() {
    // Drei Pixel mit 1 Bit: Die letzten fünf Bits der Zeile sind nicht
    // festgelegt.
    let png = decode(
        &file(&[ihdr(3, 1, 1, 0, 0), idat(&[0, 0b1011_1111]), iend()]),
        LIMIT,
    )
    .expect("Grau mit 1 Bit");
    assert_eq!(png.data(), [0b1010_0000]);
    assert_eq!(png.row_bytes(), 1);
}

#[test]
fn chunk_rules() {
    let header = ihdr(1, 1, 8, 0, 0);
    let data = idat(&[0, 7]);
    let decoded = |chunks: &[Vec<u8>]| decode(&file(chunks), LIMIT).map(|png| png.to_rgba8());
    let pixel = Ok(vec![7, 7, 7, 255]);

    // Zusatz-Chunks: unbekannte und solche mit falscher CRC werden
    // übergangen.
    assert_eq!(
        decoded(&[
            header.clone(),
            chunk(b"prIv", &[1, 2, 3]),
            damaged(b"tRNS", &[0, 7]),
            data.clone(),
            iend(),
        ]),
        pixel
    );
    // Kritische Chunks: unbekannte und falsche CRC sind fatal.
    assert_eq!(
        decoded(&[header.clone(), chunk(b"CRIT", &[]), data.clone(), iend()]),
        Err(Error::UnknownCritical)
    );
    assert_eq!(
        decoded(&[header.clone(), damaged(b"IDAT", &zlib(&[0, 7])), iend()]),
        Err(Error::Checksum)
    );
    assert_eq!(
        decoded(&[damaged(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0])]),
        Err(Error::Checksum)
    );
    assert_eq!(
        decoded(&[header.clone(), chunk(b"ab1d", &[]), data.clone(), iend()]),
        Err(Error::ChunkType)
    );
    let mut too_long = file(std::slice::from_ref(&header));
    too_long.extend_from_slice(&[0x80, 0, 0, 0]);
    too_long.extend_from_slice(b"teXt");
    assert_eq!(decode(&too_long, LIMIT).err(), Some(Error::ChunkLength));

    // IHDR zuerst und nur einmal.
    assert_eq!(decoded(&[data.clone(), iend()]), Err(Error::MissingHeader));
    assert_eq!(
        decoded(&[header.clone(), header.clone(), data.clone(), iend()]),
        Err(Error::Order)
    );
    assert_eq!(
        decoded(&[header.clone(), iend()]),
        Err(Error::MissingImageData)
    );

    // IEND: Ohne ist es ein Fehler, ein beschädigter und alles danach zählt
    // nicht.
    assert_eq!(
        decoded(&[header.clone(), data.clone()]),
        Err(Error::UnexpectedEnd)
    );
    assert_eq!(
        decoded(&[
            header.clone(),
            data.clone(),
            damaged(b"IEND", &[]),
            vec![1, 2, 3]
        ]),
        pixel
    );
    let mut cut = file(&[header.clone(), data.clone()]);
    cut.truncate(cut.len() - 1);
    assert_eq!(decode(&cut, LIMIT).err(), Some(Error::UnexpectedEnd));
    assert_eq!(decode(&SIGNATURE[..7], LIMIT).err(), Some(Error::Signature));

    // IDAT: beliebig geteilt, auch leer, aber ohne andere Chunks dazwischen.
    let stream = zlib(&[0, 7]);
    let (start, end) = stream.split_at(3);
    assert_eq!(
        decoded(&[
            header.clone(),
            chunk(b"IDAT", start),
            chunk(b"IDAT", &[]),
            chunk(b"IDAT", end),
            iend(),
        ]),
        pixel
    );
    assert_eq!(
        decoded(&[
            header.clone(),
            chunk(b"IDAT", start),
            chunk(b"tEXt", b"a\0b"),
            chunk(b"IDAT", end),
            iend(),
        ]),
        Err(Error::Order)
    );

    // Nach dem zlib-Strom darf noch etwas kommen, die Bilddaten selbst
    // müssen genau passen.
    let mut padded = stream.clone();
    padded.extend_from_slice(&[0, 0, 0]);
    assert_eq!(
        decoded(&[header.clone(), chunk(b"IDAT", &padded), iend()]),
        pixel
    );
    assert_eq!(
        decoded(&[header.clone(), idat(&[0]), iend()]),
        Err(Error::ImageDataLength)
    );
    assert_eq!(
        decoded(&[header.clone(), idat(&[0, 7, 0]), iend()]),
        Err(Error::ImageDataLength)
    );
    assert_eq!(
        decoded(&[header.clone(), idat(&[5, 7]), iend()]),
        Err(Error::Filter)
    );

    // Die Grenze für die Pixel.
    let square = file(&[ihdr(2, 2, 8, 0, 0), idat(&[0, 1, 2, 0, 3, 4]), iend()]);
    assert_eq!(decode(&square, 3).err(), Some(Error::TooLarge));
    assert!(decode(&square, 4).is_ok());
}

#[test]
fn header_values() {
    let decoded = |header: Vec<u8>| decode(&file(&[header, idat(&[0, 7]), iend()]), LIMIT).err();
    assert_eq!(decoded(ihdr(1, 1, 8, 0, 0)), None);
    assert_eq!(decoded(ihdr(0, 1, 8, 0, 0)), Some(Error::Header));
    assert_eq!(decoded(ihdr(1, 0x8000_0000, 8, 0, 0)), Some(Error::Header));
    assert_eq!(decoded(ihdr(1, 1, 3, 0, 0)), Some(Error::Header));
    assert_eq!(decoded(ihdr(1, 1, 16, 3, 0)), Some(Error::Header));
    assert_eq!(decoded(ihdr(1, 1, 8, 5, 0)), Some(Error::Header));
    assert_eq!(decoded(ihdr(1, 1, 8, 0, 2)), Some(Error::Header));
    let mut compression = ihdr(1, 1, 8, 0, 0);
    // Länge (4), Typ (4), Breite, Höhe, Tiefe und Farbtyp (10): dann die
    // Kompression.
    if let Some(byte) = compression.get_mut(18) {
        *byte = 1;
    }
    let length = compression.len();
    let data = compression.get(8..length - 4).unwrap_or_default().to_vec();
    assert_eq!(decoded(chunk(b"IHDR", &data)), Some(Error::Header));
    assert_eq!(
        decoded(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0])),
        Some(Error::Header)
    );
}

#[test]
fn palette_and_transparency_rules() {
    let header = ihdr(2, 1, 1, 3, 0);
    let palette = chunk(b"PLTE", &[1, 2, 3, 4, 5, 6]);
    // Pixel 0 und 1.
    let data = idat(&[0, 0b0100_0000]);
    let decoded = |chunks: &[Vec<u8>]| decode(&file(chunks), LIMIT);
    let transparency = |chunks: &[Vec<u8>]| {
        decoded(chunks)
            .map(|png| png.transparency().clone())
            .expect("gültiges Bild")
    };

    assert_eq!(
        decoded(&[header.clone(), data.clone(), iend()]).err(),
        Some(Error::MissingPalette)
    );
    for entries in [&[1, 2, 3, 4][..], &[], &[1, 2, 3, 4, 5, 6, 7, 8, 9]] {
        assert_eq!(
            decoded(&[
                header.clone(),
                chunk(b"PLTE", entries),
                data.clone(),
                iend()
            ])
            .err(),
            Some(Error::Palette),
            "{entries:?}"
        );
    }
    assert_eq!(
        decoded(&[
            header.clone(),
            palette.clone(),
            palette.clone(),
            data.clone(),
            iend()
        ])
        .err(),
        Some(Error::Order)
    );
    assert_eq!(
        decoded(&[
            header.clone(),
            palette.clone(),
            data.clone(),
            palette.clone(),
            iend()
        ])
        .err(),
        Some(Error::Order)
    );
    let png = decoded(&[header.clone(), palette.clone(), data.clone(), iend()])
        .expect("Bild mit Palette");
    assert_eq!(png.palette(), [[1, 2, 3], [4, 5, 6]]);
    assert_eq!(png.to_rgba8(), [1, 2, 3, 255, 4, 5, 6, 255]);

    // Ohne Palette im Farbtyp wird PLTE übergangen, auch ein fehlerhafter.
    let png = decoded(&[
        ihdr(1, 1, 8, 0, 0),
        chunk(b"PLTE", &[1, 2, 3, 4]),
        idat(&[0, 7]),
        iend(),
    ])
    .expect("Grau mit PLTE");
    assert!(png.palette().is_empty());

    // tRNS: nach PLTE, vor IDAT, nicht länger als die Palette, nur der
    // erste zählt, und bei Farbtypen mit Alpha gar nicht.
    let alphas = chunk(b"tRNS", &[9]);
    assert_eq!(
        transparency(&[
            header.clone(),
            palette.clone(),
            alphas.clone(),
            data.clone(),
            iend()
        ]),
        Transparency::Palette(vec![9])
    );
    assert_eq!(
        transparency(&[
            header.clone(),
            alphas.clone(),
            palette.clone(),
            data.clone(),
            iend()
        ]),
        Transparency::None
    );
    assert_eq!(
        transparency(&[
            header.clone(),
            palette.clone(),
            data.clone(),
            alphas.clone(),
            iend()
        ]),
        Transparency::None
    );
    assert_eq!(
        transparency(&[
            header.clone(),
            palette.clone(),
            chunk(b"tRNS", &[1, 2, 3]),
            data.clone(),
            iend(),
        ]),
        Transparency::None
    );
    assert_eq!(
        transparency(&[
            header.clone(),
            palette.clone(),
            alphas.clone(),
            chunk(b"tRNS", &[1, 2]),
            data.clone(),
            iend(),
        ]),
        Transparency::Palette(vec![9])
    );
    assert_eq!(
        transparency(&[
            ihdr(1, 1, 8, 4, 0),
            chunk(b"tRNS", &[0, 7]),
            idat(&[0, 7, 255]),
            iend(),
        ]),
        Transparency::None
    );
}

/// Die Bilder der Referenztests für M1 (`tools/testrunner/m1-referenztests.txt`)
/// sind einfarbig.
#[test]
fn m1_reftest_images() {
    let images: [(&str, [u8; 4]); 7] = [
        (
            "css/CSS2/normal-flow/support/1x1-green.png",
            [0, 128, 0, 255],
        ),
        (
            "css/CSS2/normal-flow/support/black96x96.png",
            [0, 0, 0, 255],
        ),
        (
            "css/CSS2/normal-flow/support/blue15x15.png",
            [0, 0, 255, 255],
        ),
        (
            "css/CSS2/normal-flow/support/blue96x96.png",
            [0, 0, 255, 255],
        ),
        (
            "css/CSS2/normal-flow/support/swatch-green.png",
            [0, 128, 0, 255],
        ),
        (
            "css/CSS2/normal-flow/support/swatch-orange.png",
            [255, 165, 0, 255],
        ),
        ("css/CSS2/support/black96x96.png", [0, 0, 0, 255]),
    ];
    for (path, color) in images {
        let png = decode(&testdata::read_bytes("wpt", path), LIMIT)
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        let rgba = png.to_rgba8();
        let (pixels, rest) = rgba.as_chunks::<4>();
        let count = u64::from(png.width()) * u64::from(png.height());
        assert_eq!(u64::try_from(pixels.len()), Ok(count), "{path}");
        assert!(
            rest.is_empty() && pixels.iter().all(|pixel| *pixel == color),
            "{path}"
        );
    }
}
