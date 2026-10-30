//! Arbeitsbudgets: wie viel ein Auftrag noch rechnen darf und ob er von
//! außen abgebrochen wurde.
//!
//! Fremde Inhalte können Arbeit verlangen, die mit ihrer Größe stark wächst,
//! etwa tief verschachtelte Selektoren oder Größen aus dem Inhalt. Ein
//! Auftrag wie Stil und Layout einer Seite bekommt deshalb ein [`Budget`] an
//! Arbeitsschritten. Die Teile, durch die er läuft, verbrauchen davon an
//! festen Stellen und hören auf, sobald es verbraucht ist. Was ein Schritt
//! ist, legen die Teile selbst fest; er soll ungefähr gleich viel kosten,
//! etwa einen Verbundselektor gegen ein Element prüfen.
//!
//! Ein [`Cancel`] bricht einen Auftrag von einem anderen Faden aus ab, etwa
//! wenn die Oberfläche eine neue Größe braucht.

use std::cell::Cell;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Warum ein Auftrag aufgehört hat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exhausted {
    /// Die Arbeitsschritte sind verbraucht.
    Steps,
    /// Der Auftrag wurde von außen abgebrochen.
    Cancelled,
}

impl fmt::Display for Exhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Exhausted::Steps => "zu viel Arbeit, das Budget ist verbraucht",
            Exhausted::Cancelled => "abgebrochen",
        })
    }
}

impl std::error::Error for Exhausted {}

/// Ein Abbruchsignal, das jeder Faden setzen kann, der eine Kopie hat.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Die Arbeitsschritte, die ein Auftrag noch tun darf. Ein Budget gehört
/// zu einem Faden; abbrechen lässt es sich über ein [`Cancel`].
#[derive(Debug)]
pub struct Budget {
    remaining: Cell<u64>,
    cancel: Option<Cancel>,
    exhausted: Cell<Option<Exhausted>>,
}

impl Budget {
    pub fn new(steps: u64) -> Budget {
        Budget {
            remaining: Cell::new(steps),
            cancel: None,
            exhausted: Cell::new(None),
        }
    }

    /// Ein Budget, das nie ausgeht, für vertrauenswürdige Eingaben wie
    /// Tests.
    pub fn unlimited() -> Budget {
        Budget::new(u64::MAX)
    }

    /// Ein Budget, das zusätzlich endet, sobald `cancel` gesetzt ist.
    pub fn with_cancel(steps: u64, cancel: Cancel) -> Budget {
        Budget {
            cancel: Some(cancel),
            ..Budget::new(steps)
        }
    }

    /// Verbraucht Schritte. `false`, sobald das Budget verbraucht oder der
    /// Auftrag abgebrochen ist; danach bleibt es dabei.
    pub fn spend(&self, steps: u64) -> bool {
        if self.exhausted.get().is_some() {
            return false;
        }
        if self.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
            self.exhausted.set(Some(Exhausted::Cancelled));
            return false;
        }
        match self.remaining.get().checked_sub(steps) {
            Some(remaining) => {
                self.remaining.set(remaining);
                true
            }
            None => {
                self.remaining.set(0);
                self.exhausted.set(Some(Exhausted::Steps));
                false
            }
        }
    }

    /// Warum der Auftrag aufgehört hat, `None` solange er weiterlaufen darf.
    pub fn exhausted(&self) -> Option<Exhausted> {
        self.exhausted.get()
    }

    /// `Err`, sobald das Budget verbraucht oder der Auftrag abgebrochen ist.
    pub fn check(&self) -> Result<(), Exhausted> {
        match self.exhausted() {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }

    /// Die Schritte, die noch übrig sind.
    pub fn remaining(&self) -> u64 {
        self.remaining.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_run_out() {
        let budget = Budget::new(3);
        assert!(budget.spend(2));
        assert_eq!(budget.remaining(), 1);
        assert_eq!(budget.check(), Ok(()));
        assert!(!budget.spend(2));
        assert_eq!(budget.exhausted(), Some(Exhausted::Steps));
        // Einmal verbraucht, bleibt es verbraucht.
        assert!(!budget.spend(0));
        assert_eq!(budget.check(), Err(Exhausted::Steps));
    }

    #[test]
    fn cancel_from_another_thread() {
        let cancel = Cancel::new();
        let budget = Budget::with_cancel(u64::MAX, cancel.clone());
        assert!(budget.spend(1));
        std::thread::spawn(move || cancel.cancel())
            .join()
            .expect("Faden");
        assert!(!budget.spend(1));
        assert_eq!(budget.exhausted(), Some(Exhausted::Cancelled));
    }
}
