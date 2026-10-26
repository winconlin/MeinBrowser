//! Zeichenfläche im Arbeitsspeicher.

use std::fmt;

use crate::path::{self, Path};

/// Größte erlaubte Kantenlänge einer Zeichenfläche in Pixeln.
pub const MAX_DIMENSION: u32 = 32_767;

/// Größte erlaubte Anzahl Pixel einer Zeichenfläche (2²⁸, also 1 GiB).
pub const MAX_PIXELS: u64 = 1 << 28;

/// Eine Farbe mit 8 Bit je Kanal, nicht vormultipliziert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    /// Eine deckende Farbe.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    /// Die Farbe als vormultipliziertes Pixel `0xAARRGGBB`.
    pub fn to_pixel(self) -> u32 {
        let a = u32::from(self.a);
        let r = mul_div_255(u32::from(self.r), a);
        let g = mul_div_255(u32::from(self.g), a);
        let b = mul_div_255(u32::from(self.b), a);
        (a << 24) | (r << 16) | (g << 8) | b
    }
}

/// Die Zeichenfläche ist größer als [`MAX_DIMENSION`] oder [`MAX_PIXELS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLarge {
    pub width: u32,
    pub height: u32,
}

impl fmt::Display for TooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Zeichenfläche {}×{} ist zu groß (höchstens {MAX_DIMENSION} Pixel je Kante und {MAX_PIXELS} Pixel insgesamt)",
            self.width, self.height
        )
    }
}

impl std::error::Error for TooLarge {}

/// Eine Zeichenfläche mit Pixeln im Format `0xAARRGGBB` (vormultipliziert),
/// zeilenweise von oben nach unten.
#[derive(Clone, PartialEq, Eq)]
pub struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl fmt::Debug for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Canvas")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Canvas {
    /// Eine Zeichenfläche, ganz mit `background` gefüllt.
    pub fn new(width: u32, height: u32, background: Color) -> Result<Canvas, TooLarge> {
        let count = u64::from(width) * u64::from(height);
        if width > MAX_DIMENSION || height > MAX_DIMENSION || count > MAX_PIXELS {
            return Err(TooLarge { width, height });
        }
        let count = usize::try_from(count).map_err(|_| TooLarge { width, height })?;
        Ok(Canvas {
            width,
            height,
            pixels: vec![background.to_pixel(); count],
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Alle Pixel, zeilenweise von oben nach unten.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Das Pixel an `(x, y)`, oder `None` außerhalb der Fläche.
    pub fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels.get(self.index(x, y)).copied()
    }

    /// Füllt ein Rechteck. Teile außerhalb der Fläche werden abgeschnitten.
    pub fn fill_rect(&mut self, x: i32, y: i32, width: u32, height: u32, color: Color) {
        let (x0, x1) = clip_span(i64::from(x), i64::from(width), self.width);
        let (y0, y1) = clip_span(i64::from(y), i64::from(height), self.height);
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, color, 255);
            }
        }
    }

    /// Füllt einen Kreis mit geglättetem Rand.
    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        self.fill_ring(cx, cy, 0.0, radius, color);
    }

    /// Füllt einen Ring zwischen `inner` und `outer` (Radien) mit geglätteten
    /// Rändern. Mit `inner <= 0` entsteht ein voller Kreis.
    pub fn fill_ring(&mut self, cx: f32, cy: f32, inner: f32, outer: f32, color: Color) {
        if !(cx.is_finite() && cy.is_finite() && inner.is_finite() && outer.is_finite()) {
            return;
        }
        if outer <= 0.0 || inner >= outer {
            return;
        }
        let (x0, x1) = clip_float_span(cx - outer - 1.0, cx + outer + 1.0, self.width);
        let (y0, y1) = clip_float_span(cy - outer - 1.0, cy + outer + 1.0, self.height);
        for py in y0..y1 {
            for px in x0..x1 {
                let dx = px as f32 + 0.5 - cx;
                let dy = py as f32 + 0.5 - cy;
                let distance = (dx * dx + dy * dy).sqrt();
                let mut coverage = (outer - distance + 0.5).clamp(0.0, 1.0);
                if inner > 0.0 {
                    coverage -= (inner - distance + 0.5).clamp(0.0, 1.0);
                }
                // Abdeckung 0.0 bis 1.0 als Alpha 0 bis 255, gerundet.
                let alpha = (coverage.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                if alpha > 0 {
                    self.blend(px, py, color, alpha);
                }
            }
        }
    }

    /// Füllt einen Pfad mit geglätteten Kanten (Regel *nonzero*). Teile
    /// außerhalb der Fläche werden abgeschnitten, Pfade mit nicht endlichen
    /// Koordinaten zeichnen nichts.
    pub fn fill_path(&mut self, path: &Path, color: Color) {
        let Some(coverage) = path::coverage(path, self.width, self.height) else {
            return;
        };
        let Ok(width) = usize::try_from(coverage.width) else {
            return;
        };
        if width == 0 {
            return;
        }
        for (y, row) in (coverage.y..).zip(coverage.values.chunks_exact(width)) {
            for (x, &value) in (coverage.x..).zip(row) {
                // Abdeckung 0.0 bis 1.0 als Alpha 0 bis 255, gerundet.
                let alpha = (value * 255.0 + 0.5) as u8;
                if alpha > 0 {
                    self.blend(x, y, color, alpha);
                }
            }
        }
    }

    fn index(&self, x: u32, y: u32) -> usize {
        // Passt immer: Breite mal Höhe ist beim Anlegen geprüft worden.
        y as usize * self.width as usize + x as usize
    }

    /// Zeichnet `color` mit zusätzlicher Deckkraft `alpha` über das Pixel
    /// (Porter-Duff „source over“, vormultipliziert).
    fn blend(&mut self, x: u32, y: u32, color: Color, alpha: u8) {
        let index = self.index(x, y);
        let Some(dst) = self.pixels.get_mut(index) else {
            return;
        };
        let a = mul_div_255(u32::from(color.a), u32::from(alpha));
        if a == 0 {
            return;
        }
        let inverse = 255 - a;
        let channel = |shift: u32, source: u8| {
            let src = mul_div_255(u32::from(source), a);
            let old = (*dst >> shift) & 0xff;
            (src + mul_div_255(old, inverse)).min(255) << shift
        };
        let out_a = (a + mul_div_255(*dst >> 24, inverse)).min(255) << 24;
        *dst = out_a | channel(16, color.r) | channel(8, color.g) | channel(0, color.b);
    }
}

/// `x * y / 255`, korrekt gerundet, für `x, y <= 255`.
fn mul_div_255(x: u32, y: u32) -> u32 {
    let t = x * y + 128;
    (t + (t >> 8)) >> 8
}

/// Schneidet die Spanne `[start, start + length)` auf `[0, limit)` zu.
fn clip_span(start: i64, length: i64, limit: u32) -> (u32, u32) {
    let limit = i64::from(limit);
    let begin = start.clamp(0, limit);
    let end = start.saturating_add(length).clamp(0, limit);
    // Beide Werte liegen in [0, limit] und passen damit in u32.
    (begin as u32, end.max(begin) as u32)
}

/// Wie [`clip_span`], aber für Gleitkommagrenzen `[start, end)`.
fn clip_float_span(start: f32, end: f32, limit: u32) -> (u32, u32) {
    let limit_f = limit as f32;
    let begin = start.floor().clamp(0.0, limit_f) as u32;
    let end = end.ceil().clamp(0.0, limit_f) as u32;
    (begin.min(limit), end.clamp(begin.min(limit), limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplies_colors() {
        assert_eq!(Color::rgb(0x12, 0x34, 0x56).to_pixel(), 0xff12_3456);
        assert_eq!(Color::rgba(255, 255, 255, 128).to_pixel(), 0x8080_8080);
        assert_eq!(Color::rgba(200, 100, 50, 0).to_pixel(), 0);
    }

    #[test]
    fn mul_div_255_rounds_exactly() {
        for x in 0..=255u32 {
            for y in 0..=255u32 {
                let exact = (f64::from(x * y) / 255.0).round() as u32;
                assert_eq!(mul_div_255(x, y), exact, "{x} * {y}");
            }
        }
    }

    #[test]
    fn rejects_huge_canvases() {
        assert!(Canvas::new(MAX_DIMENSION + 1, 1, Color::BLACK).is_err());
        assert!(Canvas::new(MAX_DIMENSION, MAX_DIMENSION, Color::BLACK).is_err());
        assert!(Canvas::new(0, 0, Color::BLACK).is_ok());
    }

    #[test]
    fn fill_rect_is_clipped() {
        let mut canvas = Canvas::new(4, 3, Color::BLACK).unwrap();
        canvas.fill_rect(-2, 1, 4, 10, Color::WHITE);
        canvas.fill_rect(i32::MAX, i32::MAX, u32::MAX, u32::MAX, Color::WHITE);
        canvas.fill_rect(i32::MIN, i32::MIN, u32::MAX, 1, Color::WHITE);
        let white = Color::WHITE.to_pixel();
        let black = Color::BLACK.to_pixel();
        #[rustfmt::skip]
        let expected = [
            black, black, black, black,
            white, white, black, black,
            white, white, black, black,
        ];
        assert_eq!(canvas.pixels(), expected);
    }

    #[test]
    fn half_transparent_rect_blends() {
        let mut canvas = Canvas::new(1, 1, Color::BLACK).unwrap();
        canvas.fill_rect(0, 0, 1, 1, Color::rgba(255, 0, 0, 128));
        assert_eq!(canvas.pixel(0, 0), Some(0xff80_0000));
    }

    #[test]
    fn circle_is_covered_inside_and_smooth_at_the_edge() {
        let mut canvas = Canvas::new(21, 21, Color::BLACK).unwrap();
        canvas.fill_circle(10.5, 10.5, 8.0, Color::WHITE);
        assert_eq!(canvas.pixel(10, 10), Some(Color::WHITE.to_pixel()));
        assert_eq!(canvas.pixel(0, 0), Some(Color::BLACK.to_pixel()));
        // Am Rand liegen Zwischenwerte.
        let edge = canvas.pixel(10, 2).unwrap() & 0xff;
        assert!(edge > 0 && edge < 255, "Randpixel {edge:#x}");
        // Der Kreis ist symmetrisch.
        for y in 0..21 {
            for x in 0..21 {
                assert_eq!(canvas.pixel(x, y), canvas.pixel(20 - x, y));
                assert_eq!(canvas.pixel(x, y), canvas.pixel(x, 20 - y));
            }
        }
    }

    #[test]
    fn ring_leaves_the_center_free() {
        let mut canvas = Canvas::new(21, 21, Color::BLACK).unwrap();
        canvas.fill_ring(10.5, 10.5, 5.0, 9.0, Color::WHITE);
        assert_eq!(canvas.pixel(10, 10), Some(Color::BLACK.to_pixel()));
        assert_eq!(canvas.pixel(10, 3), Some(Color::WHITE.to_pixel()));
    }

    #[test]
    fn invalid_circles_draw_nothing() {
        let mut canvas = Canvas::new(5, 5, Color::BLACK).unwrap();
        let before = canvas.clone();
        canvas.fill_circle(f32::NAN, 2.0, 2.0, Color::WHITE);
        canvas.fill_circle(2.0, 2.0, f32::INFINITY, Color::WHITE);
        canvas.fill_circle(2.0, 2.0, -1.0, Color::WHITE);
        canvas.fill_ring(2.0, 2.0, 3.0, 2.0, Color::WHITE);
        assert_eq!(canvas, before);
        // Weit außerhalb liegende Kreise werden abgeschnitten.
        canvas.fill_circle(-1e30, 1e30, 5.0, Color::WHITE);
        assert_eq!(canvas, before);
    }
}
