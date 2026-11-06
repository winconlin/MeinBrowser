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

#![forbid(unsafe_code)]

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::conversion::to_boolean;
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::ordinary_object_create;
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

/// GetIterator (Abschnitt 7.4.4) mit ~sync~, GetIteratorFromMethod
/// (Abschnitt 7.4.3) und GetIteratorDirect (Abschnitt 7.4.2): der Iterator
/// und der Wert seiner Eigenschaft `next`. Ob der aufrufbar ist, zeigt erst
/// der erste Aufruf.
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
    let iterator = call(agent, scope, method, value, &[])?;
    if !is_object(agent, iterator) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let key = agent.key_from_units(scope, &units("next"))?;
    let next = get(agent, scope, iterator, key)?;
    Ok((iterator, next))
}

/// IteratorStep (Abschnitt 7.4.9) mit IteratorNext und IteratorComplete
/// (Abschnitte 7.4.6 und 7.4.7): das Ergebnis von `next` oder `None`, wenn
/// der Iterator fertig ist. Danach gilt [[Done]] bei `None` und nach jedem
/// Fehler.
pub(crate) fn iterator_step<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    iterator: Local<'_>,
    next: Local<'_>,
) -> Completion<Option<Local<'s>>> {
    let result = call(agent, scope, next, iterator, &[])?;
    if !is_object(agent, result) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let key = agent.key_from_units(scope, &units("done"))?;
    let done = get(agent, scope, result, key)?;
    if to_boolean(agent.heap(), agent.heap().get(done)) {
        return Ok(None);
    }
    Ok(Some(result))
}

/// IteratorStepValue (Abschnitt 7.4.10) mit IteratorStep und IteratorValue
/// (Abschnitt 7.4.8): der nächste Wert oder `None`, wenn der Iterator
/// fertig ist. Danach gilt [[Done]] bei `None` und nach jedem Fehler.
pub(crate) fn iterator_step_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    iterator: Local<'_>,
    next: Local<'_>,
) -> Completion<Option<Local<'s>>> {
    let Some(result) = iterator_step(agent, scope, iterator, next)? else {
        return Ok(None);
    };
    let key = agent.key_from_units(scope, &units("value"))?;
    Ok(Some(get(agent, scope, result, key)?))
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
