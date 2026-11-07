//! Test262 über ganze Verzeichnisse mit je einer Datei der erwarteten
//! Ergebnisse: alle Dateien in `test/language/` mit
//! `erwartete-fehlschlaege.txt` und die Teile von `test/built-ins/`, die es
//! schon ganz gibt, mit `erwartete-fehlschlaege-built-ins.txt`
//! ([`BUILT_INS`]). Eine solche Datei nennt jede Variante, die noch nicht
//! besteht; jede andere muss bestehen. Weicht ein Lauf ab, durch einen
//! Rückschritt oder einen unerwarteten Erfolg, scheitert der Test und
//! schreibt die neue Datei unter ihrem Namen nach `target/test262/`, zum
//! Prüfen und Übernehmen.
//!
//! Jede Variante läuft wie in der festen Auswahl ([`super::run`]) in einem
//! frischen Agent mit dem Host für Test262, aber nur ohne erzwungene
//! Sammlungen; den Belastungsmodus prüft die feste Auswahl. Vorschläge aus
//! `features.txt` laufen nicht. Metadaten, die der Leser ablehnt, etwa
//! `module`, lassen die ganze Datei scheitern. Eine Panik der Engine darf
//! nie erwartet werden.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::metadata::{Metadata, Mode};
use super::{Outcome, expected, proposals, run, selection};

const SOURCE: &str = "test262";

/// Ein Lauf über ein Verzeichnis von Test262 mit seiner Datei der
/// erwarteten Fehlschläge.
struct Suite {
    /// Das Verzeichnis, zu dem die Pfade der Datei relativ sind.
    directory: &'static str,
    /// Die Teile darin, die laufen; leer für alle.
    parts: &'static [&'static str],
    /// Ob die feste Auswahl ([`selection`]) darin liegt und auch hier
    /// bestehen muss.
    selection: bool,
    /// Der Name der Datei der erwarteten Fehlschläge.
    file: &'static str,
    /// Ihr Inhalt.
    expected: &'static str,
}

const LANGUAGE: Suite = Suite {
    directory: "test/language",
    parts: &[],
    selection: true,
    file: "erwartete-fehlschlaege.txt",
    expected: include_str!("erwartete-fehlschlaege.txt"),
};

/// Die eingebauten Objekte, die es schon mit allen Eigenschaften gibt.
const BUILT_INS: Suite = Suite {
    directory: "test/built-ins",
    parts: &["Promise"],
    selection: false,
    file: "erwartete-fehlschlaege-built-ins.txt",
    expected: include_str!("erwartete-fehlschlaege-built-ins.txt"),
};

/// Eine Variante einer Datei: ein Modus, oder die ganze Datei, wenn sie
/// sich nicht ausführen lässt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Variant {
    File,
    Sloppy,
    Strict,
}

impl Variant {
    fn of(mode: Mode) -> Variant {
        match mode {
            Mode::Sloppy => Variant::Sloppy,
            Mode::Strict => Variant::Strict,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Variant::File => "",
            Variant::Sloppy => "sloppy",
            Variant::Strict => "strict",
        }
    }
}

/// Was eine Datei ergab.
enum FileResult {
    /// Ein Vorschlag, der nicht läuft.
    Skipped,
    /// Jede Variante mit dem Grund ihres Scheiterns, `None`, wenn sie
    /// bestand.
    Ran(Vec<(Variant, Option<String>)>),
}

/// Alle Testdateien unter `directory`, sortiert, ohne `_FIXTURE`.
fn test_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut work = vec![directory.to_path_buf()];
    while let Some(path) = work.pop() {
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) => panic!(
                "Externe Testdaten fehlen ({}: {error}). Bitte `cargo run -p testdata` ausführen.",
                path.display()
            ),
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                work.push(path);
            } else if path.extension().is_some_and(|extension| extension == "js")
                && !path.to_string_lossy().contains("_FIXTURE")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Alle Dateien der Harness, einmal gelesen.
fn harness_files() -> BTreeMap<String, String> {
    testdata::files(SOURCE, "harness", "js")
        .into_iter()
        .map(|name| {
            let text = testdata::read(SOURCE, &format!("harness/{name}"));
            (name, text)
        })
        .collect()
}

/// Die Harness eines Tests aus den gelesenen Dateien, wie
/// [`super::harness`].
fn harness_for(
    metadata: &Metadata,
    files: &BTreeMap<String, String>,
) -> Result<Vec<(String, String)>, String> {
    if metadata.raw() {
        return Ok(Vec::new());
    }
    ["assert.js", "sta.js"]
        .into_iter()
        .chain(metadata.is_async().then_some("doneprintHandle.js"))
        .map(str::to_owned)
        .chain(metadata.includes.iter().cloned())
        .map(|name| match files.get(&name) {
            Some(text) => Ok((name, text.clone())),
            None => Err(format!("Harness-Datei fehlt: {name}")),
        })
        .collect()
}

/// Der Grund eines Fehlschlags, kurz genug für die Übersicht.
fn reason(result: &Result<Outcome, String>) -> String {
    match result {
        Ok(outcome) => format!("{outcome:?}"),
        Err(message) => match message.strip_prefix("Harness ") {
            Some(rest) => format!(
                "Harness: {}",
                rest.split_once(": ").map_or(rest, |(_, outcome)| outcome)
            ),
            None => message.clone(),
        },
    }
}

fn check_file(
    file: &Path,
    proposals: &BTreeSet<String>,
    harness: &BTreeMap<String, String>,
    panics: &mut Vec<String>,
    relative: &str,
) -> FileResult {
    let Ok(source) = String::from_utf8(std::fs::read(file).unwrap_or_default()) else {
        return FileResult::Ran(vec![(Variant::File, Some("kein UTF-8".to_owned()))]);
    };
    let metadata = match Metadata::read(&source) {
        Ok(metadata) => metadata,
        Err(rejected) => {
            if rejected
                .features
                .iter()
                .any(|feature| proposals.contains(feature))
            {
                return FileResult::Skipped;
            }
            let reason = format!("Metadaten: {}", rejected.reason);
            return FileResult::Ran(vec![(Variant::File, Some(reason))]);
        }
    };
    if metadata
        .features
        .iter()
        .any(|feature| proposals.contains(feature))
    {
        return FileResult::Skipped;
    }
    let files = match harness_for(&metadata, harness) {
        Ok(files) => files,
        Err(reason) => return FileResult::Ran(vec![(Variant::File, Some(reason))]),
    };
    let wanted = expected(metadata.negative);
    let mut variants = Vec::new();
    for mode in metadata.modes() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            run(&source, &metadata, mode, &files, false)
        }));
        let failure = match result {
            Ok(result) if result == Ok(wanted) => None,
            Ok(result) => Some(reason(&result)),
            Err(_) => {
                panics.push(format!("{relative} ({mode:?})"));
                Some("Panik".to_owned())
            }
        };
        variants.push((Variant::of(mode), failure));
    }
    FileResult::Ran(variants)
}

/// Die Datei der erwarteten Fehlschläge zu diesen Ergebnissen: eine Zeile
/// je Datei, deren Varianten alle scheitern, sonst eine je gescheiterter
/// Variante.
fn render(suite: &Suite, results: &[(String, FileResult)], summary: &str) -> String {
    let mut text = format!(
        "# Erwartete Fehlschläge der Test262-Dateien unter {}/, Pfade\n\
         # relativ dazu. Ohne Modus scheitern alle Varianten der Datei, sonst\n\
         # nur der genannte. Erzeugt von script/test262/language.rs; bei einer\n\
         # Abweichung schreibt der Test die neue Fassung nach\n\
         # target/test262/{}.\n",
        suite.directory, suite.file
    );
    let _ = writeln!(text, "# {summary}");
    for (path, result) in results {
        let FileResult::Ran(variants) = result else {
            continue;
        };
        let failed: Vec<Variant> = variants
            .iter()
            .filter(|(_, failure)| failure.is_some())
            .map(|&(variant, _)| variant)
            .collect();
        if failed.is_empty() {
            continue;
        }
        if failed.len() == variants.len() {
            let _ = writeln!(text, "{path}");
        } else {
            for variant in failed {
                let _ = writeln!(text, "{path} {}", variant.name());
            }
        }
    }
    text
}

/// Die Varianten, die eine Fassung der Datei als gescheitert nennt; eine
/// Zeile ohne Modus steht für alle Varianten aus `results`.
fn failing(
    text: &str,
    variants: &BTreeMap<&str, Vec<Variant>>,
) -> Result<BTreeSet<(String, Variant)>, String> {
    let mut set = BTreeSet::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (path, mode) = match line.split_once(' ') {
            Some((path, "sloppy")) => (path, Some(Variant::Sloppy)),
            Some((path, "strict")) => (path, Some(Variant::Strict)),
            Some(_) => return Err(format!("unbekannte Zeile: {line}")),
            None => (line, None),
        };
        let Some(known) = variants.get(path) else {
            return Err(format!("keine ausgeführte Datei: {path}"));
        };
        match mode {
            Some(mode) => {
                set.insert((path.to_owned(), mode));
            }
            None => set.extend(known.iter().map(|&variant| (path.to_owned(), variant))),
        }
    }
    Ok(set)
}

#[test]
fn test262_language() {
    check_suite(&LANGUAGE);
}

#[test]
fn test262_built_ins() {
    check_suite(&BUILT_INS);
}

fn check_suite(suite: &Suite) {
    let root = testdata::root().join(SOURCE);
    let base = root.join(suite.directory);
    let mut files = if suite.parts.is_empty() {
        test_files(&base)
    } else {
        suite
            .parts
            .iter()
            .flat_map(|part| test_files(&base.join(part)))
            .collect()
    };
    files.sort();
    let proposals = proposals();
    let harness = harness_files();
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let mut results: Vec<(usize, String, FileResult)> = Vec::new();
    let mut panics = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let (files, base, proposals, harness, next) =
                    (&files, &base, &proposals, &harness, &next);
                scope.spawn(move || {
                    let mut done = Vec::new();
                    let mut panics = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(file) = files.get(index) else {
                            break;
                        };
                        let relative = file
                            .strip_prefix(base)
                            .unwrap_or(file)
                            .to_string_lossy()
                            .replace('\\', "/");
                        let result = check_file(file, proposals, harness, &mut panics, &relative);
                        done.push((index, relative, result));
                    }
                    (done, panics)
                })
            })
            .collect();
        for worker in workers {
            match worker.join() {
                Ok((done, found)) => {
                    results.extend(done);
                    panics.extend(found);
                }
                Err(_) => panics.push("ein Faden ist abgebrochen".to_owned()),
            }
        }
    });
    results.sort_by_key(|(index, _, _)| *index);
    let results: Vec<(String, FileResult)> = results
        .into_iter()
        .map(|(_, path, result)| (path, result))
        .collect();

    let mut skipped = 0usize;
    let mut ran = 0usize;
    let mut passed_files = 0usize;
    let mut variant_count = 0usize;
    let mut passed_variants = 0usize;
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut variants: BTreeMap<&str, Vec<Variant>> = BTreeMap::new();
    let mut actual = BTreeSet::new();
    for (path, result) in &results {
        let FileResult::Ran(list) = result else {
            skipped += 1;
            continue;
        };
        ran += 1;
        variant_count += list.len();
        if list.iter().all(|(_, failure)| failure.is_none()) {
            passed_files += 1;
        }
        variants.insert(path, list.iter().map(|&(variant, _)| variant).collect());
        for (variant, failure) in list {
            match failure {
                None => passed_variants += 1,
                Some(reason) => {
                    *reasons.entry(reason.clone()).or_default() += 1;
                    actual.insert((path.clone(), *variant));
                }
            }
        }
    }
    let summary = format!(
        "{} Dateien, {skipped} Vorschläge übersprungen; bestanden {passed_files} von {ran} \
         Dateien und {passed_variants} von {variant_count} Varianten",
        files.len()
    );
    println!("Test262 {}/: {summary}", suite.directory);
    let mut ranked: Vec<_> = reasons.iter().collect();
    ranked.sort_by(|left, right| right.1.cmp(left.1).then(left.0.cmp(right.0)));
    for (reason, count) in ranked {
        println!("{count:>7}  {reason}");
    }
    assert!(
        panics.is_empty(),
        "Panik der Engine:\n{}",
        panics.join("\n")
    );
    // Die feste Auswahl besteht auch hier.
    for file in selection().into_iter().filter(|_| suite.selection) {
        let path = file.strip_prefix("test/language/").unwrap_or(file);
        assert!(
            variants.contains_key(path)
                && !actual.iter().any(|(failed, _)| failed.as_str() == path),
            "Datei der festen Auswahl besteht nicht: {file}"
        );
    }

    let rendered = render(suite, &results, &summary);
    if rendered == suite.expected {
        return;
    }
    let out = testdata::root()
        .parent()
        .map(|target| target.join("test262"))
        .unwrap_or_default();
    let written = std::fs::create_dir_all(&out)
        .and_then(|()| std::fs::write(out.join(suite.file), &rendered));
    let expected_set = failing(suite.expected, &variants);
    let mut report = String::new();
    match expected_set {
        Ok(expected_set) => {
            for (path, variant) in actual.difference(&expected_set) {
                let _ = writeln!(report, "Rückschritt: {path} {}", variant.name());
            }
            for (path, variant) in expected_set.difference(&actual) {
                let _ = writeln!(report, "unerwarteter Erfolg: {path} {}", variant.name());
            }
        }
        Err(error) => {
            let _ = writeln!(report, "{}: {error}", suite.file);
        }
    }
    if report.is_empty() {
        report.push_str("nur die Zahlen im Kopf weichen ab\n");
    }
    panic!(
        "Test262 {}/ weicht von {} ab ({}):\n{report}",
        suite.directory,
        suite.file,
        match written {
            Ok(()) => format!("neue Fassung in {}", out.join(suite.file).display()),
            Err(error) => format!("neue Fassung nicht geschrieben: {error}"),
        }
    );
}

#[test]
fn expected_failures_round_trip_through_the_file() {
    let results = vec![
        (
            "a.js".to_owned(),
            FileResult::Ran(vec![
                (Variant::Sloppy, Some("Unsupported".to_owned())),
                (Variant::Strict, Some("Unsupported".to_owned())),
            ]),
        ),
        (
            "b.js".to_owned(),
            FileResult::Ran(vec![
                (Variant::Sloppy, None),
                (Variant::Strict, Some("Test262Error".to_owned())),
            ]),
        ),
        (
            "c.js".to_owned(),
            FileResult::Ran(vec![(Variant::Sloppy, None), (Variant::Strict, None)]),
        ),
        (
            "d.js".to_owned(),
            FileResult::Ran(vec![(Variant::File, Some("Metadaten".to_owned()))]),
        ),
        ("e.js".to_owned(), FileResult::Skipped),
    ];
    let text = render(&LANGUAGE, &results, "Zahlen");
    let entries: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
    assert_eq!(entries, ["a.js", "b.js strict", "d.js"]);
    assert!(text.contains("# Zahlen\n"));
    let variants: BTreeMap<&str, Vec<Variant>> = [
        ("a.js", vec![Variant::Sloppy, Variant::Strict]),
        ("b.js", vec![Variant::Sloppy, Variant::Strict]),
        ("c.js", vec![Variant::Sloppy, Variant::Strict]),
        ("d.js", vec![Variant::File]),
    ]
    .into_iter()
    .collect();
    let set = failing(&text, &variants).expect("lesbar");
    let expected: BTreeSet<(String, Variant)> = [
        ("a.js", Variant::Sloppy),
        ("a.js", Variant::Strict),
        ("b.js", Variant::Strict),
        ("d.js", Variant::File),
    ]
    .into_iter()
    .map(|(path, variant)| (path.to_owned(), variant))
    .collect();
    assert_eq!(set, expected);
    for bad in ["x.js", "a.js module", "e.js"] {
        assert!(failing(bad, &variants).is_err(), "{bad}");
    }
}
