use super::*;

pub(super) const BOOLEAN: u8 = 1;
pub(super) const BIGINT: u8 = 2;
pub(super) const NULL: u8 = 4;
pub(super) const UNDEFINED: u8 = 8;
pub(super) const ANY: u8 = 16;
pub(super) const SEQUENCE: u8 = 32;

#[derive(Clone, Copy)]
pub(super) enum StringValue {
    Builtin(StringType),
    Enum(DefinitionId),
}
#[derive(Clone, Copy, Default)]
pub(super) struct Shape {
    pub flags: u8,
    pub number: Option<Primitive>,
    pub string: Option<StringValue>,
    pub dictionary: Option<DefinitionId>,
}
impl Shape {
    fn merge(&mut self, other: Self) {
        self.flags |= other.flags;
        self.number = self.number.or(other.number);
        self.string = self.string.or(other.string);
        self.dictionary = self.dictionary.or(other.dictionary);
    }
}

/// Nur transparente Kanten verfolgen; ein Containerparameter gehört nicht
/// zum Wert des Containers. Gültige Unions haben höchstens ein Glied pro
/// hier relevantem Kategorie-Slot. Die Unionprüfung läuft vorher.
pub(super) fn build(
    index: &Index<'_>,
    types: &Types,
    work: &Work<'_>,
) -> Result<Vec<Shape>, Error> {
    work.spend(types.len() as u64, None)?;
    let mut result = vec![Shape::default(); types.len()];
    let mut done = vec![false; types.len()];
    for (root, _) in types.iter() {
        if done[root.index()] {
            continue;
        }
        let mut pending = vec![(root, 0usize)];
        while let Some((id, next)) = pending.last_mut() {
            let ty = types.get(*id).expect("Typ");
            work.spend(1, Some(&ty.location))?;
            let children = types.children(*id).expect("Kindtypen");
            let dependency = match ty.kind {
                Kind::Named(target) => types.typedef(target).filter(|_| *next == 0),
                Kind::Annotated { .. } | Kind::Nullable | Kind::Union => {
                    children.get(*next).copied()
                }
                _ => None,
            };
            if let Some(child) = dependency {
                *next += 1;
                if !done[child.index()] {
                    pending.push((child, 0));
                }
            } else {
                let mut shape = Shape::default();
                match ty.kind {
                    Kind::Atom(Atom::Primitive(Primitive::Boolean)) => shape.flags = BOOLEAN,
                    Kind::Atom(Atom::Primitive(Primitive::BigInt)) => shape.flags = BIGINT,
                    Kind::Atom(Atom::Primitive(p)) => shape.number = Some(p),
                    Kind::Atom(Atom::String(s)) => shape.string = Some(StringValue::Builtin(s)),
                    Kind::Atom(Atom::Any) => shape.flags = ANY,
                    Kind::Atom(Atom::Undefined) => shape.flags = UNDEFINED,
                    Kind::Named(target) => {
                        if let Some(inner) = types.typedef(target) {
                            shape = result[inner.index()];
                        } else {
                            match index.definition(target).expect("Definition").kind {
                                DefinitionKind::Enum => {
                                    shape.string = Some(StringValue::Enum(target))
                                }
                                DefinitionKind::Dictionary => shape.dictionary = Some(target),
                                _ => {}
                            }
                        }
                    }
                    Kind::Annotated { .. } | Kind::Nullable => {
                        shape = result[children[0].index()];
                        if ty.kind == Kind::Nullable {
                            shape.flags |= NULL;
                        }
                    }
                    Kind::Union => {
                        for child in children {
                            work.spend(1, Some(&ty.location))?;
                            shape.merge(result[child.index()]);
                        }
                    }
                    Kind::Generic(Generic::Sequence) => shape.flags = SEQUENCE,
                    _ => {}
                }
                result[id.index()] = shape;
                done[id.index()] = true;
                pending.pop();
            }
        }
    }
    Ok(result)
}
