use super::*;

#[test]
fn number_palette_covers_signed_zero_exponent_boundaries_and_rounding_ties() {
    let mut commands = Vec::new();
    for index in 0..NUMBERS.len() as u8 {
        commands.extend([3, index, 0, 0]);
    }
    exercise(&commands);
}

#[test]
fn exact_dyadic_oracle_uses_plain_minimal_decimal_text() {
    for (numerator, exponent, text) in [
        (0, 8, "0"),
        (1, 8, "0.00390625"),
        (-1, 8, "-0.00390625"),
        (32767, 8, "127.99609375"),
        (-32768, 8, "-128"),
        (32767, 1, "16383.5"),
        (32767, 0, "32767"),
        (8, 3, "1"),
    ] {
        assert_eq!(dyadic(numerator, exponent).1, units(text));
    }
    for numerator in [i16::MIN, -32767, -1, 0, 1, i16::MAX] {
        let [a, b] = numerator.to_le_bytes();
        let mut commands = Vec::new();
        for exponent in 0..=8 {
            commands.extend([5, a, b, exponent]);
        }
        exercise(&commands);
    }
}

#[test]
fn key_text_is_not_coerced_as_a_number_or_trimmed() {
    for (text, expected) in [
        ("0", Some(0)),
        ("1", Some(1)),
        ("4294967294", Some(4_294_967_294)),
        ("4294967295", None),
        ("-0", None),
        ("+0", None),
        ("00", None),
        ("1.0", None),
        ("1e0", None),
        ("\u{feff}0", None),
        (" 0 ", None),
    ] {
        assert_eq!(index(&units(text)), expected, "{text:?}");
    }
    let mut commands = Vec::new();
    for index in 0..KEYS.len() as u8 {
        commands.extend([6, index, 0, 0]);
    }
    for style in 0..8 {
        commands.extend([9, 0, 0, style]);
        commands.extend([9, 255, 255, style]);
    }
    exercise(&commands);
}

#[test]
fn symbol_keys_keep_identity_and_string_errors_leave_no_pending_exception() {
    let mut commands = Vec::new();
    for kind in 0..4 {
        commands.extend([8, kind, 0, 0]);
        commands.extend([8, kind, 255, 255]);
        commands.extend([0, 0, 0, 0]);
        commands.extend([1, 0, 0, 0]);
        commands.extend([2, kind, 0, 0]);
    }
    exercise(&commands);
}

#[test]
fn utf16_text_preserves_lone_surrogates_nul_and_wrapping_units() {
    for (bytes, expected) in [
        ([7, 0xd8, 0, 1], vec![0xd800]),
        ([7, 0xdc, 0, 1], vec![0xdc00]),
        ([7, 0, 0, 0], vec![]),
        ([7, 0, 0, 1], vec![0]),
        ([7, 0xfe, 0xff, 2], vec![0xfeff, 0]),
        ([7, 0xff, 0xff, 2], vec![0xffff, 0x100]),
    ] {
        let Input::Text(text) = command(bytes) else {
            panic!("text command")
        };
        assert_eq!(text, expected);
        exercise(&bytes);
    }
}

#[test]
fn empty_partial_and_excessive_commands_have_fixed_bounds() {
    for stress in [false, true] {
        assert_eq!(run(&[], stress), 0);
        assert_eq!(run(&[8, 3, 0], stress), 0);
        assert_eq!(run(&[2, 1, 0, 0, 8, 3, 0], stress), 1);
        assert_eq!(run(&[0, 0, 0, 0].repeat(80), stress), MAX_COMMANDS);
    }
}
