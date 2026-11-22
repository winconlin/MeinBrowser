//! On-demand-Vergleich der IDL-Grundtypen, ohne Laufzeitkonvertierung.

use super::*;
use std::collections::BTreeSet;
type Pair = (TypeId, TypeId);

pub(super) struct Identity<'a, 'b> {
    types: &'a Types,
    work: &'a Work<'b>,
    cores: Vec<TypeId>,
    cache: BTreeMap<Pair, bool>,
}
#[derive(Default)]
struct Search {
    seen: BTreeSet<Pair>,
    pending: Vec<Pair>,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Primitive(u8),
    String(u8),
    Buffer(u8),
    Any,
    Undefined,
    Object,
    Symbol,
    Named(DefinitionId),
    Generic(u8),
    Record(u8),
}

impl<'a, 'b> Identity<'a, 'b> {
    pub fn new(types: &'a Types, work: &'a Work<'b>) -> Result<Self, Error> {
        work.spend(types.len() as u64, None)?;
        let mut cores: Vec<_> = types.iter().map(|(id, _)| id).collect();
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
                    cores[id.index()] = cores[inner.index()];
                }
                done[id.index()] = true;
                pending.pop();
            }
        }
        Ok(Self {
            types,
            work,
            cores,
            cache: BTreeMap::new(),
        })
    }
    pub fn kind(&self, id: TypeId) -> Kind {
        self.types
            .get(self.cores[id.index()])
            .expect("Grundtyp")
            .kind
    }
    fn pair(&self, a: TypeId, b: TypeId) -> Pair {
        let a = self.cores[a.index()];
        let b = self.cores[b.index()];
        if a <= b { (a, b) } else { (b, a) }
    }
    fn schedule(&self, search: &mut Search, a: TypeId, b: TypeId) -> Result<bool, Error> {
        let pair = self.pair(a, b);
        let at = &self.types.get(a).expect("Typ").location;
        self.work.spend(1, Some(at))?;
        if pair.0 == pair.1 {
            return Ok(true);
        }
        if let Some(&known) = self.cache.get(&pair) {
            return Ok(known);
        }
        if search.seen.contains(&pair) {
            return Ok(true);
        }
        if self.cache.len() + search.seen.len() >= self.work.limits.collection_pairs {
            return Err(error(ErrorKind::Limit(Limit::CollectionPairs), Some(at)));
        }
        search.seen.insert(pair);
        search.pending.push(pair);
        Ok(true)
    }
    pub fn same(&mut self, a: TypeId, b: TypeId) -> Result<bool, Error> {
        let root = self.pair(a, b);
        let mut search = Search::default();
        if !self.schedule(&mut search, a, b)? {
            return Ok(false);
        }
        while let Some((a, b)) = search.pending.pop() {
            let at = &self.types.get(a).expect("Typ").location;
            self.work.spend(1, Some(at))?;
            let ka = self.kind(a);
            let kb = self.kind(b);
            let matched = if self.union(a) || self.union(b) {
                let (an, al) = self.flatten(a)?;
                let (bn, bl) = self.flatten(b)?;
                let mut same = an == bn && al.len() == bl.len();
                if same {
                    for ((ak, av), (bk, bv)) in al.into_iter().zip(bl) {
                        if ak != bk || !self.schedule(&mut search, av, bv)? {
                            same = false;
                            break;
                        }
                    }
                }
                same
            } else if ka != kb {
                false
            } else {
                match ka {
                    Kind::Atom(_) | Kind::Named(_) => true,
                    Kind::Nullable | Kind::Generic(_) | Kind::Record(_) => {
                        let ac = self.types.children(a).expect("Parameter")[0];
                        let bc = self.types.children(b).expect("Parameter")[0];
                        self.schedule(&mut search, ac, bc)?
                    }
                    _ => unreachable!("Alias oder Union vorher behandelt"),
                }
            };
            if !matched {
                self.cache.insert(root, false);
                return Ok(false);
            }
        }
        self.cache
            .extend(search.seen.into_iter().map(|pair| (pair, true)));
        Ok(true)
    }
    fn union(&self, id: TypeId) -> bool {
        match self.kind(id) {
            Kind::Union => true,
            Kind::Nullable => {
                self.kind(
                    self.types
                        .children(self.cores[id.index()])
                        .expect("Nullable")[0],
                ) == Kind::Union
            }
            _ => false,
        }
    }
    fn flatten(&self, root: TypeId) -> Result<(bool, BTreeMap<Key, TypeId>), Error> {
        let mut nullable = false;
        let mut leaves = BTreeMap::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            let id = self.cores[id.index()];
            let ty = self.types.get(id).expect("Typ");
            self.work.spend(1, Some(&ty.location))?;
            let children = self.types.children(id).expect("Kindtypen");
            match ty.kind {
                Kind::Union => pending.extend(children.iter().copied()),
                Kind::Nullable => {
                    nullable = true;
                    pending.push(children[0]);
                }
                kind => {
                    assert!(
                        leaves.insert(key(kind), id).is_none(),
                        "unterscheidbare Unionglieder"
                    );
                }
            }
        }
        Ok((nullable, leaves))
    }
}
fn key(kind: Kind) -> Key {
    match kind {
        Kind::Atom(Atom::Primitive(p)) => Key::Primitive(p as u8),
        Kind::Atom(Atom::String(s)) => Key::String(s as u8),
        Kind::Atom(Atom::Buffer(b)) => Key::Buffer(b as u8),
        Kind::Atom(Atom::Any) => Key::Any,
        Kind::Atom(Atom::Undefined) => Key::Undefined,
        Kind::Atom(Atom::Object) => Key::Object,
        Kind::Atom(Atom::Symbol) => Key::Symbol,
        Kind::Named(id) => Key::Named(id),
        Kind::Generic(g) => Key::Generic(g as u8),
        Kind::Record(s) => Key::Record(s as u8),
        _ => unreachable!("abgeflachter Grundtyp"),
    }
}
