//! Die XML-Testsuite des W3C (Version 20130923) aus
//! `xml-conformance-suite` (siehe `tools/testdata`), mit deren flachem
//! Katalog `cleaned/xmlconf-flattened.xml`.
//!
//! Es laufen die Tests für XML 1.0 in der fünften Ausgabe: ohne `VERSION`
//! oder mit 1.0, ohne `EDITION` oder mit 5 darunter. Nicht wohlgeformte
//! Dokumente (`not-wf`) müssen einen fatalen Fehler ergeben, gültige und
//! ungültige (`valid`, `invalid`) keinen; ein Parser, der nicht validiert,
//! unterscheidet die beiden nicht. Die Fälle `error` legen kein Verhalten
//! fest. Weil der Parser keine externen Entitäten liest, zählen nicht
//! wohlgeformte Dokumente nur, wenn der Fehler nicht in einer externen
//! Entität liegt (`ENTITIES="none"`), und erwartete Ausgaben auch nur
//! dann. Die Ausgaben sind „Canonical XML“ nach James Clark; die zweite
//! Form beginnt mit einer Dokumenttypdeklaration nur für die Notationen,
//! die der Vergleich auslässt.

use mb_base::DomString;
use mb_dom::{Document, NodeId, NodeKind};

use crate::{Options, parse};

const SOURCE: &str = "xml-conformance-suite";
const ROOT: &str = "packages/test-data/xmlconf/";

struct Case {
    id: String,
    kind: String,
    entities: String,
    namespaces: bool,
    path: String,
    output: Option<String>,
}

fn attribute(document: &Document, node: NodeId, name: &str) -> Option<String> {
    document
        .element(node)?
        .attributes
        .iter()
        .find(|attribute| attribute.local_name == *name)
        .map(|attribute| attribute.value.to_string_lossy())
}

/// Die Fälle aus dem Katalog, gelesen mit diesem Parser.
fn catalog() -> Vec<Case> {
    let bytes = testdata::read_bytes(SOURCE, "packages/test-data/cleaned/xmlconf-flattened.xml");
    let parsed = parse(&bytes, &Options::default());
    assert_eq!(parsed.error, None, "Katalog nicht lesbar");
    let document = &parsed.document;
    let mut cases = Vec::new();
    // Ohne Rekursion: Knoten mit dem `xml:base`, das für sie gilt.
    let mut work: Vec<(NodeId, String)> = vec![(document.root(), ROOT.to_owned())];
    while let Some((node, base)) = work.pop() {
        let Some(element) = document.element(node) else {
            for child in document.children(node) {
                work.push((child, base.clone()));
            }
            continue;
        };
        let base = match element.attributes.iter().find(|attribute| {
            attribute.namespace == Some(mb_dom::Namespace::Xml) && attribute.local_name == "base"
        }) {
            Some(attribute) => format!("{base}{}", attribute.value.to_string_lossy()),
            None => base,
        };
        if element.local_name == "TEST" {
            let get = |name: &str| attribute(document, node, name);
            let version = get("VERSION");
            let edition = get("EDITION");
            let fifth = edition
                .as_deref()
                .is_none_or(|edition| edition.split(' ').any(|number| number == "5"));
            if version.as_deref().is_none_or(|version| version == "1.0") && fifth {
                cases.push(Case {
                    id: get("ID").expect("ID"),
                    kind: get("TYPE").expect("TYPE"),
                    entities: get("ENTITIES").unwrap_or_else(|| "none".to_owned()),
                    namespaces: get("NAMESPACE").as_deref() != Some("no"),
                    path: format!("{base}{}", get("URI").expect("URI")),
                    output: get("OUTPUT").map(|output| format!("{base}{output}")),
                });
            }
            continue;
        }
        for child in document.children(node) {
            work.push((child, base.clone()));
        }
    }
    cases
}

fn qualified_name(prefix: Option<&DomString>, local_name: &DomString) -> String {
    match prefix {
        Some(prefix) => format!(
            "{}:{}",
            prefix.to_string_lossy(),
            local_name.to_string_lossy()
        ),
        None => local_name.to_string_lossy(),
    }
}

fn escape(text: &DomString, out: &mut String) {
    for c in text.to_string_lossy().chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
}

/// Das Dokument in der ersten Form von „Canonical XML“: Attribute nach
/// Namen geordnet, keine Kommentare, keine Dokumenttypdeklaration.
fn canonical(document: &Document) -> String {
    enum Step {
        Open(NodeId),
        Close(String),
    }
    let mut out = String::new();
    let mut work: Vec<Step> = document.children(document.root()).map(Step::Open).collect();
    work.reverse();
    while let Some(step) = work.pop() {
        let node = match step {
            Step::Open(node) => node,
            Step::Close(name) => {
                out.push_str("</");
                out.push_str(&name);
                out.push('>');
                continue;
            }
        };
        match document.kind(node) {
            Some(NodeKind::Element) => {
                let Some(element) = document.element(node) else {
                    continue;
                };
                let name = qualified_name(element.prefix.as_ref(), &element.local_name);
                out.push('<');
                out.push_str(&name);
                let mut attributes: Vec<(String, &DomString)> = element
                    .attributes
                    .iter()
                    .map(|attribute| {
                        (
                            qualified_name(attribute.prefix.as_ref(), &attribute.local_name),
                            &attribute.value,
                        )
                    })
                    .collect();
                attributes.sort();
                for (name, value) in attributes {
                    out.push(' ');
                    out.push_str(&name);
                    out.push_str("=\"");
                    escape(value, &mut out);
                    out.push('"');
                }
                out.push('>');
                work.push(Step::Close(name));
                let children: Vec<NodeId> = document.children(node).collect();
                work.extend(children.into_iter().rev().map(Step::Open));
            }
            Some(NodeKind::Text | NodeKind::CdataSection) => {
                if let Some(data) = document.data(node) {
                    escape(data, &mut out);
                }
            }
            Some(NodeKind::ProcessingInstruction) => {
                out.push_str("<?");
                if let Some(target) = document.target(node) {
                    out.push_str(&target.to_string_lossy());
                }
                out.push(' ');
                if let Some(data) = document.data(node) {
                    out.push_str(&data.to_string_lossy());
                }
                out.push_str("?>");
            }
            _ => {}
        }
    }
    out
}

/// Die Fälle, die anders ausgehen als die Testsuite erwartet, mit Grund.
const DEVIATIONS: [(&str, &str); 1] = [(
    "ibm-valid-P29-ibm29v01.xml",
    "Die Ausgabe enthält eine Verarbeitungsanweisung aus dem internen Teil der DTD. \
     Im DOM hat sie keinen Platz, ein DocumentType hat keine Kinder.",
)];

#[test]
fn w3c_xml_test_suite() {
    let cases = catalog();
    let mut run = 0_usize;
    let mut not_well_formed = 0_usize;
    let mut outputs = 0_usize;
    let mut failures: Vec<(String, String)> = Vec::new();
    for case in &cases {
        if case.kind == "error" || (case.kind == "not-wf" && case.entities != "none") {
            continue;
        }
        run += 1;
        if case.kind == "not-wf" {
            not_well_formed += 1;
        }
        let bytes = testdata::read_bytes(SOURCE, &case.path);
        let parsed = parse(
            &bytes,
            &Options {
                namespaces: case.namespaces,
            },
        );
        let failure = match (case.kind.as_str(), parsed.error) {
            ("not-wf", None) => Some("nicht wohlgeformt, aber kein Fehler".to_owned()),
            ("not-wf", Some(_)) => None,
            (_, Some(error)) => Some(format!("Fehler: {error}")),
            (_, None) => match &case.output {
                Some(output) if case.entities == "none" => {
                    outputs += 1;
                    let expected = testdata::read(SOURCE, output);
                    // Die zweite Form: `<!DOCTYPE … [` mit Notationen `]>` und
                    // einem Zeilenumbruch.
                    let expected = match expected.strip_prefix("<!DOCTYPE") {
                        Some(rest) => rest
                            .split_once("]>\n")
                            .map_or(expected.as_str(), |(_, rest)| rest),
                        None => expected.as_str(),
                    };
                    let actual = canonical(&parsed.document);
                    (actual != expected)
                        .then(|| format!("Ausgabe {actual:?}, erwartet {expected:?}"))
                }
                _ => None,
            },
        };
        let deviation = DEVIATIONS.iter().any(|(id, _)| *id == case.id);
        match (failure, deviation) {
            (Some(failure), false) => {
                failures.push((case.id.clone(), format!("{}: {failure}", case.path)))
            }
            (None, true) => failures.push((
                case.id.clone(),
                "besteht jetzt, aus DEVIATIONS streichen".to_owned(),
            )),
            _ => {}
        }
    }
    let report: Vec<String> = failures
        .iter()
        .map(|(id, failure)| format!("{id}: {failure}"))
        .collect();
    assert!(
        failures.is_empty(),
        "{} von {run} Fällen:\n{}",
        failures.len(),
        report.join("\n")
    );
    assert_eq!(
        (run, not_well_formed, outputs),
        (1908, 951, 262),
        "Zahl der Fälle"
    );
}
