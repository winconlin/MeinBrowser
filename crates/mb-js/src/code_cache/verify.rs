//! Die Prüfung eines geladenen Scripts.
//!
//! Der Interpreter liest Register, Konstanten und Sprungziele so, wie der
//! Übersetzer sie anlegt. Für Bytecode aus einem Speicher prüft diese Datei
//! vorher alle Regeln, die der Übersetzer einhält. Sie sind dieselben, die
//! das Fuzz-Ziel `compiler` für frisch übersetzten Code verlangt
//! (`fuzz/src/execution/compiler.rs`), dazu die Angaben neben dem Code:
//!
//! - Jeder Befehl ist lesbar, der letzte ist `Return`, und die Befehle
//!   decken den Bytecode genau ab.
//! - Register liegen im Rahmen, auch die Folgen von Registern, die
//!   Aufrufe, Iteratoren und Klassen lesen; Konstanten liegen im Pool und
//!   sind von der Art, die der Befehl liest.
//! - Sprünge, Handler und strikte Bereiche beginnen und enden an
//!   Befehlsgrenzen, Lagen im Quelltext liegen in ihm.
//! - Befehle für Generatoren und `async` stehen nur in solchem Code, wie
//!   ihn die Angaben zur Funktion verlangen. Befehle, die nur im Code von
//!   Modulen oder `eval` vorkommen, gibt es nicht.
//! - Jede Funktion und Klasse, die der Code nennt, steht im Eintrag, und
//!   ihr Quelltext liegt im Quelltext des Scripts. Was eine Funktion vom
//!   umgebenden Code sieht, hat höchstens so viele Stufen, wie der
//!   Übersetzer anlegen kann.

use super::{CompiledScript, MAX_LEVELS, Rejected};
use crate::ast::FunctionKind;
use crate::bytecode::{Code, Constant, Instruction, Op, Operand};
use crate::compiler::CompileError;
use crate::unit::Shape;

fn reject<T>(reason: &'static str) -> Result<T, Rejected> {
    Err(Rejected::new(reason))
}

fn check(condition: bool, reason: &'static str) -> Result<(), Rejected> {
    if condition { Ok(()) } else { reject(reason) }
}

/// Wozu ein Code gehört.
#[derive(Clone, Copy)]
enum Owner {
    Script,
    Function(Shape),
}

pub(super) fn verify(script: &CompiledScript) -> Result<(), Rejected> {
    let source = script.source.len();
    let in_source =
        |start: u32, end: u32| start <= end && usize::try_from(end).is_ok_and(|end| end <= source);
    let mut previous = None;
    for (id, function) in &script.functions {
        check(
            previous.is_none_or(|previous| previous < id.to_raw()),
            "Funktionen nicht aufsteigend",
        )?;
        previous = Some(id.to_raw());
        let (start, end) = function.text;
        check(in_source(start, end), "Quelltext einer Funktion außerhalb")?;
        match &function.code {
            Ok(code) => verify_code(script, code, Owner::Function(function.shape))?,
            Err(CompileError::Unsupported(position)) => {
                check(*position <= source, "Lage eines Fehlers außerhalb")?;
            }
            Err(CompileError::TooLarge) => {}
        }
    }
    let mut previous = None;
    for &(id, start, end) in &script.classes {
        check(
            previous.is_none_or(|previous| previous < id.to_raw()),
            "Klassen nicht aufsteigend",
        )?;
        previous = Some(id.to_raw());
        check(in_source(start, end), "Quelltext einer Klasse außerhalb")?;
    }
    let names = &script.script;
    verify_code(script, &names.code, Owner::Script)?;
    for &(_, function) in &names.functions {
        let shape = script.function(function).map(|function| function.shape);
        check(
            shape.is_some_and(|shape| shape.kind == FunctionKind::Normal),
            "Funktionsdeklaration fehlt",
        )?;
    }
    for &(_, register) in &names.hoisted_functions {
        check(
            usize::try_from(register).is_ok_and(|register| register < names.code.register_count()),
            "Register einer Funktion in einem Block außerhalb",
        )?;
    }
    Ok(())
}

/// Ob die `length` Register ab `first` im Rahmen liegen.
fn spans(code: &Code, first: i64, length: usize) -> bool {
    usize::try_from(first)
        .ok()
        .and_then(|first| first.checked_add(length))
        .is_some_and(|end| end <= code.register_count())
}

fn verify_code(script: &CompiledScript, code: &Code, owner: Owner) -> Result<(), Rejected> {
    let length = code.bytes().len();
    let registers = code.register_count();
    check(registers >= 2, "zu wenige Register")?;
    check(
        code.parameter_count()
            .checked_add(2)
            .is_some_and(|end| end <= registers),
        "Parameter außerhalb des Rahmens",
    )?;
    for register in [
        code.arguments_register(),
        code.new_target_register(),
        code.generator_register(),
        code.async_state_register(),
    ]
    .into_iter()
    .flatten()
    {
        check(register < registers, "besonderes Register außerhalb")?;
    }
    let generator = code.generator_register().is_some();
    let async_state = code.async_state_register().is_some();
    let async_generator = code.is_async_generator();
    check(
        !async_generator || (generator && !async_state),
        "async-Generator ohne Generator",
    )?;
    match owner {
        Owner::Script => check(
            code.parameter_count() == 0
                && code.arguments_register().is_none()
                && code.new_target_register().is_none()
                && !generator
                && !async_state
                && !code.is_derived_constructor()
                && code.parameter_map().is_empty(),
            "Script mit Angaben einer Funktion",
        )?,
        Owner::Function(shape) => check(
            generator == shape.generator
                && async_generator == (shape.generator && shape.is_async)
                && async_state == (shape.is_async && !shape.generator)
                && code.is_derived_constructor()
                    == (shape.kind == FunctionKind::DerivedConstructor),
            "Code passt nicht zur Art der Funktion",
        )?,
    }
    let is_async = async_state || async_generator;
    // Jede Konstante wird beim Anlegen des Pools ein Wert, auch eine, die
    // kein Befehl liest.
    for constant in code.constants() {
        let known = match constant {
            Constant::Function(template) => {
                check(
                    template.outer.depth() <= MAX_LEVELS,
                    "Stufen zu tief verschachtelt",
                )?;
                script.function(template.function).is_some()
            }
            Constant::DefaultConstructor { class, .. } => script.class_text(*class).is_some(),
            Constant::EvalInstantiation(_) | Constant::EvalBinding(_) => false,
            _ => true,
        };
        check(known, "Konstante ohne Gegenstück im Eintrag")?;
    }

    // Die Befehle.
    let mut starts = vec![false; length];
    let mut jumps = Vec::new();
    let mut offset = 0;
    let mut last = None;
    while offset < length {
        let Some(instruction) = code.decode(offset) else {
            return reject("Befehl nicht lesbar");
        };
        check(
            instruction.end > offset && instruction.end <= length,
            "Befehl über das Ende",
        )?;
        if let Some(start) = starts.get_mut(offset) {
            *start = true;
        }
        let op = instruction.op;
        for (index, &kind) in op.operands().iter().enumerate() {
            let value = instruction.operand(index);
            match kind {
                Operand::Register => check(spans(code, value, 1), "Register außerhalb")?,
                Operand::Constant => {
                    check(code.constant(value).is_some(), "Konstante nicht im Pool")?
                }
                Operand::Jump => {
                    let target = i64::try_from(instruction.start)
                        .ok()
                        .and_then(|start| start.checked_add(value))
                        .and_then(|target| usize::try_from(target).ok())
                        .filter(|&target| target < length);
                    let Some(target) = target else {
                        return reject("Sprung aus dem Code");
                    };
                    check(
                        (op == Op::JumpLoop) == (value <= 0) || value == 0,
                        "Sprung in die falsche Richtung",
                    )?;
                    jumps.push(target);
                }
                Operand::Immediate => {}
            }
        }
        check(
            constants_fit(script, code, &instruction),
            "Konstante falscher Art",
        )?;
        let first = instruction.operand(0);
        let second = instruction.operand(1);
        let count = usize::try_from(second).unwrap_or(usize::MAX);
        let fits = match op {
            Op::Call | Op::TailCall | Op::Construct => spans(code, first, count.saturating_add(2)),
            Op::SuperCall => first >= 1 && spans(code, first, count.saturating_add(2)),
            Op::CallSpread | Op::TailCallSpread | Op::ConstructSpread => spans(code, first, 3),
            Op::SuperCallSpread => first >= 1 && spans(code, first, 3),
            Op::CallEval => match code.constant(second) {
                Some(Constant::EvalSite(site)) => {
                    let arguments = if site.spread {
                        1
                    } else {
                        usize::try_from(site.arguments).unwrap_or(usize::MAX)
                    };
                    spans(code, first, arguments.saturating_add(2))
                        && site.options().strict == code.is_strict_at(instruction.start)
                }
                _ => false,
            },
            Op::GetIterator
            | Op::IteratorElement
            | Op::IteratorSkip
            | Op::IteratorClose
            | Op::IteratorCloseOnThrow => spans(code, first, 3),
            Op::IteratorNext | Op::YieldStar => spans(code, second, 3),
            Op::GetAsyncIterator | Op::AsyncIteratorNext => spans(code, first, 2),
            Op::AsyncIteratorClose => spans(code, second, 2),
            Op::AsyncYieldStar => spans(code, first, 3),
            Op::AsyncYieldStarResult => spans(code, second, 3),
            Op::CreateRestObject => spans(code, first, count.saturating_add(1)),
            Op::LdaSuperNamed | Op::StaSuperNamed => spans(code, first, 2),
            Op::LdaSuperKeyed | Op::StaSuperKeyed => spans(code, first, 3),
            Op::CreateClass => spans(code, second, 2) && code.is_strict_at(instruction.start),
            Op::NextField => spans(code, second, 5),
            _ => true,
        };
        check(fits, "Register eines Befehls außerhalb")?;

        // Generatoren, async und was nur in Modulen oder eval vorkommt.
        let allowed = match op {
            Op::CreateGenerator | Op::ResumeGenerator => generator,
            Op::Yield | Op::YieldResult | Op::YieldStar => generator && !async_generator,
            Op::AsyncYield | Op::AsyncYieldStar | Op::AsyncYieldStarResult => async_generator,
            Op::TailCall | Op::TailCallSpread => {
                !generator && !async_state && code.is_strict_at(instruction.start)
            }
            Op::Await | Op::GetAsyncIterator | Op::AsyncIteratorNext | Op::AsyncIteratorClose => {
                is_async
            }
            Op::ResumeAsync => is_async && last == Some(Op::Await),
            Op::BeginAsync => async_state && offset == 0,
            Op::ResolveAsync | Op::RejectAsync => async_state,
            Op::Return => !async_state || matches!(last, Some(Op::ResolveAsync | Op::RejectAsync)),
            Op::EvalInstantiate
            | Op::DeclareFunction
            | Op::DeclareVar
            | Op::LdaImport
            | Op::ImportMeta => false,
            _ => true,
        };
        check(allowed, "Befehl gehört nicht in diesen Code")?;
        check(
            op == Op::ResumeAsync || last != Some(Op::Await),
            "await ohne Fortsetzung",
        )?;
        last = Some(op);
        offset = instruction.end;
    }
    check(last == Some(Op::Return), "Code endet nicht mit Return")?;
    check(
        !async_state
            || code
                .decode(0)
                .is_some_and(|first| first.op == Op::BeginAsync),
        "async-Funktion ohne BeginAsync",
    )?;
    let start = |offset: usize| starts.get(offset).copied().unwrap_or(false);
    let boundary = |offset: usize| offset == length || start(offset);
    for target in jumps {
        check(start(target), "Sprung nicht auf einen Befehl")?;
    }
    for handler in code.handlers() {
        check(
            handler.start <= handler.end && boundary(handler.start) && boundary(handler.end),
            "Bereich eines Handlers nicht an Befehlsgrenzen",
        )?;
        check(start(handler.target), "Handler nicht auf einen Befehl")?;
    }
    let mut previous_end = 0;
    for (start, end) in code.strict_ranges() {
        check(!code.is_strict(), "strikter Bereich in striktem Code")?;
        check(
            previous_end <= start && start < end && boundary(start) && boundary(end),
            "strikte Bereiche nicht geordnet",
        )?;
        previous_end = end;
    }
    let source = script.source.len();
    let mut previous = None;
    for &(offset, position) in &code.positions {
        check(
            previous.is_none_or(|previous| previous <= offset)
                && usize::try_from(offset).is_ok_and(|offset| offset <= length)
                && usize::try_from(position).is_ok_and(|position| position <= source),
            "Lage im Quelltext außerhalb",
        )?;
        previous = Some(offset);
    }
    Ok(())
}

/// Ob die Konstanten, die der Befehl liest, von der Art sind, die er
/// erwartet, und ob die Funktionen und Klassen darin im Eintrag stehen.
fn constants_fit(script: &CompiledScript, code: &Code, instruction: &Instruction) -> bool {
    let constant = |index: usize| code.constant(instruction.operand(index));
    let name = |index: usize| matches!(constant(index), Some(Constant::Name(_)));
    let string = |index: usize| matches!(constant(index), Some(Constant::String(_)));
    let function = |index: usize| match constant(index) {
        Some(Constant::Function(template)) => script.function(template.function).is_some(),
        _ => false,
    };
    match instruction.op {
        Op::LdaConstant => matches!(
            constant(0),
            Some(
                Constant::Number(_) | Constant::BigInt(_) | Constant::String(_) | Constant::Name(_)
            )
        ),
        Op::CheckHole | Op::DefineNamed | Op::StaNamed | Op::LdaSuperNamed | Op::StaSuperNamed => {
            name(1)
        }
        Op::ThrowConstAssignment
        | Op::ThrowIfHole
        | Op::ResolveGlobal
        | Op::LdaGlobal
        | Op::LdaGlobalRef
        | Op::StaGlobal
        | Op::InitGlobal
        | Op::TypeofGlobal
        | Op::DeleteGlobal
        | Op::LdaNamed
        | Op::DeleteNamed => name(0),
        Op::ResolveDynamic
        | Op::LdaDynamic
        | Op::StaDynamic
        | Op::TypeofDynamic
        | Op::DeleteDynamic
        | Op::WithBaseObject => matches!(constant(0), Some(Constant::Dynamic(_))),
        Op::CallEval => matches!(constant(1), Some(Constant::EvalSite(_))),
        Op::ImportCall => matches!(constant(0), Some(Constant::Module)),
        Op::CreateRegExp => string(0) && string(1),
        // Nur dieser Befehl liest den Platz eines Templates und schreibt
        // das Objekt hinein.
        Op::GetTemplateObject => matches!(constant(0), Some(Constant::Template(_))),
        Op::CreatePrivateName => string(0),
        Op::CreateClosure => function(0) && name(1),
        Op::CreateClosureKeyed => function(0),
        Op::CreateClass => match constant(0) {
            Some(Constant::Function(_)) => function(0),
            Some(Constant::DefaultConstructor { class, .. }) => script.class_text(*class).is_some(),
            _ => false,
        },
        _ => {
            // Jeder andere Befehl mit einer Konstanten fehlt hier; dann
            // gilt er als falsch.
            !instruction.op.operands().contains(&Operand::Constant)
        }
    }
}
