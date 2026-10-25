//! Lässt die Referenztests für M1 laufen.
//!
//! `cargo run -p testrunner --release -- [--out VERZEICHNIS] [--alle]
//! [MUSTER …]`: nur die Tests, deren Pfad eines der Muster enthält; mit
//! `--out` kommen Test und Referenz jedes gescheiterten Tests als PPM in das
//! Verzeichnis, mit `--alle` auch die der bestandenen.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use mb_layout::Fonts;
use mb_text::FontLibrary;
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
    let mut library = FontLibrary::new(Box::new(|path| std::fs::read(path).ok()));
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
