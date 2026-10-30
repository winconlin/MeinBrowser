//! Vom Fragmentbaum zur Szene, in der Reihenfolge aus CSS 2, Anhang E, für
//! einen Stapelkontext ohne Positionierung und Floats:
//!
//! 1. der Hintergrund der Leinwand (*canvas*), dann Rahmen der Wurzel;
//! 2. Hintergründe und Rahmen der blockartigen Nachfahren in
//!    Baumreihenfolge;
//! 3. je Block die ersetzten Inhalte und Zeilen: Hintergrund und Rahmen der
//!    Inline-Kästen, Text, Kästen auf Atomebene wie eigene Stapelkontexte;
//! 4. zuletzt die Umrisse (`outline`).
//!
//! Die Leinwand bekommt den Hintergrund der Wurzel; ist der durchsichtig
//! und ohne Bild, bei einem `html`-Element den des ersten `body`-Kindes
//! (Backgrounds 3, Abschnitt 2.11.2), sonst bleibt sie weiß. Hintergrund-
//! bilder werden bei `space` und `round` wie bei `repeat` gekachelt.
//!
//! Rahmen und Umrisse haben Ecken ohne Rundung. `dotted` zeichnet runde
//! Punkte, `dashed` Striche mit geraden Enden (Backgrounds 3, Abschnitt
//! 3.2); `double`, `groove`, `ridge`, `inset` und `outset` sind noch
//! durchgezogen. Wie lang Striche und Lücken sind, lässt die Spezifikation
//! offen; festgelegt ist hier:
//!
//! - Ein Strich ist dreimal so lang, wie die Seite breit ist. Eine Seite
//!   beginnt und endet mit einem Strich über die Ecke, und die Lücken
//!   dazwischen sind gleich lang, mindestens so lang wie ein Strich. Ist
//!   dafür kein Platz, ist die Seite durchgezogen.
//! - Ein Punkt ist so breit wie die Seite, von Mitte zu Mitte sind es etwa
//!   zwei Breiten. Der erste und der letzte Punkt sitzen in der Mitte der
//!   Ecke, ohne Nachbarseite am Rand.
//! - In einer Ecke trennt die Diagonale zwei verschiedene Seiten (Abschnitt
//!   4.4 überlässt die Form des Übergangs dem Browser). Gleiche Seiten
//!   treffen sich gerade, die Ecke gehört dann der Seite oben oder unten.
//!   Kanten liegen auf ganzen Gerätepixeln, die Enden der Striche auch.
//!
//! `visibility: hidden` lässt Hintergrund, Rahmen, Text, Bild und Umriss des
//! Kastens weg, nicht die seiner Kinder.
//!
//! Gezeichnet wird, was der Viewport an seiner Scrollposition zeigt: Alle
//! Kästen sind um sie verschoben. Der Hintergrund der Leinwand füllt immer
//! den ganzen Viewport, und Hintergrundbilder mit `background-attachment:
//! fixed` liegen fest im Viewport.

use std::sync::Arc;

use mb_dom::NodeId;
use mb_scene::{Color, Image, Item, PositionedGlyph, Scene};
use mb_style::properties::{
    Attachment, BgClip, BgImage, BgSize, HorizontalEdge, Layers, LengthPercentageOrAuto, LineStyle,
    Longhand, PositionComponent, RepeatStyle, Repetition, SpecifiedValue, VerticalEdge, VisualBox,
};
use mb_style::values::color::Rgba;
use mb_url::Url;

use crate::boxes::StyleRef;
use crate::fonts::to_f32;
use crate::fragment::{BoxFragment, Fragment, InlineFragment, Rect, TextFragment};
use crate::style::{Side, Sides, Style, resolve};

/// Lädt ein Bild.
pub(crate) type Images<'a> = &'a dyn Fn(&Url) -> Option<Arc<Image>>;

fn scene_rect(rect: &Rect) -> mb_scene::Rect {
    mb_scene::Rect::new(
        to_f32(rect.x),
        to_f32(rect.y),
        to_f32(rect.width),
        to_f32(rect.height),
    )
}

fn scene_color(rgba: Rgba) -> Color {
    Color::rgba(rgba.red, rgba.green, rgba.blue, rgba.alpha)
}

/// Was die Szene über die Leinwand wissen muss.
pub(crate) struct Canvas<'a> {
    pub(crate) width: f64,
    pub(crate) height: f64,
    /// Der Stil, dessen Hintergrund die Leinwand trägt.
    pub(crate) background: Option<&'a StyleRef>,
    /// Das `body`-Element, dessen Hintergrund auf die Leinwand ging.
    pub(crate) propagated_from: Option<NodeId>,
    /// Wie weit der Viewport nach rechts und unten gescrollt ist.
    pub(crate) scroll: (f64, f64),
    /// Gerätepixel je CSS-Pixel.
    pub(crate) device_pixel_ratio: f64,
}

pub(crate) struct Painter<'a> {
    scene: Scene,
    images: Images<'a>,
    viewport: Rect,
    propagated_from: Option<NodeId>,
    /// Gerätepixel je CSS-Pixel, gültig.
    scale: f64,
}

/// Zeichnet den Fragmentbaum; `root` liegt im anfänglichen umgebenden
/// Block, der Viewport an `canvas.scroll`.
pub(crate) fn paint(root: Option<&BoxFragment>, canvas: &Canvas<'_>, images: Images<'_>) -> Scene {
    let viewport = Rect::new(0.0, 0.0, canvas.width, canvas.height);
    let mut painter = Painter {
        scene: Scene::new(to_f32(canvas.width), to_f32(canvas.height), Color::WHITE),
        images,
        viewport,
        propagated_from: canvas.propagated_from,
        scale: if canvas.device_pixel_ratio.is_finite() && canvas.device_pixel_ratio > 0.0 {
            canvas.device_pixel_ratio
        } else {
            1.0
        },
    };
    let origin = (-canvas.scroll.0, -canvas.scroll.1);
    if let (Some(root), Some(style)) = (root, canvas.background) {
        // Die Bilder liegen, wie sie für die Wurzel lägen; gezeichnet wird
        // auf der ganzen Leinwand.
        painter.background(style, root, origin, Some(viewport));
    }
    if let Some(root) = root {
        painter.stacking_context(root, origin, true);
    }
    painter.scene
}

impl Painter<'_> {
    /// Ein Kasten mit allem darin, wie ein Stapelkontext.
    fn stacking_context(&mut self, fragment: &BoxFragment, origin: (f64, f64), root: bool) {
        self.block_backgrounds(fragment, origin, root);
        self.block_contents(fragment, origin);
        self.outlines(fragment, origin);
    }

    /// Hintergrund und Rahmen eines Blocks und seiner blockartigen
    /// Nachfahren.
    fn block_backgrounds(&mut self, fragment: &BoxFragment, origin: (f64, f64), root: bool) {
        if fragment.style.visible() {
            let propagated = fragment.node.is_some() && fragment.node == self.propagated_from;
            if !root && !propagated {
                self.background(&fragment.style, fragment, origin, None);
            }
            self.border(
                &fragment.style,
                &fragment.rect.translate(origin.0, origin.1),
                &fragment.border,
            );
        }
        let inner = content_origin(fragment, origin);
        for child in &fragment.children {
            if let Fragment::Box(child) = child
                && !child.atomic
            {
                self.block_backgrounds(child, inner, false);
            }
        }
    }

    /// Ersetzte Inhalte und Zeilen eines Blocks und seiner blockartigen
    /// Nachfahren.
    fn block_contents(&mut self, fragment: &BoxFragment, origin: (f64, f64)) {
        let inner = content_origin(fragment, origin);
        for child in &fragment.children {
            match child {
                Fragment::Box(child) if child.atomic => self.stacking_context(child, inner, false),
                Fragment::Box(child) => self.block_contents(child, inner),
                Fragment::Image(image) => {
                    if image.style.visible() {
                        let rect = image.rect.translate(inner.0, inner.1);
                        self.scene.items.push(Item::Image {
                            rect: scene_rect(&rect),
                            clip: scene_rect(&rect),
                            image: Arc::clone(&image.image),
                            repeat_x: false,
                            repeat_y: false,
                        });
                    }
                }
                Fragment::Inline(inline) => self.inline(inline, inner),
                Fragment::Text(text) => self.text(text, inner),
            }
        }
    }

    fn inline(&mut self, fragment: &InlineFragment, origin: (f64, f64)) {
        let rect = fragment.rect.translate(origin.0, origin.1);
        if fragment.style.visible() {
            let area = BoxAreas {
                border_box: rect,
                border: fragment.border,
                padding: fragment.padding,
            };
            self.background_layers(&fragment.style, &area, None);
            self.border(&fragment.style, &rect, &fragment.border);
        }
        for child in &fragment.children {
            match child {
                Fragment::Box(child) if child.atomic => self.stacking_context(child, origin, false),
                Fragment::Box(child) => self.block_contents(child, origin),
                Fragment::Inline(inline) => self.inline(inline, origin),
                Fragment::Text(text) => self.text(text, origin),
                Fragment::Image(_) => {}
            }
        }
    }

    fn text(&mut self, fragment: &TextFragment, origin: (f64, f64)) {
        if !fragment.style.visible() {
            return;
        }
        let Some(color) = fragment.style.text_color() else {
            return;
        };
        let color = scene_color(color);
        if color.is_transparent() {
            return;
        }
        let y = to_f32(origin.1 + fragment.baseline);
        for run in &fragment.runs {
            self.scene.items.push(Item::Glyphs(mb_scene::GlyphRun {
                face: Arc::clone(&run.face),
                size: to_f32(run.size),
                color,
                glyphs: run
                    .glyphs
                    .iter()
                    .map(|&(id, x)| PositionedGlyph {
                        id,
                        x: to_f32(origin.0 + x),
                        y,
                    })
                    .collect(),
                oblique: to_f32(run.oblique),
            }));
        }
    }

    /// Die Umrisse eines Kastens und seiner Nachfahren außerhalb von
    /// Kästen auf Atomebene, die ihre eigenen zeichnen.
    fn outlines(&mut self, fragment: &BoxFragment, origin: (f64, f64)) {
        self.outline(
            &fragment.style,
            &fragment.rect.translate(origin.0, origin.1),
        );
        let inner = content_origin(fragment, origin);
        self.child_outlines(&fragment.children, inner);
    }

    fn child_outlines(&mut self, children: &[Fragment], origin: (f64, f64)) {
        for child in children {
            match child {
                Fragment::Box(child) if !child.atomic => self.outlines(child, origin),
                Fragment::Inline(inline) => {
                    self.outline(&inline.style, &inline.rect.translate(origin.0, origin.1));
                    self.child_outlines(&inline.children, origin);
                }
                Fragment::Box(_) | Fragment::Text(_) | Fragment::Image(_) => {}
            }
        }
    }

    fn outline(&mut self, style: &StyleRef, border_box: &Rect) {
        if !style.visible() {
            return;
        }
        let Some((width, line_style, Some(color))) = style.outline() else {
            return;
        };
        let inner = border_box.inflate(style.outline_offset());
        let outer = inner.inflate(width);
        let color = scene_color(color);
        if patterned(line_style) {
            self.sides(&outer, &inner, [Some(color); 4], [line_style; 4]);
            return;
        }
        let sides = [
            Rect::new(outer.x, outer.y, outer.width, width),
            Rect::new(outer.x, inner.y + inner.height, outer.width, width),
            Rect::new(outer.x, inner.y, width, inner.height),
            Rect::new(inner.x + inner.width, inner.y, width, inner.height),
        ];
        for side in sides {
            self.scene.push_rect(scene_rect(&side), color);
        }
    }

    /// Die Rahmen eines Kastens. Geteilte Inline-Kästen haben an der
    /// Teilungsstelle schon die Breite null.
    fn border(&mut self, style: &StyleRef, rect: &Rect, border: &Sides) {
        let colors = Side::ALL.map(|side| {
            let width = match side {
                Side::Top => border.top,
                Side::Right => border.right,
                Side::Bottom => border.bottom,
                Side::Left => border.left,
            };
            (width > 0.0)
                .then(|| style.border_color(side).map(scene_color))
                .flatten()
                .filter(|color| !color.is_transparent())
        });
        if colors.iter().all(Option::is_none) {
            return;
        }
        let styles = Side::ALL.map(|side| style.border_style(side));
        let [top, right, bottom, left] = colors;
        let outer = *rect;
        let inner = rect.deflate(border);
        // Rechtecke nur, wenn alle Seiten mit einer Breite dieselbe sichtbare
        // Farbe haben und durchgezogen sind; neben einer durchsichtigen Seite
        // endet ein Rahmen schräg in der Ecke.
        let mut sides = [border.top, border.right, border.bottom, border.left]
            .into_iter()
            .zip(colors.into_iter().zip(styles))
            .filter(|(width, _)| *width > 0.0)
            .map(|(_, side)| side);
        let first = sides
            .next()
            .and_then(|(color, style)| color.filter(|_| !patterned(style)));
        if first.is_some() && sides.all(|(color, style)| color == first && !patterned(style)) {
            // Eine Farbe: Oben und unten reichen über die Ecken.
            let rects = [
                (top, Rect::new(outer.x, outer.y, outer.width, border.top)),
                (
                    bottom,
                    Rect::new(outer.x, inner.y + inner.height, outer.width, border.bottom),
                ),
                (left, Rect::new(outer.x, inner.y, border.left, inner.height)),
                (
                    right,
                    Rect::new(inner.x + inner.width, inner.y, border.right, inner.height),
                ),
            ];
            for (color, rect) in rects {
                if let Some(color) = color {
                    self.scene.push_rect(scene_rect(&rect), color);
                }
            }
            return;
        }
        self.sides(&outer, &inner, colors, styles);
    }

    /// Die vier Seiten zwischen `outer` und `inner` (oben, rechts, unten,
    /// links), die sich in den Ecken schräg treffen. Die Ecken liegen auf
    /// ganzen Gerätepixeln wie die Kanten von Rechtecken. Gleiche Seiten
    /// (Farbe, Stil und bei Mustern die Breite) treffen sich gerade: Die Ecke
    /// gehört dann ganz der Seite oben oder unten. An der Diagonale bekämen
    /// beide Seiten dort nur einen Teil der Pixel, und die Ecke hätte eine
    /// helle Naht.
    fn sides(
        &mut self,
        outer: &Rect,
        inner: &Rect,
        colors: [Option<Color>; 4],
        styles: [LineStyle; 4],
    ) {
        let scale = self.scale;
        let snap = move |value: f64| (value * scale).round() / scale;
        let (x0, y0, x1, y1) = (
            snap(outer.x),
            snap(outer.y),
            snap(outer.x + outer.width),
            snap(outer.y + outer.height),
        );
        let (ix0, iy0, ix1, iy1) = (
            snap(inner.x),
            snap(inner.y),
            snap(inner.x + inner.width),
            snap(inner.y + inner.height),
        );
        let (top, right, bottom, left) = (iy0 - y0, x1 - ix1, y1 - iy1, ix0 - x0);
        let widths = [top, right, bottom, left];
        let same = |a: usize, b: usize| {
            let (Some(color_a), Some(color_b)) = (
                colors.get(a).copied().flatten(),
                colors.get(b).copied().flatten(),
            ) else {
                return false;
            };
            let (Some(&style_a), Some(&style_b)) = (styles.get(a), styles.get(b)) else {
                return false;
            };
            color_a == color_b
                && patterned(style_a) == patterned(style_b)
                && (!patterned(style_a) || (style_a == style_b && widths.get(a) == widths.get(b)))
        };
        let (top_left, top_right, bottom_right, bottom_left) =
            (same(0, 3), same(0, 1), same(2, 1), same(2, 3));
        let pick = |joined: bool, straight: f64, slanted: f64| {
            if joined { straight } else { slanted }
        };
        let bands = [
            Band {
                region: [
                    (x0, y0),
                    (x1, y0),
                    (pick(top_right, x1, ix1), iy0),
                    (pick(top_left, x0, ix0), iy0),
                ],
                along: (x0, x1),
                across: (y0, iy0),
                ends: (left, right),
                vertical: false,
            },
            Band {
                region: [
                    (x1, pick(top_right, iy0, y0)),
                    (x1, pick(bottom_right, iy1, y1)),
                    (ix1, iy1),
                    (ix1, iy0),
                ],
                along: (y0, y1),
                across: (ix1, x1),
                ends: (top, bottom),
                vertical: true,
            },
            Band {
                region: [
                    (x1, y1),
                    (x0, y1),
                    (pick(bottom_left, x0, ix0), iy1),
                    (pick(bottom_right, x1, ix1), iy1),
                ],
                along: (x0, x1),
                across: (iy1, y1),
                ends: (left, right),
                vertical: false,
            },
            Band {
                region: [
                    (x0, pick(bottom_left, iy1, y1)),
                    (x0, pick(top_left, iy0, y0)),
                    (ix0, iy0),
                    (ix0, iy1),
                ],
                along: (y0, y1),
                across: (x0, ix0),
                ends: (top, bottom),
                vertical: true,
            },
        ];
        for ((band, color), style) in bands.iter().zip(colors).zip(styles) {
            let Some(color) = color else {
                continue;
            };
            if !patterned(style) {
                self.push_polygon(band.region.to_vec(), color);
                continue;
            }
            // Nur, was der Viewport zeigt: Eine sehr lange Seite hätte sonst
            // sehr viele Striche oder Punkte.
            let (along, across) = if band.vertical {
                (
                    (self.viewport.y, self.viewport.y + self.viewport.height),
                    (self.viewport.x, self.viewport.x + self.viewport.width),
                )
            } else {
                (
                    (self.viewport.x, self.viewport.x + self.viewport.width),
                    (self.viewport.y, self.viewport.y + self.viewport.height),
                )
            };
            if band.across.1 < across.0 || band.across.0 > across.1 {
                continue;
            }
            let width = band.across.1 - band.across.0;
            let piece = |(start, end): (f64, f64)| {
                vec![
                    band.point(start, band.across.0),
                    band.point(end, band.across.0),
                    band.point(end, band.across.1),
                    band.point(start, band.across.1),
                ]
            };
            let pieces: Vec<Vec<(f64, f64)>> = if style == LineStyle::Dotted {
                let middle = (band.across.0 + band.across.1) / 2.0;
                let radius = width / 2.0;
                // Rund wären Punkte unter drei Gerätepixeln nur blasser.
                let square = width * scale < 3.0;
                match dots(band.along, band.ends, width, along) {
                    Some(centers) => centers
                        .into_iter()
                        .map(|center| {
                            if square {
                                let start = snap(center - radius);
                                piece((start, start + width))
                            } else {
                                circle(band.point(center, middle), radius, width * scale)
                            }
                        })
                        .collect(),
                    None => vec![band.region.to_vec()],
                }
            } else {
                dashes(band.along, width, along, snap)
                    .into_iter()
                    .map(piece)
                    .collect()
            };
            for piece in pieces {
                // Was in einer geraden Ecke der anderen Seite gehört, bleibt
                // ohne Fläche übrig.
                let clipped = clip_convex(piece, &band.region);
                if area(&clipped).abs() > 1e-6 {
                    self.push_polygon(clipped, color);
                }
            }
        }
    }

    fn push_polygon(&mut self, points: Vec<(f64, f64)>, color: Color) {
        if points.len() < 3 {
            return;
        }
        let points = points
            .into_iter()
            .map(|(x, y)| (to_f32(x), to_f32(y)))
            .collect();
        self.scene.items.push(Item::Polygon { points, color });
    }

    /// Der Hintergrund eines Kastens; mit `canvas` auf die ganze Leinwand.
    fn background(
        &mut self,
        style: &StyleRef,
        fragment: &BoxFragment,
        origin: (f64, f64),
        canvas: Option<Rect>,
    ) {
        let area = BoxAreas {
            border_box: fragment.rect.translate(origin.0, origin.1),
            border: fragment.border,
            padding: fragment.padding,
        };
        self.background_layers(style, &area, canvas);
    }

    fn background_layers(&mut self, style: &StyleRef, area: &BoxAreas, canvas: Option<Rect>) {
        let images = match style.values().get(Longhand::BackgroundImage) {
            SpecifiedValue::BackgroundImage(Layers(layers)) => layers.as_slice(),
            _ => &[],
        };
        let clips = layer_list(style, Longhand::BackgroundClip, |value| match value {
            SpecifiedValue::BackgroundClip(Layers(layers)) => Some(layers.as_slice()),
            _ => None,
        });
        // Die Farbe liegt unter allen Bildern und wird wie das unterste
        // beschnitten.
        let bottom = images.len().saturating_sub(1);
        let clip_of = |index: usize| -> Rect {
            if let Some(canvas) = canvas {
                return canvas;
            }
            match cycle(clips, index) {
                Some(BgClip::Box(VisualBox::PaddingBox)) => area.padding_box(),
                Some(BgClip::Box(VisualBox::ContentBox)) => area.content_box(),
                _ => area.border_box,
            }
        };
        if let Some(color) = style.color_of(Longhand::BackgroundColor) {
            let rect = clip_of(bottom);
            self.scene.push_rect(scene_rect(&rect), scene_color(color));
        }
        for (index, image) in images.iter().enumerate().rev() {
            let BgImage::Url(url) = image else {
                continue;
            };
            let Some(image) = url.resolved.as_ref().and_then(|url| (self.images)(url)) else {
                continue;
            };
            let clip = clip_of(index);
            self.background_image(style, area, index, image, &clip);
        }
    }

    fn background_image(
        &mut self,
        style: &StyleRef,
        area: &BoxAreas,
        index: usize,
        image: Arc<Image>,
        clip: &Rect,
    ) {
        let attachment = layer_list(style, Longhand::BackgroundAttachment, |value| match value {
            SpecifiedValue::BackgroundAttachment(Layers(layers)) => Some(layers.as_slice()),
            _ => None,
        });
        let origins = layer_list(style, Longhand::BackgroundOrigin, |value| match value {
            SpecifiedValue::BackgroundOrigin(Layers(layers)) => Some(layers.as_slice()),
            _ => None,
        });
        let positioning = if cycle(attachment, index) == Some(&Attachment::Fixed) {
            self.viewport
        } else {
            match cycle(origins, index) {
                Some(VisualBox::BorderBox) => area.border_box,
                Some(VisualBox::ContentBox) => area.content_box(),
                _ => area.padding_box(),
            }
        };
        let natural = (f64::from(image.width()), f64::from(image.height()));
        if natural.0 <= 0.0 || natural.1 <= 0.0 {
            return;
        }
        let sizes = layer_list(style, Longhand::BackgroundSize, |value| match value {
            SpecifiedValue::BackgroundSize(Layers(layers)) => Some(layers.as_slice()),
            _ => None,
        });
        let (width, height) = image_size(cycle(sizes, index).copied(), natural, &positioning);
        if !(width > 0.0 && height > 0.0) {
            return;
        }
        let xs = layer_list(style, Longhand::BackgroundPositionX, |value| match value {
            SpecifiedValue::BackgroundPositionX(Layers(layers)) => Some(layers.as_slice()),
            _ => None,
        });
        let ys = layer_list(style, Longhand::BackgroundPositionY, |value| match value {
            SpecifiedValue::BackgroundPositionY(Layers(layers)) => Some(layers.as_slice()),
            _ => None,
        });
        let free_x = positioning.width - width;
        let free_y = positioning.height - height;
        let x = match cycle(xs, index) {
            Some(component) => horizontal_offset(component, free_x),
            None => 0.0,
        };
        let y = match cycle(ys, index) {
            Some(component) => vertical_offset(component, free_y),
            None => 0.0,
        };
        let repeats = layer_list(style, Longhand::BackgroundRepeat, |value| match value {
            SpecifiedValue::BackgroundRepeat(Layers(layers)) => Some(layers.as_slice()),
            _ => None,
        });
        let (repeat_x, repeat_y) = match cycle(repeats, index) {
            Some(RepeatStyle::Pair(x, y)) => {
                (*x != Repetition::NoRepeat, *y != Repetition::NoRepeat)
            }
            Some(RepeatStyle::Inline) => (true, false),
            Some(RepeatStyle::Block) => (false, true),
            None => (true, true),
        };
        let tile = Rect::new(positioning.x + x, positioning.y + y, width, height);
        self.scene.items.push(Item::Image {
            rect: scene_rect(&tile),
            clip: scene_rect(clip),
            image,
            repeat_x,
            repeat_y,
        });
    }
}

/// Der Ursprung für die Kinder eines Kastens: sein Inhaltsbereich.
fn content_origin(fragment: &BoxFragment, origin: (f64, f64)) -> (f64, f64) {
    let content = fragment.content_rect();
    (origin.0 + content.x, origin.1 + content.y)
}

/// Die Bereiche eines Kastens für Hintergründe.
struct BoxAreas {
    border_box: Rect,
    border: Sides,
    padding: Sides,
}

impl BoxAreas {
    fn padding_box(&self) -> Rect {
        self.border_box.deflate(&self.border)
    }

    fn content_box(&self) -> Rect {
        self.padding_box().deflate(&self.padding)
    }
}

/// Die Liste einer Langform mit Ebenen.
fn layer_list<T>(
    style: &StyleRef,
    longhand: Longhand,
    get: impl Fn(&SpecifiedValue) -> Option<&[T]>,
) -> &[T] {
    get(style.values().get(longhand)).unwrap_or(&[])
}

/// Der Wert für eine Ebene: Kürzere Listen wiederholen sich.
fn cycle<T>(list: &[T], index: usize) -> Option<&T> {
    let len = list.len();
    if len == 0 {
        return None;
    }
    list.get(index % len)
}

/// Die Größe eines Hintergrundbildes nach `background-size`.
fn image_size(size: Option<BgSize>, natural: (f64, f64), area: &Rect) -> (f64, f64) {
    let (natural_width, natural_height) = natural;
    match size.unwrap_or(BgSize::AUTO) {
        BgSize::Cover | BgSize::Contain if area.width <= 0.0 || area.height <= 0.0 => (0.0, 0.0),
        BgSize::Cover => {
            let scale = (area.width / natural_width).max(area.height / natural_height);
            (natural_width * scale, natural_height * scale)
        }
        BgSize::Contain => {
            let scale = (area.width / natural_width).min(area.height / natural_height);
            (natural_width * scale, natural_height * scale)
        }
        BgSize::Size { width, height } => {
            let length = |value: LengthPercentageOrAuto, basis: f64| match value {
                LengthPercentageOrAuto::Auto => None,
                LengthPercentageOrAuto::LengthPercentage(value) => Some(resolve(value, basis)),
            };
            match (length(width, area.width), length(height, area.height)) {
                (Some(width), Some(height)) => (width, height),
                (Some(width), None) => (width, width * natural_height / natural_width),
                (None, Some(height)) => (height * natural_width / natural_height, height),
                (None, None) => natural,
            }
        }
    }
}

fn horizontal_offset(component: &PositionComponent<HorizontalEdge>, free: f64) -> f64 {
    let from_end =
        |edge: &HorizontalEdge| matches!(edge, HorizontalEdge::Right | HorizontalEdge::XEnd);
    match component {
        PositionComponent::Center => free / 2.0,
        PositionComponent::Edge(edge) => {
            if from_end(edge) {
                free
            } else {
                0.0
            }
        }
        PositionComponent::Offset(value) => resolve(*value, free),
        PositionComponent::EdgeOffset(edge, value) => {
            if from_end(edge) {
                free - resolve(*value, free)
            } else {
                resolve(*value, free)
            }
        }
    }
}

fn vertical_offset(component: &PositionComponent<VerticalEdge>, free: f64) -> f64 {
    let from_end = |edge: &VerticalEdge| matches!(edge, VerticalEdge::Bottom | VerticalEdge::YEnd);
    match component {
        PositionComponent::Center => free / 2.0,
        PositionComponent::Edge(edge) => {
            if from_end(edge) {
                free
            } else {
                0.0
            }
        }
        PositionComponent::Offset(value) => resolve(*value, free),
        PositionComponent::EdgeOffset(edge, value) => {
            if from_end(edge) {
                free - resolve(*value, free)
            } else {
                resolve(*value, free)
            }
        }
    }
}

/// Höchstens so viele Striche oder Punkte je Seite im Viewport; bei mehr
/// ist die Seite durchgezogen. Das kommt erst bei sehr hohen Skalierungen
/// vor, denn ein Strich mit Lücke ist mindestens sechs Gerätepixel lang.
const MAX_PIECES: f64 = 65536.0;

/// Ob ein Linienstil ein Muster hat, statt durchgezogen zu sein.
fn patterned(style: LineStyle) -> bool {
    matches!(style, LineStyle::Dotted | LineStyle::Dashed)
}

/// Eine Seite eines Rahmens oder Umrisses.
struct Band {
    /// Ihr Bereich, ein Trapez mit der Außenkante zuerst.
    region: [(f64, f64); 4],
    /// Längs von Ecke zu Ecke, außen gemessen.
    along: (f64, f64),
    /// Quer, der kleinere Wert zuerst.
    across: (f64, f64),
    /// Die Breiten der Nachbarseiten am Anfang und am Ende.
    ends: (f64, f64),
    /// Ob die Seite senkrecht verläuft (rechts und links).
    vertical: bool,
}

impl Band {
    /// Der Punkt `along` längs und `across` quer.
    fn point(&self, along: f64, across: f64) -> (f64, f64) {
        if self.vertical {
            (across, along)
        } else {
            (along, across)
        }
    }
}

/// Die Striche einer Seite längs von `along.0` bis `along.1`, die `visible`
/// berühren, als Anfang und Ende; `snap` legt sie auf ganze Gerätepixel.
/// Ohne Platz für zwei Striche oder bei zu vielen ist es ein Strich über
/// die ganze Seite.
fn dashes(
    along: (f64, f64),
    width: f64,
    visible: (f64, f64),
    snap: impl Fn(f64) -> f64,
) -> Vec<(f64, f64)> {
    let (start, end) = along;
    let length = end - start;
    if !(width > 0.0 && length > 0.0 && length.is_finite()) {
        return Vec::new();
    }
    let whole = || {
        if start <= visible.1 && end >= visible.0 {
            vec![(start, end)]
        } else {
            Vec::new()
        }
    };
    let dash = 3.0 * width;
    // So viele Striche, dass die Lücken mindestens einen Strich lang sind.
    let count = ((length + dash) / (2.0 * dash)).floor();
    if count < 2.0 {
        return whole();
    }
    let step = dash + (length - count * dash) / (count - 1.0);
    let first = ((visible.0 - start - dash) / step).floor().max(0.0);
    let last = ((visible.1 - start) / step).ceil().min(count - 1.0);
    let span = last - first;
    if span > MAX_PIECES {
        return whole();
    }
    let mut result = Vec::new();
    // Gezählt wird ab null, damit es auch bei sehr großen `first` weitergeht.
    let mut offset = 0.0;
    while offset <= span {
        let from = start + (first + offset) * step;
        result.push((snap(from), snap((from + dash).min(end))));
        offset += 1.0;
    }
    result
}

/// Die Mittelpunkte der Punkte einer Seite längs von `along.0` bis
/// `along.1`, die `visible` berühren. `ends` sind die Breiten der
/// Nachbarseiten: Der erste und der letzte Punkt sitzen in der Mitte der
/// Ecken. `None`, wenn es zu viele wären.
fn dots(along: (f64, f64), ends: (f64, f64), width: f64, visible: (f64, f64)) -> Option<Vec<f64>> {
    if !(width > 0.0 && (along.1 - along.0).is_finite()) {
        return Some(Vec::new());
    }
    let radius = width / 2.0;
    let inset = |neighbour: f64| {
        if neighbour > 0.0 {
            neighbour / 2.0
        } else {
            radius
        }
    };
    let first_center = along.0 + inset(ends.0);
    let last_center = along.1 - inset(ends.1);
    let distance = last_center - first_center;
    // Ist kein Platz für zwei Punkte nebeneinander, gibt es einen in der
    // Mitte; zwei würden sich überdecken, bei durchscheinender Farbe mit
    // einer dunkleren Stelle.
    if distance < width {
        let center = (first_center + last_center) / 2.0;
        let touches = center + radius >= visible.0 && center - radius <= visible.1;
        return Some(if touches { vec![center] } else { Vec::new() });
    }
    // Von Mitte zu Mitte etwa zwei Breiten.
    let intervals = (distance / (2.0 * width)).round().max(1.0);
    let step = distance / intervals;
    let first = ((visible.0 - radius - first_center) / step)
        .floor()
        .max(0.0);
    let last = ((visible.1 + radius - first_center) / step)
        .ceil()
        .min(intervals);
    let span = last - first;
    if span > MAX_PIECES {
        return None;
    }
    let mut centers = Vec::new();
    let mut offset = 0.0;
    while offset <= span {
        centers.push(first_center + (first + offset) * step);
        offset += 1.0;
    }
    Some(centers)
}

/// Ein Kreis als Vieleck, mit mehr Ecken, je größer er auf dem Gerät ist.
fn circle(center: (f64, f64), radius: f64, device_diameter: f64) -> Vec<(f64, f64)> {
    let corners: u32 = if device_diameter <= 8.0 {
        16
    } else if device_diameter <= 32.0 {
        32
    } else {
        64
    };
    (0..corners)
        .map(|corner| {
            let angle = std::f64::consts::TAU * f64::from(corner) / f64::from(corners);
            (
                center.0 + radius * angle.cos(),
                center.1 + radius * angle.sin(),
            )
        })
        .collect()
}

fn cross(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Die doppelte Fläche eines Vielecks mit Vorzeichen; das Vorzeichen sagt,
/// auf welcher Seite der Kanten das Innere liegt.
fn area(points: &[(f64, f64)]) -> f64 {
    let Some(&anchor) = points.first() else {
        return 0.0;
    };
    points
        .windows(2)
        .map(|pair| match pair {
            [a, b] => cross(anchor, *a, *b),
            _ => 0.0,
        })
        .sum()
}

/// Der Teil des konvexen Vielecks `subject`, der im konvexen Vieleck `clip`
/// liegt, nach Sutherland und Hodgman; leer, wenn `clip` keine Fläche hat.
fn clip_convex(subject: Vec<(f64, f64)>, clip: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let area = area(clip);
    let orientation = if area > 0.0 {
        1.0
    } else if area < 0.0 {
        -1.0
    } else {
        return Vec::new();
    };
    let mut result = subject;
    for (&from, &to) in clip.iter().zip(clip.iter().cycle().skip(1)) {
        let side = |point: (f64, f64)| orientation * cross(from, to, point);
        let crossing = |a: (f64, f64), b: (f64, f64)| {
            let (side_a, side_b) = (side(a), side(b));
            let t = side_a / (side_a - side_b);
            (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1))
        };
        let input = std::mem::take(&mut result);
        let Some(&last) = input.last() else {
            break;
        };
        let mut previous = last;
        for &current in &input {
            match (side(previous) >= 0.0, side(current) >= 0.0) {
                (true, true) => result.push(current),
                (true, false) => result.push(crossing(previous, current)),
                (false, true) => {
                    result.push(crossing(previous, current));
                    result.push(current);
                }
                (false, false) => {}
            }
            previous = current;
        }
    }
    result
}
