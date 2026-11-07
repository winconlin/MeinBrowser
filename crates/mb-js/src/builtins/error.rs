//! Fehlerobjekte (Abschnitt 20.5): %Error%, die sechs %NativeError%
//! (Abschnitt 20.5.6) und %AggregateError% (Abschnitt 20.5.7) mit ihren
//! Prototypen und `Error.prototype.toString`.
//!
//! Die Prototypen sind gewöhnliche Objekte, keine Fehlerobjekte, mit dem
//! Namen in `name` und der leeren Zeichenkette in `message` (Abschnitte
//! 20.5.3, 20.5.6.3 und 20.5.7.3). Die Konstruktoren der %NativeError% und
//! %AggregateError% haben %Error% als [[Prototype]], ihre Prototypen
//! %Error.prototype% (Abschnitte 20.5.6.2, 20.5.6.3, 20.5.7.2 und
//! 20.5.7.3).

use super::{define_functions, link_constructor, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{
    Builtin, BuiltinCall, builtin_function_with_prototype, create_builtin_function,
};
use crate::conversion::to_string;
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::iterator::{get_iterator, iterator_step_value};
use crate::object::{error_create, get_prototype_from_constructor, ordinary_object_create};
use crate::operations::{
    create_array_from_list, create_non_enumerable_data_property_or_throw, define_property_or_throw,
    get, has_property,
};
use crate::realm::{Intrinsic, RealmId};
use crate::value::Kind;

static ERROR: Builtin = Builtin {
    name: "Error",
    length: 1,
    constructor: true,
    behaviour: error,
};

static AGGREGATE_ERROR: Builtin = Builtin {
    name: "AggregateError",
    length: 2,
    constructor: true,
    behaviour: aggregate_error,
};

static PROTOTYPE_FUNCTIONS: [Builtin; 1] = [Builtin {
    name: "toString",
    length: 0,
    constructor: false,
    behaviour: error_prototype_to_string,
}];

/// Ein %NativeError% mit seinen beiden Intrinsics.
struct NativeError {
    builtin: Builtin,
    constructor: Intrinsic,
    prototype: Intrinsic,
}

const fn native(
    name: &'static str,
    behaviour: crate::builtin::Behaviour,
    constructor: Intrinsic,
    prototype: Intrinsic,
) -> NativeError {
    NativeError {
        builtin: Builtin {
            name,
            length: 1,
            constructor: true,
            behaviour,
        },
        constructor,
        prototype,
    }
}

/// Die %NativeError% aus Abschnitt 20.5.5.
static NATIVE_ERRORS: [NativeError; 6] = [
    native(
        "EvalError",
        eval_error,
        Intrinsic::EvalError,
        Intrinsic::EvalErrorPrototype,
    ),
    native(
        "RangeError",
        range_error,
        Intrinsic::RangeError,
        Intrinsic::RangeErrorPrototype,
    ),
    native(
        "ReferenceError",
        reference_error,
        Intrinsic::ReferenceError,
        Intrinsic::ReferenceErrorPrototype,
    ),
    native(
        "SyntaxError",
        syntax_error,
        Intrinsic::SyntaxError,
        Intrinsic::SyntaxErrorPrototype,
    ),
    native(
        "TypeError",
        type_error,
        Intrinsic::TypeError,
        Intrinsic::TypeErrorPrototype,
    ),
    native(
        "URIError",
        uri_error,
        Intrinsic::URIError,
        Intrinsic::URIErrorPrototype,
    ),
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object_prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = error_prototype(agent, scope, object_prototype, "Error")?;
    let constructor = create_builtin_function(agent, scope, realm, &ERROR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    agent.set_intrinsic(realm, Intrinsic::Error, constructor);
    agent.set_intrinsic(realm, Intrinsic::ErrorPrototype, prototype);
    for native in &NATIVE_ERRORS {
        agent.scope(|agent, inner| -> Completion<()> {
            let own = error_prototype(agent, inner, prototype, native.builtin.name)?;
            let function =
                builtin_function_with_prototype(agent, inner, realm, &native.builtin, constructor)?;
            link_constructor(agent, inner, function, own)?;
            agent.set_intrinsic(realm, native.constructor, function);
            agent.set_intrinsic(realm, native.prototype, own);
            Ok(())
        })?;
    }
    let own = error_prototype(agent, scope, prototype, AGGREGATE_ERROR.name)?;
    let function =
        builtin_function_with_prototype(agent, scope, realm, &AGGREGATE_ERROR, constructor)?;
    link_constructor(agent, scope, function, own)?;
    agent.set_intrinsic(realm, Intrinsic::AggregateError, function);
    agent.set_intrinsic(realm, Intrinsic::AggregateErrorPrototype, own);
    Ok(())
}

/// %Error.prototype% oder ein %NativeError.prototype%: ein gewöhnliches
/// Objekt mit `name` und `message`, beide mit den üblichen Attributen.
fn error_prototype<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
    name: &str,
) -> Completion<Local<'s>> {
    let object = ordinary_object_create(agent, scope, Some(prototype))?;
    let name_value = agent.heap_mut().alloc_string(scope, &units(name))?;
    let message = agent.heap_mut().alloc_string(scope, &[])?;
    for (key, value) in [("name", name_value), ("message", message)] {
        let key = agent.key_from_units(scope, &units(key))?;
        let descriptor = PropertyDescriptor::data(value, true, false, true);
        define_property_or_throw(agent, scope, object, key, &descriptor)?;
    }
    Ok(object)
}

/// Error ( message [ , options ] ) (Abschnitt 20.5.1.1) und
/// *NativeError* ( message [ , options ] ) (Abschnitt 20.5.6.1.1) mit dem
/// Prototyp `fallback`.
fn construct<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    fallback: Intrinsic,
) -> Completion<Local<'s>> {
    let new_target = arguments.new_target.unwrap_or(arguments.function);
    let prototype =
        get_prototype_from_constructor(agent, scope, new_target, fallback, arguments.realm)?;
    let object = error_create(agent, scope, prototype)?;
    let message = arguments.argument(agent, scope, 0);
    let options = arguments.argument(agent, scope, 1);
    install_message_and_cause(agent, scope, object, message, options)?;
    Ok(object)
}

/// Die Schritte der Fehlerkonstruktoren für `message` und InstallErrorCause.
fn install_message_and_cause(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    message: Local<'_>,
    options: Local<'_>,
) -> Completion<()> {
    if !agent.heap().get(message).is_undefined() {
        let text = to_string(agent, scope, message)?;
        let key = agent.key_from_units(scope, &units("message"))?;
        create_non_enumerable_data_property_or_throw(agent, scope, object, key, text)?;
    }
    install_error_cause(agent, scope, object, options)
}

/// AggregateError ( errors, message [ , options ] ) (Abschnitt 20.5.7.1.1)
/// mit IteratorToList (Abschnitt 7.4.19).
fn aggregate_error<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let new_target = arguments.new_target.unwrap_or(arguments.function);
    let realm = arguments.realm;
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::AggregateErrorPrototype,
        realm,
    )?;
    let object = error_create(agent, scope, prototype)?;
    let message = arguments.argument(agent, scope, 1);
    let options = arguments.argument(agent, scope, 2);
    install_message_and_cause(agent, scope, object, message, options)?;
    let errors = arguments.argument(agent, scope, 0);
    let (iterator, next) = get_iterator(agent, scope, realm, errors)?;
    let mut list = Vec::new();
    while let Some(value) = iterator_step_value(agent, scope, iterator, next)? {
        list.push(value);
    }
    define_errors(agent, scope, realm, object, &list)?;
    Ok(object)
}

/// Ein neues AggregateError-Objekt mit `errors` als Array in der
/// Eigenschaft `errors`, wie es Promise.any anlegt. `realm` ist der
/// aktuelle Realm.
pub(super) fn aggregate_error_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    errors: &[Local<'_>],
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::AggregateErrorPrototype);
    let object = error_create(agent, scope, prototype)?;
    define_errors(agent, scope, realm, object, errors)?;
    Ok(object)
}

/// Die Eigenschaft `errors`: beschreibbar und konfigurierbar, nicht
/// aufzählbar.
fn define_errors(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    object: Local<'_>,
    errors: &[Local<'_>],
) -> Completion<()> {
    let array = create_array_from_list(agent, scope, realm, errors)?;
    let key = agent.key_from_units(scope, &units("errors"))?;
    let descriptor = PropertyDescriptor::data(array, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// InstallErrorCause (Abschnitt 20.5.9.1).
fn install_error_cause(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    options: Local<'_>,
) -> Completion<()> {
    if !matches!(agent.heap().get(options).kind(), Kind::Object(_)) {
        return Ok(());
    }
    let key = agent.key_from_units(scope, &units("cause"))?;
    if has_property(agent, options, key)? {
        let cause = get(agent, scope, options, key)?;
        create_non_enumerable_data_property_or_throw(agent, scope, object, key, cause)?;
    }
    Ok(())
}

fn error<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    construct(agent, scope, arguments, Intrinsic::ErrorPrototype)
}

fn eval_error<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    construct(agent, scope, arguments, Intrinsic::EvalErrorPrototype)
}

fn range_error<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    construct(agent, scope, arguments, Intrinsic::RangeErrorPrototype)
}

fn reference_error<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    construct(agent, scope, arguments, Intrinsic::ReferenceErrorPrototype)
}

fn syntax_error<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    construct(agent, scope, arguments, Intrinsic::SyntaxErrorPrototype)
}

fn type_error<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    construct(agent, scope, arguments, Intrinsic::TypeErrorPrototype)
}

fn uri_error<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    construct(agent, scope, arguments, Intrinsic::URIErrorPrototype)
}

/// Error.prototype.toString ( ) (Abschnitt 20.5.3.4).
fn error_prototype_to_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    if !matches!(agent.heap().get(object).kind(), Kind::Object(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let name = text_property(agent, scope, object, "name", "Error")?;
    let message = text_property(agent, scope, object, "message", "")?;
    if name.is_empty() {
        return Ok(agent.heap_mut().alloc_string(scope, &message)?);
    }
    if message.is_empty() {
        return Ok(agent.heap_mut().alloc_string(scope, &name)?);
    }
    let mut text = name;
    text.extend(units(": "));
    text.extend(message);
    Ok(agent.heap_mut().alloc_string(scope, &text)?)
}

/// Get und ToString für eine Eigenschaft, `missing` für *undefined*.
fn text_property(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: &str,
    missing: &str,
) -> Completion<Vec<u16>> {
    let key = agent.key_from_units(scope, &units(name))?;
    let value = get(agent, scope, object, key)?;
    if agent.heap().get(value).is_undefined() {
        return Ok(units(missing));
    }
    let text = to_string(agent, scope, value)?;
    let heap = agent.heap();
    Ok(match heap.get(text).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => Vec::new(),
    })
}
