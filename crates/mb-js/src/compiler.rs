//! Der Übersetzer vom Syntaxbaum zum Bytecode, für Scripts und Funktionen.
//! Was er noch nicht kann, meldet er als [`CompileError::Unsupported`] mit
//! der Lage im Quelltext.
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
//! Registern oder, wenn eine innere Funktion ihren Namen nennt, in
//! Kontexten: in Funktionen die Parameter, `arguments`, `var`,
//! Funktionsdeklarationen und `let` und `const` auf oberster Ebene
//! (FunctionDeclarationInstantiation, Abschnitt 10.2.11), überall die Bindungen in Blöcken, im Kopf von `for`
//! und in `switch`. Beim Betreten eines Blocks bekommen sie die Lücke
//! (BlockDeclarationInstantiation, Abschnitt 14.2.3). Gelesen und
//! geschrieben wird mit [`Op::CheckHole`] oder [`Op::ThrowIfHole`] davor,
//! außer wenn die Stelle im Quelltext hinter der Deklaration liegt und kein
//! `switch` dazwischen springen kann; dann ist die Bindung sicher
//! initialisiert. Aus einer inneren Funktion prüft jeder Zugriff auf eine
//! lexikalische Bindung die TDZ.
//!
//! Closures: Ein Bereich mit Bindungen in einem Kontext legt ihn beim
//! Betreten an ([`Op::PushContext`]) und gibt ihn beim Verlassen wieder ab,
//! auch bei `break` und `continue`. Eine Funktion bekommt den aktuellen
//! Kontext als [[Environment]] und mit ihrer [`FunctionTemplate`] die
//! Beschreibung der Kontexte um sie herum ([`Enclosing`]); daraus findet
//! der Übersetzer ihre Bindungen. Pfeilfunktionen lesen `this` aus einem
//! Platz im Kontext der Funktion oder des Scripts, in dem sie stehen,
//! `new.target` aus einem im Kontext der Funktion. Ein `for` mit `let`
//! kopiert seinen Kontext für jeden Durchlauf.
//!
//! Funktionsdeklarationen in Blöcken sind dort lexikalisch und entstehen
//! beim Betreten. Außerhalb des strikten Modus setzen sie nach Anhang B.3.2
//! und B.3.3 bei ihrer Auswertung zusätzlich die gleichnamige var-Bindung,
//! wenn eine `var`-Deklaration an ihrer Stelle keinen frühen Fehler ergäbe;
//! in Scripts entscheidet GlobalDeclarationInstantiation, ob es die
//! globale Bindung gibt, und setzt dafür ein Register.
//!
//! Noch nicht unterstützt sind `with`, `import()`,
//! BigInt-Literale, reguläre Ausdrücke, Templates mit Tag und
//! Funktionsdeklarationen mit Marke (Anhang B.3.1).
//!
//! Klassen: Eine Klasse hat einen eigenen Bereich mit ihrem Namen als
//! Konstante, die bis zum Ende der Klasse in der TDZ bleibt (classEnv). Darin
//! wertet sie die Oberklasse aus, dann legt [`Op::CreateClass`] Konstruktor
//! und Prototyp an, dann definiert sie die Methoden der Reihe nach, jede mit
//! ihrem Namen vor dem Funktionsobjekt. Eine Klassendeklaration bindet den
//! Namen danach wie `let`. Der Konstruktor einer Klasse ohne `constructor`
//! ist fester Bytecode ([`compile_default_constructor`]). Alle Teile einer
//! Klasse sind strikt; steht sie in nicht striktem Code, bilden ihre
//! Befehle dort einen strikten Bereich ([`Emitter::strict_range`]).
//!
//! Felder und private Namen: Die privaten Namen einer Klasse liegen in
//! Plätzen ihres Bereichs, innere Funktionen finden sie wie Bindungen über
//! [`Enclosing`]. Der Initialisierer eines Feldes und ein `static`-Block
//! sind eigene Funktionen. Jeder Konstruktor einer Klasse führt
//! InitializeInstanceElements als Schleife im Bytecode aus, ohne
//! Oberklasse zu Beginn, sonst nach jedem `super()`; so ruft er die
//! Initialisierer wie jede Funktion auf.
//!
//! `super`: Eine Referenz mit `super` liest erst `this`, dann den Namen,
//! dann die Basis aus [[HomeObject]] der Funktion (MakeSuperPropertyReference,
//! Abschnitt 13.3.7.3). Im Konstruktor einer abgeleiteten Klasse hält
//! `this` bis `super()` die Lücke; jedes Lesen prüft sie, und `super()`
//! bindet sein Ergebnis nur, wenn sie noch da ist (BindThisValue). Liest eine
//! Pfeilfunktion darin `this`, liegt es im Kontext, und der Konstruktor legt
//! es vor jedem `return` zurück in Register 0, wo [[Construct]] es prüft.
//! Pfeilfunktionen lesen das Funktionsobjekt für `super` aus einem Platz im
//! Kontext der Funktion, die sie umgibt.
//!
//! `try`: Der Block und `catch` bekommen je einen Eintrag in der
//! Ausnahmetabelle. Mit `finally` führen alle Wege hinaus durch dessen
//! Block, auch `break`, `continue` und `return`: Jeder legt in ein Register,
//! wie es danach weitergeht, und `finally` verzweigt an seinem Ende danach.
//! Liegen mehrere `finally` auf dem Weg, geht es so durch alle.
//!
//! Iteratoren: `for`-`of` und die Ausbreitung in Array-Literalen und
//! Aufrufen halten den Iterator Record in drei Registern
//! ([`Op::GetIterator`], [`Op::IteratorNext`]). Verlässt ein Weg `for`-`of`
//! vorzeitig, schließt er den Iterator: `break` direkt, `return` und
//! Sprünge zu äußeren Zielen über ein Ziel wie bei `finally`, eine
//! Ausnahme über einen Handler. `for await` hält nur [[Iterator]] und
//! [[NextMethod]] ([`Op::GetAsyncIterator`]) und wartet auf die Ergebnisse
//! von `next` und `return`. Ein Aufruf mit Ausbreitung sammelt seine
//! Argumente in einem Array ([`Op::CallSpread`]). `for`-`in` hält seinen
//! Aufzähler in einem Register ([`Op::ForInPrepare`], [`Op::ForInNext`])
//! und bindet jeden Schlüssel wie `for`-`of` seinen Wert; schließen muss es
//! nichts.
//!
//! Muster: Deklarationen, Parameter, `catch`, Zuweisungen und `for`-`of`
//! binden mit demselben Weg den Wert im Akkumulator; was sie
//! unterscheidet, ist, ob ein Name mit PutValue oder mit
//! InitializeReferencedBinding seinen Wert bekommt. Ein Element wertet
//! erst sein Ziel zur Referenz aus, dann holt es den Wert, dann den
//! Vorgabewert. Ein Array-Muster hält seinen Iterator Record in drei
//! Registern ([`Op::IteratorElement`], [`Op::IteratorSkip`]); bei einer
//! Ausnahme schließt ihn ein Handler, am Ende [`Op::IteratorClose`], beides
//! nur, solange [[Done]] nicht gilt. Ein Objekt-Muster mit Rest hält die
//! Schlüssel davor in Registern direkt hinter dem Wert
//! ([`Op::CreateRestObject`]). Innere Muster übersetzt der Übersetzer
//! rekursiv; ihre Tiefe begrenzt der Parser.
//!
//! Generatoren: Eine Generatorfunktion führt FunctionDeclarationInstantiation
//! aus wie jede Funktion, dann legt [`Op::CreateGenerator`] den Generator in
//! einem eigenen Register an und kehrt mit ihm zurück; der Rumpf dahinter
//! läuft erst mit `next`. Nach jedem [`Op::Yield`] verzweigt
//! [`Op::ResumeGenerator`]: mit `next` weiter im Ausdruck, mit `throw` in die
//! Handler, mit `return` auf dem Weg von `return` durch `finally` und das
//! Schließen der Iteratoren von `for`-`of` und Array-Mustern. `yield*`
//! hält den Iterator Record in drei Registern und wiederholt
//! [`Op::YieldStar`] und [`Op::YieldResult`], bis der innere Iterator
//! fertig ist. Aufrufe in Generatoren stehen nie in Endposition.
//!
//! async-Funktionen: Ihr Code beginnt mit [`Op::BeginAsync`], noch vor
//! FunctionDeclarationInstantiation. Ein Handler um alles danach lehnt das
//! Promise mit jeder Ausnahme ab, auch aus den Parametern
//! ([`Op::RejectAsync`]); jedes `return` und das Ende des Rumpfs lösen es
//! auf ([`Op::ResolveAsync`]). `await` ist [`Op::Await`], gefolgt von
//! [`Op::ResumeAsync`]. Aufrufe in async-Funktionen stehen nie in
//! Endposition.
//!
//! async-Generatoren: Ihr Code beginnt wie der eines Generators, ohne
//! [`Op::BeginAsync`]; Ausnahmen aus FunctionDeclarationInstantiation gehen
//! an den Aufrufer (EvaluateAsyncGeneratorBody, Abschnitt 15.6.2). `await`
//! ist dasselbe Paar wie in async-Funktionen, `return` mit einem Ausdruck
//! wartet auf seinen Wert. `yield` wartet auf seinen Wert, dann folgt
//! [`Op::AsyncYield`] mit demselben Weg nach dem Halt wie bei Generatoren.
//! `yield*` hält den Iterator von [`Op::GetAsyncIterator`] und die Art des
//! Durchlaufs in drei Registern und wiederholt [`Op::AsyncYieldStar`],
//! [`Op::Await`], [`Op::ResumeAsync`], [`Op::AsyncYieldStarResult`] und
//! [`Op::AsyncYield`], bis der innere Iterator fertig ist.
//!
//! Abschlusswert: Scripts führen ein Register dafür (UpdateEmpty, Abschnitt
//! 6.2.4.4). Jede Ausdrucksanweisung schreibt ihren Wert hinein. `if`,
//! Schleifen, `switch` und `try` liefern nie ~empty~, sondern ohne eigenen
//! Wert *undefined*; sie setzen das Register darum zu Beginn auf
//! *undefined*, `catch` und `finally` ebenso. Endet `finally` normal, gilt
//! wieder der Wert von davor. Funktionen brauchen keinen Abschlusswert.
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
//! `this` auf. `new` legt den Konstruktor und die Argumente genauso ab,
//! `this` setzt erst [`Op::Construct`].
//!
//! Endaufrufe: Im strikten Modus wird ein Aufruf in Endposition
//! (IsInTailPosition, Abschnitt 15.10.1) zu [`Op::TailCall`]. In Endposition
//! stehen Aufrufe im Ausdruck von `return` und im Rumpf einer
//! Pfeilfunktion ohne Block, nach HasCallInTailPosition (Abschnitt
//! 15.10.2) auch hinter Klammern, Komma, `? :`, `&&`, `||` und `??`, nicht
//! aber im Block von `try`, mit `finally` auch nicht in `catch`, und nicht
//! im Rumpf von `for`-`of`.
//!
//! Tiefe: Ketten zweistelliger Operatoren und Ketten von Zugriffen und
//! Aufrufen wie `a.b(c)[d]?.e` baut der Parser in Schleifen, sie werden
//! beliebig tief. Der Übersetzer liest sie mit einem eigenen Stapel oder
//! entlang der Kette. Alles andere begrenzt der Parser mit `MAX_DEPTH`, das
//! übersetzt er rekursiv (RS-8).

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::ast::{
    AssignOperator, Ast, Atom, BinaryOperator, ClassId, DeclarationKind, FunctionCaptures,
    FunctionId, FunctionKind, List, MethodKind, Node, NodeId, PropertyKind, UnaryOperator,
    UpdateOperator,
};
use crate::bytecode::{
    BindingKind, Code, Constant, ContextBinding, ContextLayout, Emitter, Enclosing,
    FunctionTemplate, HandlerKind, Label, Op, TooLarge,
};
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
    /// Die Funktionsdeklarationen in Blöcken, die nach Anhang B.3.2.2 auch
    /// eine globale Bindung setzen, wenn es sie geben darf: ihr Name und das
    /// Register, das dann zu Beginn *true* hält.
    pub hoisted_functions: Vec<(Vec<u16>, u32)>,
}

/// Register für `this`.
const THIS: i64 = 0;
/// Register für den Abschlusswert eines Scripts.
const COMPLETION: i64 = 1;

/// Wie es nach `finally` weitergeht: normal hinter `try`, mit der Ausnahme
/// wieder werfen oder ab [`FIRST_EXIT`] auf einem der Wege hinaus.
const NORMAL: i64 = 0;
const THROW: i64 = 1;
const FIRST_EXIT: usize = 2;

/// Wo eine Bindung liegt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Location {
    Register(u32),
    /// Ein Platz im Kontext so viele Stufen außerhalb des aktuellen, wie
    /// `depth` sagt. Im Bereich selbst, der den Kontext hat, ist `depth` 0.
    Context {
        depth: u32,
        slot: u32,
    },
}

#[derive(Clone, Copy, Debug)]
struct LocalBinding {
    location: Location,
    kind: BindingKind,
    /// Ab hier im Quelltext ist die Bindung initialisiert, wenn kein
    /// `switch` dorthin springt.
    initialized_from: usize,
    in_switch: bool,
}

impl LocalBinding {
    /// Eine Bindung, die von Anfang an einen Wert hat: Parameter, `var`,
    /// Funktionen und der Name eines Funktionsausdrucks.
    fn initialized(location: Location, kind: BindingKind) -> LocalBinding {
        LocalBinding {
            location,
            kind,
            initialized_from: 0,
            in_switch: false,
        }
    }

    /// Ob die Bindung vor ihrer Initialisierung die Lücke hält.
    fn is_lexical(self) -> bool {
        self.in_switch || self.initialized_from > 0
    }

    fn at_depth(self, depth: u32) -> LocalBinding {
        match self.location {
            Location::Register(_) => self,
            Location::Context { slot, .. } => LocalBinding {
                location: Location::Context { depth, slot },
                ..self
            },
        }
    }
}

#[derive(Debug, Default)]
struct BlockScope {
    bindings: BTreeMap<Atom, LocalBinding>,
    /// Die Plätze im eigenen Kontext, ohne den für den äußeren; ohne
    /// Plätze hat der Bereich keinen Kontext.
    slots: u32,
    /// Der Platz mit `this`.
    this: Option<u32>,
    /// Der Platz mit `new.target`.
    new_target: Option<u32>,
    /// Der Platz mit dem Funktionsobjekt, für `super` in Pfeilfunktionen.
    function: Option<u32>,
    /// Ob der Platz mit `this` die Lücke halten kann.
    derived: bool,
    /// Die Plätze mit den privaten Namen einer Klasse.
    privates: BTreeMap<Atom, u32>,
}

impl BlockScope {
    fn has_context(&self) -> bool {
        self.slots > 0
    }

    /// Ein neuer Platz im Kontext.
    fn add_slot(&mut self) -> Result<u32> {
        self.slots = self.slots.checked_add(1).ok_or(CompileError::TooLarge)?;
        Ok(self.slots)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetKind {
    Loop,
    Switch,
    /// Eine Anweisung mit Marke, aus der nur `break` mit dieser Marke
    /// herausführt.
    Labeled,
    /// Ein `try` mit `finally`: Kein Sprung zielt hierher, aber wer den
    /// Block oder `catch` verlässt, geht erst durch `finally`.
    Finally,
}

/// Ein Weg aus Anweisungen hinaus, der an `finally` vorbeikommen kann.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exit {
    /// `break` oder `continue` zum Ziel mit dieser Stelle in `targets`.
    Jump { target: usize, is_continue: bool },
    /// `return` mit dem Wert im Akkumulator.
    Return,
}

/// Was links in `for`-`of` steht.
#[derive(Clone, Copy, Debug)]
enum ForTarget {
    /// `let` oder `const` mit einem Bezeichner oder Muster.
    Lexical(NodeId),
    /// `var` mit einem Bezeichner oder Muster.
    Var(NodeId),
    /// Ein Ausdruck oder ein Muster als Ziel einer Zuweisung.
    Expression,
}

/// Wie die Namen in einem Muster ihren Wert bekommen.
#[derive(Clone, Copy, Debug)]
enum Init {
    /// Mit PutValue über eine Referenz: in Zuweisungen
    /// (DestructuringAssignmentEvaluation, Abschnitt 13.15.5.2) und mit
    /// `var` (BindingInitialization mit *undefined* als Umgebung, Abschnitt
    /// 8.6.2).
    Put,
    /// Mit InitializeReferencedBinding im Bereich mit der Stelle in
    /// `scopes`; hat der den Namen nicht, ist es eine lexikalische Bindung
    /// der globalen Umgebung.
    Initialize(Option<usize>),
}

/// Das Ziel eines Elements in einem Muster, vor dem Wert ausgewertet.
#[derive(Clone, Copy, Debug)]
enum Leaf {
    /// PutValue, mit der Stelle des Ziels im Quelltext für die TDZ.
    Put(Reference, usize),
    /// InitializeReferencedBinding.
    Initialize(Atom),
}

/// Woher ein Element eines Musters seinen Wert holt.
#[derive(Clone, Copy, Debug)]
enum Fetch {
    /// Aus dem Iterator Record ab dem Register.
    Iterator(i64),
    /// GetV mit festem Namen auf dem Wert im Register.
    Named { object: i64, name: i64 },
    /// GetV mit dem Schlüssel aus dem zweiten Register.
    Keyed { object: i64, key: i64 },
}

/// Ein Ziel von `break` und `continue`.
#[derive(Debug)]
struct Target {
    kind: TargetKind,
    labels: Vec<Atom>,
    breaks: Vec<Label>,
    continues: Vec<Label>,
    /// Wie viele Kontexte am Ziel offen sind.
    contexts: u32,
    /// Bei `finally` die Register für die Fortsetzung danach und für den
    /// Wert von `return` und `throw`. Die Sprünge hinein stehen in
    /// `breaks`.
    finally: Option<(u32, u32)>,
    /// Bei `finally` die Wege hinaus, die dort weitergehen; der Weg mit der
    /// Stelle `n` hat die Fortsetzung `FIRST_EXIT + n`.
    exits: Vec<Exit>,
}

impl Target {
    fn new(kind: TargetKind, labels: Vec<Atom>) -> Target {
        Target {
            kind,
            labels,
            breaks: Vec::new(),
            continues: Vec::new(),
            contexts: 0,
            finally: None,
            exits: Vec::new(),
        }
    }
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
    /// Eine Eigenschaft mit `super` und festem Namen: die Basis aus
    /// GetSuperBase im Register, `this` im Register danach.
    SuperNamed {
        base: u32,
        name: i64,
    },
    /// Dasselbe mit berechnetem Namen im dritten Register ab der Basis.
    SuperKeyed {
        base: u32,
    },
    /// Eine private Referenz (MakePrivateReference, Abschnitt 6.2.5.9): die
    /// Basis im Register, der private Name im Register danach.
    Private {
        base: u32,
        name: u32,
    },
}

/// Was IsAnonymousFunctionDefinition (Abschnitt 8.4.3) fand.
#[derive(Clone, Copy)]
enum Anonymous {
    Function(NodeId, FunctionId),
    Class(NodeId, ClassId),
}

/// Was ein Glied einer Kette von Zugriffen liest.
#[derive(Clone, Copy)]
enum Access {
    /// `.name`
    Named(Atom),
    /// `[expression]`
    Keyed(NodeId),
    /// `.#name`
    Private(Atom),
}

/// Die Register einer Klasse während ihrer Elemente: der Prototyp, der
/// Konstruktor und, wenn die Instanzen private Methoden haben, das Objekt
/// für [[PrivateMethods]].
#[derive(Clone, Copy)]
struct ClassRegisters {
    prototype: u32,
    constructor: u32,
    holder: Option<u32>,
}

/// Ein statisches Feld oder ein `static`-Block, ausgewertet nach den
/// übrigen Elementen der Klasse (ClassDefinitionEvaluation, Abschnitt
/// 15.7.14, Schritt 33).
enum StaticElement {
    Field {
        node: NodeId,
        key: FieldKey,
        value: Option<FunctionId>,
    },
    Block {
        node: NodeId,
        function: FunctionId,
    },
}

/// Der Name eines statischen Feldes, bis es definiert wird.
enum FieldKey {
    Name(Vec<u16>),
    Private(Atom),
    /// Ein berechneter Name nach ToPropertyKey im Register.
    Register(u32),
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
    /// Was der Code vom umgebenden sieht.
    outer: Rc<Enclosing>,
    /// Was innere Funktionen dieses Codes sehen.
    captures: &'a FunctionCaptures,
    /// Ob `this` und `new.target` aus dem umgebenden Code kommen, wie in
    /// Pfeilfunktionen.
    lexical_this: bool,
    /// Das Register mit `new.target`, in Funktionen, die es lesen.
    new_target: Option<u32>,
    /// Das Register mit dem Generator, in Generatorfunktionen.
    generator: Option<u32>,
    /// Das Register mit dem Zustand, in async-Funktionen.
    async_state: Option<u32>,
    /// Ob der Code der eines async-Generators ist, mit dem Register aus
    /// `generator` und ohne Zustand.
    async_generator: bool,
    /// Ob der Code der Konstruktor einer abgeleiteten Klasse ist.
    derived: bool,
    /// Die Aufrufe in Endposition im Ausdruck, der gerade übersetzt wird.
    tail_calls: BTreeSet<NodeId>,
    /// Wie viele Bereiche um die aktuelle Stelle eine Ausnahme fangen oder
    /// `finally` haben; dort gibt es keine Endaufrufe.
    protected: u32,
    /// Wo in `scopes` die Bindungen aus `var` einer Funktion liegen.
    var_scope: Option<usize>,
    /// Die Funktionsdeklarationen in Blöcken, die nach Anhang B.3.2 auch die
    /// var-Bindung setzen, in Scripts mit dem Register, das sagt, ob es die
    /// globale Bindung gibt.
    hoisted: BTreeMap<NodeId, Option<u32>>,
    /// Ob der Übersetzer gerade eine Klasse in nicht striktem Code
    /// übersetzt; ihre Befehle bilden einen strikten Bereich.
    strict_class: bool,
}

/// Übersetzt ein Script (Abschnitt 16.1).
pub fn compile_script(ast: &Ast) -> Result<Script> {
    if ast.is_module() {
        return Err(CompileError::Unsupported(0));
    }
    let captures = ast.captures().script();
    let mut compiler = Compiler::new(ast, ast.is_strict(), Rc::default(), captures);
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
    let mut hoisted_functions = Vec::new();
    for (name, node) in compiler.hoisted_functions(body, &BTreeSet::new()) {
        let flag = compiler.allocate()?;
        compiler.hoisted.insert(node, Some(flag));
        hoisted_functions.push((ast.atom(name).to_vec(), flag));
    }
    // Pfeilfunktionen lesen `this` aus einem Kontext.
    if captures.this {
        let mut scope = BlockScope::default();
        let slot = scope.add_slot()?;
        scope.this = Some(slot);
        compiler.emit(Op::PushContext, &[1])?;
        compiler.emit(Op::Ldar, &[THIS])?;
        compiler.emit(Op::StaContext, &[0, i64::from(slot)])?;
        compiler.scopes.push(scope);
    }
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
        hoisted_functions,
    })
}

/// Ein formaler Parameter mit einem Bezeichner.
struct Formal {
    /// Der Bezeichner oder das Muster.
    target: NodeId,
    /// Der Name, wenn das Ziel ein Bezeichner ist.
    atom: Option<Atom>,
    /// BoundNames des Ziels.
    names: Vec<Atom>,
    default: Option<NodeId>,
    rest: bool,
    /// Das Ende des Parameters im Quelltext.
    end: usize,
}

/// Übersetzt eine Funktion für ihren ersten Aufruf, mit
/// FunctionDeclarationInstantiation (Abschnitt 10.2.11). Die Register:
/// `this`, das Funktionsobjekt, die Parameter ohne den Rest-Parameter,
/// dann die Argumente als interne Zelle, wenn das arguments-Objekt oder der
/// Rest-Parameter sie braucht, dann `new.target`, wenn der Code es liest,
/// dann die übrigen Bindungen. Was innere Funktionen davon sehen, kommt in
/// einen Kontext, die Parameter mit einer Kopie.
///
/// Ohne Vorgabewerte teilen sich Parameter, `arguments`, `var`, die
/// Funktionen und die lexikalischen Deklarationen des Rumpfs einen
/// Bereich. Mit Vorgabewerten haben die Parameter und `arguments` einen
/// eigenen Bereich außerhalb dessen des Rumpfs; dort hält ein Parameter bis
/// zu seiner Initialisierung die Lücke, und ein `var` mit seinem Namen
/// beginnt mit seinem Wert.
///
/// Das arguments-Objekt entsteht nur, wenn die Funktion oder eine
/// Pfeilfunktion darin den Bezeichner `arguments` enthält; sonst sieht es
/// niemand. Ist es verknüpft, liegen die Parameter im Kontext, denn das
/// Objekt liest und schreibt ihre Bindungen dort.
///
/// Konstruktoren von Klassen übersetzt es wie Methoden; im Konstruktor einer
/// abgeleiteten Klasse beginnt `this` als Lücke. Der Konstruktor einer
/// Klasse ohne Oberklasse beginnt mit InitializeInstanceElements
/// ([[Construct]], Abschnitt 10.2.2, Schritt 6.b), noch vor den
/// Parametern. Initialisierer von Feldern und `static`-Blöcke sind
/// Funktionen ohne Parameter.
pub fn compile_function(ast: &Ast, template: &FunctionTemplate) -> Result<Code> {
    let Some(function) = ast.function(template.function) else {
        return Err(CompileError::Unsupported(0));
    };
    let position = function.range.start;
    let arrow = function.kind == FunctionKind::Arrow;
    let supported = matches!(
        function.kind,
        FunctionKind::Normal
            | FunctionKind::Arrow
            | FunctionKind::Method
            | FunctionKind::Getter
            | FunctionKind::Setter
            | FunctionKind::Constructor
            | FunctionKind::DerivedConstructor
            | FunctionKind::FieldInitializer
            | FunctionKind::StaticBlock
    );
    let captures = ast.captures().function(template.function);
    let (true, Some(captures)) = (supported, captures) else {
        return Err(CompileError::Unsupported(position));
    };
    let mut compiler = Compiler::new(ast, function.strict, Rc::clone(&template.outer), captures);
    compiler.lexical_this = arrow;
    compiler.derived = function.kind == FunctionKind::DerivedConstructor;
    compiler.allocate()?;
    let callee = compiler.allocate()?;
    let formals = compiler.formals(function.parameters)?;
    let has_rest = formals.iter().any(|formal| formal.rest);
    // Die Register, in die ein Aufruf die Argumente legt.
    let mut registers = Vec::new();
    for _ in formals.iter().filter(|formal| !formal.rest) {
        registers.push(compiler.allocate()?);
    }
    let positional = i64::try_from(registers.len()).map_err(|_| CompileError::TooLarge)?;
    // ContainsExpression (Abschnitt 15.1.2): Vorgabewerte, auch in Mustern,
    // und berechnete Namen in Mustern.
    let has_expressions = formals
        .iter()
        .any(|formal| formal.default.is_some() || compiler.contains_expression(formal.target));
    let parameter_names: BTreeSet<Atom> = formals
        .iter()
        .flat_map(|formal| formal.names.iter().copied())
        .collect();
    // Der Rumpf einer Pfeilfunktion mit `expression_body` ist ein Ausdruck;
    // er deklariert nichts.
    let body = ast.list(function.body);
    let statements = if function.expression_body { &[] } else { body };
    let functions = compiler.top_level_functions(statements)?;
    let mut var_names = Vec::new();
    for &item in statements {
        compiler.var_declared_names(item, &mut var_names)?;
    }
    let items: Vec<NodeId> = statements
        .iter()
        .copied()
        .filter(|&item| !matches!(ast.node(item), Node::FunctionDeclaration(_)))
        .collect();
    let mut lexical_names = Vec::new();
    for &item in &items {
        lexical_names.extend(compiler.lexical_declarations(item)?);
    }
    // argumentsObjectNeeded: Pfeilfunktionen haben kein Objekt. Ein
    // Parameter mit dem Namen ersetzt es, ohne Vorgabewerte auch eine
    // Funktion oder eine lexikalische Deklaration, `var` nie.
    let needed = !arrow
        && !parameter_names
            .iter()
            .any(|&atom| compiler.is_arguments(atom))
        && (has_expressions
            || !functions
                .iter()
                .any(|&(name, _, _)| compiler.is_arguments(name))
                && !lexical_names
                    .iter()
                    .any(|&(atom, _, _)| compiler.is_arguments(atom)));
    let arguments_atom = ast.find_atom(&units("arguments"));
    let created = arguments_atom.filter(|_| needed && captures.arguments);
    let mapped = created.is_some() && !function.strict && function.simple_parameters;
    let cell = if created.is_some() || has_rest {
        Some(i64::from(compiler.allocate()?))
    } else {
        None
    };
    if captures.new_target && !arrow {
        compiler.new_target = Some(compiler.allocate()?);
    }
    if function.generator {
        compiler.generator = Some(compiler.allocate()?);
    }
    if function.is_async && function.generator {
        compiler.async_generator = true;
    } else if function.is_async {
        compiler.async_state = Some(compiler.allocate()?);
    }
    // Anhang B.3.2.1: Zu parameterNames gehört dann auch `arguments`
    // (test262 block-decl-func-skip-arguments).
    let mut excluded = parameter_names;
    if needed && let Some(atom) = arguments_atom {
        excluded.insert(atom);
    }
    let hoisted = compiler.hoisted_functions(statements, &excluded);
    let function_names: Vec<Atom> = functions.iter().map(|&(name, _, _)| name).collect();
    for &(name, node) in &hoisted {
        // Ohne Objekt und ohne var-Bindung entstünde die Bindung erst bei
        // der Auswertung.
        if compiler.is_arguments(name)
            && !var_names.contains(&name)
            && !function_names.contains(&name)
        {
            return Err(compiler.unsupported(node));
        }
        compiler.hoisted.insert(node, None);
    }
    // Der Bereich der Parameter, ohne Vorgabewerte auch der des Rumpfs.
    let mut parameters = BlockScope::default();
    let mut holes = Vec::new();
    let mut copies = Vec::new();
    if has_expressions {
        for formal in &formals {
            for &atom in &formal.names {
                let location = compiler.locate(&mut parameters, atom)?;
                holes.push(location);
                let binding = LocalBinding {
                    location,
                    kind: BindingKind::Mutable,
                    initialized_from: formal.end,
                    in_switch: false,
                };
                parameters.bindings.insert(atom, binding);
            }
        }
    } else {
        for (formal, &register) in formals.iter().zip(&registers) {
            let Some(atom) = formal.atom else {
                continue;
            };
            // Bei doppelten Namen gilt der letzte Parameter.
            let binding =
                LocalBinding::initialized(Location::Register(register), BindingKind::Mutable);
            parameters.bindings.insert(atom, binding);
        }
        for (atom, binding) in &mut parameters.bindings {
            if let Location::Register(register) = binding.location
                && (mapped || captures.names.contains(atom))
            {
                let slot = parameters
                    .slots
                    .checked_add(1)
                    .ok_or(CompileError::TooLarge)?;
                parameters.slots = slot;
                binding.location = Location::Context { depth: 0, slot };
                copies.push((register, slot));
            }
        }
        // Der Rest-Parameter und die Namen in Mustern bekommen eigene Orte.
        // Ohne Ausdrücke in den Parametern sieht sie niemand vor ihrer
        // Initialisierung.
        for formal in formals
            .iter()
            .filter(|formal| formal.rest || formal.atom.is_none())
        {
            for &atom in &formal.names {
                let location = compiler.locate(&mut parameters, atom)?;
                let binding = LocalBinding::initialized(location, BindingKind::Mutable);
                parameters.bindings.insert(atom, binding);
            }
        }
    }
    // Ein Parameter, dessen Name später wiederkommt, ist nicht verknüpft.
    let mut parameter_map = Vec::new();
    if mapped {
        let slots: BTreeMap<u32, u32> = copies.iter().copied().collect();
        parameter_map = registers
            .iter()
            .map(|register| slots.get(register).copied())
            .collect();
    }
    if let Some(atom) = created {
        let location = compiler.locate(&mut parameters, atom)?;
        let binding = LocalBinding::initialized(location, BindingKind::Mutable);
        parameters.bindings.insert(atom, binding);
    }
    // Die Bindungen des Rumpfs: Mit Vorgabewerten liegen sie in einem
    // eigenen Bereich, dann beginnt ein `var` mit dem Namen eines Parameters
    // mit dessen Wert.
    let mut separate = BlockScope::default();
    let mut var_copies = Vec::new();
    let names = var_names
        .iter()
        .map(|&atom| (atom, true))
        .chain(function_names.iter().map(|&atom| (atom, false)))
        .chain(hoisted.iter().map(|&(atom, _)| (atom, false)));
    for (atom, var) in names {
        let copy = has_expressions
            && var
            && !function_names.contains(&atom)
            && parameters.bindings.contains_key(&atom);
        let scope = if has_expressions {
            &mut separate
        } else {
            &mut parameters
        };
        if scope.bindings.contains_key(&atom) {
            continue;
        }
        let location = compiler.locate(scope, atom)?;
        let binding = LocalBinding::initialized(location, BindingKind::Mutable);
        scope.bindings.insert(atom, binding);
        if copy {
            var_copies.push(atom);
        }
    }
    // Die lexikalischen Deklarationen auf oberster Ebene teilen sich den
    // Bereich, denn keine hat denselben Namen wie ein Parameter, `var` oder
    // eine Funktion.
    let mut lexical_holes = Vec::new();
    for (atom, constant, end) in lexical_names {
        let scope = if has_expressions {
            &mut separate
        } else {
            &mut parameters
        };
        let location = compiler.locate(scope, atom)?;
        lexical_holes.push(location);
        let kind = if constant {
            BindingKind::Const
        } else {
            BindingKind::Mutable
        };
        let binding = LocalBinding {
            location,
            kind,
            initialized_from: end,
            in_switch: false,
        };
        scope.bindings.insert(atom, binding);
    }
    if captures.this && !arrow {
        parameters.this = Some(parameters.add_slot()?);
        parameters.derived = compiler.derived;
    }
    if captures.arrow_new_target && !arrow {
        parameters.new_target = Some(parameters.add_slot()?);
    }
    if captures.arrow_super && !arrow {
        parameters.function = Some(parameters.add_slot()?);
    }
    // EvaluateAsyncFunctionBody und EvaluateAsyncConciseBody (Abschnitte
    // 15.8.4 und 15.9.3): Die Capability entsteht vor
    // FunctionDeclarationInstantiation, und jede Ausnahme danach, aus den
    // Parametern wie aus dem Rumpf, lehnt ihr Promise ab.
    let async_body = match compiler.async_state {
        Some(_) => {
            compiler.emit(Op::BeginAsync, &[])?;
            Some(compiler.emitter.offset())
        }
        None => None,
    };
    // Alle Bindungen haben ihren Ort; die Register der Schleife über die
    // Felder sind danach wieder frei.
    if function.kind == FunctionKind::Constructor {
        let first = compiler.allocate()?;
        for _ in 0..4 {
            compiler.allocate()?;
        }
        let register = i64::from(first);
        compiler.emit(Op::Mov, &[i64::from(callee), register])?;
        compiler.emit(Op::Mov, &[THIS, register + 3])?;
        emit_instance_elements(&mut compiler.emitter, register)?;
        compiler.release(first);
    }
    // Der Name eines Funktionsausdrucks liegt außen (funcEnv, Abschnitt
    // 15.2.5); das Objekt `arguments` und jede Bindung im Bereich der
    // Parameter verdecken ihn.
    if template.binds_name
        && let Some(name) = function.name
        && !(needed && compiler.is_arguments(name))
        && !parameters.bindings.contains_key(&name)
    {
        let mut outer = BlockScope::default();
        let location = if captures.names.contains(&name) {
            let slot = outer.add_slot()?;
            compiler.emit(Op::PushContext, &[1])?;
            compiler.emit(Op::Ldar, &[i64::from(callee)])?;
            compiler.emit(Op::StaContext, &[0, i64::from(slot)])?;
            Location::Context { depth: 0, slot }
        } else {
            Location::Register(callee)
        };
        let binding = LocalBinding::initialized(location, BindingKind::FunctionName);
        outer.bindings.insert(name, binding);
        compiler.scopes.push(outer);
    }
    if parameters.has_context() {
        compiler.emit(Op::PushContext, &[i64::from(parameters.slots)])?;
        for (register, slot) in copies {
            compiler.emit(Op::Ldar, &[i64::from(register)])?;
            compiler.emit(Op::StaContext, &[0, i64::from(slot)])?;
        }
        if let Some(slot) = parameters.this {
            compiler.emit(Op::Ldar, &[THIS])?;
            compiler.emit(Op::StaContext, &[0, i64::from(slot)])?;
        }
        if let (Some(slot), Some(register)) = (parameters.new_target, compiler.new_target) {
            compiler.emit(Op::Ldar, &[i64::from(register)])?;
            compiler.emit(Op::StaContext, &[0, i64::from(slot)])?;
        }
        if let Some(slot) = parameters.function {
            compiler.emit(Op::Ldar, &[i64::from(callee)])?;
            compiler.emit(Op::StaContext, &[0, i64::from(slot)])?;
        }
    }
    compiler.set_holes(&holes)?;
    let parameter_scope = compiler.scopes.len();
    compiler.scopes.push(parameters);
    if let (Some(cell), Some(atom)) = (cell, created) {
        let op = if mapped {
            Op::CreateMappedArguments
        } else {
            Op::CreateArguments
        };
        compiler.emit(op, &[cell])?;
        compiler.store_binding(parameter_scope, atom)?;
    }
    // IteratorBindingInitialization der Parameter (Abschnitt 8.6.3), mit
    // Vorgabewert für ein fehlendes oder *undefined* übergebenes Argument.
    for (index, formal) in formals.iter().enumerate() {
        if formal.rest {
            if let Some(cell) = cell {
                compiler.emit(Op::CreateRestParameter, &[cell, positional])?;
                compiler.bind_parameter(parameter_scope, formal)?;
            }
            continue;
        }
        // Ohne Ausdrücke in den Parametern liegt ein Bezeichner schon im
        // Register seines Arguments.
        if !has_expressions && formal.atom.is_some() {
            continue;
        }
        let Some(&register) = registers.get(index) else {
            return Err(CompileError::Unsupported(position));
        };
        compiler.emit(Op::Ldar, &[i64::from(register)])?;
        if let Some(default) = formal.default {
            let skip = compiler.jump(Op::JumpIfNotUndefined)?;
            // NamedEvaluation nur für einen Bezeichner.
            match formal.atom {
                Some(atom) => compiler.named_expression(default, ast.atom(atom))?,
                None => compiler.expression(default)?,
            }
            compiler.bind(skip)?;
        }
        compiler.bind_parameter(parameter_scope, formal)?;
    }
    if has_expressions {
        if separate.has_context() {
            compiler.emit(Op::PushContext, &[i64::from(separate.slots)])?;
        }
        compiler.scopes.push(separate);
        let var_scope = compiler.scopes.len().saturating_sub(1);
        for atom in var_copies {
            let (Some(from), Some(to)) = (
                compiler.lookup_in(parameter_scope, atom),
                compiler.lookup_in(var_scope, atom),
            ) else {
                return Err(CompileError::Unsupported(position));
            };
            compiler.load_location(from.location)?;
            compiler.store_location(to.location)?;
        }
    }
    compiler.set_holes(&lexical_holes)?;
    compiler.var_scope = Some(compiler.scopes.len().saturating_sub(1));
    for &(name, id, node) in &functions {
        compiler.function_declaration(node, id)?;
        if let Some(binding) = compiler.lookup(name) {
            compiler.store_location(binding.location)?;
        }
    }
    // EvaluateGeneratorBody und EvaluateAsyncGeneratorBody (Abschnitte
    // 15.5.2 und 15.6.2): Nach FunctionDeclarationInstantiation kehrt der
    // Aufruf mit dem Generator zurück; der Rumpf läuft erst mit `next`.
    if compiler.generator.is_some() {
        compiler.emit(Op::CreateGenerator, &[])?;
    }
    if function.expression_body {
        for &expression in body {
            if function.kind == FunctionKind::FieldInitializer {
                compiler.field_initializer(expression)?;
                continue;
            }
            compiler.tail_calls = compiler.tail_position_calls(expression);
            compiler.expression(expression)?;
        }
        compiler.tail_calls.clear();
        compiler.emit_return()?;
    } else {
        for &item in body {
            compiler.top_level_item(item)?;
        }
        compiler.emit(Op::LdaUndefined, &[])?;
        compiler.emit_return()?;
    }
    if let Some(start) = async_body {
        let end = compiler.emitter.offset();
        compiler.emitter.handler(start, end, HandlerKind::Catch);
        compiler.emit(Op::RejectAsync, &[])?;
        compiler.emit(Op::Return, &[])?;
    }
    let new_target = compiler.new_target;
    let generator = compiler.generator;
    let async_state = compiler.async_state;
    let async_generator = compiler.async_generator;
    let derived = compiler.derived;
    let mut code = compiler.finish();
    code.parameters = u32::try_from(registers.len()).map_err(|_| CompileError::TooLarge)?;
    code.arguments = cell.and_then(|cell| u32::try_from(cell).ok());
    code.parameter_map = parameter_map;
    code.new_target = new_target;
    code.generator = generator;
    code.async_state = async_state;
    code.async_generator = async_generator;
    code.derived = derived;
    Ok(code)
}

/// Der Code des Konstruktors einer Klasse ohne `constructor`
/// (ClassDefinitionEvaluation, Abschnitt 15.7.14, Schritt 14.a), mit
/// `derived` für eine abgeleitete Klasse. Ohne Oberklasse gibt er das neue
/// Objekt zurück wie `constructor() {}`. Sonst ruft er wie
/// `constructor(...args) { super(...args); }` den Konstruktor der
/// Oberklasse mit den Argumenten und new.target auf, aber ohne
/// %Array.prototype%[%Symbol.iterator%]: Das Array mit den Argumenten
/// liest der Aufruf direkt. Sein Ergebnis gibt er zurück; es ist stets ein
/// Objekt. Beide führen davor InitializeInstanceElements aus.
pub fn compile_default_constructor(derived: bool) -> Result<Code> {
    let mut emitter = Emitter::default();
    if !derived {
        // Register: `this`, das Funktionsobjekt, dann die der Schleife über
        // die Felder.
        emitter.emit(Op::Mov, &[1, 2])?;
        emitter.emit(Op::Mov, &[THIS, 5])?;
        emit_instance_elements(&mut emitter, 2)?;
        emitter.emit(Op::LdaUndefined, &[])?;
        emitter.emit(Op::Return, &[])?;
        return Ok(emitter.finish(Vec::new(), 7, true));
    }
    // Register: `this`, das Funktionsobjekt, die Argumente, new.target,
    // dann new.target, Funktion, `this` und das Array für den Aufruf.
    // Danach nimmt die Schleife über die Felder die Register ab 3.
    let (arguments, new_target, callee, array) = (2, 3, 5, 7);
    emitter.emit(Op::Ldar, &[1])?;
    emitter.emit(Op::GetSuperConstructor, &[])?;
    emitter.emit(Op::Star, &[callee])?;
    emitter.emit(Op::Ldar, &[new_target])?;
    emitter.emit(Op::Star, &[callee - 1])?;
    emitter.emit(Op::CreateRestParameter, &[arguments, 0])?;
    emitter.emit(Op::Star, &[array])?;
    emitter.emit(Op::SuperCallSpread, &[callee])?;
    emitter.emit(Op::Star, &[6])?;
    emitter.emit(Op::Mov, &[1, 3])?;
    emit_instance_elements(&mut emitter, 3)?;
    emitter.emit(Op::Ldar, &[6])?;
    emitter.emit(Op::Return, &[])?;
    let mut code = emitter.finish(Vec::new(), 8, true);
    code.arguments = Some(2);
    code.new_target = Some(3);
    code.derived = true;
    Ok(code)
}

/// InitializeInstanceElements (Abschnitt 7.3.33) für den Konstruktor im
/// Register `first` und das Objekt im Register `first + 3`: erst die
/// privaten Methoden, dann die Felder der Reihe nach, jedes mit einem
/// Aufruf seines Initialisierers mit dem Objekt als `this` und DefineField
/// (Abschnitt 7.3.32). Die Register `first + 1`, `first + 2` und
/// `first + 4` nimmt die Schleife.
fn emit_instance_elements(emitter: &mut Emitter, first: i64) -> std::result::Result<(), TooLarge> {
    let register = |offset: i64| first.checked_add(offset).ok_or(TooLarge);
    let object = register(3)?;
    emitter.emit(Op::InitializePrivateMethods, &[first, object])?;
    emitter.emit(Op::LdaSmi, &[0])?;
    emitter.emit(Op::Star, &[register(1)?])?;
    let start = emitter.offset();
    let done = emitter.jump_with(Op::NextField, &[first])?;
    let define = emitter.jump(Op::JumpIfNullish)?;
    emitter.emit(Op::Call, &[register(2)?, 0])?;
    emitter.bind(define)?;
    emitter.emit(Op::DefineField, &[object, register(4)?])?;
    emitter.jump_loop(start)?;
    emitter.bind(done)
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl<'a> Compiler<'a> {
    fn new(
        ast: &'a Ast,
        strict: bool,
        outer: Rc<Enclosing>,
        captures: &'a FunctionCaptures,
    ) -> Compiler<'a> {
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
            outer,
            captures,
            lexical_this: false,
            new_target: None,
            generator: None,
            async_state: None,
            async_generator: false,
            derived: false,
            tail_calls: BTreeSet::new(),
            protected: 0,
            var_scope: None,
            hoisted: BTreeMap::new(),
            strict_class: false,
        }
    }

    /// Ob der Code an der aktuellen Stelle strikt ist: im ganzen Code oder
    /// in einer Klasse (Abschnitt 11.2.2).
    fn is_strict(&self) -> bool {
        self.strict || self.strict_class
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
    /// eine, jede Funktion und jeder Standardkonstruktor bekommt eine eigene.
    fn constant(&mut self, constant: Constant) -> Result<i64> {
        let key = match &constant {
            Constant::Number(number) => Some(ConstantKey::Number(number.to_bits())),
            Constant::String(units) => Some(ConstantKey::String(units.clone())),
            Constant::Name(units) => Some(ConstantKey::Name(units.clone())),
            Constant::Function(_) | Constant::DefaultConstructor { .. } => None,
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
                    let mut names = Vec::new();
                    self.bound_names(target, &mut names);
                    let end = self.ast.range(declarator).end;
                    for atom in names {
                        bindings.push((atom, kind == DeclarationKind::Const, end));
                    }
                }
            }
            Node::Variables { .. } => {}
            // Wie `let` (Abschnitt 15.7.15).
            Node::ClassDeclaration(id) => {
                let Some(name) = self.ast.class(id).and_then(|class| class.name) else {
                    return Err(self.unsupported(item));
                };
                bindings.push((name, false, self.ast.range(item).end));
            }
            Node::FunctionDeclaration(_) => return Err(self.unsupported(item)),
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
                    let mut bound = Vec::new();
                    self.bound_names(target, &mut bound);
                    for atom in bound {
                        add(atom);
                    }
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
            Node::ForOf { left, body, .. } | Node::ForIn { left, body, .. } => {
                self.var_declared_names(left, names)?;
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
            Node::Try {
                block,
                handler,
                finalizer,
            } => {
                self.var_declared_names(block, names)?;
                if let Some(handler) = handler
                    && let Node::Catch { body, .. } = *self.ast.node(handler)
                {
                    self.var_declared_names(body, names)?;
                }
                if let Some(finalizer) = finalizer {
                    self.var_declared_names(finalizer, names)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Anhang B.3.2: die Funktionsdeklarationen in Blöcken, in `case`- und
    /// `default`-Klauseln und nach Anhang B.3.3 in den Zweigen von `if`, die
    /// außerhalb des strikten Modus zusätzlich die gleichnamige var-Bindung
    /// setzen, mit ihrem Namen. Das gilt, wenn eine `var`-Deklaration an
    /// ihrer Stelle keinen frühen Fehler ergäbe: Keine lexikalische
    /// Deklaration auf dem Weg nach außen hat denselben Namen, auch keine
    /// zweite Funktion im selben Block, und keiner der Namen in `excluded`
    /// (parameterNames) ist es.
    fn hoisted_functions(&self, body: &[NodeId], excluded: &BTreeSet<Atom>) -> Vec<(Atom, NodeId)> {
        let mut found = Vec::new();
        if self.strict {
            return found;
        }
        let mut path = vec![self.lexically_declared(body, false)];
        for &item in body {
            self.hoisting_walk(item, &mut path, excluded, &mut found);
        }
        found
    }

    /// Sucht in einer Anweisung nach Funktionsdeklarationen in Blöcken,
    /// ohne in Funktionen hinabzusteigen. `path` hält die lexikalisch
    /// deklarierten Namen der Bereiche außen.
    fn hoisting_walk(
        &self,
        node: NodeId,
        path: &mut Vec<Vec<Atom>>,
        excluded: &BTreeSet<Atom>,
        found: &mut Vec<(Atom, NodeId)>,
    ) {
        match *self.ast.node(node) {
            Node::Block(list) => self.hoisting_block(self.ast.list(list), path, excluded, found),
            Node::If {
                consequent,
                alternate,
                ..
            } => {
                for clause in std::iter::once(consequent).chain(alternate) {
                    if matches!(self.ast.node(clause), Node::FunctionDeclaration(_)) {
                        self.hoisting_block(&[clause], path, excluded, found);
                    } else {
                        self.hoisting_walk(clause, path, excluded, found);
                    }
                }
            }
            Node::While { body, .. } | Node::DoWhile { body, .. } | Node::Labeled { body, .. } => {
                self.hoisting_walk(body, path, excluded, found);
            }
            Node::For { init, body, .. } => {
                let names =
                    init.map_or_else(Vec::new, |init| self.lexically_declared(&[init], false));
                path.push(names);
                self.hoisting_walk(body, path, excluded, found);
                path.pop();
            }
            Node::ForOf { left, body, .. } | Node::ForIn { left, body, .. } => {
                path.push(self.lexically_declared(&[left], false));
                self.hoisting_walk(body, path, excluded, found);
                path.pop();
            }
            Node::Switch { cases, .. } => {
                let mut items = Vec::new();
                for &case in self.ast.list(cases) {
                    if let Node::Case { body, .. } = *self.ast.node(case) {
                        items.extend_from_slice(self.ast.list(body));
                    }
                }
                self.hoisting_block(&items, path, excluded, found);
            }
            // Ein Bezeichner als Parameter von `catch` steht einer
            // `var`-Deklaration im Block nicht im Weg, die Namen eines
            // Musters schon (Anhang B.3.4).
            Node::Try {
                block,
                handler,
                finalizer,
            } => {
                self.hoisting_walk(block, path, excluded, found);
                if let Some(handler) = handler
                    && let Node::Catch { parameter, body } = *self.ast.node(handler)
                {
                    let mut names = Vec::new();
                    if let Some(parameter) = parameter
                        && !matches!(self.ast.node(parameter), Node::BindingIdentifier(_))
                    {
                        self.bound_names(parameter, &mut names);
                    }
                    path.push(names);
                    self.hoisting_walk(body, path, excluded, found);
                    path.pop();
                }
                if let Some(finalizer) = finalizer {
                    self.hoisting_walk(finalizer, path, excluded, found);
                }
            }
            // Was der Übersetzer sonst nicht kann, übersetzt er gar nicht.
            _ => {}
        }
    }

    fn hoisting_block(
        &self,
        items: &[NodeId],
        path: &mut Vec<Vec<Atom>>,
        excluded: &BTreeSet<Atom>,
        found: &mut Vec<(Atom, NodeId)>,
    ) {
        path.push(self.lexically_declared(items, true));
        for &item in items {
            let Node::FunctionDeclaration(id) = *self.ast.node(item) else {
                self.hoisting_walk(item, path, excluded, found);
                continue;
            };
            let Some(name) = self
                .ast
                .function(id)
                .filter(|function| !function.generator && !function.is_async)
                .and_then(|function| function.name)
            else {
                continue;
            };
            let conflict = path.split_last().is_some_and(|(own, outer)| {
                own.iter().filter(|&&atom| atom == name).count() > 1
                    || outer.iter().any(|names| names.contains(&name))
            });
            if !conflict && !excluded.contains(&name) {
                found.push((name, item));
            }
        }
        path.pop();
    }

    /// Die Namen aus LexicallyDeclaredNames der Anweisungen, mit den
    /// Funktionen, wenn sie dort lexikalisch sind.
    fn lexically_declared(&self, items: &[NodeId], functions: bool) -> Vec<Atom> {
        let mut names = Vec::new();
        for &item in items {
            match *self.ast.node(item) {
                Node::Variables { kind, declarators } if kind != DeclarationKind::Var => {
                    for &declarator in self.ast.list(declarators) {
                        if let Node::Declarator { target, .. } = *self.ast.node(declarator) {
                            self.bound_names(target, &mut names);
                        }
                    }
                }
                Node::ClassDeclaration(id) => {
                    names.extend(self.ast.class(id).and_then(|class| class.name));
                }
                Node::FunctionDeclaration(id) if functions => {
                    names.extend(self.ast.function(id).and_then(|function| function.name));
                }
                _ => {}
            }
        }
        names
    }

    /// Ein Ort für eine Bindung namens `atom` im Bereich `scope`: ein Platz
    /// in seinem Kontext, wenn eine innere Funktion den Namen nennt, sonst
    /// ein Register.
    fn locate(&mut self, scope: &mut BlockScope, atom: Atom) -> Result<Location> {
        if self.captures.names.contains(&atom) {
            let slot = scope.add_slot()?;
            return Ok(Location::Context { depth: 0, slot });
        }
        Ok(Location::Register(self.allocate()?))
    }

    /// Gibt den Bindungen an `locations` im neuen Bereich die Lücke.
    fn set_holes(&mut self, locations: &[Location]) -> Result<()> {
        if locations.is_empty() {
            return Ok(());
        }
        self.emit(Op::LdaHole, &[])?;
        for &location in locations {
            self.store_location(location)?;
        }
        Ok(())
    }

    /// Betritt einen Block mit den Deklarationen aus `items` und gibt die
    /// Marke für [`Compiler::leave_block`] zurück: BlockDeclarationInstantiation
    /// (Abschnitt 14.2.3), die Funktionen darin entstehen sofort.
    fn enter_block(&mut self, items: &[NodeId], in_switch: bool) -> Result<u32> {
        let mark = self.next_register;
        let mut scope = BlockScope::default();
        let mut holes = Vec::new();
        let mut functions = Vec::new();
        for &item in items {
            if let Node::FunctionDeclaration(id) = *self.ast.node(item) {
                let name = self.block_function(item, id)?;
                // Außerhalb des strikten Modus darf eine Funktion im selben
                // Block wiederkommen, die letzte gilt (Anhang B.3.2.4).
                if !scope.bindings.contains_key(&name) {
                    let location = self.locate(&mut scope, name)?;
                    let binding = LocalBinding::initialized(location, BindingKind::Mutable);
                    scope.bindings.insert(name, binding);
                }
                functions.push((name, id, item));
                continue;
            }
            for (atom, constant, end) in self.lexical_declarations(item)? {
                let location = self.locate(&mut scope, atom)?;
                holes.push(location);
                let kind = if constant {
                    BindingKind::Const
                } else {
                    BindingKind::Mutable
                };
                scope.bindings.insert(
                    atom,
                    LocalBinding {
                        location,
                        kind,
                        initialized_from: end,
                        in_switch,
                    },
                );
            }
        }
        if scope.has_context() {
            self.emit(Op::PushContext, &[i64::from(scope.slots)])?;
        }
        self.set_holes(&holes)?;
        self.scopes.push(scope);
        for (name, id, node) in functions {
            self.function_declaration(node, id)?;
            if let Some(binding) = self.lookup(name) {
                self.store_location(binding.location)?;
            }
        }
        Ok(mark)
    }

    fn leave_block(&mut self, mark: u32) -> Result<()> {
        if self.scopes.pop().is_some_and(|scope| scope.has_context()) {
            self.emit(Op::PopContext, &[])?;
        }
        self.release(mark);
        Ok(())
    }

    /// Der Name einer Funktionsdeklaration in einem Block.
    fn block_function(&self, node: NodeId, id: FunctionId) -> Result<Atom> {
        self.ast
            .function(id)
            .and_then(|function| function.name)
            .ok_or_else(|| self.unsupported(node))
    }

    /// Wie viele Bereiche einen eigenen Kontext haben.
    fn open_contexts(&self) -> u32 {
        let count = self
            .scopes
            .iter()
            .filter(|scope| scope.has_context())
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    fn lookup(&self, atom: Atom) -> Option<LocalBinding> {
        let mut depth: u32 = 0;
        for scope in self.scopes.iter().rev() {
            if let Some(&binding) = scope.bindings.get(&atom) {
                return Some(binding.at_depth(depth));
            }
            if scope.has_context() {
                depth = depth.saturating_add(1);
            }
        }
        None
    }

    /// Die Bindung `atom` im Bereich mit der Stelle `index` in `scopes`.
    fn lookup_in(&self, index: usize, atom: Atom) -> Option<LocalBinding> {
        let binding = *self.scopes.get(index)?.bindings.get(&atom)?;
        let inner = self.scopes.get(index.checked_add(1)?..)?;
        let depth = inner.iter().filter(|scope| scope.has_context()).count();
        Some(binding.at_depth(u32::try_from(depth).ok()?))
    }

    /// Initialisiert die Bindung `atom` im Bereich mit der Stelle `index` in
    /// `scopes` mit dem Akkumulator.
    fn store_binding(&mut self, index: usize, atom: Atom) -> Result<()> {
        let Some(binding) = self.lookup_in(index, atom) else {
            return Err(CompileError::Unsupported(0));
        };
        self.store_location(binding.location)
    }

    /// Initialisiert den Parameter `formal` im Bereich mit der Stelle
    /// `index` in `scopes` mit dem Akkumulator. Parameter mit Mustern haben
    /// nie doppelte Namen und darum stets eine Umgebung.
    fn bind_parameter(&mut self, index: usize, formal: &Formal) -> Result<()> {
        match formal.atom {
            Some(atom) => self.store_binding(index, atom),
            None => self.destructure(formal.target, Init::Initialize(Some(index))),
        }
    }

    /// Die formalen Parameter einer Funktion, jeder ein Bezeichner oder
    /// ein Muster, vielleicht mit Vorgabewert, zuletzt vielleicht der
    /// Rest-Parameter.
    fn formals(&self, list: List) -> Result<Vec<Formal>> {
        let mut formals = Vec::new();
        for &parameter in self.ast.list(list) {
            let (target, default, rest) = match *self.ast.node(parameter) {
                Node::WithDefault { target, default } => (target, Some(default), false),
                Node::Rest(target) => (target, None, true),
                _ => (parameter, None, false),
            };
            let atom = match *self.ast.node(target) {
                Node::BindingIdentifier(atom) => Some(atom),
                Node::ArrayPattern(_) | Node::ObjectPattern(_) => None,
                _ => return Err(self.unsupported(target)),
            };
            let mut names = Vec::new();
            self.bound_names(target, &mut names);
            let end = self.ast.range(parameter).end;
            formals.push(Formal {
                target,
                atom,
                names,
                default,
                rest,
                end,
            });
        }
        Ok(formals)
    }

    /// BoundNames (Abschnitt 8.2.1) eines Bezeichners oder Bindungsmusters
    /// in der Reihenfolge des Quelltexts, angehängt an `names`; ohne
    /// Rekursion.
    fn bound_names(&self, target: NodeId, names: &mut Vec<Atom>) {
        let mut work = vec![target];
        while let Some(node) = work.pop() {
            match *self.ast.node(node) {
                Node::BindingIdentifier(atom) => names.push(atom),
                Node::ArrayPattern(list) | Node::ObjectPattern(list) => {
                    work.extend(self.ast.list(list).iter().rev());
                }
                Node::PatternProperty { value, .. } => work.push(value),
                Node::WithDefault { target, .. } | Node::Rest(target) => work.push(target),
                _ => {}
            }
        }
    }

    /// ContainsExpression (Abschnitt 15.1.2) eines Bezeichners oder
    /// Bindungsmusters: ob darin ein Vorgabewert oder ein berechneter Name
    /// steht.
    fn contains_expression(&self, target: NodeId) -> bool {
        let mut work = vec![target];
        while let Some(node) = work.pop() {
            match *self.ast.node(node) {
                Node::WithDefault { .. } => return true,
                Node::PatternProperty { key, value, .. } => {
                    if matches!(self.ast.node(key), Node::ComputedKey(_)) {
                        return true;
                    }
                    work.push(value);
                }
                Node::ArrayPattern(list) | Node::ObjectPattern(list) => {
                    work.extend_from_slice(self.ast.list(list));
                }
                Node::Rest(target) => work.push(target),
                _ => {}
            }
        }
        false
    }

    fn is_arguments(&self, atom: Atom) -> bool {
        self.ast
            .atom(atom)
            .iter()
            .copied()
            .eq("arguments".encode_utf16())
    }

    /// ResolveBinding (Abschnitt 9.4.2) beim Übersetzen: eine Bindung in
    /// einem Register oder Kontext oder `None` für die globale Umgebung.
    fn resolve(&self, atom: Atom, node: NodeId) -> Result<Option<LocalBinding>> {
        if let Some(binding) = self.lookup(atom) {
            return Ok(Some(binding));
        }
        let mut depth = self.open_contexts();
        let mut enclosing = Some(&self.outer);
        while let Some(current) = enclosing {
            for layout in &current.contexts {
                if let Some(binding) = layout.bindings.get(&atom) {
                    let initialized_from = if binding.lexical { usize::MAX } else { 0 };
                    return Ok(Some(LocalBinding {
                        location: Location::Context {
                            depth,
                            slot: binding.slot,
                        },
                        kind: binding.kind,
                        initialized_from,
                        in_switch: false,
                    }));
                }
                depth = depth.checked_add(1).ok_or(CompileError::TooLarge)?;
            }
            if current.registers.contains(&atom) {
                return Err(self.unsupported(node));
            }
            enclosing = current.parent.as_ref();
        }
        Ok(None)
    }

    /// ResolvePrivateIdentifier (Abschnitt 9.2.1.2) beim Übersetzen: der
    /// Platz des privaten Namens im Kontext so viele Stufen außerhalb des
    /// aktuellen, wie die erste Zahl sagt. Der Parser lässt nur Namen zu,
    /// die eine Klasse um die Stelle deklariert.
    fn private_location(&self, atom: Atom, node: NodeId) -> Result<(u32, u32)> {
        let mut depth: u32 = 0;
        for scope in self.scopes.iter().rev() {
            if let Some(&slot) = scope.privates.get(&atom) {
                return Ok((depth, slot));
            }
            if scope.has_context() {
                depth = depth.checked_add(1).ok_or(CompileError::TooLarge)?;
            }
        }
        let mut enclosing = Some(&self.outer);
        while let Some(current) = enclosing {
            for layout in &current.contexts {
                if let Some(&slot) = layout.privates.get(&atom) {
                    return Ok((depth, slot));
                }
                depth = depth.checked_add(1).ok_or(CompileError::TooLarge)?;
            }
            enclosing = current.parent.as_ref();
        }
        Err(self.unsupported(node))
    }

    /// Lädt den privaten Namen `atom` in den Akkumulator.
    fn load_private_name(&mut self, atom: Atom, node: NodeId) -> Result<()> {
        let (depth, slot) = self.private_location(atom, node)?;
        self.emit(Op::LdaContext, &[i64::from(depth), i64::from(slot)])
    }

    /// Was eine Funktion sähe, die hier entsteht: die offenen Kontexte vom
    /// innersten an und dahinter, was dieser Code vom umgebenden sieht.
    fn enclosing(&self) -> Rc<Enclosing> {
        let mut contexts = Vec::new();
        let mut registers = BTreeSet::new();
        for scope in self.scopes.iter().rev() {
            let mut layout = ContextLayout {
                bindings: BTreeMap::new(),
                this: scope.this,
                new_target: scope.new_target,
                function: scope.function,
                derived: scope.derived,
                privates: scope.privates.clone(),
            };
            for (&atom, &binding) in &scope.bindings {
                match binding.location {
                    Location::Register(_) => {
                        registers.insert(atom);
                    }
                    Location::Context { slot, .. } => {
                        let binding = ContextBinding {
                            slot,
                            kind: binding.kind,
                            lexical: binding.is_lexical(),
                        };
                        layout.bindings.insert(atom, binding);
                    }
                }
            }
            if scope.has_context() {
                contexts.push(layout);
            }
        }
        if contexts.is_empty() && registers.is_empty() {
            return Rc::clone(&self.outer);
        }
        Rc::new(Enclosing {
            contexts,
            registers,
            parent: Some(Rc::clone(&self.outer)),
        })
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
                self.leave_block(mark)
            }
            Node::FunctionDeclaration(_) => self.block_function_evaluation(node),
            Node::ClassDeclaration(id) => self.class_declaration(node, id),
            Node::Empty | Node::Debugger => Ok(()),
            Node::If {
                test,
                consequent,
                alternate,
            } => self.if_statement(test, consequent, alternate),
            Node::While { .. }
            | Node::DoWhile { .. }
            | Node::For { .. }
            | Node::ForOf { .. }
            | Node::ForIn { .. }
            | Node::Switch { .. } => self.breakable(node, Vec::new()),
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
                    Some(argument) => {
                        self.tail_calls = self.tail_position_calls(argument);
                        let result = self.expression(argument);
                        self.tail_calls.clear();
                        result?;
                        // Abschnitt 14.10.1: In einem async-Generator wartet
                        // `return` auf seinen Wert.
                        if self.async_generator {
                            self.position(node)?;
                            self.emit(Op::Await, &[])?;
                            self.emit(Op::ResumeAsync, &[])?;
                        }
                    }
                    None => self.emit(Op::LdaUndefined, &[])?,
                }
                self.position(node)?;
                self.exit(Exit::Return)
            }
            Node::Try {
                block,
                handler,
                finalizer,
            } => self.try_statement(block, handler, finalizer),
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

    /// Eine Funktionsdeklaration in einem Block: Das Funktionsobjekt
    /// entstand beim Betreten. Nach Anhang B.3.2 kommt es hier außerdem in
    /// die var-Bindung der Funktion oder die globale Bindung des Scripts,
    /// wenn GlobalDeclarationInstantiation sie angelegt hat.
    fn block_function_evaluation(&mut self, node: NodeId) -> Result<()> {
        let Some(&hoisted) = self.hoisted.get(&node) else {
            return Ok(());
        };
        let name = match *self.ast.node(node) {
            Node::FunctionDeclaration(id) => self.block_function(node, id)?,
            _ => return Err(self.unsupported(node)),
        };
        let Some(binding) = self.lookup(name) else {
            return Err(self.unsupported(node));
        };
        match hoisted {
            Some(flag) => {
                self.emit(Op::Ldar, &[i64::from(flag)])?;
                let skip = self.jump(Op::JumpIfFalse)?;
                let constant = self.name(name)?;
                let register = self.allocate()?;
                self.emit(Op::ResolveGlobal, &[constant, i64::from(register)])?;
                self.load_location(binding.location)?;
                self.emit(Op::StaGlobal, &[constant, i64::from(register)])?;
                self.release(register);
                self.bind(skip)
            }
            None => {
                let target = self
                    .var_scope
                    .and_then(|index| self.lookup_in(index, name))
                    .ok_or_else(|| self.unsupported(node))?;
                self.load_location(binding.location)?;
                self.store_location(target.location)
            }
        }
    }

    /// `var`, `let` und `const` (Abschnitte 14.3.1.2 und 14.3.2.1).
    fn variables(&mut self, kind: DeclarationKind, declarators: List) -> Result<()> {
        let ast = self.ast;
        for &declarator in ast.list(declarators) {
            let Node::Declarator { target, init } = *self.ast.node(declarator) else {
                return Err(self.unsupported(declarator));
            };
            let Node::BindingIdentifier(atom) = *self.ast.node(target) else {
                // Ein Muster hat stets einen Initialisierer, ohne
                // NamedEvaluation; dann bindet es mit dessen Wert.
                let Some(init) = init else {
                    return Err(self.unsupported(declarator));
                };
                let mode = match kind {
                    DeclarationKind::Var => Init::Put,
                    DeclarationKind::Let | DeclarationKind::Const => {
                        Init::Initialize(self.scopes.len().checked_sub(1))
                    }
                    DeclarationKind::Using | DeclarationKind::AwaitUsing => {
                        return Err(self.unsupported(declarator));
                    }
                };
                self.expression(init)?;
                self.position(declarator)?;
                self.destructure(target, mode)?;
                continue;
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
                        Some(binding) => self.store_location(binding.location)?,
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
        self.clause(consequent)?;
        match alternate {
            Some(alternate) => {
                let end = self.jump(Op::Jump)?;
                self.bind(otherwise)?;
                self.clause(alternate)?;
                self.bind(end)
            }
            None => self.bind(otherwise),
        }
    }

    /// Ein Zweig von `if`. Eine Funktionsdeklaration dort steht nach Anhang
    /// B.3.3 wie in einem eigenen Block.
    fn clause(&mut self, node: NodeId) -> Result<()> {
        if !matches!(self.ast.node(node), Node::FunctionDeclaration(_)) {
            return self.statement(node);
        }
        self.position(node)?;
        let mark = self.enter_block(&[node], false)?;
        self.statement(node)?;
        self.leave_block(mark)
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
            Node::While { .. }
            | Node::DoWhile { .. }
            | Node::For { .. }
            | Node::ForOf { .. }
            | Node::ForIn { .. }
            | Node::Switch { .. } => self.breakable(body, labels),
            // Anhang B.3.1.
            Node::FunctionDeclaration(_) => Err(self.unsupported(body)),
            _ => {
                self.push_target(Target::new(TargetKind::Labeled, labels));
                self.statement(body)?;
                let target = self.pop_target();
                self.bind_all(target.breaks)
            }
        }
    }

    /// Ein Ziel, an dem so viele Kontexte offen sind wie jetzt.
    fn push_target(&mut self, mut target: Target) {
        target.contexts = self.open_contexts();
        self.targets.push(target);
    }

    fn pop_target(&mut self) -> Target {
        self.targets
            .pop()
            .unwrap_or(Target::new(TargetKind::Labeled, Vec::new()))
    }

    /// Schleifen und `switch` mit ihren Marken.
    fn breakable(&mut self, node: NodeId, labels: Vec<Atom>) -> Result<()> {
        self.position(node)?;
        let kind = if matches!(self.ast.node(node), Node::Switch { .. }) {
            TargetKind::Switch
        } else {
            TargetKind::Loop
        };
        let target = Target::new(kind, labels);
        match *self.ast.node(node) {
            Node::While { test, body } => self.while_loop(target, test, body),
            Node::DoWhile { body, test } => self.do_while_loop(target, body, test),
            Node::For {
                init,
                test,
                update,
                body,
            } => self.for_loop(target, init, test, update, body),
            Node::ForOf {
                left,
                right,
                body,
                is_await,
            } => self.for_of_loop(node, target, left, right, body, is_await),
            Node::ForIn { left, right, body } => self.for_in_loop(node, target, left, right, body),
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
        self.push_target(target);
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

    /// `for` (Abschnitt 14.7.4.2). Liegen die Bindungen aus `let` im Kopf in
    /// einem Kontext, bekommt jeder Durchlauf eine Kopie davon
    /// (CreatePerIterationEnvironment): vor dem ersten Test und vor jedem
    /// Schritt danach. Bindungen in Registern sieht keine innere Funktion,
    /// sie brauchen keine Kopie.
    fn for_loop(
        &mut self,
        target: Target,
        init: Option<NodeId>,
        test: Option<NodeId>,
        update: Option<NodeId>,
        body: NodeId,
    ) -> Result<()> {
        let mut per_iteration = false;
        let mark = match init {
            Some(init) if matches!(self.ast.node(init), Node::Variables { .. }) => {
                let mark = self.enter_block(&[init], false)?;
                self.statement(init)?;
                per_iteration = matches!(
                    self.ast.node(init),
                    Node::Variables {
                        kind: DeclarationKind::Let,
                        ..
                    }
                ) && self.scopes.last().is_some_and(BlockScope::has_context);
                if per_iteration {
                    self.emit(Op::CloneContext, &[])?;
                }
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
        if per_iteration {
            self.emit(Op::CloneContext, &[])?;
        }
        if let Some(update) = update {
            self.expression(update)?;
        }
        self.emitter.jump_loop(start)?;
        if let Some(exit) = exit {
            self.bind(exit)?;
        }
        self.bind_all(target.breaks)?;
        if let Some(mark) = mark {
            self.leave_block(mark)?;
        }
        Ok(())
    }

    /// `for`-`of` (Abschnitte 14.7.5.6 und 14.7.5.7). Mit `let` oder `const`
    /// wertet die rechte Seite in einem Bereich aus, in dem die Namen im Kopf
    /// in der TDZ bleiben, und jeder Durchlauf bekommt einen neuen Bereich für
    /// sie.
    ///
    /// Wer die Schleife mit `break` verlässt, schließt den Iterator
    /// ([`Op::IteratorClose`]), `continue` zur Schleife selbst nicht. Die
    /// übrigen Wege hinaus, `return` und Sprünge zu äußeren Zielen, gehen
    /// durch ein Ziel der Art [`TargetKind::Finally`], das ihn schließt und
    /// sie danach weiterführt. Eine Ausnahme beim Binden oder im Rumpf
    /// schließt ihn über einen Handler ([`Op::IteratorCloseOnThrow`]) und
    /// wird dann wieder geworfen, eine aus `next` selbst nicht.
    ///
    /// `for await` gibt es in async-Funktionen und async-Generatoren. Es
    /// holt den Iterator mit [`Op::GetAsyncIterator`] und wartet auf jedes
    /// Ergebnis von `next`, bevor [`Op::IteratorResult`] es prüft.
    /// Geschlossen wird mit AsyncIteratorClose (Abschnitt 7.4.15), das auf
    /// das Ergebnis von `return` wartet; bei einer Ausnahme fängt ein
    /// zweiter Handler, was das Schließen wirft, und es gilt die Ausnahme
    /// davor.
    fn for_of_loop(
        &mut self,
        node: NodeId,
        target: Target,
        left: NodeId,
        right: NodeId,
        body: NodeId,
        is_await: bool,
    ) -> Result<()> {
        if is_await && !self.is_async() {
            return Err(self.unsupported(node));
        }
        let lhs = self.for_of_target(left)?;
        let mark = self.next_register;
        if let ForTarget::Lexical(_) = lhs {
            let tdz = self.enter_tdz(left)?;
            self.expression(right)?;
            self.leave_block(tdz)?;
        } else {
            self.expression(right)?;
        }
        self.position(right)?;
        let iterator = self.allocate()?;
        self.allocate()?;
        if is_await {
            self.emit(Op::GetAsyncIterator, &[i64::from(iterator)])?;
        } else {
            self.allocate()?;
            self.emit(Op::GetIterator, &[i64::from(iterator)])?;
        }
        let context = i64::from(self.allocate()?);
        self.emit(Op::SaveContext, &[context])?;
        let kind = self.allocate()?;
        let value = self.allocate()?;
        let item = i64::from(self.allocate()?);
        let mut close = Target::new(TargetKind::Finally, Vec::new());
        close.finally = Some((kind, value));
        self.push_target(close);
        self.reset_completion()?;
        let start = self.emitter.offset();
        self.position(node)?;
        let done = if is_await {
            self.emit(Op::AsyncIteratorNext, &[i64::from(iterator)])?;
            self.emit(Op::Await, &[])?;
            self.emit(Op::ResumeAsync, &[])?;
            self.jump(Op::IteratorResult)?
        } else {
            self.emitter
                .jump_with(Op::IteratorNext, &[i64::from(iterator)])?
        };
        let region = self.emitter.offset();
        // HasCallInTailPosition (Abschnitt 15.10.2.1): nie im Rumpf von
        // `for`-`of`, denn nach `return` ist der Iterator noch zu schließen.
        self.protected = self
            .protected
            .checked_add(1)
            .ok_or(CompileError::TooLarge)?;
        self.push_target(target);
        let block = self.for_of_binding(left, lhs, item)?;
        self.statement(body)?;
        if let Some(block) = block {
            self.leave_block(block)?;
        }
        let mut target = self.pop_target();
        self.protected = self.protected.saturating_sub(1);
        self.bind_all(std::mem::take(&mut target.continues))?;
        self.emitter.jump_loop(start)?;
        let end = self.emitter.offset();
        let (iterator, kind, value) = (i64::from(iterator), i64::from(kind), i64::from(value));
        self.emitter.handler(region, end, HandlerKind::Finally);
        self.emit(Op::RestoreContext, &[context])?;
        self.emit(Op::Star, &[value])?;
        if is_await {
            let closing = self.emitter.offset();
            let skip = self
                .emitter
                .jump_with(Op::AsyncIteratorClose, &[iterator])?;
            self.emit(Op::Await, &[])?;
            self.emit(Op::ResumeAsync, &[])?;
            let closed = self.emitter.offset();
            self.bind(skip)?;
            self.emitter.handler(closing, closed, HandlerKind::Finally);
        } else {
            self.emit(Op::IteratorCloseOnThrow, &[iterator])?;
        }
        self.emit(Op::Ldar, &[value])?;
        self.emit(Op::Rethrow, &[])?;
        let close = self.pop_target();
        let mut after = vec![done];
        if !target.breaks.is_empty() {
            self.bind_all(target.breaks)?;
            self.close_iterator(iterator, is_await)?;
            after.push(self.jump(Op::Jump)?);
        }
        if !close.breaks.is_empty() {
            self.bind_all(close.breaks)?;
            self.close_iterator(iterator, is_await)?;
            self.dispatch_exits(kind, value, close.exits)?;
        }
        self.bind_all(after)?;
        self.release(mark);
        Ok(())
    }

    /// Schließt den Iterator von `for`-`of` ab dem Register `iterator` für
    /// einen Weg hinaus, der keine Ausnahme ist. Für `for await` wartet es
    /// auf das Ergebnis von `return`, das ein Objekt sein muss.
    fn close_iterator(&mut self, iterator: i64, is_await: bool) -> Result<()> {
        if !is_await {
            return self.emit(Op::IteratorClose, &[iterator]);
        }
        let skip = self
            .emitter
            .jump_with(Op::AsyncIteratorClose, &[iterator])?;
        self.emit(Op::Await, &[])?;
        self.emit(Op::ResumeAsync, &[])?;
        self.emit(Op::ThrowIfNotObject, &[])?;
        self.bind(skip)
    }

    /// Was links in `for`-`of` steht. Eine Deklaration hat genau einen
    /// Bezeichner oder ein Muster ohne Initialisierer; ein Ausdruck, der
    /// keine Referenz ergibt, scheitert erst in [`Compiler::reference`].
    fn for_of_target(&self, left: NodeId) -> Result<ForTarget> {
        let Node::Variables { kind, declarators } = *self.ast.node(left) else {
            return Ok(ForTarget::Expression);
        };
        let &[declarator] = self.ast.list(declarators) else {
            return Err(self.unsupported(left));
        };
        let Node::Declarator { target, init: None } = *self.ast.node(declarator) else {
            return Err(self.unsupported(declarator));
        };
        match kind {
            DeclarationKind::Var => Ok(ForTarget::Var(target)),
            DeclarationKind::Let | DeclarationKind::Const => Ok(ForTarget::Lexical(target)),
            DeclarationKind::Using | DeclarationKind::AwaitUsing => Err(self.unsupported(left)),
        }
    }

    /// `for`-`in` (Abschnitt 14.7.5.5 mit ForIn/OfHeadEvaluation und
    /// ForIn/OfBodyEvaluation): Die rechte Seite wird wie bei `for`-`of`
    /// ausgewertet, [`Op::ForInPrepare`] legt den Aufzähler in ein Register,
    /// und jeder Durchlauf holt mit [`Op::ForInNext`] den nächsten Schlüssel
    /// in den Akkumulator und bindet ihn wie `for`-`of` seinen Wert. Nach
    /// Anhang B.3.5 darf `var` im Kopf einen Initialisierer haben, der vor
    /// der rechten Seite in die Variable kommt.
    fn for_in_loop(
        &mut self,
        node: NodeId,
        target: Target,
        left: NodeId,
        right: NodeId,
        body: NodeId,
    ) -> Result<()> {
        let (lhs, initializer) = self.for_in_target(left)?;
        let mark = self.next_register;
        if let (ForTarget::Var(declared), Some(init)) = (lhs, initializer) {
            self.for_in_initializer(left, declared, init)?;
        }
        if let ForTarget::Lexical(_) = lhs {
            let tdz = self.enter_tdz(left)?;
            self.expression(right)?;
            self.leave_block(tdz)?;
        } else {
            self.expression(right)?;
        }
        self.position(right)?;
        let enumerator = self.allocate()?;
        self.emit(Op::ForInPrepare, &[i64::from(enumerator)])?;
        let item = i64::from(self.allocate()?);
        self.reset_completion()?;
        let start = self.emitter.offset();
        self.position(node)?;
        let done = self
            .emitter
            .jump_with(Op::ForInNext, &[i64::from(enumerator)])?;
        self.push_target(target);
        let block = self.for_of_binding(left, lhs, item)?;
        self.statement(body)?;
        if let Some(block) = block {
            self.leave_block(block)?;
        }
        let mut target = self.pop_target();
        self.bind_all(std::mem::take(&mut target.continues))?;
        self.emitter.jump_loop(start)?;
        self.bind(done)?;
        self.bind_all(target.breaks)?;
        self.release(mark);
        Ok(())
    }

    /// Was links in `for`-`in` steht, dazu der Initialisierer aus Anhang
    /// B.3.5, den der Parser nur bei `var` mit einem Bezeichner zulässt.
    fn for_in_target(&self, left: NodeId) -> Result<(ForTarget, Option<NodeId>)> {
        let Node::Variables { kind, declarators } = *self.ast.node(left) else {
            return Ok((ForTarget::Expression, None));
        };
        let &[declarator] = self.ast.list(declarators) else {
            return Err(self.unsupported(left));
        };
        let Node::Declarator { target, init } = *self.ast.node(declarator) else {
            return Err(self.unsupported(declarator));
        };
        match kind {
            DeclarationKind::Var => Ok((ForTarget::Var(target), init)),
            DeclarationKind::Let | DeclarationKind::Const if init.is_none() => {
                Ok((ForTarget::Lexical(target), None))
            }
            _ => Err(self.unsupported(left)),
        }
    }

    /// Der Initialisierer von `for (var x = init in …)` (Anhang B.3.5):
    /// Erst wird der Name aufgelöst, dann der Wert mit NamedEvaluation
    /// berechnet und mit PutValue geschrieben, wie bei `var x = init`.
    fn for_in_initializer(&mut self, left: NodeId, target: NodeId, init: NodeId) -> Result<()> {
        let ast = self.ast;
        let Node::BindingIdentifier(atom) = *ast.node(target) else {
            return Err(self.unsupported(left));
        };
        let units = ast.atom(atom);
        if let Some(binding) = self.resolve(atom, target)? {
            self.named_expression(init, units)?;
            self.position(left)?;
            return self.store_local(binding, atom, ast.range(left).end);
        }
        let name = self.name(atom)?;
        let register = self.allocate()?;
        self.emit(Op::ResolveGlobal, &[name, i64::from(register)])?;
        self.named_expression(init, units)?;
        self.position(left)?;
        self.emit(Op::StaGlobal, &[name, i64::from(register)])?;
        self.release(register);
        Ok(())
    }

    /// Der Bereich, in dem die rechte Seite von `for`-`of` die Namen aus
    /// `let` und `const` im Kopf sieht (ForIn/OfHeadEvaluation, Abschnitt
    /// 14.7.5.6): Sie halten die Lücke und werden nie initialisiert. Gibt die
    /// Marke für [`Compiler::leave_block`] zurück.
    fn enter_tdz(&mut self, declaration: NodeId) -> Result<u32> {
        let mark = self.next_register;
        let mut scope = BlockScope::default();
        let mut holes = Vec::new();
        for (atom, constant, _) in self.lexical_declarations(declaration)? {
            let location = self.locate(&mut scope, atom)?;
            holes.push(location);
            let kind = if constant {
                BindingKind::Const
            } else {
                BindingKind::Mutable
            };
            let binding = LocalBinding {
                location,
                kind,
                initialized_from: usize::MAX,
                in_switch: false,
            };
            scope.bindings.insert(atom, binding);
        }
        if scope.has_context() {
            self.emit(Op::PushContext, &[i64::from(scope.slots)])?;
        }
        self.set_holes(&holes)?;
        self.scopes.push(scope);
        Ok(mark)
    }

    /// Bindet den Wert im Akkumulator an das Ziel links in `for`-`of`
    /// (ForIn/OfBodyEvaluation, Abschnitt 14.7.5.7). Mit `let` und `const`
    /// beginnt dafür ein neuer Bereich, dessen Marke zurückkommt; `var`
    /// und ein Ausdruck lösen ihren Namen oder ihre Referenz in jedem
    /// Durchlauf neu auf. Das Register `item` hält den Wert dazwischen.
    fn for_of_binding(&mut self, left: NodeId, lhs: ForTarget, item: i64) -> Result<Option<u32>> {
        let position = self.ast.range(left).start;
        match lhs {
            ForTarget::Lexical(target) => {
                self.emit(Op::Star, &[item])?;
                let block = self.enter_block(&[left], false)?;
                self.emit(Op::Ldar, &[item])?;
                let Node::BindingIdentifier(atom) = *self.ast.node(target) else {
                    let scope = self.scopes.len().checked_sub(1);
                    self.destructure(target, Init::Initialize(scope))?;
                    return Ok(Some(block));
                };
                // InitializeReferencedBinding.
                let binding = self.lookup(atom).ok_or_else(|| self.unsupported(left))?;
                self.store_location(binding.location)?;
                Ok(Some(block))
            }
            ForTarget::Var(target) => {
                let Node::BindingIdentifier(atom) = *self.ast.node(target) else {
                    self.destructure(target, Init::Put)?;
                    return Ok(None);
                };
                if let Some(binding) = self.resolve(atom, target)? {
                    self.store_local(binding, atom, position)?;
                    return Ok(None);
                }
                let name = self.name(atom)?;
                let register = self.allocate()?;
                self.emit(Op::ResolveGlobal, &[name, i64::from(register)])?;
                self.emit(Op::StaGlobal, &[name, i64::from(register)])?;
                self.release(register);
                Ok(None)
            }
            ForTarget::Expression => {
                if matches!(
                    self.ast.node(left),
                    Node::ArrayPattern(_) | Node::ObjectPattern(_)
                ) {
                    self.destructure(left, Init::Put)?;
                    return Ok(None);
                }
                self.emit(Op::Star, &[item])?;
                let reference = self.reference(left)?;
                self.emit(Op::Ldar, &[item])?;
                self.position(left)?;
                self.put_value(reference, position)?;
                self.release_reference(reference);
                Ok(None)
            }
        }
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
        self.push_target(target);
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
        self.leave_block(mark)?;
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
                label.is_some() || !matches!(target.kind, TargetKind::Labeled | TargetKind::Finally)
            };
            kind_fits && label.is_none_or(|label| target.labels.contains(&label))
        });
        let Some(target) = index else {
            return Err(self.unsupported(node));
        };
        self.exit(Exit::Jump {
            target,
            is_continue,
        })
    }

    /// Geht den Weg `exit` hinaus. Kontexte, die bis zum Ziel offen sind,
    /// gibt der Sprung ab. Liegt ein `try` mit `finally` dazwischen, geht es
    /// erst zum innersten: bis zu dessen Kontexten, mit der Nummer des Wegs
    /// im Register für die Fortsetzung, bei `return` mit dem Wert im
    /// Register für den Wert. Am Ende von `finally` geht der Weg weiter.
    fn exit(&mut self, exit: Exit) -> Result<()> {
        let floor = match exit {
            Exit::Jump { target, .. } => target.saturating_add(1),
            Exit::Return => 0,
        };
        let finally = self
            .targets
            .iter()
            .rposition(|target| target.kind == TargetKind::Finally)
            .filter(|&index| index >= floor);
        let index = match (finally, exit) {
            (Some(index), _) | (None, Exit::Jump { target: index, .. }) => index,
            (None, Exit::Return) => return self.emit_return(),
        };
        // Kann nicht fehlen: Die Stellen stammen aus `targets`.
        let Some(target) = self.targets.get(index) else {
            return Err(CompileError::Unsupported(0));
        };
        let contexts = target.contexts;
        let registers = target.finally.filter(|_| finally.is_some());
        if let (Some((_, value)), Exit::Return) = (registers, exit) {
            self.emit(Op::Star, &[i64::from(value)])?;
        }
        for _ in contexts..self.open_contexts() {
            self.emit(Op::PopContext, &[])?;
        }
        if let Some((kind, _)) = registers {
            let exits = self
                .targets
                .get(index)
                .map_or(&[][..], |target| &target.exits);
            let position = exits.iter().position(|&known| known == exit);
            let number = position.unwrap_or(exits.len());
            let token = FIRST_EXIT
                .checked_add(number)
                .and_then(|token| i64::try_from(token).ok())
                .ok_or(CompileError::TooLarge)?;
            self.emit(Op::LdaSmi, &[token])?;
            self.emit(Op::Star, &[i64::from(kind)])?;
            let jump = self.jump(Op::Jump)?;
            if let Some(target) = self.targets.get_mut(index) {
                if position.is_none() {
                    target.exits.push(exit);
                }
                target.breaks.push(jump);
            }
            return Ok(());
        }
        let jump = self.jump(Op::Jump)?;
        if let Some(target) = self.targets.get_mut(index) {
            match exit {
                Exit::Jump {
                    is_continue: true, ..
                } => target.continues.push(jump),
                _ => target.breaks.push(jump),
            }
        }
        Ok(())
    }

    /// `try` (Abschnitt 14.15.3). Zu Beginn kommt der aktuelle Kontext in
    /// ein Register, und jeder Handler stellt ihn daraus wieder her.
    ///
    /// Mit `finally` führen drei Arten von Wegen in dessen Block: das
    /// normale Ende von Block oder `catch`, ein Handler für jede Ausnahme
    /// aus beiden und die Wege hinaus ([`Compiler::exit`]). Jeder legt in
    /// ein Register, wie es danach weitergeht. Eine Ausnahme bleibt dabei,
    /// wie sie geworfen wurde, und kommt danach mit [`Op::Rethrow`] wieder.
    fn try_statement(
        &mut self,
        block: NodeId,
        handler: Option<NodeId>,
        finalizer: Option<NodeId>,
    ) -> Result<()> {
        self.reset_completion()?;
        let mark = self.next_register;
        let context = i64::from(self.allocate()?);
        self.emit(Op::SaveContext, &[context])?;
        if finalizer.is_some() {
            let kind = self.allocate()?;
            let value = self.allocate()?;
            let mut target = Target::new(TargetKind::Finally, Vec::new());
            target.finally = Some((kind, value));
            self.push_target(target);
        }
        let start = self.emitter.offset();
        // HasCallInTailPosition (Abschnitt 15.10.2.1): Im Block nie, in
        // `catch` nur ohne `finally`.
        self.protected = self
            .protected
            .checked_add(1)
            .ok_or(CompileError::TooLarge)?;
        self.statement(block)?;
        if finalizer.is_none() {
            self.protected = self.protected.saturating_sub(1);
        }
        let mut end = self.emitter.offset();
        if let Some(handler) = handler {
            let skip = self.jump(Op::Jump)?;
            self.emitter.handler(start, end, HandlerKind::Catch);
            self.emit(Op::RestoreContext, &[context])?;
            self.catch_clause(handler)?;
            end = self.emitter.offset();
            self.bind(skip)?;
        }
        if finalizer.is_some() {
            self.protected = self.protected.saturating_sub(1);
        }
        let Some(finalizer) = finalizer else {
            self.release(mark);
            return Ok(());
        };
        let target = self.pop_target();
        let Some((kind, value)) = target.finally else {
            return Err(self.unsupported(finalizer));
        };
        let (kind, value) = (i64::from(kind), i64::from(value));
        self.emit(Op::LdaSmi, &[NORMAL])?;
        self.emit(Op::Star, &[kind])?;
        let normal = self.jump(Op::Jump)?;
        self.emitter.handler(start, end, HandlerKind::Finally);
        self.emit(Op::RestoreContext, &[context])?;
        self.emit(Op::Star, &[value])?;
        self.emit(Op::LdaSmi, &[THROW])?;
        self.emit(Op::Star, &[kind])?;
        self.bind(normal)?;
        self.bind_all(target.breaks)?;
        // Endet `finally` normal, zählt sein Abschlusswert nicht, endet es
        // abrupt, gilt seiner, ohne eigenen *undefined*.
        let saved = match self.completion {
            Some(completion) => {
                let saved = i64::from(self.allocate()?);
                self.emit(Op::Ldar, &[completion])?;
                self.emit(Op::Star, &[saved])?;
                self.reset_completion()?;
                Some((completion, saved))
            }
            None => None,
        };
        self.statement(finalizer)?;
        if let Some((completion, saved)) = saved {
            self.emit(Op::Ldar, &[saved])?;
            self.emit(Op::Star, &[completion])?;
        }
        self.emit(Op::LdaSmi, &[THROW])?;
        self.emit(Op::TestEqualStrict, &[kind])?;
        let next = self.jump(Op::JumpIfFalse)?;
        self.emit(Op::Ldar, &[value])?;
        self.emit(Op::Rethrow, &[])?;
        self.bind(next)?;
        self.dispatch_exits(kind, value, target.exits)?;
        self.release(mark);
        Ok(())
    }

    /// Geht den Weg hinaus weiter, dessen Nummer im Register `kind` liegt,
    /// bei `return` mit dem Wert aus `value`: am Ende von `finally` und nach
    /// dem Schließen eines Iterators. Ist es keiner der Wege, geht es
    /// dahinter weiter.
    fn dispatch_exits(&mut self, kind: i64, value: i64, exits: Vec<Exit>) -> Result<()> {
        for (number, exit) in exits.into_iter().enumerate() {
            let token = FIRST_EXIT
                .checked_add(number)
                .and_then(|token| i64::try_from(token).ok())
                .ok_or(CompileError::TooLarge)?;
            self.emit(Op::LdaSmi, &[token])?;
            self.emit(Op::TestEqualStrict, &[kind])?;
            let next = self.jump(Op::JumpIfFalse)?;
            if exit == Exit::Return {
                self.emit(Op::Ldar, &[value])?;
            }
            self.exit(exit)?;
            self.bind(next)?;
        }
        Ok(())
    }

    /// CatchClauseEvaluation (Abschnitt 14.15.2) mit der Ausnahme im
    /// Akkumulator. Der Parameter hat einen eigenen Bereich um den Block;
    /// ein `var` mit seinem Namen im Block schreibt mit seinem
    /// Initialisierer in ihn (Anhang B.3.4).
    fn catch_clause(&mut self, handler: NodeId) -> Result<()> {
        let Node::Catch { parameter, body } = *self.ast.node(handler) else {
            return Err(self.unsupported(handler));
        };
        self.position(handler)?;
        let Some(parameter) = parameter else {
            self.reset_completion()?;
            return self.statement(body);
        };
        let Node::BindingIdentifier(atom) = *self.ast.node(parameter) else {
            return self.catch_pattern(parameter, body);
        };
        let mark = self.next_register;
        let mut scope = BlockScope::default();
        let location = self.locate(&mut scope, atom)?;
        let binding = LocalBinding::initialized(location, BindingKind::Mutable);
        scope.bindings.insert(atom, binding);
        if scope.has_context() {
            self.emit(Op::PushContext, &[i64::from(scope.slots)])?;
        }
        self.scopes.push(scope);
        self.store_location(location)?;
        self.reset_completion()?;
        self.statement(body)?;
        self.leave_block(mark)
    }

    /// `catch` mit einem Muster als Parameter: Seine Namen bleiben in der
    /// TDZ, bis das Muster sie bindet (CatchClauseEvaluation, Schritte 2
    /// bis 4). Die Ausnahme wartet dafür in einem Register.
    fn catch_pattern(&mut self, parameter: NodeId, body: NodeId) -> Result<()> {
        let mark = self.next_register;
        let exception = i64::from(self.allocate()?);
        self.emit(Op::Star, &[exception])?;
        let mut names = Vec::new();
        self.bound_names(parameter, &mut names);
        let end = self.ast.range(parameter).end;
        let mut scope = BlockScope::default();
        let mut holes = Vec::new();
        for atom in names {
            let location = self.locate(&mut scope, atom)?;
            holes.push(location);
            let binding = LocalBinding {
                location,
                kind: BindingKind::Mutable,
                initialized_from: end,
                in_switch: false,
            };
            scope.bindings.insert(atom, binding);
        }
        if scope.has_context() {
            self.emit(Op::PushContext, &[i64::from(scope.slots)])?;
        }
        self.set_holes(&holes)?;
        self.scopes.push(scope);
        self.emit(Op::Ldar, &[exception])?;
        let index = self.scopes.len().checked_sub(1);
        self.destructure(parameter, Init::Initialize(index))?;
        self.reset_completion()?;
        self.statement(body)?;
        self.leave_block(mark)
    }

    // Muster

    /// BindingInitialization (Abschnitt 8.6.2) und
    /// DestructuringAssignmentEvaluation (Abschnitt 13.15.5.2) des Musters
    /// `pattern` mit dem Wert im Akkumulator. Danach ist der Akkumulator
    /// beliebig.
    fn destructure(&mut self, pattern: NodeId, init: Init) -> Result<()> {
        self.position(pattern)?;
        match *self.ast.node(pattern) {
            Node::ArrayPattern(list) => self.array_pattern(pattern, list, init),
            Node::ObjectPattern(list) => self.object_pattern(list, init),
            _ => Err(self.unsupported(pattern)),
        }
    }

    /// Ein Array-Muster: GetIterator, dann die Elemente der Reihe nach.
    /// Endet das ohne Ausnahme, schließt [`Op::IteratorClose`] den
    /// Iterator mit einer normalen Beendigung, sonst ein Handler mit der
    /// Ausnahme; beides nur, solange [[Done]] nicht gilt.
    fn array_pattern(&mut self, pattern: NodeId, list: List, init: Init) -> Result<()> {
        let iterator = self.allocate()?;
        self.allocate()?;
        self.allocate()?;
        let record = i64::from(iterator);
        self.emit(Op::GetIterator, &[record])?;
        let close = self.pattern_close_target()?;
        let start = self.emitter.offset();
        let ast = self.ast;
        for &element in ast.list(list) {
            match *ast.node(element) {
                Node::Elision => {
                    self.position(element)?;
                    self.emit(Op::IteratorSkip, &[record])?;
                }
                Node::Rest(target) => {
                    let leaf = self.leaf(target, init)?;
                    self.position(element)?;
                    let array = self.allocate()?;
                    self.emit(Op::CreateArray, &[])?;
                    self.emit(Op::Star, &[i64::from(array)])?;
                    let next = self.emitter.offset();
                    let done = self.emitter.jump_with(Op::IteratorNext, &[record])?;
                    self.emit(Op::ArrayPush, &[i64::from(array)])?;
                    self.emitter.jump_loop(next)?;
                    self.bind(done)?;
                    self.emit(Op::Ldar, &[i64::from(array)])?;
                    self.release(array);
                    self.bind_leaf(target, leaf, init)?;
                }
                _ => self.pattern_element(element, init, Fetch::Iterator(record))?,
            }
        }
        let end = self.emitter.offset();
        self.position(pattern)?;
        self.emit(Op::IteratorClose, &[record])?;
        self.close_on_abrupt_exits(record, start, end, close)?;
        self.release(iterator);
        Ok(())
    }

    /// In Generatoren kann aus einem `yield` in einem Element eines
    /// Array-Musters `return` kommen. Dann schließt der Iterator wie nach
    /// `break` (BindingInitialization und DestructuringAssignmentEvaluation,
    /// Abschnitte 8.6.2 und 13.15.5.2: IteratorClose für jede Beendigung,
    /// solange [[Done]] nicht gesetzt ist), über ein Ziel wie bei `finally`.
    /// Gibt dessen Register zurück. Eigene Funktion wie
    /// [`Compiler::close_on_abrupt_exits`], damit der Rahmen der Rekursion
    /// über verschachtelte Muster klein bleibt.
    fn pattern_close_target(&mut self) -> Result<Option<(i64, i64)>> {
        if self.generator.is_none() {
            return Ok(None);
        }
        let kind = self.allocate()?;
        let value = self.allocate()?;
        let mut target = Target::new(TargetKind::Finally, Vec::new());
        target.finally = Some((kind, value));
        self.push_target(target);
        Ok(Some((i64::from(kind), i64::from(value))))
    }

    /// Nach den Elementen eines Array-Musters zwischen `start` und `end`:
    /// ein Wurf schließt den Iterator über einen Handler, `return` aus
    /// einem `yield` über das Ziel aus [`Compiler::pattern_close_target`].
    fn close_on_abrupt_exits(
        &mut self,
        record: i64,
        start: usize,
        end: usize,
        close: Option<(i64, i64)>,
    ) -> Result<()> {
        let closing = close.map(|registers| (registers, self.pop_target()));
        if end <= start {
            return Ok(());
        }
        let after = self.jump(Op::Jump)?;
        self.emitter.handler(start, end, HandlerKind::Finally);
        let exception = i64::from(self.allocate()?);
        self.emit(Op::Star, &[exception])?;
        self.emit(Op::IteratorCloseOnThrow, &[record])?;
        self.emit(Op::Ldar, &[exception])?;
        self.emit(Op::Rethrow, &[])?;
        if let Some(((kind, value), target)) = closing
            && !target.breaks.is_empty()
        {
            self.bind_all(target.breaks)?;
            self.emit(Op::IteratorClose, &[record])?;
            self.dispatch_exits(kind, value, target.exits)?;
        }
        self.bind(after)
    }

    /// Ein Objekt-Muster: RequireObjectCoercible, dann je Eigenschaft erst
    /// der Name, dann das Element. Mit Rest liegen die Schlüssel davor in
    /// Registern direkt hinter dem Wert, für [`Op::CreateRestObject`].
    fn object_pattern(&mut self, list: List, init: Init) -> Result<()> {
        let ast = self.ast;
        let properties = ast.list(list);
        let rest = properties
            .last()
            .is_some_and(|&property| matches!(ast.node(property), Node::Rest(_)));
        self.emit(Op::RequireObjectCoercible, &[])?;
        let source = self.allocate()?;
        let object = i64::from(source);
        self.emit(Op::Star, &[object])?;
        let mut excluded = Vec::new();
        if rest {
            for _ in 1..properties.len() {
                excluded.push(self.allocate()?);
            }
        }
        for (index, &property) in properties.iter().enumerate() {
            self.position(property)?;
            match *ast.node(property) {
                Node::PatternProperty { key, value, .. } => {
                    let mark = self.next_register;
                    let fetch = match self.property_key(key)? {
                        Some(units) => {
                            let name = self.constant(Constant::Name(units))?;
                            if let Some(&register) = excluded.get(index) {
                                self.emit(Op::LdaConstant, &[name])?;
                                self.emit(Op::Star, &[i64::from(register)])?;
                            }
                            Fetch::Named { object, name }
                        }
                        // Der Schlüssel aus ToPropertyKey im Akkumulator.
                        None => {
                            let register = match excluded.get(index) {
                                Some(&register) => register,
                                None => self.allocate()?,
                            };
                            let key = i64::from(register);
                            self.emit(Op::Star, &[key])?;
                            Fetch::Keyed { object, key }
                        }
                    };
                    self.pattern_element(value, init, fetch)?;
                    self.release(mark);
                }
                Node::Rest(target) => {
                    let leaf = self.leaf(target, init)?;
                    self.position(property)?;
                    let count =
                        i64::try_from(excluded.len()).map_err(|_| CompileError::TooLarge)?;
                    self.emit(Op::CreateRestObject, &[object, count])?;
                    self.bind_leaf(target, leaf, init)?;
                }
                _ => return Err(self.unsupported(property)),
            }
        }
        self.release(source);
        Ok(())
    }

    /// Ein Element eines Musters, vielleicht mit Vorgabewert: erst das
    /// Ziel, wenn es kein Muster ist (Abschnitte 13.15.5.5 und 13.15.5.6),
    /// dann der Wert aus `fetch`, für *undefined* der Vorgabewert, dann das
    /// Binden. NamedEvaluation gibt es nur für einen Bezeichner ohne
    /// Klammern (IsIdentifierRef).
    fn pattern_element(&mut self, element: NodeId, init: Init, fetch: Fetch) -> Result<()> {
        let (target, default) = match *self.ast.node(element) {
            Node::WithDefault { target, default } => (target, Some(default)),
            _ => (element, None),
        };
        let leaf = self.leaf(target, init)?;
        self.position(element)?;
        match fetch {
            Fetch::Iterator(record) => self.emit(Op::IteratorElement, &[record])?,
            Fetch::Named { object, name } => {
                self.emit(Op::Ldar, &[object])?;
                self.emit(Op::LdaNamed, &[name])?;
            }
            Fetch::Keyed { object, key } => {
                self.emit(Op::Ldar, &[key])?;
                self.emit(Op::LdaKeyed, &[object])?;
            }
        }
        if let Some(default) = default {
            let skip = self.jump(Op::JumpIfNotUndefined)?;
            let ast = self.ast;
            match *ast.node(target) {
                Node::BindingIdentifier(atom) | Node::Identifier(atom) => {
                    self.named_expression(default, ast.atom(atom))?;
                }
                _ => self.expression(default)?,
            }
            self.bind(skip)?;
        }
        self.bind_leaf(target, leaf, init)
    }

    /// Wertet das Ziel `target` eines Elements vor seinem Wert aus: eine
    /// Referenz, ein Name zum Initialisieren oder `None` für ein inneres
    /// Muster, das erst mit dem Wert bindet.
    fn leaf(&mut self, target: NodeId, init: Init) -> Result<Option<Leaf>> {
        match (*self.ast.node(target), init) {
            (Node::ArrayPattern(_) | Node::ObjectPattern(_), _) => Ok(None),
            (Node::BindingIdentifier(atom), Init::Initialize(_)) => {
                Ok(Some(Leaf::Initialize(atom)))
            }
            _ => {
                let position = self.ast.range(target).start;
                let reference = self.reference(target)?;
                Ok(Some(Leaf::Put(reference, position)))
            }
        }
    }

    /// Bindet den Akkumulator an das Ziel eines Elements.
    fn bind_leaf(&mut self, target: NodeId, leaf: Option<Leaf>, init: Init) -> Result<()> {
        match leaf {
            None => self.destructure(target, init),
            Some(Leaf::Put(reference, position)) => {
                self.position(target)?;
                self.put_value(reference, position)?;
                self.release_reference(reference);
                Ok(())
            }
            Some(Leaf::Initialize(atom)) => {
                let index = match init {
                    Init::Initialize(index) => index,
                    Init::Put => None,
                };
                match index.and_then(|index| self.lookup_in(index, atom)) {
                    Some(binding) => self.store_location(binding.location),
                    None => {
                        let name = self.name(atom)?;
                        self.emit(Op::InitGlobal, &[name])
                    }
                }
            }
        }
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
            Node::This => self.load_this(node),
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
            Node::Member { .. }
            | Node::ComputedMember { .. }
            | Node::PrivateMember { .. }
            | Node::Call { .. } => self.chain(node, ChainEnd::Value).map(|_| ()),
            Node::Chain(chain) => self.chain(chain, ChainEnd::Value).map(|_| ()),
            Node::New { callee, arguments } => self.construct(node, callee, arguments),
            Node::NewTarget => self.load_new_target(node),
            Node::Function(id) => self.closure(node, id, ClosureName::Static(&[])),
            Node::Class(id) => self.class_definition(node, id, ClosureName::Static(&[])),
            Node::SuperMember(_) | Node::SuperComputedMember(_) => {
                let reference = self.super_reference(node)?;
                self.get_value(reference, self.ast.range(node).start)?;
                self.release_reference(reference);
                Ok(())
            }
            Node::SuperCall(arguments) => self.super_call(node, arguments),
            Node::Yield { argument, delegate } => self.yield_expression(node, argument, delegate),
            Node::Await(argument) => self.await_expression(node, argument),
            _ => Err(self.unsupported(node)),
        }
    }

    /// `yield` und `yield*` (Abschnitt 15.5.5). Der Parser lässt sie nur im
    /// Rumpf eines Generators zu, nicht in seinen Parametern und nicht in
    /// inneren Funktionen.
    fn yield_expression(
        &mut self,
        node: NodeId,
        argument: Option<NodeId>,
        delegate: bool,
    ) -> Result<()> {
        if self.generator.is_none() {
            return Err(self.unsupported(node));
        }
        if self.async_generator {
            return self.async_yield_expression(node, argument, delegate);
        }
        if !delegate {
            match argument {
                Some(argument) => self.expression(argument)?,
                None => self.emit(Op::LdaUndefined, &[])?,
            }
            self.position(node)?;
            self.emit(Op::Yield, &[])?;
            return self.resumption();
        }
        let Some(argument) = argument else {
            return Err(self.unsupported(node));
        };
        self.expression(argument)?;
        self.position(node)?;
        let record = self.allocate()?;
        self.allocate()?;
        self.allocate()?;
        self.emit(Op::GetIterator, &[i64::from(record)])?;
        // Der erste Durchlauf ruft `next` mit *undefined*.
        self.emit(Op::LdaUndefined, &[])?;
        let start = self.emitter.offset();
        let done = self
            .emitter
            .jump_with(Op::YieldStar, &[i64::from(record)])?;
        self.emit(Op::YieldResult, &[])?;
        self.emitter.jump_loop(start)?;
        self.bind(done)?;
        self.release(record);
        self.resumption()
    }

    /// `yield` und `yield*` in einem async-Generator. `yield` wartet auf
    /// seinen Wert (Yield, Abschnitt 27.8.3.7), dann gibt ihn
    /// [`Op::AsyncYield`] weiter. Für `yield*` wartet jeder Durchlauf auf
    /// das Ergebnis des inneren Iterators, und sein Wert geht ohne weiteres
    /// Warten weiter.
    fn async_yield_expression(
        &mut self,
        node: NodeId,
        argument: Option<NodeId>,
        delegate: bool,
    ) -> Result<()> {
        if !delegate {
            match argument {
                Some(argument) => self.expression(argument)?,
                None => self.emit(Op::LdaUndefined, &[])?,
            }
            self.position(node)?;
            self.emit(Op::Await, &[])?;
            self.emit(Op::ResumeAsync, &[])?;
            self.emit(Op::AsyncYield, &[])?;
            return self.resumption();
        }
        let Some(argument) = argument else {
            return Err(self.unsupported(node));
        };
        self.expression(argument)?;
        self.position(node)?;
        // Der Iterator, `next` und die Art des Durchlaufs.
        let record = self.allocate()?;
        self.allocate()?;
        self.allocate()?;
        self.emit(Op::GetAsyncIterator, &[i64::from(record)])?;
        // Der erste Durchlauf ruft `next` mit *undefined*.
        self.emit(Op::LdaUndefined, &[])?;
        let start = self.emitter.offset();
        self.emit(Op::AsyncYieldStar, &[i64::from(record)])?;
        self.emit(Op::Await, &[])?;
        self.emit(Op::ResumeAsync, &[])?;
        let done = self
            .emitter
            .jump_with(Op::AsyncYieldStarResult, &[i64::from(record)])?;
        self.emit(Op::AsyncYield, &[])?;
        self.emitter.jump_loop(start)?;
        self.bind(done)?;
        self.release(record);
        self.resumption()
    }

    /// Ob der Code `await` enthalten kann: der einer async-Funktion oder
    /// eines async-Generators.
    fn is_async(&self) -> bool {
        self.async_state.is_some() || self.async_generator
    }

    /// `await` (Abschnitt 15.8.5). Der Parser lässt es nur im Rumpf einer
    /// async-Funktion oder eines async-Generators zu, nicht in den
    /// Parametern und nicht in inneren Funktionen; auf oberster Ebene eines
    /// Moduls kommt es mit den Modulen.
    fn await_expression(&mut self, node: NodeId, argument: NodeId) -> Result<()> {
        if !self.is_async() {
            return Err(self.unsupported(node));
        }
        self.expression(argument)?;
        self.position(node)?;
        self.emit(Op::Await, &[])?;
        self.emit(Op::ResumeAsync, &[])
    }

    /// Der Weg nach einem Halt: Mit `next` geht es hinter dem Weg von
    /// `return` weiter, mit `return` gilt der Akkumulator als Wert von
    /// `return` und geht durch `finally` und das Schließen der Iteratoren von
    /// `for`-`of`. In einem async-Generator wurde auf diesen Wert schon
    /// gewartet.
    fn resumption(&mut self) -> Result<()> {
        let next = self.jump(Op::ResumeGenerator)?;
        self.exit(Exit::Return)?;
        self.bind(next)
    }

    /// IsAnonymousFunctionDefinition (Abschnitt 8.4.3) für die Funktionen
    /// und Klassen, die es schon gibt; Klammern zählen nicht.
    fn anonymous_definition(&self, node: NodeId) -> Option<Anonymous> {
        let inner = self.without_parens(node);
        match *self.ast.node(inner) {
            Node::Function(id)
                if self
                    .ast
                    .function(id)
                    .is_some_and(|function| function.name.is_none()) =>
            {
                Some(Anonymous::Function(inner, id))
            }
            Node::Class(id) if self.ast.class(id).is_some_and(|class| class.name.is_none()) => {
                Some(Anonymous::Class(inner, id))
            }
            _ => None,
        }
    }

    /// NamedEvaluation (Abschnitt 8.4.5): Eine anonyme Funktion oder Klasse,
    /// auch in Klammern, bekommt den Namen `name`; alles andere wird
    /// gewöhnlich ausgewertet.
    fn named_expression(&mut self, node: NodeId, name: &[u16]) -> Result<()> {
        self.named_definition(node, ClosureName::Static(name))
    }

    /// Dasselbe mit dem Schlüssel im Register als Namen.
    fn keyed_expression(&mut self, node: NodeId, key: u32) -> Result<()> {
        self.named_definition(node, ClosureName::Keyed(key))
    }

    /// Der Rumpf des Initialisierers eines Feldes (EvaluateBody, Abschnitt
    /// 10.2.1.3): Eine anonyme Funktion oder Klasse bekommt den Namen des
    /// Feldes ([[ClassFieldInitializerName]]). Den hat das Funktionsobjekt
    /// des Initialisierers als `name`, und Code der Sprache sieht es nie.
    /// Endaufrufe gibt es hier nicht (IsInTailPosition, Abschnitt 15.10.1).
    fn field_initializer(&mut self, expression: NodeId) -> Result<()> {
        if self.anonymous_definition(expression).is_none() {
            return self.expression(expression);
        }
        let key = self.allocate()?;
        let name = self.constant(Constant::Name(units("name")))?;
        self.emit(Op::Ldar, &[1])?;
        self.emit(Op::LdaNamed, &[name])?;
        self.emit(Op::Star, &[i64::from(key)])?;
        self.keyed_expression(expression, key)?;
        self.release(key);
        Ok(())
    }

    fn named_definition(&mut self, node: NodeId, name: ClosureName<'_>) -> Result<()> {
        match self.anonymous_definition(node) {
            Some(Anonymous::Function(function, id)) => self.closure(function, id, name),
            Some(Anonymous::Class(class, id)) => self.class_definition(class, id, name),
            None => self.expression(node),
        }
    }

    /// Ein neues Funktionsobjekt im Akkumulator, für einen
    /// Funktionsausdruck (InstantiateOrdinaryFunctionExpression, Abschnitt
    /// 15.2.5) oder eine Methode (DefineMethod, Abschnitt 15.4.4). Ein
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
                | FunctionKind::Arrow
                | FunctionKind::Method
                | FunctionKind::Getter
                | FunctionKind::Setter
                | FunctionKind::FieldInitializer
                | FunctionKind::StaticBlock
        );
        if !supported {
            return Err(self.unsupported(node));
        }
        let template = FunctionTemplate {
            function: id,
            binds_name,
            outer: self.enclosing(),
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
    /// (MethodDefinitionEvaluation, Abschnitt 15.4.5): erst der Name, dann
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
                self.home_object(id, object)?;
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
        self.home_object(id, object)?;
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

    /// MakeMethod (Abschnitt 10.2.7) für das Funktionsobjekt der Funktion
    /// `id` im Akkumulator mit dem Objekt im Register `object`. Nur wer
    /// `super.x` liest, liest auch [[HomeObject]]; sonst fehlt der Befehl.
    fn home_object(&mut self, id: FunctionId, object: i64) -> Result<()> {
        let reads = self
            .ast
            .captures()
            .function(id)
            .is_some_and(|captures| captures.super_property);
        if reads {
            self.emit(Op::SetHomeObject, &[object])?;
        }
        Ok(())
    }

    /// BindingClassDeclarationEvaluation (Abschnitt 15.7.15): die Klasse,
    /// dann InitializeBoundName im umgebenden Bereich wie bei `let`.
    fn class_declaration(&mut self, node: NodeId, id: ClassId) -> Result<()> {
        let Some(name) = self.ast.class(id).and_then(|class| class.name) else {
            return Err(self.unsupported(node));
        };
        self.class_definition(node, id, ClosureName::Static(&[]))?;
        match self
            .scopes
            .last()
            .and_then(|scope| scope.bindings.get(&name).copied())
        {
            Some(binding) => self.store_location(binding.location),
            None => {
                let name = self.name(name)?;
                self.emit(Op::InitGlobal, &[name])
            }
        }
    }

    /// ClassDefinitionEvaluation (Abschnitt 15.7.14) mit dem Konstruktor im
    /// Akkumulator. Eine Klasse mit Namen heißt wie er und bindet ihn in
    /// ihrem Bereich, eine ohne heißt `name` (className). Der Bereich
    /// (classEnv) umfasst die Oberklasse und die berechneten Namen; der
    /// Name darin ist eine Konstante und bis zum Ende der Klasse in der TDZ.
    /// Das Register hinter dem für die Oberklasse hält erst den Namen, nach
    /// [`Op::CreateClass`] den Konstruktor.
    ///
    /// Die privaten Namen (classPrivateEnvironment) liegen in Plätzen des
    /// Bereichs. Sie entstehen nach der Oberklasse, die sie nicht sieht;
    /// beobachten lässt sich das nicht. Die privaten Methoden der Instanzen
    /// kommen an das Objekt aus [`Op::CreatePrivateMethods`], die statischen
    /// gleich an den Konstruktor, die Felder der Instanzen an [[Fields]].
    /// Statische Felder und `static`-Blöcke laufen nach dem Binden des
    /// Namens der Reihe nach (Schritte 29 bis 33); ihre Funktionsobjekte
    /// entstehen erst dann, die berechneten Namen vorher.
    fn class_definition(&mut self, node: NodeId, id: ClassId, name: ClosureName<'_>) -> Result<()> {
        if self.is_strict() {
            return self.class_parts(node, id, name);
        }
        // Alle Teile einer Klasse sind strikter Code (Abschnitt 11.2.2),
        // auch die Oberklasse und die berechneten Namen, die hier laufen;
        // ihre Befehle bilden einen strikten Bereich.
        let start = self.emitter.offset();
        self.strict_class = true;
        let result = self.class_parts(node, id, name);
        self.strict_class = false;
        result?;
        self.emitter.strict_range(start, self.emitter.offset())?;
        Ok(())
    }

    /// [`Compiler::class_definition`] in strikt übersetztem Code.
    fn class_parts(&mut self, node: NodeId, id: ClassId, name: ClosureName<'_>) -> Result<()> {
        let ast = self.ast;
        let Some(class) = ast.class(id) else {
            return Err(self.unsupported(node));
        };
        let elements = ast.list(class.elements);
        let mark = self.next_register;
        let mut scope = BlockScope::default();
        let mut holes = Vec::new();
        if let Some(atom) = class.name {
            let location = self.locate(&mut scope, atom)?;
            holes.push(location);
            let binding = LocalBinding {
                location,
                kind: BindingKind::Const,
                initialized_from: class.range.end,
                in_switch: false,
            };
            scope.bindings.insert(atom, binding);
        }
        // PrivateBoundIdentifiers; Getter und Setter teilen sich einen Namen.
        let mut privates = BTreeMap::new();
        let mut instance_private_methods = false;
        for &element in elements {
            let (key, method, is_static) = match *ast.node(element) {
                Node::ClassMethod { key, is_static, .. } => (key, true, is_static),
                Node::ClassField { key, is_static, .. } => (key, false, is_static),
                _ => continue,
            };
            let Node::PrivateName(atom) = *ast.node(key) else {
                continue;
            };
            instance_private_methods |= method && !is_static;
            if let std::collections::btree_map::Entry::Vacant(entry) = privates.entry(atom) {
                entry.insert(scope.add_slot()?);
            }
        }
        if scope.has_context() {
            self.emit(Op::PushContext, &[i64::from(scope.slots)])?;
        }
        self.set_holes(&holes)?;
        self.scopes.push(scope);
        let parent = self.allocate()?;
        let constructor = self.allocate()?;
        match class.heritage {
            Some(heritage) => self.expression(heritage)?,
            None => self.emit(Op::LdaHole, &[])?,
        }
        self.emit(Op::Star, &[i64::from(parent)])?;
        self.position(node)?;
        for (&atom, &slot) in &privates {
            let mut description = units("#");
            description.extend_from_slice(ast.atom(atom));
            let description = self.constant(Constant::String(description))?;
            self.emit(Op::CreatePrivateName, &[description])?;
            self.emit(Op::StaContext, &[0, i64::from(slot)])?;
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.privates = privates;
        }
        match (class.name, name) {
            (Some(atom), _) => {
                let constant = self.name(atom)?;
                self.emit(Op::LdaConstant, &[constant])?;
                self.emit(Op::Star, &[i64::from(constructor)])?;
            }
            (None, ClosureName::Static(units)) => {
                let constant = self.constant(Constant::Name(units.to_vec()))?;
                self.emit(Op::LdaConstant, &[constant])?;
                self.emit(Op::Star, &[i64::from(constructor)])?;
            }
            (None, ClosureName::Keyed(register)) => {
                self.emit(Op::Mov, &[i64::from(register), i64::from(constructor)])?;
            }
        }
        let function = match class.constructor {
            Some(function) => Constant::Function(FunctionTemplate {
                function,
                binds_name: false,
                outer: self.enclosing(),
            }),
            None => Constant::DefaultConstructor {
                derived: class.heritage.is_some(),
                class: id,
            },
        };
        let function = self.constant(function)?;
        self.position(node)?;
        self.emit(Op::CreateClass, &[function, i64::from(parent)])?;
        self.emit(Op::Star, &[i64::from(constructor)])?;
        let holder = if instance_private_methods {
            let holder = self.allocate()?;
            self.emit(Op::CreatePrivateMethods, &[i64::from(constructor)])?;
            self.emit(Op::Star, &[i64::from(holder)])?;
            Some(holder)
        } else {
            None
        };
        let mut statics = Vec::new();
        for &element in elements {
            let class = ClassRegisters {
                prototype: parent,
                constructor,
                holder,
            };
            if let Some(element) = self.class_element(element, class)? {
                statics.push(element);
            }
        }
        if let Some(atom) = class.name
            && let Some(binding) = self.lookup(atom)
        {
            self.emit(Op::Ldar, &[i64::from(constructor)])?;
            self.store_location(binding.location)?;
        }
        for element in statics {
            self.static_element(element, constructor)?;
        }
        self.emit(Op::Ldar, &[i64::from(constructor)])?;
        self.leave_block(mark)
    }

    /// ClassElementEvaluation (Abschnitt 15.7.13) eines Elements außer dem
    /// Konstruktor. Methoden, Getter und Setter wertet es aus wie
    /// MethodDefinitionEvaluation im Objektliteral, aber nicht aufzählbar,
    /// auf dem Prototyp oder mit `static` auf dem Konstruktor, der dann auch
    /// [[HomeObject]] ist; private kommen an das Objekt für
    /// [[PrivateMethods]] oder an den Konstruktor. Ein Feld einer Instanz
    /// kommt mit seinem Namen und dem Funktionsobjekt seines Initialisierers
    /// an [[Fields]] (ClassFieldDefinitionEvaluation, Abschnitt 15.7.10).
    /// Ein statisches Feld wertet nur seinen Namen aus und gibt es wie
    /// einen `static`-Block für später zurück.
    fn class_element(
        &mut self,
        element: NodeId,
        class: ClassRegisters,
    ) -> Result<Option<StaticElement>> {
        self.position(element)?;
        match *self.ast.node(element) {
            Node::ClassMethod {
                key,
                function,
                kind,
                is_static,
            } => {
                self.class_method(element, class, key, function, kind, is_static)?;
                Ok(None)
            }
            Node::ClassField {
                key,
                value,
                is_static: false,
            } => {
                let name = self.allocate()?;
                let register = i64::from(name);
                self.field_key(key)?;
                self.emit(Op::Star, &[register])?;
                match value {
                    Some(initializer) => {
                        self.closure(element, initializer, ClosureName::Keyed(name))?;
                        self.home_object(initializer, i64::from(class.prototype))?;
                    }
                    None => self.emit(Op::LdaUndefined, &[])?,
                }
                self.position(element)?;
                self.emit(Op::AddField, &[i64::from(class.constructor), register])?;
                self.release(name);
                Ok(None)
            }
            Node::ClassField {
                key,
                value,
                is_static: true,
            } => {
                let key = match *self.ast.node(key) {
                    Node::PrivateName(atom) => FieldKey::Private(atom),
                    _ => match self.property_key(key)? {
                        Some(units) => FieldKey::Name(units),
                        None => {
                            // Der Name bleibt im Register bis zum Ende der
                            // Klasse.
                            let register = self.allocate()?;
                            self.emit(Op::Star, &[i64::from(register)])?;
                            FieldKey::Register(register)
                        }
                    },
                };
                Ok(Some(StaticElement::Field {
                    node: element,
                    key,
                    value,
                }))
            }
            Node::StaticBlock(function) => Ok(Some(StaticElement::Block {
                node: element,
                function,
            })),
            _ => Err(self.unsupported(element)),
        }
    }

    /// Eine Methode, ein Getter oder ein Setter einer Klasse: erst der Name,
    /// dann das Funktionsobjekt, dann die Eigenschaft oder das private
    /// Element.
    fn class_method(
        &mut self,
        element: NodeId,
        class: ClassRegisters,
        key: NodeId,
        function: FunctionId,
        kind: MethodKind,
        is_static: bool,
    ) -> Result<()> {
        let home = i64::from(if is_static {
            class.constructor
        } else {
            class.prototype
        });
        let name = self.allocate()?;
        let register = i64::from(name);
        if let Node::PrivateName(atom) = *self.ast.node(key) {
            self.load_private_name(atom, key)?;
            self.emit(Op::Star, &[register])?;
            self.closure(element, function, ClosureName::Keyed(name))?;
            self.home_object(function, home)?;
            let target = match (is_static, class.holder) {
                (false, Some(holder)) => i64::from(holder),
                (false, None) => return Err(self.unsupported(element)),
                (true, _) => home,
            };
            self.position(key)?;
            let op = match kind {
                MethodKind::Method => Op::DefinePrivateMethod,
                MethodKind::Get => Op::DefinePrivateGetter,
                MethodKind::Set => Op::DefinePrivateSetter,
            };
            self.emit(op, &[target, register])?;
            self.release(name);
            return Ok(());
        }
        match self.property_key(key)? {
            Some(units) => {
                let constant = self.constant(Constant::Name(units.clone()))?;
                self.emit(Op::LdaConstant, &[constant])?;
                self.emit(Op::Star, &[register])?;
                self.closure(element, function, ClosureName::Static(&units))?;
            }
            None => {
                self.emit(Op::Star, &[register])?;
                self.closure(element, function, ClosureName::Keyed(name))?;
            }
        }
        self.home_object(function, home)?;
        self.position(key)?;
        let op = match kind {
            MethodKind::Method => Op::DefineMethod,
            MethodKind::Get => Op::DefineClassGetter,
            MethodKind::Set => Op::DefineClassSetter,
        };
        self.emit(op, &[home, register])?;
        self.release(name);
        Ok(())
    }

    /// Der Name eines Feldes im Akkumulator (Evaluation von
    /// *ClassElementName*): ein privater Name, ein fester oder ein
    /// berechneter nach ToPropertyKey.
    fn field_key(&mut self, key: NodeId) -> Result<()> {
        if let Node::PrivateName(atom) = *self.ast.node(key) {
            return self.load_private_name(atom, key);
        }
        if let Some(units) = self.property_key(key)? {
            let constant = self.constant(Constant::Name(units))?;
            self.emit(Op::LdaConstant, &[constant])?;
        }
        Ok(())
    }

    /// Ein statisches Feld oder ein `static`-Block nach den übrigen
    /// Elementen (ClassDefinitionEvaluation, Schritt 33): Das Feld bekommt
    /// mit DefineField den Wert seines Initialisierers mit dem Konstruktor
    /// als `this`, der Block wird mit ihm als `this` aufgerufen.
    fn static_element(&mut self, element: StaticElement, constructor: u32) -> Result<()> {
        let target = i64::from(constructor);
        match element {
            StaticElement::Field { node, key, value } => {
                let mark = self.next_register;
                let name = match key {
                    FieldKey::Register(register) => register,
                    FieldKey::Name(units) => {
                        let register = self.allocate()?;
                        let constant = self.constant(Constant::Name(units))?;
                        self.emit(Op::LdaConstant, &[constant])?;
                        self.emit(Op::Star, &[i64::from(register)])?;
                        register
                    }
                    FieldKey::Private(atom) => {
                        let register = self.allocate()?;
                        self.load_private_name(atom, node)?;
                        self.emit(Op::Star, &[i64::from(register)])?;
                        register
                    }
                };
                match value {
                    Some(initializer) => {
                        self.closure(node, initializer, ClosureName::Keyed(name))?;
                        self.home_object(initializer, target)?;
                        self.call_with_this(node, target)?;
                    }
                    None => self.emit(Op::LdaUndefined, &[])?,
                }
                self.position(node)?;
                self.emit(Op::DefineField, &[target, i64::from(name)])?;
                self.release(mark);
                Ok(())
            }
            StaticElement::Block { node, function } => {
                self.closure(node, function, ClosureName::Static(&[]))?;
                self.home_object(function, target)?;
                self.call_with_this(node, target)
            }
        }
    }

    /// Ruft die Funktion im Akkumulator ohne Argumente mit dem Wert im
    /// Register `this` als `this` auf.
    fn call_with_this(&mut self, node: NodeId, this: i64) -> Result<()> {
        let callee = self.allocate()?;
        let register = i64::from(callee);
        self.allocate()?;
        self.emit(Op::Star, &[register])?;
        self.emit(Op::Mov, &[this, register + 1])?;
        self.position(node)?;
        self.emit(Op::Call, &[register, 0])?;
        self.release(callee);
        Ok(())
    }

    /// Array-Literale (Abschnitte 13.2.4.1 und 13.2.4.2).
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
                Node::Spread(argument) => {
                    self.expression(argument)?;
                    self.position(element)?;
                    self.spread(register)?;
                }
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

    /// Hängt die Werte, die der Iterator des Akkumulators liefert, an das
    /// Array im Register `array` (ArrayAccumulation für *SpreadElement*,
    /// Abschnitt 13.2.4.1, und ArgumentListEvaluation, Abschnitt 13.3.8.1).
    /// Einen Fehler wirft der Iterator selbst; geschlossen wird er nicht.
    fn spread(&mut self, array: i64) -> Result<()> {
        let iterator = self.allocate()?;
        self.allocate()?;
        self.allocate()?;
        self.emit(Op::GetIterator, &[i64::from(iterator)])?;
        let start = self.emitter.offset();
        let done = self
            .emitter
            .jump_with(Op::IteratorNext, &[i64::from(iterator)])?;
        self.emit(Op::ArrayPush, &[array])?;
        self.emitter.jump_loop(start)?;
        self.bind(done)?;
        self.release(iterator);
        Ok(())
    }

    /// Der Kopf einer Kette aus Zugriffen und Aufrufen und ihre Glieder von
    /// innen nach außen, ohne Rekursion entlang der Kette.
    fn chain_links(&self, node: NodeId) -> (NodeId, Vec<NodeId>) {
        let mut links = Vec::new();
        let mut head = node;
        loop {
            match *self.ast.node(head) {
                Node::Member { object, .. }
                | Node::ComputedMember { object, .. }
                | Node::PrivateMember { object, .. } => {
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
            Node::Member { .. } | Node::ComputedMember { .. } | Node::PrivateMember { .. } => {
                self.chain(inner, ChainEnd::Callee(this))
            }
            Node::Chain(chain) => self.chain(chain, ChainEnd::Callee(this)),
            // `super.m()` ruft mit dem `this` der Referenz auf.
            Node::SuperMember(_) | Node::SuperComputedMember(_) => {
                let reference = self.super_reference(inner)?;
                self.get_value(reference, self.ast.range(inner).start)?;
                if let Reference::SuperNamed { base, .. } | Reference::SuperKeyed { base } =
                    reference
                {
                    let actual = base.checked_add(1).ok_or(CompileError::TooLarge)?;
                    self.emit(Op::Mov, &[i64::from(actual), i64::from(this)])?;
                }
                self.release_reference(reference);
                Ok(true)
            }
            _ => {
                self.expression(head)?;
                Ok(false)
            }
        }
    }

    /// Ein Aufruf der Funktion im Akkumulator (EvaluateCall, Abschnitt
    /// 13.3.6.2): Funktion, `this` und Argumente in den Registern ab
    /// `callee`. Ohne Basis ist `this` *undefined*. In Endposition wird es
    /// ein [`Op::TailCall`].
    fn call(&mut self, node: NodeId, callee: u32, has_this: bool, arguments: List) -> Result<()> {
        let register = i64::from(callee);
        self.emit(Op::Star, &[register])?;
        if !has_this {
            self.emit(Op::LdaUndefined, &[])?;
            self.emit(Op::Star, &[register + 1])?;
        }
        let op = if self.tail_calls.contains(&node) {
            Op::TailCall
        } else {
            Op::Call
        };
        self.call_arguments(node, op, callee, arguments)
    }

    /// `new` (EvaluateNew, Abschnitt 13.3.5.1.1): erst der Konstruktor,
    /// dann die Argumente, dann [`Op::Construct`].
    fn construct(&mut self, node: NodeId, callee: NodeId, arguments: List) -> Result<()> {
        let mark = self.next_register;
        let register = self.allocate()?;
        self.allocate()?;
        self.expression(callee)?;
        self.emit(Op::Star, &[i64::from(register)])?;
        self.call_arguments(node, Op::Construct, register, arguments)?;
        self.release(mark);
        Ok(())
    }

    /// Die Argumente eines Aufrufs in die Register hinter `callee` und
    /// `this`, dann der Befehl.
    fn call_arguments(&mut self, node: NodeId, op: Op, callee: u32, arguments: List) -> Result<()> {
        let register = i64::from(callee);
        // Die Argumente folgen auf `this`; Zwischenwerte davor sind schon
        // zurückgegeben.
        let first = self.next_register;
        if callee.checked_add(2) != Some(first) {
            return Err(self.unsupported(node));
        }
        let ast = self.ast;
        let arguments = ast.list(arguments);
        if arguments
            .iter()
            .any(|&argument| matches!(self.ast.node(argument), Node::Spread(_)))
        {
            return self.spread_arguments(node, op, callee, arguments);
        }
        for &argument in arguments {
            let slot = self.allocate()?;
            self.expression(argument)?;
            self.emit(Op::Star, &[i64::from(slot)])?;
        }
        self.position(node)?;
        let count = i64::try_from(arguments.len()).map_err(|_| CompileError::TooLarge)?;
        self.emit(op, &[register, count])?;
        self.release(first);
        Ok(())
    }

    /// Die Argumente eines Aufrufs mit Ausbreitung als Elemente eines neuen
    /// Arrays im Register hinter `this`, dann der Befehl für Ausbreitung zu
    /// `op` ([`Op::CallSpread`]).
    fn spread_arguments(
        &mut self,
        node: NodeId,
        op: Op,
        callee: u32,
        arguments: &[NodeId],
    ) -> Result<()> {
        let op = match op {
            Op::TailCall => Op::TailCallSpread,
            Op::Construct => Op::ConstructSpread,
            Op::SuperCall => Op::SuperCallSpread,
            _ => Op::CallSpread,
        };
        let array = self.allocate()?;
        let register = i64::from(array);
        self.emit(Op::CreateArray, &[])?;
        self.emit(Op::Star, &[register])?;
        for &argument in arguments {
            if let Node::Spread(inner) = *self.ast.node(argument) {
                self.expression(inner)?;
                self.position(argument)?;
                self.spread(register)?;
            } else {
                self.expression(argument)?;
                self.emit(Op::ArrayPush, &[register])?;
            }
        }
        self.position(node)?;
        self.emit(op, &[i64::from(callee)])?;
        self.release(array);
        Ok(())
    }

    /// Die Aufrufe in Endposition im Ausdruck `node`, nach
    /// HasCallInTailPosition (Abschnitt 15.10.2.2), wenn der Ausdruck selbst
    /// in Endposition steht: im strikten Modus und nicht in `try`.
    fn tail_position_calls(&self, node: NodeId) -> BTreeSet<NodeId> {
        let mut calls = BTreeSet::new();
        // IsInTailPosition (Abschnitt 15.10.1): nie im Rumpf eines
        // Generators oder einer async-Funktion.
        if !self.strict
            || self.protected > 0
            || self.generator.is_some()
            || self.async_state.is_some()
        {
            return calls;
        }
        let mut pending = vec![node];
        while let Some(node) = pending.pop() {
            match *self.ast.node(node) {
                Node::Paren(inner) => pending.push(inner),
                Node::Sequence(list) => pending.extend(self.ast.list(list).last()),
                Node::Conditional {
                    consequent,
                    alternate,
                    ..
                } => pending.extend([consequent, alternate]),
                Node::Binary {
                    operator:
                        BinaryOperator::LogicalAnd
                        | BinaryOperator::LogicalOr
                        | BinaryOperator::Coalesce,
                    right,
                    ..
                } => pending.push(right),
                Node::Call { .. } => {
                    calls.insert(node);
                }
                // Am Ende einer optionalen Kette zählt nur ein Aufruf.
                Node::Chain(chain) if self.is_call(chain) => {
                    calls.insert(chain);
                }
                _ => {}
            }
        }
        calls
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
            Node::PrivateMember { name, optional, .. } => (Access::Private(name), optional),
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
            // `delete` lässt der Parser für private Namen nicht zu.
            Access::Private(_) if delete => Err(self.unsupported(link)),
            Access::Private(atom) => {
                let mark = self.next_register;
                let base = match this {
                    Some(this) => this,
                    None => self.allocate()?,
                };
                self.emit(Op::Star, &[i64::from(base)])?;
                let name = self.allocate()?;
                self.load_private_name(atom, link)?;
                self.emit(Op::Star, &[i64::from(name)])?;
                self.emit(Op::Ldar, &[i64::from(base)])?;
                self.position(link)?;
                self.emit(Op::LdaPrivate, &[i64::from(name)])?;
                self.release(mark);
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

    /// Lädt den Wert einer Bindung ohne Prüfung in den Akkumulator.
    fn load_location(&mut self, location: Location) -> Result<()> {
        match location {
            Location::Register(register) => self.emit(Op::Ldar, &[i64::from(register)]),
            Location::Context { depth, slot } => {
                self.emit(Op::LdaContext, &[i64::from(depth), i64::from(slot)])
            }
        }
    }

    /// Schreibt den Akkumulator ohne Prüfung in eine Bindung.
    fn store_location(&mut self, location: Location) -> Result<()> {
        match location {
            Location::Register(register) => self.emit(Op::Star, &[i64::from(register)]),
            Location::Context { depth, slot } => {
                self.emit(Op::StaContext, &[i64::from(depth), i64::from(slot)])
            }
        }
    }

    fn load_local(&mut self, binding: LocalBinding, atom: Atom, position: usize) -> Result<()> {
        let check = Self::needs_hole_check(binding, position);
        match binding.location {
            Location::Register(register) => {
                let register = i64::from(register);
                if check {
                    let name = self.name(atom)?;
                    self.emit(Op::CheckHole, &[register, name])?;
                }
                self.emit(Op::Ldar, &[register])
            }
            Location::Context { .. } => {
                self.load_location(binding.location)?;
                if check {
                    let name = self.name(atom)?;
                    self.emit(Op::ThrowIfHole, &[name])?;
                }
                Ok(())
            }
        }
    }

    /// PutValue für eine Bindung in einem Register oder Kontext
    /// (SetMutableBinding, Abschnitt 9.1.1.1.5): erst die TDZ, dann die
    /// Konstante.
    fn store_local(&mut self, binding: LocalBinding, atom: Atom, position: usize) -> Result<()> {
        if binding.kind == BindingKind::FunctionName {
            if self.is_strict() {
                let name = self.name(atom)?;
                return self.emit(Op::ThrowConstAssignment, &[name]);
            }
            return Ok(());
        }
        let constant = binding.kind == BindingKind::Const;
        if !Self::needs_hole_check(binding, position) && !constant {
            return self.store_location(binding.location);
        }
        let name = self.name(atom)?;
        match binding.location {
            Location::Register(register) => {
                self.emit(Op::CheckHole, &[i64::from(register), name])?;
            }
            Location::Context { .. } => {
                // Der Wert wartet in einem Register, während der
                // Akkumulator die Bindung prüft.
                let value = self.allocate()?;
                self.emit(Op::Star, &[i64::from(value)])?;
                self.load_location(binding.location)?;
                self.emit(Op::ThrowIfHole, &[name])?;
                if !constant {
                    self.emit(Op::Ldar, &[i64::from(value)])?;
                }
                self.release(value);
            }
        }
        if constant {
            return self.emit(Op::ThrowConstAssignment, &[name]);
        }
        self.store_location(binding.location)
    }

    /// `this` (Abschnitt 13.2.1): in Pfeilfunktionen aus dem Kontext der
    /// Funktion oder des Scripts, das sie umgibt. Im Konstruktor einer
    /// abgeleiteten Klasse und in Pfeilfunktionen darin wirft die Lücke den
    /// ReferenceError aus GetThisBinding (Abschnitt 9.1.1.3.3).
    fn load_this(&mut self, node: NodeId) -> Result<()> {
        if self.lexical_this {
            let (depth, slot, derived) = self.find_lexical(node, |layout| layout.this)?;
            self.emit(Op::LdaContext, &[i64::from(depth), i64::from(slot)])?;
            if derived {
                let name = self.constant(Constant::Name(units("this")))?;
                self.emit(Op::ThrowIfHole, &[name])?;
            }
            return Ok(());
        }
        if !self.derived {
            return self.emit(Op::Ldar, &[THIS]);
        }
        let name = self.constant(Constant::Name(units("this")))?;
        match self.own_this() {
            Some((depth, slot)) => {
                self.emit(Op::LdaContext, &[i64::from(depth), i64::from(slot)])?;
                self.emit(Op::ThrowIfHole, &[name])
            }
            None => {
                self.emit(Op::CheckHole, &[THIS, name])?;
                self.emit(Op::Ldar, &[THIS])
            }
        }
    }

    /// Der Platz im Kontext mit dem eigenen `this`, wenn Pfeilfunktionen es
    /// lesen: wie viele Stufen außerhalb des aktuellen und welcher.
    fn own_this(&self) -> Option<(u32, u32)> {
        let mut depth: u32 = 0;
        for scope in self.scopes.iter().rev() {
            if let Some(slot) = scope.this {
                return Some((depth, slot));
            }
            if scope.has_context() {
                depth = depth.saturating_add(1);
            }
        }
        None
    }

    /// BindThisValue (Abschnitt 9.1.1.3.1) mit dem Ergebnis von `super()` im
    /// Akkumulator: ein ReferenceError, wenn `this` schon gebunden ist. Der
    /// Akkumulator behält das Ergebnis.
    fn bind_this(&mut self, node: NodeId) -> Result<()> {
        let place = if self.lexical_this {
            let (depth, slot, _) = self.find_lexical(node, |layout| layout.this)?;
            Some((depth, slot))
        } else {
            self.own_this()
        };
        let result = self.allocate()?;
        self.emit(Op::Star, &[i64::from(result)])?;
        match place {
            Some((depth, slot)) => {
                self.emit(Op::LdaContext, &[i64::from(depth), i64::from(slot)])?;
            }
            None => self.emit(Op::Ldar, &[THIS])?,
        }
        self.emit(Op::ThrowIfNotHole, &[])?;
        self.emit(Op::Ldar, &[i64::from(result)])?;
        match place {
            Some((depth, slot)) => {
                self.emit(Op::StaContext, &[i64::from(depth), i64::from(slot)])?;
            }
            None => self.emit(Op::Star, &[THIS])?,
        }
        self.release(result);
        Ok(())
    }

    /// Beendet den Rahmen mit dem Akkumulator. Im Konstruktor einer
    /// abgeleiteten Klasse, dessen `this` im Kontext liegt, kommt es vorher
    /// zurück in Register 0; dort prüft [[Construct]] die Bindung. In einer
    /// async-Funktion löst der Akkumulator vorher das Promise auf, mit dem
    /// der Rahmen dann zurückkehrt.
    fn emit_return(&mut self) -> Result<()> {
        if self.derived
            && let Some((depth, slot)) = self.own_this()
        {
            let value = self.allocate()?;
            self.emit(Op::Star, &[i64::from(value)])?;
            self.emit(Op::LdaContext, &[i64::from(depth), i64::from(slot)])?;
            self.emit(Op::Star, &[THIS])?;
            self.emit(Op::Ldar, &[i64::from(value)])?;
            self.release(value);
        }
        // AsyncBlockStart (Abschnitt 27.10.5.2): Ein `return` und das Ende
        // des Rumpfs lösen das Promise auf.
        if self.async_state.is_some() {
            self.emit(Op::ResolveAsync, &[])?;
        }
        self.emit(Op::Return, &[])
    }

    /// Das Funktionsobjekt, für das `super` gilt (GetThisEnvironment und
    /// [[FunctionObject]]): Register 1, in Pfeilfunktionen aus dem Kontext
    /// der Funktion um sie.
    fn load_function(&mut self, node: NodeId) -> Result<()> {
        if !self.lexical_this {
            return self.emit(Op::Ldar, &[1]);
        }
        let (depth, slot, _) = self.find_lexical(node, |layout| layout.function)?;
        self.emit(Op::LdaContext, &[i64::from(depth), i64::from(slot)])
    }

    /// Eine Referenz mit `super` (Abschnitt 13.3.7.1): erst GetThisBinding,
    /// dann der berechnete Name ohne ToPropertyKey, dann GetSuperBase
    /// (MakeSuperPropertyReference, Abschnitt 13.3.7.3). Basis, `this` und
    /// Name liegen in Registern hintereinander.
    fn super_reference(&mut self, node: NodeId) -> Result<Reference> {
        let base = self.allocate()?;
        let this = self.allocate()?;
        self.load_this(node)?;
        self.emit(Op::Star, &[i64::from(this)])?;
        let reference = match *self.ast.node(node) {
            Node::SuperMember(atom) => {
                let name = self.name(atom)?;
                Reference::SuperNamed { base, name }
            }
            Node::SuperComputedMember(expression) => {
                let key = self.allocate()?;
                self.expression(expression)?;
                self.emit(Op::Star, &[i64::from(key)])?;
                Reference::SuperKeyed { base }
            }
            _ => return Err(self.unsupported(node)),
        };
        self.position(node)?;
        self.load_function(node)?;
        self.emit(Op::GetSuperBase, &[])?;
        self.emit(Op::Star, &[i64::from(base)])?;
        Ok(reference)
    }

    /// `super(...)` (Abschnitt 13.3.7.1): GetNewTarget, GetSuperConstructor,
    /// die Argumente, dann [[Construct]] ([`Op::SuperCall`]), BindThisValue
    /// und InitializeInstanceElements mit dem Funktionsobjekt, für das
    /// `super` gilt. Der Akkumulator hält danach das Ergebnis.
    fn super_call(&mut self, node: NodeId, arguments: List) -> Result<()> {
        let mark = self.next_register;
        let new_target = self.allocate()?;
        let callee = self.allocate()?;
        self.allocate()?;
        self.load_new_target(node)?;
        self.emit(Op::Star, &[i64::from(new_target)])?;
        self.load_function(node)?;
        self.position(node)?;
        self.emit(Op::GetSuperConstructor, &[])?;
        self.emit(Op::Star, &[i64::from(callee)])?;
        self.call_arguments(node, Op::SuperCall, callee, arguments)?;
        self.release(mark);
        self.position(node)?;
        self.bind_this(node)?;
        let first = self.allocate()?;
        for _ in 0..4 {
            self.allocate()?;
        }
        let register = i64::from(first);
        self.emit(Op::Star, &[register + 3])?;
        self.load_function(node)?;
        self.emit(Op::Star, &[register])?;
        emit_instance_elements(&mut self.emitter, register)?;
        self.emit(Op::Ldar, &[register + 3])?;
        self.release(first);
        Ok(())
    }

    /// `new.target` (Abschnitt 13.3.12.1): GetNewTarget aus dem eigenen
    /// Register, in Pfeilfunktionen aus dem Kontext der Funktion.
    fn load_new_target(&mut self, node: NodeId) -> Result<()> {
        if self.lexical_this {
            return self.load_lexical(node, |layout| layout.new_target);
        }
        match self.new_target {
            Some(register) => self.emit(Op::Ldar, &[i64::from(register)]),
            None => Err(self.unsupported(node)),
        }
    }

    /// Liest den innersten Platz, den `slot` in den Kontexten um den Code
    /// nennt, für `this` und `new.target` in Pfeilfunktionen.
    fn load_lexical(
        &mut self,
        node: NodeId,
        slot: impl Fn(&ContextLayout) -> Option<u32>,
    ) -> Result<()> {
        let (depth, slot, _) = self.find_lexical(node, slot)?;
        self.emit(Op::LdaContext, &[i64::from(depth), i64::from(slot)])
    }

    /// Der innerste Platz, den `slot` in den Kontexten um den Code nennt:
    /// Tiefe, Platz und ob sein Kontext `this` als Lücke halten kann.
    fn find_lexical(
        &self,
        node: NodeId,
        slot: impl Fn(&ContextLayout) -> Option<u32>,
    ) -> Result<(u32, u32, bool)> {
        let mut depth = self.open_contexts();
        let mut enclosing = Some(&self.outer);
        while let Some(current) = enclosing {
            for layout in &current.contexts {
                if let Some(slot) = slot(layout) {
                    return Ok((depth, slot, layout.derived));
                }
                depth = depth.checked_add(1).ok_or(CompileError::TooLarge)?;
            }
            enclosing = current.parent.as_ref();
        }
        Err(self.unsupported(node))
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
            // ResolveBinding auch für die Namen in `var`.
            Node::Identifier(atom) | Node::BindingIdentifier(atom) => {
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
            Node::SuperMember(_) | Node::SuperComputedMember(_) => {
                self.super_reference(self.without_parens(target))
            }
            Node::PrivateMember {
                object,
                name,
                optional: false,
            } => {
                self.expression(object)?;
                let base = self.allocate()?;
                self.emit(Op::Star, &[i64::from(base)])?;
                let register = self.allocate()?;
                self.load_private_name(name, target)?;
                self.emit(Op::Star, &[i64::from(register)])?;
                Ok(Reference::Private {
                    base,
                    name: register,
                })
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
            Reference::SuperNamed { base, name } => {
                self.emit(Op::LdaSuperNamed, &[i64::from(base), name])
            }
            Reference::SuperKeyed { base } => self.emit(Op::LdaSuperKeyed, &[i64::from(base)]),
            Reference::Private { base, name } => {
                self.emit(Op::Ldar, &[i64::from(base)])?;
                self.emit(Op::LdaPrivate, &[i64::from(name)])
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
            Reference::SuperNamed { base, name } => {
                self.emit(Op::StaSuperNamed, &[i64::from(base), name])
            }
            Reference::SuperKeyed { base } => self.emit(Op::StaSuperKeyed, &[i64::from(base)]),
            Reference::Private { base, name } => {
                self.emit(Op::StaPrivate, &[i64::from(base), i64::from(name)])
            }
        }
    }

    fn release_reference(&mut self, reference: Reference) {
        match reference {
            Reference::Local(..) => {}
            Reference::Global { register, .. }
            | Reference::Named { base: register, .. }
            | Reference::Keyed { base: register, .. }
            | Reference::SuperNamed { base: register, .. }
            | Reference::SuperKeyed { base: register }
            | Reference::Private { base: register, .. } => self.release(register),
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
        if operator == AssignOperator::Assign
            && matches!(
                self.ast.node(target),
                Node::ArrayPattern(_) | Node::ObjectPattern(_)
            )
        {
            // Erst die rechte Seite, dann das Muster; das Ergebnis ist der
            // Wert der rechten Seite.
            self.expression(value)?;
            let result = self.allocate()?;
            self.emit(Op::Star, &[i64::from(result)])?;
            self.destructure(target, Init::Put)?;
            self.emit(Op::Ldar, &[i64::from(result)])?;
            self.release(result);
            return Ok(());
        }
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
            // Schritt 4.b: erst die Referenz, dann der ReferenceError.
            Node::SuperMember(_) | Node::SuperComputedMember(_) => {
                let reference = self.super_reference(target)?;
                self.position(target)?;
                self.emit(Op::ThrowReferenceError, &[])?;
                self.release_reference(reference);
                return Ok(());
            }
            Node::PrivateMember { .. } => return Err(self.unsupported(inner)),
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
                    // `#x in y`: links der private Name.
                    if let Node::PrivateName(atom) = *self.ast.node(left) {
                        self.load_private_name(atom, left)?;
                        continue;
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
            BinaryOperator::In => match self.ast.node(node) {
                Node::Binary { left, .. }
                    if matches!(self.ast.node(*left), Node::PrivateName(_)) =>
                {
                    (Op::TestPrivateIn, false)
                }
                _ => (Op::TestIn, false),
            },
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
