//! %AsyncIteratorPrototype% (Abschnitt 27.1.4), bisher nur mit
//! @@asyncIterator, und %AsyncFromSyncIteratorPrototype% (Abschnitt
//! 27.1.5.2) mit `next`, `return` und `throw`.
//!
//! Einen Async-from-Sync Iterator legt GetIterator mit ~async~ für Objekte
//! ohne @@asyncIterator an (`crate::iterator::get_async_iterator`). Er ist
//! ein Objekt der Art [`ObjectClass::AsyncFromSyncIterator`] mit
//! [[Iterator]] und [[NextMethod]] seines [[SyncIteratorRecord]]; dessen
//! [[Done]] liest keine der Methoden. Von den Closures aus
//! AsyncFromSyncIteratorContinuation fängt *unwrap* *done* ein und
//! *closeIterator* den synchronen Iterator, denn IteratorClose liest nur
//! [[Iterator]].
//!
//! @@asyncDispose an %AsyncIteratorPrototype% fehlt noch, wie @@dispose an
//! %Iterator.prototype%.

use super::promise::if_abrupt_reject_promise;
use super::{define_functions, define_symbol_function, units};
use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_closure};
use crate::heap::{Local, ObjectClass, Scope};
use crate::iterator::{
    SYNC_ITERATOR, SYNC_NEXT, create_iterator_result_object, iterator_close,
    iterator_close_on_throw, iterator_complete, iterator_next, iterator_value,
};
use crate::key::WellKnownSymbol;
use crate::object::ordinary_object_create;
use crate::operations::{call, get_method};
use crate::promise::{Capability, new_promise_capability, perform_promise_then, promise_resolve};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static ASYNC_ITERATOR_PROTOTYPE_ASYNC_ITERATOR: Builtin = Builtin {
    name: "[Symbol.asyncIterator]",
    length: 0,
    constructor: false,
    behaviour: async_iterator_prototype_async_iterator,
};

/// Der Parameter der drei Methoden ist optional und zählt darum nicht zu
/// `length` (Abschnitt 18).
static ASYNC_FROM_SYNC_FUNCTIONS: [Builtin; 3] = [
    Builtin {
        name: "next",
        length: 0,
        constructor: false,
        behaviour: async_from_sync_next,
    },
    Builtin {
        name: "return",
        length: 0,
        constructor: false,
        behaviour: async_from_sync_return,
    },
    Builtin {
        name: "throw",
        length: 0,
        constructor: false,
        behaviour: async_from_sync_throw,
    },
];

/// Die Closures aus AsyncFromSyncIteratorContinuation (Abschnitt
/// 27.1.5.4): ohne Namen und ohne [[Construct]].
static UNWRAP: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: unwrap,
};
static CLOSE_ITERATOR: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: close_iterator,
};

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(object))?;
    define_symbol_function(
        agent,
        scope,
        realm,
        prototype,
        WellKnownSymbol::AsyncIterator,
        &ASYNC_ITERATOR_PROTOTYPE_ASYNC_ITERATOR,
    )?;
    agent.set_intrinsic(realm, Intrinsic::AsyncIteratorPrototype, prototype);
    let async_from_sync = ordinary_object_create(agent, scope, Some(prototype))?;
    define_functions(
        agent,
        scope,
        realm,
        async_from_sync,
        &ASYNC_FROM_SYNC_FUNCTIONS,
    )?;
    agent.set_intrinsic(
        realm,
        Intrinsic::AsyncFromSyncIteratorPrototype,
        async_from_sync,
    );
    Ok(())
}

/// %AsyncIteratorPrototype% [ %Symbol.asyncIterator% ] ( ) (Abschnitt
/// 27.1.4.2).
fn async_iterator_prototype_async_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    Ok(heap.local(scope, heap.get(arguments.this)))
}

/// [[Iterator]] und [[NextMethod]] aus [[SyncIteratorRecord]] von `this`.
/// Die Spezifikation setzt voraus, dass `this` ein Async-from-Sync Iterator
/// ist, denn Code der Sprache bekommt keinen zu sehen; für andere Werte
/// wirft es einen TypeError.
fn sync_record<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    this: Local<'_>,
) -> Completion<(Local<'s>, Local<'s>)> {
    let heap = agent.heap();
    let fields = match heap.get(this).kind() {
        Kind::Object(object)
            if heap.object_class(object) == Some(ObjectClass::AsyncFromSyncIterator) =>
        {
            heap.async_from_sync_field(object, SYNC_ITERATOR)
                .zip(heap.async_from_sync_field(object, SYNC_NEXT))
        }
        _ => None,
    };
    let Some((iterator, next)) = fields else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    Ok((heap.local(scope, iterator), heap.local(scope, next)))
}

/// NewPromiseCapability(%Promise%) mit %Promise% aus `realm`.
fn intrinsic_capability<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
) -> Completion<Capability<'s>> {
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
    new_promise_capability(agent, scope, realm, constructor)
}

/// Das erste Argument, wenn es eines gibt: „If _value_ is present“.
fn present<'a, 'l>(arguments: &BuiltinCall<'a, 'l>) -> &'a [Local<'l>] {
    arguments.arguments.get(..1).unwrap_or(&[])
}

/// Call von [[Reject]] mit einem neuen TypeError, dann [[Promise]].
fn reject_with_type_error<'s>(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    capability: &Capability<'s>,
) -> Completion<Local<'s>> {
    let abrupt = agent.throw_error(ErrorKind::TypeError);
    if_abrupt_reject_promise(agent, scope, realm, abrupt, capability)
}

/// %AsyncFromSyncIteratorPrototype%.next ( [ value ] ) (Abschnitt
/// 27.1.5.2.1).
fn async_from_sync_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let (iterator, next) = sync_record(agent, scope, arguments.this)?;
    let capability = intrinsic_capability(agent, scope, realm)?;
    let result = match iterator_next(agent, scope, iterator, next, present(arguments)) {
        Ok(result) => result,
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    continuation(agent, scope, realm, result, capability, iterator, true)
}

/// %AsyncFromSyncIteratorPrototype%.return ( [ value ] ) (Abschnitt
/// 27.1.5.2.2).
fn async_from_sync_return<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let (iterator, _) = sync_record(agent, scope, arguments.this)?;
    let capability = intrinsic_capability(agent, scope, realm)?;
    let key = agent.key_from_units(scope, &units("return"))?;
    let method = match get_method(agent, scope, iterator, key) {
        Ok(method) => method,
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    let Some(method) = method else {
        let value = arguments.argument(agent, scope, 0);
        let result = create_iterator_result_object(agent, scope, realm, value, true)?;
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        call(agent, scope, capability.resolve, undefined, &[result])?;
        return Ok(capability.promise);
    };
    let result = match call(agent, scope, method, iterator, present(arguments)) {
        Ok(result) => result,
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    if !matches!(agent.heap().get(result).kind(), Kind::Object(_)) {
        return reject_with_type_error(agent, scope, realm, &capability);
    }
    continuation(agent, scope, realm, result, capability, iterator, false)
}

/// %AsyncFromSyncIteratorPrototype%.throw ( [ value ] ) (Abschnitt
/// 27.1.5.2.3). Ohne `throw` am synchronen Iterator schließt es ihn und
/// lehnt mit einem TypeError ab.
fn async_from_sync_throw<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let (iterator, _) = sync_record(agent, scope, arguments.this)?;
    let capability = intrinsic_capability(agent, scope, realm)?;
    let key = agent.key_from_units(scope, &units("throw"))?;
    let method = match get_method(agent, scope, iterator, key) {
        Ok(method) => method,
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    let Some(method) = method else {
        if let Err(abrupt) = iterator_close(agent, scope, iterator) {
            return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability);
        }
        return reject_with_type_error(agent, scope, realm, &capability);
    };
    let result = match call(agent, scope, method, iterator, present(arguments)) {
        Ok(result) => result,
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    if !matches!(agent.heap().get(result).kind(), Kind::Object(_)) {
        return reject_with_type_error(agent, scope, realm, &capability);
    }
    continuation(agent, scope, realm, result, capability, iterator, true)
}

/// AsyncFromSyncIteratorContinuation (Abschnitt 27.1.5.4) mit dem
/// synchronen Iterator `iterator`.
fn continuation<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    result: Local<'_>,
    capability: Capability<'s>,
    iterator: Local<'_>,
    close_on_rejection: bool,
) -> Completion<Local<'s>> {
    let done = match iterator_complete(agent, scope, result) {
        Ok(done) => done,
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    let value = match iterator_value(agent, scope, result) {
        Ok(value) => value,
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let wrapper = match promise_resolve(agent, scope, realm, constructor, value) {
        Ok(wrapper) => wrapper,
        Err(Abrupt::Throw) if !done && close_on_rejection => {
            // IteratorClose mit der Ausnahme: Sie gilt, auch wenn das
            // Schließen scheitert.
            let reason = agent.take_thrown_value(scope, realm)?;
            iterator_close_on_throw(agent, scope, iterator)?;
            let undefined = agent.heap().local(scope, Value::UNDEFINED);
            call(agent, scope, capability.reject, undefined, &[reason])?;
            return Ok(capability.promise);
        }
        Err(abrupt) => return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability),
    };
    let captured = agent.heap().local(scope, Value::boolean(done));
    let on_fulfilled = create_builtin_closure(agent, scope, realm, &UNWRAP, captured)?;
    let on_rejected = if done || !close_on_rejection {
        agent.heap().local(scope, Value::UNDEFINED)
    } else {
        create_builtin_closure(agent, scope, realm, &CLOSE_ITERATOR, iterator)?
    };
    perform_promise_then(
        agent,
        scope,
        realm,
        wrapper,
        on_fulfilled,
        on_rejected,
        Some(capability),
    )?;
    Ok(capability.promise)
}

/// *unwrap*: CreateIteratorResultObject mit dem Wert und dem eingefangenen
/// *done*.
fn unwrap<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let done = arguments.captures(agent, scope);
    let done = matches!(agent.heap().get(done).kind(), Kind::Boolean(true));
    create_iterator_result_object(agent, scope, arguments.realm, value, done)
}

/// *closeIterator*: IteratorClose des eingefangenen Iterators mit der
/// Ausnahme `error`, die danach gilt.
fn close_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let error = arguments.argument(agent, scope, 0);
    let iterator = arguments.captures(agent, scope);
    iterator_close_on_throw(agent, scope, iterator)?;
    Err(agent.throw_value(error))
}
