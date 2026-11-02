use super::*;

fn metadata(text: &str) -> Metadata {
    Metadata::read(&format!("/*---\n{text}\n---*/")).expect("Testmetadaten")
}

fn ordinary(source: &str) -> Outcome {
    let metadata = metadata("");
    run(source, &metadata, Mode::Sloppy, &harness(&metadata), false).expect("Harness läuft")
}

#[test]
fn strict_raw_and_default_modes_follow_metadata() {
    assert_eq!(metadata("").modes(), [Mode::Sloppy, Mode::Strict]);
    assert_eq!(metadata("flags: [onlyStrict]").modes(), [Mode::Strict]);
    for flag in ["noStrict", "raw"] {
        assert_eq!(
            metadata(&format!("flags: [{flag}]")).modes(),
            [Mode::Sloppy]
        );
    }
    let input = "/*---\nflags: [raw]\n---*/\n#!not-at-start";
    let meta = Metadata::read(input).expect("raw");
    // Kein Entfernen des Kopfes oder Verschieben eines Hashbang-Kommentars.
    assert_eq!(
        run(input, &meta, Mode::Sloppy, &[], false),
        Ok(Outcome::Syntax)
    );
    let meta = metadata("");
    for stress in [false, true] {
        assert_eq!(
            run("undeclared = 1;", &meta, Mode::Sloppy, &[], stress),
            Ok(Outcome::Normal)
        );
        assert_eq!(
            run("undeclared = 1;", &meta, Mode::Strict, &[], stress),
            Ok(Outcome::Error(ErrorKind::ReferenceError))
        );
    }
}

#[test]
fn list_and_negative_fields_are_read_in_both_orders() {
    let meta = metadata(
        "description: |\n  flags: [module]\nflags:\n  - noStrict\nincludes: [compareArray.js]\nfeatures:\n  - default-parameters\nnegative:\n  type: ReferenceError\n  phase: runtime",
    );
    assert_eq!(meta.flags, ["noStrict"]);
    assert_eq!(meta.includes, ["compareArray.js"]);
    assert_eq!(meta.features, ["default-parameters"]);
    assert_eq!(
        meta.negative,
        Some(Negative::Runtime(ErrorKind::ReferenceError))
    );
    assert_eq!(
        metadata("negative:\n  phase: parse\n  type: SyntaxError").negative,
        Some(Negative::Parse)
    );
}

#[test]
fn unsupported_or_ambiguous_metadata_cannot_silently_pass() {
    for text in [
        "flags: [async]",
        "flags: [module, raw]",
        "flags: [CanBlockIsTrue]",
        "flags: [unknown]",
        "flags: [raw, onlyStrict]",
        "flags: [noStrict, onlyStrict]",
        "flags: [raw, raw]",
        "flags: [raw]\nflags: []",
        "flags: [raw",
        "includes: [../assert.js]",
        "includes: [C:/assert.js]",
        "includes: [a\\b.js]",
        "includes: [/assert.js]",
        "includes: [a//b.js]",
        "includes: [a_FIXTURE.js]",
        "flags: [raw]\nincludes: [assert.js]",
        "flags: raw",
        "flags:\n  module",
        "negative:\n  phase: resolution\n  type: SyntaxError",
        "negative:\n  phase: runtime",
        "negative:\n  phase: runtime\n  phase: parse\n  type: SyntaxError",
        "negative: {phase: runtime, type: TypeError}",
        "negative:\n  phase: parse\n  type: TypeError",
        "locale: [en]",
    ] {
        assert!(
            Metadata::read(&format!("/*---\n{text}\n---*/")).is_err(),
            "{text}"
        );
    }
    assert!(Metadata::read("var x = 1").is_err());
    assert!(Metadata::read("/*---\nflags: []").is_err());
}

#[test]
fn same_value_assertions_detect_wrong_results_signed_zero_and_nan() {
    assert_eq!(
        ordinary(
            "assert.sameValue(NaN, NaN); assert.sameValue(-0, -0); assert.notSameValue(0, -0); assert.sameValue(undefined, undefined); assert.sameValue('abc', 'abc'); var a = {}; assert.sameValue(a, a); assert.notSameValue({}, {});"
        ),
        Outcome::Normal
    );
    for source in [
        "assert.sameValue(1, 2)",
        "assert.sameValue(0, -0)",
        "assert.sameValue(NaN, 0)",
        "assert.notSameValue(NaN, NaN)",
        "assert.notSameValue(0, 0)",
        "assert.sameValue({}, {})",
    ] {
        assert_eq!(ordinary(source), Outcome::ThrownValue, "{source}");
    }
}

#[test]
fn runtime_negative_requires_the_exact_error_and_phase() {
    let meta = metadata("negative:\n  phase: runtime\n  type: TypeError");
    let wanted = expected(meta.negative);
    let files = harness(&meta);
    let run_negative = |source| run(source, &meta, Mode::Sloppy, &files, false).expect("Harness");
    assert_eq!(run_negative("null.x"), wanted);
    for source in [
        "42",
        "missingName",
        "throw 'TypeError'",
        "throw {name: 'TypeError'}",
        "assert.sameValue(1, 2)",
        "let = ;",
        "try {} catch (e) {}",
        "while (true) {}",
    ] {
        assert_ne!(run_negative(source), wanted, "{source}");
    }
    assert_eq!(
        ordinary("while (true) {}"),
        Outcome::Host(Abrupt::Exhausted(crate::agent::Exhausted::Steps))
    );
    assert_eq!(ordinary("try {} catch (e) {}"), Outcome::Unsupported);
}

#[test]
fn harness_failure_cannot_fulfil_a_runtime_negative() {
    let meta = metadata("negative:\n  phase: runtime\n  type: TypeError");
    let files = vec![("kaputt.js".to_owned(), "null.x".to_owned())];
    assert!(run("null.x", &meta, Mode::Sloppy, &files, false).is_err());
    let raw = metadata("flags: [raw]");
    assert_eq!(
        run("42", &raw, Mode::Sloppy, &files, false),
        Ok(Outcome::Normal)
    );
}

#[test]
fn harness_order_and_realm_isolation_are_observable() {
    let meta = metadata("");
    let files = vec![
        ("eins.js".to_owned(), "var order = 1;".to_owned()),
        ("zwei.js".to_owned(), "order = order * 10 + 2;".to_owned()),
    ];
    let source =
        "if (order !== 12 || typeof leaked !== 'undefined') throw 'wrong'; var leaked = 1;";
    for stress in [false, true] {
        for _ in 0..2 {
            assert_eq!(
                run(source, &meta, Mode::Sloppy, &files, stress),
                Ok(Outcome::Normal)
            );
        }
    }
}

#[test]
fn observation_does_not_run_getters_on_thrown_objects() {
    assert_eq!(
        ordinary(
            "throw { get constructor() { while (true) {} }, get name() { while (true) {} } };"
        ),
        Outcome::ThrownValue
    );
}

#[test]
fn setup_and_test_share_the_remaining_work_quota() {
    let mut agent = Agent::new().expect("Agent");
    let realm = agent.create_realm().expect("Realm");
    let budget = Rc::new(Budget::new(20));
    assert_eq!(evaluate(&mut agent, realm, "1", &budget), Outcome::Normal);
    assert!(budget.remaining() < 20);
    assert_eq!(
        evaluate(&mut agent, realm, "while (true) {}", &budget),
        Outcome::Host(Abrupt::Exhausted(crate::agent::Exhausted::Steps))
    );
    assert_eq!(
        evaluate(&mut agent, realm, "1", &budget),
        Outcome::Host(Abrupt::Exhausted(crate::agent::Exhausted::Steps))
    );
}
