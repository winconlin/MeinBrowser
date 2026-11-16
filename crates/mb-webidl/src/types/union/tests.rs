use super::*;
use crate::{index, members};

fn check(source: &str) -> Result<Types, Error> {
    let index = index::build(&[source]).expect("syntax and names");
    let members = members::build(&index).expect("member rules");
    super::super::build(&index, &members)
}

#[test]
fn primitive_categories_are_distinguished_instead_of_rust_numeric_variants() {
    check("typedef (boolean or long or bigint or DOMString or symbol) U;").unwrap();
    for source in [
        "typedef (long or double) U;",
        "typedef (DOMString or USVString) U;",
        "typedef (long or long) U;",
    ] {
        assert_eq!(
            check(source).unwrap_err().kind,
            ErrorKind::IndistinguishableUnion
        );
    }
}

const PREFIX: &str = "interface I {}; interface J {}; dictionary D {}; dictionary E {}; callback F = undefined (); callback G = undefined (); callback interface C { undefined handle(); }; enum State {\"on\"};";

#[test]
fn all_twelve_category_pairs_match_the_normative_table_in_both_directions() {
    // Rows transcribed from the standard, independent of table(). (a) uses
    // unrelated I/J; (c) uses a callback without the legacy attribute.
    let expected = [
        "011111111011",
        "101111111111",
        "110111111111",
        "111011111111",
        "111101111111",
        "111110100000",
        "111111011111",
        "111110111111",
        "111110110111",
        "011110111011",
        "111110111100",
        "111110111100",
    ];
    let left = [
        "undefined",
        "boolean",
        "long",
        "bigint",
        "DOMString",
        "object",
        "symbol",
        "I",
        "F",
        "D",
        "async_sequence<long>",
        "sequence<long>",
    ];
    let right = [
        "undefined",
        "boolean",
        "double",
        "bigint",
        "USVString",
        "object",
        "symbol",
        "J",
        "G",
        "E",
        "async_sequence<DOMString>",
        "FrozenArray<DOMString>",
    ];
    for (i, a) in left.iter().enumerate() {
        for (j, b) in right.iter().enumerate() {
            let source = format!("{PREFIX} typedef ({a} or {b}) Choice;");
            let result = check(&source);
            assert_eq!(
                result.is_ok(),
                expected[i].as_bytes()[j] == b'1',
                "{a} / {b}: {result:?}"
            );
            if let Err(error) = result {
                assert_eq!(error.kind, ErrorKind::IndistinguishableUnion);
            }
        }
    }
}

#[test]
fn enums_callback_interfaces_and_generic_parameters_use_their_correct_categories() {
    for pair in [
        "State or DOMString",
        "C or D",
        "record<DOMString,long> or D",
        "C or undefined",
        "sequence<long> or FrozenArray<DOMString>",
        "async_sequence<long> or sequence<long>",
    ] {
        assert_eq!(
            check(&format!("{PREFIX} typedef ({pair}) Choice;"))
                .unwrap_err()
                .kind,
            ErrorKind::IndistinguishableUnion,
            "{pair}"
        );
    }
    for pair in [
        "C or I",
        "record<DOMString,long> or F",
        "async_sequence<long> or DOMString",
        "bigint or double",
    ] {
        check(&format!("{PREFIX} typedef ({pair}) Choice;")).unwrap();
    }
}

#[test]
fn nominal_interfaces_must_be_distinct_and_have_no_ancestor_relationship() {
    for source in [
        "interface I {}; typedef (I or I) Choice;",
        "interface A {}; interface B : A {}; typedef (A or B) Choice;",
        "interface B : A {}; interface A {}; typedef (B or A) Choice;",
    ] {
        assert_eq!(
            check(source).unwrap_err().kind,
            ErrorKind::IndistinguishableUnion
        );
    }
    check(
        "interface Base {}; interface I : Base {}; interface J : Base {}; typedef (I or J) Choice;",
    )
    .unwrap();
    check("interface I {}; interface J {}; interface mixin M {}; I includes M; J includes M; typedef (I or J) Choice;").unwrap();
}

#[test]
fn all_fifteen_buffer_kinds_have_nominal_identity() {
    let buffers = [
        "ArrayBuffer",
        "SharedArrayBuffer",
        "DataView",
        "Int8Array",
        "Int16Array",
        "Int32Array",
        "Uint8Array",
        "Uint16Array",
        "Uint32Array",
        "Uint8ClampedArray",
        "BigInt64Array",
        "BigUint64Array",
        "Float16Array",
        "Float32Array",
        "Float64Array",
    ];
    for (i, a) in buffers.iter().enumerate() {
        for (j, b) in buffers.iter().enumerate() {
            let result = check(&format!("typedef ({a} or {b}) Choice;"));
            assert_eq!(result.is_ok(), i != j, "{a}/{b}");
        }
    }
    check("interface _ArrayBuffer {}; typedef (_ArrayBuffer or ArrayBuffer) Choice;").unwrap();
}

#[test]
fn callback_dictionary_distinguishability_obeys_the_definition_legacy_attribute() {
    for other in ["D", "C", "record<DOMString,long>"] {
        check(&format!("{PREFIX} typedef (F or {other}) Choice;")).unwrap();
        let source = format!(
            "{PREFIX} [LegacyTreatNonObjectAsNull] callback Legacy = undefined (); typedef (Legacy or {other}) Choice;"
        );
        assert_eq!(
            check(&source).unwrap_err().kind,
            ErrorKind::IndistinguishableUnion
        );
        let source = format!(
            "{PREFIX} [LegacyTreatNonObjectAsNull] callback Legacy = undefined (); typedef (record<DOMString,long> or Legacy) Choice;"
        );
        assert_eq!(
            check(&source).unwrap_err().kind,
            ErrorKind::IndistinguishableUnion
        );
    }
    check(
        "[LegacyTreatNonObjectAsNull] callback F = undefined (); typedef (F or DOMString) Choice;",
    )
    .unwrap();
    assert_eq!(check("dictionary D {}; [LegacyTreatNonObjectAsNull] callback F = undefined (); typedef F Alias; typedef (Alias or D) Choice;").unwrap_err().kind,ErrorKind::IndistinguishableUnion);
}

#[test]
fn known_callback_flag_is_read_only_at_the_top_attribute_name_and_in_no_args_form() {
    check("dictionary D {}; [Other=(LegacyTreatNonObjectAsNull)] callback F = undefined (); typedef (F or D) Choice;").unwrap();
    assert_eq!(check("dictionary D {}; [Other=(A,B), /*x*/ LegacyTreatNonObjectAsNull /*y*/] callback F = undefined (); typedef (F or D) Choice;").unwrap_err().kind, ErrorKind::IndistinguishableUnion);
    for form in [
        "LegacyTreatNonObjectAsNull=Value",
        "LegacyTreatNonObjectAsNull()",
        "LegacyTreatNonObjectAsNull(Thing)",
    ] {
        let source = format!("[{form}] callback F = undefined (); typedef (F or long) Choice;");
        assert_eq!(
            check(&source).unwrap_err().kind,
            ErrorKind::InvalidCallbackAttribute
        );
    }
}

#[test]
fn aliases_annotations_nullable_and_nested_unions_do_not_hide_duplicates() {
    for source in [
        "typedef long Number; typedef (Number or double) Choice;",
        "typedef (long or DOMString) Inner; typedef (Inner or USVString) Choice;",
        "typedef ([Tag] long? or double) Choice;",
        "typedef (long or DOMString) Inner; typedef (Inner or Inner) Choice;",
        "typedef Promise<(long or double)> P;",
        "typedef record<DOMString,(long or double)> R;",
    ] {
        assert_eq!(
            check(source).unwrap_err().kind,
            ErrorKind::IndistinguishableUnion,
            "{source}"
        );
    }
    check("typedef [Outer] (long or DOMString) Inner; typedef (Inner or boolean) Choice;").unwrap();
    check("typedef (long or DOMString)? Nullable; typedef Promise<Nullable> P;").unwrap();
    check("typedef (long? or DOMString) Inner; typedef (sequence<Inner> or boolean?) Choice;")
        .unwrap();
}

#[test]
fn unsupported_table_types_are_rejected_inside_unions_only() {
    assert_eq!(
        check("typedef (ObservableArray<long> or DOMString) Choice;")
            .unwrap_err()
            .kind,
        ErrorKind::IndistinguishableUnion
    );
    check("typedef ObservableArray<long> Values;").unwrap();
    // any/Promise keep their earlier, more specific union-form error.
    for source in [
        "typedef any A; typedef (A or long) Choice;",
        "typedef Promise<long> P; typedef (P or long) Choice;",
    ] {
        assert_eq!(check(source).unwrap_err().kind, ErrorKind::UnionType);
    }
}

#[test]
fn forward_aliases_in_other_files_are_checked_and_report_real_union_locations() {
    let sources = ["typedef (Name or DOMString) Choice;", "enum Name {\"x\"};"];
    let index = index::build(&sources).unwrap();
    let members = members::build(&index).unwrap();
    let error = super::super::build(&index, &members).unwrap_err();
    assert_eq!(error.kind, ErrorKind::IndistinguishableUnion);
    let at = error.location.unwrap();
    assert_eq!(at.source, 0);
    assert!(sources[0][at.span].starts_with("(Name or DOMString)"));
}

#[test]
fn visit_and_interface_limits_and_joint_budget_are_explicit_host_failures() {
    let source = "interface I {}; interface J {}; typedef (I or J) Choice;";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let limits = Limits {
        union_visits: 4,
        union_interfaces: 2,
        ..Limits::default()
    };
    super::super::build_within(&index, &members, limits, &Budget::unlimited()).unwrap();
    for (limits, kind) in [
        (
            Limits {
                union_visits: 3,
                ..limits
            },
            Limit::UnionVisits,
        ),
        (
            Limits {
                union_interfaces: 1,
                ..limits
            },
            Limit::UnionInterfaces,
        ),
    ] {
        assert_eq!(
            super::super::build_within(&index, &members, limits, &Budget::unlimited())
                .unwrap_err()
                .kind,
            ErrorKind::Limit(kind)
        );
    }
    let source = "interface I {}; typedef (long or DOMString) Choice;";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let limits = Limits {
        union_interfaces: 0,
        ..Limits::default()
    };
    let budget = Budget::unlimited();
    super::super::build_within(&index, &members, limits, &budget).unwrap();
    let cost = u64::MAX - budget.remaining();
    assert_eq!(
        super::super::build_within(&index, &members, limits, &Budget::new(cost - 1))
            .unwrap_err()
            .kind,
        ErrorKind::Exhausted(Exhausted::Steps)
    );
    let exact = Budget::new(cost);
    super::super::build_within(&index, &members, limits, &exact).unwrap();
    assert_eq!(exact.remaining(), 0);
}

#[test]
fn deep_nested_unions_and_drop_fit_a_small_stack_without_quadratic_prefix_checks() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::new();
            for n in 0..2000 {
                source.push_str(&format!("interface I{n} {{}}; "));
            }
            source.push_str("typedef ");
            for n in 0..1999 {
                source.push_str(&format!("(I{n} or "));
            }
            source.push_str("I1999");
            source.push_str(&")".repeat(1999));
            source.push_str(" Choice;");
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            let types = super::super::build_within(
                &index,
                &members,
                Limits {
                    union_visits: 10000,
                    union_interfaces: 2000,
                    ..Limits::default()
                },
                &Budget::new(300_000),
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

#[test]
fn many_deep_siblings_use_one_interface_index_instead_of_pairwise_ancestor_walks() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::from("interface Base0 {}; ");
            for n in 1..1000 {
                source.push_str(&format!("interface Base{n} : Base{} {{}}; ", n - 1));
            }
            for n in 0..600 {
                source.push_str(&format!("interface Leaf{n} : Base999 {{}}; "));
            }
            source.push_str("typedef (");
            for n in 0..600 {
                if n != 0 {
                    source.push_str(" or ");
                }
                source.push_str(&format!("Leaf{n}"));
            }
            source.push_str(") Choice;");
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            let types = super::super::build_within(
                &index,
                &members,
                Limits {
                    union_visits: 10000,
                    union_interfaces: 1600,
                    ..Limits::default()
                },
                &Budget::new(150_000),
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
