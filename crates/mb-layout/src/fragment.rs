//! Der Fragmentbaum: das Ergebnis des Layouts, unveränderlich.
//!
//! Ein Kastenfragment liegt relativ zum Inhaltsbereich (*content box*) des
//! Kastenfragments, in dem es steht; seine Kinder liegen relativ zu seinem
//! eigenen Inhaltsbereich. Inline-Kästen bilden kein eigenes
//! Koordinatensystem: Ihre Kinder liegen wie sie selbst relativ zum
//! Blockcontainer der Zeile.

use std::sync::Arc;

use mb_dom::NodeId;
use mb_scene::Image;
use mb_text::{Face, GlyphId};

use crate::boxes::StyleRef;
use crate::style::Sides;

/// Ein Rechteck in `px`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn translate(&self, dx: f64, dy: f64) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    /// Das Rechteck um `sides` verkleinert, nie kleiner als null.
    pub(crate) fn deflate(&self, sides: &Sides) -> Rect {
        Rect {
            x: self.x + sides.left,
            y: self.y + sides.top,
            width: (self.width - sides.horizontal()).max(0.0),
            height: (self.height - sides.vertical()).max(0.0),
        }
    }

    /// Das kleinste Rechteck, das beide enthält.
    pub(crate) fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Rect::new(x, y, right - x, bottom - y)
    }

    pub(crate) fn inflate(&self, amount: f64) -> Rect {
        Rect {
            x: self.x - amount,
            y: self.y - amount,
            width: self.width + 2.0 * amount,
            height: self.height + 2.0 * amount,
        }
    }
}

/// Ein Fragment.
#[derive(Clone, Debug)]
pub(crate) enum Fragment {
    Box(BoxFragment),
    /// Ein Teil eines Inline-Kastens auf einer Zeile.
    Inline(InlineFragment),
    Text(TextFragment),
    Image(ImageFragment),
}

/// Das Fragment eines Blockcontainers, Inline-Blocks oder ersetzten
/// Elements.
#[derive(Clone, Debug)]
pub(crate) struct BoxFragment {
    pub(crate) style: StyleRef,
    pub(crate) node: Option<NodeId>,
    /// Der Rahmenbereich (*border box*).
    pub(crate) rect: Rect,
    pub(crate) border: Sides,
    pub(crate) padding: Sides,
    pub(crate) children: Vec<Fragment>,
    /// Die Zeilen, die der Kasten direkt enthält, zusammengefasst und mit
    /// ihrem Inhalt, auch wo er über sie hinausragt; relativ zum
    /// Inhaltsbereich. `None` ohne Zeilen.
    pub(crate) lines: Option<Rect>,
    /// Ein Kasten auf Atomebene in einer Zeile; er wird wie ein eigener
    /// Stapelkontext gezeichnet (CSS 2, Anhang E).
    pub(crate) atomic: bool,
}

impl BoxFragment {
    pub(crate) fn padding_rect(&self) -> Rect {
        self.rect.deflate(&self.border)
    }

    pub(crate) fn content_rect(&self) -> Rect {
        self.padding_rect().deflate(&self.padding)
    }
}

/// Ein Teil eines Inline-Kastens; ohne Kante dort, wo er geteilt ist.
#[derive(Clone, Debug)]
pub(crate) struct InlineFragment {
    pub(crate) style: StyleRef,
    pub(crate) rect: Rect,
    pub(crate) border: Sides,
    pub(crate) padding: Sides,
    pub(crate) children: Vec<Fragment>,
}

/// Glyphen eines Schnitts.
#[derive(Clone, Debug)]
pub(crate) struct GlyphRun {
    pub(crate) face: Arc<Face>,
    pub(crate) size: f64,
    /// Glyphe und Anfang auf der Grundlinie, waagrecht.
    pub(crate) glyphs: Vec<(GlyphId, f64)>,
    /// Um wie viel Grad die Glyphen künstlich schräg gestellt werden, 0 für
    /// keine Scherung.
    pub(crate) oblique: f64,
}

/// Text auf einer Zeile in einem Stil.
#[derive(Clone, Debug)]
pub(crate) struct TextFragment {
    pub(crate) style: StyleRef,
    /// Die Grundlinie.
    pub(crate) baseline: f64,
    pub(crate) runs: Vec<GlyphRun>,
}

/// Der Inhalt eines ersetzten Elements.
#[derive(Clone, Debug)]
pub(crate) struct ImageFragment {
    pub(crate) style: StyleRef,
    pub(crate) rect: Rect,
    pub(crate) image: Arc<Image>,
}
