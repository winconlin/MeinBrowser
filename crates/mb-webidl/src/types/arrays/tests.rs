use super::*;
use crate::{index, members};

fn check(sources: &[&str]) -> Result<Types, Error> {
    let index = index::build(sources).expect("syntax and names");
    let members = members::build(&index).expect("member names");
    super::super::build(&index, &members)
}
fn invalid(source: &str, kind: ErrorKind) {
    let e = check(&[source]).unwrap_err();
    assert_eq!(e.kind, kind, "{source}");
    assert!(source.get(e.location.unwrap().span).is_some());
}

#[test]
fn frozen_arrays_allow_regular_static_readonly_nullable_union_and_inherited_attributes() {
    check(&["typedef [Tag] FrozenArray<long> Values; interface Base { attribute Values writable; readonly attribute FrozenArray<sequence<long>> nested; static attribute Values shared; attribute FrozenArray<long>? maybe; attribute (FrozenArray<long> or DOMString) choice; }; interface Child : Base {}; partial interface Child { readonly attribute Values more; };"]).unwrap();
}
#[test]
fn observable_arrays_allow_regular_attributes_and_nullable_elements() {
    check(&["typedef long? Element; typedef [Tag] ObservableArray<Element> Values; interface Base { attribute Values writable; readonly attribute ObservableArray<object> fixed; }; interface Child : Base {}; partial interface Child { attribute ObservableArray<DOMString> more; };"]).unwrap();
}
#[test]
fn included_and_unused_mixins_are_templates_for_interface_attributes() {
    check(&["interface mixin M { attribute FrozenArray<long> frozen; }; partial interface mixin M { readonly attribute ObservableArray<long> observed; }; interface A {}; interface B {}; A includes M; B includes M; interface mixin Unused { attribute ObservableArray<long> pending; };"]).unwrap();
}
#[test]
fn standalone_aliases_preserve_json_classification_without_creating_a_use() {
    let source = "typedef FrozenArray<long> F; typedef ObservableArray<long> O;";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let types = super::super::build(&index, &members).unwrap();
    assert_eq!(
        types.is_json(types.typedef(index.find("F").unwrap()).unwrap()),
        Some(true)
    );
    assert_eq!(
        types.is_json(types.typedef(index.find("O").unwrap()).unwrap()),
        Some(false)
    );
}
#[test]
fn frozen_arrays_reject_operations_arguments_dictionary_fields_callbacks_and_collections() {
    for body in [
        "interface I { FrozenArray<long> run(); };",
        "interface I { static FrozenArray<long> run(); };",
        "interface I { undefined run(FrozenArray<long> arg); };",
        "interface I { constructor(FrozenArray<long> arg); };",
        "dictionary D { FrozenArray<long> field; };",
        "callback F = FrozenArray<long> ();",
        "callback F = undefined (FrozenArray<long> arg);",
        "callback interface C { FrozenArray<long> run(); };",
        "interface I { setlike<FrozenArray<long>>; };",
        "interface I { maplike<DOMString,FrozenArray<long>>; };",
        "interface I { iterable<DOMString,FrozenArray<long>>; };",
        "interface I { async_iterable<FrozenArray<long>>; };",
    ] {
        invalid(body, ErrorKind::FrozenArrayUse);
    }
}
#[test]
fn namespace_attributes_are_not_ordinary_interface_attributes() {
    for array in ["FrozenArray", "ObservableArray"] {
        let kind = if array == "FrozenArray" {
            ErrorKind::FrozenArrayUse
        } else {
            ErrorKind::ObservableArrayUse
        };
        invalid(
            &format!("namespace N {{ readonly attribute {array}<long> values; }};"),
            kind,
        );
    }
}
#[test]
fn observable_arrays_reject_static_attributes_and_every_non_attribute_signature() {
    for body in [
        "interface I { static attribute ObservableArray<long> values; };",
        "interface I { ObservableArray<long> run(); };",
        "interface I { undefined run(ObservableArray<long> arg); };",
        "dictionary D { ObservableArray<long> field; };",
        "callback F = ObservableArray<long> ();",
        "callback F = undefined (ObservableArray<long> arg);",
        "interface I { async_iterable<long>(optional ObservableArray<long> args); };",
    ] {
        invalid(body, ErrorKind::ObservableArrayUse);
    }
}
#[test]
fn containers_are_separate_use_sites_even_in_unused_typedefs() {
    for array in ["FrozenArray", "ObservableArray"] {
        let kind = if array == "FrozenArray" {
            ErrorKind::FrozenArrayUse
        } else {
            ErrorKind::ObservableArrayUse
        };
        for wrapper in ["sequence", "async_sequence", "Promise", "FrozenArray"] {
            invalid(&format!("typedef {wrapper}<{array}<long>> Nested;"), kind);
        }
        invalid(
            &format!("typedef record<DOMString,{array}<long>> Nested;"),
            kind,
        );
    }
}
#[test]
fn nullable_and_union_wrappers_do_not_hide_argument_uses() {
    for ty in ["FrozenArray<long>?", "(FrozenArray<long> or DOMString)"] {
        invalid(
            &format!("interface I {{ undefined run({ty} value); }};"),
            ErrorKind::FrozenArrayUse,
        );
    }
}
#[test]
fn observable_element_restrictions_follow_aliases_annotations_and_nullability() {
    for ty in [
        "D",
        "D?",
        "sequence<long>",
        "sequence<long>?",
        "record<DOMString,long>",
        "record<DOMString,long>?",
        "ObservableArray<long>",
    ] {
        invalid(
            &format!(
                "dictionary D {{}}; typedef [Tag] {ty} Item; interface I {{ attribute ObservableArray<Item> values; }};"
            ),
            ErrorKind::ObservableArrayElement,
        );
    }
}
#[test]
fn element_restrictions_apply_to_the_parameter_type_not_flattened_union_members() {
    check(&["dictionary D {}; interface I { attribute ObservableArray<(D or DOMString)> values; attribute ObservableArray<(sequence<long> or DOMString)> other; };"]).unwrap();
}
#[test]
fn interfaces_callbacks_promises_and_async_sequences_remain_valid_element_types() {
    for ty in [
        "I",
        "C",
        "F",
        "Promise<long>",
        "async_sequence<long>",
        "any",
        "symbol",
        "undefined",
        "bigint",
    ] {
        check(&[&format!("interface I {{}}; callback interface C {{ undefined call(); }}; callback F = undefined (); interface Host {{ attribute ObservableArray<{ty}> values; }};")]).unwrap();
    }
}
#[test]
fn cross_file_usage_diagnostic_points_to_alias_use_not_declaration() {
    let sources = [
        "typedef FrozenArray<long> Values;",
        "interface I { Values run(); };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::FrozenArrayUse);
    let at = e.location.unwrap();
    assert_eq!(at.source, 1);
    assert_eq!(sources[1][at.span].trim(), "Values");
}
#[test]
fn cross_file_element_diagnostic_points_to_actual_parameter_alias() {
    let sources = [
        "dictionary D {}; typedef sequence<D> Item;",
        "interface I { attribute ObservableArray<Item> values; };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::ObservableArrayElement);
    let at = e.location.unwrap();
    assert_eq!(at.source, 1);
    assert_eq!(&sources[1][at.span], "Item");
}
#[test]
fn earlier_semantic_errors_retain_their_diagnostics() {
    invalid(
        "typedef ObservableArray<long>? Bad;",
        ErrorKind::NullableType,
    );
    invalid(
        "typedef (ObservableArray<long> or DOMString) Bad;",
        ErrorKind::IndistinguishableUnion,
    );
    invalid(
        "dictionary D { FrozenArray<D> self; };",
        ErrorKind::DictionaryCycle,
    );
    invalid(
        "interface I { FrozenArray<any> toJSON(); };",
        ErrorKind::ToJSONType,
    );
}
#[test]
fn exact_budget_and_cancellation_do_not_return_partial_types() {
    let source = "typedef [Tag] long? Item; interface I { attribute ObservableArray<Item> values; static attribute FrozenArray<long> fixed; };";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let unlimited = Budget::unlimited();
    super::super::build_within(&index, &members, Limits::default(), &unlimited).unwrap();
    let cost = u64::MAX - unlimited.remaining();
    let exact = Budget::new(cost);
    super::super::build_within(&index, &members, Limits::default(), &exact).unwrap();
    assert_eq!(exact.remaining(), 0);
    assert_eq!(
        super::super::build_within(&index, &members, Limits::default(), &Budget::new(cost - 1))
            .unwrap_err()
            .kind,
        ErrorKind::Exhausted(Exhausted::Steps)
    );
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(
        super::super::build_within(
            &index,
            &members,
            Limits::default(),
            &Budget::with_cancel(100_000, cancel)
        )
        .unwrap_err()
        .kind,
        ErrorKind::Exhausted(Exhausted::Cancelled)
    );
}
#[test]
fn three_thousand_container_layers_and_two_thousand_shared_uses_fit_small_stack_and_budget() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = format!(
                "typedef FrozenArray<{}long{}> Values; interface I {{ ",
                "sequence<".repeat(3000),
                ">".repeat(3000)
            );
            for n in 0..2000 {
                source.push_str(&format!("readonly attribute Values value{n}; "));
            }
            source.push_str("};");
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            super::super::build_within(
                &index,
                &members,
                Limits::default(),
                &Budget::new(1_000_000),
            )
            .unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
}
