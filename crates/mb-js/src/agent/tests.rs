use super::*;
use crate::value::Value;

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

fn bits(agent: &Agent, local: Local<'_>) -> u64 {
    agent.heap().get(local).bits()
}

fn text(agent: &Agent, local: Local<'_>) -> Vec<u16> {
    match agent.heap().get(local).kind() {
        Kind::String(string) => agent.heap().string_units(string),
        kind => panic!("keine Zeichenkette: {kind:?}"),
    }
}

#[test]
fn atoms_are_unique_per_content() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let first = agent.atom(scope, &units("länge")).expect("Speicher");
        let again = agent.atom(scope, &units("länge")).expect("Speicher");
        let other = agent.atom(scope, &units("Länge")).expect("Speicher");
        assert_eq!(bits(agent, first), bits(agent, again));
        assert_ne!(bits(agent, first), bits(agent, other));

        // Eine Zeichenkette mit dem Inhalt eines Atoms wird zu diesem Atom.
        let copy = agent
            .heap_mut()
            .alloc_string(scope, &units("länge"))
            .expect("Speicher");
        assert_ne!(bits(agent, copy), bits(agent, first));
        let atom = agent.atomize(scope, copy);
        assert_eq!(bits(agent, atom), bits(agent, first));

        // Ohne Atom wird die Zeichenkette selbst eines.
        let fresh = agent
            .heap_mut()
            .alloc_string(scope, &units("neu"))
            .expect("Speicher");
        let atom = agent.atomize(scope, fresh);
        assert_eq!(bits(agent, atom), bits(agent, fresh));
        let looked_up = agent.atom(scope, &units("neu")).expect("Speicher");
        assert_eq!(bits(agent, looked_up), bits(agent, fresh));

        // Andere Werte bleiben, wie sie sind.
        let number = agent.heap().local(scope, Value::int32(1));
        let same = agent.atomize(scope, number);
        assert_eq!(bits(agent, same), bits(agent, number));
    });
}

#[test]
fn atoms_outlive_their_scope() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        agent.atom(scope, &units("bleibt")).expect("Speicher");
    });
    agent.heap_mut().collect();
    agent.scope(|agent, scope| {
        let before = agent.heap().used_bytes();
        let atom = agent.atom(scope, &units("bleibt")).expect("Speicher");
        assert_eq!(agent.heap().used_bytes(), before);
        assert_eq!(text(agent, atom), units("bleibt"));
        let Kind::String(string) = agent.heap().get(atom).kind() else {
            panic!("keine Zeichenkette");
        };
        assert!(agent.heap().is_atom(string));
    });
}

#[test]
fn an_exception_is_taken_once() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let value = agent
            .heap_mut()
            .alloc_string(scope, &units("Fehler"))
            .expect("Speicher");
        assert_eq!(agent.throw_value(value), Abrupt::Throw);
        agent.heap_mut().collect();
        match agent.take_exception(scope) {
            Some(Exception::Value(thrown)) => assert_eq!(text(agent, thrown), units("Fehler")),
            other => panic!("unerwartet: {other:?}"),
        }
        assert!(agent.take_exception(scope).is_none());

        assert_eq!(agent.throw_error(ErrorKind::TypeError), Abrupt::Throw);
        assert!(matches!(
            agent.take_exception(scope),
            Some(Exception::Error(ErrorKind::TypeError))
        ));
        assert!(agent.take_exception(scope).is_none());
    });
}

#[test]
fn a_new_exception_releases_the_old_value() {
    let mut agent = checked();
    agent.heap_mut().collect();
    let base = agent.heap().used_bytes();
    agent.scope(|agent, scope| {
        let value = agent
            .heap_mut()
            .alloc_string(scope, &units("alt"))
            .expect("Speicher");
        agent.throw_value(value);
    });
    agent.heap_mut().collect();
    // Kopfwort und ein Wort mit drei Latin-1-Zeichen.
    assert_eq!(agent.heap().used_bytes(), base + 16);
    agent.throw_error(ErrorKind::TypeError);
    agent.heap_mut().collect();
    assert_eq!(agent.heap().used_bytes(), base);
}

#[test]
fn out_of_memory_ends_abruptly() {
    assert_eq!(Abrupt::from(OutOfMemory), Abrupt::OutOfMemory);
    let tiny = Agent::with_options(Options {
        limit: 0,
        ..Options::default()
    });
    assert_eq!(tiny.err(), Some(OutOfMemory));
}

#[test]
fn exhausted_runs_release_interpreter_depth_and_active_quotas() {
    use std::rc::Rc;

    use crate::script::ScriptError;

    let mut agent = checked();
    let realm = agent.create_realm().expect("realm");
    for source in [
        "for (;;) {}",
        "function recurse() { return recurse(); } recurse();",
        "var o = {get x() {for (var i=0;i<512;i++) {}}}; o.x;",
    ] {
        let budget = Rc::new(Budget::new(128));
        let result = agent.scope(|agent, scope| {
            agent
                .evaluate_script_within(scope, realm, &units(source), Rc::clone(&budget))
                .map(|_| ())
        });
        assert_eq!(
            result,
            Err(ScriptError::Abrupt(Abrupt::Exhausted(Exhausted::Steps)))
        );
        assert_eq!(agent.runs, 0, "all reentrant interpreter runs ended");
        assert!(
            agent.execution_budgets.is_empty(),
            "scoped quotas were removed"
        );
        assert_eq!(agent.heap_mut().push_registers(0), Some(0));
        agent.heap_mut().collect();
        agent.heap().verify().expect("valid collected heap");
        agent.scope(|agent, scope| {
            assert!(agent.take_exception(scope).is_none());
            let value = agent
                .evaluate_script(scope, realm, &units("2+3"))
                .expect("recovery");
            assert_eq!(agent.heap().get(value).as_number(), Some(5.0));
        });
        assert_eq!(agent.runs, 0);
        assert!(agent.execution_budgets.is_empty());
    }
}
