//! Holt die externen Testsammlungen aus [`testdata::SOURCES`] mit `git` in
//! der festgelegten Version nach `target/testdata/`. Schon vorhandene
//! Sammlungen im richtigen Stand bleiben, wie sie sind.
//!
//! Aufruf: `cargo run -p testdata`

#![forbid(unsafe_code)]

use std::path::Path;
use std::process::{Command, ExitCode};

use testdata::{SOURCES, Source};

fn main() -> ExitCode {
    if std::env::args_os().len() > 1 {
        eprintln!("Aufruf: testdata");
        return ExitCode::FAILURE;
    }
    let root = testdata::root();
    let mut failed = false;
    for source in &SOURCES {
        match fetch(&root, source) {
            Ok(message) => println!("testdata: {}: {message}", source.name),
            Err(message) => {
                eprintln!("testdata: {}: {message}", source.name);
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

/// Der Inhalt der Stand-Datei neben dem Ordner einer Sammlung.
fn stamp(source: &Source) -> String {
    let mut text = format!("{}\n{}\n", source.repository, source.commit);
    for path in source.paths {
        text.push_str(path);
        text.push('\n');
    }
    text
}

fn fetch(root: &Path, source: &Source) -> Result<String, String> {
    let directory = root.join(source.name);
    let stamp_file = root.join(format!("{}.stand", source.name));
    let expected = stamp(source);
    if directory.is_dir() && std::fs::read_to_string(&stamp_file).ok() == Some(expected.clone()) {
        return Ok(format!("ist aktuell ({}).", source.commit));
    }
    let _ = std::fs::remove_file(&stamp_file);
    if directory.exists() {
        std::fs::remove_dir_all(&directory).map_err(|error| {
            format!("{} lässt sich nicht löschen: {error}", directory.display())
        })?;
    }
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{} lässt sich nicht anlegen: {error}", directory.display()))?;
    git(&directory, &["init", "-q"])?;
    // Byte-genau wie im Repository, auch unter Windows.
    git(&directory, &["config", "core.autocrlf", "false"])?;
    git(&directory, &["remote", "add", "origin", source.repository])?;
    let mut sparse = vec!["sparse-checkout", "set", "--no-cone"];
    sparse.extend(source.paths);
    git(&directory, &sparse)?;
    // Nur dieser eine Commit, Dateiinhalte nur für die gewählten Pfade.
    git(
        &directory,
        &[
            "fetch",
            "-q",
            "--no-tags",
            "--depth=1",
            "--filter=blob:none",
            "origin",
            source.commit,
        ],
    )?;
    git(&directory, &["checkout", "-q", "FETCH_HEAD"])?;
    let head = git(&directory, &["rev-parse", "HEAD"])?;
    if head.trim() != source.commit {
        return Err(format!(
            "geholt wurde {}, erwartet war {}",
            head.trim(),
            source.commit
        ));
    }
    for path in source.paths {
        let relative = path.trim_start_matches('/');
        if !directory.join(relative).exists() {
            return Err(format!("{path} fehlt im Commit {}", source.commit));
        }
    }
    std::fs::write(&stamp_file, expected).map_err(|error| {
        format!(
            "{} lässt sich nicht schreiben: {error}",
            stamp_file.display()
        )
    })?;
    Ok(format!("geholt ({}).", source.commit))
}

/// Führt `git` im Ordner aus und liefert die Standardausgabe.
fn git(directory: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .map_err(|error| format!("git lässt sich nicht starten: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} ist gescheitert: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
