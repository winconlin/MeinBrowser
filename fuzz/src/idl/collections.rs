//! Independent declaration, reserved-name, iterator-type and async oracles.

use super::*;

const DECLS: [&str; 8] = [
    "",
    "iterable<long>",
    "iterable<DOMString,long>",
    "async_iterable<long>",
    "readonly maplike<DOMString,long>",
    "maplike<DOMString,long>",
    "readonly setlike<long>",
    "setlike<long>",
];
pub(super) fn model(data: &[u8]) {
    let byte = |at| data.get(at).copied().unwrap_or(0);
    declarations(byte(0), byte(1), byte(2));
    names(byte(3), byte(4), byte(5), byte(6));
    iterator(byte(7), byte(8), byte(9));
    asynchronous(byte(10), byte(11));
}
fn declarations(parent: u8, own: u8, flags: u8) {
    let parent = usize::from(parent % 8);
    let own = usize::from(own % 8);
    let parent_indexed = flags & 1 != 0;
    let own_indexed = flags & 2 != 0;
    let duplicate_parent = parent != 0 && flags & 8 != 0;
    let duplicate_own = own != 0 && flags & 4 != 0;
    let body = |row: usize, indexed: bool, duplicate: bool| {
        let mut text = String::new();
        if row != 0 {
            text.push_str(DECLS[row]);
            text.push(';');
            if duplicate {
                text.push_str(DECLS[row]);
                text.push(';');
            }
        }
        if indexed {
            text.push_str(
                "readonly attribute unsigned long length; getter long item(unsigned long index);",
            );
        }
        text
    };
    let early = format!(
        "interface Child : Base {{ {} }};",
        body(own, own_indexed, duplicate_own)
    );
    let late = format!(
        "interface Base {{ {} }};",
        body(parent, parent_indexed, duplicate_parent)
    );
    let condition = |row: usize, indexed: bool| {
        if row == 1 && !indexed {
            Some(types::ErrorKind::ValueIteratorGetter)
        } else if matches!(row, 2 | 4..=7) && indexed {
            Some(types::ErrorKind::CollectionIndexedGetter)
        } else {
            None
        }
    };
    let expected = if duplicate_own || duplicate_parent {
        Some(types::ErrorKind::CollectionConflict)
    } else {
        condition(parent, parent_indexed).or_else(|| {
            if parent != 0 && own != 0 {
                Some(types::ErrorKind::CollectionConflict)
            } else {
                condition(own, parent_indexed || own_indexed)
            }
        })
    };
    check(&[&early, &late], expected);
}
fn names(row: u8, name: u8, kind: u8, place: u8) {
    let row = usize::from(row % 6);
    let name = usize::from(name % 11);
    let kind = kind % 6;
    let place = place % 3;
    let declarations = [
        "iterable<DOMString,long>",
        "async_iterable<long>",
        "readonly maplike<DOMString,long>",
        "maplike<DOMString,long>",
        "readonly setlike<long>",
        "setlike<long>",
    ];
    let names = [
        "entries", "forEach", "keys", "values", "get", "has", "size", "clear", "delete", "set",
        "add",
    ];
    // Direct transcription of the normative name lists. Static operations
    // are excluded; named special operations also declare regular operations.
    const COMMON: [&[usize]; 6] = [
        &[0, 1, 2, 3],
        &[0, 2, 3],
        &[0, 1, 2, 3, 4, 5, 6],
        &[0, 1, 2, 3, 4, 5, 6],
        &[0, 1, 2, 3, 5, 6],
        &[0, 1, 2, 3, 5, 6],
    ];
    let n = names[name];
    let member = match kind {
        0 => format!("attribute long {n}"),
        1 => format!("const long {n}=0"),
        2 => format!("undefined {n}()"),
        3 => format!("static undefined {n}()"),
        4 => format!("static readonly attribute long {n}"),
        _ => format!("getter long {n}(DOMString key)"),
    };
    let declaration = declarations[row];
    let (early, late) = match place {
        1 => (
            format!("interface Child : Base {{ {declaration}; }};"),
            format!("interface Base {{ {member}; }};"),
        ),
        2 if !matches!(kind, 3..=5) => (
            format!("interface Child {{ {declaration}; }}; Child includes M;"),
            format!("interface mixin M {{ {member}; }};"),
        ),
        _ => (
            format!("interface Child {{ {declaration}; {member}; }};"),
            String::new(),
        ),
    };
    let non_operation = matches!(kind, 0 | 1 | 4);
    let mutation = non_operation
        && ((row == 3 && [7, 8, 9].contains(&name)) || (row == 5 && [7, 8, 10].contains(&name)));
    let forbidden = kind != 3 && (COMMON[row].contains(&name) || mutation);
    check(
        &[&early, &late],
        forbidden.then_some(types::ErrorKind::CollectionMemberName),
    );
}
fn iterator(left: u8, right: u8, depth: u8) {
    // Labels are mathematical equality classes selected before generating
    // sources. They do not parse or compare type graphs like Identity does.
    const TYPES: [(&str, u8); 24] = [
        ("long", 0),
        ("Signed", 0),
        ("unsigned long", 1),
        ("Count", 1),
        ("DOMString", 2),
        ("USVString", 3),
        ("A", 4),
        ("B", 5),
        ("E", 6),
        ("F", 7),
        ("ArrayBuffer", 8),
        ("DataView", 9),
        ("long?", 10),
        ("sequence<long>", 11),
        ("FrozenArray<long>", 12),
        ("record<DOMString,long>", 13),
        ("record<USVString,long>", 14),
        ("(long or DOMString)", 15),
        ("(DOMString or long)", 15),
        ("(long or DOMString)?", 16),
        ("(DOMString or long?)", 16),
        ("(sequence<long> or DOMString)", 17),
        ("(DOMString or sequence<long>)", 17),
        ("sequence<double>", 18),
    ];
    let (left, lc) = TYPES[usize::from(left) % TYPES.len()];
    let (right, rc) = TYPES[usize::from(right) % TYPES.len()];
    let depth = usize::from(depth % 8);
    let wrap = |text: &str| format!("{}{text}{}", "sequence<".repeat(depth), ">".repeat(depth));
    let early = format!(
        "interface I {{ readonly attribute unsigned long length; getter {} item(unsigned long index); iterable<{}>; }};",
        wrap(left),
        wrap(right)
    );
    let late = "interface A {}; interface B : A {}; enum E {\"x\"}; enum F {\"x\"}; typedef [Opaque] long Signed; typedef [Opaque] unsigned long Count;";
    check(
        &[&early, late],
        (lc != rc).then_some(types::ErrorKind::ValueIteratorType),
    );
}
fn asynchronous(count: u8, flags: u8) {
    let count = usize::from(count % 4);
    let mut required = false;
    let mut args = Vec::new();
    for n in 0..count {
        let variadic = n + 1 == count && flags & 8 != 0;
        let optional = flags & (1 << n) != 0;
        required |= !optional && !variadic;
        args.push(if variadic {
            format!("long... arg{n}")
        } else if optional {
            format!("optional long arg{n}")
        } else {
            format!("long arg{n}")
        });
    }
    let source = format!(
        "interface I {{ async_iterable<long>({}); }};",
        args.join(",")
    );
    check(
        &[&source],
        required.then_some(types::ErrorKind::AsyncIteratorArgument),
    );
}
fn check(sources: &[&str], expected: Option<types::ErrorKind>) {
    let budget = Budget::new(100_000);
    let index = index::build_within(
        sources,
        index::Limits {
            sources: 2,
            source_bytes: 8192,
            nodes: 8192,
            records: 1024,
            parser: Limits {
                source_bytes: 8192,
                nodes: 8192,
                pending: 8192,
            },
        },
        &budget,
    )
    .expect("linked collection model");
    let members = members::build_within(
        &index,
        members::Limits {
            records: 1024,
            visits: 4096,
        },
        &budget,
    )
    .expect("valid model member names");
    let result = types::build_within(
        &index,
        &members,
        types::Limits {
            source_nodes: 8192,
            types: 1024,
            edges: 2048,
            dictionary_nodes: 16,
            dictionary_edges: 512,
            union_visits: 8192,
            union_interfaces: 16,
            collection_pairs: 4096,
        },
        &budget,
    );
    if let Some(expected) = expected {
        let e = result.expect_err("independent collection violation");
        assert_eq!(e.kind, expected);
        let at = e.location.expect("source position");
        assert!(
            sources
                .get(at.source)
                .and_then(|s| s.get(at.span))
                .is_some()
        );
    } else {
        check_types(
            &index,
            &members,
            &result.expect("independently valid collections"),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declaration_oracle_covers_all_1024_inheritance_index_and_duplicate_cases() {
        for parent in 0..8 {
            for own in 0..8 {
                for flags in 0..16 {
                    declarations(parent, own, flags);
                }
            }
        }
    }
    #[test]
    fn name_oracle_covers_1188_kind_name_and_location_parameters() {
        for row in 0..6 {
            for name in 0..11 {
                for kind in 0..6 {
                    for place in 0..3 {
                        names(row, name, kind, place);
                    }
                }
            }
        }
    }
    #[test]
    fn type_oracle_covers_all_576_equivalence_class_pairs() {
        for left in 0..24 {
            for right in 0..24 {
                iterator(left, right, (left ^ right) % 4);
            }
        }
    }
    #[test]
    fn async_oracle_covers_64_argument_flag_cases_and_varied_entry_bytes() {
        for count in 0..4 {
            for flags in 0..16 {
                asynchronous(count, flags);
            }
        }
        for n in 0u8..64 {
            let data: Vec<u8> = (0..20).map(|at| n.wrapping_add(at * 13)).collect();
            model(&data);
        }
    }
}
