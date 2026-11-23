use super::*;
use crate::{index, members};
fn check(sources: &[&str]) -> Result<Types, Error> {
    let index = index::build(sources).expect("syntax and names");
    let members = members::build(&index).expect("member names");
    super::super::build(&index, &members)
}
#[test]
fn object_tojson_is_valid() {
    check(&["interface I { object toJSON(); };"]).unwrap();
}

fn invalid(source: &str, kind: ErrorKind) {
    let e = check(&[source]).unwrap_err();
    assert_eq!(e.kind, kind, "{source}");
    assert!(source.get(e.location.unwrap().span).is_some());
}
fn flag(prefix: &str, ty: &str) -> bool {
    let probe = if ty.starts_with("FrozenArray<") || ty.starts_with("ObservableArray<") {
        format!("readonly attribute {ty} probe;")
    } else {
        format!("{ty} probe();")
    };
    let source = format!("{prefix} interface JSONProbe {{ {probe} }};");
    let index = index::build(&[&source]).unwrap();
    let members = members::build(&index).unwrap();
    let types = super::super::build(&index, &members).unwrap();
    let member = &members.members(index.find("JSONProbe").unwrap()).unwrap()[0];
    let ty = types
        .at(member.location.source, member.types[0].unwrap())
        .unwrap();
    types.is_json(ty).unwrap()
}

#[test]
fn all_numeric_boolean_string_and_enum_types_are_json_but_bigint_is_not() {
    for ty in [
        "byte",
        "octet",
        "short",
        "unsigned short",
        "long",
        "unsigned long",
        "long long",
        "unsigned long long",
        "float",
        "unrestricted float",
        "double",
        "unrestricted double",
        "boolean",
        "DOMString",
        "ByteString",
        "USVString",
        "State",
        "object",
    ] {
        assert!(flag("enum State {\"on\"};", ty), "{ty}");
    }
    for ty in [
        "bigint",
        "any",
        "undefined",
        "symbol",
        "ArrayBuffer",
        "SharedArrayBuffer",
        "DataView",
        "Uint8Array",
        "BigInt64Array",
        "Float64Array",
    ] {
        assert!(!flag("", ty), "{ty}");
    }
}

#[test]
fn json_closure_follows_nullable_annotations_typedefs_unions_sequences_and_records() {
    let prefix = "typedef [Opaque] long Count; enum State {\"on\"};";
    for ty in [
        "Count",
        "Count?",
        "(Count or DOMString)",
        "(boolean or (long or State)?)",
        "sequence<Count?>",
        "FrozenArray<Count>",
        "record<USVString, sequence<Count>>",
    ] {
        assert!(flag(prefix, ty), "{ty}");
    }
    for ty in [
        "bigint?",
        "(long or bigint)",
        "sequence<any>",
        "FrozenArray<symbol>",
        "record<DOMString, bigint>",
        "Promise<Count>",
        "async_sequence<Count>",
        "ObservableArray<Count>",
    ] {
        assert!(!flag(prefix, ty), "{ty}");
    }
}

#[test]
fn dictionary_json_types_include_all_own_partial_and_inherited_fields() {
    assert!(flag("dictionary D {};", "D"));
    assert!(flag(
        "dictionary Base { sequence<long> values; }; dictionary D : Base {}; partial dictionary D { record<DOMString,boolean> states; };",
        "D"
    ));
    assert!(!flag(
        "dictionary Base { any opaque; }; dictionary D : Base {};",
        "D"
    ));
    assert!(!flag(
        "dictionary D {}; partial dictionary D { bigint count; };",
        "D"
    ));
    assert!(!flag(
        "dictionary Bad { symbol value; }; dictionary D { sequence<Bad> values; };",
        "D"
    ));
}

#[test]
fn non_json_containers_stop_cycles_that_are_not_dictionary_inclusion_edges() {
    assert!(!flag("dictionary D { Promise<D> future; };", "D"));
    assert!(!flag("dictionary D { async_sequence<D> future; };", "D"));
    invalid(
        "dictionary D { sequence<D> self; }; interface I { D toJSON(); };",
        ErrorKind::DictionaryCycle,
    );
}

#[test]
fn ordinary_interfaces_need_a_regular_tojson_operation() {
    assert!(!flag("interface I {};", "I"));
    assert!(!flag("interface I { static object toJSON(); };", "I"));
    assert!(!flag(
        "interface I { readonly attribute DOMString toJSON; };",
        "I"
    ));
    assert!(flag("interface I { object toJSON(); };", "I"));
    assert!(flag(
        "interface I {}; partial interface I { object _toJSON(); };",
        "I"
    ));
    assert!(flag(
        "interface Base { object toJSON(); }; interface I : Base {};",
        "I"
    ));
    assert!(flag(
        "interface I {}; interface mixin M { object toJSON(); }; I includes M;",
        "I"
    ));
}

#[test]
fn static_and_inherited_properties_do_not_erase_a_declared_regular_operation() {
    assert!(flag(
        "interface Base { object toJSON(); }; interface I : Base { static object toJSON(long ignored); };",
        "I"
    ));
    assert!(flag(
        "interface Base { object toJSON(); }; interface I : Base { readonly attribute DOMString toJSON; };",
        "I"
    ));
    check(&["interface I { static bigint toJSON(long ignored); };"]).unwrap();
}

#[test]
fn self_and_mutually_returned_interfaces_are_nominal_json_leaves() {
    check(&["interface I { I toJSON(); };"]).unwrap();
    check(&["interface A { B toJSON(); }; interface B { A toJSON(); };"]).unwrap();
    assert!(flag(
        "interface I { I toJSON(); }; dictionary D { I value; };",
        "D"
    ));
    invalid(
        "interface Plain {}; interface I { Plain toJSON(); };",
        ErrorKind::ToJSONType,
    );
}

#[test]
fn callbacks_and_callback_interfaces_do_not_become_json_interface_types() {
    assert!(!flag("callback F = undefined ();", "F"));
    assert!(!flag("callback interface C { object toJSON(); };", "C"));
    invalid(
        "callback interface C { object toJSON(); }; interface I { C toJSON(); };",
        ErrorKind::ToJSONType,
    );
}

#[test]
fn tojson_requires_zero_declared_arguments_even_if_omittable() {
    for args in [
        "long value",
        "optional long value",
        "optional long value=0",
        "long... values",
    ] {
        invalid(
            &format!("interface I {{ object toJSON({args}); }};"),
            ErrorKind::ToJSONArguments,
        );
    }
    invalid(
        "interface I { getter long toJSON(DOMString key); };",
        ErrorKind::ToJSONArguments,
    );
}

#[test]
fn regular_tojson_rules_cover_mixins_namespaces_and_callback_interfaces() {
    check(&["namespace N { object toJSON(); }; callback interface C { object toJSON(); }; interface mixin M { object toJSON(); };"]).unwrap();
    for source in [
        "namespace N { object toJSON(long value); };",
        "callback interface C { object toJSON(long value); };",
        "interface mixin M { object toJSON(long value); };",
    ] {
        invalid(source, ErrorKind::ToJSONArguments);
    }
    for source in [
        "namespace N { bigint toJSON(); };",
        "callback interface C { bigint toJSON(); };",
        "interface mixin M { bigint toJSON(); };",
    ] {
        invalid(source, ErrorKind::ToJSONType);
    }
}

#[test]
fn invalid_results_are_rejected_after_alias_resolution() {
    for ty in [
        "any",
        "undefined",
        "bigint",
        "symbol",
        "ArrayBuffer",
        "Promise<long>",
        "sequence<any>",
        "record<DOMString,bigint>",
        "(long or bigint)",
    ] {
        invalid(
            &format!("interface I {{ {ty} toJSON(); }};"),
            ErrorKind::ToJSONType,
        );
    }
    invalid(
        "typedef Promise<long> Future; interface I { Future toJSON(); };",
        ErrorKind::ToJSONType,
    );
    check(&["interface I { unrestricted double toJSON(); };"]).unwrap();
}

#[test]
fn diagnostics_keep_the_actual_method_use_across_files() {
    let sources = [
        "interface I { Data toJSON(); };",
        "dictionary Data { any value; };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::ToJSONType);
    let at = e.location.unwrap();
    assert_eq!(at.source, 0);
    assert_eq!(sources[0][at.span].trim_end(), "Data");
    let sources = [
        "interface I {};",
        "partial interface I { object toJSON(optional long value); };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::ToJSONArguments);
    let at = e.location.unwrap();
    assert_eq!(at.source, 1);
    assert_eq!(&sources[1][at.span], "value");
}

#[test]
fn exact_budget_and_external_cancellation_do_not_return_partial_json_facts() {
    let source = "dictionary Base { long count; }; dictionary D : Base { sequence<DOMString> names; }; interface I { D toJSON(); };";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let unlimited = Budget::unlimited();
    super::super::build_within(&index, &members, Limits::default(), &unlimited).unwrap();
    let cost = u64::MAX - unlimited.remaining();
    let exact = Budget::new(cost);
    let graph = super::super::build_within(&index, &members, Limits::default(), &exact).unwrap();
    assert_eq!(exact.remaining(), 0);
    assert!(graph.iter().all(|(id, _)| graph.is_json(id).is_some()));
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
fn two_thousand_shared_dictionary_layers_do_not_expand_exponentially() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            for valid in [true, false] {
                let mut source = format!(
                    "dictionary D0 {{ {} value; }}; ",
                    if valid { "long" } else { "any" }
                );
                for n in 1..2000 {
                    source.push_str(&format!(
                        "dictionary D{n} {{ D{} left; D{} right; }}; ",
                        n - 1,
                        n - 1
                    ));
                }
                source.push_str("interface I { D1999 toJSON(); };");
                let index = index::build(&[&source]).unwrap();
                let members = members::build(&index).unwrap();
                let result = super::super::build_within(
                    &index,
                    &members,
                    Limits::default(),
                    &Budget::new(500_000),
                );
                assert_eq!(result.is_ok(), valid);
                if let Err(e) = result {
                    assert_eq!(e.kind, ErrorKind::ToJSONType);
                }
                drop(members);
                drop(index);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn long_forward_dictionary_inheritance_fits_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::new();
            for n in (1..2000).rev() {
                source.push_str(&format!("dictionary D{n} : D{} {{}}; ", n - 1));
            }
            source.push_str("dictionary D0 { long value; }; interface I { D1999 toJSON(); };");
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            let graph = super::super::build_within(
                &index,
                &members,
                Limits::default(),
                &Budget::new(500_000),
            )
            .unwrap();
            drop(graph);
            drop(members);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn interface_presence_propagates_once_along_deep_inheritance() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::from("interface I0 { object toJSON(); }; ");
            for n in 1..2000 {
                source.push_str(&format!("interface I{n} : I{} {{}}; ", n - 1));
            }
            source.push_str("interface Serializer { I1999 toJSON(); };");
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            let graph = super::super::build_within(
                &index,
                &members,
                Limits::default(),
                &Budget::new(250_000),
            )
            .unwrap();
            drop(graph);
            drop(members);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn sibling_interfaces_and_mixin_hosts_have_independent_presence_flags() {
    assert!(flag(
        "interface Base {}; interface A : Base { object toJSON(); }; interface B : Base {};",
        "A"
    ));
    assert!(!flag(
        "interface Base {}; interface A : Base { object toJSON(); }; interface B : Base {};",
        "B"
    ));
    assert!(!flag(
        "interface mixin M { object toJSON(); }; interface A {}; interface B {}; A includes M;",
        "B"
    ));
}
