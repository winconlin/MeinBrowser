//! Der Übersetzer vom Syntaxbaum zum Bytecode, bisher für Scripts ohne
//! Funktionen, Objekte und Eigenschaftszugriffe. Was er noch nicht kann,
//! meldet er als [`CompileError::Unsupported`] mit der Lage im Quelltext.
//!
//! Bereichsanalyse: Ein Script bindet `var` im globalen Objekt und `let`
//! und `const` auf oberster Ebene im deklarativen Record der globalen
//! Umgebung. Beides legt GlobalDeclarationInstantiation (Abschnitt 16.1.7)
//! zur Laufzeit an, der Übersetzer sammelt dafür die Namen. Bindungen in
//! Blöcken, im Kopf von `for` und in `switch` liegen in Registern, denn
//! ohne Funktionen fängt sie keine Closure. Beim Betreten eines Blocks
//! bekommen sie die Lücke (BlockDeclarationInstantiation, Abschnitt
//! 14.2.3). Gelesen und geschrieben wird mit [`Op::CheckHole`] davor, außer
//! wenn die Stelle im Quelltext hinter der Deklaration liegt und kein
//! `switch` dazwischen springen kann; dann ist die Bindung sicher
//! initialisiert.
//!
//! Abschlusswert: Scripts führen ein Register dafür (UpdateEmpty, Abschnitt
//! 6.2.4.4). Jede Ausdrucksanweisung schreibt ihren Wert hinein. `if`,
//! Schleifen und `switch` liefern nie ~empty~, sondern ohne eigenen Wert
//! *undefined*; sie setzen das Register darum zu Beginn auf *undefined*.
//!
//! Tiefe: Ketten zweistelliger Operatoren baut der Parser in Schleifen,
//! sie werden beliebig tief. Der Übersetzer liest sie mit einem eigenen
//! Stapel. Alles andere begrenzt der Parser mit `MAX_DEPTH`, das übersetzt
//! er rekursiv (RS-8).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use crate::ast::{
    AssignOperator, Ast, Atom, BinaryOperator, DeclarationKind, List, Node, NodeId, UnaryOperator,
    UpdateOperator,
};
use crate::bytecode::{Code, Constant, Emitter, Label, Op, TooLarge};
use crate::value::Value;

/// Warum ein Script nicht übersetzt wurde.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// Etwas, das der Übersetzer noch nicht kann, an dieser Lage im
    /// Quelltext (in Codeeinheiten).
    Unsupported(usize),
    /// Mehr Bytecode, Register oder Konstanten, als ein Befehl erreicht.
    TooLarge,
}

impl From<TooLarge> for CompileError {
    fn from(_: TooLarge) -> CompileError {
        CompileError::TooLarge
    }
}

type Result<T> = std::result::Result<T, CompileError>;

/// Ein übersetztes Script mit den Namen für GlobalDeclarationInstantiation.
#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub code: Code,
    /// VarDeclaredNames ohne Wiederholungen, in der Reihenfolge des
    /// Quelltexts.
    pub var_names: Vec<Vec<u16>>,
    /// Die lexikalischen Deklarationen auf oberster Ebene, `true` für
    /// `const`.
    pub lexical_names: Vec<(Vec<u16>, bool)>,
}

/// Register für `this`.
const THIS: i64 = 0;
/// Register für den Abschlusswert.
const COMPLETION: i64 = 1;

#[derive(Clone, Copy, Debug)]
struct LocalBinding {
    register: u32,
    constant: bool,
    /// Ab hier im Quelltext ist die Bindung initialisiert, wenn kein
    /// `switch` dorthin springt.
    initialized_from: usize,
    in_switch: bool,
}

#[derive(Debug, Default)]
struct BlockScope {
    bindings: BTreeMap<Atom, LocalBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetKind {
    Loop,
    Switch,
    /// Eine Anweisung mit Marke, aus der nur `break` mit dieser Marke
    /// herausführt.
    Labeled,
}

/// Ein Ziel von `break` und `continue`.
#[derive(Debug)]
struct Target {
    kind: TargetKind,
    labels: Vec<Atom>,
    breaks: Vec<Label>,
    continues: Vec<Label>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ConstantKey {
    Number(u64),
    String(Vec<u16>),
    Name(Vec<u16>),
}

/// Eine Referenz (Abschnitt 6.2.5) auf einen Bezeichner, zwischen
/// Auflösen und Schreiben.
#[derive(Clone, Copy, Debug)]
enum Reference {
    Local(LocalBinding, Atom),
    /// Das Register hält, wo [`Op::ResolveGlobal`] den Namen fand.
    Global {
        name: i64,
        register: u32,
    },
}

/// Ein Schritt beim Übersetzen einer Kette zweistelliger Operatoren.
enum Task {
    Visit(NodeId),
    /// Legt den linken Operanden in ein neues Register.
    StoreLeft,
    /// Rechnet mit dem Register und dem rechten Operanden.
    Apply(BinaryOperator, NodeId),
    /// Springt am rechten Operanden vorbei, wenn der linke entscheidet.
    ShortCircuit(BinaryOperator),
    /// Das Ziel des letzten Sprungs.
    Bind,
}

struct Compiler<'a> {
    ast: &'a Ast,
    emitter: Emitter,
    constants: Vec<Constant>,
    constant_index: BTreeMap<ConstantKey, i64>,
    next_register: u32,
    register_count: u32,
    scopes: Vec<BlockScope>,
    targets: Vec<Target>,
    strict: bool,
}

/// Übersetzt ein Script (Abschnitt 16.1).
pub fn compile_script(ast: &Ast) -> Result<Script> {
    if ast.is_module() {
        return Err(CompileError::Unsupported(0));
    }
    let mut compiler = Compiler {
        ast,
        emitter: Emitter::default(),
        constants: Vec::new(),
        constant_index: BTreeMap::new(),
        next_register: 0,
        register_count: 0,
        scopes: Vec::new(),
        targets: Vec::new(),
        strict: ast.is_strict(),
    };
    // `this` und der Abschlusswert.
    compiler.allocate()?;
    compiler.allocate()?;
    let body = ast.body();
    let lexical_names = compiler.top_level_lexical_names(body)?;
    let mut var_names = Vec::new();
    for &item in body {
        compiler.var_declared_names(item, &mut var_names)?;
    }
    for &item in body {
        compiler.statement(item)?;
    }
    compiler.emit(Op::Ldar, &[COMPLETION])?;
    compiler.emit(Op::Return, &[])?;
    let code =
        compiler
            .emitter
            .finish(compiler.constants, compiler.register_count, compiler.strict);
    Ok(Script {
        code,
        var_names,
        lexical_names,
    })
}

impl Compiler<'_> {
    fn unsupported(&self, node: NodeId) -> CompileError {
        CompileError::Unsupported(self.ast.range(node).start)
    }

    fn position(&mut self, node: NodeId) -> Result<()> {
        Ok(self.emitter.set_position(self.ast.range(node).start)?)
    }

    fn emit(&mut self, op: Op, operands: &[i64]) -> Result<()> {
        Ok(self.emitter.emit(op, operands)?)
    }

    fn jump(&mut self, op: Op) -> Result<Label> {
        Ok(self.emitter.jump(op)?)
    }

    fn bind(&mut self, label: Label) -> Result<()> {
        Ok(self.emitter.bind(label)?)
    }

    fn bind_all(&mut self, labels: Vec<Label>) -> Result<()> {
        for label in labels {
            self.bind(label)?;
        }
        Ok(())
    }

    // Register und Konstanten

    fn allocate(&mut self) -> Result<u32> {
        let register = self.next_register;
        self.next_register = register.checked_add(1).ok_or(CompileError::TooLarge)?;
        self.register_count = self.register_count.max(self.next_register);
        Ok(register)
    }

    /// Gibt alle Register ab `mark` zurück. Register werden wie ein Stapel
    /// vergeben.
    fn release(&mut self, mark: u32) {
        self.next_register = mark;
    }

    fn constant(&mut self, constant: Constant) -> Result<i64> {
        let key = match &constant {
            Constant::Number(number) => ConstantKey::Number(number.to_bits()),
            Constant::String(units) => ConstantKey::String(units.clone()),
            Constant::Name(units) => ConstantKey::Name(units.clone()),
        };
        if let Some(&index) = self.constant_index.get(&key) {
            return Ok(index);
        }
        let index = i64::try_from(self.constants.len())
            .ok()
            .filter(|&index| u32::try_from(index).is_ok())
            .ok_or(CompileError::TooLarge)?;
        self.constants.push(constant);
        self.constant_index.insert(key, index);
        Ok(index)
    }

    fn name(&mut self, atom: Atom) -> Result<i64> {
        let units = self.ast.atom(atom).to_vec();
        self.constant(Constant::Name(units))
    }

    fn string(&mut self, atom: Atom) -> Result<i64> {
        let units = self.ast.atom(atom).to_vec();
        self.constant(Constant::String(units))
    }

    // Bereiche

    /// LexicallyDeclaredNames auf oberster Ebene des Scripts mit
    /// IsConstantDeclaration.
    fn top_level_lexical_names(&self, body: &[NodeId]) -> Result<Vec<(Vec<u16>, bool)>> {
        let mut names = Vec::new();
        for &item in body {
            for (atom, constant, _) in self.lexical_declarations(item)? {
                names.push((self.ast.atom(atom).to_vec(), constant));
            }
        }
        Ok(names)
    }

    /// Die lexikalischen Bindungen, die `item` in seinem Block deklariert:
    /// Name, ob `const`, und wo die Deklaration endet.
    fn lexical_declarations(&self, item: NodeId) -> Result<Vec<(Atom, bool, usize)>> {
        let mut bindings = Vec::new();
        match *self.ast.node(item) {
            Node::Variables {
                kind: kind @ (DeclarationKind::Let | DeclarationKind::Const),
                declarators,
            } => {
                for &declarator in self.ast.list(declarators) {
                    let Node::Declarator { target, .. } = *self.ast.node(declarator) else {
                        return Err(self.unsupported(declarator));
                    };
                    let Node::BindingIdentifier(atom) = *self.ast.node(target) else {
                        return Err(self.unsupported(target));
                    };
                    let end = self.ast.range(declarator).end;
                    bindings.push((atom, kind == DeclarationKind::Const, end));
                }
            }
            Node::Variables { .. } => {}
            Node::FunctionDeclaration(_) | Node::ClassDeclaration(_) => {
                return Err(self.unsupported(item));
            }
            _ => {}
        }
        Ok(bindings)
    }

    /// VarDeclaredNames (Abschnitt 8.2.8) der Anweisung, angehängt an
    /// `names`, ohne Wiederholungen.
    fn var_declared_names(&self, node: NodeId, names: &mut Vec<Vec<u16>>) -> Result<()> {
        let mut add = |atom: Atom| {
            let units = self.ast.atom(atom).to_vec();
            if !names.contains(&units) {
                names.push(units);
            }
        };
        match *self.ast.node(node) {
            Node::Variables {
                kind: DeclarationKind::Var,
                declarators,
            } => {
                for &declarator in self.ast.list(declarators) {
                    let Node::Declarator { target, .. } = *self.ast.node(declarator) else {
                        return Err(self.unsupported(declarator));
                    };
                    let Node::BindingIdentifier(atom) = *self.ast.node(target) else {
                        return Err(self.unsupported(target));
                    };
                    add(atom);
                }
            }
            Node::Block(list) => {
                for &item in self.ast.list(list) {
                    self.var_declared_names(item, names)?;
                }
            }
            Node::If {
                consequent,
                alternate,
                ..
            } => {
                self.var_declared_names(consequent, names)?;
                if let Some(alternate) = alternate {
                    self.var_declared_names(alternate, names)?;
                }
            }
            Node::While { body, .. } | Node::DoWhile { body, .. } => {
                self.var_declared_names(body, names)?;
            }
            Node::For { init, body, .. } => {
                if let Some(init) = init {
                    self.var_declared_names(init, names)?;
                }
                self.var_declared_names(body, names)?;
            }
            Node::Labeled { body, .. } => self.var_declared_names(body, names)?,
            Node::Switch { cases, .. } => {
                for &case in self.ast.list(cases) {
                    if let Node::Case { body, .. } = *self.ast.node(case) {
                        for &item in self.ast.list(body) {
                            self.var_declared_names(item, names)?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Betritt einen Block mit den lexikalischen Deklarationen aus `items`
    /// und gibt die Marke für [`Compiler::leave_block`] zurück.
    fn enter_block(&mut self, items: &[NodeId], in_switch: bool) -> Result<u32> {
        let mark = self.next_register;
        let mut scope = BlockScope::default();
        let mut registers = Vec::new();
        for &item in items {
            for (atom, constant, end) in self.lexical_declarations(item)? {
                let register = self.allocate()?;
                registers.push(register);
                scope.bindings.insert(
                    atom,
                    LocalBinding {
                        register,
                        constant,
                        initialized_from: end,
                        in_switch,
                    },
                );
            }
        }
        if !registers.is_empty() {
            self.emit(Op::LdaHole, &[])?;
            for register in registers {
                self.emit(Op::Star, &[i64::from(register)])?;
            }
        }
        self.scopes.push(scope);
        Ok(mark)
    }

    fn leave_block(&mut self, mark: u32) {
        self.scopes.pop();
        self.release(mark);
    }

    fn lookup(&self, atom: Atom) -> Option<LocalBinding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.bindings.get(&atom).copied())
    }

    /// Ob eine Stelle im Quelltext die Bindung vor ihrer Initialisierung
    /// erreichen kann.
    fn needs_hole_check(binding: LocalBinding, position: usize) -> bool {
        binding.in_switch || position < binding.initialized_from
    }

    // Anweisungen

    fn statement(&mut self, node: NodeId) -> Result<()> {
        self.position(node)?;
        match *self.ast.node(node) {
            Node::Expression(expression) => {
                self.expression(expression)?;
                self.emit(Op::Star, &[COMPLETION])
            }
            Node::Variables { kind, declarators } => self.variables(kind, declarators),
            Node::Block(list) => {
                let ast = self.ast;
                let items = ast.list(list);
                let mark = self.enter_block(items, false)?;
                for &item in items {
                    self.statement(item)?;
                }
                self.leave_block(mark);
                Ok(())
            }
            Node::Empty | Node::Debugger => Ok(()),
            Node::If {
                test,
                consequent,
                alternate,
            } => self.if_statement(test, consequent, alternate),
            Node::While { .. } | Node::DoWhile { .. } | Node::For { .. } | Node::Switch { .. } => {
                self.breakable(node, Vec::new())
            }
            Node::Labeled { .. } => self.labeled(node),
            Node::Break(label) => self.jump_to_target(node, label, false),
            Node::Continue(label) => self.jump_to_target(node, label, true),
            Node::Throw(expression) => {
                self.expression(expression)?;
                self.position(node)?;
                self.emit(Op::Throw, &[])
            }
            _ => Err(self.unsupported(node)),
        }
    }

    fn reset_completion(&mut self) -> Result<()> {
        self.emit(Op::LdaUndefined, &[])?;
        self.emit(Op::Star, &[COMPLETION])
    }

    /// `var`, `let` und `const` (Abschnitte 14.3.1.2 und 14.3.2.1).
    fn variables(&mut self, kind: DeclarationKind, declarators: List) -> Result<()> {
        let ast = self.ast;
        for &declarator in ast.list(declarators) {
            let Node::Declarator { target, init } = *self.ast.node(declarator) else {
                return Err(self.unsupported(declarator));
            };
            let Node::BindingIdentifier(atom) = *self.ast.node(target) else {
                return Err(self.unsupported(target));
            };
            match kind {
                DeclarationKind::Var => {
                    // Ohne Initialisierer tut die Deklaration hier nichts.
                    // Mit ihm wird erst der Name aufgelöst, dann gelesen
                    // und geschrieben; Namen aus `var` liegen immer in der
                    // globalen Umgebung.
                    let Some(init) = init else {
                        continue;
                    };
                    let name = self.name(atom)?;
                    let register = self.allocate()?;
                    self.emit(Op::ResolveGlobal, &[name, i64::from(register)])?;
                    self.expression(init)?;
                    self.position(declarator)?;
                    self.emit(Op::StaGlobal, &[name, i64::from(register)])?;
                    self.release(register);
                }
                DeclarationKind::Let | DeclarationKind::Const => {
                    match init {
                        Some(init) => self.expression(init)?,
                        None => self.emit(Op::LdaUndefined, &[])?,
                    }
                    // InitializeReferencedBinding.
                    match self
                        .scopes
                        .last()
                        .and_then(|scope| scope.bindings.get(&atom).copied())
                    {
                        Some(binding) => {
                            let register = i64::from(binding.register);
                            self.emit(Op::Star, &[register])?;
                        }
                        None => {
                            let name = self.name(atom)?;
                            self.emit(Op::InitGlobal, &[name])?;
                        }
                    }
                }
                DeclarationKind::Using | DeclarationKind::AwaitUsing => {
                    return Err(self.unsupported(declarator));
                }
            }
        }
        Ok(())
    }

    /// `if` (Abschnitt 14.6.2).
    fn if_statement(
        &mut self,
        test: NodeId,
        consequent: NodeId,
        alternate: Option<NodeId>,
    ) -> Result<()> {
        self.reset_completion()?;
        self.expression(test)?;
        let otherwise = self.jump(Op::JumpIfFalse)?;
        self.statement(consequent)?;
        match alternate {
            Some(alternate) => {
                let end = self.jump(Op::Jump)?;
                self.bind(otherwise)?;
                self.statement(alternate)?;
                self.bind(end)
            }
            None => self.bind(otherwise),
        }
    }

    /// Anweisungen mit Marken (Abschnitt 14.13). Die Marken einer Schleife
    /// oder eines `switch` gehören zu deren Ziel, sonst bekommt die
    /// Anweisung ein eigenes.
    fn labeled(&mut self, node: NodeId) -> Result<()> {
        let mut labels = Vec::new();
        let mut body = node;
        while let Node::Labeled { label, body: inner } = *self.ast.node(body) {
            labels.push(label);
            body = inner;
        }
        match *self.ast.node(body) {
            Node::While { .. } | Node::DoWhile { .. } | Node::For { .. } | Node::Switch { .. } => {
                self.breakable(body, labels)
            }
            _ => {
                self.targets.push(Target {
                    kind: TargetKind::Labeled,
                    labels,
                    breaks: Vec::new(),
                    continues: Vec::new(),
                });
                self.statement(body)?;
                let target = self.pop_target();
                self.bind_all(target.breaks)
            }
        }
    }

    fn pop_target(&mut self) -> Target {
        self.targets.pop().unwrap_or(Target {
            kind: TargetKind::Labeled,
            labels: Vec::new(),
            breaks: Vec::new(),
            continues: Vec::new(),
        })
    }

    /// Schleifen und `switch` mit ihren Marken.
    fn breakable(&mut self, node: NodeId, labels: Vec<Atom>) -> Result<()> {
        self.position(node)?;
        let kind = if matches!(self.ast.node(node), Node::Switch { .. }) {
            TargetKind::Switch
        } else {
            TargetKind::Loop
        };
        let target = Target {
            kind,
            labels,
            breaks: Vec::new(),
            continues: Vec::new(),
        };
        match *self.ast.node(node) {
            Node::While { test, body } => self.while_loop(target, test, body),
            Node::DoWhile { body, test } => self.do_while_loop(target, body, test),
            Node::For {
                init,
                test,
                update,
                body,
            } => self.for_loop(target, init, test, update, body),
            Node::Switch {
                discriminant,
                cases,
            } => self.switch(target, discriminant, cases),
            _ => Err(self.unsupported(node)),
        }
    }

    /// Übersetzt den Rumpf einer Schleife und bindet ihre Sprünge mit
    /// `continue` an das Ende des Rumpfs.
    fn loop_body(&mut self, target: Target, body: NodeId) -> Result<Target> {
        self.targets.push(target);
        self.statement(body)?;
        let mut target = self.pop_target();
        self.bind_all(std::mem::take(&mut target.continues))?;
        Ok(target)
    }

    /// `while` (Abschnitt 14.7.3.2).
    fn while_loop(&mut self, target: Target, test: NodeId, body: NodeId) -> Result<()> {
        self.reset_completion()?;
        let start = self.emitter.offset();
        self.expression(test)?;
        let exit = self.jump(Op::JumpIfFalse)?;
        let target = self.loop_body(target, body)?;
        self.emitter.jump_loop(start)?;
        self.bind(exit)?;
        self.bind_all(target.breaks)
    }

    /// `do`-`while` (Abschnitt 14.7.2.2).
    fn do_while_loop(&mut self, target: Target, body: NodeId, test: NodeId) -> Result<()> {
        self.reset_completion()?;
        let start = self.emitter.offset();
        let target = self.loop_body(target, body)?;
        self.expression(test)?;
        let exit = self.jump(Op::JumpIfFalse)?;
        self.emitter.jump_loop(start)?;
        self.bind(exit)?;
        self.bind_all(target.breaks)
    }

    /// `for` (Abschnitt 14.7.4.2). Ohne Closures braucht es keine Kopien
    /// der Bindungen je Durchlauf (CreatePerIterationEnvironment).
    fn for_loop(
        &mut self,
        target: Target,
        init: Option<NodeId>,
        test: Option<NodeId>,
        update: Option<NodeId>,
        body: NodeId,
    ) -> Result<()> {
        let mark = match init {
            Some(init) if matches!(self.ast.node(init), Node::Variables { .. }) => {
                let mark = self.enter_block(&[init], false)?;
                self.statement(init)?;
                Some(mark)
            }
            // Der Parser legt einen Ausdruck im Kopf in `Node::Expression`.
            Some(init) => {
                let init = match self.ast.node(init) {
                    Node::Expression(expression) => *expression,
                    _ => init,
                };
                self.expression(init)?;
                None
            }
            None => None,
        };
        self.reset_completion()?;
        let start = self.emitter.offset();
        let exit = match test {
            Some(test) => {
                self.expression(test)?;
                Some(self.jump(Op::JumpIfFalse)?)
            }
            None => None,
        };
        let target = self.loop_body(target, body)?;
        if let Some(update) = update {
            self.expression(update)?;
        }
        self.emitter.jump_loop(start)?;
        if let Some(exit) = exit {
            self.bind(exit)?;
        }
        self.bind_all(target.breaks)?;
        if let Some(mark) = mark {
            self.leave_block(mark);
        }
        Ok(())
    }

    /// `switch` (Abschnitt 14.12.4) mit CaseBlockEvaluation: erst die
    /// Vergleiche aller `case` in ihrer Reihenfolge, dann `default`, dann
    /// die Anweisungen ab dem Treffer.
    fn switch(&mut self, target: Target, discriminant: NodeId, cases: List) -> Result<()> {
        self.expression(discriminant)?;
        let input = self.allocate()?;
        self.emit(Op::Star, &[i64::from(input)])?;
        let ast = self.ast;
        let cases = ast.list(cases);
        let mut items = Vec::new();
        for &case in cases {
            if let Node::Case { body, .. } = *self.ast.node(case) {
                items.extend_from_slice(self.ast.list(body));
            }
        }
        let mark = self.enter_block(&items, true)?;
        self.reset_completion()?;
        let mut tests = Vec::new();
        for &case in cases {
            if let Node::Case {
                test: Some(test), ..
            } = *self.ast.node(case)
            {
                self.expression(test)?;
                self.position(case)?;
                self.emit(Op::TestEqualStrict, &[i64::from(input)])?;
                tests.push(self.jump(Op::JumpIfTrue)?);
            }
        }
        let mut otherwise = Some(self.jump(Op::Jump)?);
        self.targets.push(target);
        let mut tests = tests.into_iter();
        for &case in cases {
            let Node::Case { test, body } = *self.ast.node(case) else {
                return Err(self.unsupported(case));
            };
            let label = match test {
                Some(_) => tests.next(),
                None => otherwise.take(),
            };
            if let Some(label) = label {
                self.bind(label)?;
            }
            for &item in ast.list(body) {
                self.statement(item)?;
            }
        }
        if let Some(otherwise) = otherwise {
            self.bind(otherwise)?;
        }
        let target = self.pop_target();
        self.bind_all(target.breaks)?;
        self.leave_block(mark);
        self.release(input);
        Ok(())
    }

    /// `break` und `continue` (Abschnitte 14.8.2 und 14.9.2). Die frühen
    /// Fehler garantieren ein passendes Ziel.
    fn jump_to_target(
        &mut self,
        node: NodeId,
        label: Option<Atom>,
        is_continue: bool,
    ) -> Result<()> {
        let index = self.targets.iter().rposition(|target| {
            let kind_fits = if is_continue {
                target.kind == TargetKind::Loop
            } else {
                label.is_some() || target.kind != TargetKind::Labeled
            };
            kind_fits && label.is_none_or(|label| target.labels.contains(&label))
        });
        let Some(index) = index else {
            return Err(self.unsupported(node));
        };
        let jump = self.jump(Op::Jump)?;
        if let Some(target) = self.targets.get_mut(index) {
            if is_continue {
                target.continues.push(jump);
            } else {
                target.breaks.push(jump);
            }
        }
        Ok(())
    }

    // Ausdrücke

    fn expression(&mut self, node: NodeId) -> Result<()> {
        self.position(node)?;
        match *self.ast.node(node) {
            Node::Paren(inner) => self.expression(inner),
            Node::Number(number) => match Value::number(number).as_int32() {
                Some(small) => self.emit(Op::LdaSmi, &[i64::from(small)]),
                None => {
                    let constant = self.constant(Constant::Number(number))?;
                    self.emit(Op::LdaConstant, &[constant])
                }
            },
            Node::String(atom) => {
                let constant = self.string(atom)?;
                self.emit(Op::LdaConstant, &[constant])
            }
            Node::Boolean(true) => self.emit(Op::LdaTrue, &[]),
            Node::Boolean(false) => self.emit(Op::LdaFalse, &[]),
            Node::Null => self.emit(Op::LdaNull, &[]),
            Node::This => self.emit(Op::Ldar, &[THIS]),
            Node::Identifier(atom) => self.load_identifier(node, atom),
            Node::Template {
                quasis,
                expressions,
            } => self.template(node, quasis, expressions),
            Node::Unary { operator, argument } => self.unary(node, operator, argument),
            Node::Update {
                operator,
                prefix,
                argument,
            } => self.update(node, operator, prefix, argument),
            Node::Binary { .. } => self.binary(node),
            Node::Conditional {
                test,
                consequent,
                alternate,
            } => {
                self.expression(test)?;
                let otherwise = self.jump(Op::JumpIfFalse)?;
                self.expression(consequent)?;
                let end = self.jump(Op::Jump)?;
                self.bind(otherwise)?;
                self.expression(alternate)?;
                self.bind(end)
            }
            Node::Assign {
                operator,
                target,
                value,
            } => self.assign(node, operator, target, value),
            Node::Sequence(list) => {
                let ast = self.ast;
                for &expression in ast.list(list) {
                    self.expression(expression)?;
                }
                Ok(())
            }
            _ => Err(self.unsupported(node)),
        }
    }

    /// Der Bezeichner hinter Klammern, denn auch geklammert bleibt er eine
    /// Referenz.
    fn identifier_reference(&self, node: NodeId) -> Option<Atom> {
        let mut node = node;
        loop {
            match *self.ast.node(node) {
                Node::Paren(inner) => node = inner,
                Node::Identifier(atom) => return Some(atom),
                _ => return None,
            }
        }
    }

    fn load_local(&mut self, binding: LocalBinding, atom: Atom, position: usize) -> Result<()> {
        let register = i64::from(binding.register);
        if Self::needs_hole_check(binding, position) {
            let name = self.name(atom)?;
            self.emit(Op::CheckHole, &[register, name])?;
        }
        self.emit(Op::Ldar, &[register])
    }

    /// PutValue für eine Bindung im Register (SetMutableBinding, Abschnitt
    /// 9.1.1.1.5): erst die TDZ, dann die Konstante.
    fn store_local(&mut self, binding: LocalBinding, atom: Atom, position: usize) -> Result<()> {
        let register = i64::from(binding.register);
        if Self::needs_hole_check(binding, position) || binding.constant {
            let name = self.name(atom)?;
            self.emit(Op::CheckHole, &[register, name])?;
            if binding.constant {
                return self.emit(Op::ThrowConstAssignment, &[name]);
            }
        }
        self.emit(Op::Star, &[register])
    }

    /// Ein Bezeichner als Ausdruck: ResolveBinding und GetValue.
    fn load_identifier(&mut self, node: NodeId, atom: Atom) -> Result<()> {
        match self.lookup(atom) {
            Some(binding) => self.load_local(binding, atom, self.ast.range(node).start),
            None => {
                let name = self.name(atom)?;
                self.emit(Op::LdaGlobal, &[name])
            }
        }
    }

    /// Wertet das Ziel einer Zuweisung zur Referenz aus.
    fn reference(&mut self, target: NodeId) -> Result<Reference> {
        let Some(atom) = self.identifier_reference(target) else {
            return Err(self.unsupported(target));
        };
        if let Some(binding) = self.lookup(atom) {
            return Ok(Reference::Local(binding, atom));
        }
        let name = self.name(atom)?;
        let register = self.allocate()?;
        self.emit(Op::ResolveGlobal, &[name, i64::from(register)])?;
        Ok(Reference::Global { name, register })
    }

    fn get_value(&mut self, reference: Reference, position: usize) -> Result<()> {
        match reference {
            Reference::Local(binding, atom) => self.load_local(binding, atom, position),
            Reference::Global { name, register } => {
                self.emit(Op::LdaGlobalRef, &[name, i64::from(register)])
            }
        }
    }

    fn put_value(&mut self, reference: Reference, position: usize) -> Result<()> {
        match reference {
            Reference::Local(binding, atom) => self.store_local(binding, atom, position),
            Reference::Global { name, register } => {
                self.emit(Op::StaGlobal, &[name, i64::from(register)])
            }
        }
    }

    fn release_reference(&mut self, reference: Reference) {
        if let Reference::Global { register, .. } = reference {
            self.release(register);
        }
    }

    /// Zuweisungen (Abschnitt 13.15.2).
    fn assign(
        &mut self,
        node: NodeId,
        operator: AssignOperator,
        target: NodeId,
        value: NodeId,
    ) -> Result<()> {
        let position = self.ast.range(node).start;
        let reference = self.reference(target)?;
        let binary = match operator {
            AssignOperator::Assign => None,
            AssignOperator::LogicalAnd | AssignOperator::LogicalOr | AssignOperator::Coalesce => {
                self.get_value(reference, position)?;
                let skip = self.jump(match operator {
                    AssignOperator::LogicalAnd => Op::JumpIfFalse,
                    AssignOperator::LogicalOr => Op::JumpIfTrue,
                    _ => Op::JumpIfNotNullish,
                })?;
                self.expression(value)?;
                self.position(node)?;
                self.put_value(reference, position)?;
                self.bind(skip)?;
                self.release_reference(reference);
                return Ok(());
            }
            AssignOperator::Add => Some(Op::Add),
            AssignOperator::Subtract => Some(Op::Sub),
            AssignOperator::Multiply => Some(Op::Mul),
            AssignOperator::Divide => Some(Op::Div),
            AssignOperator::Remainder => Some(Op::Mod),
            AssignOperator::Exponent => Some(Op::Exp),
            AssignOperator::ShiftLeft => Some(Op::ShiftLeft),
            AssignOperator::ShiftRight => Some(Op::ShiftRight),
            AssignOperator::UnsignedShiftRight => Some(Op::ShiftRightLogical),
            AssignOperator::BitwiseOr => Some(Op::BitwiseOr),
            AssignOperator::BitwiseXor => Some(Op::BitwiseXor),
            AssignOperator::BitwiseAnd => Some(Op::BitwiseAnd),
        };
        match binary {
            None => {
                self.expression(value)?;
                self.position(node)?;
            }
            Some(op) => {
                self.get_value(reference, position)?;
                let left = self.allocate()?;
                self.emit(Op::Star, &[i64::from(left)])?;
                self.expression(value)?;
                self.position(node)?;
                self.emit(op, &[i64::from(left)])?;
                self.release(left);
            }
        }
        self.put_value(reference, position)?;
        self.release_reference(reference);
        Ok(())
    }

    /// `++` und `--` davor und dahinter (Abschnitte 13.4.2 bis 13.4.5).
    fn update(
        &mut self,
        node: NodeId,
        operator: UpdateOperator,
        prefix: bool,
        argument: NodeId,
    ) -> Result<()> {
        let position = self.ast.range(node).start;
        let reference = self.reference(argument)?;
        self.get_value(reference, position)?;
        self.position(node)?;
        self.emit(Op::ToNumeric, &[])?;
        let old = if prefix {
            None
        } else {
            let old = self.allocate()?;
            self.emit(Op::Star, &[i64::from(old)])?;
            Some(old)
        };
        self.emit(
            match operator {
                UpdateOperator::Increment => Op::Inc,
                UpdateOperator::Decrement => Op::Dec,
            },
            &[],
        )?;
        self.put_value(reference, position)?;
        if let Some(old) = old {
            self.emit(Op::Ldar, &[i64::from(old)])?;
            self.release(old);
        }
        self.release_reference(reference);
        Ok(())
    }

    /// Einstellige Operatoren (Abschnitte 13.5.1 bis 13.5.7).
    fn unary(&mut self, node: NodeId, operator: UnaryOperator, argument: NodeId) -> Result<()> {
        match operator {
            UnaryOperator::Typeof => {
                if let Some(atom) = self.identifier_reference(argument) {
                    // Ein unbekannter Name ergibt "undefined" statt eines
                    // ReferenceError, die TDZ gilt aber.
                    match self.lookup(atom) {
                        Some(binding) => {
                            self.load_local(binding, atom, self.ast.range(argument).start)?;
                        }
                        None => {
                            let name = self.name(atom)?;
                            return self.emit(Op::TypeofGlobal, &[name]);
                        }
                    }
                } else {
                    self.expression(argument)?;
                }
                self.position(node)?;
                self.emit(Op::TypeOf, &[])
            }
            UnaryOperator::Delete => self.delete(argument),
            UnaryOperator::Void => {
                self.expression(argument)?;
                self.emit(Op::LdaUndefined, &[])
            }
            UnaryOperator::Not
            | UnaryOperator::Minus
            | UnaryOperator::Plus
            | UnaryOperator::BitwiseNot => {
                self.expression(argument)?;
                self.position(node)?;
                self.emit(
                    match operator {
                        UnaryOperator::Not => Op::LogicalNot,
                        UnaryOperator::Minus => Op::Negate,
                        UnaryOperator::Plus => Op::ToNumber,
                        _ => Op::BitwiseNot,
                    },
                    &[],
                )
            }
        }
    }

    /// `delete` (Abschnitt 13.5.1.2). Bindungen aus Deklarationen in
    /// Blöcken lassen sich nicht löschen. Was keine Referenz ergibt, wird
    /// nur ausgewertet.
    fn delete(&mut self, argument: NodeId) -> Result<()> {
        if let Some(atom) = self.identifier_reference(argument) {
            if self.lookup(atom).is_some() {
                return self.emit(Op::LdaFalse, &[]);
            }
            let name = self.name(atom)?;
            return self.emit(Op::DeleteGlobal, &[name]);
        }
        let mut inner = argument;
        while let Node::Paren(next) = *self.ast.node(inner) {
            inner = next;
        }
        if matches!(
            self.ast.node(inner),
            Node::Member { .. }
                | Node::ComputedMember { .. }
                | Node::PrivateMember { .. }
                | Node::SuperMember(_)
                | Node::SuperComputedMember(_)
        ) {
            return Err(self.unsupported(inner));
        }
        self.expression(argument)?;
        self.emit(Op::LdaTrue, &[])
    }

    /// Templates ohne Tag (Abschnitt 13.2.8.6): die Teile und die Werte der
    /// Ausdrücke nach ToString, von links nach rechts verkettet.
    fn template(&mut self, node: NodeId, quasis: List, expressions: List) -> Result<()> {
        let ast = self.ast;
        let quasis = ast.list(quasis);
        let expressions = ast.list(expressions);
        let mut cooked = Vec::new();
        for &quasi in quasis {
            let Node::TemplateElement {
                cooked: Some(atom), ..
            } = *self.ast.node(quasi)
            else {
                return Err(self.unsupported(quasi));
            };
            cooked.push(atom);
        }
        let mut parts = cooked.into_iter();
        let Some(head) = parts.next() else {
            return Err(self.unsupported(node));
        };
        let constant = self.string(head)?;
        self.emit(Op::LdaConstant, &[constant])?;
        if expressions.is_empty() {
            return Ok(());
        }
        let text = self.allocate()?;
        for &expression in expressions {
            self.emit(Op::Star, &[i64::from(text)])?;
            self.expression(expression)?;
            self.position(expression)?;
            self.emit(Op::ToString, &[])?;
            self.emit(Op::Add, &[i64::from(text)])?;
            if let Some(part) = parts.next()
                && !self.ast.atom(part).is_empty()
            {
                self.emit(Op::Star, &[i64::from(text)])?;
                let constant = self.string(part)?;
                self.emit(Op::LdaConstant, &[constant])?;
                self.emit(Op::Add, &[i64::from(text)])?;
            }
        }
        self.release(text);
        Ok(())
    }

    /// Zweistellige Operatoren (Abschnitte 13.6 bis 13.13), mit eigenem
    /// Stapel statt Rekursion.
    fn binary(&mut self, node: NodeId) -> Result<()> {
        let mut tasks = vec![Task::Visit(node)];
        let mut registers = Vec::new();
        let mut labels = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(node) => {
                    let Node::Binary {
                        operator,
                        left,
                        right,
                    } = *self.ast.node(node)
                    else {
                        self.expression(node)?;
                        continue;
                    };
                    self.position(node)?;
                    match operator {
                        BinaryOperator::LogicalAnd
                        | BinaryOperator::LogicalOr
                        | BinaryOperator::Coalesce => {
                            tasks.push(Task::Bind);
                            tasks.push(Task::Visit(right));
                            tasks.push(Task::ShortCircuit(operator));
                        }
                        BinaryOperator::In | BinaryOperator::Instanceof => {
                            return Err(self.unsupported(node));
                        }
                        _ => {
                            tasks.push(Task::Apply(operator, node));
                            tasks.push(Task::Visit(right));
                            tasks.push(Task::StoreLeft);
                        }
                    }
                    tasks.push(Task::Visit(left));
                }
                Task::StoreLeft => {
                    let register = self.allocate()?;
                    self.emit(Op::Star, &[i64::from(register)])?;
                    registers.push(register);
                }
                Task::Apply(operator, node) => {
                    let Some(register) = registers.pop() else {
                        return Err(self.unsupported(node));
                    };
                    self.position(node)?;
                    self.binary_operation(node, operator, register)?;
                    self.release(register);
                }
                Task::ShortCircuit(operator) => {
                    labels.push(self.jump(match operator {
                        BinaryOperator::LogicalAnd => Op::JumpIfFalse,
                        BinaryOperator::LogicalOr => Op::JumpIfTrue,
                        _ => Op::JumpIfNotNullish,
                    })?);
                }
                Task::Bind => {
                    if let Some(label) = labels.pop() {
                        self.bind(label)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Rechnet mit dem Register links und dem Akkumulator rechts.
    fn binary_operation(
        &mut self,
        node: NodeId,
        operator: BinaryOperator,
        register: u32,
    ) -> Result<()> {
        let register = i64::from(register);
        let (op, negate) = match operator {
            BinaryOperator::Equal => (Op::TestEqual, false),
            BinaryOperator::NotEqual => (Op::TestEqual, true),
            BinaryOperator::StrictEqual => (Op::TestEqualStrict, false),
            BinaryOperator::StrictNotEqual => (Op::TestEqualStrict, true),
            BinaryOperator::Less => (Op::TestLessThan, false),
            BinaryOperator::LessEqual => (Op::TestLessThanOrEqual, false),
            BinaryOperator::Greater => (Op::TestGreaterThan, false),
            BinaryOperator::GreaterEqual => (Op::TestGreaterThanOrEqual, false),
            BinaryOperator::ShiftLeft => (Op::ShiftLeft, false),
            BinaryOperator::ShiftRight => (Op::ShiftRight, false),
            BinaryOperator::UnsignedShiftRight => (Op::ShiftRightLogical, false),
            BinaryOperator::Add => (Op::Add, false),
            BinaryOperator::Subtract => (Op::Sub, false),
            BinaryOperator::Multiply => (Op::Mul, false),
            BinaryOperator::Divide => (Op::Div, false),
            BinaryOperator::Remainder => (Op::Mod, false),
            BinaryOperator::Exponent => (Op::Exp, false),
            BinaryOperator::BitwiseOr => (Op::BitwiseOr, false),
            BinaryOperator::BitwiseXor => (Op::BitwiseXor, false),
            BinaryOperator::BitwiseAnd => (Op::BitwiseAnd, false),
            BinaryOperator::In
            | BinaryOperator::Instanceof
            | BinaryOperator::LogicalAnd
            | BinaryOperator::LogicalOr
            | BinaryOperator::Coalesce => return Err(self.unsupported(node)),
        };
        self.emit(op, &[register])?;
        if negate {
            self.emit(Op::LogicalNot, &[])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
