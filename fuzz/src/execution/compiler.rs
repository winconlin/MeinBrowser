//! Arbitrary sources are compiled and inspected, never executed here.
//!
//! The checks do not depend on the interpreter accepting malformed code:
//! instruction coverage, operand encoding, bounds and branch targets are
//! checked directly against the compiler's output.

use std::collections::BTreeSet;

use mb_js::bytecode::{Code, Op, Operand};
use mb_js::compiler::{CompileError, compile_script};
use mb_js::parser::parse_script;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Stats {
    instructions: usize,
    forward_jumps: usize,
    loop_jumps: usize,
    wide: usize,
    extra_wide: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Syntax,
    Unsupported,
    TooLarge,
    Compiled(Stats),
}

fn check_code(code: &Code, source_length: usize) -> Stats {
    let bytes = code.bytes();
    assert!(!bytes.is_empty(), "compiled scripts contain a return");
    assert!(code.register_count() >= 2, "this and completion registers");
    let mut stats = Stats::default();
    let mut starts = BTreeSet::new();
    let mut targets = Vec::new();
    let mut offset = 0;
    let mut last = None;
    while offset < bytes.len() {
        let instruction = code.decode(offset).expect("compiler emits decodable code");
        assert_eq!(instruction.start, offset);
        assert!(instruction.end > offset && instruction.end <= bytes.len());
        assert!(starts.insert(offset));
        if let Some(position) = code.position(offset) {
            assert!(position <= source_length, "source position is in bounds");
        }

        let (width, prefix) = match bytes[offset] {
            byte if byte == Op::Wide.byte() => {
                stats.wide += 1;
                (2, 1)
            }
            byte if byte == Op::ExtraWide.byte() => {
                stats.extra_wide += 1;
                (4, 1)
            }
            _ => (1, 0),
        };
        assert!(!matches!(instruction.op, Op::Wide | Op::ExtraWide));
        assert_eq!(bytes[offset + prefix], instruction.op.byte());
        let mut operand_at = offset + prefix + 1;
        assert_eq!(
            instruction.end,
            operand_at + width * instruction.op.operands().len()
        );
        for (index, &kind) in instruction.op.operands().iter().enumerate() {
            let encoded = &bytes[operand_at..operand_at + width];
            let bits = encoded.iter().enumerate().fold(0_u64, |bits, (at, byte)| {
                bits | (u64::from(*byte) << (at * 8))
            });
            let signed = matches!(kind, Operand::Immediate | Operand::Jump);
            let negative = signed && encoded[width - 1] & 0x80 != 0;
            let expected = bits as i64 - if negative { 1_i64 << (width * 8) } else { 0 };
            let value = instruction.operand(index);
            assert_eq!(value, expected, "operand matches its little-endian bytes");
            match kind {
                Operand::Register => {
                    let register = usize::try_from(value).expect("nonnegative register");
                    assert!(register < code.register_count(), "register is in frame");
                }
                Operand::Constant => {
                    let constant = usize::try_from(value).expect("nonnegative constant");
                    assert!(constant < code.constants().len(), "constant is in pool");
                    assert!(code.constant(value).is_some());
                }
                Operand::Jump => {
                    let target = instruction.target().expect("nonnegative jump target");
                    assert!(target < bytes.len(), "jump remains inside bytecode");
                    targets.push(target);
                    if instruction.op == Op::JumpLoop {
                        // An empty for(;;) can jump to its own instruction.
                        assert!(value <= 0);
                        stats.loop_jumps += 1;
                    } else {
                        assert!(value >= 0);
                        stats.forward_jumps += 1;
                    }
                }
                Operand::Immediate => {}
            }
            operand_at += width;
        }
        stats.instructions += 1;
        last = Some(instruction.op);
        offset = instruction.end;
    }
    assert_eq!(offset, bytes.len(), "instructions cover all bytecode bytes");
    assert_eq!(last, Some(Op::Return));
    for target in targets {
        assert!(
            starts.contains(&target),
            "jump targets an instruction boundary"
        );
    }
    assert!(code.decode(bytes.len()).is_none());
    assert!(code.decode(usize::MAX).is_none());
    stats
}

fn check_source(source: &[u16]) -> Outcome {
    let ast = match parse_script(source) {
        Ok(ast) => ast,
        Err(error) => {
            assert!(error.position <= source.len(), "syntax error position");
            return Outcome::Syntax;
        }
    };
    match compile_script(&ast) {
        Ok(script) => Outcome::Compiled(check_code(&script.code, source.len())),
        Err(CompileError::Unsupported(position)) => {
            assert!(position <= source.len(), "unsupported construct position");
            Outcome::Unsupported
        }
        Err(CompileError::TooLarge) => Outcome::TooLarge,
    }
}

pub(super) fn exercise(data: &[u8]) {
    let utf8: Vec<u16> = String::from_utf8_lossy(data).encode_utf16().collect();
    check_source(&utf8);
    let utf16: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    check_source(&utf16);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(source: &str) -> Stats {
        let source: Vec<u16> = source.encode_utf16().collect();
        match check_source(&source) {
            Outcome::Compiled(stats) => stats,
            outcome => panic!("expected supported source, got {outcome:?}"),
        }
    }

    #[test]
    fn loops_are_only_compiled_including_zero_distance_backedges() {
        for source in [
            "for(;;);",
            "while(true){}",
            "do {} while(true);",
            "for(let i=0;i<3;i++){if(i===1)continue;else break;}",
        ] {
            let stats = checked(source);
            assert!(stats.loop_jumps > 0);
        }
        assert!(checked("if(true){1;}else{2;}").forward_jumps > 0);
        assert_eq!(checked("").instructions, 2);
    }

    #[test]
    fn supported_literals_and_property_operations_have_valid_operands() {
        let stats = checked("var a=[1,,3]; let o={a:1,['b']:2}; o.c=a[2]; delete o.a; 'b' in o;");
        assert!(stats.instructions > 20);
        assert!(checked("throw 1;").instructions > 2);
    }

    #[test]
    fn compiler_widths_are_exercised_without_large_sources() {
        let source: String = (0..300).map(|value| format!("{value}.25;")).collect();
        assert!(source.len() <= crate::MAX_INPUT);
        let stats = checked(&source);
        assert!(stats.wide > 0, "constant indices pass 255");
        let stats = checked("127;128;32768;70000;");
        assert!(stats.wide > 0 && stats.extra_wide > 0);
    }

    #[test]
    fn syntax_and_unsupported_paths_preserve_positions() {
        for source in ["function f(){}", "class A{}", "try{}catch(e){}", "f();"] {
            let source: Vec<u16> = source.encode_utf16().collect();
            assert_eq!(check_source(&source), Outcome::Unsupported);
        }
        let source: Vec<u16> = "let = ;".encode_utf16().collect();
        assert_eq!(check_source(&source), Outcome::Syntax);
        let source = [u16::from(b'\''), 0xd800, u16::from(b'\''), u16::from(b';')];
        assert!(matches!(check_source(&source), Outcome::Compiled(_)));
        let bytes: Vec<u8> = source.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        exercise(&bytes);
        exercise(&[0xff, 0, 0x80]);
    }
}
