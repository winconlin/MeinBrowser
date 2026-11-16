use super::*;

pub(super) fn check(
    index: &Index<'_>,
    members: &Members<'_>,
    types: &mut Types,
    work: &Work<'_>,
) -> Result<(), Error> {
    work.spend(types.len() as u64, None)?;
    let mut facts = vec![None; types.len()];
    let mut colors = vec![0u8; types.len()];
    for root in 0..types.len() {
        work.spend(1, None)?;
        if colors[root] != 0 {
            continue;
        }
        let root = TypeId(NonZeroU32::new(root as u32 + 1).expect("Wurzeltyp"));
        let mut pending = vec![(root, 0usize)];
        colors[root.index()] = 1;
        while let Some((id, next)) = pending.last_mut() {
            let ty = types.get(*id).expect("lokaler Typ");
            work.spend(1, Some(&ty.location))?;
            let dependency = if let Kind::Named(target) = ty.kind {
                types.typedef(target).filter(|_| *next == 0)
            } else {
                types.children(*id).expect("Kindtypen").get(*next).copied()
            };
            if let Some(dependency) = dependency {
                *next += 1;
                match colors[dependency.index()] {
                    0 => {
                        colors[dependency.index()] = 1;
                        pending.push((dependency, 0));
                    }
                    1 => return Err(error(ErrorKind::TypedefCycle, Some(&ty.location))),
                    _ => {}
                }
            } else {
                facts[id.index()] = Some(summarize(index, types, *id, &facts, work)?);
                colors[id.index()] = 2;
                pending.pop();
            }
        }
    }
    types.facts = facts
        .into_iter()
        .map(|f| f.expect("jeder Typ zusammengefasst"))
        .collect();
    for (id, definition) in index.definitions() {
        work.spend(1, Some(&definition.location))?;
        for member in members.members(id).expect("zugehörige Memberbeschreibung") {
            work.spend(1, Some(&member.location))?;
            if let Some(node) = member.types[0] {
                let ty = types.at(member.location.source, node).expect("Membertyp");
                let f = types.facts[ty.index()];
                let invalid = match member.kind {
                    crate::members::Kind::Constant if f.category != Category::Primitive => {
                        Some(ErrorKind::ConstantType)
                    }
                    crate::members::Kind::Attribute if f.attribute_forbidden => {
                        Some(ErrorKind::AttributeType)
                    }
                    crate::members::Kind::Attribute
                        if f.category == Category::Promise && !member.readonly =>
                    {
                        Some(ErrorKind::AttributePromiseReadonly)
                    }
                    crate::members::Kind::DictionaryMember
                        if f.category == Category::Nullable && f.dictionary =>
                    {
                        Some(ErrorKind::NullableDictionary)
                    }
                    _ => None,
                };
                if let Some(kind) = invalid {
                    return Err(error(kind, Some(&types.get(ty).expect("Typ").location)));
                }
            }
            if let Some(signature) = &member.signature {
                arguments(members, types, signature, work)?;
            }
        }
        if let Some(signature) = members.callback(id) {
            arguments(members, types, signature, work)?;
        }
    }
    super::dictionary::check(index, members, types, work)
}

fn arguments(
    members: &Members<'_>,
    types: &Types,
    signature: &crate::members::Signature,
    work: &Work<'_>,
) -> Result<(), Error> {
    for argument in members.arguments(signature).expect("Argumente") {
        work.spend(1, Some(&argument.identifier.location))?;
        let ty = types
            .at(argument.identifier.location.source, argument.ty)
            .expect("Argumenttyp");
        let f = types.facts[ty.index()];
        if f.category == Category::Nullable && f.dictionary {
            return Err(error(
                ErrorKind::NullableDictionary,
                Some(&types.get(ty).expect("Typ").location),
            ));
        }
    }
    Ok(())
}

fn summarize(
    index: &Index<'_>,
    types: &Types,
    id: TypeId,
    facts: &[Option<Facts>],
    work: &Work<'_>,
) -> Result<Facts, Error> {
    let ty = types.get(id).expect("Typ");
    let children = types.children(id).expect("Kinder");
    let inner = || facts[children[0].index()].expect("Kinder bereits bekannt");
    let result = match ty.kind {
        Kind::Atom(atom) => Facts::new(match atom {
            Atom::Any => Category::Any,
            Atom::Primitive(_) => Category::Primitive,
            Atom::String(_) => Category::String,
            Atom::Buffer(_) => Category::Buffer,
            Atom::Object => Category::Object,
            Atom::Symbol => Category::Symbol,
            Atom::Undefined => Category::Undefined,
        }),
        Kind::Named(target) => {
            if let Some(ty) = types.typedef(target) {
                facts[ty.index()].expect("Typedef bereits bekannt")
            } else {
                Facts::new(
                    match index.definition(target).expect("Typdefinition").kind {
                        DefinitionKind::Interface => Category::Interface,
                        DefinitionKind::CallbackInterface => Category::CallbackInterface,
                        DefinitionKind::Dictionary => Category::Dictionary,
                        DefinitionKind::Enum => Category::Enum,
                        DefinitionKind::Callback => Category::Callback,
                        _ => unreachable!("benannter Typ"),
                    },
                )
            }
        }
        Kind::Annotated { .. } => inner(),
        Kind::Nullable => {
            let mut f = inner();
            if matches!(
                f.category,
                Category::Any | Category::Promise | Category::ObservableArray | Category::Nullable
            ) || (f.category == Category::Union && (f.nullable > 0 || f.dictionary))
            {
                return Err(error(ErrorKind::NullableType, Some(&ty.location)));
            }
            f.category = Category::Nullable;
            f.nullable = f.nullable.saturating_add(1).min(2);
            f
        }
        Kind::Generic(kind) => Facts::new(match kind {
            Generic::Sequence => Category::Sequence,
            Generic::AsyncSequence => Category::AsyncSequence,
            Generic::FrozenArray => Category::FrozenArray,
            Generic::ObservableArray => Category::ObservableArray,
            Generic::Promise => Category::Promise,
        }),
        Kind::Record(_) => Facts::new(Category::Record),
        Kind::Union => {
            let mut result = Facts::new(Category::Union);
            for child in children {
                work.spend(1, Some(&ty.location))?;
                let f = facts[child.index()].expect("Unionkind");
                result.nullable = result.nullable.saturating_add(f.nullable).min(2);
                result.dictionary |= f.dictionary;
                result.attribute_forbidden |= f.union_attribute_forbidden;
                result.union_attribute_forbidden |= f.union_attribute_forbidden;
                result.any |= f.any;
                result.promise |= f.promise;
            }
            if result.any
                || result.promise
                || result.nullable > 1
                || (result.nullable == 1 && result.dictionary)
            {
                return Err(error(ErrorKind::UnionType, Some(&ty.location)));
            }
            result
        }
    };
    Ok(result)
}
