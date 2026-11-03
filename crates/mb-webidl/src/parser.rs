//! Iterativer LL(1)-Parser nach der generierten normativen Grammatik.
//! Liefert einen Arena-Syntaxbaum, noch keine Namens-/Typprüfung oder Bindings.

use std::ops::Range;

pub use mb_base::{Budget, Cancel, Exhausted};

use crate::lexer::{Kind as TokenKind, Lexer, Token};

mod tables;

const NT: u16 = 0x8000;
pub const DEFAULT_STEPS: u64 = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub source_bytes: usize,
    pub nodes: usize,
    pub pending: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: 1 << 20,
            nodes: 200_000,
            pending: 200_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    SourceBytes,
    Nodes,
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Erwartetes Terminal oder die Produktion ohne passende Alternative.
    Syntax {
        expected: &'static str,
    },
    Limit(Limit),
    Exhausted(Exhausted),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub span: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Production(&'static str),
    Token(TokenKind),
}

#[derive(Debug)]
pub struct Node {
    pub kind: Kind,
    pub span: Range<usize>,
    children: Range<u32>,
}

/// Die Quelle bleibt geliehen. Nur signifikante Tokens erhalten Blätter;
/// Trivia bleibt unverändert in der Quelle. Produktions-Ranges schließen
/// folgende Trivia bis zum nächsten signifikanten Token ein.
#[derive(Debug)]
pub struct Document<'a> {
    source: &'a str,
    nodes: Vec<Node>,
}

impl<'a> Document<'a> {
    pub fn source(&self) -> &'a str {
        self.source
    }
    pub const fn root(&self) -> NodeId {
        NodeId(0)
    }
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0 as usize)
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn children(&self, id: NodeId) -> impl DoubleEndedIterator<Item = NodeId> + '_ {
        self.node(id)
            .map_or(0..0, |node| node.children.clone())
            .map(NodeId)
    }
    pub fn text(&self, id: NodeId) -> Option<&'a str> {
        self.source.get(self.node(id)?.span.clone())
    }
}

enum Task {
    Read(u16, NodeId),
    Close(NodeId),
}

pub fn parse(source: &str) -> Result<Document<'_>, Error> {
    parse_within(source, Limits::default(), &Budget::new(DEFAULT_STEPS))
}

/// Die volle Bytezahl der Quelle wird vorab aus dem Budget verbraucht,
/// auch bei einem frühen
/// Syntaxfehler. Zusätzlich kostet jede Parseraktion einen Schritt.
/// Abbruch wird vor dem ersten Scan und an jeder Aktion geprüft. Ein
/// einzelner Lexer-Aufruf wird nicht mitten im Token unterbrochen.
pub fn parse_within<'a>(
    source: &'a str,
    limits: Limits,
    budget: &Budget,
) -> Result<Document<'a>, Error> {
    let error = |kind| Error { kind, span: 0..0 };
    spend(budget, 0).map_err(error)?;
    if source.len() > limits.source_bytes {
        return Err(error(ErrorKind::Limit(Limit::SourceBytes)));
    }
    spend(budget, source.len() as u64).map_err(error)?;
    if limits.nodes == 0 {
        return Err(error(ErrorKind::Limit(Limit::Nodes)));
    }
    if limits.pending == 0 {
        return Err(error(ErrorKind::Limit(Limit::Pending)));
    }
    let mut lexer = Lexer::new(source);
    let mut look = next(&mut lexer, budget, 0)?;
    let mut document = Document {
        source,
        nodes: vec![node(tables::START, look.span.start)],
    };
    let mut pending = vec![Task::Read(tables::START, document.root())];
    while let Some(task) = pending.pop() {
        let error = |kind| Error {
            kind,
            span: look.span.clone(),
        };
        spend(budget, 1).map_err(error)?;
        match task {
            Task::Close(id) => document.nodes[id.0 as usize].span.end = look.span.start,
            Task::Read(symbol, id) if symbol & NT != 0 => {
                document.nodes[id.0 as usize].span.start = look.span.start;
                let production = usize::from(symbol & !NT);
                let row = &tables::ENTRIES[tables::ROWS[production]..tables::ROWS[production + 1]];
                let terminal = terminal(&look);
                let rule = terminal
                    .and_then(|terminal| row.binary_search_by_key(&terminal, |&(key, _)| key).ok())
                    .map(|at| row[at].1)
                    .ok_or_else(|| {
                        error(ErrorKind::Syntax {
                            expected: tables::NAMES[production],
                        })
                    })?;
                let (start, count) = tables::RULES[usize::from(rule)];
                let rhs = &tables::SYMBOLS[usize::from(start)..usize::from(start + count)];
                let end = document
                    .nodes
                    .len()
                    .checked_add(rhs.len())
                    .filter(|&end| end <= limits.nodes && end <= u32::MAX as usize)
                    .ok_or_else(|| error(ErrorKind::Limit(Limit::Nodes)))?;
                if pending
                    .len()
                    .checked_add(rhs.len() + 1)
                    .is_none_or(|size| size > limits.pending)
                {
                    return Err(error(ErrorKind::Limit(Limit::Pending)));
                }
                let first = document.nodes.len() as u32;
                document.nodes[id.0 as usize].children = first..end as u32;
                document
                    .nodes
                    .extend(rhs.iter().map(|&symbol| node(symbol, look.span.start)));
                pending.push(Task::Close(id));
                for (at, &symbol) in rhs.iter().enumerate().rev() {
                    pending.push(Task::Read(symbol, NodeId(first + at as u32)));
                }
            }
            Task::Read(symbol, id) => {
                if terminal(&look) != Some(symbol) {
                    return Err(error(ErrorKind::Syntax {
                        expected: tables::TERMINALS[usize::from(symbol)],
                    }));
                }
                let node = &mut document.nodes[id.0 as usize];
                node.kind = Kind::Token(look.kind);
                node.span = look.span.clone();
                look = next(&mut lexer, budget, look.span.end)?;
            }
        }
    }
    if look.kind != TokenKind::End {
        return Err(Error {
            kind: ErrorKind::Syntax { expected: "$" },
            span: look.span,
        });
    }
    document.nodes[0].span = 0..source.len();
    Ok(document)
}

fn spend(budget: &Budget, steps: u64) -> Result<(), ErrorKind> {
    if budget.spend(steps) {
        Ok(())
    } else {
        Err(ErrorKind::Exhausted(
            budget.exhausted().expect("abgelehnte Quote"),
        ))
    }
}

fn next<'a>(
    lexer: &mut Lexer<'a>,
    budget: &Budget,
    mut position: usize,
) -> Result<Token<'a>, Error> {
    loop {
        spend(budget, 0).map_err(|kind| Error {
            kind,
            span: position..position,
        })?;
        let token = lexer.next_token();
        if !token.kind.is_trivia() {
            return Ok(token);
        }
        position = token.span.end;
    }
}

fn terminal(token: &Token<'_>) -> Option<u16> {
    let name = match token.kind {
        TokenKind::Integer => "integer",
        TokenKind::Decimal => "decimal",
        TokenKind::Identifier => "identifier",
        TokenKind::String => "string",
        TokenKind::Other => "other",
        TokenKind::End => "$",
        TokenKind::Terminal => token.text,
        TokenKind::Whitespace | TokenKind::Comment => return None,
    };
    tables::TERMINALS
        .binary_search(&name)
        .ok()
        .and_then(|at| u16::try_from(at).ok())
}

fn node(symbol: u16, position: usize) -> Node {
    let kind = if symbol & NT != 0 {
        Kind::Production(tables::NAMES[usize::from(symbol & !NT)])
    } else {
        Kind::Token(TokenKind::End)
    };
    Node {
        kind,
        span: position..position,
        children: 0..0,
    }
}

#[cfg(test)]
mod tests;
