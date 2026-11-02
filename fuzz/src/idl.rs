//! Source partitioning, progress and token-isolation checks for WebIDL.

use mb_webidl::lexer::{Kind, Lexer};

pub fn webidl(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let source = String::from_utf8_lossy(data);
    let mut lexer = Lexer::new(&source);
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
}
