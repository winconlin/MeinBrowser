use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::agent::{Abrupt, Budget, Cancel};
use crate::heap::Options;
use crate::realm::Intrinsic;

fn agent() -> (Agent, RealmId) {
    let mut agent = Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    (agent, realm)
}

fn block() -> Arc<SharedBlock> {
    Arc::new(SharedBlock::new(16, None).expect("Block"))
}

/// Weckt den ersten Warter bei `index`, sobald einer da ist; höchstens zehn
/// Sekunden lang.
fn notify_soon(block: &SharedBlock, index: usize) -> bool {
    for _ in 0..10_000 {
        if notify(block, index, 1) == 1 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

#[test]
fn agents_block_only_with_can_block() {
    let (mut agent, _) = agent();
    assert!(!agent.can_suspend());
    agent.set_can_block(true);
    assert!(agent.can_suspend());
}

#[test]
fn a_blocked_agent_wakes_when_another_thread_notifies() {
    let (agent, _) = agent();
    let shared = block();
    std::thread::scope(|threads| {
        let other = Arc::clone(&shared);
        let waker = threads.spawn(move || notify_soon(&other, 4));
        let result = agent.wait_blocking(&shared, 4, 4, 0, deadline(10_000.0));
        assert_eq!(result, Ok(WaitResult::Ok));
        assert!(waker.join().expect("Faden"));
    });
    assert_eq!(notify(&shared, 4, u64::MAX), 0);
}

#[test]
fn blocking_waits_compare_and_time_out() {
    let (agent, _) = agent();
    let shared = block();
    assert!(shared.write(8, &[1, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(
        agent.wait_blocking(&shared, 8, 8, 0, None),
        Ok(WaitResult::NotEqual)
    );
    let start = Instant::now();
    assert_eq!(
        agent.wait_blocking(&shared, 8, 8, 1, deadline(20.0)),
        Ok(WaitResult::TimedOut)
    );
    assert!(start.elapsed() >= Duration::from_millis(20));
    // Eine Wartezeit von 0 endet gleich, wie in SuspendThisAgent.
    assert_eq!(
        agent.wait_blocking(&shared, 0, 4, 0, deadline(0.0)),
        Ok(WaitResult::TimedOut)
    );
    // Wer aufgegeben hat, steht nicht mehr in der Liste.
    assert_eq!(notify(&shared, 8, u64::MAX), 0);
    assert_eq!(notify(&shared, 0, u64::MAX), 0);
}

#[test]
fn the_host_can_cancel_an_endless_wait() {
    let (mut agent, _) = agent();
    let shared = block();
    let cancel = Cancel::new();
    let budget = Rc::new(Budget::with_cancel(1000, cancel.clone()));
    std::thread::scope(|threads| {
        threads.spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            cancel.cancel();
        });
        let result = agent
            .with_execution_budget(budget, |agent| agent.wait_blocking(&shared, 0, 4, 0, None));
        assert!(matches!(result, Err(Abrupt::Exhausted(_))), "{result:?}");
    });
    assert_eq!(notify(&shared, 0, u64::MAX), 0);
}

/// Legt einen Warter von `Atomics.waitAsync` an und gibt sein Promise
/// dauerhaft zurück.
fn wait_async(
    agent: &mut Agent,
    realm: RealmId,
    shared: &Arc<SharedBlock>,
    timeout: f64,
) -> (Option<WaitResult>, Persistent) {
    agent.scope(|agent, scope| {
        let prototype = agent.intrinsic(scope, realm, Intrinsic::PromisePrototype);
        let promise = crate::promise::promise_create(agent, scope, prototype).expect("Speicher");
        let started = agent.wait_async(shared, 0, 4, 0, timeout, promise, realm);
        (started, agent.heap_mut().persist(promise))
    })
}

/// Ob das Promise mit `expected` erfüllt ist, über `then` beobachtet.
fn fulfilled_with(agent: &mut Agent, realm: RealmId, promise: &Persistent) -> String {
    agent.scope(|agent, scope| {
        let promise = agent
            .heap()
            .local(scope, agent.heap().get_persistent(promise));
        let global = agent.global_object(scope, realm);
        let key = agent.key_from_units(scope, &units("p")).expect("Speicher");
        crate::operations::create_data_property_or_throw(agent, scope, global, key, promise)
            .expect("Eigenschaft");
    });
    let source = "var seen = 'offen'; p.then(function (v) { seen = v; }); 1";
    agent.scope(|agent, scope| {
        agent
            .evaluate_script(scope, realm, &units(source))
            .map(|_| ())
            .expect("Script");
    });
    agent.run_jobs().expect("Jobs");
    agent.scope(|agent, scope| {
        let value = agent
            .evaluate_script(scope, realm, &units("seen"))
            .expect("Script");
        let heap = agent.heap();
        match heap.get(value).kind() {
            crate::value::Kind::String(string) => {
                String::from_utf16(&heap.string_units(string)).expect("UTF-16")
            }
            kind => format!("{kind:?}"),
        }
    })
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn async_waiters_settle_after_notify_or_timeout() {
    let (mut agent, realm) = agent();
    let shared = block();
    let (started, notified) = wait_async(&mut agent, realm, &shared, f64::INFINITY);
    assert_eq!(started, None);
    let (started, timed) = wait_async(&mut agent, realm, &shared, 5.0);
    assert_eq!(started, None);
    assert_eq!(agent.async_waiter_count(), 2);
    assert!(agent.next_timer().is_some());
    // Der erste Warter kam zuerst und wird zuerst geweckt.
    assert_eq!(notify(&shared, 0, 1), 1);
    assert!(agent.has_pending_jobs());
    assert_eq!(fulfilled_with(&mut agent, realm, &notified), "ok");
    assert_eq!(agent.async_waiter_count(), 1);
    let next = agent.next_timer().expect("Zeitgeber");
    std::thread::sleep(next.saturating_duration_since(Instant::now()));
    assert!(agent.has_pending_jobs());
    assert_eq!(fulfilled_with(&mut agent, realm, &timed), "timed-out");
    assert_eq!(agent.async_waiter_count(), 0);
    assert_eq!(agent.next_timer(), None);
    assert_eq!(notify(&shared, 0, u64::MAX), 0);
    // Gleich beantwortet: ein anderer Wert und eine Wartezeit von 0.
    assert!(shared.write(0, &[7]));
    assert_eq!(
        wait_async(&mut agent, realm, &shared, 1.0).0,
        Some(WaitResult::NotEqual)
    );
    assert!(shared.write(0, &[0]));
    assert_eq!(
        wait_async(&mut agent, realm, &shared, 0.0).0,
        Some(WaitResult::TimedOut)
    );
    assert_eq!(agent.async_waiter_count(), 0);
    agent.heap_mut().release(notified);
    agent.heap_mut().release(timed);
}

#[test]
fn async_waiters_leave_the_list_with_their_agent() {
    let shared = block();
    {
        let (mut agent, realm) = agent();
        let (started, promise) = wait_async(&mut agent, realm, &shared, f64::INFINITY);
        assert_eq!(started, None);
        agent.heap_mut().release(promise);
    }
    assert_eq!(notify(&shared, 0, u64::MAX), 0);
}
