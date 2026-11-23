//! Independent flag/target table, placement matrix, forms and alias-isolation oracles.
use super::*;
const FLAGS: [&str; 5] = [
    "Clamp",
    "EnforceRange",
    "LegacyNullToEmptyString",
    "AllowShared",
    "AllowResizable",
];

pub(super) fn model(data: &[u8]) {
    let byte = |n| data.get(n).copied().unwrap_or(0);
    target(byte(0), byte(1), byte(2));
    placement(byte(3), byte(4));
    aliases(byte(5), byte(6));
    form(byte(7), byte(8));
}
fn check(early: &str, late: &str, expected: Option<types::ErrorKind>) {
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
    .expect("linked type-attribute model");
    let members = members::build_within(
        &index,
        members::Limits {
            records: 1024,
            visits: 4096,
        },
        &budget,
    )
    .expect("attribute model members");
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
        let e = result.expect_err("independently invalid attribute");
        assert_eq!(e.kind, expected, "{early} / {late}");
        let at = e.location.expect("attribute diagnostic");
        assert!([early, late][at.source].get(at.span).is_some());
    } else {
        check_types(
            &index,
            &members,
            &result.expect("independently valid type attributes"),
        );
    }
}
fn target(row: u8, flag: u8, nullable: u8) {
    // Accepted columns C,E,N,S,R: Clamp, EnforceRange, NullToEmpty, Shared, Resizable.
    const ROWS: [(&str, &str); 16] = [
        ("long", "11000"),
        ("octet", "11000"),
        ("bigint", "00000"),
        ("boolean", "00000"),
        ("double", "00000"),
        ("DOMString", "00100"),
        ("USVString", "00100"),
        ("ByteString", "00000"),
        ("ArrayBuffer", "00001"),
        ("SharedArrayBuffer", "00001"),
        ("DataView", "00011"),
        ("Uint8Array", "00011"),
        ("BigInt64Array", "00011"),
        ("sequence<long>", "00000"),
        ("Count", "11000"),
        ("Float16Array", "00011"),
    ];
    let (ty, allowed) = ROWS[usize::from(row % 16)];
    let flag = usize::from(flag % 5);
    let nullable = nullable & 1 != 0;
    let early = format!(
        "interface I {{ undefined run([{}] {ty}{} value); }};",
        FLAGS[flag],
        if nullable { "?" } else { "" }
    );
    let valid = allowed.as_bytes()[flag] == b'1' && !(nullable && flag == 2);
    check(
        &early,
        "typedef [Tag] long Count;",
        (!valid).then_some(types::ErrorKind::TypeAttributeTarget),
    );
}
fn placement(row: u8, flag: u8) {
    let row = row % 16;
    let flag = usize::from(flag % 5);
    let attr = FLAGS[flag];
    let ty = match flag {
        0 | 1 => "long",
        2 => "DOMString",
        _ => "DataView",
    };
    let source = match row {
        0 => format!("interface I {{ undefined run([{attr}] {ty} value); }};"),
        1 => format!("interface I {{ undefined run([{attr}] {ty}... rest); }};"),
        2 => format!("interface I {{ undefined run(optional [{attr}] {ty} value); }};"),
        3 => format!("interface I {{ undefined run([{attr}] optional {ty} value); }};"),
        4 => format!("dictionary D {{ [{attr}] {ty} value; }};"),
        5 => format!("dictionary D {{ required [{attr}] {ty} value; }};"),
        6 => format!("dictionary D {{ [{attr}] required {ty} value; }};"),
        7 => format!("interface I {{ attribute [{attr}] {ty} value; }};"),
        8 => format!("interface I {{ readonly attribute [{attr}] {ty} value; }};"),
        9 => format!("interface I {{ [{attr}] attribute {ty} value; }};"),
        10 => format!("interface I {{ [{attr}] {ty} run(); }};"),
        11 => format!("[{attr}] interface I {{}};"),
        12 => format!("typedef [{attr}] {ty} Value;"),
        13 => format!("[{attr}] typedef {ty} Value;"),
        14 => format!("callback F=undefined ([{attr}] {ty} value);"),
        _ => format!(
            "interface mixin M {{ attribute [{attr}] {ty} value; }}; interface I {{}}; I includes M;"
        ),
    };
    let expected = if [3, 6, 9, 10, 11, 13].contains(&row) {
        Some(types::ErrorKind::TypeAttributePlacement)
    } else if row == 8 && flag < 2 {
        Some(types::ErrorKind::TypeAttributeReadonly)
    } else {
        None
    };
    check(&source, "", expected);
}
fn aliases(left: u8, right: u8) {
    let flags = |bits: u8| match bits % 4 {
        0 => "",
        1 => "[Clamp]",
        2 => "[EnforceRange]",
        _ => "[Clamp,EnforceRange]",
    };
    let early = format!(
        "interface I {{ undefined first({} Count value); undefined second({} Count value); readonly attribute Count plain; }};",
        flags(left),
        flags(right)
    );
    let invalid = left % 4 == 3 || right % 4 == 3;
    check(
        &early,
        "typedef [Opaque] long Count;",
        invalid.then_some(types::ErrorKind::TypeAttributeConflict),
    );
}
fn form(flag: u8, shape: u8) {
    let flag = usize::from(flag % 5);
    let attr = FLAGS[flag];
    let ty = match flag {
        0 | 1 => "long",
        2 => "DOMString",
        _ => "DataView",
    };
    let attr = match shape % 5 {
        0 => attr.to_owned(),
        1 => format!("{attr}=No"),
        2 => format!("{attr}()"),
        3 => format!("{attr}=(A,B)"),
        _ => format!("Opaque=({attr})"),
    };
    let source = format!("typedef [{attr}] {ty} Value;");
    check(
        &source,
        "",
        matches!(shape % 5, 1..=3).then_some(types::ErrorKind::TypeAttributeForm),
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_covers_all_160_target_flag_nullable_combinations() {
        for row in 0..16 {
            for flag in 0..5 {
                for nullable in 0..2 {
                    target(row, flag, nullable);
                }
            }
        }
    }
    #[test]
    fn matrix_covers_all_80_placement_flag_combinations() {
        for row in 0..16 {
            for flag in 0..5 {
                placement(row, flag);
            }
        }
    }
    #[test]
    fn alias_oracle_covers_all_16_independent_mask_pairs_without_leaking() {
        for left in 0..4 {
            for right in 0..4 {
                aliases(left, right);
            }
        }
    }
    #[test]
    fn forms_and_mixed_inputs_cover_named_flags_and_opaque_contents() {
        for flag in 0..5 {
            for shape in 0..5 {
                form(flag, shape);
            }
        }
        for n in 0u8..64 {
            let data: Vec<u8> = (0..9).map(|at| n.wrapping_add(at * 7)).collect();
            model(&data);
        }
    }
}
