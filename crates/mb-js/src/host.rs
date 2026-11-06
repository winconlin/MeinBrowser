//! Der Host (Abschnitt 4.2): die Umgebung, in der die Engine läuft, später
//! der Browser, in den Tests der Läufer für Test262.
//!
//! Die Hooks aus Anhang D.1 werden Methoden des Traits [`Host`], sobald die
//! Engine die Teile hat, die sie aufrufen: HostEnqueuePromiseJob mit den
//! Promises, HostLoadImportedModule mit den Modulen,
//! HostEnsureCanCompileStrings mit `eval`. Bisher gibt es nur den Schritt
//! aus InitializeHostDefinedRealm (Abschnitt 9.3.1), in dem der Host dem
//! globalen Objekt eigene Eigenschaften gibt.
//!
//! Eingebaute Funktionen des Hosts erreichen ihren Host über
//! [`Agent::host`]; wer Zustand braucht, wandelt das `Rc` in ein
//! `Rc<dyn Any>` und dann in seinen Typ.

#![forbid(unsafe_code)]

use std::any::Any;

use crate::agent::{Agent, Completion};
use crate::heap::Scope;
use crate::realm::RealmId;

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
}

#[cfg(test)]
mod tests;
