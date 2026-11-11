use std::rc::Rc;

use super::*;
use crate::agent::{Abrupt, Budget, Exception, Exhausted};
use crate::heap::Options;
use crate::script::ScriptError;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

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
    assert_eq!(agent.heap_mut().push_registers(0), Some(0), "{source}");
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
    let after = evaluate(&mut agent, realm, "log");
    (before, after)
}

/// `log` vor und nach den Jobs, in beiden Modi der Sammlung gleich.
fn logs(source: &str) -> (String, String) {
    let normal = log_with(source, false);
    assert_eq!(log_with(source, true), normal, "{source}");
    normal
}

/// `log` nach den Jobs.
fn check(cases: &[(&str, &str)]) {
    for &(source, expected) in cases {
        assert_eq!(logs(source).1, expected, "{source}");
    }
}

/// AsyncGeneratorStart (Abschnitt 27.9.3.2): Der Aufruf führt den Rumpf
/// nicht aus. Erst `next` setzt ihn fort, bis zum Await von `yield`, und
/// gibt ein Promise zurück; das Ergebnis kommt mit den Jobs.
#[test]
fn next_starts_the_body_and_returns_promises() {
    let source = "async function* g() { log += 'a,'; yield 1; log += 'b,'; return 2; }
        var it = g();
        log += 's,';
        var p = it.next();
        log += (p instanceof Promise) + ',';
        p.then(function (r) { log += r.value + ':' + r.done + ','; return it.next(); })
         .then(function (r) { log += r.value + ':' + r.done + ','; return it.next(); })
         .then(function (r) { log += r.value + ':' + r.done + ','; });";
    assert_eq!(
        logs(source),
        (
            "s,a,true,".to_owned(),
            "s,a,true,1:false,b,2:true,undefined:true,".to_owned()
        )
    );
}

/// AsyncGeneratorEnqueue und AsyncGeneratorYield, Schritt 11: Wartet nach
/// einem `yield` schon der nächste Eintrag, geht der Rumpf gleich weiter.
#[test]
fn requests_wait_in_the_queue() {
    check(&[
        (
            "async function* g() { yield 'x'; yield 'y'; }
             var it = g();
             it.next().then(function (r) { log += r.value + ','; });
             it.next().then(function (r) { log += r.value + ','; });
             it.next().then(function (r) { log += r.done + ','; });",
            "x,y,true,",
        ),
        // `next` aus dem laufenden Rumpf reiht sich nur ein.
        (
            "var it;
             async function* g() {
               var p = it.next('inner'); log += 'q,';
               var x = yield 1; log += 'x' + x + ','; yield 2;
             }
             it = g();
             it.next('outer').then(function (r) { log += r.value + ','; });",
            "q,xinner,1,",
        ),
    ]);
}

/// `yield` und `return` warten auf ihren Wert (Yield, Abschnitt 27.8.3.7,
/// und Abschnitt 14.10.1); eine Ablehnung wirft dort.
#[test]
fn yield_and_return_await_their_values() {
    check(&[
        (
            "async function* g() { var x = yield Promise.resolve('p'); log += 'x' + x + ','; }
             var it = g();
             it.next().then(function (r) { log += r.value + ','; return it.next('v'); })
               .then(function (r) { log += r.done + ','; });",
            "p,xv,true,",
        ),
        (
            "async function* g() { try { yield Promise.reject('r'); } catch (e) { log += 'c' + e + ','; } }
             g().next().then(function (r) { log += r.done + ','; });",
            "cr,true,",
        ),
        (
            "async function* g() { return Promise.resolve('r'); }
             g().next().then(function (r) { log += r.value + r.done; });",
            "rtrue",
        ),
        (
            "async function* g() { try { await Promise.reject('a'); } catch (e) { log += 'c' + e + ','; } throw 'b'; }
             var it = g();
             it.next().then(null, function (e) { log += 'r' + e + ','; return it.next(); })
               .then(function (r) { log += r.done + ','; });",
            "ca,rb,true,",
        ),
    ]);
}

/// `return` und `throw` (Abschnitte 27.9.1.3 und 27.9.1.4) vor dem
/// Start, an einem `yield` und nach dem Ende.
#[test]
fn return_and_throw_follow_the_state() {
    check(&[
        (
            "async function* g() { try { yield 1; } finally { log += 'f,'; } }
             var it = g();
             it.next().then(function () { return it.return('r'); })
               .then(function (r) { log += r.value + r.done + ','; });",
            "f,rtrue,",
        ),
        // Vor dem Start wartet `return` auf den Wert, der Rumpf läuft nie,
        // und `next` reiht sich dahinter ein (AsyncGeneratorAwaitReturn).
        (
            "async function* g() { log += 'body,'; }
             var it = g();
             it.return(Promise.resolve('v')).then(function (r) { log += r.value + r.done + ','; });
             it.next().then(function (r) { log += 'n' + r.done + ','; });",
            "vtrue,ntrue,",
        ),
        (
            "async function* g() { log += 'body,'; }
             var it = g();
             it.return(Promise.reject('x')).then(null, function (e) { log += 'r' + e + ','; });",
            "rx,",
        ),
        (
            "async function* g() { try { yield 1; } catch (e) { log += 'c' + e + ','; yield 2; } }
             var it = g();
             it.next().then(function () { return it.throw('E'); })
               .then(function (r) { log += r.value + ','; });",
            "cE,2,",
        ),
        (
            "async function* g() { log += 'body,'; }
             var it = g();
             it.throw('E').then(null, function (e) { log += 'rej' + e + ','; });
             it.next().then(function (r) { log += r.done + ','; });",
            "rejE,true,",
        ),
        (
            "async function* g() {}
             var it = g();
             it.next().then(function () { return it.return('z'); })
               .then(function (r) { log += r.value + r.done; });",
            "ztrue",
        ),
    ]);
}

/// PromiseResolve liest `constructor` eines Promise. Wirft der Getter beim
/// Await für `return`, gilt das wie ein Abschluss mit `throw`: an einem
/// `yield` im Rumpf, sonst lehnt es das Promise ab.
#[test]
fn errors_from_promise_resolve_for_return() {
    check(&[
        (
            "class P extends Promise { get ['constructor']() { throw 'c'; } }
             async function* g() {}
             g().return(new P(function () {})).then(null, function (e) { log += 'r' + e; });",
            "rc",
        ),
        (
            "class P extends Promise { get ['constructor']() { throw 'c'; } }
             async function* g() { try { yield 1; } catch (e) { log += 'caught' + e + ','; } }
             var it = g();
             it.next().then(function () { return it.return(new P(function () {})); })
               .then(function (r) { log += r.done + ','; });",
            "caughtc,true,",
        ),
        // Der Eintrag für `return` wartet schon, wenn `yield` fertig ist.
        (
            "class P extends Promise { get ['constructor']() { throw 'c'; } }
             async function* g() { try { yield 1; } catch (e) { log += 'k' + e + ','; } }
             var it = g();
             it.next();
             it.return(new P(function () {})).then(function (r) { log += r.done + ','; });",
            "kc,true,",
        ),
    ]);
}

/// `yield*` (Abschnitt 15.5.5) mit ~async~: über async-Generatoren, über
/// synchrone Iteratoren und mit `return` und `throw` für den inneren
/// Iterator.
#[test]
fn yield_star_delegates_to_async_iterators() {
    check(&[
        (
            "async function* inner() { var a = yield 1; log += 'a' + a + ','; return 'ret'; }
             async function* outer() { var r = yield* inner(); log += 'r' + r + ','; }
             var it = outer();
             it.next().then(function (r) { log += r.value + ','; return it.next('x'); })
               .then(function (r) { log += r.done + ','; });",
            "1,ax,rret,true,",
        ),
        (
            "async function* g() { yield* [Promise.resolve('a'), 'b']; }
             var it = g();
             it.next().then(function (r) { log += r.value + ','; return it.next(); })
               .then(function (r) { log += r.value + ','; return it.next(); })
               .then(function (r) { log += r.done; });",
            "a,b,true",
        ),
        (
            "var inner = { [Symbol.asyncIterator]() { return this; },
               next() { return { value: 1, done: false }; },
               return(v) { log += 'ret' + v + ','; return { value: 'rv', done: true }; } };
             async function* g() { try { yield* inner; } finally { log += 'f,'; } }
             var it = g();
             it.next().then(function () { return it.return('x'); })
               .then(function (r) { log += r.value + r.done; });",
            "retx,f,rvtrue",
        ),
        // Ohne `throw` schließt `yield*` den Iterator, dann folgt ein
        // TypeError.
        (
            "var inner = { [Symbol.asyncIterator]() { return this; },
               next() { return { value: 1, done: false }; },
               return() { log += 'close,'; return {}; } };
             async function* g() { yield* inner; }
             var it = g();
             it.next().then(function () { return it.throw('E'); })
               .then(null, function (e) { log += e.constructor === TypeError; });",
            "close,true",
        ),
        // Ohne `return` gilt der übergebene Wert.
        (
            "var inner = { [Symbol.asyncIterator]() { return this; },
               next() { return { value: 1, done: false }; } };
             async function* g() { try { yield* inner; } finally { log += 'f,'; } }
             var it = g();
             it.next().then(function () { return it.return(Promise.resolve('w')); })
               .then(function (r) { log += r.value + r.done; });",
            "f,wtrue",
        ),
        (
            "var inner = { [Symbol.asyncIterator]() { return this; },
               next(v) { log += 'n' + v + ','; return Promise.resolve({ value: 'i', done: false }); },
               throw(e) { log += 't' + e + ','; return { value: 'end', done: true }; } };
             async function* g() { var r = yield* inner; log += 'r' + r + ','; }
             var it = g();
             it.next('a').then(function (r) { log += r.value + ','; return it.throw('E'); })
               .then(function (r) { log += r.done + ','; });",
            "nundefined,i,tE,rend,true,",
        ),
        (
            "async function* g() { yield* { [Symbol.asyncIterator]() { return this; }, next() { return 1; } }; }
             g().next().then(null, function (e) { log += e.constructor === TypeError; });",
            "true",
        ),
    ]);
}

/// `for await` in einem async-Generator, auch mit `yield` im Rumpf, das
/// mit `return` den Iterator schließt.
#[test]
fn for_await_runs_in_async_generators() {
    check(&[
        (
            "async function* src() { yield 'a'; yield 'b'; }
             async function* g() { for await (var x of src()) yield x + x; }
             var it = g();
             it.next().then(function (r) { log += r.value + ','; return it.next(); })
               .then(function (r) { log += r.value + ','; return it.next(); })
               .then(function (r) { log += r.done; });",
            "aa,bb,true",
        ),
        (
            "async function* src() { try { yield 'a'; yield 'b'; } finally { log += 'closed,'; } }
             async function* g() { for await (var x of src()) yield x; }
             var it = g();
             it.next().then(function () { return it.return('r'); })
               .then(function (r) { log += r.value + r.done; });",
            "closed,rtrue",
        ),
    ]);
}

/// Die Jobs, die die Spezifikation zählt: ein Await für `yield` und eins
/// für `return`, dann erfüllt CompleteStep das Promise.
#[test]
fn yields_take_the_jobs_the_specification_counts() {
    check(&[
        (
            "async function* g() { log += 'g1,'; yield 'y'; }
             g().next().then(function (r) { log += r.value + ','; });
             Promise.resolve()
               .then(function () { log += 'p1,'; })
               .then(function () { log += 'p2,'; })
               .then(function () { log += 'p3,'; });",
            "g1,p1,y,p2,p3,",
        ),
        (
            "async function* g() { return 'r'; }
             g().next().then(function (r) { log += r.value + ','; });
             Promise.resolve()
               .then(function () { log += 'p1,'; })
               .then(function () { log += 'p2,'; })
               .then(function () { log += 'p3,'; });",
            "p1,r,p2,p3,",
        ),
    ]);
}

#[test]
fn frames_keep_this_arguments_and_super() {
    check(&[(
        "class A { m() { return 'A'; } }
         class B extends A { async *m(x) { await 0; yield super.m() + this.k + x + arguments.length; } }
         var b = new B(); b.k = 'k';
         b.m('x', 'y').next().then(function (r) { log += r.value; });",
        "Akx2",
    )]);
}

/// AsyncGeneratorValidate (Abschnitt 27.9.3.3): Ein falsches `this` lehnt
/// das Promise ab, statt zu werfen.
#[test]
fn methods_reject_other_receivers() {
    check(&[(
        "async function* g() {}
         var p = g().next.call({});
         log += (p instanceof Promise) + ',';
         p.then(null, function (e) { log += e instanceof TypeError; });",
        "true,true",
    )]);
}

#[test]
fn async_generators_are_not_constructors() {
    for source in [
        "async function* f() {} new f()",
        "new ({ async *m() {} }).m()",
    ] {
        let (mut agent, realm) = agent_with(false);
        assert_eq!(evaluate(&mut agent, realm, source), "TypeError", "{source}");
    }
    check(&[(
        "async function* f() {}
         log += typeof f + ',' + Object.prototype.toString.call(f) + ','
           + Object.prototype.toString.call(f()) + ',' + ('prototype' in f) + ',' + f.name + f.length;",
        "function,[object AsyncGeneratorFunction],[object AsyncGenerator],true,f0",
    )]);
}

/// Einem Job, der den Rumpf fortsetzt, kann die Quote ausgehen. Der
/// Generator ist danach fertig, die übrigen Jobs laufen weiter.
#[test]
fn a_used_up_budget_ends_the_generator() {
    let (mut agent, realm) = agent_with(false);
    let source = "var after = '';
        async function* spin() { await 0; for (;;) {} }
        var it = spin();
        it.next();
        Promise.resolve().then(function () { after = 'gelaufen'; });
        'ok'";
    assert_eq!(evaluate(&mut agent, realm, source), "ok");
    let budget = Rc::new(Budget::new(1000));
    assert_eq!(
        agent.with_execution_budget(budget, Agent::run_jobs),
        Err(Abrupt::Exhausted(Exhausted::Steps))
    );
    assert!(agent.has_pending_jobs());
    assert_eq!(agent.heap_mut().push_registers(0), Some(0));
    assert_eq!(agent.run_jobs(), Ok(()));
    assert_eq!(evaluate(&mut agent, realm, "after"), "gelaufen");
    let source = "var done = ''; it.next().then(function (r) { done = '' + r.done; }); 'ok'";
    assert_eq!(evaluate(&mut agent, realm, source), "ok");
    assert_eq!(agent.run_jobs(), Ok(()));
    assert_eq!(evaluate(&mut agent, realm, "done"), "true");
}

/// Viele Durchläufe mit `await` und `yield` brauchen keinen tiefen Stapel:
/// Der Rahmen liegt bei jedem Fortsetzen unten in einem neuen Lauf.
#[test]
fn long_loops_need_no_deep_stack() {
    let thread = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let (mut agent, realm) = agent_with(false);
            let source = "var log = '';
                async function* g() { for (var i = 0; i < 2000; i++) { await i; yield i; } }
                async function f() { var s = 0; for await (var x of g()) s += x; return s; }
                f().then(function (v) { log = 'fertig ' + v; });
                log";
            agent.with_execution_budget(Rc::new(Budget::unlimited()), |agent| {
                assert_eq!(evaluate(agent, realm, source), "");
                assert_eq!(agent.run_jobs(), Ok(()));
                assert_eq!(evaluate(agent, realm, "log"), "fertig 1999000");
            });
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}
