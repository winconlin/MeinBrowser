//! Das erste Einzelbild von GIF87a und GIF89a.
//!
//! Grundlage: GIF89a Specification, insbesondere Image Descriptor, Graphic
//! Control Extension, Data Sub-blocks und Appendix F (LZW Compression).
//! Globale und lokale Farbtabellen, Transparenz und Interlacing werden
//! unterstützt. Animationsablauf, Verzögerungen, Disposal und Pixel-Seiten-
//! verhältnis werden nicht ausgewertet; Plain-Text-Anzeigen werden übersprungen.
//! Nach dem ersten Bild endet das Lesen;
//! spätere Einzelbilder und der Trailer werden nicht geprüft.
//!
//! Die logische Leinwand muss in das Pixelbudget des Aufrufers passen. LZW
//! schreibt unmittelbar in die RGBA-Ausgabe, ohne entpackten Zwischenpuffer.
//! Das Wörterbuch und der Stapel sind auf je 4096 Einträge begrenzt. Ein
//! Bild liefert genau so viele Indizes, wie seine Maße verlangen.

#[cfg(test)]
mod tests;

/// Fehler beim Lesen des ersten Einzelbilds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Signature,
    UnexpectedEnd,
    Dimensions,
    TooLarge,
    MissingPalette,
    PaletteIndex,
    Block,
    LzwCodeSize,
    Lzw,
    PixelCount,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Error::Signature => "keine GIF87a- oder GIF89a-Signatur",
            Error::UnexpectedEnd => "GIF-Daten enden zu früh",
            Error::Dimensions => "ungültige GIF-Maße oder Bild außerhalb der Leinwand",
            Error::TooLarge => "GIF überschreitet das Pixel- oder Speicherbudget",
            Error::MissingPalette => "GIF-Farbtabelle fehlt",
            Error::PaletteIndex => "GIF-Farbindex außerhalb der Tabelle",
            Error::Block => "ungültiger GIF-Block oder kein Einzelbild",
            Error::LzwCodeSize => "ungültige LZW-Codegröße",
            Error::Lzw => "ungültiger GIF-LZW-Strom",
            Error::PixelCount => "GIF-Bilddaten passen nicht zur Bildgröße",
        })
    }
}

impl std::error::Error for Error {}

/// Das erste Einzelbild auf der logischen Leinwand, als RGBA8 ohne
/// Vormultiplikation von Alpha. Transparente Pixel sind `[0, 0, 0, 0]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gif {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Gif {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Übernimmt die Ausgabe ohne zusätzliche Kopie.
    pub fn into_rgba8(self) -> Vec<u8> {
        self.pixels
    }
}

struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let (bytes, rest) = self
            .rest
            .split_at_checked(length)
            .ok_or(Error::UnexpectedEnd)?;
        self.rest = rest;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.bytes(1)?[0])
    }

    fn word(&mut self) -> Result<u32, Error> {
        let bytes = self.bytes(2)?;
        Ok(u32::from(u16::from_le_bytes([bytes[0], bytes[1]])))
    }

    fn blocks(&mut self) -> Result<(), Error> {
        loop {
            let size = usize::from(self.byte()?);
            if size == 0 {
                return Ok(());
            }
            self.bytes(size)?;
        }
    }

    fn palette(&mut self, packed: u8) -> Result<Option<&'a [u8]>, Error> {
        if packed & 0x80 == 0 {
            return Ok(None);
        }
        self.bytes(3 * (2_usize << (packed & 7))).map(Some)
    }
}

fn color(palette: &[u8], index: u8) -> Result<[u8; 4], Error> {
    let start = usize::from(index) * 3;
    let rgb = palette.get(start..start + 3).ok_or(Error::PaletteIndex)?;
    Ok([rgb[0], rgb[1], rgb[2], 255])
}

/// Dekodiert das erste Einzelbild. `max_pixels` begrenzt die ganze logische
/// Leinwand einschließlich unbedeckter Flächen, nicht nur den Bildausschnitt.
/// Ohne Transparenz gilt die globale Hintergrundfarbe; ohne globale Tabelle
/// oder bei aktiviertem transparentem Index bleibt die übrige Leinwand leer.
pub fn decode(input: &[u8], max_pixels: u64) -> Result<Gif, Error> {
    let rest = input
        .strip_prefix(b"GIF87a")
        .or_else(|| input.strip_prefix(b"GIF89a"))
        .ok_or(Error::Signature)?;
    let mut reader = Reader { rest };
    let width = reader.word()?;
    let height = reader.word()?;
    if width == 0 || height == 0 {
        return Err(Error::Dimensions);
    }
    let count = u64::from(width) * u64::from(height);
    if count > max_pixels {
        return Err(Error::TooLarge);
    }
    let bytes = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(4))
        .ok_or(Error::TooLarge)?;
    let packed = reader.byte()?;
    let background = reader.byte()?;
    let _aspect_ratio = reader.byte()?;
    let global = reader.palette(packed)?;
    let screen_background = global
        .map(|table| color(table, background))
        .transpose()?
        .unwrap_or([0; 4]);
    let mut transparent = None;
    loop {
        match reader.byte()? {
            0x21 => match reader.byte()? {
                0xF9 => {
                    if reader.byte()? != 4 {
                        return Err(Error::Block);
                    }
                    let control = reader.bytes(4)?;
                    transparent = (control[0] & 1 != 0).then_some(control[3]);
                    if reader.byte()? != 0 {
                        return Err(Error::Block);
                    }
                }
                0x01 => {
                    if reader.byte()? != 12 {
                        return Err(Error::Block);
                    }
                    reader.bytes(12)?;
                    reader.blocks()?;
                    // Plain Text ist ein Graphic Rendering Block und
                    // verbraucht die vorangegangene Graphic Control Extension.
                    transparent = None;
                }
                _ => reader.blocks()?,
            },
            0x2C => {
                let left = reader.word()?;
                let top = reader.word()?;
                let image_width = reader.word()?;
                let image_height = reader.word()?;
                if image_width == 0
                    || image_height == 0
                    || left + image_width > width
                    || top + image_height > height
                {
                    return Err(Error::Dimensions);
                }
                let packed = reader.byte()?;
                let palette = reader
                    .palette(packed)?
                    .or(global)
                    .ok_or(Error::MissingPalette)?;
                let background = if transparent.is_some() {
                    [0; 4]
                } else {
                    screen_background
                };
                let mut pixels = Vec::new();
                pixels
                    .try_reserve_exact(bytes)
                    .map_err(|_| Error::TooLarge)?;
                pixels.resize(bytes, 0);
                if background != [0; 4] {
                    for pixel in pixels.as_chunks_mut::<4>().0 {
                        pixel.copy_from_slice(&background);
                    }
                }
                let mut x = 0_u32;
                let mut y = 0_u32;
                let mut pass = 0_usize;
                let interlaced = packed & 0x40 != 0;
                let size = reader.byte()?;
                lzw(
                    &mut reader,
                    size,
                    u64::from(image_width) * u64::from(image_height),
                    |index| {
                        let rgba = if transparent == Some(index) {
                            [0; 4]
                        } else {
                            color(palette, index)?
                        };
                        let at =
                            ((u64::from(top + y) * u64::from(width)) + u64::from(left + x)) * 4;
                        let at = usize::try_from(at).map_err(|_| Error::TooLarge)?;
                        let pixel = pixels.get_mut(at..at + 4).ok_or(Error::Dimensions)?;
                        pixel.copy_from_slice(&rgba);
                        x += 1;
                        if x == image_width {
                            x = 0;
                            y += if interlaced { [8, 8, 4, 2][pass] } else { 1 };
                            if interlaced {
                                while y >= image_height && pass < 3 {
                                    pass += 1;
                                    y = [0, 4, 2, 1][pass];
                                }
                            }
                        }
                        Ok(())
                    },
                )?;
                return Ok(Gif {
                    width,
                    height,
                    pixels,
                });
            }
            _ => return Err(Error::Block),
        }
    }
}

/// Codes über beliebige Subblock-Grenzen hinweg, niederwertigste Bits zuerst.
struct Codes<'a, 'b> {
    reader: &'b mut Reader<'a>,
    remaining: usize,
    buffer: u32,
    bits: u8,
}

impl Codes<'_, '_> {
    fn code(&mut self, width: u8) -> Result<u16, Error> {
        while self.bits < width {
            if self.remaining == 0 {
                self.remaining = usize::from(self.reader.byte()?);
                if self.remaining == 0 {
                    return Err(Error::Lzw);
                }
            }
            self.buffer |= u32::from(self.reader.byte()?) << self.bits;
            self.bits += 8;
            self.remaining -= 1;
        }
        let code = (self.buffer & ((1_u32 << width) - 1)) as u16;
        self.buffer >>= width;
        self.bits -= width;
        Ok(code)
    }

    fn finish(self) -> Result<(), Error> {
        self.reader.bytes(self.remaining)?;
        self.reader.blocks()
    }
}

/// Appendix F: Clear/End-Codes, wachsende Codebreite und ein Wörterbuch mit
/// höchstens 4096 Einträgen. Jeder neue Präfix verweist auf einen älteren.
fn lzw(
    reader: &mut Reader<'_>,
    minimum: u8,
    expected: u64,
    mut emit: impl FnMut(u8) -> Result<(), Error>,
) -> Result<(), Error> {
    if !(2..=8).contains(&minimum) {
        return Err(Error::LzwCodeSize);
    }
    let clear = 1_u16 << minimum;
    let end = clear + 1;
    let mut width = minimum + 1;
    let mut next = end + 1;
    let mut previous = None;
    let mut prefix = [0_u16; 4096];
    let mut suffix = [0_u8; 4096];
    let mut stack = [0_u8; 4096];
    let mut codes = Codes {
        reader,
        remaining: 0,
        buffer: 0,
        bits: 0,
    };
    // Ein anfänglicher Clear-Code wird Encodern empfohlen, ist aber nicht
    // erforderlich: Der Anfangszustand entspricht bereits einem Clear.
    let mut produced = 0_u64;
    let mut write = |byte| {
        if produced >= expected {
            return Err(Error::PixelCount);
        }
        emit(byte)?;
        produced += 1;
        Ok(())
    };
    loop {
        let code = codes.code(width)?;
        if code == clear {
            width = minimum + 1;
            next = end + 1;
            previous = None;
            continue;
        }
        if code == end {
            // Die Closure wird danach nicht mehr gebraucht.
            if produced != expected {
                return Err(Error::PixelCount);
            }
            return codes.finish();
        }
        let Some(old) = previous else {
            if code >= clear {
                return Err(Error::Lzw);
            }
            write(code as u8)?;
            previous = Some(code);
            continue;
        };
        if code > next {
            return Err(Error::Lzw);
        }
        // Sonderfall KwKwK: Der nächste Eintrag ist noch nicht angelegt.
        let special = code == next;
        let mut current = if special { old } else { code };
        let mut length = 0;
        while current >= clear {
            if current <= end || current >= next || length >= stack.len() {
                return Err(Error::Lzw);
            }
            stack[length] = suffix[usize::from(current)];
            length += 1;
            current = prefix[usize::from(current)];
        }
        let first = current as u8;
        for &byte in std::iter::once(&first).chain(stack[..length].iter().rev()) {
            write(byte)?;
        }
        if special {
            write(first)?;
        }
        if next < 4096 {
            prefix[usize::from(next)] = old;
            suffix[usize::from(next)] = first;
            next += 1;
            if next == 1_u16 << width && width < 12 {
                width += 1;
            }
        }
        previous = Some(code);
    }
}
