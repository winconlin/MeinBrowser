//! Woraus die Funktionen eines Scripts, Moduls oder `eval` ihren Code und
//! ihre Angaben holen.
//!
//! Meist ist das der Syntaxbaum: Eine Funktion wird bei ihrem ersten Aufruf
//! aus ihm übersetzt, und `length`, ihre Art und ihr Quelltext stehen in
//! ihm. Hat der Parser ihren Rumpf nur vorgeprüft
//! ([`crate::parser::PreCheck`]), liest der Baum ihn vor dem Übersetzen
//! nach; darum liegt er in einer [`RefCell`]. Ein Script aus dem Zwischenspeicher für Bytecode
//! ([`crate::code_cache`]) hat keinen Syntaxbaum. Dort ist jede Funktion
//! schon übersetzt, und die Angaben, die die Laufzeit braucht, liegen
//! daneben.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use crate::ast::{Ast, ClassId, Function, FunctionId, FunctionKind, Node};
use crate::bytecode::{Code, FunctionTemplate};
use crate::code_cache::CompiledScript;
use crate::compiler::{CompileError, compile_function};

/// Der Ursprung der Funktionen eines Codes.
#[derive(Clone, Debug)]
pub(crate) enum Unit {
    /// Ein Syntaxbaum, aus dem Funktionen bei ihrem ersten Aufruf übersetzt
    /// werden.
    Ast(Rc<RefCell<Ast>>),
    /// Ein Script, dessen Funktionen alle schon übersetzt sind.
    Compiled(Rc<CompiledScript>),
}

/// Was die Laufzeit beim Anlegen eines Funktionsobjekts über die Funktion
/// wissen muss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shape {
    pub(crate) kind: FunctionKind,
    pub(crate) generator: bool,
    pub(crate) is_async: bool,
    /// ExpectedArgumentCount (Abschnitt 15.1.5) für `length`.
    pub(crate) length: u32,
}

impl Shape {
    pub(crate) fn of(ast: &Ast, function: &Function) -> Shape {
        Shape {
            kind: function.kind,
            generator: function.generator,
            is_async: function.is_async,
            length: expected_argument_count(ast, function),
        }
    }

    /// Ob Funktionsobjekte der Funktion MakeConstructor bekommen: gewöhnliche
    /// Funktionen aus Deklarationen und Ausdrücken, keine Generatoren und
    /// keine mit `async` (Abschnitte 15.2.4 und 15.2.5). Pfeilfunktionen
    /// (Abschnitt 15.3.4), Methoden, Getter und Setter (Abschnitt 15.4)
    /// nicht. Die Konstruktoren von Klassen entstehen nur mit
    /// [`crate::function::instantiate_class_constructor`].
    pub(crate) fn constructs(self) -> bool {
        self.kind == FunctionKind::Normal && !self.generator && !self.is_async
    }
}

/// ExpectedArgumentCount (Abschnitt 15.1.5): die Parameter vor dem ersten
/// mit Vorgabewert oder dem Rest-Parameter.
fn expected_argument_count(ast: &Ast, function: &Function) -> u32 {
    let count = ast
        .list(function.parameters)
        .iter()
        .take_while(|&&parameter| {
            !matches!(
                ast.node(parameter),
                Node::WithDefault { .. } | Node::Rest(_)
            )
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Der Quelltext, den `toString` für die Funktion zeigt: bei dem
/// Konstruktor einer Klasse die ganze Klasse, sonst die Funktion.
pub(crate) fn function_text(ast: &Ast, id: FunctionId) -> Option<Range<usize>> {
    let function = ast.function(id)?;
    Some(match function.kind {
        FunctionKind::Constructor | FunctionKind::DerivedConstructor => ast
            .class_with_constructor(id)
            .map_or(function.range.clone(), |class| class.range.clone()),
        _ => function.range.clone(),
    })
}

impl Unit {
    /// Der Ursprung für den Syntaxbaum `ast`.
    pub(crate) fn from_ast(ast: Ast) -> Unit {
        Unit::Ast(Rc::new(RefCell::new(ast)))
    }

    /// Wendet `read` auf den Syntaxbaum an. Nur während des Nachlesens ist
    /// er geliehen, und dabei läuft nichts, was hierher fragen könnte; dann
    /// ist das Ergebnis `None`.
    fn read<T>(ast: &RefCell<Ast>, read: impl FnOnce(&Ast) -> Option<T>) -> Option<T> {
        read(&*ast.try_borrow().ok()?)
    }

    pub(crate) fn shape(&self, id: FunctionId) -> Option<Shape> {
        match self {
            Unit::Ast(ast) => Unit::read(ast, |ast| Some(Shape::of(ast, ast.function(id)?))),
            Unit::Compiled(script) => Some(script.function(id)?.shape),
        }
    }

    /// Wo der Quelltext der Funktion für `toString` liegt.
    pub(crate) fn function_text(&self, id: FunctionId) -> Option<Range<usize>> {
        match self {
            Unit::Ast(ast) => Unit::read(ast, |ast| function_text(ast, id)),
            Unit::Compiled(script) => Some(script.function(id)?.text()),
        }
    }

    /// Wo der Quelltext einer Klasse liegt, für ihren Standardkonstruktor.
    pub(crate) fn class_text(&self, id: ClassId) -> Option<Range<usize>> {
        match self {
            Unit::Ast(ast) => Unit::read(ast, |ast| Some(ast.class(id)?.range.clone())),
            Unit::Compiled(script) => script.class_text(id),
        }
    }

    /// Der Quelltext an der Lage `range`.
    pub(crate) fn text(&self, range: Range<usize>) -> Option<Vec<u16>> {
        match self {
            Unit::Ast(ast) => Unit::read(ast, |ast| Some(ast.source().get(range)?.to_vec())),
            Unit::Compiled(script) => Some(script.source().get(range)?.to_vec()),
        }
    }

    /// Der Bytecode der Funktion an der Stelle `template`. Einen nur
    /// vorgeprüften Rumpf liest der Syntaxbaum vorher nach.
    pub(crate) fn compile(&self, template: &FunctionTemplate) -> Result<Rc<Code>, CompileError> {
        match self {
            Unit::Ast(ast) => {
                let mut ast = ast
                    .try_borrow_mut()
                    .map_err(|_| CompileError::Unsupported(0))?;
                if ast.is_lazy(template.function) {
                    // Das scheitert nur, wenn der Baum nicht mehr zum
                    // Quelltext passt.
                    let position = ast
                        .function(template.function)
                        .map_or(0, |function| function.range.start);
                    ast.expand(template.function)
                        .map_err(|_| CompileError::Unsupported(position))?;
                }
                compile_function(&ast, template).map(Rc::new)
            }
            Unit::Compiled(script) => match script.function(template.function) {
                Some(function) => function.code.clone(),
                None => Err(CompileError::Unsupported(0)),
            },
        }
    }

    /// Ob der Quelltext zur Laufzeit aus `eval` oder `Function` kam.
    pub(crate) fn is_dynamic(&self) -> bool {
        match self {
            Unit::Ast(ast) => Unit::read(ast, |ast| Some(ast.is_dynamic())).unwrap_or(false),
            Unit::Compiled(_) => false,
        }
    }

    /// Der Syntaxbaum, wenn es einen gibt.
    pub(crate) fn ast(&self) -> Option<&Rc<RefCell<Ast>>> {
        match self {
            Unit::Ast(ast) => Some(ast),
            Unit::Compiled(_) => None,
        }
    }
}
