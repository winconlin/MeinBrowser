//! Tests des Parsers: der Aufbau des Baums, Lagen, Fehlerarten und die
//! Grenze der Verschachtelung. Was gültig ist und was nicht, prüft im
//! Ganzen `test262.rs`.

use super::*;
use crate::ast::{FunctionKind, PropertyKind};

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn script(text: &str) -> Ast {
    parse_script(&utf16(text)).unwrap_or_else(|error| panic!("{text:?}: {error:?}"))
}

fn module(text: &str) -> Ast {
    parse_module(&utf16(text)).unwrap_or_else(|error| panic!("{text:?}: {error:?}"))
}

fn script_error(text: &str) -> SyntaxError {
    match parse_script(&utf16(text)) {
        Ok(_) => panic!("{text:?} ergab keinen Fehler"),
        Err(error) => error,
    }
}

fn module_error(text: &str) -> ErrorKind {
    match parse_module(&utf16(text)) {
        Ok(_) => panic!("{text:?} ergab keinen Fehler"),
        Err(error) => error.kind,
    }
}

fn name(ast: &Ast, atom: Atom) -> String {
    String::from_utf16(ast.atom(atom)).expect("Namen in den Tests sind gültiges UTF-16")
}

/// Der Ausdruck der einzigen Anweisung.
fn expression(ast: &Ast) -> NodeId {
    match ast.body() {
        [statement] => match *ast.node(*statement) {
            Node::Expression(expression) => expression,
            ref other => panic!("keine Ausdrucksanweisung: {other:?}"),
        },
        other => panic!("nicht genau eine Anweisung: {other:?}"),
    }
}

/// Eine kurze Schreibweise für kleine Bäume.
fn show(ast: &Ast, node: NodeId) -> String {
    let list = |list: List| {
        ast.list(list)
            .iter()
            .map(|item| show(ast, *item))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match *ast.node(node) {
        Node::Identifier(atom) | Node::BindingIdentifier(atom) | Node::PropertyName(atom) => {
            name(ast, atom)
        }
        Node::Number(value) => format!("{value}"),
        Node::String(atom) => format!("{:?}", name(ast, atom)),
        Node::RegExp { body, flags } => format!("/{}/{}", name(ast, body), name(ast, flags)),
        Node::Binary {
            operator,
            left,
            right,
        } => format!("{operator:?}({}, {})", show(ast, left), show(ast, right)),
        Node::Unary { operator, argument } => format!("{operator:?}({})", show(ast, argument)),
        Node::Assign {
            operator,
            target,
            value,
        } => format!("{operator:?}({}, {})", show(ast, target), show(ast, value)),
        Node::Conditional {
            test,
            consequent,
            alternate,
        } => format!(
            "If({}, {}, {})",
            show(ast, test),
            show(ast, consequent),
            show(ast, alternate)
        ),
        Node::Paren(inner) => format!("Paren({})", show(ast, inner)),
        Node::Sequence(items) => format!("Sequence({})", list(items)),
        Node::Call {
            callee, arguments, ..
        } => format!("Call({}; {})", show(ast, callee), list(arguments)),
        Node::Member {
            object, property, ..
        } => format!("{}.{}", show(ast, object), name(ast, property)),
        Node::Array(items) => format!("Array[{}]", list(items)),
        Node::Object(items) => format!("Object{{{}}}", list(items)),
        Node::ArrayPattern(items) => format!("ArrayPattern[{}]", list(items)),
        Node::ObjectPattern(items) => format!("ObjectPattern{{{}}}", list(items)),
        Node::PatternProperty { key, value, .. } => {
            format!("{}: {}", show(ast, key), show(ast, value))
        }
        Node::WithDefault { target, default } => {
            format!("{} = {}", show(ast, target), show(ast, default))
        }
        Node::Rest(target) => format!("...{}", show(ast, target)),
        ref other => format!("{other:?}"),
    }
}

fn show_expression(text: &str) -> String {
    let ast = script(text);
    show(&ast, expression(&ast))
}

#[test]
fn operator_precedence_and_associativity() {
    let cases = [
        ("a + b * c", "Add(a, Multiply(b, c))"),
        ("a - b - c", "Subtract(Subtract(a, b), c)"),
        ("a ** b ** c", "Exponent(a, Exponent(b, c))"),
        ("a * b ** c", "Multiply(a, Exponent(b, c))"),
        ("a || b && c", "LogicalOr(a, LogicalAnd(b, c))"),
        ("a ?? b ?? c", "Coalesce(Coalesce(a, b), c)"),
        ("a < b == c in d", "Equal(Less(a, b), In(c, d))"),
        (
            "a | b ^ c & d",
            "BitwiseOr(a, BitwiseXor(b, BitwiseAnd(c, d)))",
        ),
        ("a << b + c", "ShiftLeft(a, Add(b, c))"),
        ("-a * b", "Multiply(Minus(a), b)"),
        ("(-a) ** b", "Exponent(Paren(Minus(a)), b)"),
        ("a = b += c", "Assign(a, Add(b, c))"),
        ("a ? b : c ? d : e", "If(a, b, If(c, d, e))"),
        ("a, b = c", "Sequence(a, Assign(b, c))"),
        ("typeof a.b(c)", "Typeof(Call(a.b; c))"),
    ];
    for (text, expected) in cases {
        assert_eq!(show_expression(text), expected, "{text:?}");
    }
    assert_eq!(script_error("-a ** b").kind, ErrorKind::UnaryBeforeExponent);
    assert_eq!(script_error("a ?? b || c").kind, ErrorKind::MixedCoalesce);
    assert_eq!(script_error("a && b ?? c").kind, ErrorKind::MixedCoalesce);
    assert_eq!(
        show_expression("(a ?? b) || c"),
        "LogicalOr(Paren(Coalesce(a, b)), c)"
    );
}

#[test]
fn slash_is_division_or_regular_expression_by_position() {
    assert_eq!(show_expression("a / b / g"), "Divide(Divide(a, b), g)");
    assert_eq!(show_expression("a = /b/g"), "Assign(a, /b/g)");
    assert_eq!(show_expression("a\n/b/g"), "Divide(Divide(a, b), g)");
    assert_eq!(show_expression("(a) / 2"), "Divide(Paren(a), 2)");
    // Nach einem Block beginnt eine neue Anweisung.
    let ast = script("{}\n/b/g");
    let [_, second] = ast.body() else {
        panic!("zwei Anweisungen erwartet");
    };
    let Node::Expression(regexp) = *ast.node(*second) else {
        panic!("Ausdruck erwartet");
    };
    assert_eq!(show(&ast, regexp), "/b/g");
    assert_eq!(
        script_error("a = /b/gg").kind,
        ErrorKind::InvalidRegExpFlags
    );
    assert_eq!(script_error("/b/uv").kind, ErrorKind::InvalidRegExpFlags);
}

#[test]
fn regular_expression_patterns_are_checked() {
    use crate::regexp::ErrorKind as RegExpError;
    let error = script_error("x = /a(b/");
    assert_eq!(
        error.kind,
        ErrorKind::RegExp(RegExpError::UnterminatedGroup)
    );
    assert_eq!(error.position, 6);
    assert_eq!(
        script_error("/{1}/").kind,
        ErrorKind::RegExp(RegExpError::NothingToRepeat)
    );
    assert_eq!(
        script_error("/]/; /]/u").kind,
        ErrorKind::RegExp(RegExpError::UnexpectedCharacter)
    );
    assert_eq!(
        show_expression("/(?<a>.)\\k<a>[\\q{bc}]/v"),
        "/(?<a>.)\\k<a>[\\q{bc}]/v"
    );
}

#[test]
fn automatic_semicolon_insertion() {
    assert_eq!(script("a\nb").body().len(), 2);
    assert_eq!(script("a\n++b").body().len(), 2);
    assert_eq!(show_expression("a\n(b)"), "Call(a; b)");
    assert_eq!(script("do ; while (a) b").body().len(), 2);
    let ast = script("function f() { return\na }");
    let Node::FunctionDeclaration(function) = *ast.node(ast.body()[0]) else {
        panic!("Funktion erwartet");
    };
    let body = ast.function(function).expect("Funktion").body;
    assert!(matches!(ast.node(ast.list(body)[0]), Node::Return(None)));
    assert_eq!(ast.list(body).len(), 2);
    assert_eq!(script_error("a b").kind, ErrorKind::UnexpectedToken);
    assert_eq!(script_error("throw\na").kind, ErrorKind::UnexpectedToken);
}

#[test]
fn arrow_functions_reinterpret_their_head() {
    let ast = script("(a, [b], {c} = {}, ...d) => a");
    let Node::Function(function) = *ast.node(expression(&ast)) else {
        panic!("Funktion erwartet");
    };
    let function = ast.function(function).expect("Funktion");
    assert_eq!(function.kind, FunctionKind::Arrow);
    assert!(function.expression_body && !function.simple_parameters && !function.is_async);
    let parameters: Vec<String> = ast
        .list(function.parameters)
        .iter()
        .map(|parameter| show(&ast, *parameter))
        .collect();
    assert_eq!(
        parameters,
        [
            "a",
            "ArrayPattern[b]",
            "ObjectPattern{c: c} = Object{}",
            "...d"
        ]
    );
    let ast = script("async x => { await x }");
    let Node::Function(function) = *ast.node(expression(&ast)) else {
        panic!("Funktion erwartet");
    };
    let function = ast.function(function).expect("Funktion");
    assert!(function.is_async && function.simple_parameters && !function.expression_body);
    assert!(script("async\nx => x").body().len() == 2);
    for (text, kind) in [
        ("(a, a) => 1", ErrorKind::DuplicateParameter),
        ("(a + b) => 1", ErrorKind::InvalidCoverGrammar),
        ("((a)) => 1", ErrorKind::InvalidCoverGrammar),
        ("(...a, b) => 1", ErrorKind::UnexpectedToken),
        ("async (...a, b) => 1", ErrorKind::InvalidCoverGrammar),
        ("async (await) => 1", ErrorKind::ReservedWord),
        (
            "(a = 1) => { 'use strict' }",
            ErrorKind::UseStrictWithComplexParameters,
        ),
        ("() => { super.x }", ErrorKind::InvalidSuper),
        ("(...a)", ErrorKind::InvalidCoverGrammar),
        ("()", ErrorKind::InvalidCoverGrammar),
    ] {
        assert_eq!(script_error(text).kind, kind, "{text:?}");
    }
}

#[test]
fn destructuring_assignment_reinterprets_literals() {
    assert_eq!(
        show_expression("[a, b.c, [d] = e, ...f] = g"),
        "Assign(ArrayPattern[a, b.c, ArrayPattern[d] = e, ...f], g)"
    );
    assert_eq!(
        show_expression("({a, b: [c], d = 1} = e)"),
        "Paren(Assign(ObjectPattern{a: a, b: ArrayPattern[c], d: d = 1}, e))"
    );
    for (text, kind) in [
        ("[a + b] = c", ErrorKind::InvalidAssignmentTarget),
        ("[...a, b] = c", ErrorKind::InvalidAssignmentTarget),
        ("({a = 1})", ErrorKind::InvalidCoverGrammar),
        ("({a = 1}).b = c", ErrorKind::InvalidCoverGrammar),
        ("a + b = c", ErrorKind::InvalidAssignmentTarget),
        ("a() &&= b", ErrorKind::InvalidAssignmentTarget),
        ("'use strict'; [eval] = a", ErrorKind::StrictEvalOrArguments),
    ] {
        assert_eq!(script_error(text).kind, kind, "{text:?}");
    }
    // Aufrufe als Ziel sind außerhalb des strikten Modus erst zur Laufzeit
    // ein Fehler (Abschnitt 13.15.1 für Webbrowser).
    script("f() = 1; f()++; for (f() in a);");
    assert_eq!(
        script_error("'use strict'; f() = 1").kind,
        ErrorKind::InvalidAssignmentTarget
    );
}

#[test]
fn object_literals() {
    let ast = script("x = {a, b: 1, [c]: 2, d() {}, get e() {}, set e(v) {}, async *f() {}, ...g}");
    let Node::Assign { value, .. } = *ast.node(expression(&ast)) else {
        panic!("Zuweisung erwartet");
    };
    let Node::Object(properties) = *ast.node(value) else {
        panic!("Objekt erwartet");
    };
    let kinds: Vec<Option<PropertyKind>> = ast
        .list(properties)
        .iter()
        .map(|property| match *ast.node(*property) {
            Node::Property { kind, .. } => Some(kind),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [
            Some(PropertyKind::Shorthand),
            Some(PropertyKind::Init),
            Some(PropertyKind::Init),
            Some(PropertyKind::Method),
            Some(PropertyKind::Get),
            Some(PropertyKind::Set),
            Some(PropertyKind::Method),
            None,
        ]
    );
    assert_eq!(
        script_error("x = {__proto__: 1, '__proto__': 2}").kind,
        ErrorKind::DuplicateProto
    );
    script("x = {__proto__: 1, __proto__() {}, ['__proto__']: 2}");
    script("({__proto__: a, __proto__: b} = x)");
    assert_eq!(
        script_error("x = {get a(b) {}}").kind,
        ErrorKind::UnexpectedToken
    );
    assert_eq!(
        script_error("x = {set a(...b) {}}").kind,
        ErrorKind::UnexpectedToken
    );
    assert_eq!(
        script_error("x = {a() { super() }}").kind,
        ErrorKind::InvalidSuper
    );
}

#[test]
fn strict_mode_and_directives() {
    assert!(script("'use strict'; a").is_strict());
    assert!(!script("a; 'use strict'").is_strict());
    assert!(!script("'use\\x20strict'; with (a) b").is_strict());
    assert!(module("a").is_strict());
    script("function f() { 'use strict' } with (a) b");
    for (text, kind) in [
        ("'use strict'; with (a) b", ErrorKind::StrictWith),
        ("'use strict'; 010", ErrorKind::StrictLegacyLiteral),
        ("'\\07'; 'use strict'", ErrorKind::StrictLegacyLiteral),
        ("'use strict'; delete a", ErrorKind::InvalidDelete),
        ("'use strict'; var let", ErrorKind::ReservedWord),
        (
            "function eval() { 'use strict' }",
            ErrorKind::StrictEvalOrArguments,
        ),
        (
            "function static() { 'use strict' }",
            ErrorKind::ReservedWord,
        ),
        (
            "function f(a, a) { 'use strict' }",
            ErrorKind::DuplicateParameter,
        ),
        (
            "function f(yield) { 'use strict' }",
            ErrorKind::ReservedWord,
        ),
        (
            "function f(a = 1) { 'use strict' }",
            ErrorKind::UseStrictWithComplexParameters,
        ),
    ] {
        assert_eq!(script_error(text).kind, kind, "{text:?}");
    }
    // Außerhalb des strikten Modus erlaubt.
    script("with (a) b; 010; delete a; var let; function f(a, a) {}");
}

#[test]
fn declarations_in_scopes() {
    for text in [
        "let a; let a;",
        "let a; var a;",
        "var a; let a;",
        "const a = 1; function a() {}",
        "{ let a; { var a; } }",
        "function f(a) { let a; }",
        "try {} catch (e) { let e; }",
        "try {} catch ([e]) { var e; }",
        "'use strict'; { function f() {} function f() {} }",
        "{ function* f() {} function f() {} }",
        "switch (a) { case 1: let b; case 2: let b; }",
        "for (let a;;) { var a; }",
        "label: { class a {} var a; }",
    ] {
        assert_eq!(
            script_error(text).kind,
            ErrorKind::Redeclaration,
            "{text:?}"
        );
    }
    script(
        "var a; var a; function a() {} { function f() {} function f() {} } \
         try {} catch (e) { var e; } function g(a) { var a; function a() {} } \
         let b; { let b; }",
    );
    assert_eq!(script_error("let let = 1").kind, ErrorKind::LetBinding);
    assert_eq!(script_error("const a;").kind, ErrorKind::MissingInitializer);
    assert_eq!(script_error("let [a];").kind, ErrorKind::MissingInitializer);
    assert_eq!(script_error("using a = b;").kind, ErrorKind::InvalidUsing);
    script("{ using a = b; } for (using a of b); for (using of = null;;) break; for (using of b);");
}

#[test]
fn labels_and_jumps() {
    for (text, kind) in [
        ("a: a: ;", ErrorKind::DuplicateLabel),
        ("break;", ErrorKind::InvalidJump),
        ("continue;", ErrorKind::InvalidJump),
        ("a: { continue a; }", ErrorKind::InvalidJump),
        (
            "a: while (1) { function f() { break a; } }",
            ErrorKind::UndefinedLabel,
        ),
        ("while (1) { break b; }", ErrorKind::UndefinedLabel),
        ("switch (a) { case 1: continue; }", ErrorKind::InvalidJump),
        (
            "'use strict'; a: function f() {}",
            ErrorKind::InvalidLabelledFunction,
        ),
        (
            "if (a) b: function f() {}",
            ErrorKind::InvalidLabelledFunction,
        ),
        (
            "while (a) function f() {}",
            ErrorKind::InvalidFunctionDeclaration,
        ),
        ("return", ErrorKind::InvalidReturn),
    ] {
        assert_eq!(script_error(text).kind, kind, "{text:?}");
    }
    script("a: b: while (1) { continue a; } a: { break a; } switch (a) { case 1: break; }");
    script("a: function f() {} if (a) function g() {} else function h() {}");
}

#[test]
fn classes() {
    let ast = script(
        "class C extends D { constructor() { super(); } static #a = 1; get #b() {} set #b(v) {} static { this.#a; } }",
    );
    let Node::ClassDeclaration(class) = *ast.node(ast.body()[0]) else {
        panic!("Klasse erwartet");
    };
    let class = ast.class(class).expect("Klasse");
    assert!(class.heritage.is_some());
    let constructor = class
        .constructor
        .and_then(|function| ast.function(function))
        .expect("Konstruktor");
    assert_eq!(constructor.kind, FunctionKind::DerivedConstructor);
    assert_eq!(ast.list(class.elements).len(), 4);
    for (text, kind) in [
        (
            "class C { m() { this.#a; } }",
            ErrorKind::UndeclaredPrivateName,
        ),
        ("this.#a", ErrorKind::UndeclaredPrivateName),
        ("class C { #a; #a; }", ErrorKind::DuplicatePrivateName),
        (
            "class C { get #a() {} static set #a(v) {} }",
            ErrorKind::DuplicatePrivateName,
        ),
        (
            "class C { constructor() {} constructor() {} }",
            ErrorKind::InvalidClassElement,
        ),
        (
            "class C { get constructor() {} }",
            ErrorKind::InvalidClassElement,
        ),
        (
            "class C { static prototype() {} }",
            ErrorKind::InvalidClassElement,
        ),
        (
            "class C { constructor = 1 }",
            ErrorKind::InvalidClassElement,
        ),
        ("class C { #constructor }", ErrorKind::InvalidClassElement),
        (
            "class C { constructor() { super(); } }",
            ErrorKind::InvalidSuper,
        ),
        ("class C { x = arguments }", ErrorKind::InvalidArguments),
        (
            "class C { static { await } }",
            ErrorKind::InvalidYieldOrAwait,
        ),
        ("class C { static { return } }", ErrorKind::InvalidReturn),
        ("class C { m() { with (a) b } }", ErrorKind::StrictWith),
        ("class static {}", ErrorKind::ReservedWord),
        (
            "class C { static { var a; let a; } }",
            ErrorKind::Redeclaration,
        ),
    ] {
        assert_eq!(script_error(text).kind, kind, "{text:?}");
    }
    script("class C { #a; m() { class D { n() { this.#a; } } } static async *#b() {} }");
    script("class C { static; get; set; async; static static() {} 'constructor'() {} }");
    script("class C { static { (() => { var await; }); (function await() {}); } }");
    script("class C { x = () => this.y; y = super.z; }");
}

#[test]
fn modules() {
    let ast = module(
        "import a, {b as c, 'd' as e} from 'x' with {type: 'json'}; export {a, c as default};",
    );
    let Node::Import {
        specifiers,
        source,
        attributes,
    } = *ast.node(ast.body()[0])
    else {
        panic!("Import erwartet");
    };
    assert_eq!(name(&ast, source), "x");
    let imported: Vec<String> = ast
        .list(specifiers)
        .iter()
        .map(|specifier| match *ast.node(*specifier) {
            Node::ImportDefault(local) => format!("default as {}", name(&ast, local)),
            Node::ImportSpecifier { imported, local } => {
                format!("{} as {}", name(&ast, imported), name(&ast, local))
            }
            ref other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(imported, ["default as a", "b as c", "d as e"]);
    assert_eq!(attributes.len(), 1);
    module(
        "export * from 'x'; export * as y from 'x'; export {z as 'a b'} from 'x'; export default function () {}",
    );
    module(
        "export var a, [b] = c; export let d = 1; export function f() {} export class G {} export {a as aa};",
    );
    module("await 1; for await (a of b); export {h}; var h;");
    for (text, kind) in [
        ("export {a}", ErrorKind::UndeclaredExport),
        (
            "export default 1; export default 2;",
            ErrorKind::DuplicateExport,
        ),
        ("var a, b; export {a, b as a}", ErrorKind::DuplicateExport),
        ("import {a} from 'x'; let a;", ErrorKind::Redeclaration),
        ("import a, * as a from 'x'", ErrorKind::Redeclaration),
        (
            "export {'\\uD800' as a} from 'x'",
            ErrorKind::InvalidExportName,
        ),
        ("export {'a'}", ErrorKind::InvalidExportName),
        ("export {if}", ErrorKind::ReservedWord),
        (
            "import x from 'y' with {a: 'b', a: 'c'}",
            ErrorKind::DuplicateImportAttribute,
        ),
        ("{ import x from 'y' }", ErrorKind::InvalidModuleItem),
        ("new.target", ErrorKind::InvalidNewTarget),
        ("var await;", ErrorKind::ReservedWord),
        ("function f() { await 1 }", ErrorKind::ReservedWord),
        ("export using a = b;", ErrorKind::UnexpectedToken),
    ] {
        assert_eq!(module_error(text), kind, "{text:?}");
    }
    assert_eq!(
        script_error("import x from 'y'").kind,
        ErrorKind::InvalidModuleItem
    );
    assert_eq!(
        script_error("import.meta").kind,
        ErrorKind::InvalidImportMeta
    );
    script("import('x'); import('x', {}); var await;");
}

#[test]
fn ranges_and_error_positions() {
    let ast = script("  foo(bar)  ");
    assert_eq!(ast.range(expression(&ast)), 2..10);
    let text = "x = async function f(a) { return a } + 1";
    let ast = script(text);
    let Node::Assign { value, .. } = *ast.node(expression(&ast)) else {
        panic!("Zuweisung erwartet");
    };
    let Node::Binary { left, .. } = *ast.node(value) else {
        panic!("Summe erwartet");
    };
    let Node::Function(function) = *ast.node(left) else {
        panic!("Funktion erwartet");
    };
    let range = ast.function(function).expect("Funktion").range.clone();
    assert_eq!(text.get(range), Some("async function f(a) { return a }"));
    // Lagen zählen UTF-16-Codeeinheiten.
    assert_eq!(script_error("'\u{1F600}'; a b").position, 8);
    assert_eq!(script_error("let a;\nlet a;").position, 11);
}

/// Baut `open` n-mal, dann `middle`, dann `close` n-mal.
fn nested(open: &str, middle: &str, close: &str, depth: usize) -> String {
    let mut text = open.repeat(depth);
    text.push_str(middle);
    text.push_str(&close.repeat(depth));
    text
}

/// Jede Form der Verschachtelung ergibt bei großer Tiefe `TooDeep`, ohne
/// dass ein Stapel von 1 MiB überläuft, und bei kleiner Tiefe einen Baum.
#[test]
fn deepest_nesting_fits_on_a_small_stack() {
    let forms: [(&str, &str, &str, &str, &str); 44] = [
        ("", "(", "a", ")", ""),
        ("", "[", "a", "]", ""),
        ("x = ", "{a: ", "a", "}", ""),
        ("", "a => ", "a", "", ""),
        ("", "async a => ", "a", "", ""),
        ("", "!", "a", "", ""),
        ("", "new ", "a", "", ""),
        ("", "a ? ", "a", " : a", ""),
        ("", "a = ", "a", "", ""),
        ("", "f(", "a", ")", ""),
        ("", "a[", "a", "]", ""),
        ("", "`${", "a", "}`", ""),
        ("x = ", "class extends ", "a", " {}", ""),
        ("let ", "[", "a", "]", " = b"),
        ("", "{", "", "}", ""),
        ("", "if (a) ", ";", "", ""),
        ("", "function f() {", "", "}", ""),
        ("", "for (;;) ", ";", "", ""),
        ("", "for (a in b) ", ";", "", ""),
        ("", "for (let a of b) ", ";", "", ""),
        ("", "while (a) ", ";", "", ""),
        ("", "do ", ";", " while (a)", ""),
        ("", "with (a) ", ";", "", ""),
        ("", "try {", "", "} finally {}", ""),
        ("", "switch (a) { case a: ", "", "}", ""),
        ("", "if (a) {} else ", ";", "", ""),
        ("x = ", "{[", "a", "]: a}", ""),
        ("x = ", "{...", "a", "}", ""),
        ("x = ", "{a() { return ", "a", "}}", ""),
        ("", "[...", "a", "]", ""),
        ("", "a?.(", "a", ")", ""),
        ("", "a`${", "a", "}`", ""),
        ("", "a ? a : ", "a", "", ""),
        ("", "new (", "a", ")", ""),
        ("", "import(", "a", ")", ""),
        ("", "[", "a", "] = b", ""),
        ("", "() => {", "", "}", ""),
        ("", "async () => ", "a", "", ""),
        ("", "function* g() { yield ", "a", "}", ""),
        ("", "function f(a = ", "a", ") {}", ""),
        ("", "class A extends ", "a", " {}", ""),
        ("", "x = class { a() {", "", "} }", ""),
        ("", "x = class { static {", "", "} }", ""),
        ("", "x = class { a = ", "a", " }", ""),
    ];
    let thread = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            for (before, open, middle, close, after) in forms {
                for (depth, deep) in [(8, false), (10_000, true)] {
                    let text = format!("{before}{}{after}", nested(open, middle, close, depth));
                    let result = parse_script(&utf16(&text));
                    match (result, deep) {
                        (Ok(_), false) => {}
                        (Err(error), true) if error.kind == ErrorKind::TooDeep => {}
                        (result, _) => panic!(
                            "{before}{open}…{close}{after} bei Tiefe {depth}: {:?}",
                            result.map(|_| ())
                        ),
                    }
                }
            }
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}
