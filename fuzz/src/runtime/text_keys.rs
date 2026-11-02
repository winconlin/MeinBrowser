//! Primitive ToString/ToPropertyKey with independently known text.
//! Commands [kind, a, b, c], kind modulo ten; at most 64 commands.
//! No object conversion, interpreter, float formatter or JS parser is an oracle.

use mb_js::agent::{Abrupt, Agent, ErrorKind, Exception};
use mb_js::conversion;
use mb_js::heap::{Local, Options, Scope};
use mb_js::key::{PropertyKey, WellKnownSymbol};
use mb_js::value::{Kind, Value};

const MAX_COMMANDS: usize = 64;
const MAX_TEXT_UNITS: usize = 32;
const LAST_INDEX: u64 = 4_294_967_294;
const NUMBERS: [(f64, &str); 32] = [
    (0.0, "0"),
    (-0.0, "0"),
    (f64::NAN, "NaN"),
    (f64::INFINITY, "Infinity"),
    (f64::NEG_INFINITY, "-Infinity"),
    (f64::from_bits(1), "5e-324"),
    (-f64::from_bits(1), "-5e-324"),
    (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
    (f64::MAX, "1.7976931348623157e+308"),
    (-f64::MAX, "-1.7976931348623157e+308"),
    (0.000_001, "0.000001"),
    (0.000_000_1, "1e-7"),
    (1e20, "100000000000000000000"),
    (1e21, "1e+21"),
    (1.23e22, "1.23e+22"),
    (0.1, "0.1"),
    (0.1 + 0.2, "0.30000000000000004"),
    (1.0 / 3.0, "0.3333333333333333"),
    (4_294_967_294.0, "4294967294"),
    (4_294_967_295.0, "4294967295"),
    (4_294_967_296.0, "4294967296"),
    (9_007_199_254_740_991.0, "9007199254740991"),
    (9_007_199_254_740_992.0, "9007199254740992"),
    (9_007_199_254_740_994.0, "9007199254740994"),
    (-1.0, "-1"),
    (0.5, "0.5"),
    (-0.5, "-0.5"),
    (123.456, "123.456"),
    (1e15 + 0.25, "1000000000000000.2"),
    (1e15 + 0.75, "1000000000000000.8"),
    (2_147_483_648.0, "2147483648"),
    (-2_147_483_649.0, "-2147483649"),
];
const KEYS: [&str; 21] = [
    "",
    "0",
    "-0",
    "+0",
    "00",
    "1",
    "01",
    "-1",
    "1.0",
    "1e0",
    "4294967294",
    "4294967295",
    "4294967296",
    "9007199254740991",
    "__proto__",
    "constructor",
    "toString",
    "NaN",
    "Infinity",
    "a\0b",
    "\u{feff}0",
];

#[derive(Debug)]
enum Input {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64, Vec<u16>),
    Text(Vec<u16>),
    Symbol {
        description: Option<Vec<u16>>,
        known: bool,
    },
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Exact rational long division. With denominator <= 256 and |value| <=
/// 32768, every generated decimal is also the shortest binary64 spelling:
/// its last decimal place is much larger than the binary64 rounding interval.
fn dyadic(numerator: i16, exponent: u8) -> (f64, Vec<u16>) {
    let denominator = 1_u32 << (exponent % 9);
    let magnitude = u32::from(numerator.unsigned_abs());
    let mut text = String::new();
    if numerator < 0 {
        text.push('-');
    }
    text.push_str(&(magnitude / denominator).to_string());
    let mut remainder = magnitude % denominator;
    if remainder != 0 {
        text.push('.');
        while remainder != 0 {
            remainder *= 10;
            text.push(char::from(b'0' + (remainder / denominator) as u8));
            remainder %= denominator;
        }
    }
    (f64::from(numerator) / f64::from(denominator), units(&text))
}

fn command([kind, a, b, c]: [u8; 4]) -> Input {
    match kind % 10 {
        0 => Input::Undefined,
        1 => Input::Null,
        2 => Input::Boolean(a & 1 != 0),
        3 => {
            let (number, expected) = NUMBERS[usize::from(a) % NUMBERS.len()];
            Input::Number(number, units(expected))
        }
        4 => {
            let integer = i32::from_le_bytes([a, b, c, if c & 0x80 == 0 { 0 } else { 0xff }]);
            Input::Number(f64::from(integer), units(&integer.to_string()))
        }
        5 => {
            let (number, expected) = dyadic(i16::from_le_bytes([a, b]), c);
            Input::Number(number, expected)
        }
        6 => Input::Text(units(KEYS[usize::from(a) % KEYS.len()])),
        7 => {
            let first = u16::from_be_bytes([a, b]);
            Input::Text(
                (0..u16::from(c % 17))
                    .map(|index| first.wrapping_add(index * 257))
                    .collect(),
            )
        }
        8 => Input::Symbol {
            description: match a % 4 {
                0 => None,
                1 => Some(Vec::new()),
                2 => Some(vec![0xd800 | u16::from(b), u16::from(c)]),
                _ => Some(units("Symbol.toPrimitive")),
            },
            known: a % 4 == 3,
        },
        _ => {
            let integer = u16::from_le_bytes([a, b]);
            let text = match c % 8 {
                0 => integer.to_string(),
                1 => format!("0{integer}"),
                2 => format!("+{integer}"),
                3 => format!("{integer}.0"),
                4 => format!("{integer}e0"),
                5 => format!(" {integer} "),
                6 => format!("-{integer}"),
                _ => format!("\t{integer}\n"),
            };
            Input::Text(units(&text))
        }
    }
}

impl Input {
    fn expected(&self) -> Option<Vec<u16>> {
        Some(match self {
            Self::Undefined => units("undefined"),
            Self::Null => units("null"),
            Self::Boolean(true) => units("true"),
            Self::Boolean(false) => units("false"),
            Self::Number(_, expected) | Self::Text(expected) => expected.clone(),
            Self::Symbol { .. } => return None,
        })
    }

    /// Symbols include an independent peer with the same description, so
    /// returning a cached symbol by description cannot preserve identity.
    fn local<'s>(&self, agent: &mut Agent, scope: &'s Scope) -> (Local<'s>, Option<Local<'s>>) {
        let value = match self {
            Self::Undefined => Value::UNDEFINED,
            Self::Null => Value::NULL,
            Self::Boolean(value) => Value::boolean(*value),
            Self::Number(number, _) => Value::number(*number),
            Self::Text(text) => {
                return (
                    agent
                        .heap_mut()
                        .alloc_string(scope, text)
                        .expect("bounded text"),
                    None,
                );
            }
            Self::Symbol { description, known } => {
                let description = description.as_ref().map(|text| {
                    agent
                        .heap_mut()
                        .alloc_string(scope, text)
                        .expect("bounded description")
                });
                let symbol = if *known {
                    agent.well_known_symbol(scope, WellKnownSymbol::ToPrimitive)
                } else {
                    agent
                        .new_symbol(scope, description)
                        .expect("bounded symbol")
                };
                let peer = agent
                    .new_symbol(scope, description)
                    .expect("distinct symbol");
                return (symbol, Some(peer));
            }
        };
        (agent.heap().local(scope, value), None)
    }
}

/// The oracle uses Rust's integer parser and canonical integer output,
/// not the runtime's UTF-16 array-index scanner or floating-point formatter.
fn index(text: &[u16]) -> Option<u32> {
    let text = String::from_utf16(text).ok()?;
    let number = text.parse::<u64>().ok()?;
    (number <= LAST_INDEX && number.to_string() == text).then_some(number as u32)
}

fn check_key(agent: &Agent, key: PropertyKey<'_>, expected: &[u16]) {
    match (key, index(expected)) {
        (PropertyKey::Index(actual), Some(expected)) => assert_eq!(actual, expected, "array index"),
        (PropertyKey::String(handle), None) => {
            let Kind::String(string) = agent.heap().get(handle).kind() else {
                panic!("string key")
            };
            assert_eq!(
                agent.heap().string_units(string),
                expected,
                "exact key text"
            );
        }
        other => panic!("wrong key kind/index: {other:?}"),
    }
}

fn check_symbol(agent: &mut Agent, scope: &Scope, value: Local<'_>) {
    assert!(
        matches!(
            conversion::to_string(agent, scope, value),
            Err(Abrupt::Throw)
        ),
        "ToString(Symbol) must throw"
    );
    assert!(matches!(
        agent.take_exception(scope),
        Some(Exception::Error(ErrorKind::TypeError))
    ));
    assert!(
        agent.take_exception(scope).is_none(),
        "consume TypeError once"
    );
    let key = conversion::to_property_key(agent, scope, value).expect("symbol key");
    agent.heap_mut().collect();
    let PropertyKey::Symbol(key) = key else {
        panic!("symbol remains a symbol")
    };
    assert_eq!(
        agent.heap().get(key).kind(),
        agent.heap().get(value).kind(),
        "symbol identity after GC"
    );
}

fn check(agent: &mut Agent, scope: &Scope, input: &Input) {
    let (value, peer) = input.local(agent, scope);
    agent.heap_mut().collect();
    if let Some(expected) = input.expected() {
        assert!(expected.len() <= MAX_TEXT_UNITS, "bounded expected text");
        let text = conversion::to_string(agent, scope, value).expect("primitive ToString");
        agent.heap_mut().collect();
        let Kind::String(string) = agent.heap().get(text).kind() else {
            panic!("ToString result")
        };
        assert_eq!(
            agent.heap().string_units(string),
            expected,
            "canonical text"
        );
        let key = conversion::to_property_key(agent, scope, value).expect("primitive key");
        let equivalent = agent
            .heap_mut()
            .alloc_string(scope, &expected)
            .expect("equivalent text");
        let again = conversion::to_property_key(agent, scope, equivalent).expect("equivalent key");
        agent.heap_mut().collect();
        check_key(agent, key, &expected);
        check_key(agent, again, &expected);
        if let (PropertyKey::String(first), PropertyKey::String(second)) = (key, again) {
            assert_eq!(
                agent.heap().get(first).kind(),
                agent.heap().get(second).kind(),
                "equal key texts share an atom"
            );
        }
    } else {
        let peer = peer.expect("symbol peer");
        assert_ne!(
            agent.heap().get(value).kind(),
            agent.heap().get(peer).kind(),
            "distinct symbols with the same description"
        );
        check_symbol(agent, scope, value);
        check_symbol(agent, scope, peer);
    }
    assert!(
        agent.take_exception(scope).is_none(),
        "no pending exception"
    );
}

fn run(data: &[u8], stress: bool) -> usize {
    let mut agent = Agent::with_options(Options {
        limit: 1 << 20,
        stress,
        verify: true,
    })
    .expect("bounded text/key agent");
    let mut count = 0;
    for &bytes in data.as_chunks::<4>().0.iter().take(MAX_COMMANDS) {
        let input = command(bytes);
        agent.scope(|agent, scope| check(agent, scope, &input));
        agent.heap_mut().collect();
        agent.heap().verify().expect("valid text/key heap");
        count += 1;
    }
    count
}

pub(super) fn exercise(data: &[u8]) {
    assert_eq!(
        run(data, false),
        run(data, true),
        "same bounded command count"
    );
}

#[cfg(test)]
mod tests;
