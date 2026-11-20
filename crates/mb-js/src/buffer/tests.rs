use super::*;

#[test]
fn binary16_rounds_once_to_the_nearest_even() {
    for (value, bits) in [
        (0.0, 0x0000),
        (-0.0, 0x8000),
        (1.0, 0x3C00),
        (-2.0, 0xC000),
        (0.5, 0x3800),
        (65504.0, 0x7BFF),
        // Genau zwischen 65504 und 65536: gerade ist 65536, also ∞.
        (65520.0, 0x7C00),
        (65519.99, 0x7BFF),
        (1e10, 0x7C00),
        (-1e10, 0xFC00),
        (f64::INFINITY, 0x7C00),
        (f64::NEG_INFINITY, 0xFC00),
        (f64::NAN, 0x7E00),
        // Die kleinste normale und die subnormalen Zahlen.
        (2f64.powi(-14), 0x0400),
        (2f64.powi(-24), 0x0001),
        (-2f64.powi(-24), 0x8001),
        (1023.0 * 2f64.powi(-24), 0x03FF),
        // Genau die Hälfte der kleinsten: gerade ist 0.
        (2f64.powi(-25), 0x0000),
        (3.0 * 2f64.powi(-25), 0x0002),
        (0.75 * 2f64.powi(-24), 0x0001),
        (2f64.powi(-26), 0x0000),
        (f64::MIN_POSITIVE, 0x0000),
        (5e-324, 0x0000),
        // Zwischen der größten subnormalen und der kleinsten normalen.
        (1023.5 * 2f64.powi(-24), 0x0400),
        // Gleichstände zwischen normalen Zahlen.
        (1.0 + 2f64.powi(-11), 0x3C00),
        (1.0 + 3.0 * 2f64.powi(-11), 0x3C02),
        // Über binary32 würde das doppelt gerundet: 2⁻⁴⁰ ginge verloren,
        // und der Gleichstand fiele auf die gerade 0x3C00.
        (1.0 + 2f64.powi(-11) + 2f64.powi(-40), 0x3C01),
        // Ein Übertrag beim Runden erhöht den Exponenten.
        (2.0 - 2f64.powi(-12), 0x4000),
    ] {
        assert_eq!(f16_bits(value), bits, "{value:e}");
    }
}

#[test]
fn binary16_values_are_exact_and_round_trip() {
    assert_eq!(f16_value(0x3C00), 1.0);
    assert_eq!(f16_value(0x0001), 2f64.powi(-24));
    assert_eq!(f16_value(0x7BFF), 65504.0);
    assert_eq!(f16_value(0xFC00), f64::NEG_INFINITY);
    assert!(f16_value(0x7C01).is_nan());
    assert_eq!(f16_value(0x8000).to_bits(), (-0.0f64).to_bits());
    for bits in 0..=u16::MAX {
        let value = f16_value(bits);
        if value.is_nan() {
            continue;
        }
        assert_eq!(f16_bits(value), bits, "{bits:#06x}");
    }
}

#[test]
fn numbers_become_bytes_modulo_their_width() {
    let cases: [(ElementType, f64, &[u8]); 12] = [
        (ElementType::Int8, -1.0, &[0xFF]),
        (ElementType::Int8, 128.0, &[0x80]),
        (ElementType::Uint8, 256.0, &[0x00]),
        (ElementType::Uint8, -1.5, &[0xFF]),
        (ElementType::Uint8, f64::NAN, &[0x00]),
        (ElementType::Int16, -2.0, &[0xFE, 0xFF]),
        (ElementType::Uint16, 65537.0, &[0x01, 0x00]),
        (ElementType::Int32, 4_294_967_301.0, &[0x05, 0, 0, 0]),
        (ElementType::Uint32, -1.0, &[0xFF, 0xFF, 0xFF, 0xFF]),
        (ElementType::Uint32, f64::INFINITY, &[0, 0, 0, 0]),
        (ElementType::Float32, 1.5, &[0, 0, 0xC0, 0x3F]),
        (ElementType::Float64, -0.0, &[0, 0, 0, 0, 0, 0, 0, 0x80]),
    ];
    for (kind, value, expected) in cases {
        let little = number_to_raw_bytes(kind, value, true);
        assert_eq!(&little[..kind.size()], expected, "{kind:?} {value}");
        let big = number_to_raw_bytes(kind, value, false);
        let mut reversed = expected.to_vec();
        reversed.reverse();
        assert_eq!(&big[..kind.size()], reversed.as_slice(), "{kind:?} {value}");
    }
}

#[test]
fn float32_rounds_to_the_nearest_even() {
    // 1 + 2⁻²⁴ liegt genau zwischen 1 und 1 + 2⁻²³.
    let tie = number_to_raw_bytes(ElementType::Float32, 1.0 + 2f64.powi(-24), true);
    assert_eq!(&tie[..4], &1.0f32.to_le_bytes());
    let above = number_to_raw_bytes(
        ElementType::Float32,
        1.0 + 2f64.powi(-24) + 2f64.powi(-50),
        true,
    );
    assert_eq!(&above[..4], &(1.0f32 + f32::EPSILON).to_le_bytes());
}

#[test]
fn bytes_become_numbers() {
    let read = |kind, bytes: &[u8], little| raw_bytes_to_numeric(kind, bytes, little);
    assert_eq!(
        read(ElementType::Int8, &[0x80], true),
        Numeric::Number(-128.0)
    );
    assert_eq!(
        read(ElementType::Uint8, &[0x80], true),
        Numeric::Number(128.0)
    );
    assert_eq!(
        read(ElementType::Int16, &[0xFF, 0xFE], false),
        Numeric::Number(-2.0)
    );
    assert_eq!(
        read(ElementType::Uint16, &[0xFF, 0xFE], true),
        Numeric::Number(65279.0)
    );
    assert_eq!(
        read(ElementType::Int32, &[0, 0, 0, 0x80], true),
        Numeric::Number(-2_147_483_648.0)
    );
    assert_eq!(
        read(ElementType::Uint32, &[0x80, 0, 0, 0], false),
        Numeric::Number(2_147_483_648.0)
    );
    assert_eq!(
        read(ElementType::Float16, &[0x3C, 0x00], false),
        Numeric::Number(1.0)
    );
    assert_eq!(
        read(ElementType::BigInt64, &[0xFF; 8], true),
        Numeric::BigInt64(-1)
    );
    assert_eq!(
        read(ElementType::BigUint64, &[0, 0, 0, 0, 0, 0, 0, 1], false),
        Numeric::BigUint64(1)
    );
    let Numeric::Number(nan) = read(ElementType::Float64, &f64::NAN.to_le_bytes(), true) else {
        panic!("Number erwartet");
    };
    assert!(nan.is_nan());
}

#[test]
fn bigints_become_their_low_64_bits() {
    assert_eq!(bigint_to_raw_bytes(true, 1, true), [0xFF; 8]);
    assert_eq!(
        bigint_to_raw_bytes(false, 1, false),
        [0, 0, 0, 0, 0, 0, 0, 1]
    );
    assert_eq!(
        bigint_to_raw_bytes(true, 1 << 63, true),
        [0, 0, 0, 0, 0, 0, 0, 0x80]
    );
    assert_eq!(bigint_to_raw_bytes(true, 0, true), [0; 8]);
}
