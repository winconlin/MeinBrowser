//! Selektoren nach Selectors Level 4 (Editor's Draft, Stand 23.10.2026):
//! Datenmodell, Parsen und Spezifität. Der Abgleich steht in `matching.rs`,
//! die Sprachbereiche für `:lang()` in `lang.rs`.
//!
//! Unterstützt sind Typ-, Universal-, Klassen-, ID- und Attributselektoren
//! mit Namensräumen, alle Kombinatoren, die Pseudoklassen `:is()`,
//! `:where()`, `:not()`, `:has()`, die baumstrukturellen, `:any-link`,
//! `:link`, `:visited`, `:target`, `:scope`, die Nutzeraktionen, `:lang()`,
//! `:enabled`, `:disabled` und `:checked`, dazu die Pseudoelemente
//! `::before`, `::after`, `::marker`, `::first-line`, `::first-letter` und
//! `::slotted()` sowie unbekannte `::-webkit-…` (Anhang B). Alles andere macht
//! einen Selektor ungültig, wie es die Spezifikation für Unbekanntes verlangt
//! („no usable level of support“). Pseudoelemente lassen sich nicht weiter
//! verketten (`::before::marker`), und die Verschachtelung mit `&` aus CSS
//! Nesting kommt erst mit der Kaskade.

mod lang;
mod matching;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod wpt;

use std::collections::BTreeMap;

use mb_base::DomString;
use mb_css::{
    AnPlusB, BlockKind, ComponentValue, Source, Token, Values, parse_an_plus_b,
    parse_component_value_list,
};

pub use matching::MatchingContext;

/// Wie tief Selektoren in Pseudoklassen wie `:is()` verschachtelt sein
/// dürfen; tiefer verschachtelte sind ungültig. Parser und Abgleich steigen
/// dafür rekursiv ab, die Grenze schützt sie vor einem Stapelüberlauf.
const MAX_NESTING: usize = 100;

/// Die Namensraum-Präfixe eines Stylesheets aus seinen `@namespace`-Regeln
/// (CSS Namespaces). Präfixe unterscheiden Groß- und Kleinschreibung; der
/// leere Namensraum steht für „kein Namensraum“.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Namespaces {
    default: Option<NamespaceConstraint>,
    prefixes: BTreeMap<String, NamespaceConstraint>,
}

impl Namespaces {
    /// Erklärt einen Namensraum, ohne Präfix den Standard-Namensraum. Eine
    /// spätere Erklärung ersetzt eine frühere mit demselben Präfix.
    pub fn declare(&mut self, prefix: Option<&str>, url: &str) {
        let namespace = if url.is_empty() {
            NamespaceConstraint::None
        } else {
            NamespaceConstraint::Url(url.to_owned())
        };
        match prefix {
            Some(prefix) => {
                self.prefixes.insert(prefix.to_owned(), namespace);
            }
            None => self.default = Some(namespace),
        }
    }
}

/// Der Namensraum, den ein Selektor verlangt.
#[derive(Clone, Debug, PartialEq, Eq)]
enum NamespaceConstraint {
    /// `*|`
    Any,
    /// `|` oder ein Präfix für den leeren Namensraum
    None,
    Url(String),
}

/// Ein Name aus einem Selektor, dazu in ASCII-Kleinbuchstaben für
/// HTML-Elemente in HTML-Dokumenten.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Name {
    original: DomString,
    lower: DomString,
}

impl Name {
    fn new(name: &str) -> Name {
        Name {
            original: DomString::from(name),
            lower: DomString::from(name.to_ascii_lowercase().as_str()),
        }
    }
}

/// Die Spezifität (A, B, C): IDs, dann Klassen, Attribute und Pseudoklassen,
/// dann Typen und Pseudoelemente. Die Reihenfolge der Felder ergibt den
/// Vergleich; zu große Werte bleiben beim Höchstwert stehen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Specificity {
    pub ids: u32,
    pub classes: u32,
    pub types: u32,
}

impl Specificity {
    const ID: Specificity = Specificity {
        ids: 1,
        classes: 0,
        types: 0,
    };
    const CLASS: Specificity = Specificity {
        ids: 0,
        classes: 1,
        types: 0,
    };
    const TYPE: Specificity = Specificity {
        ids: 0,
        classes: 0,
        types: 1,
    };

    fn add(self, other: Specificity) -> Specificity {
        Specificity {
            ids: self.ids.saturating_add(other.ids),
            classes: self.classes.saturating_add(other.classes),
            types: self.types.saturating_add(other.types),
        }
    }
}

/// Ein ungültiger Selektor (*invalid selector*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidSelector;

impl std::fmt::Display for InvalidSelector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ungültiger Selektor")
    }
}

impl std::error::Error for InvalidSelector {}

/// Eine Liste komplexer Selektoren (*selector list*).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelectorList {
    selectors: Vec<Selector>,
}

/// Ein komplexer Selektor (*complex selector*), höchstens mit einem
/// Pseudoelement am Ende.
#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    /// Die Verbundselektoren von links nach rechts; der letzte trifft das
    /// Subjekt.
    compounds: Vec<Compound>,
    /// `combinators[i]` steht zwischen `compounds[i]` und `compounds[i + 1]`.
    combinators: Vec<Combinator>,
    pseudo_element: Option<PseudoElementSelector>,
    specificity: Specificity,
}

/// Ein Verbundselektor (*compound selector*); leer steht er für `*`.
type Compound = Vec<Simple>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

/// Ein einfacher Selektor (*simple selector*).
#[derive(Clone, Debug, PartialEq)]
enum Simple {
    /// Typselektor, ohne Namen der Universalselektor `*`.
    Type {
        namespace: NamespaceConstraint,
        name: Option<Name>,
    },
    Id(DomString),
    Class(DomString),
    Attribute(Box<AttributeSelector>),
    PseudoClass(PseudoClass),
    /// Am Anfang eines relativen Selektors in `:has()`: das Element, an dem
    /// er verankert ist (*relative selector anchor element*).
    Anchor,
}

#[derive(Clone, Debug, PartialEq)]
struct AttributeSelector {
    namespace: NamespaceConstraint,
    name: Name,
    /// Ohne Wert prüft der Selektor nur, ob das Attribut da ist.
    value: Option<AttributeValue>,
}

#[derive(Clone, Debug, PartialEq)]
struct AttributeValue {
    matcher: AttributeMatcher,
    value: DomString,
    case: AttributeCase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttributeMatcher {
    /// `=`
    Exact,
    /// `~=`
    Word,
    /// `|=`
    Dash,
    /// `^=`
    Prefix,
    /// `$=`
    Suffix,
    /// `*=`
    Substring,
}

/// Wie Werte verglichen werden: nach den Regeln des Dokuments, mit `i`
/// ohne Unterschied zwischen ASCII-Groß- und -Kleinbuchstaben, mit `s`
/// genau.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttributeCase {
    Document,
    Insensitive,
    Sensitive,
}

#[derive(Clone, Debug, PartialEq)]
enum PseudoClass {
    Is(SelectorList),
    Where(SelectorList),
    Not(SelectorList),
    /// Die relativen Selektoren beginnen mit [`Simple::Anchor`].
    Has(SelectorList),
    Root,
    Empty,
    /// `:nth-child()`, mit `last` `:nth-last-child()`; auch `:first-child`
    /// und `:last-child`, die dasselbe wie `:nth-child(1)` bedeuten.
    NthChild {
        formula: AnPlusB,
        of: Option<SelectorList>,
        last: bool,
    },
    /// `:nth-of-type()` und `:nth-last-of-type()`, auch `:first-of-type`
    /// und `:last-of-type`.
    NthOfType {
        formula: AnPlusB,
        last: bool,
    },
    OnlyChild,
    OnlyOfType,
    AnyLink,
    Link,
    Visited,
    Target,
    Scope,
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Lang(Vec<DomString>),
    Enabled,
    Disabled,
    Checked,
}

/// Die Pseudoelemente, die Boxen erzeugen und für die die Kaskade Stile
/// berechnet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PseudoElement {
    Before,
    After,
    Marker,
    FirstLine,
    FirstLetter,
}

#[derive(Clone, Debug, PartialEq)]
struct PseudoElementSelector {
    target: PseudoTarget,
    /// Pseudoklassen dahinter; erlaubt sind die Nutzeraktionen und logische
    /// Kombinationen davon.
    states: Vec<Simple>,
}

#[derive(Clone, Debug, PartialEq)]
enum PseudoTarget {
    Element(PseudoElement),
    /// `::slotted()`. Ohne Shadow Trees gibt es keine Slots, also passt es
    /// auf nichts.
    Slotted(Compound),
    /// Ein unbekanntes `::-webkit-…`: gültig, passt aber auf nichts.
    UnknownWebkit,
}

impl SelectorList {
    /// *Parse a selector*: liest eine ⟨selector-list⟩ aus Text, etwa für
    /// `querySelectorAll()`.
    pub fn parse(input: &Source, namespaces: &Namespaces) -> Result<SelectorList, InvalidSelector> {
        let values = parse_component_value_list(input);
        SelectorList::from_values(values.values(), namespaces)
    }

    /// Liest eine ⟨selector-list⟩ aus Komponentenwerten, etwa aus dem
    /// Prelude einer Stilregel.
    pub fn from_values(
        values: Values<'_>,
        namespaces: &Namespaces,
    ) -> Result<SelectorList, InvalidSelector> {
        let values: Vec<ComponentValue<'_>> = values.iter().collect();
        let parser = Parser { namespaces };
        let rules = Rules {
            pseudo_elements: true,
            in_has: false,
            subject_namespace: true,
            pseudo_states: false,
            depth: 0,
        };
        parser.list(&values, rules).ok_or(InvalidSelector)
    }

    pub fn selectors(&self) -> &[Selector] {
        &self.selectors
    }

    /// Die größte Spezifität der Selektoren, für `:is()` und Verwandte.
    fn max_specificity(&self) -> Specificity {
        self.selectors
            .iter()
            .map(|selector| selector.specificity)
            .max()
            .unwrap_or_default()
    }
}

impl Selector {
    pub fn specificity(&self) -> Specificity {
        self.specificity
    }

    /// Das Pseudoelement, für das der Selektor gilt, falls er für eines gilt,
    /// das Boxen erzeugt.
    pub fn pseudo_element(&self) -> Option<PseudoElement> {
        match self.pseudo_element.as_ref()?.target {
            PseudoTarget::Element(element) => Some(element),
            _ => None,
        }
    }
}

/// Einschränkungen, die an einer Stelle gelten und an Argumente
/// weitergegeben werden.
#[derive(Clone, Copy)]
struct Rules {
    /// Pseudoelemente sind erlaubt; sonst nur ⟨complex-real-selector⟩s.
    pseudo_elements: bool,
    /// In `:has()`, das sich nicht verschachteln lässt.
    in_has: bool,
    /// Ob der Standard-Namensraum für ein Subjekt ohne Typselektor gilt. In
    /// `:is()`, `:where()` und `:not()` gilt er nicht.
    subject_namespace: bool,
    /// Hinter einem Pseudoelement: nur Nutzeraktionen und logische
    /// Kombinationen davon, ohne Kombinatoren.
    pseudo_states: bool,
    /// Wie tief die Argumente verschachtelt sind.
    depth: usize,
}

impl Rules {
    /// Die Einschränkungen für die Argumente einer Pseudoklasse.
    fn nested(self) -> Option<Rules> {
        let depth = self
            .depth
            .checked_add(1)
            .filter(|&depth| depth <= MAX_NESTING)?;
        Some(Rules {
            pseudo_elements: false,
            depth,
            ..self
        })
    }
}

struct Parser<'n> {
    namespaces: &'n Namespaces,
}

/// Das Token an einer Stelle, falls dort ein Token steht.
fn token<'a>(input: &[ComponentValue<'a>], index: usize) -> Option<&'a Token> {
    match input.get(index)? {
        ComponentValue::Token(token) => Some(*token),
        _ => None,
    }
}

fn is_whitespace(value: &ComponentValue<'_>) -> bool {
    matches!(value, ComponentValue::Token(Token::Whitespace))
}

/// Ohne Leerraum am Anfang und am Ende.
fn trim<'v, 'a>(input: &'v [ComponentValue<'a>]) -> &'v [ComponentValue<'a>] {
    let start = input
        .iter()
        .position(|value| !is_whitespace(value))
        .unwrap_or(input.len());
    let end = input
        .iter()
        .rposition(|value| !is_whitespace(value))
        .map_or(start, |end| end.saturating_add(1));
    input.get(start..end).unwrap_or_default()
}

/// Überspringt Leerraum und sagt, ob welcher da war.
fn skip_whitespace(input: &[ComponentValue<'_>], position: &mut usize) -> bool {
    let start = *position;
    while input.get(*position).is_some_and(is_whitespace) {
        *position = position.saturating_add(1);
    }
    *position != start
}

fn split_commas<'v, 'a>(input: &'v [ComponentValue<'a>]) -> Vec<&'v [ComponentValue<'a>]> {
    input
        .split(|value| matches!(value, ComponentValue::Token(Token::Comma)))
        .collect()
}

fn combinator(token: Option<&Token>) -> Option<Combinator> {
    match token? {
        Token::Delim('>') => Some(Combinator::Child),
        Token::Delim('+') => Some(Combinator::NextSibling),
        Token::Delim('~') => Some(Combinator::SubsequentSibling),
        _ => None,
    }
}

/// Die Pseudoelemente aus CSS Level 2, die auch mit einem Doppelpunkt
/// geschrieben werden dürfen.
fn legacy_pseudo_element(name: &str) -> Option<PseudoElement> {
    match name.to_ascii_lowercase().as_str() {
        "before" => Some(PseudoElement::Before),
        "after" => Some(PseudoElement::After),
        "first-line" => Some(PseudoElement::FirstLine),
        "first-letter" => Some(PseudoElement::FirstLetter),
        _ => None,
    }
}

/// Ob an der Stelle ein Pseudoelement beginnt.
fn starts_pseudo_element(input: &[ComponentValue<'_>], position: usize) -> bool {
    if token(input, position) != Some(&Token::Colon) {
        return false;
    }
    match token(input, position.saturating_add(1)) {
        Some(Token::Colon) => true,
        Some(Token::Ident(name)) => legacy_pseudo_element(name).is_some(),
        _ => false,
    }
}

impl Parser<'_> {
    /// Ein Präfix aus `@namespace`; ein unbekanntes macht den Selektor
    /// ungültig.
    fn prefix(&self, prefix: &str) -> Option<NamespaceConstraint> {
        self.namespaces.prefixes.get(prefix).cloned()
    }

    /// Der Namensraum eines Typselektors ohne Präfix.
    fn default_namespace(&self) -> NamespaceConstraint {
        self.namespaces
            .default
            .clone()
            .unwrap_or(NamespaceConstraint::Any)
    }

    /// Eine Liste, in der jeder Selektor gültig sein muss.
    fn list(&self, input: &[ComponentValue<'_>], rules: Rules) -> Option<SelectorList> {
        let selectors = split_commas(input)
            .into_iter()
            .map(|part| self.complex(part, rules, false))
            .collect::<Option<Vec<Selector>>>()?;
        Some(SelectorList { selectors })
    }

    /// *Parse as a forgiving selector list*: Ungültige Selektoren fallen
    /// weg, die Liste darf leer werden.
    fn forgiving_list(&self, input: &[ComponentValue<'_>], rules: Rules) -> SelectorList {
        let selectors = split_commas(input)
            .into_iter()
            .filter_map(|part| self.complex(part, rules, false))
            .collect();
        SelectorList { selectors }
    }

    /// ⟨relative-selector-list⟩ für `:has()`
    fn relative_list(&self, input: &[ComponentValue<'_>], rules: Rules) -> Option<SelectorList> {
        let selectors = split_commas(input)
            .into_iter()
            .map(|part| self.complex(part, rules, true))
            .collect::<Option<Vec<Selector>>>()?;
        Some(SelectorList { selectors })
    }

    /// Ein komplexer Selektor, mit `relative` ein relativer.
    fn complex(
        &self,
        input: &[ComponentValue<'_>],
        rules: Rules,
        relative: bool,
    ) -> Option<Selector> {
        let input = trim(input);
        let mut position = 0;
        let mut compounds = Vec::new();
        let mut combinators = Vec::new();
        // Ob ein Verbundselektor einen Typ- oder Universalselektor enthält.
        let mut explicit = Vec::new();
        if relative {
            let first = match combinator(token(input, position)) {
                Some(first) => {
                    position = position.saturating_add(1);
                    skip_whitespace(input, &mut position);
                    first
                }
                None => Combinator::Descendant,
            };
            compounds.push(vec![Simple::Anchor]);
            explicit.push(true);
            combinators.push(first);
        }
        let mut pseudo_element = None;
        loop {
            let (compound, has_type) = self.compound(input, &mut position, rules)?;
            if starts_pseudo_element(input, position) {
                if !rules.pseudo_elements {
                    return None;
                }
                pseudo_element = Some(self.pseudo_element(input, &mut position, rules)?);
            }
            if compound.is_empty() && pseudo_element.is_none() {
                return None;
            }
            compounds.push(compound);
            explicit.push(has_type);
            let space = skip_whitespace(input, &mut position);
            if position >= input.len() {
                break;
            }
            // Hinter einem Pseudoelement und in seinen Zuständen gibt es
            // keine Kombinatoren.
            if pseudo_element.is_some() || rules.pseudo_states {
                return None;
            }
            let next = match combinator(token(input, position)) {
                Some(next) => {
                    position = position.saturating_add(1);
                    skip_whitespace(input, &mut position);
                    next
                }
                None if space => Combinator::Descendant,
                None => return None,
            };
            combinators.push(next);
        }
        // Ein Standard-Namensraum gilt auch für Verbundselektoren ohne
        // Typselektor, außer für das Subjekt in `:is()` und Verwandten.
        if let Some(default) = &self.namespaces.default
            && !rules.pseudo_states
        {
            let subject = compounds.len().saturating_sub(1);
            for (index, compound) in compounds.iter_mut().enumerate() {
                let exempt = index == subject && !rules.subject_namespace;
                if !explicit.get(index).copied().unwrap_or(true) && !exempt {
                    compound.insert(
                        0,
                        Simple::Type {
                            namespace: default.clone(),
                            name: None,
                        },
                    );
                }
            }
        }
        let mut simples: Vec<&Simple> = compounds.iter().flatten().collect();
        if let Some(pseudo) = &pseudo_element {
            if let PseudoTarget::Slotted(argument) = &pseudo.target {
                simples.extend(argument);
            }
            simples.extend(&pseudo.states);
        }
        let specificity = simples
            .into_iter()
            .map(Simple::specificity)
            .fold(Specificity::default(), Specificity::add)
            .add(if pseudo_element.is_some() {
                Specificity::TYPE
            } else {
                Specificity::default()
            });
        Some(Selector {
            compounds,
            combinators,
            pseudo_element,
            specificity,
        })
    }

    /// Ein Verbundselektor ohne Pseudoelement, dazu ob er einen Typ- oder
    /// Universalselektor hat. Er endet vor Leerraum, Kombinatoren und
    /// Pseudoelementen und darf leer sein.
    fn compound(
        &self,
        input: &[ComponentValue<'_>],
        position: &mut usize,
        rules: Rules,
    ) -> Option<(Compound, bool)> {
        let mut compound = Vec::new();
        let type_selector = self.type_selector(input, position)?;
        let has_type = type_selector.is_some();
        if let Some(simple) = type_selector {
            if rules.pseudo_states {
                return None;
            }
            compound.push(simple);
        }
        loop {
            let next = position.saturating_add(1);
            let simple = match input.get(*position) {
                Some(ComponentValue::Token(Token::Hash { value, id })) => {
                    // Der Wert muss ein Bezeichner sein.
                    if !id {
                        return None;
                    }
                    *position = next;
                    Simple::Id(DomString::from(value.as_str()))
                }
                Some(ComponentValue::Token(Token::Delim('.'))) => {
                    let Some(Token::Ident(name)) = token(input, next) else {
                        return None;
                    };
                    *position = next.saturating_add(1);
                    Simple::Class(DomString::from(name.as_str()))
                }
                Some(ComponentValue::Block {
                    kind: BlockKind::Bracket,
                    value,
                }) => {
                    *position = next;
                    Simple::Attribute(Box::new(self.attribute(*value)?))
                }
                Some(ComponentValue::Token(Token::Colon)) => {
                    if starts_pseudo_element(input, *position) {
                        break;
                    }
                    let class = match input.get(next) {
                        Some(ComponentValue::Token(Token::Ident(name))) => {
                            self.pseudo_class(name, rules)?
                        }
                        Some(ComponentValue::Function { name, value }) => {
                            self.functional_pseudo_class(name, *value, rules)?
                        }
                        _ => return None,
                    };
                    *position = next.saturating_add(1);
                    Simple::PseudoClass(class)
                }
                _ => break,
            };
            if rules.pseudo_states && !matches!(simple, Simple::PseudoClass(_)) {
                return None;
            }
            compound.push(simple);
        }
        Some((compound, has_type))
    }

    /// ⟨type-selector⟩, falls einer da ist: `E`, `ns|E`, `*|E`, `|E` oder
    /// dasselbe mit `*` statt `E`. `None` heißt ungültig.
    fn type_selector(
        &self,
        input: &[ComponentValue<'_>],
        position: &mut usize,
    ) -> Option<Option<Simple>> {
        let at = |offset: usize| token(input, position.saturating_add(offset));
        // Der lokale Name nach `|`: ein Bezeichner oder `*`.
        let local = |offset: usize| match at(offset) {
            Some(Token::Ident(name)) => Some(Some(Name::new(name))),
            Some(Token::Delim('*')) => Some(None),
            _ => None,
        };
        let (namespace, name, length) = match (at(0), at(1)) {
            (Some(Token::Delim('|')), _) => (NamespaceConstraint::None, local(1)?, 2),
            (Some(Token::Delim('*')), Some(Token::Delim('|'))) => {
                (NamespaceConstraint::Any, local(2)?, 3)
            }
            (Some(Token::Ident(prefix)), Some(Token::Delim('|'))) => {
                (self.prefix(prefix)?, local(2)?, 3)
            }
            (Some(Token::Delim('*')), _) => (self.default_namespace(), None, 1),
            (Some(Token::Ident(name)), _) => (self.default_namespace(), Some(Name::new(name)), 1),
            _ => return Some(None),
        };
        *position = position.saturating_add(length);
        Some(Some(Simple::Type { namespace, name }))
    }

    /// Der Inhalt eines Attributselektors `[…]`.
    fn attribute(&self, value: Values<'_>) -> Option<AttributeSelector> {
        let values: Vec<ComponentValue<'_>> = value.iter().collect();
        let input = trim(&values);
        let at = |index: usize| token(input, index);
        let (namespace, name, mut position) = match (at(0), at(1), at(2)) {
            (Some(Token::Delim('|')), Some(Token::Ident(name)), _) => {
                (NamespaceConstraint::None, name, 2)
            }
            (Some(Token::Delim('*')), Some(Token::Delim('|')), Some(Token::Ident(name))) => {
                (NamespaceConstraint::Any, name, 3)
            }
            (Some(Token::Ident(prefix)), Some(Token::Delim('|')), Some(Token::Ident(name))) => {
                (self.prefix(prefix)?, name, 3)
            }
            // Ohne Präfix: Für Attribute gilt kein Standard-Namensraum.
            (Some(Token::Ident(name)), _, _) => (NamespaceConstraint::None, name, 1),
            _ => return None,
        };
        let name = Name::new(name);
        skip_whitespace(input, &mut position);
        if position >= input.len() {
            return Some(AttributeSelector {
                namespace,
                name,
                value: None,
            });
        }
        let next = position.saturating_add(1);
        let matcher = match (at(position), at(next)) {
            (Some(Token::Delim('=')), _) => AttributeMatcher::Exact,
            (Some(Token::Delim(prefix)), Some(Token::Delim('='))) => {
                position = next;
                match prefix {
                    '~' => AttributeMatcher::Word,
                    '|' => AttributeMatcher::Dash,
                    '^' => AttributeMatcher::Prefix,
                    '$' => AttributeMatcher::Suffix,
                    '*' => AttributeMatcher::Substring,
                    _ => return None,
                }
            }
            _ => return None,
        };
        position = position.saturating_add(1);
        skip_whitespace(input, &mut position);
        let value = match at(position)? {
            Token::Ident(value) | Token::String(value) => DomString::from(value.as_str()),
            _ => return None,
        };
        position = position.saturating_add(1);
        skip_whitespace(input, &mut position);
        let mut case = AttributeCase::Document;
        if let Some(Token::Ident(modifier)) = at(position) {
            case = match modifier.to_ascii_lowercase().as_str() {
                "i" => AttributeCase::Insensitive,
                "s" => AttributeCase::Sensitive,
                _ => return None,
            };
            position = position.saturating_add(1);
            skip_whitespace(input, &mut position);
        }
        if position < input.len() {
            return None;
        }
        Some(AttributeSelector {
            namespace,
            name,
            value: Some(AttributeValue {
                matcher,
                value,
                case,
            }),
        })
    }

    /// Eine Pseudoklasse ohne Argumente.
    fn pseudo_class(&self, name: &str, rules: Rules) -> Option<PseudoClass> {
        let first = AnPlusB { a: 0, b: 1 };
        let class = match name.to_ascii_lowercase().as_str() {
            "hover" => PseudoClass::Hover,
            "active" => PseudoClass::Active,
            "focus" => PseudoClass::Focus,
            "focus-visible" => PseudoClass::FocusVisible,
            "focus-within" => PseudoClass::FocusWithin,
            _ if rules.pseudo_states => return None,
            "root" => PseudoClass::Root,
            "empty" => PseudoClass::Empty,
            "first-child" => PseudoClass::NthChild {
                formula: first,
                of: None,
                last: false,
            },
            "last-child" => PseudoClass::NthChild {
                formula: first,
                of: None,
                last: true,
            },
            "only-child" => PseudoClass::OnlyChild,
            "first-of-type" => PseudoClass::NthOfType {
                formula: first,
                last: false,
            },
            "last-of-type" => PseudoClass::NthOfType {
                formula: first,
                last: true,
            },
            "only-of-type" => PseudoClass::OnlyOfType,
            "any-link" => PseudoClass::AnyLink,
            "link" => PseudoClass::Link,
            "visited" => PseudoClass::Visited,
            "target" => PseudoClass::Target,
            "scope" => PseudoClass::Scope,
            "enabled" => PseudoClass::Enabled,
            "disabled" => PseudoClass::Disabled,
            "checked" => PseudoClass::Checked,
            _ => return None,
        };
        Some(class)
    }

    /// Eine funktionale Pseudoklasse.
    fn functional_pseudo_class(
        &self,
        name: &str,
        value: Values<'_>,
        rules: Rules,
    ) -> Option<PseudoClass> {
        let values: Vec<ComponentValue<'_>> = value.iter().collect();
        let inner = rules.nested()?;
        let name = name.to_ascii_lowercase();
        // Logische Kombinationen geben die Einschränkungen weiter.
        let logical = Rules {
            subject_namespace: false,
            ..inner
        };
        match name.as_str() {
            "is" => return Some(PseudoClass::Is(self.forgiving_list(&values, logical))),
            "where" => return Some(PseudoClass::Where(self.forgiving_list(&values, logical))),
            "not" => return Some(PseudoClass::Not(self.list(&values, logical)?)),
            _ if rules.pseudo_states => return None,
            _ => {}
        }
        let real = Rules {
            subject_namespace: true,
            ..inner
        };
        match name.as_str() {
            "has" if !rules.in_has => {
                let has = Rules {
                    in_has: true,
                    ..real
                };
                Some(PseudoClass::Has(self.relative_list(&values, has)?))
            }
            "nth-child" | "nth-last-child" => {
                // „An+B [of S]?“: Ein Bezeichner „of“ kommt in An+B nie vor.
                let of = values.iter().position(|value| {
                    matches!(value, ComponentValue::Token(Token::Ident(ident))
                        if ident.eq_ignore_ascii_case("of"))
                });
                let (formula, of) = match of {
                    Some(index) => {
                        let formula = values.get(..index)?;
                        let selectors = values.get(index.saturating_add(1)..)?;
                        (formula, Some(self.list(selectors, real)?))
                    }
                    None => (values.as_slice(), None),
                };
                Some(PseudoClass::NthChild {
                    formula: parse_an_plus_b(formula)?,
                    of,
                    last: name == "nth-last-child",
                })
            }
            "nth-of-type" | "nth-last-of-type" => Some(PseudoClass::NthOfType {
                formula: parse_an_plus_b(&values)?,
                last: name == "nth-last-of-type",
            }),
            "lang" => {
                let ranges = split_commas(&values)
                    .into_iter()
                    .map(|part| match trim(part) {
                        [ComponentValue::Token(Token::Ident(range) | Token::String(range))] => {
                            Some(DomString::from(range.as_str()))
                        }
                        _ => None,
                    })
                    .collect::<Option<Vec<DomString>>>()?;
                Some(PseudoClass::Lang(ranges))
            }
            _ => None,
        }
    }

    /// Ein Pseudoelement mit den Pseudoklassen dahinter; `position` steht
    /// auf dem ersten Doppelpunkt.
    fn pseudo_element(
        &self,
        input: &[ComponentValue<'_>],
        position: &mut usize,
        rules: Rules,
    ) -> Option<PseudoElementSelector> {
        let at = |offset: usize| input.get(position.saturating_add(offset));
        let (target, length) = match (at(1), at(2)) {
            (
                Some(ComponentValue::Token(Token::Colon)),
                Some(ComponentValue::Token(Token::Ident(name))),
            ) => {
                let lower = name.to_ascii_lowercase();
                let target = match lower.as_str() {
                    "marker" => PseudoTarget::Element(PseudoElement::Marker),
                    _ => match legacy_pseudo_element(&lower) {
                        Some(element) => PseudoTarget::Element(element),
                        None if lower.starts_with("-webkit-") => PseudoTarget::UnknownWebkit,
                        None => return None,
                    },
                };
                (target, 3)
            }
            (
                Some(ComponentValue::Token(Token::Colon)),
                Some(ComponentValue::Function { name, value }),
            ) if name.eq_ignore_ascii_case("slotted") => {
                // ⟨compound-selector⟩ als Argument
                let values: Vec<ComponentValue<'_>> = value.iter().collect();
                let argument = trim(&values);
                let mut inner = 0;
                let (compound, _) = self.compound(argument, &mut inner, rules.nested()?)?;
                if inner < argument.len() || compound.is_empty() {
                    return None;
                }
                (PseudoTarget::Slotted(compound), 3)
            }
            (Some(ComponentValue::Token(Token::Ident(name))), _) => {
                (PseudoTarget::Element(legacy_pseudo_element(name)?), 2)
            }
            _ => return None,
        };
        *position = position.saturating_add(length);
        let states = Rules {
            pseudo_states: true,
            ..rules
        };
        let mut classes = Vec::new();
        while token(input, *position) == Some(&Token::Colon) {
            let next = position.saturating_add(1);
            let class = match input.get(next) {
                Some(ComponentValue::Token(Token::Ident(name))) => {
                    self.pseudo_class(name, states)?
                }
                Some(ComponentValue::Function { name, value }) => {
                    self.functional_pseudo_class(name, *value, states)?
                }
                // Weitere Pseudoelemente sind nicht unterstützt.
                _ => return None,
            };
            classes.push(Simple::PseudoClass(class));
            *position = next.saturating_add(1);
        }
        Some(PseudoElementSelector {
            target,
            states: classes,
        })
    }
}

impl Simple {
    fn specificity(&self) -> Specificity {
        match self {
            Simple::Type { name: Some(_), .. } => Specificity::TYPE,
            Simple::Type { name: None, .. } | Simple::Anchor => Specificity::default(),
            Simple::Id(_) => Specificity::ID,
            Simple::Class(_) | Simple::Attribute(_) => Specificity::CLASS,
            Simple::PseudoClass(class) => match class {
                PseudoClass::Is(list) | PseudoClass::Not(list) | PseudoClass::Has(list) => {
                    list.max_specificity()
                }
                PseudoClass::Where(_) => Specificity::default(),
                PseudoClass::NthChild { of: Some(list), .. } => {
                    Specificity::CLASS.add(list.max_specificity())
                }
                _ => Specificity::CLASS,
            },
        }
    }
}
