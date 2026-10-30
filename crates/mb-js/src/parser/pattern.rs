//! Muster: Bindungsmuster (Abschnitt 14.3.3), direkt gelesen, und das
//! Umdeuten von Ausdrücken zu Mustern für destrukturierende Zuweisungen
//! (Abschnitt 13.15.5) und zu Parametern von Pfeilfunktionen.

use super::{ErrorKind, Parser, Result};
use crate::ast::{AssignOperator, Atom, Node, NodeId, PropertyKind};
use crate::lexer::{Goal, Punctuator, TokenKind};

use super::expression::TargetType;

/// Gebundene Namen (*BoundNames*) mit ihrer Lage.
pub(super) type Names = Vec<(Atom, usize)>;

/// Wozu ein Ausdruck umgedeutet wird.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    /// Ein Ziel einer destrukturierenden Zuweisung.
    Assignment,
    /// Ein Parameter einer Pfeilfunktion.
    Binding,
}

impl Parser<'_> {
    /// *BindingIdentifier* oder *BindingPattern*.
    pub(super) fn binding_target(&mut self, names: &mut Names) -> Result<NodeId> {
        match self.punctuator(Goal::Div)? {
            Some(Punctuator::LeftBracket) => self.array_binding_pattern(names),
            Some(Punctuator::LeftBrace) => self.object_binding_pattern(names),
            _ => {
                let (atom, position) = self.binding_identifier_token()?;
                names.push((atom, position));
                Ok(self.node(Node::BindingIdentifier(atom), position))
            }
        }
    }

    /// *BindingElement*: ein Ziel mit Vorgabewert.
    pub(super) fn binding_element(&mut self, names: &mut Names) -> Result<NodeId> {
        let start = self.start(Goal::Div)?;
        let target = self.binding_target(names)?;
        if !self.eat(Punctuator::Assign, Goal::Div)? {
            return Ok(target);
        }
        let default = self.assignment(true, None)?;
        Ok(self.node(Node::WithDefault { target, default }, start))
    }

    fn array_binding_pattern(&mut self, names: &mut Names) -> Result<NodeId> {
        self.enter()?;
        let result = self.array_binding_pattern_inner(names);
        self.leave();
        result
    }

    fn array_binding_pattern_inner(&mut self, names: &mut Names) -> Result<NodeId> {
        let start = self.start(Goal::Div)?;
        self.advance();
        let mark = self.scratch.len();
        loop {
            if self.at(Punctuator::RightBracket, Goal::Div)? {
                break;
            }
            if self.at(Punctuator::Comma, Goal::Div)? {
                let position = self.start(Goal::Div)?;
                self.advance();
                let elision = self.node(Node::Elision, position);
                self.scratch.push(elision);
                continue;
            }
            if self.at(Punctuator::Ellipsis, Goal::Div)? {
                let rest_start = self.start(Goal::Div)?;
                self.advance();
                let target = self.binding_target(names)?;
                let rest = self.node(Node::Rest(target), rest_start);
                self.scratch.push(rest);
                break;
            }
            let element = self.binding_element(names)?;
            self.scratch.push(element);
            if self.at(Punctuator::RightBracket, Goal::Div)? {
                break;
            }
            self.expect(Punctuator::Comma, Goal::Div)?;
        }
        self.expect(Punctuator::RightBracket, Goal::Div)?;
        let list = self.finish_list(mark);
        Ok(self.node(Node::ArrayPattern(list), start))
    }

    fn object_binding_pattern(&mut self, names: &mut Names) -> Result<NodeId> {
        self.enter()?;
        let result = self.object_binding_pattern_inner(names);
        self.leave();
        result
    }

    fn object_binding_pattern_inner(&mut self, names: &mut Names) -> Result<NodeId> {
        let start = self.start(Goal::Div)?;
        self.advance();
        let mark = self.scratch.len();
        loop {
            if self.at(Punctuator::RightBrace, Goal::Div)? {
                break;
            }
            let property_start = self.start(Goal::Div)?;
            if self.eat(Punctuator::Ellipsis, Goal::Div)? {
                let (atom, position) = self.binding_identifier_token()?;
                names.push((atom, position));
                let target = self.node(Node::BindingIdentifier(atom), position);
                let rest = self.node(Node::Rest(target), property_start);
                self.scratch.push(rest);
                break;
            }
            let single = match self.name(Goal::Div)? {
                Some(_) => self
                    .peek(Goal::Div, Goal::Div)?
                    .is_none_or(|next| next.kind != TokenKind::Punctuator(Punctuator::Colon)),
                None => false,
            };
            let property = if single {
                // *SingleNameBinding*
                let (atom, position) = self.binding_identifier_token()?;
                names.push((atom, position));
                let key = self.node(Node::PropertyName(atom), position);
                let mut value = self.node(Node::BindingIdentifier(atom), position);
                if self.eat(Punctuator::Assign, Goal::Div)? {
                    let default = self.assignment(true, None)?;
                    value = self.node(
                        Node::WithDefault {
                            target: value,
                            default,
                        },
                        position,
                    );
                }
                self.node(
                    Node::PatternProperty {
                        key,
                        value,
                        shorthand: true,
                    },
                    property_start,
                )
            } else {
                let (key, _) = self.property_name(false)?;
                self.expect(Punctuator::Colon, Goal::Div)?;
                let value = self.binding_element(names)?;
                self.node(
                    Node::PatternProperty {
                        key,
                        value,
                        shorthand: false,
                    },
                    property_start,
                )
            };
            self.scratch.push(property);
            if self.at(Punctuator::RightBrace, Goal::Div)? {
                break;
            }
            self.expect(Punctuator::Comma, Goal::Div)?;
        }
        self.expect(Punctuator::RightBrace, Goal::Div)?;
        let list = self.finish_list(mark);
        Ok(self.node(Node::ObjectPattern(list), start))
    }

    /// Deutet ein Objekt- oder Array-Literal links von `=` (oder vor `in`
    /// und `of`) zum Muster um (*AssignmentPattern*).
    pub(super) fn assignment_pattern(&mut self, node: NodeId) -> Result<()> {
        let mut names = Vec::new();
        self.convert(node, Purpose::Assignment, &mut names)
    }

    /// Deutet einen Parameter aus dem Kopf einer Pfeilfunktion um und
    /// sammelt seine Namen.
    pub(super) fn arrow_parameter(
        &mut self,
        node: NodeId,
        last: bool,
        names: &mut Names,
    ) -> Result<()> {
        if let Node::Spread(argument) = *self.ast.node(node) {
            // Ein Rest-Parameter steht zuletzt und hat keinen Vorgabewert.
            if !last
                || self.spread_with_comma.contains(&node)
                || matches!(self.ast.node(argument), Node::Assign { .. })
            {
                return Err(
                    self.error_at(ErrorKind::InvalidCoverGrammar, self.ast.range(node).start)
                );
            }
            self.ast.replace(node, Node::Rest(argument));
            return self.convert(argument, Purpose::Binding, names);
        }
        if let Node::Rest(_) = self.ast.node(node)
            && !last
        {
            return Err(self.error_at(ErrorKind::InvalidCoverGrammar, self.ast.range(node).start));
        }
        self.convert(node, Purpose::Binding, names)
    }

    /// Das Umdeuten ohne Rekursion: Jeder Knoten wird für sich ersetzt,
    /// seine Teile kommen auf den Stapel.
    fn convert(&mut self, root: NodeId, purpose: Purpose, names: &mut Names) -> Result<()> {
        let mut work = vec![root];
        while let Some(id) = work.pop() {
            let start = self.ast.range(id).start;
            let invalid = |kind| {
                Err(super::SyntaxError {
                    kind,
                    position: start,
                })
            };
            match *self.ast.node(id) {
                Node::Array(list) => {
                    let items = self.ast.list(list).to_vec();
                    let count = items.len();
                    for (index, item) in items.into_iter().enumerate() {
                        match *self.ast.node(item) {
                            Node::Elision => {}
                            Node::Spread(argument) => {
                                if index.saturating_add(1) != count
                                    || self.spread_with_comma.contains(&item)
                                {
                                    return Err(self.error_at(
                                        ErrorKind::InvalidAssignmentTarget,
                                        self.ast.range(item).start,
                                    ));
                                }
                                // Ein Rest hat keinen Vorgabewert.
                                if matches!(self.ast.node(argument), Node::Assign { .. }) {
                                    return Err(self.error_at(
                                        ErrorKind::InvalidAssignmentTarget,
                                        self.ast.range(argument).start,
                                    ));
                                }
                                self.ast.replace(item, Node::Rest(argument));
                                work.push(argument);
                            }
                            _ => work.push(item),
                        }
                    }
                    self.ast.replace(id, Node::ArrayPattern(list));
                }
                Node::Object(list) => {
                    let items = self.ast.list(list).to_vec();
                    let count = items.len();
                    for (index, item) in items.into_iter().enumerate() {
                        match *self.ast.node(item) {
                            Node::Property {
                                kind: kind @ (PropertyKind::Init | PropertyKind::Shorthand),
                                key,
                                value,
                            } => {
                                self.ast.replace(
                                    item,
                                    Node::PatternProperty {
                                        key,
                                        value,
                                        shorthand: kind == PropertyKind::Shorthand,
                                    },
                                );
                                work.push(value);
                            }
                            Node::Spread(argument) => {
                                if index.saturating_add(1) != count
                                    || self.spread_with_comma.contains(&item)
                                {
                                    return Err(self.error_at(
                                        ErrorKind::InvalidAssignmentTarget,
                                        self.ast.range(item).start,
                                    ));
                                }
                                let allowed = match (purpose, self.ast.node(argument)) {
                                    (Purpose::Binding, Node::Identifier(_)) => true,
                                    (Purpose::Binding, _) => false,
                                    (_, Node::Object(_) | Node::Array(_) | Node::Assign { .. }) => {
                                        false
                                    }
                                    _ => true,
                                };
                                if !allowed {
                                    return Err(self.error_at(
                                        ErrorKind::InvalidAssignmentTarget,
                                        self.ast.range(argument).start,
                                    ));
                                }
                                self.ast.replace(item, Node::Rest(argument));
                                work.push(argument);
                            }
                            _ => {
                                return Err(self.error_at(
                                    ErrorKind::InvalidAssignmentTarget,
                                    self.ast.range(item).start,
                                ));
                            }
                        }
                    }
                    self.ast.replace(id, Node::ObjectPattern(list));
                }
                Node::Assign {
                    operator,
                    target,
                    value,
                } => {
                    // Ein Element mit Vorgabewert.
                    if operator != AssignOperator::Assign
                        || id == root && purpose == Purpose::Assignment
                    {
                        return invalid(ErrorKind::InvalidAssignmentTarget);
                    }
                    self.ast.replace(
                        id,
                        Node::WithDefault {
                            target,
                            default: value,
                        },
                    );
                    work.push(target);
                }
                Node::Identifier(atom) => match purpose {
                    Purpose::Assignment => {
                        if self.target_type(id) != TargetType::Simple {
                            return invalid(ErrorKind::StrictEvalOrArguments);
                        }
                    }
                    Purpose::Binding => {
                        if self.strict()
                            && (self.atom_is(atom, "eval") || self.atom_is(atom, "arguments"))
                        {
                            return invalid(ErrorKind::StrictEvalOrArguments);
                        }
                        self.ast.replace(id, Node::BindingIdentifier(atom));
                        names.push((atom, start));
                    }
                },
                // Schon umgedeutet: bei einer inneren Zuweisung wie in
                // `[[a] = b] = c` oder als Bindungsmuster nach `...` in
                // Klammern.
                Node::ArrayPattern(list) | Node::ObjectPattern(list) => {
                    for &item in self.ast.list(list) {
                        if !matches!(self.ast.node(item), Node::Elision) {
                            work.push(item);
                        }
                    }
                }
                Node::PatternProperty { value, .. } => work.push(value),
                Node::WithDefault { target, .. } | Node::Rest(target) => work.push(target),
                Node::BindingIdentifier(atom) if purpose == Purpose::Binding => {
                    names.push((atom, start))
                }
                Node::Paren(_)
                | Node::Member { .. }
                | Node::ComputedMember { .. }
                | Node::PrivateMember { .. }
                | Node::SuperMember(_)
                | Node::SuperComputedMember(_)
                | Node::Call { .. }
                    if purpose == Purpose::Assignment =>
                {
                    if self.target_type(id) != TargetType::Simple {
                        return invalid(ErrorKind::InvalidAssignmentTarget);
                    }
                }
                _ => {
                    return invalid(match purpose {
                        Purpose::Assignment => ErrorKind::InvalidAssignmentTarget,
                        Purpose::Binding => ErrorKind::InvalidCoverGrammar,
                    });
                }
            }
        }
        Ok(())
    }

    /// Die Namen eines schon gelesenen Bindungsmusters (*BoundNames*).
    pub(super) fn bound_names(&self, root: NodeId, names: &mut Names) {
        let mut work = vec![root];
        while let Some(id) = work.pop() {
            match *self.ast.node(id) {
                Node::BindingIdentifier(atom) => names.push((atom, self.ast.range(id).start)),
                Node::ArrayPattern(list) | Node::ObjectPattern(list) => {
                    work.extend(self.ast.list(list).iter().rev());
                }
                Node::PatternProperty { value, .. } => work.push(value),
                Node::WithDefault { target, .. } => work.push(target),
                Node::Rest(target) => work.push(target),
                _ => {}
            }
        }
    }
}
