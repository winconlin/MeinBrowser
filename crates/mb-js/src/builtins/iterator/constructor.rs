//! %Iterator% (Abschnitt 27.1.3.1) mit `Iterator.concat`, `Iterator.from`,
//! `Iterator.zip` und `Iterator.zipKeyed` (Abschnitt 27.1.3.2) und
//! %WrapForValidIteratorPrototype% (Abschnitt 27.1.3.2.2.1).
//!
//! Die Hülle aus `Iterator.from` ist ein Objekt der Art
//! [`ObjectClass::Iterator`](crate::heap::ObjectClass::Iterator) mit der Art
//! [`WRAP`]; ihre Felder halten [[Iterated]], also [[Iterator]] und
//! [[NextMethod]] des eingewickelten Iterators.

use super::helper::{
    BUFFER, CONCAT, CONCAT_ENTRY, FUNCTION, INNER, LIMIT, LONGEST, OPTION, SHORTEST, STRICT, ZIP,
    ZIP_ENTRY, create_helper, write, write_local,
};
use super::{INDEX, KIND, TARGET, WRAP, field, iterator_create, set_field, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall};
use crate::compare::same_value;
use crate::heap::{Local, Scope};
use crate::iterator::{
    create_iterator_result_object, get_iterator, get_iterator_flattenable, iterator_close,
    iterator_close_all, iterator_step_value,
};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{
    get_own_property, ordinary_create_from_constructor, ordinary_object_create, own_property_keys,
};
use crate::operations::{call, get, get_method};
use crate::realm::{Intrinsic, RealmId};
use crate::reference::ordinary_has_instance;
use crate::value::{Kind, Value};

pub(super) static CONSTRUCTOR: Builtin = Builtin {
    name: "Iterator",
    length: 0,
    constructor: true,
    behaviour: iterator,
};

pub(super) static FUNCTIONS: [Builtin; 4] = [
    Builtin {
        name: "concat",
        length: 0,
        constructor: false,
        behaviour: concat,
    },
    Builtin {
        name: "from",
        length: 1,
        constructor: false,
        behaviour: from,
    },
    Builtin {
        name: "zip",
        length: 1,
        constructor: false,
        behaviour: zip,
    },
    Builtin {
        name: "zipKeyed",
        length: 1,
        constructor: false,
        behaviour: zip_keyed,
    },
];

pub(super) static WRAP_FUNCTIONS: [Builtin; 2] = [
    Builtin {
        name: "next",
        length: 0,
        constructor: false,
        behaviour: wrap_next,
    },
    Builtin {
        name: "return",
        length: 0,
        constructor: false,
        behaviour: wrap_return,
    },
];

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

/// Iterator ( ) (Abschnitt 27.1.3.1.1): nur als Oberklasse.
fn iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let heap = agent.heap();
    if same_value(heap, heap.get(new_target), heap.get(arguments.function)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    ordinary_create_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::IteratorPrototype,
        arguments.realm,
    )
}

/// Iterator.from ( obj ) (Abschnitt 27.1.3.2.2).
fn from<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let value = arguments.argument(agent, scope, 0);
    let (iterator, next) = get_iterator_flattenable(agent, scope, realm, value, true)?;
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Iterator);
    if ordinary_has_instance(agent, scope, constructor, iterator)? {
        return Ok(iterator);
    }
    let prototype = agent.intrinsic(scope, realm, Intrinsic::WrapForValidIteratorPrototype);
    let wrapper = iterator_create(agent, scope, prototype, iterator, WRAP)?;
    set_field(agent, wrapper, INDEX, agent.heap().get(next));
    Ok(wrapper)
}

/// [[Iterated]] der Hülle `this`: [[Iterator]] und [[NextMethod]]; ein
/// TypeError für andere Werte (RequireInternalSlot).
fn iterated<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    this: Local<'_>,
) -> Completion<(Local<'s>, Local<'s>)> {
    let kind = field(agent, scope, this, KIND).and_then(|kind| agent.heap().get(kind).as_int32());
    let iterator = field(agent, scope, this, TARGET);
    let next = field(agent, scope, this, INDEX);
    match (kind, iterator, next) {
        (Some(WRAP), Some(iterator), Some(next)) => Ok((iterator, next)),
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// %WrapForValidIteratorPrototype%.next ( ) (Abschnitt 27.1.3.2.2.1.1).
fn wrap_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (iterator, next) = iterated(agent, scope, arguments.this)?;
    call(agent, scope, next, iterator, &[])
}

/// %WrapForValidIteratorPrototype%.return ( ) (Abschnitt 27.1.3.2.2.1.2).
fn wrap_return<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (iterator, _) = iterated(agent, scope, arguments.this)?;
    let key = agent.key_from_units(scope, &units("return"))?;
    match get_method(agent, scope, iterator, key)? {
        Some(method) => call(agent, scope, method, iterator, &[]),
        None => {
            let undefined = agent.heap().local(scope, Value::UNDEFINED);
            create_iterator_result_object(agent, scope, arguments.realm, undefined, true)
        }
    }
}

/// Iterator.concat ( ...items ) (Abschnitt 27.1.3.2.1).
fn concat<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let items = arguments.arguments;
    let slots = items
        .len()
        .checked_mul(CONCAT_ENTRY)
        .ok_or_else(|| agent.throw_error(ErrorKind::RangeError))?;
    let iterables = agent.heap_mut().alloc_slots(scope, slots)?;
    for (index, &item) in items.iter().enumerate() {
        if !is_object(agent, item) {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        let symbol = agent.well_known_symbol(scope, WellKnownSymbol::Iterator);
        let Some(method) = get_method(agent, scope, item, PropertyKey::Symbol(symbol))? else {
            return Err(agent.throw_error(ErrorKind::TypeError));
        };
        // `slots` passte schon, also auch jede kleinere Stelle.
        let base = index.saturating_mul(CONCAT_ENTRY);
        write_local(agent, iterables, base, method);
        write_local(agent, iterables, base.saturating_add(1), item);
    }
    let (helper, state) = create_helper(agent, scope, arguments.realm, CONCAT)?;
    write_local(agent, state, FUNCTION, iterables);
    // Mehr Argumente als 2³² − 1 nimmt kein Aufruf an.
    write(agent, state, LIMIT, Value::number(items.len() as f64));
    Ok(helper)
}

/// IfAbruptCloseIterators (Abschnitt 7.4.14).
fn close_all_if_abrupt<T>(
    agent: &mut Agent,
    scope: &Scope,
    iterators: &[Local<'_>],
    result: Completion<T>,
) -> Completion<T> {
    match result {
        Ok(value) => Ok(value),
        Err(abrupt) => {
            iterator_close_all(agent, scope, iterators, Err(abrupt))?;
            // IteratorCloseAll gibt eine abrupte Completion immer abrupt
            // zurück; hierher kommt nur, was sie unverändert ließ.
            Err(abrupt)
        }
    }
}

/// Die gemeinsamen Schritte 1 bis 6 von `zip` und `zipKeyed`: der Modus
/// und *paddingOption* aus den Optionen (GetOptionsObject, Abschnitt
/// 7.3.36).
fn zip_options<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    iterables: Local<'_>,
    options: Local<'_>,
) -> Completion<(i32, Local<'s>)> {
    if !is_object(agent, iterables) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let options = match agent.heap().get(options).kind() {
        Kind::Undefined => ordinary_object_create(agent, scope, None)?,
        Kind::Object(_) => agent.heap().local(scope, agent.heap().get(options)),
        _ => return Err(agent.throw_error(ErrorKind::TypeError)),
    };
    let key = agent.key_from_units(scope, &units("mode"))?;
    let mode = get(agent, scope, options, key)?;
    let heap = agent.heap();
    let mode = match heap.get(mode).kind() {
        Kind::Undefined => SHORTEST,
        Kind::String(string) => {
            let text = heap.string_units(string);
            if text.iter().copied().eq("shortest".encode_utf16()) {
                SHORTEST
            } else if text.iter().copied().eq("longest".encode_utf16()) {
                LONGEST
            } else if text.iter().copied().eq("strict".encode_utf16()) {
                STRICT
            } else {
                return Err(agent.throw_error(ErrorKind::TypeError));
            }
        }
        _ => return Err(agent.throw_error(ErrorKind::TypeError)),
    };
    let mut padding = agent.heap().local(scope, Value::UNDEFINED);
    if mode == LONGEST {
        let key = agent.key_from_units(scope, &units("padding"))?;
        padding = get(agent, scope, options, key)?;
        let found = agent.heap().get(padding);
        if !found.is_undefined() && !matches!(found.kind(), Kind::Object(_)) {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
    }
    Ok((mode, padding))
}

/// Legt den Helfer von IteratorZip (Abschnitt 27.1.3.4.1) an: die
/// Iteratoren mit ihren `next`, alle in *openIters*, der Modus, `padding`
/// und für `zipKeyed` die Schlüssel.
fn zip_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    iterators: &[(Local<'_>, Local<'_>)],
    mode: i32,
    padding: Option<Local<'_>>,
    keys: Option<Local<'_>>,
) -> Completion<Local<'s>> {
    let slots = iterators
        .len()
        .checked_mul(ZIP_ENTRY)
        .ok_or_else(|| agent.throw_error(ErrorKind::RangeError))?;
    let list = agent.heap_mut().alloc_slots(scope, slots)?;
    for (index, &(iterator, next)) in iterators.iter().enumerate() {
        let base = index.saturating_mul(ZIP_ENTRY);
        write_local(agent, list, base, iterator);
        write_local(agent, list, base.saturating_add(1), next);
        write(agent, list, base.saturating_add(2), Value::boolean(true));
    }
    let (helper, state) = create_helper(agent, scope, realm, ZIP)?;
    write_local(agent, state, FUNCTION, list);
    // So viele Iteratoren passen nicht in den Speicher, dass f64 sie nicht
    // genau zählte.
    write(agent, state, LIMIT, Value::number(iterators.len() as f64));
    write(agent, state, OPTION, Value::int32(mode));
    if let Some(padding) = padding {
        write_local(agent, state, BUFFER, padding);
    }
    if let Some(keys) = keys {
        write_local(agent, state, INNER, keys);
    }
    Ok(helper)
}

/// Iterator.zip ( iterables [ , options ] ) (Abschnitt 27.1.3.2.4).
fn zip<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let iterables = arguments.argument(agent, scope, 0);
    let options = arguments.argument(agent, scope, 1);
    let (mode, padding_option) = zip_options(agent, scope, iterables, options)?;
    let mut records: Vec<(Local<'s>, Local<'s>)> = Vec::new();
    let mut iterators: Vec<Local<'s>> = Vec::new();
    let (input, input_next) = get_iterator(agent, scope, realm, iterables)?;
    loop {
        agent.spend_execution_steps(1)?;
        let next = iterator_step_value(agent, scope, input, input_next);
        let Some(value) = close_all_if_abrupt(agent, scope, &iterators, next)? else {
            break;
        };
        let record = get_iterator_flattenable(agent, scope, realm, value, false);
        let record = match record {
            Ok(record) => record,
            Err(abrupt) => {
                let mut closing = Vec::with_capacity(iterators.len().saturating_add(1));
                closing.push(input);
                closing.extend_from_slice(&iterators);
                return close_all_if_abrupt(agent, scope, &closing, Err(abrupt));
            }
        };
        iterators.push(record.0);
        records.push(record);
    }
    let count = records.len();
    let padding = if mode == LONGEST {
        let padding = agent.heap_mut().alloc_slots(scope, count)?;
        if !agent.heap().get(padding_option).is_undefined() {
            let record = get_iterator(agent, scope, realm, padding_option);
            let (padding_iterator, padding_next) =
                close_all_if_abrupt(agent, scope, &iterators, record)?;
            let mut using_iterator = true;
            for index in 0..count {
                agent.spend_execution_steps(1)?;
                if !using_iterator {
                    break;
                }
                let next = iterator_step_value(agent, scope, padding_iterator, padding_next);
                match close_all_if_abrupt(agent, scope, &iterators, next)? {
                    Some(value) => write_local(agent, padding, index, value),
                    None => using_iterator = false,
                }
            }
            // Was fehlt, bleibt *undefined*.
            if using_iterator {
                let closed = iterator_close(agent, scope, padding_iterator);
                close_all_if_abrupt(agent, scope, &iterators, closed)?;
            }
        }
        Some(padding)
    } else {
        None
    };
    zip_create(agent, scope, realm, &records, mode, padding, None)
}

/// Iterator.zipKeyed ( iterables [ , options ] ) (Abschnitt 27.1.3.2.5).
fn zip_keyed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let iterables = arguments.argument(agent, scope, 0);
    let options = arguments.argument(agent, scope, 1);
    let (mode, padding_option) = zip_options(agent, scope, iterables, options)?;
    let mut records: Vec<(Local<'s>, Local<'s>)> = Vec::new();
    let mut iterators: Vec<Local<'s>> = Vec::new();
    let mut keys: Vec<PropertyKey<'s>> = Vec::new();
    let all_keys = own_property_keys(agent, scope, iterables)?;
    for key in all_keys {
        agent.spend_execution_steps(1)?;
        let descriptor = get_own_property(agent, scope, iterables, key);
        let Some(descriptor) = close_all_if_abrupt(agent, scope, &iterators, descriptor)? else {
            continue;
        };
        if descriptor.enumerable != Some(true) {
            continue;
        }
        let value = get(agent, scope, iterables, key);
        let value = close_all_if_abrupt(agent, scope, &iterators, value)?;
        if agent.heap().get(value).is_undefined() {
            continue;
        }
        keys.push(key);
        let record = get_iterator_flattenable(agent, scope, realm, value, false);
        let record = close_all_if_abrupt(agent, scope, &iterators, record)?;
        iterators.push(record.0);
        records.push(record);
    }
    let count = records.len();
    let padding = if mode == LONGEST {
        let padding = agent.heap_mut().alloc_slots(scope, count)?;
        if !agent.heap().get(padding_option).is_undefined() {
            for (index, &key) in keys.iter().enumerate() {
                agent.spend_execution_steps(1)?;
                let value = get(agent, scope, padding_option, key);
                let value = close_all_if_abrupt(agent, scope, &iterators, value)?;
                write_local(agent, padding, index, value);
            }
        }
        Some(padding)
    } else {
        None
    };
    let list = agent.heap_mut().alloc_slots(scope, count)?;
    for (index, &key) in keys.iter().enumerate() {
        let value = agent.key_as_value(scope, key)?;
        write_local(agent, list, index, value);
    }
    zip_create(agent, scope, realm, &records, mode, padding, Some(list))
}
