use super::*;
use crate::agent::{Abrupt, Exception};
use crate::descriptor::PropertyDescriptor;
use crate::heap::Options;
use crate::key::symbol_description;
use crate::object::{define_own_property, ordinary_object_create};

fn checked() -> Agent {
    Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher")
}

fn number(text: &str) -> f64 {
    string_to_number(&units(text))
}

fn thrown(agent: &mut Agent, scope: &Scope) -> Option<ErrorKind> {
    match agent.take_exception(scope) {
        Some(Exception::Error(kind)) => Some(kind),
        _ => None,
    }
}

fn same(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

#[test]
fn strings_become_numbers() {
    let cases = [
        ("", 0.0),
        ("   ", 0.0),
        (" \t\n12 ", 12.0),
        ("\u{FEFF}1\u{2028}", 1.0),
        ("\u{00A0}5\u{3000}", 5.0),
        ("\u{000B}\u{000C}\r7\u{2029}", 7.0),
        ("+1.5", 1.5),
        ("-0", -0.0),
        ("-0.0e5", -0.0),
        ("+0", 0.0),
        (".5", 0.5),
        ("5.", 5.0),
        ("1e3", 1000.0),
        ("1E-3", 0.001),
        ("1.e2", 100.0),
        ("-.5e+1", -5.0),
        ("Infinity", f64::INFINITY),
        ("+Infinity", f64::INFINITY),
        ("-Infinity", f64::NEG_INFINITY),
        ("00012", 12.0),
        ("012", 12.0),
        ("0.0000001", 1e-7),
        ("1e400", f64::INFINITY),
        ("1e-400", 0.0),
        ("1e99999999999999999999", f64::INFINITY),
        ("0x1F", 31.0),
        ("0X1f", 31.0),
        ("0b101", 5.0),
        ("0B11", 3.0),
        ("0o17", 15.0),
        ("0O7", 7.0),
        ("9007199254740993", 9_007_199_254_740_992.0),
        ("9007199254740995", 9_007_199_254_740_996.0),
        ("0x20000000000001", 9_007_199_254_740_992.0),
        ("0x20000000000003", 9_007_199_254_740_996.0),
    ];
    for (text, expected) in cases {
        let found = number(text);
        assert!(same(found, expected), "{text:?}: {found} statt {expected}");
    }
}

#[test]
fn malformed_strings_become_nan() {
    for text in [
        "-",
        "+",
        ".",
        "e5",
        "1e",
        "1e+",
        "1e5.5",
        "1e5e5",
        "1..2",
        "1.2.3",
        "infinity",
        "INFINITY",
        "-0x1",
        "+0x1",
        "0x",
        "0x1g",
        "0b2",
        "0o8",
        "1_000",
        "0x1_0",
        "1n",
        " 1 2 ",
        "1,5",
        "\u{0661}",
        "\u{FF11}",
        "\u{180E}1",
        "NaN",
        "Infinityx",
    ] {
        assert!(number(text).is_nan(), "{text:?}");
    }
    // Ein einzelnes Surrogat ist kein Leerraum.
    assert!(string_to_number(&[0x31, 0xD800]).is_nan());
}

#[test]
fn values_become_numbers() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let local = |agent: &Agent, value| agent.heap().local(scope, value);
        let cases = [
            (Value::UNDEFINED, f64::NAN),
            (Value::NULL, 0.0),
            (Value::TRUE, 1.0),
            (Value::FALSE, 0.0),
            (Value::int32(-7), -7.0),
            (Value::number(-0.0), -0.0),
            (Value::number(2.5), 2.5),
        ];
        for (value, expected) in cases {
            let value = local(agent, value);
            let found = to_number(agent, scope, value).expect("kein Fehler");
            assert!(same(found, expected), "{found} statt {expected}");
        }
        let text = agent
            .heap_mut()
            .alloc_string(scope, &units(" 0x10 "))
            .expect("Speicher");
        assert_eq!(to_number(agent, scope, text), Ok(16.0));

        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        assert_eq!(to_number(agent, scope, symbol), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));

        // Ohne aufrufbares valueOf oder toString wirft ToPrimitive.
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        assert_eq!(to_number(agent, scope, object), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn integers_are_truncated() {
    for (number, expected) in [
        (f64::NAN, 0.0),
        (-0.0, 0.0),
        (-0.5, 0.0),
        (0.9, 0.0),
        (2.7, 2.0),
        (-2.7, -2.0),
        (1e300, 1e300),
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    ] {
        let found = integer_or_infinity(number);
        assert!(same(found, expected), "{number}: {found} statt {expected}");
    }
}

#[test]
fn uint32_wraps_modulo_two_to_the_32() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        for (number, expected) in [
            (0.0, 0),
            (-0.0, 0),
            (1.9, 1),
            (-1.0, u32::MAX),
            (-1.9, u32::MAX),
            (4_294_967_295.0, u32::MAX),
            (4_294_967_296.0, 0),
            (4_294_967_297.0, 1),
            (-4_294_967_296.0, 0),
            (-4_294_967_297.0, u32::MAX),
            (1e20, 1_661_992_960),
            (9_007_199_254_740_994.0, 2),
            (f64::NAN, 0),
            (f64::INFINITY, 0),
            (f64::NEG_INFINITY, 0),
        ] {
            let value = agent.heap().local(scope, Value::number(number));
            assert_eq!(to_uint32(agent, scope, value), Ok(expected), "{number}");
        }
    });
}

#[test]
fn primitives_stay_as_they_are() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let text = agent
            .heap_mut()
            .alloc_string(scope, &units("Text"))
            .expect("Speicher");
        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        let one = agent.heap().local(scope, Value::int32(1));
        for value in [text, symbol, one] {
            let primitive = to_primitive(agent, scope, value, None).expect("kein Fehler");
            assert_eq!(
                agent.heap().get(primitive).bits(),
                agent.heap().get(value).bits()
            );
        }
    });
}

#[test]
fn objects_need_callable_conversions() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let plain = ordinary_object_create(agent, scope, None).expect("Speicher");
        for preferred in [
            None,
            Some(PreferredType::String),
            Some(PreferredType::Number),
        ] {
            assert_eq!(
                to_primitive(agent, scope, plain, preferred).err(),
                Some(Abrupt::Throw)
            );
            assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        }

        // %Symbol.toPrimitive% muss aufrufbar sein, wenn es nicht
        // undefined oder null ist.
        let symbol = agent.well_known_symbol(scope, WellKnownSymbol::ToPrimitive);
        let key = PropertyKey::Symbol(symbol);
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        let null = agent.heap().local(scope, Value::NULL);
        let descriptor = PropertyDescriptor::data(null, true, true, true);
        assert_eq!(
            define_own_property(agent, scope, object, key, &descriptor),
            Ok(true)
        );
        assert_eq!(
            ordinary_to_primitive(agent, scope, object, PreferredType::String).err(),
            Some(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        let not_callable = PropertyDescriptor::data(plain, true, true, true);
        assert_eq!(
            define_own_property(agent, scope, object, key, &not_callable),
            Ok(true)
        );
        assert_eq!(
            to_primitive(agent, scope, object, None).err(),
            Some(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn well_known_symbols_are_shared_and_described() {
    let mut agent = checked();
    agent.heap_mut().collect();
    agent.scope(|agent, scope| {
        let mut seen = Vec::new();
        for symbol in WellKnownSymbol::ALL {
            let first = agent.well_known_symbol(scope, symbol);
            let again = agent.well_known_symbol(scope, symbol);
            let heap = agent.heap();
            let bits = heap.get(first).bits();
            assert_eq!(bits, heap.get(again).bits());
            assert!(!seen.contains(&bits), "{symbol:?}");
            seen.push(bits);
            let Kind::Symbol(cell) = heap.get(first).kind() else {
                panic!("kein Symbol: {symbol:?}");
            };
            let Kind::String(text) = symbol_description(heap, cell).kind() else {
                panic!("keine Beschreibung: {symbol:?}");
            };
            assert_eq!(heap.string_units(text), units(symbol.description()));
        }
        assert_eq!(WellKnownSymbol::Iterator.description(), "Symbol.iterator");
    });
}

fn string_units_of(agent: &Agent, string: Local<'_>) -> Option<Vec<u16>> {
    match agent.heap().get(string).kind() {
        Kind::String(string) => Some(agent.heap().string_units(string)),
        _ => None,
    }
}

#[test]
fn values_become_strings() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        for (value, expected) in [
            (Value::UNDEFINED, "undefined"),
            (Value::NULL, "null"),
            (Value::TRUE, "true"),
            (Value::FALSE, "false"),
            (Value::number(-0.0), "0"),
            (Value::int32(-42), "-42"),
            (Value::number(1.5e-7), "1.5e-7"),
            (Value::number(f64::NAN), "NaN"),
        ] {
            let value = agent.heap().local(scope, value);
            let string = to_string(agent, scope, value).expect("kein Fehler");
            assert_eq!(
                string_units_of(agent, string),
                Some(units(expected)),
                "{expected}"
            );
        }
        // Eine Zeichenkette bleibt dieselbe, auch mit einzelnem Surrogat.
        let text = agent
            .heap_mut()
            .alloc_string(scope, &[0x61, 0xD800])
            .expect("Speicher");
        let same_text = to_string(agent, scope, text).expect("kein Fehler");
        assert_eq!(
            agent.heap().get(same_text).bits(),
            agent.heap().get(text).bits()
        );

        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        assert_eq!(to_string(agent, scope, symbol).err(), Some(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        assert_eq!(to_string(agent, scope, object).err(), Some(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn values_become_booleans() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let string = |agent: &mut Agent, text: &str| {
            agent
                .heap_mut()
                .alloc_string(scope, &units(text))
                .expect("Speicher")
        };
        let empty = string(agent, "");
        let zero = string(agent, "0");
        let space = string(agent, " ");
        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        let heap = agent.heap();
        let cases = [
            (Value::UNDEFINED, false),
            (Value::NULL, false),
            (Value::FALSE, false),
            (Value::TRUE, true),
            (Value::number(0.0), false),
            (Value::number(-0.0), false),
            (Value::number(f64::NAN), false),
            (Value::int32(1), true),
            (Value::number(f64::NEG_INFINITY), true),
            (Value::number(5e-324), true),
            (heap.get(empty), false),
            (heap.get(zero), true),
            (heap.get(space), true),
            (heap.get(symbol), true),
            (heap.get(object), true),
        ];
        for (position, (value, expected)) in cases.into_iter().enumerate() {
            assert_eq!(to_boolean(heap, value), expected, "Fall {position}");
        }
    });
}

fn key_of<'s>(agent: &mut Agent, scope: &'s Scope, value: Value<'static>) -> PropertyKey<'s> {
    let value = agent.heap().local(scope, value);
    to_property_key(agent, scope, value).expect("kein Fehler")
}

#[test]
fn values_become_property_keys() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        for (number, expected) in [
            (0.0, 0),
            (-0.0, 0),
            (7.0, 7),
            (4_294_967_294.0, MAX_ARRAY_INDEX),
        ] {
            let key = key_of(agent, scope, Value::number(number));
            assert!(
                matches!(key, PropertyKey::Index(index) if index == expected),
                "{number}"
            );
        }
        // Keine Indizes: 2³² − 1, Brüche, negative Zahlen, NaN und alles,
        // was keine Zahl ist.
        for (value, text) in [
            (Value::number(4_294_967_295.0), "4294967295"),
            (Value::number(1.5), "1.5"),
            (Value::int32(-1), "-1"),
            (Value::number(f64::NAN), "NaN"),
            (Value::number(1e21), "1e+21"),
            (Value::TRUE, "true"),
            (Value::UNDEFINED, "undefined"),
        ] {
            let PropertyKey::String(atom) = key_of(agent, scope, value) else {
                panic!("kein Atom: {text}");
            };
            let expected = agent.atom(scope, &units(text)).expect("Speicher");
            assert_eq!(
                agent.heap().get(atom).bits(),
                agent.heap().get(expected).bits(),
                "{text}"
            );
        }
        let seven = agent
            .heap_mut()
            .alloc_string(scope, &units("7"))
            .expect("Speicher");
        let key = to_property_key(agent, scope, seven).expect("kein Fehler");
        assert!(matches!(key, PropertyKey::Index(7)));

        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        let PropertyKey::Symbol(key) = to_property_key(agent, scope, symbol).expect("kein Fehler")
        else {
            panic!("kein Symbol");
        };
        assert_eq!(
            agent.heap().get(key).bits(),
            agent.heap().get(symbol).bits()
        );

        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        assert_eq!(
            to_property_key(agent, scope, object).err(),
            Some(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn int32_and_numeric_go_through_to_number() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let text = agent
            .heap_mut()
            .alloc_string(scope, &units(" 4294967295 "))
            .expect("Speicher");
        assert_eq!(to_int32(agent, scope, text), Ok(-1));
        assert_eq!(to_numeric(agent, scope, text), Ok(4_294_967_295.0));
        let yes = agent.heap().local(scope, Value::TRUE);
        assert_eq!(to_int32(agent, scope, yes), Ok(1));
        assert_eq!(to_numeric(agent, scope, yes), Ok(1.0));

        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        assert_eq!(to_int32(agent, scope, symbol), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        assert_eq!(to_numeric(agent, scope, symbol), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        assert_eq!(to_numeric(agent, scope, object), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}
