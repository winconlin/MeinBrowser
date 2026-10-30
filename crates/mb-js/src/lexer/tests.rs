//! Fälle aus der lexikalischen Grammatik von ECMA-262 und Anhang B.1.1.

use super::*;

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Alle Tokens bis zum Ende mit demselben Ziel, ohne `End`.
fn lex_all(source: &[u16], goal: Goal, module: bool) -> Result<Vec<Token>, Error> {
    let mut lexer = Lexer::new(source, module);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token(goal)?;
        if token.kind == TokenKind::End {
            return Ok(tokens);
        }
        tokens.push(token);
    }
}

fn kinds(text: &str, goal: Goal) -> Vec<TokenKind> {
    lex_all(&utf16(text), goal, false)
        .unwrap_or_else(|error| panic!("{text:?}: {error:?}"))
        .into_iter()
        .map(|token| token.kind)
        .collect()
}

fn error(text: &str, goal: Goal) -> ErrorKind {
    match lex_all(&utf16(text), goal, false) {
        Ok(tokens) => panic!("{text:?} ergab {tokens:?}"),
        Err(error) => error.kind,
    }
}

fn name(value: &str) -> TokenKind {
    TokenKind::Name(Name {
        value: value.to_owned(),
        escaped: false,
    })
}

fn escaped(value: &str) -> TokenKind {
    TokenKind::Name(Name {
        value: value.to_owned(),
        escaped: true,
    })
}

fn punctuator(punctuator: Punctuator) -> TokenKind {
    TokenKind::Punctuator(punctuator)
}

fn number(value: f64) -> TokenKind {
    TokenKind::Number(Number {
        value,
        legacy: false,
    })
}

fn legacy(value: f64) -> TokenKind {
    TokenKind::Number(Number {
        value,
        legacy: true,
    })
}

fn string(units: &[u16]) -> TokenKind {
    TokenKind::String(StringLiteral {
        value: units.to_vec(),
        legacy_escape: false,
    })
}

/// Der Wert eines einzelnen Tokens.
fn single(text: &str, goal: Goal) -> TokenKind {
    let mut kinds = kinds(text, goal);
    assert_eq!(kinds.len(), 1, "{text:?}: {kinds:?}");
    kinds.remove(0)
}

#[test]
fn white_space_and_line_terminators() {
    let source = "a\t\u{B}\u{C}\u{FEFF}\u{A0}\u{3000} b\u{2028}c\r\nd\u{2029}e";
    let tokens = lex_all(&utf16(source), Goal::Div, false).expect("Tokens");
    let names: Vec<(TokenKind, bool)> = tokens
        .into_iter()
        .map(|token| (token.kind, token.newline_before))
        .collect();
    assert_eq!(
        names,
        [
            (name("a"), false),
            (name("b"), false),
            (name("c"), true),
            (name("d"), true),
            (name("e"), true),
        ]
    );
    // Cf seit Unicode 6.3 und NEL (Cc) sind kein Leerraum.
    assert_eq!(
        error("a\u{180E}b", Goal::Div),
        ErrorKind::UnexpectedCharacter
    );
    assert_eq!(error("a\u{85}b", Goal::Div), ErrorKind::UnexpectedCharacter);
}

#[test]
fn comments() {
    let newline = |text: &str| -> Vec<bool> {
        lex_all(&utf16(text), Goal::Div, false)
            .expect("Tokens")
            .into_iter()
            .map(|token| token.newline_before)
            .collect()
    };
    assert_eq!(newline("a /* x */ b"), [false, false]);
    assert_eq!(newline("a /* x\n */ b"), [false, true]);
    assert_eq!(newline("a /** x **/ b"), [false, false]);
    assert_eq!(newline("a // x\nb"), [false, true]);
    assert_eq!(newline("a // x"), [false]);
    assert_eq!(error("a /* x", Goal::Div), ErrorKind::UnterminatedComment);
    assert_eq!(error("/*/", Goal::Div), ErrorKind::UnterminatedComment);
    // Escapes in Kommentaren bleiben Text.
    assert_eq!(kinds("// \\u000A b", Goal::Div), []);
}

#[test]
fn hashbang_only_at_the_start_of_a_script_or_module() {
    let source = utf16("#!/usr/bin/env node\nx");
    for module in [false, true] {
        let tokens = lex_all(&source, Goal::HashbangOrRegExp, module).expect("Tokens");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, name("x"));
        assert!(tokens[0].newline_before);
    }
    assert_eq!(error("#!x", Goal::RegExp), ErrorKind::UnexpectedCharacter);
    assert_eq!(
        error(" #!x", Goal::HashbangOrRegExp),
        ErrorKind::UnexpectedCharacter
    );
}

#[test]
fn html_like_comments_in_scripts() {
    assert_eq!(kinds("x <!-- y\nz", Goal::Div), [name("x"), name("z")]);
    assert_eq!(kinds("x\n--> y\nz", Goal::Div), [name("x"), name("z")]);
    assert_eq!(kinds("x\n /* a */ /* b */ --> y", Goal::Div), [name("x")]);
    assert_eq!(kinds("x /*\n*/ --> y", Goal::Div), [name("x")]);
    assert_eq!(kinds("-->x", Goal::HashbangOrRegExp), []);
    assert_eq!(
        kinds("x /* */ --> y", Goal::Div),
        [
            name("x"),
            punctuator(Punctuator::MinusMinus),
            punctuator(Punctuator::Greater),
            name("y"),
        ]
    );
    assert_eq!(
        kinds("-->x", Goal::Div),
        [
            punctuator(Punctuator::MinusMinus),
            punctuator(Punctuator::Greater),
            name("x"),
        ]
    );
}

#[test]
fn no_html_like_comments_in_modules() {
    let tokens = lex_all(&utf16("x <!-- y\n--> z"), Goal::Div, true).expect("Tokens");
    let kinds: Vec<TokenKind> = tokens.into_iter().map(|token| token.kind).collect();
    assert_eq!(
        kinds,
        [
            name("x"),
            punctuator(Punctuator::Less),
            punctuator(Punctuator::Bang),
            punctuator(Punctuator::MinusMinus),
            name("y"),
            punctuator(Punctuator::MinusMinus),
            punctuator(Punctuator::Greater),
            name("z"),
        ]
    );
}

#[test]
fn identifier_names() {
    assert_eq!(
        kinds("abc $ _x a1 if \u{2118} \u{1D465}", Goal::Div),
        [
            name("abc"),
            name("$"),
            name("_x"),
            name("a1"),
            name("if"),
            name("\u{2118}"),
            name("\u{1D465}"),
        ]
    );
    assert_eq!(
        kinds(
            "\\u0061b \\u{62}c i\\u0066 a\\u200Cb \\u{000000041}",
            Goal::Div
        ),
        [
            escaped("ab"),
            escaped("bc"),
            escaped("if"),
            escaped("a\u{200C}b"),
            escaped("A"),
        ]
    );
    // Escapes, die keinen erlaubten Codepunkt ergeben.
    assert_eq!(error("\\u0030", Goal::Div), ErrorKind::UnexpectedCharacter);
    assert_eq!(error("a\\u0020", Goal::Div), ErrorKind::InvalidEscape);
    assert_eq!(error("a\\u{110000}", Goal::Div), ErrorKind::InvalidEscape);
    assert_eq!(error("a\\uD800", Goal::Div), ErrorKind::InvalidEscape);
    assert_eq!(error("a\\x41", Goal::Div), ErrorKind::InvalidEscape);
    assert_eq!(error("a\\u{}", Goal::Div), ErrorKind::InvalidEscape);
    // Ein einzelnes Surrogat ist kein Teil eines Namens.
    let mut source = utf16("a");
    source.push(0xD800);
    assert_eq!(
        lex_all(&source, Goal::Div, false).map_err(|error| error.kind),
        Err(ErrorKind::UnexpectedCharacter)
    );
}

#[test]
fn private_names() {
    assert_eq!(
        kinds("#x #\\u0061", Goal::Div),
        [
            TokenKind::PrivateName(Name {
                value: "x".to_owned(),
                escaped: false,
            }),
            TokenKind::PrivateName(Name {
                value: "a".to_owned(),
                escaped: true,
            }),
        ]
    );
    assert_eq!(error("#", Goal::Div), ErrorKind::UnexpectedCharacter);
    assert_eq!(error("# x", Goal::Div), ErrorKind::UnexpectedCharacter);
    assert_eq!(error("#1", Goal::Div), ErrorKind::UnexpectedCharacter);
}

#[test]
fn every_punctuator() {
    use Punctuator as P;
    let text = "{ } ( ) [ ] . ... ; , < > <= >= == != === !== + - * % ** ++ -- \
                << >> >>> & | ^ ! ~ && || ?? ? ?. : = += -= *= %= **= <<= >>= \
                >>>= &= |= ^= &&= ||= ??= => / /=";
    let expected = [
        P::LeftBrace,
        P::RightBrace,
        P::LeftParenthesis,
        P::RightParenthesis,
        P::LeftBracket,
        P::RightBracket,
        P::Dot,
        P::Ellipsis,
        P::Semicolon,
        P::Comma,
        P::Less,
        P::Greater,
        P::LessEqual,
        P::GreaterEqual,
        P::Equal,
        P::NotEqual,
        P::StrictEqual,
        P::StrictNotEqual,
        P::Plus,
        P::Minus,
        P::Star,
        P::Percent,
        P::StarStar,
        P::PlusPlus,
        P::MinusMinus,
        P::ShiftLeft,
        P::ShiftRight,
        P::UnsignedShiftRight,
        P::Ampersand,
        P::Bar,
        P::Caret,
        P::Bang,
        P::Tilde,
        P::AmpersandAmpersand,
        P::BarBar,
        P::QuestionQuestion,
        P::Question,
        P::QuestionDot,
        P::Colon,
        P::Assign,
        P::PlusAssign,
        P::MinusAssign,
        P::StarAssign,
        P::PercentAssign,
        P::StarStarAssign,
        P::ShiftLeftAssign,
        P::ShiftRightAssign,
        P::UnsignedShiftRightAssign,
        P::AmpersandAssign,
        P::BarAssign,
        P::CaretAssign,
        P::AmpersandAmpersandAssign,
        P::BarBarAssign,
        P::QuestionQuestionAssign,
        P::Arrow,
        P::Slash,
        P::SlashAssign,
    ];
    let expected: Vec<TokenKind> = expected.into_iter().map(punctuator).collect();
    assert_eq!(kinds(text, Goal::Div), expected);
}

#[test]
fn longest_punctuators_and_optional_chaining() {
    use Punctuator as P;
    assert_eq!(
        kinds("a>>>=b", Goal::Div),
        [
            name("a"),
            punctuator(P::UnsignedShiftRightAssign),
            name("b")
        ]
    );
    assert_eq!(
        kinds("a....b", Goal::Div),
        [
            name("a"),
            punctuator(P::Ellipsis),
            punctuator(P::Dot),
            name("b")
        ]
    );
    // `?.` nur ohne Ziffer danach, sonst `?` und eine Zahl.
    assert_eq!(
        kinds("x?.5:1", Goal::Div),
        [
            name("x"),
            punctuator(P::Question),
            number(0.5),
            punctuator(P::Colon),
            number(1.0),
        ]
    );
    assert_eq!(
        kinds("x?.y", Goal::Div),
        [name("x"), punctuator(P::QuestionDot), name("y")]
    );
    assert_eq!(error("a @ b", Goal::Div), ErrorKind::UnexpectedCharacter);
}

#[test]
fn the_goal_decides_slash_and_right_brace() {
    let regexp = |body: &str, flags: &str| {
        TokenKind::RegExp(RegExp {
            body: utf16(body),
            flags: utf16(flags),
        })
    };
    assert_eq!(single("/=/g", Goal::RegExp), regexp("=", "g"));
    assert_eq!(
        kinds("/=/g", Goal::Div),
        [
            punctuator(Punctuator::SlashAssign),
            punctuator(Punctuator::Slash),
            name("g"),
        ]
    );
    assert_eq!(
        kinds("}", Goal::RegExp),
        [punctuator(Punctuator::RightBrace)]
    );
    assert_eq!(
        error("}", Goal::HashbangOrRegExp),
        ErrorKind::UnexpectedCharacter
    );
    assert!(matches!(
        single("}`", Goal::TemplateTail),
        TokenKind::Template(Template {
            part: TemplatePart::Tail,
            ..
        })
    ));
}

#[test]
fn regular_expression_literals() {
    let regexp = |body: &str, flags: &str| {
        TokenKind::RegExp(RegExp {
            body: utf16(body),
            flags: utf16(flags),
        })
    };
    assert_eq!(single("/[/]/", Goal::RegExp), regexp("[/]", ""));
    assert_eq!(single("/a\\/b/gi", Goal::RegExp), regexp("a\\/b", "gi"));
    assert_eq!(single("/[\\]/]/", Goal::RegExp), regexp("[\\]/]", ""));
    assert_eq!(single("/]/", Goal::RegExp), regexp("]", ""));
    assert_eq!(
        single("/a/$\u{1D465}", Goal::RegExp),
        regexp("a", "$\u{1D465}")
    );
    // Flags sind IdentifierPartChar ohne Escapes.
    assert_eq!(
        kinds("/a/\\u0067", Goal::RegExp),
        [regexp("a", ""), escaped("g")]
    );
    for text in ["/a\nb/", "/[a\n]/", "/a", "/a\\\n/", "/a\\"] {
        assert_eq!(
            error(text, Goal::RegExp),
            ErrorKind::UnterminatedRegExp,
            "{text:?}"
        );
    }
    // `//` und `/*` sind Kommentare, keine Ausdrücke.
    assert_eq!(kinds("//a/", Goal::RegExp), []);
}

#[test]
fn decimal_numbers() {
    let cases = [
        ("0", 0.0),
        ("1", 1.0),
        ("1.5", 1.5),
        (".5", 0.5),
        ("1.", 1.0),
        ("1.e5", 100_000.0),
        ("1e3", 1000.0),
        ("1E+3", 1000.0),
        ("1e-3", 0.001),
        ("0.1", 0.1),
        ("1_000", 1000.0),
        ("1_0.0_1e1_0", 100_100_000_000.0),
        ("0.0001", 0.0001),
        ("9007199254740993", 9_007_199_254_740_992.0),
        ("1e400", f64::INFINITY),
        ("1e-400", 0.0),
        ("2.2250738585072011e-308", 2.225_073_858_507_201e-308),
    ];
    for (text, value) in cases {
        assert_eq!(single(text, Goal::Div), number(value), "{text:?}");
    }
}

#[test]
fn non_decimal_numbers_round_to_nearest_even() {
    let cases = [
        ("0x1F", 31.0),
        ("0X1f", 31.0),
        ("0o17", 15.0),
        ("0O17", 15.0),
        ("0b101", 5.0),
        ("0B1_0_1", 5.0),
        ("0x1_F", 31.0),
        ("0x0", 0.0),
        ("0x0000_0001", 1.0),
        ("0x1fffffffffffff", 9_007_199_254_740_991.0),
        // 2^53 + 1 und 2^53 + 3 liegen genau zwischen zwei Zahlen.
        ("0x20000000000001", 9_007_199_254_740_992.0),
        ("0x20000000000003", 9_007_199_254_740_996.0),
        // Knapp über der Mitte: ein gesetztes Bit hinter den ersten 64.
        ("0x200000000000010001", 590_295_810_358_705_717_249.0),
    ];
    for (text, value) in cases {
        assert_eq!(single(text, Goal::Div), number(value), "{text:?}");
    }
    let zeros = "0".repeat(242);
    assert_eq!(
        single(&format!("0xfffffffffffff8{zeros}"), Goal::Div),
        number(f64::MAX)
    );
    assert_eq!(
        single(&format!("0xfffffffffffffbff{}", "f".repeat(240)), Goal::Div),
        number(f64::MAX)
    );
    // Genau die Mitte zwischen der größten Zahl und 2^1024.
    assert_eq!(
        single(&format!("0xfffffffffffffc{zeros}"), Goal::Div),
        number(f64::INFINITY)
    );
    assert_eq!(
        single(&format!("0x1{}", "0".repeat(300)), Goal::Div),
        number(f64::INFINITY)
    );
}

#[test]
fn legacy_numbers() {
    assert_eq!(single("017", Goal::Div), legacy(15.0));
    assert_eq!(single("00", Goal::Div), legacy(0.0));
    assert_eq!(single("08", Goal::Div), legacy(8.0));
    assert_eq!(single("019", Goal::Div), legacy(19.0));
    assert_eq!(single("08.5", Goal::Div), legacy(8.5));
    assert_eq!(single("09e1", Goal::Div), legacy(90.0));
    // Ein *LegacyOctalIntegerLiteral* hat keinen Nachkommateil.
    assert_eq!(kinds("07.5", Goal::Div), [legacy(7.0), number(0.5)]);
}

#[test]
fn big_integers() {
    let big = |radix: u32, digits: &str| {
        TokenKind::BigInt(BigInt {
            radix,
            digits: digits.to_owned(),
        })
    };
    assert_eq!(single("0n", Goal::Div), big(10, "0"));
    assert_eq!(single("123n", Goal::Div), big(10, "123"));
    assert_eq!(single("1_0n", Goal::Div), big(10, "10"));
    assert_eq!(single("0x1Fn", Goal::Div), big(16, "1F"));
    assert_eq!(single("0o7n", Goal::Div), big(8, "7"));
    assert_eq!(single("0b1n", Goal::Div), big(2, "1"));
    for text in ["1.5n", "1e3n", "01n", "08n", ".5n"] {
        assert_eq!(error(text, Goal::Div), ErrorKind::InvalidNumber, "{text:?}");
    }
}

#[test]
fn invalid_numbers() {
    for text in [
        "1_", "1__0", "0_1", "1._5", "1_.5", "1e", "1e+", "1e_5", "3in", "0x", "0x_1", "0b2",
        "0o8", "1\\u0061", "00_1", "08_1", "0x1__0", "1a", "1$",
    ] {
        assert_eq!(error(text, Goal::Div), ErrorKind::InvalidNumber, "{text:?}");
    }
}

#[test]
fn string_literals() {
    let text = "'\\b\\f\\n\\r\\t\\v\\'\\\"\\\\'";
    assert_eq!(
        single(text, Goal::Div),
        string(&[0x08, 0x0C, 0x0A, 0x0D, 0x09, 0x0B, 0x27, 0x22, 0x5C])
    );
    assert_eq!(
        single("\"\\x41\\u0041\\u{41}\\u{1F600}\"", Goal::Div),
        string(&utf16("AAA\u{1F600}"))
    );
    // Ein einzelnes Surrogat bleibt eine Codeeinheit.
    assert_eq!(single("'\\uD800'", Goal::Div), string(&[0xD800]));
    let mut source = utf16("'");
    source.extend([0xDC00, 0x27]);
    assert_eq!(
        lex_all(&source, Goal::Div, false).expect("Token")[0].kind,
        string(&[0xDC00])
    );
    assert_eq!(single("'\\a\\%'", Goal::Div), string(&utf16("a%")));
    assert_eq!(single("'\\0'", Goal::Div), string(&[0]));
    // Zeilenfortsetzungen, auch mit CR LF, LS und PS.
    assert_eq!(
        single("'a\\\nb\\\r\nc\\\u{2028}d'", Goal::Div),
        string(&utf16("abcd"))
    );
    // LS und PS dürfen unmaskiert vorkommen, LF und CR nicht.
    assert_eq!(
        single("'\u{2028}\u{2029}'", Goal::Div),
        string(&[0x2028, 0x2029])
    );
    for text in ["'a\nb'", "'a\rb'", "'abc", "\"abc'", "'\\"] {
        assert_eq!(
            error(text, Goal::Div),
            ErrorKind::UnterminatedString,
            "{text:?}"
        );
    }
    for text in [
        "'\\x4'",
        "'\\xg0'",
        "'\\u004'",
        "'\\u{}'",
        "'\\u{110000}'",
        "'\\u{41'",
    ] {
        assert_eq!(error(text, Goal::Div), ErrorKind::InvalidEscape, "{text:?}");
    }
}

#[test]
fn legacy_escapes_in_strings() {
    let legacy = |units: &[u16]| {
        TokenKind::String(StringLiteral {
            value: units.to_vec(),
            legacy_escape: true,
        })
    };
    assert_eq!(single("'\\00'", Goal::Div), legacy(&[0]));
    assert_eq!(single("'\\08'", Goal::Div), legacy(&[0, 0x38]));
    assert_eq!(single("'\\101'", Goal::Div), legacy(&utf16("A")));
    assert_eq!(single("'\\377'", Goal::Div), legacy(&[0xFF]));
    assert_eq!(single("'\\400'", Goal::Div), legacy(&[0o40, 0x30]));
    assert_eq!(single("'\\1a'", Goal::Div), legacy(&[1, 0x61]));
    assert_eq!(single("'\\0000'", Goal::Div), legacy(&[0, 0x30]));
    assert_eq!(single("'\\8'", Goal::Div), legacy(&[0x38]));
    assert_eq!(single("'\\9'", Goal::Div), legacy(&[0x39]));
}

#[test]
fn templates() {
    let template = |part: TemplatePart, cooked: Option<&str>, raw: &str| {
        TokenKind::Template(Template {
            part,
            cooked: cooked.map(utf16),
            raw: utf16(raw),
        })
    };
    assert_eq!(
        single("`a`", Goal::Div),
        template(TemplatePart::NoSubstitution, Some("a"), "a")
    );
    assert_eq!(
        single("`a${", Goal::Div),
        template(TemplatePart::Head, Some("a"), "a")
    );
    assert_eq!(
        single("}b${", Goal::TemplateTail),
        template(TemplatePart::Middle, Some("b"), "b")
    );
    assert_eq!(
        single("}c`", Goal::RegExpOrTemplateTail),
        template(TemplatePart::Tail, Some("c"), "c")
    );
    assert_eq!(
        single("`$ $$ {`", Goal::Div),
        template(TemplatePart::NoSubstitution, Some("$ $$ {"), "$ $$ {")
    );
    assert_eq!(
        single("`\\x41\\u{42}\\n`", Goal::Div),
        template(
            TemplatePart::NoSubstitution,
            Some("AB\n"),
            "\\x41\\u{42}\\n"
        )
    );
    // CR und CR LF werden in beiden Werten LF.
    assert_eq!(
        single("`a\r\nb\rc`", Goal::Div),
        template(TemplatePart::NoSubstitution, Some("a\nb\nc"), "a\nb\nc")
    );
    assert_eq!(
        single("`\\\r\n`", Goal::Div),
        template(TemplatePart::NoSubstitution, Some(""), "\\\n")
    );
    assert_eq!(
        single("`\\0`", Goal::Div),
        template(TemplatePart::NoSubstitution, Some("\0"), "\\0")
    );
    // *NotEscapeSequence*: kein Wert, aber ein Rohwert.
    for (text, raw) in [
        ("`\\unicode`", "\\unicode"),
        ("`\\01`", "\\01"),
        ("`\\1`", "\\1"),
        ("`\\8`", "\\8"),
        ("`\\xg`", "\\xg"),
        ("`\\u{110000}`", "\\u{110000}"),
        ("`\\u{12`", "\\u{12"),
    ] {
        assert_eq!(
            single(text, Goal::Div),
            template(TemplatePart::NoSubstitution, None, raw),
            "{text:?}"
        );
    }
    assert_eq!(
        single("`\\x${", Goal::Div),
        template(TemplatePart::Head, None, "\\x")
    );
    for text in ["`abc", "`a\\", "`${"] {
        let result = lex_all(&utf16(text), Goal::Div, false);
        if text == "`${" {
            assert!(result.is_ok());
        } else {
            assert_eq!(
                result.map_err(|error| error.kind),
                Err(ErrorKind::UnterminatedTemplate),
                "{text:?}"
            );
        }
    }
}

#[test]
fn ranges_count_code_units() {
    let tokens = lex_all(&utf16("\u{1D465} = 'a'"), Goal::Div, false).expect("Tokens");
    let ranges: Vec<Range<usize>> = tokens.into_iter().map(|token| token.range).collect();
    assert_eq!(ranges, [0..2, 3..4, 5..8]);
}

#[test]
fn the_end_repeats() {
    let source = utf16("a ");
    let mut lexer = Lexer::new(&source, false);
    assert_eq!(lexer.next_token(Goal::Div).expect("a").kind, name("a"));
    for _ in 0..2 {
        let end = lexer.next_token(Goal::Div).expect("Ende");
        assert_eq!(end.kind, TokenKind::End);
        assert_eq!(end.range, 2..2);
    }
}
