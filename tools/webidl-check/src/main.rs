//! Entwicklungswerkzeug, kein Loader für Webinhalte oder Binding-Generator.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use mb_webidl::index::{self, Error, Limits};
use mb_webidl::{members, types};

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
                "Aufruf: cargo run -p webidl-check -- DATEI.idl [WEITERE.idl ...]\nPrüft Syntax, Namen, Verweise, Vererbung, Dictionary-Typzyklen, Union-Unterscheidbarkeit, Konstanten-/Defaultwerte, Spezialoperationen/Stringifier und erste Member-/Typregeln.\nAlle benötigten Definitionen gemeinsam angeben.\nKein vollständiger WebIDL-Validator; erzeugt keine Bindings."
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
    let budget =
        index::Budget::new(index::DEFAULT_STEPS + members::DEFAULT_STEPS + types::DEFAULT_STEPS);
    let index = index::build_within(&borrowed, limits, &budget)
        .map_err(|error| diagnostic(&paths, &sources, &error))?;
    let members =
        members::build_within(&index, members::Limits::default(), &budget).map_err(|error| {
            positioned_error(
                &paths,
                &sources,
                &error.kind,
                error.location.as_ref(),
                error.previous.as_ref(),
            )
        })?;
    let types = types::build_within(&index, &members, types::Limits::default(), &budget).map_err(
        |error| positioned_error(&paths, &sources, &error.kind, error.location.as_ref(), None),
    )?;
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
        "{} Member, {} Argumente, {} Enumwerte erfasst und auf die unterstützten Namensregeln geprüft.",
        members.len(),
        members.argument_count(),
        members.value_count()
    );
    println!(
        "{} Typknoten und {} Kanten aufgebaut. Vollständige Typ-/Wertprüfung, Extended Attributes, Overload-Unterscheidbarkeit und Bindings fehlen weiterhin.",
        types.len(),
        types.edge_count()
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
    positioned_error(
        paths,
        sources,
        &error.kind,
        error.location.as_ref(),
        error.previous.as_ref(),
    )
}

fn positioned_error(
    paths: &[OsString],
    sources: &[String],
    kind: &impl std::fmt::Debug,
    location: Option<&index::Location>,
    previous: Option<&index::Location>,
) -> String {
    let position = |at: &index::Location| {
        let before = &sources[at.source][..at.span.start];
        let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        format!("{}:{line}:{column}", Path::new(&paths[at.source]).display())
    };
    let mut message = match location {
        Some(at) => format!("{}: {kind:?}", position(at)),
        None => format!("{kind:?}"),
    };
    if let Some(previous) = previous {
        message.push_str(&format!(" (erste Deklaration: {})", position(previous)));
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
            "two.idl:2:13: DuplicateDefinition (erste Deklaration: one.idl:1:11)"
        );
        let sources = [
            "interface X {};".to_owned(),
            "/*🦀*/ interface X {};".to_owned(),
        ];
        let error =
            index::build(&sources.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_err();
        assert!(diagnostic(&paths, &sources, &error).starts_with("two.idl:1:17:"));
    }
    #[test]
    fn member_diagnostic_identifies_both_declarations_across_sources() {
        let paths = ["base.idl".into(), "partial.idl".into()];
        let sources = [
            "interface X { attribute long value; };".to_owned(),
            "partial interface X { attribute long _value; };".to_owned(),
        ];
        let index = index::build(&sources.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
        let error = members::build(&index).unwrap_err();
        let text = positioned_error(
            &paths,
            &sources,
            &error.kind,
            error.location.as_ref(),
            error.previous.as_ref(),
        );
        assert_eq!(
            text,
            "partial.idl:1:38: DuplicateMember (erste Deklaration: base.idl:1:30)"
        );
    }
    #[test]
    fn type_diagnostic_reports_the_alias_use_in_the_correct_source() {
        let paths = ["types.idl".into(), "api.idl".into()];
        let sources = [
            "typedef sequence<long> Values;".to_owned(),
            "interface X { attribute Values value; };".to_owned(),
        ];
        let index = index::build(&sources.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
        let members = members::build(&index).unwrap();
        let error = types::build(&index, &members).unwrap_err();
        assert_eq!(
            positioned_error(&paths, &sources, &error.kind, error.location.as_ref(), None),
            "api.idl:1:25: AttributeType"
        );
    }
    #[test]
    fn dictionary_cycle_diagnostic_identifies_a_field_across_sources() {
        let paths = ["left.idl".into(), "right.idl".into()];
        let sources = [
            "dictionary A { B next; };".to_owned(),
            "dictionary B { A back; };".to_owned(),
        ];
        let index = index::build(&sources.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
        let members = members::build(&index).unwrap();
        let error = types::build(&index, &members).unwrap_err();
        assert_eq!(
            positioned_error(&paths, &sources, &error.kind, error.location.as_ref(), None),
            "right.idl:1:18: DictionaryCycle"
        );
    }
    #[test]
    fn ambiguous_union_diagnostic_points_to_the_union_in_its_own_file() {
        let paths = ["types.idl".into(), "api.idl".into()];
        let sources = [
            "enum Name {\"x\"};".to_owned(),
            "typedef (Name or DOMString) Choice;".to_owned(),
        ];
        let index = index::build(&sources.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
        let members = members::build(&index).unwrap();
        let error = types::build(&index, &members).unwrap_err();
        assert_eq!(
            positioned_error(&paths, &sources, &error.kind, error.location.as_ref(), None),
            "api.idl:1:9: IndistinguishableUnion"
        );
    }
    #[test]
    fn literal_diagnostics_keep_cross_file_locations_and_unicode_columns() {
        let paths = ["types.idl".into(), "api.idl".into()];
        for (declaration, expected) in [
            (
                "interface I { const Count n = 18446744073709551616; };",
                "api.idl:2:31: ConstantValue",
            ),
            (
                "dictionary D { State value = \"off\"; };",
                "api.idl:2:30: DefaultValue",
            ),
        ] {
            let sources = [
                "typedef unsigned long long Count; enum State {\"on\"};".to_owned(),
                format!("/*😀*/\n{declaration}"),
            ];
            let index =
                index::build(&sources.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
            let members = members::build(&index).unwrap();
            let error = types::build(&index, &members).unwrap_err();
            assert_eq!(
                positioned_error(&paths, &sources, &error.kind, error.location.as_ref(), None),
                expected
            );
        }
    }
    #[test]
    fn special_member_diagnostics_keep_actual_cross_file_type_locations() {
        let paths = ["api.idl".into(), "types.idl".into()];
        for (source, expected) in [
            (
                "interface I { getter long get(Key key); };",
                "api.idl:1:31: SpecialOperationKey",
            ),
            (
                "interface I { stringifier attribute Key text; };",
                "api.idl:1:37: StringifierType",
            ),
        ] {
            let sources = [source.to_owned(), "typedef ByteString Key;".to_owned()];
            let index =
                index::build(&sources.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
            let members = members::build(&index).unwrap();
            let error = types::build(&index, &members).unwrap_err();
            assert_eq!(
                positioned_error(&paths, &sources, &error.kind, error.location.as_ref(), None),
                expected
            );
        }
    }
}
