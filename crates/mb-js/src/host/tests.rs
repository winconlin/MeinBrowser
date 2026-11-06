use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::agent::{Abrupt, ErrorKind, Exception};
use crate::builtin::{Builtin, BuiltinCall, define_builtin_function};
use crate::heap::{Local, Options};
use crate::script::ScriptError;
use crate::value::{Kind, Value};

/// Ein Host, der zählt, wie viele Realms er eingerichtet hat, und auf
/// Wunsch scheitert.
#[derive(Default)]
struct Counter {
    realms: Cell<i32>,
    fail: Cell<bool>,
}

static COUNT: Builtin = Builtin {
    name: "count",
    length: 0,
    constructor: false,
    behaviour: count,
};

/// Erreicht den Host über den Agent und gibt seinen Zähler zurück.
fn count<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    _: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let counter = agent.host().and_then(|host| -> Option<Rc<Counter>> {
        let host: Rc<dyn Any> = host;
        host.downcast::<Counter>().ok()
    });
    let Some(counter) = counter else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    Ok(agent
        .heap()
        .local(scope, Value::int32(counter.realms.get())))
}

impl Host for Counter {
    fn define_global_properties(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        realm: RealmId,
    ) -> Completion<()> {
        if self.fail.get() {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        self.realms.set(self.realms.get().saturating_add(1));
        let global = agent.global_object(scope, realm);
        define_builtin_function(agent, scope, realm, global, &COUNT)
    }
}

/// Ein Host ohne eigene Hooks.
struct Quiet;

impl Host for Quiet {}

fn agent() -> Agent {
    Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher")
}

fn run(agent: &mut Agent, realm: RealmId, source: &str) -> String {
    let units: Vec<u16> = source.encode_utf16().collect();
    agent.scope(
        |agent, scope| match agent.evaluate_script(scope, realm, &units) {
            Ok(value) => match agent.heap().get(value).kind() {
                Kind::Number(number) => number.to_string(),
                Kind::String(string) => {
                    String::from_utf16(&agent.heap().string_units(string)).expect("UTF-16")
                }
                kind => format!("{kind:?}"),
            },
            Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
                Some(Exception::Error(kind)) => format!("{kind:?}"),
                _ => "throw".to_owned(),
            },
            Err(error) => format!("{error:?}"),
        },
    )
}

#[test]
fn the_host_defines_global_properties_in_every_realm() {
    let mut agent = agent();
    agent.set_host(Rc::new(Counter::default()));
    let first = agent.create_realm().expect("Realm");
    let second = agent.create_realm().expect("Realm");
    assert_eq!(run(&mut agent, first, "count()"), "2");
    assert_eq!(run(&mut agent, second, "count()"), "2");
    assert_eq!(run(&mut agent, second, "typeof count"), "function");
    assert_eq!(
        run(&mut agent, first, "count === this.count"),
        "Boolean(true)"
    );
}

#[test]
fn a_failing_hook_leaves_no_realm_behind() {
    let mut agent = agent();
    let counter = Rc::new(Counter::default());
    agent.set_host(Rc::clone(&counter) as Rc<dyn Host>);
    let first = agent.create_realm().expect("Realm");
    counter.fail.set(true);
    assert_eq!(agent.create_realm(), Err(Abrupt::Throw));
    agent.scope(|agent, scope| {
        assert!(matches!(
            agent.take_exception(scope),
            Some(Exception::Error(ErrorKind::TypeError))
        ));
    });
    counter.fail.set(false);
    let second = agent.create_realm().expect("Realm");
    assert_eq!(Some(second.index()), first.index().checked_add(1));
    assert_eq!(run(&mut agent, second, "count()"), "2");
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
}

#[test]
fn without_hooks_the_global_object_stays_as_it_is() {
    for host in [None, Some(Rc::new(Quiet))] {
        let mut agent = agent();
        let installed = host.is_some();
        if let Some(host) = host {
            agent.set_host(host);
        }
        assert_eq!(agent.host().is_some(), installed);
        let realm = agent.create_realm().expect("Realm");
        assert_eq!(run(&mut agent, realm, "typeof count"), "undefined");
    }
}
