//! Berechnete Werte (*computed values*, CSS Cascade 5) und die Werte, die
//! `getComputedStyle()` liefert (*resolved values*, CSSOM).
//!
//! Aus dem kaskadierten Wert jeder Langform wird zuerst der angegebene
//! (*specified value*): `inherit` nimmt den berechneten Wert beim
//! Elternelement, `initial` den Anfangswert, `unset` und ein fehlender
//! kaskadierter Wert das eine oder das andere, je nachdem, ob die
//! Eigenschaft vererbt wird. Daraus rechnet [`ComputedValues::compute`] den
//! berechneten Wert aus, wie ihn die Zeile „Computed value“ der Definition
//! beschreibt.
//!
//! Berechnete Werte stehen in denselben Typen wie deklarierte
//! ([`SpecifiedValue`]), aber nur in der Form, die die Definition als
//! berechneten Wert zulässt: Längen absolut in `px`, Farben als sRGB mit
//! 8 Bit, als Farbfunktion oder `currentcolor`, `font-size` als Länge,
//! `font-weight` als Zahl, `font-width` als Prozentwert, Winkel in Grad und
//! URLs aufgelöst. So lassen sie sich wie deklarierte Werte serialisieren
//! und wieder setzen, ohne eine zweite Familie von Werttypen, die mit der
//! ersten Schritt halten müsste.
//!
//! Die Reihenfolge: zuerst `writing-mode`, `direction` und
//! `text-orientation`, dann die Schrift, dann `line-height`, dann `color`,
//! `display` und alles Übrige. Schriftbezogene Einheiten in `font-size`
//! beziehen sich auf die Schrift des Elternelements, `lh` in `font-size` und
//! `line-height` auf dessen Zeilenhöhe; beim Wurzelelement treten die
//! Anfangswerte an die Stelle des Elternelements, ebenso für `rem` in
//! `font-size` und `rlh` in `line-height` (Values 4). Wo eine Schrift ein
//! Maß nicht hat, gelten die Ersatzwerte aus Values 4; für `cap` ohne
//! Oberlänge und für `lh` bei `line-height: normal` gibt es keinen, dann
//! scheitert die Berechnung mit [`MissingFontMetric`].
//!
//! Was der Benutzeragent festlegt: `medium` ist 16px, der Faktor für
//! `larger` und `smaller` 1,2 (Fonts 4 nennt 1,2 bis 1,5), die Familie am
//! Anfang `serif` und `hairline` ein Gerätepixel (Borders 4: höchstens 1px
//! und eine ganze Zahl von Gerätepixeln). Systemfarben liefert das
//! [`Device`].
//!
//! Es fehlen die Eigenschaften, die es noch nicht gibt, mit ihren
//! Wirkungen: `position` und `float` blockifizieren nicht, `font-size: math`
//! ist der geerbte Wert (ohne `math-depth` ist der Faktor aus MathML Core
//! immer 1), und `aspect-ratio` gilt als `auto`.

#[cfg(test)]
mod tests;

use std::sync::OnceLock;

use mb_dom::NodeId;

use crate::cascade::{Cascaded, CascadedValue, CascadedValues, Stylist};
use crate::declarations::Declared;
use crate::properties::{
    AbsoluteSize, BaselineShift, BoxSize, DeclaredValue, Direction, Display, DisplayInside,
    DisplayOutside, FlowMode, FontFamily, FontFamilyList, FontSize, FontStyle, FontVariantCaps,
    FontWeight, FontWidth, FontWidthKeyword, GenericFamily, LengthPercentageOrAuto, LineHeight,
    LineStyle, LineWidth, LineWidthKeyword, ListStyleImage, Longhand, Mapping, MaxSize,
    RelativeSize, Shorthand, Size, Spacing, SpecifiedValue, TextAlignAll, TextAlignKeyword,
    TextAlignLast, TextDecorationThickness, TextIndent, TextOrientation, WritingMode,
    serialize_shorthand,
};
use crate::selectors::MatchingContext;
use crate::values::color::{Color, Rgba, SystemColor};
use crate::values::{
    Angle, AngleUnit, FontMetric, Length, LengthPercentage, LengthUnit, SpecifiedUrl, ViewportAxis,
    clamp,
};

/// `medium`: die Schriftgröße am Anfang in `px`, vom Benutzeragenten
/// gewählt.
pub const MEDIUM_FONT_SIZE: f64 = 16.0;

/// Der Faktor für `larger` und `smaller`.
const RELATIVE_SIZE_RATIO: f64 = 1.2;

/// Die Schlüsselwörter von `font-width` mit ihren Prozentwerten (Fonts 4).
const FONT_WIDTHS: [(FontWidthKeyword, f64); 9] = [
    (FontWidthKeyword::UltraCondensed, 50.0),
    (FontWidthKeyword::ExtraCondensed, 62.5),
    (FontWidthKeyword::Condensed, 75.0),
    (FontWidthKeyword::SemiCondensed, 87.5),
    (FontWidthKeyword::Normal, 100.0),
    (FontWidthKeyword::SemiExpanded, 112.5),
    (FontWidthKeyword::Expanded, 125.0),
    (FontWidthKeyword::ExtraExpanded, 150.0),
    (FontWidthKeyword::UltraExpanded, 200.0),
];

/// Was die Berechnung über das Ausgabegerät wissen muss.
#[derive(Clone, Copy, Debug)]
pub struct Device {
    /// Breite und Höhe des Viewports in `px`. Die kleinen, großen und
    /// dynamischen Viewport-Einheiten (`sv…`, `lv…`, `dv…`) sind gleich,
    /// solange sich der Viewport nicht verändert.
    pub viewport_width: f64,
    pub viewport_height: f64,
    /// Gerätepixel je `px`.
    pub device_pixel_ratio: f64,
    /// Die Farbe einer ⟨system-color⟩. ⟨deprecated-color⟩s sind dann schon
    /// auf ihre Systemfarbe abgebildet.
    pub system_color: fn(SystemColor) -> Rgba,
}

/// Der Vorschub eines Zeichens in `px`, waagrecht und senkrecht gesetzt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Advance {
    pub horizontal: f64,
    pub vertical: f64,
}

/// Die Maße einer Schrift in `px`, soweit sie bekannt sind.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FontMetrics {
    pub x_height: Option<f64>,
    pub cap_height: Option<f64>,
    pub ascent: Option<f64>,
    /// Die Zeilenhöhe für `line-height: normal`.
    pub normal_line_height: Option<f64>,
    /// Der Vorschub von „0“ (U+0030), für `ch`.
    pub zero: Option<Advance>,
    /// Der Vorschub von „水“ (U+6C34), für `ic`.
    pub water: Option<Advance>,
}

/// Woher die Maße der Schriften kommen.
pub trait FontMetricsSource {
    /// Die Maße der ersten verfügbaren Schrift (*first available font*) für
    /// eine berechnete Schrift.
    fn metrics(&self, font: &Font<'_>) -> FontMetrics;
}

/// Ohne Schriften: Alle Maße fehlen.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFonts;

impl FontMetricsSource for NoFonts {
    fn metrics(&self, _: &Font<'_>) -> FontMetrics {
        FontMetrics::default()
    }
}

/// Ein Wert braucht ein Maß der Schrift, für das Values 4 keinen Ersatz
/// nennt: die Versalhöhe einer Schrift ohne Oberlänge (`cap`) oder die
/// Zeilenhöhe `normal` (`lh`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingFontMetric;

/// Ein Wert von `getComputedStyle()`, der vom Layout abhängt
/// (*used value*); den gibt es erst mit dem Layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeedsLayout;

/// Die berechnete Schrift eines Elements.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Font<'a> {
    pub family: &'a FontFamilyList,
    /// In `px`.
    pub size: f64,
    pub weight: f64,
    pub style: FontStyle,
    /// In Prozent.
    pub width: f64,
    pub variant_caps: FontVariantCaps,
}

/// Das Elternelement und das Wurzelelement eines Elements, das nicht selbst
/// die Wurzel ist.
#[derive(Clone, Copy, Debug)]
pub struct Ancestors<'a> {
    pub parent: &'a ComputedValues,
    pub root: &'a ComputedValues,
}

/// Die berechneten Werte aller Langformen eines Elements.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedValues {
    /// Je Langform. Eine logische hat denselben Wert wie die physische, die
    /// ihr zugeordnet ist.
    values: Vec<SpecifiedValue>,
    flow: FlowMode,
    /// Ob die Kinder blockifiziert werden: bei einem Flex- oder
    /// Grid-Container; bei `display: contents` wie beim Elternelement.
    blockifies_children: bool,
    /// Ob das Element in einem Flex- oder Grid-Container liegt.
    flex_or_grid_item: bool,
}

fn slot(longhand: Longhand) -> usize {
    longhand as usize
}

fn is_logical(longhand: Longhand) -> bool {
    longhand
        .info()
        .group
        .is_some_and(|(_, mapping)| mapping == Mapping::Logical)
}

/// `font-family` am Anfang.
fn initial_family() -> &'static FontFamilyList {
    static FAMILY: OnceLock<FontFamilyList> = OnceLock::new();
    FAMILY.get_or_init(|| FontFamilyList(vec![FontFamily::Generic(GenericFamily::Serif)]))
}

/// Der Anfangswert einer Langform als angegebener Wert, einmal gelesen.
fn initial(longhand: Longhand) -> &'static SpecifiedValue {
    static INITIAL: OnceLock<Vec<SpecifiedValue>> = OnceLock::new();
    static FALLBACK: SpecifiedValue = SpecifiedValue::Direction(Direction::Ltr);
    let values = INITIAL.get_or_init(|| {
        Longhand::ALL
            .iter()
            .map(|longhand| match longhand.initial_value() {
                Some(value) => value,
                // Nur `font-family` hat keinen festen Anfangswert; dass sich
                // alle anderen lesen lassen, prüft ein Test.
                None => SpecifiedValue::FontFamily(initial_family().clone()),
            })
            .collect()
    });
    values.get(slot(longhand)).unwrap_or(&FALLBACK)
}

/// Der angegebene Wert einer Langform nach dem Auffüllen (*defaulting*).
#[derive(Clone, Copy)]
enum Specified<'a> {
    /// Ein Wert, der noch berechnet wird.
    Value(&'a SpecifiedValue),
    /// Der berechnete Wert beim Elternelement.
    Inherited(&'a SpecifiedValue),
}

impl<'a> Specified<'a> {
    fn value(self) -> &'a SpecifiedValue {
        match self {
            Specified::Value(value) | Specified::Inherited(value) => value,
        }
    }
}

fn defaulted<'a>(
    cascaded: Option<Cascaded<'a>>,
    parent: Option<&'a ComputedValues>,
    longhand: Longhand,
) -> Specified<'a> {
    let inherited = longhand.info().inherited;
    // Bei einer logischen Langform meint `inherit` den Wert dieser
    // Langform beim Elternelement, nach dessen Zuordnung.
    let inherit = |from: Longhand| match parent {
        Some(parent) => Specified::Inherited(parent.get(from)),
        None => Specified::Value(initial(longhand)),
    };
    match cascaded {
        Some(Cascaded {
            value: CascadedValue::Specified(value),
            ..
        }) => Specified::Value(value),
        Some(Cascaded {
            longhand: from,
            value: CascadedValue::Inherit,
        }) => inherit(from),
        Some(Cascaded {
            longhand: from,
            value: CascadedValue::Unset,
        }) if inherited => inherit(from),
        None if inherited => inherit(longhand),
        Some(Cascaded {
            value: CascadedValue::Initial | CascadedValue::Unset,
            ..
        })
        | None => Specified::Value(initial(longhand)),
    }
}

/// Die Zuordnung der logischen Langformen für [`MatchedDeclarations::cascade`]
/// aus den kaskadierten Werten von `writing-mode`, `direction` und
/// `text-orientation`, mit Vererbung.
///
/// [`MatchedDeclarations::cascade`]: crate::MatchedDeclarations::cascade
pub fn flow_mode(cascaded: &CascadedValues<'_>, parent: Option<&ComputedValues>) -> FlowMode {
    let value = |longhand| defaulted(cascaded.get(longhand), parent, longhand).value();
    let writing_mode = match value(Longhand::WritingMode) {
        SpecifiedValue::WritingMode(mode) => *mode,
        _ => WritingMode::HorizontalTb,
    };
    let direction = match value(Longhand::Direction) {
        SpecifiedValue::Direction(direction) => *direction,
        _ => Direction::Ltr,
    };
    let orientation = match value(Longhand::TextOrientation) {
        SpecifiedValue::TextOrientation(orientation) => *orientation,
        _ => TextOrientation::Mixed,
    };
    FlowMode::new(writing_mode, direction, orientation)
}

/// Die Schrift eines Elements, auf die sich schriftbezogene Einheiten
/// beziehen.
#[derive(Clone, Copy)]
struct FontBasis<'a> {
    font: Font<'a>,
    /// Der berechnete Wert von `line-height`, für `lh`.
    line_height: LineHeight,
    writing_mode: WritingMode,
    text_orientation: TextOrientation,
}

impl FontBasis<'static> {
    /// Die Anfangswerte, für das Wurzelelement.
    fn initial() -> FontBasis<'static> {
        FontBasis {
            font: Font {
                family: initial_family(),
                size: MEDIUM_FONT_SIZE,
                weight: 400.0,
                style: FontStyle::Normal,
                width: 100.0,
                variant_caps: FontVariantCaps::Normal,
            },
            line_height: LineHeight::Normal,
            writing_mode: WritingMode::HorizontalTb,
            text_orientation: TextOrientation::Mixed,
        }
    }
}

impl FontBasis<'_> {
    fn vertical_typography(&self) -> bool {
        matches!(
            self.writing_mode,
            WritingMode::VerticalRl | WritingMode::VerticalLr
        )
    }

    /// Ob „0“ aufrecht gesetzt wird; dann zählt für `ch` der senkrechte
    /// Vorschub (Values 4).
    fn upright_digits(&self) -> bool {
        self.vertical_typography() && self.text_orientation == TextOrientation::Upright
    }

    /// Ob „水“ aufrecht gesetzt wird: im vertikalen *typographic mode*
    /// außer bei `sideways`.
    fn upright_ideographs(&self) -> bool {
        self.vertical_typography() && self.text_orientation != TextOrientation::Sideways
    }

    /// Die berechnete Zeilenhöhe in `px`.
    fn line_height_px(&self, fonts: &dyn FontMetricsSource) -> Result<f64, MissingFontMetric> {
        Ok(match self.line_height {
            LineHeight::Normal => fonts
                .metrics(&self.font)
                .normal_line_height
                .ok_or(MissingFontMetric)?,
            LineHeight::Number(number) => number * self.font.size,
            LineHeight::LengthPercentage(LengthPercentage::Length(length)) => length.value,
            LineHeight::LengthPercentage(LengthPercentage::Percentage(percentage)) => {
                percentage / 100.0 * self.font.size
            }
        })
    }
}

/// Wonach Längen aufgelöst werden.
#[derive(Clone, Copy)]
struct Units<'a> {
    /// Für `em`, `ex`, `cap`, `ch` und `ic`.
    font: FontBasis<'a>,
    /// Für `lh`.
    lines: FontBasis<'a>,
    /// Für dieselben Einheiten mit `r` davor.
    root_font: FontBasis<'a>,
    root_lines: FontBasis<'a>,
    /// Ob die Inline-Achse des Elements waagrecht ist, für `vi` und `vb`.
    horizontal: bool,
    device: &'a Device,
    fonts: &'a dyn FontMetricsSource,
}

impl Units<'_> {
    /// Eine Länge in `px`.
    fn px(&self, length: Length) -> Result<f64, MissingFontMetric> {
        let factor = match length.unit {
            LengthUnit::Absolute(unit) => unit.to_px(),
            LengthUnit::Font { metric, root } => {
                let (font, lines) = if root {
                    (&self.root_font, &self.root_lines)
                } else {
                    (&self.font, &self.lines)
                };
                self.metric(metric, font, lines)?
            }
            LengthUnit::Viewport { axis, .. } => {
                let width = self.device.viewport_width;
                let height = self.device.viewport_height;
                let (inline, block) = if self.horizontal {
                    (width, height)
                } else {
                    (height, width)
                };
                let size = match axis {
                    ViewportAxis::Width => width,
                    ViewportAxis::Height => height,
                    ViewportAxis::Inline => inline,
                    ViewportAxis::Block => block,
                    ViewportAxis::Min => width.min(height),
                    ViewportAxis::Max => width.max(height),
                };
                size / 100.0
            }
        };
        Ok(clamp(length.value * factor))
    }

    /// Ein Maß einer Schrift in `px`.
    fn metric(
        &self,
        metric: FontMetric,
        font: &FontBasis<'_>,
        lines: &FontBasis<'_>,
    ) -> Result<f64, MissingFontMetric> {
        let em = font.font.size;
        let metrics = || self.fonts.metrics(&font.font);
        Ok(match metric {
            FontMetric::Em => em,
            FontMetric::Lh => lines.line_height_px(self.fonts)?,
            FontMetric::Ex => metrics().x_height.unwrap_or(em * 0.5),
            FontMetric::Cap => {
                let metrics = metrics();
                metrics
                    .cap_height
                    .or(metrics.ascent)
                    .ok_or(MissingFontMetric)?
            }
            // Ohne die Ziffer gilt sie als 0,5em breit und 1em hoch.
            FontMetric::Ch => {
                let upright = font.upright_digits();
                match metrics().zero {
                    Some(advance) if upright => advance.vertical,
                    Some(advance) => advance.horizontal,
                    None if upright => em,
                    None => em * 0.5,
                }
            }
            FontMetric::Ic => {
                let upright = font.upright_ideographs();
                match metrics().water {
                    Some(advance) if upright => advance.vertical,
                    Some(advance) => advance.horizontal,
                    None => em,
                }
            }
        })
    }

    fn length(&self, length: Length) -> Result<Length, MissingFontMetric> {
        Ok(Length::px(self.px(length)?))
    }

    fn length_percentage(
        &self,
        value: LengthPercentage,
    ) -> Result<LengthPercentage, MissingFontMetric> {
        Ok(match value {
            LengthPercentage::Length(length) => LengthPercentage::Length(self.length(length)?),
            percentage => percentage,
        })
    }

    fn box_size(&self, size: BoxSize) -> Result<BoxSize, MissingFontMetric> {
        Ok(match size {
            BoxSize::LengthPercentage(value) => {
                BoxSize::LengthPercentage(self.length_percentage(value)?)
            }
            keyword => keyword,
        })
    }
}

/// Der berechnete Wert einer Farbe außer in `color`: sRGB für Namen,
/// `transparent` und Systemfarben, sonst wie angegeben (Color 4,
/// „Resolving Color Values“).
fn computed_color(color: Color, device: &Device) -> Color {
    match color {
        Color::Named(named) => {
            let [red, green, blue] = named.rgb();
            Color::Rgba(Rgba {
                red,
                green,
                blue,
                alpha: 255,
            })
        }
        Color::Transparent => Color::Rgba(Rgba {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 0,
        }),
        Color::System(system) => Color::Rgba((device.system_color)(system.undeprecated())),
        other => other,
    }
}

/// Die Prozentwerte der Schlüsselwörter von `font-width`.
fn font_width_percentage(keyword: FontWidthKeyword) -> f64 {
    FONT_WIDTHS
        .iter()
        .find(|(found, _)| *found == keyword)
        .map_or(100.0, |(_, percentage)| *percentage)
}

/// `font-weight` mit `bolder` und `lighter` nach der Tabelle in Fonts 4.
fn font_weight(weight: FontWeight, inherited: f64) -> f64 {
    match weight {
        FontWeight::Normal => 400.0,
        FontWeight::Bold => 700.0,
        FontWeight::Number(weight) => weight,
        FontWeight::Bolder => {
            if inherited < 350.0 {
                400.0
            } else if inherited < 550.0 {
                700.0
            } else if inherited < 900.0 {
                900.0
            } else {
                inherited
            }
        }
        FontWeight::Lighter => {
            if inherited < 100.0 {
                inherited
            } else if inherited < 550.0 {
                100.0
            } else if inherited < 750.0 {
                400.0
            } else {
                700.0
            }
        }
    }
}

/// Der Faktor eines ⟨absolute-size⟩ zu `medium` (Fonts 4).
fn absolute_size_factor(size: AbsoluteSize) -> f64 {
    match size {
        AbsoluteSize::XxSmall => 3.0 / 5.0,
        AbsoluteSize::XSmall => 3.0 / 4.0,
        AbsoluteSize::Small => 8.0 / 9.0,
        AbsoluteSize::Medium => 1.0,
        AbsoluteSize::Large => 6.0 / 5.0,
        AbsoluteSize::XLarge => 3.0 / 2.0,
        AbsoluteSize::XxLarge => 2.0,
        AbsoluteSize::XxxLarge => 3.0,
    }
}

/// `font-size` in `px`; `units` bezieht sich auf das Elternelement.
fn font_size(size: FontSize, parent: f64, units: &Units<'_>) -> Result<f64, MissingFontMetric> {
    Ok(clamp(match size {
        FontSize::Absolute(size) => MEDIUM_FONT_SIZE * absolute_size_factor(size),
        FontSize::Relative(RelativeSize::Larger) => parent * RELATIVE_SIZE_RATIO,
        FontSize::Relative(RelativeSize::Smaller) => parent / RELATIVE_SIZE_RATIO,
        FontSize::LengthPercentage(LengthPercentage::Length(length)) => units.px(length)?,
        FontSize::LengthPercentage(LengthPercentage::Percentage(percentage)) => {
            parent * percentage / 100.0
        }
        FontSize::Math => parent,
    }))
}

/// *Snap as a border width* (Borders 4) in Gerätepixeln: Ganze Zahlen
/// bleiben, was dazwischen liegt, wird abgerundet, aber nicht unter 1.
fn snap_border_width(px: f64, device_pixel_ratio: f64) -> f64 {
    if device_pixel_ratio.is_nan() || device_pixel_ratio <= 0.0 {
        return px;
    }
    let pixels = px * device_pixel_ratio;
    let snapped = if pixels == pixels.floor() {
        pixels
    } else if pixels < 1.0 {
        1.0
    } else {
        pixels.floor()
    };
    snapped / device_pixel_ratio
}

/// Blockifiziert einen Wert von `display` (Display 4, „Automatic Box Type
/// Transformations“).
fn blockify(display: Display) -> Display {
    match display {
        Display::None | Display::Contents => display,
        Display::Internal(_) => Display::Pair {
            outside: DisplayOutside::Block,
            inside: DisplayInside::Flow,
            list_item: false,
        },
        Display::Pair {
            outside,
            inside,
            list_item,
        } => {
            // `inline flow-root` und `run-in flow-root` werden zu `block`.
            let inside = if outside != DisplayOutside::Block && inside == DisplayInside::FlowRoot {
                DisplayInside::Flow
            } else {
                inside
            };
            Display::Pair {
                outside: DisplayOutside::Block,
                inside,
                list_item,
            }
        }
    }
}

/// `match-parent` (Text 4): der Wert beim Elternelement, `start` und `end`
/// nach dessen Richtung als `left` oder `right`; beim Wurzelelement `start`.
fn match_parent_align(parent: Option<&ComputedValues>) -> TextAlignAll {
    let Some(parent) = parent else {
        return TextAlignAll::Keyword(TextAlignKeyword::Start);
    };
    let rtl = parent.direction() == Direction::Rtl;
    match parent.get(Longhand::TextAlignAll) {
        SpecifiedValue::TextAlignAll(TextAlignAll::Keyword(TextAlignKeyword::Start)) => {
            TextAlignAll::Keyword(if rtl {
                TextAlignKeyword::Right
            } else {
                TextAlignKeyword::Left
            })
        }
        SpecifiedValue::TextAlignAll(TextAlignAll::Keyword(TextAlignKeyword::End)) => {
            TextAlignAll::Keyword(if rtl {
                TextAlignKeyword::Left
            } else {
                TextAlignKeyword::Right
            })
        }
        SpecifiedValue::TextAlignAll(value) => value.clone(),
        _ => TextAlignAll::Keyword(TextAlignKeyword::Start),
    }
}

/// `match-parent` für `text-align-last`, wie bei `text-align-all`.
fn match_parent_align_last(parent: Option<&ComputedValues>) -> TextAlignLast {
    let Some(parent) = parent else {
        return TextAlignLast::Start;
    };
    let rtl = parent.direction() == Direction::Rtl;
    match parent.get(Longhand::TextAlignLast) {
        SpecifiedValue::TextAlignLast(TextAlignLast::Start) if rtl => TextAlignLast::Right,
        SpecifiedValue::TextAlignLast(TextAlignLast::Start) => TextAlignLast::Left,
        SpecifiedValue::TextAlignLast(TextAlignLast::End) if rtl => TextAlignLast::Left,
        SpecifiedValue::TextAlignLast(TextAlignLast::End) => TextAlignLast::Right,
        SpecifiedValue::TextAlignLast(value) => *value,
        _ => TextAlignLast::Start,
    }
}

/// Eine URL wird zur aufgelösten URL; lässt sie sich nicht auflösen, bleibt
/// der angegebene Text (Values 4).
fn absolute_url(url: &SpecifiedUrl) -> SpecifiedUrl {
    match &url.resolved {
        Some(resolved) => SpecifiedUrl {
            text: resolved.href(),
            resolved: Some(resolved.clone()),
        },
        None => url.clone(),
    }
}

/// Die Langformen, deren Wert ein ⟨color⟩ ist.
fn is_color(longhand: Longhand) -> bool {
    use Longhand as L;
    matches!(
        longhand,
        L::Color
            | L::BackgroundColor
            | L::BorderTopColor
            | L::BorderRightColor
            | L::BorderBottomColor
            | L::BorderLeftColor
            | L::BorderBlockStartColor
            | L::BorderBlockEndColor
            | L::BorderInlineStartColor
            | L::BorderInlineEndColor
            | L::TextDecorationColor
    )
}

/// Die Stilangabe zu einer physischen Rahmenbreite.
fn border_style_of(width: Longhand) -> Option<Longhand> {
    use Longhand as L;
    Some(match width {
        L::BorderTopWidth => L::BorderTopStyle,
        L::BorderRightWidth => L::BorderRightStyle,
        L::BorderBottomWidth => L::BorderBottomStyle,
        L::BorderLeftWidth => L::BorderLeftStyle,
        _ => return None,
    })
}

/// Sammelt die berechneten Werte eines Elements.
struct Computer<'a> {
    cascaded: &'a CascadedValues<'a>,
    parent: Option<&'a ComputedValues>,
    values: Vec<Option<SpecifiedValue>>,
}

impl<'a> Computer<'a> {
    fn specified(&self, longhand: Longhand) -> Specified<'a> {
        defaulted(self.cascaded.get(longhand), self.parent, longhand)
    }

    fn set(&mut self, longhand: Longhand, value: SpecifiedValue) {
        if let Some(entry) = self.values.get_mut(slot(longhand)) {
            *entry = Some(value);
        }
    }

    fn is_set(&self, longhand: Longhand) -> bool {
        self.values.get(slot(longhand)).is_some_and(Option::is_some)
    }

    /// Für Langformen, die wie angegeben berechnet werden.
    fn as_specified(&mut self, longhand: Longhand) -> &'a SpecifiedValue {
        let value = self.specified(longhand).value();
        self.set(longhand, value.clone());
        value
    }
}

/// Der berechnete Wert einer Langform ohne eigene Regel in
/// [`ComputedValues::compute`].
fn compute_value(
    value: &SpecifiedValue,
    units: &Units<'_>,
) -> Result<SpecifiedValue, MissingFontMetric> {
    use SpecifiedValue as V;
    Ok(match value {
        V::Color(color) => V::Color(computed_color(*color, units.device)),
        V::LengthPercentageOrAuto(LengthPercentageOrAuto::LengthPercentage(value)) => {
            V::LengthPercentageOrAuto(LengthPercentageOrAuto::LengthPercentage(
                units.length_percentage(*value)?,
            ))
        }
        V::LengthPercentage(value) => V::LengthPercentage(units.length_percentage(*value)?),
        V::LineWidth(width) => {
            let px = match width {
                LineWidth::Length(length) => units.px(*length)?,
                LineWidth::Keyword(LineWidthKeyword::Hairline) => {
                    1.0 / units.device.device_pixel_ratio
                }
                LineWidth::Keyword(LineWidthKeyword::Thin) => 1.0,
                LineWidth::Keyword(LineWidthKeyword::Medium) => 3.0,
                LineWidth::Keyword(LineWidthKeyword::Thick) => 5.0,
            };
            let px = snap_border_width(px, units.device.device_pixel_ratio);
            V::LineWidth(LineWidth::Length(Length::px(px)))
        }
        V::Size(Size::Box(size)) => V::Size(Size::Box(units.box_size(*size)?)),
        V::MaxSize(MaxSize::Box(size)) => V::MaxSize(MaxSize::Box(units.box_size(*size)?)),
        V::TextIndent(indent) => V::TextIndent(TextIndent {
            length: units.length_percentage(indent.length)?,
            ..*indent
        }),
        // `normal` ist null (Text 4).
        V::Spacing(Spacing::Normal) => {
            V::Spacing(Spacing::LengthPercentage(LengthPercentage::ZERO))
        }
        V::Spacing(Spacing::LengthPercentage(value)) => {
            V::Spacing(Spacing::LengthPercentage(units.length_percentage(*value)?))
        }
        V::TextDecorationThickness(TextDecorationThickness::LengthPercentage(value)) => {
            V::TextDecorationThickness(TextDecorationThickness::LengthPercentage(
                units.length_percentage(*value)?,
            ))
        }
        V::BaselineShift(BaselineShift::LengthPercentage(value)) => V::BaselineShift(
            BaselineShift::LengthPercentage(units.length_percentage(*value)?),
        ),
        V::ListStyleImage(ListStyleImage::Url(url)) => {
            V::ListStyleImage(ListStyleImage::Url(Box::new(absolute_url(url))))
        }
        other => other.clone(),
    })
}

impl ComputedValues {
    /// Kaskadiert und berechnet die Werte eines Elements. `style_attribute`
    /// sind die Deklarationen seines Attributs `style`, `ancestors` fehlt
    /// beim Wurzelelement.
    pub fn of_element(
        stylist: &Stylist,
        context: &MatchingContext<'_>,
        element: NodeId,
        style_attribute: Option<&[Declared]>,
        ancestors: Option<Ancestors<'_>>,
        device: &Device,
        fonts: &dyn FontMetricsSource,
    ) -> Result<ComputedValues, MissingFontMetric> {
        let parent = ancestors.map(|ancestors| ancestors.parent);
        let matched = stylist.matching(context, element, None, style_attribute);
        let cascaded = matched.cascade(|values| flow_mode(values, parent));
        ComputedValues::compute(&cascaded, ancestors, device, fonts)
    }

    /// Berechnet die Werte aus den kaskadierten. Die müssen nach der
    /// Zuordnung aus [`flow_mode`] kaskadiert sein.
    pub fn compute(
        cascaded: &CascadedValues<'_>,
        ancestors: Option<Ancestors<'_>>,
        device: &Device,
        fonts: &dyn FontMetricsSource,
    ) -> Result<ComputedValues, MissingFontMetric> {
        use Longhand as L;
        use SpecifiedValue as V;
        let parent = ancestors.map(|ancestors| ancestors.parent);
        let is_root = ancestors.is_none();
        let mut computer = Computer {
            cascaded,
            parent,
            values: vec![None; Longhand::ALL.len()],
        };

        let writing_mode = match computer.as_specified(L::WritingMode) {
            V::WritingMode(mode) => *mode,
            _ => WritingMode::HorizontalTb,
        };
        let direction = match computer.as_specified(L::Direction) {
            V::Direction(direction) => *direction,
            _ => Direction::Ltr,
        };
        let text_orientation = match computer.as_specified(L::TextOrientation) {
            V::TextOrientation(orientation) => *orientation,
            _ => TextOrientation::Mixed,
        };
        let flow = FlowMode::new(writing_mode, direction, text_orientation);
        debug_assert_eq!(flow, cascaded.flow());

        // Die Schrift, bezogen auf das Elternelement.
        let parent_basis = match parent {
            Some(parent) => parent.basis(),
            None => FontBasis::initial(),
        };
        let root_basis = match ancestors {
            Some(ancestors) => ancestors.root.basis(),
            None => FontBasis::initial(),
        };
        let horizontal = writing_mode == WritingMode::HorizontalTb;
        let font_units = Units {
            font: parent_basis,
            lines: parent_basis,
            root_font: root_basis,
            root_lines: root_basis,
            horizontal,
            device,
            fonts,
        };
        let family = match computer.as_specified(L::FontFamily) {
            V::FontFamily(family) => family,
            _ => initial_family(),
        };
        let style = match computer.specified(L::FontStyle) {
            Specified::Value(V::FontStyle(FontStyle::Oblique(Some(angle)))) => {
                FontStyle::Oblique(Some(Angle {
                    value: angle.degrees(),
                    unit: AngleUnit::Deg,
                }))
            }
            Specified::Value(V::FontStyle(style)) | Specified::Inherited(V::FontStyle(style)) => {
                *style
            }
            _ => FontStyle::Normal,
        };
        computer.set(L::FontStyle, V::FontStyle(style));
        let weight = match computer.specified(L::FontWeight) {
            Specified::Value(V::FontWeight(weight)) => {
                font_weight(*weight, parent_basis.font.weight)
            }
            Specified::Inherited(V::FontWeight(FontWeight::Number(weight))) => *weight,
            _ => 400.0,
        };
        computer.set(L::FontWeight, V::FontWeight(FontWeight::Number(weight)));
        let width = match computer.specified(L::FontWidth).value() {
            V::FontWidth(FontWidth::Keyword(keyword)) => font_width_percentage(*keyword),
            V::FontWidth(FontWidth::Percentage(percentage)) => *percentage,
            _ => 100.0,
        };
        computer.set(L::FontWidth, V::FontWidth(FontWidth::Percentage(width)));
        let variant_caps = match computer.as_specified(L::FontVariantCaps) {
            V::FontVariantCaps(caps) => *caps,
            _ => FontVariantCaps::Normal,
        };
        let size = match computer.specified(L::FontSize) {
            Specified::Value(V::FontSize(size)) => {
                font_size(*size, parent_basis.font.size, &font_units)?
            }
            Specified::Inherited(value) => match value {
                V::FontSize(FontSize::LengthPercentage(LengthPercentage::Length(length))) => {
                    length.value
                }
                _ => MEDIUM_FONT_SIZE,
            },
            Specified::Value(_) => MEDIUM_FONT_SIZE,
        };
        computer.set(
            L::FontSize,
            V::FontSize(FontSize::LengthPercentage(LengthPercentage::Length(
                Length::px(size),
            ))),
        );
        let font = Font {
            family,
            size,
            weight,
            style,
            width,
            variant_caps,
        };

        // `line-height`: `em` nach der eigenen Schrift, `lh` nach dem
        // Elternelement.
        let own = FontBasis {
            font,
            line_height: LineHeight::Normal,
            writing_mode,
            text_orientation,
        };
        let line_units = Units {
            font: own,
            lines: parent_basis,
            root_font: if is_root { own } else { root_basis },
            root_lines: root_basis,
            ..font_units
        };
        let line_height = match computer.specified(L::LineHeight) {
            Specified::Value(V::LineHeight(LineHeight::LengthPercentage(value))) => {
                let px = match value {
                    LengthPercentage::Length(length) => line_units.px(*length)?,
                    LengthPercentage::Percentage(percentage) => clamp(size * percentage / 100.0),
                };
                LineHeight::LengthPercentage(LengthPercentage::Length(Length::px(px)))
            }
            Specified::Value(V::LineHeight(value)) | Specified::Inherited(V::LineHeight(value)) => {
                *value
            }
            _ => LineHeight::Normal,
        };
        computer.set(L::LineHeight, V::LineHeight(line_height));

        // Alles Übrige nach der eigenen Schrift.
        let own = FontBasis { line_height, ..own };
        let root_own = if is_root { own } else { root_basis };
        let units = Units {
            font: own,
            lines: own,
            root_font: root_own,
            root_lines: root_own,
            ..font_units
        };

        // `currentcolor` in `color` gilt als `inherit`.
        let color = match computer.specified(L::Color) {
            Specified::Value(V::Color(Color::CurrentColor)) => match parent {
                Some(parent) => parent.get(L::Color).clone(),
                None => compute_value(initial(L::Color), &units)?,
            },
            Specified::Value(value) => compute_value(value, &units)?,
            Specified::Inherited(value) => value.clone(),
        };
        computer.set(L::Color, color);

        let parent_blockifies = parent.is_some_and(|parent| parent.blockifies_children);
        let display = match computer.specified(L::Display).value() {
            V::Display(display) => *display,
            _ => Display::Pair {
                outside: DisplayOutside::Inline,
                inside: DisplayInside::Flow,
                list_item: false,
            },
        };
        // Beim Wurzelelement wird `contents` zu `block`.
        let display = if is_root && display == Display::Contents {
            Display::Pair {
                outside: DisplayOutside::Block,
                inside: DisplayInside::Flow,
                list_item: false,
            }
        } else if is_root || parent_blockifies {
            blockify(display)
        } else {
            display
        };
        computer.set(L::Display, V::Display(display));
        let blockifies_children = match display {
            Display::Pair {
                inside: DisplayInside::Flex | DisplayInside::Grid,
                ..
            } => true,
            Display::Contents => parent_blockifies,
            _ => false,
        };

        if let Specified::Value(V::TextAlignAll(TextAlignAll::Keyword(
            TextAlignKeyword::MatchParent,
        ))) = computer.specified(L::TextAlignAll)
        {
            computer.set(L::TextAlignAll, V::TextAlignAll(match_parent_align(parent)));
        }
        if let Specified::Value(V::TextAlignLast(TextAlignLast::MatchParent)) =
            computer.specified(L::TextAlignLast)
        {
            computer.set(
                L::TextAlignLast,
                V::TextAlignLast(match_parent_align_last(parent)),
            );
        }

        for &longhand in Longhand::ALL {
            if is_logical(longhand) || computer.is_set(longhand) {
                continue;
            }
            let value = match computer.specified(longhand) {
                Specified::Value(value) => compute_value(value, &units)?,
                Specified::Inherited(value) => value.clone(),
            };
            computer.set(longhand, value);
        }
        for &longhand in Longhand::ALL {
            if is_logical(longhand) {
                let physical = computer
                    .values
                    .get(slot(longhand.physical(flow)))
                    .cloned()
                    .flatten();
                if let Some(value) = physical {
                    computer.set(longhand, value);
                }
            }
        }

        let values = Longhand::ALL
            .iter()
            .zip(computer.values)
            .map(|(&longhand, value)| value.unwrap_or_else(|| initial(longhand).clone()))
            .collect();
        Ok(ComputedValues {
            values,
            flow,
            blockifies_children,
            flex_or_grid_item: parent_blockifies,
        })
    }

    /// Der berechnete Wert einer Langform; bei einer logischen der der
    /// physischen, die ihr zugeordnet ist.
    pub fn get(&self, longhand: Longhand) -> &SpecifiedValue {
        self.values
            .get(slot(longhand))
            .unwrap_or_else(|| initial(longhand))
    }

    /// Die Zuordnung der logischen Langformen.
    pub fn flow(&self) -> FlowMode {
        self.flow
    }

    /// Die berechnete Schrift.
    pub fn font(&self) -> Font<'_> {
        use Longhand as L;
        use SpecifiedValue as V;
        Font {
            family: match self.get(L::FontFamily) {
                V::FontFamily(family) => family,
                _ => initial_family(),
            },
            size: self.font_size(),
            weight: match self.get(L::FontWeight) {
                V::FontWeight(FontWeight::Number(weight)) => *weight,
                _ => 400.0,
            },
            style: match self.get(L::FontStyle) {
                V::FontStyle(style) => *style,
                _ => FontStyle::Normal,
            },
            width: match self.get(L::FontWidth) {
                V::FontWidth(FontWidth::Percentage(width)) => *width,
                _ => 100.0,
            },
            variant_caps: match self.get(L::FontVariantCaps) {
                V::FontVariantCaps(caps) => *caps,
                _ => FontVariantCaps::Normal,
            },
        }
    }

    /// `font-size` in `px`.
    pub fn font_size(&self) -> f64 {
        match self.get(Longhand::FontSize) {
            SpecifiedValue::FontSize(FontSize::LengthPercentage(LengthPercentage::Length(
                length,
            ))) => length.value,
            _ => MEDIUM_FONT_SIZE,
        }
    }

    /// Der berechnete Wert von `color`: nie `currentcolor`.
    pub fn color(&self) -> Color {
        match self.get(Longhand::Color) {
            SpecifiedValue::Color(color) => *color,
            _ => Color::CurrentColor,
        }
    }

    /// Der berechnete Wert von `display`.
    pub fn display(&self) -> Display {
        match self.get(Longhand::Display) {
            SpecifiedValue::Display(display) => *display,
            _ => Display::None,
        }
    }

    fn direction(&self) -> Direction {
        match self.get(Longhand::Direction) {
            SpecifiedValue::Direction(direction) => *direction,
            _ => Direction::Ltr,
        }
    }

    fn basis(&self) -> FontBasis<'_> {
        FontBasis {
            font: self.font(),
            line_height: match self.get(Longhand::LineHeight) {
                SpecifiedValue::LineHeight(line_height) => *line_height,
                _ => LineHeight::Normal,
            },
            writing_mode: self.flow.writing_mode,
            text_orientation: match self.get(Longhand::TextOrientation) {
                SpecifiedValue::TextOrientation(orientation) => *orientation,
                _ => TextOrientation::Mixed,
            },
        }
    }

    /// Der Wert für `getComputedStyle()` (*resolved value*, CSSOM). Für
    /// die Farben, `line-height` und die Rahmenbreiten (Borders 4) ist das
    /// der verwendete Wert, für Breite, Höhe, `margin-*` und `padding-*`
    /// ebenso, wenn das Element einen Kasten erzeugt; der hängt meist vom
    /// Layout ab. Nur `padding-*` mit einer Länge ist ohne Layout bekannt:
    /// Keine der vorhandenen Eigenschaften ändert ihn (`border-collapse`,
    /// das es täte, fehlt noch). `min-width` und `min-height` mit `auto`
    /// sind `0px` außer bei Flex- und Grid-Elementen (Sizing 3), ein
    /// `letter-spacing` von null ist `normal` (Text 4).
    pub fn resolved(&self, longhand: Longhand) -> Result<SpecifiedValue, NeedsLayout> {
        use Longhand as L;
        use SpecifiedValue as V;
        let longhand = longhand.physical(self.flow);
        let value = self.get(longhand);
        let generates_box = !matches!(self.display(), Display::None | Display::Contents);
        Ok(match (longhand, value) {
            (_, V::Color(Color::CurrentColor)) => V::Color(self.color()),
            (L::LineHeight, V::LineHeight(LineHeight::Number(number))) => {
                V::LineHeight(LineHeight::LengthPercentage(LengthPercentage::Length(
                    Length::px(clamp(number * self.font_size())),
                )))
            }
            (
                L::Width
                | L::Height
                | L::MarginTop
                | L::MarginRight
                | L::MarginBottom
                | L::MarginLeft,
                _,
            ) if generates_box => return Err(NeedsLayout),
            (
                L::PaddingTop | L::PaddingRight | L::PaddingBottom | L::PaddingLeft,
                V::LengthPercentage(LengthPercentage::Percentage(_)),
            ) if generates_box => return Err(NeedsLayout),
            (L::MinWidth | L::MinHeight, V::Size(Size::Auto))
                if !(generates_box && self.flex_or_grid_item) =>
            {
                V::Size(Size::Box(BoxSize::LengthPercentage(LengthPercentage::ZERO)))
            }
            (
                L::BorderTopWidth | L::BorderRightWidth | L::BorderBottomWidth | L::BorderLeftWidth,
                _,
            ) if border_style_of(longhand).is_some_and(|style| {
                matches!(
                    self.get(style),
                    V::LineStyle(LineStyle::None | LineStyle::Hidden)
                )
            }) =>
            {
                V::LineWidth(LineWidth::Length(Length::px(0.0)))
            }
            (
                L::LetterSpacing,
                V::Spacing(Spacing::LengthPercentage(LengthPercentage::Length(length))),
            ) if length.value == 0.0 => V::Spacing(Spacing::Normal),
            _ => value.clone(),
        })
    }

    /// Eine Kurzschreibweise für `getComputedStyle()`, aus den Werten von
    /// [`ComputedValues::resolved`]; leer, wenn sie die Werte nicht
    /// wiedergeben kann. CSSOM legt das für Kurzschreibweisen nicht fest
    /// (w3c/csswg-drafts#2529). Wo eine Kurzschreibweise Farben mit anderen
    /// Langformen mischt, etwa `text-decoration` oder `border-top`, bleibt
    /// `currentcolor` stehen und fällt so als Anfangswert weg; in `font`
    /// fallen `font-weight: 400` und `font-width: 100%` weg, und
    /// Prozentwerte mit einem Schlüsselwort werden zu diesem.
    pub fn resolved_shorthand(&self, shorthand: Shorthand) -> Result<String, NeedsLayout> {
        // `all` gibt nur ein CSS-weites Schlüsselwort wieder, und das ist
        // nie ein berechneter Wert.
        if shorthand == Shorthand::All {
            return Ok(String::new());
        }
        let longhands = shorthand.longhands();
        let mixed = longhands.iter().any(|&longhand| is_color(longhand))
            && longhands.iter().any(|&longhand| !is_color(longhand));
        let mut values = Vec::with_capacity(longhands.len());
        for &longhand in longhands {
            let value = match self.get(longhand) {
                SpecifiedValue::Color(Color::CurrentColor) if mixed => {
                    SpecifiedValue::Color(Color::CurrentColor)
                }
                _ => self.resolved(longhand)?,
            };
            let value = match value {
                SpecifiedValue::FontWeight(FontWeight::Number(weight))
                    if shorthand == Shorthand::Font && weight == 400.0 =>
                {
                    SpecifiedValue::FontWeight(FontWeight::Normal)
                }
                SpecifiedValue::FontWidth(FontWidth::Percentage(percentage))
                    if shorthand == Shorthand::Font =>
                {
                    match FONT_WIDTHS.iter().find(|(_, found)| *found == percentage) {
                        Some((keyword, _)) => {
                            SpecifiedValue::FontWidth(FontWidth::Keyword(*keyword))
                        }
                        None => SpecifiedValue::FontWidth(FontWidth::Percentage(percentage)),
                    }
                }
                other => other,
            };
            values.push(DeclaredValue::Value(value));
        }
        let values: Vec<&DeclaredValue> = values.iter().collect();
        Ok(serialize_shorthand(shorthand, &values).unwrap_or_default())
    }
}

/// Ein Gerät für Tests: 800 × 600 wie das Fenster der WPT, ein Gerätepixel
/// je `px` und feste Systemfarben. Die Farben sind für Tests gewählt und
/// sagen nichts über eine Plattform; Texte heben sich von ihren
/// Hintergründen ab.
#[cfg(test)]
pub(crate) fn test_device() -> Device {
    fn system_color(color: SystemColor) -> Rgba {
        let rgb = |red, green, blue| Rgba {
            red,
            green,
            blue,
            alpha: 255,
        };
        match color {
            SystemColor::Canvas
            | SystemColor::ButtonFace
            | SystemColor::Field
            | SystemColor::SelectedItemText
            | SystemColor::AccentColorText
            | SystemColor::HighlightText => rgb(255, 255, 255),
            SystemColor::LinkText => rgb(0x00, 0x00, 0xEE),
            SystemColor::VisitedText => rgb(0x55, 0x1A, 0x8B),
            SystemColor::ActiveText => rgb(0xFF, 0x00, 0x00),
            SystemColor::Mark => rgb(0xFF, 0xFF, 0x00),
            SystemColor::GrayText => rgb(0x80, 0x80, 0x80),
            SystemColor::AccentColor | SystemColor::Highlight | SystemColor::SelectedItem => {
                rgb(0x00, 0x00, 0x80)
            }
            _ => rgb(0, 0, 0),
        }
    }
    Device {
        viewport_width: 800.0,
        viewport_height: 600.0,
        device_pixel_ratio: 1.0,
        system_color,
    }
}
