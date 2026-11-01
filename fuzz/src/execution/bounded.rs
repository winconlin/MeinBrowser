//! Arbitrary Script sources under a host-owned execution quota.
//!
//! Each encoding runs in fresh normal/stress agents. Opaque objects are
//! classified without inspecting properties or invoking user code. Memory
//! exhaustion can legitimately differ with GC/cache timing; invariant
//! failures still propagate, and all other outcomes must agree.

use std::rc::Rc;

use mb_js::agent::{Abrupt, Agent, Budget, ErrorKind, Exception, Exhausted};
use mb_js::heap::{Local, Options};
use mb_js::parser::SyntaxError;
use mb_js::script::ScriptError;
use mb_js::value::Kind;

const SCRIPT_STEPS: u64 = 512;
const HEAP_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Undefined,
    Null,
    Boolean(bool),
    Number(u64),
    String(Vec<u16>),
    Object,
    Symbol,
    BigInt,
}

impl Value {
    fn number(number: f64) -> Self {
        Self::Number(if number.is_nan() {
            f64::NAN.to_bits()
        } else {
            number.to_bits()
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Value(Value),
    Thrown(Value),
    Error(ErrorKind),
    Syntax(SyntaxError),
    Unsupported(usize),
    Exhausted(Exhausted),
    OutOfMemory,
}

fn snapshot(agent: &Agent, local: Local<'_>) -> Value {
    let heap = agent.heap();
    match heap.get(local).kind() {
        Kind::Undefined => Value::Undefined,
        Kind::Null => Value::Null,
        Kind::Boolean(value) => Value::Boolean(value),
        Kind::Number(value) => Value::number(value),
        Kind::String(value) => Value::String(heap.string_units(value)),
        Kind::Object(_) => Value::Object,
        Kind::Symbol(_) => Value::Symbol,
        Kind::BigInt(_) => Value::BigInt,
        kind => panic!("internal value escaped script evaluation: {kind:?}"),
    }
}

fn run(source: &[u16], stress: bool) -> Outcome {
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress,
        verify: true,
    })
    .expect("bounded script agent");
    let realm = agent.create_realm().expect("bounded script realm");
    let budget = Rc::new(Budget::new(SCRIPT_STEPS));
    let outcome = agent.scope(|agent, scope| {
        let result = agent.evaluate_script_within(scope, realm, source, Rc::clone(&budget));
        // Results stay in a Local; thrown values stay in the Agent until
        // take_exception. Read both only after moving collection.
        agent.heap_mut().collect();
        let outcome = match result {
            Ok(value) => Outcome::Value(snapshot(agent, value)),
            Err(ScriptError::Syntax(error)) => {
                assert!(error.position <= source.len());
                Outcome::Syntax(error)
            }
            Err(ScriptError::Unsupported(position))
            | Err(ScriptError::Abrupt(Abrupt::Unsupported(position))) => {
                assert!(position <= source.len());
                Outcome::Unsupported(position)
            }
            Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
                Some(Exception::Error(kind)) => Outcome::Error(kind),
                Some(Exception::Value(value)) => Outcome::Thrown(snapshot(agent, value)),
                None => panic!("script threw without an exception"),
            },
            Err(ScriptError::Abrupt(Abrupt::Exhausted(reason))) => {
                assert_eq!(budget.exhausted(), Some(reason), "host quota was shared");
                if reason == Exhausted::Steps {
                    assert_eq!(budget.remaining(), 0);
                }
                Outcome::Exhausted(reason)
            }
            Err(ScriptError::Abrupt(Abrupt::OutOfMemory)) => Outcome::OutOfMemory,
        };
        assert!(agent.take_exception(scope).is_none(), "no stale exception");
        outcome
    });
    agent.heap_mut().collect();
    agent
        .heap()
        .verify()
        .expect("valid heap after bounded source");
    outcome
}

fn compare(source: &[u16]) {
    let normal = run(source, false);
    let stress = run(source, true);
    // Collections can clear weak shape caches or retain different temporary
    // layouts. A threshold OOM is not a language-level differential result.
    if normal != Outcome::OutOfMemory && stress != Outcome::OutOfMemory {
        assert_eq!(normal, stress, "GC timing preserves bounded script outcome");
    }
}

pub(super) fn exercise(data: &[u8]) {
    let utf8: Vec<u16> = String::from_utf8_lossy(data).encode_utf16().collect();
    compare(&utf8);
    let utf16: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    compare(&utf16);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both(source: &str) -> Outcome {
        let source: Vec<u16> = source.encode_utf16().collect();
        let normal = run(&source, false);
        assert_eq!(normal, run(&source, true));
        normal
    }

    #[test]
    fn infinite_sources_and_callback_loops_exhaust_the_supplied_quota() {
        for source in [
            "for(;;){}",
            "function f(){return f();}f()",
            "var o={get x(){for(;;){}}};o.x",
            "var o={set x(v){for(;;){}}};o.x=1",
            "var o={valueOf(){for(;;){}}};+o",
            "function spin(){for(;;){}}function f(a=spin()){return a;}f()",
        ] {
            assert_eq!(
                both(source),
                Outcome::Exhausted(Exhausted::Steps),
                "{source}"
            );
        }
    }

    #[test]
    fn primitive_results_preserve_nan_signed_zero_and_utf16() {
        assert_eq!(both("1+2"), Outcome::Value(Value::number(3.0)));
        assert_eq!(both("-0"), Outcome::Value(Value::number(-0.0)));
        assert_eq!(both("0/0"), Outcome::Value(Value::number(f64::NAN)));
        assert_eq!(both("false"), Outcome::Value(Value::Boolean(false)));
        assert_eq!(both("null"), Outcome::Value(Value::Null));
        assert_eq!(both("void 0"), Outcome::Value(Value::Undefined));
        assert_eq!(
            both("'\\uD800\\u0000'"),
            Outcome::Value(Value::String(vec![0xd800, 0]))
        );
    }

    #[test]
    fn thrown_values_and_language_errors_are_distinct_from_host_aborts() {
        assert_eq!(both("throw 7"), Outcome::Thrown(Value::number(7.0)));
        assert_eq!(
            both("throw '\\uD800'"),
            Outcome::Thrown(Value::String(vec![0xd800]))
        );
        assert_eq!(both("throw {}"), Outcome::Thrown(Value::Object));
        assert_eq!(
            both("missingName"),
            Outcome::Error(ErrorKind::ReferenceError)
        );
        assert_eq!(both("null.x"), Outcome::Error(ErrorKind::TypeError));
        assert!(matches!(both("let = ;"), Outcome::Syntax(_)));
        assert!(matches!(both("class C {}"), Outcome::Unsupported(_)));
    }

    #[test]
    fn object_snapshots_do_not_invoke_accessors_or_conversions() {
        assert_eq!(
            both("({get x(){for(;;){}},valueOf(){for(;;){}}})"),
            Outcome::Value(Value::Object)
        );
    }

    #[test]
    fn logical_memory_exhaustion_is_an_expected_outcome() {
        assert_eq!(
            both("var s='x';for(var i=0;i<24;i++){s=s+s;}s"),
            Outcome::OutOfMemory
        );
    }

    #[test]
    fn both_encodings_and_empty_sources_reach_bounded_execution() {
        exercise(&[]);
        let units = [0x27_u16, 0xd800, 0x27, 0x3b];
        let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        exercise(&bytes);
        exercise(&[0xff, 0x80, 0]);
    }
}
