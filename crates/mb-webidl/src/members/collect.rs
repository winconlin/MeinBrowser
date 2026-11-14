use super::*;

pub(super) fn collect<'a>(index: &Index<'a>, work: &mut Work<'_>) -> Result<Members<'a>, Error> {
    let mut model = Members {
        definitions: BTreeMap::new(),
        members: Vec::new(),
        arguments: Vec::new(),
        values: Vec::new(),
    };
    for (id, definition) in index.definitions() {
        work.record(&definition.location)?;
        let first_member = model.members.len();
        let first_value = model.values.len();
        let mut callback = None;
        for &fragment_id in &definition.fragments {
            let fragment = index.fragment(fragment_id).expect("Fragment");
            work.spend(1, Some(&fragment.location))?;
            let source = fragment.location.source;
            let document = index.document(source).expect("Quelle");
            let body = fragment.body;
            if definition.kind == DefinitionKind::Enum {
                enumeration(document, source, body, &mut model, work)?;
            } else if definition.kind == DefinitionKind::Callback {
                callback = Some(signature(
                    document,
                    source,
                    child(document, body, "ArgumentList").expect("Callbackargumente"),
                    child(document, body, "Type"),
                    &mut model,
                    work,
                )?);
            } else if let Some(mut list) = document.children(body).find(|&id| {
                matches!(
                    production(document, id),
                    Some(
                        "InterfaceMembers"
                            | "PartialInterfaceMembers"
                            | "MixinMembers"
                            | "CallbackInterfaceMembers"
                            | "NamespaceMembers"
                            | "DictionaryMembers"
                    )
                )
            }) {
                while document.children(list).next().is_some() {
                    work.spend(1, Some(&fragment.location))?;
                    let list_kind = production(document, list).expect("Memberliste");
                    let (member, attributes) =
                        if let Some(member) = child(document, list, "DictionaryMember") {
                            (
                                child(document, member, "DictionaryMemberRest").expect("Feld"),
                                child(document, member, "ExtendedAttributeList")
                                    .expect("Feldattribute"),
                            )
                        } else {
                            let member = document
                                .children(list)
                                .find(|&id| {
                                    matches!(
                                        production(document, id),
                                        Some(
                                            "InterfaceMember"
                                                | "PartialInterfaceMember"
                                                | "MixinMember"
                                                | "CallbackInterfaceMember"
                                                | "NamespaceMember"
                                        )
                                    )
                                })
                                .expect("Member");
                            (
                                member,
                                child(document, list, "ExtendedAttributeList")
                                    .expect("Memberattribute"),
                            )
                        };
                    member_record(
                        document,
                        source,
                        fragment_id,
                        member,
                        attributes,
                        &mut model,
                        work,
                    )?;
                    list = child(document, list, list_kind).expect("Rest der Liste");
                }
            }
        }
        model.definitions.insert(
            id,
            Definition {
                members: first_member..model.members.len(),
                values: first_value..model.values.len(),
                callback,
            },
        );
    }
    Ok(model)
}

fn member_record<'a>(
    document: &Document<'a>,
    source: usize,
    fragment: FragmentId,
    outer: NodeId,
    attributes: NodeId,
    model: &mut Members<'a>,
    work: &mut Work<'_>,
) -> Result<(), Error> {
    let at = location(document, source, outer);
    work.record(&at)?;
    let mut member = Member {
        fragment,
        location: at,
        kind: Kind::Stringifier,
        identifier: None,
        attributes,
        types: [None; 2],
        signature: None,
        is_static: false,
        readonly: false,
        inherit: false,
        stringifier: false,
        required: false,
        default: None,
        constant: None,
    };
    let mut body = outer;
    loop {
        work.spend(1, Some(&member.location))?;
        member.readonly |= terminal(document, body, "readonly")
            || child(document, body, "OptionalReadOnly")
                .is_some_and(|id| terminal(document, id, "readonly"));
        match production(document, body).expect("Memberproduktion") {
            "Const" => {
                member.kind = Kind::Constant;
                member.types[0] = child(document, body, "ConstType");
                member.identifier = Some(named(
                    document,
                    source,
                    identifier(document, body).expect("Konstantenname"),
                ));
                member.constant = child(document, body, "ConstValue");
                break;
            }
            "AttributeRest" => {
                member.kind = Kind::Attribute;
                member.types[0] = child(document, body, "TypeWithExtendedAttributes");
                member.identifier = Some(named(
                    document,
                    source,
                    token_in_name(
                        document,
                        child(document, body, "AttributeName").expect("Attributname"),
                    ),
                ));
                break;
            }
            "RegularOperation" => {
                let rest = child(document, body, "OperationRest").expect("Operation");
                let name = child(document, rest, "OptionalOperationName").expect("optionaler Name");
                member.identifier = document
                    .children(name)
                    .next()
                    .map(|id| named(document, source, token_in_name(document, id)));
                if !matches!(member.kind, Kind::Operation(_)) {
                    member.kind = Kind::Operation(Special::Regular);
                }
                member.types[0] = child(document, body, "Type");
                member.signature = Some(signature(
                    document,
                    source,
                    child(document, rest, "ArgumentList").expect("Argumentliste"),
                    member.types[0],
                    model,
                    work,
                )?);
                break;
            }
            "Constructor" => {
                member.kind = Kind::Constructor;
                member.signature = Some(signature(
                    document,
                    source,
                    child(document, body, "ArgumentList").expect("Konstruktorargumente"),
                    None,
                    model,
                    work,
                )?);
                break;
            }
            "DictionaryMemberRest" => {
                member.kind = Kind::DictionaryMember;
                member.required = terminal(document, body, "required");
                member.types[0] = child(document, body, "TypeWithExtendedAttributes")
                    .or_else(|| child(document, body, "Type"));
                member.identifier = Some(named(
                    document,
                    source,
                    identifier(document, body).expect("Feldname"),
                ));
                member.default = default(document, body);
                break;
            }
            "Iterable" | "AsyncIterable" | "MaplikeRest" | "SetlikeRest" => {
                member.kind = match production(document, body).expect("Collection") {
                    "Iterable" => Kind::Iterable,
                    "AsyncIterable" => Kind::AsyncIterable,
                    "MaplikeRest" => Kind::Maplike,
                    _ => Kind::Setlike,
                };
                for (at, ty) in document
                    .children(body)
                    .filter(|&id| production(document, id) == Some("TypeWithExtendedAttributes"))
                    .enumerate()
                {
                    member.types[at] = Some(ty);
                }
                if let Some(optional) = child(document, body, "OptionalType") {
                    member.types[1] = child(document, optional, "TypeWithExtendedAttributes");
                }
                if let Some(args) = child(document, body, "OptionalArgumentList")
                    .and_then(|id| child(document, id, "ArgumentList"))
                {
                    member.signature = Some(signature(document, source, args, None, model, work)?);
                }
                break;
            }
            "StringifierRest" if terminal(document, body, ";") => {
                member.kind = Kind::Stringifier;
                break;
            }
            "Stringifier" => member.stringifier = true,
            "StaticMember" => member.is_static = true,
            "InheritAttribute" => member.inherit = true,
            "SpecialOperation" => {
                let special = child(document, body, "Special").expect("Spezialoperation");
                member.kind = Kind::Operation(if terminal(document, special, "getter") {
                    Special::Getter
                } else if terminal(document, special, "setter") {
                    Special::Setter
                } else {
                    Special::Deleter
                });
            }
            _ => {}
        }
        body = document
            .children(body)
            .find(|&id| {
                production(document, id)
                    .is_some_and(|name| name != "OptionalReadOnly" && name != "Special")
            })
            .expect("Memberhülle");
    }
    check::member_name(&member)?;
    model.members.push(member);
    Ok(())
}

fn signature<'a>(
    document: &Document<'a>,
    source: usize,
    mut list: NodeId,
    result: Option<NodeId>,
    model: &mut Members<'a>,
    work: &mut Work<'_>,
) -> Result<Signature, Error> {
    let first = model.arguments.len();
    let mut seen = BTreeMap::new();
    while let Some(arg) = child(document, list, "Argument") {
        let rest = child(document, arg, "ArgumentRest").expect("Argument");
        let identifier = named(
            document,
            source,
            token_in_name(
                document,
                child(document, rest, "ArgumentName").expect("Argumentname"),
            ),
        );
        work.record(&identifier.location)?;
        work.spend(identifier.name.len() as u64, Some(&identifier.location))?;
        if let Some(previous) = seen.insert(identifier.name, identifier.location.clone()) {
            return Err(duplicate(
                ErrorKind::DuplicateArgument,
                &identifier.location,
                &previous,
            ));
        }
        let variadic =
            child(document, rest, "Ellipsis").is_some_and(|id| terminal(document, id, "..."));
        let tail = child(document, list, "Arguments").expect("weitere Argumente");
        if variadic && child(document, tail, "Argument").is_some() {
            return Err(error(
                ErrorKind::VariadicNotLast,
                Some(&identifier.location),
            ));
        }
        let argument = Argument {
            identifier,
            ty: child(document, rest, "TypeWithExtendedAttributes")
                .or_else(|| child(document, rest, "Type"))
                .expect("Argumenttyp"),
            attributes: child(document, arg, "ExtendedAttributeList").expect("Argumentattribute"),
            optional: variadic || terminal(document, rest, "optional"),
            variadic,
            default: default(document, rest),
        };
        model.arguments.push(argument);
        list = tail;
    }
    Ok(Signature {
        result,
        arguments: first..model.arguments.len(),
    })
}

fn default(document: &Document<'_>, body: NodeId) -> Option<NodeId> {
    child(document, body, "Default").and_then(|id| child(document, id, "DefaultValue"))
}

fn enumeration<'a>(
    document: &Document<'a>,
    source: usize,
    body: NodeId,
    model: &mut Members<'a>,
    work: &mut Work<'_>,
) -> Result<(), Error> {
    let mut list = child(document, body, "EnumValueList").expect("Enumwerte");
    let mut seen = BTreeMap::new();
    loop {
        work.spend(1, None)?;
        if let Some(id) = document.children(list).find(|&id| {
            document.node(id).expect("Enumtoken").kind == NodeKind::Token(lexer::Kind::String)
        }) {
            let text = document.text(id).expect("Enumtext");
            let value = &text[1..text.len() - 1];
            let at = location(document, source, id);
            work.record(&at)?;
            work.spend(value.len() as u64, Some(&at))?;
            if let Some(previous) = seen.insert(value, at.clone()) {
                return Err(duplicate(ErrorKind::DuplicateEnumValue, &at, &previous));
            }
            model.values.push(EnumValue {
                value,
                location: at,
            });
        }
        let next = document.children(list).find(|&id| {
            matches!(
                production(document, id),
                Some("EnumValueListComma" | "EnumValueListString")
            )
        });
        if let Some(next) = next {
            list = next;
        } else {
            break;
        }
    }
    Ok(())
}
