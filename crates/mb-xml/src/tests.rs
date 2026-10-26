use mb_base::DomString;
use mb_dom::{Namespace, NodeId, NodeKind};

use super::*;

fn parse_text(text: &str) -> Parsed {
    parse(text.as_bytes(), &Options::default())
}

fn well_formed(text: &str) -> Document {
    let parsed = parse_text(text);
    assert_eq!(parsed.error, None, "{text}");
    parsed.document
}

fn root_element(document: &Document) -> NodeId {
    document
        .children(document.root())
        .find(|&child| document.kind(child) == Some(NodeKind::Element))
        .expect("Wurzelelement")
}

fn children(document: &Document, node: NodeId) -> Vec<NodeId> {
    document.children(node).collect()
}

fn data(document: &Document, node: NodeId) -> String {
    document
        .data(node)
        .map(DomString::to_string_lossy)
        .unwrap_or_default()
}

fn attribute_value(document: &Document, node: NodeId, name: &str) -> Option<String> {
    document
        .element(node)?
        .attribute(name)
        .map(DomString::to_string_lossy)
}

const XHTML_STRICT: &str = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd">"#;

#[test]
fn namespaces_in_the_dom() {
    let document = well_formed(concat!(
        r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">"#,
        r#"<a xlink:href="&#x23;x" xml:lang="de"/><e:x xmlns:e="urn:example" e:y="1"/>"#,
        "</svg></body></html>"
    ));
    assert!(!document.is_html_document());
    let html = root_element(&document);
    assert!(document.element(html).is_some_and(|e| e.is_html("html")));
    let body = document.first_child(html).expect("body");
    let svg = document.first_child(body).expect("svg");
    let svg_element = document.element(svg).expect("Element");
    assert!(svg_element.is(Namespace::Svg, "svg"));
    let declarations: Vec<(Option<Namespace>, Option<String>, String)> = svg_element
        .attributes
        .iter()
        .map(|a| {
            (
                a.namespace,
                a.prefix.as_ref().map(DomString::to_string_lossy),
                a.local_name.to_string_lossy(),
            )
        })
        .collect();
    assert_eq!(
        declarations,
        [
            (Some(Namespace::Xmlns), None, "xmlns".to_owned()),
            (
                Some(Namespace::Xmlns),
                Some("xmlns".to_owned()),
                "xlink".to_owned()
            ),
        ]
    );
    let [a, x] = children(&document, svg)[..] else {
        panic!("zwei Kinder erwartet");
    };
    let a = document.element(a).expect("Element");
    assert!(a.is(Namespace::Svg, "a"));
    let namespaces: Vec<(Option<Namespace>, String, String)> = a
        .attributes
        .iter()
        .map(|a| {
            (
                a.namespace,
                a.local_name.to_string_lossy(),
                a.value.to_string_lossy(),
            )
        })
        .collect();
    assert_eq!(
        namespaces,
        [
            (Some(Namespace::XLink), "href".to_owned(), "#x".to_owned()),
            (Some(Namespace::Xml), "lang".to_owned(), "de".to_owned()),
        ]
    );
    let x = document.element(x).expect("Element");
    let other = x.namespace.expect("Namensraum");
    assert_eq!(document.namespace_url(other), Some("urn:example"));
    assert_eq!(
        x.prefix.as_ref().map(DomString::to_string_lossy).as_deref(),
        Some("e")
    );
    assert_eq!(x.attributes.get(1).and_then(|a| a.namespace), Some(other));
}

#[test]
fn without_namespaces() {
    let parsed = parse(
        b"<a:b xmlns:a='urn:a' a:c='1'/>",
        &Options { namespaces: false },
    );
    assert_eq!(parsed.error, None);
    let document = parsed.document;
    let element = document.element(root_element(&document)).expect("Element");
    assert_eq!(element.namespace, None);
    assert_eq!(element.local_name, "a:b");
    assert!(element.attributes.iter().all(|a| a.namespace.is_none()));
    // Mit Namensräumen ist ein nicht deklariertes Präfix ein Fehler.
    assert!(parse_text("<a:b/>").error.is_some());
}

#[test]
fn template_contents() {
    let document = well_formed(
        r#"<html xmlns="http://www.w3.org/1999/xhtml"><template><p>x</p></template></html>"#,
    );
    let html = root_element(&document);
    let template = document.first_child(html).expect("template");
    assert_eq!(document.first_child(template), None);
    let contents = document.template_contents(template).expect("Inhalt");
    let p = document.first_child(contents).expect("p");
    assert!(document.element(p).is_some_and(|e| e.is_html("p")));
}

#[test]
fn html_entities_for_xhtml() {
    let document = well_formed(&format!(
        "{XHTML_STRICT}<p a='x&Tab;y'>&nbsp;&amp;&lt;&Tab;</p>"
    ));
    let p = root_element(&document);
    assert_eq!(attribute_value(&document, p, "a").as_deref(), Some("x y"));
    let text = document.first_child(p).expect("Text");
    assert_eq!(data(&document, text), "\u{A0}&<\t");
    // Ohne den öffentlichen Bezeichner gibt es die Entitäten nicht.
    assert!(parse_text("<p>&nbsp;</p>").error.is_some());
    // Mit standalone="yes" zählen sie nicht als deklariert.
    let standalone = format!("<?xml version='1.0' standalone='yes'?>{XHTML_STRICT}<p>&nbsp;</p>");
    assert!(parse_text(&standalone).error.is_some());
    // Der interne Teil geht vor.
    let document = well_formed(&format!(
        "{}[<!ENTITY nbsp 'eigen'>]><p>&nbsp;</p>",
        XHTML_STRICT.trim_end_matches('>')
    ));
    let p = root_element(&document);
    assert_eq!(
        data(&document, document.first_child(p).expect("Text")),
        "eigen"
    );
}

#[test]
fn doctype_and_nodes_outside_the_root() {
    let document = well_formed(&format!(
        "<?xml version='1.0'?>\n<!--a-->\n{XHTML_STRICT}\n<?b c?>\n<p/>\n<!--d-->\n"
    ));
    let kinds: Vec<Option<NodeKind>> = document
        .children(document.root())
        .map(|child| document.kind(child))
        .collect();
    assert_eq!(
        kinds,
        [
            Some(NodeKind::Comment),
            Some(NodeKind::DocumentType),
            Some(NodeKind::ProcessingInstruction),
            Some(NodeKind::Element),
            Some(NodeKind::Comment),
        ]
    );
    let doctype = document
        .children(document.root())
        .find_map(|child| document.doctype(child))
        .expect("Doctype");
    assert_eq!(doctype.name, "html");
    assert_eq!(doctype.public_id, "-//W3C//DTD XHTML 1.0 Strict//EN");
    assert_eq!(
        doctype.system_id,
        "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd"
    );
}

#[test]
fn text_cdata_and_entities() {
    let document = well_formed(
        "<!DOCTYPE a [<!ENTITY e 'b<i>c</i>d'><!ENTITY f 'x&e;y'>]>\
         <a>1&f;2<![CDATA[<&>]]>3<?p  q ?></a>",
    );
    let a = root_element(&document);
    let kinds: Vec<(Option<NodeKind>, String)> = document
        .children(a)
        .map(|child| (document.kind(child), data(&document, child)))
        .collect();
    assert_eq!(
        kinds,
        [
            (Some(NodeKind::Text), "1xb".to_owned()),
            (Some(NodeKind::Element), String::new()),
            (Some(NodeKind::Text), "dy2".to_owned()),
            (Some(NodeKind::CdataSection), "<&>".to_owned()),
            (Some(NodeKind::Text), "3".to_owned()),
            (Some(NodeKind::ProcessingInstruction), "q ".to_owned()),
        ]
    );
}

#[test]
fn attribute_defaults_and_normalization() {
    let document = well_formed(
        "<!DOCTYPE a [<!ATTLIST a b NMTOKENS ' x  y ' c CDATA 'd&#9;e' f CDATA #IMPLIED>\
         <!ENTITY t '&#9;'>]><a b='  p&#32;  q ' g='1&t;2&#9;3'/>",
    );
    let a = root_element(&document);
    assert_eq!(attribute_value(&document, a, "b").as_deref(), Some("p q"));
    assert_eq!(attribute_value(&document, a, "c").as_deref(), Some("d\te"));
    assert_eq!(attribute_value(&document, a, "f"), None);
    // Leerraum aus dem Ersetzungstext wird zum Leerzeichen, ein
    // Zeichenverweis bleibt.
    assert_eq!(
        attribute_value(&document, a, "g").as_deref(),
        Some("1 2\t3")
    );
}

#[test]
fn unread_external_declarations() {
    // Mit externem Teil der DTD ist eine unbekannte Entität kein Fehler.
    let document = well_formed("<!DOCTYPE a SYSTEM 'a.dtd'><a>x&y;z</a>");
    let a = root_element(&document);
    assert_eq!(
        data(&document, document.first_child(a).expect("Text")),
        "xz"
    );
    // Nach einer ungelesenen Parameter-Entität gelten keine Deklarationen
    // von Attributlisten mehr.
    let document = well_formed(
        "<!DOCTYPE a [<!ATTLIST a b CDATA 'v'><!ENTITY % p SYSTEM 'p.dtd'>%p;\
         <!ATTLIST a c CDATA 'w'>]><a/>",
    );
    let a = root_element(&document);
    assert_eq!(attribute_value(&document, a, "b").as_deref(), Some("v"));
    assert_eq!(attribute_value(&document, a, "c"), None);
}

#[test]
fn error_position_and_partial_tree() {
    let parsed = parse_text("<a>text<b>\n  <c></a>");
    assert_eq!(
        parsed.error,
        Some(XmlError {
            line: 2,
            column: 10,
            message: "Endtag passt nicht zum Starttag",
        })
    );
    let document = parsed.document;
    let a = root_element(&document);
    let [text, b] = children(&document, a)[..] else {
        panic!("zwei Kinder erwartet");
    };
    assert_eq!(data(&document, text), "text");
    let c = children(&document, b).last().copied().expect("c");
    assert!(document.element(c).is_some_and(|e| e.local_name == "c"));
    // Ein unzulässiges Zeichen beendet das Dokument an seiner Stelle.
    let parsed = parse_text("<a>x\u{1}</a>");
    assert_eq!(
        parsed.error.map(|error| (error.column, error.message)),
        Some((5, "unzulässiges Zeichen"))
    );
}

#[test]
fn entity_expansion_is_limited() {
    let mut dtd = String::from("<!ENTITY a0 'xxxxxxxxxx'>");
    for level in 1..10 {
        let previous = format!("&a{};", level - 1);
        dtd.push_str(&format!("<!ENTITY a{level} '{}'>", previous.repeat(10)));
    }
    let parsed = parse_text(&format!("<!DOCTYPE r [{dtd}]><r>&a9;</r>"));
    assert_eq!(
        parsed.error.map(|error| error.message),
        Some("Entitäten setzen zu viel Text ein")
    );
    let parsed = parse_text("<!DOCTYPE r [<!ENTITY a '&b;'><!ENTITY b '&a;'>]><r>&a;</r>");
    assert_eq!(
        parsed.error.map(|error| error.message),
        Some("Entität verweist auf sich selbst")
    );
}

#[test]
fn deep_nesting() {
    let depth = 10_000;
    let text = format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
    let document = well_formed(&text);
    let mut node = root_element(&document);
    let mut count = 1;
    while let Some(child) = document.first_child(node) {
        node = child;
        count += 1;
    }
    assert_eq!(count, depth);
}

#[test]
fn encoding_is_reported() {
    let parsed = parse(
        b"<?xml version='1.0' encoding='iso-8859-1'?><a>\xE4</a>",
        &Options::default(),
    );
    assert_eq!(parsed.encoding, Some(mb_encoding::Encoding::Windows1252));
    let a = root_element(&parsed.document);
    assert_eq!(
        data(
            &parsed.document,
            parsed.document.first_child(a).expect("Text")
        ),
        "\u{E4}"
    );
    let parsed = parse(b"<a>\xFF</a>", &Options::default());
    assert_eq!(parsed.encoding, None);
    assert!(parsed.error.is_some());
}

/// Mit Byte-Order-Mark passt jede Bezeichnung für UTF-16 ohne
/// Bytereihenfolge, eine mit Bytereihenfolge nur, wenn sie stimmt.
#[test]
fn utf16_byte_order_and_declaration() {
    use mb_encoding::Encoding::{Utf16Be, Utf16Le};
    fn utf16(text: &str, encoding: mb_encoding::Encoding) -> Vec<u8> {
        std::iter::once(0xFEFF)
            .chain(text.encode_utf16())
            .flat_map(|unit| match encoding {
                Utf16Be => unit.to_be_bytes(),
                _ => unit.to_le_bytes(),
            })
            .collect()
    }
    for (encoding, label, fits) in [
        (Utf16Le, "UTF-16", true),
        (Utf16Be, "UTF-16", true),
        (Utf16Be, "ucs-2", true),
        (Utf16Le, "UTF-16LE", true),
        (Utf16Be, "unicodeFFFE", true),
        (Utf16Le, "UTF-16BE", false),
        (Utf16Be, "UTF-16LE", false),
        (Utf16Be, "unicodeFEFF", false),
    ] {
        let text = format!("<?xml version='1.0' encoding='{label}'?><a/>");
        let parsed = parse(&utf16(&text, encoding), &Options::default());
        if fits {
            assert_eq!(parsed.error, None, "{encoding:?}, {label}");
            assert_eq!(parsed.encoding, Some(encoding), "{label}");
        } else {
            assert!(parsed.error.is_some(), "{encoding:?}, {label}");
        }
    }
}
