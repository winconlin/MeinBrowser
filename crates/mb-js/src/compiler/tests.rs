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
fn property_references_keep_base_and_key_in_registers() {
    assert_eq!(
        listing("a.b = c;"),
        [
            "LdaGlobal a",
            "Star r2",
            "LdaGlobal c",
            "StaNamed r2 b",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    // ToPropertyKey kommt erst beim Lesen, und das Lesen merkt sich den
    // Schlüssel für das Schreiben.
    assert_eq!(
        listing("a[k] += 1;"),
        [
            "LdaGlobal a",
            "Star r2",
            "LdaGlobal k",
            "Star r3",
            "LdaKeyedRef r2 r3",
            "Star r4",
            "LdaSmi 1",
            "Add r4",
            "StaKeyed r2 r3",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    assert_eq!(
        listing("(a.b)++;"),
        [
            "LdaGlobal a",
            "Star r2",
            "Ldar r2",
            "LdaNamed b",
            "ToNumeric",
            "Star r3",
            "Inc",
            "StaNamed r2 b",
            "Ldar r3",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    assert_eq!(
        listing("a.b[0]; a in b; a instanceof b;"),
        [
            "LdaGlobal a",
            "LdaNamed b",
            "Star r2",
            "LdaSmi 0",
            "LdaKeyed r2",
            "Star r1",
            "LdaGlobal a",
            "Star r2",
            "LdaGlobal b",
            "TestIn r2",
            "Star r1",
            "LdaGlobal a",
            "Star r2",
            "LdaGlobal b",
            "TestInstanceOf r2",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
}

#[test]
fn literals_define_their_properties_in_order() {
    assert_eq!(
        listing("({a: 1, [k]: 2, ...s, __proto__: null, b, 1.5: 3});"),
        [
            "CreateObject",
            "Star r2",
            "LdaSmi 1",
            "DefineNamed r2 a",
            "LdaGlobal k",
            "ToPropertyKey",
            "Star r3",
            "LdaSmi 2",
            "DefineKeyed r2 r3",
            "LdaGlobal s",
            "CopyDataProperties r2",
            "LdaNull",
            "SetPrototypeLiteral r2",
            "LdaGlobal b",
            "DefineNamed r2 b",
            "LdaSmi 3",
            "DefineNamed r2 1.5",
            "Ldar r2",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    // Nur `__proto__: value` setzt den Prototyp, auch als Zeichenkette.
    let proto = listing("({'__proto__': a, __proto__, ['__proto__']: b});");
    assert_eq!(
        proto
            .iter()
            .filter(|line| line.starts_with("SetPrototypeLiteral"))
            .count(),
        1
    );
    assert!(proto.contains(&"DefineNamed r2 __proto__".to_owned()));
    assert!(proto.contains(&"DefineKeyed r2 r3".to_owned()));
    assert_eq!(
        listing("[1, , a];"),
        [
            "CreateArray",
            "Star r2",
            "LdaSmi 1",
            "ArrayPush r2",
            "ArrayHole r2",
            "LdaGlobal a",
            "ArrayPush r2",
            "Ldar r2",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
}

#[test]
fn optional_chains_jump_to_one_end() {
    assert_eq!(
        listing("a?.b.c;"),
        [
            "LdaGlobal a",
            "JumpIfNullish @18",
            "LdaNamed b",
            "LdaNamed c",
            "Jump @19",
            "LdaUndefined",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    assert_eq!(
        listing("delete a?.[b];"),
        [
            "LdaGlobal a",
            "JumpIfNullish @20",
            "Star r2",
            "LdaGlobal b",
            "DeleteKeyed r2",
            "Jump @21",
            "LdaTrue",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    // Ohne `?.` braucht `delete` keinen Sprung.
    assert_eq!(
        listing("delete (a.b);"),
        [
            "LdaGlobal a",
            "DeleteNamed b",
            "Star r1",
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
        ("async function* f() {}", 0),
        ("class A { async *m() {} }", 10),
        ("x = {async *a() {}};", 5),
        ("x = {async *['a']() {}};", 5),
        ("x = {1n: 1};", 5),
        ("let {1n: a} = b;", 5),
        ("[a = class { async *m() {} }] = b;", 13),
        ("for (var k in o);", 0),
        ("f`x`;", 0),
        ("1n;", 0),
        ("/a/;", 0),
        ("(async function* () {});", 1),
        ("with (a) {}", 0),
        ("l: function g() {}", 3),
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
        ("[", "1", "]"),
        ("[{a: ", "1", "}]"),
        ("a[", "0", "]"),
        ("a?.[", "0", "]"),
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

            // Ketten von Zugriffen baut der Parser in einer Schleife.
            for link in [".b", "?.b", "[0]", "?.[0]"] {
                for prefix in ["", "delete "] {
                    let text = format!("{prefix}a{};", link.repeat(100_000));
                    let ast = parse_script(&units(&text)).expect("gültiges Script");
                    let script = compile_script(&ast).expect("übersetzbar");
                    assert!(script.code.register_count() <= 3, "{prefix}{link}");
                }
            }
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}

#[test]
fn calls_put_callee_this_and_arguments_in_a_row() {
    assert_eq!(
        listing("f(1, 2);"),
        [
            "LdaGlobal f",
            "Star r2",
            "LdaUndefined",
            "Star r3",
            "LdaSmi 1",
            "Star r4",
            "LdaSmi 2",
            "Star r5",
            "Call r2 2",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    // Die Basis eines Zugriffs ist `this`, auch in Klammern.
    let member = [
        "LdaGlobal a",
        "Star r3",
        "LdaNamed b",
        "Star r2",
        "Call r2 0",
        "Star r1",
        "Ldar r1",
        "Return",
    ];
    assert_eq!(listing("a.b();"), member);
    assert_eq!(listing("(a.b)();"), member);
    assert_eq!(
        listing("a[k]();"),
        [
            "LdaGlobal a",
            "Star r3",
            "LdaGlobal k",
            "LdaKeyed r3",
            "Star r2",
            "Call r2 0",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    assert_eq!(
        listing("a.b?.();"),
        [
            "LdaGlobal a",
            "Star r3",
            "LdaNamed b",
            "JumpIfNullish @23",
            "Star r2",
            "Call r2 0",
            "Jump @24",
            "LdaUndefined",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
}

#[test]
fn functions_are_created_with_their_names() {
    assert_eq!(
        listing("x = function () {};"),
        [
            "ResolveGlobal x r2",
            "CreateClosure fn0 x",
            "StaGlobal x r2",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    assert_eq!(
        listing("x = {m() {}, get g() {}, [k]: function () {}};"),
        [
            "ResolveGlobal x r2",
            "CreateObject",
            "Star r3",
            "CreateClosure fn0 m",
            "DefineNamed r3 m",
            "LdaConstant g",
            "Star r4",
            "CreateClosure fn1 g",
            "DefineGetter r3 r4",
            "LdaGlobal k",
            "ToPropertyKey",
            "Star r4",
            "CreateClosureKeyed fn2 r4",
            "DefineKeyed r3 r4",
            "Ldar r3",
            "StaGlobal x r2",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
}

#[test]
fn function_declarations_are_left_to_global_declaration_instantiation() {
    let script = compile("var a; function f() {} var f; function g() {} function f() {}")
        .expect("übersetzbar");
    assert_eq!(names(&script.var_names), ["a"]);
    let functions: Vec<Vec<u16>> = script
        .functions
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    assert_eq!(names(&functions), ["g", "f"]);
    // Von zwei Deklarationen mit demselben Namen gilt die letzte.
    let ids: Vec<usize> = script.functions.iter().map(|(_, id)| id.index()).collect();
    assert_eq!(ids, [1, 2]);
    assert_eq!(listing("function f() {}"), ["Ldar r1", "Return"]);
}

#[test]
fn functions_keep_parameters_and_variables_in_registers() {
    let ast = parse_script(&units("function f(a) { var b = a; return b; }")).expect("gültig");
    let Some(&Node::FunctionDeclaration(id)) = ast.body().first().map(|&item| ast.node(item))
    else {
        panic!("keine Funktion");
    };
    let template = FunctionTemplate {
        function: id,
        binds_name: false,
        outer: Rc::default(),
    };
    let code = compile_function(&ast, &template).expect("übersetzbar");
    let lines: Vec<String> = code
        .to_string()
        .lines()
        .map(|line| {
            line.split_once(": ")
                .map_or(line, |(_, rest)| rest)
                .to_owned()
        })
        .collect();
    assert_eq!(
        lines,
        [
            "Ldar r2",
            "Star r3",
            "Ldar r3",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    assert_eq!((code.register_count(), code.parameter_count()), (4, 1));
}

/// Die Befehle ohne ihren Abstand.
fn lines(code: &Code) -> Vec<String> {
    code.to_string()
        .lines()
        .map(|line| {
            line.split_once(": ")
                .map_or(line, |(_, rest)| rest)
                .to_owned()
        })
        .collect()
}

/// Die erste Funktion in den Konstanten von `code`.
fn first_template(code: &Code) -> FunctionTemplate {
    code.constants()
        .iter()
        .find_map(|constant| match constant {
            Constant::Function(template) => Some(template.clone()),
            _ => None,
        })
        .expect("eine Funktion")
}

#[test]
fn closures_read_and_write_contexts() {
    let source = "var f; { let x = 1; f = function () { x = 2; return x; }; }";
    let ast = parse_script(&units(source)).expect("gültig");
    let script = compile_script(&ast).expect("übersetzbar");
    assert_eq!(
        lines(&script.code),
        [
            "PushContext 1",
            "LdaHole",
            "StaContext 0 1",
            "LdaSmi 1",
            "StaContext 0 1",
            "ResolveGlobal f r2",
            "CreateClosure fn0 f",
            "StaGlobal f r2",
            "Star r1",
            "PopContext",
            "Ldar r1",
            "Return"
        ]
    );
    // Die Funktion prüft die TDZ bei jedem Zugriff.
    let code = compile_function(&ast, &first_template(&script.code)).expect("übersetzbar");
    assert_eq!(
        lines(&code),
        [
            "LdaSmi 2",
            "Star r2",
            "LdaContext 0 1",
            "ThrowIfHole x",
            "Ldar r2",
            "StaContext 0 1",
            "LdaContext 0 1",
            "ThrowIfHole x",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
}

#[test]
fn parameters_and_this_are_copied_into_the_context() {
    let source = "function f(a, b) { return () => a + this.c; }";
    let ast = parse_script(&units(source)).expect("gültig");
    let Some(&Node::FunctionDeclaration(id)) = ast.body().first().map(|&item| ast.node(item))
    else {
        panic!("keine Funktion");
    };
    let template = FunctionTemplate {
        function: id,
        binds_name: false,
        outer: Rc::default(),
    };
    let code = compile_function(&ast, &template).expect("übersetzbar");
    assert_eq!(
        lines(&code),
        [
            "PushContext 2",
            "Ldar r2",
            "StaContext 0 1",
            "Ldar r0",
            "StaContext 0 2",
            "CreateClosure fn0 ",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    let arrow = compile_function(&ast, &first_template(&code)).expect("übersetzbar");
    assert_eq!(
        lines(&arrow),
        [
            "LdaContext 0 1",
            "Star r2",
            "LdaContext 0 2",
            "LdaNamed c",
            "Add r2",
            "Return"
        ]
    );
}

/// Der Code der ersten Funktionsdeklaration in `source`.
fn first_function(source: &str) -> Code {
    let ast = parse_script(&units(source)).expect("gültig");
    let Some(&Node::FunctionDeclaration(id)) = ast.body().first().map(|&item| ast.node(item))
    else {
        panic!("keine Funktion");
    };
    let template = FunctionTemplate {
        function: id,
        binds_name: false,
        outer: Rc::default(),
    };
    compile_function(&ast, &template).expect("übersetzbar")
}

/// `try` mit `catch`: Der Block hat einen Handler, der den Kontext
/// wiederherstellt und die Ausnahme in den Parameter legt.
#[test]
fn try_and_catch_have_a_handler() {
    assert_eq!(
        listing("try { a; } catch (e) { e; }"),
        [
            "LdaUndefined",
            "Star r1",
            "SaveContext r2",
            "LdaGlobal a",
            "Star r1",
            "Jump @26",
            "RestoreContext r2",
            "Star r3",
            "LdaUndefined",
            "Star r1",
            "Ldar r3",
            "Star r1",
            "Ldar r1",
            "Return",
            "Catch @5..@9 -> @15",
        ]
    );
}

/// `return` durch `finally`: Der Wert wartet im Register, und am Ende von
/// `finally` geht es je nach Weg weiter.
#[test]
fn return_goes_through_finally() {
    let code = first_function("function f() { try { return 1; } finally { 2; } }");
    assert_eq!(
        lines(&code),
        [
            "SaveContext r2",
            "LdaSmi 1",
            "Star r4",
            "LdaSmi 2",
            "Star r3",
            "Jump @34",
            "LdaSmi 0",
            "Star r3",
            "Jump @34",
            "RestoreContext r2",
            "Star r4",
            "LdaSmi 1",
            "Star r3",
            "LdaSmi 2",
            "LdaSmi 1",
            "TestEqualStrict r3",
            "JumpIfFalse @49",
            "Ldar r4",
            "Rethrow",
            "LdaSmi 2",
            "TestEqualStrict r3",
            "JumpIfFalse @62",
            "Ldar r4",
            "Return",
            "LdaUndefined",
            "Return",
            "Finally @2..@16 -> @26",
        ]
    );
}

#[test]
fn var_names_come_from_all_parts_of_try() {
    let script =
        compile("try { var a; } catch (e) { var b, e; } finally { var c; }").expect("übersetzbar");
    assert_eq!(names(&script.var_names), ["a", "b", "e", "c"]);
}

#[test]
fn arguments_and_rest_parameters_read_all_arguments() {
    // Das verknüpfte Objekt braucht die Parameter im Kontext.
    let code = first_function("function f(a) { return arguments; }");
    assert_eq!(
        lines(&code),
        [
            "PushContext 1",
            "Ldar r2",
            "StaContext 0 1",
            "CreateMappedArguments r3",
            "Star r4",
            "Ldar r4",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    assert_eq!(code.arguments_register(), Some(3));
    assert_eq!(code.parameter_map(), [Some(1)]);
    // Ein späterer Parameter gleichen Namens verdeckt den früheren.
    let code = first_function("function f(a, b, a) { return arguments; }");
    assert_eq!(code.parameter_map(), [None, Some(2), Some(1)]);
    let code = first_function("function f(a) { 'use strict'; return arguments; }");
    assert!(lines(&code).contains(&"CreateArguments r3".to_owned()));
    assert_eq!(code.parameter_map(), []);
    // Ohne den Bezeichner gibt es kein Objekt.
    let code = first_function("function f(a) { return a; }");
    assert_eq!(code.arguments_register(), None);
    let code = first_function("function f(a, ...r) { return r; }");
    assert_eq!(
        lines(&code),
        [
            "CreateRestParameter r3 1",
            "Star r4",
            "Ldar r4",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    assert_eq!(
        (code.parameter_count(), code.arguments_register()),
        (1, Some(3))
    );
}

#[test]
fn defaults_initialize_parameters_in_their_own_scope() {
    // Die Argumente liegen in r2 und r3, die Bindungen in r4 und r5. Bis
    // zur Initialisierung halten sie die Lücke; `a` ist im Vorgabewert von
    // `b` schon initialisiert.
    let code = first_function("function f(a, b = a) { return b; }");
    assert_eq!(
        lines(&code),
        [
            "LdaHole",
            "Star r4",
            "Star r5",
            "Ldar r2",
            "Star r4",
            "Ldar r3",
            "JumpIfNotUndefined @19",
            "Ldar r4",
            "Star r5",
            "Ldar r5",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    let code = first_function("function f(a = a) {}");
    assert!(lines(&code).contains(&"CheckHole r3 a".to_owned()));
    // Ein `var` mit dem Namen eines Parameters liegt im Bereich des Rumpfs
    // und beginnt mit dessen Wert.
    let code = first_function("function f(a = 1) { var a; return () => a; }");
    assert_eq!(
        lines(&code),
        [
            "PushContext 1",
            "LdaHole",
            "StaContext 0 1",
            "Ldar r2",
            "JumpIfNotUndefined @16",
            "LdaSmi 1",
            "StaContext 0 1",
            "PushContext 1",
            "LdaContext 1 1",
            "StaContext 0 1",
            "CreateClosure fn0 ",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
}

#[test]
fn for_with_let_copies_its_context() {
    let looped = listing("for (let i = 0; i < 2; i++) f = () => i;");
    let count = |op: &str| looped.iter().filter(|line| line.as_str() == op).count();
    assert_eq!(
        (
            count("PushContext 1"),
            count("CloneContext"),
            count("PopContext")
        ),
        (1, 2, 1)
    );
    // Ohne Closure bleibt `i` im Register.
    let plain = listing("for (let i = 0; i < 2; i++) i;");
    assert!(plain.iter().all(|line| !line.contains("Context")));
}

#[test]
fn captures_follow_names_into_inner_functions() {
    let ast = parse_script(&units(
        "function f(a, b) { var c; return () => a + this.x; } \
         function g() { return function () { return () => c + this; }; } d;",
    ))
    .expect("gültig");
    let captures = ast.captures();
    let name = |text: &str| ast.find_atom(&units(text)).expect("Name");
    let function = |index: usize| {
        let Some(&Node::FunctionDeclaration(id)) =
            ast.body().get(index).map(|&item| ast.node(item))
        else {
            panic!("keine Funktion");
        };
        captures.function(id).expect("Funktion").clone()
    };
    let f = function(0);
    assert_eq!(f.names, BTreeSet::from([name("a")]));
    assert!(f.this);
    // `this` der Pfeilfunktion gehört der Funktion ohne Namen, nicht `g`.
    let g = function(1);
    assert_eq!(g.names, BTreeSet::from([name("c")]));
    assert!(!g.this);
    let script = captures.script();
    assert_eq!(script.names, BTreeSet::from([name("a"), name("c")]));
    assert!(!script.this);
}

/// `new`: Konstruktor und Argumente liegen wie bei einem Aufruf, `this`
/// setzt erst [`Op::Construct`].
#[test]
fn new_puts_constructor_and_arguments_in_a_row() {
    assert_eq!(
        listing("new C(1);"),
        [
            "LdaGlobal C",
            "Star r2",
            "LdaSmi 1",
            "Star r4",
            "Construct r2 1",
            "Star r1",
            "Ldar r1",
            "Return"
        ]
    );
    assert_eq!(listing("new C;"), listing("new C();"));
}

/// `new.target` liegt in einem eigenen Register, für Pfeilfunktionen auch
/// im Kontext der Funktion.
#[test]
fn new_target_has_a_register_and_a_place_for_arrows() {
    let code = first_function("function f() { return new.target; }");
    assert_eq!(code.new_target_register(), Some(2));
    assert_eq!(
        lines(&code),
        ["Ldar r2", "Return", "LdaUndefined", "Return"]
    );
    assert_eq!(
        first_function("function f() {}").new_target_register(),
        None
    );
    let source = "function f() { return () => new.target; }";
    let ast = parse_script(&units(source)).expect("gültig");
    let Some(&Node::FunctionDeclaration(id)) = ast.body().first().map(|&item| ast.node(item))
    else {
        panic!("keine Funktion");
    };
    let template = FunctionTemplate {
        function: id,
        binds_name: false,
        outer: Rc::default(),
    };
    let code = compile_function(&ast, &template).expect("übersetzbar");
    assert_eq!(code.new_target_register(), Some(2));
    assert_eq!(
        lines(&code),
        [
            "PushContext 1",
            "Ldar r2",
            "StaContext 0 1",
            "CreateClosure fn0 ",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    let arrow = compile_function(&ast, &first_template(&code)).expect("übersetzbar");
    assert_eq!(arrow.new_target_register(), None);
    assert_eq!(lines(&arrow), ["LdaContext 0 1", "Return"]);
}

/// Die Endaufrufe im Code einer Funktion.
fn tail_calls(code: &Code) -> usize {
    lines(code)
        .iter()
        .filter(|line| line.starts_with("TailCall"))
        .count()
}

/// IsInTailPosition (Abschnitt 15.10.1): wie viele Aufrufe in der ersten
/// Funktion zu [`Op::TailCall`] werden.
#[test]
fn calls_in_tail_position_become_tail_calls() {
    let cases = [
        ("return g();", 1),
        ("return (g());", 1),
        ("return a, g();", 1),
        ("return a ? g() : h();", 2),
        ("return a && g();", 1),
        ("return a || g();", 1),
        ("return a ?? g();", 1),
        ("return g() ?? a;", 0),
        ("return g() + 1;", 0),
        ("return !g();", 0),
        ("return x = g();", 0),
        ("return g()();", 1),
        ("return g(h());", 1),
        ("return a.b();", 1),
        ("return a[b]();", 1),
        ("return a?.b();", 1),
        ("return a?.b().c;", 0),
        ("return new G();", 0),
        ("g();", 0),
        ("if (a) return g(); else return h();", 2),
        ("for (;;) { return g(); }", 1),
        ("for (let i = 0; ; ) return g(i);", 1),
        ("for (const i = 0; ; ) return g(i);", 1),
        ("while (a) return g();", 1),
        ("do return g(); while (a);", 1),
        ("l: { return g(); }", 1),
        ("switch (a) { case 1: return g(); default: return h(); }", 2),
        ("{ let x = 1; return g(x); }", 1),
        ("{ void 0; return g(); }", 1),
        ("try { return g(); } catch (e) {}", 0),
        ("try {} catch (e) { return g(); }", 1),
        ("try {} catch (e) { return g(); } finally {}", 0),
        ("try {} finally { return g(); }", 1),
        ("try {} catch (e) {} finally { return g(); }", 1),
        ("try { try {} catch (e) { return g(); } } finally {}", 0),
        ("try {} finally { try { return g(); } catch (e) {} }", 0),
        ("try { g(); } catch (e) {} return h();", 1),
    ];
    for (body, expected) in cases {
        let strict = format!("function f() {{ 'use strict'; {body} }}");
        assert_eq!(tail_calls(&first_function(&strict)), expected, "{strict}");
        let sloppy = format!("function f() {{ {body} }}");
        assert_eq!(tail_calls(&first_function(&sloppy)), 0, "{sloppy}");
    }
    // Der Rumpf einer Pfeilfunktion ohne Block steht in Endposition.
    for (source, expected) in [
        ("function f() { 'use strict'; return () => g(); }", 1),
        (
            "function f() { 'use strict'; return () => a ? g() : h(); }",
            2,
        ),
        ("function f() { 'use strict'; return () => { g(); }; }", 0),
        ("function f() { return () => g(); }", 0),
    ] {
        let ast = parse_script(&units(source)).expect("gültig");
        let Some(&Node::FunctionDeclaration(id)) = ast.body().first().map(|&item| ast.node(item))
        else {
            panic!("keine Funktion");
        };
        let template = FunctionTemplate {
            function: id,
            binds_name: false,
            outer: Rc::default(),
        };
        let code = compile_function(&ast, &template).expect("übersetzbar");
        assert_eq!(tail_calls(&code), 0, "{source}");
        let arrow = compile_function(&ast, &first_template(&code)).expect("übersetzbar");
        assert_eq!(tail_calls(&arrow), expected, "{source}");
    }
}

/// Im Rumpf von `for`-`of` steht kein Aufruf in Endposition, denn danach
/// kann der Iterator noch zu schließen sein (HasCallInTailPosition,
/// Abschnitt 15.10.2). Aufrufe mit Ausbreitung werden Endaufrufe wie
/// andere.
#[test]
fn for_of_bodies_hold_no_tail_calls() {
    for (body, expected) in [
        ("for (var x of a) return g(x);", 0),
        ("for (let x of a) { return g(x); }", 0),
        ("for (x of a) return g();", 0),
        ("for (;;) for (const x of a) return g();", 0),
        ("for (var x of a); return g();", 1),
        ("for (var x of a) break; return g();", 1),
        ("return g(...a);", 1),
        ("return a.b(...c, d);", 1),
        ("return a?.b(...c);", 1),
        ("return new G(...a);", 0),
        ("return g(...a) + 1;", 0),
    ] {
        let strict = format!("function f() {{ 'use strict'; {body} }}");
        assert_eq!(tail_calls(&first_function(&strict)), expected, "{strict}");
        let sloppy = format!("function f() {{ {body} }}");
        assert_eq!(tail_calls(&first_function(&sloppy)), 0, "{sloppy}");
    }
}

/// Die Befehle ohne Abstände und ohne Sprungziele.
fn shape(source: &str) -> Vec<String> {
    listing(source)
        .iter()
        .map(|line| {
            line.split(' ')
                .map(|part| if part.starts_with('@') { "@" } else { part })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// Ein Array-Muster: Jedes Ziel wird vor seinem Wert aufgelöst, eine
/// Lücke liest keinen Wert, der Rest sammelt in einer Schleife. Am Ende
/// schließt [`Op::IteratorClose`] den Iterator, bei einer Ausnahme der
/// Handler.
#[test]
fn array_patterns_close_their_iterator_on_every_way_out() {
    assert_eq!(
        shape("var [a, , ...b] = c;"),
        [
            "LdaGlobal c",
            "GetIterator r2",
            "ResolveGlobal a r5",
            "IteratorElement r2",
            "StaGlobal a r5",
            "IteratorSkip r2",
            "ResolveGlobal b r5",
            "CreateArray",
            "Star r6",
            "IteratorNext @ r2",
            "ArrayPush r6",
            "JumpLoop @",
            "Ldar r6",
            "StaGlobal b r5",
            "IteratorClose r2",
            "Jump @",
            "Star r5",
            "IteratorCloseOnThrow r2",
            "Ldar r5",
            "Rethrow",
            "Ldar r1",
            "Return",
            "Finally @ -> @",
        ]
    );
    // Ohne Elemente gibt es nichts zu schützen.
    assert_eq!(
        shape("var [] = c;"),
        [
            "LdaGlobal c",
            "GetIterator r2",
            "IteratorClose r2",
            "Ldar r1",
            "Return"
        ]
    );
}

/// Eine Zuweisung wertet erst die rechte Seite aus, dann das Muster, und
/// ergibt den Wert der rechten Seite. Die Basis einer Eigenschaft kommt
/// vor dem Wert aus dem Iterator.
#[test]
fn assignment_patterns_evaluate_their_targets_first() {
    assert_eq!(
        shape("[x.y] = z;"),
        [
            "LdaGlobal z",
            "Star r2",
            "GetIterator r3",
            "LdaGlobal x",
            "Star r6",
            "IteratorElement r3",
            "StaNamed r6 y",
            "IteratorClose r3",
            "Jump @",
            "Star r6",
            "IteratorCloseOnThrow r3",
            "Ldar r6",
            "Rethrow",
            "Ldar r2",
            "Star r1",
            "Ldar r1",
            "Return",
            "Finally @ -> @",
        ]
    );
}

/// Ein Objekt-Muster prüft den Wert zuerst. Mit Rest stehen die Schlüssel
/// davor in Registern hinter dem Wert, auch berechnete nach ToPropertyKey.
#[test]
fn object_patterns_keep_the_keys_for_their_rest() {
    assert_eq!(
        shape("let {a, [k]: b = 1, ...r} = o;"),
        [
            "LdaGlobal o",
            "RequireObjectCoercible",
            "Star r2",
            "LdaConstant a",
            "Star r3",
            "Ldar r2",
            "LdaNamed a",
            "InitGlobal a",
            "LdaGlobal k",
            "ToPropertyKey",
            "Star r4",
            "Ldar r4",
            "LdaKeyed r2",
            "JumpIfNotUndefined @",
            "LdaSmi 1",
            "InitGlobal b",
            "CreateRestObject r2 2",
            "InitGlobal r",
            "Ldar r1",
            "Return",
        ]
    );
    // Ohne Rest bleiben feste Namen in den Befehlen.
    assert_eq!(
        shape("let {a} = o;"),
        [
            "LdaGlobal o",
            "RequireObjectCoercible",
            "Star r2",
            "Ldar r2",
            "LdaNamed a",
            "InitGlobal a",
            "Ldar r1",
            "Return",
        ]
    );
}

/// Muster in Parametern: ohne Ausdrücke liegen ihre Namen wie andere
/// Bindungen in Registern und werden aus dem Register des Arguments
/// gebunden; mit Ausdrücken halten alle Namen die Lücke, bis ihr Parameter
/// an der Reihe ist.
#[test]
fn parameters_with_patterns_bind_from_their_argument() {
    let code = first_function("function f({a}, b) { return a + b; }");
    assert_eq!(code.parameter_count(), 2);
    let bound = lines(&code);
    assert_eq!(
        bound.get(..5),
        Some(
            &[
                "Ldar r2".to_owned(),
                "RequireObjectCoercible".to_owned(),
                "Star r5".to_owned(),
                "Ldar r5".to_owned(),
                "LdaNamed a".to_owned(),
            ][..]
        )
    );
    let code = first_function("function f([a = b], b) { return a; }");
    let checked = lines(&code);
    assert_eq!(checked.first().map(String::as_str), Some("LdaHole"));
    assert!(checked.iter().any(|line| line.starts_with("CheckHole")));
}

/// Die tiefsten Muster, die der Parser annimmt, übersetzen sich auf einem
/// Stapel von 1 MiB (RS-8): in Deklarationen, Zuweisungen, `catch` und
/// Parametern.
#[test]
fn deep_patterns_need_no_deep_stack() {
    let forms = [
        ("let ", "[", "a", "]", " = 1;"),
        ("var ", "{a: ", "b", "}", " = 1;"),
        ("", "[", "a", "] = 1", ";"),
        ("(", "{a: ", "b", "}", " = 1);"),
        ("try {} catch (", "[{a: ", "b", "}]", ") {}"),
        ("function f(", "[", "a", "]", ") {}"),
        ("function f(", "{a: ", "b = 1", "}", ") {}"),
    ];
    let thread = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            for (head, open, middle, close, tail) in forms {
                let mut depth = 300;
                let ast = loop {
                    let text = format!(
                        "{head}{}{middle}{}{tail}",
                        open.repeat(depth),
                        close.repeat(depth)
                    );
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
                if let Some(&Node::FunctionDeclaration(id)) =
                    ast.body().first().map(|&item| ast.node(item))
                {
                    let template = FunctionTemplate {
                        function: id,
                        binds_name: false,
                        outer: Rc::default(),
                    };
                    assert!(
                        compile_function(&ast, &template).is_ok(),
                        "Parameter {open}…{close} bei Tiefe {depth}"
                    );
                }
            }
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}

/// Die Befehle einer Funktion ohne Abstände und ohne Sprungziele.
fn function_shape(code: &Code) -> Vec<String> {
    lines(code)
        .iter()
        .map(|line| {
            line.split(' ')
                .map(|part| if part.starts_with('@') { "@" } else { part })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// Eine Generatorfunktion legt ihren Generator nach den Parametern und
/// Funktionsdeklarationen an und kehrt mit ihm zurück. Nach `yield` führt
/// der Weg für `return` aus der Funktion, `yield*` wiederholt einen Durchlauf
/// mit dem Iterator Record in drei Registern.
#[test]
fn generators_return_their_object_and_resume_after_yield() {
    let code = first_function("function* g(a) { var x = yield a; return x; }");
    assert_eq!(code.generator_register(), Some(3));
    assert_eq!(
        function_shape(&code),
        [
            "CreateGenerator",
            "Ldar r2",
            "Yield",
            "ResumeGenerator @",
            "Return",
            "Star r4",
            "Ldar r4",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    let code = first_function("function* g() { yield* a; }");
    assert_eq!(
        function_shape(&code),
        [
            "CreateGenerator",
            "LdaGlobal a",
            "GetIterator r3",
            "LdaUndefined",
            "YieldStar @ r3",
            "YieldResult",
            "JumpLoop @",
            "ResumeGenerator @",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    // FunctionDeclarationInstantiation kommt vor dem Generator.
    let code = first_function("function* g(a = f()) { function h() {} yield h; }");
    let shape = function_shape(&code);
    let create = shape
        .iter()
        .position(|line| line == "CreateGenerator")
        .expect("CreateGenerator");
    let call = shape
        .iter()
        .position(|line| line.starts_with("Call "))
        .expect("Aufruf");
    let closure = shape
        .iter()
        .position(|line| line.starts_with("CreateClosure"))
        .expect("Funktion");
    assert!(call < create && closure < create, "{shape:?}");
    assert_eq!(first_function("function f() {}").generator_register(), None);
}

/// Ein `return` aus einem `yield` geht durch `finally`.
#[test]
fn returns_after_yield_pass_through_finally() {
    let code = first_function("function* g() { try { yield 1; } finally { f(); } }");
    let shape = function_shape(&code);
    let resume = shape
        .iter()
        .position(|line| line == "ResumeGenerator @")
        .expect("ResumeGenerator");
    assert!(
        shape
            .get(resume.saturating_add(1))
            .is_some_and(|line| line.starts_with("Star ")),
        "{shape:?}"
    );
}

/// IsInTailPosition (Abschnitt 15.10.1): Im Rumpf eines Generators steht
/// kein Aufruf in Endposition, auch nicht im strikten Modus.
#[test]
fn generators_hold_no_tail_calls() {
    for body in ["return g();", "return a ? g() : h();", "return g(...a);"] {
        let strict = format!("function* f() {{ 'use strict'; {body} }}");
        assert_eq!(tail_calls(&first_function(&strict)), 0, "{strict}");
    }
    let method = "var o = { *m() { 'use strict'; return g(); } };";
    let ast = parse_script(&units(method)).expect("gültig");
    let script = compile_script(&ast).expect("übersetzbar");
    let code = compile_function(&ast, &first_template(&script.code)).expect("übersetzbar");
    assert_eq!(tail_calls(&code), 0);
    assert!(code.generator_register().is_some());
}

/// Eine async-Funktion beginnt mit [`Op::BeginAsync`], noch vor den
/// Parametern. Nach `await` geht es mit [`Op::ResumeAsync`] weiter, vor jedem
/// `return` steht [`Op::ResolveAsync`], und ein Handler um den ganzen Code
/// führt zu [`Op::RejectAsync`].
#[test]
fn async_functions_begin_with_their_capability_and_reject_from_one_handler() {
    let code = first_function("async function f(a) { var x = await a; return x; }");
    assert_eq!(code.async_state_register(), Some(3));
    assert_eq!(code.generator_register(), None);
    assert_eq!(
        function_shape(&code),
        [
            "BeginAsync",
            "Ldar r2",
            "Await",
            "ResumeAsync",
            "Star r4",
            "Ldar r4",
            "ResolveAsync",
            "Return",
            "LdaUndefined",
            "ResolveAsync",
            "Return",
            "RejectAsync",
            "Return",
            "Catch @ -> @"
        ]
    );
    let handler = code.handlers().last().copied().expect("Handler");
    assert_eq!(handler.start, 1, "hinter BeginAsync");
    assert_eq!(
        code.decode(handler.target)
            .map(|instruction| instruction.op),
        Some(Op::RejectAsync)
    );
    // Die Parameter liegen im Bereich des Handlers.
    let code = first_function("async function f(a = g()) {}");
    let shape = function_shape(&code);
    assert_eq!(shape.first().map(String::as_str), Some("BeginAsync"));
    assert!(
        shape.iter().any(|line| line.starts_with("Call ")),
        "{shape:?}"
    );
    assert_eq!(
        first_function("function f() {}").async_state_register(),
        None
    );
}

/// IsInTailPosition (Abschnitt 15.10.1): In async-Funktionen steht kein
/// Aufruf in Endposition, auch nicht im strikten Modus und nicht im Rumpf
/// einer async-Pfeilfunktion ohne Block.
#[test]
fn async_functions_hold_no_tail_calls() {
    for body in ["return g();", "return a ? g() : h();", "return g(...a);"] {
        let strict = format!("async function f() {{ 'use strict'; {body} }}");
        assert_eq!(tail_calls(&first_function(&strict)), 0, "{strict}");
    }
    let arrow = "'use strict'; var f = async () => g();";
    let ast = parse_script(&units(arrow)).expect("gültig");
    let script = compile_script(&ast).expect("übersetzbar");
    let code = compile_function(&ast, &first_template(&script.code)).expect("übersetzbar");
    assert_eq!(tail_calls(&code), 0);
    assert!(code.async_state_register().is_some());
}

/// Eine Klasse: ihr Name als Konstante im eigenen Bereich, Oberklasse und
/// Name in zwei Registern für [`Op::CreateClass`], dann die Methoden mit
/// dem Namen vor dem Funktionsobjekt. Eine Methode mit `super.x` bekommt
/// [[HomeObject]].
#[test]
fn classes_create_their_constructor_then_their_methods() {
    assert_eq!(
        listing("class A { m() { return super.x; } }"),
        [
            "LdaHole",
            "Star r2",
            "LdaHole",
            "Star r3",
            "LdaConstant A",
            "Star r4",
            "CreateClass default r3",
            "Star r4",
            "LdaConstant m",
            "Star r5",
            "CreateClosure fn0 m",
            "SetHomeObject r3",
            "DefineMethod r3 r5",
            "Ldar r4",
            "Star r2",
            "Ldar r4",
            "InitGlobal A",
            "Ldar r1",
            "Return"
        ]
    );
    // Die Referenz: erst `this`, dann GetSuperBase mit dem Funktionsobjekt.
    let ast = parse_script(&units("class A { m() { return super.x; } }")).expect("gültig");
    let script = compile_script(&ast).expect("übersetzbar");
    let method = compile_function(&ast, &first_template(&script.code)).expect("übersetzbar");
    assert_eq!(
        lines(&method),
        [
            "Ldar r0",
            "Star r3",
            "Ldar r1",
            "GetSuperBase",
            "Star r2",
            "LdaSuperNamed r2 x",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
}

/// Felder und private Namen: Die Namen entstehen nach der Oberklasse in
/// Plätzen des Bereichs der Klasse, ein Feld kommt mit seinem Namen und dem
/// Initialisierer an den Konstruktor. Der Initialisierer ist eine Funktion
/// ohne Parameter, eine anonyme Funktion darin heißt wie das Feld. Der
/// Konstruktor ohne Oberklasse beginnt mit InitializeInstanceElements.
#[test]
fn fields_and_private_names_live_in_the_class_scope() {
    assert_eq!(
        listing("class A { #x = 1; }"),
        [
            "PushContext 1",
            "LdaHole",
            "Star r2",
            "LdaHole",
            "Star r3",
            "CreatePrivateName \"#x\"",
            "StaContext 0 1",
            "LdaConstant A",
            "Star r4",
            "CreateClass default r3",
            "Star r4",
            "LdaContext 0 1",
            "Star r5",
            "CreateClosureKeyed fn0 r5",
            "AddField r4 r5",
            "Ldar r4",
            "Star r2",
            "Ldar r4",
            "PopContext",
            "InitGlobal A",
            "Ldar r1",
            "Return"
        ]
    );
    let compiled = |source: &str| {
        let ast = parse_script(&units(source)).expect("gültig");
        let script = compile_script(&ast).expect("übersetzbar");
        compile_function(&ast, &first_template(&script.code)).expect("übersetzbar")
    };
    let initializer = compiled("class A { #x = 1; }");
    assert_eq!(initializer.parameter_count(), 0);
    assert_eq!(lines(&initializer), ["LdaSmi 1", "Return"]);
    assert_eq!(
        lines(&compiled("class A { f = function () {}; }")),
        [
            "Ldar r1",
            "LdaNamed name",
            "Star r2",
            "CreateClosureKeyed fn0 r2",
            "Return"
        ]
    );
    assert_eq!(
        lines(&compiled("class A { #x; m() { return this.#x; } }")),
        [
            "Ldar r0",
            "Star r2",
            "LdaContext 0 1",
            "Star r3",
            "Ldar r2",
            "LdaPrivate r3",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    assert_eq!(
        lines(&compiled(
            "class A { #x; static has(o) { return #x in o; } }"
        )),
        [
            "LdaContext 0 1",
            "Star r3",
            "Ldar r2",
            "TestPrivateIn r3",
            "Return",
            "LdaUndefined",
            "Return"
        ]
    );
    let constructor = compiled("class A { x = 1; constructor(a) {} }");
    assert_eq!(constructor.register_count(), 8);
    assert_eq!(
        lines(&constructor),
        [
            "Mov r1 r3",
            "Mov r0 r6",
            "InitializePrivateMethods r3 r6",
            "LdaSmi 0",
            "Star r4",
            "NextField @37 r3",
            "JumpIfNullish @32",
            "Call r5 0",
            "DefineField r6 r7",
            "JumpLoop @13",
            "LdaUndefined",
            "Return"
        ]
    );
}

/// Eine Klasse in nicht striktem Code ist ein strikter Bereich von ihrem
/// ersten Befehl bis zum Konstruktor im Akkumulator, ohne die Bindung
/// ihres Namens außen; eine innere Klasse liegt im Bereich der äußeren.
/// Strikter Code hat keine Bereiche.
#[test]
fn classes_in_sloppy_code_are_strict_ranges() {
    let script =
        compile("x = 1; class A extends B { [(class {}, 'm')]() {} } y = 2;").expect("übersetzbar");
    let code = &script.code;
    assert!(!code.is_strict());
    let ranges: Vec<(usize, usize)> = code.strict_ranges().collect();
    assert_eq!(ranges.len(), 1);
    let &(start, end) = ranges.first().expect("ein Bereich");
    let op = |offset: usize| code.decode(offset).map(|instruction| instruction.op);
    // Die Lücke für den Namen im Bereich der Klasse.
    assert_eq!(op(start), Some(Op::LdaHole));
    assert_eq!(op(end), Some(Op::InitGlobal));
    let mut offset = 0;
    let mut last = None;
    while offset < end {
        let instruction = code.decode(offset).expect("Befehl");
        assert_eq!(code.is_strict_at(offset), offset >= start, "@{offset}");
        last = Some(instruction.op);
        offset = instruction.end;
    }
    assert_eq!(offset, end, "der Bereich endet an einem Befehl");
    // Der Konstruktor im Akkumulator.
    assert_eq!(last, Some(Op::Ldar));
    assert!(!code.is_strict_at(end));

    let strict = compile("'use strict'; class A extends B {}").expect("übersetzbar");
    assert!(strict.code.is_strict());
    assert_eq!(strict.code.strict_ranges().count(), 0);

    let ast = parse_script(&units("var f = function () { return class {}; };")).expect("gültig");
    let script = compile_script(&ast).expect("übersetzbar");
    assert_eq!(script.code.strict_ranges().count(), 0);
    let function = compile_function(&ast, &first_template(&script.code)).expect("übersetzbar");
    assert!(!function.is_strict());
    assert_eq!(function.strict_ranges().count(), 1);
}

/// Im Konstruktor einer abgeleiteten Klasse: new.target und
/// GetSuperConstructor vor den Argumenten, nach `super()` BindThisValue und
/// InitializeInstanceElements, und jedes `this` prüft die Lücke.
#[test]
fn derived_constructors_bind_this_after_super() {
    let source = "class B extends A { constructor() { super(1); this.x; } }";
    let ast = parse_script(&units(source)).expect("gültig");
    let script = compile_script(&ast).expect("übersetzbar");
    let code = compile_function(&ast, &first_template(&script.code)).expect("übersetzbar");
    assert!(code.is_derived_constructor());
    assert!(code.is_strict());
    assert_eq!(code.new_target_register(), Some(2));
    assert_eq!(
        lines(&code),
        [
            "Ldar r2",
            "Star r3",
            "Ldar r1",
            "GetSuperConstructor",
            "Star r4",
            "LdaSmi 1",
            "Star r6",
            "SuperCall r4 1",
            "Star r3",
            "Ldar r0",
            "ThrowIfNotHole",
            "Ldar r3",
            "Star r0",
            "Star r6",
            "Ldar r1",
            "Star r3",
            "InitializePrivateMethods r3 r6",
            "LdaSmi 0",
            "Star r4",
            "NextField @62 r3",
            "JumpIfNullish @57",
            "Call r5 0",
            "DefineField r6 r7",
            "JumpLoop @38",
            "Ldar r6",
            "CheckHole r0 this",
            "Ldar r0",
            "LdaNamed x",
            "LdaUndefined",
            "Return"
        ]
    );
    // Der Standardkonstruktor reicht die Argumente als Array weiter.
    let derived = compile_default_constructor(true).expect("übersetzbar");
    assert!(derived.is_derived_constructor() && derived.is_strict());
    assert_eq!(
        (
            derived.register_count(),
            derived.arguments_register(),
            derived.new_target_register()
        ),
        (8, Some(2), Some(3))
    );
    assert_eq!(
        lines(&derived),
        [
            "Ldar r1",
            "GetSuperConstructor",
            "Star r5",
            "Ldar r3",
            "Star r4",
            "CreateRestParameter r2 0",
            "Star r7",
            "SuperCallSpread r5",
            "Star r6",
            "Mov r1 r3",
            "InitializePrivateMethods r3 r6",
            "LdaSmi 0",
            "Star r4",
            "NextField @52 r3",
            "JumpIfNullish @47",
            "Call r5 0",
            "DefineField r6 r7",
            "JumpLoop @28",
            "Ldar r6",
            "Return"
        ]
    );
    let base = compile_default_constructor(false).expect("übersetzbar");
    assert!(!base.is_derived_constructor());
    assert_eq!(base.register_count(), 7);
    assert_eq!(
        lines(&base),
        [
            "Mov r1 r2",
            "Mov r0 r5",
            "InitializePrivateMethods r2 r5",
            "LdaSmi 0",
            "Star r3",
            "NextField @37 r2",
            "JumpIfNullish @32",
            "Call r4 0",
            "DefineField r5 r6",
            "JumpLoop @13",
            "LdaUndefined",
            "Return"
        ]
    );
}
