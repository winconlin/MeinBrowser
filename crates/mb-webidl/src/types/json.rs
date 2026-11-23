//! JSON-Typen nach der WebIDL-Liste, einschließlich geerbter Dictionaryfelder.

use super::*;
use crate::members::{Kind as MemberKind, Member};

mod interfaces;

pub(super) fn operation(member: &Member<'_>) -> bool {
    !member.is_static
        && matches!(member.kind, MemberKind::Operation(_))
        && member
            .identifier
            .as_ref()
            .is_some_and(|name| name.name == "toJSON")
}

pub(super) fn check(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &mut Types,
    work: &Work<'_>,
) -> Result<(), Error> {
    let interfaces = interfaces::build(index, members, work)?;
    let values = Graph::new(index, members, types, work, interfaces)?.classify()?;
    for (fact, json) in types.facts.iter_mut().zip(values) {
        work.spend(1, None)?;
        fact.json = json;
    }
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        for member in members.members(id).expect("Member") {
            work.spend(1, Some(&member.location))?;
            if operation(member) {
                let ty = types
                    .at(
                        member.location.source,
                        member.types[0].expect("toJSON-Ergebnis"),
                    )
                    .expect("Typ");
                if !types.is_json(ty).expect("lokaler Typ") {
                    return Err(error(
                        ErrorKind::ToJSONType,
                        Some(&types.get(ty).expect("Typ").location),
                    ));
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Node {
    Type(TypeId),
    Dictionary(usize),
}
struct Frame {
    node: Node,
    next: usize,
    json: bool,
}
struct Memo {
    types: Vec<bool>,
    dictionaries: Vec<bool>,
    type_colors: Vec<u8>,
    dictionary_colors: Vec<u8>,
}
impl Memo {
    fn color(&self, node: Node) -> u8 {
        match node {
            Node::Type(id) => self.type_colors[id.index()],
            Node::Dictionary(id) => self.dictionary_colors[id],
        }
    }
    fn value(&self, node: Node) -> bool {
        match node {
            Node::Type(id) => self.types[id.index()],
            Node::Dictionary(id) => self.dictionaries[id],
        }
    }
    fn enter(&mut self, node: Node) {
        match node {
            Node::Type(id) => self.type_colors[id.index()] = 1,
            Node::Dictionary(id) => self.dictionary_colors[id] = 1,
        }
    }
    fn finish(&mut self, node: Node, value: bool) {
        match node {
            Node::Type(id) => {
                self.type_colors[id.index()] = 2;
                self.types[id.index()] = value;
            }
            Node::Dictionary(id) => {
                self.dictionary_colors[id] = 2;
                self.dictionaries[id] = value;
            }
        }
    }
}
struct Graph<'a, 's, 'b> {
    index: &'a Index<'s>,
    members: &'a Members<'s>,
    types: &'a Types,
    work: &'a Work<'b>,
    interfaces: BTreeMap<DefinitionId, bool>,
    dictionaries: Vec<DefinitionId>,
    ids: BTreeMap<DefinitionId, usize>,
}
impl<'a, 's, 'b> Graph<'a, 's, 'b> {
    fn new(
        index: &'a Index<'s>,
        members: &'a Members<'s>,
        types: &'a Types,
        work: &'a Work<'b>,
        interfaces: BTreeMap<DefinitionId, bool>,
    ) -> Result<Self, Error> {
        let mut dictionaries = Vec::new();
        let mut ids = BTreeMap::new();
        for (id, definition) in index.definitions() {
            work.spend(1, Some(&definition.location))?;
            if definition.kind == DefinitionKind::Dictionary {
                if dictionaries.len() >= work.limits.dictionary_nodes {
                    return Err(error(
                        ErrorKind::Limit(Limit::DictionaryNodes),
                        Some(&definition.location),
                    ));
                }
                ids.insert(id, dictionaries.len());
                dictionaries.push(id);
            }
        }
        Ok(Self {
            index,
            members,
            types,
            work,
            interfaces,
            dictionaries,
            ids,
        })
    }
    fn classify(self) -> Result<Vec<bool>, Error> {
        self.work.spend(self.types.len() as u64, None)?;
        self.work.spend(self.dictionaries.len() as u64, None)?;
        let mut memo = Memo {
            types: vec![false; self.types.len()],
            dictionaries: vec![false; self.dictionaries.len()],
            type_colors: vec![0; self.types.len()],
            dictionary_colors: vec![0; self.dictionaries.len()],
        };
        let roots = self
            .types
            .iter()
            .map(|(id, _)| Node::Type(id))
            .chain((0..self.dictionaries.len()).map(Node::Dictionary));
        for root in roots {
            self.work.spend(1, Some(self.at(root)))?;
            if memo.color(root) == 2 {
                continue;
            }
            memo.enter(root);
            let mut pending = vec![Frame {
                node: root,
                next: 0,
                json: self.leaf(root),
            }];
            while let Some(frame) = pending.last_mut() {
                self.work.spend(1, Some(self.at(frame.node)))?;
                if let Some(neighbor) = self.neighbor(frame.node, frame.next) {
                    frame.next += 1;
                    match memo.color(neighbor) {
                        0 => {
                            memo.enter(neighbor);
                            pending.push(Frame {
                                node: neighbor,
                                next: 0,
                                json: self.leaf(neighbor),
                            });
                        }
                        1 => {
                            return Err(error(
                                ErrorKind::DictionaryCycle,
                                Some(self.at(frame.node)),
                            ));
                        }
                        _ => frame.json &= memo.value(neighbor),
                    }
                } else {
                    let finished = pending.pop().expect("Rahmen");
                    memo.finish(finished.node, finished.json);
                    if let Some(parent) = pending.last_mut() {
                        parent.json &= finished.json;
                    }
                }
            }
        }
        Ok(memo.types)
    }
    fn at(&self, node: Node) -> &Location {
        match node {
            Node::Type(id) => &self.types.get(id).expect("Typ").location,
            Node::Dictionary(n) => {
                &self
                    .index
                    .definition(self.dictionaries[n])
                    .expect("Dictionary")
                    .location
            }
        }
    }
    fn leaf(&self, node: Node) -> bool {
        let Node::Type(id) = node else {
            return true;
        };
        match self.types.get(id).expect("Typ").kind {
            Kind::Atom(Atom::Primitive(primitive)) => primitive != Primitive::BigInt,
            Kind::Atom(Atom::String(_) | Atom::Object) => true,
            Kind::Named(target) => match self.index.definition(target).expect("Definition").kind {
                DefinitionKind::Typedef | DefinitionKind::Dictionary | DefinitionKind::Enum => true,
                DefinitionKind::Interface => self.interfaces[&target],
                _ => false,
            },
            Kind::Annotated { .. }
            | Kind::Nullable
            | Kind::Union
            | Kind::Record(_)
            | Kind::Generic(Generic::Sequence | Generic::FrozenArray) => true,
            _ => false,
        }
    }
    fn neighbor(&self, node: Node, next: usize) -> Option<Node> {
        match node {
            Node::Dictionary(n) => {
                let id = self.dictionaries[n];
                let definition = self.index.definition(id).expect("Dictionary");
                let has_parent = usize::from(definition.parent.is_some());
                if next < has_parent {
                    return Some(Node::Dictionary(
                        self.ids[&definition.parent.expect("Vorfahre")],
                    ));
                }
                let member = self
                    .members
                    .members(id)
                    .expect("Dictionaryfelder")
                    .get(next - has_parent)?;
                Some(Node::Type(
                    self.types
                        .at(member.location.source, member.types[0].expect("Feldtyp"))
                        .expect("Typ"),
                ))
            }
            Node::Type(id) => match self.types.get(id).expect("Typ").kind {
                Kind::Named(target) if next == 0 => {
                    if let Some(ty) = self.types.typedef(target) {
                        Some(Node::Type(ty))
                    } else {
                        self.ids.get(&target).copied().map(Node::Dictionary)
                    }
                }
                Kind::Annotated { .. }
                | Kind::Nullable
                | Kind::Union
                | Kind::Record(_)
                | Kind::Generic(Generic::Sequence | Generic::FrozenArray) => self
                    .types
                    .children(id)
                    .expect("Typkinder")
                    .get(next)
                    .copied()
                    .map(Node::Type),
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests;
