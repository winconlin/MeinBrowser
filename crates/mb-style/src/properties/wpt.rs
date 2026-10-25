//! Die WPT-Tests zum Parsen und Serialisieren der unterstützten
//! Eigenschaften: `css/<modul>/parsing/<eigenschaft>-{valid,invalid,shorthand,alias}*.html`.
//!
//! Die Testdateien laufen unverändert im Interpreter aus [`crate::wpt_js`];
//! das Dokument baut mb-html. testharness.js, die eingebundenen
//! Hilfsskripte aus `css/support/`, `document`, `CSS.supports()` und
//! `CSSStyleDeclaration` stellt dieser Test bereit. Die Hilfsskripte sind
//! nach dem Stand der WPT in `testdata` Schritt für Schritt nachgebaut, die
//! Namen der Tests eingeschlossen.
//!
//! Fehlgeschlagene Tests werden eingeordnet: Tests gültiger Werte mit
//! Funktionen oder Eigenschaften, die noch fehlen ([`GAPS`]), Abweichungen
//! mit Begründung ([`DEVIATIONS`]), alle übrigen sind Fehler. Was sich in
//! einer Datei nicht ausführen lässt, nennt [`UNSUPPORTED`]. Alle Listen
//! werden geprüft, damit sie nicht veralten: Eine Lücke, die besteht, ist
//! ebenfalls ein Fehler.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mb_base::CodePoint;
use mb_css::{Source, parse_component_value_list};
use mb_dom::{Document, NodeKind, QuirksMode};
use mb_html::{Options, ScriptingMode, parse_document};
use mb_url::Url;

use super::PropertyId;
use crate::declarations::DeclarationBlock;
use crate::values::ParseContext;
use crate::wpt_js::{
    Completion, Control, Host, Interpreter, JsString, Value, describe, get, is_truthy, js_string,
    json_quote, number_to_string, own_keys, parse_float, same_value, set, strict_equals, to_number,
    to_string, unsupported,
};

/// Die Module mit Tests zu den unterstützten Eigenschaften.
const DIRECTORIES: &[&str] = &[
    "css-backgrounds",
    "css-box",
    "css-cascade",
    "css-color",
    "css-display",
    "css-fonts",
    "css-inline",
    "css-lists",
    "css-logical",
    "css-sizing",
    "css-text",
    "css-text-decor",
    "css-writing-modes",
];

/// Dateien, deren Name zwei unterstützte Eigenschaften nennt.
const TWO_PROPERTY_FILES: &[&str] = &[
    "css-logical/parsing/margin-block-inline-invalid.html",
    "css-logical/parsing/margin-block-inline-shorthand.html",
    "css-logical/parsing/margin-block-inline-valid.html",
    "css-logical/parsing/padding-block-inline-invalid.html",
    "css-logical/parsing/padding-block-inline-shorthand.html",
    "css-logical/parsing/padding-block-inline-valid.html",
];

/// Was noch fehlt, mit dem Text, an dem es zu erkennen ist. Ein Test für
/// einen gültigen Wert, dessen Eingaben diesen Text enthalten (ohne
/// Unterschied zwischen ASCII-Groß- und -Kleinbuchstaben), darf
/// fehlschlagen, und er muss es auch.
const GAPS: &[(&str, &str)] = &[
    ("calc(", "mathematische Funktionen"),
    ("calc-size(", "mathematische Funktionen"),
    ("min(", "mathematische Funktionen"),
    ("max(", "mathematische Funktionen"),
    ("clamp(", "mathematische Funktionen"),
    ("round(", "mathematische Funktionen"),
    ("mod(", "mathematische Funktionen"),
    ("rem(", "mathematische Funktionen"),
    ("sin(", "mathematische Funktionen"),
    ("cos(", "mathematische Funktionen"),
    ("tan(", "mathematische Funktionen"),
    ("pow(", "mathematische Funktionen"),
    ("sqrt(", "mathematische Funktionen"),
    ("hypot(", "mathematische Funktionen"),
    ("log(", "mathematische Funktionen"),
    ("exp(", "mathematische Funktionen"),
    ("abs(", "mathematische Funktionen"),
    ("sign(", "mathematische Funktionen"),
    ("progress(", "mathematische Funktionen"),
    ("random(", "mathematische Funktionen"),
    ("sibling-index(", "mathematische Funktionen"),
    ("sibling-count(", "mathematische Funktionen"),
    ("var(", "var() und eigene Eigenschaften"),
    ("env(", "env()"),
    ("attr(", "attr()"),
    ("if(", "if()"),
    ("color-mix(", "color-mix()"),
    ("light-dark(", "light-dark()"),
    ("contrast-color(", "contrast-color()"),
    ("device-cmyk(", "device-cmyk()"),
    ("color-layers(", "color-layers()"),
    ("alpha(", "alpha()"),
    ("(from ", "relative Farben"),
    ("gradient(", "Verläufe"),
    ("image(", "image()"),
    ("image-set(", "image-set()"),
    ("cross-fade(", "cross-fade()"),
    ("element(", "element()"),
    ("paint(", "paint()"),
    ("stripes(", "stripes()"),
    ("symbols(", "symbols()"),
    ("fit-content(", "fit-content()"),
    ("anchor(", "anchor()"),
    ("anchor-size(", "anchor-size()"),
    ("border-image-", "border-image"),
];

/// Tests, die aus benannten Gründen abweichen: Datei, Name, Grund.
const DEVIATIONS: &[(&str, &str, &str)] = &[(
    "css-fonts/parsing/font-shorthand-variant.html",
    "font shorthand returns only CSS2 font-variant values",
    "Die Kurzschreibweise font-variant fehlt noch; `font` setzt nur \
     font-variant-caps.",
)];

/// Was sich in einer Datei nicht ausführen lässt.
const UNSUPPORTED: &[(&str, &[&str])] = &[
    (
        "css-text/parsing/white-space-shorthand-text-wrap.html",
        &["getComputedStyle"],
    ),
    (
        "css-text/parsing/white-space-shorthand.html",
        &["getComputedStyle"],
    ),
];

#[derive(Clone, Copy, PartialEq)]
enum TestKind {
    /// `test_valid_value()` und Ähnliche: Ein gültiger Wert wird gesetzt.
    Valid,
    /// `test_invalid_value()`
    Invalid,
    /// `test()` im Skript der Datei
    Script,
}

enum Outcome {
    Pass,
    Fail(String),
    Unsupported(String),
}

struct Subtest {
    name: String,
    kind: TestKind,
    /// Die Eingaben als Text, um Lücken zu erkennen.
    inputs: Vec<String>,
    outcome: Outcome,
}

/// Die Seite einer Testdatei: die Elemente mit ID und die Ergebnisse.
struct Page {
    elements: Vec<(String, Value)>,
    quirks_mode: bool,
    subtests: RefCell<Vec<Subtest>>,
}

impl Page {
    fn element_by_id(&self, id: &str) -> Option<Value> {
        self.elements
            .iter()
            .find(|(found, _)| found == id)
            .map(|(_, element)| element.clone())
    }

    fn create_element(&self) -> Value {
        element(self.quirks_mode, false)
    }

    /// `document.getElementById('target') || document.createElement('div')`
    fn target(&self) -> Value {
        self.element_by_id("target")
            .unwrap_or_else(|| self.create_element())
    }

    fn record(&self, subtest: Subtest) {
        self.subtests.borrow_mut().push(subtest);
    }
}

fn element(quirks_mode: bool, style_attribute: bool) -> Value {
    let style = Value::host(Rc::new(Style {
        block: Rc::new(RefCell::new(DeclarationBlock::new())),
        quirks_mode,
    }));
    Value::host(Rc::new(Element {
        style,
        style_attribute,
    }))
}

fn argument(arguments: &[Value], index: usize) -> Value {
    arguments.get(index).cloned().unwrap_or(Value::Undefined)
}

fn text(value: &Value) -> Completion<String> {
    Ok(String::from_utf16_lossy(&to_string(value)?))
}

fn lossy(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// `document`
struct DocumentObject {
    page: Rc<Page>,
}

impl Host for DocumentObject {
    fn get(&self, key: &str) -> Completion<Option<Value>> {
        let page = self.page.clone();
        Ok(Some(match key {
            "getElementById" => Value::native(move |_, _, arguments| {
                let id = text(&argument(&arguments, 0))?;
                Ok(page.element_by_id(&id).unwrap_or(Value::Null))
            }),
            "createElement" => Value::native(move |_, _, _| Ok(page.create_element())),
            _ => return unsupported(format!("document.{key}")),
        }))
    }

    fn set(&self, key: &str, _: &Value) -> Completion<bool> {
        unsupported(format!("document.{key} setzen"))
    }
}

struct Element {
    style: Value,
    /// Ob das Element ein Attribut `style` hat. Dessen Deklarationen liest
    /// erst die Kaskade.
    style_attribute: bool,
}

impl Host for Element {
    fn get(&self, key: &str) -> Completion<Option<Value>> {
        match key {
            "style" if !self.style_attribute => Ok(Some(self.style.clone())),
            "style" => unsupported("Attribut style"),
            _ => unsupported(format!("Element.{key}")),
        }
    }

    fn set(&self, key: &str, _: &Value) -> Completion<bool> {
        unsupported(format!("Element.{key} setzen"))
    }
}

/// Die CSS-Eigenschaft zu einem Attribut von `CSSStyleDeclaration`: dem
/// *dashed attribute* oder dem *camel-cased attribute* einer unterstützten
/// Eigenschaft (CSSOM). Anders als bei `setProperty()` zählt hier Groß- und
/// Kleinschreibung.
fn attribute_property(key: &str) -> Option<String> {
    let property = if key.contains('-') {
        key.to_owned()
    } else {
        // *IDL attribute to CSS property*
        let mut property = String::new();
        for c in key.chars() {
            if c.is_ascii_uppercase() {
                property.push('-');
                property.push(c.to_ascii_lowercase());
            } else {
                property.push(c);
            }
        }
        property
    };
    let supported = !property.bytes().any(|byte| byte.is_ascii_uppercase())
        && PropertyId::from_name(&property).is_some();
    supported.then_some(property)
}

/// `[LegacyNullToEmptyString] CSSOMString`
fn legacy_null_to_empty(value: &Value) -> Completion<String> {
    match value {
        Value::Null => Ok(String::new()),
        other => text(other),
    }
}

/// `CSSStyleDeclaration` eines Elements.
struct Style {
    block: Rc<RefCell<DeclarationBlock>>,
    quirks_mode: bool,
}

impl Style {
    fn context(quirks_mode: bool) -> ParseContext<'static> {
        ParseContext {
            base_url: None,
            quirks_mode,
        }
    }
}

impl Host for Style {
    fn get(&self, key: &str) -> Completion<Option<Value>> {
        let block = self.block.clone();
        let quirks_mode = self.quirks_mode;
        Ok(Some(match key {
            "getPropertyValue" => Value::native(move |_, _, arguments| {
                let property = text(&argument(&arguments, 0))?;
                Ok(Value::string(&block.borrow().get_property_value(&property)))
            }),
            "getPropertyPriority" => Value::native(move |_, _, arguments| {
                let property = text(&argument(&arguments, 0))?;
                Ok(Value::string(
                    block.borrow().get_property_priority(&property),
                ))
            }),
            "setProperty" => Value::native(move |_, _, arguments| {
                let property = text(&argument(&arguments, 0))?;
                let value = legacy_null_to_empty(&argument(&arguments, 1))?;
                let priority = match argument(&arguments, 2) {
                    Value::Undefined => String::new(),
                    other => text(&other)?,
                };
                block.borrow_mut().set_property(
                    &property,
                    &value,
                    &priority,
                    Style::context(quirks_mode),
                );
                Ok(Value::Undefined)
            }),
            "removeProperty" => Value::native(move |_, _, arguments| {
                let property = text(&argument(&arguments, 0))?;
                Ok(Value::string(
                    &block.borrow_mut().remove_property(&property),
                ))
            }),
            "item" => Value::native(move |_, _, arguments| {
                let index = to_number(&argument(&arguments, 0))?;
                if !(index >= 0.0 && index.fract() == 0.0 && index < f64::from(u32::MAX)) {
                    return unsupported("item() mit einem Index, der umgewandelt werden müsste");
                }
                // Ganzzahlig und im Bereich, siehe oben.
                let index = index as usize;
                Ok(Value::string(block.borrow().item(index).unwrap_or("")))
            }),
            "length" => Value::Number(self.block.borrow().len() as f64),
            "cssText" => Value::string(&self.block.borrow().css_text()),
            _ => {
                if let Ok(index) = key.parse::<usize>()
                    && key.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Ok(self.block.borrow().item(index).map(Value::string));
                }
                let Some(property) = attribute_property(key) else {
                    return Ok(None);
                };
                Value::string(&self.block.borrow().get_property_value(&property))
            }
        }))
    }

    fn set(&self, key: &str, value: &Value) -> Completion<bool> {
        if matches!(key, "cssText" | "length" | "parentRule" | "cssFloat") {
            return unsupported(format!("CSSStyleDeclaration.{key} setzen"));
        }
        let Some(property) = attribute_property(key) else {
            return Ok(false);
        };
        let value = legacy_null_to_empty(value)?;
        self.block.borrow_mut().set_property(
            &property,
            &value,
            "",
            Style::context(self.quirks_mode),
        );
        Ok(true)
    }
}

/// `CSS.supports(property, value)`
fn supports(property: &Value, value: &Value) -> Completion<bool> {
    let property = text(property)?;
    if property.starts_with("--") {
        return unsupported("eigene Eigenschaften");
    }
    let Some(property) = PropertyId::from_name(&property) else {
        return Ok(false);
    };
    let values = parse_component_value_list(&Source::new(&text(value)?));
    Ok(property
        .parse(values.values(), ParseContext::default())
        .is_some())
}

/// `CSS`
struct CssNamespace;

impl Host for CssNamespace {
    fn get(&self, key: &str) -> Completion<Option<Value>> {
        match key {
            "supports" => Ok(Some(Value::native(|_, _, arguments| {
                if arguments.len() < 2 {
                    return unsupported("CSS.supports() mit einer Bedingung");
                }
                Ok(Value::Bool(supports(
                    &argument(&arguments, 0),
                    &argument(&arguments, 1),
                )?))
            }))),
            _ => unsupported(format!("CSS.{key}")),
        }
    }

    fn set(&self, key: &str, _: &Value) -> Completion<bool> {
        unsupported(format!("CSS.{key} setzen"))
    }
}

/// Das Objekt, das `test()` der Testfunktion übergibt.
struct TestObject;

impl Host for TestObject {
    fn get(&self, key: &str) -> Completion<Option<Value>> {
        unsupported(format!("Test.{key}"))
    }

    fn set(&self, key: &str, _: &Value) -> Completion<bool> {
        unsupported(format!("Test.{key} setzen"))
    }
}

// ---------------------------------------------------------------------------
// testharness.js

fn format_value(value: &Value) -> String {
    match value {
        Value::String(units) => json_quote(units),
        other => describe(other),
    }
}

fn fail<T>(message: String) -> Completion<T> {
    Err(Control::Throw(Value::String(js_string(&message))))
}

fn assert_equals(actual: &Value, expected: &Value, description: &str) -> Completion<()> {
    if same_value(actual, expected) {
        return Ok(());
    }
    fail(format!(
        "assert_equals: {description} expected {} but got {}",
        format_value(expected),
        format_value(actual)
    ))
}

fn assert_not_equals(actual: &Value, expected: &Value, description: &str) -> Completion<()> {
    if !same_value(actual, expected) {
        return Ok(());
    }
    fail(format!(
        "assert_not_equals: {description} got disallowed value {}",
        format_value(actual)
    ))
}

fn assert_bool(actual: &Value, expected: bool, description: &str) -> Completion<()> {
    if strict_equals(actual, &Value::Bool(expected)) {
        return Ok(());
    }
    fail(format!(
        "assert_{expected}: {description} expected {expected} got {}",
        format_value(actual)
    ))
}

fn assert_in_array(actual: &Value, expected: &Value, description: &str) -> Completion<()> {
    let Some(items) = expected.array_items() else {
        return unsupported("assert_in_array() ohne Liste");
    };
    if items.iter().any(|item| strict_equals(item, actual)) {
        return Ok(());
    }
    fail(format!(
        "assert_in_array: {description} value {} not in array {}",
        format_value(actual),
        describe(expected)
    ))
}

fn assert_array_approx_equals(
    actual: &[f64],
    expected: &[f64],
    epsilon: f64,
    description: &str,
) -> Completion<()> {
    if actual.len() != expected.len() {
        return fail(format!(
            "assert_array_approx_equals: {description} lengths differ, expected {expected:?} got {actual:?}"
        ));
    }
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        // Wie `Math.abs(actual - expected) <= epsilon`: NaN besteht nicht.
        if (actual - expected).abs() > epsilon || (actual - expected).is_nan() {
            return fail(format!(
                "assert_array_approx_equals: {description} expected {expected} +/- {epsilon} \
                 but got {actual} at index {index}"
            ));
        }
    }
    Ok(())
}

/// Führt einen Test aus und hält fest, wie er endete.
fn run_test(
    interpreter: &mut Interpreter,
    body: impl FnOnce(&mut Interpreter) -> Completion<()>,
) -> Outcome {
    match body(interpreter) {
        Ok(()) => Outcome::Pass,
        Err(Control::Throw(value)) => Outcome::Fail(describe(&value)),
        Err(Control::Unsupported(reason)) => Outcome::Unsupported(reason),
        Err(Control::Return(_) | Control::Break | Control::Continue) => {
            Outcome::Fail("Sprung aus dem Test".to_owned())
        }
    }
}

/// Die Beschreibung einer Prüfung, ein optionales Argument.
fn description(arguments: &[Value], index: usize) -> Completion<String> {
    match argument(arguments, index) {
        Value::Undefined => Ok(String::new()),
        other => text(&other),
    }
}

/// Eine Zusicherung von testharness.js über ihre Argumente.
type Assertion = fn(&[Value]) -> Completion<()>;

fn define_testharness(interpreter: &mut Interpreter, page: &Rc<Page>) {
    let page = page.clone();
    interpreter.define(
        "test",
        Value::native(move |interpreter, _, arguments| {
            let function = argument(&arguments, 0);
            let name = match argument(&arguments, 1) {
                Value::Undefined => return unsupported("test() ohne Namen"),
                name => text(&name)?,
            };
            let outcome = run_test(interpreter, |interpreter| {
                let test = Value::host(Rc::new(TestObject));
                interpreter
                    .call(&function, Value::Undefined, vec![test])
                    .map(drop)
            });
            page.record(Subtest {
                name,
                kind: TestKind::Script,
                inputs: Vec::new(),
                outcome,
            });
            Ok(Value::Undefined)
        }),
    );
    let assertions: [(&str, Assertion); 5] = [
        ("assert_equals", |arguments| {
            assert_equals(
                &argument(arguments, 0),
                &argument(arguments, 1),
                &description(arguments, 2)?,
            )
        }),
        ("assert_not_equals", |arguments| {
            assert_not_equals(
                &argument(arguments, 0),
                &argument(arguments, 1),
                &description(arguments, 2)?,
            )
        }),
        ("assert_true", |arguments| {
            assert_bool(&argument(arguments, 0), true, &description(arguments, 1)?)
        }),
        ("assert_false", |arguments| {
            assert_bool(&argument(arguments, 0), false, &description(arguments, 1)?)
        }),
        ("assert_in_array", |arguments| {
            assert_in_array(
                &argument(arguments, 0),
                &argument(arguments, 1),
                &description(arguments, 2)?,
            )
        }),
    ];
    for (name, assertion) in assertions {
        interpreter.define(
            name,
            Value::native(move |_, _, arguments| {
                assertion(&arguments)?;
                Ok(Value::Undefined)
            }),
        );
    }
}

// ---------------------------------------------------------------------------
// Die Hilfsskripte aus css/support/

/// `JSON.stringify()` für die Werte, die die Tests übergeben.
fn stringify(value: &Value) -> Completion<String> {
    Ok(match value {
        Value::String(units) => json_quote(units),
        Value::Number(number) if number.is_finite() => number_to_string(*number),
        Value::Number(_) | Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Undefined => "undefined".to_owned(),
        Value::Object(_) => return unsupported("JSON.stringify() eines Objekts"),
    })
}

/// Die Eingaben eines Tests als Text, Listen ausgepackt.
fn inputs(values: &[&Value]) -> Vec<String> {
    let mut inputs = Vec::new();
    for value in values {
        match value {
            Value::String(units) => inputs.push(lossy(units)),
            other => {
                for item in other.array_items().unwrap_or_default() {
                    if let Value::String(units) = item {
                        inputs.push(lossy(&units));
                    }
                }
            }
        }
    }
    inputs
}

fn style_of(element: &Value) -> Completion<Value> {
    get(element, &js_string("style"))
}

fn call_method(
    interpreter: &mut Interpreter,
    object: &Value,
    name: &str,
    arguments: Vec<Value>,
) -> Completion<Value> {
    let method = get(object, &js_string(name))?;
    interpreter.call(&method, object.clone(), arguments)
}

fn empty() -> Value {
    Value::string("")
}

/// Wie ein `finally`: Scheitert das Aufräumen, gilt dessen Ergebnis.
fn finally(result: Completion<()>, cleanup: Completion<()>) -> Completion<()> {
    cleanup.and(result)
}

/// Wie `test_valid_value()` den gelesenen Wert prüft.
enum Comparison {
    Exact,
    /// `options.comparisonFunction`
    Function(Value),
    /// `set_up_fuzzy_color_test(epsilon)` aus `color-testcommon.js`
    FuzzyColor(f64),
}

/// `test_valid_value()` aus `parsing-testcommon.js`
fn test_valid_value(
    interpreter: &mut Interpreter,
    page: &Page,
    arguments: &[Value],
) -> Completion<()> {
    let value = argument(arguments, 1);
    let serialized = if arguments.len() < 3 {
        value.clone()
    } else {
        argument(arguments, 2)
    };
    let comparison = match argument(arguments, 3) {
        Value::Undefined => Comparison::Exact,
        options => {
            let function = get(&options, &js_string("comparisonFunction"))?;
            if is_truthy(&function) {
                Comparison::Function(function)
            } else {
                Comparison::Exact
            }
        }
    };
    valid_value(
        interpreter,
        page,
        &argument(arguments, 0),
        &value,
        &serialized,
        &comparison,
    )
}

fn valid_value(
    interpreter: &mut Interpreter,
    page: &Page,
    property: &Value,
    value: &Value,
    serialized: &Value,
    comparison: &Comparison,
) -> Completion<()> {
    let name = format!(
        "e.style['{}'] = {} should set the property value",
        text(property)?,
        stringify(value)?
    );
    let outcome = run_test(interpreter, |interpreter| {
        let style = style_of(&page.target())?;
        let key = to_string(property)?;
        set(&style, &key, empty())?;
        set(&style, &key, value.clone())?;
        let read = call_method(
            interpreter,
            &style,
            "getPropertyValue",
            vec![property.clone()],
        )?;
        assert_not_equals(&read, &empty(), "property should be set")?;
        match comparison {
            Comparison::Function(function) => {
                interpreter.call(
                    function,
                    Value::Undefined,
                    vec![read.clone(), serialized.clone()],
                )?;
            }
            Comparison::FuzzyColor(epsilon) => fuzzy_compare_colors(&read, serialized, *epsilon)?,
            Comparison::Exact if serialized.array_items().is_some() => {
                assert_in_array(&read, serialized, "serialization should be sound")?;
            }
            Comparison::Exact => {
                assert_equals(&read, serialized, "serialization should be canonical")?;
            }
        }
        set(&style, &key, read.clone())?;
        let again = call_method(
            interpreter,
            &style,
            "getPropertyValue",
            vec![property.clone()],
        )?;
        assert_equals(&again, &read, "serialization should round-trip")
    });
    page.record(Subtest {
        name,
        kind: TestKind::Valid,
        inputs: inputs(&[property, value, serialized]),
        outcome,
    });
    Ok(())
}

/// `test_invalid_value()` aus `parsing-testcommon.js`
fn test_invalid_value(
    interpreter: &mut Interpreter,
    page: &Page,
    arguments: &[Value],
) -> Completion<()> {
    let property = argument(arguments, 0);
    let value = argument(arguments, 1);
    let name = format!(
        "e.style['{}'] = {} should not set the property value",
        text(&property)?,
        stringify(&value)?
    );
    let outcome = run_test(interpreter, |interpreter| {
        let style = style_of(&page.target())?;
        let key = to_string(&property)?;
        set(&style, &key, empty())?;
        set(&style, &key, value.clone())?;
        let read = call_method(
            interpreter,
            &style,
            "getPropertyValue",
            vec![property.clone()],
        )?;
        assert_equals(&read, &empty(), "")
    });
    page.record(Subtest {
        name,
        kind: TestKind::Invalid,
        inputs: inputs(&[&property, &value]),
        outcome,
    });
    Ok(())
}

/// Der Rumpf von `try` im ersten Test von `test_shorthand_value()`.
fn shorthand_sets_longhand(
    style: &Value,
    key: &JsString,
    value: &Value,
    longhand: &JsString,
    expected: &Value,
) -> Completion<()> {
    set(style, key, value.clone())?;
    let read = get(style, longhand)?;
    assert_equals(
        &read,
        expected,
        &format!("{} should be canonical", lossy(longhand)),
    )?;
    set(style, longhand, empty())?;
    set(style, longhand, read.clone())?;
    assert_equals(
        &get(style, longhand)?,
        &read,
        "serialization should round-trip",
    )
}

/// Der Rumpf von `try` im zweiten Test von `test_shorthand_value()`.
fn shorthand_sets_nothing_else(
    style: &Value,
    property: &Value,
    value: &Value,
    longhands: &[JsString],
) -> Completion<()> {
    let length = js_string("length");
    let expected_length = get(style, &length)?;
    set(style, &to_string(property)?, value.clone())?;
    assert_bool(&Value::Bool(supports(property, value)?), true, "")?;
    for longhand in longhands {
        set(style, longhand, empty())?;
    }
    assert_equals(&get(style, &length)?, &expected_length, "")
}

/// `test_shorthand_value()` aus `shorthand-testcommon.js`
fn test_shorthand_value(
    interpreter: &mut Interpreter,
    page: &Page,
    arguments: &[Value],
) -> Completion<()> {
    let property = argument(arguments, 0);
    let value = argument(arguments, 1);
    let longhands = argument(arguments, 2);
    let stringified = stringify(&value)?;
    let mut keys = own_keys(&longhands)?;
    keys.sort();
    for longhand in &keys {
        let expected = get(&longhands, longhand)?;
        let name = format!(
            "e.style['{}'] = {stringified} should set {}",
            text(&property)?,
            lossy(longhand)
        );
        let outcome = run_test(interpreter, |_| {
            let style = style_of(&page.target())?;
            let key = to_string(&property)?;
            set(&style, &key, empty())?;
            let result = shorthand_sets_longhand(&style, &key, &value, longhand, &expected);
            finally(result, set(&style, &key, empty()))
        });
        page.record(Subtest {
            name,
            kind: TestKind::Valid,
            inputs: inputs(&[
                &property,
                &value,
                &Value::String(longhand.clone()),
                &expected,
            ]),
            outcome,
        });
    }
    let name = format!(
        "e.style['{}'] = {stringified} should not set unrelated longhands",
        text(&property)?
    );
    let outcome = run_test(interpreter, |_| {
        let style = style_of(&page.target())?;
        let key = to_string(&property)?;
        set(&style, &key, empty())?;
        let result = shorthand_sets_nothing_else(&style, &property, &value, &keys);
        finally(result, set(&style, &key, empty()))
    });
    page.record(Subtest {
        name,
        kind: TestKind::Valid,
        inputs: inputs(&[&property, &value]),
        outcome,
    });
    Ok(())
}

/// Der Rumpf von `test_is_legacy_name_alias()`.
fn legacy_name_alias(
    interpreter: &mut Interpreter,
    page: &Page,
    old: &Value,
    new: &Value,
) -> Completion<()> {
    let old_name = text(old)?;
    let new_name = text(new)?;
    let style = style_of(&page.create_element())?;
    call_method(
        interpreter,
        &style,
        "setProperty",
        vec![old.clone(), Value::string("inherit")],
    )?;
    let inherit = Value::string("inherit");
    let read = call_method(interpreter, &style, "getPropertyValue", vec![old.clone()])?;
    assert_equals(&read, &inherit, &format!("{old_name} is supported"))?;
    let read = call_method(interpreter, &style, "getPropertyValue", vec![new.clone()])?;
    assert_equals(
        &read,
        &inherit,
        &format!("{old_name} is an alias for {new_name}"),
    )?;
    assert_equals(
        &get(&style, &js_string("cssText"))?,
        &Value::string(&format!("{new_name}: inherit;")),
        &format!("declarations serialize using new name {new_name}"),
    )?;
    let message = format!("{old_name} is a legacy name alias rather than a shorthand");
    for (set_name, read_name, variable) in [(old, new, "var(--v)"), (new, old, "var(--w)")] {
        let style = style_of(&page.create_element())?;
        call_method(
            interpreter,
            &style,
            "setProperty",
            vec![set_name.clone(), Value::string(variable)],
        )?;
        let read = call_method(
            interpreter,
            &style,
            "getPropertyValue",
            vec![read_name.clone()],
        )?;
        assert_equals(&read, &Value::string(variable), &message)?;
    }
    // `is_property_in_longhands()` hängt ein Element an `document.body` an
    // und fragt `getComputedStyle()`.
    unsupported("document.body")
}

/// `test_is_legacy_name_alias()` aus `shorthand-testcommon.js`
fn test_is_legacy_name_alias(
    interpreter: &mut Interpreter,
    page: &Page,
    arguments: &[Value],
) -> Completion<()> {
    let old = argument(arguments, 0);
    let new = argument(arguments, 1);
    let name = format!("{} is a legacy name alias for {}", text(&old)?, text(&new)?);
    let outcome = run_test(interpreter, |interpreter| {
        legacy_name_alias(interpreter, page, &old, &new)
    });
    page.record(Subtest {
        name,
        kind: TestKind::Valid,
        inputs: inputs(&[
            &old,
            &new,
            &Value::string("var(--v)"),
            &Value::string("var(--w)"),
        ]),
        outcome,
    });
    Ok(())
}

/// Die Zahlen in einer Farbe wie `getNumbers()`: Teile zwischen
/// Leerzeichen, `(` und `,`, gelesen mit `parseFloat()`.
fn color_numbers(color: &[u16]) -> Vec<f64> {
    color
        .split(|&unit| matches!(unit, 0x20 | 0x28 | 0x2C))
        .map(parse_float)
        .filter(|number| !number.is_nan())
        .collect()
}

/// `getNonNumbers()`: die Farbe ohne Ziffern und Punkte.
fn color_non_numbers(color: &[u16]) -> Value {
    let units: Vec<u16> = color
        .iter()
        .copied()
        .filter(|&unit| !matches!(unit, 0x30..=0x39 | 0x2E))
        .collect();
    Value::String(units.into())
}

/// `fuzzy_compare_colors()` aus `set_up_fuzzy_color_test()`
fn fuzzy_compare_colors(input: &Value, expected: &Value, epsilon: f64) -> Completion<()> {
    let input = to_string(input)?;
    let expected = to_string(expected)?;
    let result = assert_array_approx_equals(
        &color_numbers(&input),
        &color_numbers(&expected),
        epsilon,
        "Numeric parameters are approximately equal.",
    )
    .and_then(|()| {
        assert_equals(
            &color_non_numbers(&input),
            &color_non_numbers(&expected),
            "Color format is correct.",
        )
    });
    match result {
        Err(Control::Throw(error)) => fail(format!(
            "Colors do not match.\nActual:   {}\nExpected: {}.\n{}",
            lossy(&input),
            lossy(&expected),
            describe(&error)
        )),
        other => other,
    }
}

/// `fuzzy_test_valid_color()` aus `color-testcommon.js`
fn fuzzy_test_valid_color(
    interpreter: &mut Interpreter,
    page: &Page,
    arguments: &[Value],
) -> Completion<()> {
    let color = Value::string("color");
    let specified = argument(arguments, 0);
    let parsed = argument(arguments, 1);
    if !is_truthy(&parsed) {
        return valid_value(
            interpreter,
            page,
            &color,
            &specified,
            &specified,
            &Comparison::Exact,
        );
    }
    let epsilon = match argument(arguments, 2) {
        epsilon if is_truthy(&epsilon) => to_number(&epsilon)?,
        _ => 0.01,
    };
    valid_value(
        interpreter,
        page,
        &color,
        &specified,
        &parsed,
        &Comparison::FuzzyColor(epsilon),
    )
}

/// `test_computed_value()` aus `computed-testcommon.js`. Berechnete Werte
/// gibt es noch nicht; der Test fragt als Erstes `getComputedStyle()`.
fn test_computed_value(_: &mut Interpreter, page: &Page, arguments: &[Value]) -> Completion<()> {
    let extra = match argument(arguments, 3) {
        extra if is_truthy(&extra) => format!(" {}", text(&extra)?),
        _ => String::new(),
    };
    let name = format!(
        "Property {} value '{}'{extra}",
        text(&argument(arguments, 0))?,
        text(&argument(arguments, 1))?
    );
    page.record(Subtest {
        name,
        kind: TestKind::Script,
        inputs: Vec::new(),
        outcome: Outcome::Unsupported("getComputedStyle".to_owned()),
    });
    Ok(())
}

type Helper = fn(&mut Interpreter, &Page, &[Value]) -> Completion<()>;

/// Die nachgebauten Funktionen eines Hilfsskripts.
fn helpers(script: &str) -> Option<&'static [(&'static str, Helper)]> {
    Some(match script {
        "/resources/testharnessreport.js" => &[],
        "/css/support/parsing-testcommon.js" => &[
            ("test_valid_value", test_valid_value),
            ("test_invalid_value", test_invalid_value),
        ],
        "/css/support/shorthand-testcommon.js" => &[
            ("test_shorthand_value", test_shorthand_value),
            ("test_is_legacy_name_alias", test_is_legacy_name_alias),
        ],
        "/css/support/color-testcommon.js" => &[("fuzzy_test_valid_color", fuzzy_test_valid_color)],
        "/css/support/computed-testcommon.js" => &[("test_computed_value", test_computed_value)],
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Testlauf

/// Ein Skript der Seite.
enum Script {
    /// Ein eingebundenes Skript mit dem Pfad seiner URL
    External(String),
    Inline(String),
}

/// Was eine Datei ergab.
struct FileResult {
    subtests: Vec<Subtest>,
    /// Was sich nicht ausführen ließ, sortiert und ohne Doppelte.
    unsupported: Vec<String>,
    /// Fehler der Datei selbst, etwa eine Ausnahme außerhalb von `test()`.
    errors: Vec<String>,
}

fn run_file(path: &str) -> FileResult {
    let mut html = testdata::read("wpt", &format!("css/{path}"));
    if path.ends_with(".sub.html") {
        html = html.replace("{{host}}", "web-platform.test");
    }
    let mut errors = Vec::new();
    if html.contains("{{") {
        errors.push("unbekannte Ersetzung in einer .sub-Datei".to_owned());
    }
    let mut document = Document::new();
    parse_document(
        &mut document,
        html.chars().map(CodePoint::from),
        Options {
            scripting: ScriptingMode::Normal,
            collect_errors: false,
        },
    );
    document.perform_microtask_checkpoint();
    let quirks_mode = document.quirks_mode() == QuirksMode::Quirks;
    let base = Url::parse(&format!("http://web-platform.test/css/{path}"), None)
        .expect("URL der Testdatei");

    let mut scripts = Vec::new();
    let mut elements: Vec<(String, Value)> = Vec::new();
    let root = document.root();
    let mut current = document.first_child(root);
    while let Some(node) = current {
        current = document.following(node, root);
        let Some(element_data) = document.element(node) else {
            continue;
        };
        if let Some(id) = element_data.attribute("id").map(|id| id.to_string_lossy())
            && !id.is_empty()
            && !elements.iter().any(|(found, _)| *found == id)
        {
            let style_attribute = element_data.attribute("style").is_some();
            elements.push((id, element(quirks_mode, style_attribute)));
        }
        if !element_data.is_html("script") {
            continue;
        }
        if let Some(source) = element_data.attribute("src") {
            match Url::parse(&source.to_string_lossy(), Some(&base)) {
                Ok(url) => scripts.push(Script::External(url.pathname())),
                Err(_) => errors.push("ungültige URL eines Skripts".to_owned()),
            }
        } else {
            let mut source = String::new();
            for child in document.children(node) {
                if document.kind(child) == Some(NodeKind::Text)
                    && let Some(data) = document.data(child)
                {
                    source.push_str(&data.to_string_lossy());
                }
            }
            scripts.push(Script::Inline(source));
        }
    }

    let page = Rc::new(Page {
        elements,
        quirks_mode,
        subtests: RefCell::new(Vec::new()),
    });
    let mut interpreter = Interpreter::new();
    interpreter.define(
        "document",
        Value::host(Rc::new(DocumentObject { page: page.clone() })),
    );
    interpreter.define("CSS", Value::host(Rc::new(CssNamespace)));
    interpreter.define(
        "getComputedStyle",
        Value::native(|_, _, _| unsupported("getComputedStyle")),
    );
    let mut unsupported_reasons = Vec::new();
    for script in scripts {
        match script {
            Script::External(script) if script == "/resources/testharness.js" => {
                define_testharness(&mut interpreter, &page);
            }
            Script::External(script) => match helpers(&script) {
                Some(helpers) => {
                    for &(name, helper) in helpers {
                        let page = page.clone();
                        interpreter.define(
                            name,
                            Value::native(move |interpreter, _, arguments| {
                                helper(interpreter, &page, &arguments)?;
                                Ok(Value::Undefined)
                            }),
                        );
                    }
                }
                None => unsupported_reasons.push(format!("Skript {script}")),
            },
            Script::Inline(source) => match interpreter.run_script(&source) {
                Ok(result) => {
                    unsupported_reasons.extend(result.unsupported);
                    if let Some(exception) = result.exception {
                        errors.push(format!("Ausnahme außerhalb von test(): {exception}"));
                    }
                }
                Err(error) => unsupported_reasons.push(format!("Syntax: {error}")),
            },
        }
    }
    let subtests = page.subtests.take();
    for subtest in &subtests {
        if let Outcome::Unsupported(reason) = &subtest.outcome {
            unsupported_reasons.push(reason.clone());
        }
    }
    unsupported_reasons.sort();
    unsupported_reasons.dedup();
    FileResult {
        subtests,
        unsupported: unsupported_reasons,
        errors,
    }
}

/// Die Eigenschaft, die eine Testdatei nach ihrem Namen prüft: der längste
/// Anfang, der eine unterstützte Eigenschaft ist und auf den `-valid`,
/// `-invalid`, `-shorthand` oder `-alias` folgt.
fn tested_property(stem: &str) -> Option<&str> {
    stem.match_indices('-').rev().find_map(|(index, _)| {
        let name = stem.get(..index)?;
        let kind = stem.get(index.saturating_add(1)..)?.split('-').next()?;
        (matches!(kind, "valid" | "invalid" | "shorthand" | "alias")
            && PropertyId::from_name(name).is_some())
        .then_some(name)
    })
}

/// Die Testdateien, als Pfad unter `css/`. Dateien zu berechneten Werten
/// prüft erst die Kaskade.
fn test_files() -> Vec<String> {
    let mut paths = Vec::new();
    for directory in DIRECTORIES {
        let folder = format!("{directory}/parsing");
        for name in testdata::files("wpt", &format!("css/{folder}"), "html") {
            let path = format!("{folder}/{name}");
            let stem = name.strip_suffix(".html").unwrap_or(&name);
            let stem = stem.strip_suffix(".sub").unwrap_or(stem);
            if !name.contains("computed")
                && (tested_property(stem).is_some() || TWO_PROPERTY_FILES.contains(&&*path))
            {
                paths.push(path);
            }
        }
    }
    paths
}

/// Die Lücke, zu der ein Test gehört.
fn gap(inputs: &[String]) -> Option<&'static str> {
    GAPS.iter()
        .find(|(pattern, _)| {
            inputs
                .iter()
                .any(|input| input.to_ascii_lowercase().contains(pattern))
        })
        .map(|(pattern, _)| *pattern)
}

fn run_all() {
    let files = test_files();
    assert!(!files.is_empty(), "keine Testdateien gefunden");
    let mut failures = Vec::new();
    let mut passed = 0usize;
    let mut gaps: BTreeMap<&str, usize> = BTreeMap::new();
    let mut deviating = Vec::new();
    let mut not_run = 0usize;
    let mut total = 0usize;
    for path in &files {
        let result = run_file(path);
        for error in &result.errors {
            failures.push(format!("{path}: {error}"));
        }
        let expected: &[&str] = UNSUPPORTED
            .iter()
            .find(|(file, _)| file == path)
            .map_or(&[], |(_, reasons)| reasons);
        if result.unsupported != expected {
            failures.push(format!(
                "{path}: nicht ausführbar: {:?}, erwartet {expected:?}",
                result.unsupported
            ));
        }
        for subtest in &result.subtests {
            total = total.saturating_add(1);
            let gap = if subtest.kind == TestKind::Valid {
                gap(&subtest.inputs)
            } else {
                None
            };
            let deviation = DEVIATIONS
                .iter()
                .any(|(file, name, _)| file == path && *name == subtest.name);
            let name = &subtest.name;
            match (&subtest.outcome, gap) {
                (Outcome::Unsupported(_), _) => not_run = not_run.saturating_add(1),
                (Outcome::Pass, Some(gap)) => {
                    failures.push(format!("{path}: {name}: besteht trotz der Lücke {gap}"));
                }
                (Outcome::Pass, None) if deviation => {
                    failures.push(format!("{path}: {name}: weicht nicht mehr ab"));
                }
                (Outcome::Pass, None) => passed = passed.saturating_add(1),
                (Outcome::Fail(_), Some(gap)) => {
                    let count = gaps.entry(gap).or_default();
                    *count = count.saturating_add(1);
                }
                (Outcome::Fail(_), None) if deviation => deviating.push((path, name.clone())),
                (Outcome::Fail(message), None) => {
                    failures.push(format!("{path}: {name}: {message}"));
                }
            }
        }
    }
    for (file, name, _) in DEVIATIONS {
        if !deviating
            .iter()
            .any(|(path, found)| path == file && found == name)
        {
            failures.push(format!("{file}: {name}: Abweichung nicht gefunden"));
        }
    }
    for (file, _) in UNSUPPORTED {
        if !files.iter().any(|path| path == file) {
            failures.push(format!("{file}: Datei nicht gefunden"));
        }
    }
    let gap_total: usize = gaps.values().sum();
    eprintln!(
        "{} Dateien, {total} Tests: {passed} bestanden, {gap_total} in Lücken, {} Abweichungen, \
         {not_run} nicht ausführbar, {} Fehler",
        files.len(),
        deviating.len(),
        failures.len()
    );
    for (gap, count) in &gaps {
        eprintln!("  Lücke {gap}: {count}");
    }
    assert!(
        failures.is_empty(),
        "{} Fehler:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn property_parsing() {
    // Der Interpreter ist rekursiv; seine Tiefe ist begrenzt, braucht aber
    // mehr Stapel als die Voreinstellung für Testthreads.
    let worker = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run_all)
        .expect("Thread für die WPT-Tests");
    if let Err(panic) = worker.join() {
        std::panic::resume_unwind(panic);
    }
}
