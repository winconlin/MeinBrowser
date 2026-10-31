//! Ausdrücke (Abschnitt 13).

use super::{Cover, ErrorKind, Parser, Result, SyntaxError};
use crate::ast::{
    AssignOperator, Atom, BinaryOperator, List, Node, NodeId, PropertyKind, UnaryOperator,
    UpdateOperator,
};
use crate::lexer::{Goal, Name, Punctuator, TemplatePart, TokenKind};
use crate::regexp;

/// Der Kopf einer Pfeilfunktion, erkannt an `=>` hinter einem Bezeichner,
/// einer Klammer oder einem Aufruf von `async`.
#[derive(Clone, Copy, Debug)]
pub(super) struct ArrowHead {
    /// Der Knoten des Kopfes: [`Node::Identifier`] oder
    /// [`Node::Sequence`] mit den Parametern; er wird zur Funktion.
    pub(super) node: NodeId,
    pub(super) is_async: bool,
    /// Die Zähler aus [`super::FunctionState`] vor dem Kopf.
    pub(super) yield_await: u32,
    pub(super) await_identifiers: u32,
}

/// Wie ein *UnaryExpression* beginnt.
#[derive(Clone, Copy)]
enum UnaryStart {
    Operator(UnaryOperator),
    Update,
    Await,
    Other,
}

/// Was ein Name am Anfang eines *PrimaryExpression* beginnt.
#[derive(Clone, Copy)]
enum NameStart {
    /// `this`, `null`, `true` oder `false`.
    Literal(Node),
    Function,
    Class,
    AsyncFunction,
    AsyncArrow,
    Identifier,
}

/// Was in einer Kette von Zugriffen und Aufrufen folgt.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Link {
    Dot,
    Optional,
    Bracket,
    Call,
    Template,
    End,
}

/// Was die Spezifikation *AssignmentTargetType* nennt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TargetType {
    Simple,
    /// Ein Aufruf außerhalb des strikten Modus: erst zur Laufzeit ein
    /// Fehler (Abschnitt 13.15.1, für Webbrowser).
    WebCompat,
    Invalid,
}

/// Der Vorrang zweistelliger Operatoren, höher bindet stärker.
fn binary_precedence(operator: BinaryOperator) -> u8 {
    match operator {
        BinaryOperator::LogicalOr | BinaryOperator::Coalesce => 1,
        BinaryOperator::LogicalAnd => 2,
        BinaryOperator::BitwiseOr => 3,
        BinaryOperator::BitwiseXor => 4,
        BinaryOperator::BitwiseAnd => 5,
        BinaryOperator::Equal
        | BinaryOperator::NotEqual
        | BinaryOperator::StrictEqual
        | BinaryOperator::StrictNotEqual => 6,
        BinaryOperator::Less
        | BinaryOperator::LessEqual
        | BinaryOperator::Greater
        | BinaryOperator::GreaterEqual
        | BinaryOperator::In
        | BinaryOperator::Instanceof => 7,
        BinaryOperator::ShiftLeft
        | BinaryOperator::ShiftRight
        | BinaryOperator::UnsignedShiftRight => 8,
        BinaryOperator::Add | BinaryOperator::Subtract => 9,
        BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Remainder => 10,
        BinaryOperator::Exponent => 11,
    }
}

const RELATIONAL: u8 = 7;

fn binary_operator(punctuator: Punctuator) -> Option<BinaryOperator> {
    Some(match punctuator {
        Punctuator::BarBar => BinaryOperator::LogicalOr,
        Punctuator::QuestionQuestion => BinaryOperator::Coalesce,
        Punctuator::AmpersandAmpersand => BinaryOperator::LogicalAnd,
        Punctuator::Bar => BinaryOperator::BitwiseOr,
        Punctuator::Caret => BinaryOperator::BitwiseXor,
        Punctuator::Ampersand => BinaryOperator::BitwiseAnd,
        Punctuator::Equal => BinaryOperator::Equal,
        Punctuator::NotEqual => BinaryOperator::NotEqual,
        Punctuator::StrictEqual => BinaryOperator::StrictEqual,
        Punctuator::StrictNotEqual => BinaryOperator::StrictNotEqual,
        Punctuator::Less => BinaryOperator::Less,
        Punctuator::LessEqual => BinaryOperator::LessEqual,
        Punctuator::Greater => BinaryOperator::Greater,
        Punctuator::GreaterEqual => BinaryOperator::GreaterEqual,
        Punctuator::ShiftLeft => BinaryOperator::ShiftLeft,
        Punctuator::ShiftRight => BinaryOperator::ShiftRight,
        Punctuator::UnsignedShiftRight => BinaryOperator::UnsignedShiftRight,
        Punctuator::Plus => BinaryOperator::Add,
        Punctuator::Minus => BinaryOperator::Subtract,
        Punctuator::Star => BinaryOperator::Multiply,
        Punctuator::Slash => BinaryOperator::Divide,
        Punctuator::Percent => BinaryOperator::Remainder,
        Punctuator::StarStar => BinaryOperator::Exponent,
        _ => return None,
    })
}

fn assignment_operator(punctuator: Punctuator) -> Option<AssignOperator> {
    Some(match punctuator {
        Punctuator::Assign => AssignOperator::Assign,
        Punctuator::PlusAssign => AssignOperator::Add,
        Punctuator::MinusAssign => AssignOperator::Subtract,
        Punctuator::StarAssign => AssignOperator::Multiply,
        Punctuator::SlashAssign => AssignOperator::Divide,
        Punctuator::PercentAssign => AssignOperator::Remainder,
        Punctuator::StarStarAssign => AssignOperator::Exponent,
        Punctuator::ShiftLeftAssign => AssignOperator::ShiftLeft,
        Punctuator::ShiftRightAssign => AssignOperator::ShiftRight,
        Punctuator::UnsignedShiftRightAssign => AssignOperator::UnsignedShiftRight,
        Punctuator::BarAssign => AssignOperator::BitwiseOr,
        Punctuator::CaretAssign => AssignOperator::BitwiseXor,
        Punctuator::AmpersandAssign => AssignOperator::BitwiseAnd,
        Punctuator::AmpersandAmpersandAssign => AssignOperator::LogicalAnd,
        Punctuator::BarBarAssign => AssignOperator::LogicalOr,
        Punctuator::QuestionQuestionAssign => AssignOperator::Coalesce,
        _ => return None,
    })
}

impl Parser<'_> {
    /// *Expression*: Ausdrücke mit Komma.
    pub(super) fn expression(&mut self, in_: bool) -> Result<NodeId> {
        let mut cover = Cover::default();
        let expression = self.expression_with_cover(in_, &mut cover)?;
        cover.check()?;
        Ok(expression)
    }

    /// Wie [`Parser::expression`]; ist der Ausdruck ein einzelnes Objekt-
    /// oder Array-Literal, landen dessen Cover-Fehler in `cover`.
    pub(super) fn expression_with_cover(&mut self, in_: bool, cover: &mut Cover) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        let first = self.assignment(in_, Some(cover))?;
        if !self.at(Punctuator::Comma, Goal::Div)? {
            return Ok(first);
        }
        cover.check()?;
        let mark = self.scratch.len();
        self.scratch.push(first);
        while self.eat(Punctuator::Comma, Goal::Div)? {
            let next = self.assignment(in_, None)?;
            self.scratch.push(next);
        }
        let list = self.finish_list(mark);
        Ok(self.node(Node::Sequence(list), start))
    }

    /// *AssignmentExpression*. Ein Objekt- oder Array-Literal, das kein
    /// Ziel von `=` ist, gibt seine Cover-Fehler an `outer` weiter, wenn es
    /// das gibt, denn es kann noch Teil eines Musters werden.
    pub(super) fn assignment(&mut self, in_: bool, outer: Option<&mut Cover>) -> Result<NodeId> {
        self.enter()?;
        let result = self.assignment_inner(in_, outer);
        self.leave();
        result
    }

    fn assignment_inner(&mut self, in_: bool, outer: Option<&mut Cover>) -> Result<NodeId> {
        if self.context().yield_ && self.at_keyword("yield", Goal::RegExp)? {
            return self.yield_expression(in_);
        }
        let start = self.start(Goal::RegExp)?;
        let mut cover = Cover::default();
        let expression = self.conditional(in_, &mut cover)?;
        self.assignment_rest(start, expression, cover, in_, outer)
    }

    /// Was nach dem *ConditionalExpression* am Anfang folgt: `=>`, ein
    /// Zuweisungsoperator oder nichts.
    fn assignment_rest(
        &mut self,
        start: usize,
        expression: NodeId,
        cover: Cover,
        in_: bool,
        outer: Option<&mut Cover>,
    ) -> Result<NodeId> {
        let Some(punctuator) = self.punctuator(Goal::Div)? else {
            return self.pass_cover(expression, cover, outer);
        };
        if punctuator == Punctuator::Arrow {
            return match self.arrow_head.take() {
                Some(head) if head.node == expression => self.arrow_function(head, in_, start),
                _ => Err(self.error(ErrorKind::UnexpectedToken)),
            };
        }
        let Some(operator) = assignment_operator(punctuator) else {
            return self.pass_cover(expression, cover, outer);
        };
        let position = self.start(Goal::Div)?;
        if operator == AssignOperator::Assign
            && matches!(self.ast.node(expression), Node::Object(_) | Node::Array(_))
        {
            self.assignment_pattern(expression)?;
        } else {
            cover.check()?;
            let target = self.target_type(expression);
            let valid = match operator {
                AssignOperator::LogicalAnd
                | AssignOperator::LogicalOr
                | AssignOperator::Coalesce => target == TargetType::Simple,
                _ => target != TargetType::Invalid,
            };
            if !valid {
                return Err(self.error_at(ErrorKind::InvalidAssignmentTarget, position));
            }
        }
        self.advance();
        let value = self.assignment(in_, None)?;
        Ok(self.node(
            Node::Assign {
                operator,
                target: expression,
                value,
            },
            start,
        ))
    }

    /// Gibt die Cover-Fehler eines Ausdrucks, der keine Zuweisung ist,
    /// weiter oder meldet sie.
    fn pass_cover(
        &mut self,
        expression: NodeId,
        cover: Cover,
        outer: Option<&mut Cover>,
    ) -> Result<NodeId> {
        match outer {
            Some(outer)
                if matches!(self.ast.node(expression), Node::Object(_) | Node::Array(_)) =>
            {
                if let Some(error) = cover.error {
                    outer.record(error);
                }
            }
            _ => cover.check()?,
        }
        Ok(expression)
    }

    /// *YieldExpression*
    fn yield_expression(&mut self, in_: bool) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        self.function.yield_await = self.function.yield_await.saturating_add(1);
        let token = self.token(Goal::RegExp)?;
        let absent = token.newline_before
            || matches!(
                token.kind,
                TokenKind::End
                    | TokenKind::Punctuator(
                        Punctuator::RightParenthesis
                            | Punctuator::RightBracket
                            | Punctuator::RightBrace
                            | Punctuator::Comma
                            | Punctuator::Semicolon
                            | Punctuator::Colon
                    )
            );
        if absent {
            return Ok(self.node(
                Node::Yield {
                    argument: None,
                    delegate: false,
                },
                start,
            ));
        }
        let delegate = self.eat(Punctuator::Star, Goal::RegExp)?;
        let argument = self.assignment(in_, None)?;
        Ok(self.node(
            Node::Yield {
                argument: Some(argument),
                delegate,
            },
            start,
        ))
    }

    /// *ConditionalExpression*
    fn conditional(&mut self, in_: bool, cover: &mut Cover) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        let test = self.binary(in_, cover)?;
        if !self.eat(Punctuator::Question, Goal::Div)? {
            return Ok(test);
        }
        let consequent = self.assignment(true, None)?;
        self.expect(Punctuator::Colon, Goal::Div)?;
        let alternate = self.assignment(in_, None)?;
        Ok(self.node(
            Node::Conditional {
                test,
                consequent,
                alternate,
            },
            start,
        ))
    }

    /// Der nächste zweistellige Operator, `in` nur mit `in_`.
    fn next_binary_operator(&mut self, in_: bool) -> Result<Option<BinaryOperator>> {
        Ok(match &self.token(Goal::Div)?.kind {
            TokenKind::Punctuator(punctuator) => binary_operator(*punctuator),
            TokenKind::Name(name) if !name.escaped && name.value == "instanceof" => {
                Some(BinaryOperator::Instanceof)
            }
            TokenKind::Name(name) if in_ && !name.escaped && name.value == "in" => {
                Some(BinaryOperator::In)
            }
            _ => None,
        })
    }

    /// *ShortCircuitExpression* und alles darunter bis
    /// *ExponentiationExpression*, mit Vorrangstufen und eigenen Stapeln
    /// statt Rekursion.
    fn binary(&mut self, in_: bool, cover: &mut Cover) -> Result<NodeId> {
        let first = self.unary(cover)?;
        let Some(operator) = self.next_binary_operator(in_)? else {
            if matches!(self.ast.node(first), Node::PrivateName(_)) {
                return Err(self.error(ErrorKind::UnexpectedToken));
            }
            return Ok(first);
        };
        self.binary_operations(first, operator, in_, cover)
    }

    /// Ab dem ersten zweistelligen Operator.
    fn binary_operations(
        &mut self,
        first: NodeId,
        mut operator: BinaryOperator,
        in_: bool,
        cover: &mut Cover,
    ) -> Result<NodeId> {
        let mut operands = vec![first];
        let mut operators: Vec<BinaryOperator> = Vec::new();
        loop {
            let precedence = binary_precedence(operator);
            let left = *operands.last().unwrap_or(&first);
            // `#x in y`: Der private Name ist nur links von `in` erlaubt.
            if matches!(self.ast.node(left), Node::PrivateName(_)) {
                let binds = operator == BinaryOperator::In
                    && operators
                        .last()
                        .is_none_or(|top| binary_precedence(*top) < RELATIONAL);
                if !binds {
                    return Err(self.error(ErrorKind::UnexpectedToken));
                }
            }
            if operator == BinaryOperator::Exponent
                && matches!(self.ast.node(left), Node::Unary { .. } | Node::Await(_))
            {
                return Err(self.error(ErrorKind::UnaryBeforeExponent));
            }
            while let Some(top) = operators.last().copied() {
                let top_precedence = binary_precedence(top);
                let reduce = top_precedence > precedence
                    || (top_precedence == precedence && operator != BinaryOperator::Exponent);
                if !reduce {
                    break;
                }
                operators.pop();
                self.reduce_binary(&mut operands, top)?;
            }
            operators.push(operator);
            self.advance();
            let right = self.unary(cover)?;
            operands.push(right);
            match self.next_binary_operator(in_)? {
                Some(next) => operator = next,
                None => break,
            }
        }
        if operands
            .last()
            .is_some_and(|last| matches!(self.ast.node(*last), Node::PrivateName(_)))
        {
            return Err(self.error(ErrorKind::UnexpectedToken));
        }
        while let Some(top) = operators.pop() {
            self.reduce_binary(&mut operands, top)?;
        }
        Ok(operands.pop().unwrap_or(first))
    }

    fn reduce_binary(
        &mut self,
        operands: &mut Vec<NodeId>,
        operator: BinaryOperator,
    ) -> Result<()> {
        let (Some(right), Some(left)) = (operands.pop(), operands.pop()) else {
            return Err(self.error(ErrorKind::UnexpectedToken));
        };
        // `??` nicht mit `&&` oder `||` ohne Klammern (Abschnitt 13.13).
        let logical = |node: &Node| {
            matches!(
                node,
                Node::Binary {
                    operator: BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr,
                    ..
                }
            )
        };
        let coalesce = |node: &Node| {
            matches!(
                node,
                Node::Binary {
                    operator: BinaryOperator::Coalesce,
                    ..
                }
            )
        };
        let mixed = match operator {
            BinaryOperator::Coalesce => {
                logical(self.ast.node(left)) || logical(self.ast.node(right))
            }
            BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr => {
                coalesce(self.ast.node(left)) || coalesce(self.ast.node(right))
            }
            _ => false,
        };
        if mixed {
            return Err(self.error_at(ErrorKind::MixedCoalesce, self.ast.range(right).start));
        }
        let start = self.ast.range(left).start;
        let end = self.ast.range(right).end;
        let node = self.node_at(
            Node::Binary {
                operator,
                left,
                right,
            },
            start,
            end,
        );
        operands.push(node);
        Ok(())
    }

    /// *UnaryExpression*
    fn unary(&mut self, cover: &mut Cover) -> Result<NodeId> {
        self.enter()?;
        let result = self.unary_inner(cover);
        self.leave();
        result
    }

    fn unary_inner(&mut self, cover: &mut Cover) -> Result<NodeId> {
        match self.unary_start()? {
            UnaryStart::Operator(operator) => self.unary_operator(operator),
            UnaryStart::Update => self.prefix_update(),
            UnaryStart::Await => self.await_expression(),
            UnaryStart::Other => self.postfix(cover),
        }
    }

    fn unary_start(&mut self) -> Result<UnaryStart> {
        let await_operator = self.context().await_;
        Ok(match &self.token(Goal::RegExp)?.kind {
            TokenKind::Punctuator(Punctuator::Bang) => UnaryStart::Operator(UnaryOperator::Not),
            TokenKind::Punctuator(Punctuator::Tilde) => {
                UnaryStart::Operator(UnaryOperator::BitwiseNot)
            }
            TokenKind::Punctuator(Punctuator::Plus) => UnaryStart::Operator(UnaryOperator::Plus),
            TokenKind::Punctuator(Punctuator::Minus) => UnaryStart::Operator(UnaryOperator::Minus),
            TokenKind::Punctuator(Punctuator::PlusPlus | Punctuator::MinusMinus) => {
                UnaryStart::Update
            }
            TokenKind::Name(name) if !name.escaped => match name.value.as_str() {
                "delete" => UnaryStart::Operator(UnaryOperator::Delete),
                "void" => UnaryStart::Operator(UnaryOperator::Void),
                "typeof" => UnaryStart::Operator(UnaryOperator::Typeof),
                "await" if await_operator => UnaryStart::Await,
                _ => UnaryStart::Other,
            },
            _ => UnaryStart::Other,
        })
    }

    /// Ein einstelliger Operator mit seinem Operanden.
    fn unary_operator(&mut self, operator: UnaryOperator) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let mut inner = Cover::default();
        let argument = self.unary(&mut inner)?;
        inner.check()?;
        if matches!(self.ast.node(argument), Node::PrivateName(_)) {
            return Err(self.error_at(ErrorKind::UnexpectedToken, self.ast.range(argument).start));
        }
        if operator == UnaryOperator::Delete {
            self.check_delete(argument)?;
        }
        Ok(self.node(Node::Unary { operator, argument }, start))
    }

    /// Die frühen Fehler zu `delete` (Abschnitt 13.5.1.1), auch durch
    /// Klammern hindurch.
    fn check_delete(&self, argument: NodeId) -> Result<()> {
        let mut node = argument;
        loop {
            match *self.ast.node(node) {
                Node::Paren(inner) => node = inner,
                Node::Identifier(_) if self.strict() => break,
                Node::PrivateMember { .. } => break,
                Node::Chain(inner)
                    if matches!(self.ast.node(inner), Node::PrivateMember { .. }) =>
                {
                    break;
                }
                _ => return Ok(()),
            }
        }
        Err(self.error_at(ErrorKind::InvalidDelete, self.ast.range(argument).start))
    }

    /// `await` mit seinem Operanden.
    fn await_expression(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        if self.context().static_block {
            return Err(self.error(ErrorKind::InvalidYieldOrAwait));
        }
        self.advance();
        self.function.yield_await = self.function.yield_await.saturating_add(1);
        let mut inner = Cover::default();
        let argument = self.unary(&mut inner)?;
        inner.check()?;
        if matches!(self.ast.node(argument), Node::PrivateName(_)) {
            return Err(self.error_at(ErrorKind::UnexpectedToken, self.ast.range(argument).start));
        }
        Ok(self.node(Node::Await(argument), start))
    }

    /// `++` und `--` vor dem Operanden.
    fn prefix_update(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        let operator = if self.at(Punctuator::PlusPlus, Goal::RegExp)? {
            UpdateOperator::Increment
        } else {
            UpdateOperator::Decrement
        };
        self.advance();
        let mut inner = Cover::default();
        let argument = self.unary(&mut inner)?;
        inner.check()?;
        if self.target_type(argument) == TargetType::Invalid {
            return Err(self.error_at(
                ErrorKind::InvalidAssignmentTarget,
                self.ast.range(argument).start,
            ));
        }
        Ok(self.node(
            Node::Update {
                operator,
                prefix: true,
                argument,
            },
            start,
        ))
    }

    /// *UpdateExpression* mit `++` und `--` hinter dem Operanden.
    fn postfix(&mut self, cover: &mut Cover) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        let argument = self.left_hand_side(cover)?;
        let token = self.token(Goal::Div)?;
        let operator = match token.kind {
            _ if token.newline_before => return Ok(argument),
            TokenKind::Punctuator(Punctuator::PlusPlus) => UpdateOperator::Increment,
            TokenKind::Punctuator(Punctuator::MinusMinus) => UpdateOperator::Decrement,
            _ => return Ok(argument),
        };
        if self.target_type(argument) == TargetType::Invalid {
            return Err(self.error(ErrorKind::InvalidAssignmentTarget));
        }
        self.advance();
        Ok(self.node(
            Node::Update {
                operator,
                prefix: false,
                argument,
            },
            start,
        ))
    }

    /// *AssignmentTargetType* (Abschnitt 13.15.1 und Folgende).
    pub(super) fn target_type(&self, expression: NodeId) -> TargetType {
        let mut node = expression;
        loop {
            match *self.ast.node(node) {
                Node::Paren(inner) => node = inner,
                Node::Identifier(name) => {
                    let invalid = self.strict()
                        && (self.atom_is(name, "eval") || self.atom_is(name, "arguments"));
                    return if invalid {
                        TargetType::Invalid
                    } else {
                        TargetType::Simple
                    };
                }
                Node::Member { .. }
                | Node::ComputedMember { .. }
                | Node::PrivateMember { .. }
                | Node::SuperMember(_)
                | Node::SuperComputedMember(_) => return TargetType::Simple,
                Node::Call { .. } if !self.strict() => return TargetType::WebCompat,
                _ => return TargetType::Invalid,
            }
        }
    }

    /// *LeftHandSideExpression*: `new`, Aufrufe, Zugriffe und optionale
    /// Ketten.
    pub(super) fn left_hand_side(&mut self, cover: &mut Cover) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        let expression = self.left_hand_side_head(cover, start)?;
        if self.arrow_head.is_some_and(|head| head.node == expression) {
            return Ok(expression);
        }
        self.links(expression, start)
    }

    /// Der Anfang vor den Gliedern einer Kette, mit `async(…)`.
    fn left_hand_side_head(&mut self, cover: &mut Cover, start: usize) -> Result<NodeId> {
        let expression = if self.at_keyword("new", Goal::RegExp)? {
            self.new_expression()?
        } else if self.at_keyword("super", Goal::RegExp)? {
            self.super_expression(true)?
        } else if self.at_keyword("import", Goal::RegExp)? {
            self.import_expression(true)?
        } else {
            self.primary(cover)?
        };
        if self.arrow_head.is_some_and(|head| head.node == expression) {
            return Ok(expression);
        }
        // `async(…)` könnte der Kopf einer async-Pfeilfunktion sein.
        if self.is_async_name(expression)
            && self.at(Punctuator::LeftParenthesis, Goal::Div)?
            && !self.newline_before(Goal::Div)?
        {
            return self.async_call(expression, start);
        }
        Ok(expression)
    }

    /// Die Glieder einer Kette nach `expression`.
    fn links(&mut self, mut expression: NodeId, start: usize) -> Result<NodeId> {
        let mut chain = false;
        loop {
            match self.link()? {
                Link::Dot => {
                    self.advance();
                    expression = self.member_after_dot(expression, start, false)?;
                }
                Link::Optional => {
                    self.advance();
                    chain = true;
                    expression = self.optional_link(expression, start)?;
                }
                Link::Bracket => {
                    expression = self.computed_member(expression, start, false)?;
                }
                Link::Call => {
                    let arguments = self.arguments(None)?;
                    expression = self.node(
                        Node::Call {
                            callee: expression,
                            arguments,
                            optional: false,
                        },
                        start,
                    );
                }
                Link::Template => {
                    if chain {
                        return Err(self.error(ErrorKind::InvalidOptionalChain));
                    }
                    let quasi = self.template(true)?;
                    expression = self.node(
                        Node::TaggedTemplate {
                            tag: expression,
                            quasi,
                        },
                        start,
                    );
                }
                Link::End => break,
            }
        }
        if chain {
            expression = self.node(Node::Chain(expression), start);
        }
        Ok(expression)
    }

    /// Was nach einem Operanden einer Kette folgt.
    fn link(&mut self) -> Result<Link> {
        Ok(match &self.token(Goal::Div)?.kind {
            TokenKind::Punctuator(Punctuator::Dot) => Link::Dot,
            TokenKind::Punctuator(Punctuator::QuestionDot) => Link::Optional,
            TokenKind::Punctuator(Punctuator::LeftBracket) => Link::Bracket,
            TokenKind::Punctuator(Punctuator::LeftParenthesis) => Link::Call,
            TokenKind::Template(_) => Link::Template,
            _ => Link::End,
        })
    }

    /// Ob der Knoten der Bezeichner `async` ohne Escapes ist.
    fn is_async_name(&self, node: NodeId) -> bool {
        matches!(self.ast.node(node), Node::Identifier(name) if self.atom_is(*name, "async"))
            && self.ast.range(node).len() == "async".len()
    }

    /// `async(…)`: ein Aufruf oder, wenn `=>` folgt, der Kopf einer
    /// async-Pfeilfunktion (*CoverCallExpressionAndAsyncArrowHead*).
    fn async_call(&mut self, callee: NodeId, start: usize) -> Result<NodeId> {
        let yield_await = self.function.yield_await;
        let await_identifiers = self.function.await_identifiers;
        let mut cover = Cover::default();
        let arguments = self.arguments(Some(&mut cover))?;
        let token = self.token(Goal::Div)?;
        if token.kind == TokenKind::Punctuator(Punctuator::Arrow) && !token.newline_before {
            let node = self.node(Node::Sequence(arguments), start);
            self.arrow_head = Some(ArrowHead {
                node,
                is_async: true,
                yield_await,
                await_identifiers,
            });
            return Ok(node);
        }
        cover.check()?;
        Ok(self.node(
            Node::Call {
                callee,
                arguments,
                optional: false,
            },
            start,
        ))
    }

    /// Der Teil nach `.`: ein Name oder ein privater Name.
    fn member_after_dot(&mut self, object: NodeId, start: usize, optional: bool) -> Result<NodeId> {
        match &self.token(Goal::Div)?.kind {
            TokenKind::Name(name) => {
                let name = name.clone();
                self.advance();
                let property = self.intern_name(&name);
                Ok(self.node(
                    Node::Member {
                        object,
                        property,
                        optional,
                    },
                    start,
                ))
            }
            TokenKind::PrivateName(name) => {
                let name = name.clone();
                let position = self.start(Goal::Div)?;
                self.advance();
                let name = self.intern_name(&name);
                self.use_private_name(name, position)?;
                Ok(self.node(
                    Node::PrivateMember {
                        object,
                        name,
                        optional,
                    },
                    start,
                ))
            }
            _ => Err(self.unexpected(Goal::Div)),
        }
    }

    fn computed_member(&mut self, object: NodeId, start: usize, optional: bool) -> Result<NodeId> {
        self.expect(Punctuator::LeftBracket, Goal::Div)?;
        let property = self.expression(true)?;
        self.expect(Punctuator::RightBracket, Goal::Div)?;
        Ok(self.node(
            Node::ComputedMember {
                object,
                property,
                optional,
            },
            start,
        ))
    }

    /// Das Glied direkt nach `?.`.
    fn optional_link(&mut self, object: NodeId, start: usize) -> Result<NodeId> {
        match self.link()? {
            Link::Call => {
                let arguments = self.arguments(None)?;
                Ok(self.node(
                    Node::Call {
                        callee: object,
                        arguments,
                        optional: true,
                    },
                    start,
                ))
            }
            Link::Bracket => self.computed_member(object, start, true),
            Link::Template => Err(self.error(ErrorKind::InvalidOptionalChain)),
            _ => self.member_after_dot(object, start, true),
        }
    }

    /// *Arguments*. Mit `cover` gehören die Argumente vielleicht zum Kopf
    /// einer async-Pfeilfunktion.
    pub(super) fn arguments(&mut self, mut cover: Option<&mut Cover>) -> Result<List> {
        self.expect(Punctuator::LeftParenthesis, Goal::Div)?;
        let mark = self.scratch.len();
        loop {
            if self.at(Punctuator::RightParenthesis, Goal::RegExp)? {
                break;
            }
            let argument = if self.at(Punctuator::Ellipsis, Goal::RegExp)? {
                let start = self.start(Goal::RegExp)?;
                self.advance();
                let argument = self.assignment(true, cover.as_deref_mut())?;
                let spread = self.node(Node::Spread(argument), start);
                if self.at(Punctuator::Comma, Goal::Div)? {
                    self.spread_with_comma.insert(spread);
                }
                spread
            } else {
                self.assignment(true, cover.as_deref_mut())?
            };
            self.scratch.push(argument);
            if !self.eat(Punctuator::Comma, Goal::Div)? {
                break;
            }
        }
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        Ok(self.finish_list(mark))
    }

    /// `new` mit seinem Ausdruck, oder `new.target`.
    fn new_expression(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        if self.eat(Punctuator::Dot, Goal::Div)? {
            if !self.at_keyword("target", Goal::Div)? {
                return Err(self.unexpected(Goal::Div));
            }
            if !self.context().new_target {
                return Err(self.error(ErrorKind::InvalidNewTarget));
            }
            self.advance();
            return Ok(self.node(Node::NewTarget, start));
        }
        self.enter()?;
        let result = self.new_callee(start);
        self.leave();
        result
    }

    fn new_callee(&mut self, start: usize) -> Result<NodeId> {
        let callee_start = self.start(Goal::RegExp)?;
        let mut callee = if self.at_keyword("new", Goal::RegExp)? {
            self.new_expression()?
        } else if self.at_keyword("super", Goal::RegExp)? {
            self.super_expression(false)?
        } else if self.at_keyword("import", Goal::RegExp)? {
            self.import_expression(false)?
        } else {
            let mut cover = Cover::default();
            let callee = self.primary(&mut cover)?;
            cover.check()?;
            callee
        };
        loop {
            match self.link()? {
                Link::Dot => {
                    self.advance();
                    callee = self.member_after_dot(callee, callee_start, false)?;
                }
                Link::Bracket => {
                    callee = self.computed_member(callee, callee_start, false)?;
                }
                Link::Template => {
                    let quasi = self.template(true)?;
                    callee = self.node(Node::TaggedTemplate { tag: callee, quasi }, callee_start);
                }
                Link::Optional => return Err(self.error(ErrorKind::InvalidOptionalChain)),
                Link::Call | Link::End => break,
            }
        }
        let arguments = if self.at(Punctuator::LeftParenthesis, Goal::Div)? {
            self.arguments(None)?
        } else {
            List::default()
        };
        Ok(self.node(Node::New { callee, arguments }, start))
    }

    /// `super` mit `.`, `[` oder, wenn `call`, `(`.
    fn super_expression(&mut self, call: bool) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let context = self.context();
        match self.punctuator(Goal::Div)? {
            Some(Punctuator::LeftParenthesis) if call => {
                if !context.super_call {
                    return Err(self.error_at(ErrorKind::InvalidSuper, start));
                }
                let arguments = self.arguments(None)?;
                Ok(self.node(Node::SuperCall(arguments), start))
            }
            Some(Punctuator::Dot) => {
                if !context.super_property {
                    return Err(self.error_at(ErrorKind::InvalidSuper, start));
                }
                self.advance();
                let Some(name) = self.name(Goal::Div)? else {
                    return Err(self.unexpected(Goal::Div));
                };
                self.advance();
                let property = self.intern_name(&name);
                Ok(self.node(Node::SuperMember(property), start))
            }
            Some(Punctuator::LeftBracket) => {
                if !context.super_property {
                    return Err(self.error_at(ErrorKind::InvalidSuper, start));
                }
                self.advance();
                let property = self.expression(true)?;
                self.expect(Punctuator::RightBracket, Goal::Div)?;
                Ok(self.node(Node::SuperComputedMember(property), start))
            }
            _ => Err(self.error_at(ErrorKind::InvalidSuper, start)),
        }
    }

    /// `import(…)` (nur wenn `call`) und `import.meta`.
    fn import_expression(&mut self, call: bool) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        if self.eat(Punctuator::Dot, Goal::Div)? {
            if !self.at_keyword("meta", Goal::Div)? {
                return Err(self.unexpected(Goal::Div));
            }
            if !self.module {
                return Err(self.error_at(ErrorKind::InvalidImportMeta, start));
            }
            self.advance();
            return Ok(self.node(Node::ImportMeta, start));
        }
        if !call {
            return Err(self.unexpected(Goal::Div));
        }
        self.expect(Punctuator::LeftParenthesis, Goal::Div)?;
        let specifier = self.assignment(true, None)?;
        let mut options = None;
        if self.eat(Punctuator::Comma, Goal::Div)?
            && !self.at(Punctuator::RightParenthesis, Goal::RegExp)?
        {
            options = Some(self.assignment(true, None)?);
            self.eat(Punctuator::Comma, Goal::Div)?;
        }
        self.expect(Punctuator::RightParenthesis, Goal::Div)?;
        Ok(self.node(Node::ImportCall { specifier, options }, start))
    }

    /// *PrimaryExpression*
    fn primary(&mut self, cover: &mut Cover) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        let token = self.token(Goal::RegExp)?;
        match &token.kind {
            TokenKind::Name(_) => self.primary_name(start),
            TokenKind::PrivateName(_) => self.private_name_operand(start),
            TokenKind::Number(_) | TokenKind::BigInt(_) | TokenKind::String(_) => {
                Ok(self.literal_property_name()?.0)
            }
            TokenKind::Template(_) => self.template(false),
            TokenKind::RegExp(_) => self.regexp_literal(start),
            TokenKind::Punctuator(Punctuator::LeftParenthesis) => self.parenthesized(),
            TokenKind::Punctuator(Punctuator::LeftBracket) => self.array_literal(cover),
            TokenKind::Punctuator(Punctuator::LeftBrace) => self.object_literal(cover),
            _ => Err(self.unexpected(Goal::RegExp)),
        }
    }

    /// `#x`, nur vor `in` erlaubt (Abschnitt 13.10).
    fn private_name_operand(&mut self, start: usize) -> Result<NodeId> {
        let TokenKind::PrivateName(name) = &self.token(Goal::RegExp)?.kind else {
            return Err(self.unexpected(Goal::RegExp));
        };
        let name = name.clone();
        self.advance();
        let atom = self.intern_name(&name);
        self.use_private_name(atom, start)?;
        Ok(self.node(Node::PrivateName(atom), start))
    }

    fn regexp_literal(&mut self, start: usize) -> Result<NodeId> {
        let token = self.token(Goal::RegExp)?;
        let TokenKind::RegExp(literal) = &token.kind else {
            return Err(self.unexpected(Goal::RegExp));
        };
        let body_start = token.range.start.saturating_add(1);
        let body = literal.body.clone();
        let flags = literal.flags.clone();
        let Some(parsed_flags) = regexp::Flags::parse(&flags) else {
            return Err(self.error(ErrorKind::InvalidRegExpFlags));
        };
        if let Err(error) = regexp::parse(&body, parsed_flags) {
            return Err(SyntaxError {
                kind: ErrorKind::RegExp(error.kind),
                position: body_start.saturating_add(error.position),
            });
        }
        self.advance();
        let body = self.intern(&body);
        let flags = self.intern(&flags);
        Ok(self.node(Node::RegExp { body, flags }, start))
    }

    fn primary_name(&mut self, start: usize) -> Result<NodeId> {
        match self.name_start()? {
            NameStart::Literal(node) => {
                self.advance();
                Ok(self.node(node, start))
            }
            NameStart::Function => self.function_expression(start, false),
            NameStart::Class => self.class_expression(),
            NameStart::AsyncFunction => {
                self.advance();
                self.function_expression(start, true)
            }
            NameStart::AsyncArrow => self.async_arrow_identifier(start),
            NameStart::Identifier => self.identifier_expression(start),
        }
    }

    fn name_start(&mut self) -> Result<NameStart> {
        let Some(name) = self.name(Goal::RegExp)? else {
            return Err(self.unexpected(Goal::RegExp));
        };
        if name.escaped {
            return Ok(NameStart::Identifier);
        }
        Ok(match name.value.as_str() {
            "this" => NameStart::Literal(Node::This),
            "null" => NameStart::Literal(Node::Null),
            "true" => NameStart::Literal(Node::Boolean(true)),
            "false" => NameStart::Literal(Node::Boolean(false)),
            "function" => NameStart::Function,
            "class" => NameStart::Class,
            "async" => {
                if let Some(next) = self.peek(Goal::RegExp, Goal::Div)?
                    && !next.newline_before
                    && let TokenKind::Name(next_name) = &next.kind
                    && !next_name.escaped
                    && next_name.value == "function"
                {
                    NameStart::AsyncFunction
                } else if self.async_arrow_follows()? {
                    NameStart::AsyncArrow
                } else {
                    NameStart::Identifier
                }
            }
            _ => NameStart::Identifier,
        })
    }

    /// Ein Bezeichner; folgt `=>`, ist er der Kopf einer Pfeilfunktion.
    fn identifier_expression(&mut self, start: usize) -> Result<NodeId> {
        let Some(name) = self.name(Goal::RegExp)? else {
            return Err(self.unexpected(Goal::RegExp));
        };
        let yield_await = self.function.yield_await;
        let await_identifiers = self.function.await_identifiers;
        let atom = self.identifier_reference(&name, start)?;
        self.advance();
        let node = self.node(Node::Identifier(atom), start);
        let token = self.token(Goal::Div)?;
        if token.kind == TokenKind::Punctuator(Punctuator::Arrow) && !token.newline_before {
            self.arrow_head = Some(ArrowHead {
                node,
                is_async: false,
                yield_await,
                await_identifiers,
            });
        }
        Ok(node)
    }

    /// Ob nach `async` ein Bezeichner und `=>` folgen, alles auf einer
    /// Zeile. Sonst ist `async` ein Bezeichner, etwa in
    /// `for (async of x)`.
    fn async_arrow_follows(&mut self) -> Result<bool> {
        self.token(Goal::RegExp)?;
        let Some(lookahead) = &self.current else {
            return Ok(false);
        };
        let mut lexer = lookahead.after.clone();
        let Ok(second) = lexer.next_token(Goal::Div) else {
            return Ok(false);
        };
        let name =
            matches!(&second.kind, TokenKind::Name(name) if !super::is_reserved_word(&name.value));
        if !name || second.newline_before {
            return Ok(false);
        }
        Ok(lexer.next_token(Goal::Div).is_ok_and(|third| {
            third.kind == TokenKind::Punctuator(Punctuator::Arrow) && !third.newline_before
        }))
    }

    /// `async x => …`: der Kopf mit einem Bezeichner.
    fn async_arrow_identifier(&mut self, start: usize) -> Result<NodeId> {
        self.advance();
        let yield_await = self.function.yield_await;
        let await_identifiers = self.function.await_identifiers;
        let position = self.start(Goal::Div)?;
        let Some(name) = self.name(Goal::Div)? else {
            return Err(self.unexpected(Goal::Div));
        };
        let atom = self.identifier_reference(&name, position)?;
        self.advance();
        let parameter = self.node(Node::Identifier(atom), position);
        let token = self.token(Goal::Div)?;
        if token.kind != TokenKind::Punctuator(Punctuator::Arrow) || token.newline_before {
            return Err(self.unexpected(Goal::Div));
        }
        let mark = self.scratch.len();
        self.scratch.push(parameter);
        let list = self.finish_list(mark);
        let node = self.node(Node::Sequence(list), start);
        self.arrow_head = Some(ArrowHead {
            node,
            is_async: true,
            yield_await,
            await_identifiers,
        });
        Ok(node)
    }

    /// Ein Klammerausdruck oder die Parameter einer Pfeilfunktion
    /// (*CoverParenthesizedExpressionAndArrowParameterList*).
    fn parenthesized(&mut self) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        let yield_await = self.function.yield_await;
        let await_identifiers = self.function.await_identifiers;
        self.advance();
        let mut cover = Cover::default();
        let mark = self.scratch.len();
        // Wo etwas steht, das nur Parameter erlauben: `()`, `(...x)`,
        // `(a,)`.
        let mut only_parameters: Option<usize> = None;
        if self.at(Punctuator::RightParenthesis, Goal::RegExp)? {
            only_parameters = Some(self.start(Goal::RegExp)?);
        }
        while only_parameters.is_none() {
            if self.at(Punctuator::Ellipsis, Goal::RegExp)? {
                only_parameters = Some(self.parenthesized_rest()?);
                break;
            }
            let element = self.assignment(true, Some(&mut cover))?;
            self.scratch.push(element);
            if !self.eat(Punctuator::Comma, Goal::Div)? {
                break;
            }
            if self.at(Punctuator::RightParenthesis, Goal::RegExp)? {
                only_parameters = Some(self.start(Goal::RegExp)?);
            }
        }
        self.expect(Punctuator::RightParenthesis, Goal::RegExp)?;
        let list = self.finish_list(mark);
        self.parenthesized_end(
            start,
            list,
            only_parameters,
            cover,
            yield_await,
            await_identifiers,
        )
    }

    /// `...` mit seinem Ziel, nur in Parametern erlaubt. Gibt zurück, wo
    /// es beginnt.
    fn parenthesized_rest(&mut self) -> Result<usize> {
        let rest_start = self.start(Goal::RegExp)?;
        self.advance();
        let mut names = Vec::new();
        let target = self.binding_target(&mut names)?;
        let rest = self.node(Node::Rest(target), rest_start);
        self.scratch.push(rest);
        Ok(rest_start)
    }

    /// Nach `)`: mit `=>` der Kopf einer Pfeilfunktion, sonst ein
    /// Klammerausdruck.
    fn parenthesized_end(
        &mut self,
        start: usize,
        list: List,
        only_parameters: Option<usize>,
        cover: Cover,
        yield_await: u32,
        await_identifiers: u32,
    ) -> Result<NodeId> {
        let token = self.token(Goal::Div)?;
        if token.kind == TokenKind::Punctuator(Punctuator::Arrow) && !token.newline_before {
            let node = self.node(Node::Sequence(list), start);
            self.arrow_head = Some(ArrowHead {
                node,
                is_async: false,
                yield_await,
                await_identifiers,
            });
            return Ok(node);
        }
        if let Some(position) = only_parameters {
            return Err(self.error_at(ErrorKind::InvalidCoverGrammar, position));
        }
        cover.check()?;
        let items = self.ast.list(list).to_vec();
        let inner = match items.as_slice() {
            [single] => *single,
            _ => {
                let first = items
                    .first()
                    .map_or(start, |first| self.ast.range(*first).start);
                let last = items.last().map_or(start, |last| self.ast.range(*last).end);
                self.node_at(Node::Sequence(list), first, last)
            }
        };
        Ok(self.node(Node::Paren(inner), start))
    }

    /// *ArrayLiteral*
    fn array_literal(&mut self, cover: &mut Cover) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let mark = self.scratch.len();
        loop {
            if self.at(Punctuator::RightBracket, Goal::RegExp)? {
                break;
            }
            if self.at(Punctuator::Comma, Goal::RegExp)? {
                let position = self.start(Goal::RegExp)?;
                self.advance();
                let elision = self.node(Node::Elision, position);
                self.scratch.push(elision);
                continue;
            }
            let element = if self.at(Punctuator::Ellipsis, Goal::RegExp)? {
                let spread_start = self.start(Goal::RegExp)?;
                self.advance();
                let argument = self.assignment(true, Some(cover))?;
                let spread = self.node(Node::Spread(argument), spread_start);
                if self.at(Punctuator::Comma, Goal::Div)? {
                    self.spread_with_comma.insert(spread);
                }
                spread
            } else {
                self.assignment(true, Some(cover))?
            };
            self.scratch.push(element);
            if self.at(Punctuator::RightBracket, Goal::Div)? {
                break;
            }
            self.expect(Punctuator::Comma, Goal::Div)?;
        }
        self.expect(Punctuator::RightBracket, Goal::RegExp)?;
        let list = self.finish_list(mark);
        Ok(self.node(Node::Array(list), start))
    }

    /// *ObjectLiteral*
    fn object_literal(&mut self, cover: &mut Cover) -> Result<NodeId> {
        let start = self.start(Goal::RegExp)?;
        self.advance();
        let mark = self.scratch.len();
        let mut proto = false;
        loop {
            if self.at(Punctuator::RightBrace, Goal::Div)? {
                break;
            }
            let property = self.property_definition(cover, &mut proto)?;
            self.scratch.push(property);
            if self.at(Punctuator::RightBrace, Goal::Div)? {
                break;
            }
            self.expect(Punctuator::Comma, Goal::Div)?;
        }
        self.expect(Punctuator::RightBrace, Goal::Div)?;
        let list = self.finish_list(mark);
        Ok(self.node(Node::Object(list), start))
    }

    /// *PropertyDefinition*
    fn property_definition(&mut self, cover: &mut Cover, proto: &mut bool) -> Result<NodeId> {
        let start = self.start(Goal::Div)?;
        if self.eat(Punctuator::Ellipsis, Goal::Div)? {
            return self.spread_property(start, cover);
        }
        let modifiers = self.method_modifiers(false)?;
        if modifiers.any() {
            return self.method_property(start, modifiers);
        }
        let name = self.name(Goal::Div)?;
        let (key, key_name) = self.property_name(false)?;
        match self.punctuator(Goal::Div)? {
            Some(Punctuator::Colon) => self.init_property(start, key, key_name, cover, proto),
            Some(Punctuator::LeftParenthesis) => {
                let function = self.method(MethodModifiers::default(), start, None)?;
                let value = self.node(Node::Function(function), start);
                Ok(self.node(
                    Node::Property {
                        kind: PropertyKind::Method,
                        key,
                        value,
                    },
                    start,
                ))
            }
            _ => self.shorthand_property(start, key, name, cover),
        }
    }

    /// `...` und sein Ausdruck in einem Objekt-Literal.
    fn spread_property(&mut self, start: usize, cover: &mut Cover) -> Result<NodeId> {
        let argument = self.assignment(true, Some(cover))?;
        let spread = self.node(Node::Spread(argument), start);
        if self.at(Punctuator::Comma, Goal::Div)? {
            self.spread_with_comma.insert(spread);
        }
        Ok(spread)
    }

    /// Eine Methode mit `get`, `set`, `async` oder `*` davor.
    fn method_property(&mut self, start: usize, modifiers: MethodModifiers) -> Result<NodeId> {
        let (key, _) = self.property_name(false)?;
        let function = self.method(modifiers, start, None)?;
        let kind = match modifiers.accessor {
            Some(crate::ast::MethodKind::Get) => PropertyKind::Get,
            Some(crate::ast::MethodKind::Set) => PropertyKind::Set,
            _ => PropertyKind::Method,
        };
        let value = self.node(Node::Function(function), start);
        Ok(self.node(Node::Property { kind, key, value }, start))
    }

    /// `key: value`; die aktuelle Stelle ist `:`.
    fn init_property(
        &mut self,
        start: usize,
        key: NodeId,
        key_name: Option<Atom>,
        cover: &mut Cover,
        proto: &mut bool,
    ) -> Result<NodeId> {
        self.advance();
        let value = self.assignment(true, Some(cover))?;
        if key_name.is_some_and(|key_name| self.atom_is(key_name, "__proto__")) {
            if *proto {
                cover.record(self.error_at(ErrorKind::DuplicateProto, start));
            }
            *proto = true;
        }
        Ok(self.node(
            Node::Property {
                kind: PropertyKind::Init,
                key,
                value,
            },
            start,
        ))
    }

    /// Die Kurzform: der Name ist eine *IdentifierReference*.
    fn shorthand_property(
        &mut self,
        start: usize,
        key: NodeId,
        name: Option<Name>,
        cover: &mut Cover,
    ) -> Result<NodeId> {
        let Some(name) = name else {
            return Err(self.unexpected(Goal::Div));
        };
        let atom = self.identifier_reference(&name, start)?;
        let mut value = self.node_at(Node::Identifier(atom), start, self.previous_end);
        if self.at(Punctuator::Assign, Goal::Div)? {
            // *CoverInitializedName*, nur in Mustern gültig.
            cover.record(self.error(ErrorKind::InvalidCoverGrammar));
            self.advance();
            let initializer = self.assignment(true, None)?;
            value = self.node(
                Node::Assign {
                    operator: AssignOperator::Assign,
                    target: value,
                    value: initializer,
                },
                start,
            );
        }
        Ok(self.node(
            Node::Property {
                kind: PropertyKind::Shorthand,
                key,
                value,
            },
            start,
        ))
    }

    /// *PropertyName* (und mit `private` *ClassElementName*). Gibt auch
    /// *PropName* zurück, wenn der Name kein berechneter ist.
    pub(super) fn property_name(&mut self, private: bool) -> Result<(NodeId, Option<Atom>)> {
        let start = self.start(Goal::Div)?;
        let token = self.token(Goal::Div)?;
        match &token.kind {
            TokenKind::Name(name) => {
                let name = name.clone();
                self.advance();
                let atom = self.intern_name(&name);
                Ok((self.node(Node::PropertyName(atom), start), Some(atom)))
            }
            TokenKind::String(_) | TokenKind::Number(_) | TokenKind::BigInt(_) => {
                self.literal_property_name()
            }
            TokenKind::PrivateName(name) if private => {
                let name = name.clone();
                self.advance();
                let atom = self.intern_name(&name);
                Ok((self.node(Node::PrivateName(atom), start), None))
            }
            TokenKind::Punctuator(Punctuator::LeftBracket) => self.computed_key(start),
            _ => Err(self.unexpected(Goal::Div)),
        }
    }

    /// `[…]` als Name.
    fn computed_key(&mut self, start: usize) -> Result<(NodeId, Option<Atom>)> {
        self.advance();
        let expression = self.assignment(true, None)?;
        self.expect(Punctuator::RightBracket, Goal::Div)?;
        Ok((self.node(Node::ComputedKey(expression), start), None))
    }

    /// Eine Zahl oder Zeichenkette als aktuelles Token, mit *PropName* für
    /// Zeichenketten.
    fn literal_property_name(&mut self) -> Result<(NodeId, Option<Atom>)> {
        let start = self.start(Goal::Div)?;
        let strict = self.strict();
        let (node, legacy) = match &self.token(Goal::Div)?.kind {
            TokenKind::Number(number) => (Literal::Number(number.value), number.legacy),
            TokenKind::BigInt(big) => (
                Literal::BigInt(
                    u8::try_from(big.radix).unwrap_or(10),
                    big.digits.encode_utf16().collect(),
                ),
                false,
            ),
            TokenKind::String(string) => {
                (Literal::String(string.value.clone()), string.legacy_escape)
            }
            _ => return Err(self.unexpected(Goal::Div)),
        };
        if legacy && strict {
            return Err(self.error(ErrorKind::StrictLegacyLiteral));
        }
        self.advance();
        Ok(match node {
            Literal::Number(value) => (self.node(Node::Number(value), start), None),
            Literal::BigInt(radix, digits) => {
                let digits = self.intern(&digits);
                (self.node(Node::BigInt { radix, digits }, start), None)
            }
            Literal::String(value) => {
                let atom = self.intern(&value);
                (self.node(Node::String(atom), start), Some(atom))
            }
        })
    }

    /// Ein *Template*, mit `tagged` nach einem Tag. Das aktuelle Token ist
    /// sein Anfang.
    pub(super) fn template(&mut self, tagged: bool) -> Result<NodeId> {
        let start = self.start(Goal::Div)?;
        let mut quasis = Vec::new();
        let mut expressions = Vec::new();
        let mut goal = Goal::Div;
        loop {
            let position = self.start(goal)?;
            let TokenKind::Template(template) = &self.token(goal)?.kind else {
                return Err(self.unexpected(goal));
            };
            let part = template.part;
            let continued = matches!(part, TemplatePart::Middle | TemplatePart::Tail);
            if continued != (goal == Goal::TemplateTail) {
                return Err(self.unexpected(goal));
            }
            let cooked = template.cooked.clone();
            let raw = template.raw.clone();
            if cooked.is_none() && !tagged {
                return Err(self.error(ErrorKind::InvalidTemplateEscape));
            }
            self.advance();
            let cooked = cooked.map(|cooked| self.intern(&cooked));
            let raw = self.intern(&raw);
            let element = self.node(Node::TemplateElement { cooked, raw }, position);
            quasis.push(element);
            if matches!(part, TemplatePart::NoSubstitution | TemplatePart::Tail) {
                break;
            }
            let expression = self.expression(true)?;
            expressions.push(expression);
            goal = Goal::TemplateTail;
        }
        let quasis = self.ast.add_list(&quasis);
        let expressions = self.ast.add_list(&expressions);
        Ok(self.node(
            Node::Template {
                quasis,
                expressions,
            },
            start,
        ))
    }

    // Bezeichner (Abschnitt 13.1)

    /// Die Regeln für *IdentifierReference*, *BindingIdentifier* und
    /// *LabelIdentifier*, die allen gemeinsam sind.
    fn check_identifier(&mut self, name: &Name, position: usize) -> Result<()> {
        let value = name.value.as_str();
        let context = self.context();
        let error = |kind| Err(super::SyntaxError { kind, position });
        match value {
            "yield" => {
                if context.yield_ || context.strict {
                    return error(ErrorKind::ReservedWord);
                }
            }
            "await" => {
                if context.await_ || self.module || context.static_block {
                    return error(ErrorKind::ReservedWord);
                }
                self.function.await_identifiers = self.function.await_identifiers.saturating_add(1);
            }
            _ if super::is_reserved_word(value) => {
                return error(if name.escaped {
                    ErrorKind::ReservedWord
                } else {
                    ErrorKind::UnexpectedToken
                });
            }
            _ if context.strict && super::is_strict_reserved_word(value) => {
                return error(ErrorKind::ReservedWord);
            }
            _ => {}
        }
        Ok(())
    }

    /// *IdentifierReference*
    pub(super) fn identifier_reference(&mut self, name: &Name, position: usize) -> Result<Atom> {
        self.check_identifier(name, position)?;
        if name.value == "arguments" && self.context().no_arguments {
            return Err(self.error_at(ErrorKind::InvalidArguments, position));
        }
        Ok(self.intern_name(name))
    }

    /// *BindingIdentifier*
    pub(super) fn binding_identifier(&mut self, name: &Name, position: usize) -> Result<Atom> {
        self.check_identifier(name, position)?;
        if self.strict() && (name.value == "eval" || name.value == "arguments") {
            return Err(self.error_at(ErrorKind::StrictEvalOrArguments, position));
        }
        Ok(self.intern_name(name))
    }

    /// *LabelIdentifier*
    pub(super) fn label_identifier(&mut self, name: &Name, position: usize) -> Result<Atom> {
        self.check_identifier(name, position)?;
        Ok(self.intern_name(name))
    }

    /// Liest einen *BindingIdentifier* als aktuelles Token.
    pub(super) fn binding_identifier_token(&mut self) -> Result<(Atom, usize)> {
        let position = self.start(Goal::Div)?;
        let Some(name) = self.name(Goal::Div)? else {
            return Err(self.unexpected(Goal::Div));
        };
        let atom = self.binding_identifier(&name, position)?;
        self.advance();
        Ok((atom, position))
    }
}

/// Der Inhalt eines Literals als Token.
enum Literal {
    Number(f64),
    BigInt(u8, Vec<u16>),
    String(Vec<u16>),
}

/// Die Vorsätze einer Methode: `get`, `set`, `async` und `*`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MethodModifiers {
    pub(super) accessor: Option<crate::ast::MethodKind>,
    pub(super) is_async: bool,
    pub(super) generator: bool,
}

impl MethodModifiers {
    pub(super) fn any(self) -> bool {
        self.accessor.is_some() || self.is_async || self.generator
    }
}

impl Parser<'_> {
    /// Liest `get`, `set`, `async` und `*` vor einem Methodennamen, wenn
    /// ein Name folgt. Mit `class` auch vor privaten Namen.
    pub(super) fn method_modifiers(&mut self, class: bool) -> Result<MethodModifiers> {
        let mut modifiers = MethodModifiers::default();
        if let Some(name) = self.name(Goal::Div)?
            && !name.escaped
            && matches!(name.value.as_str(), "get" | "set" | "async")
        {
            let Some(next) = self.peek(Goal::Div, Goal::Div)? else {
                return Ok(modifiers);
            };
            let name_follows = match &next.kind {
                TokenKind::Name(_)
                | TokenKind::String(_)
                | TokenKind::Number(_)
                | TokenKind::BigInt(_)
                | TokenKind::Punctuator(Punctuator::LeftBracket) => true,
                TokenKind::PrivateName(_) => class,
                TokenKind::Punctuator(Punctuator::Star) => name.value == "async",
                _ => false,
            };
            if !name_follows || (name.value == "async" && next.newline_before) {
                return Ok(modifiers);
            }
            self.advance();
            match name.value.as_str() {
                "get" => modifiers.accessor = Some(crate::ast::MethodKind::Get),
                "set" => modifiers.accessor = Some(crate::ast::MethodKind::Set),
                _ => modifiers.is_async = true,
            }
            if modifiers.accessor.is_some() {
                return Ok(modifiers);
            }
        }
        if self.eat(Punctuator::Star, Goal::Div)? {
            modifiers.generator = true;
        }
        Ok(modifiers)
    }
}
