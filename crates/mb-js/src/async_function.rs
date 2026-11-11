//! async-Funktionen (Abschnitte 15.8, 15.9 und 27.10): der Zustand einer
//! async-Funktion, die läuft oder bei `await` angehalten ist, und Await
//! (Abschnitt 27.10.5.3).
//!
//! Der Zustand ist eine interne Zelle mit sieben Plätzen, die nur der Code
//! der Funktion in einem Register und die beiden Funktionen eines `await`
//! sehen: das Funktionsobjekt, aus dem Code und Realm des Rahmens kommen;
//! der angehaltene Rahmen als interne Zelle mit dem Kopf und den Registern,
//! *undefined* bis zum ersten `await` und wieder, sobald die Funktion fertig
//! ist; der Befehl, bei dem der Rahmen weitergeht, [`RUNNING`], solange er
//! läuft; ob er mit einem Grund fortgesetzt wurde, bis der Befehl danach es
//! liest; und die PromiseCapability aus EvaluateAsyncFunctionBody (Abschnitt
//! 15.8.4) mit [[Promise]], [[Resolve]] und [[Reject]].
//!
//! Anders als der eines Generators beginnt der Rumpf sofort, im Rahmen des
//! Aufrufs (AsyncFunctionStart, Abschnitt 27.10.5.1). Ein `await` legt den
//! Rahmen in den Zustand und kehrt mit dem Promise zurück wie `return`, an
//! jeder Stelle im Stapel eines Laufs (RunCallerContext). Fortgesetzt wird
//! er von einer der beiden Funktionen, die Await an das Promise hängt: Ein
//! Job ruft sie auf, und sie legt den Rahmen in einem neuen Lauf wieder auf
//! den Registerstapel ([`resume_async`]). Hält er wieder an oder wird er
//! fertig, endet dieser Lauf, und die Funktion gibt *undefined* zurück.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_closure};
use crate::generator::Resumption;
use crate::heap::{Heap, Local, Scope};
use crate::interpreter::resume_async;
use crate::operations::call;
use crate::promise::{
    create_resolving_functions, perform_promise_then, promise_create, promise_resolve,
};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Der Platz mit dem Funktionsobjekt.
const FUNCTION: usize = 0;
/// Der Platz mit der Zelle für den Rahmen.
const FRAME: usize = 1;
/// Der Platz mit dem Befehl, bei dem der Rahmen weitergeht.
const OFFSET: usize = 2;
/// Der Platz, der sagt, ob der Rahmen mit einem Grund fortgesetzt wurde.
const THROW: usize = 3;
/// Die Plätze mit der PromiseCapability.
const PROMISE: usize = 4;
const RESOLVE: usize = 5;
const REJECT: usize = 6;
const SLOTS: usize = 7;

/// Im Platz [`OFFSET`]: Der Rahmen läuft oder ist fertig.
const RUNNING: i32 = -1;

/// Await, Schritte 3 und 4: *onFulfilled* setzt den Rahmen mit dem Wert
/// fort.
static AWAIT_FULFILLED: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: await_fulfilled,
};

/// Await, Schritte 5 und 6: *onRejected* setzt ihn mit dem Grund fort.
static AWAIT_REJECTED: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: await_rejected,
};

fn slot<'h>(heap: &'h Heap, state: Value<'_>, index: usize) -> Value<'h> {
    match state.kind() {
        Kind::Internal(cell) => heap.slot(cell, index).unwrap_or(Value::UNDEFINED),
        _ => Value::UNDEFINED,
    }
}

fn set_slot(heap: &Heap, state: Value<'_>, index: usize, value: Value<'_>) {
    if let Kind::Internal(cell) = state.kind() {
        heap.set_slot(cell, index, value);
    }
}

/// EvaluateAsyncFunctionBody und EvaluateAsyncConciseBody (Abschnitte
/// 15.8.4 und 15.9.3), Schritt 1: NewPromiseCapability(%Promise%) im
/// aktuellen Realm `realm` und der Zustand für das Funktionsobjekt
/// `function`, dessen Rahmen läuft.
///
/// NewPromiseCapability ruft dafür %Promise% mit einem Executor auf. Der
/// Konstruktor nimmt mit GetPrototypeFromConstructor %Promise.prototype%,
/// denn `prototype` an %Promise% ist weder beschreibbar noch konfigurierbar
/// (Abschnitt 27.5.4.4), und übergibt dem Executor die Funktionen aus
/// CreateResolvingFunctions. Dasselbe tut dieser Schritt ohne die Aufrufe;
/// beobachten lässt sich der Unterschied nicht.
pub(crate) fn async_start<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    function: Local<'_>,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::PromisePrototype);
    let promise = promise_create(agent, scope, prototype)?;
    let (resolve, reject) = create_resolving_functions(agent, scope, realm, promise)?;
    let state = agent.heap_mut().alloc_slots(scope, SLOTS)?;
    let heap = agent.heap();
    let value = heap.get(state);
    set_slot(heap, value, FUNCTION, heap.get(function));
    set_slot(heap, value, OFFSET, Value::int32(RUNNING));
    set_slot(heap, value, THROW, Value::FALSE);
    set_slot(heap, value, PROMISE, heap.get(promise));
    set_slot(heap, value, RESOLVE, heap.get(resolve));
    set_slot(heap, value, REJECT, heap.get(reject));
    Ok(state)
}

/// Die Zelle für den Rahmen mit `length` Plätzen, beim ersten `await` neu.
pub(crate) fn frame_cell<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
    length: usize,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let found = slot(heap, heap.get(state), FRAME);
    if !found.is_undefined() {
        return Ok(heap.local(scope, found));
    }
    let cell = agent.heap_mut().alloc_slots(scope, length)?;
    let heap = agent.heap();
    set_slot(heap, heap.get(state), FRAME, heap.get(cell));
    Ok(cell)
}

/// Await (Abschnitt 27.10.5.3), Schritte 2 bis 7, im aktuellen Realm
/// `realm`: PromiseResolve(%Promise%, `value`), dann PerformPromiseThen mit
/// *onFulfilled* und *onRejected*, die den Zustand `state` einfangen. Den
/// Rahmen hält danach der Interpreter an (Schritt 8).
pub(crate) fn await_value(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    value: Local<'_>,
) -> Completion<()> {
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let promise = promise_resolve(agent, scope, realm, constructor, value)?;
    perform_then_with(
        agent,
        scope,
        realm,
        promise,
        state,
        &AWAIT_FULFILLED,
        &AWAIT_REJECTED,
    )
}

/// PerformPromiseThen für `promise` ohne Capability für das Ergebnis, mit
/// zwei neuen eingebauten Funktionen aus `fulfilled` und `rejected` im
/// Realm `realm`, die `captures` einfangen: Await, Schritte 3 bis 7, auch
/// für async-Generatoren, und AsyncGeneratorAwaitReturn (Abschnitt
/// 27.9.3.9), Schritte 11 bis 15.
pub(crate) fn perform_then_with(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    promise: Local<'_>,
    captures: Local<'_>,
    fulfilled: &'static Builtin,
    rejected: &'static Builtin,
) -> Completion<()> {
    let on_fulfilled = create_builtin_closure(agent, scope, realm, fulfilled, captures)?;
    let on_rejected = create_builtin_closure(agent, scope, realm, rejected, captures)?;
    perform_promise_then(
        agent,
        scope,
        realm,
        promise,
        on_fulfilled,
        on_rejected,
        None,
    )?;
    Ok(())
}

/// Await, Schritt 8, bis auf den Wechsel des Laufs: Der Rahmen geht beim
/// Befehl bei `offset` weiter. `None`, wenn der Abstand nicht passt.
pub(crate) fn suspend(heap: &Heap, state: Value<'_>, offset: usize) -> Option<()> {
    let offset = i32::try_from(offset).ok()?;
    set_slot(heap, state, OFFSET, Value::int32(offset));
    Some(())
}

/// [[Promise]] der Capability.
pub(crate) fn promise<'h>(heap: &'h Heap, state: Value<'_>) -> Value<'h> {
    slot(heap, state, PROMISE)
}

/// Was das Fortsetzen des angehaltenen Rahmens braucht: das
/// Funktionsobjekt, die Zelle mit dem Rahmen und den nächsten Befehl.
/// `None`, wenn der Rahmen läuft oder die Funktion fertig ist.
pub(crate) fn suspended_parts<'s>(
    agent: &Agent,
    scope: &'s Scope,
    state: Local<'_>,
) -> Option<(Local<'s>, Local<'s>, usize)> {
    let heap = agent.heap();
    let value = heap.get(state);
    let offset = usize::try_from(slot(heap, value, OFFSET).as_int32()?).ok()?;
    let frame = slot(heap, value, FRAME);
    if frame.is_undefined() {
        return None;
    }
    let function = slot(heap, value, FUNCTION);
    Some((
        heap.local(scope, function),
        heap.local(scope, frame),
        offset,
    ))
}

/// Der Rahmen läuft wieder, fortgesetzt mit `resumption`. Erst dann, wenn
/// er wieder auf dem Registerstapel steht.
pub(crate) fn start_running(heap: &Heap, state: Value<'_>, resumption: Resumption) {
    set_slot(heap, state, OFFSET, Value::int32(RUNNING));
    let throw = Value::boolean(resumption == Resumption::Throw);
    set_slot(heap, state, THROW, throw);
}

/// Ob der Rahmen mit einem Grund fortgesetzt wurde. Danach gilt wieder ein
/// Wert, bis er das nächste Mal fortgesetzt wird.
pub(crate) fn take_throw(heap: &Heap, state: Value<'_>) -> bool {
    let throw = matches!(slot(heap, state, THROW).kind(), Kind::Boolean(true));
    set_slot(heap, state, THROW, Value::FALSE);
    throw
}

/// AsyncBlockStart (Abschnitt 27.10.5.2), Schritte 1.e bis 1.g, und
/// EvaluateAsyncFunctionBody, Schritt 3: Call von [[Resolve]] mit `value`,
/// mit `reject` von [[Reject]]. Den Rahmen braucht danach niemand mehr. Das
/// Ergebnis ist [[Promise]].
pub(crate) fn settle<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
    value: Local<'_>,
    reject: bool,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let target = heap.get(state);
    let function = heap.local(
        scope,
        slot(heap, target, if reject { REJECT } else { RESOLVE }),
    );
    let promise = heap.local(scope, slot(heap, target, PROMISE));
    set_slot(heap, target, FRAME, Value::UNDEFINED);
    set_slot(heap, target, OFFSET, Value::int32(RUNNING));
    let undefined = heap.local(scope, Value::UNDEFINED);
    call(agent, scope, function, undefined, &[value])?;
    Ok(promise)
}

/// *onFulfilled* aus Await.
fn await_fulfilled<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume(agent, scope, arguments, Resumption::Next)
}

/// *onRejected* aus Await.
fn await_rejected<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume(agent, scope, arguments, Resumption::Throw)
}

/// RunSuspendedContext mit dem ersten Argument als Wert oder Grund (Await,
/// Schritte 3.a und 5.a). Was der Rahmen danach tut, kommt nicht zurück; das
/// Ergebnis ist *undefined*.
fn resume<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    resumption: Resumption,
) -> Completion<Local<'s>> {
    let state = arguments.captures(agent, scope);
    let value = arguments.argument(agent, scope, 0);
    resume_async(agent, scope, state, resumption, value)?;
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

#[cfg(test)]
mod tests;
