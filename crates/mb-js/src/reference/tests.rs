use super::*;
use crate::agent::{Abrupt, Exception};
use crate::heap::Options;
use crate::object::ordinary_object_create;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn checked() -> (Agent, RealmId) {
    let mut agent = Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    (agent, realm)
}

/// Die Art des Fehlers der Engine, wenn `result` einer ist.
fn thrown<T>(agent: &mut Agent, scope: &Scope, result: Completion<T>) -> Option<ErrorKind> {
    match result {
        Err(Abrupt::Throw) => match agent.take_exception(scope) {
            Some(Exception::Error(kind)) => Some(kind),
            _ => None,
        },
        _ => None,
    }
}

fn text(agent: &Agent, value: Local<'_>) -> Option<String> {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::String(string) => String::from_utf16(&heap.string_units(string)).ok(),
        _ => None,
    }
}

fn same(agent: &Agent, a: Local<'_>, b: Local<'_>) -> bool {
    let heap = agent.heap();
    same_value(heap, heap.get(a), heap.get(b))
}

#[test]
fn undefined_and_null_have_no_object() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        for value in [Value::UNDEFINED, Value::NULL] {
            let value = agent.heap().local(scope, value);
            let result = to_object_base(agent, scope, realm, value);
            assert_eq!(thrown(agent, scope, result), Some(ErrorKind::TypeError));
        }
    });
}

#[test]
fn primitives_use_the_prototype_of_their_wrapper() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        let string = agent
            .heap_mut()
            .alloc_string(scope, &units("ab"))
            .expect("Speicher");
        let number = agent.heap().local(scope, Value::number(1.5));
        let boolean = agent.heap().local(scope, Value::TRUE);
        for (value, intrinsic) in [
            (string, Intrinsic::StringPrototype),
            (number, Intrinsic::NumberPrototype),
            (boolean, Intrinsic::BooleanPrototype),
        ] {
            let expected = agent.intrinsic(scope, realm, intrinsic);
            match to_object_base(agent, scope, realm, value) {
                Ok(Base::Primitive(prototype)) => {
                    assert!(same(agent, prototype, expected), "{intrinsic:?}");
                }
                other => panic!("{intrinsic:?}: {other:?}"),
            }
        }
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        match to_object_base(agent, scope, realm, object) {
            Ok(Base::Object(base)) => assert!(same(agent, base, object)),
            other => panic!("{other:?}"),
        }
    });
}

#[test]
fn strings_have_their_units_and_length_as_own_properties() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        let string = agent
            .heap_mut()
            .alloc_string(scope, &units("ab"))
            .expect("Speicher");
        let base = to_object_base(agent, scope, realm, string).expect("Basis");
        let length = agent
            .key_from_units(scope, &units("length"))
            .expect("Speicher");
        let other = agent.key_from_units(scope, &units("x")).expect("Speicher");

        let unit = get_value(agent, scope, base, string, PropertyKey::Index(1)).expect("Wert");
        assert_eq!(text(agent, unit).as_deref(), Some("b"));
        let beyond = get_value(agent, scope, base, string, PropertyKey::Index(2)).expect("Wert");
        assert!(agent.heap().get(beyond).is_undefined());
        let value = get_value(agent, scope, base, string, length).expect("Wert");
        assert_eq!(agent.heap().get(value).as_number(), Some(2.0));
        let missing = get_value(agent, scope, base, string, other).expect("Wert");
        assert!(agent.heap().get(missing).is_undefined());

        // Eigene Eigenschaften sind fest, alles andere scheitert am
        // primitiven Empfänger.
        for key in [PropertyKey::Index(0), length, other] {
            assert!(put_value(agent, scope, base, string, key, string, false).is_ok());
            let result = put_value(agent, scope, base, string, key, string, true);
            assert_eq!(thrown(agent, scope, result), Some(ErrorKind::TypeError));
        }

        for (key, deletable) in [
            (PropertyKey::Index(0), false),
            (length, false),
            (PropertyKey::Index(2), true),
            (other, true),
        ] {
            let deleted = delete_property(agent, scope, base, string, key, false);
            assert_eq!(deleted.ok(), Some(deletable));
            let strict = delete_property(agent, scope, base, string, key, true);
            if deletable {
                assert_eq!(strict.ok(), Some(true));
            } else {
                assert_eq!(thrown(agent, scope, strict), Some(ErrorKind::TypeError));
            }
        }
    });
}

#[test]
fn copying_from_a_string_defines_its_units() {
    let (mut agent, _) = checked();
    agent.scope(|agent, scope| {
        let target = ordinary_object_create(agent, scope, None).expect("Speicher");
        let source = agent.heap_mut().alloc_string(scope, &units("ab")).expect("Speicher");
        copy_data_properties(agent, scope, target, source).expect("kopiert");
        let keys = object::own_property_keys(agent, scope, target).expect("Schlüssel");
        assert_eq!(keys.len(), 2);
        for (index, expected) in [(0, "a"), (1, "b")] {
            assert!(matches!(keys.get(index), Some(PropertyKey::Index(key)) if usize::try_from(*key) == Ok(index)));
            let key = PropertyKey::Index(u32::try_from(index).expect("klein"));
            let value = get(agent, scope, target, key).expect("Wert");
            assert_eq!(text(agent, value).as_deref(), Some(expected));
        }
    });
}

#[test]
fn copying_from_objects_takes_enumerable_own_properties_in_order() {
    let (mut agent, _) = checked();
    agent.scope(|agent, scope| {
        let prototype = ordinary_object_create(agent, scope, None).expect("Speicher");
        let inherited = agent
            .key_from_units(scope, &units("geerbt"))
            .expect("Speicher");
        let value = agent.heap().local(scope, Value::int32(1));
        create_data_property_or_throw(agent, scope, prototype, inherited, value).expect("gesetzt");
        let source = ordinary_object_create(agent, scope, Some(prototype)).expect("Speicher");
        let b = agent.key_from_units(scope, &units("b")).expect("Speicher");
        let a = agent.key_from_units(scope, &units("a")).expect("Speicher");
        for (key, number) in [(b, 2), (PropertyKey::Index(0), 0), (a, 1)] {
            let value = agent.heap().local(scope, Value::int32(number));
            create_data_property_or_throw(agent, scope, source, key, value).expect("gesetzt");
        }
        let target = ordinary_object_create(agent, scope, None).expect("Speicher");
        copy_data_properties(agent, scope, target, source).expect("kopiert");
        let keys = object::own_property_keys(agent, scope, target).expect("Schlüssel");
        let heap = agent.heap();
        assert_eq!(keys.len(), 3);
        let expected = [PropertyKey::Index(0), b, a];
        for (key, expected) in keys.iter().zip(expected) {
            assert!(same_key(heap, *key, expected));
        }
    });
}

#[test]
fn copying_skips_values_without_own_properties() {
    let (mut agent, _) = checked();
    agent.scope(|agent, scope| {
        let target = ordinary_object_create(agent, scope, None).expect("Speicher");
        for value in [Value::UNDEFINED, Value::NULL, Value::TRUE, Value::int32(3)] {
            let source = agent.heap().local(scope, value);
            copy_data_properties(agent, scope, target, source).expect("kopiert");
        }
        let keys = object::own_property_keys(agent, scope, target).expect("Schlüssel");
        assert!(keys.is_empty());
    });
}

#[test]
fn instanceof_needs_an_object_with_a_handler_or_a_callable() {
    let (mut agent, _) = checked();
    agent.scope(|agent, scope| {
        let value = ordinary_object_create(agent, scope, None).expect("Speicher");
        let number = agent.heap().local(scope, Value::int32(1));
        for target in [number, value] {
            let result = instanceof_operator(agent, scope, value, target);
            assert_eq!(thrown(agent, scope, result), Some(ErrorKind::TypeError));
        }
        // Schritt 1 von OrdinaryHasInstance.
        assert_eq!(
            ordinary_has_instance(agent, scope, value, value).ok(),
            Some(false)
        );
    });
}
