//! Die Baumaufbau-Tests aus den WPT (`html/syntax/parsing/resources/*.dat`,
//! früher html5lib-tests `tree-construction`), mit und ohne Skripte.
//!
//! Gelesen und ausgegeben wird wie in `resources/test.js` (`parseDat` und
//! `serializeTree`), die Bedeutung der Abschnitte steht in
//! `resources/README.md`.
//!
//! Verglichen wird wie in `test.js` nur der Baum. Die README verlangt zwar
//! auch die richtige Zahl an Parse-Fehlern, die Dateien passen aber nicht
//! zum Standard: `#errors` nennt die Fehler des alten html5lib-Parsers,
//! `#new-errors` die Codes des heutigen Tokenizers, teils für denselben
//! Fehler doppelt (`FOO<!-- BAR --!>BAZ` erwartet drei statt zwei), teils
//! fehlen Fehler des Baumaufbaus (`\0filler\0text` in `div` erwartet zwei
//! statt vier, `<svg><title><![CDATA[x]]>` keinen statt zwei). Die Fehler
//! des Tokenizers prüfen dessen Tests mit Code und Stelle; für die Fehler
//! des Baumaufbaus gibt es unten eigene Fälle, nachgerechnet am Standard.

use std::collections::BTreeMap;

use mb_base::{CodePoint, DomString};
use mb_dom::{Document, Namespace, NodeId, NodeKind};

use super::{Options, ScriptingMode, parse_document, parse_fragment};
use crate::error::ErrorCode;

/// Diese Dateien erwarten, dass Skripte laufen und den Baum ändern; das
/// geht erst mit der JavaScript-Engine.
const SKIPPED_FILES: [&str; 4] = [
    "scripted_adoption01.dat",
    "scripted_ark.dat",
    "scripted_foster01.dat",
    "scripted_webkit01.dat",
];

/// Diese Tests erwarten, dass `selectedcontent` den Inhalt der gewählten
/// `option` kopiert. Das leistet nicht der Parser, sondern das Verhalten von
/// `select`, `option` und `selectedcontent` (Abschnitt „Forms“): Auswahl,
/// Einfügeschritte des DOM und Klonen von Knoten. Sie kommen mit diesem
/// Verhalten dazu.
const PENDING: [(&str, &str); 4] = [
    (
        "webkit02.dat",
        "<select><button><selectedcontent></button><option>X",
    ),
    (
        "webkit02.dat",
        "<select><button><selectedcontent></button><option>x<i>i<b>ib</i>b",
    ),
    (
        "webkit02.dat",
        "<select><button><selectedcontent></button><option>X<option>Y",
    ),
    (
        "webkit02.dat",
        "<select><button><selectedcontent></button><option>X<option selected>Y",
    ),
];

/// Ein Test: die Abschnitte nach ihrer Überschrift.
type Case = BTreeMap<String, String>;

/// `parseDat` aus `test.js`: Jeder Abschnitt sammelt seine Zeilen mit
/// Zeilenumbruch; an jeder Grenze zwischen zwei Tests verliert der aktive
/// Abschnitt ein Zeichen (die Leerzeile), danach jeder Abschnitt einen
/// abschließenden Zeilenumbruch.
fn parse_dat(text: &str) -> Vec<Case> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut cases = Vec::new();
    let mut data: Option<Case> = None;
    let mut key: Option<String> = None;
    let normalise = |case: Case| -> Case {
        case.into_iter()
            .map(|(key, mut value)| {
                if value.ends_with('\n') {
                    value.pop();
                }
                (key, value)
            })
            .collect()
    };
    let count = lines.len();
    for (index, line) in lines.into_iter().enumerate() {
        let mut line = line.to_owned();
        if index.saturating_add(1) < count {
            line.push('\n');
        }
        let heading = line
            .strip_prefix('#')
            .map(str::trim)
            .filter(|heading| !heading.is_empty())
            .map(str::to_owned);
        if let Some(heading) = heading {
            if let Some(mut case) = data.take() {
                if heading == "description"
                    || (heading == "data" && key.as_deref() != Some("description"))
                {
                    if let Some(key) = &key
                        && let Some(value) = case.get_mut(key)
                    {
                        value.pop();
                    }
                    cases.push(normalise(case));
                } else {
                    data = Some(case);
                }
            }
            data.get_or_insert_with(Case::new)
                .insert(heading.clone(), String::new());
            key = Some(heading);
        } else if let Some(key) = &key
            && let Some(value) = data.as_mut().and_then(|case| case.get_mut(key))
        {
            value.push_str(&line);
        }
    }
    if let Some(case) = data {
        cases.push(normalise(case));
    }
    cases
}

fn push_line(out: &mut DomString, depth: usize, line: &DomString) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push('|');
    // „|“, dann zwei Leerzeichen je Ebene, die erste Ebene mit einem.
    for _ in 0..depth.saturating_mul(2).saturating_sub(1) {
        out.push(' ');
    }
    out.push_dom_string(line);
}

fn text(parts: &[&DomString]) -> DomString {
    let mut out = DomString::new();
    for part in parts {
        out.push_dom_string(part);
    }
    out
}

/// `serializeTree` aus `test.js` für die Kinder von `root`, ohne Rekursion.
/// Wie `root.normalize()` dort fasst die Ausgabe benachbarte Textknoten
/// zusammen, nur nicht im Inhalt eines `template` (der gehört nicht zu den
/// Nachkommen).
fn serialize(document: &Document, root: NodeId) -> DomString {
    let mut out = DomString::new();
    // Knoten, Tiefe, innerhalb eines Template-Inhalts.
    let mut stack: Vec<(NodeId, usize, bool)> = Vec::new();
    let push_children = |stack: &mut Vec<(NodeId, usize, bool)>, parent, depth, inside| {
        let children: Vec<NodeId> = document.children(parent).collect();
        for &child in children.iter().rev() {
            stack.push((child, depth, inside));
        }
    };
    push_children(&mut stack, root, 1, false);
    let empty = DomString::new();
    let quote = DomString::from("\"");
    while let Some((node, depth, inside)) = stack.pop() {
        match document.kind(node) {
            Some(NodeKind::DocumentType) => {
                let Some(doctype) = document.doctype(node) else {
                    continue;
                };
                let line = if doctype.name.is_empty() {
                    DomString::from("<!DOCTYPE >")
                } else if !doctype.public_id.is_empty() || !doctype.system_id.is_empty() {
                    text(&[
                        &DomString::from("<!DOCTYPE "),
                        &doctype.name,
                        &DomString::from(" \""),
                        &doctype.public_id,
                        &DomString::from("\" \""),
                        &doctype.system_id,
                        &DomString::from("\">"),
                    ])
                } else {
                    text(&[
                        &DomString::from("<!DOCTYPE "),
                        &doctype.name,
                        &DomString::from(">"),
                    ])
                };
                push_line(&mut out, depth, &line);
            }
            Some(NodeKind::Comment) => {
                let data = document.data(node).unwrap_or(&empty);
                let line = text(&[&DomString::from("<!-- "), data, &DomString::from(" -->")]);
                push_line(&mut out, depth, &line);
            }
            Some(NodeKind::ProcessingInstruction) => {
                let target = document.target(node).unwrap_or(&empty);
                let data = document.data(node).unwrap_or(&empty);
                let line = text(&[
                    &DomString::from("<?"),
                    target,
                    &DomString::from(" "),
                    data,
                    &DomString::from("?>"),
                ]);
                push_line(&mut out, depth, &line);
            }
            Some(NodeKind::Text) => {
                let follows_text = document
                    .previous_sibling(node)
                    .is_some_and(|previous| document.kind(previous) == Some(NodeKind::Text));
                if follows_text && !inside {
                    continue;
                }
                let mut data = document.data(node).cloned().unwrap_or_default();
                let mut next = document.next_sibling(node);
                while !inside
                    && let Some(sibling) = next
                    && document.kind(sibling) == Some(NodeKind::Text)
                {
                    data.push_dom_string(document.data(sibling).unwrap_or(&empty));
                    next = document.next_sibling(sibling);
                }
                push_line(&mut out, depth, &text(&[&quote, &data, &quote]));
            }
            Some(NodeKind::Element) => {
                let Some(element) = document.element(node) else {
                    continue;
                };
                if element.attribute("data-skip").is_some() {
                    continue;
                }
                let prefix = match element.namespace {
                    Some(Namespace::Svg) => "svg ",
                    Some(Namespace::MathMl) => "math ",
                    _ => "",
                };
                let tag = text(&[
                    &DomString::from("<"),
                    &DomString::from(prefix),
                    &element.local_name,
                    &DomString::from(">"),
                ]);
                push_line(&mut out, depth, &tag);
                let mut attributes: Vec<(DomString, &DomString)> = element
                    .attributes
                    .iter()
                    .map(|attribute| {
                        let prefix = match attribute.namespace {
                            Some(Namespace::XLink) => "xlink ",
                            Some(Namespace::Xml) => "xml ",
                            Some(Namespace::Xmlns) => "xmlns ",
                            _ => "",
                        };
                        (
                            text(&[&DomString::from(prefix), &attribute.local_name]),
                            &attribute.value,
                        )
                    })
                    .collect();
                attributes.sort();
                for (name, value) in attributes {
                    let line = text(&[&name, &DomString::from("=\""), value, &quote]);
                    push_line(&mut out, depth.saturating_add(1), &line);
                }
                push_children(&mut stack, node, depth.saturating_add(1), inside);
                if element.is_html("template")
                    && let Some(contents) = document.template_contents(node)
                {
                    // Der Inhalt steht vor den Kindern des Elements.
                    push_children(&mut stack, contents, depth.saturating_add(2), true);
                    push_line(
                        &mut out,
                        depth.saturating_add(1),
                        &DomString::from("content"),
                    );
                }
            }
            _ => {}
        }
    }
    out
}

/// Der Kontext aus `#document-fragment`: „svg name“, „math name“ oder ein
/// HTML-Name.
fn context_element(document: &mut Document, context: &str) -> NodeId {
    let (namespace, name) = if let Some(name) = context.strip_prefix("svg ") {
        (Namespace::Svg, name)
    } else if let Some(name) = context.strip_prefix("math ") {
        (Namespace::MathMl, name)
    } else {
        (Namespace::Html, context)
    };
    document.create_element(Some(namespace), DomString::from(name), Vec::new())
}

/// Parst einen Test und gibt den Baum zurück.
fn run(data: &str, fragment: Option<&str>, scripting: bool) -> DomString {
    let input = data.chars().map(CodePoint::from);
    let mut document = Document::new();
    match fragment {
        None => {
            let options = Options {
                scripting: if scripting {
                    ScriptingMode::Normal
                } else {
                    ScriptingMode::Disabled
                },
                collect_errors: false,
            };
            parse_document(&mut document, input, options);
            let root = document.root();
            serialize(&document, root)
        }
        Some(context) => {
            let options = Options {
                scripting: if scripting {
                    ScriptingMode::Inert
                } else {
                    ScriptingMode::Disabled
                },
                collect_errors: false,
            };
            let context = context_element(&mut document, context);
            let (fragment, _) =
                parse_fragment(&mut document, context, input, options).expect("Kontextelement");
            serialize(&document, fragment)
        }
    }
}

#[test]
fn wpt_tree_construction() {
    let files = testdata::files("wpt", "html/syntax/parsing/resources", "dat");
    let mut total = 0;
    let mut failures = Vec::new();
    for file in &files {
        if SKIPPED_FILES.contains(&file.as_str()) {
            continue;
        }
        let source = testdata::read("wpt", &format!("html/syntax/parsing/resources/{file}"));
        for case in parse_dat(&source) {
            let data = case.get("data").map_or("", String::as_str);
            let expected = DomString::from(case.get("document").map_or("", String::as_str));
            let fragment = case.get("document-fragment").map(String::as_str);
            if PENDING.contains(&(file.as_str(), data)) {
                continue;
            }
            let modes: &[bool] = if case.contains_key("script-off") {
                &[false]
            } else if case.contains_key("script-on") {
                &[true]
            } else {
                &[false, true]
            };
            for &scripting in modes {
                total += 1;
                let tree = run(data, fragment, scripting);
                let label = format!(
                    "{file}: {data:?}{} (Skripte {})",
                    fragment
                        .map(|context| format!(" in {context}"))
                        .unwrap_or_default(),
                    if scripting { "an" } else { "aus" }
                );
                if tree != expected {
                    failures.push(format!(
                        "{label}\n  erwartet:\n{}\n  erhalten:\n{}",
                        expected.to_string_lossy(),
                        tree.to_string_lossy()
                    ));
                }
            }
        }
    }
    assert!(total > 3000, "zu wenige Tests gefunden: {total}");
    let shown: Vec<&str> = failures.iter().take(30).map(String::as_str).collect();
    assert!(
        failures.is_empty(),
        "{} von {total} Läufen ergeben einen anderen Baum:\n{}",
        failures.len(),
        shown.join("\n")
    );
}

/// Die Fehler eines Dokuments: Code oder `None` für Fehler des Baumaufbaus.
fn errors(input: &str) -> Vec<Option<ErrorCode>> {
    let mut document = Document::new();
    let options = Options {
        scripting: ScriptingMode::Normal,
        collect_errors: true,
    };
    parse_document(&mut document, input.chars().map(CodePoint::from), options)
        .into_iter()
        .map(|error| error.code)
        .collect()
}

#[test]
fn tree_construction_errors() {
    // Ohne DOCTYPE: „initial“, „anything else“. Ein offenes `p` am Ende ist
    // erlaubt.
    assert_eq!(errors("<p>"), [None]);
    // U+0000 meldet der Tokenizer und „in body“ noch einmal.
    assert_eq!(
        errors("<!DOCTYPE html>x\0y"),
        [Some(ErrorCode::UnexpectedNullCharacter), None]
    );
    // Ein Fehler je Zeichen, das aus der Tabelle wandert.
    assert_eq!(errors("<!DOCTYPE html><table>xy</table>"), [None, None]);
    assert_eq!(errors("<!DOCTYPE html><table> </table>"), []);
    // Der unbestätigte Schrägstrich, dann das offene `div` am Dateiende.
    assert_eq!(
        errors("<!DOCTYPE html><div/>"),
        [
            Some(ErrorCode::NonVoidHtmlElementStartTagWithTrailingSolidus),
            None
        ]
    );
    // Bei leeren Elementen bestätigt der Baumaufbau den Schrägstrich.
    assert_eq!(errors("<!DOCTYPE html><br/><img/>"), []);
}

/// Ein CDATA-Abschnitt ganz am Anfang eines Fragments: Im Kontext `svg` ist
/// er Text, in einem HTML-Kontext ein Kommentar mit Fehler.
#[test]
fn fragment_starts_with_cdata() {
    let fragment = |context: &str| {
        let mut document = Document::new();
        let context = context_element(&mut document, context);
        let options = Options {
            scripting: ScriptingMode::Inert,
            collect_errors: true,
        };
        let input = "<![CDATA[x]]>".chars().map(CodePoint::from);
        let (fragment, errors) =
            parse_fragment(&mut document, context, input, options).expect("Kontextelement");
        let codes: Vec<Option<ErrorCode>> = errors.into_iter().map(|error| error.code).collect();
        (serialize(&document, fragment), codes)
    };
    assert_eq!(fragment("svg svg"), (DomString::from("| \"x\""), vec![]));
    assert_eq!(fragment("math math"), (DomString::from("| \"x\""), vec![]));
    assert_eq!(
        fragment("div"),
        (
            DomString::from("| <!-- [CDATA[x]] -->"),
            vec![Some(ErrorCode::CdataInHtmlContent)]
        )
    );
}

#[test]
fn parse_dat_like_test_js() {
    let text = "#data\n<p>One\n#errors\n(1,3): x\n#document\n| <html>\n|   \"a\n\"\n\n#data\nb\n#errors\n#document\n| \"b\"\n";
    let cases = parse_dat(text);
    assert_eq!(cases.len(), 2);
    let first = cases.first().expect("erster Test");
    assert_eq!(first.get("data").map(String::as_str), Some("<p>One"));
    assert_eq!(first.get("errors").map(String::as_str), Some("(1,3): x"));
    assert_eq!(
        first.get("document").map(String::as_str),
        Some("| <html>\n|   \"a\n\"")
    );
    let second = cases.get(1).expect("zweiter Test");
    assert_eq!(second.get("errors").map(String::as_str), Some(""));
    assert_eq!(second.get("document").map(String::as_str), Some("| \"b\""));
}
