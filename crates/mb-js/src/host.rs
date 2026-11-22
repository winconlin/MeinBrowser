//! Der Host (Abschnitt 4.2): die Umgebung, in der die Engine läuft, später
//! der Browser, in den Tests der Läufer für Test262.
//!
//! Die Hooks aus Anhang D.1 werden Methoden des Traits [`Host`], sobald die
//! Engine die Teile hat, die sie aufrufen. Bisher gibt es den Schritt aus
//! InitializeHostDefinedRealm (Abschnitt 9.3.1), in dem der Host dem
//! globalen Objekt eigene Eigenschaften gibt, HostPromiseRejectionTracker
//! (Abschnitt 27.5.1.9) und für Module HostLoadImportedModule und
//! HostGetSupportedImportAttributes (Abschnitte 16.2.1.10 und 16.2.1.12.1)
//! sowie HostGetImportMetaProperties und HostFinalizeImportMeta (Abschnitte
//! 13.3.12.1.1 und 13.3.12.1.2). Dazu kommt, was die
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

use crate::agent::{Agent, Completion, ErrorKind};
use crate::date::{self, TimeZone};
use crate::heap::{Local, Scope};
use crate::key::PropertyKey;
use crate::module::{LoadPayload, ModuleId, ModuleRequest, Referrer};
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
    /// HostLoadImportedModule (Abschnitt 16.2.1.10): Der Host findet das
    /// Modul zu `request` für `referrer`, legt es bei Bedarf mit
    /// [`Agent::parse_module`] oder [`Agent::parse_json_module`] an und ruft
    /// dann, sofort oder später, [`Agent::finish_loading_imported_module`]
    /// mit `payload` und dem Modul oder einer Ausnahme. Für dieselbe Anfrage
    /// desselben `referrer` muss er stets dasselbe Modul liefern. Die
    /// Vorgabe kennt kein Modul und beendet das Laden mit einem TypeError.
    fn load_imported_module(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        referrer: Referrer,
        request: &ModuleRequest,
        payload: LoadPayload,
    ) -> Completion<()> {
        let error = agent.throw_error(ErrorKind::TypeError);
        agent.finish_loading_imported_module(scope, referrer, request, payload, Err(error))
    }

    /// Ob `key` in HostGetSupportedImportAttributes (Abschnitt 16.2.1.12.1)
    /// steht. Die Vorgabe kennt keinen Schlüssel; dann scheitert jedes
    /// `import` mit `with { … }` beim Laden mit einem SyntaxError.
    fn supports_import_attribute(&self, key: &[u16]) -> bool {
        let _ = key;
        false
    }

    /// HostGetImportMetaProperties (Abschnitt 13.3.12.1.1): die
    /// Eigenschaften, mit denen `import.meta` des Moduls beginnt. Die
    /// Vorgabe hat keine.
    fn import_meta_properties<'s>(
        &self,
        agent: &mut Agent,
        scope: &'s Scope,
        module: ModuleId,
    ) -> Completion<Vec<(PropertyKey<'s>, Local<'s>)>> {
        let _ = (agent, scope, module);
        Ok(Vec::new())
    }

    /// HostFinalizeImportMeta (Abschnitt 13.3.12.1.2) für das neue Objekt
    /// `meta` des Moduls. Die Vorgabe tut nichts.
    fn finalize_import_meta(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        meta: Local<'_>,
        module: ModuleId,
    ) -> Completion<()> {
        let _ = (agent, scope, meta, module);
        Ok(())
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
