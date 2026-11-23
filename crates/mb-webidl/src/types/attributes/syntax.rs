//! Nur echte ExtendedAttributeList-Produktionen sammeln, keine Attributinhalte.
use super::*;
type Key = (usize, NodeId);
struct Entry {
    mask: u8,
    location: Location,
}

pub(super) fn collect(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &Types,
    attribute_lists: &[(usize, NodeId)],
    work: &Work<'_>,
) -> Result<Option<Vec<u8>>, Error> {
    let mut lists = BTreeMap::new();
    for &(source, id) in attribute_lists {
        let document = index.document(source).expect("Quelle");
        let mut list = id;
        let mut entry = None;
        while let Some(attribute) = child(document, list, "ExtendedAttribute") {
            let at = location(document, source, attribute);
            work.spend(1, Some(&at))?;
            let mask = attribute_mask(document, attribute, &at, work)?;
            if mask != 0 {
                let value = entry.get_or_insert(Entry {
                    mask: 0,
                    location: at,
                });
                value.mask |= mask;
            }
            list = child(document, list, "ExtendedAttributes").expect("Attributrest");
        }
        if let Some(entry) = entry {
            lists.insert((source, id), entry);
        }
    }
    if lists.is_empty() {
        return Ok(None);
    }
    work.spend(types.len() as u64, None)?;
    let mut local = vec![0u8; types.len()];
    for (id, ty) in types.iter() {
        work.spend(1, Some(&ty.location))?;
        if let Kind::Annotated { attributes } = ty.kind {
            attach(&mut lists, (ty.location.source, attributes), id, &mut local);
        }
    }
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        for member in members.members(id).expect("Member") {
            work.spend(1, Some(&member.location))?;
            if member.kind == crate::members::Kind::DictionaryMember && !member.required {
                let ty = types
                    .at(member.location.source, member.types[0].expect("Feldtyp"))
                    .expect("Typ");
                attach(
                    &mut lists,
                    (member.location.source, member.attributes),
                    ty,
                    &mut local,
                );
            }
            if let Some(signature) = &member.signature {
                arguments(members, types, signature, &mut lists, &mut local, work)?;
            }
        }
        if let Some(signature) = members.callback(id) {
            arguments(members, types, signature, &mut lists, &mut local, work)?;
        }
    }
    if let Some((_, entry)) = lists.first_key_value() {
        return Err(error(
            ErrorKind::TypeAttributePlacement,
            Some(&entry.location),
        ));
    }
    Ok(Some(local))
}
fn attach(lists: &mut BTreeMap<Key, Entry>, key: Key, id: TypeId, local: &mut [u8]) {
    if let Some(entry) = lists.remove(&key) {
        local[id.index()] |= entry.mask;
    }
}
fn arguments(
    members: &Members<'_>,
    types: &Types,
    signature: &crate::members::Signature,
    lists: &mut BTreeMap<Key, Entry>,
    local: &mut [u8],
    work: &Work<'_>,
) -> Result<(), Error> {
    for argument in members.arguments(signature).expect("Argumente") {
        work.spend(1, Some(&argument.identifier.location))?;
        // Variadisch ist im Membermodell implizit optional; syntaktisch steht
        // dort kein optional-Schlüsselwort und die Typassoziation gilt weiter.
        if !argument.optional || argument.variadic {
            let source = argument.identifier.location.source;
            let id = types.at(source, argument.ty).expect("Argumenttyp");
            attach(lists, (source, argument.attributes), id, local);
        }
    }
    Ok(())
}
fn attribute_mask(
    document: &Document<'_>,
    root: NodeId,
    at: &Location,
    work: &Work<'_>,
) -> Result<u8, Error> {
    let mut pending = vec![root];
    let mut mask = None;
    while let Some(id) = pending.pop() {
        work.spend(1, Some(at))?;
        if let NodeKind::Token(_) = document.node(id).expect("Attributknoten").kind {
            if mask.is_some() {
                return Err(error(ErrorKind::TypeAttributeForm, Some(at)));
            }
            mask = Some(match document.text(id).expect("Token") {
                "Clamp" => CLAMP,
                "EnforceRange" => ENFORCE,
                "LegacyNullToEmptyString" => EMPTY,
                "AllowShared" => SHARED,
                "AllowResizable" => RESIZABLE,
                _ => return Ok(0),
            });
        } else {
            pending.extend(document.children(id).rev());
        }
    }
    Ok(mask.unwrap_or(0))
}
