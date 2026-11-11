//! async-Generatoren (Abschnitt 27.9): die Objekte, die der Aufruf einer
//! async-Generatorfunktion zurückgibt, ihre Warteschlange und die
//! abstrakten Operationen aus Abschnitt 27.9.3 für `next`, `return` und
//! `throw` und für die Befehle im Rumpf.
//!
//! Ein async-Generator ist ein Objekt der Art
//! [`ObjectClass::AsyncGenerator`] mit acht internen Feldern:
//! [[AsyncGeneratorState]] als Zahl; das Funktionsobjekt, aus dem Code und
//! Realm seines Rahmens kommen; der angehaltene Rahmen
//! ([[AsyncGeneratorContext]]) als interne Zelle mit dem Kopf und den
//! Registern, *undefined*, sobald der Rumpf fertig ist; der Befehl, bei dem
//! der Rahmen weitergeht, [`RUNNING`], solange er läuft; wie er zuletzt
//! fortgesetzt wurde, bis der Befehl danach es liest; Anfang und Ende von
//! [[AsyncGeneratorQueue]]; und der Realm dessen, der den Rahmen zuletzt
//! fortsetzte, für AsyncGeneratorYield. [[GeneratorBrand]] ist immer
//! ~empty~.
//!
//! Die Warteschlange ist eine Kette interner Zellen, eine für jeden
//! AsyncGeneratorRequest Record (Abschnitt 27.9.3.1): die Art des
//! Abschlusses, sein Wert, [[Resolve]] und [[Reject]] der
//! PromiseCapability und die nächste Zelle. [[Promise]] gibt die Methode
//! gleich zurück; danach braucht es niemand mehr.
//!
//! Wie bei Generatoren läuft der Rumpf erst, wenn ihn `next` fortsetzt,
//! und dann in einem neuen Lauf, dessen unterster Rahmen er ist
//! ([`resume_async_generator`]). Er hält an zwei Stellen an: bei `await`,
//! dann bleibt der Generator ~executing~, und eine der beiden Funktionen,
//! die Await an das Promise hängt, setzt ihn in einem Job fort; und bei
//! `yield`, wenn die Warteschlange danach leer ist (~suspended-yield~).
//! Kommt für ein `yield` ein Abschluss mit `return`, wartet der Generator
//! zuerst auf dessen Wert (AsyncGeneratorUnwrapYieldResumption, Abschnitt
//! 27.9.3.7), und der Rahmen geht erst danach weiter, nach einer Ablehnung
//! wie mit `throw`. Der Befehl hinter `yield` sieht also nur Abschlüsse,
//! auf die schon gewartet wurde. Endet der Rumpf, gilt der erste Eintrag
//! der Warteschlange, dann die übrigen (AsyncGeneratorStart, Schritt 4).

#![forbid(unsafe_code)]

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::async_function::perform_then_with;
use crate::builtin::{Builtin, BuiltinCall};
use crate::function::function_parts;
use crate::generator::Resumption;
use crate::heap::{Heap, Local, ObjectClass, Scope};
use crate::interpreter::{Ending, resume_async_generator};
use crate::iterator::create_iterator_result_object;
use crate::object::{INLINE, get_prototype_from_constructor};
use crate::operations::call;
use crate::promise::{Capability, promise_resolve};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Das interne Feld mit [[AsyncGeneratorState]].
const STATE: usize = 0;
/// Das interne Feld mit dem Funktionsobjekt.
const FUNCTION: usize = 1;
/// Das interne Feld mit dem Rahmen, *undefined*, wenn der Rumpf fertig ist.
const FRAME: usize = 2;
/// Das interne Feld mit dem Befehl, bei dem der Rahmen weitergeht.
const OFFSET: usize = 3;
/// Das interne Feld mit der Art der letzten Fortsetzung.
const MODE: usize = 4;
/// Die internen Felder mit dem ersten und dem letzten Eintrag der
/// Warteschlange, *undefined*, wenn sie leer ist.
const HEAD: usize = 5;
const TAIL: usize = 6;
/// Das interne Feld mit dem Realm dessen, der den Rahmen zuletzt
/// fortsetzte.
const CALLER: usize = 7;

/// Im Feld [`OFFSET`]: Der Rahmen läuft oder ist fertig.
const RUNNING: i32 = -1;

/// Die Plätze eines Eintrags der Warteschlange.
const REQUEST_KIND: usize = 0;
const REQUEST_VALUE: usize = 1;
const REQUEST_RESOLVE: usize = 2;
const REQUEST_REJECT: usize = 3;
const REQUEST_NEXT: usize = 4;
const REQUEST_SLOTS: usize = 5;

/// Await im Rumpf, Schritte 3 und 5: Die Funktionen setzen den Rahmen mit
/// dem Wert oder mit dem Grund fort.
static RESUME_NEXT: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: resume_next,
};
static RESUME_THROW: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: resume_throw,
};
/// Das Await aus AsyncGeneratorUnwrapYieldResumption: Mit dem Wert geht
/// der Rahmen mit `return` weiter.
static RESUME_RETURN: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: resume_return,
};

/// AsyncGeneratorAwaitReturn (Abschnitt 27.9.3.9), Schritte 11 und 13: Die
/// Funktionen begleichen den ersten Eintrag, dann geht es mit der
/// Warteschlange weiter.
static RETURN_FULFILLED: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: return_fulfilled,
};
static RETURN_REJECTED: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: return_rejected,
};

/// [[AsyncGeneratorState]] (Abschnitt 27.9.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    SuspendedStart,
    SuspendedYield,
    Executing,
    DrainingQueue,
    Completed,
}

impl State {
    fn code(self) -> i32 {
        match self {
            State::SuspendedStart => 0,
            State::SuspendedYield => 1,
            State::Executing => 2,
            State::DrainingQueue => 3,
            State::Completed => 4,
        }
    }

    fn from_code(code: i32) -> Option<State> {
        Some(match code {
            0 => State::SuspendedStart,
            1 => State::SuspendedYield,
            2 => State::Executing,
            3 => State::DrainingQueue,
            4 => State::Completed,
            _ => return None,
        })
    }
}

/// Das interne Feld `index` des async-Generators `generator`, `None` für
/// andere Werte.
fn field<'h>(heap: &'h Heap, generator: Value<'_>, index: usize) -> Option<Value<'h>> {
    let Kind::Object(object) = generator.kind() else {
        return None;
    };
    heap.async_generator_field(object, index)
}

fn set_field(heap: &Heap, generator: Value<'_>, index: usize, value: Value<'_>) {
    if let Kind::Object(object) = generator.kind() {
        heap.set_async_generator_field(object, index, value);
    }
}

fn state(heap: &Heap, generator: Value<'_>) -> Option<State> {
    State::from_code(field(heap, generator, STATE)?.as_int32()?)
}

fn set_state(heap: &Heap, generator: Value<'_>, state: State) {
    set_field(heap, generator, STATE, Value::int32(state.code()));
}

/// Ob `value` ein async-Generator ist: AsyncGeneratorValidate (Abschnitt
/// 27.9.3.3) mit ~empty~ als Marke, denn eine andere hat keiner.
pub(crate) fn is_async_generator(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        Kind::Object(object) => heap.object_class(object) == Some(ObjectClass::AsyncGenerator),
        _ => false,
    }
}

/// Der Rumpf ist fertig oder läuft nie mehr: Seinen Rahmen braucht
/// niemand mehr.
fn drop_frame(heap: &Heap, generator: Value<'_>) {
    set_field(heap, generator, FRAME, Value::UNDEFINED);
    set_field(heap, generator, OFFSET, Value::int32(RUNNING));
}

/// EvaluateAsyncGeneratorBody (Abschnitt 15.6.2), Schritte 2 bis 5: ein
/// neuer async-Generator für das Funktionsobjekt `function`, mit dem
/// Prototyp nach OrdinaryCreateFromConstructor (Abschnitt 10.1.13),
/// ~suspended-start~ und mit leerer Warteschlange, dazu eine Zelle mit
/// `frame` Plätzen für den Rahmen. `current` ist der aktuelle Realm. Den
/// Rahmen legt danach [`suspend_frame`] hinein.
pub(crate) fn async_generator_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    function: Local<'_>,
    current: RealmId,
    frame: usize,
) -> Completion<Local<'s>> {
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        function,
        Intrinsic::AsyncGeneratorPrototype,
        current,
    )?;
    let cell = agent.heap_mut().alloc_slots(scope, frame)?;
    let shape = agent.empty_shape(scope);
    let generator = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::AsyncGenerator,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    let value = heap.get(generator);
    set_state(heap, value, State::SuspendedStart);
    set_field(heap, value, FUNCTION, heap.get(function));
    set_field(heap, value, FRAME, heap.get(cell));
    set_field(heap, value, OFFSET, Value::int32(RUNNING));
    set_field(heap, value, MODE, Value::int32(Resumption::Next.code()));
    Ok(generator)
}

/// Hält den Rahmen an: Er kommt ab dem Platz `header` des Registerstapels
/// in die Zelle des Generators, so viele Plätze, wie sie hat, und geht beim
/// Befehl bei `offset` weiter. Den Zustand ändert das nicht. `None`, wenn
/// der Generator keine Zelle hat oder der Abstand nicht passt.
pub(crate) fn suspend_frame(
    heap: &Heap,
    generator: Value<'_>,
    header: usize,
    offset: usize,
) -> Option<()> {
    let offset = i32::try_from(offset).ok()?;
    let Kind::Internal(cell) = field(heap, generator, FRAME)?.kind() else {
        return None;
    };
    for index in 0..heap.slot_count(cell) {
        heap.set_slot(cell, index, heap.register(header.saturating_add(index)));
    }
    set_field(heap, generator, OFFSET, Value::int32(offset));
    Some(())
}

/// Was das Fortsetzen des angehaltenen Rahmens braucht: das
/// Funktionsobjekt, die Zelle mit dem Rahmen und den nächsten Befehl.
/// `None`, wenn der Rahmen läuft oder fertig ist.
pub(crate) fn suspended_parts<'s>(
    agent: &Agent,
    scope: &'s Scope,
    generator: Local<'_>,
) -> Option<(Local<'s>, Local<'s>, usize)> {
    let heap = agent.heap();
    let value = heap.get(generator);
    let offset = usize::try_from(field(heap, value, OFFSET)?.as_int32()?).ok()?;
    let frame = field(heap, value, FRAME)?;
    if frame.is_undefined() {
        return None;
    }
    let function = field(heap, value, FUNCTION)?;
    Some((
        heap.local(scope, function),
        heap.local(scope, frame),
        offset,
    ))
}

/// Der Rahmen läuft wieder, fortgesetzt mit `resumption`, und der
/// Generator ist ~executing~. Erst dann, wenn der Rahmen wieder steht.
pub(crate) fn start_running(heap: &Heap, generator: Value<'_>, resumption: Resumption) {
    set_state(heap, generator, State::Executing);
    set_field(heap, generator, OFFSET, Value::int32(RUNNING));
    set_resumption(heap, generator, resumption);
}

/// Womit der Rahmen zuletzt fortgesetzt wurde. Danach gilt wieder ~next~,
/// bis er das nächste Mal fortgesetzt wird.
pub(crate) fn take_resumption(heap: &Heap, generator: Value<'_>) -> Resumption {
    let code = field(heap, generator, MODE).and_then(|mode| mode.as_int32());
    set_resumption(heap, generator, Resumption::Next);
    Resumption::from_code(code.unwrap_or(0))
}

/// Merkt `resumption` für den nächsten Befehl, der es liest.
pub(crate) fn set_resumption(heap: &Heap, generator: Value<'_>, resumption: Resumption) {
    set_field(heap, generator, MODE, Value::int32(resumption.code()));
}

/// Der Realm dessen, der den Rahmen zuletzt fortsetzte.
fn caller(heap: &Heap, generator: Value<'_>) -> Option<RealmId> {
    let index = field(heap, generator, CALLER)?.as_int32()?;
    usize::try_from(index).ok().map(RealmId::from_index)
}

fn set_caller(agent: &mut Agent, generator: Local<'_>, realm: RealmId) -> Completion<()> {
    let index =
        i32::try_from(realm.index()).map_err(|_| agent.throw_error(ErrorKind::RangeError))?;
    let heap = agent.heap();
    set_field(heap, heap.get(generator), CALLER, Value::int32(index));
    Ok(())
}

// Die Warteschlange

/// AsyncGeneratorEnqueue (Abschnitt 27.9.3.4): ein Eintrag mit dem
/// Abschluss aus `resumption` und `value` und mit `capability` am Ende der
/// Warteschlange.
fn enqueue(
    agent: &mut Agent,
    scope: &Scope,
    generator: Local<'_>,
    resumption: Resumption,
    value: Local<'_>,
    capability: &Capability<'_>,
) -> Completion<()> {
    let request = agent.heap_mut().alloc_slots(scope, REQUEST_SLOTS)?;
    let heap = agent.heap();
    let entry = heap.get(request);
    let Kind::Internal(cell) = entry.kind() else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    heap.set_slot(cell, REQUEST_KIND, Value::int32(resumption.code()));
    heap.set_slot(cell, REQUEST_VALUE, heap.get(value));
    heap.set_slot(cell, REQUEST_RESOLVE, heap.get(capability.resolve));
    heap.set_slot(cell, REQUEST_REJECT, heap.get(capability.reject));
    let target = heap.get(generator);
    match field(heap, target, TAIL).map(|tail| tail.kind()) {
        Some(Kind::Internal(tail)) => {
            heap.set_slot(tail, REQUEST_NEXT, entry);
        }
        _ => set_field(heap, target, HEAD, entry),
    }
    set_field(heap, target, TAIL, entry);
    Ok(())
}

/// Der Abschluss des ersten Eintrags der Warteschlange, `None`, wenn sie
/// leer ist.
fn first_request<'s>(
    agent: &Agent,
    scope: &'s Scope,
    generator: Local<'_>,
) -> Option<(Resumption, Local<'s>)> {
    let heap = agent.heap();
    let Kind::Internal(cell) = field(heap, heap.get(generator), HEAD)?.kind() else {
        return None;
    };
    let kind = heap.slot(cell, REQUEST_KIND)?.as_int32()?;
    let value = heap.slot(cell, REQUEST_VALUE)?;
    Some((Resumption::from_code(kind), heap.local(scope, value)))
}

/// Nimmt den ersten Eintrag aus der Warteschlange und gibt [[Resolve]] und
/// [[Reject]] seiner Capability zurück, `None`, wenn sie leer ist.
fn remove_first<'s>(
    agent: &Agent,
    scope: &'s Scope,
    generator: Local<'_>,
) -> Option<(Local<'s>, Local<'s>)> {
    let heap = agent.heap();
    let target = heap.get(generator);
    let Kind::Internal(cell) = field(heap, target, HEAD)?.kind() else {
        return None;
    };
    let next = heap.slot(cell, REQUEST_NEXT).unwrap_or(Value::UNDEFINED);
    let resolve = heap.slot(cell, REQUEST_RESOLVE)?;
    let reject = heap.slot(cell, REQUEST_REJECT)?;
    let functions = (heap.local(scope, resolve), heap.local(scope, reject));
    set_field(heap, target, HEAD, next);
    if next.is_undefined() {
        set_field(heap, target, TAIL, Value::UNDEFINED);
    }
    Some(functions)
}

/// AsyncGeneratorCompleteStep (Abschnitt 27.9.3.5): Der erste Eintrag geht
/// aus der Warteschlange. Mit `throw` wird sein Promise mit `value`
/// abgelehnt, sonst mit einem Ergebnis aus `value` und `done` erfüllt, das
/// im Realm `realm` entsteht. Ohne Eintrag geschieht nichts; das kommt
/// nicht vor.
pub(crate) fn complete_step(
    agent: &mut Agent,
    scope: &Scope,
    generator: Local<'_>,
    value: Local<'_>,
    throw: bool,
    done: bool,
    realm: RealmId,
) -> Completion<()> {
    let Some((resolve, reject)) = remove_first(agent, scope, generator) else {
        return Ok(());
    };
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    if throw {
        call(agent, scope, reject, undefined, &[value])?;
        return Ok(());
    }
    let result = create_iterator_result_object(agent, scope, realm, value, done)?;
    call(agent, scope, resolve, undefined, &[result])?;
    Ok(())
}

/// Was mit dem ersten Eintrag der Warteschlange geschah.
enum Drained {
    /// Er ist beglichen; es geht mit dem nächsten weiter.
    Settled,
    /// Er wartet auf den Wert für `return`.
    Waiting,
    /// Es gibt keinen.
    Empty,
}

/// AsyncGeneratorDrainQueue (Abschnitt 27.9.3.10) im Realm `realm`, bei
/// ~draining-queue~: Jeder Eintrag bis zum ersten mit `return` wird
/// beglichen, wie nach dem Ende des Rumpfs. Für einen mit `return` folgt
/// AsyncGeneratorAwaitReturn (Abschnitt 27.9.3.9); scheitert dort
/// PromiseResolve, geht die Schleife weiter, statt dass sich beide
/// gegenseitig aufrufen. Ist die Warteschlange leer, ist der Generator
/// ~completed~.
fn drain_queue(agent: &mut Agent, generator: Local<'_>, realm: RealmId) -> Completion<()> {
    loop {
        let drained = agent.scope(|agent, inner| drain_first(agent, inner, generator, realm))?;
        match drained {
            Drained::Settled => {}
            Drained::Waiting => return Ok(()),
            Drained::Empty => {
                let heap = agent.heap();
                set_state(heap, heap.get(generator), State::Completed);
                return Ok(());
            }
        }
    }
}

/// Ein Schritt von [`drain_queue`] für den ersten Eintrag.
fn drain_first(
    agent: &mut Agent,
    scope: &Scope,
    generator: Local<'_>,
    realm: RealmId,
) -> Completion<Drained> {
    let Some((resumption, value)) = first_request(agent, scope, generator) else {
        return Ok(Drained::Empty);
    };
    match resumption {
        Resumption::Next => {
            let undefined = agent.heap().local(scope, Value::UNDEFINED);
            complete_step(agent, scope, generator, undefined, false, true, realm)?;
        }
        Resumption::Throw => complete_step(agent, scope, generator, value, true, true, realm)?,
        Resumption::Return => {
            let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
            match promise_resolve(agent, scope, realm, constructor, value) {
                Ok(promise) => {
                    perform_then_with(
                        agent,
                        scope,
                        realm,
                        promise,
                        generator,
                        &RETURN_FULFILLED,
                        &RETURN_REJECTED,
                    )?;
                    return Ok(Drained::Waiting);
                }
                Err(Abrupt::Throw) => {
                    let reason = agent.take_thrown_value(scope, realm)?;
                    complete_step(agent, scope, generator, reason, true, true, realm)?;
                }
                Err(abrupt) => return Err(abrupt),
            }
        }
    }
    Ok(Drained::Settled)
}

/// AsyncGeneratorStart (Abschnitt 27.9.3.2), Schritte 4.g bis 4.k: Der Rumpf
/// endete mit `value`, mit `throw` durch eine Ausnahme. Der Generator ist
/// ~draining-queue~, der erste Eintrag bekommt das Ergebnis, dann die
/// übrigen.
fn finish(
    agent: &mut Agent,
    scope: &Scope,
    generator: Local<'_>,
    value: Local<'_>,
    throw: bool,
    realm: RealmId,
) -> Completion<()> {
    let heap = agent.heap();
    let target = heap.get(generator);
    set_state(heap, target, State::DrainingQueue);
    drop_frame(heap, target);
    complete_step(agent, scope, generator, value, throw, true, realm)?;
    drain_queue(agent, generator, realm)
}

/// Der Generator läuft nach einem Abbruch, der keine Ausnahme ist, etwa
/// einer verbrauchten Quote, nie mehr: Er ist ~completed~, und die
/// Warteschlange ist leer. Ihre Promises bleiben offen.
fn abandon(heap: &Heap, generator: Value<'_>) {
    set_state(heap, generator, State::Completed);
    drop_frame(heap, generator);
    set_field(heap, generator, HEAD, Value::UNDEFINED);
    set_field(heap, generator, TAIL, Value::UNDEFINED);
}

/// Setzt den Rahmen mit `resumption` und `value` fort, im Auftrag des
/// Realms `realm` (AsyncGeneratorResume, Abschnitt 27.9.3.6, und die
/// Funktionen eines Await), und erledigt, was nach dem Ende des Rumpfs
/// kommt. Eine Ausnahme beendet den Rumpf, auch wenn schon das Fortsetzen
/// an ihr scheitert, etwa am Stapel: Dann lehnt sie den ersten Eintrag ab.
/// Andere Abbrüche gehen an den Aufrufer weiter, und der Generator bleibt
/// fertig zurück.
fn resume(
    agent: &mut Agent,
    scope: &Scope,
    generator: Local<'_>,
    resumption: Resumption,
    value: Local<'_>,
    realm: RealmId,
) -> Completion<()> {
    set_caller(agent, generator, realm)?;
    match resume_async_generator(agent, scope, generator, resumption, value) {
        Ok((Ending::Suspended, _)) => Ok(()),
        Ok((Ending::Returned, result)) => finish(agent, scope, generator, result, false, realm),
        Err(Abrupt::Throw) => {
            let reason = agent.take_thrown_value(scope, realm)?;
            finish(agent, scope, generator, reason, true, realm)
        }
        Err(abrupt) => {
            let heap = agent.heap();
            abandon(heap, heap.get(generator));
            Err(abrupt)
        }
    }
}

/// Await im Rumpf des Generators `generator` (Abschnitt 27.10.5.3),
/// Schritte 2 bis 7, im aktuellen Realm `realm`: PromiseResolve mit
/// %Promise%, dann PerformPromiseThen mit zwei Funktionen, die den Rahmen
/// fortsetzen, mit `return` als Fortsetzung für den Wert für
/// AsyncGeneratorUnwrapYieldResumption, sonst mit `next`. Den Rahmen hält
/// danach der Aufrufer an.
pub(crate) fn await_value(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    generator: Local<'_>,
    value: Local<'_>,
    returning: bool,
) -> Completion<()> {
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let promise = promise_resolve(agent, scope, realm, constructor, value)?;
    let fulfilled = if returning {
        &RESUME_RETURN
    } else {
        &RESUME_NEXT
    };
    perform_then_with(
        agent,
        scope,
        realm,
        promise,
        generator,
        fulfilled,
        &RESUME_THROW,
    )
}

/// AsyncGeneratorYield (Abschnitt 27.9.3.8) mit `value`, auf das schon
/// gewartet wurde, im Rahmen ab dem Platz `header` im Realm `realm`, bis
/// auf den Wechsel des Laufs: Der erste Eintrag bekommt ein Ergebnis aus
/// dem Realm dessen, der den Rahmen fortsetzte. Wartet danach ein weiterer
/// Eintrag, gilt sein Abschluss gleich (Schritt 11): Das Ergebnis ist sein
/// Wert, und die Fortsetzung steht im Generator; auf den Wert für `return`
/// wartet erst der Rahmen, mit dem Befehl bei `offset` als Fortsetzung,
/// und das Ergebnis ist `None`. Scheitert dabei PromiseResolve, gilt die
/// Ausnahme wie ein Abschluss mit `throw`. Ist die Warteschlange leer,
/// hält der Rahmen ~suspended-yield~ an, und das Ergebnis ist ebenfalls
/// `None`.
pub(crate) fn async_generator_yield<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    generator: Local<'_>,
    value: Local<'_>,
    header: usize,
    offset: usize,
) -> Completion<Option<Local<'s>>> {
    let previous = {
        let heap = agent.heap();
        caller(heap, heap.get(generator)).unwrap_or(realm)
    };
    complete_step(agent, scope, generator, value, false, false, previous)?;
    let suspended = match first_request(agent, scope, generator) {
        None => {
            let heap = agent.heap();
            set_state(heap, heap.get(generator), State::SuspendedYield);
            suspend_frame(heap, heap.get(generator), header, offset)
        }
        Some((Resumption::Return, value)) => {
            match await_value(agent, scope, realm, generator, value, true) {
                Ok(()) => {
                    let heap = agent.heap();
                    suspend_frame(heap, heap.get(generator), header, offset)
                }
                Err(Abrupt::Throw) => {
                    let reason = agent.take_thrown_value(scope, realm)?;
                    let heap = agent.heap();
                    set_resumption(heap, heap.get(generator), Resumption::Throw);
                    return Ok(Some(reason));
                }
                Err(abrupt) => return Err(abrupt),
            }
        }
        Some((resumption, value)) => {
            let heap = agent.heap();
            set_resumption(heap, heap.get(generator), resumption);
            return Ok(Some(value));
        }
    };
    // Kann nicht scheitern: Der Generator hat seine Zelle, und der
    // Übersetzer schreibt höchstens `i32::MAX` Bytes.
    if suspended.is_none() {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    Ok(None)
}

/// AsyncGeneratorResume (Abschnitt 27.9.3.6) mit einem Abschluss mit
/// `return` für ein `yield`, im Auftrag des Realms `realm`: Der Generator
/// ist ~executing~, und im Rahmen beginnt AsyncGeneratorUnwrapYieldResumption
/// mit dem Await auf `value`, im Realm des Rahmens. Scheitert dort
/// PromiseResolve, geht der Rahmen gleich mit `throw` weiter.
fn resume_with_return(
    agent: &mut Agent,
    scope: &Scope,
    generator: Local<'_>,
    value: Local<'_>,
    realm: RealmId,
) -> Completion<()> {
    let own = {
        let heap = agent.heap();
        let target = heap.get(generator);
        set_state(heap, target, State::Executing);
        field(heap, target, FUNCTION)
            .and_then(|function| function_parts(heap, function))
            .map_or(realm, |(_, own)| own)
    };
    set_caller(agent, generator, realm)?;
    match await_value(agent, scope, own, generator, value, true) {
        Ok(()) => Ok(()),
        Err(Abrupt::Throw) => {
            let reason = agent.take_thrown_value(scope, own)?;
            resume(agent, scope, generator, Resumption::Throw, reason, realm)
        }
        Err(abrupt) => {
            let heap = agent.heap();
            abandon(heap, heap.get(generator));
            Err(abrupt)
        }
    }
}

/// %AsyncGeneratorPrototype%.next, `return` und `throw` (Abschnitte
/// 27.9.1.2 bis 27.9.1.4) ab Schritt 5, mit dem Generator `generator`, den
/// der Aufrufer schon geprüft hat, und `capability` aus dem aktuellen Realm
/// `realm`.
pub(crate) fn async_generator_request(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    generator: Local<'_>,
    resumption: Resumption,
    value: Local<'_>,
    capability: &Capability<'_>,
) -> Completion<()> {
    let found = {
        let heap = agent.heap();
        state(heap, heap.get(generator))
    };
    let Some(mut current) = found else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    match resumption {
        Resumption::Next => {
            if current == State::Completed {
                let result = create_iterator_result_object(agent, scope, realm, undefined, true)?;
                call(agent, scope, capability.resolve, undefined, &[result])?;
                return Ok(());
            }
            enqueue(agent, scope, generator, resumption, value, capability)?;
            if matches!(current, State::SuspendedStart | State::SuspendedYield) {
                resume(agent, scope, generator, resumption, value, realm)?;
            }
        }
        Resumption::Return => {
            enqueue(agent, scope, generator, resumption, value, capability)?;
            match current {
                State::SuspendedStart | State::Completed => {
                    let heap = agent.heap();
                    let target = heap.get(generator);
                    set_state(heap, target, State::DrainingQueue);
                    drop_frame(heap, target);
                    drain_queue(agent, generator, realm)?;
                }
                State::SuspendedYield => {
                    resume_with_return(agent, scope, generator, value, realm)?;
                }
                State::Executing | State::DrainingQueue => {}
            }
        }
        Resumption::Throw => {
            if current == State::SuspendedStart {
                let heap = agent.heap();
                let target = heap.get(generator);
                set_state(heap, target, State::Completed);
                drop_frame(heap, target);
                current = State::Completed;
            }
            if current == State::Completed {
                call(agent, scope, capability.reject, undefined, &[value])?;
                return Ok(());
            }
            enqueue(agent, scope, generator, resumption, value, capability)?;
            if current == State::SuspendedYield {
                resume(agent, scope, generator, resumption, value, realm)?;
            }
        }
    }
    Ok(())
}

/// Setzt den Rahmen des eingefangenen Generators mit dem ersten Argument
/// fort, im Realm der Funktion.
fn resume_captured<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    resumption: Resumption,
) -> Completion<Local<'s>> {
    let generator = arguments.captures(agent, scope);
    let value = arguments.argument(agent, scope, 0);
    resume(agent, scope, generator, resumption, value, arguments.realm)?;
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// *onFulfilled* aus Await.
fn resume_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume_captured(agent, scope, arguments, Resumption::Next)
}

/// *onRejected* aus Await.
fn resume_throw<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume_captured(agent, scope, arguments, Resumption::Throw)
}

/// *onFulfilled* aus dem Await von AsyncGeneratorUnwrapYieldResumption:
/// ReturnCompletion mit dem Wert.
fn resume_return<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume_captured(agent, scope, arguments, Resumption::Return)
}

/// *fulfilledClosure* und *rejectedClosure* aus AsyncGeneratorAwaitReturn.
fn settle_return<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    throw: bool,
) -> Completion<Local<'s>> {
    let generator = arguments.captures(agent, scope);
    let value = arguments.argument(agent, scope, 0);
    let realm = arguments.realm;
    complete_step(agent, scope, generator, value, throw, true, realm)?;
    drain_queue(agent, generator, realm)?;
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

fn return_fulfilled<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    settle_return(agent, scope, arguments, false)
}

fn return_rejected<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    settle_return(agent, scope, arguments, true)
}

#[cfg(test)]
mod tests;
