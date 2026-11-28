//! Lässt die Referenztests für M1 laufen.
//!
//! `cargo run -p testrunner --release -- [--out VERZEICHNIS] [--alle]
//! [MUSTER …]`: nur die Tests, deren Pfad eines der Muster enthält; mit
//! `--out` kommen Test und Referenz jedes gescheiterten Tests als PPM in das
//! Verzeichnis, mit `--alle` auch die der bestandenen.
//!
//! `cargo run -p testrunner --release -- --testharness [--wpt VERZEICHNIS]
//! [--erwartet DATEI] [MUSTER …]`: die Tests mit testharness.js in der
//! Shell (siehe `testharness.rs`), aus `target/testdata/wpt` oder dem
//! angegebenen Verzeichnis. Jede Datei, deren Harness nicht OK ist, und
//! jeder Untertest, der nicht besteht, kommt mit Meldung auf die Ausgabe;
//! mit `--erwartet` kommen die erwarteten Ergebnisse in die Datei.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use mb_layout::Fonts;
use mb_text::FontLibrary;
use testrunner::testharness::{self, HarnessStatus, Status};
use testrunner::{M1_REFTESTS, RefTest, Relation, differing_pixels, parse_list, render, to_ppm};

/// Der Stapel je Arbeitsfaden; Layout und Zeichnen arbeiten rekursiv.
const STACK: usize = 64 << 20;

/// Was ein Test ergab.
enum Outcome {
    /// Bestanden; `true`, wenn beide Flächen einfarbig sind. Dann zeigt
    /// der Test wahrscheinlich nicht, was er soll.
    Pass(bool),
    Fail(String),
}

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--testharness") {
        return testharness_main();
    }
    let mut out: Option<PathBuf> = None;
    let mut all = false;
    let mut patterns = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--out" {
            out = args.next().map(PathBuf::from);
        } else if arg == "--alle" {
            all = true;
        } else {
            patterns.push(arg);
        }
    }
    let list = match parse_list(M1_REFTESTS) {
        Ok(list) => list,
        Err(error) => {
            eprintln!("Liste: {error}");
            return ExitCode::FAILURE;
        }
    };
    let tests: Vec<RefTest> = list
        .into_iter()
        .filter(|test| patterns.is_empty() || patterns.iter().any(|p| test.test.contains(p)))
        .collect();
    if let Some(out) = &out
        && let Err(error) = std::fs::create_dir_all(out)
    {
        eprintln!("{}: {error}", out.display());
        return ExitCode::FAILURE;
    }
    let mut library = FontLibrary::new(Box::new(mb_platform::fonts::read_font));
    for file in mb_platform::fonts::font_files() {
        library.add_file(file);
    }
    let wpt = testdata::root().join("wpt");
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Outcome)>> = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let builder = std::thread::Builder::new().stack_size(STACK);
            let spawned = builder.spawn_scoped(scope, || {
                let fonts = Fonts::new(&library);
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(test) = tests.get(index) else {
                        break;
                    };
                    let outcome = run(test, &wpt, &fonts, out.as_deref(), all);
                    results
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push((index, outcome));
                }
            });
            if let Err(error) = spawned {
                eprintln!("Faden: {error}");
            }
        }
    });
    // Alle Arbeitsfäden sind fertig. Während ihrer Textformung bestehen
    // zeitweise nur FaceIds; darum nicht zwischen parallelen Tests trimmen.
    library.trim_cache();
    let mut results = results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    results.sort_by_key(|(index, _)| *index);
    let mut passed = 0;
    let mut uniform = 0;
    for (index, outcome) in &results {
        match outcome {
            Outcome::Pass(blank) => {
                passed += 1;
                if *blank {
                    uniform += 1;
                    let name = tests.get(*index).map_or("?", |test| test.test.as_str());
                    println!("EINFARBIG {name}");
                }
            }
            Outcome::Fail(reason) => {
                let name = tests.get(*index).map_or("?", |test| test.test.as_str());
                println!("FAIL {name}: {reason}");
            }
        }
    }
    println!(
        "{passed} von {} bestanden, davon {uniform} mit einfarbigen Flächen",
        tests.len()
    );
    if passed == tests.len() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run(
    test: &RefTest,
    wpt: &std::path::Path,
    fonts: &Fonts<'_>,
    out: Option<&std::path::Path>,
    all: bool,
) -> Outcome {
    let Some(actual) = render(&wpt.join(&test.test), fonts) else {
        return Outcome::Fail("Test lässt sich nicht laden".into());
    };
    let Some(expected) = render(&wpt.join(&test.reference), fonts) else {
        return Outcome::Fail("Referenz lässt sich nicht laden".into());
    };
    let differing = differing_pixels(&actual, &expected);
    let pass = match test.relation {
        Relation::Match => differing == 0,
        Relation::Mismatch => differing > 0,
    };
    if let Some(out) = out
        && (all || !pass)
    {
        let stem = test.test.replace('/', "_");
        let written = std::fs::write(out.join(format!("{stem}.test.ppm")), to_ppm(&actual))
            .and_then(|()| std::fs::write(out.join(format!("{stem}.ref.ppm")), to_ppm(&expected)));
        if let Err(error) = written {
            eprintln!("{}: {error}", out.display());
        }
    }
    if pass {
        let single = |canvas: &mb_raster::Canvas| {
            let first = canvas.pixels().first().copied();
            canvas.pixels().iter().all(|&pixel| Some(pixel) == first)
        };
        return Outcome::Pass(single(&actual) && single(&expected));
    }
    match test.relation {
        Relation::Match => Outcome::Fail(format!("{differing} Pixel verschieden")),
        Relation::Mismatch => Outcome::Fail("gleich, sollte verschieden sein".into()),
    }
}

/// Die Tests mit testharness.js.
fn testharness_main() -> ExitCode {
    let mut wpt = testdata::root().join("wpt");
    let mut expectations: Option<PathBuf> = None;
    let mut patterns = Vec::new();
    let mut args = std::env::args().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--wpt" => wpt = args.next().map_or(wpt, PathBuf::from),
            "--erwartet" => expectations = args.next().map(PathBuf::from),
            _ => patterns.push(arg),
        }
    }
    let paths: Vec<String> = testharness::select(&wpt)
        .into_iter()
        .filter(|path| patterns.is_empty() || patterns.iter().any(|p| path.contains(p)))
        .collect();
    let results = testharness::run_all(&wpt, &paths);
    for (path, result) in &results {
        if result.harness != HarnessStatus::Ok {
            let message = result.message.as_deref().unwrap_or("");
            println!("{} {path}: {message}", result.harness.name());
        }
        for subtest in &result.subtests {
            if subtest.status != Status::Pass {
                let message = subtest.message.as_deref().unwrap_or("");
                println!("  {} {}: {message}", subtest.status.name(), subtest.name);
            }
        }
    }
    let rendered = testharness::render_expectations(&results);
    if let Some(summary) = rendered.lines().find(|line| line.contains(" Dateien, ")) {
        println!("{}", summary.trim_start_matches("# "));
    }
    if let Some(file) = expectations
        && let Err(error) = std::fs::write(&file, &rendered)
    {
        eprintln!("{}: {error}", file.display());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
