use super::*;
use crate::parser::{parse_module, parse_script};
use crate::pre_check::{Difference, compare};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn same(text: &str) {
    let source = units(text);
    assert_eq!(compare(&source, false, PreCheck::ALL), Ok(()), "{text}");
}

fn same_module(text: &str) {
    let source = units(text);
    assert_eq!(compare(&source, true, PreCheck::ALL), Ok(()), "{text}");
}

fn lazy_count(ast: &Ast) -> usize {
    (0..ast.function_count())
        .filter(|&index| ast.is_lazy(FunctionId::from_raw(index as u32)))
        .count()
}

#[test]
fn bodies_are_dropped_and_read_again() {
    let source = units(
        "function f(a, b) { var c = a + b; return function g() { return c * 2; }; }\n\
         var h = (x) => { return x.y.z; };",
    );
    let full = parse_script(&source).expect("ganz");
    let mut ast = parse_script_pre_checked(&source, PreCheck::ALL).expect("vorgeprüft");
    assert!(ast.node_count() < full.node_count());
    assert_eq!(lazy_count(&ast), 2, "f und die Pfeilfunktion, g liegt in f");
    assert_eq!(ast.function_count(), 2);
    let f = FunctionId::from_raw(0);
    assert!(
        ast.function(f)
            .is_some_and(|function| function.body.is_empty())
    );
    ast.expand(f).expect("nachlesen");
    assert!(!ast.is_lazy(f));
    assert_eq!(lazy_count(&ast), 2, "jetzt g und die Pfeilfunktion");
    // Nochmal ist nichts zu tun.
    let count = ast.node_count();
    ast.expand(f).expect("nichts zu tun");
    assert_eq!(ast.node_count(), count);
    ast.expand_all().expect("alle");
    assert_eq!(lazy_count(&ast), 0);
    assert_eq!(ast.node_count(), full.node_count());
    assert_eq!(ast.function_count(), full.function_count());
}

#[test]
fn what_stays_whole() {
    let source = units(
        "class A { x = function () { return 1; }; static { var s = 1; } }\n\
         var e = (a) => a + 1;",
    );
    let ast = parse_script_pre_checked(&source, PreCheck::ALL).expect("vorgeprüft");
    // Nur die Funktion im Feld hat einen Rumpf in Klammern.
    assert_eq!(lazy_count(&ast), 1);
    let none = PreCheck {
        min_nodes: usize::MAX,
        max_depth: MAX_DEPTH,
    };
    let ast = parse_script_pre_checked(&units("function f() { return 1; }"), none).expect("ganz");
    assert_eq!(lazy_count(&ast), 0);
}

#[test]
fn deep_bodies_stay_whole() {
    let shallow = "function f() { return 1; }";
    let deep = "function f() { return [[[[[[[[1]]]]]]]]; }";
    let pre_check = PreCheck {
        min_nodes: 0,
        max_depth: 6,
    };
    let ast = parse_script_pre_checked(&units(shallow), pre_check).expect("flach");
    assert_eq!(lazy_count(&ast), 1);
    let ast = parse_script_pre_checked(&units(deep), pre_check).expect("tief");
    assert_eq!(lazy_count(&ast), 0);
    // Eine innere Funktion zählt mit, denn das Nachlesen prüft sie wieder.
    let nested = "function f() { return function () { return [[[[[[[[1]]]]]]]]; }; }";
    let ast = parse_script_pre_checked(&units(nested), pre_check).expect("innen tief");
    assert_eq!(lazy_count(&ast), 0);
    same(deep);
    same(nested);
}

#[test]
fn early_errors_are_reported_at_once() {
    for text in [
        "function f() { let a; let a; }",
        "function f() { break; }",
        "function f() { x: x: ; }",
        "function f() { continue y; }",
        "function f(a, a) { 'use strict'; }",
        "function f(a = 1) { 'use strict'; }",
        "function f() { 'use strict'; with (a) {} }",
        "function f() { 'use strict'; 010; }",
        "function f() { return /(/; }",
        "function f() { return /a/gg; }",
        "class A { m() { return this.#x; } }",
        "function f() { class B { #y; m() { this.#z; } } }",
        "function f() { super.x; }",
        "function f() { new.target; }; () => { new.target; }",
        "function* g() { function h() { yield 1; } }",
        "async function f() { function g() { await 1; } }",
        "function f() { import.meta; }",
        "class A { static { function f() { arguments; } } x = () => { arguments; } }",
        "function f() { ({a = 1}); }",
        "function f() { a?.b = 1; }",
        "function f() { for (let x of y) { var x; } }",
        "function f() {",
        "function f() { return; } }",
    ] {
        let source = units(text);
        let full = parse_script(&source).err();
        let pre_checked = parse_script_pre_checked(&source, PreCheck::ALL).err();
        assert!(full.is_some(), "{text}");
        assert_eq!(full, pre_checked, "{text}");
        same(text);
    }
}

#[test]
fn analysis_matches_the_whole_tree() {
    for text in [
        // Namen, die innere Funktionen sehen
        "function f() { var a = 1, b = 2; function g() { return a; } return g; }",
        "function f() { var a; return function () { return function () { return a; }; }; }",
        "var t = 1; function f() { return t; } let u; function g() { u = 2; }",
        // this, arguments, new.target und super in Pfeilfunktionen
        "function f() { return () => { return () => { return this.x; }; }; }",
        "var g = () => { return () => { return this; }; };",
        "function f() { return () => { return arguments[0]; }; }",
        "function f() { return () => { return () => new.target; }; }",
        "class A extends Object { constructor() { var h = () => { super(); }; h(); } }",
        "class A { m() { return () => { return () => super.x; }; } }",
        "var o = { m() { return () => { return super.y; }; }, n() { return super.z; } };",
        "class A { m() { function g() { return this; } return g; } }",
        // direktes eval
        "function f(a) { return () => { return eval('a'); }; }",
        "function f() { function g() { eval(''); } return g; }",
        "var k = () => { eval('this'); };",
        "function f() { (eval)('x'); eval?.('y'); }",
        // with
        "function f(o) { with (o) { return function () { return x; }; } }",
        "function f(o) { with (o) { var y; return () => { let z; return z; }; } }",
        "with ({}) { var q = function () { var r = 1; return function () { return r; }; }; }",
        // Parameter mit Funktionen und Muster
        "function f(a = () => b, { c } = {}, ...[d]) { var b; return a() + c + d; }",
        "var p = (a = function () { return a; }) => { return a; };",
        "var q = async (x, y = x) => { await y; return () => x; };",
        "function f(a, b) { 'use strict'; return function () { return a; }; }",
        // Generatoren, async, Klassen
        "function* g() { var a = yield 1; yield* (function* () { yield a; })(); }",
        "async function* h() { for await (const x of y) { yield () => x; } }",
        "class B { #p = 1; static #q() {} get r() { return this.#p; } set r(v) { B.#q(); } }",
        "class C extends B { constructor(a) { super(a); this.f = () => super.r; } }",
        "class D { static s = () => { return this; }; m() { return class { n() { return D; } }; } }",
        // Anhang B
        "function f() { { function g() { return 1; } } return g(); }",
        "function f() { if (1) function g() {} return g; }",
        "function f() { try {} catch (e) { var e = 1; function h() { return e; } } }",
        // Marken, Schleifen, switch
        "function f() { a: for (;;) { b: while (1) { continue a; } break; } }",
        "function f(x) { switch (x) { case 1: let y = () => x; return y; default: return 0; } }",
        // Literale und Templates
        "function f() { return `a${() => 1}b${function () { return 2; }}c`; }",
        "function f() { return [...a, ...b, 1, { ...c }, /re/g, 1n, 'x']; }",
        // Spreads und Klammern vor und hinter vorgeprüften Rümpfen
        "function f() { [...a, ]; (b); } [...c] = d;",
        // Die Spread-Knoten bekommen dieselbe Nummer: Der Parser muss sich
        // den verworfenen vergessen.
        "function f() { [...(a), ]; } [...b] = c;",
        "var g = () => { [...(a), ]; }; [...b] = c;",
        "function f() { (a, b); } var g = (c, d) => { return c + d; }; [...e] = f;",
        "var o = { a() { [...x, 1]; }, b: function () { (y); } }; var [...z] = o;",
        "x = [function () { [...a, ]; }, ...b]; [...c] = x;",
        // verschachtelte Rümpfe in Methoden mit berechneten Namen
        "var o = { [function () { return 'k'; }()]() { return () => this; } };",
        "class E { [(() => { return 'm'; })()]() { return () => super.m; } }",
    ] {
        same(text);
    }
}

#[test]
fn modules_match_too() {
    for text in [
        "export function f() { return g(); } function g() { return import.meta; }",
        "export default function () { return () => this; }",
        "export default class { m() { return 1; } }",
        "import { a } from 'x'; export const b = () => { return a; };",
        "await 1; export async function h() { await 2; return () => h; }",
        "export let n = 1; function bump() { n += 1; }",
    ] {
        same_module(text);
    }
}

#[test]
fn eval_code_can_be_pre_checked() {
    let options = crate::parser::EvalOptions {
        pre_check: Some(PreCheck::ALL),
        new_target: true,
        super_property: true,
        ..crate::parser::EvalOptions::default()
    };
    let source =
        units("var f = () => { return () => super.x + new.target; }; var g = () => { this; };");
    let mut ast = crate::parser::parse_eval(&source, &options).expect("eval");
    assert_eq!(lazy_count(&ast), 2);
    let full_options = crate::parser::EvalOptions {
        pre_check: None,
        ..options
    };
    let full = crate::parser::parse_eval(&source, &full_options).expect("ganz");
    assert_eq!(ast.captures().script(), full.captures().script());
    ast.expand_all().expect("nachlesen");
    assert_eq!(ast.captures().script(), full.captures().script());
}

#[test]
fn a_changed_tree_is_left_alone() {
    let source = units("function f() { return 1; }");
    let mut ast = parse_script_pre_checked(&source, PreCheck::ALL).expect("vorgeprüft");
    let before = ast.node_count();
    // Ein Rumpf, der nicht mehr dort beginnt, wo er stand.
    if let Some(lazy) = ast
        .function_mut(FunctionId::from_raw(0))
        .and_then(|function| function.lazy.as_mut())
    {
        lazy.start = 1;
    }
    assert!(ast.expand(FunctionId::from_raw(0)).is_err());
    assert_eq!(ast.node_count(), before);
    assert!(ast.is_lazy(FunctionId::from_raw(0)));
}

#[test]
fn errors_are_compared_too() {
    let source = units("function f() { let a; let a; }");
    assert_eq!(compare(&source, false, PreCheck::ALL), Ok(()));
    let full = parse_module(&units("function f() { await; }")).err();
    assert!(full.is_some());
    assert!(matches!(
        compare(&units("function f() { 1; }"), false, PreCheck::ALL),
        Ok(())
    ));
    let _: Option<Difference> = None;
}
