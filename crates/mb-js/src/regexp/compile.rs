//! Übersetzt einen Musterbaum in ein [`Program`]: die Semantik aus
//! Abschnitt 22.2.2 (CompilePattern und die Abschnitte darunter) als Befehle
//! statt als verschachtelte Abschlüsse.
//!
//! - Der Baum wird mit einem Stapel von Aufgaben abgelaufen, nicht rekursiv
//!   (RS-8). Wie tief ein Muster verschachtelt ist, begrenzt nur seine
//!   Länge.
//! - Ein Quantor wird nicht ausgerollt: `a{2,1000}` ist eine Schleife mit
//!   einem Zähler. Das Programm wächst deshalb höchstens linear mit dem
//!   Muster, von den Mengen abgesehen, und die begrenzt [`Budget::bytes`].
//! - In einem Lookbehind stehen die Terme einer Alternative in umgekehrter
//!   Reihenfolge, und jeder Befehl liest rückwärts (MatchSequence mit
//!   ~backward~).
//! - Eine Menge steht im Programm schon als Bild unter Canonicalize; der
//!   Abgleich bildet nur noch das gelesene Zeichen ab (CharacterSetMatcher,
//!   Abschnitt 22.2.2.7.1).

use std::collections::BTreeMap;

use super::charset::{self, CharSet, Folding, Mode, Range};
use super::program::{Budget, Instruction, NONE, Program, Step, WORK_BATCH};
use super::{
    Assertion, ClassOperation, Flags, Lookaround, NameId, Node, NodeId, Pattern, Property, to_usize,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompileError {
    /// Das Muster nennt eine Unicode-Eigenschaft, deren Tabellen
    /// `mb-unicode` noch nicht hat.
    Unsupported,
    /// Das Programm wäre größer als erlaubt.
    TooLarge,
    /// [`Budget::spend`] hat abgebrochen.
    Stopped,
}

type Result<T> = std::result::Result<T, CompileError>;

/// Übersetzt `pattern`, das mit denselben `flags` gelesen wurde.
pub(crate) fn compile(pattern: &Pattern, flags: Flags, budget: Budget<'_>) -> Result<Program> {
    let unicode = flags.unicode || flags.unicode_sets;
    let mut named: BTreeMap<NameId, Vec<u32>> = BTreeMap::new();
    let mut names = Vec::new();
    for group in 1..=pattern.capture_count() {
        let name = pattern.group_name(group);
        if let Some(name) = name {
            named.entry(name).or_default().push(group);
        }
        names.push(name.map(|name| pattern.name(name).to_vec()));
    }
    let mut compiler = Compiler {
        pattern,
        unicode,
        sets_mode: flags.unicode_sets,
        named,
        info: Vec::new(),
        instructions: Vec::new(),
        sets: Vec::new(),
        set_ids: BTreeMap::new(),
        leaves: Vec::new(),
        group_lists: Vec::new(),
        list_ids: BTreeMap::new(),
        registers: 0,
        bytes: 0,
        pending: 0,
        budget,
        tasks: Vec::new(),
        splits: Vec::new(),
        jumps: Vec::new(),
    };
    compiler.gather()?;
    let context = Context {
        backward: false,
        ignore_case: flags.ignore_case,
        multiline: flags.multiline,
        dot_all: flags.dot_all,
    };
    compiler.tasks.push(Task::Node(pattern.root(), context));
    while let Some(task) = compiler.tasks.pop() {
        compiler.run(task)?;
    }
    compiler.emit(Instruction::Match)?;
    if compiler.pending > 0 && !(compiler.budget.spend)(compiler.pending) {
        return Err(CompileError::Stopped);
    }
    Ok(Program {
        instructions: compiler.instructions,
        sets: compiler.sets,
        group_lists: compiler.group_lists,
        groups: pattern.capture_count(),
        names,
        registers: compiler.registers,
        unicode,
    })
}

/// Was beim Übersetzen eines Knotens gilt: die Richtung und die Flags, die
/// eine Gruppe wie `(?i:…)` ändern kann.
#[derive(Clone, Copy)]
struct Context {
    backward: bool,
    ignore_case: bool,
    multiline: bool,
    dot_all: bool,
}

/// Was der Übersetzer vorab über einen Knoten weiß.
#[derive(Clone, Copy, Default)]
struct Info {
    /// Ob der Knoten passen kann, ohne ein Zeichen zu verbrauchen. Im
    /// Zweifel ja: Dann prüft eine Schleife um ihn, ob ein Durchlauf leer
    /// war.
    empty: bool,
    /// Die Gruppen in diesem Knoten: die kleinste Nummer und ihre Zahl. Sie
    /// sind fortlaufend nummeriert, weil die Nummern der Reihenfolge der
    /// öffnenden Klammern folgen.
    first_group: u32,
    groups: u32,
}

impl Info {
    fn with_groups_of(mut self, other: Info) -> Info {
        if other.groups > 0 {
            self.first_group = if self.groups == 0 {
                other.first_group
            } else {
                self.first_group.min(other.first_group)
            };
            self.groups = self.groups.saturating_add(other.groups);
        }
        self
    }
}

enum Task {
    Node(NodeId, Context),
    /// Vor einer Alternative, der noch eine folgt.
    Split,
    /// Hinter einer Alternative, der noch eine folgt.
    Else,
    /// Hinter der letzten Alternative; `jumps` ist die Zahl der offenen
    /// Sprünge davor.
    Join {
        jumps: usize,
    },
    /// Hinter dem Atom eines Quantors.
    Repeat(Repeat),
    /// Hinter dem Rumpf einer Gruppe.
    Capture {
        group: u32,
        register: u32,
    },
    /// Hinter dem Rumpf eines Lookaround, `start` ist sein
    /// [`Instruction::LookStart`].
    Look {
        start: u32,
        negative: bool,
    },
}

#[derive(Clone, Copy)]
enum Repeat {
    /// `+` um ein Atom, das immer etwas verbraucht: `head` ist der Anfang
    /// des Atoms.
    Plus { head: u32, greedy: bool },
    /// Alle anderen Formen. Jeder Eintrag kann fehlen ([`NONE`]).
    Loop {
        /// Wohin der Rumpf zurückspringt; fehlt bei `?`.
        head: u32,
        /// Das [`Instruction::JumpIfReached`], dem das Ziel noch fehlt.
        reached: u32,
        /// Das [`Instruction::Split`], dem das Ziel noch fehlt.
        split: u32,
        /// Das Register mit der Stelle am Anfang des Durchlaufs.
        position: u32,
        counter: u32,
        min: u32,
    },
}

struct Compiler<'a, 'b> {
    pattern: &'a Pattern,
    /// `u` oder `v`
    unicode: bool,
    /// `v`
    sets_mode: bool,
    /// Die Gruppen zu jedem Namen, aufsteigend.
    named: BTreeMap<NameId, Vec<u32>>,
    /// Nach dem Index des Knotens.
    info: Vec<Info>,
    instructions: Vec<Instruction>,
    sets: Vec<Vec<Range>>,
    /// Die Nummer der Menge zu ihren Bereichen vor Canonicalize.
    set_ids: BTreeMap<(Folding, Vec<Range>), u32>,
    /// Die Mengen zu `\d`, `\p{…}` und ihren Verwandten, je nachdem, ob `i`
    /// gilt: Sie kommen aus Tabellen und kehren in Mustern oft wieder.
    leaves: Vec<(Node, bool, CharSet)>,
    group_lists: Vec<Vec<u32>>,
    list_ids: BTreeMap<NameId, u32>,
    registers: u32,
    /// Was Befehle und Mengen bisher belegen.
    bytes: usize,
    /// Arbeit, die [`Budget::spend`] noch nicht gesehen hat.
    pending: u64,
    budget: Budget<'b>,
    tasks: Vec<Task>,
    /// Die offenen [`Instruction::Split`] der Alternativen.
    splits: Vec<u32>,
    /// Die Sprünge ans Ende der offenen Disjunktionen.
    jumps: Vec<u32>,
}

impl Compiler<'_, '_> {
    fn spend(&mut self, work: usize) -> Result<()> {
        let work = u64::try_from(work).unwrap_or(u64::MAX);
        self.pending = self.pending.saturating_add(work);
        if self.pending >= WORK_BATCH {
            let pending = std::mem::take(&mut self.pending);
            if !(self.budget.spend)(pending) {
                return Err(CompileError::Stopped);
            }
        }
        Ok(())
    }

    fn grow(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > self.budget.bytes {
            return Err(CompileError::TooLarge);
        }
        Ok(())
    }

    /// Der Index des nächsten Befehls.
    fn here(&self) -> u32 {
        // `emit` lässt nicht mehr Befehle zu, als in ein u32 passen.
        u32::try_from(self.instructions.len()).unwrap_or(NONE)
    }

    fn emit(&mut self, instruction: Instruction) -> Result<u32> {
        let index = self.here();
        if index >= NONE.saturating_sub(1) {
            return Err(CompileError::TooLarge);
        }
        self.grow(size_of::<Instruction>())?;
        self.instructions.push(instruction);
        Ok(index)
    }

    /// Trägt das Ziel nach, das einem Befehl noch fehlt.
    fn patch(&mut self, index: u32, target: u32) {
        match self.instructions.get_mut(to_usize(index)) {
            Some(
                Instruction::Jump { target: missing }
                | Instruction::JumpIfReached {
                    target: missing, ..
                }
                | Instruction::LookStart { end: missing, .. },
            ) => *missing = target,
            Some(Instruction::Split { first, second }) => {
                if *first == NONE {
                    *first = target;
                } else {
                    *second = target;
                }
            }
            _ => {}
        }
    }

    fn register(&mut self) -> Result<u32> {
        let register = self.registers;
        self.registers = register.checked_add(1).ok_or(CompileError::TooLarge)?;
        if self.registers == NONE {
            return Err(CompileError::TooLarge);
        }
        self.grow(size_of::<u32>())?;
        Ok(register)
    }

    fn info_of(&self, id: NodeId) -> Info {
        self.info.get(id.index()).copied().unwrap_or(Info {
            empty: true,
            ..Info::default()
        })
    }

    fn mode(&self, context: Context) -> Mode {
        Mode {
            unicode: self.unicode,
            sets: self.sets_mode,
            ignore_case: context.ignore_case,
        }
    }

    /// Füllt [`Compiler::info`], die Kinder vor den Eltern.
    fn gather(&mut self) -> Result<()> {
        let pattern = self.pattern;
        self.info = vec![Info::default(); pattern.nodes.len()];
        let mut stack = vec![(pattern.root(), false)];
        while let Some((id, seen)) = stack.pop() {
            let node = pattern.node(id);
            let (list, child) = match node {
                Node::Disjunction(list)
                | Node::Alternative(list)
                | Node::ClassStrings(list)
                | Node::Class { items: list, .. } => (pattern.list(list), None),
                Node::Lookaround { body: child, .. }
                | Node::Quantified { atom: child, .. }
                | Node::Capture { body: child, .. }
                | Node::Group { body: child, .. } => (&[][..], Some(child)),
                _ => (&[][..], None),
            };
            if !seen {
                self.spend(1)?;
                stack.push((id, true));
                stack.extend(list.iter().chain(child.iter()).map(|&child| (child, false)));
                continue;
            }
            let children = || list.iter().chain(child.iter()).map(|&id| self.info_of(id));
            let groups = children().fold(Info::default(), Info::with_groups_of);
            let empty = match node {
                Node::Alternative(_) => children().all(|info| info.empty),
                Node::Disjunction(_) | Node::ClassStrings(_) | Node::Class { .. } => {
                    children().any(|info| info.empty)
                }
                Node::Capture { .. } | Node::Group { .. } => children().all(|info| info.empty),
                Node::Quantified { min, max, .. } => {
                    min == 0 || max == Some(0) || children().all(|info| info.empty)
                }
                Node::Assertion(_)
                | Node::Lookaround { .. }
                | Node::Backreference(_)
                | Node::NamedBackreference(_) => true,
                Node::Property { property, .. } => matches!(property, Property::Strings(_)),
                Node::Character(_) | Node::Dot | Node::ClassEscape(_) | Node::Range { .. } => false,
            };
            let info = match node {
                Node::Capture { index, .. } => Info {
                    empty,
                    first_group: index,
                    groups: groups.groups.saturating_add(1),
                },
                _ => Info { empty, ..groups },
            };
            if let Some(slot) = self.info.get_mut(id.index()) {
                *slot = info;
            }
        }
        Ok(())
    }

    fn run(&mut self, task: Task) -> Result<()> {
        match task {
            Task::Node(id, context) => self.node(id, context),
            Task::Split => {
                let first = self.here().saturating_add(1);
                let split = self.emit(Instruction::Split {
                    first,
                    second: NONE,
                })?;
                self.splits.push(split);
                Ok(())
            }
            Task::Else => {
                let jump = self.emit(Instruction::Jump { target: NONE })?;
                self.jumps.push(jump);
                if let Some(split) = self.splits.pop() {
                    self.patch(split, self.here());
                }
                Ok(())
            }
            Task::Join { jumps } => {
                let here = self.here();
                for jump in self.jumps.split_off(jumps.min(self.jumps.len())) {
                    self.patch(jump, here);
                }
                Ok(())
            }
            Task::Repeat(repeat) => self.repeat_end(repeat),
            Task::Capture { group, register } => {
                self.emit(Instruction::Capture { group, register })?;
                Ok(())
            }
            Task::Look { start, negative } => {
                self.emit(Instruction::LookEnd { negative })?;
                self.patch(start, self.here());
                Ok(())
            }
        }
    }

    fn node(&mut self, id: NodeId, context: Context) -> Result<()> {
        self.spend(1)?;
        let pattern = self.pattern;
        let mode = self.mode(context);
        let step = Step {
            backward: context.backward,
            folding: mode.folding(),
        };
        match pattern.node(id) {
            Node::Disjunction(list) => {
                let Some((&last, others)) = pattern.list(list).split_last() else {
                    return Ok(());
                };
                if others.is_empty() {
                    self.tasks.push(Task::Node(last, context));
                    return Ok(());
                }
                self.tasks.push(Task::Join {
                    jumps: self.jumps.len(),
                });
                self.tasks.push(Task::Node(last, context));
                for &alternative in others.iter().rev() {
                    self.tasks.push(Task::Else);
                    self.tasks.push(Task::Node(alternative, context));
                    self.tasks.push(Task::Split);
                }
            }
            Node::Alternative(list) => {
                // Der Stapel gibt die Aufgaben in umgekehrter Reihenfolge
                // heraus: Vorwärts kommt der erste Term zuletzt darauf.
                let terms = pattern.list(list).iter();
                let task = |&term| Task::Node(term, context);
                if context.backward {
                    self.tasks.extend(terms.map(task));
                } else {
                    self.tasks.extend(terms.rev().map(task));
                }
            }
            Node::Assertion(assertion) => {
                let multiline = context.multiline;
                // WordCharacters (Abschnitt 22.2.2.9.3) kennt die Zeichen
                // außerhalb von ASCII nur mit `i` und `u` oder `v`.
                let folded = self.unicode && context.ignore_case;
                self.emit(match assertion {
                    Assertion::Start => Instruction::LineStart { multiline },
                    Assertion::End => Instruction::LineEnd { multiline },
                    Assertion::WordBoundary => Instruction::WordBoundary {
                        negated: false,
                        folded,
                    },
                    Assertion::NotWordBoundary => Instruction::WordBoundary {
                        negated: true,
                        folded,
                    },
                })?;
            }
            Node::Lookaround { kind, body } => {
                let (negative, backward) = match kind {
                    Lookaround::Ahead => (false, false),
                    Lookaround::NotAhead => (true, false),
                    Lookaround::Behind => (false, true),
                    Lookaround::NotBehind => (true, true),
                };
                let start = self.emit(Instruction::LookStart {
                    negative,
                    end: NONE,
                })?;
                self.tasks.push(Task::Look { start, negative });
                let inner = Context {
                    backward,
                    ..context
                };
                self.tasks.push(Task::Node(body, inner));
            }
            Node::Quantified {
                atom,
                min,
                max,
                greedy,
            } => self.repeat(atom, min, max, greedy, context)?,
            Node::Character(character) => {
                let code = step.folding.canonicalize(character);
                self.emit(Instruction::Character { code, step })?;
            }
            Node::Dot => {
                self.emit(Instruction::Any {
                    line_terminators: context.dot_all,
                    backward: context.backward,
                })?;
            }
            Node::ClassEscape(_) | Node::Property { .. } => {
                let set = self.leaf(pattern.node(id), mode)?;
                self.matcher(&set, false, step)?;
            }
            Node::Class { negated, .. } => {
                let set = self.class(id, mode)?;
                // CompileCharacterClass (Abschnitt 22.2.2.9.1): Mit `v`
                // steckt `^` schon als Komplement in der Menge.
                self.matcher(&set, negated && !self.sets_mode, step)?;
            }
            // Nur in Klassen.
            Node::Range { .. } | Node::ClassStrings(_) => {}
            Node::Backreference(group) => {
                self.emit(Instruction::BackReference { group, step })?;
            }
            Node::NamedBackreference(name) => {
                let groups = self.named.get(&name).map_or(&[][..], Vec::as_slice);
                if let &[group] = groups {
                    self.emit(Instruction::BackReference { group, step })?;
                } else {
                    let list = self.group_list(name)?;
                    self.emit(Instruction::NamedBackReference { list, step })?;
                }
            }
            Node::Capture { index, body, .. } => {
                let register = self.register()?;
                self.emit(Instruction::Position { register })?;
                self.tasks.push(Task::Capture {
                    group: index,
                    register,
                });
                self.tasks.push(Task::Node(body, context));
            }
            Node::Group { add, remove, body } => {
                // UpdateModifiers (Abschnitt 22.2.2.9.7)
                let change = |flag: bool, add: bool, remove: bool| (flag || add) && !remove;
                let inner = Context {
                    backward: context.backward,
                    ignore_case: change(context.ignore_case, add.ignore_case, remove.ignore_case),
                    multiline: change(context.multiline, add.multiline, remove.multiline),
                    dot_all: change(context.dot_all, add.dot_all, remove.dot_all),
                };
                self.tasks.push(Task::Node(body, inner));
            }
        }
        Ok(())
    }

    /// Die Liste der Gruppen mit diesem Namen für `\k<name>`.
    fn group_list(&mut self, name: NameId) -> Result<u32> {
        if let Some(&list) = self.list_ids.get(&name) {
            return Ok(list);
        }
        let groups = self.named.get(&name).cloned().unwrap_or_default();
        let list = u32::try_from(self.group_lists.len()).map_err(|_| CompileError::TooLarge)?;
        self.grow(groups.len().saturating_mul(size_of::<u32>()))?;
        self.group_lists.push(groups);
        self.list_ids.insert(name, list);
        Ok(list)
    }

    // Quantoren

    /// Der Anfang einer Wiederholung (RepeatMatcher, Abschnitt 22.2.2.3.1);
    /// das Ende schreibt [`Compiler::repeat_end`] hinter das Atom.
    ///
    /// ```text
    /// *       L: Split(B, E)   B: atom  Jump L   E:
    /// ?          Split(B, E)   B: atom           E:
    /// +       L: atom  Split(L, E)               E:
    /// {n,m}      SetCounter c
    ///         L: JumpIfReached c, m, E
    ///            JumpIfBelow c, n, B
    ///            Split(B, E)
    ///         B: atom  Increment c  Jump L       E:
    /// ```
    ///
    /// Nicht gierig stehen die Ziele von Split umgekehrt. Vor dem Atom
    /// werden seine Gruppen undefiniert. Kann das Atom leer passen, merkt
    /// sich der Durchlauf seine Stelle und scheitert, wenn er nichts
    /// verbraucht hat und nicht mehr nötig war. Bei `{n}` entfallen
    /// JumpIfBelow, Split und diese Prüfung, ohne Obergrenze JumpIfReached.
    fn repeat(
        &mut self,
        atom: NodeId,
        min: u32,
        max: Option<u32>,
        greedy: bool,
        context: Context,
    ) -> Result<()> {
        if max == Some(0) {
            return Ok(());
        }
        let info = self.info_of(atom);
        let exact = max == Some(min);
        if exact && min == 1 {
            self.tasks.push(Task::Node(atom, context));
            return Ok(());
        }
        if min == 1 && max.is_none() && !info.empty {
            let head = self.here();
            self.reset(info)?;
            self.tasks.push(Task::Repeat(Repeat::Plus { head, greedy }));
            self.tasks.push(Task::Node(atom, context));
            return Ok(());
        }
        let optional = min == 0 && max == Some(1);
        let counted = !(optional || (min == 0 && max.is_none()));
        let mut counter = NONE;
        if counted {
            counter = self.register()?;
            self.emit(Instruction::SetCounter { register: counter })?;
        }
        let head = self.here();
        let mut reached = NONE;
        if let (true, Some(value)) = (counted, max) {
            reached = self.emit(Instruction::JumpIfReached {
                register: counter,
                value,
                target: NONE,
            })?;
        }
        let mut split = NONE;
        if !exact {
            if counted && min > 0 {
                let target = self.here().saturating_add(2);
                self.emit(Instruction::JumpIfBelow {
                    register: counter,
                    value: min,
                    target,
                })?;
            }
            let body = self.here().saturating_add(1);
            split = self.emit(if greedy {
                Instruction::Split {
                    first: body,
                    second: NONE,
                }
            } else {
                Instruction::Split {
                    first: NONE,
                    second: body,
                }
            })?;
        }
        self.reset(info)?;
        let mut position = NONE;
        if info.empty && !exact {
            position = self.register()?;
            self.emit(Instruction::Position { register: position })?;
        }
        self.tasks.push(Task::Repeat(Repeat::Loop {
            head: if optional { NONE } else { head },
            reached,
            split,
            position,
            counter,
            min,
        }));
        self.tasks.push(Task::Node(atom, context));
        Ok(())
    }

    /// Macht die Gruppen eines Atoms vor jedem Durchlauf undefiniert.
    fn reset(&mut self, info: Info) -> Result<()> {
        if info.groups > 0 {
            self.emit(Instruction::ResetCaptures {
                first: info.first_group,
                count: info.groups,
            })?;
        }
        Ok(())
    }

    fn repeat_end(&mut self, repeat: Repeat) -> Result<()> {
        match repeat {
            Repeat::Plus { head, greedy } => {
                let next = self.here().saturating_add(1);
                self.emit(if greedy {
                    Instruction::Split {
                        first: head,
                        second: next,
                    }
                } else {
                    Instruction::Split {
                        first: next,
                        second: head,
                    }
                })?;
            }
            Repeat::Loop {
                head,
                reached,
                split,
                position,
                counter,
                min,
            } => {
                if position != NONE {
                    self.emit(Instruction::EmptyCheck {
                        position,
                        counter,
                        min,
                    })?;
                }
                if counter != NONE {
                    self.emit(Instruction::Increment { register: counter })?;
                }
                if head != NONE {
                    self.emit(Instruction::Jump { target: head })?;
                }
                let end = self.here();
                for missing in [reached, split] {
                    if missing != NONE {
                        self.patch(missing, end);
                    }
                }
            }
        }
        Ok(())
    }

    // Mengen

    /// Die Menge zu `\d`, `\p{…}` und ihren Verwandten, auch als Eintrag
    /// einer Klasse (CompileToCharSet, Abschnitt 22.2.2.9).
    fn leaf(&mut self, node: Node, mode: Mode) -> Result<CharSet> {
        let found = self
            .leaves
            .iter()
            .find(|(known, ignore_case, _)| *known == node && *ignore_case == mode.ignore_case);
        if let Some((_, _, set)) = found {
            let set = set.clone();
            self.spend(set.ranges().len())?;
            return Ok(set);
        }
        let set = match node {
            Node::ClassEscape(escape) => mode.class_escape(escape),
            Node::Property { property, negated } => {
                let set = charset::property(property).ok_or(CompileError::Unsupported)?;
                // Ein Wert von General_Category ohne den Namen der
                // Eigenschaft bleibt ungefaltet.
                let set = match property {
                    Property::GeneralCategory(_) => set,
                    _ => mode.maybe_simple_case_folding(set),
                };
                if negated { mode.complement(&set) } else { set }
            }
            _ => return Ok(CharSet::new()),
        };
        // Die Tabellen von mb-unicode einmal durchzugehen, ist Arbeit, die
        // nicht von der Größe der Menge abhängt.
        self.spend(set.ranges().len().saturating_add(TABLE_WORK))?;
        self.leaves.push((node, mode.ignore_case, set.clone()));
        Ok(set)
    }

    /// Die Menge einer Klasse. Mit `v` ist `^` schon angewandt, ohne `v`
    /// nicht: Dort kehrt der Befehl die Prüfung um.
    fn class(&mut self, id: NodeId, mode: Mode) -> Result<CharSet> {
        enum Work {
            Enter(NodeId),
            Combine {
                negated: bool,
                operation: ClassOperation,
                count: usize,
            },
        }
        let pattern = self.pattern;
        let mut work = vec![Work::Enter(id)];
        let mut values: Vec<CharSet> = Vec::new();
        while let Some(next) = work.pop() {
            let set = match next {
                Work::Enter(id) => match pattern.node(id) {
                    Node::Class {
                        negated,
                        operation,
                        items,
                    } => {
                        let items = pattern.list(items);
                        work.push(Work::Combine {
                            // Ohne `v` gibt es nur die äußere Klasse.
                            negated: negated && mode.sets,
                            operation,
                            count: items.len(),
                        });
                        work.extend(items.iter().rev().map(|&item| Work::Enter(item)));
                        continue;
                    }
                    Node::Character(character) => {
                        mode.maybe_simple_case_folding(CharSet::single(character))
                    }
                    Node::Range { first, last } => {
                        mode.maybe_simple_case_folding(CharSet::from_ranges([(first, last)]))
                    }
                    Node::ClassStrings(strings) => {
                        let mut set = CharSet::new();
                        for &string in pattern.list(strings) {
                            let Node::Alternative(characters) = pattern.node(string) else {
                                continue;
                            };
                            let characters = pattern.list(characters).iter();
                            set.insert_string(
                                characters
                                    .filter_map(|&character| match pattern.node(character) {
                                        Node::Character(character) => Some(character),
                                        _ => None,
                                    })
                                    .collect(),
                            );
                        }
                        mode.maybe_simple_case_folding(set)
                    }
                    node => self.leaf(node, mode)?,
                },
                Work::Combine {
                    negated,
                    operation,
                    count,
                } => {
                    let operands = values.split_off(values.len().saturating_sub(count));
                    self.spend(operands.iter().map(size).sum())?;
                    let set = match operation {
                        ClassOperation::Union => CharSet::union_all(&operands),
                        ClassOperation::Intersection => combine(&operands, CharSet::intersection),
                        ClassOperation::Subtraction => combine(&operands, CharSet::difference),
                    };
                    if negated { mode.complement(&set) } else { set }
                }
            };
            self.spend(size(&set))?;
            values.push(set);
        }
        Ok(values.pop().unwrap_or_default())
    }

    /// Die Nummer der Menge mit diesen Bereichen, im Programm als Bild
    /// unter `folding`.
    fn set(&mut self, ranges: &[Range], folding: Folding) -> Result<u32> {
        let key = (folding, ranges.to_vec());
        if let Some(&set) = self.set_ids.get(&key) {
            return Ok(set);
        }
        let canonical = folding.canonical_ranges(ranges);
        let tables = if folding == Folding::None {
            0
        } else {
            TABLE_WORK
        };
        self.spend(canonical.len().saturating_add(tables))?;
        let set = u32::try_from(self.sets.len()).map_err(|_| CompileError::TooLarge)?;
        // Die Bereiche stehen im Programm und bis zum Ende des Übersetzens
        // noch einmal im Schlüssel.
        let bytes = canonical.len().saturating_add(ranges.len());
        self.grow(bytes.saturating_mul(size_of::<Range>()))?;
        self.sets.push(canonical);
        self.set_ids.insert(key, set);
        Ok(set)
    }

    /// Die Befehle für ein Zeichen aus `set` (CompileAtom, Abschnitt
    /// 22.2.2.7). Enthält die Menge Zeichenketten, was es nur mit `v` gibt,
    /// sind es Alternativen: die Zeichenketten von der längsten zur
    /// kürzesten, dann die einzelnen Zeichen, zuletzt die leere.
    fn matcher(&mut self, set: &CharSet, invert: bool, step: Step) -> Result<()> {
        if set.strings().is_empty() {
            let set = self.set(set.ranges(), step.folding)?;
            self.emit(Instruction::Set { set, invert, step })?;
            return Ok(());
        }
        let mut strings: Vec<&Vec<u32>> = set
            .strings()
            .iter()
            .filter(|string| !string.is_empty())
            .collect();
        strings.sort_by_key(|string| std::cmp::Reverse(string.len()));
        let mut jumps = Vec::new();
        for string in strings {
            let first = self.here().saturating_add(1);
            let split = self.emit(Instruction::Split {
                first,
                second: NONE,
            })?;
            self.spend(string.len())?;
            let mut characters = string.clone();
            if step.backward {
                characters.reverse();
            }
            for character in characters {
                let code = step.folding.canonicalize(character);
                self.emit(Instruction::Character { code, step })?;
            }
            jumps.push(self.emit(Instruction::Jump { target: NONE })?);
            self.patch(split, self.here());
        }
        let singles = self.set(set.ranges(), step.folding)?;
        let singles = Instruction::Set {
            set: singles,
            invert: false,
            step,
        };
        if set.strings().contains(&Vec::new()) {
            let first = self.here().saturating_add(1);
            let split = self.emit(Instruction::Split {
                first,
                second: NONE,
            })?;
            self.emit(singles)?;
            self.patch(split, self.here());
        } else {
            self.emit(singles)?;
        }
        let end = self.here();
        for jump in jumps {
            self.patch(jump, end);
        }
        Ok(())
    }
}

/// Was es ungefähr kostet, eine Tabelle von `mb-unicode` einmal
/// durchzugehen, in Einheiten von [`Budget::spend`].
const TABLE_WORK: usize = 1024;

/// Die Größe einer Menge als Arbeit.
fn size(set: &CharSet) -> usize {
    let strings: usize = set.strings().iter().map(Vec::len).sum();
    set.ranges().len().saturating_add(strings).saturating_add(1)
}

/// Verknüpft die Operanden von links nach rechts.
fn combine(operands: &[CharSet], operation: fn(&CharSet, &CharSet) -> CharSet) -> CharSet {
    let Some((first, others)) = operands.split_first() else {
        return CharSet::new();
    };
    others
        .iter()
        .fold(first.clone(), |set, operand| operation(&set, operand))
}
