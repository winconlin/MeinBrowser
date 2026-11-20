use super::*;

/// Ein Budget ohne Grenzen.
fn free<R>(f: impl FnOnce(&mut Budget<'_>) -> R) -> R {
    let mut spend = |_| true;
    let mut budget = Budget {
        bytes: usize::MAX,
        spend: &mut spend,
    };
    f(&mut budget)
}

fn big(value: i128) -> BigInt {
    let magnitude = value.unsigned_abs();
    BigInt::from_parts(value < 0, vec![low(magnitude), high(magnitude)]).expect("klein")
}

fn small(value: &BigInt) -> Option<i128> {
    let magnitude = match value.magnitude() {
        [] => 0u128,
        [word] => u128::from(*word),
        [word, upper] => u128::from(*word) | (u128::from(*upper) << 64),
        _ => return None,
    };
    let magnitude = i128::try_from(magnitude).ok()?;
    Some(if value.is_negative() {
        -magnitude
    } else {
        magnitude
    })
}

fn parse(text: &str) -> BigInt {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let value = free(|budget| BigInt::from_digits(10, digits.as_bytes(), budget)).expect("Ziffern");
    if negative { value.negate() } else { value }
}

fn show(value: &BigInt, radix: u32) -> String {
    let text = free(|budget| value.to_radix_string(radix, budget)).expect("Text");
    String::from_utf8(text).expect("ASCII")
}

/// Zufallszahlen nach xorshift64, für jeden Lauf gleich.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Ein Wert bis knapp 2^62 in beide Richtungen, mit vielen kleinen
    /// und Grenzwerten.
    fn value(&mut self) -> i128 {
        let raw = self.next();
        let bits = raw % 63;
        let magnitude = i128::from(self.next() >> (64 - bits.max(1)));
        if raw & (1 << 40) == 0 {
            magnitude
        } else {
            -magnitude
        }
    }

    /// Ein Betrag mit `words` zufälligen Wörtern.
    fn words(&mut self, words: usize) -> Vec<u64> {
        (0..words).map(|_| self.next()).collect()
    }
}

#[test]
fn representation_is_canonical() {
    assert_eq!(BigInt::from_i64(0), BigInt::zero());
    assert_eq!(BigInt::from_parts(true, vec![0, 0]), Ok(BigInt::zero()));
    assert!(!BigInt::zero().negate().is_negative());
    assert_eq!(
        BigInt::from_parts(false, vec![5, 0, 0]),
        Ok(BigInt::from_u64(5))
    );
    assert_eq!(big(-7).bit_length(), 3);
    assert_eq!(big(1 << 64).bit_length(), 65);
}

#[test]
fn arithmetic_agrees_with_i128() {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    for _ in 0..20_000 {
        let (x, y) = (random.value(), random.value());
        let (a, b) = (big(x), big(y));
        free(|budget| {
            assert_eq!(
                small(&a.add(&b, budget).expect("Summe")),
                Some(x + y),
                "{x} + {y}"
            );
            assert_eq!(
                small(&a.subtract(&b, budget).expect("Differenz")),
                Some(x - y)
            );
            assert_eq!(
                small(&a.multiply(&b, budget).expect("Produkt")),
                Some(x * y)
            );
            match a.divide(&b, budget).expect("Division") {
                None => assert_eq!(y, 0),
                Some((quotient, remainder)) => {
                    // i128 rundet wie BigInt::divide zur Null hin, der Rest
                    // hat das Vorzeichen des Dividenden.
                    assert_eq!(small(&quotient), Some(x / y), "{x} / {y}");
                    assert_eq!(small(&remainder), Some(x % y), "{x} % {y}");
                }
            }
            for (op, expected) in [
                (Bitwise::And, x & y),
                (Bitwise::Xor, x ^ y),
                (Bitwise::Or, x | y),
            ] {
                let result = a.bitwise(op, &b, budget).expect("bitweise");
                assert_eq!(small(&result), Some(expected), "{x} {op:?} {y}");
            }
            let shift = i128::from(random.next() % 64);
            let left = a.shift_left(&big(shift), budget).expect("links");
            assert_eq!(small(&left), Some(x << shift), "{x} << {shift}");
            let right = a.shift_left(&big(-shift), budget).expect("rechts");
            assert_eq!(small(&right), Some(x >> shift), "{x} >> {shift}");
        });
        assert_eq!(
            small(&free(|budget| a.not(budget)).expect("nicht")),
            Some(!x)
        );
        assert_eq!(free(|budget| a.negate_within(budget)), Ok(a.negate()));
        assert_eq!(a.cmp(&b), x.cmp(&y));
    }
}

#[test]
fn division_of_long_values_restores_the_dividend() {
    let mut random = Random(0x2545_F491_4F6C_DD1D);
    for round in 0..400 {
        let length = 2 + round % 9;
        let divisor_length = 1 + round % length;
        let mut dividend = random.words(length);
        let mut divisor = random.words(divisor_length);
        // Grenzfälle für die Schätzung in D3: oberste Wörter gleich oder
        // ganz gefüllt.
        if round % 5 == 0
            && let (Some(&top), Some(slot)) = (divisor.last(), dividend.last_mut())
        {
            *slot = top;
        }
        if round % 7 == 0 {
            divisor.fill(u64::MAX);
        }
        if round % 11 == 0 && divisor.len() > 1 {
            divisor[0] = 0;
        }
        let a = BigInt::from_parts(false, dividend.clone()).expect("klein");
        let b = BigInt::from_parts(false, divisor).expect("klein");
        if b.is_zero() {
            continue;
        }
        free(|budget| {
            let (quotient, remainder) = a.divide(&b, budget).expect("Division").expect("Divisor");
            assert!(remainder < b, "Rest kleiner als der Divisor");
            let back = quotient
                .multiply(&b, budget)
                .and_then(|product| product.add(&remainder, budget))
                .expect("Probe");
            assert_eq!(back, a);
        });
    }
}

#[test]
fn decimal_and_radix_text_round_trips() {
    let samples = [
        "0",
        "1",
        "-1",
        "18446744073709551615",
        "18446744073709551616",
        "-340282366920938463463374607431768211456",
        "123456789012345678901234567890123456789012345678901234567890",
    ];
    for text in samples {
        let value = parse(text);
        assert_eq!(show(&value, 10), text);
        for radix in [2, 7, 16, 36] {
            let digits = show(&value, radix);
            let (negative, rest) = match digits.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, digits.as_str()),
            };
            let back = free(|budget| BigInt::from_digits(radix, rest.as_bytes(), budget))
                .expect("Ziffern");
            let back = if negative { back.negate() } else { back };
            assert_eq!(back, value, "{text} zur Basis {radix}");
        }
    }
    assert_eq!(show(&big(255), 16), "ff");
    assert_eq!(show(&big(-35), 36), "-z");
    assert_eq!(show(&parse("000123"), 10), "123");
}

#[test]
fn conversion_to_numbers_rounds_to_nearest_even() {
    assert_eq!(parse("9007199254740993").to_f64(), 9_007_199_254_740_992.0);
    assert_eq!(parse("9007199254740995").to_f64(), 9_007_199_254_740_996.0);
    // Knapp über der Hälfte rundet auf, auch weit unten im Betrag.
    let above = parse("9007199254740993").shift_left(
        &big(80),
        &mut Budget {
            bytes: usize::MAX,
            spend: &mut |_| true,
        },
    );
    let above = above.expect("Wert");
    let one = BigInt::from_u64(1);
    let above = free(|budget| above.add(&one, budget)).expect("Summe");
    assert_eq!(above.to_f64(), 9_007_199_254_740_994.0 * 2f64.powi(80));
    assert_eq!(big(-(1 << 70)).to_f64(), -(2f64.powi(70)));
    let huge = free(|budget| BigInt::from_u64(1).shift_left(&big(1024), budget)).expect("Wert");
    assert_eq!(huge.to_f64(), f64::INFINITY);
    let largest = free(|budget| {
        let below = BigInt::from_u64(1).shift_left(&big(1024), budget)?;
        let gap = BigInt::from_u64(1).shift_left(&big(971), budget)?;
        below.subtract(&gap, budget)
    })
    .expect("Wert");
    assert_eq!(largest.to_f64(), f64::MAX);
}

#[test]
fn integral_numbers_convert_exactly() {
    for number in [
        0.0,
        -0.0,
        1.0,
        -1.0,
        2f64.powi(53),
        -(2f64.powi(80)),
        f64::MAX,
    ] {
        let value = BigInt::from_f64(number).expect("ganz");
        assert_eq!(value.to_f64(), number, "{number}");
    }
    assert!(!BigInt::from_f64(-0.0).expect("ganz").is_negative());
    assert_eq!(BigInt::from_f64(0.5), None);
    assert_eq!(BigInt::from_f64(f64::NAN), None);
    assert_eq!(BigInt::from_f64(f64::INFINITY), None);
    assert_eq!(
        show(&BigInt::from_f64(1e21).expect("ganz"), 10),
        "1000000000000000000000"
    );
}

#[test]
fn comparison_with_numbers_is_exact() {
    let value = parse("9007199254740993");
    assert_eq!(
        value.compare_number(9_007_199_254_740_992.0),
        Some(Ordering::Greater)
    );
    assert_eq!(
        value.compare_number(9_007_199_254_740_994.0),
        Some(Ordering::Less)
    );
    assert_eq!(big(1).compare_number(1.0), Some(Ordering::Equal));
    assert_eq!(big(1).compare_number(1.5), Some(Ordering::Less));
    assert_eq!(big(2).compare_number(1.5), Some(Ordering::Greater));
    assert_eq!(big(-2).compare_number(-1.5), Some(Ordering::Less));
    assert_eq!(big(-1).compare_number(-1.5), Some(Ordering::Greater));
    assert_eq!(big(0).compare_number(-0.0), Some(Ordering::Equal));
    assert_eq!(big(0).compare_number(f64::NAN), None);
    assert_eq!(
        big(i128::MAX).compare_number(f64::INFINITY),
        Some(Ordering::Less)
    );
    assert_eq!(
        big(i128::MIN).compare_number(f64::NEG_INFINITY),
        Some(Ordering::Greater)
    );
}

#[test]
fn as_int_n_and_as_uint_n_wrap_like_fixed_widths() {
    let mut random = Random(0xD1B5_4A32_D192_ED03);
    for _ in 0..5_000 {
        let x = random.value();
        let bits = random.next() % 70;
        free(|budget| {
            let unsigned = big(x).as_uint_n(bits, budget).expect("asUintN");
            let modulus = 1i128 << bits;
            assert_eq!(
                small(&unsigned),
                Some(x.rem_euclid(modulus)),
                "asUintN({bits}, {x})"
            );
            let signed = big(x).as_int_n(bits, budget).expect("asIntN");
            let expected = if bits == 0 {
                0
            } else {
                let wrapped = x.rem_euclid(modulus);
                if wrapped >= modulus / 2 {
                    wrapped - modulus
                } else {
                    wrapped
                }
            };
            assert_eq!(small(&signed), Some(expected), "asIntN({bits}, {x})");
        });
    }
    free(|budget| {
        // Sehr große Breiten lassen kleine Werte, wie sie sind.
        let huge = u64::MAX >> 11;
        assert_eq!(big(-5).as_int_n(huge, budget), Ok(big(-5)));
        assert_eq!(big(5).as_uint_n(huge, budget), Ok(big(5)));
        assert_eq!(big(-5).as_uint_n(huge, budget), Err(Failure::TooLarge));
        assert_eq!(big(-(1 << 63)).as_int_n(64, budget), Ok(big(-(1 << 63))));
        assert_eq!(big(1 << 63).as_int_n(64, budget), Ok(big(-(1 << 63))));
    });
}

#[test]
fn exponentiation_and_limits() {
    free(|budget| {
        assert_eq!(big(3).exponentiate(&big(5), budget), Ok(big(243)));
        assert_eq!(big(-2).exponentiate(&big(63), budget), Ok(big(-(1 << 63))));
        assert_eq!(big(0).exponentiate(&big(0), budget), Ok(big(1)));
        assert_eq!(big(-1).exponentiate(&big(i128::MAX), budget), Ok(big(-1)));
        assert_eq!(
            big(2).exponentiate(&big(i128::MAX), budget),
            Err(Failure::TooLarge)
        );
        let power = big(10).exponentiate(&big(40), budget).expect("Potenz");
        assert_eq!(show(&power, 10), format!("1{}", "0".repeat(40)));
        let edge = BigInt::from_u64(1).shift_left(&big(i128::from(MAX_BITS) - 1), budget);
        assert!(edge.is_ok(), "genau MAX_BITS Bit");
        let over = BigInt::from_u64(1).shift_left(&big(i128::from(MAX_BITS)), budget);
        assert_eq!(over, Err(Failure::TooLarge));
        // Nach rechts ergeben große Weiten 0 oder -1.
        assert_eq!(big(-5).shift_left(&big(-(1 << 100)), budget), Ok(big(-1)));
        assert_eq!(big(5).shift_left(&big(-(1 << 100)), budget), Ok(big(0)));
        assert_eq!(big(0).shift_left(&big(1 << 100), budget), Ok(big(0)));
    });
}

#[test]
fn budgets_stop_before_the_work() {
    let large = BigInt::from_parts(false, vec![u64::MAX; 64]).expect("Wert");
    let mut spent = 0;
    let mut spend = |work| {
        spent += work;
        false
    };
    let mut budget = Budget {
        bytes: usize::MAX,
        spend: &mut spend,
    };
    assert_eq!(large.multiply(&large, &mut budget), Err(Failure::Stopped));
    assert_eq!(spent, 64 * 64 + 128 * COPY_WORK);
    let mut spend = |_| true;
    let mut budget = Budget {
        bytes: 100,
        spend: &mut spend,
    };
    assert_eq!(
        large.multiply(&large, &mut budget),
        Err(Failure::OutOfMemory)
    );
}
