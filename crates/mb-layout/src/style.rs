//! Die berechneten Werte, wie das Layout sie braucht. Längen sind nach der
//! Berechnung immer `px`; Prozente beziehen sich auf eine Größe, die erst
//! das Layout kennt.

use mb_style::ComputedValues;
use mb_style::properties::{
    AlignmentBaseline, BaselineShift, BoxSize, BoxSizing, LengthPercentageOrAuto, LineHeight,
    LineStyle, LineWidth, Longhand, MaxSize, OutlineColor, OutlineStyle, Size, SpecifiedValue,
    TextAlignAll, TextAlignKeyword, TextAlignLast, TextIndent, TextWrapMode, Visibility,
    WhiteSpaceCollapse,
};
use mb_style::values::LengthPercentage;
use mb_style::values::color::{Color, ColorSpace, Rgba};

/// Eine Seite eines Kastens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub(crate) const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];
}

/// Vier Werte, je Seite einer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Sides {
    pub(crate) top: f64,
    pub(crate) right: f64,
    pub(crate) bottom: f64,
    pub(crate) left: f64,
}

impl Sides {
    pub(crate) fn horizontal(&self) -> f64 {
        self.left + self.right
    }

    pub(crate) fn vertical(&self) -> f64 {
        self.top + self.bottom
    }
}

/// Ein ⟨length-percentage⟩ in `px`, Prozente von `basis`.
pub(crate) fn resolve(value: LengthPercentage, basis: f64) -> f64 {
    match value {
        LengthPercentage::Length(length) => length.value,
        LengthPercentage::Percentage(percentage) => basis * percentage / 100.0,
    }
}

/// Ein ⟨length-percentage⟩ in `px`; ein Prozentwert ohne Bezugsgröße ist
/// `None`.
pub(crate) fn resolve_definite(value: LengthPercentage, basis: Option<f64>) -> Option<f64> {
    match value {
        LengthPercentage::Length(length) => Some(length.value),
        LengthPercentage::Percentage(percentage) => basis.map(|basis| basis * percentage / 100.0),
    }
}

/// Eine Größe aus `width`, `height` und den Grenzen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SizeValue {
    Auto,
    LengthPercentage(LengthPercentage),
    MinContent,
    MaxContent,
    FitContent,
    Stretch,
}

impl SizeValue {
    fn from_box(size: BoxSize) -> SizeValue {
        use mb_style::properties::SizeKeyword as K;
        match size {
            BoxSize::LengthPercentage(value) => SizeValue::LengthPercentage(value),
            BoxSize::Keyword(K::MinContent) => SizeValue::MinContent,
            BoxSize::Keyword(K::MaxContent) => SizeValue::MaxContent,
            BoxSize::Keyword(K::FitContent) => SizeValue::FitContent,
            BoxSize::Keyword(K::Stretch) => SizeValue::Stretch,
        }
    }
}

/// Ausrichtung des Inhalts einer Zeile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Right,
    Center,
    Justify,
}

/// `vertical-align`, wie es das Layout auswertet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum VerticalAlign {
    /// Verschiebung der Grundlinie nach oben in `px`, von der des
    /// Elternkastens aus (`baseline`, `sub`, `super`, Längen).
    Shift(f64),
    TextTop,
    TextBottom,
    Middle,
    Top,
    Bottom,
}

/// Lesezugriffe auf berechnete Werte.
pub(crate) trait Style {
    fn values(&self) -> &ComputedValues;

    fn visible(&self) -> bool {
        matches!(
            self.values().get(Longhand::Visibility),
            SpecifiedValue::Visibility(Visibility::Visible)
        )
    }

    fn margin(&self, side: Side) -> LengthPercentageOrAuto {
        let longhand = match side {
            Side::Top => Longhand::MarginTop,
            Side::Right => Longhand::MarginRight,
            Side::Bottom => Longhand::MarginBottom,
            Side::Left => Longhand::MarginLeft,
        };
        match self.values().get(longhand) {
            SpecifiedValue::LengthPercentageOrAuto(value) => *value,
            _ => LengthPercentageOrAuto::LengthPercentage(LengthPercentage::ZERO),
        }
    }

    /// Die Außenabstände in `px`, `auto` als null. Prozente beziehen sich
    /// auf die Breite des umgebenden Blocks.
    fn margins_or_zero(&self, basis: f64) -> Sides {
        let side = |side| match self.margin(side) {
            LengthPercentageOrAuto::Auto => 0.0,
            LengthPercentageOrAuto::LengthPercentage(value) => resolve(value, basis),
        };
        Sides {
            top: side(Side::Top),
            right: side(Side::Right),
            bottom: side(Side::Bottom),
            left: side(Side::Left),
        }
    }

    fn padding(&self, basis: f64) -> Sides {
        let side = |longhand| match self.values().get(longhand) {
            SpecifiedValue::LengthPercentage(value) => resolve(*value, basis).max(0.0),
            _ => 0.0,
        };
        Sides {
            top: side(Longhand::PaddingTop),
            right: side(Longhand::PaddingRight),
            bottom: side(Longhand::PaddingBottom),
            left: side(Longhand::PaddingLeft),
        }
    }

    fn border_style(&self, side: Side) -> LineStyle {
        let longhand = match side {
            Side::Top => Longhand::BorderTopStyle,
            Side::Right => Longhand::BorderRightStyle,
            Side::Bottom => Longhand::BorderBottomStyle,
            Side::Left => Longhand::BorderLeftStyle,
        };
        match self.values().get(longhand) {
            SpecifiedValue::LineStyle(style) => *style,
            _ => LineStyle::None,
        }
    }

    /// Die Rahmenbreiten: null bei `none` und `hidden` (Backgrounds 3).
    fn border(&self) -> Sides {
        let side = |side| {
            let longhand = match side {
                Side::Top => Longhand::BorderTopWidth,
                Side::Right => Longhand::BorderRightWidth,
                Side::Bottom => Longhand::BorderBottomWidth,
                Side::Left => Longhand::BorderLeftWidth,
            };
            if matches!(self.border_style(side), LineStyle::None | LineStyle::Hidden) {
                return 0.0;
            }
            match self.values().get(longhand) {
                SpecifiedValue::LineWidth(LineWidth::Length(length)) => length.value.max(0.0),
                _ => 0.0,
            }
        };
        Sides {
            top: side(Side::Top),
            right: side(Side::Right),
            bottom: side(Side::Bottom),
            left: side(Side::Left),
        }
    }

    fn border_color(&self, side: Side) -> Option<Rgba> {
        let longhand = match side {
            Side::Top => Longhand::BorderTopColor,
            Side::Right => Longhand::BorderRightColor,
            Side::Bottom => Longhand::BorderBottomColor,
            Side::Left => Longhand::BorderLeftColor,
        };
        self.color_of(longhand)
    }

    /// Die Farbe in einer Langform mit ⟨color⟩, `currentcolor` aufgelöst.
    fn color_of(&self, longhand: Longhand) -> Option<Rgba> {
        match self.values().get(longhand) {
            SpecifiedValue::Color(color) => self.resolve_color(*color),
            _ => None,
        }
    }

    /// `color`.
    fn text_color(&self) -> Option<Rgba> {
        srgb(self.values().color())
    }

    fn resolve_color(&self, color: Color) -> Option<Rgba> {
        match color {
            Color::CurrentColor => self.text_color(),
            other => srgb(other),
        }
    }

    /// Breite, Stil und Farbe des Umrisses; `None` ohne Umriss.
    fn outline(&self) -> Option<(f64, LineStyle, Option<Rgba>)> {
        let style = match self.values().get(Longhand::OutlineStyle) {
            // `auto` darf wie `solid` gezeichnet werden (CSS UI 4).
            SpecifiedValue::OutlineStyle(OutlineStyle::Auto) => LineStyle::Solid,
            SpecifiedValue::OutlineStyle(OutlineStyle::Line(style))
                if *style != LineStyle::None =>
            {
                *style
            }
            _ => return None,
        };
        let width = match self.values().get(Longhand::OutlineWidth) {
            SpecifiedValue::LineWidth(LineWidth::Length(length)) => length.value,
            _ => 0.0,
        };
        let color = match self.values().get(Longhand::OutlineColor) {
            SpecifiedValue::OutlineColor(OutlineColor::Color(color)) => self.resolve_color(*color),
            // `auto` steht für die Akzentfarbe; die gibt es noch nicht.
            _ => self.text_color(),
        };
        (width > 0.0).then_some((width, style, color))
    }

    fn outline_offset(&self) -> f64 {
        match self.values().get(Longhand::OutlineOffset) {
            SpecifiedValue::Length(length) => length.value,
            _ => 0.0,
        }
    }

    fn size(&self, longhand: Longhand) -> SizeValue {
        match self.values().get(longhand) {
            SpecifiedValue::Size(Size::Box(size)) => SizeValue::from_box(*size),
            SpecifiedValue::MaxSize(MaxSize::Box(size)) => SizeValue::from_box(*size),
            _ => SizeValue::Auto,
        }
    }

    /// `max-width` oder `max-height`; `none` ist `None`.
    fn max_size(&self, longhand: Longhand) -> Option<SizeValue> {
        match self.values().get(longhand) {
            SpecifiedValue::MaxSize(MaxSize::Box(size)) => Some(SizeValue::from_box(*size)),
            _ => None,
        }
    }

    fn border_box_sizing(&self) -> bool {
        matches!(
            self.values().get(Longhand::BoxSizing),
            SpecifiedValue::BoxSizing(BoxSizing::BorderBox)
        )
    }

    fn line_height(&self) -> LineHeight {
        match self.values().get(Longhand::LineHeight) {
            SpecifiedValue::LineHeight(value) => *value,
            _ => LineHeight::Normal,
        }
    }

    fn white_space_collapse(&self) -> WhiteSpaceCollapse {
        match self.values().get(Longhand::WhiteSpaceCollapse) {
            SpecifiedValue::WhiteSpaceCollapse(value) => *value,
            _ => WhiteSpaceCollapse::Collapse,
        }
    }

    fn wraps(&self) -> bool {
        !matches!(
            self.values().get(Longhand::TextWrapMode),
            SpecifiedValue::TextWrapMode(TextWrapMode::Nowrap)
        )
    }

    fn ltr(&self) -> bool {
        !matches!(
            self.values().get(Longhand::Direction),
            SpecifiedValue::Direction(mb_style::properties::Direction::Rtl)
        )
    }

    /// `text-align-all` mit `start` und `end` nach der Richtung.
    fn text_align(&self) -> Align {
        let keyword = match self.values().get(Longhand::TextAlignAll) {
            SpecifiedValue::TextAlignAll(TextAlignAll::Keyword(keyword)) => *keyword,
            _ => TextAlignKeyword::Start,
        };
        self.align_keyword(keyword)
    }

    /// `text-align-last`; `auto` ist `None` (dann gilt `text-align-all`,
    /// bei `justify` aber `start`).
    fn text_align_last(&self) -> Option<Align> {
        let keyword = match self.values().get(Longhand::TextAlignLast) {
            SpecifiedValue::TextAlignLast(keyword) => *keyword,
            _ => TextAlignLast::Auto,
        };
        let keyword = match keyword {
            TextAlignLast::Auto => return None,
            TextAlignLast::Start => TextAlignKeyword::Start,
            TextAlignLast::End => TextAlignKeyword::End,
            TextAlignLast::Left => TextAlignKeyword::Left,
            TextAlignLast::Right => TextAlignKeyword::Right,
            TextAlignLast::Center => TextAlignKeyword::Center,
            TextAlignLast::Justify => TextAlignKeyword::Justify,
            // Ist schon beim Berechnen aufgelöst.
            TextAlignLast::MatchParent => TextAlignKeyword::Start,
        };
        Some(self.align_keyword(keyword))
    }

    fn align_keyword(&self, keyword: TextAlignKeyword) -> Align {
        let ltr = self.ltr();
        match keyword {
            TextAlignKeyword::Left => Align::Left,
            TextAlignKeyword::Right => Align::Right,
            TextAlignKeyword::Center => Align::Center,
            TextAlignKeyword::Justify => Align::Justify,
            TextAlignKeyword::End if ltr => Align::Right,
            TextAlignKeyword::End => Align::Left,
            TextAlignKeyword::Start | TextAlignKeyword::MatchParent if ltr => Align::Left,
            TextAlignKeyword::Start | TextAlignKeyword::MatchParent => Align::Right,
        }
    }

    fn text_indent(&self) -> Option<TextIndent> {
        match self.values().get(Longhand::TextIndent) {
            SpecifiedValue::TextIndent(indent) => Some(*indent),
            _ => None,
        }
    }

    /// `letter-spacing` oder `word-spacing` in `px`; Prozente beziehen sich
    /// auf die Breite des Zeichens beziehungsweise des Leerzeichens.
    fn spacing(&self, longhand: Longhand, basis: f64) -> f64 {
        match self.values().get(longhand) {
            SpecifiedValue::Spacing(mb_style::properties::Spacing::LengthPercentage(value)) => {
                resolve(*value, basis)
            }
            _ => 0.0,
        }
    }

    /// `vertical-align` aus `alignment-baseline` und `baseline-shift`;
    /// Prozente von `baseline-shift` beziehen sich auf `line-height`.
    fn vertical_align(&self, line_height: f64, parent_font_size: f64) -> VerticalAlign {
        let shift = match self.values().get(Longhand::BaselineShift) {
            SpecifiedValue::BaselineShift(shift) => *shift,
            _ => BaselineShift::LengthPercentage(LengthPercentage::ZERO),
        };
        let offset = match shift {
            BaselineShift::Top => return VerticalAlign::Top,
            BaselineShift::Bottom => return VerticalAlign::Bottom,
            BaselineShift::LengthPercentage(value) => resolve(value, line_height),
            // Die Werte aus CSS Inline 3 für Schriften ohne eigene Angaben,
            // nach der Schriftgröße des Elternkastens.
            BaselineShift::Sub => -parent_font_size / 5.0,
            BaselineShift::Super => parent_font_size / 3.0,
            // `center` gehört zum Schreibmodus vertikaler Texte.
            BaselineShift::Center => 0.0,
        };
        match self.values().get(Longhand::AlignmentBaseline) {
            SpecifiedValue::AlignmentBaseline(AlignmentBaseline::TextTop) => VerticalAlign::TextTop,
            SpecifiedValue::AlignmentBaseline(AlignmentBaseline::TextBottom) => {
                VerticalAlign::TextBottom
            }
            SpecifiedValue::AlignmentBaseline(AlignmentBaseline::Middle) => VerticalAlign::Middle,
            _ => VerticalAlign::Shift(offset),
        }
    }
}

impl Style for ComputedValues {
    fn values(&self) -> &ComputedValues {
        self
    }
}

/// Eine berechnete Farbe als sRGB. `currentcolor` muss vorher aufgelöst
/// sein. Von den Farbfunktionen gehen bisher nur die in sRGB; die anderen
/// Farbräume fehlen noch und zeichnen nichts.
pub(crate) fn srgb(color: Color) -> Option<Rgba> {
    match color {
        Color::Rgba(rgba) => Some(rgba),
        Color::Named(named) => {
            let [red, green, blue] = named.rgb();
            Some(Rgba {
                red,
                green,
                blue,
                alpha: 255,
            })
        }
        Color::Transparent => Some(Rgba {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 0,
        }),
        Color::Function(function) if function.space == ColorSpace::Srgb => {
            let [red, green, blue] = function
                .components
                .map(|component| byte(component.unwrap_or(0.0) * 255.0));
            Some(Rgba {
                red,
                green,
                blue,
                alpha: byte(function.alpha.unwrap_or(1.0) * 255.0),
            })
        }
        Color::CurrentColor | Color::System(_) | Color::Function(_) => None,
    }
}

/// Ein Kanal von 0 bis 255, gerundet.
fn byte(value: f64) -> u8 {
    // Nach dem Begrenzen passt der Wert in u8; NaN wird 0.
    value.round().clamp(0.0, 255.0) as u8
}
