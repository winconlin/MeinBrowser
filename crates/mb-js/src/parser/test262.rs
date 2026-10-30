//! Die Syntaxtests aus Test262 (`tools/testdata`): alle Tests unter
//! `test/language/` und `test/annexB/language/`, nur gelesen, nicht
//! ausgeführt.
//!
//! Jeder Test wird so gelesen, wie `INTERPRETING.md` es verlangt: mit
//! `module` als Modul, mit `raw` unverändert, sonst als Script außerhalb des
//! strikten Modus und mit vorangestelltem `"use strict";` (nur eines davon
//! mit `noStrict` und `onlyStrict`). Tests mit `negative` in der Phase
//! `parse` müssen einen Fehler ergeben, alle anderen keinen; Fehler aus
//! späteren Phasen sieht der Parser nicht. Tests für Vorschläge, die noch
//! nicht in der Spezifikation stehen (der Abschnitt „Proposed language
//! features“ in `features.txt`), laufen nicht.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{parse_module, parse_script};

const SOURCE: &str = "test262";
const DIRECTORIES: [&str; 2] = ["test/language", "test/annexB/language"];

/// Was ein Test über sich sagt.
#[derive(Debug, Default)]
struct Metadata {
    flags: Vec<String>,
    features: Vec<String>,
    /// Die Phase aus `negative`.
    negative: Option<String>,
}

/// Wie ein Test gelesen wird.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Sloppy,
    Strict,
    Module,
}

/// Eine Liste in YAML, in einer Zeile (`[a, b]`).
fn inline_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Liest die Teile des YAML-Kopfes zwischen `/*---` und `---*/`, die der
/// Lauf braucht. Listen stehen in einer Zeile oder als Zeilen mit `-`.
fn metadata(source: &str) -> Metadata {
    let mut metadata = Metadata::default();
    let Some((_, rest)) = source.split_once("/*---") else {
        return metadata;
    };
    let Some((yaml, _)) = rest.split_once("---*/") else {
        return metadata;
    };
    let mut current = "";
    for line in yaml.lines() {
        let indented = line.starts_with([' ', '\t']);
        let trimmed = line.trim();
        if !indented {
            let Some((key, value)) = trimmed.split_once(':') else {
                current = "";
                continue;
            };
            current = match key {
                "flags" | "features" | "negative" => key,
                _ => "",
            };
            let items = inline_list(value);
            match key {
                "flags" => metadata.flags.extend(items),
                "features" => metadata.features.extend(items),
                _ => {}
            }
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            match current {
                "flags" => metadata.flags.push(item.trim().to_owned()),
                "features" => metadata.features.push(item.trim().to_owned()),
                _ => {}
            }
        } else if current == "negative"
            && let Some(phase) = trimmed.strip_prefix("phase:")
        {
            metadata.negative = Some(phase.trim().to_owned());
        }
    }
    metadata
}

/// Die Vorschläge aus `features.txt`.
fn proposals() -> BTreeSet<String> {
    let text = testdata::read(SOURCE, "features.txt");
    let mut proposals = BTreeSet::new();
    let mut proposed = false;
    for line in text.lines() {
        let line = line.trim();
        // Nur diese beiden Überschriften trennen die Abschnitte, andere
        // Zeilen mit `#` sind Kommentare.
        match line {
            "## Proposed language features" => proposed = true,
            "## Standard language features" => proposed = false,
            _ => {}
        }
        let name = line.split('#').next().unwrap_or_default().trim();
        if proposed && !name.is_empty() {
            proposals.insert(name.to_owned());
        }
    }
    proposals
}

/// Alle Testdateien unter `directory`, ohne Rekursion und sortiert.
fn test_files(root: &Path, directory: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut work = vec![root.join(directory)];
    while let Some(path) = work.pop() {
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) => panic!(
                "Externe Testdaten fehlen ({}: {error}). Bitte `cargo run -p testdata` ausführen.",
                path.display()
            ),
        };
        for entry in entries.filter_map(std::result::Result::ok) {
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

/// Ordner, in denen Tests mit `negative` noch keinen Fehler ergeben, mit
/// Grund. Andere Fehlschläge zählen auch hier.
const MISSING_ERRORS: [(&str, &str); 1] = [(
    "language/literals/regexp/",
    "Die Muster regulärer Ausdrücke prüft der Parser noch nicht, nur die Flags.",
)];

/// Was vor einen Test im strikten Modus kommt.
const USE_STRICT: &str = "\"use strict\";\n";

/// Die Ergebnisse eines Teils der Tests.
#[derive(Default)]
struct Tally {
    runs: usize,
    negative: usize,
    skipped: usize,
    failures: Vec<String>,
    /// Wie viele fehlende Fehler jeder Eintrag in MISSING_ERRORS erklärt.
    missing: [usize; MISSING_ERRORS.len()],
}

impl Tally {
    fn add(&mut self, other: Tally) {
        self.runs += other.runs;
        self.negative += other.negative;
        self.skipped += other.skipped;
        self.failures.extend(other.failures);
        for (count, other) in self.missing.iter_mut().zip(other.missing) {
            *count += other;
        }
    }

    /// Liest einen Test in allen Modi, die er verlangt.
    fn check(&mut self, root: &Path, file: &Path, proposals: &BTreeSet<String>) {
        let relative = file
            .strip_prefix(root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        let relative = relative
            .strip_prefix("test/")
            .unwrap_or(&relative)
            .to_owned();
        let bytes = std::fs::read(file).unwrap_or_default();
        let Ok(text) = String::from_utf8(bytes) else {
            self.failures.push(format!("{relative}: kein UTF-8"));
            return;
        };
        let metadata = metadata(&text);
        if metadata
            .features
            .iter()
            .any(|feature| proposals.contains(feature))
        {
            self.skipped += 1;
            return;
        }
        let flag = |name: &str| metadata.flags.iter().any(|flag| flag == name);
        let modes: &[Mode] = if flag("module") {
            &[Mode::Module]
        } else if flag("raw") || flag("noStrict") {
            &[Mode::Sloppy]
        } else if flag("onlyStrict") {
            &[Mode::Strict]
        } else {
            &[Mode::Sloppy, Mode::Strict]
        };
        let expect_error = metadata.negative.as_deref() == Some("parse");
        if expect_error {
            self.negative += 1;
        }
        for &mode in modes {
            self.runs += 1;
            let prefix = if mode == Mode::Strict { USE_STRICT } else { "" };
            let source: Vec<u16> = prefix.encode_utf16().chain(text.encode_utf16()).collect();
            let result = match mode {
                Mode::Module => parse_module(&source).map(|_| ()),
                Mode::Sloppy | Mode::Strict => parse_script(&source).map(|_| ()),
            };
            match (result, expect_error) {
                (Ok(()), true) => {
                    let explained = MISSING_ERRORS
                        .iter()
                        .position(|(directory, _)| relative.starts_with(directory));
                    match explained.and_then(|index| self.missing.get_mut(index)) {
                        Some(count) => *count += 1,
                        None => self.failures.push(format!(
                            "{relative} ({mode:?}): kein Fehler, erwartet SyntaxError"
                        )),
                    }
                }
                (Err(error), false) => {
                    let lines = source
                        .get(..error.position)
                        .unwrap_or_default()
                        .iter()
                        .filter(|unit| **unit == 0x0A)
                        .count();
                    let line = (lines + 1).saturating_sub(prefix.matches('\n').count());
                    self.failures.push(format!(
                        "{relative} ({mode:?}): {:?} in Zeile {line}",
                        error.kind
                    ));
                }
                _ => {}
            }
        }
    }
}

#[test]
fn test262_syntax() {
    let root = testdata::root().join(SOURCE);
    let proposals = proposals();
    let mut files = Vec::new();
    for directory in DIRECTORIES {
        files.extend(test_files(&root, directory));
    }
    // Die Dateien in Teilen auf mehrere Fäden verteilt.
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let chunk = files.len().div_ceil(threads).max(1);
    let mut tally = Tally::default();
    std::thread::scope(|scope| {
        let workers: Vec<_> = files
            .chunks(chunk)
            .map(|part| {
                let (root, proposals) = (&root, &proposals);
                scope.spawn(move || {
                    let mut tally = Tally::default();
                    for file in part {
                        tally.check(root, file, proposals);
                    }
                    tally
                })
            })
            .collect();
        for worker in workers {
            match worker.join() {
                Ok(part) => tally.add(part),
                Err(_) => tally.failures.push("ein Faden ist abgebrochen".to_owned()),
            }
        }
    });
    for ((directory, _), count) in MISSING_ERRORS.iter().zip(tally.missing) {
        if count == 0 {
            tally.failures.push(format!(
                "{directory}: keine fehlenden Fehler mehr, aus MISSING_ERRORS streichen"
            ));
        }
    }
    tally.failures.sort();
    assert!(
        tally.failures.is_empty(),
        "{} Fehlschläge in {} Läufen:\n{}",
        tally.failures.len(),
        tally.runs,
        tally.failures.join("\n")
    );
    assert_eq!(
        (
            files.len(),
            tally.skipped,
            tally.negative,
            tally.runs,
            tally.missing
        ),
        (24673, 532, 4236, 44689, [334]),
        "Zahl der Fälle"
    );
}
