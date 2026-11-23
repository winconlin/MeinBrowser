//! Independent use-site matrix and preselected ObservableArray element classes.

use super::*;

pub(super) fn model(data: &[u8]) {
    let byte = |at| data.get(at).copied().unwrap_or(0);
    site(byte(0), byte(1), byte(2));
    element(byte(3), byte(4));
}

fn check(early: &str, late: &str, expected: Option<types::ErrorKind>) {
    let budget = Budget::new(75_000);
    let index = index::build_within(
        &[early, late],
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
    .expect("linked array model");
    let members = members::build_within(
        &index,
        members::Limits {
            records: 1024,
            visits: 4096,
        },
        &budget,
    )
    .expect("array model members");
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
    match expected {
        Some(expected) => {
            let e = result.expect_err("independent array rule");
            assert_eq!(e.kind, expected, "{early} / {late}");
            let at = e.location.expect("array use position");
            assert!([early, late][at.source].get(at.span).is_some());
        }
        None => check_types(
            &index,
            &members,
            &result.expect("independently permitted array use"),
        ),
    }
}

fn site(row: u8, array: u8, wrapper: u8) {
    // Permission rows chosen before rendering IDL: F=FrozenArray, O=ObservableArray.
    const PERMITTED: [(bool, bool); 16] = [
        (true, true),
        (true, false),
        (false, false),
        (true, true),
        (true, true),
        (false, false),
        (false, false),
        (false, false),
        (false, false),
        (false, false),
        (false, false),
        (false, false),
        (false, false),
        (false, false),
        (false, false),
        (true, true),
    ];
    let row = usize::from(row % 16);
    let observable = array & 1 != 0;
    let array = if observable {
        "ObservableArray"
    } else {
        "FrozenArray"
    };
    let ty = match wrapper % 3 {
        0 => "Values",
        1 => "Values?",
        _ => "(Values or DOMString)",
    };
    let early = match row {
        0 => format!("interface I {{ attribute {ty} values; }};"),
        1 => format!("interface I {{ static readonly attribute {ty} values; }};"),
        2 => format!("namespace I {{ readonly attribute {ty} values; }};"),
        3 => format!(
            "interface mixin M {{ attribute {ty} values; }}; interface I {{}}; I includes M;"
        ),
        4 => {
            format!("partial interface I {{ readonly attribute {ty} values; }}; interface I {{}};")
        }
        5 => format!("interface I {{ {ty} run(); }};"),
        6 => format!("interface I {{ static {ty} run(); }};"),
        7 => format!("interface I {{ undefined run({ty} value); }};"),
        8 => format!("interface I {{ constructor({ty} value); }};"),
        9 => format!("callback F = {ty} ();"),
        10 => format!("callback F = undefined ({ty} value);"),
        11 => format!("dictionary D {{ {ty} field; }};"),
        12 => format!("interface I {{ setlike<{ty}>; }};"),
        13 => format!("interface I {{ async_iterable<long>(optional {ty} value); }};"),
        14 => format!("typedef sequence<{ty}> Nested;"),
        _ => format!("interface mixin M {{ readonly attribute {ty} values; }};"),
    };
    let late = format!("typedef [Tag] {array}<long> Values;");
    let expected = if observable && wrapper % 3 == 1 {
        Some(types::ErrorKind::NullableType)
    } else if observable && wrapper % 3 == 2 {
        Some(types::ErrorKind::IndistinguishableUnion)
    } else if if observable {
        PERMITTED[row].1
    } else {
        PERMITTED[row].0
    } {
        None
    } else {
        Some(if observable {
            types::ErrorKind::ObservableArrayUse
        } else {
            types::ErrorKind::FrozenArrayUse
        })
    };
    check(&early, &late, expected);
}

fn element(row: u8, nullable: u8) {
    // Kinds chosen independently from the implementation's facts/graph walk.
    // code 0: permitted; 1: prohibited parameter class; 2: nested FrozenArray use.
    const ROWS: [(&str, u8); 16] = [
        ("long", 0),
        ("DOMString", 0),
        ("I", 0),
        ("C", 0),
        ("F", 0),
        ("any", 0),
        ("Promise<long>", 0),
        ("async_sequence<long>", 0),
        ("D", 1),
        ("sequence<long>", 1),
        ("record<DOMString,long>", 1),
        ("ObservableArray<long>", 1),
        ("FrozenArray<long>", 2),
        ("(D or DOMString)", 0),
        ("(sequence<long> or DOMString)", 0),
        ("bigint", 0),
    ];
    let (ty, class) = ROWS[usize::from(row % 16)];
    let nullable = nullable & 1 != 0;
    let early = format!(
        "interface Host {{ attribute ObservableArray<Item{}> values; }};",
        if nullable { "?" } else { "" }
    );
    let late = format!(
        "dictionary D {{}}; interface I {{}}; callback interface C {{ undefined call(); }}; callback F = undefined (); typedef [Tag] {ty} Item;"
    );
    let expected = if nullable && matches!(row % 16, 5 | 6 | 11 | 13) {
        Some(types::ErrorKind::NullableType)
    } else if class == 1 {
        Some(types::ErrorKind::ObservableArrayElement)
    } else if class == 2 {
        Some(types::ErrorKind::FrozenArrayUse)
    } else {
        None
    };
    check(&early, &late, expected);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn use_site_matrix_covers_all_96_context_kind_wrapper_cases() {
        for row in 0..16 {
            for kind in 0..2 {
                for wrapper in 0..3 {
                    site(row, kind, wrapper);
                }
            }
        }
    }
    #[test]
    fn element_table_covers_32_parameter_class_and_nullability_cases() {
        for row in 0..16 {
            for nullable in 0..2 {
                element(row, nullable);
            }
        }
    }
    #[test]
    fn mixed_input_bytes_reach_both_array_oracles() {
        for n in 0u8..64 {
            model(&[
                n,
                n.wrapping_mul(7),
                n.wrapping_add(1),
                n.wrapping_add(3),
                n >> 1,
            ]);
        }
    }
}
