//! Die WPT-Tests zu `CSS.supports()` aus `css/css-conditional/js/`. Sie
//! laufen unverändert mit der Seite aus den Tests der Eigenschaften
//! (`properties/wpt.rs`).
//!
//! Nicht dabei ist `supports-at-rule.html`: Seine Tests haben keinen Namen,
//! den leitet testharness.js aus dem Quelltext ab, und das fehlt hier. Die
//! Fälle stehen in den Tests von `supports.rs`. Die übrigen Dateien prüfen
//! `CSSStyleDeclaration`, CSSOM-Regeln und `@supports` im Zusammenspiel
//! mit Elementen, die es hier noch nicht gibt.

use crate::properties::wpt::{Outcome, run_file};

const FILES: &[&str] = &[
    "css-conditional/js/CSS-supports-L3.html",
    "css-conditional/js/CSS-supports-L4.html",
    "css-conditional/js/CSS-supports-L5.html",
    "css-conditional/js/CSS-supports-details-content-pseudo-parsing.html",
    "css-conditional/js/CSS-supports-selector-detecting-invalid-in-logical-combinations.html",
];

const CUSTOM_PROPERTIES: &str = "Custom Properties und var() fehlen noch.";
const FONT_FACE: &str = "Ohne @font-face lassen sich Schriften aus Stylesheets nicht verwenden.";
const DETAILS_CONTENT: &str = "Das Pseudoelement ::details-content fehlt noch.";

/// Tests, die fehlschlagen dürfen und müssen: Datei, Name, was fehlt.
const GAPS: &[(&str, &str, &str)] = &[
    (
        "css-conditional/js/CSS-supports-L3.html",
        "Variable references always parse",
        CUSTOM_PROPERTIES,
    ),
    (
        "css-conditional/js/CSS-supports-L3.html",
        "Variable references in an unknown function always parse",
        CUSTOM_PROPERTIES,
    ),
    (
        "css-conditional/js/CSS-supports-L3.html",
        "one argument form succeeds for custom property",
        CUSTOM_PROPERTIES,
    ),
    (
        "css-conditional/js/CSS-supports-L5.html",
        "font-format() function accepts a known format",
        FONT_FACE,
    ),
    (
        "css-conditional/js/CSS-supports-L5.html",
        "font-tech() function accepts a known technology",
        FONT_FACE,
    ),
    (
        "css-conditional/js/CSS-supports-details-content-pseudo-parsing.html",
        "selector() function accepts ::details-content",
        DETAILS_CONTENT,
    ),
    (
        "css-conditional/js/CSS-supports-details-content-pseudo-parsing.html",
        "selector() function accepts ::details-content followed by ::before",
        DETAILS_CONTENT,
    ),
    (
        "css-conditional/js/CSS-supports-details-content-pseudo-parsing.html",
        "selector() function accepts ::details-content followed by ::first-line",
        DETAILS_CONTENT,
    ),
    (
        "css-conditional/js/CSS-supports-details-content-pseudo-parsing.html",
        "selector() function accepts ::details-content followed by a state pseudo-class",
        DETAILS_CONTENT,
    ),
    (
        "css-conditional/js/CSS-supports-details-content-pseudo-parsing.html",
        "selector() function accepts ::details-content followed by :lang()",
        DETAILS_CONTENT,
    ),
];

/// Was sich in einer Datei nicht ausführen lässt.
const UNSUPPORTED: &[(&str, &[&str])] = &[(
    "css-conditional/js/CSS-supports-L3.html",
    &["eigene Eigenschaften"],
)];

fn run_all() {
    let mut failures = Vec::new();
    let mut found = Vec::new();
    let mut passed = 0usize;
    let mut not_run = 0usize;
    for path in FILES {
        let result = run_file(path);
        for error in &result.errors {
            failures.push(format!("{path}: {error}"));
        }
        let expected: &[&str] = UNSUPPORTED
            .iter()
            .find(|(file, _)| file == path)
            .map_or(&[], |(_, reasons)| reasons);
        if result.unsupported != expected {
            failures.push(format!(
                "{path}: nicht ausführbar: {:?}, erwartet {expected:?}",
                result.unsupported
            ));
        }
        if result.subtests.is_empty() {
            failures.push(format!("{path}: keine Tests"));
        }
        for subtest in &result.subtests {
            let name = &subtest.name;
            let gap = GAPS
                .iter()
                .any(|(file, gap, _)| file == path && gap == name);
            match &subtest.outcome {
                Outcome::Unsupported(_) => not_run = not_run.saturating_add(1),
                Outcome::Pass if gap => {
                    failures.push(format!("{path}: {name}: besteht trotz der Lücke"));
                }
                Outcome::Pass => passed = passed.saturating_add(1),
                Outcome::Fail(_) if gap => found.push((*path, name.clone())),
                Outcome::Fail(message) => failures.push(format!("{path}: {name}: {message}")),
            }
        }
    }
    for (file, name, _) in GAPS {
        if !found
            .iter()
            .any(|(path, found)| path == file && found == name)
        {
            failures.push(format!("{file}: {name}: Lücke nicht gefunden"));
        }
    }
    eprintln!(
        "{} Dateien: {passed} bestanden, {} in Lücken, {not_run} nicht ausführbar, {} Fehler",
        FILES.len(),
        found.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} Fehler:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn css_supports() {
    // Der Interpreter ist rekursiv, siehe `properties/wpt.rs`.
    let worker = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run_all)
        .expect("Thread für die WPT-Tests");
    if let Err(panic) = worker.join() {
        std::panic::resume_unwind(panic);
    }
}
