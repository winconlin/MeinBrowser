//! Die Seite eines Agents für `Atomics.wait` und `Atomics.waitAsync`:
//! [[CanBlock]] des Agent Record (Abschnitt 9.7), das
//! Schlafen in DoWait (Abschnitt 25.4.3.14) mit SuspendThisAgent, die
//! Warter von `Atomics.waitAsync` mit ihren Zeitgebern und die Jobs, die
//! ihre Promises erfüllen. Die WaiterLists selbst gehören zum geteilten
//! Block (siehe `crate::shared`).
//!
//! Festlegungen:
//! - [[CanBlock]] ist zunächst *false*, wie im Hauptfaden eines Browsers;
//!   ein Host, der schlafen darf, setzt es mit [`Agent::set_can_block`].
//! - Ein Agent, der in `Atomics.wait` schläft, wacht alle [`SLICE`] auf und
//!   fragt das Abbruchsignal seiner Arbeitsquote ab. So kann der Host auch
//!   ein Warten ohne Ende abbrechen; das Warten selbst kostet keine
//!   Schritte.
//! - Die Zeit ist die monotone Uhr des Systems ([`Instant`]);
//!   additionalTimeout ist 0.
//! - HostEnqueueGenericJob und HostEnqueueTimeoutJob (Abschnitte 9.5.6 und
//!   9.5.7) für EnqueueResolveInAgentJob und
//!   EnqueueAtomicsWaitAsyncTimeoutJob (Abschnitte 25.4.3.15 und
//!   25.4.3.16) gehen in eine eigene Warteschlange von Aufgaben des Agents.
//!   [`Agent::run_jobs`] nimmt eine davon, wenn keine Promise-Jobs und
//!   keine Aufräum-Jobs mehr warten. Ein abgelaufener Zeitgeber erfüllt
//!   sein Promise direkt mit *"timed-out"*, ohne den Umweg über NotifyWaiter
//!   und einen zweiten Job; zu beobachten ist nur, dass es danach erfüllt
//!   ist.
//! - Wann ein Zeitgeber abläuft, prüft der Agent nur in
//!   [`Agent::run_jobs`]. Ein Host, der auf ihn warten will, fragt
//!   [`Agent::next_timer`].

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, PoisonError};
use std::time::{Duration, Instant};

use crate::agent::{Agent, Completion};
use crate::heap::{Local, Persistent};
use crate::promise::resolve_promise;
use crate::realm::RealmId;
use crate::shared::{Mailbox, SharedBlock, Wake};

/// So lange schläft ein Agent in `Atomics.wait` höchstens am Stück, bevor
/// er das Abbruchsignal abfragt.
const SLICE: Duration = Duration::from_millis(10);

/// Das Ergebnis eines Wartens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitResult {
    Ok,
    TimedOut,
    NotEqual,
}

impl WaitResult {
    /// Die Zeichenkette, die `Atomics.wait` zurückgibt.
    pub(crate) fn text(self) -> &'static str {
        match self {
            WaitResult::Ok => "ok",
            WaitResult::TimedOut => "timed-out",
            WaitResult::NotEqual => "not-equal",
        }
    }
}

/// Ein Eintrag in einer WaiterList. Er verschwindet mit ihr aus der Liste,
/// wenn er noch darin steht, etwa wenn der Agent endet.
struct Registration {
    block: Arc<SharedBlock>,
    index: usize,
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.block.critical_section().remove(self.index, self.id);
    }
}

/// Ein Warter von `Atomics.waitAsync` dieses Agents: das Promise aus
/// [[PromiseCapability]], der Realm, in dem es entstand, sein Eintrag und
/// [[TimeoutTime]], `None` für +∞.
struct AsyncWaiter {
    promise: Persistent,
    realm: RealmId,
    registration: Registration,
    deadline: Option<Instant>,
}

/// Eine Aufgabe: ein Promise eines Warters mit dem Ergebnis erfüllen.
struct Task {
    promise: Persistent,
    realm: RealmId,
    result: WaitResult,
}

/// Was ein Agent fürs Warten braucht.
#[derive(Default)]
pub(crate) struct Waiting {
    can_block: bool,
    /// Wohin ein anderer Agent oder dieser selbst die Nummern geweckter
    /// Warter von `Atomics.waitAsync` legt.
    mailbox: Arc<Mailbox>,
    /// Die Warter von `Atomics.waitAsync` nach ihrer Nummer.
    waiters: BTreeMap<u64, AsyncWaiter>,
    next_ticket: u64,
    tasks: VecDeque<Task>,
}

/// [[TimeoutTime]] für eine Wartezeit von `timeout` Millisekunden, nicht
/// negativ; `None` für +∞ und für Zeiten, die die Uhr nicht fasst.
pub(crate) fn deadline(timeout: f64) -> Option<Instant> {
    let duration = Duration::try_from_secs_f64(timeout / 1000.0).ok()?;
    Instant::now().checked_add(duration)
}

impl Agent {
    /// Setzt [[CanBlock]]: ob `Atomics.wait` diesen Agent schlafen legen
    /// darf (AgentCanSuspend, Abschnitt 9.7.1).
    pub fn set_can_block(&mut self, can_block: bool) {
        self.waiting.can_block = can_block;
    }

    /// AgentCanSuspend (Abschnitt 9.7.1).
    pub(crate) fn can_suspend(&self) -> bool {
        self.waiting.can_block
    }

    /// Wann der nächste Zeitgeber eines Warters von `Atomics.waitAsync`
    /// abläuft, `None`, wenn keiner einen hat. Danach erfüllt
    /// [`Agent::run_jobs`] sein Promise.
    pub fn next_timer(&self) -> Option<Instant> {
        self.waiting
            .waiters
            .values()
            .filter_map(|waiter| waiter.deadline)
            .min()
    }

    /// Ob eine Aufgabe wartet oder bald entsteht: ein Warter wurde geweckt
    /// oder sein Zeitgeber ist abgelaufen.
    pub(crate) fn has_pending_tasks(&self) -> bool {
        !self.waiting.tasks.is_empty()
            || !self.waiting.mailbox.is_empty()
            || self
                .next_timer()
                .is_some_and(|deadline| deadline <= Instant::now())
    }

    /// Die Schritte 17 bis 29 und 32 von DoWait für ~sync~: Ist das
    /// Element von `size` Bytes bei `index` gleich `expected`, schläft der
    /// Agent, bis ein anderer ihn weckt oder `deadline` vorbei ist.
    /// Unterbricht der Host, kommt das als Abbruch zurück, und der Warter
    /// steht nicht mehr in der Liste.
    pub(crate) fn wait_blocking(
        &self,
        block: &SharedBlock,
        index: usize,
        size: usize,
        expected: u64,
        deadline: Option<Instant>,
    ) -> Completion<WaitResult> {
        let condition = Arc::new(Condvar::new());
        let mut lists = block.critical_section();
        if block.load(index, size) != Some(expected) {
            return Ok(WaitResult::NotEqual);
        }
        let id = lists.add(index, Wake::Blocking(Arc::clone(&condition)));
        loop {
            // SuspendThisAgent, Schritt 9: Wer nicht mehr in der Liste
            // steht, wurde geweckt.
            if !lists.contains(index, id) {
                return Ok(WaitResult::Ok);
            }
            let now = Instant::now();
            let slice = match deadline {
                Some(deadline) if deadline <= now => {
                    lists.remove(index, id);
                    return Ok(WaitResult::TimedOut);
                }
                Some(deadline) => deadline.duration_since(now).min(SLICE),
                None => SLICE,
            };
            lists = condition
                .wait_timeout(lists, slice)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            if self.has_execution_budget()
                && let Err(abrupt) = self.spend_execution_steps(0)
            {
                lists.remove(index, id);
                return Err(abrupt);
            }
        }
    }

    /// Die Schritte 17 bis 31 von DoWait für ~async~ mit dem Promise
    /// `promise` aus dem Realm `realm`: *"not-equal"*, wenn das Element
    /// nicht `expected` ist, *"timed-out"* bei einer Wartezeit von 0, sonst
    /// steht ein Warter in der Liste, und das Ergebnis ist `None`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn wait_async(
        &mut self,
        block: &Arc<SharedBlock>,
        index: usize,
        size: usize,
        expected: u64,
        timeout: f64,
        promise: Local<'_>,
        realm: RealmId,
    ) -> Option<WaitResult> {
        let ticket = self.waiting.next_ticket;
        let id = {
            let mut lists = block.critical_section();
            if block.load(index, size) != Some(expected) {
                return Some(WaitResult::NotEqual);
            }
            if timeout == 0.0 {
                return Some(WaitResult::TimedOut);
            }
            lists.add(
                index,
                Wake::Async {
                    ticket,
                    mailbox: Arc::clone(&self.waiting.mailbox),
                },
            )
        };
        self.waiting.next_ticket = ticket.wrapping_add(1);
        let promise = self.heap_mut().persist(promise);
        self.waiting.waiters.insert(
            ticket,
            AsyncWaiter {
                promise,
                realm,
                registration: Registration {
                    block: Arc::clone(block),
                    index,
                    id,
                },
                deadline: deadline(timeout),
            },
        );
        None
    }

    /// Macht aus geweckten Wartern und abgelaufenen Zeitgebern Aufgaben.
    fn collect_wakeups(&mut self) {
        for ticket in self.waiting.mailbox.take() {
            if let Some(waiter) = self.waiting.waiters.remove(&ticket) {
                self.waiting.tasks.push_back(Task {
                    promise: waiter.promise,
                    realm: waiter.realm,
                    result: WaitResult::Ok,
                });
            }
        }
        let now = Instant::now();
        let expired: Vec<u64> = self
            .waiting
            .waiters
            .iter()
            .filter(|(_, waiter)| waiter.deadline.is_some_and(|deadline| deadline <= now))
            .map(|(&ticket, _)| ticket)
            .collect();
        for ticket in expired {
            let Some(waiter) = self.waiting.waiters.remove(&ticket) else {
                continue;
            };
            let registration = &waiter.registration;
            // Hat ihn inzwischen jemand geweckt, steht er nicht mehr in der
            // Liste, und seine Nummer liegt im Briefkasten. Dann gilt
            // *"ok"*.
            let timed_out = registration
                .block
                .critical_section()
                .remove(registration.index, registration.id);
            self.waiting.tasks.push_back(Task {
                promise: waiter.promise,
                realm: waiter.realm,
                result: if timed_out {
                    WaitResult::TimedOut
                } else {
                    WaitResult::Ok
                },
            });
        }
    }

    /// Führt die nächste Aufgabe aus, wenn es eine gibt; `false`, wenn
    /// keine wartet.
    pub(crate) fn run_next_task(&mut self) -> Completion<bool> {
        self.collect_wakeups();
        if self.waiting.tasks.is_empty() {
            return Ok(false);
        }
        self.spend_execution_steps(1)?;
        let Some(task) = self.waiting.tasks.pop_front() else {
            return Ok(false);
        };
        self.scope(|agent, scope| {
            let heap = agent.heap();
            let promise = heap.local(scope, heap.get_persistent(&task.promise));
            agent.heap_mut().release(task.promise);
            let units: Vec<u16> = task.result.text().encode_utf16().collect();
            let result = agent.heap_mut().alloc_string(scope, &units)?;
            resolve_promise(agent, scope, task.realm, promise, result)
        })?;
        Ok(true)
    }
}

/// Atomics.notify, Schritte 9 bis 13 (Abschnitt 25.4.4.12): weckt die
/// ersten `count` Warter der Liste von `index` und gibt zurück, wie viele
/// es waren. NotifyWaiter (Abschnitt 25.4.3.13) weckt einen schlafenden
/// Agent an seiner Bedingung und legt für einen Warter von
/// `Atomics.waitAsync` seine Nummer in den Briefkasten seines Agents.
pub(crate) fn notify(block: &SharedBlock, index: usize, count: u64) -> usize {
    let mut lists = block.critical_section();
    let woken = lists.remove_first(index, count);
    for waiter in &woken {
        match &waiter.wake {
            Wake::Blocking(condition) => condition.notify_one(),
            Wake::Async { ticket, mailbox } => mailbox.push(*ticket),
        }
    }
    woken.len()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
impl Agent {
    /// Wie viele Warter von `Atomics.waitAsync` der Agent hat.
    pub(crate) fn async_waiter_count(&self) -> usize {
        self.waiting.waiters.len()
    }
}
