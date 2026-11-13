//! %Map% (Abschnitt 24.1.1) mit `Map.groupBy`, %Map.prototype% (Abschnitt
//! 24.1.3) und %MapIteratorPrototype% (Abschnitt 24.1.5.2). Die Tabelle
//! hinter [[MapData]] steht in [`super::collection`].

use super::collection::{
    MAP, append, canonicalize, clear, collection_create, create_iterator, delete, find, for_each,
    iterator_next, set_value_at, size, table_create, this_table, value_at,
};
use super::iterator::{MAP_ENTRIES, MAP_KEYS, MAP_VALUES};
use super::{
    define_functions, define_getter, define_method, define_species, define_symbol_property,
    define_to_string_tag, link_constructor, units,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::heap::{Local, Scope};
use crate::iterator::{
    add_entries_from_iterable, close_if_abrupt, get_iterator, iterator_step_value,
};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{get_prototype_from_constructor, ordinary_object_create};
use crate::operations::{call, create_data_property_or_throw, get, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::Value;

static CONSTRUCTOR: Builtin = Builtin {
    name: "Map",
    length: 0,
    constructor: true,
    behaviour: map,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 1] = [Builtin {
    name: "groupBy",
    length: 2,
    constructor: false,
    behaviour: group_by,
}];

/// Die Methoden von %Map.prototype% in der Reihenfolge der Spezifikation
/// ohne `entries`, `set`, `size` und `values`, die eigens angelegt werden.
static PROTOTYPE_FUNCTIONS: [Builtin; 8] = [
    method("clear", 0, clear_method),
    method("delete", 1, delete_method),
    method("forEach", 1, for_each_method),
    method("get", 1, get_method),
    method("getOrInsert", 2, get_or_insert),
    method("getOrInsertComputed", 2, get_or_insert_computed),
    method("has", 1, has),
    method("keys", 0, keys),
];

static ENTRIES: Builtin = method("entries", 0, entries);
static SET: Builtin = method("set", 2, set);
static VALUES: Builtin = method("values", 0, values);
static SIZE: Builtin = method("get size", 0, size_getter);

static ITERATOR_FUNCTIONS: [Builtin; 1] = [method("next", 0, map_iterator_next)];

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
    let prototype = ordinary_object_create(agent, scope, Some(object))?;
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    define_species(agent, scope, realm, constructor)?;
    // clear, constructor, delete, entries, forEach, get, getOrInsert,
    // getOrInsertComputed, has, keys, set, size, values (24.1.3.1 bis
    // 24.1.3.13), dann @@iterator und @@toStringTag.
    let (first, rest) = PROTOTYPE_FUNCTIONS.split_at(2);
    define_functions_from(agent, scope, realm, prototype, first)?;
    let entries = define_method(agent, scope, realm, prototype, &ENTRIES)?;
    define_functions_from(agent, scope, realm, prototype, rest)?;
    define_method(agent, scope, realm, prototype, &SET)?;
    define_getter(agent, scope, realm, prototype, "size", &SIZE)?;
    define_method(agent, scope, realm, prototype, &VALUES)?;
    define_symbol_property(agent, scope, prototype, WellKnownSymbol::Iterator, entries)?;
    define_to_string_tag(agent, scope, prototype, "Map")?;
    agent.set_intrinsic(realm, Intrinsic::Map, constructor);
    agent.set_intrinsic(realm, Intrinsic::MapPrototype, prototype);
    let iterator_prototype = agent.intrinsic(scope, realm, Intrinsic::IteratorPrototype);
    let iterator = ordinary_object_create(agent, scope, Some(iterator_prototype))?;
    define_functions(agent, scope, realm, iterator, &ITERATOR_FUNCTIONS)?;
    define_to_string_tag(agent, scope, iterator, "Map Iterator")?;
    agent.set_intrinsic(realm, Intrinsic::MapIteratorPrototype, iterator);
    Ok(())
}

fn define_functions_from(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    object: Local<'_>,
    table: &'static [Builtin],
) -> Completion<()> {
    for builtin in table {
        define_method(agent, scope, realm, object, builtin)?;
    }
    Ok(())
}

/// Map ( [ iterable ] ) (Abschnitt 24.1.1.1).
fn map<'s>(
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
        Intrinsic::MapPrototype,
        arguments.realm,
    )?;
    let table = table_create(agent, scope)?;
    let map = collection_create(agent, scope, prototype, MAP, table)?;
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

/// Map.groupBy ( items, callback ) (Abschnitt 24.1.2.1) mit GroupBy
/// (Abschnitt 7.3.35) und ~collection~. Die Gruppen entstehen gleich als
/// Tabelle mit Arrays als Werten; beides sieht niemand, bevor die Map fertig
/// ist, und Construct(%Map%) ohne Argumente ruft keinen Code der Sprache
/// auf.
fn group_by<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let items = arguments.argument(agent, scope, 0);
    let callback = arguments.argument(agent, scope, 1);
    let value = agent.heap().get(items);
    if value.is_undefined() || value.is_null() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if !is_callable(agent.heap(), agent.heap().get(callback)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let table = table_create(agent, scope)?;
    let (iterator, next) = get_iterator(agent, scope, realm, items)?;
    // Mehr als 2⁵³ − 1 Werte bekommt kein Schlüssel als genaue Zahl.
    const LIMIT: u64 = (1 << 53) - 1;
    let mut count: u64 = 0;
    loop {
        let done = agent.scope(|agent, inner| -> Completion<bool> {
            if count >= LIMIT {
                let error = Err(agent.throw_error(ErrorKind::TypeError));
                return close_if_abrupt(agent, inner, iterator, error);
            }
            agent.spend_execution_steps(1)?;
            let Some(value) = iterator_step_value(agent, inner, iterator, next)? else {
                return Ok(true);
            };
            // u64 bis 2⁵³ − 1 ist als f64 genau.
            let index = agent.heap().local(inner, Value::number(count as f64));
            let undefined = agent.heap().local(inner, Value::UNDEFINED);
            let key = call(agent, inner, callback, undefined, &[value, index]);
            let key = close_if_abrupt(agent, inner, iterator, key)?;
            let key = canonicalize(agent, inner, key);
            // AddValueToKeyedGroup (Abschnitt 7.3.34).
            let elements = match find(agent, table, key) {
                Some(found) => value_at(agent, inner, table, found),
                None => {
                    let prototype = agent.intrinsic(inner, realm, Intrinsic::ArrayPrototype);
                    let elements = crate::object::array_create(agent, inner, 0, Some(prototype))?;
                    append(agent, inner, table, key, elements)?;
                    elements
                }
            };
            let length = crate::operations::length_of_array_like(agent, inner, elements)?;
            // Mehr Elemente als Indizes bis 2³² − 2 kann kein Array halten.
            if length >= f64::from(u32::MAX) {
                return Err(agent.throw_error(ErrorKind::RangeError));
            }
            // Eine ganze Zahl unter 2³² − 1 bleibt unverändert.
            let position = crate::number::to_uint32(length);
            create_data_property_or_throw(
                agent,
                inner,
                elements,
                PropertyKey::Index(position),
                value,
            )?;
            Ok(false)
        })?;
        if done {
            break;
        }
        count = count.saturating_add(1);
    }
    let prototype = agent.intrinsic(scope, realm, Intrinsic::MapPrototype);
    Ok(collection_create(agent, scope, prototype, MAP, table)?)
}

/// Map.prototype.clear ( ) (Abschnitt 24.1.3.1).
fn clear_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, MAP)?;
    clear(agent, table);
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Map.prototype.delete ( key ) (Abschnitt 24.1.3.3).
fn delete_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, MAP)?;
    let key = arguments.argument(agent, scope, 0);
    let key = canonicalize(agent, scope, key);
    let deleted = delete(agent, table, key);
    Ok(agent.heap().local(scope, Value::boolean(deleted)))
}

/// Map.prototype.entries ( ) (Abschnitt 24.1.3.4), auch @@iterator.
fn entries<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_iterator(agent, scope, arguments.realm, arguments.this, MAP_ENTRIES)
}

/// Map.prototype.forEach ( callback [ , thisArg ] ) (Abschnitt 24.1.3.5).
fn for_each_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    for_each(agent, scope, arguments, MAP)
}

/// Map.prototype.get ( key ) (Abschnitt 24.1.3.6).
fn get_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, MAP)?;
    let key = arguments.argument(agent, scope, 0);
    let key = canonicalize(agent, scope, key);
    Ok(match find(agent, table, key) {
        Some(index) => value_at(agent, scope, table, index),
        None => agent.heap().local(scope, Value::UNDEFINED),
    })
}

/// Map.prototype.getOrInsert ( key, value ) (Abschnitt 24.1.3.7).
fn get_or_insert<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, MAP)?;
    let key = arguments.argument(agent, scope, 0);
    let key = canonicalize(agent, scope, key);
    if let Some(index) = find(agent, table, key) {
        return Ok(value_at(agent, scope, table, index));
    }
    let value = arguments.argument(agent, scope, 1);
    append(agent, scope, table, key, value)?;
    Ok(value)
}

/// Map.prototype.getOrInsertComputed ( key, callback ) (Abschnitt
/// 24.1.3.8).
fn get_or_insert_computed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, MAP)?;
    let callback = arguments.argument(agent, scope, 1);
    if !is_callable(agent.heap(), agent.heap().get(callback)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let key = arguments.argument(agent, scope, 0);
    let key = canonicalize(agent, scope, key);
    if let Some(index) = find(agent, table, key) {
        return Ok(value_at(agent, scope, table, index));
    }
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let value = call(agent, scope, callback, undefined, &[key])?;
    // Der Aufruf kann die Map verändert haben.
    match find(agent, table, key) {
        Some(index) => set_value_at(agent, table, index, value),
        None => append(agent, scope, table, key, value)?,
    }
    Ok(value)
}

/// Map.prototype.has ( key ) (Abschnitt 24.1.3.9).
fn has<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, MAP)?;
    let key = arguments.argument(agent, scope, 0);
    let key = canonicalize(agent, scope, key);
    let found = find(agent, table, key).is_some();
    Ok(agent.heap().local(scope, Value::boolean(found)))
}

/// Map.prototype.keys ( ) (Abschnitt 24.1.3.10).
fn keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_iterator(agent, scope, arguments.realm, arguments.this, MAP_KEYS)
}

/// Map.prototype.set ( key, value ) (Abschnitt 24.1.3.11).
fn set<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, MAP)?;
    let key = arguments.argument(agent, scope, 0);
    let key = canonicalize(agent, scope, key);
    let value = arguments.argument(agent, scope, 1);
    match find(agent, table, key) {
        Some(index) => set_value_at(agent, table, index, value),
        None => append(agent, scope, table, key, value)?,
    }
    let heap = agent.heap();
    Ok(heap.local(scope, heap.get(arguments.this)))
}

/// get Map.prototype.size (Abschnitt 24.1.3.12).
fn size_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, MAP)?;
    let count = size(agent, table);
    // Mehr Einträge als 2³¹ fasst der Heap nicht.
    let count = i32::try_from(count).unwrap_or(i32::MAX);
    Ok(agent.heap().local(scope, Value::int32(count)))
}

/// Map.prototype.values ( ) (Abschnitt 24.1.3.13).
fn values<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_iterator(agent, scope, arguments.realm, arguments.this, MAP_VALUES)
}

/// %MapIteratorPrototype%.next ( ) (Abschnitt 24.1.5.2.1).
fn map_iterator_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    iterator_next(agent, scope, arguments, MAP)
}
