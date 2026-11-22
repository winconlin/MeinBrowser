//! Signaturen und deklarationsübergreifende Bedingungen von Spezialoperationen.

use super::*;
use crate::members::{Kind as MemberKind, Member, Special};

mod terminals;
use terminals::terminals;

const NAMED_GETTER: usize = 0;
const INDEXED_GETTER: usize = 1;
const NAMED_SETTER: usize = 2;
const INDEXED_SETTER: usize = 3;
const DELETER: usize = 4;
const STRINGIFIER: usize = 5;

#[derive(Clone, Copy, Default)]
struct Summary<'m, 's> {
    slots: [Option<&'m Member<'s>>; 6],
    length: Option<(&'m Member<'s>, bool)>,
}
impl<'m, 's> Summary<'m, 's> {
    fn insert(&mut self, slot: usize, member: &'m Member<'s>) -> Result<(), Error> {
        if self.slots[slot].is_some() {
            let kind = if slot == STRINGIFIER {
                ErrorKind::DuplicateStringifier
            } else {
                ErrorKind::DuplicateSpecialOperation
            };
            return Err(error(kind, Some(&member.location)));
        }
        self.slots[slot] = Some(member);
        Ok(())
    }
    fn include(&mut self, other: Self, work: &Work<'_>) -> Result<(), Error> {
        work.spend(6, None)?;
        for (slot, member) in other.slots.into_iter().enumerate() {
            if let Some(member) = member {
                self.insert(slot, member)?;
            }
        }
        self.length = self.length.or(other.length);
        Ok(())
    }
    fn inherit(&mut self, parent: Self) {
        for (slot, inherited) in self.slots.iter_mut().zip(parent.slots) {
            *slot = slot.or(inherited);
        }
        self.length = self.length.or(parent.length);
    }
    fn conditions(&self, types: &Types) -> Result<(), Error> {
        for (slot, getter) in [
            (NAMED_SETTER, NAMED_GETTER),
            (INDEXED_SETTER, INDEXED_GETTER),
            (DELETER, NAMED_GETTER),
        ] {
            if let Some(member) = self.slots[slot]
                && self.slots[getter].is_none()
            {
                return Err(error(
                    ErrorKind::MissingSpecialGetter,
                    Some(&member.location),
                ));
            }
        }
        if let Some(getter) = self.slots[INDEXED_GETTER] {
            match self.length {
                Some((_, true)) => {}
                Some((member, false)) => {
                    let ty = types
                        .at(member.location.source, member.types[0].expect("length-Typ"))
                        .expect("Typ");
                    return Err(error(
                        ErrorKind::IndexedLength,
                        Some(&types.get(ty).expect("Typ").location),
                    ));
                }
                None => return Err(error(ErrorKind::IndexedLength, Some(&getter.location))),
            }
        }
        Ok(())
    }
}

pub(super) fn check(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &Types,
    work: &Work<'_>,
) -> Result<(), Error> {
    let terminals = terminals(types, work)?;
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
            if let MemberKind::Operation(special) = member.kind
                && special != Special::Regular
            {
                summary.insert(
                    operation(member, special, members, types, &terminals, work)?,
                    member,
                )?;
            }
            if member.stringifier {
                if member.kind == MemberKind::Attribute {
                    let ty = types
                        .at(
                            member.location.source,
                            member.types[0].expect("Stringifier-Typ"),
                        )
                        .expect("Typ");
                    if member.is_static
                        || !matches!(
                            terminals[ty.index()],
                            Some(Atom::String(StringType::DOMString | StringType::USVString))
                        )
                    {
                        return Err(error(
                            ErrorKind::StringifierType,
                            Some(&types.get(ty).expect("Typ").location),
                        ));
                    }
                }
                summary.insert(STRINGIFIER, member)?;
            }
            if member.kind == MemberKind::Attribute
                && !member.is_static
                && member
                    .identifier
                    .as_ref()
                    .is_some_and(|name| name.name == "length")
            {
                let ty = types
                    .at(member.location.source, member.types[0].expect("length-Typ"))
                    .expect("Typ");
                let integer = matches!(
                    terminals[ty.index()],
                    Some(Atom::Primitive(
                        Primitive::Byte
                            | Primitive::Octet
                            | Primitive::Short
                            | Primitive::UnsignedShort
                            | Primitive::Long
                            | Primitive::UnsignedLong
                            | Primitive::LongLong
                            | Primitive::UnsignedLongLong
                    ))
                );
                summary.length = Some((member, integer));
            }
        }
        local.insert(id, summary);
    }
    // Mixins einmal beschreiben; beim Include nur sechs Slots übertragen.
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if definition.kind != DefinitionKind::Interface {
            continue;
        }
        for mixin in &definition.mixins {
            work.spend(1, Some(&definition.location))?;
            let addition = local[mixin];
            local
                .get_mut(&id)
                .expect("Interface")
                .include(addition, work)?;
        }
    }
    // Eigene Deklarationen müssen eindeutig sein. Geerbte Slots dienen hier
    // nur den Bedingungen; dies legt keinen späteren Runtime-Dispatch fest.
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
            work.spend(6, None)?;
            let mut own = local[&id];
            own.inherit(inherited);
            own.conditions(types)?;
            effective.insert(id, own);
            inherited = own;
        }
    }
    Ok(())
}

fn operation(
    member: &Member<'_>,
    special: Special,
    members: &Members<'_>,
    types: &Types,
    terminals: &[Option<Atom>],
    work: &Work<'_>,
) -> Result<usize, Error> {
    let arguments = members
        .arguments(member.signature.as_ref().expect("Spezialsignatur"))
        .expect("Argumente");
    work.spend(arguments.len() as u64, Some(&member.location))?;
    let arity = if special == Special::Setter { 2 } else { 1 };
    if arguments.len() != arity || arguments.iter().any(|arg| arg.optional || arg.variadic) {
        return Err(error(
            ErrorKind::SpecialOperationSignature,
            Some(&member.location),
        ));
    }
    let argument = &arguments[0];
    let ty = types
        .at(argument.identifier.location.source, argument.ty)
        .expect("Schlüsseltyp");
    let indexed = match terminals[ty.index()] {
        Some(Atom::String(StringType::DOMString)) => false,
        Some(Atom::Primitive(Primitive::UnsignedLong)) if special != Special::Deleter => true,
        _ => {
            return Err(error(
                ErrorKind::SpecialOperationKey,
                Some(&types.get(ty).expect("Typ").location),
            ));
        }
    };
    Ok(match special {
        Special::Getter => {
            if indexed {
                INDEXED_GETTER
            } else {
                NAMED_GETTER
            }
        }
        Special::Setter => {
            if indexed {
                INDEXED_SETTER
            } else {
                NAMED_SETTER
            }
        }
        Special::Deleter => DELETER,
        Special::Regular => unreachable!("gewöhnliche Operation ausgeschlossen"),
    })
}

#[cfg(test)]
mod tests;
