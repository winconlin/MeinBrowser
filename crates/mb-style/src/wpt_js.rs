//! Ein kleiner JavaScript-Interpreter für die Skripte der Web Platform
//! Tests, nur für Tests.
//!
//! Er führt die Teilmenge von ECMAScript aus, die die Testdateien selbst
//! verwenden: `var`, `let` und `const`, Funktionen und Pfeilfunktionen mit
//! Closures, `if`, `for`, `for`-`of`, `while`, `try`, die üblichen
//! Operatoren, Listen, Objekte und Vorlagen (*template literals*). Was ein
//! Test braucht, etwa testharness.js oder `document`, stellt er über
//! [`Interpreter::define`] und [`Host`] bereit.
//!
//! Was der Interpreter nicht kann, meldet er als [`Control::Unsupported`],
//! nicht als Ausnahme des Skripts. So bleiben nicht ausführbare Tests von
//! fehlgeschlagenen getrennt. Zeichenketten sind wie in ECMAScript Folgen von
//! UTF-16-Codeeinheiten. Die Zahl der Schritte und die Verschachtelung sind
//! begrenzt (RS-8). Closures, die in ihrem eigenen Gültigkeitsbereich
//! stehen, bilden Zyklen und werden erst am Ende des Tests freigegeben.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

/// Eine Zeichenkette aus UTF-16-Codeeinheiten.
pub(crate) type JsString = Rc<[u16]>;

pub(crate) fn js_string(text: &str) -> JsString {
    text.encode_utf16().collect()
}

fn lossy(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

fn is(units: &[u16], text: &str) -> bool {
    units.iter().copied().eq(text.encode_utf16())
}

/// Eine Funktion des Wirts. Sie erhält `this` und die Argumente.
pub(crate) type NativeFunction =
    Rc<dyn Fn(&mut Interpreter, Value, Vec<Value>) -> Completion<Value>>;

/// Ein Wert von ECMAScript.
#[derive(Clone)]
pub(crate) enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(JsString),
    Object(Rc<Object>),
}

/// Ein Objekt mit seinen eigenen Eigenschaften in der Reihenfolge, in der
/// sie angelegt wurden.
pub(crate) struct Object {
    kind: Kind,
    properties: RefCell<Vec<(JsString, Value)>>,
}

enum Kind {
    Ordinary,
    Array(RefCell<Vec<Value>>),
    Closure {
        function: Rc<Function>,
        scope: Rc<Scope>,
    },
    Native(NativeFunction),
    RegExp {
        source: JsString,
        flags: Rc<str>,
    },
    Host(Rc<dyn Host>),
}

/// Ein Objekt des Wirts, etwa `document` oder eine `CSSStyleDeclaration`.
pub(crate) trait Host {
    /// Liest eine Eigenschaft. `None` heißt, der Wirt kennt sie nicht; dann
    /// gelten die eigenen Eigenschaften des Objekts.
    fn get(&self, key: &str) -> Completion<Option<Value>>;

    /// Setzt eine Eigenschaft. `false` heißt wie bei [`Host::get`], sie wird
    /// eine eigene Eigenschaft des Objekts.
    fn set(&self, key: &str, value: &Value) -> Completion<bool>;
}

impl Value {
    pub(crate) fn string(text: &str) -> Value {
        Value::String(js_string(text))
    }

    fn object(kind: Kind) -> Value {
        Value::Object(Rc::new(Object {
            kind,
            properties: RefCell::new(Vec::new()),
        }))
    }

    pub(crate) fn array(items: Vec<Value>) -> Value {
        Value::object(Kind::Array(RefCell::new(items)))
    }

    pub(crate) fn native(
        function: impl Fn(&mut Interpreter, Value, Vec<Value>) -> Completion<Value> + 'static,
    ) -> Value {
        Value::object(Kind::Native(Rc::new(function)))
    }

    pub(crate) fn host(host: Rc<dyn Host>) -> Value {
        Value::object(Kind::Host(host))
    }

    pub(crate) fn is_callable(&self) -> bool {
        matches!(self, Value::Object(object)
            if matches!(object.kind, Kind::Closure { .. } | Kind::Native(_)))
    }

    /// Die Elemente, falls der Wert eine Liste ist (`Array.isArray()`).
    pub(crate) fn array_items(&self) -> Option<Vec<Value>> {
        match self {
            Value::Object(object) => match &object.kind {
                Kind::Array(items) => Some(items.borrow().clone()),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Wie eine Anweisung oder ein Ausdruck endet, wenn nicht normal.
pub(crate) enum Control {
    Return(Value),
    Break,
    Continue,
    /// Eine Ausnahme des Skripts.
    Throw(Value),
    /// Etwas, das der Interpreter oder der Wirt nicht kann, mit Grund.
    Unsupported(String),
}

pub(crate) type Completion<T> = Result<T, Control>;

pub(crate) fn unsupported<T>(reason: impl Into<String>) -> Completion<T> {
    Err(Control::Unsupported(reason.into()))
}

/// Wirft eine Ausnahme mit einer Meldung als Zeichenkette.
pub(crate) fn throw<T>(message: &str) -> Completion<T> {
    Err(Control::Throw(Value::string(message)))
}

// ---------------------------------------------------------------------------
// Lexer

#[derive(Clone)]
struct Token {
    kind: TokenKind,
    /// Ob davor ein Zeilenende steht (für das automatische Einfügen von
    /// Semikolons).
    newline_before: bool,
    line: usize,
}

#[derive(Clone)]
enum TokenKind {
    Name(Rc<str>),
    Number(f64),
    String(JsString),
    /// Eine Vorlage: Texte und dazwischen die Tokens der Ersetzungen.
    Template {
        strings: Vec<JsString>,
        substitutions: Vec<Vec<Token>>,
    },
    RegExp {
        source: JsString,
        flags: Rc<str>,
    },
    Punctuator(&'static str),
    End,
}

/// Die Satzzeichen, längste zuerst.
const PUNCTUATORS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**",
    "<<", ">>", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%", "&",
    "|", "^", "!", "~", "?", ":", "=", ".",
];

/// Die größte Verschachtelung von Vorlagen.
const MAX_TEMPLATE_DEPTH: usize = 16;

/// *WhiteSpace* ohne Zeilenenden: die Leerzeichen der Kategorie Zs und
/// einige Steuerzeichen.
fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\u{B}' | '\u{C}' | ' ' | '\u{A0}' | '\u{FEFF}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_name_start(c: char) -> bool {
    c == '$' || c == '_' || c.is_ascii_alphabetic() || (!c.is_ascii() && c.is_alphabetic())
}

fn is_name_part(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || (!c.is_ascii() && c.is_alphanumeric())
}

/// Ob an dieser Stelle ein `/` einen regulären Ausdruck beginnt und keine
/// Division.
fn regexp_allowed(previous: Option<&Token>) -> bool {
    match previous.map(|token| &token.kind) {
        None => true,
        Some(TokenKind::Punctuator(punctuator)) => !matches!(*punctuator, ")" | "]" | "}"),
        Some(TokenKind::Name(name)) => matches!(
            &**name,
            "return"
                | "typeof"
                | "case"
                | "do"
                | "else"
                | "in"
                | "instanceof"
                | "new"
                | "delete"
                | "void"
                | "throw"
                | "of"
        ),
        Some(_) => false,
    }
}

struct Lexer {
    chars: Vec<char>,
    position: usize,
    line: usize,
    depth: usize,
}

impl Lexer {
    fn new(source: &str) -> Lexer {
        Lexer {
            chars: source.chars().collect(),
            position: 0,
            line: 1,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars
            .get(self.position.saturating_add(offset))
            .copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.position = self.position.saturating_add(1);
        if c == '\n' {
            self.line = self.line.saturating_add(1);
        }
        Some(c)
    }

    fn starts_with(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(offset, c)| self.peek_at(offset) == Some(c))
    }

    fn error<T>(&self, message: &str) -> Result<T, String> {
        Err(format!("Zeile {}: {message}", self.line))
    }

    /// Die Tokens bis zum Ende, oder in einer Vorlage bis zur schließenden
    /// geschweiften Klammer der Ersetzung.
    fn tokens(&mut self, in_template: bool) -> Result<Vec<Token>, String> {
        let mut tokens: Vec<Token> = Vec::new();
        let mut braces = 0usize;
        loop {
            let newline_before = self.skip()?;
            let line = self.line;
            let Some(c) = self.peek() else {
                if in_template {
                    return self.error("Vorlage endet nicht");
                }
                tokens.push(Token {
                    kind: TokenKind::End,
                    newline_before,
                    line,
                });
                return Ok(tokens);
            };
            let kind = if in_template && c == '}' && braces == 0 {
                self.bump();
                tokens.push(Token {
                    kind: TokenKind::End,
                    newline_before,
                    line,
                });
                return Ok(tokens);
            } else if c == '"' || c == '\'' {
                self.bump();
                TokenKind::String(self.string(c)?)
            } else if c == '`' {
                self.bump();
                self.template()?
            } else if c.is_ascii_digit()
                || (c == '.' && self.peek_at(1).is_some_and(|next| next.is_ascii_digit()))
            {
                TokenKind::Number(self.number()?)
            } else if is_name_start(c) {
                let mut name = String::new();
                while let Some(c) = self.peek().filter(|&c| is_name_part(c)) {
                    name.push(c);
                    self.bump();
                }
                TokenKind::Name(Rc::from(name))
            } else if c == '/'
                && !matches!(self.peek_at(1), Some('/' | '*'))
                && regexp_allowed(tokens.last())
            {
                self.bump();
                self.regexp()?
            } else {
                let Some(&punctuator) = PUNCTUATORS.iter().find(|p| self.starts_with(p)) else {
                    return self.error(&format!("unerwartetes Zeichen {c:?}"));
                };
                self.position = self.position.saturating_add(punctuator.len());
                match punctuator {
                    "{" => braces = braces.saturating_add(1),
                    "}" => braces = braces.saturating_sub(1),
                    _ => {}
                }
                TokenKind::Punctuator(punctuator)
            };
            tokens.push(Token {
                kind,
                newline_before,
                line,
            });
        }
    }

    /// Überspringt Leerraum und Kommentare und meldet, ob ein Zeilenende
    /// dabei war.
    fn skip(&mut self) -> Result<bool, String> {
        let mut newline = false;
        loop {
            match (self.peek(), self.peek_at(1)) {
                (Some(c), _) if is_whitespace(c) => {
                    self.bump();
                }
                (Some(c), _) if is_line_terminator(c) => {
                    newline = true;
                    self.bump();
                }
                (Some('/'), Some('/')) => {
                    while self.peek().is_some_and(|c| !is_line_terminator(c)) {
                        self.bump();
                    }
                }
                (Some('/'), Some('*')) => {
                    self.position = self.position.saturating_add(2);
                    while !matches!((self.peek(), self.peek_at(1)), (Some('*'), Some('/'))) {
                        match self.bump() {
                            Some(c) if is_line_terminator(c) => newline = true,
                            Some(_) => {}
                            None => return self.error("Kommentar endet nicht"),
                        }
                    }
                    self.position = self.position.saturating_add(2);
                }
                _ => return Ok(newline),
            }
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        let radix = match (self.peek(), self.peek_at(1)) {
            (Some('0'), Some('x' | 'X')) => Some(16),
            (Some('0'), Some('o' | 'O')) => Some(8),
            (Some('0'), Some('b' | 'B')) => Some(2),
            _ => None,
        };
        let value = if let Some(radix) = radix {
            self.position = self.position.saturating_add(2);
            let mut value = 0.0;
            let mut digits = 0usize;
            while let Some(digit) = self.peek().and_then(|c| c.to_digit(radix)) {
                value = value * f64::from(radix) + f64::from(digit);
                digits = digits.saturating_add(1);
                self.bump();
            }
            if digits == 0 {
                return self.error("Ziffer erwartet");
            }
            value
        } else {
            if self.peek() == Some('0') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                return self.error("Oktalzahlen alter Form werden nicht unterstützt");
            }
            let mut text = String::new();
            while let Some(c) = self.peek().filter(char::is_ascii_digit) {
                text.push(c);
                self.bump();
            }
            if self.peek() == Some('.') {
                text.push('.');
                self.bump();
                while let Some(c) = self.peek().filter(char::is_ascii_digit) {
                    text.push(c);
                    self.bump();
                }
            }
            if let Some(e @ ('e' | 'E')) = self.peek() {
                text.push(e);
                self.bump();
                if let Some(sign @ ('+' | '-')) = self.peek() {
                    text.push(sign);
                    self.bump();
                }
                let before = text.len();
                while let Some(c) = self.peek().filter(char::is_ascii_digit) {
                    text.push(c);
                    self.bump();
                }
                if text.len() == before {
                    return self.error("Exponent erwartet");
                }
            }
            match decimal_prefix(&text.encode_utf16().collect::<Vec<_>>()) {
                Some((value, length)) if length == text.len() => value,
                _ => return self.error("ungültige Zahl"),
            }
        };
        if self.peek().is_some_and(is_name_part) {
            return self.error("Name direkt nach einer Zahl");
        }
        Ok(value)
    }

    fn string(&mut self, quote: char) -> Result<JsString, String> {
        let mut units = Vec::new();
        loop {
            match self.bump() {
                None => return self.error("Zeichenkette endet nicht"),
                Some(c) if c == quote => return Ok(units.into()),
                Some('\\') => self.escape(&mut units, false)?,
                Some('\n' | '\r') => return self.error("Zeilenende in einer Zeichenkette"),
                Some(c) => units.extend(c.encode_utf16(&mut [0; 2]).iter()),
            }
        }
    }

    /// Ein Escape nach `\` in einer Zeichenkette oder Vorlage.
    fn escape(&mut self, units: &mut Vec<u16>, template: bool) -> Result<(), String> {
        let Some(escape) = self.bump() else {
            return self.error("Escape am Ende");
        };
        match escape {
            'n' => units.push(0x0A),
            't' => units.push(0x09),
            'r' => units.push(0x0D),
            'b' => units.push(0x08),
            'f' => units.push(0x0C),
            'v' => units.push(0x0B),
            '0' if !self.peek().is_some_and(|c| c.is_ascii_digit()) => units.push(0),
            '1'..='9' if template => return self.error("Oktal-Escape in einer Vorlage"),
            '0'..='9' => return self.error("Oktal-Escapes werden nicht unterstützt"),
            'x' => {
                let value = self.hex(2)?;
                units.push(self.unit(value)?);
            }
            'u' if self.peek() == Some('{') => {
                self.bump();
                let mut value: u32 = 0;
                while let Some(digit) = self.peek().and_then(|c| c.to_digit(16)) {
                    value = value.saturating_mul(16).saturating_add(digit);
                    self.bump();
                }
                if self.bump() != Some('}') {
                    return self.error("„}“ erwartet");
                }
                match char::from_u32(value) {
                    Some(c) => units.extend(c.encode_utf16(&mut [0; 2]).iter()),
                    None => units.push(self.unit(value)?),
                }
            }
            'u' => {
                let value = self.hex(4)?;
                units.push(self.unit(value)?);
            }
            // Fortsetzung über das Zeilenende
            '\r' => {
                if self.peek() == Some('\n') {
                    self.bump();
                }
            }
            '\n' | '\u{2028}' | '\u{2029}' => {}
            // *NonEscapeCharacter* und `\\`, `\'`, `\"`, `\``, `\$`
            other => units.extend(other.encode_utf16(&mut [0; 2]).iter()),
        }
        Ok(())
    }

    fn hex(&mut self, count: usize) -> Result<u32, String> {
        let mut value: u32 = 0;
        for _ in 0..count {
            let Some(digit) = self.bump().and_then(|c| c.to_digit(16)) else {
                return self.error("Hexadezimalziffer erwartet");
            };
            value = value.saturating_mul(16).saturating_add(digit);
        }
        Ok(value)
    }

    fn unit(&self, value: u32) -> Result<u16, String> {
        u16::try_from(value).or_else(|_| self.error("Codepunkt zu groß"))
    }

    fn template(&mut self) -> Result<TokenKind, String> {
        if self.depth >= MAX_TEMPLATE_DEPTH {
            return self.error("Vorlagen zu tief verschachtelt");
        }
        let mut strings = Vec::new();
        let mut substitutions = Vec::new();
        let mut current = Vec::new();
        loop {
            match self.bump() {
                None => return self.error("Vorlage endet nicht"),
                Some('`') => {
                    strings.push(current.into());
                    return Ok(TokenKind::Template {
                        strings,
                        substitutions,
                    });
                }
                Some('$') if self.peek() == Some('{') => {
                    self.bump();
                    strings.push(std::mem::take(&mut current).into());
                    self.depth = self.depth.saturating_add(1);
                    let tokens = self.tokens(true);
                    self.depth = self.depth.saturating_sub(1);
                    substitutions.push(tokens?);
                }
                Some('\\') => self.escape(&mut current, true)?,
                // Zeilenenden werden in Vorlagen zu U+000A.
                Some('\r') => {
                    if self.peek() == Some('\n') {
                        self.bump();
                    }
                    current.push(0x0A);
                }
                Some(c) => current.extend(c.encode_utf16(&mut [0; 2]).iter()),
            }
        }
    }

    fn regexp(&mut self) -> Result<TokenKind, String> {
        let mut source = String::new();
        let mut in_class = false;
        loop {
            match self.bump() {
                None => return self.error("regulärer Ausdruck endet nicht"),
                Some(c) if is_line_terminator(c) => {
                    return self.error("Zeilenende in einem regulären Ausdruck");
                }
                Some('\\') => {
                    source.push('\\');
                    match self.bump() {
                        Some(c) if !is_line_terminator(c) => source.push(c),
                        _ => return self.error("Escape am Ende eines regulären Ausdrucks"),
                    }
                }
                Some('/') if !in_class => break,
                Some(c) => {
                    match c {
                        '[' => in_class = true,
                        ']' => in_class = false,
                        _ => {}
                    }
                    source.push(c);
                }
            }
        }
        let mut flags = String::new();
        while let Some(c) = self.peek().filter(|&c| is_name_part(c)) {
            flags.push(c);
            self.bump();
        }
        Ok(TokenKind::RegExp {
            source: js_string(&source),
            flags: Rc::from(flags),
        })
    }
}

// ---------------------------------------------------------------------------
// Syntaxbaum

#[derive(Clone, Copy, PartialEq)]
enum DeclarationKind {
    Var,
    Let,
    Const,
}

enum Statement {
    Empty,
    Expression(Expression),
    Declaration(DeclarationKind, Vec<(Rc<str>, Option<Expression>)>),
    /// Eine Funktionsdeklaration; sie wird zu Beginn ihres Blocks angelegt.
    Function(Rc<Function>),
    If(Expression, Box<Statement>, Option<Box<Statement>>),
    Block(Vec<Statement>),
    For {
        init: Option<Box<Statement>>,
        test: Option<Expression>,
        update: Option<Expression>,
        body: Box<Statement>,
    },
    ForOf {
        kind: DeclarationKind,
        name: Rc<str>,
        iterable: Expression,
        body: Box<Statement>,
    },
    While(Expression, Box<Statement>),
    Return(Option<Expression>),
    Break,
    Continue,
    Throw(Expression),
    Try {
        block: Vec<Statement>,
        catch: Option<(Option<Rc<str>>, Vec<Statement>)>,
        finally: Option<Vec<Statement>>,
    },
}

#[derive(Clone, Copy)]
enum UnaryOperator {
    Not,
    Negate,
    Plus,
    Typeof,
}

#[derive(Clone, Copy)]
enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    Equal,
    NotEqual,
    StrictEqual,
    StrictNotEqual,
}

#[derive(Clone, Copy)]
enum LogicalOperator {
    And,
    Or,
    Coalesce,
}

#[derive(Clone, Copy)]
enum Operator {
    Binary(BinaryOperator),
    Logical(LogicalOperator),
}

/// Ein Element einer Liste oder ein Argument, eventuell mit `...`.
struct Element {
    spread: bool,
    value: Expression,
}

enum Expression {
    Number(f64),
    String(JsString),
    Template(Vec<JsString>, Vec<Expression>),
    RegExp(JsString, Rc<str>),
    Bool(bool),
    Null,
    Identifier(Rc<str>),
    Array(Vec<Element>),
    Object(Vec<(JsString, Expression)>),
    Function(Rc<Function>),
    Unary(UnaryOperator, Box<Expression>),
    Update {
        increment: bool,
        prefix: bool,
        target: Box<Expression>,
    },
    Binary(BinaryOperator, Box<Expression>, Box<Expression>),
    Logical(LogicalOperator, Box<Expression>, Box<Expression>),
    Conditional(Box<Expression>, Box<Expression>, Box<Expression>),
    /// Eine Zuweisung, bei `+=` und ähnlichen mit dem Operator.
    Assign(Option<BinaryOperator>, Box<Expression>, Box<Expression>),
    Sequence(Vec<Expression>),
    Member(Box<Expression>, JsString),
    Index(Box<Expression>, Box<Expression>),
    Call(Box<Expression>, Vec<Element>),
    New,
}

struct Function {
    name: Option<Rc<str>>,
    parameters: Vec<(Rc<str>, Option<Expression>)>,
    body: Body,
    strict: bool,
}

enum Body {
    Block(Vec<Statement>),
    /// Der Rumpf einer Pfeilfunktion ohne geschweifte Klammern.
    Expression(Expression),
}

struct Program {
    body: Vec<Statement>,
    strict: bool,
}

// ---------------------------------------------------------------------------
// Parser

type ParseResult<T> = Result<T, String>;

/// Die größte Verschachtelung von Anweisungen und Ausdrücken.
const MAX_PARSE_DEPTH: usize = 128;

const RESERVED: &[&str] = &[
    "await",
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
    "let",
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
    "yield",
];

fn parse_program(source: &str) -> ParseResult<Program> {
    let tokens = Lexer::new(source).tokens(false)?;
    let mut parser = Parser::new(tokens, 0, false, false);
    parser.strict = parser.directive();
    let mut body = Vec::new();
    while !parser.at_end() {
        body.push(parser.statement()?);
    }
    Ok(Program {
        body,
        strict: parser.strict,
    })
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    end: Token,
    depth: usize,
    strict: bool,
    in_function: bool,
}

impl Parser {
    fn new(tokens: Vec<Token>, depth: usize, strict: bool, in_function: bool) -> Parser {
        let line = tokens.last().map_or(0, |token| token.line);
        Parser {
            tokens,
            position: 0,
            end: Token {
                kind: TokenKind::End,
                newline_before: true,
                line,
            },
            depth,
            strict,
            in_function,
        }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&self.end)
    }

    fn kind_at(&self, position: usize) -> &TokenKind {
        &self.tokens.get(position).unwrap_or(&self.end).kind
    }

    fn advance(&mut self) -> Token {
        let token = self.peek().clone();
        self.position = self.position.saturating_add(1);
        token
    }

    fn at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::End)
    }

    fn is(&self, punctuator: &str) -> bool {
        matches!(self.peek().kind, TokenKind::Punctuator(found) if found == punctuator)
    }

    fn eat(&mut self, punctuator: &str) -> bool {
        let found = self.is(punctuator);
        if found {
            self.advance();
        }
        found
    }

    fn expect(&mut self, punctuator: &str) -> ParseResult<()> {
        if self.eat(punctuator) {
            Ok(())
        } else {
            self.error(&format!("„{punctuator}“ erwartet"))
        }
    }

    fn is_name(&self, word: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Name(name) if &**name == word)
    }

    fn eat_name(&mut self, word: &str) -> bool {
        let found = self.is_name(word);
        if found {
            self.advance();
        }
        found
    }

    fn error<T>(&self, message: &str) -> ParseResult<T> {
        Err(format!("Zeile {}: {message}", self.peek().line))
    }

    fn identifier(&mut self) -> ParseResult<Rc<str>> {
        match &self.peek().kind {
            TokenKind::Name(name) if !RESERVED.contains(&&**name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => self.error("Name erwartet"),
        }
    }

    /// Ein Semikolon, oder an seiner Stelle ein Zeilenende, `}` oder das
    /// Ende (*automatic semicolon insertion*).
    fn semicolon(&mut self) -> ParseResult<()> {
        if self.eat(";") || self.is("}") || self.at_end() || self.peek().newline_before {
            Ok(())
        } else {
            self.error("„;“ erwartet")
        }
    }

    /// Ob die Anweisungen mit der Direktive `"use strict"` beginnen.
    fn directive(&self) -> bool {
        matches!(&self.peek().kind, TokenKind::String(text) if is(text, "use strict"))
    }

    fn nested<T>(&mut self, parse: impl FnOnce(&mut Parser) -> ParseResult<T>) -> ParseResult<T> {
        if self.depth >= MAX_PARSE_DEPTH {
            return self.error("zu tief verschachtelt");
        }
        self.depth = self.depth.saturating_add(1);
        let result = parse(self);
        self.depth = self.depth.saturating_sub(1);
        result
    }

    fn statement(&mut self) -> ParseResult<Statement> {
        self.nested(Parser::statement_inner)
    }

    fn statement_inner(&mut self) -> ParseResult<Statement> {
        if self.is("{") {
            return Ok(Statement::Block(self.block()?));
        }
        if self.eat(";") {
            return Ok(Statement::Empty);
        }
        let TokenKind::Name(word) = &self.peek().kind else {
            let expression = self.expression()?;
            self.semicolon()?;
            return Ok(Statement::Expression(expression));
        };
        let word = word.clone();
        match &*word {
            "var" | "let" | "const" => {
                let statement = self.declaration()?;
                self.semicolon()?;
                Ok(statement)
            }
            "function" => {
                self.advance();
                let name = self.identifier()?;
                Ok(Statement::Function(self.function(Some(name))?))
            }
            "if" => {
                self.advance();
                self.expect("(")?;
                let test = self.expression()?;
                self.expect(")")?;
                let consequent = Box::new(self.statement()?);
                let alternate = if self.eat_name("else") {
                    Some(Box::new(self.statement()?))
                } else {
                    None
                };
                Ok(Statement::If(test, consequent, alternate))
            }
            "for" => {
                self.advance();
                self.for_statement()
            }
            "while" => {
                self.advance();
                self.expect("(")?;
                let test = self.expression()?;
                self.expect(")")?;
                Ok(Statement::While(test, Box::new(self.statement()?)))
            }
            "return" => {
                if !self.in_function {
                    return self.error("„return“ außerhalb einer Funktion");
                }
                self.advance();
                let value = if self.is(";")
                    || self.is("}")
                    || self.at_end()
                    || self.peek().newline_before
                {
                    None
                } else {
                    Some(self.expression()?)
                };
                self.semicolon()?;
                Ok(Statement::Return(value))
            }
            "break" | "continue" => {
                self.advance();
                if matches!(self.peek().kind, TokenKind::Name(_)) && !self.peek().newline_before {
                    return self.error("Sprungmarken werden nicht unterstützt");
                }
                self.semicolon()?;
                Ok(if &*word == "break" {
                    Statement::Break
                } else {
                    Statement::Continue
                })
            }
            "throw" => {
                self.advance();
                if self.peek().newline_before {
                    return self.error("Zeilenende nach „throw“");
                }
                let value = self.expression()?;
                self.semicolon()?;
                Ok(Statement::Throw(value))
            }
            "try" => {
                self.advance();
                self.try_statement()
            }
            "class" | "switch" | "do" | "with" | "async" | "import" | "export" | "yield"
            | "await" | "debugger" => self.error(&format!("„{word}“ wird nicht unterstützt")),
            _ => {
                if matches!(
                    self.kind_at(self.position.saturating_add(1)),
                    TokenKind::Punctuator(":")
                ) {
                    return self.error("Sprungmarken werden nicht unterstützt");
                }
                let expression = self.expression()?;
                self.semicolon()?;
                Ok(Statement::Expression(expression))
            }
        }
    }

    fn block(&mut self) -> ParseResult<Vec<Statement>> {
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.eat("}") {
            if self.at_end() {
                return self.error("„}“ erwartet");
            }
            body.push(self.statement()?);
        }
        Ok(body)
    }

    fn declaration_kind(&mut self) -> ParseResult<DeclarationKind> {
        let kind = if self.eat_name("var") {
            DeclarationKind::Var
        } else if self.eat_name("let") {
            DeclarationKind::Let
        } else if self.eat_name("const") {
            DeclarationKind::Const
        } else {
            return self.error("Deklaration erwartet");
        };
        if self.is("[") || self.is("{") {
            return self.error("Destrukturierung wird nicht unterstützt");
        }
        Ok(kind)
    }

    fn declaration(&mut self) -> ParseResult<Statement> {
        let kind = self.declaration_kind()?;
        let name = self.identifier()?;
        self.declarators(kind, name)
    }

    /// Der Rest einer Deklaration nach dem ersten Namen.
    fn declarators(&mut self, kind: DeclarationKind, first: Rc<str>) -> ParseResult<Statement> {
        let mut declarations = Vec::new();
        let mut name = first;
        loop {
            let init = if self.eat("=") {
                Some(self.assignment()?)
            } else if kind == DeclarationKind::Const {
                return self.error("„const“ ohne Wert");
            } else {
                None
            };
            declarations.push((name, init));
            if !self.eat(",") {
                return Ok(Statement::Declaration(kind, declarations));
            }
            name = self.identifier()?;
        }
    }

    fn for_statement(&mut self) -> ParseResult<Statement> {
        self.expect("(")?;
        let init = if self.is_name("var") || self.is_name("let") || self.is_name("const") {
            let kind = self.declaration_kind()?;
            let name = self.identifier()?;
            if self.eat_name("of") {
                let iterable = self.assignment()?;
                self.expect(")")?;
                return Ok(Statement::ForOf {
                    kind,
                    name,
                    iterable,
                    body: Box::new(self.statement()?),
                });
            }
            if self.is_name("in") {
                return self.error("„for-in“ wird nicht unterstützt");
            }
            Some(Box::new(self.declarators(kind, name)?))
        } else if self.is(";") {
            None
        } else {
            let expression = self.expression()?;
            if self.is_name("of") || self.is_name("in") {
                return self.error("Schleifen über vorhandene Variablen werden nicht unterstützt");
            }
            Some(Box::new(Statement::Expression(expression)))
        };
        self.expect(";")?;
        let test = if self.is(";") {
            None
        } else {
            Some(self.expression()?)
        };
        self.expect(";")?;
        let update = if self.is(")") {
            None
        } else {
            Some(self.expression()?)
        };
        self.expect(")")?;
        Ok(Statement::For {
            init,
            test,
            update,
            body: Box::new(self.statement()?),
        })
    }

    fn try_statement(&mut self) -> ParseResult<Statement> {
        let block = self.block()?;
        let catch = if self.eat_name("catch") {
            let parameter = if self.eat("(") {
                let name = self.identifier()?;
                self.expect(")")?;
                Some(name)
            } else {
                None
            };
            Some((parameter, self.block()?))
        } else {
            None
        };
        let finally = if self.eat_name("finally") {
            Some(self.block()?)
        } else {
            None
        };
        if catch.is_none() && finally.is_none() {
            return self.error("„catch“ oder „finally“ erwartet");
        }
        Ok(Statement::Try {
            block,
            catch,
            finally,
        })
    }

    fn parameters(&mut self) -> ParseResult<Vec<(Rc<str>, Option<Expression>)>> {
        self.expect("(")?;
        let mut parameters = Vec::new();
        while !self.eat(")") {
            if self.is("...") || self.is("[") || self.is("{") {
                return self.error("Rest- und Destrukturierungsparameter werden nicht unterstützt");
            }
            let name = self.identifier()?;
            let default = if self.eat("=") {
                Some(self.assignment()?)
            } else {
                None
            };
            parameters.push((name, default));
            if !self.eat(",") {
                self.expect(")")?;
                break;
            }
        }
        Ok(parameters)
    }

    /// Ein Funktionsrumpf in geschweiften Klammern; liefert auch, ob er
    /// *strict* ist.
    fn function_body(&mut self) -> ParseResult<(Vec<Statement>, bool)> {
        self.expect("{")?;
        let outer = (self.strict, self.in_function);
        if self.directive() {
            self.strict = true;
        }
        self.in_function = true;
        let strict = self.strict;
        let mut body = Vec::new();
        while !self.eat("}") {
            if self.at_end() {
                return self.error("„}“ erwartet");
            }
            body.push(self.statement()?);
        }
        (self.strict, self.in_function) = outer;
        Ok((body, strict))
    }

    fn function(&mut self, name: Option<Rc<str>>) -> ParseResult<Rc<Function>> {
        let parameters = self.parameters()?;
        let (body, strict) = self.function_body()?;
        Ok(Rc::new(Function {
            name,
            parameters,
            body: Body::Block(body),
            strict,
        }))
    }

    /// Ob hier eine Pfeilfunktion beginnt.
    fn arrow_ahead(&self) -> bool {
        match &self.peek().kind {
            TokenKind::Name(name) if !RESERVED.contains(&&**name) => matches!(
                self.kind_at(self.position.saturating_add(1)),
                TokenKind::Punctuator("=>")
            ),
            TokenKind::Punctuator("(") => {
                let mut depth = 0usize;
                let mut position = self.position;
                loop {
                    match self.kind_at(position) {
                        TokenKind::Punctuator("(" | "[" | "{") => depth = depth.saturating_add(1),
                        TokenKind::Punctuator(")" | "]" | "}") => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                return matches!(
                                    self.kind_at(position.saturating_add(1)),
                                    TokenKind::Punctuator("=>")
                                );
                            }
                        }
                        TokenKind::End => return false,
                        _ => {}
                    }
                    position = position.saturating_add(1);
                }
            }
            _ => false,
        }
    }

    fn arrow(&mut self) -> ParseResult<Expression> {
        let parameters = if self.is("(") {
            self.parameters()?
        } else {
            vec![(self.identifier()?, None)]
        };
        self.expect("=>")?;
        let function = if self.is("{") {
            let (body, strict) = self.function_body()?;
            Function {
                name: None,
                parameters,
                body: Body::Block(body),
                strict,
            }
        } else {
            Function {
                name: None,
                parameters,
                body: Body::Expression(self.assignment()?),
                strict: self.strict,
            }
        };
        Ok(Expression::Function(Rc::new(function)))
    }

    fn expression(&mut self) -> ParseResult<Expression> {
        let first = self.assignment()?;
        if !self.is(",") {
            return Ok(first);
        }
        let mut list = vec![first];
        while self.eat(",") {
            list.push(self.assignment()?);
        }
        Ok(Expression::Sequence(list))
    }

    fn assignment(&mut self) -> ParseResult<Expression> {
        self.nested(Parser::assignment_inner)
    }

    fn assignment_inner(&mut self) -> ParseResult<Expression> {
        if self.arrow_ahead() {
            return self.arrow();
        }
        let target = self.conditional()?;
        let operator = match &self.peek().kind {
            TokenKind::Punctuator("=") => None,
            TokenKind::Punctuator("+=") => Some(BinaryOperator::Add),
            TokenKind::Punctuator("-=") => Some(BinaryOperator::Subtract),
            TokenKind::Punctuator("*=") => Some(BinaryOperator::Multiply),
            TokenKind::Punctuator("/=") => Some(BinaryOperator::Divide),
            TokenKind::Punctuator("%=") => Some(BinaryOperator::Remainder),
            TokenKind::Punctuator(
                "**=" | "<<=" | ">>=" | ">>>=" | "&=" | "|=" | "^=" | "&&=" | "||=" | "??=",
            ) => return self.error("dieser Zuweisungsoperator wird nicht unterstützt"),
            _ => return Ok(target),
        };
        if !matches!(
            target,
            Expression::Identifier(_) | Expression::Member(..) | Expression::Index(..)
        ) {
            return self.error("ungültiges Ziel einer Zuweisung");
        }
        self.advance();
        let value = self.assignment()?;
        Ok(Expression::Assign(
            operator,
            Box::new(target),
            Box::new(value),
        ))
    }

    fn conditional(&mut self) -> ParseResult<Expression> {
        let test = self.binary(0)?;
        if !self.eat("?") {
            return Ok(test);
        }
        let consequent = self.assignment()?;
        self.expect(":")?;
        let alternate = self.assignment()?;
        Ok(Expression::Conditional(
            Box::new(test),
            Box::new(consequent),
            Box::new(alternate),
        ))
    }

    /// Der Operator am aktuellen Token mit seinem Vorrang.
    fn operator(&self) -> ParseResult<Option<(Operator, u8)>> {
        use BinaryOperator as B;
        use Operator::{Binary, Logical};
        let operator = match &self.peek().kind {
            TokenKind::Punctuator(punctuator) => match *punctuator {
                "??" => (Logical(LogicalOperator::Coalesce), 1),
                "||" => (Logical(LogicalOperator::Or), 2),
                "&&" => (Logical(LogicalOperator::And), 3),
                "==" => (Binary(B::Equal), 7),
                "!=" => (Binary(B::NotEqual), 7),
                "===" => (Binary(B::StrictEqual), 7),
                "!==" => (Binary(B::StrictNotEqual), 7),
                "<" => (Binary(B::Less), 8),
                ">" => (Binary(B::Greater), 8),
                "<=" => (Binary(B::LessEqual), 8),
                ">=" => (Binary(B::GreaterEqual), 8),
                "+" => (Binary(B::Add), 10),
                "-" => (Binary(B::Subtract), 10),
                "*" => (Binary(B::Multiply), 11),
                "/" => (Binary(B::Divide), 11),
                "%" => (Binary(B::Remainder), 11),
                "|" | "^" | "&" | "<<" | ">>" | ">>>" | "**" => {
                    return self.error(&format!("Operator „{punctuator}“ wird nicht unterstützt"));
                }
                _ => return Ok(None),
            },
            TokenKind::Name(name) if matches!(&**name, "in" | "instanceof") => {
                return self.error(&format!("Operator „{name}“ wird nicht unterstützt"));
            }
            _ => return Ok(None),
        };
        Ok(Some(operator))
    }

    fn binary(&mut self, minimum: u8) -> ParseResult<Expression> {
        let mut left = self.unary()?;
        while let Some((operator, precedence)) = self.operator()? {
            if precedence < minimum {
                break;
            }
            self.advance();
            let right = self.binary(precedence.saturating_add(1))?;
            left = match operator {
                Operator::Binary(operator) => {
                    Expression::Binary(operator, Box::new(left), Box::new(right))
                }
                Operator::Logical(operator) => {
                    Expression::Logical(operator, Box::new(left), Box::new(right))
                }
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> ParseResult<Expression> {
        self.nested(Parser::unary_inner)
    }

    fn unary_inner(&mut self) -> ParseResult<Expression> {
        let operator = match &self.peek().kind {
            TokenKind::Punctuator("!") => Some(UnaryOperator::Not),
            TokenKind::Punctuator("-") => Some(UnaryOperator::Negate),
            TokenKind::Punctuator("+") => Some(UnaryOperator::Plus),
            TokenKind::Name(name) if &**name == "typeof" => Some(UnaryOperator::Typeof),
            TokenKind::Punctuator(increment @ ("++" | "--")) => {
                let increment = *increment == "++";
                self.advance();
                let target = self.unary()?;
                return self.update(increment, true, target);
            }
            TokenKind::Punctuator("~") => return self.error("Operator „~“ wird nicht unterstützt"),
            TokenKind::Name(name) if matches!(&**name, "void" | "delete" | "await") => {
                return self.error(&format!("„{name}“ wird nicht unterstützt"));
            }
            _ => None,
        };
        if let Some(operator) = operator {
            self.advance();
            return Ok(Expression::Unary(operator, Box::new(self.unary()?)));
        }
        let expression = self.call_member()?;
        let postfix = match &self.peek().kind {
            TokenKind::Punctuator("++") => Some(true),
            TokenKind::Punctuator("--") => Some(false),
            _ => None,
        };
        match postfix {
            Some(increment) if !self.peek().newline_before => {
                self.advance();
                self.update(increment, false, expression)
            }
            _ => Ok(expression),
        }
    }

    fn update(&self, increment: bool, prefix: bool, target: Expression) -> ParseResult<Expression> {
        if !matches!(
            target,
            Expression::Identifier(_) | Expression::Member(..) | Expression::Index(..)
        ) {
            return self.error("ungültiges Ziel von „++“ oder „--“");
        }
        Ok(Expression::Update {
            increment,
            prefix,
            target: Box::new(target),
        })
    }

    fn call_member(&mut self) -> ParseResult<Expression> {
        let mut expression = if self.eat_name("new") {
            // `new` gibt es noch nicht; der Ausdruck wird nur übersprungen.
            self.primary()?;
            while self.eat(".") {
                self.advance();
            }
            if self.is("(") {
                self.arguments()?;
            }
            Expression::New
        } else {
            self.primary()?
        };
        loop {
            if self.eat(".") {
                let TokenKind::Name(name) = self.advance().kind else {
                    return self.error("Name nach „.“ erwartet");
                };
                expression = Expression::Member(Box::new(expression), js_string(&name));
            } else if self.eat("[") {
                let key = self.expression()?;
                self.expect("]")?;
                expression = Expression::Index(Box::new(expression), Box::new(key));
            } else if self.is("(") {
                let arguments = self.arguments()?;
                expression = Expression::Call(Box::new(expression), arguments);
            } else if matches!(self.peek().kind, TokenKind::Template { .. }) {
                return self.error("getaggte Vorlagen werden nicht unterstützt");
            } else {
                return Ok(expression);
            }
        }
    }

    /// Elemente bis zum schließenden Zeichen, mit `...` und einem Komma am
    /// Ende.
    fn elements(&mut self, close: &str) -> ParseResult<Vec<Element>> {
        let mut elements = Vec::new();
        while !self.eat(close) {
            if self.is(",") {
                return self.error("Lücken in Listen werden nicht unterstützt");
            }
            let spread = self.eat("...");
            elements.push(Element {
                spread,
                value: self.assignment()?,
            });
            if !self.eat(",") {
                self.expect(close)?;
                break;
            }
        }
        Ok(elements)
    }

    fn arguments(&mut self) -> ParseResult<Vec<Element>> {
        self.expect("(")?;
        self.elements(")")
    }

    fn primary(&mut self) -> ParseResult<Expression> {
        let token = self.advance();
        match token.kind {
            TokenKind::Number(value) => Ok(Expression::Number(value)),
            TokenKind::String(text) => Ok(Expression::String(text)),
            TokenKind::RegExp { source, flags } => Ok(Expression::RegExp(source, flags)),
            TokenKind::Template {
                strings,
                substitutions,
            } => {
                let mut expressions = Vec::new();
                for tokens in substitutions {
                    let mut parser = Parser::new(tokens, self.depth, self.strict, self.in_function);
                    expressions.push(parser.expression()?);
                    if !parser.at_end() {
                        return parser.error("„}“ erwartet");
                    }
                }
                Ok(Expression::Template(strings, expressions))
            }
            TokenKind::Name(name) => match &*name {
                "true" => Ok(Expression::Bool(true)),
                "false" => Ok(Expression::Bool(false)),
                "null" => Ok(Expression::Null),
                "function" => {
                    let name = if matches!(self.peek().kind, TokenKind::Name(_)) {
                        Some(self.identifier()?)
                    } else {
                        None
                    };
                    Ok(Expression::Function(self.function(name)?))
                }
                _ if RESERVED.contains(&&*name) => {
                    self.position = self.position.saturating_sub(1);
                    self.error(&format!("„{name}“ wird hier nicht unterstützt"))
                }
                _ => Ok(Expression::Identifier(name)),
            },
            TokenKind::Punctuator("(") => {
                let expression = self.expression()?;
                self.expect(")")?;
                Ok(expression)
            }
            TokenKind::Punctuator("[") => Ok(Expression::Array(self.elements("]")?)),
            TokenKind::Punctuator("{") => self.object(),
            _ => {
                self.position = self.position.saturating_sub(1);
                self.error("Ausdruck erwartet")
            }
        }
    }

    fn object(&mut self) -> ParseResult<Expression> {
        let mut properties = Vec::new();
        while !self.eat("}") {
            let token = self.advance();
            let key = match &token.kind {
                TokenKind::Name(name) => {
                    if self.is(",") || self.is("}") {
                        if RESERVED.contains(&&**name) {
                            return self.error("Name erwartet");
                        }
                        properties.push((js_string(name), Expression::Identifier(name.clone())));
                        if !self.eat(",") {
                            self.expect("}")?;
                            break;
                        }
                        continue;
                    }
                    js_string(name)
                }
                TokenKind::String(text) => text.clone(),
                TokenKind::Number(value) => js_string(&number_to_string(*value)),
                _ => return self.error("Name einer Eigenschaft erwartet"),
            };
            if !self.eat(":") {
                return self.error("„:“ erwartet (Methoden und berechnete Namen fehlen)");
            }
            properties.push((key, self.assignment()?));
            if !self.eat(",") {
                self.expect("}")?;
                break;
            }
        }
        Ok(Expression::Object(properties))
    }
}

// ---------------------------------------------------------------------------
// Gültigkeitsbereiche

struct Binding {
    name: Rc<str>,
    value: Value,
    mutable: bool,
}

struct Scope {
    bindings: RefCell<Vec<Binding>>,
    parent: Option<Rc<Scope>>,
}

enum Assigned {
    Done,
    Missing,
    Constant,
}

impl Scope {
    fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        Rc::new(Scope {
            bindings: RefCell::new(Vec::new()),
            parent,
        })
    }

    /// Eine Kopie für die nächste Runde einer Schleife mit `let`
    /// (*CreatePerIterationEnvironment*).
    fn copy(&self) -> Rc<Scope> {
        let bindings = self
            .bindings
            .borrow()
            .iter()
            .map(|binding| Binding {
                name: binding.name.clone(),
                value: binding.value.clone(),
                mutable: binding.mutable,
            })
            .collect();
        Rc::new(Scope {
            bindings: RefCell::new(bindings),
            parent: self.parent.clone(),
        })
    }

    fn declare(&self, name: &Rc<str>, value: Value, mutable: bool) {
        let mut bindings = self.bindings.borrow_mut();
        match bindings.iter_mut().find(|binding| binding.name == *name) {
            Some(binding) => {
                binding.value = value;
                binding.mutable = mutable;
            }
            None => bindings.push(Binding {
                name: name.clone(),
                value,
                mutable,
            }),
        }
    }

    /// Legt eine Variable aus `var` an, falls es sie hier noch nicht gibt.
    fn declare_var(&self, name: &Rc<str>) {
        let mut bindings = self.bindings.borrow_mut();
        if !bindings.iter().any(|binding| binding.name == *name) {
            bindings.push(Binding {
                name: name.clone(),
                value: Value::Undefined,
                mutable: true,
            });
        }
    }
}

fn lookup(scope: &Rc<Scope>, name: &str) -> Option<Value> {
    let mut current = Some(scope.clone());
    while let Some(scope) = current {
        if let Some(binding) = scope
            .bindings
            .borrow()
            .iter()
            .find(|binding| &*binding.name == name)
        {
            return Some(binding.value.clone());
        }
        current = scope.parent.clone();
    }
    None
}

fn assign_binding(scope: &Rc<Scope>, name: &str, value: Value) -> Assigned {
    let mut current = Some(scope.clone());
    while let Some(scope) = current {
        if let Some(binding) = scope
            .bindings
            .borrow_mut()
            .iter_mut()
            .find(|binding| &*binding.name == name)
        {
            if !binding.mutable {
                return Assigned::Constant;
            }
            binding.value = value;
            return Assigned::Done;
        }
        current = scope.parent.clone();
    }
    Assigned::Missing
}

/// Die Namen aus `var` in einem Funktionsrumpf, ohne innere Funktionen.
/// Die Tiefe ist durch [`MAX_PARSE_DEPTH`] begrenzt.
fn var_names(statement: &Statement, names: &mut Vec<Rc<str>>) {
    match statement {
        Statement::Declaration(DeclarationKind::Var, declarations) => {
            names.extend(declarations.iter().map(|(name, _)| name.clone()));
        }
        Statement::ForOf {
            kind: DeclarationKind::Var,
            name,
            body,
            ..
        } => {
            names.push(name.clone());
            var_names(body, names);
        }
        Statement::ForOf { body, .. } | Statement::While(_, body) => var_names(body, names),
        Statement::For { init, body, .. } => {
            if let Some(init) = init {
                var_names(init, names);
            }
            var_names(body, names);
        }
        Statement::If(_, consequent, alternate) => {
            var_names(consequent, names);
            if let Some(alternate) = alternate {
                var_names(alternate, names);
            }
        }
        Statement::Block(body) => body
            .iter()
            .for_each(|statement| var_names(statement, names)),
        Statement::Try {
            block,
            catch,
            finally,
        } => {
            let catch = catch.iter().flat_map(|(_, body)| body);
            for statement in block.iter().chain(catch).chain(finally.iter().flatten()) {
                var_names(statement, names);
            }
        }
        Statement::Empty
        | Statement::Expression(_)
        | Statement::Declaration(..)
        | Statement::Function(_)
        | Statement::Return(_)
        | Statement::Break
        | Statement::Continue
        | Statement::Throw(_) => {}
    }
}

fn hoist_vars(body: &[Statement], scope: &Scope) {
    let mut names = Vec::new();
    for statement in body {
        var_names(statement, &mut names);
    }
    for name in &names {
        scope.declare_var(name);
    }
}

fn closure(function: &Rc<Function>, scope: &Rc<Scope>) -> Value {
    Value::object(Kind::Closure {
        function: function.clone(),
        scope: scope.clone(),
    })
}

// ---------------------------------------------------------------------------
// Ausführung

/// Die größte Zahl von Schritten eines Interpreters.
const MAX_STEPS: usize = 20_000_000;
/// Die größte Verschachtelung beim Ausführen, Aufrufe eingeschlossen.
const MAX_DEPTH: usize = 512;

/// Was beim Ausführen eines Skripts geschah.
#[derive(Default)]
pub(crate) struct ScriptResult {
    /// Die Gründe für Anweisungen auf oberster Ebene, die übersprungen
    /// wurden, weil etwas fehlt.
    pub(crate) unsupported: Vec<String>,
    /// Eine Ausnahme, die das Skript beendet hat.
    pub(crate) exception: Option<String>,
}

enum Reference {
    Variable(Rc<str>),
    Property(Value, JsString),
}

pub(crate) struct Interpreter {
    global: Rc<Scope>,
    steps: usize,
    depth: usize,
    strict: bool,
}

impl Interpreter {
    pub(crate) fn new() -> Interpreter {
        let global = Scope::new(None);
        for (name, value) in [
            ("undefined", Value::Undefined),
            ("NaN", Value::Number(f64::NAN)),
            ("Infinity", Value::Number(f64::INFINITY)),
        ] {
            global.declare(&Rc::from(name), value, false);
        }
        Interpreter {
            global,
            steps: 0,
            depth: 0,
            strict: false,
        }
    }

    /// Legt eine globale Variable an.
    pub(crate) fn define(&mut self, name: &str, value: Value) {
        self.global.declare(&Rc::from(name), value, true);
    }

    /// Führt ein Skript aus. Ein Syntaxfehler, auch für nicht unterstützte
    /// Syntax, liefert `Err`.
    pub(crate) fn run_script(&mut self, source: &str) -> Result<ScriptResult, String> {
        let program = parse_program(source)?;
        self.strict = program.strict;
        let global = self.global.clone();
        hoist_vars(&program.body, &global);
        hoist_functions(&program.body, &global);
        let mut result = ScriptResult::default();
        for statement in &program.body {
            match self.exec(statement, &global) {
                Ok(()) => {}
                Err(Control::Unsupported(reason)) => result.unsupported.push(reason),
                Err(Control::Throw(value)) => {
                    result.exception = Some(describe(&value));
                    break;
                }
                Err(Control::Return(_) | Control::Break | Control::Continue) => {
                    result.exception = Some("Sprung außerhalb einer Funktion".to_owned());
                    break;
                }
            }
        }
        Ok(result)
    }

    fn enter(&mut self) -> Completion<()> {
        self.steps = self.steps.saturating_add(1);
        if self.steps > MAX_STEPS {
            return unsupported("Schrittgrenze erreicht");
        }
        if self.depth >= MAX_DEPTH {
            return throw("RangeError: Maximum call stack size exceeded");
        }
        self.depth = self.depth.saturating_add(1);
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn exec(&mut self, statement: &Statement, scope: &Rc<Scope>) -> Completion<()> {
        self.enter()?;
        let result = self.exec_inner(statement, scope);
        self.leave();
        result
    }

    fn exec_list(&mut self, body: &[Statement], scope: &Rc<Scope>) -> Completion<()> {
        hoist_functions(body, scope);
        for statement in body {
            self.exec(statement, scope)?;
        }
        Ok(())
    }

    fn exec_block(&mut self, body: &[Statement], scope: &Rc<Scope>) -> Completion<()> {
        let inner = Scope::new(Some(scope.clone()));
        self.exec_list(body, &inner)
    }

    fn exec_inner(&mut self, statement: &Statement, scope: &Rc<Scope>) -> Completion<()> {
        match statement {
            Statement::Empty | Statement::Function(_) => Ok(()),
            Statement::Expression(expression) => self.eval(expression, scope).map(drop),
            Statement::Declaration(kind, declarations) => {
                for (name, init) in declarations {
                    if *kind == DeclarationKind::Var {
                        if let Some(init) = init {
                            let value = self.eval(init, scope)?;
                            self.assign(name, value, scope)?;
                        }
                    } else {
                        let value = match init {
                            Some(init) => self.eval(init, scope)?,
                            None => Value::Undefined,
                        };
                        scope.declare(name, value, *kind == DeclarationKind::Let);
                    }
                }
                Ok(())
            }
            Statement::If(test, consequent, alternate) => {
                if is_truthy(&self.eval(test, scope)?) {
                    self.exec(consequent, scope)
                } else if let Some(alternate) = alternate {
                    self.exec(alternate, scope)
                } else {
                    Ok(())
                }
            }
            Statement::Block(body) => self.exec_block(body, scope),
            Statement::While(test, body) => loop {
                if !is_truthy(&self.eval(test, scope)?) {
                    return Ok(());
                }
                match self.exec(body, scope) {
                    Ok(()) | Err(Control::Continue) => {}
                    Err(Control::Break) => return Ok(()),
                    Err(other) => return Err(other),
                }
            },
            Statement::For {
                init,
                test,
                update,
                body,
            } => self.exec_for(init.as_deref(), test.as_ref(), update.as_ref(), body, scope),
            Statement::ForOf {
                kind,
                name,
                iterable,
                body,
            } => self.exec_for_of(*kind, name, iterable, body, scope),
            Statement::Return(value) => {
                let value = match value {
                    Some(value) => self.eval(value, scope)?,
                    None => Value::Undefined,
                };
                Err(Control::Return(value))
            }
            Statement::Break => Err(Control::Break),
            Statement::Continue => Err(Control::Continue),
            Statement::Throw(value) => {
                let value = self.eval(value, scope)?;
                Err(Control::Throw(value))
            }
            Statement::Try {
                block,
                catch,
                finally,
            } => {
                let mut result = self.exec_block(block, scope);
                let caught = match (&result, catch) {
                    (Err(Control::Throw(exception)), Some(catch)) => {
                        Some((exception.clone(), catch))
                    }
                    _ => None,
                };
                if let Some((exception, (parameter, handler))) = caught {
                    let inner = Scope::new(Some(scope.clone()));
                    if let Some(parameter) = parameter {
                        inner.declare(parameter, exception, true);
                    }
                    result = self.exec_list(handler, &inner);
                }
                if let Some(finally) = finally {
                    self.exec_block(finally, scope)?;
                }
                result
            }
        }
    }

    fn exec_for(
        &mut self,
        init: Option<&Statement>,
        test: Option<&Expression>,
        update: Option<&Expression>,
        body: &Statement,
        scope: &Rc<Scope>,
    ) -> Completion<()> {
        let mut iteration = Scope::new(Some(scope.clone()));
        if let Some(init) = init {
            self.exec(init, &iteration)?;
        }
        let per_iteration = matches!(init, Some(Statement::Declaration(DeclarationKind::Let, _)));
        if per_iteration {
            iteration = iteration.copy();
        }
        loop {
            if let Some(test) = test
                && !is_truthy(&self.eval(test, &iteration)?)
            {
                return Ok(());
            }
            match self.exec(body, &iteration) {
                Ok(()) | Err(Control::Continue) => {}
                Err(Control::Break) => return Ok(()),
                Err(other) => return Err(other),
            }
            if per_iteration {
                iteration = iteration.copy();
            }
            if let Some(update) = update {
                self.eval(update, &iteration)?;
            }
        }
    }

    fn exec_for_of(
        &mut self,
        kind: DeclarationKind,
        name: &Rc<str>,
        iterable: &Expression,
        body: &Statement,
        scope: &Rc<Scope>,
    ) -> Completion<()> {
        let iterable = self.eval(iterable, scope)?;
        // Zeichenketten durchläuft die Schleife nach Codepunkten, Listen
        // „lebendig“: Was der Rumpf anhängt, kommt auch noch dran.
        let code_points = match &iterable {
            Value::String(units) => Some(code_points(units)),
            _ => None,
        };
        let mut index = 0usize;
        loop {
            let item = match &code_points {
                Some(items) => items.get(index).cloned(),
                None => match &iterable {
                    Value::Object(object) => match &object.kind {
                        Kind::Array(items) => items.borrow().get(index).cloned(),
                        _ => return unsupported("for-of über ein Objekt, das keine Liste ist"),
                    },
                    _ => return throw("TypeError: Wert ist nicht iterierbar"),
                },
            };
            let Some(item) = item else {
                return Ok(());
            };
            index = index.saturating_add(1);
            let iteration = Scope::new(Some(scope.clone()));
            if kind == DeclarationKind::Var {
                self.assign(name, item, scope)?;
            } else {
                iteration.declare(name, item, kind == DeclarationKind::Let);
            }
            match self.exec(body, &iteration) {
                Ok(()) | Err(Control::Continue) => {}
                Err(Control::Break) => return Ok(()),
                Err(other) => return Err(other),
            }
        }
    }

    fn eval(&mut self, expression: &Expression, scope: &Rc<Scope>) -> Completion<Value> {
        self.enter()?;
        let result = self.eval_inner(expression, scope);
        self.leave();
        result
    }

    fn eval_inner(&mut self, expression: &Expression, scope: &Rc<Scope>) -> Completion<Value> {
        Ok(match expression {
            Expression::Number(value) => Value::Number(*value),
            Expression::String(text) => Value::String(text.clone()),
            Expression::Bool(value) => Value::Bool(*value),
            Expression::Null => Value::Null,
            Expression::Template(strings, substitutions) => {
                let mut units: Vec<u16> = Vec::new();
                for (index, text) in strings.iter().enumerate() {
                    units.extend_from_slice(text);
                    if let Some(substitution) = substitutions.get(index) {
                        let value = self.eval(substitution, scope)?;
                        units.extend_from_slice(&to_string(&value)?);
                    }
                }
                Value::String(units.into())
            }
            Expression::RegExp(source, flags) => Value::object(Kind::RegExp {
                source: source.clone(),
                flags: flags.clone(),
            }),
            Expression::Identifier(name) => match lookup(scope, name) {
                Some(value) => value,
                None => return unsupported(format!("unbekannter Name {name}")),
            },
            Expression::Array(elements) => Value::array(self.elements(elements, scope)?),
            Expression::Object(properties) => {
                let object = Value::object(Kind::Ordinary);
                for (key, value) in properties {
                    let value = self.eval(value, scope)?;
                    set(&object, key, value)?;
                }
                object
            }
            Expression::Function(function) => closure(function, scope),
            Expression::Unary(operator, operand) => match operator {
                UnaryOperator::Typeof => {
                    let value = match &**operand {
                        Expression::Identifier(name) => {
                            lookup(scope, name).unwrap_or(Value::Undefined)
                        }
                        other => self.eval(other, scope)?,
                    };
                    Value::string(type_of(&value))
                }
                UnaryOperator::Not => Value::Bool(!is_truthy(&self.eval(operand, scope)?)),
                UnaryOperator::Negate => Value::Number(-to_number(&self.eval(operand, scope)?)?),
                UnaryOperator::Plus => Value::Number(to_number(&self.eval(operand, scope)?)?),
            },
            Expression::Update {
                increment,
                prefix,
                target,
            } => {
                let reference = self.reference(target, scope)?;
                let old = to_number(&self.read(&reference, scope)?)?;
                let new = if *increment { old + 1.0 } else { old - 1.0 };
                self.write(&reference, Value::Number(new), scope)?;
                Value::Number(if *prefix { new } else { old })
            }
            Expression::Binary(operator, left, right) => {
                let left = self.eval(left, scope)?;
                let right = self.eval(right, scope)?;
                binary(*operator, &left, &right)?
            }
            Expression::Logical(operator, left, right) => {
                let left = self.eval(left, scope)?;
                let done = match operator {
                    LogicalOperator::And => !is_truthy(&left),
                    LogicalOperator::Or => is_truthy(&left),
                    LogicalOperator::Coalesce => !matches!(left, Value::Undefined | Value::Null),
                };
                if done { left } else { self.eval(right, scope)? }
            }
            Expression::Conditional(test, consequent, alternate) => {
                if is_truthy(&self.eval(test, scope)?) {
                    self.eval(consequent, scope)?
                } else {
                    self.eval(alternate, scope)?
                }
            }
            Expression::Assign(operator, target, value) => {
                let reference = self.reference(target, scope)?;
                let value = match operator {
                    None => self.eval(value, scope)?,
                    Some(operator) => {
                        let old = self.read(&reference, scope)?;
                        let right = self.eval(value, scope)?;
                        binary(*operator, &old, &right)?
                    }
                };
                self.write(&reference, value.clone(), scope)?;
                value
            }
            Expression::Sequence(list) => {
                let mut last = Value::Undefined;
                for expression in list {
                    last = self.eval(expression, scope)?;
                }
                last
            }
            Expression::Member(object, key) => {
                let object = self.eval(object, scope)?;
                get(&object, key)?
            }
            Expression::Index(object, key) => {
                let object = self.eval(object, scope)?;
                let key = property_key(&self.eval(key, scope)?)?;
                get(&object, &key)?
            }
            Expression::Call(callee, arguments) => {
                let (function, this) = match &**callee {
                    Expression::Member(object, key) => {
                        let object = self.eval(object, scope)?;
                        (get(&object, key)?, object)
                    }
                    Expression::Index(object, key) => {
                        let object = self.eval(object, scope)?;
                        let key = property_key(&self.eval(key, scope)?)?;
                        (get(&object, &key)?, object)
                    }
                    other => (self.eval(other, scope)?, Value::Undefined),
                };
                let arguments = self.elements(arguments, scope)?;
                if !function.is_callable() {
                    let name = match &**callee {
                        Expression::Identifier(name) => name.to_string(),
                        Expression::Member(_, key) => lossy(key),
                        _ => "Ausdruck".to_owned(),
                    };
                    return throw(&format!("TypeError: {name} is not a function"));
                }
                self.call(&function, this, arguments)?
            }
            Expression::New => return unsupported("new"),
        })
    }

    fn elements(&mut self, elements: &[Element], scope: &Rc<Scope>) -> Completion<Vec<Value>> {
        let mut values = Vec::new();
        for element in elements {
            let value = self.eval(&element.value, scope)?;
            if !element.spread {
                values.push(value);
                continue;
            }
            if let Value::String(units) = &value {
                values.extend(code_points(units));
            } else if let Some(items) = value.array_items() {
                values.extend(items);
            } else {
                return unsupported("Spread über ein Objekt, das keine Liste ist");
            }
        }
        Ok(values)
    }

    fn reference(&mut self, target: &Expression, scope: &Rc<Scope>) -> Completion<Reference> {
        match target {
            Expression::Identifier(name) => Ok(Reference::Variable(name.clone())),
            Expression::Member(object, key) => {
                Ok(Reference::Property(self.eval(object, scope)?, key.clone()))
            }
            Expression::Index(object, key) => {
                let object = self.eval(object, scope)?;
                let key = property_key(&self.eval(key, scope)?)?;
                Ok(Reference::Property(object, key))
            }
            _ => throw("SyntaxError: ungültiges Ziel einer Zuweisung"),
        }
    }

    fn read(&mut self, reference: &Reference, scope: &Rc<Scope>) -> Completion<Value> {
        match reference {
            Reference::Variable(name) => match lookup(scope, name) {
                Some(value) => Ok(value),
                None => unsupported(format!("unbekannter Name {name}")),
            },
            Reference::Property(object, key) => get(object, key),
        }
    }

    fn write(&mut self, reference: &Reference, value: Value, scope: &Rc<Scope>) -> Completion<()> {
        match reference {
            Reference::Variable(name) => self.assign(name, value, scope),
            Reference::Property(object, key) => set(object, key, value),
        }
    }

    fn assign(&mut self, name: &Rc<str>, value: Value, scope: &Rc<Scope>) -> Completion<()> {
        match assign_binding(scope, name, value.clone()) {
            Assigned::Done => Ok(()),
            Assigned::Constant => throw("TypeError: Assignment to constant variable."),
            Assigned::Missing if self.strict => {
                throw(&format!("ReferenceError: {name} is not defined"))
            }
            Assigned::Missing => {
                self.global.declare(name, value, true);
                Ok(())
            }
        }
    }

    /// Ruft eine Funktion auf.
    pub(crate) fn call(
        &mut self,
        function: &Value,
        this: Value,
        arguments: Vec<Value>,
    ) -> Completion<Value> {
        let Value::Object(object) = function else {
            return throw("TypeError: Wert ist keine Funktion");
        };
        match &object.kind {
            Kind::Native(native) => {
                let native = native.clone();
                self.enter()?;
                let result = native(self, this, arguments);
                self.leave();
                result
            }
            Kind::Closure { function, scope } => {
                self.enter()?;
                let local = Scope::new(Some(scope.clone()));
                let outer = std::mem::replace(&mut self.strict, function.strict);
                let result = self.run_function(function, &local, arguments);
                self.strict = outer;
                self.leave();
                result
            }
            _ => throw("TypeError: Wert ist keine Funktion"),
        }
    }

    fn run_function(
        &mut self,
        function: &Function,
        local: &Rc<Scope>,
        arguments: Vec<Value>,
    ) -> Completion<Value> {
        let mut arguments = arguments.into_iter();
        for (name, default) in &function.parameters {
            let mut value = arguments.next().unwrap_or(Value::Undefined);
            if let (Value::Undefined, Some(default)) = (&value, default) {
                value = self.eval(default, local)?;
            }
            local.declare(name, value, true);
        }
        match &function.body {
            Body::Expression(expression) => self.eval(expression, local),
            Body::Block(body) => {
                hoist_vars(body, local);
                match self.exec_list(body, local) {
                    Ok(()) => Ok(Value::Undefined),
                    Err(Control::Return(value)) => Ok(value),
                    Err(Control::Break | Control::Continue) => {
                        throw("SyntaxError: „break“ oder „continue“ außerhalb einer Schleife")
                    }
                    Err(other) => Err(other),
                }
            }
        }
    }
}

fn hoist_functions(body: &[Statement], scope: &Rc<Scope>) {
    for statement in body {
        if let Statement::Function(function) = statement
            && let Some(name) = &function.name
        {
            scope.declare(name, closure(function, scope), true);
        }
    }
}

// ---------------------------------------------------------------------------
// Eigenschaften

/// Die Methoden von `Object.prototype`, die es hier nicht gibt.
const OBJECT_PROTOTYPE: &[&str] = &[
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
    "__proto__",
];

/// Ein Arrayindex: eine Dezimalzahl ohne führende Nullen unter 2³² − 1.
fn array_index(key: &[u16]) -> Option<usize> {
    let text = String::from_utf16(key).ok()?;
    if text.is_empty()
        || !text.bytes().all(|byte| byte.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    let index: u64 = text.parse().ok()?;
    if index >= u64::from(u32::MAX) {
        return None;
    }
    usize::try_from(index).ok()
}

fn own_property(object: &Object, key: &[u16]) -> Option<Value> {
    object
        .properties
        .borrow()
        .iter()
        .find(|(name, _)| **name == *key)
        .map(|(_, value)| value.clone())
}

fn set_own(object: &Object, key: &[u16], value: Value) {
    let mut properties = object.properties.borrow_mut();
    match properties.iter_mut().find(|(name, _)| **name == *key) {
        Some((_, existing)) => *existing = value,
        None => properties.push((key.into(), value)),
    }
}

/// Liest eine Eigenschaft (`[[Get]]`).
pub(crate) fn get(object: &Value, key: &[u16]) -> Completion<Value> {
    match object {
        Value::Undefined | Value::Null => throw(&format!(
            "TypeError: Cannot read properties of {} (reading '{}')",
            describe(object),
            lossy(key)
        )),
        Value::Bool(_) | Value::Number(_) => unsupported(format!(
            "Eigenschaft {} eines Werts vom Typ {}",
            lossy(key),
            type_of(object)
        )),
        Value::String(units) => string_property(units, key),
        Value::Object(object) => object_property(object, key),
    }
}

fn string_property(units: &JsString, key: &[u16]) -> Completion<Value> {
    if is(key, "length") {
        return Ok(Value::Number(units.len() as f64));
    }
    if let Some(index) = array_index(key) {
        return Ok(match units.get(index) {
            Some(&unit) => Value::String(Rc::from([unit])),
            None => Value::Undefined,
        });
    }
    let method: fn(&mut Interpreter, Value, Vec<Value>) -> Completion<Value> = match &*lossy(key) {
        "replace" => string_replace,
        "includes" => string_includes,
        "indexOf" => string_index_of,
        other => return unsupported(format!("String.prototype.{other}")),
    };
    Ok(Value::native(method))
}

fn object_property(object: &Rc<Object>, key: &[u16]) -> Completion<Value> {
    match &object.kind {
        Kind::Array(items) => {
            if is(key, "length") {
                return Ok(Value::Number(items.borrow().len() as f64));
            }
            if let Some(index) = array_index(key) {
                return Ok(items
                    .borrow()
                    .get(index)
                    .cloned()
                    .unwrap_or(Value::Undefined));
            }
            let method: fn(&mut Interpreter, Value, Vec<Value>) -> Completion<Value> =
                match &*lossy(key) {
                    "push" => array_push,
                    "join" => array_join,
                    "includes" => array_includes,
                    "indexOf" => array_index_of,
                    "concat" => array_concat,
                    other => {
                        return match own_property(object, key) {
                            Some(value) => Ok(value),
                            None => unsupported(format!("Array.prototype.{other}")),
                        };
                    }
                };
            Ok(Value::native(method))
        }
        Kind::Ordinary => match own_property(object, key) {
            Some(value) => Ok(value),
            None if OBJECT_PROTOTYPE.iter().any(|name| is(key, name)) => {
                unsupported(format!("Object.prototype.{}", lossy(key)))
            }
            None => Ok(Value::Undefined),
        },
        Kind::Host(host) => match host.get(&lossy(key))? {
            Some(value) => Ok(value),
            None => Ok(own_property(object, key).unwrap_or(Value::Undefined)),
        },
        Kind::Closure { .. } | Kind::Native(_) | Kind::RegExp { .. } => {
            match own_property(object, key) {
                Some(value) => Ok(value),
                None => unsupported(format!("Eigenschaft {} einer Funktion", lossy(key))),
            }
        }
    }
}

/// Setzt eine Eigenschaft (`[[Set]]`).
pub(crate) fn set(object: &Value, key: &[u16], value: Value) -> Completion<()> {
    let Value::Object(object) = object else {
        return match object {
            Value::Undefined | Value::Null => throw(&format!(
                "TypeError: Cannot set properties of {} (setting '{}')",
                describe(object),
                lossy(key)
            )),
            _ => unsupported(format!(
                "Eigenschaft eines Werts vom Typ {} setzen",
                type_of(object)
            )),
        };
    };
    match &object.kind {
        Kind::Array(items) => {
            if let Some(index) = array_index(key) {
                let mut items = items.borrow_mut();
                if index >= items.len().saturating_add(1024) {
                    return unsupported("Liste mit großen Lücken");
                }
                if index >= items.len() {
                    items.resize(index.saturating_add(1), Value::Undefined);
                }
                if let Some(item) = items.get_mut(index) {
                    *item = value;
                }
                return Ok(());
            }
            if is(key, "length") {
                return unsupported("Setzen von length");
            }
        }
        Kind::Host(host) if host.set(&lossy(key), &value)? => return Ok(()),
        _ => {}
    }
    set_own(object, key, value);
    Ok(())
}

/// Die Namen der eigenen Eigenschaften eines einfachen Objekts wie bei
/// `Object.keys()`: erst Arrayindizes aufsteigend, dann die übrigen in der
/// Reihenfolge, in der sie angelegt wurden.
pub(crate) fn own_keys(value: &Value) -> Completion<Vec<JsString>> {
    let Value::Object(object) = value else {
        return unsupported("Object.keys() eines einfachen Werts");
    };
    if !matches!(object.kind, Kind::Ordinary) {
        return unsupported("Object.keys() eines besonderen Objekts");
    }
    let names: Vec<JsString> = object
        .properties
        .borrow()
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let mut indices: Vec<(usize, JsString)> = names
        .iter()
        .filter_map(|name| Some((array_index(name)?, name.clone())))
        .collect();
    indices.sort_by_key(|(index, _)| *index);
    let mut keys: Vec<JsString> = indices.into_iter().map(|(_, name)| name).collect();
    keys.extend(names.into_iter().filter(|name| array_index(name).is_none()));
    Ok(keys)
}

// ---------------------------------------------------------------------------
// Eingebaute Methoden

fn array_items_of(this: &Value) -> Completion<Vec<Value>> {
    match this.array_items() {
        Some(items) => Ok(items),
        None => unsupported("Methode einer Liste für ein anderes Objekt"),
    }
}

fn array_push(_: &mut Interpreter, this: Value, arguments: Vec<Value>) -> Completion<Value> {
    if let Value::Object(object) = &this
        && let Kind::Array(items) = &object.kind
    {
        let mut items = items.borrow_mut();
        items.extend(arguments);
        return Ok(Value::Number(items.len() as f64));
    }
    unsupported("Array.prototype.push für ein anderes Objekt")
}

fn array_join(_: &mut Interpreter, this: Value, arguments: Vec<Value>) -> Completion<Value> {
    let items = array_items_of(&this)?;
    let separator: Vec<u16> = match arguments.first() {
        None | Some(Value::Undefined) => vec![u16::from(b',')],
        Some(value) => to_string(value)?.to_vec(),
    };
    Ok(Value::String(join(&items, &separator, 0)?.into()))
}

fn join(items: &[Value], separator: &[u16], depth: usize) -> Completion<Vec<u16>> {
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.extend_from_slice(separator);
        }
        if !matches!(item, Value::Undefined | Value::Null) {
            out.extend(to_text(item, depth.saturating_add(1))?);
        }
    }
    Ok(out)
}

fn array_includes(_: &mut Interpreter, this: Value, arguments: Vec<Value>) -> Completion<Value> {
    if arguments.len() > 1 {
        return unsupported("Array.prototype.includes mit Startindex");
    }
    let search = arguments.first().cloned().unwrap_or(Value::Undefined);
    let items = array_items_of(&this)?;
    Ok(Value::Bool(
        items.iter().any(|item| same_value_zero(item, &search)),
    ))
}

fn array_index_of(_: &mut Interpreter, this: Value, arguments: Vec<Value>) -> Completion<Value> {
    if arguments.len() > 1 {
        return unsupported("Array.prototype.indexOf mit Startindex");
    }
    let search = arguments.first().cloned().unwrap_or(Value::Undefined);
    let items = array_items_of(&this)?;
    Ok(Value::Number(
        items
            .iter()
            .position(|item| strict_equals(item, &search))
            .map_or(-1.0, |index| index as f64),
    ))
}

fn array_concat(_: &mut Interpreter, this: Value, arguments: Vec<Value>) -> Completion<Value> {
    let mut items = array_items_of(&this)?;
    for argument in arguments {
        match argument.array_items() {
            Some(more) => items.extend(more),
            None => items.push(argument),
        }
    }
    Ok(Value::array(items))
}

fn this_string(this: &Value) -> Completion<JsString> {
    match this {
        Value::String(units) => Ok(units.clone()),
        _ => unsupported("Methode einer Zeichenkette für einen anderen Wert"),
    }
}

/// Die erste Stelle von `needle` in `haystack` ab `from`.
fn find(haystack: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    let last = haystack.len().checked_sub(needle.len())?;
    (from..=last)
        .find(|&start| haystack.get(start..start.saturating_add(needle.len())) == Some(needle))
}

fn string_includes(_: &mut Interpreter, this: Value, arguments: Vec<Value>) -> Completion<Value> {
    let (subject, search) = string_search(&this, &arguments)?;
    Ok(Value::Bool(find(&subject, &search, 0).is_some()))
}

fn string_index_of(_: &mut Interpreter, this: Value, arguments: Vec<Value>) -> Completion<Value> {
    let (subject, search) = string_search(&this, &arguments)?;
    Ok(Value::Number(
        find(&subject, &search, 0).map_or(-1.0, |index| index as f64),
    ))
}

fn string_search(this: &Value, arguments: &[Value]) -> Completion<(JsString, JsString)> {
    if arguments.len() > 1 {
        return unsupported("Suche in einer Zeichenkette mit Startindex");
    }
    let subject = this_string(this)?;
    let search = match arguments.first() {
        Some(Value::Object(object)) if matches!(object.kind, Kind::RegExp { .. }) => {
            return unsupported("Suche mit einem regulären Ausdruck");
        }
        Some(value) => to_string(value)?,
        None => js_string("undefined"),
    };
    Ok((subject, search))
}

/// Der Text, für den ein regulärer Ausdruck ohne Sonderzeichen steht.
/// Andere Ausdrücke werden nicht unterstützt.
fn regexp_literal(source: &[u16]) -> Completion<Vec<u16>> {
    let mut out = Vec::new();
    let mut units = source.iter().copied();
    while let Some(unit) = units.next() {
        if unit == u16::from(b'\\') {
            match units.next() {
                Some(next)
                    if !u8::try_from(next).is_ok_and(|byte| byte.is_ascii_alphanumeric()) =>
                {
                    out.push(next);
                }
                _ => {
                    return unsupported("regulärer Ausdruck mit Zeichenklassen oder Rückverweisen");
                }
            }
        } else if u8::try_from(unit).is_ok_and(|byte| b"^$.*+?()[]{}|".contains(&byte)) {
            return unsupported("regulärer Ausdruck mit Sonderzeichen");
        } else {
            out.push(unit);
        }
    }
    Ok(out)
}

fn string_replace(_: &mut Interpreter, this: Value, arguments: Vec<Value>) -> Completion<Value> {
    let subject = this_string(&this)?;
    let (pattern, global) = match arguments.first() {
        Some(Value::Object(object)) if matches!(object.kind, Kind::RegExp { .. }) => {
            let Kind::RegExp { source, flags } = &object.kind else {
                return unsupported("regulärer Ausdruck");
            };
            let global = match &**flags {
                "" => false,
                "g" => true,
                _ => return unsupported(format!("regulärer Ausdruck mit den Flags {flags}")),
            };
            (regexp_literal(source)?, global)
        }
        Some(value) => (to_string(value)?.to_vec(), false),
        None => (js_string("undefined").to_vec(), false),
    };
    let replacement = match arguments.get(1) {
        Some(value) if value.is_callable() => return unsupported("Ersetzung durch eine Funktion"),
        Some(value) => to_string(value)?,
        None => js_string("undefined"),
    };
    if pattern.is_empty() && global {
        return unsupported("globales Ersetzen eines leeren Musters");
    }
    let mut out = Vec::new();
    let mut position = 0usize;
    while let Some(found) = find(&subject, &pattern, position) {
        out.extend_from_slice(subject.get(position..found).unwrap_or_default());
        let end = found.saturating_add(pattern.len());
        substitute(&mut out, &replacement, &subject, found, end)?;
        position = end;
        if !global {
            break;
        }
    }
    out.extend_from_slice(subject.get(position..).unwrap_or_default());
    Ok(Value::String(out.into()))
}

/// *GetSubstitution* ohne Gruppen.
fn substitute(
    out: &mut Vec<u16>,
    replacement: &[u16],
    subject: &[u16],
    start: usize,
    end: usize,
) -> Completion<()> {
    let dollar = u16::from(b'$');
    let mut units = replacement.iter().copied().peekable();
    while let Some(unit) = units.next() {
        if unit != dollar {
            out.push(unit);
            continue;
        }
        match units
            .peek()
            .copied()
            .and_then(|next| u8::try_from(next).ok())
        {
            Some(b'$') => out.push(dollar),
            Some(b'&') => out.extend_from_slice(subject.get(start..end).unwrap_or_default()),
            Some(b'`') => out.extend_from_slice(subject.get(..start).unwrap_or_default()),
            Some(b'\'') => out.extend_from_slice(subject.get(end..).unwrap_or_default()),
            Some(b'0'..=b'9' | b'<') => return unsupported("Gruppen in der Ersetzung"),
            _ => {
                out.push(dollar);
                continue;
            }
        }
        units.next();
    }
    Ok(())
}

/// Die Codepunkte einer Zeichenkette als einzelne Zeichenketten, wie beim
/// Iterieren.
fn code_points(units: &[u16]) -> Vec<Value> {
    let mut values = Vec::new();
    let mut index = 0usize;
    while let Some(&unit) = units.get(index) {
        let pair = (0xD800..0xDC00).contains(&unit)
            && units
                .get(index.saturating_add(1))
                .is_some_and(|next| (0xDC00..0xE000).contains(next));
        let end = index.saturating_add(if pair { 2 } else { 1 });
        values.push(Value::String(
            units.get(index..end).unwrap_or_default().into(),
        ));
        index = end;
    }
    values
}

// ---------------------------------------------------------------------------
// Umwandlungen und Operatoren

pub(crate) fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => *value != 0.0 && !value.is_nan(),
        Value::String(units) => !units.is_empty(),
        Value::Object(_) => true,
    }
}

fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Object(object) => match object.kind {
            Kind::Closure { .. } | Kind::Native(_) => "function",
            _ => "object",
        },
    }
}

/// `ToString`
pub(crate) fn to_string(value: &Value) -> Completion<JsString> {
    match value {
        Value::String(units) => Ok(units.clone()),
        other => Ok(to_text(other, 0)?.into()),
    }
}

/// Die größte Verschachtelung von Listen beim Umwandeln in Text.
const MAX_JOIN_DEPTH: usize = 32;

fn to_text(value: &Value, depth: usize) -> Completion<Vec<u16>> {
    Ok(match value {
        Value::Undefined => js_string("undefined").to_vec(),
        Value::Null => js_string("null").to_vec(),
        Value::Bool(value) => js_string(if *value { "true" } else { "false" }).to_vec(),
        Value::Number(value) => js_string(&number_to_string(*value)).to_vec(),
        Value::String(units) => units.to_vec(),
        Value::Object(object) => match &object.kind {
            Kind::Array(items) => {
                if depth > MAX_JOIN_DEPTH {
                    return unsupported("zu tief verschachtelte Listen");
                }
                let items = items.borrow().clone();
                join(&items, &[u16::from(b',')], depth)?
            }
            Kind::Ordinary => js_string("[object Object]").to_vec(),
            _ => return unsupported("Umwandlung eines besonderen Objekts in Text"),
        },
    })
}

/// `ToNumber`
pub(crate) fn to_number(value: &Value) -> Completion<f64> {
    Ok(match value {
        Value::Undefined => f64::NAN,
        Value::Null => 0.0,
        Value::Bool(value) => f64::from(u8::from(*value)),
        Value::Number(value) => *value,
        Value::String(units) => string_to_number(units),
        Value::Object(_) => return unsupported("Umwandlung eines Objekts in eine Zahl"),
    })
}

/// `ToPropertyKey`
fn property_key(value: &Value) -> Completion<JsString> {
    to_string(value)
}

/// `ToPrimitive` für die Objekte, die es hier gibt.
fn to_primitive(value: &Value) -> Completion<Value> {
    match value {
        Value::Object(_) => Ok(Value::String(to_string(value)?)),
        other => Ok(other.clone()),
    }
}

pub(crate) fn strict_equals(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Number(left), Value::Number(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Object(left), Value::Object(right)) => Rc::ptr_eq(left, right),
        _ => false,
    }
}

/// *SameValue*, wie `assert_equals()` vergleicht: `NaN` gleicht sich
/// selbst, +0 und −0 sind verschieden.
pub(crate) fn same_value(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            (left.is_nan() && right.is_nan())
                || (left == right && left.is_sign_negative() == right.is_sign_negative())
        }
        _ => strict_equals(left, right),
    }
}

fn same_value_zero(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            (left.is_nan() && right.is_nan()) || left == right
        }
        _ => strict_equals(left, right),
    }
}

/// *IsLooselyEqual*
fn loose_equals(left: &Value, right: &Value) -> Completion<bool> {
    if std::mem::discriminant(left) == std::mem::discriminant(right) {
        return Ok(strict_equals(left, right));
    }
    Ok(match (left, right) {
        (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
        (Value::Number(_), Value::String(units)) => {
            strict_equals(left, &Value::Number(string_to_number(units)))
        }
        (Value::String(units), Value::Number(_)) => {
            strict_equals(&Value::Number(string_to_number(units)), right)
        }
        (Value::Bool(value), _) => {
            return loose_equals(&Value::Number(f64::from(u8::from(*value))), right);
        }
        (_, Value::Bool(value)) => {
            return loose_equals(left, &Value::Number(f64::from(u8::from(*value))));
        }
        (Value::Object(_), Value::Undefined | Value::Null)
        | (Value::Undefined | Value::Null, Value::Object(_)) => false,
        (Value::Object(_), _) | (_, Value::Object(_)) => {
            return unsupported("Vergleich eines Objekts mit einem einfachen Wert");
        }
        _ => false,
    })
}

fn compare(left: &Value, right: &Value) -> Completion<Option<Ordering>> {
    let left = to_primitive(left)?;
    let right = to_primitive(right)?;
    if let (Value::String(left), Value::String(right)) = (&left, &right) {
        return Ok(Some(left.cmp(right)));
    }
    Ok(to_number(&left)?.partial_cmp(&to_number(&right)?))
}

fn binary(operator: BinaryOperator, left: &Value, right: &Value) -> Completion<Value> {
    use BinaryOperator as B;
    Ok(match operator {
        B::Add => {
            let left = to_primitive(left)?;
            let right = to_primitive(right)?;
            if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
                let mut units = to_string(&left)?.to_vec();
                units.extend_from_slice(&to_string(&right)?);
                Value::String(units.into())
            } else {
                Value::Number(to_number(&left)? + to_number(&right)?)
            }
        }
        B::Subtract => Value::Number(to_number(left)? - to_number(right)?),
        B::Multiply => Value::Number(to_number(left)? * to_number(right)?),
        B::Divide => Value::Number(to_number(left)? / to_number(right)?),
        B::Remainder => Value::Number(to_number(left)? % to_number(right)?),
        B::Less => Value::Bool(compare(left, right)? == Some(Ordering::Less)),
        B::Greater => Value::Bool(compare(left, right)? == Some(Ordering::Greater)),
        B::LessEqual => Value::Bool(matches!(
            compare(left, right)?,
            Some(Ordering::Less | Ordering::Equal)
        )),
        B::GreaterEqual => Value::Bool(matches!(
            compare(left, right)?,
            Some(Ordering::Greater | Ordering::Equal)
        )),
        B::Equal => Value::Bool(loose_equals(left, right)?),
        B::NotEqual => Value::Bool(!loose_equals(left, right)?),
        B::StrictEqual => Value::Bool(strict_equals(left, right)),
        B::StrictNotEqual => Value::Bool(!strict_equals(left, right)),
    })
}

fn is_string_whitespace(unit: u16) -> bool {
    char::from_u32(u32::from(unit)).is_some_and(|c| is_whitespace(c) || is_line_terminator(c))
}

/// Der längste Anfang, der ein *StrDecimalLiteral* ist, mit seinem Wert und
/// seiner Länge.
fn decimal_prefix(units: &[u16]) -> Option<(f64, usize)> {
    let at = |index: usize| units.get(index).and_then(|&unit| u8::try_from(unit).ok());
    let digits_from = |mut index: usize| {
        while at(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index = index.saturating_add(1);
        }
        index
    };
    let text = |from: usize, to: usize| -> String {
        String::from_utf16_lossy(units.get(from..to).unwrap_or_default())
    };
    let mut index = 0usize;
    let negative = match at(0) {
        Some(b'-') => {
            index = 1;
            true
        }
        Some(b'+') => {
            index = 1;
            false
        }
        _ => false,
    };
    let sign = if negative { -1.0 } else { 1.0 };
    if units
        .get(index..index.saturating_add(8))
        .is_some_and(|word| is(word, "Infinity"))
    {
        return Some((sign * f64::INFINITY, index.saturating_add(8)));
    }
    let integer_start = index;
    index = digits_from(index);
    let integer = text(integer_start, index);
    let mut fraction = String::new();
    if at(index) == Some(b'.') {
        let start = index.saturating_add(1);
        let end = digits_from(start);
        if !integer.is_empty() || end > start {
            fraction = text(start, end);
            index = end;
        }
    }
    if integer.is_empty() && fraction.is_empty() {
        return None;
    }
    let mut exponent = String::from("0");
    if let Some(b'e' | b'E') = at(index) {
        let mut start = index.saturating_add(1);
        let mut exponent_sign = "";
        if let Some(found @ (b'+' | b'-')) = at(start) {
            exponent_sign = if found == b'-' { "-" } else { "" };
            start = start.saturating_add(1);
        }
        let end = digits_from(start);
        if end > start {
            exponent = format!("{exponent_sign}{}", text(start, end));
            index = end;
        }
    }
    let integer = if integer.is_empty() { "0" } else { &integer };
    let fraction = if fraction.is_empty() { "0" } else { &fraction };
    let value: f64 = format!("{integer}.{fraction}e{exponent}").parse().ok()?;
    Some((sign * value, index))
}

/// `parseFloat()`
pub(crate) fn parse_float(units: &[u16]) -> f64 {
    let start = units
        .iter()
        .position(|&unit| !is_string_whitespace(unit))
        .unwrap_or(units.len());
    decimal_prefix(units.get(start..).unwrap_or_default()).map_or(f64::NAN, |(value, _)| value)
}

/// *StringToNumber*
fn string_to_number(units: &[u16]) -> f64 {
    let start = units
        .iter()
        .position(|&unit| !is_string_whitespace(unit))
        .unwrap_or(units.len());
    let end = units
        .iter()
        .rposition(|&unit| !is_string_whitespace(unit))
        .map_or(start, |last| last.saturating_add(1));
    let trimmed = units.get(start..end).unwrap_or_default();
    if trimmed.is_empty() {
        return 0.0;
    }
    let text = String::from_utf16_lossy(trimmed);
    let radix = match text.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = text.get(2..).unwrap_or_default();
        if digits.is_empty() {
            return f64::NAN;
        }
        let mut value = 0.0;
        for c in digits.chars() {
            let Some(digit) = c.to_digit(radix) else {
                return f64::NAN;
            };
            value = value * f64::from(radix) + f64::from(digit);
        }
        return value;
    }
    match decimal_prefix(trimmed) {
        Some((value, length)) if length == trimmed.len() => value,
        _ => f64::NAN,
    }
}

/// `Number::toString(10)`
pub(crate) fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if value < 0.0 {
        return format!("-{}", number_to_string(-value));
    }
    // Rust schreibt mit `{:e}` die kürzesten Ziffern, die den Wert
    // eindeutig bestimmen, wie es ECMAScript verlangt.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    let exponent: i64 = exponent.parse().unwrap_or(0);
    let k = i64::try_from(digits.len()).unwrap_or(i64::MAX);
    let n = exponent.saturating_add(1);
    let at = |count: i64| usize::try_from(count).unwrap_or(0);
    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat(at(n.saturating_sub(k))))
    } else if 0 < n && n <= 21 {
        let (integer, fraction) = digits.split_at(at(n).min(digits.len()));
        format!("{integer}.{fraction}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat(at(-n)))
    } else {
        let e = n.saturating_sub(1);
        let sign = if e < 0 { '-' } else { '+' };
        let (first, rest) = digits.split_at(1.min(digits.len()));
        let point = if rest.is_empty() { "" } else { "." };
        format!("{first}{point}{rest}e{sign}{}", e.unsigned_abs())
    }
}

/// `JSON.stringify()` für eine Zeichenkette (*QuoteJSONString*).
pub(crate) fn json_quote(units: &[u16]) -> String {
    let mut out = String::from("\"");
    for item in char::decode_utf16(units.iter().copied()) {
        match item {
            Ok('"') => out.push_str("\\\""),
            Ok('\\') => out.push_str("\\\\"),
            Ok('\u{8}') => out.push_str("\\b"),
            Ok('\u{C}') => out.push_str("\\f"),
            Ok('\n') => out.push_str("\\n"),
            Ok('\r') => out.push_str("\\r"),
            Ok('\t') => out.push_str("\\t"),
            Ok(c) if c < ' ' => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            Ok(c) => out.push(c),
            Err(error) => out.push_str(&format!("\\u{:04x}", error.unpaired_surrogate())),
        }
    }
    out.push('"');
    out
}

/// Ein Wert als Text für Meldungen.
pub(crate) fn describe(value: &Value) -> String {
    match value {
        Value::String(units) => lossy(units),
        Value::Object(object) => match object.kind {
            Kind::Closure { .. } | Kind::Native(_) => "function".to_owned(),
            Kind::Array(_) => match to_text(value, 0) {
                Ok(units) => format!("[{}]", lossy(&units)),
                Err(_) => "[…]".to_owned(),
            },
            _ => "[object]".to_owned(),
        },
        other => to_text(other, 0).map_or_else(|_| String::new(), |units| lossy(&units)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Führt ein Skript aus und liefert den Wert der globalen Variable
    /// `result` als Text.
    fn run(source: &str) -> String {
        let mut interpreter = Interpreter::new();
        let result = interpreter.run_script(source).expect("Syntax");
        assert!(result.unsupported.is_empty(), "{:?}", result.unsupported);
        assert_eq!(result.exception, None);
        let value = lookup(&interpreter.global, "result").expect("result");
        describe(&value)
    }

    #[test]
    fn numbers_to_strings() {
        for (value, expected) in [
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
            (123.456, "123.456"),
            (f64::NAN, "NaN"),
            (-0.0, "0"),
        ] {
            assert_eq!(number_to_string(value), expected);
        }
    }

    #[test]
    fn strings_to_numbers() {
        let parse = |text: &str| string_to_number(&js_string(text));
        assert_eq!(parse("  12.5e1 "), 125.0);
        assert_eq!(parse(""), 0.0);
        assert_eq!(parse("0x1f"), 31.0);
        assert!(parse("1px").is_nan());
        assert!(parse("inf").is_nan());
        assert_eq!(parse_float(&js_string(" 50%")), 50.0);
        assert_eq!(parse_float(&js_string(".5)")), 0.5);
        assert!(parse_float(&js_string("none")).is_nan());
    }

    #[test]
    fn closures_and_loops() {
        assert_eq!(
            run(
                "const next = (() => { let counter = 0; return () => counter++ % 3; })();
                 var result = [];
                 for (let i = 0; i < 5; i++) result.push(next());"
            ),
            "[0,1,2,0,1]"
        );
        assert_eq!(
            run("function f(list, extra = 'x') { const out = [];
                   for (const item of list) { if (item === 'b') continue; out.push(item + extra); }
                   return out.join(' '); }
                 result = f(['a', 'b', ...['c']])"),
            "ax cx"
        );
        assert_eq!(
            run("let result = `a${1 + 1}b${'c'.replace(/c/g, 'd')}`; result += [1, [2, 3]]"),
            "a2bd1,2,3"
        );
        assert_eq!(
            run("var result = '';
                 try { throw 'x'; } catch (e) { result = e; } finally { result += '!'; }"),
            "x!"
        );
    }

    #[test]
    fn unsupported_statements_are_skipped() {
        let mut interpreter = Interpreter::new();
        let result = interpreter
            .run_script("var a = 1; unknown(); a = 2;")
            .expect("Syntax");
        assert_eq!(result.unsupported, ["unbekannter Name unknown"]);
        let value = lookup(&interpreter.global, "a").expect("a");
        assert_eq!(describe(&value), "2");
        assert!(Interpreter::new().run_script("class A {}").is_err());
    }
}
