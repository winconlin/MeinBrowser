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
        ("function* f() {}", 0),
        ("class A {}", 0),
        ("x = {*a() {}};", 5),
        ("x = {async a() {}};", 5),
        ("x = {1n: 1};", 5),
        ("x = [...a];", 5),
        ("new a();", 0),
        ("a?.b(...c);", 5),
        ("try {} catch {}", 0),
        ("for (var k in o);", 0),
        ("for (let k of o);", 0),
        ("f`x`;", 0),
        ("[a] = b;", 0),
        ("let {a} = b;", 4),
        ("1n;", 0),
        ("/a/;", 0),
        ("async x => x;", 0),
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
