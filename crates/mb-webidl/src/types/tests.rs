use super::*;
use crate::{index, members};

#[test]
fn canonical_primitives_ignore_trivia_and_preserve_unsigned_unrestricted_width() {
    let source = "typedef unsigned /* unsigned */ long // next\n long Wide; typedef unrestricted /*float*/ double Real; interface X { const Wide answer = 1; attribute Real value; };";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let types = build(&index, &members).unwrap();
    assert_eq!(
        types
            .get(types.typedef(index.find("Wide").unwrap()).unwrap())
            .unwrap()
            .kind,
        Kind::Atom(Atom::Primitive(Primitive::UnsignedLongLong))
    );
    assert_eq!(
        types
            .get(types.typedef(index.find("Real").unwrap()).unwrap())
            .unwrap()
            .kind,
        Kind::Atom(Atom::Primitive(Primitive::UnrestrictedDouble))
    );
}

fn model(source: &str) -> (Index<'_>, Members<'_>, Types) {
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let types = build(&index, &members).unwrap();
    (index, members, types)
}
fn rejected(source: &str, expected: ErrorKind) {
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let e = build(&index, &members).unwrap_err();
    assert_eq!(e.kind, expected, "{source}");
    let at = e.location.unwrap();
    assert!(source.get(at.span).is_some());
    assert!(
        index
            .document(at.source)
            .unwrap()
            .node(at.node.unwrap())
            .is_some()
    );
}
fn typedef(types: &Types, index: &Index<'_>, name: &str) -> TypeId {
    types.typedef(index.find(name).unwrap()).unwrap()
}

#[test]
fn all_primitive_string_and_buffer_types_have_exact_canonical_kinds() {
    let primitives = [
        ("boolean", Primitive::Boolean),
        ("byte", Primitive::Byte),
        ("octet", Primitive::Octet),
        ("short", Primitive::Short),
        ("unsigned short", Primitive::UnsignedShort),
        ("long", Primitive::Long),
        ("unsigned long", Primitive::UnsignedLong),
        ("long long", Primitive::LongLong),
        ("unsigned long long", Primitive::UnsignedLongLong),
        ("float", Primitive::Float),
        ("unrestricted float", Primitive::UnrestrictedFloat),
        ("double", Primitive::Double),
        ("unrestricted double", Primitive::UnrestrictedDouble),
        ("bigint", Primitive::BigInt),
    ];
    for (text, expected) in primitives {
        let s = format!("typedef {text} T;");
        let (i, _, t) = model(&s);
        assert_eq!(
            t.get(typedef(&t, &i, "T")).unwrap().kind,
            Kind::Atom(Atom::Primitive(expected))
        );
    }
    for (text, expected) in [
        ("ByteString", StringType::ByteString),
        ("DOMString", StringType::DOMString),
        ("USVString", StringType::USVString),
    ] {
        let s = format!("typedef {text} T;");
        let (i, _, t) = model(&s);
        assert_eq!(
            t.get(typedef(&t, &i, "T")).unwrap().kind,
            Kind::Atom(Atom::String(expected))
        );
    }
    for text in [
        "ArrayBuffer",
        "SharedArrayBuffer",
        "DataView",
        "Int8Array",
        "Int16Array",
        "Int32Array",
        "Uint8Array",
        "Uint16Array",
        "Uint32Array",
        "Uint8ClampedArray",
        "BigInt64Array",
        "BigUint64Array",
        "Float16Array",
        "Float32Array",
        "Float64Array",
    ] {
        let s = format!("typedef {text} T;");
        let (i, _, t) = model(&s);
        assert_eq!(t.category(typedef(&t, &i, "T")), Some(Category::Buffer));
    }
}

#[test]
fn generics_records_nullable_unions_and_annotations_retain_their_structure() {
    let source = "interface Item {}; typedef [Outer=(Opaque,Name)] (sequence<[Element] Item?> or record<USVString, FrozenArray<long>>) Choice; typedef Promise<Choice?> Result; typedef ObservableArray<long> Values; typedef async_sequence<Item> Stream;";
    let (index, _, types) = model(source);
    let choice = typedef(&types, &index, "Choice");
    assert!(matches!(
        types.get(choice).unwrap().kind,
        Kind::Annotated { .. }
    ));
    let union = types.children(choice).unwrap()[0];
    assert_eq!(types.get(union).unwrap().kind, Kind::Union);
    let parts = types.children(union).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(
        types.get(parts[0]).unwrap().kind,
        Kind::Generic(Generic::Sequence)
    );
    assert_eq!(
        types.get(parts[1]).unwrap().kind,
        Kind::Record(StringType::USVString)
    );
    let element = types.children(parts[0]).unwrap()[0];
    assert!(matches!(
        types.get(element).unwrap().kind,
        Kind::Annotated { .. }
    ));
    let nullable = types.children(element).unwrap()[0];
    assert_eq!(types.get(nullable).unwrap().kind, Kind::Nullable);
    assert_eq!(
        types.category(typedef(&types, &index, "Result")),
        Some(Category::Promise)
    );
    assert_eq!(
        types.category(typedef(&types, &index, "Values")),
        Some(Category::ObservableArray)
    );
    assert_eq!(
        types.category(typedef(&types, &index, "Stream")),
        Some(Category::AsyncSequence)
    );
    for (id, ty) in types.iter() {
        for &child in types.children(id).unwrap() {
            assert!(child < id);
            assert!(types.get(child).is_some());
        }
        let doc = index.document(ty.location.source).unwrap();
        assert!(doc.source().get(ty.location.span.clone()).is_some());
        if let Kind::Annotated { attributes } = ty.kind {
            assert_eq!(
                doc.node(attributes).unwrap().kind,
                NodeKind::Production("ExtendedAttributeList")
            );
        }
    }
}

#[test]
fn named_definitions_and_typedefs_resolve_forward_across_sources_without_copying() {
    let sources = [
        "interface X { readonly attribute Count count; undefined run(Item value); };",
        "typedef unsigned long Count; interface Item {}; dictionary D {}; enum E {\"a\"}; callback F = undefined (); callback interface C {}; typedef record<DOMString, Count> R;",
    ];
    let index = index::build(&sources).unwrap();
    let members = members::build(&index).unwrap();
    let types = build(&index, &members).unwrap();
    let member = &members.members(index.find("X").unwrap()).unwrap()[0];
    let ty = types.at(0, member.types[0].unwrap()).unwrap();
    assert_eq!(
        types.get(ty).unwrap().kind,
        Kind::Named(index.find("Count").unwrap())
    );
    assert_eq!(types.category(ty), Some(Category::Primitive));
    assert_ne!(ty, typedef(&types, &index, "Count"));
    assert_eq!(types.children(ty).unwrap(), []);
}

#[test]
fn every_member_argument_callback_and_typedef_type_node_is_mapped() {
    let source = "interface X { const long n = 1; readonly attribute Promise<long> ready; constructor(optional long n = 0); undefined run(long n, DOMString... rest); iterable<DOMString,long>; async_iterable<long>(optional long n); readonly maplike<DOMString,long>; setlike<long>; }; dictionary D { required DOMString n; long m = 0; }; callback F = undefined (long n); typedef record<DOMString, long> R;";
    let (index, members, types) = model(source);
    for (id, _) in index.definitions() {
        for member in members.members(id).unwrap() {
            for node in member.types.iter().flatten() {
                assert!(types.at(member.location.source, *node).is_some());
            }
            if let Some(signature) = &member.signature {
                for arg in members.arguments(signature).unwrap() {
                    assert!(types.at(arg.identifier.location.source, arg.ty).is_some());
                }
            }
        }
        if let Some(callback) = members.callback(id) {
            assert!(types.at(0, callback.result.unwrap()).is_some());
            for arg in members.arguments(callback).unwrap() {
                assert!(types.at(0, arg.ty).is_some());
            }
        }
    }
}

#[test]
fn escaped_names_stay_distinct_from_builtin_tokens() {
    let source = "interface _DOMString {}; typedef _DOMString Named; typedef DOMString Builtin;";
    let (index, _, types) = model(source);
    assert_eq!(
        types.get(typedef(&types, &index, "Named")).unwrap().kind,
        Kind::Named(index.find("DOMString").unwrap())
    );
    assert_eq!(
        types.get(typedef(&types, &index, "Builtin")).unwrap().kind,
        Kind::Atom(Atom::String(StringType::DOMString))
    );
}

#[test]
fn nullable_rules_apply_after_typedefs_and_annotations() {
    for source in [
        "typedef any A; typedef A? Bad;",
        "typedef Promise<long> A; typedef A? Bad;",
        "typedef ObservableArray<long> A; typedef A? Bad;",
        "typedef long? A; typedef A? Bad;",
        "typedef (long? or DOMString) A; typedef A? Bad;",
        "dictionary D {}; typedef (D or DOMString)? Bad;",
    ] {
        rejected(source, ErrorKind::NullableType);
    }
    model(
        "dictionary D {}; typedef D? Nullable; typedef [Tag] long? N; typedef Promise<long?> P; typedef (long or DOMString)? U;",
    );
}

#[test]
fn union_rules_count_nullable_members_through_nested_unions_and_typedefs() {
    for source in [
        "typedef any A; typedef (A or long) Bad;",
        "typedef Promise<long> A; typedef (A or long) Bad;",
        "typedef (long? or DOMString?) Bad;",
        "typedef long? A; typedef (A or DOMString?) Bad;",
        "dictionary D {}; typedef (D or long?) Bad;",
        "typedef (long? or DOMString) A; typedef (A or boolean?) Bad;",
    ] {
        rejected(source, ErrorKind::UnionType);
    }
    model("typedef (long? or DOMString) A; typedef (sequence<A> or boolean?) B;");
}

#[test]
fn attribute_types_follow_aliases_and_flattened_unions_without_descending_into_containers() {
    for source in [
        "interface X { attribute sequence<long> value; };",
        "interface X { attribute async_sequence<long> value; };",
        "dictionary D {}; interface X { attribute D value; };",
        "typedef record<DOMString,long> R; interface X { attribute R value; };",
        "typedef sequence<long> S; interface X { attribute S? value; };",
        "typedef (sequence<long> or DOMString) U; interface X { attribute U value; };",
    ] {
        rejected(source, ErrorKind::AttributeType);
    }
    model(
        "dictionary D {}; interface X { attribute FrozenArray<D> value; readonly attribute Promise<sequence<D>> ready; };",
    );
    rejected(
        "typedef Promise<long> P; interface X { attribute P ready; };",
        ErrorKind::AttributePromiseReadonly,
    );
}

#[test]
fn constants_need_primitive_types_even_through_typedefs() {
    for source in [
        "typedef DOMString S; interface X { const S n = 1; };",
        "enum E {\"x\"}; interface X { const E n = 1; };",
        "typedef long? N; interface X { const N n = 1; };",
    ] {
        rejected(source, ErrorKind::ConstantType);
    }
    model("typedef unrestricted double F; interface X { const F n = NaN; };");
}

#[test]
fn nullable_dictionaries_are_rejected_only_in_argument_and_dictionary_field_positions() {
    for source in [
        "dictionary D {}; callback F = undefined (D? value);",
        "dictionary D {}; dictionary E { D? value; };",
        "dictionary D {}; typedef D? N; interface X { constructor(N value); };",
        "dictionary D {}; interface X { async_iterable<long>(D? value); };",
    ] {
        rejected(source, ErrorKind::NullableDictionary);
    }
    model("dictionary D {}; callback F = D? (); typedef D? N;");
}

#[test]
fn graph_sharing_does_not_expand_typedefs_exponentially() {
    let mut source = String::from("typedef long T0; ");
    for n in 1..35 {
        source.push_str(&format!("typedef (T{} or T{}) T{n}; ", n - 1, n - 1));
    }
    // Gleiche Unionglieder sind noch keine vollständige Unterscheidbarkeitsprüfung.
    let index = index::build(&[&source]).unwrap();
    let members = members::build(&index).unwrap();
    let types = build_within(
        &index,
        &members,
        Limits {
            source_nodes: 10000,
            types: 200,
            edges: 200,
            ..Limits::default()
        },
        &Budget::new(15000),
    )
    .unwrap();
    assert!(types.len() < 110);
    assert!(types.edge_count() < 80);
}

#[test]
fn node_type_edge_limits_and_exact_work_boundaries_are_host_failures() {
    let source = "typedef sequence<long> T;";
    let (index, members, types) = model(source);
    let nodes = index.document(0).unwrap().len();
    for (limits, expected) in [
        (
            Limits {
                source_nodes: nodes - 1,
                ..Limits::default()
            },
            Limit::SourceNodes,
        ),
        (
            Limits {
                types: types.len() - 1,
                ..Limits::default()
            },
            Limit::Types,
        ),
        (
            Limits {
                edges: 0,
                ..Limits::default()
            },
            Limit::Edges,
        ),
    ] {
        assert_eq!(
            build_within(&index, &members, limits, &Budget::unlimited())
                .unwrap_err()
                .kind,
            ErrorKind::Limit(expected)
        );
    }
    let limits = Limits {
        source_nodes: nodes,
        types: types.len(),
        edges: types.edge_count(),
        ..Limits::default()
    };
    let budget = Budget::unlimited();
    build_within(&index, &members, limits, &budget).unwrap();
    let cost = u64::MAX - budget.remaining();
    for n in [0, 1, cost / 2, cost - 1] {
        let b = Budget::new(n);
        assert_eq!(
            build_within(&index, &members, limits, &b).unwrap_err().kind,
            ErrorKind::Exhausted(Exhausted::Steps)
        );
        assert_eq!(b.exhausted(), Some(Exhausted::Steps));
    }
    let exact = Budget::new(cost);
    build_within(&index, &members, limits, &exact).unwrap();
    assert_eq!(exact.remaining(), 0);
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(
        build_within(&index, &members, limits, &Budget::with_cancel(cost, cancel))
            .unwrap_err()
            .kind,
        ErrorKind::Exhausted(Exhausted::Cancelled)
    );
}

#[test]
fn deeply_nested_types_and_their_flat_drop_fit_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let source = format!(
                "typedef {}long{} T;",
                "sequence<".repeat(2000),
                ">".repeat(2000)
            );
            let (index, members, types) = model(&source);
            assert_eq!(types.len(), 2001);
            drop(types);
            drop(members);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn empty_sources_have_empty_types_and_compact_optional_ids() {
    let (_, _, types) = model("");
    assert!(types.is_empty());
    assert_eq!(std::mem::size_of::<Option<TypeId>>(), 4);
}

#[test]
fn index_members_and_types_share_the_same_remaining_quote() {
    let source = "typedef unsigned long Count; interface X { readonly attribute Count value; };";
    let unlimited = Budget::unlimited();
    let index = index::build_within(&[source], index::Limits::default(), &unlimited).unwrap();
    let members = members::build_within(&index, members::Limits::default(), &unlimited).unwrap();
    build_within(&index, &members, Limits::default(), &unlimited).unwrap();
    let total = u64::MAX - unlimited.remaining();
    for (amount, ok) in [(total, true), (total - 1, false)] {
        let budget = Budget::new(amount);
        let index = index::build_within(&[source], index::Limits::default(), &budget).unwrap();
        let members = members::build_within(&index, members::Limits::default(), &budget).unwrap();
        let result = build_within(&index, &members, Limits::default(), &budget);
        if ok {
            assert!(result.is_ok());
            assert_eq!(budget.remaining(), 0);
        } else {
            assert_eq!(
                result.unwrap_err().kind,
                ErrorKind::Exhausted(Exhausted::Steps)
            );
        }
    }
}
