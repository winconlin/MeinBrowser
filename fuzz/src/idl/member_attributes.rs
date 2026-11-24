//! Independent member-flag target/context/conflict tables and form oracle.
use super::*;
const FLAGS: [&str; 4] = [
    "SameObject",
    "NewObject",
    "Replaceable",
    "LegacyLenientSetter",
];
pub(super) fn model(data: &[u8]) {
    let b = |n| data.get(n).copied().unwrap_or(0);
    target(b(0), b(1));
    placement(b(2), b(3));
    conflicts(b(4), b(5));
    form(b(6), b(7));
}
fn check(early: &str, expected: Option<types::ErrorKind>) {
    let late = "interface Value { attribute DOMString text; }; typedef [Tag] Value Alias; typedef Promise<long> Result; callback interface C { undefined run(); }; callback F=undefined ();";
    let budget = Budget::new(100_000);
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
    .expect("member-attribute model names");
    let members = members::build_within(
        &index,
        members::Limits {
            records: 1024,
            visits: 4096,
        },
        &budget,
    )
    .expect("member-attribute model members");
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
    if let Some(kind) = expected {
        let e = result.expect_err("independently invalid member attribute");
        assert_eq!(e.kind, kind, "{early}");
        let at = e.location.expect("diagnostic");
        assert!([early, late][at.source].get(at.span).is_some());
    } else {
        check_types(
            &index,
            &members,
            &result.expect("independent member-attribute rules"),
        );
    }
}
fn target(row: u8, flag: u8) {
    const TYPES: [&str; 16] = [
        "Value",
        "object",
        "Promise<long>",
        "Value?",
        "object?",
        "long",
        "DOMString",
        "C",
        "F",
        "ArrayBuffer",
        "FrozenArray<long>",
        "Alias",
        "Result",
        "Promise<undefined>",
        "(Value or DOMString)",
        "Uint8Array",
    ];
    let row = usize::from(row % 16);
    let flag = usize::from(flag % 4);
    let attr = FLAGS[flag];
    let ty = TYPES[row];
    let member = if flag == 1 {
        format!("[{attr}] {ty} run();")
    } else {
        format!("[{attr}] readonly attribute {ty} value;")
    };
    let expected = if flag == 1 && row == 10 {
        Some(types::ErrorKind::FrozenArrayUse)
    } else if flag != 1 && [2, 12, 13].contains(&row) {
        Some(types::ErrorKind::MemberAttributePromise)
    } else if flag == 0 && ![0, 1, 11].contains(&row)
        || flag == 1 && ![0, 2, 11, 12, 13].contains(&row)
    {
        Some(types::ErrorKind::MemberAttributeType)
    } else {
        None
    };
    check(&format!("interface I {{ {member} }};"), expected);
}
fn placement(row: u8, flag: u8) {
    let row = row % 12;
    let flag = usize::from(flag % 4);
    let attr = FLAGS[flag];
    let normal = if flag == 1 {
        format!("[{attr}] Value run();")
    } else {
        format!("[{attr}] readonly attribute object value;")
    };
    let source = match row {
        0 => format!("interface I {{ {normal} }};"),
        1 => {
            if flag == 1 {
                format!("interface I {{ [{attr}] static Value run(); }};")
            } else {
                format!("interface I {{ [{attr}] static readonly attribute object value; }};")
            }
        }
        2 => format!("namespace N {{ {normal} }};"),
        3 => format!("interface mixin M {{ {normal} }}; interface I {{}}; I includes M;"),
        4 => format!("partial interface I {{ {normal} }}; interface I {{}};"),
        5 => format!("[{attr}] interface I {{}};"),
        6 => format!("typedef [{attr}] object Bad;"),
        7 => format!("interface I {{ undefined run([{attr}] object arg); }};"),
        8 => format!("[{attr}] callback G=undefined ();"),
        9 => {
            if flag == 1 {
                format!("interface I {{ [{attr}] readonly attribute Value value; }};")
            } else {
                format!("interface I {{ [{attr}] object run(); }};")
            }
        }
        10 => {
            if flag == 1 {
                format!("interface I {{ {normal} }};")
            } else {
                format!("interface I {{ [{attr}] attribute object value; }};")
            }
        }
        _ => format!(
            "interface I {{ [{}=No] readonly attribute object value; }};",
            FLAGS[flag]
        ),
    };
    let expected = if row == 11 {
        Some(types::ErrorKind::MemberAttributeForm)
    } else if matches!(row, 5..=9) || (matches!(row, 1 | 2) && flag >= 2) {
        Some(types::ErrorKind::MemberAttributePlacement)
    } else if row == 10 && flag != 1 {
        Some(types::ErrorKind::MemberAttributeReadonly)
    } else {
        None
    };
    check(&source, expected);
}
fn conflicts(row: u8, promise: u8) {
    const FLAGS: [&str; 9] = [
        "Replaceable",
        "LegacyLenientSetter",
        "PutForwards=text",
        "Replaceable,LegacyLenientSetter",
        "Replaceable,PutForwards=text",
        "PutForwards=text,LegacyLenientSetter",
        "SameObject,Replaceable",
        "SameObject,LegacyLenientSetter",
        "SameObject,PutForwards=text",
    ];
    let row = usize::from(row % 9);
    let promise = promise & 1 != 0;
    let ty = if promise { "Promise<Value>" } else { "Value" };
    let expected = if (3..=5).contains(&row) {
        Some(types::ErrorKind::MemberAttributeConflict)
    } else if promise {
        Some(types::ErrorKind::MemberAttributePromise)
    } else {
        None
    };
    check(
        &format!(
            "interface I {{ [{}] readonly attribute {ty} value; }};",
            FLAGS[row]
        ),
        expected,
    );
}
fn form(flag: u8, shape: u8) {
    let flag = usize::from(flag % 4);
    let attr = FLAGS[flag];
    let raw = match shape % 6 {
        0 => attr.to_owned(),
        1 => format!("{attr}=No"),
        2 => format!("{attr}()"),
        3 => format!("{attr}=(A,B)"),
        4 => format!("{attr},{attr}"),
        _ => format!("Opaque=({attr})"),
    };
    let member = if flag == 1 {
        format!("[{raw}] Value run();")
    } else {
        format!("[{raw}] readonly attribute object value;")
    };
    check(
        &format!("interface I {{ {member} }};"),
        matches!(shape % 6, 1..=3).then_some(types::ErrorKind::MemberAttributeForm),
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_table_covers_all_64_type_flag_pairs() {
        for row in 0..16 {
            for flag in 0..4 {
                target(row, flag);
            }
        }
    }
    #[test]
    fn placement_table_covers_all_48_context_flag_pairs() {
        for row in 0..12 {
            for flag in 0..4 {
                placement(row, flag);
            }
        }
    }
    #[test]
    fn conflict_table_covers_all_18_flag_promise_pairs() {
        for row in 0..9 {
            for promise in 0..2 {
                conflicts(row, promise);
            }
        }
    }
    #[test]
    fn forms_and_mixed_inputs_cover_24_forms_and_64_entry_sequences() {
        for flag in 0..4 {
            for shape in 0..6 {
                form(flag, shape);
            }
        }
        for n in 0u8..64 {
            let data: Vec<_> = (0..8).map(|at| n.wrapping_add(at * 7)).collect();
            model(&data);
        }
    }
}
