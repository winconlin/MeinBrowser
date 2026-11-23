//! Verwendungsorte referenzbasierter Arrays und ObservableArray-Elemente.
//! Die topologisch berechneten Fakten folgen Alias-/Annotations-/Nullable-/
//! Unionkanten; Containerparameter sind eigene Verwendungsorte.

use super::*;
use crate::members::Kind as MemberKind;

pub(super) const FROZEN: u8 = 1;
pub(super) const OBSERVABLE: u8 = 2;

pub(super) fn check(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &Types,
    work: &Work<'_>,
) -> Result<(), Error> {
    for (id, ty) in types.iter() {
        work.spend(1, Some(&ty.location))?;
        if !matches!(ty.kind, Kind::Generic(_) | Kind::Record(_)) {
            continue;
        }
        for &child in types.children(id).expect("Containerparameter") {
            let at = &types.get(child).expect("Parameter").location;
            work.spend(1, Some(at))?;
            if ty.kind == Kind::Generic(Generic::ObservableArray)
                && types.facts[child.index()].observable_element_forbidden
            {
                return Err(error(ErrorKind::ObservableArrayElement, Some(at)));
            }
            usage(types, child, 0, work)?;
        }
    }
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        for member in members.members(id).expect("Member") {
            work.spend(1, Some(&member.location))?;
            let allowed = if member.kind == MemberKind::Attribute
                && matches!(
                    definition.kind,
                    DefinitionKind::Interface | DefinitionKind::Mixin
                ) {
                FROZEN | if member.is_static { 0 } else { OBSERVABLE }
            } else {
                0
            };
            for node in member.types.iter().flatten() {
                let ty = types.at(member.location.source, *node).expect("Membertyp");
                usage(types, ty, allowed, work)?;
            }
            if let Some(signature) = &member.signature {
                arguments(members, types, signature, work)?;
            }
        }
        if let Some(signature) = members.callback(id) {
            if let Some(node) = signature.result {
                // Callback-Signaturen gehören zum primären Fragment.
                let fragment = index.fragment(definition.fragments[0]).expect("Callback");
                let ty = types
                    .at(fragment.location.source, node)
                    .expect("Callback-Rückgabe");
                usage(types, ty, 0, work)?;
            }
            arguments(members, types, signature, work)?;
        }
    }
    Ok(())
}

fn arguments(
    members: &Members<'_>,
    types: &Types,
    signature: &crate::members::Signature,
    work: &Work<'_>,
) -> Result<(), Error> {
    for argument in members.arguments(signature).expect("Argumente") {
        let ty = types
            .at(argument.identifier.location.source, argument.ty)
            .expect("Argumenttyp");
        usage(types, ty, 0, work)?;
    }
    Ok(())
}

fn usage(types: &Types, id: TypeId, allowed: u8, work: &Work<'_>) -> Result<(), Error> {
    let at = &types.get(id).expect("Verwendungstyp").location;
    work.spend(1, Some(at))?;
    let forbidden = types.facts[id.index()].array_use & !allowed;
    let kind = if forbidden & FROZEN != 0 {
        Some(ErrorKind::FrozenArrayUse)
    } else if forbidden & OBSERVABLE != 0 {
        Some(ErrorKind::ObservableArrayUse)
    } else {
        None
    };
    if let Some(kind) = kind {
        return Err(error(kind, Some(at)));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
