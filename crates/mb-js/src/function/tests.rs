use super::*;
use crate::heap::Options;
use crate::key::WellKnownSymbol;
use crate::object::{get_own_property, get_prototype_of, own_property_keys};
use crate::parser::parse_script;
use crate::realm::RealmId;

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

fn key<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> PropertyKey<'s> {
    agent.key_from_units(scope, &units(text)).expect("Speicher")
}

fn evaluate<'s>(agent: &mut Agent, scope: &'s Scope, realm: RealmId, source: &str) -> Local<'s> {
    match agent.evaluate_script(scope, realm, &units(source)) {
        Ok(value) => value,
        Err(error) => panic!("{source}: {error:?}"),
    }
}

fn text(agent: &Agent, value: Local<'_>) -> String {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::String(string) => String::from_utf16(&heap.string_units(string)).expect("UTF-16"),
        kind => format!("{kind:?}"),
    }
}

fn number(agent: &Agent, value: Local<'_>) -> Option<f64> {
    agent.heap().get(value).as_number()
}

fn bits(agent: &Agent, value: Local<'_>) -> u64 {
    agent.heap().get(value).bits()
}

/// Die eigenen Schlüssel als Text, in der Reihenfolge von
/// [[OwnPropertyKeys]].
fn keys(agent: &mut Agent, scope: &Scope, object: Local<'_>) -> Vec<String> {
    let keys = own_property_keys(agent, scope, object).expect("kein Fehler");
    keys.into_iter()
        .map(|key| match key {
            PropertyKey::Index(index) => index.to_string(),
            PropertyKey::String(name) => text(agent, name),
            PropertyKey::Symbol(_) => "Symbol".to_owned(),
        })
        .collect()
}

fn own<'s>(
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
    let property = own(agent, scope, object, name);
    let value = property.value.expect("Dateneigenschaft");
    (
        value,
        [
            property.writable,
            property.enumerable,
            property.configurable,
        ],
    )
}

fn function_name(agent: &mut Agent, scope: &Scope, function: Local<'_>) -> String {
    let (name, _) = data(agent, scope, function, "name");
    text(agent, name)
}

const FIXED: [Option<bool>; 3] = [Some(false), Some(false), Some(true)];

#[test]
fn functions_have_length_name_and_prototype() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        let function = evaluate(agent, scope, realm, "function f(a, b) {} f");
        assert_eq!(
            keys(agent, scope, function),
            ["length", "name", "prototype"]
        );
        let (length, flags) = data(agent, scope, function, "length");
        assert_eq!((number(agent, length), flags), (Some(2.0), FIXED));
        let (name, flags) = data(agent, scope, function, "name");
        assert_eq!((text(agent, name), flags), ("f".to_owned(), FIXED));
        let (prototype, flags) = data(agent, scope, function, "prototype");
        assert_eq!(flags, [Some(true), Some(false), Some(false)]);
        assert_eq!(keys(agent, scope, prototype), ["constructor"]);
        let (constructor, flags) = data(agent, scope, prototype, "constructor");
        assert_eq!(bits(agent, constructor), bits(agent, function));
        assert_eq!(flags, [Some(true), Some(false), Some(true)]);

        let object_prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
        let function_prototype = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
        let actual = get_prototype_of(agent, scope, prototype).expect("kein Fehler");
        assert_eq!(
            actual.map(|actual| bits(agent, actual)),
            Some(bits(agent, object_prototype))
        );
        let actual = get_prototype_of(agent, scope, function).expect("kein Fehler");
        assert_eq!(
            actual.map(|actual| bits(agent, actual)),
            Some(bits(agent, function_prototype))
        );
        // %Function.prototype% hat selbst einen Namen und eine Länge, dazu
        // bisher `call`.
        assert_eq!(
            keys(agent, scope, function_prototype),
            ["length", "name", "call"]
        );
        assert_eq!(function_name(agent, scope, function_prototype), "");
    });
}

/// Generatorfunktionen und Generator-Methoden (Abschnitte 15.5.3, 15.5.4
/// und 15.4.5): %GeneratorFunction.prototype% als Prototyp, ein
/// `prototype` aus %GeneratorPrototype% ohne `constructor` und kein
/// [[Construct]].
#[test]
fn generator_functions_have_a_generator_prototype() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        let generator_function =
            agent.intrinsic(scope, realm, Intrinsic::GeneratorFunctionPrototype);
        let generator = agent.intrinsic(scope, realm, Intrinsic::GeneratorPrototype);
        for (source, name) in [
            ("function* g(a, b = 1) {} g", "g"),
            ("var e = function* () {}; e", "e"),
            ("(function* own() {})", "own"),
            ("({ *m(a) {} }).m", "m"),
        ] {
            let function = evaluate(agent, scope, realm, source);
            assert_eq!(
                keys(agent, scope, function),
                ["length", "name", "prototype"],
                "{source}"
            );
            assert_eq!(function_name(agent, scope, function), name, "{source}");
            let actual = get_prototype_of(agent, scope, function).expect("kein Fehler");
            assert_eq!(
                actual.map(|actual| bits(agent, actual)),
                Some(bits(agent, generator_function)),
                "{source}"
            );
            let (prototype, flags) = data(agent, scope, function, "prototype");
            assert_eq!(flags, [Some(true), Some(false), Some(false)], "{source}");
            assert!(keys(agent, scope, prototype).is_empty(), "{source}");
            let actual = get_prototype_of(agent, scope, prototype).expect("kein Fehler");
            assert_eq!(
                actual.map(|actual| bits(agent, actual)),
                Some(bits(agent, generator)),
                "{source}"
            );
            assert!(
                !is_constructor(agent, agent.heap().get(function)),
                "{source}"
            );
        }
        let function = evaluate(agent, scope, realm, "g");
        let (length, _) = data(agent, scope, function, "length");
        assert_eq!(number(agent, length), Some(1.0));
    });
}

#[test]
fn the_length_ends_before_defaults_and_rest() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        for (source, expected) in [
            ("function f() {} f", 0.0),
            ("function f(a, b = 1, c) {} f", 1.0),
            ("function f(a, ...r) {} f", 1.0),
            ("function f([a], {b}) {} f", 2.0),
        ] {
            let function = evaluate(agent, scope, realm, source);
            let (length, _) = data(agent, scope, function, "length");
            assert_eq!(number(agent, length), Some(expected), "{source}");
        }
    });
}

#[test]
fn methods_and_accessors_have_no_prototype() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        let object = evaluate(
            agent,
            scope,
            realm,
            "({m(a) {}, get g() {}, set s(v) {}, get 1() {}, get [2 + 1]() {}})",
        );
        let (method, _) = data(agent, scope, object, "m");
        assert_eq!(keys(agent, scope, method), ["length", "name"]);
        assert_eq!(function_name(agent, scope, method), "m");
        for (property, expected, length) in [
            ("g", "get g", 0.0),
            ("s", "set s", 1.0),
            ("1", "get 1", 0.0),
            ("3", "get 3", 0.0),
        ] {
            let descriptor = own(agent, scope, object, property);
            assert_eq!(
                (descriptor.enumerable, descriptor.configurable),
                (Some(true), Some(true)),
                "{property}"
            );
            let accessor = if expected.starts_with("set") {
                descriptor.set
            } else {
                descriptor.get
            };
            let accessor = accessor.expect("Accessor");
            assert_eq!(
                keys(agent, scope, accessor),
                ["length", "name"],
                "{property}"
            );
            assert_eq!(function_name(agent, scope, accessor), expected);
            let (actual, _) = data(agent, scope, accessor, "length");
            assert_eq!(number(agent, actual), Some(length), "{property}");
        }
        // Getter und Setter mit demselben Namen teilen sich die Eigenschaft.
        let object = evaluate(agent, scope, realm, "({get a() {}, set a(v) {}})");
        let descriptor = own(agent, scope, object, "a");
        let getter = descriptor.get.expect("Getter");
        let setter = descriptor.set.expect("Setter");
        assert_eq!(function_name(agent, scope, getter), "get a");
        assert_eq!(function_name(agent, scope, setter), "set a");
    });
}

#[test]
fn symbols_name_functions_by_their_description() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        let ast = parse_script(&units("function f() {}")).expect("gültig");
        let Some(&Node::FunctionDeclaration(id)) = ast.body().first().map(|&item| ast.node(item))
        else {
            panic!("keine Funktion");
        };
        let unit = Rc::new(ast);
        let template = FunctionTemplate {
            function: id,
            binds_name: false,
            outer: Rc::default(),
        };
        let shared = agent
            .add_shared_function(Rc::clone(&unit), template)
            .expect("Speicher");
        let iterator = agent.well_known_symbol(scope, WellKnownSymbol::Iterator);
        let anonymous = agent.new_symbol(scope, None).expect("Speicher");
        let empty = agent.heap_mut().alloc_string(scope, &[]).expect("Speicher");
        let described = agent.new_symbol(scope, Some(empty)).expect("Speicher");
        for (name, expected) in [
            (PropertyKey::Symbol(iterator), "[Symbol.iterator]"),
            (PropertyKey::Symbol(anonymous), ""),
            (PropertyKey::Symbol(described), "[]"),
            (PropertyKey::Index(7), "7"),
        ] {
            let function =
                instantiate_function(agent, scope, realm, shared, name).expect("Speicher");
            assert_eq!(function_name(agent, scope, function), expected);
            // Funktionsobjekte derselben Stelle teilen sich die Daten.
            let parts = function_parts(agent.heap(), agent.heap().get(function));
            assert_eq!(parts, Some((0, realm)));
        }
        assert_eq!(agent.functions.len(), 1);
    });
}

#[test]
fn function_objects_survive_collections() {
    let (mut agent, realm) = agent_with(Options {
        stress: true,
        verify: true,
        ..Options::default()
    });
    agent.scope(|agent, scope| {
        let source = "var o = {m(a) { return a + 1; }, get g() { return this.m(1); }}; \
                      function f(n) { return n ? f(n - 1) + o.g : 0; } f(5)";
        let result = evaluate(agent, scope, realm, source);
        assert_eq!(number(agent, result), Some(10.0));
        let function = evaluate(agent, scope, realm, "f");
        assert_eq!(function_name(agent, scope, function), "f");
        assert!(function_parts(agent.heap(), agent.heap().get(function)).is_some());
    });
}

/// Klassen (ClassDefinitionEvaluation, Abschnitt 15.7.14): `length`,
/// `name`, ein festes `prototype` und dann die statischen Methoden; auf dem
/// Prototyp `constructor` und die Methoden, nicht aufzählbar.
#[test]
fn classes_have_a_fixed_prototype_and_hidden_methods() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        let class = evaluate(
            agent,
            scope,
            realm,
            "class A { constructor(a, b) {} m() {} get g() {} set g(v) {} static s() {} } A",
        );
        assert_eq!(
            keys(agent, scope, class),
            ["length", "name", "prototype", "s"]
        );
        let (length, flags) = data(agent, scope, class, "length");
        assert_eq!((number(agent, length), flags), (Some(2.0), FIXED));
        let (name, flags) = data(agent, scope, class, "name");
        assert_eq!((text(agent, name), flags), ("A".to_owned(), FIXED));
        let (prototype, flags) = data(agent, scope, class, "prototype");
        assert_eq!(flags, [Some(false), Some(false), Some(false)]);
        assert_eq!(keys(agent, scope, prototype), ["constructor", "m", "g"]);
        let (constructor, flags) = data(agent, scope, prototype, "constructor");
        assert_eq!(bits(agent, constructor), bits(agent, class));
        assert_eq!(flags, [Some(true), Some(false), Some(true)]);
        for (object, property) in [(prototype, "m"), (class, "s")] {
            let (method, flags) = data(agent, scope, object, property);
            assert_eq!(flags, [Some(true), Some(false), Some(true)], "{property}");
            assert_eq!(keys(agent, scope, method), ["length", "name"], "{property}");
            assert_eq!(function_name(agent, scope, method), property);
            assert!(
                !is_constructor(agent, agent.heap().get(method)),
                "{property}"
            );
        }
        let descriptor = own(agent, scope, prototype, "g");
        assert_eq!(
            (descriptor.enumerable, descriptor.configurable),
            (Some(false), Some(true))
        );
        let getter = descriptor.get.expect("Getter");
        let setter = descriptor.set.expect("Setter");
        assert_eq!(function_name(agent, scope, getter), "get g");
        assert_eq!(function_name(agent, scope, setter), "set g");

        let function_prototype = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
        let object_prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
        for (object, expected) in [(class, function_prototype), (prototype, object_prototype)] {
            let actual = get_prototype_of(agent, scope, object).expect("kein Fehler");
            assert_eq!(
                actual.map(|actual| bits(agent, actual)),
                Some(bits(agent, expected))
            );
        }
        let value = agent.heap().get(class);
        assert!(is_constructor(agent, value));
        assert_eq!(construction(agent, value), Construction::Base);
    });
}

/// Eine abgeleitete Klasse erbt vom Konstruktor und vom Prototyp der
/// Oberklasse, mit *null* nur vom Prototyp nichts. Ohne `constructor` hat
/// sie `length` 0.
#[test]
fn derived_classes_inherit_from_their_superclass() {
    let (mut agent, realm) = checked();
    agent.scope(|agent, scope| {
        let base = evaluate(agent, scope, realm, "class A {} A");
        assert_eq!(
            construction(agent, agent.heap().get(base)),
            Construction::Base
        );
        let derived = evaluate(agent, scope, realm, "class B extends A {} B");
        assert_eq!(
            construction(agent, agent.heap().get(derived)),
            Construction::Derived
        );
        assert_eq!(keys(agent, scope, derived), ["length", "name", "prototype"]);
        let (length, _) = data(agent, scope, derived, "length");
        assert_eq!(number(agent, length), Some(0.0));
        assert_eq!(function_name(agent, scope, derived), "B");
        let actual = get_prototype_of(agent, scope, derived).expect("kein Fehler");
        assert_eq!(
            actual.map(|actual| bits(agent, actual)),
            Some(bits(agent, base))
        );
        let (prototype, _) = data(agent, scope, derived, "prototype");
        let (parent, _) = data(agent, scope, base, "prototype");
        let actual = get_prototype_of(agent, scope, prototype).expect("kein Fehler");
        assert_eq!(
            actual.map(|actual| bits(agent, actual)),
            Some(bits(agent, parent))
        );

        let null = evaluate(agent, scope, realm, "(class extends null {})");
        assert_eq!(function_name(agent, scope, null), "");
        let function_prototype = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
        let actual = get_prototype_of(agent, scope, null).expect("kein Fehler");
        assert_eq!(
            actual.map(|actual| bits(agent, actual)),
            Some(bits(agent, function_prototype))
        );
        let (prototype, _) = data(agent, scope, null, "prototype");
        let actual = get_prototype_of(agent, scope, prototype).expect("kein Fehler");
        assert!(actual.is_none());
        let named = evaluate(agent, scope, realm, "var C = class {}; C");
        assert_eq!(function_name(agent, scope, named), "C");
    });
}
