//! Erzeugt Tabellen aus den Rohdaten unter `data/`, zurzeit die
//! Unicode-Tabellen für `mb-unicode` (siehe `docs/architektur.md`,
//! Abschnitt 4.3). Die erzeugten Dateien werden eingecheckt.
//!
//! Aufruf: `cargo run -p datagen [-- --check]`. Mit `--check` schreibt das
//! Programm nichts, sondern prüft nur, ob die eingecheckten Dateien aktuell
//! sind.

#![forbid(unsafe_code)]

mod ucd;
mod unicode;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "Aufruf: datagen [--check]";

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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    match run(&root, check) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("datagen: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: &Path, check: bool) -> Result<(), String> {
    let data = root.join(unicode::DATA);
    let read = |name: &str| {
        let path = data.join(name);
        std::fs::read_to_string(&path)
            .map_err(|err| format!("{} lässt sich nicht lesen: {err}", path.display()))
    };
    let generated = unicode::generate(
        &read("UnicodeData.txt")?,
        &read("PropertyValueAliases.txt")?,
    )?;
    let output = root.join(unicode::OUTPUT);
    // Fehlt die Datei, gilt sie als veraltet.
    let current = std::fs::read_to_string(&output).ok();
    let up_to_date = current.as_deref() == Some(generated.as_str());
    if check {
        if !up_to_date {
            return Err(format!(
                "{} ist veraltet, bitte `cargo run -p datagen` ausführen",
                unicode::OUTPUT
            ));
        }
        println!("datagen: {} ist aktuell.", unicode::OUTPUT);
    } else if up_to_date {
        println!("datagen: {} ist unverändert.", unicode::OUTPUT);
    } else {
        std::fs::write(&output, generated)
            .map_err(|err| format!("{} lässt sich nicht schreiben: {err}", output.display()))?;
        println!("datagen: {} geschrieben.", unicode::OUTPUT);
    }
    Ok(())
}
