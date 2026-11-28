//! Tests für den Läufer mit testharness.js. Die kleinen Testdateien liegen
//! hier im Speicher; testharness.js selbst kommt aus den Testdaten
//! (`cargo run -p testdata`).

use std::collections::BTreeSet;

use super::{
    Directory, EXPECTATIONS, FileResult, Files, HarnessStatus, Include, Status, Subtest,
    parse_meta, render_expectations, resolve, run_all, run_file, run_limited, select,
};

/// Dateien im Speicher vor den Testdaten der WPT.
struct Overlay {
    files: Vec<(&'static str, &'static str)>,
    wpt: Directory,
}

impl Files for Overlay {
    fn read(&self, path: &str) -> Option<String> {
        match self.files.iter().find(|(found, _)| *found == path) {
            Some((_, source)) => Some((*source).to_owned()),
            None => self.wpt.read(path),
        }
    }
}

fn overlay(files: &[(&'static str, &'static str)]) -> Overlay {
    Overlay {
        files: files.to_vec(),
        wpt: Directory {
            root: testdata::root().join("wpt"),
        },
    }
}

/// Führt `source` als `t/test.any.js` aus.
fn run(source: &'static str) -> FileResult {
    run_file(&overlay(&[("t/test.any.js", source)]), "t/test.any.js")
}

/// Die Untertests als `name:STATUS`.
fn statuses(result: &FileResult) -> Vec<String> {
    result
        .subtests
        .iter()
        .map(|subtest| format!("{}:{}", subtest.name, subtest.status.name()))
        .collect()
}

#[track_caller]
fn check(source: &'static str, harness: HarnessStatus, expected: &[&str]) -> FileResult {
    let result = run(source);
    assert_eq!(
        (result.harness, statuses(&result)),
        (
            harness,
            expected.iter().map(|item| (*item).to_owned()).collect()
        ),
        "{source}\n{result:?}"
    );
    result
}

#[test]
fn meta_lines_follow_the_wpt_manifest() {
    let meta = parse_meta(
        "// META: global=window,worker,jsshell\n\
         //META:   script=/common/gc.js\r\n\
         // META: title=Ein \"Titel\"\n\
         // META: global=window\n\
         // META: timeout=long\n\
         // META: variant=?1-10\n\
         // META: script=helper.js\n\
         \n\
         // META: script=nicht-mehr.js\n",
    );
    assert_eq!(
        meta.globals,
        [
            "window",
            "dedicatedworker",
            "sharedworker",
            "serviceworker",
            "jsshell"
        ]
    );
    assert!(meta.for_shell());
    assert!(meta.long_timeout);
    assert_eq!(meta.variants, ["?1-10"]);
    assert_eq!(
        meta.includes,
        [
            Include::Script("/common/gc.js".to_owned()),
            Include::Title("Ein \"Titel\"".to_owned()),
            Include::Script("helper.js".to_owned()),
        ]
    );
    // Ohne `global=` oder mit leerem Wert gelten `window` und
    // `dedicatedworker`; unbekannte Namen zählen nicht.
    assert_eq!(parse_meta("test();").globals, ["window", "dedicatedworker"]);
    assert_eq!(
        parse_meta("// META: global=\n").globals,
        ["window", "dedicatedworker"]
    );
    assert_eq!(
        parse_meta("// META: global=shadowrealm,nirgends\n")
            .globals
            .len(),
        6
    );
    assert!(!parse_meta("// META: global=window\n").for_shell());
    // Ein Name mit anderen Zeichen beendet die Zeilen.
    assert!(
        parse_meta("// META: a-b=1\n// META: timeout=long\n")
            .includes
            .is_empty()
    );
    assert!(!parse_meta("// META: a-b=1\n// META: timeout=long\n").long_timeout);
}

#[test]
fn scripts_resolve_against_the_file_or_the_root() {
    assert_eq!(resolve("a/b/c.any.js", "d.js").as_deref(), Some("a/b/d.js"));
    assert_eq!(
        resolve("a/b/c.any.js", "../d.js").as_deref(),
        Some("a/d.js")
    );
    assert_eq!(
        resolve("a/b/c.any.js", "./x/./d.js").as_deref(),
        Some("a/b/x/d.js")
    );
    assert_eq!(
        resolve("a/b/c.any.js", "/common/gc.js").as_deref(),
        Some("common/gc.js")
    );
    assert_eq!(
        resolve("a/b/c.any.js", "d.js?x=1#y").as_deref(),
        Some("a/b/d.js")
    );
    assert_eq!(resolve("a/c.any.js", "../../d.js"), None);
    assert_eq!(resolve("a/c.any.js", "/"), None);
}

#[test]
fn subtests_report_their_status_and_message() {
    let result = check(
        "test(() => {}, 'leer');\n\
         test(() => assert_equals(1, 2, 'eins'), 'falsch');\n\
         test(() => { throw new RangeError('weg'); }, 'wirft');\n\
         test(() => assert_implements_optional(false, 'fehlt'), 'optional');\n\
         test(() => assert_true(true), 'Name mit\\tTab');",
        HarnessStatus::Ok,
        &[
            "leer:PASS",
            "falsch:FAIL",
            "wirft:FAIL",
            "optional:PRECONDITION_FAILED",
            "Name mit\tTab:PASS",
        ],
    );
    let message = |index: usize| {
        result
            .subtests
            .get(index)
            .and_then(|subtest| subtest.message.clone())
            .unwrap_or_default()
    };
    assert_eq!(message(0), "");
    assert!(
        message(1).starts_with("assert_equals: eins expected 2"),
        "{}",
        message(1)
    );
    assert_eq!(message(2), "weg");
}

#[test]
fn timers_run_on_the_virtual_clock() {
    // Die Uhr springt zum nächsten Zeitgeber; bei gleicher Zeit läuft der
    // ältere zuerst, ein gelöschter gar nicht. Date.now folgt der Uhr.
    check(
        "var start = Date.now(); var log = [];\n\
         async_test(t => {\n\
           setTimeout(t.step_func(() => log.push('b', Date.now() - start)), 20);\n\
           setTimeout((x, y) => log.push(x + y), 10, 'a', '!');\n\
           var id = setTimeout(() => log.push('nie'), 15);\n\
           setTimeout(() => log.push('c'), 20);\n\
           clearTimeout(id);\n\
           setTimeout(t.step_func_done(() => {\n\
             assert_equals(log.join(), 'a!,b,20,c');\n\
           }), 20);\n\
         }, 'Reihenfolge');\n\
         async_test(t => {\n\
           var ids = [setTimeout(() => {}, -5), setTimeout(() => {}, 'x'), setTimeout(() => {})];\n\
           assert_true(ids.every(id => typeof id === 'number' && id > 0));\n\
           assert_throws_js(TypeError, () => setTimeout('code', 0));\n\
           t.step_timeout(() => t.done(), 1e9 % 5000);\n\
         }, 'Werte');",
        HarnessStatus::Ok,
        &["Reihenfolge:PASS", "Werte:PASS"],
    );
}

#[test]
fn microtasks_run_before_the_next_timer() {
    check(
        "promise_test(async () => {\n\
           var log = [];\n\
           setTimeout(() => log.push('timer'), 0);\n\
           Promise.resolve().then(() => log.push('job'));\n\
           await new Promise(resolve => setTimeout(resolve, 0));\n\
           assert_equals(log.join(), 'job,timer');\n\
         }, 'Jobs zuerst');\n\
         promise_test(() => Promise.reject(new TypeError('nein')), 'abgelehnt');",
        HarnessStatus::Ok,
        &["Jobs zuerst:PASS", "abgelehnt:FAIL"],
    );
}

#[test]
fn uncaught_errors_set_the_harness_to_error() {
    // Wirft die Datei nach einem Test, ist die Harness ERROR.
    let result = check(
        "test(() => {}, 'vorher');\nthrow new Error('oben');",
        HarnessStatus::Error,
        &["vorher:PASS"],
    );
    assert_eq!(result.message.as_deref(), Some("Error: oben"));
    // Ein Fehler der Engine kommt als Fehlerobjekt an; seine Meldung ist
    // leer.
    let result = check("null.x;", HarnessStatus::Error, &[]);
    assert_eq!(result.message.as_deref(), Some("TypeError"));
    // Mit single_test wird die Datei zu einem einzigen Test, der scheitert;
    // er heißt nach `title=`.
    check(
        "setup({ single_test: true }); null.x;",
        HarnessStatus::Ok,
        &["Untitled:FAIL"],
    );
    check(
        "// META: title=Einzeln\nsetup({ single_test: true }); null.x;",
        HarnessStatus::Ok,
        &["Einzeln:FAIL"],
    );
    // Ein Fehler in einem Zeitgeber und ein abgelehntes Promise ohne
    // Handler.
    check(
        "async_test(t => { setTimeout(() => { throw 1; }, 5); }, 'a');",
        HarnessStatus::Error,
        &["a:TIMEOUT"],
    );
    // Die Meldung über ein abgelehntes Promise kommt nach den Microtasks.
    // Wartet dann noch ein Test, ist die Harness ERROR; war sie schon
    // fertig, bleibt sie OK, wie in einem Worker.
    let result = check(
        "test(() => {}, 'a'); Promise.reject(new Error('weg'));\n\
         async_test(t => { setTimeout(() => t.done(), 0); }, 'b');",
        HarnessStatus::Error,
        &["a:PASS", "b:PASS"],
    );
    assert_eq!(result.message.as_deref(), Some("Unhandled rejection: weg"));
    check(
        "test(() => {}, 'a'); Promise.reject(new Error('weg'));",
        HarnessStatus::Ok,
        &["a:PASS"],
    );
    // Ein Handler, der später kommt, aber vor dem Ende der Microtasks,
    // zählt noch.
    check(
        "var p = Promise.reject(1); Promise.resolve().then(() => p.catch(() => {}));\n\
         async_test(t => { setTimeout(() => t.done(), 0); }, 'b');",
        HarnessStatus::Ok,
        &["b:PASS"],
    );
    // Mit allow_uncaught_exception bleibt die Harness OK.
    check(
        "setup({ allow_uncaught_exception: true });\n\
         test(() => {}, 'a'); throw new Error('oben');",
        HarnessStatus::Ok,
        &["a:PASS"],
    );
}

#[test]
fn early_errors_and_missing_scripts_end_loading() {
    // Ein früher Fehler: Von der Datei läuft nichts.
    let result = check("test(() => {}, 'a');\n}", HarnessStatus::Error, &[]);
    assert_eq!(
        result.message.as_deref(),
        Some("SyntaxError in t/test.any.js bei Codeeinheit 21")
    );
    // Fehlt ein Skript, laufen es und die Datei nicht.
    let result = check(
        "// META: script=fehlt.js\ntest(() => {}, 'a');",
        HarnessStatus::Error,
        &[],
    );
    assert_eq!(result.message.as_deref(), Some("Skript fehlt: /t/fehlt.js"));
}

#[test]
fn meta_scripts_and_titles_run_before_the_file() {
    let files = overlay(&[
        (
            "t/test.any.js",
            "// META: script=helper.js\n// META: title=Titel \"mit\" \\ Zeichen\n\
             test(() => assert_equals(helper(), 2), 'Hilfe');\n\
             test(() => assert_equals(self.META_TITLE, 'Titel \"mit\" \\\\ Zeichen'), 'Titel');",
        ),
        ("t/helper.js", "function helper() { return 2; }"),
    ]);
    let result = run_file(&files, "t/test.any.js");
    assert_eq!(
        statuses(&result),
        ["Hilfe:PASS", "Titel:PASS"],
        "{result:?}"
    );
    assert_eq!(result.harness, HarnessStatus::Ok);
}

#[test]
fn runs_that_never_finish_end_with_timeout() {
    // Kein Test in der Shell hat eine eigene Zeitgrenze; die virtuelle Uhr
    // endet nach 10 s, mit timeout=long nach 60 s. Dann ruft der Läufer
    // `timeout()` aus testharness.js.
    let result = check(
        "async_test(t => { function again() { setTimeout(again, 1000); } again(); }, 'a');",
        HarnessStatus::Timeout,
        &["a:TIMEOUT"],
    );
    assert_eq!(
        result
            .subtests
            .first()
            .and_then(|subtest| subtest.message.as_deref()),
        Some("Test timed out")
    );
    assert_eq!(
        result.message.as_deref(),
        Some("Zeitgrenze von 10 s erreicht")
    );
    let result = check(
        "// META: timeout=long\n\
         var start = Date.now();\n\
         async_test(t => { function again() { setTimeout(again, 1000); } again(); }, 'a');\n\
         test(() => {}, 'b');",
        HarnessStatus::Timeout,
        &["a:TIMEOUT", "b:PASS"],
    );
    assert_eq!(
        result.message.as_deref(),
        Some("Zeitgrenze von 60 s erreicht")
    );
    // Wartet nichts mehr, endet der Lauf gleich.
    let result = check(
        "async_test(t => {}, 'a');",
        HarnessStatus::Timeout,
        &["a:TIMEOUT"],
    );
    assert!(
        result
            .message
            .is_some_and(|message| message.contains("nichts wartet"))
    );
    // Eine Schleife verbraucht die Arbeitsschritte.
    let files = overlay(&[(
        "t/test.any.js",
        "test(() => {}, 'a'); async_test(t => { setTimeout(() => { for (;;) {} }, 1); }, 'b');",
    )]);
    let result = run_limited(&files, "t/test.any.js", 2_000_000);
    assert_eq!(
        (result.harness, statuses(&result), result.message.as_deref()),
        (
            HarnessStatus::Timeout,
            vec!["a:PASS".to_owned(), "b:TIMEOUT".to_owned()],
            Some("Arbeitsschritte erschöpft")
        )
    );
}

#[test]
fn gc_runs_the_cleanup_of_finalization_registries() {
    check(
        "// META: script=/common/gc.js\n\
         promise_test(async () => {\n\
           var held = [];\n\
           var registry = new FinalizationRegistry(value => held.push(value));\n\
           (function () { registry.register({}, 'x'); })();\n\
           await garbageCollect();\n\
           await new Promise(resolve => setTimeout(resolve, 0));\n\
           assert_equals(held.join(), 'x');\n\
         }, 'Aufräumen');",
        HarnessStatus::Ok,
        &["Aufräumen:PASS"],
    );
}

#[test]
fn expectations_escape_names_and_count_results() {
    let subtest = |name: &str, status| Subtest {
        name: name.to_owned(),
        status,
        message: None,
    };
    let results = vec![
        (
            "a.any.js".to_owned(),
            FileResult {
                harness: HarnessStatus::Ok,
                message: None,
                subtests: vec![
                    subtest("gut", Status::Pass),
                    subtest("a\tb\\c\nd\re", Status::Fail),
                ],
            },
        ),
        (
            "b.any.js".to_owned(),
            FileResult {
                harness: HarnessStatus::Error,
                message: Some("weg".to_owned()),
                subtests: vec![subtest("x", Status::NotRun)],
            },
        ),
    ];
    let rendered = render_expectations(&results);
    let body: Vec<&str> = rendered
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    assert_eq!(
        body,
        [
            "a.any.js\tFAIL\ta\\tb\\\\c\\nd\\re",
            "b.any.js\tERROR",
            "b.any.js\tNOTRUN\tx",
        ]
    );
    assert!(rendered.contains("# 2 Dateien, Harness OK in 1; bestanden 1 von 3 Untertests\n"));
}

/// Alle ausgewählten Dateien der WPT gegen `testharness-erwartet.txt`. Bei
/// einer Abweichung kommt die neue Fassung nach
/// `target/testrunner/testharness-erwartet.txt`.
#[test]
fn selected_files_match_the_expectations() {
    let root = testdata::root().join("wpt");
    let paths = select(&root);
    assert!(
        paths.len() >= 80,
        "nur {} Dateien gefunden; fehlen die Testdaten (cargo run -p testdata)?",
        paths.len()
    );
    let results = run_all(&root, &paths);
    let rendered = render_expectations(&results);
    if rendered == EXPECTATIONS {
        return;
    }
    let out = testdata::root()
        .parent()
        .map(|target| target.join("testrunner"))
        .unwrap_or_default();
    let file = out.join("testharness-erwartet.txt");
    let written = std::fs::create_dir_all(&out).and_then(|()| std::fs::write(&file, &rendered));
    let lines = |text: &str| -> BTreeSet<String> {
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .map(str::to_owned)
            .collect()
    };
    let (expected, actual) = (lines(EXPECTATIONS), lines(&rendered));
    let mut report = String::new();
    for line in actual.difference(&expected) {
        report.push_str(&format!("Rückschritt: {line}\n"));
    }
    for line in expected.difference(&actual) {
        report.push_str(&format!("unerwarteter Erfolg: {line}\n"));
    }
    if report.is_empty() {
        report.push_str("nur die Zahlen im Kopf weichen ab\n");
    }
    panic!(
        "Tests mit testharness.js weichen von testharness-erwartet.txt ab ({}):\n{report}",
        match written {
            Ok(()) => format!("neue Fassung in {}", file.display()),
            Err(error) => format!("neue Fassung nicht geschrieben: {error}"),
        }
    );
}
