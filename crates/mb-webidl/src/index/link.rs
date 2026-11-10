//! Auflösen nach dem Sammeln aller Quellen: Vorwärtsreferenzen und Partials
//! vor ihrer Grunddefinition benötigen keine Sonderfälle beim Parser.

use super::*;

pub(super) fn resolve<'a>(
    documents: Vec<Document<'a>>,
    raw: Raw<'a>,
    work: &mut Work<'_>,
) -> Result<Index<'a>, Error> {
    let mut index = Index {
        documents,
        definitions: Vec::new(),
        fragments: Vec::new(),
        references: Vec::new(),
        includes: Vec::new(),
        named: BTreeMap::new(),
        mixins: BTreeMap::new(),
    };
    for fragment in &raw.fragments {
        work.spend(
            fragment.name.text.len() as u64 + 1,
            Some(&fragment.name.location),
        )?;
        if fragment.partial {
            continue;
        }
        let table = if fragment.kind == DefinitionKind::Mixin {
            &mut index.mixins
        } else {
            &mut index.named
        };
        if let Some(previous) = table.get(fragment.name.text) {
            return Err(Error {
                kind: ErrorKind::DuplicateDefinition,
                location: Some(fragment.name.location.clone()),
                previous: Some(index.definitions[previous.0].location.clone()),
            });
        }
        work.record(&fragment.name.location)?;
        let id = DefinitionId(index.definitions.len());
        table.insert(fragment.name.text, id);
        index.definitions.push(Definition {
            name: fragment.name.text,
            kind: fragment.kind,
            location: fragment.name.location.clone(),
            fragments: Vec::new(),
            parent: None,
            mixins: Vec::new(),
        });
    }
    for (at, fragment) in raw.fragments.iter().enumerate() {
        work.spend(
            fragment.name.text.len() as u64 + 1,
            Some(&fragment.name.location),
        )?;
        let table = if fragment.kind == DefinitionKind::Mixin {
            &index.mixins
        } else {
            &index.named
        };
        let id = table
            .get(fragment.name.text)
            .copied()
            .ok_or_else(|| error(ErrorKind::MissingDefinition, Some(&fragment.name.location)))?;
        let definition = &mut index.definitions[id.0];
        if definition.kind != fragment.kind {
            return Err(Error {
                kind: ErrorKind::PartialKind,
                location: Some(fragment.name.location.clone()),
                previous: Some(definition.location.clone()),
            });
        }
        definition.fragments.push(FragmentId(at));
        index.fragments.push(Fragment {
            definition: id,
            partial: fragment.partial,
            location: fragment.name.location.clone(),
            body: fragment.body,
            attributes: fragment.attributes,
        });
        if let Some(parent) = &fragment.parent {
            work.spend(parent.text.len() as u64 + 1, Some(&parent.location))?;
            let parent_id = index
                .named
                .get(parent.text)
                .copied()
                .ok_or_else(|| error(ErrorKind::InvalidParent, Some(&parent.location)))?;
            let expected = fragment.kind;
            if index.definitions[parent_id.0].kind != expected {
                return Err(error(ErrorKind::InvalidParent, Some(&parent.location)));
            }
            index.definitions[id.0].parent = Some(parent_id);
        }
    }
    for include in raw.includes {
        work.spend(
            (include.interface.text.len() + include.mixin.text.len()) as u64 + 1,
            Some(&include.interface.location),
        )?;
        let interface = index
            .named
            .get(include.interface.text)
            .copied()
            .filter(|id| index.definitions[id.0].kind == DefinitionKind::Interface)
            .ok_or_else(|| {
                error(
                    ErrorKind::InvalidIncludes,
                    Some(&include.interface.location),
                )
            })?;
        let mixin = index
            .mixins
            .get(include.mixin.text)
            .copied()
            .ok_or_else(|| error(ErrorKind::InvalidIncludes, Some(&include.mixin.location)))?;
        index.definitions[interface.0].mixins.push(mixin);
        index.includes.push(Include {
            location: include.interface.location,
            interface,
            mixin,
        });
    }
    // Zyklenprüfung auf ID-Graphen statt rekursiver Expansion der Typen oder
    // Kopieren von Mixin-Membern. Jeder Graphknoten/jede Kante wird einmal besucht.
    let mut inheritance = vec![Vec::new(); index.definitions.len()];
    for (i, definition) in index.definitions.iter().enumerate() {
        if let Some(parent) = definition.parent {
            inheritance[i].push(parent);
        }
    }
    acyclic(
        &inheritance,
        &index.definitions,
        ErrorKind::InheritanceCycle,
        work,
    )?;
    let mut typedefs = vec![Vec::new(); index.definitions.len()];
    for reference in raw.references {
        let name = reference.name;
        work.spend(name.text.len() as u64 + 1, Some(&name.location))?;
        let target = index.named.get(name.text).copied().ok_or_else(|| {
            let kind = if index.mixins.contains_key(name.text) {
                ErrorKind::NotAType
            } else {
                ErrorKind::UnknownType
            };
            error(kind, Some(&name.location))
        })?;
        let target_kind = index.definitions[target.0].kind;
        if target_kind == DefinitionKind::Namespace {
            return Err(error(ErrorKind::NotAType, Some(&name.location)));
        }
        let fragment = &raw.fragments[reference.fragment.0];
        let owner = index.fragments[reference.fragment.0].definition;
        if fragment.kind == DefinitionKind::Typedef && target_kind == DefinitionKind::Typedef {
            if fragment.direct_alias == name.location.node {
                return Err(error(ErrorKind::TypedefAlias, Some(&name.location)));
            }
            typedefs[owner.0].push(target);
        }
        index.references.push(TypeReference {
            name: name.text,
            location: name.location,
            fragment: reference.fragment,
            target,
        });
    }
    acyclic(&typedefs, &index.definitions, ErrorKind::TypedefCycle, work)?;
    Ok(index)
}

fn acyclic(
    edges: &[Vec<DefinitionId>],
    definitions: &[Definition<'_>],
    kind: ErrorKind,
    work: &Work<'_>,
) -> Result<(), Error> {
    let mut colors = vec![0u8; edges.len()];
    for root in 0..edges.len() {
        work.spend(1, Some(&definitions[root].location))?;
        if colors[root] != 0 {
            continue;
        }
        colors[root] = 1;
        let mut pending = vec![(root, 0usize)];
        while let Some((node, next)) = pending.last_mut() {
            work.spend(1, Some(&definitions[*node].location))?;
            if let Some(target) = edges[*node].get(*next) {
                *next += 1;
                match colors[target.0] {
                    0 => {
                        colors[target.0] = 1;
                        pending.push((target.0, 0));
                    }
                    1 => return Err(error(kind, Some(&definitions[target.0].location))),
                    _ => {}
                }
            } else {
                colors[*node] = 2;
                pending.pop();
            }
        }
    }
    Ok(())
}
