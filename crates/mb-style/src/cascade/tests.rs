//! Tests für die Kaskade: die Beispiele aus CSS Cascade 5, CSS Nesting und
//! CSS Logical, nachgebaute WPT-Tests aus `css/css-cascade` und das
//! Stylesheet der Benutzeragenten. Wo die WPT-Tests `z-index` prüfen, das es
//! hier noch nicht gibt, steht `width`.

use mb_base::CodePoint;
use mb_css::{Source, parse_component_value_list};
use mb_dom::{Document, NodeId};
use mb_html::{Options, ScriptingMode, parse_document};

use super::*;
use crate::declarations::parse_declaration_block;
use crate::properties::{Direction, PropertyId, TextOrientation, WritingMode};
use crate::values::{ParseContext, ToCss};

fn document(html: &str) -> Document {
    let mut document = Document::new();
    parse_document(
        &mut document,
        html.chars().map(CodePoint::from),
        Options {
            scripting: ScriptingMode::Disabled,
            collect_errors: false,
        },
    );
    document.perform_microtask_checkpoint();
    document
}

/// Ein Dokument mit seinen Stylesheets.
struct Page {
    document: Document,
    stylist: Stylist,
}

/// `user_agent` nimmt die Stylesheets des HTML Standard dazu.
fn load(html: &str, sheets: &[(Origin, &str)], user_agent: bool) -> Page {
    let document = document(html);
    let mut builder = StylistBuilder::new();
    if user_agent {
        builder.add_user_agent_sheets(DocumentInfo {
            quirks_mode: document.quirks_mode(),
            html_document: true,
            iso_8859_8: false,
        });
    }
    for (origin, text) in sheets {
        builder.add(*origin, &StyleSheet::parse(text, ParseContext::default()));
    }
    Page {
        document,
        stylist: builder.build(),
    }
}

fn author(html: &str, css: &str) -> Page {
    load(html, &[(Origin::Author, css)], false)
}

/// Die Zuordnung der logischen Eigenschaften nach den kaskadierten Werten;
/// für die Tests reicht das ohne Vererbung.
fn flow(values: &CascadedValues<'_>) -> FlowMode {
    let specified = |longhand| values.get(longhand).map(|cascaded| cascaded.value);
    let writing_mode = match specified(Longhand::WritingMode) {
        Some(CascadedValue::Specified(SpecifiedValue::WritingMode(mode))) => *mode,
        _ => WritingMode::HorizontalTb,
    };
    let direction = match specified(Longhand::Direction) {
        Some(CascadedValue::Specified(SpecifiedValue::Direction(direction))) => *direction,
        _ => Direction::Ltr,
    };
    let orientation = match specified(Longhand::TextOrientation) {
        Some(CascadedValue::Specified(SpecifiedValue::TextOrientation(orientation))) => {
            *orientation
        }
        _ => TextOrientation::Mixed,
    };
    FlowMode::new(writing_mode, direction, orientation)
}

/// Ein kaskadierter Wert als Text; „-“, wenn es keinen gibt.
fn describe(cascaded: Option<Cascaded<'_>>) -> String {
    match cascaded.map(|cascaded| cascaded.value) {
        None => "-".to_owned(),
        Some(CascadedValue::Specified(value)) => value.to_css_string(),
        Some(CascadedValue::Initial) => "initial".to_owned(),
        Some(CascadedValue::Inherit) => "inherit".to_owned(),
        Some(CascadedValue::Unset) => "unset".to_owned(),
    }
}

/// Ein Wert für eine Langform, gelesen und wieder serialisiert, zum
/// Vergleichen.
fn css(longhand: Longhand, text: &str) -> String {
    if text == "-" {
        return text.to_owned();
    }
    let values = parse_component_value_list(&Source::new(text));
    let parsed = PropertyId::Longhand(longhand).parse(values.values(), ParseContext::default());
    let declaration = parsed
        .as_deref()
        .and_then(<[PropertyDeclaration]>::first)
        .unwrap_or_else(|| panic!("{}: {text}", longhand.name()));
    declaration.value.to_css_string()
}

impl Page {
    fn element(&self, id: &str) -> NodeId {
        let document = &self.document;
        let mut current = Some(document.root());
        while let Some(node) = current {
            let element_id = document
                .element(node)
                .and_then(|element| element.attribute("id"));
            if element_id.is_some_and(|value| *value == id) {
                return node;
            }
            current = document.following(node, document.root());
        }
        panic!("kein Element mit der ID {id}");
    }

    /// Kaskadiert für das Element mit der ID oder eines seiner
    /// Pseudoelemente, mit dem Attribut `style`.
    fn cascade<R>(
        &self,
        id: &str,
        pseudo_element: Option<PseudoElement>,
        f: impl FnOnce(&CascadedValues<'_>) -> R,
    ) -> R {
        let element = self.element(id);
        let attribute = self
            .document
            .element(element)
            .and_then(|element| element.attribute("style"))
            .map(|text| {
                parse_declaration_block(
                    &Source::new(&text.to_string_lossy()),
                    ParseContext::default(),
                )
            });
        let context = MatchingContext::new(&self.document);
        let matched =
            self.stylist
                .matching(&context, element, pseudo_element, attribute.as_deref());
        f(&matched.cascade(flow))
    }

    fn value(&self, id: &str, longhand: Longhand) -> String {
        self.cascade(id, None, |values| describe(values.get(longhand)))
    }

    /// Prüft Werte als (ID, Langform, erwarteter Wert).
    fn check(&self, cases: &[(&str, Longhand, &str)]) {
        let failures: Vec<String> = cases
            .iter()
            .filter_map(|&(id, longhand, expected)| {
                let actual = self.value(id, longhand);
                let expected = css(longhand, expected);
                (actual != expected)
                    .then(|| format!("#{id} {}: {actual}, erwartet {expected}", longhand.name()))
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

#[test]
fn origins_and_importance() {
    // Das Beispiel aus „Important Declarations“ in CSS Cascade 5.
    let user = "p { text-indent: 1em !important }
        p { font-style: italic !important }
        p { font-size: 18pt }";
    let author = "p { text-indent: 1.5em !important }
        p { font: normal 12pt sans-serif !important }
        p { font-size: 24pt }";
    let page = load(
        "<p id=p>",
        &[(Origin::User, user), (Origin::Author, author)],
        false,
    );
    page.check(&[
        ("p", Longhand::TextIndent, "1em"),
        ("p", Longhand::FontStyle, "italic"),
        ("p", Longhand::FontSize, "12pt"),
    ]);

    // Ohne Wichtigkeit gewinnt der Autor vor dem Nutzer, unabhängig von
    // der Spezifität; die Benutzeragenten gewinnen mit `!important` vor
    // allen (`input[type=hidden i]` im HTML Standard).
    let page = load(
        "<div id=d></div><input type=hidden id=h>",
        &[
            (
                Origin::User,
                "#d { color: red } #h { display: block !important }",
            ),
            (
                Origin::Author,
                "div { color: green } #h { display: inline !important }",
            ),
        ],
        true,
    );
    page.check(&[
        ("d", Longhand::Color, "green"),
        ("h", Longhand::Display, "none"),
    ]);
}

#[test]
fn specificity_and_order() {
    let page = author(
        "<div id=a class=b>",
        "#a { color: green } .b { color: red }
         .b { margin-top: 1px; margin-top: 2px } div { margin-top: 3px }
         div { width: 1px } div { width: 2px }",
    );
    page.check(&[
        ("a", Longhand::Color, "green"),
        ("a", Longhand::MarginTop, "2px"),
        ("a", Longhand::Width, "2px"),
        ("a", Longhand::Height, "-"),
    ]);
}

#[test]
fn rule_index() {
    // Die Regeln werden nach ID, Klasse und Typ vorsortiert, ohne
    // Unterschied zwischen ASCII-Groß- und -Kleinbuchstaben. Ob eine Regel
    // trifft, entscheidet weiter der Abgleich: Klassen unterscheiden die
    // Schreibung außer im *quirks mode*.
    let css = "#t { color: green } .b { margin-top: 1px } SPAN { margin-left: 2px }
        [id] { width: 3px } .B { margin-right: 4px } #T { height: 5px }";
    let html = "<span id=t class='a  b'></span>";
    let standard = author(&format!("<!DOCTYPE html>{html}"), css);
    standard.check(&[
        ("t", Longhand::Color, "green"),
        ("t", Longhand::MarginTop, "1px"),
        ("t", Longhand::MarginLeft, "2px"),
        ("t", Longhand::Width, "3px"),
        ("t", Longhand::MarginRight, "-"),
        ("t", Longhand::Height, "-"),
    ]);
    let quirks = author(html, css);
    quirks.check(&[
        ("t", Longhand::MarginRight, "4px"),
        ("t", Longhand::Height, "5px"),
    ]);
}

#[test]
fn layers() {
    let page = author(
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div>
         <div id=e></div><div id=f></div><div id=g></div>",
        "@layer one, two;
         @layer two { #a { color: green } #b { color: red !important } }
         @layer one { #a { color: red } #b { color: green !important } }
         #b { color: red !important }
         #c { color: green } @layer three { #c { color: red } }
         @layer four { #d { color: green } @layer x { #d { color: red } } }
         @layer five.x { #e { color: red } } @layer five { #e { color: green } }
         @layer { #f { color: red } } @layer { #f { color: green } }
         @layer six { #g { color: red } } @layer seven { div { color: green } }",
    );
    page.check(&[
        // Die spätere Ebene gewinnt, bei `!important` die frühere, und
        // wichtige Deklarationen in Ebenen vor denen ohne Ebene.
        ("a", Longhand::Color, "green"),
        ("b", Longhand::Color, "green"),
        // Ohne Ebene gewinnt vor jeder Ebene.
        ("c", Longhand::Color, "green"),
        // Die Regeln einer Ebene selbst kommen nach ihren Unterebenen.
        ("d", Longhand::Color, "green"),
        ("e", Longhand::Color, "green"),
        // Anonyme Ebenen sind jede für sich.
        ("f", Longhand::Color, "green"),
        // Die Ebene geht vor der Spezifität.
        ("g", Longhand::Color, "green"),
    ]);
}

#[test]
fn revert() {
    let page = load(
        "<div id=a></div><address id=b></address><address id=c></address><div id=d></div>",
        &[
            (Origin::UserAgent, "#d { color: red; color: revert }"),
            (
                Origin::User,
                "#a { color: green } #c { font-style: normal }",
            ),
            (
                Origin::Author,
                "#a { color: red; color: revert }
                 #b, #c { font-style: oblique; font-style: revert }",
            ),
        ],
        true,
    );
    page.check(&[
        // Zurück zum Nutzer, dann zu den Benutzeragenten.
        ("a", Longhand::Color, "green"),
        ("b", Longhand::FontStyle, "italic"),
        ("c", Longhand::FontStyle, "normal"),
        // Bei den Benutzeragenten wie `unset`.
        ("d", Longhand::Color, "unset"),
    ]);
}

#[test]
fn presentational_hints() {
    // `pre[wrap]` steht im HTML Standard als *presentational hint*: Er
    // gewinnt vor dem Nutzer und den Benutzeragenten (`white-space: pre`),
    // verliert gegen den Autor und fällt mit `revert` im Autor weg.
    let page = load(
        "<pre id=a></pre><pre wrap id=b></pre><pre wrap id=c></pre><pre wrap id=d></pre>",
        &[
            (Origin::User, "pre { text-wrap-mode: nowrap }"),
            (
                Origin::Author,
                "#c { text-wrap-mode: nowrap } #d { text-wrap-mode: revert }",
            ),
        ],
        true,
    );
    page.check(&[
        ("a", Longhand::TextWrapMode, "nowrap"),
        ("b", Longhand::TextWrapMode, "wrap"),
        ("c", Longhand::TextWrapMode, "nowrap"),
        ("d", Longhand::TextWrapMode, "nowrap"),
        ("b", Longhand::WhiteSpaceCollapse, "preserve"),
    ]);
    // Die Spezifität der Hinweise ist null, es entscheidet die Reihenfolge.
    let page = load(
        "<pre id=e></pre>",
        &[(
            Origin::PresentationalHint,
            "pre#e { text-wrap-mode: nowrap } pre { text-wrap-mode: wrap }",
        )],
        false,
    );
    page.check(&[("e", Longhand::TextWrapMode, "wrap")]);
}

#[test]
fn revert_layer() {
    // revert-layer-001, -002, -005, -007
    let page = author(
        "<div id=t1></div><div id=t2></div><div id=t5></div><div id=t7></div>",
        "@layer { #t1 { background-color: green } }
         @layer { #t1 { background-color: red; background-color: revert-layer } }
         @layer { #t2 { background-color: green } }
         #t2 { background-color: red; background-color: revert-layer }
         @layer { #t5 { background-color: green } }
         @layer { #t5 { background-color: red; background-color: red !important; background-color: revert-layer !important } }
         @layer { #t5 { background-color: red; background-color: red !important } }
         @layer { #t7 { background-color: green } }
         @layer { #t7 { background-color: red; background-color: revert-layer } }
         @layer { #t7 { background-color: red; background-color: revert-layer } }
         @layer { #t7 { background-color: red; background-color: revert-layer } }",
    );
    page.check(&[
        ("t1", Longhand::BackgroundColor, "green"),
        ("t2", Longhand::BackgroundColor, "green"),
        ("t5", Longhand::BackgroundColor, "green"),
        ("t7", Longhand::BackgroundColor, "green"),
    ]);
    // revert-layer-006: ohne frühere Ebene wie `revert`.
    let page = load(
        "<div id=inner></div>",
        &[(
            Origin::Author,
            "#inner { display: inline; display: revert-layer }",
        )],
        true,
    );
    page.check(&[("inner", Longhand::Display, "block")]);
    // revert-layer-009 und -012: im Attribut `style`.
    let page = author(
        "<div id=t9 style='background-color: red; background-color: revert-layer'></div>
         <div id=t12 style='background-color: red !important; background-color: revert-layer !important'></div>",
        "#t9 { background-color: green } #t12 { background-color: green !important }",
    );
    page.check(&[
        ("t9", Longhand::BackgroundColor, "green"),
        ("t12", Longhand::BackgroundColor, "green"),
    ]);
}

#[test]
fn revert_rule() {
    // revert-rule-basic, -important, -layer und -revert-layer
    let page = author(
        "<div id=b1></div><div id=b2></div><div id=b3 style='color:red; color:revert-rule'></div>
         <div id=b4></div><div id=i1></div><div id=i2></div><div id=i3></div>
         <div id=l1></div><div id=l2></div><div id=r1></div><div id=r2></div>",
        "#b1 { color: green }
         #b1 { color: red; color: revert-rule }
         #b2 { color: red }
         #b2#b2#b2 { color: red; color: revert-rule }
         #b2#b2 { color: green }
         #b3 { color: red }
         #b3 { color: green }
         #b4 { width: 1px; }
         #b4 { width: 2px; }
         #b4 { width: 3px; width: revert-rule; }
         #b4 { width: 3px; width: revert-rule; }
         #b4 { width: 3px; width: revert-rule; }
         #i1 { color: revert-rule !important }
         #i1 { color: green }
         @layer { #i2 { color: revert-rule !important } }
         #i2 { color: green }
         #i3 { color: green }
         #i3 { color: revert-rule !important }
         @layer { #l1 { color: green } }
         @layer { #l1 { color: red; color: revert-rule } }
         @layer { #l2 { color: red } }
         @layer { #l2 { color: green } #l2 { color: red; color: revert-rule } }
         @layer { #r1 { color: green } }
         @layer { #r1 { color: red } #r1 { color: revert-layer } }
         @layer { #r1 { color: revert-rule; background-color: green } }
         @layer { #r2 { color: green } }
         @layer { #r2 { color: revert-rule; background-color: green } }
         @layer { #r2 { color: red } #r2 { color: revert-layer; background-color: revert-rule } }",
    );
    page.check(&[
        ("b1", Longhand::Color, "green"),
        ("b2", Longhand::Color, "green"),
        ("b3", Longhand::Color, "green"),
        ("b4", Longhand::Width, "2px"),
        ("i1", Longhand::Color, "green"),
        ("i2", Longhand::Color, "green"),
        ("i3", Longhand::Color, "green"),
        ("l1", Longhand::Color, "green"),
        ("l2", Longhand::Color, "green"),
        ("r1", Longhand::Color, "green"),
        ("r1", Longhand::BackgroundColor, "green"),
        ("r2", Longhand::Color, "green"),
        ("r2", Longhand::BackgroundColor, "green"),
    ]);
}

#[test]
fn style_attribute() {
    let page = author(
        "<div id=a style='color: green'></div>
         <div id=b style='color: red'></div>
         <div id=c style='color: green !important'></div>
         <div id=d style='color: red'></div>",
        "#a { color: red } #b { color: green !important } #c { color: red !important }
         #d::before { color: green }",
    );
    page.check(&[
        ("a", Longhand::Color, "green"),
        ("b", Longhand::Color, "green"),
        ("c", Longhand::Color, "green"),
    ]);
    // Für Pseudoelemente gilt das Attribut nicht.
    let before = page.cascade("d", Some(PseudoElement::Before), |values| {
        describe(values.get(Longhand::Color))
    });
    assert_eq!(before, css(Longhand::Color, "green"));
}

#[test]
fn nesting() {
    let page = author(
        "<article id=a></article><div id=b></div><div class=p id=c></div>
         <p><span id=d class=x></span></p>
         <div class=q><span id=e></span><b><span id=f></span></b></div>
         <div class=r id=g></div>",
        // Das Beispiel aus „Mixing Nesting Rules and Declarations“: die
        // Deklarationen nach der verschachtelten Regel kommen nach ihr.
        "article { color: green; & { color: blue; } color: red; }
         div#b { color: red; & { color: green } }
         div { color: green } .p { :where(&) { color: red } }
         #a, p { & span { color: green } } p span.x { color: red }
         .q { > span { color: green } }
         .r { color: green; @layer z { color: red } }",
    );
    page.check(&[
        ("a", Longhand::Color, "red"),
        // `&` hat die Spezifität der Elternregel, und verschachtelte Regeln
        // kommen nach ihr.
        ("b", Longhand::Color, "green"),
        ("c", Longhand::Color, "green"),
        // Die Spezifität von `&` ist die größte der Liste darüber, hier die
        // von `#a`.
        ("d", Longhand::Color, "green"),
        // Relative Selektoren hängen an `&`.
        ("e", Longhand::Color, "green"),
        ("f", Longhand::Color, "-"),
        // Deklarationen in `@layer` gehören zur Ebene.
        ("g", Longhand::Color, "green"),
    ]);
}

#[test]
fn repeated_nesting_selectors() {
    // `&&` gleicht die Liste darüber zweimal ab. Ohne Speicher für die
    // Ergebnisse von `&` verdoppelte sich die Arbeit mit jeder Ebene; bei 60
    // Ebenen wäre sie nicht mehr abzuwarten.
    let depth = 60;
    let css = format!(
        "div {{ {} color: green {} }}",
        "&& { ".repeat(depth),
        "} ".repeat(depth)
    );
    let page = author("<div id=a></div><p id=b></p>", &css);
    page.check(&[("a", Longhand::Color, "green"), ("b", Longhand::Color, "-")]);
}

#[test]
fn logical_properties() {
    let page = author(
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div>",
        "#a { margin-left: 1px; margin-inline-start: 2px }
         #b { margin-inline-start: 2px; margin-left: 1px }
         #c { margin-left: 1px; margin-inline-start: 2px; direction: rtl }
         #d { writing-mode: vertical-rl; inline-size: 5px; width: 3px }
         #e { margin-inline-start: inherit }",
    );
    page.check(&[
        ("a", Longhand::MarginLeft, "2px"),
        ("a", Longhand::MarginInlineStart, "2px"),
        ("b", Longhand::MarginLeft, "1px"),
        ("c", Longhand::MarginLeft, "1px"),
        ("c", Longhand::MarginRight, "2px"),
        ("c", Longhand::MarginInlineStart, "2px"),
        ("d", Longhand::Width, "3px"),
        ("d", Longhand::Height, "5px"),
    ]);
    // `inherit` meint die logische Langform beim Elternelement.
    let cascaded = page.cascade("e", None, |values| {
        values
            .get(Longhand::MarginLeft)
            .map(|cascaded| (cascaded.longhand, cascaded.value == CascadedValue::Inherit))
    });
    assert_eq!(cascaded, Some((Longhand::MarginInlineStart, true)));
}

#[test]
fn user_agent_sheet() {
    let html = "<head id=h></head><p id=p></p><form id=f></form><address id=a></address>";
    let standard = load(&format!("<!DOCTYPE html>{html}"), &[], true);
    standard.check(&[
        ("h", Longhand::Display, "none"),
        ("p", Longhand::Display, "block"),
        ("p", Longhand::MarginTop, "1em"),
        ("p", Longhand::MarginLeft, "-"),
        ("f", Longhand::MarginBottom, "-"),
        ("a", Longhand::FontStyle, "italic"),
    ]);
    // Im *quirks mode* hat `form` unten einen Rand.
    let quirks = load(html, &[], true);
    quirks.check(&[("f", Longhand::MarginBottom, "1em")]);
}
