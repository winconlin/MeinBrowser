use super::*;
use crate::parser::{ErrorKind, parse_module, parse_script};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn compile(source: &str) -> Result<Script> {
    let ast = parse_script(&units(source)).expect("gültiges Script");
    compile_script(&ast)
}

/// Die Befehle ohne ihren Abstand.
fn listing(source: &str) -> Vec<String> {
    let script = compile(source).expect("übersetzbar");
    script
        .code
        .to_string()
        .lines()
        .map(|line| {
            line.split_once(": ")
                .map_or(line, |(_, rest)| rest)
                .to_owned()
        })
        .collect()
}

fn names(list: &[Vec<u16>]) -> Vec<String> {
    list.iter()
        .map(|units| String::from_utf16(units).expect("UTF-16"))
        .collect()
}

#[test]
fn expressions_use_the_accumulator() {
    assert_eq!(
        listing("1 + 2;"),
        [
            "LdaSmi 1", "Star r2", "LdaSmi 2", "Add r2", "Star r1", "Ldar r1", "Return"
        ]
    );
    let script = compile("1 + 2;").expect("übersetzbar");
    assert_eq!(script.code.register_count(), 3);
    assert!(!script.code.is_strict());
    assert!(
        compile("'use strict'; 1;")
            .expect("übersetzbar")
            .code
            .is_strict()
    );
}

#[test]
fn constants_are_shared_and_kept_apart_by_kind() {
    assert_eq!(
        listing("1.5; 2147483648; 1.5;"),
        [
            "LdaConstant 1.5",
            "Star r1",
            "LdaConstant 2147483648.0",
            "Star r1",
            "LdaConstant 1.5",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    let script = compile("'x'; x; 'x'; x;").expect("übersetzbar");
    assert_eq!(
        script.code.constants(),
        [Constant::String(units("x")), Constant::Name(units("x"))]
    );
}

#[test]
fn global_declarations_are_collected() {
    let script = compile(
        "var a, b = 1; let c; const d = 2; var a;
         { var e; let f; }
         for (var g;;) break;
         switch (0) { case 0: var h; }
         if (a) var i; else var j;
         while (0) var k;
         do var l; while (0)
         m: var n;",
    )
    .expect("übersetzbar");
    assert_eq!(
        names(&script.var_names),
        ["a", "b", "e", "g", "h", "i", "j", "k", "l", "n"]
    );
    let lexical: Vec<(String, bool)> = script
        .lexical_names
        .iter()
        .map(|(units, constant)| (String::from_utf16(units).expect("UTF-16"), *constant))
        .collect();
    assert_eq!(lexical, [("c".to_owned(), false), ("d".to_owned(), true)]);
}

#[test]
fn block_bindings_live_in_registers() {
    assert_eq!(
        listing("{ let x = 1; x; }"),
        [
            "LdaHole", "Star r2", "LdaSmi 1", "Star r2", "Ldar r2", "Star r1", "Ldar r1", "Return"
        ]
    );
}

#[test]
fn reads_before_the_declaration_check_the_hole() {
    assert_eq!(
        listing("{ x; let x = 1; }"),
        [
            "LdaHole",
            "Star r2",
            "CheckHole r2 x",
            "Ldar r2",
            "Star r1",
            "LdaSmi 1",
            "Star r2",
            "Ldar r1",
            "Return"
        ]
    );
    // Auch im eigenen Initialisierer.
    assert!(listing("{ let x = x; }").contains(&"CheckHole r2 x".to_owned()));
}

#[test]
fn bindings_in_switch_always_check_the_hole() {
    assert_eq!(
        listing("switch (0) { case 0: let x = 1; x; }"),
        [
            "LdaSmi 0",
            "Star r2",
            "LdaHole",
            "Star r3",
            "LdaUndefined",
            "Star r1",
            "LdaSmi 0",
            "TestEqualStrict r2",
            "JumpIfTrue @26",
            "Jump @37",
            "LdaSmi 1",
            "Star r3",
            "CheckHole r3 x",
            "Ldar r3",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
}

#[test]
fn assigning_a_constant_checks_the_hole_first() {
    assert_eq!(
        listing("{ const c = 1; c = 2; }"),
        [
            "LdaHole",
            "Star r2",
            "LdaSmi 1",
            "Star r2",
            "LdaSmi 2",
            "CheckHole r2 c",
            "ThrowConstAssignment c",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
}

#[test]
fn global_assignment_resolves_before_the_right_side() {
    assert_eq!(
        listing("x = y;"),
        [
            "ResolveGlobal x r2",
            "LdaGlobal y",
            "StaGlobal x r2",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
}

#[test]
fn loops_reset_the_completion_value() {
    assert_eq!(
        listing("while (a) b;"),
        [
            "LdaUndefined",
            "Star r1",
            "LdaGlobal a",
            "JumpIfFalse @17",
            "LdaGlobal b",
            "Star r1",
            "JumpLoop @3",
            "Ldar r1",
            "Return"
        ]
    );
}

#[test]
fn instructions_know_their_place_in_the_source() {
    let script = compile("x;\nthrow 1;").expect("übersetzbar");
    let code = &script.code;
    let throw = code.decode(6).expect("Befehl");
    assert_eq!(throw.op, Op::Throw);
    assert_eq!(code.position(6), Some(3));
    assert_eq!(code.position(0), Some(0));
}

#[test]
fn unsupported_constructs_name_their_place() {
    let cases = [
        ("function f() {}", 0),
        ("class A {}", 0),
        ("a.b;", 0),
        ("x = {};", 4),
        ("x = [];", 4),
        ("a in b;", 0),
        ("a instanceof b;", 0),
        ("delete a.b;", 7),
        ("delete (a[0]);", 8),
        ("try {} catch {}", 0),
        ("for (var k in o);", 0),
        ("for (let k of o);", 0),
        ("f();", 0),
        ("[a] = b;", 0),
        ("let {a} = b;", 4),
        ("1n;", 0),
        ("/a/;", 0),
        ("x => x;", 0),
        ("with (a) {}", 0),
        ("{ function g() {} }", 2),
    ];
    for (source, position) in cases {
        assert_eq!(
            compile(source).map(|_| ()),
            Err(CompileError::Unsupported(position)),
            "{source}"
        );
    }
    let module = parse_module(&units("1;")).expect("gültiges Modul");
    assert_eq!(
        compile_script(&module).map(|_| ()),
        Err(CompileError::Unsupported(0))
    );
}

/// Die tiefste Verschachtelung, die der Parser annimmt, übersetzt sich auf
/// einem Stapel von 1 MiB, und lange Ketten zweistelliger Operatoren
/// brauchen keinen tiefen Stapel (RS-8).
#[test]
fn deep_nesting_needs_no_deep_stack() {
    let forms = [
        ("(", "1", ")"),
        ("{", "", "}"),
        ("{ let a = 1; ", "a;", " }"),
        ("if (1) ", "1;", ""),
        ("while (0) ", ";", ""),
        ("switch (1) { case 1: ", "1;", " }"),
        ("a = ", "1", ""),
        // Mit Leerzeichen, sonst wird daraus `--`.
        ("- ", "1", ""),
        ("!(", "1", ")"),
        ("1 ? ", "1", " : 1"),
        ("`${", "1", "}`"),
        ("(1, ", "1", ")"),
        ("1 + (", "1", ")"),
        ("typeof (", "a", ")"),
    ];
    let thread = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            for (open, middle, close) in forms {
                let mut depth = 300;
                let ast = loop {
                    let text = format!("{}{middle}{};", open.repeat(depth), close.repeat(depth));
                    match parse_script(&units(&text)) {
                        Ok(ast) => break ast,
                        Err(error) if error.kind == ErrorKind::TooDeep && depth > 1 => depth -= 1,
                        Err(error) => panic!("{open}…{close}: {error:?}"),
                    }
                };
                assert!(depth > 50, "{open}…{close}: nur Tiefe {depth}");
                assert!(
                    compile_script(&ast).is_ok(),
                    "{open}…{close} bei Tiefe {depth}"
                );
            }

            for operator in ["+", "&&", "??", "==="] {
                let text = format!("{}1;", format!("1 {operator} ").repeat(100_000));
                let ast = parse_script(&units(&text)).expect("gültiges Script");
                let script = compile_script(&ast).expect("übersetzbar");
                // Links geklammert braucht die Kette nur ein Register mehr.
                assert!(script.code.register_count() <= 3, "{operator}");
            }
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}
