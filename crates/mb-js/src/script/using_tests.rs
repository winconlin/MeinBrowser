//! Tests für `using` und `await using` (Schritt 13c).

use super::*;
use crate::agent::Exception;
use crate::heap::Options;
use crate::number;
use crate::value::Kind;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn describe(agent: &Agent, value: Local<'_>) -> String {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::Undefined => "undefined".to_owned(),
        Kind::Null => "null".to_owned(),
        Kind::Boolean(boolean) => boolean.to_string(),
        Kind::Number(number) => number::to_string(number),
        Kind::String(string) => String::from_utf16(&heap.string_units(string)).expect("UTF-16"),
        Kind::Object(_) => "Objekt".to_owned(),
        kind => format!("{kind:?}"),
    }
}

fn run(agent: &mut Agent, realm: RealmId, source: &str) -> String {
    let result =
        agent.scope(
            |agent, scope| match agent.evaluate_script(scope, realm, &units(source)) {
                Ok(value) => describe(agent, value),
                Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
                    Some(Exception::Error(kind)) => format!("{kind:?}"),
                    Some(Exception::Value(value)) => format!("throw {}", describe(agent, value)),
                    None => "throw ohne Ausnahme".to_owned(),
                },
                Err(error) => format!("{error:?}"),
            },
        );
    assert_eq!(agent.heap_mut().push_registers(0), Some(0), "{source}");
    result
}

/// Vor jedem Fall stehen `log`, ein Array für die Reihenfolge, und
/// `res(name)`, eine Ressource, die beim Freigeben `name` in `log`
/// schreibt. Mit `jobs` laufen nach dem Script alle Jobs, und das Ergebnis
/// ist `log.join()`; sonst ist es der Wert des Scripts.
fn check_with(cases: &[(&str, &str)], jobs: bool, stress: bool) {
    let mut failures = Vec::new();
    for &(source, expected) in cases {
        let mut agent = Agent::with_options(Options {
            verify: true,
            stress,
            ..Options::default()
        })
        .expect("Speicher");
        let realm = agent.create_realm().expect("Speicher");
        let prelude = "var log = []; \
            function res(name) { return { [Symbol.dispose]() { log.push(name); } }; }";
        assert_eq!(run(&mut agent, realm, prelude), "undefined");
        let mut actual = run(&mut agent, realm, source);
        if jobs {
            assert_eq!(agent.run_jobs(), Ok(()), "{source}");
            actual = run(&mut agent, realm, "log.join()");
        }
        agent.heap_mut().collect();
        agent.heap().verify().expect("gültiger Heap");
        if actual != expected {
            failures.push(format!(
                "{source}\n  erwartet {expected}, bekommen {actual}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn check(cases: &[(&str, &str)]) {
    check_with(cases, false, false);
}

fn check_async(cases: &[(&str, &str)]) {
    check_with(cases, true, false);
    check_with(cases, true, true);
}

#[test]
fn resources_are_disposed_in_reverse_order_at_the_end_of_their_block() {
    check(&[
        (
            "{ using a = res('a'), b = res('b'); log.push('body'); using c = res('c'); } log.join()",
            "body,c,b,a",
        ),
        ("{ using a = null, b = undefined; } log.length", "0"),
        (
            "{ using a = res('a'); { using b = res('b'); } log.push('mid'); } log.join()",
            "b,mid,a",
        ),
        // Der Abschlusswert des Blocks bleibt.
        ("{ using a = res('a'); 5 }", "5"),
        // Mit `this`, ohne Argumente.
        (
            "var r = { [Symbol.dispose]() { log.push(this === r, arguments.length); } }; \
             { using a = r; } log.join()",
            "true,0",
        ),
        // Eine Ressource, die nicht erreicht wurde, kommt nicht dazu.
        (
            "try { { using a = res('a'); throw 1; using b = res('b'); } } catch (e) {} log.join()",
            "a",
        ),
        (
            "try { { using a = res('a'), b = (() => { throw 1; })(), c = res('c'); } } \
             catch (e) { log.push(e); } log.join()",
            "a,1",
        ),
        // @@dispose wird bei der Deklaration gelesen, einmal.
        (
            "var n = 0; var o = { get [Symbol.dispose]() { n++; return function () { log.push(n); }; } }; \
             { using a = o; n = 10; } log.join()",
            "10",
        ),
    ]);
}

#[test]
fn declarations_check_their_values() {
    check(&[
        ("{ using a = 1; }", "TypeError"),
        ("{ using a = 'x'; }", "TypeError"),
        ("{ using a = {}; }", "TypeError"),
        ("{ using a = { [Symbol.dispose]: 1 }; }", "TypeError"),
        ("{ using a = { [Symbol.dispose]: null }; }", "TypeError"),
        ("{ using a = { [Symbol.asyncDispose]() {} }; }", "TypeError"),
        // Nach einem Fehler bleibt die Bindung in der TDZ.
        (
            "var f; try { { f = () => a; using a = 1; } } catch (e) {} \
             try { f(); } catch (e) { e instanceof ReferenceError }",
            "true",
        ),
        ("{ a; using a = null; }", "ReferenceError"),
        ("{ using a = null; a = 1; }", "TypeError"),
        ("{ using a = res('a'); typeof a }", "object"),
    ]);
}

#[test]
fn errors_from_disposal_are_joined_with_suppressed_error() {
    check(&[
        (
            "try { { using a = { [Symbol.dispose]() { throw 1; } }; } } catch (e) { e }",
            "1",
        ),
        (
            "try { { using a = { [Symbol.dispose]() { throw 2; } }; throw 1; } } \
             catch (e) { [e instanceof SuppressedError, e.error, e.suppressed].join() }",
            "true,2,1",
        ),
        (
            "try { { using a = { [Symbol.dispose]() { throw 'a'; } }, \
                     b = { [Symbol.dispose]() { throw 'b'; } }; } } \
             catch (e) { [e.error, e.suppressed].join() }",
            "a,b",
        ),
        (
            "try { { using a = { [Symbol.dispose]() { throw 'a'; } }, b = res('b'); \
                     using c = { [Symbol.dispose]() { throw 'c'; } }; throw 'x'; } } \
             catch (e) { [e.error, e.suppressed.error, e.suppressed.suppressed, log].join() }",
            "a,c,x,b",
        ),
        // Die Eigenschaften sind nicht aufzählbar, `message` fehlt.
        (
            "try { { using a = { [Symbol.dispose]() { throw 2; } }; throw 1; } } \
             catch (e) { [Object.keys(e).length, 'message' in e, e.message, \
                          Object.getPrototypeOf(e) === SuppressedError.prototype].join() }",
            "0,true,,true",
        ),
        // Fehler der Engine kommen als Fehlerobjekte hinein.
        (
            "try { { using a = { [Symbol.dispose]() { throw 'd'; } }; null.x; } } \
             catch (e) { [e.suppressed instanceof TypeError, e.error].join() }",
            "true,d",
        ),
        (
            "try { { using a = { [Symbol.dispose]() { null.x; } }; throw 1; } } \
             catch (e) { [e.error instanceof TypeError, e.suppressed].join() }",
            "true,1",
        ),
        (
            "try { { using a = { [Symbol.dispose]() { null.x; } }; } } \
             catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "{ using a = { [Symbol.dispose]() { null.x; } }; }",
            "throw Objekt",
        ),
    ]);
}

#[test]
fn every_way_out_disposes() {
    check(&[
        (
            "for (var i = 0; i < 3; i++) { using a = res(i); if (i == 0) continue; break; } log.join()",
            "0,1",
        ),
        (
            "function f() { using a = res('a'); return 'r'; } [f(), log].join()",
            "r,a",
        ),
        (
            "function f() { using a = { [Symbol.dispose]() { throw 'x'; } }; return 1; } \
             try { f(); } catch (e) { e }",
            "x",
        ),
        ("l: { using a = res('a'); break l; } log.join()", "a"),
        (
            "outer: for (var i = 0; i < 2; i++) { for (;;) { using a = res(i); continue outer; } } \
             log.join()",
            "0,1",
        ),
        (
            "function f() { try { using a = res('a'); return 1; } finally { log.push('f'); } } \
             f(); log.join()",
            "a,f",
        ),
        (
            "function f() { for (var x of [1]) { using a = res('a'); return x; } } \
             [f(), log].join()",
            "1,a",
        ),
        // Ein Rumpf von `using` hat keine Endaufrufe, auch strikt nicht.
        (
            "'use strict'; function g() { return 1; } \
             function f() { using a = res('a'); return g(); } f(); log.join()",
            "a",
        ),
    ]);
}

#[test]
fn function_bodies_and_generators_dispose() {
    check(&[
        (
            "function f() { using a = res('a'); log.push('b'); } f(); log.join()",
            "b,a",
        ),
        (
            "var f = () => { using a = res('a'); log.push('b'); }; f(); log.join()",
            "b,a",
        ),
        (
            "class C { static { using a = res('s'); log.push('b'); } } log.join()",
            "b,s",
        ),
        (
            "function* g() { using a = res('a'); yield 1; log.push('after'); } \
             var it = g(); it.next(); log.push('mid'); it.next(); log.join()",
            "mid,after,a",
        ),
        (
            "function* g() { using a = res('a'); yield 1; } \
             var it = g(); it.next(); it.return(5); log.join()",
            "a",
        ),
        (
            "function* g() { using a = res('a'); yield 1; } \
             var it = g(); it.next(); try { it.throw(7); } catch (e) { log.push(e); } log.join()",
            "a,7",
        ),
        // Eine Ressource, die eine innere Funktion sieht.
        (
            "function f() { using a = res('a'); return () => a; } var g = f(); \
             [typeof g(), log].join()",
            "object,a",
        ),
        // NamedEvaluation.
        (
            "{ using x = class { static [Symbol.dispose]() {} }; log.push(x.name); } log.join()",
            "x",
        ),
    ]);
}

#[test]
fn loops_with_using_in_their_head() {
    check(&[
        (
            "for (using x of [res('a'), res('b')]) log.push('body'); log.join()",
            "body,a,body,b",
        ),
        (
            "for (using x of [res('a'), null, res('b')]) { if (x === null) continue; log.push('body'); } \
             log.join()",
            "body,a,body,b",
        ),
        // Erst wird die Ressource frei, dann der Iterator geschlossen.
        (
            "var it = { [Symbol.iterator]() { return this; }, next() { return { value: res('v') }; }, \
                        return() { log.push('close'); return {}; } }; \
             for (using x of it) break; log.join()",
            "v,close",
        ),
        (
            "var it = { [Symbol.iterator]() { return this; }, next() { return { value: res('v') }; }, \
                        return() { log.push('close'); return {}; } }; \
             try { for (using x of it) throw 1; } catch (e) { log.push(e); } log.join()",
            "v,close,1",
        ),
        // Ein Wert ohne @@dispose schließt den Iterator.
        (
            "var it = { [Symbol.iterator]() { return this; }, next() { return { value: 1 }; }, \
                        return() { log.push('close'); return {}; } }; \
             try { for (using x of it) log.push('body'); } catch (e) { log.push(e.constructor.name); } \
             log.join()",
            "close,TypeError",
        ),
        // Jeder Durchlauf hat seine eigene Bindung.
        (
            "var fs = []; for (using x of [res('a'), res('b')]) fs.push(() => x); \
             fs[0]() === fs[1]()",
            "false",
        ),
        ("for (using x of [null]) { x = 1; }", "TypeError"),
        ("var x = 1; for (using x of [x]) ;", "ReferenceError"),
        (
            "for (using x = res('a'), y = res('b'); log.length < 2; ) log.push('i'); log.join()",
            "i,i,b,a",
        ),
        (
            "try { for (using x = res('a'), y = 1; ; ) ; } catch (e) { log.push(e.constructor.name); } \
             log.join()",
            "a,TypeError",
        ),
        ("for (using x = res('a'); ; ) { break; } log.join()", "a"),
    ]);
}

#[test]
fn await_using_awaits_disposal() {
    check_async(&[
        (
            "async function f() { { await using a = { async [Symbol.asyncDispose]() { log.push('d'); } }; \
             log.push('b'); } log.push('after'); } f(); log.push('sync');",
            "b,d,sync,after",
        ),
        // Auf das Ergebnis von @@asyncDispose wird gewartet.
        (
            "async function f() { { await using a = { [Symbol.asyncDispose]() { \
                 return new Promise(r => { log.push('start'); r(); }).then(() => log.push('done')); } }; } \
             log.push('after'); } f(); log.push('sync');",
            "start,sync,done,after",
        ),
        // Auch mit *null* wartet `await using` einmal.
        (
            "async function f() { { await using a = null; } log.push('after'); } f(); log.push('sync');",
            "sync,after",
        ),
        // Wurde `await using` nicht erreicht, wartet nichts.
        (
            "async function f() { l: { if (true) break l; await using a = null; } log.push('after'); } \
             f(); log.push('sync');",
            "after,sync",
        ),
        // @@dispose statt @@asyncDispose: nicht abgewartet, aber in einem
        // Promise.
        (
            "async function f() { { await using a = { [Symbol.dispose]() { log.push('d'); \
                 return new Promise(() => {}); } }; } log.push('after'); } f(); log.push('sync');",
            "d,sync,after",
        ),
        (
            "async function f() { { await using a = { [Symbol.dispose]() { throw 'x'; } }; } } \
             f().catch(e => log.push('caught', e));",
            "caught,x",
        ),
        // @@asyncDispose geht vor @@dispose.
        (
            "async function f() { { await using a = { [Symbol.asyncDispose]() { log.push('async'); }, \
                 [Symbol.dispose]() { log.push('sync'); } }; } } f();",
            "async",
        ),
        // Abgelehnte Promises werden zu Fehlern beim Freigeben.
        (
            "async function f() { { await using a = { [Symbol.asyncDispose]() { return Promise.reject(2); } }; \
                 throw 1; } } f().catch(e => log.push(e.error, e.suppressed));",
            "2,1",
        ),
        // Vor `using` nach `await using` mit *null* kommt ein Await.
        (
            "async function f() { { using b = res('b'); await using a = null; } log.push('after'); } \
             f(); log.push('sync');",
            "sync,b,after",
        ),
        (
            "async function f() { { await using c = { async [Symbol.asyncDispose]() { log.push('c'); } }; \
                 using b = res('b'); await using a = null; } } f(); log.push('sync');",
            "sync,b,c",
        ),
        (
            "async function f() { for (await using x of [res('a'), null]) log.push('body'); \
             log.push('after'); } f(); log.push('sync');",
            "body,a,sync,body,after",
        ),
        (
            "async function f() { for await (using x of [res('a')]) log.push('body'); } f(); \
             log.push('sync');",
            "sync,body,a",
        ),
        (
            "async function* g() { await using a = { async [Symbol.asyncDispose]() { log.push('d'); } }; \
                 yield 1; } var it = g(); it.next().then(() => it.return()).then(() => log.push('ret'));",
            "d,ret",
        ),
        (
            "async function f() { await using a = 1; } f().catch(e => log.push(e.constructor.name));",
            "TypeError",
        ),
    ]);
}
