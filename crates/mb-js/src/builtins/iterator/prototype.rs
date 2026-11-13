//! Die Methoden von %Iterator.prototype% (Abschnitt 27.1.3.3) außer
//! @@iterator: die Helfer, die einen neuen Iterator anlegen (`map`,
//! `filter`, `take`, `drop`, `flatMap`, `chunks`, `windows`), die Methoden,
//! die den Iterator sofort durchlaufen (`every`, `find`, `forEach`,
//! `includes`, `join`, `reduce`, `some`, `toArray`), @@dispose und die
//! Accessoren `constructor` und @@toStringTag.
//!
//! `includes` steht noch nicht im festgehaltenen Stand der Spezifikation,
//! aber im aktuellen (dort Abschnitt 27.1.3.3.9), und Test262 führt es als
//! Standard (`iterator-includes`). `join` steht in beiden Ständen, Test262
//! führt es aber noch als Vorschlag (`Iterator.prototype.join`), und der
//! Läufer überspringt seine Tests.

use super::helper::{
    CHUNKS, DROP, FILTER, FLAT_MAP, FUNCTION, ITERATOR, LIMIT, MAP, NEXT, OPTION, TAKE, WINDOWS,
    create_helper, read, write, write_local,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall};
use crate::builtins::array::{append, units_of};
use crate::compare::{same_value, same_value_zero};
use crate::conversion::{to_boolean, to_number, to_string};
use crate::heap::{Local, Scope};
use crate::iterator::{close_if_abrupt, get_iterator_direct, iterator_close, iterator_step_value};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::number::integer_or_infinity;
use crate::object::get_own_property;
use crate::operations::{
    call, create_array_from_list, create_data_property_or_throw, get_method, is_callable, set,
};
use crate::realm::Intrinsic;
use crate::value::{Kind, Value};

/// 2⁵³ − 1.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// 2³² − 1, die größte Größe für `chunks` und `windows`.
const MAX_SIZE: f64 = 4_294_967_295.0;

pub(super) static FUNCTIONS: [Builtin; 15] = [
    Builtin {
        name: "chunks",
        length: 1,
        constructor: false,
        behaviour: chunks,
    },
    Builtin {
        name: "drop",
        length: 1,
        constructor: false,
        behaviour: drop,
    },
    Builtin {
        name: "every",
        length: 1,
        constructor: false,
        behaviour: every,
    },
    Builtin {
        name: "filter",
        length: 1,
        constructor: false,
        behaviour: filter,
    },
    Builtin {
        name: "find",
        length: 1,
        constructor: false,
        behaviour: find,
    },
    Builtin {
        name: "flatMap",
        length: 1,
        constructor: false,
        behaviour: flat_map,
    },
    Builtin {
        name: "forEach",
        length: 1,
        constructor: false,
        behaviour: for_each,
    },
    Builtin {
        name: "includes",
        length: 1,
        constructor: false,
        behaviour: includes,
    },
    Builtin {
        name: "join",
        length: 1,
        constructor: false,
        behaviour: join,
    },
    Builtin {
        name: "map",
        length: 1,
        constructor: false,
        behaviour: map,
    },
    Builtin {
        name: "reduce",
        length: 1,
        constructor: false,
        behaviour: reduce,
    },
    Builtin {
        name: "some",
        length: 1,
        constructor: false,
        behaviour: some,
    },
    Builtin {
        name: "take",
        length: 1,
        constructor: false,
        behaviour: take,
    },
    Builtin {
        name: "toArray",
        length: 0,
        constructor: false,
        behaviour: to_array,
    },
    Builtin {
        name: "windows",
        length: 1,
        constructor: false,
        behaviour: windows,
    },
];

/// Iterator.prototype [ %Symbol.dispose% ] (Abschnitt 27.1.3.3.16).
pub(super) static DISPOSE: Builtin = Builtin {
    name: "[Symbol.dispose]",
    length: 0,
    constructor: false,
    behaviour: dispose,
};

/// get und set Iterator.prototype.constructor (Abschnitt 27.1.3.3.2).
pub(super) static CONSTRUCTOR_GETTER: Builtin = Builtin {
    name: "get constructor",
    length: 0,
    constructor: false,
    behaviour: constructor_getter,
};
pub(super) static CONSTRUCTOR_SETTER: Builtin = Builtin {
    name: "set constructor",
    length: 1,
    constructor: false,
    behaviour: constructor_setter,
};

/// get und set Iterator.prototype [ %Symbol.toStringTag% ] (Abschnitt
/// 27.1.3.3.18).
pub(super) static TAG_GETTER: Builtin = Builtin {
    name: "get [Symbol.toStringTag]",
    length: 0,
    constructor: false,
    behaviour: tag_getter,
};
pub(super) static TAG_SETTER: Builtin = Builtin {
    name: "set [Symbol.toStringTag]",
    length: 1,
    constructor: false,
    behaviour: tag_setter,
};

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

/// Schritt 2 aller Methoden: *this* muss ein Objekt sein.
fn this_object<'l>(agent: &mut Agent, arguments: &BuiltinCall<'_, 'l>) -> Completion<Local<'l>> {
    if !is_object(agent, arguments.this) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(arguments.this)
}

/// IteratorClose(iterated, ThrowCompletion(…)) mit einem neuen Fehler der
/// Art `kind`, für einen Iterator Record ohne [[NextMethod]].
fn close_with<T>(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    kind: ErrorKind,
) -> Completion<T> {
    let error = Err(agent.throw_error(kind));
    close_if_abrupt(agent, scope, object, error)
}

/// Die ersten Schritte von `map`, `filter`, `flatMap`, `every`, `find`,
/// `forEach`, `some` und `reduce`: *this* als Objekt, das erste Argument
/// muss aufrufbar sein, dann GetIteratorDirect.
fn callable_prelude<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<(Local<'s>, Local<'s>, Local<'s>)> {
    let object = this_object(agent, arguments)?;
    let object = agent.heap().local(scope, agent.heap().get(object));
    let function = arguments.argument(agent, scope, 0);
    if !is_callable(agent.heap(), agent.heap().get(function)) {
        return close_with(agent, scope, object, ErrorKind::TypeError);
    }
    let next = get_iterator_direct(agent, scope, object)?;
    Ok((object, next, function))
}

/// Ein Helfer der Art `operation` über *this* mit der Funktion aus dem
/// ersten Argument.
fn callable_helper<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    operation: i32,
) -> Completion<Local<'s>> {
    let (object, next, function) = callable_prelude(agent, scope, arguments)?;
    let (helper, state) = create_helper(agent, scope, arguments.realm, operation)?;
    write_local(agent, state, ITERATOR, object);
    write_local(agent, state, NEXT, next);
    write_local(agent, state, FUNCTION, function);
    Ok(helper)
}

/// Iterator.prototype.map ( mapper ) (Abschnitt 27.1.3.3.10).
fn map<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    callable_helper(agent, scope, arguments, MAP)
}

/// Iterator.prototype.filter ( predicate ) (Abschnitt 27.1.3.3.5).
fn filter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    callable_helper(agent, scope, arguments, FILTER)
}

/// Iterator.prototype.flatMap ( mapper ) (Abschnitt 27.1.3.3.7).
fn flat_map<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    callable_helper(agent, scope, arguments, FLAT_MAP)
}

/// `take` und `drop`: *limit* als ganze Zahl von 0 bis 2⁵³ − 1 oder +∞.
fn limit_helper<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    operation: i32,
) -> Completion<Local<'s>> {
    let object = this_object(agent, arguments)?;
    let object = agent.heap().local(scope, agent.heap().get(object));
    let limit = arguments.argument(agent, scope, 0);
    let number = to_number(agent, scope, limit);
    let number = close_if_abrupt(agent, scope, object, number)?;
    if number.is_nan() || (number.is_finite() && number > MAX_SAFE_INTEGER) {
        return close_with(agent, scope, object, ErrorKind::RangeError);
    }
    let limit = integer_or_infinity(number);
    if limit < 0.0 {
        return close_with(agent, scope, object, ErrorKind::RangeError);
    }
    let next = get_iterator_direct(agent, scope, object)?;
    let (helper, state) = create_helper(agent, scope, arguments.realm, operation)?;
    write_local(agent, state, ITERATOR, object);
    write_local(agent, state, NEXT, next);
    write(agent, state, LIMIT, Value::number(limit));
    Ok(helper)
}

/// Iterator.prototype.take ( limit ) (Abschnitt 27.1.3.3.13).
fn take<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    limit_helper(agent, scope, arguments, TAKE)
}

/// Iterator.prototype.drop ( limit ) (Abschnitt 27.1.3.3.3).
fn drop<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    limit_helper(agent, scope, arguments, DROP)
}

/// Prüft die Größe von `chunks` und `windows`: eine ganze Zahl (TypeError)
/// von 1 bis 2³² − 1 (RangeError).
fn size_argument(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    size: Local<'_>,
) -> Completion<f64> {
    let Some(size) = agent.heap().get(size).as_number() else {
        return close_with(agent, scope, object, ErrorKind::TypeError);
    };
    if !size.is_finite() || size.trunc() != size {
        return close_with(agent, scope, object, ErrorKind::TypeError);
    }
    if !(1.0..=MAX_SIZE).contains(&size) {
        return close_with(agent, scope, object, ErrorKind::RangeError);
    }
    Ok(size)
}

/// Iterator.prototype.chunks ( chunkSize ) (Abschnitt 27.1.3.3.1).
fn chunks<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, arguments)?;
    let object = agent.heap().local(scope, agent.heap().get(object));
    let size = arguments.argument(agent, scope, 0);
    let size = size_argument(agent, scope, object, size)?;
    let next = get_iterator_direct(agent, scope, object)?;
    let (helper, state) = create_helper(agent, scope, arguments.realm, CHUNKS)?;
    write_local(agent, state, ITERATOR, object);
    write_local(agent, state, NEXT, next);
    write(agent, state, LIMIT, Value::number(size));
    Ok(helper)
}

/// Ob `value` die Zeichenkette `text` ist.
fn is_string(agent: &Agent, value: Local<'_>, text: &str) -> bool {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::String(string) => heap
            .string_units(string)
            .into_iter()
            .eq(text.encode_utf16()),
        _ => false,
    }
}

/// Iterator.prototype.windows ( windowSize [ , undersized ] ) (Abschnitt
/// 27.1.3.3.15).
fn windows<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, arguments)?;
    let object = agent.heap().local(scope, agent.heap().get(object));
    let size = arguments.argument(agent, scope, 0);
    let size = size_argument(agent, scope, object, size)?;
    let undersized = arguments.argument(agent, scope, 1);
    let allow_partial = if agent.heap().get(undersized).is_undefined()
        || is_string(agent, undersized, "only-full")
    {
        false
    } else if is_string(agent, undersized, "allow-partial") {
        true
    } else {
        return close_with(agent, scope, object, ErrorKind::TypeError);
    };
    let next = get_iterator_direct(agent, scope, object)?;
    let (helper, state) = create_helper(agent, scope, arguments.realm, WINDOWS)?;
    write_local(agent, state, ITERATOR, object);
    write_local(agent, state, NEXT, next);
    write(agent, state, LIMIT, Value::number(size));
    write(agent, state, OPTION, Value::boolean(allow_partial));
    Ok(helper)
}

/// Durchläuft den Iterator und ruft `body` mit jedem Wert auf, jeden
/// Durchlauf mit einem Schritt und einem eigenen Bereich. `Some` aus
/// `body` beendet die Schleife; `None` zurück heißt, der Iterator ist
/// fertig.
fn each_value<'s>(
    agent: &mut Agent,
    object: Local<'_>,
    next: Local<'_>,
    mut body: impl FnMut(&mut Agent, &Scope, Local<'_>) -> Completion<Option<Local<'s>>>,
) -> Completion<Option<Local<'s>>> {
    loop {
        agent.spend_execution_steps(1)?;
        let outcome = agent.scope(|agent, inner| -> Completion<Option<Option<Local<'s>>>> {
            let Some(value) = iterator_step_value(agent, inner, object, next)? else {
                return Ok(Some(None));
            };
            Ok(body(agent, inner, value)?.map(Some))
        })?;
        if let Some(result) = outcome {
            return Ok(result);
        }
    }
}

/// Ruft `function` mit `value` und dem Zähler auf; eine Ausnahme schließt
/// den Iterator. Ab 2⁵³ bleibt der Zähler stehen, wie f64 rechnet.
fn call_counted<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    function: Local<'_>,
    leading: &[Local<'_>],
    counter: &mut f64,
) -> Completion<Local<'s>> {
    let index = agent.heap().local(scope, Value::number(*counter));
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let mut list: Vec<Local<'_>> = leading.to_vec();
    list.push(index);
    let result = call(agent, scope, function, undefined, &list);
    let result = close_if_abrupt(agent, scope, object, result)?;
    *counter += 1.0;
    Ok(result)
}

/// `every`, `some` und `find`: endet beim ersten Wert, für den
/// `predicate` `stop_on` ergibt, schließt dann den Iterator und gibt den
/// Wert (`find`) oder `stop_on` zurück.
fn search<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    stop_on: bool,
    return_value: bool,
) -> Completion<Option<Local<'s>>> {
    let (object, next, predicate) = callable_prelude(agent, scope, arguments)?;
    let mut counter = 0.0;
    each_value(agent, object, next, |agent, inner, value| {
        let result = call_counted(agent, inner, object, predicate, &[value], &mut counter)?;
        if to_boolean(agent.heap(), agent.heap().get(result)) != stop_on {
            return Ok(None);
        }
        iterator_close(agent, inner, object)?;
        let heap = agent.heap();
        let found = if return_value {
            heap.get(value)
        } else {
            Value::boolean(stop_on)
        };
        Ok(Some(heap.local(scope, found)))
    })
}

/// Iterator.prototype.every ( predicate ) (Abschnitt 27.1.3.3.4).
fn every<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    match search(agent, scope, arguments, false, false)? {
        Some(result) => Ok(result),
        None => Ok(agent.heap().local(scope, Value::boolean(true))),
    }
}

/// Iterator.prototype.some ( predicate ) (Abschnitt 27.1.3.3.12).
fn some<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    match search(agent, scope, arguments, true, false)? {
        Some(result) => Ok(result),
        None => Ok(agent.heap().local(scope, Value::boolean(false))),
    }
}

/// Iterator.prototype.find ( predicate ) (Abschnitt 27.1.3.3.6).
fn find<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    match search(agent, scope, arguments, true, true)? {
        Some(result) => Ok(result),
        None => Ok(agent.heap().local(scope, Value::UNDEFINED)),
    }
}

/// Iterator.prototype.forEach ( procedure ) (Abschnitt 27.1.3.3.8).
fn for_each<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (object, next, procedure) = callable_prelude(agent, scope, arguments)?;
    let mut counter = 0.0;
    each_value(agent, object, next, |agent, inner, value| {
        call_counted(agent, inner, object, procedure, &[value], &mut counter)?;
        Ok(None)
    })?;
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Iterator.prototype.reduce ( reducer [ , initialValue ] ) (Abschnitt
/// 27.1.3.3.11). Ob *initialValue* da ist, entscheidet die Zahl der
/// Argumente, nicht ihr Wert.
fn reduce<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (object, next, reducer) = callable_prelude(agent, scope, arguments)?;
    let (accumulator, mut counter) = if arguments.arguments.len() < 2 {
        let Some(first) = iterator_step_value(agent, scope, object, next)? else {
            return Err(agent.throw_error(ErrorKind::TypeError));
        };
        (first, 1.0)
    } else {
        (arguments.argument(agent, scope, 1), 0.0)
    };
    // Der Akkumulator liegt in einer Zelle, damit jeder Durchlauf ihn in
    // seinem eigenen Bereich ersetzen kann.
    let cell = agent.heap_mut().alloc_slots(scope, 1)?;
    write_local(agent, cell, 0, accumulator);
    each_value(agent, object, next, |agent, inner, value| {
        let accumulator = read(agent, inner, cell, 0);
        let result = call_counted(
            agent,
            inner,
            object,
            reducer,
            &[accumulator, value],
            &mut counter,
        )?;
        write_local(agent, cell, 0, result);
        Ok(None)
    })?;
    Ok(read(agent, scope, cell, 0))
}

/// Iterator.prototype.toArray ( ) (Abschnitt 27.1.3.3.14).
fn to_array<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, arguments)?;
    let next = get_iterator_direct(agent, scope, object)?;
    let mut items = Vec::new();
    each_value(agent, object, next, |agent, _, value| {
        let heap = agent.heap();
        items.push(heap.local(scope, heap.get(value)));
        Ok(None)
    })?;
    create_array_from_list(agent, scope, arguments.realm, &items)
}

/// Iterator.prototype.includes ( searchElement [ , skippedElements ] )
/// (Abschnitt 27.1.3.3.9 im aktuellen Stand der Spezifikation).
fn includes<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, arguments)?;
    let object = agent.heap().local(scope, agent.heap().get(object));
    let search_element = arguments.argument(agent, scope, 0);
    let skipped = arguments.argument(agent, scope, 1);
    let to_skip = if agent.heap().get(skipped).is_undefined() {
        0.0
    } else {
        // ±∞ oder eine ganze Zahl, sonst ein TypeError.
        let Some(number) = agent.heap().get(skipped).as_number() else {
            return close_with(agent, scope, object, ErrorKind::TypeError);
        };
        if number.is_nan() || (number.is_finite() && number.trunc() != number) {
            return close_with(agent, scope, object, ErrorKind::TypeError);
        }
        if number < 0.0 || (number.is_finite() && number > MAX_SAFE_INTEGER) {
            return close_with(agent, scope, object, ErrorKind::RangeError);
        }
        number
    };
    let next = get_iterator_direct(agent, scope, object)?;
    let mut skipped = 0.0;
    let found = each_value(agent, object, next, |agent, inner, value| {
        if skipped < to_skip {
            skipped += 1.0;
            return Ok(None);
        }
        let heap = agent.heap();
        if !same_value_zero(heap, heap.get(value), heap.get(search_element)) {
            return Ok(None);
        }
        iterator_close(agent, inner, object)?;
        Ok(Some(agent.heap().local(scope, Value::boolean(true))))
    })?;
    match found {
        Some(result) => Ok(result),
        None => Ok(agent.heap().local(scope, Value::boolean(false))),
    }
}

/// Iterator.prototype.join ( separator ) (Abschnitt 27.1.3.3.9).
fn join<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, arguments)?;
    let separator = arguments.argument(agent, scope, 0);
    let separator = if agent.heap().get(separator).is_undefined() {
        super::units(",")
    } else {
        let string = to_string(agent, scope, separator);
        let string = close_if_abrupt(agent, scope, object, string)?;
        units_of(agent, string)
    };
    let next = get_iterator_direct(agent, scope, object)?;
    let mut text = Vec::new();
    let mut first = true;
    each_value(agent, object, next, |agent, inner, value| {
        if first {
            first = false;
        } else {
            append(agent, &mut text, &separator)?;
        }
        let found = agent.heap().get(value);
        if found.is_undefined() || found.is_null() {
            return Ok(None);
        }
        let string = to_string(agent, inner, value);
        let string = close_if_abrupt(agent, inner, object, string)?;
        append(agent, &mut text, &units_of(agent, string))?;
        Ok(None)
    })?;
    Ok(agent.heap_mut().alloc_string(scope, &text)?)
}

/// Iterator.prototype [ %Symbol.dispose% ] ( ) (Abschnitt 27.1.3.3.16).
fn dispose<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    let key = agent.key_from_units(scope, &super::units("return"))?;
    if let Some(method) = get_method(agent, scope, object, key)? {
        call(agent, scope, method, object, &[])?;
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// SetterThatIgnoresPrototypeProperties (Abschnitt 7.3.37) mit
/// %Iterator.prototype% aus dem Realm des Setters als *home*.
fn setter_that_ignores_prototype_properties(
    agent: &mut Agent,
    scope: &Scope,
    arguments: &BuiltinCall<'_, '_>,
    key: PropertyKey<'_>,
) -> Completion<()> {
    let this = arguments.this;
    if !is_object(agent, this) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let home = agent.intrinsic(scope, arguments.realm, Intrinsic::IteratorPrototype);
    let heap = agent.heap();
    if same_value(heap, heap.get(this), heap.get(home)) {
        // So, als wäre die Eigenschaft von *home* in striktem Code nicht
        // beschreibbar.
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let value = arguments.argument(agent, scope, 0);
    if get_own_property(agent, scope, this, key)?.is_none() {
        create_data_property_or_throw(agent, scope, this, key, value)
    } else {
        set(agent, scope, this, key, value, true)
    }
}

/// get Iterator.prototype.constructor (Abschnitt 27.1.3.3.2.1).
fn constructor_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(agent.intrinsic(scope, arguments.realm, Intrinsic::Iterator))
}

/// set Iterator.prototype.constructor (Abschnitt 27.1.3.3.2.2).
fn constructor_setter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let key = agent.key_from_units(scope, &super::units("constructor"))?;
    setter_that_ignores_prototype_properties(agent, scope, arguments, key)?;
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// get Iterator.prototype [ %Symbol.toStringTag% ] (Abschnitt
/// 27.1.3.3.18.1).
fn tag_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    _arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(agent
        .heap_mut()
        .alloc_string(scope, &super::units("Iterator"))?)
}

/// set Iterator.prototype [ %Symbol.toStringTag% ] (Abschnitt
/// 27.1.3.3.18.2).
fn tag_setter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::ToStringTag));
    setter_that_ignores_prototype_properties(agent, scope, arguments, key)?;
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}
