//! Blockformatierungskontexte: Breiten und Höhen nach CSS 2, Abschnitte
//! 10.3 bis 10.7, mit den Schlüsselwörtern aus CSS Sizing 3, und das
//! Zusammenfallen von Außenabständen nach Abschnitt 8.3.1.
//!
//! - Zusammenfallende Außenabstände werden wie in Servo als größter
//!   positiver und kleinster negativer Wert gesammelt. Der untere Abstand
//!   eines Blocks fällt mit dem seines letzten Kindes nur zusammen, wenn
//!   `height` `auto` ist, unten Rand und Innenabstand fehlen und
//!   `min-height` und `max-height` die Höhe nicht ändern; ändern sie sie,
//!   geht der Abstand des Kindes verloren (so die WPT; CSS 2 nennt die
//!   Grenzen hier nicht). Die Außenabstände des Wurzelelements fallen nicht
//!   zusammen.
//! - Prozente bei Höhen gelten nur, wenn die Höhe des umgebenden Blocks
//!   feststeht (der anfängliche umgebende Block hat die des Viewports);
//!   sonst sind sie `auto`, bei `min-height` null und bei `max-height`
//!   `none`. Eine Höhe steht fest, wenn `height` eine Länge ist oder ein
//!   Prozentwert einer festen Höhe, nach `min-height` und `max-height`.
//! - Die Breite eines Inline-Blocks mit `width: auto` passt sich dem Inhalt
//!   an (*shrink-to-fit*): min(max(kleinste Breite, verfügbare Breite),
//!   bevorzugte Breite). Für die Größen aus dem Inhalt sind Prozente, die
//!   sich auf diese Größe beziehen, bei Breiten `auto` und bei Abständen
//!   null (CSS Sizing 3, Abschnitt 5.2.1).
//! - Die Grundlinie eines Inline-Blocks ist die seiner letzten Zeile im
//!   Fluss, sonst die untere Kante seines Außenabstands; die eines ersetzten
//!   Elements immer die untere Kante.
//! - Ein ersetztes Element ohne Bild hat die Größe null. Das ist eine
//!   Festlegung dieses Browsers: Wie HTML ein Bild darstellt, das fehlt
//!   (Ersatztext, Symbol), fehlt noch.

use std::sync::Arc;

use mb_scene::Image;
use mb_style::properties::Longhand;
use mb_style::values::LengthPercentage;

use crate::LayoutContext;
use crate::boxes::{Atomic, BlockBox, BlockContents, BlockLevel, ReplacedBox, StyleRef};
use crate::fragment::{BoxFragment, Fragment, ImageFragment, Rect};
use crate::inline::{self, Sizing};
use crate::style::{Side, Sides, SizeValue, Style, resolve, resolve_definite};

/// Der umgebende Block (*containing block*) eines Kastens im Fluss: der
/// Inhaltsbereich seines Blockcontainers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ContainingBlock {
    pub(crate) width: f64,
    /// Die Höhe, wenn sie feststeht.
    pub(crate) height: Option<f64>,
    pub(crate) ltr: bool,
}

/// Außenabstände, die zusammenfallen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CollapsedMargin {
    max_positive: f64,
    min_negative: f64,
}

impl CollapsedMargin {
    pub(crate) fn new(margin: f64) -> CollapsedMargin {
        CollapsedMargin {
            max_positive: margin.max(0.0),
            min_negative: margin.min(0.0),
        }
    }

    pub(crate) fn adjoin(&self, other: &CollapsedMargin) -> CollapsedMargin {
        CollapsedMargin {
            max_positive: self.max_positive.max(other.max_positive),
            min_negative: self.min_negative.min(other.min_negative),
        }
    }

    /// Der Abstand, der bleibt.
    pub(crate) fn solve(&self) -> f64 {
        self.max_positive + self.min_negative
    }
}

/// Die Außenabstände oben und unten eines Kastens, mit denen seiner Kinder,
/// soweit sie zusammenfallen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CollapsedBlockMargins {
    /// Ob oberer und unterer Abstand selbst zusammenfallen
    /// (*collapse through*).
    pub(crate) collapsed_through: bool,
    pub(crate) start: CollapsedMargin,
    pub(crate) end: CollapsedMargin,
}

impl CollapsedBlockMargins {
    fn from_margin(margin: &Sides) -> CollapsedBlockMargins {
        CollapsedBlockMargins {
            collapsed_through: false,
            start: CollapsedMargin::new(margin.top),
            end: CollapsedMargin::new(margin.bottom),
        }
    }
}

/// Ein blockartiger Kasten nach dem Layout, noch ohne senkrechte Stelle.
#[derive(Debug)]
pub(crate) struct BlockLayout {
    /// Waagrecht schon an seiner Stelle, senkrecht bei null.
    pub(crate) fragment: BoxFragment,
    pub(crate) margins: CollapsedBlockMargins,
    /// Die erste und die letzte Grundlinie, vom oberen Rand des Rahmens aus.
    pub(crate) first_baseline: Option<f64>,
    pub(crate) last_baseline: Option<f64>,
}

/// Ein Kasten auf Atomebene nach dem Layout.
#[derive(Debug)]
pub(crate) struct AtomicLayout {
    /// Relativ zur oberen linken Ecke des Außenabstands.
    fragment: BoxFragment,
    margin: Sides,
    /// Die Grundlinie, vom oberen Rand des Außenabstands aus.
    pub(crate) baseline: f64,
}

impl AtomicLayout {
    pub(crate) fn margin_width(&self) -> f64 {
        self.fragment.rect.width + self.margin.horizontal()
    }

    pub(crate) fn margin_height(&self) -> f64 {
        self.fragment.rect.height + self.margin.vertical()
    }

    /// Das Fragment mit der oberen linken Ecke des Außenabstands bei
    /// (`x`, `y`).
    pub(crate) fn place(&self, x: f64, y: f64) -> Fragment {
        let mut fragment = self.fragment.clone();
        fragment.rect = fragment.rect.translate(x, y);
        Fragment::Box(fragment)
    }
}

/// Welche Größe aus dem Inhalt (*intrinsic size*).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Intrinsic {
    Min,
    Max,
}

impl Intrinsic {
    fn sizing(self) -> Sizing {
        match self {
            Intrinsic::Min => Sizing::MinContent,
            Intrinsic::Max => Sizing::MaxContent,
        }
    }
}

/// Eine Größe des Inhaltsbereichs aus `box-sizing`.
fn content_size(value: f64, padding_border: f64, border_box: bool) -> f64 {
    if border_box {
        (value - padding_border).max(0.0)
    } else {
        value
    }
}

/// Die Größe des Inhalts eines Blockcontainers in der Breite, beim ersten
/// Mal gemessen und dann gemerkt.
pub(crate) fn intrinsic_width(
    context: &LayoutContext<'_>,
    block: &BlockBox,
    which: Intrinsic,
    depth: usize,
) -> f64 {
    let key = (std::ptr::from_ref(block).addr(), which);
    if let Some(&width) = context.intrinsic.borrow().get(&key) {
        return width;
    }
    if !context.budget.spend(1) {
        return 0.0;
    }
    let width = measure_intrinsic_width(context, block, which, depth);
    // Was bei verbrauchtem Budget herauskam, stimmt nicht.
    if context.budget.exhausted().is_none() {
        context.intrinsic.borrow_mut().insert(key, width);
    }
    width
}

fn measure_intrinsic_width(
    context: &LayoutContext<'_>,
    block: &BlockBox,
    which: Intrinsic,
    depth: usize,
) -> f64 {
    match &block.contents {
        BlockContents::Inline(content) => {
            let container = inline::Container {
                style: &block.style,
                width: 0.0,
                height: None,
                first_formatted_line: block.first_formatted_line,
            };
            inline::layout(context, content, container, which.sizing(), depth).width
        }
        BlockContents::Blocks(children) => children
            .iter()
            .map(|child| contribution(context, child, which, depth.saturating_add(1)))
            .fold(0.0, f64::max),
    }
}

/// Der Beitrag eines blockartigen Kastens zur Größe seines Containers aus
/// dem Inhalt, mit Außenabständen.
fn contribution(
    context: &LayoutContext<'_>,
    child: &BlockLevel,
    which: Intrinsic,
    depth: usize,
) -> f64 {
    match child {
        BlockLevel::Block(block) => {
            let style = &block.style;
            let margin = style.margins_or_zero(0.0);
            content_contribution(context, block, which, depth)
                + style.padding(0.0).horizontal()
                + style.border().horizontal()
                + margin.horizontal()
        }
        BlockLevel::Replaced(replaced) => replaced_contribution(replaced, which),
    }
}

/// Die Breite des Inhaltsbereichs, die ein Kasten aus seinem Inhalt
/// beiträgt, nach `width`, `min-width` und `max-width`.
fn content_contribution(
    context: &LayoutContext<'_>,
    block: &BlockBox,
    which: Intrinsic,
    depth: usize,
) -> f64 {
    let style = &block.style;
    let padding_border = style.padding(0.0).horizontal() + style.border().horizontal();
    let border_box = style.border_box_sizing();
    let value = |size: SizeValue| match size {
        SizeValue::LengthPercentage(LengthPercentage::Length(length)) => {
            Some(content_size(length.value, padding_border, border_box))
        }
        SizeValue::MinContent => Some(intrinsic_width(context, block, Intrinsic::Min, depth)),
        SizeValue::MaxContent => Some(intrinsic_width(context, block, Intrinsic::Max, depth)),
        SizeValue::FitContent => Some(intrinsic_width(context, block, which, depth)),
        // Ein Prozentwert bezieht sich auf die Größe, die erst entsteht.
        SizeValue::LengthPercentage(LengthPercentage::Percentage(_))
        | SizeValue::Auto
        | SizeValue::Stretch => None,
    };
    let width = value(style.size(Longhand::Width))
        .unwrap_or_else(|| intrinsic_width(context, block, which, depth));
    let max = style
        .max_size(Longhand::MaxWidth)
        .and_then(value)
        .unwrap_or(f64::INFINITY);
    let min = value(style.size(Longhand::MinWidth)).unwrap_or(0.0);
    width.min(max).max(min)
}

/// Der Beitrag eines ersetzten Elements mit Außenabständen.
fn replaced_contribution(replaced: &ReplacedBox, which: Intrinsic) -> f64 {
    let style = &replaced.style;
    // Für die kleinste Breite beziehen sich Prozente auf null, für die
    // bevorzugte gelten sie als `auto` (CSS Sizing 3, Abschnitt 5.2.1).
    let basis = match which {
        Intrinsic::Min => Some(0.0),
        Intrinsic::Max => None,
    };
    let padding = style.padding(0.0);
    let border = style.border();
    let (width, _) = replaced_size(
        style,
        replaced.image.as_deref(),
        basis,
        None,
        padding.horizontal() + border.horizontal(),
        padding.vertical() + border.vertical(),
    );
    width + padding.horizontal() + border.horizontal() + style.margins_or_zero(0.0).horizontal()
}

/// Die Breite, die sich dem Inhalt anpasst.
fn shrink_to_fit(
    context: &LayoutContext<'_>,
    block: &BlockBox,
    available: f64,
    depth: usize,
) -> f64 {
    let min = intrinsic_width(context, block, Intrinsic::Min, depth);
    let max = intrinsic_width(context, block, Intrinsic::Max, depth);
    min.max(available).min(max)
}

/// `width`, `min-width` oder `max-width` eines nicht ersetzten Kastens als
/// Breite des Inhaltsbereichs; `None` für `auto` und `stretch`.
fn width_value(
    context: &LayoutContext<'_>,
    block: &BlockBox,
    size: SizeValue,
    basis: f64,
    padding_border: f64,
    available: f64,
    depth: usize,
) -> Option<f64> {
    let border_box = block.style.border_box_sizing();
    match size {
        SizeValue::LengthPercentage(value) => Some(content_size(
            resolve(value, basis),
            padding_border,
            border_box,
        )),
        SizeValue::MinContent => Some(intrinsic_width(context, block, Intrinsic::Min, depth)),
        SizeValue::MaxContent => Some(intrinsic_width(context, block, Intrinsic::Max, depth)),
        SizeValue::FitContent => Some(shrink_to_fit(context, block, available, depth)),
        SizeValue::Auto | SizeValue::Stretch => None,
    }
}

/// `height`, `min-height` oder `max-height` als Höhe des Inhaltsbereichs;
/// `None` für `auto`, die Schlüsselwörter und Prozente ohne feste Höhe des
/// umgebenden Blocks. Die Schlüsselwörter aus Sizing 3 verhalten sich bei
/// Höhen im Blocklayout wie `auto`.
fn height_value(
    style: &StyleRef,
    size: SizeValue,
    basis: Option<f64>,
    padding_border: f64,
) -> Option<f64> {
    match size {
        SizeValue::LengthPercentage(value) => resolve_definite(value, basis)
            .map(|height| content_size(height, padding_border, style.border_box_sizing())),
        _ => None,
    }
}

/// Die Grenzen der Höhe: `min-height` und `max-height`.
fn height_limits(style: &StyleRef, basis: Option<f64>, padding_border: f64) -> (f64, f64) {
    let min = height_value(
        style,
        style.size(Longhand::MinHeight),
        basis,
        padding_border,
    )
    .unwrap_or(0.0);
    let max = style
        .max_size(Longhand::MaxHeight)
        .and_then(|size| height_value(style, size, basis, padding_border))
        .unwrap_or(f64::INFINITY);
    (min, max.max(min))
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.min(max).max(min)
}

/// Die Außenabstände links und rechts nach CSS 2, Abschnitt 10.3.3, für
/// eine Breite des Inhaltsbereichs.
fn horizontal_margins(
    style: &StyleRef,
    cb: &ContainingBlock,
    width: f64,
    padding_border: f64,
) -> (f64, f64) {
    let resolve_margin = |side| match style.margin(side) {
        mb_style::properties::LengthPercentageOrAuto::Auto => None,
        mb_style::properties::LengthPercentageOrAuto::LengthPercentage(value) => {
            Some(resolve(value, cb.width))
        }
    };
    let mut left = resolve_margin(Side::Left);
    let mut right = resolve_margin(Side::Right);
    let fixed = width + padding_border + left.unwrap_or(0.0) + right.unwrap_or(0.0);
    // Ist der Kasten breiter als der Block, gelten `auto`-Abstände als null.
    if fixed > cb.width {
        left = left.or(Some(0.0));
        right = right.or(Some(0.0));
    }
    let free = cb.width - width - padding_border;
    match (left, right) {
        (None, None) => (free / 2.0, free / 2.0),
        (None, Some(right)) => (free - right, right),
        (Some(left), None) => (left, free - left),
        // Überbestimmt: Der Abstand am Ende weicht, nach der Richtung des
        // umgebenden Blocks.
        (Some(left), Some(right)) => {
            if cb.ltr {
                (left, free - left)
            } else {
                (free - right, right)
            }
        }
    }
}

/// Die Breite des Inhaltsbereichs eines Blocks im Fluss nach CSS 2,
/// Abschnitte 10.3.3 und 10.4.
fn block_width(
    context: &LayoutContext<'_>,
    block: &BlockBox,
    cb: &ContainingBlock,
    padding_border: f64,
    depth: usize,
) -> f64 {
    let style = &block.style;
    let margins = style.margins_or_zero(cb.width);
    let available = (cb.width - margins.horizontal() - padding_border).max(0.0);
    let value = |size| {
        width_value(
            context,
            block,
            size,
            cb.width,
            padding_border,
            available,
            depth,
        )
    };
    let tentative = value(style.size(Longhand::Width)).unwrap_or(available);
    let max = style
        .max_size(Longhand::MaxWidth)
        .and_then(value)
        .unwrap_or(f64::INFINITY);
    let min = value(style.size(Longhand::MinWidth)).unwrap_or(0.0);
    clamp(tentative, min, max.max(min))
}

/// Lays out a block-level box in normal flow.
pub(crate) fn layout_block_level(
    context: &LayoutContext<'_>,
    child: &BlockLevel,
    cb: &ContainingBlock,
    depth: usize,
) -> BlockLayout {
    match child {
        BlockLevel::Block(block) => layout_block(context, block, cb, depth),
        BlockLevel::Replaced(replaced) => layout_replaced_block(replaced, cb),
    }
}

/// Ein Block im Fluss: Breite, Außenabstände links und rechts, Inhalt.
pub(crate) fn layout_block(
    context: &LayoutContext<'_>,
    block: &BlockBox,
    cb: &ContainingBlock,
    depth: usize,
) -> BlockLayout {
    let style = &block.style;
    let padding = style.padding(cb.width);
    let border = style.border();
    let padding_border = padding.horizontal() + border.horizontal();
    let width = block_width(context, block, cb, padding_border, depth);
    let (left, right) = horizontal_margins(style, cb, width, padding_border);
    let mut margin = style.margins_or_zero(cb.width);
    margin.left = left;
    margin.right = right;
    layout_block_contents(context, block, cb, width, margin, depth)
}

/// Höhe und Inhalt eines Blockcontainers mit fester Breite.
fn layout_block_contents(
    context: &LayoutContext<'_>,
    block: &BlockBox,
    cb: &ContainingBlock,
    width: f64,
    margin: Sides,
    depth: usize,
) -> BlockLayout {
    let style = &block.style;
    let padding = style.padding(cb.width);
    let border = style.border();
    let padding_border_vertical = padding.vertical() + border.vertical();
    let height = height_value(
        style,
        style.size(Longhand::Height),
        cb.height,
        padding_border_vertical,
    );
    let (min_height, max_height) = height_limits(style, cb.height, padding_border_vertical);
    let children_cb = ContainingBlock {
        width,
        height: height.map(|height| clamp(height, min_height, max_height)),
        ltr: style.ltr(),
    };
    let independent = block.independent;
    let start_collapses = !independent && border.top == 0.0 && padding.top == 0.0;
    let end_collapses =
        !independent && border.bottom == 0.0 && padding.bottom == 0.0 && height.is_none();

    // Ist das Budget verbraucht, steigt das Layout nicht weiter ab.
    let flow = match &block.contents {
        _ if !context.budget.spend(1) => Flow {
            fragments: Vec::new(),
            lines: None,
            height: 0.0,
            margins: CollapsedBlockMargins::default(),
            first_baseline: None,
            last_baseline: None,
        },
        BlockContents::Blocks(children) => {
            layout_children(context, children, &children_cb, start_collapses, depth)
        }
        BlockContents::Inline(content) => {
            let container = inline::Container {
                style,
                width,
                height: children_cb.height,
                first_formatted_line: block.first_formatted_line,
            };
            let inline =
                inline::layout(context, content, container, Sizing::Available(width), depth);
            Flow {
                fragments: inline.fragments,
                lines: inline.lines,
                height: inline.height,
                margins: CollapsedBlockMargins {
                    collapsed_through: !inline.has_lines,
                    ..CollapsedBlockMargins::default()
                },
                first_baseline: inline.first_baseline,
                last_baseline: inline.last_baseline,
            }
        }
    };
    let mut children_margins = flow.margins;
    let mut margins = CollapsedBlockMargins::from_margin(&margin);
    if start_collapses {
        margins.start = margins.start.adjoin(&children_margins.start);
        if children_margins.collapsed_through {
            margins.start = margins.start.adjoin(&children_margins.end);
            children_margins.end = CollapsedMargin::default();
        }
    }
    // Ändern `min-height` oder `max-height` die Höhe, fällt der untere
    // Abstand des letzten Kindes nicht mit dem des Blocks zusammen und zählt
    // auch nicht zur Höhe.
    let collapsed = flow.height;
    let used_height = match height {
        Some(height) => clamp(height, min_height, max_height),
        None if end_collapses => {
            let clamped = clamp(collapsed, min_height, max_height);
            if clamped == collapsed {
                margins.end = margins.end.adjoin(&children_margins.end);
            }
            clamped
        }
        None => clamp(
            flow.height + children_margins.end.solve(),
            min_height,
            max_height,
        ),
    };
    margins.collapsed_through = children_margins.collapsed_through
        && !independent
        && padding_border_vertical == 0.0
        && height.is_none_or(|height| height == 0.0)
        && min_height == 0.0;
    if margins.collapsed_through {
        // Oben und unten fallen zusammen, auch mit den Kindern.
        margins.start = margins.start.adjoin(&margins.end);
        margins.end = margins.start;
    }
    let content_top = border.top + padding.top;
    let fragment = BoxFragment {
        style: style.clone(),
        node: block.node,
        rect: Rect::new(
            margin.left,
            0.0,
            width + padding.horizontal() + border.horizontal(),
            used_height + padding_border_vertical,
        ),
        border,
        padding,
        children: flow.fragments,
        lines: flow.lines,
        atomic: false,
    };
    BlockLayout {
        fragment,
        margins,
        first_baseline: flow.first_baseline.map(|baseline| baseline + content_top),
        last_baseline: flow.last_baseline.map(|baseline| baseline + content_top),
    }
}

/// Die Kinder eines Blockcontainers nach dem Layout.
struct Flow {
    fragments: Vec<Fragment>,
    /// Die Zeilen eines Inline-Formatierungskontexts.
    lines: Option<Rect>,
    /// Bis zur unteren Kante des letzten Kindes, ohne dessen unteren
    /// Außenabstand.
    height: f64,
    margins: CollapsedBlockMargins,
    first_baseline: Option<f64>,
    last_baseline: Option<f64>,
}

/// Setzt blockartige Kinder untereinander, mit zusammenfallenden
/// Außenabständen. `collapses_with_start` sagt, ob der obere Abstand des
/// ersten Kindes mit dem des Containers zusammenfällt.
fn layout_children(
    context: &LayoutContext<'_>,
    children: &[BlockLevel],
    cb: &ContainingBlock,
    collapses_with_start: bool,
    depth: usize,
) -> Flow {
    let mut fragments = Vec::with_capacity(children.len());
    let mut with_start = collapses_with_start;
    let mut start_margin = CollapsedMargin::default();
    let mut current_margin = CollapsedMargin::default();
    let mut position = 0.0;
    let mut first_baseline = None;
    let mut last_baseline = None;
    for child in children {
        if !context.budget.spend(1) {
            break;
        }
        let layout = layout_block_level(context, child, cb, depth.saturating_add(1));
        let BlockLayout {
            mut fragment,
            margins,
            first_baseline: child_first,
            last_baseline: child_last,
        } = layout;
        if with_start {
            start_margin = start_margin.adjoin(&margins.start);
            if margins.collapsed_through {
                // Die obere Kante liegt dann bei der des Containers.
                start_margin = start_margin.adjoin(&margins.end);
                fragment.rect.y = position;
                fragments.push(Fragment::Box(fragment));
                continue;
            }
            with_start = false;
        } else {
            current_margin = current_margin.adjoin(&margins.start);
        }
        // Die obere Kante liegt, wo sie mit einem unteren Rand läge.
        fragment.rect.y = position + current_margin.solve();
        if margins.collapsed_through {
            current_margin = current_margin.adjoin(&margins.end);
            fragments.push(Fragment::Box(fragment));
            continue;
        }
        let top = fragment.rect.y;
        if first_baseline.is_none() {
            first_baseline = child_first.map(|baseline| baseline + top);
        }
        if let Some(baseline) = child_last {
            last_baseline = Some(baseline + top);
        }
        position = top + fragment.rect.height;
        current_margin = margins.end;
        fragments.push(Fragment::Box(fragment));
    }
    Flow {
        fragments,
        lines: None,
        height: position,
        margins: CollapsedBlockMargins {
            collapsed_through: with_start,
            start: start_margin,
            end: current_margin,
        },
        first_baseline,
        last_baseline,
    }
}

/// Breite und Höhe des Inhaltsbereichs eines ersetzten Elements nach
/// CSS 2, Abschnitte 10.3.2, 10.4, 10.6.2 und 10.7. Ohne Bezugsgröße
/// gelten Prozente als `auto`.
fn replaced_size(
    style: &StyleRef,
    image: Option<&Image>,
    basis_width: Option<f64>,
    basis_height: Option<f64>,
    padding_border_horizontal: f64,
    padding_border_vertical: f64,
) -> (f64, f64) {
    let border_box = style.border_box_sizing();
    // Die Schlüsselwörter aus Sizing 3 ergeben bei ersetzten Elementen die
    // Größe, die auch `auto` ergäbe.
    let value = |size: SizeValue, basis: Option<f64>, padding_border: f64| match size {
        SizeValue::LengthPercentage(value) => resolve_definite(value, basis)
            .map(|value| content_size(value, padding_border, border_box)),
        _ => None,
    };
    let width = value(
        style.size(Longhand::Width),
        basis_width,
        padding_border_horizontal,
    );
    let height = value(
        style.size(Longhand::Height),
        basis_height,
        padding_border_vertical,
    );
    let min_width = value(
        style.size(Longhand::MinWidth),
        basis_width,
        padding_border_horizontal,
    )
    .unwrap_or(0.0);
    let max_width = style
        .max_size(Longhand::MaxWidth)
        .and_then(|size| value(size, basis_width, padding_border_horizontal))
        .unwrap_or(f64::INFINITY)
        .max(min_width);
    let min_height = value(
        style.size(Longhand::MinHeight),
        basis_height,
        padding_border_vertical,
    )
    .unwrap_or(0.0);
    let max_height = style
        .max_size(Longhand::MaxHeight)
        .and_then(|size| value(size, basis_height, padding_border_vertical))
        .unwrap_or(f64::INFINITY)
        .max(min_height);
    let natural = image.map_or((0.0, 0.0), |image| {
        (f64::from(image.width()), f64::from(image.height()))
    });
    let ratio = (natural.0 > 0.0 && natural.1 > 0.0).then(|| natural.0 / natural.1);
    match (width, height) {
        (None, None) => match ratio {
            Some(_) => constrain(
                natural.0, natural.1, min_width, max_width, min_height, max_height,
            ),
            None => (
                clamp(natural.0, min_width, max_width),
                clamp(natural.1, min_height, max_height),
            ),
        },
        (Some(width), None) => {
            let width = clamp(width, min_width, max_width);
            let height = ratio.map_or(natural.1, |ratio| width / ratio);
            (width, clamp(height, min_height, max_height))
        }
        (None, Some(height)) => {
            let height = clamp(height, min_height, max_height);
            let width = ratio.map_or(natural.0, |ratio| height * ratio);
            (clamp(width, min_width, max_width), height)
        }
        (Some(width), Some(height)) => (
            clamp(width, min_width, max_width),
            clamp(height, min_height, max_height),
        ),
    }
}

/// Die Tabelle aus CSS 2, Abschnitt 10.4, für ersetzte Elemente mit
/// Seitenverhältnis, bei denen `width` und `height` `auto` sind. `w` und
/// `h` sind größer als null.
fn constrain(w: f64, h: f64, min_w: f64, max_w: f64, min_h: f64, max_h: f64) -> (f64, f64) {
    let over_w = w > max_w;
    let under_w = w < min_w;
    let over_h = h > max_h;
    let under_h = h < min_h;
    match (over_w, under_w, over_h, under_h) {
        (true, _, true, _) if max_w / w <= max_h / h => (max_w, min_h.max(max_w * h / w)),
        (true, _, true, _) => (min_w.max(max_h * w / h), max_h),
        (_, true, _, true) if min_w / w <= min_h / h => (max_w.min(min_h * w / h), min_h),
        (_, true, _, true) => (min_w, max_h.min(min_w * h / w)),
        (_, true, true, _) => (min_w, max_h),
        (true, _, _, true) => (max_w, min_h),
        (true, _, _, _) => (max_w, (max_w * h / w).max(min_h)),
        (_, true, _, _) => (min_w, (min_w * h / w).min(max_h)),
        (_, _, true, _) => ((max_h * w / h).max(min_w), max_h),
        (_, _, _, true) => ((min_h * w / h).min(max_w), min_h),
        _ => (w, h),
    }
}

/// Das Fragment eines ersetzten Elements mit dem Bild im Inhaltsbereich.
fn replaced_fragment(
    replaced: &ReplacedBox,
    width: f64,
    height: f64,
    padding: Sides,
    border: Sides,
) -> BoxFragment {
    let children = match &replaced.image {
        Some(image) => vec![Fragment::Image(ImageFragment {
            style: replaced.style.clone(),
            rect: Rect::new(0.0, 0.0, width, height),
            image: Arc::clone(image),
        })],
        None => Vec::new(),
    };
    BoxFragment {
        style: replaced.style.clone(),
        node: Some(replaced.node),
        rect: Rect::new(
            0.0,
            0.0,
            width + padding.horizontal() + border.horizontal(),
            height + padding.vertical() + border.vertical(),
        ),
        border,
        padding,
        children,
        lines: None,
        atomic: false,
    }
}

/// Ein blockartiges ersetztes Element (CSS 2, Abschnitt 10.3.4).
fn layout_replaced_block(replaced: &ReplacedBox, cb: &ContainingBlock) -> BlockLayout {
    let style = &replaced.style;
    let padding = style.padding(cb.width);
    let border = style.border();
    let (width, height) = replaced_size(
        style,
        replaced.image.as_deref(),
        Some(cb.width),
        cb.height,
        padding.horizontal() + border.horizontal(),
        padding.vertical() + border.vertical(),
    );
    let (left, right) =
        horizontal_margins(style, cb, width, padding.horizontal() + border.horizontal());
    let mut margin = style.margins_or_zero(cb.width);
    margin.left = left;
    margin.right = right;
    let mut fragment = replaced_fragment(replaced, width, height, padding, border);
    fragment.rect.x = margin.left;
    BlockLayout {
        fragment,
        margins: CollapsedBlockMargins::from_margin(&margin),
        first_baseline: None,
        last_baseline: None,
    }
}

/// Lays out an atomic inline-level box. `containing_width` is the width of
/// the block container; for the intrinsic sizes only the width counts.
pub(crate) fn layout_atomic(
    context: &LayoutContext<'_>,
    atomic: &Atomic,
    containing_width: f64,
    containing_height: Option<f64>,
    sizing: Sizing,
    depth: usize,
) -> AtomicLayout {
    let depth = depth.saturating_add(1);
    let style = atomic.style();
    let intrinsic = match sizing {
        Sizing::Available(_) => None,
        Sizing::MinContent => Some(Intrinsic::Min),
        Sizing::MaxContent => Some(Intrinsic::Max),
    };
    let basis = if intrinsic.is_some() {
        0.0
    } else {
        containing_width
    };
    // Bei Kästen auf Atomebene ist `auto` als Außenabstand null.
    let margin = style.margins_or_zero(basis);
    let padding = style.padding(basis);
    let border = style.border();
    let padding_border = padding.horizontal() + border.horizontal();
    if let Some(which) = intrinsic {
        let width = match atomic {
            Atomic::Block(block) => content_contribution(context, block, which, depth),
            Atomic::Replaced(replaced) => {
                replaced_contribution(replaced, which) - padding_border - margin.horizontal()
            }
        };
        return AtomicLayout {
            fragment: BoxFragment {
                style: style.clone(),
                node: None,
                rect: Rect::new(margin.left, margin.top, width + padding_border, 0.0),
                border,
                padding,
                children: Vec::new(),
                lines: None,
                atomic: true,
            },
            margin,
            baseline: 0.0,
        };
    }
    let cb = ContainingBlock {
        width: containing_width,
        height: containing_height,
        ltr: style.ltr(),
    };
    let (mut fragment, baseline) = match atomic {
        Atomic::Block(block) => {
            let available = (containing_width - margin.horizontal() - padding_border).max(0.0);
            let value = |size| {
                width_value(
                    context,
                    block,
                    size,
                    containing_width,
                    padding_border,
                    available,
                    depth,
                )
            };
            let tentative = value(style.size(Longhand::Width))
                .unwrap_or_else(|| shrink_to_fit(context, block, available, depth));
            let max = style
                .max_size(Longhand::MaxWidth)
                .and_then(value)
                .unwrap_or(f64::INFINITY);
            let min = value(style.size(Longhand::MinWidth)).unwrap_or(0.0);
            let width = clamp(tentative, min, max.max(min));
            let layout = layout_block_contents(context, block, &cb, width, margin, depth);
            let height = layout.fragment.rect.height;
            let baseline = match layout.last_baseline {
                Some(baseline) => margin.top + baseline,
                None => margin.top + height + margin.bottom,
            };
            (layout.fragment, baseline)
        }
        Atomic::Replaced(replaced) => {
            let (width, height) = replaced_size(
                style,
                replaced.image.as_deref(),
                Some(containing_width),
                containing_height,
                padding_border,
                padding.vertical() + border.vertical(),
            );
            let fragment = replaced_fragment(replaced, width, height, padding, border);
            let baseline = margin.top + fragment.rect.height + margin.bottom;
            (fragment, baseline)
        }
    };
    fragment.rect.x = margin.left;
    fragment.rect.y = margin.top;
    fragment.atomic = true;
    AtomicLayout {
        fragment,
        margin,
        baseline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margins_collapse() {
        let a = CollapsedMargin::new(10.0);
        let b = CollapsedMargin::new(-4.0);
        let c = CollapsedMargin::new(20.0);
        assert_eq!(a.adjoin(&b).solve(), 6.0);
        assert_eq!(a.adjoin(&b).adjoin(&c).solve(), 16.0);
        assert_eq!(CollapsedMargin::new(-3.0).adjoin(&b).solve(), -4.0);
    }

    #[test]
    fn replaced_constraints() {
        // 100 × 50 mit max-width 60: Die Höhe folgt dem Verhältnis.
        assert_eq!(
            constrain(100.0, 50.0, 0.0, 60.0, 0.0, f64::INFINITY),
            (60.0, 30.0)
        );
        // Mit min-height 40 gewinnt die Grenze.
        assert_eq!(
            constrain(100.0, 50.0, 0.0, 60.0, 40.0, f64::INFINITY),
            (60.0, 40.0)
        );
        // Beide zu groß: das stärker beschränkte Maß zählt.
        assert_eq!(constrain(100.0, 50.0, 0.0, 50.0, 0.0, 10.0), (20.0, 10.0));
        // Beide zu klein.
        assert_eq!(
            constrain(10.0, 10.0, 20.0, f64::INFINITY, 40.0, f64::INFINITY),
            (40.0, 40.0)
        );
        assert_eq!(
            constrain(10.0, 10.0, 0.0, 5.0, 20.0, f64::INFINITY),
            (5.0, 20.0)
        );
        assert_eq!(
            constrain(10.0, 10.0, 20.0, f64::INFINITY, 0.0, 5.0),
            (20.0, 5.0)
        );
    }
}
