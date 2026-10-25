//! Die WPT-Tests zu `querySelectorAll()` (`dom/nodes/ParentNode-querySelector-All.html`
//! mit `selectors.js`), nachgebaut ohne JavaScript.
//!
//! Der Test liest die Listen `invalidSelectors` und `validSelectors` aus
//! `selectors.js` und baut die Schritte von `init()` und
//! `setupSpecialElements()` nach: dieselben zusätzlichen Elemente, dieselben
//! Klone und die `div`s, die `verifyStaticList()` anhängt, in derselben
//! Reihenfolge. Die Seite wird wie im Test mit `#target` geladen.

use std::collections::BTreeMap;

use mb_base::{CodePoint, DomString};
use mb_css::Source;
use mb_dom::{Attribute, Document, Namespace, NodeId, NodeKind};
use mb_html::{Options, ScriptingMode, parse_document};

use super::{MatchingContext, Namespaces, SelectorList};

/// Fälle, die mit dem bisherigen Dokumentspeicher nicht nachzubauen sind,
/// mit Begründung. Der Test prüft, dass sie weiter abweichen.
const DEVIATIONS: &[(&str, &str)] = &[
    (
        "#any-namespace *|div",
        "`setupSpecialElements()` legt ein `div` im Namensraum \
         http://www.example.org/ns an; mb-dom kennt nur die Namensräume des \
         HTML-Parsers, das Element fehlt also.",
    ),
    (
        "#attr-presence [*|TiTlE]",
        "`setupSpecialElements()` setzt ein Attribut `title` im Namensraum \
         http://www.example.org/ns; auch das lässt sich noch nicht anlegen.",
    ),
];

/// `TEST_QSA` aus `selectors.js`
const TEST_QSA: i64 = 0x01;

/// Ein Wert aus `selectors.js`. Zeichenketten bleiben UTF-16, denn die
/// Tests enthalten Escapes für Codeeinheiten.
#[derive(Clone, Debug)]
enum Js {
    Number(i64),
    String(Vec<u16>),
    Array(Vec<Js>),
    Object(Vec<(String, Js)>),
}

impl Js {
    fn get(&self, key: &str) -> Option<&Js> {
        match self {
            Js::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    fn as_array(&self) -> &[Js] {
        match self {
            Js::Array(items) => items,
            _ => panic!("keine Liste: {self:?}"),
        }
    }

    fn as_units(&self) -> &[u16] {
        match self {
            Js::String(units) => units,
            _ => panic!("keine Zeichenkette: {self:?}"),
        }
    }

    fn as_text(&self) -> String {
        String::from_utf16_lossy(self.as_units())
    }
}

/// Liest die Folge von `var NAME = WERT;` in `selectors.js`. Werte sind
/// Literale für Zahlen, Zeichenketten, Listen und Objekte, Namen schon
/// gelesener Variablen und Verknüpfungen mit `|`.
struct Reader {
    chars: Vec<char>,
    position: usize,
    variables: BTreeMap<String, Js>,
}

impl Reader {
    fn new(text: &str) -> Reader {
        Reader {
            chars: text.chars().collect(),
            position: 0,
            variables: BTreeMap::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars
            .get(self.position.saturating_add(offset))
            .copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.position = self.position.saturating_add(1);
        Some(c)
    }

    fn fail(&self, what: &str) -> ! {
        let line = self
            .chars
            .iter()
            .take(self.position)
            .filter(|&&c| c == '\n')
            .count()
            .saturating_add(1);
        panic!("selectors.js, Zeile {line}: {what}");
    }

    fn expect(&mut self, expected: char) {
        self.skip();
        if self.bump() != Some(expected) {
            self.fail(&format!("„{expected}“ erwartet"));
        }
    }

    /// Überspringt Leerraum und Kommentare.
    fn skip(&mut self) {
        loop {
            match (self.peek(), self.peek_at(1)) {
                (Some(c), _) if c.is_whitespace() => {
                    self.bump();
                }
                (Some('/'), Some('/')) => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                (Some('/'), Some('*')) => {
                    self.position = self.position.saturating_add(2);
                    while !matches!((self.peek(), self.peek_at(1)), (Some('*'), Some('/'))) {
                        if self.bump().is_none() {
                            self.fail("Kommentar endet nicht");
                        }
                    }
                    self.position = self.position.saturating_add(2);
                }
                _ => return,
            }
        }
    }

    fn identifier(&mut self) -> String {
        self.skip();
        let mut name = String::new();
        while let Some(c) = self
            .peek()
            .filter(|&c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        {
            name.push(c);
            self.bump();
        }
        if name.is_empty() {
            self.fail("Name erwartet");
        }
        name
    }

    fn script(mut self) -> BTreeMap<String, Js> {
        loop {
            self.skip();
            if self.peek().is_none() {
                return self.variables;
            }
            if self.identifier() != "var" {
                self.fail("„var“ erwartet");
            }
            let name = self.identifier();
            self.expect('=');
            let value = self.value();
            self.expect(';');
            self.variables.insert(name, value);
        }
    }

    fn value(&mut self) -> Js {
        let mut value = self.primary();
        loop {
            self.skip();
            if self.peek() != Some('|') {
                return value;
            }
            self.bump();
            let (Js::Number(left), Js::Number(right)) = (&value, self.primary()) else {
                self.fail("„|“ nur zwischen Zahlen");
            };
            value = Js::Number(left | right);
        }
    }

    fn primary(&mut self) -> Js {
        self.skip();
        match self.peek() {
            Some(quote @ ('"' | '\'')) => {
                self.bump();
                Js::String(self.string(quote))
            }
            Some('[') => {
                self.bump();
                let mut items = Vec::new();
                loop {
                    self.skip();
                    if self.peek() == Some(']') {
                        self.bump();
                        return Js::Array(items);
                    }
                    items.push(self.value());
                    self.skip();
                    if self.peek() == Some(',') {
                        self.bump();
                    }
                }
            }
            Some('{') => {
                self.bump();
                let mut entries = Vec::new();
                loop {
                    self.skip();
                    if self.peek() == Some('}') {
                        self.bump();
                        return Js::Object(entries);
                    }
                    let key = self.identifier();
                    self.expect(':');
                    entries.push((key, self.value()));
                    self.skip();
                    if self.peek() == Some(',') {
                        self.bump();
                    }
                }
            }
            Some(c) if c.is_ascii_digit() => self.number(),
            Some(_) => {
                let name = self.identifier();
                match self.variables.get(&name) {
                    Some(value) => value.clone(),
                    None => self.fail(&format!("unbekannte Variable {name}")),
                }
            }
            None => self.fail("Wert erwartet"),
        }
    }

    fn number(&mut self) -> Js {
        let (radix, mut digits) =
            if self.peek() == Some('0') && matches!(self.peek_at(1), Some('x' | 'X')) {
                self.position = self.position.saturating_add(2);
                (16, String::new())
            } else {
                (10, String::new())
            };
        while let Some(c) = self.peek().filter(|c| c.is_digit(radix)) {
            digits.push(c);
            self.bump();
        }
        match i64::from_str_radix(&digits, radix) {
            Ok(value) => Js::Number(value),
            Err(_) => self.fail("Zahl erwartet"),
        }
    }

    /// Eine Zeichenkette bis zum schließenden `quote`, mit den Escapes von
    /// ECMAScript.
    fn string(&mut self, quote: char) -> Vec<u16> {
        let mut units = Vec::new();
        loop {
            let Some(c) = self.bump() else {
                self.fail("Zeichenkette endet nicht");
            };
            if c == quote {
                return units;
            }
            if c != '\\' {
                units.extend(c.encode_utf16(&mut [0; 2]).iter());
                continue;
            }
            let Some(escape) = self.bump() else {
                self.fail("Escape am Ende");
            };
            match escape {
                'n' => units.push(0x0A),
                't' => units.push(0x09),
                'r' => units.push(0x0D),
                'b' => units.push(0x08),
                'f' => units.push(0x0C),
                'v' => units.push(0x0B),
                '0' if !self.peek().is_some_and(|c| c.is_ascii_digit()) => units.push(0),
                'x' => {
                    let value = self.hex(2);
                    units.push(self.unit(value));
                }
                'u' if self.peek() == Some('{') => {
                    self.bump();
                    let mut value: u32 = 0;
                    while let Some(digit) = self.peek().and_then(|c| c.to_digit(16)) {
                        value = value.saturating_mul(16).saturating_add(digit);
                        self.bump();
                    }
                    if self.bump() != Some('}') {
                        self.fail("„}“ erwartet");
                    }
                    match char::from_u32(value) {
                        Some(c) => units.extend(c.encode_utf16(&mut [0; 2]).iter()),
                        None => units.push(self.unit(value)),
                    }
                }
                'u' => {
                    let value = self.hex(4);
                    units.push(self.unit(value));
                }
                // Fortsetzung über das Zeilenende
                '\r' => {
                    if self.peek() == Some('\n') {
                        self.bump();
                    }
                }
                '\n' | '\u{2028}' | '\u{2029}' => {}
                // *NonEscapeCharacter* und `\\`, `\'`, `\"`
                other => units.extend(other.encode_utf16(&mut [0; 2]).iter()),
            }
        }
    }

    fn hex(&mut self, count: usize) -> u32 {
        let mut value: u32 = 0;
        for _ in 0..count {
            let Some(digit) = self.bump().and_then(|c| c.to_digit(16)) else {
                self.fail("Hexadezimalziffer erwartet");
            };
            value = value.saturating_mul(16).saturating_add(digit);
        }
        value
    }

    fn unit(&self, value: u32) -> u16 {
        u16::try_from(value).unwrap_or_else(|_| self.fail("Wert zu groß"))
    }
}

/// Das erste Element unter den Nachkommen von `root` mit dieser ID, wie
/// `getElementById()`.
fn element_by_id(document: &Document, root: NodeId, id: &str) -> NodeId {
    let mut current = document.first_child(root);
    while let Some(node) = current {
        if document
            .element(node)
            .and_then(|element| element.attribute("id"))
            .is_some_and(|value| *value == *id)
        {
            return node;
        }
        current = document.following(node, root);
    }
    panic!("kein Element mit der ID {id}");
}

/// `createElement()` oder `createElementNS()` mit gesetzter ID.
fn create(document: &mut Document, namespace: Option<Namespace>, name: &str, id: &str) -> NodeId {
    document.create_element(
        namespace,
        DomString::from(name),
        vec![Attribute::new(DomString::from("id"), DomString::from(id))],
    )
}

fn append(document: &mut Document, parent: NodeId, node: NodeId) {
    document
        .append(parent, node)
        .expect("Einfügen sollte gelingen");
}

/// `setupSpecialElements()`. Das `div` im Namensraum
/// http://www.example.org/ns und das Attribut in diesem Namensraum fehlen,
/// siehe [`DEVIATIONS`].
fn setup_special_elements(document: &mut Document, parent: NodeId) {
    let null = document.create_element(Some(Namespace::Html), DomString::from("null"), Vec::new());
    append(document, parent, null);
    let undefined = document.create_element(
        Some(Namespace::Html),
        DomString::from("undefined"),
        Vec::new(),
    );
    append(document, parent, undefined);
    for prefix in ["any-namespace", "no-namespace"] {
        let container = create(document, Some(Namespace::Html), "div", prefix);
        for (index, namespace) in [Some(Namespace::Html), Some(Namespace::Html), None]
            .into_iter()
            .enumerate()
        {
            let id = format!("{prefix}-div{}", index.saturating_add(1));
            let div = create(document, namespace, "div", &id);
            append(document, container, div);
        }
        append(document, parent, container);
    }
}

fn clone(document: &mut Document, node: NodeId) -> NodeId {
    document
        .clone_node(node, true)
        .expect("Klonen sollte gelingen")
}

fn append_div(document: &mut Document, parent: NodeId) {
    let div = document.create_element(Some(Namespace::Html), DomString::from("div"), Vec::new());
    append(document, parent, div);
}

#[test]
fn query_selector_all() {
    let variables = Reader::new(&testdata::read("wpt", "dom/nodes/selectors.js")).script();
    let invalid = variables
        .get("invalidSelectors")
        .expect("invalidSelectors")
        .as_array();
    let valid = variables
        .get("validSelectors")
        .expect("validSelectors")
        .as_array();
    let namespaces = Namespaces::default();
    let parse = |units: &[u16]| SelectorList::parse(&Source::from_utf16(units), &namespaces);

    let mut failures = Vec::new();
    for entry in invalid {
        let selector = entry.get("selector").expect("selector");
        if parse(selector.as_units()).is_ok() {
            failures.push(format!("gültig statt ungültig: {}", selector.as_text()));
        }
    }

    // `init()`
    let html = testdata::read("wpt", "dom/nodes/ParentNode-querySelector-All-content.html");
    let mut document = Document::new();
    parse_document(
        &mut document,
        html.chars().map(CodePoint::from),
        Options {
            scripting: ScriptingMode::Normal,
            collect_errors: false,
        },
    );
    document.perform_microtask_checkpoint();
    let doc = document.root();
    let target = element_by_id(&document, doc, "target");
    let body = element_by_id(&document, doc, "body");
    let element = element_by_id(&document, doc, "root");
    setup_special_elements(&mut document, element);
    let out_of_scope = clone(&mut document, element);
    let mut current = Some(out_of_scope);
    while let Some(node) = current {
        if document.kind(node) == Some(NodeKind::Element) {
            document.append_attribute(
                node,
                Attribute::new(DomString::from("data-clone"), DomString::new()),
            );
        }
        current = document.following(node, out_of_scope);
    }
    let detached = clone(&mut document, element);
    let fragment = document.create_fragment();
    let copy = clone(&mut document, element);
    append(&mut document, fragment, copy);
    let contexts = [
        ("Document", doc, "document"),
        ("Detached Element", detached, "detached"),
        ("Fragment", fragment, "fragment"),
        ("In-document Element", element, "element"),
    ];

    // `runSpecialSelectorTests()`: `null` und `undefined` werden zu Text.
    for (name, root, _) in contexts {
        for selector in ["null", "undefined"] {
            let list = SelectorList::parse(&Source::new(selector), &namespaces).expect(selector);
            let found = list.scope_match(root, &MatchingContext::new(&document));
            if found.len() != 1 {
                failures.push(format!("{name}: {selector}: {} Treffer", found.len()));
            }
        }
    }

    // `verifyStaticList()` hängt in jedem Kontext ein `div` an.
    for parent in [body, detached, fragment, element] {
        append_div(&mut document, parent);
    }

    let mut deviating = Vec::new();
    for (index, (name, root, kind)) in contexts.into_iter().enumerate() {
        // Vor den Tests im Element im Dokument kommt der Klon ins Dokument.
        if index == 3 {
            append(&mut document, body, out_of_scope);
        }
        let context = MatchingContext {
            target: Some(target),
            ..MatchingContext::new(&document)
        };
        for entry in valid {
            let excluded = entry.get("exclude").is_some_and(|exclude| {
                exclude
                    .as_array()
                    .iter()
                    .any(|value| value.as_text() == kind || value.as_text() == "html")
            });
            let test_type = match entry.get("testType") {
                Some(Js::Number(value)) => *value,
                _ => 0,
            };
            if excluded || test_type & TEST_QSA == 0 {
                continue;
            }
            let selector = entry.get("selector").expect("selector");
            let expected: Vec<Option<String>> = entry
                .get("expect")
                .expect("expect")
                .as_array()
                .iter()
                .map(|id| Some(id.as_text()))
                .collect();
            let Ok(list) = parse(selector.as_units()) else {
                failures.push(format!("{name}: ungültig: {}", selector.as_text()));
                continue;
            };
            let found = list.scope_match(root, &context);
            let ids: Vec<Option<String>> = found
                .iter()
                .map(|&node| {
                    let element = document.element(node)?;
                    Some(element.attribute("id")?.to_string_lossy())
                })
                .collect();
            let clones = found.iter().any(|&node| {
                document
                    .element(node)
                    .is_some_and(|element| element.attribute("data-clone").is_some())
            });
            if ids == expected && !clones {
                continue;
            }
            let text = selector.as_text();
            if DEVIATIONS.iter().any(|(deviation, _)| *deviation == text) {
                deviating.push(text);
            } else {
                failures.push(format!(
                    "{name}: {text}: erwartet {expected:?}, gefunden {ids:?}"
                ));
            }
        }
    }
    for (selector, _) in DEVIATIONS {
        if !deviating.iter().any(|found| found == selector) {
            failures.push(format!("weicht nicht mehr ab: {selector}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} Fehler:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
