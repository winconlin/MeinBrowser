use super::*;
use crate::{index, members};
fn check(sources: &[&str]) -> Result<Types, Error> {
    let i = index::build(sources).expect("syntax and names");
    let m = members::build(&i).expect("members");
    super::super::build(&i, &m)
}
fn invalid(source: &str, kind: ErrorKind) {
    let e = check(&[source]).unwrap_err();
    assert_eq!(e.kind, kind, "{source}");
    assert!(source.get(e.location.unwrap().span).is_some());
}
#[test]
fn same_object_accepts_readonly_regular_static_namespace_and_mixin_interface_or_object_attributes()
{
    check(&["interface Value {}; typedef [Tag] Value Alias; interface I { [SameObject] readonly attribute Alias first; [SameObject] static readonly attribute object second; }; partial interface I { [SameObject] readonly attribute object third; }; interface mixin M { [SameObject] readonly attribute Value fourth; }; I includes M; namespace N { [SameObject] readonly attribute object value; };"]).unwrap();
}
#[test]
fn same_object_rejects_nullable_union_callback_buffer_and_frozen_array_types() {
    for ty in [
        "Value?",
        "object?",
        "(Value or DOMString)",
        "long",
        "DOMString",
        "C",
        "F",
        "ArrayBuffer",
        "FrozenArray<long>",
    ] {
        invalid(
            &format!(
                "interface Value {{}}; callback interface C {{ undefined run(); }}; callback F=undefined (); interface I {{ [SameObject] readonly attribute {ty} value; }};"
            ),
            ErrorKind::MemberAttributeType,
        );
    }
}
#[test]
fn same_object_requires_readonly_and_may_coexist_with_each_setter_flag() {
    invalid(
        "interface I { [SameObject] attribute object value; };",
        ErrorKind::MemberAttributeReadonly,
    );
    check(&["interface I { [SameObject,Replaceable] readonly attribute object replaced; [SameObject,LegacyLenientSetter] readonly attribute object ignored; };"]).unwrap();
}
#[test]
fn new_object_accepts_regular_static_namespace_and_callback_operations_returning_interface_or_promise()
 {
    check(&["interface Value {}; typedef Promise<long> Result; interface I { [NewObject] Value create(); [NewObject] static Result later(); }; interface mixin M { [NewObject] Promise<undefined> make(); }; I includes M; namespace N { [NewObject] Value create(); }; callback interface C { [NewObject] Value create(); };"]).unwrap();
}
#[test]
fn named_special_getter_is_also_regular_but_anonymous_getter_is_not() {
    check(&["interface Value {}; interface I { [NewObject] getter Value item(DOMString key); };"])
        .unwrap();
    invalid(
        "interface Value {}; interface I { [NewObject] getter Value (DOMString key); };",
        ErrorKind::MemberAttributePlacement,
    );
}
#[test]
fn new_object_rejects_object_nullable_and_callback_results() {
    for ty in [
        "object",
        "Value?",
        "long",
        "undefined",
        "C",
        "F",
        "ArrayBuffer",
    ] {
        invalid(
            &format!(
                "interface Value {{}}; callback interface C {{ undefined run(); }}; callback F=undefined (); interface I {{ [NewObject] {ty} create(); }};"
            ),
            ErrorKind::MemberAttributeType,
        );
    }
}
#[test]
fn new_object_does_not_apply_to_attributes_or_constructors() {
    invalid(
        "interface Value {}; interface I { [NewObject] readonly attribute Value value; };",
        ErrorKind::MemberAttributePlacement,
    );
    invalid(
        "interface I { [NewObject] constructor(); };",
        ErrorKind::MemberAttributePlacement,
    );
}
#[test]
fn replaceable_and_lenient_setter_accept_regular_readonly_attributes_and_mixins() {
    for name in ["Replaceable", "LegacyLenientSetter"] {
        check(&[&format!("interface I {{ [{name}] readonly attribute long value; }}; interface mixin M {{ [{name}] readonly attribute object other; }}; I includes M;")]).unwrap();
        invalid(
            &format!("interface I {{ [{name}] attribute long value; }};"),
            ErrorKind::MemberAttributeReadonly,
        );
    }
}
#[test]
fn setter_flags_reject_static_and_namespace_attributes() {
    for name in ["Replaceable", "LegacyLenientSetter"] {
        invalid(
            &format!("interface I {{ [{name}] static readonly attribute object value; }};"),
            ErrorKind::MemberAttributePlacement,
        );
        invalid(
            &format!("namespace N {{ [{name}] readonly attribute object value; }};"),
            ErrorKind::MemberAttributePlacement,
        );
    }
}
#[test]
fn setter_flags_and_put_forwards_are_mutually_exclusive() {
    for flags in [
        "Replaceable,LegacyLenientSetter",
        "LegacyLenientSetter,Replaceable",
        "Replaceable,PutForwards=value",
        "PutForwards=value,LegacyLenientSetter",
    ] {
        invalid(
            &format!("interface I {{ [{flags}] readonly attribute object value; }};"),
            ErrorKind::MemberAttributeConflict,
        );
    }
}
#[test]
fn all_four_restricted_flags_are_forbidden_on_promise_attributes_after_alias_resolution() {
    for name in [
        "SameObject",
        "Replaceable",
        "LegacyLenientSetter",
        "PutForwards=value",
    ] {
        invalid(
            &format!(
                "typedef Promise<long> Result; interface I {{ [{name}] readonly attribute Result value; }};"
            ),
            ErrorKind::MemberAttributePromise,
        );
    }
    check(&[
        "interface I { [Replaceable] readonly attribute FrozenArray<Promise<long>> values; };",
    ])
    .unwrap();
}
#[test]
fn promise_returning_new_object_operations_do_not_trigger_attribute_only_bans() {
    check(&["interface I { [NewObject] Promise<long> run(); }; namespace N { [NewObject] Promise<undefined> run(); };"]).unwrap();
}
#[test]
fn only_bare_flags_are_recognized_forms_and_repeated_identical_flags_are_idempotent() {
    for name in [
        "SameObject",
        "NewObject",
        "Replaceable",
        "LegacyLenientSetter",
    ] {
        for suffix in ["=Value", "()", "(long arg)", "=*", "=(A,B)"] {
            invalid(
                &format!("interface I {{ [{name}{suffix}] readonly attribute object value; }};"),
                ErrorKind::MemberAttributeForm,
            );
        }
    }
    check(&[
        "interface I { [SameObject /*comment*/,SameObject] readonly attribute object value; };",
    ])
    .unwrap();
}
#[test]
fn type_argument_dictionary_and_definition_positions_are_not_member_positions() {
    for source in [
        "[SameObject] interface I {};",
        "typedef [SameObject] object Value;",
        "interface I { undefined run([NewObject] object value); };",
        "dictionary D { [Replaceable] object value; };",
        "callback F=undefined ([LegacyLenientSetter] object value);",
        "interface I {}; interface mixin M {}; [SameObject] I includes M;",
    ] {
        invalid(source, ErrorKind::MemberAttributePlacement);
    }
}
#[test]
fn opaque_names_and_nested_contents_remain_uninterpreted() {
    check(&["[Opaque=(SameObject,NewObject)] interface I { [Custom=Replaceable,sameObject,_SameObject] readonly attribute long value; };"]).unwrap();
}
#[test]
fn put_forwards_target_resolution_forms_and_cycles_are_explicitly_outside_this_stage() {
    check(&["interface I { [PutForwards=missing] readonly attribute I value; };"]).unwrap();
    check(&["interface I { [SameObject,PutForwards=value] readonly attribute I value; };"])
        .unwrap();
}
#[test]
fn cross_file_type_diagnostics_point_to_the_attribute_alias_use() {
    let sources = [
        "interface I { [SameObject] readonly attribute Value value; };",
        "typedef object? Value;",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::MemberAttributeType);
    let at = e.location.unwrap();
    assert_eq!(at.source, 0);
    assert_eq!(sources[0][at.span].trim(), "Value");
}
#[test]
fn readonly_diagnostic_points_to_the_actual_member_name() {
    let sources = [
        "interface I {};",
        "partial interface I { [Replaceable] attribute object value; };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::MemberAttributeReadonly);
    let at = e.location.unwrap();
    assert_eq!(at.source, 1);
    assert_eq!(&sources[1][at.span], "value");
}
#[test]
fn shared_mixins_and_sibling_hosts_do_not_share_mutable_attribute_state() {
    check(&["interface Value {}; interface mixin M { [SameObject] readonly attribute Value value; }; interface A {}; interface B {}; A includes M; B includes M; interface C { [NewObject] Promise<long> value(); };"]).unwrap();
}
#[test]
fn exact_budget_and_external_cancel_remain_host_failures() {
    let source = "interface Value {}; interface I { [SameObject] readonly attribute Value value; [NewObject] Promise<long> create(); };";
    let i = index::build(&[source]).unwrap();
    let m = members::build(&i).unwrap();
    let unlimited = Budget::unlimited();
    super::super::build_within(&i, &m, Limits::default(), &unlimited).unwrap();
    let cost = u64::MAX - unlimited.remaining();
    let exact = Budget::new(cost);
    super::super::build_within(&i, &m, Limits::default(), &exact).unwrap();
    assert_eq!(exact.remaining(), 0);
    assert_eq!(
        super::super::build_within(&i, &m, Limits::default(), &Budget::new(cost - 1))
            .unwrap_err()
            .kind,
        ErrorKind::Exhausted(Exhausted::Steps)
    );
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(
        super::super::build_within(
            &i,
            &m,
            Limits::default(),
            &Budget::with_cancel(100_000, cancel)
        )
        .unwrap_err()
        .kind,
        ErrorKind::Exhausted(Exhausted::Cancelled)
    );
}
#[test]
fn large_mixin_is_checked_once_despite_two_thousand_hosts() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut s = String::from("interface Value {}; interface mixin M { ");
            for n in 0..1000 {
                s.push_str(&format!("[SameObject] readonly attribute Value field{n}; "));
            }
            s.push_str("};");
            for n in 0..2000 {
                s.push_str(&format!("interface I{n} {{}}; I{n} includes M; "));
            }
            let i = index::build(&[&s]).unwrap();
            let m = members::build_within(
                &i,
                crate::members::Limits {
                    records: 200_000,
                    visits: 3_000_000,
                },
                &Budget::new(20_000_000),
            )
            .unwrap();
            super::super::build_within(&i, &m, Limits::default(), &Budget::new(1_000_000)).unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
}
