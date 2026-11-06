//! Generatoren (Abschnitt 27.8): die Objekte, die der Aufruf einer
//! Generatorfunktion zurückgibt, und GeneratorResume und
//! GeneratorResumeAbrupt (Abschnitte 27.8.3.3 und 27.8.3.4) für `next`,
//! `return` und `throw`.
//!
//! Ein Generator ist ein Objekt der Art [`ObjectClass::Generator`] mit fünf
//! internen Feldern: [[GeneratorState]] als Zahl; das Funktionsobjekt, aus
//! dem Code und Realm seines Rahmens kommen; der angehaltene Rahmen
//! ([[GeneratorContext]]) als interne Zelle mit dem Kopf und den Registern;
//! der Befehl, mit dem der Rahmen weitergeht; und wie der Generator zuletzt
//! fortgesetzt wurde, bis der Befehl danach es liest. [[GeneratorBrand]]
//! ist für alle ~empty~: Andere Marken bringen erst Iteratoren aus Closures
//! (CreateIteratorFromClosure, Abschnitt 27.8.3.8), und der Iterator für
//! Zeichenketten ist kein Generator, sondern ein eigener Iterator.
//!
//! Fortsetzen heißt: Der Interpreter legt den Rahmen aus der Zelle wieder
//! auf den Registerstapel und beginnt mit ihm einen neuen Lauf
//! ([`resume_frame`]). Hält der Rahmen bei `yield` an, schreibt der Befehl
//! ihn zurück in die Zelle. Jeder Aufruf von `next` ist so ein Lauf wie der
//! eines Getters und kostet Rust-Stapel nur im Rahmen von
//! [`crate::agent::STACK_BUDGET`]; ein `yield*` auf einen Generator
//! schachtelt einen weiteren.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::heap::{Heap, Local, ObjectClass, Scope};
use crate::interpreter::{Ending, resume_frame};
use crate::iterator::create_iterator_result_object;
use crate::object::{INLINE, get_prototype_from_constructor};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Das interne Feld mit [[GeneratorState]].
const STATE: usize = 0;
/// Das interne Feld mit dem Funktionsobjekt.
const FUNCTION: usize = 1;
/// Das interne Feld mit dem Rahmen, *undefined*, wenn der Generator fertig
/// ist.
const FRAME: usize = 2;
/// Das interne Feld mit dem Befehl, bei dem der Rahmen weitergeht.
const OFFSET: usize = 3;
/// Das interne Feld mit der Art der letzten Fortsetzung.
const MODE: usize = 4;

/// [[GeneratorState]] (Abschnitt 27.8.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    SuspendedStart,
    SuspendedYield,
    Executing,
    Completed,
}

impl State {
    fn code(self) -> i32 {
        match self {
            State::SuspendedStart => 0,
            State::SuspendedYield => 1,
            State::Executing => 2,
            State::Completed => 3,
        }
    }

    fn from_code(code: i32) -> Option<State> {
        Some(match code {
            0 => State::SuspendedStart,
            1 => State::SuspendedYield,
            2 => State::Executing,
            3 => State::Completed,
            _ => return None,
        })
    }
}

/// Womit ein Generator fortgesetzt wird: der Abschluss, den `yield` im
/// Rahmen ergibt, normal mit `next`, sonst mit `throw` oder `return`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resumption {
    Next,
    Throw,
    Return,
}

impl Resumption {
    fn code(self) -> i32 {
        match self {
            Resumption::Next => 0,
            Resumption::Throw => 1,
            Resumption::Return => 2,
        }
    }

    fn from_code(code: i32) -> Resumption {
        match code {
            1 => Resumption::Throw,
            2 => Resumption::Return,
            _ => Resumption::Next,
        }
    }
}

/// Das interne Feld `index` des Generators `generator`, `None` für andere
/// Werte.
fn field<'h>(heap: &'h Heap, generator: Value<'_>, index: usize) -> Option<Value<'h>> {
    let Kind::Object(object) = generator.kind() else {
        return None;
    };
    heap.generator_field(object, index)
}

fn set_field(heap: &Heap, generator: Value<'_>, index: usize, value: Value<'_>) {
    if let Kind::Object(object) = generator.kind() {
        heap.set_generator_field(object, index, value);
    }
}

fn state(heap: &Heap, generator: Value<'_>) -> Option<State> {
    State::from_code(field(heap, generator, STATE)?.as_int32()?)
}

fn set_state(heap: &Heap, generator: Value<'_>, state: State) {
    set_field(heap, generator, STATE, Value::int32(state.code()));
}

/// Der Generator ist fertig (~completed~) und bleibt es. Seinen Rahmen
/// braucht niemand mehr.
fn complete(heap: &Heap, generator: Value<'_>) {
    set_state(heap, generator, State::Completed);
    set_field(heap, generator, FRAME, Value::UNDEFINED);
}

/// EvaluateGeneratorBody (Abschnitt 15.5.2), Schritte 2 bis 5: ein neuer
/// Generator für das Funktionsobjekt `function`, mit dem Prototyp nach
/// OrdinaryCreateFromConstructor (Abschnitt 10.1.13) und ~suspended-start~,
/// dazu eine Zelle mit `frame` Plätzen für den Rahmen, der beim Befehl bei
/// `offset` beginnt. `current` ist der aktuelle Realm.
pub(crate) fn generator_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    function: Local<'_>,
    current: RealmId,
    frame: usize,
    offset: usize,
) -> Completion<Local<'s>> {
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        function,
        Intrinsic::GeneratorPrototype,
        current,
    )?;
    // Der Übersetzer schreibt höchstens `i32::MAX` Bytes.
    let offset = i32::try_from(offset).map_err(|_| agent.throw_error(ErrorKind::RangeError))?;
    let cell = agent.heap_mut().alloc_slots(scope, frame)?;
    let shape = agent.empty_shape(scope);
    let generator = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::Generator,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    let value = heap.get(generator);
    set_state(heap, value, State::SuspendedStart);
    set_field(heap, value, FUNCTION, heap.get(function));
    set_field(heap, value, FRAME, heap.get(cell));
    set_field(heap, value, OFFSET, Value::int32(offset));
    set_field(heap, value, MODE, Value::int32(Resumption::Next.code()));
    Ok(generator)
}

/// Kopiert den Rahmen ab dem Platz `header` des Registerstapels in die
/// Zelle des Generators, so viele Plätze, wie sie hat. `None`, wenn
/// `generator` keine Zelle hat.
pub(crate) fn save_frame(heap: &Heap, generator: Value<'_>, header: usize) -> Option<()> {
    let Kind::Internal(cell) = field(heap, generator, FRAME)?.kind() else {
        return None;
    };
    for index in 0..heap.slot_count(cell) {
        heap.set_slot(cell, index, heap.register(header.saturating_add(index)));
    }
    Some(())
}

/// GeneratorYield (Abschnitt 27.8.3.6) bis auf den Wechsel des Laufs: Der
/// Generator wird ~suspended-yield~, sein Rahmen ab dem Platz `header`
/// kommt in die Zelle und geht beim Befehl bei `offset` weiter.
pub(crate) fn generator_yield(
    heap: &Heap,
    generator: Value<'_>,
    header: usize,
    offset: usize,
) -> Option<()> {
    let offset = i32::try_from(offset).ok()?;
    save_frame(heap, generator, header)?;
    set_field(heap, generator, OFFSET, Value::int32(offset));
    set_state(heap, generator, State::SuspendedYield);
    Some(())
}

/// Womit der Generator zuletzt fortgesetzt wurde. Danach gilt wieder
/// ~next~, bis er das nächste Mal fortgesetzt wird.
pub(crate) fn take_resumption(heap: &Heap, generator: Value<'_>) -> Resumption {
    let code = field(heap, generator, MODE).and_then(|mode| mode.as_int32());
    set_field(heap, generator, MODE, Value::int32(Resumption::Next.code()));
    Resumption::from_code(code.unwrap_or(0))
}

/// Merkt `resumption` für den nächsten Befehl, der es liest.
pub(crate) fn set_resumption(heap: &Heap, generator: Value<'_>, resumption: Resumption) {
    set_field(heap, generator, MODE, Value::int32(resumption.code()));
}

/// Was das Fortsetzen des angehaltenen Generators `generator` braucht: sein
/// Funktionsobjekt, die Zelle mit dem Rahmen und den nächsten Befehl.
pub(crate) fn suspended_parts<'s>(
    agent: &Agent,
    scope: &'s Scope,
    generator: Local<'_>,
) -> Option<(Local<'s>, Local<'s>, usize)> {
    let heap = agent.heap();
    let value = heap.get(generator);
    let function = field(heap, value, FUNCTION)?;
    let frame = field(heap, value, FRAME)?;
    let offset = usize::try_from(field(heap, value, OFFSET)?.as_int32()?).ok()?;
    Some((
        heap.local(scope, function),
        heap.local(scope, frame),
        offset,
    ))
}

/// Der Generator läuft (~executing~), fortgesetzt mit `resumption`. Erst
/// dann, wenn sein Rahmen wieder steht.
pub(crate) fn start_executing(heap: &Heap, generator: Value<'_>, resumption: Resumption) {
    set_state(heap, generator, State::Executing);
    set_resumption(heap, generator, resumption);
}

/// GeneratorResume und GeneratorResumeAbrupt (Abschnitte 27.8.3.3 und
/// 27.8.3.4) mit ~empty~ als Marke und GeneratorValidate (Abschnitt
/// 27.8.3.2): `next` setzt den Generator mit `value` fort, `return` und
/// `throw` mit einem Abschluss mit `value`. `realm` ist der aktuelle Realm,
/// der der Methode; aus ihm kommt das Ergebnis, wenn der Generator fertig ist
/// oder es dabei wird (GeneratorStart, Abschnitt 27.8.3.1). Das Ergebnis
/// eines `yield` legt der Rahmen in seinem Realm an.
pub(crate) fn resume_generator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    generator: Local<'_>,
    resumption: Resumption,
    value: Local<'_>,
) -> Completion<Local<'s>> {
    let found = {
        let heap = agent.heap();
        state(heap, heap.get(generator))
    };
    let current = match found {
        None | Some(State::Executing) => return Err(agent.throw_error(ErrorKind::TypeError)),
        // GeneratorResumeAbrupt, Schritt 2: Vor dem ersten `next` beendet
        // ein Abschluss den Generator, ohne dass sein Rumpf läuft.
        Some(State::SuspendedStart) if resumption != Resumption::Next => {
            let heap = agent.heap();
            complete(heap, heap.get(generator));
            State::Completed
        }
        Some(state) => state,
    };
    if current == State::Completed {
        return match resumption {
            Resumption::Next => {
                let undefined = agent.heap().local(scope, Value::UNDEFINED);
                create_iterator_result_object(agent, scope, realm, undefined, true)
            }
            Resumption::Return => create_iterator_result_object(agent, scope, realm, value, true),
            Resumption::Throw => Err(agent.throw_value(value)),
        };
    }
    match resume_frame(agent, scope, generator, resumption, value) {
        Ok((Ending::Suspended, result)) => Ok(result),
        Ok((Ending::Returned, result)) => {
            let heap = agent.heap();
            complete(heap, heap.get(generator));
            create_iterator_result_object(agent, scope, realm, result, true)
        }
        Err(abrupt) => {
            // Lief der Rahmen, ist der Generator danach fertig, auch nach
            // einer anderen abrupten Beendigung als einer Ausnahme.
            // Scheiterte schon das Fortsetzen, etwa am Stapel, bleibt er,
            // wie er war.
            let heap = agent.heap();
            let value = heap.get(generator);
            if state(heap, value) == Some(State::Executing) {
                complete(heap, value);
            }
            Err(abrupt)
        }
    }
}
