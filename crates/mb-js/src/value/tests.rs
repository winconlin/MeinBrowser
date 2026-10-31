use super::*;

#[test]
fn whole_numbers_become_int32() {
    for (number, int) in [
        (0.0, 0),
        (1.0, 1),
        (-1.0, -1),
        (2_147_483_647.0, i32::MAX),
        (-2_147_483_648.0, i32::MIN),
    ] {
        let value = Value::number(number);
        assert_eq!(value.as_int32(), Some(int), "{number}");
        assert_eq!(value.bits(), Value::int32(int).bits(), "{number}");
        assert_eq!(value.kind(), Kind::Number(number));
    }
}

#[test]
fn other_numbers_stay_doubles() {
    for number in [
        0.5,
        -0.25,
        2_147_483_648.0,
        -2_147_483_649.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN_POSITIVE,
        5e-324,
    ] {
        let value = Value::number(number);
        assert_eq!(value.as_int32(), None, "{number}");
        assert_eq!(value.bits(), number.to_bits(), "{number}");
        assert_eq!(value.as_number(), Some(number));
    }
}

#[test]
fn negative_zero_keeps_its_sign() {
    let value = Value::number(-0.0);
    assert_eq!(value.as_int32(), None);
    assert!(
        value
            .as_number()
            .is_some_and(|number| number.is_sign_negative())
    );
    assert_eq!(Value::number(0.0).as_int32(), Some(0));
}

#[test]
fn every_nan_gets_one_pattern() {
    for bits in [
        0x7FF8_0000_0000_0000,
        0x7FF0_0000_0000_0001,
        0x7FFF_FFFF_FFFF_FFFF,
        0xFFF0_0000_0000_0001,
        0xFFF8_0000_0000_0000,
        // Sähe ohne festes Muster aus wie Int32 5 oder wie undefined.
        0xFFF8_0000_0000_0005,
        0xFFF9_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFFF,
    ] {
        let value = Value::number(f64::from_bits(bits));
        assert_eq!(value.bits(), CANONICAL_NAN, "{bits:#x}");
        assert!(value.as_number().is_some_and(f64::is_nan), "{bits:#x}");
        assert_eq!(value.as_int32(), None);
    }
}

#[test]
fn special_values() {
    assert_eq!(Value::UNDEFINED.kind(), Kind::Undefined);
    assert_eq!(Value::NULL.kind(), Kind::Null);
    assert_eq!(Value::TRUE.kind(), Kind::Boolean(true));
    assert_eq!(Value::FALSE.kind(), Kind::Boolean(false));
    assert_eq!(Value::HOLE.kind(), Kind::Hole);
    assert_eq!(Value::boolean(true).bits(), Value::TRUE.bits());
    assert_eq!(Value::boolean(false).bits(), Value::FALSE.bits());
    assert!(Value::UNDEFINED.is_undefined() && !Value::NULL.is_undefined());
    assert!(Value::NULL.is_null() && Value::HOLE.is_hole());
    for value in [
        Value::UNDEFINED,
        Value::NULL,
        Value::TRUE,
        Value::FALSE,
        Value::HOLE,
    ] {
        assert_eq!(value.as_number(), None);
        assert_eq!(value.reference_parts(), None);
    }
}

#[test]
fn references_keep_kind_and_offset() {
    for kind in [
        RefKind::String,
        RefKind::Symbol,
        RefKind::BigInt,
        RefKind::Object,
        RefKind::Internal,
    ] {
        for offset in [0, 8, 0x0004_0000, u32::MAX] {
            let value = Value::reference(kind, offset);
            assert_eq!(value.reference_parts(), Some((kind, offset)));
            assert_eq!(value.as_number(), None);
            let reference = Ref::new(offset);
            let expected = match kind {
                RefKind::String => Kind::String(reference),
                RefKind::Symbol => Kind::Symbol(reference),
                RefKind::BigInt => Kind::BigInt(reference),
                RefKind::Object => Kind::Object(reference),
                RefKind::Internal => Kind::Internal(reference),
            };
            assert_eq!(value.kind(), expected);
        }
    }
}

#[test]
fn debug_shows_the_kind() {
    assert_eq!(format!("{:?}", Value::TRUE), "Value(Boolean(true))");
    assert_eq!(format!("{:?}", Value::number(1.5)), "Value(Number(1.5))");
}
