//! Klassen (Abschnitt 15.7) mit privaten Namen, Feldern und
//! `static`-Blöcken. Der ganze Code einer Klasse ist strikt.

use super::function::Signature;
use super::scope::{Binding, ScopeKind};
use super::{
    Context, Cover, ErrorKind, FunctionState, Parser, PrivateDeclaration, PrivateScope, Result,
    SyntaxError,
};
use crate::ast::{
    Atom, Class, ClassId, Function, FunctionId, FunctionKind, List, MethodKind, Node, NodeId,
};
use crate::lexer::{Goal, Punctuator, TokenKind};

/// Ein Element einer Klasse.
enum Element {
    Constructor(FunctionId),
    Other(NodeId),
}

impl Parser<'_> {
    /// *ClassDeclaration*; mit `default` darf der Name fehlen
    /// (`export default`). Das aktuelle Token ist `class`.
    pub(super) fn class_declaration(&mut self, default: bool) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let strict = self.function.context.strict;
        self.function.context.strict = true;
        let result = self.class_declaration_inner(start, default);
        self.function.context.strict = strict;
        result
    }

    fn class_declaration_inner(&mut self, start: usize, default: bool) -> Result<NodeId> {
        let anonymous = default
            && (self.at_keyword("extends", Goal::Div)?
                || self.at(Punctuator::LeftBrace, Goal::Div)?);
        let name = if anonymous {
            None
        } else {
            let (atom, position) = self.binding_identifier_token()?;
            self.declare_lexical(atom, Binding::Lexical, position)?;
            Some(atom)
        };
        let class = self.class_tail(start, name)?;
        Ok(self.node(Node::ClassDeclaration(class), start))
    }

    /// *ClassExpression*. Das aktuelle Token ist `class`.
    pub(super) fn class_expression(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let strict = self.function.context.strict;
        self.function.context.strict = true;
        let result = self.class_expression_inner(start);
        self.function.context.strict = strict;
        result
    }

    fn class_expression_inner(&mut self, start: usize) -> Result<NodeId> {
        let anonymous =
            self.at_keyword("extends", Goal::Div)? || self.at(Punctuator::LeftBrace, Goal::Div)?;
        let name = if anonymous {
            None
        } else {
            Some(self.binding_identifier_token()?.0)
        };
        let class = self.class_tail(start, name)?;
        Ok(self.node(Node::Class(class), start))
    }

    /// *ClassTail*
    fn class_tail(&mut self, start: usize, name: Option<Atom>) -> Result<ClassId> {
        self.enter()?;
        let result = self.class_tail_inner(start, name);
        self.leave();
        result
    }

    fn class_tail_inner(&mut self, start: usize, name: Option<Atom>) -> Result<ClassId> {
        // Die Oberklasse gehört noch nicht zum Bereich der privaten Namen.
        let heritage = if self.eat_keyword("extends", Goal::Div)? {
            let mut cover = Cover::default();
            let heritage = self.left_hand_side(&mut cover)?;
            cover.check()?;
            Some(heritage)
        } else {
            None
        };
        self.expect(Punctuator::LeftBrace, Goal::Div)?;
        self.classes.push(PrivateScope::default());
        let result = self.class_body(heritage.is_some());
        let scope = self.classes.pop().unwrap_or_default();
        let (constructor, elements) = result?;
        self.resolve_private_names(scope)?;
        Ok(self.ast.add_class(Class {
            name,
            heritage,
            constructor,
            elements,
            range: start..self.previous_end,
        }))
    }

    /// *ClassBody* bis einschließlich `}`.
    fn class_body(&mut self, derived: bool) -> Result<(Option<FunctionId>, List)> {
        let mark = self.scratch.len();
        let mut constructor = None;
        loop {
            if self.eat(Punctuator::Semicolon, Goal::Div)? {
                continue;
            }
            if self.at(Punctuator::RightBrace, Goal::Div)? {
                break;
            }
            let start = self.start(Goal::Div)?;
            match self.class_element(derived)? {
                Element::Constructor(function) => {
                    if constructor.is_some() {
                        return Err(self.error_at(ErrorKind::InvalidClassElement, start));
                    }
                    constructor = Some(function);
                }
                Element::Other(element) => self.scratch.push(element),
            }
        }
        self.expect(Punctuator::RightBrace, Goal::Div)?;
        Ok((constructor, self.finish_list(mark)))
    }

    /// *ClassElement* außer `;`.
    fn class_element(&mut self, derived: bool) -> Result<Element> {
        let start = self.start(Goal::Div)?;
        let mut is_static = false;
        if self.at_keyword("static", Goal::Div)? {
            // `static` ist ein Vorsatz, wenn danach ein Name, `*`, `[` oder
            // `{` folgt; sonst der Name einer Methode oder eines Feldes.
            let modifier = self.peek(Goal::Div, Goal::Div)?.is_some_and(|next| {
                !matches!(
                    next.kind,
                    TokenKind::End
                        | TokenKind::Punctuator(
                            Punctuator::LeftParenthesis
                                | Punctuator::Assign
                                | Punctuator::Semicolon
                                | Punctuator::RightBrace
                        )
                )
            });
            if modifier {
                self.advance();
                is_static = true;
                if self.at(Punctuator::LeftBrace, Goal::Div)? {
                    return Ok(Element::Other(self.static_block(start)?));
                }
            }
        }
        let modifiers = self.method_modifiers(true)?;
        let key_start = self.start(Goal::Div)?;
        let (key, name) = self.property_name(true)?;
        let private = match *self.ast.node(key) {
            Node::PrivateName(atom) => Some(atom),
            _ => None,
        };
        if private.is_some_and(|atom| self.atom_is(atom, "constructor")) {
            return Err(self.error_at(ErrorKind::InvalidClassElement, key_start));
        }
        let named = |parser: &Self, text: &str| name.is_some_and(|name| parser.atom_is(name, text));
        if self.at(Punctuator::LeftParenthesis, Goal::Div)? {
            if !is_static && named(self, "constructor") {
                if modifiers.any() {
                    return Err(self.error_at(ErrorKind::InvalidClassElement, key_start));
                }
                let kind = if derived {
                    FunctionKind::DerivedConstructor
                } else {
                    FunctionKind::Constructor
                };
                return Ok(Element::Constructor(self.method(
                    modifiers,
                    start,
                    Some(kind),
                )?));
            }
            if is_static && named(self, "prototype") {
                return Err(self.error_at(ErrorKind::InvalidClassElement, key_start));
            }
            let function = self.method(modifiers, start, None)?;
            let kind = modifiers.accessor.unwrap_or(MethodKind::Method);
            if let Some(atom) = private {
                self.declare_private(atom, kind, is_static, key_start)?;
            }
            let method = self.node(
                Node::ClassMethod {
                    key,
                    function,
                    kind,
                    is_static,
                },
                start,
            );
            return Ok(Element::Other(method));
        }
        if modifiers.any() {
            return Err(self.unexpected(Goal::Div));
        }
        // *FieldDefinition*
        if named(self, "constructor") || (is_static && named(self, "prototype")) {
            return Err(self.error_at(ErrorKind::InvalidClassElement, key_start));
        }
        if let Some(atom) = private {
            self.declare_private(atom, MethodKind::Method, is_static, key_start)?;
        }
        let value = if self.eat(Punctuator::Assign, Goal::Div)? {
            Some(self.field_initializer()?)
        } else {
            None
        };
        self.semicolon()?;
        Ok(Element::Other(self.node(
            Node::ClassField {
                key,
                value,
                is_static,
            },
            start,
        )))
    }

    /// Die Initialisierung eines Feldes. Sie ist eine eigene Funktion für
    /// `arguments` und `super`; `yield` und `await` richten sich nach der
    /// Umgebung (*Initializer[+In, ?Yield, ?Await]*).
    fn field_initializer(&mut self) -> Result<NodeId> {
        let outer = self.context();
        let context = Context {
            strict: true,
            yield_: outer.yield_,
            await_: outer.await_,
            super_property: true,
            new_target: true,
            no_arguments: true,
            ..Context::default()
        };
        let state = std::mem::replace(&mut self.function, FunctionState::new(context));
        let result = self.assignment(true, None);
        self.function = state;
        result
    }

    /// *ClassStaticBlock*: `static` ist gelesen, das aktuelle Token ist
    /// `{`.
    fn static_block(&mut self, start: usize) -> Result<NodeId> {
        let signature = Signature {
            kind: FunctionKind::StaticBlock,
            generator: false,
            is_async: false,
        };
        let context = self.function_context(signature);
        let state = std::mem::replace(&mut self.function, FunctionState::new(context));
        self.push_scope(ScopeKind::Function);
        let result = self.static_block_body();
        self.pop_scope();
        self.function = state;
        let body = result?;
        let function = self.ast.add_function(Function {
            kind: FunctionKind::StaticBlock,
            generator: false,
            is_async: false,
            strict: true,
            simple_parameters: true,
            name: None,
            parameters: List::default(),
            body,
            expression_body: false,
            range: start..self.previous_end,
        });
        Ok(self.node(Node::StaticBlock(function), start))
    }

    fn static_block_body(&mut self) -> Result<List> {
        self.expect(Punctuator::LeftBrace, Goal::Div)?;
        let mark = self.scratch.len();
        while !self.at(Punctuator::RightBrace, Goal::RegExp)? {
            let item = self.statement_list_item()?;
            self.scratch.push(item);
        }
        self.expect(Punctuator::RightBrace, Goal::RegExp)?;
        Ok(self.finish_list(mark))
    }

    /// Deklariert einen privaten Namen der innersten Klasse. Doppelt darf
    /// ein Name nur als Getter und Setter vorkommen, beide statisch oder
    /// beide nicht.
    fn declare_private(
        &mut self,
        name: Atom,
        kind: MethodKind,
        is_static: bool,
        position: usize,
    ) -> Result<()> {
        let new = match kind {
            MethodKind::Get => PrivateDeclaration::Getter { is_static },
            MethodKind::Set => PrivateDeclaration::Setter { is_static },
            MethodKind::Method => PrivateDeclaration::Other,
        };
        let Some(scope) = self.classes.last_mut() else {
            return Ok(());
        };
        let merged = match (scope.declared.get(&name).copied(), new) {
            (None, new) => new,
            (
                Some(PrivateDeclaration::Getter { is_static: first }),
                PrivateDeclaration::Setter { is_static: second },
            )
            | (
                Some(PrivateDeclaration::Setter { is_static: first }),
                PrivateDeclaration::Getter { is_static: second },
            ) if first == second => PrivateDeclaration::Accessor,
            _ => {
                return Err(SyntaxError {
                    kind: ErrorKind::DuplicatePrivateName,
                    position,
                });
            }
        };
        scope.declared.insert(name, merged);
        Ok(())
    }

    /// Merkt sich einen verwendeten privaten Namen für das Ende der
    /// Klasse. Außerhalb jeder Klasse ist er ein Fehler.
    pub(super) fn use_private_name(&mut self, name: Atom, position: usize) -> Result<()> {
        match self.classes.last_mut() {
            Some(scope) => {
                scope.used.push((name, position));
                Ok(())
            }
            None => Err(self.error_at(ErrorKind::UndeclaredPrivateName, position)),
        }
    }

    /// Am Ende einer Klasse: Was sie nicht deklariert, muss eine umgebende
    /// Klasse deklarieren (*AllPrivateIdentifiersValid*).
    fn resolve_private_names(&mut self, scope: PrivateScope) -> Result<()> {
        for (name, position) in scope.used {
            if scope.declared.contains_key(&name) {
                continue;
            }
            match self.classes.last_mut() {
                Some(outer) => outer.used.push((name, position)),
                None => return Err(self.error_at(ErrorKind::UndeclaredPrivateName, position)),
            }
        }
        Ok(())
    }
}
