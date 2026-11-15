use super::*;
use crate::index;

#[test]
fn every_member_form_retains_flags_types_signatures_and_source_nodes() {
    let source = r#"
      interface Base { attribute long inherited; };
      interface X : Base {
        [MemberTag] const long answer = 42;
        [Other] readonly attribute DOMString name;
        inherit attribute long inherited;
        static attribute long count;
        static undefined run(optional long count = 1);
        getter DOMString (unsigned long index);
        setter undefined item(unsigned long index, DOMString value);
        deleter boolean remove(DOMString name);
        constructor([ArgumentTag] optional long count = 0);
        iterable<DOMString, long>;
        async_iterable<DOMString>(optional long count);
        readonly maplike<DOMString, long>;
        setlike<DOMString>;
        stringifier;
      };
      interface StringLike { stringifier readonly attribute DOMString text; };
      dictionary D { [FieldTag] required DOMString name; long count = 2; };
      enum E { "é🦀", "", "a\nb", };
      callback F = Promise<long> ([CallbackArg] optional D options = {});
      callback interface C { undefined run(long value); };
      namespace N { readonly attribute long value; undefined run(); };
      interface mixin M { readonly attribute long other; };
    "#;
    let index = index::build(&[source]).unwrap();
    let model = build(&index).unwrap();
    let members = model.members(index.find("X").unwrap()).unwrap();
    assert_eq!(members.len(), 14);
    assert_eq!(members[0].kind, Kind::Constant);
    assert_eq!(members[0].identifier.as_ref().unwrap().name, "answer");
    assert!(members[0].constant.is_some());
    assert!(members[1].readonly);
    assert!(members[2].inherit);
    assert!(members[3].is_static);
    assert!(members[4].is_static);
    assert_eq!(members[5].kind, Kind::Operation(Special::Getter));
    assert!(members[5].identifier.is_none());
    assert_eq!(members[6].kind, Kind::Operation(Special::Setter));
    assert_eq!(members[7].kind, Kind::Operation(Special::Deleter));
    assert_eq!(members[8].kind, Kind::Constructor);
    let args = model
        .arguments(members[8].signature.as_ref().unwrap())
        .unwrap();
    assert_eq!(args.len(), 1);
    assert!(args[0].optional);
    assert!(!args[0].variadic);
    assert!(args[0].default.is_some());
    let document = index.document(0).unwrap();
    assert_eq!(
        document.text(args[0].attributes).unwrap().trim(),
        "[ArgumentTag]"
    );
    assert_eq!(members[9].kind, Kind::Iterable);
    assert!(members[9].types.iter().all(Option::is_some));
    assert_eq!(members[10].kind, Kind::AsyncIterable);
    assert!(members[10].signature.is_some());
    assert_eq!(members[11].kind, Kind::Maplike);
    assert!(members[11].readonly);
    assert_eq!(members[12].kind, Kind::Setlike);
    assert!(!members[12].readonly);
    assert_eq!(members[13].kind, Kind::Stringifier);
    assert!(members[13].stringifier);
    assert!(model.members(index.find("StringLike").unwrap()).unwrap()[0].stringifier);
    let fields = model.members(index.find("D").unwrap()).unwrap();
    assert!(fields[0].required);
    assert!(!fields[1].required);
    assert!(fields[1].default.is_some());
    let values: Vec<_> = model
        .values(index.find("E").unwrap())
        .unwrap()
        .iter()
        .map(|value| value.value)
        .collect();
    assert_eq!(values, ["é🦀", "", "a\\nb"]);
    let callback = model.callback(index.find("F").unwrap()).unwrap();
    assert!(callback.result.is_some());
    assert_eq!(model.arguments(callback).unwrap().len(), 1);
}

fn reject(sources: &[&str], expected: ErrorKind, token: &str) -> Error {
    let index = index::build(sources).expect("linked test fixture");
    let error = build(&index).expect_err("member validation");
    assert_eq!(error.kind, expected, "{sources:?}");
    let at = error.location.as_ref().expect("location");
    assert_eq!(&sources[at.source][at.span.clone()], token);
    assert!(
        index
            .document(at.source)
            .unwrap()
            .node(at.node.unwrap())
            .is_some()
    );
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
fn attribute_constant_and_operation_collisions_are_rejected_in_both_orders() {
    for members in [
        "attribute long value; attribute long value;",
        "attribute long value; static attribute long value;",
        "const long value = 0; const long value = 1;",
        "const long value = 0; undefined value();",
        "undefined value(); const long value = 0;",
        "attribute long value; undefined value();",
        "undefined value(); attribute long value;",
        "static undefined value(); attribute long value;",
    ] {
        reject(
            &[&format!("interface X {{ {members} }};")],
            ErrorKind::DuplicateMember,
            "value",
        );
    }
    reject(
        &["namespace X { readonly attribute long value; undefined value(); };"],
        ErrorKind::DuplicateMember,
        "value",
    );
    reject(
        &["callback interface X { const long value = 0; undefined value(); };"],
        ErrorKind::DuplicateMember,
        "value",
    );
}

#[test]
fn partials_and_included_mixins_contribute_to_the_host_member_set() {
    let error = reject(
        &[
            "interface X { attribute long value; };",
            "partial interface X { readonly attribute long value; };",
        ],
        ErrorKind::DuplicateMember,
        "value",
    );
    assert_eq!(error.location.as_ref().unwrap().source, 1);
    assert_eq!(error.previous.as_ref().unwrap().source, 0);
    reject(
        &[
            "interface X { attribute long value; }; interface mixin M { undefined value(); }; X includes M;",
        ],
        ErrorKind::DuplicateMember,
        "value",
    );
    reject(
        &[
            "interface X {}; interface mixin M { attribute long value; }; interface mixin N { attribute long value; }; X includes M; X includes N;",
        ],
        ErrorKind::DuplicateMember,
        "value",
    );
    reject(
        &[
            "interface mixin M { attribute long value; }; partial interface mixin M { attribute long _value; };",
        ],
        ErrorKind::DuplicateMember,
        "_value",
    );
}

#[test]
fn same_fragment_overloads_and_distinct_static_and_regular_operations_are_preserved() {
    let source = "interface X { constructor(); constructor(long value); undefined run(); undefined run(long value); static undefined run(); }; partial interface X { static undefined other(); }; interface Base { attribute long value; }; interface Child : Base { attribute long value; };";
    let index = index::build(&[source]).unwrap();
    let model = build(&index).unwrap();
    assert_eq!(model.members(index.find("X").unwrap()).unwrap().len(), 6);
    // Der Index ist noch keine Prüfung der Typunterscheidbarkeit von Overloads.
    let index =
        index::build(&["interface X { undefined run(long value); undefined run(long other); };"])
            .unwrap();
    assert!(build(&index).is_ok());
}

#[test]
fn operations_cannot_be_overloaded_across_interface_or_mixin_fragments() {
    for source in [
        "interface X { undefined run(); }; partial interface X { undefined run(long value); };",
        "interface mixin M { undefined run(); }; partial interface mixin M { undefined run(long value); };",
        "interface X { undefined run(); }; interface mixin M { undefined run(long value); }; X includes M;",
        "interface X {}; interface mixin M { undefined run(); }; interface mixin N { undefined run(long value); }; X includes M; X includes N;",
    ] {
        reject(&[source], ErrorKind::CrossFragmentOverload, "run");
    }
    let index = index::build(&[
        "interface X { undefined run(); }; partial interface X { static undefined run(); };",
    ])
    .unwrap();
    assert!(build(&index).is_ok());
}

#[test]
fn dictionary_fields_cannot_repeat_across_partials_or_ancestors() {
    reject(
        &["dictionary X { long value; long value; };"],
        ErrorKind::DuplicateDictionaryMember,
        "value",
    );
    reject(
        &["partial dictionary X { long _value; }; dictionary X { long value; };"],
        ErrorKind::DuplicateDictionaryMember,
        "value",
    );
    let error = reject(
        &[
            "dictionary Child : Parent { long value; };",
            "dictionary Parent { long value; };",
        ],
        ErrorKind::DuplicateDictionaryMember,
        "value",
    );
    assert_eq!(error.location.as_ref().unwrap().source, 0);
    assert_eq!(error.previous.as_ref().unwrap().source, 1);
    reject(
        &[
            "dictionary A { long value; }; dictionary B : A {}; dictionary C : B { required long value; };",
        ],
        ErrorKind::DuplicateDictionaryMember,
        "value",
    );
}

#[test]
fn dictionary_scope_is_restored_between_siblings_and_independent_roots() {
    let index = index::build(&["dictionary Root { long root; }; dictionary Left : Root { long value; }; dictionary Right : Root { long value; }; dictionary Other { long value; }; dictionary Leaf : Left { long leaf; };"]).unwrap();
    let model = build(&index).unwrap();
    assert_eq!(model.len(), 5);
}

#[test]
fn argument_names_are_unique_after_unescaping_in_each_callable_kind() {
    for source in [
        "interface X { undefined run(long value, DOMString _value); };",
        "interface X { constructor(long value, long value); };",
        "interface X { async_iterable<long>(long value, long value); };",
        "callback X = undefined (long value, long value);",
        "callback interface X { undefined run(long value, long value); };",
        "namespace X { undefined run(long value, long value); };",
    ] {
        reject(
            &[source],
            ErrorKind::DuplicateArgument,
            if source.contains("_value") {
                "_value"
            } else {
                "value"
            },
        );
    }
    let index = index::build(&["interface X { undefined run(long constructor, long toString, long required, long _interface); undefined other(long constructor); };"]).unwrap();
    let model = build(&index).unwrap();
    let args = model
        .arguments(
            model.members(index.find("X").unwrap()).unwrap()[0]
                .signature
                .as_ref()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        args.iter()
            .map(|arg| arg.identifier.name)
            .collect::<Vec<_>>(),
        ["constructor", "toString", "required", "interface"]
    );
}

#[test]
fn variadic_arguments_are_optional_and_must_be_last() {
    reject(
        &["interface X { undefined run(long... values, long other); };"],
        ErrorKind::VariadicNotLast,
        "values",
    );
    reject(
        &["callback X = undefined (long... values, optional long other);"],
        ErrorKind::VariadicNotLast,
        "values",
    );
    let index = index::build(&["callback X = undefined (long value, DOMString... rest);"]).unwrap();
    let model = build(&index).unwrap();
    let args = model
        .arguments(model.callback(index.find("X").unwrap()).unwrap())
        .unwrap();
    assert!(!args[0].optional);
    assert!(args[1].optional);
    assert!(args[1].variadic);
    assert!(args[1].default.is_none());
}

#[test]
fn unnamed_operations_need_a_special_keyword() {
    let source = "interface X { undefined (); };";
    let index = index::build(&[source]).unwrap();
    assert_eq!(
        build(&index).unwrap_err().kind,
        ErrorKind::MissingOperationName
    );
    let source = "interface X { static undefined (); };";
    let index = index::build(&[source]).unwrap();
    assert_eq!(
        build(&index).unwrap_err().kind,
        ErrorKind::MissingOperationName
    );
    let index = index::build(&["interface X { getter DOMString (unsigned long index); setter undefined (unsigned long index, DOMString value); deleter boolean (DOMString name); stringifier; };"]).unwrap();
    assert_eq!(build(&index).unwrap().len(), 4);
}

#[test]
fn member_identifier_restrictions_are_applied_after_unescaping() {
    for (member, token) in [
        ("attribute long toString;", "toString"),
        ("undefined _constructor();", "_constructor"),
        ("const long length = 0;", "length"),
        ("const long name = 0;", "name"),
        ("const long prototype = 0;", "prototype"),
        ("static attribute long prototype;", "prototype"),
        ("static undefined prototype();", "prototype"),
    ] {
        reject(
            &[&format!("interface X {{ {member} }};")],
            ErrorKind::ReservedIdentifier,
            token,
        );
    }
    reject(
        &["dictionary X { long toString; };"],
        ErrorKind::ReservedIdentifier,
        "toString",
    );
    let index = index::build(&["interface X { constructor(); attribute long _required; attribute long prototype; undefined _interface(); };"]).unwrap();
    assert!(build(&index).is_ok());
}

#[test]
fn enum_values_compare_exact_contents_without_case_or_escape_normalization() {
    reject(
        &["enum E { \"x\", \"x\" };"],
        ErrorKind::DuplicateEnumValue,
        "\"x\"",
    );
    reject(
        &["enum E { \"é🦀\", \"é🦀\" };"],
        ErrorKind::DuplicateEnumValue,
        "\"é🦀\"",
    );
    reject(
        &["enum E { \"\", \"\" };"],
        ErrorKind::DuplicateEnumValue,
        "\"\"",
    );
    let source = "enum E { \"x\", \"X\", \"é\", \"e\u{301}\", \"a\\nb\", \"a\nb\" };";
    let index = index::build(&[source]).unwrap();
    assert_eq!(build(&index).unwrap().value_count(), 6);
}

#[test]
fn structured_data_remains_borrowed_and_preserves_attribute_and_default_nodes() {
    let source = "[Definition=(Ignore,Names)] interface X { [Member=Missing] undefined run([Argument=(No,Types)] optional [TypeTag] long value = 12); };";
    let index = index::build(&[source]).unwrap();
    let model = build(&index).unwrap();
    let member = &model.members(index.find("X").unwrap()).unwrap()[0];
    let arg = &model.arguments(member.signature.as_ref().unwrap()).unwrap()[0];
    let document = index.document(0).unwrap();
    assert_eq!(
        document.text(member.attributes).unwrap().trim(),
        "[Member=Missing]"
    );
    assert_eq!(
        document.text(arg.attributes).unwrap().trim(),
        "[Argument=(No,Types)]"
    );
    assert_eq!(document.text(arg.ty).unwrap().trim(), "[TypeTag] long");
    assert_eq!(document.text(arg.default.unwrap()).unwrap().trim(), "12");
}

#[test]
fn record_and_effective_member_visit_limits_are_explicit_host_failures() {
    let index =
        index::build(&["interface X { undefined run(long value); }; enum E { \"on\" };"]).unwrap();
    // Zwei Definitionen, ein Member, ein Argument und ein Enumwert.
    let limits = Limits {
        records: 5,
        visits: 100,
    };
    assert!(build_within(&index, limits, &Budget::unlimited()).is_ok());
    for maximum in [0, 1, 4] {
        let e = build_within(
            &index,
            Limits {
                records: maximum,
                ..limits
            },
            &Budget::unlimited(),
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Limit(Limit::Records));
    }
    assert_eq!(
        build_within(
            &index,
            Limits {
                visits: 0,
                ..limits
            },
            &Budget::unlimited()
        )
        .unwrap_err()
        .kind,
        ErrorKind::Limit(Limit::Visits)
    );
    let source = format!(
        "interface mixin M {{ {} }}; {}",
        (0..30)
            .map(|n| format!("attribute long value{n};"))
            .collect::<String>(),
        (0..30)
            .map(|n| format!("interface X{n} {{}}; X{n} includes M;"))
            .collect::<String>()
    );
    let index = index::build(&[&source]).unwrap();
    assert_eq!(
        build_within(
            &index,
            Limits {
                visits: 200,
                ..Limits::default()
            },
            &Budget::unlimited()
        )
        .unwrap_err()
        .kind,
        ErrorKind::Limit(Limit::Visits)
    );
}

#[test]
fn shared_work_budget_and_cancellation_cover_collection_and_effective_member_checks() {
    let source = "interface X {}; interface mixin M { attribute long value; }; X includes M; dictionary D { long n; };";
    let budget = Budget::unlimited();
    let index = index::build_within(&[source], index::Limits::default(), &budget).unwrap();
    let remaining_after_index = budget.remaining();
    build_within(&index, Limits::default(), &budget).unwrap();
    let cost = remaining_after_index - budget.remaining();
    for steps in [0, 1, cost / 2, cost - 1] {
        let budget = Budget::new(steps);
        let e = build_within(&index, Limits::default(), &budget).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Exhausted(Exhausted::Steps));
        assert_eq!(budget.exhausted(), Some(Exhausted::Steps));
    }
    let exact = Budget::new(cost);
    build_within(&index, Limits::default(), &exact).unwrap();
    assert_eq!(exact.remaining(), 0);
    let cancel = Cancel::new();
    cancel.cancel();
    let budget = Budget::with_cancel(u64::MAX, cancel);
    assert_eq!(
        build_within(&index, Limits::default(), &budget)
            .unwrap_err()
            .kind,
        ErrorKind::Exhausted(Exhausted::Cancelled)
    );
    let parsed_cost = u64::MAX - remaining_after_index;
    let common = Budget::new(parsed_cost + cost - 1);
    let index = index::build_within(&[source], index::Limits::default(), &common).unwrap();
    assert_eq!(
        build_within(&index, Limits::default(), &common)
            .unwrap_err()
            .kind,
        ErrorKind::Exhausted(Exhausted::Steps)
    );
}

#[test]
fn dictionary_inheritance_builds_and_drops_iteratively_without_copying_ancestors() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::from("dictionary D0 { long n0; }; ");
            for n in 1..2000 {
                source.push_str(&format!("dictionary D{n} : D{} {{ long n{n}; }}; ", n - 1));
            }
            let index = index::build(&[&source]).unwrap();
            let model = build_within(
                &index,
                Limits {
                    records: 4000,
                    visits: 12_000,
                },
                &Budget::new(150_000),
            )
            .unwrap();
            assert_eq!(model.len(), 2000);
            drop(model);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn empty_models_and_all_empty_definition_forms_are_supported() {
    let index = index::build(&[]).unwrap();
    assert!(build(&index).unwrap().is_empty());
    let index = index::build(&["interface X {}; interface mixin M {}; dictionary D {}; namespace N {}; callback F = undefined (); callback interface C {}; typedef long T;"]).unwrap();
    let model = build(&index).unwrap();
    assert!(model.is_empty());
    assert_eq!(model.argument_count(), 0);
    assert_eq!(model.value_count(), 0);
}
