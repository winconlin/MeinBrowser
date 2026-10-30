//! PNG nach der PNG Specification (Fourth Edition, Editor's Draft im
//! Repository w3c/png, Commit e200ab5).
//!
//! [`decode`] liest ein ganzes Bild aus einem Puffer: die kritischen Chunks
//! und von den übrigen nur tRNS. Bei einer Animation (APNG) ist das Ergebnis
//! das statische Bild. Farbmanagement (gAMA, cHRM, sRGB, iCCP, cICP) wendet
//! der Dekoder noch nicht an.
//!
//! Wo die Spezifikation dem Dekoder die Wahl lässt, gilt:
//!
//! - Ein Fehler in einem kritischen Chunk ist fatal, auch eine falsche CRC.
//!   Ein Zusatz-Chunk mit falscher CRC wird übergangen, ebenso einer mit
//!   unerwarteten Werten oder an falscher Stelle (Abschnitt „Error
//!   checking“).
//! - Bei Farbtypen ohne Palette wird PLTE übergangen, auch ein fehlerhafter.
//!   Für Truecolor erlaubt die Spezifikation das ausdrücklich; für Grau, wo
//!   PLTE nicht stehen darf, gilt hier dasselbe.
//! - Von mehreren tRNS zählt nur der erste.
//! - IEND beendet das Lesen. Länge und CRC von IEND werden nicht geprüft,
//!   was danach kommt, zählt nicht. Endet die Datei vor IEND, ist das ein
//!   Fehler.
//! - Die Bilddaten müssen genau so lang sein, wie das Bild es verlangt.
//!   Bytes nach dem Ende des zlib-Stroms zählen nicht (Abschnitt „IDAT Image
//!   data“), die Prüfsumme des zlib-Stroms wird geprüft.
//!
//! Gegen die Testbilder von image-png und dessen Prüfsummen prüft
//! `png/tests.rs`.

mod pixels;

#[cfg(test)]
mod tests;

use mb_compress::{Crc32, zlib_decompress_sized};

/// So viele Bytes belegt [`decode`] höchstens je Pixel, dazu höchstens so
/// viele, wie die Eingabe lang ist, für die gesammelten Bilddaten und
/// Tabellen fester Größe für die Huffman-Codes. Je Pixel gleichzeitig: die
/// entpackten Daten mit den Filterbytes (bis zu 9 Bytes), das Bild (bis zu
/// 8 Bytes bei 16 Bit RGBA) und zwei Zeilen zum Umkehren der Filter, die bei
/// einem Bild aus einer Zeile zusammen doppelt so groß sind wie das Bild
/// (bis zu 16 Bytes). [`Png::to_rgba8`] braucht mit dem Bild zusammen
/// höchstens 12 Bytes je Pixel.
pub const DECODE_BYTES_PER_PIXEL: u64 = 33;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Die größte Zahl in vier Bytes, die PNG erlaubt: 2³¹ − 1. Sie begrenzt
/// die Länge eines Chunks und Breite und Höhe des Bildes.
const MAX_NUMBER: u32 = 0x7FFF_FFFF;

/// Der Farbtyp aus IHDR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorType {
    /// Farbtyp 0.
    Grey,
    /// Farbtyp 2.
    Truecolor,
    /// Farbtyp 3.
    Indexed,
    /// Farbtyp 4.
    GreyAlpha,
    /// Farbtyp 6.
    TruecolorAlpha,
}

impl ColorType {
    /// Die Abtastwerte je Pixel.
    pub fn channels(self) -> u8 {
        match self {
            ColorType::Grey | ColorType::Indexed => 1,
            ColorType::GreyAlpha => 2,
            ColorType::Truecolor => 3,
            ColorType::TruecolorAlpha => 4,
        }
    }

    /// Farbtyp und Bittiefe nach der Tabelle „Allowed combinations of color
    /// type and bit depth“.
    fn from_header(color_type: u8, bit_depth: u8) -> Option<ColorType> {
        let (kind, depths): (ColorType, &[u8]) = match color_type {
            0 => (ColorType::Grey, &[1, 2, 4, 8, 16]),
            2 => (ColorType::Truecolor, &[8, 16]),
            3 => (ColorType::Indexed, &[1, 2, 4, 8]),
            4 => (ColorType::GreyAlpha, &[8, 16]),
            6 => (ColorType::TruecolorAlpha, &[8, 16]),
            _ => return None,
        };
        depths.contains(&bit_depth).then_some(kind)
    }
}

/// Die Transparenz aus tRNS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transparency {
    None,
    /// Alpha je Palettenindex; wo die Liste endet, ist Alpha 255.
    Palette(Vec<u8>),
    /// Pixel mit diesem Grauwert (in der Bittiefe des Bildes) sind
    /// durchsichtig, alle anderen undurchsichtig.
    Grey(u16),
    /// Pixel mit diesen Werten für Rot, Grün und Blau sind durchsichtig.
    Rgb([u16; 3]),
}

/// Fehler beim Dekodieren.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Die ersten acht Bytes sind nicht die PNG-Signatur.
    Signature,
    /// Die Daten enden vor IEND.
    UnexpectedEnd,
    /// Ein Chunk ist länger als 2³¹ − 1 Bytes.
    ChunkLength,
    /// Der Typ eines Chunks besteht nicht aus vier Buchstaben.
    ChunkType,
    /// Die CRC eines kritischen Chunks passt nicht.
    Checksum,
    /// Der erste Chunk ist nicht IHDR.
    MissingHeader,
    /// IHDR hat die falsche Länge oder unzulässige Werte.
    Header,
    /// Ein kritischer Chunk, den der Dekoder nicht kennt.
    UnknownCritical,
    /// Ein kritischer Chunk an falscher Stelle: ein zweiter IHDR, ein
    /// zweiter PLTE, PLTE nach IDAT oder IDAT mit anderen Chunks dazwischen.
    Order,
    /// Ein Bild mit Palette hat vor IDAT keinen PLTE.
    MissingPalette,
    /// PLTE hat eine unzulässige Länge oder mehr Einträge, als die Bittiefe
    /// erlaubt.
    Palette,
    /// Es gibt keinen IDAT.
    MissingImageData,
    /// Der zlib-Strom in IDAT ist fehlerhaft.
    Compression(mb_compress::Error),
    /// Die Bilddaten sind kürzer oder länger, als das Bild es verlangt.
    ImageDataLength,
    /// Eine Zeile hat einen unbekannten Filtertyp.
    Filter,
    /// Das Bild hat mehr Pixel als erlaubt.
    TooLarge,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Signature => f.write_str("keine PNG-Signatur"),
            Error::UnexpectedEnd => f.write_str("Daten enden vor IEND"),
            Error::ChunkLength => f.write_str("Chunk zu lang"),
            Error::ChunkType => f.write_str("ungültiger Chunk-Typ"),
            Error::Checksum => f.write_str("CRC eines kritischen Chunks passt nicht"),
            Error::MissingHeader => f.write_str("IHDR fehlt am Anfang"),
            Error::Header => f.write_str("ungültiger IHDR"),
            Error::UnknownCritical => f.write_str("unbekannter kritischer Chunk"),
            Error::Order => f.write_str("kritischer Chunk an falscher Stelle"),
            Error::MissingPalette => f.write_str("PLTE fehlt vor IDAT"),
            Error::Palette => f.write_str("ungültiger PLTE"),
            Error::MissingImageData => f.write_str("kein IDAT"),
            Error::Compression(error) => write!(f, "Bilddaten: {error}"),
            Error::ImageDataLength => f.write_str("Bilddaten passen nicht zur Bildgröße"),
            Error::Filter => f.write_str("unbekannter Filtertyp"),
            Error::TooLarge => f.write_str("Bild zu groß"),
        }
    }
}

impl std::error::Error for Error {}

/// Ein dekodiertes Bild.
#[derive(Clone, Debug)]
pub struct Png {
    width: u32,
    height: u32,
    color_type: ColorType,
    bit_depth: u8,
    palette: Vec<[u8; 3]>,
    transparency: Transparency,
    row_bytes: usize,
    data: Vec<u8>,
}

impl Png {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn color_type(&self) -> ColorType {
        self.color_type
    }

    /// Die Bits je Abtastwert, bei Paletten je Index.
    pub fn bit_depth(&self) -> u8 {
        self.bit_depth
    }

    /// Die Palette; leer, wenn der Farbtyp keine hat.
    pub fn palette(&self) -> &[[u8; 3]] {
        &self.palette
    }

    pub fn transparency(&self) -> &Transparency {
        &self.transparency
    }

    /// Die Länge einer Zeile in [`Png::data`].
    pub fn row_bytes(&self) -> usize {
        self.row_bytes
    }

    /// Die Zeilen von oben nach unten im Format der Datei, aber ohne Filter
    /// und bei Adam7 schon zusammengesetzt: Abtastwerte mit 16 Bit als
    /// Big-Endian, Pixel mit weniger als 8 Bit dicht gepackt, das höchste
    /// Bit zuerst. Jede Zeile beginnt an einer Bytegrenze, ungenutzte Bits
    /// am Ende sind 0.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Die Pixel als RGBA mit 8 Bit je Kanal, Alpha nicht vormultipliziert,
    /// Zeile für Zeile ohne Lücken.
    pub fn to_rgba8(&self) -> Vec<u8> {
        pixels::to_rgba8(self)
    }
}

/// Die Werte aus IHDR.
struct Header {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: ColorType,
    interlaced: bool,
}

/// Liest IHDR („IHDR Image header“).
fn header(data: &[u8]) -> Result<Header, Error> {
    let Ok(
        &[
            w0,
            w1,
            w2,
            w3,
            h0,
            h1,
            h2,
            h3,
            bit_depth,
            color_type,
            compression,
            filter,
            interlace,
        ],
    ) = <&[u8; 13]>::try_from(data)
    else {
        return Err(Error::Header);
    };
    let width = u32::from_be_bytes([w0, w1, w2, w3]);
    let height = u32::from_be_bytes([h0, h1, h2, h3]);
    let size = 1..=MAX_NUMBER;
    if !size.contains(&width)
        || !size.contains(&height)
        || compression != 0
        || filter != 0
        || interlace > 1
    {
        return Err(Error::Header);
    }
    Ok(Header {
        width,
        height,
        bit_depth,
        color_type: ColorType::from_header(color_type, bit_depth).ok_or(Error::Header)?,
        interlaced: interlace == 1,
    })
}

/// Liest die Chunks nacheinander („Chunk layout“).
struct Chunks<'a> {
    rest: &'a [u8],
}

impl<'a> Chunks<'a> {
    /// Länge und Typ des nächsten Chunks.
    fn header(&mut self) -> Result<(usize, [u8; 4]), Error> {
        let Some((&[l0, l1, l2, l3, t0, t1, t2, t3], rest)) = self.rest.split_first_chunk::<8>()
        else {
            return Err(Error::UnexpectedEnd);
        };
        let length = u32::from_be_bytes([l0, l1, l2, l3]);
        if length > MAX_NUMBER {
            return Err(Error::ChunkLength);
        }
        self.rest = rest;
        let length = usize::try_from(length).map_err(|_| Error::ChunkLength)?;
        Ok((length, [t0, t1, t2, t3]))
    }

    /// Die Daten des Chunks, dazu, ob seine CRC stimmt.
    fn body(&mut self, kind: [u8; 4], length: usize) -> Result<(&'a [u8], bool), Error> {
        let data = self.rest.get(..length).ok_or(Error::UnexpectedEnd)?;
        let end = length.checked_add(4).ok_or(Error::UnexpectedEnd)?;
        let Some(&[c0, c1, c2, c3]) = self.rest.get(length..end) else {
            return Err(Error::UnexpectedEnd);
        };
        self.rest = self.rest.get(end..).unwrap_or_default();
        let mut crc = Crc32::new();
        crc.update(&kind);
        crc.update(data);
        Ok((data, crc.value() == u32::from_be_bytes([c0, c1, c2, c3])))
    }
}

/// Wo der Dekoder steht: vor, in oder nach den IDAT-Chunks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Before,
    Within,
    After,
}

/// Dekodiert ein Bild mit höchstens `max_pixels` Pixeln.
pub fn decode(input: &[u8], max_pixels: u64) -> Result<Png, Error> {
    let rest = input.strip_prefix(&SIGNATURE).ok_or(Error::Signature)?;
    let mut chunks = Chunks { rest };
    let (length, kind) = chunks.header()?;
    if kind != *b"IHDR" {
        return Err(Error::MissingHeader);
    }
    let (data, intact) = chunks.body(kind, length)?;
    if !intact {
        return Err(Error::Checksum);
    }
    let header = header(data)?;
    let count = u64::from(header.width)
        .checked_mul(u64::from(header.height))
        .ok_or(Error::TooLarge)?;
    if count > max_pixels {
        return Err(Error::TooLarge);
    }
    let layout = pixels::Layout::new(&header)?;

    let mut palette: Option<Vec<[u8; 3]>> = None;
    let mut transparency: Option<Transparency> = None;
    // Die Bilddaten sind ein Teil der Eingabe; so wächst der Puffer nicht nach.
    let mut image_data = Vec::with_capacity(input.len());
    let mut position = Position::Before;
    loop {
        let (length, kind) = chunks.header()?;
        if kind == *b"IEND" {
            break;
        }
        if !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err(Error::ChunkType);
        }
        let (data, intact) = chunks.body(kind, length)?;
        // Bit 5 des ersten Bytes: 0 heißt kritisch.
        let [first, ..] = kind;
        let critical = first & 0x20 == 0;
        if kind != *b"IDAT" && position == Position::Within {
            position = Position::After;
        }
        if !intact {
            if critical {
                return Err(Error::Checksum);
            }
            continue;
        }
        match &kind {
            b"IHDR" => return Err(Error::Order),
            b"PLTE" if header.color_type == ColorType::Indexed => {
                if palette.is_some() || position != Position::Before {
                    return Err(Error::Order);
                }
                palette = Some(read_palette(data, header.bit_depth)?);
            }
            b"PLTE" => {}
            b"IDAT" => {
                if position == Position::After {
                    return Err(Error::Order);
                }
                if header.color_type == ColorType::Indexed && palette.is_none() {
                    return Err(Error::MissingPalette);
                }
                image_data.extend_from_slice(data);
                position = Position::Within;
            }
            b"tRNS" => {
                if transparency.is_none() {
                    // tRNS steht nach PLTE und vor IDAT.
                    let valid = (position == Position::Before)
                        .then(|| read_transparency(data, &header, palette.as_deref()))
                        .flatten();
                    transparency = Some(valid.unwrap_or(Transparency::None));
                }
            }
            _ if critical => return Err(Error::UnknownCritical),
            _ => {}
        }
    }
    if position == Position::Before {
        return Err(Error::MissingImageData);
    }
    let raw = zlib_decompress_sized(&image_data, layout.raw_size).map_err(|error| match error {
        mb_compress::Error::TooLarge => Error::ImageDataLength,
        error => Error::Compression(error),
    })?;
    if raw.len() != layout.raw_size {
        return Err(Error::ImageDataLength);
    }
    let data = layout.reconstruct(&raw)?;
    Ok(Png {
        width: header.width,
        height: header.height,
        color_type: header.color_type,
        bit_depth: header.bit_depth,
        palette: palette.unwrap_or_default(),
        transparency: transparency.unwrap_or(Transparency::None),
        row_bytes: layout.row_bytes,
        data,
    })
}

/// Liest PLTE („PLTE Palette“).
fn read_palette(data: &[u8], bit_depth: u8) -> Result<Vec<[u8; 3]>, Error> {
    let (entries, rest) = data.as_chunks::<3>();
    if entries.is_empty() || !rest.is_empty() || entries.len() > 1_usize << bit_depth {
        return Err(Error::Palette);
    }
    Ok(entries.to_vec())
}

/// Liest tRNS („tRNS Transparency“); keine Transparenz, wenn der Chunk
/// nicht zum Bild passt.
fn read_transparency(
    data: &[u8],
    header: &Header,
    palette: Option<&[[u8; 3]]>,
) -> Option<Transparency> {
    // Bei weniger als 16 Bit zählen nur die unteren Bits.
    let mask = u16::MAX >> 16_u8.saturating_sub(header.bit_depth);
    let sample = |high: u8, low: u8| u16::from_be_bytes([high, low]) & mask;
    match header.color_type {
        ColorType::Grey => {
            let &[high, low] = <&[u8; 2]>::try_from(data).ok()?;
            Some(Transparency::Grey(sample(high, low)))
        }
        ColorType::Truecolor => {
            let &[r0, r1, g0, g1, b0, b1] = <&[u8; 6]>::try_from(data).ok()?;
            Some(Transparency::Rgb([
                sample(r0, r1),
                sample(g0, g1),
                sample(b0, b1),
            ]))
        }
        ColorType::Indexed => {
            let palette = palette?;
            (data.len() <= palette.len()).then(|| Transparency::Palette(data.to_vec()))
        }
        ColorType::GreyAlpha | ColorType::TruecolorAlpha => None,
    }
}
