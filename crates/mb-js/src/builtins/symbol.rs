//! %Symbol% (Abschnitt 20.4.1) mit den bekannten Symbolen, `for` und
//! `keyFor` (Abschnitt 20.4.2) und %Symbol.prototype% (Abschnitt 20.4.3),
//! ein gewöhnliches Objekt.

use super::{define_functions, define_to_string_tag, link_constructor, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::to_string;
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, WellKnownSymbol, symbol_description};
use crate::operations::define_property_or_throw;
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "Symbol",
    length: 0,
    constructor: true,
    behaviour: symbol,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 2] = [
    Builtin {
        name: "for",
        length: 1,
        constructor: false,
        behaviour: symbol_for,
    },
    Builtin {
        name: "keyFor",
        length: 1,
        constructor: false,
        behaviour: key_for,
    },
];

static PROTOTYPE_FUNCTIONS: [Builtin; 2] = [
    Builtin {
        name: "toString",
        length: 0,
        constructor: false,
        behaviour: to_string_function,
    },
    Builtin {
        name: "valueOf",
        length: 0,
        constructor: false,
        behaviour: value_of,
    },
];

static DESCRIPTION: Builtin = Builtin {
    name: "get description",
    length: 0,
    constructor: false,
    behaviour: description,
};

static TO_PRIMITIVE: Builtin = Builtin {
    name: "[Symbol.toPrimitive]",
    length: 1,
    constructor: false,
    behaviour: value_of,
};

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::SymbolPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    // Die bekannten Symbole von Symbol.asyncDispose bis Symbol.unscopables
    // (in den Abschnitten 20.4.2.1 bis 20.4.2.18): weder beschreibbar noch
    // aufzählbar noch konfigurierbar.
    for symbol in WellKnownSymbol::ALL {
        let description = symbol.description();
        let name = description.strip_prefix("Symbol.").unwrap_or(description);
        let key = agent.key_from_units(scope, &units(name))?;
        let value = agent.well_known_symbol(scope, symbol);
        let descriptor = PropertyDescriptor::data(value, false, false, false);
        define_property_or_throw(agent, scope, constructor, key, &descriptor)?;
    }
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    // get Symbol.prototype.description (Abschnitt 20.4.3.2): ein Accessor
    // ohne Setter, konfigurierbar und nicht aufzählbar.
    let getter = create_builtin_function(agent, scope, realm, &DESCRIPTION)?;
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let key = agent.key_from_units(scope, &units("description"))?;
    let descriptor = PropertyDescriptor::accessor(getter, undefined, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    // Symbol.prototype [ %Symbol.toPrimitive% ] (Abschnitt 20.4.3.5): nur
    // konfigurierbar.
    let function = create_builtin_function(agent, scope, realm, &TO_PRIMITIVE)?;
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::ToPrimitive));
    let descriptor = PropertyDescriptor::data(function, false, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    define_to_string_tag(agent, scope, prototype, "Symbol")?;
    agent.set_intrinsic(realm, Intrinsic::Symbol, constructor);
    Ok(())
}

/// Symbol ( [ description ] ) (Abschnitt 20.4.1.1).
fn symbol<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if arguments.new_target.is_some() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let description = arguments.argument(agent, scope, 0);
    let description = if agent.heap().get(description).is_undefined() {
        None
    } else {
        Some(to_string(agent, scope, description)?)
    };
    Ok(agent.new_symbol(scope, description)?)
}

/// Symbol.for ( key ) (Abschnitt 20.4.2.2).
fn symbol_for<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let key = arguments.argument(agent, scope, 0);
    let key = to_string(agent, scope, key)?;
    let units = match agent.heap().get(key).kind() {
        Kind::String(string) => agent.heap().string_units(string),
        _ => Vec::new(),
    };
    Ok(agent.registered_symbol(scope, &units)?)
}

/// Symbol.keyFor ( sym ) (Abschnitt 20.4.2.6) mit KeyForSymbol (Abschnitt
/// 20.4.5.1): Ein Symbol der Registry hat seinen [[Key]] als Beschreibung.
fn key_for<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let heap = agent.heap();
    let Kind::Symbol(symbol) = heap.get(value).kind() else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let key = if heap.is_registered_symbol(symbol) {
        symbol_description(heap, symbol)
    } else {
        Value::UNDEFINED
    };
    Ok(heap.local(scope, key))
}

/// ThisSymbolValue (Abschnitt 20.4.3.4.1).
fn this_symbol_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let found = match heap.get(value).kind() {
        Kind::Symbol(_) => Some(heap.get(value)),
        Kind::Object(object) => heap
            .primitive_data(object)
            .filter(|data| matches!(data.kind(), Kind::Symbol(_))),
        _ => None,
    };
    match found {
        Some(symbol) => Ok(heap.local(scope, symbol)),
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// get Symbol.prototype.description (Abschnitt 20.4.3.2).
fn description<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let symbol = this_symbol_value(agent, scope, arguments.this)?;
    let heap = agent.heap();
    let description = match heap.get(symbol).kind() {
        Kind::Symbol(symbol) => symbol_description(heap, symbol),
        _ => Value::UNDEFINED,
    };
    Ok(heap.local(scope, description))
}

/// SymbolDescriptiveString (Abschnitt 20.4.3.3.1): `Symbol(` mit der
/// Beschreibung und `)`.
pub(super) fn symbol_descriptive_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    symbol: Local<'_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let mut text = units("Symbol(");
    if let Kind::Symbol(symbol) = heap.get(symbol).kind()
        && let Kind::String(description) = symbol_description(heap, symbol).kind()
    {
        text.extend(heap.string_units(description));
    }
    text.push(u16::from(b')'));
    Ok(agent.heap_mut().alloc_string(scope, &text)?)
}

/// Symbol.prototype.toString ( ) (Abschnitt 20.4.3.3).
fn to_string_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let symbol = this_symbol_value(agent, scope, arguments.this)?;
    symbol_descriptive_string(agent, scope, symbol)
}

/// Symbol.prototype.valueOf ( ) (Abschnitt 20.4.3.4) und
/// Symbol.prototype [ %Symbol.toPrimitive% ] ( hint ) (Abschnitt
/// 20.4.3.5), die beide ThisSymbolValue(*this*) zurückgeben.
fn value_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    this_symbol_value(agent, scope, arguments.this)
}
