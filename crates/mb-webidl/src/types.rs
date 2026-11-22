//! Flache Typbeschreibung mit expliziten Typedef-Verweisen und ersten Typregeln.
//! Keine vollständige WebIDL-Validierung oder Freigabe zur Binding-Erzeugung.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::ops::Range;

use crate::index::{DefinitionId, DefinitionKind, Index, Location};
use crate::lexer;
use crate::members::Members;
use crate::parser::{Document, Kind as NodeKind, NodeId};
pub use mb_base::{Budget, Cancel, Exhausted};

mod check;
mod dictionary;
mod lower;
mod union;
mod values;

pub const DEFAULT_STEPS: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub source_nodes: usize,
    pub types: usize,
    pub edges: usize,
    /// Dictionary-Knoten im impliziten Einschlussgraphen.
    pub dictionary_nodes: usize,
    /// Besuchte Einschlusskanten, einschließlich Feldern und Vererbung.
    pub dictionary_edges: usize,
    pub union_visits: usize,
    pub union_interfaces: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            source_nodes: 1_000_000,
            types: 200_000,
            edges: 400_000,
            dictionary_nodes: 200_000,
            dictionary_edges: 1_000_000,
            union_visits: 1_000_000,
            union_interfaces: 200_000,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    SourceNodes,
    Types,
    Edges,
    DictionaryNodes,
    DictionaryEdges,
    UnionVisits,
    UnionInterfaces,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Limit(Limit),
    Exhausted(Exhausted),
    TypedefCycle,
    NullableType,
    UnionType,
    ConstantType,
    AttributeType,
    AttributePromiseReadonly,
    NullableDictionary,
    DictionaryCycle,
    IndistinguishableUnion,
    InvalidCallbackAttribute,
    ConstantValue,
    DefaultValue,
    UndefinedType,
    DictionaryArgumentDefault,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub location: Option<Location>,
}

/// Lokale 32-Bit-ID. Auch Option<TypeId> benötigt nur vier Bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TypeId(NonZeroU32);
impl TypeId {
    fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Boolean,
    Byte,
    Octet,
    Short,
    UnsignedShort,
    Long,
    UnsignedLong,
    LongLong,
    UnsignedLongLong,
    Float,
    UnrestrictedFloat,
    Double,
    UnrestrictedDouble,
    BigInt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringType {
    ByteString,
    DOMString,
    USVString,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Buffer {
    ArrayBuffer,
    SharedArrayBuffer,
    DataView,
    Int8Array,
    Int16Array,
    Int32Array,
    Uint8Array,
    Uint16Array,
    Uint32Array,
    Uint8ClampedArray,
    BigInt64Array,
    BigUint64Array,
    Float16Array,
    Float32Array,
    Float64Array,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atom {
    Any,
    Primitive(Primitive),
    String(StringType),
    Buffer(Buffer),
    Object,
    Symbol,
    Undefined,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Generic {
    Sequence,
    AsyncSequence,
    FrozenArray,
    ObservableArray,
    Promise,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Atom(Atom),
    Named(DefinitionId),
    Nullable,
    Generic(Generic),
    /// Genau ein Kind: der Werttyp. Schlüsseltypen sind stets Stringtypen.
    Record(StringType),
    Union,
    /// Explizite, noch nicht semantisch ausgewertete Typattribute.
    Annotated {
        attributes: NodeId,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Any,
    Primitive,
    String,
    Buffer,
    Object,
    Symbol,
    Undefined,
    Interface,
    CallbackInterface,
    Dictionary,
    Enum,
    Callback,
    Nullable,
    Sequence,
    AsyncSequence,
    FrozenArray,
    ObservableArray,
    Promise,
    Record,
    Union,
}

#[derive(Debug)]
pub struct Type {
    pub kind: Kind,
    pub location: Location,
    children: Range<usize>,
}

/// Jeder Quelltyp erhält eine lokale Form; Grammatikhüllen teilen deren ID.
/// Typedefs werden über IDs verbunden, ohne ihre Bäume zu kopieren.
#[derive(Debug)]
pub struct Types {
    nodes: Vec<Type>,
    edges: Vec<TypeId>,
    sources: Vec<Vec<Option<TypeId>>>,
    typedefs: BTreeMap<DefinitionId, TypeId>,
    facts: Vec<Facts>,
}
impl Types {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
    pub fn get(&self, id: TypeId) -> Option<&Type> {
        self.nodes.get(id.index())
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (TypeId, &Type)> + DoubleEndedIterator {
        self.nodes.iter().enumerate().map(|(n, ty)| {
            (
                TypeId(NonZeroU32::new(n as u32 + 1).expect("lokale Typ-ID")),
                ty,
            )
        })
    }
    pub fn children(&self, id: TypeId) -> Option<&[TypeId]> {
        self.edges.get(self.get(id)?.children.clone())
    }
    pub fn at(&self, source: usize, node: NodeId) -> Option<TypeId> {
        self.sources
            .get(source)?
            .get(node.index())
            .copied()
            .flatten()
    }
    pub fn typedef(&self, id: DefinitionId) -> Option<TypeId> {
        self.typedefs.get(&id).copied()
    }
    /// Nach Auflösen von Typedefs und expliziten Annotationshüllen.
    pub fn category(&self, id: TypeId) -> Option<Category> {
        Some(self.facts.get(id.index())?.category)
    }
}

pub fn build(index: &Index<'_>, members: &Members<'_>) -> Result<Types, Error> {
    build_within(
        index,
        members,
        Limits::default(),
        &Budget::new(DEFAULT_STEPS),
    )
}
pub fn build_within(
    index: &Index<'_>,
    members: &Members<'_>,
    limits: Limits,
    budget: &Budget,
) -> Result<Types, Error> {
    let work = Work { limits, budget };
    work.spend(0, None)?;
    let mut types = lower::lower(index, &work)?;
    check::check(index, members, &mut types, &work)?;
    Ok(types)
}
fn error(kind: ErrorKind, at: Option<&Location>) -> Error {
    Error {
        kind,
        location: at.cloned(),
    }
}
struct Work<'b> {
    limits: Limits,
    budget: &'b Budget,
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
}
#[derive(Clone, Copy, Debug)]
struct Facts {
    category: Category,
    nullable: u8,
    dictionary: bool,
    attribute_forbidden: bool,
    union_attribute_forbidden: bool,
    any: bool,
    promise: bool,
}
impl Facts {
    fn new(category: Category) -> Self {
        Self {
            category,
            nullable: 0,
            dictionary: category == Category::Dictionary,
            attribute_forbidden: matches!(
                category,
                Category::Dictionary
                    | Category::Sequence
                    | Category::AsyncSequence
                    | Category::Record
            ),
            union_attribute_forbidden: matches!(
                category,
                Category::Dictionary | Category::Sequence | Category::Record
            ),
            any: category == Category::Any,
            promise: category == Category::Promise,
        }
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
fn location(document: &Document<'_>, source: usize, id: NodeId) -> Location {
    Location {
        source,
        node: Some(id),
        span: document.node(id).expect("Quellknoten").span.clone(),
    }
}
#[cfg(test)]
mod tests;
