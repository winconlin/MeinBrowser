//! Tests mit css-parsing-tests (`tools/testdata`). Die Sammlung beruht auf
//! einem älteren Entwurf von CSS Syntax Level 3 (laut ihrer README auf dem
//! von 2021). Benutzt werden nur die Dateien, deren Darstellung zum
//! aktuellen Entwurf passt:
//!
//! - `component_value_list.json` und `one_component_value.json` für den
//!   Tokenizer und die Komponentenwerte,
//! - `stylesheet_bytes.json` für das Dekodieren. Die Regeln darin sind nur
//!   At-Regeln ohne Block.
//! - `An+B.json` für das An+B-Format.
//!
//! Die übrigen Dateien zeigen die Blöcke von Regeln als rohe
//! Komponentenwerte, behalten At-Regeln in Deklarationslisten und kürzen
//! Leerraum anders. Der aktuelle Entwurf liest Blöcke als Regeln und
//! Deklarationen; dafür gibt es eigene Tests in `parser.rs`.
//!
//! So liest dieser Test die Daten:
//!
//! - `["error", "eof-in-string"]` und `["error", "eof-in-url"]` markieren
//!   Parse-Fehler, keine Tokens. Sie ändern das Ergebnis nicht und werden
//!   übersprungen.
//! - `["error", "invalid"]` markiert eine Regel, die der Parser verwirft;
//!   der Entwurf liefert für sie nichts.
//! - `["error", "empty"]` und `["error", "extra-input"]` stehen für den
//!   *syntax error* eines Einstiegs.
//! - Von der Darstellung einer Zahl als Text prüft der Test nur das
//!   Vorzeichen, denn nur das gehört zum Token. Prozentwerte haben im Entwurf
//!   kein Typ-Flag; die Angabe in den Daten wird nicht geprüft.
//! - Fälle mit ⟨unicode-range-token⟩s laufen über *consume the value of a
//!   unicode-range descriptor*, denn nur dort erzeugt der aktuelle Tokenizer
//!   sie. Im älteren Entwurf tat er es immer.

use mb_encoding::Encoding;
use mb_json::Value;

use crate::{
    ComponentValue, Rule, Sign, Source, Syntax, Token, Values, decode_stylesheet, parse_an_plus_b,
    parse_component_value, parse_component_value_list, parse_stylesheet,
};

/// Fälle, in denen die Daten dem aktuellen Entwurf widersprechen, mit
/// Begründung. Der Test prüft, dass sie weiter abweichen.
const DEVIATIONS: &[(&str, &str)] = &[
    (
        "\\- red0 -red --red -\\-red\\ blue 0red -0red \0red _Red .red rêd r\\êd \u{7F}\u{80}\u{81}",
        "Ein „non-ASCII ident code point“ beginnt im Entwurf erst bei U+00B7, \
         U+0080 und U+0081 sind also ⟨delim-token⟩s. In den Daten zählt alles ab U+0080.",
    ),
    (
        "~=|=^=$=*=||<!------> |/**/| ~/**/=",
        "Der Entwurf kennt keine ⟨include-match-token⟩, ⟨dash-match-token⟩, \
         ⟨prefix-match-token⟩, ⟨suffix-match-token⟩, ⟨substring-match-token⟩ \
         und ⟨column-token⟩ mehr; `~=` sind zwei ⟨delim-token⟩s.",
    ),
    (
        "a:not([href^=http\\:],  [href ^=\t'https\\:'\n]) { color: rgba(0%, 100%, 50%); }",
        "Wie oben: `^=` sind zwei ⟨delim-token⟩s.",
    ),
];

fn data(name: &str) -> Value {
    let text = testdata::read("css-parsing-tests", name);
    mb_json::parse(&text).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// Die Paare aus Eingabe und Erwartung einer Datei.
fn pairs(data: &Value) -> Vec<(&Value, &Value)> {
    let (pairs, rest) = data.as_array().expect("eine Liste").as_chunks::<2>();
    assert!(rest.is_empty(), "ungerade Zahl von Einträgen");
    pairs
        .iter()
        .map(|[input, output]| (input, output))
        .collect()
}

fn string(text: &str) -> Value {
    Value::String(text.to_owned())
}

fn list(items: impl IntoIterator<Item = Value>) -> Value {
    Value::Array(items.into_iter().collect())
}

fn number(value: f64) -> Value {
    // −0 und 0 sind derselbe Wert; das Vorzeichen prüft der Test getrennt.
    Value::Number(format!("{:?}", value + 0.0))
}

fn is_marker(value: &Value, kinds: &[&str]) -> bool {
    match value.as_array() {
        Some([kind, what]) => {
            kind.as_str() == Some("error")
                && what.as_str().is_some_and(|what| kinds.contains(&what))
        }
        _ => false,
    }
}

/// Die Erwartung in der Form, die [`actual`] erzeugt (siehe Moduldoku).
fn expected(value: &Value) -> Value {
    match value {
        Value::Number(text) => number(text.parse().unwrap_or(f64::NAN)),
        Value::Array(items) => {
            let kind = items.first().and_then(Value::as_str);
            let sign = || {
                let repr = items.get(1).and_then(Value::as_str).unwrap_or_default();
                string(match repr.chars().next() {
                    Some('+') => "+",
                    Some('-') => "-",
                    _ => "",
                })
            };
            let at = |index: usize| items.get(index).map(expected).unwrap_or(Value::Null);
            match kind {
                Some("number") => list([string("number"), sign(), at(2), at(3)]),
                Some("percentage") => list([string("percentage"), sign(), at(2)]),
                Some("dimension") => list([string("dimension"), sign(), at(2), at(3), at(4)]),
                _ => list(
                    items
                        .iter()
                        .filter(|item| {
                            !is_marker(item, &["eof-in-string", "eof-in-url", "invalid"])
                        })
                        .map(expected),
                ),
            }
        }
        Value::String(text) => string(text),
        Value::Bool(value) => Value::Bool(*value),
        Value::Null => Value::Null,
        Value::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, value)| (key.clone(), expected(value)))
                .collect(),
        ),
    }
}

fn sign(sign: Option<Sign>) -> Value {
    string(match sign {
        Some(Sign::Plus) => "+",
        Some(Sign::Minus) => "-",
        None => "",
    })
}

fn kind(integer: bool) -> Value {
    string(if integer { "integer" } else { "number" })
}

fn actual_token(token: &Token) -> Value {
    let error = |what: &str| list([string("error"), string(what)]);
    let named = |kind: &str, value: &str| list([string(kind), string(value)]);
    match token {
        Token::Ident(value) => named("ident", value),
        Token::Function(value) => named("function-token", value),
        Token::AtKeyword(value) => named("at-keyword", value),
        Token::Hash { value, id } => list([
            string("hash"),
            string(value),
            string(if *id { "id" } else { "unrestricted" }),
        ]),
        Token::String(value) => named("string", value),
        Token::BadString => error("bad-string"),
        Token::Url(value) => named("url", value),
        Token::BadUrl => error("bad-url"),
        Token::Delim(value) => Value::String(value.to_string()),
        Token::Number {
            value,
            integer,
            sign: s,
        } => list([string("number"), sign(*s), number(*value), kind(*integer)]),
        Token::Percentage { value, sign: s } => {
            list([string("percentage"), sign(*s), number(*value)])
        }
        Token::Dimension {
            value,
            integer,
            sign: s,
            unit,
        } => list([
            string("dimension"),
            sign(*s),
            number(*value),
            kind(*integer),
            string(unit),
        ]),
        Token::UnicodeRange { start, end } => list([
            string("unicode-range"),
            number(f64::from(*start)),
            number(f64::from(*end)),
        ]),
        Token::Whitespace => string(" "),
        Token::Cdo => string("<!--"),
        Token::Cdc => string("-->"),
        Token::Colon => string(":"),
        Token::Semicolon => string(";"),
        Token::Comma => string(","),
        Token::CloseBracket => error("]"),
        Token::CloseParenthesis => error(")"),
        Token::CloseBrace => error("}"),
        Token::OpenBracket => string("[-token"),
        Token::OpenParenthesis => string("(-token"),
        Token::OpenBrace => string("{-token"),
    }
}

fn actual_value(value: ComponentValue<'_>) -> Value {
    match value {
        ComponentValue::Token(token) => actual_token(token),
        ComponentValue::Function { name, value } => list(
            [string("function"), string(name)]
                .into_iter()
                .chain(actual_values(value)),
        ),
        ComponentValue::Block { kind, value } => {
            let kind = match kind {
                crate::BlockKind::Brace => "{}",
                crate::BlockKind::Bracket => "[]",
                crate::BlockKind::Parenthesis => "()",
            };
            list([string(kind)].into_iter().chain(actual_values(value)))
        }
    }
}

fn actual_values(values: Values<'_>) -> Vec<Value> {
    values.iter().map(actual_value).collect()
}

fn contains_unicode_range(value: &Value) -> bool {
    match value {
        Value::Array(items) => {
            items.first().and_then(Value::as_str) == Some("unicode-range")
                || items.iter().any(contains_unicode_range)
        }
        _ => false,
    }
}

/// Vergleicht alle Fälle einer Datei und endet mit allen Abweichungen auf
/// einmal, außer den bekannten in [`DEVIATIONS`].
fn check(name: &str, run: impl Fn(&Value, &Value) -> (Value, Value)) {
    let data = data(name);
    let cases = pairs(&data);
    assert!(!cases.is_empty(), "{name}: keine Testfälle");
    let mut failures = Vec::new();
    for (input, expectation) in &cases {
        let (wanted, got) = run(input, expectation);
        let label = input
            .as_str()
            .map_or_else(|| format!("{input:?}"), str::to_owned);
        let known = DEVIATIONS.iter().any(|(case, _)| *case == label);
        if known {
            assert_ne!(wanted, got, "{name}: {label:?} weicht nicht mehr ab");
        } else if wanted != got {
            failures.push(format!(
                "{label:?}\n  erwartet: {wanted:?}\n  erhalten: {got:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{name}: {} von {} Fällen weichen ab:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn deviations_exist() {
    let files = ["component_value_list.json", "one_component_value.json"];
    let inputs: Vec<String> = files
        .iter()
        .flat_map(|name| {
            let data = data(name);
            pairs(&data)
                .iter()
                .filter_map(|(input, _)| input.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect();
    for (case, reason) in DEVIATIONS {
        assert!(!reason.is_empty());
        assert!(
            inputs.iter().any(|input| input == case),
            "{case:?} fehlt in den Daten"
        );
    }
}

#[test]
fn component_value_list() {
    check("component_value_list.json", |input, expectation| {
        let text = input.as_str().expect("Eingabe als Text");
        let wanted = expected(expectation);
        let values = if contains_unicode_range(&wanted) {
            crate::parser::consume_unicode_range_value(Source::new(text).chars())
        } else {
            parse_component_value_list(&Source::new(text))
        };
        (wanted, list(actual_values(values.values())))
    });
}

#[test]
fn one_component_value() {
    check("one_component_value.json", |input, expectation| {
        let text = input.as_str().expect("Eingabe als Text");
        let wanted = if is_marker(expectation, &["empty", "extra-input"]) {
            string("syntax error")
        } else {
            expected(expectation)
        };
        let got = match parse_component_value(&Source::new(text)) {
            Ok(values) => list(actual_values(values.values())),
            Err(_) => string("syntax error"),
        };
        // Das Ergebnis ist eine Liste mit genau einem Wert.
        let got = match got {
            Value::Array(mut items) if items.len() == 1 => items.remove(0),
            other => other,
        };
        (wanted, got)
    });
}

#[test]
fn stylesheet_bytes() {
    check("stylesheet_bytes.json", |input, expectation| {
        let field = |key| input.get(key).and_then(Value::as_str);
        let bytes: Vec<u8> = field("css_bytes")
            .expect("css_bytes")
            .chars()
            .map(|c| u8::try_from(u32::from(c)).expect("ein Byte je Zeichen"))
            .collect();
        let protocol = field("protocol_encoding").map(str::as_bytes);
        let environment = field("environment_encoding")
            .map(|label| Encoding::for_label(label.as_bytes()).expect("bekannte Kodierung"));
        let (text, encoding) = decode_stylesheet(&bytes, protocol, environment);
        let tree = parse_stylesheet(&Source::new(&text), &Syntax);
        let rules = tree.roots().iter().map(|&rule| match tree.get(rule) {
            Some(Rule::At(rule)) if rule.block.is_none() => list([
                string("at-rule"),
                string(&rule.name),
                list(actual_values(rule.prelude.values())),
                Value::Null,
            ]),
            other => string(&format!("unerwartet: {other:?}")),
        });
        let got = list([list(rules), string(&encoding.name().to_ascii_lowercase())]);
        (expected(expectation), got)
    });
}

#[test]
fn an_plus_b() {
    check("An+B.json", |input, expectation| {
        let text = input.as_str().expect("Eingabe als Text");
        let values = parse_component_value_list(&Source::new(text));
        let values: Vec<ComponentValue<'_>> = values.iter().collect();
        let got = match parse_an_plus_b(&values) {
            Some(value) => list([number(f64::from(value.a)), number(f64::from(value.b))]),
            None => Value::Null,
        };
        (expected(expectation), got)
    });
}
