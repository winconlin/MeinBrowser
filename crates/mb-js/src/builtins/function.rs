//! %Function.prototype% (Abschnitt 20.2.3) mit allen Eigenschaften,
//! %ThrowTypeError% (Abschnitt 10.2.4.1) und
//! %Function% (Abschnitt 20.2.1) als Konstruktor, dessen Aufruf noch nicht
//! geht: Er übersetzt Quelltext zur Laufzeit (CreateDynamicFunction,
//! Abschnitt 20.2.1.1.1) und kommt mit `eval` in Schritt 9 des Plans; bis
//! dahin endet jeder Aufruf und jedes `new` wie ein Script mit einer nicht
//! unterstützten Form ([`Abrupt::Unsupported`]), nicht mit einem Fehler,
//! den ein Script fangen könnte.

use super::{define_functions, link_constructor, units};
use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::bound_function::bound_function_create;
use crate::builtin::{Builtin, BuiltinCall, builtin_entry, create_builtin_function};
use crate::conversion::to_integer_or_infinity;
use crate::descriptor::PropertyDescriptor;
use crate::function::{function_source_text, set_function_length, set_function_name};
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::prevent_extensions;
use crate::operations::{
    call, create_list_from_array_like, define_property_or_throw, get, has_own_property, is_callable,
};
use crate::realm::{Intrinsic, RealmId};
use crate::reference::ordinary_has_instance;
use crate::value::{Kind, Value};

/// %Function.prototype% selbst, eine eingebaute Funktion ohne [[Construct]]
/// mit `length` 0 und dem leeren Namen.
pub(crate) static PROTOTYPE: Builtin = Builtin {
    name: "",
    length: 0,
    constructor: false,
    behaviour: function_prototype,
};

static CONSTRUCTOR: Builtin = Builtin {
    name: "Function",
    length: 1,
    constructor: true,
    behaviour: function,
};

static PROTOTYPE_FUNCTIONS: [Builtin; 4] = [
    Builtin {
        name: "apply",
        length: 2,
        constructor: false,
        behaviour: function_prototype_apply,
    },
    Builtin {
        name: "bind",
        length: 1,
        constructor: false,
        behaviour: function_prototype_bind,
    },
    Builtin {
        name: "call",
        length: 1,
        constructor: false,
        behaviour: function_prototype_call,
    },
    Builtin {
        name: "toString",
        length: 0,
        constructor: false,
        behaviour: function_prototype_to_string,
    },
];

static HAS_INSTANCE: Builtin = Builtin {
    name: "[Symbol.hasInstance]",
    length: 1,
    constructor: false,
    behaviour: function_prototype_has_instance,
};

/// %ThrowTypeError% (Abschnitt 10.2.4.1), eine anonyme Funktion ohne
/// Argumente.
static THROW_TYPE_ERROR: Builtin = Builtin {
    name: "",
    length: 0,
    constructor: false,
    behaviour: throw_type_error,
};

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
    create_throw_type_error(agent, scope, realm)?;
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    // Function.prototype [ %Symbol.hasInstance% ] (Abschnitt 20.2.3.6) ist
    // weder beschreibbar noch aufzählbar noch konfigurierbar, damit niemand
    // die Zielfunktion einer gebundenen Funktion erfragen kann.
    let has_instance = create_builtin_function(agent, scope, realm, &HAS_INSTANCE)?;
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::HasInstance));
    let descriptor = PropertyDescriptor::data(has_instance, false, false, false);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    agent.set_intrinsic(realm, Intrinsic::Function, constructor);
    Ok(())
}

/// Legt %ThrowTypeError% an: `length` und `name` sind anders als bei jeder
/// anderen eingebauten Funktion nicht konfigurierbar, und die Funktion ist
/// nicht erweiterbar (Abschnitt 10.2.4.1).
fn create_throw_type_error(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let thrower = create_builtin_function(agent, scope, realm, &THROW_TYPE_ERROR)?;
    let zero = agent.heap().local(scope, Value::int32(0));
    let empty = agent.heap_mut().alloc_string(scope, &[])?;
    for (name, value) in [("length", zero), ("name", empty)] {
        let key = agent.key_from_units(scope, &units(name))?;
        let descriptor = PropertyDescriptor::data(value, false, false, false);
        define_property_or_throw(agent, scope, thrower, key, &descriptor)?;
    }
    prevent_extensions(agent, thrower)?;
    agent.set_intrinsic(realm, Intrinsic::ThrowTypeError, thrower);
    Ok(())
}

/// %ThrowTypeError% ( ) (Abschnitt 10.2.4.1).
fn throw_type_error<'s>(
    agent: &mut Agent,
    _: &'s Scope,
    _: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Err(agent.throw_error(ErrorKind::TypeError))
}

/// %Function.prototype% nimmt beliebige Argumente und gibt *undefined*
/// zurück (Abschnitt 20.2.3).
fn function_prototype<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    _: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Function ( ...parameterArgs, bodyArg ) (Abschnitt 20.2.1.1): noch ohne
/// CreateDynamicFunction, siehe oben.
fn function<'s>(_: &mut Agent, _: &'s Scope, _: &BuiltinCall<'_, '_>) -> Completion<Local<'s>> {
    Err(Abrupt::Unsupported(0))
}

/// Function.prototype.apply ( thisArg, argArray ) (Abschnitt 20.2.3.1).
///
/// Abweichung wie bei [`function_prototype_call`]: PrepareForTailCall gibt
/// den Rahmen des Aufrufers nicht frei.
fn function_prototype_apply<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let function = arguments.this;
    if !is_callable(agent.heap(), agent.heap().get(function)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let this = arguments.argument(agent, scope, 0);
    let array = arguments.argument(agent, scope, 1);
    let found = agent.heap().get(array);
    if found.is_undefined() || found.is_null() {
        return call(agent, scope, function, this, &[]);
    }
    let list = create_list_from_array_like(agent, scope, array)?;
    call(agent, scope, function, this, &list)
}

/// Function.prototype.bind ( thisArg, ...args ) (Abschnitt 20.2.3.2).
fn function_prototype_bind<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = arguments.this;
    if !is_callable(agent.heap(), agent.heap().get(target)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let this = arguments.argument(agent, scope, 0);
    let bound_arguments = arguments.arguments.get(1..).unwrap_or_default();
    let bound = bound_function_create(agent, scope, target, this, bound_arguments)?;
    let mut length = 0.0;
    let key = agent.key_from_units(scope, &units("length"))?;
    if has_own_property(agent, scope, target, key)? {
        let target_length = get(agent, scope, target, key)?;
        if agent.heap().get(target_length).as_number().is_some() {
            let integer = to_integer_or_infinity(agent, scope, target_length)?;
            if integer == f64::INFINITY {
                length = f64::INFINITY;
            } else if integer == f64::NEG_INFINITY {
                length = 0.0;
            } else {
                let count = u32::try_from(bound_arguments.len()).map_or(f64::MAX, f64::from);
                length = (integer - count).max(0.0);
            }
        }
    }
    set_function_length(agent, scope, bound, length)?;
    let key = agent.key_from_units(scope, &units("name"))?;
    let target_name = get(agent, scope, target, key)?;
    let name = match agent.heap().get(target_name).kind() {
        Kind::String(_) => agent.key_from_value(scope, target_name),
        _ => None,
    };
    let name = match name {
        Some(name) => name,
        None => agent.key_from_units(scope, &[])?,
    };
    set_function_name(agent, scope, bound, name, Some("bound"))?;
    Ok(bound)
}

/// Function.prototype.call ( thisArg, ...args ) (Abschnitt 20.2.3.3).
///
/// Abweichung: PrepareForTailCall gibt den Rahmen des Aufrufers nicht frei.
/// Steht `f.call(…)` in Endposition, baut erst das Return nach dem Aufruf
/// den Rahmen ab, und der Aufruf von `f` beginnt einen neuen Lauf. Sichtbar
/// wird das nur an einem RangeError bei großer Tiefe.
fn function_prototype_call<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let function = arguments.this;
    if !is_callable(agent.heap(), agent.heap().get(function)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let this = arguments.argument(agent, scope, 0);
    let rest = arguments.arguments.get(1..).unwrap_or_default();
    call(agent, scope, function, this, rest)
}

/// Function.prototype.toString ( ) (Abschnitt 20.2.3.5): der Quelltext
/// eines Funktionsobjekts, für eingebaute Funktionen `function name() {
/// [native code] }` mit ihrem [[InitialName]], für gebundene und andere
/// aufrufbare Objekte dasselbe ohne Namen (*NativeFunction*).
fn function_prototype_to_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let function = arguments.this;
    if let Some(text) = function_source_text(agent, scope, function)? {
        return Ok(text);
    }
    let value = agent.heap().get(function);
    let name = match builtin_entry(agent, value) {
        Some(builtin) => builtin.name,
        None if is_callable(agent.heap(), value) => "",
        None => return Err(agent.throw_error(ErrorKind::TypeError)),
    };
    let text = units(&format!("function {name}() {{ [native code] }}"));
    Ok(agent.heap_mut().alloc_string(scope, &text)?)
}

/// Function.prototype [ %Symbol.hasInstance% ] ( V ) (Abschnitt 20.2.3.6).
fn function_prototype_has_instance<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let found = ordinary_has_instance(agent, scope, arguments.this, value)?;
    Ok(agent.heap().local(scope, Value::boolean(found)))
}
