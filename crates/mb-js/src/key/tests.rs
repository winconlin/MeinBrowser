use super::*;
use crate::heap::Options;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn checked() -> Agent {
    Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher")
}

#[test]
fn array_indices_are_canonical_numbers_below_the_limit() {
    let index = |text: &str| array_index(&units(text));
    assert_eq!(index("0"), Some(0));
    assert_eq!(index("7"), Some(7));
    assert_eq!(index("1000"), Some(1000));
    assert_eq!(index("4294967294"), Some(MAX_ARRAY_INDEX));
    for text in [
        "",
        "4294967295",
        "4294967296",
        "99999999999",
        "01",
        "00",
        "-0",
        "-1",
        "+1",
        "1.0",
        "1e3",
        " 1",
        "1 ",
        "0x1",
        "\u{0661}",
        "\u{FF11}",
    ] {
        assert_eq!(index(text), None, "{text:?}");
    }
}

#[test]
fn keys_from_units_keep_indices_apart_from_names() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let index = agent.key_from_units(scope, &units("42")).expect("Speicher");
        assert!(matches!(index, PropertyKey::Index(42)));
        let name = agent
            .key_from_units(scope, &units("042"))
            .expect("Speicher");
        let again = agent
            .key_from_units(scope, &units("042"))
            .expect("Speicher");
        assert!(matches!(name, PropertyKey::String(_)));
        assert!(same_key(agent.heap(), name, again));
        assert!(!same_key(agent.heap(), name, index));
        let limit = agent
            .key_from_units(scope, &units("4294967295"))
            .expect("Speicher");
        assert!(matches!(limit, PropertyKey::String(_)));
    });
}

#[test]
fn keys_from_values() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let text = agent
            .heap_mut()
            .alloc_string(scope, &units("länge"))
            .expect("Speicher");
        let key = agent.key_from_value(scope, text).expect("Schlüssel");
        let atom = agent
            .key_from_units(scope, &units("länge"))
            .expect("Speicher");
        assert!(same_key(agent.heap(), key, atom));

        let digits = agent
            .heap_mut()
            .alloc_string(scope, &units("17"))
            .expect("Speicher");
        assert!(matches!(
            agent.key_from_value(scope, digits),
            Some(PropertyKey::Index(17))
        ));

        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        let key = agent.key_from_value(scope, symbol).expect("Schlüssel");
        assert!(same_key(agent.heap(), key, PropertyKey::Symbol(symbol)));

        // Zahlen und andere Werte brauchen erst ToPrimitive und ToString.
        let number = agent.heap().local(scope, Value::int32(17));
        assert!(agent.key_from_value(scope, number).is_none());
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        assert!(agent.key_from_value(scope, undefined).is_none());
    });
}

#[test]
fn symbols_are_unique_and_keep_their_description() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let description = agent
            .heap_mut()
            .alloc_string(scope, &units("Beschreibung"))
            .expect("Speicher");
        let first = agent
            .new_symbol(scope, Some(description))
            .expect("Speicher");
        let second = agent
            .new_symbol(scope, Some(description))
            .expect("Speicher");
        let bare = agent.new_symbol(scope, None).expect("Speicher");
        let heap = agent.heap();
        assert!(!same_key(
            heap,
            PropertyKey::Symbol(first),
            PropertyKey::Symbol(second)
        ));
        assert!(same_key(
            heap,
            PropertyKey::Symbol(first),
            PropertyKey::Symbol(first)
        ));

        agent.heap_mut().collect();
        let heap = agent.heap();
        for symbol in [first, second] {
            let Kind::Symbol(symbol) = heap.get(symbol).kind() else {
                panic!("kein Symbol");
            };
            let Kind::String(text) = symbol_description(heap, symbol).kind() else {
                panic!("keine Beschreibung");
            };
            assert_eq!(heap.string_units(text), units("Beschreibung"));
        }
        let Kind::Symbol(bare) = heap.get(bare).kind() else {
            panic!("kein Symbol");
        };
        assert!(symbol_description(heap, bare).is_undefined());
    });
}
