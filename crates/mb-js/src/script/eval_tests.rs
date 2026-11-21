//! Tests für `eval` und `Function` (Schritt 9a): direktes und indirektes
//! `eval`, Bindungen aus `var` in Funktionen und global, Konflikte,
//! Anhang B und CreateDynamicFunction.

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
    for &(source, expected) in cases {
        let mut agent = Agent::with_options(Options {
            verify: true,
            ..Options::default()
        })
        .expect("Speicher");
        let realm = agent.create_realm().expect("Speicher");
        assert_eq!(run(&mut agent, realm, source), expected, "{source}");
    }
}

#[test]
fn eval_returns_the_completion_value() {
    check(&[
        ("eval('1 + 2')", "3"),
        ("eval('')", "undefined"),
        ("eval()", "undefined"),
        ("eval(42)", "42"),
        ("var o = {}; eval(o) === o", "true"),
        ("eval('1; if (true) {}')", "undefined"),
        ("eval('2; var x = 3;')", "2"),
        ("eval('\"a\" + \"b\"', 'ignoriert')", "\"ab\""),
        ("eval(...['4 * 5'])", "20"),
        ("eval('(')", "SyntaxError"),
        ("try { eval('throw 7') } catch (e) { e }", "7"),
        ("typeof eval", "\"function\""),
        ("eval.length", "1"),
        ("eval.name", "\"eval\""),
    ]);
}

#[test]
fn direct_eval_sees_the_bindings_at_the_call() {
    check(&[
        (
            "function f(a) { let b = 2; return eval('a + b'); } f(1)",
            "3",
        ),
        (
            "function f() { let x = 1; { let x = 2; return eval('x'); } } f()",
            "2",
        ),
        (
            "function f() { var x = 1; eval('x = 5'); return x; } f()",
            "5",
        ),
        ("let g = 10; function f() { return eval('g'); } f()", "10"),
        (
            "function f() { return eval('this'); } f.call(7) == 7",
            "true",
        ),
        (
            "var o = { m() { return eval('this'); } }; o.m() === o",
            "true",
        ),
        (
            "function f() { 'use strict'; return () => eval('this'); } f.call('s')()",
            "\"s\"",
        ),
        (
            "function f() { return eval('arguments.length'); } f(1, 2, 3)",
            "3",
        ),
        (
            "function f() { return eval('new.target'); } new f() !== undefined",
            "true",
        ),
        (
            "function f() { return eval('new.target'); } f()",
            "undefined",
        ),
        ("eval('new.target')", "SyntaxError"),
        (
            "function f() { let x = 1; return eval('(function () { return x; })')(); } f()",
            "1",
        ),
        (
            "function f() { const c = 1; eval('c = 2'); } f()",
            "TypeError",
        ),
        (
            "function f() { eval('x'); let x = 1; } f()",
            "ReferenceError",
        ),
    ]);
}

#[test]
fn indirect_eval_sees_only_the_global_environment() {
    check(&[
        (
            "var x = 'global'; function f() { var x = 'local'; return (0, eval)('x'); } f()",
            "\"global\"",
        ),
        (
            "function f() { var y = 1; return (0, eval)('typeof y'); } f()",
            "\"undefined\"",
        ),
        ("(0, eval)('this') === globalThis", "true"),
        (
            "var e = eval; function f() { e('var g1 = 1'); } f(); g1",
            "1",
        ),
        ("'use strict'; (0, eval)('var g2 = 2'); g2", "2"),
        ("(0, eval)('let l = 1'); typeof l", "\"undefined\""),
    ]);
}

#[test]
fn sloppy_eval_declares_vars_in_the_function() {
    check(&[
        ("function f() { eval('var x = 1'); return x; } f()", "1"),
        (
            "function f() { eval('var x = 1'); return typeof x; } f()",
            "\"number\"",
        ),
        (
            "function f() { eval('var x = 1'); return delete x; } f()",
            "true",
        ),
        (
            "function f() { eval('var x = 1'); delete x; return typeof x; } f()",
            "\"undefined\"",
        ),
        (
            "var x = 'g'; function f() { eval('var x = 1'); return x; } f() + x",
            "\"1g\"",
        ),
        (
            "function f() { eval('function h() { return 4; }'); return h(); } f()",
            "4",
        ),
        ("function f() { { eval('var x = 1'); } return x; } f()", "1"),
        (
            "function f() { eval('var x = 1'); return () => x; } f()()",
            "1",
        ),
        (
            "function f() { eval('var x = 1'); return function () { return x; }; } f()()",
            "1",
        ),
        (
            "function f() { eval('var x = 1'); x = 2; return eval('x'); } f()",
            "2",
        ),
        (
            "function f() { var x; eval('var x = 3'); return x; } f()",
            "3",
        ),
        ("function f(a) { eval('var a = 5'); return a; } f(1)", "5"),
        (
            "function f() { eval('var arguments'); return typeof arguments; } f()",
            "\"object\"",
        ),
        ("function f() { eval('var x'); return x; } f()", "undefined"),
    ]);
}

#[test]
fn eval_at_script_level_declares_deletable_globals() {
    check(&[
        ("eval('var g = 1'); g", "1"),
        ("eval('var g = 1'); delete g", "true"),
        ("eval('var g = 1'); delete g; typeof g", "\"undefined\""),
        ("var v = 1; eval('var v = 2'); delete v", "false"),
        ("eval('function h() { return 3; }'); h()", "3"),
        (
            "eval('function h() {}'); Object.getOwnPropertyDescriptor(globalThis, 'h').configurable",
            "true",
        ),
        ("let l; eval('var l')", "SyntaxError"),
        ("let l; eval('function l() {}')", "SyntaxError"),
        (
            "Object.defineProperty(globalThis, 'nc', { value: 1 }); eval('function nc() {}')",
            "TypeError",
        ),
        (
            "Object.preventExtensions(globalThis); eval('var neu')",
            "TypeError",
        ),
    ]);
}

#[test]
fn strict_eval_keeps_its_declarations() {
    check(&[
        ("'use strict'; eval('var x = 1'); typeof x", "\"undefined\""),
        (
            "eval('\"use strict\"; var x = 1'); typeof x",
            "\"undefined\"",
        ),
        (
            "function f() { 'use strict'; eval('var x = 1'); return typeof x; } f()",
            "\"undefined\"",
        ),
        ("'use strict'; eval('var x = 1; x')", "1"),
        ("'use strict'; eval('function h() { return 2; } h()')", "2"),
        ("eval('let x = 1; x')", "1"),
        ("eval('let x = 1'); typeof x", "\"undefined\""),
        (
            "function f() { eval('let y = 1'); return typeof y; } f()",
            "\"undefined\"",
        ),
        ("'use strict'; eval('with ({}) {}')", "SyntaxError"),
    ]);
}

#[test]
fn eval_declarations_conflicting_with_lexical_bindings_throw() {
    check(&[
        ("function f() { let x; eval('var x'); } f()", "SyntaxError"),
        (
            "function f() { let x; { eval('var x'); } } f()",
            "SyntaxError",
        ),
        (
            "function f() { { let x; eval('var x'); } } f()",
            "SyntaxError",
        ),
        (
            "function f() { { let x; eval('function x() {}'); } } f()",
            "SyntaxError",
        ),
        (
            "function f() { try { throw 1; } catch (x) { eval('var x = 2'); return x; } } f()",
            "2",
        ),
        (
            "function f() { try { throw 1; } catch (x) { eval('var x = 2'); } return x; } f()",
            "undefined",
        ),
        ("function f(a = eval('var a = 1')) {} f()", "SyntaxError"),
        (
            "function f() { let x; eval('var y'); return typeof y; } f()",
            "\"undefined\"",
        ),
    ]);
}

#[test]
fn eval_in_parameters_declares_outside_the_body() {
    check(&[
        (
            "function f(a = eval('var z = 2'), b = z) { return b; } f()",
            "2",
        ),
        (
            "function f(a = eval('var z = 2')) { var z; return z; } f()",
            "undefined",
        ),
        (
            "function f(a = eval('var z = 2'), b = () => z) { var z = 3; return b(); } f()",
            "2",
        ),
    ]);
}

#[test]
fn annex_b_hoists_functions_in_blocks_of_eval_code() {
    check(&[
        ("eval('{ function bf() { return 1; } }'); bf()", "1"),
        (
            "function f() { eval('{ function bf() { return 2; } }'); return bf(); } f()",
            "2",
        ),
        (
            "function f() { eval('if (true) function bf() { return 3; }'); return bf(); } f()",
            "3",
        ),
        (
            "function f() { let bf = 1; eval('{ function bf() {} }'); return bf; } f()",
            "1",
        ),
        (
            "'use strict'; eval('{ function bf() {} }'); typeof bf",
            "\"undefined\"",
        ),
        ("let bf = 5; eval('{ function bf() {} }'); bf", "5"),
    ]);
}

#[test]
fn nested_eval_reaches_the_same_var_scope() {
    check(&[
        (
            "function f() { eval('eval(\"var x = 1\")'); return x; } f()",
            "1",
        ),
        (
            "function f() { var y = 2; return eval('eval(\"y\")'); } f()",
            "2",
        ),
        ("eval('eval(\"var n = 3\")'); n", "3"),
        (
            "function f() { return eval('(() => eval(\"this\"))()'); } f.call(9) == 9",
            "true",
        ),
    ]);
}

#[test]
fn eval_sees_super_and_private_names_in_classes() {
    check(&[
        (
            "class A { m() { return 1; } } class B extends A { m() { return eval('super.m()'); } } new B().m()",
            "1",
        ),
        (
            "class A { constructor() { this.a = 1; } } class B extends A { constructor() { eval('super()'); } } new B().a",
            "1",
        ),
        (
            "class C { #p = 4; m() { return eval('this.#p'); } } new C().m()",
            "4",
        ),
        (
            "class C { m() { return eval('this.#q'); } } new C().m()",
            "SyntaxError",
        ),
        (
            "function f() { return eval('super.x'); } f()",
            "SyntaxError",
        ),
        ("class C { f = eval('arguments'); } new C()", "SyntaxError"),
        (
            "class C { f = eval('this'); } new C().f instanceof C",
            "true",
        ),
    ]);
}

#[test]
fn eval_is_direct_only_through_the_name() {
    check(&[
        (
            "var x = 'g'; function f() { var x = 'l'; return eval?.('x'); } f()",
            "\"g\"",
        ),
        (
            "var x = 'g'; function f() { var x = 'l'; return (eval)('x'); } f()",
            "\"l\"",
        ),
        (
            "var x = 'g'; function f() { var x = 'l'; var e = eval; return e('x'); } f()",
            "\"g\"",
        ),
        (
            "var x = 'g'; function f(eval) { var x = 'l'; return eval('x'); } f(globalThis.eval)",
            "\"l\"",
        ),
        (
            "function f() { var eval = (s) => 'eigen ' + s; return eval('x'); } f()",
            "\"eigen x\"",
        ),
    ]);
}

#[test]
fn function_constructor_creates_global_functions() {
    check(&[
        ("Function('a', 'b', 'return a + b')(1, 2)", "3"),
        ("new Function('a, b', 'return a * b')(3, 4)", "12"),
        ("Function()()", "undefined"),
        ("Function('return this')() === globalThis", "true"),
        (
            "var x = 'g'; function f() { var x = 'l'; return Function('return x')(); } f()",
            "\"g\"",
        ),
        ("Function('a', 'return a').length", "1"),
        ("Function('').name", "\"anonymous\""),
        (
            "Function('a', 'b', 'return 1').toString()",
            "\"function anonymous(a,b\\n) {\\nreturn 1\\n}\"",
        ),
        ("Function('/*', '*/){')", "SyntaxError"),
        ("Function('a = 1', 'return a')()", "1"),
        ("Function('\"use strict\"; return this')()", "undefined"),
        ("Function('a', 'a', '\"use strict\";')", "SyntaxError"),
        ("Function('super()')", "SyntaxError"),
        ("Function('-->')", "Objekt"),
        (
            "Object.getPrototypeOf(Function('')) === Function.prototype",
            "true",
        ),
        ("typeof Function('').prototype", "\"object\""),
    ]);
}

#[test]
fn generator_and_async_constructors_exist() {
    check(&[
        (
            "var G = Object.getPrototypeOf(function* () {}).constructor; G.name",
            "\"GeneratorFunction\"",
        ),
        (
            "var G = Object.getPrototypeOf(function* () {}).constructor; G('yield 1')().next().value",
            "1",
        ),
        (
            "var G = Object.getPrototypeOf(function* () {}).constructor; Object.getPrototypeOf(G) === Function",
            "true",
        ),
        (
            "var G = Object.getPrototypeOf(function* () {}).constructor; G.prototype === Object.getPrototypeOf(function* () {})",
            "true",
        ),
        (
            "var A = Object.getPrototypeOf(async function () {}).constructor; A.name",
            "\"AsyncFunction\"",
        ),
        (
            "var A = Object.getPrototypeOf(async function () {}).constructor; A('await 1') instanceof A",
            "true",
        ),
        (
            "var A = Object.getPrototypeOf(async function* () {}).constructor; A.name",
            "\"AsyncGeneratorFunction\"",
        ),
        (
            "var G = Object.getPrototypeOf(function* () {}).constructor; G('yield', '')",
            "SyntaxError",
        ),
        (
            "var A = Object.getPrototypeOf(async function () {}).constructor; A('await', '')",
            "SyntaxError",
        ),
        (
            "var G = Object.getPrototypeOf(function* () {}).constructor; Object.getOwnPropertyDescriptor(G.prototype, 'constructor').writable",
            "false",
        ),
        (
            "class D extends Function {} var d = new D('return 5'); (d instanceof D) + ':' + d()",
            "\"true:5\"",
        ),
    ]);
}
