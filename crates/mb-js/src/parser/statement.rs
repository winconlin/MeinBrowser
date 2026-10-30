//! Anweisungen und Deklarationen (Abschnitt 14) mit den Erweiterungen aus
//! Anhang B.3.

use super::pattern::Names;
use super::scope::{Binding, ScopeKind};
use super::{Cover, ErrorKind, Label, Parser, Result};
use crate::ast::{Atom, DeclarationKind, Node, NodeId};
use crate::lexer::{Goal, Punctuator, TokenKind};

/// Wo eine Anweisung steht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Position {
    /// In einer Liste von Anweisungen.
    List,
    /// Als Rumpf von `if`, einer Schleife oder `with`; dort darf keine
    /// Funktion mit Marke stehen (*IsLabelledFunction*).
    Body,
}

/// Wo eine Deklaration mit `var`, `let`, `const` oder `using` steht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Declarations {
    Statement,
    /// Im Kopf von `for`: Ohne `in` und vielleicht vor `in` oder `of`.
    ForHead,
}

/// Womit eine Anweisung beginnt.
#[derive(Clone, Copy)]
enum StatementStart {
    Var,
    If,
    For,
    While,
    Do,
    Jump { is_break: bool },
    Return,
    With,
    Switch,
    Throw,
    Try,
    Debugger,
    Labelled,
    Block,
    Empty,
    Expression,
}

/// Der Kopf von `for` bis vor das erste `;` oder bis vor `in` oder `of`.
enum ForHead {
    Classic(Option<NodeId>),
    InOf { left: NodeId, of: bool },
}

impl Parser<'_> {
    /// *StatementListItem*
    pub(super) fn statement_list_item(&mut self) -> Result<NodeId> {
        self.enter()?;
        let pending = self.function.pending_labels;
        self.function.pending_labels = self.function.labels.len();
        let result = self.statement_list_item_inner();
        self.function.pending_labels = pending;
        self.leave();
        result
    }

    fn statement_list_item_inner(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        if let Some(name) = self.name(Goal::RegExp)?
            && !name.escaped
        {
            match name.value.as_str() {
                "function" => return self.function_declaration(start, false, false),
                "class" => return self.class_declaration(false),
                "async" if self.async_function_follows()? => {
                    self.advance();
                    return self.function_declaration(start, true, false);
                }
                "const" => return self.lexical_declaration(DeclarationKind::Const),
                "let" if self.let_declaration_follows()? => {
                    return self.lexical_declaration(DeclarationKind::Let);
                }
                "using" if self.using_declaration_follows()? => {
                    return self.lexical_declaration(DeclarationKind::Using);
                }
                "await" if self.await_operator() && self.await_using_follows()? => {
                    return self.lexical_declaration(DeclarationKind::AwaitUsing);
                }
                _ => {}
            }
        }
        self.statement(Position::List)
    }

    /// Ob nach `async` auf derselben Zeile `function` folgt.
    pub(super) fn async_function_follows(&mut self) -> Result<bool> {
        Ok(self.peek(Goal::RegExp, Goal::Div)?.is_some_and(|next| {
            !next.newline_before
                && matches!(&next.kind, TokenKind::Name(name) if !name.escaped && name.value == "function")
        }))
    }

    /// Ob `let` eine Deklaration beginnt: Es folgt `[`, `{` oder ein
    /// Bezeichner.
    pub(super) fn let_declaration_follows(&mut self) -> Result<bool> {
        Ok(self
            .peek(Goal::RegExp, Goal::Div)?
            .is_some_and(|next| match &next.kind {
                TokenKind::Punctuator(Punctuator::LeftBracket | Punctuator::LeftBrace) => true,
                TokenKind::Name(name) => !super::is_reserved_word(&name.value),
                _ => false,
            }))
    }

    /// Ob `using` eine Deklaration beginnt: Auf derselben Zeile folgt ein
    /// Bezeichner.
    fn using_declaration_follows(&mut self) -> Result<bool> {
        Ok(self.peek(Goal::RegExp, Goal::Div)?.is_some_and(|next| {
            !next.newline_before
                && matches!(&next.kind, TokenKind::Name(name) if !super::is_reserved_word(&name.value))
        }))
    }

    /// Ob `await` ein Operator sein kann; im `static`-Block ist er
    /// verboten und wird als Ausdruck gemeldet.
    fn await_operator(&self) -> bool {
        let context = self.context();
        context.await_ && !context.static_block
    }

    /// Ob `await using` mit einem Bezeichner folgt, alles auf einer Zeile.
    fn await_using_follows(&mut self) -> Result<bool> {
        self.token(Goal::RegExp)?;
        let Some(lookahead) = &self.current else {
            return Ok(false);
        };
        let mut lexer = lookahead.after.clone();
        let Ok(second) = lexer.next_token(Goal::Div) else {
            return Ok(false);
        };
        if second.newline_before
            || !matches!(&second.kind, TokenKind::Name(name) if !name.escaped && name.value == "using")
        {
            return Ok(false);
        }
        let Ok(third) = lexer.next_token(Goal::Div) else {
            return Ok(false);
        };
        Ok(!third.newline_before
            && matches!(&third.kind, TokenKind::Name(name) if !super::is_reserved_word(&name.value)))
    }

    /// *Statement*
    pub(super) fn statement(&mut self, position: Position) -> Result<NodeId> {
        self.enter()?;
        let result = self.statement_inner(position);
        self.leave();
        result
    }

    fn statement_inner(&mut self, position: Position) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        match self.statement_start()? {
            StatementStart::Var => self.var_statement(start),
            StatementStart::If => self.if_statement(),
            StatementStart::For => self.iteration(|parser| parser.for_statement()),
            StatementStart::While => self.iteration(|parser| parser.while_statement()),
            StatementStart::Do => self.iteration(|parser| parser.do_while_statement()),
            StatementStart::Jump { is_break } => self.jump(is_break),
            StatementStart::Return => self.return_statement(),
            StatementStart::With => self.with_statement(),
            StatementStart::Switch => self.switch_statement(),
            StatementStart::Throw => self.throw_statement(),
            StatementStart::Try => self.try_statement(),
            StatementStart::Debugger => self.debugger_statement(start),
            StatementStart::Labelled => self.labelled_statement(start, position),
            StatementStart::Block => self.block_statement(),
            StatementStart::Empty => {
                self.advance();
                Ok(self.node(Node::Empty, start))
            }
            StatementStart::Expression => self.expression_statement(start),
        }
    }

    /// Welche Anweisung beginnt. Wörter, mit denen hier keine Anweisung
    /// beginnen darf, sind Fehler.
    fn statement_start(&mut self) -> Result<StatementStart> {
        if let Some(name) = self.name(Goal::RegExp)? {
            if !name.escaped {
                match name.value.as_str() {
                    "var" => return Ok(StatementStart::Var),
                    "if" => return Ok(StatementStart::If),
                    "for" => return Ok(StatementStart::For),
                    "while" => return Ok(StatementStart::While),
                    "do" => return Ok(StatementStart::Do),
                    "continue" | "break" => {
                        return Ok(StatementStart::Jump {
                            is_break: name.value == "break",
                        });
                    }
                    "return" => return Ok(StatementStart::Return),
                    "with" => return Ok(StatementStart::With),
                    "switch" => return Ok(StatementStart::Switch),
                    "throw" => return Ok(StatementStart::Throw),
                    "try" => return Ok(StatementStart::Try),
                    "debugger" => return Ok(StatementStart::Debugger),
                    "function" => return Err(self.error(ErrorKind::InvalidFunctionDeclaration)),
                    "class" => return Err(self.error(ErrorKind::UnexpectedToken)),
                    "async" if self.async_function_follows()? => {
                        return Err(self.error(ErrorKind::InvalidFunctionDeclaration));
                    }
                    "let" => {
                        if self.peek(Goal::RegExp, Goal::Div)?.is_some_and(|next| {
                            next.kind == TokenKind::Punctuator(Punctuator::LeftBracket)
                        }) {
                            return Err(self.error(ErrorKind::UnexpectedToken));
                        }
                    }
                    "import" => {
                        let next = self.peek(Goal::RegExp, Goal::Div)?;
                        let expression = next.is_some_and(|next| {
                            matches!(
                                next.kind,
                                TokenKind::Punctuator(
                                    Punctuator::LeftParenthesis | Punctuator::Dot
                                )
                            )
                        });
                        if !expression {
                            return Err(self.error(ErrorKind::InvalidModuleItem));
                        }
                    }
                    "export" => return Err(self.error(ErrorKind::InvalidModuleItem)),
                    _ => {}
                }
            }
            let label = self
                .peek(Goal::RegExp, Goal::Div)?
                .is_some_and(|next| next.kind == TokenKind::Punctuator(Punctuator::Colon));
            if label {
                return Ok(StatementStart::Labelled);
            }
        }
        Ok(match self.punctuator(Goal::RegExp)? {
            Some(Punctuator::LeftBrace) => StatementStart::Block,
            Some(Punctuator::Semicolon) => StatementStart::Empty,
            _ => StatementStart::Expression,
        })
    }

    fn var_statement(&mut self, start: usize) -> Result<NodeId> {
        self.advance();
        let node = self
            .declarations(DeclarationKind::Var, Declarations::Statement, start)?
            .0;
        self.semicolon()?;
        self.ast.set_range(node, start..self.previous_end);
        Ok(node)
    }

    fn debugger_statement(&mut self, start: usize) -> Result<NodeId> {
        self.advance();
        self.semicolon()?;
        Ok(self.node(Node::Debugger, start))
    }

    fn expression_statement(&mut self, start: usize) -> Result<NodeId> {
        let expression = self.expression(true)?;
        self.semicolon()?;
        Ok(self.node(Node::Expression(expression), start))
    }

    /// Eine Schleife: Die Marken direkt davor markieren sie, und der Rumpf
    /// erlaubt `break` und `continue`.
    fn iteration(&mut self, parse: impl FnOnce(&mut Self) -> Result<NodeId>) -> Result<NodeId> {
        let pending = self.function.pending_labels;
        for label in self.function.labels.iter_mut().skip(pending) {
            label.iteration = true;
        }
        self.function.pending_labels = self.function.labels.len();
        self.function.iterations = self.function.iterations.saturating_add(1);
        let result = parse(self);
        self.function.iterations = self.function.iterations.saturating_sub(1);
        self.function.pending_labels = pending;
        result
    }

    /// Eine Anweisung als Rumpf, ohne Marken von außen.
    fn body(&mut self) -> Result<NodeId> {
        let pending = self.function.pending_labels;
        self.function.pending_labels = self.function.labels.len();
        let result = self.statement(Position::Body);
        self.function.pending_labels = pending;
        result
    }

    fn labelled_statement(&mut self, start: usize, position: Position) -> Result<NodeId> {
        let label = self.label(start)?;
        if self
            .function
            .labels
            .iter()
            .any(|existing| existing.name == label)
        {
            return Err(self.error_at(ErrorKind::DuplicateLabel, start));
        }
        self.function.labels.push(Label {
            name: label,
            iteration: false,
        });
        let body = if self.at_keyword("function", Goal::RegExp)? {
            // *LabelledItem : FunctionDeclaration*, nur außerhalb des
            // strikten Modus (Anhang B.3.1).
            if self.strict() || position == Position::Body {
                Err(self.error(ErrorKind::InvalidLabelledFunction))
            } else {
                let function_start = self.start(Goal::RegExp)?;
                self.plain_function_declaration(function_start)
            }
        } else {
            self.statement(position)
        };
        self.function.labels.pop();
        let body = body?;
        Ok(self.node(Node::Labeled { label, body }, start))
    }

    /// Die Marke mit dem Doppelpunkt danach.
    fn label(&mut self, start: usize) -> Result<Atom> {
        let Some(name) = self.name(Goal::RegExp)? else {
            return Err(self.unexpected(Goal::RegExp));
        };
        let label = self.label_identifier(&name, start)?;
        self.advance();
        self.expect(Punctuator::Colon, Goal::Div)?;
        Ok(label)
    }

    /// Eine Funktionsdeklaration ohne `*` und `async`, wie sie nach einer
    /// Marke und in `if` stehen darf.
    fn plain_function_declaration(&mut self, start: usize) -> Result<NodeId> {
        let declaration = self.function_declaration(start, false, false)?;
        if let Node::FunctionDeclaration(function) = *self.ast.node(declaration)
            && self
                .ast
                .function(function)
                .is_some_and(|function| function.generator)
        {
            return Err(self.error_at(ErrorKind::InvalidFunctionDeclaration, start));
        }
        Ok(declaration)
    }

    /// *BlockStatement*
    fn block_statement(&mut self) -> Result<NodeId> {
        self.block(ScopeKind::Block)
    }

    /// *Block* in einem neuen Gültigkeitsbereich der Art `kind`.
    pub(super) fn block(&mut self, kind: ScopeKind) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.expect(Punctuator::LeftBrace, Goal::RegExp)?;
        self.push_scope(kind);
        let mark = self.scratch.len();
        while !self.at(Punctuator::RightBrace, Goal::RegExp)? {
            let item = self.statement_list_item()?;
            self.scratch.push(item);
        }
        self.expect(Punctuator::RightBrace, Goal::RegExp)?;
        self.pop_scope();
        let list = self.finish_list(mark);
        Ok(self.node(Node::Block(list), start))
    }

    /// *LexicalDeclaration* mit `let`, `const`, `using` oder `await using`.
    fn lexical_declaration(&mut self, kind: DeclarationKind) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        if matches!(kind, DeclarationKind::Using | DeclarationKind::AwaitUsing)
            && matches!(
                self.scopes.last().map(|scope| scope.kind),
                Some(ScopeKind::Script | ScopeKind::Switch)
            )
        {
            return Err(self.error_at(ErrorKind::InvalidUsing, start));
        }
        self.advance();
        if kind == DeclarationKind::AwaitUsing {
            self.expect_keyword("using", Goal::Div)?;
        }
        let node = self.declarations(kind, Declarations::Statement, start)?.0;
        self.semicolon()?;
        self.ast.set_range(node, start..self.previous_end);
        Ok(node)
    }

    /// Die Liste nach `var`, `let`, `const` oder `using`, deklariert im
    /// passenden Bereich. Gibt auch zurück, ob es eine einzige Bindung
    /// ohne Initialisierung ist (für `for`-`in` und `for`-`of`) und wo
    /// eine nötige Initialisierung fehlt.
    fn declarations(
        &mut self,
        kind: DeclarationKind,
        place: Declarations,
        start: usize,
    ) -> Result<(NodeId, Option<usize>)> {
        let in_ = place == Declarations::Statement;
        let mark = self.scratch.len();
        let mut missing: Option<usize> = None;
        loop {
            let declarator_start = self.start(Goal::Div)?;
            let mut names: Names = Vec::new();
            let target = if matches!(kind, DeclarationKind::Using | DeclarationKind::AwaitUsing) {
                let (atom, position) = self.binding_identifier_token()?;
                names.push((atom, position));
                self.node(Node::BindingIdentifier(atom), position)
            } else {
                self.binding_target(&mut names)?
            };
            self.declare_names(kind, &names)?;
            let init = if self.eat(Punctuator::Assign, Goal::Div)? {
                Some(self.assignment(in_, None)?)
            } else {
                let required = kind != DeclarationKind::Var && kind != DeclarationKind::Let
                    || !matches!(self.ast.node(target), Node::BindingIdentifier(_));
                if required && missing.is_none() {
                    missing = Some(self.previous_end);
                }
                None
            };
            let declarator = self.node(Node::Declarator { target, init }, declarator_start);
            self.scratch.push(declarator);
            if !self.eat(Punctuator::Comma, Goal::Div)? {
                break;
            }
        }
        if place == Declarations::Statement
            && let Some(position) = missing
        {
            return Err(self.error_at(ErrorKind::MissingInitializer, position));
        }
        let declarators = self.finish_list(mark);
        let node = self.node(Node::Variables { kind, declarators }, start);
        Ok((node, missing))
    }

    fn declare_names(&mut self, kind: DeclarationKind, names: &Names) -> Result<()> {
        for &(name, position) in names {
            if kind == DeclarationKind::Var {
                self.declare_var(name, position)?;
            } else {
                if self.atom_is(name, "let") {
                    return Err(self.error_at(ErrorKind::LetBinding, position));
                }
                self.declare_lexical(name, Binding::Lexical, position)?;
            }
        }
        Ok(())
    }

    fn if_statement(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        self.expect(Punctuator::LeftParenthesis, Goal::Div)?;
        let test = self.expression(true)?;
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        let consequent = self.if_clause()?;
        let alternate = if self.eat_keyword("else", Goal::RegExp)? {
            Some(self.if_clause()?)
        } else {
            None
        };
        Ok(self.node(
            Node::If {
                test,
                consequent,
                alternate,
            },
            start,
        ))
    }

    /// Ein Zweig von `if`; außerhalb des strikten Modus auch eine
    /// Funktionsdeklaration wie in einem eigenen Block (Anhang B.3.3).
    fn if_clause(&mut self) -> Result<NodeId> {
        if !self.strict() && self.at_keyword("function", Goal::RegExp)? {
            let start = self.start(Goal::RegExp)?;
            self.push_scope(ScopeKind::Block);
            let declaration = self.plain_function_declaration(start);
            self.pop_scope();
            let declaration = declaration?;
            let mark = self.scratch.len();
            self.scratch.push(declaration);
            let list = self.finish_list(mark);
            return Ok(self.node(Node::Block(list), start));
        }
        self.body()
    }

    fn while_statement(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        self.expect(Punctuator::LeftParenthesis, Goal::Div)?;
        let test = self.expression(true)?;
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        let body = self.body()?;
        Ok(self.node(Node::While { test, body }, start))
    }

    fn do_while_statement(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let body = self.body()?;
        self.expect_keyword("while", Goal::RegExp)?;
        self.expect(Punctuator::LeftParenthesis, Goal::Div)?;
        let test = self.expression(true)?;
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        // Das Semikolon nach `do … while (…)` darf immer fehlen
        // (Abschnitt 12.10.1).
        self.eat(Punctuator::Semicolon, Goal::RegExp)?;
        Ok(self.node(Node::DoWhile { body, test }, start))
    }

    /// `for`, `for`-`in`, `for`-`of` und `for await`-`of`.
    fn for_statement(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let is_await = self.at_keyword("await", Goal::Div)?;
        if is_await {
            let context = self.context();
            if !context.await_ || context.static_block {
                return Err(self.error(ErrorKind::InvalidYieldOrAwait));
            }
            self.advance();
        }
        self.expect(Punctuator::LeftParenthesis, Goal::Div)?;
        self.push_scope(ScopeKind::For);
        let result = match self.for_head(is_await) {
            Ok(ForHead::Classic(init)) => self.for_classic(start, init),
            Ok(ForHead::InOf { left, of }) => self.for_in_of(start, left, of, is_await),
            Err(error) => Err(error),
        };
        self.pop_scope();
        result
    }

    fn for_head(&mut self, is_await: bool) -> Result<ForHead> {
        let head_start = self.start(Goal::RegExp)?;
        let kind = match self.name(Goal::RegExp)? {
            Some(name) if !name.escaped => match name.value.as_str() {
                "var" => Some(DeclarationKind::Var),
                "const" => Some(DeclarationKind::Const),
                "let" if self.let_declaration_follows()? => Some(DeclarationKind::Let),
                "using" if self.using_declaration_follows()? && !self.using_of_follows()? => {
                    Some(DeclarationKind::Using)
                }
                "await" if self.await_operator() && self.await_using_follows()? => {
                    Some(DeclarationKind::AwaitUsing)
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(kind) = kind {
            self.advance();
            if kind == DeclarationKind::AwaitUsing {
                self.expect_keyword("using", Goal::Div)?;
            }
            let (declaration, missing) =
                self.declarations(kind, Declarations::ForHead, head_start)?;
            let Node::Variables { declarators, .. } = *self.ast.node(declaration) else {
                return Err(self.error(ErrorKind::UnexpectedToken));
            };
            let single = declarators.len() == 1;
            let of = self.at_keyword("of", Goal::Div)?;
            let in_ = self.at_keyword("in", Goal::Div)?;
            if of || in_ {
                let declarator = self.ast.list(declarators).first().copied();
                let initialized = declarator.is_some_and(|declarator| {
                    matches!(
                        self.ast.node(declarator),
                        Node::Declarator { init: Some(_), .. }
                    )
                });
                let using = matches!(kind, DeclarationKind::Using | DeclarationKind::AwaitUsing);
                // `for (var x = 1 in y)` außerhalb des strikten Modus
                // (Anhang B.3.5).
                let legacy_initializer = in_
                    && kind == DeclarationKind::Var
                    && !self.strict()
                    && declarator.is_some_and(|declarator| {
                        matches!(
                            self.ast.node(declarator),
                            Node::Declarator { target, .. }
                                if matches!(self.ast.node(*target), Node::BindingIdentifier(_))
                        )
                    });
                if !single
                    || (initialized && !legacy_initializer)
                    || (in_ && using)
                    || (in_ && is_await)
                {
                    return Err(self.error(ErrorKind::InvalidForInOf));
                }
                return Ok(ForHead::InOf {
                    left: declaration,
                    of,
                });
            }
            if is_await {
                return Err(self.unexpected(Goal::Div));
            }
            if let Some(position) = missing {
                return Err(self.error_at(ErrorKind::MissingInitializer, position));
            }
            return Ok(ForHead::Classic(Some(declaration)));
        }
        if self.at(Punctuator::Semicolon, Goal::RegExp)? {
            if is_await {
                return Err(self.unexpected(Goal::RegExp));
            }
            return Ok(ForHead::Classic(None));
        }
        let starts_with_let = self.at_keyword("let", Goal::RegExp)?;
        let starts_with_async = self.at_keyword("async", Goal::RegExp)?;
        let mut cover = Cover::default();
        let init = self.expression_with_cover(false, &mut cover)?;
        let of = self.at_keyword("of", Goal::Div)?;
        let in_ = !of && self.at_keyword("in", Goal::Div)?;
        if !of && !in_ {
            if is_await {
                return Err(self.unexpected(Goal::Div));
            }
            cover.check()?;
            let init = self.node(Node::Expression(init), head_start);
            return Ok(ForHead::Classic(Some(init)));
        }
        // `for (async of …)` ist verboten, `for await (async of …)` nicht.
        let async_of = starts_with_async && !is_await && self.is_plain_async(init);
        if of && (starts_with_let || async_of) {
            return Err(self.error_at(ErrorKind::InvalidForInOf, head_start));
        }
        if !self.is_left_hand_side(init) {
            return Err(self.error_at(ErrorKind::InvalidAssignmentTarget, head_start));
        }
        if matches!(self.ast.node(init), Node::Object(_) | Node::Array(_)) {
            self.assignment_pattern(init)?;
        } else {
            cover.check()?;
            if self.target_type(init) == super::expression::TargetType::Invalid {
                return Err(self.error_at(ErrorKind::InvalidAssignmentTarget, head_start));
            }
        }
        Ok(ForHead::InOf { left: init, of })
    }

    /// Ob `using of` ohne `=` danach folgt: Dann ist `using` das Ziel von
    /// `for`-`of`, denn dort beginnt keine Deklaration mit `using of`.
    /// Mit `=` ist es eine Deklaration in einem klassischen `for`.
    fn using_of_follows(&mut self) -> Result<bool> {
        self.token(Goal::RegExp)?;
        let Some(lookahead) = &self.current else {
            return Ok(false);
        };
        let mut lexer = lookahead.after.clone();
        let Ok(second) = lexer.next_token(Goal::Div) else {
            return Ok(false);
        };
        if !matches!(&second.kind, TokenKind::Name(name) if !name.escaped && name.value == "of") {
            return Ok(false);
        }
        Ok(lexer
            .next_token(Goal::RegExp)
            .is_ok_and(|third| third.kind != TokenKind::Punctuator(Punctuator::Assign)))
    }

    /// Ob der Ausdruck nur der Bezeichner `async` ohne Escapes ist.
    fn is_plain_async(&self, node: NodeId) -> bool {
        matches!(self.ast.node(node), Node::Identifier(name) if self.atom_is(*name, "async"))
            && self.ast.range(node).len() == "async".len()
    }

    /// Ob ein Ausdruck ein *LeftHandSideExpression* ist.
    fn is_left_hand_side(&self, node: NodeId) -> bool {
        !matches!(
            self.ast.node(node),
            Node::Sequence(_)
                | Node::Unary { .. }
                | Node::Update { .. }
                | Node::Binary { .. }
                | Node::Conditional { .. }
                | Node::Assign { .. }
                | Node::Yield { .. }
                | Node::Await(_)
                | Node::PrivateName(_)
        ) && !matches!(self.ast.node(node), Node::Function(function)
            if self.ast.function(*function).is_some_and(|function| function.kind == crate::ast::FunctionKind::Arrow))
    }

    fn for_in_of(
        &mut self,
        start: usize,
        left: NodeId,
        of: bool,
        is_await: bool,
    ) -> Result<NodeId> {
        if is_await && !of {
            return Err(self.unexpected(Goal::Div));
        }
        self.advance();
        let right = if of {
            self.assignment(true, None)?
        } else {
            self.expression(true)?
        };
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        let body = self.body()?;
        Ok(self.node(
            if of {
                Node::ForOf {
                    left,
                    right,
                    body,
                    is_await,
                }
            } else {
                Node::ForIn { left, right, body }
            },
            start,
        ))
    }

    fn for_classic(&mut self, start: usize, init: Option<NodeId>) -> Result<NodeId> {
        self.expect(Punctuator::Semicolon, Goal::Div)?;
        let test = if self.at(Punctuator::Semicolon, Goal::RegExp)? {
            None
        } else {
            Some(self.expression(true)?)
        };
        self.expect(Punctuator::Semicolon, Goal::Div)?;
        let update = if self.at(Punctuator::RightParenthesis, Goal::RegExp)? {
            None
        } else {
            Some(self.expression(true)?)
        };
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        let body = self.body()?;
        Ok(self.node(
            Node::For {
                init,
                test,
                update,
                body,
            },
            start,
        ))
    }

    /// `break` und `continue`.
    fn jump(&mut self, is_break: bool) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let token = self.token(Goal::RegExp)?;
        let label = match &token.kind {
            TokenKind::Name(name) if !token.newline_before => {
                let name = name.clone();
                let position = self.start(Goal::RegExp)?;
                let label = self.label_identifier(&name, position)?;
                self.advance();
                let target = self
                    .function
                    .labels
                    .iter()
                    .rev()
                    .find(|existing| existing.name == label);
                match target {
                    None => return Err(self.error_at(ErrorKind::UndefinedLabel, position)),
                    Some(target) if !is_break && !target.iteration => {
                        return Err(self.error_at(ErrorKind::InvalidJump, position));
                    }
                    Some(_) => {}
                }
                Some(label)
            }
            _ => {
                let allowed =
                    self.function.iterations > 0 || (is_break && self.function.switches > 0);
                if !allowed {
                    return Err(self.error_at(ErrorKind::InvalidJump, start));
                }
                None
            }
        };
        self.semicolon()?;
        Ok(self.node(
            if is_break {
                Node::Break(label)
            } else {
                Node::Continue(label)
            },
            start,
        ))
    }

    fn return_statement(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        if !self.context().return_ {
            return Err(self.error(ErrorKind::InvalidReturn));
        }
        self.advance();
        let token = self.token(Goal::RegExp)?;
        let argument = if token.newline_before
            || matches!(
                token.kind,
                TokenKind::End
                    | TokenKind::Punctuator(Punctuator::Semicolon | Punctuator::RightBrace)
            ) {
            None
        } else {
            Some(self.expression(true)?)
        };
        self.semicolon()?;
        Ok(self.node(Node::Return(argument), start))
    }

    fn with_statement(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        if self.strict() {
            return Err(self.error(ErrorKind::StrictWith));
        }
        self.advance();
        self.expect(Punctuator::LeftParenthesis, Goal::Div)?;
        let object = self.expression(true)?;
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        let body = self.body()?;
        Ok(self.node(Node::With { object, body }, start))
    }

    fn switch_statement(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        self.expect(Punctuator::LeftParenthesis, Goal::Div)?;
        let discriminant = self.expression(true)?;
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        self.expect(Punctuator::LeftBrace, Goal::Div)?;
        self.push_scope(ScopeKind::Switch);
        self.function.switches = self.function.switches.saturating_add(1);
        let pending = self.function.pending_labels;
        self.function.pending_labels = self.function.labels.len();
        let result = self.case_block();
        self.function.pending_labels = pending;
        self.function.switches = self.function.switches.saturating_sub(1);
        self.pop_scope();
        let cases = result?;
        Ok(self.node(
            Node::Switch {
                discriminant,
                cases,
            },
            start,
        ))
    }

    fn case_block(&mut self) -> Result<crate::ast::List> {
        let mark = self.scratch.len();
        let mut default = false;
        while !self.at(Punctuator::RightBrace, Goal::RegExp)? {
            let start = self.start(Goal::RegExp)?;
            let test = if self.eat_keyword("case", Goal::RegExp)? {
                Some(self.expression(true)?)
            } else if self.at_keyword("default", Goal::RegExp)? {
                if default {
                    return Err(self.error(ErrorKind::UnexpectedToken));
                }
                default = true;
                self.advance();
                None
            } else {
                return Err(self.unexpected(Goal::RegExp));
            };
            self.expect(Punctuator::Colon, Goal::Div)?;
            let body_mark = self.scratch.len();
            loop {
                if self.at(Punctuator::RightBrace, Goal::RegExp)?
                    || self.at_keyword("case", Goal::RegExp)?
                    || self.at_keyword("default", Goal::RegExp)?
                {
                    break;
                }
                let item = self.statement_list_item()?;
                self.scratch.push(item);
            }
            let body = self.finish_list(body_mark);
            let case = self.node(Node::Case { test, body }, start);
            self.scratch.push(case);
        }
        self.expect(Punctuator::RightBrace, Goal::RegExp)?;
        Ok(self.finish_list(mark))
    }

    fn throw_statement(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        if self.newline_before(Goal::RegExp)? {
            return Err(self.error(ErrorKind::UnexpectedToken));
        }
        let argument = self.expression(true)?;
        self.semicolon()?;
        Ok(self.node(Node::Throw(argument), start))
    }

    fn try_statement(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let block = self.block(ScopeKind::Block)?;
        let mut handler = None;
        if self.at_keyword("catch", Goal::RegExp)? {
            let catch_start = self.start(Goal::RegExp)?;
            self.advance();
            if self.eat(Punctuator::LeftParenthesis, Goal::Div)? {
                let mut names: Names = Vec::new();
                let simple = matches!(self.token(Goal::Div)?.kind, TokenKind::Name(_));
                self.push_scope(ScopeKind::Catch { simple });
                let result = self.catch_rest(&mut names);
                self.pop_scope();
                let (parameter, body) = result?;
                handler = Some(self.node(
                    Node::Catch {
                        parameter: Some(parameter),
                        body,
                    },
                    catch_start,
                ));
            } else {
                let body = self.block(ScopeKind::Block)?;
                handler = Some(self.node(
                    Node::Catch {
                        parameter: None,
                        body,
                    },
                    catch_start,
                ));
            }
        }
        let finalizer = if self.eat_keyword("finally", Goal::RegExp)? {
            Some(self.block(ScopeKind::Block)?)
        } else {
            None
        };
        if handler.is_none() && finalizer.is_none() {
            return Err(self.unexpected(Goal::RegExp));
        }
        Ok(self.node(
            Node::Try {
                block,
                handler,
                finalizer,
            },
            start,
        ))
    }

    /// Der Parameter von `catch` und der Block danach.
    fn catch_rest(&mut self, names: &mut Names) -> Result<(NodeId, NodeId)> {
        let parameter = self.binding_target(names)?;
        for &(name, position) in names.iter() {
            self.declare_lexical(name, Binding::CatchParameter, position)?;
        }
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        let body = self.block(ScopeKind::CatchBody)?;
        Ok((parameter, body))
    }
}
