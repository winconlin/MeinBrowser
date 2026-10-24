//! Erzeugt Tabellen aus den Rohdaten unter `data/`: die Unicode-Tabellen und
//! die IDNA-Zuordnungstabelle für `mb-unicode` (siehe `docs/architektur.md`,
//! Abschnitt 4.3) und die Tabellen des WHATWG Encoding Standard für
//! `mb-encoding` (Abschnitt 4.4). Die erzeugten Dateien werden eingecheckt.
//!
//! Aufruf: `cargo run -p datagen [-- --check]`. Mit `--check` schreibt das
//! Programm nichts, sondern prüft nur, ob die eingecheckten Dateien aktuell
//! sind.

#![forbid(unsafe_code)]

mod encoding;
mod idna;
mod ucd;
mod unicode;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "Aufruf: datagen [--check]";

/// Liest eine Datei aus dem Datenordner eines Generators.
type Read<'a> = &'a dyn Fn(&str) -> Result<String, String>;

/// Ein Generator: Rohdaten, Ziel (beides relativ zum Workspace) und die
/// Funktion, die aus den Rohdaten den Quelltext macht.
struct Generator {
    data: &'static str,
    output: &'static str,
    run: fn(Read<'_>) -> Result<String, String>,
}

const GENERATORS: [Generator; 3] = [
    Generator {
        data: unicode::DATA,
        output: unicode::OUTPUT,
        run: unicode::run,
    },
    Generator {
        data: idna::DATA,
        output: idna::OUTPUT,
        run: idna::run,
    },
    Generator {
        data: encoding::DATA,
        output: encoding::OUTPUT,
        run: encoding::run,
    },
];

fn main() -> ExitCode {
    let mut check = false;
    for argument in std::env::args_os().skip(1) {
        match argument.to_str() {
            Some("--check") => check = true,
            Some("--help") => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("datagen: unbekanntes Argument {}", argument.display());
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    let root = workspace();
    let mut failed = false;
    for generator in &GENERATORS {
        match update(&root, generator, check) {
            Ok(message) => println!("datagen: {message}"),
            Err(message) => {
                eprintln!("datagen: {message}");
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn workspace() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Erzeugt die Datei eines Generators neu.
fn generate(root: &Path, generator: &Generator) -> Result<String, String> {
    let data = root.join(generator.data);
    let read = |name: &str| {
        let path = data.join(name);
        std::fs::read_to_string(&path)
            .map_err(|err| format!("{} lässt sich nicht lesen: {err}", path.display()))
    };
    (generator.run)(&read)
}

/// Schreibt oder prüft die Datei eines Generators und sagt, was geschah.
fn update(root: &Path, generator: &Generator, check: bool) -> Result<String, String> {
    let generated = generate(root, generator)?;
    let output = root.join(generator.output);
    // Fehlt die Datei, gilt sie als veraltet.
    let current = std::fs::read_to_string(&output).ok();
    let up_to_date = current.as_deref() == Some(generated.as_str());
    if check {
        if up_to_date {
            Ok(format!("{} ist aktuell.", generator.output))
        } else {
            Err(format!(
                "{} ist veraltet, bitte `cargo run -p datagen` ausführen",
                generator.output
            ))
        }
    } else if up_to_date {
        Ok(format!("{} ist unverändert.", generator.output))
    } else {
        std::fs::write(&output, generated)
            .map_err(|err| format!("{} lässt sich nicht schreiben: {err}", output.display()))?;
        Ok(format!("{} geschrieben.", generator.output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_tables_are_current() {
        let root = workspace();
        for generator in &GENERATORS {
            let generated = generate(&root, generator).expect("Rohdaten lesbar");
            let current = std::fs::read_to_string(root.join(generator.output)).unwrap_or_default();
            assert!(
                generated == current,
                "{} ist veraltet, bitte `cargo run -p datagen` ausführen",
                generator.output
            );
        }
    }
}
