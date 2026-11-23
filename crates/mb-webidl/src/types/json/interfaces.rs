use super::*;

/// Das Vorhandensein der Operation ist eine nominelle JSON-Typbedingung.
/// Ihr Ergebnis wird erst nach der gemeinsamen Typklassifikation geprüft.
pub(super) fn build(
    index: &Index<'_>,
    members: &Members<'_>,
    work: &Work<'_>,
) -> Result<BTreeMap<DefinitionId, bool>, Error> {
    let mut local = BTreeMap::new();
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        let mut found = false;
        for member in members.members(id).expect("Member") {
            work.spend(1, Some(&member.location))?;
            if operation(member) {
                let arguments = members
                    .arguments(member.signature.as_ref().expect("Signatur"))
                    .expect("Argumente");
                if let Some(argument) = arguments.first() {
                    return Err(error(
                        ErrorKind::ToJSONArguments,
                        Some(&argument.identifier.location),
                    ));
                }
                found = true;
            }
        }
        if matches!(
            definition.kind,
            DefinitionKind::Interface | DefinitionKind::Mixin
        ) {
            local.insert(id, found);
        }
    }
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if definition.kind != DefinitionKind::Interface {
            continue;
        }
        for mixin in &definition.mixins {
            work.spend(1, Some(&definition.location))?;
            let value = local[mixin];
            *local.get_mut(&id).expect("Interface") |= value;
        }
    }
    let mut result = BTreeMap::new();
    for (root, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if definition.kind != DefinitionKind::Interface {
            continue;
        }
        let mut cursor = Some(root);
        let mut pending = Vec::new();
        let mut inherited = false;
        while let Some(id) = cursor {
            work.spend(1, None)?;
            if let Some(&known) = result.get(&id) {
                inherited = known;
                break;
            }
            pending.push(id);
            cursor = index.definition(id).expect("Interface").parent;
        }
        while let Some(id) = pending.pop() {
            work.spend(1, None)?;
            inherited |= local[&id];
            result.insert(id, inherited);
        }
    }
    Ok(result)
}
