//! Iterator Helper objects (Abschnitt 27.1.2) und %IteratorHelperPrototype%
//! (Abschnitt 27.1.2.1).
//!
//! Die Spezifikation legt jeden Helfer mit CreateIteratorFromClosure
//! (Abschnitt 27.5.3.4) als Generator aus einer Closure an, die an ihren
//! Yields anhält. Eine eingebaute Funktion kann nicht mitten in Rust-Code
//! anhalten; darum ist ein Helfer ein Objekt der Art
//! [`ObjectClass::Iterator`](crate::heap::ObjectClass::Iterator) mit der Art
//! [`HELPER`], dessen erstes Feld auf eine interne Zelle mit dem Zustand der
//! Closure zeigt: welche Methode ihn anlegte, sein [[GeneratorState]], der
//! Realm des Generator-Kontexts, der Iterator Record der Quelle und was die
//! Closure sonst einfängt. Jedes `next` setzt die Closure am letzten Yield
//! fort und läuft bis zum nächsten, `return` setzt sie mit einer
//! Return-Completion fort (GeneratorResume und GeneratorResumeAbrupt,
//! Abschnitte 27.5.3.3 und 27.5.3.4). Die Marke [[GeneratorBrand]]
//! *"Iterator Helper"* ist die Art des Objekts; Generatoren aus
//! Generatorfunktionen sind andere Objekte.
//!
//! Jeder Durchlauf einer Schleife in der Closure kostet einen Arbeitsschritt
//! und hat einen eigenen Bereich für Handles. Was die Closure in Listen hält
//! (die Puffer von `chunks` und `windows`, die Quellen von `concat` und
//! `zip`), liegt in weiteren internen Zellen.

use super::{HELPER, KIND, TARGET, field, iterator_create};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall};
use crate::conversion::to_boolean;
use crate::heap::{Local, Scope};
use crate::iterator::{
    close_if_abrupt, create_iterator_result_object, get_iterator_direct, get_iterator_flattenable,
    iterator_close, iterator_close_all, iterator_step, iterator_step_value,
};
use crate::key::PropertyKey;
use crate::object::{array_create, ordinary_object_create};
use crate::operations::{call, create_array_from_list, create_data_property_or_throw};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Die Methoden, die einen Helfer anlegen; die Zahl steht im Feld
/// [`OPERATION`].
pub(super) const MAP: i32 = 0;
pub(super) const FILTER: i32 = 1;
pub(super) const TAKE: i32 = 2;
pub(super) const DROP: i32 = 3;
pub(super) const FLAT_MAP: i32 = 4;
pub(super) const CHUNKS: i32 = 5;
pub(super) const WINDOWS: i32 = 6;
pub(super) const CONCAT: i32 = 7;
pub(super) const ZIP: i32 = 8;

/// [[GeneratorState]]: ~suspended-start~, ~suspended-yield~, ~executing~
/// und ~completed~.
const START: i32 = 0;
const SUSPENDED: i32 = 1;
const EXECUTING: i32 = 2;
const COMPLETED: i32 = 3;

/// Die Methode, die den Helfer anlegte.
const OPERATION: usize = 0;
/// [[GeneratorState]].
const STATE: usize = 1;
/// Die Nummer des Realms, in dem der Helfer entstand: der Realm des
/// Generator-Kontexts, in dem die Closure läuft.
const REALM: usize = 2;
/// [[Iterator]] und [[NextMethod]] der Quelle, für `concat` der Iterator,
/// den die Closure gerade durchläuft. *undefined*, wenn es keinen gibt
/// oder die Quelle fertig ist und nur noch ein letzter Puffer kam.
pub(super) const ITERATOR: usize = 3;
pub(super) const NEXT: usize = 4;
/// `mapper` oder `predicate`; für `concat` die Liste der Quellen, für `zip`
/// die Liste der Iteratoren.
pub(super) const FUNCTION: usize = 5;
/// Der Zähler `counter`; für `concat` der Index der nächsten Quelle.
const COUNTER: usize = 6;
/// [[Iterator]] und [[NextMethod]] des inneren Iterators von `flatMap`.
/// Für `zipKeyed` hält [`INNER`] die Liste der Schlüssel.
pub(super) const INNER: usize = 7;
const INNER_NEXT: usize = 8;
/// Der Puffer von `chunks` und `windows`, ein Ring in einer internen Zelle;
/// für `zip` die Liste `padding`.
pub(super) const BUFFER: usize = 9;
/// Wie viele Werte der Puffer hält.
const LENGTH: usize = 10;
/// Wo der Ring beginnt.
const FIRST: usize = 11;
/// `limit` von `take` und `drop` als übrige Zahl, die Größe von `chunks`
/// und `windows`; für `concat` und `zip` die Zahl der Quellen.
pub(super) const LIMIT: usize = 12;
/// Für `windows` *true* bei *"allow-partial"*, für `zip` der Modus.
pub(super) const OPTION: usize = 13;
const FIELDS: usize = 14;

/// Die Modi von `zip` und `zipKeyed`.
pub(super) const SHORTEST: i32 = 0;
pub(super) const LONGEST: i32 = 1;
pub(super) const STRICT: i32 = 2;

/// Felder je Iterator in der Liste von `zip`: [[Iterator]], [[NextMethod]]
/// und ob er noch in *openIters* steht.
pub(super) const ZIP_ENTRY: usize = 3;

/// Felder je Quelle in der Liste von `concat`: [[OpenMethod]] und
/// [[Iterable]].
pub(super) const CONCAT_ENTRY: usize = 2;

/// Die Anfangsgröße eines Puffers.
const INITIAL_CAPACITY: usize = 8;

pub(super) static FUNCTIONS: [Builtin; 2] = [
    Builtin {
        name: "next",
        length: 0,
        constructor: false,
        behaviour: helper_next,
    },
    Builtin {
        name: "return",
        length: 0,
        constructor: false,
        behaviour: helper_return,
    },
];

/// Wie die Closure fortgesetzt wird.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Resumption {
    /// Mit `next`: Yield gibt *undefined* zurück.
    Next,
    /// Mit `return`: Yield gibt eine Return-Completion zurück.
    Return,
}

/// Was ein Durchlauf einer Schleife der Closure ergab.
enum Step<'s> {
    /// Die Closure endet mit einer Return-Completion.
    Done,
    /// Die Schleife läuft weiter.
    Continue,
    /// Die Closure hält an diesem Yield an.
    Yield(Local<'s>),
}

/// Legt einen Helfer der Art `operation` im Zustand ~suspended-start~ an
/// (CreateIteratorFromClosure mit %IteratorHelperPrototype% aus `realm`,
/// dem aktuellen Realm). Die Felder der Closure setzt der Aufrufer in die
/// zurückgegebene Zelle.
pub(super) fn create_helper<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    operation: i32,
) -> Completion<(Local<'s>, Local<'s>)> {
    let state = agent.heap_mut().alloc_slots(scope, FIELDS)?;
    write(agent, state, OPERATION, Value::int32(operation));
    write(agent, state, STATE, Value::int32(START));
    // So viele Realms passen nicht in den Speicher.
    let index =
        i32::try_from(realm.index()).map_err(|_| agent.throw_error(ErrorKind::RangeError))?;
    write(agent, state, REALM, Value::int32(index));
    write(agent, state, COUNTER, Value::number(0.0));
    write(agent, state, LENGTH, Value::number(0.0));
    write(agent, state, FIRST, Value::number(0.0));
    let prototype = agent.intrinsic(scope, realm, Intrinsic::IteratorHelperPrototype);
    let object = iterator_create(agent, scope, prototype, state, HELPER)?;
    Ok((object, state))
}

/// Ein Feld der internen Zelle `cell`, *undefined*, wenn es fehlt.
pub(super) fn read<'s>(
    agent: &Agent,
    scope: &'s Scope,
    cell: Local<'_>,
    index: usize,
) -> Local<'s> {
    let heap = agent.heap();
    let value = match heap.get(cell).kind() {
        Kind::Internal(cell) => heap.slot(cell, index).unwrap_or(Value::UNDEFINED),
        _ => Value::UNDEFINED,
    };
    heap.local(scope, value)
}

/// Schreibt ein Feld der internen Zelle `cell`.
pub(super) fn write(agent: &Agent, cell: Local<'_>, index: usize, value: Value<'_>) {
    let heap = agent.heap();
    if let Kind::Internal(cell) = heap.get(cell).kind() {
        heap.set_slot(cell, index, value);
    }
}

/// Schreibt den Wert des Handles `value` in ein Feld.
pub(super) fn write_local(agent: &Agent, cell: Local<'_>, index: usize, value: Local<'_>) {
    write(agent, cell, index, agent.heap().get(value));
}

fn number(agent: &Agent, cell: Local<'_>, index: usize) -> f64 {
    let heap = agent.heap();
    match heap.get(cell).kind() {
        Kind::Internal(cell) => heap
            .slot(cell, index)
            .and_then(Value::as_number)
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

fn int(agent: &Agent, cell: Local<'_>, index: usize) -> i32 {
    let heap = agent.heap();
    match heap.get(cell).kind() {
        Kind::Internal(cell) => heap
            .slot(cell, index)
            .and_then(Value::as_int32)
            .unwrap_or(0),
        _ => 0,
    }
}

/// Eine Zahl aus einem Feld als Index. Die Felder für Längen, Größen und
/// Indizes halten ganze Zahlen von 0 bis 2³² − 1.
fn count(agent: &Agent, cell: Local<'_>, index: usize) -> usize {
    // Ganze Zahlen von 0 bis 2³² − 1 passen ohne Verlust in `usize`.
    number(agent, cell, index) as usize
}

fn set_count(agent: &Agent, cell: Local<'_>, index: usize, value: usize) {
    // Die Werte bleiben unter 2³², als f64 genau.
    write(agent, cell, index, Value::number(value as f64));
}

fn flag(agent: &Agent, cell: Local<'_>, index: usize) -> bool {
    let heap = agent.heap();
    match heap.get(cell).kind() {
        Kind::Internal(cell) => heap
            .slot(cell, index)
            .is_some_and(|value| matches!(value.kind(), Kind::Boolean(true))),
        _ => false,
    }
}

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

fn realm_of(agent: &Agent, state: Local<'_>) -> RealmId {
    let index = usize::try_from(int(agent, state, REALM)).unwrap_or(0);
    RealmId::from_index(index)
}

/// Die Zustandszelle des Helfers `this`; ein TypeError für andere Werte
/// (RequireInternalSlot mit [[UnderlyingIterators]] und GeneratorValidate
/// mit der Marke *"Iterator Helper"*).
fn helper_state<'s>(agent: &mut Agent, scope: &'s Scope, this: Local<'_>) -> Completion<Local<'s>> {
    let kind = field(agent, scope, this, KIND).and_then(|kind| agent.heap().get(kind).as_int32());
    if kind != Some(HELPER) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    match field(agent, scope, this, TARGET) {
        Some(state) => Ok(state),
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

fn done<'s>(agent: &mut Agent, scope: &'s Scope, realm: RealmId) -> Completion<Local<'s>> {
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    create_iterator_result_object(agent, scope, realm, undefined, true)
}

/// %IteratorHelperPrototype%.next ( ) (Abschnitt 27.1.2.1.1) mit
/// GeneratorResume (Abschnitt 27.5.3.3).
fn helper_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let state = helper_state(agent, scope, arguments.this)?;
    match int(agent, state, STATE) {
        EXECUTING => return Err(agent.throw_error(ErrorKind::TypeError)),
        COMPLETED => return done(agent, scope, arguments.realm),
        _ => {}
    }
    write(agent, state, STATE, Value::int32(EXECUTING));
    let result = resume(agent, scope, state, Resumption::Next);
    finish(agent, scope, state, result)
}

/// %IteratorHelperPrototype%.return ( ) (Abschnitt 27.1.2.1.2) mit
/// GeneratorResumeAbrupt (Abschnitt 27.5.3.4).
fn helper_return<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let state = helper_state(agent, scope, arguments.this)?;
    match int(agent, state, STATE) {
        START => {
            write(agent, state, STATE, Value::int32(COMPLETED));
            let closed = close_underlying(agent, scope, state);
            clear(agent, state);
            closed?;
            done(agent, scope, arguments.realm)
        }
        EXECUTING => Err(agent.throw_error(ErrorKind::TypeError)),
        COMPLETED => done(agent, scope, arguments.realm),
        _ => {
            write(agent, state, STATE, Value::int32(EXECUTING));
            let result = resume(agent, scope, state, Resumption::Return);
            finish(agent, scope, state, result)
        }
    }
}

/// Das Ergebnis eines Laufs der Closure: Ein Yield hält sie an, eine
/// Return-Completion oder eine Ausnahme beenden sie (~completed~). Fehler
/// der Engine aus der Closure gehören zum Realm des Helfers.
fn finish<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
    result: Completion<Option<Local<'_>>>,
) -> Completion<Local<'s>> {
    let realm = realm_of(agent, state);
    match result {
        Ok(Some(value)) => {
            write(agent, state, STATE, Value::int32(SUSPENDED));
            create_iterator_result_object(agent, scope, realm, value, false)
        }
        Ok(None) => {
            write(agent, state, STATE, Value::int32(COMPLETED));
            clear(agent, state);
            done(agent, scope, realm)
        }
        Err(abrupt) => {
            write(agent, state, STATE, Value::int32(COMPLETED));
            clear(agent, state);
            agent.set_error_realm(realm);
            Err(abrupt)
        }
    }
}

/// Gibt frei, was die beendete Closure festhielt.
fn clear(agent: &Agent, state: Local<'_>) {
    for index in [ITERATOR, NEXT, FUNCTION, INNER, INNER_NEXT, BUFFER] {
        write(agent, state, index, Value::UNDEFINED);
    }
}

/// IteratorCloseAll über [[UnderlyingIterators]] mit einer normalen
/// Completion, für `return` vor dem ersten `next`.
fn close_underlying(agent: &mut Agent, scope: &Scope, state: Local<'_>) -> Completion<()> {
    match int(agent, state, OPERATION) {
        // [[UnderlyingIterators]] von `concat` ist leer.
        CONCAT => Ok(()),
        ZIP => close_zip(agent, state, Ok(())),
        _ => {
            let iterator = read(agent, scope, state, ITERATOR);
            iterator_close(agent, scope, iterator)
        }
    }
}

/// Setzt die Closure fort: `Some` mit dem Wert ihres nächsten Yield,
/// `None`, wenn sie mit einer Return-Completion endet.
fn resume<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
    how: Resumption,
) -> Completion<Option<Local<'s>>> {
    let operation = int(agent, state, OPERATION);
    match (operation, how) {
        (FLAT_MAP, Resumption::Return) => {
            // Erst der innere Iterator, dann die Quelle; scheitert das
            // erste Schließen, wird die Quelle für eine Ausnahme
            // geschlossen.
            let inner = read(agent, scope, state, INNER);
            let iterator = read(agent, scope, state, ITERATOR);
            let backup = iterator_close(agent, scope, inner);
            close_if_abrupt(agent, scope, iterator, backup)?;
            iterator_close(agent, scope, iterator)?;
            Ok(None)
        }
        (CONCAT, Resumption::Return) => {
            let iterator = read(agent, scope, state, ITERATOR);
            iterator_close(agent, scope, iterator)?;
            Ok(None)
        }
        (ZIP, Resumption::Return) => {
            // IfAbruptCloseIterators(completion, openIters).
            close_zip(agent, state, Ok(()))?;
            Ok(None)
        }
        (_, Resumption::Return) => {
            // IfAbruptCloseIterator(completion, iterated) mit der
            // Return-Completion. Kam nach dem Ende der Quelle noch ein
            // letzter Puffer, kehrt die Closure ohne Schließen zurück.
            let iterator = read(agent, scope, state, ITERATOR);
            if is_object(agent, iterator) {
                iterator_close(agent, scope, iterator)?;
            }
            Ok(None)
        }
        (FLAT_MAP, Resumption::Next) => run(agent, scope, state, flat_map_step),
        (CONCAT, Resumption::Next) => run(agent, scope, state, concat_step),
        (ZIP, Resumption::Next) => zip_next(agent, scope, state),
        (_, Resumption::Next) => {
            if !is_object(agent, read(agent, scope, state, ITERATOR)) {
                return Ok(None);
            }
            match operation {
                MAP => run(agent, scope, state, map_step),
                FILTER => run(agent, scope, state, filter_step),
                TAKE => run(agent, scope, state, take_step),
                DROP => run(agent, scope, state, drop_step),
                CHUNKS => run(agent, scope, state, chunks_step),
                _ => run(agent, scope, state, windows_step),
            }
        }
    }
}

/// Eine Schleife der Closure: jeder Durchlauf mit einem Schritt und einem
/// eigenen Bereich, bis sie anhält oder endet.
fn run<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
    body: for<'i> fn(&mut Agent, &'i Scope, Local<'_>) -> Completion<Step<'i>>,
) -> Completion<Option<Local<'s>>> {
    loop {
        agent.spend_execution_steps(1)?;
        let step = agent.scope(|agent, inner| -> Completion<Option<Option<Local<'s>>>> {
            match body(agent, inner, state)? {
                Step::Done => Ok(Some(None)),
                Step::Continue => Ok(None),
                Step::Yield(value) => {
                    let heap = agent.heap();
                    Ok(Some(Some(heap.local(scope, heap.get(value)))))
                }
            }
        })?;
        if let Some(result) = step {
            return Ok(result);
        }
    }
}

/// Der nächste Wert der Quelle; `None`, wenn sie fertig ist.
fn step_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
) -> Completion<Option<Local<'s>>> {
    let iterator = read(agent, scope, state, ITERATOR);
    let next = read(agent, scope, state, NEXT);
    iterator_step_value(agent, scope, iterator, next)
}

/// Ruft `mapper` oder `predicate` mit dem Wert und dem Zähler auf und
/// zählt weiter; eine Ausnahme schließt die Quelle (IfAbruptCloseIterator).
/// Ab 2⁵³ bleibt der Zähler stehen, wie f64 rechnet.
fn call_with_counter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
    value: Local<'_>,
) -> Completion<Local<'s>> {
    let function = read(agent, scope, state, FUNCTION);
    let counter = number(agent, state, COUNTER);
    let index = agent.heap().local(scope, Value::number(counter));
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let result = call(agent, scope, function, undefined, &[value, index]);
    let iterator = read(agent, scope, state, ITERATOR);
    let result = close_if_abrupt(agent, scope, iterator, result)?;
    write(agent, state, COUNTER, Value::number(counter + 1.0));
    Ok(result)
}

/// Iterator.prototype.map (Abschnitt 27.1.3.3.10), Schritt 6.b.
fn map_step<'s>(agent: &mut Agent, scope: &'s Scope, state: Local<'_>) -> Completion<Step<'s>> {
    let Some(value) = step_value(agent, scope, state)? else {
        return Ok(Step::Done);
    };
    Ok(Step::Yield(call_with_counter(agent, scope, state, value)?))
}

/// Iterator.prototype.filter (Abschnitt 27.1.3.3.5), Schritt 6.b.
fn filter_step<'s>(agent: &mut Agent, scope: &'s Scope, state: Local<'_>) -> Completion<Step<'s>> {
    let Some(value) = step_value(agent, scope, state)? else {
        return Ok(Step::Done);
    };
    let selected = call_with_counter(agent, scope, state, value)?;
    if to_boolean(agent.heap(), agent.heap().get(selected)) {
        Ok(Step::Yield(value))
    } else {
        Ok(Step::Continue)
    }
}

/// Iterator.prototype.take (Abschnitt 27.1.3.3.13), Schritt 11.b.
/// *remaining* kann +∞ sein und bleibt es dann.
fn take_step<'s>(agent: &mut Agent, scope: &'s Scope, state: Local<'_>) -> Completion<Step<'s>> {
    let remaining = number(agent, state, LIMIT);
    if remaining == 0.0 {
        let iterator = read(agent, scope, state, ITERATOR);
        iterator_close(agent, scope, iterator)?;
        return Ok(Step::Done);
    }
    write(agent, state, LIMIT, Value::number(remaining - 1.0));
    match step_value(agent, scope, state)? {
        Some(value) => Ok(Step::Yield(value)),
        None => Ok(Step::Done),
    }
}

/// Iterator.prototype.drop (Abschnitt 27.1.3.3.3), Schritte 11.b und 11.c:
/// erst `remaining` Werte überspringen, dann alle weiteren liefern.
fn drop_step<'s>(agent: &mut Agent, scope: &'s Scope, state: Local<'_>) -> Completion<Step<'s>> {
    let remaining = number(agent, state, LIMIT);
    if remaining > 0.0 {
        write(agent, state, LIMIT, Value::number(remaining - 1.0));
        let iterator = read(agent, scope, state, ITERATOR);
        let next = read(agent, scope, state, NEXT);
        return match iterator_step(agent, scope, iterator, next)? {
            Some(_) => Ok(Step::Continue),
            None => Ok(Step::Done),
        };
    }
    match step_value(agent, scope, state)? {
        Some(value) => Ok(Step::Yield(value)),
        None => Ok(Step::Done),
    }
}

/// Iterator.prototype.flatMap (Abschnitt 27.1.3.3.7), Schritt 6.b ohne
/// das Schließen bei `return`.
fn flat_map_step<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
) -> Completion<Step<'s>> {
    let iterator = read(agent, scope, state, ITERATOR);
    let inner = read(agent, scope, state, INNER);
    if is_object(agent, inner) {
        let inner_next = read(agent, scope, state, INNER_NEXT);
        let value = iterator_step_value(agent, scope, inner, inner_next);
        return match close_if_abrupt(agent, scope, iterator, value)? {
            Some(value) => Ok(Step::Yield(value)),
            None => {
                write(agent, state, INNER, Value::UNDEFINED);
                write(agent, state, INNER_NEXT, Value::UNDEFINED);
                let counter = number(agent, state, COUNTER);
                write(agent, state, COUNTER, Value::number(counter + 1.0));
                Ok(Step::Continue)
            }
        };
    }
    let Some(value) = step_value(agent, scope, state)? else {
        return Ok(Step::Done);
    };
    let function = read(agent, scope, state, FUNCTION);
    let counter = number(agent, state, COUNTER);
    let index = agent.heap().local(scope, Value::number(counter));
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let mapped = call(agent, scope, function, undefined, &[value, index]);
    let mapped = close_if_abrupt(agent, scope, iterator, mapped)?;
    let realm = realm_of(agent, state);
    let inner = get_iterator_flattenable(agent, scope, realm, mapped, false);
    let (inner, inner_next) = close_if_abrupt(agent, scope, iterator, inner)?;
    write_local(agent, state, INNER, inner);
    write_local(agent, state, INNER_NEXT, inner_next);
    Ok(Step::Continue)
}

/// Iterator.prototype.chunks (Abschnitt 27.1.3.3.1), Schritt 7.b.
fn chunks_step<'s>(agent: &mut Agent, scope: &'s Scope, state: Local<'_>) -> Completion<Step<'s>> {
    let Some(value) = step_value(agent, scope, state)? else {
        if count(agent, state, LENGTH) == 0 {
            return Ok(Step::Done);
        }
        // Der letzte, kürzere Block; danach endet die Closure, ohne die
        // Quelle zu schließen.
        let chunk = take_buffer(agent, scope, state)?;
        write(agent, state, ITERATOR, Value::UNDEFINED);
        write(agent, state, NEXT, Value::UNDEFINED);
        return Ok(Step::Yield(chunk));
    };
    push(agent, scope, state, value)?;
    if count(agent, state, LENGTH) == count(agent, state, LIMIT) {
        return Ok(Step::Yield(take_buffer(agent, scope, state)?));
    }
    Ok(Step::Continue)
}

/// Iterator.prototype.windows (Abschnitt 27.1.3.3.15), Schritt 9.b.
fn windows_step<'s>(agent: &mut Agent, scope: &'s Scope, state: Local<'_>) -> Completion<Step<'s>> {
    let size = count(agent, state, LIMIT);
    let Some(value) = step_value(agent, scope, state)? else {
        let length = count(agent, state, LENGTH);
        if flag(agent, state, OPTION) && length > 0 && length < size {
            let window = buffer_array(agent, scope, state)?;
            write(agent, state, ITERATOR, Value::UNDEFINED);
            write(agent, state, NEXT, Value::UNDEFINED);
            return Ok(Step::Yield(window));
        }
        return Ok(Step::Done);
    };
    if count(agent, state, LENGTH) == size {
        shift(agent, state);
    }
    push(agent, scope, state, value)?;
    if count(agent, state, LENGTH) == size {
        return Ok(Step::Yield(buffer_array(agent, scope, state)?));
    }
    Ok(Step::Continue)
}

/// Die Zahl der Felder einer internen Zelle, 0 für andere Werte.
fn capacity(agent: &Agent, cell: Local<'_>) -> usize {
    match agent.heap().get(cell).kind() {
        Kind::Internal(cell) => agent.heap().slot_count(cell),
        _ => 0,
    }
}

/// Hängt `value` an den Ring im Feld [`BUFFER`] an. Ist er voll, wächst er
/// auf das Doppelte, höchstens auf die Größe aus [`LIMIT`].
fn push(agent: &mut Agent, scope: &Scope, state: Local<'_>, value: Local<'_>) -> Completion<()> {
    let length = count(agent, state, LENGTH);
    let mut buffer = read(agent, scope, state, BUFFER);
    let mut size = capacity(agent, buffer);
    if length >= size {
        let first = count(agent, state, FIRST);
        let limit = count(agent, state, LIMIT).max(1);
        let grown = size
            .saturating_mul(2)
            .clamp(INITIAL_CAPACITY.min(limit), limit);
        let larger = agent.heap_mut().alloc_slots(scope, grown)?;
        for offset in 0..length {
            let element = read(agent, scope, buffer, ring_index(first, offset, size));
            write_local(agent, larger, offset, element);
        }
        write_local(agent, state, BUFFER, larger);
        set_count(agent, state, FIRST, 0);
        buffer = larger;
        size = grown;
    }
    let first = count(agent, state, FIRST);
    write_local(agent, buffer, ring_index(first, length, size), value);
    set_count(agent, state, LENGTH, length.saturating_add(1));
    Ok(())
}

/// Entfernt das erste Element des Rings.
fn shift(agent: &Agent, state: Local<'_>) {
    let length = count(agent, state, LENGTH);
    if length == 0 {
        return;
    }
    let heap = agent.heap();
    let Kind::Internal(cell) = heap.get(state).kind() else {
        return;
    };
    let Some(Kind::Internal(buffer)) = heap.slot(cell, BUFFER).map(Value::kind) else {
        return;
    };
    let size = heap.slot_count(buffer);
    let first = count(agent, state, FIRST);
    heap.set_slot(buffer, first, Value::UNDEFINED);
    set_count(agent, state, FIRST, ring_index(first, 1, size));
    set_count(agent, state, LENGTH, length - 1);
}

/// Die Stelle `offset` hinter `first` in einem Ring mit `size` Feldern.
fn ring_index(first: usize, offset: usize, size: usize) -> usize {
    match first.checked_add(offset) {
        Some(index) if size > 0 => index % size,
        _ => 0,
    }
}

/// CreateArrayFromList über den Ring im Realm des Helfers.
fn buffer_array<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
) -> Completion<Local<'s>> {
    let length = count(agent, state, LENGTH);
    let first = count(agent, state, FIRST);
    let buffer = read(agent, scope, state, BUFFER);
    let size = capacity(agent, buffer);
    let realm = realm_of(agent, state);
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    let array = array_create(agent, scope, 0, Some(prototype))?;
    for offset in 0..length {
        let element = read(agent, scope, buffer, ring_index(first, offset, size));
        // Der Ring hält höchstens 2³² − 1 Werte, die Indizes bleiben
        // darunter.
        let index = u32::try_from(offset).map_err(|_| agent.throw_error(ErrorKind::RangeError))?;
        create_data_property_or_throw(agent, scope, array, PropertyKey::Index(index), element)?;
    }
    Ok(array)
}

/// [`buffer_array`] und danach ein neuer, leerer Puffer.
fn take_buffer<'s>(agent: &mut Agent, scope: &'s Scope, state: Local<'_>) -> Completion<Local<'s>> {
    let array = buffer_array(agent, scope, state)?;
    write(agent, state, BUFFER, Value::UNDEFINED);
    set_count(agent, state, LENGTH, 0);
    set_count(agent, state, FIRST, 0);
    Ok(array)
}

/// Iterator.concat (Abschnitt 27.1.3.2.1), Schritt 3 ohne das Schließen
/// bei `return`: die Quellen der Reihe nach öffnen und durchlaufen.
fn concat_step<'s>(agent: &mut Agent, scope: &'s Scope, state: Local<'_>) -> Completion<Step<'s>> {
    let iterator = read(agent, scope, state, ITERATOR);
    if is_object(agent, iterator) {
        let next = read(agent, scope, state, NEXT);
        return match iterator_step_value(agent, scope, iterator, next)? {
            Some(value) => Ok(Step::Yield(value)),
            None => {
                write(agent, state, ITERATOR, Value::UNDEFINED);
                write(agent, state, NEXT, Value::UNDEFINED);
                Ok(Step::Continue)
            }
        };
    }
    let index = count(agent, state, COUNTER);
    if index >= count(agent, state, LIMIT) {
        return Ok(Step::Done);
    }
    let iterables = read(agent, scope, state, FUNCTION);
    let base = index.saturating_mul(CONCAT_ENTRY);
    let method = read(agent, scope, iterables, base);
    let iterable = read(agent, scope, iterables, base.saturating_add(1));
    set_count(agent, state, COUNTER, index.saturating_add(1));
    let iterator = call(agent, scope, method, iterable, &[])?;
    if !is_object(agent, iterator) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let next = get_iterator_direct(agent, scope, iterator)?;
    write_local(agent, state, ITERATOR, iterator);
    write_local(agent, state, NEXT, next);
    Ok(Step::Continue)
}

/// Die Iteratoren aus *openIters* in ihrer Reihenfolge.
fn open_iterators<'s>(agent: &Agent, scope: &'s Scope, state: Local<'_>) -> Vec<Local<'s>> {
    let iterators = read(agent, scope, state, FUNCTION);
    let total = count(agent, state, LIMIT);
    let mut open = Vec::new();
    for index in 0..total {
        let base = index.saturating_mul(ZIP_ENTRY);
        if flag(agent, iterators, base.saturating_add(2)) {
            open.push(read(agent, scope, iterators, base));
        }
    }
    open
}

/// IteratorCloseAll(openIters, completion).
fn close_zip(agent: &mut Agent, state: Local<'_>, completion: Completion<()>) -> Completion<()> {
    agent.scope(|agent, inner| {
        let open = open_iterators(agent, inner, state);
        iterator_close_all(agent, inner, &open, completion)
    })
}

/// Nimmt den Iterator `index` aus *openIters*.
fn remove_open(agent: &Agent, scope: &Scope, state: Local<'_>, index: usize) {
    let iterators = read(agent, scope, state, FUNCTION);
    let slot = index.saturating_mul(ZIP_ENTRY).saturating_add(2);
    write(agent, iterators, slot, Value::boolean(false));
}

/// Die Closure von IteratorZip (Abschnitt 27.1.3.4.1), Schritt 3 ohne das
/// Schließen bei `return`: ein Durchlauf über alle Iteratoren.
fn zip_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: Local<'_>,
) -> Completion<Option<Local<'s>>> {
    let total = count(agent, state, LIMIT);
    if total == 0 {
        return Ok(None);
    }
    let mode = int(agent, state, OPTION);
    let mut results = Vec::with_capacity(total);
    for index in 0..total {
        agent.spend_execution_steps(1)?;
        let iterators = read(agent, scope, state, FUNCTION);
        let base = index.saturating_mul(ZIP_ENTRY);
        if !flag(agent, iterators, base.saturating_add(2)) {
            // Nur bei *"longest"* bleibt ein fertiger Iterator stehen.
            let padding = read(agent, scope, state, BUFFER);
            results.push(read(agent, scope, padding, index));
            continue;
        }
        let iterator = read(agent, scope, iterators, base);
        let next = read(agent, scope, iterators, base.saturating_add(1));
        let result = agent.scope(|agent, inner| {
            iterator_step_value(agent, inner, iterator, next).map(|value| {
                value.map(|value| {
                    let heap = agent.heap();
                    heap.local(scope, heap.get(value))
                })
            })
        });
        let result = match result {
            Ok(result) => result,
            Err(abrupt) => {
                remove_open(agent, scope, state, index);
                close_zip(agent, state, Err(abrupt))?;
                return Ok(None);
            }
        };
        if let Some(value) = result {
            results.push(value);
            continue;
        }
        remove_open(agent, scope, state, index);
        match mode {
            SHORTEST => {
                close_zip(agent, state, Ok(()))?;
                return Ok(None);
            }
            STRICT => {
                if index != 0 {
                    let error = Err(agent.throw_error(ErrorKind::TypeError));
                    close_zip(agent, state, error)?;
                    return Ok(None);
                }
                zip_strict_rest(agent, scope, state, total)?;
                return Ok(None);
            }
            _ => {
                if open_iterators(agent, scope, state).is_empty() {
                    return Ok(None);
                }
                let padding = read(agent, scope, state, BUFFER);
                results.push(read(agent, scope, padding, index));
            }
        }
    }
    let realm = realm_of(agent, state);
    let keys = read(agent, scope, state, INNER);
    if !matches!(agent.heap().get(keys).kind(), Kind::Internal(_)) {
        return Ok(Some(create_array_from_list(agent, scope, realm, &results)?));
    }
    // finishResults von `zipKeyed`: ein Objekt ohne Prototyp mit den
    // Schlüsseln in ihrer Reihenfolge.
    let object = ordinary_object_create(agent, scope, None)?;
    for (index, &value) in results.iter().enumerate() {
        let key = read(agent, scope, keys, index);
        let Some(key) = agent.key_from_value(scope, key) else {
            continue;
        };
        create_data_property_or_throw(agent, scope, object, key, value)?;
    }
    Ok(Some(object))
}

/// IteratorZip mit *"strict"*: Ist der erste Iterator fertig, müssen es
/// alle anderen auch sein.
fn zip_strict_rest(
    agent: &mut Agent,
    scope: &Scope,
    state: Local<'_>,
    total: usize,
) -> Completion<()> {
    for index in 1..total {
        agent.spend_execution_steps(1)?;
        let iterators = read(agent, scope, state, FUNCTION);
        let base = index.saturating_mul(ZIP_ENTRY);
        let iterator = read(agent, scope, iterators, base);
        let next = read(agent, scope, iterators, base.saturating_add(1));
        let open = agent.scope(|agent, inner| {
            iterator_step(agent, inner, iterator, next).map(|result| result.is_some())
        });
        match open {
            Err(abrupt) => {
                remove_open(agent, scope, state, index);
                return close_zip(agent, state, Err(abrupt));
            }
            Ok(false) => remove_open(agent, scope, state, index),
            Ok(true) => {
                let error = Err(agent.throw_error(ErrorKind::TypeError));
                return close_zip(agent, state, error);
            }
        }
    }
    Ok(())
}
