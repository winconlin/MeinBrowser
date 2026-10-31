use super::*;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn flags(text: &str) -> Flags {
    Flags::parse(&units(text)).expect("gültige Flags")
}

fn check(pattern: &str, flag_text: &str) -> Result<Pattern> {
    parse(&units(pattern), flags(flag_text))
}

fn error(pattern: &str, flag_text: &str) -> Error {
    match check(pattern, flag_text) {
        Ok(_) => panic!("/{pattern}/{flag_text} sollte ein Fehler sein"),
        Err(error) => error,
    }
}

/// Der Baum als Text: Alternativen mit ` | `, Terme mit Leerzeichen.
fn show(pattern: &Pattern, id: NodeId) -> String {
    let join = |list: List, separator: &str| {
        pattern
            .list(list)
            .iter()
            .map(|&item| show(pattern, item))
            .collect::<Vec<_>>()
            .join(separator)
    };
    match pattern.node(id) {
        Node::Disjunction(list) => join(list, " | "),
        Node::Alternative(list) if list.is_empty() => "ε".to_owned(),
        Node::Alternative(list) => join(list, " "),
        Node::Assertion(assertion) => match assertion {
            Assertion::Start => "^",
            Assertion::End => "$",
            Assertion::WordBoundary => "\\b",
            Assertion::NotWordBoundary => "\\B",
        }
        .to_owned(),
        Node::Lookaround { kind, body } => {
            let prefix = match kind {
                Lookaround::Ahead => "?=",
                Lookaround::NotAhead => "?!",
                Lookaround::Behind => "?<=",
                Lookaround::NotBehind => "?<!",
            };
            format!("({prefix}{})", show(pattern, body))
        }
        Node::Quantified {
            atom,
            min,
            max,
            greedy,
        } => {
            let max = max.map_or_else(String::new, |max| max.to_string());
            let lazy = if greedy { "" } else { "?" };
            format!("{}{{{min},{max}}}{lazy}", show(pattern, atom))
        }
        Node::Character(c) => character(c),
        Node::Dot => ".".to_owned(),
        Node::ClassEscape(escape) => match escape {
            ClassEscape::Digit => "\\d",
            ClassEscape::NotDigit => "\\D",
            ClassEscape::Space => "\\s",
            ClassEscape::NotSpace => "\\S",
            ClassEscape::Word => "\\w",
            ClassEscape::NotWord => "\\W",
        }
        .to_owned(),
        Node::Property { property, negated } => {
            let (name, value) = match property {
                Property::GeneralCategory(value) => ("gc=", value),
                Property::Script(value) => ("sc=", value),
                Property::ScriptExtensions(value) => ("scx=", value),
                Property::Binary(value) | Property::Strings(value) => ("", value),
            };
            let p = if negated { 'P' } else { 'p' };
            format!("\\{p}{{{name}{value}}}")
        }
        Node::Class {
            negated,
            operation,
            items,
        } => {
            let separator = match operation {
                ClassOperation::Union => " ",
                ClassOperation::Intersection => " && ",
                ClassOperation::Subtraction => " -- ",
            };
            let negated = if negated { "^" } else { "" };
            format!("[{negated}{}]", join(items, separator))
        }
        Node::Range { first, last } => format!("{}-{}", character(first), character(last)),
        Node::ClassStrings(list) => format!("\\q{{{}}}", join(list, "|")),
        Node::Backreference(number) => format!("\\{number}"),
        Node::NamedBackreference(name) => {
            format!("\\k<{}>", String::from_utf16_lossy(pattern.name(name)))
        }
        Node::Capture { index, name, body } => {
            let name = name.map_or_else(String::new, |name| {
                format!("<{}>", String::from_utf16_lossy(pattern.name(name)))
            });
            format!("(#{index}{name} {})", show(pattern, body))
        }
        Node::Group { add, remove, body } => {
            let set = |modifiers: Modifiers| {
                [
                    (modifiers.ignore_case, 'i'),
                    (modifiers.multiline, 'm'),
                    (modifiers.dot_all, 's'),
                ]
                .into_iter()
                .filter_map(|(on, flag)| on.then_some(flag))
                .collect::<String>()
            };
            let remove = if remove.is_empty() {
                String::new()
            } else {
                format!("-{}", set(remove))
            };
            format!("(?{}{remove}: {})", set(add), show(pattern, body))
        }
    }
}

fn character(c: u32) -> String {
    match char::from_u32(c) {
        Some(c) if c.is_ascii_graphic() => c.to_string(),
        _ => format!("U+{c:04X}"),
    }
}

fn tree(pattern: &str, flag_text: &str) -> String {
    match check(pattern, flag_text) {
        Ok(parsed) => show(&parsed, parsed.root()),
        Err(error) => panic!("/{pattern}/{flag_text}: {error:?}"),
    }
}

#[test]
fn flag_parsing() {
    let all = Flags::parse(&units("dgimsuy")).expect("gültig");
    assert!(all.has_indices && all.global && all.ignore_case && all.multiline);
    assert!(all.dot_all && all.unicode && all.sticky && !all.unicode_sets);
    assert_eq!(Flags::parse(&[]), Some(Flags::default()));
    for invalid in ["gg", "uv", "G", "x", "\u{0131}", "g\u{FF47}"] {
        assert_eq!(Flags::parse(&units(invalid)), None, "{invalid}");
    }
    let both = Flags {
        unicode: true,
        unicode_sets: true,
        ..Flags::default()
    };
    assert_eq!(
        parse(&units("a"), both).map(|_| ()),
        Err(Error {
            kind: ErrorKind::InvalidFlags,
            position: 0
        })
    );
}

#[test]
fn alternatives_terms_and_quantifiers() {
    assert_eq!(tree("", ""), "ε");
    assert_eq!(tree("a|bc|", ""), "a | b c | ε");
    assert_eq!(
        tree("a*b+?c?d{2}e{2,}f{2,3}?", ""),
        "a{0,} b{1,}? c{0,1} d{2,2} e{2,} f{2,3}?"
    );
    assert_eq!(tree("^a$|.", "u"), "^ a $ | .");
    assert_eq!(tree("a{0001,1}", "u"), "a{1,1}");
    // Zu große Zahlen werden zu u32::MAX, verglichen wird genau.
    assert_eq!(
        tree("a{99999999999,99999999999}", "u"),
        "a{4294967295,4294967295}"
    );
    assert_eq!(
        error("a{99999999999,99999999998}", "u").kind,
        ErrorKind::QuantifierOutOfOrder
    );
    assert_eq!(error("a{2,1}", "").kind, ErrorKind::QuantifierOutOfOrder);
}

#[test]
fn groups() {
    assert_eq!(
        tree("(a)(?<n>b|c)(?:d)(?i-s:e)(?m:f)", ""),
        "(#1 a) (#2<n> b | c) (?: d) (?i-s: e) (?m: f)"
    );
    assert_eq!(
        tree("(?=a)(?!b)(?<=c)(?<!d)", "u"),
        "(?=a) (?!b) (?<=c) (?<!d)"
    );
    assert_eq!(tree("((a)(b))*", ""), "(#1 (#2 a) (#3 b)){0,}");
    let parsed = check("(a)(?<x>b)(?<$_\u{200C}\u{1D400}>c)", "").expect("gültig");
    assert_eq!(parsed.capture_count(), 3);
    assert_eq!(parsed.group_name(1), None);
    let name = parsed.group_name(2).expect("Name");
    assert_eq!(parsed.name(name), units("x"));
    let name = parsed.group_name(3).expect("Name");
    assert_eq!(parsed.name(name), units("$_\u{200C}\u{1D400}"));
    assert_eq!(parsed.group_name(0), None);
    assert_eq!(parsed.group_name(4), None);
}

#[test]
fn group_names_with_escapes_and_surrogates() {
    // Escapes in Namen folgen immer den Regeln mit `u`.
    for flag_text in ["", "u"] {
        let parsed = check(
            "(?<\\u0061\\u{62}\\uD835\\uDC00>.)\\k<ab\u{1D400}>",
            flag_text,
        )
        .expect("gültig");
        let name = parsed.group_name(1).expect("Name");
        assert_eq!(parsed.name(name), units("ab\u{1D400}"));
    }
    for invalid in [
        "(?<>a)",
        "(?<a>",
        "(?<a",
        "(?<1a>a)",
        "(?<a:>a)",
        "(?<a\\>.)",
        "(?<a\\x61>.)",
        "(?<a\\uD801>.)",
        "(?<a\\u{110000}>.)",
        "(?<\u{2764}>a)",
        "(?<a\u{1F712}>.)",
    ] {
        for flag_text in ["", "u"] {
            assert!(check(invalid, flag_text).is_err(), "/{invalid}/{flag_text}");
        }
    }
}

#[test]
fn duplicate_group_names_in_different_alternatives() {
    for valid in [
        "(?<a>x)|(?<a>y)",
        "((?<a>x)|(?<a>y))",
        "(?:(?<a>x)|(?<a>y))\\k<a>",
        "(?<a>x)|(?:(?<a>y)|(?<a>z))",
        "(?:(?<a>x)|(?<b>y))|(?:(?<a>z)|(?<b>w))",
        "(?<a>x)|((?<a>y)(?<b>z))|(?<b>w)",
        "(?:(?<a>x)(?<b>y)|(?<b>z)(?<a>w))",
    ] {
        assert!(check(valid, "").is_ok(), "{valid}");
        assert!(check(valid, "u").is_ok(), "{valid}");
    }
    for invalid in [
        "(?<a>x)(?<a>y)",
        "(?<a>x)(?<b>y)(?<a>z)",
        "(?<a>(?<a>x))",
        "(?:(?<a>x)|(?<a>y))(?<a>z)",
        "(?:(?<a>x)|y)(?:(?<a>z)|w)",
        "(?<a>x)(?:(?<a>y)|(?<a>z))",
        "(?<a>x)|(?<a>y)(?<a>z)",
    ] {
        let error = error(invalid, "");
        assert_eq!(error.kind, ErrorKind::DuplicateGroupName, "{invalid}");
    }
}

#[test]
fn escapes_with_unicode() {
    assert_eq!(
        tree("\\d\\D\\s\\S\\w\\W\\b\\B\\f\\n\\r\\t\\v\\cJ\\0", "u"),
        "\\d \\D \\s \\S \\w \\W \\b \\B U+000C U+000A U+000D U+0009 U+000B U+000A U+0000"
    );
    assert_eq!(
        tree(
            "\\x41\\u0042\\u{43}\\u{0000000044}\\uD83D\\uDE00\\uD83D",
            "u"
        ),
        "A B C D U+1F600 U+D83D"
    );
    assert_eq!(tree("\u{1F600}+", "u"), "U+1F600{1,}");
    assert_eq!(tree("\\^\\$\\\\\\.\\*\\/\\|", "u"), "^ $ \\ . * / |");
    assert_eq!(tree("(a)\\1", "u"), "(#1 a) \\1");
    assert_eq!(tree("\\1(a)", "u"), "\\1 (#1 a)");
    for (invalid, kind) in [
        ("\\M", ErrorKind::InvalidEscape),
        ("\\-", ErrorKind::InvalidEscape),
        ("\\c0", ErrorKind::InvalidEscape),
        ("\\x4", ErrorKind::InvalidEscape),
        ("\\u12", ErrorKind::InvalidEscape),
        ("\\u{110000}", ErrorKind::InvalidEscape),
        ("\\u{1F_639}", ErrorKind::InvalidEscape),
        ("\\u{}", ErrorKind::InvalidEscape),
        ("\\00", ErrorKind::InvalidEscape),
        ("\\1", ErrorKind::InvalidBackreference),
        ("(a)\\2", ErrorKind::InvalidBackreference),
        ("\\k", ErrorKind::InvalidGroupName),
        ("\\k<a>", ErrorKind::UndefinedGroupName),
        ("(?<a>.)\\k<b>", ErrorKind::UndefinedGroupName),
        ("\\", ErrorKind::UnexpectedEnd),
    ] {
        assert_eq!(error(invalid, "u").kind, kind, "/{invalid}/u");
    }
}

#[test]
fn annex_b_without_unicode() {
    // Rückverweise nur bis zur Zahl der Gruppen, sonst oktal oder das Zeichen.
    assert_eq!(tree("\\1(a)", ""), "\\1 (#1 a)");
    assert_eq!(tree("(a)\\2", ""), "(#1 a) U+0002");
    assert_eq!(tree("\\12(a)", ""), "U+000A (#1 a)");
    assert_eq!(
        tree("\\8\\9\\0\\08\\377\\400", ""),
        "8 9 U+0000 U+0000 8 U+00FF U+0020 0"
    );
    // `\c` ohne Buchstaben ist der Rückstrich, in Klassen auch mit Ziffern.
    assert_eq!(tree("\\c1\\cj", ""), "\\ c 1 U+000A");
    assert_eq!(tree("[\\c1\\c_\\c*]", ""), "[U+0011 U+001F \\ c *]");
    // Unvollständige Escapes und Identitäts-Escapes stehen für sich.
    assert_eq!(tree("\\x4\\u12\\M\\p{L}", ""), "x 4 u 1 2 M p { L }");
    assert_eq!(tree("\\u{41}", ""), "u{41,41}");
    // `{`, `}` und `]` als Zeichen, auch mit Quantor; Lookaheads mit Quantor.
    assert_eq!(tree("{}]{*a{,1}", ""), "{ } ] {{0,} a { , 1 }");
    assert_eq!(tree("(?=a)*(?!b){2}", ""), "(?=a){0,} (?!b){2,2}");
    // `\k` ist ohne benannte Gruppen ein `k`.
    assert_eq!(tree("\\k<a>", ""), "k < a >");
    assert_eq!(tree("\\k<a>(?<a>b)", ""), "\\k<a> (#1<a> b)");
    // Bereiche mit Klassen-Escapes stehen einzeln in der Klasse.
    assert_eq!(tree("[\\d-a-\\w]", ""), "[\\d - a - \\w]");
    // Ohne `u` sind Zeichen Codeeinheiten.
    assert_eq!(tree("\u{1F600}", ""), "U+D83D U+DE00");
    for (invalid, kind) in [
        ("{1}", ErrorKind::NothingToRepeat),
        ("a{1}{2}", ErrorKind::NothingToRepeat),
        ("a**", ErrorKind::NothingToRepeat),
        ("^*", ErrorKind::NothingToRepeat),
        ("(?<=a)*", ErrorKind::NothingToRepeat),
        ("\\b+", ErrorKind::NothingToRepeat),
        ("\\k<a>(?<b>c)", ErrorKind::UndefinedGroupName),
        ("\\k(?<a>c)", ErrorKind::InvalidGroupName),
        ("(?<a>c)[\\k]", ErrorKind::InvalidEscape),
        ("[\u{1F600}-\u{1F602}]", ErrorKind::ClassRangeOutOfOrder),
    ] {
        assert_eq!(error(invalid, "").kind, kind, "/{invalid}/");
    }
    assert!(check("[\\k]", "").is_ok());
}

#[test]
fn errors_with_unicode() {
    for (invalid, kind) in [
        ("{", ErrorKind::UnexpectedCharacter),
        ("}", ErrorKind::UnexpectedCharacter),
        ("]", ErrorKind::UnexpectedCharacter),
        ("{1}", ErrorKind::NothingToRepeat),
        ("(?=a)*", ErrorKind::NothingToRepeat),
        ("a)", ErrorKind::UnmatchedParenthesis),
        ("(a", ErrorKind::UnterminatedGroup),
        ("(?", ErrorKind::InvalidGroup),
        ("(?a)", ErrorKind::InvalidGroup),
        ("[a", ErrorKind::UnterminatedClass),
        ("[\\d-a]", ErrorKind::InvalidClassRange),
        ("[z-a]", ErrorKind::ClassRangeOutOfOrder),
        ("(?ii:a)", ErrorKind::InvalidModifiers),
        ("(?-:a)", ErrorKind::InvalidModifiers),
        ("(?i-i:a)", ErrorKind::InvalidModifiers),
        ("(?i-)", ErrorKind::InvalidModifiers),
        ("(?x:a)", ErrorKind::InvalidGroup),
        ("(?\\u0069:a)", ErrorKind::InvalidGroup),
    ] {
        assert_eq!(error(invalid, "u").kind, kind, "/{invalid}/u");
    }
}

#[test]
fn error_positions_count_code_units() {
    assert_eq!(error("ab)", "").position, 2);
    assert_eq!(error("a(b", "").position, 1);
    assert_eq!(error("a[b", "u").position, 1);
    assert_eq!(error("a{2,1}", "").position, 1);
    assert_eq!(error("\u{1F600})", "u").position, 2);
    assert_eq!(error("(?<a>.)(?<a>.)", "").position, 7);
}

#[test]
fn classes() {
    assert_eq!(tree("[a-z\\d-]", ""), "[a-z \\d -]");
    assert_eq!(tree("[^-a\\b\\-]", "u"), "[^- a U+0008 -]");
    assert_eq!(tree("[]|[^]", ""), "[] | [^]");
    assert_eq!(tree("[--a]", ""), "[--a]");
    assert_eq!(tree("[\u{1F600}-\u{1F602}]", "u"), "[U+1F600-U+1F602]");
    assert_eq!(
        tree("[\\p{Lu}\\P{sc=Grek}]", "u"),
        "[\\p{gc=Lu} \\P{sc=Grek}]"
    );
    assert_eq!(error("[a--]", "").kind, ErrorKind::ClassRangeOutOfOrder);
}

#[test]
fn class_sets() {
    assert_eq!(tree("[\\w&&\\d]", "v"), "[\\w && \\d]");
    assert_eq!(
        tree("[[a-z]--[aeiou]--x]", "v"),
        "[[a-z] -- [a e i o u] -- x]"
    );
    assert_eq!(tree("[a-c[d]\\q{ef|g|}]", "v"), "[a-c [d] \\q{e f|g|ε}]");
    assert_eq!(tree("[\\&\\-\\!\\b&]", "v"), "[& - ! U+0008 &]");
    assert_eq!(tree("[^\\q{a|b}]", "v"), "[^\\q{a|b}]");
    assert_eq!(
        tree("\\p{RGI_Emoji}[\\p{Basic_Emoji}--\\q{x}]", "v"),
        "\\p{RGI_Emoji} [\\p{Basic_Emoji} -- \\q{x}]"
    );
    for invalid in [
        "[a-]",
        "[-a]",
        "[a&&b--c]",
        "[ab&&c]",
        "[a-b&&c]",
        "[a&&b-c]",
        "[a&&&b]",
        "[a&&]",
        "[&&a]",
        "[a!!b]",
        "[(]",
        "[a|b]",
        "[\\q{a&&b}]",
        "[\\B]",
        "[a",
        "[[a]",
    ] {
        assert!(check(invalid, "v").is_err(), "/{invalid}/v");
    }
    // Was Zeichenketten enthalten kann, darf nicht verneint werden.
    for invalid in [
        "[^\\q{ab}]",
        "[^\\q{}]",
        "[^\\p{RGI_Emoji}]",
        "[^[\\q{ab}]]",
        "[^\\q{ab}&&\\q{cd}]",
        "[^\\q{ab}--a]",
        "\\P{RGI_Emoji}",
        "[\\P{RGI_Emoji}]",
    ] {
        assert_eq!(
            error(invalid, "v").kind,
            ErrorKind::NegatedStrings,
            "{invalid}"
        );
    }
    for valid in ["[^\\q{ab}&&a]", "[^a--\\q{ab}]", "[^[^\\q{a}]]"] {
        assert!(check(valid, "v").is_ok(), "{valid}");
    }
}

#[test]
fn properties() {
    assert_eq!(
        tree(
            "\\p{L}\\p{Letter}\\p{gc=Lu}\\p{General_Category=punct}\\p{sc=Latin}\\p{Script_Extensions=Qaai}\\P{Alpha}\\p{Any}",
            "u"
        ),
        "\\p{gc=L} \\p{gc=L} \\p{gc=Lu} \\p{gc=P} \\p{sc=Latn} \\p{scx=Zinh} \\P{Alphabetic} \\p{Any}"
    );
    for invalid in [
        "\\p",
        "\\p{",
        "\\p{}",
        "\\p{L",
        "\\p{l}",
        "\\p{ L}",
        "\\p{Latin}",
        "\\p{Script}",
        "\\p{sc}",
        "\\p{gc=Latin}",
        "\\p{sc=Lu}",
        "\\p{ASCII=Y}",
        "\\p{Block=Basic_Latin}",
        "\\p{IsLatin}",
        "\\p{=Lu}",
        "\\p{gc=}",
    ] {
        assert_eq!(
            error(invalid, "u").kind,
            ErrorKind::InvalidProperty,
            "{invalid}"
        );
    }
    assert_eq!(
        error("\\p{RGI_Emoji}", "u").kind,
        ErrorKind::PropertyOfStringsWithoutV
    );
}

#[test]
fn deep_nesting_needs_no_deep_stack() {
    const DEPTH: usize = 100_000;
    let groups = format!("{}a{}", "(".repeat(DEPTH), ")".repeat(DEPTH));
    let classes = format!("{}a{}", "[".repeat(DEPTH), "]".repeat(DEPTH));
    let names: String = (0..DEPTH / 10).map(|_| "(?<a>b)|").collect();
    let worker = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let groups = check(&groups, "").expect("gültig");
            assert_eq!(groups.capture_count(), u32::try_from(DEPTH).unwrap_or(0));
            check(&classes, "v").expect("gültig");
            check(&names, "u").expect("gültig");
        })
        .expect("Faden startet");
    assert!(worker.join().is_ok());
}
