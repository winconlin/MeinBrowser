//! Independent scalar-bound, default-table and inherited-required oracles.

use super::*;

pub(super) fn model(data: &[u8]) {
    let byte = |at| data.get(at).copied().unwrap_or(0);
    integer(byte(0), byte(1), byte(2));
    default(byte(3), byte(4));
    dictionary(byte(5), byte(6));
}

fn integer(control: u8, choice: u8, spelling: u8) {
    let types = [
        "byte",
        "octet",
        "short",
        "unsigned short",
        "long",
        "unsigned long",
        "long long",
        "unsigned long long",
    ];
    let at = usize::from(control % 8);
    let signed = at % 2 == 0;
    let bits = 8 << (at / 2);
    // i128 is wider than every WebIDL fixed-width integer. This oracle does
    // not use u64 parsing or floating point like the code under test.
    let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
    let max = (1i128 << (bits - i32::from(signed))) - 1;
    let n = [
        min - 1,
        min,
        min + 1,
        0,
        max - 1,
        max,
        max + 1,
        i128::from(choice) - 128,
    ][usize::from(choice % 8)];
    let magnitude = n.unsigned_abs();
    let sign = if n < 0 { "-" } else { "" };
    let raw = match spelling % 3 {
        0 => n.to_string(),
        1 => format!("{sign}0x{magnitude:x}"),
        _ => format!("{sign}0{magnitude:o}"),
    };
    let early = format!("typedef [Origin] {} Scalar;", types[at]);
    let late = format!("interface I {{ const Scalar n = {raw}; }};");
    check(
        &[&early, &late],
        (!(min..=max).contains(&n)).then_some(types::ErrorKind::ConstantValue),
    );
}

fn default(row: u8, column: u8) {
    let types = [
        "boolean",
        "octet",
        "long long",
        "double",
        "unrestricted float",
        "DOMString",
        "ByteString",
        "Mode",
        "long?",
        "sequence<long>",
        "FrozenArray<long>",
        "Settings",
        "record<DOMString,long>",
        "any",
        "(boolean or sequence<long>)",
        "(Mode or long)?",
    ];
    let literals = [
        "false",
        "0",
        "256",
        "-1",
        "1.5",
        "Infinity",
        "\"ready\"",
        "\"other\"",
        "\"ÿ\"",
        "\"😀\"",
        "null",
        "undefined",
        "[]",
        "{}",
    ];
    // Explicit accepted columns per row; no type-graph traversal or numeric
    // parser is shared with the implementation.
    const ALLOWED: [&[usize]; 16] = [
        &[0],
        &[1],
        &[1, 2, 3],
        &[1, 2, 3, 4],
        &[1, 2, 3, 4, 5],
        &[6, 7, 8, 9],
        &[6, 7, 8],
        &[6, 8],
        &[1, 2, 3, 10],
        &[12],
        &[],
        &[13],
        &[],
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        &[0, 12],
        &[1, 2, 3, 6, 8, 10],
    ];
    let row = usize::from(row) % types.len();
    let column = usize::from(column) % literals.len();
    let early = format!(
        "enum Mode {{\"ready\",\"ÿ\"}}; dictionary Settings {{}}; typedef [Origin] {} Choice;",
        types[row]
    );
    let late = format!(
        "dictionary Holder {{ Choice value = {}; }};",
        literals[column]
    );
    check(
        &[&early, &late],
        (!ALLOWED[row].contains(&column)).then_some(types::ErrorKind::DefaultValue),
    );
}

fn dictionary(flags: u8, variant: u8) {
    let ancestor_required = flags & 1 != 0;
    let child_required = flags & 2 != 0;
    let early = format!(
        "partial dictionary Base {{ {} }}; dictionary Child : Base {{ {} }};",
        if ancestor_required {
            "required long root;"
        } else {
            "long root;"
        },
        if child_required {
            "required long own;"
        } else {
            "long own;"
        }
    );
    let arguments = [
        "Child options",
        "optional Child options",
        "optional Child options = {}",
        "Child options, long count",
        "Child options, optional long count = 0",
        "optional (Child or DOMString) options = \"x\"",
    ];
    let case = usize::from(variant) % arguments.len();
    let late = format!(
        "dictionary Base {{}}; interface I {{ undefined f({}); }};",
        arguments[case]
    );
    let missing_default = !ancestor_required && !child_required && matches!(case, 0 | 1 | 4);
    check(
        &[&early, &late],
        missing_default.then_some(types::ErrorKind::DictionaryArgumentDefault),
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
    .expect("bounded linked value model");
    let members = members::build_within(
        &index,
        members::Limits {
            records: 1024,
            visits: 4096,
        },
        &budget,
    )
    .expect("valid model member forms");
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
        },
        &budget,
    );
    if let Some(expected) = expected {
        let error = result.expect_err("independently invalid literal or dictionary argument");
        assert_eq!(error.kind, expected);
        let at = error.location.expect("actual source location");
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
            &result.expect("independently valid value model"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wider_integer_oracle_covers_all_widths_bounds_and_bases() {
        for control in 0..8 {
            for choice in 0..8 {
                for spelling in 0..3 {
                    integer(control, choice, spelling);
                }
            }
        }
    }
    #[test]
    fn default_oracle_covers_all_224_type_literal_combinations() {
        for row in 0..16 {
            for column in 0..14 {
                default(row, column);
            }
        }
    }
    #[test]
    fn required_dictionary_oracle_covers_partials_ancestors_and_optional_suffixes() {
        for flags in 0..4 {
            for variant in 0..6 {
                dictionary(flags, variant);
            }
        }
        for n in 0u8..64 {
            let data: Vec<u8> = (0..20).map(|at| n.wrapping_add(at * 13)).collect();
            model(&data);
        }
    }
}
