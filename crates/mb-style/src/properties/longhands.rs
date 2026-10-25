//! Die Werttypen der Langformen: Parsen nach der Grammatik und
//! Serialisieren des deklarierten Werts nach CSSOM („serialize a CSS
//! value“). Schreibweisen mit `||` werden in der Reihenfolge der Grammatik
//! geschrieben, Weglassbares fehlt.

use crate::values::{
    Angle, Input, Length, LengthPercentage, Parse, SpecifiedUrl, ToCss, custom_ident,
    is_css_wide_keyword, keywords, non_negative_length, non_negative_length_percentage,
    write_identifier, write_number, write_string,
};

keywords! {
    /// ⟨display-outside⟩
    pub enum DisplayOutside {
        Block = "block",
        Inline = "inline",
        RunIn = "run-in",
    }
}

keywords! {
    /// ⟨display-inside⟩
    pub enum DisplayInside {
        Flow = "flow",
        FlowRoot = "flow-root",
        Table = "table",
        Flex = "flex",
        Grid = "grid",
        Ruby = "ruby",
    }
}

keywords! {
    /// ⟨display-internal⟩
    pub enum DisplayInternal {
        TableRowGroup = "table-row-group",
        TableHeaderGroup = "table-header-group",
        TableFooterGroup = "table-footer-group",
        TableRow = "table-row",
        TableCell = "table-cell",
        TableColumnGroup = "table-column-group",
        TableColumn = "table-column",
        TableCaption = "table-caption",
        RubyBase = "ruby-base",
        RubyText = "ruby-text",
        RubyBaseContainer = "ruby-base-container",
        RubyTextContainer = "ruby-text-container",
    }
}

/// Ein Wert von `display` (CSS Display 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Display {
    /// Äußerer und innerer Anzeigetyp, bei ⟨display-listitem⟩ mit
    /// `list_item`. Auch die Schlüsselwörter aus ⟨display-legacy⟩ werden so
    /// gespeichert, etwa `inline-block` als `inline flow-root`.
    Pair {
        outside: DisplayOutside,
        inside: DisplayInside,
        list_item: bool,
    },
    Internal(DisplayInternal),
    /// `contents`
    Contents,
    /// `none`
    None,
}

impl Parse for Display {
    fn parse(input: &mut Input<'_>) -> Option<Display> {
        if input.keyword("none") {
            return Some(Display::None);
        }
        if input.keyword("contents") {
            return Some(Display::Contents);
        }
        if let Some(internal) = DisplayInternal::parse(input) {
            return Some(Display::Internal(internal));
        }
        for (keyword, inside) in [
            ("inline-block", DisplayInside::FlowRoot),
            ("inline-table", DisplayInside::Table),
            ("inline-flex", DisplayInside::Flex),
            ("inline-grid", DisplayInside::Grid),
        ] {
            if input.keyword(keyword) {
                return Some(Display::Pair {
                    outside: DisplayOutside::Inline,
                    inside,
                    list_item: false,
                });
            }
        }
        // `[ <display-outside> || <display-inside> ]` und ⟨display-listitem⟩:
        // jedes Teil höchstens einmal, in beliebiger Reihenfolge.
        let mut outside = None;
        let mut inside = None;
        let mut list_item = false;
        loop {
            if outside.is_none()
                && let Some(value) = DisplayOutside::parse(input)
            {
                outside = Some(value);
                continue;
            }
            if inside.is_none()
                && let Some(value) = DisplayInside::parse(input)
            {
                inside = Some(value);
                continue;
            }
            if !list_item && input.keyword("list-item") {
                list_item = true;
                continue;
            }
            break;
        }
        if outside.is_none() && inside.is_none() && !list_item {
            return None;
        }
        // In ⟨display-listitem⟩ ist nur `flow` oder `flow-root` erlaubt.
        if list_item
            && !matches!(
                inside,
                None | Some(DisplayInside::Flow | DisplayInside::FlowRoot)
            )
        {
            return None;
        }
        let inside = inside.unwrap_or(DisplayInside::Flow);
        // Fehlt der äußere Typ, ist er `block`, bei `ruby` aber `inline`.
        let outside = outside.unwrap_or(if inside == DisplayInside::Ruby {
            DisplayOutside::Inline
        } else {
            DisplayOutside::Block
        });
        Some(Display::Pair {
            outside,
            inside,
            list_item,
        })
    }
}

impl ToCss for Display {
    /// Die kürzeste Form: ein Schlüsselwort, wo es eines gibt, sonst die
    /// Teile, die nicht dem Standardwert gleichen.
    fn to_css(&self, out: &mut String) {
        use DisplayInside::{Flex, Flow, FlowRoot, Grid, Ruby, Table};
        use DisplayOutside::{Block, Inline, RunIn};
        match *self {
            Display::None => out.push_str("none"),
            Display::Contents => out.push_str("contents"),
            Display::Internal(internal) => internal.to_css(out),
            Display::Pair {
                outside,
                inside,
                list_item: true,
            } => {
                if outside != Block {
                    outside.to_css(out);
                    out.push(' ');
                }
                if inside != Flow {
                    inside.to_css(out);
                    out.push(' ');
                }
                out.push_str("list-item");
            }
            Display::Pair {
                outside, inside, ..
            } => match (outside, inside) {
                (Block, Flow) => out.push_str("block"),
                (Block, FlowRoot) => out.push_str("flow-root"),
                (Block, Table | Flex | Grid) => inside.to_css(out),
                (Block, Ruby) => out.push_str("block ruby"),
                (Inline, Flow) => out.push_str("inline"),
                (Inline, FlowRoot) => out.push_str("inline-block"),
                (Inline, Table | Flex | Grid) => {
                    out.push_str("inline-");
                    inside.to_css(out);
                }
                (Inline, Ruby) => out.push_str("ruby"),
                (RunIn, Flow) => out.push_str("run-in"),
                (RunIn, _) => {
                    out.push_str("run-in ");
                    inside.to_css(out);
                }
            },
        }
    }
}

keywords! {
    /// `visibility`
    pub enum Visibility {
        Visible = "visible",
        Hidden = "hidden",
        ForceHidden = "force-hidden",
        Collapse = "collapse",
    }
}

/// `<length-percentage> | auto`, etwa für `margin-top`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthPercentageOrAuto {
    Auto,
    LengthPercentage(LengthPercentage),
}

impl Parse for LengthPercentageOrAuto {
    fn parse(input: &mut Input<'_>) -> Option<LengthPercentageOrAuto> {
        if input.keyword("auto") {
            return Some(LengthPercentageOrAuto::Auto);
        }
        LengthPercentage::parse(input).map(LengthPercentageOrAuto::LengthPercentage)
    }
}

impl ToCss for LengthPercentageOrAuto {
    fn to_css(&self, out: &mut String) {
        match self {
            LengthPercentageOrAuto::Auto => out.push_str("auto"),
            LengthPercentageOrAuto::LengthPercentage(value) => value.to_css(out),
        }
    }
}

/// Ein Wert je Hintergrundebene (*background image layer*), durch Kommas
/// getrennt: `<T>#`. Die Listen der Langformen von `background` können
/// verschieden lang sein; welche Werte gelten, entscheidet erst die Zahl
/// der Bilder (Backgrounds 3, „Layering Multiple Background Images“).
#[derive(Clone, Debug, PartialEq)]
pub struct Layers<T>(pub Vec<T>);

impl<T: Parse> Parse for Layers<T> {
    fn parse(input: &mut Input<'_>) -> Option<Layers<T>> {
        input.try_parse(|input| {
            let mut layers = vec![T::parse(input)?];
            while input.comma() {
                layers.push(T::parse(input)?);
            }
            Some(Layers(layers))
        })
    }
}

impl<T: ToCss> ToCss for Layers<T> {
    fn to_css(&self, out: &mut String) {
        for (index, layer) in self.0.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            layer.to_css(out);
        }
    }
}

/// ⟨bg-image⟩: `<image> | none`. Von ⟨image⟩ gibt es bisher nur ⟨url⟩,
/// Verläufe und die Bildfunktionen fehlen noch.
#[derive(Clone, Debug, PartialEq)]
pub enum BgImage {
    None,
    Url(Box<SpecifiedUrl>),
}

impl Parse for BgImage {
    fn parse(input: &mut Input<'_>) -> Option<BgImage> {
        if input.keyword("none") {
            return Some(BgImage::None);
        }
        SpecifiedUrl::parse(input).map(|url| BgImage::Url(Box::new(url)))
    }
}

impl ToCss for BgImage {
    fn to_css(&self, out: &mut String) {
        match self {
            BgImage::None => out.push_str("none"),
            BgImage::Url(url) => url.to_css(out),
        }
    }
}

/// Von welcher Seite eine Kante in `background-position-x` oder `-y` zählt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeSide {
    /// `left` oder `top`
    Start,
    /// `right` oder `bottom`
    End,
    /// `x-start`, `x-end`, `y-start` oder `y-end`: nach der Schreibrichtung
    Flow,
}

/// Eine Kante in einer Achse von `background-position`.
pub trait PositionEdge: Copy {
    fn side(self) -> EdgeSide;
}

keywords! {
    /// Die Kanten von `background-position-x`.
    pub enum HorizontalEdge {
        Left = "left",
        Right = "right",
        XStart = "x-start",
        XEnd = "x-end",
    }
}

impl PositionEdge for HorizontalEdge {
    fn side(self) -> EdgeSide {
        match self {
            HorizontalEdge::Left => EdgeSide::Start,
            HorizontalEdge::Right => EdgeSide::End,
            HorizontalEdge::XStart | HorizontalEdge::XEnd => EdgeSide::Flow,
        }
    }
}

keywords! {
    /// Die Kanten von `background-position-y`.
    pub enum VerticalEdge {
        Top = "top",
        Bottom = "bottom",
        YStart = "y-start",
        YEnd = "y-end",
    }
}

impl PositionEdge for VerticalEdge {
    fn side(self) -> EdgeSide {
        match self {
            VerticalEdge::Top => EdgeSide::Start,
            VerticalEdge::Bottom => EdgeSide::End,
            VerticalEdge::YStart | VerticalEdge::YEnd => EdgeSide::Flow,
        }
    }
}

/// Ein Eintrag in `background-position-x` oder `background-position-y`:
/// `center | [ <kante>? <length-percentage>? ]!`. Ohne Kante zählt der
/// Abstand von `left` oder `top`, ohne Abstand ist er null.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PositionComponent<E> {
    Center,
    Edge(E),
    Offset(LengthPercentage),
    EdgeOffset(E, LengthPercentage),
}

impl<E: Parse> Parse for PositionComponent<E> {
    fn parse(input: &mut Input<'_>) -> Option<PositionComponent<E>> {
        if input.keyword("center") {
            return Some(PositionComponent::Center);
        }
        let edge = E::parse(input);
        let offset = LengthPercentage::parse(input);
        Some(match (edge, offset) {
            (Some(edge), Some(offset)) => PositionComponent::EdgeOffset(edge, offset),
            (Some(edge), None) => PositionComponent::Edge(edge),
            (None, Some(offset)) => PositionComponent::Offset(offset),
            (None, None) => return None,
        })
    }
}

impl<E: ToCss> ToCss for PositionComponent<E> {
    fn to_css(&self, out: &mut String) {
        match self {
            PositionComponent::Center => out.push_str("center"),
            PositionComponent::Edge(edge) => edge.to_css(out),
            PositionComponent::Offset(offset) => offset.to_css(out),
            PositionComponent::EdgeOffset(edge, offset) => {
                edge.to_css(out);
                out.push(' ');
                offset.to_css(out);
            }
        }
    }
}

/// ⟨bg-size⟩: `[ <length-percentage [0,∞]> | auto ]{1,2} | cover |
/// contain`. Eine fehlende Höhe ist `auto`. Geschrieben wird `auto auto`
/// als `auto`, sonst stehen beide Angaben da, wie es die WPT erwarten
/// (w3c/csswg-drafts#7802); Backgrounds 4 verlangt immer zwei.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BgSize {
    Cover,
    Contain,
    Size {
        width: LengthPercentageOrAuto,
        height: LengthPercentageOrAuto,
    },
}

impl BgSize {
    pub const AUTO: BgSize = BgSize::Size {
        width: LengthPercentageOrAuto::Auto,
        height: LengthPercentageOrAuto::Auto,
    };
}

/// `<length-percentage [0,∞]> | auto`
fn bg_size_component(input: &mut Input<'_>) -> Option<LengthPercentageOrAuto> {
    if input.keyword("auto") {
        return Some(LengthPercentageOrAuto::Auto);
    }
    non_negative_length_percentage(input).map(LengthPercentageOrAuto::LengthPercentage)
}

impl Parse for BgSize {
    fn parse(input: &mut Input<'_>) -> Option<BgSize> {
        if input.keyword("cover") {
            return Some(BgSize::Cover);
        }
        if input.keyword("contain") {
            return Some(BgSize::Contain);
        }
        let width = bg_size_component(input)?;
        let height = bg_size_component(input).unwrap_or(LengthPercentageOrAuto::Auto);
        Some(BgSize::Size { width, height })
    }
}

impl ToCss for BgSize {
    fn to_css(&self, out: &mut String) {
        match self {
            BgSize::Cover => out.push_str("cover"),
            BgSize::Contain => out.push_str("contain"),
            BgSize::Size { width, height } => {
                width.to_css(out);
                if *self != BgSize::AUTO {
                    out.push(' ');
                    height.to_css(out);
                }
            }
        }
    }
}

keywords! {
    /// ⟨repetition⟩
    pub enum Repetition {
        Repeat = "repeat",
        Space = "space",
        Round = "round",
        NoRepeat = "no-repeat",
    }
}

/// ⟨repeat-style⟩: `repeat-x | repeat-y | repeat-block | repeat-inline |
/// <repetition>{1,2}`. `repeat-x`, `repeat-y` und ein einzelnes
/// Schlüsselwort stehen als das Paar da, zu dem sie berechnet werden;
/// `repeat-block` und `repeat-inline` werden das erst beim Berechnen, nach
/// dem Schreibmodus. Ein Paar wird so kurz wie möglich geschrieben.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatStyle {
    /// Waagrecht, dann senkrecht
    Pair(Repetition, Repetition),
    Block,
    Inline,
}

impl RepeatStyle {
    pub const REPEAT: RepeatStyle = RepeatStyle::Pair(Repetition::Repeat, Repetition::Repeat);
}

impl Parse for RepeatStyle {
    fn parse(input: &mut Input<'_>) -> Option<RepeatStyle> {
        use Repetition::{NoRepeat, Repeat};
        if input.keyword("repeat-x") {
            return Some(RepeatStyle::Pair(Repeat, NoRepeat));
        }
        if input.keyword("repeat-y") {
            return Some(RepeatStyle::Pair(NoRepeat, Repeat));
        }
        if input.keyword("repeat-block") {
            return Some(RepeatStyle::Block);
        }
        if input.keyword("repeat-inline") {
            return Some(RepeatStyle::Inline);
        }
        let horizontal = Repetition::parse(input)?;
        let vertical = Repetition::parse(input).unwrap_or(horizontal);
        Some(RepeatStyle::Pair(horizontal, vertical))
    }
}

impl ToCss for RepeatStyle {
    fn to_css(&self, out: &mut String) {
        use Repetition::{NoRepeat, Repeat};
        match *self {
            RepeatStyle::Pair(Repeat, NoRepeat) => out.push_str("repeat-x"),
            RepeatStyle::Pair(NoRepeat, Repeat) => out.push_str("repeat-y"),
            RepeatStyle::Pair(horizontal, vertical) => {
                horizontal.to_css(out);
                if vertical != horizontal {
                    out.push(' ');
                    vertical.to_css(out);
                }
            }
            RepeatStyle::Block => out.push_str("repeat-block"),
            RepeatStyle::Inline => out.push_str("repeat-inline"),
        }
    }
}

keywords! {
    /// ⟨attachment⟩
    pub enum Attachment {
        Scroll = "scroll",
        Fixed = "fixed",
        Local = "local",
    }
}

keywords! {
    /// ⟨visual-box⟩
    pub enum VisualBox {
        ContentBox = "content-box",
        PaddingBox = "padding-box",
        BorderBox = "border-box",
    }
}

/// ⟨bg-clip⟩: `<visual-box> | [ border-area || text ]`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BgClip {
    Box(VisualBox),
    /// Mindestens eins von beiden ist gesetzt.
    Area {
        border_area: bool,
        text: bool,
    },
}

impl BgClip {
    /// `border-area || text`, was nur `background-clip` kennt.
    pub(crate) fn parse_area(input: &mut Input<'_>) -> Option<BgClip> {
        let mut border_area = false;
        let mut text = false;
        loop {
            if !border_area && input.keyword("border-area") {
                border_area = true;
            } else if !text && input.keyword("text") {
                text = true;
            } else {
                break;
            }
        }
        (border_area || text).then_some(BgClip::Area { border_area, text })
    }
}

impl Parse for BgClip {
    fn parse(input: &mut Input<'_>) -> Option<BgClip> {
        if let Some(visual_box) = VisualBox::parse(input) {
            return Some(BgClip::Box(visual_box));
        }
        BgClip::parse_area(input)
    }
}

impl ToCss for BgClip {
    fn to_css(&self, out: &mut String) {
        match *self {
            BgClip::Box(visual_box) => visual_box.to_css(out),
            BgClip::Area { border_area, text } => {
                if border_area {
                    out.push_str("border-area");
                }
                if text {
                    if border_area {
                        out.push(' ');
                    }
                    out.push_str("text");
                }
            }
        }
    }
}

keywords! {
    /// Die Schlüsselwörter von ⟨line-width⟩.
    pub enum LineWidthKeyword {
        Hairline = "hairline",
        Thin = "thin",
        Medium = "medium",
        Thick = "thick",
    }
}

/// ⟨line-width⟩ = `<length [0,∞]> | hairline | thin | medium | thick`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineWidth {
    Length(Length),
    Keyword(LineWidthKeyword),
}

impl Parse for LineWidth {
    fn parse(input: &mut Input<'_>) -> Option<LineWidth> {
        if let Some(keyword) = LineWidthKeyword::parse(input) {
            return Some(LineWidth::Keyword(keyword));
        }
        non_negative_length(input).map(LineWidth::Length)
    }
}

impl ToCss for LineWidth {
    fn to_css(&self, out: &mut String) {
        match self {
            LineWidth::Length(length) => length.to_css(out),
            LineWidth::Keyword(keyword) => keyword.to_css(out),
        }
    }
}

keywords! {
    /// ⟨line-style⟩
    pub enum LineStyle {
        None = "none",
        Hidden = "hidden",
        Dotted = "dotted",
        Dashed = "dashed",
        Solid = "solid",
        Double = "double",
        Groove = "groove",
        Ridge = "ridge",
        Inset = "inset",
        Outset = "outset",
    }
}

keywords! {
    /// Die Schlüsselwörter von ⟨box-size⟩ (CSS Box Sizing 3).
    pub enum SizeKeyword {
        Stretch = "stretch",
        MinContent = "min-content",
        MaxContent = "max-content",
        FitContent = "fit-content",
    }
}

/// ⟨box-size⟩ = `<length-percentage [0,∞]> | stretch | min-content |
/// max-content | fit-content`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxSize {
    LengthPercentage(LengthPercentage),
    Keyword(SizeKeyword),
}

impl Parse for BoxSize {
    fn parse(input: &mut Input<'_>) -> Option<BoxSize> {
        if let Some(keyword) = SizeKeyword::parse(input) {
            return Some(BoxSize::Keyword(keyword));
        }
        non_negative_length_percentage(input).map(BoxSize::LengthPercentage)
    }
}

impl ToCss for BoxSize {
    fn to_css(&self, out: &mut String) {
        match self {
            BoxSize::LengthPercentage(value) => value.to_css(out),
            BoxSize::Keyword(keyword) => keyword.to_css(out),
        }
    }
}

/// `auto | <box-size>`, etwa für `width` und `min-width`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Auto,
    Box(BoxSize),
}

impl Parse for Size {
    fn parse(input: &mut Input<'_>) -> Option<Size> {
        if input.keyword("auto") {
            return Some(Size::Auto);
        }
        BoxSize::parse(input).map(Size::Box)
    }
}

impl ToCss for Size {
    fn to_css(&self, out: &mut String) {
        match self {
            Size::Auto => out.push_str("auto"),
            Size::Box(size) => size.to_css(out),
        }
    }
}

/// `none | <box-size>`, für `max-width` und verwandte.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MaxSize {
    None,
    Box(BoxSize),
}

impl Parse for MaxSize {
    fn parse(input: &mut Input<'_>) -> Option<MaxSize> {
        if input.keyword("none") {
            return Some(MaxSize::None);
        }
        BoxSize::parse(input).map(MaxSize::Box)
    }
}

impl ToCss for MaxSize {
    fn to_css(&self, out: &mut String) {
        match self {
            MaxSize::None => out.push_str("none"),
            MaxSize::Box(size) => size.to_css(out),
        }
    }
}

keywords! {
    /// `box-sizing`
    pub enum BoxSizing {
        ContentBox = "content-box",
        BorderBox = "border-box",
    }
}

keywords! {
    /// ⟨generic-font-complete⟩ und ⟨generic-font-incomplete⟩
    pub enum GenericFamily {
        Serif = "serif",
        SansSerif = "sans-serif",
        SystemUi = "system-ui",
        Cursive = "cursive",
        Fantasy = "fantasy",
        Math = "math",
        Monospace = "monospace",
        UiSerif = "ui-serif",
        UiSansSerif = "ui-sans-serif",
        UiMonospace = "ui-monospace",
        UiRounded = "ui-rounded",
    }
}

keywords! {
    /// Die Argumente von ⟨generic-font-script-specific⟩, also
    /// `generic(fangsong)` und so weiter.
    pub enum ScriptGenericFamily {
        Fangsong = "fangsong",
        Kai = "kai",
        KhmerMul = "khmer-mul",
        Nastaliq = "nastaliq",
    }
}

/// Ein Eintrag in `font-family`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FontFamily {
    /// Ein ⟨font-family-name⟩. Aus Bezeichnern wird der Name mit einzelnen
    /// Leerzeichen zusammengesetzt.
    Name(String),
    Generic(GenericFamily),
    ScriptGeneric(ScriptGenericFamily),
}

/// Ob ein Bezeichner kein ⟨custom-ident⟩ in einem Schriftnamen sein darf:
/// die CSS-weiten Schlüsselwörter, `default` und die Schlüsselwörter der
/// generischen Familien (Fonts 4, „Syntax of ⟨font-family-name⟩“).
fn reserved_in_family_name(ident: &str) -> bool {
    is_css_wide_keyword(ident)
        || ident.eq_ignore_ascii_case("default")
        || GenericFamily::from_ident(ident).is_some()
}

impl Parse for FontFamily {
    fn parse(input: &mut Input<'_>) -> Option<FontFamily> {
        if let Some(name) = input.string() {
            return Some(FontFamily::Name(name.to_owned()));
        }
        if let Some(family) = input.try_parse(|input| {
            let (name, mut arguments) = input.function()?;
            if !name.eq_ignore_ascii_case("generic") {
                return None;
            }
            let family = ScriptGenericFamily::parse(&mut arguments)?;
            arguments.finish(family)
        }) {
            return Some(FontFamily::ScriptGeneric(family));
        }
        input.try_parse(|input| {
            let first = input.ident()?;
            // Ein generisches Schlüsselwort steht allein; folgt ein
            // weiterer Bezeichner, scheitert die Liste danach.
            if let Some(generic) = GenericFamily::from_ident(first) {
                return Some(FontFamily::Generic(generic));
            }
            if reserved_in_family_name(first) {
                return None;
            }
            let mut name = first.to_owned();
            while let Some(ident) = input.ident() {
                if reserved_in_family_name(ident) {
                    return None;
                }
                name.push(' ');
                name.push_str(ident);
            }
            Some(FontFamily::Name(name))
        })
    }
}

impl ToCss for FontFamily {
    /// Ein Name bleibt ohne Anführungszeichen, wenn er als Folge von
    /// Bezeichnern genauso wieder gelesen wird, sonst wird er eine
    /// Zeichenkette. So verhalten sich die Browser und erwarten es die WPT;
    /// CSSOM verlangt dagegen, Namen immer als Zeichenkette zu schreiben.
    fn to_css(&self, out: &mut String) {
        match self {
            FontFamily::Generic(generic) => generic.to_css(out),
            FontFamily::ScriptGeneric(family) => {
                out.push_str("generic(");
                family.to_css(out);
                out.push(')');
            }
            FontFamily::Name(name) => {
                let as_identifiers = name.split(' ').all(|word| {
                    let mut written = String::new();
                    write_identifier(&mut written, word);
                    !word.is_empty() && written == word && !reserved_in_family_name(word)
                });
                if as_identifiers {
                    out.push_str(name);
                } else {
                    write_string(out, name);
                }
            }
        }
    }
}

/// `font-family`: `[ <font-family-name> | <generic-font-family> ]#`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontFamilyList(pub Vec<FontFamily>);

impl Parse for FontFamilyList {
    fn parse(input: &mut Input<'_>) -> Option<FontFamilyList> {
        input.try_parse(|input| {
            let mut families = vec![FontFamily::parse(input)?];
            while input.comma() {
                families.push(FontFamily::parse(input)?);
            }
            Some(FontFamilyList(families))
        })
    }
}

impl ToCss for FontFamilyList {
    fn to_css(&self, out: &mut String) {
        for (index, family) in self.0.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            family.to_css(out);
        }
    }
}

keywords! {
    /// ⟨absolute-size⟩
    pub enum AbsoluteSize {
        XxSmall = "xx-small",
        XSmall = "x-small",
        Small = "small",
        Medium = "medium",
        Large = "large",
        XLarge = "x-large",
        XxLarge = "xx-large",
        XxxLarge = "xxx-large",
    }
}

keywords! {
    /// ⟨relative-size⟩
    pub enum RelativeSize {
        Larger = "larger",
        Smaller = "smaller",
    }
}

/// `font-size`: `<absolute-size> | <relative-size> |
/// <length-percentage [0,∞]> | math`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontSize {
    Absolute(AbsoluteSize),
    Relative(RelativeSize),
    LengthPercentage(LengthPercentage),
    Math,
}

impl Parse for FontSize {
    fn parse(input: &mut Input<'_>) -> Option<FontSize> {
        if let Some(size) = AbsoluteSize::parse(input) {
            return Some(FontSize::Absolute(size));
        }
        if let Some(size) = RelativeSize::parse(input) {
            return Some(FontSize::Relative(size));
        }
        if input.keyword("math") {
            return Some(FontSize::Math);
        }
        non_negative_length_percentage(input).map(FontSize::LengthPercentage)
    }
}

impl ToCss for FontSize {
    fn to_css(&self, out: &mut String) {
        match self {
            FontSize::Absolute(size) => size.to_css(out),
            FontSize::Relative(size) => size.to_css(out),
            FontSize::LengthPercentage(value) => value.to_css(out),
            FontSize::Math => out.push_str("math"),
        }
    }
}

/// `font-weight`: `<font-weight-absolute> | bolder | lighter` mit
/// ⟨font-weight-absolute⟩ = `[ normal | bold | <number [1,1000]> ]`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontWeight {
    Normal,
    Bold,
    Number(f64),
    Bolder,
    Lighter,
}

impl Parse for FontWeight {
    fn parse(input: &mut Input<'_>) -> Option<FontWeight> {
        for (keyword, weight) in [
            ("normal", FontWeight::Normal),
            ("bold", FontWeight::Bold),
            ("bolder", FontWeight::Bolder),
            ("lighter", FontWeight::Lighter),
        ] {
            if input.keyword(keyword) {
                return Some(weight);
            }
        }
        input
            .try_parse(|input| {
                input
                    .number()
                    .filter(|weight| (1.0..=1000.0).contains(weight))
            })
            .map(FontWeight::Number)
    }
}

impl ToCss for FontWeight {
    fn to_css(&self, out: &mut String) {
        match self {
            FontWeight::Normal => out.push_str("normal"),
            FontWeight::Bold => out.push_str("bold"),
            FontWeight::Number(weight) => write_number(out, *weight),
            FontWeight::Bolder => out.push_str("bolder"),
            FontWeight::Lighter => out.push_str("lighter"),
        }
    }
}

/// `font-style`: `normal | italic | left | right |
/// oblique <angle [-90deg,90deg]>?`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontStyle {
    Normal,
    Italic,
    Left,
    Right,
    /// `oblique` mit dem Winkel, falls angegeben.
    Oblique(Option<Angle>),
}

impl Parse for FontStyle {
    /// `oblique 0deg` ist dasselbe wie `normal` und wird gleich so
    /// gespeichert; die WPT erwarten `normal` als Serialisierung.
    fn parse(input: &mut Input<'_>) -> Option<FontStyle> {
        for (keyword, style) in [
            ("normal", FontStyle::Normal),
            ("italic", FontStyle::Italic),
            ("left", FontStyle::Left),
            ("right", FontStyle::Right),
        ] {
            if input.keyword(keyword) {
                return Some(style);
            }
        }
        if !input.keyword("oblique") {
            return None;
        }
        let angle = input.try_parse(|input| {
            Angle::parse(input).filter(|angle| (-90.0..=90.0).contains(&angle.degrees()))
        });
        Some(match angle {
            Some(angle) if angle.degrees() == 0.0 => FontStyle::Normal,
            angle => FontStyle::Oblique(angle),
        })
    }
}

impl ToCss for FontStyle {
    fn to_css(&self, out: &mut String) {
        match self {
            FontStyle::Normal => out.push_str("normal"),
            FontStyle::Italic => out.push_str("italic"),
            FontStyle::Left => out.push_str("left"),
            FontStyle::Right => out.push_str("right"),
            FontStyle::Oblique(angle) => {
                out.push_str("oblique");
                if let Some(angle) = angle {
                    out.push(' ');
                    angle.to_css(out);
                }
            }
        }
    }
}

keywords! {
    /// Die Schlüsselwörter von `font-width`, zugleich ⟨font-width-css3⟩.
    pub enum FontWidthKeyword {
        Normal = "normal",
        UltraCondensed = "ultra-condensed",
        ExtraCondensed = "extra-condensed",
        Condensed = "condensed",
        SemiCondensed = "semi-condensed",
        SemiExpanded = "semi-expanded",
        Expanded = "expanded",
        ExtraExpanded = "extra-expanded",
        UltraExpanded = "ultra-expanded",
    }
}

/// `font-width`: ein Schlüsselwort oder `<percentage [0,∞]>`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontWidth {
    Keyword(FontWidthKeyword),
    Percentage(f64),
}

impl Parse for FontWidth {
    fn parse(input: &mut Input<'_>) -> Option<FontWidth> {
        if let Some(keyword) = FontWidthKeyword::parse(input) {
            return Some(FontWidth::Keyword(keyword));
        }
        input
            .try_parse(|input| input.percentage().filter(|value| *value >= 0.0))
            .map(FontWidth::Percentage)
    }
}

impl ToCss for FontWidth {
    fn to_css(&self, out: &mut String) {
        match self {
            FontWidth::Keyword(keyword) => keyword.to_css(out),
            FontWidth::Percentage(value) => {
                write_number(out, *value);
                out.push('%');
            }
        }
    }
}

keywords! {
    /// `font-variant-caps`
    pub enum FontVariantCaps {
        Normal = "normal",
        SmallCaps = "small-caps",
        AllSmallCaps = "all-small-caps",
        PetiteCaps = "petite-caps",
        AllPetiteCaps = "all-petite-caps",
        Unicase = "unicase",
        TitlingCaps = "titling-caps",
    }
}

/// `line-height`: `normal | <number [0,∞]> | <length-percentage [0,∞]>`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Normal,
    Number(f64),
    LengthPercentage(LengthPercentage),
}

impl Parse for LineHeight {
    /// Eine Zahl wird zuerst versucht, `0` ist also die Zahl 0 und keine
    /// Länge.
    fn parse(input: &mut Input<'_>) -> Option<LineHeight> {
        if input.keyword("normal") {
            return Some(LineHeight::Normal);
        }
        if let Some(number) = input.try_parse(|input| input.number().filter(|value| *value >= 0.0))
        {
            return Some(LineHeight::Number(number));
        }
        non_negative_length_percentage(input).map(LineHeight::LengthPercentage)
    }
}

impl ToCss for LineHeight {
    fn to_css(&self, out: &mut String) {
        match self {
            LineHeight::Normal => out.push_str("normal"),
            LineHeight::Number(number) => write_number(out, *number),
            LineHeight::LengthPercentage(value) => value.to_css(out),
        }
    }
}

keywords! {
    /// Die Schlüsselwörter von `text-align-all`.
    pub enum TextAlignKeyword {
        Start = "start",
        End = "end",
        Left = "left",
        Right = "right",
        Center = "center",
        Justify = "justify",
        MatchParent = "match-parent",
    }
}

/// `text-align-all`: ein Schlüsselwort oder ⟨string⟩
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextAlignAll {
    Keyword(TextAlignKeyword),
    /// Das Zeichen, an dem Tabellenzellen ausgerichtet werden.
    String(String),
}

impl Parse for TextAlignAll {
    /// Die Zeichenkette muss „a single character“ sein. CSS Text 4 legt
    /// nicht fest, ob ein Codepunkt oder ein Graphem-Cluster gemeint ist;
    /// bis die Segmentierung nach UAX #29 vorhanden ist, gilt hier ein
    /// Codepunkt.
    fn parse(input: &mut Input<'_>) -> Option<TextAlignAll> {
        if let Some(keyword) = TextAlignKeyword::parse(input) {
            return Some(TextAlignAll::Keyword(keyword));
        }
        input.try_parse(|input| {
            let text = input.string()?;
            let mut chars = text.chars();
            (chars.next().is_some() && chars.next().is_none())
                .then(|| TextAlignAll::String(text.to_owned()))
        })
    }
}

impl ToCss for TextAlignAll {
    fn to_css(&self, out: &mut String) {
        match self {
            TextAlignAll::Keyword(keyword) => keyword.to_css(out),
            TextAlignAll::String(text) => write_string(out, text),
        }
    }
}

keywords! {
    /// `text-align-last`
    pub enum TextAlignLast {
        Auto = "auto",
        Start = "start",
        End = "end",
        Left = "left",
        Right = "right",
        Center = "center",
        Justify = "justify",
        MatchParent = "match-parent",
    }
}

/// `text-indent`: `[ <length-percentage> ] && hanging? && each-line?`
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextIndent {
    pub length: LengthPercentage,
    pub hanging: bool,
    pub each_line: bool,
}

impl Parse for TextIndent {
    fn parse(input: &mut Input<'_>) -> Option<TextIndent> {
        input.try_parse(|input| {
            let mut length = None;
            let mut hanging = false;
            let mut each_line = false;
            loop {
                if length.is_none()
                    && let Some(value) = LengthPercentage::parse(input)
                {
                    length = Some(value);
                } else if !hanging && input.keyword("hanging") {
                    hanging = true;
                } else if !each_line && input.keyword("each-line") {
                    each_line = true;
                } else {
                    break;
                }
            }
            Some(TextIndent {
                length: length?,
                hanging,
                each_line,
            })
        })
    }
}

impl ToCss for TextIndent {
    fn to_css(&self, out: &mut String) {
        self.length.to_css(out);
        if self.hanging {
            out.push_str(" hanging");
        }
        if self.each_line {
            out.push_str(" each-line");
        }
    }
}

keywords! {
    /// Die Groß- und Kleinschreibung in `text-transform`.
    pub enum TextTransformCase {
        Capitalize = "capitalize",
        Uppercase = "uppercase",
        Lowercase = "lowercase",
    }
}

/// `text-transform`: `none | [capitalize | uppercase | lowercase ] ||
/// full-width || full-size-kana | math-auto`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextTransform {
    None,
    /// Mindestens eines der Teile ist gesetzt.
    Transforms {
        case: Option<TextTransformCase>,
        full_width: bool,
        full_size_kana: bool,
    },
    MathAuto,
}

impl Parse for TextTransform {
    fn parse(input: &mut Input<'_>) -> Option<TextTransform> {
        if input.keyword("none") {
            return Some(TextTransform::None);
        }
        if input.keyword("math-auto") {
            return Some(TextTransform::MathAuto);
        }
        let mut case = None;
        let mut full_width = false;
        let mut full_size_kana = false;
        loop {
            if case.is_none()
                && let Some(value) = TextTransformCase::parse(input)
            {
                case = Some(value);
            } else if !full_width && input.keyword("full-width") {
                full_width = true;
            } else if !full_size_kana && input.keyword("full-size-kana") {
                full_size_kana = true;
            } else {
                break;
            }
        }
        (case.is_some() || full_width || full_size_kana).then_some(TextTransform::Transforms {
            case,
            full_width,
            full_size_kana,
        })
    }
}

impl ToCss for TextTransform {
    fn to_css(&self, out: &mut String) {
        match self {
            TextTransform::None => out.push_str("none"),
            TextTransform::MathAuto => out.push_str("math-auto"),
            TextTransform::Transforms {
                case,
                full_width,
                full_size_kana,
            } => {
                let mut parts = Vec::new();
                if let Some(case) = case {
                    parts.push(case.name());
                }
                if *full_width {
                    parts.push("full-width");
                }
                if *full_size_kana {
                    parts.push("full-size-kana");
                }
                out.push_str(&parts.join(" "));
            }
        }
    }
}

keywords! {
    /// `white-space-collapse`
    pub enum WhiteSpaceCollapse {
        Collapse = "collapse",
        Discard = "discard",
        Preserve = "preserve",
        PreserveBreaks = "preserve-breaks",
        PreserveSpaces = "preserve-spaces",
        BreakSpaces = "break-spaces",
    }
}

keywords! {
    /// `text-wrap-mode`
    pub enum TextWrapMode {
        Wrap = "wrap",
        Nowrap = "nowrap",
    }
}

/// `white-space-trim`: `none | discard-before || discard-after ||
/// discard-inner`; `none`, wenn nichts gesetzt ist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WhiteSpaceTrim {
    pub discard_before: bool,
    pub discard_after: bool,
    pub discard_inner: bool,
}

impl Parse for WhiteSpaceTrim {
    fn parse(input: &mut Input<'_>) -> Option<WhiteSpaceTrim> {
        if input.keyword("none") {
            return Some(WhiteSpaceTrim::default());
        }
        let mut trim = WhiteSpaceTrim::default();
        loop {
            if !trim.discard_before && input.keyword("discard-before") {
                trim.discard_before = true;
            } else if !trim.discard_after && input.keyword("discard-after") {
                trim.discard_after = true;
            } else if !trim.discard_inner && input.keyword("discard-inner") {
                trim.discard_inner = true;
            } else {
                break;
            }
        }
        (trim != WhiteSpaceTrim::default()).then_some(trim)
    }
}

impl ToCss for WhiteSpaceTrim {
    fn to_css(&self, out: &mut String) {
        let parts: Vec<&str> = [
            (self.discard_before, "discard-before"),
            (self.discard_after, "discard-after"),
            (self.discard_inner, "discard-inner"),
        ]
        .into_iter()
        .filter_map(|(set, name)| set.then_some(name))
        .collect();
        if parts.is_empty() {
            out.push_str("none");
        } else {
            out.push_str(&parts.join(" "));
        }
    }
}

keywords! {
    /// `text-wrap-style`
    pub enum TextWrapStyle {
        Auto = "auto",
        Balance = "balance",
        Stable = "stable",
        Pretty = "pretty",
        AvoidShortLastLine = "avoid-short-last-line",
    }
}

/// `letter-spacing` und `word-spacing`: `normal | <length-percentage>`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spacing {
    Normal,
    LengthPercentage(LengthPercentage),
}

impl Parse for Spacing {
    fn parse(input: &mut Input<'_>) -> Option<Spacing> {
        if input.keyword("normal") {
            return Some(Spacing::Normal);
        }
        LengthPercentage::parse(input).map(Spacing::LengthPercentage)
    }
}

impl ToCss for Spacing {
    fn to_css(&self, out: &mut String) {
        match self {
            Spacing::Normal => out.push_str("normal"),
            Spacing::LengthPercentage(value) => value.to_css(out),
        }
    }
}

/// Die Linien aus `[ underline || overline || line-through || blink ]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DecorationLines {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
    pub blink: bool,
}

/// `text-decoration-line`: `none | [ underline || overline ||
/// line-through || blink ] | spelling-error | grammar-error`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextDecorationLine {
    None,
    /// Mindestens eine Linie ist gesetzt.
    Lines(DecorationLines),
    SpellingError,
    GrammarError,
}

impl Parse for TextDecorationLine {
    fn parse(input: &mut Input<'_>) -> Option<TextDecorationLine> {
        if input.keyword("none") {
            return Some(TextDecorationLine::None);
        }
        if input.keyword("spelling-error") {
            return Some(TextDecorationLine::SpellingError);
        }
        if input.keyword("grammar-error") {
            return Some(TextDecorationLine::GrammarError);
        }
        let mut lines = DecorationLines::default();
        loop {
            if !lines.underline && input.keyword("underline") {
                lines.underline = true;
            } else if !lines.overline && input.keyword("overline") {
                lines.overline = true;
            } else if !lines.line_through && input.keyword("line-through") {
                lines.line_through = true;
            } else if !lines.blink && input.keyword("blink") {
                lines.blink = true;
            } else {
                break;
            }
        }
        (lines != DecorationLines::default()).then_some(TextDecorationLine::Lines(lines))
    }
}

impl ToCss for TextDecorationLine {
    fn to_css(&self, out: &mut String) {
        match self {
            TextDecorationLine::None => out.push_str("none"),
            TextDecorationLine::SpellingError => out.push_str("spelling-error"),
            TextDecorationLine::GrammarError => out.push_str("grammar-error"),
            TextDecorationLine::Lines(lines) => {
                let parts: Vec<&str> = [
                    (lines.underline, "underline"),
                    (lines.overline, "overline"),
                    (lines.line_through, "line-through"),
                    (lines.blink, "blink"),
                ]
                .into_iter()
                .filter_map(|(set, name)| set.then_some(name))
                .collect();
                out.push_str(&parts.join(" "));
            }
        }
    }
}

keywords! {
    /// `text-decoration-style`
    pub enum TextDecorationStyle {
        Solid = "solid",
        Double = "double",
        Dotted = "dotted",
        Dashed = "dashed",
        Wavy = "wavy",
    }
}

/// `text-decoration-thickness`: `auto | from-font | <length-percentage> |
/// <line-width>`. Die Längen aus ⟨line-width⟩ sind schon in
/// ⟨length-percentage⟩ enthalten, es bleiben dessen Schlüsselwörter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextDecorationThickness {
    Auto,
    FromFont,
    LengthPercentage(LengthPercentage),
    Keyword(LineWidthKeyword),
}

impl Parse for TextDecorationThickness {
    fn parse(input: &mut Input<'_>) -> Option<TextDecorationThickness> {
        if input.keyword("auto") {
            return Some(TextDecorationThickness::Auto);
        }
        if input.keyword("from-font") {
            return Some(TextDecorationThickness::FromFont);
        }
        if let Some(keyword) = LineWidthKeyword::parse(input) {
            return Some(TextDecorationThickness::Keyword(keyword));
        }
        LengthPercentage::parse(input).map(TextDecorationThickness::LengthPercentage)
    }
}

impl ToCss for TextDecorationThickness {
    fn to_css(&self, out: &mut String) {
        match self {
            TextDecorationThickness::Auto => out.push_str("auto"),
            TextDecorationThickness::FromFont => out.push_str("from-font"),
            TextDecorationThickness::LengthPercentage(value) => value.to_css(out),
            TextDecorationThickness::Keyword(keyword) => keyword.to_css(out),
        }
    }
}

/// Die Namen der Zählerstile aus CSS Counter Styles 3 (Abschnitte 6 und
/// 7). Sie werden beim Parsen klein geschrieben, alle anderen Namen bleiben,
/// wie sie sind („Counter Style Names“).
const PREDEFINED_COUNTER_STYLES: [&str; 55] = [
    "decimal",
    "decimal-leading-zero",
    "arabic-indic",
    "armenian",
    "upper-armenian",
    "lower-armenian",
    "bengali",
    "cambodian",
    "khmer",
    "cjk-decimal",
    "devanagari",
    "georgian",
    "gujarati",
    "gurmukhi",
    "hebrew",
    "kannada",
    "lao",
    "malayalam",
    "mongolian",
    "myanmar",
    "oriya",
    "persian",
    "lower-roman",
    "upper-roman",
    "tamil",
    "telugu",
    "thai",
    "tibetan",
    "lower-alpha",
    "lower-latin",
    "upper-alpha",
    "upper-latin",
    "lower-greek",
    "hiragana",
    "hiragana-iroha",
    "katakana",
    "katakana-iroha",
    "disc",
    "circle",
    "square",
    "disclosure-open",
    "disclosure-closed",
    "cjk-earthly-branch",
    "cjk-heavenly-stem",
    "japanese-informal",
    "japanese-formal",
    "korean-hangul-formal",
    "korean-hanja-informal",
    "korean-hanja-formal",
    "simp-chinese-informal",
    "simp-chinese-formal",
    "trad-chinese-informal",
    "trad-chinese-formal",
    "cjk-ideographic",
    "ethiopic-numeric",
];

/// `list-style-type`: `<counter-style> | <string> | none`. Von
/// ⟨counter-style⟩ fehlt noch `symbols()`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ListStyleType {
    None,
    /// Ein ⟨counter-style-name⟩.
    CounterStyle(String),
    String(String),
}

impl ListStyleType {
    pub const DISC: &'static str = "disc";
}

impl Parse for ListStyleType {
    fn parse(input: &mut Input<'_>) -> Option<ListStyleType> {
        if let Some(text) = input.string() {
            return Some(ListStyleType::String(text.to_owned()));
        }
        if input.keyword("none") {
            return Some(ListStyleType::None);
        }
        let name = custom_ident(input, &["none"])?;
        let name = match PREDEFINED_COUNTER_STYLES
            .iter()
            .find(|predefined| name.eq_ignore_ascii_case(predefined))
        {
            Some(predefined) => (*predefined).to_owned(),
            None => name.to_owned(),
        };
        Some(ListStyleType::CounterStyle(name))
    }
}

impl ToCss for ListStyleType {
    fn to_css(&self, out: &mut String) {
        match self {
            ListStyleType::None => out.push_str("none"),
            ListStyleType::CounterStyle(name) => write_identifier(out, name),
            ListStyleType::String(text) => write_string(out, text),
        }
    }
}

keywords! {
    /// `list-style-position`
    pub enum ListStylePosition {
        Inside = "inside",
        Outside = "outside",
    }
}

/// `list-style-image`: `<image> | none`. Von ⟨image⟩ gibt es bisher nur
/// ⟨url⟩, Verläufe fehlen noch.
#[derive(Clone, Debug, PartialEq)]
pub enum ListStyleImage {
    None,
    Url(Box<SpecifiedUrl>),
}

impl Parse for ListStyleImage {
    fn parse(input: &mut Input<'_>) -> Option<ListStyleImage> {
        if input.keyword("none") {
            return Some(ListStyleImage::None);
        }
        SpecifiedUrl::parse(input).map(|url| ListStyleImage::Url(Box::new(url)))
    }
}

impl ToCss for ListStyleImage {
    fn to_css(&self, out: &mut String) {
        match self {
            ListStyleImage::None => out.push_str("none"),
            ListStyleImage::Url(url) => url.to_css(out),
        }
    }
}

keywords! {
    /// `baseline-source`
    pub enum BaselineSource {
        Auto = "auto",
        First = "first",
        Last = "last",
    }
}

keywords! {
    /// `alignment-baseline`: `baseline | <baseline-metric>`
    pub enum AlignmentBaseline {
        Baseline = "baseline",
        TextBottom = "text-bottom",
        Alphabetic = "alphabetic",
        Ideographic = "ideographic",
        Middle = "middle",
        Central = "central",
        Mathematical = "mathematical",
        Hanging = "hanging",
        TextTop = "text-top",
    }
}

/// `baseline-shift`: `<length-percentage> | sub | super | top | center |
/// bottom`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BaselineShift {
    LengthPercentage(LengthPercentage),
    Sub,
    Super,
    Top,
    Center,
    Bottom,
}

impl Parse for BaselineShift {
    fn parse(input: &mut Input<'_>) -> Option<BaselineShift> {
        for (keyword, shift) in [
            ("sub", BaselineShift::Sub),
            ("super", BaselineShift::Super),
            ("top", BaselineShift::Top),
            ("center", BaselineShift::Center),
            ("bottom", BaselineShift::Bottom),
        ] {
            if input.keyword(keyword) {
                return Some(shift);
            }
        }
        LengthPercentage::parse(input).map(BaselineShift::LengthPercentage)
    }
}

impl ToCss for BaselineShift {
    fn to_css(&self, out: &mut String) {
        match self {
            BaselineShift::LengthPercentage(value) => value.to_css(out),
            BaselineShift::Sub => out.push_str("sub"),
            BaselineShift::Super => out.push_str("super"),
            BaselineShift::Top => out.push_str("top"),
            BaselineShift::Center => out.push_str("center"),
            BaselineShift::Bottom => out.push_str("bottom"),
        }
    }
}

keywords! {
    /// `direction`
    pub enum Direction {
        Ltr = "ltr",
        Rtl = "rtl",
    }
}

keywords! {
    /// `unicode-bidi`
    pub enum UnicodeBidi {
        Normal = "normal",
        Embed = "embed",
        Isolate = "isolate",
        BidiOverride = "bidi-override",
        IsolateOverride = "isolate-override",
        Plaintext = "plaintext",
    }
}

keywords! {
    /// `writing-mode`
    pub enum WritingMode {
        HorizontalTb = "horizontal-tb",
        VerticalRl = "vertical-rl",
        VerticalLr = "vertical-lr",
        SidewaysRl = "sideways-rl",
        SidewaysLr = "sideways-lr",
    }
}

keywords! {
    /// `text-orientation`
    pub enum TextOrientation {
        Mixed = "mixed",
        Upright = "upright",
        Sideways = "sideways",
    }
}
