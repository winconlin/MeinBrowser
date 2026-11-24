//! Fünf standardisierte Typattribute; weitere Extended Attributes bleiben opaque.
//! Assoziationen sind zustandsabhängig und ändern nie die Fakten geteilter Aliasse.

use super::*;
mod syntax;

pub(super) const INTEGER: u8 = 1;
pub(super) const STRING: u8 = 2;
pub(super) const BUFFER: u8 = 4;
pub(super) const VIEW: u8 = 8;
const CLAMP: u8 = 1;
const ENFORCE: u8 = 2;
const EMPTY: u8 = 4;
const SHARED: u8 = 8;
const RESIZABLE: u8 = 16;

#[derive(Clone, Copy)]
struct State {
    id: TypeId,
    mask: u8,
    anchor: TypeId,
    readonly: Option<TypeId>,
}
struct Graph<'a, 'b> {
    types: &'a Types,
    work: &'a Work<'b>,
    local: Vec<u8>,
    seen: Vec<u64>,
    pending: Vec<State>,
}
pub(super) fn check(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &Types,
    attribute_lists: &[(usize, NodeId)],
    work: &Work<'_>,
) -> Result<(), Error> {
    let Some(local) = syntax::collect(index, members, types, attribute_lists, work)? else {
        return Ok(());
    };
    work.spend(types.len() as u64, None)?;
    let mut graph = Graph {
        types,
        work,
        local,
        seen: vec![0; types.len()],
        pending: Vec::new(),
    };
    for (id, _) in types.iter() {
        graph.run(State {
            id,
            mask: 0,
            anchor: id,
            readonly: None,
        })?;
    }
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        for member in members.members(id).expect("Member") {
            work.spend(1, Some(&member.location))?;
            if member.kind == crate::members::Kind::Attribute && member.readonly {
                let id = types
                    .at(
                        member.location.source,
                        member.types[0].expect("Attributtyp"),
                    )
                    .expect("Typ");
                graph.run(State {
                    id,
                    mask: 0,
                    anchor: id,
                    readonly: Some(id),
                })?;
            }
        }
    }
    Ok(())
}
impl Graph<'_, '_> {
    fn push(&mut self, mut state: State) -> Result<(), Error> {
        let at = &self.types.get(state.id).expect("Typ").location;
        self.work.spend(1, Some(at))?;
        if state.mask == 0 && self.local[state.id.index()] != 0 {
            state.anchor = state.id;
        }
        state.mask |= self.local[state.id.index()];
        let bit = 1u64 << (state.mask + if state.readonly.is_some() { 32 } else { 0 });
        if self.seen[state.id.index()] & bit == 0 {
            self.seen[state.id.index()] |= bit;
            self.pending.push(state);
        }
        Ok(())
    }
    fn run(&mut self, root: State) -> Result<(), Error> {
        self.push(root)?;
        while let Some(state) = self.pending.pop() {
            let ty = self.types.get(state.id).expect("Typ");
            self.work.spend(1, Some(&ty.location))?;
            let flags = self.types.facts[state.id.index()].attribute_targets;
            let kind = if state.mask & (CLAMP | ENFORCE) == CLAMP | ENFORCE {
                Some(ErrorKind::TypeAttributeConflict)
            } else if (state.mask & (CLAMP | ENFORCE) != 0 && flags & INTEGER == 0)
                || (state.mask & EMPTY != 0 && flags & STRING == 0)
                || (state.mask & SHARED != 0 && flags & VIEW == 0)
                || (state.mask & RESIZABLE != 0 && flags & BUFFER == 0)
            {
                Some(ErrorKind::TypeAttributeTarget)
            } else {
                None
            };
            if let Some(kind) = kind {
                return Err(error(
                    kind,
                    Some(
                        &self
                            .types
                            .get(state.anchor)
                            .expect("Assoziationsort")
                            .location,
                    ),
                ));
            }
            if let Some(id) = state.readonly
                && state.mask & (CLAMP | ENFORCE) != 0
            {
                return Err(error(
                    ErrorKind::TypeAttributeReadonly,
                    Some(&self.types.get(id).expect("Readonly-Verwendung").location),
                ));
            }
            if let Kind::Named(target) = ty.kind {
                if let Some(id) = self.types.typedef(target) {
                    self.push(State { id, ..state })?;
                }
            } else {
                let transparent = matches!(
                    ty.kind,
                    Kind::Annotated { .. } | Kind::Nullable | Kind::Union
                );
                for &id in self.types.children(state.id).expect("Kindtypen") {
                    self.push(State {
                        id,
                        mask: if transparent { state.mask } else { 0 },
                        anchor: if transparent { state.anchor } else { id },
                        readonly: state.readonly,
                    })?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
