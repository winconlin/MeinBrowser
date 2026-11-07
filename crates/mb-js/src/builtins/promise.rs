//! %Promise% (Abschnitt 27.5.3) mit den Eigenschaften des Konstruktors
//! (Abschnitt 27.5.4) und %Promise.prototype% (Abschnitt 27.5.5). Die
//! abstrakten Operationen und die Jobs stehen in `crate::promise`.
//!
//! Die Funktionen, die Promise.all, Promise.allSettled und Promise.any für
//! jedes Element anlegen, fangen eine interne Zelle ein: den Zustand des
//! Aufrufs, ihren Index und [[AlreadyCalled]]. Der Zustand ist eine
//! weitere Zelle mit der Liste der Werte oder Fehler, der Zahl
//! *remainingElementsCount* und der Capability des Ergebnisses. Bei
//! Promise.allSettled teilen sich die beiden Funktionen eines Elements ihre
//! Zelle und damit [[AlreadyCalled]], so wie den Record *alreadyCalled* der
//! Spezifikation.

use super::error::aggregate_error_create;
use super::{define_functions, define_to_string_tag, link_constructor, units};
use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_closure, create_builtin_function};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::iterator::{get_iterator, iterator_close_on_throw, iterator_step_value};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{get_prototype_from_constructor, ordinary_object_create};
use crate::operations::{
    call, create_array_from_list, create_data_property_or_throw, define_property_or_throw, get,
    invoke, is_callable, species_constructor,
};
use crate::promise::{
    Capability, create_resolving_functions, is_promise, list_len, list_push, list_reserve,
    list_set, list_value, new_promise_capability, perform_promise_then, promise_create,
    promise_resolve,
};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static PROMISE: Builtin = Builtin {
    name: "Promise",
    length: 1,
    constructor: true,
    behaviour: promise,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 8] = [
    Builtin {
        name: "all",
        length: 1,
        constructor: false,
        behaviour: all,
    },
    Builtin {
        name: "allSettled",
        length: 1,
        constructor: false,
        behaviour: all_settled,
    },
    Builtin {
        name: "any",
        length: 1,
        constructor: false,
        behaviour: any,
    },
    Builtin {
        name: "race",
        length: 1,
        constructor: false,
        behaviour: race,
    },
    Builtin {
        name: "reject",
        length: 1,
        constructor: false,
        behaviour: reject,
    },
    Builtin {
        name: "resolve",
        length: 1,
        constructor: false,
        behaviour: resolve,
    },
    Builtin {
        name: "try",
        length: 1,
        constructor: false,
        behaviour: promise_try,
    },
    Builtin {
        name: "withResolvers",
        length: 0,
        constructor: false,
        behaviour: with_resolvers,
    },
];

static SPECIES: Builtin = Builtin {
    name: "get [Symbol.species]",
    length: 0,
    constructor: false,
    behaviour: species,
};

static PROTOTYPE_FUNCTIONS: [Builtin; 3] = [
    Builtin {
        name: "catch",
        length: 1,
        constructor: false,
        behaviour: catch,
    },
    Builtin {
        name: "finally",
        length: 1,
        constructor: false,
        behaviour: finally,
    },
    Builtin {
        name: "then",
        length: 2,
        constructor: false,
        behaviour: then,
    },
];

/// Die Closures aus Promise.prototype.finally (Abschnitt 27.5.5.3).
static THEN_FINALLY: Builtin = closure(1, then_finally);
static CATCH_FINALLY: Builtin = closure(1, catch_finally);
static VALUE_THUNK: Builtin = closure(0, value_thunk);
static THROWER: Builtin = closure(0, thrower);

/// Die Funktionen für die Elemente.
static ALL_FULFILLED: Builtin = closure(1, all_fulfilled);
static ALL_SETTLED_FULFILLED: Builtin = closure(1, all_settled_fulfilled);
static ALL_SETTLED_REJECTED: Builtin = closure(1, all_settled_rejected);
static ANY_REJECTED: Builtin = closure(1, any_rejected);

/// Der Eintrag einer Funktion aus einem Abstract Closure: ohne Namen und
/// ohne [[Construct]].
const fn closure(length: u32, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name: "",
        length,
        constructor: false,
        behaviour,
    }
}

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(object))?;
    let constructor = create_builtin_function(agent, scope, realm, &PROMISE)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    // get Promise [ %Symbol.species% ] (Abschnitt 27.5.4.10): ein Accessor
    // ohne Setter, konfigurierbar und nicht aufzählbar.
    let getter = create_builtin_function(agent, scope, realm, &SPECIES)?;
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::Species));
    let descriptor = PropertyDescriptor::accessor(getter, undefined, false, true);
    define_property_or_throw(agent, scope, constructor, key, &descriptor)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    define_to_string_tag(agent, scope, prototype, "Promise")?;
    agent.set_intrinsic(realm, Intrinsic::Promise, constructor);
    agent.set_intrinsic(realm, Intrinsic::PromisePrototype, prototype);
    Ok(())
}

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

fn property_key<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    name: &str,
) -> Completion<PropertyKey<'s>> {
    Ok(agent.key_from_units(scope, &units(name))?)
}

/// Ruft `reject` mit dem geworfenen Wert, wenn `abrupt` eine Ausnahme ist
/// (IfAbruptRejectPromise, Abschnitt 27.5.1.1.1); andere abrupte
/// Beendigungen gehen weiter. Ein Fehler der Engine ohne Realm bekommt
/// `realm`, den aktuellen.
fn reject_abrupt(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    abrupt: Abrupt,
    reject: Local<'_>,
) -> Completion<()> {
    if abrupt != Abrupt::Throw {
        return Err(abrupt);
    }
    let reason = agent.take_thrown_value(scope, realm)?;
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    call(agent, scope, reject, undefined, &[reason])?;
    Ok(())
}

/// IfAbruptRejectPromise mit dem Ergebnis [[Promise]] von `capability`.
pub(super) fn if_abrupt_reject_promise<'s>(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    abrupt: Abrupt,
    capability: &Capability<'s>,
) -> Completion<Local<'s>> {
    reject_abrupt(agent, scope, realm, abrupt, capability.reject)?;
    Ok(capability.promise)
}

/// Promise ( executor ) (Abschnitt 27.5.3.1).
fn promise<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let executor = arguments.argument(agent, scope, 0);
    if !is_callable(agent.heap(), agent.heap().get(executor)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let realm = arguments.realm;
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::PromisePrototype,
        realm,
    )?;
    let promise = promise_create(agent, scope, prototype)?;
    let (resolve, reject) = create_resolving_functions(agent, scope, realm, promise)?;
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    if let Err(abrupt) = call(agent, scope, executor, undefined, &[resolve, reject]) {
        reject_abrupt(agent, scope, realm, abrupt, reject)?;
    }
    Ok(promise)
}

// Die Eigenschaften des Konstruktors

/// Promise.all ( iterable ) (Abschnitt 27.5.4.1).
fn all<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    combine(agent, scope, arguments, Combinator::All)
}

/// Promise.allSettled ( iterable ) (Abschnitt 27.5.4.2).
fn all_settled<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    combine(agent, scope, arguments, Combinator::AllSettled)
}

/// Promise.any ( iterable ) (Abschnitt 27.5.4.3).
fn any<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    combine(agent, scope, arguments, Combinator::Any)
}

/// Promise.race ( iterable ) (Abschnitt 27.5.4.5).
fn race<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    combine(agent, scope, arguments, Combinator::Race)
}

/// Promise.reject ( r ) (Abschnitt 27.5.4.6).
fn reject<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let capability = new_promise_capability(agent, scope, arguments.realm, arguments.this)?;
    let reason = arguments.argument(agent, scope, 0);
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    call(agent, scope, capability.reject, undefined, &[reason])?;
    Ok(capability.promise)
}

/// Promise.resolve ( x ) (Abschnitt 27.5.4.7).
fn resolve<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if !is_object(agent, arguments.this) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let resolution = arguments.argument(agent, scope, 0);
    promise_resolve(agent, scope, arguments.realm, arguments.this, resolution)
}

/// Promise.try ( callback, ...args ) (Abschnitt 27.5.4.8).
fn promise_try<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let constructor = arguments.this;
    if !is_object(agent, constructor) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let callback = arguments.argument(agent, scope, 0);
    let rest = arguments.arguments.get(1..).unwrap_or_default();
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    match call(agent, scope, callback, undefined, rest) {
        Ok(status) => promise_resolve(agent, scope, arguments.realm, constructor, status),
        Err(Abrupt::Throw) => {
            let reason = agent.take_thrown_value(scope, arguments.realm)?;
            let capability = new_promise_capability(agent, scope, arguments.realm, constructor)?;
            call(agent, scope, capability.reject, undefined, &[reason])?;
            Ok(capability.promise)
        }
        Err(abrupt) => Err(abrupt),
    }
}

/// Promise.withResolvers ( ) (Abschnitt 27.5.4.9).
fn with_resolvers<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let capability = new_promise_capability(agent, scope, arguments.realm, arguments.this)?;
    let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::ObjectPrototype);
    let object = ordinary_object_create(agent, scope, Some(prototype))?;
    for (name, value) in [
        ("promise", capability.promise),
        ("resolve", capability.resolve),
        ("reject", capability.reject),
    ] {
        let key = property_key(agent, scope, name)?;
        create_data_property_or_throw(agent, scope, object, key, value)?;
    }
    Ok(object)
}

/// get Promise [ %Symbol.species% ] (Abschnitt 27.5.4.10).
fn species<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    Ok(heap.local(scope, heap.get(arguments.this)))
}

// Promise.all und seine Geschwister

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Combinator {
    All,
    AllSettled,
    Any,
    Race,
}

/// Die Plätze des Zustands: die Liste der Werte oder Fehler,
/// *remainingElementsCount* und die drei Teile der Capability.
const VALUES: usize = 0;
const REMAINING: usize = 1;
const RESULT_PROMISE: usize = 2;
const RESULT_RESOLVE: usize = 3;
const RESULT_REJECT: usize = 4;
const STATE_SLOTS: usize = 5;

/// Die Plätze der Zelle einer Funktion für ein Element: der Zustand, der
/// Index und [[AlreadyCalled]].
const ELEMENT_STATE: usize = 0;
const ELEMENT_INDEX: usize = 1;
const ELEMENT_CALLED: usize = 2;
const ELEMENT_SLOTS: usize = 3;

/// Die gemeinsamen Schritte von Promise.all, Promise.allSettled,
/// Promise.any und Promise.race bis auf Perform… (Abschnitte 27.5.4.1 bis
/// 27.5.4.3 und 27.5.4.5).
fn combine<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    combinator: Combinator,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let constructor = arguments.this;
    let capability = new_promise_capability(agent, scope, realm, constructor)?;
    let resolve = match get_promise_resolve(agent, scope, constructor) {
        Ok(resolve) => resolve,
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    let iterable = arguments.argument(agent, scope, 0);
    let (iterator, next) = match get_iterator(agent, scope, realm, iterable) {
        Ok(record) => record,
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    let mut record = IteratorRecord {
        iterator,
        next,
        done: false,
    };
    let result = match combinator {
        Combinator::Race => {
            perform_race(agent, realm, &mut record, constructor, &capability, resolve)
        }
        _ => perform_collect(
            agent,
            scope,
            realm,
            combinator,
            &mut record,
            constructor,
            &capability,
            resolve,
        ),
    };
    match result {
        Ok(promise) => Ok(promise),
        Err(Abrupt::Throw) => {
            // Der geworfene Wert gilt auch, wenn IteratorClose scheitert.
            let reason = agent.take_thrown_value(scope, realm)?;
            if !record.done {
                iterator_close_on_throw(agent, scope, iterator)?;
            }
            let undefined = agent.heap().local(scope, Value::UNDEFINED);
            call(agent, scope, capability.reject, undefined, &[reason])?;
            Ok(capability.promise)
        }
        Err(abrupt) => Err(abrupt),
    }
}

/// Ein Iterator Record (Abschnitt 7.4.1).
struct IteratorRecord<'s> {
    iterator: Local<'s>,
    next: Local<'s>,
    done: bool,
}

impl IteratorRecord<'_> {
    /// IteratorStepValue: der nächste Wert, `None` am Ende. Danach und nach
    /// jedem Fehler gilt [[Done]].
    fn step<'s>(&mut self, agent: &mut Agent, scope: &'s Scope) -> Completion<Option<Local<'s>>> {
        let result = iterator_step_value(agent, scope, self.iterator, self.next);
        if !matches!(result, Ok(Some(_))) {
            self.done = true;
        }
        result
    }
}

/// GetPromiseResolve (Abschnitt 27.5.4.1.1).
fn get_promise_resolve<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    constructor: Local<'_>,
) -> Completion<Local<'s>> {
    let key = property_key(agent, scope, "resolve")?;
    let resolve = get(agent, scope, constructor, key)?;
    if !is_callable(agent.heap(), agent.heap().get(resolve)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(resolve)
}

/// PerformPromiseRace (Abschnitt 27.5.4.5.1).
fn perform_race<'s>(
    agent: &mut Agent,
    realm: RealmId,
    record: &mut IteratorRecord<'_>,
    constructor: Local<'_>,
    capability: &Capability<'s>,
    resolve: Local<'_>,
) -> Completion<Local<'s>> {
    loop {
        let finished = agent.scope(|agent, inner| -> Completion<bool> {
            let Some(value) = record.step(agent, inner)? else {
                return Ok(true);
            };
            let next = call(agent, inner, resolve, constructor, &[value])?;
            let key = property_key(agent, inner, "then")?;
            invoke(
                agent,
                inner,
                realm,
                next,
                key,
                &[capability.resolve, capability.reject],
            )?;
            Ok(false)
        })?;
        if finished {
            return Ok(capability.promise);
        }
    }
}

/// PerformPromiseAll, PerformPromiseAllSettled und PerformPromiseAny
/// (Abschnitte 27.5.4.1.2, 27.5.4.2.1 und 27.5.4.3.1).
#[allow(clippy::too_many_arguments)]
fn perform_collect<'s>(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    combinator: Combinator,
    record: &mut IteratorRecord<'_>,
    constructor: Local<'_>,
    capability: &Capability<'s>,
    resolve: Local<'_>,
) -> Completion<Local<'s>> {
    let state = agent.heap_mut().alloc_slots(scope, STATE_SLOTS)?;
    {
        let heap = agent.heap();
        if let Kind::Internal(cell) = heap.get(state).kind() {
            heap.set_slot(cell, REMAINING, Value::int32(1));
            heap.set_slot(cell, RESULT_PROMISE, heap.get(capability.promise));
            heap.set_slot(cell, RESULT_RESOLVE, heap.get(capability.resolve));
            heap.set_slot(cell, RESULT_REJECT, heap.get(capability.reject));
        }
    }
    loop {
        let finished = agent.scope(|agent, inner| -> Completion<bool> {
            let Some(value) = record.step(agent, inner)? else {
                return Ok(true);
            };
            let index = append_undefined(agent, inner, state)?;
            let next = call(agent, inner, resolve, constructor, &[value])?;
            let element = agent.heap_mut().alloc_slots(inner, ELEMENT_SLOTS)?;
            {
                let heap = agent.heap();
                if let Kind::Internal(cell) = heap.get(element).kind() {
                    heap.set_slot(cell, ELEMENT_STATE, heap.get(state));
                    heap.set_slot(cell, ELEMENT_INDEX, Value::int32(index));
                    heap.set_slot(cell, ELEMENT_CALLED, Value::FALSE);
                }
            }
            let (on_fulfilled, on_rejected) = match combinator {
                Combinator::AllSettled => (
                    create_builtin_closure(agent, inner, realm, &ALL_SETTLED_FULFILLED, element)?,
                    create_builtin_closure(agent, inner, realm, &ALL_SETTLED_REJECTED, element)?,
                ),
                Combinator::Any => (
                    capability.resolve,
                    create_builtin_closure(agent, inner, realm, &ANY_REJECTED, element)?,
                ),
                Combinator::All | Combinator::Race => (
                    create_builtin_closure(agent, inner, realm, &ALL_FULFILLED, element)?,
                    capability.reject,
                ),
            };
            change_remaining(agent, state, 1)?;
            let key = property_key(agent, inner, "then")?;
            invoke(agent, inner, realm, next, key, &[on_fulfilled, on_rejected])?;
            Ok(false)
        })?;
        if finished {
            break;
        }
    }
    if change_remaining(agent, state, -1)? == 0 {
        finish(agent, scope, realm, combinator, state)?;
    }
    Ok(capability.promise)
}

/// Hängt *undefined* an die Liste des Zustands `state` und gibt den Index
/// zurück.
fn append_undefined(agent: &mut Agent, scope: &Scope, state: Local<'_>) -> Completion<i32> {
    let heap = agent.heap();
    let Kind::Internal(cell) = heap.get(state).kind() else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let list = heap.local(scope, heap.slot(cell, VALUES).unwrap_or(Value::UNDEFINED));
    let list = list_reserve(agent, scope, list, 1)?;
    let heap = agent.heap();
    if let Kind::Internal(cell) = heap.get(state).kind() {
        heap.set_slot(cell, VALUES, heap.get(list));
    }
    let index = list_push(heap, list, &[Value::UNDEFINED])?;
    // Mehr Elemente als Indizes eines Arrays gibt CreateArrayFromList nicht
    // her.
    i32::try_from(index).map_err(|_| agent.throw_error(ErrorKind::RangeError))
}

/// Ändert *remainingElementsCount* des Zustands `state` um `delta` und
/// gibt den neuen Wert zurück.
fn change_remaining(agent: &mut Agent, state: Local<'_>, delta: i32) -> Completion<i32> {
    let heap = agent.heap();
    let changed = match heap.get(state).kind() {
        Kind::Internal(cell) => heap
            .slot(cell, REMAINING)
            .and_then(Value::as_int32)
            .and_then(|remaining| remaining.checked_add(delta))
            .inspect(|&remaining| {
                heap.set_slot(cell, REMAINING, Value::int32(remaining));
            }),
        _ => None,
    };
    changed.ok_or_else(|| agent.throw_error(ErrorKind::RangeError))
}

/// Die Schritte, wenn *remainingElementsCount* 0 wird: die Liste als Array
/// an [[Resolve]], bei Promise.any in einem neuen AggregateError an
/// [[Reject]]. `realm` ist der aktuelle Realm.
fn finish<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    combinator: Combinator,
    state: Local<'_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let Kind::Internal(cell) = heap.get(state).kind() else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let read = |slot| heap.local(scope, heap.slot(cell, slot).unwrap_or(Value::UNDEFINED));
    let (list, resolve, reject) = (read(VALUES), read(RESULT_RESOLVE), read(RESULT_REJECT));
    let values = heap.get(list);
    let elements: Vec<Local<'_>> = (0..list_len(heap, values))
        .map(|index| heap.local(scope, list_value(heap, values, 1, index, 0)))
        .collect();
    let undefined = heap.local(scope, Value::UNDEFINED);
    if combinator == Combinator::Any {
        let error = aggregate_error_create(agent, scope, realm, &elements)?;
        return call(agent, scope, reject, undefined, &[error]);
    }
    let array = create_array_from_list(agent, scope, realm, &elements)?;
    call(agent, scope, resolve, undefined, &[array])
}

/// Die Schritte einer Funktion für ein Element: Ist [[AlreadyCalled]]
/// gesetzt, geschieht nichts; sonst kommt `value` an ihren Index der
/// Liste, bei Promise.allSettled als Objekt mit `status` und `value` oder
/// `reason`, und wird *remainingElementsCount* 0, schließt der Aufruf ab.
fn element<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    combinator: Combinator,
    settled: Option<&str>,
) -> Completion<Local<'s>> {
    let captures = arguments.captures(agent, scope);
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let (state, index) = {
        let heap = agent.heap();
        let Kind::Internal(cell) = heap.get(captures).kind() else {
            return Ok(undefined);
        };
        if matches!(
            heap.slot(cell, ELEMENT_CALLED).map(|called| called.kind()),
            Some(Kind::Boolean(true))
        ) {
            return Ok(undefined);
        }
        heap.set_slot(cell, ELEMENT_CALLED, Value::TRUE);
        let state = heap.local(
            scope,
            heap.slot(cell, ELEMENT_STATE).unwrap_or(Value::UNDEFINED),
        );
        let index = heap
            .slot(cell, ELEMENT_INDEX)
            .and_then(Value::as_int32)
            .and_then(|index| usize::try_from(index).ok())
            .unwrap_or(0);
        (state, index)
    };
    let argument = arguments.argument(agent, scope, 0);
    let value = match settled {
        None => argument,
        Some(status) => {
            let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::ObjectPrototype);
            let object = ordinary_object_create(agent, scope, Some(prototype))?;
            let text = agent.heap_mut().alloc_string(scope, &units(status))?;
            let key = property_key(agent, scope, "status")?;
            create_data_property_or_throw(agent, scope, object, key, text)?;
            let name = if status == "fulfilled" {
                "value"
            } else {
                "reason"
            };
            let key = property_key(agent, scope, name)?;
            create_data_property_or_throw(agent, scope, object, key, argument)?;
            object
        }
    };
    {
        let heap = agent.heap();
        if let Kind::Internal(cell) = heap.get(state).kind() {
            let list = heap.slot(cell, VALUES).unwrap_or(Value::UNDEFINED);
            list_set(heap, list, 1, index, 0, heap.get(value));
        }
    }
    if change_remaining(agent, state, -1)? != 0 {
        return Ok(undefined);
    }
    finish(agent, scope, arguments.realm, combinator, state)
}

/// Die Funktion *onFulfilled* aus PerformPromiseAll.
fn all_fulfilled<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    element(agent, scope, arguments, Combinator::All, None)
}

/// Die Funktion *onFulfilled* aus PerformPromiseAllSettled.
fn all_settled_fulfilled<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    element(
        agent,
        scope,
        arguments,
        Combinator::AllSettled,
        Some("fulfilled"),
    )
}

/// Die Funktion *onRejected* aus PerformPromiseAllSettled.
fn all_settled_rejected<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    element(
        agent,
        scope,
        arguments,
        Combinator::AllSettled,
        Some("rejected"),
    )
}

/// Die Funktion *onRejected* aus PerformPromiseAny.
fn any_rejected<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    element(agent, scope, arguments, Combinator::Any, None)
}

// Die Eigenschaften des Prototyps

/// Promise.prototype.catch ( onRejected ) (Abschnitt 27.5.5.1).
fn catch<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let on_rejected = arguments.argument(agent, scope, 0);
    let key = property_key(agent, scope, "then")?;
    invoke(
        agent,
        scope,
        arguments.realm,
        arguments.this,
        key,
        &[undefined, on_rejected],
    )
}

/// Promise.prototype.finally ( onFinally ) (Abschnitt 27.5.5.3). Beide
/// Closures fangen dieselbe Zelle mit *onFinally* und *C* ein.
fn finally<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let promise = arguments.this;
    if !is_object(agent, promise) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let realm = arguments.realm;
    let default = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let constructor = species_constructor(agent, scope, promise, default)?;
    let on_finally = arguments.argument(agent, scope, 0);
    let (then_finally, catch_finally) = if is_callable(agent.heap(), agent.heap().get(on_finally)) {
        let captures = agent.heap_mut().alloc_slots(scope, 2)?;
        let heap = agent.heap();
        if let Kind::Internal(cell) = heap.get(captures).kind() {
            heap.set_slot(cell, 0, heap.get(on_finally));
            heap.set_slot(cell, 1, heap.get(constructor));
        }
        (
            create_builtin_closure(agent, scope, realm, &THEN_FINALLY, captures)?,
            create_builtin_closure(agent, scope, realm, &CATCH_FINALLY, captures)?,
        )
    } else {
        (on_finally, on_finally)
    };
    let key = property_key(agent, scope, "then")?;
    invoke(
        agent,
        scope,
        realm,
        promise,
        key,
        &[then_finally, catch_finally],
    )
}

/// Die ersten beiden Schritte von *thenFinally* und *catchFinally*: Call
/// von *onFinally* und PromiseResolve mit *C*.
fn finally_result<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let captures = arguments.captures(agent, scope);
    let heap = agent.heap();
    let Kind::Internal(cell) = heap.get(captures).kind() else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let read = |slot| heap.local(scope, heap.slot(cell, slot).unwrap_or(Value::UNDEFINED));
    let (on_finally, constructor) = (read(0), read(1));
    let undefined = heap.local(scope, Value::UNDEFINED);
    let result = call(agent, scope, on_finally, undefined, &[])?;
    promise_resolve(agent, scope, arguments.realm, constructor, result)
}

/// *thenFinally* aus Promise.prototype.finally.
fn then_finally<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let promise = finally_result(agent, scope, arguments)?;
    let thunk = create_builtin_closure(agent, scope, arguments.realm, &VALUE_THUNK, value)?;
    let key = property_key(agent, scope, "then")?;
    invoke(agent, scope, arguments.realm, promise, key, &[thunk])
}

/// *catchFinally* aus Promise.prototype.finally.
fn catch_finally<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let reason = arguments.argument(agent, scope, 0);
    let promise = finally_result(agent, scope, arguments)?;
    let thrower = create_builtin_closure(agent, scope, arguments.realm, &THROWER, reason)?;
    let key = property_key(agent, scope, "then")?;
    invoke(agent, scope, arguments.realm, promise, key, &[thrower])
}

/// *valueThunk*: gibt den Wert zurück, den sie einfängt.
fn value_thunk<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(arguments.captures(agent, scope))
}

/// *thrower*: wirft den Wert, den sie einfängt.
fn thrower<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let reason = arguments.captures(agent, scope);
    Err(agent.throw_value(reason))
}

/// Promise.prototype.then ( onFulfilled, onRejected ) (Abschnitt
/// 27.5.5.4).
fn then<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let promise = arguments.this;
    if !is_promise(agent.heap(), agent.heap().get(promise)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let realm = arguments.realm;
    let default = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let constructor = species_constructor(agent, scope, promise, default)?;
    let capability = new_promise_capability(agent, scope, realm, constructor)?;
    let on_fulfilled = arguments.argument(agent, scope, 0);
    let on_rejected = arguments.argument(agent, scope, 1);
    perform_promise_then(
        agent,
        scope,
        realm,
        promise,
        on_fulfilled,
        on_rejected,
        Some(capability),
    )
}
