//! Die Methoden von %RegExp.prototype% unter @@match, @@matchAll,
//! @@replace, @@search und @@split (Abschnitte 22.2.6.8, 22.2.6.9,
//! 22.2.6.11, 22.2.6.12 und 22.2.6.14) und %RegExpStringIteratorPrototype%
//! (Abschnitt 22.2.9).
//!
//! Sie rufen den Abgleich über RegExpExec auf, also über eine
//! überschriebene Methode `exec`, wenn es eine gibt. Jeder Durchlauf ihrer
//! Schleifen kostet einen Arbeitsschritt und hat einen eigenen Bereich für
//! seine Handles.
//!
//! Der Iterator ist ein Objekt der Art [`crate::heap::ObjectClass::Iterator`]:
//! [[IteratingRegExp]] im Feld des Ziels, *undefined* für [[Done]],
//! [[IteratedString]] im Feld des Index und [[Global]] und [[Unicode]] in
//! seiner Art.

use super::{
    advance_string_index, full_unicode, is_object, number, regexp_exec, undefined, units, units_of,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall};
use crate::builtins::array::{index_key, step};
use crate::builtins::iterator::{self, REGEXP_STRING};
use crate::builtins::string::{Match, count, get_substitution, length_of, push_units, substring};
use crate::compare::same_value;
use crate::conversion::{to_integer_or_infinity, to_length, to_object, to_string, to_uint32};
use crate::heap::{Local, Scope};
use crate::iterator::create_iterator_result_object;
use crate::key::PropertyKey;
use crate::object::array_create;
use crate::operations::{
    call, construct, create_data_property_or_throw, get, is_callable, length_of_array_like, set,
    species_constructor,
};
use crate::realm::Intrinsic;
use crate::value::Value;

pub(super) static MATCH: Builtin = method("[Symbol.match]", 1, symbol_match);
pub(super) static MATCH_ALL: Builtin = method("[Symbol.matchAll]", 1, symbol_match_all);
pub(super) static REPLACE: Builtin = method("[Symbol.replace]", 2, symbol_replace);
pub(super) static SEARCH: Builtin = method("[Symbol.search]", 1, symbol_search);
pub(super) static SPLIT: Builtin = method("[Symbol.split]", 2, symbol_split);

pub(super) static ITERATOR_FUNCTIONS: [Builtin; 1] = [method("next", 0, iterator_next)];

const fn method(name: &'static str, length: u32, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

/// *this* als Objekt, sonst ein TypeError, und ToString des ersten
/// Arguments: die ersten Schritte aller fünf Methoden.
fn this_and_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<(Local<'s>, Local<'s>)> {
    let regexp = arguments.this;
    if !is_object(agent, regexp) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let string = arguments.argument(agent, scope, 0);
    let string = to_string(agent, scope, string)?;
    let heap = agent.heap();
    Ok((heap.local(scope, heap.get(regexp)), string))
}

/// ToString(? Get(regexp, "flags")) als Codeeinheiten.
fn flags_of(agent: &mut Agent, scope: &Scope, regexp: Local<'_>) -> Completion<Vec<u16>> {
    let key = agent.key_from_units(scope, &units("flags"))?;
    let flags = get(agent, scope, regexp, key)?;
    let flags = to_string(agent, scope, flags)?;
    Ok(units_of(agent, flags))
}

fn get_named<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    name: &str,
) -> Completion<Local<'s>> {
    let key = agent.key_from_units(scope, &units(name))?;
    get(agent, scope, object, key)
}

fn set_last_index(
    agent: &mut Agent,
    scope: &Scope,
    regexp: Local<'_>,
    value: Local<'_>,
) -> Completion<()> {
    let key = agent.key_from_units(scope, &units("lastIndex"))?;
    set(agent, scope, regexp, key, value, true)
}

/// Nach einem leeren Treffer: `lastIndex` um ein Zeichen weiter.
fn advance_after_empty(
    agent: &mut Agent,
    scope: &Scope,
    regexp: Local<'_>,
    string: Local<'_>,
    unicode: bool,
) -> Completion<()> {
    let index = get_named(agent, scope, regexp, "lastIndex")?;
    let index = to_length(agent, scope, index)?;
    let next = advance_string_index(agent, string, index, unicode);
    let next = number(agent, scope, next);
    set_last_index(agent, scope, regexp, next)
}

/// ToString(? Get(result, "0")).
fn matched_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    result: Local<'_>,
) -> Completion<Local<'s>> {
    let matched = get(agent, scope, result, PropertyKey::Index(0))?;
    to_string(agent, scope, matched)
}

/// RegExp.prototype [ @@match ] ( string ) (Abschnitt 22.2.6.8).
fn symbol_match<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let (regexp, string) = this_and_string(agent, scope, arguments)?;
    let flags = flags_of(agent, scope, regexp)?;
    if !flags.contains(&u16::from(b'g')) {
        return regexp_exec(agent, scope, realm, regexp, string);
    }
    let unicode = full_unicode(&flags);
    let zero = number(agent, scope, 0.0);
    set_last_index(agent, scope, regexp, zero)?;
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    let array = array_create(agent, scope, 0, Some(prototype))?;
    let mut count = 0.0;
    loop {
        let found = agent.scope(|agent, inner| -> Completion<bool> {
            step(agent)?;
            let result = regexp_exec(agent, inner, realm, regexp, string)?;
            if agent.heap().get(result).is_null() {
                return Ok(false);
            }
            let matched = matched_string(agent, inner, result)?;
            let key = index_key(agent, inner, count)?;
            create_data_property_or_throw(agent, inner, array, key, matched)?;
            if length_of(agent, matched) == 0 {
                advance_after_empty(agent, inner, regexp, string, unicode)?;
            }
            Ok(true)
        })?;
        if !found {
            break;
        }
        count += 1.0;
    }
    if count == 0.0 {
        return Ok(agent.heap().local(scope, Value::NULL));
    }
    Ok(array)
}

/// RegExp.prototype [ @@matchAll ] ( string ) (Abschnitt 22.2.6.9) mit
/// CreateRegExpStringIterator (Abschnitt 22.2.9.1).
fn symbol_match_all<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let (regexp, string) = this_and_string(agent, scope, arguments)?;
    let default = agent.intrinsic(scope, realm, Intrinsic::RegExp);
    let constructor = species_constructor(agent, scope, regexp, default)?;
    let flags = flags_of(agent, scope, regexp)?;
    let flags_string = agent.heap_mut().alloc_string(scope, &flags)?;
    let matcher = construct(
        agent,
        scope,
        realm,
        constructor,
        &[regexp, flags_string],
        None,
    )?;
    let last_index = get_named(agent, scope, regexp, "lastIndex")?;
    let last_index = to_length(agent, scope, last_index)?;
    let last_index = number(agent, scope, last_index);
    set_last_index(agent, scope, matcher, last_index)?;
    let global = flags.contains(&u16::from(b'g'));
    let kind = REGEXP_STRING + i32::from(global) + 2 * i32::from(full_unicode(&flags));
    let prototype = agent.intrinsic(scope, realm, Intrinsic::RegExpStringIteratorPrototype);
    let created = iterator::iterator_create(agent, scope, prototype, matcher, kind)?;
    iterator::set_field(agent, created, iterator::INDEX, agent.heap().get(string));
    Ok(created)
}

/// %RegExpStringIteratorPrototype%.next ( ) (Abschnitt 22.2.9.2.1).
fn iterator_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    let realm = arguments.realm;
    let kind = iterator::kind_of(agent, scope, this)
        .filter(|kind| (REGEXP_STRING..REGEXP_STRING + 4).contains(kind));
    let (Some(kind), Some(regexp), Some(string)) = (
        kind,
        iterator::field(agent, scope, this, iterator::TARGET),
        iterator::field(agent, scope, this, iterator::INDEX),
    ) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let done = |agent: &mut Agent| -> Completion<Local<'s>> {
        iterator::set_field(agent, this, iterator::TARGET, Value::UNDEFINED);
        let undefined = undefined(agent, scope);
        create_iterator_result_object(agent, scope, realm, undefined, true)
    };
    if agent.heap().get(regexp).is_undefined() {
        return done(agent);
    }
    let offset = kind - REGEXP_STRING;
    let (global, unicode) = (offset & 1 != 0, offset & 2 != 0);
    let found = regexp_exec(agent, scope, realm, regexp, string)?;
    if agent.heap().get(found).is_null() {
        return done(agent);
    }
    if !global {
        iterator::set_field(agent, this, iterator::TARGET, Value::UNDEFINED);
        return create_iterator_result_object(agent, scope, realm, found, false);
    }
    let matched = matched_string(agent, scope, found)?;
    if length_of(agent, matched) == 0 {
        advance_after_empty(agent, scope, regexp, string, unicode)?;
    }
    create_iterator_result_object(agent, scope, realm, found, false)
}

/// RegExp.prototype [ @@replace ] ( string, replaceValue ) (Abschnitt
/// 22.2.6.11).
fn symbol_replace<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let (regexp, string) = this_and_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let mut replace_value = arguments.argument(agent, scope, 1);
    let functional = is_callable(agent.heap(), agent.heap().get(replace_value));
    if !functional {
        replace_value = to_string(agent, scope, replace_value)?;
    }
    let flags = flags_of(agent, scope, regexp)?;
    let global = flags.contains(&u16::from(b'g'));
    if global {
        let zero = number(agent, scope, 0.0);
        set_last_index(agent, scope, regexp, zero)?;
    }
    let mut results = Vec::new();
    loop {
        step(agent)?;
        let result = regexp_exec(agent, scope, realm, regexp, string)?;
        if agent.heap().get(result).is_null() {
            break;
        }
        results.push(result);
        if !global {
            break;
        }
        agent.scope(|agent, inner| -> Completion<()> {
            let matched = matched_string(agent, inner, result)?;
            if length_of(agent, matched) == 0 {
                advance_after_empty(agent, inner, regexp, string, full_unicode(&flags))?;
            }
            Ok(())
        })?;
    }
    let string_units = units_of(agent, string);
    let template = (!functional).then(|| units_of(agent, replace_value));
    let mut accumulated = Vec::new();
    let mut next_source_position = 0;
    for result in results {
        agent.scope(|agent, inner| -> Completion<()> {
            step(agent)?;
            let captures_count = length_of_array_like(agent, inner, result)?;
            let captures_count = (captures_count - 1.0).max(0.0);
            let matched = matched_string(agent, inner, result)?;
            let matched_units = units_of(agent, matched);
            let position = get_named(agent, inner, result, "index")?;
            let position = to_integer_or_infinity(agent, inner, position)?;
            let position = count(length).min(position.max(0.0));
            // Zwischen 0 und der Länge, also eine Stelle.
            let position = position as usize;
            let mut captures = Vec::new();
            let mut capture_values = Vec::new();
            let mut number_of_capture = 1.0;
            while number_of_capture <= captures_count {
                step(agent)?;
                let key = index_key(agent, inner, number_of_capture)?;
                let capture = get(agent, inner, result, key)?;
                if agent.heap().get(capture).is_undefined() {
                    captures.push(None);
                    capture_values.push(capture);
                } else {
                    let capture = to_string(agent, inner, capture)?;
                    captures.push(Some(units_of(agent, capture)));
                    capture_values.push(capture);
                }
                number_of_capture += 1.0;
            }
            let named_captures = get_named(agent, inner, result, "groups")?;
            let has_named = !agent.heap().get(named_captures).is_undefined();
            let mut replacement = Vec::new();
            match &template {
                None => {
                    let mut call_arguments = vec![matched];
                    call_arguments.extend(capture_values);
                    call_arguments.push(number(agent, inner, position as f64));
                    call_arguments.push(string);
                    if has_named {
                        call_arguments.push(named_captures);
                    }
                    let this = undefined(agent, inner);
                    let value = call(agent, inner, replace_value, this, &call_arguments)?;
                    let value = to_string(agent, inner, value)?;
                    replacement = units_of(agent, value);
                }
                Some(template) => {
                    let named = if has_named {
                        Some(to_object(agent, inner, realm, named_captures)?)
                    } else {
                        None
                    };
                    let found = Match {
                        matched: &matched_units,
                        string: &string_units,
                        position,
                        captures: &captures,
                        named_captures: named,
                    };
                    get_substitution(agent, &found, template, &mut replacement)?;
                }
            }
            if position >= next_source_position {
                let between = string_units
                    .get(next_source_position..position)
                    .unwrap_or_default();
                push_units(agent, &mut accumulated, between)?;
                push_units(agent, &mut accumulated, &replacement)?;
                next_source_position = position.saturating_add(matched_units.len());
            }
            Ok(())
        })?;
    }
    if next_source_position < length {
        let rest = string_units.get(next_source_position..).unwrap_or_default();
        push_units(agent, &mut accumulated, rest)?;
    }
    super::new_string(agent, scope, &accumulated)
}

/// RegExp.prototype [ @@search ] ( string ) (Abschnitt 22.2.6.12).
fn symbol_search<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let (regexp, string) = this_and_string(agent, scope, arguments)?;
    let previous = get_named(agent, scope, regexp, "lastIndex")?;
    let zero = number(agent, scope, 0.0);
    if !same_value(agent.heap(), agent.heap().get(previous), Value::number(0.0)) {
        set_last_index(agent, scope, regexp, zero)?;
    }
    let result = regexp_exec(agent, scope, realm, regexp, string)?;
    let current = get_named(agent, scope, regexp, "lastIndex")?;
    let heap = agent.heap();
    if !same_value(heap, heap.get(current), heap.get(previous)) {
        set_last_index(agent, scope, regexp, previous)?;
    }
    if agent.heap().get(result).is_null() {
        return Ok(number(agent, scope, -1.0));
    }
    get_named(agent, scope, result, "index")
}

/// RegExp.prototype [ @@split ] ( string, limit ) (Abschnitt 22.2.6.14).
fn symbol_split<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let (regexp, string) = this_and_string(agent, scope, arguments)?;
    let default = agent.intrinsic(scope, realm, Intrinsic::RegExp);
    let constructor = species_constructor(agent, scope, regexp, default)?;
    let flags = flags_of(agent, scope, regexp)?;
    let unicode = full_unicode(&flags);
    let mut new_flags = flags.clone();
    if !flags.contains(&u16::from(b'y')) {
        new_flags.push(u16::from(b'y'));
    }
    let new_flags = agent.heap_mut().alloc_string(scope, &new_flags)?;
    let splitter = construct(agent, scope, realm, constructor, &[regexp, new_flags], None)?;
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    let array = array_create(agent, scope, 0, Some(prototype))?;
    let limit = arguments.argument(agent, scope, 1);
    let limit = if agent.heap().get(limit).is_undefined() {
        u32::MAX
    } else {
        to_uint32(agent, scope, limit)?
    };
    if limit == 0 {
        return Ok(array);
    }
    let size = length_of(agent, string);
    if size == 0 {
        let result = regexp_exec(agent, scope, realm, splitter, string)?;
        if !agent.heap().get(result).is_null() {
            return Ok(array);
        }
        create_data_property_or_throw(agent, scope, array, PropertyKey::Index(0), string)?;
        return Ok(array);
    }
    let size = size as f64;
    let mut length: u32 = 0;
    let mut last_match_end = 0.0;
    let mut search_index = last_match_end;
    // Ob die Grenze erreicht ist.
    let push = |agent: &mut Agent, inner: &Scope, value: Local<'_>, length: &mut u32| {
        let key = index_key(agent, inner, f64::from(*length))?;
        create_data_property_or_throw(agent, inner, array, key, value)?;
        // Die Grenze ist höchstens 2³² − 1; spätestens dort endet es.
        *length = length.saturating_add(1);
        Completion::Ok(*length == limit)
    };
    while search_index < size {
        let full = agent.scope(|agent, inner| -> Completion<bool> {
            step(agent)?;
            let index = number(agent, inner, search_index);
            set_last_index(agent, inner, splitter, index)?;
            let result = regexp_exec(agent, inner, realm, splitter, string)?;
            if agent.heap().get(result).is_null() {
                search_index = advance_string_index(agent, string, search_index, unicode);
                return Ok(false);
            }
            let end = get_named(agent, inner, splitter, "lastIndex")?;
            let end = to_length(agent, inner, end)?.min(size);
            if end == last_match_end {
                search_index = advance_string_index(agent, string, search_index, unicode);
                return Ok(false);
            }
            // Beide Stellen liegen in der Zeichenkette.
            let part = substring(
                agent,
                inner,
                string,
                last_match_end as usize,
                search_index as usize,
            )?;
            if push(agent, inner, part, &mut length)? {
                return Ok(true);
            }
            last_match_end = end;
            let captures = length_of_array_like(agent, inner, result)?;
            let captures = (captures - 1.0).max(0.0);
            let mut index = 1.0;
            while index <= captures {
                step(agent)?;
                let key = index_key(agent, inner, index)?;
                let capture = get(agent, inner, result, key)?;
                if push(agent, inner, capture, &mut length)? {
                    return Ok(true);
                }
                index += 1.0;
            }
            search_index = last_match_end;
            Ok(false)
        })?;
        if full {
            return Ok(array);
        }
    }
    let rest = substring(agent, scope, string, last_match_end as usize, size as usize)?;
    let key = index_key(agent, scope, f64::from(length))?;
    create_data_property_or_throw(agent, scope, array, key, rest)?;
    Ok(array)
}
