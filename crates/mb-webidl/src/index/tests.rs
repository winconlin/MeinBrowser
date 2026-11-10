use super::*;

#[test]
fn forward_references_partials_inheritance_and_includes_resolve_across_sources() {
    let sources = [
        "partial interface Child { attribute Options options; }; Child includes Extra;",
        "partial interface mixin Extra { undefined run(Handler handler); }; partial dictionary Options { long n = 0; };",
        "interface Child : Base {}; interface Base {}; interface mixin Extra {}; dictionary Options : Parent {}; dictionary Parent {}; callback Handler = undefined (Child value);",
    ];
    let index = build(&sources).expect("linked");
    assert_eq!(index.sources(), 3);
    assert_eq!(index.definitions().len(), 6);
    let child = index.definition(index.find("Child").unwrap()).unwrap();
    assert_eq!(child.parent, index.find("Base"));
    assert_eq!(child.fragments.len(), 2);
    assert!(index.fragment(child.fragments[0]).unwrap().partial);
    assert!(!index.fragment(child.fragments[1]).unwrap().partial);
    assert_eq!(child.mixins, [index.find_mixin("Extra").unwrap()]);
    assert_eq!(index.references().len(), 3);
    assert_eq!(index.includes().len(), 1);
}

fn rejected(sources: &[&str], expected: ErrorKind, token: &str) -> Error {
    let error = build(sources).expect_err("invalid names/links");
    assert_eq!(error.kind, expected, "{sources:?}");
    let at = error.location.as_ref().expect("source location");
    assert_eq!(&sources[at.source][at.span.clone()], token);
    if let Some(previous) = &error.previous {
        assert!(
            sources[previous.source]
                .get(previous.span.clone())
                .is_some()
        );
    }
    error
}

#[test]
fn all_definition_kinds_and_opaque_attributes_keep_their_original_trees() {
    let source = r#"
      [Exposed=Window, Future=(Missing, {Other})] interface A {};
      interface mixin M {};
      callback interface C { const long n = 1; };
      namespace N { undefined f(); };
      dictionary D { required A value; };
      enum E { "é🦀", "off" };
      callback F = Promise<A> (optional D options = {});
      typedef (sequence<A> or E) T;
    "#;
    let index = build(&[source]).unwrap();
    let kinds: Vec<_> = index.definitions().map(|(_, d)| d.kind).collect();
    assert_eq!(
        kinds,
        [
            DefinitionKind::Interface,
            DefinitionKind::Mixin,
            DefinitionKind::CallbackInterface,
            DefinitionKind::Namespace,
            DefinitionKind::Dictionary,
            DefinitionKind::Enum,
            DefinitionKind::Callback,
            DefinitionKind::Typedef
        ]
    );
    assert_eq!(index.references().len(), 5);
    for (id, definition) in index.definitions() {
        let fragment = index.fragment(definition.fragments[0]).unwrap();
        assert_eq!(fragment.definition, id);
        let document = index.document(fragment.location.source).unwrap();
        assert!(
            document
                .text(fragment.body)
                .unwrap()
                .contains(definition.name)
        );
        assert_eq!(
            production(document, fragment.attributes),
            Some("ExtendedAttributeList")
        );
    }
    let first = index
        .fragment(
            index
                .definition(index.find("A").unwrap())
                .unwrap()
                .fragments[0],
        )
        .unwrap();
    assert!(
        index
            .document(0)
            .unwrap()
            .text(first.attributes)
            .unwrap()
            .contains("Future=(Missing, {Other})")
    );
}

#[test]
fn identifiers_are_unescaped_in_names_references_partials_and_relations() {
    let sources = [
        "partial interface _interface { attribute _long value; }; _interface includes _mixin;",
        "interface _interface : _dictionary {}; interface _dictionary {}; interface _long {}; interface mixin _mixin {};",
    ];
    let index = build(&sources).unwrap();
    assert!(index.find("_interface").is_none());
    let id = index.find("interface").unwrap();
    assert_eq!(index.definition(id).unwrap().fragments.len(), 2);
    assert_eq!(index.references()[0].name, "long");
    assert_eq!(index.references()[0].target, index.find("long").unwrap());
    assert_eq!(
        index.definition(id).unwrap().parent,
        index.find("dictionary")
    );
    assert_eq!(
        index.definition(id).unwrap().mixins,
        [index.find_mixin("mixin").unwrap()]
    );
}

#[test]
fn named_definitions_share_one_namespace_and_collisions_report_both_locations() {
    let error = rejected(
        &["interface Example {};", "dictionary _Example {};"],
        ErrorKind::DuplicateDefinition,
        "_Example",
    );
    assert_eq!(error.location.as_ref().unwrap().source, 1);
    assert_eq!(error.previous.as_ref().unwrap().source, 0);
    rejected(
        &["typedef long X; enum X { \"x\" };"],
        ErrorKind::DuplicateDefinition,
        "X",
    );
    rejected(
        &["interface mixin X {}; interface mixin _X {};"],
        ErrorKind::DuplicateDefinition,
        "_X",
    );
}

#[test]
fn partials_require_a_matching_non_partial_definition() {
    for source in [
        "partial interface X {};",
        "partial dictionary X {};",
        "partial namespace X {};",
        "partial interface mixin X {};",
    ] {
        rejected(&[source], ErrorKind::MissingDefinition, "X");
    }
    for source in [
        "partial interface X {}; dictionary X {};",
        "interface X {}; partial dictionary X {};",
        "callback interface X {}; partial interface X {};",
        "namespace X {}; partial interface X {};",
    ] {
        rejected(&[source], ErrorKind::PartialKind, "X");
    }
    let index = build(&[
        "partial namespace X {}; partial dictionary D {}; namespace X {}; dictionary D {};",
    ])
    .unwrap();
    assert_eq!(
        index
            .definition(index.find("X").unwrap())
            .unwrap()
            .fragments
            .len(),
        2
    );
    assert_eq!(
        index
            .definition(index.find("D").unwrap())
            .unwrap()
            .fragments
            .len(),
        2
    );
}

#[test]
fn type_references_are_checked_in_every_grammar_type_position() {
    for source in [
        "interface X { attribute Missing value; };",
        "interface X { const Missing value = 1; };",
        "interface X { Missing op(); };",
        "interface X { undefined op(Missing arg); };",
        "interface X { constructor(Missing arg); };",
        "interface X { iterable<DOMString, Missing>; };",
        "interface X { async_iterable<Missing>(optional Missing arg); };",
        "interface X { readonly maplike<DOMString, Missing>; };",
        "interface X { setlike<Missing>; };",
        "dictionary X { Missing value; };",
        "callback X = Missing ();",
        "callback interface X { undefined op(Missing arg); };",
        "namespace X { readonly attribute Missing value; };",
        "typedef record<DOMString, FrozenArray<ObservableArray<Promise<sequence<Missing>>>>> X;",
        "typedef (long or (DOMString or Missing)) X;",
        "interface mixin X { attribute Missing value; };",
    ] {
        rejected(&[source], ErrorKind::UnknownType, "Missing");
    }
}

#[test]
fn mixins_and_namespaces_are_not_types_but_other_named_definitions_are() {
    rejected(&["namespace N {}; typedef N T;"], ErrorKind::NotAType, "N");
    rejected(
        &["interface mixin M {}; typedef M T;"],
        ErrorKind::NotAType,
        "M",
    );
    let index = build(&["interface I {}; callback interface C {}; dictionary D {}; enum E { \"x\" }; callback F = undefined (); typedef long T; callback All = undefined (I i, C c, D d, E e, F f, T t);"]).unwrap();
    assert_eq!(index.references().len(), 6);
    // Mixins sind keine named definitions; der Mixin-Index bleibt getrennt.
    let index = build(&["interface Shared {}; interface mixin Shared {}; Shared includes Shared;"])
        .unwrap();
    assert_ne!(index.find("Shared"), index.find_mixin("Shared"));
}

#[test]
fn inheritance_targets_have_the_correct_kind_and_no_cycles() {
    for source in [
        "interface X : Missing {};",
        "dictionary X : Missing {};",
        "dictionary D {}; interface X : D {};",
        "interface I {}; dictionary X : I {};",
        "callback interface C {}; interface X : C {};",
    ] {
        let target = if source.contains("Missing") {
            "Missing"
        } else if source.contains(": D") {
            "D"
        } else if source.contains(": I") {
            "I"
        } else {
            "C"
        };
        rejected(&[source], ErrorKind::InvalidParent, target);
    }
    rejected(&["interface X : X {};"], ErrorKind::InheritanceCycle, "X");
    rejected(
        &["interface A : B {}; interface B : C {}; interface C : A {};"],
        ErrorKind::InheritanceCycle,
        "A",
    );
    rejected(
        &["dictionary A : B {}; dictionary B : A {};"],
        ErrorKind::InheritanceCycle,
        "A",
    );
}

#[test]
fn includes_require_regular_interfaces_and_mixins_and_preserve_source_order() {
    for source in [
        "interface mixin M {}; Missing includes M;",
        "dictionary X {}; interface mixin M {}; X includes M;",
        "callback interface X {}; interface mixin M {}; X includes M;",
    ] {
        rejected(
            &[source],
            ErrorKind::InvalidIncludes,
            if source.contains("Missing") {
                "Missing"
            } else {
                "X"
            },
        );
    }
    rejected(
        &["interface X {}; X includes Missing;"],
        ErrorKind::InvalidIncludes,
        "Missing",
    );
    rejected(
        &["interface X {}; interface M {}; X includes M;"],
        ErrorKind::InvalidIncludes,
        "M",
    );
    let index = build(&[
        "X includes B; interface X {}; interface mixin A {}; interface mixin B {}; X includes A;",
    ])
    .unwrap();
    let definition = index.definition(index.find("X").unwrap()).unwrap();
    assert_eq!(
        definition.mixins,
        [
            index.find_mixin("B").unwrap(),
            index.find_mixin("A").unwrap()
        ]
    );
}

#[test]
fn typedef_edges_are_not_recursively_expanded_and_cycles_are_rejected() {
    rejected(&["typedef T T;"], ErrorKind::TypedefAlias, "T");
    rejected(
        &["typedef long A; typedef A B;"],
        ErrorKind::TypedefAlias,
        "A",
    );
    rejected(&["typedef sequence<T> T;"], ErrorKind::TypedefCycle, "T");
    rejected(
        &["typedef sequence<B> A; typedef FrozenArray<A> B;"],
        ErrorKind::TypedefCycle,
        "A",
    );
    let index = build(&["typedef long Number; typedef sequence<Number> Numbers; typedef Numbers? Maybe; callback Use = undefined (Maybe value);"]).unwrap();
    assert_eq!(index.references().len(), 3);
}

#[test]
fn reserved_definition_names_are_rejected_after_unescaping() {
    rejected(
        &["interface _constructor {};"],
        ErrorKind::ReservedIdentifier,
        "_constructor",
    );
    rejected(
        &["typedef long toString;"],
        ErrorKind::ReservedIdentifier,
        "toString",
    );
}

#[test]
fn syntax_errors_keep_source_and_span_without_fabricating_a_tree_node() {
    let error = rejected(
        &["interface X {};", "interface 🦀 {};"],
        ErrorKind::Parser(parser::ErrorKind::Syntax {
            expected: "InterfaceOrMixin",
        }),
        "🦀",
    );
    let at = error.location.unwrap();
    assert_eq!(at.source, 1);
    assert!(at.node.is_none());
}

#[test]
fn whole_input_and_record_limits_are_checked_before_accepting_a_partial_result() {
    let sources = ["interface X {};", "interface Y {};"];
    for (limits, kind) in [
        (
            Limits {
                sources: 1,
                ..Limits::default()
            },
            Limit::Sources,
        ),
        (
            Limits {
                source_bytes: sources[0].len(),
                ..Limits::default()
            },
            Limit::SourceBytes,
        ),
        (
            Limits {
                nodes: 1,
                ..Limits::default()
            },
            Limit::Nodes,
        ),
        (
            Limits {
                records: 1,
                ..Limits::default()
            },
            Limit::Records,
        ),
        (
            Limits {
                records: 3,
                ..Limits::default()
            },
            Limit::Records,
        ),
    ] {
        let error = build_within(&sources, limits, &Budget::unlimited()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Limit(kind));
    }
    let parsed_nodes: usize = sources
        .iter()
        .map(|source| parser::parse(source).unwrap().len())
        .sum();
    let limits = Limits {
        sources: 2,
        source_bytes: sources.iter().map(|s| s.len()).sum(),
        nodes: parsed_nodes,
        records: 4,
        ..Limits::default()
    };
    assert!(build_within(&sources, limits, &Budget::unlimited()).is_ok());
    let error = build_within(
        &sources,
        Limits {
            nodes: parsed_nodes - 1,
            ..limits
        },
        &Budget::unlimited(),
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit(Limit::Nodes));
    let error = build_within(
        &sources,
        Limits {
            parser: parser::Limits {
                nodes: 2,
                ..parser::Limits::default()
            },
            ..Limits::default()
        },
        &Budget::unlimited(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        ErrorKind::Parser(parser::ErrorKind::Limit(parser::Limit::Nodes))
    );
}

#[test]
fn budgets_are_shared_and_exhaustion_in_each_phase_is_a_host_failure() {
    let sources = ["interface X {}; typedef sequence<X> L;"];
    let unlimited = Budget::unlimited();
    build_within(&sources, Limits::default(), &unlimited).unwrap();
    let cost = u64::MAX - unlimited.remaining();
    for steps in [0, 1, 20, cost / 2, cost - 1] {
        let budget = Budget::new(steps);
        let error = build_within(&sources, Limits::default(), &budget).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Exhausted(Exhausted::Steps));
        assert_eq!(budget.exhausted(), Some(Exhausted::Steps));
    }
    let budget = Budget::new(cost);
    build_within(&sources, Limits::default(), &budget).unwrap();
    assert_eq!(budget.remaining(), 0);
    assert!(build_within(&sources, Limits::default(), &budget).is_err());
    let cancel = Cancel::new();
    cancel.cancel();
    let budget = Budget::with_cancel(cost, cancel);
    assert_eq!(
        build_within(&sources, Limits::default(), &budget)
            .unwrap_err()
            .kind,
        ErrorKind::Exhausted(Exhausted::Cancelled)
    );
}

#[test]
fn empty_sources_have_a_valid_empty_index() {
    assert_eq!(build(&[]).unwrap().definitions().len(), 0);
    let index = build(&["", " // trivia\n"]).unwrap();
    assert_eq!(index.sources(), 2);
    assert!(index.references().is_empty());
    assert!(index.includes().is_empty());
}

#[test]
fn deep_type_and_inheritance_graphs_build_and_drop_on_a_small_rust_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let deep = format!(
                "typedef {}long{} T;",
                "sequence<".repeat(2000),
                ">".repeat(2000)
            );
            let mut chain = String::from("interface N0 {}; ");
            for n in 1..2000 {
                chain.push_str(&format!("interface N{n} : N{} {{}}; ", n - 1));
            }
            let index = build(&[&deep, &chain]).unwrap();
            assert_eq!(index.definitions().len(), 2001);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn webref_syntax_trees_are_indexed_without_treating_attribute_names_as_types() {
    let files = testdata::files("webref", "ed/idl", "idl");
    assert_eq!(files.len(), 350);
    let sources: Vec<_> = files
        .iter()
        .map(|file| testdata::read("webref", &format!("ed/idl/{file}")))
        .collect();
    let mut raw = Raw::default();
    let budget = Budget::new(DEFAULT_STEPS);
    let mut work = Work {
        budget: &budget,
        limits: Limits::default(),
        records: 0,
    };
    let mut accepted = 0;
    for (source_id, source) in sources.iter().enumerate() {
        match parser::parse_within(source, parser::Limits::default(), &budget) {
            Ok(document) => {
                collect::document(&document, source_id, &mut raw, &mut work).unwrap();
                accepted += 1;
            }
            Err(error) => {
                assert!(matches!(
                    files[source_id].as_str(),
                    "mediacapture-surface-control.idl" | "webrtc-ice.idl"
                ));
                assert_eq!(
                    error.kind,
                    parser::ErrorKind::Syntax {
                        expected: "PartialInterfaceMembers"
                    }
                );
                assert_eq!(&source[error.span], "constructor");
            }
        }
    }
    assert_eq!(accepted, 348);
    assert_eq!(raw.fragments.len(), 3424);
    assert_eq!(raw.references.len(), 8480);
    assert_eq!(raw.includes.len(), 270);
    for reference in &raw.references {
        assert!(reference.fragment.0 < raw.fragments.len());
        let at = &reference.name.location;
        let text = &sources[at.source][at.span.clone()];
        assert_eq!(text.strip_prefix('_').unwrap_or(text), reference.name.text);
    }
    // Corpus-Verarbeitung ist kein Nachweis vollständiger semantischer Gültigkeit.
}
