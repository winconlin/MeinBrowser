use super::*;

fn check_tree(document: &Document<'_>) {
    let mut pending = vec![document.root()];
    let mut seen = vec![false; document.len()];
    let mut tokens = Lexer::new(document.source()).filter(|token| !token.kind.is_trivia());
    while let Some(id) = pending.pop() {
        assert!(!seen[id.0 as usize], "tree, not a shared/cyclic graph");
        seen[id.0 as usize] = true;
        let node = document.node(id).expect("node");
        assert!(document.source().get(node.span.clone()).is_some());
        match node.kind {
            Kind::Token(kind) => {
                let token = tokens.next().expect("significant token");
                assert_eq!(kind, token.kind);
                assert_eq!(node.span, token.span);
                assert_eq!(document.text(id), Some(token.text));
                assert_eq!(document.children(id).count(), 0);
            }
            Kind::Production(_) => {
                for child in document.children(id).rev() {
                    assert!(child.0 > id.0, "forward arena edges");
                    let child_node = document.node(child).expect("child");
                    assert!(
                        node.span.start <= child_node.span.start
                            && child_node.span.end <= node.span.end
                    );
                    pending.push(child);
                }
            }
        }
    }
    assert!(seen.iter().all(|seen| *seen));
    assert!(
        tokens.next().is_none(),
        "every significant token exactly once"
    );
    assert_eq!(
        document.node(document.root()).expect("root").span,
        0..document.source().len()
    );
}

#[test]
fn definitions_members_types_and_extended_attributes_have_source_trees() {
    let source = r#"
      [Exposed=Window] interface Example : Base {
        constructor();
        readonly attribute DOMString name;
        undefined run(optional sequence<long> values = []);
      };
      dictionary Options { required DOMString name; long count = 0; };
      interface mixin More { attribute long other; };
      Example includes More;
      partial interface Example { undefined stop(); };
      callback Handler = undefined (long value);
      typedef (DOMString or long) Choice;
      enum State { "on", "off", };
      namespace Helpers { readonly attribute boolean ready; };
    "#;
    let document = parse(source).expect("IDL grammar");
    check_tree(&document);
    let interface = document
        .nodes
        .iter()
        .find(|node| node.kind == Kind::Production("InterfaceRest"))
        .expect("interface");
    assert!(source[interface.span.clone()].starts_with("Example : Base"));
    let dictionary = document
        .nodes
        .iter()
        .find(|node| node.kind == Kind::Production("Dictionary"))
        .expect("dictionary");
    assert!(source[dictionary.span.clone()].starts_with("dictionary Options"));
}

#[test]
fn trivia_empty_files_and_unicode_strings_keep_exact_source() {
    for source in [
        "",
        " // comment\n/* trivia */ ",
        "enum Text { \"é🦀\", \"a\nb\" };",
        "[Named=(A,B)] typedef long _interface;",
    ] {
        let document = parse(source).expect("IDL source");
        assert_eq!(document.source(), source);
        check_tree(&document);
    }
}

#[test]
fn malformed_fragments_and_trailing_input_report_valid_byte_positions() {
    for source in [
        "interface",
        "interface X {",
        "interface X {}",
        "interface X { attribute ; };",
        "typedef sequence<> X;",
        "typedef (long or) X;",
        "dictionary X { required long n = 1; };",
        "enum X { 'bad' };",
        "typedef long X; garbage",
        "/* unfinished",
        "namespace X { constructor(); };",
        "partial interface X { constructor(); };",
    ] {
        let error = parse(source).expect_err(source);
        assert!(
            matches!(error.kind, ErrorKind::Syntax { .. }),
            "{source}: {error:?}"
        );
        assert!(source.get(error.span).is_some());
    }
    let source = "interface X { attribute 🦀 wrong; };";
    let error = parse(source).expect_err("non-ASCII type");
    assert_eq!(&source[error.span], "🦀");
}

#[test]
fn grammar_acceptance_is_separate_from_semantic_validity() {
    // Duplicate names and undefined types belong to later semantic passes.
    let document = parse("typedef Missing X; typedef long X;").expect("syntactically valid");
    check_tree(&document);
}

#[test]
fn source_nodes_and_pending_limits_are_host_failures() {
    let source = "typedef sequence<long> X;";
    for (limits, expected) in [
        (
            Limits {
                source_bytes: 1,
                ..Limits::default()
            },
            Limit::SourceBytes,
        ),
        (
            Limits {
                nodes: 0,
                ..Limits::default()
            },
            Limit::Nodes,
        ),
        (
            Limits {
                nodes: 2,
                ..Limits::default()
            },
            Limit::Nodes,
        ),
        (
            Limits {
                pending: 0,
                ..Limits::default()
            },
            Limit::Pending,
        ),
        (
            Limits {
                pending: 1,
                ..Limits::default()
            },
            Limit::Pending,
        ),
    ] {
        let error =
            parse_within(source, limits, &Budget::new(DEFAULT_STEPS)).expect_err("host limit");
        assert_eq!(error.kind, ErrorKind::Limit(expected));
    }
}

#[test]
fn exact_work_boundary_shared_quotes_and_thread_cancellation_are_observable() {
    let limits = Limits {
        source_bytes: 0,
        nodes: 1,
        pending: 1,
    };
    let budget = Budget::new(2);
    assert!(parse_within("", limits, &budget).is_ok());
    assert_eq!(budget.remaining(), 0);
    assert_eq!(
        parse_within("", limits, &budget)
            .expect_err("shared exhausted budget")
            .kind,
        ErrorKind::Exhausted(Exhausted::Steps)
    );
    assert_eq!(
        parse_within("", limits, &Budget::new(1))
            .expect_err("close also costs a step")
            .kind,
        ErrorKind::Exhausted(Exhausted::Steps)
    );
    let cancel = Cancel::new();
    let signal = cancel.clone();
    std::thread::spawn(move || signal.cancel())
        .join()
        .expect("cancel thread");
    assert_eq!(
        parse_within(
            "typedef long X;",
            Limits::default(),
            &Budget::with_cancel(DEFAULT_STEPS, cancel)
        )
        .expect_err("cancelled")
        .kind,
        ErrorKind::Exhausted(Exhausted::Cancelled)
    );
}

#[test]
fn deep_types_and_their_flat_tree_drop_work_on_a_small_rust_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let source = format!(
                "typedef {}long{} Deep;",
                "sequence<".repeat(2000),
                ">".repeat(2000)
            );
            let document = parse(&source).expect("iterative deep parsing");
            check_tree(&document);
            drop(document);
        })
        .expect("test thread")
        .join()
        .expect("no Rust stack recursion");
}

#[test]
fn literal_terminals_and_generated_tables_fit_the_lexer_and_symbol_encoding() {
    for &text in tables::TERMINALS {
        if matches!(
            text,
            "$" | "integer" | "decimal" | "identifier" | "string" | "other"
        ) {
            continue;
        }
        let mut lexer = Lexer::new(text);
        let token = lexer.next_token();
        assert_eq!(token.kind, TokenKind::Terminal, "{text}");
        assert_eq!(token.text, text);
        assert_eq!(lexer.next_token().kind, TokenKind::End);
    }
    assert_eq!(tables::NAMES.len(), 117);
    assert_eq!(tables::RULES.len(), 312);
    assert_eq!(tables::ENTRIES.len(), 1880);
    assert_eq!(tables::ROWS.len(), tables::NAMES.len() + 1);
    for window in tables::ROWS.windows(2) {
        assert!(window[0] <= window[1]);
        assert!(
            tables::ENTRIES[window[0]..window[1]]
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
    }
}

#[test]
fn webref_snapshot_is_checked_against_the_current_grammar_including_two_negative_cases() {
    let files = testdata::files("webref", "ed/idl", "idl");
    assert_eq!(files.len(), 350);
    let mut accepted = 0;
    let mut rejected = 0;
    for file in files {
        let source = testdata::read("webref", &format!("ed/idl/{file}"));
        match parse(&source) {
            Ok(document) => {
                check_tree(&document);
                accepted += 1;
            }
            Err(error) => {
                assert!(
                    matches!(
                        file.as_str(),
                        "mediacapture-surface-control.idl" | "webrtc-ice.idl"
                    ),
                    "{file}: {error:?}"
                );
                assert_eq!(
                    error.kind,
                    ErrorKind::Syntax {
                        expected: "PartialInterfaceMembers"
                    },
                    "{file}"
                );
                assert_eq!(&source[error.span], "constructor", "{file}");
                rejected += 1;
            }
        }
    }
    assert_eq!(
        (accepted, rejected),
        (348, 2),
        "two partial interfaces declare forbidden constructors"
    );
}
