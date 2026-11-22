//! Independent small signature tables and declaration-presence oracles.

use super::*;

pub(super) fn model(data: &[u8]) {
    let byte = |at| data.get(at).copied().unwrap_or(0);
    signature(byte(0), byte(1), byte(2), byte(3));
    declarations(byte(4), byte(5), byte(6));
    stringifiers(byte(7), byte(8));
}

fn signature(operation: u8, key: u8, count: u8, modifier: u8) {
    let operation = usize::from(operation % 3);
    let keys = [
        "DOMString",
        "unsigned long",
        "USVString",
        "ByteString",
        "long",
        "unsigned long long",
        "DOMString?",
        "(DOMString or long)",
        "_DOMString",
    ];
    let key = usize::from(key) % keys.len();
    let count = usize::from(count % 4);
    let modifier = modifier % 3;
    let mut arguments = Vec::new();
    for n in 0..count {
        let ty = if n == 0 { "Key" } else { "long" };
        let optional = if modifier == 1 && n == 0 {
            "optional "
        } else {
            ""
        };
        let variadic = if modifier == 2 && n + 1 == count {
            "..."
        } else {
            ""
        };
        arguments.push(format!("{optional}{ty}{variadic} arg{n}"));
    }
    let early = format!(
        "interface Child : Base {{ {} any selected({}); }};",
        ["getter", "setter", "deleter"][operation],
        arguments.join(",")
    );
    let late = format!(
        "interface _DOMString {{}}; typedef [Opaque] {} Key; interface Base {{ readonly attribute unsigned long length; getter any named(DOMString name); getter any indexed(unsigned long index); }};",
        keys[key]
    );
    let expected = if count != [1, 2, 1][operation] || (count > 0 && modifier != 0) {
        Some(types::ErrorKind::SpecialOperationSignature)
    } else if key != 0 && !(key == 1 && operation != 2) {
        Some(types::ErrorKind::SpecialOperationKey)
    } else {
        None
    };
    check(&[&early, &late], expected);
}

fn declarations(mask: u8, duplicate: u8, length: u8) {
    let operations = [
        "getter any named(DOMString key)",
        "getter any indexed(unsigned long index)",
        "setter any put(DOMString key, long value)",
        "setter any set(unsigned long index, long value)",
        "deleter any remove(DOMString key)",
        "stringifier",
    ];
    let mut body = String::new();
    for (n, operation) in operations.iter().enumerate() {
        if mask & (1 << n) != 0 {
            body.push_str(operation);
            body.push(';');
        }
    }
    let duplicate = usize::from(duplicate % 7);
    if duplicate < operations.len() && mask & (1 << duplicate) != 0 {
        // Unique names reach the variety check instead of name collisions.
        let extra = [
            "getter any extra(DOMString key)",
            "getter any extra(unsigned long index)",
            "setter any extra(DOMString key,long value)",
            "setter any extra(unsigned long index,long value)",
            "deleter any extra(DOMString key)",
            "stringifier",
        ][duplicate];
        body.push_str(extra);
        body.push(';');
    }
    let length = length % 4;
    body.push_str(
        [
            "",
            "readonly attribute unsigned long length;",
            "readonly attribute double length;",
            "static readonly attribute unsigned long length;",
        ][usize::from(length)],
    );
    let source = format!("interface I {{ {body} }};");
    let named = mask & 1 != 0;
    let indexed = mask & 2 != 0;
    let missing =
        (mask & 4 != 0 && !named) || (mask & 8 != 0 && !indexed) || (mask & 16 != 0 && !named);
    let expected = if duplicate < operations.len() && mask & (1 << duplicate) != 0 {
        Some(if duplicate == 5 {
            types::ErrorKind::DuplicateStringifier
        } else {
            types::ErrorKind::DuplicateSpecialOperation
        })
    } else if missing {
        Some(types::ErrorKind::MissingSpecialGetter)
    } else if indexed && length != 1 {
        Some(types::ErrorKind::IndexedLength)
    } else {
        None
    };
    check(&[&source], expected);
}

fn stringifiers(ty: u8, flags: u8) {
    let types = [
        "DOMString",
        "USVString",
        "ByteString",
        "DOMString?",
        "long",
        "(DOMString or long)",
        "Text",
    ];
    let ty = usize::from(ty) % types.len();
    let own = flags & 1 != 0;
    let a = flags & 2 != 0;
    let b = flags & 4 != 0;
    let mut early = String::from("interface Child : Base {}; ");
    if own {
        early.push_str("partial interface Child { stringifier attribute Choice text; }; ");
    }
    if a {
        early.push_str("Child includes A; ");
    }
    if b {
        early.push_str("Child includes B; ");
    }
    let late = format!(
        "enum Text {{\"on\"}}; typedef [Opaque] {} Choice; interface Base {{ stringifier; }}; interface mixin A {{ stringifier; }}; interface mixin B {{ stringifier; }};",
        types[ty]
    );
    let expected = if own && ty >= 2 {
        Some(types::ErrorKind::StringifierType)
    } else if u8::from(own) + u8::from(a) + u8::from(b) > 1 {
        Some(types::ErrorKind::DuplicateStringifier)
    } else {
        None
    };
    check(&[&early, &late], expected);
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
    .expect("linked special-member model");
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
        let error = result.expect_err("independent special-member violation");
        assert_eq!(error.kind, expected);
        let at = error.location.expect("actual declaration or type location");
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
            &result.expect("independently valid special members"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_oracle_covers_all_324_key_arity_modifier_combinations() {
        for operation in 0..3 {
            for key in 0..9 {
                for count in 0..4 {
                    for modifier in 0..3 {
                        signature(operation, key, count, modifier);
                    }
                }
            }
        }
    }
    #[test]
    fn declaration_oracle_covers_all_masks_duplicates_and_length_forms() {
        for mask in 0..64 {
            for duplicate in 0..7 {
                for length in 0..4 {
                    declarations(mask, duplicate, length);
                }
            }
        }
    }
    #[test]
    fn stringifier_oracle_covers_type_aliases_partials_mixins_and_inheritance() {
        for ty in 0..7 {
            for flags in 0..8 {
                stringifiers(ty, flags);
            }
        }
        for n in 0u8..64 {
            let data: Vec<u8> = (0..20).map(|at| n.wrapping_add(at * 13)).collect();
            model(&data);
        }
    }
}
