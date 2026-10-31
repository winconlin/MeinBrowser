use super::*;
use crate::agent::Agent;
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
