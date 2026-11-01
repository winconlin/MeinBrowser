//! Execution quotas are host failures and remain shared across VM entries.

use std::rc::Rc;

use crate::agent::{Abrupt, Agent, Budget, Cancel, DEFAULT_EXECUTION_STEPS, Exhausted};
use crate::compiler::compile_script;
use crate::heap::Options;
use crate::parser::parse_script;
use crate::realm::RealmId;
use crate::script::ScriptError;

fn units(source: &str) -> Vec<u16> {
    source.encode_utf16().collect()
}

fn checked(stress: bool) -> (Agent, RealmId) {
    let mut agent = Agent::with_options(Options {
        limit: 1 << 20,
        stress,
        verify: true,
    })
    .expect("bounded test agent");
    let realm = agent.create_realm().expect("test realm");
    (agent, realm)
}

fn clean(agent: &mut Agent) {
    assert_eq!(
        agent.heap_mut().push_registers(0),
        Some(0),
        "no saved frames"
    );
    agent.heap_mut().collect();
    agent.heap().verify().expect("valid heap after execution");
    agent.scope(|agent, scope| {
        assert!(
            agent.take_exception(scope).is_none(),
            "no pending script throw"
        );
    });
}

fn within(
    agent: &mut Agent,
    realm: RealmId,
    source: &str,
    budget: Rc<Budget>,
) -> Result<Option<f64>, ScriptError> {
    let result = agent.scope(|agent, scope| {
        agent
            .evaluate_script_within(scope, realm, &units(source), budget)
            .map(|value| agent.heap().get(value).as_number())
    });
    clean(agent);
    result
}

fn ordinary(agent: &mut Agent, realm: RealmId, source: &str) -> Result<Option<f64>, ScriptError> {
    let result = agent.scope(|agent, scope| {
        agent
            .evaluate_script(scope, realm, &units(source))
            .map(|value| agent.heap().get(value).as_number())
    });
    clean(agent);
    result
}

fn exhausted(reason: Exhausted) -> Result<Option<f64>, ScriptError> {
    Err(ScriptError::Abrupt(Abrupt::Exhausted(reason)))
}

#[test]
fn entry_and_instruction_costs_have_an_exact_success_boundary() {
    let source = units("7;");
    let ast = parse_script(&source).expect("test syntax");
    let script = compile_script(&ast).expect("test code");
    let mut offset = 0;
    // One script entry and one interpreter-run entry precede the code.
    let mut steps = 2;
    while let Some(instruction) = script.code.decode(offset) {
        steps += 1;
        offset = instruction.end;
    }
    assert_eq!(offset, script.code.bytes().len());

    for stress in [false, true] {
        let (mut agent, realm) = checked(stress);
        let exact = Rc::new(Budget::new(steps));
        assert_eq!(
            within(&mut agent, realm, "7;", Rc::clone(&exact)),
            Ok(Some(7.0))
        );
        assert_eq!(exact.remaining(), 0);
        assert_eq!(
            exact.exhausted(),
            None,
            "last allowed instruction can finish"
        );
        assert_eq!(
            within(&mut agent, realm, "7;", Rc::clone(&exact)),
            exhausted(Exhausted::Steps)
        );
        let short = Rc::new(Budget::new(steps - 1));
        assert_eq!(
            within(&mut agent, realm, "7;", Rc::clone(&short)),
            exhausted(Exhausted::Steps)
        );
        assert_eq!(short.remaining(), 0);
        assert_eq!(short.exhausted(), Some(Exhausted::Steps));
    }
}

#[test]
fn zero_steps_and_thread_cancellation_stop_before_global_declarations() {
    for stress in [false, true] {
        let (mut agent, realm) = checked(stress);
        let empty = Rc::new(Budget::new(0));
        assert_eq!(
            within(&mut agent, realm, "var shouldNotExist = 9;", empty),
            exhausted(Exhausted::Steps)
        );
        assert_eq!(
            ordinary(
                &mut agent,
                realm,
                "typeof shouldNotExist === 'undefined' ? 1 : 0"
            ),
            Ok(Some(1.0))
        );
        // A missing value alone is insufficient: no var binding may remain.
        assert_eq!(
            ordinary(&mut agent, realm, "let shouldNotExist = 4; shouldNotExist"),
            Ok(Some(4.0))
        );

        let cancel = Cancel::new();
        let signal = cancel.clone();
        std::thread::spawn(move || signal.cancel())
            .join()
            .expect("cancel thread");
        let budget = Rc::new(Budget::with_cancel(100, cancel));
        assert_eq!(
            within(
                &mut agent,
                realm,
                "var cancelledName = 9;",
                Rc::clone(&budget)
            ),
            exhausted(Exhausted::Cancelled)
        );
        assert_eq!(budget.remaining(), 100);
        assert_eq!(budget.exhausted(), Some(Exhausted::Cancelled));
        assert_eq!(
            ordinary(&mut agent, realm, "let cancelledName = 5; cancelledName"),
            Ok(Some(5.0))
        );
    }
}

#[test]
fn loops_and_direct_calls_use_the_supplied_ledger_and_recover() {
    for stress in [false, true] {
        for source in [
            "var ticks = 0; for (;;) ticks++;",
            "function recurse() { return recurse(); } recurse();",
        ] {
            let (mut agent, realm) = checked(stress);
            let budget = Rc::new(Budget::new(128));
            assert_eq!(
                within(&mut agent, realm, source, Rc::clone(&budget)),
                exhausted(Exhausted::Steps)
            );
            assert_eq!(budget.remaining(), 0);
            assert_eq!(budget.exhausted(), Some(Exhausted::Steps));
            assert_eq!(ordinary(&mut agent, realm, "6 * 7"), Ok(Some(42.0)));
        }
        let (mut agent, realm) = checked(stress);
        let budget = Rc::new(Budget::new(128));
        assert_eq!(
            within(
                &mut agent,
                realm,
                "var ticks = 0; for (;;) ticks++;",
                budget
            ),
            exhausted(Exhausted::Steps)
        );
        assert_eq!(
            ordinary(&mut agent, realm, "ticks > 0 ? 1 : 0"),
            Ok(Some(1.0)),
            "already executed side effects remain"
        );
    }
}

#[test]
fn callbacks_and_parameter_initializers_cannot_start_fresh_quotas() {
    // Finite work well above the small quota also makes a broken sharing
    // implementation fail promptly instead of leaving a test hung forever.
    for stress in [false, true] {
        for source in [
            "var o = { get x() { for (var i = 0; i < 512; i++) {} return 1; } }; o.x;",
            "var o = { set x(v) { for (var i = 0; i < 512; i++) {} } }; o.x = 1;",
            "var o = { valueOf() { for (var i = 0; i < 512; i++) {} return 1; } }; +o;",
            "var o = { toString() { for (var i = 0; i < 512; i++) {} return 'x'; } }; '' + o;",
            "function burn() { for (var i = 0; i < 512; i++) {} return 1; } function f(a = burn(), ...rest) { return a; } f();",
            "function f(a, ...rest) { arguments[0] = a; for (var i = 0; i < 512; i++) {} return rest.length; } f(1, 2, 3);",
        ] {
            let (mut agent, realm) = checked(stress);
            let budget = Rc::new(Budget::new(128));
            assert_eq!(
                within(&mut agent, realm, source, Rc::clone(&budget)),
                exhausted(Exhausted::Steps),
                "{source}"
            );
            assert_eq!(
                budget.remaining(),
                0,
                "the caller ledger was charged: {source}"
            );
            assert_eq!(budget.exhausted(), Some(Exhausted::Steps));
            assert_eq!(ordinary(&mut agent, realm, "21 + 21"), Ok(Some(42.0)));
        }
    }
}

#[test]
fn consecutive_scripts_share_and_do_not_replenish_an_active_budget() {
    let (mut agent, realm) = checked(false);
    let budget = Rc::new(Budget::new(128));
    agent.with_execution_budget(Rc::clone(&budget), |agent| {
        assert_eq!(
            ordinary(agent, realm, "var saved = 4; saved"),
            Ok(Some(4.0))
        );
        assert!(budget.remaining() < 128 && budget.remaining() > 0);
        assert_eq!(
            ordinary(agent, realm, "for (;;) {}"),
            exhausted(Exhausted::Steps)
        );
        assert_eq!(
            ordinary(agent, realm, "saved = 9;"),
            exhausted(Exhausted::Steps)
        );
    });
    assert_eq!(budget.remaining(), 0);
    assert_eq!(ordinary(&mut agent, realm, "saved"), Ok(Some(4.0)));
}

#[test]
fn every_distinct_parent_and_child_quota_remains_effective() {
    for (parent_steps, child_steps) in [(64, 1024), (1024, 64)] {
        let (mut agent, realm) = checked(false);
        let parent = Rc::new(Budget::new(parent_steps));
        let child = Rc::new(Budget::new(child_steps));
        agent.with_execution_budget(Rc::clone(&parent), |agent| {
            agent.with_execution_budget(Rc::clone(&child), |agent| {
                assert_eq!(
                    ordinary(agent, realm, "for (;;) {}"),
                    exhausted(Exhausted::Steps)
                );
            });
            if parent_steps < child_steps {
                assert_eq!(parent.remaining(), 0);
                assert!(child.remaining() > 0 && child.remaining() < child_steps);
                assert_eq!(ordinary(agent, realm, "11;"), exhausted(Exhausted::Steps));
            } else {
                assert_eq!(child.remaining(), 0);
                assert!(parent.remaining() > 0 && parent.remaining() < parent_steps);
                assert_eq!(
                    ordinary(agent, realm, "11;"),
                    Ok(Some(11.0)),
                    "finished child scope does not poison its parent"
                );
            }
        });
        assert_eq!(ordinary(&mut agent, realm, "17;"), Ok(Some(17.0)));
    }
}

#[test]
fn installing_the_same_rc_multiple_times_does_not_charge_twice() {
    let (mut baseline_agent, baseline_realm) = checked(false);
    let baseline = Rc::new(Budget::new(100));
    assert_eq!(
        within(
            &mut baseline_agent,
            baseline_realm,
            "7;",
            Rc::clone(&baseline)
        ),
        Ok(Some(7.0))
    );
    let (mut agent, realm) = checked(false);
    let budget = Rc::new(Budget::new(100));
    agent.with_execution_budget(Rc::clone(&budget), |agent| {
        agent.with_execution_budget(Rc::clone(&budget), |agent| {
            assert_eq!(
                within(agent, realm, "7;", Rc::clone(&budget)),
                Ok(Some(7.0))
            );
        });
    });
    assert_eq!(budget.remaining(), baseline.remaining());
}

#[test]
fn a_shared_scope_observes_cancellation_between_scripts() {
    let (mut agent, realm) = checked(false);
    let cancel = Cancel::new();
    let budget = Rc::new(Budget::with_cancel(100, cancel.clone()));
    agent.with_execution_budget(Rc::clone(&budget), |agent| {
        assert_eq!(
            ordinary(agent, realm, "var saved = 4; saved"),
            Ok(Some(4.0))
        );
        let before_cancel = budget.remaining();
        std::thread::spawn(move || cancel.cancel())
            .join()
            .expect("cancel thread");
        assert_eq!(
            ordinary(agent, realm, "saved = 9;"),
            exhausted(Exhausted::Cancelled)
        );
        assert_eq!(budget.remaining(), before_cancel);
    });
    assert_eq!(ordinary(&mut agent, realm, "saved"), Ok(Some(4.0)));
}

#[test]
fn exhaustion_is_not_a_language_throw_and_real_throws_still_work() {
    let (mut agent, realm) = checked(false);
    let budget = Rc::new(Budget::new(64));
    assert_eq!(
        within(&mut agent, realm, "for (;;) {}", budget),
        exhausted(Exhausted::Steps)
    );
    agent.scope(|agent, scope| {
        let result = agent.evaluate_script(scope, realm, &units("throw 9;"));
        assert!(matches!(result, Err(ScriptError::Abrupt(Abrupt::Throw))));
        let Some(crate::agent::Exception::Value(value)) = agent.take_exception(scope) else {
            panic!("the script throw retains its value");
        };
        assert_eq!(agent.heap().get(value).as_number(), Some(9.0));
        assert!(agent.take_exception(scope).is_none());
    });
    clean(&mut agent);
}

#[test]
fn ordinary_evaluation_has_a_finite_default_and_gets_a_fresh_quota() {
    let (mut agent, realm) = checked(false);
    // Finite even if the default were accidentally disabled.
    let source = format!("var n = 0; for (; n <= {DEFAULT_EXECUTION_STEPS}; n++) {{}} n");
    assert_eq!(
        ordinary(&mut agent, realm, &source),
        exhausted(Exhausted::Steps)
    );
    assert_eq!(ordinary(&mut agent, realm, "42;"), Ok(Some(42.0)));
}

#[test]
fn native_function_calls_outside_script_evaluation_get_a_default_quota() {
    let (mut agent, realm) = checked(false);
    // Finite work above the default quota keeps a broken implementation
    // testable without relying on an unbounded function returning.
    let source = units(&format!(
        "(function() {{ for (var i = 0; i < {DEFAULT_EXECUTION_STEPS}; i++) {{}} return 7; }})"
    ));
    let result = agent.scope(|agent, scope| {
        let function = agent
            .evaluate_script(scope, realm, &source)
            .expect("function object");
        // evaluate_script has returned; its default scope is no longer active.
        let this = agent.heap().local(scope, crate::value::Value::UNDEFINED);
        crate::operations::call(agent, scope, function, this, &[]).map(|_| ())
    });
    assert_eq!(result, Err(Abrupt::Exhausted(Exhausted::Steps)));
    clean(&mut agent);
    assert_eq!(ordinary(&mut agent, realm, "42;"), Ok(Some(42.0)));
}
