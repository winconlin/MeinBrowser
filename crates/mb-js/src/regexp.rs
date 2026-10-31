//! Muster regulärer Ausdrücke nach ECMA-262, Abschnitt 22.2.1, mit den
//! Erweiterungen für Webbrowser aus Anhang B.1.2 und allen frühen Fehlern.
//!
//! [`parse`] liest ein Muster zu seinen Flags zu einem Baum, aus dem die
//! Engine später ihre Automaten baut; der Parser für Scripts prüft damit die
//! Literale.
//!
//! - Ohne `u` und `v` sind die Zeichen eines Musters Codeeinheiten, mit
//!   ihnen Codepunkte: Ersatzpaare werden zusammengefasst, einzelne Surrogate
//!   bleiben stehen. Lagen zählen immer in Codeeinheiten.
//! - Gruppen und Klassen liest der Parser mit eigenen Stapeln statt
//!   rekursiv (RS-8); wie tief sie verschachtelt sind, begrenzt nur die
//!   Länge des Musters.
//! - Anhang B: Ohne `u` und `v` zählt ein Vorlauf zuerst die Gruppen. Ob
//!   `\2` ein Rückverweis ist, hängt von der Zahl aller Gruppen ab, auch der
//!   späteren, und ob `\k` einen Namen verlangt, davon, ob es benannte
//!   Gruppen gibt. ParsePattern liest das Muster dafür ein zweites Mal mit
//!   *[+NamedCaptureGroups]*; hier steht das vorher fest.
//! - Zwei Gruppen dürfen gleich heißen, wenn sie in verschiedenen
//!   Alternativen stehen (MightBothParticipate). Geprüft wird jede Gruppe nur
//!   gegen die letzte mit demselben Namen davor: Stehen alle benachbarten
//!   Paare in verschiedenen Alternativen, gilt das für alle Paare.
//! - Zahlen in Quantoren über `u32::MAX` werden zu `u32::MAX`; so lange
//!   Zeichenketten gibt es nicht. Ob `{n,m}` mit n > m ein Fehler ist,
//!   entscheiden die genauen Werte.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use crate::lexer::{is_identifier_part, is_identifier_start};

mod class;
mod property;
#[cfg(test)]
mod tests;

pub use property::Property;

/// Ein Knoten in [`Pattern`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId(u32);

impl NodeId {
    fn index(self) -> usize {
        to_usize(self.0)
    }
}

/// Eine Liste von Knoten in [`Pattern`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct List {
    start: u32,
    len: u32,
}

impl List {
    pub fn len(self) -> usize {
        to_usize(self.len)
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Der Name einer Gruppe in [`Pattern`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NameId(u32);

impl NameId {
    fn index(self) -> usize {
        to_usize(self.0)
    }
}

/// Die Flags eines regulären Ausdrucks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// `d`
    pub has_indices: bool,
    /// `g`
    pub global: bool,
    /// `i`
    pub ignore_case: bool,
    /// `m`
    pub multiline: bool,
    /// `s`
    pub dot_all: bool,
    /// `u`
    pub unicode: bool,
    /// `v`
    pub unicode_sets: bool,
    /// `y`
    pub sticky: bool,
}

impl Flags {
    /// Liest Flags: nur `dgimsuvy`, jedes höchstens einmal und nicht `u`
    /// zusammen mit `v` (IsValidRegularExpressionLiteral und ParsePattern).
    pub fn parse(units: &[u16]) -> Option<Flags> {
        let mut flags = Flags::default();
        for &unit in units {
            let flag = match u8::try_from(unit).ok()? {
                b'd' => &mut flags.has_indices,
                b'g' => &mut flags.global,
                b'i' => &mut flags.ignore_case,
                b'm' => &mut flags.multiline,
                b's' => &mut flags.dot_all,
                b'u' => &mut flags.unicode,
                b'v' => &mut flags.unicode_sets,
                b'y' => &mut flags.sticky,
                _ => return None,
            };
            if *flag {
                return None;
            }
            *flag = true;
        }
        (!(flags.unicode && flags.unicode_sets)).then_some(flags)
    }
}

/// Die Flags, die eine Gruppe wie `(?i-s:…)` ein- oder ausschaltet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// `i`
    pub ignore_case: bool,
    /// `m`
    pub multiline: bool,
    /// `s`
    pub dot_all: bool,
}

impl Modifiers {
    fn is_empty(self) -> bool {
        !(self.ignore_case || self.multiline || self.dot_all)
    }

    fn overlaps(self, other: Modifiers) -> bool {
        (self.ignore_case && other.ignore_case)
            || (self.multiline && other.multiline)
            || (self.dot_all && other.dot_all)
    }
}

/// `^`, `$`, `\b` und `\B`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assertion {
    Start,
    End,
    WordBoundary,
    NotWordBoundary,
}

/// `(?=…)`, `(?!…)`, `(?<=…)` und `(?<!…)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookaround {
    Ahead,
    NotAhead,
    Behind,
    NotBehind,
}

/// `\d`, `\D`, `\s`, `\S`, `\w` und `\W`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassEscape {
    Digit,
    NotDigit,
    Space,
    NotSpace,
    Word,
    NotWord,
}

/// Wie die Einträge einer Klasse zusammengehen. Ohne `v` gibt es nur
/// [`ClassOperation::Union`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassOperation {
    Union,
    /// `&&`
    Intersection,
    /// `--`
    Subtraction,
}

/// Ein Knoten in [`Pattern`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    /// Alternativen, getrennt durch `|`, jede ein [`Node::Alternative`].
    Disjunction(List),
    /// Terme, die nacheinander passen müssen.
    Alternative(List),
    Assertion(Assertion),
    /// Der Rumpf ist ein [`Node::Disjunction`].
    Lookaround {
        kind: Lookaround,
        body: NodeId,
    },
    /// Ein Atom mit Quantor, `max` ohne Wert ist unbegrenzt.
    Quantified {
        atom: NodeId,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    },
    /// Ein Zeichen: mit `u` oder `v` ein Codepunkt, sonst eine Codeeinheit.
    Character(u32),
    /// `.`
    Dot,
    ClassEscape(ClassEscape),
    /// `\p{…}` und `\P{…}`.
    Property {
        property: Property,
        negated: bool,
    },
    /// `[…]` und `[^…]`. Ohne `v` sind die Einträge Zeichen, Bereiche,
    /// Klassen-Escapes und Eigenschaften, mit `v` auch Klassen und
    /// [`Node::ClassStrings`].
    Class {
        negated: bool,
        operation: ClassOperation,
        items: List,
    },
    /// `a-z` in einer Klasse.
    Range {
        first: u32,
        last: u32,
    },
    /// `\q{…}` mit `v`: Zeichenketten, jede ein [`Node::Alternative`] aus
    /// Zeichen.
    ClassStrings(List),
    /// `\1` und höher.
    Backreference(u32),
    /// `\k<name>`
    NamedBackreference(NameId),
    /// `(…)` und `(?<name>…)` mit der Nummer der Gruppe, ab 1.
    Capture {
        index: u32,
        name: Option<NameId>,
        body: NodeId,
    },
    /// `(?:…)` und `(?ims-ims:…)`.
    Group {
        add: Modifiers,
        remove: Modifiers,
        body: NodeId,
    },
}

/// Ein gelesenes Muster.
#[derive(Debug)]
pub struct Pattern {
    nodes: Vec<Node>,
    lists: Vec<NodeId>,
    names: Vec<Vec<u16>>,
    /// Der Name jeder Gruppe, nach ihrer Nummer.
    group_names: Vec<Option<NameId>>,
    root: NodeId,
}

impl Pattern {
    /// Das ganze Muster, ein [`Node::Disjunction`].
    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn node(&self, id: NodeId) -> Node {
        // Knoten entstehen nur im Parser, ihre Indizes sind gültig.
        self.nodes.get(id.index()).copied().unwrap_or(Node::Dot)
    }

    pub fn list(&self, list: List) -> &[NodeId] {
        let start = to_usize(list.start);
        self.lists
            .get(start..start.saturating_add(list.len()))
            .unwrap_or_default()
    }

    /// Wie viele Gruppen das Muster hat (CountLeftCapturingParensWithin).
    pub fn capture_count(&self) -> u32 {
        u32::try_from(self.group_names.len()).unwrap_or(u32::MAX)
    }

    /// Der Name der Gruppe mit dieser Nummer, gezählt ab 1.
    pub fn group_name(&self, index: u32) -> Option<NameId> {
        let index = to_usize(index).checked_sub(1)?;
        self.group_names.get(index).copied().flatten()
    }

    /// Ein Name als UTF-16-Codeeinheiten.
    pub fn name(&self, name: NameId) -> &[u16] {
        self.names.get(name.index()).map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// `u` und `v` zusammen.
    InvalidFlags,
    /// Ein Zeichen, das die Grammatik an dieser Stelle nicht erlaubt, etwa
    /// `]` oder `{` mit `u`.
    UnexpectedCharacter,
    /// Das Muster endet mit `\`.
    UnexpectedEnd,
    /// Eine Gruppe ohne `)`.
    UnterminatedGroup,
    /// Eine Klasse ohne `]`.
    UnterminatedClass,
    /// `)` ohne `(`.
    UnmatchedParenthesis,
    /// `(?` mit etwas, das keine Gruppe beginnt.
    InvalidGroup,
    /// Ein Quantor ohne Atom davor oder hinter einer Zusicherung, die sich
    /// nicht wiederholen lässt; ohne `u` auch `{1}` als Atom.
    NothingToRepeat,
    /// `{n,m}` mit n > m.
    QuantifierOutOfOrder,
    /// Ein Escape, das in diesem Modus nicht erlaubt oder unvollständig
    /// ist, etwa `\M` oder `\x4` mit `u`.
    InvalidEscape,
    /// Ein Rückverweis über die Zahl der Gruppen, mit `u` oder `v`.
    InvalidBackreference,
    InvalidGroupName,
    /// Zwei Gruppen mit gleichem Namen, die beide passen könnten.
    DuplicateGroupName,
    /// `\k<name>` ohne Gruppe dieses Namens.
    UndefinedGroupName,
    /// Ein Flag doppelt, ein anderes Zeichen als `i`, `m` oder `s`, beide
    /// Seiten von `-` leer oder auf beiden Seiten dasselbe Flag.
    InvalidModifiers,
    /// Mindestens `u32::MAX` Gruppen.
    TooManyGroups,
    /// Mehr als `u32::MAX` Knoten oder Namen.
    TooLong,
    /// Ein Bereich mit vertauschten Grenzen.
    ClassRangeOutOfOrder,
    /// Ein Bereich mit einer Klasse als Grenze, mit `u` oder `v`.
    InvalidClassRange,
    /// Mit `v`: `&&` und `--` gemischt oder mit Bereichen, ein
    /// reserviertes doppeltes Satzzeichen oder ein Zeichen, das ein Escape
    /// braucht.
    InvalidClassSetOperation,
    /// Eine unbekannte Eigenschaft oder ein unbekannter Wert in `\p{…}`.
    InvalidProperty,
    /// Eine Eigenschaft von Zeichenketten ohne `v`.
    PropertyOfStringsWithoutV,
    /// Was Zeichenketten enthalten kann, in `[^…]` oder `\P{…}`.
    NegatedStrings,
}

/// Ein früher Fehler im Muster mit seiner Lage in Codeeinheiten.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub position: usize,
}

type Result<T> = std::result::Result<T, Error>;

/// Liest ein Muster (ParsePattern).
pub fn parse(pattern: &[u16], flags: Flags) -> Result<Pattern> {
    if flags.unicode && flags.unicode_sets {
        return Err(Error {
            kind: ErrorKind::InvalidFlags,
            position: 0,
        });
    }
    let unicode = flags.unicode || flags.unicode_sets;
    let (total_captures, named) = if unicode {
        (0, true)
    } else {
        count_groups(pattern)
    };
    Parser {
        units: pattern,
        position: 0,
        unicode,
        sets: flags.unicode_sets,
        named,
        total_captures,
        nodes: Vec::new(),
        lists: Vec::new(),
        terms: Vec::new(),
        alternatives: Vec::new(),
        frames: Vec::new(),
        names: Vec::new(),
        name_ids: BTreeMap::new(),
        declared: Vec::new(),
        group_names: Vec::new(),
        references: Vec::new(),
        backreferences: Vec::new(),
    }
    .pattern()
}

/// Ohne `u` und `v`: wie viele Gruppen das Muster hat und ob eine davon
/// einen Namen hat. Ein Muster, das hier anders gezählt würde als beim
/// Lesen, ist ohnehin ungültig.
fn count_groups(units: &[u16]) -> (u32, bool) {
    let at = |position: usize| units.get(position).copied().and_then(ascii_unit);
    let mut count = 0u32;
    let mut named = false;
    let mut class = false;
    let mut position = 0usize;
    while position < units.len() {
        match at(position) {
            Some(b'\\') => position = position.saturating_add(1),
            Some(b'[') => class = true,
            Some(b']') => class = false,
            Some(b'(') if !class => {
                let next = |offset: usize| at(position.saturating_add(offset));
                if next(1) != Some(b'?') {
                    count = count.saturating_add(1);
                } else if next(2) == Some(b'<') && !matches!(next(3), Some(b'=' | b'!')) {
                    count = count.saturating_add(1);
                    named = true;
                }
            }
            _ => {}
        }
        position = position.saturating_add(1);
    }
    (count, named)
}

/// Eine offene Gruppe oder das ganze Muster.
struct Frame {
    kind: FrameKind,
    /// Die Lage des `(`.
    start: usize,
    /// Ab wo in `Parser::terms` die laufende Alternative steht.
    terms: usize,
    /// Ab wo in `Parser::alternatives` die fertigen Alternativen stehen.
    alternatives: usize,
    /// Die Lage hinter dem `(`: Eine Gruppe, deren `(` nicht davor steht,
    /// liegt in dieser.
    opened: usize,
    /// Wo die laufende Alternative beginnt.
    alternative: usize,
}

#[derive(Clone, Copy)]
enum FrameKind {
    Pattern,
    Capture { index: u32, name: Option<NameId> },
    Group { add: Modifiers, remove: Modifiers },
    Lookaround(Lookaround),
}

struct Parser<'a> {
    units: &'a [u16],
    position: usize,
    /// *[UnicodeMode]*: mit `u` oder `v`.
    unicode: bool,
    /// *[UnicodeSetsMode]*: mit `v`.
    sets: bool,
    /// *[NamedCaptureGroups]*
    named: bool,
    /// Ohne `u` und `v` die Zahl aller Gruppen aus dem Vorlauf.
    total_captures: u32,
    nodes: Vec<Node>,
    lists: Vec<NodeId>,
    /// Die Terme der offenen Alternativen und die Einträge offener Klassen.
    terms: Vec<NodeId>,
    /// Die fertigen Alternativen der offenen Disjunktionen und Zeichenketten
    /// in `\q{…}`.
    alternatives: Vec<NodeId>,
    frames: Vec<Frame>,
    names: Vec<Vec<u16>>,
    name_ids: BTreeMap<Vec<u16>, NameId>,
    /// Zu jedem Namen die Lage des `(` der letzten Gruppe, die so heißt.
    declared: Vec<Option<usize>>,
    group_names: Vec<Option<NameId>>,
    /// `\k<name>` mit seiner Lage, geprüft am Ende.
    references: Vec<(NameId, usize)>,
    /// Rückverweise mit `u` oder `v` und ihre Lage, geprüft am Ende.
    backreferences: Vec<(u32, usize)>,
}

impl Parser<'_> {
    fn pattern(mut self) -> Result<Pattern> {
        self.frames.push(Frame {
            kind: FrameKind::Pattern,
            start: 0,
            terms: 0,
            alternatives: 0,
            opened: 0,
            alternative: 0,
        });
        while let Some(c) = self.peek() {
            match ascii(c) {
                Some(b'|') => {
                    self.advance(1);
                    self.next_alternative()?;
                }
                Some(b')') => {
                    if self.frames.len() < 2 {
                        return Err(self.error(ErrorKind::UnmatchedParenthesis));
                    }
                    self.advance(1);
                    let (group, quantifiable) = self.close_group()?;
                    self.quantifier(group, quantifiable)?;
                }
                Some(b'(') => self.open_group()?,
                _ => self.term()?,
            }
        }
        if self.frames.len() > 1
            && let Some(frame) = self.frames.last()
        {
            return Err(Error {
                kind: ErrorKind::UnterminatedGroup,
                position: frame.start,
            });
        }
        let root = self.close_disjunction()?;
        let captures = u32::try_from(self.group_names.len()).unwrap_or(u32::MAX);
        if let Some(&(_, position)) = self
            .backreferences
            .iter()
            .find(|&&(number, _)| number > captures)
        {
            return Err(Error {
                kind: ErrorKind::InvalidBackreference,
                position,
            });
        }
        if let Some(&(_, position)) = self
            .references
            .iter()
            .find(|&&(name, _)| self.declared.get(name.index()).is_none_or(Option::is_none))
        {
            return Err(Error {
                kind: ErrorKind::UndefinedGroupName,
                position,
            });
        }
        debug_assert!(
            self.unicode || captures == self.total_captures,
            "Vorlauf und Parser zählen die Gruppen gleich"
        );
        Ok(Pattern {
            nodes: self.nodes,
            lists: self.lists,
            names: self.names,
            group_names: self.group_names,
            root,
        })
    }

    // Gruppen und Alternativen

    fn open_group(&mut self) -> Result<()> {
        let start = self.position;
        self.advance(1);
        let kind = if self.eat(b'?') {
            match self.unit(0) {
                Some(b':') => {
                    self.advance(1);
                    FrameKind::Group {
                        add: Modifiers::default(),
                        remove: Modifiers::default(),
                    }
                }
                Some(b'=') => {
                    self.advance(1);
                    FrameKind::Lookaround(Lookaround::Ahead)
                }
                Some(b'!') => {
                    self.advance(1);
                    FrameKind::Lookaround(Lookaround::NotAhead)
                }
                Some(b'<') => match self.unit(1) {
                    Some(b'=') => {
                        self.advance(2);
                        FrameKind::Lookaround(Lookaround::Behind)
                    }
                    Some(b'!') => {
                        self.advance(2);
                        FrameKind::Lookaround(Lookaround::NotBehind)
                    }
                    _ => {
                        self.advance(1);
                        let name = self.group_name()?;
                        self.declare(name, start)?;
                        let index = self.next_capture(Some(name), start)?;
                        FrameKind::Capture {
                            index,
                            name: Some(name),
                        }
                    }
                },
                Some(b'i' | b'm' | b's' | b'-') => self.modifiers(start)?,
                _ => {
                    return Err(Error {
                        kind: ErrorKind::InvalidGroup,
                        position: start,
                    });
                }
            }
        } else {
            let index = self.next_capture(None, start)?;
            FrameKind::Capture { index, name: None }
        };
        let opened = start.saturating_add(1);
        self.frames.push(Frame {
            kind,
            start,
            terms: self.terms.len(),
            alternatives: self.alternatives.len(),
            opened,
            alternative: opened,
        });
        Ok(())
    }

    /// `ims-ims:` hinter `(?`.
    fn modifiers(&mut self, start: usize) -> Result<FrameKind> {
        let invalid = Error {
            kind: ErrorKind::InvalidModifiers,
            position: start,
        };
        let add = self.modifier_set().ok_or(invalid)?;
        let mut remove = Modifiers::default();
        if self.eat(b'-') {
            remove = self.modifier_set().ok_or(invalid)?;
            if (add.is_empty() && remove.is_empty()) || add.overlaps(remove) {
                return Err(invalid);
            }
        }
        if !self.eat(b':') {
            return Err(invalid);
        }
        Ok(FrameKind::Group { add, remove })
    }

    /// *RegularExpressionModifiers*, ohne Wert, wenn ein Flag doppelt
    /// vorkommt.
    fn modifier_set(&mut self) -> Option<Modifiers> {
        let mut set = Modifiers::default();
        loop {
            let flag = match self.unit(0) {
                Some(b'i') => &mut set.ignore_case,
                Some(b'm') => &mut set.multiline,
                Some(b's') => &mut set.dot_all,
                _ => return Some(set),
            };
            if *flag {
                return None;
            }
            *flag = true;
            self.advance(1);
        }
    }

    fn next_capture(&mut self, name: Option<NameId>, start: usize) -> Result<u32> {
        // CountLeftCapturingParensWithin muss kleiner als 2³² - 1 bleiben.
        let index = u32::try_from(self.group_names.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .filter(|&index| index < u32::MAX)
            .ok_or(Error {
                kind: ErrorKind::TooManyGroups,
                position: start,
            })?;
        self.group_names.push(name);
        Ok(index)
    }

    /// Merkt sich eine benannte Gruppe, deren `(` bei `start` steht. Die
    /// letzte Gruppe mit demselben Namen muss in einer anderen Alternative
    /// stehen: Die tiefste offene Gruppe, die vor ihr begann, enthält beide;
    /// beginnt deren laufende Alternative erst hinter ihr, ist es eine
    /// andere.
    fn declare(&mut self, name: NameId, start: usize) -> Result<()> {
        let Some(slot) = self.declared.get_mut(name.index()) else {
            return Ok(());
        };
        if let Some(previous) = slot.replace(start) {
            let open = self
                .frames
                .partition_point(|frame| frame.opened <= previous);
            let common = open.checked_sub(1).and_then(|index| self.frames.get(index));
            if common.is_none_or(|frame| previous >= frame.alternative) {
                return Err(Error {
                    kind: ErrorKind::DuplicateGroupName,
                    position: start,
                });
            }
        }
        Ok(())
    }

    fn next_alternative(&mut self) -> Result<()> {
        let alternative = self.finish_alternative()?;
        self.alternatives.push(alternative);
        let position = self.position;
        if let Some(frame) = self.frames.last_mut() {
            frame.alternative = position;
        }
        Ok(())
    }

    fn finish_alternative(&mut self) -> Result<NodeId> {
        let start = self.frames.last().map_or(0, |frame| frame.terms);
        let list = self.take_terms(start)?;
        self.node(Node::Alternative(list))
    }

    /// Schließt die Disjunktion der innersten offenen Gruppe.
    fn close_disjunction(&mut self) -> Result<NodeId> {
        let alternative = self.finish_alternative()?;
        self.alternatives.push(alternative);
        let start = self.frames.last().map_or(0, |frame| frame.alternatives);
        let list = take(&mut self.lists, &mut self.alternatives, start).ok_or(self.too_long())?;
        self.node(Node::Disjunction(list))
    }

    /// Schließt die innerste Gruppe hinter ihrem `)` und sagt, ob sie einen
    /// Quantor haben darf.
    fn close_group(&mut self) -> Result<(NodeId, bool)> {
        let body = self.close_disjunction()?;
        let kind = self
            .frames
            .pop()
            .map_or(FrameKind::Pattern, |frame| frame.kind);
        let (node, quantifiable) = match kind {
            FrameKind::Pattern => return Err(self.error(ErrorKind::UnmatchedParenthesis)),
            FrameKind::Capture { index, name } => (Node::Capture { index, name, body }, true),
            FrameKind::Group { add, remove } => (Node::Group { add, remove, body }, true),
            // Anhang B: Ohne `u` und `v` lassen sich Lookaheads wiederholen.
            FrameKind::Lookaround(kind) => (
                Node::Lookaround { kind, body },
                !self.unicode && matches!(kind, Lookaround::Ahead | Lookaround::NotAhead),
            ),
        };
        Ok((self.node(node)?, quantifiable))
    }

    // Terme

    fn term(&mut self) -> Result<()> {
        let start = self.position;
        let Some((c, width)) = self.peek_with_width() else {
            return Ok(());
        };
        let (node, quantifiable) = match ascii(c) {
            Some(b'^') => {
                self.advance(1);
                (self.node(Node::Assertion(Assertion::Start))?, false)
            }
            Some(b'$') => {
                self.advance(1);
                (self.node(Node::Assertion(Assertion::End))?, false)
            }
            Some(b'.') => {
                self.advance(1);
                (self.node(Node::Dot)?, true)
            }
            Some(b'[') => (self.class()?, true),
            Some(b'\\') => self.atom_escape()?,
            Some(b'*' | b'+' | b'?') => return Err(self.error(ErrorKind::NothingToRepeat)),
            Some(b'{') => {
                if self.braced_quantifier()?.is_some() {
                    return Err(Error {
                        kind: ErrorKind::NothingToRepeat,
                        position: start,
                    });
                }
                if self.unicode {
                    return Err(self.error(ErrorKind::UnexpectedCharacter));
                }
                self.advance(1);
                (self.node(Node::Character(c))?, true)
            }
            Some(b'}' | b']') if self.unicode => {
                return Err(self.error(ErrorKind::UnexpectedCharacter));
            }
            _ => {
                self.advance(width);
                (self.node(Node::Character(c))?, true)
            }
        };
        self.quantifier(node, quantifiable)
    }

    /// Hängt ein Atom an die laufende Alternative, mit seinem Quantor,
    /// falls einer folgt.
    fn quantifier(&mut self, atom: NodeId, quantifiable: bool) -> Result<()> {
        let start = self.position;
        let prefix = match self.unit(0) {
            Some(b'*') => Some((0, None)),
            Some(b'+') => Some((1, None)),
            Some(b'?') => Some((0, Some(1))),
            Some(b'{') => self.braced_quantifier()?,
            _ => None,
        };
        let Some((min, max)) = prefix else {
            self.terms.push(atom);
            return Ok(());
        };
        if !quantifiable {
            return Err(Error {
                kind: ErrorKind::NothingToRepeat,
                position: start,
            });
        }
        if self.position == start {
            self.advance(1);
        }
        let greedy = !self.eat(b'?');
        let node = self.node(Node::Quantified {
            atom,
            min,
            max,
            greedy,
        })?;
        self.terms.push(node);
        Ok(())
    }

    /// `{n}`, `{n,}` oder `{n,m}` an der Lage. Ohne gültige Form bleibt die
    /// Lage, wo sie ist.
    fn braced_quantifier(&mut self) -> Result<Option<(u32, Option<u32>)>> {
        let units = self.units;
        let start = self.position;
        let first_start = start.saturating_add(1);
        let first_end = self.digits_end(first_start);
        if first_end == first_start {
            return Ok(None);
        }
        let first = units.get(first_start..first_end).unwrap_or_default();
        let at = |position: usize| units.get(position).copied().and_then(ascii_unit);
        let (max, end) = match at(first_end) {
            Some(b'}') => (Some(first), first_end),
            Some(b',') => {
                let second_start = first_end.saturating_add(1);
                let second_end = self.digits_end(second_start);
                if at(second_end) != Some(b'}') {
                    return Ok(None);
                }
                if second_end == second_start {
                    (None, second_end)
                } else {
                    (
                        Some(units.get(second_start..second_end).unwrap_or_default()),
                        second_end,
                    )
                }
            }
            _ => return Ok(None),
        };
        if max.is_some_and(|max| greater(first, max)) {
            return Err(Error {
                kind: ErrorKind::QuantifierOutOfOrder,
                position: start,
            });
        }
        let min = decimal(first);
        let max = max.map(decimal);
        self.position = end.saturating_add(1);
        Ok(Some((min, max)))
    }

    /// Wo die Dezimalziffern ab `start` enden.
    fn digits_end(&self, start: usize) -> usize {
        let count = self
            .units
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(|&&unit| is_decimal_digit(unit))
            .count();
        start.saturating_add(count)
    }

    // Escapes

    /// Ein Escape als Atom oder Zusicherung; sagt, ob es einen Quantor
    /// haben darf.
    fn atom_escape(&mut self) -> Result<(NodeId, bool)> {
        let start = self.position;
        self.advance(1);
        if self.position >= self.units.len() {
            return Err(self.error_at(ErrorKind::UnexpectedEnd, start));
        }
        if let Some(escape) = self.unit(0).and_then(class_escape) {
            self.advance(1);
            return Ok((self.node(Node::ClassEscape(escape))?, true));
        }
        let node = match self.unit(0) {
            Some(b'b') => {
                self.advance(1);
                return Ok((self.node(Node::Assertion(Assertion::WordBoundary))?, false));
            }
            Some(b'B') => {
                self.advance(1);
                return Ok((
                    self.node(Node::Assertion(Assertion::NotWordBoundary))?,
                    false,
                ));
            }
            Some(b'1'..=b'9') => self.decimal_escape(start)?,
            Some(b'k') if self.named => {
                self.advance(1);
                if !self.eat(b'<') {
                    return Err(self.error_at(ErrorKind::InvalidGroupName, start));
                }
                let name = self.group_name()?;
                self.references.push((name, start));
                Node::NamedBackreference(name)
            }
            Some(b'p' | b'P') if self.unicode => self.property(start)?.0,
            _ => Node::Character(self.character_escape(false)?),
        };
        Ok((self.node(node)?, true))
    }

    /// `\1` und höher außerhalb von Klassen. Ohne `u` und `v` ist es nur
    /// ein Rückverweis, wenn es so viele Gruppen gibt, sonst ein oktales
    /// Escape oder `8` und `9` als sie selbst (Anhang B).
    fn decimal_escape(&mut self, start: usize) -> Result<Node> {
        let end = self.digits_end(self.position);
        let value = decimal(self.units.get(self.position..end).unwrap_or_default());
        if self.unicode {
            self.position = end;
            self.backreferences.push((value, start));
            return Ok(Node::Backreference(value));
        }
        if value <= self.total_captures {
            self.position = end;
            return Ok(Node::Backreference(value));
        }
        Ok(Node::Character(self.character_escape(false)?))
    }

    /// *CharacterEscape* hinter `\` und sein Wert. Ohne `u` und `v` ergibt
    /// `\c` ohne Buchstaben (in Klassen auch ohne Ziffer und `_`) den
    /// Rückstrich selbst und lässt das `c` stehen (Anhang B).
    fn character_escape(&mut self, in_class: bool) -> Result<u32> {
        let start = self.position.saturating_sub(1);
        let invalid = Error {
            kind: ErrorKind::InvalidEscape,
            position: start,
        };
        let Some((c, width)) = self.peek_with_width() else {
            return Err(self.error_at(ErrorKind::UnexpectedEnd, start));
        };
        let control = match ascii(c) {
            Some(b'f') => Some(0x0C),
            Some(b'n') => Some(0x0A),
            Some(b'r') => Some(0x0D),
            Some(b't') => Some(0x09),
            Some(b'v') => Some(0x0B),
            _ => None,
        };
        if let Some(value) = control {
            self.advance(1);
            return Ok(value);
        }
        match ascii(c) {
            Some(b'c') => match self.unit(1) {
                Some(letter) if letter.is_ascii_alphabetic() => {
                    self.advance(2);
                    Ok(u32::from(letter % 32))
                }
                Some(other)
                    if !self.unicode && in_class && (other.is_ascii_digit() || other == b'_') =>
                {
                    self.advance(2);
                    Ok(u32::from(other % 32))
                }
                _ if !self.unicode => Ok(u32::from(b'\\')),
                _ => Err(invalid),
            },
            Some(b'0') if !self.unit(1).is_some_and(|unit| unit.is_ascii_digit()) => {
                self.advance(1);
                Ok(0)
            }
            Some(b'0'..=b'7') if !self.unicode => Ok(self.legacy_octal()),
            Some(b'x') => match self.hex(1, 2) {
                Some(value) => {
                    self.advance(3);
                    Ok(value)
                }
                None if !self.unicode => {
                    self.advance(1);
                    Ok(c)
                }
                None => Err(invalid),
            },
            Some(b'u') if self.unicode => self.unicode_escape().ok_or(invalid),
            Some(b'u') => match self.hex(1, 4) {
                Some(value) => {
                    self.advance(5);
                    Ok(value)
                }
                None => {
                    self.advance(1);
                    Ok(c)
                }
            },
            // *IdentityEscape*
            _ if self.unicode => {
                if ascii(c).is_some_and(|c| is_syntax_character(c) || c == b'/') {
                    self.advance(1);
                    Ok(c)
                } else {
                    Err(invalid)
                }
            }
            Some(b'k') if self.named => Err(invalid),
            _ => {
                self.advance(width);
                Ok(c)
            }
        }
    }

    /// *LegacyOctalEscapeSequence* (Anhang B): bis zu drei Ziffern, wenn
    /// die erste höchstens 3 ist, sonst bis zu zwei.
    fn legacy_octal(&mut self) -> u32 {
        let limit = if self.unit(0).is_some_and(|unit| unit <= b'3') {
            3
        } else {
            2
        };
        let mut value = 0u32;
        for _ in 0..limit {
            let Some(digit) = self.unit(0).filter(|unit| (b'0'..=b'7').contains(unit)) else {
                break;
            };
            value = value * 8 + u32::from(digit - b'0');
            self.advance(1);
        }
        value
    }

    /// *RegExpUnicodeEscapeSequence[+UnicodeMode]* ab dem `u`: `u{…}` bis
    /// U+10FFFF, `uXXXX` und zwei solche als Ersatzpaar.
    fn unicode_escape(&mut self) -> Option<u32> {
        if self.unit(1) == Some(b'{') {
            let mut end = self.position.saturating_add(2);
            let mut value = 0u32;
            let digits_start = end;
            while let Some(digit) = self.units.get(end).copied().and_then(hex_digit) {
                value = value.saturating_mul(16).saturating_add(digit);
                end = end.saturating_add(1);
            }
            if end == digits_start
                || self.units.get(end).copied().and_then(ascii_unit) != Some(b'}')
                || value > MAX_CODE_POINT
            {
                return None;
            }
            self.position = end.saturating_add(1);
            return Some(value);
        }
        let lead = self.hex(1, 4)?;
        self.advance(5);
        if (0xD800..=0xDBFF).contains(&lead)
            && self.unit(0) == Some(b'\\')
            && self.unit(1) == Some(b'u')
            && let Some(trail) = self
                .hex(2, 4)
                .filter(|trail| (0xDC00..=0xDFFF).contains(trail))
        {
            self.advance(6);
            return Some(0x1_0000 + ((lead - 0xD800) << 10) + (trail - 0xDC00));
        }
        Some(lead)
    }

    /// `count` Hexziffern ab `offset` hinter der Lage.
    fn hex(&self, offset: usize, count: usize) -> Option<u32> {
        let start = self.position.checked_add(offset)?;
        let digits = self.units.get(start..start.checked_add(count)?)?;
        digits
            .iter()
            .try_fold(0u32, |value, &unit| Some(value * 16 + hex_digit(unit)?))
    }

    /// *GroupName* ab dem Namen hinter `<` bis einschließlich `>`. Escapes
    /// im Namen folgen immer den Regeln mit `u`, Ersatzpaare zählen auch
    /// ohne `u` als ein Zeichen.
    fn group_name(&mut self) -> Result<NameId> {
        let start = self.position;
        let invalid = Error {
            kind: ErrorKind::InvalidGroupName,
            position: start,
        };
        let mut name = Vec::new();
        loop {
            let unit = self.units.get(self.position).copied().ok_or(invalid)?;
            if unit == u16::from(b'>') && !name.is_empty() {
                self.advance(1);
                break;
            }
            let c = if unit == u16::from(b'\\') {
                self.advance(1);
                if self.unit(0) != Some(b'u') {
                    return Err(invalid);
                }
                self.unicode_escape().ok_or(invalid)?
            } else {
                let (c, width) = code_point_at(self.units, self.position);
                self.advance(width);
                c
            };
            let valid = if name.is_empty() {
                is_identifier_start(c)
            } else {
                is_identifier_part(c)
            };
            if !valid {
                return Err(invalid);
            }
            push_utf16(&mut name, c);
        }
        self.intern(name)
    }

    fn intern(&mut self, name: Vec<u16>) -> Result<NameId> {
        if let Some(&id) = self.name_ids.get(&name) {
            return Ok(id);
        }
        let id = NameId(u32::try_from(self.names.len()).map_err(|_| self.too_long())?);
        self.names.push(name.clone());
        self.name_ids.insert(name, id);
        self.declared.push(None);
        Ok(id)
    }

    // Aufbau

    fn node(&mut self, node: Node) -> Result<NodeId> {
        let id = NodeId(u32::try_from(self.nodes.len()).map_err(|_| self.too_long())?);
        self.nodes.push(node);
        Ok(id)
    }

    /// Macht aus den Termen ab `start` eine Liste.
    fn take_terms(&mut self, start: usize) -> Result<List> {
        take(&mut self.lists, &mut self.terms, start).ok_or(self.too_long())
    }

    // Lesen

    /// Die Codeeinheit an `offset` hinter der Lage, wenn sie ASCII ist.
    fn unit(&self, offset: usize) -> Option<u8> {
        self.units
            .get(self.position.checked_add(offset)?)
            .copied()
            .and_then(ascii_unit)
    }

    /// Das Zeichen an der Lage: mit `u` oder `v` ein Codepunkt, sonst eine
    /// Codeeinheit.
    fn peek(&self) -> Option<u32> {
        self.peek_with_width().map(|(c, _)| c)
    }

    fn peek_with_width(&self) -> Option<(u32, usize)> {
        let unit = *self.units.get(self.position)?;
        if self.unicode {
            Some(code_point_at(self.units, self.position))
        } else {
            Some((u32::from(unit), 1))
        }
    }

    fn advance(&mut self, units: usize) {
        self.position = self.position.saturating_add(units).min(self.units.len());
    }

    fn eat(&mut self, expected: u8) -> bool {
        let found = self.unit(0) == Some(expected);
        if found {
            self.advance(1);
        }
        found
    }

    fn error(&self, kind: ErrorKind) -> Error {
        self.error_at(kind, self.position)
    }

    fn error_at(&self, kind: ErrorKind, position: usize) -> Error {
        Error { kind, position }
    }

    fn too_long(&self) -> Error {
        self.error(ErrorKind::TooLong)
    }
}

const MAX_CODE_POINT: u32 = 0x10_FFFF;

/// Hängt die Einträge von `stack` ab `start` an `lists` und gibt sie als
/// Liste zurück; ohne Wert, wenn die Liste zu lang würde.
fn take(lists: &mut Vec<NodeId>, stack: &mut Vec<NodeId>, start: usize) -> Option<List> {
    let start = start.min(stack.len());
    let list_start = u32::try_from(lists.len()).ok()?;
    let len = u32::try_from(stack.len() - start).ok()?;
    list_start.checked_add(len)?;
    lists.extend(stack.drain(start..));
    Some(List {
        start: list_start,
        len,
    })
}

/// Der Codepunkt an `position`, Ersatzpaare zusammengefasst, und wie viele
/// Codeeinheiten er belegt.
fn code_point_at(units: &[u16], position: usize) -> (u32, usize) {
    let Some(&unit) = units.get(position) else {
        return (0, 0);
    };
    if (0xD800..=0xDBFF).contains(&unit)
        && let Some(&trail) = units.get(position.saturating_add(1))
        && (0xDC00..=0xDFFF).contains(&trail)
    {
        let value = 0x1_0000 + ((u32::from(unit) - 0xD800) << 10) + (u32::from(trail) - 0xDC00);
        return (value, 2);
    }
    (u32::from(unit), 1)
}

fn push_utf16(out: &mut Vec<u16>, c: u32) {
    let mut buffer = [0; 2];
    match char::from_u32(c) {
        Some(c) => out.extend_from_slice(c.encode_utf16(&mut buffer)),
        // Ein einzelnes Surrogat bleibt eine Codeeinheit.
        None => out.extend(u16::try_from(c).ok()),
    }
}

/// Ein Zeichen als ASCII, falls es eines ist.
fn ascii(c: u32) -> Option<u8> {
    u8::try_from(c).ok().filter(u8::is_ascii)
}

fn ascii_unit(unit: u16) -> Option<u8> {
    ascii(u32::from(unit))
}

fn is_decimal_digit(unit: u16) -> bool {
    (u16::from(b'0')..=u16::from(b'9')).contains(&unit)
}

fn hex_digit(unit: u16) -> Option<u32> {
    char::from_u32(u32::from(unit))?.to_digit(16)
}

/// Der Wert einer Folge von Dezimalziffern, höchstens `u32::MAX`.
fn decimal(digits: &[u16]) -> u32 {
    digits.iter().fold(0u32, |value, &unit| {
        let digit = u32::from(unit.saturating_sub(u16::from(b'0')));
        value.saturating_mul(10).saturating_add(digit)
    })
}

/// Ob eine Folge von Dezimalziffern einen größeren Wert hat als eine
/// andere, ohne Grenze für die Länge.
fn greater(first: &[u16], second: &[u16]) -> bool {
    fn significant(digits: &[u16]) -> &[u16] {
        let zeros = digits
            .iter()
            .take_while(|&&unit| unit == u16::from(b'0'))
            .count();
        digits.get(zeros..).unwrap_or_default()
    }
    let (first, second) = (significant(first), significant(second));
    first
        .len()
        .cmp(&second.len())
        .then_with(|| first.cmp(second))
        == std::cmp::Ordering::Greater
}

/// *SyntaxCharacter*
fn is_syntax_character(c: u8) -> bool {
    matches!(
        c,
        b'^' | b'$'
            | b'\\'
            | b'.'
            | b'*'
            | b'+'
            | b'?'
            | b'('
            | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'|'
    )
}

fn class_escape(unit: u8) -> Option<ClassEscape> {
    Some(match unit {
        b'd' => ClassEscape::Digit,
        b'D' => ClassEscape::NotDigit,
        b's' => ClassEscape::Space,
        b'S' => ClassEscape::NotSpace,
        b'w' => ClassEscape::Word,
        b'W' => ClassEscape::NotWord,
        _ => return None,
    })
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}
