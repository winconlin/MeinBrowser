//! Strukturierte Member und erste semantische Prüfungen auf dem Namensindex.
//!
//! Typen, Werte und Extended Attributes bleiben Quellknoten. Diese Stufe ist
//! keine vollständige WebIDL-Validierung und erzeugt keine Sprachbindungen.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::index::{DefinitionId, DefinitionKind, FragmentId, Index, Location};
use crate::lexer;
use crate::parser::{Document, Kind as NodeKind, NodeId};
pub use mb_base::{Budget, Cancel, Exhausted};

mod check;
mod collect;

pub const DEFAULT_STEPS: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Definitionen, Member, Argumente und Enumwerte zusammen.
    pub records: usize,
    /// Besuche bei der Prüfung wirksamer Member, auch durch Includes.
    pub visits: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            records: 200_000,
            visits: 1_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Records,
    Visits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Limit(Limit),
    Exhausted(Exhausted),
    ReservedIdentifier,
    DuplicateMember,
    DuplicateDictionaryMember,
    DuplicateArgument,
    DuplicateEnumValue,
    MissingOperationName,
    VariadicNotLast,
    CrossFragmentOverload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub location: Option<Location>,
    pub previous: Option<Location>,
}

#[derive(Clone, Debug)]
pub struct Identifier<'a> {
    pub name: &'a str,
    pub location: Location,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    Regular,
    Getter,
    Setter,
    Deleter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Constant,
    Attribute,
    Operation(Special),
    Constructor,
    DictionaryMember,
    Iterable,
    AsyncIterable,
    Maplike,
    Setlike,
    Stringifier,
}

#[derive(Debug)]
pub struct Signature {
    pub result: Option<NodeId>,
    arguments: Range<usize>,
}

#[derive(Debug)]
pub struct Argument<'a> {
    pub identifier: Identifier<'a>,
    pub ty: NodeId,
    pub attributes: NodeId,
    /// Explizites optional oder implizit optional durch ... .
    pub optional: bool,
    pub variadic: bool,
    pub default: Option<NodeId>,
}

#[derive(Debug)]
pub struct Member<'a> {
    pub fragment: FragmentId,
    pub location: Location,
    pub kind: Kind,
    pub identifier: Option<Identifier<'a>>,
    pub attributes: NodeId,
    /// Typ-/TypeWithExtendedAttributes-/ConstType-Knoten; bei Collections bis zwei.
    pub types: [Option<NodeId>; 2],
    pub signature: Option<Signature>,
    pub is_static: bool,
    pub readonly: bool,
    pub inherit: bool,
    pub stringifier: bool,
    pub required: bool,
    pub default: Option<NodeId>,
    pub constant: Option<NodeId>,
}

#[derive(Debug)]
pub struct EnumValue<'a> {
    pub value: &'a str,
    pub location: Location,
}

#[derive(Debug)]
struct Definition {
    members: Range<usize>,
    values: Range<usize>,
    callback: Option<Signature>,
}

/// Flache Beschreibung mit Verweisen auf den ursprünglichen Namensindex.
/// IDs und Quellknoten gehören nur zum Index, für den sie erzeugt wurde.
#[derive(Debug)]
pub struct Members<'a> {
    definitions: BTreeMap<DefinitionId, Definition>,
    members: Vec<Member<'a>>,
    arguments: Vec<Argument<'a>>,
    values: Vec<EnumValue<'a>>,
}

impl<'a> Members<'a> {
    pub fn members(&self, id: DefinitionId) -> Option<&[Member<'a>]> {
        self.members.get(self.definitions.get(&id)?.members.clone())
    }
    pub fn values(&self, id: DefinitionId) -> Option<&[EnumValue<'a>]> {
        self.values.get(self.definitions.get(&id)?.values.clone())
    }
    pub fn callback(&self, id: DefinitionId) -> Option<&Signature> {
        self.definitions.get(&id)?.callback.as_ref()
    }
    pub fn arguments(&self, signature: &Signature) -> Option<&[Argument<'a>]> {
        self.arguments.get(signature.arguments.clone())
    }
    pub fn len(&self) -> usize {
        self.members.len()
    }
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
    pub fn argument_count(&self) -> usize {
        self.arguments.len()
    }
    pub fn value_count(&self) -> usize {
        self.values.len()
    }
}

pub fn build<'a>(index: &Index<'a>) -> Result<Members<'a>, Error> {
    build_within(index, Limits::default(), &Budget::new(DEFAULT_STEPS))
}

/// Teilen sich Indexaufbau und diese Stufe dasselbe Budget, wird dessen Rest
/// weiterverwendet. Es wird kein ungeprüftes Teilergebnis zurückgegeben.
pub fn build_within<'a>(
    index: &Index<'a>,
    limits: Limits,
    budget: &Budget,
) -> Result<Members<'a>, Error> {
    let mut work = Work {
        limits,
        budget,
        records: 0,
        visits: 0,
    };
    work.spend(0, None)?;
    let model = collect::collect(index, &mut work)?;
    check::check(index, &model, &mut work)?;
    Ok(model)
}

fn error(kind: ErrorKind, at: Option<&Location>) -> Error {
    Error {
        kind,
        location: at.cloned(),
        previous: None,
    }
}

fn duplicate(kind: ErrorKind, at: &Location, previous: &Location) -> Error {
    Error {
        kind,
        location: Some(at.clone()),
        previous: Some(previous.clone()),
    }
}

struct Work<'b> {
    limits: Limits,
    budget: &'b Budget,
    records: usize,
    visits: usize,
}

impl Work<'_> {
    fn spend(&self, steps: u64, at: Option<&Location>) -> Result<(), Error> {
        if self.budget.spend(steps) {
            Ok(())
        } else {
            Err(error(
                ErrorKind::Exhausted(self.budget.exhausted().expect("abgelehnte Quote")),
                at,
            ))
        }
    }
    fn record(&mut self, at: &Location) -> Result<(), Error> {
        self.spend(1, Some(at))?;
        if self.records >= self.limits.records {
            return Err(error(ErrorKind::Limit(Limit::Records), Some(at)));
        }
        self.records += 1;
        Ok(())
    }
    fn visit(&mut self, at: &Location) -> Result<(), Error> {
        self.spend(1, Some(at))?;
        if self.visits >= self.limits.visits {
            return Err(error(ErrorKind::Limit(Limit::Visits), Some(at)));
        }
        self.visits += 1;
        Ok(())
    }
}

fn production(document: &Document<'_>, id: NodeId) -> Option<&'static str> {
    match document.node(id).expect("eigener Knoten").kind {
        NodeKind::Production(name) => Some(name),
        _ => None,
    }
}
fn child(document: &Document<'_>, id: NodeId, name: &str) -> Option<NodeId> {
    document
        .children(id)
        .find(|&id| production(document, id) == Some(name))
}
fn identifier(document: &Document<'_>, id: NodeId) -> Option<NodeId> {
    document.children(id).find(|&id| {
        document.node(id).expect("Kind").kind == NodeKind::Token(lexer::Kind::Identifier)
    })
}
fn location(document: &Document<'_>, source: usize, id: NodeId) -> Location {
    Location {
        source,
        node: Some(id),
        span: document.node(id).expect("Quellknoten").span.clone(),
    }
}
fn named<'a>(document: &Document<'a>, source: usize, id: NodeId) -> Identifier<'a> {
    let raw = document.text(id).expect("Bezeichner");
    Identifier {
        name: raw.strip_prefix('_').unwrap_or(raw),
        location: location(document, source, id),
    }
}
fn terminal(document: &Document<'_>, id: NodeId, text: &str) -> bool {
    document.children(id).any(|id| {
        matches!(document.node(id).expect("Kind").kind, NodeKind::Token(_))
            && document.text(id) == Some(text)
    })
}
fn token_in_name(document: &Document<'_>, id: NodeId) -> NodeId {
    let mut id = id;
    while production(document, id).is_some() {
        id = document.children(id).next().expect("nichtleerer Name");
    }
    id
}

#[cfg(test)]
mod tests;
