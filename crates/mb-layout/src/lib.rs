//! Formatierungskontexte und unveränderliche Fragmente.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.7. Bisher gibt es den
//! Kastenbaum aus Dokument und Stilen, Blockformatierungskontexte mit
//! zusammenfallenden Außenabständen, Inline-Formatierungskontexte mit
//! Leerraum, Zeilenumbruch und senkrechter Ausrichtung, Inline-Blöcke,
//! Bilder als ersetzte Elemente und das Zeichnen des Fragmentbaums in eine
//! Szene. Es fehlen noch Floats, Positionierung, Überlauf, Tabellen, Flex,
//! Grid, Listen, generierter Inhalt, Bidi und senkrechte Schreibmodi.

#![forbid(unsafe_code)]

mod block;
mod boxes;
mod fonts;
mod fragment;
mod inline;
mod paint;
mod style;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use mb_dom::{Document, NodeId};
use mb_scene::{Image, Scene};
use mb_style::properties::{BgImage, Layers, Longhand, SpecifiedValue};
use mb_style::{Device, Stylist};
use mb_url::Url;

pub use fonts::Fonts;

use crate::block::ContainingBlock;
use crate::boxes::{Builder, StyleRef};
use crate::fragment::BoxFragment;
use crate::style::Style;

/// Was das Layout überall braucht.
pub(crate) struct LayoutContext<'a> {
    pub(crate) fonts: &'a Fonts<'a>,
}

/// Was das Layout eines Dokuments braucht.
pub struct LayoutInput<'a> {
    pub document: &'a Document,
    pub stylist: &'a Stylist,
    /// Der Viewport ist der anfängliche umgebende Block.
    pub device: &'a Device,
    pub fonts: &'a Fonts<'a>,
    /// Die Basis für URLs in Attributen.
    pub base: &'a Url,
    /// Lädt die Bilder von `<img>`.
    pub images: &'a dyn Fn(&Url) -> Option<Arc<Image>>,
}

/// Das Ergebnis des Layouts eines Dokuments.
#[derive(Debug)]
pub struct FragmentTree {
    /// Der Kasten des Wurzelelements im anfänglichen umgebenden Block.
    root: Option<BoxFragment>,
    /// Der Stil, dessen Hintergrund die Leinwand trägt.
    canvas: Option<StyleRef>,
    /// Das `body`-Element, dessen Hintergrund auf die Leinwand ging.
    propagated_from: Option<NodeId>,
    width: f64,
    height: f64,
}

/// Ob ein Kasten einen Hintergrund hat: eine Farbe, die nicht ganz
/// durchsichtig ist, oder ein Bild.
fn has_background(style: &StyleRef) -> bool {
    let color = style
        .color_of(Longhand::BackgroundColor)
        .is_some_and(|color| color.alpha > 0);
    let image = match style.values().get(Longhand::BackgroundImage) {
        SpecifiedValue::BackgroundImage(Layers(layers)) => {
            layers.iter().any(|layer| !matches!(layer, BgImage::None))
        }
        _ => false,
    };
    color || image
}

/// Lays out a document in its viewport.
pub fn layout(input: &LayoutInput<'_>) -> FragmentTree {
    let mut builder = Builder::new(
        input.document,
        input.stylist,
        input.device,
        input.fonts,
        input.base,
        input.images,
    );
    let root = builder.build();
    let context = LayoutContext { fonts: input.fonts };
    let width = input.device.viewport_width;
    let height = input.device.viewport_height;
    let fragment = root.as_ref().map(|root| {
        let icb = ContainingBlock {
            width,
            height: Some(height),
            ltr: root.style.ltr(),
        };
        let layout = block::layout_block(&context, root, &icb, 0);
        let mut fragment = layout.fragment;
        // Die Außenabstände der Wurzel fallen mit nichts zusammen.
        fragment.rect.y = layout.margins.start.solve();
        fragment
    });
    // Ohne eigenen Hintergrund der Wurzel trägt die Leinwand den von
    // `body` (Backgrounds 3, Abschnitt 2.11.2).
    let root_style = builder.root_style.clone();
    let (canvas, propagated_from) = match (&root_style, &builder.body_style) {
        (Some(root), Some(body)) if !has_background(root) => (Some(body.clone()), builder.body),
        (root, _) => (root.clone(), None),
    };
    FragmentTree {
        root: fragment,
        canvas,
        propagated_from,
        width,
        height,
    }
}

impl FragmentTree {
    /// Zeichnet das Dokument; `images` lädt die Hintergrundbilder.
    pub fn paint(&self, images: &dyn Fn(&Url) -> Option<Arc<Image>>) -> Scene {
        let canvas = paint::Canvas {
            width: self.width,
            height: self.height,
            background: self.canvas.as_ref(),
            propagated_from: self.propagated_from,
        };
        paint::paint(self.root.as_ref(), &canvas, images)
    }
}
