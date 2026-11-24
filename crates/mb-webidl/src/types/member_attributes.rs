//! Vier argumentlose Memberattribute und explizite PutForwards-Konflikte.
//! Keine Setter-, Objektidentitäts- oder Weiterleitungs-Laufzeitimplementierung.
use super::*;
use crate::members::{Kind as MemberKind, Member};
mod syntax;
const SAME: u8 = 1;
const NEW: u8 = 2;
const REPLACE: u8 = 4;
const LENIENT: u8 = 8;
const FORWARD: u8 = 16;
const CHECKED: u8 = SAME | NEW | REPLACE | LENIENT;
struct Entry {
    mask: u8,
    location: Location,
}

pub(super) fn check(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &Types,
    attribute_lists: &[(usize, NodeId)],
    work: &Work<'_>,
) -> Result<(), Error> {
    let mut lists = syntax::collect(index, attribute_lists, work)?;
    if lists.is_empty() {
        return Ok(());
    }
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        for member in members.members(id).expect("Member") {
            work.spend(1, Some(&member.location))?;
            if let Some(entry) = lists.remove(&(member.location.source, member.attributes)) {
                conditions(definition.kind, member, &entry, types)?;
            }
        }
    }
    for entry in lists.values() {
        work.spend(1, Some(&entry.location))?;
        if entry.mask & CHECKED != 0 {
            return Err(error(
                ErrorKind::MemberAttributePlacement,
                Some(&entry.location),
            ));
        }
    }
    Ok(())
}
fn conditions(
    owner: DefinitionKind,
    member: &Member<'_>,
    entry: &Entry,
    types: &Types,
) -> Result<(), Error> {
    let mask = entry.mask;
    let attribute = member.kind == MemberKind::Attribute;
    let operation = matches!(member.kind, MemberKind::Operation(_)) && member.identifier.is_some();
    if (mask & NEW != 0 && !operation)
        || (mask & (SAME | REPLACE | LENIENT) != 0 && !attribute)
        || (mask & (REPLACE | LENIENT) != 0
            && (member.is_static || owner == DefinitionKind::Namespace))
    {
        return Err(error(
            ErrorKind::MemberAttributePlacement,
            Some(&entry.location),
        ));
    }
    if mask & (SAME | REPLACE | LENIENT) != 0 && !member.readonly {
        return Err(error(
            ErrorKind::MemberAttributeReadonly,
            Some(&member.identifier.as_ref().expect("Attributname").location),
        ));
    }
    if (mask & REPLACE != 0 && mask & (LENIENT | FORWARD) != 0)
        || (mask & LENIENT != 0 && mask & FORWARD != 0)
    {
        return Err(error(
            ErrorKind::MemberAttributeConflict,
            Some(&entry.location),
        ));
    }
    if let Some(node) = member.types[0] {
        let id = types.at(member.location.source, node).expect("Membertyp");
        let category = types.facts[id.index()].category;
        let at = &types.get(id).expect("Verwendungstyp").location;
        if attribute
            && category == Category::Promise
            && mask & (SAME | REPLACE | LENIENT | FORWARD) != 0
        {
            return Err(error(ErrorKind::MemberAttributePromise, Some(at)));
        }
        if (mask & SAME != 0 && !matches!(category, Category::Interface | Category::Object))
            || (mask & NEW != 0 && !matches!(category, Category::Interface | Category::Promise))
        {
            return Err(error(ErrorKind::MemberAttributeType, Some(at)));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
