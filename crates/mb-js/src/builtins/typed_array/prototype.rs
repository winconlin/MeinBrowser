//! %TypedArray.prototype% (Abschnitt 23.2.3): die Methoden und Accessoren,
//! die alle TypedArrays erben. Keine ist generisch: *this* muss ein
//! TypedArray sein, meist eines innerhalb der Grenzen seines ArrayBuffer
//! (ValidateTypedArray).

use super::{
    clone_array_buffer, convert_element, copy_buffer_bytes, require_typed_array, set_element,
    to_absolute_index, to_clamped_index, to_f64, typed_array_create_same_type,
    typed_array_species_create, validate_typed_array,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::buffer::ElementType;
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::builtins::array::{
    find_via_predicate_in, index_key, join_indexed_properties, sort_typed_array_elements, step,
    units_of,
};
use crate::builtins::iterator::{ENTRIES, KEYS, VALUES, iterator_create};
use crate::builtins::{define_functions, define_getter, define_symbol_property, units};
use crate::compare::{is_strictly_equal, same_value, same_value_zero};
use crate::conversion::{to_bigint, to_boolean, to_integer_or_infinity, to_number, to_string};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{
    TypedArrayRecord, is_valid_integer_index, typed_array_get_element, typed_array_ref,
    typed_array_set_element, viewed_buffer,
};
use crate::operations::{call, define_property_or_throw, get, has_property, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::Value;

/// Ein Eintrag der Tabelle der Methoden.
const fn method(name: &'static str, length: u32, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

static GETTERS: [(&str, Builtin); 4] = [
    ("buffer", method("get buffer", 0, buffer_getter)),
    (
        "byteLength",
        method("get byteLength", 0, byte_length_getter),
    ),
    (
        "byteOffset",
        method("get byteOffset", 0, byte_offset_getter),
    ),
    ("length", method("get length", 0, length_getter)),
];

static FUNCTIONS: [Builtin; 29] = [
    method("at", 1, at),
    method("copyWithin", 2, copy_within),
    method("entries", 0, entries),
    method("every", 1, every),
    method("fill", 1, fill),
    method("filter", 1, filter),
    method("find", 1, find),
    method("findIndex", 1, find_index),
    method("findLast", 1, find_last),
    method("findLastIndex", 1, find_last_index),
    method("forEach", 1, for_each),
    method("includes", 1, includes),
    method("indexOf", 1, index_of),
    method("join", 1, join),
    method("keys", 0, keys),
    method("lastIndexOf", 1, last_index_of),
    method("map", 1, map),
    method("reduce", 1, reduce),
    method("reduceRight", 1, reduce_right),
    method("reverse", 0, reverse),
    method("set", 1, set),
    method("slice", 2, slice),
    method("some", 1, some),
    method("sort", 1, sort),
    method("subarray", 2, subarray),
    method("toLocaleString", 0, to_locale_string),
    method("toReversed", 0, to_reversed),
    method("toSorted", 1, to_sorted),
    method("with", 2, with),
];

static VALUES_FUNCTION: Builtin = method("values", 0, values);

static TO_STRING_TAG: Builtin = method("get [Symbol.toStringTag]", 0, to_string_tag);

/// Die Eigenschaften von %TypedArray.prototype%; `constructor` setzt der
/// Aufrufer.
pub(super) fn define(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    prototype: Local<'_>,
) -> Completion<()> {
    for (name, getter) in &GETTERS {
        define_getter(agent, scope, realm, prototype, name, getter)?;
    }
    define_functions(agent, scope, realm, prototype, &FUNCTIONS)?;
    // `values` und @@iterator sind dasselbe Funktionsobjekt (Abschnitt
    // 23.2.3.37).
    let values = create_builtin_function(agent, scope, realm, &VALUES_FUNCTION)?;
    let key = agent.key_from_units(scope, &units("values"))?;
    let descriptor = PropertyDescriptor::data(values, true, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    define_symbol_property(agent, scope, prototype, WellKnownSymbol::Iterator, values)?;
    // `toString` ist %Array.prototype.toString% (Abschnitt 23.2.3.34).
    let array_prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    let key = agent.key_from_units(scope, &units("toString"))?;
    let to_string = get(agent, scope, array_prototype, key)?;
    let descriptor = PropertyDescriptor::data(to_string, true, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    // get %TypedArray%.prototype [ %Symbol.toStringTag% ] (Abschnitt
    // 23.2.3.38): nur konfigurierbar.
    let getter = create_builtin_function(agent, scope, realm, &TO_STRING_TAG)?;
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::ToStringTag));
    let descriptor = PropertyDescriptor::accessor(getter, undefined, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)
}

// Helfer

fn number<'s>(agent: &Agent, scope: &'s Scope, value: f64) -> Local<'s> {
    agent.heap().local(scope, Value::number(value))
}

fn boolean<'s>(agent: &Agent, scope: &'s Scope, value: bool) -> Local<'s> {
    agent.heap().local(scope, Value::boolean(value))
}

fn undefined<'s>(agent: &Agent, scope: &'s Scope) -> Local<'s> {
    agent.heap().local(scope, Value::UNDEFINED)
}

/// Die Größe eines Elements als `u64`.
fn size_of_kind(kind: ElementType) -> u64 {
    u64::try_from(kind.size()).unwrap_or(u64::MAX)
}

/// Das Argument `index`, das eine Funktion sein muss.
fn callable<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    index: usize,
) -> Completion<Local<'s>> {
    let function = arguments.argument(agent, scope, index);
    if !is_callable(agent.heap(), agent.heap().get(function)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(function)
}

/// Das Argument 0 für `sort` und `toSorted`: *undefined* oder eine
/// Funktion.
fn comparator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let comparator = arguments.argument(agent, scope, 0);
    let value = agent.heap().get(comparator);
    if !value.is_undefined() && !is_callable(agent.heap(), value) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(comparator)
}

/// TypedArrayGetElement für einen Index als ganze Zahl.
fn get_element<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    typed_array: Local<'_>,
    index: u64,
) -> Completion<Local<'s>> {
    typed_array_get_element(agent, scope, typed_array, to_f64(index))
}

/// ToBigInt oder ToNumber nach [[ContentType]] der Art `kind`, wie in
/// `fill` und `with`.
fn to_content<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
    kind: ElementType,
) -> Completion<Local<'s>> {
    if kind.is_bigint() {
        return to_bigint(agent, scope, value);
    }
    let number = to_number(agent, scope, value)?;
    Ok(agent.heap().local(scope, Value::number(number)))
}

/// Ruft `callback` mit *thisArg* aus Argument 1 für jedes Element bis
/// `length` mit Element, Index und *this* auf und gibt `visit` das
/// Element, den Index und das Ergebnis; `visit` sagt, ob die Schleife
/// weitergeht. Die Schleife von `every`, `some`, `forEach`, `map` und
/// `filter`.
fn each_element(
    agent: &mut Agent,
    arguments: &BuiltinCall<'_, '_>,
    length: u64,
    callback: Local<'_>,
    mut visit: impl FnMut(&mut Agent, &Scope, Local<'_>, u64, Local<'_>) -> Completion<bool>,
) -> Completion<()> {
    let object = arguments.this;
    for k in 0..length {
        step(agent)?;
        let go_on = agent.scope(|agent, inner| -> Completion<bool> {
            let this_arg = arguments.argument(agent, inner, 1);
            let value = get_element(agent, inner, object, k)?;
            let index = number(agent, inner, to_f64(k));
            let result = call(agent, inner, callback, this_arg, &[value, index, object])?;
            visit(agent, inner, value, k, result)
        })?;
        if !go_on {
            break;
        }
    }
    Ok(())
}

/// Kopiert `count` Bytes von `from` nach `to` im selben ArrayBuffer oder
/// zwischen zwei ArrayBuffern Byte für Byte aufsteigend, wie es `slice`
/// verlangt (Abschnitt 23.2.3.27, Schritt 14.b.ix): Liegt das Ziel im
/// selben Block hinter der Quelle und überlappen sich beide, wiederholen
/// sich die ersten Bytes. Das leisten Stücke, die nie länger als der
/// Abstand beider Bereiche sind.
fn copy_ascending(
    agent: &mut Agent,
    to_buffer: Local<'_>,
    to: u64,
    from_buffer: Local<'_>,
    from: u64,
    count: u64,
) -> Completion<()> {
    let same = {
        let heap = agent.heap();
        same_value(heap, heap.get(to_buffer), heap.get(from_buffer))
    };
    let distance = to.saturating_sub(from);
    if !same || to <= from || distance >= count {
        return copy_buffer_bytes(agent, to_buffer, to, from_buffer, from, count);
    }
    // Jedes Stück kostet einen Schritt, wie ein Element: Bei kleinem
    // Abstand sind es viele kurze Kopien.
    let mut done = 0;
    while done < count {
        step(agent)?;
        let piece = distance.min(count - done);
        copy_buffer_bytes(agent, to_buffer, to + done, from_buffer, from + done, piece)?;
        done += piece;
    }
    Ok(())
}

// Die Accessoren

/// get %TypedArray%.prototype.buffer (Abschnitt 23.2.3.2).
fn buffer_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_typed_array(agent, arguments.this)?;
    Ok(viewed_buffer(agent, scope, arguments.this))
}

/// get %TypedArray%.prototype.byteLength (Abschnitt 23.2.3.3).
fn byte_length_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let record = require_typed_array(agent, arguments.this)?;
    let size = if record.is_out_of_bounds() {
        0
    } else {
        record.byte_length()
    };
    Ok(number(agent, scope, to_f64(size)))
}

/// get %TypedArray%.prototype.byteOffset (Abschnitt 23.2.3.4).
fn byte_offset_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let record = require_typed_array(agent, arguments.this)?;
    let offset = if record.is_out_of_bounds() {
        0
    } else {
        record.byte_offset
    };
    Ok(number(agent, scope, to_f64(offset)))
}

/// get %TypedArray%.prototype.length (Abschnitt 23.2.3.21).
fn length_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let record = require_typed_array(agent, arguments.this)?;
    let length = record.length_in_bounds().unwrap_or(0);
    Ok(number(agent, scope, to_f64(length)))
}

/// get %TypedArray%.prototype [ %Symbol.toStringTag% ] (Abschnitt
/// 23.2.3.38): [[TypedArrayName]] oder *undefined*.
fn to_string_tag<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(record) = crate::object::typed_array_record_of(agent, arguments.this) else {
        return Ok(undefined(agent, scope));
    };
    let name = units(record.kind.constructor_name());
    Ok(agent.heap_mut().alloc_string(scope, &name)?)
}

// Die Methoden

/// %TypedArray%.prototype.at ( index ) (Abschnitt 23.2.3.1).
fn at<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let record = validate_typed_array(agent, arguments.this)?;
    let length = record.length();
    let index = arguments.argument(agent, scope, 0);
    let k = to_absolute_index(agent, scope, index, length)?;
    if k < 0.0 || k >= to_f64(length) {
        return Ok(undefined(agent, scope));
    }
    typed_array_get_element(agent, scope, arguments.this, k)
}

/// %TypedArray%.prototype.copyWithin ( target, start [ , end ] )
/// (Abschnitt 23.2.3.6).
fn copy_within<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    let record = validate_typed_array(agent, object)?;
    let length = record.length();
    let target = arguments.argument(agent, scope, 0);
    let target_index = to_clamped_index(agent, scope, target, length)?;
    let start = arguments.argument(agent, scope, 1);
    let start_index = to_clamped_index(agent, scope, start, length)?;
    let end = arguments.argument(agent, scope, 2);
    let end_index = if agent.heap().get(end).is_undefined() {
        length
    } else {
        to_clamped_index(agent, scope, end, length)?
    };
    let count = end_index
        .saturating_sub(start_index)
        .min(length - target_index);
    if count > 0 {
        let buffer = viewed_buffer(agent, scope, object);
        let record = validate_typed_array(agent, object)?;
        let length = record.length();
        let count = count
            .min(length.saturating_sub(start_index))
            .min(length.saturating_sub(target_index));
        let size = size_of_kind(record.kind);
        let to = target_index * size + record.byte_offset;
        let from = start_index * size + record.byte_offset;
        // Die Richtung der Spezifikation ist die von `memmove`.
        copy_buffer_bytes(agent, buffer, to, buffer, from, count * size)?;
    }
    Ok(agent.heap().local(scope, agent.heap().get(object)))
}

/// CreateArrayIterator für `entries`, `keys` und `values`.
fn array_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    kind: i32,
) -> Completion<Local<'s>> {
    validate_typed_array(agent, arguments.this)?;
    let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::ArrayIteratorPrototype);
    Ok(iterator_create(
        agent,
        scope,
        prototype,
        arguments.this,
        kind,
    )?)
}

/// %TypedArray%.prototype.entries ( ) (Abschnitt 23.2.3.7).
fn entries<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    array_iterator(agent, scope, arguments, ENTRIES)
}

/// %TypedArray%.prototype.keys ( ) (Abschnitt 23.2.3.19).
fn keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    array_iterator(agent, scope, arguments, KEYS)
}

/// %TypedArray%.prototype.values ( ) (Abschnitt 23.2.3.35).
fn values<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    array_iterator(agent, scope, arguments, VALUES)
}

/// %TypedArray%.prototype.every ( callback [ , thisArg ] ) (Abschnitt
/// 23.2.3.8).
fn every<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let length = validate_typed_array(agent, arguments.this)?.length();
    let callback = callable(agent, scope, arguments, 0)?;
    let mut all = true;
    each_element(
        agent,
        arguments,
        length,
        callback,
        |agent, _, _, _, result| {
            let heap = agent.heap();
            all = to_boolean(heap, heap.get(result));
            Ok(all)
        },
    )?;
    Ok(boolean(agent, scope, all))
}

/// %TypedArray%.prototype.some ( callback [ , thisArg ] ) (Abschnitt
/// 23.2.3.28).
fn some<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let length = validate_typed_array(agent, arguments.this)?.length();
    let callback = callable(agent, scope, arguments, 0)?;
    let mut any = false;
    each_element(
        agent,
        arguments,
        length,
        callback,
        |agent, _, _, _, result| {
            let heap = agent.heap();
            any = to_boolean(heap, heap.get(result));
            Ok(!any)
        },
    )?;
    Ok(boolean(agent, scope, any))
}

/// %TypedArray%.prototype.forEach ( callback [ , thisArg ] ) (Abschnitt
/// 23.2.3.15).
fn for_each<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let length = validate_typed_array(agent, arguments.this)?.length();
    let callback = callable(agent, scope, arguments, 0)?;
    each_element(agent, arguments, length, callback, |_, _, _, _, _| Ok(true))?;
    Ok(undefined(agent, scope))
}

/// %TypedArray%.prototype.map ( callback [ , thisArg ] ) (Abschnitt
/// 23.2.3.22).
fn map<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let record = validate_typed_array(agent, arguments.this)?;
    let length = record.length();
    let callback = callable(agent, scope, arguments, 0)?;
    let count = number(agent, scope, to_f64(length));
    let (result, _) = typed_array_species_create(
        agent,
        scope,
        arguments.realm,
        arguments.this,
        record.kind,
        &[count],
    )?;
    each_element(
        agent,
        arguments,
        length,
        callback,
        |agent, inner, _, k, mapped| {
            set_element(agent, inner, result, k, mapped)?;
            Ok(true)
        },
    )?;
    Ok(result)
}

/// %TypedArray%.prototype.filter ( callback [ , thisArg ] ) (Abschnitt
/// 23.2.3.10).
fn filter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let record = validate_typed_array(agent, arguments.this)?;
    let length = record.length();
    let callback = callable(agent, scope, arguments, 0)?;
    let mut kept = Vec::new();
    each_element(
        agent,
        arguments,
        length,
        callback,
        |agent, _, value, _, selected| {
            let heap = agent.heap();
            if to_boolean(heap, heap.get(selected)) {
                kept.push(heap.local(scope, heap.get(value)));
            }
            Ok(true)
        },
    )?;
    let captured = u64::try_from(kept.len()).unwrap_or(u64::MAX);
    let count = number(agent, scope, to_f64(captured));
    let (result, _) = typed_array_species_create(
        agent,
        scope,
        arguments.realm,
        arguments.this,
        record.kind,
        &[count],
    )?;
    for (n, &element) in (0..captured).zip(&kept) {
        step(agent)?;
        set_element(agent, scope, result, n, element)?;
    }
    Ok(result)
}

/// FindViaPredicate für `find`, `findIndex`, `findLast` und
/// `findLastIndex` (Abschnitte 23.2.3.11 bis 23.2.3.14).
fn find_in<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    from_end: bool,
) -> Completion<(f64, Local<'s>)> {
    let length = validate_typed_array(agent, arguments.this)?.length();
    find_via_predicate_in(
        agent,
        scope,
        arguments.this,
        to_f64(length),
        arguments,
        from_end,
    )
}

/// %TypedArray%.prototype.find ( predicate [ , thisArg ] ) (Abschnitt
/// 23.2.3.11).
fn find<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(find_in(agent, scope, arguments, false)?.1)
}

/// %TypedArray%.prototype.findIndex ( predicate [ , thisArg ] )
/// (Abschnitt 23.2.3.12).
fn find_index<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (index, _) = find_in(agent, scope, arguments, false)?;
    Ok(number(agent, scope, index))
}

/// %TypedArray%.prototype.findLast ( predicate [ , thisArg ] ) (Abschnitt
/// 23.2.3.13).
fn find_last<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(find_in(agent, scope, arguments, true)?.1)
}

/// %TypedArray%.prototype.findLastIndex ( predicate [ , thisArg ] )
/// (Abschnitt 23.2.3.14).
fn find_last_index<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (index, _) = find_in(agent, scope, arguments, true)?;
    Ok(number(agent, scope, index))
}

/// %TypedArray%.prototype.fill ( value [ , start [ , end ] ] ) (Abschnitt
/// 23.2.3.9).
fn fill<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    let record = validate_typed_array(agent, object)?;
    let length = record.length();
    let value = arguments.argument(agent, scope, 0);
    let value = to_content(agent, scope, value, record.kind)?;
    let start = arguments.argument(agent, scope, 1);
    let start_index = to_clamped_index(agent, scope, start, length)?;
    let end = arguments.argument(agent, scope, 2);
    let end_index = if agent.heap().get(end).is_undefined() {
        length
    } else {
        to_clamped_index(agent, scope, end, length)?
    };
    let length = validate_typed_array(agent, object)?.length();
    for k in start_index..end_index.min(length) {
        step(agent)?;
        let heap = agent.heap();
        if let Some(typed_array) = typed_array_ref(heap, heap.get(object)) {
            crate::object::typed_array_write_element(heap, typed_array, to_f64(k), heap.get(value));
        }
    }
    Ok(agent.heap().local(scope, agent.heap().get(object)))
}

/// %TypedArray%.prototype.includes ( searchElement [ , fromIndex ] )
/// (Abschnitt 23.2.3.16).
fn includes<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let length = validate_typed_array(agent, arguments.this)?.length();
    if length == 0 {
        return Ok(boolean(agent, scope, false));
    }
    let from_index = arguments.argument(agent, scope, 1);
    let start = to_clamped_index(agent, scope, from_index, length)?;
    let search = arguments.argument(agent, scope, 0);
    for k in start..length {
        step(agent)?;
        let found = agent.scope(|agent, inner| -> Completion<bool> {
            let element = get_element(agent, inner, arguments.this, k)?;
            let heap = agent.heap();
            Ok(same_value_zero(heap, heap.get(search), heap.get(element)))
        })?;
        if found {
            return Ok(boolean(agent, scope, true));
        }
    }
    Ok(boolean(agent, scope, false))
}

/// Ob das Element `k` vorhanden ist und streng gleich `search`, für
/// `indexOf` und `lastIndexOf`: HasProperty ist IsValidIntegerIndex.
fn element_is(
    agent: &mut Agent,
    typed_array: Local<'_>,
    k: u64,
    search: Local<'_>,
) -> Completion<bool> {
    agent.scope(|agent, inner| -> Completion<bool> {
        let key = index_key(agent, inner, to_f64(k))?;
        if !has_property(agent, typed_array, key)? {
            return Ok(false);
        }
        let element = get_element(agent, inner, typed_array, k)?;
        let heap = agent.heap();
        Ok(is_strictly_equal(heap, heap.get(search), heap.get(element)))
    })
}

/// %TypedArray%.prototype.indexOf ( searchElement [ , fromIndex ] )
/// (Abschnitt 23.2.3.17).
fn index_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let length = validate_typed_array(agent, arguments.this)?.length();
    if length == 0 {
        return Ok(number(agent, scope, -1.0));
    }
    let from_index = arguments.argument(agent, scope, 1);
    let start = to_clamped_index(agent, scope, from_index, length)?;
    let search = arguments.argument(agent, scope, 0);
    for k in start..length {
        step(agent)?;
        if element_is(agent, arguments.this, k, search)? {
            return Ok(number(agent, scope, to_f64(k)));
        }
    }
    Ok(number(agent, scope, -1.0))
}

/// %TypedArray%.prototype.lastIndexOf ( searchElement [ , fromIndex ] )
/// (Abschnitt 23.2.3.20).
fn last_index_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let length = validate_typed_array(agent, arguments.this)?.length();
    if length == 0 {
        return Ok(number(agent, scope, -1.0));
    }
    let last = to_f64(length - 1);
    let start = if arguments.arguments.len() < 2 {
        last
    } else {
        let from_index = arguments.argument(agent, scope, 1);
        to_absolute_index(agent, scope, from_index, length)?.min(last)
    };
    let search = arguments.argument(agent, scope, 0);
    if start < 0.0 {
        return Ok(number(agent, scope, -1.0));
    }
    // Ganz und zwischen 0 und `length` − 1.
    let mut k = start as u64;
    loop {
        step(agent)?;
        if element_is(agent, arguments.this, k, search)? {
            return Ok(number(agent, scope, to_f64(k)));
        }
        let Some(next) = k.checked_sub(1) else {
            return Ok(number(agent, scope, -1.0));
        };
        k = next;
    }
}

/// %TypedArray%.prototype.join ( separator ) (Abschnitt 23.2.3.18).
fn join<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let length = validate_typed_array(agent, arguments.this)?.length();
    let separator = arguments.argument(agent, scope, 0);
    let separator = if agent.heap().get(separator).is_undefined() {
        units(",")
    } else {
        let string = to_string(agent, scope, separator)?;
        units_of(agent, string)
    };
    let realm = arguments.realm;
    join_indexed_properties(
        agent,
        scope,
        realm,
        arguments.this,
        to_f64(length),
        &separator,
        false,
    )
}

/// %TypedArray%.prototype.toLocaleString ( [ reserved1 [ , reserved2 ] ] )
/// (Abschnitt 23.2.3.31, ohne ECMA-402): das Komma als
/// LocalizedListSeparator.
fn to_locale_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let length = validate_typed_array(agent, arguments.this)?.length();
    let realm = arguments.realm;
    let separator = units(",");
    join_indexed_properties(
        agent,
        scope,
        realm,
        arguments.this,
        to_f64(length),
        &separator,
        true,
    )
}

/// `reduce` und `reduceRight` (Abschnitte 23.2.3.23 und 23.2.3.24).
fn reduce_in<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    from_end: bool,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    let length = validate_typed_array(agent, object)?.length();
    let callback = callable(agent, scope, arguments, 0)?;
    let initial_present = arguments.arguments.len() >= 2;
    if length == 0 && !initial_present {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let index_at = |position: u64| {
        if from_end {
            length - 1 - position
        } else {
            position
        }
    };
    let mut position = 0;
    let accumulator = if initial_present {
        arguments.argument(agent, scope, 1)
    } else {
        position = 1;
        get_element(agent, scope, object, index_at(0))?
    };
    let undefined = undefined(agent, scope);
    while position < length {
        step(agent)?;
        let k = index_at(position);
        agent.scope(|agent, inner| -> Completion<()> {
            let value = get_element(agent, inner, object, k)?;
            let index = number(agent, inner, to_f64(k));
            let result = call(
                agent,
                inner,
                callback,
                undefined,
                &[accumulator, value, index, object],
            )?;
            let heap = agent.heap();
            heap.set(accumulator, heap.get(result));
            Ok(())
        })?;
        position += 1;
    }
    Ok(accumulator)
}

/// %TypedArray%.prototype.reduce ( callback [ , initialValue ] )
/// (Abschnitt 23.2.3.23).
fn reduce<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    reduce_in(agent, scope, arguments, false)
}

/// %TypedArray%.prototype.reduceRight ( callback [ , initialValue ] )
/// (Abschnitt 23.2.3.24).
fn reduce_right<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    reduce_in(agent, scope, arguments, true)
}

/// %TypedArray%.prototype.reverse ( ) (Abschnitt 23.2.3.25): Die Elemente
/// tauschen ihre Plätze, ohne dass Code läuft.
fn reverse<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    let record = validate_typed_array(agent, object)?;
    let length = record.length();
    let buffer = viewed_buffer(agent, scope, object);
    let kind = record.kind;
    let size = size_of_kind(kind);
    for lower in 0..length / 2 {
        step(agent)?;
        let upper = length - lower - 1;
        let lower_index = lower * size + record.byte_offset;
        let upper_index = upper * size + record.byte_offset;
        swap_elements(agent, buffer, lower_index, upper_index, kind);
    }
    Ok(agent.heap().local(scope, agent.heap().get(object)))
}

/// Tauscht zwei Elemente der Art `kind` im ArrayBuffer `buffer` wie die
/// Spezifikation mit Get und Set: Beide Werte gehen durch RawBytesToNumeric
/// und NumericToRawBytes, ein NaN kommt also als das NaN zurück, das
/// NumericToRawBytes schreibt, nicht mit seinen alten Bytes.
fn swap_elements(agent: &Agent, buffer: Local<'_>, first: u64, second: u64, kind: ElementType) {
    let heap = agent.heap();
    let crate::value::Kind::Object(buffer) = heap.get(buffer).kind() else {
        return;
    };
    let (Ok(first), Ok(second)) = (usize::try_from(first), usize::try_from(second)) else {
        return;
    };
    let size = kind.size();
    let (mut a, mut b) = ([0u8; 8], [0u8; 8]);
    let (Some(a_bytes), Some(b_bytes)) = (a.get_mut(..size), b.get_mut(..size)) else {
        return;
    };
    if !(crate::buffer::read_block(heap, buffer, first, a_bytes)
        && crate::buffer::read_block(heap, buffer, second, b_bytes))
    {
        return;
    }
    let lower = crate::buffer::raw_bytes_to_numeric(kind, &a, true);
    let upper = crate::buffer::raw_bytes_to_numeric(kind, &b, true);
    let lower = crate::buffer::numeric_to_raw_bytes(kind, lower, true);
    let upper = crate::buffer::numeric_to_raw_bytes(kind, upper, true);
    if let (Some(lower), Some(upper)) = (lower.get(..size), upper.get(..size)) {
        crate::buffer::write_block(heap, buffer, first, upper);
        crate::buffer::write_block(heap, buffer, second, lower);
    }
}

/// %TypedArray%.prototype.set ( source [ , offset ] ) (Abschnitt
/// 23.2.3.26).
fn set<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = arguments.this;
    require_typed_array(agent, target)?;
    let offset = arguments.argument(agent, scope, 1);
    let target_offset = to_integer_or_infinity(agent, scope, offset)?;
    if target_offset < 0.0 {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let source = arguments.argument(agent, scope, 0);
    if crate::object::typed_array_record_of(agent, source).is_some() {
        set_from_typed_array(agent, scope, arguments.realm, target, target_offset, source)?;
    } else {
        set_from_array_like(agent, scope, arguments.realm, target, target_offset, source)?;
    }
    Ok(undefined(agent, scope))
}

/// SetTypedArrayFromArrayLike (Abschnitt 23.2.3.26.1).
fn set_from_array_like(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    target: Local<'_>,
    target_offset: f64,
    source: Local<'_>,
) -> Completion<()> {
    let target_length = validate_typed_array(agent, target)?.length();
    let source = crate::conversion::to_object(agent, scope, realm, source)?;
    let source_length = crate::operations::length_of_array_like(agent, scope, source)?;
    if target_offset.is_infinite() || source_length + target_offset > to_f64(target_length) {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    // Beide ganz und unter 2⁵³.
    let (source_length, target_offset) = (source_length as u64, target_offset as u64);
    for k in 0..source_length {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let key = index_key(agent, inner, to_f64(k))?;
            let value = get(agent, inner, source, key)?;
            set_element(agent, inner, target, target_offset + k, value)
        })?;
    }
    Ok(())
}

/// SetTypedArrayFromTypedArray (Abschnitt 23.2.3.26.2). Bei gleicher Art
/// kopiert es die Bytes wie `memmove`; das ergibt dasselbe wie die Kopie
/// über CloneArrayBuffer, die die Spezifikation für denselben ArrayBuffer
/// vorsieht.
fn set_from_typed_array(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    target: Local<'_>,
    target_offset: f64,
    source: Local<'_>,
) -> Completion<()> {
    let target_buffer = viewed_buffer(agent, scope, target);
    let target_record = validate_typed_array(agent, target)?;
    let target_length = target_record.length();
    let source_buffer = viewed_buffer(agent, scope, source);
    let source_record = validate_typed_array(agent, source)?;
    let source_length = source_record.length();
    if target_offset.is_infinite() || to_f64(source_length) + target_offset > to_f64(target_length)
    {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let (target_kind, source_kind) = (target_record.kind, source_record.kind);
    if target_kind.is_bigint() != source_kind.is_bigint() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    // Ganz und unter 2⁵³.
    let target_offset = target_offset as u64;
    let (target_size, source_size) = (size_of_kind(target_kind), size_of_kind(source_kind));
    let target_index = target_offset * target_size + target_record.byte_offset;
    if source_kind == target_kind {
        let count = target_size * source_length;
        let from = source_record.byte_offset;
        return copy_buffer_bytes(
            agent,
            target_buffer,
            target_index,
            source_buffer,
            from,
            count,
        );
    }
    let same = {
        let heap = agent.heap();
        same_value(heap, heap.get(source_buffer), heap.get(target_buffer))
    };
    let (source_buffer, source_index) = if same {
        let byte_length = source_record.byte_length();
        let offset = source_record.byte_offset;
        let clone = clone_array_buffer(agent, scope, realm, source_buffer, offset, byte_length)?;
        (clone, 0)
    } else {
        (source_buffer, source_record.byte_offset)
    };
    for k in 0..source_length {
        step(agent)?;
        convert_element(
            agent,
            target_buffer,
            target_index + k * target_size,
            target_kind,
            source_buffer,
            source_index + k * source_size,
            source_kind,
        );
    }
    Ok(())
}

/// %TypedArray%.prototype.slice ( start, end ) (Abschnitt 23.2.3.27).
fn slice<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    let record = validate_typed_array(agent, object)?;
    let source_length = record.length();
    let start = arguments.argument(agent, scope, 0);
    let start_index = to_clamped_index(agent, scope, start, source_length)?;
    let end = arguments.argument(agent, scope, 1);
    let end_index = if agent.heap().get(end).is_undefined() {
        source_length
    } else {
        to_clamped_index(agent, scope, end, source_length)?
    };
    let count = end_index.saturating_sub(start_index);
    let count_value = number(agent, scope, to_f64(count));
    let (result, result_record) = typed_array_species_create(
        agent,
        scope,
        arguments.realm,
        object,
        record.kind,
        &[count_value],
    )?;
    if count == 0 {
        return Ok(result);
    }
    let record = validate_typed_array(agent, object)?;
    let end_index = end_index.min(record.length());
    let count = end_index.saturating_sub(start_index);
    let (source_kind, target_kind) = (record.kind, result_record.kind);
    if source_kind == target_kind {
        let source_buffer = viewed_buffer(agent, scope, object);
        let target_buffer = viewed_buffer(agent, scope, result);
        let size = size_of_kind(source_kind);
        let from = start_index * size + record.byte_offset;
        let to = result_record.byte_offset;
        copy_ascending(agent, target_buffer, to, source_buffer, from, count * size)?;
        return Ok(result);
    }
    for (n, k) in (start_index..end_index).enumerate() {
        step(agent)?;
        let n = u64::try_from(n).unwrap_or(u64::MAX);
        agent.scope(|agent, inner| -> Completion<()> {
            let value = get_element(agent, inner, object, k)?;
            set_element(agent, inner, result, n, value)
        })?;
    }
    Ok(result)
}

/// %TypedArray%.prototype.sort ( comparator ) (Abschnitt 23.2.3.29).
fn sort<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let comparator = comparator(agent, scope, arguments)?;
    let object = arguments.this;
    let length = validate_typed_array(agent, object)?.length();
    let sorted = sort_typed_array_elements(agent, scope, object, to_f64(length), comparator)?;
    for (j, &value) in (0..length).zip(&sorted) {
        step(agent)?;
        set_element(agent, scope, object, j, value)?;
    }
    Ok(agent.heap().local(scope, agent.heap().get(object)))
}

/// %TypedArray%.prototype.toSorted ( comparator ) (Abschnitt 23.2.3.33).
fn to_sorted<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let comparator = comparator(agent, scope, arguments)?;
    let object = arguments.this;
    let record = validate_typed_array(agent, object)?;
    let length = record.length();
    let result = typed_array_create_same_type(agent, scope, arguments.realm, record.kind, length)?;
    let sorted = sort_typed_array_elements(agent, scope, object, to_f64(length), comparator)?;
    for (j, &value) in (0..length).zip(&sorted) {
        step(agent)?;
        set_element(agent, scope, result, j, value)?;
    }
    Ok(result)
}

/// %TypedArray%.prototype.subarray ( start, end ) (Abschnitt 23.2.3.30).
fn subarray<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    let record = require_typed_array(agent, object)?;
    let buffer = viewed_buffer(agent, scope, object);
    let source_length = record.length_in_bounds().unwrap_or(0);
    let start = arguments.argument(agent, scope, 0);
    let start_index = to_clamped_index(agent, scope, start, source_length)?;
    let size = size_of_kind(record.kind);
    let begin = record.byte_offset + start_index * size;
    let begin = number(agent, scope, to_f64(begin));
    let end = arguments.argument(agent, scope, 1);
    let end_undefined = agent.heap().get(end).is_undefined();
    let realm = arguments.realm;
    if record.array_length.is_none() && end_undefined {
        let arguments = [buffer, begin];
        return Ok(typed_array_species_create(
            agent,
            scope,
            realm,
            object,
            record.kind,
            &arguments,
        )?
        .0);
    }
    let end_index = if end_undefined {
        source_length
    } else {
        to_clamped_index(agent, scope, end, source_length)?
    };
    let new_length = number(agent, scope, to_f64(end_index.saturating_sub(start_index)));
    let arguments = [buffer, begin, new_length];
    Ok(typed_array_species_create(agent, scope, realm, object, record.kind, &arguments)?.0)
}

/// %TypedArray%.prototype.toReversed ( ) (Abschnitt 23.2.3.32).
fn to_reversed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    let record = validate_typed_array(agent, object)?;
    let length = record.length();
    let result = typed_array_create_same_type(agent, scope, arguments.realm, record.kind, length)?;
    for k in 0..length {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let value = get_element(agent, inner, object, length - k - 1)?;
            set_element(agent, inner, result, k, value)
        })?;
    }
    Ok(result)
}

/// %TypedArray%.prototype.with ( index, value ) (Abschnitt 23.2.3.36).
fn with<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    let record: TypedArrayRecord = validate_typed_array(agent, object)?;
    let length = record.length();
    let index = arguments.argument(agent, scope, 0);
    let relative = to_integer_or_infinity(agent, scope, index)?;
    let actual = if relative >= 0.0 {
        relative
    } else {
        to_f64(length) + relative
    };
    let value = arguments.argument(agent, scope, 1);
    let numeric = to_content(agent, scope, value, record.kind)?;
    let valid = {
        let heap = agent.heap();
        typed_array_ref(heap, heap.get(object))
            .is_some_and(|typed_array| is_valid_integer_index(heap, typed_array, actual))
    };
    if !valid {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let result = typed_array_create_same_type(agent, scope, arguments.realm, record.kind, length)?;
    for k in 0..length {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let from = if to_f64(k) == actual {
                numeric
            } else {
                get_element(agent, inner, object, k)?
            };
            typed_array_set_element(agent, inner, result, to_f64(k), from)
        })?;
    }
    Ok(result)
}
