use super::*;
use crate::{index, members};

fn check(sources: &[&str]) -> Result<Types, Error> {
    let index = index::build(sources).expect("syntaktisch gültige Namen");
    let members = members::build(&index).expect("gültige Member-Namen");
    super::super::build(&index, &members)
}

fn cycle(sources: &[&str]) -> Error {
    let error = check(sources).expect_err("zyklischer Dictionary-Typ");
    assert_eq!(error.kind, ErrorKind::DictionaryCycle, "{sources:?}");
    let at = error.location.as_ref().expect("echtes Feld");
    assert!(sources[at.source].get(at.span.clone()).is_some());
    error
}

#[test]
fn direct_mutual_and_partial_field_cycles_are_rejected() {
    let e = cycle(&["dictionary D { D self; };"]);
    assert_eq!(
        &"dictionary D { D self; };"[e.location.unwrap().span],
        "self"
    );
    cycle(&["dictionary A { B next; }; dictionary B { A previous; };"]);
    cycle(&["partial dictionary D { D self; }; dictionary D {};"]);
    cycle(&["partial dictionary A { B next; }; dictionary A {}; dictionary B { A previous; };"]);
}

#[test]
fn every_standard_container_and_typedef_path_is_followed() {
    for wrapped in [
        "D",
        "sequence<D>",
        "FrozenArray<D>",
        "record<DOMString,D>",
        "(D or DOMString)",
        "sequence<D>?",
        "sequence<D?>",
        "record<USVString,FrozenArray<sequence<D>>>",
        "[Opaque] sequence<D>",
    ] {
        cycle(&[&format!(
            "typedef {wrapped} Alias; dictionary D {{ Alias recursive; }};"
        )]);
    }
    cycle(&[
        "typedef sequence<D> Inner; typedef FrozenArray<Inner> Outer; dictionary D { Outer recursive; };",
    ]);
}

#[test]
fn later_sources_aliases_and_partial_fields_keep_a_real_field_location() {
    let sources = [
        "partial dictionary A { List next; };",
        "typedef sequence<B> List; dictionary B { A previous; };",
        "dictionary A {};",
    ];
    let e = cycle(&sources);
    let at = e.location.unwrap();
    assert!(matches!(&sources[at.source][at.span], "next" | "previous"));
}

#[test]
fn a_descendant_type_in_a_base_field_is_rejected_even_without_descendant_fields() {
    cycle(&["dictionary Base { Child child; }; dictionary Child : Base {};"]);
    cycle(&["dictionary Child : Base {}; dictionary Base { Child child; };"]);
    cycle(&[
        "dictionary Base { Last child; }; dictionary Middle : Base {}; dictionary Last : Middle {};",
    ]);
    cycle(&[
        "dictionary Base {}; dictionary Child : Base {}; partial dictionary Base { Child child; };",
    ]);
}

#[test]
fn inherited_fields_and_cross_dictionary_paths_both_contribute() {
    cycle(&[
        "dictionary Base { Holder value; }; dictionary Child : Base {}; dictionary Holder { Child child; };",
    ]);
    cycle(&["dictionary A { B next; }; dictionary B : C {}; dictionary C { A back; };"]);
    cycle(&["dictionary C { A back; }; dictionary B : C {}; dictionary A { sequence<B> next; };"]);
}

#[test]
fn ancestors_may_be_field_types_when_the_path_does_not_return_to_the_owner() {
    check(&["dictionary Base { long number; }; dictionary Child : Base { Base ancestor; };"])
        .unwrap();
    check(&["dictionary Child : Base { record<DOMString,Base> ancestors; }; dictionary Base { long number; };"]).unwrap();
}

#[test]
fn reference_boundaries_are_not_treated_as_by_value_inclusion_paths() {
    for wrapped in ["Promise<D>", "async_sequence<D>", "async_sequence<D>?"] {
        check(&[&format!(
            "typedef {wrapped} Ref; dictionary D {{ Ref value; }};"
        )])
        .unwrap();
    }
    check(&["callback F = undefined (optional D value = {}); dictionary D { F _callback; object opaque; };"]).unwrap();
    check(&["interface Handle {}; dictionary D { Handle handle; };"]).unwrap();
    // Die Grenze gilt auch tief im Container: Sequence -> Promise stoppt.
    check(&["dictionary D { sequence<Promise<D>> values; };"]).unwrap();
    cycle(&["dictionary D { sequence<FrozenArray<D>> values; };"]);
}

#[test]
fn a_safe_prefix_does_not_hide_a_later_cycle_or_become_its_reported_field() {
    let source = "dictionary Safe { long number; }; dictionary B { Safe first; C next; }; dictionary C { Safe harmless; B back; };";
    let e = cycle(&[source]);
    let at = e.location.unwrap();
    assert!(matches!(&source[at.span], "next" | "back"));
}

#[test]
fn empty_dictionaries_and_shared_acyclic_dependencies_are_accepted() {
    check(&["dictionary Empty {}; dictionary A { Empty value; }; dictionary B { A a; Empty empty; }; dictionary C { A a; Empty empty; };"]).unwrap();
    check(&["dictionary Empty {}; partial dictionary Empty {};"]).unwrap();
}

#[test]
fn node_and_edge_limits_are_checked_even_for_inheritance_and_empty_dictionaries() {
    let source = "dictionary Base { long base; }; dictionary Child : Base { long own; };";
    let index = index::build(&[source]).unwrap();
    let members = members::build(&index).unwrap();
    let limits = Limits {
        dictionary_nodes: 2,
        dictionary_edges: 3,
        ..Limits::default()
    };
    super::super::build_within(&index, &members, limits, &Budget::unlimited()).unwrap();
    for (limits, kind) in [
        (
            Limits {
                dictionary_nodes: 1,
                ..limits
            },
            Limit::DictionaryNodes,
        ),
        (
            Limits {
                dictionary_edges: 2,
                ..limits
            },
            Limit::DictionaryEdges,
        ),
        (
            Limits {
                dictionary_edges: 0,
                ..limits
            },
            Limit::DictionaryEdges,
        ),
    ] {
        let error =
            super::super::build_within(&index, &members, limits, &Budget::unlimited()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Limit(kind));
    }
    let index = index::build(&["dictionary Empty {};"]).unwrap();
    let members = members::build(&index).unwrap();
    assert!(
        super::super::build_within(
            &index,
            &members,
            Limits {
                dictionary_nodes: 1,
                dictionary_edges: 0,
                ..limits
            },
            &Budget::unlimited()
        )
        .is_ok()
    );
    assert_eq!(
        super::super::build_within(
            &index,
            &members,
            Limits {
                dictionary_nodes: 0,
                ..limits
            },
            &Budget::unlimited()
        )
        .unwrap_err()
        .kind,
        ErrorKind::Limit(Limit::DictionaryNodes)
    );
    let index = index::build(&["typedef long Number;"]).unwrap();
    let members = members::build(&index).unwrap();
    assert!(
        super::super::build_within(
            &index,
            &members,
            Limits {
                dictionary_nodes: 0,
                dictionary_edges: 0,
                ..limits
            },
            &Budget::unlimited()
        )
        .is_ok()
    );
}

#[test]
fn graph_work_uses_the_remaining_budget_instead_of_starting_a_new_one() {
    let source =
        "typedef long Count; dictionary Base { Count n; }; dictionary Child : Base { Count m; };";
    let unlimited = Budget::unlimited();
    let index = index::build_within(&[source], index::Limits::default(), &unlimited).unwrap();
    let members = members::build_within(&index, members::Limits::default(), &unlimited).unwrap();
    super::super::build_within(&index, &members, Limits::default(), &unlimited).unwrap();
    let total = u64::MAX - unlimited.remaining();
    for (steps, success) in [(total, true), (total - 1, false)] {
        let budget = Budget::new(steps);
        let index = index::build_within(&[source], index::Limits::default(), &budget).unwrap();
        let members = members::build_within(&index, members::Limits::default(), &budget).unwrap();
        let result = super::super::build_within(&index, &members, Limits::default(), &budget);
        if success {
            assert!(result.is_ok());
            assert_eq!(budget.remaining(), 0);
        } else {
            assert_eq!(
                result.unwrap_err().kind,
                ErrorKind::Exhausted(Exhausted::Steps)
            );
            assert_eq!(budget.exhausted(), Some(Exhausted::Steps));
        }
    }
}

#[test]
fn shared_dictionary_diamonds_are_not_unfolded_exponentially() {
    let mut source = String::from("dictionary D0 { long value; }; ");
    for n in 1..36 {
        source.push_str(&format!(
            "dictionary D{n} {{ D{} left; D{} right; }}; ",
            n - 1,
            n - 1
        ));
    }
    let index = index::build(&[&source]).unwrap();
    let members = members::build(&index).unwrap();
    let limits = Limits {
        dictionary_nodes: 36,
        dictionary_edges: 150,
        ..Limits::default()
    };
    let types = super::super::build_within(&index, &members, limits, &Budget::new(20_000)).unwrap();
    assert_eq!(types.len(), 71);
    let cyclic = source.replace("long value;", "D35 back;");
    let index = index::build(&[&cyclic]).unwrap();
    let members = members::build(&index).unwrap();
    assert_eq!(
        super::super::build_within(&index, &members, limits, &Budget::new(20_000))
            .unwrap_err()
            .kind,
        ErrorKind::DictionaryCycle
    );
}

#[test]
fn long_field_chains_are_iterative_and_drop_on_a_small_rust_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::new();
            for n in 0..1999 {
                source.push_str(&format!("dictionary D{n} {{ D{} next; }}; ", n + 1));
            }
            source.push_str("dictionary D1999 { long end; };");
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            let types = super::super::build_within(
                &index,
                &members,
                Limits {
                    dictionary_nodes: 2000,
                    dictionary_edges: 4000,
                    ..Limits::default()
                },
                &Budget::new(250_000),
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
fn long_reversed_inheritance_reuses_parent_fields_without_copying_them() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut source = String::from("typedef long Count; ");
            for n in (1..2000).rev() {
                source.push_str(&format!(
                    "dictionary D{n} : D{} {{ Count value{n}; }}; ",
                    n - 1
                ));
            }
            source.push_str("dictionary D0 { Count value0; };");
            let index = index::build(&[&source]).unwrap();
            let members = members::build(&index).unwrap();
            let types = super::super::build_within(
                &index,
                &members,
                Limits {
                    dictionary_nodes: 2000,
                    dictionary_edges: 6000,
                    ..Limits::default()
                },
                &Budget::new(300_000),
            )
            .unwrap();
            assert_eq!(types.len(), 2001);
            drop(types);
            drop(members);
            drop(index);
        })
        .unwrap()
        .join()
        .unwrap();
}
