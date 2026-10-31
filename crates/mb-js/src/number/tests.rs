use super::*;

fn same(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

#[test]
fn numbers_become_strings() {
    let cases = [
        (0.0, "0"),
        (-0.0, "0"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (1.0, "1"),
        (-1.0, "-1"),
        (100.0, "100"),
        (123.456, "123.456"),
        (0.5, "0.5"),
        (0.1, "0.1"),
        (0.1 + 0.2, "0.30000000000000004"),
        (1.0 / 3.0, "0.3333333333333333"),
        (2_147_483_648.0, "2147483648"),
        (9_007_199_254_740_992.0, "9007199254740992"),
        (9_007_199_254_740_994.0, "9007199254740994"),
        (1e20, "100000000000000000000"),
        (123_456_789_012_345_680_000.0, "123456789012345680000"),
        (1e21, "1e+21"),
        (1.23e22, "1.23e+22"),
        (-1.5e300, "-1.5e+300"),
        (f64::MAX, "1.7976931348623157e+308"),
        (0.000_001, "0.000001"),
        (0.000_001_2, "0.0000012"),
        (0.000_012_34, "0.00001234"),
        (1e-7, "1e-7"),
        (1.5e-7, "1.5e-7"),
        (5e-324, "5e-324"),
        (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
    ];
    for (number, expected) in cases {
        assert_eq!(to_string(number), expected, "{number:e}");
    }
}

#[test]
fn exponentiation_follows_the_special_cases() {
    let third = 1.0 / 3.0;
    let cases = [
        (f64::NAN, 0.0, 1.0),
        (f64::NAN, -0.0, 1.0),
        (1.0, f64::NAN, f64::NAN),
        (f64::NAN, 1.0, f64::NAN),
        (1.0, f64::INFINITY, f64::NAN),
        (-1.0, f64::NEG_INFINITY, f64::NAN),
        (2.0, f64::INFINITY, f64::INFINITY),
        (0.5, f64::INFINITY, 0.0),
        (2.0, f64::NEG_INFINITY, 0.0),
        (-0.5, f64::NEG_INFINITY, f64::INFINITY),
        (f64::INFINITY, -1.0, 0.0),
        (f64::NEG_INFINITY, 3.0, f64::NEG_INFINITY),
        (f64::NEG_INFINITY, 2.0, f64::INFINITY),
        (f64::NEG_INFINITY, -3.0, -0.0),
        (f64::NEG_INFINITY, -2.0, 0.0),
        (0.0, -1.0, f64::INFINITY),
        (-0.0, 3.0, -0.0),
        (-0.0, 2.0, 0.0),
        (-0.0, -3.0, f64::NEG_INFINITY),
        (-0.0, -2.0, f64::INFINITY),
        (-8.0, third, f64::NAN),
        (2.0, 10.0, 1024.0),
        (-2.0, 3.0, -8.0),
    ];
    for (base, exponent, expected) in cases {
        let found = exponentiate(base, exponent);
        assert!(
            same(found, expected),
            "{base} ** {exponent}: {found} statt {expected}"
        );
    }
}

#[test]
fn remainders_keep_the_sign_of_the_dividend() {
    let cases = [
        (-4.0, 2.0, -0.0),
        (4.0, -2.0, 0.0),
        (5.5, 2.0, 1.5),
        (-5.5, 2.0, -1.5),
        (5.0, f64::INFINITY, 5.0),
        (-0.0, 1.0, -0.0),
        (f64::INFINITY, 1.0, f64::NAN),
        (1.0, 0.0, f64::NAN),
        (f64::NAN, 1.0, f64::NAN),
        (1.0, f64::NAN, f64::NAN),
    ];
    for (numerator, denominator, expected) in cases {
        let found = remainder(numerator, denominator);
        assert!(
            same(found, expected),
            "{numerator} % {denominator}: {found} statt {expected}"
        );
    }
}

#[test]
fn shifts_use_five_bits_of_the_count() {
    assert!(same(left_shift(1.0, 31.0), -2_147_483_648.0));
    assert!(same(left_shift(1.0, 32.0), 1.0));
    assert!(same(left_shift(5.0, -1.0), -2_147_483_648.0));
    assert!(same(left_shift(4_294_967_297.0, 1.0), 2.0));
    assert!(same(signed_right_shift(-8.0, 1.0), -4.0));
    assert!(same(signed_right_shift(-1.0, 31.0), -1.0));
    assert!(same(signed_right_shift(f64::NAN, 1.0), 0.0));
    assert!(same(unsigned_right_shift(-1.0, 0.0), 4_294_967_295.0));
    assert!(same(unsigned_right_shift(-8.0, 28.0), 15.0));
    assert!(same(unsigned_right_shift(-8.0, 60.0), 15.0));
}

#[test]
fn bitwise_operators_work_on_32_bits() {
    assert!(same(bitwise(BitwiseOp::And, 61_680.0, 4_080.0), 240.0));
    assert!(same(bitwise(BitwiseOp::Or, -1.0, 0.0), -1.0));
    assert!(same(bitwise(BitwiseOp::Or, 1.9, 0.0), 1.0));
    assert!(same(bitwise(BitwiseOp::Xor, 4_294_967_301.0, 1.0), 4.0));
    assert!(same(bitwise(BitwiseOp::Xor, f64::INFINITY, 3.0), 3.0));
    assert!(same(bitwise_not(0.0), -1.0));
    assert!(same(bitwise_not(2_147_483_648.0), 2_147_483_647.0));
    assert!(same(bitwise_not(f64::NAN), -1.0));
}

#[test]
fn int32_reads_the_bits_as_twos_complement() {
    for (number, expected) in [
        (0.0, 0),
        (-0.0, 0),
        (-0.5, 0),
        (-1.5, -1),
        (2_147_483_647.0, i32::MAX),
        (2_147_483_648.0, i32::MIN),
        (-2_147_483_649.0, i32::MAX),
        (4_294_967_295.0, -1),
        (1e20, 1_661_992_960),
        (f64::NAN, 0),
        (f64::NEG_INFINITY, 0),
    ] {
        assert_eq!(to_int32(number), expected, "{number}");
    }
    assert_eq!(to_uint32(-1.0), u32::MAX);
}

#[test]
fn comparisons_treat_nan_and_zeros_like_the_specification() {
    assert_eq!(less_than(f64::NAN, 1.0), None);
    assert_eq!(less_than(1.0, f64::NAN), None);
    assert_eq!(less_than(-0.0, 0.0), Some(false));
    assert_eq!(less_than(f64::NEG_INFINITY, f64::MIN), Some(true));
    assert_eq!(less_than(2.0, 1.0), Some(false));
    assert!(equal(-0.0, 0.0));
    assert!(!equal(f64::NAN, f64::NAN));
    assert!(same(unary_minus(0.0), -0.0));
    assert!(unary_minus(f64::NAN).is_nan());
}
