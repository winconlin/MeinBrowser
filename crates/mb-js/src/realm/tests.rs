use super::*;
use crate::agent::{Abrupt, Exception};
use crate::heap::Options;
use crate::object::get_prototype_of;

fn checked() -> Agent {
    Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher")
}

fn key<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> PropertyKey<'s> {
    agent.key_from_units(scope, &units(text)).expect("Speicher")
}

fn thrown(agent: &mut Agent, scope: &Scope) -> Option<ErrorKind> {
    match agent.take_exception(scope) {
        Some(Exception::Error(kind)) => Some(kind),
        _ => None,
    }
}

fn bits(agent: &Agent, local: Local<'_>) -> u64 {
    agent.heap().get(local).bits()
}

#[test]
fn the_global_object_has_the_value_properties() {
    let mut agent = checked();
    let realm = agent.create_realm().expect("Speicher");
    agent.scope(|agent, scope| {
        let global = agent.global_object(scope, realm);
        let prototype = agent.object_prototype(scope, realm);
        let actual = get_prototype_of(agent, scope, global)
            .expect("kein Fehler")
            .expect("Prototyp");
        assert_eq!(bits(agent, actual), bits(agent, prototype));
        let none = get_prototype_of(agent, scope, prototype).expect("kein Fehler");
        assert!(none.is_none());

        for (name, writable) in [
            ("globalThis", true),
            ("Infinity", false),
            ("NaN", false),
            ("undefined", false),
        ] {
            let key = key(agent, scope, name);
            let property = object::get_own_property(agent, scope, global, key)
                .expect("kein Fehler")
                .expect("Eigenschaft");
            assert_eq!(property.writable, Some(writable), "{name}");
            assert_eq!(property.enumerable, Some(false), "{name}");
            assert_eq!(property.configurable, Some(writable), "{name}");
            let value = agent.heap().get(property.value.expect("Wert"));
            match name {
                "globalThis" => assert_eq!(value.bits(), agent.heap().get(global).bits()),
                "Infinity" => assert_eq!(value.as_number(), Some(f64::INFINITY)),
                "NaN" => assert!(value.as_number().is_some_and(f64::is_nan)),
                _ => assert!(value.is_undefined()),
            }
            let restricted = agent
                .has_restricted_global_property(scope, realm, key)
                .expect("kein Fehler");
            assert_eq!(restricted, !writable, "{name}");
        }
    });
}

#[test]
fn realms_are_separate() {
    let mut agent = checked();
    let first = agent.create_realm().expect("Speicher");
    let second = agent.create_realm().expect("Speicher");
    assert_ne!(first, second);
    agent.scope(|agent, scope| {
        let a = agent.global_object(scope, first);
        let b = agent.global_object(scope, second);
        assert_ne!(bits(agent, a), bits(agent, b));
        agent
            .create_lexical_binding(scope, first, &units("x"), true)
            .expect("kein Fehler");
        assert!(agent.has_lexical_declaration(first, &units("x")));
        assert!(!agent.has_lexical_declaration(second, &units("x")));
    });
}

#[test]
fn lexical_bindings_have_a_temporal_dead_zone() {
    let mut agent = checked();
    let realm = agent.create_realm().expect("Speicher");
    agent.scope(|agent, scope| {
        let name = units("x");
        let key = key(agent, scope, "x");
        agent
            .create_lexical_binding(scope, realm, &name, true)
            .expect("kein Fehler");
        assert_eq!(
            agent.create_lexical_binding(scope, realm, &name, true),
            Err(Abrupt::Throw)
        );
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));

        let reference = agent
            .resolve_global(scope, realm, &name, key)
            .expect("kein Fehler");
        assert_eq!(reference, GlobalReference::Global);
        let read = agent.get_global(scope, realm, reference, &name, key, false);
        assert_eq!(read.map(|_| ()), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::ReferenceError));
        let one = agent.heap().local(scope, Value::int32(1));
        let write = agent.put_global(scope, realm, reference, &name, key, one, false);
        assert_eq!(write, Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::ReferenceError));

        agent.initialize_lexical_binding(realm, &name, one);
        let value = agent
            .get_global(scope, realm, reference, &name, key, false)
            .expect("kein Fehler");
        assert_eq!(agent.heap().get(value).as_int32(), Some(1));
        let two = agent.heap().local(scope, Value::int32(2));
        agent
            .put_global(scope, realm, reference, &name, key, two, true)
            .expect("kein Fehler");
        let value = agent
            .get_global(scope, realm, reference, &name, key, true)
            .expect("kein Fehler");
        assert_eq!(agent.heap().get(value).as_int32(), Some(2));

        // Lexikalische Bindungen lassen sich nicht löschen und liegen
        // nicht im globalen Objekt.
        assert_eq!(agent.delete_global(scope, realm, &name, key), Ok(false));
        let global = agent.global_object(scope, realm);
        assert_eq!(has_own_property(agent, scope, global, key), Ok(false));
    });
}

#[test]
fn constants_reject_assignment() {
    let mut agent = checked();
    let realm = agent.create_realm().expect("Speicher");
    agent.scope(|agent, scope| {
        let name = units("c");
        let key = key(agent, scope, "c");
        agent
            .create_lexical_binding(scope, realm, &name, false)
            .expect("kein Fehler");
        let one = agent.heap().local(scope, Value::int32(1));
        agent.initialize_lexical_binding(realm, &name, one);
        let reference = agent
            .resolve_global(scope, realm, &name, key)
            .expect("kein Fehler");
        // Auch ohne strikten Modus, denn die Bindung ist strikt.
        let write = agent.put_global(scope, realm, reference, &name, key, one, false);
        assert_eq!(write, Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
    });
}

#[test]
fn var_bindings_are_properties_of_the_global_object() {
    let mut agent = checked();
    let realm = agent.create_realm().expect("Speicher");
    agent.scope(|agent, scope| {
        let name = units("v");
        let key = key(agent, scope, "v");
        assert_eq!(agent.can_declare_global_var(scope, realm, key), Ok(true));
        agent
            .create_global_var_binding(scope, realm, key, false)
            .expect("kein Fehler");
        let global = agent.global_object(scope, realm);
        let property = object::get_own_property(agent, scope, global, key)
            .expect("kein Fehler")
            .expect("Eigenschaft");
        assert_eq!(property.writable, Some(true));
        assert_eq!(property.enumerable, Some(true));
        assert_eq!(property.configurable, Some(false));
        assert!(
            agent
                .heap()
                .get(property.value.expect("Wert"))
                .is_undefined()
        );

        // Ein zweites Mal ändert nichts.
        let one = agent.heap().local(scope, Value::int32(1));
        let reference = agent
            .resolve_global(scope, realm, &name, key)
            .expect("kein Fehler");
        agent
            .put_global(scope, realm, reference, &name, key, one, true)
            .expect("kein Fehler");
        agent
            .create_global_var_binding(scope, realm, key, false)
            .expect("kein Fehler");
        let value = agent
            .get_global(scope, realm, reference, &name, key, true)
            .expect("kein Fehler");
        assert_eq!(agent.heap().get(value).as_int32(), Some(1));

        // Nicht löschbar.
        assert_eq!(agent.delete_global(scope, realm, &name, key), Ok(false));
        assert_eq!(has_own_property(agent, scope, global, key), Ok(true));
    });
}

#[test]
fn unresolvable_references_depend_on_strictness() {
    let mut agent = checked();
    let realm = agent.create_realm().expect("Speicher");
    agent.scope(|agent, scope| {
        let name = units("u");
        let key = key(agent, scope, "u");
        let reference = agent
            .resolve_global(scope, realm, &name, key)
            .expect("kein Fehler");
        assert_eq!(reference, GlobalReference::Unresolvable);
        let read = agent.get_global(scope, realm, reference, &name, key, false);
        assert_eq!(read.map(|_| ()), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::ReferenceError));

        let one = agent.heap().local(scope, Value::int32(1));
        let strict = agent.put_global(scope, realm, reference, &name, key, one, true);
        assert_eq!(strict, Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::ReferenceError));
        let global = agent.global_object(scope, realm);
        assert_eq!(has_own_property(agent, scope, global, key), Ok(false));

        // Ohne strikten Modus entsteht eine Eigenschaft, die sich löschen
        // lässt.
        agent
            .put_global(scope, realm, reference, &name, key, one, false)
            .expect("kein Fehler");
        assert_eq!(has_own_property(agent, scope, global, key), Ok(true));
        assert_eq!(agent.delete_global(scope, realm, &name, key), Ok(true));
        assert_eq!(has_own_property(agent, scope, global, key), Ok(false));

        // Eine Referenz, deren Eigenschaft danach verschwand.
        let reference = GlobalReference::Global;
        let lenient = agent
            .get_global(scope, realm, reference, &name, key, false)
            .expect("kein Fehler");
        assert!(agent.heap().get(lenient).is_undefined());
        let strict = agent.get_global(scope, realm, reference, &name, key, true);
        assert_eq!(strict.map(|_| ()), Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::ReferenceError));
        let strict = agent.put_global(scope, realm, reference, &name, key, one, true);
        assert_eq!(strict, Err(Abrupt::Throw));
        assert_eq!(thrown(agent, scope), Some(ErrorKind::ReferenceError));
    });
}

#[test]
fn reference_numbers_round_trip() {
    for reference in [GlobalReference::Unresolvable, GlobalReference::Global] {
        assert_eq!(
            GlobalReference::from_number(f64::from(reference.number())),
            reference
        );
    }
    assert_eq!(
        GlobalReference::from_number(f64::NAN),
        GlobalReference::Unresolvable
    );
}
