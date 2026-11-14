//! Source partitioning, progress and token-isolation checks for WebIDL.

use mb_webidl::lexer::{Kind, Lexer};
use mb_webidl::parser::{self, Budget, Document, ErrorKind, Kind as NodeKind, Limits};
use mb_webidl::{index, members};

pub fn webidl(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let source = String::from_utf8_lossy(data);
    tokens(&source);
    syntax(&source);
    names(&[&source]);
    name_model(data);
    member_model(data);
}

fn tokens(source: &str) {
    let mut lexer = Lexer::new(source);
    let mut position = 0;
    for _ in 0..=source.chars().count() {
        let token = lexer.next_token();
        assert_eq!(token.span.start, position, "complete source partition");
        assert_eq!(
            source.get(token.span.clone()),
            Some(token.text),
            "UTF-8 boundaries and exact text"
        );
        if token.kind == Kind::End {
            assert_eq!(position, source.len(), "all source consumed");
            assert!(token.text.is_empty());
            assert_eq!(lexer.next_token(), token, "stable EOF");
            assert!(lexer.next().is_none());
            return;
        }
        assert!(token.span.end > position, "token makes progress");
        position = token.span.end;
        if token.kind == Kind::Other {
            assert_eq!(token.text.chars().count(), 1, "one other scalar");
        }
        let mut isolated = Lexer::new(token.text);
        let again = isolated.next_token();
        assert_eq!(again.kind, token.kind, "isolated token kind");
        assert_eq!(
            again.span,
            0..token.text.len(),
            "isolated token consumes its text"
        );
        assert_eq!(again.text, token.text);
        assert_eq!(isolated.next_token().kind, Kind::End);
    }
    panic!("more tokens than source scalars plus EOF");
}

fn syntax(source: &str) {
    let limits = Limits {
        source_bytes: 3 * crate::MAX_INPUT,
        nodes: 8192,
        pending: 8192,
    };
    let budget = Budget::new(50_000);
    match parser::parse_within(source, limits, &budget) {
        Ok(document) => {
            assert_eq!(document.source(), source);
            assert!(document.len() <= limits.nodes);
            assert!(budget.exhausted().is_none());
            tree(&document);
        }
        Err(error) => {
            assert!(source.get(error.span).is_some(), "valid error range");
            if let ErrorKind::Exhausted(reason) = error.kind {
                assert_eq!(budget.exhausted(), Some(reason), "observed host exhaustion");
            }
        }
    }
}

fn tree(document: &Document<'_>) {
    let mut pending = vec![document.root()];
    let mut seen = std::collections::BTreeSet::new();
    let mut tokens = Lexer::new(document.source()).filter(|token| !token.kind.is_trivia());
    assert_eq!(
        document.node(document.root()).expect("root").span,
        0..document.source().len()
    );
    while let Some(id) = pending.pop() {
        assert!(seen.insert(id), "no duplicate or cyclic tree edges");
        let node = document.node(id).expect("valid node id");
        assert!(document.source().get(node.span.clone()).is_some());
        match node.kind {
            NodeKind::Token(kind) => {
                let token = tokens.next().expect("significant token");
                assert_eq!(kind, token.kind);
                assert_eq!(node.span, token.span);
                assert_eq!(document.text(id), Some(token.text));
                assert_eq!(document.children(id).count(), 0);
            }
            NodeKind::Production(_) => {
                for child in document.children(id).rev() {
                    assert!(child > id, "forward arena edges");
                    let child_node = document.node(child).expect("valid child");
                    assert!(
                        node.span.start <= child_node.span.start
                            && child_node.span.end <= node.span.end
                    );
                    pending.push(child);
                }
            }
        }
    }
    assert_eq!(seen.len(), document.len(), "all nodes reachable");
    assert!(tokens.next().is_none(), "every token appears exactly once");
}

// Auch syntaktisch gültige Eingaben dürfen an Namen/Links oder Hostgrenzen
// scheitern. Erfolgreiche Indizes besitzen ausschließlich lokale, gültige IDs.
fn names(sources: &[&str]) {
    let limits = index::Limits {
        sources: 2,
        source_bytes: 3 * crate::MAX_INPUT,
        nodes: 8192,
        records: 1024,
        parser: Limits {
            source_bytes: 3 * crate::MAX_INPUT,
            nodes: 8192,
            pending: 8192,
        },
    };
    let budget = Budget::new(75_000);
    match index::build_within(sources, limits, &budget) {
        Ok(index) => {
            check_index(&index);
            match members::build_within(
                &index,
                members::Limits {
                    records: 1024,
                    visits: 4096,
                },
                &budget,
            ) {
                Ok(model) => check_members(&index, &model),
                Err(error) => {
                    for location in error.location.iter().chain(error.previous.iter()) {
                        assert!(
                            sources
                                .get(location.source)
                                .and_then(|s| s.get(location.span.clone()))
                                .is_some()
                        );
                    }
                    if let members::ErrorKind::Exhausted(reason) = error.kind {
                        assert_eq!(budget.exhausted(), Some(reason));
                    }
                }
            }
        }
        Err(error) => {
            for location in error.location.iter().chain(error.previous.iter()) {
                assert!(
                    sources
                        .get(location.source)
                        .and_then(|s| s.get(location.span.clone()))
                        .is_some()
                );
            }
            if let index::ErrorKind::Exhausted(reason) = error.kind {
                assert_eq!(budget.exhausted(), Some(reason));
            }
        }
    }
}

fn check_index(index: &index::Index<'_>) {
    let mut fragments = std::collections::BTreeSet::new();
    for (id, definition) in index.definitions() {
        let lookup = if definition.kind == index::DefinitionKind::Mixin {
            index.find_mixin(definition.name)
        } else {
            index.find(definition.name)
        };
        assert_eq!(lookup, Some(id));
        for &fragment_id in &definition.fragments {
            assert!(fragments.insert(fragment_id));
            let fragment = index.fragment(fragment_id).expect("local fragment");
            assert_eq!(fragment.definition, id);
            let document = index
                .document(fragment.location.source)
                .expect("local source");
            assert!(document.node(fragment.body).is_some());
            assert!(document.node(fragment.attributes).is_some());
            assert!(
                document
                    .source()
                    .get(fragment.location.span.clone())
                    .is_some()
            );
            assert_eq!(
                document
                    .node(fragment.location.node.expect("name node"))
                    .expect("local name node")
                    .span,
                fragment.location.span
            );
        }
        let mut parent = definition.parent;
        for _ in 0..index.definitions().len() {
            let Some(id) = parent else {
                break;
            };
            let inherited = index.definition(id).expect("local parent");
            assert_eq!(definition.kind, inherited.kind);
            parent = inherited.parent;
        }
        assert!(parent.is_none(), "acyclic inheritance");
    }
    for reference in index.references() {
        let target = index.definition(reference.target).expect("resolved type");
        assert_eq!(target.name, reference.name);
        assert!(!matches!(
            target.kind,
            index::DefinitionKind::Mixin | index::DefinitionKind::Namespace
        ));
        let fragment = index
            .fragment(reference.fragment)
            .expect("local owning fragment");
        assert_eq!(fragment.location.source, reference.location.source);
        let document = index.document(reference.location.source).unwrap();
        let text = document
            .source()
            .get(reference.location.span.clone())
            .expect("type span");
        assert_eq!(text.strip_prefix('_').unwrap_or(text), reference.name);
    }
    for (id, definition) in index.definitions() {
        let includes: Vec<_> = index
            .includes()
            .iter()
            .filter(|include| include.interface == id)
            .map(|include| include.mixin)
            .collect();
        assert_eq!(definition.mixins, includes);
    }
    for include in index.includes() {
        assert_eq!(
            index.definition(include.interface).expect("host").kind,
            index::DefinitionKind::Interface
        );
        assert_eq!(
            index.definition(include.mixin).expect("mixin").kind,
            index::DefinitionKind::Mixin
        );
    }
}

// Ein unabhängiges, kleines Graphmodell sorgt für gültige Dateien, Partials
// vor der Grunddefinition, Vorwärtsreferenzen und Includes in beiden Dateien.
fn name_model(data: &[u8]) {
    let count = usize::from(data.first().copied().unwrap_or(0) % 16) + 1;
    let byte = |at: usize| data.get(at).copied().unwrap_or(0);
    let mut early = String::from("interface mixin _M0 {}; interface mixin M1 {}; ");
    let mut late = String::new();
    for n in 0..count {
        let target = (n + 1) % count;
        early.push_str(&format!(
            "partial interface _I{n} {{ attribute I{target} value; }}; "
        ));
        if byte(n + 1) & 1 == 0 {
            early.push_str(&format!("I{n} includes M0; "));
        }
        late.push_str(&format!("interface I{n}"));
        if n != 0 {
            late.push_str(&format!(" : _I{}", usize::from(byte(n + 2)) % n));
        }
        late.push_str(" {}; ");
        late.push_str(&format!("_I{n} includes _M1; "));
    }
    let budget = Budget::new(75_000);
    let index = index::build_within(
        &[&early, &late],
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
    .expect("bounded valid model");
    check_index(&index);
    assert_eq!(index.definitions().len(), count + 2);
    assert_eq!(index.references().len(), count);
    for n in 0..count {
        let id = index.find(&format!("I{n}")).unwrap();
        let definition = index.definition(id).unwrap();
        assert_eq!(definition.fragments.len(), 2);
        assert!(index.fragment(definition.fragments[0]).unwrap().partial);
        let expected_parent = if n == 0 {
            None
        } else {
            index.find(&format!("I{}", usize::from(byte(n + 2)) % n))
        };
        assert_eq!(definition.parent, expected_parent);
        let mut expected_mixins = Vec::new();
        if byte(n + 1) & 1 == 0 {
            expected_mixins.push(index.find_mixin("M0").unwrap());
        }
        expected_mixins.push(index.find_mixin("M1").unwrap());
        assert_eq!(definition.mixins, expected_mixins);
        let reference = &index.references()[n];
        assert_eq!(
            reference.target,
            index.find(&format!("I{}", (n + 1) % count)).unwrap()
        );
        assert_eq!(index.fragment(reference.fragment).unwrap().definition, id);
    }
    // Jeder Knoten des erzeugten Vererbungsbaums führt zu I0. Eine neue
    // Kante von I0 zum letzten Knoten muss deshalb immer einen Zyklus bilden.
    let cyclic = late.replacen(
        "interface I0 {};",
        &format!("interface I0 : I{} {{}};", count - 1),
        1,
    );
    let error = index::build_within(
        &[&early, &cyclic],
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
        &Budget::new(75_000),
    )
    .expect_err("one added edge forms a cycle");
    assert_eq!(error.kind, index::ErrorKind::InheritanceCycle);
}

fn check_members(index: &index::Index<'_>, model: &members::Members<'_>) {
    let mut member_count = 0;
    let mut argument_count = 0;
    let mut value_count = 0;
    for (id, definition) in index.definitions() {
        let args = |signature: &members::Signature| {
            let args = model.arguments(signature).expect("local argument range");
            let mut seen = std::collections::BTreeSet::new();
            for (n, arg) in args.iter().enumerate() {
                assert!(seen.insert(arg.identifier.name));
                let document = index
                    .document(arg.identifier.location.source)
                    .expect("argument source");
                assert!(document.node(arg.ty).is_some());
                assert_eq!(
                    document.node(arg.attributes).unwrap().kind,
                    NodeKind::Production("ExtendedAttributeList")
                );
                let raw = document
                    .source()
                    .get(arg.identifier.location.span.clone())
                    .expect("argument name");
                assert_eq!(raw.strip_prefix('_').unwrap_or(raw), arg.identifier.name);
                if arg.variadic {
                    assert_eq!(n + 1, args.len());
                    assert!(arg.optional);
                    assert!(arg.default.is_none());
                }
                if let Some(default) = arg.default {
                    assert_eq!(
                        document.node(default).unwrap().kind,
                        NodeKind::Production("DefaultValue")
                    );
                }
            }
            args.len()
        };
        for member in model.members(id).expect("local member range") {
            member_count += 1;
            assert!(definition.fragments.contains(&member.fragment));
            let document = index
                .document(member.location.source)
                .expect("member source");
            assert!(
                document
                    .source()
                    .get(member.location.span.clone())
                    .is_some()
            );
            assert_eq!(
                document.node(member.attributes).unwrap().kind,
                NodeKind::Production("ExtendedAttributeList")
            );
            for ty in member.types.iter().flatten() {
                assert!(matches!(
                    document.node(*ty).unwrap().kind,
                    NodeKind::Production("Type" | "TypeWithExtendedAttributes" | "ConstType")
                ));
            }
            if let Some(identifier) = &member.identifier {
                let raw = document
                    .source()
                    .get(identifier.location.span.clone())
                    .expect("member identifier");
                assert_eq!(raw.strip_prefix('_').unwrap_or(raw), identifier.name);
            }
            if let Some(signature) = &member.signature {
                argument_count += args(signature);
            }
        }
        if let Some(callback) = model.callback(id) {
            argument_count += args(callback);
        }
        let mut values = std::collections::BTreeSet::new();
        for value in model.values(id).expect("local enum range") {
            value_count += 1;
            assert!(values.insert(value.value));
            let document = index.document(value.location.source).unwrap();
            let raw = document
                .source()
                .get(value.location.span.clone())
                .expect("enum span");
            assert_eq!(&raw[1..raw.len() - 1], value.value);
        }
    }
    assert_eq!(member_count, model.len());
    assert_eq!(argument_count, model.argument_count());
    assert_eq!(value_count, model.value_count());
}

// Eigenes Orakel für Member über zwei Quellen. Jede Variante fügt genau
// einen bekannten Fehler in einen sonst gültigen kleinen Entwurf ein.
fn member_model(data: &[u8]) {
    let count = usize::from(data.first().copied().unwrap_or(0) % 7) + 2;
    let mode = data.get(1).copied().unwrap_or(0) % 8;
    let mut early = String::from("dictionary D0 { long root; }; ");
    for n in 1..count {
        early.push_str(&format!(
            "dictionary D{n} : D{} {{ long field{n}; }}; ",
            n - 1
        ));
    }
    early.push_str("enum Mode { \"on\", \"off\" }; interface mixin A { readonly attribute long a; }; partial interface Host { undefined extra(long value); }; Host includes A;");
    let mut late = String::from(
        "interface Host { undefined run(optional long count = 0, DOMString... rest); }; interface mixin B { attribute long b; }; Host includes B;",
    );
    let expected = match mode {
        1 => {
            early = early.replace(&format!("long field{};", count - 1), "long root;");
            Some(members::ErrorKind::DuplicateDictionaryMember)
        }
        2 => {
            late = late.replace("DOMString... rest", "DOMString... _count");
            Some(members::ErrorKind::DuplicateArgument)
        }
        3 => {
            early = early.replace("\"off\"", "\"on\"");
            Some(members::ErrorKind::DuplicateEnumValue)
        }
        4 => {
            early = early.replace("undefined extra", "undefined run");
            Some(members::ErrorKind::CrossFragmentOverload)
        }
        5 => {
            late = late.replace("attribute long b", "attribute long a");
            Some(members::ErrorKind::DuplicateMember)
        }
        6 => {
            late = late.replace("attribute long b", "attribute long _constructor");
            Some(members::ErrorKind::ReservedIdentifier)
        }
        7 => {
            late = late.replace(
                "optional long count = 0, DOMString... rest",
                "long... rest, long count",
            );
            Some(members::ErrorKind::VariadicNotLast)
        }
        _ => None,
    };
    let budget = Budget::new(100_000);
    let index = index::build_within(
        &[&early, &late],
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
    .expect("linked member model");
    let result = members::build_within(
        &index,
        members::Limits {
            records: 1024,
            visits: 4096,
        },
        &budget,
    );
    match expected {
        Some(kind) => assert_eq!(
            result.expect_err("one known invalid declaration").kind,
            kind
        ),
        None => {
            let model = result.expect("valid member model");
            check_members(&index, &model);
            assert_eq!(model.len(), count + 4);
            assert_eq!(model.argument_count(), 3);
            assert_eq!(model.value_count(), 2);
            let host = model.members(index.find("Host").unwrap()).unwrap();
            assert_eq!(host.len(), 2);
            assert_eq!(host[0].identifier.as_ref().unwrap().name, "extra");
            assert_eq!(host[1].identifier.as_ref().unwrap().name, "run");
            let args = model
                .arguments(host[1].signature.as_ref().unwrap())
                .unwrap();
            assert!(args[0].optional && !args[0].variadic && args[0].default.is_some());
            assert!(args[1].optional && args[1].variadic && args[1].default.is_none());
            assert_eq!(
                model
                    .values(index.find("Mode").unwrap())
                    .unwrap()
                    .iter()
                    .map(|v| v.value)
                    .collect::<Vec<_>>(),
                ["on", "off"]
            );
            for n in 0..count {
                let fields = model
                    .members(index.find(&format!("D{n}")).unwrap())
                    .unwrap();
                assert_eq!(fields.len(), 1);
                assert_eq!(
                    fields[0].identifier.as_ref().unwrap().name,
                    if n == 0 {
                        "root".to_owned()
                    } else {
                        format!("field{n}")
                    }
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_name_graphs_have_independent_link_expectations() {
        for count in 0..16 {
            for control in [0, 1, 2, 127, 255] {
                let mut data = vec![control; 20];
                data[0] = count;
                name_model(&data);
            }
        }
        for source in [
            "interface A : B {}; interface B : A {};",
            "typedef sequence<T> T;",
            "partial interface Missing {};",
            "interface X {}; dictionary X {};",
            "interface X { attribute Missing value; };",
            "interface X {}; X includes Missing;",
        ] {
            names(&[source]);
        }
    }

    #[test]
    fn generated_member_models_cover_valid_graphs_and_seven_independent_errors() {
        for count in 0..7 {
            for mode in 0..8 {
                member_model(&[count, mode]);
            }
        }
    }

    #[test]
    fn scalar_boundaries_lossy_bytes_and_input_limit_reach_the_harness() {
        for bytes in [
            b"".as_slice(),
            &[0xff, 0, 0xc0, 0xaf],
            "é🦀\u{feff}".as_bytes(),
            b"/*x/*x",
            b"\"x\\\"y\"",
        ] {
            webidl(bytes);
        }
        webidl(&vec![0xff; crate::MAX_INPUT]);
        webidl(&vec![0xff; crate::MAX_INPUT + 1]);
    }

    #[test]
    fn syntax_trees_and_bounded_failures_reach_the_harness() {
        for source in [
            "",
            "interface X { attribute long n; };",
            "[A=(B,C)] typedef (long or DOMString) X;",
            "partial interface X { constructor(); };",
            "enum X { \"é🦀\" };",
            "typedef sequence<> X;",
        ] {
            webidl(source.as_bytes());
        }
        let deep = format!(
            "typedef {}long{} T;",
            "sequence<".repeat(200),
            ">".repeat(200)
        );
        webidl(deep.as_bytes());
        let members = format!("interface X {{ {} }};", "attribute long n;".repeat(200));
        webidl(members.as_bytes());
    }
}
