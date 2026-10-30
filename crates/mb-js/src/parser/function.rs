//! Funktionen, Methoden und Pfeilfunktionen (Abschnitt 15.1 bis 15.6 und
//! 15.8 bis 15.9), mit Parametern und Rumpf.

use std::collections::BTreeSet;

use super::expression::{ArrowHead, MethodModifiers};
use super::pattern::Names;
use super::scope::ScopeKind;
use super::{Context, ErrorKind, FunctionState, Parser, Result};
use crate::ast::{Atom, Function, FunctionId, FunctionKind, List, MethodKind, Node, NodeId};
use crate::lexer::{Goal, Punctuator};

/// Was vor den Parametern einer Funktion feststeht.
#[derive(Clone, Copy, Debug)]
pub(super) struct Signature {
    pub(super) kind: FunctionKind,
    pub(super) generator: bool,
    pub(super) is_async: bool,
}

/// Eine gelesene Parameterliste.
#[derive(Clone, Copy, Debug)]
struct Parameters {
    list: List,
    /// *IsSimpleParameterList*
    simple: bool,
    rest: bool,
    trailing_comma: bool,
}

impl Parser<'_> {
    /// Der Kontext im Code einer Funktion.
    pub(super) fn function_context(&self, signature: Signature) -> Context {
        let outer = self.context();
        let base = Context {
            strict: outer.strict,
            yield_: signature.generator,
            await_: signature.is_async,
            return_: true,
            new_target: true,
            ..Context::default()
        };
        match signature.kind {
            FunctionKind::Normal => base,
            // Pfeilfunktionen erben `super`, `new.target` und das Verbot von
            // `arguments`. Ein `static`-Block endet an ihnen.
            FunctionKind::Arrow => Context {
                super_property: outer.super_property,
                super_call: outer.super_call,
                new_target: outer.new_target,
                no_arguments: outer.no_arguments,
                ..base
            },
            FunctionKind::Method
            | FunctionKind::Getter
            | FunctionKind::Setter
            | FunctionKind::Constructor => Context {
                super_property: true,
                ..base
            },
            FunctionKind::DerivedConstructor => Context {
                super_property: true,
                super_call: true,
                ..base
            },
            FunctionKind::StaticBlock => Context {
                await_: true,
                return_: false,
                super_property: true,
                no_arguments: true,
                static_block: true,
                ..base
            },
        }
    }

    /// *FunctionDeclaration* und die Formen mit `*` und `async`. Das
    /// aktuelle Token ist `function`, `start` die Lage davor (mit
    /// `async`). Mit `default` darf der Name fehlen (`export default`).
    pub(super) fn function_declaration(
        &mut self,
        start: usize,
        is_async: bool,
        default: bool,
    ) -> Result<NodeId> {
        self.expect_keyword("function", Goal::RegExp)?;
        let generator = self.eat(Punctuator::Star, Goal::Div)?;
        let name = if default && self.at(Punctuator::LeftParenthesis, Goal::Div)? {
            None
        } else {
            Some(self.binding_identifier_token()?)
        };
        if let Some((atom, position)) = name {
            self.declare_function(atom, !generator && !is_async, position)?;
        }
        let signature = Signature {
            kind: FunctionKind::Normal,
            generator,
            is_async,
        };
        let function = self.function_rest(signature, name, start)?;
        Ok(self.node(Node::FunctionDeclaration(function), start))
    }

    /// *FunctionExpression* und die Formen mit `*` und `async`. Das
    /// aktuelle Token ist `function`.
    pub(super) fn function_expression(&mut self, start: usize, is_async: bool) -> Result<NodeId> {
        self.expect_keyword("function", Goal::RegExp)?;
        let generator = self.eat(Punctuator::Star, Goal::Div)?;
        let name = if self.at(Punctuator::LeftParenthesis, Goal::Div)? {
            None
        } else {
            // Der Name gehört zur Funktion: `yield` und `await` richten
            // sich nach ihr, nicht nach der Umgebung.
            let context = self.function.context;
            let await_identifiers = self.function.await_identifiers;
            self.function.context.yield_ = generator;
            self.function.context.await_ = is_async;
            self.function.context.static_block = false;
            let name = self.binding_identifier_token();
            self.function.context = context;
            self.function.await_identifiers = await_identifiers;
            Some(name?)
        };
        let signature = Signature {
            kind: FunctionKind::Normal,
            generator,
            is_async,
        };
        let function = self.function_rest(signature, name, start)?;
        Ok(self.node(Node::Function(function), start))
    }

    /// Eine Methode ab `(`, mit `kind` als Konstruktor einer Klasse.
    pub(super) fn method(
        &mut self,
        modifiers: MethodModifiers,
        start: usize,
        kind: Option<FunctionKind>,
    ) -> Result<FunctionId> {
        let kind = kind.unwrap_or(match modifiers.accessor {
            Some(MethodKind::Get) => FunctionKind::Getter,
            Some(MethodKind::Set) => FunctionKind::Setter,
            Some(MethodKind::Method) | None => FunctionKind::Method,
        });
        let signature = Signature {
            kind,
            generator: modifiers.generator,
            is_async: modifiers.is_async,
        };
        self.function_rest(signature, None, start)
    }

    /// Parameter und Rumpf einer Funktion ab `(` in ihrem eigenen
    /// Zustand und Gültigkeitsbereich. `name` ist der Name der Funktion
    /// für die Prüfungen, die eine Direktive `"use strict"` nachholt.
    fn function_rest(
        &mut self,
        signature: Signature,
        name: Option<(Atom, usize)>,
        start: usize,
    ) -> Result<FunctionId> {
        self.enter()?;
        let context = self.function_context(signature);
        let outer = std::mem::replace(&mut self.function, FunctionState::new(context));
        self.push_scope(ScopeKind::Function);
        let result = self.function_inner(signature, name, start);
        self.pop_scope();
        self.function = outer;
        self.leave();
        result
    }

    fn function_inner(
        &mut self,
        signature: Signature,
        name: Option<(Atom, usize)>,
        start: usize,
    ) -> Result<FunctionId> {
        let parameters_start = self.start(Goal::Div)?;
        let mut names: Names = Vec::new();
        let parameters = self.formal_parameters(&mut names)?;
        if self.function.yield_await > 0 {
            return Err(self.error_at(ErrorKind::InvalidYieldOrAwait, parameters_start));
        }
        let count = parameters.list.len();
        let valid = match signature.kind {
            FunctionKind::Getter => count == 0,
            FunctionKind::Setter => count == 1 && !parameters.rest && !parameters.trailing_comma,
            _ => true,
        };
        if !valid {
            return Err(self.error_at(ErrorKind::UnexpectedToken, parameters_start));
        }
        // Doppelte Namen erlauben nur einfache Parameter von Funktionen mit
        // `function` außerhalb des strikten Modus.
        let unique = signature.kind != FunctionKind::Normal || !parameters.simple || self.strict();
        if unique && let Some(position) = duplicate(&names) {
            return Err(self.error_at(ErrorKind::DuplicateParameter, position));
        }
        for &(parameter, _) in &names {
            self.declare_parameter(parameter);
        }
        let body = self.function_body(parameters.simple, &names, name)?;
        let function = Function {
            kind: signature.kind,
            generator: signature.generator,
            is_async: signature.is_async,
            strict: self.strict(),
            simple_parameters: parameters.simple,
            name: name.map(|(atom, _)| atom),
            parameters: parameters.list,
            body,
            expression_body: false,
            range: start..self.previous_end,
        };
        Ok(self.ast.add_function(function))
    }

    /// *FormalParameters* in Klammern.
    fn formal_parameters(&mut self, names: &mut Names) -> Result<Parameters> {
        self.expect(Punctuator::LeftParenthesis, Goal::Div)?;
        let mark = self.scratch.len();
        let mut parameters = Parameters {
            list: List::default(),
            simple: true,
            rest: false,
            trailing_comma: false,
        };
        loop {
            if self.at(Punctuator::RightParenthesis, Goal::Div)? {
                break;
            }
            if self.at(Punctuator::Ellipsis, Goal::Div)? {
                let rest_start = self.start(Goal::Div)?;
                self.advance();
                let target = self.binding_target(names)?;
                let rest = self.node(Node::Rest(target), rest_start);
                self.scratch.push(rest);
                parameters.simple = false;
                parameters.rest = true;
                break;
            }
            let element = self.binding_element(names)?;
            if !matches!(self.ast.node(element), Node::BindingIdentifier(_)) {
                parameters.simple = false;
            }
            self.scratch.push(element);
            if !self.eat(Punctuator::Comma, Goal::Div)? {
                break;
            }
            if self.at(Punctuator::RightParenthesis, Goal::Div)? {
                parameters.trailing_comma = true;
                break;
            }
        }
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        parameters.list = self.finish_list(mark);
        Ok(parameters)
    }

    /// *FunctionBody* in geschweiften Klammern, mit Direktiven.
    fn function_body(
        &mut self,
        simple: bool,
        names: &Names,
        name: Option<(Atom, usize)>,
    ) -> Result<List> {
        let start = self.start(Goal::Div)?;
        self.expect(Punctuator::LeftBrace, Goal::Div)?;
        let mark = self.scratch.len();
        let was_strict = self.strict();
        if self.directives()? {
            self.check_use_strict(start, simple, was_strict, names, name)?;
        }
        while !self.at(Punctuator::RightBrace, Goal::RegExp)? {
            let item = self.statement_list_item()?;
            self.scratch.push(item);
        }
        self.expect(Punctuator::RightBrace, Goal::RegExp)?;
        Ok(self.finish_list(mark))
    }

    /// Was eine Direktive `"use strict"` im Rumpf für die Parameter und den
    /// Namen der Funktion bedeutet: Sie müssen einfach sein, und was schon
    /// gelesen ist, wird nach den Regeln des strikten Modus nachgeprüft.
    fn check_use_strict(
        &self,
        position: usize,
        simple: bool,
        was_strict: bool,
        names: &Names,
        name: Option<(Atom, usize)>,
    ) -> Result<()> {
        if !simple {
            return Err(self.error_at(ErrorKind::UseStrictWithComplexParameters, position));
        }
        if was_strict {
            return Ok(());
        }
        for &(atom, at) in name.iter().chain(names.iter()) {
            if self.atom_is(atom, "eval") || self.atom_is(atom, "arguments") {
                return Err(self.error_at(ErrorKind::StrictEvalOrArguments, at));
            }
            if self.atom_is_strict_reserved(atom) {
                return Err(self.error_at(ErrorKind::ReservedWord, at));
            }
        }
        if let Some(at) = duplicate(names) {
            return Err(self.error_at(ErrorKind::DuplicateParameter, at));
        }
        Ok(())
    }

    /// *ArrowFunction* und *AsyncArrowFunction*: der Kopf ist gelesen, das
    /// aktuelle Token ist `=>`.
    pub(super) fn arrow_function(
        &mut self,
        head: ArrowHead,
        in_: bool,
        start: usize,
    ) -> Result<NodeId> {
        // *ArrowParameters Contains YieldExpression* und *AwaitExpression*.
        if self.function.yield_await > head.yield_await {
            return Err(self.error_at(ErrorKind::InvalidYieldOrAwait, start));
        }
        // Im Kopf einer async-Pfeilfunktion ist `await` reserviert.
        if head.is_async && self.function.await_identifiers > head.await_identifiers {
            return Err(self.error_at(ErrorKind::ReservedWord, start));
        }
        let (items, list) = match *self.ast.node(head.node) {
            Node::Sequence(list) => (self.ast.list(list).to_vec(), list),
            _ => {
                let items = vec![head.node];
                let list = self.ast.add_list(&items);
                (items, list)
            }
        };
        let mut names: Names = Vec::new();
        let count = items.len();
        for (index, item) in items.iter().enumerate() {
            self.arrow_parameter(*item, index.saturating_add(1) == count, &mut names)?;
        }
        let simple = items
            .iter()
            .all(|item| matches!(self.ast.node(*item), Node::BindingIdentifier(_)));
        if let Some(position) = duplicate(&names) {
            return Err(self.error_at(ErrorKind::DuplicateParameter, position));
        }
        self.advance();
        self.enter()?;
        let signature = Signature {
            kind: FunctionKind::Arrow,
            generator: false,
            is_async: head.is_async,
        };
        let context = self.function_context(signature);
        let outer = std::mem::replace(&mut self.function, FunctionState::new(context));
        self.push_scope(ScopeKind::Function);
        let result = self.arrow_body(signature, list, simple, &names, in_, start);
        self.pop_scope();
        self.function = outer;
        self.leave();
        let function = result?;
        Ok(self.node(Node::Function(function), start))
    }

    fn arrow_body(
        &mut self,
        signature: Signature,
        parameters: List,
        simple: bool,
        names: &Names,
        in_: bool,
        start: usize,
    ) -> Result<FunctionId> {
        for &(parameter, _) in names {
            self.declare_parameter(parameter);
        }
        let expression_body = !self.at(Punctuator::LeftBrace, Goal::RegExp)?;
        let body = if expression_body {
            let expression = self.assignment(in_, None)?;
            self.ast.add_list(&[expression])
        } else {
            self.function_body(simple, names, None)?
        };
        let function = Function {
            kind: FunctionKind::Arrow,
            generator: false,
            is_async: signature.is_async,
            strict: self.strict(),
            simple_parameters: simple,
            name: None,
            parameters,
            body,
            expression_body,
            range: start..self.previous_end,
        };
        Ok(self.ast.add_function(function))
    }
}

/// Die Lage des ersten Namens, der in `names` zum zweiten Mal vorkommt.
fn duplicate(names: &Names) -> Option<usize> {
    let mut seen = BTreeSet::new();
    names
        .iter()
        .find(|(atom, _)| !seen.insert(*atom))
        .map(|&(_, position)| position)
}
