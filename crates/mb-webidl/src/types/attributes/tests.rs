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
fn clamp_and_enforce_accept_all_fixed_width_integers_and_nullable_forms() {
    for attr in ["Clamp", "EnforceRange"] {
        for ty in [
            "byte",
            "octet",
            "short",
            "unsigned short",
            "long",
            "unsigned long",
            "long long",
            "unsigned long long",
        ] {
            for suffix in ["", "?"] {
                check(&[&format!("typedef [{attr}] {ty}{suffix} Value;")]).unwrap();
            }
        }
    }
}
#[test]
fn integer_attributes_reject_other_primitives_unions_and_containers() {
    for attr in ["Clamp", "EnforceRange"] {
        for ty in [
            "boolean",
            "bigint",
            "float",
            "unrestricted float",
            "double",
            "unrestricted double",
            "DOMString",
            "any",
            "object",
            "symbol",
            "ArrayBuffer",
            "(long or DOMString)",
            "sequence<long>",
            "Promise<long>",
        ] {
            invalid(
                &format!("typedef [{attr}] {ty} Value;"),
                ErrorKind::TypeAttributeTarget,
            );
        }
    }
}
#[test]
fn empty_string_attribute_accepts_only_non_nullable_dom_and_usv_strings() {
    for ty in ["DOMString", "USVString"] {
        check(&[&format!("typedef [LegacyNullToEmptyString] {ty} Value;")]).unwrap();
    }
    for ty in [
        "DOMString?",
        "USVString?",
        "ByteString",
        "long",
        "object",
        "(DOMString or long)",
    ] {
        invalid(
            &format!("typedef [LegacyNullToEmptyString] {ty} Value;"),
            ErrorKind::TypeAttributeTarget,
        );
    }
}
#[test]
fn allow_shared_accepts_nullable_views_but_not_buffers() {
    for ty in [
        "DataView",
        "Int8Array",
        "Int16Array",
        "Int32Array",
        "Uint8Array",
        "Uint8ClampedArray",
        "Uint16Array",
        "Uint32Array",
        "BigInt64Array",
        "BigUint64Array",
        "Float16Array",
        "Float32Array",
        "Float64Array",
    ] {
        for suffix in ["", "?"] {
            check(&[&format!("typedef [AllowShared] {ty}{suffix} Value;")]).unwrap();
        }
    }
    for ty in [
        "ArrayBuffer",
        "SharedArrayBuffer",
        "long",
        "object",
        "(DataView or DOMString)",
    ] {
        invalid(
            &format!("typedef [AllowShared] {ty} Value;"),
            ErrorKind::TypeAttributeTarget,
        );
    }
}
#[test]
fn allow_resizable_accepts_buffers_and_views_with_nullable_forms() {
    for ty in [
        "ArrayBuffer",
        "SharedArrayBuffer",
        "DataView",
        "Uint8Array",
        "Float64Array",
    ] {
        for suffix in ["", "?"] {
            check(&[&format!("typedef [AllowResizable] {ty}{suffix} Value;")]).unwrap();
        }
    }
    for ty in [
        "long",
        "any",
        "object",
        "sequence<ArrayBuffer>",
        "(ArrayBuffer or DOMString)",
    ] {
        invalid(
            &format!("typedef [AllowResizable] {ty} Value;"),
            ErrorKind::TypeAttributeTarget,
        );
    }
    check(&["typedef [AllowShared,AllowResizable] DataView? Value;"]).unwrap();
}
#[test]
fn only_bare_flags_are_accepted_for_the_five_recognized_attributes() {
    for name in [
        "Clamp",
        "EnforceRange",
        "LegacyNullToEmptyString",
        "AllowShared",
        "AllowResizable",
    ] {
        for form in [
            format!("{name}=X"),
            format!("{name}()"),
            format!("{name}(long value)"),
            format!("{name}=*"),
            format!("{name}=(X,Y)"),
        ] {
            invalid(
                &format!("typedef [{form}] long Value;"),
                ErrorKind::TypeAttributeForm,
            );
        }
    }
    check(&["typedef [Clamp /*comment*/, Clamp] long Value;"]).unwrap();
}
#[test]
fn opaque_attributes_are_preserved_without_interpreting_nested_flag_names() {
    check(&["[Custom=(Clamp,EnforceRange)] interface I { [Other(Clamp)] undefined run([Tag] long value); }; typedef [Opaque={Clamp=No},clamp,_Clamp] long Value;"]).unwrap();
}
#[test]
fn non_optional_variadic_constructor_and_callback_arguments_associate_type_flags() {
    check(&["interface I { constructor([Clamp] long count); undefined run([EnforceRange] long count, [Clamp] long... rest); async_iterable<long>([Clamp] long... rest); }; callback F = undefined ([Clamp] long value); callback interface C { undefined run([EnforceRange] long value); };"]).unwrap();
}
#[test]
fn optional_argument_flags_belong_after_the_optional_keyword() {
    check(&["interface I { undefined run(optional [Clamp] long value=0); };"]).unwrap();
    invalid(
        "interface I { undefined run([Clamp] optional long value=0); };",
        ErrorKind::TypeAttributePlacement,
    );
    invalid(
        "interface I { undefined run(optional [Clamp] DOMString value); };",
        ErrorKind::TypeAttributeTarget,
    );
}
#[test]
fn dictionary_flag_association_distinguishes_required_and_optional_fields() {
    check(&["dictionary D { [Clamp] long count; required [EnforceRange] long requiredCount; }; partial dictionary D { [AllowShared] DataView view; };"]).unwrap();
    invalid(
        "dictionary D { [Clamp] required long count; };",
        ErrorKind::TypeAttributePlacement,
    );
    invalid(
        "dictionary D { [EnforceRange] DOMString count; };",
        ErrorKind::TypeAttributeTarget,
    );
}
#[test]
fn known_type_flags_on_definitions_members_includes_and_typedef_headers_are_rejected() {
    for source in [
        "[Clamp] interface I {};",
        "[Clamp] dictionary D {};",
        "[Clamp] typedef long Value;",
        "interface I { [Clamp] attribute long value; };",
        "interface I { [Clamp] long run(); };",
        "interface I { [Clamp] constructor(); };",
        "interface I {}; interface mixin M {}; [Clamp] I includes M;",
    ] {
        invalid(source, ErrorKind::TypeAttributePlacement);
    }
}
#[test]
fn conflicting_integer_flags_are_rejected_directly_and_through_aliases() {
    for source in [
        "typedef [Clamp,EnforceRange] long Value;",
        "typedef [Clamp] long Base; interface I { undefined run([EnforceRange] Base value); };",
        "typedef [EnforceRange] long Base; interface I { attribute [Clamp] Base? value; };",
    ] {
        invalid(source, ErrorKind::TypeAttributeConflict);
    }
}
#[test]
fn inherited_flags_never_leak_between_shared_alias_uses() {
    check(&["typedef [Tag] long Number; interface I { undefined clamp([Clamp] Number value); undefined enforce([EnforceRange] Number value); readonly attribute Number plain; };"]).unwrap();
    check(&["typedef [AllowResizable] DataView View; interface I { undefined shared([AllowShared] View value); undefined plain(View value); };"]).unwrap();
}
#[test]
fn nullable_types_preserve_association_direction_and_do_not_annotate_containers() {
    check(&["typedef [LegacyNullToEmptyString] DOMString Text; interface I { undefined run(Text? value); }; typedef sequence<[Clamp] long> Values;"]).unwrap();
    invalid(
        "typedef DOMString? Text; interface I { undefined run([LegacyNullToEmptyString] Text value); };",
        ErrorKind::TypeAttributeTarget,
    );
    invalid(
        "typedef [Clamp] sequence<long> Values;",
        ErrorKind::TypeAttributeTarget,
    );
}
#[test]
fn readonly_attributes_reject_range_flags_in_aliases_nullable_unions_and_parameters() {
    for source in [
        "interface I { readonly attribute [Clamp] long value; };",
        "typedef [EnforceRange] long Value; interface I { readonly attribute Value? value; };",
        "typedef [Clamp] long Value; interface I { readonly attribute (Value or DOMString) value; };",
        "interface I { readonly attribute FrozenArray<[EnforceRange] long> values; };",
        "typedef [Clamp] long Value; interface mixin M { readonly attribute FrozenArray<Value> values; };",
        "typedef [Clamp] long Value; interface I { readonly attribute Promise<Value> future; };",
    ] {
        invalid(source, ErrorKind::TypeAttributeReadonly);
    }
    check(&["interface I { attribute [Clamp] long value; readonly attribute [AllowShared] DataView view; };"]).unwrap();
}
#[test]
fn cross_file_conflict_reports_the_new_annotation_use() {
    let sources = [
        "interface I { undefined run([EnforceRange] Value value); };",
        "typedef [Clamp] long Value;",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::TypeAttributeConflict);
    let at = e.location.unwrap();
    assert_eq!(at.source, 0);
    assert!(sources[0][at.span].starts_with("Value"));
}
#[test]
fn cross_file_readonly_reports_the_readonly_use_not_the_alias_definition() {
    let sources = [
        "typedef [Clamp] long Value;",
        "interface I { readonly attribute Value value; };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::TypeAttributeReadonly);
    let at = e.location.unwrap();
    assert_eq!(at.source, 1);
    assert_eq!(sources[1][at.span].trim(), "Value");
}
#[test]
fn cross_file_target_diagnostic_reports_the_argument_use() {
    let sources = [
        "interface I { undefined run([AllowShared] Value value); };",
        "typedef ArrayBuffer Value;",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::TypeAttributeTarget);
    let at = e.location.unwrap();
    assert_eq!(at.source, 0);
    assert_eq!(sources[0][at.span].trim(), "Value");
}
#[test]
fn exact_budget_and_external_cancel_do_not_return_partial_types() {
    let source = "typedef [Tag] long Number; interface I { undefined clamp([Clamp] Number value); undefined enforce([EnforceRange] Number value); readonly attribute Number plain; };";
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
fn three_thousand_container_layers_and_two_thousand_alias_uses_fit_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = format!(
                "typedef FrozenArray<{}[Clamp] long{}> Values; interface I {{ ",
                "sequence<".repeat(3000),
                ">".repeat(3000)
            );
            for n in 0..2000 {
                source.push_str(&format!("attribute Values value{n}; "));
            }
            source.push_str("};");
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            super::super::build_within(
                &index,
                &members,
                Limits::default(),
                &Budget::new(2_000_000),
            )
            .unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn nominal_dictionary_fields_are_independent_uses_inside_readonly_arrays() {
    check(&["dictionary D { [Clamp] long count; }; interface I { readonly attribute FrozenArray<D> values; };"]).unwrap();
}
#[test]
fn shared_container_diamonds_do_not_expand_paths_exponentially() {
    std::thread::Builder::new().stack_size(1 << 20).spawn(|| {
        let mut source=String::from("typedef [Clamp] long Leaf; typedef (sequence<Leaf> or record<DOMString,Leaf>) U0; ");
        for n in 1..1500 {source.push_str(&format!("typedef (sequence<U{0}> or record<DOMString,U{0}>) U{n}; ",n-1));}
        source.push_str("interface I { attribute FrozenArray<U1499> values; };");
        let index=index::build(&[&source]).unwrap();let members=members::build(&index).unwrap();
        super::super::build_within(&index,&members,Limits::default(),&Budget::new(1_500_000)).unwrap();
    }).unwrap().join().unwrap();
}
