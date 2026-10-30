//! Eigene Tests für Selektoren. Die WPT-Tests zu `querySelectorAll()` stehen
//! in `wpt.rs`.

use mb_base::{Budget, Cancel, CodePoint, DomString, Exhausted};
use mb_css::{Source, parse_component_value_list};
use mb_dom::Document;
use mb_html::{Options, ScriptingMode, parse_document};

use super::*;

fn parse_with(text: &str, namespaces: &Namespaces) -> Result<SelectorList, InvalidSelector> {
    SelectorList::parse(&Source::new(text), namespaces)
}

fn parse(text: &str) -> Result<SelectorList, InvalidSelector> {
    parse_with(text, &Namespaces::default())
}

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

/// Die IDs der Elemente, die die Liste im Dokument trifft; Elemente ohne ID
/// stehen als „-“ darin.
fn ids(document: &Document, list: &SelectorList) -> Vec<String> {
    list.scope_match(document.root(), &MatchingContext::new(document))
        .into_iter()
        .map(|node| {
            document
                .element(node)
                .and_then(|element| element.attribute("id"))
                .map_or_else(|| "-".to_owned(), DomString::to_string_lossy)
        })
        .collect()
}

fn select(document: &Document, selector: &str) -> Vec<String> {
    let list = parse(selector).unwrap_or_else(|_| panic!("ungültig: {selector}"));
    ids(document, &list)
}

#[test]
fn specificity() {
    // Die Beispiele aus „Calculating a selector’s specificity“ und ein paar
    // für die Pseudoklassen mit Argumenten.
    for (selector, expected) in [
        ("*", (0, 0, 0)),
        ("LI", (0, 0, 1)),
        ("UL LI", (0, 0, 2)),
        ("UL OL+LI", (0, 0, 3)),
        ("H1 + *[REL=up]", (0, 1, 1)),
        ("UL OL LI.red", (0, 1, 3)),
        ("LI.red.level", (0, 2, 1)),
        ("#x34y", (1, 0, 0)),
        ("#s12:not(FOO)", (1, 0, 1)),
        (".foo :is(.bar, #baz)", (1, 1, 0)),
        (":where(#a) .b", (0, 1, 0)),
        (":nth-child(2n of #a, .b)", (1, 1, 0)),
        (":has(> #a, b)", (1, 0, 0)),
        ("::before", (0, 0, 1)),
        ("a:hover::before:hover", (0, 2, 2)),
        ("::slotted(.a)", (0, 1, 1)),
        (".a.a", (0, 2, 0)),
    ] {
        let list = parse(selector).unwrap_or_else(|_| panic!("ungültig: {selector}"));
        let specificity = list
            .selectors()
            .first()
            .expect("ein Selektor")
            .specificity();
        assert_eq!(
            (specificity.ids, specificity.classes, specificity.types),
            expected,
            "{selector}"
        );
    }
}

#[test]
fn validity() {
    for selector in [
        "a::before b",
        "::before::marker",
        "::before.a",
        "::before:first-child",
        ":has(:has(a))",
        ":not(::before)",
        ":nth-child(2n of)",
        ":nth-child(2n of ::before)",
        ":lang()",
        ":lang(1)",
        "[a=1]",
        "[a=b c]",
        "[a ~ = b]",
        "a|b",
        ":dir(ltr)",
        "::-webkit-thing()",
        "a,",
    ] {
        assert!(parse(selector).is_err(), "gültig: {selector}");
    }
    for selector in [
        ":is()",
        ":is(::before, a)",
        ":where(:unknown, a)",
        "::-webkit-scrollbar",
        "::before:hover",
        "::after:not(:hover)",
        "a:before",
        "::slotted(foo",
        ":has(+ a, ~ b > c)",
        "a:lang(\\*-CH, 'de')",
        "[a='b' i]",
        "[a|=b]",
        "|a",
        "*|*",
    ] {
        assert!(parse(selector).is_ok(), "ungültig: {selector}");
    }
    let mut namespaces = Namespaces::default();
    namespaces.declare(Some("svg"), "http://www.w3.org/2000/svg");
    assert!(parse_with("svg|rect", &namespaces).is_ok());
    assert!(parse_with("SVG|rect", &namespaces).is_err());
    assert!(parse_with("[svg|href]", &namespaces).is_ok());
}

#[test]
fn nesting_limit() {
    let nested = |depth: usize, name: &str| {
        format!(
            "{}a{}",
            format!(":{name}(").repeat(depth),
            ")".repeat(depth)
        )
    };
    assert!(parse(&nested(MAX_NESTING, "not")).is_ok());
    assert!(parse(&nested(MAX_NESTING.saturating_add(1), "not")).is_err());
    // In `:is()` fällt das zu tief Verschachtelte weg, die Liste bleibt
    // gültig.
    let deep = parse(&nested(10_000, "is")).expect("gültig");
    let document = document("<a></a>");
    assert!(ids(&document, &deep).is_empty());
}

#[test]
fn long_selectors_do_not_recurse() {
    let document = document("<div><p id=t></p></div>");
    let selector = format!("{}p", "div ".repeat(100_000));
    assert!(select(&document, &selector).is_empty());
    let selector = format!("{}p", "* ".repeat(100_000));
    assert!(select(&document, &selector).is_empty());
    assert_eq!(select(&document, "* * div p"), ["t"]);
}

#[test]
fn combinators() {
    let document = document(
        "<div id=a class='x Y'>\
           <p id=b lang=de-CH><span id=c></span></p>\
           <p id=d></p>\
           <svg id=e><foreignObject id=f></foreignObject></svg>\
         </div>",
    );
    assert_eq!(select(&document, "div > p"), ["b", "d"]);
    assert_eq!(select(&document, "div p + p"), ["d"]);
    assert_eq!(select(&document, "p ~ svg"), ["e"]);
    assert_eq!(select(&document, "p ~ * > *"), ["f"]);
    assert_eq!(select(&document, "#a > :not(p)"), ["e"]);
    assert_eq!(select(&document, ":is(p, svg) > *"), ["c", "f"]);
}

#[test]
fn backtracking() {
    // Rücksprünge, bei denen die Abkürzungen nicht zu früh aufgeben dürfen.
    let document = document(
        "<div class=a><div class=b><div class=a><p id=t class=c></p></div></div></div>\
         <div class=s></div>\
         <div class=t><div class=t><p id=u></p></div></div>\
         <section><div class=x></div><div class=y><p id=v></p></div></section>",
    );
    assert_eq!(select(&document, ".a > .b .c"), ["t"]);
    assert_eq!(select(&document, ".a > .b > .c"), Vec::<String>::new());
    assert_eq!(select(&document, ".s + .t p"), ["u"]);
    assert_eq!(select(&document, ".s ~ .t > .t p"), ["u"]);
    assert_eq!(select(&document, ".x ~ .y > p"), ["v"]);
    assert_eq!(select(&document, ".x + .z > p"), Vec::<String>::new());
    assert_eq!(select(&document, "section .x + * p"), ["v"]);
}

#[test]
fn html_case_rules() {
    let document = document(
        "<!DOCTYPE html><div id=a class=Foo lang=DE title=Bar>\
         <svg id=s><foreignObject id=f viewBox='0 0 1 1'></foreignObject></svg></div>",
    );
    // Typselektoren und Attributnamen für HTML-Elemente in Kleinbuchstaben.
    assert_eq!(select(&document, "DIV"), ["a"]);
    assert_eq!(select(&document, "[TITLE]"), ["a"]);
    assert_eq!(select(&document, "foreignObject"), ["f"]);
    assert!(select(&document, "foreignobject").is_empty());
    assert_eq!(select(&document, "[viewBox]"), ["f"]);
    assert!(select(&document, "[viewbox]").is_empty());
    // Werte: `lang` ohne Unterschied, `title` und `class` genau.
    assert_eq!(select(&document, "[lang=de]"), ["a"]);
    assert!(select(&document, "[lang=de s]").is_empty());
    assert!(select(&document, "[title=bar]").is_empty());
    assert_eq!(select(&document, "[title=bar i]"), ["a"]);
    assert!(select(&document, ".foo").is_empty());

    // Im Quirks-Modus gelten Klassen und IDs ohne Unterschied.
    let quirks = self::document("<div id=a class=Foo></div>");
    assert_eq!(select(&quirks, ".foo"), ["a"]);
    assert_eq!(select(&quirks, "#A"), ["a"]);
    assert!(select(&quirks, "[class=foo]").is_empty());
}

#[test]
fn default_namespace() {
    let document = document("<div id=a class=x><svg id=s class=x></svg></div>");
    let mut namespaces = Namespaces::default();
    namespaces.declare(None, "http://www.w3.org/2000/svg");
    let select = |selector: &str| {
        let list = parse_with(selector, &namespaces).expect(selector);
        ids(&document, &list)
    };
    assert_eq!(select(".x"), ["s"]);
    assert_eq!(select("div"), Vec::<String>::new());
    // Für das Subjekt in `:is()` gilt der Standard-Namensraum nur mit
    // eigenem Typselektor.
    assert_eq!(select("*|*:is(.x)"), ["a", "s"]);
    assert_eq!(select("*|*:is(*.x)"), ["s"]);
    assert_eq!(select("*|*:not(.x)"), ["-", "-", "-"]);
}

#[test]
fn structural_pseudo_classes() {
    let document = document(
        "<ul id=l><li id=a class=k></li><li id=b></li><li id=c class=k></li><li id=d></li></ul>",
    );
    assert_eq!(select(&document, "li:nth-child(odd)"), ["a", "c"]);
    assert_eq!(select(&document, "li:nth-child(-n+2)"), ["a", "b"]);
    assert_eq!(select(&document, ":nth-child(2 of .k)"), ["c"]);
    assert_eq!(select(&document, ":nth-last-child(1 of .k)"), ["c"]);
    assert_eq!(
        select(&document, "li:first-child, li:last-child"),
        ["a", "d"]
    );
    assert_eq!(select(&document, "li:nth-last-of-type(2)"), ["c"]);
    assert_eq!(select(&document, "ul:only-child"), ["l"]);
    assert_eq!(select(&document, "ul:only-of-type"), ["l"]);
    assert_eq!(select(&document, ":root"), ["-"]);
    assert_eq!(select(&document, ":scope"), ["-"]);
}

#[test]
fn links() {
    let document = document(
        "<a id=a href=x></a><a id=b></a><area id=c href=''><link id=d href=x>\
         <svg><a id=e href=x></a><a id=f xlink:href=x></a><a id=g></a></svg>",
    );
    assert_eq!(select(&document, ":any-link"), ["a", "c", "e", "f"]);
    assert_eq!(select(&document, ":link"), ["a", "c", "e", "f"]);
    assert!(select(&document, ":visited").is_empty());
}

#[test]
fn has() {
    let document =
        document("<div id=a><p id=b><span id=c></span></p><p id=d></p><em id=e></em></div>");
    assert_eq!(select(&document, "p:has(> span)"), ["b"]);
    assert_eq!(select(&document, "div:has(#c), p:has(#c)"), ["a", "b"]);
    assert_eq!(select(&document, "p:has(+ p)"), ["b"]);
    assert_eq!(select(&document, "p:has(~ em)"), ["b", "d"]);
    assert_eq!(select(&document, "div > :has(~ p)"), ["b"]);
    // Auch das leere `head` hat keine Nachkommen.
    assert_eq!(select(&document, "*:not(:has(*))"), ["-", "c", "d", "e"]);
}

#[test]
fn lang() {
    let document = document(
        "<html lang=en><div id=a lang=de-CH><p id=b></p></div>\
         <div id=c lang=''><p id=d></p></div><svg id=s lang=fr></svg></html>",
    );
    assert_eq!(select(&document, "p:lang(de)"), ["b"]);
    assert_eq!(select(&document, "p:lang(\\*-CH)"), ["b"]);
    assert_eq!(select(&document, "p:lang('')"), ["d"]);
    assert_eq!(select(&document, "svg:lang(fr)"), ["s"]);
    assert_eq!(select(&document, "div:lang(en, de)"), ["a"]);
}

#[test]
fn pseudo_elements() {
    let document = document("<p id=a></p>");
    let context = MatchingContext::new(&document);
    let node = parse("p")
        .expect("p")
        .scope_match(document.root(), &context)
        .first()
        .copied()
        .expect("ein p");
    let selector = |text: &str| {
        parse(text)
            .expect(text)
            .selectors()
            .first()
            .expect("ein Selektor")
            .clone()
    };
    assert!(!selector("p::before").matches(node, &context));
    assert!(selector("p::before").matches_pseudo_element(node, PseudoElement::Before, &context));
    assert!(!selector("p::before").matches_pseudo_element(node, PseudoElement::After, &context));
    assert!(selector("::after:not(:hover)").matches_pseudo_element(
        node,
        PseudoElement::After,
        &context
    ));
    assert!(!selector("::before:hover").matches_pseudo_element(
        node,
        PseudoElement::Before,
        &context
    ));
    assert_eq!(
        selector("a:first-line").pseudo_element(),
        Some(PseudoElement::FirstLine)
    );
    assert_eq!(selector("::-webkit-thumb").pseudo_element(), None);
}

#[test]
fn headings() {
    let document = document(
        "<h1 id=a></h1><section headingoffset=1><h1 id=b></h1>\
         <div headingreset><h2 id=c></h2></div><h6 id=d></h6></section>\
         <section headingoffset=20><h3 id=e></h3></section><p id=f></p>",
    );
    assert_eq!(select(&document, ":heading"), ["a", "b", "c", "d", "e"]);
    assert_eq!(select(&document, ":heading(1)"), ["a"]);
    assert_eq!(select(&document, ":heading(2)"), ["b", "c"]);
    assert_eq!(select(&document, ":heading(7, 9)"), ["d", "e"]);
    assert!(parse(":heading(1.5)").is_err());
    assert!(parse(":heading()").is_err());
    // Ohne Skript gibt es keine offenen Popover und keine modalen Dialoge.
    assert!(select(&document, ":popover-open, :modal, :autofill").is_empty());
    let backdrop = parse("::backdrop").expect("::backdrop");
    let selector = backdrop.selectors().first().expect("ein Selektor");
    assert_eq!(selector.pseudo_element(), None);
}

#[test]
fn nesting_selector() {
    let specificities = |list: &SelectorList| -> Vec<(u32, u32, u32)> {
        list.selectors()
            .iter()
            .map(|selector| {
                let specificity = selector.specificity();
                (specificity.ids, specificity.classes, specificity.types)
            })
            .collect()
    };
    // Ganz oben steht `&` für `:scope`, mit der Spezifität null.
    let top = parse("& > p, &").expect("gültig");
    assert_eq!(specificities(&top), [(0, 0, 1), (0, 0, 0)]);

    let nested = |text: &str, parent: &Arc<SelectorList>| {
        let values = parse_component_value_list(&Source::new(text));
        SelectorList::from_values_nested(values.values(), &Namespaces::default(), parent)
    };
    let document =
        document("<div class=a id=a><p id=p><span id=s></span></p></div><b id=b><i id=i></i></b>");
    // `&` hat die größte Spezifität der Liste darüber. Ohne Kombinator am
    // Anfang und ohne `&` ist ein Selektor relativ zu `&`.
    let parent = Arc::new(parse(".a, #b").expect("gültig"));
    for (text, expected, specificity) in [
        ("span", &["s"][..], (1, 0, 1)),
        ("> p", &["p"], (1, 0, 1)),
        ("& > i", &["i"], (1, 0, 1)),
        ("&.a", &["a"], (1, 1, 0)),
        ("body &", &["a", "b"], (1, 0, 1)),
        ("p &", &[], (1, 0, 1)),
        (":where(&) span", &["s"], (0, 0, 1)),
    ] {
        let list = nested(text, &parent).unwrap_or_else(|_| panic!("ungültig: {text}"));
        assert_eq!(ids(&document, &list), expected, "{text}");
        assert_eq!(specificities(&list), [specificity], "{text}");
    }
    // Selektoren mit Pseudoelement in der Liste darüber treffen über `&`
    // nichts.
    let parent = Arc::new(parse(".a::before, #b").expect("gültig"));
    assert_eq!(
        ids(&document, &nested("i", &parent).expect("gültig")),
        ["i"]
    );
    assert!(ids(&document, &nested("span", &parent).expect("gültig")).is_empty());

    // Jede Ebene zählt für die Grenze der Verschachtelung.
    let mut list = Arc::new(parse("a").expect("gültig"));
    for level in 1..=MAX_NESTING {
        let next = nested("& a", &list).unwrap_or_else(|_| panic!("Ebene {level}"));
        list = Arc::new(next);
    }
    assert!(nested("& a", &list).is_err());
    assert!(ids(&document, &list).is_empty());
}

/// Wie [`select`], aber in einem Durchgang mit diesem Budget.
fn select_within(document: &Document, selector: &str, budget: &Budget) -> Vec<String> {
    let list = parse(selector).unwrap_or_else(|_| panic!("ungültig: {selector}"));
    let pass = MatchingPass::new(budget);
    let context = MatchingContext {
        pass: Some(&pass),
        ..MatchingContext::new(document)
    };
    list.scope_match(document.root(), &context)
        .into_iter()
        .map(|node| {
            document
                .element(node)
                .and_then(|element| element.attribute("id"))
                .map_or_else(|| "-".to_owned(), DomString::to_string_lossy)
        })
        .collect()
}

#[test]
fn nested_nth_child_of_stays_linear() {
    // Ohne gemerkte Stellen wüchse die Arbeit mit jeder Ebene um den Faktor
    // der Geschwister; 20 Ebenen über 30 Geschwister wären nie fertig.
    let document = document(&format!(
        "<!DOCTYPE html><body>{}",
        "<div></div>".repeat(30)
    ));
    let mut selector = "*".to_owned();
    for _ in 0..20 {
        selector = format!(":nth-child(n of {selector})");
    }
    let budget = Budget::new(20_000);
    // `html`, `head`, `body` und die 30 `div`.
    assert_eq!(select_within(&document, &selector, &budget).len(), 33);
    assert_eq!(budget.exhausted(), None);
}

#[test]
fn nested_is_with_combinators_stays_linear() {
    let document = document(&format!(
        "<!DOCTYPE html><body>{}<p id=t></p>{}",
        "<div>".repeat(150),
        "</div>".repeat(150)
    ));
    let mut selector = "div".to_owned();
    for _ in 0..10 {
        selector = format!(":is({selector} div)");
    }
    let budget = Budget::new(100_000);
    assert_eq!(
        select_within(&document, &format!("{selector} p"), &budget),
        ["t"]
    );
    assert_eq!(budget.exhausted(), None);
}

#[test]
fn positions_are_counted_per_parent() {
    let document = document(
        "<ul><li id=a></li><li id=b class=k></li><li id=c class=k></li></ul>\
         <ol><li id=d class=k></li><li id=e></li><p id=f></p><li id=g class=k></li></ol>",
    );
    let budget = Budget::unlimited();
    assert_eq!(
        select_within(&document, ":nth-child(1 of .k)", &budget),
        ["b", "d"]
    );
    assert_eq!(
        select_within(&document, ":nth-last-child(1 of .k)", &budget),
        ["c", "g"]
    );
    assert_eq!(
        select_within(&document, "li:nth-last-of-type(1), p:only-of-type", &budget),
        ["c", "f", "g"]
    );
    assert_eq!(
        select_within(&document, "li:nth-child(2), li:nth-child(4)", &budget),
        ["b", "e", "g"]
    );
}

#[test]
fn spent_budget_stops_matching() {
    let document = document(&format!(
        "<!DOCTYPE html><body>{}",
        "<div></div>".repeat(100)
    ));
    let budget = Budget::new(50);
    select_within(&document, "div:nth-child(odd)", &budget);
    assert_eq!(budget.exhausted(), Some(Exhausted::Steps));

    let cancel = Cancel::new();
    cancel.cancel();
    let budget = Budget::with_cancel(u64::MAX, cancel);
    assert!(select_within(&document, "div", &budget).is_empty());
    assert_eq!(budget.exhausted(), Some(Exhausted::Cancelled));
}
