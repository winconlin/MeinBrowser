//! Deklarations-, Namens- und Typbedingungen der vier Collection-Arten.

use super::*;
use crate::members::{Kind as MemberKind, Member, Special};

mod identity;
use identity::Identity;

const NAMES: [&str; 11] = [
    "entries", "forEach", "keys", "values", "get", "has", "size", "clear", "delete", "set", "add",
];

#[derive(Clone, Copy, Default)]
struct Summary<'m, 's> {
    collection: Option<&'m Member<'s>>,
    indexed: Option<&'m Member<'s>>,
    names: [Option<&'m Member<'s>>; 11],
}
impl<'m, 's> Summary<'m, 's> {
    fn merge(&mut self, parent: Self) {
        self.collection = self.collection.or(parent.collection);
        self.indexed = self.indexed.or(parent.indexed);
        for (own, inherited) in self.names.iter_mut().zip(parent.names) {
            // Auch ein überschattetes Attribut im Vorfahren zählt für das
            // explizite Verbot auf geerbten Interfaces. Attribute bevorzugen.
            if let Some(member) = inherited
                && (own.is_none() || non_operation(member))
            {
                *own = Some(member);
            }
        }
    }
}
fn collection(kind: MemberKind) -> bool {
    matches!(
        kind,
        MemberKind::Iterable
            | MemberKind::AsyncIterable
            | MemberKind::Maplike
            | MemberKind::Setlike
    )
}
fn non_operation(member: &Member<'_>) -> bool {
    matches!(member.kind, MemberKind::Attribute | MemberKind::Constant)
}

pub(super) fn check(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &Types,
    work: &Work<'_>,
) -> Result<(), Error> {
    let mut present = false;
    'definitions: for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        for member in members.members(id).expect("Member") {
            work.spend(1, Some(&member.location))?;
            if collection(member.kind) {
                present = true;
                break 'definitions;
            }
        }
    }
    if !present {
        return Ok(());
    }
    let mut identity = Identity::new(types, work)?;
    let mut local = BTreeMap::new();
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if !matches!(
            definition.kind,
            DefinitionKind::Interface | DefinitionKind::Mixin
        ) {
            continue;
        }
        let mut summary = Summary::default();
        for member in members.members(id).expect("Member") {
            work.spend(1, Some(&member.location))?;
            if collection(member.kind) {
                if summary.collection.replace(member).is_some() {
                    return Err(error(ErrorKind::CollectionConflict, Some(&member.location)));
                }
                if member.kind == MemberKind::AsyncIterable
                    && let Some(signature) = &member.signature
                {
                    for argument in members.arguments(signature).expect("Async-Argumente") {
                        work.spend(1, Some(&argument.identifier.location))?;
                        if !argument.optional {
                            return Err(error(
                                ErrorKind::AsyncIteratorArgument,
                                Some(&argument.identifier.location),
                            ));
                        }
                    }
                }
            }
            if member.kind == MemberKind::Operation(Special::Getter) {
                let arguments = members
                    .arguments(member.signature.as_ref().expect("Getter"))
                    .expect("Argumente");
                let key = types
                    .at(member.location.source, arguments[0].ty)
                    .expect("Schlüsseltyp");
                if identity.kind(key) == Kind::Atom(Atom::Primitive(Primitive::UnsignedLong)) {
                    summary.indexed = Some(member);
                }
            }
            let regular = matches!(member.kind, MemberKind::Operation(_)) && !member.is_static;
            if (non_operation(member) || regular)
                && let Some(identifier) = &member.identifier
            {
                work.spend(identifier.name.len() as u64, Some(&identifier.location))?;
                if let Some(at) = NAMES.iter().position(|&name| name == identifier.name)
                    && (summary.names[at].is_none() || non_operation(member))
                {
                    summary.names[at] = Some(member);
                }
            }
        }
        local.insert(id, summary);
    }
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if definition.kind != DefinitionKind::Interface {
            continue;
        }
        for mixin in &definition.mixins {
            work.spend(13, Some(&definition.location))?;
            let addition = local[mixin];
            local.get_mut(&id).expect("Interface").merge(addition);
        }
    }
    let mut effective = BTreeMap::new();
    for (root, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if definition.kind != DefinitionKind::Interface {
            continue;
        }
        let mut pending = Vec::new();
        let mut cursor = Some(root);
        let mut inherited = Summary::default();
        while let Some(id) = cursor {
            work.spend(1, None)?;
            if let Some(&known) = effective.get(&id) {
                inherited = known;
                break;
            }
            pending.push(id);
            cursor = index.definition(id).expect("Interface").parent;
        }
        while let Some(id) = pending.pop() {
            work.spend(13, None)?;
            let mut own = local[&id];
            if let Some(declaration) = own.collection {
                if inherited.collection.is_some() {
                    return Err(error(
                        ErrorKind::CollectionConflict,
                        Some(&declaration.location),
                    ));
                }
                own.merge(inherited);
                conditions(declaration, &own, types, &mut identity, work)?;
            } else {
                own.merge(inherited);
            }
            effective.insert(id, own);
            inherited = own;
        }
    }
    Ok(())
}

fn conditions(
    declaration: &Member<'_>,
    summary: &Summary<'_, '_>,
    types: &Types,
    identity: &mut Identity<'_, '_>,
    work: &Work<'_>,
) -> Result<(), Error> {
    let indexed = summary.indexed.is_some();
    let pair = declaration.types[1].is_some();
    if indexed
        && (matches!(declaration.kind, MemberKind::Maplike | MemberKind::Setlike)
            || (declaration.kind == MemberKind::Iterable && pair))
    {
        return Err(error(
            ErrorKind::CollectionIndexedGetter,
            Some(&declaration.location),
        ));
    }
    if declaration.kind == MemberKind::Iterable && !pair {
        let getter = summary
            .indexed
            .ok_or_else(|| error(ErrorKind::ValueIteratorGetter, Some(&declaration.location)))?;
        let value = types
            .at(
                declaration.location.source,
                declaration.types[0].expect("Iteratorwert"),
            )
            .expect("Typ");
        let result = types
            .at(
                getter.location.source,
                getter.types[0].expect("Getterresultat"),
            )
            .expect("Typ");
        if !identity.same(value, result)? {
            return Err(error(
                ErrorKind::ValueIteratorType,
                Some(&types.get(value).expect("Werttyp").location),
            ));
        }
    }
    for (at, member) in summary.names.iter().enumerate() {
        work.spend(1, Some(&declaration.location))?;
        if let Some(member) = member {
            let common = match declaration.kind {
                MemberKind::Iterable => at <= 3,
                MemberKind::AsyncIterable => matches!(at, 0 | 2 | 3),
                MemberKind::Maplike => at <= 6,
                MemberKind::Setlike => matches!(at, 0..=3 | 5 | 6),
                _ => unreachable!("Collection"),
            };
            let mutation = !declaration.readonly
                && non_operation(member)
                && match declaration.kind {
                    MemberKind::Maplike => matches!(at, 7..=9),
                    MemberKind::Setlike => matches!(at, 7 | 8 | 10),
                    _ => false,
                };
            if common || mutation {
                return Err(error(
                    ErrorKind::CollectionMemberName,
                    member.identifier.as_ref().map(|name| &name.location),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
