//! Der Agent (Abschnitt 9.6): Heap mit dem Registerstapel, Atome, die
//! Wurzel aller Gestalten, die bekannten Symbole, die Realms, die
//! gemeinsamen Daten der Funktionen und die Ausnahme, die gerade geworfen
//! wird. Warteschlange der Jobs und Symbolregister kommen mit den
//! Schritten, die sie brauchen.
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

use std::collections::BTreeMap;
use std::rc::Rc;

pub use mb_base::{Budget, Cancel, Exhausted};

use crate::function::SharedFunction;
use crate::heap::{Heap, Local, Options, OutOfMemory, Persistent, Scope};
use crate::key::WellKnownSymbol;
use crate::object::shape;
use crate::realm::Realm;
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

/// Die geworfene Ausnahme.
#[derive(Clone, Copy, Debug)]
pub enum Exception<'s> {
    /// Ein Fehler der Engine. Das Fehlerobjekt dazu entsteht, sobald es
    /// Realms mit ihren Intrinsics gibt.
    Error(ErrorKind),
    Value(Local<'s>),
}

enum Pending {
    Error(ErrorKind),
    Value(Persistent),
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
    exception: Option<Pending>,
    /// Wie viele Läufe des Interpreters gerade ineinander laufen.
    runs: usize,
    /// Die Lage des Rust-Stapels beim äußersten Lauf.
    stack_start: usize,
    /// Alle aktiven Hostquoten. Verschachtelte Aufrufe belasten sie
    /// gemeinsam; eine zusätzliche Quote ersetzt keine äußere.
    execution_budgets: Vec<Rc<Budget>>,
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
            exception: None,
            runs: 0,
            stack_start: 0,
            execution_budgets: Vec::new(),
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
        self.set_exception(Pending::Error(kind));
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
            Pending::Error(kind) => Some(Exception::Error(kind)),
            Pending::Value(handle) => {
                let value = self.heap.get_persistent(&handle);
                let local = self.heap.local(scope, value);
                self.heap.release(handle);
                Some(Exception::Value(local))
            }
        }
    }
}

#[cfg(test)]
mod tests;
