//! Einmalige, iterative Traversierung. Extended Attributes sind undurchsichtig:
//! darin vorkommende Bezeichner sind keine normalen Typreferenzen.

use super::*;

pub(super) fn document<'a>(
    document: &Document<'a>,
    source: usize,
    raw: &mut Raw<'a>,
    work: &mut Work<'_>,
) -> Result<(), Error> {
    let mut definitions = document.root();
    while let Some(definition) = child(document, definitions, "Definition") {
        work.spend(1, None)?;
        let attributes = child(document, definitions, "ExtendedAttributeList").expect("Attribute");
        let (body, kind, partial) = concrete(document, definition, work)?;
        match kind {
            None => {
                let mut ids = document.children(body).filter(|&id| {
                    document.node(id).expect("Token").kind == Kind::Token(lexer::Kind::Identifier)
                });
                let interface = name(document, source, ids.next().expect("Includes links"));
                let mixin = name(document, source, ids.next().expect("Includes rechts"));
                work.record(&interface.location)?;
                raw.includes.push(RawInclude { interface, mixin });
            }
            Some(kind) => {
                let name = name(
                    document,
                    source,
                    identifier(document, body).expect("Definitionsname"),
                );
                // Mixins gehören nicht zu den named definitions der allgemeinen
                // Identifier-Regel. Member-Regeln folgen in einer eigenen Phase.
                if kind != DefinitionKind::Mixin && matches!(name.text, "constructor" | "toString")
                {
                    return Err(error(ErrorKind::ReservedIdentifier, Some(&name.location)));
                }
                let parent = child(document, body, "Inheritance")
                    .and_then(|id| identifier(document, id))
                    .map(|id| super::name(document, source, id));
                work.record(&name.location)?;
                let fragment = FragmentId(raw.fragments.len());
                let direct_alias = if kind == DefinitionKind::Typedef {
                    direct_alias(document, body)
                } else {
                    None
                };
                raw.fragments.push(RawFragment {
                    name,
                    kind,
                    partial,
                    body,
                    attributes,
                    parent,
                    direct_alias,
                });
                references(document, source, body, fragment, raw, work)?;
            }
        }
        definitions = child(document, definitions, "Definitions").expect("Definitionsliste");
    }
    Ok(())
}

fn concrete(
    document: &Document<'_>,
    mut id: NodeId,
    work: &Work<'_>,
) -> Result<(NodeId, Option<DefinitionKind>, bool), Error> {
    let mut partial = false;
    loop {
        work.spend(1, None)?;
        let kind = match production(document, id).expect("Produktion") {
            "Partial" => {
                partial = true;
                None
            }
            "InterfaceRest" | "PartialInterfaceRest" => Some(DefinitionKind::Interface),
            "MixinRest" => Some(DefinitionKind::Mixin),
            "Namespace" => Some(DefinitionKind::Namespace),
            "Dictionary" | "PartialDictionary" => Some(DefinitionKind::Dictionary),
            "Enum" => Some(DefinitionKind::Enum),
            "Typedef" => Some(DefinitionKind::Typedef),
            "CallbackRest" => Some(DefinitionKind::Callback),
            "CallbackRestOrInterface" if child(document, id, "CallbackRest").is_none() => {
                Some(DefinitionKind::CallbackInterface)
            }
            "IncludesStatement" => return Ok((id, None, partial)),
            _ => None,
        };
        if kind.is_some() {
            return Ok((id, kind, partial));
        }
        id = document
            .children(id)
            .find(|&id| production(document, id).is_some())
            .expect("Definitionshülle");
    }
}

fn references<'a>(
    document: &Document<'a>,
    source: usize,
    body: NodeId,
    fragment: FragmentId,
    raw: &mut Raw<'a>,
    work: &mut Work<'_>,
) -> Result<(), Error> {
    let mut pending = vec![body];
    while let Some(id) = pending.pop() {
        work.spend(1, Some(&raw.fragments[fragment.0].name.location))?;
        match production(document, id) {
            Some("ExtendedAttributeList") => continue,
            Some("DistinguishableType" | "ConstType") => {
                if let Some(id) = identifier(document, id) {
                    let name = name(document, source, id);
                    work.record(&name.location)?;
                    raw.references.push(RawReference { name, fragment });
                }
            }
            _ => {}
        }
        pending.extend(document.children(id).rev());
    }
    Ok(())
}

/// Nur der unmittelbare, nicht-nullable Identifier ist durch die spezielle
/// typedef-Regel verboten. Namen in sequence<T> usw. sind weiterhin erlaubt.
fn direct_alias(document: &Document<'_>, body: NodeId) -> Option<NodeId> {
    let ty = child(document, body, "TypeWithExtendedAttributes")?;
    let ty = child(document, ty, "Type")?;
    let ty = child(document, ty, "SingleType")?;
    let ty = child(document, ty, "DistinguishableType")?;
    if child(document, ty, "Null").is_some_and(|id| document.children(id).next().is_some()) {
        return None;
    }
    identifier(document, ty)
}
