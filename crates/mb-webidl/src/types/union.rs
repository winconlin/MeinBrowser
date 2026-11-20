//! Paarweise Unterscheidbarkeit der abgeflachten Unionglieder nach der
//! normativen Tabelle. Nullable-/Dictionary-Vorbedingungen prüft check.rs.

use super::*;

mod metadata;
use metadata::{Interval, callback_legacy, interfaces};

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Class {
    Undefined,
    Boolean,
    Numeric,
    BigInt,
    String,
    Object,
    Symbol,
    Interface,
    Callback,
    Dictionary,
    AsyncSequence,
    Sequence,
}
const CLASSES: [Class; 12] = [
    Class::Undefined,
    Class::Boolean,
    Class::Numeric,
    Class::BigInt,
    Class::String,
    Class::Object,
    Class::Symbol,
    Class::Interface,
    Class::Callback,
    Class::Dictionary,
    Class::AsyncSequence,
    Class::Sequence,
];

#[derive(Clone, Copy)]
struct Leaf {
    class: Class,
    detail: Detail,
}
#[derive(Clone, Copy)]
enum Detail {
    Plain,
    Interface(DefinitionId),
    Buffer(Buffer),
    Callback(bool),
}

pub(super) fn check(index: &Index<'_>, types: &Types, work: &Work<'_>) -> Result<(), Error> {
    work.spend(types.len() as u64, None)?;
    if !types.iter().any(|(_, ty)| ty.kind == Kind::Union) {
        return Ok(());
    }
    // Transparente Elternkanten bestimmen: Unionparameter in Generics/Records
    // bilden eigene Wurzeln, Typedef-Verweise dagegen gehören zum Unionpfad.
    let mut incoming = vec![false; types.len()];
    for (id, ty) in types.iter() {
        work.spend(1, Some(&ty.location))?;
        if let Some(target) = alias(types, id, ty.kind) {
            incoming[target.index()] = true;
        } else if matches!(ty.kind, Kind::Union) {
            for &child in types.children(id).expect("Unionglieder") {
                work.spend(1, Some(&ty.location))?;
                incoming[child.index()] = true;
            }
        }
    }
    let mut context = Context {
        index,
        types,
        work,
        callbacks: BTreeMap::new(),
        interfaces: None,
        visits: 0,
    };
    let mut checked = vec![false; types.len()];
    let mut seen = vec![0u32; types.len()];
    for (id, _) in types.iter().rev() {
        if incoming[id.index()] {
            continue;
        }
        let mut root = id;
        loop {
            context.visit(root)?;
            let kind = types.get(root).expect("lokaler Typ").kind;
            if let Some(inner) = alias(types, root, kind) {
                root = inner;
            } else {
                break;
            }
        }
        if types.get(root).expect("Typ").kind != Kind::Union || checked[root.index()] {
            continue;
        }
        let epoch = root.0.get();
        let mut pending = vec![root];
        let mut covered = Vec::new();
        let mut state = Seen::default();
        while let Some(id) = pending.pop() {
            context.visit(id)?;
            if seen[id.index()] == epoch {
                return Err(error(
                    ErrorKind::IndistinguishableUnion,
                    Some(&types.get(root).expect("Union").location),
                ));
            }
            seen[id.index()] = epoch;
            let kind = types.get(id).expect("lokaler Typ").kind;
            if let Some(inner) = alias(types, id, kind) {
                pending.push(inner);
            } else if kind == Kind::Union {
                covered.push(id);
                pending.extend(
                    types
                        .children(id)
                        .expect("Unionglieder")
                        .iter()
                        .rev()
                        .copied(),
                );
            } else {
                let leaf = context.classify(id)?.ok_or_else(|| {
                    error(
                        ErrorKind::IndistinguishableUnion,
                        Some(&types.get(root).expect("Union").location),
                    )
                })?;
                if !state.insert(leaf, &mut context)? {
                    return Err(error(
                        ErrorKind::IndistinguishableUnion,
                        Some(&types.get(root).expect("Union").location),
                    ));
                }
            }
        }
        // Jede enthaltene Union ist eine Teilmenge der gerade geprüften Glieder.
        for id in covered {
            checked[id.index()] = true;
        }
    }
    Ok(())
}

fn alias(types: &Types, id: TypeId, kind: Kind) -> Option<TypeId> {
    match kind {
        Kind::Named(id) => types.typedef(id),
        Kind::Annotated { .. } | Kind::Nullable => Some(types.children(id).expect("Hüllentyp")[0]),
        _ => None,
    }
}

struct Context<'a, 's, 'b> {
    index: &'a Index<'s>,
    types: &'a Types,
    work: &'a Work<'b>,
    callbacks: BTreeMap<DefinitionId, bool>,
    interfaces: Option<BTreeMap<DefinitionId, Interval>>,
    visits: usize,
}
impl Context<'_, '_, '_> {
    fn visit(&mut self, id: TypeId) -> Result<(), Error> {
        let at = &self.types.get(id).expect("Typ").location;
        self.work.spend(1, Some(at))?;
        if self.visits >= self.work.limits.union_visits {
            return Err(error(ErrorKind::Limit(Limit::UnionVisits), Some(at)));
        }
        self.visits += 1;
        Ok(())
    }
    fn classify(&mut self, id: TypeId) -> Result<Option<Leaf>, Error> {
        let kind = self.types.get(id).expect("Blatttyp").kind;
        let mut detail = Detail::Plain;
        let class = match kind {
            Kind::Atom(Atom::Undefined) => Class::Undefined,
            Kind::Atom(Atom::Primitive(Primitive::Boolean)) => Class::Boolean,
            Kind::Atom(Atom::Primitive(Primitive::BigInt)) => Class::BigInt,
            Kind::Atom(Atom::Primitive(_)) => Class::Numeric,
            Kind::Atom(Atom::String(_)) => Class::String,
            Kind::Atom(Atom::Object) => Class::Object,
            Kind::Atom(Atom::Symbol) => Class::Symbol,
            Kind::Atom(Atom::Buffer(buffer)) => {
                detail = Detail::Buffer(buffer);
                Class::Interface
            }
            Kind::Named(target) => match self.index.definition(target).expect("Typdefinition").kind
            {
                DefinitionKind::Interface => {
                    detail = Detail::Interface(target);
                    Class::Interface
                }
                DefinitionKind::Enum => Class::String,
                DefinitionKind::Dictionary | DefinitionKind::CallbackInterface => Class::Dictionary,
                DefinitionKind::Callback => {
                    let legacy = if let Some(&value) = self.callbacks.get(&target) {
                        value
                    } else {
                        let value = callback_legacy(self.index, target, self.work)?;
                        self.callbacks.insert(target, value);
                        value
                    };
                    detail = Detail::Callback(legacy);
                    Class::Callback
                }
                _ => unreachable!("aufgelöster Typedef oder nicht verwendbarer Typ"),
            },
            Kind::Generic(Generic::Sequence | Generic::FrozenArray) => Class::Sequence,
            Kind::Generic(Generic::AsyncSequence) => Class::AsyncSequence,
            Kind::Record(_) => Class::Dictionary,
            _ => return Ok(None),
        };
        Ok(Some(Leaf { class, detail }))
    }
    fn interval(&mut self, id: DefinitionId) -> Result<Interval, Error> {
        if self.interfaces.is_none() {
            self.interfaces = Some(interfaces(self.index, self.work)?);
        }
        Ok(*self
            .interfaces
            .as_ref()
            .expect("Interface-Intervalle")
            .get(&id)
            .expect("Interface"))
    }
}

#[derive(Default)]
struct Seen {
    classes: u16,
    buffers: u16,
    legacy_callback: bool,
    interfaces: BTreeMap<usize, usize>,
}
impl Seen {
    fn insert(&mut self, leaf: Leaf, context: &mut Context<'_, '_, '_>) -> Result<bool, Error> {
        for class in CLASSES {
            if self.classes & (1 << class as u8) != 0 && !table(class, leaf.class) {
                return Ok(false);
            }
        }
        if (leaf.class == Class::Dictionary && self.legacy_callback)
            || (matches!(leaf.detail, Detail::Callback(true))
                && self.classes & (1 << Class::Dictionary as u8) != 0)
        {
            return Ok(false);
        }
        match leaf.detail {
            Detail::Buffer(buffer) => {
                let bit = 1 << buffer as u8;
                if self.buffers & bit != 0 {
                    return Ok(false);
                }
                self.buffers |= bit;
            }
            Detail::Interface(id) => {
                let Interval { start, end } = context.interval(id)?;
                if self
                    .interfaces
                    .range(..=start)
                    .next_back()
                    .is_some_and(|(_, &old_end)| old_end > start)
                    || self
                        .interfaces
                        .range(start..)
                        .next()
                        .is_some_and(|(&old_start, _)| old_start < end)
                {
                    return Ok(false);
                }
                self.interfaces.insert(start, end);
            }
            Detail::Callback(legacy) => self.legacy_callback = legacy,
            Detail::Plain => {}
        }
        self.classes |= 1 << leaf.class as u8;
        Ok(true)
    }
}

/// Die symmetrischen leeren Felder der normativen 12-Kategorien-Tabelle.
/// Die Fälle a/c werden zusätzlich geprüft; b/d bleiben unterscheidbar.
fn table(a: Class, b: Class) -> bool {
    if a == b {
        return a == Class::Interface;
    }
    if matches!(
        (a, b),
        (Class::Undefined, Class::Dictionary)
            | (Class::Dictionary, Class::Undefined)
            | (Class::AsyncSequence, Class::Sequence)
            | (Class::Sequence, Class::AsyncSequence)
    ) {
        return false;
    }
    if a == Class::Object || b == Class::Object {
        let other = if a == Class::Object { b } else { a };
        return !matches!(
            other,
            Class::Interface
                | Class::Callback
                | Class::Dictionary
                | Class::AsyncSequence
                | Class::Sequence
        );
    }
    true
}

#[cfg(test)]
mod tests;
