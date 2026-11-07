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

/// AsyncFunctionStart (Abschnitt 27.10.5.1): Der Rumpf läuft beim Aufruf
/// bis zum ersten `await`, der Aufrufer bekommt dann das Promise, und ein
/// Job setzt den Rumpf nach dem Script fort.
#[test]
fn the_body_runs_until_the_first_await() {
    let source = "async function f() { log += 'a,'; await 1; log += 'c,'; await 2; log += 'd,'; }
        var p = f();
        log += (p instanceof Promise) + ',';";
    assert_eq!(
        logs(source),
        ("a,true,".to_owned(), "a,true,c,d,".to_owned())
    );
    // Ohne `await` ist das Promise schon erfüllt, wenn der Aufruf endet.
    let source = "async function f() { log += 'a,'; return 'r'; }
        f().then(function (v) { log += v + ','; });
        log += 's,';";
    assert_eq!(logs(source), ("a,s,".to_owned(), "a,s,r,".to_owned()));
}

#[test]
fn awaited_values_and_returns_settle_the_promise() {
    check(&[
        (
            "async function f() { var x = await 2; var y = await Promise.resolve(3); return x + y; }
             f().then(function (v) { log += v; });",
            "5",
        ),
        (
            "async function f() { await 1; } f().then(function (v) { log += typeof v; });",
            "undefined",
        ),
        ("(async () => 7)().then(function (v) { log += v; });", "7"),
        (
            "var g = async (a, b) => await a + b; g(1, 2).then(function (v) { log += v; });",
            "3",
        ),
        // `await` liest den Wert eines Thenable über einen weiteren Job.
        (
            "async function f() { return await { then: function (r) { r('t'); } }; }
             f().then(function (v) { log += v; });",
            "t",
        ),
        // Ein Promise als Rückgabewert folgt dem Promise der Funktion.
        (
            "async function f() { return Promise.resolve('p'); } f().then(function (v) { log += v; });",
            "p",
        ),
    ]);
}

#[test]
fn exceptions_reject_the_promise() {
    check(&[
        (
            "async function f() { throw 'e'; } f().catch(function (e) { log += e; });",
            "e",
        ),
        (
            "async function f() { await 1; null.x; } f().catch(function (e) { log += e.constructor.name; });",
            "TypeError",
        ),
        (
            "async function f() { await Promise.reject('r'); } f().catch(function (e) { log += e; });",
            "r",
        ),
        (
            "async function f() {
               try { await Promise.reject('r'); } catch (e) { log += 'c' + e; } finally { log += 'f'; }
               try { await 1; throw 't'; } catch (e) { log += e; }
             }
             f().then(function () { log += 'ok'; });",
            "crftok",
        ),
        // `finally` mit `await` nach `return` und nach einer Ausnahme.
        (
            "async function f() { try { return 1; } finally { await 0; log += 'f'; } }
             async function g() { try { throw 'x'; } finally { await 0; log += 'g'; } }
             f().then(function (v) { log += v; });
             g().catch(function (e) { log += e; });",
            "fg1x",
        ),
        // EvaluateAsyncFunctionBody, Schritt 3: Ein Fehler in den Parametern
        // lehnt das Promise ab, statt zu werfen.
        (
            "async function f(a = (function () { throw 'p'; })()) {}
             var p = f(); log += 's';
             p.catch(function (e) { log += e; });",
            "sp",
        ),
        (
            "var f = async ({ x }) => x; f().catch(function (e) { log += e.constructor.name; });",
            "TypeError",
        ),
    ]);
}

/// Await (Abschnitt 27.10.5.3) braucht für einen Wert und für ein Promise
/// aus %Promise% je einen Job. Ein Promise als Ergebnis braucht zwei Jobs
/// mehr: NewPromiseResolveThenableJob und den Job aus `then`.
#[test]
fn awaits_take_the_jobs_the_specification_counts() {
    check(&[
        (
            "async function a() { log += 'a1,'; await undefined; log += 'a2,'; await Promise.resolve(); log += 'a3,'; }
             a();
             Promise.resolve()
               .then(function () { log += 'p1,'; })
               .then(function () { log += 'p2,'; });
             log += 's,';",
            "a1,s,a2,p1,a3,p2,",
        ),
        (
            "async function r() { return Promise.resolve(1); }
             r().then(function (v) { log += 'r' + v + ','; });
             Promise.resolve()
               .then(function () { log += '1,'; })
               .then(function () { log += '2,'; })
               .then(function () { log += '3,'; });",
            "1,2,r1,3,",
        ),
    ]);
}

#[test]
fn frames_keep_this_arguments_bindings_and_super() {
    check(&[
        (
            "class A { m() { return 'A'; } }
             class B extends A { async m(x) { await 0; return super.m() + this.k + x + arguments.length; } }
             var b = new B(); b.k = 'k';
             b.m('x', 'y').then(function (v) { log += v; });",
            "Akx2",
        ),
        (
            "async function f() { let i = 0; var g = () => i; await 0; i = 5; return g(); }
             f().then(function (v) { log += v; });",
            "5",
        ),
        (
            "function outer() { var f = async () => { await 0; return this.v + arguments[0]; }; return f(); }
             outer.call({ v: 'v' }, 'a').then(function (v) { log += v; });",
            "va",
        ),
        (
            "async function f() { var s = 0; for (var x of [1, 2, 3]) s += await x; return s; }
             f().then(function (v) { log += v; });",
            "6",
        ),
        (
            "var o = { async m() { return await this.n; }, n: 4 };
             o.m().then(function (v) { log += v; });",
            "4",
        ),
        // Im Block ist die Deklaration lexikalisch, ohne Anhang B.3.3.
        (
            "{ async function inner() { await 0; return 'i'; }
               async function outer() { return 'o' + await inner(); }
               outer().then(function (v) { log += v; }); }
             log += typeof inner;",
            "undefinedoi",
        ),
    ]);
}

/// Ein Rahmen hält auch an, wenn er unten in einem Lauf liegt, den die
/// Engine begann, oder an der Stelle eines Endaufrufs.
#[test]
fn frames_suspend_wherever_they_run() {
    check(&[
        (
            "async function f() { await 0; return this.v; }
             f.call({ v: 3 }).then(function (v) { log += v; });",
            "3",
        ),
        (
            "Promise.resolve().then(async function () { await 0; log += 'h'; });",
            "h",
        ),
        (
            "'use strict';
             async function a() { await 0; return 'a'; }
             function t() { return a(); }
             t().then(function (v) { log += v; });",
            "a",
        ),
        (
            "var o = { get x() { return (async function () { await 0; log += 'g'; })(); } };
             o.x;",
            "g",
        ),
    ]);
}

/// PromiseResolve liest `constructor` eines Promise; was der Getter wirft,
/// fängt `catch` in der Funktion.
#[test]
fn errors_from_promise_resolve_are_thrown_at_the_await() {
    let source = "class P extends Promise { get ['constructor']() { throw 'c'; } }
        async function f() { try { await new P(function () {}); } catch (e) { log += e; } }
        f();";
    assert_eq!(logs(source), ("c".to_owned(), "c".to_owned()));
}

/// Viele verschachtelte Aufrufe, die alle anhalten, brauchen weder beim
/// Aufruf noch beim Fortsetzen einen tiefen Stapel (RS-8): Jeder Rahmen
/// läuft nach dem ersten `await` in einem eigenen Job weiter.
#[test]
fn deep_awaits_need_no_deep_stack() {
    let thread = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let (mut agent, realm) = agent_with(false);
            let source = "var log = '';
                async function f(n) { if (n > 0) { await f(n - 1); } return n; }
                f(5000).then(function (v) { log = 'fertig ' + v; });
                log";
            agent.with_execution_budget(Rc::new(Budget::unlimited()), |agent| {
                assert_eq!(evaluate(agent, realm, source), "");
                assert_eq!(agent.run_jobs(), Ok(()));
                assert_eq!(evaluate(agent, realm, "log"), "fertig 5000");
            });
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}

/// Einem Job, der einen Rahmen fortsetzt, kann die Quote ausgehen; der
/// Rahmen bleibt dann stehen, die übrigen Jobs warten.
#[test]
fn a_used_up_budget_stops_the_resumed_frame() {
    let (mut agent, realm) = agent_with(false);
    let source = "var after = '';
        async function spin() { await 0; for (;;) {} }
        spin();
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
}

#[test]
fn async_functions_are_neither_constructors_nor_generators() {
    for source in [
        "async function f() {} new f()",
        "var f = async () => {}; new f()",
        "new ({ async m() {} }).m()",
    ] {
        let (mut agent, realm) = agent_with(false);
        assert_eq!(evaluate(&mut agent, realm, source), "TypeError", "{source}");
    }
    check(&[(
        "async function f() {}
         log += typeof f + ',' + Object.prototype.toString.call(f) + ',' + ('prototype' in f)
           + ',' + f.name + f.length;",
        "function,[object AsyncFunction],false,f0",
    )]);
}

/// `for await` (Abschnitte 14.7.5.6 und 14.7.5.7) mit eigenen
/// async-Iteratoren: Auf jedes Ergebnis von `next` wartet die Schleife.
#[test]
fn for_await_waits_for_each_result() {
    check(&[
        (
            "var it = { i: 0, [Symbol.asyncIterator]() { return this; },
               next() { this.i++; return Promise.resolve({ value: this.i, done: this.i > 3 }); } };
             async function f() { var s = 0; for await (const x of it) s += x; return s; }
             f().then(function (v) { log += v; });",
            "6",
        ),
        (
            "var it = { [Symbol.asyncIterator]() { return { next() { return { value: 'd', done: true }; } }; } };
             async function f() { for await (var x of it) log += 'b'; return typeof x; }
             f().then(function (v) { log += v; });",
            "undefined",
        ),
        (
            "async function f() { var s = '';
               for await (const [a, { b }] of [['a', { b: 'b' }]]) s += a + b;
               outer: for await (let x of ['x', 'y']) { for await (var y of [1, 2]) { s += x + y; continue outer; } }
               return s; }
             f().then(function (v) { log += v; });",
            "abx1y1",
        ),
        // Fehler aus `next` und aus seinem Ergebnis schließen den Iterator
        // nicht.
        (
            "var it = { [Symbol.asyncIterator]() { return { next() { return 1; },
               return() { log += 'r,'; return {}; } }; } };
             async function f() { for await (var x of it); }
             f().catch(function (e) { log += e.constructor.name; });",
            "TypeError",
        ),
        (
            "var it = { [Symbol.asyncIterator]() { return { next() { return Promise.reject('n'); },
               return() { log += 'r,'; return {}; } }; } };
             async function f() { for await (var x of it); }
             f().catch(function (e) { log += e; });",
            "n",
        ),
        (
            "var it = { [Symbol.asyncIterator]() { return {
               next() { return { get done() { throw 'd'; } }; }, return() { log += 'r,'; return {}; } }; } };
             async function f() { for await (var x of it); }
             f().catch(function (e) { log += e; });",
            "d",
        ),
        // Ohne @@asyncIterator und @@iterator.
        (
            "async function f() { for await (var x of {}); }
             f().catch(function (e) { log += e.constructor.name; });",
            "TypeError",
        ),
    ]);
}

/// AsyncIteratorClose (Abschnitt 7.4.15): `break`, `continue` zu einer
/// äußeren Schleife und `return` warten auf das Ergebnis von `return`, das
/// ein Objekt sein muss. Bei einer Ausnahme fällt weg, was das Schließen
/// wirft oder ablehnt.
#[test]
fn for_await_closes_its_iterator_and_waits_for_it() {
    let iterator = |closing: &str| {
        format!(
            "var it = {{ [Symbol.asyncIterator]() {{ return {{
               next() {{ return {{ value: 1, done: false }}; }},
               return() {{ log += 'r,'; {closing} }} }}; }} }};"
        )
    };
    let thenable = iterator("return { then(r) { log += 't,'; r({}); } };");
    let cases = [
        (
            format!(
                "{thenable}
                 async function f() {{ for await (var x of it) {{ log += 'b,'; break; }} log += 'e,'; }}
                 f();"
            ),
            "b,r,t,e,",
        ),
        (
            format!(
                "{thenable}
                 async function f() {{ for await (var x of it) return 'v' + x; }}
                 f().then(function (v) {{ log += v; }});"
            ),
            "r,t,v1",
        ),
        (
            format!(
                "{thenable}
                 async function f() {{ outer: for (var i = 0; i < 2; i++) for await (var x of it) continue outer;
                   return i; }}
                 f().then(function (v) {{ log += v; }});"
            ),
            "r,t,r,t,2",
        ),
        (
            format!(
                "{}
                 async function f() {{ for await (var x of it) break; }}
                 f().catch(function (e) {{ log += e.constructor.name; }});",
                iterator("return 5;")
            ),
            "r,TypeError",
        ),
        (
            format!(
                "{}
                 async function f() {{ for await (var x of it) break; }}
                 f().catch(function (e) {{ log += e; }});",
                iterator("return Promise.reject('no');")
            ),
            "r,no",
        ),
        (
            format!(
                "{}
                 async function f() {{ for await (var x of it) break; }}
                 f().catch(function (e) {{ log += e; }});",
                iterator("throw 'thrown';")
            ),
            "r,thrown",
        ),
    ];
    for (source, expected) in &cases {
        assert_eq!(logs(source).1, *expected, "{source}");
    }
    for closing in [
        "throw 'ignored';",
        "return Promise.reject('ignored');",
        "return 5;",
        "return { then(r) { log += 't,'; r(5); } };",
    ] {
        let source = format!(
            "{}
             async function f() {{ for await (var x of it) {{ log += 'b,'; throw 'body'; }} }}
             f().catch(function (e) {{ log += e; }});",
            iterator(closing)
        );
        let expected = if closing.contains("then") {
            "b,r,t,body"
        } else {
            "b,r,body"
        };
        assert_eq!(logs(&source).1, expected, "{source}");
    }
    // Ohne `return` gibt es nichts zu schließen.
    check(&[(
        "var it = { [Symbol.asyncIterator]() { return { next() { return { value: 1, done: false }; } }; } };
         async function f() { for await (var x of it) break; for await (var y of it) throw 'b'; }
         f().catch(function (e) { log += e; });",
        "b",
    )]);
}

/// CreateAsyncFromSyncIterator (Abschnitt 27.1.5.1): Ohne @@asyncIterator
/// läuft `for await` über den synchronen Iterator und wartet auf jeden Wert.
/// Ist ein Wert ein abgelehntes Promise, schließt es den Iterator.
#[test]
fn for_await_wraps_sync_iterators() {
    check(&[
        (
            "async function f() { var s = '';
               for await (var x of ['a', Promise.resolve('b'), { then(r) { r('c'); } }]) s += x;
               return s; }
             f().then(function (v) { log += v; });",
            "abc",
        ),
        (
            "function* g() { try { yield 1; yield 2; } finally { log += 'f,'; } }
             async function f() { for await (var x of g()) { log += x + ','; break; } return 'e'; }
             f().then(function (v) { log += v; });",
            "1,f,e",
        ),
        (
            "var it = { [Symbol.iterator]() { return {
               next() { return { value: Promise.reject('v'), done: false }; },
               return() { log += 'r,'; return {}; } }; } };
             async function f() { for await (var x of it); }
             f().catch(function (e) { log += e; });",
            "r,v",
        ),
        // Ein fertiger Iterator wird nicht geschlossen, die Ablehnung gilt
        // trotzdem.
        (
            "var it = { [Symbol.iterator]() { return {
               next() { return { value: Promise.reject('v'), done: true }; },
               return() { log += 'r,'; return {}; } }; } };
             async function f() { for await (var x of it); }
             f().catch(function (e) { log += e; });",
            "v",
        ),
        // `return` des synchronen Iterators muss ein Objekt liefern.
        (
            "var it = { [Symbol.iterator]() { return {
               next() { return { value: 1, done: false }; }, return() { return 5; } }; } };
             async function f() { for await (var x of it) break; }
             f().catch(function (e) { log += e.constructor.name; });",
            "TypeError",
        ),
        // Ohne `return` des synchronen Iterators gilt er als geschlossen.
        (
            "async function f() { for await (var x of [1, 2]) { log += x; break; } return 'e'; }
             f().then(function (v) { log += v; });",
            "1e",
        ),
        // Bei einer Ausnahme ruft es `return` auf, was dort geworfen wird,
        // fällt weg.
        (
            "var it = { [Symbol.iterator]() { return {
               next() { return { value: 1, done: false }; }, return() { log += 'r,'; throw 'x'; } }; } };
             async function f() { for await (var x of it) throw 'b'; }
             f().catch(function (e) { log += e; });",
            "r,b",
        ),
    ]);
}

/// Für einen synchronen Iterator braucht jeder Schritt zwei Jobs: einen, der
/// das Ergebnis aus dem Wert macht, und einen für das Await der Schleife.
#[test]
fn for_await_over_sync_iterators_takes_two_jobs_per_step() {
    check(&[(
        "async function f() { for await (var x of ['a']) log += x + ','; log += 'e,'; }
         f();
         Promise.resolve()
           .then(function () { log += 'p1,'; })
           .then(function () { log += 'p2,'; })
           .then(function () { log += 'p3,'; });",
        "p1,a,p2,p3,e,",
    )]);
}
