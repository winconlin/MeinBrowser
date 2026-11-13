//! %Boolean% (Abschnitt 20.3.1) und %Boolean.prototype% (Abschnitt 20.3.3),
//! selbst ein Boolean-Objekt mit *false*.

use super::{define_functions, link_constructor, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::to_boolean;
use crate::heap::{Local, Scope};
use crate::object::{get_prototype_from_constructor, primitive_object_create};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "Boolean",
    length: 1,
    constructor: true,
    behaviour: boolean,
};

static PROTOTYPE_FUNCTIONS: [Builtin; 2] = [
    Builtin {
        name: "toString",
        length: 0,
        constructor: false,
        behaviour: to_string,
    },
    Builtin {
        name: "valueOf",
        length: 0,
        constructor: false,
        behaviour: value_of,
    },
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::BooleanPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    agent.set_intrinsic(realm, Intrinsic::Boolean, constructor);
    Ok(())
}

/// Boolean ( value ) (Abschnitt 20.3.1.1).
fn boolean<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let b = to_boolean(agent.heap(), agent.heap().get(value));
    let b = agent.heap().local(scope, Value::boolean(b));
    let Some(new_target) = arguments.new_target else {
        return Ok(b);
    };
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::BooleanPrototype,
        arguments.realm,
    )?;
    Ok(primitive_object_create(agent, scope, b, prototype)?)
}

/// ThisBooleanValue (Abschnitt 20.3.3.3.1).
fn this_boolean_value(agent: &mut Agent, value: Local<'_>) -> Completion<bool> {
    let heap = agent.heap();
    let found = match heap.get(value).kind() {
        Kind::Boolean(b) => Some(b),
        Kind::Object(object) => match heap.primitive_data(object).map(Value::kind) {
            Some(Kind::Boolean(b)) => Some(b),
            _ => None,
        },
        _ => None,
    };
    found.ok_or_else(|| agent.throw_error(ErrorKind::TypeError))
}

/// Boolean.prototype.toString ( ) (Abschnitt 20.3.3.2).
fn to_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let b = this_boolean_value(agent, arguments.this)?;
    let text = units(if b { "true" } else { "false" });
    Ok(agent.heap_mut().alloc_string(scope, &text)?)
}

/// Boolean.prototype.valueOf ( ) (Abschnitt 20.3.3.3).
fn value_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let b = this_boolean_value(agent, arguments.this)?;
    Ok(agent.heap().local(scope, Value::boolean(b)))
}
