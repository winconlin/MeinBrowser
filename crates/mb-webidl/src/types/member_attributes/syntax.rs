//! Exakte Namen in echten Listen; unbekannte Attributinhalte bleiben opaque.
use super::*;
pub(super) fn collect(
    index: &Index<'_>,
    lists: &[(usize, NodeId)],
    work: &Work<'_>,
) -> Result<BTreeMap<(usize, NodeId), Entry>, Error> {
    let mut result = BTreeMap::new();
    for &(source, root) in lists {
        let document = index.document(source).expect("Quelle");
        let mut list = root;
        let mut entry = None;
        while let Some(attribute) = child(document, list, "ExtendedAttribute") {
            let at = location(document, source, attribute);
            work.spend(1, Some(&at))?;
            let mask = mask(document, attribute, &at, work)?;
            if mask != 0 {
                entry
                    .get_or_insert(Entry {
                        mask: 0,
                        location: at,
                    })
                    .mask |= mask;
            }
            list = child(document, list, "ExtendedAttributes").expect("Attributrest");
        }
        if let Some(entry) = entry {
            result.insert((source, root), entry);
        }
    }
    Ok(result)
}
fn mask(
    document: &Document<'_>,
    root: NodeId,
    at: &Location,
    work: &Work<'_>,
) -> Result<u8, Error> {
    let mut pending = vec![root];
    let mut found = None;
    while let Some(id) = pending.pop() {
        work.spend(1, Some(at))?;
        if let NodeKind::Token(_) = document.node(id).expect("Attributknoten").kind {
            if found.is_some() {
                return Err(error(ErrorKind::MemberAttributeForm, Some(at)));
            }
            found = Some(match document.text(id).expect("Token") {
                "SameObject" => SAME,
                "NewObject" => NEW,
                "Replaceable" => REPLACE,
                "LegacyLenientSetter" => LENIENT,
                // Nur Präsenz für die hier normierten Konflikt-/Promise-Regeln.
                // Argumentform, Zielauflösung und Kettenzyklen folgen gesondert.
                "PutForwards" => return Ok(FORWARD),
                _ => return Ok(0),
            });
        } else {
            pending.extend(document.children(id).rev());
        }
    }
    Ok(found.unwrap_or(0))
}
