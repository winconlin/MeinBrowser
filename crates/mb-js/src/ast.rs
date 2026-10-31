//! Der Syntaxbaum, kompakt in einer Arena (`docs/architektur.md`,
//! Abschnitt 4.9).
//!
//! Knoten verweisen über 32-Bit-Indizes aufeinander ([`NodeId`]). Listen
//! von Knoten sind Abschnitte einer gemeinsamen Liste ([`List`]), Namen und
//! Zeichenketten liegen interniert als UTF-16-Codeeinheiten vor ([`Atom`]).
//! Funktionen und Klassen stehen in eigenen Tabellen, damit die Knoten
//! klein bleiben. Zu jedem Knoten gehört seine Lage im Quelltext.
//!
//! Der Baum hält fest, was der Quelltext sagt, auch Klammern
//! ([`Node::Paren`]), denn manche Regeln der Spezifikation hängen daran.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::ops::Range;
use std::rc::Rc;

/// Ein Knoten in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(NonZeroU32);

impl NodeId {
    fn index(self) -> usize {
        // Kann nicht scheitern: Der Index stammt aus einer Länge.
        usize::try_from(self.0.get() - 1).unwrap_or(usize::MAX)
    }
}

/// Eine Liste von Knoten in [`Ast`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct List {
    start: u32,
    len: u32,
}

impl List {
    pub fn len(self) -> usize {
        usize::try_from(self.len).unwrap_or(usize::MAX)
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Ein internierter Name oder eine internierte Zeichenkette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Atom(NonZeroU32);

impl Atom {
    fn index(self) -> usize {
        usize::try_from(self.0.get() - 1).unwrap_or(usize::MAX)
    }
}

/// Eine Funktion in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionId(u32);

impl FunctionId {
    /// Die Nummer der Funktion im Baum, in der Reihenfolge, in der der
    /// Parser sie abschließt.
    pub fn index(self) -> usize {
        to_usize(self.0)
    }
}

/// Eine Klasse in [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOperator {
    Minus,
    Plus,
    Not,
    BitwiseNot,
    Typeof,
    Void,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOperator {
    Increment,
    Decrement,
}

/// Zweistellige Operatoren, auch die logischen und `??`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOperator {
    Equal,
    NotEqual,
    StrictEqual,
    StrictNotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    ShiftLeft,
    ShiftRight,
    UnsignedShiftRight,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Exponent,
    BitwiseOr,
    BitwiseXor,
    BitwiseAnd,
    In,
    Instanceof,
    LogicalAnd,
    LogicalOr,
    Coalesce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOperator {
    Assign,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Exponent,
    ShiftLeft,
    ShiftRight,
    UnsignedShiftRight,
    BitwiseOr,
    BitwiseXor,
    BitwiseAnd,
    LogicalAnd,
    LogicalOr,
    Coalesce,
}

/// Wie eine Eigenschaft in einem Objektliteral entsteht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyKind {
    /// `key: value`
    Init,
    /// `name`, der Wert ist [`Node::Identifier`].
    Shorthand,
    /// Eine Methode, der Wert ist [`Node::Function`].
    Method,
    Get,
    Set,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodKind {
    Method,
    Get,
    Set,
}

/// Die Art einer Deklaration mit [`Node::Variables`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationKind {
    Var,
    Let,
    Const,
    Using,
    AwaitUsing,
}

/// Ein Knoten. Ausdrücke, Muster, Anweisungen und die Teile von Modulen
/// teilen sich einen Typ, damit Cover-Grammatiken (Abschnitt 5.1.4) Knoten
/// an Ort und Stelle umdeuten können.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Node {
    // Ausdrücke
    This,
    /// *IdentifierReference*
    Identifier(Atom),
    Null,
    Boolean(bool),
    Number(f64),
    /// Eine BigInt-Zahl als Ziffern zur Basis `radix`.
    BigInt {
        radix: u8,
        digits: Atom,
    },
    String(Atom),
    RegExp {
        body: Atom,
        flags: Atom,
    },
    /// Ein Template: Teile ([`Node::TemplateElement`]) und dazwischen die
    /// Ausdrücke, ein Teil mehr als Ausdrücke.
    Template {
        quasis: List,
        expressions: List,
    },
    /// Ein Teil eines Templates mit Wert (`None` ist *undefined*) und
    /// Rohwert.
    TemplateElement {
        cooked: Option<Atom>,
        raw: Atom,
    },
    TaggedTemplate {
        tag: NodeId,
        quasi: NodeId,
    },
    /// Ein Array-Literal mit [`Node::Elision`] für Lücken und
    /// [`Node::Spread`].
    Array(List),
    Elision,
    Spread(NodeId),
    /// Ein Objektliteral mit [`Node::Property`] und [`Node::Spread`].
    Object(List),
    Property {
        kind: PropertyKind,
        key: NodeId,
        value: NodeId,
    },
    /// Ein Eigenschaftsname aus einem *IdentifierName*. Namen aus
    /// Zeichenketten und Zahlen sind [`Node::String`] und
    /// [`Node::Number`] oder [`Node::BigInt`].
    PropertyName(Atom),
    /// `[expression]` als Eigenschaftsname.
    ComputedKey(NodeId),
    /// *PrivateIdentifier*, der Name ohne `#`.
    PrivateName(Atom),
    Function(FunctionId),
    Class(ClassId),
    /// `object.property`, mit `?.` davor, wenn `optional`.
    Member {
        object: NodeId,
        property: Atom,
        optional: bool,
    },
    /// `object[property]`
    ComputedMember {
        object: NodeId,
        property: NodeId,
        optional: bool,
    },
    /// `object.#name`
    PrivateMember {
        object: NodeId,
        name: Atom,
        optional: bool,
    },
    /// `super.property`
    SuperMember(Atom),
    /// `super[property]`
    SuperComputedMember(NodeId),
    Call {
        callee: NodeId,
        arguments: List,
        optional: bool,
    },
    /// `super(arguments)`
    SuperCall(List),
    New {
        callee: NodeId,
        arguments: List,
    },
    NewTarget,
    ImportMeta,
    ImportCall {
        specifier: NodeId,
        options: Option<NodeId>,
    },
    /// Eine Kette mit `?.` (*OptionalExpression*). Ist ein Glied
    /// *undefined* oder *null*, ergibt die ganze Kette *undefined*.
    Chain(NodeId),
    Unary {
        operator: UnaryOperator,
        argument: NodeId,
    },
    Update {
        operator: UpdateOperator,
        prefix: bool,
        argument: NodeId,
    },
    Binary {
        operator: BinaryOperator,
        left: NodeId,
        right: NodeId,
    },
    Conditional {
        test: NodeId,
        consequent: NodeId,
        alternate: NodeId,
    },
    /// Eine Zuweisung. Das Ziel ist ein Ausdruck oder bei `=` auch ein
    /// Muster.
    Assign {
        operator: AssignOperator,
        target: NodeId,
        value: NodeId,
    },
    Sequence(List),
    Yield {
        argument: Option<NodeId>,
        delegate: bool,
    },
    Await(NodeId),
    /// Ein geklammerter Ausdruck.
    Paren(NodeId),

    // Muster
    /// *BindingIdentifier*
    BindingIdentifier(Atom),
    /// Mit [`Node::PatternProperty`] und zuletzt vielleicht
    /// [`Node::Rest`].
    ObjectPattern(List),
    /// Mit [`Node::Elision`], Mustern und zuletzt vielleicht
    /// [`Node::Rest`].
    ArrayPattern(List),
    PatternProperty {
        key: NodeId,
        value: NodeId,
        shorthand: bool,
    },
    /// Ein Muster mit Vorgabewert.
    WithDefault {
        target: NodeId,
        default: NodeId,
    },
    Rest(NodeId),

    // Anweisungen und Deklarationen
    /// `var`, `let`, `const`, `using` oder `await using` mit
    /// [`Node::Declarator`].
    Variables {
        kind: DeclarationKind,
        declarators: List,
    },
    Declarator {
        target: NodeId,
        init: Option<NodeId>,
    },
    FunctionDeclaration(FunctionId),
    ClassDeclaration(ClassId),
    Expression(NodeId),
    Block(List),
    Empty,
    Debugger,
    If {
        test: NodeId,
        consequent: NodeId,
        alternate: Option<NodeId>,
    },
    /// `for (init; test; update) body`, `init` ist eine Deklaration
    /// oder ein Ausdruck.
    For {
        init: Option<NodeId>,
        test: Option<NodeId>,
        update: Option<NodeId>,
        body: NodeId,
    },
    /// `left` ist eine Deklaration mit einem Element, ein Muster oder
    /// ein Ausdruck.
    ForIn {
        left: NodeId,
        right: NodeId,
        body: NodeId,
    },
    ForOf {
        left: NodeId,
        right: NodeId,
        body: NodeId,
        is_await: bool,
    },
    While {
        test: NodeId,
        body: NodeId,
    },
    DoWhile {
        body: NodeId,
        test: NodeId,
    },
    Continue(Option<Atom>),
    Break(Option<Atom>),
    Return(Option<NodeId>),
    With {
        object: NodeId,
        body: NodeId,
    },
    /// Mit [`Node::Case`].
    Switch {
        discriminant: NodeId,
        cases: List,
    },
    /// `case test:`, ohne `test` ist es `default:`.
    Case {
        test: Option<NodeId>,
        body: List,
    },
    Labeled {
        label: Atom,
        body: NodeId,
    },
    Throw(NodeId),
    Try {
        block: NodeId,
        handler: Option<NodeId>,
        finalizer: Option<NodeId>,
    },
    Catch {
        parameter: Option<NodeId>,
        body: NodeId,
    },

    // Klassen
    /// Eine Methode, ein Getter oder ein Setter.
    ClassMethod {
        key: NodeId,
        function: FunctionId,
        kind: MethodKind,
        is_static: bool,
    },
    ClassField {
        key: NodeId,
        value: Option<NodeId>,
        is_static: bool,
    },
    StaticBlock(FunctionId),

    // Module
    /// `import … from source`, mit [`Node::ImportDefault`],
    /// [`Node::ImportNamespace`], [`Node::ImportSpecifier`] und
    /// [`Node::ImportAttribute`].
    Import {
        specifiers: List,
        source: Atom,
        attributes: List,
    },
    ImportDefault(Atom),
    ImportNamespace(Atom),
    ImportSpecifier {
        imported: Atom,
        local: Atom,
    },
    ImportAttribute {
        key: Atom,
        value: Atom,
    },
    /// `export { … }`, mit `source` eine Weitergabe aus einem anderen
    /// Modul. Mit [`Node::ExportSpecifier`].
    ExportNamed {
        specifiers: List,
        source: Option<Atom>,
        attributes: List,
    },
    ExportSpecifier {
        local: Atom,
        exported: Atom,
    },
    /// `export * from source` und `export * as exported from source`.
    ExportAll {
        exported: Option<Atom>,
        source: Atom,
        attributes: List,
    },
    /// `export` vor einer Deklaration.
    ExportDeclaration(NodeId),
    /// `export default`, mit einem Ausdruck oder einer Deklaration.
    ExportDefault(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionKind {
    /// Deklarationen und Ausdrücke mit `function`.
    Normal,
    Arrow,
    /// Methoden in Objektliteralen und Klassen.
    Method,
    Getter,
    Setter,
    /// Der Konstruktor einer Klasse ohne `extends`.
    Constructor,
    /// Der Konstruktor einer Klasse mit `extends`.
    DerivedConstructor,
    /// Ein `static`-Block.
    StaticBlock,
}

/// Eine Funktion.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub kind: FunctionKind,
    pub generator: bool,
    pub is_async: bool,
    /// Ob der Code der Funktion strikt ist.
    pub strict: bool,
    /// *IsSimpleParameterList*
    pub simple_parameters: bool,
    /// Der *BindingIdentifier* von Deklarationen und Ausdrücken.
    pub name: Option<Atom>,
    /// Muster, zuletzt vielleicht [`Node::Rest`].
    pub parameters: List,
    /// Die Anweisungen des Rumpfs, bei einer Pfeilfunktion mit
    /// `expression_body` nur der Ausdruck.
    pub body: List,
    pub expression_body: bool,
    /// Der Quelltext der Funktion (für `Function.prototype.toString`).
    pub range: Range<usize>,
}

/// Eine Klasse.
#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    pub name: Option<Atom>,
    pub heritage: Option<NodeId>,
    /// Der Konstruktor, wenn die Klasse einen angibt.
    pub constructor: Option<FunctionId>,
    /// Die übrigen Elemente: [`Node::ClassMethod`], [`Node::ClassField`]
    /// und [`Node::StaticBlock`].
    pub elements: List,
    pub range: Range<usize>,
}

/// Ein Script oder Modul als Syntaxbaum.
#[derive(Clone, Debug, Default)]
pub struct Ast {
    nodes: Vec<Node>,
    ranges: Vec<(u32, u32)>,
    lists: Vec<NodeId>,
    functions: Vec<Function>,
    classes: Vec<Class>,
    atoms: Vec<Rc<[u16]>>,
    atom_index: BTreeMap<Rc<[u16]>, Atom>,
    body: List,
    module: bool,
    strict: bool,
    overflow: bool,
}

impl Ast {
    pub fn node(&self, id: NodeId) -> &Node {
        // Knoten entstehen nur hier, ihre Indizes sind gültig.
        self.nodes.get(id.index()).unwrap_or(&Node::Empty)
    }

    /// Wo der Knoten im Quelltext steht, in Codeeinheiten.
    pub fn range(&self, id: NodeId) -> Range<usize> {
        let (start, end) = self.ranges.get(id.index()).copied().unwrap_or_default();
        to_usize(start)..to_usize(end)
    }

    pub fn list(&self, list: List) -> &[NodeId] {
        let start = to_usize(list.start);
        self.lists
            .get(start..start.saturating_add(list.len()))
            .unwrap_or_default()
    }

    pub fn atom(&self, atom: Atom) -> &[u16] {
        self.atoms.get(atom.index()).map_or(&[], |units| units)
    }

    /// Die Funktion `id`. Verweise stammen aus diesem Baum und sind gültig.
    pub fn function(&self, id: FunctionId) -> Option<&Function> {
        self.functions.get(to_usize(id.0))
    }

    /// Die Klasse `id`. Verweise stammen aus diesem Baum und sind gültig.
    pub fn class(&self, id: ClassId) -> Option<&Class> {
        self.classes.get(to_usize(id.0))
    }

    /// Die Anweisungen des Scripts oder Moduls.
    pub fn body(&self) -> &[NodeId] {
        self.list(self.body)
    }

    pub fn is_module(&self) -> bool {
        self.module
    }

    /// Ob der Code auf oberster Ebene strikt ist.
    pub fn is_strict(&self) -> bool {
        self.strict
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    // Aufbau, nur für den Parser

    /// Ob eine Tabelle mehr als `u32::MAX` Einträge bekommen sollte. Der
    /// Parser lehnt den Quelltext dann ab, die Verweise ab dieser Stelle
    /// stimmen nicht mehr.
    pub(crate) fn overflowed(&self) -> bool {
        self.overflow
    }

    /// Legt einen Knoten an. Der Quelltext ist höchstens `u32::MAX`
    /// Codeeinheiten lang, das prüft der Parser vorher.
    pub(crate) fn add(&mut self, node: Node, range: Range<usize>) -> NodeId {
        let Some(id) = self.next_id(self.nodes.len()) else {
            return NodeId(NonZeroU32::MIN);
        };
        self.nodes.push(node);
        self.ranges.push((to_u32(range.start), to_u32(range.end)));
        NodeId(id)
    }

    pub(crate) fn replace(&mut self, id: NodeId, node: Node) {
        if let Some(slot) = self.nodes.get_mut(id.index()) {
            *slot = node;
        }
    }

    pub(crate) fn set_range(&mut self, id: NodeId, range: Range<usize>) {
        if let Some(slot) = self.ranges.get_mut(id.index()) {
            *slot = (to_u32(range.start), to_u32(range.end));
        }
    }

    pub(crate) fn add_list(&mut self, items: &[NodeId]) -> List {
        let (Ok(start), Some(end)) = (
            u32::try_from(self.lists.len()),
            self.lists
                .len()
                .checked_add(items.len())
                .and_then(|end| u32::try_from(end).ok()),
        ) else {
            self.overflow = true;
            return List::default();
        };
        self.lists.extend_from_slice(items);
        List {
            start,
            len: end - start,
        }
    }

    pub(crate) fn add_function(&mut self, function: Function) -> FunctionId {
        match u32::try_from(self.functions.len()) {
            Ok(index) if index < u32::MAX => {
                self.functions.push(function);
                FunctionId(index)
            }
            _ => {
                self.overflow = true;
                FunctionId(0)
            }
        }
    }

    pub(crate) fn add_class(&mut self, class: Class) -> ClassId {
        match u32::try_from(self.classes.len()) {
            Ok(index) if index < u32::MAX => {
                self.classes.push(class);
                ClassId(index)
            }
            _ => {
                self.overflow = true;
                ClassId(0)
            }
        }
    }

    pub(crate) fn intern(&mut self, units: &[u16]) -> Atom {
        if let Some(atom) = self.atom_index.get(units) {
            return *atom;
        }
        let Some(id) = self.next_id(self.atoms.len()) else {
            return Atom(NonZeroU32::MIN);
        };
        let shared: Rc<[u16]> = Rc::from(units);
        self.atoms.push(Rc::clone(&shared));
        let atom = Atom(id);
        self.atom_index.insert(shared, atom);
        atom
    }

    pub(crate) fn intern_str(&mut self, text: &str) -> Atom {
        let units: Vec<u16> = text.encode_utf16().collect();
        self.intern(&units)
    }

    pub(crate) fn finish(&mut self, body: List, module: bool, strict: bool) {
        self.body = body;
        self.module = module;
        self.strict = strict;
    }

    /// Der Verweis auf einen neuen Eintrag hinter `length` Einträgen.
    fn next_id(&mut self, length: usize) -> Option<NonZeroU32> {
        let id = length
            .checked_add(1)
            .and_then(|id| u32::try_from(id).ok())
            .and_then(NonZeroU32::new);
        if id.is_none() {
            self.overflow = true;
        }
        id
    }
}

/// Lagen im Quelltext passen in `u32`, das prüft der Parser.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}
