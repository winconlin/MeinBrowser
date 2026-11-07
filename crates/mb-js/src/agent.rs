//! Der Agent (Abschnitt 9.6): Heap mit dem Registerstapel, Atome, die
//! Wurzel aller Gestalten, die bekannten Symbole, die Realms, die
//! gemeinsamen Daten der Funktionen, die Einträge der eingebauten
//! Funktionen, der Host, die Ausnahme, die gerade geworfen wird, und die
//! Warteschlange der Jobs. Das Symbolregister kommt mit `Symbol.for`.
//!
//! Jobs (Abschnitt 9.5) gibt es bisher nur für Promises. HostEnqueuePromiseJob
//! (Abschnitt 9.5.5) hängt sie hinten an die Warteschlange; der Host lässt
//! sie mit [`Agent::run_jobs`] laufen, wenn kein Script läuft, und sie
//! laufen in der Reihenfolge, in der sie kamen. Jeder Job ist eine interne
//! Zelle in einem dauerhaften Handle, also eine Wurzel, bis er läuft.
//!
//! Atome sind die Zeichenketten, die als Eigenschaftsschlüssel dienen. Zu
//! jedem Inhalt gibt es höchstens eines, also vergleicht man Schlüssel über
//! ihren Abstand. Atome bleiben vorerst, solange der Agent lebt.
//!
//! Aufrufe zwischen JavaScript-Funktionen brauchen keinen Rust-Stapel. Ruft
//! aber Rust-Code eine Funktion auf, etwa einen Getter, beginnt ein neuer
//! Lauf des Interpreters, und solche Läufe schachteln sich auf dem
//! Rust-Stapel. Der Agent merkt sich, wo der Stapel beim äußersten Lauf
//! stand, und lässt verschachtelte Läufe nur [`STACK_BUDGET`] Bytes tiefer
//! beginnen; danach wirft ein weiterer einen RangeError (RS-8).

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

pub use mb_base::{Budget, Cancel, Exhausted};

use crate::builtin::Builtin;
use crate::function::SharedFunction;
use crate::heap::{Heap, Local, Options, OutOfMemory, Persistent, Scope};
use crate::host::Host;
use crate::key::WellKnownSymbol;
use crate::object::{error_create, shape};
use crate::promise::run_job;
use crate::realm::{Intrinsic, Realm, RealmId};
use crate::value::{Kind, Value};

/// So viele Bytes Rust-Stapel dürfen verschachtelte Läufe des Interpreters
/// unter dem äußersten belegen. Ein Faden mit 1 MiB Stapel behält dahinter
/// Platz für den letzten Lauf und das Übersetzen einer Funktion bei ihrem
/// ersten Aufruf; das prüfen die Tests im Debug-Build. Gemessen reichte der
/// Platz dort bis zu einem Budget von 640 KiB, ein Lauf braucht etwa
/// 13 KiB. Der Rest bleibt für den Code, der das Script startet.
pub const STACK_BUDGET: usize = 384 * 1024;

/// Standardquote eines äußersten Script- oder Funktionsaufrufs. Ein Schritt
/// fällt vor jeder Script-Auswertung, bei jedem Interpreterlauf-Einstieg
/// und vor jedem Bytecodebefehl an. Parser, Compiler und die Arbeit eines
/// einzelnen Befehls sind damit nicht vollständig begrenzt.
pub const DEFAULT_EXECUTION_STEPS: u64 = 1_000_000;

/// Eine abrupte Beendigung, die Rust-Code mit `?` nach oben reicht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abrupt {
    /// Eine Ausnahme (Abschnitt 6.2.4, Art *throw*). Den geworfenen Wert
    /// hält der Agent, siehe [`Agent::take_exception`].
    Throw,
    /// Das Speicherbudget ist verbraucht. Kein Skript kann das abfangen.
    OutOfMemory,
    /// Der Übersetzer kann eine Funktion noch nicht übersetzen, die eben
    /// das erste Mal aufgerufen wird; die Zahl ist die Lage in ihrem
    /// Quelltext (in Codeeinheiten). Funktionen werden erst bei ihrem
    /// ersten Aufruf übersetzt, darum zeigt sich das zur Laufzeit. Kein
    /// Skript kann das abfangen.
    Unsupported(usize),
    /// Die Arbeitsquote ist verbraucht oder der Host hat abgebrochen.
    /// Das ist keine JavaScript-Ausnahme und kein Skript kann es abfangen.
    Exhausted(Exhausted),
}

impl From<OutOfMemory> for Abrupt {
    fn from(_: OutOfMemory) -> Abrupt {
        Abrupt::OutOfMemory
    }
}

/// Das Ergebnis einer Operation, die abrupt enden kann.
pub type Completion<T> = Result<T, Abrupt>;

/// Die Fehlerarten aus Abschnitt 20.5.5, die die Engine selbst wirft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    RangeError,
    ReferenceError,
    SyntaxError,
    TypeError,
}

impl ErrorKind {
    const ALL: [ErrorKind; 4] = [
        ErrorKind::RangeError,
        ErrorKind::ReferenceError,
        ErrorKind::SyntaxError,
        ErrorKind::TypeError,
    ];

    /// Die Nummer der Art in einer Zelle für [`Agent::rethrow`].
    fn code(self) -> i32 {
        match self {
            ErrorKind::RangeError => 0,
            ErrorKind::ReferenceError => 1,
            ErrorKind::SyntaxError => 2,
            ErrorKind::TypeError => 3,
        }
    }

    fn from_code(code: i32) -> Option<ErrorKind> {
        ErrorKind::ALL.into_iter().find(|kind| kind.code() == code)
    }
}

/// Die geworfene Ausnahme.
#[derive(Clone, Copy, Debug)]
pub enum Exception<'s> {
    /// Ein Fehler der Engine, den kein `catch` gefangen hat. Erst ein
    /// Handler für `catch` macht daraus ein Fehlerobjekt.
    Error(ErrorKind),
    Value(Local<'s>),
}

enum Pending {
    /// Ein Fehler der Engine; sobald der Interpreter ihn sieht, mit dem
    /// Realm des Rahmens, dessen Befehl ihn warf. Dort entsteht sein
    /// Fehlerobjekt.
    Error(ErrorKind, Option<RealmId>),
    Value(Persistent),
}

/// Ein Job in der Warteschlange: seine Zelle und sein Realm, *null* als
/// `None` (Abschnitt 9.5).
struct Job {
    cell: Persistent,
    realm: Option<RealmId>,
}

pub struct Agent {
    heap: Heap,
    /// Die Atome nach ihren Codeeinheiten.
    atoms: BTreeMap<Vec<u16>, Persistent>,
    /// Die Gestalt ohne Eigenschaften, Wurzel aller Gestalten.
    empty_shape: Persistent,
    /// Die bekannten Symbole, die alle Realms teilen.
    symbols: BTreeMap<WellKnownSymbol, Persistent>,
    pub(crate) realms: Vec<Realm>,
    /// Die gemeinsamen Daten der Funktionen, nach ihrer Nummer.
    pub(crate) functions: Vec<SharedFunction>,
    /// Die Einträge der eingebauten Funktionen, nach ihrer Nummer.
    pub(crate) builtins: Vec<&'static Builtin>,
    /// Die Nummern der Einträge nach ihrer Adresse.
    pub(crate) builtin_numbers: BTreeMap<usize, i32>,
    /// Der Host, der neue Realms um seine globalen Eigenschaften ergänzt.
    host: Option<Rc<dyn Host>>,
    exception: Option<Pending>,
    /// Wie viele Läufe des Interpreters gerade ineinander laufen.
    runs: usize,
    /// Die Lage des Rust-Stapels beim äußersten Lauf.
    stack_start: usize,
    /// Alle aktiven Hostquoten. Verschachtelte Aufrufe belasten sie
    /// gemeinsam; eine zusätzliche Quote ersetzt keine äußere.
    execution_budgets: Vec<Rc<Budget>>,
    /// Die Warteschlange der Jobs.
    jobs: VecDeque<Job>,
}

impl Agent {
    pub fn new() -> Result<Agent, OutOfMemory> {
        Agent::with_options(Options::default())
    }

    pub fn with_options(options: Options) -> Result<Agent, OutOfMemory> {
        let mut heap = Heap::with_options(options);
        let empty_shape = heap.scope(|heap, scope| {
            shape::alloc_empty(heap, scope).map(|shape| heap.persist(shape))
        })?;
        let mut agent = Agent {
            heap,
            atoms: BTreeMap::new(),
            empty_shape,
            symbols: BTreeMap::new(),
            realms: Vec::new(),
            functions: Vec::new(),
            builtins: Vec::new(),
            builtin_numbers: BTreeMap::new(),
            host: None,
            exception: None,
            runs: 0,
            stack_start: 0,
            execution_budgets: Vec::new(),
            jobs: VecDeque::new(),
        };
        agent.scope(|agent, scope| -> Result<(), OutOfMemory> {
            for symbol in WellKnownSymbol::ALL {
                let units: Vec<u16> = symbol.description().encode_utf16().collect();
                let description = agent.heap.alloc_string(scope, &units)?;
                let value = agent.new_symbol(scope, Some(description))?;
                let handle = agent.heap.persist(value);
                agent.symbols.insert(symbol, handle);
            }
            Ok(())
        })?;
        Ok(agent)
    }

    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    pub fn heap_mut(&mut self) -> &mut Heap {
        &mut self.heap
    }

    /// Setzt den Host. Er gilt für die Realms, die danach entstehen.
    pub fn set_host(&mut self, host: Rc<dyn Host>) {
        self.host = Some(host);
    }

    /// Der Host, wenn es einen gibt.
    pub fn host(&self) -> Option<Rc<dyn Host>> {
        self.host.clone()
    }

    /// Die Realms und zugleich der Heap, in dem die Werte ihrer Bindungen
    /// liegen.
    pub(crate) fn realms_and_heap(&mut self) -> (&[Realm], &mut Heap) {
        (&self.realms, &mut self.heap)
    }

    /// Führt `f` in einem neuen Bereich für Handles aus, wie
    /// [`Heap::scope`].
    pub fn scope<R, F: FnOnce(&mut Agent, &Scope) -> R>(&mut self, f: F) -> R {
        let scope = self.heap.enter_scope();
        let result = f(self, &scope);
        self.heap.leave_scope(&scope);
        result
    }

    /// Führt `f` unter einer zusätzlichen Arbeitsquote aus. Jeder Schritt
    /// belastet alle aktiven Quoten; dasselbe `Rc` zählt dabei nur einmal.
    /// Nach normaler oder abrupter Rückkehr gilt wieder der vorherige
    /// Quotenstapel. Der Verbrauch im übergebenen Budget bleibt erhalten,
    /// sodass ein Host dieselbe Quote über mehrere Scripts teilen kann.
    ///
    /// Dies begrenzt die Script- und Interpreter-Einstiege sowie
    /// Bytecodebefehle, nicht die Laufzeit jeder einzelnen Hostoperation.
    pub fn with_execution_budget<R>(
        &mut self,
        budget: Rc<Budget>,
        f: impl FnOnce(&mut Agent) -> R,
    ) -> R {
        let previous = self.execution_budgets.len();
        if !self
            .execution_budgets
            .iter()
            .any(|active| Rc::ptr_eq(active, &budget))
        {
            self.execution_budgets.push(budget);
        }
        let result = f(self);
        self.execution_budgets.truncate(previous);
        result
    }

    /// Ein äußerster Aufruf erhält eine endliche Standardquote. Direkte
    /// JS-Aufrufe und native Wiedereintritte behalten bestehende Quoten.
    pub(crate) fn with_default_execution_budget<R>(
        &mut self,
        f: impl FnOnce(&mut Agent) -> R,
    ) -> R {
        if self.execution_budgets.is_empty() {
            self.with_execution_budget(Rc::new(Budget::new(DEFAULT_EXECUTION_STEPS)), f)
        } else {
            f(self)
        }
    }

    /// Auch null Schritte fragen ein Abbruchsignal ab. Bei mehreren
    /// Quoten kann eine frühere Quote den Versuch bereits bezahlt haben,
    /// wenn eine spätere ihn ablehnt. Kein Befehl läuft danach weiter.
    pub(crate) fn spend_execution_steps(&self, steps: u64) -> Completion<()> {
        assert!(
            !self.execution_budgets.is_empty(),
            "Ausführung braucht eine Arbeitsquote"
        );
        for budget in &self.execution_budgets {
            if !budget.spend(steps) {
                return Err(Abrupt::Exhausted(
                    budget.exhausted().expect("abgelehnte Arbeitsquote"),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn empty_shape<'s>(&self, scope: &'s Scope) -> Local<'s> {
        let shape = self.heap.get_persistent(&self.empty_shape);
        self.heap.local(scope, shape)
    }

    /// Das bekannte Symbol `symbol` (Abschnitt 6.1.5.1).
    pub fn well_known_symbol<'s>(&self, scope: &'s Scope, symbol: WellKnownSymbol) -> Local<'s> {
        // Der Agent legt alle beim Start an.
        let value = self
            .symbols
            .get(&symbol)
            .map_or(Value::UNDEFINED, |handle| self.heap.get_persistent(handle));
        self.heap.local(scope, value)
    }

    // Atome

    /// Das Atom mit den Codeeinheiten `units`.
    pub fn atom<'s>(&mut self, scope: &'s Scope, units: &[u16]) -> Result<Local<'s>, OutOfMemory> {
        if let Some(handle) = self.atoms.get(units) {
            let atom = self.heap.get_persistent(handle);
            return Ok(self.heap.local(scope, atom));
        }
        let string = self.heap.alloc_string(scope, units)?;
        if let Kind::String(reference) = self.heap.get(string).kind() {
            self.heap.mark_atom(reference);
        }
        let handle = self.heap.persist(string);
        self.atoms.insert(units.to_vec(), handle);
        Ok(string)
    }

    /// Das Atom mit dem Inhalt der Zeichenkette `string`. Gibt es noch
    /// keines, wird `string` selbst zum Atom. Ein Wert, der keine
    /// Zeichenkette ist, kommt unverändert zurück.
    pub fn atomize<'s>(&mut self, scope: &'s Scope, string: Local<'s>) -> Local<'s> {
        let Kind::String(reference) = self.heap.get(string).kind() else {
            return string;
        };
        if self.heap.is_atom(reference) {
            return string;
        }
        let units = self.heap.string_units(reference);
        if let Some(handle) = self.atoms.get(&units) {
            let atom = self.heap.get_persistent(handle);
            return self.heap.local(scope, atom);
        }
        self.heap.mark_atom(reference);
        let handle = self.heap.persist(string);
        self.atoms.insert(units, handle);
        string
    }

    // Jobs

    /// HostEnqueuePromiseJob (Abschnitt 9.5.5): Der Job in der internen
    /// Zelle `cell` kommt hinten in die Warteschlange.
    pub(crate) fn enqueue_promise_job(&mut self, cell: Local<'_>, realm: Option<RealmId>) {
        let cell = self.heap.persist(cell);
        self.jobs.push_back(Job { cell, realm });
    }

    /// Ob Jobs warten.
    pub fn has_pending_jobs(&self) -> bool {
        !self.jobs.is_empty()
    }

    /// Führt die wartenden Jobs aus, auch die, die dabei dazukommen, bis
    /// die Warteschlange leer ist. Jeder Job kostet einen Arbeitsschritt;
    /// ohne Quote des Hosts teilen sich alle die Standardquote eines
    /// äußersten Aufrufs. Endet ein Job abrupt, kommt das zurück, und die
    /// übrigen warten weiter; bei [`Abrupt::Throw`] hält der Agent die
    /// Ausnahme wie nach einem Script.
    ///
    /// Jobs laufen nur, wenn kein Script und kein anderer Job läuft
    /// (Abschnitt 9.5); ruft eine Funktion des Hosts dies aus JavaScript
    /// auf, geschieht nichts.
    pub fn run_jobs(&mut self) -> Completion<()> {
        if self.runs > 0 {
            return Ok(());
        }
        self.with_default_execution_budget(|agent| {
            while !agent.jobs.is_empty() {
                agent.spend_execution_steps(1)?;
                let Some(job) = agent.jobs.pop_front() else {
                    break;
                };
                agent.scope(|agent, scope| {
                    let heap = agent.heap();
                    let cell = heap.local(scope, heap.get_persistent(&job.cell));
                    agent.heap_mut().release(job.cell);
                    run_job(agent, scope, cell, job.realm)
                })?;
            }
            Ok(())
        })
    }

    // Läufe des Interpreters

    /// Beginnt einen Lauf des Interpreters. Liegt er mehr als
    /// [`STACK_BUDGET`] Bytes unter dem äußersten, wirft das einen
    /// RangeError. Jeder Lauf, der beginnt, endet mit
    /// [`Agent::leave_interpreter`], auch abrupt.
    pub(crate) fn enter_interpreter(&mut self) -> Completion<()> {
        self.spend_execution_steps(1)?;
        let marker = 0u8;
        let here = std::hint::black_box(std::ptr::from_ref(&marker)).addr();
        if self.runs == 0 {
            self.stack_start = here;
        } else if here.abs_diff(self.stack_start) > STACK_BUDGET {
            return Err(self.throw_error(ErrorKind::RangeError));
        }
        self.runs = self.runs.saturating_add(1);
        Ok(())
    }

    pub(crate) fn leave_interpreter(&mut self) {
        self.runs = self.runs.saturating_sub(1);
    }

    // Ausnahmen

    /// Wirft einen Fehler der Art `kind` (ThrowCompletion, Abschnitt
    /// 6.2.4.2) und gibt [`Abrupt::Throw`] für `Err` zurück.
    pub(crate) fn throw_error(&mut self, kind: ErrorKind) -> Abrupt {
        self.set_exception(Pending::Error(kind, None));
        Abrupt::Throw
    }

    /// Wirft `value`.
    pub fn throw_value(&mut self, value: Local<'_>) -> Abrupt {
        let handle = self.heap.persist(value);
        self.set_exception(Pending::Value(handle));
        Abrupt::Throw
    }

    fn set_exception(&mut self, pending: Pending) {
        if let Some(Pending::Value(old)) = self.exception.replace(pending) {
            self.heap.release(old);
        }
    }

    /// Nimmt die geworfene Ausnahme heraus, nachdem eine Operation mit
    /// [`Abrupt::Throw`] endete.
    pub fn take_exception<'s>(&mut self, scope: &'s Scope) -> Option<Exception<'s>> {
        match self.exception.take()? {
            Pending::Error(kind, _) => Some(Exception::Error(kind)),
            Pending::Value(handle) => {
                let value = self.heap.get_persistent(&handle);
                let local = self.heap.local(scope, value);
                self.heap.release(handle);
                Some(Exception::Value(local))
            }
        }
    }

    /// Gibt einem Fehler der Engine, der noch keinen Realm hat, den Realm
    /// `realm`: den des Rahmens, dessen Befehl ihn warf.
    pub(crate) fn set_error_realm(&mut self, realm: RealmId) {
        if let Some(Pending::Error(_, found @ None)) = &mut self.exception {
            *found = Some(realm);
        }
    }

    /// Nimmt die Ausnahme für einen Handler von `catch` heraus: den
    /// geworfenen Wert, für einen Fehler der Engine ein neues Fehlerobjekt
    /// in dessen Realm, ohne Realm in `realm`. Ohne Ausnahme ist es
    /// *undefined*.
    pub(crate) fn take_thrown_value<'s>(
        &mut self,
        scope: &'s Scope,
        realm: RealmId,
    ) -> Result<Local<'s>, OutOfMemory> {
        if let Some(Pending::Error(kind, found)) = self.exception {
            self.exception = None;
            let realm = found.unwrap_or(realm);
            let prototype = self.intrinsic(scope, realm, Intrinsic::error_prototype(kind));
            return error_create(self, scope, prototype);
        }
        Ok(self.take_thrown(scope))
    }

    /// Nimmt die Ausnahme für einen Handler von `finally` heraus, der sie
    /// danach mit [`Agent::rethrow`] weiterwirft. Ein Fehler der Engine
    /// bleibt dabei einer: Er kommt als interne Zelle aus seiner Art und
    /// seinem Realm, die nur der Übersetzer in einem Register hält.
    pub(crate) fn take_thrown_for_rethrow<'s>(
        &mut self,
        scope: &'s Scope,
    ) -> Result<Local<'s>, OutOfMemory> {
        let Some(Pending::Error(kind, realm)) = self.exception else {
            return Ok(self.take_thrown(scope));
        };
        let cell = self.heap.alloc_slots(scope, 2)?;
        self.exception = None;
        let realm = realm
            .and_then(|realm| i32::try_from(realm.index()).ok())
            .map_or(Value::UNDEFINED, Value::int32);
        if let Kind::Internal(reference) = self.heap.get(cell).kind() {
            self.heap.set_slot(reference, 0, Value::int32(kind.code()));
            self.heap.set_slot(reference, 1, realm);
        }
        Ok(cell)
    }

    /// Der geworfene Wert, *undefined* ohne Ausnahme. Fehler der Engine
    /// nehmen die Aufrufer vorher heraus.
    fn take_thrown<'s>(&mut self, scope: &'s Scope) -> Local<'s> {
        match self.exception.take() {
            Some(Pending::Value(handle)) => {
                let value = self.heap.get_persistent(&handle);
                let local = self.heap.local(scope, value);
                self.heap.release(handle);
                local
            }
            Some(Pending::Error(..)) | None => self.heap.local(scope, Value::UNDEFINED),
        }
    }

    /// Wirft wieder, was [`Agent::take_thrown_for_rethrow`] lieferte.
    pub(crate) fn rethrow(&mut self, value: Local<'_>) -> Abrupt {
        let Kind::Internal(cell) = self.heap.get(value).kind() else {
            return self.throw_value(value);
        };
        let kind = self.heap.slot(cell, 0).and_then(|code| code.as_int32());
        let realm = self
            .heap
            .slot(cell, 1)
            .and_then(|index| index.as_int32())
            .and_then(|index| usize::try_from(index).ok())
            .map(RealmId::from_index);
        match kind.and_then(ErrorKind::from_code) {
            Some(kind) => self.set_exception(Pending::Error(kind, realm)),
            // Kann nicht vorkommen: Die Zelle stammt immer aus
            // `take_thrown_for_rethrow`.
            None => self.set_exception(Pending::Error(ErrorKind::TypeError, realm)),
        }
        Abrupt::Throw
    }
}

#[cfg(test)]
mod tests;
