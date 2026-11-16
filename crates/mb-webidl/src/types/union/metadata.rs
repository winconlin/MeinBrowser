use super::*;

#[derive(Clone, Copy)]
pub(super) struct Interval {
    pub start: usize,
    pub end: usize,
}

pub(super) fn interfaces(
    index: &Index<'_>,
    work: &Work<'_>,
) -> Result<BTreeMap<DefinitionId, Interval>, Error> {
    let mut children: BTreeMap<DefinitionId, Vec<DefinitionId>> = BTreeMap::new();
    let mut roots = Vec::new();
    let mut count = 0;
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if definition.kind != DefinitionKind::Interface {
            continue;
        }
        if count >= work.limits.union_interfaces {
            return Err(error(
                ErrorKind::Limit(Limit::UnionInterfaces),
                Some(&definition.location),
            ));
        }
        count += 1;
        if let Some(parent) = definition.parent {
            children.entry(parent).or_default().push(id);
        } else {
            roots.push(id);
        }
    }
    work.spend(count as u64, None)?;
    let mut pending: Vec<_> = roots.into_iter().rev().map(|id| (id, false)).collect();
    let mut result: BTreeMap<DefinitionId, Interval> = BTreeMap::new();
    let mut clock = 0;
    while let Some((id, leaving)) = pending.pop() {
        work.spend(1, None)?;
        if leaving {
            result.get_mut(&id).expect("Interface-Eintritt").end = clock;
        } else {
            result.insert(
                id,
                Interval {
                    start: clock,
                    end: clock + 1,
                },
            );
            clock += 1;
            pending.push((id, true));
            if let Some(children) = children.get(&id) {
                pending.extend(children.iter().rev().map(|&id| (id, false)));
            }
        }
    }
    Ok(result)
}

/// Nur das für Tabelle (c) benötigte Definitionsattribut auswerten. Andere
/// Attribute bleiben unangetastete Syntaxdaten der vorherigen Stufen.
pub(super) fn callback_legacy(
    index: &Index<'_>,
    id: DefinitionId,
    work: &Work<'_>,
) -> Result<bool, Error> {
    let definition = index.definition(id).expect("Callback");
    let fragment = index
        .fragment(definition.fragments[0])
        .expect("Callbackdefinition");
    let document = index.document(fragment.location.source).expect("Quelle");
    let mut list = fragment.attributes;
    let mut found = false;
    while let Some(attribute) = child(document, list, "ExtendedAttribute") {
        work.spend(1, Some(&definition.location))?;
        found |= legacy_attribute(document, fragment.location.source, attribute, work)?;
        list = child(document, list, "ExtendedAttributes").expect("Attributrest");
    }
    Ok(found)
}

fn legacy_attribute(
    document: &Document<'_>,
    source: usize,
    attribute: NodeId,
    work: &Work<'_>,
) -> Result<bool, Error> {
    let at = location(document, source, attribute);
    let mut pending = vec![attribute];
    let mut found = false;
    while let Some(id) = pending.pop() {
        work.spend(1, Some(&at))?;
        if let NodeKind::Token(kind) = document.node(id).expect("Attributknoten").kind {
            if found {
                return Err(error(ErrorKind::InvalidCallbackAttribute, Some(&at)));
            }
            if kind != lexer::Kind::Identifier
                || document.text(id) != Some("LegacyTreatNonObjectAsNull")
            {
                return Ok(false);
            }
            found = true;
        } else {
            pending.extend(document.children(id).rev());
        }
    }
    Ok(found)
}
