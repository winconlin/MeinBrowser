//! %Object% (Abschnitt 20.1.1) und `Object.prototype.toString`. Die
//! übrigen Funktionen des Konstruktors und des Prototyps kommen mit der
//! ersten Welle eingebauter Objekte.

use super::{define_functions, link_constructor, units};
use crate::agent::{Agent, Completion};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::compare::same_value;
use crate::conversion::to_object;
use crate::heap::{Local, ObjectClass, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{ordinary_create_from_constructor, ordinary_object_create};
use crate::operations::{get, is_array, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "Object",
    length: 1,
    constructor: true,
    behaviour: object,
};

static PROTOTYPE_FUNCTIONS: [Builtin; 1] = [Builtin {
    name: "toString",
    length: 0,
    constructor: false,
    behaviour: object_prototype_to_string,
}];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    agent.set_intrinsic(realm, Intrinsic::Object, constructor);
    Ok(())
}

/// Object ( [ value ] ) (Abschnitt 20.1.1.1).
fn object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if let Some(new_target) = arguments.new_target {
        let heap = agent.heap();
        if !same_value(heap, heap.get(new_target), heap.get(arguments.function)) {
            return ordinary_create_from_constructor(
                agent,
                scope,
                new_target,
                Intrinsic::ObjectPrototype,
                arguments.realm,
            );
        }
    }
    let value = arguments.argument(agent, scope, 0);
    let found = agent.heap().get(value);
    if found.is_undefined() || found.is_null() {
        let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::ObjectPrototype);
        return Ok(ordinary_object_create(agent, scope, Some(prototype))?);
    }
    to_object(agent, scope, arguments.realm, value)
}

/// Der Teil *builtinTag* aus Object.prototype.toString.
fn builtin_tag(agent: &Agent, object: Local<'_>) -> &'static str {
    let heap = agent.heap();
    let value = heap.get(object);
    if is_array(heap, value) {
        return "Array";
    }
    let Kind::Object(reference) = value.kind() else {
        return "Object";
    };
    let class = heap.object_class(reference);
    if class == Some(ObjectClass::Arguments) {
        return "Arguments";
    }
    if is_callable(heap, value) {
        return "Function";
    }
    match class {
        Some(ObjectClass::Error) => "Error",
        Some(ObjectClass::Primitive) => match heap.primitive_data(reference).map(Value::kind) {
            Some(Kind::Boolean(_)) => "Boolean",
            Some(Kind::Number(_)) => "Number",
            Some(Kind::String(_)) => "String",
            // Symbol-Objekte haben [[SymbolData]], das hier nicht zählt.
            _ => "Object",
        },
        _ => "Object",
    }
}

/// Object.prototype.toString ( ) (Abschnitt 20.1.3.6).
fn object_prototype_to_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = agent.heap().get(arguments.this);
    if this.is_undefined() {
        return Ok(agent
            .heap_mut()
            .alloc_string(scope, &units("[object Undefined]"))?);
    }
    if this.is_null() {
        return Ok(agent
            .heap_mut()
            .alloc_string(scope, &units("[object Null]"))?);
    }
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    let builtin = builtin_tag(agent, object);
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::ToStringTag);
    let tag = get(agent, scope, object, PropertyKey::Symbol(symbol))?;
    let mut text = units("[object ");
    let heap = agent.heap();
    match heap.get(tag).kind() {
        Kind::String(tag) => text.extend(heap.string_units(tag)),
        _ => text.extend(units(builtin)),
    }
    text.push(u16::from(b']'));
    Ok(agent.heap_mut().alloc_string(scope, &text)?)
}
