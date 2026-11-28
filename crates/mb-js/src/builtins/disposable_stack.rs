//! %DisposableStack% und %AsyncDisposableStack% mit ihren Prototypen, aus
//! dem Teil der Spezifikation über Explicit Resource Management. Seine
//! Abschnitte haben noch keine festen Nummern; die Verweise nennen darum
//! die Anker, etwa sec-disposablestack.prototype.adopt.
//!
//! Beide Arten sind Objekte der Klasse
//! [`ObjectClass::DisposableStack`]. Ein Feld sagt, ob es ein
//! AsyncDisposableStack ist; [[DisposableState]] und
//! [[AsyncDisposableState]] sind damit zwei verschiedene interne Felder,
//! wie RequireInternalSlot es verlangt. [[DisposeCapability]] ist eine
//! Zelle mit drei Plätzen je Ressource und die Zahl der Ressourcen. Die
//! Plätze halten [[ResourceValue]], [[DisposeMethod]] (*undefined* für eine
//! Ressource aus *null* oder *undefined* bei ~async-dispose~) und ob sie
//! aus `adopt` kommt. Für `adopt` legt die Spezifikation eine Closure an,
//! die `onDispose` mit dem Wert aufruft und nie nach außen gelangt; hier
//! steht stattdessen `onDispose` mit dem Wert in der Ressource, und
//! DisposeResources ruft es mit dem Wert als Argument auf. Die Art der
//! Ressourcen ([[Hint]]) folgt aus der Art des Stapels. Ist die Zelle voll,
//! ziehen die Ressourcen in eine doppelt so große um.
//!
//! DisposeResources läuft für `dispose` als Schleife in Rust. Für
//! `disposeAsync` wartet es auf jedes Ergebnis mit Await; wie bei
//! `Array.fromAsync` hält dann eine interne Zelle den Zustand, und jedes
//! Await ist ein PerformPromiseThen mit zwei Closures, die die Schleife
//! wieder aufnehmen. Jede Ressource kostet beim Freigeben einen
//! Arbeitsschritt.

use super::promise::reject_abrupt;
use super::{
    define_functions, define_getter, define_method, define_symbol_property, define_to_string_tag,
    link_constructor,
};
use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::async_function::perform_then_with;
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::builtins::{create_disposable_resource, suppressed_error_create};
use crate::heap::{Local, ObjectClass, OutOfMemory, Scope};
use crate::key::WellKnownSymbol;
use crate::object::{INLINE, get_prototype_from_constructor, ordinary_object_create};
use crate::operations::{call, is_callable};
use crate::promise::{new_promise_capability, promise_resolve};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Die internen Felder eines Stapels: ob er async ist, ob er ~disposed~
/// ist, die Zelle mit den Ressourcen oder *undefined* und deren Zahl.
const ASYNC: usize = 0;
const DISPOSED: usize = 1;
const RESOURCES: usize = 2;
const COUNT: usize = 3;

/// So viele Plätze belegt eine Ressource: Wert, Methode und ob sie aus
/// `adopt` kommt.
const STRIDE: usize = 3;
/// So viele Ressourcen fasst die Zelle eines Stapels mindestens.
const MIN_CAPACITY: usize = 4;

/// Die Felder des Zustands von DisposeResources für `disposeAsync`:
/// [[Resolve]] und [[Reject]] der Capability, die Zelle mit den Ressourcen,
/// wie viele davon noch übrig sind, *needsAwait*, *hasAwaited*, ob die
/// Completion ein *throw* ist und ihr Wert.
const RESOLVE: usize = 0;
const REJECT: usize = 1;
const STATE_RESOURCES: usize = 2;
const LEFT: usize = 3;
const NEEDS_AWAIT: usize = 4;
const HAS_AWAITED: usize = 5;
const THROWING: usize = 6;
const ERROR: usize = 7;
const STATE_FIELDS: usize = 8;

static DISPOSABLE_STACK: Builtin = Builtin {
    name: "DisposableStack",
    length: 0,
    constructor: true,
    behaviour: disposable_stack,
};

static ASYNC_DISPOSABLE_STACK: Builtin = Builtin {
    name: "AsyncDisposableStack",
    length: 0,
    constructor: true,
    behaviour: async_disposable_stack,
};

/// Die Methoden von %DisposableStack.prototype% vor und nach `dispose` und
/// `disposed`, in der Reihenfolge der Spezifikation.
static FUNCTIONS: [Builtin; 2] = [method("adopt", 2, adopt), method("defer", 1, defer)];
static MORE_FUNCTIONS: [Builtin; 2] = [method("move", 0, move_stack), method("use", 1, use_value)];
static DISPOSE: Builtin = method("dispose", 0, dispose);
static DISPOSED_GETTER: Builtin = method("get disposed", 0, disposed);

/// Dasselbe für %AsyncDisposableStack.prototype%.
static ASYNC_FUNCTIONS: [Builtin; 2] = [
    method("adopt", 2, async_adopt),
    method("defer", 1, async_defer),
];
static ASYNC_MORE_FUNCTIONS: [Builtin; 2] = [
    method("move", 0, async_move_stack),
    method("use", 1, async_use_value),
];
static DISPOSE_ASYNC: Builtin = method("disposeAsync", 0, dispose_async);
static ASYNC_DISPOSED_GETTER: Builtin = method("get disposed", 0, async_disposed);

/// Die Closures von Await in `disposeAsync`.
static ON_FULFILLED: Builtin = method("", 1, on_fulfilled);
static ON_REJECTED: Builtin = method("", 1, on_rejected);

const fn method(name: &'static str, length: u32, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

/// Was die beiden Arten von Stapeln unterscheidet, für [`create`].
struct Parts {
    constructor: &'static Builtin,
    functions: &'static [Builtin],
    dispose: &'static Builtin,
    disposed: &'static Builtin,
    more: &'static [Builtin],
    symbol: WellKnownSymbol,
    tag: &'static str,
    intrinsics: (Intrinsic, Intrinsic),
}

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let kinds = [
        Parts {
            constructor: &DISPOSABLE_STACK,
            functions: &FUNCTIONS,
            dispose: &DISPOSE,
            disposed: &DISPOSED_GETTER,
            more: &MORE_FUNCTIONS,
            symbol: WellKnownSymbol::Dispose,
            tag: "DisposableStack",
            intrinsics: (
                Intrinsic::DisposableStack,
                Intrinsic::DisposableStackPrototype,
            ),
        },
        Parts {
            constructor: &ASYNC_DISPOSABLE_STACK,
            functions: &ASYNC_FUNCTIONS,
            dispose: &DISPOSE_ASYNC,
            disposed: &ASYNC_DISPOSED_GETTER,
            more: &ASYNC_MORE_FUNCTIONS,
            symbol: WellKnownSymbol::AsyncDispose,
            tag: "AsyncDisposableStack",
            intrinsics: (
                Intrinsic::AsyncDisposableStack,
                Intrinsic::AsyncDisposableStackPrototype,
            ),
        },
    ];
    for parts in kinds {
        let prototype = ordinary_object_create(agent, scope, Some(object))?;
        let constructor = create_builtin_function(agent, scope, realm, parts.constructor)?;
        link_constructor(agent, scope, constructor, prototype)?;
        define_functions(agent, scope, realm, prototype, parts.functions)?;
        let dispose = define_method(agent, scope, realm, prototype, parts.dispose)?;
        define_getter(agent, scope, realm, prototype, "disposed", parts.disposed)?;
        define_functions(agent, scope, realm, prototype, parts.more)?;
        // @@dispose und @@asyncDispose sind dieselben Funktionen wie
        // `dispose` und `disposeAsync`.
        define_symbol_property(agent, scope, prototype, parts.symbol, dispose)?;
        define_to_string_tag(agent, scope, prototype, parts.tag)?;
        agent.set_intrinsic(realm, parts.intrinsics.0, constructor);
        agent.set_intrinsic(realm, parts.intrinsics.1, prototype);
    }
    Ok(())
}

// Der Stapel und seine Felder

/// Ein neuer Stapel mit dem Prototyp `prototype`: ~pending~ und mit einer
/// leeren [[DisposeCapability]] (NewDisposeCapability).
fn stack_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
    asynchronous: bool,
) -> Completion<Local<'s>> {
    let shape = agent.empty_shape(scope);
    let stack = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::DisposableStack,
        INLINE,
        shape,
        Some(prototype),
    )?;
    write(agent, stack, ASYNC, Value::boolean(asynchronous));
    write(agent, stack, DISPOSED, Value::FALSE);
    write(agent, stack, COUNT, Value::int32(0));
    Ok(stack)
}

/// Ob `value` ein Stapel der Art `asynchronous` ist, für
/// RequireInternalSlot mit [[DisposableState]] oder
/// [[AsyncDisposableState]].
fn is_stack(agent: &Agent, value: Local<'_>, asynchronous: bool) -> bool {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::Object(object) => heap
            .disposable_stack_field(object, ASYNC)
            .is_some_and(|found| found.kind() == Kind::Boolean(asynchronous)),
        _ => false,
    }
}

/// RequireInternalSlot und danach die Prüfung, dass der Stapel nicht
/// ~disposed~ ist, wie zu Beginn von `adopt`, `defer`, `move` und `use`.
fn require_pending(agent: &mut Agent, stack: Local<'_>, asynchronous: bool) -> Completion<()> {
    if !is_stack(agent, stack, asynchronous) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if is_disposed(agent, stack) {
        return Err(agent.throw_error(ErrorKind::ReferenceError));
    }
    Ok(())
}

fn is_disposed(agent: &Agent, stack: Local<'_>) -> bool {
    field(agent, stack, DISPOSED).kind() == Kind::Boolean(true)
}

fn field<'h>(agent: &'h Agent, stack: Local<'_>, index: usize) -> Value<'h> {
    let heap = agent.heap();
    match heap.get(stack).kind() {
        Kind::Object(object) => heap
            .disposable_stack_field(object, index)
            .unwrap_or(Value::UNDEFINED),
        _ => Value::UNDEFINED,
    }
}

fn write(agent: &Agent, stack: Local<'_>, index: usize, value: Value<'_>) {
    let heap = agent.heap();
    if let Kind::Object(object) = heap.get(stack).kind() {
        heap.set_disposable_stack_field(object, index, value);
    }
}

/// Die Zahl der Ressourcen des Stapels.
fn count(agent: &Agent, stack: Local<'_>) -> usize {
    field(agent, stack, COUNT)
        .as_int32()
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(0)
}

/// Hängt eine Ressource an [[DisposableResourceStack]] an, für
/// AddDisposableResource.
fn push_resource(
    agent: &mut Agent,
    scope: &Scope,
    stack: Local<'_>,
    value: Local<'_>,
    method: Local<'_>,
    adopted: bool,
) -> Completion<()> {
    let count = count(agent, stack);
    let next = count.checked_add(1).ok_or(OutOfMemory)?;
    let counted = i32::try_from(next).map_err(|_| OutOfMemory)?;
    let resources = agent.heap().local(scope, field(agent, stack, RESOURCES));
    let capacity = match agent.heap().get(resources).kind() {
        Kind::Internal(cell) => agent.heap().slot_count(cell) / STRIDE,
        _ => 0,
    };
    let resources = if next > capacity {
        let records = capacity.saturating_mul(2).max(MIN_CAPACITY);
        let slots = records.checked_mul(STRIDE).ok_or(OutOfMemory)?;
        let grown = agent.heap_mut().alloc_slots(scope, slots)?;
        let heap = agent.heap();
        if let (Kind::Internal(old), Kind::Internal(new)) =
            (heap.get(resources).kind(), heap.get(grown).kind())
        {
            for index in 0..count.saturating_mul(STRIDE) {
                heap.set_slot(
                    new,
                    index,
                    heap.slot(old, index).unwrap_or(Value::UNDEFINED),
                );
            }
        }
        write(agent, stack, RESOURCES, heap.get(grown));
        grown
    } else {
        resources
    };
    let heap = agent.heap();
    if let Kind::Internal(cell) = heap.get(resources).kind() {
        let start = count.saturating_mul(STRIDE);
        heap.set_slot(cell, start, heap.get(value));
        heap.set_slot(cell, start + 1, heap.get(method));
        heap.set_slot(cell, start + 2, Value::boolean(adopted));
    }
    write(agent, stack, COUNT, Value::int32(counted));
    Ok(())
}

/// Nimmt dem Stapel seine Ressourcen und lässt ihm eine leere
/// [[DisposeCapability]]: die Zelle oder *undefined* und die Zahl.
fn take_resources<'s>(agent: &Agent, scope: &'s Scope, stack: Local<'_>) -> (Local<'s>, usize) {
    let resources = agent.heap().local(scope, field(agent, stack, RESOURCES));
    let count = count(agent, stack);
    write(agent, stack, RESOURCES, Value::UNDEFINED);
    write(agent, stack, COUNT, Value::int32(0));
    (resources, count)
}

/// Die Ressource `index` aus `resources`: Wert, Methode und ob sie aus
/// `adopt` kommt.
fn resource<'s>(
    agent: &Agent,
    scope: &'s Scope,
    resources: Local<'_>,
    index: usize,
) -> (Local<'s>, Local<'s>, bool) {
    let heap = agent.heap();
    let slot = |offset: usize| match heap.get(resources).kind() {
        Kind::Internal(cell) => heap
            .slot(cell, index.saturating_mul(STRIDE).saturating_add(offset))
            .unwrap_or(Value::UNDEFINED),
        _ => Value::UNDEFINED,
    };
    let adopted = slot(2).kind() == Kind::Boolean(true);
    (
        heap.local(scope, slot(0)),
        heap.local(scope, slot(1)),
        adopted,
    )
}

/// Der Aufruf, mit dem DisposeResources eine Ressource freigibt: Call der
/// Methode mit dem Wert als *this*, bei `adopt` die Closure, die
/// `onDispose` mit *undefined* als *this* und dem Wert aufruft.
fn call_dispose<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
    method: Local<'_>,
    adopted: bool,
) -> Completion<Local<'s>> {
    if adopted {
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        call(agent, scope, method, undefined, &[value])
    } else {
        call(agent, scope, method, value, &[])
    }
}

/// Fügt `thrown` zur Completion von DisposeResources hinzu, die in
/// `error` steht, wenn sie schon ein *throw* ist (Schritt 3.e.iii): sonst
/// wird es die Completion, sonst ein neuer SuppressedError mit dem alten
/// Wert als `suppressed`.
fn suppress<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    error: Option<Local<'_>>,
    thrown: Local<'s>,
) -> Completion<Local<'s>> {
    match error {
        Some(previous) => suppressed_error_create(agent, scope, realm, thrown, previous),
        None => Ok(thrown),
    }
}

// Die Konstruktoren

/// DisposableStack ( ) (Anker sec-disposablestack).
fn disposable_stack<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    construct(agent, scope, arguments, false)
}

/// AsyncDisposableStack ( ) (Anker sec-asyncdisposablestack).
fn async_disposable_stack<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    construct(agent, scope, arguments, true)
}

/// Beide Konstruktoren: TypeError ohne NewTarget, sonst
/// OrdinaryCreateFromConstructor.
fn construct<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    asynchronous: bool,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let fallback = if asynchronous {
        Intrinsic::AsyncDisposableStackPrototype
    } else {
        Intrinsic::DisposableStackPrototype
    };
    let prototype =
        get_prototype_from_constructor(agent, scope, new_target, fallback, arguments.realm)?;
    stack_create(agent, scope, prototype, asynchronous)
}

// adopt, defer, move und use

fn adopt<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    adopt_value(agent, scope, arguments, false)
}

fn async_adopt<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    adopt_value(agent, scope, arguments, true)
}

/// DisposableStack.prototype.adopt ( value, onDispose ) und
/// AsyncDisposableStack.prototype.adopt ( value, onDisposeAsync ).
fn adopt_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    asynchronous: bool,
) -> Completion<Local<'s>> {
    let stack = arguments.this;
    require_pending(agent, stack, asynchronous)?;
    let value = arguments.argument(agent, scope, 0);
    let on_dispose = arguments.argument(agent, scope, 1);
    if !is_callable(agent.heap(), agent.heap().get(on_dispose)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    push_resource(agent, scope, stack, value, on_dispose, true)?;
    Ok(value)
}

fn defer<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    defer_callback(agent, scope, arguments, false)
}

fn async_defer<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    defer_callback(agent, scope, arguments, true)
}

/// DisposableStack.prototype.defer ( onDispose ) und
/// AsyncDisposableStack.prototype.defer ( onDisposeAsync ): eine Ressource
/// mit *undefined* als Wert.
fn defer_callback<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    asynchronous: bool,
) -> Completion<Local<'s>> {
    let stack = arguments.this;
    require_pending(agent, stack, asynchronous)?;
    let on_dispose = arguments.argument(agent, scope, 0);
    if !is_callable(agent.heap(), agent.heap().get(on_dispose)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    push_resource(agent, scope, stack, undefined, on_dispose, false)?;
    Ok(undefined)
}

fn move_stack<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    move_resources(agent, scope, arguments, false)
}

fn async_move_stack<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    move_resources(agent, scope, arguments, true)
}

/// DisposableStack.prototype.move ( ) und
/// AsyncDisposableStack.prototype.move ( ): Der neue Stapel kommt immer
/// aus %DisposableStack% beziehungsweise %AsyncDisposableStack%, auch bei
/// Unterklassen, und übernimmt die [[DisposeCapability]].
fn move_resources<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    asynchronous: bool,
) -> Completion<Local<'s>> {
    let stack = arguments.this;
    require_pending(agent, stack, asynchronous)?;
    let prototype = if asynchronous {
        Intrinsic::AsyncDisposableStackPrototype
    } else {
        Intrinsic::DisposableStackPrototype
    };
    let prototype = agent.intrinsic(scope, arguments.realm, prototype);
    let moved = stack_create(agent, scope, prototype, asynchronous)?;
    let (resources, count) = take_resources(agent, scope, stack);
    write(agent, moved, RESOURCES, agent.heap().get(resources));
    write(
        agent,
        moved,
        COUNT,
        Value::int32(i32::try_from(count).unwrap_or(0)),
    );
    write(agent, stack, DISPOSED, Value::TRUE);
    Ok(moved)
}

fn use_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    use_resource(agent, scope, arguments, false)
}

fn async_use_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    use_resource(agent, scope, arguments, true)
}

/// DisposableStack.prototype.use ( value ) und
/// AsyncDisposableStack.prototype.use ( value ): AddDisposableResource
/// ohne Methode, wie bei `using` und `await using`.
fn use_resource<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    asynchronous: bool,
) -> Completion<Local<'s>> {
    let stack = arguments.this;
    require_pending(agent, stack, asynchronous)?;
    let value = arguments.argument(agent, scope, 0);
    if let Some((method, resource)) =
        create_disposable_resource(agent, scope, arguments.realm, value, asynchronous)?
    {
        // *null* steht bei `await using` für eine Ressource ohne Methode.
        if agent.heap().get(method).is_null() {
            agent.heap().set(method, Value::UNDEFINED);
        }
        push_resource(agent, scope, stack, resource, method, false)?;
    }
    Ok(value)
}

// disposed

fn disposed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    disposed_state(agent, scope, arguments, false)
}

fn async_disposed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    disposed_state(agent, scope, arguments, true)
}

/// get DisposableStack.prototype.disposed und
/// get AsyncDisposableStack.prototype.disposed.
fn disposed_state<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    asynchronous: bool,
) -> Completion<Local<'s>> {
    if !is_stack(agent, arguments.this, asynchronous) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let disposed = is_disposed(agent, arguments.this);
    Ok(agent.heap().local(scope, Value::boolean(disposed)))
}

// dispose

/// DisposableStack.prototype.dispose ( ), zugleich @@dispose, mit
/// DisposeResources für Ressourcen mit ~sync-dispose~.
fn dispose<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let stack = arguments.this;
    if !is_stack(agent, stack, false) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    if is_disposed(agent, stack) {
        return Ok(undefined);
    }
    write(agent, stack, DISPOSED, Value::TRUE);
    // Schritt 6 von DisposeResources leert die Liste erst am Ende; der
    // Stapel ist aber schon ~disposed~, nichts sieht sie vorher wieder.
    let (resources, count) = take_resources(agent, scope, stack);
    // Die Completion: *undefined* oder der geworfene Wert.
    let error = agent.heap().local(scope, Value::UNDEFINED);
    let mut throwing = false;
    for index in (0..count).rev() {
        agent.spend_execution_steps(1)?;
        throwing = agent.scope(|agent, inner| -> Completion<bool> {
            let (value, method, adopted) = resource(agent, inner, resources, index);
            match call_dispose(agent, inner, value, method, adopted) {
                Ok(_) => Ok(throwing),
                Err(Abrupt::Throw) => {
                    let thrown = agent.take_thrown_value(inner, realm)?;
                    let previous = throwing.then_some(error);
                    let combined = suppress(agent, inner, realm, previous, thrown)?;
                    agent.heap().set(error, agent.heap().get(combined));
                    Ok(true)
                }
                Err(abrupt) => Err(abrupt),
            }
        })?;
    }
    if throwing {
        return Err(agent.throw_value(error));
    }
    Ok(undefined)
}

// disposeAsync

/// AsyncDisposableStack.prototype.disposeAsync ( ), zugleich
/// @@asyncDispose: ein Promise, das DisposeResources für Ressourcen mit
/// ~async-dispose~ erfüllt oder ablehnt. Was ohne Await geht, läuft noch
/// in diesem Aufruf; ohne Ressourcen ist das Promise darum schon erfüllt.
fn dispose_async<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let stack = arguments.this;
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let capability = new_promise_capability(agent, scope, realm, constructor)?;
    if !is_stack(agent, stack, true) {
        let abrupt = agent.throw_error(ErrorKind::TypeError);
        reject_abrupt(agent, scope, realm, abrupt, capability.reject)?;
        return Ok(capability.promise);
    }
    if is_disposed(agent, stack) {
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        call(agent, scope, capability.resolve, undefined, &[undefined])?;
        return Ok(capability.promise);
    }
    write(agent, stack, DISPOSED, Value::TRUE);
    let (resources, count) = take_resources(agent, scope, stack);
    let state = agent.heap_mut().alloc_slots(scope, STATE_FIELDS)?;
    let heap = agent.heap();
    write_state(agent, state, RESOLVE, heap.get(capability.resolve));
    write_state(agent, state, REJECT, heap.get(capability.reject));
    write_state(agent, state, STATE_RESOURCES, heap.get(resources));
    write_state(
        agent,
        state,
        LEFT,
        Value::int32(i32::try_from(count).unwrap_or(0)),
    );
    for flag in [NEEDS_AWAIT, HAS_AWAITED, THROWING] {
        write_state(agent, state, flag, Value::FALSE);
    }
    if let Err(abrupt) = continue_disposal(agent, scope, realm, state) {
        reject_abrupt(agent, scope, realm, abrupt, capability.reject)?;
    }
    Ok(capability.promise)
}

/// Die Schleife von DisposeResources ab der nächsten übrigen Ressource,
/// bis zum nächsten Await oder zum Ende, das das Promise erfüllt oder
/// ablehnt. Ressourcen aus `disposeAsync` haben alle ~async-dispose~;
/// Schritt 3.d, der vor einer mit ~sync-dispose~ wartet, kommt hier nie
/// vor.
fn continue_disposal(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
) -> Completion<()> {
    loop {
        let left = state_field(agent, state, LEFT)
            .as_int32()
            .and_then(|left| usize::try_from(left).ok())
            .unwrap_or(0);
        let Some(index) = left.checked_sub(1) else {
            break;
        };
        agent.spend_execution_steps(1)?;
        write_state(
            agent,
            state,
            LEFT,
            Value::int32(i32::try_from(index).unwrap_or(0)),
        );
        let suspended = agent.scope(|agent, inner| -> Completion<bool> {
            let resources = agent
                .heap()
                .local(inner, state_field(agent, state, STATE_RESOURCES));
            let (value, method, adopted) = resource(agent, inner, resources, index);
            if agent.heap().get(method).is_undefined() {
                write_state(agent, state, NEEDS_AWAIT, Value::TRUE);
                return Ok(false);
            }
            let result = match call_dispose(agent, inner, value, method, adopted) {
                // Schritt 3.e.ii: Await, danach gilt *hasAwaited*, auch
                // wenn schon PromiseResolve wirft.
                Ok(result) => {
                    write_state(agent, state, HAS_AWAITED, Value::TRUE);
                    await_value(agent, inner, realm, state, result)
                }
                Err(abrupt) => Err(abrupt),
            };
            match result {
                Ok(()) => Ok(true),
                Err(Abrupt::Throw) => {
                    let thrown = agent.take_thrown_value(inner, realm)?;
                    record_error(agent, inner, realm, state, thrown)?;
                    Ok(false)
                }
                Err(abrupt) => Err(abrupt),
            }
        })?;
        if suspended {
            return Ok(());
        }
    }
    // Schritt 4: ein Await für Ressourcen aus *null* oder *undefined*,
    // wenn sonst keins war.
    if is_true(agent, state, NEEDS_AWAIT) && !is_true(agent, state, HAS_AWAITED) {
        write_state(agent, state, HAS_AWAITED, Value::TRUE);
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        return await_value(agent, scope, realm, state, undefined);
    }
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    if is_true(agent, state, THROWING) {
        let reject = state_local(agent, scope, state, REJECT);
        let error = state_local(agent, scope, state, ERROR);
        call(agent, scope, reject, undefined, &[error])?;
    } else {
        let resolve = state_local(agent, scope, state, RESOLVE);
        call(agent, scope, resolve, undefined, &[undefined])?;
    }
    Ok(())
}

/// Await (Abschnitt 27.7.5.3) auf `value`; danach geht die Schleife weiter.
fn await_value(
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
        &ON_FULFILLED,
        &ON_REJECTED,
    )
}

/// Schritt 3.e.iii mit dem geworfenen Wert `thrown`.
fn record_error(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    state: Local<'_>,
    thrown: Local<'_>,
) -> Completion<()> {
    let thrown = agent.heap().local(scope, agent.heap().get(thrown));
    let previous = is_true(agent, state, THROWING).then(|| state_local(agent, scope, state, ERROR));
    let error = suppress(agent, scope, realm, previous, thrown)?;
    write_state(agent, state, ERROR, agent.heap().get(error));
    write_state(agent, state, THROWING, Value::TRUE);
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

/// Nimmt die Schleife nach einem Await wieder auf, nach einer Ablehnung
/// mit ihrem Grund als geworfenem Wert. Eine Ausnahme der Fortsetzung
/// lehnt das Promise ab.
fn resume<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    rejected: bool,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let state = arguments.captures(agent, scope);
    let mut result = Ok(());
    if rejected {
        let reason = arguments.argument(agent, scope, 0);
        result = record_error(agent, scope, realm, state, reason);
    }
    if result.is_ok() {
        result = continue_disposal(agent, scope, realm, state);
    }
    if let Err(abrupt) = result {
        let reject = state_local(agent, scope, state, REJECT);
        reject_abrupt(agent, scope, realm, abrupt, reject)?;
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

fn state_field<'h>(agent: &'h Agent, state: Local<'_>, index: usize) -> Value<'h> {
    let heap = agent.heap();
    match heap.get(state).kind() {
        Kind::Internal(cell) => heap.slot(cell, index).unwrap_or(Value::UNDEFINED),
        _ => Value::UNDEFINED,
    }
}

fn state_local<'s>(agent: &Agent, scope: &'s Scope, state: Local<'_>, index: usize) -> Local<'s> {
    agent.heap().local(scope, state_field(agent, state, index))
}

fn write_state(agent: &Agent, state: Local<'_>, index: usize, value: Value<'_>) {
    let heap = agent.heap();
    if let Kind::Internal(cell) = heap.get(state).kind() {
        heap.set_slot(cell, index, value);
    }
}

fn is_true(agent: &Agent, state: Local<'_>, index: usize) -> bool {
    state_field(agent, state, index).kind() == Kind::Boolean(true)
}
