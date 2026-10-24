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
pub const SOURCES: [Source; 2] = [
    Source {
        name: "wpt",
        repository: "https://github.com/web-platform-tests/wpt",
        commit: "c7902c0b13fb010a78126eb9d951e1f22c421013",
        paths: &["/url/resources/"],
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
