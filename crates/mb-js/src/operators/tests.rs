use super::*;
use crate::agent::{Abrupt, Exception};
use crate::heap::Options;
use crate::object::{array_create, ordinary_object_create};

fn checked() -> Agent {
    Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher")
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn same(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

fn thrown(agent: &mut Agent, scope: &Scope) -> Option<ErrorKind> {
    match agent.take_exception(scope) {
        Some(Exception::Error(kind)) => Some(kind),
        _ => None,
    }
}

/// Ein Operand: eine Zeichenkette oder ein Wert ohne Verweis.
#[derive(Clone, Copy)]
enum Operand {
    Text(&'static str),
    Plain(Value<'static>),
}

fn operand<'s>(agent: &mut Agent, scope: &'s Scope, operand: Operand) -> Local<'s> {
    match operand {
        Operand::Text(text) => agent
            .heap_mut()
            .alloc_string(scope, &units(text))
            .expect("Speicher"),
        Operand::Plain(value) => agent.heap().local(scope, value),
    }
}

fn apply<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    left: Operand,
    op: BinaryOp,
    right: Operand,
) -> Completion<Local<'s>> {
    let left = operand(agent, scope, left);
    let right = operand(agent, scope, right);
    apply_string_or_numeric_binary_operator(agent, scope, left, op, right)
}

fn string_units_of(agent: &Agent, string: Local<'_>) -> Option<Vec<u16>> {
    match agent.heap().get(string).kind() {
        Kind::String(string) => Some(agent.heap().string_units(string)),
        _ => None,
    }
}

use Operand::{Plain, Text};

fn number(value: f64) -> Operand {
    Plain(Value::number(value))
}

#[test]
fn addition_concatenates_when_either_side_is_a_string() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        for (left, right, expected) in [
            (Text("1"), number(2.0), "12"),
            (number(1.5), Text(""), "1.5"),
            (Text("a"), Plain(Value::UNDEFINED), "aundefined"),
            (Plain(Value::NULL), Text("b"), "nullb"),
            (Text("x"), number(-0.0), "x0"),
            (Plain(Value::TRUE), Text(""), "true"),
            (Text(""), Text(""), ""),
            (Text("€"), Text("1"), "€1"),
            (Text("ab"), Text("cd"), "abcd"),
        ] {
            let result = apply(agent, scope, left, BinaryOp::Add, right).expect("kein Fehler");
            assert_eq!(
                string_units_of(agent, result),
                Some(units(expected)),
                "{expected}"
            );
        }
        // Mit der leeren Zeichenkette bleibt die andere dieselbe.
        let text = operand(agent, scope, Text("Text"));
        let empty = operand(agent, scope, Text(""));
        for (left, right) in [(text, empty), (empty, text)] {
            let result =
                apply_string_or_numeric_binary_operator(agent, scope, left, BinaryOp::Add, right)
                    .expect("kein Fehler");
            assert_eq!(
                agent.heap().get(result).bits(),
                agent.heap().get(text).bits()
            );
        }
    });
}

#[test]
fn arithmetic_converts_to_numbers() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let cases = [
            (number(1.0), BinaryOp::Add, number(2.0), 3.0),
            (Plain(Value::TRUE), BinaryOp::Add, number(1.0), 2.0),
            (Plain(Value::NULL), BinaryOp::Add, number(1.0), 1.0),
            (
                Plain(Value::UNDEFINED),
                BinaryOp::Add,
                number(1.0),
                f64::NAN,
            ),
            (number(0.1), BinaryOp::Add, number(0.2), 0.1 + 0.2),
            (Text("3"), BinaryOp::Multiply, Text("4"), 12.0),
            (Text("8"), BinaryOp::Subtract, Plain(Value::TRUE), 7.0),
            (Text(""), BinaryOp::Subtract, number(1.0), -1.0),
            (
                Plain(Value::UNDEFINED),
                BinaryOp::Subtract,
                number(1.0),
                f64::NAN,
            ),
            (number(2.0), BinaryOp::Exponentiate, number(10.0), 1024.0),
            (Plain(Value::NULL), BinaryOp::Exponentiate, number(0.0), 1.0),
            (Text("-4"), BinaryOp::Remainder, number(2.0), -0.0),
            (number(1.0), BinaryOp::Divide, number(0.0), f64::INFINITY),
            (
                number(-1.0),
                BinaryOp::Divide,
                number(0.0),
                f64::NEG_INFINITY,
            ),
            (
                number(1.0),
                BinaryOp::Divide,
                number(-0.0),
                f64::NEG_INFINITY,
            ),
            (Text("a"), BinaryOp::Multiply, number(1.0), f64::NAN),
            (
                Text("1"),
                BinaryOp::LeftShift,
                number(31.0),
                -2_147_483_648.0,
            ),
            (number(-8.0), BinaryOp::SignedRightShift, number(1.0), -4.0),
            (
                number(-1.0),
                BinaryOp::UnsignedRightShift,
                number(0.0),
                4_294_967_295.0,
            ),
            (number(5.0), BinaryOp::BitwiseAnd, number(3.0), 1.0),
            (number(5.0), BinaryOp::BitwiseOr, Text("2"), 7.0),
            (number(5.0), BinaryOp::BitwiseXor, number(1.0), 4.0),
        ];
        for (position, (left, op, right, expected)) in cases.into_iter().enumerate() {
            let result = apply(agent, scope, left, op, right).expect("kein Fehler");
            let found = agent.heap().get(result).as_number();
            assert!(
                found.is_some_and(|found| same(found, expected)),
                "Fall {position}: {found:?} statt {expected}"
            );
        }
    });
}

#[test]
fn symbols_and_objects_without_conversions_throw() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        let empty = operand(agent, scope, Text(""));
        let one = operand(agent, scope, number(1.0));
        for (left, op, right) in [
            (symbol, BinaryOp::Add, empty),
            (empty, BinaryOp::Add, symbol),
            (symbol, BinaryOp::Add, one),
            (symbol, BinaryOp::Multiply, one),
            (object, BinaryOp::Add, one),
            (one, BinaryOp::Subtract, object),
        ] {
            assert_eq!(
                apply_string_or_numeric_binary_operator(agent, scope, left, op, right).err(),
                Some(Abrupt::Throw)
            );
            assert_eq!(thrown(agent, scope), Some(ErrorKind::TypeError));
        }
    });
}

#[test]
fn typeof_names_the_type() {
    let mut agent = checked();
    agent.scope(|agent, scope| {
        let text = operand(agent, scope, Text(""));
        let symbol = agent.new_symbol(scope, None).expect("Speicher");
        let object = ordinary_object_create(agent, scope, None).expect("Speicher");
        let array = array_create(agent, scope, 0, None).expect("kein Fehler");
        let heap = agent.heap();
        for (value, expected) in [
            (Value::UNDEFINED, "undefined"),
            (Value::NULL, "object"),
            (Value::TRUE, "boolean"),
            (Value::int32(1), "number"),
            (Value::number(f64::NAN), "number"),
            (heap.get(text), "string"),
            (heap.get(symbol), "symbol"),
            (heap.get(object), "object"),
            (heap.get(array), "object"),
        ] {
            assert_eq!(type_of(heap, value), expected);
        }
    });
}
