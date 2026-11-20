//! Der Host (Abschnitt 4.2): die Umgebung, in der die Engine läuft, später
//! der Browser, in den Tests der Läufer für Test262.
//!
//! Die Hooks aus Anhang D.1 werden Methoden des Traits [`Host`], sobald die
//! Engine die Teile hat, die sie aufrufen: HostLoadImportedModule mit den
//! Modulen, HostEnsureCanCompileStrings mit `eval`. Bisher gibt es den
//! Schritt aus InitializeHostDefinedRealm (Abschnitt 9.3.1), in dem der
//! Host dem globalen Objekt eigene Eigenschaften gibt, und
//! HostPromiseRejectionTracker (Abschnitt 27.5.1.9). Dazu kommt, was die
//! Spezifikation für `Date` der Umgebung überlässt: die aktuelle Zeit und
//! die Zeitzone (SystemTimeZoneIdentifier, Abschnitt 21.4.1.24). Ohne Host
//! gelten dieselben Vorgaben wie für einen Host, der sie nicht ändert.
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
use crate::date::{self, TimeZone};
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

    /// Die aktuelle Zeit für `Date.now` und `Date`: „the time value (UTC)
    /// identifying the current time“ (Abschnitt 21.4.2.1), Millisekunden
    /// seit dem 1. Januar 1970 UTC. Die Engine kürzt den Wert mit TimeClip
    /// (Abschnitt 21.4.1.31). Die Vorgabe liest die Uhr des Systems
    /// ([`system_time`]); ein Browser kann sie vergröbern.
    fn current_time(&self) -> f64 {
        system_time()
    }

    /// Die Zeitzone der Umgebung, SystemTimeZoneIdentifier (Abschnitt
    /// 21.4.1.24). Die Vorgabe ist UTC wie für eine Implementierung, die
    /// nur UTC kennt; [`TimeZone::from_identifier`] liest auch einen festen
    /// Abstand wie *"+01:00"*.
    fn time_zone(&self) -> TimeZone {
        TimeZone::Utc
    }
}

/// Die Uhr des Systems in Millisekunden seit dem 1. Januar 1970 UTC, auf
/// ganze Millisekunden abgerundet.
pub fn system_time() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanoseconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i128::try_from(elapsed.as_nanos()).unwrap_or(i128::MAX),
        Err(error) => i128::try_from(error.duration().as_nanos()).map_or(i128::MIN, |value| -value),
    };
    // Abgerundet, auch vor 1970; jeder Wert passt als Number, TimeClip
    // macht zu große zu NaN.
    nanoseconds.div_euclid(1_000_000) as f64
}

/// Die aktuelle Zeit nach TimeClip, vom Host des Agenten oder aus der Uhr
/// des Systems.
pub(crate) fn current_time(agent: &Agent) -> f64 {
    let time = agent
        .host()
        .map_or_else(system_time, |host| host.current_time());
    date::time_clip(time)
}

/// Die Zeitzone des Hosts, ohne Host UTC.
pub(crate) fn time_zone(agent: &Agent) -> TimeZone {
    agent.host().map_or(TimeZone::Utc, |host| host.time_zone())
}

#[cfg(test)]
mod tests;
