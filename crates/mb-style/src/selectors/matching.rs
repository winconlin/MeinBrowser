//! Der Abgleich von Selektoren mit Elementen („Match a Selector Against an
//! Element“ und Verwandte).
//!
//! Komplexe Selektoren werden von rechts nach links abgeglichen, mit einem
//! eigenen Stapel statt Rekursion. Die Abkürzungen beim Zurückgehen stammen
//! aus WebKit: Scheitert etwa `a > b c` an einem Vorfahren für `b`, muss kein
//! weiterer Vorfahr für `c` mehr probiert werden.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use mb_base::DomString;
use mb_dom::{Document, Element, Namespace, NodeId, NodeKind, QuirksMode};

use super::{
    AttributeCase, AttributeMatcher, AttributeSelector, AttributeValue, Combinator, Compound, Name,
    NamespaceConstraint, PseudoClass, PseudoElement, PseudoTarget, Selector, SelectorList, Simple,
    lang,
};

/// Was der Abgleich über den Baum hinaus wissen muss.
#[derive(Clone, Copy)]
pub struct MatchingContext<'d> {
    pub document: &'d Document,
    /// Der *scoping root* für `:scope`. Ohne ihn, oder wenn er kein Element
    /// ist, steht `:scope` für die Wurzel des Baums. Ein Fragment als
    /// *virtual scoping root* fehlt noch.
    pub scoping_root: Option<NodeId>,
    /// Das Ziel des Dokuments (*target element*) für `:target`.
    pub target: Option<NodeId>,
    /// Die *pragma-set default language* oder die Sprache aus HTTP, für
    /// `:lang()`.
    pub default_language: Option<&'d DomString>,
}

impl<'d> MatchingContext<'d> {
    pub fn new(document: &'d Document) -> MatchingContext<'d> {
        MatchingContext {
            document,
            scoping_root: None,
            target: None,
            default_language: None,
        }
    }
}

impl SelectorList {
    /// *Match a selector against an element*: ob ein Selektor der Liste das
    /// Element trifft.
    pub fn matches(&self, element: NodeId, context: &MatchingContext<'_>) -> bool {
        self.selectors
            .iter()
            .any(|selector| selector.matches(element, context))
    }

    /// *Scope-match a selectors string* ohne das Parsen: die Nachkommen von
    /// `node` in Baumreihenfolge, die die Liste trifft, mit `node` als
    /// *scoping root*. So arbeiten `querySelectorAll()` und Verwandte.
    pub fn scope_match(&self, node: NodeId, context: &MatchingContext<'_>) -> Vec<NodeId> {
        let context = MatchingContext {
            scoping_root: Some(node),
            ..*context
        };
        let document = context.document;
        let mut found = Vec::new();
        let mut current = document.first_child(node);
        while let Some(candidate) = current {
            if self.matches(candidate, &context) {
                found.push(candidate);
            }
            current = document.following(candidate, node);
        }
        found
    }
}

impl Selector {
    /// Ob der Selektor das Element trifft. Selektoren für Pseudoelemente
    /// treffen nie ein Element.
    pub fn matches(&self, element: NodeId, context: &MatchingContext<'_>) -> bool {
        let nesting = NestingCache::default();
        self.pseudo_element.is_none() && Matcher::new(context, &nesting).complex(self, element)
    }

    /// *Match a selector against a pseudo-element*: ob der Selektor das
    /// Pseudoelement des Elements trifft.
    pub fn matches_pseudo_element(
        &self,
        element: NodeId,
        pseudo_element: PseudoElement,
        context: &MatchingContext<'_>,
    ) -> bool {
        let Some(selector) = &self.pseudo_element else {
            return false;
        };
        if selector.target != PseudoTarget::Element(pseudo_element) {
            return false;
        }
        let nesting = NestingCache::default();
        let matcher = Matcher::new(context, &nesting);
        // Hinter einem Pseudoelement stehen nur Nutzeraktionen und logische
        // Kombinationen davon. Sie hängen nicht vom Element ab, also darf
        // das Ursprungselement für das Pseudoelement einstehen.
        matcher.compound(&selector.states, element) && matcher.complex(self, element)
    }
}

/// Wie ein Versuch ausgeht, den Rest eines Selektors abzugleichen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Matched,
    /// Kein weiterer Kandidat kann passen.
    NotMatchedGlobally,
    /// Weiter beim nächsten Kandidaten des nächsten Geschwister-Kombinators
    /// rechts davon.
    RestartFromClosestLaterSibling,
    /// Weiter beim nächsten Kandidaten des nächsten Nachfahren-Kombinators
    /// rechts davon.
    RestartFromClosestDescendant,
}

/// Ein Eintrag des Stapels: Der Verbundselektor `index` passt; gesucht wird
/// ein Element für den links davon, und `candidate` ist das nächste, das
/// in Frage kommt.
struct Frame {
    index: usize,
    candidate: Option<NodeId>,
}

/// Was `&` ergab, je Liste (Adresse ihres `Arc`), Element und Anker von
/// `:has()`. `&&` gleicht die Liste darüber zweimal ab; ohne diesen Speicher
/// wüchse die Arbeit mit jeder Ebene der Verschachtelung auf das Doppelte.
type NestingCache = RefCell<BTreeMap<(usize, NodeId, Option<NodeId>), bool>>;

#[derive(Clone, Copy)]
struct Matcher<'c, 'd> {
    context: &'c MatchingContext<'d>,
    document: &'d Document,
    /// Das Element, an dem die relativen Selektoren von `:has()` hängen.
    anchor: Option<NodeId>,
    nesting: &'c NestingCache,
}

/// Leerraum in Selektoren und in Attributwerten wie `class`.
fn is_whitespace(unit: u16) -> bool {
    matches!(unit, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

/// Die Codeeinheiten eines Werts, auf Wunsch mit ASCII-Kleinbuchstaben.
fn code_units(value: &DomString, ignore_case: bool) -> Vec<u16> {
    value
        .code_units()
        .map(|unit| match u8::try_from(unit) {
            Ok(byte) if ignore_case => u16::from(byte.to_ascii_lowercase()),
            _ => unit,
        })
        .collect()
}

/// Die Attribute, deren Werte der HTML Standard in Attributselektoren ohne
/// Unterschied zwischen ASCII-Groß- und -Kleinbuchstaben vergleicht
/// („Case-sensitivity of selectors“).
const CASE_INSENSITIVE_ATTRIBUTES: [&str; 46] = [
    "accept",
    "accept-charset",
    "align",
    "alink",
    "axis",
    "bgcolor",
    "charset",
    "checked",
    "clear",
    "codetype",
    "color",
    "compact",
    "declare",
    "defer",
    "dir",
    "direction",
    "disabled",
    "enctype",
    "face",
    "frame",
    "hreflang",
    "http-equiv",
    "lang",
    "language",
    "link",
    "media",
    "method",
    "multiple",
    "nohref",
    "noresize",
    "noshade",
    "nowrap",
    "readonly",
    "rel",
    "rev",
    "rules",
    "scope",
    "scrolling",
    "selected",
    "shape",
    "target",
    "text",
    "type",
    "valign",
    "valuetype",
    "vlink",
];

impl<'c, 'd> Matcher<'c, 'd> {
    fn new(context: &'c MatchingContext<'d>, nesting: &'c NestingCache) -> Matcher<'c, 'd> {
        Matcher {
            context,
            document: context.document,
            anchor: None,
            nesting,
        }
    }

    fn list(self, list: &SelectorList, node: NodeId) -> bool {
        list.selectors
            .iter()
            .any(|selector| self.complex(selector, node))
    }

    /// Gleicht einen komplexen Selektor ohne sein Pseudoelement ab.
    fn complex(self, selector: &Selector, node: NodeId) -> bool {
        let Some(last) = selector.compounds.len().checked_sub(1) else {
            return false;
        };
        let mut stack = Vec::new();
        let mut returned = self.enter(selector, last, node, &mut stack);
        loop {
            let Some(frame) = stack.last_mut() else {
                return returned == Some(Outcome::Matched);
            };
            let Some(previous) = frame.index.checked_sub(1) else {
                return false;
            };
            let Some(&combinator) = selector.combinators.get(previous) else {
                return false;
            };
            if let Some(outcome) = returned.take() {
                let verdict = match (outcome, combinator) {
                    (Outcome::Matched | Outcome::NotMatchedGlobally, _)
                    | (_, Combinator::NextSibling) => Some(outcome),
                    (_, Combinator::Child) => Some(Outcome::RestartFromClosestDescendant),
                    (Outcome::RestartFromClosestDescendant, Combinator::SubsequentSibling) => {
                        Some(outcome)
                    }
                    // Beim Nachfahren- und beim späteren Geschwister-
                    // Kombinator geht es mit dem nächsten Kandidaten weiter.
                    _ => None,
                };
                if verdict.is_some() {
                    stack.pop();
                    returned = verdict;
                    continue;
                }
            }
            match frame.candidate {
                Some(candidate) => {
                    frame.candidate = self.next_candidate(candidate, combinator);
                    returned = self.enter(selector, previous, candidate, &mut stack);
                }
                None => {
                    returned = Some(match combinator {
                        Combinator::Descendant | Combinator::Child => Outcome::NotMatchedGlobally,
                        Combinator::NextSibling | Combinator::SubsequentSibling => {
                            Outcome::RestartFromClosestDescendant
                        }
                    });
                    stack.pop();
                }
            }
        }
    }

    /// Gleicht den Verbundselektor `index` ab. Passt er und steht links noch
    /// einer, kommt ein Eintrag auf den Stapel, und das Ergebnis steht noch
    /// aus (`None`).
    fn enter(
        self,
        selector: &Selector,
        index: usize,
        node: NodeId,
        stack: &mut Vec<Frame>,
    ) -> Option<Outcome> {
        let matched = selector
            .compounds
            .get(index)
            .is_some_and(|compound| self.compound(compound, node));
        if !matched {
            return Some(Outcome::RestartFromClosestLaterSibling);
        }
        let Some(previous) = index.checked_sub(1) else {
            return Some(Outcome::Matched);
        };
        let Some(&combinator) = selector.combinators.get(previous) else {
            return Some(Outcome::NotMatchedGlobally);
        };
        stack.push(Frame {
            index,
            candidate: self.next_candidate(node, combinator),
        });
        None
    }

    /// Der nächste Kandidat für den Verbundselektor links von `combinator`.
    fn next_candidate(self, node: NodeId, combinator: Combinator) -> Option<NodeId> {
        match combinator {
            Combinator::Descendant | Combinator::Child => self
                .document
                .parent(node)
                .filter(|&parent| self.document.element(parent).is_some()),
            Combinator::NextSibling | Combinator::SubsequentSibling => {
                self.previous_element_sibling(node)
            }
        }
    }

    fn previous_element_sibling(self, node: NodeId) -> Option<NodeId> {
        let mut current = self.document.previous_sibling(node);
        while let Some(sibling) = current {
            if self.document.element(sibling).is_some() {
                return Some(sibling);
            }
            current = self.document.previous_sibling(sibling);
        }
        None
    }

    fn compound(self, compound: &Compound, node: NodeId) -> bool {
        let Some(element) = self.document.element(node) else {
            return false;
        };
        compound
            .iter()
            .all(|simple| self.simple(simple, node, element))
    }

    /// Ob für das Element die Regeln für HTML-Elemente in HTML-Dokumenten
    /// gelten.
    fn is_html(self, element: &Element) -> bool {
        element.namespace == Some(Namespace::Html) && self.document.is_html_document()
    }

    fn quirks(self) -> bool {
        self.document.quirks_mode() == QuirksMode::Quirks
    }

    fn simple(self, simple: &Simple, node: NodeId, element: &Element) -> bool {
        match simple {
            Simple::Type { namespace, name } => {
                namespace_matches(self.document, namespace, element.namespace)
                    && name
                        .as_ref()
                        .is_none_or(|name| element.local_name == *self.name(name, element))
            }
            Simple::Id(id) => element.attribute("id").is_some_and(|value| {
                let quirks = self.quirks();
                code_units(value, quirks) == code_units(id, quirks)
            }),
            Simple::Class(class) => element.attribute("class").is_some_and(|value| {
                let quirks = self.quirks();
                let class = code_units(class, quirks);
                code_units(value, quirks)
                    .split(|&unit| is_whitespace(unit))
                    .any(|word| word == class.as_slice())
            }),
            Simple::Attribute(attribute) => self.attribute(attribute, element),
            Simple::PseudoClass(class) => self.pseudo_class(class, node, element),
            Simple::Anchor => self.anchor == Some(node),
        }
    }

    /// Ein Name aus dem Selektor, wie er mit einem Element verglichen wird.
    fn name<'n>(self, name: &'n Name, element: &Element) -> &'n DomString {
        if self.is_html(element) {
            &name.lower
        } else {
            &name.original
        }
    }

    fn attribute(self, selector: &AttributeSelector, element: &Element) -> bool {
        let html = self.is_html(element);
        element.attributes.iter().any(|attribute| {
            // Namen mit Namensraum vergleicht HTML genau.
            let lower = html && attribute.namespace.is_none();
            let name = if lower {
                &selector.name.lower
            } else {
                &selector.name.original
            };
            if attribute.local_name != *name
                || !namespace_matches(self.document, &selector.namespace, attribute.namespace)
            {
                return false;
            }
            let Some(value) = &selector.value else {
                return true;
            };
            let ignore_case = match value.case {
                AttributeCase::Insensitive => true,
                AttributeCase::Sensitive => false,
                AttributeCase::Document => {
                    lower
                        && CASE_INSENSITIVE_ATTRIBUTES
                            .iter()
                            .any(|&name| attribute.local_name == name)
                }
            };
            value_matches(value, &attribute.value, ignore_case)
        })
    }

    fn pseudo_class(self, class: &PseudoClass, node: NodeId, element: &Element) -> bool {
        match class {
            PseudoClass::Is(list) | PseudoClass::Where(list) => self.list(list, node),
            PseudoClass::Not(list) => !self.list(list, node),
            PseudoClass::Has(list) => self.has(list, node),
            PseudoClass::Root => self.parent_kind(node) == Some(NodeKind::Document),
            PseudoClass::Empty => self.is_empty(node),
            PseudoClass::NthChild { formula, of, last } => {
                let counts = |sibling: NodeId| of.as_ref().is_none_or(|of| self.list(of, sibling));
                counts(node) && formula.contains(self.index(node, *last, counts))
            }
            PseudoClass::NthOfType { formula, last } => {
                formula
                    .contains(self.index(node, *last, |sibling| self.same_type(sibling, element)))
            }
            PseudoClass::OnlyChild => {
                self.index(node, false, |_| true) == 1 && self.index(node, true, |_| true) == 1
            }
            PseudoClass::OnlyOfType => {
                let same = |sibling: NodeId| self.same_type(sibling, element);
                self.index(node, false, same) == 1 && self.index(node, true, same) == 1
            }
            // Ohne Verlauf ist jeder Verweis unbesucht.
            PseudoClass::AnyLink | PseudoClass::Link => is_link(element),
            PseudoClass::Visited => false,
            PseudoClass::Target => self.context.target == Some(node),
            PseudoClass::Scope | PseudoClass::Nesting(None) => self.is_scope(node),
            PseudoClass::Nesting(Some(list)) => self.nesting(list, node),
            // Ohne Nutzer und Fokus trifft keine Nutzeraktion zu.
            PseudoClass::Hover
            | PseudoClass::Active
            | PseudoClass::Focus
            | PseudoClass::FocusVisible
            | PseudoClass::FocusWithin => false,
            PseudoClass::Lang(ranges) => {
                let unknown = DomString::new();
                let language = self.language(node).unwrap_or(&unknown);
                ranges.iter().any(|range| lang::matches(language, range))
            }
            PseudoClass::Enabled => {
                [
                    "button", "input", "select", "textarea", "optgroup", "option", "fieldset",
                ]
                .iter()
                .any(|name| element.is_html(name))
                    && !self.document.is_actually_disabled(node)
            }
            PseudoClass::Disabled => self.document.is_actually_disabled(node),
            PseudoClass::Heading => self.document.heading_level(node).is_some(),
            PseudoClass::HeadingLevel(levels) => {
                self.document.heading_level(node).is_some_and(|level| {
                    levels
                        .iter()
                        .any(|&wanted| u64::try_from(wanted) == Ok(level))
                })
            }
            // Ohne Skript zeigt sich kein Popover, und kein Dialog ist modal;
            // ohne Formulardaten füllt nichts automatisch aus.
            PseudoClass::PopoverOpen | PseudoClass::Modal | PseudoClass::Autofill => false,
            PseudoClass::Checked => {
                let checkable = element.is_html("input")
                    && element.attribute("type").is_some_and(|kind| {
                        kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio")
                    });
                (checkable && self.document.checkedness(node)) || self.document.selectedness(node)
            }
        }
    }

    fn parent_kind(self, node: NodeId) -> Option<NodeKind> {
        self.document
            .parent(node)
            .and_then(|parent| self.document.kind(parent))
    }

    /// `:empty` nach Selectors Level 3: keine Kinder außer Kommentaren,
    /// Verarbeitungsanweisungen und leeren Texten. Der Entwurf von Level 4
    /// lässt auch Leerraum zu; die WPT-Tests und die Browser nicht.
    fn is_empty(self, node: NodeId) -> bool {
        self.document
            .children(node)
            .all(|child| match self.document.kind(child) {
                Some(NodeKind::Text | NodeKind::CdataSection) => {
                    self.document.data(child).is_none_or(DomString::is_empty)
                }
                Some(NodeKind::Comment | NodeKind::ProcessingInstruction) => true,
                _ => false,
            })
    }

    /// Die Stelle des Elements unter seinen Geschwistern, die `counts`
    /// zählt, von vorn oder mit `last` von hinten, beginnend bei 1. Das
    /// Element selbst zählt immer.
    fn index(self, node: NodeId, last: bool, counts: impl Fn(NodeId) -> bool) -> i64 {
        let step = |node: NodeId| {
            if last {
                self.document.next_sibling(node)
            } else {
                self.document.previous_sibling(node)
            }
        };
        let mut index: i64 = 1;
        let mut current = step(node);
        while let Some(sibling) = current {
            if self.document.element(sibling).is_some() && counts(sibling) {
                index = index.saturating_add(1);
            }
            current = step(sibling);
        }
        index
    }

    /// Ob ein Geschwister denselben Typ hat: Namensraum und lokalen Namen.
    fn same_type(self, sibling: NodeId, element: &Element) -> bool {
        self.document.element(sibling).is_some_and(|other| {
            other.namespace == element.namespace && other.local_name == element.local_name
        })
    }

    fn is_scope(self, node: NodeId) -> bool {
        match self.context.scoping_root {
            Some(root) if self.document.element(root).is_some() => node == root,
            // Ohne Element als *scoping root* steht `:scope` für die Wurzel
            // des Baums, in einem Dokument also wie `:root` für dessen
            // Element.
            _ => matches!(self.parent_kind(node), None | Some(NodeKind::Document)),
        }
    }

    /// `&` in einer verschachtelten Regel: ob die Liste der Elternregel das
    /// Element trifft. Wie `:is()` trifft `&` keine Pseudoelemente.
    fn nesting(self, list: &Arc<SelectorList>, node: NodeId) -> bool {
        let key = (Arc::as_ptr(list).addr(), node, self.anchor);
        if let Some(known) = self.nesting.borrow().get(&key).copied() {
            return known;
        }
        let matched = list
            .selectors
            .iter()
            .any(|selector| selector.pseudo_element.is_none() && self.complex(selector, node));
        self.nesting.borrow_mut().insert(key, matched);
        matched
    }

    /// `:has()`: Ob ein Element, das zu einem relativen Selektor passt, an
    /// diesem Element verankert ist. Gesucht wird unter den Nachkommen, bei
    /// Geschwister-Kombinatoren unter den folgenden Geschwistern und, wenn
    /// der Selektor dort weiter absteigt, auch in deren Nachkommen.
    fn has(self, list: &SelectorList, node: NodeId) -> bool {
        let matcher = Matcher {
            anchor: Some(node),
            ..self
        };
        let document = self.document;
        list.selectors.iter().any(|selector| {
            let siblings = matches!(
                selector.combinators.first(),
                Some(Combinator::NextSibling | Combinator::SubsequentSibling)
            );
            if !siblings {
                let mut current = document.first_child(node);
                while let Some(candidate) = current {
                    if matcher.complex(selector, candidate) {
                        return true;
                    }
                    current = document.following(candidate, node);
                }
                return false;
            }
            let descends =
                selector.combinators.iter().skip(1).any(|combinator| {
                    matches!(combinator, Combinator::Descendant | Combinator::Child)
                });
            let mut sibling = document.next_sibling(node);
            while let Some(root) = sibling {
                let mut current = Some(root);
                while let Some(candidate) = current {
                    if matcher.complex(selector, candidate) {
                        return true;
                    }
                    current = if descends {
                        document.following(candidate, root)
                    } else {
                        None
                    };
                }
                sibling = document.next_sibling(root);
            }
            false
        })
    }

    /// Die Sprache eines Elements nach dem HTML Standard („The lang and
    /// xml:lang attributes“); `None` heißt unbekannt.
    fn language(self, node: NodeId) -> Option<&'d DomString> {
        let document = self.document;
        let mut current = Some(node);
        while let Some(node) = current {
            let element = document.element(node)?;
            let xml_lang = element.attributes.iter().find(|attribute| {
                attribute.namespace == Some(Namespace::Xml) && attribute.local_name == "lang"
            });
            if let Some(attribute) = xml_lang {
                return Some(&attribute.value);
            }
            if matches!(element.namespace, Some(Namespace::Html | Namespace::Svg))
                && let Some(value) = element.attribute("lang")
            {
                return Some(value);
            }
            current = document
                .parent(node)
                .filter(|&parent| document.element(parent).is_some());
        }
        self.context.default_language
    }
}

/// Ob das Element ein Verweis ist: `a` und `area` mit `href` (HTML,
/// „Pseudo-classes“) und `a` aus SVG mit `href` oder `xlink:href` (SVG 2,
/// „Links out of SVG content“).
fn is_link(element: &Element) -> bool {
    if element.is_html("a") || element.is_html("area") {
        return element.attribute("href").is_some();
    }
    element.is(Namespace::Svg, "a")
        && element.attributes.iter().any(|attribute| {
            matches!(attribute.namespace, None | Some(Namespace::XLink))
                && attribute.local_name == "href"
        })
}

fn namespace_matches(
    document: &Document,
    constraint: &NamespaceConstraint,
    namespace: Option<Namespace>,
) -> bool {
    match constraint {
        NamespaceConstraint::Any => true,
        NamespaceConstraint::None => namespace.is_none(),
        NamespaceConstraint::Url(url) => namespace
            .and_then(|namespace| document.namespace_url(namespace))
            .is_some_and(|found| found == url),
    }
}

fn value_matches(selector: &AttributeValue, value: &DomString, ignore_case: bool) -> bool {
    let actual = code_units(value, ignore_case);
    let expected = code_units(&selector.value, ignore_case);
    match selector.matcher {
        AttributeMatcher::Exact => actual == expected,
        AttributeMatcher::Word => {
            !expected.is_empty()
                && !expected.iter().any(|&unit| is_whitespace(unit))
                && actual
                    .split(|&unit| is_whitespace(unit))
                    .any(|word| word == expected.as_slice())
        }
        AttributeMatcher::Dash => {
            actual == expected
                || (actual.starts_with(&expected) && actual.get(expected.len()) == Some(&0x2D))
        }
        AttributeMatcher::Prefix => !expected.is_empty() && actual.starts_with(&expected),
        AttributeMatcher::Suffix => !expected.is_empty() && actual.ends_with(&expected),
        AttributeMatcher::Substring => {
            !expected.is_empty()
                && actual
                    .windows(expected.len())
                    .any(|window| window == expected.as_slice())
        }
    }
}
