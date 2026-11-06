use super::*;
use crate::agent::Exception;
use crate::heap::Options;
use crate::number;
use crate::value::{Kind, Value};

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
        ("globalThis.x", "undefined"),
        ("var g = 1; globalThis.g", "1"),
        ("globalThis.h = 2; h", "2"),
        ("this.t = 3; t", "3"),
        ("let l = 1; globalThis.l", "undefined"),
        ("delete globalThis.NaN", "false"),
        ("'use strict'; delete globalThis.NaN", "TypeError"),
        ("globalThis.NaN = 1", "1"),
        ("'use strict'; globalThis.NaN = 1", "TypeError"),
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
    assert_eq!(eval("class A {}"), "Unsupported(0)");
}

#[test]
fn try_catches_what_its_block_throws() {
    check(&[
        ("try { throw 1; } catch (e) { e + 1; }", "2"),
        (
            "try { throw 1; } catch { 'ohne Bindung'; }",
            "\"ohne Bindung\"",
        ),
        ("try { 1; } catch (e) { 2; }", "1"),
        (
            "var log = ''; try { log += 'a'; throw 'b'; log += 'x'; } catch (e) { log += e; } log",
            "\"ab\"",
        ),
        (
            "try { try { throw 1; } catch (e) { throw e + 1; } } catch (e) { e * 10; }",
            "20",
        ),
        ("try { throw 1; } catch (e) { e = 2; e; }", "2"),
        (
            "var e = 'außen'; try { throw 1; } catch (e) {} e",
            "\"außen\"",
        ),
        ("let e = 'außen'; try { throw 1; } catch (e) { e; }", "1"),
        // Aus aufgerufenen Funktionen und aus Läufen der Engine.
        (
            "function f() { throw 'f'; } try { f(); } catch (e) { e; }",
            "\"f\"",
        ),
        (
            "function g() { return h(); } function h() { throw 'h'; } try { g(); } catch (e) { e; }",
            "\"h\"",
        ),
        (
            "var o = { get x() { throw 'Getter'; } }; try { o.x; } catch (e) { e; }",
            "\"Getter\"",
        ),
        (
            "function f() { try { throw 1; } catch (e) { return e + 1; } } f()",
            "2",
        ),
        (
            "function f() { try { return g(); } catch (e) { return 'gefangen ' + e; } } \
             function g() { throw 'g'; } f()",
            "\"gefangen g\"",
        ),
        // Was kein catch fängt, beendet das Script.
        ("try { throw 1; } finally {}", "throw 1"),
        ("try { null.x; } finally {}", "TypeError"),
        ("try { throw 1; } catch (e) { throw 2; }", "throw 2"),
        ("try { throw 1; } catch (e) { null.x; }", "TypeError"),
    ]);
}

/// Abschnitt 14.15.3: `try` ergibt den Wert von Block oder `catch`, ohne
/// eigenen *undefined*. Endet `finally` normal, zählt sein Wert nicht.
#[test]
fn try_has_a_completion_value() {
    check(&[
        ("1; try {} catch (e) {}", "undefined"),
        ("1; try { 2; } catch (e) {}", "2"),
        ("1; try { 2; throw 3; } catch (e) {}", "undefined"),
        ("try { 2; throw 3; } catch (e) { 4; }", "4"),
        ("1; try {} finally {}", "undefined"),
        ("try { 1; } finally { 2; }", "1"),
        ("try { throw 1; } catch (e) { 2; } finally { 3; }", "2"),
        ("do { try { 1; break; } finally { 2; } } while (false)", "1"),
        ("do { try { 1; } finally { 2; break; } } while (false)", "2"),
        (
            "do { try { 1; } finally { break; } } while (false)",
            "undefined",
        ),
        (
            "do { try { throw 1; } catch (e) { 2; break; } finally { 3; } } while (false)",
            "2",
        ),
    ]);
}

#[test]
fn finally_runs_on_every_way_out() {
    check(&[
        (
            "var log = ''; try { log += 'a'; } finally { log += 'f'; } log",
            "\"af\"",
        ),
        (
            "var log = ''; try { try { throw 1; } finally { log += 'f'; } } catch (e) { log += e; } log",
            "\"f1\"",
        ),
        (
            "var log = ''; function f() { try { return 'r'; } finally { log += 'f'; } } f() + log",
            "\"rf\"",
        ),
        (
            "function f() { try { return 1; } finally { return 2; } } f()",
            "2",
        ),
        (
            "function f() { try { throw 1; } finally { return 2; } } f()",
            "2",
        ),
        (
            "function f() { try { return 1; } finally { throw 2; } } f()",
            "throw 2",
        ),
        (
            "function f() { var x = 1; try { return x; } finally { x = 2; } } f()",
            "1",
        ),
        (
            "var log = ''; function f() { try { try { return 'r'; } finally { log += 1; } } \
             finally { log += 2; } } f() + log",
            "\"r12\"",
        ),
        (
            "var n = 0; for (var i = 0; i < 3; i++) { try { if (i == 1) continue; if (i == 2) break; } \
             finally { n++; } } n",
            "3",
        ),
        (
            "var log = ''; a: { try { try { break a; } finally { log += 1; } } finally { log += 2; } \
             log += 'x'; } log",
            "\"12\"",
        ),
        (
            "var log = ''; for (var i = 0; i < 2; i++) { try { try { continue; } finally { log += i; } } \
             finally { log += '.'; } } log",
            "\"0.1.\"",
        ),
        (
            "do { try { throw 1; } finally { break; } } while (false); 'weiter'",
            "\"weiter\"",
        ),
        (
            "var log = ''; while (true) { try { break; } finally { log += 'f'; } } log",
            "\"f\"",
        ),
        (
            "var log = ''; switch (1) { case 1: try { break; } finally { log += 'f'; } log += 'x'; } log",
            "\"f\"",
        ),
        // Mehrere Wege durch dasselbe finally.
        (
            "var log = ''; for (var i = 0; i < 4; i++) { try { if (i == 0) continue; \
             if (i == 1) { log += 'b'; continue; } if (i == 3) break; } finally { log += i; } } log",
            "\"0b123\"",
        ),
        (
            "function f(x) { for (;;) { try { if (x) return 'r'; break; } finally { x = 0; } } \
             return 'b'; } f(1) + f(0)",
            "\"rb\"",
        ),
    ]);
}

/// Ein Handler beginnt im Kontext von `try`, auch wenn der Block beim
/// Werfen einen eigenen hatte; ebenso der Weg durch `finally`.
#[test]
fn handlers_restore_the_context() {
    check(&[
        (
            "function f() { let outer = 'o'; (() => outer); try { let inner = 'i'; (() => inner); \
             throw 1; } catch (e) { return outer; } } f()",
            "\"o\"",
        ),
        (
            "function f() { let outer = 'o'; (() => outer); try { let inner = 'i'; (() => inner); \
             null.x; } finally { return outer; } } f()",
            "\"o\"",
        ),
        (
            "function f() { let outer = 'o'; (() => outer); for (;;) { try { let inner = 'i'; \
             (() => inner); break; } finally { return outer; } } } f()",
            "\"o\"",
        ),
        (
            "function f() { let outer = 'o'; (() => outer); try { g(); } catch (e) { return outer + e; } } \
             function g() { let deep = 'd'; (() => deep); throw deep; } f()",
            "\"od\"",
        ),
        (
            "var g; try { let x = 1; g = () => x; throw 2; } catch (e) { let y = e; (() => y + g())(); }",
            "3",
        ),
        (
            "var g; try { throw 5; } catch (e) { g = () => e; } g()",
            "5",
        ),
        (
            "var g; try { throw 5; } catch (e) { { let f = e + 1; g = () => e + f; } } g()",
            "11",
        ),
        (
            "var log = ''; for (let i = 0; i < 2; i++) { try { let k = i; (() => k); \
             if (i == 0) continue; } finally { log += (() => i)(); } } log",
            "\"01\"",
        ),
    ]);
}

#[test]
fn caught_engine_errors_are_error_objects() {
    check(&[
        ("try { null.x; } catch (e) { typeof e; }", "\"object\""),
        ("try { null.x; } catch (e) { e.name; }", "\"TypeError\""),
        (
            "try { missing; } catch (e) { e.name; }",
            "\"ReferenceError\"",
        ),
        (
            "try { let a = a; } catch (e) { e.name; }",
            "\"ReferenceError\"",
        ),
        (
            "try { const c = 1; c = 2; } catch (e) { e.name; }",
            "\"TypeError\"",
        ),
        ("try { null.x; } catch (e) { e.message; }", "\"\""),
        (
            "try { null.x; } catch (e) { e.name = 'anders'; e.name; }",
            "\"anders\"",
        ),
        (
            "try { try { null.x; } finally {} } catch (e) { e.name; }",
            "\"TypeError\"",
        ),
        (
            "function f() { return null.x; } try { f(); } catch (e) { e.name; }",
            "\"TypeError\"",
        ),
        (
            "var o = { get x() { return null.y; } }; try { o.x; } catch (e) { e.name; }",
            "\"TypeError\"",
        ),
        ("try { null.x; } catch (e) { throw e; }", "throw Objekt"),
        (
            "var a, b; try { null.x; } catch (e) { a = e; } try { null.x; } catch (e) { b = e; } a === b",
            "false",
        ),
    ]);
}

/// Das Fehlerobjekt entsteht im Realm des Rahmens, dessen Befehl warf.
#[test]
fn error_objects_belong_to_the_realm_that_threw() {
    let (mut agent, first) = checked();
    let second = agent.create_realm().expect("Speicher");
    agent.scope(|agent, scope| {
        let thrower = agent
            .evaluate_script(scope, second, &units("(function () { null.x; })"))
            .expect("Funktion");
        let global = agent.global_object(scope, first);
        let key = agent
            .key_from_units(scope, &units("thrower"))
            .expect("Speicher");
        crate::operations::set(agent, scope, global, key, thrower, true).expect("Zuweisung");
        for (source, realm) in [
            ("try { null.x; } catch (e) { e }", first),
            ("try { thrower(); } catch (e) { e }", second),
        ] {
            let error = agent
                .evaluate_script(scope, first, &units(source))
                .expect("Fehlerobjekt");
            let heap = agent.heap();
            let Kind::Object(object) = heap.get(error).kind() else {
                panic!("{source}: kein Objekt");
            };
            assert_eq!(
                heap.object_class(object),
                Some(crate::heap::ObjectClass::Error),
                "{source}"
            );
            let prototype = crate::object::get_prototype_of(agent, scope, error)
                .expect("kein Fehler")
                .expect("Prototyp");
            let expected =
                agent.intrinsic(scope, realm, crate::realm::Intrinsic::TypeErrorPrototype);
            let heap = agent.heap();
            assert_eq!(
                heap.get(prototype).bits(),
                heap.get(expected).bits(),
                "{source}"
            );
        }
    });
}

/// Anhang B.3.4: Ein `var` im Block von `catch` mit dem Namen des
/// Parameters schreibt mit seinem Initialisierer in den Parameter.
#[test]
fn var_in_catch_assigns_the_parameter() {
    check(&[
        (
            "function f() { try { throw 1; } catch (e) { var e = 2; } return e; } f()",
            "undefined",
        ),
        ("try { throw 1; } catch (e) { var e = 2; } e", "undefined"),
        (
            "function f() { try { throw 1; } catch (e) { var e = 2; return e; } } f()",
            "2",
        ),
        (
            "function capturedFoo() { return foo; } foo = 'davor'; \
             try { throw 1; } catch (foo) { var foo = 'im catch'; } capturedFoo()",
            "\"davor\"",
        ),
        // var-Namen aus allen drei Teilen von try.
        (
            "'use strict'; function f() { a = 1; b = 2; c = 3; try { var a; } catch (e) { var b; } \
             finally { var c; } return a + b + c; } f()",
            "6",
        ),
        ("'use strict'; x = 1; try { var x; } finally {} x", "1"),
        // Anhang B.3.2 mit einem Bezeichner als Parameter von catch.
        (
            "(function () { var before = f; try { throw null; } catch (f) { { function f() { return 123; } } } \
             return (before === undefined) + typeof f + f(); })()",
            "\"truefunction123\"",
        ),
        (
            "var before = typeof g; try { throw null; } catch (g) { { function g() { return 1; } } } \
             before + typeof g",
            "\"undefinedfunction\"",
        ),
    ]);
}

/// Ausnahmen, ihre Fehlerobjekte und die Zellen für finally überstehen
/// jede Sammlung.
#[test]
fn exceptions_survive_collections() {
    let (mut agent, realm) = agent_with(Options {
        stress: true,
        verify: true,
        ..Options::default()
    });
    let source = "var log = ''; function f(n) { try { if (n) f(n - 1); else null.x; } finally { log += n; } } \
                  try { f(3); } catch (e) { log += e.name; } \
                  try { try { throw 'w' + 1; } finally { log += 'f'; } } catch (e) { log += e; } log";
    assert_eq!(run(&mut agent, realm, source), "\"0123TypeErrorfw1\"");
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
    assert_eq!(run(&mut agent, realm, "s.length"), "70");
    assert_eq!(run(&mut agent, realm, "typeof s + 1"), "\"string1\"");
    assert_eq!(
        run(
            &mut agent,
            realm,
            "var o = {s, t: [s, s[69], {...'ab'}]}; o.t[0] === o.s && o.t[1] + o.t[2][1]"
        ),
        "\"9b\""
    );
}

#[test]
fn objects_and_arrays_from_literals() {
    check(&[
        ("var o = {a: 1, b: 2}; o.a + o.b", "3"),
        ("({a: 1}).a", "1"),
        ("var o = {a: 1, a: 2}; o.a", "2"),
        ("var b = 3; var o = {b}; o.b", "3"),
        (
            "var o = {1.5: 'x', 0: 'y', 1e21: 'z'}; o['1.5'] + o[0] + o['1e+21']",
            "\"xyz\"",
        ),
        (
            "var k = 'a'; var o = {[k]: 1, [k + 'b']: 2}; o.a + o.ab",
            "3",
        ),
        ("var o = {a: {b: {c: 'tief'}}}; o.a.b.c", "\"tief\""),
        ("var p = {}; var o = {__proto__: p}; p.y = 2; o.y", "2"),
        ("var o = {'__proto__': {y: 1}}; o.y", "1"),
        ("var o = {__proto__: 5}; o.y", "undefined"),
        ("var o = {__proto__: null}; 'y' in o", "false"),
        ("var o = {['__proto__']: {y: 1}}; o.y", "undefined"),
        ("var o = {['__proto__']: {y: 1}}; o.__proto__.y", "1"),
        (
            "var __proto__ = {y: 1}; var o = {__proto__}; o.y",
            "undefined",
        ),
        ("var s = {a: 1, b: 2}; var o = {...s, b: 3}; o.a + o.b", "4"),
        ("var o = {...'ab'}; o[0] + o[1]", "\"ab\""),
        ("var o = {...'ab'}; o.length", "undefined"),
        (
            "var o = {...null, ...undefined, ...1, ...true}; o",
            "Objekt",
        ),
        ("var o = {...{__proto__: {y: 1}}}; o.y", "undefined"),
        ("[1, 2, 3].length", "3"),
        ("[].length", "0"),
        ("[,].length", "1"),
        ("[1, , ].length", "2"),
        ("[1, , 3].length", "3"),
        ("1 in [1, , 3]", "false"),
        ("2 in [1, , 3]", "true"),
        ("var a = [10, 20]; a[1] + a['0']", "30"),
        ("var a = []; a[3] = 1; a.length", "4"),
        ("var a = [1, 2, 3]; a.length = 1; a[1]", "undefined"),
        ("var a = [1]; a[0] += 1; a[0]", "2"),
        ("var a = [[1, [2]]]; a[0][1][0]", "2"),
    ]);
}

#[test]
fn properties_of_primitives() {
    check(&[
        ("'abc'.length", "3"),
        ("''.length", "0"),
        ("'abc'[1]", "\"b\""),
        ("'abc'[3]", "undefined"),
        ("'abc'[1.5]", "undefined"),
        ("'abc'[-0]", "\"a\""),
        ("'abc'['length']", "3"),
        ("(1).x", "undefined"),
        ("true.x", "undefined"),
        ("var s = 'abc'; s.x = 1; s.x", "undefined"),
        ("'abc'.length = 1", "1"),
        ("'use strict'; var s = 'abc'; s.x = 1", "TypeError"),
        ("'use strict'; 'abc'.length = 1", "TypeError"),
        ("'use strict'; 'abc'[0] = 'x'", "TypeError"),
        ("delete 'abc'.length", "false"),
        ("delete 'abc'[0]", "false"),
        ("delete 'abc'[5]", "true"),
        ("delete 'abc'.x", "true"),
        ("'use strict'; delete 'abc'.length", "TypeError"),
        ("'use strict'; delete 'abc'.x", "true"),
        ("undefined.x", "TypeError"),
        ("null[0]", "TypeError"),
        ("var u; u.x = 1", "TypeError"),
        ("var u; u.x += 1", "TypeError"),
        ("var u; delete u.x", "TypeError"),
        ("var o = {}; o[-0] = 1; o['0']", "1"),
    ]);
}

#[test]
fn assignment_to_a_property_converts_the_base_last() {
    let (mut agent, realm) = checked();
    assert_eq!(
        run(&mut agent, realm, "var n = 0; var u; u[n++] = n++;"),
        "TypeError"
    );
    assert_eq!(run(&mut agent, realm, "n"), "2");
    assert_eq!(run(&mut agent, realm, "null.x = (n = 5);"), "TypeError");
    assert_eq!(run(&mut agent, realm, "n"), "5");
    // Beim Lesen dagegen vor dem nächsten Glied.
    assert_eq!(run(&mut agent, realm, "null[n = 6];"), "TypeError");
    assert_eq!(run(&mut agent, realm, "n"), "6");
}

#[test]
fn delete_in_and_instanceof() {
    check(&[
        ("var o = {a: 1}; delete o.a", "true"),
        ("var o = {a: 1}; delete o.a; o.a", "undefined"),
        ("var o = {a: 1}; delete o['a']; 'a' in o", "false"),
        ("var o = {}; delete (o.a)", "true"),
        ("delete [1].length", "false"),
        ("'use strict'; delete [1].length", "TypeError"),
        ("var a = [1, 2]; delete a[1]; a.length", "2"),
        ("'a' in {a: 1}", "true"),
        ("'b' in {a: 1}", "false"),
        ("'x' in {__proto__: {x: 1}}", "true"),
        ("0 in [5]", "true"),
        ("'length' in []", "true"),
        ("'a' in 'abc'", "TypeError"),
        ("1 in null", "TypeError"),
        ("1 instanceof 1", "TypeError"),
        ("var o = {}; o instanceof o", "TypeError"),
    ]);
}

#[test]
fn optional_chains_stop_at_undefined_and_null() {
    check(&[
        ("var u; u?.x", "undefined"),
        ("var u = null; u?.x.y.z", "undefined"),
        ("var u; u?.[nothing]", "undefined"),
        ("var o = {a: null}; o.a?.b", "undefined"),
        ("var o = {a: {b: 1}}; o?.a?.b", "1"),
        ("var o = {a: {}}; o.a.b?.c", "undefined"),
        ("var o = {}; o.a.b?.c", "TypeError"),
        ("var u; (u?.a).b", "TypeError"),
        ("'abc'?.length", "3"),
        ("var o = {a: 1}; delete o?.a", "true"),
        ("var o = {a: 1}; delete o?.a; 'a' in o", "false"),
        ("var u; delete u?.a", "true"),
        ("var u; delete u?.a.b", "true"),
        ("var u; delete u?.[nothing]", "true"),
        ("'use strict'; delete 'abc'?.length", "TypeError"),
    ]);
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
            assert_eq!(run(&mut agent, realm, "var o = {}; o.o = o;"), "Objekt");
            for link in [".o", "?.o", "['o']"] {
                let chain = format!("o{} === o", link.repeat(100_000));
                assert_eq!(run(&mut agent, realm, &chain), "true", "{link}");
            }
            let delete = format!("delete o{}.x", "?.o".repeat(100_000));
            assert_eq!(run(&mut agent, realm, &delete), "true");
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}

#[test]
fn functions_are_called_with_their_arguments() {
    check(&[
        ("function f(a, b) { return a + b; } f(1, 2)", "3"),
        ("function f(a, b) { return b; } f(1)", "undefined"),
        ("function f(a) { return a; } f(1, 2, 3)", "1"),
        ("function f(a, a) { return a; } f(1, 2)", "2"),
        ("function f() {} f()", "undefined"),
        ("function f() { 1; } f()", "undefined"),
        ("function f() { return; } f()", "undefined"),
        ("function f(x) { x = 2; return x; } f(1)", "2"),
        (
            "function f() { var x = 1; { let y = 2; x += y; } return x; } f()",
            "3",
        ),
        ("function f() { return x; var x = 1; } f()", "undefined"),
        (
            "function f(n) { return n === 0 ? 0 : n + f(n - 1); } f(10)",
            "55",
        ),
        ("var f = function (x) { return x * 2; }; f(21)", "42"),
        ("(function (x) { return x; })(5)", "5"),
        ("var o = {m(x) { return x + 1; }}; o.m(1)", "2"),
        ("var o = {m(x) { return x + 1; }}; o['m'](1)", "2"),
        (
            "function f() { return function () { return 4; }; } f()()",
            "4",
        ),
        ("1()", "TypeError"),
        ("var o = {}; o.m()", "TypeError"),
        ("var u; u()", "TypeError"),
        ("({})()", "TypeError"),
        ("nirgends()", "ReferenceError"),
        // Die Argumente kommen von links nach rechts, erst danach IsCallable.
        (
            "var log = ''; function f() { return log; } f(log += 'a', log += 'b')",
            "\"ab\"",
        ),
        ("var n = 0; var o = {}; o.m(n = 1)", "TypeError"),
    ]);
    let (mut agent, realm) = checked();
    assert_eq!(run(&mut agent, realm, "var n = 0; n(n = 1)"), "TypeError");
    assert_eq!(run(&mut agent, realm, "n"), "1");
}

#[test]
fn functions_see_their_own_bindings_and_the_global_ones() {
    check(&[
        (
            "var x = 1; function f() { var x = 2; return x; } f() + x",
            "3",
        ),
        ("function f() { return x; } var x = 5; f()", "5"),
        (
            "function f() { return g(); } function g() { return 7; } f()",
            "7",
        ),
        ("function f() { y = 3; } f(); y", "3"),
        (
            "function f() { 'use strict'; y = 3; } f()",
            "ReferenceError",
        ),
        (
            "'use strict'; function f() { y = 3; } f()",
            "ReferenceError",
        ),
        ("function f() { return h; } f()", "ReferenceError"),
        ("function f() { let a = a; } f()", "ReferenceError"),
        ("function f() { const c = 1; c = 2; } f()", "TypeError"),
        (
            "function f() { return typeof nirgends; } f()",
            "\"undefined\"",
        ),
        ("function f(a) { return delete a; } f(1)", "false"),
        ("var o = {}; function f() { o.x = 1; } f(); o.x", "1"),
        // Funktionen stehen schon vor dem ersten Befehl.
        ("f(); function f() { return 1; }", "1"),
        (
            "function f() { return g(); function g() { return 2; } } f()",
            "2",
        ),
        (
            "function f() { return 1; } function f() { return 2; } f()",
            "2",
        ),
        ("typeof f; function f() {}", "\"function\""),
        ("var f = 1; function f() {} typeof f", "\"number\""),
        ("var f; function f() {} typeof f", "\"function\""),
        (
            "function f() { var g = 1; function g() {} return g; } f()",
            "1",
        ),
        ("1; function f() {}", "1"),
        ("function f() {} delete f", "false"),
        ("function NaN() {}", "TypeError"),
    ]);
    let (mut agent, realm) = checked();
    assert_eq!(run(&mut agent, realm, "let h = 1;"), "undefined");
    assert_eq!(run(&mut agent, realm, "function h() {}"), "SyntaxError");
    assert_eq!(
        run(&mut agent, realm, "function k() { return 1; }"),
        "undefined"
    );
    assert_eq!(
        run(&mut agent, realm, "function k() { return 2; } k()"),
        "2"
    );
}

#[test]
fn this_comes_from_the_reference() {
    check(&[
        ("var o = {m() { return this; }}; o.m() === o", "true"),
        ("var o = {m() { return this; }}; o['m']() === o", "true"),
        ("var o = {m() { return this; }}; (o.m)() === o", "true"),
        (
            "var o = {a: {m() { return this; }}}; o.a.m() === o.a",
            "true",
        ),
        (
            "var o = {m() { return this; }}; (0, o.m)() === this",
            "true",
        ),
        (
            "var o = {m() { return this; }}; var m = o.m; m() === this",
            "true",
        ),
        (
            "var o = {m() { 'use strict'; return this; }}; var m = o.m; m()",
            "undefined",
        ),
        ("function f() { return this; } f() === this", "true"),
        (
            "'use strict'; function f() { return this; } f()",
            "undefined",
        ),
        (
            "var o = {f: function () { return this; }}; o.f() === o",
            "true",
        ),
        ("var o = {get g() { return this; }}; o.g === o", "true"),
        ("var o = {m() { return this.v; }, v: 3}; o.m()", "3"),
        ("var o = {m() { return this; }}; o?.m() === o", "true"),
        ("var o = {m() { return this; }}; (o?.m)() === o", "true"),
        ("var o = {m() { return this; }}; o.m?.() === o", "true"),
        ("var o = {m() { return this; }}; o?.['m']() === o", "true"),
    ]);
}

#[test]
fn optional_calls_stop_at_undefined_and_null() {
    check(&[
        ("var o = {}; o.m?.()", "undefined"),
        ("var u; u?.m()", "undefined"),
        ("var u = null; u?.m().x.y", "undefined"),
        ("var f; f?.()", "undefined"),
        ("var n = 0; var f; f?.(n++); n", "0"),
        ("var o = {}; (o?.m)()", "TypeError"),
        ("var o = {m() { return null; }}; o.m()?.x", "undefined"),
        ("var o = {m() { return o; }, v: 1}; o.m().m()?.m().v", "1"),
    ]);
}

#[test]
fn accessors_and_methods_run_from_the_engine() {
    check(&[
        ("var o = {get a() { return 1; }}; o.a", "1"),
        ("var o = {set a(v) { this.b = v * 2; }}; o.a = 2; o.b", "4"),
        ("var o = {set a(v) {}}; o.a = 2", "2"),
        (
            "var o = {get a() { return 1; }, set a(v) {}}; o.a = 5; o.a",
            "1",
        ),
        ("var o = {get a() { return 1; }}; o.a = 5; o.a", "1"),
        (
            "'use strict'; var o = {get a() { return 1; }}; o.a = 5",
            "TypeError",
        ),
        ("var o = {get a() { return 1; }, a: 2}; o.a", "2"),
        ("var o = {a: 2, get a() { return 1; }}; o.a", "1"),
        ("var k = 'x'; var o = {get [k]() { return 3; }}; o.x", "3"),
        ("var o = {get 1() { return 'eins'; }}; o[1]", "\"eins\""),
        (
            "var p = {get a() { return this.b; }}; var o = {__proto__: p, b: 5}; o.a",
            "5",
        ),
        (
            "var o = {get a() { throw 'innen'; }}; o.a",
            "throw \"innen\"",
        ),
        ("var o = {valueOf() { return 4; }}; o + 1", "5"),
        ("var o = {toString() { return 'ich'; }}; '' + o", "\"ich\""),
        ("var o = {toString() { return {}; }}; '' + o", "TypeError"),
        ("var o = {valueOf() { return 2; }}; o * o", "4"),
        (
            "var o = {toString() { return 'k'; }}; var p = {k: 1}; p[o]",
            "1",
        ),
        ("'x' in {get x() {}}", "true"),
    ]);
}

#[test]
fn functions_are_objects_with_a_prototype() {
    check(&[
        ("function f() {} typeof f", "\"function\""),
        ("typeof function () {}", "\"function\""),
        ("var o = {m() {}}; typeof o.m", "\"function\""),
        ("var o = {m() {}}; typeof o.m.prototype", "\"undefined\""),
        ("function f(a, b) {} f.length", "2"),
        ("function f() {} typeof f.prototype", "\"object\""),
        ("function C() {} C.prototype.constructor === C", "true"),
        ("function C() {} ({}) instanceof C", "false"),
        (
            "function C() {} var o = {__proto__: C.prototype}; o instanceof C",
            "true",
        ),
        (
            "function C() {} C.prototype = 1; ({}) instanceof C",
            "TypeError",
        ),
        ("function f() {} f.x = 1; f.x", "1"),
        ("function f() {} f.length = 5; f.length", "0"),
        // Danach erbt sie die Länge von %Function.prototype%.
        ("function f() {} delete f.length; f.length", "0"),
        ("function f() {} f === f", "true"),
        (
            "function f() { return function () {}; } f() === f()",
            "false",
        ),
    ]);
}

#[test]
fn anonymous_functions_get_the_name_of_their_place() {
    check(&[
        ("function f() {} f.name", "\"f\""),
        ("var f = function () {}; f.name", "\"f\""),
        ("var f = (function () {}); f.name", "\"f\""),
        ("let f = function () {}; f.name", "\"f\""),
        ("const f = function () {}; f.name", "\"f\""),
        ("var g; g = function () {}; g.name", "\"g\""),
        ("var g; (g) = function () {}; g.name", "\"\""),
        ("var g; g ||= function () {}; g.name", "\"g\""),
        ("var g; g ??= function () {}; g.name", "\"g\""),
        ("var g = 1; g &&= function () {}; g.name", "\"g\""),
        ("var o = {}; o.p = function () {}; o.p.name", "\"\""),
        ("var o = {p: function () {}}; o.p.name", "\"p\""),
        ("var o = {1: function () {}}; o[1].name", "\"1\""),
        (
            "var k = 'q'; var o = {[k]: function () {}}; o.q.name",
            "\"q\"",
        ),
        ("var o = {m() {}}; o.m.name", "\"m\""),
        ("var o = {__proto__: function () {}}; o.name", "\"\""),
        ("var f = function g() {}; f.name", "\"g\""),
        ("(function () {}).name", "\"\""),
        (
            "function f() { var h = function () {}; return h.name; } f()",
            "\"h\"",
        ),
    ]);
}

#[test]
fn named_function_expressions_bind_their_name() {
    check(&[
        (
            "var g = function f(n) { return n ? f(n - 1) + 1 : 0; }; g(3)",
            "3",
        ),
        (
            "var f = function g() { return typeof g; }; f()",
            "\"function\"",
        ),
        (
            "var f = function g() { g = 1; return g; }; f() === f",
            "true",
        ),
        (
            "var f = function g() { 'use strict'; g = 1; }; f()",
            "TypeError",
        ),
        ("var f = function g(g) { return g; }; f(1)", "1"),
        ("var f = function g() { var g = 2; return g; }; f()", "2"),
        ("var g = 1; var f = function g() {}; g", "1"),
        ("var f = function g() {}; typeof g", "\"undefined\""),
    ]);
}

#[test]
fn arguments_and_its_name() {
    check(&[
        (
            "function f() { return typeof arguments; } f()",
            "\"object\"",
        ),
        ("function f() { return arguments.length; } f(1, 2, 3)", "3"),
        ("function f(a) { return arguments[2]; } f(1, 2, 3)", "3"),
        ("function f(a) { return 0 in arguments; } f()", "false"),
        (
            "function f() { return arguments.callee === f; } f()",
            "true",
        ),
        (
            "function f() { arguments.length = 7; return arguments.length; } f()",
            "7",
        ),
        (
            "function f() { return arguments === arguments; } f()",
            "true",
        ),
        ("function f() { return arguments; } f() === f()", "false"),
        // `var` deklariert nichts Neues, eine Funktion ersetzt das Objekt.
        (
            "function f() { var arguments; return typeof arguments; } f()",
            "\"object\"",
        ),
        (
            "function f() { var arguments = 1; return arguments; } f()",
            "1",
        ),
        (
            "function f() { function arguments() {} return typeof arguments; } f()",
            "\"function\"",
        ),
        ("function f(arguments) { return arguments; } f(3)", "3"),
        // `arguments` verdeckt den Namen eines Funktionsausdrucks.
        (
            "var f = function arguments() { return arguments === f; }; f()",
            "false",
        ),
        (
            "var f = function arguments(arguments) { return arguments; }; f(4)",
            "4",
        ),
        (
            "var f = function arguments() { let arguments = 5; return arguments; }; f()",
            "5",
        ),
        // Eine Pfeilfunktion meint das Objekt der Funktion um sie herum.
        ("function f() { return () => arguments[0]; } f(6)()", "6"),
        (
            "function f() { return () => () => arguments.length; } f(1, 2)()()",
            "2",
        ),
        (
            "function f() { let arguments = 6; return () => arguments; } f()()",
            "6",
        ),
        (
            "function f() { return function () { return arguments.length; }(1, 2); } f(1)",
            "2",
        ),
        ("var f = () => arguments; f()", "ReferenceError"),
        (
            "var f = () => { var arguments = 1; return arguments; }; f()",
            "1",
        ),
        // Aufrufe aus der Engine.
        ("var o = { get x() { return arguments.length; } }; o.x", "0"),
        (
            "var o = { set x(v) { this.n = arguments[0]; } }; o.x = 5; o.n",
            "5",
        ),
        // Anhang B.3.2.1: Die Funktion im Block setzt das Objekt nicht.
        (
            "function f() { var r = typeof arguments; { function arguments() {} r += typeof arguments; } return r + typeof arguments; } f()",
            "\"objectfunctionobject\"",
        ),
        (
            "var f = () => { var arguments; { function arguments() { return 7; } } return arguments(); }; f()",
            "7",
        ),
        ("var f = async () => 1; f", "Unsupported(8)"),
    ]);
}

#[test]
fn mapped_arguments_share_the_parameter_bindings() {
    check(&[
        ("function f(a) { a = 2; return arguments[0]; } f(1)", "2"),
        ("function f(a) { arguments[0] = 2; return a; } f(1)", "2"),
        (
            "function f(a, b) { b = 3; return arguments[1]; } f(1, 2)",
            "3",
        ),
        // Nur Argumente, die es gibt, sind verknüpft.
        (
            "function f(a) { a = 2; return arguments[0]; } f()",
            "undefined",
        ),
        (
            "function f(a) { arguments[0] = 2; return a; } f()",
            "undefined",
        ),
        (
            "function f(a) { arguments[0] = 2; return arguments.length; } f()",
            "0",
        ),
        // Beim selben Namen ist der letzte Parameter verknüpft.
        (
            "function f(a, a) { return arguments[0] + ',' + arguments[1] + ',' + a; } f(1, 2)",
            "\"1,2,2\"",
        ),
        (
            "function f(a, a) { a = 9; return arguments[0] + ',' + arguments[1]; } f(1, 2)",
            "\"1,9\"",
        ),
        (
            "function f(a, a) { arguments[0] = 9; return a; } f(1, 2)",
            "2",
        ),
        // Löschen beendet die Verknüpfung.
        (
            "function f(a) { delete arguments[0]; arguments[0] = 3; return a; } f(1)",
            "1",
        ),
        (
            "function f(a) { delete arguments[0]; a = 4; return arguments[0]; } f(1)",
            "undefined",
        ),
        // Auch Closures und Pfeilfunktionen sehen dieselbe Bindung.
        (
            "function f(a) { var g = () => arguments[0]; a = 3; return g(); } f(1)",
            "3",
        ),
        (
            "function f(a, b) { arguments[1] = 'x'; return (function () { return b; })(); } f(1, 2)",
            "\"x\"",
        ),
        (
            "function f(a) { function a() {} return typeof arguments[0]; } f(1)",
            "\"function\"",
        ),
    ]);
}

#[test]
fn unmapped_arguments_keep_their_values() {
    check(&[
        (
            "function f(a) { 'use strict'; a = 2; return arguments[0]; } f(1)",
            "1",
        ),
        (
            "function f(a) { 'use strict'; arguments[0] = 2; return a; } f(1)",
            "1",
        ),
        (
            "function f(a = 0) { a = 2; return arguments[0]; } f(1)",
            "1",
        ),
        (
            "function f(a, ...r) { a = 2; return arguments[0]; } f(1)",
            "1",
        ),
        (
            "function f(a, ...r) { arguments[0] = 2; return a; } f(1)",
            "1",
        ),
        (
            "function f() { 'use strict'; return arguments.length; } f(1, 2)",
            "2",
        ),
        // Abweichung: Ohne %ThrowTypeError% ist `callee` *undefined*.
        (
            "function f() { 'use strict'; return arguments.callee; } f()",
            "undefined",
        ),
        (
            "function f() { 'use strict'; arguments.callee = 1; } f()",
            "TypeError",
        ),
        (
            "function f() { 'use strict'; return delete arguments.callee; } f()",
            "TypeError",
        ),
    ]);
}

#[test]
fn parameters_with_defaults() {
    check(&[
        ("function f(a = 1) { return a; } f()", "1"),
        ("function f(a = 1) { return a; } f(undefined)", "1"),
        ("function f(a = 1) { return a; } f(null)", "null"),
        ("function f(a = 1) { return a; } f(0)", "0"),
        ("function f(a, b = a + 1) { return b; } f(1)", "2"),
        (
            "function f(a = 1, b) { return a + b; } f(undefined, 2)",
            "3",
        ),
        ("function f(a = 1) { return arguments.length; } f()", "0"),
        (
            "function f(a = arguments.length) { return a; } f(undefined, 2)",
            "2",
        ),
        (
            "function f(a = () => arguments.length) { return a(); } f(undefined, 1, 2)",
            "3",
        ),
        // Bis zu seiner Initialisierung hält ein Parameter die Lücke.
        ("function f(a = b, b) { return a; } f()", "ReferenceError"),
        ("function f(a = b, b) { return a; } f(1)", "1"),
        ("function f(a = a) {} f()", "ReferenceError"),
        ("function f(g = () => b, b = g()) {} f()", "ReferenceError"),
        // Ausdrücke in Parametern sehen den Rumpf nicht.
        (
            "var x = 'außen'; function f(g = () => x) { var x = 'innen'; return g(); } f()",
            "\"außen\"",
        ),
        (
            "function f(a, g = () => a) { var a; return g(); } f(1)",
            "1",
        ),
        (
            "function f(a, g = () => a) { var a = 2; return g(); } f(1)",
            "1",
        ),
        (
            "function f(a, g = () => a) { a = 2; return g(); } f(1)",
            "2",
        ),
        (
            "function f(a, g = () => { a = 5; }) { var a; g(); return a; } f(1)",
            "1",
        ),
        ("function f(a = 1) { var a; return a; } f()", "1"),
        ("function f(a = 1) { var a = 2; return a; } f()", "2"),
        (
            "function f(a = 1) { function a() {} return typeof a; } f()",
            "\"function\"",
        ),
        (
            "function f(a = 1) { function arguments() {} return typeof arguments; } f()",
            "\"function\"",
        ),
        (
            "function f(a = 1) { let arguments = 2; return arguments; } f()",
            "2",
        ),
        (
            "function f(a = 1) { var arguments; return typeof arguments; } f()",
            "\"object\"",
        ),
        // NamedEvaluation und `this`.
        (
            "function f(g = function () {}) { return g.name; } f()",
            "\"g\"",
        ),
        ("function f(g = () => 1) { return g.name; } f()", "\"g\""),
        ("var o = { m(a = this) { return a === o; } }; o.m()", "true"),
        (
            "var o = { m(g = () => this) { return g() === o; } }; o.m()",
            "true",
        ),
        // Der Name eines Funktionsausdrucks gilt in den Parametern, auch
        // wenn ihn ein `var` im Rumpf verdeckt.
        (
            "var f = function g(a = g) { var g; return a === f && g === undefined; }; f()",
            "true",
        ),
        ("var f = (a, b = a * 2) => a + b; f(3)", "9"),
        ("function f(a, b = 1, c) {} f.length", "1"),
    ]);
}

#[test]
fn rest_parameters_collect_the_remaining_arguments() {
    check(&[
        (
            "function f(a, ...r) { return r.length + ',' + r[0] + ',' + r[1]; } f(1, 2, 3)",
            "\"2,2,3\"",
        ),
        ("function f(...r) { return r.length; } f()", "0"),
        ("function f(a, b, ...r) { return r.length; } f(1)", "0"),
        // Ein Array: `length` folgt den Indizes.
        ("function f(...r) { r[5] = 1; return r.length; } f(1)", "6"),
        ("function f(...r) { return r; } f() === f()", "false"),
        ("var f = (...r) => r.length; f(1, 2)", "2"),
        (
            "function f(a = 1, ...r) { return a + r[0]; } f(undefined, 2)",
            "3",
        ),
        ("function f(...r) { return () => r[0]; } f(4)()", "4"),
        ("function f(a, ...r) {} f.length", "1"),
    ]);
}

#[test]
fn closures_see_the_bindings_around_them() {
    check(&[
        (
            "function f(x) { return function () { return x; }; } f(1)()",
            "1",
        ),
        (
            "function f(x) { x = 5; return function () { return x; }; } f(1)()",
            "5",
        ),
        (
            "function f(x) { var g = function () { x = 7; }; g(); return x; } f(1)",
            "7",
        ),
        (
            "function counter() { var n = 0; return function () { return ++n; }; } \
             var c = counter(); c(); c(); c()",
            "3",
        ),
        // Jeder Aufruf hat seinen eigenen Kontext, Closures desselben Aufrufs
        // teilen ihn.
        (
            "function counter() { var n = 0; return function () { return ++n; }; } \
             var a = counter(), b = counter(); a(); a(); b()",
            "1",
        ),
        (
            "function f() { var n = 0; return {inc() { n++; }, get v() { return n; }}; } \
             var o = f(); o.inc(); o.inc(); o.v",
            "2",
        ),
        (
            "function f(a) { return function (b) { return function (c) { return a + b + c; }; }; } \
             f(1)(2)(3)",
            "6",
        ),
        (
            "function f() { let x = 1; { let y = 2; return function () { return x * 10 + y; }; } } f()()",
            "12",
        ),
        // Alle Bindungen mit dem Namen liegen in Kontexten, gemeint ist die
        // nächste.
        (
            "function f() { var x = 1; { let x = 2; } return function () { return x; }; } f()()",
            "1",
        ),
        (
            "function f() { let x = 1, g; { let x = 2; g = function () { return x; }; } return g() * 10 + x; } f()",
            "21",
        ),
        (
            "function f() { var g = function () { var x = 3; return x; }; var x = 4; return g() + x; } f()",
            "7",
        ),
        (
            "var f = function g(n) { return function () { return n ? g(n - 1)() + 1 : 0; }; }; f(3)()",
            "3",
        ),
        (
            "var f = function g() { return function () { g = 1; return typeof g; }; }; f()()",
            "\"function\"",
        ),
        (
            "var g; { let x = 3; g = function () { return x; }; } g()",
            "3",
        ),
        (
            "var g; { let x = 3; g = function () { return ++x; }; } g(); g()",
            "5",
        ),
        (
            "var o = {m(a) { return {get b() { return a; }}; }}; o.m(8).b",
            "8",
        ),
    ]);
}

#[test]
fn closures_check_the_temporal_dead_zone() {
    check(&[
        (
            "function f() { function g() { return x; } let x = 1; return g(); } f()",
            "1",
        ),
        (
            "function f() { function g() { return x; } g(); let x = 1; } f()",
            "ReferenceError",
        ),
        (
            "function f() { function g() { x = 2; } g(); let x = 1; } f()",
            "ReferenceError",
        ),
        (
            "function f() { function g() { return typeof x; } g(); let x; } f()",
            "ReferenceError",
        ),
        (
            "function f() { const c = 1; function g() { c = 2; } g(); } f()",
            "TypeError",
        ),
        (
            "function f() { function g() { c = 2; } g(); const c = 1; } f()",
            "ReferenceError",
        ),
        (
            "function f() { var g = function () { return x; }; x; let x = 1; } f()",
            "ReferenceError",
        ),
        (
            "function f() { var g = function () { return x; }; x = 1; let x; } f()",
            "ReferenceError",
        ),
        (
            "function f() { var g = function () { return x; }; let x = 1; x = 2; return g(); } f()",
            "2",
        ),
        (
            "var g; { g = function () { return y; }; let y = 1; } g()",
            "1",
        ),
        (
            "var g; switch (1) { case 0: let y = 1; case 1: g = function () { return y; }; } g()",
            "ReferenceError",
        ),
        // `var` und Parameter haben keine TDZ.
        (
            "function f() { function g() { return v; } var r = g(); var v = 1; return r; } f()",
            "undefined",
        ),
    ]);
}

#[test]
fn arrow_functions_take_this_from_their_surroundings() {
    check(&[
        ("var f = (a, b) => a + b; f(1, 2)", "3"),
        ("var f = x => x * 2; f(4)", "8"),
        ("var f = () => { return 5; }; f()", "5"),
        ("var f = () => {}; f()", "undefined"),
        ("var f = x => y => x - y; f(5)(3)", "2"),
        ("var f = () => 1; f.name", "\"f\""),
        ("var f = (a, b) => 1; f.length", "2"),
        ("var f = () => 1; typeof f.prototype", "\"undefined\""),
        ("var o = {f: () => 1}; o.f.name", "\"f\""),
        ("var o = {v: 1, m() { return () => this.v; }}; o.m()()", "1"),
        (
            "var o = {v: 2, m() { return () => () => this.v; }}; o.m()()()",
            "2",
        ),
        (
            "var o = {v: 3, get g() { return (() => this.v)(); }}; o.g",
            "3",
        ),
        // Das `this` des Aufrufs zählt nicht.
        (
            "function F() { return () => this; } var o = {F: F}; var a = o.F(); var p = {a: a}; p.a() === o",
            "true",
        ),
        (
            "'use strict'; function f() { return () => this; } f()()",
            "undefined",
        ),
        ("function f() { return () => this; } f()() === this", "true"),
        // Im Script ist `this` das globale Objekt.
        ("var v = 4; var f = () => this.v; f()", "4"),
        ("(() => this)() === this", "true"),
        ("var o = {f: () => this}; o.f() === this", "true"),
        (
            "var f = () => { 'use strict'; return this; }; f() === this",
            "true",
        ),
        (
            "var f = function () { return (() => typeof this)(); }; f()",
            "\"object\"",
        ),
        (
            "var f = function () { 'use strict'; return (() => typeof this)(); }; f()",
            "\"undefined\"",
        ),
    ]);
}

#[test]
fn computed_method_names_see_the_surrounding_this() {
    // Der berechnete Name läuft im umgebenden Code, nicht in der Methode.
    check(&[
        (
            "var o = {x: 'key', f() { return {[(() => this.x)()]() { return 7; }}; }}; o.f().key()",
            "7",
        ),
        (
            "var o = {x: 'g', f() { return {get [(() => this.x)()]() { return 8; }}; }}; o.f().g",
            "8",
        ),
        (
            "var x = 'k'; var o = {[(() => this.x)()]() { return 9; }}; o.k()",
            "9",
        ),
    ]);
}

#[test]
fn let_in_for_binds_each_iteration() {
    check(&[
        (
            "var a = {}; for (let i = 0; i < 3; i++) { a[i] = function () { return i; }; } \
             a[0]() + a[1]() * 10 + a[2]() * 100",
            "210",
        ),
        (
            "var a = {}; for (var i = 0; i < 3; i++) { a[i] = function () { return i; }; } \
             a[0]() + a[1]() * 10 + a[2]() * 100",
            "333",
        ),
        (
            "var a = {}; for (let i = 0; i < 3; i++) { let j = i * 2; a[i] = () => j; } \
             a[0]() + a[1]() + a[2]()",
            "6",
        ),
        // Die Kopie entsteht vor dem ersten Test und vor jedem Schritt; was
        // der Rumpf ändert, nimmt sie mit.
        (
            "var g; for (let i = 0; g = g || (() => i), i < 3; i++) {} g()",
            "0",
        ),
        (
            "var a = {}, n = 0; for (let i = 0; i < 6; i++) { a[n++] = () => i; i++; } \
             a[0]() * 100 + a[1]() * 10 + a[2]()",
            "135",
        ),
        (
            "var a = {}; for (let i = 0; i < 3; i++) { let j = i + 1; a[i] = () => i * j; if (i) continue; } \
             a[0]() + a[1]() + a[2]()",
            "8",
        ),
        (
            "var g; for (let i = 0; ; i++) { let j = i; g = () => i + j; if (i === 2) break; } g()",
            "4",
        ),
        (
            "var g; outer: for (let i = 0; i < 3; i++) { for (let j = 0; j < 3; j++) { \
             let k = i * 10 + j; g = () => k; if (j === 1) continue outer; if (i === 2) break outer; } } g()",
            "20",
        ),
        (
            "var a = {}; for (const i = 0; ; ) { a.f = () => i; break; } a.f()",
            "0",
        ),
    ]);
}

/// `break` und `continue` geben die Kontexte ab, die sie verlassen: Eine
/// Pfeilfunktion danach findet `this` des Scripts an der richtigen Stelle.
#[test]
fn jumps_leave_their_contexts() {
    check(&[
        (
            "var g; for (let i = 0; i < 3; i++) { let j = i; g = () => j; if (i === 1) break; } \
             var f = () => this; f() === this && g()",
            "1",
        ),
        (
            "var g; switch (1) { case 1: { let x = 5; g = () => x; break; } } \
             var f = () => this; f() === this && g()",
            "5",
        ),
        (
            "var g; l: { let x = 6; g = () => x; break l; } var f = () => this; f() === this && g()",
            "6",
        ),
        (
            "var g, n = 0; while (n < 2) { let x = n++; g = () => x; { let y = x; g = () => y; continue; } } \
             var f = () => this; f() === this && g()",
            "1",
        ),
    ]);
}

#[test]
fn functions_in_blocks_are_lexical() {
    check(&[
        (
            "'use strict'; { function f() { return 1; } } typeof f",
            "\"undefined\"",
        ),
        (
            "'use strict'; var r; { r = f(); function f() { return 2; } } r",
            "2",
        ),
        (
            "function g() { 'use strict'; { function f() {} } return typeof f; } g()",
            "\"undefined\"",
        ),
        // Funktionen in Blöcken sehen den Block.
        (
            "function g() { { let x = 3; function f() { return x; } return f; } } g()()",
            "3",
        ),
        (
            "var h; { let x = 4; function f() { return x; } h = f; } h()",
            "4",
        ),
        (
            "var r; { function f() { return 1; } function f() { return 2; } r = f(); } r",
            "2",
        ),
        (
            "var r; switch (1) { case 0: function f() { return 3; } case 1: r = f(); } r",
            "3",
        ),
    ]);
}

/// Anhang B.3.2 und B.3.3: Außerhalb des strikten Modus setzt eine Funktion
/// in einem Block auch die gleichnamige var-Bindung, wenn eine
/// `var`-Deklaration an ihrer Stelle keinen frühen Fehler ergäbe.
#[test]
fn functions_in_blocks_also_set_var_bindings() {
    check(&[
        ("{ function f() { return 3; } } f()", "3"),
        ("var t = typeof f; { function f() {} } t", "\"undefined\""),
        ("var r = f; { function f() {} } r", "undefined"),
        (
            "{ function f() { return 1; } } { function f() { return 2; } } f()",
            "2",
        ),
        ("var f = 1; { function f() { return 2; } } f()", "2"),
        ("let f = 1; { function f() {} } f", "1"),
        (
            "{ let f = 1; { function f() {} } } typeof f",
            "\"undefined\"",
        ),
        (
            "for (let f = 0; f < 1; f++) { function f() {} } typeof f",
            "\"undefined\"",
        ),
        (
            "switch (1) { default: let f; { function f() {} } } typeof f",
            "\"undefined\"",
        ),
        // Zwei im selben Block: Eine `var`-Deklaration an der Stelle der
        // einen ergäbe mit der anderen einen frühen Fehler.
        (
            "{ function f() { return 1; } function f() { return 2; } } typeof f",
            "\"undefined\"",
        ),
        ("if (true) function f() { return 4; } f()", "4"),
        ("if (false) function f() {} f", "undefined"),
        ("if (false) ; else function f() { return 5; } f()", "5"),
        ("switch (1) { case 1: function f() { return 6; } } f()", "6"),
        ("l: { function f() { return 7; } } f()", "7"),
        ("l: function f() {}", "Unsupported(3)"),
        // In Funktionen.
        (
            "function g() { { function f() { return 7; } } return f(); } g()",
            "7",
        ),
        (
            "function g() { var t = typeof f; { function f() {} } return t; } g()",
            "\"undefined\"",
        ),
        (
            "function g() { var f = 1; { function f() {} } return typeof f; } g()",
            "\"function\"",
        ),
        ("function g(f) { { function f() {} } return f; } g(8)", "8"),
        (
            "function g() { let f = 9; { function f() {} } return f; } g()",
            "9",
        ),
        (
            "function g() { { function f() { return 1; } { function f() { return 2; } } } return f(); } g()",
            "1",
        ),
        (
            "function g() { { function f() { return 1; } } return () => f(); } g()()",
            "1",
        ),
        (
            "var f = function g() { { function g() { return 2; } } return g(); }; f()",
            "2",
        ),
        // Mit dem Objekt `arguments` gehört der Name zu parameterNames.
        (
            "function g() { { function arguments() { return 1; } return arguments(); } } g()",
            "1",
        ),
        (
            "var g = () => { { function arguments() {} } }; g()",
            "Unsupported(18)",
        ),
    ]);
}

#[test]
fn functions_in_blocks_set_global_bindings() {
    let (mut agent, realm) = checked();
    // Gibt es schon eine lexikalische Bindung, bleibt sie.
    assert_eq!(run(&mut agent, realm, "let a = 1;"), "undefined");
    assert_eq!(run(&mut agent, realm, "{ function a() {} } a"), "1");
    assert_eq!(
        run(
            &mut agent,
            realm,
            "var b = 2; { function b() { return 3; } } b()"
        ),
        "3"
    );
    // Die Bindung zählt danach als eine aus `var`.
    assert_eq!(
        run(&mut agent, realm, "if (true) { function c() {} }"),
        "undefined"
    );
    assert_eq!(run(&mut agent, realm, "let c;"), "SyntaxError");
    assert_eq!(run(&mut agent, realm, "typeof c"), "\"function\"");
}

/// Kontexte und die Funktionen, die sie halten, überstehen jede Sammlung.
#[test]
fn contexts_survive_collections() {
    let (mut agent, realm) = agent_with(Options {
        stress: true,
        verify: true,
        ..Options::default()
    });
    let source = "var a = {}; for (let i = 0; i < 4; i++) { let s = 'n' + i; a[i] = () => s + i; } \
                  function make(x) { return function (y) { return () => x + y; }; } \
                  a[0]() + a[3]() + make('p')('q')()";
    assert_eq!(run(&mut agent, realm, source), "\"n00n33pq\"");
}

/// arguments-Objekte, Rest-Parameter und Vorgabewerte überstehen jede
/// Sammlung.
#[test]
fn arguments_survive_collections() {
    let (mut agent, realm) = agent_with(Options {
        stress: true,
        verify: true,
        ..Options::default()
    });
    let source = "function m(a, b) { b = 'b' + a; return arguments; } \
                  function u(a, ...r) { return r; } \
                  function d(a, g = () => a + arguments.length) { return g; } \
                  var o = m('x', 'y', 'z'); var r = u(1, 'p', 'q'); \
                  o[0] + o[1] + o[2] + o.length + r[0] + r[1] + r.length + d('s', undefined, 3)()";
    assert_eq!(run(&mut agent, realm, source), "\"xbxz3pq2s3\"");
}

/// Passen die Argumente eines Aufrufs aus der Engine nicht mehr als Zelle in
/// den Heap, bleibt kein Rahmen auf dem Registerstapel liegen.
#[test]
fn a_failed_argument_cell_leaves_no_frame() {
    let (mut agent, realm) = agent_with(Options {
        limit: 512 * 1024,
        verify: true,
        ..Options::default()
    });
    let source = "function f() { return arguments.length; }";
    assert_eq!(run(&mut agent, realm, source), "undefined");
    agent.scope(|agent, scope| {
        let function = agent
            .evaluate_script(scope, realm, &units("f"))
            .expect("die Funktion");
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        let arguments = vec![undefined; 100_000];
        let result = crate::operations::call(agent, scope, function, undefined, &arguments);
        assert_eq!(result.map(|_| ()), Err(Abrupt::OutOfMemory));
        assert_eq!(agent.heap_mut().push_registers(0), Some(0));
        let result = crate::operations::call(agent, scope, function, undefined, &[undefined])
            .expect("ein Aufruf");
        assert_eq!(agent.heap().get(result).as_number(), Some(1.0));
    });
}

/// Verschachtelte Pfeilfunktionen, jede mit einem eigenen Kontext, so tief,
/// wie der Parser sie annimmt, auf einem Stapel von 1 MiB (RS-8).
#[test]
fn deep_closures_need_no_deep_stack() {
    let thread = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut depth = 300;
            let (source, depth) = loop {
                let parameters: Vec<String> = (0..depth).map(|index| format!("a{index}")).collect();
                let source = format!(
                    "var f = {} => {}; f{}",
                    parameters.join(" => "),
                    parameters.join(" + "),
                    "(1)".repeat(depth)
                );
                if crate::parser::parse_script(&units(&source)).is_ok() || depth == 1 {
                    break (source, depth);
                }
                depth -= 1;
            };
            assert!(depth > 100, "{depth}");
            let (mut agent, realm) = checked();
            assert_eq!(run(&mut agent, realm, &source), depth.to_string());
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}

/// Aufrufe aus JavaScript brauchen keinen Stapel der Maschine, Aufrufe aus
/// der Engine wie Getter höchstens [`crate::agent::STACK_BUDGET`]; beides
/// endet bei zu tiefer Rekursion mit einem RangeError (RS-8).
#[test]
fn calls_run_on_a_small_stack() {
    let thread = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let (mut agent, realm) = checked();
            // This trusted regression targets frame/stack limits independently
            // of the normal execution quota. Other script tests keep that quota.
            agent.with_execution_budget(std::rc::Rc::new(crate::agent::Budget::unlimited()), |agent| {
                let deep = "function f(n) { return n === 0 ? 0 : f(n - 1) + 1; } f(100000)";
                assert_eq!(run(agent, realm, deep), "100000");
                assert_eq!(run(agent, realm, "function g() { return g(); } g()"), "RangeError");
                // Der RangeError lässt sich fangen, auch aus verschachtelten
                // Läufen, und der Abbau der Rahmen braucht keinen Stapel.
                let caught = "function h() { return h(); } try { h(); } catch (e) { e.name }";
                assert_eq!(run(agent, realm, caught), "\"RangeError\"");
                let nested = "var t = {get a() { return this.a; }}; try { t.a; } catch (e) { e.name }";
                assert_eq!(run(agent, realm, nested), "\"RangeError\"");
                for source in [
                    "var o = {get a() { return this.a; }}; o.a",
                    "var p = {valueOf() { return +p; }}; +p",
                    "var q = {set a(v) { this.a = v; }}; q.a = 1",
                    "var r = {toString() { return 'x' + r; }}; '' + r",
                ] {
                    assert_eq!(run(agent, realm, source), "RangeError", "{source}");
                }
                // Übersetzt wird erst beim ersten Aufruf, auch in der tiefsten
                // Verschachtelung, die das Budget erlaubt.
                let counting = "var n = 0, limit = Infinity, deep; function a(x) { return x; } \
                                var o = {get a() { n++; if (n < limit) return this.a; return deep(); }}; \
                                o.a";
                assert_eq!(run(agent, realm, counting), "RangeError");
                let levels = run(agent, realm, "n");
                for (open, middle, close) in [
                    ("(", "1", ")"),
                    ("[{a: ", "1", "}]"),
                    ("{ let a = 1; ", "a;", " }"),
                    ("if (1) ", "1;", ""),
                    ("1 ? ", "1", " : 1"),
                    ("a(", "1", ")"),
                ] {
                    let body = deepest_body(open, middle, close);
                    let source = format!(
                        "n = 0; limit = {levels} - 1; deep = function () {{ {body} }}; \
                         typeof o.a"
                    );
                    let result = run(agent, realm, &source);
                    assert!(result.starts_with('"'), "{open}…{close}: {result}");
                }
            });
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}

/// Ein Rumpf aus `open`, `middle` und `close`, so tief verschachtelt, wie
/// der Parser es in einer Funktion annimmt.
fn deepest_body(open: &str, middle: &str, close: &str) -> String {
    let mut depth = 300;
    loop {
        let body = format!("{}{middle}{};", open.repeat(depth), close.repeat(depth));
        let source = format!("deep = function () {{ {body} }};");
        if crate::parser::parse_script(&units(&source)).is_ok() || depth == 0 {
            return body;
        }
        depth -= 1;
    }
}

/// `new` (EvaluateNew, Abschnitt 13.3.5.1.1) mit [[Construct]] (Abschnitt
/// 10.2.2): ein neues Objekt mit dem Prototyp aus `prototype` als `this`,
/// außer der Rumpf gibt ein Objekt zurück.
#[test]
fn new_constructs_objects() {
    check(&[
        (
            "function C(a, b) { this.sum = a + b; } new C(1, 2).sum",
            "3",
        ),
        ("function C() { this.x = 1; } typeof new C", "\"object\""),
        ("function C() {} C.prototype.y = 2; new C().y", "2"),
        ("function C() {} new C() instanceof C", "true"),
        ("function C() {} new C() === new C()", "false"),
        (
            "var t; function C() { t = this; } var o = new C(); t === o",
            "true",
        ),
        (
            "function C() { this.n = arguments.length; } new C(1, 2, 3).n",
            "3",
        ),
        (
            "function C() { this.a = 1; return {b: 2}; } var o = new C(); typeof o.a + o.b",
            "\"undefined2\"",
        ),
        ("function C() { this.a = 1; return 5; } new C().a", "1"),
        ("function C() { this.a = 1; return null; } new C().a", "1"),
        ("function C() { this.a = 1; return; } new C().a", "1"),
        (
            "function C() { return function () {}; } typeof new C()",
            "\"function\"",
        ),
        (
            "function A() { this.b = new B(); } function B() { this.v = 7; } new A().b.v",
            "7",
        ),
        (
            "var ns = {C: function () { this.k = 3; }}; new ns.C().k",
            "3",
        ),
        (
            "function C() { this.x = 1; } new C.prototype.constructor().x",
            "1",
        ),
        (
            "function C() { throw 'innen'; } try { new C(); } catch (e) { e; }",
            "\"innen\"",
        ),
        // Erst der Konstruktor, dann die Argumente, dann IsConstructor.
        (
            "var log = ''; function C() {} new (log += 'k', C)(log += 'a'); log",
            "\"ka\"",
        ),
        (
            "var log = ''; try { new 1(log += 'a'); } catch (e) { log += e.name; } log",
            "\"aTypeError\"",
        ),
        // Ein Endaufruf im Rahmen von [[Construct]] bleibt ein gewöhnlicher
        // Aufruf, das Ergebnis wird noch geprüft.
        (
            "'use strict'; function g() { return 5; } function C() { this.a = 1; return g(); } new C().a",
            "1",
        ),
        (
            "'use strict'; function g() { return {z: 2}; } function C() { return g(); } new C().z",
            "2",
        ),
    ]);
}

/// Nur gewöhnliche Funktionen haben [[Construct]], keine Pfeilfunktionen,
/// Methoden oder Accessoren.
#[test]
fn new_needs_a_constructor() {
    check(&[
        ("new 1", "TypeError"),
        ("var u; new u()", "TypeError"),
        ("new ({})", "TypeError"),
        ("var f = () => {}; new f()", "TypeError"),
        ("var o = {m() {}}; new o.m()", "TypeError"),
        (
            "var o = {f: function () {}}; typeof new o.f()",
            "\"object\"",
        ),
        (
            "function C() {} try { new C.x(); } catch (e) { e.name; }",
            "\"TypeError\"",
        ),
    ]);
}

/// Ist `prototype` kein Objekt, bekommt das neue Objekt %Object.prototype%
/// aus dem Realm des Konstruktors (GetPrototypeFromConstructor, Abschnitt
/// 10.1.14).
#[test]
fn constructed_objects_fall_back_to_the_realm_of_the_constructor() {
    let (mut agent, first) = checked();
    let second = agent.create_realm().expect("Speicher");
    agent.scope(|agent, scope| {
        let constructor = agent
            .evaluate_script(scope, second, &units("(function () {})"))
            .expect("Funktion");
        let global = agent.global_object(scope, first);
        let key = agent.key_from_units(scope, &units("C")).expect("Speicher");
        crate::operations::set(agent, scope, global, key, constructor, true).expect("Zuweisung");
        for (source, realm) in [
            ("function D() {} D.prototype = null; new D()", first),
            ("C.prototype = 1; new C()", second),
        ] {
            let object = agent
                .evaluate_script(scope, first, &units(source))
                .expect("Objekt");
            let prototype = crate::object::get_prototype_of(agent, scope, object)
                .expect("kein Fehler")
                .expect("Prototyp");
            let expected = agent.intrinsic(scope, realm, crate::realm::Intrinsic::ObjectPrototype);
            let heap = agent.heap();
            assert_eq!(
                heap.get(prototype).bits(),
                heap.get(expected).bits(),
                "{source}"
            );
        }
    });
}

/// `new.target` (Abschnitt 13.3.12.1): der Konstruktor bei [[Construct]],
/// sonst *undefined*; Pfeilfunktionen sehen das ihrer Funktion.
#[test]
fn new_target_names_the_constructor() {
    check(&[
        ("function f() { return new.target; } f()", "undefined"),
        (
            "var t; function f() { t = new.target; } new f(); t === f",
            "true",
        ),
        (
            "function f() { return () => new.target; } f()()",
            "undefined",
        ),
        (
            "var t; function f() { t = (() => () => new.target)(); } new f(); t() === f",
            "true",
        ),
        ("var o = {m() { return new.target; }}; o.m()", "undefined"),
        (
            "var o = {get x() { return (() => new.target)(); }}; o.x",
            "undefined",
        ),
        (
            "var t; function f(a = new.target) { t = a; } new f(); t === f",
            "true",
        ),
        (
            "var t; function f(a = () => new.target) { t = a; } new f(); t() === f",
            "true",
        ),
        (
            "var r; function f() { r = (function () { return new.target; })(); } new f(); r",
            "undefined",
        ),
        (
            "var t; function C() { var a = () => [this, new.target]; t = a(); } \
             var o = new C(); t[0] === o && t[1] === C",
            "true",
        ),
        (
            "var t; function C(x) { var g = function () { return x; }; t = new.target; } \
             new C(1); t === C",
            "true",
        ),
    ]);
}

/// Endaufrufe im strikten Modus (Abschnitt 15.10) ersetzen den Rahmen, der
/// Registerstapel wächst nicht. 150 000 Rahmen mit je mindestens acht
/// Registern (Kopf, `this`, Funktion, Parameter und für den Aufruf
/// Funktion, `this` und Argument) passen nicht in seine 2^20 Register.
#[test]
fn tail_calls_replace_the_frame() {
    let (mut agent, realm) = checked();
    let budget = std::rc::Rc::new(crate::agent::Budget::unlimited());
    agent.with_execution_budget(budget, |agent| {
        for (source, expected) in [
            (
                "'use strict'; function f(n) { return n === 0 ? 'ende' : f(n - 1); } f(150000)",
                "\"ende\"",
            ),
            (
                "'use strict'; function even(n) { return n === 0 || odd(n - 1); } \
                 function odd(n) { return n !== 0 && even(n - 1); } even(150001)",
                "false",
            ),
            (
                "'use strict'; var f = n => n === 0 ? 'ende' : f(n - 1); f(150000)",
                "\"ende\"",
            ),
            (
                "'use strict'; var o = {m(n) { return n === 0 ? this === o : this.m(n - 1); }}; o.m(150000)",
                "true",
            ),
            (
                "'use strict'; function f(n) { try { if (n === 0) return 'ende'; throw n; } \
                 catch (e) { return f(e - 1); } } f(150000)",
                "\"ende\"",
            ),
            // Der unterste Rahmen eines Laufs aus der Engine.
            (
                "'use strict'; function f(n) { return n === 0 ? 'ende' : f(n - 1); } \
                 var o = {get x() { return f(150000); }}; o.x",
                "\"ende\"",
            ),
        ] {
            assert_eq!(run(agent, realm, source), expected, "{source}");
        }
    });
}

/// Ohne strikten Modus, nach einer Rechnung und im Block von `try` bleibt
/// derselbe Aufruf ein gewöhnlicher, und die Rekursion endet mit einem
/// RangeError.
#[test]
fn calls_outside_tail_position_keep_their_frame() {
    let (mut agent, realm) = checked();
    let budget = std::rc::Rc::new(crate::agent::Budget::unlimited());
    agent.with_execution_budget(budget, |agent| {
        for (source, expected) in [
            (
                "function f(n) { return n === 0 ? 'ende' : f(n - 1); } f(150000)",
                "RangeError",
            ),
            (
                "'use strict'; function f(n) { return n === 0 ? 0 : 1 + f(n - 1); } f(150000)",
                "RangeError",
            ),
            (
                "'use strict'; function f(n) { try { return n === 0 ? 0 : f(n - 1); } \
                 catch (e) { return e.name; } } f(150000)",
                "\"RangeError\"",
            ),
        ] {
            assert_eq!(run(agent, realm, source), expected, "{source}");
        }
    });
}

/// Ausnahmen aus Endaufrufen gehen an den Aufrufer des ersetzten Rahmens,
/// auch wenn der neue Rahmen nicht mehr auf den Registerstapel passt.
#[test]
fn tail_calls_throw_to_the_caller() {
    check(&[
        (
            "'use strict'; function thrower() { null.x; } function f(n) { return n === 0 ? thrower() : f(n - 1); } \
             try { f(1000); } catch (e) { e.name }",
            "\"TypeError\"",
        ),
        (
            "'use strict'; function g() { throw 'g'; } function f() { return g(); } \
             function h() { try { return f(); } catch (e) { return 'gefangen ' + e; } } h()",
            "\"gefangen g\"",
        ),
        (
            "'use strict'; function f() { return (0)(); } try { f(); } catch (e) { e.name }",
            "\"TypeError\"",
        ),
    ]);
    // Die Rekursion füllt den Registerstapel. Die Funktion, die den
    // RangeError fängt, ruft in Endposition `small` auf, und `small` ruft
    // `big`, dessen Rahmen größer ist als zwei von `f`. Der RangeError dafür
    // geht an den Aufruf im Rahmen darunter, bis `big` passt.
    let variables: Vec<String> = (0..40).map(|index| format!("a{index}")).collect();
    let source = format!(
        "'use strict'; var caught = 0; function big() {{ var {}; return 'groß'; }} \
         function small() {{ return big(); }} \
         function f(n) {{ try {{ return f(n + 1); }} catch (e) {{ caught++; return small(); }} }} \
         f(0) + (caught > 1)",
        variables.join(", ")
    );
    let (mut agent, realm) = checked();
    let budget = std::rc::Rc::new(crate::agent::Budget::unlimited());
    agent.with_execution_budget(budget, |agent| {
        assert_eq!(run(agent, realm, &source), "\"großtrue\"");
    });
}

/// Ein Endaufruf, dessen Funktion sich nicht übersetzen lässt, lässt keinen
/// Rahmen zurück.
#[test]
fn a_failed_tail_call_leaves_no_frame() {
    let (mut agent, realm) = checked();
    let source = "function g() { class A {} } (function () { 'use strict'; return g(); })()";
    assert!(run(&mut agent, realm, source).starts_with("Unsupported"));
    assert_eq!(run(&mut agent, realm, "1 + 1"), "2");
}

/// Konstruierte Objekte, new.target und die Argumente von Endaufrufen
/// überstehen jede Sammlung.
#[test]
fn new_and_tail_calls_survive_collections() {
    let (mut agent, realm) = agent_with(Options {
        stress: true,
        verify: true,
        ..Options::default()
    });
    let source = "'use strict'; function C(a, n) { this.s = a + n + (new.target === C); } \
                  function f(n, acc) { return n === 0 ? acc : f(n - 1, acc + new C('x', n).s); } \
                  f(3, '')";
    assert_eq!(run(&mut agent, realm, source), "\"x3truex2truex1true\"");
}

/// Ein Iterator, der seine Aufrufe in `log` mitschreibt: `@` für
/// @@iterator, `n` für `next`, `r` für `return`, jeweils hinter seinem
/// Namen. Er liefert 1 bis `count`.
const ITER: &str = "var log = ''; \
    function iter(name, count) { \
        var i = 0; var it = {}; \
        it[Symbol.iterator] = function () { log += name + '@'; return it; }; \
        it.next = function () { log += name + 'n'; i++; return { done: i > count, value: i }; }; \
        it.return = function () { log += name + 'r'; return {}; }; \
        return it; \
    } ";

fn check_iter(cases: &[(&str, &str)]) {
    for &(source, expected) in cases {
        let source = format!("{ITER}{source}");
        assert_eq!(eval(&source), expected, "{source}");
    }
}

#[test]
fn for_of_iterates_arrays_strings_and_arguments() {
    check(&[
        ("var s = ''; for (var x of [1, 2, 3]) s += x; s", "\"123\""),
        ("var s = 0; for (let x of [1, 2, 3]) s += x; s", "6"),
        ("var s = 0; for (const x of [1, 2]) { s += x; } s", "3"),
        // Codepunkte, ein einzelnes Surrogat bleibt eine Einheit.
        (
            "var s = ''; for (var c of 'a\\uD83D\\uDE00b\\uD800') s += c.length; s",
            "\"1211\"",
        ),
        ("var n = 0; for (var c of '') n++; n", "0"),
        (
            "function f() { var s = ''; for (var x of arguments) s += x; return s; } f(1, 2, 3)",
            "\"123\"",
        ),
        (
            "function f(a) { a = 5; var s = ''; for (var x of arguments) s += x; return s; } f(1, 2)",
            "\"52\"",
        ),
        // Ein Element, das während der Schleife dazukommt, kommt noch dran.
        (
            "var a = [1, 2]; var n = 0; for (var x of a) { if (n++ === 0) a[2] = 3; } n",
            "3",
        ),
        (
            "var a = [, 1]; var s = ''; for (var x of a) s += x; s",
            "\"undefined1\"",
        ),
    ]);
}

#[test]
fn for_of_binds_each_kind_of_target() {
    check(&[
        ("for (var g of [7]); g", "7"),
        ("for (h of [8]); h", "8"),
        ("var o = {}; for (o.p of [1, 2]); o.p", "2"),
        (
            "var a = []; var i = 0; for (a[i++] of 'xy'); a[0] + a[1] + i",
            "\"xy2\"",
        ),
        ("function f() { for (var x of [1, 2]); return x; } f()", "2"),
        ("function f() { var y; for (y of [3]); return y; } f()", "3"),
        ("'use strict'; for (u of [1]);", "ReferenceError"),
        ("for (const x of [1]) x = 2;", "TypeError"),
        // Jeder Durchlauf hat eigene Bindungen.
        (
            "var fs = [], i = 0; for (let x of [1, 2, 3]) fs[i++] = function () { return x; }; \
             '' + fs[0]() + fs[1]() + fs[2]()",
            "\"123\"",
        ),
        (
            "var fs = [], i = 0; for (const x of 'ab') { let y = x + i; fs[i++] = () => y + x; } \
             fs[0]() + fs[1]()",
            "\"a0ab1b\"",
        ),
        // Die rechte Seite sieht die Namen aus dem Kopf in der TDZ.
        ("for (let x of [x]);", "ReferenceError"),
        ("let x = 1; { for (let x of [x]); }", "ReferenceError"),
        (
            "var f; for (let x of (f = function () { return x; }, [1])); f()",
            "ReferenceError",
        ),
        ("var x = 1; for (var x of [x + 1]); x", "2"),
        // Funktionen im Rumpf nach Anhang B.3.2.
        (
            "for (var x of [1]) { function g() { return 'g'; } } g()",
            "\"g\"",
        ),
        (
            "for (let g of [1]) { function g() {} } typeof g",
            "\"undefined\"",
        ),
    ]);
}

#[test]
fn for_of_has_a_completion_value() {
    check(&[
        ("1; for (var x of []);", "undefined"),
        ("for (var x of [1, 2]) x * 10;", "20"),
        ("1; for (var x of [1]) { 2; break; }", "2"),
        ("1; for (var x of [1]) { break; }", "undefined"),
        // `if` macht aus ~empty~ *undefined* (Abschnitt 14.6.2).
        (
            "1; for (var x of [1, 2]) { if (x == 2) continue; x; }",
            "undefined",
        ),
        ("1; for (var x of ['a', 'b']) { x; continue; }", "\"b\""),
    ]);
}

#[test]
fn for_of_closes_iterators_it_leaves() {
    check_iter(&[
        (
            "for (var x of iter('a', 2)) log += x; log",
            "\"a@an1an2an\"",
        ),
        ("for (var x of iter('a', 5)) break; log", "\"a@anar\""),
        ("for (var x of iter('a', 2)) continue; log", "\"a@ananan\""),
        (
            "function f() { for (var x of iter('a', 5)) return x; } f() + log",
            "\"1a@anar\"",
        ),
        (
            "try { for (var x of iter('a', 5)) throw 'e'; } catch (e) { log += e; } log",
            "\"a@anare\"",
        ),
        (
            "outer: for (var x of iter('a', 2)) for (var y of iter('b', 2)) continue outer; log",
            "\"a@anb@bnbranb@bnbran\"",
        ),
        (
            "outer: for (var x of iter('a', 2)) for (var y of iter('b', 2)) break outer; log",
            "\"a@anb@bnbrar\"",
        ),
        (
            "block: { for (var x of iter('a', 2)) break block; log += 'x'; } log",
            "\"a@anar\"",
        ),
        (
            "function f() { for (var x of iter('a', 5)) try { return 'v'; } finally { log += 'f'; } } \
             f() + log",
            "\"va@anfar\"",
        ),
        (
            "function f() { try { for (var x of iter('a', 5)) return 'v'; } finally { log += 'f'; } } \
             f() + log",
            "\"va@anarf\"",
        ),
        (
            "for (var x of iter('a', 3)) { try { if (x < 3) continue; } finally { log += 'f'; } } log",
            "\"a@anfanfanfan\"",
        ),
        // Ein Fehler beim Binden schließt den Iterator auch.
        (
            "(function () { 'use strict'; try { for (u of iter('a', 1)); } catch (e) { log += 'c'; } })(); log",
            "\"a@anarc\"",
        ),
        (
            "var o = { set p(v) { throw 's'; } }; \
             try { for (o.p of iter('a', 1)); } catch (e) { log += e; } log",
            "\"a@anars\"",
        ),
        (
            "try { for (var x of iter('a', 1)) null.p; } catch (e) { log += e.constructor === TypeError; } log",
            "\"a@anartrue\"",
        ),
    ]);
}

#[test]
fn for_of_follows_the_iterator_protocol() {
    check_iter(&[
        ("for (var x of 1);", "TypeError"),
        ("for (var x of {});", "TypeError"),
        ("for (var x of undefined);", "TypeError"),
        (
            "var it = {}; it[Symbol.iterator] = function () { return 1; }; for (var x of it);",
            "TypeError",
        ),
        (
            "var it = {}; it[Symbol.iterator] = 1; for (var x of it);",
            "TypeError",
        ),
        (
            "var it = {}; it[Symbol.iterator] = function () { return {}; }; for (var x of it);",
            "TypeError",
        ),
        (
            "var it = iter('a', 5); it.next = function () { return 1; }; for (var x of it);",
            "TypeError",
        ),
        // `next` steht einmal fest.
        (
            "var it = iter('a', 2); var next = it.next; \
             it.next = function () { it.next = null; return next(); }; \
             for (var x of it) log += x; log",
            "\"a@an1an2an\"",
        ),
        // Fehler aus `next`, `done` und `value` schließen den Iterator nicht.
        (
            "var it = iter('a', 5); it.next = function () { throw 'n'; }; \
             try { for (var x of it); } catch (e) { log += e; } log",
            "\"a@n\"",
        ),
        (
            "var it = iter('a', 5); it.next = function () { return { get done() { throw 'd'; } }; }; \
             try { for (var x of it); } catch (e) { log += e; } log",
            "\"a@d\"",
        ),
        (
            "var it = iter('a', 5); it.next = function () { return { done: 0, get value() { throw 'v'; } }; }; \
             try { for (var x of it); } catch (e) { log += e; } log",
            "\"a@v\"",
        ),
        (
            "var it = iter('a', 5); it.next = function () { return { done: 'ja', value: 1 }; }; \
             for (var x of it) log += x; log",
            "\"a@\"",
        ),
        // `return` beim Verlassen: Ohne Ausnahme gelten sein Fehler und ein
        // Ergebnis, das kein Objekt ist; mit einer bleibt sie.
        (
            "var it = iter('a', 5); it.return = function () { throw 'r'; }; \
             try { for (var x of it) break; } catch (e) { log += e; } log",
            "\"a@anr\"",
        ),
        (
            "var it = iter('a', 5); it.return = function () { return 1; }; \
             try { for (var x of it) break; } catch (e) { log += e.constructor === TypeError; } log",
            "\"a@antrue\"",
        ),
        (
            "var it = iter('a', 5); it.return = function () { throw 'r'; }; \
             try { for (var x of it) throw 'e'; } catch (e) { log += e; } log",
            "\"a@ane\"",
        ),
        (
            "var it = iter('a', 5); it.return = 1; \
             try { for (var x of it) throw 'e'; } catch (e) { log += e; } log",
            "\"a@ane\"",
        ),
        (
            "var it = iter('a', 5); it.return = 1; \
             try { for (var x of it) break; } catch (e) { log += e.constructor === TypeError; } log",
            "\"a@antrue\"",
        ),
        (
            "var it = iter('a', 5); it.return = null; for (var x of it) break; log",
            "\"a@an\"",
        ),
        (
            "var it = iter('a', 5); it.return = function () { return 1; }; \
             try { for (var x of it) throw 'e'; } catch (e) { log += e; } log",
            "\"a@ane\"",
        ),
    ]);
}

#[test]
fn spread_fills_arrays() {
    check_iter(&[
        ("[...'ab', ...[1, 2], 3].length", "5"),
        (
            "var a = [...'a\\uD83D\\uDE00']; a.length + a[1]",
            "\"2\u{1F600}\"",
        ),
        (
            "var a = [, ...[1]]; a.length + ' ' + (0 in a) + a[1]",
            "\"2 false1\"",
        ),
        ("var a = [...[], , ...[]]; a.length", "1"),
        ("[...[1, , 3]][1]", "undefined"),
        ("[...[1, , 3]].length", "3"),
        ("[...iter('a', 2)].length + log", "\"2a@ananan\""),
        (
            "var a = [...[1].keys(), ...[5].entries()]; a[0] + a[1][0] + a[1][1]",
            "5",
        ),
        ("[...1]", "TypeError"),
        ("[...{}]", "TypeError"),
        ("[...null]", "TypeError"),
        (
            "var it = iter('a', 5); it.next = function () { throw 'n'; }; \
             try { [...it]; } catch (e) { log += e; } log",
            "\"a@n\"",
        ),
    ]);
}

#[test]
fn spread_fills_arguments() {
    check_iter(&[
        (
            "function f(a, b, c) { return '' + a + b + c + arguments.length; } f(...[1, 2], 3)",
            "\"1233\"",
        ),
        (
            "function f(a, b, c) { return '' + a + b + c + arguments.length; } f(...[])",
            "\"undefinedundefinedundefined0\"",
        ),
        (
            "function f(a, b, c) { return '' + a + b + c + arguments.length; } f(0, ...'ab', ...[])",
            "\"0ab3\"",
        ),
        (
            "function f(...r) { return r.length + r[3]; } f(...iter('a', 3), 'x') + log",
            "\"4xa@anananan\"",
        ),
        (
            "var o = { m: function (x) { return this === o && x; } }; o.m(...[5])",
            "5",
        ),
        ("var o = { m(x) { return x; } }; o?.m(...[6])", "6"),
        ("var n = null; n?.m(...(log = 'x', [])); log", "\"\""),
        ("var o = {}; o.m?.(...(log = 'x', [])); log", "\"\""),
        (
            "function C(a, b) { this.s = a + b; } new C(...['x', 'y']).s",
            "\"xy\"",
        ),
        (
            "function D() { this.t = new.target === D; } new D(...[]).t",
            "true",
        ),
        ("String(...['ab', 'c'])", "\"ab\""),
        ("new String(...['ab']).length", "2"),
        ("String(Symbol(...['d']))", "\"Symbol(d)\""),
        ("new Symbol(...[])", "TypeError"),
        ("function f() {} f(...1)", "TypeError"),
        ("var x = 1; x(...[])", "TypeError"),
        ("var x = 1; new x(...[])", "TypeError"),
        ("var x = () => 1; new x(...[])", "TypeError"),
        // Erst die Argumente, dann IsCallable und IsConstructor.
        (
            "var x = 1; try { x(...(log += 'a', [])); } catch (e) { log += 'c'; } log",
            "\"ac\"",
        ),
        (
            "var x = 1; try { new x(...(log += 'a', [])); } catch (e) { log += 'c'; } log",
            "\"ac\"",
        ),
    ]);
}

#[test]
fn spread_calls_in_tail_position_replace_the_frame() {
    let (mut agent, realm) = checked();
    let budget = std::rc::Rc::new(crate::agent::Budget::unlimited());
    agent.with_execution_budget(budget, |agent| {
        let source = "'use strict'; function f(n) { return n === 0 ? 'ende' : f(...[n - 1]); } \
                      f(100000)";
        assert_eq!(run(agent, realm, source), "\"ende\"");
        let source = "function f(n) { return n === 0 ? 'ende' : f(...[n - 1]); } f(100000)";
        assert_eq!(run(agent, realm, source), "RangeError");
    });
}

#[test]
fn symbols_come_from_the_symbol_function() {
    check(&[
        ("typeof Symbol()", "\"symbol\""),
        ("typeof Symbol.iterator", "\"symbol\""),
        ("Symbol('d') === Symbol('d')", "false"),
        (
            "String(Symbol('d')) + String(Symbol())",
            "\"Symbol(d)Symbol()\"",
        ),
        ("String(Symbol(1))", "\"Symbol(1)\""),
        ("String(Symbol.iterator)", "\"Symbol(Symbol.iterator)\""),
        ("new Symbol()", "TypeError"),
        ("Symbol(Symbol())", "TypeError"),
        ("Symbol.length + Symbol.name", "\"0Symbol\""),
        ("Symbol.iterator = 1; typeof Symbol.iterator", "\"symbol\""),
        ("'use strict'; Symbol.iterator = 1;", "TypeError"),
        ("Symbol.prototype.constructor === Symbol", "true"),
        ("var s = Symbol(); var o = {}; o[s] = 1; o[s]", "1"),
    ]);
}

#[test]
fn array_and_string_iterators_step_through_their_values() {
    check(&[
        (
            "var it = [1, 2].values(); var r = it.next(); r.value + ' ' + r.done",
            "\"1 false\"",
        ),
        (
            "var it = [1].values(); it.next(); var r = it.next(); r.value + ' ' + r.done",
            "\"undefined true\"",
        ),
        // Einmal fertig, bleibt er fertig.
        (
            "var a = [1]; var it = a.values(); it.next(); it.next(); a[1] = 2; it.next().done",
            "true",
        ),
        ("var r = ['a'].keys().next(); r.value", "0"),
        (
            "var r = ['a'].entries().next().value; '' + r.length + r[0] + r[1]",
            "\"20a\"",
        ),
        ("[][Symbol.iterator] === [].values", "true"),
        ("var it = [].values(); it[Symbol.iterator]() === it", "true"),
        (
            "Object.prototype.toString.call([].values())",
            "\"[object Array Iterator]\"",
        ),
        (
            "Object.prototype.toString.call(''[Symbol.iterator]())",
            "\"[object String Iterator]\"",
        ),
        // Array.prototype.values nimmt jedes Objekt mit `length`.
        (
            "var o = { length: 2, 0: 'x', 1: 'y' }; var s = ''; \
             for (var v of [].values.call(o)) s += v; s",
            "\"xy\"",
        ),
        (
            "var s = ''; for (var v of [].values.call('ab')) s += v; s",
            "\"ab\"",
        ),
        ("[].values.call(null)", "TypeError"),
        ("[].values().next.call({})", "TypeError"),
        ("[].values().next.call(''[Symbol.iterator]())", "TypeError"),
        ("''[Symbol.iterator]().next.call([].values())", "TypeError"),
        ("''[Symbol.iterator].call(null)", "TypeError"),
        (
            "var it = ''[Symbol.iterator].call(12); it.next().value + it.next().value",
            "\"12\"",
        ),
        (
            "var it = 'a'[Symbol.iterator](); it.next(); var r = it.next(); r.value + ' ' + r.done",
            "\"undefined true\"",
        ),
        // Die Länge kommt bei jedem Schritt neu, ToLength begrenzt sie.
        (
            "var o = { length: -1 }; [].values.call(o).next().done",
            "true",
        ),
        (
            "var n = 0; var o = { get length() { n++; return 1; } }; var it = [].values.call(o); \
             it.next(); it.next(); it.next(); n",
            "2",
        ),
        (
            "var o = { get length() { throw 'l'; } }; [].values.call(o).next()",
            "throw \"l\"",
        ),
    ]);
}

#[test]
fn arguments_objects_are_iterable() {
    check(&[
        (
            "function g(a) { 'use strict'; return [...arguments].length; } g(1, 2)",
            "2",
        ),
        (
            "(function () { return arguments[Symbol.iterator] === [].values; })()",
            "true",
        ),
        (
            "(function () { 'use strict'; arguments[Symbol.iterator] = 1; return arguments[Symbol.iterator]; })()",
            "1",
        ),
        (
            "(function (a) { return delete arguments[Symbol.iterator] && arguments[Symbol.iterator]; })(1)",
            "undefined",
        ),
    ]);
}

/// Iteratoren, `for`-`of` und die Ausbreitung überstehen jede Sammlung.
#[test]
fn iteration_survives_collections() {
    let (mut agent, realm) = agent_with(Options {
        stress: true,
        verify: true,
        ..Options::default()
    });
    let source = format!(
        "{ITER} var s = ''; var fs = [], i = 0; \
         for (let c of 'x\\uD83D\\uDE00') fs[i++] = () => c; \
         for (var e of [1, 2].entries()) s += e[0] + e[1]; \
         function f(a, b) {{ return a + b + arguments.length; }} \
         for (var x of iter('a', 3)) {{ if (x == 2) break; s += f(...[x, 'y'], ...'z'); }} \
         s + fs[0]() + fs[1]().length + log + [...iter('b', 1)].length"
    );
    assert_eq!(run(&mut agent, realm, &source), "\"131y3x2a@ananar1\"");
}
