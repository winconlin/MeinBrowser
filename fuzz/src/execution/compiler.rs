//! Arbitrary sources are compiled and inspected, never executed here.
//!
//! The checks do not depend on the interpreter accepting malformed code:
//! instruction coverage, operand encoding, bounds and branch targets are
//! checked directly against the compiler's output. Each source is compiled
//! as a script and, separately, as a module; modules are neither loaded
//! nor linked.

use std::collections::BTreeSet;
use std::rc::Rc;

use mb_js::ast::{Ast, Node};
use mb_js::bytecode::{Code, Constant, EvalSite, FunctionTemplate, Op, Operand};
use mb_js::compiler::{
    CompileError, EvalError, ModuleInit, compile_default_constructor, compile_eval,
    compile_function, compile_module, compile_script,
};
use mb_js::parser::{parse_eval, parse_module, parse_script};

/// The source is also compiled as the code of a direct `eval` at this many
/// of its call sites, besides once as indirect `eval` code.
const EVAL_SITES: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Stats {
    instructions: usize,
    forward_jumps: usize,
    loop_jumps: usize,
    wide: usize,
    extra_wide: usize,
    calls: usize,
    handlers: usize,
    functions: usize,
    unsupported_functions: usize,
    too_large_functions: usize,
    generators: usize,
    yields: usize,
    async_functions: usize,
    awaits: usize,
    async_iterators: usize,
    async_generators: usize,
    classes: usize,
    super_references: usize,
    strict_ranges: usize,
    direct_eval_calls: usize,
    dynamic_names: usize,
    eval_declarations: usize,
    evals: usize,
    eval_conflicts: usize,
    unsupported_evals: usize,
    import_loads: usize,
    import_metas: usize,
    import_calls: usize,
}

impl Stats {
    fn include(&mut self, other: Self) {
        self.instructions += other.instructions;
        self.forward_jumps += other.forward_jumps;
        self.loop_jumps += other.loop_jumps;
        self.wide += other.wide;
        self.extra_wide += other.extra_wide;
        self.calls += other.calls;
        self.handlers += other.handlers;
        self.generators += other.generators;
        self.yields += other.yields;
        self.async_functions += other.async_functions;
        self.awaits += other.awaits;
        self.async_iterators += other.async_iterators;
        self.async_generators += other.async_generators;
        self.classes += other.classes;
        self.super_references += other.super_references;
        self.strict_ranges += other.strict_ranges;
        self.direct_eval_calls += other.direct_eval_calls;
        self.dynamic_names += other.dynamic_names;
        self.eval_declarations += other.eval_declarations;
        self.import_loads += other.import_loads;
        self.import_metas += other.import_metas;
        self.import_calls += other.import_calls;
    }
}

/// What compiling the source as a module gave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ModuleStats {
    compiled: bool,
    bindings: usize,
    imports: usize,
    functions: usize,
    import_loads: usize,
    import_metas: usize,
    import_calls: usize,
    top_level_await: bool,
    awaits: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Syntax,
    Unsupported,
    TooLarge,
    Compiled(Box<Stats>),
}

/// Checks one compiled code. `top_level_await` marks the code of a module
/// with top-level `await`: it keeps an async state like an async function,
/// but ExecuteAsyncModule creates it, so the code does not begin with
/// `BeginAsync`.
fn check_code(code: &Code, source_length: usize, top_level_await: bool) -> Stats {
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
    if let Some(register) = code.new_target_register() {
        assert!(
            register < code.register_count(),
            "new.target register is in frame"
        );
    }
    if let Some(register) = code.generator_register() {
        assert!(
            register < code.register_count(),
            "generator register is in frame"
        );
    }
    if let Some(register) = code.async_state_register() {
        assert!(
            register < code.register_count(),
            "async state register is in frame"
        );
        assert_eq!(
            code.decode(0).map(|instruction| instruction.op) == Some(Op::BeginAsync),
            !top_level_await,
            "async functions begin with their capability, module code gets its state from outside"
        );
    }
    assert!(
        !top_level_await || code.async_state_register().is_some(),
        "a module with top-level await keeps its async state in a register"
    );
    if code.is_async_generator() {
        assert!(
            code.generator_register().is_some(),
            "async generator code keeps its generator in a register"
        );
        assert!(
            code.async_state_register().is_none(),
            "async generators settle the promises of their requests, not one of their own"
        );
    }
    let is_async = code.async_state_register().is_some() || code.is_async_generator();
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
        if matches!(
            instruction.op,
            Op::CreateGenerator
                | Op::Yield
                | Op::YieldResult
                | Op::ResumeGenerator
                | Op::YieldStar
                | Op::AsyncYield
                | Op::AsyncYieldStar
                | Op::AsyncYieldStarResult
        ) {
            assert!(
                code.generator_register().is_some(),
                "generator instructions only in generator code"
            );
        }
        if matches!(instruction.op, Op::Yield | Op::YieldResult | Op::YieldStar) {
            assert!(
                !code.is_async_generator(),
                "synchronous yields only in synchronous generators"
            );
        }
        if matches!(
            instruction.op,
            Op::AsyncYield | Op::AsyncYieldStar | Op::AsyncYieldStarResult
        ) {
            assert!(
                code.is_async_generator(),
                "async yields only in async generator code"
            );
        }
        if matches!(instruction.op, Op::TailCall | Op::TailCallSpread) {
            assert!(
                code.generator_register().is_none(),
                "no tail calls in generator bodies"
            );
            assert!(
                code.async_state_register().is_none(),
                "no tail calls in async bodies"
            );
        }
        if matches!(
            instruction.op,
            Op::Await
                | Op::ResumeAsync
                | Op::GetAsyncIterator
                | Op::AsyncIteratorNext
                | Op::AsyncIteratorClose
        ) {
            assert!(is_async, "async instructions only in async code");
        }
        if matches!(
            instruction.op,
            Op::BeginAsync | Op::ResolveAsync | Op::RejectAsync
        ) {
            assert!(
                code.async_state_register().is_some(),
                "only async functions settle a promise of their own"
            );
        }
        if instruction.op == Op::BeginAsync {
            assert_eq!(offset, 0, "only async code begins with BeginAsync");
        }
        assert_eq!(
            instruction.op == Op::ResumeAsync,
            last == Some(Op::Await),
            "every await resumes right after it"
        );
        if instruction.op == Op::Return && code.async_state_register().is_some() {
            assert!(
                matches!(last, Some(Op::ResolveAsync | Op::RejectAsync)),
                "async code settles its promise before returning"
            );
        }
        match instruction.op {
            Op::Call | Op::TailCall | Op::Construct | Op::SuperCall => {
                if instruction.op == Op::TailCall {
                    assert!(code.is_strict(), "tail calls only in strict code");
                }
                let callee = usize::try_from(instruction.operand(0)).expect("callee register");
                if instruction.op == Op::SuperCall {
                    assert!(callee >= 1, "new.target precedes the callee");
                }
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
            Op::CallSpread | Op::TailCallSpread | Op::ConstructSpread | Op::SuperCallSpread => {
                if instruction.op == Op::TailCallSpread {
                    assert!(code.is_strict(), "tail calls only in strict code");
                }
                let callee = usize::try_from(instruction.operand(0)).expect("callee register");
                if instruction.op == Op::SuperCallSpread {
                    assert!(callee >= 1, "new.target precedes the callee");
                }
                assert!(
                    callee
                        .checked_add(3)
                        .is_some_and(|end| end <= code.register_count()),
                    "callee, this and the argument array fit their frame"
                );
                stats.calls += 1;
            }
            Op::GetIterator
            | Op::IteratorNext
            | Op::IteratorElement
            | Op::IteratorSkip
            | Op::IteratorClose
            | Op::IteratorCloseOnThrow
            | Op::YieldStar => {
                // The record is the iterator, its next method and [[Done]].
                let operand =
                    usize::from(matches!(instruction.op, Op::IteratorNext | Op::YieldStar));
                let iterator =
                    usize::try_from(instruction.operand(operand)).expect("iterator register");
                assert!(
                    iterator
                        .checked_add(3)
                        .is_some_and(|end| end <= code.register_count()),
                    "the iterator record fits its frame"
                );
            }
            Op::GetAsyncIterator | Op::AsyncIteratorNext | Op::AsyncIteratorClose => {
                // The record of for await is the iterator and its next method.
                let operand = usize::from(instruction.op == Op::AsyncIteratorClose);
                let iterator =
                    usize::try_from(instruction.operand(operand)).expect("iterator register");
                assert!(
                    iterator
                        .checked_add(2)
                        .is_some_and(|end| end <= code.register_count()),
                    "the async iterator record fits its frame"
                );
                if instruction.op == Op::GetAsyncIterator {
                    stats.async_iterators += 1;
                }
            }
            Op::ForInPrepare | Op::ForInNext => {
                // The enumerator of a for-in loop is one register.
                let operand = usize::from(instruction.op == Op::ForInNext);
                let enumerator =
                    usize::try_from(instruction.operand(operand)).expect("enumerator register");
                assert!(
                    enumerator < code.register_count(),
                    "the enumerator fits its frame"
                );
            }
            Op::AsyncYieldStar | Op::AsyncYieldStarResult => {
                // The record of an async yield* is the iterator, its next
                // method and the kind of the current step.
                let operand = usize::from(instruction.op == Op::AsyncYieldStarResult);
                let iterator =
                    usize::try_from(instruction.operand(operand)).expect("iterator register");
                assert!(
                    iterator
                        .checked_add(3)
                        .is_some_and(|end| end <= code.register_count()),
                    "the async yield* record fits its frame"
                );
            }
            Op::CreateRestObject => {
                // The source value, then the keys the rest excludes.
                let source = usize::try_from(instruction.operand(0)).expect("source register");
                let keys = usize::try_from(instruction.operand(1)).expect("nonnegative key count");
                assert!(
                    source
                        .checked_add(1)
                        .and_then(|first| first.checked_add(keys))
                        .is_some_and(|end| end <= code.register_count()),
                    "the value and its excluded keys fit their frame"
                );
            }
            Op::LdaSuperNamed | Op::StaSuperNamed | Op::LdaSuperKeyed | Op::StaSuperKeyed => {
                // The base, `this` and, for computed names, the key.
                let base = usize::try_from(instruction.operand(0)).expect("base register");
                let length = if matches!(instruction.op, Op::LdaSuperKeyed | Op::StaSuperKeyed) {
                    3
                } else {
                    2
                };
                assert!(
                    base.checked_add(length)
                        .is_some_and(|end| end <= code.register_count()),
                    "the super reference fits its frame"
                );
                stats.super_references += 1;
            }
            Op::CreateClass => {
                // The superclass or the hole, then the class name.
                let parent = usize::try_from(instruction.operand(1)).expect("parent register");
                assert!(
                    parent
                        .checked_add(2)
                        .is_some_and(|end| end <= code.register_count()),
                    "the superclass and the name fit their frame"
                );
                match code.constant(instruction.operand(0)) {
                    Some(Constant::Function(_)) => {}
                    Some(Constant::DefaultConstructor { derived, .. }) => {
                        let default = compile_default_constructor(*derived)
                            .expect("the default constructor compiles");
                        assert_eq!(default.is_derived_constructor(), *derived);
                        assert!(default.is_strict(), "class code is strict");
                        stats.include(check_code(&default, source_length, false));
                    }
                    constant => panic!("a class needs a constructor, got {constant:?}"),
                }
                assert!(
                    code.is_strict_at(instruction.start),
                    "class code runs strict, in sloppy code within a strict range"
                );
                stats.classes += 1;
            }
            Op::NextField => {
                // The constructor, the index, the initializer, the object
                // and the field name.
                let first = usize::try_from(instruction.operand(1)).expect("field loop register");
                assert!(
                    first
                        .checked_add(5)
                        .is_some_and(|end| end <= code.register_count()),
                    "the field loop fits its frame"
                );
            }
            Op::CreatePrivateName => {
                assert!(
                    matches!(
                        code.constant(instruction.operand(0)),
                        Some(Constant::String(_))
                    ),
                    "a private name has a string description"
                );
            }
            Op::CreateGenerator => {
                stats.generators += 1;
                if code.is_async_generator() {
                    stats.async_generators += 1;
                }
            }
            Op::Yield | Op::YieldResult | Op::AsyncYield => stats.yields += 1,
            Op::BeginAsync => stats.async_functions += 1,
            Op::Await => stats.awaits += 1,
            Op::CallEval => {
                // The callee, `this`, then the arguments or their array.
                let callee = usize::try_from(instruction.operand(0)).expect("callee register");
                let Some(Constant::EvalSite(site)) = code.constant(instruction.operand(1)) else {
                    panic!("a possible direct eval names its site");
                };
                let arguments = if site.is_spread() {
                    1
                } else {
                    usize::try_from(site.arguments()).expect("argument count")
                };
                assert!(
                    callee
                        .checked_add(2)
                        .and_then(|first| first.checked_add(arguments))
                        .is_some_and(|end| end <= code.register_count()),
                    "the eval call fits its frame"
                );
                assert_eq!(
                    site.options().strict,
                    code.is_strict_at(instruction.start),
                    "the site is as strict as its call"
                );
                stats.direct_eval_calls += 1;
            }
            Op::ResolveDynamic | Op::LdaDynamic | Op::StaDynamic | Op::WithBaseObject => {
                assert!(matches!(
                    code.constant(instruction.operand(0)),
                    Some(Constant::Dynamic(_))
                ));
                let register =
                    usize::try_from(instruction.operand(1)).expect("resolution register");
                assert!(
                    register < code.register_count(),
                    "the resolution is in frame"
                );
                stats.dynamic_names += 1;
            }
            Op::TypeofDynamic | Op::DeleteDynamic => {
                assert!(matches!(
                    code.constant(instruction.operand(0)),
                    Some(Constant::Dynamic(_))
                ));
                stats.dynamic_names += 1;
            }
            Op::EvalInstantiate => {
                let Some(Constant::EvalInstantiation(names)) =
                    code.constant(instruction.operand(0))
                else {
                    panic!("eval instantiation names its declarations");
                };
                for register in names.hoisted_registers() {
                    assert!(
                        usize::try_from(register)
                            .is_ok_and(|register| register < code.register_count()),
                        "the flags of hoisted functions are in frame"
                    );
                }
                assert_eq!(instruction.start, 0, "eval instantiation comes first");
                stats.eval_declarations += 1;
            }
            Op::DeclareFunction | Op::DeclareVar => {
                assert!(matches!(
                    code.constant(instruction.operand(0)),
                    Some(Constant::EvalBinding(_))
                ));
                stats.eval_declarations += 1;
            }
            Op::LdaImport => {
                assert!(
                    matches!(
                        code.constant(instruction.operand(0)),
                        Some(Constant::Name(_))
                    ),
                    "an import binding names itself for its ReferenceError"
                );
                assert!(code.is_strict(), "import bindings only in module code");
                stats.import_loads += 1;
            }
            Op::ImportMeta => {
                assert!(
                    matches!(
                        code.constant(instruction.operand(0)),
                        Some(Constant::Module)
                    ),
                    "import.meta asks for the module of its code"
                );
                assert!(code.is_strict(), "import.meta only in module code");
                stats.import_metas += 1;
            }
            Op::ImportCall => {
                assert!(
                    matches!(
                        code.constant(instruction.operand(0)),
                        Some(Constant::Module)
                    ),
                    "import() names the module of its code, or none, as referrer"
                );
                assert!(
                    usize::try_from(instruction.operand(1))
                        .is_ok_and(|register| register < code.register_count()),
                    "the specifier waits in a register of the frame"
                );
                stats.import_calls += 1;
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
    let boundary = |offset: usize| starts.contains(&offset) || offset == bytes.len();
    for handler in code.handlers() {
        assert!(handler.start <= handler.end, "handler range is ordered");
        assert!(
            boundary(handler.start) && boundary(handler.end),
            "handler range starts and ends at instruction boundaries"
        );
        assert!(
            starts.contains(&handler.target),
            "handler targets an instruction boundary"
        );
        stats.handlers += 1;
    }
    let mut previous_end = 0;
    let mut ranges = Vec::new();
    for (start, end) in code.strict_ranges() {
        assert!(!code.is_strict(), "only sloppy code has strict ranges");
        assert!(
            previous_end <= start && start < end,
            "strict ranges are ascending, disjoint and not empty"
        );
        assert!(
            boundary(start) && boundary(end),
            "strict ranges start and end at instruction boundaries"
        );
        previous_end = end;
        ranges.push((start, end));
        stats.strict_ranges += 1;
    }
    for &start in &starts {
        let inside = ranges.iter().any(|&(from, to)| from <= start && start < to);
        assert_eq!(
            code.is_strict_at(start),
            code.is_strict() || inside,
            "strictness at an instruction follows the strict ranges"
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
    sites: &mut Vec<Rc<EvalSite>>,
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
                stats.include(check_code(&code, source_length, false));
                stats.functions += 1;
                eval_sites(&code, sites);
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
            let mut stats = check_code(&script.code, source.len(), false);
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
            let mut sites = Vec::new();
            eval_sites(&script.code, &mut sites);
            check_functions(&ast, source.len(), pending, &mut stats, &mut sites);
            check_evals(source, &sites, &mut stats);
            Outcome::Compiled(Box::new(stats))
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

/// Compiles `source` as a module: its code is strict, each top-level
/// binding has its own slot in the module context, and the declared
/// functions compile with that context around them. With top-level
/// `await`, the module code keeps an async state without `BeginAsync`.
/// Direct `eval` sites in modules are not followed.
fn check_module(source: &[u16]) -> Option<ModuleStats> {
    let ast = match parse_module(source) {
        Ok(ast) => ast,
        Err(error) => {
            assert!(
                error.position <= source.len(),
                "module syntax error position"
            );
            return None;
        }
    };
    assert!(ast.is_module() && ast.is_strict(), "module code is strict");
    let module = match compile_module(&ast) {
        Ok(module) => module,
        Err(error) => {
            check_error(error, source.len());
            return Some(ModuleStats::default());
        }
    };
    assert_eq!(
        module.code.parameter_count(),
        0,
        "modules have no parameters"
    );
    assert!(module.code.is_strict(), "module code is strict");
    let mut slots = BTreeSet::new();
    let mut result = ModuleStats {
        compiled: true,
        bindings: module.bindings.len(),
        ..ModuleStats::default()
    };
    let mut pending = Vec::new();
    for binding in &module.bindings {
        // Slot 0 links to the outer context, the last one holds `this`.
        assert!(
            binding.slot >= 1 && binding.slot < module.slots,
            "a binding has a slot of the module context"
        );
        assert!(slots.insert(binding.slot), "each binding has its own slot");
        assert!(!binding.name.is_empty(), "a binding has a name");
        match &binding.init {
            ModuleInit::Import => result.imports += 1,
            ModuleInit::Function(template) => pending.push(template.clone()),
            ModuleInit::Var | ModuleInit::Lexical => {}
        }
    }
    let mut stats = check_code(&module.code, source.len(), ast.has_top_level_await());
    pending.extend(
        module
            .code
            .constants()
            .iter()
            .filter_map(|constant| match constant {
                Constant::Function(template) => Some(template.clone()),
                _ => None,
            }),
    );
    check_functions(&ast, source.len(), pending, &mut stats, &mut Vec::new());
    result.functions = stats.functions;
    result.import_loads = stats.import_loads;
    result.import_metas = stats.import_metas;
    result.import_calls = stats.import_calls;
    result.top_level_await = ast.has_top_level_await();
    result.awaits = stats.awaits;
    Some(result)
}

/// Collects the sites of possible direct `eval` calls in `code`.
fn eval_sites(code: &Code, sites: &mut Vec<Rc<EvalSite>>) {
    sites.extend(
        code.constants()
            .iter()
            .filter_map(|constant| match constant {
                Constant::EvalSite(site) => Some(Rc::clone(site)),
                _ => None,
            }),
    );
}

/// Compiles `source` as `eval` code: once indirectly, with the global
/// environment, and directly at the first [`EVAL_SITES`] of `sites`, where
/// names resolve through the enclosing contexts of the call. A conflict
/// with a lexical binding is a SyntaxError of the call, not a failure.
/// Nested functions are compiled, but sites inside `eval` code are not
/// followed, so the work stays bounded.
fn check_evals(source: &[u16], sites: &[Rc<EvalSite>], stats: &mut Stats) {
    let direct = sites.iter().take(EVAL_SITES).map(|site| Some(&**site));
    for site in std::iter::once(None).chain(direct) {
        let options = site.map(EvalSite::options).unwrap_or_default();
        let ast = match parse_eval(source, &options) {
            Ok(ast) => ast,
            Err(error) => {
                assert!(error.position <= source.len(), "eval syntax error position");
                continue;
            }
        };
        assert!(ast.is_dynamic(), "eval code has no position in the script");
        assert!(
            !options.strict || ast.is_strict(),
            "eval code in strict code is strict"
        );
        match compile_eval(&ast, site) {
            Ok(code) => {
                assert_eq!(code.parameter_count(), 0, "eval code has no parameters");
                let mut inner = check_code(&code, source.len(), false);
                let pending = code
                    .constants()
                    .iter()
                    .filter_map(|constant| match constant {
                        Constant::Function(template) => Some(template.clone()),
                        _ => None,
                    })
                    .collect();
                check_functions(&ast, source.len(), pending, &mut inner, &mut Vec::new());
                stats.evals += 1;
                stats.dynamic_names += inner.dynamic_names;
                stats.eval_declarations += inner.eval_declarations;
            }
            Err(EvalError::Conflict) => {
                assert!(
                    site.is_some() && !ast.is_strict(),
                    "only sloppy direct eval code meets the bindings at its call"
                );
                stats.eval_conflicts += 1;
            }
            Err(EvalError::Compile(error)) => {
                if check_error(error, source.len()) {
                    stats.unsupported_evals += 1;
                }
            }
        }
    }
}

pub(super) fn exercise(data: &[u8]) {
    let utf8: Vec<u16> = String::from_utf8_lossy(data).encode_utf16().collect();
    check_source(&utf8);
    check_module(&utf8);
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
            Outcome::Compiled(stats) => *stats,
            outcome => panic!("expected supported source, got {outcome:?}"),
        }
    }

    #[test]
    fn eval_sites_compile_the_source_as_eval_code() {
        let stats = checked("var x = 1; eval('x');");
        assert_eq!(stats.direct_eval_calls, 1);
        assert_eq!(stats.evals, 2, "indirect and at the one site");
        assert!(
            stats.eval_declarations >= 2,
            "the global var of the eval code"
        );
        let stats = checked("function f(a) { eval(a); return b; } f('var b');");
        assert_eq!(stats.direct_eval_calls, 1);
        assert!(
            stats.dynamic_names > 0,
            "names past the var scope are dynamic"
        );
        // At the call, the eval code's `var f` meets the `let f` of the
        // block around it.
        let stats = checked("{ let f; eval(''); } var f;");
        assert_eq!(stats.eval_conflicts, 1);
        assert_eq!(stats.evals, 1, "only the indirect eval compiles");
        let stats = checked("'use strict'; var y; eval(y); let z;");
        assert_eq!(
            stats.eval_conflicts, 0,
            "strict eval keeps its declarations"
        );
        assert_eq!(stats.evals, 2);
        let stats = checked("class C { #p; m() { return eval('this.#p'); } }");
        assert_eq!(stats.direct_eval_calls, 1);
        assert_eq!(stats.evals, 2, "the site passes the private name");
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

    fn checked_module(source: &str) -> ModuleStats {
        let source: Vec<u16> = source.encode_utf16().collect();
        check_module(&source).expect("module parses")
    }

    #[test]
    fn modules_compile_with_their_bindings_in_context_slots() {
        let stats = checked_module(
            "import d, { a as b } from './m.js'; import * as ns from './n.js' with { type: 'json' }; \
             export var v = b; export let l = ns; export function f() { return d + import.meta.x; } \
             export default class { m() { return b; } } export * from './o.js'; \
             export { b as c }; d; (() => b)();",
        );
        assert!(stats.compiled);
        assert_eq!(stats.imports, 3);
        assert_eq!(stats.bindings, 7, "three imports, v, l, f and the default");
        assert!(stats.functions >= 3, "f, the method and the arrow");
        assert!(stats.import_loads >= 4, "b, ns, d and the captured b");
        assert_eq!(stats.import_metas, 1);
        let stats = checked_module("export default 1 + 2; import.meta;");
        assert!(stats.compiled);
        assert_eq!(stats.bindings, 1, "the default value");
        assert_eq!(stats.import_metas, 1);
        let stats = checked_module("export default function () {}");
        assert_eq!(
            (stats.bindings, stats.functions),
            (1, 1),
            "the default function"
        );
        let stats = checked_module("await 1; for await (const x of []); import('./m.js');");
        assert!(stats.compiled && stats.top_level_await);
        assert!(stats.awaits >= 2, "await and the steps of for await");
        assert_eq!(stats.import_calls, 1);
        let stats = checked_module("async function f() { await 1; } import.meta;");
        assert!(
            stats.compiled && !stats.top_level_await,
            "await inside a function"
        );
        let source: Vec<u16> = "export default 1; export default 2;"
            .encode_utf16()
            .collect();
        assert_eq!(
            check_module(&source),
            None,
            "a duplicate export is an early error"
        );
        // Assigning an import throws when it runs.
        let stats = checked_module("import x from 'm'; x = 1;");
        assert!(stats.compiled);
        assert_eq!((stats.imports, stats.import_loads), (1, 0));
        let source: Vec<u16> = "import.meta;".encode_utf16().collect();
        assert_eq!(
            check_source(&source),
            Outcome::Syntax,
            "scripts have no import.meta"
        );
        let stats = checked("import('./m.js', { with: {} }); function f() { return import(f); }");
        assert_eq!(stats.import_calls, 2, "import() in scripts and functions");
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
        for source in ["class A{[f`x`](){}}", "x={a:f`x`};", "f`x`;"] {
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
        for source in ["function f(){x=[f`x`];}", "function f(){f`x`;}"] {
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
    fn with_bodies_resolve_names_dynamically() {
        let stats = checked(
            "var o={x:1,f(){}};with(o){x++;f();typeof y;delete x;var v=x;} \
             function g(a){with(o){return function(){return a+x;};}}",
        );
        assert_eq!(stats.functions, 3);
        assert_eq!(stats.unsupported_functions, 0);
        assert!(
            stats.dynamic_names >= 8,
            "names in the body resolve dynamically"
        );
    }

    #[test]
    fn for_in_keeps_its_enumerator_in_frame() {
        let stats = checked(
            "for (var k in o) { if (k) continue; } for (let [a] in b) break; \
             function f(){ for (const k in o) return k; } for (var i = 0 in c);",
        );
        assert_eq!(stats.functions, 1);
        assert_eq!(stats.unsupported_functions, 0);
        assert!(stats.loop_jumps >= 3, "each for-in loops back");
    }

    #[test]
    fn iteration_and_spread_keep_their_records_in_frame() {
        let stats = checked(
            "for (let x of [1, ...'ab']) { if (x) break; f(...[x], x); } \
             outer: for (var y of o) for (z of p) continue outer; new F(...q);",
        );
        assert_eq!(stats.calls, 2, "a spread call and a spread construct");
        assert_eq!(
            stats.handlers, 3,
            "each for-of closes its iterator on a throw"
        );
        let stats = checked("'use strict'; function f(a) { return f(...a); }");
        assert_eq!(stats.functions, 1);
        assert_eq!(stats.calls, 1, "the tail call with spread");
    }

    #[test]
    fn destructuring_keeps_records_and_rest_keys_in_frame() {
        let stats = checked(
            "var [a, , ...b] = c; let {d, [e]: f = 1, ...g} = h; [i.j, k = 2] = l; \
             ({m, ...n.o} = p); for (const [q, {r}] of s); try {} catch ([t]) {} \
             function u([v] = [], {w, ...x}, ...[y]) { return v + w + x + y; }",
        );
        assert_eq!(stats.functions, 1);
        assert_eq!(stats.unsupported_functions, 0);
        assert_eq!(
            stats.handlers, 8,
            "each non-empty array pattern, the for-of and the catch"
        );
    }

    #[test]
    fn generators_keep_their_register_and_records_in_frame() {
        let stats = checked(
            "function* g(a) { var x = yield a; try { yield* x; } finally { yield; } return x; } \
             var o = { *m() { yield 1; } }; { function* h() {} }",
        );
        assert_eq!(stats.functions, 3);
        assert_eq!(stats.unsupported_functions, 0);
        assert_eq!(stats.generators, 3, "each generator creates its object");
        assert_eq!(stats.yields, 4, "three yields and the one of yield*");
        let stats = checked("'use strict'; function* g() { return g(); }");
        assert_eq!(stats.functions, 1);
        assert_eq!(stats.calls, 1, "a plain call, never a tail call");
    }

    #[test]
    fn async_functions_keep_their_state_in_frame() {
        let stats = checked(
            "async function f(a = 0) { var x = await a; try { await x; } finally { await 1; } \
             return x; } var o = { async m() { return await 2; } }; var g = async () => await 3; \
             { async function h() {} }",
        );
        assert_eq!(stats.functions, 4);
        assert_eq!(stats.unsupported_functions, 0);
        assert_eq!(
            stats.async_functions, 4,
            "each async function begins with its capability"
        );
        assert_eq!(stats.awaits, 5);
        let stats = checked("'use strict'; async function f() { return f(); }");
        assert_eq!(stats.functions, 1);
        assert_eq!(stats.calls, 1, "a plain call, never a tail call");
    }

    #[test]
    fn async_generators_keep_their_register_and_records_in_frame() {
        let stats = checked(
            "async function* g(a) { var x = yield a; try { yield* x; } finally { await x; } \
             for await (var y of x) yield y; return x; } \
             var o = { async *m() { yield 1; } }; { async function* h() {} }",
        );
        assert_eq!(stats.functions, 3);
        assert_eq!(stats.unsupported_functions, 0);
        assert_eq!(
            stats.generators, 3,
            "each async generator creates its object"
        );
        assert_eq!(stats.async_generators, 3);
        assert_eq!(stats.async_functions, 0, "no capability of their own");
        assert_eq!(stats.yields, 4, "three yields and the one of yield*");
        assert_eq!(stats.async_iterators, 2, "yield* and for await");
        assert!(stats.awaits >= 7, "yields, returns and awaits all await");
        let stats = checked("'use strict'; async function* g() { return g(); }");
        assert_eq!(stats.functions, 1);
        assert_eq!(stats.calls, 1, "a plain call, never a tail call");
    }

    #[test]
    fn for_await_keeps_its_record_in_frame() {
        let stats = checked(
            "async function f(o) { for await (const x of o) { if (x) break; return x; } \
             outer: for await (var y of o) for await (z of o) continue outer; }",
        );
        assert_eq!(stats.functions, 1);
        assert_eq!(stats.unsupported_functions, 0);
        assert_eq!(stats.async_iterators, 3);
        assert_eq!(
            stats.handlers, 7,
            "the rejection, and per loop the throw and its closing"
        );
        assert_eq!(
            stats.awaits, 9,
            "each next, each close on a throw, a break, a return and a continue to the outer loop"
        );
    }

    #[test]
    fn classes_and_super_keep_their_registers_in_frame() {
        let stats = checked(
            "class A { constructor(a) { this.a = a; } m() { return 1; } static s() {} \
             get g() { return 1; } set g(v) {} [Symbol.iterator]() {} } \
             class B extends A { constructor(...r) { super(...r); super.m(); super['m'] = 1; } \
             m() { return () => super.m() + 1; } } \
             let C = class extends B {}, D = class {}; \
             var o = { m() { return super.x; }, get n() { delete super.y; } };",
        );
        assert_eq!(stats.classes, 4);
        assert_eq!(stats.unsupported_functions, 0);
        assert!(
            stats.functions >= 9,
            "constructors, methods, accessors and the arrow"
        );
        assert_eq!(
            stats.super_references, 4,
            "delete only evaluates its reference"
        );
        assert!(
            stats.calls >= 3,
            "super(), the default super() and the calls"
        );
    }

    #[test]
    fn fields_private_names_and_static_blocks_keep_their_registers_in_frame() {
        let stats = checked(
            "class A { x = 1; #y = () => this.#y; static z = this; static #w; ['v'] = 2; \
             #m() { return #y in this; } get #g() { return this.#m(); } set #g(v) {} \
             static { this.#w = A.z; } constructor() { this.#g = this.#y?.(); } } \
             class B extends A { u = super.x; constructor() { (() => super())(); } } \
             let C = class extends B { #t = 1; };",
        );
        assert_eq!(stats.classes, 3);
        assert_eq!(stats.unsupported_functions, 0);
        assert!(
            stats.functions >= 12,
            "constructors, methods, accessors, initializers, the block and the arrows"
        );
        assert!(
            stats.calls >= 6,
            "each constructor calls the field initializers, the statics run once"
        );
    }

    #[test]
    fn classes_in_sloppy_code_get_strict_ranges() {
        let stats = checked(
            "x = 1; class A extends (y = Object) { [z = 'm']() {} [w = 'f'] = 1; } \
             var B = class { static [(class extends Object {}, 'n')] = 2; }; \
             function f() { return class {}; } v = 2;",
        );
        assert_eq!(stats.classes, 4);
        assert_eq!(
            stats.strict_ranges, 3,
            "one for each outermost class in sloppy code, the inner class shares it"
        );
        let stats = checked("'use strict'; class A extends Object { [1]() {} }");
        assert_eq!(stats.classes, 1);
        assert_eq!(stats.strict_ranges, 0, "strict code has no ranges");
    }

    #[test]
    fn try_statements_have_checked_handlers() {
        let stats = checked(
            "try { throw 1; } catch (e) { e; } finally { 2; } \
             function f(){ for(;;) { try { return 1; } finally { break; } } }",
        );
        assert_eq!(stats.functions, 1);
        assert_eq!(stats.unsupported_functions, 0);
        assert_eq!(
            stats.handlers, 3,
            "catch and finally, then the function's finally"
        );
    }

    #[test]
    fn new_new_target_and_tail_calls_are_checked() {
        let stats = checked(
            "'use strict'; function f(n){ return n ? f(n - 1) : new.target; } \
             new f(1); let g = () => f(0);",
        );
        assert_eq!(stats.functions, 2);
        assert_eq!(stats.unsupported_functions, 0);
        assert_eq!(stats.calls, 3, "new, then a tail call in each function");
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
