use super::canonical_numeric_index;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn canonical_numeric_strings() {
    let index = |text: &str| canonical_numeric_index(&units(text));
    assert_eq!(index("0"), Some(0.0));
    assert_eq!(index("1.5"), Some(1.5));
    assert_eq!(index("-1"), Some(-1.0));
    assert_eq!(index("Infinity"), Some(f64::INFINITY));
    assert_eq!(index("-Infinity"), Some(f64::NEG_INFINITY));
    assert!(index("NaN").is_some_and(f64::is_nan));
    assert_eq!(index("1e+21"), Some(1e21));
    let negative_zero = index("-0").expect("-0 ist kanonisch");
    assert!(negative_zero == 0.0 && negative_zero.is_sign_negative());
    // Nicht kanonisch: was ToString(ToNumber(s)) nicht wieder ergibt.
    for text in [
        "", "01", "1.0", "+1", " 1", "1e21", "0x10", "-0.0", "foo", "infinity",
    ] {
        assert_eq!(index(text), None, "{text:?}");
    }
}
