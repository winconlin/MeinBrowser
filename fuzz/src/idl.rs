//! Source partitioning, progress and token-isolation checks for WebIDL.

use mb_webidl::lexer::{Kind, Lexer};
use mb_webidl::parser::{self, Budget, Document, ErrorKind, Kind as NodeKind, Limits};

pub fn webidl(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let source = String::from_utf8_lossy(data);
    tokens(&source);
    syntax(&source);
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

#[cfg(test)]
mod tests {
    use super::*;

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
