//! Zeichenklassen: ohne `v` *NonemptyClassRanges*, mit `v`
//! *ClassSetExpression*, deren verschachtelte Klassen ein eigener Stapel
//! hält.

use super::{
    ClassOperation, Error, ErrorKind, Node, NodeId, Parser, Result, ascii, class_escape, take,
};

/// Ein Eintrag einer Klasse ohne `v`: ein Zeichen oder eine Klasse wie
/// `\d`, die keine Grenze eines Bereichs sein kann.
enum ClassAtom {
    Character(u32),
    Class(NodeId),
}

/// Eine offene Klasse mit `v`.
struct SetFrame {
    /// Die Lage des `[`.
    start: usize,
    negated: bool,
    /// Steht fest, sobald ein Operator oder ein zweiter Operand folgt.
    operation: Option<ClassOperation>,
    /// Ab wo in `Parser::terms` die Operanden stehen.
    items: usize,
    count: usize,
    /// Ob hinter `&&` oder `--` der Operand noch fehlt.
    expect_operand: bool,
    first_is_range: bool,
    /// MayContainStrings des ersten Operanden, eines beliebigen und aller.
    first_strings: bool,
    any_strings: bool,
    all_strings: bool,
}

impl Parser<'_> {
    /// Eine Klasse ab dem `[`.
    pub(super) fn class(&mut self) -> Result<NodeId> {
        if self.sets {
            self.class_set()
        } else {
            self.class_ranges()
        }
    }

    // Ohne `v`

    fn class_ranges(&mut self) -> Result<NodeId> {
        let start = self.position;
        self.advance(1);
        let negated = self.eat(b'^');
        let items = self.terms.len();
        loop {
            if self.position >= self.units.len() {
                return Err(self.error_at(ErrorKind::UnterminatedClass, start));
            }
            if self.eat(b']') {
                break;
            }
            let first_start = self.position;
            let first = self.class_atom(start)?;
            // `-` vor `]` ist ein Zeichen, sonst beginnt ein Bereich.
            if self.unit(0) != Some(b'-') || self.unit(1) == Some(b']') {
                self.push_class_atom(first)?;
                continue;
            }
            self.advance(1);
            let second = self.class_atom(start)?;
            match (first, second) {
                (ClassAtom::Character(first), ClassAtom::Character(last)) => {
                    if first > last {
                        return Err(self.error_at(ErrorKind::ClassRangeOutOfOrder, first_start));
                    }
                    let range = self.node(Node::Range { first, last })?;
                    self.terms.push(range);
                }
                (first, second) => {
                    if self.unicode {
                        return Err(self.error_at(ErrorKind::InvalidClassRange, first_start));
                    }
                    // Anhang B: Mit einer Klasse als Grenze stehen beide
                    // Seiten und `-` einzeln in der Klasse.
                    self.push_class_atom(first)?;
                    let dash = self.node(Node::Character(u32::from(b'-')))?;
                    self.terms.push(dash);
                    self.push_class_atom(second)?;
                }
            }
        }
        let items = self.take_terms(items)?;
        self.node(Node::Class {
            negated,
            operation: ClassOperation::Union,
            items,
        })
    }

    /// *ClassAtom*; `class` ist die Lage des `[`.
    fn class_atom(&mut self, class: usize) -> Result<ClassAtom> {
        let Some((c, width)) = self.peek_with_width() else {
            return Err(self.error_at(ErrorKind::UnterminatedClass, class));
        };
        if ascii(c) != Some(b'\\') {
            self.advance(width);
            return Ok(ClassAtom::Character(c));
        }
        let start = self.position;
        self.advance(1);
        if self.position >= self.units.len() {
            return Err(self.error_at(ErrorKind::UnexpectedEnd, start));
        }
        if let Some(escape) = self.unit(0).and_then(class_escape) {
            self.advance(1);
            return Ok(ClassAtom::Class(self.node(Node::ClassEscape(escape))?));
        }
        Ok(match self.unit(0) {
            Some(b'b') => {
                self.advance(1);
                ClassAtom::Character(0x08)
            }
            Some(b'-') if self.unicode => {
                self.advance(1);
                ClassAtom::Character(u32::from(b'-'))
            }
            Some(b'p' | b'P') if self.unicode => {
                let (property, _) = self.property(start)?;
                ClassAtom::Class(self.node(property)?)
            }
            _ => ClassAtom::Character(self.character_escape(true)?),
        })
    }

    fn push_class_atom(&mut self, atom: ClassAtom) -> Result<()> {
        let node = match atom {
            ClassAtom::Character(c) => self.node(Node::Character(c))?,
            ClassAtom::Class(node) => node,
        };
        self.terms.push(node);
        Ok(())
    }

    // Mit `v`

    fn class_set(&mut self) -> Result<NodeId> {
        let mut stack = Vec::new();
        self.open_set(&mut stack);
        loop {
            let Some(frame) = stack.last_mut() else {
                return Err(self.error(ErrorKind::UnterminatedClass));
            };
            if self.position >= self.units.len() {
                return Err(self.error_at(ErrorKind::UnterminatedClass, frame.start));
            }
            let invalid = self.error(ErrorKind::InvalidClassSetOperation);
            if self.unit(0) == Some(b']') {
                if frame.expect_operand {
                    return Err(invalid);
                }
                self.advance(1);
                let Some(frame) = stack.pop() else {
                    return Err(invalid);
                };
                let (class, strings) = self.close_set(&frame)?;
                if stack.is_empty() {
                    return Ok(class);
                }
                self.set_operand(&mut stack, class, strings, false)?;
                continue;
            }
            if frame.count > 0 && !frame.expect_operand {
                let operator = match (self.unit(0), self.unit(1)) {
                    (Some(b'&'), Some(b'&')) => Some(ClassOperation::Intersection),
                    (Some(b'-'), Some(b'-')) => Some(ClassOperation::Subtraction),
                    _ => None,
                };
                if let Some(operator) = operator {
                    let valid = match frame.operation {
                        None => frame.count == 1 && !frame.first_is_range,
                        Some(current) => current == operator,
                    };
                    if !valid {
                        return Err(invalid);
                    }
                    self.advance(2);
                    // `&&&` ist reserviert.
                    if operator == ClassOperation::Intersection && self.unit(0) == Some(b'&') {
                        return Err(self.error(ErrorKind::InvalidClassSetOperation));
                    }
                    frame.operation = Some(operator);
                    frame.expect_operand = true;
                    continue;
                }
                if matches!(
                    frame.operation,
                    Some(ClassOperation::Intersection | ClassOperation::Subtraction)
                ) {
                    return Err(invalid);
                }
            }
            if self.unit(0) == Some(b'[') {
                self.open_set(&mut stack);
                continue;
            }
            let (operand, strings, is_range) = self.set_operand_atom()?;
            self.set_operand(&mut stack, operand, strings, is_range)?;
        }
    }

    fn open_set(&mut self, stack: &mut Vec<SetFrame>) {
        let start = self.position;
        self.advance(1);
        let negated = self.eat(b'^');
        stack.push(SetFrame {
            start,
            negated,
            operation: None,
            items: self.terms.len(),
            count: 0,
            expect_operand: false,
            first_is_range: false,
            first_strings: false,
            any_strings: false,
            all_strings: false,
        });
    }

    /// Schließt eine Klasse mit `v` und sagt, ob sie Zeichenketten
    /// enthalten kann (MayContainStrings).
    fn close_set(&mut self, frame: &SetFrame) -> Result<(NodeId, bool)> {
        let items = self.take_terms(frame.items)?;
        let operation = frame.operation.unwrap_or(ClassOperation::Union);
        let strings = frame.count > 0
            && match operation {
                ClassOperation::Union => frame.any_strings,
                ClassOperation::Intersection => frame.all_strings,
                ClassOperation::Subtraction => frame.first_strings,
            };
        if frame.negated && strings {
            return Err(self.error_at(ErrorKind::NegatedStrings, frame.start));
        }
        let class = self.node(Node::Class {
            negated: frame.negated,
            operation,
            items,
        })?;
        Ok((class, strings))
    }

    /// Fügt der innersten Klasse einen Operanden hinzu.
    fn set_operand(
        &mut self,
        stack: &mut [SetFrame],
        operand: NodeId,
        strings: bool,
        is_range: bool,
    ) -> Result<()> {
        let error = self.error(ErrorKind::InvalidClassSetOperation);
        let Some(frame) = stack.last_mut() else {
            return Err(error);
        };
        if frame.expect_operand {
            // Hinter `&&` und `--` stehen keine Bereiche.
            if is_range {
                return Err(error);
            }
        } else if frame.count > 0 {
            frame.operation = Some(ClassOperation::Union);
        }
        if frame.count == 0 {
            frame.first_is_range = is_range;
            frame.first_strings = strings;
            frame.all_strings = strings;
        }
        frame.any_strings |= strings;
        frame.all_strings &= strings;
        frame.count = frame.count.saturating_add(1);
        frame.expect_operand = false;
        self.terms.push(operand);
        Ok(())
    }

    /// Ein Operand außer einer verschachtelten Klasse in `[…]`: `\q{…}`, ein
    /// Klassen-Escape, eine Eigenschaft, ein Zeichen oder ein Bereich. Sagt
    /// auch, ob er Zeichenketten enthalten kann und ob er ein Bereich ist.
    fn set_operand_atom(&mut self) -> Result<(NodeId, bool, bool)> {
        let start = self.position;
        if self.unit(0) == Some(b'\\') {
            match self.unit(1) {
                Some(b'q') => {
                    self.advance(2);
                    let (strings, may_contain) = self.class_strings(start)?;
                    return Ok((strings, may_contain, false));
                }
                Some(unit) if class_escape(unit).is_some() => {
                    self.advance(2);
                    let escape = class_escape(unit).map(Node::ClassEscape);
                    let node = self.node(escape.unwrap_or(Node::Dot))?;
                    return Ok((node, false, false));
                }
                Some(b'p' | b'P') => {
                    self.advance(1);
                    let (property, strings) = self.property(start)?;
                    return Ok((self.node(property)?, strings, false));
                }
                _ => {}
            }
        }
        let first = self.class_set_character()?;
        if self.unit(0) == Some(b'-') && self.unit(1) != Some(b'-') {
            self.advance(1);
            let last = self.class_set_character()?;
            if first > last {
                return Err(self.error_at(ErrorKind::ClassRangeOutOfOrder, start));
            }
            return Ok((self.node(Node::Range { first, last })?, false, true));
        }
        Ok((self.node(Node::Character(first))?, false, false))
    }

    /// *ClassSetCharacter*
    fn class_set_character(&mut self) -> Result<u32> {
        let start = self.position;
        let Some((c, width)) = self.peek_with_width() else {
            return Err(self.error(ErrorKind::UnterminatedClass));
        };
        if ascii(c) == Some(b'\\') {
            self.advance(1);
            if self.position >= self.units.len() {
                return Err(self.error_at(ErrorKind::UnexpectedEnd, start));
            }
            return match self.unit(0) {
                Some(b'b') => {
                    self.advance(1);
                    Ok(0x08)
                }
                Some(unit) if is_reserved_punctuator(unit) => {
                    self.advance(1);
                    Ok(u32::from(unit))
                }
                _ => self.character_escape(true),
            };
        }
        if let Some(c) = ascii(c)
            && (is_set_syntax_character(c)
                || (is_reserved_double_punctuator(c) && self.unit(1) == Some(c)))
        {
            return Err(self.error(ErrorKind::InvalidClassSetOperation));
        }
        self.advance(width);
        Ok(c)
    }

    /// `{…}` hinter `\q`; `start` ist die Lage des `\`. Sagt auch, ob es
    /// Zeichenketten enthalten kann, also eine leere oder eine aus mehr als
    /// einem Zeichen.
    fn class_strings(&mut self, start: usize) -> Result<(NodeId, bool)> {
        if !self.eat(b'{') {
            return Err(self.error_at(ErrorKind::InvalidEscape, start));
        }
        let alternatives = self.alternatives.len();
        let mut may_contain = false;
        loop {
            let items = self.terms.len();
            let mut length = 0usize;
            while self.position < self.units.len() && !matches!(self.unit(0), Some(b'|' | b'}')) {
                let c = self.class_set_character()?;
                let node = self.node(Node::Character(c))?;
                self.terms.push(node);
                length = length.saturating_add(1);
            }
            if self.position >= self.units.len() {
                return Err(self.error_at(ErrorKind::UnterminatedClass, start));
            }
            may_contain |= length != 1;
            let string = self.take_terms(items)?;
            let alternative = self.node(Node::Alternative(string))?;
            self.alternatives.push(alternative);
            let end = self.eat(b'}');
            if end {
                break;
            }
            self.advance(1);
        }
        let list = take(&mut self.lists, &mut self.alternatives, alternatives).ok_or(Error {
            kind: ErrorKind::TooLong,
            position: start,
        })?;
        Ok((self.node(Node::ClassStrings(list))?, may_contain))
    }
}

/// *ClassSetSyntaxCharacter*
fn is_set_syntax_character(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'/' | b'-' | b'\\' | b'|'
    )
}

/// Die Zeichen, die doppelt *ClassSetReservedDoublePunctuator* sind.
fn is_reserved_double_punctuator(c: u8) -> bool {
    matches!(
        c,
        b'&' | b'!'
            | b'#'
            | b'$'
            | b'%'
            | b'*'
            | b'+'
            | b','
            | b'.'
            | b':'
            | b';'
            | b'<'
            | b'='
            | b'>'
            | b'?'
            | b'@'
            | b'^'
            | b'`'
            | b'~'
    )
}

/// *ClassSetReservedPunctuator*
fn is_reserved_punctuator(c: u8) -> bool {
    matches!(
        c,
        b'&' | b'-'
            | b'!'
            | b'#'
            | b'%'
            | b','
            | b':'
            | b';'
            | b'<'
            | b'='
            | b'>'
            | b'@'
            | b'`'
            | b'~'
    )
}
