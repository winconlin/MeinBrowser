//! Grundwerte nach CSS Values and Units Level 4 (Editor's Draft, Stand
//! 23.10.2026) und ihr Serialisieren nach CSSOM („Serializing CSS Values“).
//! Farben stehen in `values/color.rs`.
//!
//! Werte werden aus den Komponentenwerten einer Deklaration gelesen
//! ([`Input`]); Leerraum zwischen ihnen ist bedeutungslos und wird
//! übersprungen. Zahlen behalten die Genauigkeit der Tokens (`f64`), werden
//! aber auf den Bereich von `f32` begrenzt, mit dem später gerechnet wird.
//! Values 4 überlässt Genauigkeit und Bereich der Implementierung
//! („implementation-defined“); was nicht darstellbar ist, wird zum nächsten
//! darstellbaren Wert.
//!
//! Es fehlen noch die mathematischen Funktionen wie `calc()` sowie `var()`,
//! `env()` und `attr()`. Werte mit ihnen sind bisher ungültig.

pub mod color;

use std::fmt::Write as _;

use mb_css::{ComponentValue, Iter, Token, Values};
use mb_url::Url;

/// Was das Parsen eines Werts außer dem Text braucht.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParseContext<'a> {
    /// Die Basis für relative URLs, etwa die URL des Stylesheets.
    pub base_url: Option<&'a Url>,
    /// Ob das Dokument im *quirks mode* ist. Dann gelten ⟨quirky-length⟩s
    /// (Anhang C).
    pub quirks_mode: bool,
}

/// Der größte Betrag einer Zahl, siehe die Moduldokumentation.
const MAX: f64 = f32::MAX as f64;

/// Begrenzt eine Zahl aus einem Token auf den unterstützten Bereich.
pub(crate) fn clamp(value: f64) -> f64 {
    value.clamp(-MAX, MAX)
}

/// Liest Komponentenwerte und überspringt dabei Leerraum. Scheitert eine
/// Methode, bleibt die Position unverändert.
#[derive(Clone, Debug)]
pub(crate) struct Input<'a> {
    values: Iter<'a>,
    context: ParseContext<'a>,
    /// Ob hier ⟨quirky-length⟩s gelten. Sie gelten nur in bestimmten
    /// Eigenschaften und nie innerhalb von Funktionen.
    quirky_lengths: bool,
}

impl<'a> Input<'a> {
    pub(crate) fn new(values: Values<'a>, context: ParseContext<'a>) -> Input<'a> {
        Input {
            values: values.iter(),
            context,
            quirky_lengths: false,
        }
    }

    pub(crate) fn context(&self) -> ParseContext<'a> {
        self.context
    }

    /// Erlaubt ⟨quirky-length⟩s, wenn das Dokument im *quirks mode* ist.
    pub(crate) fn allow_quirky_lengths(&mut self) {
        self.quirky_lengths = self.context.quirks_mode;
    }

    /// Der nächste Komponentenwert, ohne ihn zu verbrauchen.
    pub(crate) fn peek(&self) -> Option<ComponentValue<'a>> {
        self.clone().next()
    }

    pub(crate) fn next(&mut self) -> Option<ComponentValue<'a>> {
        loop {
            match self.values.next()? {
                ComponentValue::Token(Token::Whitespace) => {}
                value => return Some(value),
            }
        }
    }

    /// Ob nur noch Leerraum folgt.
    pub(crate) fn at_end(&self) -> bool {
        self.peek().is_none()
    }

    /// Verbraucht den nächsten Wert, wenn `test` für ihn etwas liefert.
    pub(crate) fn next_if<T>(
        &mut self,
        test: impl FnOnce(ComponentValue<'a>) -> Option<T>,
    ) -> Option<T> {
        let mut ahead = self.clone();
        let result = test(ahead.next()?)?;
        *self = ahead;
        Some(result)
    }

    /// Führt `parse` aus und setzt die Position zurück, wenn es scheitert.
    pub(crate) fn try_parse<T>(
        &mut self,
        parse: impl FnOnce(&mut Input<'a>) -> Option<T>,
    ) -> Option<T> {
        let saved = self.values.clone();
        let result = parse(self);
        if result.is_none() {
            self.values = saved;
        }
        result
    }

    /// Ein ⟨ident-token⟩.
    pub(crate) fn ident(&mut self) -> Option<&'a str> {
        self.next_if(|value| match value {
            ComponentValue::Token(Token::Ident(name)) => Some(name.as_str()),
            _ => None,
        })
    }

    /// Das Schlüsselwort `keyword`, ohne Unterschied zwischen ASCII-Groß-
    /// und -Kleinbuchstaben.
    pub(crate) fn keyword(&mut self, keyword: &str) -> bool {
        self.next_if(|value| match value {
            ComponentValue::Token(Token::Ident(name)) if name.eq_ignore_ascii_case(keyword) => {
                Some(())
            }
            _ => None,
        })
        .is_some()
    }

    pub(crate) fn comma(&mut self) -> bool {
        self.next_if(|value| match value {
            ComponentValue::Token(Token::Comma) => Some(()),
            _ => None,
        })
        .is_some()
    }

    pub(crate) fn delim(&mut self, delim: char) -> bool {
        self.next_if(|value| match value {
            ComponentValue::Token(Token::Delim(found)) if *found == delim => Some(()),
            _ => None,
        })
        .is_some()
    }

    /// Eine Funktion: ihr Name und eine Eingabe für ihre Argumente.
    pub(crate) fn function(&mut self) -> Option<(&'a str, Input<'a>)> {
        let context = self.context;
        self.next_if(|value| match value {
            ComponentValue::Function { name, value } => Some((name, Input::new(value, context))),
            _ => None,
        })
    }

    /// Ein ⟨number⟩.
    pub(crate) fn number(&mut self) -> Option<f64> {
        self.next_if(|value| match value {
            ComponentValue::Token(Token::Number { value, .. }) => Some(clamp(*value)),
            _ => None,
        })
    }

    /// Ein ⟨percentage⟩, als Zahl vor dem `%`.
    pub(crate) fn percentage(&mut self) -> Option<f64> {
        self.next_if(|value| match value {
            ComponentValue::Token(Token::Percentage { value, .. }) => Some(clamp(*value)),
            _ => None,
        })
    }

    /// Ein ⟨string-token⟩.
    pub(crate) fn string(&mut self) -> Option<&'a str> {
        self.next_if(|value| match value {
            ComponentValue::Token(Token::String(text)) => Some(text.as_str()),
            _ => None,
        })
    }

    /// Prüft, dass die Eingabe ganz verbraucht ist.
    pub(crate) fn finish<T>(&self, value: T) -> Option<T> {
        self.at_end().then_some(value)
    }
}

/// Ein Wert, der sich aus Komponentenwerten lesen lässt.
pub(crate) trait Parse: Sized {
    fn parse(input: &mut Input<'_>) -> Option<Self>;
}

/// Ein Wert, der sich nach CSSOM serialisieren lässt.
pub trait ToCss {
    fn to_css(&self, out: &mut String);

    fn to_css_string(&self) -> String {
        let mut out = String::new();
        self.to_css(&mut out);
        out
    }
}

/// Legt eine Aufzählung von Schlüsselwörtern an, mit [`Parse`], [`ToCss`]
/// und dem Namen jedes Werts.
macro_rules! keywords {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($(#[$variant_meta:meta])* $variant:ident = $text:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)*
        }

        impl $name {
            /// Das Schlüsselwort in Kleinbuchstaben.
            pub fn name(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)*
                }
            }

            /// Das Schlüsselwort zu einem Bezeichner, ohne Unterschied
            /// zwischen ASCII-Groß- und -Kleinbuchstaben.
            pub(crate) fn from_ident(ident: &str) -> Option<$name> {
                $(if ident.eq_ignore_ascii_case($text) {
                    return Some($name::$variant);
                })*
                None
            }
        }

        impl $crate::values::Parse for $name {
            fn parse(input: &mut $crate::values::Input<'_>) -> Option<$name> {
                input.next_if(|value| match value {
                    mb_css::ComponentValue::Token(mb_css::Token::Ident(ident)) => {
                        $name::from_ident(ident)
                    }
                    _ => None,
                })
            }
        }

        impl $crate::values::ToCss for $name {
            fn to_css(&self, out: &mut String) {
                out.push_str(self.name());
            }
        }
    };
}

pub(crate) use keywords;

/// *serialize a number* (CSSOM, ⟨number⟩): kürzeste Dezimalform ohne
/// Exponent, auf höchstens sechs Nachkommastellen gerundet, Hälften
/// aufwärts (Values 4, „rounded towards +∞“). −0 wird zu „0“.
pub(crate) fn write_number(out: &mut String, value: f64) {
    write_rounded(out, value, 6);
}

/// Rundet auf `decimals` Nachkommastellen, Hälften aufwärts, und schreibt
/// die kürzeste Dezimalform.
fn write_rounded(out: &mut String, value: f64, decimals: i32) {
    let scale = 10f64.powi(decimals);
    let scaled = value * scale;
    // Ab 2⁵³ hat ein `f64` keine Nachkommastellen mehr; dann bleibt der
    // Wert, wie er ist.
    let rounded = if scaled.abs() < 9.0e15 {
        (scaled + 0.5).floor() / scale
    } else {
        value
    };
    // Auch −0 gibt „0“.
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    // `Display` für `f64` schreibt die kürzeste Form, die den Wert
    // wiedergibt, und nie einen Exponenten.
    let _ = write!(out, "{rounded}");
}

/// Schreibt eine Zahl mit sechs signifikanten Stellen, Hälften aufwärts.
/// So serialisieren Browser die Komponenten von Farben; CSS Color 4 legt
/// deren Genauigkeit nicht fest, verlangt aber mindestens drei
/// Nachkommastellen für ⟨lab()⟩ und fünf für ⟨oklab()⟩ in deren üblichen
/// Bereichen, was sechs signifikante Stellen erfüllen.
pub(crate) fn write_significant(out: &mut String, value: f64) {
    if value == 0.0 || !value.is_finite() {
        write_number(out, value);
        return;
    }
    let magnitude = value.abs().log10().floor();
    // Höchstens 330 Stellen, die Umwandlung kann also nicht scheitern.
    let digits = 5 - magnitude.clamp(-330.0, 330.0) as i32;
    if digits >= 0 {
        write_rounded(out, value, digits);
    } else {
        let scale = 10f64.powi(-digits);
        write_number(out, ((value / scale) + 0.5).floor() * scale);
    }
}

/// *serialize an identifier* (CSSOM)
pub(crate) fn write_identifier(out: &mut String, identifier: &str) {
    let mut chars = identifier.chars().peekable();
    let mut first = true;
    let mut after_leading_hyphen = false;
    while let Some(c) = chars.next() {
        match c {
            '\0' => out.push(char::REPLACEMENT_CHARACTER),
            '\u{1}'..='\u{1F}' | '\u{7F}' => write_code_point_escape(out, c),
            '0'..='9' if first || after_leading_hyphen => write_code_point_escape(out, c),
            '-' if first && chars.peek().is_none() => out.push_str("\\-"),
            '-' | '_' | '0'..='9' | 'A'..='Z' | 'a'..='z' => out.push(c),
            _ if !c.is_ascii() => out.push(c),
            _ => {
                out.push('\\');
                out.push(c);
            }
        }
        after_leading_hyphen = first && c == '-';
        first = false;
    }
}

/// *escape a character as code point*
fn write_code_point_escape(out: &mut String, c: char) {
    let _ = write!(out, "\\{:x} ", u32::from(c));
}

/// *serialize a string* (CSSOM)
pub(crate) fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '\0' => out.push(char::REPLACEMENT_CHARACTER),
            '\u{1}'..='\u{1F}' | '\u{7F}' => write_code_point_escape(out, c),
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push('"');
}

/// Ein ⟨length⟩ mit Einheit, wie angegeben.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Length {
    pub value: f64,
    pub unit: LengthUnit,
}

impl Length {
    pub const fn px(value: f64) -> Length {
        Length {
            value,
            unit: LengthUnit::Absolute(AbsoluteUnit::Px),
        }
    }
}

/// Die Einheit eines ⟨length⟩ (Abschnitt 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LengthUnit {
    Absolute(AbsoluteUnit),
    /// Schriftbezogen; `root` für die Einheiten mit `r`, die sich auf das
    /// Wurzelelement beziehen.
    Font {
        metric: FontMetric,
        root: bool,
    },
    /// Bezogen auf den Viewport.
    Viewport {
        size: ViewportSize,
        axis: ViewportAxis,
    },
}

/// Die absoluten Längeneinheiten.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AbsoluteUnit {
    Px,
    Cm,
    Mm,
    /// Viertelmillimeter
    Q,
    In,
    Pt,
    Pc,
}

impl AbsoluteUnit {
    /// Wie viele `px` eine Einheit sind (1in = 2,54cm = 96px).
    pub fn to_px(self) -> f64 {
        match self {
            AbsoluteUnit::Px => 1.0,
            AbsoluteUnit::Cm => 96.0 / 2.54,
            AbsoluteUnit::Mm => 96.0 / 25.4,
            AbsoluteUnit::Q => 96.0 / 101.6,
            AbsoluteUnit::In => 96.0,
            AbsoluteUnit::Pt => 96.0 / 72.0,
            AbsoluteUnit::Pc => 16.0,
        }
    }
}

/// Das Maß einer Schrift, auf das sich eine Einheit bezieht.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontMetric {
    /// `em`: die Schriftgröße
    Em,
    /// `ex`: die x-Höhe
    Ex,
    /// `cap`: die Versalhöhe
    Cap,
    /// `ch`: die Vorschubbreite der Ziffer „0“
    Ch,
    /// `ic`: die Vorschubbreite von „水“
    Ic,
    /// `lh`: die Zeilenhöhe
    Lh,
}

/// Welche Größe des Viewports eine Einheit meint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewportSize {
    /// `v…`, die *UA-default viewport-percentage units*. Hier gleich den
    /// großen.
    Default,
    /// `sv…`
    Small,
    /// `lv…`
    Large,
    /// `dv…`
    Dynamic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewportAxis {
    Width,
    Height,
    Inline,
    Block,
    Min,
    Max,
}

/// Alle Längeneinheiten mit ihrer Schreibweise in der Spezifikation.
const LENGTH_UNITS: [(&str, LengthUnit); 43] = {
    use FontMetric::{Cap, Ch, Em, Ex, Ic, Lh};
    use LengthUnit::{Absolute, Font, Viewport};
    use ViewportAxis::{Block, Height, Inline, Max, Min, Width};
    use ViewportSize::{Default, Dynamic, Large, Small};
    const fn v(size: ViewportSize, axis: ViewportAxis) -> LengthUnit {
        Viewport { size, axis }
    }
    [
        ("px", Absolute(AbsoluteUnit::Px)),
        ("cm", Absolute(AbsoluteUnit::Cm)),
        ("mm", Absolute(AbsoluteUnit::Mm)),
        ("Q", Absolute(AbsoluteUnit::Q)),
        ("in", Absolute(AbsoluteUnit::In)),
        ("pt", Absolute(AbsoluteUnit::Pt)),
        ("pc", Absolute(AbsoluteUnit::Pc)),
        (
            "em",
            Font {
                metric: Em,
                root: false,
            },
        ),
        (
            "rem",
            Font {
                metric: Em,
                root: true,
            },
        ),
        (
            "ex",
            Font {
                metric: Ex,
                root: false,
            },
        ),
        (
            "rex",
            Font {
                metric: Ex,
                root: true,
            },
        ),
        (
            "cap",
            Font {
                metric: Cap,
                root: false,
            },
        ),
        (
            "rcap",
            Font {
                metric: Cap,
                root: true,
            },
        ),
        (
            "ch",
            Font {
                metric: Ch,
                root: false,
            },
        ),
        (
            "rch",
            Font {
                metric: Ch,
                root: true,
            },
        ),
        (
            "ic",
            Font {
                metric: Ic,
                root: false,
            },
        ),
        (
            "ric",
            Font {
                metric: Ic,
                root: true,
            },
        ),
        (
            "lh",
            Font {
                metric: Lh,
                root: false,
            },
        ),
        (
            "rlh",
            Font {
                metric: Lh,
                root: true,
            },
        ),
        ("vw", v(Default, Width)),
        ("vh", v(Default, Height)),
        ("vi", v(Default, Inline)),
        ("vb", v(Default, Block)),
        ("vmin", v(Default, Min)),
        ("vmax", v(Default, Max)),
        ("svw", v(Small, Width)),
        ("svh", v(Small, Height)),
        ("svi", v(Small, Inline)),
        ("svb", v(Small, Block)),
        ("svmin", v(Small, Min)),
        ("svmax", v(Small, Max)),
        ("lvw", v(Large, Width)),
        ("lvh", v(Large, Height)),
        ("lvi", v(Large, Inline)),
        ("lvb", v(Large, Block)),
        ("lvmin", v(Large, Min)),
        ("lvmax", v(Large, Max)),
        ("dvw", v(Dynamic, Width)),
        ("dvh", v(Dynamic, Height)),
        ("dvi", v(Dynamic, Inline)),
        ("dvb", v(Dynamic, Block)),
        ("dvmin", v(Dynamic, Min)),
        ("dvmax", v(Dynamic, Max)),
    ]
};

impl LengthUnit {
    /// Die Einheit zu einem Namen; Einheiten sind *ASCII case-insensitive*.
    pub fn from_name(name: &str) -> Option<LengthUnit> {
        LENGTH_UNITS
            .iter()
            .find(|(unit, _)| unit.eq_ignore_ascii_case(name))
            .map(|(_, unit)| *unit)
    }

    /// Der Name, wie ihn die Spezifikation schreibt („canonical form“ in
    /// CSSOM): klein, nur `Q` groß.
    pub fn name(self) -> &'static str {
        LENGTH_UNITS
            .iter()
            .find(|(_, unit)| *unit == self)
            .map_or("px", |(name, _)| name)
    }
}

impl Parse for Length {
    fn parse(input: &mut Input<'_>) -> Option<Length> {
        let quirky = input.quirky_lengths;
        input.next_if(|value| match value {
            ComponentValue::Token(Token::Dimension { value, unit, .. }) => Some(Length {
                value: clamp(*value),
                unit: LengthUnit::from_name(unit)?,
            }),
            // Eine Länge null darf ohne Einheit stehen. Ein ⟨quirky-length⟩
            // ist jede Zahl, gelesen als `px`.
            ComponentValue::Token(Token::Number { value, .. }) if *value == 0.0 || quirky => {
                Some(Length::px(clamp(*value)))
            }
            _ => None,
        })
    }
}

impl ToCss for Length {
    fn to_css(&self, out: &mut String) {
        write_number(out, self.value);
        out.push_str(self.unit.name());
    }
}

/// ⟨length-percentage⟩
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthPercentage {
    Length(Length),
    /// Die Zahl vor dem `%`.
    Percentage(f64),
}

impl LengthPercentage {
    pub const ZERO: LengthPercentage = LengthPercentage::Length(Length::px(0.0));

    fn value(self) -> f64 {
        match self {
            LengthPercentage::Length(length) => length.value,
            LengthPercentage::Percentage(value) => value,
        }
    }

    /// Ob der Wert in [0,∞] liegt.
    pub(crate) fn is_non_negative(self) -> bool {
        self.value() >= 0.0
    }
}

impl Parse for LengthPercentage {
    fn parse(input: &mut Input<'_>) -> Option<LengthPercentage> {
        if let Some(value) = input.percentage() {
            return Some(LengthPercentage::Percentage(value));
        }
        Length::parse(input).map(LengthPercentage::Length)
    }
}

impl ToCss for LengthPercentage {
    fn to_css(&self, out: &mut String) {
        match self {
            LengthPercentage::Length(length) => length.to_css(out),
            LengthPercentage::Percentage(value) => {
                write_number(out, *value);
                out.push('%');
            }
        }
    }
}

/// ⟨length-percentage [0,∞]⟩: Negative Werte sind ungültig.
pub(crate) fn non_negative_length_percentage(input: &mut Input<'_>) -> Option<LengthPercentage> {
    input.try_parse(|input| LengthPercentage::parse(input).filter(|value| value.is_non_negative()))
}

/// ⟨length [0,∞]⟩
pub(crate) fn non_negative_length(input: &mut Input<'_>) -> Option<Length> {
    input.try_parse(|input| Length::parse(input).filter(|length| length.value >= 0.0))
}

/// Die Einheit eines ⟨angle⟩.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AngleUnit {
    Deg,
    Grad,
    Rad,
    Turn,
}

/// Ein ⟨angle⟩ mit Einheit, wie angegeben.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Angle {
    pub value: f64,
    pub unit: AngleUnit,
}

impl Angle {
    pub fn degrees(self) -> f64 {
        match self.unit {
            AngleUnit::Deg => self.value,
            AngleUnit::Grad => self.value * 0.9,
            AngleUnit::Rad => self.value.to_degrees(),
            AngleUnit::Turn => self.value * 360.0,
        }
    }
}

impl Parse for Angle {
    fn parse(input: &mut Input<'_>) -> Option<Angle> {
        input.next_if(|value| match value {
            ComponentValue::Token(Token::Dimension { value, unit, .. }) => {
                let unit = [
                    ("deg", AngleUnit::Deg),
                    ("grad", AngleUnit::Grad),
                    ("rad", AngleUnit::Rad),
                    ("turn", AngleUnit::Turn),
                ]
                .into_iter()
                .find(|(name, _)| unit.eq_ignore_ascii_case(name))?
                .1;
                Some(Angle {
                    value: clamp(*value),
                    unit,
                })
            }
            _ => None,
        })
    }
}

impl ToCss for Angle {
    fn to_css(&self, out: &mut String) {
        write_number(out, self.value);
        out.push_str(match self.unit {
            AngleUnit::Deg => "deg",
            AngleUnit::Grad => "grad",
            AngleUnit::Rad => "rad",
            AngleUnit::Turn => "turn",
        });
    }
}

/// Ein ⟨url⟩, wie angegeben, und zur Basis aufgelöst. Ein leerer Text
/// verweist auf nichts (*invalid*); er wird nicht aufgelöst.
#[derive(Clone, Debug, PartialEq)]
pub struct SpecifiedUrl {
    pub text: String,
    pub resolved: Option<Url>,
}

impl Parse for SpecifiedUrl {
    /// `url( <string> <url-modifier>* ) | <url-token>`. Values 4 definiert
    /// keine ⟨url-modifier⟩ und `src()` nur als Alternative mit denselben
    /// Regeln; Modifikatoren machen den Wert hier ungültig.
    fn parse(input: &mut Input<'_>) -> Option<SpecifiedUrl> {
        let base = input.context.base_url;
        let text = input.next_if(|value| match value {
            ComponentValue::Token(Token::Url(text)) => Some(text.as_str()),
            ComponentValue::Function { name, value }
                if name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("src") =>
            {
                let mut arguments = Input::new(value, ParseContext::default());
                let text = arguments.string()?;
                arguments.finish(text)
            }
            _ => None,
        })?;
        let resolved = if text.is_empty() {
            None
        } else {
            Url::parse(text, base).ok()
        };
        Some(SpecifiedUrl {
            text: text.to_owned(),
            resolved,
        })
    }
}

impl ToCss for SpecifiedUrl {
    /// *serialize a URL*: der angegebene Text als Zeichenkette in `url()`.
    fn to_css(&self, out: &mut String) {
        out.push_str("url(");
        write_string(out, &self.text);
        out.push(')');
    }
}

/// Ob ein Bezeichner ein CSS-weites Schlüsselwort ist (CSS Cascade 5).
pub(crate) fn is_css_wide_keyword(ident: &str) -> bool {
    [
        "initial",
        "inherit",
        "unset",
        "revert",
        "revert-layer",
        "revert-rule",
    ]
    .iter()
    .any(|keyword| ident.eq_ignore_ascii_case(keyword))
}

/// Ein ⟨custom-ident⟩: kein CSS-weites Schlüsselwort und nicht `default`,
/// außerdem keins aus `excluded`. Groß- und Kleinschreibung bleiben.
pub(crate) fn custom_ident<'a>(input: &mut Input<'a>, excluded: &[&str]) -> Option<&'a str> {
    input.try_parse(|input| {
        let ident = input.ident()?;
        let reserved = is_css_wide_keyword(ident)
            || ident.eq_ignore_ascii_case("default")
            || excluded
                .iter()
                .any(|keyword| ident.eq_ignore_ascii_case(keyword));
        (!reserved).then_some(ident)
    })
}

#[cfg(test)]
mod tests {
    use mb_css::{Source, parse_component_value_list};

    use super::*;

    fn parse<T>(text: &str, parse: impl FnOnce(&mut Input<'_>) -> Option<T>) -> Option<T> {
        let values = parse_component_value_list(&Source::new(text));
        let mut input = Input::new(values.values(), ParseContext::default());
        let value = parse(&mut input)?;
        input.finish(value)
    }

    fn number(value: f64) -> String {
        let mut out = String::new();
        write_number(&mut out, value);
        out
    }

    #[test]
    fn numbers() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(-0.0000001), "0");
        assert_eq!(number(1.5), "1.5");
        assert_eq!(number(0.1234565), "0.123457");
        assert_eq!(number(-2.0000005), "-2");
        assert_eq!(number(1e21), "1000000000000000000000");
        assert_eq!(number(1.0 / 3.0), "0.333333");
    }

    #[test]
    fn significant_digits() {
        let significant = |value| {
            let mut out = String::new();
            write_significant(&mut out, value);
            out
        };
        assert_eq!(significant(1.28f64.to_degrees()), "73.3386");
        assert_eq!(significant(128.0 / 255.0), "0.501961");
        assert_eq!(significant(-160.0), "-160");
        assert_eq!(significant(1234567.0), "1234570");
        assert_eq!(significant(0.0), "0");
    }

    #[test]
    fn identifiers_and_strings() {
        let identifier = |text: &str| {
            let mut out = String::new();
            write_identifier(&mut out, text);
            out
        };
        assert_eq!(identifier("abc"), "abc");
        assert_eq!(identifier("1a"), "\\31 a");
        assert_eq!(identifier("-1a"), "-\\31 a");
        assert_eq!(identifier("-"), "\\-");
        assert_eq!(identifier("--x"), "--x");
        assert_eq!(identifier("a b"), "a\\ b");
        assert_eq!(identifier("a\u{1}\0é"), "a\\1 \u{FFFD}é");
        let mut out = String::new();
        write_string(&mut out, "a\"b\\c'\n");
        assert_eq!(out, "\"a\\\"b\\\\c'\\a \"");
    }

    #[test]
    fn lengths() {
        let length = |text| parse(text, Length::parse).map(|length| length.to_css_string());
        assert_eq!(length("10PX").as_deref(), Some("10px"));
        assert_eq!(length("1q").as_deref(), Some("1Q"));
        assert_eq!(length("0").as_deref(), Some("0px"));
        assert_eq!(length("2.5Dvmax").as_deref(), Some("2.5dvmax"));
        assert_eq!(length("5"), None);
        assert_eq!(length("5%"), None);
        assert_eq!(length("1e40px").as_deref().map(str::len), Some(41));
        let quirky = {
            let values = parse_component_value_list(&Source::new("5"));
            let context = ParseContext {
                quirks_mode: true,
                ..ParseContext::default()
            };
            let mut input = Input::new(values.values(), context);
            input.allow_quirky_lengths();
            Length::parse(&mut input).map(|length| length.to_css_string())
        };
        assert_eq!(quirky.as_deref(), Some("5px"));
    }

    #[test]
    fn urls() {
        let base = Url::parse("https://example.com/a/b", None).expect("Basis");
        let parse_url = |text: &str| {
            let values = parse_component_value_list(&Source::new(text));
            let context = ParseContext {
                base_url: Some(&base),
                quirks_mode: false,
            };
            let mut input = Input::new(values.values(), context);
            let url = SpecifiedUrl::parse(&mut input)?;
            input.finish(url)
        };
        let url = parse_url("url(c.png)").expect("url");
        assert_eq!(url.to_css_string(), "url(\"c.png\")");
        assert_eq!(
            url.resolved.map(|url| url.href()).as_deref(),
            Some("https://example.com/a/c.png")
        );
        let quoted = parse_url("URL( 'x\"y' )").expect("url()");
        assert_eq!(quoted.to_css_string(), "url(\"x\\\"y\")");
        assert_eq!(parse_url("url('')").and_then(|url| url.resolved), None);
        assert!(parse_url("url('a' b)").is_none());
    }
}
