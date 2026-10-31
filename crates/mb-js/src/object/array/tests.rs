use super::*;
use crate::agent::{Abrupt, Exception};
use crate::heap::Options;
use crate::key::MAX_ARRAY_INDEX;
use crate::object;
use crate::operations::{self, is_array};
use crate::value::Kind;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn agent_with(stress: bool) -> Agent {
    Agent::with_options(Options {
        stress,
        verify: true,
        ..Options::default()
    })
    .expect("Speicher")
}

fn thrown(agent: &mut Agent, scope: &Scope) -> Option<ErrorKind> {
    match agent.take_exception(scope) {
        Some(Exception::Error(kind)) => Some(kind),
        _ => None,
    }
}

fn new_array<'s>(agent: &mut Agent, scope: &'s Scope, length: u64) -> Local<'s> {
    array_create(agent, scope, length, None).expect("kein Fehler")
}

fn length(agent: &mut Agent, scope: &Scope, array: Local<'_>) -> Option<f64> {
    let key = length_key(agent, scope).expect("Speicher");
    let value = object::get(agent, scope, array, key, array).expect("kein Fehler");
    agent.heap().get(value).as_number()
}

fn length_descriptor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    array: Local<'_>,
) -> PropertyDescriptor<'s> {
    let key = length_key(agent, scope).expect("Speicher");
    object::get_own_property(agent, scope, array, key)
        .expect("kein Fehler")
        .expect("Länge")
}

fn define_length(
    agent: &mut Agent,
    scope: &Scope,
    array: Local<'_>,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<bool> {
    let key = length_key(agent, scope)?;
    object::define_own_property(agent, scope, array, key, descriptor)
}

fn set_length_local(
    agent: &mut Agent,
    scope: &Scope,
    array: Local<'_>,
    value: Local<'_>,
) -> Completion<bool> {
    let descriptor = PropertyDescriptor {
        value: Some(value),
        ..PropertyDescriptor::default()
    };
    define_length(agent, scope, array, &descriptor)
}

fn set_length(
    agent: &mut Agent,
    scope: &Scope,
    array: Local<'_>,
    value: Value<'static>,
) -> Completion<bool> {
    let value = agent.heap().local(scope, value);
    set_length_local(agent, scope, array, value)
}

fn put(
    agent: &mut Agent,
    scope: &Scope,
    array: Local<'_>,
    index: u32,
    value: f64,
) -> Completion<bool> {
    let value = agent.heap().local(scope, Value::number(value));
    operations::create_data_property(agent, scope, array, PropertyKey::Index(index), value)
}

fn fill(agent: &mut Agent, scope: &Scope, array: Local<'_>, count: u32) {
    for index in 0..count {
        assert_eq!(put(agent, scope, array, index, f64::from(index)), Ok(true));
    }
}

fn indices(agent: &mut Agent, scope: &Scope, array: Local<'_>) -> Vec<u32> {
    object::own_property_keys(agent, scope, array)
        .expect("kein Fehler")
        .into_iter()
        .filter_map(|key| match key {
            PropertyKey::Index(index) => Some(index),
            _ => None,
        })
        .collect()
}

#[test]
fn arrays_start_with_their_length() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 3);
        assert_eq!(length(agent, scope, array), Some(3.0));
        let descriptor = length_descriptor(agent, scope, array);
        assert_eq!(
            (
                descriptor.writable,
                descriptor.enumerable,
                descriptor.configurable
            ),
            (Some(true), Some(false), Some(false))
        );
        let keys = object::own_property_keys(agent, scope, array).expect("kein Fehler");
        let length_key = length_key(agent, scope).expect("Speicher");
        assert_eq!(keys.len(), 1);
        assert!(
            keys.first()
                .is_some_and(|&key| same_key(agent.heap(), key, length_key))
        );
        assert!(indices(agent, scope, array).is_empty());

        let plain = object::ordinary_object_create(agent, scope, None).expect("Speicher");
        assert!(is_array(agent.heap(), agent.heap().get(array)));
        assert!(!is_array(agent.heap(), agent.heap().get(plain)));
        assert!(!is_array(agent.heap(), Value::int32(3)));

        assert!(
            object::get_prototype_of(agent, scope, array)
                .expect("kein Fehler")
                .is_none()
        );
        let with_prototype = array_create(agent, scope, 0, Some(plain)).expect("kein Fehler");
        let prototype = object::get_prototype_of(agent, scope, with_prototype)
            .expect("kein Fehler")
            .expect("Prototyp");
        assert_eq!(
            agent.heap().get(prototype).bits(),
            agent.heap().get(plain).bits()
        );
    });
}

#[test]
fn array_create_limits_the_length() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let longest = new_array(agent, scope, u64::from(u32::MAX));
        assert_eq!(length(agent, scope, longest), Some(4_294_967_295.0));
        for length in [u64::from(u32::MAX) + 1, u64::MAX] {
            assert_eq!(
                array_create(agent, scope, length, None).err(),
                Some(Abrupt::Throw)
            );
            assert_eq!(thrown(agent, scope), Some(ErrorKind::RangeError));
        }
    });
}

#[test]
fn elements_extend_the_length() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 0);
        assert_eq!(put(agent, scope, array, 0, 1.0), Ok(true));
        assert_eq!(length(agent, scope, array), Some(1.0));
        assert_eq!(put(agent, scope, array, 5, 2.0), Ok(true));
        assert_eq!(length(agent, scope, array), Some(6.0));
        assert_eq!(put(agent, scope, array, 2, 3.0), Ok(true));
        assert_eq!(length(agent, scope, array), Some(6.0));

        // Benannte Eigenschaften ändern die Länge nicht.
        let name = agent.key_from_units(scope, &units("x")).expect("Speicher");
        let value = agent.heap().local(scope, Value::int32(4));
        assert_eq!(
            operations::create_data_property(agent, scope, array, name, value),
            Ok(true)
        );
        assert_eq!(length(agent, scope, array), Some(6.0));

        // [[Set]] führt über [[DefineOwnProperty]] des Arrays.
        assert_eq!(
            operations::set(agent, scope, array, PropertyKey::Index(9), value, true),
            Ok(())
        );
        assert_eq!(length(agent, scope, array), Some(10.0));

        assert_eq!(put(agent, scope, array, MAX_ARRAY_INDEX, 5.0), Ok(true));
        assert_eq!(length(agent, scope, array), Some(4_294_967_295.0));
        assert_eq!(indices(agent, scope, array), [0, 2, 5, 9, MAX_ARRAY_INDEX]);
    });
}

#[test]
fn shrinking_deletes_elements() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 0);
        fill(agent, scope, array, 10);
        assert_eq!(set_length(agent, scope, array, Value::int32(3)), Ok(true));
        assert_eq!(length(agent, scope, array), Some(3.0));
        assert_eq!(indices(agent, scope, array), [0, 1, 2]);

        // Die Länge wird mit ToNumber und ToUint32 gelesen.
        let text = agent
            .heap_mut()
            .alloc_string(scope, &units(" 2 "))
            .expect("Speicher");
        assert_eq!(set_length_local(agent, scope, array, text), Ok(true));
        assert_eq!(length(agent, scope, array), Some(2.0));
        assert_eq!(indices(agent, scope, array), [0, 1]);

        assert_eq!(set_length(agent, scope, array, Value::int32(100)), Ok(true));
        assert_eq!(length(agent, scope, array), Some(100.0));
        assert_eq!(indices(agent, scope, array), [0, 1]);

        // -0 ist nach SameValueZero dieselbe Länge wie 0.
        assert_eq!(
            set_length(agent, scope, array, Value::number(-0.0)),
            Ok(true)
        );
        assert_eq!(length(agent, scope, array), Some(0.0));
        assert!(indices(agent, scope, array).is_empty());
    });
}

#[test]
fn sparse_arrays_shrink_by_their_elements() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 0);
        assert_eq!(put(agent, scope, array, 0, 1.0), Ok(true));
        assert_eq!(put(agent, scope, array, 70_000, 2.0), Ok(true));
        assert_eq!(put(agent, scope, array, MAX_ARRAY_INDEX, 3.0), Ok(true));
        assert_eq!(set_length(agent, scope, array, Value::int32(1)), Ok(true));
        assert_eq!(indices(agent, scope, array), [0]);
        assert_eq!(length(agent, scope, array), Some(1.0));
    });
}

#[test]
fn invalid_lengths_throw() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 0);
        fill(agent, scope, array, 3);
        for value in [1.5, -1.0, f64::NAN, 4_294_967_296.0, f64::INFINITY] {
            assert_eq!(
                set_length(agent, scope, array, Value::number(value)),
                Err(Abrupt::Throw),
                "{value}"
            );
            assert_eq!(thrown(agent, scope), Some(ErrorKind::RangeError));
        }
        assert_eq!(length(agent, scope, array), Some(3.0));

        // ToNumber wirft für Symbole, ToPrimitive für Objekte ohne
        // aufrufbare Umwandlung.
        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        assert_eq!(
            set_length_local(agent, scope, array, symbol),
            Err(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        let plain = object::ordinary_object_create(agent, scope, None).expect("Speicher");
        assert_eq!(
            set_length_local(agent, scope, array, plain),
            Err(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        assert_eq!(indices(agent, scope, array), [0, 1, 2]);
    });
}

#[test]
fn fixed_lengths_refuse_changes() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 0);
        fill(agent, scope, array, 3);
        let read_only = PropertyDescriptor {
            writable: Some(false),
            ..PropertyDescriptor::default()
        };
        assert_eq!(define_length(agent, scope, array, &read_only), Ok(true));
        assert_eq!(put(agent, scope, array, 5, 1.0), Ok(false));
        assert_eq!(put(agent, scope, array, 1, 9.0), Ok(true));
        assert_eq!(length(agent, scope, array), Some(3.0));
        assert_eq!(set_length(agent, scope, array, Value::int32(1)), Ok(false));
        assert_eq!(set_length(agent, scope, array, Value::int32(4)), Ok(false));
        assert_eq!(set_length(agent, scope, array, Value::int32(3)), Ok(true));
        assert_eq!(indices(agent, scope, array), [0, 1, 2]);

        let key = length_key(agent, scope).expect("Speicher");
        let one = agent.heap().local(scope, Value::int32(1));
        assert_eq!(
            operations::set(agent, scope, array, key, one, true),
            Err(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn non_configurable_elements_stop_shrinking() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 0);
        fill(agent, scope, array, 6);
        let three = agent.heap().local(scope, Value::int32(3));
        let fixed = PropertyDescriptor::data(three, true, true, false);
        assert_eq!(
            object::define_own_property(agent, scope, array, PropertyKey::Index(3), &fixed),
            Ok(true)
        );
        assert_eq!(set_length(agent, scope, array, Value::int32(1)), Ok(false));
        assert_eq!(length(agent, scope, array), Some(4.0));
        assert_eq!(indices(agent, scope, array), [0, 1, 2, 3]);
        assert_eq!(length_descriptor(agent, scope, array).writable, Some(true));

        // Mit gewünschtem Schreibschutz bleibt die Länge danach fest.
        let one = agent.heap().local(scope, Value::int32(1));
        let shrink_and_fix = PropertyDescriptor {
            value: Some(one),
            writable: Some(false),
            ..PropertyDescriptor::default()
        };
        assert_eq!(
            define_length(agent, scope, array, &shrink_and_fix),
            Ok(false)
        );
        assert_eq!(length(agent, scope, array), Some(4.0));
        assert_eq!(length_descriptor(agent, scope, array).writable, Some(false));
    });
}

#[test]
fn shrinking_can_fix_the_length() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 0);
        fill(agent, scope, array, 5);
        let two = agent.heap().local(scope, Value::int32(2));
        let descriptor = PropertyDescriptor {
            value: Some(two),
            writable: Some(false),
            ..PropertyDescriptor::default()
        };
        assert_eq!(define_length(agent, scope, array, &descriptor), Ok(true));
        assert_eq!(length(agent, scope, array), Some(2.0));
        assert_eq!(indices(agent, scope, array), [0, 1]);
        assert_eq!(length_descriptor(agent, scope, array).writable, Some(false));
    });
}

#[test]
fn other_length_attributes_stay_fixed() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 0);
        let nothing = agent.heap().local(scope, Value::UNDEFINED);
        let refused = [
            PropertyDescriptor {
                enumerable: Some(true),
                ..PropertyDescriptor::default()
            },
            PropertyDescriptor {
                configurable: Some(true),
                ..PropertyDescriptor::default()
            },
            PropertyDescriptor {
                get: Some(nothing),
                ..PropertyDescriptor::default()
            },
        ];
        for descriptor in &refused {
            assert_eq!(define_length(agent, scope, array, descriptor), Ok(false));
        }
        let writable = PropertyDescriptor {
            writable: Some(true),
            ..PropertyDescriptor::default()
        };
        assert_eq!(define_length(agent, scope, array, &writable), Ok(true));
        let key = length_key(agent, scope).expect("Speicher");
        assert_eq!(object::delete(agent, scope, array, key), Ok(false));
    });
}

#[test]
fn arrays_serve_as_prototypes() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let prototype = new_array(agent, scope, 0);
        assert_eq!(put(agent, scope, prototype, 0, 1.0), Ok(true));
        let object =
            object::ordinary_object_create(agent, scope, Some(prototype)).expect("Speicher");
        assert_eq!(length(agent, scope, object), Some(1.0));

        // Elemente und die geerbte Länge landen beim Empfänger.
        assert_eq!(put(agent, scope, object, 5, 2.0), Ok(true));
        let key = length_key(agent, scope).expect("Speicher");
        let seven = agent.heap().local(scope, Value::int32(7));
        assert_eq!(
            operations::set(agent, scope, object, key, seven, true),
            Ok(())
        );
        assert_eq!(length(agent, scope, object), Some(7.0));
        assert_eq!(length(agent, scope, prototype), Some(1.0));
        assert_eq!(indices(agent, scope, prototype), [0]);
        let own = object::get_own_property(agent, scope, object, key)
            .expect("kein Fehler")
            .expect("eigene Länge");
        assert_eq!(own.enumerable, Some(true));
    });
}

#[test]
fn arrays_survive_collection_before_every_allocation() {
    let mut agent = agent_with(true);
    agent.scope(|agent, scope| {
        let array = new_array(agent, scope, 0);
        for index in 0..40u32 {
            let text = agent
                .heap_mut()
                .alloc_string(scope, &units(&format!("Element {index}")))
                .expect("Speicher");
            assert_eq!(
                operations::create_data_property(
                    agent,
                    scope,
                    array,
                    PropertyKey::Index(index),
                    text
                ),
                Ok(true)
            );
        }
        assert_eq!(put(agent, scope, array, 100_000, 1.0), Ok(true));
        assert_eq!(set_length(agent, scope, array, Value::int32(10)), Ok(true));
        agent.heap_mut().collect();

        assert_eq!(length(agent, scope, array), Some(10.0));
        assert_eq!(indices(agent, scope, array), (0..10).collect::<Vec<_>>());
        for index in 0..10u32 {
            let value = object::get(agent, scope, array, PropertyKey::Index(index), array)
                .expect("kein Fehler");
            let Kind::String(text) = agent.heap().get(value).kind() else {
                panic!("keine Zeichenkette bei {index}");
            };
            assert_eq!(
                agent.heap().string_units(text),
                units(&format!("Element {index}"))
            );
        }
        assert!(agent.heap().collections() > 40);
    });
}
