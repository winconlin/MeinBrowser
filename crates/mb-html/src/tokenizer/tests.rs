//! Die Tokenizer-Tests aus html5lib-tests (`tokenizer/*.test`), mit Tokens,
//! Fehlercodes und Fehlerstellen.

use mb_base::DomString;
use mb_json::Value;

use super::{Token, Tokenizer, TokenizerState};

/// Drei Tests in `domjs.test` erwarten für `<?` mit folgendem Zeilenumbruch
/// noch den Code `unexpected-question-mark-instead-of-tag-name`. Den kennt
/// der Standard nicht mehr: Seit es Verarbeitungsanweisungen gibt, führt `?`
/// in den Zustand „Processing instruction open state“, und der Umbruch ist
/// dort `invalid-first-character-of-processing-instruction-target`. Die
/// Tokens bleiben gleich. Für diese Tests gilt statt der Fehlerliste der
/// Datei der Fehler nach dem Standard an der Stelle des Umbruchs.
const OUTDATED_ERRORS: [(&str, &str); 3] = [
    ("domjs.test", "CR in bogus comment state"),
    ("domjs.test", "CRLF in bogus comment state"),
    ("domjs.test", "CRLFLF in bogus comment state"),
];

/// `xmlViolation.test` prüft die Anpassungen aus „Coercing an HTML DOM into
/// an infoset“. Die braucht nur ein Werkzeug, das aus HTML XML macht, kein
/// Browser; die Datei bleibt deshalb außen vor.
const SKIPPED_FILES: [&str; 1] = ["xmlViolation.test"];

/// Ein Token in der Form der Testdateien.
#[derive(Debug, PartialEq)]
enum Expected {
    Doctype {
        name: Option<DomString>,
        public_id: Option<DomString>,
        system_id: Option<DomString>,
        correctness: bool,
    },
    StartTag {
        name: DomString,
        attributes: Vec<(DomString, DomString)>,
        self_closing: bool,
    },
    EndTag(DomString),
    Comment(DomString),
    ProcessingInstruction(DomString, DomString),
    Character(DomString),
}

/// Ein Fehler in der Form der Testdateien: Code, Zeile, Spalte.
type ExpectedError = (String, usize, usize);

/// Die zweite Runde Entschlüsselung bei `doubleEscaped`: `\uHHHH` wird zur
/// Codeeinheit, auch zu einem einzelnen Surrogat.
fn unescape(text: &str) -> DomString {
    let mut out = DomString::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'u') {
            let digits: String = chars.clone().skip(1).take(4).collect();
            if let Ok(unit) = u16::from_str_radix(&digits, 16)
                && digits.len() == 4
            {
                chars.nth(4);
                out.push_code_unit(unit);
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn text(value: &Value, double_escaped: bool) -> DomString {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("Zeichenkette erwartet: {value:?}"));
    if double_escaped {
        unescape(text)
    } else {
        DomString::from(text)
    }
}

fn optional_text(value: &Value, double_escaped: bool) -> Option<DomString> {
    if value.is_null() {
        None
    } else {
        Some(text(value, double_escaped))
    }
}

/// Fasst benachbarte Zeichen-Tokens zusammen.
fn push(tokens: &mut Vec<Expected>, token: Expected) {
    if let Expected::Character(more) = &token
        && let Some(Expected::Character(data)) = tokens.last_mut()
    {
        data.push_dom_string(more);
        return;
    }
    tokens.push(token);
}

fn expected_tokens(output: &Value, double_escaped: bool) -> Vec<Expected> {
    let mut tokens = Vec::new();
    for token in output.as_array().expect("output ist eine Liste") {
        let parts = token.as_array().expect("ein Token ist eine Liste");
        let part = |index: usize| {
            parts
                .get(index)
                .unwrap_or_else(|| panic!("Token zu kurz: {token:?}"))
        };
        let kind = part(0).as_str().expect("Art des Tokens");
        let token = match kind {
            "DOCTYPE" => Expected::Doctype {
                name: optional_text(part(1), double_escaped),
                public_id: optional_text(part(2), double_escaped),
                system_id: optional_text(part(3), double_escaped),
                correctness: part(4).as_bool().expect("correctness"),
            },
            "StartTag" => {
                let Value::Object(members) = part(2) else {
                    panic!("Attribute erwartet: {token:?}");
                };
                let mut attributes: Vec<(DomString, DomString)> = members
                    .iter()
                    .map(|(name, value)| {
                        let name = if double_escaped {
                            unescape(name)
                        } else {
                            DomString::from(name.as_str())
                        };
                        (name, text(value, double_escaped))
                    })
                    .collect();
                attributes.sort();
                Expected::StartTag {
                    name: text(part(1), double_escaped),
                    attributes,
                    self_closing: parts.get(3).and_then(Value::as_bool).unwrap_or(false),
                }
            }
            "EndTag" => Expected::EndTag(text(part(1), double_escaped)),
            "Comment" => Expected::Comment(text(part(1), double_escaped)),
            "ProcessingInstruction" => Expected::ProcessingInstruction(
                text(part(1), double_escaped),
                text(part(2), double_escaped),
            ),
            "Character" => Expected::Character(text(part(1), double_escaped)),
            _ => panic!("unbekannte Art {kind}"),
        };
        push(&mut tokens, token);
    }
    tokens
}

fn expected_errors(errors: Option<&Value>) -> Vec<ExpectedError> {
    let Some(errors) = errors else {
        return Vec::new();
    };
    errors
        .as_array()
        .expect("errors ist eine Liste")
        .iter()
        .map(|error| {
            let number = |key: &str| match error.get(key) {
                Some(Value::Number(digits)) => digits.parse().expect("Zahl"),
                _ => panic!("{key} fehlt: {error:?}"),
            };
            let code = error.get("code").and_then(Value::as_str).expect("code");
            (code.to_owned(), number("line"), number("col"))
        })
        .collect()
}

fn state(name: &str) -> TokenizerState {
    match name {
        "Data state" => TokenizerState::Data,
        "PLAINTEXT state" => TokenizerState::Plaintext,
        "RCDATA state" => TokenizerState::Rcdata,
        "RAWTEXT state" => TokenizerState::Rawtext,
        "Script data state" => TokenizerState::ScriptData,
        "CDATA section state" => TokenizerState::CdataSection,
        _ => panic!("unbekannter Anfangszustand {name}"),
    }
}

fn run(
    input: &DomString,
    initial: TokenizerState,
    last_start_tag: Option<&str>,
) -> (Vec<Expected>, Vec<ExpectedError>) {
    let mut tokenizer = Tokenizer::from_code_points(input.code_points());
    tokenizer.collect_errors();
    tokenizer.set_state(initial);
    tokenizer.set_last_start_tag(last_start_tag.map(DomString::from));
    let mut tokens = Vec::new();
    loop {
        let token = match tokenizer.next_token() {
            Token::Eof => break,
            Token::Doctype(doctype) => Expected::Doctype {
                name: doctype.name,
                public_id: doctype.public_id,
                system_id: doctype.system_id,
                correctness: !doctype.force_quirks,
            },
            Token::StartTag(tag) => {
                let mut attributes: Vec<(DomString, DomString)> = tag
                    .attributes
                    .into_iter()
                    .map(|attribute| (attribute.name, attribute.value))
                    .collect();
                attributes.sort();
                Expected::StartTag {
                    name: tag.name,
                    attributes,
                    self_closing: tag.self_closing,
                }
            }
            Token::EndTag(tag) => Expected::EndTag(tag.name),
            Token::Comment(data) => Expected::Comment(data),
            Token::ProcessingInstruction { target, data } => {
                Expected::ProcessingInstruction(target, data)
            }
            Token::Character(code_point) => {
                let mut data = DomString::new();
                data.push_code_point(code_point);
                Expected::Character(data)
            }
        };
        push(&mut tokens, token);
    }
    let errors = tokenizer
        .take_errors()
        .into_iter()
        .map(|error| {
            let code = error.code.map_or("(ohne Code)", |code| code.name());
            (code.to_owned(), error.line, error.column)
        })
        .collect();
    (tokens, errors)
}

#[test]
fn html5lib_tokenizer() {
    let files = testdata::files("html5lib-tests", "tokenizer", "test");
    let mut total = 0;
    let mut failures = Vec::new();
    for file in &files {
        if SKIPPED_FILES.contains(&file.as_str()) {
            continue;
        }
        let source = testdata::read("html5lib-tests", &format!("tokenizer/{file}"));
        let data = mb_json::parse(&source).unwrap_or_else(|error| panic!("{file}: {error}"));
        let tests = data
            .get("tests")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{file}: keine Liste tests"));
        for test in tests {
            let description = test
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            let double_escaped = test
                .get("doubleEscaped")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let input = text(test.get("input").expect("input"), double_escaped);
            let expected = expected_tokens(test.get("output").expect("output"), double_escaped);
            let outdated = OUTDATED_ERRORS.contains(&(file.as_str(), description));
            let errors = if outdated {
                // Der Umbruch steht als drittes Zeichen in Zeile 1.
                vec![(
                    "invalid-first-character-of-processing-instruction-target".to_owned(),
                    1,
                    3,
                )]
            } else {
                expected_errors(test.get("errors"))
            };
            let last_start_tag = test.get("lastStartTag").and_then(Value::as_str);
            let states: Vec<&str> = match test.get("initialStates") {
                Some(states) => states
                    .as_array()
                    .expect("initialStates ist eine Liste")
                    .iter()
                    .map(|name| name.as_str().expect("Name eines Zustands"))
                    .collect(),
                None => vec!["Data state"],
            };
            for name in states {
                total += 1;
                let (tokens, found) = run(&input, state(name), last_start_tag);
                if tokens != expected || found != errors {
                    failures.push(format!(
                        "{file}: {description} ({name})\n  Eingabe: {input:?}\n  erwartet: {expected:?} {errors:?}\n  erhalten: {tokens:?} {found:?}"
                    ));
                }
            }
        }
    }
    assert!(total > 6000, "zu wenige Tests gefunden: {total}");
    let shown: Vec<&String> = failures.iter().take(60).collect();
    assert!(
        failures.is_empty(),
        "{} von {total} Tests weichen ab:\n{}",
        failures.len(),
        shown
            .iter()
            .map(|failure| failure.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn unescape_double_escaped_input() {
    let text = unescape("a\\uD800b\\u0041\\u12");
    let units: Vec<u16> = text.code_units().collect();
    assert_eq!(units, [0x61, 0xD800, 0x62, 0x41, 0x5C, 0x75, 0x31, 0x32]);
}
