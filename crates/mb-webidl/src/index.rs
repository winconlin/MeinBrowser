//! Quellenübergreifender Namensindex für WebIDL, noch kein Binding-Modell.
//!
//! Partials, Includes, Vererbung und benannte Typverweise werden aufgelöst.
//! Die Syntaxbäume bleiben erhalten: Attribute, Member, Defaults und Typformen
//! werden hier weder verworfen noch als vollständig semantisch geprüft ausgegeben.

use std::collections::BTreeMap;
use std::ops::Range;

pub use mb_base::{Budget, Cancel, Exhausted};

use crate::lexer;
use crate::parser::{self, Document, Kind, NodeId};

mod collect;
mod link;

pub const DEFAULT_STEPS: u64 = 25_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub sources: usize,
    pub source_bytes: usize,
    pub nodes: usize,
    /// Definitionen, Fragmente, Typverweise und Includes zusammen.
    pub records: usize,
    /// Zusätzlich gelten diese Grenzen für jedes einzelne Quelldokument.
    pub parser: parser::Limits,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            sources: 4096,
            source_bytes: 16 << 20,
            nodes: 1_000_000,
            records: 200_000,
            parser: parser::Limits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Sources,
    SourceBytes,
    Nodes,
    Records,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Parser(parser::ErrorKind),
    Limit(Limit),
    Exhausted(Exhausted),
    ReservedIdentifier,
    DuplicateDefinition,
    MissingDefinition,
    PartialKind,
    UnknownType,
    NotAType,
    InvalidParent,
    InvalidIncludes,
    InheritanceCycle,
    TypedefCycle,
    /// Der unmittelbar benannte Typ eines typedef darf kein typedef sein.
    TypedefAlias,
}

/// Die Quellen sind in der Reihenfolge des Aufrufs nummeriert. `node` gehört
/// ausschließlich zum Syntaxbaum dieser Quelle; `span` zählt UTF-8-Bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub source: usize,
    /// Bei einem Parsefehler wurde noch kein Syntaxbaum erzeugt.
    pub node: Option<NodeId>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    /// Bei einem globalen Limit oder Abbruch vor dem Parsen noch ohne Knoten.
    pub location: Option<Location>,
    /// Etwa die zuerst deklarierte Definition bei einer Namenskollision.
    pub previous: Option<Location>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DefinitionId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FragmentId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefinitionKind {
    Interface,
    Mixin,
    CallbackInterface,
    Namespace,
    Dictionary,
    Enum,
    Callback,
    Typedef,
}

#[derive(Debug)]
pub struct Definition<'a> {
    pub name: &'a str,
    pub kind: DefinitionKind,
    pub location: Location,
    /// Grunddefinition und Partials in ihrer Quellreihenfolge, ohne Kopien.
    pub fragments: Vec<FragmentId>,
    pub parent: Option<DefinitionId>,
    /// Reihenfolge der Includes-Anweisungen. Member werden nicht vervielfacht.
    pub mixins: Vec<DefinitionId>,
}

#[derive(Debug)]
pub struct Fragment {
    pub definition: DefinitionId,
    pub partial: bool,
    pub location: Location,
    /// Konkrete Grammatikproduktion, etwa InterfaceRest oder Typedef.
    pub body: NodeId,
    /// ExtendedAttributeList vor der Definition, auch wenn sie leer ist.
    pub attributes: NodeId,
}

#[derive(Debug)]
pub struct TypeReference<'a> {
    pub name: &'a str,
    pub location: Location,
    pub fragment: FragmentId,
    pub target: DefinitionId,
}

#[derive(Debug)]
pub struct Include {
    pub location: Location,
    pub interface: DefinitionId,
    pub mixin: DefinitionId,
}

/// Aufgelöste Namen sind keine Zusage vollständiger WebIDL-Gültigkeit.
/// Insbesondere fehlen noch Member-/Overload-, Typform-, Defaultwert- und
/// Extended-Attribute-Prüfung sowie Sprachbindungen.
#[derive(Debug)]
pub struct Index<'a> {
    documents: Vec<Document<'a>>,
    definitions: Vec<Definition<'a>>,
    fragments: Vec<Fragment>,
    references: Vec<TypeReference<'a>>,
    includes: Vec<Include>,
    named: BTreeMap<&'a str, DefinitionId>,
    mixins: BTreeMap<&'a str, DefinitionId>,
}

impl<'a> Index<'a> {
    pub fn document(&self, source: usize) -> Option<&Document<'a>> {
        self.documents.get(source)
    }
    pub fn sources(&self) -> usize {
        self.documents.len()
    }
    pub fn definitions(&self) -> impl ExactSizeIterator<Item = (DefinitionId, &Definition<'a>)> {
        self.definitions
            .iter()
            .enumerate()
            .map(|(i, d)| (DefinitionId(i), d))
    }
    pub fn definition(&self, id: DefinitionId) -> Option<&Definition<'a>> {
        self.definitions.get(id.0)
    }
    pub fn fragment(&self, id: FragmentId) -> Option<&Fragment> {
        self.fragments.get(id.0)
    }
    pub fn references(&self) -> &[TypeReference<'a>] {
        &self.references
    }
    pub fn includes(&self) -> &[Include] {
        &self.includes
    }
    /// Erwartet bereits normalisierte Bezeichner (ohne Escape-Unterstrich).
    pub fn find(&self, name: &str) -> Option<DefinitionId> {
        self.named.get(name).copied()
    }
    /// Mixins sind keine Typnamen und besitzen einen eigenen Index.
    pub fn find_mixin(&self, name: &str) -> Option<DefinitionId> {
        self.mixins.get(name).copied()
    }
}

pub fn build<'a>(sources: &[&'a str]) -> Result<Index<'a>, Error> {
    build_within(sources, Limits::default(), &Budget::new(DEFAULT_STEPS))
}

/// Alle Quellen teilen dieselbe Arbeitsquote und dieselben Gesamtgrenzen.
/// Es wird bei Fehlern kein unvollständig aufgelöster Index zurückgegeben.
pub fn build_within<'a>(
    sources: &[&'a str],
    limits: Limits,
    budget: &Budget,
) -> Result<Index<'a>, Error> {
    let mut work = Work {
        budget,
        limits,
        records: 0,
    };
    work.spend(0, None)?;
    if sources.len() > limits.sources {
        return Err(error(ErrorKind::Limit(Limit::Sources), None));
    }
    let mut bytes = 0usize;
    for source in sources {
        work.spend(1, None)?;
        bytes = bytes
            .checked_add(source.len())
            .filter(|&n| n <= limits.source_bytes)
            .ok_or_else(|| error(ErrorKind::Limit(Limit::SourceBytes), None))?;
    }
    let mut documents = Vec::new();
    let mut raw = Raw::default();
    let mut nodes = 0;
    for (source_id, source) in sources.iter().enumerate() {
        work.spend(1, None)?;
        let remaining = limits.nodes - nodes;
        let parser_limits = parser::Limits {
            nodes: limits.parser.nodes.min(remaining),
            ..limits.parser
        };
        let document = parser::parse_within(source, parser_limits, budget).map_err(|e| {
            let kind = match e.kind {
                parser::ErrorKind::Exhausted(reason) => ErrorKind::Exhausted(reason),
                parser::ErrorKind::Limit(parser::Limit::Nodes)
                    if remaining <= limits.parser.nodes =>
                {
                    ErrorKind::Limit(Limit::Nodes)
                }
                kind => ErrorKind::Parser(kind),
            };
            // Kein Baum wurde zurückgegeben; die Quellposition bleibt trotzdem verfügbar.
            Error {
                kind,
                location: Some(Location {
                    source: source_id,
                    node: None,
                    span: e.span,
                }),
                previous: None,
            }
        })?;
        nodes += document.len();
        collect::document(&document, source_id, &mut raw, &mut work)?;
        documents.push(document);
    }
    link::resolve(documents, raw, &mut work)
}

fn error(kind: ErrorKind, location: Option<&Location>) -> Error {
    Error {
        kind,
        location: location.cloned(),
        previous: None,
    }
}

struct Work<'b> {
    budget: &'b Budget,
    limits: Limits,
    records: usize,
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
}

#[derive(Default)]
struct Raw<'a> {
    fragments: Vec<RawFragment<'a>>,
    references: Vec<RawReference<'a>>,
    includes: Vec<RawInclude<'a>>,
}

struct Name<'a> {
    text: &'a str,
    location: Location,
}

struct RawFragment<'a> {
    name: Name<'a>,
    kind: DefinitionKind,
    partial: bool,
    body: NodeId,
    attributes: NodeId,
    parent: Option<Name<'a>>,
    direct_alias: Option<NodeId>,
}

struct RawReference<'a> {
    name: Name<'a>,
    fragment: FragmentId,
}

struct RawInclude<'a> {
    interface: Name<'a>,
    mixin: Name<'a>,
}

fn production(document: &Document<'_>, id: NodeId) -> Option<&'static str> {
    match document.node(id).expect("eigener Knoten").kind {
        Kind::Production(name) => Some(name),
        _ => None,
    }
}

fn child(document: &Document<'_>, id: NodeId, name: &str) -> Option<NodeId> {
    document
        .children(id)
        .find(|&id| production(document, id) == Some(name))
}

fn identifier(document: &Document<'_>, id: NodeId) -> Option<NodeId> {
    document
        .children(id)
        .find(|&id| document.node(id).expect("Kind").kind == Kind::Token(lexer::Kind::Identifier))
}

fn name<'a>(document: &Document<'a>, source: usize, id: NodeId) -> Name<'a> {
    let text = document.text(id).expect("Bezeichnertext");
    Name {
        text: text.strip_prefix('_').unwrap_or(text),
        location: Location {
            source,
            node: Some(id),
            span: document.node(id).expect("Bezeichner").span.clone(),
        },
    }
}

#[cfg(test)]
mod tests;
