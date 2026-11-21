//! %WeakMap% mit %WeakMap.prototype% (Abschnitt 24.3) und %WeakSet% mit
//! %WeakSet.prototype% (Abschnitt 24.4). Beide halten ihre Einträge in
//! einer schwachen Tabelle aus [`super::collection`]: Was nur noch als
//! Schlüssel einer WeakMap oder als Element eines WeakSet vorkommt, räumt
//! der Collector samt Wert ab (Abschnitt 9.9.3). Schlüssel vergleicht
//! SameValue; für Objekte und Symbole ist das dasselbe wie SameValueZero.

use super::collection::{
    WEAK_MAP, WEAK_SET, append, collection_create, delete, find, set_value_at, this_table,
    value_at, weak_table_create,
};
use super::{define_functions, define_to_string_tag, link_constructor, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::heap::{Local, Scope};
use crate::iterator::{
    add_entries_from_iterable, close_if_abrupt, get_iterator, iterator_step_value,
};
use crate::object::{get_prototype_from_constructor, ordinary_object_create};
use crate::operations::{call, can_be_held_weakly, get, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::Value;

static WEAK_MAP_CONSTRUCTOR: Builtin = Builtin {
    name: "WeakMap",
    length: 0,
    constructor: true,
    behaviour: weak_map,
};

/// Die Methoden von %WeakMap.prototype% in der Reihenfolge der
/// Spezifikation.
static WEAK_MAP_FUNCTIONS: [Builtin; 6] = [
    method("delete", 1, weak_map_delete),
    method("get", 1, weak_map_get),
    method("getOrInsert", 2, weak_map_get_or_insert),
    method("getOrInsertComputed", 2, weak_map_get_or_insert_computed),
    method("has", 1, weak_map_has),
    method("set", 2, weak_map_set),
];

static WEAK_SET_CONSTRUCTOR: Builtin = Builtin {
    name: "WeakSet",
    length: 0,
    constructor: true,
    behaviour: weak_set,
};

/// Die Methoden von %WeakSet.prototype% in der Reihenfolge der
/// Spezifikation.
static WEAK_SET_FUNCTIONS: [Builtin; 3] = [
    method("add", 1, weak_set_add),
    method("delete", 1, weak_set_delete),
    method("has", 1, weak_set_has),
];

const fn method(name: &'static str, length: u32, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    for (constructor, functions, tag, intrinsics) in [
        (
            &WEAK_MAP_CONSTRUCTOR,
            &WEAK_MAP_FUNCTIONS[..],
            "WeakMap",
            (Intrinsic::WeakMap, Intrinsic::WeakMapPrototype),
        ),
        (
            &WEAK_SET_CONSTRUCTOR,
            &WEAK_SET_FUNCTIONS[..],
            "WeakSet",
            (Intrinsic::WeakSet, Intrinsic::WeakSetPrototype),
        ),
    ] {
        let prototype = ordinary_object_create(agent, scope, Some(object))?;
        let function = create_builtin_function(agent, scope, realm, constructor)?;
        link_constructor(agent, scope, function, prototype)?;
        define_functions(agent, scope, realm, prototype, functions)?;
        define_to_string_tag(agent, scope, prototype, tag)?;
        agent.set_intrinsic(realm, intrinsics.0, function);
        agent.set_intrinsic(realm, intrinsics.1, prototype);
    }
    Ok(())
}

/// Ob `key` CanBeHeldWeakly (Abschnitt 9.13) erfüllt.
fn weakly(agent: &Agent, key: Local<'_>) -> bool {
    let heap = agent.heap();
    can_be_held_weakly(heap, heap.get(key))
}

fn boolean<'s>(agent: &Agent, scope: &'s Scope, value: bool) -> Completion<Local<'s>> {
    Ok(agent.heap().local(scope, Value::boolean(value)))
}

fn this_value<'s>(agent: &Agent, scope: &'s Scope, arguments: &BuiltinCall<'_, '_>) -> Local<'s> {
    let heap = agent.heap();
    heap.local(scope, heap.get(arguments.this))
}

/// WeakMap ( [ iterable ] ) (Abschnitt 24.3.1.1).
fn weak_map<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::WeakMapPrototype,
        arguments.realm,
    )?;
    let table = weak_table_create(agent, scope)?;
    let map = collection_create(agent, scope, prototype, WEAK_MAP, table)?;
    let iterable = arguments.argument(agent, scope, 0);
    let value = agent.heap().get(iterable);
    if value.is_undefined() || value.is_null() {
        return Ok(map);
    }
    let key = agent.key_from_units(scope, &units("set"))?;
    let adder = get(agent, scope, map, key)?;
    if !is_callable(agent.heap(), agent.heap().get(adder)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    add_entries_from_iterable(agent, scope, arguments.realm, map, iterable, adder)
}

/// WeakMap.prototype.delete ( key ) (Abschnitt 24.3.3.2).
fn weak_map_delete<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, WEAK_MAP)?;
    let key = arguments.argument(agent, scope, 0);
    let deleted = weakly(agent, key) && delete(agent, table, key);
    boolean(agent, scope, deleted)
}

/// WeakMap.prototype.get ( key ) (Abschnitt 24.3.3.3).
fn weak_map_get<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, WEAK_MAP)?;
    let key = arguments.argument(agent, scope, 0);
    if !weakly(agent, key) {
        return Ok(agent.heap().local(scope, Value::UNDEFINED));
    }
    Ok(match find(agent, table, key) {
        Some(index) => value_at(agent, scope, table, index),
        None => agent.heap().local(scope, Value::UNDEFINED),
    })
}

/// WeakMap.prototype.getOrInsert ( key, value ) (Abschnitt 24.3.3.4).
fn weak_map_get_or_insert<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, WEAK_MAP)?;
    let key = arguments.argument(agent, scope, 0);
    if !weakly(agent, key) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if let Some(index) = find(agent, table, key) {
        return Ok(value_at(agent, scope, table, index));
    }
    let value = arguments.argument(agent, scope, 1);
    append(agent, scope, table, key, value)?;
    Ok(value)
}

/// WeakMap.prototype.getOrInsertComputed ( key, callback ) (Abschnitt
/// 24.3.3.5).
fn weak_map_get_or_insert_computed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, WEAK_MAP)?;
    let key = arguments.argument(agent, scope, 0);
    if !weakly(agent, key) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let callback = arguments.argument(agent, scope, 1);
    if !is_callable(agent.heap(), agent.heap().get(callback)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if let Some(index) = find(agent, table, key) {
        return Ok(value_at(agent, scope, table, index));
    }
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let value = call(agent, scope, callback, undefined, &[key])?;
    // Der Aufruf kann die WeakMap verändert haben.
    match find(agent, table, key) {
        Some(index) => set_value_at(agent, table, index, value),
        None => append(agent, scope, table, key, value)?,
    }
    Ok(value)
}

/// WeakMap.prototype.has ( key ) (Abschnitt 24.3.3.6).
fn weak_map_has<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, WEAK_MAP)?;
    let key = arguments.argument(agent, scope, 0);
    let found = weakly(agent, key) && find(agent, table, key).is_some();
    boolean(agent, scope, found)
}

/// WeakMap.prototype.set ( key, value ) (Abschnitt 24.3.3.7).
fn weak_map_set<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, WEAK_MAP)?;
    let key = arguments.argument(agent, scope, 0);
    if !weakly(agent, key) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let value = arguments.argument(agent, scope, 1);
    match find(agent, table, key) {
        Some(index) => set_value_at(agent, table, index, value),
        None => append(agent, scope, table, key, value)?,
    }
    Ok(this_value(agent, scope, arguments))
}

/// WeakSet ( [ iterable ] ) (Abschnitt 24.4.1.1).
fn weak_set<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::WeakSetPrototype,
        arguments.realm,
    )?;
    let table = weak_table_create(agent, scope)?;
    let set = collection_create(agent, scope, prototype, WEAK_SET, table)?;
    let iterable = arguments.argument(agent, scope, 0);
    let value = agent.heap().get(iterable);
    if value.is_undefined() || value.is_null() {
        return Ok(set);
    }
    let key = agent.key_from_units(scope, &units("add"))?;
    let adder = get(agent, scope, set, key)?;
    if !is_callable(agent.heap(), agent.heap().get(adder)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let (iterator, next) = get_iterator(agent, scope, arguments.realm, iterable)?;
    loop {
        let done = agent.scope(|agent, inner| -> Completion<bool> {
            let Some(value) = iterator_step_value(agent, inner, iterator, next)? else {
                return Ok(true);
            };
            let status = call(agent, inner, adder, set, &[value]);
            close_if_abrupt(agent, inner, iterator, status)?;
            Ok(false)
        })?;
        if done {
            return Ok(set);
        }
    }
}

/// WeakSet.prototype.add ( value ) (Abschnitt 24.4.3.1). Der Wert eines
/// Eintrags bleibt *undefined*.
fn weak_set_add<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, WEAK_SET)?;
    let value = arguments.argument(agent, scope, 0);
    if !weakly(agent, value) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if find(agent, table, value).is_none() {
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        append(agent, scope, table, value, undefined)?;
    }
    Ok(this_value(agent, scope, arguments))
}

/// WeakSet.prototype.delete ( value ) (Abschnitt 24.4.3.3).
fn weak_set_delete<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, WEAK_SET)?;
    let value = arguments.argument(agent, scope, 0);
    let deleted = weakly(agent, value) && delete(agent, table, value);
    boolean(agent, scope, deleted)
}

/// WeakSet.prototype.has ( value ) (Abschnitt 24.4.3.4).
fn weak_set_has<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, WEAK_SET)?;
    let value = arguments.argument(agent, scope, 0);
    let found = weakly(agent, value) && find(agent, table, value).is_some();
    boolean(agent, scope, found)
}
