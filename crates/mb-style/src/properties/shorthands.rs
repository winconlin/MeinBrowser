//! Die Kurzschreibweisen: ihre Langformen in kanonischer Reihenfolge, das
//! Zerlegen eines Werts und das Serialisieren aus den Werten der
//! Langformen.
//!
//! Beim Serialisieren werden Teile weggelassen, die dem Anfangswert ihrer
//! Langform gleichen, wie es CSSOM verlangt („omit … without changing the
//! meaning“) und die WPT erwarten.

use std::sync::OnceLock;

use super::{
    AlignmentBaseline, Attachment, BaselineShift, BaselineSource, BgClip, BgImage, BgSize,
    EdgeSide, FontFamilyList, FontSize, FontStyle, FontVariantCaps, FontWeight, FontWidth,
    FontWidthKeyword, HorizontalEdge, Layers, LengthPercentageOrAuto, LineHeight, LineStyle,
    LineWidth, ListStyleImage, ListStylePosition, ListStyleType, Longhand, OutlineColor,
    OutlineStyle, PositionComponent, PositionEdge, RepeatStyle, SpecifiedValue, TextAlignAll,
    TextAlignKeyword, TextAlignLast, TextDecorationLine, TextDecorationStyle,
    TextDecorationThickness, TextWrapMode, TextWrapStyle, VerticalEdge, VisualBox,
    WhiteSpaceCollapse, WhiteSpaceTrim,
};
use crate::values::color::Color;
use crate::values::{Input, LengthPercentage, Parse, ToCss, non_negative_length_percentage};

/// Die Definition einer Kurzschreibweise.
#[derive(Debug)]
pub struct ShorthandInfo {
    pub name: &'static str,
    /// Die Datei in webref, deren Definition gilt.
    pub spec: &'static str,
    pub syntax: &'static str,
    /// Die Langformen in kanonischer Reihenfolge. Bei `all` steht hier
    /// nichts, siehe [`Shorthand::longhands`].
    pub longhands: &'static [Longhand],
    /// Ob ⟨quirky-length⟩s gelten (CSS Values 4, Anhang C).
    pub quirky: bool,
}

/// Legt [`Shorthand`] mit einer [`ShorthandInfo`] je Eigenschaft an.
macro_rules! shorthands {
    ($($variant:ident = $name:literal, $spec:literal, $syntax:expr, [$($longhand:ident),*] $(, $quirky:ident)?;)*) => {
        /// Eine Kurzschreibweise.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Shorthand {
            $($variant,)*
        }

        impl Shorthand {
            /// Alle Kurzschreibweisen.
            pub const ALL: &'static [Shorthand] = &[$(Shorthand::$variant,)*];

            pub fn info(self) -> &'static ShorthandInfo {
                match self {
                    $(Shorthand::$variant => {
                        const INFO: ShorthandInfo = ShorthandInfo {
                            name: $name,
                            spec: $spec,
                            syntax: $syntax,
                            longhands: &[$(Longhand::$longhand),*],
                            quirky: shorthands!(@quirky $($quirky)?),
                        };
                        &INFO
                    })*
                }
            }
        }
    };
    (@quirky quirky) => { true };
    (@quirky) => { false };
}

const SIDE: &str = "<line-width> || <line-style> || <color>";

shorthands! {
    BackgroundPosition = "background-position", "css-backgrounds-4", "<bg-position>#",
        [BackgroundPositionX, BackgroundPositionY];
    Background = "background", "css-backgrounds-4", "<bg-layer>#? , <final-bg-layer>",
        [
            BackgroundImage, BackgroundPositionX, BackgroundPositionY, BackgroundSize,
            BackgroundRepeat, BackgroundAttachment, BackgroundOrigin, BackgroundClip,
            BackgroundColor
        ];
    Margin = "margin", "css-box", "<'margin-top'>{1,4}",
        [MarginTop, MarginRight, MarginBottom, MarginLeft], quirky;
    Padding = "padding", "css-box", "<'padding-top'>{1,4}",
        [PaddingTop, PaddingRight, PaddingBottom, PaddingLeft], quirky;
    MarginBlock = "margin-block", "css-logical", "<'margin-top'>{1,2}",
        [MarginBlockStart, MarginBlockEnd];
    MarginInline = "margin-inline", "css-logical", "<'margin-top'>{1,2}",
        [MarginInlineStart, MarginInlineEnd];
    PaddingBlock = "padding-block", "css-logical", "<'padding-top'>{1,2}",
        [PaddingBlockStart, PaddingBlockEnd];
    PaddingInline = "padding-inline", "css-logical", "<'padding-top'>{1,2}",
        [PaddingInlineStart, PaddingInlineEnd];
    BorderWidth = "border-width", "css-borders", "<'border-top-width'>{1,4}",
        [BorderTopWidth, BorderRightWidth, BorderBottomWidth, BorderLeftWidth], quirky;
    BorderStyle = "border-style", "css-borders", "<'border-top-style'>{1,4}",
        [BorderTopStyle, BorderRightStyle, BorderBottomStyle, BorderLeftStyle];
    BorderColor = "border-color", "css-borders", "[ <color> | <image-1D> ]{1,4}",
        [BorderTopColor, BorderRightColor, BorderBottomColor, BorderLeftColor];
    BorderBlockWidth = "border-block-width", "css-borders", "<'border-top-width'>{1,2}",
        [BorderBlockStartWidth, BorderBlockEndWidth];
    BorderBlockStyle = "border-block-style", "css-borders", "<'border-top-style'>{1,2}",
        [BorderBlockStartStyle, BorderBlockEndStyle];
    BorderBlockColor = "border-block-color", "css-borders", "<'border-top-color'>{1,2}",
        [BorderBlockStartColor, BorderBlockEndColor];
    BorderInlineWidth = "border-inline-width", "css-borders", "<'border-top-width'>{1,2}",
        [BorderInlineStartWidth, BorderInlineEndWidth];
    BorderInlineStyle = "border-inline-style", "css-borders", "<'border-top-style'>{1,2}",
        [BorderInlineStartStyle, BorderInlineEndStyle];
    BorderInlineColor = "border-inline-color", "css-borders", "<'border-top-color'>{1,2}",
        [BorderInlineStartColor, BorderInlineEndColor];
    BorderTop = "border-top", "css-borders", SIDE,
        [BorderTopWidth, BorderTopStyle, BorderTopColor];
    BorderRight = "border-right", "css-borders", SIDE,
        [BorderRightWidth, BorderRightStyle, BorderRightColor];
    BorderBottom = "border-bottom", "css-borders", SIDE,
        [BorderBottomWidth, BorderBottomStyle, BorderBottomColor];
    BorderLeft = "border-left", "css-borders", SIDE,
        [BorderLeftWidth, BorderLeftStyle, BorderLeftColor];
    BorderBlockStart = "border-block-start", "css-borders", SIDE,
        [BorderBlockStartWidth, BorderBlockStartStyle, BorderBlockStartColor];
    BorderBlockEnd = "border-block-end", "css-borders", SIDE,
        [BorderBlockEndWidth, BorderBlockEndStyle, BorderBlockEndColor];
    BorderInlineStart = "border-inline-start", "css-borders", SIDE,
        [BorderInlineStartWidth, BorderInlineStartStyle, BorderInlineStartColor];
    BorderInlineEnd = "border-inline-end", "css-borders", SIDE,
        [BorderInlineEndWidth, BorderInlineEndStyle, BorderInlineEndColor];
    BorderBlock = "border-block", "css-borders", "<'border-block-start'>",
        [
            BorderBlockStartWidth, BorderBlockStartStyle, BorderBlockStartColor,
            BorderBlockEndWidth, BorderBlockEndStyle, BorderBlockEndColor
        ];
    BorderInline = "border-inline", "css-borders", "<'border-block-start'>",
        [
            BorderInlineStartWidth, BorderInlineStartStyle, BorderInlineStartColor,
            BorderInlineEndWidth, BorderInlineEndStyle, BorderInlineEndColor
        ];
    Border = "border", "css-borders", SIDE,
        [
            BorderTopWidth, BorderRightWidth, BorderBottomWidth, BorderLeftWidth,
            BorderTopStyle, BorderRightStyle, BorderBottomStyle, BorderLeftStyle,
            BorderTopColor, BorderRightColor, BorderBottomColor, BorderLeftColor
        ];
    Outline = "outline", "css-ui", "<'outline-width'> || <'outline-style'> || <'outline-color'>",
        [OutlineWidth, OutlineStyle, OutlineColor];
    Font = "font", "css-fonts",
        "[ [ <'font-style'> || <font-variant-css2> || <'font-weight'> || <font-width-css3> ]? \
         <'font-size'> [ / <'line-height'> ]? <'font-family'># ] | <system-font-family-name>",
        [FontStyle, FontVariantCaps, FontWeight, FontWidth, FontSize, LineHeight, FontFamily];
    TextAlign = "text-align", "css-text-4",
        "start | end | left | right | center | <string> | justify | match-parent | justify-all",
        [TextAlignAll, TextAlignLast];
    WhiteSpace = "white-space", "css-text-4",
        "normal | pre | pre-wrap | pre-line | <'white-space-collapse'> || <'text-wrap-mode'> || \
         <'white-space-trim'>",
        [WhiteSpaceCollapse, TextWrapMode, WhiteSpaceTrim];
    TextWrap = "text-wrap", "css-text-4", "<'text-wrap-mode'> || <'text-wrap-style'>",
        [TextWrapMode, TextWrapStyle];
    TextDecoration = "text-decoration", "css-text-decor-4",
        "<'text-decoration-line'> || <'text-decoration-thickness'> || \
         <'text-decoration-style'> || <'text-decoration-color'>",
        [
            TextDecorationLine, TextDecorationThickness, TextDecorationStyle,
            TextDecorationColor
        ];
    ListStyle = "list-style", "css-lists",
        "<'list-style-position'> || <'list-style-image'> || <'list-style-type'>",
        [ListStylePosition, ListStyleImage, ListStyleType];
    VerticalAlign = "vertical-align", "css-inline",
        "[ first | last] || <'alignment-baseline'> || <'baseline-shift'>",
        [BaselineSource, AlignmentBaseline, BaselineShift], quirky;
    All = "all", "css-cascade", "initial | inherit | unset | revert | revert-layer | revert-rule",
        [];
}

impl Shorthand {
    pub fn name(self) -> &'static str {
        self.info().name
    }

    /// Die Langformen in kanonischer Reihenfolge. `all` steht für alle
    /// außer `direction` und `unicode-bidi` (CSS Cascade 5).
    pub fn longhands(self) -> &'static [Longhand] {
        if self != Shorthand::All {
            return self.info().longhands;
        }
        static ALL: OnceLock<Vec<Longhand>> = OnceLock::new();
        ALL.get_or_init(|| {
            Longhand::ALL
                .iter()
                .copied()
                .filter(|longhand| !matches!(longhand, Longhand::Direction | Longhand::UnicodeBidi))
                .collect()
        })
    }

    /// Die Kurzschreibweisen, zu denen eine Langform gehört, in der
    /// *preferred order* von CSSOM: nach Namen sortiert, dann nach der Zahl
    /// der Langformen, die größte zuerst.
    pub fn containing(longhand: Longhand) -> Vec<Shorthand> {
        let mut shorthands: Vec<Shorthand> = Shorthand::ALL
            .iter()
            .copied()
            .filter(|shorthand| shorthand.longhands().contains(&longhand))
            .collect();
        shorthands.sort_by_key(|shorthand| shorthand.name());
        // `sort_by_key` ist stabil, die Namensfolge bleibt bei gleicher Zahl.
        shorthands.sort_by_key(|shorthand| std::cmp::Reverse(shorthand.longhands().len()));
        shorthands
    }

    /// Zerlegt einen Wert in die Werte der Langformen, in der Reihenfolge
    /// von [`Shorthand::longhands`]. Ob die Eingabe ganz verbraucht ist,
    /// prüft der Aufrufer.
    pub(crate) fn parse_value(self, input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
        use SpecifiedValue as V;
        if self.info().quirky {
            input.allow_quirky_lengths();
        }
        Some(match self {
            Shorthand::BackgroundPosition => background_position(input)?,
            Shorthand::Background => background(input)?,
            Shorthand::Margin => four(
                input,
                LengthPercentageOrAuto::parse,
                V::LengthPercentageOrAuto,
            )?,
            Shorthand::Padding => four(input, non_negative_length_percentage, V::LengthPercentage)?,
            Shorthand::BorderWidth => four(input, LineWidth::parse, V::LineWidth)?,
            Shorthand::BorderStyle => four(input, LineStyle::parse, V::LineStyle)?,
            Shorthand::BorderColor => four(input, Color::parse, V::Color)?,
            Shorthand::MarginBlock | Shorthand::MarginInline => two(
                input,
                LengthPercentageOrAuto::parse,
                V::LengthPercentageOrAuto,
            )?,
            Shorthand::PaddingBlock | Shorthand::PaddingInline => {
                two(input, non_negative_length_percentage, V::LengthPercentage)?
            }
            Shorthand::BorderBlockWidth | Shorthand::BorderInlineWidth => {
                two(input, LineWidth::parse, V::LineWidth)?
            }
            Shorthand::BorderBlockStyle | Shorthand::BorderInlineStyle => {
                two(input, LineStyle::parse, V::LineStyle)?
            }
            Shorthand::BorderBlockColor | Shorthand::BorderInlineColor => {
                two(input, Color::parse, V::Color)?
            }
            Shorthand::BorderTop
            | Shorthand::BorderRight
            | Shorthand::BorderBottom
            | Shorthand::BorderLeft
            | Shorthand::BorderBlockStart
            | Shorthand::BorderBlockEnd
            | Shorthand::BorderInlineStart
            | Shorthand::BorderInlineEnd => border_side(input)?.to_vec(),
            Shorthand::BorderBlock | Shorthand::BorderInline => {
                let side = border_side(input)?;
                side.iter().chain(side.iter()).cloned().collect()
            }
            Shorthand::Border => {
                let [width, style, color] = border_side(input)?;
                [width, style, color]
                    .into_iter()
                    .flat_map(|value| std::iter::repeat_n(value, 4))
                    .collect()
            }
            Shorthand::Outline => outline(input)?,
            Shorthand::Font => font(input)?,
            Shorthand::TextAlign => text_align(input)?,
            Shorthand::WhiteSpace => white_space(input)?,
            Shorthand::TextWrap => text_wrap(input)?,
            Shorthand::TextDecoration => text_decoration(input)?,
            Shorthand::ListStyle => list_style(input)?,
            Shorthand::VerticalAlign => vertical_align(input)?,
            // `all` nimmt nur die CSS-weiten Schlüsselwörter an.
            Shorthand::All => return None,
        })
    }

    /// Serialisiert die Werte der Langformen, in der Reihenfolge von
    /// [`Shorthand::longhands`]. `None`, wenn die Kurzschreibweise sie nicht
    /// genau wiedergeben kann.
    pub(crate) fn serialize(self, values: &[&SpecifiedValue]) -> Option<String> {
        let longhands = self.longhands();
        if values.len() != longhands.len() {
            return None;
        }
        match self {
            Shorthand::BackgroundPosition => match values {
                [
                    SpecifiedValue::BackgroundPositionX(x),
                    SpecifiedValue::BackgroundPositionY(y),
                ] => serialize_background_position(x, y),
                _ => None,
            },
            Shorthand::Background => serialize_background(values),
            Shorthand::Margin
            | Shorthand::Padding
            | Shorthand::BorderWidth
            | Shorthand::BorderStyle
            | Shorthand::BorderColor => match values {
                [top, right, bottom, left] => Some(write_four([top, right, bottom, left])),
                _ => None,
            },
            Shorthand::MarginBlock
            | Shorthand::MarginInline
            | Shorthand::PaddingBlock
            | Shorthand::PaddingInline
            | Shorthand::BorderBlockWidth
            | Shorthand::BorderBlockStyle
            | Shorthand::BorderBlockColor
            | Shorthand::BorderInlineWidth
            | Shorthand::BorderInlineStyle
            | Shorthand::BorderInlineColor => match values {
                [start, end] if start == end => Some(start.to_css_string()),
                [start, end] => Some(format!("{} {}", start.to_css_string(), end.to_css_string())),
                _ => None,
            },
            Shorthand::BorderTop
            | Shorthand::BorderRight
            | Shorthand::BorderBottom
            | Shorthand::BorderLeft
            | Shorthand::BorderBlockStart
            | Shorthand::BorderBlockEnd
            | Shorthand::BorderInlineStart
            | Shorthand::BorderInlineEnd => non_initial(longhands, values, "medium"),
            Shorthand::BorderBlock | Shorthand::BorderInline => {
                let (start, end) = values.split_at_checked(3)?;
                let start_longhands = longhands.get(..3)?;
                (start == end).then(|| non_initial(start_longhands, start, "medium"))?
            }
            Shorthand::Border => {
                // Alle vier Seiten müssen gleich sein.
                let mut sides = Vec::new();
                for chunk in values.chunks(4) {
                    let first = chunk.first()?;
                    if chunk.iter().any(|value| value != first) {
                        return None;
                    }
                    sides.push(*first);
                }
                let side_longhands = [
                    Longhand::BorderTopWidth,
                    Longhand::BorderTopStyle,
                    Longhand::BorderTopColor,
                ];
                non_initial(&side_longhands, &sides, "medium")
            }
            // Gleichen alle Teile dem Anfangswert, lassen die WPT
            // `currentcolor`, `none` oder `medium` zu.
            Shorthand::Outline => non_initial(longhands, values, "none"),
            Shorthand::Font => serialize_font(values),
            Shorthand::TextAlign => serialize_text_align(values),
            Shorthand::WhiteSpace => serialize_white_space(longhands, values),
            Shorthand::TextWrap => non_initial(longhands, values, "wrap"),
            Shorthand::TextDecoration => non_initial(longhands, values, "none"),
            Shorthand::ListStyle => serialize_list_style(values),
            Shorthand::VerticalAlign => non_initial(longhands, values, "baseline"),
            Shorthand::All => None,
        }
    }
}

/// Ein bis vier Werte für oben, rechts, unten und links: Fehlt links, gilt
/// rechts; fehlt unten, gilt oben; fehlt rechts, gilt oben.
fn four<T: Clone>(
    input: &mut Input<'_>,
    parse: impl Fn(&mut Input<'_>) -> Option<T>,
    wrap: impl Fn(T) -> SpecifiedValue,
) -> Option<Vec<SpecifiedValue>> {
    let top = parse(input)?;
    let right = parse(input);
    let bottom = right.as_ref().and_then(|_| parse(input));
    let left = bottom.as_ref().and_then(|_| parse(input));
    let right = right.unwrap_or_else(|| top.clone());
    let bottom = bottom.unwrap_or_else(|| top.clone());
    let left = left.unwrap_or_else(|| right.clone());
    Some([top, right, bottom, left].into_iter().map(wrap).collect())
}

/// Ein oder zwei Werte für Anfang und Ende.
fn two<T: Clone>(
    input: &mut Input<'_>,
    parse: impl Fn(&mut Input<'_>) -> Option<T>,
    wrap: impl Fn(T) -> SpecifiedValue,
) -> Option<Vec<SpecifiedValue>> {
    let start = parse(input)?;
    let end = parse(input).unwrap_or_else(|| start.clone());
    Some(vec![wrap(start), wrap(end)])
}

/// Die kürzeste Form aus CSS 2: so wenige Werte wie möglich.
fn write_four(values: [&&SpecifiedValue; 4]) -> String {
    let [top, right, bottom, left] = values;
    let count = if left != right {
        4
    } else if bottom != top {
        3
    } else if right != top {
        2
    } else {
        1
    };
    [top, right, bottom, left]
        .iter()
        .take(count)
        .map(|value| value.to_css_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Die Teile, die nicht dem Anfangswert ihrer Langform gleichen, durch
/// Leerzeichen getrennt; gleichen alle ihm, `fallback`.
fn non_initial(
    longhands: &[Longhand],
    values: &[&SpecifiedValue],
    fallback: &str,
) -> Option<String> {
    let mut parts = Vec::new();
    for (longhand, value) in longhands.iter().zip(values) {
        if longhand.initial_value().as_ref() != Some(*value) {
            parts.push(value.to_css_string());
        }
    }
    if parts.is_empty() {
        return Some(fallback.to_owned());
    }
    Some(parts.join(" "))
}

type ValueParser<'a> = &'a dyn Fn(&mut Input<'_>) -> Option<SpecifiedValue>;

/// Liest bis zu einen Wert je Langform, in beliebiger Reihenfolge
/// (`a || b || …`). Gibt `None` zurück, wenn gar nichts passt. Jeder
/// Parser darf bei Misserfolg nichts verbrauchen.
fn any_order<const N: usize>(
    input: &mut Input<'_>,
    parsers: [ValueParser<'_>; N],
) -> Option<[Option<SpecifiedValue>; N]> {
    let mut found: [Option<SpecifiedValue>; N] = std::array::from_fn(|_| None);
    'tokens: loop {
        for (slot, parse) in found.iter_mut().zip(parsers.iter()) {
            if slot.is_none()
                && let Some(value) = parse(input)
            {
                *slot = Some(value);
                continue 'tokens;
            }
        }
        break;
    }
    found.iter().any(Option::is_some).then_some(found)
}

/// Füllt fehlende Teile mit den Anfangswerten der Langformen.
fn with_initial(
    longhands: &[Longhand],
    found: impl IntoIterator<Item = Option<SpecifiedValue>>,
) -> Option<Vec<SpecifiedValue>> {
    longhands
        .iter()
        .zip(found)
        .map(|(longhand, value)| value.or_else(|| longhand.initial_value()))
        .collect()
}

/// `<line-width> || <line-style> || <color>` für eine Seite.
fn border_side(input: &mut Input<'_>) -> Option<[SpecifiedValue; 3]> {
    let found = any_order(
        input,
        [
            &|input| LineWidth::parse(input).map(SpecifiedValue::LineWidth),
            &|input| LineStyle::parse(input).map(SpecifiedValue::LineStyle),
            &|input| Color::parse(input).map(SpecifiedValue::Color),
        ],
    )?;
    let longhands = [
        Longhand::BorderTopWidth,
        Longhand::BorderTopStyle,
        Longhand::BorderTopColor,
    ];
    let values = with_initial(&longhands, found)?;
    values.try_into().ok()
}

/// `outline`. Ein `auto` allein oder nur mit einer Breite setzt
/// `outline-style` und `outline-color` auf `auto` (CSS UI 4); da `auto` der
/// Anfangswert von `outline-color` ist, genügt es, das erste `auto` als Stil
/// zu lesen.
fn outline(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    let found = any_order(
        input,
        [
            &|input| LineWidth::parse(input).map(SpecifiedValue::LineWidth),
            &|input| OutlineStyle::parse(input).map(SpecifiedValue::OutlineStyle),
            &|input| OutlineColor::parse(input).map(SpecifiedValue::OutlineColor),
        ],
    )?;
    with_initial(Shorthand::Outline.longhands(), found)
}

/// `font` ohne die Systemschriften: bis zu vier Angaben aus Stil,
/// `small-caps`, Gewicht und Breite, jede höchstens einmal; `normal` zählt
/// für eine beliebige freie. Dann Größe, wahlweise `/ <'line-height'>`, und
/// die Familien.
fn font(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    let mut style = None;
    let mut variant = None;
    let mut weight = None;
    let mut width = None;
    let mut count = 0;
    while count < 4 {
        if input.keyword("normal") {
            // Belegt eine der freien Angaben, welche, ist gleich.
        } else if style.is_none()
            && let Some(value) = FontStyle::parse(input)
        {
            style = Some(value);
        } else if variant.is_none() && input.keyword("small-caps") {
            variant = Some(FontVariantCaps::SmallCaps);
        } else if weight.is_none()
            && let Some(value) = FontWeight::parse(input)
        {
            weight = Some(value);
        } else if width.is_none()
            && let Some(value) = FontWidthKeyword::parse(input)
        {
            width = Some(FontWidth::Keyword(value));
        } else {
            break;
        }
        count += 1;
    }
    let size = FontSize::parse(input)?;
    let line_height = if input.delim('/') {
        LineHeight::parse(input)?
    } else {
        LineHeight::Normal
    };
    let family = FontFamilyList::parse(input)?;
    Some(vec![
        SpecifiedValue::FontStyle(style.unwrap_or(FontStyle::Normal)),
        SpecifiedValue::FontVariantCaps(variant.unwrap_or(FontVariantCaps::Normal)),
        SpecifiedValue::FontWeight(weight.unwrap_or(FontWeight::Normal)),
        SpecifiedValue::FontWidth(width.unwrap_or(FontWidth::Keyword(FontWidthKeyword::Normal))),
        SpecifiedValue::FontSize(size),
        SpecifiedValue::LineHeight(line_height),
        SpecifiedValue::FontFamily(family),
    ])
}

/// `font` kann nur `small-caps` aus `font-variant-caps` und nur die
/// Schlüsselwörter von `font-width` wiedergeben.
fn serialize_font(values: &[&SpecifiedValue]) -> Option<String> {
    let [
        SpecifiedValue::FontStyle(style),
        SpecifiedValue::FontVariantCaps(variant),
        SpecifiedValue::FontWeight(weight),
        SpecifiedValue::FontWidth(width),
        SpecifiedValue::FontSize(size),
        SpecifiedValue::LineHeight(line_height),
        SpecifiedValue::FontFamily(family),
    ] = values
    else {
        return None;
    };
    let mut parts = Vec::new();
    if *style != FontStyle::Normal {
        parts.push(style.to_css_string());
    }
    match variant {
        FontVariantCaps::Normal => {}
        FontVariantCaps::SmallCaps => parts.push(variant.to_css_string()),
        _ => return None,
    }
    if *weight != FontWeight::Normal {
        parts.push(weight.to_css_string());
    }
    match width {
        FontWidth::Keyword(FontWidthKeyword::Normal) => {}
        FontWidth::Keyword(keyword) => parts.push(keyword.to_css_string()),
        FontWidth::Percentage(_) => return None,
    }
    let mut size = size.to_css_string();
    if *line_height != LineHeight::Normal {
        size.push_str(" / ");
        size.push_str(&line_height.to_css_string());
    }
    parts.push(size);
    parts.push(FontFamilyList::to_css_string(family));
    Some(parts.join(" "))
}

/// `text-align`: `justify-all` setzt beide Langformen auf `justify`,
/// `match-parent` beide auf `match-parent`; alle anderen Werte gehen an
/// `text-align-all`, und `text-align-last` wird `auto`.
fn text_align(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    let (all, last) = if input.keyword("justify-all") {
        (
            TextAlignAll::Keyword(TextAlignKeyword::Justify),
            TextAlignLast::Justify,
        )
    } else {
        let all = TextAlignAll::parse(input)?;
        let last = if all == TextAlignAll::Keyword(TextAlignKeyword::MatchParent) {
            TextAlignLast::MatchParent
        } else {
            TextAlignLast::Auto
        };
        (all, last)
    };
    Some(vec![
        SpecifiedValue::TextAlignAll(all),
        SpecifiedValue::TextAlignLast(last),
    ])
}

fn serialize_text_align(values: &[&SpecifiedValue]) -> Option<String> {
    let [
        SpecifiedValue::TextAlignAll(all),
        SpecifiedValue::TextAlignLast(last),
    ] = values
    else {
        return None;
    };
    match (all, last) {
        // `match-parent` setzt beide Langformen; mit `auto` passt es nicht.
        (TextAlignAll::Keyword(TextAlignKeyword::MatchParent), TextAlignLast::Auto) => None,
        (all, TextAlignLast::Auto) => Some(all.to_css_string()),
        (TextAlignAll::Keyword(TextAlignKeyword::Justify), TextAlignLast::Justify) => {
            Some("justify-all".to_owned())
        }
        (TextAlignAll::Keyword(TextAlignKeyword::MatchParent), TextAlignLast::MatchParent) => {
            Some("match-parent".to_owned())
        }
        _ => None,
    }
}

/// Die festen Schlüsselwörter von `white-space` mit ihren Langformen
/// (CSS Text 4, Tabelle in „White Space and Wrapping“).
const WHITE_SPACE_KEYWORDS: [(&str, WhiteSpaceCollapse, TextWrapMode); 4] = [
    ("normal", WhiteSpaceCollapse::Collapse, TextWrapMode::Wrap),
    ("pre", WhiteSpaceCollapse::Preserve, TextWrapMode::Nowrap),
    ("pre-wrap", WhiteSpaceCollapse::Preserve, TextWrapMode::Wrap),
    (
        "pre-line",
        WhiteSpaceCollapse::PreserveBreaks,
        TextWrapMode::Wrap,
    ),
];

fn white_space(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    for (keyword, collapse, mode) in WHITE_SPACE_KEYWORDS {
        if input.keyword(keyword) {
            return Some(vec![
                SpecifiedValue::WhiteSpaceCollapse(collapse),
                SpecifiedValue::TextWrapMode(mode),
                SpecifiedValue::WhiteSpaceTrim(WhiteSpaceTrim::default()),
            ]);
        }
    }
    let found = any_order(
        input,
        [
            &|input| WhiteSpaceCollapse::parse(input).map(SpecifiedValue::WhiteSpaceCollapse),
            &|input| TextWrapMode::parse(input).map(SpecifiedValue::TextWrapMode),
            &|input| WhiteSpaceTrim::parse(input).map(SpecifiedValue::WhiteSpaceTrim),
        ],
    )?;
    with_initial(Shorthand::WhiteSpace.longhands(), found)
}

/// Ein festes Schlüsselwort, wo die Werte eines ergeben, sonst die Teile,
/// die nicht dem Anfangswert gleichen.
fn serialize_white_space(longhands: &[Longhand], values: &[&SpecifiedValue]) -> Option<String> {
    if let [
        SpecifiedValue::WhiteSpaceCollapse(collapse),
        SpecifiedValue::TextWrapMode(mode),
        SpecifiedValue::WhiteSpaceTrim(trim),
    ] = values
        && *trim == WhiteSpaceTrim::default()
        && let Some((keyword, _, _)) =
            WHITE_SPACE_KEYWORDS
                .iter()
                .find(|(_, found_collapse, found_mode)| {
                    found_collapse == collapse && found_mode == mode
                })
    {
        return Some((*keyword).to_owned());
    }
    non_initial(longhands, values, "normal")
}

fn text_wrap(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    let found = any_order(
        input,
        [
            &|input| TextWrapMode::parse(input).map(SpecifiedValue::TextWrapMode),
            &|input| TextWrapStyle::parse(input).map(SpecifiedValue::TextWrapStyle),
        ],
    )?;
    with_initial(Shorthand::TextWrap.longhands(), found)
}

fn text_decoration(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    let found = any_order(
        input,
        [
            &|input| TextDecorationLine::parse(input).map(SpecifiedValue::TextDecorationLine),
            &|input| {
                TextDecorationThickness::parse(input).map(SpecifiedValue::TextDecorationThickness)
            },
            &|input| TextDecorationStyle::parse(input).map(SpecifiedValue::TextDecorationStyle),
            &|input| Color::parse(input).map(SpecifiedValue::Color),
        ],
    )?;
    with_initial(Shorthand::TextDecoration.longhands(), found)
}

/// `list-style`. Ein `none` gilt für das, was von `list-style-image` und
/// `list-style-type` sonst nicht gesetzt ist (CSS Lists 3); ein Bezeichner
/// wie `inside` wird nur dann ein Zählerstil, wenn `list-style-position`
/// schon gesetzt ist (CSS Values 4, „positionally-ambiguous keywords“).
fn list_style(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    let mut position = None;
    let mut image = None;
    let mut list_type = None;
    let mut nones = 0;
    loop {
        if input.keyword("none") {
            nones += 1;
        } else if position.is_none()
            && let Some(value) = ListStylePosition::parse(input)
        {
            position = Some(value);
        } else if image.is_none()
            && let Some(value) = ListStyleImage::parse(input)
        {
            image = Some(value);
        } else if list_type.is_none()
            && let Some(value) = ListStyleType::parse(input)
        {
            list_type = Some(value);
        } else {
            break;
        }
    }
    if nones == 0 && position.is_none() && image.is_none() && list_type.is_none() {
        return None;
    }
    let (image, list_type) = match (nones, image, list_type) {
        (0, image, list_type) => (
            image.unwrap_or(ListStyleImage::None),
            list_type.unwrap_or_else(|| ListStyleType::CounterStyle(ListStyleType::DISC.into())),
        ),
        (1, Some(image), None) => (image, ListStyleType::None),
        (1, None, Some(list_type)) => (ListStyleImage::None, list_type),
        (1 | 2, None, None) => (ListStyleImage::None, ListStyleType::None),
        _ => return None,
    };
    Some(vec![
        SpecifiedValue::ListStylePosition(position.unwrap_or(ListStylePosition::Outside)),
        SpecifiedValue::ListStyleImage(image),
        SpecifiedValue::ListStyleType(list_type),
    ])
}

/// `list-style`: die Position nur, wenn sie nicht `outside` ist oder der
/// Zählerstil `inside` oder `outside` heißt (sonst ginge er beim Lesen an
/// die Position); `none` für Bild und Typ zusammen nur einmal; sonst die
/// Teile, die nicht dem Anfangswert gleichen, und ohne jedes `outside`.
fn serialize_list_style(values: &[&SpecifiedValue]) -> Option<String> {
    let [
        SpecifiedValue::ListStylePosition(position),
        SpecifiedValue::ListStyleImage(image),
        SpecifiedValue::ListStyleType(list_type),
    ] = values
    else {
        return None;
    };
    let mut parts = Vec::new();
    let ambiguous_type = matches!(list_type, ListStyleType::CounterStyle(name)
        if name.eq_ignore_ascii_case("inside") || name.eq_ignore_ascii_case("outside"));
    if *position != ListStylePosition::Outside || ambiguous_type {
        parts.push(position.to_css_string());
    }
    if *image == ListStyleImage::None && *list_type == ListStyleType::None {
        parts.push("none".to_owned());
    } else {
        if *image != ListStyleImage::None {
            parts.push(image.to_css_string());
        }
        if *list_type != ListStyleType::CounterStyle(ListStyleType::DISC.into()) {
            parts.push(list_type.to_css_string());
        }
    }
    if parts.is_empty() {
        return Some("outside".to_owned());
    }
    Some(parts.join(" "))
}

fn vertical_align(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    let found = any_order(
        input,
        [
            &|input| {
                if input.keyword("first") {
                    Some(SpecifiedValue::BaselineSource(BaselineSource::First))
                } else if input.keyword("last") {
                    Some(SpecifiedValue::BaselineSource(BaselineSource::Last))
                } else {
                    None
                }
            },
            &|input| AlignmentBaseline::parse(input).map(SpecifiedValue::AlignmentBaseline),
            &|input| BaselineShift::parse(input).map(SpecifiedValue::BaselineShift),
        ],
    )?;
    with_initial(Shorthand::VerticalAlign.longhands(), found)
}

/// Ein Eintrag aus `background-position-x` und einer aus `-y`.
type Position = (
    PositionComponent<HorizontalEdge>,
    PositionComponent<VerticalEdge>,
);

/// `left`, `center` oder `right`
fn horizontal_keyword(input: &mut Input<'_>) -> Option<PositionComponent<HorizontalEdge>> {
    if input.keyword("center") {
        Some(PositionComponent::Center)
    } else if input.keyword("left") {
        Some(PositionComponent::Edge(HorizontalEdge::Left))
    } else if input.keyword("right") {
        Some(PositionComponent::Edge(HorizontalEdge::Right))
    } else {
        None
    }
}

/// `top`, `center` oder `bottom`
fn vertical_keyword(input: &mut Input<'_>) -> Option<PositionComponent<VerticalEdge>> {
    if input.keyword("center") {
        Some(PositionComponent::Center)
    } else if input.keyword("top") {
        Some(PositionComponent::Edge(VerticalEdge::Top))
    } else if input.keyword("bottom") {
        Some(PositionComponent::Edge(VerticalEdge::Bottom))
    } else {
        None
    }
}

/// `[ left | right ] <length-percentage>`
fn horizontal_edge_offset(input: &mut Input<'_>) -> Option<PositionComponent<HorizontalEdge>> {
    input.try_parse(|input| {
        let PositionComponent::Edge(edge) = horizontal_keyword(input)? else {
            return None;
        };
        Some(PositionComponent::EdgeOffset(
            edge,
            LengthPercentage::parse(input)?,
        ))
    })
}

/// `[ top | bottom ] <length-percentage>`
fn vertical_edge_offset(input: &mut Input<'_>) -> Option<PositionComponent<VerticalEdge>> {
    input.try_parse(|input| {
        let PositionComponent::Edge(edge) = vertical_keyword(input)? else {
            return None;
        };
        Some(PositionComponent::EdgeOffset(
            edge,
            LengthPercentage::parse(input)?,
        ))
    })
}

fn offset<E>(input: &mut Input<'_>) -> Option<PositionComponent<E>> {
    LengthPercentage::parse(input).map(PositionComponent::Offset)
}

/// Ein ⟨bg-position⟩: ⟨position⟩ aus Values 4 oder ⟨position-three⟩,
/// gierig gelesen, also die längste Form zuerst. Die Schlüsselwörter nach
/// der Schreibrichtung, die Values 5 in ⟨position⟩ aufnimmt, fehlen: Wie
/// sie die Langformen setzen, ist offen (w3c/csswg-drafts#9690).
fn bg_position(input: &mut Input<'_>) -> Option<Position> {
    // Vier Werte
    input
        .try_parse(|input| Some((horizontal_edge_offset(input)?, vertical_edge_offset(input)?)))
        .or_else(|| {
            input.try_parse(|input| {
                let y = vertical_edge_offset(input)?;
                Some((horizontal_edge_offset(input)?, y))
            })
        })
        // Drei Werte
        .or_else(|| {
            input
                .try_parse(|input| Some((horizontal_edge_offset(input)?, vertical_keyword(input)?)))
        })
        .or_else(|| {
            input.try_parse(|input| {
                let y = vertical_edge_offset(input)?;
                Some((horizontal_keyword(input)?, y))
            })
        })
        .or_else(|| {
            input
                .try_parse(|input| Some((horizontal_keyword(input)?, vertical_edge_offset(input)?)))
        })
        .or_else(|| {
            input.try_parse(|input| {
                let y = vertical_keyword(input)?;
                Some((horizontal_edge_offset(input)?, y))
            })
        })
        // Zwei Werte
        .or_else(|| {
            input.try_parse(|input| {
                let x = horizontal_keyword(input).or_else(|| offset(input))?;
                let y = vertical_keyword(input).or_else(|| offset(input))?;
                Some((x, y))
            })
        })
        .or_else(|| {
            input.try_parse(|input| {
                let y = vertical_keyword(input)?;
                Some((horizontal_keyword(input)?, y))
            })
        })
        // Ein Wert, die andere Achse ist `center`.
        .or_else(|| {
            if let Some(x) = horizontal_keyword(input) {
                return Some((x, PositionComponent::Center));
            }
            if let Some(y) = vertical_keyword(input) {
                return Some((PositionComponent::Center, y));
            }
            Some((offset(input)?, PositionComponent::Center))
        })
}

fn background_position(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    loop {
        let (x, y) = bg_position(input)?;
        xs.push(x);
        ys.push(y);
        if !input.comma() {
            break;
        }
    }
    Some(vec![
        SpecifiedValue::BackgroundPositionX(Layers(xs)),
        SpecifiedValue::BackgroundPositionY(Layers(ys)),
    ])
}

/// Ein ⟨bg-position⟩, waagrecht zuerst (Values 4, „Serializing
/// <position>“). Steht ein Abstand von einer Kante neben einem ohne, bekommt
/// dieser seine Kante `left` oder `top` dazu, denn nur die Form mit vier
/// Teilen erlaubt Abstände an beiden Achsen. `None` für die Kanten nach der
/// Schreibrichtung.
fn write_bg_position(
    x: &PositionComponent<HorizontalEdge>,
    y: &PositionComponent<VerticalEdge>,
) -> Option<String> {
    use PositionComponent as P;
    let flow = |side: Option<EdgeSide>| side == Some(EdgeSide::Flow);
    let x_side = match x {
        P::Edge(edge) | P::EdgeOffset(edge, _) => Some(edge.side()),
        _ => None,
    };
    let y_side = match y {
        P::Edge(edge) | P::EdgeOffset(edge, _) => Some(edge.side()),
        _ => None,
    };
    if flow(x_side) || flow(y_side) {
        return None;
    }
    Some(match (x, y) {
        (P::EdgeOffset(..), P::Offset(offset)) => {
            format!("{} top {}", x.to_css_string(), offset.to_css_string())
        }
        (P::Offset(offset), P::EdgeOffset(..)) => {
            format!("left {} {}", offset.to_css_string(), y.to_css_string())
        }
        _ => format!("{} {}", x.to_css_string(), y.to_css_string()),
    })
}

fn serialize_background_position(
    x: &Layers<PositionComponent<HorizontalEdge>>,
    y: &Layers<PositionComponent<VerticalEdge>>,
) -> Option<String> {
    if x.0.len() != y.0.len() {
        return None;
    }
    let layers =
        x.0.iter()
            .zip(&y.0)
            .map(|(x, y)| write_bg_position(x, y))
            .collect::<Option<Vec<_>>>()?;
    Some(layers.join(", "))
}

/// Die Anfangswerte von `background-position-x` und `-y`.
const INITIAL_POSITION: Position = (
    PositionComponent::Offset(LengthPercentage::Percentage(0.0)),
    PositionComponent::Offset(LengthPercentage::Percentage(0.0)),
);

/// Eine Ebene von `background`, mit den Anfangswerten für das Fehlende.
struct BgLayer {
    image: BgImage,
    position: Position,
    size: BgSize,
    repeat: RepeatStyle,
    attachment: Attachment,
    origin: VisualBox,
    clip: BgClip,
    color: Option<Color>,
}

/// ⟨bg-layer⟩ oder, mit Farbe, ⟨final-bg-layer⟩. Ein ⟨visual-box⟩ allein
/// setzt `background-origin` und `background-clip`, zwei setzen sie
/// nacheinander (Backgrounds 4). Ein Wert, den nur `background-clip` kennt,
/// setzt `background-origin` auf `border-box`; steht ein ⟨visual-box⟩
/// daneben, ist der `background-origin`, wie es die WPT erwarten.
fn bg_layer(input: &mut Input<'_>) -> Option<BgLayer> {
    let mut image = None;
    let mut position = None;
    let mut repeat = None;
    let mut attachment = None;
    let mut visual_box = None;
    let mut clip = None;
    let mut color = None;
    loop {
        if image.is_none()
            && let Some(value) = BgImage::parse(input)
        {
            image = Some(value);
        } else if position.is_none()
            && let Some(value) = bg_position(input)
        {
            // `/ <bg-size>` nur gleich nach der Position
            let size = if input.delim('/') {
                Some(BgSize::parse(input)?)
            } else {
                None
            };
            position = Some((value, size));
        } else if repeat.is_none()
            && let Some(value) = RepeatStyle::parse(input)
        {
            repeat = Some(value);
        } else if attachment.is_none()
            && let Some(value) = Attachment::parse(input)
        {
            attachment = Some(value);
        } else if clip.is_none()
            && let Some(value) = BgClip::parse_area(input)
        {
            clip = Some(value);
        } else if (visual_box.is_none() || clip.is_none())
            && let Some(value) = VisualBox::parse(input)
        {
            if visual_box.is_none() {
                visual_box = Some(value);
            } else {
                clip = Some(BgClip::Box(value));
            }
        } else if color.is_none()
            && let Some(value) = Color::parse(input)
        {
            color = Some(value);
        } else {
            break;
        }
    }
    let found = image.is_some()
        || position.is_some()
        || repeat.is_some()
        || attachment.is_some()
        || visual_box.is_some()
        || clip.is_some()
        || color.is_some();
    if !found {
        return None;
    }
    let (origin, clip) = match (visual_box, clip) {
        (Some(origin), Some(clip)) => (origin, clip),
        (Some(visual_box), None) => (visual_box, BgClip::Box(visual_box)),
        (None, Some(clip)) => (VisualBox::BorderBox, clip),
        (None, None) => (VisualBox::PaddingBox, BgClip::Box(VisualBox::BorderBox)),
    };
    let (position, size) = position.unwrap_or((INITIAL_POSITION, None));
    Some(BgLayer {
        image: image.unwrap_or(BgImage::None),
        position,
        size: size.unwrap_or(BgSize::AUTO),
        repeat: repeat.unwrap_or(RepeatStyle::REPEAT),
        attachment: attachment.unwrap_or(Attachment::Scroll),
        origin,
        clip,
        color,
    })
}

/// `background`: Ebenen durch Kommas getrennt, eine Farbe nur in der
/// letzten.
fn background(input: &mut Input<'_>) -> Option<Vec<SpecifiedValue>> {
    let mut layers = Vec::new();
    let color = loop {
        let layer = bg_layer(input)?;
        let color = layer.color;
        layers.push(layer);
        if !input.comma() {
            break color;
        }
        if color.is_some() {
            return None;
        }
    };
    let color = match color {
        Some(color) => SpecifiedValue::Color(color),
        None => Longhand::BackgroundColor.initial_value()?,
    };
    Some(vec![
        SpecifiedValue::BackgroundImage(Layers(
            layers.iter().map(|layer| layer.image.clone()).collect(),
        )),
        SpecifiedValue::BackgroundPositionX(Layers(
            layers.iter().map(|layer| layer.position.0).collect(),
        )),
        SpecifiedValue::BackgroundPositionY(Layers(
            layers.iter().map(|layer| layer.position.1).collect(),
        )),
        SpecifiedValue::BackgroundSize(Layers(layers.iter().map(|layer| layer.size).collect())),
        SpecifiedValue::BackgroundRepeat(Layers(layers.iter().map(|layer| layer.repeat).collect())),
        SpecifiedValue::BackgroundAttachment(Layers(
            layers.iter().map(|layer| layer.attachment).collect(),
        )),
        SpecifiedValue::BackgroundOrigin(Layers(layers.iter().map(|layer| layer.origin).collect())),
        SpecifiedValue::BackgroundClip(Layers(layers.iter().map(|layer| layer.clip).collect())),
        color,
    ])
}

/// `background` in der Reihenfolge der Grammatik, wie Blink: Bild,
/// Position mit Größe, Wiederholung, `background-attachment`,
/// `background-origin` und `background-clip`, in der letzten Ebene die
/// Farbe. Was dem Anfangswert gleicht, fällt weg, die Position aber nur
/// ohne Größe; eine Ebene ganz aus Anfangswerten ist `none`. Alle Listen
/// müssen gleich lang sein.
fn serialize_background(values: &[&SpecifiedValue]) -> Option<String> {
    use SpecifiedValue as V;
    let [
        V::BackgroundImage(images),
        V::BackgroundPositionX(xs),
        V::BackgroundPositionY(ys),
        V::BackgroundSize(sizes),
        V::BackgroundRepeat(repeats),
        V::BackgroundAttachment(attachments),
        V::BackgroundOrigin(origins),
        V::BackgroundClip(clips),
        color,
    ] = values
    else {
        return None;
    };
    let count = images.0.len();
    let lengths = [
        xs.0.len(),
        ys.0.len(),
        sizes.0.len(),
        repeats.0.len(),
        attachments.0.len(),
        origins.0.len(),
        clips.0.len(),
    ];
    if count == 0 || lengths.iter().any(|&length| length != count) {
        return None;
    }
    let initial_color = Longhand::BackgroundColor.initial_value();
    let mut layers = Vec::with_capacity(count);
    for index in 0..count {
        let mut parts = Vec::new();
        let image = images.0.get(index)?;
        if *image != BgImage::None {
            parts.push(image.to_css_string());
        }
        let x = xs.0.get(index)?;
        let y = ys.0.get(index)?;
        let size = sizes.0.get(index)?;
        if (*x, *y) != INITIAL_POSITION || *size != BgSize::AUTO {
            let mut position = write_bg_position(x, y)?;
            if *size != BgSize::AUTO {
                position.push_str(" / ");
                size.to_css(&mut position);
            }
            parts.push(position);
        }
        let repeat = repeats.0.get(index)?;
        if *repeat != RepeatStyle::REPEAT {
            parts.push(repeat.to_css_string());
        }
        let attachment = attachments.0.get(index)?;
        if *attachment != Attachment::Scroll {
            parts.push(attachment.to_css_string());
        }
        match (*origins.0.get(index)?, *clips.0.get(index)?) {
            (VisualBox::PaddingBox, BgClip::Box(VisualBox::BorderBox)) => {}
            (origin, BgClip::Box(clip)) if origin == clip => parts.push(origin.to_css_string()),
            (VisualBox::BorderBox, clip @ BgClip::Area { .. }) => parts.push(clip.to_css_string()),
            (origin, clip) => {
                parts.push(origin.to_css_string());
                parts.push(clip.to_css_string());
            }
        }
        if index.saturating_add(1) == count && initial_color.as_ref() != Some(*color) {
            parts.push(color.to_css_string());
        }
        if parts.is_empty() {
            layers.push("none".to_owned());
        } else {
            layers.push(parts.join(" "));
        }
    }
    Some(layers.join(", "))
}
