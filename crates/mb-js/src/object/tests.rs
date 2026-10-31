use super::*;
use crate::agent::Exception;
use crate::heap::Options;
use crate::key::{MAX_ARRAY_INDEX, same_key};
use crate::operations::{
    create_data_property_or_throw, define_property_or_throw, delete_property_or_throw,
    has_own_property,
};

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

fn stressed() -> Agent {
    Agent::with_options(Options {
        stress: true,
        verify: true,
        ..Options::default()
    })
    .expect("Speicher")
}

fn key<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> PropertyKey<'s> {
    agent.key_from_units(scope, &units(text)).expect("Speicher")
}

fn number<'s>(agent: &Agent, scope: &'s Scope, value: f64) -> Local<'s> {
    agent.heap().local(scope, Value::number(value))
}

fn undefined<'s>(agent: &Agent, scope: &'s Scope) -> Local<'s> {
    agent.heap().local(scope, Value::UNDEFINED)
}

fn number_of(agent: &Agent, local: Local<'_>) -> Option<f64> {
    agent.heap().get(local).as_number()
}

fn text_of(agent: &Agent, local: Local<'_>) -> Vec<u16> {
    match agent.heap().get(local).kind() {
        Kind::String(string) => agent.heap().string_units(string),
        kind => panic!("keine Zeichenkette: {kind:?}"),
    }
}

fn new_object<'s>(agent: &mut Agent, scope: &'s Scope, prototype: Option<Local<'_>>) -> Local<'s> {
    ordinary_object_create(agent, scope, prototype).expect("Speicher")
}

fn put_key(agent: &mut Agent, scope: &Scope, object: Local<'_>, key: PropertyKey<'_>, value: f64) {
    let value = number(agent, scope, value);
    assert_eq!(
        create_data_property(agent, scope, object, key, value),
        Ok(true)
    );
}

fn put(agent: &mut Agent, scope: &Scope, object: Local<'_>, name: &str, value: f64) {
    let key = key(agent, scope, name);
    put_key(agent, scope, object, key, value);
}

fn read_key(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Option<f64> {
    let value = get(agent, scope, object, key, object).expect("kein Fehler");
    number_of(agent, value)
}

fn read(agent: &mut Agent, scope: &Scope, object: Local<'_>, name: &str) -> Option<f64> {
    let key = key(agent, scope, name);
    read_key(agent, scope, object, key)
}

fn define(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: &str,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<bool> {
    let key = key(agent, scope, name);
    define_own_property(agent, scope, object, key, descriptor)
}

fn own<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> PropertyDescriptor<'s> {
    get_own_property(agent, scope, object, key)
        .expect("kein Fehler")
        .expect("eigene Eigenschaft")
}

fn indices(agent: &mut Agent, scope: &Scope, object: Local<'_>) -> Vec<u32> {
    own_property_keys(agent, scope, object)
        .expect("kein Fehler")
        .into_iter()
        .filter_map(|key| match key {
            PropertyKey::Index(index) => Some(index),
            _ => None,
        })
        .collect()
}

fn thrown(agent: &mut Agent, scope: &Scope) -> Option<ErrorKind> {
    match agent.take_exception(scope) {
        Some(Exception::Error(kind)) => Some(kind),
        _ => None,
    }
}

fn shape_of(agent: &Agent, object: Local<'_>) -> Option<u32> {
    let heap = agent.heap();
    object_of(heap, object)
        .and_then(|object| heap.object_shape(object))
        .map(Ref::offset)
}

fn same(agent: &Agent, a: Local<'_>, b: Local<'_>) -> bool {
    agent.heap().get(a).bits() == agent.heap().get(b).bits()
}

#[test]
fn data_properties_are_created_read_and_written() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        put(agent, scope, object, "a", 1.0);
        assert_eq!(read(agent, scope, object, "a"), Some(1.0));

        let a = key(agent, scope, "a");
        let two = number(agent, scope, 2.0);
        assert_eq!(set(agent, scope, object, a, two, object), Ok(true));
        assert_eq!(read(agent, scope, object, "a"), Some(2.0));
        let descriptor = own(agent, scope, object, a);
        assert!(descriptor.is_data());
        assert_eq!(
            descriptor.value.and_then(|value| number_of(agent, value)),
            Some(2.0)
        );
        assert_eq!(
            (
                descriptor.writable,
                descriptor.enumerable,
                descriptor.configurable
            ),
            (Some(true), Some(true), Some(true))
        );

        // [[Set]] legt eine fehlende Eigenschaft an.
        let b = key(agent, scope, "b");
        let three = number(agent, scope, 3.0);
        assert_eq!(set(agent, scope, object, b, three, object), Ok(true));
        assert_eq!(read(agent, scope, object, "b"), Some(3.0));
        assert_eq!(own(agent, scope, object, b).configurable, Some(true));

        let missing = key(agent, scope, "fehlt");
        let value = get(agent, scope, object, missing, object).expect("kein Fehler");
        assert!(agent.heap().get(value).is_undefined());
        assert_eq!(has_property(agent, object, missing), Ok(false));
        assert_eq!(has_own_property(agent, scope, object, missing), Ok(false));
        assert_eq!(has_own_property(agent, scope, object, a), Ok(true));
    });
}

#[test]
fn defaults_fill_missing_fields() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        let empty = PropertyDescriptor::default();
        assert_eq!(define(agent, scope, object, "leer", &empty), Ok(true));
        let leer = key(agent, scope, "leer");
        let descriptor = own(agent, scope, object, leer);
        assert!(descriptor.is_data());
        assert!(
            descriptor
                .value
                .is_some_and(|value| agent.heap().get(value).is_undefined())
        );
        assert_eq!(
            (
                descriptor.writable,
                descriptor.enumerable,
                descriptor.configurable
            ),
            (Some(false), Some(false), Some(false))
        );
    });
}

#[test]
fn objects_with_the_same_keys_share_their_shape() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let first = new_object(agent, scope, None);
        put(agent, scope, first, "a", 1.0);
        put(agent, scope, first, "b", 2.0);
        let second = new_object(agent, scope, None);
        put(agent, scope, second, "a", 3.0);
        put(agent, scope, second, "b", 4.0);
        let reversed = new_object(agent, scope, None);
        put(agent, scope, reversed, "b", 5.0);
        put(agent, scope, reversed, "a", 6.0);
        let fixed = new_object(agent, scope, None);
        let seven = number(agent, scope, 7.0);
        let descriptor = PropertyDescriptor::data(seven, false, true, true);
        assert_eq!(define(agent, scope, fixed, "a", &descriptor), Ok(true));
        put(agent, scope, fixed, "b", 8.0);

        assert_eq!(shape_of(agent, first), shape_of(agent, second));
        assert_ne!(shape_of(agent, first), shape_of(agent, reversed));
        assert_ne!(shape_of(agent, first), shape_of(agent, fixed));

        agent.heap_mut().collect();
        assert_eq!(shape_of(agent, first), shape_of(agent, second));
        assert_eq!(read(agent, scope, second, "a"), Some(3.0));
        assert_eq!(read(agent, scope, second, "b"), Some(4.0));
        assert_eq!(read(agent, scope, reversed, "a"), Some(6.0));
        assert_eq!(read(agent, scope, fixed, "a"), Some(7.0));

        // Wer später dieselben Schlüssel bekommt, findet die Gestalt wieder.
        let third = new_object(agent, scope, None);
        put(agent, scope, third, "a", 9.0);
        put(agent, scope, third, "b", 10.0);
        assert_eq!(shape_of(agent, third), shape_of(agent, first));
    });
}

#[test]
fn unused_shapes_are_freed() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        // Atome bleiben, solange der Agent lebt.
        key(agent, scope, "a");
        key(agent, scope, "b");
        agent.heap_mut().collect();
        let base = agent.heap().used_bytes();
        let mut rounds = Vec::new();
        for _ in 0..3 {
            let alive = agent.scope(|agent, scope| {
                let object = new_object(agent, scope, None);
                put(agent, scope, object, "a", 1.0);
                put(agent, scope, object, "b", 2.0);
                agent.heap_mut().collect();
                agent.heap().used_bytes()
            });
            agent.heap_mut().collect();
            let dead = agent.heap().used_bytes();
            rounds.push((alive.checked_sub(base), dead.checked_sub(base)));
        }
        // Solange das Objekt lebt: das Objekt (7 Wörter), die Gestalten
        // für a und für a, b (je 6) und die Übergangstabellen der leeren
        // Gestalt und der Gestalt für a (je 7). Danach bleibt nur die
        // Tabelle der leeren Gestalt; ihr geleerter Eintrag wird in der
        // nächsten Runde wieder belegt.
        let alive = 8 * (7 + 6 + 6 + 7 + 7);
        assert_eq!(rounds, [(Some(alive), Some(8 * 7)); 3]);
    });
}

#[test]
fn transition_tables_grow() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let names: Vec<String> = (0..10).map(|index| format!("k{index}")).collect();
        let objects: Vec<Local<'_>> = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let object = new_object(agent, scope, None);
                put(
                    agent,
                    scope,
                    object,
                    name,
                    f64::from(u8::try_from(index).unwrap_or(0)),
                );
                object
            })
            .collect();
        agent.heap_mut().collect();
        for (index, (object, name)) in objects.iter().zip(&names).enumerate() {
            let again = new_object(agent, scope, None);
            put(agent, scope, again, name, 0.0);
            assert_eq!(shape_of(agent, again), shape_of(agent, *object));
            let expected = f64::from(u8::try_from(index).unwrap_or(0));
            assert_eq!(read(agent, scope, *object, name), Some(expected));
        }
    });
}

#[test]
fn own_keys_follow_the_specified_order() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        let first = agent.new_symbol(scope, None).expect("Speicher");
        let second = agent.new_symbol(scope, None).expect("Speicher");
        put(agent, scope, object, "b", 1.0);
        put_key(agent, scope, object, PropertyKey::Symbol(first), 2.0);
        put(agent, scope, object, "10", 3.0);
        put(agent, scope, object, "a", 4.0);
        put(agent, scope, object, "0", 5.0);
        put_key(agent, scope, object, PropertyKey::Symbol(second), 6.0);
        put(agent, scope, object, "4294967295", 7.0);
        put(agent, scope, object, "c", 8.0);
        // Löschen und neu Anlegen stellt ans Ende, neue Attribute nicht.
        let b = key(agent, scope, "b");
        assert_eq!(delete(agent, scope, object, b), Ok(true));
        put(agent, scope, object, "b", 9.0);
        let read_only = PropertyDescriptor {
            writable: Some(false),
            ..PropertyDescriptor::default()
        };
        assert_eq!(define(agent, scope, object, "a", &read_only), Ok(true));

        let keys = own_property_keys(agent, scope, object).expect("kein Fehler");
        let expected = [
            PropertyKey::Index(0),
            PropertyKey::Index(10),
            key(agent, scope, "a"),
            key(agent, scope, "4294967295"),
            key(agent, scope, "c"),
            key(agent, scope, "b"),
            PropertyKey::Symbol(first),
            PropertyKey::Symbol(second),
        ];
        assert_eq!(keys.len(), expected.len());
        for (found, expected) in keys.iter().zip(&expected) {
            assert!(
                same_key(agent.heap(), *found, *expected),
                "{found:?} statt {expected:?}"
            );
        }
        for (name, value) in [
            ("0", 5.0),
            ("10", 3.0),
            ("a", 4.0),
            ("4294967295", 7.0),
            ("c", 8.0),
            ("b", 9.0),
        ] {
            assert_eq!(read(agent, scope, object, name), Some(value), "{name}");
        }
        assert_eq!(
            read_key(agent, scope, object, PropertyKey::Symbol(first)),
            Some(2.0)
        );
        assert_eq!(
            read_key(agent, scope, object, PropertyKey::Symbol(second)),
            Some(6.0)
        );
    });
}

#[test]
fn deleting_moves_later_properties_up() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        // Mehr Eigenschaften, als eingebettet Platz haben.
        let names = ["a", "b", "c", "d", "e", "f"];
        for (value, name) in (1..).zip(names) {
            put(agent, scope, object, name, f64::from(value));
        }
        let b = key(agent, scope, "b");
        assert_eq!(delete(agent, scope, object, b), Ok(true));
        assert_eq!(delete(agent, scope, object, b), Ok(true));
        put(agent, scope, object, "g", 7.0);
        let a = key(agent, scope, "a");
        assert_eq!(delete(agent, scope, object, a), Ok(true));
        agent.heap_mut().collect();

        for (name, value) in [
            ("a", None),
            ("b", None),
            ("c", Some(3.0)),
            ("d", Some(4.0)),
            ("e", Some(5.0)),
            ("f", Some(6.0)),
            ("g", Some(7.0)),
        ] {
            assert_eq!(read(agent, scope, object, name), value, "{name}");
        }
        let keys = own_property_keys(agent, scope, object).expect("kein Fehler");
        let expected: Vec<PropertyKey<'_>> = ["c", "d", "e", "f", "g"]
            .into_iter()
            .map(|name| key(agent, scope, name))
            .collect();
        assert_eq!(keys.len(), expected.len());
        assert!(keys.iter().zip(&expected).all(|(found, expected)| same_key(
            agent.heap(),
            *found,
            *expected
        )));
    });
}

#[test]
fn elements_grow_dense_and_turn_sparse() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        for index in 0..6 {
            put_key(
                agent,
                scope,
                object,
                PropertyKey::Index(index),
                f64::from(index * 10),
            );
        }
        // Eine kleine Lücke bleibt dicht.
        put_key(agent, scope, object, PropertyKey::Index(1000), 1.0);
        assert_eq!(
            has_property(agent, object, PropertyKey::Index(500)),
            Ok(false)
        );
        assert_eq!(indices(agent, scope, object), [0, 1, 2, 3, 4, 5, 1000]);
        // Ein Index weit hinten macht die Elemente dünn besetzt.
        put_key(agent, scope, object, PropertyKey::Index(100_000), 2.0);
        put_key(
            agent,
            scope,
            object,
            PropertyKey::Index(MAX_ARRAY_INDEX),
            3.0,
        );
        put_key(agent, scope, object, PropertyKey::Index(7), 4.0);
        let three = PropertyKey::Index(3);
        assert_eq!(delete(agent, scope, object, three), Ok(true));
        agent.heap_mut().collect();

        assert_eq!(
            indices(agent, scope, object),
            [0, 1, 2, 4, 5, 7, 1000, 100_000, MAX_ARRAY_INDEX]
        );
        for (index, value) in [
            (0, 0.0),
            (2, 20.0),
            (5, 50.0),
            (7, 4.0),
            (1000, 1.0),
            (100_000, 2.0),
            (MAX_ARRAY_INDEX, 3.0),
        ] {
            assert_eq!(
                read_key(agent, scope, object, PropertyKey::Index(index)),
                Some(value),
                "{index}"
            );
        }
        assert_eq!(read_key(agent, scope, object, three), None);
        let five = own(agent, scope, object, PropertyKey::Index(5));
        assert_eq!(
            (five.writable, five.enumerable, five.configurable),
            (Some(true), Some(true), Some(true))
        );
    });
}

#[test]
fn elements_with_other_attributes_turn_sparse() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        for index in 0..3 {
            put_key(
                agent,
                scope,
                object,
                PropertyKey::Index(index),
                f64::from(index),
            );
        }
        let hidden = PropertyDescriptor {
            enumerable: Some(false),
            ..PropertyDescriptor::default()
        };
        let one = PropertyKey::Index(1);
        assert_eq!(
            define_own_property(agent, scope, object, one, &hidden),
            Ok(true)
        );
        let read_only = PropertyDescriptor {
            writable: Some(false),
            ..PropertyDescriptor::default()
        };
        assert_eq!(
            define_own_property(agent, scope, object, PropertyKey::Index(2), &read_only),
            Ok(true)
        );
        agent.heap_mut().collect();

        assert_eq!(indices(agent, scope, object), [0, 1, 2]);
        let descriptor = own(agent, scope, object, one);
        assert_eq!(
            (
                descriptor.writable,
                descriptor.enumerable,
                descriptor.configurable
            ),
            (Some(true), Some(false), Some(true))
        );
        assert_eq!(
            own(agent, scope, object, PropertyKey::Index(0)).enumerable,
            Some(true)
        );
        let nine = number(agent, scope, 9.0);
        assert_eq!(
            set(agent, scope, object, PropertyKey::Index(2), nine, object),
            Ok(false)
        );
        assert_eq!(set(agent, scope, object, one, nine, object), Ok(true));
        assert_eq!(read_key(agent, scope, object, one), Some(9.0));
        assert_eq!(
            read_key(agent, scope, object, PropertyKey::Index(2)),
            Some(2.0)
        );
    });
}

#[test]
fn lookups_follow_the_prototype_chain() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let prototype = new_object(agent, scope, None);
        put(agent, scope, prototype, "x", 1.0);
        let two = number(agent, scope, 2.0);
        let read_only = PropertyDescriptor::data(two, false, true, true);
        assert_eq!(define(agent, scope, prototype, "y", &read_only), Ok(true));
        let nothing = undefined(agent, scope);
        let accessor = PropertyDescriptor::accessor(nothing, nothing, true, true);
        assert_eq!(define(agent, scope, prototype, "z", &accessor), Ok(true));
        put_key(agent, scope, prototype, PropertyKey::Index(0), 9.0);
        let object = new_object(agent, scope, Some(prototype));

        let x = key(agent, scope, "x");
        assert_eq!(read_key(agent, scope, object, x), Some(1.0));
        assert_eq!(
            read_key(agent, scope, object, PropertyKey::Index(0)),
            Some(9.0)
        );
        assert_eq!(has_property(agent, object, x), Ok(true));
        assert_eq!(has_own_property(agent, scope, object, x), Ok(false));
        let found = get_prototype_of(agent, scope, object)
            .expect("kein Fehler")
            .expect("Prototyp");
        assert!(same(agent, found, prototype));

        // Schreiben legt die Eigenschaft beim Empfänger an.
        let five = number(agent, scope, 5.0);
        assert_eq!(set(agent, scope, object, x, five, object), Ok(true));
        assert_eq!(read_key(agent, scope, object, x), Some(5.0));
        assert_eq!(read_key(agent, scope, prototype, x), Some(1.0));

        // Eine schreibgeschützte Eigenschaft im Prototyp verhindert das.
        let y = key(agent, scope, "y");
        assert_eq!(set(agent, scope, object, y, five, object), Ok(false));
        assert_eq!(has_own_property(agent, scope, object, y), Ok(false));

        // Ebenso eine Zugriffseigenschaft ohne Setter.
        let z = key(agent, scope, "z");
        assert_eq!(set(agent, scope, object, z, five, object), Ok(false));
        assert_eq!(read_key(agent, scope, object, z), None);
        assert_eq!(
            crate::operations::set(agent, scope, object, z, five, true),
            Err(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn receivers_decide_where_values_land() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let holder = new_object(agent, scope, None);
        put(agent, scope, holder, "p", 1.0);
        let receiver = new_object(agent, scope, None);
        let p = key(agent, scope, "p");
        let two = number(agent, scope, 2.0);
        assert_eq!(set(agent, scope, holder, p, two, receiver), Ok(true));
        assert_eq!(read_key(agent, scope, holder, p), Some(1.0));
        assert_eq!(read_key(agent, scope, receiver, p), Some(2.0));

        // Ein Empfänger mit eigener schreibgeschützter oder
        // Zugriffseigenschaft lehnt ab.
        let fixed = new_object(agent, scope, None);
        let three = number(agent, scope, 3.0);
        let read_only = PropertyDescriptor::data(three, false, true, true);
        assert_eq!(define(agent, scope, fixed, "p", &read_only), Ok(true));
        assert_eq!(set(agent, scope, holder, p, two, fixed), Ok(false));
        let nothing = undefined(agent, scope);
        let accessor = PropertyDescriptor::accessor(nothing, nothing, true, true);
        let with_accessor = new_object(agent, scope, None);
        assert_eq!(
            define(agent, scope, with_accessor, "p", &accessor),
            Ok(true)
        );
        assert_eq!(set(agent, scope, holder, p, two, with_accessor), Ok(false));

        // Ein Empfänger, der kein Objekt ist, ebenso.
        assert_eq!(set(agent, scope, holder, p, two, three), Ok(false));
    });
}

#[test]
fn accessors_need_callable_functions() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        // Gewöhnliche Objekte sind nicht aufrufbar.
        let not_callable = new_object(agent, scope, None);
        let nothing = undefined(agent, scope);
        let getter = PropertyDescriptor::accessor(not_callable, nothing, true, true);
        assert_eq!(define(agent, scope, object, "g", &getter), Ok(true));
        let setter = PropertyDescriptor::accessor(nothing, not_callable, true, true);
        assert_eq!(define(agent, scope, object, "s", &setter), Ok(true));

        let g = key(agent, scope, "g");
        assert_eq!(
            get(agent, scope, object, g, object).err(),
            Some(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        let s = key(agent, scope, "s");
        let one = number(agent, scope, 1.0);
        assert_eq!(
            set(agent, scope, object, s, one, object),
            Err(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn long_prototype_chains_need_no_deep_stack() {
    const LENGTH: usize = 100_000;
    let worker = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut agent = checked();
            agent.scope(|agent, scope| {
                let root = new_object(agent, scope, None);
                put(agent, scope, root, "x", 1.0);
                put_key(agent, scope, root, PropertyKey::Index(3), 2.0);
                let head = agent.heap().local(scope, agent.heap().get(root));
                for _ in 1..LENGTH {
                    agent.scope(|agent, inner| {
                        let next = new_object(agent, inner, Some(head));
                        let heap = agent.heap();
                        heap.set(head, heap.get(next));
                    });
                }
                agent.heap_mut().collect();

                assert_eq!(read(agent, scope, head, "x"), Some(1.0));
                assert_eq!(
                    read_key(agent, scope, head, PropertyKey::Index(3)),
                    Some(2.0)
                );
                let missing = key(agent, scope, "y");
                assert_eq!(has_property(agent, head, missing), Ok(false));
                assert_eq!(read_key(agent, scope, head, missing), None);
                let three = number(agent, scope, 3.0);
                assert_eq!(set(agent, scope, head, missing, three, head), Ok(true));
                assert_eq!(has_own_property(agent, scope, head, missing), Ok(true));
                assert_eq!(has_own_property(agent, scope, root, missing), Ok(false));
                // Der Zyklus über die ganze Kette fällt auf.
                assert_eq!(set_prototype_of(agent, root, Some(head)), Ok(false));
                let length = agent.scope(|agent, inner| {
                    let cursor = agent.heap().local(inner, agent.heap().get(head));
                    let mut length = 1;
                    while let Some(next) =
                        get_prototype_of(agent, inner, cursor).expect("kein Fehler")
                    {
                        let heap = agent.heap();
                        heap.set(cursor, heap.get(next));
                        length += 1;
                    }
                    length
                });
                assert_eq!(length, LENGTH);
            });
        })
        .expect("Faden startet");
    assert!(worker.join().is_ok());
}

#[test]
fn prototypes_change_without_cycles() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let a = new_object(agent, scope, None);
        let b = new_object(agent, scope, Some(a));
        let c = new_object(agent, scope, None);
        assert_eq!(set_prototype_of(agent, a, Some(b)), Ok(false));
        assert_eq!(set_prototype_of(agent, a, Some(a)), Ok(false));
        assert_eq!(set_prototype_of(agent, b, Some(c)), Ok(true));
        let found = get_prototype_of(agent, scope, b)
            .expect("kein Fehler")
            .expect("Prototyp");
        assert!(same(agent, found, c));
        assert_eq!(set_prototype_of(agent, a, Some(b)), Ok(true));
        assert_eq!(set_prototype_of(agent, b, None), Ok(true));
        assert!(
            get_prototype_of(agent, scope, b)
                .expect("kein Fehler")
                .is_none()
        );

        // Ohne Erweiterbarkeit bleibt der Prototyp, derselbe geht noch.
        assert_eq!(prevent_extensions(agent, b), Ok(true));
        assert_eq!(set_prototype_of(agent, b, Some(c)), Ok(false));
        assert_eq!(set_prototype_of(agent, b, None), Ok(true));

        // Nur Objekte sind Prototypen.
        let one = number(agent, scope, 1.0);
        assert_eq!(set_prototype_of(agent, c, Some(one)), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn non_extensible_objects_take_no_new_properties() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        put(agent, scope, object, "a", 1.0);
        put_key(agent, scope, object, PropertyKey::Index(0), 2.0);
        assert_eq!(is_extensible(agent, object), Ok(true));
        assert_eq!(prevent_extensions(agent, object), Ok(true));
        assert_eq!(prevent_extensions(agent, object), Ok(true));
        assert_eq!(is_extensible(agent, object), Ok(false));

        let b = key(agent, scope, "b");
        let three = number(agent, scope, 3.0);
        assert_eq!(
            create_data_property(agent, scope, object, b, three),
            Ok(false)
        );
        assert_eq!(
            create_data_property(agent, scope, object, PropertyKey::Index(1), three),
            Ok(false)
        );
        assert_eq!(set(agent, scope, object, b, three, object), Ok(false));
        assert_eq!(
            create_data_property_or_throw(agent, scope, object, b, three),
            Err(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));

        // Vorhandene Eigenschaften lassen sich ändern und löschen.
        let a = key(agent, scope, "a");
        assert_eq!(set(agent, scope, object, a, three, object), Ok(true));
        assert_eq!(read_key(agent, scope, object, a), Some(3.0));
        assert_eq!(delete(agent, scope, object, a), Ok(true));
        let keys = own_property_keys(agent, scope, object).expect("kein Fehler");
        assert!(matches!(keys.as_slice(), [PropertyKey::Index(0)]));
    });
}

#[test]
fn fixed_properties_accept_only_the_same_value() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        let nan = number(agent, scope, f64::NAN);
        let zero = number(agent, scope, 0.0);
        let minus_zero = number(agent, scope, -0.0);
        let fixed = |value| PropertyDescriptor::data(value, false, false, false);
        assert_eq!(define(agent, scope, object, "nan", &fixed(nan)), Ok(true));
        assert_eq!(define(agent, scope, object, "zero", &fixed(zero)), Ok(true));

        let value = |value| PropertyDescriptor {
            value: Some(value),
            ..PropertyDescriptor::default()
        };
        let other_nan = number(agent, scope, f64::NAN.copysign(-1.0));
        assert_eq!(
            define(agent, scope, object, "nan", &value(other_nan)),
            Ok(true)
        );
        assert_eq!(
            define(agent, scope, object, "zero", &value(minus_zero)),
            Ok(false)
        );
        assert_eq!(define(agent, scope, object, "zero", &value(zero)), Ok(true));
        assert_eq!(define(agent, scope, object, "zero", &fixed(zero)), Ok(true));
        assert_eq!(
            define(agent, scope, object, "zero", &PropertyDescriptor::default()),
            Ok(true)
        );

        let refused = [
            PropertyDescriptor {
                writable: Some(true),
                ..PropertyDescriptor::default()
            },
            PropertyDescriptor {
                configurable: Some(true),
                ..PropertyDescriptor::default()
            },
            PropertyDescriptor {
                enumerable: Some(true),
                ..PropertyDescriptor::default()
            },
            PropertyDescriptor {
                get: Some(undefined(agent, scope)),
                ..PropertyDescriptor::default()
            },
        ];
        for descriptor in &refused {
            assert_eq!(define(agent, scope, object, "zero", descriptor), Ok(false));
        }
        let same_enumerable = PropertyDescriptor {
            enumerable: Some(false),
            ..PropertyDescriptor::default()
        };
        assert_eq!(
            define(agent, scope, object, "zero", &same_enumerable),
            Ok(true)
        );

        let name = key(agent, scope, "zero");
        assert_eq!(delete(agent, scope, object, name), Ok(false));
        assert_eq!(
            delete_property_or_throw(agent, scope, object, name),
            Err(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        let stored = read_key(agent, scope, object, name).expect("Zahl");
        assert!(stored == 0.0 && stored.is_sign_positive());
        assert!(read(agent, scope, object, "nan").is_some_and(f64::is_nan));
        assert_eq!(
            define_property_or_throw(agent, scope, object, name, &value(minus_zero)),
            Err(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn writable_fixed_properties_still_change_their_value() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        let one = number(agent, scope, 1.0);
        let two = number(agent, scope, 2.0);
        let three = number(agent, scope, 3.0);
        let value = |value| PropertyDescriptor {
            value: Some(value),
            ..PropertyDescriptor::default()
        };
        let writable = |writable| PropertyDescriptor {
            writable: Some(writable),
            ..PropertyDescriptor::default()
        };
        let descriptor = PropertyDescriptor::data(one, true, true, false);
        assert_eq!(define(agent, scope, object, "w", &descriptor), Ok(true));
        assert_eq!(define(agent, scope, object, "w", &value(two)), Ok(true));
        assert_eq!(read(agent, scope, object, "w"), Some(2.0));
        assert_eq!(
            define(agent, scope, object, "w", &writable(false)),
            Ok(true)
        );
        assert_eq!(
            define(agent, scope, object, "w", &writable(true)),
            Ok(false)
        );
        assert_eq!(define(agent, scope, object, "w", &value(three)), Ok(false));
        assert_eq!(define(agent, scope, object, "w", &value(two)), Ok(true));
        assert_eq!(read(agent, scope, object, "w"), Some(2.0));
    });
}

#[test]
fn properties_switch_between_data_and_accessor() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let object = new_object(agent, scope, None);
        let one = number(agent, scope, 1.0);
        let nothing = undefined(agent, scope);
        let data = PropertyDescriptor::data(one, true, false, true);
        assert_eq!(define(agent, scope, object, "p", &data), Ok(true));
        let getter_only = PropertyDescriptor {
            get: Some(nothing),
            ..PropertyDescriptor::default()
        };
        assert_eq!(define(agent, scope, object, "p", &getter_only), Ok(true));
        let p = key(agent, scope, "p");
        let descriptor = own(agent, scope, object, p);
        assert!(descriptor.is_accessor() && !descriptor.is_data());
        assert!(
            descriptor
                .set
                .is_some_and(|setter| agent.heap().get(setter).is_undefined())
        );
        assert_eq!(
            (descriptor.enumerable, descriptor.configurable),
            (Some(false), Some(true))
        );
        assert_eq!(read_key(agent, scope, object, p), None);

        let five = number(agent, scope, 5.0);
        let value_only = PropertyDescriptor {
            value: Some(five),
            ..PropertyDescriptor::default()
        };
        assert_eq!(define(agent, scope, object, "p", &value_only), Ok(true));
        let descriptor = own(agent, scope, object, p);
        assert!(descriptor.is_data() && !descriptor.is_accessor());
        assert_eq!(descriptor.writable, Some(false));
        assert_eq!(
            (descriptor.enumerable, descriptor.configurable),
            (Some(false), Some(true))
        );
        assert_eq!(read_key(agent, scope, object, p), Some(5.0));

        // Elemente ebenso.
        put_key(agent, scope, object, PropertyKey::Index(0), 1.0);
        let accessor = PropertyDescriptor::accessor(nothing, nothing, true, true);
        assert_eq!(
            define_own_property(agent, scope, object, PropertyKey::Index(0), &accessor),
            Ok(true)
        );
        assert!(own(agent, scope, object, PropertyKey::Index(0)).is_accessor());
        assert_eq!(read_key(agent, scope, object, PropertyKey::Index(0)), None);

        // Eine feste Zugriffseigenschaft bleibt, wie sie ist.
        let fixed = PropertyDescriptor::accessor(nothing, nothing, false, false);
        assert_eq!(define(agent, scope, object, "q", &fixed), Ok(true));
        assert_eq!(define(agent, scope, object, "q", &value_only), Ok(false));
        let other = new_object(agent, scope, None);
        let other_getter = PropertyDescriptor {
            get: Some(other),
            ..PropertyDescriptor::default()
        };
        assert_eq!(define(agent, scope, object, "q", &other_getter), Ok(false));
        assert_eq!(define(agent, scope, object, "q", &getter_only), Ok(true));
    });
}

#[test]
fn compatible_descriptors_without_an_object() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let one = number(agent, scope, 1.0);
        let two = number(agent, scope, 2.0);
        let data = PropertyDescriptor::data(one, true, true, true);
        assert_eq!(
            is_compatible_property_descriptor(agent, scope, true, &data, None),
            Ok(true)
        );
        assert_eq!(
            is_compatible_property_descriptor(agent, scope, false, &data, None),
            Ok(false)
        );
        let current = PropertyDescriptor::data(one, false, true, false);
        let change = PropertyDescriptor::data(two, false, true, false);
        assert_eq!(
            is_compatible_property_descriptor(agent, scope, true, &change, Some(&current)),
            Ok(false)
        );
        assert_eq!(
            is_compatible_property_descriptor(agent, scope, true, &current, Some(&current)),
            Ok(true)
        );
    });
}

#[test]
fn values_without_internal_methods_throw() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let one = number(agent, scope, 1.0);
        let a = key(agent, scope, "a");
        assert_eq!(
            get_own_property(agent, scope, one, a).err(),
            Some(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        assert_eq!(has_property(agent, one, a), Err(Abrupt::Throw));
        assert_eq!(get(agent, scope, one, a, one).err(), Some(Abrupt::Throw));
        assert_eq!(
            own_property_keys(agent, scope, one).err(),
            Some(Abrupt::Throw)
        );
        assert_eq!(is_extensible(agent, one), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn objects_survive_collection_before_every_allocation() {
    let mut agent = stressed();
    agent.scope(|agent, scope| {
        let prototype = new_object(agent, scope, None);
        let object = new_object(agent, scope, Some(prototype));
        for index in 0..20u32 {
            let text = agent
                .heap_mut()
                .alloc_string(scope, &units(&format!("Wert {index}")))
                .expect("Speicher");
            let name = key(agent, scope, &format!("p{index}"));
            assert_eq!(
                create_data_property(agent, scope, object, name, text),
                Ok(true)
            );
            put_key(
                agent,
                scope,
                object,
                PropertyKey::Index(index),
                f64::from(index),
            );
        }
        put_key(agent, scope, object, PropertyKey::Index(1_000_000), 1.0);
        let description = agent
            .heap_mut()
            .alloc_string(scope, &units("Symbol"))
            .expect("Speicher");
        let symbol = agent
            .new_symbol(scope, Some(description))
            .expect("Speicher");
        let accessor = PropertyDescriptor::accessor(prototype, prototype, false, true);
        assert_eq!(
            define_own_property(agent, scope, object, PropertyKey::Symbol(symbol), &accessor),
            Ok(true)
        );
        let p3 = key(agent, scope, "p3");
        assert_eq!(delete(agent, scope, object, p3), Ok(true));
        let read_only = PropertyDescriptor {
            writable: Some(false),
            ..PropertyDescriptor::default()
        };
        assert_eq!(define(agent, scope, object, "p5", &read_only), Ok(true));
        put(agent, scope, prototype, "geerbt", 7.0);
        agent.heap_mut().collect();

        for index in (0..20u32).filter(|&index| index != 3) {
            let name = key(agent, scope, &format!("p{index}"));
            let value = get(agent, scope, object, name, object).expect("kein Fehler");
            assert_eq!(text_of(agent, value), units(&format!("Wert {index}")));
            assert_eq!(
                read_key(agent, scope, object, PropertyKey::Index(index)),
                Some(f64::from(index))
            );
        }
        assert_eq!(read(agent, scope, object, "p3"), None);
        let p5 = key(agent, scope, "p5");
        assert_eq!(own(agent, scope, object, p5).writable, Some(false));
        assert_eq!(read(agent, scope, object, "geerbt"), Some(7.0));
        let descriptor = own(agent, scope, object, PropertyKey::Symbol(symbol));
        assert!(
            descriptor
                .get
                .is_some_and(|getter| same(agent, getter, prototype))
        );
        let keys = own_property_keys(agent, scope, object).expect("kein Fehler");
        // 21 Indizes, 19 Namen, ein Symbol.
        assert_eq!(keys.len(), 41);
        assert!(keys.last().is_some_and(|&last| same_key(
            agent.heap(),
            last,
            PropertyKey::Symbol(symbol)
        )));
        assert!(agent.heap().collections() > 100);
    });
}
