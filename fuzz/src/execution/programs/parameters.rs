//! Independent, small arithmetic oracles for closures and parameter lists.
//! Every source is fixed syntax with bounded numbers or escaped UTF-16.

use super::{ErrorKind, Outcome, Primitive, Step, literal, units};

#[cfg(test)]
mod tests;

fn value(source: &str, expected: Primitive) -> Step {
    Step {
        source: source.to_owned(),
        expected: Outcome::Value(expected),
    }
}

pub(super) fn program(family: usize, a: i32, b: i32, c: i32, raw: [u8; 3]) -> Vec<Step> {
    let [x, y, z] = [a, b, c].map(f64::from);
    match family {
        32 => vec![
            Step::number(
                format!(
                    "function make(p) {{ return function(q) {{ p += q; return p; }}; }} var f = make(({a})); f(({b}))"
                ),
                x + y,
            ),
            Step::number(format!("f(({c}))"), x + y + z),
            Step::number(format!("f(({a}))"), 2.0 * x + y + z),
        ],
        33 => vec![
            Step::number(
                format!(
                    "function make(p) {{ return {{add(q) {{ p += q; return p; }}, get() {{ return p; }}}}; }} var o = make(({a})); o.add(({b}))"
                ),
                x + y,
            ),
            Step::number("o.get()".to_owned(), x + y),
            Step::number(format!("o.add(({c}))"), x + y + z),
            Step::number("o.get()".to_owned(), x + y + z),
        ],
        34 => vec![
            Step::number(
                format!(
                    "function make(p) {{ return q => (p += q); }} var f = make(({a})); var g = make(({b})); f(({c}))"
                ),
                x + z,
            ),
            Step::number(format!("g(({c}))"), y + z),
            Step::number(format!("f(({b}))"), x + z + y),
            Step::number(format!("g(({a}))"), y + z + x),
        ],
        35 => vec![
            Step::number(
                format!(
                    "function outer(p) {{ let q = ({b}); return function middle(r) {{ return function(s) {{ p += s; q += r; return p * 1000 + q; }}; }}; }} var f = outer(({a}))(({c})); f(({b}))"
                ),
                (x + y) * 1000.0 + y + z,
            ),
            Step::number(format!("f(({c}))"), (x + y + z) * 1000.0 + y + 2.0 * z),
        ],
        36 => vec![
            Step::error(
                format!("var f; function make() {{ f = () => p; f(); let p = ({a}); }} make()"),
                ErrorKind::ReferenceError,
            ),
            Step::error("f()".to_owned(), ErrorKind::ReferenceError),
            Step::number(format!("f = () => ({b}) + ({c}); f()"), y + z),
        ],
        37..=39 => {
            let count = i32::from(raw[2] % 9);
            let (loop_source, captured, expected) = match family {
                37 => (
                    "let",
                    format!("i + ({a})"),
                    count * a + count * (count - 1) / 2,
                ),
                38 => ("var", format!("i + ({a})"), count * (count + a)),
                _ => ("var", "p".to_owned(), count * a + count * (count - 1) / 2),
            };
            let binding = if family == 39 {
                format!("const p = i + ({a});")
            } else {
                String::new()
            };
            vec![Step::number(
                format!(
                    "var fs = []; for ({loop_source} i = 0; i < {count}; i++) {{ {binding} fs[i] = () => {captured}; }} var total = 0; for (var j = 0; j < {count}; j++) total += fs[j](); total"
                ),
                f64::from(expected),
            )]
        }
        40 => vec![
            Step::number(
                format!(
                    "var o = {{v: ({a}), make(p) {{ return q => this.v + p + q; }}}}; var f = o.make(({b})); f(({c}))"
                ),
                x + y + z,
            ),
            Step::number(
                format!("o.v = ({c}); var other = {{v: 999, f}}; other.f(({a}))"),
                z + y + x,
            ),
        ],
        41 => vec![
            Step::number(
                format!(
                    "function make(p) {{ return () => arguments[0]; }} var g = make(({a})); g()"
                ),
                x,
            ),
            Step::number(format!("var o = {{g}}; o.g(({b}))"), x),
            Step::number(
                format!(
                    "function h(p) {{ return q => arguments[0] + q; }} var k = h(({b})); k(({c}))"
                ),
                y + z,
            ),
        ],
        42 => vec![
            Step::number(
                format!(
                    "function make(p) {{ return (q = p) => {{ p += q; return p; }}; }} var f = make(({a})); f()"
                ),
                2.0 * x,
            ),
            Step::number(format!("f(({b}))"), 2.0 * x + y),
            Step::number("f()".to_owned(), 4.0 * x + 2.0 * y),
        ],
        43 => vec![
            Step::number(
                format!(
                    "function make(p) {{ return (q, ...rest) => arguments[0] + q + rest.length * 1000 + (rest[0] ?? 0); }} var f = make(({a})); f(({b}), ({c}))"
                ),
                x + y + 1000.0 + z,
            ),
            Step::number(format!("f(({c}))"), x + z),
        ],
        44 => vec![
            Step::number(
                format!("function f(p) {{ arguments[0] = ({b}); return p; }} f(({a}))"),
                y,
            ),
            Step::number(
                format!("function g(p) {{ p = ({c}); return arguments[0]; }} g(({a}))"),
                z,
            ),
            Step::number(
                format!(
                    "function h(p) {{ arguments[0] = ({b}); p += ({c}); return arguments[0]; }} h(({a}))"
                ),
                y + z,
            ),
        ],
        45 => vec![
            Step::number(
                format!(
                    "function f(p) {{ 'use strict'; p = ({b}); return arguments[0]; }} f(({a}))"
                ),
                x,
            ),
            Step::number(
                format!(
                    "function g(p) {{ 'use strict'; arguments[0] = ({c}); return p; }} g(({a}))"
                ),
                x,
            ),
        ],
        46 => vec![
            Step::number(
                format!(
                    "function f(p, p) {{ arguments[0] = ({a}); arguments[1] = ({c}); return p * 1000 + arguments[0]; }} f(({a}), ({b}))"
                ),
                z * 1000.0 + x,
            ),
            Step::number(
                format!(
                    "function g(p, p) {{ p = ({c}); return arguments[0] * 1000 + arguments[1]; }} g(({a}), ({b}))"
                ),
                x * 1000.0 + z,
            ),
            Step::number(
                format!("function h(p, p) {{ p = ({c}); return arguments[0]; }} h(({a}))"),
                x,
            ),
        ],
        47 => vec![
            Step::number(
                format!(
                    "function f(p, q) {{ q = ({b}); arguments[1] = ({c}); return q * 1000 + arguments.length; }} f(({a}))"
                ),
                y * 1000.0 + 1.0,
            ),
            Step::number(format!("f(({a}), undefined)"), z * 1000.0 + 2.0),
            Step::number(format!("f(({a}), ({b}), ({c}))"), z * 1000.0 + 3.0),
        ],
        48 => vec![
            Step::number(
                format!(
                    "function f(p) {{ delete arguments[0]; p = ({b}); arguments[0] = ({c}); return p * 1000 + arguments[0]; }} f(({a}))"
                ),
                y * 1000.0 + z,
            ),
            Step::number(
                format!(
                    "function g(p) {{ arguments.length = 0; p = ({c}); return arguments[0]; }} g(({a}))"
                ),
                z,
            ),
            value(
                &format!(
                    "function h(p) {{ delete arguments[0]; return 0 in arguments; }} h(({a}))"
                ),
                Primitive::Boolean(false),
            ),
        ],
        49 => vec![
            Step::number(
                format!(
                    "function make(p) {{ return {{args: arguments, set(q) {{ p = q; }}, get() {{ return p; }}}}; }} var o = make(({a})); o.args[0]"
                ),
                x,
            ),
            Step::number(format!("o.set(({b})); o.args[0]"), y),
            Step::number(format!("o.args[0] = ({c}); o.get()"), z),
        ],
        50 => vec![
            Step::number(
                format!(
                    "var hits = 0; function f(p = (hits = hits * 10 + 1, ({a})), q = (hits = hits * 10 + 2, p + ({b}))) {{ return p + q; }} f()"
                ),
                2.0 * x + y,
            ),
            Step::number("hits".to_owned(), 12.0),
            Step::number(format!("f(({c}))"), 2.0 * z + y),
            Step::number("hits".to_owned(), 122.0),
        ],
        51 => vec![
            Step::number(
                format!("var hits = 0; function f(p = (hits++, ({a}))) {{ return p; }} f()"),
                x,
            ),
            Step::number("f(undefined)".to_owned(), x),
            value("f(null)", Primitive::Null),
            value(
                "var zero = f(0); var no = f(false); zero === 0 && no === false && hits === 2",
                Primitive::Boolean(true),
            ),
        ],
        52 => vec![
            Step::number(
                format!(
                    "function f(p = ({a}), g = () => p) {{ var p = ({b}); return g; }} var h = f(); h()"
                ),
                x,
            ),
            Step::number("h()".to_owned(), x),
            Step::number(
                format!(
                    "function other(p = ({a}), g = () => p) {{ p = ({c}); return g; }} var j = other(); j()"
                ),
                z,
            ),
        ],
        53 => vec![
            Step::error(
                format!(
                    "var hits = 0; function f(p = (hits++, q), q = ({a})) {{ hits += 1000; return p; }} f()"
                ),
                ErrorKind::ReferenceError,
            ),
            Step::number("hits".to_owned(), 1.0),
            Step::number(format!("f(({b}), ({c}))"), y),
            Step::number("hits".to_owned(), 1001.0),
        ],
        54 => vec![
            Step::number(
                format!(
                    "function f(p, ...r) {{ arguments[0] = ({b}); return p * 1000 + r[0]; }} f(({a}), ({c}))"
                ),
                x * 1000.0 + z,
            ),
            Step::number(
                format!(
                    "function g(p, ...r) {{ r[0] = ({b}); return arguments[1]; }} g(({a}), ({c}))"
                ),
                z,
            ),
            Step::number(
                format!(
                    "function h(p, ...r) {{ arguments[1] = ({b}); return r[0]; }} h(({a}), ({c}))"
                ),
                z,
            ),
        ],
        55 => vec![
            value(
                &format!(
                    "function f(...r) {{ return r; }} var first = f(({a})); var second = f(({b})); first === second"
                ),
                Primitive::Boolean(false),
            ),
            Step::number("first[0]".to_owned(), x),
            Step::number(format!("first[0] = ({c}); second[0]"), y),
            Step::number("f().length".to_owned(), 0.0),
        ],
        56 => vec![
            Step::number(
                "function f(p, ...r) { return arguments.length * 100 + r.length; } f()".to_owned(),
                0.0,
            ),
            Step::number(format!("f(({a}))"), 100.0),
            Step::number(format!("f(({a}), ({b}), ({c}))"), 302.0),
        ],
        57 => vec![
            Step::number(format!("function f(p, q = ({a}), ...r) {{}} f.length"), 1.0),
            Step::number("function g(p, q, ...r) {} g.length".to_owned(), 2.0),
            Step::number("var h = (...r) => r; h.length".to_owned(), 0.0),
            Step::number(format!("function j(p = ({b}), q) {{}} j.length"), 0.0),
        ],
        58 => vec![
            Step::number(
                format!(
                    "function make(p = ({a}), g = () => p) {{ return {{g, set(q) {{ p = q; }}}}; }} var o = make(); o.g()"
                ),
                x,
            ),
            Step::number(format!("o.set(({b})); o.g()"), y),
            Step::number(format!("var other = make(({c})); other.g()"), z),
            Step::number("o.g()".to_owned(), y),
        ],
        59 => vec![
            Step::number(
                format!("function f(p, q = ({b})) {{ arguments[0] = ({c}); return p; }} f(({a}))"),
                x,
            ),
            Step::number(
                format!("function g(p, q = ({b})) {{ p = ({c}); return arguments[0]; }} g(({a}))"),
                x,
            ),
            Step::number(
                format!("function h(p = ({a})) {{ return arguments.length; }} h()"),
                0.0,
            ),
        ],
        60 => vec![
            Step::number(
                format!(
                    "var o = {{v: ({a}), get f() {{ return () => this.v + ({b}); }}}}; var f = o.f; f()"
                ),
                x + y,
            ),
            Step::number(format!("o.v = ({c}); f()"), z + y),
            Step::number("var other = {v: 999, f}; other.f()".to_owned(), z + y),
        ],
        61 => vec![
            Step::number(
                format!(
                    "function make(p) {{ return function(arguments) {{ return () => arguments; }}(({b})); }} var f = make(({a})); f()"
                ),
                y,
            ),
            Step::number(
                format!(
                    "function make2(p) {{ return arguments => () => arguments; }} var g = make2(({a}))(({c})); g()"
                ),
                z,
            ),
        ],
        62 => {
            let units = units(raw[0], raw[1], raw[2]);
            let text = literal(&units);
            vec![
                value(
                    &format!("function make(p = {text}) {{ return () => p; }} var f = make(); f()"),
                    Primitive::String(units.clone()),
                ),
                value(
                    &format!(
                        "function rest(...r) {{ return () => r[0]; }} var g = rest({text}); g()"
                    ),
                    Primitive::String(units.clone()),
                ),
                value("f()[0] + g()[1]", Primitive::String(units)),
            ]
        }
        63 => vec![
            Step {
                source: format!(
                    "function make(p) {{ return {{bad(q) {{ p += q; throw p; }}, good() {{ return p; }}}}; }} var o = make(({a})); o.bad(({b}))"
                ),
                expected: Outcome::Thrown(Primitive::number(x + y)),
            },
            Step::number("o.good()".to_owned(), x + y),
            Step {
                source: format!("o.bad(({c}))"),
                expected: Outcome::Thrown(Primitive::number(x + y + z)),
            },
            Step::number("o.good()".to_owned(), x + y + z),
        ],
        _ => unreachable!("parameter family {family}"),
    }
}
