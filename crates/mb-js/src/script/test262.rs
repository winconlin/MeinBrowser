//! Erste feste Test262-Auswahl für die Ausführung, nicht nur die Syntax.
//! Grenzen und die vorübergehenden Harness-Anpassungen stehen in
//! `docs/m2-test262-ausfuehrung.md`.

use std::collections::BTreeSet;
use std::rc::Rc;

use crate::agent::{Abrupt, Agent, Budget, ErrorKind, Exception};
use crate::compiler::{CompileError, compile_script};
use crate::heap::Options;
use crate::parser::parse_script;
use crate::realm::RealmId;
use crate::script::ScriptError;

mod metadata;
#[cfg(test)]
mod tests;

use metadata::{Metadata, Mode, Negative};

const STEPS: u64 = 100_000;
const HEAP_BYTES: usize = 8 << 20;
const ADAPTATION: &str = include_str!("test262/same-value.js");
const SELECTION: &str = include_str!("test262/runtime.txt");

/// Ergebnisse bleiben ohne JavaScript-Aufrufe beobachtbar. Insbesondere
/// wird kein `constructor` oder `name` eines geworfenen Objekts gelesen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Normal,
    Syntax,
    Error(ErrorKind),
    ThrownValue,
    Unsupported,
    CompileLimit,
    Host(Abrupt),
}

fn evaluate(agent: &mut Agent, realm: RealmId, source: &str, budget: &Rc<Budget>) -> Outcome {
    let source: Vec<u16> = source.encode_utf16().collect();
    let outcome = agent.scope(|agent, scope| {
        match agent.evaluate_script_within(scope, realm, &source, Rc::clone(budget)) {
            Ok(_) => Outcome::Normal,
            Err(ScriptError::Syntax(_)) => Outcome::Syntax,
            Err(ScriptError::Unsupported(_)) => Outcome::Unsupported,
            Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
                Some(Exception::Error(kind)) => Outcome::Error(kind),
                Some(Exception::Value(_)) => Outcome::ThrownValue,
                None => panic!("Throw ohne Ausnahme"),
            },
            Err(ScriptError::Abrupt(abrupt)) => Outcome::Host(abrupt),
        }
    });
    assert_eq!(agent.heap_mut().push_registers(0), Some(0), "Register frei");
    agent.scope(|agent, scope| {
        assert!(agent.take_exception(scope).is_none(), "keine alte Ausnahme");
    });
    agent.heap().verify().expect("gültiger Heap nach Script");
    outcome
}

fn expected(negative: Option<Negative>) -> Outcome {
    match negative {
        None => Outcome::Normal,
        Some(Negative::Parse) => Outcome::Syntax,
        Some(Negative::Runtime(kind)) => Outcome::Error(kind),
    }
}

/// Ein Agent und Realm je Variante; Harness und Test teilen eine Quote.
/// Setupfehler können niemals einen negativen Test erfüllen.
fn run(
    source: &str,
    metadata: &Metadata,
    mode: Mode,
    harness: &[(String, String)],
    stress: bool,
) -> Result<Outcome, String> {
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress,
        verify: true,
    })
    .map_err(|error| format!("Agent: {error:?}"))?;
    let realm = agent
        .create_realm()
        .map_err(|error| format!("Realm: {error:?}"))?;
    let budget = Rc::new(Budget::new(STEPS));
    if !metadata.raw() {
        for (name, text) in harness {
            let result = evaluate(&mut agent, realm, text, &budget);
            if result != Outcome::Normal {
                return Err(format!("Harness {name}: {result:?}"));
            }
        }
    }
    let text = match mode {
        Mode::Sloppy => source.to_owned(),
        Mode::Strict => format!("\"use strict\";\n{source}"),
    };
    // evaluate_script meldet ein Größenlimit des äußeren Compilers als
    // RangeError. Es darf keinen erwarteten *Laufzeit*-RangeError erfüllen.
    // Deshalb bei negativen Laufzeittests die frühe Phase separat prüfen.
    if matches!(metadata.negative, Some(Negative::Runtime(_))) {
        let units: Vec<u16> = text.encode_utf16().collect();
        let ast = match parse_script(&units) {
            Ok(ast) => ast,
            Err(_) => return Ok(Outcome::Syntax),
        };
        match compile_script(&ast) {
            Ok(_) => {}
            Err(CompileError::Unsupported(_)) => return Ok(Outcome::Unsupported),
            Err(CompileError::TooLarge) => return Ok(Outcome::CompileLimit),
        }
    }
    let result = evaluate(&mut agent, realm, &text, &budget);
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap nach Sammeln");
    Ok(result)
}

fn harness(metadata: &Metadata) -> Vec<(String, String)> {
    if metadata.raw() {
        return Vec::new();
    }
    let mut files: Vec<_> = ["assert.js", "sta.js"]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                testdata::read("test262", &format!("harness/{name}")),
            )
        })
        .collect();
    files.push(("M2 SameValue-Anpassung".to_owned(), ADAPTATION.to_owned()));
    for name in &metadata.includes {
        files.push((
            name.clone(),
            testdata::read("test262", &format!("harness/{name}")),
        ));
    }
    files
}

/// Keine implizite Erweiterung durch Verzeichnisabfragen: Jeder neue Fall
/// wird mit Pfad und Anforderungen in der eingecheckten Liste geprüft.
fn selection() -> Vec<&'static str> {
    let files: Vec<_> = SELECTION
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let mut seen = BTreeSet::new();
    for file in &files {
        assert!(
            file.starts_with("test/language/")
                && file.ends_with(".js")
                && !file.contains("_FIXTURE")
                && metadata::relative_path(file),
            "Testpfad: {file}"
        );
        assert!(seen.insert(*file), "doppelter Testpfad: {file}");
    }
    assert!(!files.is_empty(), "keine Laufzeittests gewählt");
    files
}

fn proposals() -> BTreeSet<String> {
    let text = testdata::read("test262", "features.txt");
    let mut proposed = false;
    let mut names = BTreeSet::new();
    for line in text.lines().map(str::trim) {
        match line {
            "## Proposed language features" => proposed = true,
            "## Standard language features" => proposed = false,
            _ => {}
        }
        let name = line.split('#').next().unwrap_or_default().trim();
        if proposed && !name.is_empty() {
            names.insert(name.to_owned());
        }
    }
    names
}

#[test]
fn test262_execution() {
    let mut failures = Vec::new();
    let mut runs = 0;
    let mut runtime_negative = 0;
    let files = selection();
    let proposals = proposals();
    for file in &files {
        let source = testdata::read("test262", file);
        let metadata = Metadata::read(&source).unwrap_or_else(|error| panic!("{file}: {error}"));
        assert!(
            metadata
                .features
                .iter()
                .all(|feature| !proposals.contains(feature)),
            "Vorschläge gehören nicht in diese M2-Auswahl: {file}"
        );
        // Syntax-Negativtests zählen bereits in parser/test262.rs. Diese
        // Auswahl soll Ausführung messen, keine Parserzahlen aufaddieren.
        assert_ne!(metadata.negative, Some(Negative::Parse), "{file}");
        let harness = harness(&metadata);
        runtime_negative += usize::from(metadata.negative.is_some());
        for mode in metadata.modes() {
            for stress in [false, true] {
                runs += 1;
                let result = run(&source, &metadata, mode, &harness, stress);
                if result != Ok(expected(metadata.negative)) {
                    failures.push(format!("{file} ({mode:?}, stress={stress}): {result:?}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} Fehlschläge in {runs} Läufen:\n{}",
        failures.len(),
        failures.join("\n")
    );
    println!(
        "Test262-Ausführung: {} Dateien, {runs} Läufe mit zwei GC-Modi, \
         {runtime_negative} Dateien mit erwartetem Laufzeitfehler",
        files.len()
    );
}
