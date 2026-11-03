//! Entwicklungswerkzeug, kein Loader für Webinhalte oder Binding-Generator.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use mb_webidl::index::{self, Error, Limits};

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("webidl-check: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: impl Iterator<Item = OsString>) -> Result<(), String> {
    let limits = Limits::default();
    let mut paths = Vec::new();
    for arg in args {
        if arg == "--help" || arg == "-h" {
            println!(
                "Aufruf: cargo run -p webidl-check -- DATEI.idl [WEITERE.idl ...]\nPrüft Syntax, Definitionsnamen, Partials, Typverweise, Includes und Vererbung.\nAlle benötigten Definitionen gemeinsam angeben.\nKein vollständiger WebIDL-Validator; erzeugt keine Bindings."
            );
            return Ok(());
        }
        if paths.len() >= limits.sources {
            return Err("zu viele Quelldateien".into());
        }
        paths.push(arg);
    }
    if paths.is_empty() {
        return Err("mindestens eine IDL-Datei angeben; Hilfe mit --help".into());
    }
    let mut sources = Vec::new();
    let mut remaining = limits.source_bytes;
    for path in &paths {
        let maximum = remaining.min(limits.parser.source_bytes);
        let source = read(Path::new(path), maximum)
            .map_err(|e| format!("{}: {e}", Path::new(path).display()))?;
        remaining -= source.len();
        sources.push(source);
    }
    let borrowed: Vec<_> = sources.iter().map(String::as_str).collect();
    let index = index::build(&borrowed).map_err(|error| diagnostic(&paths, &sources, &error))?;
    let fragments: usize = index
        .definitions()
        .map(|(_, definition)| definition.fragments.len())
        .sum();
    println!(
        "{} Quellen, {} Definitionen, {} Fragmente, {} Typverweise, {} Includes aufgelöst.",
        index.sources(),
        index.definitions().len(),
        fragments,
        index.references().len(),
        index.includes().len()
    );
    println!(
        "Member, Overloads, Typformen, Defaultwerte, Extended Attributes und Bindings benötigen weitere Prüfung."
    );
    Ok(())
}

fn read(path: &Path, maximum: usize) -> io::Result<String> {
    let file = File::open(path)?;
    read_bounded(file, maximum)
}

fn read_bounded(reader: impl Read, maximum: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(io::Error::other("Quelltextlimit überschritten"));
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Quelle ist kein UTF-8"))
}

fn diagnostic(paths: &[OsString], sources: &[String], error: &Error) -> String {
    let position = |at: &index::Location| {
        let before = &sources[at.source][..at.span.start];
        let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        format!("{}:{line}:{column}", Path::new(&paths[at.source]).display())
    };
    let mut message = match &error.location {
        Some(at) => format!("{}: {:?}", position(at), error.kind),
        None => format!("{:?}", error.kind),
    };
    if let Some(previous) = &error.previous {
        message.push_str(&format!(" (erste Definition: {})", position(previous)));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reader_checks_actual_bytes_and_utf8() {
        assert_eq!(read_bounded("é".as_bytes(), 2).unwrap(), "é");
        assert!(read_bounded("é".as_bytes(), 1).is_err());
        assert!(read_bounded([0xff].as_slice(), 1).is_err());
        assert_eq!(read_bounded([].as_slice(), 0).unwrap(), "");
        assert!(read_bounded(b"x".as_slice(), 0).is_err());
    }

    #[test]
    fn duplicate_diagnostics_keep_both_files_and_count_unicode_columns() {
        let paths = ["one.idl".into(), "two.idl".into()];
        let sources = [
            "interface X {};".to_owned(),
            "/*🦀*/\n  interface X {};".to_owned(),
        ];
        let error =
            index::build(&sources.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_err();
        assert_eq!(
            diagnostic(&paths, &sources, &error),
            "two.idl:2:13: DuplicateDefinition (erste Definition: one.idl:1:11)"
        );
        let sources = [
            "interface X {};".to_owned(),
            "/*🦀*/ interface X {};".to_owned(),
        ];
        let error =
            index::build(&sources.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_err();
        assert!(diagnostic(&paths, &sources, &error).starts_with("two.idl:1:17:"));
    }
}
