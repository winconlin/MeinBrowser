//! Parser nach CSS Syntax Level 3 („Parsing“).
//!
//! Die Algorithmen der Spezifikation rufen sich gegenseitig rekursiv auf.
//! Damit tief verschachteltes CSS keinen Stapelüberlauf auslöst, laufen sie
//! hier mit eigenen Stapeln, und die Ergebnisse sind flach gespeichert:
//! Komponentenwerte in Vorordnung ([`ComponentValues`]), Regeln in einem
//! Speicher, in dem Kinder per [`RuleId`] verweisen ([`RuleTree`]). So
//! braucht auch das Freigeben keine Rekursion.
//!
//! Ob etwas „valid in the current context“ ist, lässt die Syntax offen; das
//! entscheidet ein [`Context`]. [`Syntax`] lässt alles zu.

use std::ops::Range;

use crate::Source;
use crate::tokenizer::{Token, tokenize};

/// Die Klammer eines einfachen Blocks (*associated token*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// {}-block
    Brace,
    /// []-block
    Bracket,
    /// ()-block
    Parenthesis,
}

/// Ein Knoten einer flachen Liste von Komponentenwerten.
#[derive(Clone, Debug, PartialEq)]
enum Node {
    Token(Token),
    /// `len` ist die Zahl der Knoten, die zum Wert gehören.
    Function {
        name: String,
        len: usize,
    },
    Block {
        kind: BlockKind,
        len: usize,
    },
}

/// Eine Liste von Komponentenwerten (*component values*). Auf eine Funktion
/// oder einen Block folgen die Knoten ihres Werts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComponentValues {
    nodes: Vec<Node>,
}

impl ComponentValues {
    /// Die Werte der obersten Ebene.
    pub fn iter(&self) -> Iter<'_> {
        self.values().iter()
    }

    pub fn values(&self) -> Values<'_> {
        Values { nodes: &self.nodes }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl<'a> IntoIterator for &'a ComponentValues {
    type Item = ComponentValue<'a>;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// Ein Komponentenwert (*component value*).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComponentValue<'a> {
    /// Ein *preserved token*. ⟨function-token⟩ und öffnende Klammern kommen
    /// hier nie vor, sie werden immer zu Funktionen und Blöcken.
    Token(&'a Token),
    /// *function*
    Function { name: &'a str, value: Values<'a> },
    /// *simple block*
    Block { kind: BlockKind, value: Values<'a> },
}

/// Eine geliehene Liste von Komponentenwerten, etwa der Wert einer Funktion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Values<'a> {
    nodes: &'a [Node],
}

impl<'a> Values<'a> {
    /// Die Werte der obersten Ebene.
    pub fn iter(self) -> Iter<'a> {
        Iter { nodes: self.nodes }
    }

    pub fn is_empty(self) -> bool {
        self.nodes.is_empty()
    }
}

impl<'a> IntoIterator for Values<'a> {
    type Item = ComponentValue<'a>;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// Durchläuft eine Liste auf ihrer obersten Ebene.
#[derive(Clone, Debug)]
pub struct Iter<'a> {
    nodes: &'a [Node],
}

impl<'a> Iterator for Iter<'a> {
    type Item = ComponentValue<'a>;

    fn next(&mut self) -> Option<ComponentValue<'a>> {
        let (first, rest) = self.nodes.split_first()?;
        let (value, rest) = match first {
            Node::Token(token) => (ComponentValue::Token(token), rest),
            Node::Function { name, len } => {
                let (value, rest) = split(rest, *len);
                let value = Values { nodes: value };
                (ComponentValue::Function { name, value }, rest)
            }
            Node::Block { kind, len } => {
                let (value, rest) = split(rest, *len);
                let value = Values { nodes: value };
                (ComponentValue::Block { kind: *kind, value }, rest)
            }
        };
        self.nodes = rest;
        Some(value)
    }
}

fn split(nodes: &[Node], len: usize) -> (&[Node], &[Node]) {
    // Die Längen stimmen immer, der Rückfall vermeidet nur eine Panic.
    nodes.split_at_checked(len).unwrap_or((nodes, &[]))
}

/// Verweis auf eine Regel in einem [`RuleTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleId(usize);

/// Eine Regel (*rule*) oder eine Liste von Deklarationen zwischen Regeln.
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// *at-rule*
    At(AtRule),
    /// *qualified rule*
    Qualified(QualifiedRule),
    /// Deklarationen zwischen Regeln eines Blocks. In einer At-Regel liefert
    /// die Syntax sie so, in einer qualifizierten Regel sind es *nested
    /// declarations rules*.
    Declarations(Vec<Declaration>),
}

/// *at-rule*
#[derive(Clone, Debug, PartialEq)]
pub struct AtRule {
    pub name: String,
    pub prelude: ComponentValues,
    /// Der Inhalt des {}-Blocks, `None` bei einer At-Regel, die mit einem
    /// Semikolon endet. Die Syntax setzt nur die Kindregeln; was mit den
    /// Deklarationslisten darin geschieht, legt die jeweilige Regel fest.
    pub block: Option<Vec<RuleId>>,
}

/// *qualified rule*
#[derive(Clone, Debug, PartialEq)]
pub struct QualifiedRule {
    pub prelude: ComponentValues,
    pub declarations: Vec<Declaration>,
    /// Kindregeln, darunter [`Rule::Declarations`] für Deklarationen nach
    /// der ersten Kindregel.
    pub rules: Vec<RuleId>,
}

/// *declaration*
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub value: ComponentValues,
    pub important: bool,
    /// *original text*, nur bei Custom Properties: der Quelltext des Werts
    /// samt Kommentaren.
    pub original_text: Option<String>,
}

/// Regeln aus einem Einstieg. Alle liegen in einem Speicher, Kinder
/// verweisen per [`RuleId`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuleTree {
    rules: Vec<Rule>,
    roots: Vec<RuleId>,
}

impl RuleTree {
    /// Die Regeln der obersten Ebene, etwa die eines Stylesheets.
    pub fn roots(&self) -> &[RuleId] {
        &self.roots
    }

    pub fn get(&self, id: RuleId) -> Option<&Rule> {
        self.rules.get(id.0)
    }
}

/// Entscheidet, was „valid in the current context“ ist. Die Syntax lässt
/// offen, wie der Kontext weitergereicht wird. Hier liefert ein Kontext für
/// den Block jeder Regel einen eigenen, der dort gilt.
pub trait Context: Sized {
    fn accepts_declaration(&self, declaration: &Declaration) -> bool;

    fn accepts_at_rule(&self, rule: &AtRule) -> bool;

    fn accepts_qualified_rule(&self, rule: &QualifiedRule) -> bool;

    /// Der Kontext im Block einer At-Regel.
    fn at_rule_block(&self, name: &str, prelude: &ComponentValues) -> Self;

    /// Der Kontext im Block einer qualifizierten Regel.
    fn qualified_rule_block(&self, prelude: &ComponentValues) -> Self;
}

/// Ein Kontext ohne Grammatiken: Gültig ist alles, was die Syntax zulässt.
#[derive(Clone, Copy, Debug, Default)]
pub struct Syntax;

impl Context for Syntax {
    fn accepts_declaration(&self, _: &Declaration) -> bool {
        true
    }

    fn accepts_at_rule(&self, _: &AtRule) -> bool {
        true
    }

    fn accepts_qualified_rule(&self, _: &QualifiedRule) -> bool {
        true
    }

    fn at_rule_block(&self, _: &str, _: &ComponentValues) -> Syntax {
        Syntax
    }

    fn qualified_rule_block(&self, _: &ComponentValues) -> Syntax {
        Syntax
    }
}

/// Der *syntax error* der Einstiege, die genau ein Ergebnis liefern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxError;

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CSS-Syntaxfehler")
    }
}

impl std::error::Error for SyntaxError {}

// Einstiege („Parser Entry Points“). *Normalize into a token stream* heißt
// hier: Die Eingabe ist eine gefilterte [`Source`], der Parser zerlegt sie
// in Tokens.

/// *Parse a stylesheet* aus Text; für Bytes vorher
/// [`crate::decode_stylesheet`]. Den Ort (*location*) hält der Aufrufer.
/// *Parse a stylesheet's contents* ist für Text dasselbe.
pub fn parse_stylesheet<C: Context>(input: &Source, context: &C) -> RuleTree {
    let mut parser = Parser::new(input.chars(), false);
    let roots = parser.consume_stylesheet_contents(context);
    RuleTree {
        rules: parser.rules,
        roots,
    }
}

/// *Parse a list of declarations*, zum Beispiel für das Attribut `style`.
/// At-Regeln werden verbraucht und verworfen.
pub fn parse_declaration_list<C: Context>(input: &Source, context: &C) -> Vec<Declaration> {
    let mut parser = Parser::new(input.chars(), false);
    let mut declarations = Vec::new();
    while !parser.input.is_empty() {
        parser.input.discard_whitespace();
        match parser.input.next() {
            Some(Token::Semicolon) => parser.input.discard(),
            Some(Token::AtKeyword(_)) => {
                parser.consume_rule(Start::AtRule { nested: false }, context);
            }
            _ => {
                if let Some(declaration) = parser.consume_declaration(false, false, context) {
                    declarations.push(declaration);
                }
            }
        }
    }
    declarations
}

/// *Parse a rule*: Der Baum hat dann genau eine Wurzel.
pub fn parse_rule<C: Context>(input: &Source, context: &C) -> Result<RuleTree, SyntaxError> {
    let mut parser = Parser::new(input.chars(), false);
    parser.input.discard_whitespace();
    let start = match parser.input.next() {
        None => return Err(SyntaxError),
        Some(Token::AtKeyword(_)) => Start::AtRule { nested: false },
        Some(_) => Start::QualifiedRule {
            stop: None,
            nested: false,
        },
    };
    let Outcome::Rule(rule) = parser.consume_rule(start, context) else {
        return Err(SyntaxError);
    };
    parser.input.discard_whitespace();
    if !parser.input.is_empty() {
        return Err(SyntaxError);
    }
    Ok(RuleTree {
        rules: parser.rules,
        roots: vec![rule],
    })
}

/// *Parse a declaration*
pub fn parse_declaration<C: Context>(
    input: &Source,
    context: &C,
) -> Result<Declaration, SyntaxError> {
    let mut parser = Parser::new(input.chars(), false);
    parser.input.discard_whitespace();
    if parser.input.is_empty() {
        return Err(SyntaxError);
    }
    let declaration = parser
        .consume_declaration(false, false, context)
        .ok_or(SyntaxError)?;
    parser.input.discard_whitespace();
    if !parser.input.is_empty() {
        return Err(SyntaxError);
    }
    Ok(declaration)
}

/// *Parse a component value*: Die Liste hat dann genau einen Wert.
pub fn parse_component_value(input: &Source) -> Result<ComponentValues, SyntaxError> {
    let mut parser = Parser::new(input.chars(), false);
    parser.input.discard_whitespace();
    if parser.input.is_empty() {
        return Err(SyntaxError);
    }
    let mut nodes = Vec::new();
    parser.consume_component_value(&mut nodes);
    parser.input.discard_whitespace();
    if !parser.input.is_empty() {
        return Err(SyntaxError);
    }
    Ok(ComponentValues { nodes })
}

/// *Parse a list of component values*
pub fn parse_component_value_list(input: &Source) -> ComponentValues {
    let mut parser = Parser::new(input.chars(), false);
    let mut nodes = Vec::new();
    parser.consume_list(None, false, &mut nodes);
    ComponentValues { nodes }
}

/// *Parse a comma-separated list of component values*
pub fn parse_comma_separated_component_value_list(input: &Source) -> Vec<ComponentValues> {
    let mut parser = Parser::new(input.chars(), false);
    let mut groups = Vec::new();
    while !parser.input.is_empty() {
        let mut nodes = Vec::new();
        parser.consume_list(Some(Stop::Comma), false, &mut nodes);
        groups.push(ComponentValues { nodes });
        parser.input.discard();
    }
    groups
}

/// *Token stream* über die ganze, vorab zerlegte Eingabe.
struct TokenStream<'a> {
    source: &'a [char],
    /// Die Tokens mit ihrer Lage in `source`.
    tokens: Vec<(Token, Range<usize>)>,
    index: usize,
    marks: Vec<usize>,
}

impl<'a> TokenStream<'a> {
    fn new(source: &'a [char], unicode_ranges_allowed: bool) -> TokenStream<'a> {
        TokenStream {
            source,
            tokens: tokenize(source, unicode_ranges_allowed),
            index: 0,
            marks: Vec::new(),
        }
    }

    /// *next token*; `None` steht für das ⟨eof-token⟩.
    fn next(&self) -> Option<&Token> {
        self.tokens.get(self.index).map(|(token, _)| token)
    }

    /// *empty*
    fn is_empty(&self) -> bool {
        self.next().is_none()
    }

    /// *consume a token*. Am Ende bleibt der Index stehen, das nächste Token
    /// ist dann ohnehin weiter das ⟨eof-token⟩.
    fn consume(&mut self) -> Option<Token> {
        let token = self.next().cloned();
        self.discard();
        token
    }

    /// *discard a token*
    fn discard(&mut self) {
        if !self.is_empty() {
            self.index += 1;
        }
    }

    /// *mark*
    fn mark(&mut self) {
        self.marks.push(self.index);
    }

    /// *restore a mark*
    fn restore_mark(&mut self) {
        if let Some(index) = self.marks.pop() {
            self.index = index;
        }
    }

    /// *discard a mark*
    fn discard_mark(&mut self) {
        self.marks.pop();
    }

    /// *discard whitespace*
    fn discard_whitespace(&mut self) {
        while self.next() == Some(&Token::Whitespace) {
            self.discard();
        }
    }

    /// Der Quelltext der Tokens `tokens`, samt Kommentaren dazwischen.
    fn text(&self, tokens: Range<usize>) -> &'a [char] {
        let first = self.tokens.get(tokens.start);
        let last = tokens
            .end
            .checked_sub(1)
            .and_then(|last| self.tokens.get(last));
        match (first, last) {
            (Some((_, first)), Some((_, last))) if tokens.start < tokens.end => {
                self.source.get(first.start..last.end).unwrap_or_default()
            }
            _ => &[],
        }
    }
}

/// Wo eine Liste von Komponentenwerten endet (*stop token*).
#[derive(Clone, Copy)]
enum Stop {
    Semicolon,
    Comma,
}

impl Stop {
    fn matches(self, token: &Token) -> bool {
        matches!(
            (self, token),
            (Stop::Semicolon, Token::Semicolon) | (Stop::Comma, Token::Comma)
        )
    }
}

/// Ein Wert auf der obersten Ebene einer Liste: sein erster Knoten und die
/// Tokens, aus denen er besteht.
struct Item {
    node: usize,
    tokens: Range<usize>,
}

/// Wie eine Regel beginnt: *consume an at-rule* oder *consume a qualified
/// rule*, mit ihren Parametern.
#[derive(Clone, Copy)]
enum Start {
    AtRule { nested: bool },
    QualifiedRule { stop: Option<Stop>, nested: bool },
}

/// Was *consume an at-rule* und *consume a qualified rule* liefern.
enum Outcome {
    Rule(RuleId),
    Nothing,
    /// *invalid rule error*
    Invalid,
}

/// Was nach dem Prelude einer Regel kommt.
enum Prelude {
    Finished(Outcome),
    /// Ein {}-Block folgt.
    Block(Owner),
}

/// Die Regel, deren Block gerade gelesen wird.
enum Owner {
    AtRule {
        name: String,
        prelude: ComponentValues,
    },
    QualifiedRule {
        prelude: ComponentValues,
    },
    /// Eine Regel auf oberster Ebene, die wie eine Custom Property aussieht:
    /// Ihr Block wird nur verbraucht.
    Ignored {
        prelude: ComponentValues,
    },
}

/// Ein Eintrag aus *consume a block's contents*.
enum Entry {
    Rule(RuleId),
    Declarations(Vec<Declaration>),
}

/// Ein offener Block auf dem Stapel von [`Parser::consume_rule`].
struct Frame<C> {
    owner: Owner,
    /// Der Kontext im Block.
    context: C,
    /// `rules` und `decls` aus *consume a block's contents*.
    rules: Vec<Entry>,
    decls: Vec<Declaration>,
    /// Die Größe des Regelspeichers, als der Block begann. Was danach
    /// hinzukommt, gehört zur Regel und fällt weg, wenn sie ungültig ist.
    stored: usize,
}

impl<C> Frame<C> {
    /// „If decls is not empty, append it to rules, and set decls to a fresh
    /// empty list of declarations.“
    fn flush(&mut self) {
        if !self.decls.is_empty() {
            let decls = std::mem::take(&mut self.decls);
            self.rules.push(Entry::Declarations(decls));
        }
    }

    /// Das Ergebnis einer Regel im Block, wie *consume a block's contents*
    /// es behandelt. Vor einer At-Regel ist `decls` schon geleert.
    fn receive(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Nothing => {}
            Outcome::Invalid => self.flush(),
            Outcome::Rule(rule) => {
                self.flush();
                self.rules.push(Entry::Rule(rule));
            }
        }
    }
}

/// Was *consume a block's contents* als Nächstes braucht.
enum Step {
    /// Eine verschachtelte Regel beginnt.
    Rule(Start),
    /// Der Block ist zu Ende.
    End,
}

struct Parser<'a> {
    input: TokenStream<'a>,
    /// Alle fertigen Regeln; [`RuleId`] ist ein Index hierin.
    rules: Vec<Rule>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a [char], unicode_ranges_allowed: bool) -> Parser<'a> {
        Parser {
            input: TokenStream::new(source, unicode_ranges_allowed),
            rules: Vec::new(),
        }
    }

    fn store(&mut self, rule: Rule) -> RuleId {
        self.rules.push(rule);
        RuleId(self.rules.len() - 1)
    }

    /// *Consume a stylesheet's contents*
    fn consume_stylesheet_contents<C: Context>(&mut self, context: &C) -> Vec<RuleId> {
        let mut rules = Vec::new();
        loop {
            let start = match self.input.next() {
                None => return rules,
                Some(Token::Whitespace | Token::Cdo | Token::Cdc) => {
                    self.input.discard();
                    continue;
                }
                Some(Token::AtKeyword(_)) => Start::AtRule { nested: false },
                Some(_) => Start::QualifiedRule {
                    stop: None,
                    nested: false,
                },
            };
            if let Outcome::Rule(rule) = self.consume_rule(start, context) {
                rules.push(rule);
            }
        }
    }

    /// *Consume an at-rule* oder *consume a qualified rule*, samt allen
    /// verschachtelten Regeln. Statt der Rekursion *consume a block* →
    /// *consume a block's contents* → *consume a qualified rule* → … liegen
    /// die offenen Blöcke auf `stack`.
    fn consume_rule<C: Context>(&mut self, start: Start, context: &C) -> Outcome {
        let mut stack: Vec<Frame<C>> = Vec::new();
        let mut next = Some(start);
        loop {
            let outcome = if let Some(start) = next.take() {
                let current = stack.last().map_or(context, |frame| &frame.context);
                match self.consume_prelude(start, current) {
                    Prelude::Finished(outcome) => outcome,
                    Prelude::Block(owner) => {
                        let context = match &owner {
                            Owner::AtRule { name, prelude } => current.at_rule_block(name, prelude),
                            Owner::QualifiedRule { prelude } | Owner::Ignored { prelude } => {
                                current.qualified_rule_block(prelude)
                            }
                        };
                        // *Consume a block*: das „{“.
                        self.input.discard();
                        stack.push(Frame {
                            owner,
                            context,
                            rules: Vec::new(),
                            decls: Vec::new(),
                            stored: self.rules.len(),
                        });
                        continue;
                    }
                }
            } else {
                let Some(frame) = stack.last_mut() else {
                    return Outcome::Nothing;
                };
                match self.consume_block_contents(frame) {
                    Step::Rule(start) => {
                        next = Some(start);
                        continue;
                    }
                    Step::End => {
                        // *Consume a block*: das „}“.
                        self.input.discard();
                        let Some(frame) = stack.pop() else {
                            return Outcome::Nothing;
                        };
                        let parent = stack.last().map_or(context, |frame| &frame.context);
                        self.finish(frame, parent)
                    }
                }
            };
            match stack.last_mut() {
                Some(parent) => parent.receive(outcome),
                None => return outcome,
            }
        }
    }

    fn consume_prelude<C: Context>(&mut self, start: Start, context: &C) -> Prelude {
        match start {
            Start::AtRule { nested } => self.consume_at_rule_prelude(nested, context),
            Start::QualifiedRule { stop, nested } => {
                self.consume_qualified_rule_prelude(stop, nested)
            }
        }
    }

    /// *Consume an at-rule* bis zum Block.
    fn consume_at_rule_prelude<C: Context>(&mut self, nested: bool, context: &C) -> Prelude {
        let name = match self.input.consume() {
            Some(Token::AtKeyword(name)) => name,
            _ => String::new(),
        };
        let mut prelude = Vec::new();
        loop {
            match self.input.next() {
                None | Some(Token::Semicolon) => {
                    self.input.discard();
                    break;
                }
                Some(Token::CloseBrace) => {
                    if nested {
                        break;
                    }
                    if let Some(token) = self.input.consume() {
                        prelude.push(Node::Token(token));
                    }
                }
                Some(Token::OpenBrace) => {
                    let prelude = ComponentValues { nodes: prelude };
                    return Prelude::Block(Owner::AtRule { name, prelude });
                }
                Some(_) => self.consume_component_value(&mut prelude),
            }
        }
        let rule = AtRule {
            name,
            prelude: ComponentValues { nodes: prelude },
            block: None,
        };
        Prelude::Finished(self.store_at_rule(rule, context))
    }

    fn store_at_rule<C: Context>(&mut self, rule: AtRule, context: &C) -> Outcome {
        if context.accepts_at_rule(&rule) {
            Outcome::Rule(self.store(Rule::At(rule)))
        } else {
            Outcome::Nothing
        }
    }

    /// *Consume a qualified rule* bis zum Block.
    fn consume_qualified_rule_prelude(&mut self, stop: Option<Stop>, nested: bool) -> Prelude {
        let mut prelude = Vec::new();
        loop {
            match self.input.next() {
                // Ein Parse-Fehler.
                None => return Prelude::Finished(Outcome::Nothing),
                Some(token) if stop.is_some_and(|stop| stop.matches(token)) => {
                    return Prelude::Finished(Outcome::Nothing);
                }
                Some(Token::CloseBrace) => {
                    // Ein Parse-Fehler.
                    if nested {
                        return Prelude::Finished(Outcome::Nothing);
                    }
                    if let Some(token) = self.input.consume() {
                        prelude.push(Node::Token(token));
                    }
                }
                Some(Token::OpenBrace) => {
                    let prelude = ComponentValues { nodes: prelude };
                    if !looks_like_custom_property(&prelude) {
                        return Prelude::Block(Owner::QualifiedRule { prelude });
                    }
                    if nested {
                        self.consume_bad_declaration(true);
                        return Prelude::Finished(Outcome::Nothing);
                    }
                    return Prelude::Block(Owner::Ignored { prelude });
                }
                Some(_) => self.consume_component_value(&mut prelude),
            }
        }
    }

    /// *Consume a block's contents*, bis eine verschachtelte Regel beginnt
    /// oder der Block endet.
    fn consume_block_contents<C: Context>(&mut self, frame: &mut Frame<C>) -> Step {
        loop {
            match self.input.next() {
                Some(Token::Whitespace | Token::Semicolon) => self.input.discard(),
                None | Some(Token::CloseBrace) => {
                    // Der Entwurf gibt hier nur `rules` zurück, damit gingen
                    // die letzten Deklarationen verloren (csswg-drafts
                    // #11017). Wie dort vorgeschlagen, kommen sie dazu.
                    frame.flush();
                    return Step::End;
                }
                Some(Token::AtKeyword(_)) => {
                    frame.flush();
                    return Step::Rule(Start::AtRule { nested: true });
                }
                Some(_) => {
                    self.input.mark();
                    if let Some(declaration) = self.consume_declaration(true, true, &frame.context)
                    {
                        frame.decls.push(declaration);
                        self.input.discard_mark();
                    } else {
                        self.input.restore_mark();
                        return Step::Rule(Start::QualifiedRule {
                            stop: Some(Stop::Semicolon),
                            nested: true,
                        });
                    }
                }
            }
        }
    }

    /// Die letzten Schritte von *consume an at-rule* und *consume a qualified
    /// rule*, wenn ihr Block gelesen ist.
    fn finish<C: Context>(&mut self, frame: Frame<C>, context: &C) -> Outcome {
        let Frame {
            owner,
            rules,
            stored,
            ..
        } = frame;
        let outcome = match owner {
            Owner::AtRule { name, prelude } => {
                let block = rules.into_iter().map(|entry| self.child(entry)).collect();
                let rule = AtRule {
                    name,
                    prelude,
                    block: Some(block),
                };
                self.store_at_rule(rule, context)
            }
            Owner::QualifiedRule { prelude } => {
                let mut entries = rules.into_iter().peekable();
                let declarations =
                    match entries.next_if(|entry| matches!(entry, Entry::Declarations(_))) {
                        Some(Entry::Declarations(declarations)) => declarations,
                        _ => Vec::new(),
                    };
                let rules = entries.map(|entry| self.child(entry)).collect();
                let rule = QualifiedRule {
                    prelude,
                    declarations,
                    rules,
                };
                if context.accepts_qualified_rule(&rule) {
                    Outcome::Rule(self.store(Rule::Qualified(rule)))
                } else {
                    Outcome::Invalid
                }
            }
            Owner::Ignored { .. } => Outcome::Nothing,
        };
        if !matches!(outcome, Outcome::Rule(_)) {
            self.rules.truncate(stored);
        }
        outcome
    }

    /// Ein Eintrag als Kindregel; Deklarationslisten werden eigene Regeln.
    fn child(&mut self, entry: Entry) -> RuleId {
        match entry {
            Entry::Rule(rule) => rule,
            Entry::Declarations(declarations) => self.store(Rule::Declarations(declarations)),
        }
    }

    /// *Consume a declaration*. Mit `restore` kehrt der Aufrufer nach einem
    /// Fehlschlag ohnehin zur Marke zurück. Dann darf die Deklaration
    /// abbrechen, sobald ihr Scheitern feststeht, wie der
    /// Implementierungshinweis zu *consume a block's contents* es erlaubt:
    /// Was sie sonst noch verbrauchte, würde gleich zurückgesetzt. Ohne diese
    /// Abkürzung läse jede Ebene von `a{a{a{…` den ganzen Rest.
    fn consume_declaration<C: Context>(
        &mut self,
        nested: bool,
        restore: bool,
        context: &C,
    ) -> Option<Declaration> {
        let name = match self.input.next() {
            Some(Token::Ident(_)) => match self.input.consume() {
                Some(Token::Ident(name)) => name,
                _ => return None,
            },
            _ => {
                if !restore {
                    self.consume_bad_declaration(nested);
                }
                return None;
            }
        };
        self.input.discard_whitespace();
        if self.input.next() == Some(&Token::Colon) {
            self.input.discard();
        } else {
            if !restore {
                self.consume_bad_declaration(nested);
            }
            return None;
        }
        self.input.discard_whitespace();
        let mut nodes = Vec::new();
        let start = self.input.index;
        let mut items = if restore && !is_custom_property_name(&name) {
            self.consume_list_while(
                Some(Stop::Semicolon),
                nested,
                &mut nodes,
                block_value_guard(),
            )?
        } else {
            self.consume_list(Some(Stop::Semicolon), nested, &mut nodes)
        };
        let tokens = start..self.input.index;

        let mut important = false;
        let significant: Vec<usize> = (0..items.len())
            .filter(|&index| !is_whitespace_item(&nodes, &items, index))
            .collect();
        if let [.., bang, ident] = significant[..]
            && token_at(&nodes, &items, bang) == Some(&Token::Delim('!'))
            && matches!(token_at(&nodes, &items, ident),
                Some(Token::Ident(value)) if value.eq_ignore_ascii_case("important"))
        {
            remove_item(&mut nodes, &mut items, ident);
            remove_item(&mut nodes, &mut items, bang);
            important = true;
        }
        while let Some(last) = items.len().checked_sub(1)
            && is_whitespace_item(&nodes, &items, last)
        {
            remove_item(&mut nodes, &mut items, last);
        }

        let mut original_text = None;
        if is_custom_property_name(&name) {
            let text = match (items.first(), items.last()) {
                (Some(first), Some(last)) => self.input.text(first.tokens.start..last.tokens.end),
                _ => &[],
            };
            original_text = Some(text.iter().collect());
        } else if has_brace_block_with_others(&nodes, &items) {
            return None;
        } else if name.eq_ignore_ascii_case("unicode-range") {
            nodes = consume_unicode_range_value(self.input.text(tokens)).nodes;
        }

        let declaration = Declaration {
            name,
            value: ComponentValues { nodes },
            important,
            original_text,
        };
        context
            .accepts_declaration(&declaration)
            .then_some(declaration)
    }

    /// *Consume the remnants of a bad declaration*
    fn consume_bad_declaration(&mut self, nested: bool) {
        let mut ignored = Vec::new();
        loop {
            match self.input.next() {
                None | Some(Token::Semicolon) => {
                    self.input.discard();
                    return;
                }
                Some(Token::CloseBrace) => {
                    if nested {
                        return;
                    }
                    self.input.discard();
                }
                Some(_) => {
                    ignored.clear();
                    self.consume_component_value(&mut ignored);
                }
            }
        }
    }

    /// *Consume a list of component values* nach `out`. Liefert die Werte
    /// der obersten Ebene.
    fn consume_list(&mut self, stop: Option<Stop>, nested: bool, out: &mut Vec<Node>) -> Vec<Item> {
        self.consume_list_while(stop, nested, out, |_| true)
            .unwrap_or_default()
    }

    /// Wie [`Parser::consume_list`], bricht aber mit `None` ab, sobald
    /// `proceed` das erste Token eines Werts ablehnt.
    fn consume_list_while(
        &mut self,
        stop: Option<Stop>,
        nested: bool,
        out: &mut Vec<Node>,
        mut proceed: impl FnMut(&Token) -> bool,
    ) -> Option<Vec<Item>> {
        let mut items = Vec::new();
        loop {
            let node = out.len();
            let start = self.input.index;
            match self.input.next() {
                None => return Some(items),
                Some(token) if stop.is_some_and(|stop| stop.matches(token)) => return Some(items),
                Some(Token::CloseBrace) if nested => return Some(items),
                Some(token) if !proceed(token) => return None,
                Some(Token::CloseBrace) => {
                    // Ein Parse-Fehler.
                    if let Some(token) = self.input.consume() {
                        out.push(Node::Token(token));
                    }
                }
                Some(_) => self.consume_component_value(out),
            }
            items.push(Item {
                node,
                tokens: start..self.input.index,
            });
        }
    }

    /// *Consume a component value* nach `out`, mit *consume a simple block*
    /// und *consume a function*. Offene Blöcke und Funktionen liegen auf
    /// einem eigenen Stapel.
    fn consume_component_value(&mut self, out: &mut Vec<Node>) {
        // Je offenem Wert sein Knoten und die Klammer, die ihn schließt.
        let mut open: Vec<(usize, BlockKind)> = Vec::new();
        loop {
            if let Some(&(node, closing)) = open.last()
                && self.input.next().is_none_or(|token| closes(closing, token))
            {
                self.input.discard();
                let len = out.len().saturating_sub(node).saturating_sub(1);
                if let Some(Node::Function { len: slot, .. } | Node::Block { len: slot, .. }) =
                    out.get_mut(node)
                {
                    *slot = len;
                }
                open.pop();
                if open.is_empty() {
                    return;
                }
                continue;
            }
            match self.input.next() {
                Some(Token::OpenBrace | Token::OpenBracket | Token::OpenParenthesis) => {
                    let kind = match self.input.consume() {
                        Some(Token::OpenBrace) => BlockKind::Brace,
                        Some(Token::OpenBracket) => BlockKind::Bracket,
                        _ => BlockKind::Parenthesis,
                    };
                    open.push((out.len(), kind));
                    out.push(Node::Block { kind, len: 0 });
                }
                Some(Token::Function(_)) => {
                    if let Some(Token::Function(name)) = self.input.consume() {
                        open.push((out.len(), BlockKind::Parenthesis));
                        out.push(Node::Function { name, len: 0 });
                    }
                }
                Some(_) => {
                    if let Some(token) = self.input.consume() {
                        out.push(Node::Token(token));
                    }
                }
                None => {}
            }
            if open.is_empty() {
                return;
            }
        }
    }
}

/// *Consume the value of a unicode-range descriptor*: Den Text noch einmal
/// zerlegen, diesmal mit ⟨unicode-range-token⟩s.
pub(crate) fn consume_unicode_range_value(text: &[char]) -> ComponentValues {
    let mut parser = Parser::new(text, true);
    let mut nodes = Vec::new();
    parser.consume_list(None, false, &mut nodes);
    ComponentValues { nodes }
}

/// Prüft die ersten Tokens der Werte einer Deklaration, die keine Custom
/// Property ist. Ein {}-Block neben einem anderen Wert außer Leerraum macht
/// sie ungültig, und nach dem Entfernen von `!important` steht das schon
/// fest, wenn ein „{“ nach einem anderen Wert kommt oder nach dem Block
/// etwas anderes als `!` und dann `important` folgt.
fn block_value_guard() -> impl FnMut(&Token) -> bool {
    let mut significant = false;
    // Wie viel von „{} ! important“ schon gelesen ist.
    let mut after_block = None;
    move |token| {
        if *token == Token::Whitespace {
            return true;
        }
        match after_block {
            None => {
                if *token == Token::OpenBrace {
                    if significant {
                        return false;
                    }
                    after_block = Some(0);
                }
                significant = true;
                true
            }
            Some(0) if *token == Token::Delim('!') => {
                after_block = Some(1);
                true
            }
            Some(1) if matches!(token, Token::Ident(value) if value.eq_ignore_ascii_case("important")) =>
            {
                after_block = Some(2);
                true
            }
            Some(_) => false,
        }
    }
}

/// Ob `token` einen Wert mit dieser Klammer schließt (*ending token*).
fn closes(kind: BlockKind, token: &Token) -> bool {
    matches!(
        (kind, token),
        (BlockKind::Brace, Token::CloseBrace)
            | (BlockKind::Bracket, Token::CloseBracket)
            | (BlockKind::Parenthesis, Token::CloseParenthesis)
    )
}

/// *Custom property name string* (CSS Typed OM): beginnt mit zwei Bindestrichen.
fn is_custom_property_name(name: &str) -> bool {
    name.starts_with("--")
}

/// Die Prüfung in *consume a qualified rule*: Sind die ersten zwei Werte
/// außer Leerraum ein ⟨ident-token⟩ mit „--“ am Anfang und ein Doppelpunkt?
fn looks_like_custom_property(prelude: &ComponentValues) -> bool {
    let mut values = prelude
        .iter()
        .filter(|value| !matches!(value, ComponentValue::Token(Token::Whitespace)));
    matches!(values.next(),
        Some(ComponentValue::Token(Token::Ident(name))) if name.starts_with("--"))
        && matches!(values.next(), Some(ComponentValue::Token(Token::Colon)))
}

fn token_at<'n>(nodes: &'n [Node], items: &[Item], index: usize) -> Option<&'n Token> {
    match nodes.get(items.get(index)?.node)? {
        Node::Token(token) => Some(token),
        _ => None,
    }
}

fn is_whitespace_item(nodes: &[Node], items: &[Item], index: usize) -> bool {
    token_at(nodes, items, index) == Some(&Token::Whitespace)
}

/// Entfernt den Wert `index` der obersten Ebene samt seinen Knoten.
fn remove_item(nodes: &mut Vec<Node>, items: &mut Vec<Item>, index: usize) {
    let Some(item) = items.get(index) else {
        return;
    };
    let start = item.node;
    let end = items.get(index + 1).map_or(nodes.len(), |next| next.node);
    if start > end || end > nodes.len() {
        return;
    }
    nodes.drain(start..end);
    items.remove(index);
    for later in items.iter_mut().skip(index) {
        later.node = later.node.saturating_sub(end - start);
    }
}

/// Enthält der Wert einen {}-Block auf oberster Ebene und daneben noch
/// irgendeinen Wert außer Leerraum?
fn has_brace_block_with_others(nodes: &[Node], items: &[Item]) -> bool {
    let brace = items.iter().any(|item| {
        matches!(
            nodes.get(item.node),
            Some(Node::Block {
                kind: BlockKind::Brace,
                ..
            })
        )
    });
    let significant = (0..items.len())
        .filter(|&index| !is_whitespace_item(nodes, items, index))
        .count();
    brace && significant > 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kurzschreibweise für Regeln und Werte, nur für Tests.
    fn describe(tree: &RuleTree) -> String {
        tree.roots()
            .iter()
            .map(|&rule| describe_rule(tree, rule))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn describe_rule(tree: &RuleTree, rule: RuleId) -> String {
        let children = |rules: &[RuleId]| {
            rules
                .iter()
                .map(|&rule| describe_rule(tree, rule))
                .collect::<String>()
        };
        match tree.get(rule) {
            Some(Rule::At(rule)) => {
                let block = match &rule.block {
                    Some(block) => format!("{{{}}}", children(block)),
                    None => ";".to_owned(),
                };
                format!(
                    "@{}{}{block}",
                    rule.name,
                    describe_values(rule.prelude.values())
                )
            }
            Some(Rule::Qualified(rule)) => format!(
                "{}{{{}{}}}",
                describe_values(rule.prelude.values()),
                describe_declarations(&rule.declarations),
                children(&rule.rules)
            ),
            Some(Rule::Declarations(declarations)) => {
                format!("[{}]", describe_declarations(declarations))
            }
            None => "?".to_owned(),
        }
    }

    fn describe_declarations(declarations: &[Declaration]) -> String {
        declarations
            .iter()
            .map(|declaration| {
                let important = if declaration.important { "!" } else { "" };
                let value = describe_values(declaration.value.values());
                format!("{}:{value}{important};", declaration.name)
            })
            .collect()
    }

    fn describe_values(values: Values<'_>) -> String {
        values
            .iter()
            .map(|value| match value {
                ComponentValue::Token(token) => describe_token(token),
                ComponentValue::Function { name, value } => {
                    format!("{name}({})", describe_values(value))
                }
                ComponentValue::Block { kind, value } => {
                    let (open, close) = match kind {
                        BlockKind::Brace => ('{', '}'),
                        BlockKind::Bracket => ('[', ']'),
                        BlockKind::Parenthesis => ('(', ')'),
                    };
                    format!("{open}{}{close}", describe_values(value))
                }
            })
            .collect()
    }

    fn describe_token(token: &Token) -> String {
        match token {
            Token::Ident(value) => value.clone(),
            Token::Whitespace => " ".to_owned(),
            Token::Colon => ":".to_owned(),
            Token::Semicolon => ";".to_owned(),
            Token::Comma => ",".to_owned(),
            Token::Delim(value) => value.to_string(),
            Token::Number { value, .. } => value.to_string(),
            Token::UnicodeRange { start, end } => format!("U+{start:X}-{end:X}"),
            Token::CloseBrace => "}".to_owned(),
            other => format!("{other:?}"),
        }
    }

    fn stylesheet(text: &str) -> String {
        describe(&parse_stylesheet(&Source::new(text), &Syntax))
    }

    #[test]
    fn rules_and_nested_declarations() {
        assert_eq!(
            stylesheet("a { color: red; b { c: d } e: f } @media x { g {} h: i } @import y;"),
            "a {color:red;b {c:d;}[e:f;]} @media x {g {}[h:i;]} @import y;"
        );
        // Ohne den Schritt aus csswg-drafts #11017 fehlte hier `color`.
        assert_eq!(stylesheet("a{color:red}"), "a{color:red;}");
        assert_eq!(stylesheet("<!-- a{} -->"), "a{}");
    }

    #[test]
    fn declarations_then_rules() {
        // `foo: {} bar` ist keine Deklaration, also wird ab `foo` eine Regel
        // gelesen. `bar;` ist weder Deklaration noch Regel.
        assert_eq!(stylesheet("a { foo: {} bar; x: y }"), "a {foo: {}[x:y;]}");
        // Ein {}-Block als ganzer Wert bleibt eine Deklaration.
        assert_eq!(stylesheet("a { foo: {} !important }"), "a {foo:{}!;}");
        // Wie eine Custom Property: auf oberster Ebene samt Block verworfen.
        assert_eq!(stylesheet("--x:hover { a: b } c {}"), "c {}");
        assert_eq!(stylesheet("a { b } c {}"), "a {} c {}");
        assert_eq!(
            stylesheet("a { @x y } b: c } d {}"),
            "a {@x y ;} b: c } d {}"
        );
    }

    #[test]
    fn important_and_original_text() {
        let declaration = parse_declaration(
            &Source::new("--x: { a } /* c */ b  ! /**/ IMPORTANT  "),
            &Syntax,
        )
        .expect("eine Deklaration");
        assert!(declaration.important);
        assert_eq!(
            declaration.original_text.as_deref(),
            Some("{ a } /* c */ b")
        );
        assert_eq!(describe_values(declaration.value.values()), "{ a }  b");

        let declaration =
            parse_declaration(&Source::new("color : red !important!"), &Syntax).expect("eine");
        assert!(!declaration.important);
        assert_eq!(declaration.original_text, None);
        assert_eq!(
            parse_declaration(&Source::new("foo:;"), &Syntax),
            Err(SyntaxError)
        );
        assert_eq!(
            parse_declaration(&Source::new("  "), &Syntax),
            Err(SyntaxError)
        );
    }

    #[test]
    fn unicode_range_descriptor() {
        let tree = parse_stylesheet(
            &Source::new("@font-face { unicode-range: U+0-7F, u+4??; foo: u+1 }"),
            &Syntax,
        );
        assert_eq!(
            describe(&tree),
            "@font-face {[unicode-range:U+0-7F, U+400-4FF;foo:u1;]}"
        );
    }

    #[test]
    fn single_results() {
        let rule = |text| parse_rule(&Source::new(text), &Syntax).map(|tree| describe(&tree));
        assert_eq!(rule(" a{} "), Ok("a{}".to_owned()));
        assert_eq!(rule("@a b"), Ok("@a b;".to_owned()));
        assert_eq!(rule("a{} b{}"), Err(SyntaxError));
        assert_eq!(rule("a"), Err(SyntaxError));
        assert_eq!(rule(""), Err(SyntaxError));

        let value =
            |text| parse_component_value(&Source::new(text)).map(|v| describe_values(v.values()));
        assert_eq!(value(" f(a, [b]) "), Ok("f(a, [b])".to_owned()));
        assert_eq!(value("a b"), Err(SyntaxError));
        assert_eq!(value("/**/"), Err(SyntaxError));
    }

    #[test]
    fn lists() {
        let declarations = parse_declaration_list(&Source::new("a:b; @x { y: z } c:d; e"), &Syntax);
        assert_eq!(describe_declarations(&declarations), "a:b;c:d;");

        let groups = parse_comma_separated_component_value_list(&Source::new("a, b c ,d,"));
        let groups: Vec<String> = groups
            .iter()
            .map(|group| describe_values(group.values()))
            .collect();
        assert_eq!(groups, ["a", " b c ", "d"]);

        let values = parse_component_value_list(&Source::new("a } (b"));
        assert_eq!(describe_values(values.values()), "a } (b)");
    }

    /// Ein Kontext, der Regeln mit dem Prelude `x` und At-Regeln namens
    /// `@bad` ablehnt, außerdem Deklarationen namens `foo`.
    struct Picky;

    impl Context for Picky {
        fn accepts_declaration(&self, declaration: &Declaration) -> bool {
            declaration.name != "foo"
        }

        fn accepts_at_rule(&self, rule: &AtRule) -> bool {
            rule.name != "bad"
        }

        fn accepts_qualified_rule(&self, rule: &QualifiedRule) -> bool {
            describe_values(rule.prelude.values()).trim() != "x"
        }

        fn at_rule_block(&self, _: &str, _: &ComponentValues) -> Picky {
            Picky
        }

        fn qualified_rule_block(&self, _: &ComponentValues) -> Picky {
            Picky
        }
    }

    #[test]
    fn context_decides_validity() {
        let tree = parse_stylesheet(
            &Source::new("a { b: c; x { y {} } d: e; @bad { f {} } g: h } x {} @bad;"),
            &Picky,
        );
        // Die ungültige Regel `x` trennt die Deklarationen trotzdem.
        assert_eq!(describe(&tree), "a {b:c;[d:e;][g:h;]}");
        // Was zu verworfenen Regeln gehörte, ist nicht mehr gespeichert.
        assert_eq!(tree.rules.len(), 3);

        // Ungültig, also noch einmal als Regel gelesen.
        let tree = parse_stylesheet(&Source::new("a { foo: {}; b: c }"), &Picky);
        assert_eq!(describe(&tree), "a {foo: {}[b:c;]}");
    }

    #[test]
    fn deep_nesting_needs_no_recursion() {
        const DEPTH: usize = 100_000;
        let values = parse_component_value_list(&Source::new(&"([{f(".repeat(DEPTH)));
        let mut depth = 0;
        let mut current = values.values();
        while let Some(
            ComponentValue::Block { value, .. } | ComponentValue::Function { value, .. },
        ) = current.iter().next()
        {
            depth += 1;
            current = value;
        }
        assert_eq!(depth, 4 * DEPTH);

        // Auch ohne die frühen Ausstiege in `consume_declaration` liefe
        // nichts über, aber jede Ebene läse den ganzen Rest noch einmal.
        for text in ["a{", "b:c d{"] {
            let tree = parse_stylesheet(&Source::new(&text.repeat(DEPTH)), &Syntax);
            let mut depth = 0;
            let mut current = tree.roots().first().copied();
            while let Some(Rule::Qualified(rule)) = current.and_then(|rule| tree.get(rule)) {
                depth += 1;
                current = rule.rules.first().copied();
            }
            assert_eq!(depth, DEPTH, "{text}");
        }
    }
}
