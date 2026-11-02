//! Only generated, bounded programs are executed. Input bytes become small
//! numbers or explicit UTF-16 escapes, never arbitrary JavaScript syntax.
//!
//! Byte 0 modulo sixty-four selects a family; bytes 1..3 supply a, b and c.
//! Missing bytes are zero. Numeric values are i8, loop counts and recursive
//! arguments are c modulo nine. Closure and parameter families additionally
//! test captured bindings, argument mapping and parameter environments.

mod parameters;

use mb_js::agent::{Abrupt, Agent, ErrorKind, Exception};
use mb_js::heap::{Local, Options, Scope};
use mb_js::realm::RealmId;
use mb_js::script::ScriptError;
use mb_js::value::Kind;

const FAMILIES: usize = 64;
const MAX_SOURCE_UNITS: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Primitive {
    Undefined,
    Null,
    Boolean(bool),
    Number(u64),
    String(Vec<u16>),
}

impl Primitive {
    fn number(value: f64) -> Self {
        Self::Number(if value.is_nan() {
            f64::NAN.to_bits()
        } else {
            value.to_bits()
        })
    }

    fn string(value: &str) -> Self {
        Self::String(value.encode_utf16().collect())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Value(Primitive),
    Thrown(Primitive),
    Error(ErrorKind),
}

struct Step {
    source: String,
    expected: Outcome,
}

impl Step {
    fn number(source: String, expected: f64) -> Self {
        Self {
            source,
            expected: Outcome::Value(Primitive::number(expected)),
        }
    }

    fn error(source: String, kind: ErrorKind) -> Self {
        Self {
            source,
            expected: Outcome::Error(kind),
        }
    }
}

fn input(data: &[u8], index: usize) -> u8 {
    data.get(index).copied().unwrap_or(0)
}

fn units(a: u8, b: u8, c: u8) -> Vec<u16> {
    vec![
        u16::from_be_bytes([a, b]),
        if c & 1 == 0 {
            u16::from(c)
        } else {
            0xdc00 | u16::from(c)
        },
    ]
}

fn literal(units: &[u16]) -> String {
    let escaped: String = units.iter().map(|unit| format!("\\u{unit:04x}")).collect();
    format!("'{escaped}'")
}

fn program(data: &[u8]) -> Vec<Step> {
    let family = usize::from(input(data, 0)) % FAMILIES;
    let [raw_a, raw_b, raw_c] = [input(data, 1), input(data, 2), input(data, 3)];
    let [a, b, c] = [raw_a, raw_b, raw_c].map(|byte| i32::from(i8::from_ne_bytes([byte])));
    let [x, y, z] = [a, b, c].map(f64::from);
    match family {
        0 => vec![Step::number(
            format!("(({a}) + ({b})) * (({c}) - 1)"),
            (x + y) * (z - 1.0),
        )],
        1 => {
            let count = i32::from(raw_c % 9);
            let expected = a + b * count * (count - 1) / 2;
            vec![Step::number(
                format!("var s = ({a}); for (let i = 0; i < {count}; i++) {{ s += i * ({b}); }} s"),
                f64::from(expected),
            )]
        }
        2 => {
            let count = i32::from(raw_c % 9);
            let skip = i32::from(raw_a % 9);
            let stop = i32::from(raw_b % 9);
            let expected: i32 = (0..count)
                .take_while(|&i| i != stop)
                .filter(|&i| i != skip)
                .sum();
            vec![Step::number(
                format!(
                    "var s = 0; for (var i = 0; i < {count}; i++) {{ if (i == {stop}) break; if (i == {skip}) continue; s += i; }} s"
                ),
                f64::from(expected),
            )]
        }
        3 => vec![Step::number(
            format!("let x = ({a}); {{ let x = ({b}); x += ({c}); }} x"),
            x,
        )],
        4 => vec![
            Step::error(format!("const x = ({a}); x = ({b});"), ErrorKind::TypeError),
            Step::number("x".to_owned(), x),
        ],
        5 => vec![
            Step::error(
                format!("{{ x; let x = ({a}); }}"),
                ErrorKind::ReferenceError,
            ),
            Step {
                source: "typeof x".to_owned(),
                expected: Outcome::Value(Primitive::string("undefined")),
            },
        ],
        6 => {
            let present = raw_b & 1 != 0;
            let object = if present {
                format!("{{0: ({b})}}")
            } else {
                "null".to_owned()
            };
            vec![Step::number(
                format!(
                    "var hits = 0; var o = {object}; var v = o?.[hits++] ?? ({a}); hits * 1000 + v"
                ),
                if present { 1000.0 + y } else { x },
            )]
        }
        7 => vec![Step::number(
            format!(
                "var o = {{a: ({a}), b: ({b})}}; o.a += ({c}); delete o.b; o.a + ('b' in o ? 1000 : 0)"
            ),
            x + z,
        )],
        8 => vec![Step::number(
            format!(
                "var v = [({a}), , ({b})]; v[1] = ({c}); delete v[0]; v.length * 1000 + v[1] * 10 + (0 in v ? 1 : 0) + v[2]"
            ),
            3000.0 + z * 10.0 + y,
        )],
        9 => vec![Step::number(
            format!(
                "var v = [({a}), ({b}), ({c})]; v.length = 1; v[0] + v.length * 1000 + (1 in v ? 1 : 0)"
            ),
            x + 1000.0,
        )],
        10 => {
            let units = units(raw_a, raw_b, raw_c);
            vec![
                Step {
                    source: format!("let s = {}; s[0] + s[1]", literal(&units)),
                    expected: Outcome::Value(Primitive::String(units)),
                },
                Step::number("s.length".to_owned(), 2.0),
            ]
        }
        11 => vec![Step {
            source: format!("throw (({a}) + ({b}));"),
            expected: Outcome::Thrown(Primitive::number(x + y)),
        }],
        12 => {
            let units = units(raw_a, raw_b, raw_c);
            vec![Step {
                source: format!("throw {};", literal(&units)),
                expected: Outcome::Thrown(Primitive::String(units)),
            }]
        }
        13 => vec![
            Step::error(
                "'use strict'; var s = 'a'; s[0] = 'x';".to_owned(),
                ErrorKind::TypeError,
            ),
            Step {
                source: "s".to_owned(),
                expected: Outcome::Value(Primitive::string("a")),
            },
        ],
        14 => vec![
            Step::error(
                format!("var n = ({a}); var u; u[n++] = n++;"),
                ErrorKind::TypeError,
            ),
            Step::number("n".to_owned(), x + 2.0),
        ],
        15 => vec![
            Step::number(format!("var x = ({a}); let y = ({b}); x + y"), x + y),
            Step::error("let x;".to_owned(), ErrorKind::SyntaxError),
            Step::number("x + y".to_owned(), x + y),
            Step::number(format!("y = ({c}); x + y"), x + z),
        ],
        16 => vec![
            Step::number(
                format!("function f(p, q) {{ return p + q; }} f(({a}), ({b}))"),
                x + y,
            ),
            Step::number(format!("f(({b}), ({c}))"), y + z),
            Step::number(format!("f(({a}), ({c}))"), x + z),
        ],
        17 => vec![
            Step {
                source: format!("function f(p, q) {{ return typeof q; }} f(({a}))"),
                expected: Outcome::Value(Primitive::string("undefined")),
            },
            Step::number(
                format!("function g(p) {{ return p; }} g(({b}), ({a}), ({c}))"),
                y,
            ),
        ],
        18 => vec![
            Step::number(
                format!("function f(p, p) {{ return p; }} f(({a}), ({b}))"),
                y,
            ),
            Step {
                source: format!("f(({c}))"),
                expected: Outcome::Value(Primitive::Undefined),
            },
        ],
        19 => vec![
            Step::number(
                format!(
                    "var x = ({a}); function f(x) {{ var y = ({b}); return x + y; }} f(({c})) + x"
                ),
                z + y + x,
            ),
            Step::number(
                format!("let z = ({b}); function g() {{ return z; }} g()"),
                y,
            ),
        ],
        20 => vec![
            Step::number(
                format!(
                    "var o = {{v: ({a}), m(p) {{ return this.v + p; }}}}; (o.m)(({b})) + o['m'](({c}))"
                ),
                2.0 * x + y + z,
            ),
            Step::number(format!("o.v = ({a}) + ({c}); o.m(({b}))"), x + z + y),
        ],
        21 => vec![
            Step {
                source: "var o = {m() { return this; }}; var f = o.m; f() === globalThis"
                    .to_owned(),
                expected: Outcome::Value(Primitive::Boolean(true)),
            },
            Step {
                source: "function strict() { 'use strict'; return this; } strict()".to_owned(),
                expected: Outcome::Value(Primitive::Undefined),
            },
            Step {
                source: "o.m() === o".to_owned(),
                expected: Outcome::Value(Primitive::Boolean(true)),
            },
        ],
        22 => {
            let present = raw_b & 1 != 0;
            let object = if present {
                format!("{{v: ({a}), m(p) {{ return this.v + p; }}}}")
            } else {
                "{}".to_owned()
            };
            vec![Step::number(
                format!(
                    "var hits = 0; var o = {object}; var v = o.m?.((hits += 1, ({b}))) ?? ({c}); hits * 1000 + v"
                ),
                if present { 1000.0 + x + y } else { z },
            )]
        }
        23 => vec![
            Step::number(
                format!(
                    "var f = function named(p) {{ named = ({a}); return p + (named === f ? ({b}) : 1000); }}; f(({c}))"
                ),
                z + y,
            ),
            Step {
                source: "typeof named".to_owned(),
                expected: Outcome::Value(Primitive::string("undefined")),
            },
            Step::error(
                format!("var g = function fixed() {{ 'use strict'; fixed = ({a}); }}; g()"),
                ErrorKind::TypeError,
            ),
            Step::number(format!("f(({a}))"), x + y),
        ],
        24 => vec![
            Step::number(
                format!("var f = function arguments(arguments) {{ return arguments; }}; f(({a}))"),
                x,
            ),
            Step::number(
                format!(
                    "var g = function arguments() {{ let arguments = ({b}); return arguments; }}; g()"
                ),
                y,
            ),
            Step::number(
                format!("function h(arguments) {{ return arguments + ({c}); }} h(({b}))"),
                y + z,
            ),
        ],
        25 => {
            let count = i32::from(raw_c % 9);
            let sum = f64::from(count * (count + 1) / 2);
            vec![
                Step::number(
                    format!(
                        "var bias = ({a}); var f = function sum(n) {{ return n === 0 ? bias : n + sum(n - 1); }}; f({count})"
                    ),
                    x + sum,
                ),
                Step::number("f(0)".to_owned(), x),
                Step::number(format!("bias = ({b}); f({count})"), y + sum),
            ]
        }
        26 => {
            let units = units(raw_a, raw_b, raw_c);
            let string = literal(&units);
            vec![
                Step {
                    source: format!("function f() {{ return {string}; }} f()"),
                    expected: Outcome::Value(Primitive::String(units.clone())),
                },
                Step {
                    source: "f()[0] + f()[1]".to_owned(),
                    expected: Outcome::Value(Primitive::String(units.clone())),
                },
                Step {
                    source: format!("function g() {{ throw {string}; }} g()"),
                    expected: Outcome::Thrown(Primitive::String(units.clone())),
                },
                Step {
                    source: "f()".to_owned(),
                    expected: Outcome::Value(Primitive::String(units)),
                },
            ]
        }
        27 => vec![
            Step::number(
                format!(
                    "var o = {{v: ({a}), get x() {{ return this.v + ({b}); }}, set x(p) {{ this.v = p * 2; }}}}; o.x = ({c}); o.x"
                ),
                2.0 * z + y,
            ),
            Step::number(format!("o.x = ({a}); o.x"), 2.0 * x + y),
            Step::number("o.v".to_owned(), 2.0 * x),
        ],
        28 => vec![
            Step::number(
                "function C(p, q) {} C.length * 100 + (C.prototype.constructor === C ? 10 : 0) + ({__proto__: C.prototype} instanceof C ? 1 : 0)".to_owned(),
                211.0,
            ),
            Step {
                source: "C.name".to_owned(),
                expected: Outcome::Value(Primitive::string("C")),
            },
            Step::number("var o = {m(p) { return p; }}; o.m.length".to_owned(), 1.0),
            Step {
                source: "typeof o.m.prototype".to_owned(),
                expected: Outcome::Value(Primitive::string("undefined")),
            },
        ],
        29 => vec![
            Step::error(
                format!("var hits = ({a}); var notCallable = ({b}); notCallable(hits++, hits++);"),
                ErrorKind::TypeError,
            ),
            Step::number("hits".to_owned(), x + 2.0),
            Step::number(
                "function f(p, q) { return p * 1000 + q; } f(hits++, hits++)".to_owned(),
                (x + 2.0) * 1000.0 + x + 3.0,
            ),
            Step::number("hits".to_owned(), x + 4.0),
        ],
        30 => vec![
            Step::error(
                format!(
                    "var hits = ({a}); function f(p) {{ hits += 1; const x = p; x = ({b}); return x; }} f(({c}))"
                ),
                ErrorKind::TypeError,
            ),
            Step::number("hits".to_owned(), x + 1.0),
            Step::number(format!("function good(p) {{ return p + ({c}); }} good(({b}))"), y + z),
            Step::number(format!("good(({a}))"), x + z),
        ],
        31 => vec![
            Step::number(
                format!("function f(p) {{ return g(p); function g(q) {{ return q + ({b}); }} }} f(({a}))"),
                x + y,
            ),
            Step::number(format!("f(({c}))"), z + y),
            Step::number(
                format!("function factory() {{ return function(p) {{ return p + ({a}); }}; }} factory()(({b}))"),
                x + y,
            ),
        ],
        _ => parameters::program(family, a, b, c, [raw_a, raw_b, raw_c]),
    }
}

fn snapshot(agent: &Agent, value: Local<'_>) -> Primitive {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::Undefined => Primitive::Undefined,
        Kind::Null => Primitive::Null,
        Kind::Boolean(value) => Primitive::Boolean(value),
        Kind::Number(value) => Primitive::number(value),
        Kind::String(value) => Primitive::String(heap.string_units(value)),
        kind => panic!("generated program returned unexpected kind: {kind:?}"),
    }
}

fn evaluate(agent: &mut Agent, scope: &Scope, realm: RealmId, source: &str) -> Outcome {
    let source: Vec<u16> = source.encode_utf16().collect();
    assert!(source.len() <= MAX_SOURCE_UNITS, "bounded generated source");
    let result = agent.evaluate_script(scope, realm, &source);
    // A normal result is rooted by its Local; a thrown value by the Agent.
    // Check both after relocation, before extracting their content.
    agent.heap_mut().collect();
    let outcome = match result {
        Ok(value) => Outcome::Value(snapshot(agent, value)),
        Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
            Some(Exception::Error(kind)) => Outcome::Error(kind),
            Some(Exception::Value(value)) => Outcome::Thrown(snapshot(agent, value)),
            None => panic!("throw without an exception"),
        },
        Err(error) => panic!("generated program must be supported and fit its budget: {error:?}"),
    };
    assert!(
        agent.take_exception(scope).is_none(),
        "exception consumed once"
    );
    outcome
}

fn run(steps: &[Step], stress: bool) -> Vec<Outcome> {
    let mut agent = Agent::with_options(Options {
        limit: 1 << 20,
        stress,
        verify: true,
    })
    .expect("bounded generated-program agent");
    let realm = agent
        .create_realm()
        .expect("bounded generated-program realm");
    let mut actual = Vec::new();
    for step in steps {
        let outcome = agent.scope(|agent, scope| evaluate(agent, scope, realm, &step.source));
        assert_eq!(
            outcome, step.expected,
            "generated script differs from its oracle"
        );
        agent.heap_mut().collect();
        agent
            .heap()
            .verify()
            .expect("valid heap after generated script");
        actual.push(outcome);
    }
    actual
}

pub(super) fn exercise(data: &[u8]) {
    let steps = program(data);
    let normal = run(&steps, false);
    assert_eq!(
        normal,
        run(&steps, true),
        "GC timing preserves script results"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_runs_with_boundary_operands_and_both_collectors() {
        for family in 0..FAMILIES as u8 {
            for operands in [[0, 0, 0], [127, 128, 255], [128, 255, 8]] {
                exercise(&[family, operands[0], operands[1], operands[2]]);
            }
        }
    }

    #[test]
    fn bounded_loops_cover_empty_full_break_and_continue_paths() {
        for count in 0..=8 {
            exercise(&[1, 250, 3, count]);
            exercise(&[2, 1, 8, count]);
            exercise(&[2, 8, 3, count]);
        }
        let steps = program(&[1, 250, 3, 8]);
        assert_eq!(steps[0].expected, Outcome::Value(Primitive::number(78.0)));
    }

    #[test]
    fn utf16_strings_and_thrown_values_preserve_lone_surrogates() {
        for family in [10, 12] {
            let steps = program(&[family, 0xd8, 0, 0]);
            let actual = run(&steps, true);
            let string = Primitive::String(vec![0xd800, 0]);
            let expected = if family == 10 {
                Outcome::Value(string)
            } else {
                Outcome::Thrown(string)
            };
            assert_eq!(actual[0], expected);
        }
    }

    #[test]
    fn failed_writes_and_declaration_conflicts_preserve_following_script_state() {
        let steps = program(&[14, 127, 0, 0]);
        let actual = run(&steps, true);
        assert_eq!(
            actual,
            [
                Outcome::Error(ErrorKind::TypeError),
                Outcome::Value(Primitive::number(129.0))
            ]
        );
        let steps = program(&[15, 2, 3, 4]);
        assert_eq!(
            run(&steps, true),
            [
                Outcome::Value(Primitive::number(5.0)),
                Outcome::Error(ErrorKind::SyntaxError),
                Outcome::Value(Primitive::number(5.0)),
                Outcome::Value(Primitive::number(6.0)),
            ]
        );
    }

    #[test]
    fn bounded_function_recursion_checks_each_depth_and_cached_global_reads() {
        for depth in 0..=8 {
            exercise(&[25, 250, 127, depth]);
        }
        let steps = program(&[25, 250, 127, 8]);
        assert_eq!(
            run(&steps, true),
            [
                Outcome::Value(Primitive::number(30.0)),
                Outcome::Value(Primitive::number(-6.0)),
                Outcome::Value(Primitive::number(163.0)),
            ]
        );
    }

    #[test]
    fn arguments_bindings_and_function_names_follow_distinct_shadowing_rules() {
        let steps = program(&[24, 128, 255, 127]);
        assert_eq!(
            run(&steps, true),
            [
                Outcome::Value(Primitive::number(-128.0)),
                Outcome::Value(Primitive::number(-1.0)),
                Outcome::Value(Primitive::number(126.0)),
            ]
        );
        let steps = program(&[23, 2, 3, 4]);
        assert_eq!(
            run(&steps, true),
            [
                Outcome::Value(Primitive::number(7.0)),
                Outcome::Value(Primitive::string("undefined")),
                Outcome::Error(ErrorKind::TypeError),
                Outcome::Value(Primitive::number(5.0)),
            ]
        );
    }

    #[test]
    fn failed_calls_keep_argument_side_effects_and_allow_later_calls() {
        let steps = program(&[29, 127, 0, 0]);
        assert_eq!(
            run(&steps, true),
            [
                Outcome::Error(ErrorKind::TypeError),
                Outcome::Value(Primitive::number(129.0)),
                Outcome::Value(Primitive::number(129_130.0)),
                Outcome::Value(Primitive::number(131.0)),
            ]
        );
        let steps = program(&[30, 2, 3, 4]);
        assert_eq!(
            run(&steps, true),
            [
                Outcome::Error(ErrorKind::TypeError),
                Outcome::Value(Primitive::number(3.0)),
                Outcome::Value(Primitive::number(7.0)),
                Outcome::Value(Primitive::number(6.0)),
            ]
        );
    }

    #[test]
    fn returned_and_thrown_surrogates_and_accessor_values_survive_collection() {
        let steps = program(&[26, 0xd8, 0, 0]);
        let string = Primitive::String(vec![0xd800, 0]);
        assert_eq!(
            run(&steps, true),
            [
                Outcome::Value(string.clone()),
                Outcome::Value(string.clone()),
                Outcome::Thrown(string.clone()),
                Outcome::Value(string),
            ]
        );
        let steps = program(&[27, 2, 3, 4]);
        assert_eq!(
            run(&steps, true),
            [
                Outcome::Value(Primitive::number(11.0)),
                Outcome::Value(Primitive::number(7.0)),
                Outcome::Value(Primitive::number(4.0)),
            ]
        );
    }
}
