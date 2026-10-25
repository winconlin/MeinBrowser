//! Externe Testsammlungen (siehe `docs/architektur.md`, Abschnitte 10 und 11).
//!
//! Sie liegen nicht im Repository, sondern werden in genau der Version, die
//! [`SOURCES`] festlegt, mit `cargo run -p testdata` nach `target/testdata/`
//! geholt. Tests lesen sie mit [`read`].

#![forbid(unsafe_code)]

use std::path::PathBuf;

/// Eine Testsammlung: ein Git-Repository in einem festen Commit, davon nur
/// die genannten Pfade.
pub struct Source {
    /// Name des Ordners unter `target/testdata/`.
    pub name: &'static str,
    pub repository: &'static str,
    pub commit: &'static str,
    /// Muster für `git sparse-checkout` ohne Kegelmodus, mit `/` am Anfang.
    pub paths: &'static [&'static str],
}

/// Alle Testsammlungen. Für einen neuen Stand den Commit ändern und
/// `cargo run -p testdata` ausführen.
pub const SOURCES: [Source; 5] = [
    Source {
        name: "wpt",
        repository: "https://github.com/web-platform-tests/wpt",
        commit: "c7902c0b13fb010a78126eb9d951e1f22c421013",
        // Die Tests für den HTML-Baumaufbau aus html5lib-tests werden seit
        // einiger Zeit in den WPT gepflegt.
        paths: &[
            "/url/resources/",
            "/html/syntax/parsing/resources/",
            "/dom/nodes/selectors.js",
            "/dom/nodes/ParentNode-querySelector-All-content.html",
            // Parsen und Serialisieren der Eigenschaften, siehe
            // `crates/mb-style/src/properties/wpt.rs`.
            "/css/css-backgrounds/parsing/",
            "/css/css-box/parsing/",
            "/css/css-cascade/parsing/",
            "/css/css-color/parsing/",
            "/css/css-display/parsing/",
            "/css/css-fonts/parsing/",
            "/css/css-inline/parsing/",
            "/css/css-lists/parsing/",
            "/css/css-logical/parsing/",
            "/css/css-sizing/parsing/",
            "/css/css-text/parsing/",
            "/css/css-text-decor/parsing/",
            "/css/css-writing-modes/parsing/",
        ],
    },
    Source {
        name: "html5lib-tests",
        repository: "https://github.com/html5lib/html5lib-tests",
        commit: "c777c408b61078ea2eb4acefc2535f54dbc8b28a",
        paths: &["/tokenizer/"],
    },
    Source {
        name: "unicodetools",
        repository: "https://github.com/unicode-org/unicodetools",
        commit: "578f6c0516f5f268292ebda50b51848407a06478",
        paths: &[
            "/unicodetools/data/ucd/18.0.0/NormalizationTest.txt",
            "/unicodetools/data/idna/18.0.0/IdnaTestV2.txt",
        ],
    },
    Source {
        name: "css-parsing-tests",
        repository: "https://github.com/SimonSapin/css-parsing-tests",
        commit: "203ce36bffd617db7f118c551e32794561fb273d",
        // Nur die Dateien, die zum aktuellen Entwurf von CSS Syntax passen,
        // siehe `crates/mb-css/src/tests.rs`.
        paths: &[
            "/component_value_list.json",
            "/one_component_value.json",
            "/stylesheet_bytes.json",
            "/An+B.json",
        ],
    },
    Source {
        name: "webref",
        repository: "https://github.com/w3c/webref",
        commit: "85e517c8fb6271560d09c1b550c40bcfc849f580",
        // Die aus den Editor's Drafts gelesenen CSS-Definitionen, siehe
        // `crates/mb-style/src/properties/webref.rs`.
        paths: &["/ed/css/"],
    },
];

/// Der Ordner `target/testdata/` im Workspace, unabhängig von
/// `CARGO_TARGET_DIR`.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("testdata")
}

/// Die Namen der Dateien mit der Endung `extension` in einem Ordner einer
/// Testsammlung, sortiert, zum Beispiel
/// `files("html5lib-tests", "tokenizer", "test")`.
pub fn files(source: &str, directory: &str, extension: &str) -> Vec<String> {
    let path = root().join(source).join(directory);
    let entries = match std::fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(error) => panic!(
            "Externe Testdaten fehlen ({}: {error}). Bitte `cargo run -p testdata` ausführen.",
            path.display()
        ),
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|found| found == extension))
        .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
        .collect();
    names.sort();
    names
}

/// Liest eine Datei einer Testsammlung, zum Beispiel
/// `read("wpt", "url/resources/urltestdata.json")`. Fehlt sie, endet der
/// Test mit einem Hinweis, wie die Daten zu holen sind.
pub fn read(source: &str, path: &str) -> String {
    let file = root().join(source).join(path);
    match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) => panic!(
            "Externe Testdaten fehlen ({}: {error}). Bitte `cargo run -p testdata` ausführen.",
            file.display()
        ),
    }
}
