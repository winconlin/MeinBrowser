use super::*;
use crate::{index, members};

fn check(sources: &[&str]) -> Result<Types, Error> {
    let index = index::build(sources).expect("syntax and names");
    let members = members::build(&index).expect("member forms");
    super::super::build(&index, &members)
}
fn invalid(source: &str, expected: ErrorKind) {
    let e = check(&[source]).unwrap_err();
    assert_eq!(e.kind, expected, "{source}");
    assert!(source.get(e.location.unwrap().span).is_some());
}
fn constant(ty: &str, value: &str) -> String {
    format!("interface I {{ const {ty} value = {value}; }};")
}
const PREFIX: &str = "enum State {\"on\",\"off\"}; dictionary D {}; interface Handle {}; callback F = undefined (); callback interface C { undefined run(); }; ";
fn default(ty: &str, value: &str) -> String {
    format!("{PREFIX} dictionary V {{ {ty} value = {value}; }};")
}

#[test]
fn all_integer_widths_check_exact_bounds_in_decimal_hex_and_octal() {
    for (ty, bits, signed) in [
        ("byte", 8, true),
        ("octet", 8, false),
        ("short", 16, true),
        ("unsigned short", 16, false),
        ("long", 32, true),
        ("unsigned long", 32, false),
        ("long long", 64, true),
        ("unsigned long long", 64, false),
    ] {
        let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
        let max = (1i128 << (bits - i32::from(signed))) - 1;
        for n in [min - 1, min, min + 1, max - 1, max, max + 1] {
            let sign = if n < 0 { "-" } else { "" };
            let magnitude = n.unsigned_abs();
            for raw in [
                n.to_string(),
                format!("{sign}0x{magnitude:x}"),
                format!("{sign}0{magnitude:o}"),
            ] {
                let result = check(&[&constant(ty, &raw)]);
                assert_eq!(
                    result.is_ok(),
                    (min..=max).contains(&n),
                    "{ty} {raw}: {result:?}"
                );
                if let Err(e) = result {
                    assert_eq!(e.kind, ErrorKind::ConstantValue);
                }
            }
        }
        check(&[&constant(ty, "-0")]).unwrap();
    }
    // These adjacent 64-bit values collapse under an f64 conversion.
    check(&[&constant("unsigned long long", "18446744073709551615")]).unwrap();
    invalid(
        &constant("unsigned long long", "18446744073709551616"),
        ErrorKind::ConstantValue,
    );
}

#[test]
fn constants_use_literal_types_instead_of_javascript_coercion() {
    for (ty, raw) in [
        ("boolean", "true"),
        ("boolean", "false"),
        ("double", "1"),
        ("float", "0.5"),
        ("unrestricted float", "Infinity"),
        ("unrestricted double", "-Infinity"),
        ("unrestricted double", "NaN"),
        ("bigint", "0xffffffffffffffffffffffffffffffffffff"),
    ] {
        check(&[&constant(ty, raw)]).unwrap();
    }
    for (ty, raw) in [
        ("boolean", "0"),
        ("long", "true"),
        ("long", "1.0"),
        ("long", "NaN"),
        ("bigint", "1.0"),
        ("bigint", "false"),
        ("float", "Infinity"),
        ("double", "NaN"),
        ("double", "true"),
    ] {
        invalid(&constant(ty, raw), ErrorKind::ConstantValue);
    }
}

#[test]
fn floating_decimal_literals_round_at_the_declared_precision() {
    for (ty, raw) in [
        ("float", "3.4028234e38"),
        ("float", "1e-10000"),
        ("double", "1.7976931348623157e308"),
        ("double", "-0.0"),
        ("unrestricted float", "1e10000"),
        ("unrestricted double", "-1e10000"),
    ] {
        check(&[&constant(ty, raw)]).unwrap();
    }
    for (ty, raw) in [
        ("float", "3.4028236e38"),
        ("float", "1e10000"),
        ("double", "1.7976931348623159e308"),
    ] {
        invalid(&constant(ty, raw), ErrorKind::ConstantValue);
    }
}

#[test]
fn string_defaults_preserve_raw_content_and_bytestring_checks_scalars() {
    for (ty, raw) in [
        ("DOMString", "\"a\\nb\""),
        ("USVString", "\"😀\""),
        ("ByteString", "\"ÿ\""),
        ("DOMString", "\"a\nb\""),
        ("State", "\"on\""),
        ("DOMString?", "null"),
    ] {
        check(&[&default(ty, raw)]).unwrap();
    }
    for (ty, raw) in [
        ("ByteString", "\"Ā\""),
        ("ByteString", "\"😀\""),
        ("State", "\"ON\""),
        ("State", "\"o\\n\""),
        ("State", "1"),
        ("DOMString", "null"),
        ("long", "\"0\""),
    ] {
        invalid(&default(ty, raw), ErrorKind::DefaultValue);
    }
    check(&[r#"enum E {"\u0061"}; dictionary V { E value = "\u0061"; };"#]).unwrap();
    invalid(
        r#"enum E {"\u0061"}; dictionary V { E value = "a"; };"#,
        ErrorKind::DefaultValue,
    );
}

#[test]
fn empty_sequence_and_dictionary_defaults_match_only_the_prescribed_types() {
    for (ty, raw) in [
        ("sequence<long>", "[]"),
        ("sequence<long>?", "[ /*empty*/ ]"),
        ("(sequence<long> or DOMString)", "[]"),
        ("D", "{}"),
        ("(D or DOMString)", "{ /*empty*/ }"),
    ] {
        check(&[&default(ty, raw)]).unwrap();
    }
    for (ty, raw) in [
        ("FrozenArray<long>", "[]"),
        ("async_sequence<long>", "[]"),
        ("record<DOMString,long>", "{}"),
        ("C", "{}"),
        ("object", "{}"),
        ("any", "{}"),
        ("any", "[]"),
        ("long", "[]"),
        ("sequence<long>", "{}"),
        ("Promise<long>", "null"),
    ] {
        invalid(&default(ty, raw), ErrorKind::DefaultValue);
    }
    // No invented prohibition: this pass checks the explicit dictionary type
    // condition, not a future binding's default-value instantiation.
    check(&["dictionary D { required long x; }; dictionary V { D d = {}; };"]).unwrap();
}

#[test]
fn any_defaults_accept_scalar_values_and_nullable_wrappers_stay_transparent() {
    for raw in [
        "null",
        "undefined",
        "true",
        "18446744073709551616",
        "1.5",
        "NaN",
        "\"x\"",
    ] {
        check(&[&default("any", raw)]).unwrap();
    }
    for (ty, raw) in [
        ("long?", "null"),
        ("long?", "1"),
        ("Handle?", "null"),
        ("F?", "null"),
        ("(long? or DOMString)", "null"),
        ("(long or DOMString)?", "\"x\""),
        ("(bigint or long)", "18446744073709551616"),
    ] {
        check(&[&default(ty, raw)]).unwrap();
    }
    for (ty, raw) in [
        ("Handle", "null"),
        ("F", "null"),
        ("object", "null"),
        ("(long or DOMString)", "null"),
        ("boolean", "1"),
    ] {
        invalid(&default(ty, raw), ErrorKind::DefaultValue);
    }
}

#[test]
fn aliases_annotations_and_cross_file_values_report_the_actual_literal() {
    let sources = [
        "typedef [Tag] unsigned long long Count; enum State {\"on\"};",
        "interface I { const Count value = 18446744073709551616; };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::ConstantValue);
    let at = e.location.unwrap();
    assert_eq!(at.source, 1);
    assert_eq!(&sources[1][at.span], "18446744073709551616");
    let sources = [
        "typedef [Tag] State Choice; enum State {\"on\"};",
        "dictionary V { Choice value = \"off\"; };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::DefaultValue);
    let at = e.location.unwrap();
    assert_eq!(at.source, 1);
    assert_eq!(&sources[1][at.span], "\"off\"");
    check(&["typedef [Tag] (long or DOMString) Choice; interface I { undefined run(optional Choice value = \"x\"); };"]).unwrap();
}

#[test]
fn all_signature_kinds_and_dictionary_partials_validate_defaults() {
    for source in [
        "interface I { constructor(optional octet x = 256); };",
        "interface I { undefined f(optional octet x = 256); };",
        "namespace N { undefined f(optional octet x = 256); };",
        "callback F = undefined (optional octet x = 256);",
        "callback interface C { undefined f(optional octet x = 256); };",
        "interface I { async_iterable<long>(optional octet x = 256); };",
        "dictionary D {}; partial dictionary D { octet x = 256; };",
    ] {
        invalid(source, ErrorKind::DefaultValue);
    }
}

#[test]
fn undefined_cannot_be_hidden_in_argument_or_dictionary_union_types() {
    for source in [
        "interface I { undefined f(undefined x); };",
        "callback F = undefined (undefined x);",
        "interface I { constructor((undefined or long) x); };",
        "dictionary D { (undefined or DOMString) x; };",
        "typedef [Tag] (undefined or long) U; interface I { undefined f(U x); };",
    ] {
        invalid(source, ErrorKind::UndefinedType);
    }
    check(&["typedef (undefined or long) U; interface I { U result(); undefined f(Promise<undefined> p, optional any x = undefined); };"]).unwrap();
}

#[test]
fn dictionary_suffix_arguments_need_explicit_optional_and_a_default() {
    for arguments in [
        "D d",
        "optional D d",
        "D d, optional long x",
        "optional D d, optional long x = 0",
        "D... values",
    ] {
        invalid(
            &format!("dictionary D {{}}; interface I {{ undefined f({arguments}); }};"),
            ErrorKind::DictionaryArgumentDefault,
        );
    }
    for arguments in [
        "optional D d = {}",
        "optional D d = {}, optional long x = 0",
        "D d, long x",
        "D first, optional D last = {}, long x",
    ] {
        check(&[&format!(
            "dictionary D {{}}; interface I {{ undefined f({arguments}); }};"
        )])
        .unwrap();
    }
    check(&["dictionary D {}; interface I { undefined f(optional (D or DOMString) d = \"x\"); };"])
        .unwrap();
    invalid(
        "dictionary D {}; callback F = undefined (D d);",
        ErrorKind::DictionaryArgumentDefault,
    );
}

#[test]
fn dictionary_required_members_include_partials_and_ancestors() {
    check(&["dictionary Base {}; dictionary D : Base {}; partial dictionary Base { required long x; }; interface I { undefined f(D d, optional long x); };"]).unwrap();
    check(&["dictionary D { required long x; }; interface I { undefined f(optional D d); };"])
        .unwrap();
    invalid(
        "dictionary Base {}; dictionary D : Base {}; interface I { undefined f(D d); };",
        ErrorKind::DictionaryArgumentDefault,
    );
    check(&["dictionary Base {}; dictionary Sibling : Base { required long x; }; dictionary D : Base {}; interface I { undefined f(optional D d = {}); };"]).unwrap();
}

#[test]
fn bounded_work_is_shared_and_cancellation_is_not_a_type_error() {
    let source = "enum E {\"on\"}; dictionary D { E e = \"on\"; octet n = 255; }; interface I { undefined f(optional D d = {}); };";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let budget = Budget::unlimited();
    super::super::build_within(&index, &members, Limits::default(), &budget).unwrap();
    let cost = u64::MAX - budget.remaining();
    let exact = Budget::new(cost);
    super::super::build_within(&index, &members, Limits::default(), &exact).unwrap();
    assert_eq!(exact.remaining(), 0);
    let e = super::super::build_within(&index, &members, Limits::default(), &Budget::new(cost - 1))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Exhausted(Exhausted::Steps));
    let cancel = Cancel::new();
    cancel.cancel();
    let e = super::super::build_within(
        &index,
        &members,
        Limits::default(),
        &Budget::with_cancel(100_000, cancel),
    )
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Exhausted(Exhausted::Cancelled));
}

#[test]
fn long_inheritance_with_many_dictionary_arguments_fits_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::from("dictionary D0 { required long value; }; ");
            for n in 1..2000 {
                source.push_str(&format!("dictionary D{n} : D{} {{}}; ", n - 1));
            }
            source.push_str("interface I { ");
            for n in 0..2000 {
                source.push_str(&format!("undefined f{n}(D{n} d); "));
            }
            source.push_str("};");
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
fn enormous_integer_literals_are_valid_bigints_without_allocating_bigint_values() {
    let integer = format!("0x{}", "f".repeat(50_000));
    check(&[&constant("bigint", &integer)]).unwrap();
    invalid(
        &constant("unsigned long long", &integer),
        ErrorKind::ConstantValue,
    );
    let source = format!("interface I {{ const bigint n = {integer}; }};");
    let index = index::build(&[&source]).unwrap();
    let members = members::build(&index).unwrap();
    let e = super::super::build_within(&index, &members, Limits::default(), &Budget::new(10_000))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Exhausted(Exhausted::Steps));
}

#[test]
fn exact_floating_integer_bounds_cover_three_bases_and_both_signs() {
    for (ty, raw, valid) in [
        ("float", "340282346638528859811704183484516925439", true),
        ("float", "-340282346638528859811704183484516925439", true),
        ("float", "0xfffffeffffffffffffffffffffffffff", true),
        ("float", "-0xfffffeffffffffffffffffffffffffff", true),
        (
            "float",
            "03777777737777777777777777777777777777777777",
            true,
        ),
        (
            "float",
            "-03777777737777777777777777777777777777777777",
            true,
        ),
        ("float", "340282346638528859811704183484516925440", true),
        ("float", "-340282346638528859811704183484516925440", true),
        ("float", "0xffffff00000000000000000000000000", true),
        ("float", "-0xffffff00000000000000000000000000", true),
        (
            "float",
            "03777777740000000000000000000000000000000000",
            true,
        ),
        (
            "float",
            "-03777777740000000000000000000000000000000000",
            true,
        ),
        ("float", "340282346638528859811704183484516925441", false),
        ("float", "-340282346638528859811704183484516925441", false),
        ("float", "0xffffff00000000000000000000000001", false),
        ("float", "-0xffffff00000000000000000000000001", false),
        (
            "float",
            "03777777740000000000000000000000000000000001",
            false,
        ),
        (
            "float",
            "-03777777740000000000000000000000000000000001",
            false,
        ),
        (
            "double",
            "179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858367",
            true,
        ),
        (
            "double",
            "-179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858367",
            true,
        ),
        (
            "double",
            "0xfffffffffffff7ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            true,
        ),
        (
            "double",
            "-0xfffffffffffff7ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            true,
        ),
        (
            "double",
            "0177777777777777777377777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777",
            true,
        ),
        (
            "double",
            "-0177777777777777777377777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777777",
            true,
        ),
        (
            "double",
            "179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858368",
            true,
        ),
        (
            "double",
            "-179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858368",
            true,
        ),
        (
            "double",
            "0xfffffffffffff800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            true,
        ),
        (
            "double",
            "-0xfffffffffffff800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            true,
        ),
        (
            "double",
            "0177777777777777777400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            true,
        ),
        (
            "double",
            "-0177777777777777777400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            true,
        ),
        (
            "double",
            "179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858369",
            false,
        ),
        (
            "double",
            "-179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858369",
            false,
        ),
        (
            "double",
            "0xfffffffffffff800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
            false,
        ),
        (
            "double",
            "-0xfffffffffffff800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
            false,
        ),
        (
            "double",
            "0177777777777777777400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
            false,
        ),
        (
            "double",
            "-0177777777777777777400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
            false,
        ),
    ] {
        let result = check(&[&constant(ty, raw)]);
        assert_eq!(result.is_ok(), valid, "{ty} {raw}: {result:?}");
        if let Err(e) = result {
            assert_eq!(e.kind, ErrorKind::ConstantValue);
        }
    }
}
