//! Misst, was die Vorprüfung spart und kostet, über Dateien aus Test262:
//!
//! ```sh
//! cargo test --release -p mb-js --lib -- --ignored measure_pre_check --nocapture
//! ```
//!
//! Für jede Einstellung stehen dort die Bytes der Tabellen des Baums nach
//! dem Lesen ([`Ast::table_bytes`]), die Zeit für das Lesen und die Zeit,
//! alle vorgeprüften Rümpfe nachzulesen. Das Nachlesen aller Rümpfe ist der
//! ungünstigste Fall: Jede Funktion wird aufgerufen.

use std::time::{Duration, Instant};

use super::*;
use crate::parser::parse_script;

/// Einzeln gemessene Dateien, die Bibliotheken am nächsten kommen.
const FILES: [(&str, &str); 4] = [
    ("test262", "harness/testIntl.js"),
    ("test262", "harness/temporalHelpers.js"),
    ("test262", "harness/propertyHelper.js"),
    ("wpt", "dom/nodes/selectors.js"),
];

/// So oft wird jede einzelne Datei gelesen; es zählt die kürzeste Zeit.
const REPEAT: usize = 7;

#[derive(Clone, Copy, Default)]
struct Sample {
    bytes: usize,
    functions: usize,
    lazy: usize,
    parse: Duration,
    expand: Duration,
}

impl Sample {
    fn add(&mut self, other: Sample) {
        self.bytes += other.bytes;
        self.functions += other.functions;
        self.lazy += other.lazy;
        self.parse += other.parse;
        self.expand += other.expand;
    }
}

fn settings() -> Vec<(String, Option<PreCheck>)> {
    let mut settings = vec![("ganz".to_owned(), None)];
    for min_nodes in [0, 8, 16, 32, 64, 128] {
        let pre_check = PreCheck {
            min_nodes,
            ..PreCheck::DEFAULT
        };
        settings.push((format!("ab {min_nodes} Knoten"), Some(pre_check)));
    }
    settings
}

fn lazy_functions(ast: &Ast) -> usize {
    (0..ast.function_count())
        .filter(|&index| {
            u32::try_from(index).is_ok_and(|index| ast.is_lazy(FunctionId::from_raw(index)))
        })
        .count()
}

/// Liest `source` einmal mit `pre_check`; `None` ohne Vorprüfung oder
/// wenn das Script nicht gültig ist.
fn sample(source: &[u16], pre_check: Option<PreCheck>) -> Option<Sample> {
    let started = Instant::now();
    let mut ast = match pre_check {
        Some(pre_check) => parse_script_pre_checked(source, pre_check),
        None => parse_script(source),
    }
    .ok()?;
    let parse = started.elapsed();
    let bytes = ast.table_bytes();
    let functions = ast.function_count();
    let lazy = lazy_functions(&ast);
    let started = Instant::now();
    ast.expand_all().ok()?;
    Some(Sample {
        bytes,
        functions,
        lazy,
        parse,
        expand: started.elapsed(),
    })
}

fn report(label: &str, sample: Sample, full: Sample) {
    let percent = |part: usize, whole: usize| {
        if whole == 0 {
            0.0
        } else {
            part as f64 * 100.0 / whole as f64
        }
    };
    println!(
        "  {label:<14} {:>10} Bytes ({:>5.1} %)  {:>6} von {:>6} vorgeprüft  \
         lesen {:>8.2} ms ({:>5.1} %)  alles nachlesen {:>8.2} ms",
        sample.bytes,
        percent(sample.bytes, full.bytes),
        sample.lazy,
        sample.functions,
        sample.parse.as_secs_f64() * 1000.0,
        percent(
            sample.parse.as_nanos().try_into().unwrap_or(usize::MAX),
            full.parse.as_nanos().try_into().unwrap_or(usize::MAX)
        ),
        sample.expand.as_secs_f64() * 1000.0,
    );
}

#[test]
#[ignore = "Messung; gibt nur Zahlen aus"]
fn measure_pre_check() {
    let settings = settings();
    for (collection, path) in FILES {
        let source: Vec<u16> = testdata::read(collection, path).encode_utf16().collect();
        println!("{collection}/{path}: {} Codeeinheiten", source.len());
        let mut full = Sample::default();
        for (label, pre_check) in &settings {
            let mut best: Option<Sample> = None;
            for _ in 0..REPEAT {
                let Some(sample) = sample(&source, *pre_check) else {
                    break;
                };
                best = Some(match best {
                    Some(best) if best.parse <= sample.parse => Sample {
                        expand: best.expand.min(sample.expand),
                        ..best
                    },
                    Some(best) => Sample {
                        expand: best.expand.min(sample.expand),
                        ..sample
                    },
                    None => sample,
                });
            }
            let Some(best) = best else {
                println!("  {label}: kein gültiges Script");
                continue;
            };
            if pre_check.is_none() {
                full = best;
            }
            report(label, best, full);
        }
    }
    // Alle Tests als Scripts, die sich ohne Vorprüfung lesen lassen.
    let root = testdata::root().join("test262");
    let mut sources = Vec::new();
    for directory in ["test/language", "test/built-ins", "test/annexB"] {
        for file in crate::parser::test262::test_files(&root, directory) {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let source: Vec<u16> = text.encode_utf16().collect();
            if parse_script(&source).is_ok() {
                sources.push(source);
            }
        }
    }
    let units: usize = sources.iter().map(Vec::len).sum();
    println!(
        "Test262, {} Dateien als Script, {units} Codeeinheiten",
        sources.len()
    );
    let mut full = Sample::default();
    for (label, pre_check) in &settings {
        // Drei Durchgänge; es zählen die kürzesten Zeiten.
        let mut best: Option<Sample> = None;
        for _ in 0..3 {
            let mut total = Sample::default();
            for source in &sources {
                if let Some(sample) = sample(source, *pre_check) {
                    total.add(sample);
                }
            }
            best = Some(match best {
                Some(best) => Sample {
                    parse: best.parse.min(total.parse),
                    expand: best.expand.min(total.expand),
                    ..total
                },
                None => total,
            });
        }
        let total = best.unwrap_or_default();
        if pre_check.is_none() {
            full = total;
        }
        report(label, total, full);
    }
}
