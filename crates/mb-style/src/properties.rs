//! Eigenschaften: Langformen (*longhand properties*) und Kurzschreibweisen
//! (*shorthand properties*) mit ihren Grammatiken, ihren deklarierten Werten
//! und deren Serialisierung nach CSSOM.
//!
//! Welche Definition gilt, entscheidet webref (die von w3c/webref
//! ausgelesenen Editor's Drafts): die der höchsten Stufe einer
//! Spezifikation, die einen Wert angibt, und von dort auch die Werttypen.
//! [`LonghandInfo`] und [`ShorthandInfo`] nennen Datei, Grammatik und
//! Anfangswert, ein Test vergleicht sie mit webref.
//!
//! Zu jeder Langform gehört ein Werttyp aus `properties/longhands.rs`, das
//! Parsen und Serialisieren der Kurzschreibweisen steht in
//! `properties/shorthands.rs`. Außer den CSS-weiten Schlüsselwörtern fehlen
//! noch `var()` und die mathematischen Funktionen (siehe [`crate::values`])
//! sowie die Werte, die die Kommentare der Typen nennen.

mod logical;
mod longhands;
mod shorthands;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod webref;
#[cfg(test)]
mod wpt;

pub use logical::FlowMode;
pub use longhands::{
    AbsoluteSize, AlignmentBaseline, Attachment, BaselineShift, BaselineSource, BgClip, BgImage,
    BgSize, BoxSize, BoxSizing, DecorationLines, Direction, Display, DisplayInside,
    DisplayInternal, DisplayOutside, EdgeSide, FontFamily, FontFamilyList, FontSize, FontStyle,
    FontVariantCaps, FontWeight, FontWidth, FontWidthKeyword, GenericFamily, HorizontalEdge,
    Layers, LengthPercentageOrAuto, LineHeight, LineStyle, LineWidth, LineWidthKeyword,
    ListStyleImage, ListStylePosition, ListStyleType, MaxSize, PositionComponent, PositionEdge,
    RelativeSize, RepeatStyle, Repetition, ScriptGenericFamily, Size, SizeKeyword, Spacing,
    TextAlignAll, TextAlignKeyword, TextAlignLast, TextDecorationLine, TextDecorationStyle,
    TextDecorationThickness, TextIndent, TextOrientation, TextTransform, TextTransformCase,
    TextWrapMode, TextWrapStyle, UnicodeBidi, VerticalEdge, Visibility, VisualBox,
    WhiteSpaceCollapse, WhiteSpaceTrim, WritingMode,
};
pub use shorthands::{Shorthand, ShorthandInfo};

use mb_css::{Source, Values, parse_component_value_list};

use crate::values::color::Color;
use crate::values::{
    Input, LengthPercentage, Parse, ParseContext, ToCss, keywords, non_negative_length_percentage,
};

keywords! {
    /// Die CSS-weiten Schlüsselwörter (CSS Cascade 5), die jede Eigenschaft
    /// annimmt.
    pub enum CssWideKeyword {
        Initial = "initial",
        Inherit = "inherit",
        Unset = "unset",
        Revert = "revert",
        RevertLayer = "revert-layer",
        RevertRule = "revert-rule",
    }
}

/// Eine *logical property group* (CSS Logical 1): Eigenschaften, die je nach
/// Schreibmodus dieselbe Seite oder Größe meinen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogicalGroup {
    Margin,
    Padding,
    BorderWidth,
    BorderStyle,
    BorderColor,
    Size,
    MinSize,
    MaxSize,
    BackgroundPosition,
}

impl LogicalGroup {
    /// Der Name, wie ihn die Spezifikationen verwenden.
    pub fn name(self) -> &'static str {
        match self {
            LogicalGroup::Margin => "margin",
            LogicalGroup::Padding => "padding",
            LogicalGroup::BorderWidth => "border-width",
            LogicalGroup::BorderStyle => "border-style",
            LogicalGroup::BorderColor => "border-color",
            LogicalGroup::Size => "size",
            LogicalGroup::MinSize => "min-size",
            LogicalGroup::MaxSize => "max-size",
            LogicalGroup::BackgroundPosition => "background-position",
        }
    }
}

/// Die *mapping logic* einer Eigenschaft in ihrer Gruppe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mapping {
    Physical,
    Logical,
}

/// Die Definition einer Langform.
#[derive(Debug)]
pub struct LonghandInfo {
    pub name: &'static str,
    /// Die Datei in webref (`ed/css/<spec>.json`), deren Definition gilt.
    pub spec: &'static str,
    /// Die Grammatik, wie sie dort steht.
    pub syntax: &'static str,
    /// Der Anfangswert, wie er dort steht.
    pub initial: &'static str,
    pub inherited: bool,
    pub group: Option<(LogicalGroup, Mapping)>,
    /// Ob ⟨quirky-length⟩s gelten (CSS Values 4, Anhang C).
    pub quirky: bool,
}

const fn info(
    name: &'static str,
    spec: &'static str,
    syntax: &'static str,
    initial: &'static str,
    inherited: bool,
) -> LonghandInfo {
    LonghandInfo {
        name,
        spec,
        syntax,
        initial,
        inherited,
        group: None,
        quirky: false,
    }
}

impl LonghandInfo {
    const fn physical(self, group: LogicalGroup) -> LonghandInfo {
        LonghandInfo {
            group: Some((group, Mapping::Physical)),
            ..self
        }
    }

    const fn logical(self, group: LogicalGroup) -> LonghandInfo {
        LonghandInfo {
            group: Some((group, Mapping::Logical)),
            ..self
        }
    }

    const fn quirky(self) -> LonghandInfo {
        LonghandInfo {
            quirky: true,
            ..self
        }
    }
}

/// Legt [`Longhand`] mit einer [`LonghandInfo`] je Eigenschaft an.
macro_rules! longhands {
    ($($variant:ident = $info:expr,)*) => {
        /// Eine Langform.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Longhand {
            $($variant,)*
        }

        impl Longhand {
            /// Alle Langformen.
            pub const ALL: &'static [Longhand] = &[$(Longhand::$variant,)*];

            pub fn info(self) -> &'static LonghandInfo {
                match self {
                    $(Longhand::$variant => {
                        const INFO: LonghandInfo = $info;
                        &INFO
                    })*
                }
            }
        }
    };
}

const MARGIN: &str = "<length-percentage> | auto";
const PADDING: &str = "<length-percentage [0,∞]>";
const SIZE: &str = "auto | <box-size>";
const MAX_SIZE: &str = "none | <box-size>";
const BORDER_COLOR: &str = "<color> | <image-1D>";
const BACKGROUND_POSITION_X: &str =
    "[ center | [ [ left | right | x-start | x-end ]? <length-percentage>? ]! ]#";
const BACKGROUND_POSITION_Y: &str =
    "[ center | [ [ top | bottom | y-start | y-end ]? <length-percentage>? ]! ]#";

longhands! {
    Display = info(
        "display",
        "css-display",
        "[ <display-outside> || <display-inside> ] | <display-listitem> | <display-internal> | \
         <display-box> | <display-legacy>",
        "inline",
        false,
    ),
    Visibility = info(
        "visibility",
        "css-display",
        "visible | hidden | force-hidden | collapse",
        "visible",
        true,
    ),
    Color = info("color", "css-color", "<color>", "CanvasText", true),
    BackgroundColor = info("background-color", "css-backgrounds-4", "<color>", "transparent", false),
    BackgroundImage = info("background-image", "css-backgrounds-4", "<bg-image>#", "none", false),
    BackgroundPositionX = info(
        "background-position-x",
        "css-backgrounds-4",
        BACKGROUND_POSITION_X,
        "0%",
        false,
    )
    .physical(LogicalGroup::BackgroundPosition),
    BackgroundPositionY = info(
        "background-position-y",
        "css-backgrounds-4",
        BACKGROUND_POSITION_Y,
        "0%",
        false,
    )
    .physical(LogicalGroup::BackgroundPosition),
    BackgroundSize = info("background-size", "css-backgrounds-4", "<bg-size>#", "auto", false),
    BackgroundRepeat = info(
        "background-repeat",
        "css-backgrounds-4",
        "<repeat-style>#",
        "repeat",
        false,
    ),
    BackgroundAttachment = info(
        "background-attachment",
        "css-backgrounds-4",
        "<attachment>#",
        "scroll",
        false,
    ),
    BackgroundOrigin = info(
        "background-origin",
        "css-backgrounds-4",
        "<visual-box>#",
        "padding-box",
        false,
    ),
    BackgroundClip = info("background-clip", "css-backgrounds-4", "<bg-clip>#", "border-box", false),
    MarginTop = info("margin-top", "css-box", MARGIN, "0", false)
        .physical(LogicalGroup::Margin)
        .quirky(),
    MarginRight = info("margin-right", "css-box", MARGIN, "0", false)
        .physical(LogicalGroup::Margin)
        .quirky(),
    MarginBottom = info("margin-bottom", "css-box", MARGIN, "0", false)
        .physical(LogicalGroup::Margin)
        .quirky(),
    MarginLeft = info("margin-left", "css-box", MARGIN, "0", false)
        .physical(LogicalGroup::Margin)
        .quirky(),
    MarginBlockStart = info("margin-block-start", "css-logical", "<'margin-top'>", "0", false)
        .logical(LogicalGroup::Margin),
    MarginBlockEnd = info("margin-block-end", "css-logical", "<'margin-top'>", "0", false)
        .logical(LogicalGroup::Margin),
    MarginInlineStart = info("margin-inline-start", "css-logical", "<'margin-top'>", "0", false)
        .logical(LogicalGroup::Margin),
    MarginInlineEnd = info("margin-inline-end", "css-logical", "<'margin-top'>", "0", false)
        .logical(LogicalGroup::Margin),
    PaddingTop = info("padding-top", "css-box", PADDING, "0", false)
        .physical(LogicalGroup::Padding)
        .quirky(),
    PaddingRight = info("padding-right", "css-box", PADDING, "0", false)
        .physical(LogicalGroup::Padding)
        .quirky(),
    PaddingBottom = info("padding-bottom", "css-box", PADDING, "0", false)
        .physical(LogicalGroup::Padding)
        .quirky(),
    PaddingLeft = info("padding-left", "css-box", PADDING, "0", false)
        .physical(LogicalGroup::Padding)
        .quirky(),
    PaddingBlockStart = info("padding-block-start", "css-logical", "<'padding-top'>", "0", false)
        .logical(LogicalGroup::Padding),
    PaddingBlockEnd = info("padding-block-end", "css-logical", "<'padding-top'>", "0", false)
        .logical(LogicalGroup::Padding),
    PaddingInlineStart = info("padding-inline-start", "css-logical", "<'padding-top'>", "0", false)
        .logical(LogicalGroup::Padding),
    PaddingInlineEnd = info("padding-inline-end", "css-logical", "<'padding-top'>", "0", false)
        .logical(LogicalGroup::Padding),
    BorderTopWidth = info("border-top-width", "css-borders", "<line-width>", "medium", false)
        .physical(LogicalGroup::BorderWidth)
        .quirky(),
    BorderRightWidth = info("border-right-width", "css-borders", "<line-width>", "medium", false)
        .physical(LogicalGroup::BorderWidth)
        .quirky(),
    BorderBottomWidth = info("border-bottom-width", "css-borders", "<line-width>", "medium", false)
        .physical(LogicalGroup::BorderWidth)
        .quirky(),
    BorderLeftWidth = info("border-left-width", "css-borders", "<line-width>", "medium", false)
        .physical(LogicalGroup::BorderWidth)
        .quirky(),
    BorderBlockStartWidth = info(
        "border-block-start-width",
        "css-borders",
        "<line-width>",
        "medium",
        false,
    )
    .logical(LogicalGroup::BorderWidth),
    BorderBlockEndWidth = info(
        "border-block-end-width",
        "css-borders",
        "<line-width>",
        "medium",
        false,
    )
    .logical(LogicalGroup::BorderWidth),
    BorderInlineStartWidth = info(
        "border-inline-start-width",
        "css-borders",
        "<line-width>",
        "medium",
        false,
    )
    .logical(LogicalGroup::BorderWidth),
    BorderInlineEndWidth = info(
        "border-inline-end-width",
        "css-borders",
        "<line-width>",
        "medium",
        false,
    )
    .logical(LogicalGroup::BorderWidth),
    BorderTopStyle = info("border-top-style", "css-borders", "<line-style>", "none", false)
        .physical(LogicalGroup::BorderStyle),
    BorderRightStyle = info("border-right-style", "css-borders", "<line-style>", "none", false)
        .physical(LogicalGroup::BorderStyle),
    BorderBottomStyle = info("border-bottom-style", "css-borders", "<line-style>", "none", false)
        .physical(LogicalGroup::BorderStyle),
    BorderLeftStyle = info("border-left-style", "css-borders", "<line-style>", "none", false)
        .physical(LogicalGroup::BorderStyle),
    BorderBlockStartStyle = info(
        "border-block-start-style",
        "css-borders",
        "<line-style>",
        "none",
        false,
    )
    .logical(LogicalGroup::BorderStyle),
    BorderBlockEndStyle = info(
        "border-block-end-style",
        "css-borders",
        "<line-style>",
        "none",
        false,
    )
    .logical(LogicalGroup::BorderStyle),
    BorderInlineStartStyle = info(
        "border-inline-start-style",
        "css-borders",
        "<line-style>",
        "none",
        false,
    )
    .logical(LogicalGroup::BorderStyle),
    BorderInlineEndStyle = info(
        "border-inline-end-style",
        "css-borders",
        "<line-style>",
        "none",
        false,
    )
    .logical(LogicalGroup::BorderStyle),
    BorderTopColor = info("border-top-color", "css-borders", BORDER_COLOR, "currentcolor", false)
        .physical(LogicalGroup::BorderColor),
    BorderRightColor = info(
        "border-right-color",
        "css-borders",
        BORDER_COLOR,
        "currentcolor",
        false,
    )
    .physical(LogicalGroup::BorderColor),
    BorderBottomColor = info(
        "border-bottom-color",
        "css-borders",
        BORDER_COLOR,
        "currentcolor",
        false,
    )
    .physical(LogicalGroup::BorderColor),
    BorderLeftColor = info("border-left-color", "css-borders", BORDER_COLOR, "currentcolor", false)
        .physical(LogicalGroup::BorderColor),
    BorderBlockStartColor = info(
        "border-block-start-color",
        "css-borders",
        BORDER_COLOR,
        "currentcolor",
        false,
    )
    .logical(LogicalGroup::BorderColor),
    BorderBlockEndColor = info(
        "border-block-end-color",
        "css-borders",
        BORDER_COLOR,
        "currentcolor",
        false,
    )
    .logical(LogicalGroup::BorderColor),
    BorderInlineStartColor = info(
        "border-inline-start-color",
        "css-borders",
        BORDER_COLOR,
        "currentcolor",
        false,
    )
    .logical(LogicalGroup::BorderColor),
    BorderInlineEndColor = info(
        "border-inline-end-color",
        "css-borders",
        BORDER_COLOR,
        "currentcolor",
        false,
    )
    .logical(LogicalGroup::BorderColor),
    Width = info("width", "css-sizing", SIZE, "auto", false)
        .physical(LogicalGroup::Size)
        .quirky(),
    Height = info("height", "css-sizing", SIZE, "auto", false)
        .physical(LogicalGroup::Size)
        .quirky(),
    MinWidth = info("min-width", "css-sizing", SIZE, "auto", false)
        .physical(LogicalGroup::MinSize)
        .quirky(),
    MinHeight = info("min-height", "css-sizing", SIZE, "auto", false)
        .physical(LogicalGroup::MinSize)
        .quirky(),
    MaxWidth = info("max-width", "css-sizing", MAX_SIZE, "none", false)
        .physical(LogicalGroup::MaxSize)
        .quirky(),
    MaxHeight = info("max-height", "css-sizing", MAX_SIZE, "none", false)
        .physical(LogicalGroup::MaxSize)
        .quirky(),
    InlineSize = info("inline-size", "css-sizing", SIZE, "auto", false)
        .logical(LogicalGroup::Size),
    BlockSize = info("block-size", "css-sizing", SIZE, "auto", false)
        .logical(LogicalGroup::Size),
    MinInlineSize = info("min-inline-size", "css-sizing", SIZE, "auto", false)
        .logical(LogicalGroup::MinSize),
    MinBlockSize = info("min-block-size", "css-sizing", SIZE, "auto", false)
        .logical(LogicalGroup::MinSize),
    MaxInlineSize = info("max-inline-size", "css-sizing", MAX_SIZE, "none", false)
        .logical(LogicalGroup::MaxSize),
    MaxBlockSize = info("max-block-size", "css-sizing", MAX_SIZE, "none", false)
        .logical(LogicalGroup::MaxSize),
    BoxSizing = info(
        "box-sizing",
        "css-sizing",
        "content-box | border-box",
        "content-box",
        false,
    ),
    FontFamily = info(
        "font-family",
        "css-fonts",
        "[ <font-family-name> | <generic-font-family> ]#",
        "depends on user agent",
        true,
    ),
    FontSize = info(
        "font-size",
        "css-fonts",
        "<absolute-size> | <relative-size> | <length-percentage [0,∞]> | math",
        "medium",
        true,
    )
    .quirky(),
    FontWeight = info(
        "font-weight",
        "css-fonts",
        "<font-weight-absolute> | bolder | lighter",
        "normal",
        true,
    ),
    FontStyle = info(
        "font-style",
        "css-fonts",
        "normal | italic | left | right | oblique <angle [-90deg,90deg]>?",
        "normal",
        true,
    ),
    FontWidth = info(
        "font-width",
        "css-fonts",
        "normal | <percentage [0,∞]> | ultra-condensed | extra-condensed | condensed | \
         semi-condensed | semi-expanded | expanded | extra-expanded | ultra-expanded",
        "normal",
        true,
    ),
    FontVariantCaps = info(
        "font-variant-caps",
        "css-fonts",
        "normal | small-caps | all-small-caps | petite-caps | all-petite-caps | unicase | \
         titling-caps",
        "normal",
        true,
    ),
    LineHeight = info(
        "line-height",
        "css-inline",
        "normal | <number [0,∞]> | <length-percentage [0,∞]>",
        "normal",
        true,
    ),
    TextAlignAll = info(
        "text-align-all",
        "css-text-4",
        "start | end | left | right | center | <string> | justify | match-parent",
        "start",
        true,
    ),
    TextAlignLast = info(
        "text-align-last",
        "css-text-4",
        "auto | start | end | left | right | center | justify | match-parent",
        "auto",
        true,
    ),
    TextIndent = info(
        "text-indent",
        "css-text-4",
        "[ <length-percentage> ] && hanging? && each-line?",
        "0",
        true,
    )
    .quirky(),
    TextTransform = info(
        "text-transform",
        "css-text-4",
        "none | [capitalize | uppercase | lowercase ] || full-width || full-size-kana | math-auto",
        "none",
        true,
    ),
    WhiteSpaceCollapse = info(
        "white-space-collapse",
        "css-text-4",
        "collapse | discard | preserve | preserve-breaks | preserve-spaces | break-spaces",
        "collapse",
        true,
    ),
    TextWrapMode = info("text-wrap-mode", "css-text-4", "wrap | nowrap", "wrap", true),
    WhiteSpaceTrim = info(
        "white-space-trim",
        "css-text-4",
        "none | discard-before || discard-after || discard-inner",
        "none",
        false,
    ),
    TextWrapStyle = info(
        "text-wrap-style",
        "css-text-4",
        "auto | balance | stable | pretty | avoid-short-last-line",
        "auto",
        true,
    ),
    LetterSpacing = info(
        "letter-spacing",
        "css-text-4",
        "normal | <length-percentage>",
        "normal",
        true,
    )
    .quirky(),
    WordSpacing = info(
        "word-spacing",
        "css-text-4",
        "normal | <length-percentage>",
        "normal",
        true,
    )
    .quirky(),
    TextDecorationLine = info(
        "text-decoration-line",
        "css-text-decor-4",
        "none | [ underline || overline || line-through || blink ] | spelling-error | \
         grammar-error",
        "none",
        false,
    ),
    TextDecorationStyle = info(
        "text-decoration-style",
        "css-text-decor-4",
        "solid | double | dotted | dashed | wavy",
        "solid",
        false,
    ),
    TextDecorationColor = info(
        "text-decoration-color",
        "css-text-decor-4",
        "<color>",
        "currentcolor",
        false,
    ),
    TextDecorationThickness = info(
        "text-decoration-thickness",
        "css-text-decor-4",
        "auto | from-font | <length-percentage> | <line-width>",
        "auto",
        false,
    ),
    ListStyleType = info(
        "list-style-type",
        "css-lists",
        "<counter-style> | <string> | none",
        "disc",
        true,
    ),
    ListStylePosition = info(
        "list-style-position",
        "css-lists",
        "inside | outside",
        "outside",
        true,
    ),
    ListStyleImage = info("list-style-image", "css-lists", "<image> | none", "none", true),
    BaselineSource = info("baseline-source", "css-inline", "auto | first | last", "auto", false),
    AlignmentBaseline = info(
        "alignment-baseline",
        "css-inline",
        "baseline | <baseline-metric>",
        "baseline",
        false,
    ),
    BaselineShift = info(
        "baseline-shift",
        "css-inline",
        "<length-percentage> | sub | super | top | center | bottom",
        "0",
        false,
    ),
    Direction = info("direction", "css-writing-modes", "ltr | rtl", "ltr", true),
    UnicodeBidi = info(
        "unicode-bidi",
        "css-writing-modes",
        "normal | embed | isolate | bidi-override | isolate-override | plaintext",
        "normal",
        false,
    ),
    WritingMode = info(
        "writing-mode",
        "css-writing-modes",
        "horizontal-tb | vertical-rl | vertical-lr | sideways-rl | sideways-lr",
        "horizontal-tb",
        true,
    ),
    TextOrientation = info(
        "text-orientation",
        "css-writing-modes",
        "mixed | upright | sideways",
        "mixed",
        true,
    ),
}

impl Longhand {
    pub fn name(self) -> &'static str {
        self.info().name
    }

    /// Ob zwei Langformen in derselben *logical property group* sind, aber
    /// eine andere *mapping logic* haben.
    pub fn conflicts_with(self, other: Longhand) -> bool {
        match (self.info().group, other.info().group) {
            (Some((group, mapping)), Some((other_group, other_mapping))) => {
                group == other_group && mapping != other_mapping
            }
            _ => false,
        }
    }

    /// Der Anfangswert als deklarierter Wert, gelesen aus
    /// [`LonghandInfo::initial`]. `font-family` hat keinen festen
    /// Anfangswert („depends on user agent“).
    pub fn initial_value(self) -> Option<SpecifiedValue> {
        if self == Longhand::FontFamily {
            return None;
        }
        let values = parse_component_value_list(&Source::new(self.info().initial));
        let mut input = Input::new(values.values(), ParseContext::default());
        self.parse_value(&mut input)
    }

    /// Liest einen Wert nach der Grammatik der Langform; die Eingabe muss
    /// danach verbraucht sein.
    pub(crate) fn parse_value(self, input: &mut Input<'_>) -> Option<SpecifiedValue> {
        use SpecifiedValue as V;
        let mut input = input.clone();
        if self.info().quirky {
            input.allow_quirky_lengths();
        }
        let input = &mut input;
        let value = match self {
            Longhand::Display => V::Display(Display::parse(input)?),
            Longhand::Visibility => V::Visibility(Visibility::parse(input)?),
            Longhand::Color
            | Longhand::BackgroundColor
            | Longhand::BorderTopColor
            | Longhand::BorderRightColor
            | Longhand::BorderBottomColor
            | Longhand::BorderLeftColor
            | Longhand::BorderBlockStartColor
            | Longhand::BorderBlockEndColor
            | Longhand::BorderInlineStartColor
            | Longhand::BorderInlineEndColor
            | Longhand::TextDecorationColor => V::Color(Color::parse(input)?),
            Longhand::BackgroundImage => V::BackgroundImage(Layers::parse(input)?),
            Longhand::BackgroundPositionX => V::BackgroundPositionX(Layers::parse(input)?),
            Longhand::BackgroundPositionY => V::BackgroundPositionY(Layers::parse(input)?),
            Longhand::BackgroundSize => V::BackgroundSize(Layers::parse(input)?),
            Longhand::BackgroundRepeat => V::BackgroundRepeat(Layers::parse(input)?),
            Longhand::BackgroundAttachment => V::BackgroundAttachment(Layers::parse(input)?),
            Longhand::BackgroundOrigin => V::BackgroundOrigin(Layers::parse(input)?),
            Longhand::BackgroundClip => V::BackgroundClip(Layers::parse(input)?),
            Longhand::MarginTop
            | Longhand::MarginRight
            | Longhand::MarginBottom
            | Longhand::MarginLeft
            | Longhand::MarginBlockStart
            | Longhand::MarginBlockEnd
            | Longhand::MarginInlineStart
            | Longhand::MarginInlineEnd => {
                V::LengthPercentageOrAuto(LengthPercentageOrAuto::parse(input)?)
            }
            Longhand::PaddingTop
            | Longhand::PaddingRight
            | Longhand::PaddingBottom
            | Longhand::PaddingLeft
            | Longhand::PaddingBlockStart
            | Longhand::PaddingBlockEnd
            | Longhand::PaddingInlineStart
            | Longhand::PaddingInlineEnd => {
                V::LengthPercentage(non_negative_length_percentage(input)?)
            }
            Longhand::BorderTopWidth
            | Longhand::BorderRightWidth
            | Longhand::BorderBottomWidth
            | Longhand::BorderLeftWidth
            | Longhand::BorderBlockStartWidth
            | Longhand::BorderBlockEndWidth
            | Longhand::BorderInlineStartWidth
            | Longhand::BorderInlineEndWidth => V::LineWidth(LineWidth::parse(input)?),
            Longhand::BorderTopStyle
            | Longhand::BorderRightStyle
            | Longhand::BorderBottomStyle
            | Longhand::BorderLeftStyle
            | Longhand::BorderBlockStartStyle
            | Longhand::BorderBlockEndStyle
            | Longhand::BorderInlineStartStyle
            | Longhand::BorderInlineEndStyle => V::LineStyle(LineStyle::parse(input)?),
            Longhand::Width
            | Longhand::Height
            | Longhand::MinWidth
            | Longhand::MinHeight
            | Longhand::InlineSize
            | Longhand::BlockSize
            | Longhand::MinInlineSize
            | Longhand::MinBlockSize => V::Size(Size::parse(input)?),
            Longhand::MaxWidth
            | Longhand::MaxHeight
            | Longhand::MaxInlineSize
            | Longhand::MaxBlockSize => V::MaxSize(MaxSize::parse(input)?),
            Longhand::BoxSizing => V::BoxSizing(BoxSizing::parse(input)?),
            Longhand::FontFamily => V::FontFamily(FontFamilyList::parse(input)?),
            Longhand::FontSize => V::FontSize(FontSize::parse(input)?),
            Longhand::FontWeight => V::FontWeight(FontWeight::parse(input)?),
            Longhand::FontStyle => V::FontStyle(FontStyle::parse(input)?),
            Longhand::FontWidth => V::FontWidth(FontWidth::parse(input)?),
            Longhand::FontVariantCaps => V::FontVariantCaps(FontVariantCaps::parse(input)?),
            Longhand::LineHeight => V::LineHeight(LineHeight::parse(input)?),
            Longhand::TextAlignAll => V::TextAlignAll(TextAlignAll::parse(input)?),
            Longhand::TextAlignLast => V::TextAlignLast(TextAlignLast::parse(input)?),
            Longhand::TextIndent => V::TextIndent(TextIndent::parse(input)?),
            Longhand::TextTransform => V::TextTransform(TextTransform::parse(input)?),
            Longhand::WhiteSpaceCollapse => {
                V::WhiteSpaceCollapse(WhiteSpaceCollapse::parse(input)?)
            }
            Longhand::TextWrapMode => V::TextWrapMode(TextWrapMode::parse(input)?),
            Longhand::WhiteSpaceTrim => V::WhiteSpaceTrim(WhiteSpaceTrim::parse(input)?),
            Longhand::TextWrapStyle => V::TextWrapStyle(TextWrapStyle::parse(input)?),
            Longhand::LetterSpacing | Longhand::WordSpacing => V::Spacing(Spacing::parse(input)?),
            Longhand::TextDecorationLine => {
                V::TextDecorationLine(TextDecorationLine::parse(input)?)
            }
            Longhand::TextDecorationStyle => {
                V::TextDecorationStyle(TextDecorationStyle::parse(input)?)
            }
            Longhand::TextDecorationThickness => {
                V::TextDecorationThickness(TextDecorationThickness::parse(input)?)
            }
            Longhand::ListStyleType => V::ListStyleType(ListStyleType::parse(input)?),
            Longhand::ListStylePosition => V::ListStylePosition(ListStylePosition::parse(input)?),
            Longhand::ListStyleImage => V::ListStyleImage(ListStyleImage::parse(input)?),
            Longhand::BaselineSource => V::BaselineSource(BaselineSource::parse(input)?),
            Longhand::AlignmentBaseline => V::AlignmentBaseline(AlignmentBaseline::parse(input)?),
            Longhand::BaselineShift => V::BaselineShift(BaselineShift::parse(input)?),
            Longhand::Direction => V::Direction(Direction::parse(input)?),
            Longhand::UnicodeBidi => V::UnicodeBidi(UnicodeBidi::parse(input)?),
            Longhand::WritingMode => V::WritingMode(WritingMode::parse(input)?),
            Longhand::TextOrientation => V::TextOrientation(TextOrientation::parse(input)?),
        };
        input.finish(value)
    }
}

/// Der deklarierte Wert einer Langform, nach Werttyp. Welche Langform er
/// hat, steht in der [`PropertyDeclaration`].
#[derive(Clone, Debug, PartialEq)]
pub enum SpecifiedValue {
    Display(Display),
    Visibility(Visibility),
    Color(Color),
    BackgroundImage(Layers<BgImage>),
    BackgroundPositionX(Layers<PositionComponent<HorizontalEdge>>),
    BackgroundPositionY(Layers<PositionComponent<VerticalEdge>>),
    BackgroundSize(Layers<BgSize>),
    BackgroundRepeat(Layers<RepeatStyle>),
    BackgroundAttachment(Layers<Attachment>),
    BackgroundOrigin(Layers<VisualBox>),
    BackgroundClip(Layers<BgClip>),
    /// `margin-*`
    LengthPercentageOrAuto(LengthPercentageOrAuto),
    /// `padding-*`, nie negativ
    LengthPercentage(LengthPercentage),
    LineWidth(LineWidth),
    LineStyle(LineStyle),
    /// `width`, `height` und die Mindestgrößen
    Size(Size),
    /// die Höchstgrößen
    MaxSize(MaxSize),
    BoxSizing(BoxSizing),
    FontFamily(FontFamilyList),
    FontSize(FontSize),
    FontWeight(FontWeight),
    FontStyle(FontStyle),
    FontWidth(FontWidth),
    FontVariantCaps(FontVariantCaps),
    LineHeight(LineHeight),
    TextAlignAll(TextAlignAll),
    TextAlignLast(TextAlignLast),
    TextIndent(TextIndent),
    TextTransform(TextTransform),
    WhiteSpaceCollapse(WhiteSpaceCollapse),
    TextWrapMode(TextWrapMode),
    WhiteSpaceTrim(WhiteSpaceTrim),
    TextWrapStyle(TextWrapStyle),
    /// `letter-spacing` und `word-spacing`
    Spacing(Spacing),
    TextDecorationLine(TextDecorationLine),
    TextDecorationStyle(TextDecorationStyle),
    TextDecorationThickness(TextDecorationThickness),
    ListStyleType(ListStyleType),
    ListStylePosition(ListStylePosition),
    ListStyleImage(ListStyleImage),
    BaselineSource(BaselineSource),
    AlignmentBaseline(AlignmentBaseline),
    BaselineShift(BaselineShift),
    Direction(Direction),
    UnicodeBidi(UnicodeBidi),
    WritingMode(WritingMode),
    TextOrientation(TextOrientation),
}

impl ToCss for SpecifiedValue {
    fn to_css(&self, out: &mut String) {
        match self {
            SpecifiedValue::Display(value) => value.to_css(out),
            SpecifiedValue::Visibility(value) => value.to_css(out),
            SpecifiedValue::Color(value) => value.to_css(out),
            SpecifiedValue::BackgroundImage(value) => value.to_css(out),
            SpecifiedValue::BackgroundPositionX(value) => value.to_css(out),
            SpecifiedValue::BackgroundPositionY(value) => value.to_css(out),
            SpecifiedValue::BackgroundSize(value) => value.to_css(out),
            SpecifiedValue::BackgroundRepeat(value) => value.to_css(out),
            SpecifiedValue::BackgroundAttachment(value) => value.to_css(out),
            SpecifiedValue::BackgroundOrigin(value) => value.to_css(out),
            SpecifiedValue::BackgroundClip(value) => value.to_css(out),
            SpecifiedValue::LengthPercentageOrAuto(value) => value.to_css(out),
            SpecifiedValue::LengthPercentage(value) => value.to_css(out),
            SpecifiedValue::LineWidth(value) => value.to_css(out),
            SpecifiedValue::LineStyle(value) => value.to_css(out),
            SpecifiedValue::Size(value) => value.to_css(out),
            SpecifiedValue::MaxSize(value) => value.to_css(out),
            SpecifiedValue::BoxSizing(value) => value.to_css(out),
            SpecifiedValue::FontFamily(value) => value.to_css(out),
            SpecifiedValue::FontSize(value) => value.to_css(out),
            SpecifiedValue::FontWeight(value) => value.to_css(out),
            SpecifiedValue::FontStyle(value) => value.to_css(out),
            SpecifiedValue::FontWidth(value) => value.to_css(out),
            SpecifiedValue::FontVariantCaps(value) => value.to_css(out),
            SpecifiedValue::LineHeight(value) => value.to_css(out),
            SpecifiedValue::TextAlignAll(value) => value.to_css(out),
            SpecifiedValue::TextAlignLast(value) => value.to_css(out),
            SpecifiedValue::TextIndent(value) => value.to_css(out),
            SpecifiedValue::TextTransform(value) => value.to_css(out),
            SpecifiedValue::WhiteSpaceCollapse(value) => value.to_css(out),
            SpecifiedValue::TextWrapMode(value) => value.to_css(out),
            SpecifiedValue::WhiteSpaceTrim(value) => value.to_css(out),
            SpecifiedValue::TextWrapStyle(value) => value.to_css(out),
            SpecifiedValue::Spacing(value) => value.to_css(out),
            SpecifiedValue::TextDecorationLine(value) => value.to_css(out),
            SpecifiedValue::TextDecorationStyle(value) => value.to_css(out),
            SpecifiedValue::TextDecorationThickness(value) => value.to_css(out),
            SpecifiedValue::ListStyleType(value) => value.to_css(out),
            SpecifiedValue::ListStylePosition(value) => value.to_css(out),
            SpecifiedValue::ListStyleImage(value) => value.to_css(out),
            SpecifiedValue::BaselineSource(value) => value.to_css(out),
            SpecifiedValue::AlignmentBaseline(value) => value.to_css(out),
            SpecifiedValue::BaselineShift(value) => value.to_css(out),
            SpecifiedValue::Direction(value) => value.to_css(out),
            SpecifiedValue::UnicodeBidi(value) => value.to_css(out),
            SpecifiedValue::WritingMode(value) => value.to_css(out),
            SpecifiedValue::TextOrientation(value) => value.to_css(out),
        }
    }
}

/// Der Wert einer Deklaration: ein Wert nach der Grammatik oder ein
/// CSS-weites Schlüsselwort.
#[derive(Clone, Debug, PartialEq)]
pub enum DeclaredValue {
    Value(SpecifiedValue),
    CssWide(CssWideKeyword),
}

impl ToCss for DeclaredValue {
    fn to_css(&self, out: &mut String) {
        match self {
            DeclaredValue::Value(value) => value.to_css(out),
            DeclaredValue::CssWide(keyword) => keyword.to_css(out),
        }
    }
}

/// Eine Deklaration für eine Langform. Kurzschreibweisen werden beim Parsen
/// in die Deklarationen ihrer Langformen zerlegt.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyDeclaration {
    pub longhand: Longhand,
    pub value: DeclaredValue,
}

/// Eine unterstützte Eigenschaft.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyId {
    Longhand(Longhand),
    Shorthand(Shorthand),
}

/// Die Altnamen (*legacy name aliases*) mit der Eigenschaft, für die sie
/// stehen.
const ALIASES: [(&str, Longhand); 1] = [("font-stretch", Longhand::FontWidth)];

impl PropertyId {
    /// Die Eigenschaft zu einem Namen, ohne Unterschied zwischen ASCII-Groß-
    /// und -Kleinbuchstaben. Ein Altname liefert die Eigenschaft, für die er
    /// steht.
    pub fn from_name(name: &str) -> Option<PropertyId> {
        if let Some(longhand) = Longhand::ALL
            .iter()
            .find(|longhand| longhand.name().eq_ignore_ascii_case(name))
        {
            return Some(PropertyId::Longhand(*longhand));
        }
        if let Some(shorthand) = Shorthand::ALL
            .iter()
            .find(|shorthand| shorthand.name().eq_ignore_ascii_case(name))
        {
            return Some(PropertyId::Shorthand(*shorthand));
        }
        ALIASES
            .iter()
            .find(|(alias, _)| alias.eq_ignore_ascii_case(name))
            .map(|(_, longhand)| PropertyId::Longhand(*longhand))
    }

    pub fn name(self) -> &'static str {
        match self {
            PropertyId::Longhand(longhand) => longhand.name(),
            PropertyId::Shorthand(shorthand) => shorthand.name(),
        }
    }

    /// Ob `longhand` die Eigenschaft selbst ist oder zu ihr gehört.
    pub fn contains(self, longhand: Longhand) -> bool {
        match self {
            PropertyId::Longhand(own) => own == longhand,
            PropertyId::Shorthand(shorthand) => shorthand.longhands().contains(&longhand),
        }
    }

    /// *Parse a CSS value* für die Eigenschaft: ein CSS-weites
    /// Schlüsselwort allein oder ein Wert nach ihrer Grammatik. Liefert die
    /// Deklarationen der Langformen in kanonischer Reihenfolge.
    pub fn parse(
        self,
        values: Values<'_>,
        context: ParseContext<'_>,
    ) -> Option<Vec<PropertyDeclaration>> {
        let mut input = Input::new(values, context);
        if let Some(keyword) = input.try_parse(|input| {
            let keyword = CssWideKeyword::parse(input)?;
            input.finish(keyword)
        }) {
            let declaration = |longhand| PropertyDeclaration {
                longhand,
                value: DeclaredValue::CssWide(keyword),
            };
            return Some(match self {
                PropertyId::Longhand(longhand) => vec![declaration(longhand)],
                PropertyId::Shorthand(shorthand) => shorthand
                    .longhands()
                    .iter()
                    .map(|&longhand| declaration(longhand))
                    .collect(),
            });
        }
        match self {
            PropertyId::Longhand(longhand) => {
                let value = longhand.parse_value(&mut input)?;
                Some(vec![PropertyDeclaration {
                    longhand,
                    value: DeclaredValue::Value(value),
                }])
            }
            PropertyId::Shorthand(shorthand) => {
                let values = shorthand.parse_value(&mut input)?;
                let values = input.finish(values)?;
                let longhands = shorthand.longhands();
                (values.len() == longhands.len()).then(|| {
                    longhands
                        .iter()
                        .zip(values)
                        .map(|(&longhand, value)| PropertyDeclaration {
                            longhand,
                            value: DeclaredValue::Value(value),
                        })
                        .collect()
                })
            }
        }
    }
}

/// Serialisiert die Werte der Langformen einer Kurzschreibweise, in der
/// Reihenfolge von [`Shorthand::longhands`] („serialize a CSS value“ für
/// eine Liste). Ein CSS-weites Schlüsselwort gilt nur, wenn alle dasselbe
/// haben. `None` heißt, die Kurzschreibweise kann die Werte nicht genau
/// wiedergeben.
pub fn serialize_shorthand(shorthand: Shorthand, values: &[&DeclaredValue]) -> Option<String> {
    if values.len() != shorthand.longhands().len() {
        return None;
    }
    let mut keywords = values.iter().filter_map(|value| match value {
        DeclaredValue::CssWide(keyword) => Some(*keyword),
        DeclaredValue::Value(_) => None,
    });
    if let Some(first) = keywords.next() {
        let all_same = values
            .iter()
            .all(|value| **value == DeclaredValue::CssWide(first));
        return all_same.then(|| first.name().to_owned());
    }
    let specified: Vec<&SpecifiedValue> = values
        .iter()
        .filter_map(|value| match value {
            DeclaredValue::Value(value) => Some(value),
            DeclaredValue::CssWide(_) => None,
        })
        .collect();
    shorthand.serialize(&specified)
}
