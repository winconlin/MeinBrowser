//! Parser nach ECMA-262, Abschnitte 13 bis 16, mit den Erweiterungen für
//! Webbrowser aus Anhang B.3 und allen frühen Fehlern (*early errors*)
//! dieser Abschnitte.
//!
//! - Ein rekursiver Abstieg über die Grammatik mit einem Token Vorausschau,
//!   an wenigen Stellen bis zu drei (`await using x`, `using of =`,
//!   `async x =>`). Welches Ziel der Lexer für ein Token nimmt,
//!   entscheidet der Parser; hat er ein Token mit `/` oder `}` schon mit
//!   einem anderen Ziel gelesen, liest er es neu.
//! - Cover-Grammatiken (Abschnitt 5.1.4): Klammerausdrücke vor `=>`, Aufrufe
//!   von `async` vor `=>` und Objekt- und Array-Literale vor `=` werden
//!   zuerst als Ausdruck gelesen und dann an Ort und Stelle zu Parametern
//!   oder Mustern umgedeutet ([`pattern`]). Was nur im Ausdruck ein Fehler
//!   ist, merkt sich [`Cover`] bis zur Entscheidung.
//! - Die frühen Fehler prüft der Parser beim Lesen, ohne einen zweiten Gang
//!   über den Baum: Deklarationen in Gültigkeitsbereichen ([`scope`]),
//!   Sprungmarken und Sprünge, `super`, `new.target`, `arguments`, private
//!   Namen und der strikte Modus. Was eine spätere Direktive `"use strict"`
//!   noch ändert (Name und Parameter einer Funktion, frühere Direktiven),
//!   wird beim Lesen der Direktive nachgeprüft.
//! - Die Verschachtelung ist begrenzt (RS-8, [`MAX_DEPTH`]); tiefer
//!   verschachtelter Quelltext ergibt [`ErrorKind::TooDeep`].
//! - Was noch fehlt: das Prüfen der Muster regulärer Ausdrücke (die Flags
//!   werden geprüft), Quelltext für `eval` und den Konstruktor `Function`
//!   und das bloße Vorprüfen von Funktionsrümpfen ohne Baum.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Ast, Atom, List, Node, NodeId};
use crate::lexer::{self, Goal, Lexer, Name, Punctuator, Token, TokenKind};

mod class;
mod expression;
mod function;
mod module;
mod pattern;
mod scope;
mod statement;
#[cfg(test)]
mod test262;
#[cfg(test)]
mod tests;

use scope::Scope;

/// Wie tief Ausdrücke, Anweisungen und Muster höchstens verschachtelt
/// sind. Jede Ebene kostet einige Stapelrahmen; der Test
/// `deepest_nesting_fits_on_a_small_stack` prüft, dass die tiefste Eingabe
/// in einem Faden mit 1 MiB Stapel bleibt.
pub const MAX_DEPTH: u32 = 256;

/// Ein früher Fehler (*SyntaxError*) mit seiner Lage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub kind: ErrorKind,
    /// Wo der Fehler erkannt wurde, in Codeeinheiten.
    pub position: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Lexer(lexer::ErrorKind),
    /// Ein Token, das die Grammatik an dieser Stelle nicht erlaubt.
    UnexpectedToken,
    /// Mehr als `u32::MAX` Codeeinheiten oder Einträge im Baum.
    TooLong,
    /// Tiefer verschachtelt als [`MAX_DEPTH`].
    TooDeep,
    /// Ein reserviertes Wort als Bezeichner, auch eines, das nur im
    /// strikten Modus, in Generatoren, in async-Funktionen oder in Modulen
    /// reserviert ist.
    ReservedWord,
    /// `eval` oder `arguments` als Bindung oder Ziel im strikten Modus.
    StrictEvalOrArguments,
    /// Altformen von Zahlen und Escapes im strikten Modus.
    StrictLegacyLiteral,
    /// `with` im strikten Modus.
    StrictWith,
    /// `delete` eines Bezeichners im strikten Modus oder eines privaten
    /// Namens.
    InvalidDelete,
    /// Ein Template ohne Tag mit einem ungültigen Escape.
    InvalidTemplateEscape,
    InvalidRegExpFlags,
    /// Ein Ziel einer Zuweisung oder von `++` und `--`, das keines ist.
    InvalidAssignmentTarget,
    /// Ein Ausdruck, der nur als Muster oder als Parameter einer
    /// Pfeilfunktion gültig wäre, etwa `{a = 1}` oder `(...a)`.
    InvalidCoverGrammar,
    /// Ein Name, der im selben Gültigkeitsbereich schon deklariert ist.
    Redeclaration,
    DuplicateParameter,
    /// Zwei Eigenschaften `__proto__: …` in einem Objektliteral.
    DuplicateProto,
    DuplicateLabel,
    UndefinedLabel,
    /// `break` außerhalb einer Schleife oder von `switch`, `continue`
    /// außerhalb einer Schleife oder zu einer Marke, die keine Schleife
    /// markiert.
    InvalidJump,
    /// `return` außerhalb einer Funktion.
    InvalidReturn,
    /// `super` an einer Stelle, die es nicht erlaubt.
    InvalidSuper,
    InvalidNewTarget,
    InvalidImportMeta,
    /// `arguments` in einer Feldinitialisierung oder einem `static`-Block.
    InvalidArguments,
    /// `yield` oder `await` an einer Stelle, die keinen solchen Ausdruck
    /// erlaubt, etwa in Parametern.
    InvalidYieldOrAwait,
    /// `"use strict"` in einer Funktion mit Parametern, die nicht einfach
    /// sind.
    UseStrictWithComplexParameters,
    /// Ein privater Name, den keine umgebende Klasse deklariert.
    UndeclaredPrivateName,
    DuplicatePrivateName,
    /// Zwei Konstruktoren, ein Konstruktor als Getter, Setter, Generator
    /// oder async-Methode, ein Feld `constructor`, eine statische
    /// Eigenschaft `prototype` oder `#constructor`.
    InvalidClassElement,
    /// `const`, `using` oder `await using` ohne Initialisierung.
    MissingInitializer,
    /// `let` als Name einer lexikalischen Bindung.
    LetBinding,
    /// Eine Funktion mit Marke an einer Stelle, die nur eine Anweisung
    /// erlaubt, oder im strikten Modus.
    InvalidLabelledFunction,
    /// Eine Funktionsdeklaration, wo nur eine Anweisung stehen darf.
    InvalidFunctionDeclaration,
    /// Ein Kopf von `for`-`in` oder `for`-`of`, den die Grammatik nicht
    /// erlaubt, etwa mit Initialisierung.
    InvalidForInOf,
    /// `??` gemischt mit `&&` oder `||` ohne Klammern.
    MixedCoalesce,
    /// Ein einstelliger Operator links von `**`.
    UnaryBeforeExponent,
    /// Ein Template oder `new` an einer optionalen Kette.
    InvalidOptionalChain,
    /// `using` oder `await using` an einer Stelle, die es nicht erlaubt.
    InvalidUsing,
    /// Ein Name, der zweimal exportiert wird.
    DuplicateExport,
    /// Ein exportierter Name ohne Deklaration im Modul.
    UndeclaredExport,
    /// Ein Name für den Export aus einer Zeichenkette, die kein gültiges
    /// UTF-16 ist, oder eine Zeichenkette als lokaler Name.
    InvalidExportName,
    DuplicateImportAttribute,
    /// `import` und `export` außerhalb der obersten Ebene eines Moduls.
    InvalidModuleItem,
}

impl From<lexer::Error> for SyntaxError {
    fn from(error: lexer::Error) -> SyntaxError {
        SyntaxError {
            kind: ErrorKind::Lexer(error.kind),
            position: error.position,
        }
    }
}

type Result<T> = std::result::Result<T, SyntaxError>;

/// Liest ein Script (Abschnitt 16.1).
pub fn parse_script(source: &[u16]) -> Result<Ast> {
    Parser::new(source, false)?.program()
}

/// Liest ein Modul (Abschnitt 16.2).
pub fn parse_module(source: &[u16]) -> Result<Ast> {
    Parser::new(source, true)?.program()
}

/// Was für den Code einer Funktion gilt, die Parameter der Grammatik
/// eingeschlossen.
#[derive(Clone, Copy, Debug, Default)]
struct Context {
    strict: bool,
    /// *[Yield]*: `yield` ist ein Operator.
    yield_: bool,
    /// *[Await]*: `await` ist ein Operator.
    await_: bool,
    /// *[Return]*
    return_: bool,
    /// `super.x` und `super[x]` sind erlaubt.
    super_property: bool,
    /// `super(…)` ist erlaubt.
    super_call: bool,
    new_target: bool,
    /// In Feldinitialisierungen und `static`-Blöcken ist `arguments`
    /// verboten (*ContainsArguments*).
    no_arguments: bool,
    /// In `static`-Blöcken ist `await` verboten (*Contains `await`*).
    static_block: bool,
}

/// Eine Sprungmarke und ob sie eine Schleife markiert.
#[derive(Clone, Copy, Debug)]
struct Label {
    name: Atom,
    iteration: bool,
}

/// Der Zustand der Funktion, die gerade gelesen wird.
#[derive(Debug, Default)]
struct FunctionState {
    context: Context,
    labels: Vec<Label>,
    /// Ab hier markieren die Marken in `labels` die nächste Anweisung.
    pending_labels: usize,
    /// Wie viele Schleifen und `switch` die Stelle umgeben.
    iterations: u32,
    switches: u32,
    /// Wie viele `yield`- und `await`-Ausdrücke bisher vorkamen, für die
    /// Regeln über Parameter.
    yield_await: u32,
    /// Wie oft `await` bisher als Bezeichner vorkam, für die Parameter von
    /// async-Pfeilfunktionen.
    await_identifiers: u32,
}

impl FunctionState {
    fn new(context: Context) -> FunctionState {
        FunctionState {
            context,
            ..FunctionState::default()
        }
    }
}

/// Was nur ein Fehler ist, wenn ein Ausdruck nicht als Muster oder als
/// Parameter umgedeutet wird.
#[derive(Clone, Copy, Debug, Default)]
struct Cover {
    /// Ein *CoverInitializedName* (`{a = 1}`) oder zwei `__proto__`.
    error: Option<SyntaxError>,
}

impl Cover {
    fn record(&mut self, error: SyntaxError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    fn check(self) -> Result<()> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// Die privaten Namen einer Klasse.
#[derive(Debug, Default)]
struct PrivateScope {
    /// Deklarierte Namen: ob sie Getter oder Setter sind und ob statisch.
    declared: BTreeMap<Atom, PrivateDeclaration>,
    /// Verwendete Namen, die noch nicht aufgelöst sind.
    used: Vec<(Atom, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivateDeclaration {
    /// Feld oder Methode.
    Other,
    Getter {
        is_static: bool,
    },
    Setter {
        is_static: bool,
    },
    /// Getter und Setter.
    Accessor,
}

/// Ein Token mit dem Ziel, mit dem es gelesen wurde, und dem Lexer danach.
#[derive(Clone, Debug)]
struct Lookahead<'a> {
    goal: Goal,
    token: std::result::Result<Token, lexer::Error>,
    after: Lexer<'a>,
}

pub(crate) struct Parser<'a> {
    source: &'a [u16],
    /// Der Lexer vor dem aktuellen Token.
    lexer: Lexer<'a>,
    current: Option<Lookahead<'a>>,
    /// Ob noch kein Token verbraucht ist (Ziel *InputElementHashbangOrRegExp*).
    at_start: bool,
    /// Das Ende des zuletzt verbrauchten Tokens.
    previous_end: usize,
    ast: Ast,
    /// Gemeinsamer Stapel für Listen im Aufbau.
    scratch: Vec<NodeId>,
    module: bool,
    function: FunctionState,
    scopes: Vec<Scope>,
    classes: Vec<PrivateScope>,
    depth: u32,
    /// Exportierte Namen und lokale Bindungen hinter `export`, für die
    /// Prüfungen am Ende eines Moduls.
    exported_names: BTreeSet<Vec<u16>>,
    exported_bindings: Vec<(Atom, usize)>,
    /// Der zuletzt erkannte Kopf einer Pfeilfunktion, vor `=>`.
    arrow_head: Option<expression::ArrowHead>,
    /// Spreads mit einem Komma dahinter: Sie können kein Rest-Element
    /// werden.
    spread_with_comma: BTreeSet<NodeId>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a [u16], module: bool) -> Result<Parser<'a>> {
        if u32::try_from(source.len()).is_err() {
            return Err(SyntaxError {
                kind: ErrorKind::TooLong,
                position: 0,
            });
        }
        let context = Context {
            strict: module,
            await_: module,
            ..Context::default()
        };
        Ok(Parser {
            source,
            lexer: Lexer::new(source, module),
            current: None,
            at_start: true,
            previous_end: 0,
            ast: Ast::default(),
            scratch: Vec::new(),
            module,
            function: FunctionState::new(context),
            scopes: Vec::new(),
            classes: Vec::new(),
            depth: 0,
            exported_names: BTreeSet::new(),
            exported_bindings: Vec::new(),
            arrow_head: None,
            spread_with_comma: BTreeSet::new(),
        })
    }

    fn program(mut self) -> Result<Ast> {
        let kind = if self.module {
            scope::ScopeKind::Module
        } else {
            scope::ScopeKind::Script
        };
        self.scopes.push(Scope::new(kind));
        let mark = self.scratch.len();
        if self.module {
            while !self.at_end()? {
                let item = self.module_item()?;
                self.scratch.push(item);
            }
            self.check_exports()?;
        } else {
            self.directives()?;
            while !self.at_end()? {
                let item = self.statement_list_item()?;
                self.scratch.push(item);
            }
        }
        let body = self.finish_list(mark);
        let strict = self.function.context.strict;
        self.ast.finish(body, self.module, strict);
        if self.ast.overflowed() {
            return Err(SyntaxError {
                kind: ErrorKind::TooLong,
                position: self.source.len(),
            });
        }
        Ok(self.ast)
    }

    // Tokens

    /// Das aktuelle Token, gelesen mit dem Ziel `goal`.
    fn token(&mut self, goal: Goal) -> Result<&Token> {
        let goal = if self.at_start && goal == Goal::RegExp {
            Goal::HashbangOrRegExp
        } else {
            goal
        };
        let stale = match &self.current {
            None => true,
            Some(lookahead) => lookahead.goal != goal && self.goal_matters(lookahead),
        };
        if stale {
            let mut after = self.lexer.clone();
            let token = after.next_token(goal);
            self.current = Some(Lookahead { goal, token, after });
        }
        match self.current.as_ref().map(|lookahead| &lookahead.token) {
            Some(Ok(token)) => Ok(token),
            Some(Err(error)) => Err((*error).into()),
            None => Err(self.error(ErrorKind::UnexpectedToken)),
        }
    }

    /// Ob das Token mit einem anderen Ziel anders aussehen könnte: Es
    /// beginnt mit `/`, `}` oder `#!` oder war ein Fehler.
    fn goal_matters(&self, lookahead: &Lookahead<'a>) -> bool {
        match &lookahead.token {
            Err(_) => true,
            Ok(token) => matches!(
                self.source.get(token.range.start).copied(),
                Some(0x2F | 0x7D | 0x23)
            ),
        }
    }

    /// Verbraucht das aktuelle Token.
    fn advance(&mut self) {
        if let Some(lookahead) = self.current.take() {
            if let Ok(token) = &lookahead.token {
                self.previous_end = token.range.end;
            }
            self.lexer = lookahead.after;
            self.at_start = false;
        }
    }

    /// Das Token nach dem aktuellen, gelesen mit `goal`. Ein Fehler beim
    /// Lesen ergibt `None`; er zeigt sich, wenn das Token gelesen wird.
    fn peek(&mut self, current: Goal, goal: Goal) -> Result<Option<Token>> {
        self.token(current)?;
        let Some(lookahead) = &self.current else {
            return Ok(None);
        };
        let mut lexer = lookahead.after.clone();
        Ok(lexer.next_token(goal).ok())
    }

    fn start(&mut self, goal: Goal) -> Result<usize> {
        Ok(self.token(goal)?.range.start)
    }

    fn punctuator(&mut self, goal: Goal) -> Result<Option<Punctuator>> {
        Ok(match self.token(goal)?.kind {
            TokenKind::Punctuator(punctuator) => Some(punctuator),
            _ => None,
        })
    }

    fn at(&mut self, punctuator: Punctuator, goal: Goal) -> Result<bool> {
        Ok(self.punctuator(goal)? == Some(punctuator))
    }

    fn eat(&mut self, punctuator: Punctuator, goal: Goal) -> Result<bool> {
        let found = self.at(punctuator, goal)?;
        if found {
            self.advance();
        }
        Ok(found)
    }

    fn expect(&mut self, punctuator: Punctuator, goal: Goal) -> Result<()> {
        if self.eat(punctuator, goal)? {
            Ok(())
        } else {
            Err(self.unexpected(goal))
        }
    }

    /// Ob das aktuelle Token das Wort `word` ohne Escapes ist, also das
    /// Terminal `word` der Grammatik.
    fn at_keyword(&mut self, word: &str, goal: Goal) -> Result<bool> {
        Ok(matches!(
            &self.token(goal)?.kind,
            TokenKind::Name(name) if !name.escaped && name.value == word
        ))
    }

    fn eat_keyword(&mut self, word: &str, goal: Goal) -> Result<bool> {
        let found = self.at_keyword(word, goal)?;
        if found {
            self.advance();
        }
        Ok(found)
    }

    fn expect_keyword(&mut self, word: &str, goal: Goal) -> Result<()> {
        if self.eat_keyword(word, goal)? {
            Ok(())
        } else {
            Err(self.unexpected(goal))
        }
    }

    /// Der Name, wenn das aktuelle Token ein *IdentifierName* ist.
    fn name(&mut self, goal: Goal) -> Result<Option<Name>> {
        Ok(match &self.token(goal)?.kind {
            TokenKind::Name(name) => Some(name.clone()),
            _ => None,
        })
    }

    fn newline_before(&mut self, goal: Goal) -> Result<bool> {
        Ok(self.token(goal)?.newline_before)
    }

    fn at_end(&mut self) -> Result<bool> {
        Ok(self.token(Goal::RegExp)?.kind == TokenKind::End)
    }

    /// Ein `;` oder die automatische Semikolon-Einfügung (Abschnitt 12.10).
    fn semicolon(&mut self) -> Result<()> {
        let token = self.token(Goal::Div)?;
        if token.kind == TokenKind::Punctuator(Punctuator::Semicolon) {
            self.advance();
            return Ok(());
        }
        if token.newline_before
            || matches!(
                token.kind,
                TokenKind::End | TokenKind::Punctuator(Punctuator::RightBrace)
            )
        {
            return Ok(());
        }
        Err(self.unexpected(Goal::Div))
    }

    // Fehler

    fn error(&self, kind: ErrorKind) -> SyntaxError {
        let position = match &self.current {
            Some(Lookahead {
                token: Ok(token), ..
            }) => token.range.start,
            _ => self.lexer.position(),
        };
        SyntaxError { kind, position }
    }

    fn error_at(&self, kind: ErrorKind, position: usize) -> SyntaxError {
        SyntaxError { kind, position }
    }

    /// Ein Fehler beim aktuellen Token, oder dessen eigener Fehler.
    fn unexpected(&mut self, goal: Goal) -> SyntaxError {
        match self.token(goal) {
            Ok(_) => self.error(ErrorKind::UnexpectedToken),
            Err(error) => error,
        }
    }

    // Tiefe

    fn enter(&mut self) -> Result<()> {
        if self.depth >= MAX_DEPTH {
            return Err(self.error(ErrorKind::TooDeep));
        }
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // Baum

    fn node(&mut self, node: Node, start: usize) -> NodeId {
        let end = self.previous_end.max(start);
        self.ast.add(node, start..end)
    }

    fn node_at(&mut self, node: Node, start: usize, end: usize) -> NodeId {
        self.ast.add(node, start..end)
    }

    fn finish_list(&mut self, mark: usize) -> List {
        let list = self
            .ast
            .add_list(self.scratch.get(mark..).unwrap_or_default());
        self.scratch.truncate(mark);
        list
    }

    fn intern(&mut self, units: &[u16]) -> Atom {
        self.ast.intern(units)
    }

    fn intern_name(&mut self, name: &Name) -> Atom {
        self.ast.intern_str(&name.value)
    }

    fn atom_is(&self, atom: Atom, text: &str) -> bool {
        self.ast.atom(atom).iter().copied().eq(text.encode_utf16())
    }

    // Kontext

    fn context(&self) -> Context {
        self.function.context
    }

    fn strict(&self) -> bool {
        self.function.context.strict
    }

    /// Liest die Direktiven am Anfang eines Scripts oder Funktionsrumpfs
    /// (Abschnitt 11.2.1) als Anweisungen nach `self.scratch` und schaltet
    /// bei `"use strict"` in den strikten Modus. Gibt zurück, ob die
    /// Direktive vorkam; was sie für Name und Parameter einer Funktion
    /// bedeutet, prüft der Aufrufer.
    fn directives(&mut self) -> Result<bool> {
        let mut use_strict = false;
        let mut legacy: Option<usize> = None;
        loop {
            let token = self.token(Goal::RegExp)?;
            let TokenKind::String(string) = &token.kind else {
                break;
            };
            let range = token.range.clone();
            let legacy_escape = string.legacy_escape;
            let statement = self.statement_list_item()?;
            let Node::Expression(expression) = *self.ast.node(statement) else {
                self.scratch.push(statement);
                break;
            };
            if !matches!(self.ast.node(expression), Node::String(_))
                || self.ast.range(expression) != range
            {
                self.scratch.push(statement);
                break;
            }
            self.scratch.push(statement);
            if legacy_escape && legacy.is_none() {
                legacy = Some(range.start);
            }
            // Ohne die Anführungszeichen und genau so geschrieben, ohne
            // Escapes.
            let raw = self
                .source
                .get(range.start.saturating_add(1)..range.end.saturating_sub(1))
                .unwrap_or_default();
            if raw.iter().copied().eq("use strict".encode_utf16()) {
                use_strict = true;
                self.function.context.strict = true;
            }
        }
        if use_strict && let Some(position) = legacy {
            return Err(self.error_at(ErrorKind::StrictLegacyLiteral, position));
        }
        Ok(use_strict)
    }
}

/// Die reservierten Wörter (*ReservedWord*, Abschnitt 12.7.2) ohne `yield`
/// und `await`, die von der Stelle abhängen.
const RESERVED_WORDS: [&str; 36] = [
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
];

/// Wörter, die nur im strikten Modus reserviert sind (Abschnitt 13.1.1),
/// ohne `yield`.
const STRICT_RESERVED_WORDS: [&str; 8] = [
    "implements",
    "interface",
    "let",
    "package",
    "private",
    "protected",
    "public",
    "static",
];

fn is_reserved_word(name: &str) -> bool {
    RESERVED_WORDS.contains(&name)
}

fn is_strict_reserved_word(name: &str) -> bool {
    STRICT_RESERVED_WORDS.contains(&name) || name == "yield"
}

impl Parser<'_> {
    /// Wie [`is_strict_reserved_word`] für einen internierten Namen.
    fn atom_is_strict_reserved(&self, atom: Atom) -> bool {
        STRICT_RESERVED_WORDS
            .iter()
            .any(|word| self.atom_is(atom, word))
            || self.atom_is(atom, "yield")
    }
}
