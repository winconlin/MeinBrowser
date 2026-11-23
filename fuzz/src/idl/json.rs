//! Independent JSON eligibility, matrix closure, presence and method oracles.

use super::*;
pub(super) fn model(data: &[u8]) {
    let byte = |at| data.get(at).copied().unwrap_or(0);
    shape(byte(0), byte(1));
    dictionaries(data);
    interfaces(data);
    signature(byte(2));
}

fn build<'a>(
    sources: &[&'a str],
    expected: Option<types::ErrorKind>,
    verify: impl FnOnce(&index::Index<'a>, &members::Members<'a>, &types::Types),
) {
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
    .expect("linked JSON model");
    let members = members::build_within(
        &index,
        members::Limits {
            records: 1024,
            visits: 4096,
        },
        &budget,
    )
    .expect("valid JSON model member names");
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
        let e = result.expect_err("independently invalid toJSON");
        assert_eq!(e.kind, expected);
        let at = e.location.expect("source position");
        assert!(
            sources
                .get(at.source)
                .and_then(|s| s.get(at.span))
                .is_some()
        );
    } else {
        let types = result.expect("valid JSON model graph");
        check_types(&index, &members, &types);
        verify(&index, &members, &types);
    }
}
fn result(
    index: &index::Index<'_>,
    members: &members::Members<'_>,
    types: &types::Types,
    owner: &str,
    at: usize,
) -> bool {
    let member = &members.members(index.find(owner).unwrap()).unwrap()[at];
    let id = types
        .at(member.location.source, member.types[0].unwrap())
        .unwrap();
    types.is_json(id).unwrap()
}

fn shape(row: u8, wrapper: u8) {
    const ROWS: [(&str, bool); 32] = [
        ("long", true),
        ("boolean", true),
        ("unrestricted double", true),
        ("DOMString", true),
        ("ByteString", true),
        ("USVString", true),
        ("Mode", true),
        ("object", true),
        ("any", false),
        ("undefined", false),
        ("bigint", false),
        ("symbol", false),
        ("ArrayBuffer", false),
        ("Uint8Array", false),
        ("Promise<long>", false),
        ("async_sequence<long>", false),
        ("ObservableArray<long>", false),
        ("Plain", false),
        ("Rich", true),
        ("Derived", true),
        ("F", false),
        ("C", false),
        ("Empty", true),
        ("Bad", false),
        ("Count", true),
        ("long?", true),
        ("bigint?", false),
        ("(long or DOMString)", true),
        ("(long or bigint)", false),
        ("record<DOMString,long>", true),
        ("record<DOMString,any>", false),
        ("sequence<Plain>", false),
    ];
    let (leaf, expected) = ROWS[usize::from(row) % ROWS.len()];
    let ty = match wrapper % 4 {
        0 => leaf.to_owned(),
        1 => format!("sequence<{leaf}>"),
        2 => format!("FrozenArray<{leaf}>"),
        _ => format!("record<DOMString,{leaf}>"),
    };
    let probe =
        if wrapper % 4 == 2 || (row as usize % ROWS.len() == 16 && wrapper.is_multiple_of(4)) {
            format!("readonly attribute {ty} probe;")
        } else {
            format!("{ty} probe();")
        };
    let early = format!("interface Probe {{ {probe} }};");
    let late = "enum Mode {\"on\"}; interface Plain {}; interface Rich { object toJSON(); }; interface Derived : Rich {}; callback F = undefined (); callback interface C { object toJSON(); }; dictionary Empty {}; dictionary Bad { any opaque; }; typedef [Tag] long Count;";
    let error = (row as usize % ROWS.len() == 16 && !wrapper.is_multiple_of(4))
        .then_some(types::ErrorKind::ObservableArrayUse);
    build(&[&early, late], error, |index, members, types| {
        assert_eq!(result(index, members, types, "Probe", 0), expected)
    });
}

fn dictionaries(data: &[u8]) {
    let byte = |at| data.get(at).copied().unwrap_or(0);
    let count = usize::from(byte(0) % 8) + 1;
    let mut bad = [false; 8];
    let mut frozen_use = false;
    let mut reach = [[false; 8]; 8];
    let mut early = String::new();
    let mut late = String::new();
    for n in 0..count {
        let control = byte(n + 10);
        bad[n] = control & 1 != 0;
        early.push_str(&format!(
            "partial dictionary D{n} {{ {} {} value{n}; }}; ",
            if control & 4 != 0 { "required" } else { "" },
            if bad[n] { "any" } else { "long" }
        ));
        late.push_str(&format!("dictionary D{n}"));
        if n > 0 && control & 2 != 0 {
            let parent = usize::from(byte(n + 3)) % n;
            late.push_str(&format!(" : D{parent}"));
            reach[n][parent] = true;
        }
        late.push_str(" { ");
        if n > 0 {
            let target = usize::from(byte(n + 2)) % n;
            let wrapper = byte(n + 1) % 7;
            frozen_use |= wrapper == 2;
            let d = format!("D{target}");
            let ty = match wrapper {
                0 => d,
                1 => format!("sequence<{d}>"),
                2 => format!("FrozenArray<{d}>"),
                3 => format!("record<DOMString,{d}>"),
                4 => format!("({d} or DOMString)"),
                5 => format!("Promise<{d}>"),
                _ => format!("async_sequence<{d}>"),
            };
            late.push_str(&format!("{ty} edge{n}; "));
            if wrapper < 5 {
                reach[n][target] = true;
            } else {
                bad[n] = true;
            }
        }
        late.push_str("}; ");
    }
    early.push_str("interface Probe { ");
    for n in 0..count {
        early.push_str(&format!("D{n} value{n}(); "));
    }
    early.push_str("};");
    // Transitive matrix closure differs from the implementation's shared
    // type/dictionary DFS. A node is JSON iff no reachable local failure exists.
    for via in 0..count {
        for from in 0..count {
            for to in 0..count {
                reach[from][to] |= reach[from][via] && reach[via][to];
            }
        }
    }
    let expected: Vec<_> = (0..count)
        .map(|n| !bad[n] && !(0..count).any(|j| reach[n][j] && bad[j]))
        .collect();
    build(
        &[&early, &late],
        frozen_use.then_some(types::ErrorKind::FrozenArrayUse),
        |index, members, types| {
            for (n, expected) in expected.into_iter().enumerate() {
                assert_eq!(result(index, members, types, "Probe", n), expected);
            }
        },
    );
}

fn interfaces(data: &[u8]) {
    let byte = |at| data.get(at).copied().unwrap_or(0);
    let count = usize::from(byte(0) % 8) + 1;
    let mut own = [false; 8];
    let mut reach = [[false; 8]; 8];
    let mut early = String::from("interface Probe { ");
    for n in 0..count {
        early.push_str(&format!("I{n} value{n}(); "));
    }
    early.push_str("}; ");
    let mut late = String::from("interface mixin M { object toJSON(); }; ");
    for n in 0..count {
        let mode = byte(n + 1) % 4;
        let control = byte(n + 10);
        late.push_str(&format!("interface I{n}"));
        if n > 0 && control & 1 != 0 {
            let parent = usize::from(byte(n + 3)) % n;
            late.push_str(&format!(" : I{parent}"));
            reach[n][parent] = true;
        }
        late.push_str(" { ");
        late.push_str(
            [
                "",
                "object toJSON();",
                "static object toJSON(long ignored);",
                "readonly attribute DOMString toJSON;",
            ][usize::from(mode)],
        );
        late.push_str(" }; ");
        let include = control & 2 != 0 && matches!(mode, 0 | 2);
        if include {
            late.push_str(&format!("I{n} includes M; "));
        }
        own[n] = mode == 1 || include;
    }
    for via in 0..count {
        for from in 0..count {
            for to in 0..count {
                reach[from][to] |= reach[from][via] && reach[via][to];
            }
        }
    }
    let expected: Vec<_> = (0..count)
        .map(|n| own[n] || (0..count).any(|j| reach[n][j] && own[j]))
        .collect();
    build(&[&early, &late], None, |index, members, types| {
        for (n, expected) in expected.into_iter().enumerate() {
            assert_eq!(result(index, members, types, "Probe", n), expected);
        }
    });
}

fn signature(case: u8) {
    let sources = [
        "interface I { object toJSON(); };",
        "interface I { bigint toJSON(); };",
        "interface I { object toJSON(optional long value=0); };",
        "interface I { static bigint toJSON(long ignored); };",
        "interface I { getter long toJSON(DOMString key); };",
        "interface mixin M { object toJSON(); };",
        "interface mixin M { bigint toJSON(); };",
        "interface mixin M { object toJSON(long value); };",
        "namespace N { object toJSON(); };",
        "namespace N { bigint toJSON(); };",
        "namespace N { object toJSON(long value); };",
        "callback interface C { object toJSON(); };",
        "callback interface C { bigint toJSON(); };",
        "callback interface C { object toJSON(long value); };",
        "interface I { object _toJSON(long value); };",
        "typedef Promise<long> Future; interface I { Future toJSON(); };",
    ];
    let expected = [
        None,
        Some(types::ErrorKind::ToJSONType),
        Some(types::ErrorKind::ToJSONArguments),
        None,
        Some(types::ErrorKind::ToJSONArguments),
        None,
        Some(types::ErrorKind::ToJSONType),
        Some(types::ErrorKind::ToJSONArguments),
        None,
        Some(types::ErrorKind::ToJSONType),
        Some(types::ErrorKind::ToJSONArguments),
        None,
        Some(types::ErrorKind::ToJSONType),
        Some(types::ErrorKind::ToJSONArguments),
        Some(types::ErrorKind::ToJSONArguments),
        Some(types::ErrorKind::ToJSONType),
    ];
    let case = usize::from(case % 16);
    build(&[sources[case]], expected[case], |_, _, _| {});
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn type_oracle_covers_128_leaf_and_wrapper_parameters() {
        for row in 0..32 {
            for wrapper in 0..4 {
                shape(row, wrapper);
            }
        }
    }
    #[test]
    fn dictionary_matrix_oracle_covers_448_graph_parameters() {
        for count in 0..8 {
            for wrapper in 0..7 {
                for flags in 0..8 {
                    let mut data = vec![wrapper; 24];
                    data[0] = count;
                    data[10..18].fill(flags);
                    dictionaries(&data);
                }
            }
        }
    }
    #[test]
    fn interface_matrix_oracle_covers_128_presence_parameters() {
        for count in 0..8 {
            for mode in 0..4 {
                for flags in 0..4 {
                    let mut data = vec![mode; 24];
                    data[0] = count;
                    data[10..18].fill(flags);
                    interfaces(&data);
                }
            }
        }
    }
    #[test]
    fn method_oracle_covers_all_16_cases_and_varied_entry_bytes() {
        for case in 0..16 {
            signature(case);
        }
        for n in 0u8..64 {
            let data: Vec<u8> = (0..24).map(|at| n.wrapping_add(at * 9)).collect();
            model(&data);
        }
    }
}
