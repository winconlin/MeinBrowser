//! Arbitrary sources are compiled and inspected, never executed here.
//!
//! The checks do not depend on the interpreter accepting malformed code:
//! instruction coverage, operand encoding, bounds and branch targets are
//! checked directly against the compiler's output.

use std::collections::BTreeSet;
use std::rc::Rc;

use mb_js::ast::{Ast, Node};
use mb_js::bytecode::{Code, Constant, FunctionTemplate, Op, Operand};
use mb_js::compiler::{CompileError, compile_function, compile_script};
use mb_js::parser::parse_script;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Stats {
    instructions: usize,
    forward_jumps: usize,
    loop_jumps: usize,
    wide: usize,
    extra_wide: usize,
    calls: usize,
    functions: usize,
    unsupported_functions: usize,
    too_large_functions: usize,
}

impl Stats {
    fn include(&mut self, other: Self) {
        self.instructions += other.instructions;
        self.forward_jumps += other.forward_jumps;
        self.loop_jumps += other.loop_jumps;
        self.wide += other.wide;
        self.extra_wide += other.extra_wide;
        self.calls += other.calls;
    }
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
    assert!(
        code.register_count() >= 2,
        "this and completion/callee registers"
    );
    assert!(
        code.parameter_count()
            .checked_add(2)
            .is_some_and(|end| end <= code.register_count()),
        "parameters fit their frame after this and callee"
    );
    if let Some(register) = code.arguments_register() {
        assert!(
            register < code.register_count(),
            "arguments register is in frame"
        );
    }
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
        match instruction.op {
            Op::Call => {
                let callee = usize::try_from(instruction.operand(0)).expect("callee register");
                let arguments =
                    usize::try_from(instruction.operand(1)).expect("nonnegative argument count");
                assert!(
                    callee
                        .checked_add(2)
                        .and_then(|first| first.checked_add(arguments))
                        .is_some_and(|end| end <= code.register_count()),
                    "callee, this and arguments fit their frame"
                );
                stats.calls += 1;
            }
            Op::CreateClosure | Op::CreateClosureKeyed => {
                assert!(matches!(
                    code.constant(instruction.operand(0)),
                    Some(Constant::Function(_))
                ));
                if instruction.op == Op::CreateClosure {
                    assert!(matches!(
                        code.constant(instruction.operand(1)),
                        Some(Constant::Name(_))
                    ));
                }
            }
            _ => {}
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

fn check_error(error: CompileError, source_length: usize) -> bool {
    match error {
        CompileError::Unsupported(position) => {
            assert!(position <= source_length, "unsupported construct position");
            true
        }
        CompileError::TooLarge => false,
    }
}

/// Context layouts support structural equality, not a total ordering.
/// Preserve the complete template identity, including the enclosing
/// context chain, rather than relying on allocation addresses or IDs alone.
fn remember_template(seen: &mut Vec<FunctionTemplate>, template: &FunctionTemplate) -> bool {
    if seen.contains(template) {
        return false;
    }
    seen.push(template.clone());
    true
}

/// Follow the function templates the compiler exposes, including global
/// declarations whose templates are constructed during script evaluation.
/// Compilation does not invoke a function, even for a recursive source or
/// an infinite loop. An explicit worklist avoids recursion in the harness.
fn check_functions(
    ast: &Ast,
    source_length: usize,
    mut pending: Vec<FunctionTemplate>,
    stats: &mut Stats,
) {
    let mut seen = Vec::new();
    while let Some(template) = pending.pop() {
        if !remember_template(&mut seen, &template) {
            continue;
        }
        let function = ast
            .function(template.function)
            .expect("function belongs to AST");
        assert!(function.range.start <= function.range.end && function.range.end <= source_length);
        assert!(!template.binds_name || function.name.is_some());
        match compile_function(ast, &template) {
            Ok(code) => {
                let positional = ast
                    .list(function.parameters)
                    .iter()
                    .filter(|&&parameter| !matches!(ast.node(parameter), Node::Rest(_)))
                    .count();
                assert_eq!(code.parameter_count(), positional);
                stats.include(check_code(&code, source_length));
                stats.functions += 1;
                pending.extend(
                    code.constants()
                        .iter()
                        .filter_map(|constant| match constant {
                            Constant::Function(template) => Some(template.clone()),
                            _ => None,
                        }),
                );
            }
            Err(error) => {
                if check_error(error, source_length) {
                    stats.unsupported_functions += 1;
                } else {
                    stats.too_large_functions += 1;
                }
            }
        }
    }
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
        Ok(script) => {
            assert_eq!(
                script.code.parameter_count(),
                0,
                "scripts have no parameters"
            );
            let mut stats = check_code(&script.code, source.len());
            let mut pending: Vec<_> = script
                .functions
                .iter()
                .map(|(_, function)| FunctionTemplate {
                    function: *function,
                    binds_name: false,
                    outer: Rc::default(),
                })
                .collect();
            pending.extend(
                script
                    .code
                    .constants()
                    .iter()
                    .filter_map(|constant| match constant {
                        Constant::Function(template) => Some(template.clone()),
                        _ => None,
                    }),
            );
            check_functions(&ast, source.len(), pending, &mut stats);
            Outcome::Compiled(stats)
        }
        Err(error) => {
            if check_error(error, source.len()) {
                Outcome::Unsupported
            } else {
                Outcome::TooLarge
            }
        }
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
        for source in ["class A{}", "try{}catch(e){}", "f(...[]);"] {
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

    #[test]
    fn declared_and_nested_function_bodies_are_compiled_without_being_called() {
        let stats = checked(
            "function f(a){ function g(b){return b+1;} return g(a); } \
             let h=function own(x){return own(x);}; f(1); h(2);",
        );
        assert_eq!(stats.functions, 3);
        assert_eq!(stats.unsupported_functions, 0);
        assert!(stats.calls >= 4, "script and nested call frames checked");
        let stats = checked("function f(){ for(;;); } let g=function(){return g();};");
        assert_eq!(stats.functions, 2);
        assert!(
            stats.loop_jumps > 0,
            "an infinite function body is only compiled"
        );
        assert!(stats.calls > 0, "recursive calls are never executed");
    }

    #[test]
    fn unsupported_deferred_bodies_preserve_positions() {
        for source in [
            "function f(){try {} catch(e){}}",
            "function f([a]){return a;}",
        ] {
            let stats = checked(source);
            assert_eq!(stats.functions, 0);
            assert_eq!(stats.unsupported_functions, 1);
        }
        assert!(
            checked("f();").calls > 0,
            "unknown callees can still compile"
        );
    }

    #[test]
    fn defaults_rest_arguments_arrows_and_captures_are_supported() {
        for (source, expected_functions) in [
            ("function f(a=1,...r){return a+r.length;}", 1),
            ("function f(...r){return r.length;}", 1),
            ("function f(a){arguments[0]=2;return a;}", 1),
            ("let f=function(){return ()=>1;};", 2),
            (
                "function factory(a){return function inner(){return a;};}",
                2,
            ),
        ] {
            let stats = checked(source);
            assert_eq!(stats.functions, expected_functions, "{source}");
            assert_eq!(stats.unsupported_functions, 0, "{source}");
        }
    }

    #[test]
    fn template_deduplication_preserves_binding_and_structural_outer_context() {
        let source: Vec<u16> =
            "function factory(a){return function inner(){return a;};} function other(){}"
                .encode_utf16()
                .collect();
        let ast = parse_script(&source).expect("identity regression parses");
        let script = compile_script(&ast).expect("factory script compiles");
        let factory = FunctionTemplate {
            function: script.functions[0].1,
            binds_name: false,
            outer: Rc::default(),
        };
        let code = compile_function(&ast, &factory).expect("factory compiles");
        let captured = code
            .constants()
            .iter()
            .find_map(|constant| match constant {
                Constant::Function(template) => Some(template.clone()),
                _ => None,
            })
            .expect("factory exposes its captured function template");
        assert_ne!(captured.outer, Rc::default(), "a real captured context");

        let mut seen = Vec::new();
        assert!(remember_template(&mut seen, &captured));
        assert!(!remember_template(&mut seen, &captured.clone()));
        let mut equal_contents = captured.clone();
        equal_contents.outer = Rc::new((*captured.outer).clone());
        assert!(!Rc::ptr_eq(&equal_contents.outer, &captured.outer));
        assert!(!remember_template(&mut seen, &equal_contents));

        let mut different_outer = captured.clone();
        different_outer.outer = Rc::default();
        assert!(remember_template(&mut seen, &different_outer));
        let mut different_binding = captured.clone();
        different_binding.binds_name = !different_binding.binds_name;
        assert!(remember_template(&mut seen, &different_binding));
        let mut different_function = captured;
        different_function.function = factory.function;
        assert!(remember_template(&mut seen, &different_function));
        assert_eq!(seen.len(), 4);
    }
}
