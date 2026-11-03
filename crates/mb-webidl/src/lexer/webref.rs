use super::{Kind, Lexer};

#[test]
fn webref_fragments_have_complete_lexical_coverage() {
    let files = testdata::files("webref", "ed/idl", "idl");
    assert_eq!(files.len(), 350, "gepinntes webref-Korpus");
    let mut total = 0;
    let mut unexpected = Vec::new();
    for file in &files {
        let source = testdata::read("webref", &format!("ed/idl/{file}"));
        let mut position = 0;
        for token in Lexer::new(&source) {
            assert_eq!(token.span.start, position, "{file}");
            assert!(token.span.end > position, "Fortschritt: {file}");
            assert_eq!(source.get(token.span.clone()), Some(token.text), "{file}");
            position = token.span.end;
            if token.kind == Kind::Other {
                unexpected.push(format!("{file}:{}: {:?}", token.span.start, token.text));
            }
            total += 1;
        }
        assert_eq!(position, source.len(), "{file}");
    }
    assert!(
        unexpected.is_empty(),
        "unerwartete Zeichen:\n{}",
        unexpected.join("\n")
    );
    assert_eq!(total, 191_651, "Tokenzahl des gepinnten Korpus");
    println!(
        "WebIDL: {} webref-Dateien, {total} Tokens einschließlich Trivia",
        files.len()
    );
}
