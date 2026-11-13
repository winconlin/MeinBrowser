use super::*;
use crate::agent::Exception;
use crate::compare::same_value;
use crate::function::is_constructor;
use crate::heap::Options;
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::number;
use crate::object::{get_own_property, get_prototype_of, own_property_keys};
use crate::operations::{call, create_data_property_or_throw, is_callable};
use crate::script::ScriptError;

fn agent_with(stress: bool) -> (Agent, RealmId) {
    let mut agent = Agent::with_options(Options {
        stress,
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    (agent, realm)
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

/// Gibt dem globalen Objekt die Eigenschaft `tag` mit @@toStringTag. Ohne
/// %Symbol% kommt ein Script sonst an kein Symbol.
fn define_tag(agent: &mut Agent, realm: RealmId) {
    agent.scope(|agent, scope| {
        let symbol = agent.well_known_symbol(scope, WellKnownSymbol::ToStringTag);
        let global = agent.global_object(scope, realm);
        let key = agent
            .key_from_units(scope, &units("tag"))
            .expect("Speicher");
        create_data_property_or_throw(agent, scope, global, key, symbol).expect("Eigenschaft");
    });
}

/// Führt ein Script aus und beschreibt das Ergebnis wie die Tests der
/// Scripts. Danach ist der Registerstapel wieder leer.
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

/// Jeder Fall in einem eigenen Agent mit `tag`.
fn check(cases: &[(&str, &str)]) {
    for &(source, expected) in cases {
        let (mut agent, realm) = agent_with(false);
        define_tag(&mut agent, realm);
        assert_eq!(run(&mut agent, realm, source), expected, "{source}");
    }
}

fn key<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> PropertyKey<'s> {
    agent.key_from_units(scope, &units(text)).expect("Speicher")
}

fn property<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    name: &str,
) -> PropertyDescriptor<'s> {
    let key = key(agent, scope, name);
    get_own_property(agent, scope, object, key)
        .expect("kein Fehler")
        .unwrap_or_else(|| panic!("keine Eigenschaft {name}"))
}

/// Wert, [[Writable]], [[Enumerable]] und [[Configurable]] einer
/// Dateneigenschaft.
fn data<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    name: &str,
) -> (Local<'s>, [Option<bool>; 3]) {
    let descriptor = property(agent, scope, object, name);
    let value = descriptor.value.expect("Dateneigenschaft");
    (
        value,
        [
            descriptor.writable,
            descriptor.enumerable,
            descriptor.configurable,
        ],
    )
}

fn keys(agent: &mut Agent, scope: &Scope, object: Local<'_>) -> Vec<String> {
    let keys = own_property_keys(agent, scope, object).expect("kein Fehler");
    keys.into_iter()
        .map(|key| match key {
            PropertyKey::Index(index) => index.to_string(),
            PropertyKey::String(name) => describe(agent, name).trim_matches('"').to_owned(),
            PropertyKey::Symbol(_) => "Symbol".to_owned(),
        })
        .collect()
}

fn same(agent: &Agent, left: Local<'_>, right: Local<'_>) -> bool {
    let heap = agent.heap();
    same_value(heap, heap.get(left), heap.get(right))
}

fn prototype_of<'s>(agent: &mut Agent, scope: &'s Scope, object: Local<'_>) -> Local<'s> {
    get_prototype_of(agent, scope, object)
        .expect("kein Fehler")
        .expect("Prototyp")
}

const READ_ONLY: [Option<bool>; 3] = [Some(false), Some(false), Some(false)];
const HIDDEN: [Option<bool>; 3] = [Some(true), Some(false), Some(true)];

#[test]
fn function_prototype_is_a_builtin_function() {
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let prototype = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
        let value = agent.heap().get(prototype);
        assert!(is_callable(agent.heap(), value));
        assert!(!is_constructor(agent, value));
        let parent = prototype_of(agent, scope, prototype);
        let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
        assert!(same(agent, parent, object));
        assert_eq!(
            keys(agent, scope, prototype),
            [
                "length",
                "name",
                "constructor",
                "apply",
                "bind",
                "call",
                "toString",
                "caller",
                "arguments",
                "Symbol"
            ]
        );
        let (length, attributes) = data(agent, scope, prototype, "length");
        assert_eq!(describe(agent, length), "0");
        assert_eq!(attributes, [Some(false), Some(false), Some(true)]);
        let (name, _) = data(agent, scope, prototype, "name");
        assert_eq!(describe(agent, name), "\"\"");
        let (_, attributes) = data(agent, scope, prototype, "call");
        assert_eq!(attributes, HIDDEN);
        let one = agent.heap().local(scope, Value::int32(1));
        let result = call(agent, scope, prototype, one, &[one]).expect("kein Fehler");
        assert_eq!(describe(agent, result), "undefined");
    });
}

#[test]
fn constructors_are_global_and_linked_to_their_prototypes() {
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let global = agent.global_object(scope, realm);
        for (name, constructor, prototype, parent) in [
            (
                "Object",
                Intrinsic::Object,
                Intrinsic::ObjectPrototype,
                Intrinsic::FunctionPrototype,
            ),
            (
                "Function",
                Intrinsic::Function,
                Intrinsic::FunctionPrototype,
                Intrinsic::FunctionPrototype,
            ),
            (
                "String",
                Intrinsic::String,
                Intrinsic::StringPrototype,
                Intrinsic::FunctionPrototype,
            ),
            (
                "Error",
                Intrinsic::Error,
                Intrinsic::ErrorPrototype,
                Intrinsic::FunctionPrototype,
            ),
            (
                "EvalError",
                Intrinsic::EvalError,
                Intrinsic::EvalErrorPrototype,
                Intrinsic::Error,
            ),
            (
                "RangeError",
                Intrinsic::RangeError,
                Intrinsic::RangeErrorPrototype,
                Intrinsic::Error,
            ),
            (
                "ReferenceError",
                Intrinsic::ReferenceError,
                Intrinsic::ReferenceErrorPrototype,
                Intrinsic::Error,
            ),
            (
                "SyntaxError",
                Intrinsic::SyntaxError,
                Intrinsic::SyntaxErrorPrototype,
                Intrinsic::Error,
            ),
            (
                "TypeError",
                Intrinsic::TypeError,
                Intrinsic::TypeErrorPrototype,
                Intrinsic::Error,
            ),
            (
                "URIError",
                Intrinsic::URIError,
                Intrinsic::URIErrorPrototype,
                Intrinsic::Error,
            ),
        ] {
            let constructor = agent.intrinsic(scope, realm, constructor);
            let prototype = agent.intrinsic(scope, realm, prototype);
            let parent = agent.intrinsic(scope, realm, parent);
            let (value, attributes) = data(agent, scope, global, name);
            assert!(same(agent, value, constructor), "{name}");
            assert_eq!(attributes, HIDDEN, "{name}");
            assert!(
                is_constructor(agent, agent.heap().get(constructor)),
                "{name}"
            );
            let found = prototype_of(agent, scope, constructor);
            assert!(same(agent, found, parent), "{name}");
            let (value, attributes) = data(agent, scope, constructor, "prototype");
            assert!(same(agent, value, prototype), "{name}");
            assert_eq!(attributes, READ_ONLY, "{name}");
            let (value, attributes) = data(agent, scope, prototype, "constructor");
            assert!(same(agent, value, constructor), "{name}");
            assert_eq!(attributes, HIDDEN, "{name}");
            let (length, _) = data(agent, scope, constructor, "length");
            assert_eq!(describe(agent, length), "1", "{name}");
            let (value, _) = data(agent, scope, constructor, "name");
            assert_eq!(describe(agent, value), format!("{name:?}"));
        }
    });
}

#[test]
fn builtin_functions_have_names_lengths_and_no_construct() {
    check(&[
        ("typeof Object", "\"function\""),
        ("typeof Object.prototype.toString", "\"function\""),
        ("Object.prototype.toString.name", "\"toString\""),
        ("Object.prototype.toString.length", "0"),
        ("Object.prototype.toString.prototype", "undefined"),
        ("(function () {}).call.name", "\"call\""),
        ("(function () {}).call.length", "1"),
        ("Error.prototype.toString.name", "\"toString\""),
        ("Error.prototype.name", "\"Error\""),
        ("Error.prototype.message", "\"\""),
        ("EvalError.prototype.name", "\"EvalError\""),
        ("URIError.prototype.message", "\"\""),
        (
            "TypeError.prototype.toString === Error.prototype.toString",
            "true",
        ),
        ("TypeError.prototype instanceof Error", "true"),
        ("Error.prototype instanceof Object", "true"),
        ("TypeError instanceof Object", "true"),
        ("new Object.prototype.toString()", "TypeError"),
        ("new (function () {}).call()", "TypeError"),
        ("new Error.prototype.toString()", "TypeError"),
        ("'abc'.constructor === String", "true"),
        ("String.prototype.length", "0"),
        ("String.prototype[0]", "undefined"),
    ]);
}

#[test]
fn call_passes_this_and_arguments() {
    check(&[
        (
            "function f(a, b) { 'use strict'; return this + a + b; } f.call(1, 2, 3)",
            "6",
        ),
        (
            "function f() { 'use strict'; return arguments.length; } f.call(0, 1, 2, 3)",
            "3",
        ),
        (
            "function f() { 'use strict'; return this; } f.call()",
            "undefined",
        ),
        (
            "function f() { 'use strict'; return this; } f.call(null)",
            "null",
        ),
        (
            "function f() { return this === globalThis; } f.call() && f.call(undefined) && f.call(null)",
            "true",
        ),
        (
            "function f() { return typeof this; } f.call(1)",
            "\"object\"",
        ),
        (
            "function f() { return this; } var o = {}; f.call(o) === o",
            "true",
        ),
        (
            "function f() { return this; } f.call(1) === f.call(1)",
            "false",
        ),
        (
            "function f() { return this; } f.call('ab').length + f.call('ab')[1]",
            "\"2b\"",
        ),
        (
            "function f() { return Object.prototype.toString.call(this); } f.call(true)",
            "\"[object Boolean]\"",
        ),
        (
            "(function () {}).call.call(function (x) { 'use strict'; return this + x; }, 1, 2)",
            "3",
        ),
        ("(function () {}).call.call(1)", "TypeError"),
        (
            "var o = { call: (function () {}).call }; o.call()",
            "TypeError",
        ),
        (
            "try { (function () {}).call.call({}); } catch (e) { e instanceof TypeError && e.constructor === TypeError }",
            "true",
        ),
        // In Endposition kommt der Aufruf aus dem Befehl selbst zurück.
        (
            "'use strict'; function f(g) { return g.call(undefined, 5); } f(function (x) { return x * 2; })",
            "10",
        ),
        (
            "function f(g) { return g.call(undefined, 5); } f(function (x) { throw x; })",
            "throw 5",
        ),
    ]);
}

#[test]
fn object_converts_its_argument() {
    check(&[
        ("typeof Object()", "\"object\""),
        ("typeof Object(undefined)", "\"object\""),
        ("typeof Object(null)", "\"object\""),
        ("Object() === Object()", "false"),
        ("var o = {}; Object(o) === o && new Object(o) === o", "true"),
        ("typeof Object(1)", "\"object\""),
        ("Object(1) === Object(1)", "false"),
        ("Object(1) instanceof Object", "true"),
        ("Object('ab').length", "2"),
        ("new Object('ab')[1]", "\"b\""),
        (
            "Object.prototype.toString.call(new Object(true))",
            "\"[object Boolean]\"",
        ),
        ("typeof Object(tag)", "\"object\""),
        ("var f = function () {}; Object(f) === f", "true"),
    ]);
}

#[test]
fn object_prototype_to_string_names_the_builtin_tag() {
    check(&[
        ("Object.prototype.toString.call()", "\"[object Undefined]\""),
        ("Object.prototype.toString.call(null)", "\"[object Null]\""),
        ("Object.prototype.toString.call([])", "\"[object Array]\""),
        (
            "Object.prototype.toString.call(function () {})",
            "\"[object Function]\"",
        ),
        (
            "Object.prototype.toString.call(Object.prototype.toString)",
            "\"[object Function]\"",
        ),
        (
            "(function () { return Object.prototype.toString.call(arguments); })()",
            "\"[object Arguments]\"",
        ),
        (
            "Object.prototype.toString.call(new TypeError())",
            "\"[object Error]\"",
        ),
        (
            "try { null.x; } catch (e) { Object.prototype.toString.call(e) }",
            "\"[object Error]\"",
        ),
        // Die Prototypen der Fehler sind gewöhnliche Objekte, die der
        // primitiven Werte Wrapper.
        (
            "Object.prototype.toString.call(Error.prototype)",
            "\"[object Object]\"",
        ),
        (
            "Object.prototype.toString.call(String.prototype)",
            "\"[object String]\"",
        ),
        (
            "Object.prototype.toString.call(true)",
            "\"[object Boolean]\"",
        ),
        ("Object.prototype.toString.call(-0)", "\"[object Number]\""),
        ("Object.prototype.toString.call('')", "\"[object String]\""),
        ("Object.prototype.toString.call({})", "\"[object Object]\""),
        ("String({})", "\"[object Object]\""),
        ("'' + {}", "\"[object Object]\""),
        (
            "var o = {}; o[tag] = 'Eigen'; Object.prototype.toString.call(o)",
            "\"[object Eigen]\"",
        ),
        (
            "var a = []; a[tag] = 'Liste'; Object.prototype.toString.call(a)",
            "\"[object Liste]\"",
        ),
        (
            "var o = {}; o[tag] = 1; Object.prototype.toString.call(o)",
            "\"[object Object]\"",
        ),
        (
            "String.prototype[tag] = 'S'; Object.prototype.toString.call('a')",
            "\"[object S]\"",
        ),
        (
            "var o = { get [tag]() { throw 7; } }; Object.prototype.toString.call(o)",
            "throw 7",
        ),
    ]);
}

#[test]
fn string_converts_and_wraps() {
    check(&[
        ("String()", "\"\""),
        ("String(undefined)", "\"undefined\""),
        ("String(null)", "\"null\""),
        ("String(-0)", "\"0\""),
        ("String(1.5)", "\"1.5\""),
        ("String(true)", "\"true\""),
        ("String('x', 'y')", "\"x\""),
        ("String(tag)", "\"Symbol(Symbol.toStringTag)\""),
        ("new String(tag)", "TypeError"),
        ("'' + tag", "TypeError"),
        (
            "String({ toString: function () { return 'eigen'; } })",
            "\"eigen\"",
        ),
        ("String({ toString: function () { throw 3; } })", "throw 3"),
        ("typeof new String('ab')", "\"object\""),
        ("new String('ab').length", "2"),
        ("new String().length", "0"),
        ("new String(12)[1]", "\"2\""),
        ("new String('ab')[2]", "undefined"),
        ("'1' in new String('ab')", "true"),
        ("'2' in new String('ab')", "false"),
        ("new String('ab') instanceof String", "true"),
        ("new String('ab') === new String('ab')", "false"),
        // Codeeinheiten und `length` lassen sich weder ändern noch löschen.
        ("var s = new String('ab'); s[0] = 'x'; s[0]", "\"a\""),
        (
            "'use strict'; var s = new String('ab'); s[0] = 'x';",
            "TypeError",
        ),
        ("'use strict'; new String('ab').length = 5;", "TypeError"),
        ("delete new String('ab')[0]", "false"),
        ("'use strict'; delete new String('ab')[1];", "TypeError"),
        (
            "var s = new String('ab'); s[5] = 'z'; s[5] + s.length",
            "\"z2\"",
        ),
    ]);
}

#[test]
fn string_objects_have_index_properties_and_length() {
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let object = agent
            .evaluate_script(
                scope,
                realm,
                &units("var s = new String('ab'); s.x = 1; s[5] = 2; s"),
            )
            .expect("Script");
        assert_eq!(keys(agent, scope, object), ["0", "1", "5", "length", "x"]);
        let (unit, attributes) = data(agent, scope, object, "0");
        assert_eq!(describe(agent, unit), "\"a\"");
        assert_eq!(attributes, [Some(false), Some(true), Some(false)]);
        let (length, attributes) = data(agent, scope, object, "length");
        assert_eq!(describe(agent, length), "2");
        assert_eq!(attributes, READ_ONLY);
        let prototype = agent.intrinsic(scope, realm, Intrinsic::StringPrototype);
        let found = prototype_of(agent, scope, object);
        assert!(same(agent, found, prototype));
        assert_eq!(
            keys(agent, scope, prototype),
            ["length", "constructor", "Symbol"]
        );
    });
}

#[test]
fn error_constructors_create_error_objects() {
    check(&[
        ("new Error('m').message", "\"m\""),
        ("Error('m').message", "\"m\""),
        ("Error('m') instanceof Error", "true"),
        ("new Error().message", "\"\""),
        ("typeof new Error(1).message", "\"string\""),
        ("new Error(undefined).message", "\"\""),
        (
            "var e = new TypeError('x'); e instanceof TypeError && e instanceof Error && !(e instanceof RangeError)",
            "true",
        ),
        ("TypeError('x').constructor === TypeError", "true"),
        ("URIError('u').message", "\"u\""),
        ("new Error('m', { cause: 1 }).cause", "1"),
        ("'cause' in new Error('m', {})", "false"),
        ("'cause' in new Error('m', 1)", "false"),
        ("'cause' in new Error('m', { cause: undefined })", "true"),
        ("new RangeError('m', { cause: 'c' }).cause", "\"c\""),
        (
            "new Error({ toString: function () { throw 7; } })",
            "throw 7",
        ),
        (
            "var log = ''; new Error({ toString: function () { log += 'm'; return ''; } }, { get cause() { log += 'c'; } }); log",
            "\"mc\"",
        ),
        ("new Error('m', tag).message", "\"m\""),
        ("new Error(tag)", "TypeError"),
    ]);
}

#[test]
fn error_objects_get_own_message_and_cause() {
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let plain = agent
            .evaluate_script(scope, realm, &units("new Error()"))
            .expect("Script");
        assert!(keys(agent, scope, plain).is_empty());
        let full = agent
            .evaluate_script(scope, realm, &units("new SyntaxError(2, { cause: 3 })"))
            .expect("Script");
        assert_eq!(keys(agent, scope, full), ["message", "cause"]);
        let (message, attributes) = data(agent, scope, full, "message");
        assert_eq!(describe(agent, message), "\"2\"");
        assert_eq!(attributes, HIDDEN);
        let (_, attributes) = data(agent, scope, full, "cause");
        assert_eq!(attributes, HIDDEN);
        let prototype = agent.intrinsic(scope, realm, Intrinsic::SyntaxErrorPrototype);
        let found = prototype_of(agent, scope, full);
        assert!(same(agent, found, prototype));
    });
}

#[test]
fn error_prototype_to_string_joins_name_and_message() {
    check(&[
        ("new TypeError('x').toString()", "\"TypeError: x\""),
        ("new Error().toString()", "\"Error\""),
        ("String(new RangeError('r'))", "\"RangeError: r\""),
        ("'' + new EvalError('e')", "\"EvalError: e\""),
        (
            "Error.prototype.toString.call({ name: 'N', message: 'M' })",
            "\"N: M\"",
        ),
        (
            "Error.prototype.toString.call({ message: 'M' })",
            "\"Error: M\"",
        ),
        (
            "Error.prototype.toString.call({ name: '', message: 'M' })",
            "\"M\"",
        ),
        (
            "Error.prototype.toString.call({ name: 'N', message: '' })",
            "\"N\"",
        ),
        ("Error.prototype.toString.call({})", "\"Error\""),
        (
            "Error.prototype.toString.call({ name: 1, message: 2 })",
            "\"1: 2\"",
        ),
        ("Error.prototype.toString.call(1)", "TypeError"),
        ("Error.prototype.toString.call({ name: tag })", "TypeError"),
        (
            "try { undefinedName; } catch (e) { String(e) }",
            "\"ReferenceError\"",
        ),
        (
            "try { null.x; } catch (e) { e.constructor === TypeError && e.name }",
            "\"TypeError\"",
        ),
    ]);
}

/// Ein Wert aus `second` wird Eigenschaft des globalen Objekts von `first`.
fn share(agent: &mut Agent, first: RealmId, second: RealmId, name: &str, source: &str) {
    agent.scope(|agent, scope| {
        let value = agent
            .evaluate_script(scope, second, &units(source))
            .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        let global = agent.global_object(scope, first);
        let key = key(agent, scope, name);
        create_data_property_or_throw(agent, scope, global, key, value).expect("Eigenschaft");
    });
}

#[test]
fn builtins_work_in_their_own_realm() {
    for stress in [false, true] {
        let (mut agent, first) = agent_with(stress);
        let second = agent.create_realm().expect("Speicher");
        share(
            &mut agent,
            first,
            second,
            "otherCall",
            "(function () {}).call",
        );
        share(&mut agent, first, second, "OtherObject", "Object");
        share(&mut agent, first, second, "OtherError", "Error");
        share(&mut agent, first, second, "OtherTypeError", "TypeError");
        share(&mut agent, first, second, "OtherString", "String");
        share(
            &mut agent,
            first,
            second,
            "otherThis",
            "(function () { return this; })",
        );
        for (source, intrinsic) in [
            (
                "try { otherCall(); } catch (error) { error }",
                Intrinsic::TypeErrorPrototype,
            ),
            ("OtherObject()", Intrinsic::ObjectPrototype),
            ("OtherObject(1)", Intrinsic::NumberPrototype),
            ("new OtherError('m')", Intrinsic::ErrorPrototype),
            ("OtherTypeError('m')", Intrinsic::TypeErrorPrototype),
            ("new OtherString('s')", Intrinsic::StringPrototype),
            // Der Realm der aufgerufenen Funktion macht `this` zum Objekt,
            // auch wenn das `call` aus dem ersten Realm stammt.
            (
                "(function () {}).call.call(otherThis, true)",
                Intrinsic::BooleanPrototype,
            ),
            ("otherThis.call('a')", Intrinsic::StringPrototype),
            ("otherThis()", Intrinsic::ObjectPrototype),
        ] {
            agent.scope(|agent, scope| {
                let value = agent
                    .evaluate_script(scope, first, &units(source))
                    .unwrap_or_else(|error| panic!("{source}: {error:?}"));
                let found = prototype_of(agent, scope, value);
                let expected = agent.intrinsic(scope, second, intrinsic);
                assert!(same(agent, found, expected), "{source}");
                let local = agent.intrinsic(scope, first, intrinsic);
                assert!(!same(agent, found, local), "{source}");
            });
        }
        assert_eq!(
            run(
                &mut agent,
                first,
                "try { otherCall(); } catch (e) { e instanceof TypeError || e instanceof OtherTypeError }"
            ),
            "true"
        );
        assert_eq!(
            run(
                &mut agent,
                first,
                "try { otherCall(); } catch (e) { e instanceof TypeError }"
            ),
            "false"
        );
    }
}

#[test]
fn builtins_survive_collections() {
    let (mut agent, realm) = agent_with(true);
    define_tag(&mut agent, realm);
    let source = "var s = new String('abc'); \
        var e = new TypeError('m', { cause: s }); \
        var call = (function (x) {}).call; \
        var bound = (function (a, b) { return this.x + a + b; }).bind({ x: 'p' }, 'q'); \
        var frozen = Object.freeze(Object.fromEntries([['k', 'v']])); \
        e.cause[2] + e.message + String(e) + Object.prototype.toString.call(s) + \
        call.call(function (x) { 'use strict'; return x; }, 0, 'z') + String(tag) + \
        (Object(1).constructor === Number) + bound('r') + bound.name + \
        Object.keys(frozen)[0] + Object.isFrozen(frozen) + \
        Object.getOwnPropertyDescriptor(frozen, 'k').value";
    assert_eq!(
        run(&mut agent, realm, source),
        "\"cmTypeError: m[object String]zSymbol(Symbol.toStringTag)truepqrbound ktruev\""
    );
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
}

/// Wert und Attribute einer Dateneigenschaft mit einem bekannten Symbol als
/// Schlüssel.
fn symbol_data<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    symbol: WellKnownSymbol,
) -> (Local<'s>, [Option<bool>; 3]) {
    let key = agent.well_known_symbol(scope, symbol);
    let descriptor = get_own_property(agent, scope, object, PropertyKey::Symbol(key))
        .expect("kein Fehler")
        .unwrap_or_else(|| panic!("keine Eigenschaft {}", symbol.description()));
    let value = descriptor.value.expect("Dateneigenschaft");
    (
        value,
        [
            descriptor.writable,
            descriptor.enumerable,
            descriptor.configurable,
        ],
    )
}

#[test]
fn symbol_holds_the_well_known_symbols() {
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let global = agent.global_object(scope, realm);
        let constructor = agent.intrinsic(scope, realm, Intrinsic::Symbol);
        let prototype = agent.intrinsic(scope, realm, Intrinsic::SymbolPrototype);
        let function = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
        let (value, attributes) = data(agent, scope, global, "Symbol");
        assert!(same(agent, value, constructor));
        assert_eq!(attributes, HIDDEN);
        assert!(is_constructor(agent, agent.heap().get(constructor)));
        let parent = prototype_of(agent, scope, constructor);
        assert!(same(agent, parent, function));
        let (value, attributes) = data(agent, scope, constructor, "prototype");
        assert!(same(agent, value, prototype));
        assert_eq!(attributes, READ_ONLY);
        let (value, attributes) = data(agent, scope, prototype, "constructor");
        assert!(same(agent, value, constructor));
        assert_eq!(attributes, HIDDEN);
        let (length, _) = data(agent, scope, constructor, "length");
        assert_eq!(describe(agent, length), "0");
        let (name, _) = data(agent, scope, constructor, "name");
        assert_eq!(describe(agent, name), "\"Symbol\"");
        for symbol in WellKnownSymbol::ALL {
            let name = symbol
                .description()
                .strip_prefix("Symbol.")
                .expect("Beschreibung mit Symbol.");
            let (value, attributes) = data(agent, scope, constructor, name);
            let expected = agent.well_known_symbol(scope, symbol);
            assert!(same(agent, value, expected), "{name}");
            assert_eq!(attributes, READ_ONLY, "{name}");
        }
        // `length`, `name`, `prototype`, `for` und `keyFor`.
        assert_eq!(
            keys(agent, scope, constructor).len(),
            5 + WellKnownSymbol::ALL.len()
        );
    });
}

#[test]
fn iterator_prototypes_inherit_from_iterator_prototype() {
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
        let iterator = agent.intrinsic(scope, realm, Intrinsic::IteratorPrototype);
        let parent = prototype_of(agent, scope, iterator);
        assert!(same(agent, parent, object));
        // Die Methoden, dann der Accessor `constructor` und die Symbole
        // @@iterator, @@dispose und @@toStringTag.
        assert_eq!(
            keys(agent, scope, iterator),
            [
                "chunks",
                "drop",
                "every",
                "filter",
                "find",
                "flatMap",
                "forEach",
                "includes",
                "join",
                "map",
                "reduce",
                "some",
                "take",
                "toArray",
                "windows",
                "constructor",
                "Symbol",
                "Symbol",
                "Symbol"
            ]
        );
        let (method, attributes) = symbol_data(agent, scope, iterator, WellKnownSymbol::Iterator);
        assert_eq!(attributes, HIDDEN);
        let (name, _) = data(agent, scope, method, "name");
        assert_eq!(describe(agent, name), "\"[Symbol.iterator]\"");
        for (intrinsic, tag) in [
            (Intrinsic::ArrayIteratorPrototype, "Array Iterator"),
            (Intrinsic::StringIteratorPrototype, "String Iterator"),
        ] {
            let prototype = agent.intrinsic(scope, realm, intrinsic);
            let parent = prototype_of(agent, scope, prototype);
            assert!(same(agent, parent, iterator), "{tag}");
            assert_eq!(keys(agent, scope, prototype), ["next", "Symbol"], "{tag}");
            let (next, attributes) = data(agent, scope, prototype, "next");
            assert_eq!(attributes, HIDDEN, "{tag}");
            assert!(!is_constructor(agent, agent.heap().get(next)), "{tag}");
            let (value, attributes) =
                symbol_data(agent, scope, prototype, WellKnownSymbol::ToStringTag);
            assert_eq!(describe(agent, value), format!("{tag:?}"));
            assert_eq!(attributes, [Some(false), Some(false), Some(true)], "{tag}");
        }
    });
}

/// %AsyncIteratorPrototype% hat nur @@asyncIterator, das `this` zurückgibt,
/// %AsyncFromSyncIteratorPrototype% erbt von ihm und hat `next`, `return`
/// und `throw` mit optionalem Parameter, also mit `length` 0.
#[test]
fn async_iterator_prototypes_inherit_from_object_prototype() {
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
        let iterator = agent.intrinsic(scope, realm, Intrinsic::AsyncIteratorPrototype);
        let parent = prototype_of(agent, scope, iterator);
        assert!(same(agent, parent, object));
        assert_eq!(keys(agent, scope, iterator), ["Symbol"]);
        let (method, attributes) =
            symbol_data(agent, scope, iterator, WellKnownSymbol::AsyncIterator);
        assert_eq!(attributes, HIDDEN);
        let (name, _) = data(agent, scope, method, "name");
        assert_eq!(describe(agent, name), "\"[Symbol.asyncIterator]\"");
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        let result = call(agent, scope, method, object, &[undefined]).expect("Aufruf");
        assert!(same(agent, result, object));
        let prototype = agent.intrinsic(scope, realm, Intrinsic::AsyncFromSyncIteratorPrototype);
        let parent = prototype_of(agent, scope, prototype);
        assert!(same(agent, parent, iterator));
        assert_eq!(keys(agent, scope, prototype), ["next", "return", "throw"]);
        for name in ["next", "return", "throw"] {
            let (function, attributes) = data(agent, scope, prototype, name);
            assert_eq!(attributes, HIDDEN, "{name}");
            assert!(!is_constructor(agent, agent.heap().get(function)), "{name}");
            let (length, _) = data(agent, scope, function, "length");
            assert_eq!(describe(agent, length), "0", "{name}");
        }
    });
}

#[test]
fn generator_prototypes_link_functions_and_iterators() {
    const FIXED: [Option<bool>; 3] = [Some(false), Some(false), Some(true)];
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let function = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
        let iterator = agent.intrinsic(scope, realm, Intrinsic::IteratorPrototype);
        let generator_function =
            agent.intrinsic(scope, realm, Intrinsic::GeneratorFunctionPrototype);
        let generator = agent.intrinsic(scope, realm, Intrinsic::GeneratorPrototype);
        let parent = prototype_of(agent, scope, generator_function);
        assert!(same(agent, parent, function));
        assert!(!is_callable(
            agent.heap(),
            agent.heap().get(generator_function)
        ));
        assert_eq!(
            keys(agent, scope, generator_function),
            ["prototype", "Symbol"]
        );
        let (value, attributes) = data(agent, scope, generator_function, "prototype");
        assert!(same(agent, value, generator));
        assert_eq!(attributes, FIXED);
        let (value, attributes) = symbol_data(
            agent,
            scope,
            generator_function,
            WellKnownSymbol::ToStringTag,
        );
        assert_eq!(describe(agent, value), "\"GeneratorFunction\"");
        assert_eq!(attributes, FIXED);
        let parent = prototype_of(agent, scope, generator);
        assert!(same(agent, parent, iterator));
        assert_eq!(
            keys(agent, scope, generator),
            ["constructor", "next", "return", "throw", "Symbol"]
        );
        let (value, attributes) = data(agent, scope, generator, "constructor");
        assert!(same(agent, value, generator_function));
        assert_eq!(attributes, FIXED);
        for name in ["next", "return", "throw"] {
            let (method, attributes) = data(agent, scope, generator, name);
            assert_eq!(attributes, HIDDEN, "{name}");
            assert!(!is_constructor(agent, agent.heap().get(method)), "{name}");
            let (length, _) = data(agent, scope, method, "length");
            assert_eq!(describe(agent, length), "1", "{name}");
            let (text, _) = data(agent, scope, method, "name");
            assert_eq!(describe(agent, text), format!("{name:?}"));
        }
        let (value, attributes) =
            symbol_data(agent, scope, generator, WellKnownSymbol::ToStringTag);
        assert_eq!(describe(agent, value), "\"Generator\"");
        assert_eq!(attributes, FIXED);
    });
    check(&[
        (
            "Object.prototype.toString.call(function* () {})",
            "\"[object GeneratorFunction]\"",
        ),
        (
            "Object.prototype.toString.call((function* () {})())",
            "\"[object Generator]\"",
        ),
        ("typeof (function* () {})()", "\"object\""),
        // Die Methoden prüfen `this` (GeneratorValidate, Abschnitt 27.8.3.2).
        (
            "var g = (function* () {})(); try { g.next.call({}); } catch (e) { e.name }",
            "\"TypeError\"",
        ),
        (
            "var g = (function* () {})(); try { g.return.call(1); } catch (e) { e.name }",
            "\"TypeError\"",
        ),
        (
            "var g = (function* () {})(); try { g.throw.call(undefined, 2); } catch (e) { e.name }",
            "\"TypeError\"",
        ),
    ]);
}

/// %AsyncFunction.prototype% (Abschnitt 27.10.3): ein gewöhnliches Objekt
/// mit %Function.prototype% als Prototyp, bisher nur mit @@toStringTag.
#[test]
fn the_async_function_prototype_tags_async_functions() {
    const FIXED: [Option<bool>; 3] = [Some(false), Some(false), Some(true)];
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let function = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
        let async_function = agent.intrinsic(scope, realm, Intrinsic::AsyncFunctionPrototype);
        let parent = prototype_of(agent, scope, async_function);
        assert!(same(agent, parent, function));
        assert!(!is_callable(agent.heap(), agent.heap().get(async_function)));
        assert_eq!(keys(agent, scope, async_function), ["Symbol"]);
        let (value, attributes) =
            symbol_data(agent, scope, async_function, WellKnownSymbol::ToStringTag);
        assert_eq!(describe(agent, value), "\"AsyncFunction\"");
        assert_eq!(attributes, FIXED);
    });
    check(&[
        (
            "Object.prototype.toString.call(async function () {})",
            "\"[object AsyncFunction]\"",
        ),
        (
            "Object.prototype.toString.call(async () => {})",
            "\"[object AsyncFunction]\"",
        ),
    ]);
}

/// %AsyncGeneratorFunction.prototype% (Abschnitt 27.7.3) und
/// %AsyncGeneratorPrototype% (Abschnitt 27.9.1), das von
/// %AsyncIteratorPrototype% erbt. Ohne den Konstruktor
/// %AsyncGeneratorFunction% fehlt dem ersten noch `constructor`.
#[test]
fn async_generator_prototypes_link_functions_and_iterators() {
    const FIXED: [Option<bool>; 3] = [Some(false), Some(false), Some(true)];
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let function = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
        let iterator = agent.intrinsic(scope, realm, Intrinsic::AsyncIteratorPrototype);
        let generator_function =
            agent.intrinsic(scope, realm, Intrinsic::AsyncGeneratorFunctionPrototype);
        let generator = agent.intrinsic(scope, realm, Intrinsic::AsyncGeneratorPrototype);
        let parent = prototype_of(agent, scope, generator_function);
        assert!(same(agent, parent, function));
        assert!(!is_callable(
            agent.heap(),
            agent.heap().get(generator_function)
        ));
        assert_eq!(
            keys(agent, scope, generator_function),
            ["prototype", "Symbol"]
        );
        let (value, attributes) = data(agent, scope, generator_function, "prototype");
        assert!(same(agent, value, generator));
        assert_eq!(attributes, FIXED);
        let (value, attributes) = symbol_data(
            agent,
            scope,
            generator_function,
            WellKnownSymbol::ToStringTag,
        );
        assert_eq!(describe(agent, value), "\"AsyncGeneratorFunction\"");
        assert_eq!(attributes, FIXED);
        let parent = prototype_of(agent, scope, generator);
        assert!(same(agent, parent, iterator));
        assert_eq!(
            keys(agent, scope, generator),
            ["constructor", "next", "return", "throw", "Symbol"]
        );
        let (value, attributes) = data(agent, scope, generator, "constructor");
        assert!(same(agent, value, generator_function));
        assert_eq!(attributes, FIXED);
        for name in ["next", "return", "throw"] {
            let (method, attributes) = data(agent, scope, generator, name);
            assert_eq!(attributes, HIDDEN, "{name}");
            assert!(!is_constructor(agent, agent.heap().get(method)), "{name}");
            let (length, _) = data(agent, scope, method, "length");
            assert_eq!(describe(agent, length), "1", "{name}");
            let (text, _) = data(agent, scope, method, "name");
            assert_eq!(describe(agent, text), format!("{name:?}"));
        }
        let (value, attributes) =
            symbol_data(agent, scope, generator, WellKnownSymbol::ToStringTag);
        assert_eq!(describe(agent, value), "\"AsyncGenerator\"");
        assert_eq!(attributes, FIXED);
    });
    check(&[
        (
            "Object.prototype.toString.call(async function* () {})",
            "\"[object AsyncGeneratorFunction]\"",
        ),
        (
            "Object.prototype.toString.call(({ async *m() {} }).m())",
            "\"[object AsyncGenerator]\"",
        ),
        ("typeof (async function* () {})()", "\"object\""),
        // Die Methoden prüfen `this` (AsyncGeneratorValidate, Abschnitt
        // 27.9.3.3), lehnen aber das Promise ab, statt zu werfen.
        (
            "var g = (async function* () {})(); g.next.call({}) instanceof Promise",
            "true",
        ),
    ]);
}

#[test]
fn array_and_string_prototypes_return_iterators() {
    let (mut agent, realm) = agent_with(false);
    agent.scope(|agent, scope| {
        let array = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
        let values = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototypeValues);
        let (value, attributes) = data(agent, scope, array, "values");
        assert!(same(agent, value, values));
        assert_eq!(attributes, HIDDEN);
        let (value, attributes) = symbol_data(agent, scope, array, WellKnownSymbol::Iterator);
        assert!(same(agent, value, values));
        assert_eq!(attributes, HIDDEN);
        for name in ["keys", "entries"] {
            let (_, attributes) = data(agent, scope, array, name);
            assert_eq!(attributes, HIDDEN, "{name}");
        }
        let string = agent.intrinsic(scope, realm, Intrinsic::StringPrototype);
        let (_, attributes) = symbol_data(agent, scope, string, WellKnownSymbol::Iterator);
        assert_eq!(attributes, HIDDEN);
    });
    check(&[
        ("[].values.name + [].values.length", "\"values0\""),
        ("[].keys.name + [].keys.length", "\"keys0\""),
        ("[].entries.name + [].entries.length", "\"entries0\""),
        ("[][Symbol.iterator].name", "\"values\""),
        ("''[Symbol.iterator].name", "\"[Symbol.iterator]\""),
        ("''[Symbol.iterator].length", "0"),
        (
            "[].values().next.name + [].values().next.length",
            "\"next0\"",
        ),
        ("''[Symbol.iterator]().next.name", "\"next\""),
        ("[].values()[Symbol.iterator].length", "0"),
        (
            "[].values()[Symbol.iterator] === ''[Symbol.iterator]()[Symbol.iterator]",
            "true",
        ),
        ("[].values().next === [].keys().next", "true"),
        ("new ([].values)()", "TypeError"),
        ("new ([].values().next)()", "TypeError"),
        ("new (''[Symbol.iterator])()", "TypeError"),
        ("typeof [].values()", "\"object\""),
        ("[].values() instanceof Object", "true"),
    ]);
}

#[test]
fn object_constructor_functions_follow_the_spec() {
    check(&[
        (
            "Object.assign.length + Object.defineProperty.length + \
             Object.getOwnPropertyDescriptor.length",
            "7",
        ),
        ("Object.getOwnPropertyNames.name", "\"getOwnPropertyNames\""),
        ("Object.assign({}, {a: 1}, null, undefined, {b: 2}).b", "2"),
        ("var t = {}; Object.assign(t, {a: 1}) === t", "true"),
        (
            "Object.assign({}, Object.defineProperty({}, 'x', {value: 1})).x",
            "undefined",
        ),
        ("Object.assign({}, {get x() { return 5; }}).x", "5"),
        ("Object.assign(null)", "TypeError"),
        ("Object.getPrototypeOf(Object.create(null))", "null"),
        ("Object.create(1)", "TypeError"),
        ("Object.create({x: 1}).x", "1"),
        (
            "Object.create({}, {y: {value: 2, enumerable: true}}).y",
            "2",
        ),
        (
            "var d = Object.getOwnPropertyDescriptor(\
                 Object.defineProperty({}, 'x', {value: 1}), 'x'); \
             d.value + ',' + d.writable + ',' + d.enumerable + ',' + d.configurable",
            "\"1,false,false,false\"",
        ),
        (
            "var d = Object.getOwnPropertyDescriptor({get x() { return 1; }}, 'x'); \
             typeof d.get + ',' + d.set + ',' + d.enumerable + ',' + \
             Object.keys(d).length",
            "\"function,undefined,true,4\"",
        ),
        ("Object.getOwnPropertyDescriptor({}, 'x')", "undefined"),
        ("Object.getOwnPropertyDescriptor('ab', 1).value", "\"b\""),
        ("Object.defineProperty(1, 'x', {})", "TypeError"),
        ("Object.defineProperty({}, 'x', 1)", "TypeError"),
        ("Object.defineProperty({}, 'x', {get: 1})", "TypeError"),
        (
            "Object.defineProperty({}, 'x', {get: function () {}, value: 1})",
            "TypeError",
        ),
        (
            "var o = Object.defineProperties({}, \
                 {a: {value: 1, enumerable: true}, b: {get: function () { return 2; }}}); \
             o.a + o.b",
            "3",
        ),
        ("Object.defineProperties(1, {})", "TypeError"),
        (
            "Object.entries({a: 1})[0][0] + Object.entries({a: 1})[0][1]",
            "\"a1\"",
        ),
        ("Object.values({a: 1, b: 2})[1]", "2"),
        ("Object.keys([3, 4])[1]", "\"1\""),
        ("typeof Object.keys([3])[0]", "\"string\""),
        ("Object.keys({[Symbol.iterator]: 1, a: 2}).length", "1"),
        ("Object.keys('ab').length", "2"),
        ("Object.keys(null)", "TypeError"),
        ("Object.isFrozen(Object.freeze({a: 1}))", "true"),
        ("Object.isFrozen({})", "false"),
        ("Object.isFrozen(Object.preventExtensions({}))", "true"),
        ("Object.isSealed(Object.seal({a: 1}))", "true"),
        ("Object.isFrozen(Object.seal({a: 1}))", "false"),
        ("Object.isSealed(Object.freeze({get a() {}}))", "true"),
        ("Object.isExtensible(Object.preventExtensions({}))", "false"),
        ("Object.isExtensible(1)", "false"),
        ("Object.isFrozen(1)", "true"),
        ("Object.freeze(1)", "1"),
        ("Object.preventExtensions(1)", "1"),
        (
            "'use strict'; var o = Object.freeze({a: 1}); o.a = 2;",
            "TypeError",
        ),
        ("var o = Object.freeze({a: 1}); o.a = 2; o.a", "1"),
        ("var o = Object.seal({a: 1}); o.a = 2; delete o.a; o.a", "2"),
        ("Object.fromEntries([['a', 1], ['b', 2]]).b", "2"),
        ("Object.keys(Object.fromEntries([])).length", "0"),
        ("Object.fromEntries([1])", "TypeError"),
        ("Object.fromEntries(null)", "TypeError"),
        ("Object.getOwnPropertyDescriptors({a: 1}).a.value", "1"),
        ("Object.getOwnPropertyNames([1]).length", "2"),
        (
            "Object.getOwnPropertyNames({[Symbol.iterator]: 1, a: 2}).length",
            "1",
        ),
        (
            "Object.getOwnPropertySymbols({[Symbol.iterator]: 1, a: 2})[0] === \
             Symbol.iterator",
            "true",
        ),
        ("Object.getPrototypeOf({}) === Object.prototype", "true"),
        ("Object.getPrototypeOf(null)", "TypeError"),
        (
            "var g = Object.groupBy([1, 2, 3], function (x) { return x % 2 ? 'odd' : 'even'; }); \
             g.odd.length + ',' + g.even[0] + ',' + Object.getPrototypeOf(g)",
            "\"2,2,null\"",
        ),
        (
            "var g = Object.groupBy('aba', function (x, i) { return i; }); Object.keys(g)[2]",
            "\"2\"",
        ),
        ("Object.groupBy(null, function () {})", "TypeError"),
        ("Object.groupBy([], 1)", "TypeError"),
        ("Object.hasOwn({a: 1}, 'a')", "true"),
        ("Object.hasOwn({}, 'toString')", "false"),
        (
            "Object.is(NaN, NaN) + ',' + Object.is(0, -0) + ',' + Object.is('a', 'a')",
            "\"true,false,true\"",
        ),
        ("var p = {x: 1}; Object.setPrototypeOf({}, p).x", "1"),
        ("Object.setPrototypeOf(1, {})", "1"),
        ("Object.setPrototypeOf({}, 1)", "TypeError"),
        ("Object.setPrototypeOf(null, {})", "TypeError"),
        (
            "Object.setPrototypeOf(Object.preventExtensions({}), {})",
            "TypeError",
        ),
        // %Object.prototype% hat einen unveränderlichen Prototyp.
        (
            "Object.setPrototypeOf(Object.prototype, Object.create(null))",
            "TypeError",
        ),
        ("Object.setPrototypeOf(Object.prototype, {})", "TypeError"),
        (
            "Object.setPrototypeOf(Object.prototype, null) === Object.prototype \
             && Object.isExtensible(Object.prototype)",
            "true",
        ),
        (
            "Object.prototype.__proto__ = Object.create(null)",
            "TypeError",
        ),
        (
            "Object.prototype.__proto__ = null; Object.prototype.__proto__",
            "null",
        ),
        // Schlägt das Lesen eines Eintrags fehl, schließt fromEntries den
        // Iterator.
        (
            "var closed = ''; \
             function entries(entry) { return { [Symbol.iterator]() { return { \
               next() { return {done: false, value: entry}; }, \
               return() { closed += 'r'; return {}; } }; } }; } \
             var thrown = ''; \
             for (var e of [ {get 0() { throw 'k'; }}, {0: 'a', get 1() { throw 'v'; }}, 1 ]) { \
               try { Object.fromEntries(entries(e)); } catch (x) { thrown += x; } \
             } \
             closed + '/' + thrown",
            "\"rrr/kvTypeError\"",
        ),
    ]);
}

#[test]
fn object_prototype_methods_follow_the_spec() {
    check(&[
        ("({a: 1}).hasOwnProperty('a')", "true"),
        (
            "Object.prototype.hasOwnProperty.call('abc', 'length')",
            "true",
        ),
        (
            "Object.prototype.hasOwnProperty.call(null, 'x')",
            "TypeError",
        ),
        (
            "Object.prototype.isPrototypeOf(Object.create(null))",
            "false",
        ),
        ("Object.prototype.isPrototypeOf({})", "true"),
        ("({}).isPrototypeOf(1)", "false"),
        ("Object.prototype.isPrototypeOf.call(null, 1)", "false"),
        ("({a: 1}).propertyIsEnumerable('a')", "true"),
        ("({}).propertyIsEnumerable('toString')", "false"),
        (
            "({toString: function () { return 'x'; }}).toLocaleString()",
            "\"x\"",
        ),
        ("var o = {}; o.valueOf() === o", "true"),
        ("typeof Object.prototype.valueOf.call(1)", "\"object\""),
        ("Object.prototype.valueOf.call(undefined)", "TypeError"),
        ("({}).__proto__ === Object.prototype", "true"),
        ("var o = {}; o.__proto__ = {y: 2}; o.y", "2"),
        (
            "var o = {}; o.__proto__ = 1; Object.getPrototypeOf(o) === Object.prototype",
            "true",
        ),
        ("Object.prototype.__proto__", "null"),
        (
            "var d = Object.getOwnPropertyDescriptor(Object.prototype, '__proto__'); \
             d.get.name + ',' + d.set.name + ',' + d.enumerable + ',' + d.configurable",
            "\"get __proto__,set __proto__,false,true\"",
        ),
        (
            "Object.getOwnPropertyDescriptor(Object.prototype, '__proto__').set.call(1, {})",
            "undefined",
        ),
        (
            "Object.getOwnPropertyDescriptor(Object.prototype, '__proto__').set.call(null, {})",
            "TypeError",
        ),
        (
            "var o = {}; o.__defineGetter__('x', function () { return 3; }); o.x",
            "3",
        ),
        (
            "var o = {}; o.__defineSetter__('x', function (v) { this.y = v; }); o.x = 4; o.y",
            "4",
        ),
        (
            "var o = {get x() { return 1; }}; o.__lookupGetter__('x')()",
            "1",
        ),
        ("({}).__lookupGetter__('x')", "undefined"),
        (
            "Object.create({get x() { return 7; }}).__lookupGetter__('x')()",
            "7",
        ),
        ("({}).__lookupSetter__('toString')", "undefined"),
        (
            "var o = {set x(v) {}}; typeof o.__lookupSetter__('x') + typeof o.__lookupGetter__('x')",
            "\"functionundefined\"",
        ),
        ("({}).__defineGetter__('x', 1)", "TypeError"),
        ("Object.prototype.__defineGetter__.length", "2"),
    ]);
}

/// Führt `source` aus, dann alle Jobs, und liest danach die globale
/// Variable `r`. Scheitert das Script selbst, kommt sein Fehler.
fn settle(source: &str) -> String {
    let (mut agent, realm) = agent_with(false);
    define_tag(&mut agent, realm);
    let result = run(&mut agent, realm, source);
    if result.ends_with("Error") || result.starts_with("throw ") {
        return result;
    }
    agent.run_jobs().expect("Jobs");
    run(&mut agent, realm, "r")
}

#[test]
fn array_constructor_and_functions_follow_the_spec() {
    check(&[
        (
            "Array.length + ',' + Array.from.length + ',' + Array.of.length + ',' + \
             Array.isArray.length + ',' + Array.fromAsync.length",
            "\"1,1,0,1,1\"",
        ),
        ("Array.prototype.constructor === Array", "true"),
        (
            "Object.getPrototypeOf(Array.prototype) === Object.prototype",
            "true",
        ),
        (
            "Array.isArray(Array.prototype) + ',' + Array.prototype.length",
            "\"true,0\"",
        ),
        ("Array[Symbol.species] === Array", "true"),
        (
            "Object.getOwnPropertyDescriptor(Array, Symbol.species).get.name",
            "\"get [Symbol.species]\"",
        ),
        (
            "Array.prototype[Symbol.iterator] === Array.prototype.values",
            "true",
        ),
        (
            "var u = Array.prototype[Symbol.unscopables]; \
             (Object.getPrototypeOf(u) === null) + ',' + u.flat + ',' + u.toSorted + ',' + \
             Object.keys(u).length",
            "\"true,true,true,16\"",
        ),
        (
            "var d = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.unscopables); \
             d.writable + ',' + d.enumerable + ',' + d.configurable",
            "\"false,false,true\"",
        ),
        // Der Konstruktor (Abschnitt 23.1.1.1).
        ("Array().length + ',' + new Array().length", "\"0,0\""),
        (
            "new Array(3).length + ',' + Array(3).length + ',' + (0 in Array(3))",
            "\"3,3,false\"",
        ),
        ("Array('3').length + ',' + Array('3')[0]", "\"1,3\""),
        ("new Array(1.5)", "RangeError"),
        ("new Array(-1)", "RangeError"),
        ("new Array(4294967295).length", "4294967295"),
        ("new Array(4294967296)", "RangeError"),
        ("Array(1, 2).length + ',' + Array(1, 2)[1]", "\"2,2\""),
        (
            "class A extends Array {} var a = new A(2); \
             (a instanceof A) + ',' + a.length + ',' + Array.isArray(a)",
            "\"true,2,true\"",
        ),
        (
            "Array.isArray([]) + ',' + Array.isArray({}) + ',' + Array.isArray()",
            "\"true,false,false\"",
        ),
        // Array.from (Abschnitt 23.1.2.1).
        ("Array.from('abc').join('')", "\"abc\""),
        (
            "Array.from({length: 2, 0: 'x'}).join() + ',' + Array.from({length: 2}).length",
            "\"x,,2\"",
        ),
        ("Array.from([1, 2], x => x * 2).join()", "\"2,4\""),
        (
            "Array.from([1, 2], function (x, i) { return this.v + x + i; }, {v: 10}).join()",
            "\"11,13\"",
        ),
        ("Array.from([1], 1)", "TypeError"),
        ("Array.from(null)", "TypeError"),
        (
            "Array.from.call(function C(n) { this.n = n; }, {length: 3}).n",
            "3",
        ),
        (
            "Array.from.call(function C() { this.c = 1; }, [1, 2]).length",
            "2",
        ),
        ("Array.from.call(1, [1, 2]).length", "2"),
        (
            "var c = 0; var it = {[Symbol.iterator]() { return {next() { return {value: 1, done: false}; }, \
             return() { c++; return {}; }}; }}; \
             try { Array.from(it, () => { throw 1; }); } catch (e) { c += e * 10; } c",
            "11",
        ),
        // Array.of (Abschnitt 23.1.2.4).
        (
            "Array.of(7).length + ',' + Array.of(1, 2, 3).join()",
            "\"1,1,2,3\"",
        ),
        ("Array.of.call(function C(n) { this.n = n; }, 1, 2).n", "2"),
        ("Array.of.call(undefined, 1).length", "1"),
    ]);
}

#[test]
fn array_prototype_methods_follow_the_spec() {
    check(&[
        (
            "[Array.prototype.at.length, Array.prototype.concat.length, \
             Array.prototype.copyWithin.length, Array.prototype.slice.length, \
             Array.prototype.splice.length, Array.prototype.toSpliced.length, \
             Array.prototype.with.length, Array.prototype.flat.length, \
             Array.prototype.push.length, Array.prototype.unshift.length, \
             Array.prototype.reduce.length, Array.prototype.toLocaleString.length].join()",
            "\"1,1,2,2,2,2,2,0,1,1,1,0\"",
        ),
        (
            "[1, 2, 3].at(-1) + ',' + [1].at(5) + ',' + [1, 2].at(0.5)",
            "\"3,undefined,1\"",
        ),
        (
            "[1].concat([2, [3]], 4).length + ',' + [1].concat([2, [3]], 4)[2].length",
            "\"4,1\"",
        ),
        (
            "var o = {length: 1, 0: 'x', [Symbol.isConcatSpreadable]: true}; [].concat(o).join()",
            "\"x\"",
        ),
        (
            "var a = [1]; a[Symbol.isConcatSpreadable] = false; [].concat(a)[0] === a",
            "true",
        ),
        (
            "[1, , 3].concat([]).hasOwnProperty(1) + ',' + [1, , 3].concat([]).length",
            "\"false,3\"",
        ),
        ("[1, 2, 3, 4, 5].copyWithin(0, 3).join()", "\"4,5,3,4,5\""),
        (
            "[1, 2, 3, 4, 5].copyWithin(1, 3, 4).join()",
            "\"1,4,3,4,5\"",
        ),
        (
            "[1, 2, 3, 4, 5].copyWithin(-2, -3, -1).join()",
            "\"1,2,3,3,4\"",
        ),
        ("[1, 2, 3, 4, 5].copyWithin(2, 0).join()", "\"1,2,1,2,3\""),
        (
            "var a = [1, , 3]; a.copyWithin(0, 1); a.hasOwnProperty(0) + ',' + a[1]",
            "\"false,3\"",
        ),
        (
            "[...[7, 8].keys()].join() + ';' + [...['a'].entries()][0].join()",
            "\"0,1;0,a\"",
        ),
        (
            "[1, 2].every(x => x > 0) + ',' + [1, 2].some(x => x > 1) + ',' + \
             [].every(x => false) + ',' + [].some(x => true)",
            "\"true,true,true,false\"",
        ),
        ("var n = 0; [1, , 3].forEach(() => n++); n", "2"),
        (
            "[1].every(function () { return this.v; }, {v: true})",
            "true",
        ),
        ("[1].forEach(1)", "TypeError"),
        (
            "[1, 2, 3].fill(0, 1).join() + ';' + [1, 2, 3].fill(9, -1).join()",
            "\"1,0,0;1,2,9\"",
        ),
        ("new Array(2).fill('x').join()", "\"x,x\""),
        ("[1, 2, 3, 4].filter(x => x % 2).join()", "\"1,3\""),
        (
            "[1, , 3].map(x => x * 2).length + ',' + [1, , 3].map(x => x * 2).hasOwnProperty(1)",
            "\"3,false\"",
        ),
        (
            "[1, 2].map((x, i, a) => x + i + a.length).join()",
            "\"3,5\"",
        ),
        (
            "[1, 2, 3].find(x => x > 1) + ',' + [1, 2, 3].findIndex(x => x > 5) + ',' + \
             [1, 2, 3].findLast(x => x < 3) + ',' + [1, 2, 3].findLastIndex(x => x < 3)",
            "\"2,-1,2,1\"",
        ),
        ("var n = 0; [, 1].find(() => n++); n", "2"),
        (
            "[1, [2, [3, [4]]]].flat().length + ',' + [1, [2, [3, [4]]]].flat(Infinity).join()",
            "\"3,1,2,3,4\"",
        ),
        (
            "[1, [2, [3]]].flat(0).length + ',' + [[1], [2]].flat(-1).length + ',' + [1, , 3].flat().length",
            "\"2,2,2\"",
        ),
        (
            "[1, 2].flatMap(x => [x, x * 10]).join() + ',' + [1].flatMap(x => [[x]]).length",
            "\"1,10,2,20,1\"",
        ),
        ("[1].flatMap(1)", "TypeError"),
        (
            "[1, NaN].includes(NaN) + ',' + [1, NaN].indexOf(NaN)",
            "\"true,-1\"",
        ),
        (
            "[, undefined].includes(undefined, 0) + ',' + [, 1].indexOf(undefined)",
            "\"true,-1\"",
        ),
        (
            "[1, 2, 1].lastIndexOf(1) + ',' + [1, 2, 1].lastIndexOf(1, -2) + ',' + \
             [1, 2, 1].indexOf(1, 1) + ',' + [1, 2].lastIndexOf(2, -3)",
            "\"2,0,2,-1\"",
        ),
        (
            "[1, 2].indexOf(2, -5) + ',' + [1, 2].includes(2, 2) + ',' + [].includes()",
            "\"1,false,false\"",
        ),
        (
            "[1, null, undefined, 'a'].join() + ';' + [1, 2].join('-') + ';' + [[1, 2], 3].join(';')",
            "\"1,,,a;1-2;1,2;3\"",
        ),
        (
            "Array.prototype.join.call({length: 2, 0: 'a', 1: 'b'})",
            "\"a,b\"",
        ),
        (
            "var a = [1, 2]; a.push(3, 4) + ',' + a.join()",
            "\"4,1,2,3,4\"",
        ),
        (
            "var a = [1, 2]; a.pop() + ',' + a.length + ',' + [].pop()",
            "\"2,1,undefined\"",
        ),
        (
            "var a = [1, 2]; a.shift() + ',' + a.join() + ',' + [].shift()",
            "\"1,2,undefined\"",
        ),
        (
            "var a = [2]; a.unshift(0, 1) + ',' + a.join() + ',' + a.unshift()",
            "\"3,0,1,2,3\"",
        ),
        (
            "var o = {length: 0}; Array.prototype.push.call(o, 'x'); o.length + ',' + o[0]",
            "\"1,x\"",
        ),
        (
            "Array.prototype.push.call({length: 2 ** 53 - 1}, 1)",
            "TypeError",
        ),
        (
            "Array.prototype.unshift.call({length: 2 ** 53 - 1}, 1)",
            "TypeError",
        ),
        (
            "[1, 2, 3].reduce((a, b) => a + b) + ',' + [1, 2, 3].reduce((a, b) => a + b, 10)",
            "\"6,16\"",
        ),
        ("[].reduce((a, b) => a + b)", "TypeError"),
        (
            "[, , 3].reduce((a, b) => a + b) + ',' + [].reduce((a, b) => a + b, 'i')",
            "\"3,i\"",
        ),
        ("['a', 'b', 'c'].reduceRight((a, b) => a + b)", "\"cba\""),
        ("[1, 2, 3].reverse().join()", "\"3,2,1\""),
        (
            "var a = [1, , 3]; a.reverse(); a.join() + ',' + a.hasOwnProperty(1)",
            "\"3,,1,false\"",
        ),
        (
            "var o = {length: 2, 0: 'a'}; Array.prototype.reverse.call(o); o[1] + ',' + (0 in o)",
            "\"a,false\"",
        ),
        (
            "[1, 2, 3, 4].slice(1, -1).join() + ';' + [1, 2, 3].slice(-2).join()",
            "\"2,3;2,3\"",
        ),
        (
            "[1, , 3].slice(0).hasOwnProperty(1) + ',' + [1, 2].slice(5).length",
            "\"false,0\"",
        ),
        (
            "var a = [1, 2, 3, 4]; a.splice(1, 2, 'x').join() + ';' + a.join()",
            "\"2,3;1,x,4\"",
        ),
        (
            "var a = [1, 2, 3]; a.splice(1).join() + ';' + a.join()",
            "\"2,3;1\"",
        ),
        (
            "var a = [1, 2, 3]; a.splice().length + ',' + a.length",
            "\"0,3\"",
        ),
        (
            "var a = [1, 2]; a.splice(1, 0, 'a', 'b'); a.join()",
            "\"1,a,b,2\"",
        ),
        ("var a = [1, 2, 3]; a.splice(-1, 1); a.join()", "\"1,2\""),
        ("var a = [1, 2, 3]; a.splice(0, 2, 9); a.join()", "\"9,3\""),
        // `toLocaleString` von Number und String kommen mit Welle 8c; bis
        // dahin antwortet Object.prototype.toLocaleString.
        (
            "var o = {toLocaleString() { return 'L'; }}; [o, null, undefined, o].toLocaleString()",
            "\"L,,,L\"",
        ),
        (
            "[{toLocaleString() { return 1; }}, {toLocaleString: 2}].toLocaleString()",
            "TypeError",
        ),
        (
            "var a = [1, 2]; var b = a.toReversed(); a.join() + ';' + b.join()",
            "\"1,2;2,1\"",
        ),
        ("[1, , 3].toReversed().hasOwnProperty(1)", "true"),
        (
            "[1, 2, 3].toSpliced(1, 1, 'x', 'y').join() + ';' + [1, 2, 3].toSpliced().join()",
            "\"1,x,y,3;1,2,3\"",
        ),
        (
            "[1, 2, 3].toSpliced(1).join() + ';' + [1, 2].with(-1, 'z').join()",
            "\"1;1,z\"",
        ),
        ("[1].with(1, 0)", "RangeError"),
        ("var a = [1, 2]; a.with(0, 9); a.join()", "\"1,2\""),
        (
            "Array.prototype.toReversed.call({length: 2 ** 32})",
            "RangeError",
        ),
        (
            "[1, [2, 3]].toString() + ';' + String([1, 2])",
            "\"1,2,3;1,2\"",
        ),
        (
            "var a = [1]; a.join = 5; a.toString()",
            "\"[object Array]\"",
        ),
        (
            "Array.prototype.toString.call({}) + ';' + Array.prototype.toString.call({join() { return 'j'; }})",
            "\"[object Object];j\"",
        ),
        // ArraySpeciesCreate (Abschnitt 10.4.2.3).
        (
            "class A extends Array { static get [Symbol.species]() { return Array; } } \
             new A(1, 2).map(x => x) instanceof A",
            "false",
        ),
        (
            "class B extends Array {} new B(1, 2).filter(() => true) instanceof B",
            "true",
        ),
        (
            "var a = [1]; a.constructor = undefined; Array.isArray(a.slice())",
            "true",
        ),
        (
            "var a = [1]; a.constructor = {[Symbol.species]: null}; Array.isArray(a.map(x => x))",
            "true",
        ),
        ("var a = [1]; a.constructor = 1; a.map(x => x)", "TypeError"),
        (
            "var a = [1]; a.constructor = {[Symbol.species]: function (n) { this.n = n; }}; \
             a.map(x => x).n + ',' + a.slice().n + ',' + a.splice(0).n + ',' + a.concat().n",
            "\"1,1,1,0\"",
        ),
        // Große und unübliche Längen.
        (
            "Array.prototype.indexOf.call({length: 2 ** 53 - 1, [2 ** 53 - 2]: 'x'}, 'x', 2 ** 53 - 2)",
            "9007199254740990",
        ),
        (
            "var o = {length: 2 ** 32 + 2}; o[2 ** 32 + 1] = 'y'; Array.prototype.at.call(o, -1)",
            "\"y\"",
        ),
        (
            "Array.prototype.at.call({length: -5}, 0) + ',' + Array.prototype.join.call({length: '2'})",
            "\"undefined,,\"",
        ),
        (
            "Array.prototype.map.call('ab', c => c + c).join()",
            "\"aa,bb\"",
        ),
    ]);
}

#[test]
fn array_sorting_is_stable_and_follows_the_spec() {
    check(&[
        (
            "[10, 9, 1].sort().join() + ';' + [3, undefined, 1].sort().join()",
            "\"1,10,9;1,3,\"",
        ),
        (
            "[{k: 1, v: 'a'}, {k: 0, v: 'b'}, {k: 1, v: 'c'}].sort((x, y) => x.k - y.k).map(x => x.v).join('')",
            "\"bac\"",
        ),
        (
            "var a = [3, , 1]; a.sort(); a.length + ':' + a.join() + ':' + (2 in a)",
            "\"3:1,3,:false\"",
        ),
        (
            "var a = [3, , 1]; a.toSorted().join() + ',' + a.toSorted().hasOwnProperty(2)",
            "\"1,3,,true\"",
        ),
        ("[2, 1].sort(() => { throw 7; })", "throw 7"),
        ("[].sort(1)", "TypeError"),
        ("[].toSorted(1)", "TypeError"),
        ("var a = [1, 2]; a.sort() === a", "true"),
        (
            "[5, 1, 4].sort((x, y) => y - x).join() + ';' + [1, 2].sort(() => NaN).join()",
            "\"5,4,1;1,2\"",
        ),
        (
            "var o = {length: 3, 0: 'c', 1: 'a', 2: 'b'}; Array.prototype.sort.call(o); o[0] + o[1] + o[2]",
            "\"abc\"",
        ),
        (
            "var s = ''; for (var i = 0; i < 40; i++) s += i % 7; \
             var a = Array.from(s); a.sort(); a.join('')",
            "\"0000001111112222223333334444445555566666\"",
        ),
        (
            "[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].sort().join()",
            "\"1,10,2,3,4,5,6,7,8,9\"",
        ),
        (
            "Array.prototype.toSorted.call({length: 2 ** 32})",
            "RangeError",
        ),
    ]);
}

#[test]
fn array_from_async_awaits_each_element() {
    for (source, expected) in [
        ("var r = Array.fromAsync([]) instanceof Promise;", "true"),
        (
            "var r; Array.fromAsync([1, Promise.resolve(2), 3]).then(a => r = a.join());",
            "\"1,2,3\"",
        ),
        (
            "var r; Array.fromAsync([1, 2], x => x * 2).then(a => r = a.join());",
            "\"2,4\"",
        ),
        (
            "var r; Array.fromAsync([1, 2], function (x, i) { return this.v + x + i; }, {v: 10})\
             .then(a => r = a.join());",
            "\"11,13\"",
        ),
        (
            "var r; async function* g() { yield 1; yield Promise.resolve(2); } \
             Array.fromAsync(g()).then(a => r = a.join());",
            "\"1,2\"",
        ),
        (
            "var r; Array.fromAsync({length: 2, 0: 'a', 1: Promise.resolve('b')}).then(a => r = a.join());",
            "\"a,b\"",
        ),
        (
            "var r; Array.fromAsync([], 1).then(() => r = 'ok', e => r = e instanceof TypeError);",
            "true",
        ),
        (
            "var r; Array.fromAsync(null).then(() => r = 'ok', e => r = e instanceof TypeError);",
            "true",
        ),
        (
            "var r; Array.fromAsync([Promise.reject(5)]).then(null, e => r = e);",
            "5",
        ),
        (
            "var closed = false; var it = {[Symbol.iterator]() { return {next() { return {value: 1, done: false}; }, \
             return() { closed = true; return {}; }}; }}; var r; \
             Array.fromAsync(it, () => { throw 1; }).then(null, e => r = e + ',' + closed);",
            "\"1,true\"",
        ),
        // Wirft schon PromiseResolve im Await auf das Ergebnis von `mapper`,
        // schließt das den Iterator.
        (
            "var closed = false; var p = Promise.resolve(1); \
             Object.defineProperty(p, 'constructor', {get() { throw new RangeError(); }}); \
             var it = {[Symbol.asyncIterator]() { return {next() { return {value: 1, done: false}; }, \
             return() { closed = true; return {}; }}; }}; var r; \
             Array.fromAsync(it, () => p).then(null, e => r = (e instanceof RangeError) + ',' + closed);",
            "\"true,true\"",
        ),
        // Wirft es im Await auf das Ergebnis von `return`, bleibt es bei der
        // Ausnahme aus `mapper`.
        (
            "var q = Promise.resolve({}); \
             Object.defineProperty(q, 'constructor', {get() { throw new TypeError(); }}); \
             var it = {[Symbol.asyncIterator]() { return {next() { return {value: 1, done: false}; }, \
             return() { return q; }}; }}; var r; \
             Array.fromAsync(it, () => { throw 7; }).then(null, e => r = e);",
            "7",
        ),
        (
            "var r; function C() { this.c = true; } \
             Array.fromAsync.call(C, [1]).then(a => r = a.c + ',' + a.length + ',' + a[0]);",
            "\"true,1,1\"",
        ),
        (
            "var r; var order = []; Array.fromAsync({length: 1, get 0() { order.push('get'); return 1; }})\
             .then(a => r = order.join() + ';' + a.join()); order.push('sync');",
            "\"get,sync;1\"",
        ),
    ] {
        assert_eq!(settle(source), expected, "{source}");
    }
}

#[test]
fn function_prototype_methods_and_bound_functions_follow_the_spec() {
    check(&[
        (
            "Function.prototype.apply.length + Function.prototype.bind.length + \
             Function.prototype.toString.length",
            "3",
        ),
        (
            "(function (a, b) { return a + b; }).apply(null, [1, 2])",
            "3",
        ),
        (
            "typeof (function () { return this; }).apply(5)",
            "\"object\"",
        ),
        ("(function () { 'use strict'; return this; }).apply(5)", "5"),
        (
            "(function () { return arguments.length; }).apply(null, {length: 2})",
            "2",
        ),
        (
            "(function () { return arguments.length; }).apply(null, null)",
            "0",
        ),
        ("(function () {}).apply(null, 1)", "TypeError"),
        ("Function.prototype.apply.call(1)", "TypeError"),
        (
            "var b = (function (a, b) { return this.x + a + b; }).bind({x: 1}, 2); b(3)",
            "6",
        ),
        ("(function () {}).bind().name", "\"bound \""),
        (
            "function f(a, b, c) {} f.bind(null, 1).length + ',' + f.bind(null, 1).name",
            "\"2,bound f\"",
        ),
        ("function f(a, b, c) {} f.bind().length", "3"),
        ("(function () {}).bind(null, 1, 2).length", "0"),
        (
            "(function (a, b, c) {}).bind(null, 1).bind(null, 2).length",
            "1",
        ),
        (
            "var f = function () {}; Object.defineProperty(f, 'length', {value: 'x'}); \
             f.bind().length",
            "0",
        ),
        (
            "var f = function () {}; Object.defineProperty(f, 'length', {value: Infinity}); \
             f.bind().length",
            "Infinity",
        ),
        (
            "var f = function () {}; Object.defineProperty(f, 'length', {value: -Infinity}); \
             f.bind().length",
            "0",
        ),
        (
            "var f = function () {}; Object.defineProperty(f, 'name', {value: 1}); \
             f.bind().name",
            "\"bound \"",
        ),
        ("typeof (function () {}).bind()", "\"function\""),
        (
            "Object.getPrototypeOf((function () {}).bind()) === Function.prototype",
            "true",
        ),
        (
            "(function () {}).bind().hasOwnProperty('prototype')",
            "false",
        ),
        (
            "Object.prototype.toString.call((function () {}).bind())",
            "\"[object Function]\"",
        ),
        ("Function.prototype.bind.call(1)", "TypeError"),
        (
            "new ((function (a, b) { this.s = a + b; }).bind(null, 1))(2).s",
            "3",
        ),
        (
            "function F() { return new.target; } new (F.bind())() === F",
            "true",
        ),
        (
            "function F() {} var B = F.bind(); new F() instanceof B",
            "true",
        ),
        (
            "function F() {} var B = F.bind(); new B() instanceof F",
            "true",
        ),
        (
            "function F() {} var B = F.bind(); Object.getPrototypeOf(new B()) === F.prototype",
            "true",
        ),
        ("new ((() => {}).bind())()", "TypeError"),
        ("class D extends (function () {}).bind() {}", "TypeError"),
        (
            "function f(a, b, c) { return this.x + ':' + a + b + c; } \
             f.bind({x: 'A'}, 1).bind({x: 'B'}, 2)(3)",
            "\"A:123\"",
        ),
        (
            "function F(a, b) { this.v = a + b; this.t = new.target === F; } \
             var B = F.bind(null, 1).bind(null, 2); var o = new B(); o.v + ',' + o.t",
            "\"3,true\"",
        ),
        (
            "var create = Object.create.bind(null, null); Object.getPrototypeOf(create())",
            "null",
        ),
        (
            "var bound = (function () { return this; }).bind(1); \
             typeof bound() + typeof bound.call(2)",
            "\"objectobject\"",
        ),
        (
            "(function f(a) { return a; }).toString()",
            "\"function f(a) { return a; }\"",
        ),
        ("((a) => a).toString()", "\"(a) => a\""),
        ("({ m(x) { } }).m.toString()", "\"m(x) { }\""),
        (
            "Object.getOwnPropertyDescriptor({ get x() { return 1; } }, 'x').get.toString()",
            "\"get x() { return 1; }\"",
        ),
        (
            "(class A { constructor() {} m() {} }).toString()",
            "\"class A { constructor() {} m() {} }\"",
        ),
        ("(class B {}).toString()", "\"class B {}\""),
        (
            "class C extends Object {} C.toString()",
            "\"class C extends Object {}\"",
        ),
        ("class C { static m() {} } C.m.toString()", "\"m() {}\""),
        (
            "class C { static async * m() {} } C.m.toString()",
            "\"async * m() {}\"",
        ),
        (
            "(async function g() {}).toString()",
            "\"async function g() {}\"",
        ),
        ("(function* h() {}).toString()", "\"function* h() {}\""),
        (
            "Object.keys.toString()",
            "\"function keys() { [native code] }\"",
        ),
        (
            "Object.getOwnPropertyDescriptor(Object.prototype, '__proto__').get.toString()",
            "\"function get __proto__() { [native code] }\"",
        ),
        (
            "(function f() {}).bind().toString()",
            "\"function () { [native code] }\"",
        ),
        (
            "Function.prototype.toString()",
            "\"function () { [native code] }\"",
        ),
        (
            "Function.prototype[Symbol.hasInstance].toString()",
            "\"function [Symbol.hasInstance]() { [native code] }\"",
        ),
        ("Function.prototype.toString.call({})", "TypeError"),
        ("Function.prototype.toString.call(1)", "TypeError"),
        (
            "Function.prototype[Symbol.hasInstance].call(Object, {})",
            "true",
        ),
        (
            "Function.prototype[Symbol.hasInstance].call({}, {})",
            "false",
        ),
        (
            "Function.prototype[Symbol.hasInstance].call(function () {}, 1)",
            "false",
        ),
        (
            "function F() {} Function.prototype[Symbol.hasInstance].call(F.bind(), new F())",
            "true",
        ),
        (
            "Function.prototype[Symbol.hasInstance].name + \
             Function.prototype[Symbol.hasInstance].length",
            "\"[Symbol.hasInstance]1\"",
        ),
        (
            "var d = Object.getOwnPropertyDescriptor(Function.prototype, Symbol.hasInstance); \
             d.writable + ',' + d.enumerable + ',' + d.configurable",
            "\"false,false,false\"",
        ),
        ("typeof Function", "\"function\""),
        ("Function.prototype.constructor === Function", "true"),
        ("Function.length + ',' + Function.name", "\"1,Function\""),
        (
            "Object.getPrototypeOf(Function) === Function.prototype",
            "true",
        ),
        (
            "Function.prototype === Object.getPrototypeOf(function () {})",
            "true",
        ),
        ("Function('return 1')", "Unsupported(0)"),
        ("new Function()", "Unsupported(0)"),
        // Eine lange Kette gebundener Funktionen ohne Rekursion, auch mit
        // @@hasInstance aus %Function.prototype%.
        (
            "function F() {} var B = F; \
             for (var i = 0; i < 2000; i++) { B = B.bind(null); } \
             '' + (new F() instanceof B) + (new B() instanceof F) \
             + (B instanceof Function) + ({} instanceof B) \
             + Function.prototype[Symbol.hasInstance].call(B, new F())",
            "\"truetruetruefalsetrue\"",
        ),
        (
            "function F() {} \
             Object.defineProperty(F, Symbol.hasInstance, {value: function (v) { return v === 1; }}); \
             var B = F.bind(); '' + (1 instanceof B) + ({} instanceof B)",
            "\"truefalse\"",
        ),
        (
            "function F() {} F.prototype = 1; var B = F.bind().bind(); \
             ({}) instanceof B",
            "TypeError",
        ),
        (
            "var o = Object.create(Function.prototype); ({}) instanceof o",
            "false",
        ),
        // AddRestrictedFunctionProperties und %ThrowTypeError%.
        (
            "var c = Object.getOwnPropertyDescriptor(Function.prototype, 'caller'); \
             var a = Object.getOwnPropertyDescriptor(Function.prototype, 'arguments'); \
             '' + (c.get === c.set) + (c.get === a.get) + (c.get === a.set) \
             + c.enumerable + c.configurable + a.enumerable + a.configurable",
            "\"truetruetruefalsetruefalsetrue\"",
        ),
        (
            "var t = Object.getOwnPropertyDescriptor(Function.prototype, 'caller').get; \
             var l = Object.getOwnPropertyDescriptor(t, 'length'); \
             var n = Object.getOwnPropertyDescriptor(t, 'name'); \
             typeof t + t.length + '[' + t.name + ']' + l.writable + l.configurable \
             + n.writable + n.configurable + Object.isExtensible(t) + Object.isFrozen(t)",
            "\"function0[]falsefalsefalsefalsefalsetrue\"",
        ),
        (
            "Object.getOwnPropertyDescriptor(Function.prototype, 'caller').get()",
            "TypeError",
        ),
        ("(function () {}).caller", "TypeError"),
        ("(function () { 'use strict'; }).arguments", "TypeError"),
        ("(function () {}).caller = 1", "TypeError"),
        (
            "(function () { 'use strict'; return arguments.callee; })()",
            "TypeError",
        ),
        (
            "var d = (function () { 'use strict'; \
                return Object.getOwnPropertyDescriptor(arguments, 'callee'); })(); \
             d.get === Object.getOwnPropertyDescriptor(Function.prototype, 'caller').get \
             && d.get === d.set && !d.enumerable && !d.configurable",
            "true",
        ),
        (
            "var f = function () {}; Object.prototype.hasOwnProperty.call(f, 'caller') \
             || Object.prototype.hasOwnProperty.call(f, 'arguments')",
            "false",
        ),
    ]);
}

#[test]
fn number_formats_round_like_the_specification() {
    check(&[
        ("(1.005).toFixed(2)", "\"1.00\""),
        (
            "(0.5).toFixed(0) + (2.5).toFixed(0) + (-1.5).toFixed(0)",
            "\"13-2\"",
        ),
        ("(1e21).toFixed(2)", "\"1e+21\""),
        ("(0.000001).toFixed(7)", "\"0.0000010\""),
        ("(123.456).toExponential()", "\"1.23456e+2\""),
        ("(1.25).toExponential(1)", "\"1.3e+0\""),
        (
            "(0).toPrecision(3) + ' ' + (1e-7).toPrecision(2)",
            "\"0.00 1.0e-7\"",
        ),
        ("(123.456).toPrecision()", "\"123.456\""),
        (
            "(255).toString(16) + (-255).toString(36) + (0.5).toString(2)",
            "\"ff-730.1\"",
        ),
        ("(-0).toString(2)", "\"0\""),
        ("NaN.toFixed(200)", "RangeError"),
        ("NaN.toExponential(200)", "\"NaN\""),
        ("Infinity.toPrecision(0)", "\"Infinity\""),
        ("(1).toPrecision(0)", "RangeError"),
        ("(1).toString(37)", "RangeError"),
        ("(1).toString(undefined)", "\"1\""),
        ("Number.prototype.toString.call('1')", "TypeError"),
        ("Number.prototype.valueOf.call(new Number(4))", "4"),
        ("(12345.678).toLocaleString()", "\"12345.678\""),
        (
            "var log = ''; (1).toFixed({ valueOf: function () { log += 'f'; return 1; } }); log",
            "\"f\"",
        ),
    ]);
}

#[test]
fn number_converts_and_classifies() {
    check(&[
        ("Number('0x10') + Number('  12  ') + Number()", "28"),
        ("Number('1_0')", "NaN"),
        ("new Number(5) + 1", "6"),
        ("typeof new Number(5)", "\"object\""),
        (
            "Object.prototype.toString.call(new Number(5))",
            "\"[object Number]\"",
        ),
        (
            "Number.isInteger(5) && !Number.isInteger(5.5) && !Number.isInteger('5')",
            "true",
        ),
        (
            "Number.isSafeInteger(2 ** 53 - 1) && !Number.isSafeInteger(2 ** 53)",
            "true",
        ),
        ("Number.isNaN('x') || !Number.isNaN(NaN)", "false"),
        ("Number.isFinite('1')", "false"),
        (
            "Number.parseInt === parseInt && Number.parseFloat === parseFloat",
            "true",
        ),
        (
            "Number.MAX_SAFE_INTEGER === 2 ** 53 - 1 && Number.EPSILON === 2 ** -52 \
             && Number.MIN_VALUE === 5e-324 && Number.MAX_VALUE === 1.7976931348623157e308",
            "true",
        ),
        (
            "var d = Object.getOwnPropertyDescriptor(Number, 'NaN'); \
             d.writable || d.enumerable || d.configurable",
            "false",
        ),
        ("Number.prototype.valueOf()", "0"),
    ]);
}

#[test]
fn global_functions_parse_and_test_numbers() {
    check(&[
        ("parseInt('  -0x1F')", "-31"),
        ("parseInt('0x1F', 10)", "0"),
        ("parseInt('z', 36) + parseInt('11', 2)", "38"),
        ("1 / parseInt('-0')", "-Infinity"),
        ("parseInt('12', 1)", "NaN"),
        ("parseInt('9007199254740993')", "9007199254740992"),
        (
            "parseFloat('3.5e2px') + parseFloat('.5') + parseFloat('-.5e-1')",
            "350.45",
        ),
        ("parseFloat('Infinityx') + parseFloat('1e')", "Infinity"),
        ("parseFloat('x1')", "NaN"),
        (
            "var log = ''; parseInt({ toString: function () { log += 's'; return '7'; } }, \
             { valueOf: function () { log += 'r'; return 10; } }) + log",
            "\"7sr\"",
        ),
        ("isNaN('abc') && isNaN(undefined) && !isNaN('1')", "true"),
        (
            "isFinite('1') && !isFinite('Infinity') && !isFinite(NaN)",
            "true",
        ),
        ("isNaN(Symbol())", "TypeError"),
    ]);
}

#[test]
fn math_functions_follow_their_special_cases() {
    check(&[
        ("1 / Math.round(-0.5)", "-Infinity"),
        (
            "Math.round(2.5) + Math.round(-2.5) + Math.round(0.49999999999999994)",
            "1",
        ),
        ("Math.max()", "-Infinity"),
        ("Math.min()", "Infinity"),
        ("1 / Math.max(-0, 0) + 1 / Math.min(0, -0)", "NaN"),
        ("1 / Math.max(-0, 0)", "Infinity"),
        ("1 / Math.min(0, -0)", "-Infinity"),
        ("Math.max(1, NaN, 2)", "NaN"),
        (
            "var log = ''; Math.max(NaN, { valueOf: function () { log += 'v'; return 1; } }); log",
            "\"v\"",
        ),
        ("Math.hypot(3, 4) + Math.hypot() + Math.hypot(-2)", "7"),
        ("Math.hypot(NaN, Infinity)", "Infinity"),
        ("Math.hypot(1e300, 1e300, 1e300) > 1e300", "true"),
        ("Math.pow(1, Infinity)", "NaN"),
        ("Math.clz32(1) + Math.clz32(0)", "63"),
        ("Math.imul(0xffffffff, 5)", "-5"),
        ("1 / Math.sign(-0)", "-Infinity"),
        ("Math.sign(-3) + Math.sign(7)", "0"),
        ("Math.fround(5.5) + Math.fround(5.05)", "10.550000190734863"),
        ("Math.f16round(65519.99) + Math.f16round(65520)", "Infinity"),
        ("Math.f16round(65519.99)", "65504"),
        (
            "Math.f16round(2 ** -25) + Math.f16round(1.5 * 2 ** -25)",
            "5.960464477539063e-8",
        ),
        ("Math.f16round(1.0009765625 + 2 ** -12)", "1.0009765625"),
        (
            "Math.atan2(0, -0) === Math.PI && 1 / Math.atan2(-0, 0) === -Infinity",
            "true",
        ),
        ("Object.prototype.toString.call(Math)", "\"[object Math]\""),
        (
            "var ok = true; for (var i = 0; i < 100; i++) { var r = Math.random(); \
             if (!(r >= 0 && r < 1)) ok = false; } ok",
            "true",
        ),
    ]);
}

#[test]
fn math_sum_precise_rounds_once_and_checks_every_value() {
    check(&[
        ("Math.sumPrecise([0.1, 0.2])", "0.30000000000000004"),
        ("Math.sumPrecise([1e20, 0.1, -1e20])", "0.1"),
        ("Math.sumPrecise([1, 1e100, 1, -1e100])", "2"),
        ("1 / Math.sumPrecise([])", "-Infinity"),
        ("1 / Math.sumPrecise([-0, -0])", "-Infinity"),
        ("1 / Math.sumPrecise([-0, 0])", "Infinity"),
        ("Math.sumPrecise([Infinity, -Infinity])", "NaN"),
        (
            "Math.sumPrecise([1.7976931348623157e308, 1.7976931348623157e308])",
            "Infinity",
        ),
        ("Math.sumPrecise([NaN, 'x'])", "TypeError"),
        ("Math.sumPrecise()", "TypeError"),
        (
            "var closed = false; var it = { [Symbol.iterator]() { return { \
               next() { return { done: false, value: '1' }; }, \
               return() { closed = true; return {}; } }; } }; \
             try { Math.sumPrecise(it); } catch (e) { closed && e instanceof TypeError }",
            "true",
        ),
    ]);
}

#[test]
fn symbols_have_a_registry_and_prototype_methods() {
    check(&[
        ("Symbol.for('a') === Symbol.for('a')", "true"),
        ("Symbol.for('a') === Symbol('a')", "false"),
        ("Symbol.keyFor(Symbol.for('k'))", "\"k\""),
        ("Symbol.keyFor(Symbol('k'))", "undefined"),
        ("Symbol.keyFor(Symbol.iterator)", "undefined"),
        ("Symbol.keyFor('k')", "TypeError"),
        ("Symbol.for().description", "\"undefined\""),
        (
            "Symbol('x').description + Symbol().description",
            "\"xundefined\"",
        ),
        (
            "Symbol('x').toString() + Object(Symbol()).toString()",
            "\"Symbol(x)Symbol()\"",
        ),
        (
            "var s = Symbol(); Object(s).valueOf() === s && s[Symbol.toPrimitive]() === s",
            "true",
        ),
        ("Symbol.prototype.valueOf.call({})", "TypeError"),
        (
            "Object.getOwnPropertyDescriptor(Symbol.prototype, 'description').get.call(1)",
            "TypeError",
        ),
        (
            "Object.prototype.toString.call(Symbol())",
            "\"[object Symbol]\"",
        ),
        (
            "var d = Object.getOwnPropertyDescriptor(Symbol.prototype, Symbol.toPrimitive); \
             d.writable + ',' + d.configurable + ',' + d.value.name",
            "\"false,true,[Symbol.toPrimitive]\"",
        ),
        ("String(Symbol('s'))", "\"Symbol(s)\""),
    ]);
}

#[test]
fn the_symbol_registry_is_shared_by_realms_and_survives_collections() {
    for stress in [false, true] {
        let (mut agent, first) = agent_with(stress);
        let second = agent.create_realm().expect("Speicher");
        share(&mut agent, first, second, "other", "Symbol.for('shared')");
        assert_eq!(
            run(
                &mut agent,
                first,
                "for (var i = 0; i < 50; i++) Symbol.for('x' + i); \
                 Symbol.for('shared') === other && Symbol.keyFor(other) === 'shared'"
            ),
            "true"
        );
    }
}

#[test]
fn booleans_and_error_is_error() {
    check(&[
        (
            "Boolean('') || Boolean(0) || Boolean(NaN) || Boolean()",
            "false",
        ),
        ("Boolean(new Boolean(false))", "true"),
        ("typeof new Boolean(false)", "\"object\""),
        ("new Boolean(1).valueOf() === true", "true"),
        (
            "true.toString() + false.toString() + new Boolean(0).toString()",
            "\"truefalsefalse\"",
        ),
        ("Boolean.prototype.toString.call(1)", "TypeError"),
        ("Boolean.prototype.valueOf()", "false"),
        (
            "Object.prototype.toString.call(new Boolean(1))",
            "\"[object Boolean]\"",
        ),
        (
            "Error.isError(new TypeError()) && Error.isError(new AggregateError([]))",
            "true",
        ),
        (
            "Error.isError(Object.create(Error.prototype)) || Error.isError(Error)",
            "false",
        ),
        (
            "Error.isError({ __proto__: RangeError.prototype, message: '' })",
            "false",
        ),
        ("Error.isError()", "false"),
    ]);
}

#[test]
fn iterator_constructor_and_prototype_accessors() {
    check(&[
        (
            "try { Iterator() } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "try { new Iterator() } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "class Sub extends Iterator {} var m = new Sub(); \
             (Object.getPrototypeOf(Sub.prototype) === Iterator.prototype) && m instanceof Iterator",
            "true",
        ),
        ("Iterator.prototype.constructor === Iterator", "true"),
        ("Iterator.prototype[tag]", "\"Iterator\""),
        (
            "var d = Object.getOwnPropertyDescriptor(Iterator.prototype, 'constructor'); \
             typeof d.get + typeof d.set + d.enumerable + d.configurable",
            "\"functionfunctionfalsetrue\"",
        ),
        (
            "var o = Object.create(Iterator.prototype); o.constructor = 5; o[tag] = 'x'; \
             o.hasOwnProperty('constructor') + ':' + o.constructor + ':' + o[tag] + ':' + \
             (Iterator.prototype.constructor === Iterator)",
            "\"true:5:x:true\"",
        ),
        (
            "try { Iterator.prototype.constructor = 1; 'kein Fehler' } \
             catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "var d = Object.getOwnPropertyDescriptor(Iterator, 'prototype'); \
             d.writable || d.enumerable || d.configurable",
            "false",
        ),
        (
            "Object.prototype.toString.call([].values().map(function (x) { return x; }))",
            "\"[object Iterator Helper]\"",
        ),
        (
            "var n = 0; Iterator.prototype[Symbol.dispose].call({ return: function () { n++; } }); n",
            "1",
        ),
        ("Iterator.prototype[Symbol.dispose].call({})", "undefined"),
    ]);
}

#[test]
fn iterator_from_wraps_only_foreign_iterators() {
    check(&[
        ("var a = [1].values(); Iterator.from(a) === a", "true"),
        (
            "var f = Iterator.from({ next: function () { return { value: 7, done: false }; } }); \
             Object.getPrototypeOf(f) !== Iterator.prototype && f.take(2).toArray().join()",
            "\"7,7\"",
        ),
        ("Iterator.from('ab').toArray().join()", "\"a,b\""),
        (
            "try { Iterator.from(1) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "var f = Iterator.from({ next: function () {} }); f.return().done",
            "true",
        ),
        (
            "var f = Iterator.from({ next: function () {}, return: function () { return 'r'; } }); \
             f.return()",
            "\"r\"",
        ),
        (
            "var f = Iterator.from({ next: function () {} }); \
             try { f.next.call({}) } catch (e) { e instanceof TypeError }",
            "true",
        ),
    ]);
}

#[test]
fn iterator_helpers_are_lazy_and_count() {
    check(&[
        (
            "[1, 2, 3].values().map(function (x, i) { return x * 10 + i; }).toArray().join()",
            "\"10,21,32\"",
        ),
        (
            "[1, 2, 3, 4].values().filter(function (x) { return x % 2; }).toArray().join()",
            "\"1,3\"",
        ),
        (
            "[1, 2, 3].values().drop(1).take(1).toArray().join()",
            "\"2\"",
        ),
        (
            "[1, 2, 3].values().drop(Infinity).toArray().length + ':' + \
             [1, 2, 3].values().take(Infinity).toArray().length",
            "\"0:3\"",
        ),
        (
            "[[1, 2], [], [3]].values().flatMap(function (x) { return x; }).toArray().join()",
            "\"1,2,3\"",
        ),
        (
            "try { [1].values().flatMap(function () { return 'ab'; }).next() } \
             catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "var log = []; var h = [1, 2].values().map(function (x) { log.push(x); return x; }); \
             log.length + ':' + h.next().value + ':' + log.join()",
            "\"0:1:1\"",
        ),
        (
            "try { [1].values().map(1) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "try { [1].values().take(NaN) } catch (e) { e instanceof RangeError }",
            "true",
        ),
        (
            "try { [1].values().drop(-1) } catch (e) { e instanceof RangeError }",
            "true",
        ),
        (
            "try { Iterator.prototype.map.call(1, function () {}) } \
             catch (e) { e instanceof TypeError }",
            "true",
        ),
    ]);
}

#[test]
fn iterator_helpers_close_their_source() {
    let source = "var closed = 0; \
        function source() { \
          var s = Object.create(Iterator.prototype); \
          s.next = function () { return { value: 1, done: false }; }; \
          s.return = function () { closed++; return {}; }; \
          return s; \
        }";
    check(&[
        (
            &format!(
                "{source} var h = source().map(function (x) {{ return x; }}); h.return(); h.return(); closed"
            ),
            "1",
        ),
        (
            &format!("{source} var h = source().take(1); h.next(); h.next().done + ':' + closed"),
            "\"true:1\"",
        ),
        (
            &format!(
                "{source} var h = source().filter(function () {{ return true; }}); h.next(); h.return(); closed"
            ),
            "1",
        ),
        (
            &format!(
                "{source} try {{ source().map(function () {{ throw 1; }}).next() }} catch (e) {{}} closed"
            ),
            "1",
        ),
        (
            &format!("{source} try {{ source().map(1) }} catch (e) {{}} closed"),
            "1",
        ),
        (
            &format!("{source} source().some(function (x) {{ return x; }}) + ':' + closed"),
            "\"true:1\"",
        ),
        (
            &format!(
                "{source} var inner = source(); \
                var h = [0].values().flatMap(function () {{ return inner; }}); \
                h.next(); h.return(); closed"
            ),
            "1",
        ),
    ]);
}

#[test]
fn iterator_helpers_guard_their_state() {
    check(&[
        (
            "var it = [1, 2].values().map(function () { return it.next(); }); \
             try { it.next() } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "var it = [1].values().map(function (x) { return x; }); it.next(); it.next(); \
             it.next().done",
            "true",
        ),
        (
            "var h = [1].values().map(function (x) { return x; }); \
             try { h.next.call([].values()) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        // Eine Kette aus Helfern ruft sich in Rust auf; zu tief wird ein
        // RangeError, kein Überlauf des Stapels.
        (
            "var it = [1].values(); \
             for (var i = 0; i < 20000; i++) it = it.map(function (x) { return x; }); \
             try { it.next(); 'kein Fehler' } catch (e) { e instanceof RangeError }",
            "true",
        ),
    ]);
}

#[test]
fn iterator_chunks_and_windows() {
    check(&[
        (
            "[1, 2, 3, 4, 5].values().chunks(2).toArray().map(function (a) { return a.join('+'); }).join()",
            "\"1+2,3+4,5\"",
        ),
        (
            "[1, 2, 3, 4, 5].values().windows(3).toArray().map(function (a) { return a.join('+'); }).join()",
            "\"1+2+3,2+3+4,3+4+5\"",
        ),
        ("[1, 2].values().windows(3).toArray().length", "0"),
        (
            "[1, 2].values().windows(3, 'allow-partial').toArray()[0].join()",
            "\"1,2\"",
        ),
        (
            "var r = []; for (var i = 0; i < 40; i++) r.push(i); \
             r.values().windows(20).toArray()[20].join()",
            "\"20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39\"",
        ),
        (
            "try { [1].values().chunks(0) } catch (e) { e instanceof RangeError }",
            "true",
        ),
        (
            "try { [1].values().chunks(2 ** 32) } catch (e) { e instanceof RangeError }",
            "true",
        ),
        (
            "try { [1].values().windows(1.5) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "try { [1].values().windows('2') } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "try { [1].values().windows(2, 'x') } catch (e) { e instanceof TypeError }",
            "true",
        ),
    ]);
}

#[test]
fn iterator_eager_methods() {
    check(&[
        (
            "[1, 2, 3].values().every(function (x) { return x > 0; }) + ':' + \
             [1, 2, 3].values().some(function (x) { return x > 2; }) + ':' + \
             [1, 2, 3].values().find(function (x) { return x > 1; })",
            "\"true:true:2\"",
        ),
        (
            "var log = []; [1, 2].values().forEach(function (x, i) { log.push(x + '@' + i); }); \
             log.join()",
            "\"1@0,2@1\"",
        ),
        (
            "[1, 2, 3].values().reduce(function (a, b) { return a + b; })",
            "6",
        ),
        (
            "[1, 2].values().reduce(function (a, b, i) { return a + b + i; }, 10)",
            "14",
        ),
        (
            "[1].values().reduce(function (a, b) { return a + b; }, undefined)",
            "NaN",
        ),
        (
            "try { [].values().reduce(function () {}) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "[NaN].values().includes(NaN) + ':' + [1, 2, 3].values().includes(1, 1) + ':' + \
             [1, 2, 3].values().includes(3, Infinity) + ':' + [-0].values().includes(0)",
            "\"true:false:false:true\"",
        ),
        (
            "try { [1].values().includes(1, -1) } catch (e) { e instanceof RangeError }",
            "true",
        ),
        (
            "try { [1].values().includes(1, 0.5) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        ("[1, 2].values().toArray().join()", "\"1,2\""),
        (
            "[1, null, undefined, 'a'].values().join() + ':' + [1, 2].values().join(' - ') + \
             ':' + [].values().join('x') + ':' + [[1, 2]].values().join(';')",
            "\"1,,,a:1 - 2::1,2\"",
        ),
        (
            "var closed = 0; var it = { next: function () { return { value: 1 }; }, \
             return: function () { closed++; return {}; } }; \
             Object.setPrototypeOf(it, Iterator.prototype); \
             var bad = { toString: function () { throw 1; } }; \
             try { it.join(bad) } catch (e) { closed += e; } \
             var next = 0; it.next = function () { next++; return { value: bad }; }; \
             try { it.join() } catch (e) { closed += 10 * e; } closed + ':' + next",
            "\"13:1\"",
        ),
    ]);
}

#[test]
fn iterator_concat_zip_and_zip_keyed() {
    check(&[
        (
            "Iterator.concat([1, 2], [], [3]).toArray().join()",
            "\"1,2,3\"",
        ),
        ("Iterator.concat().next().done", "true"),
        (
            "try { Iterator.concat(1) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "try { Iterator.concat({}) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "Iterator.zip([[1, 2], [3, 4, 5]]).toArray().map(String).join('|')",
            "\"1,3|2,4\"",
        ),
        (
            "Iterator.zip([[1, 2], [3]], { mode: 'longest', padding: ['a', 'b'] }).toArray() \
             .map(String).join('|')",
            "\"1,3|2,b\"",
        ),
        (
            "try { Iterator.zip([[1, 2], [3]], { mode: 'strict' }).toArray() } \
             catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "Iterator.zip([[1], [2]], { mode: 'strict' }).toArray().length",
            "1",
        ),
        (
            "try { Iterator.zip([], { mode: 'x' }) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "try { Iterator.zip([], null) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "try { Iterator.zip([1]) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "var z = Iterator.zipKeyed({ a: [1], b: [2, 3] }, { mode: 'longest' }).toArray(); \
             z.length + ':' + z[1].a + ':' + z[1].b + ':' + (Object.getPrototypeOf(z[0]) === null)",
            "\"2:undefined:3:true\"",
        ),
        (
            "Object.keys(Iterator.zipKeyed({ b: [1], a: [2], c: undefined }).next().value).join()",
            "\"b,a\"",
        ),
        (
            "var closed = 0; var it = [1].values(); \
             it.return = function () { closed++; return {}; }; \
             Iterator.zip([[1, 2], it]).toArray(); closed",
            "0",
        ),
        (
            "var closed = 0; var it = [1, 2].values(); \
             it.return = function () { closed++; return {}; }; \
             Iterator.zip([[1], it]).toArray(); closed",
            "1",
        ),
    ]);
}

#[test]
fn json_parse_builds_values_and_revives() {
    check(&[
        (
            "var v = JSON.parse('{\"a\": [1, 2, {\"b\": null}], \"c\": \"d\", \"a\": 5}'); v.a + v.c",
            "\"5d\"",
        ),
        (
            "JSON.parse(' [1, \"2\", true, null, -0.5e1] ').join('|')",
            "\"1|2|true||-5\"",
        ),
        ("1 / JSON.parse('-0')", "-Infinity"),
        ("JSON.parse('\"\\\\u00e4\\\\n\"') === '\\u00e4\\n'", "true"),
        ("JSON.parse('{\"__proto__\": 1}').__proto__", "1"),
        (
            "Object.keys(JSON.parse('{\"b\": 1, \"a\": 2, \"b\": 3}')).join()",
            "\"b,a\"",
        ),
        ("JSON.parse(1)", "1"),
        (
            "JSON.parse({ toString: function () { return '[]'; } }).length",
            "0",
        ),
        (
            "try { JSON.parse(\"{'a': 1}\") } catch (e) { e instanceof SyntaxError }",
            "true",
        ),
        (
            "try { JSON.parse('[1,]') } catch (e) { e instanceof SyntaxError }",
            "true",
        ),
        (
            "try { JSON.parse('') } catch (e) { e instanceof SyntaxError }",
            "true",
        ),
        (
            "JSON.parse('{\"a\": [1, 2], \"b\": 3}', function (k, v) { \
               return typeof v === 'number' ? v + 1 : v; }).a.join()",
            "\"2,3\"",
        ),
        (
            "var log = []; JSON.parse('{\"a\": -1.50, \"b\": \"x\\\\u0041\", \"c\": [true]}', \
               function (k, v, c) { log.push(k + '=' + c.source); return v; }); log.join(' ')",
            "\"a=-1.50 b=\\\"x\\\\u0041\\\" 0=true c=undefined =undefined\"",
        ),
        (
            "var log = []; JSON.parse('[1, 2]', function (k, v, c) { \
               if (k === '0') this[1] = 3; log.push(k + '=' + c.source); return v; }); log.join(' ')",
            "\"0=1 1=undefined =undefined\"",
        ),
        (
            "var x = JSON.parse('[1, [2, 3]]', function (k, v) { return k === '0' ? undefined : v; }); \
             x.length + ':' + (0 in x) + ':' + JSON.stringify(x)",
            "\"2:false:[null,[null,3]]\"",
        ),
        (
            "var s = ''; for (var i = 0; i < 10000; i++) s += '['; \
             for (var i = 0; i < 10000; i++) s += ']'; \
             var v = JSON.parse(s); var d = 0; while (v.length) { v = v[0]; d++; } d",
            "9999",
        ),
    ]);
}

#[test]
fn json_stringify_serializes_iteratively() {
    check(&[
        (
            "JSON.stringify({ a: [1, 'x', null, true, undefined], b: { c: undefined, d: function () {} } })",
            "\"{\\\"a\\\":[1,\\\"x\\\",null,true,null],\\\"b\\\":{}}\"",
        ),
        (
            "JSON.stringify([1, [2]], null, 2)",
            "\"[\\n  1,\\n  [\\n    2\\n  ]\\n]\"",
        ),
        (
            "JSON.stringify({ a: 1 }, null, '--------------')",
            "\"{\\n----------\\\"a\\\": 1\\n}\"",
        ),
        (
            "JSON.stringify({ a: [] , b: {} }, null, new Number(1))",
            "\"{\\n \\\"a\\\": [],\\n \\\"b\\\": {}\\n}\"",
        ),
        (
            "JSON.stringify('\\u2028\\ud800\\udc00\\ud800\"\\\\\\n\\u0001')",
            "\"\\\"\\u{2028}\u{10000}\\\\ud800\\\\\\\"\\\\\\\\\\\\n\\\\u0001\\\"\"",
        ),
        (
            "var a = []; a[0] = a; try { JSON.stringify(a) } catch (e) { e instanceof TypeError }",
            "true",
        ),
        ("var a = [1]; JSON.stringify([a, a])", "\"[[1],[1]]\""),
        (
            "JSON.stringify({ a: 1, b: 2, c: 3, 1: 4 }, ['c', 'a', 1, 'c'])",
            "\"{\\\"c\\\":3,\\\"a\\\":1,\\\"1\\\":4}\"",
        ),
        (
            "JSON.stringify({ a: 1, b: [2] }, function (k, v) { return typeof v === 'number' ? v * 10 : v; })",
            "\"{\\\"a\\\":10,\\\"b\\\":[20]}\"",
        ),
        (
            "JSON.stringify({ toJSON: function (k) { return 'key:' + k; } })",
            "\"\\\"key:\\\"\"",
        ),
        (
            "JSON.stringify(new Number(3)) + JSON.stringify(new Boolean(false)) + JSON.stringify(NaN) + \
             JSON.stringify(-0)",
            "\"3falsenull0\"",
        ),
        (
            "JSON.stringify(undefined) === undefined && JSON.stringify(function () {}) === undefined",
            "true",
        ),
        ("JSON.stringify(Symbol())", "undefined"),
        (
            "var o = {}; var c = o; for (var i = 0; i < 10000; i++) { c.a = {}; c = c.a; } \
             JSON.stringify(o).length",
            "60002",
        ),
    ]);
}

/// Ein Baum, der nie als Werte ins Speicherbudget passt, endet schon im
/// Zerleger mit OutOfMemory, hier noch vor dem Komma am Ende, das sonst
/// einen SyntaxError ergäbe.
#[test]
fn json_parse_stays_within_the_memory_budget() {
    let mut agent = Agent::with_options(Options {
        limit: 4 * 1024 * 1024,
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    let source = "var small = '[' + Array(1001).join('0,') + '0]'; \
                  JSON.parse(small).length";
    assert_eq!(run(&mut agent, realm, source), "1001");
    let source = "try { JSON.parse('[' + Array(1001).join('0,') + ']') } \
                  catch (e) { e instanceof SyntaxError }";
    assert_eq!(run(&mut agent, realm, source), "true");
    // Der Text selbst passt ins Budget.
    let source = "var large = '[' + Array(600001).join('0,') + ']'; large.length";
    assert_eq!(run(&mut agent, realm, source), "1200002");
    assert_eq!(
        run(&mut agent, realm, "JSON.parse(large)"),
        "Abrupt(OutOfMemory)"
    );
}

/// `stringify` und `parse` mit *reviver* halten nur die Arrays und Objekte
/// fest, die gerade offen sind: Was fertig ist und sonst niemand erreicht,
/// kann die Speicherbereinigung freigeben. Zusammen bräuchten die 40
/// Arrays mit je 20.000 Elementen mehr als die 4 MiB des Heaps.
#[test]
fn json_releases_finished_containers() {
    let mut agent = Agent::with_options(Options {
        limit: 4 * 1024 * 1024,
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    let source = "JSON.stringify(Array(40), function (k, v) { \
                    if (k === '') return v; \
                    if (k === 'payload') return undefined; \
                    return { payload: Array(20000).fill(0) }; \
                  }).length";
    assert_eq!(run(&mut agent, realm, source), "121");
    let source = "var text = '[' + Array(40).join('{\"a\": {}}, ') + '{\"a\": {}}]'; \
                  JSON.parse(text, function (k, v) { \
                    if (k === 'a') return Array(20000).fill(0); \
                    if (k === '') return v; \
                    return undefined; \
                  }).length";
    assert_eq!(run(&mut agent, realm, source), "40");
}

#[test]
fn json_raw_values() {
    check(&[
        (
            "var r = JSON.rawJSON('123.40'); JSON.isRawJSON(r) + ':' + Object.isFrozen(r) + ':' + \
             Object.getPrototypeOf(r) + ':' + r.rawJSON + ':' + JSON.stringify({ n: r, m: [r] })",
            "\"true:true:null:123.40:{\\\"n\\\":123.40,\\\"m\\\":[123.40]}\"",
        ),
        (
            "JSON.isRawJSON({ rawJSON: '1' }) || JSON.isRawJSON(1) || JSON.isRawJSON()",
            "false",
        ),
        ("JSON.rawJSON('\"a\"').rawJSON", "\"\\\"a\\\"\""),
        ("JSON.rawJSON(null).rawJSON", "\"null\""),
        (
            "try { JSON.rawJSON('[1]') } catch (e) { e instanceof SyntaxError }",
            "true",
        ),
        (
            "try { JSON.rawJSON(' 1') } catch (e) { e instanceof SyntaxError }",
            "true",
        ),
        (
            "try { JSON.rawJSON('') } catch (e) { e instanceof SyntaxError }",
            "true",
        ),
        (
            "try { JSON.rawJSON('1a') } catch (e) { e instanceof SyntaxError }",
            "true",
        ),
        ("Object.prototype.toString.call(JSON)", "\"[object JSON]\""),
    ]);
}

#[test]
fn iterator_helpers_and_json_survive_collections() {
    let (mut agent, realm) = agent_with(true);
    define_tag(&mut agent, realm);
    let source = "var h = [1, 2, 3, 4, 5].values().map(function (x) { return [x, String(x)]; }) \
        .flatMap(function (p) { return p; }).chunks(3); \
        var w = Iterator.zipKeyed({ a: h, b: [1, 2, 3].values().windows(2) }, { mode: 'longest' }) \
        .toArray(); \
        var text = JSON.stringify(w, null, 1); \
        var back = JSON.parse(text, function (k, v, c) { return c.source !== undefined ? k + c.source : v; }); \
        back.length + ':' + JSON.stringify(back[0]) + ':' + JSON.stringify(back[3]) + ':' + \
        Iterator.concat([], [7]).toArray().join() + ':' + \
        JSON.stringify(JSON.rawJSON('1e3'))";
    assert_eq!(
        run(&mut agent, realm, source),
        "\"4:{\\\"a\\\":[\\\"01\\\",\\\"1\\\\\\\"1\\\\\\\"\\\",\\\"22\\\"],\\\"b\\\":[\\\"01\\\",\\\"12\\\"]}:{\\\"a\\\":[\\\"0\\\\\\\"5\\\\\\\"\\\"]}:7:1e3\""
    );
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
}

#[test]
fn maps_keep_insertion_order_and_compare_with_same_value_zero() {
    check(&[
        (
            "var m = new Map([[1, 'a'], ['1', 'b'], [NaN, 'c']]); m.set(-0, 'd'); \
             m.size + ':' + m.get(1) + m.get('1') + m.get(NaN) + m.get(0) + ':' + \
             Object.is(Array.from(m.keys())[3], 0) + ':' + m.has(+0) + m.has(2)",
            "\"4:abcd:true:truefalse\"",
        ),
        (
            "var m = new Map([[1, 1], [2, 2], [3, 3]]); m.delete(2); m.set(2, 4); m.set(1, 5); \
             Array.from(m).join(';')",
            "\"1,5;3,3;2,4\"",
        ),
        (
            "var o = {}, s = Symbol(), m = new Map(); m.set(o, 1).set(s, 2).set({}, 3); \
             m.get(o) + ':' + m.get(s) + ':' + m.get({}) + ':' + m.delete(o) + m.delete(o) + ':' + m.size",
            "\"1:2:undefined:truefalse:2\"",
        ),
        ("Map()", "TypeError"),
        ("new Map([1])", "TypeError"),
        ("Map.prototype.get.call(new Set(), 1)", "TypeError"),
        ("Map.prototype.size", "TypeError"),
        (
            "var m = new Map(); m.getOrInsert('a', 1) + ':' + m.getOrInsert('a', 2) + ':' + \
             m.getOrInsertComputed('b', function (k) { m.set('b', 9); m.set('c', 0); return k + 3; }) + \
             ':' + Array.from(m).join(';')",
            "\"1:1:b3:a,1;b,b3;c,0\"",
        ),
        ("new Map().getOrInsertComputed(1, 2)", "TypeError"),
        (
            "var g = Map.groupBy([1, 2, 3, 4, -0, 0], function (x) { return x % 2; }); \
             Array.from(g.keys()).join() + ':' + g.get(1).join() + ':' + g.get(0).length",
            "\"1,0:1,3:4\"",
        ),
        (
            "Map.prototype[Symbol.iterator] === Map.prototype.entries && \
             Set.prototype[Symbol.iterator] === Set.prototype.values && \
             Set.prototype.keys === Set.prototype.values && Map[Symbol.species] === Map",
            "true",
        ),
        (
            "Object.prototype.toString.call(new Map().keys()) + \
             Object.prototype.toString.call(new Set().entries())",
            "\"[object Map Iterator][object Set Iterator]\"",
        ),
        (
            "class M extends Map { constructor(x) { super(x); this.extra = 1; } } \
             var m = new M([[1, 2]]); (m instanceof M) + ':' + m.get(1) + ':' + m.extra",
            "\"true:2:1\"",
        ),
    ]);
}

#[test]
fn sets_add_delete_and_iterate() {
    check(&[
        (
            "var s = new Set([1, 2, 2, NaN, NaN, -0]); s.add(3).add(1); \
             s.size + ':' + Array.from(s).join() + ':' + s.has(0) + s.has(4) + ':' + \
             s.delete(2) + s.delete(2) + ':' + Array.from(s.entries()).join(';')",
            "\"5:1,2,NaN,0,3:truefalse:truefalse:1,1;NaN,NaN;0,0;3,3\"",
        ),
        ("Set()", "TypeError"),
        ("Set.prototype.has.call(new Map(), 1)", "TypeError"),
        (
            "var added = []; var S = class extends Set { add(v) { added.push(v); return super.add(v); } }; \
             new S('ab'); added.join()",
            "\"a,b\"",
        ),
        (
            "var closed = 0; var it = { [Symbol.iterator]() { return { next() { return { value: 1, done: false }; }, \
             return() { closed++; return {}; } }; } }; \
             var S = class extends Set { add() { throw 7; } }; \
             try { new S(it); } catch (e) { e + ':' + closed }",
            "\"7:1\"",
        ),
    ]);
}

/// Iteratoren und `forEach` sehen, was während des Laufs dazukommt, und
/// überspringen, was vorher wegfällt, auch wenn die Tabelle dabei umzieht
/// oder geleert wird.
#[test]
fn collection_iteration_follows_changes() {
    check(&[
        (
            "var m = new Map([[1, 1], [2, 2], [3, 3]]); var seen = []; \
             m.forEach(function (v, k) { seen.push(k); if (k === 1) { m.delete(2); m.set(4, 4); } \
             if (k === 3) { m.delete(1); m.set(1, 1); } }); seen.join()",
            "\"1,3,4,1\"",
        ),
        (
            "var s = new Set([0]); var seen = []; \
             for (var x of s) { seen.push(x); if (x < 100) { s.delete(x); s.add(x + 1); } } \
             seen.length + ':' + seen[100] + ':' + s.size",
            "\"101:100:1\"",
        ),
        (
            "var s = new Set([1, 2, 3]); var it = s.values(); it.next(); s.clear(); s.add(7); \
             var r = it.next(); r.value + ':' + r.done + ':' + it.next().done + ':' + \
             (s.add(8), it.next().done)",
            "\"7:false:true:true\"",
        ),
        (
            "var m = new Map(); for (var i = 0; i < 20; i++) m.set(i, i); var it = m.keys(); \
             it.next(); it.next(); for (var i = 0; i < 18; i++) m.delete(i); \
             for (var i = 20; i < 60; i++) m.set(i, i); var rest = []; \
             for (var k of it) rest.push(k); rest.length + ':' + rest[0] + ':' + rest[41]",
            "\"42:18:59\"",
        ),
        (
            "var s = new Set([1]); var it = s[Symbol.iterator](); \
             Map.prototype.entries.call(new Map()).next.call(it)",
            "TypeError",
        ),
        (
            "var s = new Set([1, 2]); var a = s.values(), b = s.values(); a.next(); s.delete(1); \
             b.next().value + ':' + a.next().value",
            "\"2:2\"",
        ),
    ]);
}

#[test]
fn set_methods_follow_the_set_record() {
    check(&[
        (
            "var a = new Set([1, 2, 3]), b = new Set([3, 4]); \
             [a.union(b), a.intersection(b), a.difference(b), a.symmetricDifference(b)] \
             .map(function (s) { return Array.from(s).join(); }).join(';') + ':' + \
             a.isSubsetOf(b) + new Set([3]).isSubsetOf(b) + a.isSupersetOf(new Set([1, 3])) + \
             a.isDisjointFrom(new Set([5])) + a.isDisjointFrom(b)",
            "\"1,2,3,4;3;1,2;1,2,4:falsetruetruetruefalse\"",
        ),
        (
            "var calls = []; var other = { size: 1, has: function (v) { calls.push('has ' + v); return v === 2; }, \
             keys: function () { calls.push('keys'); return [2, 9].values(); } }; \
             var a = new Set([1, 2, 3]); \
             Array.from(a.intersection(other)).join() + ':' + calls.join()",
            "\"2:keys\"",
        ),
        (
            "var calls = []; var other = { size: 10, has: function (v) { calls.push(v); return v !== 2; }, \
             keys: function () { throw 1; } }; \
             var a = new Set([1, 2, 3]); \
             Array.from(a.intersection(other)).join() + ':' + calls.join()",
            "\"1,3:1,2,3\"",
        ),
        (
            "new Set().union({ size: undefined, has() {}, keys() {} })",
            "TypeError",
        ),
        (
            "new Set().union({ size: -1, has() {}, keys() {} })",
            "RangeError",
        ),
        (
            "new Set().union({ size: 1, has: 1, keys() {} })",
            "TypeError",
        ),
        ("new Set().union([1])", "TypeError"),
        (
            "var closed = 0; var other = { size: 1, has() { return true; }, keys() { \
             return { next() { return { value: 1, done: false }; }, return() { closed++; return {}; } }; } }; \
             new Set([1, 2]).isDisjointFrom(other) + ':' + new Set([2, 3]).isSupersetOf(other) + ':' + closed",
            "\"false:false:2\"",
        ),
        (
            "var a = new Set([1, 2, 3]); var other = { size: Infinity, has(v) { if (v === 1) a.add(4); return true; }, \
             keys() {} }; a.isSubsetOf(other) + ':' + Array.from(a.difference(other)).join()",
            "\"true:\"",
        ),
        (
            "var s = new Set([-0]).union({ size: 1, has() {}, keys() { return [-0, 0, NaN].values(); } }); \
             Object.is(Array.from(s)[0], 0) + ':' + s.size",
            "\"true:2\"",
        ),
    ]);
}

/// Objekte und Symbole als Schlüssel bleiben auffindbar, auch wenn der
/// Collector sie verschiebt.
#[test]
fn collections_find_moved_keys() {
    let (mut agent, realm) = agent_with(false);
    let source = "var keys = []; var m = new Map(); var s = new Set(); \
                  for (var i = 0; i < 300; i++) { var k = i % 3 ? {} : Symbol(); keys.push(k); \
                  m.set(k, i); s.add(k); if (i % 7 === 0) s.add('t' + i); } \
                  m.size + s.size";
    assert_eq!(run(&mut agent, realm, source), "643");
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
    let source = "keys.every(function (k, i) { return m.get(k) === i && s.has(k); }) && \
                  !m.has({}) && s.has('t7')";
    assert_eq!(run(&mut agent, realm, source), "true");
    agent.heap_mut().collect();
    let source = "for (var i = 0; i < 300; i += 2) { m.delete(keys[i]); s.delete(keys[i]); } \
                  for (var i = 0; i < 300; i++) m.set({}, i); \
                  keys.filter(function (k) { return m.has(k); }).length + ':' + m.size + ':' + s.size";
    assert_eq!(run(&mut agent, realm, source), "\"150:450:193\"");

    let (mut agent, realm) = agent_with(true);
    let source = "var m = new Map(); var keys = []; \
                  for (var i = 0; i < 100; i++) { var k = [i]; keys.push(k); m.set(k, i); } \
                  var it = m.values(); it.next(); m.delete(keys[1]); \
                  keys.every(function (k, i) { return i === 1 ? !m.has(k) : m.get(k) === i; }) + ':' + \
                  it.next().value + ':' + Array.from(Map.groupBy(keys, function (k) { return k[0] % 2; }).keys()).join()";
    assert_eq!(run(&mut agent, realm, source), "\"true:2:0,1\"");
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
}

/// Gelöschte Einträge und `clear` geben ihren Platz frei: Bleibt die
/// Größe fast gleich, bleibt auch die Tabelle klein, über beliebig viele
/// Einträge hinweg.
#[test]
fn collections_reuse_their_space() {
    let (mut agent, realm) = agent_with(false);
    run(&mut agent, realm, "var m = new Map(); var s = new Set();");
    agent.heap_mut().collect();
    let before = agent.heap().used_bytes();
    let source = "for (var i = 0; i < 12000; i++) { m.set(i, i); m.delete(i - 10); } \
                  for (var r = 0; r < 3; r++) { for (var i = 0; i < 4000; i++) s.add(i); s.clear(); } \
                  m.size + ':' + m.keys().next().value + ':' + s.size";
    assert_eq!(run(&mut agent, realm, source), "\"10:11990:0\"");
    agent.heap_mut().collect();
    // Ohne Umzug bräuchten die 12.000 Einträge 384 KiB.
    let after = agent.heap().used_bytes();
    assert!(after < before + 16 * 1024, "{before} → {after}");

    let mut agent = Agent::with_options(Options {
        limit: 4 * 1024 * 1024,
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    assert_eq!(
        run(&mut agent, realm, "new Set(Array(200000).keys()).size"),
        "Abrupt(OutOfMemory)"
    );
    assert_eq!(
        run(&mut agent, realm, "new Set(Array(1000).keys()).size"),
        "1000"
    );
}

#[test]
fn proxies_forward_to_their_target_or_ask_the_handler() {
    check(&[
        (
            "var t = {a: 1}; var p = new Proxy(t, {}); p.b = 2; delete p.a; \
             [t.b, 'a' in t, Object.keys(p).join(), typeof p].join()",
            "\"2,false,b,object\"",
        ),
        (
            "var p = new Proxy({}, { get: function (t, k, r) { return k + (r === p); } }); p.x",
            "\"xtrue\"",
        ),
        (
            "var log = []; var h = { has: function (t, k) { log.push(k); return k === 'x'; } }; \
             var p = new Proxy({}, h); [('x' in p), ('y' in p), Reflect.has(p, 1), log.join()].join()",
            "\"true,false,false,x,y,1\"",
        ),
        (
            "var d; var p = new Proxy({}, { defineProperty: function (t, k, desc) { \
             d = desc; return Reflect.defineProperty(t, k, desc); } }); p.x = 1; \
             [d.value, d.writable, d.enumerable, d.configurable, p.x].join()",
            "\"1,true,true,true,1\"",
        ),
        (
            "var p = new Proxy({a: 1, b: 2}, { ownKeys: function () { return ['b', 'a', 'c']; } }); \
             var s = ''; for (var k in p) s += k; s + ':' + Object.keys(p).join()",
            "\"ba:b,a\"",
        ),
        (
            "var p = new Proxy({}, { getPrototypeOf: function () { return Array.prototype; } }); \
             [p instanceof Array, Array.isArray(p)].join()",
            "\"true,false\"",
        ),
        (
            "var p = new Proxy({}, { deleteProperty: function () { return false; } }); \
             (function () { 'use strict'; try { delete p.x; return 'kein Fehler'; } \
             catch (e) { return e instanceof TypeError; } })()",
            "true",
        ),
        (
            "var log = []; var p = new Proxy({}, { set: function (t, k, v, r) { log.push(r === o); \
             return Reflect.set(t, k, v, r); } }); var o = Object.create(p); o.y = 1; \
             [log.join(), o.hasOwnProperty('y'), Object.keys(p).length].join()",
            "\"true,true,0\"",
        ),
        (
            "var o = Object.create(new Proxy({}, { get: function (t, k) { return 'aus ' + String(k); } })); \
             o.x",
            "\"aus x\"",
        ),
    ]);
}

#[test]
fn proxy_traps_keep_the_invariants_of_their_target() {
    let throws = |body: &str| {
        format!("try {{ {body}; 'kein Fehler' }} catch (e) {{ e instanceof TypeError }}")
    };
    let cases = [
        "var t = Object.freeze({x: 1}); new Proxy(t, { get: function () { return 2; } }).x",
        "var t = {}; Object.defineProperty(t, 'x', { get: undefined }); \
         new Proxy(t, { get: function () { return 1; } }).x",
        "var t = Object.freeze({x: 1}); 'x' in new Proxy(t, { has: function () { return false; } })",
        "var t = Object.preventExtensions({x: 1}); \
         'x' in new Proxy(t, { has: function () { return false; } })",
        "var t = Object.freeze({x: 1}); new Proxy(t, { set: function () { return true; } }).x = 2",
        "var t = {}; Object.defineProperty(t, 'x', { value: 1 }); \
         delete new Proxy(t, { deleteProperty: function () { return true; } }).x",
        "var t = {}; Object.defineProperty(t, 'x', { value: 1 }); \
         Reflect.ownKeys(new Proxy(t, { ownKeys: function () { return []; } }))",
        "Reflect.ownKeys(new Proxy({}, { ownKeys: function () { return ['a', 'a']; } }))",
        "Reflect.ownKeys(new Proxy({}, { ownKeys: function () { return [1]; } }))",
        "var t = Object.preventExtensions({}); \
         Reflect.ownKeys(new Proxy(t, { ownKeys: function () { return ['a']; } }))",
        "Object.getOwnPropertyDescriptor(new Proxy({}, { getOwnPropertyDescriptor: function () { \
         return { value: 1, configurable: false }; } }), 'x')",
        "Object.getOwnPropertyDescriptor(new Proxy({}, { getOwnPropertyDescriptor: function () { \
         return 1; } }), 'x')",
        "Object.defineProperty(new Proxy({}, { defineProperty: function () { return true; } }), \
         'x', { value: 1, configurable: false })",
        "Object.isExtensible(new Proxy({}, { isExtensible: function () { return false; } }))",
        "Object.preventExtensions(new Proxy({}, { preventExtensions: function () { return true; } }))",
        "var t = Object.preventExtensions({}); \
         Object.getPrototypeOf(new Proxy(t, { getPrototypeOf: function () { return Array.prototype; } }))",
        "var t = Object.preventExtensions({}); Reflect.setPrototypeOf(new Proxy(t, { \
         setPrototypeOf: function () { return true; } }), Array.prototype)",
        "new (new Proxy(function () {}, { construct: function () { return 1; } }))()",
        "new Proxy({}, { get: 1 }).x",
        "new Proxy(1, {})",
        "Proxy({}, {})",
    ];
    let cases: Vec<(String, &str)> = cases.iter().map(|body| (throws(body), "true")).collect();
    let cases: Vec<(&str, &str)> = cases
        .iter()
        .map(|(source, expected)| (source.as_str(), *expected))
        .collect();
    check(&cases);
}

#[test]
fn proxies_call_construct_and_revoke() {
    check(&[
        (
            "var p = new Proxy(function (a, b) { return a + b; }, \
             { apply: function (t, th, args) { return t.apply(th, args) * 2; } }); \
             [p(1, 2), typeof p, typeof new Proxy({}, {}), p.length].join()",
            "\"6,function,object,2\"",
        ),
        (
            "var C = new Proxy(function () { this.x = 1; }, {}); var o = new C(); \
             [o.x, o instanceof C, Object.getPrototypeOf(o) === C.prototype].join()",
            "\"1,true,true\"",
        ),
        (
            "class A extends new Proxy(class { constructor() { this.a = new.target; } }, {}) {} \
             new A().a === A",
            "true",
        ),
        (
            "var log = []; var C = new Proxy(function () {}, { construct: function (t, args, nt) { \
             log.push(args.length, nt === C); return { made: true }; } }); new C(1, 2).made + ',' + log",
            "\"true,2,true\"",
        ),
        (
            "var f = new Proxy(function () { return this; }, {}); \
             var g = f.bind(7); [g() == 7, Function.prototype.toString.call(f)].join()",
            "\"true,function () { [native code] }\"",
        ),
        (
            "[Array.isArray(new Proxy([], {})), JSON.stringify(new Proxy([1, {a: 2}], {})), \
             Object.prototype.toString.call(new Proxy([], {})), \
             Object.prototype.toString.call(new Proxy(function () {}, {})), \
             [].concat(new Proxy([1, 2], {})).length].join()",
            "\"true,[1,{\\\"a\\\":2}],[object Array],[object Function],2\"",
        ),
        (
            "var r = Proxy.revocable({}, {}); var first = r.revoke(); r.revoke(); \
             var e; try { r.proxy.x; } catch (error) { e = error; } \
             [first, e instanceof TypeError, typeof r.proxy, r.revoke.name === '', r.revoke.length].join()",
            "\",true,object,true,0\"",
        ),
        (
            "var r = Proxy.revocable(function () {}, {}); r.revoke(); \
             var e; try { Array.isArray(Proxy.revocable([], {}).proxy) && (function () { \
             var s = Proxy.revocable([], {}); s.revoke(); return Array.isArray(s.proxy); })(); } \
             catch (error) { e = error; } [typeof r.proxy, e instanceof TypeError].join()",
            "\"function,true\"",
        ),
        (
            "var r = Proxy.revocable(function () {}, {}); r.revoke(); \
             try { Reflect.construct(Object, [], r.proxy); 'kein Fehler' } \
             catch (e) { e instanceof TypeError }",
            "true",
        ),
        (
            "class B { constructor(o) { return o; } } \
             class C extends B { #x = 1; static has(o) { return #x in o; } } \
             var p = new Proxy({}, {}); new C(p); [C.has(p), C.has({})].join()",
            "\"true,false\"",
        ),
        (
            "var p = new Proxy({}, {}); var o = {}; Object.setPrototypeOf(o, p); \
             Object.setPrototypeOf(p, o) && Object.getPrototypeOf(p) === o",
            "true",
        ),
    ]);
}

#[test]
fn reflect_calls_the_internal_methods() {
    check(&[
        (
            "Reflect.ownKeys({b: 1, a: 2, 1: 0, [Symbol.iterator]: 3}).map(String).join()",
            "\"1,b,a,Symbol(Symbol.iterator)\"",
        ),
        (
            "function F() { this.nt = new.target; } function G() {} var o = Reflect.construct(F, [], G); \
             [o.nt === G, Object.getPrototypeOf(o) === G.prototype, Reflect.apply(Math.max, null, [1, 3, 2])].join()",
            "\"true,true,3\"",
        ),
        (
            "var o = {}; Object.defineProperty(o, 'x', { value: 1 }); \
             [Reflect.defineProperty(o, 'x', { value: 2 }), Reflect.defineProperty(o, 'y', { value: 3 }), \
             Reflect.deleteProperty(o, 'x'), Reflect.deleteProperty(o, 'y')].join()",
            "\"false,true,false,false\"",
        ),
        (
            "var o = { get x() { return this.y; } }; var r = { y: 5 }; \
             [Reflect.get(o, 'x'), Reflect.get(o, 'x', r), Reflect.set(o, 'z', 1, r), r.z, o.z].join()",
            "\",5,true,1,\"",
        ),
        (
            "[Reflect.setPrototypeOf(Object.preventExtensions({}), {}), \
             Reflect.setPrototypeOf({}, null), Reflect.isExtensible(Object.seal({})), \
             Reflect.preventExtensions({}), Reflect.getPrototypeOf([]) === Array.prototype, \
             Reflect.getOwnPropertyDescriptor({x: 1}, 'x').writable, \
             Object.prototype.toString.call(Reflect)].join()",
            "\"false,true,false,true,true,true,[object Reflect]\"",
        ),
        (
            "var e = []; for (var f of [function () { Reflect.getPrototypeOf(1); }, \
             function () { Reflect.construct(function () {}, [], Math.max); }, \
             function () { Reflect.apply({}, null, []); }, \
             function () { Reflect.setPrototypeOf({}, 1); }]) { \
             try { f(); e.push('kein Fehler'); } catch (error) { e.push(error instanceof TypeError); } } e.join()",
            "\"true,true,true,true\"",
        ),
    ]);
}

/// Die Objekte für den Handler entstehen im aktuellen Realm: dem des
/// Rahmens, dessen Befehl die interne Methode auslöst, oder dem der
/// eingebauten Funktion, die sie auslöst.
#[test]
fn proxies_create_objects_in_the_current_realm() {
    let (mut agent, realm) = agent_with(false);
    let other = agent.create_realm().expect("Speicher");
    agent.scope(|agent, scope| {
        let global = agent.global_object(scope, realm);
        let value = agent.global_object(scope, other);
        let key = agent
            .key_from_units(scope, &units("other"))
            .expect("Speicher");
        create_data_property_or_throw(agent, scope, global, key, value).expect("Eigenschaft");
    });
    let source = "var d; var p = new other.Proxy({}, { defineProperty: other.Reflect.defineProperty }); \
                  var q = new other.Proxy({}, { defineProperty: function (t, k, desc) { d = desc; return true; } }); \
                  p.a = 0; q.a = 0; \
                  var f = new other.Proxy(other.Object, { apply: other.Array.of }); \
                  [Object.getPrototypeOf(d) === Object.prototype, \
                  f()[2].constructor === Array, \
                  other.Array.prototype.map.call([1], f)[0][2].constructor === other.Array, \
                  [1].map(f)[0][2].constructor === Array].join()";
    assert_eq!(run(&mut agent, realm, source), "\"true,true,true,true\"");
}

/// Ketten von Proxys ohne Fallen laufen in Schleifen; ein Handler, der ein
/// Proxy mit einem Proxy als Handler ist, endet an der Grenze des Stapels.
#[test]
fn proxy_chains_run_on_a_small_stack() {
    let thread = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let (mut agent, realm) = agent_with(false);
            let source = "var p = {x: 1}; var f = function () { return 7; }; var a = []; \
                          for (var i = 0; i < 5000; i++) { p = new Proxy(p, {}); f = new Proxy(f, {}); \
                          a = new Proxy(a, {}); } \
                          var C = new Proxy(function () { this.y = 2; }, {}); \
                          for (var i = 0; i < 5000; i++) C = new Proxy(C, {}); \
                          [p.x, 'x' in p, f(), Array.isArray(a), new C().y, Object.keys(p).join()].join()";
            assert_eq!(run(&mut agent, realm, source), "\"1,true,7,true,2,x\"");
            let source = "var h = {}; for (var i = 0; i < 20000; i++) h = new Proxy({}, h); \
                          try { h.x; 'kein Fehler' } catch (e) { e instanceof RangeError }";
            assert_eq!(run(&mut agent, realm, source), "true");
            let source = "var o = {}; var p = new Proxy({}, {}); Object.setPrototypeOf(o, p); \
                          Object.setPrototypeOf(p, o); o.x";
            assert_eq!(run(&mut agent, realm, source), "Abrupt(Exhausted(Steps))");
        })
        .expect("Faden");
    assert!(thread.join().is_ok());
}

#[test]
fn proxies_survive_moving_collections() {
    let (mut agent, realm) = agent_with(true);
    let source = "var log = []; var t = {a: 1}; \
                  var p = new Proxy(t, { get: function (t, k, r) { log.push(String(k)); return Reflect.get(t, k, r); }, \
                  ownKeys: function (t) { return Reflect.ownKeys(t).concat(['b']); }, \
                  getOwnPropertyDescriptor: function (t, k) { return k === 'b' ? \
                  { value: 2, enumerable: true, configurable: true } : Reflect.getOwnPropertyDescriptor(t, k); } }); \
                  var f = new Proxy(function () { return [].slice.call(arguments); }, {}); \
                  JSON.stringify(p) + ':' + f(1, 2).join() + ':' + log.join()";
    assert_eq!(
        run(&mut agent, realm, source),
        "\"{\\\"a\\\":1}:1,2:toJSON,a,b\""
    );
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
}
