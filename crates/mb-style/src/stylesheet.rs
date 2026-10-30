//! Stylesheets: die Regeln aus dem CSS-Parser mit gelesenen Selektoren und
//! Deklarationen.
//!
//! Unterstützt sind Stilregeln mit Verschachtelung nach CSS Nesting
//! (verschachtelte Stilregeln, *nested group rules* und *nested declarations
//! rules*), `@layer` als Block und als Anweisung und `@import` (CSS Cascade
//! 5), `@namespace` (CSS Namespaces 3) sowie `@media` und `@supports` (CSS
//! Conditional 3). Alle anderen At-Regeln fallen bisher weg; sie ändern dann
//! auch nicht, ob später noch `@import` und `@namespace` stehen dürfen.
//!
//! Die Stylesheets hinter `@import` lädt der Aufrufer
//! ([`StyleSheet::pending_imports`]). Ob die Bedingungen in `@media` und
//! nach `@import` zutreffen, entscheidet erst die Kaskade für ein Gerät; die
//! Bedingungen in `@supports` und `supports()` stehen schon beim Lesen fest.

use std::sync::Arc;

use mb_css::{
    AtRule, ComponentValue, ComponentValues, Context, Declaration, QualifiedRule, Rule, RuleId,
    RuleTree, Source, Token, parse_stylesheet,
};
use mb_url::Url;

use crate::declarations::{Declared, parse_declaration, parse_declarations};
use crate::media::MediaQueryList;
use crate::selectors::{Namespaces, SelectorList};
use crate::supports;
use crate::values::{ParseContext, is_css_wide_keyword};

/// Wie tief Regeln verschachtelt sein dürfen; tiefere fallen weg. Das
/// Lesen steigt rekursiv ab, die Grenze schützt vor einem Stapelüberlauf.
const MAX_DEPTH: usize = 100;

/// Wie tief Stylesheets mit `@import` ineinander stecken dürfen; tiefere
/// lädt der Aufrufer nicht, und die Kaskade liest sie nicht. Festgelegt von
/// diesem Browser; CSS Cascade 5 nennt keine Grenze.
pub const MAX_IMPORT_DEPTH: usize = 32;

/// Ein gelesenes Stylesheet.
#[derive(Clone, Debug, Default)]
pub struct StyleSheet {
    pub(crate) rules: Vec<CssRule>,
    /// Die gültigen `@import`-Regeln. Sie stehen hinter den ersten
    /// `import_position` Regeln, die alle `@layer`-Anweisungen sind.
    pub(crate) imports: Vec<ImportRule>,
    pub(crate) import_position: usize,
    /// Die Media Queries des Elements, das auf das Stylesheet verweist.
    pub(crate) media: Option<Arc<MediaQueryList>>,
}

/// Eine Regel eines Stylesheets.
#[derive(Clone, Debug)]
pub(crate) enum CssRule {
    Style(StyleRule),
    /// Ein `@layer`-Block; ohne Namen eine anonyme Ebene.
    LayerBlock {
        name: Option<LayerName>,
        rules: Vec<CssRule>,
    },
    /// `@layer a, b;`
    LayerStatement(Vec<LayerName>),
    /// *Nested declarations rule*: Deklarationen zwischen Regeln in einer
    /// Stilregel oder einer *nested group rule*. Sie gelten mit den
    /// Selektoren der nächsten Stilregel darüber.
    NestedDeclarations(Arc<[Declared]>),
    /// `@media`
    Media {
        queries: Arc<MediaQueryList>,
        rules: Vec<CssRule>,
    },
    /// `@supports` mit dem Ergebnis der Bedingung
    Supports {
        matches: bool,
        rules: Vec<CssRule>,
    },
}

/// Eine `@import`-Regel.
#[derive(Clone, Debug)]
pub(crate) struct ImportRule {
    /// `None`, wenn die URL leer ist oder sich nicht auflösen lässt.
    pub url: Option<Url>,
    /// `layer` (`Some(None)`, eine anonyme Ebene) oder `layer(<name>)`.
    pub layer: Option<Option<LayerName>>,
    /// Ob die Bedingung in `supports()` erfüllt ist; ohne sie `true`.
    pub supports: bool,
    pub media: Arc<MediaQueryList>,
    /// Das geladene Stylesheet.
    pub sheet: Option<Box<StyleSheet>>,
}

/// Eine `@import`-Regel, deren Stylesheet noch geladen werden soll.
#[derive(Debug)]
pub struct PendingImport<'s> {
    url: Url,
    rule: &'s mut ImportRule,
}

impl PendingImport<'_> {
    /// Die URL des Stylesheets, aufgelöst zur Basis des importierenden.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Setzt das geladene Stylesheet ein. Es gilt mit dem Ursprung und an
    /// der Stelle der Regel, auch wenn dasselbe Stylesheet noch anderswo
    /// eingebunden ist (CSS Cascade 5).
    pub fn load(self, sheet: StyleSheet) {
        self.rule.sheet = Some(Box::new(sheet));
    }
}

/// Eine Stilregel (*style rule*).
#[derive(Clone, Debug)]
pub(crate) struct StyleRule {
    pub selectors: Arc<SelectorList>,
    pub declarations: Arc<[Declared]>,
    /// Verschachtelte Regeln in ihrer Reihenfolge.
    pub rules: Vec<CssRule>,
}

/// Ein ⟨layer-name⟩: die Bezeichner der Ebenen von außen nach innen.
pub(crate) type LayerName = Vec<String>;

impl StyleSheet {
    /// Liest ein Stylesheet aus Text. `context` gibt die Basis für relative
    /// URLs und den *quirks mode* des Dokuments.
    pub fn parse(text: &str, context: ParseContext<'_>) -> StyleSheet {
        let grammar = Grammar {
            block: Block::Rules,
            values: context,
        };
        let tree = parse_stylesheet(&Source::new(text), &grammar);
        let reader = Reader {
            tree: &tree,
            namespaces: Namespaces::default(),
            values: context,
        };
        reader.top_level()
    }

    /// Setzt die Media Queries des Elements, das auf das Stylesheet
    /// verweist, etwa aus dem Attribut `media`.
    pub fn set_media(&mut self, media: MediaQueryList) {
        self.media = Some(Arc::new(media));
    }

    /// Die `@import`-Regeln, deren Stylesheet noch geladen werden soll: mit
    /// einer URL und ohne eine Bedingung in `supports()`, die nicht erfüllt
    /// ist; dann darf es nicht geladen werden (CSS Cascade 5). Media Queries
    /// verhindern das Laden nicht, denn sie können sich ändern.
    pub fn pending_imports(&mut self) -> Vec<PendingImport<'_>> {
        self.imports
            .iter_mut()
            .filter(|rule| rule.supports && rule.sheet.is_none())
            .filter_map(|rule| {
                let url = rule.url.clone()?;
                Some(PendingImport { url, rule })
            })
            .collect()
    }
}

/// Was in einem Block erlaubt ist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Block {
    /// ⟨rule-list⟩: das Stylesheet oder eine Gruppenregel darin.
    Rules,
    /// ⟨block-contents⟩ einer Stilregel oder einer *nested group rule*:
    /// Deklarationen und Regeln.
    Style,
    /// Der Block einer At-Regel, die wegfällt.
    Ignored,
}

/// Der Kontext für den Parser: Er entscheidet, welche Deklarationen gültig
/// sind. Ob eine Deklaration gilt, ändert das Parsen, denn eine ungültige
/// wird noch einmal als verschachtelte Regel gelesen. Selektoren und
/// At-Regeln prüft erst [`Reader`]; ob der Parser sie ablehnt oder nicht,
/// ändert an den übrigen Regeln nichts.
#[derive(Clone, Copy)]
struct Grammar<'a> {
    block: Block,
    values: ParseContext<'a>,
}

impl<'a> Context for Grammar<'a> {
    fn accepts_declaration(&self, declaration: &Declaration) -> bool {
        self.block == Block::Style && parse_declaration(declaration, self.values).is_some()
    }

    fn accepts_at_rule(&self, _: &AtRule) -> bool {
        true
    }

    fn accepts_qualified_rule(&self, _: &QualifiedRule) -> bool {
        true
    }

    fn at_rule_block(&self, name: &str, _: &ComponentValues) -> Grammar<'a> {
        let block = if self.block != Block::Ignored && is_group_rule(name) {
            self.block
        } else {
            Block::Ignored
        };
        Grammar {
            block,
            values: self.values,
        }
    }

    fn qualified_rule_block(&self, _: &ComponentValues) -> Grammar<'a> {
        let block = match self.block {
            Block::Ignored => Block::Ignored,
            Block::Rules | Block::Style => Block::Style,
        };
        Grammar {
            block,
            values: self.values,
        }
    }
}

/// Die unterstützten Gruppenregeln, deren Block Regeln enthält.
fn is_group_rule(name: &str) -> bool {
    ["layer", "media", "supports"]
        .iter()
        .any(|group| name.eq_ignore_ascii_case(group))
}

/// Wie weit ein Stylesheet ist: `@import` und `@namespace` dürfen nur am
/// Anfang stehen, davor nur `@layer`-Anweisungen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Start,
    Imports,
    Namespaces,
    Body,
}

/// Liest die Regeln aus dem Baum des Parsers.
struct Reader<'t, 'v> {
    tree: &'t RuleTree,
    namespaces: Namespaces,
    values: ParseContext<'v>,
}

/// Die Stilregel, in der eine Regel steht, für `&` und *nested declarations
/// rules*.
type Parent<'p> = Option<&'p Arc<SelectorList>>;

impl Reader<'_, '_> {
    /// Das Stylesheet aus den Regeln auf oberster Ebene.
    fn top_level(mut self) -> StyleSheet {
        let mut rules = Vec::new();
        let mut imports = Vec::new();
        let mut import_position = 0;
        let mut stage = Stage::Start;
        for &id in self.tree.roots() {
            let Some(rule) = self.tree.get(id) else {
                continue;
            };
            match rule {
                Rule::At(at) if at.name.eq_ignore_ascii_case("import") => {
                    if stage <= Stage::Imports
                        && at.block.is_none()
                        && let Some(import) = self.import(&at.prelude)
                    {
                        if imports.is_empty() {
                            import_position = rules.len();
                        }
                        imports.push(import);
                        stage = Stage::Imports;
                    }
                }
                Rule::At(at) if at.name.eq_ignore_ascii_case("namespace") => {
                    if stage <= Stage::Namespaces
                        && at.block.is_none()
                        && let Some((prefix, url)) = namespace(&at.prelude)
                    {
                        self.namespaces.declare(prefix.as_deref(), &url);
                        stage = Stage::Namespaces;
                    }
                }
                _ => {
                    if let Some(rule) = self.rule(id, None, 0) {
                        let statement = matches!(rule, CssRule::LayerStatement(_));
                        if !(statement && stage == Stage::Start) {
                            stage = Stage::Body;
                        }
                        rules.push(rule);
                    }
                }
            }
        }
        StyleSheet {
            rules,
            imports,
            import_position,
            media: None,
        }
    }

    /// Wofür Bedingungen in `@supports` gelesen werden.
    fn supports_environment(&self) -> supports::Environment<'_> {
        supports::Environment {
            namespaces: &self.namespaces,
            values: self.values,
        }
    }

    /// `@import [ <url> | <string> ] [ layer | layer(<layer-name>) ]?
    /// <import-conditions>`. Ein ungültiges `layer()` oder `supports()`
    /// gehört dann zu den Media Queries, als ⟨general-enclosed⟩.
    fn import(&self, prelude: &ComponentValues) -> Option<ImportRule> {
        let values = trimmed(prelude);
        let (first, rest) = values.split_first()?;
        let text = url_text(first)?;
        let url = if text.is_empty() {
            None
        } else {
            Url::parse(text, self.values.base_url).ok()
        };
        let mut rest = skip_whitespace(rest);
        let mut layer = None;
        match rest.split_first() {
            Some((ComponentValue::Token(Token::Ident(keyword)), tail))
                if keyword.eq_ignore_ascii_case("layer") =>
            {
                layer = Some(None);
                rest = skip_whitespace(tail);
            }
            Some((ComponentValue::Function { name, value }, tail))
                if name.eq_ignore_ascii_case("layer") =>
            {
                let arguments: Vec<ComponentValue<'_>> = value.iter().collect();
                if let Some(name) = layer_name(&arguments) {
                    layer = Some(Some(name));
                    rest = skip_whitespace(tail);
                }
            }
            _ => {}
        }
        let mut matches = true;
        if let Some((ComponentValue::Function { name, value }, tail)) = rest.split_first()
            && name.eq_ignore_ascii_case("supports")
            && let Some(result) = supports::import_condition(*value, self.supports_environment())
        {
            matches = result;
            rest = tail;
        }
        Some(ImportRule {
            url,
            layer,
            supports: matches,
            media: Arc::new(MediaQueryList::from_values(rest)),
            sheet: None,
        })
    }

    /// Eine Regel in einem Stylesheet, einer Gruppenregel oder einer
    /// Stilregel (`parent`). `None`, wenn sie ungültig ist oder wegfällt.
    fn rule(&self, id: RuleId, parent: Parent<'_>, depth: usize) -> Option<CssRule> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.tree.get(id)? {
            Rule::Qualified(rule) => self.style_rule(rule, parent, depth).map(CssRule::Style),
            Rule::At(rule) if rule.name.eq_ignore_ascii_case("layer") => {
                self.layer(rule, parent, depth)
            }
            Rule::At(rule) if rule.name.eq_ignore_ascii_case("media") => {
                let block = rule.block.as_ref()?;
                let prelude: Vec<ComponentValue<'_>> = rule.prelude.iter().collect();
                Some(CssRule::Media {
                    queries: Arc::new(MediaQueryList::from_values(&prelude)),
                    rules: self.children(block, parent, depth),
                })
            }
            // Mit einer Bedingung, die nicht zur Grammatik passt, fällt die
            // Regel samt Inhalt weg.
            Rule::At(rule) if rule.name.eq_ignore_ascii_case("supports") => {
                let block = rule.block.as_ref()?;
                let prelude: Vec<ComponentValue<'_>> = rule.prelude.iter().collect();
                let matches = supports::condition(&prelude, self.supports_environment())?;
                Some(CssRule::Supports {
                    matches,
                    rules: self.children(block, parent, depth),
                })
            }
            Rule::At(_) => None,
            // Deklarationen zwischen Regeln gibt es nur in Stilregeln und
            // darin verschachtelten Gruppenregeln.
            Rule::Declarations(declarations) => {
                parent?;
                let declarations = parse_declarations(declarations, self.values);
                (!declarations.is_empty()).then(|| CssRule::NestedDeclarations(declarations.into()))
            }
        }
    }

    fn style_rule(
        &self,
        rule: &QualifiedRule,
        parent: Parent<'_>,
        depth: usize,
    ) -> Option<StyleRule> {
        let values = rule.prelude.values();
        let selectors = match parent {
            None => SelectorList::from_values(values, &self.namespaces),
            Some(parent) => SelectorList::from_values_nested(values, &self.namespaces, parent),
        };
        let selectors = Arc::new(selectors.ok()?);
        let declarations = parse_declarations(&rule.declarations, self.values).into();
        let rules = self.children(&rule.rules, Some(&selectors), depth);
        Some(StyleRule {
            selectors,
            declarations,
            rules,
        })
    }

    fn children(&self, ids: &[RuleId], parent: Parent<'_>, depth: usize) -> Vec<CssRule> {
        let depth = depth.saturating_add(1);
        ids.iter()
            .filter_map(|&id| self.rule(id, parent, depth))
            .collect()
    }

    /// `@layer`: `@layer <layer-name>#;` oder `@layer <layer-name>? { … }`.
    fn layer(&self, rule: &AtRule, parent: Parent<'_>, depth: usize) -> Option<CssRule> {
        let names = layer_names(&rule.prelude)?;
        match &rule.block {
            None => (!names.is_empty()).then_some(CssRule::LayerStatement(names)),
            Some(block) => {
                let name = match <[LayerName; 1]>::try_from(names) {
                    Ok([name]) => Some(name),
                    Err(names) if names.is_empty() => None,
                    Err(_) => return None,
                };
                let rules = self.children(block, parent, depth);
                Some(CssRule::LayerBlock { name, rules })
            }
        }
    }
}

/// Die Werte eines Prelude ohne Leerraum am Anfang und am Ende.
fn trimmed<'a>(prelude: &'a ComponentValues) -> Vec<ComponentValue<'a>> {
    let values: Vec<ComponentValue<'a>> = prelude.iter().collect();
    let is_space =
        |value: &ComponentValue<'_>| matches!(value, ComponentValue::Token(Token::Whitespace));
    let start = values.iter().position(|value| !is_space(value));
    let end = values.iter().rposition(|value| !is_space(value));
    match (start, end) {
        (Some(start), Some(end)) => values.get(start..=end).unwrap_or_default().to_vec(),
        _ => Vec::new(),
    }
}

/// Ohne Leerraum am Anfang.
fn skip_whitespace<'v, 'a>(values: &'v [ComponentValue<'a>]) -> &'v [ComponentValue<'a>] {
    let start = values
        .iter()
        .position(|value| !matches!(value, ComponentValue::Token(Token::Whitespace)))
        .unwrap_or(values.len());
    values.get(start..).unwrap_or_default()
}

/// Der Text eines ⟨url⟩ oder ⟨string⟩: `url( <string> )`, `src( <string> )`
/// oder ein ⟨url-token⟩ (Values 4, ohne ⟨url-modifier⟩).
fn url_text<'a>(value: &ComponentValue<'a>) -> Option<&'a str> {
    match value {
        ComponentValue::Token(Token::String(text) | Token::Url(text)) => Some(text.as_str()),
        ComponentValue::Function { name, value }
            if name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("src") =>
        {
            let arguments: Vec<ComponentValue<'a>> = value
                .iter()
                .filter(|value| !matches!(value, ComponentValue::Token(Token::Whitespace)))
                .collect();
            match arguments.as_slice() {
                [ComponentValue::Token(Token::String(text))] => Some(text.as_str()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `@namespace <namespace-prefix>? [ <string> | <url> ]`: das Präfix und
/// der Namensraum. Eine URL bleibt Text, ohne Auflösen.
fn namespace(prelude: &ComponentValues) -> Option<(Option<String>, String)> {
    let values = trimmed(prelude);
    let (prefix, rest) = match values.as_slice() {
        [ComponentValue::Token(Token::Ident(prefix)), rest @ ..] => (Some(prefix.clone()), rest),
        rest => (None, rest),
    };
    let rest: Vec<&ComponentValue<'_>> = rest
        .iter()
        .filter(|value| !matches!(value, ComponentValue::Token(Token::Whitespace)))
        .collect();
    // Hinter der URL darf nichts mehr kommen.
    let url = match rest.as_slice() {
        [ComponentValue::Token(Token::String(url) | Token::Url(url))] => url.clone(),
        [ComponentValue::Function { name, value }] if name.eq_ignore_ascii_case("url") => {
            let arguments: Vec<ComponentValue<'_>> = value
                .iter()
                .filter(|value| !matches!(value, ComponentValue::Token(Token::Whitespace)))
                .collect();
            match arguments.as_slice() {
                [ComponentValue::Token(Token::String(url))] => url.clone(),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some((prefix, url))
}

/// Die Namen einer `@layer`-Regel: ⟨layer-name⟩, durch Kommas getrennt.
/// `None`, wenn einer ungültig ist; ein leeres Prelude ergibt keine Namen.
fn layer_names(prelude: &ComponentValues) -> Option<Vec<LayerName>> {
    let values = trimmed(prelude);
    if values.is_empty() {
        return Some(Vec::new());
    }
    values
        .split(|value| matches!(value, ComponentValue::Token(Token::Comma)))
        .map(layer_name)
        .collect()
}

/// ⟨layer-name⟩ = ⟨ident⟩ [ '.' ⟨ident⟩ ]*, ohne Leerraum dazwischen. Die
/// CSS-weiten Schlüsselwörter sind als Namen ungültig.
fn layer_name(values: &[ComponentValue<'_>]) -> Option<LayerName> {
    let is_space =
        |value: &ComponentValue<'_>| matches!(value, ComponentValue::Token(Token::Whitespace));
    let start = values.iter().position(|value| !is_space(value))?;
    let end = values.iter().rposition(|value| !is_space(value))?;
    let values = values.get(start..=end)?;
    let mut segments = Vec::new();
    let mut expect_ident = true;
    for value in values {
        match (expect_ident, value) {
            (true, ComponentValue::Token(Token::Ident(ident))) => {
                if is_css_wide_keyword(ident) {
                    return None;
                }
                segments.push(ident.clone());
                expect_ident = false;
            }
            (false, ComponentValue::Token(Token::Delim('.'))) => expect_ident = true,
            _ => return None,
        }
    }
    (!expect_ident).then_some(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(text: &str) -> StyleSheet {
        StyleSheet::parse(text, ParseContext::default())
    }

    /// Eine kurze Beschreibung der Regeln, zum Vergleichen.
    fn describe(rules: &[CssRule]) -> String {
        let mut parts = Vec::new();
        for rule in rules {
            parts.push(match rule {
                CssRule::Style(rule) => format!(
                    "style({}; {})",
                    rule.declarations.len(),
                    describe(&rule.rules)
                ),
                CssRule::LayerBlock { name, rules } => format!(
                    "layer {}({})",
                    name.as_ref().map_or("-".to_owned(), |name| name.join(".")),
                    describe(rules)
                ),
                CssRule::LayerStatement(names) => format!(
                    "layers {}",
                    names
                        .iter()
                        .map(|name| name.join("."))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                CssRule::NestedDeclarations(declarations) => {
                    format!("decls({})", declarations.len())
                }
                CssRule::Media { rules, .. } => format!("media({})", describe(rules)),
                CssRule::Supports { matches, rules } => {
                    format!("supports {matches}({})", describe(rules))
                }
            });
        }
        parts.join(" ")
    }

    #[test]
    fn nesting() {
        // Deklarationen nach einer Regel werden eine eigene Regel, auch in
        // einer verschachtelten @layer-Regel; die Kurzschreibweise ergibt
        // vier Langformen.
        let sheet = sheet(
            ".a { color: red; & b { color: blue } margin: 0; \
             @layer x { color: green; c { color: red } } }",
        );
        assert_eq!(
            describe(&sheet.rules),
            "style(1; style(1; ) decls(4) layer x(decls(1) style(1; )))"
        );
        // Ungültige Deklarationen werden als Regel gelesen und fallen weg.
        let sheet = super::tests::sheet(".a { colour: red; color: blue; foo:bar { } }");
        assert_eq!(describe(&sheet.rules), "style(1; )");
        // Ganz oben gibt es keine Deklarationen.
        let sheet = super::tests::sheet("@layer { color: red; a { color: blue } }");
        assert_eq!(describe(&sheet.rules), "layer -(style(1; ))");
    }

    #[test]
    fn layer_rules() {
        let sheet = sheet(
            "@layer a, b.c; @layer { } @layer d.e { } @layer f g {} @layer a. {} \
             @layer initial {} @layer a, {} @layer x;",
        );
        assert_eq!(
            describe(&sheet.rules),
            "layers a,b.c layer -() layer d.e() layers x"
        );
    }

    #[test]
    fn namespaces_come_first() {
        let rules = |text: &str| {
            let sheet = sheet(text);
            sheet.rules.len()
        };
        // Ein Präfix macht Selektoren mit `svg|` gültig.
        assert_eq!(
            rules("@namespace svg url(http://www.w3.org/2000/svg); svg|a {}"),
            1
        );
        assert_eq!(rules("@namespace svg 'x'; svg|a {}"), 1);
        assert_eq!(rules("svg|a {}"), 0);
        // Nach einer Regel ist @namespace ungültig, davor stört eine
        // ungültige Regel nicht.
        assert_eq!(rules("a {} @namespace svg 'x'; svg|a {}"), 1);
        assert_eq!(rules("1a {} @namespace svg 'x'; svg|a {}"), 1);
        // @layer-Anweisungen dürfen vor @import und @namespace stehen,
        // aber nicht dazwischen.
        assert_eq!(rules("@layer x; @namespace svg 'x'; svg|a {}"), 2);
        assert_eq!(
            rules("@import 'a.css'; @layer x; @namespace svg 'x'; svg|a {}"),
            1
        );
        assert_eq!(rules("@namespace svg; svg|a {}"), 0);
        assert_eq!(rules("@namespace svg 'x' 'y'; svg|a {}"), 0);
    }

    #[test]
    fn conditional_group_rules() {
        let sheet = sheet(
            "@media screen { a {} @layer x { b {} } } @media; @media {} \
             @supports (color: red) { c {} } @supports (colour: red) { d {} } \
             @supports foo { e {} } @supports (a: b) and (c: d) or (e: f) { f {} } \
             .a { @media print { color: red; } @supports not (color: red) { g {} } }",
        );
        assert_eq!(
            describe(&sheet.rules),
            "media(style(0; ) layer x(style(0; ))) media() supports true(style(0; )) \
             supports false(style(0; )) style(0; media(decls(1)) supports false(style(0; )))"
        );
    }

    /// Die `@import`-Regeln: Ebene, ob `supports()` erfüllt ist und ob die
    /// Media Queries auf dem Gerät für Tests zutreffen.
    fn imports(text: &str) -> Vec<String> {
        use crate::computed::{NoFonts, test_device};
        sheet(text)
            .imports
            .iter()
            .map(|import| {
                let layer = match &import.layer {
                    None => "-".to_owned(),
                    Some(None) => "anonym".to_owned(),
                    Some(Some(name)) => name.join("."),
                };
                let media = import.media.matches(&test_device(), &NoFonts);
                format!("{layer} {} {media}", import.supports)
            })
            .collect()
    }

    #[test]
    fn import_rules() {
        assert_eq!(
            imports(
                "@import 'a' LAYER; @import url('b') layer(x.y) supports(color: red) screen; \
                 @import url(c) supports(colour: red) print; @import 'd' supports((color: red)); \
                 @import src('e') supports(color: red !important) (min-width: 1px);"
            ),
            [
                "anonym true true",
                "x.y true true",
                "- false false",
                "- true true",
                "- true true",
            ]
        );
        // Ein ungültiges `layer()` oder `supports()` gehört zu den Media
        // Queries und trifft nie zu; `layer` nach `supports()` auch nicht.
        assert_eq!(
            imports(
                "@import 'a' layer(1); @import 'b' supports(foo bar); \
                 @import 'c' supports(color: red) layer; @import 'd' layer screen;"
            ),
            [
                "- true false",
                "- true false",
                "- true false",
                "anonym true true"
            ]
        );
        // Nur vor allen anderen Regeln außer `@layer`-Anweisungen, und ohne
        // Regeln dazwischen.
        assert!(imports("a {} @import 'x';").is_empty());
        assert_eq!(imports("@layer a; @import 'x'; @import 'y';").len(), 2);
        assert_eq!(imports("@import 'x'; @layer a; @import 'y';").len(), 1);
        assert_eq!(
            imports("@import 'x'; @namespace y 'z'; @import 'y';").len(),
            1
        );
        assert!(imports("@import; @import x; @import 'x' {}").is_empty());
        let sheet = sheet("@layer a, b; @layer c; @import 'x'; d {}");
        assert_eq!(sheet.import_position, 2);
    }

    #[test]
    fn pending_imports() {
        let base = Url::parse("http://example.test/css/main.css", None).expect("URL");
        let context = ParseContext {
            base_url: Some(&base),
            quirks_mode: false,
        };
        let mut sheet = StyleSheet::parse(
            "@import 'a.css'; @import url(../b.css) print; @import 'c.css' supports(colour: red); \
             @import ''; @import 'http://[';",
            context,
        );
        let urls: Vec<String> = sheet
            .pending_imports()
            .iter()
            .map(|pending| pending.url().href())
            .collect();
        assert_eq!(
            urls,
            ["http://example.test/css/a.css", "http://example.test/b.css"]
        );
        for pending in sheet.pending_imports() {
            pending.load(StyleSheet::default());
        }
        assert!(sheet.pending_imports().is_empty());
        assert_eq!(sheet.imports.len(), 5);
    }
}
