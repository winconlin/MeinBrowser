//! Was `using` und `await using` zur Laufzeit von eingebauten Funktionen
//! brauchen: CreateDisposableResource mit GetDisposeMethod für
//! [`Op::AddDisposableResource`](crate::bytecode::Op::AddDisposableResource).
//! DisposeResources selbst übersetzt der Übersetzer in Bytecode, denn es
//! wartet bei `await using` mit Await.
//!
//! Hat ein Wert bei `await using` kein @@asyncDispose, nimmt
//! GetDisposeMethod @@dispose in einer neuen eingebauten Funktion ohne Namen
//! und mit `length` 0. Sie ruft @@dispose mit ihrem `this` auf und gibt ein
//! Promise zurück, das mit *undefined* erfüllt oder mit dem Fehler abgelehnt
//! ist. So wartet DisposeResources nie auf das Ergebnis von @@dispose, und
//! ein Fehler kommt erst mit dem Promise.

use super::promise::if_abrupt_reject_promise;
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_closure};
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::operations::{call, get_method};
use crate::promise::new_promise_capability;
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Die Funktion um @@dispose aus GetDisposeMethod mit ~async-dispose~.
static ASYNC_FROM_SYNC_DISPOSE: Builtin = Builtin {
    name: "",
    length: 0,
    constructor: false,
    behaviour: async_from_sync_dispose,
};

/// CreateDisposableResource ohne Methode für den Wert `value`, mit
/// ~async-dispose~, wenn `asynchronous` gilt: die Methode und der Wert für
/// die beiden Register. Bei ~sync-dispose~ mit *null* oder *undefined*
/// kommt nach AddDisposableResource nichts hinzu, das ist `None`; bei
/// ~async-dispose~ sind es *null* als Zeichen für eine Ressource ohne
/// Methode und *undefined*. `realm` ist der aktuelle Realm.
pub(crate) fn create_disposable_resource<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    value: Local<'s>,
    asynchronous: bool,
) -> Completion<Option<(Local<'s>, Local<'s>)>> {
    let heap = agent.heap();
    let found = heap.get(value);
    if found.is_undefined() || found.is_null() {
        if !asynchronous {
            return Ok(None);
        }
        let null = heap.local(scope, Value::NULL);
        let undefined = heap.local(scope, Value::UNDEFINED);
        return Ok(Some((null, undefined)));
    }
    if !matches!(found.kind(), Kind::Object(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    match get_dispose_method(agent, scope, realm, value, asynchronous)? {
        Some(method) => Ok(Some((method, value))),
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// GetDisposeMethod für das Objekt `object`, mit ~async-dispose~, wenn
/// `asynchronous` gilt.
fn get_dispose_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    object: Local<'_>,
    asynchronous: bool,
) -> Completion<Option<Local<'s>>> {
    if asynchronous {
        let symbol = agent.well_known_symbol(scope, WellKnownSymbol::AsyncDispose);
        if let Some(method) = get_method(agent, scope, object, PropertyKey::Symbol(symbol))? {
            return Ok(Some(method));
        }
    }
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::Dispose);
    let Some(method) = get_method(agent, scope, object, PropertyKey::Symbol(symbol))? else {
        return Ok(None);
    };
    if !asynchronous {
        return Ok(Some(method));
    }
    let closure = create_builtin_closure(agent, scope, realm, &ASYNC_FROM_SYNC_DISPOSE, method)?;
    Ok(Some(closure))
}

/// Die Funktion um @@dispose: Call mit `this`, dann ein Promise mit
/// *undefined* oder dem Fehler (IfAbruptRejectPromise).
fn async_from_sync_dispose<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let method = arguments.captures(agent, scope);
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let capability = new_promise_capability(agent, scope, realm, constructor)?;
    if let Err(abrupt) = call(agent, scope, method, arguments.this, &[]) {
        return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability);
    }
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    call(agent, scope, capability.resolve, undefined, &[undefined])?;
    Ok(capability.promise)
}
