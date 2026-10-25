//! Farben nach CSS Color Level 4 (Editor's Draft, Stand 23.10.2026): das
//! Parsen von ⟨color⟩ und das Serialisieren des deklarierten Werts
//! („Resolving Color Values“, „Serializing ⟨color⟩ Values“).
//!
//! Die sRGB-Farben der Altsyntax (Hex-Farben, `rgb()`, `rgba()`, `hsl()`,
//! `hsla()` und `hwb()` ohne fehlende Komponenten) werden wie in Browsern
//! mit 8 Bit je Kanal gespeichert ([`Rgba`]), Hälften aufwärts gerundet.
//! Die Spezifikation empfiehlt, die Genauigkeit zu erhalten („should
//! honor“), schreibt sie aber nicht vor; die WPT erwarten für `hsl()` und
//! `hwb()` ganzzahlige Kanäle. Mit fehlenden Komponenten (`none`) bleiben
//! `hsl()` und `hwb()` in ihrer eigenen Form, `rgb()` wird zu
//! `color(srgb …)`, wie es die Serialisierung verlangt.
//!
//! Es fehlen noch `color-mix()`, `light-dark()`, `contrast-color()`,
//! `alpha()` und die relative Farbsyntax (`from`) aus CSS Color 5 sowie
//! `device-cmyk()`.

use mb_css::{ComponentValue, Token};

use super::{Angle, Input, Parse, ToCss, keywords, write_number, write_significant};

/// Ein ⟨color⟩ als deklarierter Wert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    /// `currentcolor`
    CurrentColor,
    /// `transparent`
    Transparent,
    Named(NamedColor),
    /// Ein ⟨system-color⟩ oder ⟨deprecated-color⟩.
    System(SystemColor),
    /// Eine sRGB-Farbe der Altsyntax ohne fehlende Komponenten.
    Rgba(Rgba),
    /// Jede andere Farbfunktion.
    Function(ColorFunction),
}

/// Eine sRGB-Farbe mit 8 Bit je Kanal, auch für Alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Rgba {
    /// Aus Kanälen im Bereich [0, 255] und Alpha in [0, 1]; Werte außerhalb
    /// werden begrenzt.
    fn from_channels(channels: [f64; 3], alpha: f64) -> Rgba {
        let [red, green, blue] = channels.map(to_byte);
        Rgba {
            red,
            green,
            blue,
            alpha: to_byte(alpha * 255.0),
        }
    }
}

/// Rundet einen Kanal auf eine ganze Zahl in [0, 255], Hälften aufwärts.
/// Rundungsfehler der Umrechnung unter 10⁻⁹ gelten dabei als genau eine
/// Hälfte, damit etwa der Blaukanal von `hwb(320deg 30% 40%)` (genau 127,5)
/// zu 128 wird.
fn to_byte(value: f64) -> u8 {
    let value = if value.is_nan() { 0.0 } else { value };
    // Nach dem Begrenzen liegt der Wert in [0, 255]; `as` schneidet dann
    // nichts ab.
    (value.clamp(0.0, 255.0) + 0.5 + 1e-9).floor().min(255.0) as u8
}

/// Eine Farbfunktion mit Komponenten, wie sie der deklarierte Wert behält.
/// `None` steht für eine fehlende Komponente (`none`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorFunction {
    pub space: ColorSpace,
    /// Die Komponenten als Zahlen in den Referenzbereichen der Funktion,
    /// Prozente schon umgerechnet, Farbtöne in Grad und auf [0, 360)
    /// gebracht.
    pub components: [Option<f64>; 3],
    /// Alpha in [0, 1].
    pub alpha: Option<f64>,
}

/// Der Farbraum einer Farbfunktion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// `hsl()` mit fehlenden Komponenten
    Hsl,
    /// `hwb()` mit fehlenden Komponenten
    Hwb,
    Lab,
    Lch,
    Oklab,
    Oklch,
    /// `color(srgb …)`, auch für `rgb()` mit fehlenden Komponenten
    Srgb,
    SrgbLinear,
    DisplayP3,
    DisplayP3Linear,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    /// `xyz-d65` und `xyz`
    XyzD65,
}

/// Die Räume von `color()` mit ihrem Namen. `xyz` ist ein anderer Name für
/// `xyz-d65` und wird auch so serialisiert.
const PREDEFINED_SPACES: [(&str, ColorSpace); 10] = [
    ("srgb", ColorSpace::Srgb),
    ("srgb-linear", ColorSpace::SrgbLinear),
    ("display-p3", ColorSpace::DisplayP3),
    ("display-p3-linear", ColorSpace::DisplayP3Linear),
    ("a98-rgb", ColorSpace::A98Rgb),
    ("prophoto-rgb", ColorSpace::ProphotoRgb),
    ("rec2020", ColorSpace::Rec2020),
    ("xyz-d50", ColorSpace::XyzD50),
    ("xyz-d65", ColorSpace::XyzD65),
    ("xyz", ColorSpace::XyzD65),
];

/// Die benannten Farben (Abschnitt 6.1) mit ihren sRGB-Werten, sortiert.
const NAMED_COLORS: [(&str, [u8; 3]); 148] = [
    ("aliceblue", [0xf0, 0xf8, 0xff]),
    ("antiquewhite", [0xfa, 0xeb, 0xd7]),
    ("aqua", [0x00, 0xff, 0xff]),
    ("aquamarine", [0x7f, 0xff, 0xd4]),
    ("azure", [0xf0, 0xff, 0xff]),
    ("beige", [0xf5, 0xf5, 0xdc]),
    ("bisque", [0xff, 0xe4, 0xc4]),
    ("black", [0x00, 0x00, 0x00]),
    ("blanchedalmond", [0xff, 0xeb, 0xcd]),
    ("blue", [0x00, 0x00, 0xff]),
    ("blueviolet", [0x8a, 0x2b, 0xe2]),
    ("brown", [0xa5, 0x2a, 0x2a]),
    ("burlywood", [0xde, 0xb8, 0x87]),
    ("cadetblue", [0x5f, 0x9e, 0xa0]),
    ("chartreuse", [0x7f, 0xff, 0x00]),
    ("chocolate", [0xd2, 0x69, 0x1e]),
    ("coral", [0xff, 0x7f, 0x50]),
    ("cornflowerblue", [0x64, 0x95, 0xed]),
    ("cornsilk", [0xff, 0xf8, 0xdc]),
    ("crimson", [0xdc, 0x14, 0x3c]),
    ("cyan", [0x00, 0xff, 0xff]),
    ("darkblue", [0x00, 0x00, 0x8b]),
    ("darkcyan", [0x00, 0x8b, 0x8b]),
    ("darkgoldenrod", [0xb8, 0x86, 0x0b]),
    ("darkgray", [0xa9, 0xa9, 0xa9]),
    ("darkgreen", [0x00, 0x64, 0x00]),
    ("darkgrey", [0xa9, 0xa9, 0xa9]),
    ("darkkhaki", [0xbd, 0xb7, 0x6b]),
    ("darkmagenta", [0x8b, 0x00, 0x8b]),
    ("darkolivegreen", [0x55, 0x6b, 0x2f]),
    ("darkorange", [0xff, 0x8c, 0x00]),
    ("darkorchid", [0x99, 0x32, 0xcc]),
    ("darkred", [0x8b, 0x00, 0x00]),
    ("darksalmon", [0xe9, 0x96, 0x7a]),
    ("darkseagreen", [0x8f, 0xbc, 0x8f]),
    ("darkslateblue", [0x48, 0x3d, 0x8b]),
    ("darkslategray", [0x2f, 0x4f, 0x4f]),
    ("darkslategrey", [0x2f, 0x4f, 0x4f]),
    ("darkturquoise", [0x00, 0xce, 0xd1]),
    ("darkviolet", [0x94, 0x00, 0xd3]),
    ("deeppink", [0xff, 0x14, 0x93]),
    ("deepskyblue", [0x00, 0xbf, 0xff]),
    ("dimgray", [0x69, 0x69, 0x69]),
    ("dimgrey", [0x69, 0x69, 0x69]),
    ("dodgerblue", [0x1e, 0x90, 0xff]),
    ("firebrick", [0xb2, 0x22, 0x22]),
    ("floralwhite", [0xff, 0xfa, 0xf0]),
    ("forestgreen", [0x22, 0x8b, 0x22]),
    ("fuchsia", [0xff, 0x00, 0xff]),
    ("gainsboro", [0xdc, 0xdc, 0xdc]),
    ("ghostwhite", [0xf8, 0xf8, 0xff]),
    ("gold", [0xff, 0xd7, 0x00]),
    ("goldenrod", [0xda, 0xa5, 0x20]),
    ("gray", [0x80, 0x80, 0x80]),
    ("green", [0x00, 0x80, 0x00]),
    ("greenyellow", [0xad, 0xff, 0x2f]),
    ("grey", [0x80, 0x80, 0x80]),
    ("honeydew", [0xf0, 0xff, 0xf0]),
    ("hotpink", [0xff, 0x69, 0xb4]),
    ("indianred", [0xcd, 0x5c, 0x5c]),
    ("indigo", [0x4b, 0x00, 0x82]),
    ("ivory", [0xff, 0xff, 0xf0]),
    ("khaki", [0xf0, 0xe6, 0x8c]),
    ("lavender", [0xe6, 0xe6, 0xfa]),
    ("lavenderblush", [0xff, 0xf0, 0xf5]),
    ("lawngreen", [0x7c, 0xfc, 0x00]),
    ("lemonchiffon", [0xff, 0xfa, 0xcd]),
    ("lightblue", [0xad, 0xd8, 0xe6]),
    ("lightcoral", [0xf0, 0x80, 0x80]),
    ("lightcyan", [0xe0, 0xff, 0xff]),
    ("lightgoldenrodyellow", [0xfa, 0xfa, 0xd2]),
    ("lightgray", [0xd3, 0xd3, 0xd3]),
    ("lightgreen", [0x90, 0xee, 0x90]),
    ("lightgrey", [0xd3, 0xd3, 0xd3]),
    ("lightpink", [0xff, 0xb6, 0xc1]),
    ("lightsalmon", [0xff, 0xa0, 0x7a]),
    ("lightseagreen", [0x20, 0xb2, 0xaa]),
    ("lightskyblue", [0x87, 0xce, 0xfa]),
    ("lightslategray", [0x77, 0x88, 0x99]),
    ("lightslategrey", [0x77, 0x88, 0x99]),
    ("lightsteelblue", [0xb0, 0xc4, 0xde]),
    ("lightyellow", [0xff, 0xff, 0xe0]),
    ("lime", [0x00, 0xff, 0x00]),
    ("limegreen", [0x32, 0xcd, 0x32]),
    ("linen", [0xfa, 0xf0, 0xe6]),
    ("magenta", [0xff, 0x00, 0xff]),
    ("maroon", [0x80, 0x00, 0x00]),
    ("mediumaquamarine", [0x66, 0xcd, 0xaa]),
    ("mediumblue", [0x00, 0x00, 0xcd]),
    ("mediumorchid", [0xba, 0x55, 0xd3]),
    ("mediumpurple", [0x93, 0x70, 0xdb]),
    ("mediumseagreen", [0x3c, 0xb3, 0x71]),
    ("mediumslateblue", [0x7b, 0x68, 0xee]),
    ("mediumspringgreen", [0x00, 0xfa, 0x9a]),
    ("mediumturquoise", [0x48, 0xd1, 0xcc]),
    ("mediumvioletred", [0xc7, 0x15, 0x85]),
    ("midnightblue", [0x19, 0x19, 0x70]),
    ("mintcream", [0xf5, 0xff, 0xfa]),
    ("mistyrose", [0xff, 0xe4, 0xe1]),
    ("moccasin", [0xff, 0xe4, 0xb5]),
    ("navajowhite", [0xff, 0xde, 0xad]),
    ("navy", [0x00, 0x00, 0x80]),
    ("oldlace", [0xfd, 0xf5, 0xe6]),
    ("olive", [0x80, 0x80, 0x00]),
    ("olivedrab", [0x6b, 0x8e, 0x23]),
    ("orange", [0xff, 0xa5, 0x00]),
    ("orangered", [0xff, 0x45, 0x00]),
    ("orchid", [0xda, 0x70, 0xd6]),
    ("palegoldenrod", [0xee, 0xe8, 0xaa]),
    ("palegreen", [0x98, 0xfb, 0x98]),
    ("paleturquoise", [0xaf, 0xee, 0xee]),
    ("palevioletred", [0xdb, 0x70, 0x93]),
    ("papayawhip", [0xff, 0xef, 0xd5]),
    ("peachpuff", [0xff, 0xda, 0xb9]),
    ("peru", [0xcd, 0x85, 0x3f]),
    ("pink", [0xff, 0xc0, 0xcb]),
    ("plum", [0xdd, 0xa0, 0xdd]),
    ("powderblue", [0xb0, 0xe0, 0xe6]),
    ("purple", [0x80, 0x00, 0x80]),
    ("rebeccapurple", [0x66, 0x33, 0x99]),
    ("red", [0xff, 0x00, 0x00]),
    ("rosybrown", [0xbc, 0x8f, 0x8f]),
    ("royalblue", [0x41, 0x69, 0xe1]),
    ("saddlebrown", [0x8b, 0x45, 0x13]),
    ("salmon", [0xfa, 0x80, 0x72]),
    ("sandybrown", [0xf4, 0xa4, 0x60]),
    ("seagreen", [0x2e, 0x8b, 0x57]),
    ("seashell", [0xff, 0xf5, 0xee]),
    ("sienna", [0xa0, 0x52, 0x2d]),
    ("silver", [0xc0, 0xc0, 0xc0]),
    ("skyblue", [0x87, 0xce, 0xeb]),
    ("slateblue", [0x6a, 0x5a, 0xcd]),
    ("slategray", [0x70, 0x80, 0x90]),
    ("slategrey", [0x70, 0x80, 0x90]),
    ("snow", [0xff, 0xfa, 0xfa]),
    ("springgreen", [0x00, 0xff, 0x7f]),
    ("steelblue", [0x46, 0x82, 0xb4]),
    ("tan", [0xd2, 0xb4, 0x8c]),
    ("teal", [0x00, 0x80, 0x80]),
    ("thistle", [0xd8, 0xbf, 0xd8]),
    ("tomato", [0xff, 0x63, 0x47]),
    ("turquoise", [0x40, 0xe0, 0xd0]),
    ("violet", [0xee, 0x82, 0xee]),
    ("wheat", [0xf5, 0xde, 0xb3]),
    ("white", [0xff, 0xff, 0xff]),
    ("whitesmoke", [0xf5, 0xf5, 0xf5]),
    ("yellow", [0xff, 0xff, 0x00]),
    ("yellowgreen", [0x9a, 0xcd, 0x32]),
];

/// Eine benannte Farbe, als Index in die Tabelle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NamedColor(usize);

impl NamedColor {
    fn from_ident(ident: &str) -> Option<NamedColor> {
        NAMED_COLORS
            .iter()
            .position(|(name, _)| ident.eq_ignore_ascii_case(name))
            .map(NamedColor)
    }

    pub fn name(self) -> &'static str {
        NAMED_COLORS.get(self.0).map_or("black", |(name, _)| name)
    }

    pub fn rgb(self) -> [u8; 3] {
        NAMED_COLORS.get(self.0).map_or([0; 3], |(_, rgb)| *rgb)
    }
}

keywords! {
    /// Die ⟨system-color⟩s und die ⟨deprecated-color⟩s (Abschnitt 6.2 und
    /// Anhang A).
    pub enum SystemColor {
        AccentColor = "accentcolor",
        AccentColorText = "accentcolortext",
        ActiveText = "activetext",
        ButtonBorder = "buttonborder",
        ButtonFace = "buttonface",
        ButtonText = "buttontext",
        Canvas = "canvas",
        CanvasText = "canvastext",
        Field = "field",
        FieldText = "fieldtext",
        GrayText = "graytext",
        Highlight = "highlight",
        HighlightText = "highlighttext",
        LinkText = "linktext",
        Mark = "mark",
        MarkText = "marktext",
        SelectedItem = "selecteditem",
        SelectedItemText = "selecteditemtext",
        VisitedText = "visitedtext",
        ActiveBorder = "activeborder",
        ActiveCaption = "activecaption",
        AppWorkspace = "appworkspace",
        Background = "background",
        ButtonHighlight = "buttonhighlight",
        ButtonShadow = "buttonshadow",
        CaptionText = "captiontext",
        InactiveBorder = "inactiveborder",
        InactiveCaption = "inactivecaption",
        InactiveCaptionText = "inactivecaptiontext",
        InfoBackground = "infobackground",
        InfoText = "infotext",
        Menu = "menu",
        MenuText = "menutext",
        Scrollbar = "scrollbar",
        ThreeDDarkShadow = "threeddarkshadow",
        ThreeDFace = "threedface",
        ThreeDHighlight = "threedhighlight",
        ThreeDLightShadow = "threedlightshadow",
        ThreeDShadow = "threedshadow",
        Window = "window",
        WindowFrame = "windowframe",
        WindowText = "windowtext",
    }
}

impl SystemColor {
    /// Die Systemfarbe, deren Wert eine ⟨deprecated-color⟩ annimmt
    /// (Anhang A); andere bleiben, wie sie sind.
    pub fn undeprecated(self) -> SystemColor {
        use SystemColor::*;
        match self {
            ActiveBorder | InactiveBorder | ThreeDDarkShadow | ThreeDHighlight
            | ThreeDLightShadow | ThreeDShadow | WindowFrame => ButtonBorder,
            ActiveCaption | AppWorkspace | Background | InactiveCaption | InfoBackground | Menu
            | Scrollbar | Window => Canvas,
            ButtonHighlight | ButtonShadow | ThreeDFace => ButtonFace,
            CaptionText | InfoText | MenuText | WindowText => CanvasText,
            InactiveCaptionText => GrayText,
            other => other,
        }
    }
}

impl Parse for Color {
    fn parse(input: &mut Input<'_>) -> Option<Color> {
        input.try_parse(|input| match input.next()? {
            ComponentValue::Token(Token::Ident(ident)) => keyword(ident),
            ComponentValue::Token(Token::Hash { value, .. }) => hex(value),
            ComponentValue::Function { name, value } => {
                let mut arguments = Input::new(value, input.context());
                let color = function(&name.to_ascii_lowercase(), &mut arguments)?;
                arguments.finish(color)
            }
            _ => None,
        })
    }
}

fn keyword(ident: &str) -> Option<Color> {
    if ident.eq_ignore_ascii_case("currentcolor") {
        return Some(Color::CurrentColor);
    }
    if ident.eq_ignore_ascii_case("transparent") {
        return Some(Color::Transparent);
    }
    NamedColor::from_ident(ident)
        .map(Color::Named)
        .or_else(|| SystemColor::from_ident(ident).map(Color::System))
}

/// ⟨hex-color⟩: drei, vier, sechs oder acht hexadezimale Ziffern.
fn hex(value: &str) -> Option<Color> {
    let digits: Vec<u8> = value
        .chars()
        .map(|c| c.to_digit(16).and_then(|digit| u8::try_from(digit).ok()))
        .collect::<Option<_>>()?;
    let pair = |high: u8, low: u8| high.saturating_mul(16).saturating_add(low);
    let [red, green, blue, alpha] = match digits[..] {
        [r, g, b] => [r, g, b, 15].map(|digit| pair(digit, digit)),
        [r, g, b, a] => [r, g, b, a].map(|digit| pair(digit, digit)),
        [r1, r2, g1, g2, b1, b2] => [pair(r1, r2), pair(g1, g2), pair(b1, b2), 255],
        [r1, r2, g1, g2, b1, b2, a1, a2] => {
            [pair(r1, r2), pair(g1, g2), pair(b1, b2), pair(a1, a2)]
        }
        _ => return None,
    };
    Some(Color::Rgba(Rgba {
        red,
        green,
        blue,
        alpha,
    }))
}

fn function(name: &str, arguments: &mut Input<'_>) -> Option<Color> {
    match name {
        "rgb" | "rgba" => arguments
            .try_parse(legacy_rgb)
            .or_else(|| modern_rgb(arguments)),
        "hsl" | "hsla" => arguments
            .try_parse(legacy_hsl)
            .or_else(|| hsl_or_hwb(ColorSpace::Hsl, arguments)),
        "hwb" => hsl_or_hwb(ColorSpace::Hwb, arguments),
        "lab" => lab_like(ColorSpace::Lab, arguments),
        "lch" => lab_like(ColorSpace::Lch, arguments),
        "oklab" => lab_like(ColorSpace::Oklab, arguments),
        "oklch" => lab_like(ColorSpace::Oklch, arguments),
        "color" => color_function(arguments),
        _ => None,
    }
}

/// ⟨alpha-value⟩, auf [0, 1] begrenzt.
fn alpha_value(input: &mut Input<'_>) -> Option<f64> {
    let alpha = match input.number() {
        Some(number) => number,
        None => input.percentage()? / 100.0,
    };
    Some(alpha.clamp(0.0, 1.0))
}

/// `[ / [<alpha-value> | none] ]?` der modernen Syntax. Ohne Angabe ist
/// Alpha 1; `Some(None)` steht für `none`.
fn modern_alpha(input: &mut Input<'_>) -> Option<Option<f64>> {
    if !input.delim('/') {
        return Some(Some(1.0));
    }
    if input.keyword("none") {
        return Some(None);
    }
    alpha_value(input).map(Some)
}

/// Eine Komponente oder `none`: `Some(None)` für `none`.
fn component(
    input: &mut Input<'_>,
    parse: impl FnOnce(&mut Input<'_>) -> Option<f64>,
) -> Option<Option<f64>> {
    if input.keyword("none") {
        return Some(None);
    }
    parse(input).map(Some)
}

/// ⟨number⟩ oder ⟨percentage⟩, Prozente mit `percent` multipliziert.
fn number_or_percentage(input: &mut Input<'_>, percent: f64) -> Option<f64> {
    match input.number() {
        Some(number) => Some(number),
        None => input.percentage().map(|value| value * percent),
    }
}

/// ⟨hue⟩ in Grad, auf [0, 360) gebracht.
fn hue(input: &mut Input<'_>) -> Option<f64> {
    let degrees = match input.number() {
        Some(number) => number,
        None => Angle::parse(input)?.degrees(),
    };
    Some(normalize_hue(degrees))
}

fn normalize_hue(degrees: f64) -> f64 {
    let hue = degrees.rem_euclid(360.0);
    // Sehr kleine negative Werte ergeben genau 360.
    if hue >= 360.0 || hue.is_nan() {
        0.0
    } else {
        hue
    }
}

/// `rgb( <percentage>#{3} , <alpha-value>? ) | rgb( <number>#{3} ,
/// <alpha-value>? )`
fn legacy_rgb(input: &mut Input<'_>) -> Option<Color> {
    let percentages = matches!(
        input.peek(),
        Some(ComponentValue::Token(Token::Percentage { .. }))
    );
    let mut channels = [0.0; 3];
    for (index, channel) in channels.iter_mut().enumerate() {
        if index > 0 && !input.comma() {
            return None;
        }
        *channel = if percentages {
            input.percentage()? * 255.0 / 100.0
        } else {
            input.number()?
        };
    }
    let alpha = if input.comma() {
        alpha_value(input)?
    } else {
        1.0
    };
    input.finish(Color::Rgba(Rgba::from_channels(channels, alpha)))
}

/// `rgb( [<number> | <percentage> | none]{3} [ / [<alpha-value> | none] ]? )`
fn modern_rgb(input: &mut Input<'_>) -> Option<Color> {
    let mut channels = [None; 3];
    for channel in &mut channels {
        *channel = component(input, |input| number_or_percentage(input, 2.55))?
            .map(|value| value.clamp(0.0, 255.0));
    }
    let alpha = modern_alpha(input)?;
    let color = match (channels, alpha) {
        ([Some(red), Some(green), Some(blue)], Some(alpha)) => {
            Color::Rgba(Rgba::from_channels([red, green, blue], alpha))
        }
        _ => Color::Function(ColorFunction {
            space: ColorSpace::Srgb,
            components: channels.map(|channel| channel.map(|value| value / 255.0)),
            alpha,
        }),
    };
    input.finish(color)
}

/// `hsl( <hue>, <percentage>, <percentage>, <alpha-value>? )`
fn legacy_hsl(input: &mut Input<'_>) -> Option<Color> {
    let hue = hue(input)?;
    if !input.comma() {
        return None;
    }
    let saturation = input.percentage()?.max(0.0);
    if !input.comma() {
        return None;
    }
    let lightness = input.percentage()?;
    let alpha = if input.comma() {
        alpha_value(input)?
    } else {
        1.0
    };
    let channels = hsl_to_srgb(hue, saturation, lightness).map(|value| value * 255.0);
    input.finish(Color::Rgba(Rgba::from_channels(channels, alpha)))
}

/// Die moderne Syntax von `hsl()` und `hwb()`:
/// `[<hue> | none] [<percentage> | <number> | none]{2}
/// [ / [<alpha-value> | none] ]?`
fn hsl_or_hwb(space: ColorSpace, input: &mut Input<'_>) -> Option<Color> {
    let hue = component(input, hue)?;
    let mut first = component(input, |input| number_or_percentage(input, 1.0))?;
    let second = component(input, |input| number_or_percentage(input, 1.0))?;
    let alpha = modern_alpha(input)?;
    if space == ColorSpace::Hsl {
        // Negative Sättigung wird beim Parsen zu 0.
        first = first.map(|saturation| saturation.max(0.0));
    }
    let color = match (hue, first, second, alpha) {
        (Some(hue), Some(first), Some(second), Some(alpha)) => {
            let rgb = if space == ColorSpace::Hsl {
                hsl_to_srgb(hue, first, second)
            } else {
                hwb_to_srgb(hue, first, second)
            };
            Color::Rgba(Rgba::from_channels(rgb.map(|value| value * 255.0), alpha))
        }
        _ => Color::Function(ColorFunction {
            space,
            components: [hue, first, second],
            alpha,
        }),
    };
    input.finish(color)
}

/// `hslToRgb()` aus Abschnitt 7.1: Farbton in Grad, Sättigung und
/// Helligkeit in [0, 100]; liefert sRGB in [0, 1].
pub(crate) fn hsl_to_srgb(hue: f64, saturation: f64, lightness: f64) -> [f64; 3] {
    let saturation = saturation / 100.0;
    let lightness = lightness / 100.0;
    let f = |n: f64| {
        let k = (n + hue / 30.0) % 12.0;
        let a = saturation * lightness.min(1.0 - lightness);
        lightness - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

/// `hwbToRgb()` aus Abschnitt 8.1: Weiß und Schwarz in [0, 100].
pub(crate) fn hwb_to_srgb(hue: f64, white: f64, black: f64) -> [f64; 3] {
    let white = white / 100.0;
    let black = black / 100.0;
    if white + black >= 1.0 {
        let gray = white / (white + black);
        return [gray; 3];
    }
    hsl_to_srgb(hue, 100.0, 50.0).map(|value| value * (1.0 - white - black) + white)
}

/// `lab()`, `lch()`, `oklab()` und `oklch()`: drei Komponenten aus
/// ⟨percentage⟩, ⟨number⟩ oder `none`, bei den polaren Räumen zuletzt ein
/// ⟨hue⟩, dann Alpha. Helligkeit und Chroma werden begrenzt.
fn lab_like(space: ColorSpace, input: &mut Input<'_>) -> Option<Color> {
    // Je Raum: 100 % der Helligkeit, 100 % von a und b oder Chroma, die
    // größte Helligkeit.
    let (lightness_percent, percent, max_lightness) = match space {
        ColorSpace::Lab => (1.0, 1.25, 100.0),
        ColorSpace::Lch => (1.0, 1.5, 100.0),
        ColorSpace::Oklab | ColorSpace::Oklch => (0.01, 0.004, 1.0),
        _ => return None,
    };
    let polar = matches!(space, ColorSpace::Lch | ColorSpace::Oklch);
    let lightness = component(input, |input| {
        number_or_percentage(input, lightness_percent)
    })?
    .map(|value| value.clamp(0.0, max_lightness));
    let mut second = component(input, |input| number_or_percentage(input, percent))?;
    let third = if polar {
        // Negatives Chroma wird zu 0.
        second = second.map(|chroma| chroma.max(0.0));
        component(input, hue)?
    } else {
        component(input, |input| number_or_percentage(input, percent))?
    };
    let alpha = modern_alpha(input)?;
    input.finish(Color::Function(ColorFunction {
        space,
        components: [lightness, second, third],
        alpha,
    }))
}

/// `color( <colorspace-params> [ / [ <alpha-value> | none ] ]? )`.
/// Prozente stehen für 0 bis 1; die Komponenten werden nicht begrenzt.
fn color_function(input: &mut Input<'_>) -> Option<Color> {
    let name = input.ident()?;
    let space = PREDEFINED_SPACES
        .iter()
        .find(|(space, _)| name.eq_ignore_ascii_case(space))?
        .1;
    let mut components = [None; 3];
    for component_value in &mut components {
        *component_value = component(input, |input| number_or_percentage(input, 0.01))?;
    }
    let alpha = modern_alpha(input)?;
    input.finish(Color::Function(ColorFunction {
        space,
        components,
        alpha,
    }))
}

impl ToCss for Color {
    fn to_css(&self, out: &mut String) {
        match self {
            Color::CurrentColor => out.push_str("currentcolor"),
            Color::Transparent => out.push_str("transparent"),
            Color::Named(named) => out.push_str(named.name()),
            Color::System(system) => out.push_str(system.name()),
            Color::Rgba(rgba) => rgba.to_css(out),
            Color::Function(function) => function.to_css(out),
        }
    }
}

impl ToCss for Rgba {
    /// „CSS serialization of sRGB values“: `rgb()` oder `rgba()` mit Zahlen
    /// und Kommas, Alpha nur, wenn es nicht 1 ist.
    fn to_css(&self, out: &mut String) {
        let channels = format!("{}, {}, {}", self.red, self.green, self.blue);
        if self.alpha == 255 {
            out.push_str("rgb(");
            out.push_str(&channels);
        } else {
            out.push_str("rgba(");
            out.push_str(&channels);
            out.push_str(", ");
            write_legacy_alpha(out, self.alpha);
        }
        out.push(')');
    }
}

/// „Serializing legacy alpha values“ für 8-Bit-Alpha: die kürzeste Zahl mit
/// zwei Nachkommastellen, die wieder denselben Wert ergibt, sonst drei.
fn write_legacy_alpha(out: &mut String, alpha: u8) {
    let alpha = u32::from(alpha);
    // round(i * 2.55), Hälften aufwärts, ganzzahlig gerechnet.
    let percent = (0..=100u32).find(|i| (i * 255 + 50) / 100 == alpha);
    let rounded = match percent {
        Some(percent) => f64::from(percent) / 100.0,
        // round(alpha / 0.255) / 1000
        None => f64::from((alpha * 2000 + 255) / 510) / 1000.0,
    };
    write_number(out, rounded);
}

impl ToCss for ColorFunction {
    fn to_css(&self, out: &mut String) {
        let percentages = matches!(self.space, ColorSpace::Hsl | ColorSpace::Hwb);
        match self.space {
            ColorSpace::Hsl => out.push_str("hsl("),
            ColorSpace::Hwb => out.push_str("hwb("),
            ColorSpace::Lab => out.push_str("lab("),
            ColorSpace::Lch => out.push_str("lch("),
            ColorSpace::Oklab => out.push_str("oklab("),
            ColorSpace::Oklch => out.push_str("oklch("),
            space => {
                // `xyz` steht nach `xyz-d65`, wird also nie gewählt.
                let name = PREDEFINED_SPACES
                    .iter()
                    .find(|(_, found)| *found == space)
                    .map_or("srgb", |(name, _)| name);
                out.push_str("color(");
                out.push_str(name);
                out.push(' ');
            }
        }
        for (index, component) in self.components.iter().enumerate() {
            if index > 0 {
                out.push(' ');
            }
            match component {
                None => out.push_str("none"),
                Some(value) => {
                    write_significant(out, *value);
                    // Bei `hsl()` und `hwb()` sind die zweite und dritte
                    // Komponente Prozente.
                    if percentages && index > 0 {
                        out.push('%');
                    }
                }
            }
        }
        match self.alpha {
            Some(alpha) if alpha >= 1.0 => {}
            Some(alpha) => {
                out.push_str(" / ");
                write_number(out, alpha);
            }
            None => out.push_str(" / none"),
        }
        out.push(')');
    }
}

#[cfg(test)]
mod tests {
    use mb_css::{Source, parse_component_value_list};

    use super::super::ParseContext;
    use super::*;

    fn color(text: &str) -> Option<String> {
        let values = parse_component_value_list(&Source::new(text));
        let mut input = Input::new(values.values(), ParseContext::default());
        let color = Color::parse(&mut input)?;
        input.finish(color.to_css_string())
    }

    #[test]
    fn keywords_and_hex() {
        assert_eq!(color("RED").as_deref(), Some("red"));
        assert_eq!(color("CurrentColor").as_deref(), Some("currentcolor"));
        assert_eq!(color("InfoText").as_deref(), Some("infotext"));
        assert_eq!(color("#234").as_deref(), Some("rgb(34, 51, 68)"));
        assert_eq!(color("#ff000080").as_deref(), Some("rgba(255, 0, 0, 0.5)"));
        assert_eq!(color("#12"), None);
        assert_eq!(color("#ggg"), None);
        assert!(NAMED_COLORS.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn legacy_alpha() {
        let alpha = |value| {
            let mut out = String::new();
            write_legacy_alpha(&mut out, value);
            out
        };
        assert_eq!(alpha(237), "0.93");
        assert_eq!(alpha(236), "0.925");
        assert_eq!(alpha(128), "0.5");
        assert_eq!(alpha(0), "0");
    }

    #[test]
    fn functions() {
        assert_eq!(
            color("rgba(2, 3, 4, 50%)").as_deref(),
            Some("rgba(2, 3, 4, 0.5)")
        );
        assert_eq!(color("rgb(0 0 0 / 1)").as_deref(), Some("rgb(0, 0, 0)"));
        assert_eq!(color("rgb(10%, 20, 30%)"), None);
        assert_eq!(color("rgb(1 2 3 / 0.5 0.5)"), None);
        assert_eq!(
            color("rgb(128 none none)").as_deref(),
            Some("color(srgb 0.501961 none none)")
        );
        assert_eq!(
            color("hsl(120 30% 50%)").as_deref(),
            Some("rgb(89, 166, 89)")
        );
        assert_eq!(
            color("hsl(120 none 50)").as_deref(),
            Some("hsl(120 none 50%)")
        );
        assert_eq!(
            color("hwb(320deg 30% 40%)").as_deref(),
            Some("rgb(153, 77, 128)")
        );
        assert_eq!(
            color("lch(10 20 1.28rad)").as_deref(),
            Some("lch(10 20 73.3386)")
        );
        assert_eq!(
            color("oklab(50% 50% -20%)").as_deref(),
            Some("oklab(0.5 0.2 -0.08)")
        );
        assert_eq!(
            color("color(xyz 1 2 3 / none)").as_deref(),
            Some("color(xyz-d65 1 2 3 / none)")
        );
        assert_eq!(color("color(srgb 0, 0, 0)"), None);
    }
}
