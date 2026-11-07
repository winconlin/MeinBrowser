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
    assert_eq!(
        metadata("negative:\n  phase: runtime\n  type: Test262Error").negative,
        Some(Negative::Test262Error)
    );
}

#[test]
fn unsupported_or_ambiguous_metadata_cannot_silently_pass() {
    for text in [
        "flags: [async, raw]",
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
        "negative:\n  phase: parse\n  type: Test262Error",
        "negative:\n  phase: runtime\n  type: EvalError",
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
        "assert.sameValue('a', 'b', 'Nachricht')",
        "assert(false)",
        "assert(1)",
    ] {
        assert_eq!(ordinary(source), Outcome::Test262Error, "{source}");
    }
}

#[test]
fn assert_throws_checks_the_constructor() {
    assert_eq!(
        ordinary(
            "assert.throws(TypeError, function () { null.x; }); \
             assert.throws(ReferenceError, function () { missing; }); \
             assert.throws(RangeError, function () { throw new RangeError('r'); }); \
             assert.throws(Test262Error, function () { throw new Test262Error(); }); \
             assert.throws(SyntaxError, function () { $262.evalScript('let = ;'); });"
        ),
        Outcome::Normal
    );
    for source in [
        "assert.throws(TypeError, function () {})",
        "assert.throws(TypeError, function () { missing; })",
        "assert.throws(TypeError, function () { throw 1; })",
        "assert.throws(TypeError)",
    ] {
        assert_eq!(ordinary(source), Outcome::Test262Error, "{source}");
    }
    // Die Meldung entsteht mit String und Object.prototype.toString.
    assert_eq!(
        ordinary(
            "try { assert.sameValue({}, 1); } catch (e) { if (e.message !== 'Expected SameValue(«[object Object]», «1») to be true') throw e; }"
        ),
        Outcome::Normal
    );
    assert_eq!(
        ordinary(
            "try { assert.sameValue('a', -0, 'm'); } catch (e) { if (e.message !== 'm Expected SameValue(«\"a\"», «-0») to be true' || String(e) !== 'Test262Error: ' + e.message) throw e; }"
        ),
        Outcome::Normal
    );
}

#[test]
fn host_functions_follow_interpreting_md() {
    assert_eq!(
        ordinary(
            "if ($262.global !== this || typeof print !== 'function') throw 1; \
             var other = $262.createRealm(); \
             if (other === $262 || other.global === this || other.evalScript('this') !== other.global) throw 2; \
             if (other.evalScript('var x = 1; x + 1') !== 2 || other.global.x !== 1 || typeof x !== 'undefined') throw 3; \
             var error = other.evalScript('try { null.x; } catch (e) { e }'); \
             if (error instanceof TypeError || !(error instanceof other.global.TypeError)) throw 4; \
             try { other.evalScript('let = ;'); throw 5; } catch (e) { if (!(e instanceof other.global.SyntaxError)) throw e; } \
             if ($262.evalScript('var y = 3; y') !== 3 || y !== 3) throw 6; \
             if ($262.gc() !== undefined || other.global.Object === Object) throw 7;"
        ),
        Outcome::Normal
    );
    assert_eq!(
        ordinary(
            "print = 1; $262 = 2; if (print !== 1 || $262 !== 2) throw 'beschreibbar'; \
             if (!(delete this.print) || !(delete this.$262) || typeof print !== 'undefined') throw 'konfigurierbar';"
        ),
        Outcome::Normal
    );
    // Ein Fehler aus einem anderen Realm zählt nach seiner Art.
    let meta = metadata("negative:\n  phase: runtime\n  type: TypeError");
    assert_eq!(
        run(
            "throw $262.createRealm().evalScript('new TypeError()');",
            &meta,
            Mode::Sloppy,
            &harness(&meta),
            false
        ),
        Ok(Outcome::Error(ErrorKind::TypeError))
    );
}

/// Attribute nach INTERPRETING.md: beschreibbar, konfigurierbar, nicht
/// aufzählbar; für die Eigenschaften von `$262` dieselben.
#[test]
fn host_properties_are_writable_configurable_and_hidden() {
    let mut agent = Agent::new().expect("Agent");
    agent.set_host(Rc::new(Test262Host::default()));
    let realm = agent.create_realm().expect("Realm");
    agent.scope(|agent, scope| {
        let global = agent.global_object(scope, realm);
        let host = own_data(agent, scope, global, "$262").expect("$262");
        for (object, name) in [
            (global, "print"),
            (global, "$262"),
            (host, "createRealm"),
            (host, "evalScript"),
            (host, "gc"),
            (host, "global"),
        ] {
            let units: Vec<u16> = name.encode_utf16().collect();
            let key = agent.key_from_units(scope, &units).expect("Speicher");
            let descriptor = get_own_property(agent, scope, object, key)
                .expect("kein Fehler")
                .unwrap_or_else(|| panic!("{name} fehlt"));
            assert_eq!(
                (
                    descriptor.writable,
                    descriptor.enumerable,
                    descriptor.configurable
                ),
                (Some(true), Some(false), Some(true)),
                "{name}"
            );
        }
    });
}

#[test]
fn print_reports_its_argument_as_a_string() {
    let host = Rc::new(Test262Host::default());
    let mut agent = Agent::new().expect("Agent");
    agent.set_host(Rc::clone(&host) as Rc<dyn crate::host::Host>);
    let realm = agent.create_realm().expect("Realm");
    let budget = Rc::new(Budget::new(STEPS));
    assert_eq!(
        evaluate(
            &mut agent,
            realm,
            "print('Test262:AsyncTestComplete'); print(1.5); print(); \
             print({ toString: function () { return 'eigen'; } }); \
             $262.createRealm().evalScript('print(this === $262.global)');",
            &budget
        ),
        Outcome::Normal
    );
    let printed: Vec<String> = host
        .printed()
        .iter()
        .map(|units| String::from_utf16(units).expect("UTF-16"))
        .collect();
    assert_eq!(
        printed,
        [
            "Test262:AsyncTestComplete",
            "1.5",
            "undefined",
            "eigen",
            "true"
        ]
    );
}

#[test]
fn runtime_negative_requires_the_exact_error_and_phase() {
    let meta = metadata("negative:\n  phase: runtime\n  type: TypeError");
    let wanted = expected(meta.negative);
    let files = harness(&meta);
    let run_negative = |source| run(source, &meta, Mode::Sloppy, &files, false).expect("Harness");
    assert_eq!(run_negative("null.x"), wanted);
    assert_eq!(run_negative("throw new TypeError('t');"), wanted);
    assert_eq!(
        run_negative("try { null.x; } catch (e) { throw e; }"),
        wanted
    );
    for source in [
        "42",
        "missingName",
        "throw 'TypeError'",
        "throw {name: 'TypeError'}",
        "throw {constructor: TypeError}",
        "throw new RangeError()",
        "throw new Error('TypeError')",
        "assert.sameValue(1, 2)",
        "let = ;",
        "with ({}) {}",
        "while (true) {}",
    ] {
        assert_ne!(run_negative(source), wanted, "{source}");
    }
    assert_eq!(
        ordinary("while (true) {}"),
        Outcome::Host(Abrupt::Exhausted(crate::agent::Exhausted::Steps))
    );
    assert_eq!(ordinary("with ({}) {}"), Outcome::Unsupported);
}

#[test]
fn test262_error_negatives_need_the_harness_constructor() {
    let meta = metadata("negative:\n  phase: runtime\n  type: Test262Error");
    let wanted = expected(meta.negative);
    assert_eq!(wanted, Outcome::Test262Error);
    let files = harness(&meta);
    let run_negative = |source| run(source, &meta, Mode::Sloppy, &files, false).expect("Harness");
    for source in [
        "throw new Test262Error();",
        "Test262Error.thrower('m');",
        "assert(false);",
        "var e = Test262Error('ohne new'); throw e;",
    ] {
        assert_eq!(run_negative(source), wanted, "{source}");
    }
    for source in [
        "1",
        "throw 'Test262Error';",
        "throw {constructor: Test262Error};",
        "throw new TypeError();",
    ] {
        assert_ne!(run_negative(source), wanted, "{source}");
    }
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

#[test]
fn async_tests_load_done_print_handle_before_their_includes() {
    let names = |text| -> Vec<String> {
        harness(&metadata(text))
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    };
    assert_eq!(
        names("flags: [async]"),
        ["assert.js", "sta.js", "doneprintHandle.js"]
    );
    assert_eq!(
        names("flags: [async]\nincludes: [compareArray.js]"),
        [
            "assert.js",
            "sta.js",
            "doneprintHandle.js",
            "compareArray.js"
        ]
    );
    assert_eq!(names(""), ["assert.js", "sta.js"]);
}

#[test]
fn async_tests_pass_only_after_reporting_completion() {
    let meta = metadata("flags: [async]");
    let files = harness(&meta);
    let run_async = |source| run(source, &meta, Mode::Sloppy, &files, false).expect("Harness");
    for source in [
        "$DONE();",
        "Promise.resolve().then(function () { $DONE(); });",
        "Promise.resolve(1).then(function (v) { assert.sameValue(v, 1); }).then($DONE, $DONE);",
    ] {
        assert_eq!(run_async(source), Outcome::Normal, "{source}");
    }
    for (source, wanted) in [
        (
            "Promise.resolve(1).then(function (v) { assert.sameValue(v, 2); }).then($DONE, $DONE);",
            Outcome::AsyncFailure,
        ),
        (
            "Promise.reject(new TypeError('t')).then($DONE, $DONE);",
            Outcome::AsyncFailure,
        ),
        ("$DONE('Grund');", Outcome::AsyncFailure),
        (
            "$DONE(); print('Test262:AsyncTestFailure:später');",
            Outcome::AsyncFailure,
        ),
        (
            "new Promise(function () {}).then($DONE, $DONE);",
            Outcome::AsyncIncomplete,
        ),
        ("1;", Outcome::AsyncIncomplete),
        (
            "print('Test262:AsyncTestComplete ');",
            Outcome::AsyncIncomplete,
        ),
        ("null.x; $DONE();", Outcome::Error(ErrorKind::TypeError)),
    ] {
        assert_eq!(run_async(source), wanted, "{source}");
    }
    // Ohne `async` zählt `print` nicht.
    assert_eq!(
        ordinary("print('Test262:AsyncTestFailure:x');"),
        Outcome::Normal
    );
}

#[test]
fn jobs_run_after_each_script_and_count_like_it() {
    let meta = metadata("");
    let files = vec![(
        "jobs.js".to_owned(),
        "var order = ''; Promise.resolve().then(function () { order += 'job,'; }); order += 'script,';"
            .to_owned(),
    )];
    let source = "if (order !== 'script,job,') throw new Test262Error(order);";
    for stress in [false, true] {
        assert_eq!(
            run(source, &meta, Mode::Sloppy, &files, stress),
            Ok(Outcome::Normal)
        );
    }
    // Wirft ein Job, zählt das wie eine Ausnahme des Scripts: Hier wirft
    // [[Reject]] einer eigenen Capability.
    assert_eq!(
        ordinary(
            "function F(executor) { executor(function () {}, function () { throw new RangeError(); }); } \
             F[Symbol.species] = F; var p = Promise.reject(1); p.constructor = F; p.then();"
        ),
        Outcome::Error(ErrorKind::RangeError)
    );
    assert_eq!(
        ordinary("function loop() { Promise.resolve().then(loop); } loop();"),
        Outcome::Host(Abrupt::Exhausted(crate::agent::Exhausted::Steps))
    );
}
