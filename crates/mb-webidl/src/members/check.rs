use super::*;

pub(super) fn member_name(member: &Member<'_>) -> Result<(), Error> {
    if member.identifier.is_none() && member.kind == Kind::Operation(Special::Regular) {
        return Err(error(
            ErrorKind::MissingOperationName,
            Some(&member.location),
        ));
    }
    if let Some(id) = &member.identifier {
        let forbidden = matches!(id.name, "constructor" | "toString")
            || id.name.starts_with('_')
            || (member.kind == Kind::Constant
                && matches!(id.name, "length" | "name" | "prototype"))
            || (member.is_static && id.name == "prototype");
        if forbidden {
            return Err(error(ErrorKind::ReservedIdentifier, Some(&id.location)));
        }
    }
    Ok(())
}

pub(super) fn check(
    index: &Index<'_>,
    model: &Members<'_>,
    work: &mut Work<'_>,
) -> Result<(), Error> {
    for (id, definition) in index.definitions() {
        work.visit(&definition.location)?;
        if matches!(
            definition.kind,
            DefinitionKind::Interface
                | DefinitionKind::Mixin
                | DefinitionKind::CallbackInterface
                | DefinitionKind::Namespace
        ) {
            let mut seen = BTreeMap::new();
            let restricted = matches!(
                definition.kind,
                DefinitionKind::Interface | DefinitionKind::Mixin
            );
            member_set(
                model.members(id).expect("eigene Member"),
                &mut seen,
                restricted,
                work,
            )?;
            for &mixin in &definition.mixins {
                work.visit(&index.definition(mixin).expect("Mixin").location)?;
                member_set(
                    model.members(mixin).expect("Mixinmember"),
                    &mut seen,
                    restricted,
                    work,
                )?;
            }
        }
    }
    dictionaries(index, model, work)
}

#[derive(Default)]
struct Seen<'m, 'a> {
    non_operation: Option<&'m Member<'a>>,
    regular: Option<&'m Member<'a>>,
    static_operation: Option<&'m Member<'a>>,
}

fn member_set<'m, 'a>(
    members: &'m [Member<'a>],
    seen: &mut BTreeMap<&'a str, Seen<'m, 'a>>,
    restricted: bool,
    work: &mut Work<'_>,
) -> Result<(), Error> {
    for member in members {
        work.visit(&member.location)?;
        let Some(identifier) = &member.identifier else {
            continue;
        };
        work.spend(identifier.name.len() as u64, Some(&identifier.location))?;
        let entry = seen.entry(identifier.name).or_default();
        if let Some(previous) = entry.non_operation {
            return Err(duplicate(
                ErrorKind::DuplicateMember,
                &identifier.location,
                &previous.identifier.as_ref().expect("Name").location,
            ));
        }
        if matches!(member.kind, Kind::Operation(_)) {
            let slot = if member.is_static {
                &mut entry.static_operation
            } else {
                &mut entry.regular
            };
            if let Some(previous) = slot {
                if restricted && previous.fragment != member.fragment {
                    return Err(duplicate(
                        ErrorKind::CrossFragmentOverload,
                        &identifier.location,
                        &previous.identifier.as_ref().expect("Name").location,
                    ));
                }
            } else {
                *slot = Some(member);
            }
        } else {
            if let Some(previous) = entry.regular.or(entry.static_operation) {
                return Err(duplicate(
                    ErrorKind::DuplicateMember,
                    &identifier.location,
                    &previous.identifier.as_ref().expect("Name").location,
                ));
            }
            entry.non_operation = Some(member);
        }
    }
    Ok(())
}

/// Ein iterativer Walddurchlauf mit Rücknahme der lokalen Namen: gemeinsame
/// Vorfahren werden nicht für jeden Nachfahren neu kopiert oder durchlaufen.
fn dictionaries(index: &Index<'_>, model: &Members<'_>, work: &mut Work<'_>) -> Result<(), Error> {
    let mut children: BTreeMap<DefinitionId, Vec<DefinitionId>> = BTreeMap::new();
    let mut roots = Vec::new();
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if definition.kind != DefinitionKind::Dictionary {
            continue;
        }
        if let Some(parent) = definition.parent {
            children.entry(parent).or_default().push(id);
        } else {
            roots.push(id);
        }
    }
    enum Task {
        Enter(DefinitionId),
        Leave(DefinitionId),
    }
    let mut pending: Vec<_> = roots.into_iter().rev().map(Task::Enter).collect();
    let mut names = BTreeMap::new();
    while let Some(task) = pending.pop() {
        let (Task::Enter(id) | Task::Leave(id)) = task;
        work.visit(&index.definition(id).expect("Dictionary").location)?;
        let members = model.members(id).expect("Dictionarymember");
        match task {
            Task::Enter(_) => {
                for member in members {
                    let identifier = member.identifier.as_ref().expect("Feldname");
                    work.visit(&identifier.location)?;
                    work.spend(identifier.name.len() as u64, Some(&identifier.location))?;
                    if let Some(previous) = names.insert(identifier.name, &identifier.location) {
                        return Err(duplicate(
                            ErrorKind::DuplicateDictionaryMember,
                            &identifier.location,
                            previous,
                        ));
                    }
                }
                pending.push(Task::Leave(id));
                if let Some(children) = children.get(&id) {
                    pending.extend(children.iter().rev().map(|&id| Task::Enter(id)));
                }
            }
            Task::Leave(_) => {
                for member in members {
                    let identifier = member.identifier.as_ref().expect("Feldname");
                    work.visit(&identifier.location)?;
                    work.spend(identifier.name.len() as u64, Some(&identifier.location))?;
                    names.remove(identifier.name);
                }
            }
        }
    }
    Ok(())
}
