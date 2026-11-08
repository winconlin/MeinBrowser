//! Die abstrakten Operationen für Iteratoren (Abschnitt 7.4), mit denen
//! `for`-`of`, die Ausbreitung in Array-Literalen und Aufrufen,
//! Array-Muster und die eingebauten Iteratoren arbeiten.
//!
//! Ein Iterator Record (Abschnitt 7.4.1) liegt im Interpreter in drei
//! Registern hintereinander: [[Iterator]], [[NextMethod]] und [[Done]]. Die
//! Funktionen hier bekommen die ersten beiden als Handles; [[Done]] setzt
//! der Befehl, der sie aufruft. Iteratoren, die sich selbst rufen, gibt es
//! nicht: Jeder Aufruf von `next` oder `return` ist ein neuer Lauf des
//! Interpreters wie der eines Getters und kostet Rust-Stapel nur im Rahmen
//! von [`crate::agent::STACK_BUDGET`].
//!
//! GetIterator mit ~async~ legt für Objekte ohne @@asyncIterator einen
//! Async-from-Sync Iterator an (Abschnitt 27.1.5), ein Objekt der Art
//! [`ObjectClass::AsyncFromSyncIterator`] mit [[Iterator]] und
//! [[NextMethod]] seines [[SyncIteratorRecord]]; seine Methoden stehen in
//! `builtins`.

#![forbid(unsafe_code)]

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::conversion::to_boolean;
use crate::heap::{Local, ObjectClass, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{INLINE, ordinary_object_create};
use crate::operations::{call, create_data_property_or_throw, get, get_method, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::reference::{get_value, to_object_base};
use crate::value::{Kind, Value};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

/// GetMethod (Abschnitt 7.3.10) für einen beliebigen Wert, mit GetV
/// (Abschnitt 7.3.3): ein primitiver Wert liest über den Prototyp seines
/// Wrappers aus `realm`. `None` steht für *undefined*.
fn get_method_of_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Option<Local<'s>>> {
    let base = to_object_base(agent, scope, realm, value)?;
    let function = get_value(agent, scope, base, value, key)?;
    let found = agent.heap().get(function);
    if found.is_undefined() || found.is_null() {
        return Ok(None);
    }
    if !is_callable(agent.heap(), found) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(Some(function))
}

/// Die Felder eines Async-from-Sync Iterator: [[Iterator]] und
/// [[NextMethod]] seines [[SyncIteratorRecord]].
pub(crate) const SYNC_ITERATOR: usize = 0;
pub(crate) const SYNC_NEXT: usize = 1;

/// GetIterator (Abschnitt 7.4.4) mit ~sync~: der Iterator und der Wert
/// seiner Eigenschaft `next`. Ob der aufrufbar ist, zeigt erst der erste
/// Aufruf.
pub(crate) fn get_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'_>,
) -> Completion<(Local<'s>, Local<'s>)> {
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::Iterator);
    let method = get_method_of_value(agent, scope, realm, value, PropertyKey::Symbol(symbol))?;
    let Some(method) = method else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    get_iterator_from_method(agent, scope, value, method)
}

/// GetIterator mit ~async~: der Iterator aus @@asyncIterator und der Wert
/// seiner Eigenschaft `next`. Fehlt @@asyncIterator, ist es ein
/// Async-from-Sync Iterator um den Iterator aus @@iterator.
pub(crate) fn get_async_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'_>,
) -> Completion<(Local<'s>, Local<'s>)> {
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::AsyncIterator);
    let method = get_method_of_value(agent, scope, realm, value, PropertyKey::Symbol(symbol))?;
    if let Some(method) = method {
        return get_iterator_from_method(agent, scope, value, method);
    }
    let (iterator, next) = get_iterator(agent, scope, realm, value)?;
    create_async_from_sync_iterator(agent, scope, realm, iterator, next)
}

/// GetIteratorFromMethod (Abschnitt 7.4.3) mit GetIteratorDirect
/// (Abschnitt 7.4.2).
fn get_iterator_from_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
    method: Local<'_>,
) -> Completion<(Local<'s>, Local<'s>)> {
    let iterator = call(agent, scope, method, value, &[])?;
    if !is_object(agent, iterator) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let key = agent.key_from_units(scope, &units("next"))?;
    let next = get(agent, scope, iterator, key)?;
    Ok((iterator, next))
}

/// CreateAsyncFromSyncIterator (Abschnitt 27.1.5.1) mit
/// %AsyncFromSyncIteratorPrototype% aus `realm`, dem aktuellen Realm: der
/// neue Iterator und der Wert seiner Eigenschaft `next`.
fn create_async_from_sync_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    iterator: Local<'_>,
    next: Local<'_>,
) -> Completion<(Local<'s>, Local<'s>)> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::AsyncFromSyncIteratorPrototype);
    let shape = agent.empty_shape(scope);
    let object = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::AsyncFromSyncIterator,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    if let Kind::Object(cell) = heap.get(object).kind() {
        heap.set_async_from_sync_field(cell, SYNC_ITERATOR, heap.get(iterator));
        heap.set_async_from_sync_field(cell, SYNC_NEXT, heap.get(next));
    }
    let key = agent.key_from_units(scope, &units("next"))?;
    let method = get(agent, scope, object, key)?;
    Ok((object, method))
}

/// IteratorNext (Abschnitt 7.4.6): das Ergebnis von `next` mit den
/// Argumenten `arguments`, höchstens einem.
pub(crate) fn iterator_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    iterator: Local<'_>,
    next: Local<'_>,
    arguments: &[Local<'_>],
) -> Completion<Local<'s>> {
    let result = call(agent, scope, next, iterator, arguments)?;
    if !is_object(agent, result) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(result)
}

/// IteratorComplete (Abschnitt 7.4.7).
pub(crate) fn iterator_complete(
    agent: &mut Agent,
    scope: &Scope,
    result: Local<'_>,
) -> Completion<bool> {
    let key = agent.key_from_units(scope, &units("done"))?;
    let done = get(agent, scope, result, key)?;
    Ok(to_boolean(agent.heap(), agent.heap().get(done)))
}

/// IteratorValue (Abschnitt 7.4.8).
pub(crate) fn iterator_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    result: Local<'_>,
) -> Completion<Local<'s>> {
    let key = agent.key_from_units(scope, &units("value"))?;
    get(agent, scope, result, key)
}

/// IteratorStep (Abschnitt 7.4.9) mit IteratorNext und IteratorComplete:
/// das Ergebnis von `next` oder `None`, wenn der Iterator fertig ist.
/// Danach gilt [[Done]] bei `None` und nach jedem Fehler.
pub(crate) fn iterator_step<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    iterator: Local<'_>,
    next: Local<'_>,
) -> Completion<Option<Local<'s>>> {
    let result = iterator_next(agent, scope, iterator, next, &[])?;
    if iterator_complete(agent, scope, result)? {
        return Ok(None);
    }
    Ok(Some(result))
}

/// IteratorStepValue (Abschnitt 7.4.10) mit IteratorStep und IteratorValue:
/// der nächste Wert oder `None`, wenn der Iterator fertig ist. Danach gilt
/// [[Done]] bei `None` und nach jedem Fehler.
pub(crate) fn iterator_step_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    iterator: Local<'_>,
    next: Local<'_>,
) -> Completion<Option<Local<'s>>> {
    let Some(result) = iterator_step(agent, scope, iterator, next)? else {
        return Ok(None);
    };
    Ok(Some(iterator_value(agent, scope, result)?))
}

/// IteratorClose (Abschnitt 7.4.11) für eine Beendigung, die keine Ausnahme
/// ist, etwa `break` oder `return`: Ein Fehler aus `return` gilt, und ein
/// Ergebnis, das kein Objekt ist, wirft einen TypeError.
pub(crate) fn iterator_close(
    agent: &mut Agent,
    scope: &Scope,
    iterator: Local<'_>,
) -> Completion<()> {
    let key = agent.key_from_units(scope, &units("return"))?;
    let Some(method) = get_method(agent, scope, iterator, key)? else {
        return Ok(());
    };
    let result = call(agent, scope, method, iterator, &[])?;
    if !is_object(agent, result) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(())
}

/// IteratorClose für eine Ausnahme: Was GetMethod oder `return` werfen,
/// fällt weg, die Ausnahme davor bleibt. Andere abrupte Beendigungen wie ein
/// erschöpftes Budget gehen weiter; sie fängt kein Handler.
pub(crate) fn iterator_close_on_throw(
    agent: &mut Agent,
    scope: &Scope,
    iterator: Local<'_>,
) -> Completion<()> {
    let result = agent.scope(|agent, inner| -> Completion<()> {
        let key = agent.key_from_units(inner, &units("return"))?;
        if let Some(method) = get_method(agent, inner, iterator, key)? {
            call(agent, inner, method, iterator, &[])?;
        }
        Ok(())
    });
    match result {
        Err(Abrupt::Throw) => {
            agent.take_exception(scope);
            Ok(())
        }
        other => other,
    }
}

/// CreateIteratorResultObject (Abschnitt 7.4.17) mit %Object.prototype% aus
/// `realm`, dem aktuellen Realm.
pub(crate) fn create_iterator_result_object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'_>,
    done: bool,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let object = ordinary_object_create(agent, scope, Some(prototype))?;
    let key = agent.key_from_units(scope, &units("value"))?;
    create_data_property_or_throw(agent, scope, object, key, value)?;
    let done = agent.heap().local(scope, Value::boolean(done));
    let key = agent.key_from_units(scope, &units("done"))?;
    create_data_property_or_throw(agent, scope, object, key, done)?;
    Ok(object)
}
