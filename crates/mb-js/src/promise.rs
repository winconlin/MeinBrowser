//! Promises (Abschnitt 27.5): die abstrakten Operationen aus Abschnitt
//! 27.5.1 und die Jobs aus Abschnitt 27.5.2. Den Konstruktor %Promise% und
//! seine Methoden legt `builtins::promise` an.
//!
//! Ein Promise ist ein Objekt der Art [`ObjectClass::Promise`] mit vier
//! internen Feldern: [[PromiseState]] als Zahl, [[PromiseResult]], das
//! *undefined* bleibt, solange es aussteht, die Reaktionen und
//! [[PromiseIsHandled]]. [[PromiseFulfillReactions]] und
//! [[PromiseRejectReactions]] wachsen nur zusammen, in PerformPromiseThen um
//! je einen Eintrag, und werden zusammen geleert; darum hält eine Liste
//! beide. Ein Eintrag hat die PromiseCapability der beiden Reaktionen
//! ([[Promise]], [[Resolve]] und [[Reject]], ohne Capability *undefined*)
//! und ihre beiden Handler, *undefined* für ~empty~. Ein JobCallback Record
//! ist nur seine Funktion, denn HostMakeJobCallback und HostCallJobCallback
//! sind die Vorgaben der Spezifikation (siehe `host`).
//!
//! Listen sind interne Zellen: Platz 0 zählt die Einträge, dahinter folgen
//! sie mit fester Breite. Ist eine voll, kommt eine doppelt so große Kopie.
//!
//! Ein Job ist eine interne Zelle, deren erster Platz seine Art nennt:
//! NewPromiseReactionJob (Abschnitt 27.5.2.1) mit der Capability, dem
//! Handler, der Art der Reaktion und dem Argument, oder
//! NewPromiseResolveThenableJob (Abschnitt 27.5.2.2) mit dem Promise, dem
//! Objekt mit `then` und dieser Methode. Die Warteschlange hält der Agent
//! ([`Agent::run_jobs`]); [`run_job`] führt einen Job aus.

#![forbid(unsafe_code)]

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_closure};
use crate::compare::same_value;
use crate::function::{get_function_realm, is_constructor};
use crate::heap::{Heap, Local, ObjectClass, OutOfMemory, Scope};
use crate::host::RejectionOperation;
use crate::object::{INLINE, error_create};
use crate::operations::{call, construct, get, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Ref, Value};

/// Das interne Feld mit [[PromiseState]].
const STATE: usize = 0;
/// Das interne Feld mit [[PromiseResult]].
const RESULT: usize = 1;
/// Das interne Feld mit der Liste der Reaktionen, *undefined* ohne.
const REACTIONS: usize = 2;
/// Das interne Feld mit [[PromiseIsHandled]].
const HANDLED: usize = 3;

/// Die Werte eines Eintrags der Reaktionen: die drei Teile der Capability,
/// dann die Handler für ~fulfill~ und ~reject~.
const REACTION_ENTRY: usize = 5;
const ON_FULFILLED: usize = 3;
const ON_REJECTED: usize = 4;

/// Der Platz eines Jobs mit seiner Art.
const JOB_KIND: usize = 0;
const REACTION_JOB: i32 = 0;
const THENABLE_JOB: i32 = 1;
/// Die Plätze eines Jobs aus NewPromiseReactionJob: Art, die drei Teile der
/// Capability, Handler, [[Type]] und Argument.
const REACTION_JOB_SLOTS: usize = 7;
/// Die Plätze eines Jobs aus NewPromiseResolveThenableJob: Art, Promise,
/// Objekt und `then`.
const THENABLE_JOB_SLOTS: usize = 4;

/// Der Platz einer Liste mit der Zahl ihrer Einträge.
const COUNT: usize = 0;

/// [[PromiseState]] (Abschnitt 27.5.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Pending,
    Fulfilled,
    Rejected,
}

impl State {
    fn code(self) -> i32 {
        match self {
            State::Pending => 0,
            State::Fulfilled => 1,
            State::Rejected => 2,
        }
    }

    fn from_code(code: i32) -> Option<State> {
        Some(match code {
            0 => State::Pending,
            1 => State::Fulfilled,
            2 => State::Rejected,
            _ => return None,
        })
    }
}

/// [[Type]] einer Reaktion (Abschnitt 27.5.1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reaction {
    Fulfill,
    Reject,
}

impl Reaction {
    fn code(self) -> i32 {
        match self {
            Reaction::Fulfill => 0,
            Reaction::Reject => 1,
        }
    }

    fn from_code(code: Option<i32>) -> Reaction {
        match code {
            Some(1) => Reaction::Reject,
            _ => Reaction::Fulfill,
        }
    }
}

/// Ein PromiseCapability Record (Abschnitt 27.5.1.1).
#[derive(Clone, Copy)]
pub(crate) struct Capability<'s> {
    pub(crate) promise: Local<'s>,
    pub(crate) resolve: Local<'s>,
    pub(crate) reject: Local<'s>,
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

// Listen

fn list_count(heap: &Heap, cell: Ref<'_>) -> usize {
    heap.slot(cell, COUNT)
        .and_then(Value::as_int32)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(0)
}

/// Die Zahl der Einträge der Liste `list`, 0 ohne Liste.
pub(crate) fn list_len(heap: &Heap, list: Value<'_>) -> usize {
    match list.kind() {
        Kind::Internal(cell) => list_count(heap, cell),
        _ => 0,
    }
}

/// Die Liste `list` mit Platz für einen weiteren Eintrag aus `width`
/// Werten: dieselbe Zelle, wenn er noch passt, sonst eine Kopie mit
/// doppelt so vielen Plätzen, für mindestens zwei Einträge. Ohne Liste
/// (*undefined*) eine neue.
pub(crate) fn list_reserve<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    list: Local<'_>,
    width: usize,
) -> Result<Local<'s>, OutOfMemory> {
    let (count, capacity) = {
        let heap = agent.heap();
        match heap.get(list).kind() {
            Kind::Internal(cell) => (list_count(heap, cell), heap.slot_count(cell)),
            _ => (0, 0),
        }
    };
    let needed = count
        .checked_add(1)
        .and_then(|entries| entries.checked_mul(width))
        .and_then(|values| values.checked_add(1))
        .ok_or(OutOfMemory)?;
    if needed <= capacity {
        let heap = agent.heap();
        return Ok(heap.local(scope, heap.get(list)));
    }
    let doubled = capacity.checked_mul(2).ok_or(OutOfMemory)?;
    let minimum = width
        .checked_mul(2)
        .and_then(|values| values.checked_add(1))
        .ok_or(OutOfMemory)?;
    let cell = agent
        .heap_mut()
        .alloc_slots(scope, needed.max(doubled).max(minimum))?;
    let heap = agent.heap();
    if let Kind::Internal(target) = heap.get(cell).kind() {
        heap.set_slot(target, COUNT, Value::int32(0));
        if let Kind::Internal(source) = heap.get(list).kind() {
            for index in 0..capacity {
                let value = heap.slot(source, index).unwrap_or(Value::UNDEFINED);
                heap.set_slot(target, index, value);
            }
        }
    }
    Ok(cell)
}

/// Hängt an die Liste `list`, die [`list_reserve`] lieferte, den Eintrag
/// `entry` an und gibt seinen Index zurück.
pub(crate) fn list_push(
    heap: &Heap,
    list: Local<'_>,
    entry: &[Value<'_>],
) -> Result<usize, OutOfMemory> {
    let Kind::Internal(cell) = heap.get(list).kind() else {
        return Err(OutOfMemory);
    };
    let index = list_count(heap, cell);
    let first = index
        .checked_mul(entry.len())
        .and_then(|values| values.checked_add(1))
        .ok_or(OutOfMemory)?;
    for (offset, &value) in entry.iter().enumerate() {
        let place = first.checked_add(offset).ok_or(OutOfMemory)?;
        if !heap.set_slot(cell, place, value) {
            return Err(OutOfMemory);
        }
    }
    let count = index
        .checked_add(1)
        .and_then(|count| i32::try_from(count).ok())
        .ok_or(OutOfMemory)?;
    heap.set_slot(cell, COUNT, Value::int32(count));
    Ok(index)
}

/// Der Platz `offset` des Eintrags `index` einer Liste mit Einträgen aus
/// `width` Werten.
fn list_place(width: usize, index: usize, offset: usize) -> Option<usize> {
    index
        .checked_mul(width)?
        .checked_add(1)?
        .checked_add(offset)
}

/// Der Wert `offset` des Eintrags `index` der Liste `list`, *undefined*,
/// wenn es ihn nicht gibt.
pub(crate) fn list_value<'h>(
    heap: &'h Heap,
    list: Value<'_>,
    width: usize,
    index: usize,
    offset: usize,
) -> Value<'h> {
    let Kind::Internal(cell) = list.kind() else {
        return Value::UNDEFINED;
    };
    list_place(width, index, offset)
        .and_then(|place| heap.slot(cell, place))
        .unwrap_or(Value::UNDEFINED)
}

/// Schreibt den Wert `offset` des Eintrags `index` der Liste `list`.
pub(crate) fn list_set(
    heap: &Heap,
    list: Value<'_>,
    width: usize,
    index: usize,
    offset: usize,
    value: Value<'_>,
) {
    if let (Kind::Internal(cell), Some(place)) = (list.kind(), list_place(width, index, offset)) {
        heap.set_slot(cell, place, value);
    }
}

// Promise-Objekte

/// IsPromise (Abschnitt 27.5.1.6).
pub(crate) fn is_promise(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        Kind::Object(object) => heap.object_class(object) == Some(ObjectClass::Promise),
        _ => false,
    }
}

fn field<'h>(heap: &'h Heap, promise: Value<'_>, index: usize) -> Value<'h> {
    match promise.kind() {
        Kind::Object(object) => heap
            .promise_field(object, index)
            .unwrap_or(Value::UNDEFINED),
        _ => Value::UNDEFINED,
    }
}

fn set_field(heap: &Heap, promise: Value<'_>, index: usize, value: Value<'_>) {
    if let Kind::Object(object) = promise.kind() {
        heap.set_promise_field(object, index, value);
    }
}

fn state(heap: &Heap, promise: Value<'_>) -> Option<State> {
    State::from_code(field(heap, promise, STATE).as_int32()?)
}

fn is_handled(heap: &Heap, promise: Value<'_>) -> bool {
    matches!(field(heap, promise, HANDLED).kind(), Kind::Boolean(true))
}

/// Promise ( executor ) (Abschnitt 27.5.3.1), Schritte 3 bis 8 ohne
/// GetPrototypeFromConstructor: ein neues Promise mit dem Prototyp
/// `prototype`, ~pending~, ohne Reaktionen und nicht behandelt.
pub(crate) fn promise_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
) -> Result<Local<'s>, OutOfMemory> {
    let shape = agent.empty_shape(scope);
    let promise = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::Promise,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    let value = heap.get(promise);
    set_field(heap, value, STATE, Value::int32(State::Pending.code()));
    set_field(heap, value, HANDLED, Value::FALSE);
    Ok(promise)
}

/// HostPromiseRejectionTracker (Abschnitt 27.5.1.9).
fn track_rejection(
    agent: &mut Agent,
    scope: &Scope,
    promise: Local<'_>,
    operation: RejectionOperation,
) {
    if let Some(host) = agent.host() {
        host.promise_rejection_tracker(agent, scope, promise, operation);
    }
}

// Auflösen und Ablehnen

static RESOLVE: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: resolve_function,
};

static REJECT: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: reject_function,
};

/// CreateResolvingFunctions (Abschnitt 27.5.1.3) im Realm `realm`, dem
/// aktuellen. Beide Funktionen teilen den Record *promiseOrEmpty*, eine
/// interne Zelle mit dem Promise, in der *undefined* für ~empty~ steht.
pub(crate) fn create_resolving_functions<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    promise: Local<'_>,
) -> Completion<(Local<'s>, Local<'s>)> {
    let record = agent.heap_mut().alloc_slots(scope, 1)?;
    let heap = agent.heap();
    if let Kind::Internal(cell) = heap.get(record).kind() {
        heap.set_slot(cell, 0, heap.get(promise));
    }
    let resolve = create_builtin_closure(agent, scope, realm, &RESOLVE, record)?;
    let reject = create_builtin_closure(agent, scope, realm, &REJECT, record)?;
    Ok((resolve, reject))
}

/// Die ersten drei Schritte beider Funktionen aus
/// CreateResolvingFunctions: das Promise aus *promiseOrEmpty*, das danach
/// ~empty~ ist; `None`, wenn es das schon war.
fn take_promise<'s>(
    agent: &Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Option<Local<'s>> {
    let record = arguments.captures(agent, scope);
    let heap = agent.heap();
    let Kind::Internal(cell) = heap.get(record).kind() else {
        return None;
    };
    let promise = heap.slot(cell, 0)?;
    if promise.is_undefined() {
        return None;
    }
    let promise = heap.local(scope, promise);
    heap.set_slot(cell, 0, Value::UNDEFINED);
    Some(promise)
}

/// Die Funktion *resolve* aus CreateResolvingFunctions.
fn resolve_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if let Some(promise) = take_promise(agent, scope, arguments) {
        let resolution = arguments.argument(agent, scope, 0);
        resolve_promise(agent, scope, arguments.realm, promise, resolution)?;
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Die Funktion *reject* aus CreateResolvingFunctions.
fn reject_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if let Some(promise) = take_promise(agent, scope, arguments) {
        let reason = arguments.argument(agent, scope, 0);
        reject_promise(agent, scope, arguments.realm, promise, reason)?;
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Die Schritte der Funktion *resolve* ab der Prüfung auf das Promise
/// selbst: `promise` folgt `resolution`. Hat `resolution` eine Methode
/// `then`, ruft sie erst ein Job. `realm` ist der aktuelle Realm.
fn resolve_promise(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    promise: Local<'_>,
    resolution: Local<'_>,
) -> Completion<()> {
    let heap = agent.heap();
    if same_value(heap, heap.get(resolution), heap.get(promise)) {
        let prototype = agent.intrinsic(scope, realm, Intrinsic::TypeErrorPrototype);
        let error = error_create(agent, scope, prototype)?;
        return reject_promise(agent, scope, realm, promise, error);
    }
    if !matches!(agent.heap().get(resolution).kind(), Kind::Object(_)) {
        return fulfill_promise(agent, scope, realm, promise, resolution);
    }
    let key = agent.key_from_units(scope, &units("then"))?;
    let then = match get(agent, scope, resolution, key) {
        Ok(then) => then,
        Err(Abrupt::Throw) => {
            let reason = agent.take_thrown_value(scope, realm)?;
            return reject_promise(agent, scope, realm, promise, reason);
        }
        Err(abrupt) => return Err(abrupt),
    };
    if !is_callable(agent.heap(), agent.heap().get(then)) {
        return fulfill_promise(agent, scope, realm, promise, resolution);
    }
    enqueue_thenable_job(agent, scope, realm, promise, resolution, then)
}

/// FulfillPromise (Abschnitt 27.5.1.4). `realm` ist der aktuelle Realm.
fn fulfill_promise(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    promise: Local<'_>,
    value: Local<'_>,
) -> Completion<()> {
    settle(agent, scope, realm, promise, value, Reaction::Fulfill)
}

/// RejectPromise (Abschnitt 27.5.1.7). `realm` ist der aktuelle Realm.
fn reject_promise(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    promise: Local<'_>,
    reason: Local<'_>,
) -> Completion<()> {
    settle(agent, scope, realm, promise, reason, Reaction::Reject)
}

/// Die gemeinsamen Schritte von FulfillPromise und RejectPromise für ein
/// Promise, das aussteht.
fn settle(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    promise: Local<'_>,
    value: Local<'_>,
    reaction: Reaction,
) -> Completion<()> {
    let heap = agent.heap();
    let target = heap.get(promise);
    let reactions = heap.local(scope, field(heap, target, REACTIONS));
    let state = match reaction {
        Reaction::Fulfill => State::Fulfilled,
        Reaction::Reject => State::Rejected,
    };
    set_field(heap, target, RESULT, heap.get(value));
    set_field(heap, target, REACTIONS, Value::UNDEFINED);
    set_field(heap, target, STATE, Value::int32(state.code()));
    if reaction == Reaction::Reject && !is_handled(heap, target) {
        track_rejection(agent, scope, promise, RejectionOperation::Reject);
    }
    trigger_promise_reactions(agent, realm, reactions, reaction, value)
}

/// TriggerPromiseReactions (Abschnitt 27.5.1.8): für jeden Eintrag der
/// Liste `reactions` ein Job mit seiner Reaktion der Art `reaction`.
fn trigger_promise_reactions(
    agent: &mut Agent,
    realm: RealmId,
    reactions: Local<'_>,
    reaction: Reaction,
    argument: Local<'_>,
) -> Completion<()> {
    let count = list_len(agent.heap(), agent.heap().get(reactions));
    let slot = match reaction {
        Reaction::Fulfill => ON_FULFILLED,
        Reaction::Reject => ON_REJECTED,
    };
    for index in 0..count {
        agent.scope(|agent, inner| -> Completion<()> {
            let heap = agent.heap();
            let list = heap.get(reactions);
            let read =
                |offset| heap.local(inner, list_value(heap, list, REACTION_ENTRY, index, offset));
            let capability = [read(0), read(1), read(2)];
            let handler = read(slot);
            enqueue_reaction_job(agent, inner, realm, capability, handler, reaction, argument)
        })?;
    }
    Ok(())
}

// Capabilities, then und PromiseResolve

static EXECUTOR: Builtin = Builtin {
    name: "",
    length: 2,
    constructor: false,
    behaviour: capability_executor,
};

/// NewPromiseCapability (Abschnitt 27.5.1.5). Die Funktion *executor*
/// entsteht im Realm `realm`, dem aktuellen; sie fängt den Record
/// *resolvingFuncs* ein, eine interne Zelle mit zwei Plätzen.
pub(crate) fn new_promise_capability<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    constructor: Local<'_>,
) -> Completion<Capability<'s>> {
    if !is_constructor(agent, agent.heap().get(constructor)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let record = agent.heap_mut().alloc_slots(scope, 2)?;
    let executor = create_builtin_closure(agent, scope, realm, &EXECUTOR, record)?;
    let promise = construct(agent, scope, realm, constructor, &[executor], None)?;
    let heap = agent.heap();
    let (resolve, reject) = match heap.get(record).kind() {
        Kind::Internal(cell) => (
            heap.slot(cell, 0).unwrap_or(Value::UNDEFINED),
            heap.slot(cell, 1).unwrap_or(Value::UNDEFINED),
        ),
        _ => (Value::UNDEFINED, Value::UNDEFINED),
    };
    if !is_callable(heap, resolve) || !is_callable(heap, reject) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(Capability {
        promise,
        resolve: heap.local(scope, resolve),
        reject: heap.local(scope, reject),
    })
}

/// Die Funktion *executor* aus NewPromiseCapability.
fn capability_executor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let record = arguments.captures(agent, scope);
    let resolve = arguments.argument(agent, scope, 0);
    let reject = arguments.argument(agent, scope, 1);
    let heap = agent.heap();
    let Kind::Internal(cell) = heap.get(record).kind() else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let set = |index| {
        heap.slot(cell, index)
            .is_some_and(|value| !value.is_undefined())
    };
    if set(0) || set(1) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    heap.set_slot(cell, 0, heap.get(resolve));
    heap.set_slot(cell, 1, heap.get(reject));
    Ok(heap.local(scope, Value::UNDEFINED))
}

/// PerformPromiseThen (Abschnitt 27.5.5.4.1) für das Promise `promise`.
/// Das Ergebnis ist [[Promise]] von `capability`, ohne Capability
/// *undefined*. `realm` ist der aktuelle Realm.
pub(crate) fn perform_promise_then<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    promise: Local<'_>,
    on_fulfilled: Local<'_>,
    on_rejected: Local<'_>,
    capability: Option<Capability<'_>>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let handler = |value: Local<'_>| {
        let value = heap.get(value);
        heap.local(
            scope,
            if is_callable(heap, value) {
                value
            } else {
                Value::UNDEFINED
            },
        )
    };
    let on_fulfilled = handler(on_fulfilled);
    let on_rejected = handler(on_rejected);
    let undefined = heap.local(scope, Value::UNDEFINED);
    let parts = capability.map_or([undefined; 3], |capability| {
        [capability.promise, capability.resolve, capability.reject]
    });
    let target = heap.get(promise);
    match state(heap, target) {
        Some(State::Pending) => {
            let reactions = heap.local(scope, field(heap, target, REACTIONS));
            let list = list_reserve(agent, scope, reactions, REACTION_ENTRY)?;
            let heap = agent.heap();
            set_field(heap, heap.get(promise), REACTIONS, heap.get(list));
            let [first, second, third] = parts.map(|part| heap.get(part));
            let entry = [
                first,
                second,
                third,
                heap.get(on_fulfilled),
                heap.get(on_rejected),
            ];
            list_push(heap, list, &entry)?;
        }
        Some(State::Fulfilled) => {
            let value = heap.local(scope, field(heap, target, RESULT));
            enqueue_reaction_job(
                agent,
                scope,
                realm,
                parts,
                on_fulfilled,
                Reaction::Fulfill,
                value,
            )?;
        }
        Some(State::Rejected) => {
            let reason = heap.local(scope, field(heap, target, RESULT));
            if !is_handled(heap, target) {
                track_rejection(agent, scope, promise, RejectionOperation::Handle);
            }
            enqueue_reaction_job(
                agent,
                scope,
                realm,
                parts,
                on_rejected,
                Reaction::Reject,
                reason,
            )?;
        }
        // Kann nicht vorkommen: Die Aufrufer prüften IsPromise.
        None => return Err(agent.throw_error(ErrorKind::TypeError)),
    }
    let heap = agent.heap();
    set_field(heap, heap.get(promise), HANDLED, Value::TRUE);
    let [result, _, _] = parts;
    Ok(heap.local(scope, heap.get(result)))
}

/// PromiseResolve (Abschnitt 27.5.4.7.1). `realm` ist der aktuelle Realm.
pub(crate) fn promise_resolve<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    constructor: Local<'_>,
    resolution: Local<'_>,
) -> Completion<Local<'s>> {
    if is_promise(agent.heap(), agent.heap().get(resolution)) {
        let key = agent.key_from_units(scope, &units("constructor"))?;
        let found = get(agent, scope, resolution, key)?;
        let heap = agent.heap();
        if same_value(heap, heap.get(found), heap.get(constructor)) {
            return Ok(heap.local(scope, heap.get(resolution)));
        }
    }
    let capability = new_promise_capability(agent, scope, realm, constructor)?;
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    call(agent, scope, capability.resolve, undefined, &[resolution])?;
    Ok(capability.promise)
}

// Jobs

/// NewPromiseReactionJob (Abschnitt 27.5.2.1) und HostEnqueuePromiseJob.
/// `capability` sind [[Promise]], [[Resolve]] und [[Reject]], ohne
/// Capability alle *undefined*; `handler` ist *undefined* für ~empty~. Der
/// Realm des Jobs ist der des Handlers, ohne Handler *null*; `realm` ist
/// der aktuelle für GetFunctionRealm.
fn enqueue_reaction_job(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    capability: [Local<'_>; 3],
    handler: Local<'_>,
    reaction: Reaction,
    argument: Local<'_>,
) -> Completion<()> {
    let cell = agent.heap_mut().alloc_slots(scope, REACTION_JOB_SLOTS)?;
    let heap = agent.heap();
    let callback = heap.get(handler);
    let job_realm = (!callback.is_undefined()).then(|| get_function_realm(heap, callback, realm));
    if let Kind::Internal(job) = heap.get(cell).kind() {
        let [promise, resolve, reject] = capability;
        let values = [
            Value::int32(REACTION_JOB),
            heap.get(promise),
            heap.get(resolve),
            heap.get(reject),
            callback,
            Value::int32(reaction.code()),
            heap.get(argument),
        ];
        for (index, value) in values.into_iter().enumerate() {
            heap.set_slot(job, index, value);
        }
    }
    agent.enqueue_promise_job(cell, job_realm);
    Ok(())
}

/// NewPromiseResolveThenableJob (Abschnitt 27.5.2.2) mit HostMakeJobCallback
/// und HostEnqueuePromiseJob. Der Realm des Jobs ist der von `then`;
/// `realm` ist der aktuelle für GetFunctionRealm.
fn enqueue_thenable_job(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    promise: Local<'_>,
    thenable: Local<'_>,
    then: Local<'_>,
) -> Completion<()> {
    let cell = agent.heap_mut().alloc_slots(scope, THENABLE_JOB_SLOTS)?;
    let heap = agent.heap();
    let job_realm = get_function_realm(heap, heap.get(then), realm);
    if let Kind::Internal(job) = heap.get(cell).kind() {
        let values = [
            Value::int32(THENABLE_JOB),
            heap.get(promise),
            heap.get(thenable),
            heap.get(then),
        ];
        for (index, value) in values.into_iter().enumerate() {
            heap.set_slot(job, index, value);
        }
    }
    agent.enqueue_promise_job(cell, Some(job_realm));
    Ok(())
}

/// Führt den Job in der Zelle `cell` aus, mit seinem Realm `realm`.
pub(crate) fn run_job(
    agent: &mut Agent,
    scope: &Scope,
    cell: Local<'_>,
    realm: Option<RealmId>,
) -> Completion<()> {
    let heap = agent.heap();
    let Kind::Internal(job) = heap.get(cell).kind() else {
        return Ok(());
    };
    let read = |index| heap.local(scope, heap.slot(job, index).unwrap_or(Value::UNDEFINED));
    let kind = heap.slot(job, JOB_KIND).and_then(Value::as_int32);
    if kind == Some(THENABLE_JOB) {
        let (promise, thenable, then) = (read(1), read(2), read(3));
        return thenable_job(agent, scope, realm, promise, thenable, then);
    }
    let capability = [read(1), read(2), read(3)];
    let handler = read(4);
    let reaction = Reaction::from_code(heap.slot(job, 5).and_then(Value::as_int32));
    let argument = read(6);
    reaction_job(agent, scope, realm, capability, handler, reaction, argument)
}

/// Der Job aus NewPromiseReactionJob.
fn reaction_job(
    agent: &mut Agent,
    scope: &Scope,
    realm: Option<RealmId>,
    capability: [Local<'_>; 3],
    handler: Local<'_>,
    reaction: Reaction,
    argument: Local<'_>,
) -> Completion<()> {
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    // Ok für einen normalen Abschluss, Err mit dem geworfenen Wert.
    let result = if agent.heap().get(handler).is_undefined() {
        match reaction {
            Reaction::Fulfill => Ok(argument),
            Reaction::Reject => Err(argument),
        }
    } else {
        match call(agent, scope, handler, undefined, &[argument]) {
            Ok(value) => Ok(value),
            Err(Abrupt::Throw) => {
                // Kann nicht fehlen: Mit Handler hat der Job dessen Realm.
                let Some(realm) = realm else {
                    return Err(Abrupt::Throw);
                };
                Err(agent.take_thrown_value(scope, realm)?)
            }
            Err(abrupt) => return Err(abrupt),
        }
    };
    let [promise, resolve, reject] = capability;
    if agent.heap().get(promise).is_undefined() {
        // Ohne Capability ist der Abschluss nach der Spezifikation nie
        // abrupt; sonst bekommt ihn der Host.
        return match result {
            Ok(_) => Ok(()),
            Err(value) => Err(agent.throw_value(value)),
        };
    }
    let (function, value) = match result {
        Ok(value) => (resolve, value),
        Err(value) => (reject, value),
    };
    call(agent, scope, function, undefined, &[value])?;
    Ok(())
}

/// Der Job aus NewPromiseResolveThenableJob.
fn thenable_job(
    agent: &mut Agent,
    scope: &Scope,
    realm: Option<RealmId>,
    promise: Local<'_>,
    thenable: Local<'_>,
    then: Local<'_>,
) -> Completion<()> {
    // Kann nicht fehlen: Der Job hat den Realm von `then`.
    let Some(realm) = realm else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let (resolve, reject) = create_resolving_functions(agent, scope, realm, promise)?;
    match call(agent, scope, then, thenable, &[resolve, reject]) {
        Ok(_) => Ok(()),
        Err(Abrupt::Throw) => {
            let reason = agent.take_thrown_value(scope, realm)?;
            let undefined = agent.heap().local(scope, Value::UNDEFINED);
            call(agent, scope, reject, undefined, &[reason])?;
            Ok(())
        }
        Err(abrupt) => Err(abrupt),
    }
}

#[cfg(test)]
mod tests;
