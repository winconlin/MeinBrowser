use super::*;
use crate::{index, members};

fn check(sources: &[&str]) -> Result<Types, Error> {
    let index = index::build(sources).expect("syntax and names");
    let members = members::build(&index).expect("member names");
    super::super::build(&index, &members)
}

#[test]
fn named_and_indexed_accessors_are_independent_varieties() {
    check(&["interface I { readonly attribute unsigned long length; getter long item(unsigned long index); getter DOMString named(DOMString name); setter undefined set(unsigned long index, long value); setter undefined put(DOMString name, DOMString value); deleter boolean remove(DOMString name); };"]).unwrap();
}

fn invalid(source: &str, expected: ErrorKind) {
    let error = check(&[source]).unwrap_err();
    assert_eq!(error.kind, expected, "{source}");
    let at = error.location.expect("source location");
    assert!(source.get(at.span).is_some());
}

#[test]
fn getter_setter_and_deleter_require_exact_arity_and_mandatory_arguments() {
    for operation in [
        "getter long get()",
        "getter long get(DOMString key, long extra)",
        "setter undefined set(DOMString key)",
        "setter undefined set(DOMString key, long value, long extra)",
        "deleter boolean remove()",
        "deleter boolean remove(DOMString key, long extra)",
        "getter long get(optional DOMString key)",
        "getter long get(DOMString... key)",
        "setter undefined set(DOMString key, optional long value)",
        "setter undefined set(DOMString key, long... value)",
        "deleter boolean remove(optional DOMString key)",
    ] {
        invalid(
            &format!("interface I {{ {operation}; }};"),
            ErrorKind::SpecialOperationSignature,
        );
    }
}

#[test]
fn key_types_are_exact_and_do_not_use_js_string_or_integer_coercion() {
    for ty in [
        "USVString",
        "ByteString",
        "DOMString?",
        "long",
        "unsigned short",
        "unsigned long long",
        "unsigned long?",
        "(DOMString or long)",
        "object",
    ] {
        invalid(
            &format!("interface I {{ getter long get({ty} key); }};"),
            ErrorKind::SpecialOperationKey,
        );
    }
    invalid(
        "interface I { deleter boolean remove(unsigned long key); };",
        ErrorKind::SpecialOperationKey,
    );
    invalid(
        "interface _DOMString {}; interface I { getter long get(_DOMString key); };",
        ErrorKind::SpecialOperationKey,
    );
}

#[test]
fn annotations_and_forward_aliases_resolve_without_changing_the_usage_location() {
    check(&["interface I { readonly attribute Size _length; getter any item(Index index); getter long named(Key name); };", "typedef [Tag] unsigned long Index; typedef [Tag] unsigned short Size; typedef [Tag] DOMString Key;"]).unwrap();
    let sources = [
        "interface I { getter long get(Key name); };",
        "typedef USVString Key;",
    ];
    let error = check(&sources).unwrap_err();
    assert_eq!(error.kind, ErrorKind::SpecialOperationKey);
    let at = error.location.unwrap();
    assert_eq!(at.source, 0);
    assert_eq!(sources[0][at.span].trim_end(), "Key");
}

#[test]
fn duplicate_varieties_are_rejected_across_partials_without_conflating_named_and_indexed() {
    for (first, second) in [
        (
            "getter long a(DOMString key)",
            "getter DOMString b(DOMString key)",
        ),
        (
            "setter undefined a(DOMString key, long value)",
            "setter undefined b(DOMString key, DOMString value)",
        ),
        (
            "getter long a(unsigned long key)",
            "getter DOMString b(unsigned long key)",
        ),
        (
            "setter undefined a(unsigned long key, long value)",
            "setter undefined b(unsigned long key, DOMString value)",
        ),
        (
            "deleter boolean a(DOMString key)",
            "deleter boolean b(DOMString key)",
        ),
    ] {
        invalid(
            &format!("interface I {{ {first}; }}; partial interface I {{ {second}; }};"),
            ErrorKind::DuplicateSpecialOperation,
        );
    }
    let sources = [
        "partial interface I { getter long a(DOMString key); };",
        "interface I { getter long b(DOMString key); };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::DuplicateSpecialOperation);
    assert_eq!(e.location.unwrap().source, 1);
}

#[test]
fn setters_and_deleters_need_a_getter_of_the_matching_variety() {
    for source in [
        "interface I { setter undefined set(DOMString key, long value); };",
        "interface I { setter undefined set(unsigned long key, long value); };",
        "interface I { deleter boolean remove(DOMString key); };",
        "interface I { getter long get(DOMString key); setter undefined set(unsigned long key, long value); };",
        "interface I { readonly attribute long length; getter long get(unsigned long key); deleter boolean remove(DOMString key); };",
    ] {
        invalid(source, ErrorKind::MissingSpecialGetter);
    }
    check(&["interface I { setter undefined set(DOMString key, long value); deleter boolean remove(DOMString key); }; partial interface I { getter long get(DOMString key); };"]).unwrap();
}

#[test]
fn indexed_properties_need_an_instance_integer_length_attribute() {
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
        check(&[&format!(
            "interface I {{ attribute {ty} length; getter any get(unsigned long index); }};"
        )])
        .unwrap();
    }
    for ty in [
        "boolean",
        "float",
        "double",
        "bigint",
        "DOMString",
        "unsigned long?",
        "(long or DOMString)",
    ] {
        invalid(
            &format!(
                "interface I {{ readonly attribute {ty} length; getter any get(unsigned long index); }};"
            ),
            ErrorKind::IndexedLength,
        );
    }
    invalid(
        "interface I { getter any get(unsigned long index); };",
        ErrorKind::IndexedLength,
    );
    invalid(
        "interface I { static readonly attribute unsigned long length; getter any get(unsigned long index); };",
        ErrorKind::IndexedLength,
    );
    invalid(
        "interface I { unsigned long length(); getter any get(unsigned long index); };",
        ErrorKind::IndexedLength,
    );
    check(&["interface I { attribute double length; getter long named(DOMString name); };"])
        .unwrap();
}

#[test]
fn inherited_getters_and_lengths_satisfy_local_conditions_and_local_overrides_are_allowed() {
    check(&["interface Base { readonly attribute unsigned long length; getter long indexed(unsigned long index); getter long named(DOMString name); stringifier; }; interface Child : Base { setter undefined set(unsigned long index, long value); setter undefined put(DOMString name, long value); deleter boolean remove(DOMString name); getter long replacement(unsigned long index); stringifier; };"]).unwrap();
    invalid(
        "interface Base { getter long get(DOMString name); }; interface Child : Base { setter undefined set(unsigned long index, long value); };",
        ErrorKind::MissingSpecialGetter,
    );
    invalid(
        "interface Base { readonly attribute unsigned long length; getter long get(unsigned long index); }; interface Child : Base { attribute double length; };",
        ErrorKind::IndexedLength,
    );
    check(&["interface Base { attribute double length; }; interface Child : Base { attribute long length; getter long get(unsigned long index); };"]).unwrap();
}

#[test]
fn stringifier_attributes_allow_domstring_and_usvstring_but_no_other_shapes() {
    for ty in ["DOMString", "USVString"] {
        check(&[&format!(
            "typedef [Tag] {ty} Text; interface I {{ stringifier readonly attribute Text text; }};"
        )])
        .unwrap();
    }
    check(&["interface I { stringifier; };"]).unwrap();
    for ty in [
        "ByteString",
        "DOMString?",
        "USVString?",
        "long",
        "object",
        "(DOMString or long)",
    ] {
        invalid(
            &format!("interface I {{ stringifier attribute {ty} text; }};"),
            ErrorKind::StringifierType,
        );
    }
    invalid(
        "enum Text {\"on\"}; interface I { stringifier attribute Text text; };",
        ErrorKind::StringifierType,
    );
}

#[test]
fn stringifier_cardinality_covers_bare_attributes_partials_and_includes() {
    for source in [
        "interface I { stringifier; stringifier; };",
        "interface I { stringifier; stringifier attribute DOMString text; };",
        "interface I { stringifier attribute DOMString first; }; partial interface I { stringifier attribute USVString second; };",
        "interface I { stringifier; }; interface mixin M { stringifier; }; I includes M;",
        "interface I {}; interface mixin A { stringifier; }; interface mixin B { stringifier; }; I includes A; I includes B;",
    ] {
        invalid(source, ErrorKind::DuplicateStringifier);
    }
    check(&["interface mixin M { stringifier; }; interface A {}; interface B {}; A includes M; B includes M;"]).unwrap();
    check(&["interface mixin M {}; partial interface mixin M { stringifier attribute DOMString text; }; interface I {}; I includes M;"]).unwrap();
}

#[test]
fn mixin_lengths_are_resolved_before_checking_indexed_getters() {
    check(&[
        "interface I { getter long get(unsigned long index); }; I includes M;",
        "interface mixin M { readonly attribute unsigned long length; };",
    ])
    .unwrap();
    invalid(
        "interface I { getter long get(unsigned long index); }; interface mixin M { readonly attribute double length; }; I includes M;",
        ErrorKind::IndexedLength,
    );
}

#[test]
fn ordinary_operations_and_unspecified_result_types_are_not_narrowed() {
    check(&["interface I { undefined ordinary(optional USVString key); getter object get(DOMString key); setter long put(DOMString key, boolean value); deleter object remove(DOMString key); };"]).unwrap();
}

#[test]
fn exact_work_and_cancellation_remain_host_errors() {
    let source = "interface mixin M { readonly attribute unsigned long length; }; interface Base { getter any get(unsigned long index); }; Base includes M; interface Child : Base { setter undefined set(unsigned long index, long value); };";
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
            &Budget::with_cancel(100000, cancel)
        )
        .unwrap_err()
        .kind,
        ErrorKind::Exhausted(Exhausted::Cancelled)
    );
}

#[test]
fn deep_inherited_presence_checks_fit_a_small_stack_and_shared_quota() {
    std::thread::Builder::new().stack_size(1<<20).spawn(|| {
        let mut source=String::from("interface I0 { readonly attribute unsigned long length; getter long item(unsigned long index); }; ");
        for n in 1..2000 { source.push_str(&format!("interface I{n} : I{} {{ setter undefined set(unsigned long index, long value); }}; ",n-1)); }
        let index=index::build(&[&source]).unwrap();let members=members::build(&index).unwrap();
        let types=super::super::build_within(&index,&members,Limits::default(),&Budget::new(500_000)).unwrap();
        drop(types);drop(members);drop(index);
    }).unwrap().join().unwrap();
}

#[test]
fn one_large_mixin_is_summarized_once_for_many_hosts() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::from("interface mixin M { stringifier; ");
            for n in 0..1000 {
                source.push_str(&format!("readonly attribute long field{n}; "));
            }
            source.push_str("}; ");
            for n in 0..1000 {
                source.push_str(&format!("interface I{n} {{}}; I{n} includes M; "));
            }
            let index = index::build(&[&source]).unwrap();
            let members = members::build_within(
                &index,
                crate::members::Limits {
                    records: 200_000,
                    visits: 3_000_000,
                },
                &Budget::new(20_000_000),
            )
            .unwrap();
            let types = super::super::build_within(
                &index,
                &members,
                Limits::default(),
                &Budget::new(200_000),
            )
            .unwrap();
            drop(types);
            drop(members);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}
