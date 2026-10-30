//! Tests für die Kodierung aus den Bytes: die Kodierungstests aus
//! html5lib-tests (`encoding/*.dat`) und eigene Fälle, nachgerechnet am
//! Standard.
//!
//! Die html5lib-Tests prüft man wie html5lib selbst (`test_encoding.py`):
//! Das Ergebnis des Parsens zählt für alle Fälle, das des Vorab-Lesens nur
//! für Fälle bis 1024 Bytes. Die Ersatzkodierung ist windows-1252, wie der
//! erste Fall verlangt. Nicht dabei sind `encoding/scripted/`, dessen Fall
//! `document.write()` ausführen muss, und `encoding/chardet/`, ein Text für
//! die Erkennung aus dem Inhalt, die es nicht gibt.

use mb_dom::{Document, NodeKind};
use mb_encoding::Encoding;

use super::{Confidence, change, prescan, sniff};
use crate::{Options, ScriptingMode, parse_bytes};

const OPTIONS: Options = Options {
    scripting: ScriptingMode::Disabled,
    collect_errors: false,
};

/// Die Fälle einer `.dat`-Datei: die Bytes unter `#data` ohne den letzten
/// Zeilenumbruch und die Bezeichnung unter `#encoding`.
fn cases(file: &str) -> Vec<(Vec<u8>, String)> {
    let bytes = testdata::read_bytes("html5lib-tests", &format!("encoding/{file}"));
    let mut cases = Vec::new();
    let mut data: Option<Vec<&[u8]>> = None;
    let mut expect_encoding = false;
    for line in bytes.split(|&b| b == b'\n') {
        if line == b"#data" {
            data = Some(Vec::new());
        } else if line == b"#encoding" {
            expect_encoding = true;
        } else if expect_encoding {
            let lines = data.take().expect("#data vor #encoding");
            cases.push((
                lines.join(&b'\n'),
                String::from_utf8_lossy(line).trim().to_owned(),
            ));
            expect_encoding = false;
        } else if let Some(lines) = &mut data {
            lines.push(line);
        }
    }
    cases
}

#[test]
fn html5lib_encoding() {
    let files = testdata::files("html5lib-tests", "encoding", "dat");
    assert!(!files.is_empty());
    let mut total = 0;
    let mut failures = Vec::new();
    for file in &files {
        for (data, label) in cases(file) {
            let expected = Encoding::for_label(label.as_bytes()).expect("bekannte Kodierung");
            total += 1;
            let parsed = parse_bytes(&data, Encoding::Windows1252, OPTIONS).encoding;
            if parsed != expected {
                failures.push(format!("{file} (Parsen): {parsed:?} statt {label}"));
            }
            if data.len() <= 1024 {
                total += 1;
                let (sniffed, _) = sniff(&data, Encoding::Windows1252);
                if sniffed != expected {
                    failures.push(format!("{file} (Vorab-Lesen): {sniffed:?} statt {label}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} von {total}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn byte_order_marks_are_certain() {
    assert_eq!(
        sniff(b"\xEF\xBB\xBF<meta charset=koi8-r>", Encoding::Windows1252),
        (Encoding::Utf8, Confidence::Certain)
    );
    assert_eq!(
        sniff(b"\xFF\xFE<\0", Encoding::Windows1252),
        (Encoding::Utf16Le, Confidence::Certain)
    );
    assert_eq!(
        sniff(b"<meta charset=koi8-r>", Encoding::Utf8),
        (Encoding::Koi8R, Confidence::Tentative)
    );
    assert_eq!(
        sniff(b"<p>", Encoding::Utf8),
        (Encoding::Utf8, Confidence::Tentative)
    );
}

#[test]
fn only_the_first_kilobyte_is_prescanned() {
    let mut bytes = vec![b' '; 1024];
    bytes.extend_from_slice(b"<meta charset=koi8-r>");
    assert_eq!(sniff(&bytes, Encoding::Utf8).0, Encoding::Utf8);
    assert_eq!(prescan(&bytes), Some(Encoding::Koi8R));
}

#[test]
fn xml_declarations() {
    assert_eq!(prescan(b"<\0?\0x\0m\0l\0"), Some(Encoding::Utf16Le));
    assert_eq!(prescan(b"\0<\0?\0x\0m\0l"), Some(Encoding::Utf16Be));
    assert_eq!(
        prescan(b"<?xml version='1.0' encoding = 'iso-8859-2'?><p>"),
        Some(Encoding::Iso8859_2)
    );
    // UTF-16 wird UTF-8, x-user-defined bleibt hier.
    assert_eq!(
        prescan(b"<?xml encoding=\"utf-16\"?>"),
        Some(Encoding::Utf8)
    );
    assert_eq!(
        prescan(b"<?xml encoding=\"x-user-defined\"?>"),
        Some(Encoding::XUserDefined)
    );
    // `encoding` muss vor dem ersten `>` stehen, der Wert nicht.
    assert_eq!(prescan(b"<?xml encoding=\"koi8>-r\"?>"), None);
    assert_eq!(prescan(b"<?xml x='>' encoding=\"koi8-r\"?>"), None);
    assert_eq!(prescan(b"<?xml encoding=\"koi8-r \"?>"), None);
    assert_eq!(prescan(b"<?xml encoding=koi8-r?>"), None);
    assert_eq!(prescan(b" <?xml encoding=\"koi8-r\"?>"), None);
    // Ein `meta` geht vor.
    assert_eq!(
        prescan(b"<?xml encoding=\"koi8-r\"?><meta charset=iso-8859-2>"),
        Some(Encoding::Iso8859_2)
    );
}

#[test]
fn meta_declarations_in_the_prescan() {
    assert_eq!(
        prescan(b"<meta charset=x-user-defined>"),
        Some(Encoding::Windows1252)
    );
    assert_eq!(prescan(b"<META/CHARSET=KOI8-R>"), Some(Encoding::Koi8R));
    // `charset` ohne gültige Bezeichnung, danach `content`: Der Wert aus
    // `content` zählt nicht mehr, denn *charset* ist schon *failure*.
    assert_eq!(
        prescan(
            b"<meta charset=bogus http-equiv=content-type content='text/html; charset=koi8-r'>"
        ),
        None
    );
    // Doppelte Attribute zählen nur einmal.
    assert_eq!(
        prescan(b"<meta charset=koi8-r charset=iso-8859-2>"),
        Some(Encoding::Koi8R)
    );
    // `<!-->` ist schon zu Ende.
    assert_eq!(
        prescan(b"<!--><meta charset=koi8-r>"),
        Some(Encoding::Koi8R)
    );
    assert_eq!(prescan(b"<!-- --!><meta charset=koi8-r>"), None);
    // Hinter `</` ohne Buchstaben und hinter `<?` bis zum nächsten `>`.
    assert_eq!(prescan(b"</ <meta charset=koi8-r>"), None);
    assert_eq!(prescan(b"<?x <meta charset=koi8-r>"), None);
    // Ein End-Tag mit Attributen.
    assert_eq!(prescan(b"</meta charset=koi8-r>"), None);
}

#[test]
fn changing_the_encoding() {
    // Bei UTF-16 und bei derselben Kodierung wird sie nur sicher.
    assert_eq!(change(Encoding::Utf16Le, Encoding::Koi8R), None);
    assert_eq!(change(Encoding::Windows1252, Encoding::XUserDefined), None);
    assert_eq!(change(Encoding::Utf8, Encoding::Utf16Be), None);
    assert_eq!(
        change(Encoding::Utf8, Encoding::Koi8R),
        Some(Encoding::Koi8R)
    );
}

/// Der Text aller Textknoten.
fn text(document: &Document) -> String {
    let root = document.root();
    let mut text = String::new();
    let mut node = document.following(root, root);
    while let Some(current) = node {
        if document.kind(current) == Some(NodeKind::Text) {
            text.push_str(&document.data(current).expect("Text").to_string_lossy());
        }
        node = document.following(current, root);
    }
    text
}

#[test]
fn late_meta_declarations_reparse() {
    // Das `meta` steht hinter dem ersten Kilobyte; das Vorab-Lesen findet es
    // nicht, der Baumaufbau schon.
    let comment = "x".repeat(1100);
    let mut bytes = format!("<!--{comment}--><meta charset=windows-1252><p>").into_bytes();
    bytes.push(0xE9);
    let parsed = parse_bytes(&bytes, Encoding::Utf8, OPTIONS);
    assert_eq!(parsed.encoding, Encoding::Windows1252);
    assert_eq!(text(&parsed.document), "é");
    // `charset` ohne gültige Bezeichnung: Dann zählt `content`.
    let bytes = format!(
        "<!--{comment}--><meta charset=bogus http-equiv=Content-Type \
         content='text/html; charset=koi8-r'>"
    );
    let parsed = parse_bytes(bytes.as_bytes(), Encoding::Utf8, OPTIONS);
    assert_eq!(parsed.encoding, Encoding::Koi8R);
    // Ist die Kodierung sicher, ändert ein zweites `meta` nichts mehr.
    let bytes = format!("<!--{comment}--><meta charset=utf-8><meta charset=koi8-r>");
    let parsed = parse_bytes(bytes.as_bytes(), Encoding::Utf8, OPTIONS);
    assert_eq!(parsed.encoding, Encoding::Utf8);
}
