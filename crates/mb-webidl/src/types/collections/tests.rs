use super::*;
use crate::{index, members};
fn check(sources: &[&str]) -> Result<Types, Error> {
    let index = index::build(sources).expect("syntax and names");
    let members = members::build(&index).expect("member names");
    super::super::build(&index, &members)
}
#[test]
fn value_iterator_uses_the_indexed_getter_type() {
    check(&["interface I { readonly attribute unsigned long length; getter long get(unsigned long index); iterable<long>; };"]).unwrap();
}

const DECLS: [&str; 6] = [
    "iterable<DOMString,long>",
    "async_iterable<long>",
    "readonly maplike<DOMString,long>",
    "maplike<DOMString,long>",
    "readonly setlike<long>",
    "setlike<long>",
];
fn invalid(source: &str, expected: ErrorKind) {
    let e = check(&[source]).unwrap_err();
    assert_eq!(e.kind, expected, "{source}");
    assert!(source.get(e.location.unwrap().span).is_some());
}
fn value_source(left: &str, right: &str) -> String {
    format!(
        "interface A {{}}; interface B : A {{}}; enum E {{\"x\"}}; enum F {{\"x\"}}; dictionary D {{}}; callback C = undefined (); callback C2 = undefined (); interface _ArrayBuffer {{}}; typedef [Opaque] unsigned long Count; typedef [Tag] long Signed; interface I {{ readonly attribute unsigned long length; getter {left} item(unsigned long index); iterable<{right}>; }};"
    )
}

#[test]
fn all_local_collection_pairs_are_exclusive_even_across_partials() {
    for first in DECLS {
        for second in DECLS {
            invalid(
                &format!("interface I {{ {first}; {second}; }};"),
                ErrorKind::CollectionConflict,
            );
            invalid(
                &format!("partial interface I {{ {first}; }}; interface I {{ {second}; }};"),
                ErrorKind::CollectionConflict,
            );
        }
    }
}

#[test]
fn all_inherited_collection_pairs_are_exclusive_and_siblings_do_not_leak() {
    for first in DECLS {
        for second in DECLS {
            invalid(
                &format!("interface Child : Base {{ {second}; }}; interface Base {{ {first}; }};"),
                ErrorKind::CollectionConflict,
            );
        }
    }
    check(&["interface Base {}; interface A : Base { iterable<DOMString,long>; }; interface B : Base { maplike<DOMString,long>; }; interface C : Base { async_iterable<long>; };"]).unwrap();
    invalid(
        "interface Base { readonly setlike<long>; }; interface Middle : Base {}; interface Child : Middle { iterable<DOMString,long>; };",
        ErrorKind::CollectionConflict,
    );
}

#[test]
fn all_396_reserved_name_kind_combinations_follow_the_normative_lists() {
    let names = [
        "entries", "forEach", "keys", "values", "get", "has", "size", "clear", "delete", "set",
        "add",
    ];
    let common: [&[&str]; 6] = [
        &["entries", "forEach", "keys", "values"],
        &["entries", "keys", "values"],
        &["entries", "forEach", "keys", "values", "get", "has", "size"],
        &["entries", "forEach", "keys", "values", "get", "has", "size"],
        &["entries", "forEach", "keys", "values", "has", "size"],
        &["entries", "forEach", "keys", "values", "has", "size"],
    ];
    for (row, decl) in DECLS.into_iter().enumerate() {
        for name in names {
            for kind in 0..6 {
                let member = match kind {
                    0 => format!("attribute long {name}"),
                    1 => format!("const long {name}=0"),
                    2 => format!("undefined {name}()"),
                    3 => format!("static undefined {name}()"),
                    4 => format!("static readonly attribute long {name}"),
                    _ => format!("getter long {name}(DOMString key)"),
                };
                let attr = matches!(kind, 0 | 1 | 4);
                let mutation = attr
                    && ((row == 3 && ["clear", "delete", "set"].contains(&name))
                        || (row == 5 && ["add", "clear", "delete"].contains(&name)));
                let forbidden = kind != 3 && (common[row].contains(&name) || mutation);
                let source = format!("interface I {{ {decl}; {member}; }};");
                let result = check(&[&source]);
                assert_eq!(result.is_err(), forbidden, "{source}: {result:?}");
                if let Err(e) = result {
                    assert_eq!(e.kind, ErrorKind::CollectionMemberName);
                }
            }
        }
    }
}

#[test]
fn readonly_and_mutation_method_overrides_are_not_over_restricted() {
    check(&["interface Map { maplike<DOMString,long>; undefined clear(); boolean delete(DOMString key); Map set(DOMString key,long value); }; interface Set { setlike<long>; Set add(long value); undefined clear(); boolean delete(long value); };"]).unwrap();
    check(&["interface Map { readonly maplike<DOMString,long>; attribute long set; attribute long clear; attribute long delete; }; interface Set { readonly setlike<long>; attribute long add; };"]).unwrap();
    check(&[
        "interface I { async_iterable<long>; undefined forEach(); static undefined values(); };",
    ])
    .unwrap();
}

#[test]
fn parent_and_mixin_names_are_checked_at_their_actual_declarations() {
    let sources = [
        "interface Base { readonly attribute long keys; }; interface mixin M { readonly attribute long clear; };",
        "interface I : Base { maplike<DOMString,long>; };",
    ];
    let e = check(&sources).unwrap_err();
    assert_eq!(e.kind, ErrorKind::CollectionMemberName);
    let at = e.location.unwrap();
    assert_eq!(at.source, 0);
    assert_eq!(&sources[0][at.span], "keys");
    invalid(
        "interface Base { attribute long clear; }; interface I : Base { maplike<DOMString,long>; undefined clear(); };",
        ErrorKind::CollectionMemberName,
    );
    invalid(
        "interface I { setlike<long>; }; interface mixin M { readonly attribute long add; }; I includes M;",
        ErrorKind::CollectionMemberName,
    );
    invalid(
        "interface I { iterable<DOMString,long>; undefined _keys(); };",
        ErrorKind::CollectionMemberName,
    );
    check(&["interface mixin M { readonly attribute long keys; }; interface A {}; A includes M; interface B { maplike<DOMString,long>; };"]).unwrap();
}

#[test]
fn restrictions_apply_to_the_declaring_interface_and_its_ancestors() {
    // The cited rules mention the declaration owner and its ancestors, not
    // every descendant that inherits a generated method.
    check(&["interface Base { iterable<DOMString,long>; }; interface Child : Base { undefined keys(); };"]).unwrap();
    check(&["interface Base { maplike<DOMString,long>; }; interface Child : Base { readonly attribute long get; };"]).unwrap();
}

#[test]
fn indexed_getters_distinguish_sync_value_pair_map_and_set_collections() {
    for decl in [
        "iterable<DOMString,long>",
        "readonly maplike<DOMString,long>",
        "maplike<DOMString,long>",
        "readonly setlike<long>",
        "setlike<long>",
    ] {
        invalid(
            &format!(
                "interface Base {{ readonly attribute unsigned long length; getter long item(unsigned long index); }}; interface I : Base {{ {decl}; }};"
            ),
            ErrorKind::CollectionIndexedGetter,
        );
    }
    invalid(
        "interface I { getter long named(DOMString key); iterable<long>; };",
        ErrorKind::ValueIteratorGetter,
    );
    check(&["interface Base { readonly attribute unsigned long length; getter long item(unsigned long index); }; interface I : Base { iterable<long>; };"]).unwrap();
    check(&["interface I { readonly attribute unsigned long length; getter long item(unsigned long index); async_iterable<DOMString>; }; interface J { readonly attribute unsigned long length; getter long item(unsigned long index); async_iterable<DOMString,long>; };"]).unwrap();
}

#[test]
fn async_arguments_are_optional_including_implicitly_optional_variadics() {
    invalid(
        "interface I { async_iterable<long>(long count); };",
        ErrorKind::AsyncIteratorArgument,
    );
    invalid(
        "interface I { async_iterable<DOMString,long>(optional long start=0, long count); };",
        ErrorKind::AsyncIteratorArgument,
    );
    check(&["dictionary D {}; interface I { async_iterable<long>(optional D options = {}, optional long count = 0); }; interface J { async_iterable<long>(DOMString... options); };"]).unwrap();
}

#[test]
fn type_identity_preserves_atoms_nominal_types_generics_and_nullability() {
    for (left, right) in [
        ("long", "long"),
        ("Count", "unsigned long"),
        ("Signed", "long"),
        ("A", "A"),
        ("E", "E"),
        ("D", "D"),
        ("C", "C"),
        ("ArrayBuffer", "ArrayBuffer"),
        ("sequence<long?>", "sequence<long?>"),
        ("record<DOMString,long>", "record<DOMString,long>"),
        ("FrozenArray<long>", "FrozenArray<long>"),
    ] {
        check(&[&value_source(left, right)]).unwrap();
    }
    for (left, right) in [
        ("long", "unsigned long"),
        ("double", "unrestricted double"),
        ("A", "B"),
        ("E", "F"),
        ("C", "C2"),
        ("ArrayBuffer", "DataView"),
        ("_ArrayBuffer", "ArrayBuffer"),
        ("long?", "long"),
        ("sequence<long>", "sequence<double>"),
        ("FrozenArray<long>", "sequence<long>"),
        ("record<DOMString,long>", "record<USVString,long>"),
    ] {
        invalid(&value_source(left, right), ErrorKind::ValueIteratorType);
    }
}

#[test]
fn union_comparison_ignores_order_and_parenthesization_and_compares_container_parameters() {
    for (left, right) in [
        ("(long or DOMString)", "(DOMString or long)"),
        (
            "((long or DOMString) or boolean)",
            "(boolean or (DOMString or long))",
        ),
        ("(long or DOMString)?", "(DOMString or long?)"),
        (
            "sequence<(long? or DOMString)>",
            "sequence<(DOMString or long)?>",
        ),
        (
            "(sequence<long> or DOMString)",
            "(DOMString or sequence<long>)",
        ),
    ] {
        check(&[&value_source(left, right)]).unwrap();
    }
    for (left, right) in [
        ("(long or DOMString)", "(DOMString or double)"),
        ("(long or DOMString)?", "(long or DOMString)"),
        (
            "(sequence<long> or DOMString)",
            "(DOMString or sequence<double>)",
        ),
        ("(A or DOMString)", "(B or DOMString)"),
    ] {
        invalid(&value_source(left, right), ErrorKind::ValueIteratorType);
    }
}

#[test]
fn comparison_cache_quota_is_shared_across_collections_and_zero_is_lazy() {
    let source = "interface I { readonly attribute unsigned long length; getter long item(unsigned long index); iterable<long>; };";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    for (limit, ok) in [(0, false), (1, true)] {
        let result = super::super::build_within(
            &index,
            &members,
            Limits {
                collection_pairs: limit,
                ..Limits::default()
            },
            &Budget::unlimited(),
        );
        assert_eq!(result.is_ok(), ok);
        if let Err(e) = result {
            assert_eq!(e.kind, ErrorKind::Limit(Limit::CollectionPairs));
        }
    }
    let source = format!(
        "{source} interface J {{ readonly attribute unsigned long length; getter long item(unsigned long index); iterable<long>; }};"
    );
    let index = index::build(&[&source]).unwrap();
    let members = members::build(&index).unwrap();
    assert_eq!(
        super::super::build_within(
            &index,
            &members,
            Limits {
                collection_pairs: 1,
                ..Limits::default()
            },
            &Budget::unlimited()
        )
        .unwrap_err()
        .kind,
        ErrorKind::Limit(Limit::CollectionPairs)
    );
    super::super::build_within(
        &index,
        &members,
        Limits {
            collection_pairs: 2,
            ..Limits::default()
        },
        &Budget::unlimited(),
    )
    .unwrap();
    let source = "typedef long Value; interface I { readonly attribute unsigned long length; getter Value item(unsigned long index); iterable<Value>; }; interface Map { maplike<DOMString,long>; };";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    super::super::build_within(
        &index,
        &members,
        Limits {
            collection_pairs: 0,
            ..Limits::default()
        },
        &Budget::unlimited(),
    )
    .unwrap();
}

#[test]
fn budget_boundary_and_external_cancellation_remain_host_failures() {
    let source = value_source(
        "sequence<(long or DOMString)>",
        "sequence<(DOMString or long)>",
    );
    let index = index::build(&[&source]).unwrap();
    let members = members::build(&index).unwrap();
    let unlimited = Budget::unlimited();
    super::super::build_within(&index, &members, Limits::default(), &unlimited).unwrap();
    let cost = u64::MAX - unlimited.remaining();
    let exact = Budget::new(cost);
    super::super::build_within(&index, &members, Limits::default(), &exact).unwrap();
    assert_eq!(exact.remaining(), 0);
    assert_eq!(
        super::super::build_within(&index, &members, Limits::default(), &Budget::new(cost - 1))
            .unwrap_err()
            .kind,
        ErrorKind::Exhausted(Exhausted::Steps)
    );
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(
        super::super::build_within(
            &index,
            &members,
            Limits::default(),
            &Budget::with_cancel(100_000, cancel)
        )
        .unwrap_err()
        .kind,
        ErrorKind::Exhausted(Exhausted::Cancelled)
    );
}

#[test]
fn deep_container_equality_and_drop_fit_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let ty = format!("{}long{}", "sequence<".repeat(2000), ">".repeat(2000));
            let source = value_source(&ty, &ty);
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            let types = super::super::build_within(
                &index,
                &members,
                Limits::default(),
                &Budget::new(750_000),
            )
            .unwrap();
            drop(types);
            drop(members);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_union_equality_does_not_materialize_every_prefix() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut prefix = String::new();
            for n in 0..1000 {
                prefix.push_str(&format!("interface N{n} {{}}; "));
            }
            let union = |reverse| {
                let mut text = String::new();
                for n in 0..999 {
                    let n = if reverse { 999 - n } else { n };
                    text.push_str(&format!("(N{n} or "));
                }
                text.push_str(if reverse { "N0" } else { "N999" });
                text.push_str(&")".repeat(999));
                text
            };
            let source = format!("{prefix} {}", value_source(&union(false), &union(true)));
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            let types = super::super::build_within(
                &index,
                &members,
                Limits::default(),
                &Budget::new(600_000),
            )
            .unwrap();
            drop(types);
            drop(members);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn long_inheritance_and_shared_mixins_fit_small_type_quotas() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::from("interface mixin M { ");
            for n in 0..1000 {
                source.push_str(&format!("readonly attribute long field{n}; "));
            }
            source.push_str("}; interface Base {}; ");
            for n in 0..1000 {
                source.push_str(&format!(
                    "interface I{n} : Base {{ maplike<DOMString,long>; }}; I{n} includes M; "
                ));
            }
            let index = index::build(&[&source]).unwrap();
            let members = members::build_within(
                &index,
                crate::members::Limits {
                    records: 200_000,
                    visits: 3_000_000,
                },
                &Budget::new(20_000_000),
            )
            .unwrap();
            let types = super::super::build_within(
                &index,
                &members,
                Limits::default(),
                &Budget::new(300_000),
            )
            .unwrap();
            drop(types);
            drop(members);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn long_ancestor_paths_keep_one_small_name_summary_per_interface() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            for reserved in [false, true] {
                let mut source = if reserved {
                    "interface I0 { readonly attribute long keys; }; ".to_owned()
                } else {
                    "interface I0 {}; ".to_owned()
                };
                for n in 1..2000 {
                    source.push_str(&format!(
                        "interface I{n} : I{} {{ undefined f{n}(); }}; ",
                        n - 1
                    ));
                }
                source.push_str("interface Leaf : I1999 { maplike<DOMString,long>; };");
                let index = index::build(&[&source]).unwrap();
                let members = members::build(&index).unwrap();
                let result = super::super::build_within(
                    &index,
                    &members,
                    Limits::default(),
                    &Budget::new(500_000),
                );
                assert_eq!(result.is_err(), reserved);
                if let Err(e) = result {
                    assert_eq!(e.kind, ErrorKind::CollectionMemberName);
                }
                drop(members);
                drop(index);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn failed_comparisons_do_not_mark_unfinished_pairs_as_equal() {
    let index = index::build(&["typedef sequence<long> Left; typedef sequence<double> Wrong; typedef sequence<long> Equal;"]).unwrap();
    let members = members::build(&index).unwrap();
    let types = super::super::build(&index, &members).unwrap();
    let get = |name| types.typedef(index.find(name).unwrap()).unwrap();
    let budget = Budget::unlimited();
    let work = Work {
        limits: Limits::default(),
        budget: &budget,
    };
    let mut identity = Identity::new(&types, &work).unwrap();
    assert!(!identity.same(get("Left"), get("Wrong")).unwrap());
    assert!(identity.same(get("Left"), get("Equal")).unwrap());
    assert!(!identity.same(get("Wrong"), get("Equal")).unwrap());
    assert!(identity.same(get("Equal"), get("Left")).unwrap());
}
