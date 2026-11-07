//! Der Host (Abschnitt 4.2): die Umgebung, in der die Engine läuft, später
//! der Browser, in den Tests der Läufer für Test262.
//!
//! Die Hooks aus Anhang D.1 werden Methoden des Traits [`Host`], sobald die
//! Engine die Teile hat, die sie aufrufen: HostLoadImportedModule mit den
//! Modulen, HostEnsureCanCompileStrings mit `eval`. Bisher gibt es den
//! Schritt aus InitializeHostDefinedRealm (Abschnitt 9.3.1), in dem der
//! Host dem globalen Objekt eigene Eigenschaften gibt, und
//! HostPromiseRejectionTracker (Abschnitt 27.5.1.9).
//!
//! HostEnqueuePromiseJob (Abschnitt 9.5.5) ist kein Hook: Jobs halten Werte
//! des Heaps, darum führt der Agent ihre Warteschlange, und der Host
//! bestimmt nur, wann sie läuft ([`Agent::run_jobs`]), in Test262 nach
//! jedem Script, im Browser an den Stellen für Microtasks.
//! HostMakeJobCallback und HostCallJobCallback (Abschnitte 9.5.2 und 9.5.3)
//! sind die Vorgaben, die die Spezifikation für Hosts außer Browsern
//! verlangt; ein Browser braucht dort später den Hook für das *incumbent
//! settings object*.
//!
//! Eingebaute Funktionen des Hosts erreichen ihren Host über
//! [`Agent::host`]; wer Zustand braucht, wandelt das `Rc` in ein
//! `Rc<dyn Any>` und dann in seinen Typ.

#![forbid(unsafe_code)]

use std::any::Any;

use crate::agent::{Agent, Completion};
use crate::heap::{Local, Scope};
use crate::realm::RealmId;

/// Wann HostPromiseRejectionTracker gerufen wird (Abschnitt 27.5.1.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectionOperation {
    /// *"reject"*: Ein Promise wurde ohne Handler abgelehnt.
    Reject,
    /// *"handle"*: Ein abgelehntes Promise bekam seinen ersten Handler.
    Handle,
}

/// Die Hooks, die ein Host umsetzt. Jede Methode hat eine Vorgabe, die
/// nichts tut.
pub trait Host: Any {
    /// „Create any host-defined global object properties on *global*“
    /// (InitializeHostDefinedRealm, Abschnitt 9.3.1, vorletzter Schritt).
    /// Der Realm hat dann alle Intrinsics, sein globales Objekt und die
    /// Eigenschaften aus SetDefaultGlobalBindings.
    ///
    /// Der Hook führt kein JavaScript aus. Endet er abrupt, endet auch
    /// [`Agent::create_realm`] so, und den Realm gibt es nicht; nichts von
    /// ihm darf dann weiter erreichbar sein, auch nicht als geworfener Wert.
    fn define_global_properties(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        realm: RealmId,
    ) -> Completion<()> {
        let _ = (agent, scope, realm);
        Ok(())
    }

    /// HostPromiseRejectionTracker (Abschnitt 27.5.1.9) für `promise`. Der
    /// Hook führt kein JavaScript aus und kann nicht scheitern; die Vorgabe
    /// tut wie die der Spezifikation nichts.
    fn promise_rejection_tracker(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        promise: Local<'_>,
        operation: RejectionOperation,
    ) {
        let _ = (agent, scope, promise, operation);
    }
}

#[cfg(test)]
mod tests;
