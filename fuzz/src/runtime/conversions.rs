//! Primitive conversion checks with generated, exactly known numeric strings.
//!
//! Four-byte commands `[kind, a, b, c]`, kind modulo twelve: IEEE bit stream,
//! numeric palette, signed 24-bit integer, undefined, null, boolean, symbol,
//! decimal string, radix string, invalid string, whitespace/special string,
//! or an exactly representable binary fraction written in decimal.

use mb_js::agent::{Abrupt, Agent, Completion, ErrorKind, Exception};
use mb_js::conversion;
use mb_js::heap::{Local, Options, Scope};
use mb_js::value::Value;

const MAX_COMMANDS: usize = 64;
const MAX_STRING_UNITS: usize = 32;
const SPACE: [u16; 5] = [0x0009, 0x00a0, 0xfeff, 0x2028, 0x2029];
const NUMBERS: [f64; 25] = [
    0.0,
    -0.0,
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::from_bits(1),
    -f64::from_bits(1),
    f64::MAX,
    -f64::MAX,
    2_147_483_647.0,
    2_147_483_648.0,
    -2_147_483_649.0,
    4_294_967_295.0,
    4_294_967_296.0,
    4_294_967_297.0,
    -4_294_967_297.0,
    9_007_199_254_740_991.0,
    9_007_199_254_740_992.0,
    9_007_199_254_740_994.0,
    0.5,
    -0.5,
    1.75,
    -1.75,
    f64::MIN_POSITIVE,
    -f64::MIN_POSITIVE,
];
const INVALID: [&str; 18] = [
    "NaN", "inf", "infinity", "+0x1", "-0b1", "0b2", "0o8", "0xg", "0x", "1_0", "1n", "1e", "1e+",
    "1 2", ".", "+", "--1", "1\0",
];
const SPECIAL: [(&str, f64); 10] = [
    ("", 0.0),
    (" \t\r\n", 0.0),
    ("0", 0.0),
    ("-0", -0.0),
    ("+0", 0.0),
    ("Infinity", f64::INFINITY),
    ("+Infinity", f64::INFINITY),
    ("-Infinity", f64::NEG_INFINITY),
    ("00012", 12.0),
    ("-000", -0.0),
];

#[derive(Clone, Debug)]
enum Input {
    Number(f64),
    Undefined,
    Null,
    Boolean(bool),
    Symbol,
    String(Vec<u16>, f64),
}

impl Input {
    fn number(&self) -> Option<f64> {
        Some(match self {
            Self::Number(number) | Self::String(_, number) => *number,
            Self::Undefined => f64::NAN,
            Self::Null | Self::Boolean(false) => 0.0,
            Self::Boolean(true) => 1.0,
            Self::Symbol => return None,
        })
    }

    fn boolean(&self) -> bool {
        match self {
            Self::Number(number) => *number != 0.0 && !number.is_nan(),
            Self::Undefined | Self::Null => false,
            Self::Boolean(value) => *value,
            Self::Symbol => true,
            Self::String(units, _) => !units.is_empty(),
        }
    }

    fn local<'s>(&self, agent: &mut Agent, scope: &'s Scope) -> Local<'s> {
        match self {
            Self::Number(number) => agent.heap().local(scope, Value::number(*number)),
            Self::Undefined => agent.heap().local(scope, Value::UNDEFINED),
            Self::Null => agent.heap().local(scope, Value::NULL),
            Self::Boolean(value) => agent.heap().local(scope, Value::boolean(*value)),
            Self::Symbol => agent.new_symbol(scope, None).expect("bounded symbol"),
            Self::String(units, _) => {
                assert!(units.len() <= MAX_STRING_UNITS, "generated string bound");
                agent
                    .heap_mut()
                    .alloc_string(scope, units)
                    .expect("bounded string")
            }
        }
    }
}

fn text(text: &str, number: f64) -> Input {
    Input::String(text.encode_utf16().collect(), number)
}

fn padded(input: Input, selector: u8) -> Input {
    let Input::String(units, number) = input else {
        return input;
    };
    let mut padded = Vec::new();
    if selector & 0x40 != 0 {
        padded.extend(SPACE);
    }
    padded.extend(units);
    if selector & 0x80 != 0 {
        padded.extend(SPACE.into_iter().rev());
    }
    Input::String(padded, number)
}

fn command([kind, a, b, c]: [u8; 4], state: &mut u64) -> Input {
    match kind % 12 {
        0 => {
            // The 64-bit state permits every IEEE bit position to vary over
            // a command stream, without restoring boxed reference values.
            *state = state.rotate_left(17)
                ^ u64::from(u32::from_le_bytes([a, b, c, kind]))
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15);
            Input::Number(f64::from_bits(*state))
        }
        1 => Input::Number(NUMBERS[usize::from(a) % NUMBERS.len()]),
        2 => {
            let signed = i32::from_le_bytes([a, b, c, if c & 0x80 == 0 { 0 } else { 0xff }]);
            Input::Number(f64::from(signed))
        }
        3 => Input::Undefined,
        4 => Input::Null,
        5 => Input::Boolean(a & 1 != 0),
        6 => Input::Symbol,
        7 => {
            let value = i32::from(i16::from_le_bytes([a, b]));
            let magnitude = value.unsigned_abs();
            let (decimal, number) = match c % 6 {
                0 => (value.to_string(), f64::from(value)),
                1 => (format!("+{magnitude}"), f64::from(magnitude)),
                2 => (format!("-{magnitude}"), -f64::from(magnitude)),
                3 => (format!("{value:06}"), f64::from(value)),
                4 => (format!("{value}e+0"), f64::from(value)),
                _ => (format!("{value}.0"), f64::from(value)),
            };
            padded(text(&decimal, number), kind)
        }
        8 => {
            let value = u16::from_le_bytes([a, b]);
            let radix = match c % 3 {
                0 => format!("0b{value:b}"),
                1 => format!("0o{value:o}"),
                _ => format!("0x{value:x}"),
            };
            padded(text(&radix, f64::from(value)), kind)
        }
        9 => {
            let invalid = match a % 3 {
                0 => text(INVALID[usize::from(b) % INVALID.len()], f64::NAN),
                1 => Input::String(vec![0xd800 | u16::from(b), u16::from(c)], f64::NAN),
                _ => Input::String(vec![0x180e, u16::from(b'1')], f64::NAN),
            };
            padded(invalid, kind)
        }
        10 => {
            let (special, number) = SPECIAL[usize::from(a) % SPECIAL.len()];
            padded(text(special, number), kind)
        }
        _ => {
            let value = i32::from(i16::from_le_bytes([a, b]));
            let exponent = u32::from(c % 7);
            let denominator = 1_u32 << exponent;
            let scale = 10_u32.pow(exponent);
            let scaled = value.unsigned_abs() * 5_u32.pow(exponent);
            let decimal = if exponent == 0 {
                value.to_string()
            } else {
                let negative = if value < 0 { "-" } else { "" };
                format!(
                    "{negative}{}.{:0width$}",
                    scaled / scale,
                    scaled % scale,
                    width = exponent as usize
                )
            };
            padded(
                text(&decimal, f64::from(value) / f64::from(denominator)),
                kind,
            )
        }
    }
}

/// Integer truncation modulo 2^32 from the IEEE-754 representation. This
/// uses neither the runtime's floating remainder nor its conversion helpers.
fn uint32(number: f64) -> u32 {
    let bits = number.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    if biased == 0 || biased == 0x7ff {
        return 0;
    }
    let exponent = biased - 1023;
    if exponent < 0 {
        return 0;
    }
    let significand = (1_u64 << 52) | (bits & ((1_u64 << 52) - 1));
    let magnitude = if exponent < 52 {
        (significand >> (52 - exponent)) as u32
    } else if exponent < 84 {
        (significand as u32).wrapping_shl((exponent - 52) as u32)
    } else {
        0
    };
    if bits >> 63 != 0 {
        magnitude.wrapping_neg()
    } else {
        magnitude
    }
}

fn integer(number: f64) -> f64 {
    if number.is_nan() || number.abs() < 1.0 || number == 0.0 {
        0.0
    } else {
        number.trunc()
    }
}

fn same_number(actual: f64, expected: f64) {
    if expected.is_nan() {
        assert!(actual.is_nan(), "expected NaN, got {actual}");
    } else {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "number including signed zero"
        );
    }
}

fn rejected<T: std::fmt::Debug>(agent: &mut Agent, scope: &Scope, result: Completion<T>) {
    assert!(
        matches!(result, Err(Abrupt::Throw)),
        "expected TypeError, got {result:?}"
    );
    assert!(matches!(
        agent.take_exception(scope),
        Some(Exception::Error(ErrorKind::TypeError))
    ));
    assert!(
        agent.take_exception(scope).is_none(),
        "exception consumed once"
    );
}

fn check(agent: &mut Agent, scope: &Scope, input: &Input) {
    let local = input.local(agent, scope);
    // The input handle must remain valid while the collector relocates it.
    agent.heap_mut().collect();
    assert_eq!(
        conversion::to_boolean(agent.heap(), agent.heap().get(local)),
        input.boolean()
    );
    if let Some(number) = input.number() {
        if let Input::String(units, _) = input {
            same_number(conversion::string_to_number(units), number);
        }
        same_number(
            conversion::to_number(agent, scope, local).expect("primitive ToNumber"),
            number,
        );
        same_number(
            conversion::to_numeric(agent, scope, local).expect("primitive ToNumeric"),
            number,
        );
        same_number(
            conversion::to_integer_or_infinity(agent, scope, local)
                .expect("primitive ToIntegerOrInfinity"),
            integer(number),
        );
        let unsigned = uint32(number);
        assert_eq!(conversion::to_uint32(agent, scope, local), Ok(unsigned));
        assert_eq!(
            conversion::to_int32(agent, scope, local),
            Ok(unsigned as i32)
        );
        assert!(
            agent.take_exception(scope).is_none(),
            "no pending exception"
        );
    } else {
        let result = conversion::to_number(agent, scope, local);
        rejected(agent, scope, result);
        let result = conversion::to_numeric(agent, scope, local);
        rejected(agent, scope, result);
        let result = conversion::to_integer_or_infinity(agent, scope, local);
        rejected(agent, scope, result);
        let result = conversion::to_uint32(agent, scope, local);
        rejected(agent, scope, result);
        let result = conversion::to_int32(agent, scope, local);
        rejected(agent, scope, result);
    }
}

fn run(data: &[u8], stress: bool) {
    let mut agent = Agent::with_options(Options {
        limit: 1 << 20,
        stress,
        verify: true,
    })
    .expect("bounded conversion agent");
    let mut state = 0x0010_0000_0000_0001;
    for &bytes in data.as_chunks::<4>().0.iter().take(MAX_COMMANDS) {
        let input = command(bytes, &mut state);
        agent.scope(|agent, scope| check(agent, scope, &input));
        agent.heap_mut().collect();
        agent.heap().verify().expect("valid conversion heap");
    }
}

pub(super) fn exercise(data: &[u8]) {
    run(data, false);
    run(data, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ieee_oracle_handles_signed_zero_fractions_and_integer_boundaries() {
        for (number, expected) in [
            (-0.0, 0),
            (f64::NAN, 0),
            (f64::INFINITY, 0),
            (f64::from_bits(1), 0),
            (-0.75, 0),
            (1.75, 1),
            (-1.75, u32::MAX),
            (2_147_483_648.0, 0x8000_0000),
            (4_294_967_295.0, u32::MAX),
            (4_294_967_296.0, 0),
            (4_294_967_297.0, 1),
            (-4_294_967_297.0, u32::MAX),
            (9_007_199_254_740_991.0, u32::MAX),
            (9_007_199_254_740_992.0, 0),
            (9_007_199_254_740_994.0, 2),
            (f64::MAX, 0),
        ] {
            assert_eq!(uint32(number), expected, "{number}");
        }
        for value in [0.0, -0.0, f64::NAN, 0.75, -0.75] {
            same_number(integer(value), 0.0);
        }
        same_number(integer(f64::NEG_INFINITY), f64::NEG_INFINITY);
    }

    #[test]
    fn all_numeric_kinds_and_symbol_exceptions_reach_the_runtime() {
        let mut bytes = Vec::new();
        for index in 0..NUMBERS.len() as u8 {
            bytes.extend([1, index, 0, 0]);
        }
        for kind in 0..7 {
            bytes.extend([kind, 255, 0, 255]);
        }
        exercise(&bytes);
    }

    #[test]
    fn decimal_radix_and_exact_fraction_strings_have_independent_expected_values() {
        let mut bytes = Vec::new();
        for style in 0..6 {
            bytes.extend([7, 1, 128, style]);
            bytes.extend([7, 0, 0, style]);
        }
        for radix in 0..3 {
            bytes.extend([8, 255, 255, radix]);
        }
        for denominator in 0..7 {
            bytes.extend([11, 255, 127, denominator]);
            bytes.extend([11, 1, 128, denominator]);
        }
        exercise(&bytes);
    }

    #[test]
    fn invalid_unicode_whitespace_and_special_numbers_preserve_their_semantics() {
        let mut bytes = Vec::new();
        for index in 0..INVALID.len() as u8 {
            bytes.extend([9, 0, index, 0]);
        }
        bytes.extend([9, 1, 0, 0]);
        bytes.extend([9, 2, 0, 0]);
        for index in 0..SPECIAL.len() as u8 {
            bytes.extend([10, index, 0, 0]);
        }
        // Preserve the command kind modulo twelve while setting padding bits.
        bytes.extend([199, 0, 0, 2]); // kind7; prefix+suffix ECMA whitespace
        bytes.extend([200, 255, 255, 2]); // kind8; prefix+suffix
        bytes.extend([202, 3, 0, 0]); // kind10; signed zero with padding
        exercise(&bytes);
    }
}
