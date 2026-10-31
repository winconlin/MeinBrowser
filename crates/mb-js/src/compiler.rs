//! Der Übersetzer vom Syntaxbaum zum Bytecode, für Scripts und Funktionen
//! ohne Closures. Was er noch nicht kann, meldet er als
//! [`CompileError::Unsupported`] mit der Lage im Quelltext.
//!
//! Funktionen übersetzt er einzeln mit [`compile_function`], erst wenn sie
//! das erste Mal aufgerufen werden. Der Code, der sie anlegt, nennt sie mit
//! einer [`FunctionTemplate`].
//!
//! Bereichsanalyse: Ein Script bindet `var` und Funktionsdeklarationen im
//! globalen Objekt und `let` und `const` auf oberster Ebene im deklarativen
//! Record der globalen Umgebung. Beides legt
//! GlobalDeclarationInstantiation (Abschnitt 16.1.7) zur Laufzeit an, der
//! Übersetzer sammelt dafür die Namen. Alle anderen Bindungen liegen in
//! Registern: in Funktionen die Parameter, `var`, Funktionsdeklarationen
//! und `let` und `const` auf oberster Ebene (FunctionDeclarationInstantiation,
//! Abschnitt 10.2.11), überall die Bindungen in Blöcken, im Kopf von `for`
//! und in `switch`. Beim Betreten eines Blocks bekommen sie die Lücke
//! (BlockDeclarationInstantiation, Abschnitt 14.2.3). Gelesen und
//! geschrieben wird mit [`Op::CheckHole`] davor, außer wenn die Stelle im
//! Quelltext hinter der Deklaration liegt und kein `switch` dazwischen
//! springen kann; dann ist die Bindung sicher initialisiert.
//!
//! Closures gibt es noch nicht: Greift eine Funktion auf eine Bindung zu,
//! die der umgebende Code in einem Register hält, meldet der Übersetzer
//! das als nicht unterstützt. Ebenso `arguments`, Pfeilfunktionen,
//! Generatoren, `async`, Klassen, Parameter mit Vorgabewerten, Mustern oder
//! Rest und Funktionsdeklarationen in Blöcken (Anhang B.3.2).
//!
//! Abschlusswert: Scripts führen ein Register dafür (UpdateEmpty, Abschnitt
//! 6.2.4.4). Jede Ausdrucksanweisung schreibt ihren Wert hinein. `if`,
//! Schleifen und `switch` liefern nie ~empty~, sondern ohne eigenen Wert
//! *undefined*; sie setzen das Register darum zu Beginn auf *undefined*.
//! Funktionen brauchen keinen Abschlusswert.
//!
//! Eigenschaften: Eine Referenz auf eine Eigenschaft hält ihre Basis und
//! bei berechneten Namen den Namen in Registern. ToObject und
//! ToPropertyKey kommen erst beim Lesen, Schreiben oder Löschen
//! (Abschnitte 6.2.5.5 und 6.2.5.6), also nach der rechten Seite einer
//! Zuweisung.
//!
//! Aufrufe: Funktion, `this` und Argumente liegen in Registern
//! hintereinander ([`Op::Call`]). Ist die Funktion eine Eigenschaft, ist
//! ihre Basis `this`; das gilt auch in Klammern, `(a.b)()` ruft mit `a` als
//! `this` auf.
//!
//! Tiefe: Ketten zweistelliger Operatoren und Ketten von Zugriffen und
//! Aufrufen wie `a.b(c)[d]?.e` baut der Parser in Schleifen, sie werden
//! beliebig tief. Der Übersetzer liest sie mit einem eigenen Stapel oder
//! entlang der Kette. Alles andere begrenzt der Parser mit `MAX_DEPTH`, das
//! übersetzt er rekursiv (RS-8).

#![forbid(unsafe_code)]

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::ast::{
    AssignOperator, Ast, Atom, BinaryOperator, DeclarationKind, FunctionId, FunctionKind, List,
    Node, NodeId, PropertyKind, UnaryOperator, UpdateOperator,
};
use crate::bytecode::{Code, Constant, Emitter, FunctionTemplate, Label, Op, TooLarge};
use crate::number;
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
    /// VarDeclaredNames ohne Wiederholungen und ohne die Namen der
    /// Funktionen, in der Reihenfolge des Quelltexts (declaredVarNames).
    pub var_names: Vec<Vec<u16>>,
    /// Die lexikalischen Deklarationen auf oberster Ebene, `true` für
    /// `const`.
    pub lexical_names: Vec<(Vec<u16>, bool)>,
    /// Die Funktionsdeklarationen auf oberster Ebene mit ihrem Namen, von
    /// mehreren mit demselben Namen die letzte, in der Reihenfolge des
    /// Quelltexts (functionsToInitialize).
    pub functions: Vec<(Vec<u16>, FunctionId)>,
}

/// Register für `this`.
const THIS: i64 = 0;
/// Register für den Abschlusswert eines Scripts.
const COMPLETION: i64 = 1;

/// Wie eine Bindung in einem Register auf Zuweisungen reagiert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingKind {
    Mutable,
    /// Aus `const`: Zuweisen wirft einen TypeError.
    Const,
    /// Der Name eines benannten Funktionsausdrucks in seinem Rumpf
    /// (Abschnitt 15.2.5): Zuweisen wirft nur im strikten Modus einen
    /// TypeError, sonst geschieht nichts (SetMutableBinding, Abschnitt
    /// 9.1.1.1.5).
    FunctionName,
}

#[derive(Clone, Copy, Debug)]
struct LocalBinding {
    register: u32,
    kind: BindingKind,
    /// Ab hier im Quelltext ist die Bindung initialisiert, wenn kein
    /// `switch` dorthin springt.
    initialized_from: usize,
    in_switch: bool,
}

impl LocalBinding {
    /// Eine Bindung, die von Anfang an einen Wert hat: Parameter, `var`,
    /// Funktionen und der Name eines Funktionsausdrucks.
    fn initialized(register: u32, kind: BindingKind) -> LocalBinding {
        LocalBinding {
            register,
            kind,
            initialized_from: 0,
            in_switch: false,
        }
    }
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

/// Eine Referenz (Abschnitt 6.2.5) zwischen Auswerten und Schreiben.
#[derive(Clone, Copy, Debug)]
enum Reference {
    Local(LocalBinding, Atom),
    /// Das Register hält, wo [`Op::ResolveGlobal`] den Namen fand.
    Global {
        name: i64,
        register: u32,
    },
    /// Eine Eigenschaft mit festem Namen, die Basis im Register.
    Named {
        base: u32,
        name: i64,
    },
    /// Eine Eigenschaft mit berechnetem Namen. Basis und Name liegen in
    /// zwei Registern hintereinander; das erste Lesen ersetzt den Namen
    /// durch den Schlüssel aus ToPropertyKey.
    Keyed {
        base: u32,
        key: u32,
    },
}

/// Was ein Glied einer Kette von Zugriffen liest.
#[derive(Clone, Copy)]
enum Access {
    /// `.name`
    Named(Atom),
    /// `[expression]`
    Keyed(NodeId),
}

/// Wie eine Kette aus Zugriffen und Aufrufen endet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChainEnd {
    /// Mit ihrem Wert.
    Value,
    /// Mit `delete` auf den letzten Zugriff.
    Delete,
    /// Ihr Wert wird aufgerufen: Ein Zugriff am Ende legt seine Basis als
    /// `this` in das Register.
    Callee(u32),
}

/// Der Name, den ein neues Funktionsobjekt bekommt, wenn die Funktion
/// keinen eigenen hat (NamedEvaluation, Abschnitt 8.4.5).
#[derive(Clone, Copy)]
enum ClosureName<'n> {
    Static(&'n [u16]),
    /// Der Schlüssel im Register.
    Keyed(u32),
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
    /// Das Register für den Abschlusswert, nur in Scripts.
    completion: Option<i64>,
    /// Ob `arguments` ohne eigene Bindung das Objekt der Argumente meint,
    /// das es noch nicht gibt.
    arguments: bool,
    /// Die Namen, die der umgebende Code in Registern hält.
    outer: Rc<BTreeSet<Atom>>,
}

/// Übersetzt ein Script (Abschnitt 16.1).
pub fn compile_script(ast: &Ast) -> Result<Script> {
    if ast.is_module() {
        return Err(CompileError::Unsupported(0));
    }
    let mut compiler = Compiler::new(ast, ast.is_strict(), Rc::default());
    // `this` und der Abschlusswert.
    compiler.allocate()?;
    compiler.allocate()?;
    compiler.completion = Some(COMPLETION);
    let body = ast.body();
    let lexical_names = compiler.top_level_lexical_names(body)?;
    let functions = compiler.top_level_functions(body)?;
    let mut var_atoms = Vec::new();
    for &item in body {
        compiler.var_declared_names(item, &mut var_atoms)?;
    }
    let var_names = var_atoms
        .into_iter()
        .filter(|&atom| !functions.iter().any(|&(name, _, _)| name == atom))
        .map(|atom| ast.atom(atom).to_vec())
        .collect();
    for &item in body {
        compiler.top_level_item(item)?;
    }
    compiler.emit(Op::Ldar, &[COMPLETION])?;
    compiler.emit(Op::Return, &[])?;
    let functions = functions
        .into_iter()
        .map(|(name, id, _)| (ast.atom(name).to_vec(), id))
        .collect();
    Ok(Script {
        code: compiler.finish(),
        var_names,
        lexical_names,
        functions,
    })
}

/// Übersetzt eine Funktion für ihren ersten Aufruf, mit
/// FunctionDeclarationInstantiation (Abschnitt 10.2.11) für einfache
/// Parameterlisten. Die Register: `this`, das Funktionsobjekt, die
/// Parameter, dann `var` und die Funktionen des Rumpfs, dann seine
/// lexikalischen Bindungen.
pub fn compile_function(ast: &Ast, template: &FunctionTemplate) -> Result<Code> {
    let Some(function) = ast.function(template.function) else {
        return Err(CompileError::Unsupported(0));
    };
    let position = function.range.start;
    let supported = matches!(
        function.kind,
        FunctionKind::Normal | FunctionKind::Method | FunctionKind::Getter | FunctionKind::Setter
    ) && !function.generator
        && !function.is_async
        && function.simple_parameters;
    if !supported {
        return Err(CompileError::Unsupported(position));
    }
    let mut compiler = Compiler::new(ast, function.strict, Rc::clone(&template.outer));
    compiler.allocate()?;
    let callee = compiler.allocate()?;
    if template.binds_name
        && let Some(name) = function.name
    {
        let mut scope = BlockScope::default();
        let binding = LocalBinding::initialized(callee, BindingKind::FunctionName);
        scope.bindings.insert(name, binding);
        compiler.scopes.push(scope);
    }
    let mut scope = BlockScope::default();
    let parameters = ast.list(function.parameters);
    for &parameter in parameters {
        let Node::BindingIdentifier(atom) = *ast.node(parameter) else {
            return Err(compiler.unsupported(parameter));
        };
        // Bei doppelten Namen gilt der letzte Parameter.
        let register = compiler.allocate()?;
        let binding = LocalBinding::initialized(register, BindingKind::Mutable);
        scope.bindings.insert(atom, binding);
    }
    let body = ast.list(function.body);
    let functions = compiler.top_level_functions(body)?;
    let mut var_names = Vec::new();
    for &item in body {
        compiler.var_declared_names(item, &mut var_names)?;
    }
    let items: Vec<NodeId> = body
        .iter()
        .copied()
        .filter(|&item| !matches!(ast.node(item), Node::FunctionDeclaration(_)))
        .collect();
    let mut lexical_names = Vec::new();
    for &item in &items {
        lexical_names.extend(compiler.lexical_declarations(item)?);
    }
    // argumentsObjectNeeded: Ein Parameter, eine Funktion oder eine
    // lexikalische Deklaration mit dem Namen ersetzt das Objekt, `var` nicht.
    compiler.arguments = !scope
        .bindings
        .keys()
        .any(|&atom| compiler.is_arguments(atom))
        && !functions
            .iter()
            .any(|&(name, _, _)| compiler.is_arguments(name))
        && !lexical_names
            .iter()
            .any(|&(atom, _, _)| compiler.is_arguments(atom));
    if compiler.arguments && var_names.iter().any(|&atom| compiler.is_arguments(atom)) {
        return Err(CompileError::Unsupported(position));
    }
    for atom in var_names
        .into_iter()
        .chain(functions.iter().map(|&(name, _, _)| name))
    {
        if let Entry::Vacant(entry) = scope.bindings.entry(atom) {
            let register = compiler.allocate()?;
            entry.insert(LocalBinding::initialized(register, BindingKind::Mutable));
        }
    }
    compiler.scopes.push(scope);
    compiler.enter_block(&items, false)?;
    for &(name, id, node) in &functions {
        compiler.function_declaration(node, id)?;
        if let Some(binding) = compiler.lookup(name) {
            compiler.emit(Op::Star, &[i64::from(binding.register)])?;
        }
    }
    for &item in body {
        compiler.top_level_item(item)?;
    }
    compiler.emit(Op::LdaUndefined, &[])?;
    compiler.emit(Op::Return, &[])?;
    let mut code = compiler.finish();
    code.parameters = u32::try_from(parameters.len()).map_err(|_| CompileError::TooLarge)?;
    Ok(code)
}

impl<'a> Compiler<'a> {
    fn new(ast: &'a Ast, strict: bool, outer: Rc<BTreeSet<Atom>>) -> Compiler<'a> {
        Compiler {
            ast,
            emitter: Emitter::default(),
            constants: Vec::new(),
            constant_index: BTreeMap::new(),
            next_register: 0,
            register_count: 0,
            scopes: Vec::new(),
            targets: Vec::new(),
            strict,
            completion: None,
            arguments: false,
            outer,
        }
    }

    fn finish(self) -> Code {
        self.emitter
            .finish(self.constants, self.register_count, self.strict)
    }

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

    /// Eine Konstante. Gleiche Zahlen, Zeichenketten und Namen teilen sich
    /// eine, jede Funktion bekommt eine eigene.
    fn constant(&mut self, constant: Constant) -> Result<i64> {
        let key = match &constant {
            Constant::Number(number) => Some(ConstantKey::Number(number.to_bits())),
            Constant::String(units) => Some(ConstantKey::String(units.clone())),
            Constant::Name(units) => Some(ConstantKey::Name(units.clone())),
            Constant::Function(_) => None,
        };
        if let Some(&index) = key.as_ref().and_then(|key| self.constant_index.get(key)) {
            return Ok(index);
        }
        let index = i64::try_from(self.constants.len())
            .ok()
            .filter(|&index| u32::try_from(index).is_ok())
            .ok_or(CompileError::TooLarge)?;
        self.constants.push(constant);
        if let Some(key) = key {
            self.constant_index.insert(key, index);
        }
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
    /// IsConstantDeclaration. Funktionen sind dort keine lexikalischen
    /// Deklarationen.
    fn top_level_lexical_names(&self, body: &[NodeId]) -> Result<Vec<(Vec<u16>, bool)>> {
        let mut names = Vec::new();
        for &item in body {
            if matches!(self.ast.node(item), Node::FunctionDeclaration(_)) {
                continue;
            }
            for (atom, constant, _) in self.lexical_declarations(item)? {
                names.push((self.ast.atom(atom).to_vec(), constant));
            }
        }
        Ok(names)
    }

    /// functionsToInitialize (Abschnitte 10.2.11 und 16.1.7): die
    /// Funktionsdeklarationen auf oberster Ebene, von mehreren mit
    /// demselben Namen die letzte, in der Reihenfolge des Quelltexts.
    fn top_level_functions(&self, body: &[NodeId]) -> Result<Vec<(Atom, FunctionId, NodeId)>> {
        let mut functions = Vec::new();
        let mut names = BTreeSet::new();
        for &item in body.iter().rev() {
            let Node::FunctionDeclaration(id) = *self.ast.node(item) else {
                continue;
            };
            let function = self
                .ast
                .function(id)
                .ok_or_else(|| self.unsupported(item))?;
            if function.generator || function.is_async {
                return Err(self.unsupported(item));
            }
            let Some(name) = function.name else {
                return Err(self.unsupported(item));
            };
            if names.insert(name) {
                functions.push((name, id, item));
            }
        }
        functions.reverse();
        Ok(functions)
    }

    /// Eine Anweisung auf oberster Ebene eines Scripts oder Funktionsrumpfs.
    /// Die Funktionsdeklarationen dort sind zu Beginn schon angelegt.
    fn top_level_item(&mut self, item: NodeId) -> Result<()> {
        if matches!(self.ast.node(item), Node::FunctionDeclaration(_)) {
            return Ok(());
        }
        self.statement(item)
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

    /// VarDeclaredNames (Abschnitt 8.2.8) der Anweisung ohne die Namen der
    /// Funktionen, angehängt an `names`, ohne Wiederholungen.
    fn var_declared_names(&self, node: NodeId, names: &mut Vec<Atom>) -> Result<()> {
        let mut add = |atom: Atom| {
            if !names.contains(&atom) {
                names.push(atom);
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
                let kind = if constant {
                    BindingKind::Const
                } else {
                    BindingKind::Mutable
                };
                scope.bindings.insert(
                    atom,
                    LocalBinding {
                        register,
                        kind,
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

    fn is_arguments(&self, atom: Atom) -> bool {
        self.ast
            .atom(atom)
            .iter()
            .copied()
            .eq("arguments".encode_utf16())
    }

    /// ResolveBinding (Abschnitt 9.4.2) beim Übersetzen: eine Bindung in
    /// einem Register oder `None` für die globale Umgebung. Eine Bindung
    /// aus einem Register des umgebenden Codes bräuchte eine Closure, das
    /// Objekt `arguments` fehlt noch.
    fn resolve(&self, atom: Atom, node: NodeId) -> Result<Option<LocalBinding>> {
        if let Some(binding) = self.lookup(atom) {
            return Ok(Some(binding));
        }
        if self.outer.contains(&atom) || (self.arguments && self.is_arguments(atom)) {
            return Err(self.unsupported(node));
        }
        Ok(None)
    }

    /// Die Namen, die eine Funktion sähe, die hier entsteht: die Bindungen
    /// aller offenen Bereiche und die des umgebenden Codes.
    fn visible_names(&self) -> Rc<BTreeSet<Atom>> {
        if self.scopes.iter().all(|scope| scope.bindings.is_empty()) {
            return Rc::clone(&self.outer);
        }
        let mut names = BTreeSet::clone(&self.outer);
        for scope in &self.scopes {
            names.extend(scope.bindings.keys().copied());
        }
        Rc::new(names)
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
                match self.completion {
                    Some(completion) => self.emit(Op::Star, &[completion]),
                    None => Ok(()),
                }
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
            // Der Parser lässt `return` nur in Funktionen zu.
            Node::Return(argument) => {
                match argument {
                    Some(argument) => self.expression(argument)?,
                    None => self.emit(Op::LdaUndefined, &[])?,
                }
                self.position(node)?;
                self.emit(Op::Return, &[])
            }
            _ => Err(self.unsupported(node)),
        }
    }

    fn reset_completion(&mut self) -> Result<()> {
        let Some(completion) = self.completion else {
            return Ok(());
        };
        self.emit(Op::LdaUndefined, &[])?;
        self.emit(Op::Star, &[completion])
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
            let units = ast.atom(atom);
            match kind {
                DeclarationKind::Var => {
                    // Ohne Initialisierer tut die Deklaration hier nichts.
                    // Mit ihm wird erst der Name aufgelöst, dann gelesen
                    // und geschrieben. In einer Funktion liegt er in einem
                    // Register, in einem Script in der globalen Umgebung.
                    let Some(init) = init else {
                        continue;
                    };
                    if let Some(binding) = self.resolve(atom, target)? {
                        self.named_expression(init, units)?;
                        self.position(declarator)?;
                        self.store_local(binding, atom, self.ast.range(declarator).end)?;
                        continue;
                    }
                    let name = self.name(atom)?;
                    let register = self.allocate()?;
                    self.emit(Op::ResolveGlobal, &[name, i64::from(register)])?;
                    self.named_expression(init, units)?;
                    self.position(declarator)?;
                    self.emit(Op::StaGlobal, &[name, i64::from(register)])?;
                    self.release(register);
                }
                DeclarationKind::Let | DeclarationKind::Const => {
                    match init {
                        Some(init) => self.named_expression(init, units)?,
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
            Node::Object(list) => self.object_literal(list),
            Node::Array(list) => self.array_literal(list),
            Node::Member { .. } | Node::ComputedMember { .. } | Node::Call { .. } => {
                self.chain(node, ChainEnd::Value).map(|_| ())
            }
            Node::Chain(chain) => self.chain(chain, ChainEnd::Value).map(|_| ()),
            Node::Function(id) => self.closure(node, id, ClosureName::Static(&[])),
            _ => Err(self.unsupported(node)),
        }
    }

    /// IsAnonymousFunctionDefinition (Abschnitt 8.4.3) für die Funktionen,
    /// die es schon gibt; Klammern zählen nicht.
    fn anonymous_function(&self, node: NodeId) -> Option<(NodeId, FunctionId)> {
        let inner = self.without_parens(node);
        match *self.ast.node(inner) {
            Node::Function(id)
                if self
                    .ast
                    .function(id)
                    .is_some_and(|function| function.name.is_none()) =>
            {
                Some((inner, id))
            }
            _ => None,
        }
    }

    /// NamedEvaluation (Abschnitt 8.4.5): Eine anonyme Funktion, auch in
    /// Klammern, bekommt den Namen `name`; alles andere wird gewöhnlich
    /// ausgewertet.
    fn named_expression(&mut self, node: NodeId, name: &[u16]) -> Result<()> {
        match self.anonymous_function(node) {
            Some((function, id)) => self.closure(function, id, ClosureName::Static(name)),
            None => self.expression(node),
        }
    }

    /// Dasselbe mit dem Schlüssel im Register als Namen.
    fn keyed_expression(&mut self, node: NodeId, key: u32) -> Result<()> {
        match self.anonymous_function(node) {
            Some((function, id)) => self.closure(function, id, ClosureName::Keyed(key)),
            None => self.expression(node),
        }
    }

    /// Ein neues Funktionsobjekt im Akkumulator, für einen
    /// Funktionsausdruck (InstantiateOrdinaryFunctionExpression, Abschnitt
    /// 15.2.5) oder eine Methode (DefineMethod, Abschnitt 15.4.2). Ein
    /// benannter Funktionsausdruck heißt wie die Funktion und bindet den
    /// Namen in seinem Rumpf, sonst gilt `name`.
    fn closure(&mut self, node: NodeId, id: FunctionId, name: ClosureName<'_>) -> Result<()> {
        let own = self
            .ast
            .function(id)
            .filter(|function| function.kind == FunctionKind::Normal)
            .and_then(|function| function.name);
        self.create_closure(node, id, own.is_some(), own, name)
    }

    /// InstantiateOrdinaryFunctionObject (Abschnitt 15.2.4) für eine
    /// Funktionsdeklaration: Sie heißt wie die Funktion, gebunden wird der
    /// Name im umgebenden Code.
    fn function_declaration(&mut self, node: NodeId, id: FunctionId) -> Result<()> {
        let own = self.ast.function(id).and_then(|function| function.name);
        self.create_closure(node, id, false, own, ClosureName::Static(&[]))
    }

    fn create_closure(
        &mut self,
        node: NodeId,
        id: FunctionId,
        binds_name: bool,
        own: Option<Atom>,
        name: ClosureName<'_>,
    ) -> Result<()> {
        let Some(function) = self.ast.function(id) else {
            return Err(self.unsupported(node));
        };
        let supported = matches!(
            function.kind,
            FunctionKind::Normal
                | FunctionKind::Method
                | FunctionKind::Getter
                | FunctionKind::Setter
        ) && !function.generator
            && !function.is_async;
        if !supported {
            return Err(self.unsupported(node));
        }
        let template = FunctionTemplate {
            function: id,
            binds_name,
            outer: self.visible_names(),
        };
        let template = self.constant(Constant::Function(template))?;
        self.position(node)?;
        match (own, name) {
            (Some(atom), _) => {
                let name = self.name(atom)?;
                self.emit(Op::CreateClosure, &[template, name])
            }
            (None, ClosureName::Static(units)) => {
                let name = self.constant(Constant::Name(units.to_vec()))?;
                self.emit(Op::CreateClosure, &[template, name])
            }
            (None, ClosureName::Keyed(register)) => {
                self.emit(Op::CreateClosureKeyed, &[template, i64::from(register)])
            }
        }
    }

    fn without_parens(&self, node: NodeId) -> NodeId {
        let mut node = node;
        while let Node::Paren(inner) = *self.ast.node(node) {
            node = inner;
        }
        node
    }

    /// Objektliterale (Abschnitte 13.2.5.5 und 13.2.5.6).
    fn object_literal(&mut self, list: List) -> Result<()> {
        self.emit(Op::CreateObject, &[])?;
        let object = self.allocate()?;
        let register = i64::from(object);
        self.emit(Op::Star, &[register])?;
        let ast = self.ast;
        for &property in ast.list(list) {
            self.position(property)?;
            match *self.ast.node(property) {
                Node::Spread(argument) => {
                    self.expression(argument)?;
                    self.position(property)?;
                    self.emit(Op::CopyDataProperties, &[register])?;
                }
                Node::Property {
                    kind: kind @ (PropertyKind::Init | PropertyKind::Shorthand),
                    key,
                    value,
                } => self.property_definition(register, kind, key, value)?,
                Node::Property {
                    kind: kind @ (PropertyKind::Method | PropertyKind::Get | PropertyKind::Set),
                    key,
                    value,
                } => self.method_definition(register, kind, key, value)?,
                _ => return Err(self.unsupported(property)),
            }
        }
        self.emit(Op::Ldar, &[register])?;
        self.release(object);
        Ok(())
    }

    /// Der Name einer Eigenschaft im Literal: ein fester als Codeeinheiten,
    /// ein berechneter nach ToPropertyKey im Akkumulator (dann `None`).
    fn property_key(&mut self, key: NodeId) -> Result<Option<Vec<u16>>> {
        match *self.ast.node(key) {
            Node::PropertyName(atom) | Node::String(atom) => Ok(Some(self.ast.atom(atom).to_vec())),
            Node::Number(number) => Ok(Some(number::to_string(number).encode_utf16().collect())),
            Node::ComputedKey(expression) => {
                self.expression(expression)?;
                self.position(key)?;
                self.emit(Op::ToPropertyKey, &[])?;
                Ok(None)
            }
            _ => Err(self.unsupported(key)),
        }
    }

    /// Eine Eigenschaft mit Wert im Objekt im Register `object`. Eine
    /// anonyme Funktion als Wert heißt wie die Eigenschaft.
    fn property_definition(
        &mut self,
        object: i64,
        kind: PropertyKind,
        key: NodeId,
        value: NodeId,
    ) -> Result<()> {
        let Some(units) = self.property_key(key)? else {
            // Der berechnete Name kommt mit ToPropertyKey vor dem Wert.
            let name = self.allocate()?;
            self.emit(Op::Star, &[i64::from(name)])?;
            self.keyed_expression(value, name)?;
            self.position(key)?;
            self.emit(Op::DefineKeyed, &[object, i64::from(name)])?;
            self.release(name);
            return Ok(());
        };
        // `__proto__: value` ohne berechneten Namen setzt den Prototyp
        // (PropertyDefinitionEvaluation, Schritte 5 und 6), die Kurzform
        // `{__proto__}` nicht. Eine Funktion bekommt dort keinen Namen.
        let proto: Vec<u16> = "__proto__".encode_utf16().collect();
        if kind == PropertyKind::Init && units == proto {
            self.expression(value)?;
            self.position(key)?;
            return self.emit(Op::SetPrototypeLiteral, &[object]);
        }
        self.named_expression(value, &units)?;
        self.position(key)?;
        let name = self.constant(Constant::Name(units))?;
        self.emit(Op::DefineNamed, &[object, name])
    }

    /// Methoden, Getter und Setter im Objekt im Register `object`
    /// (MethodDefinitionEvaluation, Abschnitt 15.4.4): erst der Name, dann
    /// das Funktionsobjekt, dann die Eigenschaft, aufzählbar und
    /// konfigurierbar.
    fn method_definition(
        &mut self,
        object: i64,
        kind: PropertyKind,
        key: NodeId,
        value: NodeId,
    ) -> Result<()> {
        let Node::Function(id) = *self.ast.node(value) else {
            return Err(self.unsupported(value));
        };
        let name = self.allocate()?;
        let register = i64::from(name);
        match self.property_key(key)? {
            Some(units) if kind == PropertyKind::Method => {
                self.closure(value, id, ClosureName::Static(&units))?;
                self.position(key)?;
                let constant = self.constant(Constant::Name(units))?;
                self.emit(Op::DefineNamed, &[object, constant])?;
                self.release(name);
                return Ok(());
            }
            Some(units) => {
                let constant = self.constant(Constant::Name(units.clone()))?;
                self.emit(Op::LdaConstant, &[constant])?;
                self.emit(Op::Star, &[register])?;
                self.closure(value, id, ClosureName::Static(&units))?;
            }
            None => {
                self.emit(Op::Star, &[register])?;
                self.closure(value, id, ClosureName::Keyed(name))?;
            }
        }
        self.position(key)?;
        let op = match kind {
            PropertyKind::Get => Op::DefineGetter,
            PropertyKind::Set => Op::DefineSetter,
            _ => Op::DefineKeyed,
        };
        self.emit(op, &[object, register])?;
        self.release(name);
        Ok(())
    }

    /// Array-Literale (Abschnitte 13.2.4.1 und 13.2.4.2). Die Ausbreitung
    /// braucht Iteratoren.
    fn array_literal(&mut self, list: List) -> Result<()> {
        self.emit(Op::CreateArray, &[])?;
        let array = self.allocate()?;
        let register = i64::from(array);
        self.emit(Op::Star, &[register])?;
        let ast = self.ast;
        for &element in ast.list(list) {
            match *self.ast.node(element) {
                Node::Elision => {
                    self.position(element)?;
                    self.emit(Op::ArrayHole, &[register])?;
                }
                Node::Spread(_) => return Err(self.unsupported(element)),
                _ => {
                    self.expression(element)?;
                    self.position(element)?;
                    self.emit(Op::ArrayPush, &[register])?;
                }
            }
        }
        self.emit(Op::Ldar, &[register])?;
        self.release(array);
        Ok(())
    }

    /// Der Kopf einer Kette aus Zugriffen und Aufrufen und ihre Glieder von
    /// innen nach außen, ohne Rekursion entlang der Kette.
    fn chain_links(&self, node: NodeId) -> (NodeId, Vec<NodeId>) {
        let mut links = Vec::new();
        let mut head = node;
        loop {
            match *self.ast.node(head) {
                Node::Member { object, .. } | Node::ComputedMember { object, .. } => {
                    links.push(head);
                    head = object;
                }
                Node::Call { callee, .. } => {
                    links.push(head);
                    head = callee;
                }
                _ => break,
            }
        }
        links.reverse();
        (head, links)
    }

    fn is_call(&self, node: NodeId) -> bool {
        matches!(self.ast.node(node), Node::Call { .. })
    }

    /// Eine Kette aus Zugriffen und Aufrufen: GetValue für Zugriffe
    /// (Abschnitte 13.3.2 und 13.3.3), EvaluateCall für Aufrufe (Abschnitt
    /// 13.3.6) und mit `delete` das Löschen am letzten Glied. Springt ein
    /// Glied nach `?.` heraus, ergibt die ganze Kette *undefined*, mit
    /// `delete` *true*. Gibt zurück, ob am Ende eine Basis als `this` im
    /// Register aus `end` liegt.
    fn chain(&mut self, node: NodeId, end: ChainEnd) -> Result<bool> {
        let (head, links) = self.chain_links(node);
        let mark = self.next_register;
        // Funktion und `this` für die Aufrufe, dahinter ihre Argumente.
        let callee = if links.iter().any(|&link| self.is_call(link)) {
            let callee = self.allocate()?;
            self.allocate()?;
            Some(callee)
        } else {
            None
        };
        let this_register = callee.and_then(|callee| callee.checked_add(1));
        let mut nullish = Vec::new();
        let mut has_this = match (this_register, links.first()) {
            (Some(this), Some(&first)) if self.is_call(first) => self.callee_head(head, this)?,
            _ => {
                self.expression(head)?;
                false
            }
        };
        for (index, &link) in links.iter().enumerate() {
            let next = links.get(index + 1).copied();
            if let Node::Call {
                arguments,
                optional,
                ..
            } = *self.ast.node(link)
            {
                let Some(callee) = callee else {
                    return Err(self.unsupported(link));
                };
                self.position(link)?;
                if optional {
                    nullish.push(self.jump(Op::JumpIfNullish)?);
                }
                self.call(link, callee, has_this, arguments)?;
                has_this = false;
                continue;
            }
            let this = match (next, end) {
                (Some(next), _) if self.is_call(next) => this_register,
                (None, ChainEnd::Callee(register)) => Some(register),
                _ => None,
            };
            let delete = next.is_none() && end == ChainEnd::Delete;
            self.member_link(link, &mut nullish, delete, this)?;
            has_this = this.is_some();
        }
        self.release(mark);
        let result = if end == ChainEnd::Delete {
            Op::LdaTrue
        } else {
            Op::LdaUndefined
        };
        self.short_circuit(nullish, result)?;
        Ok(has_this)
    }

    /// Der Kopf einer Kette, dessen Wert aufgerufen wird. Der
    /// Gruppierungsoperator gibt eine Referenz weiter (Abschnitt 13.2.9.1),
    /// auch die aus einer optionalen Kette: Ein Zugriff in Klammern legt
    /// seine Basis als `this` ab.
    fn callee_head(&mut self, head: NodeId, this: u32) -> Result<bool> {
        let inner = self.without_parens(head);
        match *self.ast.node(inner) {
            Node::Member { .. } | Node::ComputedMember { .. } => {
                self.chain(inner, ChainEnd::Callee(this))
            }
            Node::Chain(chain) => self.chain(chain, ChainEnd::Callee(this)),
            _ => {
                self.expression(head)?;
                Ok(false)
            }
        }
    }

    /// Ein Aufruf der Funktion im Akkumulator (EvaluateCall, Abschnitt
    /// 13.3.6.2): Funktion, `this` und Argumente in den Registern ab
    /// `callee`. Ohne Basis ist `this` *undefined*.
    fn call(&mut self, node: NodeId, callee: u32, has_this: bool, arguments: List) -> Result<()> {
        let register = i64::from(callee);
        self.emit(Op::Star, &[register])?;
        if !has_this {
            self.emit(Op::LdaUndefined, &[])?;
            self.emit(Op::Star, &[register + 1])?;
        }
        // Die Argumente folgen auf `this`; Zwischenwerte davor sind schon
        // zurückgegeben.
        let first = self.next_register;
        if callee.checked_add(2) != Some(first) {
            return Err(self.unsupported(node));
        }
        let ast = self.ast;
        let arguments = ast.list(arguments);
        for &argument in arguments {
            if let Node::Spread(_) = *self.ast.node(argument) {
                return Err(self.unsupported(argument));
            }
            let slot = self.allocate()?;
            self.expression(argument)?;
            self.emit(Op::Star, &[i64::from(slot)])?;
        }
        self.position(node)?;
        let count = i64::try_from(arguments.len()).map_err(|_| CompileError::TooLarge)?;
        self.emit(Op::Call, &[register, count])?;
        self.release(first);
        Ok(())
    }

    /// Ein Glied einer Kette mit der Basis im Akkumulator: GetValue auf den
    /// Zugriff oder mit `delete` das Löschen. Nach `?.` springt es bei
    /// *undefined* und *null* nach `nullish` (Abschnitt 13.3.9.1). Mit
    /// `this` bleibt die Basis für einen Aufruf in diesem Register.
    fn member_link(
        &mut self,
        link: NodeId,
        nullish: &mut Vec<Label>,
        delete: bool,
        this: Option<u32>,
    ) -> Result<()> {
        self.position(link)?;
        let (access, optional) = match *self.ast.node(link) {
            Node::Member {
                property, optional, ..
            } => (Access::Named(property), optional),
            Node::ComputedMember {
                property, optional, ..
            } => (Access::Keyed(property), optional),
            _ => return Err(self.unsupported(link)),
        };
        if optional {
            nullish.push(self.jump(Op::JumpIfNullish)?);
        }
        match access {
            Access::Named(atom) => {
                if let Some(this) = this {
                    self.emit(Op::Star, &[i64::from(this)])?;
                }
                let name = self.name(atom)?;
                let op = if delete {
                    Op::DeleteNamed
                } else {
                    Op::LdaNamed
                };
                self.emit(op, &[name])
            }
            Access::Keyed(expression) => {
                let base = match this {
                    Some(this) => this,
                    None => self.allocate()?,
                };
                self.emit(Op::Star, &[i64::from(base)])?;
                self.expression(expression)?;
                self.position(link)?;
                let op = if delete {
                    Op::DeleteKeyed
                } else {
                    Op::LdaKeyed
                };
                self.emit(op, &[i64::from(base)])?;
                if this.is_none() {
                    self.release(base);
                }
                Ok(())
            }
        }
    }

    /// Bindet die Sprünge aus einer optionalen Kette an einen Befehl, der
    /// das Ergebnis dafür lädt.
    fn short_circuit(&mut self, labels: Vec<Label>, result: Op) -> Result<()> {
        if labels.is_empty() {
            return Ok(());
        }
        let end = self.jump(Op::Jump)?;
        self.bind_all(labels)?;
        self.emit(result, &[])?;
        self.bind(end)
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
        if binding.kind == BindingKind::FunctionName {
            if self.strict {
                let name = self.name(atom)?;
                return self.emit(Op::ThrowConstAssignment, &[name]);
            }
            return Ok(());
        }
        let constant = binding.kind == BindingKind::Const;
        if Self::needs_hole_check(binding, position) || constant {
            let name = self.name(atom)?;
            self.emit(Op::CheckHole, &[register, name])?;
            if constant {
                return self.emit(Op::ThrowConstAssignment, &[name]);
            }
        }
        self.emit(Op::Star, &[register])
    }

    /// Ein Bezeichner als Ausdruck: ResolveBinding und GetValue.
    fn load_identifier(&mut self, node: NodeId, atom: Atom) -> Result<()> {
        match self.resolve(atom, node)? {
            Some(binding) => self.load_local(binding, atom, self.ast.range(node).start),
            None => {
                let name = self.name(atom)?;
                self.emit(Op::LdaGlobal, &[name])
            }
        }
    }

    /// Wertet das Ziel einer Zuweisung zur Referenz aus.
    fn reference(&mut self, target: NodeId) -> Result<Reference> {
        match *self.ast.node(self.without_parens(target)) {
            Node::Identifier(atom) => {
                if let Some(binding) = self.resolve(atom, target)? {
                    return Ok(Reference::Local(binding, atom));
                }
                let name = self.name(atom)?;
                let register = self.allocate()?;
                self.emit(Op::ResolveGlobal, &[name, i64::from(register)])?;
                Ok(Reference::Global { name, register })
            }
            Node::Member {
                object,
                property,
                optional: false,
            } => {
                self.expression(object)?;
                let base = self.allocate()?;
                self.emit(Op::Star, &[i64::from(base)])?;
                let name = self.name(property)?;
                Ok(Reference::Named { base, name })
            }
            Node::ComputedMember {
                object,
                property,
                optional: false,
            } => {
                self.expression(object)?;
                let base = self.allocate()?;
                self.emit(Op::Star, &[i64::from(base)])?;
                self.expression(property)?;
                let key = self.allocate()?;
                self.emit(Op::Star, &[i64::from(key)])?;
                Ok(Reference::Keyed { base, key })
            }
            _ => Err(self.unsupported(target)),
        }
    }

    fn get_value(&mut self, reference: Reference, position: usize) -> Result<()> {
        match reference {
            Reference::Local(binding, atom) => self.load_local(binding, atom, position),
            Reference::Global { name, register } => {
                self.emit(Op::LdaGlobalRef, &[name, i64::from(register)])
            }
            Reference::Named { base, name } => {
                self.emit(Op::Ldar, &[i64::from(base)])?;
                self.emit(Op::LdaNamed, &[name])
            }
            Reference::Keyed { base, key } => {
                self.emit(Op::LdaKeyedRef, &[i64::from(base), i64::from(key)])
            }
        }
    }

    fn put_value(&mut self, reference: Reference, position: usize) -> Result<()> {
        match reference {
            Reference::Local(binding, atom) => self.store_local(binding, atom, position),
            Reference::Global { name, register } => {
                self.emit(Op::StaGlobal, &[name, i64::from(register)])
            }
            Reference::Named { base, name } => self.emit(Op::StaNamed, &[i64::from(base), name]),
            Reference::Keyed { base, key } => {
                self.emit(Op::StaKeyed, &[i64::from(base), i64::from(key)])
            }
        }
    }

    fn release_reference(&mut self, reference: Reference) {
        match reference {
            Reference::Local(..) => {}
            Reference::Global { register, .. }
            | Reference::Named { base: register, .. }
            | Reference::Keyed { base: register, .. } => self.release(register),
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
        // NamedEvaluation nur, wenn das Ziel ein Bezeichner ohne Klammern
        // ist (IsIdentifierRef).
        let ast = self.ast;
        let name = match *ast.node(target) {
            Node::Identifier(atom) => Some(ast.atom(atom)),
            _ => None,
        };
        let binary = match operator {
            AssignOperator::Assign => None,
            AssignOperator::LogicalAnd | AssignOperator::LogicalOr | AssignOperator::Coalesce => {
                self.get_value(reference, position)?;
                let skip = self.jump(match operator {
                    AssignOperator::LogicalAnd => Op::JumpIfFalse,
                    AssignOperator::LogicalOr => Op::JumpIfTrue,
                    _ => Op::JumpIfNotNullish,
                })?;
                match name {
                    Some(name) => self.named_expression(value, name)?,
                    None => self.expression(value)?,
                }
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
                match name {
                    Some(name) => self.named_expression(value, name)?,
                    None => self.expression(value)?,
                }
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
                    match self.resolve(atom, argument)? {
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
            if self.resolve(atom, argument)?.is_some() {
                return self.emit(Op::LdaFalse, &[]);
            }
            let name = self.name(atom)?;
            return self.emit(Op::DeleteGlobal, &[name]);
        }
        let inner = self.without_parens(argument);
        let target = match *self.ast.node(inner) {
            Node::Chain(chain) => chain,
            _ => inner,
        };
        match self.ast.node(target) {
            // Das letzte Glied löscht. Springt vorher ein Glied nach `?.`
            // heraus, ergibt die Kette keine Referenz, und das Ergebnis ist
            // *true*.
            Node::Member { .. } | Node::ComputedMember { .. } => {
                return self.chain(target, ChainEnd::Delete).map(|_| ());
            }
            Node::PrivateMember { .. } | Node::SuperMember(_) | Node::SuperComputedMember(_) => {
                return Err(self.unsupported(inner));
            }
            _ => {}
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
            BinaryOperator::In => (Op::TestIn, false),
            BinaryOperator::Instanceof => (Op::TestInstanceOf, false),
            BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr | BinaryOperator::Coalesce => {
                return Err(self.unsupported(node));
            }
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
