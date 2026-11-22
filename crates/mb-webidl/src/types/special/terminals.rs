use super::*;

/// Alias- und Annotationshüllen einmal auflösen. Nullable, Unions und
/// Container bleiben andere Formen; sie sind keine zulässigen Schlüsseltypen.
pub(super) fn terminals(types: &Types, work: &Work<'_>) -> Result<Vec<Option<Atom>>, Error> {
    work.spend(types.len() as u64, None)?;
    let mut result = vec![None; types.len()];
    let mut done = vec![false; types.len()];
    for (root, _) in types.iter() {
        if done[root.index()] {
            continue;
        }
        let mut pending = vec![root];
        while let Some(&id) = pending.last() {
            let ty = types.get(id).expect("Typ");
            work.spend(1, Some(&ty.location))?;
            let inner = match ty.kind {
                Kind::Named(target) => types.typedef(target),
                Kind::Annotated { .. } => Some(types.children(id).expect("Annotation")[0]),
                _ => None,
            };
            if let Some(inner) = inner {
                if !done[inner.index()] {
                    pending.push(inner);
                    continue;
                }
                result[id.index()] = result[inner.index()];
            } else if let Kind::Atom(atom) = ty.kind {
                result[id.index()] = Some(atom);
            }
            done[id.index()] = true;
            pending.pop();
        }
    }
    Ok(result)
}
