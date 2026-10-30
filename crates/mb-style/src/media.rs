//! Media Queries nach Media Queries 5 (Editor's Draft, Stand 23.10.2026):
//! Listen von Media Queries lesen und für ein Gerät auswerten.
//!
//! Das Gerät ist ein Bildschirm (Medientyp `screen`). Breite, Höhe und
//! Pixeldichte kommen aus [`Device`], die Maße der Schriften für
//! schriftbezogene Einheiten aus einer [`FontMetricsSource`]; diese Einheiten
//! beziehen sich auf die Anfangswerte. Alle anderen Merkmale haben feste
//! Werte, festgelegt von diesem Browser, bis es die Angaben der Plattformen
//! und die Einstellungen der Nutzer gibt (siehe [`FEATURES`]): ein Bildschirm
//! mit 8 Bit je Farbkanal im sRGB-Farbraum ohne HDR, eine Maus
//! (`pointer: fine`, `hover: hover`), ein helles Farbschema ohne weitere
//! Vorlieben, Anzeige im Browserfenster und `scripting: none`, solange Seiten
//! keine Skripte ausführen. Die Merkmale `device-width`, `device-height` und
//! `device-aspect-ratio` haben die Werte des Viewports, denn die Größe des
//! Bildschirms ist nicht bekannt.
//!
//! Lesarten nach den WPT, wo der Entwurf nichts oder anderes sagt:
//! - Nur Leerraum ergibt eine leere Liste, die immer zutrifft, und ein Komma
//!   am Ende einen weiteren Eintrag `not all` (`match-media-parsing.html`).
//!   *Parse a comma-separated list of component values* aus CSS Syntax 3
//!   ergäbe für Leerraum allein einen ungültigen Eintrag und für das Komma am
//!   Ende keinen.
//! - Das Verhältnis `0/0` ist größer als jedes andere, wie `1/0`
//!   (`aspect-ratio-002.html`, `aspect-ratio-004.html`). Values 4 sagt nur,
//!   Verhältnisse würden verglichen, indem man teilt.
//!
//! Es fehlen mathematische Funktionen wie `calc()` in Werten,
//! `@custom-media` und das Serialisieren für CSSOM.

use std::cmp::Ordering;

use mb_base::{Budget, Exhausted};
use mb_css::{BlockKind, ComponentValue, Source, Token, Values, parse_component_value_list};

use crate::computed::{Device, FontMetricsSource, initial_px};
use crate::values::{Length, LengthUnit, clamp};

/// Wie tief Bedingungen verschachtelt sein dürfen; tiefere werden zu
/// ⟨general-enclosed⟩. Lesen und Auswerten steigen rekursiv ab, die Grenze
/// schützt vor einem Stapelüberlauf.
const MAX_DEPTH: usize = 100;

/// Eine ⟨media-query-list⟩. Eine leere trifft immer zu.
#[derive(Clone, Debug, Default)]
pub struct MediaQueryList {
    queries: Vec<MediaQuery>,
}

impl MediaQueryList {
    /// Liest eine Liste aus Text, etwa aus dem Attribut `media`.
    pub fn parse(text: &str) -> MediaQueryList {
        let values = parse_component_value_list(&Source::new(text));
        let values: Vec<ComponentValue<'_>> = values.iter().collect();
        MediaQueryList::from_values(&values)
    }

    /// Liest eine Liste aus Komponentenwerten, etwa aus dem Prelude von
    /// `@media`. Jede Media Query, die nicht zur Grammatik passt, wird zu
    /// `not all`; die anderen bleiben.
    pub(crate) fn from_values(values: &[ComponentValue<'_>]) -> MediaQueryList {
        if values.iter().all(is_whitespace) {
            return MediaQueryList::default();
        }
        let queries = values
            .split(|value| matches!(value, ComponentValue::Token(Token::Comma)))
            .map(|query| media_query(query).unwrap_or(MediaQuery::NotAll))
            .collect();
        MediaQueryList { queries }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.queries.is_empty()
    }

    /// Ob die Liste für ein Gerät zutrifft: Sie ist leer, oder eine ihrer
    /// Media Queries trifft zu.
    pub fn matches(&self, device: &Device, fonts: &dyn FontMetricsSource) -> bool {
        self.matches_within(device, fonts, &Budget::unlimited())
            .expect("unbegrenztes Budget")
    }

    /// Wie [`MediaQueryList::matches`], aber jede Media Query, Bedingung und
    /// Merkmalsauswertung bezahlt aus `budget`. Ein Abbruch ist ein Fehler,
    /// kein nicht zutreffendes Ergebnis; auch eine leere Liste prüft ihn.
    pub fn matches_within(
        &self,
        device: &Device,
        fonts: &dyn FontMetricsSource,
        budget: &Budget,
    ) -> Result<bool, Exhausted> {
        let environment = Environment {
            device,
            fonts,
            budget,
        };
        environment.step()?;
        if self.queries.is_empty() {
            return Ok(true);
        }
        for query in &self.queries {
            if query.matches(&environment)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Wofür ausgewertet wird.
struct Environment<'a> {
    device: &'a Device,
    fonts: &'a dyn FontMetricsSource,
    budget: &'a Budget,
}

impl Environment<'_> {
    fn step(&self) -> Result<(), Exhausted> {
        if self.budget.spend(1) {
            Ok(())
        } else {
            self.budget.check()
        }
    }

    /// Auch ein gerade erst gesetztes Cancel bemerken, ohne einen weiteren
    /// Arbeitsschritt zu berechnen.
    fn check(&self) -> Result<(), Exhausted> {
        if self.budget.spend(0) {
            Ok(())
        } else {
            self.budget.check()
        }
    }
}

/// Ein Wahrheitswert der dreiwertigen Logik nach Kleene, von falsch nach
/// wahr geordnet: „und“ ist das Minimum, „oder“ das Maximum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kleene {
    False,
    Unknown,
    True,
}

impl Kleene {
    fn from_bool(value: bool) -> Kleene {
        if value { Kleene::True } else { Kleene::False }
    }

    fn not(self) -> Kleene {
        match self {
            Kleene::False => Kleene::True,
            Kleene::Unknown => Kleene::Unknown,
            Kleene::True => Kleene::False,
        }
    }
}

/// Eine ⟨media-query⟩.
#[derive(Clone, Debug)]
enum MediaQuery {
    /// Eine Media Query, die nicht zur Grammatik passt, wird zu `not all`.
    NotAll,
    /// `[ not | only ]? <media-type> [ and <media-condition-without-or> ]?`
    Typed {
        not: bool,
        media_type: MediaType,
        condition: Option<Condition>,
    },
    /// ⟨media-condition⟩
    Condition(Condition),
}

impl MediaQuery {
    fn matches(&self, environment: &Environment<'_>) -> Result<bool, Exhausted> {
        environment.step()?;
        let (not, value) = match self {
            MediaQuery::NotAll => return Ok(false),
            MediaQuery::Typed {
                not,
                media_type,
                condition,
            } => {
                let condition = match condition {
                    Some(condition) => condition.evaluate(environment)?,
                    None => Kleene::True,
                };
                (*not, Kleene::from_bool(media_type.matches()).min(condition))
            }
            MediaQuery::Condition(condition) => (false, condition.evaluate(environment)?),
        };
        // Eine Media Query, deren Wert „unknown“ ist, wird zu `not all`;
        // `not` davor ändert daran nichts.
        Ok(match value {
            Kleene::Unknown => false,
            Kleene::True => !not,
            Kleene::False => not,
        })
    }
}

/// Ein ⟨media-type⟩.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaType {
    All,
    Screen,
    Print,
    /// Ein unbekannter Medientyp oder einer der veralteten wie `tv` und
    /// `speech`; sie treffen nie zu.
    Other,
}

impl MediaType {
    /// `None` für die Schlüsselwörter, die kein Medientyp sein dürfen.
    fn from_ident(ident: &str) -> Option<MediaType> {
        let is = |keyword: &str| ident.eq_ignore_ascii_case(keyword);
        if ["only", "not", "and", "or", "layer"]
            .iter()
            .any(|keyword| is(keyword))
        {
            return None;
        }
        Some(if is("all") {
            MediaType::All
        } else if is("screen") {
            MediaType::Screen
        } else if is("print") {
            MediaType::Print
        } else {
            MediaType::Other
        })
    }

    fn matches(self) -> bool {
        matches!(self, MediaType::All | MediaType::Screen)
    }
}

/// Eine ⟨media-condition⟩ oder ein Teil davon.
#[derive(Clone, Debug)]
enum Condition {
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Vec<Condition>),
    Feature(Feature),
    /// ⟨general-enclosed⟩: immer „unknown“.
    GeneralEnclosed,
}

impl Condition {
    fn evaluate(&self, environment: &Environment<'_>) -> Result<Kleene, Exhausted> {
        environment.step()?;
        Ok(match self {
            Condition::Not(condition) => condition.evaluate(environment)?.not(),
            Condition::And(conditions) => {
                let mut result = Kleene::True;
                for condition in conditions {
                    result = result.min(condition.evaluate(environment)?);
                }
                result
            }
            Condition::Or(conditions) => {
                let mut result = Kleene::False;
                for condition in conditions {
                    result = result.max(condition.evaluate(environment)?);
                }
                result
            }
            Condition::Feature(feature) => feature.evaluate(environment)?,
            Condition::GeneralEnclosed => Kleene::Unknown,
        })
    }
}

/// Ein ⟨media-feature⟩, mit seiner Definition aus [`FEATURES`].
#[derive(Clone, Debug)]
enum Feature {
    /// Ein unbekannter Name oder ein Wert, der nicht zum Merkmal passt.
    Unknown,
    /// Nur der Name (*boolean context*).
    Boolean(&'static Definition),
    /// Ein Bereichsmerkmal: Der Wert des Geräts muss alle Vergleiche
    /// erfüllen, `name: wert` ist `=`, `min-` ist `>=`, `max-` ist `<=`.
    Range {
        definition: &'static Definition,
        comparisons: Vec<(Comparison, Specified)>,
    },
    Keyword {
        definition: &'static Definition,
        keyword: &'static str,
    },
    /// Ein ⟨mq-boolean⟩.
    MqBoolean {
        definition: &'static Definition,
        value: bool,
    },
}

impl Feature {
    fn evaluate(&self, environment: &Environment<'_>) -> Result<Kleene, Exhausted> {
        environment.step()?;
        Ok(match self {
            Feature::Unknown => Kleene::Unknown,
            Feature::Boolean(definition) => Kleene::from_bool(match definition.kind {
                // Wahr außer bei null.
                Kind::Range { device, .. } => device(environment.device) != 0.0,
                Kind::Keywords { current, falsy, .. } => {
                    let current = current(environment.device);
                    current != "none" && !falsy.contains(&current)
                }
                Kind::MqBoolean(current) => current,
            }),
            Feature::Range {
                definition,
                comparisons,
            } => {
                let Kind::Range { device, .. } = definition.kind else {
                    return Ok(Kleene::Unknown);
                };
                let device = device(environment.device);
                let mut result = true;
                for (comparison, specified) in comparisons {
                    environment.step()?;
                    // Fehlt ein Maß der Schrift, ist der Wert unbekannt.
                    let Some(value) = specified.resolve(environment)? else {
                        return Ok(Kleene::Unknown);
                    };
                    result &=
                        comparison.holds(positive_zero(device).total_cmp(&positive_zero(value)));
                }
                Kleene::from_bool(result)
            }
            Feature::Keyword {
                definition,
                keyword,
            } => match definition.kind {
                Kind::Keywords { current, .. } => {
                    Kleene::from_bool(current(environment.device) == *keyword)
                }
                _ => Kleene::Unknown,
            },
            Feature::MqBoolean { definition, value } => match definition.kind {
                Kind::MqBoolean(current) => Kleene::from_bool(current == *value),
                _ => Kleene::Unknown,
            },
        })
    }
}

/// `-0` als `0`, damit beide beim Vergleich gleich sind.
fn positive_zero(value: f64) -> f64 {
    value + 0.0
}

/// Ein Vergleich in der Richtung „Wert des Geräts ⟨vergleich⟩ angegebener
/// Wert“.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comparison {
    Less,
    LessEqual,
    Equal,
    GreaterEqual,
    Greater,
}

impl Comparison {
    /// Derselbe Vergleich mit vertauschten Seiten.
    fn flip(self) -> Comparison {
        match self {
            Comparison::Less => Comparison::Greater,
            Comparison::LessEqual => Comparison::GreaterEqual,
            Comparison::Equal => Comparison::Equal,
            Comparison::GreaterEqual => Comparison::LessEqual,
            Comparison::Greater => Comparison::Less,
        }
    }

    /// Ob zwei Vergleiche ⟨mf-lt⟩ oder beide ⟨mf-gt⟩ sind.
    fn same_direction(self, other: Comparison) -> bool {
        let less = |comparison| matches!(comparison, Comparison::Less | Comparison::LessEqual);
        let greater =
            |comparison| matches!(comparison, Comparison::Greater | Comparison::GreaterEqual);
        (less(self) && less(other)) || (greater(self) && greater(other))
    }

    fn holds(self, ordering: Ordering) -> bool {
        match self {
            Comparison::Less => ordering == Ordering::Less,
            Comparison::LessEqual => ordering != Ordering::Greater,
            Comparison::Equal => ordering == Ordering::Equal,
            Comparison::GreaterEqual => ordering != Ordering::Less,
            Comparison::Greater => ordering == Ordering::Greater,
        }
    }
}

/// Der angegebene Wert eines Bereichsmerkmals.
#[derive(Clone, Copy, Debug)]
enum Specified {
    /// Eine Länge, erst beim Auswerten in `px`.
    Length(Length),
    /// Eine ganze Zahl, ein Verhältnis als Quotient oder eine Auflösung in
    /// `dppx`.
    Number(f64),
}

impl Specified {
    fn resolve(self, environment: &Environment<'_>) -> Result<Option<f64>, Exhausted> {
        environment.check()?;
        let value = match self {
            Specified::Length(length) => {
                initial_px(length, environment.device, environment.fonts).ok()
            }
            Specified::Number(number) => Some(number),
        };
        // Schriftmaße kommen über einen Aufruf des Nutzers. Ein darin
        // gesetzter Abbruch darf nicht als unbekannter Wert verschwinden.
        environment.check()?;
        Ok(value)
    }
}

/// Die Definition eines Merkmals: der Name und wie es Werte annimmt.
#[derive(Debug)]
struct Definition {
    name: &'static str,
    kind: Kind,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    /// *range*: der Wert des Geräts als Zahl, Längen in `px`, Auflösungen
    /// in `dppx`, Verhältnisse als Quotient.
    Range {
        values: RangeKind,
        device: fn(&Device) -> f64,
    },
    /// *discrete* mit Schlüsselwörtern: die erlaubten, der Wert des Geräts
    /// und welche außer `none` im *boolean context* falsch sind.
    Keywords {
        values: &'static [&'static str],
        current: fn(&Device) -> &'static str,
        falsy: &'static [&'static str],
    },
    /// *discrete* mit einem ⟨mq-boolean⟩ und seinem Wert.
    MqBoolean(bool),
}

/// Die Werte eines Bereichsmerkmals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RangeKind {
    Length,
    /// ⟨integer⟩
    Integer,
    Ratio,
    /// ⟨resolution⟩ oder `infinite`
    Resolution,
}

const fn ranged(name: &'static str, values: RangeKind, device: fn(&Device) -> f64) -> Definition {
    Definition {
        name,
        kind: Kind::Range { values, device },
    }
}

const fn discrete(
    name: &'static str,
    values: &'static [&'static str],
    current: fn(&Device) -> &'static str,
) -> Definition {
    Definition {
        name,
        kind: Kind::Keywords {
            values,
            current,
            falsy: &[],
        },
    }
}

/// Die Merkmale `prefers-…`, deren Wert `no-preference` im *boolean
/// context* falsch ist.
const fn preference(name: &'static str, values: &'static [&'static str]) -> Definition {
    Definition {
        name,
        kind: Kind::Keywords {
            values,
            current: |_| "no-preference",
            falsy: &["no-preference"],
        },
    }
}

const COLOR_GAMUTS: &[&str] = &["srgb", "p3", "rec2020"];
const DYNAMIC_RANGES: &[&str] = &["standard", "high"];
const POINTERS: &[&str] = &["none", "coarse", "fine"];
const HOVERS: &[&str] = &["none", "hover"];
const COLOR_SCHEMES: &[&str] = &["light", "dark"];

/// Alle Merkmale aus Media Queries 5 mit den Werten dieses Browsers, siehe
/// die Moduldokumentation.
static FEATURES: &[Definition] = &[
    ranged("width", RangeKind::Length, |device| device.viewport_width),
    ranged("height", RangeKind::Length, |device| device.viewport_height),
    ranged("aspect-ratio", RangeKind::Ratio, |device| {
        ratio(device.viewport_width, device.viewport_height)
    }),
    discrete("orientation", &["portrait", "landscape"], |device| {
        if device.viewport_height >= device.viewport_width {
            "portrait"
        } else {
            "landscape"
        }
    }),
    discrete("overflow-block", &["none", "scroll", "paged"], |_| "scroll"),
    discrete("overflow-inline", &["none", "scroll"], |_| "scroll"),
    ranged("horizontal-viewport-segments", RangeKind::Integer, |_| 1.0),
    ranged("vertical-viewport-segments", RangeKind::Integer, |_| 1.0),
    discrete(
        "display-mode",
        &[
            "application",
            "fullscreen",
            "standalone",
            "minimal-ui",
            "browser",
            "picture-in-picture",
        ],
        |_| "browser",
    ),
    ranged("resolution", RangeKind::Resolution, |device| {
        device.device_pixel_ratio
    }),
    discrete("scan", &["interlace", "progressive"], |_| "progressive"),
    Definition {
        name: "grid",
        kind: Kind::MqBoolean(false),
    },
    discrete("update", &["none", "slow", "fast"], |_| "fast"),
    discrete(
        "environment-blending",
        &["opaque", "additive", "subtractive"],
        |_| "opaque",
    ),
    ranged("color", RangeKind::Integer, |_| 8.0),
    ranged("color-index", RangeKind::Integer, |_| 0.0),
    ranged("monochrome", RangeKind::Integer, |_| 0.0),
    discrete("color-gamut", COLOR_GAMUTS, |_| "srgb"),
    discrete("dynamic-range", DYNAMIC_RANGES, |_| "standard"),
    discrete("inverted-colors", &["none", "inverted"], |_| "none"),
    discrete("pointer", POINTERS, |_| "fine"),
    discrete("hover", HOVERS, |_| "hover"),
    discrete("any-pointer", POINTERS, |_| "fine"),
    discrete("any-hover", HOVERS, |_| "hover"),
    discrete("nav-controls", &["none", "back"], |_| "none"),
    discrete("video-color-gamut", COLOR_GAMUTS, |_| "srgb"),
    discrete("video-dynamic-range", DYNAMIC_RANGES, |_| "standard"),
    discrete(
        "scripting",
        &["none", "initial-only", "enabled"],
        |_| "none",
    ),
    preference("prefers-reduced-motion", &["no-preference", "reduce"]),
    preference("prefers-reduced-transparency", &["no-preference", "reduce"]),
    preference(
        "prefers-contrast",
        &["no-preference", "less", "more", "custom"],
    ),
    discrete("forced-colors", &["none", "active"], |_| "none"),
    discrete("prefers-color-scheme", COLOR_SCHEMES, |_| "light"),
    discrete("ua-color-scheme", COLOR_SCHEMES, |_| "light"),
    preference("prefers-reduced-data", &["no-preference", "reduce"]),
    ranged("device-width", RangeKind::Length, |device| {
        device.viewport_width
    }),
    ranged("device-height", RangeKind::Length, |device| {
        device.viewport_height
    }),
    ranged("device-aspect-ratio", RangeKind::Ratio, |device| {
        ratio(device.viewport_width, device.viewport_height)
    }),
];

/// Ein Verhältnis als Quotient; `0/0` ist unendlich, siehe die
/// Moduldokumentation.
fn ratio(first: f64, second: f64) -> f64 {
    if first == 0.0 && second == 0.0 {
        f64::INFINITY
    } else {
        first / second
    }
}

/// Die Definition zu einem Namen; Namen sind *ASCII case-insensitive*.
fn definition(name: &str) -> Option<&'static Definition> {
    FEATURES
        .iter()
        .find(|definition| definition.name.eq_ignore_ascii_case(name))
}

/// Die Definition eines Bereichsmerkmals zu einem Namen mit `min-` oder
/// `max-` und der Vergleich, den das Präfix bedeutet.
fn prefixed(name: &str) -> Option<(&'static Definition, Comparison)> {
    [
        ("min-", Comparison::GreaterEqual),
        ("max-", Comparison::LessEqual),
    ]
    .into_iter()
    .find_map(|(prefix, comparison)| {
        let head = name.get(..prefix.len())?;
        let rest = name.get(prefix.len()..)?;
        let definition = definition(rest)?;
        (head.eq_ignore_ascii_case(prefix) && matches!(definition.kind, Kind::Range { .. }))
            .then_some((definition, comparison))
    })
}

fn is_whitespace(value: &ComponentValue<'_>) -> bool {
    matches!(value, ComponentValue::Token(Token::Whitespace))
}

/// Ohne Leerraum am Anfang und am Ende.
pub(crate) fn trim<'v, 'a>(values: &'v [ComponentValue<'a>]) -> &'v [ComponentValue<'a>] {
    let start = values
        .iter()
        .position(|value| !is_whitespace(value))
        .unwrap_or(values.len());
    let end = values
        .iter()
        .rposition(|value| !is_whitespace(value))
        .map_or(start, |end| end.saturating_add(1));
    values.get(start..end).unwrap_or_default()
}

/// Liest Komponentenwerte und überspringt Leerraum davor.
pub(crate) struct Cursor<'v, 'a> {
    values: &'v [ComponentValue<'a>],
}

impl<'v, 'a> Cursor<'v, 'a> {
    pub(crate) fn new(values: &'v [ComponentValue<'a>]) -> Cursor<'v, 'a> {
        Cursor { values }
    }

    fn skip_whitespace(&mut self) {
        while let [first, rest @ ..] = self.values
            && is_whitespace(first)
        {
            self.values = rest;
        }
    }

    pub(crate) fn next(&mut self) -> Option<&'v ComponentValue<'a>> {
        self.skip_whitespace();
        let (first, rest) = self.values.split_first()?;
        self.values = rest;
        Some(first)
    }

    pub(crate) fn ident(&mut self) -> Option<&'a str> {
        self.skip_whitespace();
        match self.values {
            [ComponentValue::Token(Token::Ident(ident)), rest @ ..] => {
                self.values = rest;
                Some(ident.as_str())
            }
            _ => None,
        }
    }

    /// Das Schlüsselwort `keyword`, ohne Unterschied zwischen ASCII-Groß-
    /// und -Kleinbuchstaben.
    pub(crate) fn keyword(&mut self, keyword: &str) -> bool {
        let mut ahead = Cursor {
            values: self.values,
        };
        if ahead
            .ident()
            .is_some_and(|ident| ident.eq_ignore_ascii_case(keyword))
        {
            *self = ahead;
            true
        } else {
            false
        }
    }

    pub(crate) fn at_end(&mut self) -> bool {
        self.skip_whitespace();
        self.values.is_empty()
    }

    pub(crate) fn rest(&self) -> &'v [ComponentValue<'a>] {
        self.values
    }
}

/// Ob Komponentenwerte ein ⟨any-value⟩ sind, gegebenenfalls leer: ohne
/// ⟨bad-string-token⟩, ⟨bad-url-token⟩ und schließende Klammern ohne
/// öffnende, auch in Funktionen und Blöcken.
pub(crate) fn is_any_value(values: Values<'_>) -> bool {
    let mut stack = vec![values.iter()];
    while let Some(iter) = stack.last_mut() {
        let Some(value) = iter.next() else {
            stack.pop();
            continue;
        };
        match value {
            ComponentValue::Token(
                Token::BadString
                | Token::BadUrl
                | Token::CloseParenthesis
                | Token::CloseBracket
                | Token::CloseBrace,
            ) => return false,
            ComponentValue::Token(_) => {}
            ComponentValue::Function { value, .. } | ComponentValue::Block { value, .. } => {
                stack.push(value.iter());
            }
        }
    }
    true
}

/// Eine ⟨media-query⟩; `None`, wenn sie nicht zur Grammatik passt.
fn media_query(values: &[ComponentValue<'_>]) -> Option<MediaQuery> {
    if let Some(condition) = condition(values, true, 0) {
        return Some(MediaQuery::Condition(condition));
    }
    let mut input = Cursor::new(values);
    let first = input.ident()?;
    let not = first.eq_ignore_ascii_case("not");
    let media_type = if not || first.eq_ignore_ascii_case("only") {
        MediaType::from_ident(input.ident()?)?
    } else {
        MediaType::from_ident(first)?
    };
    let condition = if input.at_end() {
        None
    } else if input.keyword("and") {
        Some(condition(input.rest(), false, 0)?)
    } else {
        return None;
    };
    Some(MediaQuery::Typed {
        not,
        media_type,
        condition,
    })
}

/// Eine ⟨media-condition⟩, mit `or == false` eine
/// ⟨media-condition-without-or⟩.
fn condition(values: &[ComponentValue<'_>], or: bool, depth: usize) -> Option<Condition> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut input = Cursor::new(values);
    if input.keyword("not") {
        let inner = in_parens(input.next()?, depth)?;
        return input.at_end().then(|| Condition::Not(Box::new(inner)));
    }
    let mut conditions = vec![in_parens(input.next()?, depth)?];
    // Ob die Bedingungen mit `or` verbunden sind; mischen geht nicht.
    let mut disjunction = None;
    while !input.at_end() {
        let is_or = if input.keyword("and") {
            false
        } else if or && input.keyword("or") {
            true
        } else {
            return None;
        };
        if disjunction.is_some_and(|previous| previous != is_or) {
            return None;
        }
        disjunction = Some(is_or);
        conditions.push(in_parens(input.next()?, depth)?);
    }
    match disjunction {
        None => conditions.pop(),
        Some(false) => Some(Condition::And(conditions)),
        Some(true) => Some(Condition::Or(conditions)),
    }
}

/// Eine ⟨media-in-parens⟩. ⟨general-enclosed⟩ gilt erst, wenn nichts
/// anderes passt.
fn in_parens(value: &ComponentValue<'_>, depth: usize) -> Option<Condition> {
    match value {
        ComponentValue::Block {
            kind: BlockKind::Parenthesis,
            value,
        } => {
            let values: Vec<ComponentValue<'_>> = value.iter().collect();
            if let Some(condition) = condition(&values, true, depth.saturating_add(1)) {
                return Some(condition);
            }
            if let Some(feature) = feature(&values) {
                return Some(Condition::Feature(feature));
            }
            is_any_value(*value).then_some(Condition::GeneralEnclosed)
        }
        ComponentValue::Function { value, .. } => {
            is_any_value(*value).then_some(Condition::GeneralEnclosed)
        }
        _ => None,
    }
}

/// Ein ⟨mf-value⟩, wie er steht.
#[derive(Clone, Debug, PartialEq)]
enum MfValue<'a> {
    Number { value: f64, integer: bool },
    Dimension { value: f64, unit: &'a str },
    Ident(&'a str),
    Ratio(f64, f64),
}

fn mf_value<'a>(values: &[ComponentValue<'a>]) -> Option<MfValue<'a>> {
    let values: Vec<&ComponentValue<'a>> = values
        .iter()
        .filter(|value| !is_whitespace(value))
        .collect();
    let number = |value: &ComponentValue<'a>| match value {
        ComponentValue::Token(Token::Number { value, .. }) if *value >= 0.0 => Some(clamp(*value)),
        _ => None,
    };
    match values.as_slice() {
        [ComponentValue::Token(Token::Number { value, integer, .. })] => Some(MfValue::Number {
            value: clamp(*value),
            integer: *integer,
        }),
        [ComponentValue::Token(Token::Dimension { value, unit, .. })] => Some(MfValue::Dimension {
            value: clamp(*value),
            unit: unit.as_str(),
        }),
        [ComponentValue::Token(Token::Ident(ident))] => Some(MfValue::Ident(ident.as_str())),
        // ⟨ratio⟩ = ⟨number [0,∞]⟩ [ / ⟨number [0,∞]⟩ ]?
        [first, ComponentValue::Token(Token::Delim('/')), second] => {
            Some(MfValue::Ratio(number(first)?, number(second)?))
        }
        _ => None,
    }
}

/// Ein ⟨media-feature⟩ aus dem Inhalt von Klammern; `None`, wenn er nicht
/// zur Grammatik passt. Ein unbekannter Name oder ein Wert, der nicht zum
/// Merkmal passt, ergibt [`Feature::Unknown`].
fn feature(values: &[ComponentValue<'_>]) -> Option<Feature> {
    let values = trim(values);
    // ⟨mf-boolean⟩. Ein Bereichsmerkmal mit `min-` oder `max-` ist hier ein
    // Syntaxfehler.
    if let [ComponentValue::Token(Token::Ident(name))] = values {
        return match definition(name) {
            Some(definition) => Some(Feature::Boolean(definition)),
            None if prefixed(name).is_some() => None,
            None => Some(Feature::Unknown),
        };
    }
    // ⟨mf-plain⟩
    if let [ComponentValue::Token(Token::Ident(name)), rest @ ..] = values {
        let mut input = Cursor::new(rest);
        if matches!(input.next(), Some(ComponentValue::Token(Token::Colon))) {
            let value = mf_value(input.rest())?;
            return Some(plain(name, &value));
        }
    }
    // ⟨mf-range⟩: Werte und Vergleiche abwechselnd. `<=` und `>=` stehen
    // ohne Leerraum zwischen den Zeichen.
    let mut operands = Vec::new();
    let mut comparisons = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let delim = |index: usize| match values.get(index) {
        Some(ComponentValue::Token(Token::Delim(delim))) => Some(*delim),
        _ => None,
    };
    while index < values.len() {
        let comparison = match (delim(index), delim(index.saturating_add(1))) {
            (Some('<'), Some('=')) => Some((Comparison::LessEqual, 2)),
            (Some('<'), _) => Some((Comparison::Less, 1)),
            (Some('>'), Some('=')) => Some((Comparison::GreaterEqual, 2)),
            (Some('>'), _) => Some((Comparison::Greater, 1)),
            (Some('='), _) => Some((Comparison::Equal, 1)),
            _ => None,
        };
        match comparison {
            Some((comparison, length)) => {
                operands.push(values.get(start..index)?);
                comparisons.push(comparison);
                index = index.saturating_add(length);
                start = index;
            }
            None => index = index.saturating_add(1),
        }
    }
    operands.push(values.get(start..)?);
    match (operands.as_slice(), comparisons.as_slice()) {
        // Steht links ein Bezeichner, ist er der Name, außer er ist kein
        // Merkmal und rechts steht einer: `(infinite > resolution)`.
        ([left, right], [comparison]) => match (single_ident(left), single_ident(right)) {
            (Some(name), None) => Some(range(name, &[(*comparison, mf_value(right)?)])),
            (Some(name), Some(_)) if definition(name).is_some() => {
                Some(range(name, &[(*comparison, mf_value(right)?)]))
            }
            (_, Some(name)) => Some(range(name, &[(comparison.flip(), mf_value(left)?)])),
            (None, None) => None,
        },
        ([low, middle, high], [first, second])
            if first.same_direction(*second) && *first != Comparison::Equal =>
        {
            let name = single_ident(middle)?;
            let low = mf_value(low)?;
            let high = mf_value(high)?;
            Some(range(name, &[(first.flip(), low), (*second, high)]))
        }
        _ => None,
    }
}

/// Ein einzelner Bezeichner, mit Leerraum davor und danach.
fn single_ident<'a>(values: &[ComponentValue<'a>]) -> Option<&'a str> {
    match trim(values) {
        [ComponentValue::Token(Token::Ident(name))] => Some(name.as_str()),
        _ => None,
    }
}

/// `name: wert`
fn plain(name: &str, value: &MfValue<'_>) -> Feature {
    let (definition, comparison) = match definition(name) {
        Some(definition) => (definition, Comparison::Equal),
        None => match prefixed(name) {
            Some(prefixed) => prefixed,
            None => return Feature::Unknown,
        },
    };
    let feature = match definition.kind {
        Kind::Range { values, .. } => specified(values, value).map(|specified| Feature::Range {
            definition,
            comparisons: vec![(comparison, specified)],
        }),
        Kind::Keywords { values, .. } => match value {
            MfValue::Ident(ident) => values
                .iter()
                .copied()
                .find(|keyword| keyword.eq_ignore_ascii_case(ident))
                .map(|keyword| Feature::Keyword {
                    definition,
                    keyword,
                }),
            _ => None,
        },
        Kind::MqBoolean(_) => match value {
            MfValue::Number {
                value,
                integer: true,
            } if *value == 0.0 || *value == 1.0 => Some(Feature::MqBoolean {
                definition,
                value: *value == 1.0,
            }),
            _ => None,
        },
    };
    feature.unwrap_or(Feature::Unknown)
}

/// Ein Bereichsmerkmal mit Vergleichen; Merkmale mit einzelnen Werten und
/// Namen mit `min-` und `max-` sind hier unbekannt.
fn range(name: &str, comparisons: &[(Comparison, MfValue<'_>)]) -> Feature {
    let Some(definition) = definition(name) else {
        return Feature::Unknown;
    };
    let Kind::Range { values, .. } = definition.kind else {
        return Feature::Unknown;
    };
    comparisons
        .iter()
        .map(|(comparison, value)| Some((*comparison, specified(values, value)?)))
        .collect::<Option<Vec<_>>>()
        .map_or(Feature::Unknown, |comparisons| Feature::Range {
            definition,
            comparisons,
        })
}

/// Ein angegebener Wert eines Bereichsmerkmals; `None`, wenn er nicht zum
/// Merkmal passt.
fn specified(kind: RangeKind, value: &MfValue<'_>) -> Option<Specified> {
    match (kind, value) {
        (RangeKind::Length, MfValue::Dimension { value, unit }) => {
            Some(Specified::Length(Length {
                value: *value,
                unit: LengthUnit::from_name(unit)?,
            }))
        }
        // Eine Länge null darf ohne Einheit stehen.
        (RangeKind::Length, MfValue::Number { value, .. }) if *value == 0.0 => {
            Some(Specified::Length(Length::px(*value)))
        }
        (
            RangeKind::Integer,
            MfValue::Number {
                value,
                integer: true,
            },
        ) => Some(Specified::Number(*value)),
        (RangeKind::Ratio, MfValue::Number { value, .. }) if *value >= 0.0 => {
            Some(Specified::Number(ratio(*value, 1.0)))
        }
        (RangeKind::Ratio, MfValue::Ratio(first, second)) => {
            Some(Specified::Number(ratio(*first, *second)))
        }
        (RangeKind::Resolution, MfValue::Dimension { value, unit }) => {
            // `dppx` und `x` sind Gerätepixel je `px`; 1in = 96px = 2,54cm.
            let factor = if unit.eq_ignore_ascii_case("dppx") || unit.eq_ignore_ascii_case("x") {
                1.0
            } else if unit.eq_ignore_ascii_case("dpi") {
                1.0 / 96.0
            } else if unit.eq_ignore_ascii_case("dpcm") {
                2.54 / 96.0
            } else {
                return None;
            };
            Some(Specified::Number(*value * factor))
        }
        (RangeKind::Resolution, MfValue::Ident(ident))
            if ident.eq_ignore_ascii_case("infinite") =>
        {
            Some(Specified::Number(f64::INFINITY))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::{Font, FontMetrics, NoFonts, test_device};
    use mb_base::Cancel;
    use std::cell::Cell;

    #[test]
    fn empty_and_nonempty_queries_observe_budget_and_cancellation() {
        let device = test_device();
        for text in ["", "all", "not all", "(color)"] {
            let list = MediaQueryList::parse(text);
            assert_eq!(
                list.matches_within(&device, &NoFonts, &Budget::new(0)),
                Err(Exhausted::Steps),
                "{text}"
            );
            let cancel = Cancel::new();
            cancel.cancel();
            let budget = Budget::with_cancel(100, cancel);
            assert_eq!(
                list.matches_within(&device, &NoFonts, &budget),
                Err(Exhausted::Cancelled),
                "{text}"
            );
        }
    }

    #[test]
    fn wide_conditions_and_query_lists_are_bounded() {
        // Beides ist gültig und liefert unbegrenzt false bzw. true. Es
        // darf mit kleinem Budget nicht bis zum Ende ausgewertet werden.
        let texts = [
            vec!["print"; 2_000].join(", "),
            vec!["(color)"; 2_000].join(" and "),
        ];
        for text in texts {
            let list = MediaQueryList::parse(&text);
            let budget = Budget::new(32);
            assert_eq!(
                list.matches_within(&test_device(), &NoFonts, &budget),
                Err(Exhausted::Steps)
            );
        }
    }

    struct CancellingFonts {
        cancel: Cancel,
        calls: Cell<usize>,
    }

    impl FontMetricsSource for CancellingFonts {
        fn metrics(&self, _: &Font<'_>) -> FontMetrics {
            self.calls.set(self.calls.get() + 1);
            self.cancel.cancel();
            FontMetrics::default()
        }
    }

    #[test]
    fn cancellation_during_font_metrics_is_not_an_unknown_match() {
        let cancel = Cancel::new();
        let fonts = CancellingFonts {
            cancel: cancel.clone(),
            calls: Cell::new(0),
        };
        let list = MediaQueryList::parse("(width: 1cap), all");
        let budget = Budget::with_cancel(100, cancel);
        assert_eq!(
            list.matches_within(&test_device(), &fonts, &budget),
            Err(Exhausted::Cancelled)
        );
        assert_eq!(fonts.calls.get(), 1);
    }

    struct ExhaustingFonts<'a>(&'a Budget);

    impl FontMetricsSource for ExhaustingFonts<'_> {
        fn metrics(&self, _: &Font<'_>) -> FontMetrics {
            self.0.spend(u64::MAX);
            FontMetrics::default()
        }
    }

    #[test]
    fn exhaustion_during_font_metrics_is_propagated() {
        let budget = Budget::new(100);
        let list = MediaQueryList::parse("(width: 1cap), all");
        assert_eq!(
            list.matches_within(&test_device(), &ExhaustingFonts(&budget), &budget),
            Err(Exhausted::Steps)
        );
    }

    fn matches_on(device: &Device, text: &str) -> bool {
        MediaQueryList::parse(text).matches(device, &NoFonts)
    }

    /// Auf dem Gerät für Tests, 800 × 600 wie das Fenster der WPT.
    fn matches(text: &str) -> bool {
        matches_on(&test_device(), text)
    }

    /// Wie viele Media Queries eine Liste hat und welche davon `not all`
    /// sind.
    fn shape(text: &str) -> Vec<bool> {
        MediaQueryList::parse(text)
            .queries
            .iter()
            .map(|query| matches!(query, MediaQuery::NotAll))
            .collect()
    }

    #[test]
    fn list_parsing() {
        // Nach `match-media-parsing.html`; `true` steht für `not all`.
        assert!(shape("").is_empty());
        assert!(shape(" ").is_empty());
        assert_eq!(shape(" all , all "), [false, false]);
        assert_eq!(shape(" ( color   "), [false]);
        assert_eq!(shape("  color)"), [true]);
        assert_eq!(shape("  color ), ( color"), [true, false]);
        assert_eq!(shape(" foo "), [false]);
        assert_eq!(shape(","), [true, true]);
        assert_eq!(shape("  ,  ,  "), [true, true, true]);
        assert_eq!(shape(" foo,"), [false, true]);
        assert!(matches(""));
        assert!(matches(" "));
        assert!(!matches(","));
        assert!(matches("foo, all"));
        // Der Fehler bleibt in seiner Media Query.
        assert!(matches("(example, all,), all"));
        assert!(!matches("(example, all,), speech"));
    }

    #[test]
    fn media_types() {
        assert!(matches("all"));
        assert!(matches("SCREEN"));
        assert!(matches("only screen"));
        assert!(!matches("print"));
        assert!(!matches("tv"));
        assert!(!matches("unknown"));
        assert!(matches("not print"));
        assert!(matches("not unknown"));
        assert!(!matches("not all"));
        // Nach `mq-invalid-media-type-*.html`: Diese Schlüsselwörter sind
        // keine Medientypen, die Media Query wird zu `not all`.
        for text in [
            "and",
            "not and",
            "or",
            "not or",
            "not",
            "not not",
            "only",
            "not only",
            "layer",
            "not layer",
            "or and (color)",
        ] {
            assert_eq!(shape(text), [true], "{text}");
        }
        assert!(matches(
            "screen and (min-width: 480px), screen and (width: 768px)"
        ));
        assert!(!matches("screen and (min-width: 480px) screen"));
        assert!(!matches(
            "screen and (min-width: 480px) and (device-width: 768px)"
        ));
        assert!(!matches("screen and (color) or (grid)"));
    }

    #[test]
    fn conditions() {
        // Nach `negation-001.html` und `negation-002.html`.
        assert!(matches("not (monochrome)"));
        assert!(matches("(not (monochrome))"));
        assert!(matches("not (not (color))"));
        assert!(!matches("not ((unknown) or (monochrome))"));
        assert!(matches("((color) and (color))"));
        // `not` steht allein; mit `and` oder `or` daneben bleibt nur
        // ⟨general-enclosed⟩.
        assert!(!matches("(not (monochrome) and (color))"));
        assert!(!matches("(not (monochrome) or (color))"));
        assert!(!matches("((color) and not (monochrome))"));
        assert!(!matches("((color) or not (monochrome))"));
        // `and` und `or` mischen geht nicht.
        assert!(!matches("(color) and (color) or (color)"));
        // ⟨general-enclosed⟩ ist „unknown“; `or` mit einem wahren Teil ist
        // trotzdem wahr (`mq-range-001.html`), `not` davor bleibt unbekannt.
        assert!(matches("(width 500px) or (min-width: 0)"));
        assert!(!matches("not (width 500px)"));
        assert!(!matches("not unknown(function)"));
        assert!(!matches("not screen and (unknown)"));
        assert!(!matches("(color) and ()"));
        // Ohne Leerraum ist `not(` eine Funktion.
        assert!(!matches("not(monochrome)"));
        // Kein ⟨any-value⟩: die ganze Media Query wird zu `not all`.
        assert_eq!(shape("(color) or (a ])"), [true]);
        assert_eq!(shape("(color) or (\"a\n)"), [true]);
        assert!(matches("(color) or (a [ ] )"));
    }

    #[test]
    fn deep_nesting_needs_no_deep_recursion() {
        let depth = 100_000;
        let text = format!("{}{}", "(".repeat(depth), ")".repeat(depth));
        assert!(!matches(&text));
        let text = format!("{}color{}", "(".repeat(depth), ")".repeat(depth));
        assert!(!matches(&text));
        let text = format!("{}color{}", "(".repeat(50), ")".repeat(50));
        assert!(matches(&text));
    }

    #[test]
    fn ranges() {
        assert!(matches("(width: 800px)"));
        assert!(matches("(min-width: 800px) and (max-width: 800px)"));
        assert!(!matches("(min-width: 800.01px)"));
        assert!(matches("(width > 799px)"));
        assert!(matches("(799px < width)"));
        assert!(matches("(799px < width <= 800px)"));
        assert!(matches("(801px > width >= 800px)"));
        assert!(!matches("(799px < width < 800px)"));
        assert!(!matches("(799px < width > 800px)"));
        // Zwischen `<` und `=` darf kein Leerraum stehen.
        assert!(!matches("(width < = 800px)"));
        assert!(!matches("(width = 800px = 800px)"));
        assert!(matches("(height = 600px)"));
        assert!(matches("(50em = width)"));
        // Schriftbezogene Einheiten beziehen sich auf die Anfangswerte.
        assert!(matches("(width: 50rem)"));
        assert!(matches("(width: 100ex)"));
        assert!(matches("(width: 100vw)"));
        // Ohne Schriften fehlt die Versalhöhe; unbekannt.
        assert!(!matches("(width: 1cap)"));
        assert!(!matches("not (width: 1cap)"));
        // Präfixe nur bei Bereichsmerkmalen und nicht ohne Wert.
        assert!(!matches("(min-orientation: portrait)"));
        assert!(!matches("not (min-orientation: portrait)"));
        assert!(!matches("(min-width)"));
        assert!(!matches("(min-width > 0)"));
        assert!(!matches("(orientation > 0)"));
        assert!(matches("(MIN-WIDTH: 0)"));
        // Kein Wert, der zum Merkmal passt.
        assert!(!matches("not (width: 100)"));
        assert!(!matches("not (width: 1deg)"));
        assert!(!matches("not (color: 20example)"));
        assert!(!matches("not (width: 100px 200px)"));
    }

    #[test]
    fn negative_ranges() {
        // Nach `mq-negative-range-001.html` und `mq-negative-range-002.html`.
        assert!(matches("not (width <= -100px)"));
        assert!(matches("(height > -100px)"));
        assert!(matches("not (resolution: -300dpi)"));
        assert!(matches("(color > -1)"));
        assert!(!matches("(monochrome < -1)"));
        assert!(matches("(width: 0) or (width >= -0px)"));
        assert!(matches("(min-width: -100px) and (min-height: -100px)"));
    }

    #[test]
    fn booleans() {
        assert!(matches("(width)"));
        assert!(matches("(color)"));
        assert!(!matches("(color-index)"));
        assert!(!matches("(monochrome)"));
        assert!(!matches("(grid)"));
        assert!(matches("(grid: 0)"));
        assert!(!matches("(grid: 1)"));
        assert!(!matches("not (grid: 2)"));
        assert!(!matches("not (min-grid: 0)"));
        assert!(matches("(update)"));
        assert!(matches("(update: fast)"));
        assert!(!matches("(scripting)"));
        assert!(matches("(scripting: none)"));
        assert!(!matches("not (scripting: 0)"));
        assert!(!matches("(prefers-reduced-motion)"));
        assert!(matches("(prefers-reduced-motion: no-preference)"));
        assert!(!matches("(prefers-contrast)"));
        assert!(!matches("(forced-colors)"));
        assert!(!matches("(inverted-colors)"));
        assert!(!matches("(nav-controls)"));
        assert!(matches("(pointer) and (hover) and (any-pointer: fine)"));
        assert!(matches("(prefers-color-scheme: light)"));
        assert!(matches("(color-gamut: srgb) and (dynamic-range: standard)"));
        assert!(!matches("(color-gamut: p3)"));
        assert!(matches(
            "(overflow-block: scroll) and (overflow-inline: scroll)"
        ));
        assert!(matches("(display-mode: browser)"));
        assert!(matches("(horizontal-viewport-segments: 1)"));
        // Nicht nur ASCII: `İ` ist kein großes `i` (`mq-case-insensitive-001.html`).
        assert!(matches("(HEIGHT) and (Orientation: LANDSCAPE)"));
        assert!(!matches("(heİght)"));
        assert!(!matches("(--foo: bar)"));
    }

    #[test]
    fn orientation_and_ratios() {
        let mut device = test_device();
        assert!(matches_on(&device, "(orientation: landscape)"));
        device.viewport_width = 600.0;
        assert!(matches_on(&device, "(orientation: portrait)"));
        // Nach `aspect-ratio-001.html` bis `-006.html`.
        assert!(matches("screen and (min-aspect-ratio: 59/79)"));
        assert!(!matches("screen and (min-aspect-ratio: 0/0)"));
        assert!(matches("screen and (max-aspect-ratio: 1280/720)"));
        assert!(matches("screen and (max-aspect-ratio: 0/0)"));
        assert!(matches("screen and (max-aspect-ratio: 1280.1/720.01)"));
        assert!(matches("screen and (min-aspect-ratio: 0.2)"));
        assert!(matches("(aspect-ratio: 4/3) and (aspect-ratio: 8 / 6)"));
        assert!(matches("(device-aspect-ratio: 4/3)"));
        assert!(!matches("not (aspect-ratio: -4/3)"));
        assert!(matches("(aspect-ratio > 1/0) or (aspect-ratio < 1/0)"));
    }

    #[test]
    fn resolutions() {
        let mut device = test_device();
        device.device_pixel_ratio = 2.0;
        let on = |text: &str| matches_on(&device, text);
        assert!(on("(resolution: 2dppx)"));
        assert!(on("(resolution: 2x)"));
        assert!(on("(resolution: 192dpi)"));
        assert!(on("(min-resolution: 75dpcm) and (max-resolution: 76dpcm)"));
        assert!(on("(resolution < infinite)"));
        assert!(on("(infinite > resolution)"));
        assert!(!on("(resolution: infinite)"));
        assert!(!on("not (resolution: 2)"));
        assert!(on("(resolution)"));
    }
}
