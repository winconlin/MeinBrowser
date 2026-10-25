//! Der Baumaufbau (HTML Standard, Abschnitt „Tree construction“) und das
//! Fragment-Parsing („Parsing HTML fragments“).
//!
//! Wie der Tokenizer von Hand nach dem Wortlaut des Standards: Jeder
//! Einfügemodus ist eine Methode in `modes.rs`, die Regeln für Fremdinhalt
//! (SVG und MathML) stehen in `foreign.rs`, die gemeinsamen Algorithmen hier.
//! Die Kommentare nennen die Abschnitte und Begriffe des Standards.
//!
//! Noch nicht umgesetzt, weil die übrigen Teile fehlen:
//!
//! - Skripte: Ein `script` wird eingefügt, aber nie ausgeführt;
//!   `document.write()` und Nachladen kommen mit der Skriptanbindung.
//! - Der Sanitizer (*parser sanitizer configuration* bleibt null) und
//!   deklarative Shadow Roots (*allow declarative shadow roots* bleibt
//!   false).
//! - Das Patchen von Inhalten (`<template for>`): Die *insertion target*
//!   eines `template` bleibt null, und ein `template` mit `for` wird wie
//!   beim Scheitern von *prepare content patching* normal eingefügt.
//! - Benutzerdefinierte Elemente, Formularzuordnung, das Zurücksetzen
//!   (*reset algorithm*) und der Wechsel der Zeichenkodierung durch `meta`.
//! - *iframe srcdoc document* und *parser cannot change the mode flag*:
//!   Beides gilt als nicht gesetzt.

use std::collections::{BTreeMap, BTreeSet};

use mb_base::{CodePoint, DomString};
use mb_dom::{self as dom, Document, Namespace, NodeId, NodeKind};

use crate::error::{ErrorCode, ParseError};
use crate::tokenizer::{Tag, Token, Tokenizer, TokenizerState};

mod foreign;
mod modes;
#[cfg(test)]
mod tests;

/// Der Skriptmodus des Parsers (*parser scripting mode*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptingMode {
    /// Skripte laufen beim Einfügen; `noscript` ist Rohtext.
    Normal,
    /// Skripte sind abgeschaltet; `noscript` enthält normales Markup.
    Disabled,
    /// Skripte gelten als schon gestartet (Standard beim Fragment-Parsing).
    Inert,
    /// Skripte laufen sofort beim Einfügen (zum Beispiel
    /// `createContextualFragment()`).
    Fragment,
}

/// Einstellungen für einen Durchlauf des Parsers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub scripting: ScriptingMode,
    /// Parse-Fehler sammeln und zurückgeben.
    pub collect_errors: bool,
}

/// Parst ein ganzes Dokument in `document`, das noch leer sein sollte. Gibt
/// die Parse-Fehler zurück, falls [`Options::collect_errors`] gesetzt ist.
///
/// Mikroaufgaben, die beim Aufbau entstehen (etwa beim Umhängen einer
/// `option`), bleiben im Dokument liegen, bis der Aufrufer
/// [`Document::perform_microtask_checkpoint`] aufruft; bei einer Seite tut
/// das die Ereignisschleife nach der Aufgabe des Parsers.
pub fn parse_document(
    document: &mut Document,
    input: impl IntoIterator<Item = CodePoint>,
    options: Options,
) -> Vec<ParseError> {
    let mut tokenizer = Tokenizer::from_code_points(input);
    if options.collect_errors {
        tokenizer.collect_errors();
    }
    let mut builder = TreeBuilder::new(document, tokenizer, options.scripting);
    builder.run();
    builder.tokenizer.take_errors()
}

/// „HTML fragment parsing algorithm“ mit dem Element `context` aus
/// `document` als Kontext. Das Ergebnis ist ein neues `DocumentFragment` in
/// `document` mit den geparsten Knoten, dazu die Parse-Fehler wie bei
/// [`parse_document`]. Ist `context` kein Element, gibt es kein Ergebnis.
///
/// [`Options::scripting`] ist nach dem Standard [`ScriptingMode::Inert`]
/// oder [`ScriptingMode::Fragment`], oder [`ScriptingMode::Disabled`], wenn
/// Skripte im Dokument des Kontexts abgeschaltet sind.
///
/// Der Standard parst in ein eigenes, neues Dokument und übernimmt das
/// Fragment danach (*adopt*). Hier liegt alles gleich in `document`; der
/// Modus des neuen Dokuments wäre ohnehin der von `document`. Das Wurzel-
/// element `html`, das der Algorithmus anlegt, bleibt ohne Elternknoten.
pub fn parse_fragment(
    document: &mut Document,
    context: NodeId,
    input: impl IntoIterator<Item = CodePoint>,
    options: Options,
) -> Option<(NodeId, Vec<ParseError>)> {
    let context_element = document.element(context)?;
    let html_context = |name: &str| context_element.is_html(name);
    let state = if html_context("title") || html_context("textarea") {
        Some(TokenizerState::Rcdata)
    } else if ["style", "xmp", "iframe", "noembed", "noframes"]
        .iter()
        .any(|name| html_context(name))
        || (html_context("noscript") && options.scripting != ScriptingMode::Disabled)
    {
        Some(TokenizerState::Rawtext)
    } else if html_context("script") {
        Some(TokenizerState::ScriptData)
    } else if html_context("plaintext") {
        Some(TokenizerState::Plaintext)
    } else {
        None
    };
    // Das „falsche“ Start-Tag des Kontexts entscheidet, ob er ein
    // HTML-Integrationspunkt ist.
    let integration_point = context_element.is(Namespace::MathMl, "annotation-xml")
        && context_element
            .attribute("encoding")
            .is_some_and(is_html_encoding);
    let is_template = context_element.is_html("template");

    let mut tokenizer = Tokenizer::from_code_points(input);
    if options.collect_errors {
        tokenizer.collect_errors();
    }
    if let Some(state) = state {
        tokenizer.set_state(state);
    }
    let root = document.create_element(Some(Namespace::Html), DomString::from("html"), Vec::new());
    let fragment = document.create_fragment();
    let mut builder = TreeBuilder::new(document, tokenizer, options.scripting);
    builder.context = Some(context);
    if integration_point {
        builder.integration_points.insert(context);
    }
    builder.open.push(root);
    builder.redirects.insert(
        root,
        Location {
            parent: fragment,
            before: None,
        },
    );
    if is_template {
        builder.template_modes.push(Mode::InTemplate);
    }
    builder.reset_insertion_mode();
    // Das nächste `form` auf dem Weg vom Kontext nach oben, er selbst
    // eingeschlossen.
    let mut node = Some(context);
    while let Some(current) = node {
        if builder.is_html(current, "form") {
            builder.form = Some(current);
            break;
        }
        node = builder.document.parent(current);
    }
    builder.run();
    Some((fragment, builder.tokenizer.take_errors()))
}

/// Die Einfügemodi (*insertion mode*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

/// Eine Einfügestelle (*insertion location*): der Elternknoten und das Kind,
/// vor dem eingefügt wird (am Ende, wenn keins).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Location {
    parent: NodeId,
    before: Option<NodeId>,
}

/// Ein Eintrag der Liste aktiver Formatierungselemente mit dem Token, für
/// das das Element entstand.
#[derive(Clone, Debug)]
enum Formatting {
    Marker,
    Element { node: NodeId, tag: Tag },
}

/// Wie es nach einem Token weitergeht: fertig, oder das Token noch einmal
/// durch den Verteiler (*reprocess the token*).
enum Flow {
    Done,
    Reprocess(Token),
}

/// Die Bereiche für „has an element in … scope“.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Default,
    ListItem,
    Button,
    Table,
}

const LINE_FEED: CodePoint = CodePoint::from_char('\n');

/// „Generate implied end tags“
const IMPLIED_END_TAGS: [&str; 10] = [
    "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc",
];

/// „Generate all implied end tags thoroughly“
const IMPLIED_END_TAGS_THOROUGHLY: [&str; 18] = [
    "caption", "colgroup", "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc",
    "tbody", "td", "tfoot", "th", "thead", "tr",
];

/// Die HTML-Elemente der Kategorie *special*.
const SPECIAL: [&str; 83] = [
    "address",
    "applet",
    "area",
    "article",
    "aside",
    "base",
    "basefont",
    "bgsound",
    "blockquote",
    "body",
    "br",
    "button",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dir",
    "div",
    "dl",
    "dt",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "iframe",
    "img",
    "input",
    "keygen",
    "li",
    "link",
    "listing",
    "main",
    "marquee",
    "menu",
    "meta",
    "nav",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "ol",
    "p",
    "param",
    "plaintext",
    "pre",
    "script",
    "search",
    "section",
    "select",
    "source",
    "style",
    "summary",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
    "wbr",
    "xmp",
];

/// Wer im Stapel steht, wenn „in body“ das Ende von `body` oder der Datei
/// sieht, ohne dass es ein Parse-Fehler ist.
const CLOSABLE_AT_END: [&str; 18] = [
    "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc", "tbody", "td", "tfoot",
    "th", "thead", "tr", "body", "html",
];

/// ASCII-Leerraum, wie ihn die Einfügemodi bei Zeichen-Tokens prüfen.
fn is_whitespace(c: CodePoint) -> bool {
    matches!(c.value(), 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

/// Ob ein `encoding`-Attribut ein `annotation-xml` zum HTML-
/// Integrationspunkt macht.
fn is_html_encoding(value: &DomString) -> bool {
    value.eq_ignore_ascii_case("text/html") || value.eq_ignore_ascii_case("application/xhtml+xml")
}

impl Tag {
    /// Ein Start-Tag ohne Attribute, wie es der Baumaufbau an manchen
    /// Stellen selbst erzeugt („a … start tag token with no attributes“).
    fn named(name: &str) -> Tag {
        Tag {
            name: DomString::from(name),
            self_closing: false,
            attributes: Vec::new(),
        }
    }

    fn attribute(&self, name: &str) -> Option<&DomString> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == *name)
            .map(|attribute| &attribute.value)
    }
}

/// Gleicher Name und gleiche Attribute, in beliebiger Reihenfolge (für die
/// „Noah's Ark“-Regel). Der Tokenizer verwirft doppelte Attributnamen.
fn same_token(a: &Tag, b: &Tag) -> bool {
    a.name == b.name
        && a.attributes.len() == b.attributes.len()
        && a.attributes.iter().all(|attribute| {
            b.attributes
                .iter()
                .any(|other| other.name == attribute.name && other.value == attribute.value)
        })
}

struct TreeBuilder<'a> {
    document: &'a mut Document,
    tokenizer: Tokenizer,
    mode: Mode,
    original_mode: Mode,
    template_modes: Vec<Mode>,
    /// Der Stapel offener Elemente; das letzte ist der aktuelle Knoten.
    open: Vec<NodeId>,
    formatting: Vec<Formatting>,
    head: Option<NodeId>,
    form: Option<NodeId>,
    /// Das Kontextelement beim Fragment-Parsing.
    context: Option<NodeId>,
    scripting: ScriptingMode,
    frameset_ok: bool,
    foster_parenting: bool,
    /// *insertion target redirection map*
    redirects: BTreeMap<NodeId, Location>,
    /// Die `annotation-xml`-Elemente, deren Start-Tag ein passendes
    /// `encoding` hatte.
    integration_points: BTreeSet<NodeId>,
    /// *pending table character tokens*
    pending_table_characters: Vec<CodePoint>,
    /// Ein LF als nächstes Token wird übergangen (nach `pre`, `listing`,
    /// `textarea`).
    skip_line_feed: bool,
    /// Das aktuelle Start-Tag hat den Schrägstrich, und der Baumaufbau hat
    /// ihn noch nicht bestätigt (*acknowledge the self-closing flag*).
    self_closing_pending: bool,
}

impl<'a> TreeBuilder<'a> {
    fn new(
        document: &'a mut Document,
        tokenizer: Tokenizer,
        scripting: ScriptingMode,
    ) -> TreeBuilder<'a> {
        TreeBuilder {
            document,
            tokenizer,
            mode: Mode::Initial,
            original_mode: Mode::Initial,
            template_modes: Vec::new(),
            open: Vec::new(),
            formatting: Vec::new(),
            head: None,
            form: None,
            context: None,
            scripting,
            frameset_ok: true,
            foster_parenting: false,
            redirects: BTreeMap::new(),
            integration_points: BTreeSet::new(),
            pending_table_characters: Vec::new(),
            skip_line_feed: false,
            self_closing_pending: false,
        }
    }

    /// Verarbeitet Token um Token bis zum Dateiende. Jeder Einfügemodus
    /// beendet das Parsen beim Dateiende („stop parsing“).
    fn run(&mut self) {
        // Beim Fragment-Parsen ist der Kontext schon vor dem ersten Token der
        // angepasste aktuelle Knoten; ein `<![CDATA[` am Anfang braucht das.
        self.update_foreign_content();
        loop {
            let token = self.tokenizer.next_token();
            let end = token == Token::Eof;
            self.process(token);
            if end {
                break;
            }
        }
    }

    /// Ein neues Token vom Tokenizer.
    fn process(&mut self, token: Token) {
        if std::mem::take(&mut self.skip_line_feed) && token == Token::Character(LINE_FEED) {
            return;
        }
        self.self_closing_pending = matches!(&token, Token::StartTag(tag) if tag.self_closing);
        let mut token = token;
        loop {
            // „Tree construction dispatcher“
            let flow = if self.in_html_content(&token) {
                self.html_content(token)
            } else {
                self.foreign_content(token)
            };
            match flow {
                Flow::Done => break,
                Flow::Reprocess(next) => token = next,
            }
        }
        if std::mem::take(&mut self.self_closing_pending) {
            self.tokenizer.report(Some(
                ErrorCode::NonVoidHtmlElementStartTagWithTrailingSolidus,
            ));
        }
        self.update_foreign_content();
    }

    /// Sagt dem Tokenizer, ob der angepasste aktuelle Knoten außerhalb des
    /// HTML-Namensraums liegt (für CDATA-Abschnitte).
    fn update_foreign_content(&mut self) {
        let foreign = self
            .adjusted_current_node()
            .is_some_and(|node| !self.is_html_element(node));
        self.tokenizer.set_foreign_content(foreign);
    }

    /// Ob der Verteiler das Token nach den Regeln für HTML-Inhalt
    /// verarbeitet.
    fn in_html_content(&self, token: &Token) -> bool {
        let Some(node) = self.adjusted_current_node() else {
            return true;
        };
        if self.is_html_element(node) || *token == Token::Eof {
            return true;
        }
        let text_point = self.is_mathml_text_integration_point(node);
        let html_point = self.is_html_integration_point(node);
        match token {
            Token::StartTag(tag) => {
                (text_point && tag.name != "mglyph" && tag.name != "malignmark")
                    || (self.is_element(node, Namespace::MathMl, "annotation-xml")
                        && tag.name == "svg")
                    || html_point
            }
            Token::Character(_) => text_point || html_point,
            _ => false,
        }
    }

    /// Die Regeln des aktuellen Einfügemodus für HTML-Inhalt.
    fn html_content(&mut self, token: Token) -> Flow {
        match self.mode {
            Mode::Initial => self.initial(token),
            Mode::BeforeHtml => self.before_html(token),
            Mode::BeforeHead => self.before_head(token),
            Mode::InHead => self.in_head(token),
            Mode::InHeadNoscript => self.in_head_noscript(token),
            Mode::AfterHead => self.after_head(token),
            Mode::InBody => self.in_body(token),
            Mode::Text => self.text(token),
            Mode::InTable => self.in_table(token),
            Mode::InTableText => self.in_table_text(token),
            Mode::InCaption => self.in_caption(token),
            Mode::InColumnGroup => self.in_column_group(token),
            Mode::InTableBody => self.in_table_body(token),
            Mode::InRow => self.in_row(token),
            Mode::InCell => self.in_cell(token),
            Mode::InTemplate => self.in_template(token),
            Mode::AfterBody => self.after_body(token),
            Mode::InFrameset => self.in_frameset(token),
            Mode::AfterFrameset => self.after_frameset(token),
            Mode::AfterAfterBody => self.after_after_body(token),
            Mode::AfterAfterFrameset => self.after_after_frameset(token),
        }
    }

    fn parse_error(&mut self) {
        self.tokenizer.report(None);
    }

    /// „Acknowledge the token's self-closing flag, if it is set.“
    fn acknowledge_self_closing(&mut self) {
        self.self_closing_pending = false;
    }

    // Elemente erkennen

    /// Namensraum und lokaler Name. Namen außerhalb von ASCII ergeben die
    /// leere Zeichenkette; sie gleicht keinem Namen, den der Standard prüft.
    fn element_name(&self, node: NodeId) -> Option<(Namespace, &str)> {
        let element = self.document.element(node)?;
        Some((
            element.namespace?,
            element.local_name.as_ascii().unwrap_or(""),
        ))
    }

    /// Der lokale Name eines HTML-Elements.
    fn html_name(&self, node: NodeId) -> Option<&str> {
        match self.element_name(node)? {
            (Namespace::Html, name) => Some(name),
            _ => None,
        }
    }

    fn is_html(&self, node: NodeId, name: &str) -> bool {
        self.html_name(node) == Some(name)
    }

    fn is_html_element(&self, node: NodeId) -> bool {
        self.html_name(node).is_some()
    }

    /// Ein HTML-Element mit demselben Namen wie ein Token.
    fn is_html_named(&self, node: NodeId, name: &DomString) -> bool {
        self.document.element(node).is_some_and(|element| {
            element.namespace == Some(Namespace::Html) && element.local_name == *name
        })
    }

    fn is_element(&self, node: NodeId, namespace: Namespace, name: &str) -> bool {
        self.element_name(node) == Some((namespace, name))
    }

    fn is_special(&self, node: NodeId) -> bool {
        match self.element_name(node) {
            Some((Namespace::Html, name)) => SPECIAL.contains(&name),
            Some((Namespace::MathMl, name)) => {
                matches!(name, "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml")
            }
            Some((Namespace::Svg, name)) => matches!(name, "foreignObject" | "desc" | "title"),
            _ => false,
        }
    }

    /// *MathML text integration point*
    fn is_mathml_text_integration_point(&self, node: NodeId) -> bool {
        matches!(
            self.element_name(node),
            Some((Namespace::MathMl, "mi" | "mo" | "mn" | "ms" | "mtext"))
        )
    }

    /// *HTML integration point*
    fn is_html_integration_point(&self, node: NodeId) -> bool {
        match self.element_name(node) {
            Some((Namespace::MathMl, "annotation-xml")) => self.integration_points.contains(&node),
            Some((Namespace::Svg, name)) => matches!(name, "foreignObject" | "desc" | "title"),
            _ => false,
        }
    }

    fn context_is_html(&self, name: &str) -> bool {
        self.context
            .is_some_and(|context| self.is_html(context, name))
    }

    // Der Stapel offener Elemente

    fn current_node(&self) -> Option<NodeId> {
        self.open.last().copied()
    }

    /// *adjusted current node*
    fn adjusted_current_node(&self) -> Option<NodeId> {
        match self.context {
            Some(context) if self.open.len() == 1 => Some(context),
            _ => self.current_node(),
        }
    }

    fn current_is_html(&self, name: &str) -> bool {
        self.current_node()
            .is_some_and(|node| self.is_html(node, name))
    }

    fn has_open_html(&self, name: &str) -> bool {
        self.open.iter().any(|&node| self.is_html(node, name))
    }

    /// *parsing template contents*
    fn parsing_template_contents(&self) -> bool {
        self.has_open_html("template") || self.context_is_html("template")
    }

    /// Ob der Knoten den Bereich begrenzt (*has an element in specific
    /// scope*).
    fn is_scope_boundary(&self, node: NodeId, scope: Scope) -> bool {
        let Some((namespace, name)) = self.element_name(node) else {
            return false;
        };
        if scope == Scope::Table {
            return namespace == Namespace::Html && matches!(name, "html" | "table" | "template");
        }
        let basic = match namespace {
            Namespace::Html => matches!(
                name,
                "applet"
                    | "caption"
                    | "html"
                    | "table"
                    | "td"
                    | "th"
                    | "marquee"
                    | "object"
                    | "select"
                    | "template"
            ),
            Namespace::MathMl => {
                matches!(name, "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml")
            }
            Namespace::Svg => matches!(name, "foreignObject" | "desc" | "title"),
            _ => false,
        };
        basic
            || match scope {
                Scope::ListItem => namespace == Namespace::Html && matches!(name, "ol" | "ul"),
                Scope::Button => namespace == Namespace::Html && name == "button",
                Scope::Default | Scope::Table => false,
            }
    }

    /// „The stack of open elements has an element target node in a specific
    /// scope“, mit dem ersten Element von unten, für das `target` gilt.
    fn in_scope(&self, scope: Scope, target: impl Fn(NodeId) -> bool) -> bool {
        for &node in self.open.iter().rev() {
            if target(node) {
                return true;
            }
            if self.is_scope_boundary(node, scope) {
                return false;
            }
        }
        false
    }

    /// Ein HTML-Element dieses Namens im Bereich.
    fn has_in_scope(&self, name: &str, scope: Scope) -> bool {
        self.in_scope(scope, |node| self.is_html(node, name))
    }

    fn has_named_in_scope(&self, name: &DomString, scope: Scope) -> bool {
        self.in_scope(scope, |node| self.is_html_named(node, name))
    }

    /// Nimmt den aktuellen Knoten vom Stapel („pop the current node off the
    /// stack of open elements“); der Dokumentspeicher führt die Schritte
    /// dafür aus. Das Entfernen mitten aus dem Stapel („remove … from the
    /// stack“) zählt nicht dazu.
    fn pop(&mut self) -> Option<NodeId> {
        let node = self.open.pop()?;
        self.document.popped_by_parser(node);
        Some(node)
    }

    /// Nimmt Elemente vom Stapel, bis noch `len` übrig sind.
    fn pop_to(&mut self, len: usize) {
        while self.open.len() > len {
            self.pop();
        }
    }

    /// Nimmt Elemente vom Stapel, bis eins genommen ist, für das `done` gilt.
    fn pop_until(&mut self, done: impl Fn(&Self, NodeId) -> bool) {
        while let Some(node) = self.pop() {
            if done(self, node) {
                break;
            }
        }
    }

    fn pop_until_html(&mut self, name: &str) {
        self.pop_until(|builder, node| builder.is_html(node, name));
    }

    fn pop_until_named(&mut self, name: &DomString) {
        self.pop_until(|builder, node| builder.is_html_named(node, name));
    }

    /// Entfernt einen Knoten an beliebiger Stelle aus dem Stapel.
    fn remove_from_stack(&mut self, node: NodeId) {
        if let Some(index) = self.open.iter().rposition(|&open| open == node) {
            self.open.remove(index);
        }
    }

    /// „Generate implied end tags“, ohne die HTML-Elemente namens `except`.
    fn generate_implied_end_tags(&mut self, except: Option<&str>) {
        while let Some(node) = self.current_node()
            && let Some(name) = self.html_name(node)
            && IMPLIED_END_TAGS.contains(&name)
            && Some(name) != except
        {
            self.pop();
        }
    }

    /// „Generate all implied end tags thoroughly“
    fn generate_implied_end_tags_thoroughly(&mut self) {
        while let Some(node) = self.current_node()
            && let Some(name) = self.html_name(node)
            && IMPLIED_END_TAGS_THOROUGHLY.contains(&name)
        {
            self.pop();
        }
    }

    /// Ob ein Element offen ist, das „in body“ am Ende nicht erwartet.
    fn has_unclosed_elements(&self) -> bool {
        self.open.iter().any(|&node| {
            !self
                .html_name(node)
                .is_some_and(|name| CLOSABLE_AT_END.contains(&name))
        })
    }

    /// „Close a p element“
    fn close_p_element(&mut self) {
        self.generate_implied_end_tags(Some("p"));
        if !self.current_is_html("p") {
            self.parse_error();
        }
        self.pop_until_html("p");
    }

    /// „If the stack of open elements has a p element in button scope, then
    /// close a p element.“
    fn close_p_in_button_scope(&mut self) {
        if self.has_in_scope("p", Scope::Button) {
            self.close_p_element();
        }
    }

    /// „Reset the insertion mode appropriately“
    fn reset_insertion_mode(&mut self) {
        let mut index = self.open.len();
        while let Some(previous) = index.checked_sub(1) {
            index = previous;
            let Some(&open) = self.open.get(index) else {
                break;
            };
            let last = index == 0;
            let node = match self.context {
                Some(context) if last => context,
                _ => open,
            };
            let mode = match self.html_name(node) {
                Some("td" | "th") if !last => Some(Mode::InCell),
                Some("tr") => Some(Mode::InRow),
                Some("tbody" | "thead" | "tfoot") => Some(Mode::InTableBody),
                Some("caption") => Some(Mode::InCaption),
                Some("colgroup") => Some(Mode::InColumnGroup),
                Some("table") => Some(Mode::InTable),
                Some("template") => self.template_modes.last().copied(),
                Some("head") if !last => Some(Mode::InHead),
                Some("body") => Some(Mode::InBody),
                Some("frameset") => Some(Mode::InFrameset),
                Some("html") => Some(if self.head.is_none() {
                    Mode::BeforeHead
                } else {
                    Mode::AfterHead
                }),
                _ if last => Some(Mode::InBody),
                _ => None,
            };
            if let Some(mode) = mode {
                self.mode = mode;
                return;
            }
        }
    }

    /// „Stop parsing“: Von „The end“ bleibt ohne Skripte und Ereignisse nur,
    /// alle Knoten vom Stapel zu nehmen.
    fn stop_parsing(&mut self) {
        self.pop_to(0);
    }

    // Die Liste aktiver Formatierungselemente

    fn formatting_index(&self, node: NodeId) -> Option<usize> {
        self.formatting.iter().rposition(
            |entry| matches!(entry, Formatting::Element { node: element, .. } if *element == node),
        )
    }

    /// Wo die Einträge nach der letzten Marke beginnen.
    fn formatting_start(&self) -> usize {
        self.formatting
            .iter()
            .rposition(|entry| matches!(entry, Formatting::Marker))
            .map_or(0, |index| index.saturating_add(1))
    }

    /// „Push onto the list of active formatting elements“
    fn push_formatting(&mut self, node: NodeId, tag: Tag) {
        let start = self.formatting_start();
        let mut same = self
            .formatting
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, entry)| {
                matches!(entry, Formatting::Element { tag: other, .. } if same_token(other, &tag))
            })
            .map(|(index, _)| index);
        let earliest = same.next();
        if same.count() >= 2
            && let Some(earliest) = earliest
        {
            self.formatting.remove(earliest);
        }
        self.formatting.push(Formatting::Element { node, tag });
    }

    /// Eine Marke oder ein Element, das noch im Stapel steht.
    fn is_marker_or_open(&self, index: usize) -> bool {
        match self.formatting.get(index) {
            Some(Formatting::Element { node, .. }) => self.open.contains(node),
            Some(Formatting::Marker) | None => true,
        }
    }

    /// „Reconstruct the active formatting elements“
    fn reconstruct_formatting(&mut self) {
        let Some(last) = self.formatting.len().checked_sub(1) else {
            return;
        };
        if self.is_marker_or_open(last) {
            return;
        }
        // Rewind
        let mut index = last;
        while let Some(previous) = index.checked_sub(1) {
            if self.is_marker_or_open(previous) {
                break;
            }
            index = previous;
        }
        // Advance und Create
        while let Some(Formatting::Element { tag, .. }) = self.formatting.get(index) {
            let tag = tag.clone();
            let element = self.insert_html_element(&tag);
            if let Some(entry) = self.formatting.get_mut(index) {
                *entry = Formatting::Element { node: element, tag };
            }
            if index >= last {
                break;
            }
            index = index.saturating_add(1);
        }
    }

    /// „Clear the list of active formatting elements up to the last marker“
    fn clear_formatting_to_marker(&mut self) {
        while let Some(entry) = self.formatting.pop() {
            if matches!(entry, Formatting::Marker) {
                break;
            }
        }
    }

    // Knoten anlegen und einfügen

    /// „Appropriate place for inserting a node“, ohne Stapel kein Ort.
    fn appropriate_place(&self, override_target: Option<NodeId>) -> Option<Location> {
        let target = match override_target {
            Some(target) => target,
            None => self.current_node()?,
        };
        let mut parent = target;
        let mut before = None;
        if self.foster_parenting
            && matches!(
                self.html_name(target),
                Some("table" | "tbody" | "tfoot" | "thead" | "tr")
            )
        {
            let last = self
                .open
                .iter()
                .rposition(|&node| matches!(self.html_name(node), Some("template" | "table")));
            let Some(last) = last else {
                return Some(Location {
                    parent: *self.open.first()?,
                    before: None,
                });
            };
            let node = *self.open.get(last)?;
            if self.is_html(node, "template") {
                parent = node;
            } else if let Some(table_parent) = self.document.parent(node) {
                parent = table_parent;
                before = Some(node);
            } else {
                parent = *self.open.get(last.checked_sub(1)?)?;
            }
        }
        if !self.is_html(parent, "template") {
            return Some(Location { parent, before });
        }
        // Ohne Patchen ist die *insertion target* eines `template` null.
        Some(Location {
            parent: self.document.template_contents(parent)?,
            before: None,
        })
    }

    /// „Adjusted insertion location“
    fn adjusted_location(&self, location: Option<Location>) -> Option<Location> {
        let location = match location {
            Some(location) => location,
            None => self.appropriate_place(None)?,
        };
        Some(
            self.redirects
                .get(&location.parent)
                .copied()
                .unwrap_or(location),
        )
    }

    fn document_location(&self) -> Location {
        Location {
            parent: self.document.root(),
            before: None,
        }
    }

    fn is_document_with_element(&self, node: NodeId) -> bool {
        self.document.kind(node) == Some(NodeKind::Document)
            && self
                .document
                .children(node)
                .any(|child| self.document.kind(child) == Some(NodeKind::Element))
    }

    /// „Create an element for the token“. Mit `adjust_foreign` werden
    /// vorher die Attribute nach „adjust foreign attributes“ in ihre
    /// Namensräume gelegt.
    fn create_element_for_token(
        &mut self,
        tag: &Tag,
        namespace: Namespace,
        adjust_foreign: bool,
    ) -> NodeId {
        let attributes = tag
            .attributes
            .iter()
            .map(|attribute| {
                if adjust_foreign {
                    foreign::adjust_foreign_attribute(attribute)
                } else {
                    dom::Attribute::new(attribute.name.clone(), attribute.value.clone())
                }
            })
            .collect();
        let element = self
            .document
            .create_element(Some(namespace), tag.name.clone(), attributes);
        let mut errors = 0;
        if let Some(created) = self.document.element(element) {
            let xmlns = |prefix: Option<&str>, local_name: &str| {
                created.attributes.iter().find(|attribute| {
                    attribute.namespace == Some(Namespace::Xmlns)
                        && attribute.prefix.as_ref().map(|prefix| prefix.as_ascii())
                            == prefix.map(Some)
                        && attribute.local_name == *local_name
                })
            };
            if xmlns(None, "xmlns").is_some_and(|attribute| attribute.value != *namespace.url()) {
                errors += 1;
            }
            if xmlns(Some("xmlns"), "xlink")
                .is_some_and(|attribute| attribute.value != *Namespace::XLink.url())
            {
                errors += 1;
            }
        }
        for _ in 0..errors {
            self.parse_error();
        }
        if namespace == Namespace::MathMl
            && tag.name == "annotation-xml"
            && tag.attribute("encoding").is_some_and(is_html_encoding)
        {
            self.integration_points.insert(element);
        }
        element
    }

    /// „Insert an element at the adjusted insertion location“
    fn insert_element_at(&mut self, element: NodeId, location: Option<Location>) {
        let Some(Location { parent, before }) = self.adjusted_location(location) else {
            return;
        };
        if self.document.parent(element).is_some()
            || self
                .document
                .is_host_including_inclusive_ancestor(element, parent)
            || self.is_document_with_element(parent)
        {
            return;
        }
        let inserted = self.document.insert_before(parent, element, before);
        debug_assert!(inserted.is_ok(), "Einfügen nach dem Standard gültig");
    }

    /// „Insert a foreign element“
    fn insert_foreign_element(
        &mut self,
        tag: &Tag,
        namespace: Namespace,
        adjust_foreign: bool,
    ) -> NodeId {
        let element = self.create_element_for_token(tag, namespace, adjust_foreign);
        self.insert_element_at(element, None);
        self.open.push(element);
        element
    }

    /// „Insert an HTML element“
    fn insert_html_element(&mut self, tag: &Tag) -> NodeId {
        self.insert_foreign_element(tag, Namespace::Html, false)
    }

    /// „Insert a character“
    fn insert_character(&mut self, c: CodePoint) {
        let Some(Location { parent, before }) = self.adjusted_location(None) else {
            return;
        };
        if self.document.kind(parent) == Some(NodeKind::Document) {
            return;
        }
        let previous = match before {
            Some(before) => self.document.previous_sibling(before),
            None => self.document.last_child(parent),
        };
        if let Some(previous) = previous
            && self.document.kind(previous) == Some(NodeKind::Text)
        {
            self.document.append_data(previous, c);
            return;
        }
        let mut data = DomString::new();
        data.push_code_point(c);
        let text = self.document.create_text(data);
        let inserted = self.document.insert_before(parent, text, before);
        debug_assert!(inserted.is_ok(), "Einfügen nach dem Standard gültig");
    }

    /// „Insert a comment“
    fn insert_comment(&mut self, data: DomString, location: Option<Location>) {
        let Some(Location { parent, before }) = self.adjusted_location(location) else {
            return;
        };
        let comment = self.document.create_comment(data);
        let inserted = self.document.insert_before(parent, comment, before);
        debug_assert!(inserted.is_ok(), "Einfügen nach dem Standard gültig");
    }

    /// „Insert a processing instruction“
    fn insert_processing_instruction(
        &mut self,
        target: DomString,
        data: DomString,
        location: Option<Location>,
    ) {
        let Some(Location { parent, before }) = self.adjusted_location(location) else {
            return;
        };
        let instruction = self.document.create_processing_instruction(target, data);
        let inserted = self.document.insert_before(parent, instruction, before);
        debug_assert!(inserted.is_ok(), "Einfügen nach dem Standard gültig");
    }

    /// „Generic raw text element parsing algorithm“ und „generic RCDATA
    /// element parsing algorithm“
    fn parse_text_element(&mut self, tag: &Tag, state: TokenizerState) {
        self.insert_html_element(tag);
        self.tokenizer.set_state(state);
        self.original_mode = self.mode;
        self.mode = Mode::Text;
    }

    // Der Adoption-Agency-Algorithmus

    /// „The adoption agency algorithm“ für ein Token namens `subject`.
    fn adoption_agency(&mut self, subject: &DomString) {
        if let Some(current) = self.current_node()
            && self.is_html_named(current, subject)
            && self.formatting_index(current).is_none()
        {
            self.pop();
            return;
        }
        for _ in 0..8 {
            let start = self.formatting_start();
            let found = self
                .formatting
                .iter()
                .enumerate()
                .skip(start)
                .rev()
                .find_map(|(index, entry)| match entry {
                    Formatting::Element { node, .. } if self.is_html_named(*node, subject) => {
                        Some((index, *node))
                    }
                    _ => None,
                });
            let Some((formatting_index, formatting_element)) = found else {
                self.any_other_end_tag(subject);
                return;
            };
            let Some(formatting_position) = self
                .open
                .iter()
                .rposition(|&node| node == formatting_element)
            else {
                self.parse_error();
                self.formatting.remove(formatting_index);
                return;
            };
            if !self.in_scope(Scope::Default, |node| node == formatting_element) {
                self.parse_error();
                return;
            }
            if self.current_node() != Some(formatting_element) {
                self.parse_error();
            }
            let furthest = self
                .open
                .iter()
                .enumerate()
                .skip(formatting_position.saturating_add(1))
                .find(|(_, node)| self.is_special(**node))
                .map(|(index, node)| (index, *node));
            let Some((furthest_position, furthest_block)) = furthest else {
                self.pop_to(formatting_position);
                self.formatting.remove(formatting_index);
                return;
            };
            let Some(&common_ancestor) = formatting_position
                .checked_sub(1)
                .and_then(|index| self.open.get(index))
            else {
                return;
            };
            // Das Lesezeichen: `None` steht für den Platz des
            // Formatierungselements, sonst „direkt hinter diesem Element“.
            let mut bookmark: Option<NodeId> = None;
            let mut last_node = furthest_block;
            let mut position = furthest_position;
            let mut inner = 0;
            loop {
                inner += 1;
                let Some(above) = position.checked_sub(1) else {
                    break;
                };
                position = above;
                let Some(&node) = self.open.get(position) else {
                    break;
                };
                if node == formatting_element {
                    break;
                }
                let mut entry = self.formatting_index(node);
                if inner > 3
                    && let Some(index) = entry
                {
                    self.formatting.remove(index);
                    entry = None;
                }
                let Some(index) = entry else {
                    self.open.remove(position);
                    continue;
                };
                let Some(Formatting::Element { tag, .. }) = self.formatting.get(index) else {
                    break;
                };
                let tag = tag.clone();
                let element = self.create_element_for_token(&tag, Namespace::Html, false);
                if let Some(entry) = self.formatting.get_mut(index) {
                    *entry = Formatting::Element { node: element, tag };
                }
                if let Some(open) = self.open.get_mut(position) {
                    *open = element;
                }
                if bookmark.is_none() {
                    bookmark = Some(element);
                }
                let appended = self.document.append(element, last_node);
                debug_assert!(appended.is_ok(), "Einfügen nach dem Standard gültig");
                last_node = element;
            }
            let place = self.appropriate_place(Some(common_ancestor));
            let Some(Location {
                parent: target,
                before,
            }) = self.adjusted_location(place)
            else {
                return;
            };
            self.document.remove(last_node);
            if !self
                .document
                .is_host_including_inclusive_ancestor(last_node, target)
                && !self.is_document_with_element(target)
                && before.is_none_or(|before| self.document.parent(before) == Some(target))
            {
                let inserted = self.document.insert_before(target, last_node, before);
                debug_assert!(inserted.is_ok(), "Einfügen nach dem Standard gültig");
            }
            let Some(index) = self.formatting_index(formatting_element) else {
                return;
            };
            let Some(Formatting::Element { tag, .. }) = self.formatting.get(index) else {
                return;
            };
            let tag = tag.clone();
            let new_element = self.create_element_for_token(&tag, Namespace::Html, false);
            self.formatting.remove(index);
            let insert_at = match bookmark {
                None => Some(index),
                Some(node) => self
                    .formatting_index(node)
                    .map(|index| index.saturating_add(1)),
            };
            let entry = Formatting::Element {
                node: new_element,
                tag,
            };
            match insert_at {
                Some(at) if at <= self.formatting.len() => self.formatting.insert(at, entry),
                _ => self.formatting.push(entry),
            }
            self.remove_from_stack(formatting_element);
            let below = self
                .open
                .iter()
                .rposition(|&node| node == furthest_block)
                .map_or(self.open.len(), |index| index.saturating_add(1));
            self.open.insert(below, new_element);
            while let Some(child) = self.document.first_child(furthest_block) {
                if self.document.append(new_element, child).is_err() {
                    break;
                }
            }
            if let Some(Location { parent, before }) = self.adjusted_location(Some(Location {
                parent: furthest_block,
                before: None,
            })) {
                let inserted = self.document.insert_before(parent, new_element, before);
                debug_assert!(inserted.is_ok(), "Einfügen nach dem Standard gültig");
            }
            let mut below = self
                .open
                .iter()
                .position(|&node| node == new_element)
                .and_then(|index| index.checked_add(1));
            while let Some(index) = below
                && let Some(&node) = self.open.get(index)
                && self.redirects.contains_key(&node)
            {
                if let Some(location) = self.adjusted_location(Some(Location {
                    parent: new_element,
                    before: None,
                })) {
                    self.redirects.insert(node, location);
                }
                below = index.checked_add(1);
            }
        }
    }

    /// „Any other end tag“ in „in body“.
    fn any_other_end_tag(&mut self, name: &DomString) {
        let mut index = self.open.len();
        while let Some(previous) = index.checked_sub(1) {
            index = previous;
            let Some(&node) = self.open.get(index) else {
                return;
            };
            if self.is_html_named(node, name) {
                self.generate_implied_end_tags(name.as_ascii());
                if self.current_node() != Some(node) {
                    self.parse_error();
                }
                self.pop_to(index);
                return;
            }
            if self.is_special(node) {
                self.parse_error();
                return;
            }
        }
    }
}
