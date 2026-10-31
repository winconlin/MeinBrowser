use super::*;
use crate::agent::{Abrupt, Agent, ErrorKind, Exception};
use crate::object::ordinary_object_create;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn numbers_tell_zeros_apart_but_not_nans() {
    let heap = Heap::new();
    let nan = Value::number(f64::NAN);
    let negative_nan = Value::number(f64::NAN.copysign(-1.0));
    let zero = Value::number(0.0);
    let minus_zero = Value::number(-0.0);

    assert!(same_value(&heap, nan, negative_nan));
    assert!(!same_value(&heap, zero, minus_zero));
    assert!(same_value(&heap, minus_zero, Value::number(-0.0)));
    assert!(same_value(&heap, Value::int32(1), Value::number(1.0)));
    assert!(!same_value(&heap, Value::number(1.0), Value::number(1.5)));

    assert!(same_value_zero(&heap, nan, negative_nan));
    assert!(same_value_zero(&heap, zero, minus_zero));
    assert!(!same_value_zero(
        &heap,
        Value::number(1.0),
        Value::number(2.0)
    ));
}

#[test]
fn strings_compare_by_content() {
    let mut heap = Heap::new();
    heap.scope(|heap, scope| {
        let mut string = |text: &str| heap.alloc_string(scope, &units(text)).expect("Speicher");
        let (a, b, c, short) = (string("abc"), string("abc"), string("abd"), string("ab"));
        let (wide, wide_too) = (string("a€"), string("a€"));
        let get = |local| heap.get(local);
        assert!(same_value(heap, get(a), get(b)));
        assert!(!same_value(heap, get(a), get(c)));
        assert!(!same_value(heap, get(a), get(short)));
        assert!(same_value(heap, get(wide), get(wide_too)));
        assert!(!same_value(heap, get(a), get(wide)));
        assert!(same_value_zero(heap, get(a), get(b)));
    });
}

#[test]
fn other_values_compare_by_type_and_identity() {
    let mut agent = Agent::new().expect("Speicher");
    agent.scope(|agent, scope| {
        let first = ordinary_object_create(agent, scope, None).expect("Speicher");
        let second = ordinary_object_create(agent, scope, None).expect("Speicher");
        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        let other_symbol = agent.new_symbol(scope, None).expect("Speicher");
        let one = agent
            .heap_mut()
            .alloc_string(scope, &units("1"))
            .expect("Speicher");
        let heap = agent.heap();
        let get = |local| heap.get(local);

        assert!(same_value(heap, get(first), get(first)));
        assert!(!same_value(heap, get(first), get(second)));
        assert!(same_value(heap, get(symbol), get(symbol)));
        assert!(!same_value(heap, get(symbol), get(other_symbol)));
        assert!(!same_value(heap, get(one), Value::int32(1)));
        assert!(!same_value(heap, Value::UNDEFINED, Value::NULL));
        assert!(same_value(heap, Value::NULL, Value::NULL));
        assert!(same_value(heap, Value::TRUE, Value::boolean(true)));
        assert!(!same_value(heap, Value::TRUE, Value::FALSE));
        assert!(!same_value_zero(heap, Value::FALSE, Value::int32(0)));
    });
}

fn thrown(agent: &mut Agent, scope: &Scope) -> Option<ErrorKind> {
    match agent.take_exception(scope) {
        Some(Exception::Error(kind)) => Some(kind),
        _ => None,
    }
}

fn string<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> Local<'s> {
    agent
        .heap_mut()
        .alloc_string(scope, &units(text))
        .expect("Speicher")
}

#[test]
fn strict_equality_compares_numbers_by_value() {
    let mut agent = Agent::new().expect("Speicher");
    agent.scope(|agent, scope| {
        let one = string(agent, scope, "1");
        let one_again = string(agent, scope, "1");
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        let other = ordinary_object_create(agent, scope, None).expect("Speicher");
        let heap = agent.heap();
        let get = |local| heap.get(local);
        let nan = Value::number(f64::NAN);
        assert!(!is_strictly_equal(heap, nan, nan));
        assert!(is_strictly_equal(
            heap,
            Value::number(0.0),
            Value::number(-0.0)
        ));
        assert!(is_strictly_equal(heap, Value::int32(1), Value::number(1.0)));
        assert!(!is_strictly_equal(heap, Value::int32(1), get(one)));
        assert!(is_strictly_equal(heap, get(one), get(one_again)));
        assert!(!is_strictly_equal(heap, Value::NULL, Value::UNDEFINED));
        assert!(is_strictly_equal(heap, get(object), get(object)));
        assert!(!is_strictly_equal(heap, get(object), get(other)));
    });
}

#[test]
fn loose_equality_converts_step_by_step() {
    let mut agent = Agent::new().expect("Speicher");
    agent.scope(|agent, scope| {
        let one = string(agent, scope, "1");
        let one_point_zero = string(agent, scope, "1.0");
        let hex = string(agent, scope, " 0x10 ");
        let empty = string(agent, scope, "");
        let true_text = string(agent, scope, "true");
        let letter = string(agent, scope, "x");
        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        let other_symbol = agent.new_symbol(scope, None).expect("Speicher");
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        let other = ordinary_object_create(agent, scope, None).expect("Speicher");
        let local = |agent: &Agent, value: Value<'static>| agent.heap().local(scope, value);
        let null = local(agent, Value::NULL);
        let undefined = local(agent, Value::UNDEFINED);
        let yes = local(agent, Value::TRUE);
        let no = local(agent, Value::FALSE);
        let zero = local(agent, Value::number(0.0));
        let minus_zero = local(agent, Value::number(-0.0));
        let number_one = local(agent, Value::int32(1));
        let sixteen = local(agent, Value::int32(16));
        let nan = local(agent, Value::number(f64::NAN));
        let cases = [
            (null, undefined, true),
            (null, null, true),
            (null, zero, false),
            (undefined, zero, false),
            (null, no, false),
            (one, number_one, true),
            (one, one_point_zero, false),
            (hex, sixteen, true),
            (yes, number_one, true),
            (yes, one, true),
            (no, empty, true),
            (true_text, yes, false),
            (letter, nan, false),
            (nan, nan, false),
            (zero, minus_zero, true),
            (symbol, symbol, true),
            (symbol, other_symbol, false),
            (object, object, true),
            (object, other, false),
            (object, null, false),
            (object, undefined, false),
        ];
        for (position, (x, y, expected)) in cases.into_iter().enumerate() {
            // Der Vergleich ist symmetrisch.
            assert_eq!(
                is_loosely_equal(agent, scope, x, y),
                Ok(expected),
                "Fall {position}"
            );
            assert_eq!(
                is_loosely_equal(agent, scope, y, x),
                Ok(expected),
                "Fall {position} umgekehrt"
            );
        }
        // Ohne aufrufbares valueOf oder toString wirft ToPrimitive.
        for (x, y) in [(object, number_one), (letter, object), (symbol, object)] {
            assert_eq!(is_loosely_equal(agent, scope, x, y), Err(Abrupt::Throw));
            assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        }
    });
}

#[test]
fn less_than_compares_strings_by_code_units() {
    let mut agent = Agent::new().expect("Speicher");
    agent.scope(|agent, scope| {
        let text = |agent: &mut Agent, content: &[u16]| {
            agent
                .heap_mut()
                .alloc_string(scope, content)
                .expect("Speicher")
        };
        let a = text(agent, &[0x61]);
        let ab = text(agent, &[0x61, 0x62]);
        let b = text(agent, &[0x62]);
        let ten = text(agent, &[0x31, 0x30]);
        let nine = text(agent, &[0x39]);
        let astral = text(agent, &[0xD800, 0xDC00]);
        let last_bmp = text(agent, &[0xFFFF]);
        let cases = [
            (a, b, Some(true)),
            (a, ab, Some(true)),
            (ab, a, Some(false)),
            (b, ab, Some(false)),
            (a, a, Some(false)),
            (ten, nine, Some(true)),
            // Nach Codeeinheiten, nicht nach Codepunkten.
            (astral, last_bmp, Some(true)),
            (last_bmp, astral, Some(false)),
        ];
        for (position, (x, y, expected)) in cases.into_iter().enumerate() {
            assert_eq!(
                is_less_than(agent, scope, x, y, true),
                Ok(expected),
                "Fall {position}"
            );
            assert_eq!(
                is_less_than(agent, scope, x, y, false),
                Ok(expected),
                "Fall {position}"
            );
        }
    });
}

#[test]
fn less_than_compares_other_values_as_numbers() {
    let mut agent = Agent::new().expect("Speicher");
    agent.scope(|agent, scope| {
        let nine = string(agent, scope, "9");
        let letter = string(agent, scope, "x");
        let local = |agent: &Agent, value: Value<'static>| agent.heap().local(scope, value);
        let ten = local(agent, Value::int32(10));
        let one = local(agent, Value::int32(1));
        let two = local(agent, Value::int32(2));
        let null = local(agent, Value::NULL);
        let undefined = local(agent, Value::UNDEFINED);
        let yes = local(agent, Value::TRUE);
        let zero = local(agent, Value::number(0.0));
        let minus_zero = local(agent, Value::number(-0.0));
        let cases = [
            (ten, nine, Some(false)),
            (nine, ten, Some(true)),
            (letter, one, None),
            (one, undefined, None),
            (null, one, Some(true)),
            (minus_zero, zero, Some(false)),
            (yes, two, Some(true)),
        ];
        for (position, (x, y, expected)) in cases.into_iter().enumerate() {
            assert_eq!(
                is_less_than(agent, scope, x, y, true),
                Ok(expected),
                "Fall {position}"
            );
        }
        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        for (x, y) in [(symbol, one), (one, object)] {
            assert_eq!(is_less_than(agent, scope, x, y, true), Err(Abrupt::Throw));
            assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        }
    });
}
