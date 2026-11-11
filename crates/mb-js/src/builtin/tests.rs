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
        assert_eq!(keys(agent, scope, prototype), ["length", "name", "call"]);
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
        e.cause[2] + e.message + String(e) + Object.prototype.toString.call(s) + \
        call.call(function (x) { 'use strict'; return x; }, 0, 'z') + String(tag) + \
        (Object(1).constructor === Object)";
    assert_eq!(
        run(&mut agent, realm, source),
        "\"cmTypeError: m[object String]zSymbol(Symbol.toStringTag)true\""
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
        assert_eq!(
            keys(agent, scope, constructor).len(),
            3 + WellKnownSymbol::ALL.len()
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
        assert_eq!(keys(agent, scope, iterator), ["Symbol"]);
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
