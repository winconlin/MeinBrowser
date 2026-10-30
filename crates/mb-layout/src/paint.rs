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
//! (Backgrounds 3, Abschnitt 2.11.2), sonst bleibt sie weiß. Rahmen werden
//! alle durchgezogen gezeichnet, auch `dotted`, `dashed`, `double`,
//! `groove`, `ridge`, `inset` und `outset`; Ecken ohne Rundung. Hintergrund-
//! bilder werden bei `space` und `round` wie bei `repeat` gekachelt.
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
    Attachment, BgClip, BgImage, BgSize, HorizontalEdge, Layers, LengthPercentageOrAuto, Longhand,
    PositionComponent, RepeatStyle, Repetition, SpecifiedValue, VerticalEdge, VisualBox,
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
}

pub(crate) struct Painter<'a> {
    scene: Scene,
    images: Images<'a>,
    viewport: Rect,
    propagated_from: Option<NodeId>,
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
        let Some((width, _, Some(color))) = style.outline() else {
            return;
        };
        let inner = border_box.inflate(style.outline_offset());
        let outer = inner.inflate(width);
        let color = scene_color(color);
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
        let [top, right, bottom, left] = colors;
        let outer = *rect;
        let inner = rect.deflate(border);
        // Rechtecke nur, wenn alle Seiten mit einer Breite dieselbe sichtbare
        // Farbe haben; neben einer durchsichtigen Seite endet ein Rahmen
        // schräg in der Ecke.
        let mut sides = [border.top, border.right, border.bottom, border.left]
            .into_iter()
            .zip(colors)
            .filter(|(width, _)| *width > 0.0)
            .map(|(_, color)| color);
        let first = sides.next().flatten();
        if first.is_some() && sides.all(|color| color == first) {
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
        // Verschiedene Farben treffen sich in den Ecken schräg. Die Ecken
        // liegen auf ganzen Pixeln wie die Kanten von Rechtecken.
        let snap = |value: f64| to_f32(value.round());
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
        let polygons = [
            (top, vec![(x0, y0), (x1, y0), (ix1, iy0), (ix0, iy0)]),
            (right, vec![(x1, y0), (x1, y1), (ix1, iy1), (ix1, iy0)]),
            (bottom, vec![(x1, y1), (x0, y1), (ix0, iy1), (ix1, iy1)]),
            (left, vec![(x0, y1), (x0, y0), (ix0, iy0), (ix0, iy1)]),
        ];
        for (color, points) in polygons {
            if let Some(color) = color {
                self.scene.items.push(Item::Polygon { points, color });
            }
        }
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
