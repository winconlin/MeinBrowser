//! Der Agent (Abschnitt 9.6): Heap, Atome, die Wurzel aller Gestalten, die
//! bekannten Symbole und die Ausnahme, die gerade geworfen wird.
//! Registerstapel, Warteschlange der Jobs und Symbolregister kommen mit den
//! Schritten, die sie brauchen.
//!
//! Atome sind die Zeichenketten, die als Eigenschaftsschlüssel dienen. Zu
//! jedem Inhalt gibt es höchstens eines, also vergleicht man Schlüssel über
//! ihren Abstand. Atome bleiben vorerst, solange der Agent lebt.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use crate::heap::{Heap, Local, Options, OutOfMemory, Persistent, Scope};
use crate::key::WellKnownSymbol;
use crate::object::shape;
use crate::value::{Kind, Value};

/// Eine abrupte Beendigung, die Rust-Code mit `?` nach oben reicht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abrupt {
    /// Eine Ausnahme (Abschnitt 6.2.4, Art *throw*). Den geworfenen Wert
    /// hält der Agent, siehe [`Agent::take_exception`].
    Throw,
    /// Das Speicherbudget ist verbraucht. Kein Skript kann das abfangen.
    OutOfMemory,
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
    exception: Option<Pending>,
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
            exception: None,
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

    /// Führt `f` in einem neuen Bereich für Handles aus, wie
    /// [`Heap::scope`].
    pub fn scope<R, F: FnOnce(&mut Agent, &Scope) -> R>(&mut self, f: F) -> R {
        let scope = self.heap.enter_scope();
        let result = f(self, &scope);
        self.heap.leave_scope(&scope);
        result
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
