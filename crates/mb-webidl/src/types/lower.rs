use super::*;

pub(super) fn lower(index: &Index<'_>, work: &Work<'_>) -> Result<Types, Error> {
    let mut count = 0usize;
    for source in 0..index.sources() {
        work.spend(1, None)?;
        count = count
            .checked_add(index.document(source).expect("Quelle").len())
            .filter(|&n| n <= work.limits.source_nodes)
            .ok_or_else(|| error(ErrorKind::Limit(Limit::SourceNodes), None))?;
    }
    if index.references().len() > work.limits.types {
        return Err(error(ErrorKind::Limit(Limit::Types), None));
    }
    let mut references = BTreeMap::new();
    for reference in index.references() {
        work.spend(1, Some(&reference.location))?;
        references.insert(
            (
                reference.location.source,
                reference.location.node.expect("Typverweis"),
            ),
            reference.target,
        );
    }
    let mut builder = Builder {
        work,
        types: Types {
            nodes: Vec::new(),
            edges: Vec::new(),
            sources: Vec::new(),
            typedefs: BTreeMap::new(),
            facts: Vec::new(),
        },
    };
    for source in 0..index.sources() {
        let document = index.document(source).expect("Quelle");
        work.spend(document.len() as u64, None)?;
        builder.types.sources.push(vec![None; document.len()]);
        for (id, node) in document.nodes().rev() {
            work.spend(1, None)?;
            let result = match node.kind {
                NodeKind::Production("PrimitiveType") => Some(builder.add(
                    source,
                    document,
                    id,
                    Kind::Atom(Atom::Primitive(primitive(document, id))),
                    &[],
                )?),
                NodeKind::Production("StringType") => Some(builder.add(
                    source,
                    document,
                    id,
                    Kind::Atom(Atom::String(string(first_token(document, id)))),
                    &[],
                )?),
                NodeKind::Production("BufferRelatedType") => Some(builder.add(
                    source,
                    document,
                    id,
                    Kind::Atom(Atom::Buffer(buffer(first_token(document, id)))),
                    &[],
                )?),
                NodeKind::Production("ConstType") => {
                    if let Some(base) = child(document, id, "PrimitiveType") {
                        Some(builder.ty(source, base))
                    } else {
                        Some(builder.named(source, document, id, &references)?)
                    }
                }
                NodeKind::Production("PromiseType") => {
                    let inner =
                        builder.ty(source, child(document, id, "Type").expect("Promisetyp"));
                    Some(builder.add(
                        source,
                        document,
                        id,
                        Kind::Generic(Generic::Promise),
                        &[inner],
                    )?)
                }
                NodeKind::Production("RecordType") => {
                    let key = string(first_token(
                        document,
                        child(document, id, "StringType").expect("Recordschlüssel"),
                    ));
                    let inner = builder.ty(
                        source,
                        child(document, id, "TypeWithExtendedAttributes").expect("Recordwert"),
                    );
                    Some(builder.add(source, document, id, Kind::Record(key), &[inner])?)
                }
                NodeKind::Production("DistinguishableType") => {
                    let base = if let Some(ty) = child(document, id, "TypeWithExtendedAttributes") {
                        let kind = match first_token(document, id) {
                            "sequence" => Generic::Sequence,
                            "async_sequence" => Generic::AsyncSequence,
                            "FrozenArray" => Generic::FrozenArray,
                            "ObservableArray" => Generic::ObservableArray,
                            _ => unreachable!("Generischer Typ"),
                        };
                        let inner = builder.ty(source, ty);
                        builder.add(source, document, id, Kind::Generic(kind), &[inner])?
                    } else if document.children(id).any(|id| {
                        document.node(id).expect("Kind").kind
                            == NodeKind::Token(lexer::Kind::Identifier)
                    }) {
                        builder.named(source, document, id, &references)?
                    } else if let Some(inner) = document
                        .children(id)
                        .find_map(|id| builder.types.at(source, id))
                    {
                        inner
                    } else {
                        let atom = match first_token(document, id) {
                            "object" => Atom::Object,
                            "symbol" => Atom::Symbol,
                            "undefined" => Atom::Undefined,
                            _ => unreachable!("einfacher Typ"),
                        };
                        builder.add(source, document, id, Kind::Atom(atom), &[])?
                    };
                    Some(builder.nullable(source, document, id, base)?)
                }
                NodeKind::Production("SingleType") => {
                    if let Some(inner) = document
                        .children(id)
                        .find_map(|id| builder.types.at(source, id))
                    {
                        Some(inner)
                    } else {
                        Some(builder.add(source, document, id, Kind::Atom(Atom::Any), &[])?)
                    }
                }
                NodeKind::Production("UnionType") => {
                    let remaining = work.limits.edges.saturating_sub(builder.types.edges.len());
                    if remaining < 2 {
                        return Err(error(
                            ErrorKind::Limit(Limit::Edges),
                            Some(&location(document, source, id)),
                        ));
                    }
                    let mut members = Vec::new();
                    for member in document
                        .children(id)
                        .filter(|&id| production(document, id) == Some("UnionMemberType"))
                    {
                        members.push(builder.ty(source, member));
                    }
                    let mut tail = child(document, id, "UnionMemberTypes").expect("Unionliste");
                    while let Some(member) = child(document, tail, "UnionMemberType") {
                        work.spend(1, None)?;
                        if members.len() >= remaining {
                            return Err(error(
                                ErrorKind::Limit(Limit::Edges),
                                Some(&location(document, source, id)),
                            ));
                        }
                        members.push(builder.ty(source, member));
                        tail = child(document, tail, "UnionMemberTypes").expect("Unionrest");
                    }
                    Some(builder.add(source, document, id, Kind::Union, &members)?)
                }
                NodeKind::Production("UnionMemberType") => {
                    let inner = document
                        .children(id)
                        .find_map(|id| builder.types.at(source, id))
                        .expect("Unionmembertyp");
                    let inner = builder.nullable(source, document, id, inner)?;
                    Some(builder.annotated(source, document, id, inner)?)
                }
                NodeKind::Production("Type") => {
                    let inner = document
                        .children(id)
                        .find_map(|id| builder.types.at(source, id))
                        .expect("Typ");
                    Some(builder.nullable(source, document, id, inner)?)
                }
                NodeKind::Production("TypeWithExtendedAttributes") => {
                    let inner = builder.ty(
                        source,
                        child(document, id, "Type").expect("annotierter Typ"),
                    );
                    Some(builder.annotated(source, document, id, inner)?)
                }
                _ => None,
            };
            builder.types.sources[source][id.index()] = result;
        }
    }
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if definition.kind != DefinitionKind::Typedef {
            continue;
        }
        let fragment = index.fragment(definition.fragments[0]).expect("Typedef");
        let document = index
            .document(fragment.location.source)
            .expect("Typedefquelle");
        let root = builder.ty(
            fragment.location.source,
            child(document, fragment.body, "TypeWithExtendedAttributes").expect("Typedefwurzel"),
        );
        builder.types.typedefs.insert(id, root);
    }
    Ok(builder.types)
}

struct Builder<'w, 'b> {
    work: &'w Work<'b>,
    types: Types,
}
impl Builder<'_, '_> {
    fn ty(&self, source: usize, node: NodeId) -> TypeId {
        self.types
            .at(source, node)
            .expect("Kindtyp bereits aufgebaut")
    }
    fn add(
        &mut self,
        source: usize,
        document: &Document<'_>,
        node: NodeId,
        kind: Kind,
        children: &[TypeId],
    ) -> Result<TypeId, Error> {
        let at = location(document, source, node);
        self.work.spend(children.len() as u64 + 1, Some(&at))?;
        if self.types.nodes.len() >= self.work.limits.types
            || self.types.nodes.len() >= u32::MAX as usize
        {
            return Err(error(ErrorKind::Limit(Limit::Types), Some(&at)));
        }
        let end = self
            .types
            .edges
            .len()
            .checked_add(children.len())
            .filter(|&n| n <= self.work.limits.edges)
            .ok_or_else(|| error(ErrorKind::Limit(Limit::Edges), Some(&at)))?;
        let start = self.types.edges.len();
        self.types.edges.extend_from_slice(children);
        let id = TypeId(NonZeroU32::new(self.types.nodes.len() as u32 + 1).expect("neue Typ-ID"));
        self.types.nodes.push(Type {
            kind,
            location: at,
            children: start..end,
        });
        Ok(id)
    }
    fn named(
        &mut self,
        source: usize,
        document: &Document<'_>,
        node: NodeId,
        references: &BTreeMap<(usize, NodeId), DefinitionId>,
    ) -> Result<TypeId, Error> {
        let name = document
            .children(node)
            .find(|&id| {
                document.node(id).expect("Name").kind == NodeKind::Token(lexer::Kind::Identifier)
            })
            .expect("Typname");
        let target = *references
            .get(&(source, name))
            .expect("aufgelöster Typname");
        self.add(source, document, node, Kind::Named(target), &[])
    }
    fn nullable(
        &mut self,
        source: usize,
        document: &Document<'_>,
        node: NodeId,
        inner: TypeId,
    ) -> Result<TypeId, Error> {
        if child(document, node, "Null").is_some_and(|id| document.children(id).next().is_some()) {
            self.add(source, document, node, Kind::Nullable, &[inner])
        } else {
            Ok(inner)
        }
    }
    fn annotated(
        &mut self,
        source: usize,
        document: &Document<'_>,
        node: NodeId,
        inner: TypeId,
    ) -> Result<TypeId, Error> {
        if let Some(attributes) = child(document, node, "ExtendedAttributeList")
            .filter(|&id| document.children(id).next().is_some())
        {
            self.add(
                source,
                document,
                node,
                Kind::Annotated { attributes },
                &[inner],
            )
        } else {
            Ok(inner)
        }
    }
}
fn first_token<'a>(document: &Document<'a>, id: NodeId) -> &'a str {
    document
        .children(id)
        .find_map(|id| match document.node(id).expect("Token").kind {
            NodeKind::Token(_) => document.text(id),
            _ => None,
        })
        .expect("Terminal")
}
fn string(name: &str) -> StringType {
    match name {
        "ByteString" => StringType::ByteString,
        "DOMString" => StringType::DOMString,
        "USVString" => StringType::USVString,
        _ => unreachable!("Stringtyp"),
    }
}
fn primitive(document: &Document<'_>, id: NodeId) -> Primitive {
    let mut pending = vec![id];
    let mut words = Vec::new();
    while let Some(id) = pending.pop() {
        match document.node(id).expect("Primitive").kind {
            NodeKind::Token(_) => words.push(document.text(id).expect("Typwort")),
            _ => pending.extend(document.children(id).rev()),
        }
    }
    match words.as_slice() {
        ["boolean"] => Primitive::Boolean,
        ["byte"] => Primitive::Byte,
        ["octet"] => Primitive::Octet,
        ["short"] => Primitive::Short,
        ["unsigned", "short"] => Primitive::UnsignedShort,
        ["long"] => Primitive::Long,
        ["unsigned", "long"] => Primitive::UnsignedLong,
        ["long", "long"] => Primitive::LongLong,
        ["unsigned", "long", "long"] => Primitive::UnsignedLongLong,
        ["float"] => Primitive::Float,
        ["unrestricted", "float"] => Primitive::UnrestrictedFloat,
        ["double"] => Primitive::Double,
        ["unrestricted", "double"] => Primitive::UnrestrictedDouble,
        ["bigint"] => Primitive::BigInt,
        _ => unreachable!("primitiver Typ"),
    }
}
fn buffer(name: &str) -> Buffer {
    match name {
        "ArrayBuffer" => Buffer::ArrayBuffer,
        "SharedArrayBuffer" => Buffer::SharedArrayBuffer,
        "DataView" => Buffer::DataView,
        "Int8Array" => Buffer::Int8Array,
        "Int16Array" => Buffer::Int16Array,
        "Int32Array" => Buffer::Int32Array,
        "Uint8Array" => Buffer::Uint8Array,
        "Uint16Array" => Buffer::Uint16Array,
        "Uint32Array" => Buffer::Uint32Array,
        "Uint8ClampedArray" => Buffer::Uint8ClampedArray,
        "BigInt64Array" => Buffer::BigInt64Array,
        "BigUint64Array" => Buffer::BigUint64Array,
        "Float16Array" => Buffer::Float16Array,
        "Float32Array" => Buffer::Float32Array,
        "Float64Array" => Buffer::Float64Array,
        _ => unreachable!("Buffertyp"),
    }
}
