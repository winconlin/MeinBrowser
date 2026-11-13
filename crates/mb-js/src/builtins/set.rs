//! %Set% (Abschnitt 24.2.2), %Set.prototype% (Abschnitt 24.2.4) mit den
//! Methoden für Vereinigung, Schnitt und Differenz und
//! %SetIteratorPrototype% (Abschnitt 24.2.6.2). Die Tabelle hinter
//! [[SetData]] steht in [`super::collection`].

use super::collection::{
    SET, append, canonicalize, clear, collection_create, copy, create_iterator, cursor, delete,
    detach, find, for_each, iterator_next, key_at, size, step, table_create, this_table,
};
use super::iterator::{SET_ENTRIES, SET_VALUES};
use super::{
    define_getter, define_method, define_species, define_symbol_property, define_to_string_tag,
    link_constructor, units,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::{to_boolean, to_integer_or_infinity, to_number};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::iterator::{
    close_if_abrupt, get_iterator, get_iterator_from_method, iterator_close, iterator_step_value,
};
use crate::key::WellKnownSymbol;
use crate::object::{get_prototype_from_constructor, ordinary_object_create};
use crate::operations::{call, define_property_or_throw, get, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "Set",
    length: 0,
    constructor: true,
    behaviour: set_constructor,
};

/// Die Methoden von %Set.prototype% bis `isSupersetOf` in der Reihenfolge
/// der Spezifikation.
static PROTOTYPE_FUNCTIONS: [Builtin; 10] = [
    method("add", 1, add),
    method("clear", 0, clear_method),
    method("delete", 1, delete_method),
    method("difference", 1, difference),
    method("entries", 0, entries),
    method("forEach", 1, for_each_method),
    method("has", 1, has),
    method("intersection", 1, intersection),
    method("isDisjointFrom", 1, is_disjoint_from),
    method("isSubsetOf", 1, is_subset_of),
];

static IS_SUPERSET_OF: Builtin = method("isSupersetOf", 1, is_superset_of);
static SIZE: Builtin = method("get size", 0, size_getter);
static SYMMETRIC_DIFFERENCE: Builtin = method("symmetricDifference", 1, symmetric_difference);
static UNION: Builtin = method("union", 1, union);
static VALUES: Builtin = method("values", 0, values);

static ITERATOR_FUNCTIONS: [Builtin; 1] = [method("next", 0, set_iterator_next)];

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
    define_species(agent, scope, realm, constructor)?;
    for builtin in &PROTOTYPE_FUNCTIONS {
        define_method(agent, scope, realm, prototype, builtin)?;
    }
    define_method(agent, scope, realm, prototype, &IS_SUPERSET_OF)?;
    // `keys`, `values` und @@iterator sind dieselbe Funktion (Abschnitte
    // 24.2.4.13 und 24.2.4.18).
    let values = create_builtin_function(agent, scope, realm, &VALUES)?;
    define_data(agent, scope, prototype, "keys", values)?;
    define_getter(agent, scope, realm, prototype, "size", &SIZE)?;
    define_method(agent, scope, realm, prototype, &SYMMETRIC_DIFFERENCE)?;
    define_method(agent, scope, realm, prototype, &UNION)?;
    define_data(agent, scope, prototype, "values", values)?;
    define_symbol_property(agent, scope, prototype, WellKnownSymbol::Iterator, values)?;
    define_to_string_tag(agent, scope, prototype, "Set")?;
    agent.set_intrinsic(realm, Intrinsic::Set, constructor);
    agent.set_intrinsic(realm, Intrinsic::SetPrototype, prototype);
    let iterator_prototype = agent.intrinsic(scope, realm, Intrinsic::IteratorPrototype);
    let iterator = ordinary_object_create(agent, scope, Some(iterator_prototype))?;
    super::define_functions(agent, scope, realm, iterator, &ITERATOR_FUNCTIONS)?;
    define_to_string_tag(agent, scope, iterator, "Set Iterator")?;
    agent.set_intrinsic(realm, Intrinsic::SetIteratorPrototype, iterator);
    Ok(())
}

/// `value` als Eigenschaft `name` mit den üblichen Attributen.
fn define_data(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: &str,
    value: Local<'_>,
) -> Completion<()> {
    let key = agent.key_from_units(scope, &units(name))?;
    let descriptor = PropertyDescriptor::data(value, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// Set ( [ iterable ] ) (Abschnitt 24.2.2.1).
fn set_constructor<'s>(
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
        Intrinsic::SetPrototype,
        arguments.realm,
    )?;
    let table = table_create(agent, scope)?;
    let set = collection_create(agent, scope, prototype, SET, table)?;
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

/// Ein Set mit %Set.prototype% aus `realm` und der Tabelle `table`.
fn set_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    table: Local<'_>,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::SetPrototype);
    Ok(collection_create(agent, scope, prototype, SET, table)?)
}

/// Set.prototype.add ( value ) (Abschnitt 24.2.4.1).
fn add<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    let value = arguments.argument(agent, scope, 0);
    let value = canonicalize(agent, scope, value);
    if find(agent, table, value).is_none() {
        append(agent, scope, table, value, value)?;
    }
    let heap = agent.heap();
    Ok(heap.local(scope, heap.get(arguments.this)))
}

/// Set.prototype.clear ( ) (Abschnitt 24.2.4.2).
fn clear_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    clear(agent, table);
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Set.prototype.delete ( value ) (Abschnitt 24.2.4.4).
fn delete_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    let value = arguments.argument(agent, scope, 0);
    let value = canonicalize(agent, scope, value);
    let deleted = delete(agent, table, value);
    Ok(agent.heap().local(scope, Value::boolean(deleted)))
}

/// Set.prototype.entries ( ) (Abschnitt 24.2.4.6).
fn entries<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_iterator(agent, scope, arguments.realm, arguments.this, SET_ENTRIES)
}

/// Set.prototype.forEach ( callback [ , thisArg ] ) (Abschnitt 24.2.4.7).
fn for_each_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    for_each(agent, scope, arguments, SET)
}

/// Set.prototype.has ( value ) (Abschnitt 24.2.4.8).
fn has<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    let value = arguments.argument(agent, scope, 0);
    let value = canonicalize(agent, scope, value);
    let found = find(agent, table, value).is_some();
    Ok(agent.heap().local(scope, Value::boolean(found)))
}

/// get Set.prototype.size (Abschnitt 24.2.4.14).
fn size_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    let count = size(agent, table);
    // Mehr Einträge als 2³¹ fasst der Heap nicht.
    let count = i32::try_from(count).unwrap_or(i32::MAX);
    Ok(agent.heap().local(scope, Value::int32(count)))
}

/// Set.prototype.values ( ) (Abschnitt 24.2.4.17), auch `keys` und
/// @@iterator.
fn values<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_iterator(agent, scope, arguments.realm, arguments.this, SET_VALUES)
}

/// %SetIteratorPrototype%.next ( ) (Abschnitt 24.2.6.2.1).
fn set_iterator_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    iterator_next(agent, scope, arguments, SET)
}

// Vereinigung, Schnitt und Differenz

/// Ein Set Record (Abschnitt 24.2.1.1).
struct SetRecord<'s> {
    object: Local<'s>,
    /// [[Size]], eine ganze Zahl ab 0 oder +∞.
    size: f64,
    has: Local<'s>,
    keys: Local<'s>,
}

/// GetSetRecord ( obj ) (Abschnitt 24.2.1.2).
fn get_set_record<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'s>,
) -> Completion<SetRecord<'s>> {
    if !matches!(agent.heap().get(object).kind(), Kind::Object(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let key = agent.key_from_units(scope, &units("size"))?;
    let raw_size = get(agent, scope, object, key)?;
    let number_size = to_number(agent, scope, raw_size)?;
    if number_size.is_nan() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    // ToIntegerOrInfinity einer Zahl ruft keinen Code der Sprache auf.
    let number_size = agent.heap().local(scope, Value::number(number_size));
    let size = to_integer_or_infinity(agent, scope, number_size)?;
    if size < 0.0 {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let mut methods = Vec::with_capacity(2);
    for name in ["has", "keys"] {
        let key = agent.key_from_units(scope, &units(name))?;
        let method = get(agent, scope, object, key)?;
        if !is_callable(agent.heap(), agent.heap().get(method)) {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        methods.push(method);
    }
    let [has, keys] = methods[..] else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    Ok(SetRecord {
        object,
        size,
        has,
        keys,
    })
}

/// SetDataSize von *this* ≤ [[Size]] des anderen.
fn not_larger(agent: &Agent, table: Local<'_>, other: &SetRecord<'_>) -> bool {
    (size(agent, table) as f64) <= other.size
}

/// ToBoolean(? Call(otherRec.[[Has]], otherRec.[[SetObject]], « value »)).
fn other_has(
    agent: &mut Agent,
    scope: &Scope,
    other: &SetRecord<'_>,
    value: Local<'_>,
) -> Completion<bool> {
    let result = call(agent, scope, other.has, other.object, &[value])?;
    Ok(to_boolean(agent.heap(), agent.heap().get(result)))
}

/// Läuft mit einem Zeiger über die Einträge von `table`, wie die
/// Schleifen der Methoden mit `index` und `thisSize`, und ruft `visit` für
/// jeden; `Some` aus `visit` beendet den Lauf mit diesem Ergebnis. Ein
/// Arbeitsschritt je Eintrag.
fn each_entry<T>(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    table: Local<'_>,
    mut visit: impl FnMut(&mut Agent, &Scope, Local<'_>, usize) -> Completion<Option<T>>,
) -> Completion<Option<T>> {
    let cursor = cursor(agent, scope, table, realm)?;
    let result = loop {
        let found = agent.scope(|agent, inner| -> Completion<Option<Option<T>>> {
            agent.spend_execution_steps(1)?;
            let Some(index) = step(agent, table, cursor) else {
                return Ok(Some(None));
            };
            let entry = key_at(agent, inner, table, index);
            Ok(visit(agent, inner, entry, index)?.map(Some))
        });
        match found {
            Ok(None) => {}
            Ok(Some(result)) => break Ok(result),
            Err(abrupt) => break Err(abrupt),
        }
    };
    detach(agent, cursor);
    result
}

/// Läuft über die Werte aus `keys` des anderen Sets, kanonisch, und ruft
/// `visit` für jeden; `Some` aus `visit` beendet den Lauf mit diesem
/// Ergebnis, ohne den Iterator zu schließen. Ein Arbeitsschritt je Wert.
fn each_key<T>(
    agent: &mut Agent,
    iterator: (Local<'_>, Local<'_>),
    mut visit: impl FnMut(&mut Agent, &Scope, Local<'_>) -> Completion<Option<T>>,
) -> Completion<Option<T>> {
    let (iterator, next) = iterator;
    loop {
        let found = agent.scope(|agent, inner| -> Completion<Option<Option<T>>> {
            agent.spend_execution_steps(1)?;
            let Some(value) = iterator_step_value(agent, inner, iterator, next)? else {
                return Ok(Some(None));
            };
            let value = canonicalize(agent, inner, value);
            Ok(visit(agent, inner, value)?.map(Some))
        })?;
        if let Some(result) = found {
            return Ok(result);
        }
    }
}

/// GetIteratorFromMethod(otherRec.[[SetObject]], otherRec.[[Keys]]).
fn keys_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    other: &SetRecord<'_>,
) -> Completion<(Local<'s>, Local<'s>)> {
    get_iterator_from_method(agent, scope, other.object, other.keys)
}

/// Set.prototype.union ( other ) (Abschnitt 24.2.4.16).
fn union<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    let other = arguments.argument(agent, scope, 0);
    let other = get_set_record(agent, scope, other)?;
    let iterator = keys_iterator(agent, scope, &other)?;
    let result = copy(agent, scope, table)?;
    each_key(agent, iterator, |agent, inner, value| {
        if find(agent, result, value).is_none() {
            append(agent, inner, result, value, value)?;
        }
        Ok(None::<()>)
    })?;
    set_create(agent, scope, arguments.realm, result)
}

/// Set.prototype.intersection ( other ) (Abschnitt 24.2.4.9).
fn intersection<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let table = this_table(agent, scope, arguments.this, SET)?;
    let other = arguments.argument(agent, scope, 0);
    let other = get_set_record(agent, scope, other)?;
    let result = table_create(agent, scope)?;
    if not_larger(agent, table, &other) {
        each_entry(agent, scope, realm, table, |agent, inner, entry, _| {
            // Frühere Aufrufe von `has` können einen Eintrag entfernt und
            // wieder angehängt haben; er kommt dann zweimal vor.
            if other_has(agent, inner, &other, entry)? && find(agent, result, entry).is_none() {
                append(agent, inner, result, entry, entry)?;
            }
            Ok(None::<()>)
        })?;
    } else {
        let iterator = keys_iterator(agent, scope, &other)?;
        each_key(agent, iterator, |agent, inner, value| {
            // `keys` des anderen kann einen Wert mehrfach liefern.
            if find(agent, table, value).is_some() && find(agent, result, value).is_none() {
                append(agent, inner, result, value, value)?;
            }
            Ok(None::<()>)
        })?;
    }
    set_create(agent, scope, realm, result)
}

/// Set.prototype.difference ( other ) (Abschnitt 24.2.4.5). Im ersten Fall
/// läuft die Schleife über die Kopie, wie in der Spezifikation; nur sie
/// bekommt Lücken, und Aufrufe von `has` erreichen sie nicht.
fn difference<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let table = this_table(agent, scope, arguments.this, SET)?;
    let other = arguments.argument(agent, scope, 0);
    let other = get_set_record(agent, scope, other)?;
    let result = copy(agent, scope, table)?;
    if not_larger(agent, table, &other) {
        each_entry(agent, scope, realm, result, |agent, inner, entry, _| {
            if other_has(agent, inner, &other, entry)? {
                delete(agent, result, entry);
            }
            Ok(None::<()>)
        })?;
    } else {
        let iterator = keys_iterator(agent, scope, &other)?;
        each_key(agent, iterator, |agent, _, value| {
            delete(agent, result, value);
            Ok(None::<()>)
        })?;
    }
    set_create(agent, scope, realm, result)
}

/// Set.prototype.symmetricDifference ( other ) (Abschnitt 24.2.4.15).
fn symmetric_difference<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    let other = arguments.argument(agent, scope, 0);
    let other = get_set_record(agent, scope, other)?;
    let iterator = keys_iterator(agent, scope, &other)?;
    let result = copy(agent, scope, table)?;
    each_key(agent, iterator, |agent, inner, value| {
        let already = find(agent, result, value).is_some();
        if find(agent, table, value).is_some() {
            if already {
                delete(agent, result, value);
            }
        } else if !already {
            append(agent, inner, result, value, value)?;
        }
        Ok(None::<()>)
    })?;
    set_create(agent, scope, arguments.realm, result)
}

/// Set.prototype.isDisjointFrom ( other ) (Abschnitt 24.2.4.10).
fn is_disjoint_from<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    let other = arguments.argument(agent, scope, 0);
    let other = get_set_record(agent, scope, other)?;
    let disjoint = if not_larger(agent, table, &other) {
        let found = each_entry(
            agent,
            scope,
            arguments.realm,
            table,
            |agent, inner, entry, _| Ok(other_has(agent, inner, &other, entry)?.then_some(())),
        )?;
        found.is_none()
    } else {
        let iterator = keys_iterator(agent, scope, &other)?;
        let found = each_key(agent, iterator, |agent, _, value| {
            Ok(find(agent, table, value).map(|_| ()))
        })?;
        if found.is_some() {
            iterator_close(agent, scope, iterator.0)?;
        }
        found.is_none()
    };
    Ok(agent.heap().local(scope, Value::boolean(disjoint)))
}

/// Set.prototype.isSubsetOf ( other ) (Abschnitt 24.2.4.11).
fn is_subset_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    let other = arguments.argument(agent, scope, 0);
    let other = get_set_record(agent, scope, other)?;
    let subset = not_larger(agent, table, &other)
        && each_entry(
            agent,
            scope,
            arguments.realm,
            table,
            |agent, inner, entry, _| Ok((!other_has(agent, inner, &other, entry)?).then_some(())),
        )?
        .is_none();
    Ok(agent.heap().local(scope, Value::boolean(subset)))
}

/// Set.prototype.isSupersetOf ( other ) (Abschnitt 24.2.4.12).
fn is_superset_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let table = this_table(agent, scope, arguments.this, SET)?;
    let other = arguments.argument(agent, scope, 0);
    let other = get_set_record(agent, scope, other)?;
    if (size(agent, table) as f64) < other.size {
        return Ok(agent.heap().local(scope, Value::FALSE));
    }
    let iterator = keys_iterator(agent, scope, &other)?;
    let missing = each_key(agent, iterator, |agent, _, value| {
        Ok(find(agent, table, value).is_none().then_some(()))
    })?;
    if missing.is_some() {
        iterator_close(agent, scope, iterator.0)?;
    }
    Ok(agent.heap().local(scope, Value::boolean(missing.is_none())))
}
