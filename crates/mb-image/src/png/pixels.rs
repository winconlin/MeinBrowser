//! Die Bilddaten: Filter umkehren, Adam7 zusammensetzen und nach RGBA
//! wandeln.

use super::{ColorType, Error, Header, Png, Transparency};

/// Ein Durchgang: erstes Pixel (x, y) und Abstand (dx, dy).
type Pass = (usize, usize, usize, usize);

/// Die sieben Durchgänge von Adam7 („Interlacing and pass extraction“).
const ADAM7: [Pass; 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

/// Ohne Zeilensprung ist das Bild ein einziger Durchgang.
const WHOLE: [Pass; 1] = [(0, 0, 1, 1)];

/// Die Maße der Bilddaten.
pub(super) struct Layout {
    width: usize,
    height: usize,
    /// Bits je Pixel.
    bits: usize,
    interlaced: bool,
    /// Bytes je Zeile des fertigen Bildes.
    pub(super) row_bytes: usize,
    /// Bytes der Bilddaten nach dem Entpacken, mit den Filterbytes.
    pub(super) raw_size: usize,
}

/// Bytes einer Zeile mit `width` Pixeln zu je `bits` Bit.
fn line_bytes(width: usize, bits: usize) -> Option<usize> {
    Some(width.checked_mul(bits)?.div_ceil(8))
}

/// Breite und Höhe eines Durchgangs; bei kleinen Bildern sind manche leer.
fn pass_size(width: usize, height: usize, (x, y, dx, dy): Pass) -> (usize, usize) {
    (
        width.saturating_sub(x).div_ceil(dx),
        height.saturating_sub(y).div_ceil(dy),
    )
}

impl Layout {
    pub(super) fn new(header: &Header) -> Result<Layout, Error> {
        let width = usize::try_from(header.width).map_err(|_| Error::TooLarge)?;
        let height = usize::try_from(header.height).map_err(|_| Error::TooLarge)?;
        let bits = usize::from(header.color_type.channels()) * usize::from(header.bit_depth);
        let row_bytes = line_bytes(width, bits).ok_or(Error::TooLarge)?;
        row_bytes.checked_mul(height).ok_or(Error::TooLarge)?;
        let mut layout = Layout {
            width,
            height,
            bits,
            interlaced: header.interlaced,
            row_bytes,
            raw_size: 0,
        };
        for &pass in layout.passes() {
            let (pass_width, pass_height) = pass_size(width, height, pass);
            if pass_width == 0 || pass_height == 0 {
                continue;
            }
            // Jede Zeile beginnt mit ihrem Filtertyp.
            let size = line_bytes(pass_width, bits)
                .and_then(|bytes| bytes.checked_add(1))
                .and_then(|bytes| bytes.checked_mul(pass_height))
                .ok_or(Error::TooLarge)?;
            layout.raw_size = layout.raw_size.checked_add(size).ok_or(Error::TooLarge)?;
        }
        Ok(layout)
    }

    fn passes(&self) -> &'static [Pass] {
        if self.interlaced { &ADAM7 } else { &WHOLE }
    }

    /// Kehrt die Filter um und setzt die Durchgänge zum Bild zusammen.
    /// `raw` hat genau [`Layout::raw_size`] Bytes.
    pub(super) fn reconstruct(&self, raw: &[u8]) -> Result<Vec<u8>, Error> {
        let mut image = vec![0_u8; self.row_bytes.saturating_mul(self.height)];
        // Der Abstand zum Byte links daneben („Filter types for filter
        // method 0“): ein Pixel, unter 8 Bit ein Byte.
        let left = (self.bits / 8).max(1);
        let mut rest = raw;
        for &pass in self.passes() {
            let (pass_width, pass_height) = pass_size(self.width, self.height, pass);
            if pass_width == 0 || pass_height == 0 {
                continue;
            }
            let bytes = line_bytes(pass_width, self.bits).ok_or(Error::TooLarge)?;
            // Über der ersten Zeile eines Durchgangs liegt eine aus Nullen.
            let mut previous = vec![0_u8; bytes];
            let mut current = vec![0_u8; bytes];
            let (x, y, dx, dy) = pass;
            for row in 0..pass_height {
                let Some((&filter, after)) = rest.split_first() else {
                    return Err(Error::ImageDataLength);
                };
                current.copy_from_slice(after.get(..bytes).ok_or(Error::ImageDataLength)?);
                rest = after.get(bytes..).unwrap_or_default();
                unfilter(filter, &mut current, &previous, left)?;
                let start = row
                    .saturating_mul(dy)
                    .saturating_add(y)
                    .saturating_mul(self.row_bytes);
                let end = start.saturating_add(self.row_bytes);
                if let Some(target) = image.get_mut(start..end) {
                    self.place(&current, target, x, dx, pass_width);
                }
                std::mem::swap(&mut previous, &mut current);
            }
        }
        Ok(image)
    }

    /// Schreibt eine Zeile eines Durchgangs in die Zeile des Bildes.
    fn place(&self, line: &[u8], target: &mut [u8], x: usize, dx: usize, pass_width: usize) {
        if dx == 1 {
            // Die ganze Zeile: ohne Zeilensprung und im letzten Durchgang.
            if let Some(part) = target.get_mut(..line.len()) {
                part.copy_from_slice(line);
            }
            let used = self.width.saturating_mul(self.bits) % 8;
            if used != 0
                && let Some(last) = target.last_mut()
            {
                *last &= 0xFF << (8 - used);
            }
        } else if self.bits >= 8 {
            let size = self.bits / 8;
            for (index, pixel) in line.chunks_exact(size).enumerate() {
                let at = index
                    .saturating_mul(dx)
                    .saturating_add(x)
                    .saturating_mul(size);
                if let Some(part) = target.get_mut(at..at.saturating_add(size)) {
                    part.copy_from_slice(pixel);
                }
            }
        } else {
            for index in 0..pass_width {
                let value = packed(line, index, self.bits);
                set_packed(
                    target,
                    index.saturating_mul(dx).saturating_add(x),
                    self.bits,
                    value,
                );
            }
        }
    }
}

/// Kehrt den Filter einer Zeile um („Filter types for filter method 0“).
/// `left` ist der Abstand zum Byte `a`.
fn unfilter(filter: u8, row: &mut [u8], previous: &[u8], left: usize) -> Result<(), Error> {
    if filter > 4 {
        return Err(Error::Filter);
    }
    if filter == 0 {
        return Ok(());
    }
    let before = |bytes: &[u8], index: usize| {
        index
            .checked_sub(left)
            .and_then(|index| bytes.get(index))
            .copied()
            .unwrap_or(0)
    };
    for index in 0..row.len() {
        let a = before(row, index);
        let b = previous.get(index).copied().unwrap_or(0);
        let predictor = match filter {
            1 => a,
            2 => b,
            // floor((a + b) / 2) ohne Überlauf.
            3 => (a >> 1) + (b >> 1) + (a & b & 1),
            _ => paeth(a, b, before(previous, index)),
        };
        if let Some(byte) = row.get_mut(index) {
            *byte = byte.wrapping_add(predictor);
        }
    }
    Ok(())
}

/// „The PaethPredictor function“, in genau dieser Reihenfolge der
/// Vergleiche.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (wide_a, wide_b, wide_c) = (i16::from(a), i16::from(b), i16::from(c));
    let p = wide_a + wide_b - wide_c;
    let pa = (p - wide_a).abs();
    let pb = (p - wide_b).abs();
    let pc = (p - wide_c).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Der `index`-te Wert mit `bits` Bit (1, 2 oder 4) einer Zeile, das
/// höchste Bit zuerst.
fn packed(row: &[u8], index: usize, bits: usize) -> u8 {
    let per_byte = 8 / bits;
    let shift = 8 - bits * (index % per_byte + 1);
    let mask = (1_u8 << bits) - 1;
    row.get(index / per_byte)
        .map_or(0, |byte| (byte >> shift) & mask)
}

fn set_packed(row: &mut [u8], index: usize, bits: usize, value: u8) {
    let per_byte = 8 / bits;
    let shift = 8 - bits * (index % per_byte + 1);
    if let Some(byte) = row.get_mut(index / per_byte) {
        *byte |= value << shift;
    }
}

/// Der `index`-te Abtastwert einer Zeile.
fn sample(row: &[u8], index: usize, bit_depth: u8) -> u16 {
    match bit_depth {
        16 => {
            let at = index.saturating_mul(2);
            match row.get(at..at.saturating_add(2)) {
                Some(&[high, low]) => u16::from_be_bytes([high, low]),
                _ => 0,
            }
        }
        8 => row.get(index).map_or(0, |&byte| u16::from(byte)),
        bits => u16::from(packed(row, index, usize::from(bits))),
    }
}

/// Ein Abtastwert in 8 Bit („Sample depth rescaling“). Kleinere Tiefen
/// werden linear skaliert, das ist bei 1, 2 und 4 Bit exakt. Von 16 Bit
/// bleibt das obere Byte (die Schiebemethode), wie bei image-png mit
/// `STRIP_16`; `png/tests.rs` prüft das an dessen `results.txt`.
fn to_8_bits(value: u16, bit_depth: u8) -> u8 {
    match bit_depth {
        16 => {
            let [high, _] = value.to_be_bytes();
            high
        }
        8 => u8::try_from(value).unwrap_or(u8::MAX),
        bits => {
            let max = (1_u16 << bits) - 1;
            let factor = 255_u16.checked_div(max).unwrap_or(0);
            u8::try_from(value.saturating_mul(factor)).unwrap_or(u8::MAX)
        }
    }
}

pub(super) fn to_rgba8(png: &Png) -> Vec<u8> {
    let width = usize::try_from(png.width).unwrap_or(0);
    let height = usize::try_from(png.height).unwrap_or(0);
    let channels = usize::from(png.color_type.channels());
    let depth = png.bit_depth;
    let mut out = Vec::with_capacity(width.saturating_mul(height).saturating_mul(4));
    if png.row_bytes == 0 {
        return out;
    }
    for row in png.data.chunks_exact(png.row_bytes) {
        for x in 0..width {
            let first = x.saturating_mul(channels);
            let value = |channel: usize| sample(row, first.saturating_add(channel), depth);
            let scaled = |channel: usize| to_8_bits(value(channel), depth);
            let pixel = match png.color_type {
                ColorType::Grey => {
                    // Der Vergleich mit tRNS geschieht vor dem Skalieren.
                    let transparent = png.transparency == Transparency::Grey(value(0));
                    let grey = scaled(0);
                    [grey, grey, grey, if transparent { 0 } else { 255 }]
                }
                ColorType::GreyAlpha => {
                    let grey = scaled(0);
                    [grey, grey, grey, scaled(1)]
                }
                ColorType::Truecolor => {
                    let transparent =
                        png.transparency == Transparency::Rgb([value(0), value(1), value(2)]);
                    [
                        scaled(0),
                        scaled(1),
                        scaled(2),
                        if transparent { 0 } else { 255 },
                    ]
                }
                ColorType::TruecolorAlpha => [scaled(0), scaled(1), scaled(2), scaled(3)],
                ColorType::Indexed => {
                    let index = usize::from(value(0));
                    match png.palette.get(index) {
                        Some(&[red, green, blue]) => {
                            let alpha = match &png.transparency {
                                Transparency::Palette(alphas) => {
                                    alphas.get(index).copied().unwrap_or(255)
                                }
                                _ => 255,
                            };
                            [red, green, blue, alpha]
                        }
                        // Ein Index außerhalb der Palette ist undurchsichtiges
                        // Schwarz („Error handling“).
                        None => [0, 0, 0, 255],
                    }
                }
            };
            out.extend_from_slice(&pixel);
        }
    }
    out
}
