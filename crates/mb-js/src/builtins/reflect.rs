//! %Reflect% (Abschnitt 28.1): ein gewöhnliches Objekt mit einer Funktion
//! für jede interne Methode, die ohne Umweg über Operationen wie
//! OrdinaryGet oder DefinePropertyOrThrow aufruft.

use super::{define_functions, define_to_string_tag};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall};
use crate::conversion::to_property_key;
use crate::descriptor::{from_property_descriptor, to_property_descriptor};
use crate::function::is_constructor;
use crate::heap::{Local, Scope};
use crate::key::PropertyKey;
use crate::object::{self, ordinary_object_create};
use crate::operations::{
    call, construct, create_array_from_list, create_list_from_array_like, is_callable,
};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Die Funktionen in der Reihenfolge der Spezifikation (Abschnitte 28.1.1
/// bis 28.1.13).
static FUNCTIONS: [Builtin; 13] = [
    function("apply", 3, apply),
    function("construct", 2, construct_with),
    function("defineProperty", 3, define_property),
    function("deleteProperty", 2, delete_property),
    function("get", 2, get),
    function("getOwnPropertyDescriptor", 2, get_own_property_descriptor),
    function("getPrototypeOf", 1, get_prototype_of),
    function("has", 2, has),
    function("isExtensible", 1, is_extensible),
    function("ownKeys", 1, own_keys),
    function("preventExtensions", 1, prevent_extensions),
    function("set", 3, set),
    function("setPrototypeOf", 2, set_prototype_of),
];

const fn function(
    name: &'static str,
    length: u32,
    behaviour: crate::builtin::Behaviour,
) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let reflect = ordinary_object_create(agent, scope, Some(prototype))?;
    define_functions(agent, scope, realm, reflect, &FUNCTIONS)?;
    define_to_string_tag(agent, scope, reflect, "Reflect")?;
    agent.set_intrinsic(realm, Intrinsic::Reflect, reflect);
    Ok(())
}

fn boolean<'s>(agent: &Agent, scope: &'s Scope, value: bool) -> Local<'s> {
    agent.heap().local(scope, Value::boolean(value))
}

/// Das erste Argument, wenn es ein Objekt ist, sonst ein TypeError.
fn target<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = arguments.argument(agent, scope, 0);
    if !matches!(agent.heap().get(target).kind(), Kind::Object(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(target)
}

/// Das erste Argument als Objekt und das zweite als Schlüssel.
fn target_and_key<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<(Local<'s>, PropertyKey<'s>)> {
    let target = target(agent, scope, arguments)?;
    let key = arguments.argument(agent, scope, 1);
    let key = to_property_key(agent, scope, key)?;
    Ok((target, key))
}

/// Das Argument `index`, wenn es übergeben wurde, sonst `fallback`.
fn optional<'s>(
    agent: &Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    index: usize,
    fallback: Local<'s>,
) -> Local<'s> {
    if arguments.arguments.len() > index {
        arguments.argument(agent, scope, index)
    } else {
        fallback
    }
}

/// Reflect.apply ( target, thisArgument, argumentsList ) (Abschnitt
/// 28.1.1). PrepareForTailCall hat hier keine Wirkung, denn der Aufruf aus
/// Rust beginnt ohnehin einen eigenen Lauf.
fn apply<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = arguments.argument(agent, scope, 0);
    if !is_callable(agent.heap(), agent.heap().get(target)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let this = arguments.argument(agent, scope, 1);
    let list = arguments.argument(agent, scope, 2);
    let list = create_list_from_array_like(agent, scope, list)?;
    call(agent, scope, target, this, &list)
}

/// Reflect.construct ( target, argumentsList [ , newTarget ] ) (Abschnitt
/// 28.1.2).
fn construct_with<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = arguments.argument(agent, scope, 0);
    if !is_constructor(agent, agent.heap().get(target)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let new_target = optional(agent, scope, arguments, 2, target);
    if !is_constructor(agent, agent.heap().get(new_target)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let list = arguments.argument(agent, scope, 1);
    let list = create_list_from_array_like(agent, scope, list)?;
    construct(
        agent,
        scope,
        arguments.realm,
        target,
        &list,
        Some(new_target),
    )
}

/// Reflect.defineProperty ( target, propertyKey, attributes ) (Abschnitt
/// 28.1.3).
fn define_property<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (target, key) = target_and_key(agent, scope, arguments)?;
    let attributes = arguments.argument(agent, scope, 2);
    let descriptor = to_property_descriptor(agent, scope, attributes)?;
    let defined = object::define_own_property(agent, scope, target, key, &descriptor)?;
    Ok(boolean(agent, scope, defined))
}

/// Reflect.deleteProperty ( target, propertyKey ) (Abschnitt 28.1.4).
fn delete_property<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (target, key) = target_and_key(agent, scope, arguments)?;
    let deleted = object::delete(agent, scope, target, key)?;
    Ok(boolean(agent, scope, deleted))
}

/// Reflect.get ( target, propertyKey [ , receiver ] ) (Abschnitt 28.1.5).
fn get<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (target, key) = target_and_key(agent, scope, arguments)?;
    let receiver = optional(agent, scope, arguments, 2, target);
    object::get(agent, scope, target, key, receiver)
}

/// Reflect.getOwnPropertyDescriptor ( target, propertyKey ) (Abschnitt
/// 28.1.6).
fn get_own_property_descriptor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (target, key) = target_and_key(agent, scope, arguments)?;
    let descriptor = object::get_own_property(agent, scope, target, key)?;
    from_property_descriptor(agent, scope, arguments.realm, descriptor.as_ref())
}

/// Reflect.getPrototypeOf ( target ) (Abschnitt 28.1.7).
fn get_prototype_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = target(agent, scope, arguments)?;
    match object::get_prototype_of(agent, scope, target)? {
        Some(prototype) => Ok(prototype),
        None => Ok(agent.heap().local(scope, Value::NULL)),
    }
}

/// Reflect.has ( target, propertyKey ) (Abschnitt 28.1.8).
fn has<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (target, key) = target_and_key(agent, scope, arguments)?;
    let found = object::has_property(agent, target, key)?;
    Ok(boolean(agent, scope, found))
}

/// Reflect.isExtensible ( target ) (Abschnitt 28.1.9).
fn is_extensible<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = target(agent, scope, arguments)?;
    let extensible = object::is_extensible(agent, target)?;
    Ok(boolean(agent, scope, extensible))
}

/// Reflect.ownKeys ( target ) (Abschnitt 28.1.10).
fn own_keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = target(agent, scope, arguments)?;
    let keys = object::own_property_keys(agent, scope, target)?;
    let mut values = Vec::with_capacity(keys.len());
    for key in keys {
        values.push(agent.key_as_value(scope, key)?);
    }
    create_array_from_list(agent, scope, arguments.realm, &values)
}

/// Reflect.preventExtensions ( target ) (Abschnitt 28.1.11).
fn prevent_extensions<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = target(agent, scope, arguments)?;
    let prevented = object::prevent_extensions(agent, target)?;
    Ok(boolean(agent, scope, prevented))
}

/// Reflect.set ( target, propertyKey, V [ , receiver ] ) (Abschnitt
/// 28.1.12).
fn set<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (target, key) = target_and_key(agent, scope, arguments)?;
    let value = arguments.argument(agent, scope, 2);
    let receiver = optional(agent, scope, arguments, 3, target);
    let written = object::set(agent, scope, target, key, value, receiver)?;
    Ok(boolean(agent, scope, written))
}

/// Reflect.setPrototypeOf ( target, proto ) (Abschnitt 28.1.13).
fn set_prototype_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = target(agent, scope, arguments)?;
    let prototype = arguments.argument(agent, scope, 1);
    let prototype = match agent.heap().get(prototype).kind() {
        Kind::Object(_) => Some(prototype),
        Kind::Null => None,
        _ => return Err(agent.throw_error(ErrorKind::TypeError)),
    };
    let changed = object::set_prototype_of(agent, target, prototype)?;
    Ok(boolean(agent, scope, changed))
}
