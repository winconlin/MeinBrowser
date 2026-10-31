use super::*;
use crate::agent::{Abrupt, Exception};
use crate::bytecode::Emitter;
use crate::heap::{MAX_REGISTERS, Options};

fn checked() -> (Agent, RealmId) {
    let mut agent = Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    (agent, realm)
}

fn build(instructions: &[(Op, &[i64])], constants: Vec<Constant>, registers: u32) -> Code {
    let mut emitter = Emitter::default();
    for &(op, operands) in instructions {
        emitter.emit(op, operands).expect("passt");
    }
    emitter.finish(constants, registers, false)
}

/// Führt `code` mit `this` = 7 aus: die Zahl im Ergebnis oder die Art des
/// Fehlers der Engine. Danach ist der Registerstapel wieder leer.
fn run(code: &Code) -> std::result::Result<Option<f64>, Option<ErrorKind>> {
    let (mut agent, realm) = checked();
    let result = agent.scope(|agent, scope| {
        let this = agent.heap().local(scope, Value::int32(7));
        match execute(agent, scope, realm, code, this) {
            Ok(value) => Ok(agent.heap().get(value).as_number()),
            Err(Abrupt::Throw) => Err(match agent.take_exception(scope) {
                Some(Exception::Error(kind)) => Some(kind),
                _ => None,
            }),
            Err(Abrupt::OutOfMemory) => Err(None),
        }
    });
    assert_eq!(agent.heap_mut().push_registers(0), Some(0));
    result
}

#[test]
fn this_is_the_first_register() {
    let code = build(&[(Op::Ldar, &[0]), (Op::Return, &[])], Vec::new(), 1);
    assert_eq!(run(&code), Ok(Some(7.0)));
}

#[test]
fn wide_registers_and_constants_work() {
    let constants = (0..300)
        .map(|index| Constant::Number(f64::from(index) + 0.5))
        .collect();
    let code = build(
        &[
            (Op::LdaConstant, &[299]),
            (Op::Star, &[299]),
            (Op::LdaUndefined, &[]),
            (Op::Mov, &[299, 298]),
            (Op::Ldar, &[298]),
            (Op::Return, &[]),
        ],
        constants,
        300,
    );
    assert!(code.bytes().contains(&Op::Wide.byte()));
    assert_eq!(run(&code), Ok(Some(299.5)));
}

#[test]
fn jumps_follow_the_accumulator() {
    let mut emitter = Emitter::default();
    emitter.emit(Op::LdaSmi, &[1]).expect("passt");
    emitter.emit(Op::Star, &[1]).expect("passt");
    emitter.emit(Op::LdaNull, &[]).expect("passt");
    let not_nullish = emitter.jump(Op::JumpIfNotNullish).expect("passt");
    let nullish = emitter.jump(Op::JumpIfNullish).expect("passt");
    emitter.bind(not_nullish).expect("passt");
    emitter.emit(Op::LdaSmi, &[-1]).expect("passt");
    emitter.emit(Op::Return, &[]).expect("passt");
    emitter.bind(nullish).expect("passt");
    emitter.emit(Op::Ldar, &[1]).expect("passt");
    emitter.emit(Op::Return, &[]).expect("passt");
    let code = emitter.finish(Vec::new(), 2, false);
    assert_eq!(run(&code), Ok(Some(1.0)));
}

#[test]
fn a_frame_beyond_the_register_stack_throws_a_range_error() {
    let registers = u32::try_from(MAX_REGISTERS + 1).expect("passt");
    let code = build(&[(Op::Return, &[])], Vec::new(), registers);
    assert_eq!(run(&code), Err(Some(ErrorKind::RangeError)));
}

#[test]
fn bytecode_without_return_throws_a_range_error() {
    let code = build(&[(Op::LdaTrue, &[])], Vec::new(), 1);
    assert_eq!(run(&code), Err(Some(ErrorKind::RangeError)));
}

#[test]
fn engine_errors_have_their_kind() {
    let hole = build(
        &[
            (Op::LdaHole, &[]),
            (Op::Star, &[1]),
            (Op::CheckHole, &[1, 0]),
        ],
        vec![Constant::Name("x".encode_utf16().collect())],
        2,
    );
    assert_eq!(run(&hole), Err(Some(ErrorKind::ReferenceError)));
    let constant = build(
        &[(Op::ThrowConstAssignment, &[0])],
        vec![Constant::Name("c".encode_utf16().collect())],
        1,
    );
    assert_eq!(run(&constant), Err(Some(ErrorKind::TypeError)));
    let thrown = build(&[(Op::LdaSmi, &[3]), (Op::Throw, &[])], Vec::new(), 1);
    assert_eq!(run(&thrown), Err(None));
}
