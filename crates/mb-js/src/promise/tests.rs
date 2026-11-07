use std::rc::Rc;

use super::*;
use crate::agent::{Budget, Exception, Exhausted};
use crate::heap::Options;
use crate::script::ScriptError;

fn agent_with(stress: bool) -> (Agent, RealmId) {
    let mut agent = Agent::with_options(Options {
        stress,
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    (agent, realm)
}

/// Führt ein Script aus und beschreibt das Ergebnis: eine Zeichenkette als
/// sie selbst, einen Fehler der Engine mit seiner Art.
fn evaluate(agent: &mut Agent, realm: RealmId, source: &str) -> String {
    let result =
        agent.scope(
            |agent, scope| match agent.evaluate_script(scope, realm, &units(source)) {
                Ok(value) => {
                    let heap = agent.heap();
                    match heap.get(value).kind() {
                        Kind::String(string) => {
                            String::from_utf16(&heap.string_units(string)).expect("UTF-16")
                        }
                        kind => format!("{kind:?}"),
                    }
                }
                Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
                    Some(Exception::Error(kind)) => format!("{kind:?}"),
                    Some(Exception::Value(_)) => "throw".to_owned(),
                    None => "throw ohne Ausnahme".to_owned(),
                },
                Err(error) => format!("{error:?}"),
            },
        );
    assert_eq!(agent.heap_mut().push_registers(0), Some(0), "{source}");
    result
}

/// Führt `source` mit einer globalen Zeichenkette `log` aus, dann alle
/// Jobs, und gibt `log` vor und nach den Jobs zurück.
fn log_with(source: &str, stress: bool) -> (String, String) {
    let (mut agent, realm) = agent_with(stress);
    let script = format!("var log = '';\n{source}\n;log");
    let before = evaluate(&mut agent, realm, &script);
    assert_eq!(agent.run_jobs(), Ok(()), "{source}");
    assert!(!agent.has_pending_jobs(), "{source}");
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
    let after = evaluate(&mut agent, realm, "log");
    (before, after)
}

/// `log` nach den Jobs, in beiden Modi der Sammlung gleich.
fn log(source: &str) -> String {
    let (_, after) = log_with(source, false);
    assert_eq!(log_with(source, true).1, after, "{source}");
    after
}

fn check(cases: &[(&str, &str)]) {
    for &(source, expected) in cases {
        assert_eq!(log(source), expected, "{source}");
    }
}

#[test]
fn reactions_run_as_jobs_after_the_script() {
    let source = "var p = Promise.resolve(1);
        p.then(function (v) { log += 'a' + v + ','; return 2; })
         .then(function (v) { log += 'b' + v + ','; });
        p.then(function () { log += 'c,'; });
        log += 's,';";
    assert_eq!(
        log_with(source, false),
        ("s,".to_owned(), "s,a1,c,b2,".to_owned())
    );
    assert_eq!(log(source), "s,a1,c,b2,");
}

#[test]
fn thenables_and_promises_resolve_through_extra_jobs() {
    check(&[
        (
            "var thenable = { then: function (resolve) { log += 'then,'; resolve('t'); } };
             Promise.resolve(thenable).then(function (v) { log += v + ','; });
             Promise.resolve().then(function () { log += 'x,'; });
             log += 's,';",
            "s,then,x,t,",
        ),
        // NewPromiseResolveThenableJob und dann der Job aus `then` des
        // Promise: zwei Jobs mehr als ein Wert.
        (
            "var a = Promise.resolve('a');
             new Promise(function (resolve) { resolve(a); }).then(function (v) { log += v + ','; });
             Promise.resolve()
               .then(function () { log += '1,'; })
               .then(function () { log += '2,'; })
               .then(function () { log += '3,'; });",
            "1,2,a,3,",
        ),
        (
            "Promise.resolve({ get then() { throw 'g'; } }).catch(function (e) { log += e; });",
            "g",
        ),
        (
            "Promise.resolve({ then: 1 }).then(function (v) { log += v.then; });",
            "1",
        ),
    ]);
}

#[test]
fn catch_and_finally_pass_values_and_reasons_on() {
    check(&[
        (
            "Promise.reject(new TypeError('r'))
               .catch(function (e) { log += e.message + ','; return 'v'; })
               .finally(function () { log += 'f,'; return 'ignored'; })
               .then(function (v) { log += v; });",
            "r,f,v",
        ),
        (
            "Promise.reject(1)
               .finally(function () { log += 'g,'; })
               .then(null, function (e) { log += 'e' + e; });",
            "g,e1",
        ),
        (
            "Promise.resolve(2).finally(function () { throw 'w'; }).catch(function (e) { log += e; });",
            "w",
        ),
        (
            "Promise.resolve(3).finally(4).then(function (v) { log += v; });",
            "3",
        ),
    ]);
}

#[test]
fn executors_and_resolving_functions_settle_once() {
    check(&[
        (
            "new Promise(function (resolve, reject) { resolve(1); reject(2); throw 3; })
               .then(function (v) { log += v; }, function (e) { log += 'e' + e; });",
            "1",
        ),
        (
            "new Promise(function () { throw 'x'; }).catch(function (e) { log += e; });",
            "x",
        ),
        (
            "new Promise(function () { null.x; })
               .catch(function (e) { log += e instanceof TypeError; });",
            "true",
        ),
        (
            "Promise.resolve().then(function () { missing; })
               .catch(function (e) { log += e instanceof ReferenceError; });",
            "true",
        ),
        (
            "var resolveSelf;
             var p = new Promise(function (resolve) { resolveSelf = resolve; });
             resolveSelf(p);
             p.catch(function (e) { log += e instanceof TypeError; });",
            "true",
        ),
        (
            "var f;
             new Promise(function (resolve) { f = resolve; });
             log += f.name === '' && f.length === 1 && typeof f.prototype;
             try { new f(); } catch (e) { log += ',' + (e instanceof TypeError); }",
            "undefined,true",
        ),
    ]);
}

#[test]
fn combinators_collect_settle_and_race() {
    check(&[
        (
            "Promise.all([1, Promise.resolve(2), { then: function (r) { r(3); } }])
               .then(function (v) { log += v.length + ':' + v[0] + v[1] + v[2]; });",
            "3:123",
        ),
        (
            "Promise.all([]).then(function (v) { log += v.length; });",
            "0",
        ),
        (
            "Promise.all([Promise.reject('x'), Promise.reject('y')])
               .catch(function (e) { log += e; });",
            "x",
        ),
        (
            "Promise.allSettled([1, Promise.reject(2)]).then(function (r) {
               log += r[0].status + r[0].value + r[1].status + r[1].reason + r.length;
             });",
            "fulfilled1rejected22",
        ),
        (
            "Promise.any([Promise.reject(1), Promise.reject(2)]).catch(function (e) {
               log += (e instanceof AggregateError) + ':' + e.errors.length + e.errors[0]
                 + e.errors[1] + ':' + Object.prototype.toString.call(e);
             });",
            "true:212:[object Error]",
        ),
        (
            "Promise.any([Promise.reject(1), 2]).then(function (v) { log += v; });",
            "2",
        ),
        (
            "Promise.any([]).catch(function (e) { log += e.errors.length; });",
            "0",
        ),
        (
            "Promise.race([new Promise(function () {}), Promise.resolve('r'), Promise.reject('x')])
               .then(function (v) { log += v; }, function (e) { log += 'e' + e; });",
            "r",
        ),
        (
            "Promise.all(1).catch(function (e) { log += e instanceof TypeError; });",
            "true",
        ),
        // Wirft der Aufruf von `resolve`, schließt der Kombinator den
        // Iterator, und das Ergebnis wird mit dem Fehler abgelehnt.
        (
            "var closed = 0;
             var iterable = {};
             iterable[Symbol.iterator] = function () {
               return {
                 next: function () { return { value: 1, done: false }; },
                 return: function () { closed++; return {}; }
               };
             };
             Promise.resolve = function () { throw 'boom'; };
             Promise.all(iterable).catch(function (e) { log += e + closed; });",
            "boom1",
        ),
    ]);
}

#[test]
fn try_and_with_resolvers_follow_their_callbacks() {
    check(&[
        (
            "Promise.try(function (a, b) { return a + b; }, 1, 2).then(function (v) { log += v; });",
            "3",
        ),
        (
            "Promise.try(function () { throw 'x'; }).catch(function (e) { log += e; });",
            "x",
        ),
        (
            "var r = Promise.withResolvers();
             r.promise.then(function (v) { log += v; });
             r.resolve('w');
             r.reject('ignoriert');",
            "w",
        ),
    ]);
}

#[test]
fn subclasses_and_species_build_derived_promises() {
    check(&[
        (
            "class P extends Promise {}
             var p = P.resolve(1);
             log += (p instanceof P) + ',' + (p.then(function () {}) instanceof P) + ',';
             p.then(function (v) { log += v; });",
            "true,true,1",
        ),
        (
            "log += (Promise[Symbol.species] === Promise) + ','
               + Object.prototype.toString.call(Promise.resolve()) + ','
               + Promise.name + Promise.length + Promise.prototype.then.length
               + Promise.withResolvers.length + AggregateError.length;",
            "true,[object Promise],Promise1202",
        ),
        (
            "var e = new AggregateError([1, 2], 'm', { cause: 'c' });
             log += e.message + e.errors.length + e.errors[1] + e.cause + (e instanceof Error)
               + AggregateError.prototype.name + (AggregateError([]) instanceof AggregateError);",
            "m22ctrueAggregateErrortrue",
        ),
    ]);
}

#[test]
fn wrong_receivers_and_executors_throw_type_errors() {
    for source in [
        "Promise(function () {})",
        "new Promise(1)",
        "Promise.prototype.then.call({}, function () {})",
        "Promise.resolve.call(1)",
        "Promise.reject.call(function () {})",
        "Promise.withResolvers.call(undefined)",
        "Promise.prototype.finally.call(1)",
        "new AggregateError(1)",
    ] {
        let (mut agent, realm) = agent_with(false);
        assert_eq!(evaluate(&mut agent, realm, source), "TypeError", "{source}");
    }
}

#[test]
fn a_used_up_budget_leaves_the_remaining_jobs_queued() {
    let (mut agent, realm) = agent_with(false);
    // Zwei Ketten, damit hinter dem Job, dem die Quote ausgeht, immer einer
    // wartet; der abgebrochene selbst ist verbraucht.
    let source = "var count = 0; function loop() { count++; Promise.resolve().then(loop); } \
                  loop(); loop(); 'ok'";
    assert_eq!(evaluate(&mut agent, realm, source), "ok");
    let budget = Rc::new(Budget::new(50));
    assert_eq!(
        agent.with_execution_budget(budget, Agent::run_jobs),
        Err(Abrupt::Exhausted(Exhausted::Steps))
    );
    assert!(agent.has_pending_jobs());
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
    // Ein weiterer Lauf macht dort weiter.
    let budget = Rc::new(Budget::new(50));
    let _ = agent.with_execution_budget(budget, Agent::run_jobs);
    assert_eq!(
        evaluate(&mut agent, realm, "count > 1 ? 'weiter' : 'stehen'"),
        "weiter"
    );
}

#[test]
fn an_abrupt_job_returns_to_the_host_and_the_rest_wait() {
    let (mut agent, realm) = agent_with(false);
    // Eine eigene Capability, deren [[Reject]] wirft: Der Fehler aus dem
    // Job geht an den Host.
    let source = "var later = '';
        function F(executor) { executor(function () {}, function () { throw new RangeError('r'); }); }
        F[Symbol.species] = F;
        var p = Promise.reject(1);
        p.constructor = F;
        p.then();
        Promise.resolve().then(function () { later = 'gelaufen'; });
        'ok'";
    assert_eq!(evaluate(&mut agent, realm, source), "ok");
    assert_eq!(agent.run_jobs(), Err(Abrupt::Throw));
    agent.scope(|agent, scope| {
        assert!(matches!(
            agent.take_exception(scope),
            Some(Exception::Value(_))
        ));
    });
    assert!(agent.has_pending_jobs());
    assert_eq!(agent.run_jobs(), Ok(()));
    assert_eq!(evaluate(&mut agent, realm, "later"), "gelaufen");
}
