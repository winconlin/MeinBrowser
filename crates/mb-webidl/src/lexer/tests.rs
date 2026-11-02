use super::*;

fn significant(source: &str) -> Vec<(Kind, &str)> {
    Lexer::new(source)
        .filter(|token| !token.kind.is_trivia())
        .map(|token| (token.kind, token.text))
        .collect()
}

#[test]
fn interface_fragment_keeps_keywords_identifiers_and_positions() {
    let source = "[Exposed=Window] interface Example { readonly attribute DOMString name; };";
    let tokens = significant(source);
    assert_eq!(
        tokens
            .iter()
            .filter(|(kind, _)| *kind == Kind::Identifier)
            .map(|(_, text)| *text)
            .collect::<Vec<_>>(),
        ["Exposed", "Window", "Example", "name"]
    );
    assert_eq!(
        tokens
            .iter()
            .filter(|(kind, _)| *kind == Kind::Terminal)
            .map(|(_, text)| *text)
            .collect::<Vec<_>>(),
        [
            "[",
            "=",
            "]",
            "interface",
            "{",
            "readonly",
            "attribute",
            "DOMString",
            ";",
            "}",
            ";"
        ]
    );
    let mut position = 0;
    for token in Lexer::new(source) {
        assert_eq!(token.span.start, position);
        assert_eq!(source.get(token.span.clone()), Some(token.text));
        position = token.span.end;
    }
    assert_eq!(position, source.len());
}

#[test]
fn keywords_are_case_sensitive_and_never_split_identifier_prefixes() {
    assert!(KEYWORDS.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(KEYWORDS.len(), 70);
    for &word in KEYWORDS {
        assert_eq!(significant(word), [(Kind::Terminal, word)]);
        let escaped = format!("_{word}");
        if word != "-Infinity" {
            assert_eq!(
                significant(&escaped),
                [(Kind::Identifier, escaped.as_str())]
            );
        }
        let longer = format!("{word}X");
        assert_eq!(significant(&longer), [(Kind::Identifier, longer.as_str())]);
    }
    assert_eq!(
        significant("Const _interface a1 a-b __x"),
        [
            (Kind::Identifier, "Const"),
            (Kind::Identifier, "_interface"),
            (Kind::Identifier, "a1"),
            (Kind::Identifier, "a-b"),
            (Kind::Other, "_"),
            (Kind::Identifier, "_x")
        ]
    );
}

#[test]
fn number_rules_choose_the_longest_match_without_numeric_conversion() {
    for (source, expected) in [
        ("08", vec![(Kind::Integer, "0"), (Kind::Integer, "8")]),
        ("08.5", vec![(Kind::Decimal, "08.5")]),
        ("-08e2", vec![(Kind::Decimal, "-08e2")]),
        ("0x", vec![(Kind::Integer, "0"), (Kind::Identifier, "x")]),
        ("-0Xff", vec![(Kind::Integer, "-0Xff")]),
        ("0129", vec![(Kind::Integer, "012"), (Kind::Integer, "9")]),
        (
            "1e+",
            vec![
                (Kind::Integer, "1"),
                (Kind::Identifier, "e"),
                (Kind::Other, "+"),
            ],
        ),
        (".5E-2", vec![(Kind::Decimal, ".5E-2")]),
        ("-.5", vec![(Kind::Decimal, "-.5")]),
        (
            "1...",
            vec![
                (Kind::Decimal, "1."),
                (Kind::Terminal, "."),
                (Kind::Terminal, "."),
            ],
        ),
        (
            "....5",
            vec![(Kind::Terminal, "..."), (Kind::Decimal, ".5")],
        ),
        ("-Infinity", vec![(Kind::Terminal, "-Infinity")]),
    ] {
        assert_eq!(significant(source), expected, "{source}");
    }
}

#[test]
fn strings_keep_raw_contents_and_do_not_use_javascript_escapes() {
    assert_eq!(significant("\"a\nb\\c\""), [(Kind::String, "\"a\nb\\c\"")]);
    assert_eq!(
        significant("\"a\\\"b\""),
        [
            (Kind::String, "\"a\\\""),
            (Kind::Identifier, "b"),
            (Kind::Other, "\"")
        ]
    );
    assert_eq!(significant("\""), [(Kind::Other, "\"")]);
    assert_eq!(
        significant("\"unfinished"),
        [(Kind::Other, "\""), (Kind::Identifier, "unfinished")]
    );
}

#[test]
fn comments_close_at_the_first_terminator_and_line_comments_end_at_lf() {
    let tokens: Vec<_> = Lexer::new("/* a /* b */ tail // c\rinside\u{2028}still\nnext")
        .map(|token| (token.kind, token.text))
        .collect();
    assert_eq!(
        tokens,
        [
            (Kind::Comment, "/* a /* b */"),
            (Kind::Whitespace, " "),
            (Kind::Identifier, "tail"),
            (Kind::Whitespace, " "),
            (Kind::Comment, "// c\rinside\u{2028}still"),
            (Kind::Whitespace, "\n"),
            (Kind::Identifier, "next")
        ]
    );
    assert_eq!(
        significant("/*x"),
        [
            (Kind::Other, "/"),
            (Kind::Terminal, "*"),
            (Kind::Identifier, "x")
        ]
    );
}

#[test]
fn only_four_ascii_whitespace_scalars_are_trivia() {
    let tokens: Vec<_> = Lexer::new(" \t\r\n\u{000c}\u{00a0}\u{feff}é🦀").collect();
    assert_eq!(tokens[0].kind, Kind::Whitespace);
    assert_eq!(tokens[0].text, " \t\r\n");
    assert!(tokens[1..].iter().all(|token| token.kind == Kind::Other));
    assert_eq!(tokens[4].text, "é");
    assert_eq!(tokens[5].text, "🦀");
    for token in tokens {
        assert_eq!(token.span.len(), token.text.len());
    }
}

#[test]
fn fixed_punctuation_and_ellipsis_take_priority_over_other() {
    for text in [
        "(", ")", "*", ",", "-", ".", "...", ":", ";", "<", "=", ">", "?", "[", "]", "{", "}",
    ] {
        assert_eq!(significant(text), [(Kind::Terminal, text)]);
    }
    for text in ["+", "/", "_", "'", "!", "\0"] {
        assert_eq!(significant(text), [(Kind::Other, text)]);
    }
}

#[test]
fn end_is_stable_and_iterator_omits_it() {
    let mut lexer = Lexer::new("a");
    assert_eq!(lexer.size_hint(), (0, Some(1)));
    assert_eq!(lexer.next().expect("identifier").text, "a");
    assert!(lexer.next().is_none());
    let end = Token {
        kind: Kind::End,
        span: 1..1,
        text: "",
    };
    assert_eq!(lexer.next_token(), end);
    assert_eq!(lexer.next_token(), end);
    assert!(lexer.next().is_none());
    assert_eq!(Lexer::new("").count(), 0);
}

#[test]
fn many_unclosed_comment_prefixes_and_large_numbers_finish_without_recursion() {
    let comments = "/*x".repeat(20_000);
    assert_eq!(Lexer::new(&comments).count(), comments.len());
    let integer = format!("0x{}", "f".repeat(50_000));
    assert_eq!(significant(&integer), [(Kind::Integer, integer.as_str())]);
    let decimal = format!("{}.0e-{}", "0".repeat(50_000), "9".repeat(50_000));
    assert_eq!(significant(&decimal), [(Kind::Decimal, decimal.as_str())]);
}

// Vollständige Treffer statt Streaming-Zuständen: unabhängiger kleiner
// Regex-Ersatz für die begrenzte exhaustive Zahlenprüfung, nur im Test.
fn whole_number(text: &str, decimal: bool) -> bool {
    let text = text.strip_prefix('-').unwrap_or(text);
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|byte| byte.is_ascii_digit());
    if !decimal {
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            return !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
        }
        return if text.starts_with('0') {
            text.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
        } else {
            digits(text)
        };
    }
    let pieces: Vec<_> = text.split(['E', 'e']).collect();
    if pieces.len() > 2 {
        return false;
    }
    if let Some(exponent) = pieces.get(1) {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if !digits(exponent) {
            return false;
        }
    }
    let mantissa: Vec<_> = pieces[0].split('.').collect();
    if mantissa.len() == 1 {
        return pieces.len() == 2 && digits(mantissa[0]);
    }
    mantissa.len() == 2
        && (!mantissa[0].is_empty() || !mantissa[1].is_empty())
        && mantissa
            .iter()
            .all(|s| s.bytes().all(|byte| byte.is_ascii_digit()))
}

#[test]
fn short_numeric_inputs_match_an_exhaustive_whole_token_oracle() {
    let alphabet = b"08xe-+.";
    for length in 1..=4_u32 {
        for mut number in 0..alphabet.len().pow(length) {
            let mut source = String::new();
            for _ in 0..length {
                source.push(char::from(alphabet[number % alphabet.len()]));
                number /= alphabet.len();
            }
            let expected = (1..=source.len())
                .filter_map(|end| {
                    let text = &source[..end];
                    if whole_number(text, false) {
                        Some((Kind::Integer, end))
                    } else if whole_number(text, true) {
                        Some((Kind::Decimal, end))
                    } else {
                        None
                    }
                })
                .next_back();
            let token = Lexer::new(&source).next().expect("token");
            if let Some((kind, end)) = expected {
                assert_eq!((token.kind, token.span.end), (kind, end), "{source}");
            } else {
                assert!(
                    !matches!(token.kind, Kind::Integer | Kind::Decimal),
                    "{source}"
                );
            }
        }
    }
}
