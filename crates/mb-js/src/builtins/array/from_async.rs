//! Array.fromAsync ( asyncItems [ , mapper [ , thisArg ] ] ) (Abschnitt
//! 23.1.2.2).
//!
//! Die Spezifikation beschreibt die Methode als async-Closure, die an vier
//! Stellen wartet (Await). Eine eingebaute Funktion kann nicht warten;
//! darum hält eine interne Zelle den Zustand der Closure: die Stufe, an
//! der sie wartet, [[Resolve]] und [[Reject]] ihrer Capability, das
//! entstehende Array, den Index, den Iterator Record oder das
//! Array-ähnliche Objekt mit seiner Länge, `mapper` und `thisArg` und die
//! Ausnahme, mit der ein Iterator gerade geschlossen wird. Jedes Await ist
//! ein PerformPromiseThen mit zwei Closures, die den Zustand bei seiner
//! Stufe wieder aufnehmen und bis zum nächsten Await weiterlaufen. Was vor
//! dem ersten Await scheitert, lehnt das Promise ab, wie es
//! AsyncFunctionStart täte.

use super::{MAX_SAFE_INTEGER, array_create_in, index_key, set_length, step};
use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::async_function::perform_then_with;
use crate::builtin::{Builtin, BuiltinCall};
use crate::builtins::promise::reject_abrupt;
use crate::conversion::to_object;
use crate::function::is_constructor;
use crate::heap::{Local, Scope};
use crate::iterator::{
    create_async_from_sync_iterator, get_iterator_from_method, get_method_of_value,
    iterator_complete, iterator_value,
};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::operations::{
    call, construct, create_data_property_or_throw, get, get_method, is_callable,
    length_of_array_like,
};
use crate::promise::{new_promise_capability, promise_resolve};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Die Stufe, an der die Closure wartet, eine der Zahlen unten.
const STAGE: usize = 0;
/// [[Resolve]] und [[Reject]] der Capability.
const RESOLVE: usize = 1;
const REJECT: usize = 2;
/// Das entstehende Array *A*.
const ARRAY: usize = 3;
/// Der Index *k*.
const INDEX: usize = 4;
/// [[Iterator]] und [[NextMethod]] des Iterator Record, *undefined* für
/// ein Array-ähnliches Objekt.
const ITERATOR: usize = 5;
const NEXT: usize = 6;
/// `mapper` oder *undefined*, und `thisArg`.
const MAPPER: usize = 7;
const THIS_ARG: usize = 8;
/// Das Array-ähnliche Objekt und seine Länge.
const SOURCE: usize = 9;
const LENGTH: usize = 10;
/// Die Ausnahme, mit der der Iterator gerade geschlossen wird.
const ERROR: usize = 11;
const FIELDS: usize = 12;

/// Await auf das Ergebnis von `next`.
const NEXT_RESULT: i32 = 1;
/// Await auf das Ergebnis von `mapper` für einen Wert des Iterators.
const MAPPED: i32 = 2;
/// Await auf das Ergebnis von `return` beim Schließen.
const CLOSING: i32 = 3;
/// Await auf ein Element des Array-ähnlichen Objekts.
const ELEMENT: i32 = 4;
/// Await auf das Ergebnis von `mapper` für so ein Element.
const ELEMENT_MAPPED: i32 = 5;

/// Die Closures von Await: ohne Namen und ohne [[Construct]].
static ON_FULFILLED: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: on_fulfilled,
};
static ON_REJECTED: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: on_rejected,
};

pub(super) fn array_from_async<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let capability = new_promise_capability(agent, scope, realm, constructor)?;
    let state = agent.heap_mut().alloc_slots(scope, FIELDS)?;
    let mapper = arguments.argument(agent, scope, 1);
    let this_arg = arguments.argument(agent, scope, 2);
    let heap = agent.heap();
    write(agent, state, RESOLVE, heap.get(capability.resolve));
    write(agent, state, REJECT, heap.get(capability.reject));
    write(agent, state, MAPPER, heap.get(mapper));
    write(agent, state, THIS_ARG, heap.get(this_arg));
    write(agent, state, INDEX, Value::number(0.0));
    let items = arguments.argument(agent, scope, 0);
    if let Err(abrupt) = start(agent, scope, realm, state, arguments.this, items) {
        reject_abrupt(agent, scope, realm, abrupt, capability.reject)?;
    }
    Ok(capability.promise)
}

/// Die Closure bis zu ihrem ersten Await: Schritte 3.a bis 3.g.ii oder
/// 3.h.i bis 3.h.v.
fn start(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    constructor: Local<'_>,
    items: Local<'_>,
) -> Completion<()> {
    let mapper = field(agent, scope, state, MAPPER);
    let found = agent.heap().get(mapper);
    if !found.is_undefined() && !is_callable(agent.heap(), found) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::AsyncIterator);
    let using_async = get_method_of_value(agent, scope, realm, items, PropertyKey::Symbol(symbol))?;
    let record = if let Some(method) = using_async {
        Some(get_iterator_from_method(agent, scope, items, method)?)
    } else {
        let symbol = agent.well_known_symbol(scope, WellKnownSymbol::Iterator);
        let using_sync =
            get_method_of_value(agent, scope, realm, items, PropertyKey::Symbol(symbol))?;
        match using_sync {
            Some(method) => {
                let (iterator, next) = get_iterator_from_method(agent, scope, items, method)?;
                Some(create_async_from_sync_iterator(
                    agent, scope, realm, iterator, next,
                )?)
            }
            None => None,
        }
    };
    if let Some((iterator, next)) = record {
        let array = if is_constructor(agent, agent.heap().get(constructor)) {
            construct(agent, scope, realm, constructor, &[], None)?
        } else {
            array_create_in(agent, scope, realm, 0.0)?
        };
        let heap = agent.heap();
        write(agent, state, ARRAY, heap.get(array));
        write(agent, state, ITERATOR, heap.get(iterator));
        write(agent, state, NEXT, heap.get(next));
        return request_next(agent, scope, realm, state);
    }
    let source = to_object(agent, scope, realm, items)?;
    let length = length_of_array_like(agent, scope, source)?;
    let array = if is_constructor(agent, agent.heap().get(constructor)) {
        let length = agent.heap().local(scope, Value::number(length));
        construct(agent, scope, realm, constructor, &[length], None)?
    } else {
        array_create_in(agent, scope, realm, length)?
    };
    let heap = agent.heap();
    write(agent, state, ARRAY, heap.get(array));
    write(agent, state, SOURCE, heap.get(source));
    write(agent, state, LENGTH, Value::number(length));
    continue_elements(agent, scope, realm, state)
}

/// Schritte 3.g.iii.1 bis 4 der Schleife: Call von `next`, dann Await.
fn request_next(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
) -> Completion<()> {
    step(agent)?;
    if index(agent, state) >= MAX_SAFE_INTEGER {
        let error = agent.throw_error(ErrorKind::TypeError);
        return close_with(agent, scope, realm, state, error);
    }
    let iterator = field(agent, scope, state, ITERATOR);
    let next = field(agent, scope, state, NEXT);
    let result = call(agent, scope, next, iterator, &[])?;
    await_value(agent, scope, realm, state, NEXT_RESULT, result)
}

/// Schritte 3.g.iii.5 bis 9 mit dem Ergebnis von `next`.
fn next_result(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    result: Local<'_>,
) -> Completion<()> {
    if !matches!(agent.heap().get(result).kind(), Kind::Object(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if iterator_complete(agent, scope, result)? {
        let array = field(agent, scope, state, ARRAY);
        set_length(agent, scope, array, index(agent, state))?;
        return resolve_array(agent, scope, state);
    }
    let value = iterator_value(agent, scope, result)?;
    let mapper = field(agent, scope, state, MAPPER);
    if agent.heap().get(mapper).is_undefined() {
        return define_and_continue(agent, scope, realm, state, value);
    }
    let this_arg = field(agent, scope, state, THIS_ARG);
    let k = agent
        .heap()
        .local(scope, Value::number(index(agent, state)));
    match call(agent, scope, mapper, this_arg, &[value, k]) {
        Ok(mapped) => await_value(agent, scope, realm, state, MAPPED, mapped),
        Err(abrupt) => close_with(agent, scope, realm, state, abrupt),
    }
}

/// Schritte 3.g.iii.10 bis 12: das Element ins Array, dann der nächste.
fn define_and_continue(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    value: Local<'_>,
) -> Completion<()> {
    let k = index(agent, state);
    let array = field(agent, scope, state, ARRAY);
    let key = index_key(agent, scope, k)?;
    if let Err(abrupt) = create_data_property_or_throw(agent, scope, array, key, value) {
        return close_with(agent, scope, realm, state, abrupt);
    }
    write(agent, state, INDEX, Value::number(k + 1.0));
    request_next(agent, scope, realm, state)
}

/// Schritte 3.h.v bis vii: das nächste Element des Array-ähnlichen Objekts
/// lesen und darauf warten, oder am Ende das Array liefern.
fn continue_elements(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
) -> Completion<()> {
    step(agent)?;
    let k = index(agent, state);
    let length = field(agent, scope, state, LENGTH);
    let length = agent.heap().get(length).as_number().unwrap_or(0.0);
    let array = field(agent, scope, state, ARRAY);
    if k >= length {
        set_length(agent, scope, array, length)?;
        return resolve_array(agent, scope, state);
    }
    let source = field(agent, scope, state, SOURCE);
    let key = index_key(agent, scope, k)?;
    let value = get(agent, scope, source, key)?;
    await_value(agent, scope, realm, state, ELEMENT, value)
}

/// Schritt 3.h.v.4 mit dem Element: `mapper`, wenn es einen gibt.
fn element_value(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    value: Local<'_>,
) -> Completion<()> {
    let mapper = field(agent, scope, state, MAPPER);
    if agent.heap().get(mapper).is_undefined() {
        return element_mapped(agent, scope, realm, state, value);
    }
    let this_arg = field(agent, scope, state, THIS_ARG);
    let k = agent
        .heap()
        .local(scope, Value::number(index(agent, state)));
    let mapped = call(agent, scope, mapper, this_arg, &[value, k])?;
    await_value(agent, scope, realm, state, ELEMENT_MAPPED, mapped)
}

/// Schritte 3.h.v.5 und 6: das Element ins Array, dann das nächste.
fn element_mapped(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    value: Local<'_>,
) -> Completion<()> {
    let k = index(agent, state);
    let array = field(agent, scope, state, ARRAY);
    let key = index_key(agent, scope, k)?;
    create_data_property_or_throw(agent, scope, array, key, value)?;
    write(agent, state, INDEX, Value::number(k + 1.0));
    continue_elements(agent, scope, realm, state)
}

/// Await (Abschnitt 27.7.5.3) auf `value`; danach geht es bei `stage`
/// weiter.
fn await_value(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    stage: i32,
    value: Local<'_>,
) -> Completion<()> {
    write(agent, state, STAGE, Value::int32(stage));
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let promise = promise_resolve(agent, scope, realm, constructor, value)?;
    perform_then_with(
        agent,
        scope,
        realm,
        promise,
        state,
        &ON_FULFILLED,
        &ON_REJECTED,
    )
}

/// AsyncIteratorClose (Abschnitt 7.4.15) mit der Ausnahme `abrupt` als
/// Completion: Sie bleibt das Ergebnis, was `return` auch tut.
fn close_with(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    abrupt: Abrupt,
) -> Completion<()> {
    if abrupt != Abrupt::Throw {
        return Err(abrupt);
    }
    let reason = agent.take_thrown_value(scope, realm)?;
    close_with_reason(agent, scope, realm, state, reason)
}

/// Dasselbe mit dem Wert der Ausnahme.
fn close_with_reason(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    reason: Local<'_>,
) -> Completion<()> {
    write(agent, state, ERROR, agent.heap().get(reason));
    let iterator = field(agent, scope, state, ITERATOR);
    let key = agent.key_from_units(scope, &crate::builtins::units("return"))?;
    let method = match get_method(agent, scope, iterator, key) {
        Ok(Some(method)) => method,
        Ok(None) => return reject_stored(agent, scope, state),
        Err(Abrupt::Throw) => {
            agent.take_exception(scope);
            return reject_stored(agent, scope, state);
        }
        Err(abrupt) => return Err(abrupt),
    };
    match call(agent, scope, method, iterator, &[]) {
        Ok(result) => await_value(agent, scope, realm, state, CLOSING, result),
        Err(Abrupt::Throw) => {
            agent.take_exception(scope);
            reject_stored(agent, scope, state)
        }
        Err(abrupt) => Err(abrupt),
    }
}

/// Call von [[Reject]] mit der gemerkten Ausnahme.
fn reject_stored(agent: &mut Agent, scope: &Scope, state: Local<'_>) -> Completion<()> {
    let reason = field(agent, scope, state, ERROR);
    reject_with(agent, scope, state, reason)
}

/// Call von [[Reject]] mit `reason`.
fn reject_with(
    agent: &mut Agent,
    scope: &Scope,
    state: Local<'_>,
    reason: Local<'_>,
) -> Completion<()> {
    let reject = field(agent, scope, state, REJECT);
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    call(agent, scope, reject, undefined, &[reason])?;
    Ok(())
}

/// Call von [[Resolve]] mit dem Array.
fn resolve_array(agent: &mut Agent, scope: &Scope, state: Local<'_>) -> Completion<()> {
    let resolve = field(agent, scope, state, RESOLVE);
    let array = field(agent, scope, state, ARRAY);
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    call(agent, scope, resolve, undefined, &[array])?;
    Ok(())
}

fn on_fulfilled<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume(agent, scope, arguments, false)
}

fn on_rejected<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume(agent, scope, arguments, true)
}

/// Nimmt die Closure nach einem Await wieder auf: mit dem Wert, auf den
/// sie gewartet hat, oder mit dem Grund, aus dem das Warten scheiterte.
/// Eine Ausnahme der Fortsetzung lehnt das Promise ab.
fn resume<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    rejected: bool,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let state = arguments.captures(agent, scope);
    let value = arguments.argument(agent, scope, 0);
    let stage = field(agent, scope, state, STAGE);
    let stage = agent.heap().get(stage).as_int32().unwrap_or(0);
    let result = match (stage, rejected) {
        // Was `return` liefert, ändert an der Ausnahme nichts.
        (CLOSING, _) => reject_stored(agent, scope, state),
        // Scheitert das Warten auf `next` oder ein Element, wirft die
        // Closure ohne Schließen.
        (NEXT_RESULT | ELEMENT | ELEMENT_MAPPED, true) => reject_with(agent, scope, state, value),
        (NEXT_RESULT, false) => next_result(agent, scope, realm, state, value),
        // IfAbruptCloseAsyncIterator für das Ergebnis von `mapper`.
        (MAPPED, true) => close_with_reason(agent, scope, realm, state, value),
        (MAPPED, false) => define_and_continue(agent, scope, realm, state, value),
        (ELEMENT, false) => element_value(agent, scope, realm, state, value),
        (ELEMENT_MAPPED, false) => element_mapped(agent, scope, realm, state, value),
        _ => Ok(()),
    };
    if let Err(abrupt) = result {
        let reject = field(agent, scope, state, REJECT);
        reject_abrupt(agent, scope, realm, abrupt, reject)?;
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Der Platz `index` des Zustands als Handle.
fn field<'s>(agent: &Agent, scope: &'s Scope, state: Local<'_>, index: usize) -> Local<'s> {
    let heap = agent.heap();
    let value = match heap.get(state).kind() {
        Kind::Internal(cell) => heap.slot(cell, index).unwrap_or(Value::UNDEFINED),
        _ => Value::UNDEFINED,
    };
    heap.local(scope, value)
}

fn write(agent: &Agent, state: Local<'_>, index: usize, value: Value<'_>) {
    let heap = agent.heap();
    if let Kind::Internal(cell) = heap.get(state).kind() {
        heap.set_slot(cell, index, value);
    }
}

/// Der Index *k*.
fn index(agent: &Agent, state: Local<'_>) -> f64 {
    let heap = agent.heap();
    match heap.get(state).kind() {
        Kind::Internal(cell) => heap
            .slot(cell, INDEX)
            .and_then(|value| value.as_number())
            .unwrap_or(0.0),
        _ => 0.0,
    }
}
