use super::Primitive;

pub(super) fn integer(raw: &str, primitive: Primitive) -> bool {
    let negative = raw.starts_with('-');
    let unsigned = raw.strip_prefix('-').unwrap_or(raw);
    let (digits, base) = if unsigned.starts_with("0x") || unsigned.starts_with("0X") {
        (&unsigned[2..], 16)
    } else if unsigned.starts_with('0') {
        (unsigned, 8)
    } else {
        (unsigned, 10)
    };
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return true;
    }
    let bound = match primitive {
        Primitive::Byte => Some((true, 8)),
        Primitive::Octet => Some((false, 8)),
        Primitive::Short => Some((true, 16)),
        Primitive::UnsignedShort => Some((false, 16)),
        Primitive::Long => Some((true, 32)),
        Primitive::UnsignedLong => Some((false, 32)),
        Primitive::LongLong => Some((true, 64)),
        Primitive::UnsignedLongLong => Some((false, 64)),
        _ => None,
    };
    if let Some((signed, bits)) = bound {
        if negative && !signed {
            return false;
        }
        let limit = if signed {
            (1u64 << (bits - 1)) - u64::from(!negative)
        } else if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        return u64::from_str_radix(digits, base).is_ok_and(|n| n <= limit);
    }
    match primitive {
        Primitive::Float | Primitive::UnrestrictedFloat => {
            finite_integer(digits, base, 24, 128, MAX_FLOAT)
        }
        Primitive::Double | Primitive::UnrestrictedDouble => {
            finite_integer(digits, base, 53, 1024, MAX_DOUBLE)
        }
        _ => false,
    }
}

pub(super) fn decimal(raw: &str, primitive: Primitive) -> bool {
    match primitive {
        Primitive::Float => raw.parse::<f32>().is_ok_and(f32::is_finite),
        Primitive::UnrestrictedFloat => raw.parse::<f32>().is_ok(),
        Primitive::Double => raw.parse::<f64>().is_ok_and(f64::is_finite),
        Primitive::UnrestrictedDouble => raw.parse::<f64>().is_ok(),
        _ => false,
    }
}

// Compare exact integer values before rounding. Binary groups avoid allocating
// a BigInt for hex/octal literals; decimal uses bounded exact maxima.
fn finite_integer(digits: &str, base: u32, precision: usize, bits: usize, maximum: &str) -> bool {
    if base == 10 {
        return digits.len() < maximum.len()
            || (digits.len() == maximum.len() && digits <= maximum);
    }
    let group = if base == 16 { 4 } else { 3 };
    let first = digits.as_bytes()[0];
    let leading = digit(first).leading_zeros() as usize;
    let first_bits = 8 - leading;
    let length = digits
        .len()
        .saturating_sub(1)
        .saturating_mul(group)
        .saturating_add(first_bits);
    if length != bits {
        return length < bits;
    }
    let mut at = 0;
    for (n, byte) in digits.bytes().enumerate() {
        let count = if n == 0 { first_bits } else { group };
        for shift in (0..count).rev() {
            let actual = (digit(byte) >> shift) & 1;
            let expected = u8::from(at < precision);
            if actual != expected {
                return actual < expected;
            }
            at += 1;
        }
    }
    true
}
fn digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => byte - b'A' + 10,
    }
}

// (2^24 - 1) << 104, (2^53 - 1) << 971; exact positive finite IEEE maxima.
const MAX_FLOAT: &str = "340282346638528859811704183484516925440";
const MAX_DOUBLE: &str = concat!(
    "1797693134862315708145274237317043567980705675258449965989174768031572607800",
    "2853876058955863276687817154045895351438246423432132688946418276846754670353",
    "7516986049910576551282076245490090389328944075868508455133942304583236903222",
    "9481658085593321233482747978262041447231687381771809192998812504040261841248",
    "58368",
);
