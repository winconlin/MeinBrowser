use super::*;
use crate::agent::Exception;
use crate::heap::Options;
use crate::number;
use crate::value::Kind;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn agent_with(options: Options) -> (Agent, RealmId) {
    let mut agent = Agent::with_options(options).expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    (agent, realm)
}

fn checked() -> (Agent, RealmId) {
    agent_with(Options {
        verify: true,
        ..Options::default()
    })
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

/// Führt ein Script aus und beschreibt das Ergebnis: den Wert, die Art des
/// Fehlers der Engine, `throw` mit dem geworfenen Wert oder den Grund, aus
/// dem es nicht lief. Danach ist der Registerstapel wieder leer.
fn run(agent: &mut Agent, realm: RealmId, source: &str) -> String {
    let result = agent.scope(|agent, scope| {
        let result = agent.evaluate_script(scope, realm, &units(source));
        match result {
            Ok(value) => describe(agent, value),
            Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
                Some(Exception::Error(kind)) => format!("{kind:?}"),
                Some(Exception::Value(value)) => format!("throw {}", describe(agent, value)),
                None => "throw ohne Ausnahme".to_owned(),
            },
            Err(error) => format!("{error:?}"),
        }
    });
    assert_eq!(agent.heap_mut().push_registers(0), Some(0), "{source}");
    result
}

fn eval(source: &str) -> String {
    let (mut agent, realm) = checked();
    run(&mut agent, realm, source)
}

fn check(cases: &[(&str, &str)]) {
    for &(source, expected) in cases {
        assert_eq!(eval(source), expected, "{source}");
    }
}

#[test]
fn the_completion_value_is_the_last_value() {
    check(&[
        ("", "undefined"),
        ("1;;", "1"),
        ("1; {}", "1"),
        ("1; var x;", "1"),
        ("1; var x = 2;", "1"),
        ("1; let y = 2;", "1"),
        ("1; { 2; let z; }", "2"),
        ("1; debugger;", "1"),
        // `if` und Schleifen ergeben nie ~empty~.
        ("1; if (true) {}", "undefined"),
        ("1; if (false) 2;", "undefined"),
        ("1; if (true) 2; else 3;", "2"),
        ("1; if (false) 2; else 3;", "3"),
        ("1; while (false) {}", "undefined"),
        ("var i = 0; while (i < 3) { i++; }", "2"),
        ("var i = 0; do { i += 2; } while (i < 5)", "6"),
        ("for (var i = 0; i < 3; i++) i * 10;", "20"),
        ("var i; for (i = 0; i < 3; i++) {} i", "3"),
        ("1; for (2; false;);", "undefined"),
        (
            "for (let i = 0; i < 3; i++) { if (i == 1) break; i; }",
            "undefined",
        ),
        ("1; do { 2; break; } while (false)", "2"),
        ("1; do { break; } while (false)", "undefined"),
        ("1; do { 2; { break; } } while (false)", "2"),
        ("1; a: { 2; break a; }", "2"),
        ("1; a: { break a; }", "1"),
        (
            "outer: for (var i = 0; i < 2; i++) { for (var j = 0; j < 2; j++) { i * 10 + j; continue outer; } }",
            "10",
        ),
        (
            "var n = 0; a: b: while (true) { n++; if (n < 3) continue a; break b; } n",
            "3",
        ),
    ]);
}

#[test]
fn switch_compares_cases_then_default() {
    check(&[
        (
            "switch (2) { case 1: 'a'; case 2: 'b'; case 3: 'c'; }",
            "\"c\"",
        ),
        (
            "switch (2) { case 1: 'a'; case 2: 'b'; break; case 3: 'c'; }",
            "\"b\"",
        ),
        (
            "switch (5) { case 1: 'a'; default: 'd'; case 2: 'b'; }",
            "\"b\"",
        ),
        (
            "switch (2) { case 1: 'a'; default: 'd'; case 2: 'b'; }",
            "\"b\"",
        ),
        ("switch (5) { case 1: 'a'; }", "undefined"),
        ("1; switch (0) {}", "undefined"),
        (
            "switch ('1') { case 1: 'Zahl'; break; case '1': 'Text'; }",
            "\"Text\"",
        ),
        (
            "var log = ''; switch (3) { case (log += 'a', 1): break; default: log += 'd'; case (log += 'b', 3): log += '!'; } log",
            "\"ab!\"",
        ),
        (
            "var log = ''; switch (9) { case (log += 'a', 1): break; default: log += 'd'; case (log += 'b', 3): log += '!'; } log",
            "\"abd!\"",
        ),
        (
            "for (var i = 0; i < 3; i++) { switch (i) { case 1: continue; } i; }",
            "2",
        ),
    ]);
}

#[test]
fn numbers_and_strings() {
    check(&[
        ("1 + 2 * 3", "7"),
        ("(1 + 2) * 3", "9"),
        ("2 ** 10", "1024"),
        ("2 ** 3 ** 2", "512"),
        ("7 % 3", "1"),
        ("-7 % 3", "-1"),
        ("1 / 0", "Infinity"),
        ("1 / -0", "-Infinity"),
        ("0 / 0", "NaN"),
        ("0.1 + 0.2", "0.30000000000000004"),
        ("2147483647 + 1", "2147483648"),
        ("'a' + 1", "\"a1\""),
        ("1 + '2'", "\"12\""),
        ("'3' * '4'", "12"),
        ("'5' - 2", "3"),
        ("true + 1", "2"),
        ("null + 1", "1"),
        ("undefined + 1", "NaN"),
        ("5 & 3", "1"),
        ("5 | 3", "7"),
        ("5 ^ 3", "6"),
        ("~5", "-6"),
        ("1 << 31", "-2147483648"),
        ("-1 >>> 0", "4294967295"),
        ("-8 >> 1", "-4"),
        ("+'3'", "3"),
        ("-'3'", "-3"),
        ("+''", "0"),
        ("`a${1 + 1}b${'c'}`", "\"a2bc\""),
        ("`${1}${2}`", "\"12\""),
        ("`x`", "\"x\""),
        ("`${null} ${true}`", "\"null true\""),
        ("(1, 2)", "2"),
    ]);
}

#[test]
fn comparisons_and_logic() {
    check(&[
        ("1 < 2", "true"),
        ("'a' < 'b'", "true"),
        ("'10' < '9'", "true"),
        ("10 < 9", "false"),
        ("NaN < 1", "false"),
        ("NaN >= 1", "false"),
        ("1 <= 1", "true"),
        ("2 > 1", "true"),
        ("1 >= 2", "false"),
        ("null == undefined", "true"),
        ("null === undefined", "false"),
        ("'1' == 1", "true"),
        ("NaN != NaN", "true"),
        ("0 === -0", "true"),
        ("1 !== '1'", "true"),
        ("0 || 'x'", "\"x\""),
        ("1 && 2", "2"),
        ("0 && nothing", "0"),
        ("1 || nothing", "1"),
        ("null ?? 3", "3"),
        ("0 ?? 3", "0"),
        ("!0", "true"),
        ("1 ? 'j' : nothing", "\"j\""),
        ("0 ? nothing : 'n'", "\"n\""),
        ("void 1", "undefined"),
    ]);
}

#[test]
fn typeof_names_the_type() {
    check(&[
        ("typeof 1", "\"number\""),
        ("typeof 's'", "\"string\""),
        ("typeof true", "\"boolean\""),
        ("typeof undefined", "\"undefined\""),
        ("typeof null", "\"object\""),
        ("typeof globalThis", "\"object\""),
        ("typeof nothing", "\"undefined\""),
        ("typeof (nothing)", "\"undefined\""),
        ("let t = 1; typeof t", "\"number\""),
        ("{ let t = 's'; typeof t; }", "\"string\""),
    ]);
}

#[test]
fn assignments_and_updates() {
    check(&[
        ("var a = 1; a++", "1"),
        ("var a = 1; a++; a", "2"),
        ("var a = 1; ++a", "2"),
        ("var a = '5'; a++; a", "6"),
        ("var a = '5'; a++", "5"),
        ("var a = 1; a--; a", "0"),
        ("var a = 2; a *= 3", "6"),
        ("var a = 'x'; a += 1", "\"x1\""),
        ("var a = 2; a **= 3; a", "8"),
        ("var a = 0; a ||= 5", "5"),
        ("var a = 1; a &&= 7", "7"),
        ("var a = null; a ??= 9; a", "9"),
        ("var a = 1; a ??= nothing", "1"),
        ("let b = 1; b += 1; b", "2"),
        ("{ let b = 1; b <<= 3; b; }", "8"),
        ("var a, b; a = b = 3; a + b", "6"),
        ("x = 5; x", "5"),
        // Kurzschluss schreibt nicht, also auch nicht in die Konstante.
        ("const c = 1; c ||= 2", "1"),
        ("const c = 0; c ||= 2", "TypeError"),
    ]);
}

#[test]
fn global_bindings() {
    check(&[
        ("this === globalThis", "true"),
        ("'use strict'; this === globalThis", "true"),
        ("globalThis.x", "Unsupported(0)"),
        ("NaN", "NaN"),
        ("Infinity", "Infinity"),
        ("undefined = 1; undefined", "undefined"),
        ("'use strict'; undefined = 1", "TypeError"),
        ("'use strict'; y = 5", "ReferenceError"),
        ("'use strict'; var y; y = 5", "5"),
        ("nothing", "ReferenceError"),
        ("nothing = 1; nothing", "1"),
        ("delete globalThis; typeof globalThis", "\"undefined\""),
        ("delete NaN", "false"),
        ("delete nothing", "true"),
        ("var v = 1; delete v", "false"),
        ("w = 1; delete w", "true"),
        ("w = 1; delete w; typeof w", "\"undefined\""),
        ("let l = 1; delete l", "false"),
        ("{ let b; delete b; }", "false"),
        ("delete 1", "true"),
        ("delete (0, nothing)", "ReferenceError"),
    ]);
}

#[test]
fn bindings_have_a_temporal_dead_zone() {
    check(&[
        ("x; let x = 1;", "ReferenceError"),
        ("x = 2; let x = 1;", "ReferenceError"),
        ("{ x; let x; }", "ReferenceError"),
        ("{ x = 2; let x; }", "ReferenceError"),
        ("let x = x;", "ReferenceError"),
        ("{ let x = x; }", "ReferenceError"),
        ("{ let a = b, b = 1; }", "ReferenceError"),
        ("{ let a = 1, b = a; b; }", "1"),
        ("typeof z; let z;", "ReferenceError"),
        ("{ typeof z; let z; }", "ReferenceError"),
        ("{ let y = 1; y; }", "1"),
        ("{ let y; y; }", "undefined"),
        ("let y; y", "undefined"),
        (
            "switch (1) { case 0: let q = 1; case 1: q; }",
            "ReferenceError",
        ),
        (
            "switch (1) { case 0: let q = 1; case 1: q = 2; }",
            "ReferenceError",
        ),
        ("switch (1) { case 1: let q = 1; q; }", "1"),
        ("for (let i = 0; i < 2; i++) { let j = i; j; }", "1"),
        (
            "let s = 0; for (let i = 0; i < 4; i++) { let i = 10; s += i; } s",
            "40",
        ),
        ("{ let x = 1; { let x = 2; } x; }", "1"),
        ("{ let x = 1; { x; let x = 2; } }", "ReferenceError"),
        ("let x = 1; { let x = 2; } x", "1"),
    ]);
}

#[test]
fn constants_cannot_be_assigned() {
    check(&[
        ("const c = 1; c", "1"),
        ("const c = 1; c = 2", "TypeError"),
        ("const c = 1; c++", "TypeError"),
        ("{ const c = 1; c = 2; }", "TypeError"),
        ("{ const c = 1; c += 1; }", "TypeError"),
        ("{ c = 2; const c = 1; }", "ReferenceError"),
        ("for (const i = 0; i < 1; i++) {}", "TypeError"),
        ("for (const i = 0; i < 1;) { i; break; }", "0"),
    ]);
}

#[test]
fn throw_and_early_errors() {
    check(&[
        ("throw 'x'", "throw \"x\""),
        ("throw 1 + 1", "throw 2"),
        ("1; throw 2; 3", "throw 2"),
        ("for (;;) throw 'aus'", "throw \"aus\""),
    ]);
    let syntax = eval("1 +");
    assert!(syntax.starts_with("Syntax("), "{syntax}");
    let redeclared = eval("let a; var a;");
    assert!(redeclared.starts_with("Syntax("), "{redeclared}");
    assert_eq!(eval("function f() {}"), "Unsupported(0)");
}

#[test]
fn scripts_share_the_global_environment() {
    let (mut agent, realm) = checked();
    assert_eq!(
        run(&mut agent, realm, "var a = 1; let b = 2; const c = 3;"),
        "undefined"
    );
    assert_eq!(run(&mut agent, realm, "a + b + c"), "6");
    assert_eq!(run(&mut agent, realm, "b = 5; b"), "5");
    assert_eq!(run(&mut agent, realm, "c = 5"), "TypeError");
    // Bindungen aus `let` liegen nicht im globalen Objekt.
    assert_eq!(run(&mut agent, realm, "delete b; b"), "5");

    // GlobalDeclarationInstantiation lehnt Konflikte zwischen Scripts ab.
    assert_eq!(run(&mut agent, realm, "let a;"), "SyntaxError");
    assert_eq!(run(&mut agent, realm, "var b;"), "SyntaxError");
    assert_eq!(run(&mut agent, realm, "let c;"), "SyntaxError");
    assert_eq!(run(&mut agent, realm, "let NaN;"), "SyntaxError");
    assert_eq!(run(&mut agent, realm, "var NaN; NaN"), "NaN");
    // Dann entsteht keine der Bindungen.
    assert_eq!(run(&mut agent, realm, "let fresh; let b;"), "SyntaxError");
    assert_eq!(run(&mut agent, realm, "typeof fresh"), "\"undefined\"");
    assert_eq!(run(&mut agent, realm, "var other; let a;"), "SyntaxError");
    assert_eq!(run(&mut agent, realm, "typeof other"), "\"undefined\"");

    // Eine konfigurierbare Eigenschaft lässt sich überdecken.
    assert_eq!(
        run(&mut agent, realm, "implicit = 1; let globalThis = 2;"),
        "1"
    );
    assert_eq!(run(&mut agent, realm, "let implicit = 3; implicit"), "3");
    assert_eq!(run(&mut agent, realm, "globalThis"), "2");

    // Bricht ein Script ab, bleiben seine Bindungen in der TDZ.
    assert_eq!(run(&mut agent, realm, "throw 0; let late = 1;"), "throw 0");
    assert_eq!(run(&mut agent, realm, "late"), "ReferenceError");
    assert_eq!(run(&mut agent, realm, "late = 1"), "ReferenceError");
    assert_eq!(run(&mut agent, realm, "typeof late"), "ReferenceError");
}

#[test]
fn realms_have_their_own_globals() {
    let (mut agent, first) = checked();
    let second = agent.create_realm().expect("Speicher");
    assert_eq!(run(&mut agent, first, "var a = 1; let b = 2;"), "undefined");
    assert_eq!(
        run(&mut agent, second, "typeof a + typeof b"),
        "\"undefinedundefined\""
    );
    assert_eq!(run(&mut agent, second, "let a = 3; a"), "3");
    assert_eq!(run(&mut agent, first, "a"), "1");
}

#[test]
fn loops_run_to_the_end() {
    check(&[
        (
            "var s = 0; for (var i = 0; i < 1000; i++) s += i; s",
            "499500",
        ),
        (
            "var s = ''; for (let i = 0; i < 5; i++) { if (i == 2) continue; s += i; } s",
            "\"0134\"",
        ),
        ("var i = 0; while (true) { if (++i > 9) break; } i", "10"),
        ("var i = 0; do ; while (i++ < 9) i", "10"),
    ]);
}

#[test]
fn strings_survive_collection_before_every_allocation() {
    let (mut agent, realm) = agent_with(Options {
        stress: true,
        verify: true,
        ..Options::default()
    });
    let expected: String = (0..40).map(|index| index.to_string()).collect();
    assert_eq!(
        run(
            &mut agent,
            realm,
            "var s = ''; for (let i = 0; i < 40; i++) { let t = `${i}`; s = s + t; } s"
        ),
        format!("{expected:?}")
    );
    assert_eq!(run(&mut agent, realm, "s.length"), "Unsupported(0)");
    assert_eq!(run(&mut agent, realm, "typeof s + 1"), "\"string1\"");
}

#[test]
fn long_chains_run_on_a_small_stack() {
    let thread = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let (mut agent, realm) = checked();
            let sum = format!("{}1", "1 + ".repeat(100_000));
            assert_eq!(run(&mut agent, realm, &sum), "100001");
            let and = format!("{}'ende'", "true && ".repeat(100_000));
            assert_eq!(run(&mut agent, realm, &and), "\"ende\"");
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}
