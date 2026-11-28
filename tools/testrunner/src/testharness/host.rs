//! Der Host für testharness.js in einer JavaScript-Shell: das globale
//! `self`, `setTimeout` und `clearTimeout` mit einer virtuellen Uhr, `gc`
//! wie in den Shells der großen Engines und `mbWpt`, über das
//! `report.js` die Ergebnisse meldet.
//!
//! Dazu kommt ein kleines `addEventListener` für die Ereignisse `error`
//! und `unhandledrejection` am globalen Objekt. Mit ihnen meldet HTML eine
//! Ausnahme, die kein Skript fing, und ein abgelehntes Promise ohne
//! Handler (HTML, „report an exception“ und „unhandled promise
//! rejections“), und testharness.js
//! hört darauf: Es setzt dann den Status der Harness auf ERROR, außer bei
//! `allow_uncaught_exception`. Andere Ereignisse merkt sich der Host
//! nicht; ein `removeEventListener` gibt es nicht.
//!
//! Die Uhr steht, solange JavaScript läuft, und springt erst dann zum
//! nächsten Zeitgeber, wenn keine Microtasks mehr warten (die
//! Ereignisschleife in `testharness.rs`). `Date.now` folgt ihr. So ist jeder Lauf
//! gleich und dauert nicht länger als die Arbeit selbst.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mb_js::agent::{Agent, Completion, ErrorKind};
use mb_js::builtin::{Builtin, BuiltinCall, define_builtin_function};
use mb_js::compare::same_value;
use mb_js::conversion::{to_number, to_string};
use mb_js::descriptor::PropertyDescriptor;
use mb_js::heap::{Local, Persistent, Scope};
use mb_js::host::{Host, RejectionOperation, system_time};
use mb_js::object::ordinary_object_create;
use mb_js::operations::{
    call, create_data_property_or_throw, define_property_or_throw, is_callable,
};
use mb_js::realm::{Intrinsic, RealmId};
use mb_js::value::{Kind, Value};

use super::{HarnessStatus, Status, Subtest};

/// Ein Zeitgeber aus `setTimeout`.
pub(super) struct Timer {
    pub(super) id: i32,
    /// Wann er abläuft, in Millisekunden der virtuellen Uhr.
    pub(super) due: u64,
    /// Die Reihenfolge des Anlegens; bei gleicher Zeit läuft der ältere
    /// zuerst.
    pub(super) sequence: u64,
    pub(super) callback: Persistent,
    pub(super) arguments: Vec<Persistent>,
}

/// Die Ereignisse, die der Host weitergibt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Event {
    Error,
    UnhandledRejection,
}

impl Event {
    fn from_name(name: &str) -> Option<Event> {
        match name {
            "error" => Some(Event::Error),
            "unhandledrejection" => Some(Event::UnhandledRejection),
            _ => None,
        }
    }
}

/// Was `report.js` am Ende meldet.
pub(super) struct Completed {
    pub(super) status: HarnessStatus,
    pub(super) message: Option<String>,
    pub(super) subtests: Vec<Subtest>,
}

/// Der Zustand eines Laufs. Ausgeliehen wird er nur, solange kein
/// JavaScript läuft (RS-10).
pub(super) struct ShellHost {
    /// Die Zeit des Systems zu Beginn, in Millisekunden seit 1970.
    base: f64,
    /// Die virtuelle Uhr in Millisekunden seit Beginn.
    now: Cell<u64>,
    timers: RefCell<Vec<Timer>>,
    next_id: Cell<i32>,
    next_sequence: Cell<u64>,
    /// Die Untertests, die `add_result_callback` gemeldet hat.
    finished: RefCell<Vec<Subtest>>,
    /// Die Untertests aus `add_completion_callback`, bis `complete` sie
    /// mit dem Status der Harness abschließt.
    last: RefCell<Vec<Subtest>>,
    completed: RefCell<Option<Completed>>,
    /// Die Listener aus `addEventListener` in ihrer Reihenfolge.
    listeners: RefCell<Vec<(Event, Persistent)>>,
    /// Abgelehnte Promises ohne Handler, bis zum Ende der nächsten
    /// Microtasks (HTML: „about-to-be-notified rejected promises list“).
    rejected: RefCell<Vec<Persistent>>,
}

impl ShellHost {
    pub(super) fn new() -> ShellHost {
        ShellHost {
            base: system_time(),
            now: Cell::new(0),
            timers: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
            next_sequence: Cell::new(0),
            finished: RefCell::new(Vec::new()),
            last: RefCell::new(Vec::new()),
            completed: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
            rejected: RefCell::new(Vec::new()),
        }
    }

    /// Die Listener für `event`, als neue Handles in `scope`.
    pub(super) fn listeners<'s>(
        &self,
        agent: &Agent,
        scope: &'s Scope,
        event: Event,
    ) -> Vec<Local<'s>> {
        let heap = agent.heap();
        self.listeners
            .borrow()
            .iter()
            .filter(|(found, _)| *found == event)
            .map(|(_, listener)| heap.local(scope, heap.get_persistent(listener)))
            .collect()
    }

    /// Nimmt die Promises, die seit dem letzten Mal ohne Handler abgelehnt
    /// wurden und noch keinen bekamen.
    pub(super) fn take_rejected(&self) -> Vec<Persistent> {
        std::mem::take(&mut *self.rejected.borrow_mut())
    }

    /// Gibt alle übrigen Handles frei.
    pub(super) fn release_all(&self, agent: &mut Agent) {
        for timer in self.take_timers() {
            release(agent, timer);
        }
        for (_, listener) in std::mem::take(&mut *self.listeners.borrow_mut()) {
            agent.heap_mut().release(listener);
        }
        for promise in self.take_rejected() {
            agent.heap_mut().release(promise);
        }
    }

    pub(super) fn now(&self) -> u64 {
        self.now.get()
    }

    /// Stellt die Uhr vor, nie zurück.
    pub(super) fn advance(&self, to: u64) {
        self.now.set(self.now.get().max(to));
    }

    /// Nimmt den Zeitgeber, der als nächster abläuft.
    pub(super) fn take_next_timer(&self) -> Option<Timer> {
        let mut timers = self.timers.borrow_mut();
        let index = timers
            .iter()
            .enumerate()
            .min_by_key(|(_, timer)| (timer.due, timer.sequence))
            .map(|(index, _)| index)?;
        Some(timers.swap_remove(index))
    }

    /// Nimmt alle Zeitgeber, etwa am Ende eines Laufs.
    pub(super) fn take_timers(&self) -> Vec<Timer> {
        std::mem::take(&mut *self.timers.borrow_mut())
    }

    pub(super) fn is_completed(&self) -> bool {
        self.completed.borrow().is_some()
    }

    pub(super) fn take_completed(&self) -> Option<Completed> {
        self.completed.borrow_mut().take()
    }

    pub(super) fn take_finished(&self) -> Vec<Subtest> {
        std::mem::take(&mut *self.finished.borrow_mut())
    }
}

impl Host for ShellHost {
    fn define_global_properties(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        realm: RealmId,
    ) -> Completion<()> {
        let global = agent.global_object(scope, realm);
        define(agent, scope, global, "self", global)?;
        for builtin in &GLOBAL_FUNCTIONS {
            define_builtin_function(agent, scope, realm, global, builtin)?;
        }
        let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
        let report = ordinary_object_create(agent, scope, Some(prototype))?;
        for builtin in &REPORT_FUNCTIONS {
            define_builtin_function(agent, scope, realm, report, builtin)?;
        }
        define(agent, scope, global, "mbWpt", report)
    }

    /// HostPromiseRejectionTracker wie in HTML: Ein
    /// abgelehntes Promise ohne Handler kommt auf die Liste, ein Handler
    /// nimmt es wieder herunter. Promises, die schon gemeldet wurden,
    /// verfolgt der Host nicht weiter; `rejectionhandled` gibt es nicht.
    fn promise_rejection_tracker(
        &self,
        agent: &mut Agent,
        _: &Scope,
        promise: Local<'_>,
        operation: RejectionOperation,
    ) {
        match operation {
            RejectionOperation::Reject => {
                let handle = agent.heap_mut().persist(promise);
                self.rejected.borrow_mut().push(handle);
            }
            RejectionOperation::Handle => {
                let removed = {
                    let heap = agent.heap();
                    let mut rejected = self.rejected.borrow_mut();
                    rejected
                        .iter()
                        .position(|found| {
                            same_value(heap, heap.get_persistent(found), heap.get(promise))
                        })
                        .map(|index| rejected.remove(index))
                };
                if let Some(handle) = removed {
                    agent.heap_mut().release(handle);
                }
            }
        }
    }

    fn current_time(&self) -> f64 {
        // Die virtuelle Uhr bleibt unter der Zeitgrenze eines Laufs, weit
        // unter 2⁵³ Millisekunden; die Umwandlung ist darum genau.
        self.base + self.now.get() as f64
    }
}

static GLOBAL_FUNCTIONS: [Builtin; 4] = [
    Builtin {
        name: "setTimeout",
        length: 1,
        constructor: false,
        behaviour: set_timeout,
    },
    Builtin {
        name: "clearTimeout",
        length: 0,
        constructor: false,
        behaviour: clear_timeout,
    },
    Builtin {
        name: "gc",
        length: 0,
        constructor: false,
        behaviour: gc,
    },
    Builtin {
        name: "addEventListener",
        length: 2,
        constructor: false,
        behaviour: add_event_listener,
    },
];

static REPORT_FUNCTIONS: [Builtin; 3] = [
    Builtin {
        name: "result",
        length: 3,
        constructor: false,
        behaviour: report_result,
    },
    Builtin {
        name: "last",
        length: 3,
        constructor: false,
        behaviour: report_last,
    },
    Builtin {
        name: "complete",
        length: 2,
        constructor: false,
        behaviour: report_complete,
    },
];

/// Eine beschreibbare, konfigurierbare, nicht aufzählbare Eigenschaft.
fn define(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: &str,
    value: Local<'_>,
) -> Completion<()> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let key = agent.key_from_units(scope, &units)?;
    let descriptor = PropertyDescriptor::data(value, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// Der Host des Agents; ohne ihn wirft die Funktion einen TypeError.
fn shell_host(agent: &mut Agent) -> Completion<Rc<ShellHost>> {
    let host = agent.host().and_then(|host| -> Option<Rc<ShellHost>> {
        let host: Rc<dyn Any> = host;
        host.downcast::<ShellHost>().ok()
    });
    host.ok_or_else(|| agent.throw_error(ErrorKind::TypeError))
}

/// Die Umwandlung in den WebIDL-Typ `long` (ConvertToInt in WebIDL, ohne
/// `[EnforceRange]` und `[Clamp]`): ToNumber, dann
/// abgeschnitten und modulo 2³² in den Bereich mit Vorzeichen.
fn to_long(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<i32> {
    let number = to_number(agent, scope, value)?;
    if !number.is_finite() {
        return Ok(0);
    }
    let modulo = number.trunc().rem_euclid(4_294_967_296.0);
    // `modulo` liegt in [0, 2³²) und ist ganz, beide Umwandlungen sind
    // darum genau.
    let unsigned = modulo as u32;
    Ok(i32::from_ne_bytes(unsigned.to_ne_bytes()))
}

/// `setTimeout(handler, timeout, ...arguments)` nach den „timer
/// initialization steps“ von HTML, für eine Funktion als `handler`. Eine
/// Zeichenkette als Handler gibt es in der Shell nicht; sie wirft einen
/// TypeError. Verschachtelte Zeitgeber werden nicht auf 4 ms angehoben,
/// denn die Uhr ist virtuell.
fn set_timeout<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let handler = call.argument(agent, scope, 0);
    if !is_callable(agent.heap(), agent.heap().get(handler)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let timeout = call.argument(agent, scope, 1);
    let timeout = to_long(agent, scope, timeout)?.max(0);
    let host = shell_host(agent)?;
    let id = host.next_id.get();
    let Some(next) = id.checked_add(1) else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    host.next_id.set(next);
    let sequence = host.next_sequence.get();
    host.next_sequence.set(sequence.saturating_add(1));
    let delay = u64::try_from(timeout).unwrap_or(0);
    let callback = agent.heap_mut().persist(handler);
    let arguments = call
        .arguments
        .iter()
        .skip(2)
        .map(|argument| agent.heap_mut().persist(*argument))
        .collect();
    host.timers.borrow_mut().push(Timer {
        id,
        due: host.now().saturating_add(delay),
        sequence,
        callback,
        arguments,
    });
    Ok(agent.heap().local(scope, Value::int32(id)))
}

/// `clearTimeout(id)`: entfernt den Zeitgeber, falls er noch wartet.
fn clear_timeout<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let id = call.argument(agent, scope, 0);
    let id = to_long(agent, scope, id)?;
    let host = shell_host(agent)?;
    let removed = {
        let mut timers = host.timers.borrow_mut();
        timers
            .iter()
            .position(|timer| timer.id == id)
            .map(|index| timers.swap_remove(index))
    };
    if let Some(timer) = removed {
        release(agent, timer);
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// `addEventListener(type, callback)`: merkt sich eine Funktion für
/// `error` oder `unhandledrejection`. Wie in DOM kommt dieselbe Funktion
/// für dasselbe Ereignis nur einmal hinzu; die Optionen im dritten
/// Argument zählen nicht.
fn add_event_listener<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let name = call.argument(agent, scope, 0);
    let name = text(agent, scope, name)?;
    let callback = call.argument(agent, scope, 1);
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let Some(event) = Event::from_name(&name) else {
        return Ok(undefined);
    };
    if !is_callable(agent.heap(), agent.heap().get(callback)) {
        return Ok(undefined);
    }
    let host = shell_host(agent)?;
    let known = {
        let heap = agent.heap();
        host.listeners.borrow().iter().any(|(found, listener)| {
            *found == event && same_value(heap, heap.get_persistent(listener), heap.get(callback))
        })
    };
    if !known {
        let handle = agent.heap_mut().persist(callback);
        host.listeners.borrow_mut().push((event, handle));
    }
    Ok(undefined)
}

/// Gibt die Handles eines Zeitgebers frei.
pub(super) fn release(agent: &mut Agent, timer: Timer) {
    agent.heap_mut().release(timer.callback);
    for argument in timer.arguments {
        agent.heap_mut().release(argument);
    }
}

/// `gc()`: eine volle Sammlung, wie `gc()` in den Shells von V8 und
/// SpiderMonkey. `/common/gc.js` der WPT nimmt sie, wenn es kein
/// `TestUtils.gc()` gibt.
fn gc<'s>(agent: &mut Agent, scope: &'s Scope, _: &BuiltinCall<'_, '_>) -> Completion<Local<'s>> {
    agent.heap_mut().collect();
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Name, Status und Meldung eines Untertests aus den Argumenten.
fn subtest(agent: &mut Agent, scope: &Scope, call: &BuiltinCall<'_, '_>) -> Completion<Subtest> {
    let name = call.argument(agent, scope, 0);
    let name = text(agent, scope, name)?;
    let status = call.argument(agent, scope, 1);
    let status = to_number(agent, scope, status)?;
    let message = call.argument(agent, scope, 2);
    let message = optional_text(agent, scope, message)?;
    let status =
        Status::from_number(status).ok_or_else(|| agent.throw_error(ErrorKind::RangeError))?;
    Ok(Subtest {
        name,
        status,
        message,
    })
}

/// `mbWpt.result(name, status, message)` aus `add_result_callback`.
fn report_result<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let subtest = subtest(agent, scope, call)?;
    shell_host(agent)?.finished.borrow_mut().push(subtest);
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// `mbWpt.last(name, status, message)` für jeden Test aus
/// `add_completion_callback`.
fn report_last<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let subtest = subtest(agent, scope, call)?;
    shell_host(agent)?.last.borrow_mut().push(subtest);
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// `mbWpt.complete(status, message)`: der Status der Harness. Danach
/// zählt nichts mehr, was noch kommt.
fn report_complete<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let status = call.argument(agent, scope, 0);
    let status = to_number(agent, scope, status)?;
    let message = call.argument(agent, scope, 1);
    let message = optional_text(agent, scope, message)?;
    let status = HarnessStatus::from_number(status)
        .ok_or_else(|| agent.throw_error(ErrorKind::RangeError))?;
    let host = shell_host(agent)?;
    let subtests = std::mem::take(&mut *host.last.borrow_mut());
    let mut completed = host.completed.borrow_mut();
    if completed.is_none() {
        *completed = Some(Completed {
            status,
            message,
            subtests,
        });
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// ToString(`value`) als Rust-Zeichenkette; einzelne Surrogate werden zu
/// U+FFFD.
pub(super) fn text(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<String> {
    let string = to_string(agent, scope, value)?;
    let heap = agent.heap();
    let units = match heap.get(string).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => Vec::new(),
    };
    Ok(String::from_utf16_lossy(&units))
}

/// Wie [`text`], aber *undefined* und *null* werden zu `None`.
fn optional_text(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<Option<String>> {
    let found = agent.heap().get(value);
    if found.is_undefined() || found.is_null() {
        return Ok(None);
    }
    text(agent, scope, value).map(Some)
}

/// Ein neues Fehlerobjekt der Art `kind` aus `realm`, wie es ein `catch`
/// für einen Fehler der Engine bekäme: der Konstruktor, als Funktion
/// gerufen, ohne Meldung.
pub(super) fn engine_error<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    kind: ErrorKind,
) -> Completion<Local<'s>> {
    let constructor = match kind {
        ErrorKind::RangeError => Intrinsic::RangeError,
        ErrorKind::ReferenceError => Intrinsic::ReferenceError,
        ErrorKind::SyntaxError => Intrinsic::SyntaxError,
        ErrorKind::TypeError => Intrinsic::TypeError,
    };
    let constructor = agent.intrinsic(scope, realm, constructor);
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    call(agent, scope, constructor, undefined, &[])
}

/// Ein Objekt mit den Feldern von ErrorEvent, die testharness.js liest:
/// `type`, `message`, `error`, `filename`, `lineno` und `colno`. Datei und
/// Lage kennt der Läufer nicht; sie bleiben leer und 0.
pub(super) fn error_event<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    message: &str,
    error: Local<'_>,
) -> Completion<Local<'s>> {
    let event = event_object(agent, scope, realm, "error")?;
    let message = string(agent, scope, message)?;
    field(agent, scope, event, "message", message)?;
    field(agent, scope, event, "error", error)?;
    let filename = string(agent, scope, "")?;
    field(agent, scope, event, "filename", filename)?;
    let zero = agent.heap().local(scope, Value::int32(0));
    field(agent, scope, event, "lineno", zero)?;
    field(agent, scope, event, "colno", zero)?;
    Ok(event)
}

/// Ein Objekt mit den Feldern von PromiseRejectionEvent: `type`,
/// `promise` und `reason`.
pub(super) fn rejection_event<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    promise: Local<'_>,
    reason: Local<'_>,
) -> Completion<Local<'s>> {
    let event = event_object(agent, scope, realm, "unhandledrejection")?;
    field(agent, scope, event, "promise", promise)?;
    field(agent, scope, event, "reason", reason)?;
    Ok(event)
}

/// Ein gewöhnliches Objekt mit `type`.
fn event_object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    name: &str,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let event = ordinary_object_create(agent, scope, Some(prototype))?;
    let name = string(agent, scope, name)?;
    field(agent, scope, event, "type", name)?;
    Ok(event)
}

fn string<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> Completion<Local<'s>> {
    let units: Vec<u16> = text.encode_utf16().collect();
    Ok(agent.heap_mut().alloc_string(scope, &units)?)
}

fn field(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: &str,
    value: Local<'_>,
) -> Completion<()> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let key = agent.key_from_units(scope, &units)?;
    create_data_property_or_throw(agent, scope, object, key, value)
}
