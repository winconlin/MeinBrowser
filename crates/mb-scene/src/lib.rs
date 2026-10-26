//! Die Szenenliste zwischen Layout und Renderer.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.8. Bisher ist die Szene eine
//! Liste von Zeichenbefehlen in Malreihenfolge: gefüllte Rechtecke und
//! Vielecke, Glyphenfolgen und Bilder, auch gekachelt. Koordinaten sind CSS-Pixel mit dem Ursprung
//! oben links, die y-Achse nach unten. Wie Kanten auf Gerätepixel fallen,
//! entscheidet der Renderer. Das kompakte Binärformat, Kurven, Verläufe,
//! Masken, Filter und Mischmodi fehlen noch.

#![forbid(unsafe_code)]

use std::sync::Arc;

use mb_text::{Face, GlyphId};

/// Eine Farbe mit 8 Bit je Kanal in sRGB, nicht vormultipliziert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    pub fn is_transparent(self) -> bool {
        self.a == 0
    }
}

/// Ein Rechteck: die Ecke oben links, Breite und Höhe.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn is_empty(&self) -> bool {
        !(self.width > 0.0 && self.height > 0.0)
    }

    /// Die Schnittmenge, leer (mit Breite oder Höhe null), wenn sich die
    /// Rechtecke nicht überlappen.
    pub fn intersection(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
    }
}

/// Ein Rasterbild: zeilenweise von oben nach unten, je Pixel R, G, B und A
/// mit 8 Bit, nicht vormultipliziert.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Image {
    /// Ein Bild aus seinen Pixeln; `None`, wenn deren Zahl nicht zu Breite
    /// und Höhe passt.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Image> {
        let expected = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(4)?;
        (rgba.len() == expected).then_some(Image {
            width,
            height,
            rgba,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Das Pixel an `(x, y)`, oder `None` außerhalb des Bilds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::try_from(y)
            .ok()?
            .checked_mul(usize::try_from(self.width).ok()?)?
            .checked_add(usize::try_from(x).ok()?)?
            .checked_mul(4)?;
        match self.rgba.get(index..index.checked_add(4)?)? {
            &[r, g, b, a] => Some(Color { r, g, b, a }),
            _ => None,
        }
    }
}

/// Eine Glyphe mit dem Punkt auf der Grundlinie, an dem sie beginnt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    pub id: GlyphId,
    pub x: f32,
    pub y: f32,
}

/// Glyphen eines Schnitts in einer Größe und Farbe.
#[derive(Clone, Debug)]
pub struct GlyphRun {
    pub face: Arc<Face>,
    /// Die Schriftgröße in CSS-Pixeln.
    pub size: f32,
    pub color: Color,
    pub glyphs: Vec<PositionedGlyph>,
}

/// Ein Zeichenbefehl.
#[derive(Clone, Debug)]
pub enum Item {
    /// Ein gefülltes Rechteck.
    Rect {
        rect: Rect,
        color: Color,
    },
    Glyphs(GlyphRun),
    /// Ein Bild, auf `rect` gestreckt, mit `repeat_x` und `repeat_y` in der
    /// Richtung lückenlos wiederholt und auf `clip` beschnitten.
    Image {
        rect: Rect,
        clip: Rect,
        image: Arc<Image>,
        repeat_x: bool,
        repeat_y: bool,
    },
    /// Ein gefülltes Vieleck; die Punkte sind (x, y).
    Polygon {
        points: Vec<(f32, f32)>,
        color: Color,
    },
}

/// Eine Szene: was ein Fenster oder eine Seite zeigt.
#[derive(Clone, Debug)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    /// Die Farbe unter allem.
    pub background: Color,
    pub items: Vec<Item>,
}

impl Scene {
    pub fn new(width: f32, height: f32, background: Color) -> Scene {
        Scene {
            width,
            height,
            background,
            items: Vec::new(),
        }
    }

    /// Fügt ein Rechteck an, wenn es nicht leer und nicht durchsichtig ist.
    pub fn push_rect(&mut self, rect: Rect, color: Color) {
        if !rect.is_empty() && !color.is_transparent() {
            self.items.push(Item::Rect { rect, color });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 8.0, 10.0, 10.0);
        assert_eq!(a.intersection(&b), Rect::new(5.0, 8.0, 5.0, 2.0));
        assert!(a.intersection(&Rect::new(20.0, 0.0, 1.0, 1.0)).is_empty());
        let mut scene = Scene::new(10.0, 10.0, Color::WHITE);
        scene.push_rect(a, Color::TRANSPARENT);
        scene.push_rect(Rect::new(0.0, 0.0, 0.0, 5.0), Color::WHITE);
        assert!(scene.items.is_empty());
    }

    #[test]
    fn images() {
        assert!(Image::new(2, 1, vec![0; 7]).is_none());
        let image = Image::new(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).expect("Bild");
        assert_eq!(image.pixel(1, 0), Some(Color::rgba(5, 6, 7, 8)));
        assert_eq!(image.pixel(2, 0), None);
    }
}
