//! WebIDL, Dictionaries: Der Typ eines Feldes darf sein Dictionary nicht
//! einschließen. Ein gemeinsamer impliziter Graph ersetzt separate, potenziell
//! quadratische Einschlussprüfungen für jedes Feld und jeden Vorfahren.

use super::*;

struct Dictionary {
    id: DefinitionId,
    parent: Option<DefinitionId>,
}

#[derive(Clone, Copy)]
enum Node {
    Type(TypeId),
    Dictionary(usize),
}

struct Frame<'a> {
    node: Node,
    next: usize,
    /// Nur auf der Kante von einem Dictionary zu seinem eigenen Feldtyp.
    field: Option<&'a Location>,
}

pub(super) fn check(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &Types,
    work: &Work<'_>,
) -> Result<(), Error> {
    let mut dictionaries = Vec::new();
    let mut ids = BTreeMap::new();
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        if definition.kind != DefinitionKind::Dictionary {
            continue;
        }
        if dictionaries.len() >= work.limits.dictionary_nodes {
            return Err(error(
                ErrorKind::Limit(Limit::DictionaryNodes),
                Some(&definition.location),
            ));
        }
        ids.insert(id, dictionaries.len());
        dictionaries.push(Dictionary {
            id,
            parent: definition.parent,
        });
    }
    if dictionaries.is_empty() {
        return Ok(());
    }
    work.spend((types.len() + dictionaries.len()) as u64, None)?;
    let mut type_colors = vec![0u8; types.len()];
    let mut dictionary_colors = vec![0u8; dictionaries.len()];
    let mut edges = 0usize;
    for root in 0..dictionaries.len() {
        work.spend(1, None)?;
        if dictionary_colors[root] != 0 {
            continue;
        }
        let mut pending = vec![Frame {
            node: Node::Dictionary(root),
            next: 0,
            field: None,
        }];
        dictionary_colors[root] = 1;
        while let Some(frame) = pending.last_mut() {
            work.spend(1, None)?;
            let neighbor = match frame.node {
                Node::Type(id) => {
                    type_neighbor(types, &ids, id, frame.next).map(|node| (node, None))
                }
                Node::Dictionary(id) => {
                    let dictionary = &dictionaries[id];
                    let has_parent = usize::from(dictionary.parent.is_some());
                    if frame.next < has_parent {
                        let parent = dictionary.parent.expect("Vorfahre");
                        Some((
                            Node::Dictionary(*ids.get(&parent).expect("Dictionary-Vorfahre")),
                            None,
                        ))
                    } else {
                        let fields = members.members(dictionary.id).expect("zugehörige Felder");
                        fields.get(frame.next - has_parent).map(|field| {
                            let ty = types
                                .at(field.location.source, field.types[0].expect("Feldtyp"))
                                .expect("aufgebauter Feldtyp");
                            let at = &field.identifier.as_ref().expect("Feldname").location;
                            (Node::Type(ty), Some(at))
                        })
                    }
                }
            };
            if let Some((node, field)) = neighbor {
                if edges >= work.limits.dictionary_edges {
                    return Err(error(ErrorKind::Limit(Limit::DictionaryEdges), field));
                }
                edges += 1;
                frame.next += 1;
                let color = match node {
                    Node::Type(id) => &mut type_colors[id.index()],
                    Node::Dictionary(id) => &mut dictionary_colors[id],
                };
                match *color {
                    0 => {
                        *color = 1;
                        pending.push(Frame {
                            node,
                            next: 0,
                            field,
                        });
                    }
                    1 => {
                        // Typedef- und reine Vererbungszyklen wurden schon vor
                        // dieser Stufe abgelehnt. Jeder Zyklus hier enthält
                        // daher eine Feldkante. Der letzte aktive Feldeintritt
                        // liegt im Zyklus und liefert eine echte Fundstelle.
                        let at = field
                            .or_else(|| pending.iter().rev().find_map(|frame| frame.field))
                            .expect("Feld im Dictionary-Zyklus");
                        return Err(error(ErrorKind::DictionaryCycle, Some(at)));
                    }
                    _ => {}
                }
            } else {
                match frame.node {
                    Node::Type(id) => type_colors[id.index()] = 2,
                    Node::Dictionary(id) => dictionary_colors[id] = 2,
                }
                pending.pop();
            }
        }
    }
    Ok(())
}

/// Nur die im Standard genannten Einschlusskanten verfolgen. Insbesondere
/// Promise, AsyncSequence und ObservableArray sind keine Sequence-Kanten.
fn type_neighbor(
    types: &Types,
    dictionaries: &BTreeMap<DefinitionId, usize>,
    id: TypeId,
    next: usize,
) -> Option<Node> {
    match types.get(id).expect("lokaler Typ").kind {
        Kind::Named(target) if next == 0 => dictionaries
            .get(&target)
            .map(|&id| Node::Dictionary(id))
            .or_else(|| types.typedef(target).map(Node::Type)),
        Kind::Annotated { .. }
        | Kind::Nullable
        | Kind::Union
        | Kind::Record(_)
        | Kind::Generic(Generic::Sequence | Generic::FrozenArray) => types
            .children(id)
            .expect("Kindtypen")
            .get(next)
            .copied()
            .map(Node::Type),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
