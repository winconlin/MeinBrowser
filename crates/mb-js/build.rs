//! Die Kennung dieses Baus der Engine für den Zwischenspeicher für
//! Bytecode (`code_cache::ENGINE`).
//!
//! Ein gespeicherter Eintrag stimmt nur, solange Parser und Übersetzer
//! dasselbe ergeben wie beim Speichern. Darum ist die Kennung SHA-256 über
//! alle Quellen, aus denen sie entstehen: `Cargo.toml` und `src/` von
//! `mb-js` und von jedem Crate, von dem es über Pfade abhängt, auch
//! mittelbar, ohne Entwicklungs- und Build-Abhängigkeiten. Welche das sind,
//! steht in den Manifesten. Ändert sich eine Datei dort, ändert sich die
//! Kennung, auch wenn der Bytecode gleich bliebe; das kostet nur Einträge.
//! Der Übersetzer von Rust geht nicht ein: Was er aus denselben Quellen
//! macht, verhält sich gleich.
//!
//! Die Kennung landet als Array-Ausdruck in `OUT_DIR/engine.rs`.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use mb_crypto::{Builtin, HashAlgorithm, HashContext, Provider};

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let crates = crates(&manifest_dir);
    let mut context = Builtin.hash(HashAlgorithm::Sha256);
    for directory in &crates {
        let manifest = directory.join("Cargo.toml");
        let sources = directory.join("src");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rerun-if-changed={}", sources.display());
        let name = directory.file_name().expect("Name des Crates");
        update(context.as_mut(), name.as_encoded_bytes());
        let mut files = vec![(b"Cargo.toml".to_vec(), manifest)];
        files.extend(source_files(&sources));
        files.sort();
        let count = u64::try_from(files.len()).expect("Zahl der Dateien");
        update(context.as_mut(), &count.to_le_bytes());
        for (relative, path) in files {
            update(context.as_mut(), &relative);
            update(
                context.as_mut(),
                &fs::read(&path).expect("Quelldatei lesen"),
            );
        }
    }
    let digest = context.finish();
    let bytes: Vec<String> = digest.as_bytes().iter().map(u8::to_string).collect();
    fs::write(
        out_dir.join("engine.rs"),
        format!("[{}]\n", bytes.join(", ")),
    )
    .expect("engine.rs schreiben");
}

/// Hängt `bytes` mit ihrer Länge davor an, damit die Grenzen eindeutig
/// sind.
fn update(context: &mut dyn HashContext, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).expect("Länge");
    context.update(&length.to_le_bytes());
    context.update(bytes);
}

/// Die Ordner von `mb-js` und allen Crates, von denen es über Pfade
/// abhängt, ohne Rekursion und jeder einmal.
fn crates(root: &Path) -> BTreeSet<PathBuf> {
    let mut crates = BTreeSet::new();
    let mut open = vec![root.to_path_buf()];
    while let Some(directory) = open.pop() {
        let directory = directory.canonicalize().expect("Ordner eines Crates");
        if !crates.insert(directory.clone()) {
            continue;
        }
        let manifest = fs::read_to_string(directory.join("Cargo.toml")).expect("Cargo.toml lesen");
        open.extend(dependency_paths(&manifest).map(|path| directory.join(path)));
    }
    crates
}

/// Die Pfade der Abhängigkeiten in `[dependencies]` und
/// `[target.….dependencies]`. Die Manifeste des Workspace schreiben sie je
/// in eine Zeile: `name = { path = "../name" }`.
fn dependency_paths(manifest: &str) -> impl Iterator<Item = &str> {
    let mut normal = false;
    manifest.lines().filter_map(move |line| {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[') {
            let section = section.trim_end_matches(']');
            normal = section == "dependencies"
                || (section.starts_with("target.") && section.ends_with(".dependencies"));
            return None;
        }
        if !normal {
            return None;
        }
        let (_, rest) = line.split_once("path = \"")?;
        let (path, _) = rest.split_once('"')?;
        Some(path)
    })
}

/// Alle Dateien unter `sources` mit ihrem Pfad ab dem Crate, mit `/`
/// getrennt, ohne Rekursion.
fn source_files(sources: &Path) -> Vec<(Vec<u8>, PathBuf)> {
    let mut files = Vec::new();
    let mut open = vec![(b"src".to_vec(), sources.to_path_buf())];
    while let Some((relative, directory)) = open.pop() {
        for entry in fs::read_dir(&directory).expect("Ordner lesen") {
            let entry = entry.expect("Eintrag lesen");
            let mut path = relative.clone();
            path.push(b'/');
            path.extend_from_slice(entry.file_name().as_encoded_bytes());
            let kind = entry.file_type().expect("Art des Eintrags");
            if kind.is_dir() {
                open.push((path, entry.path()));
            } else {
                files.push((path, entry.path()));
            }
        }
    }
    files
}
