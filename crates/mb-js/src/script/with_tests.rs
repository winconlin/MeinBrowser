//! Tests für `with` und @@unscopables (Schritt 9b).

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
        Kind::String(string) => {
            let text = String::from_utf16(&heap.string_units(string)).expect("UTF-16");
            format!("{text:?}")
        }
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

fn check(cases: &[(&str, &str)]) {
    let mut failures = Vec::new();
    for &(source, expected) in cases {
        let mut agent = Agent::with_options(Options {
            verify: true,
            ..Options::default()
        })
        .expect("Speicher");
        let realm = agent.create_realm().expect("Speicher");
        let actual = run(&mut agent, realm, source);
        if actual != expected {
            failures.push(format!(
                "{source}\n  erwartet {expected}, bekommen {actual}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn names_resolve_through_the_object_first() {
    check(&[
        ("var o = { x: 1 }; var x = 2; with (o) x", "1"),
        ("var o = {}; var x = 2; with (o) x", "2"),
        (
            "function f() { var x = 2; with ({ x: 1 }) return x; } f()",
            "1",
        ),
        ("function f() { var x = 2; with ({}) return x; } f()", "2"),
        (
            "function f() { let x = 2; with ({}) { x = 3; } return x; } f()",
            "3",
        ),
        ("var o = { x: 1 }; with (o) { x = 5; } o.x", "5"),
        (
            "var o = {}; with (o) { var y = 4; } [o.y, y].join()",
            "\",4\"",
        ),
        (
            "var o = { y: 0 }; with (o) { var y = 4; } [o.y, typeof y].join()",
            "\"4,undefined\"",
        ),
        ("with (null) {}", "TypeError"),
        ("with (undefined) {}", "TypeError"),
        ("with (1) toFixed(1)", "\"1.0\""),
        ("1; with ({}) {}", "undefined"),
        ("1; with ({}) { 2; }", "2"),
        ("with ('ab') length", "2"),
    ]);
}

#[test]
fn calls_take_the_object_as_this() {
    check(&[
        (
            "var o = { f() { return this; } }; with (o) f() === o",
            "true",
        ),
        (
            "var o = { f() { return this; } }; with (o) (f)() === o",
            "true",
        ),
        (
            "function f() { return this; } with ({}) f() === this",
            "true",
        ),
        (
            "var o = { f() { 'use strict'; return this; } }; with ({ __proto__: o }) typeof f()",
            "\"object\"",
        ),
        (
            "function g() { 'use strict'; return this; } with ({}) typeof g()",
            "\"undefined\"",
        ),
        (
            "var o = { eval(x) { return [this === o, x].join(); } }; with (o) eval(1)",
            "\"true,1\"",
        ),
        (
            "var x = 'global'; function f() { var x = 'local'; with ({}) return eval('x'); } f()",
            "\"local\"",
        ),
        (
            "function f() { with ({ x: 3 }) return eval('x'); } f()",
            "3",
        ),
    ]);
}

#[test]
fn unscopables_hide_names() {
    check(&[
        (
            "var x = 1; var o = { x: 2, [Symbol.unscopables]: { x: true } }; with (o) x",
            "1",
        ),
        (
            "var x = 1; var o = { x: 2, [Symbol.unscopables]: { x: 0 } }; with (o) x",
            "2",
        ),
        (
            "var x = 1; var o = { x: 2, [Symbol.unscopables]: true }; with (o) x",
            "2",
        ),
        ("var values = 1; with ([]) values", "1"),
        ("with ([]) typeof push", "\"function\""),
    ]);
}

#[test]
fn closures_keep_the_object() {
    check(&[
        (
            "var o = { x: 1 }; var f; with (o) f = function () { return x; }; o.x = 7; f()",
            "7",
        ),
        (
            "var o = { x: 1 }; var f; with (o) f = () => x; delete o.x; var x = 9; f()",
            "9",
        ),
        (
            "function f() { var a = 1; with ({}) { return (function () { return a; })(); } } f()",
            "1",
        ),
        (
            "var r = []; for (var i = 0; i < 2; i++) with ({ i: i * 10 }) r.push(i); r.join()",
            "\"0,10\"",
        ),
    ]);
}

#[test]
fn typeof_delete_and_updates() {
    check(&[
        ("with ({ x: 1 }) typeof x", "\"number\""),
        ("with ({}) typeof nichtDa", "\"undefined\""),
        ("var o = { x: 1 }; with (o) delete x; 'x' in o", "false"),
        ("var o = { x: 1 }; with (o) { x++; x += 2; } o.x", "4"),
        ("with ({}) nichtDa", "ReferenceError"),
        (
            "var o = { x: 1 }; with (o) { x = (delete o.x, 2); } o.x",
            "2",
        ),
        (
            "function f(a) { with ({}) { return arguments.length + a; } } f(5)",
            "6",
        ),
        ("try { with ({ x: 1 }) throw x; } catch (e) { e }", "1"),
        ("var r; l: with ({ x: 1 }) { r = x; break l; } r", "1"),
        (
            "var o = { x: 1 }; function* g() { with (o) { yield x; yield x; } } var it = g(); it.next(); o.x = 2; it.next().value",
            "2",
        ),
    ]);
}

#[test]
fn var_and_functions_in_the_body() {
    check(&[
        (
            "var obj = { t: 1 }; with (obj) { var t = delete obj.t; } [obj.t, t].join()",
            "\"true,\"",
        ),
        ("with ({}) { p5 = 'x5'; } p5", "\"x5\""),
        ("this.p2 = 2; with ({}) { this.p2 = 'x'; } p2", "\"x\""),
        ("with ({}) { var p4 = 'x4'; } p4", "\"x4\""),
        (
            "with ({ value: 1 }) { var value = 'v'; } typeof value",
            "\"undefined\"",
        ),
        (
            "function f() { with ({ v: 1 }) { var v = 2; } return typeof v; } f()",
            "\"undefined\"",
        ),
        (
            "function f() { with ({}) { var v = 2; } return v; } f()",
            "2",
        ),
        ("with ({}) { function g() { return 3; } } g()", "3"),
        (
            "function f() { with ({}) { function g() { return 4; } } return g(); } f()",
            "4",
        ),
        (
            "var o = { g: 1 }; with (o) { function g() {} } typeof o.g",
            "\"number\"",
        ),
    ]);
}

/// Ein Aufruf eines Bezeichners `eval` in Endposition, dessen Wert nicht
/// %eval% ist, ersetzt den Rahmen wie jeder andere Endaufruf.
#[test]
fn calls_named_eval_in_tail_position_replace_the_frame() {
    let mut agent = Agent::with_options(Options::default()).expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    let budget = std::rc::Rc::new(crate::agent::Budget::unlimited());
    agent.with_execution_budget(budget, |agent| {
        for (source, expected) in [
            (
                "function outer() { var eval = function (n) { 'use strict'; \
                 return n === 0 ? 'ende' : eval(n - 1); }; return eval(150000); } outer()",
                "\"ende\"",
            ),
            (
                "var scope = {}, f; with (scope) { f = function (n) { 'use strict'; \
                 return n === 0 ? this === scope : eval(n - 1); }; } scope.eval = f; f(150000)",
                "true",
            ),
        ] {
            assert_eq!(run(agent, realm, source), expected, "{source}");
        }
    });
}
