//! Prüft, dass der Workspace nur eigene Crates enthält. Siehe
//! `docs/architektur.md`, Abschnitt 1.1.
//!
//! 1. `Cargo.lock`: Jedes Paket mit einem `source`-Eintrag stammt von
//!    außerhalb (crates.io, Git oder eine andere Registry) und verstößt gegen
//!    den Grundsatz, alles selbst zu bauen.
//! 2. Die Metadaten von Cargo (`cargo metadata`, das keine Build-Skripte
//!    ausführt) mit allen Features: Jedes Paket im Abhängigkeitsgraphen, auch
//!    Entwicklungs-, Build- und optionale Abhängigkeiten, muss ein Mitglied
//!    des Workspace sein, ohne Quelle, und sein Manifest muss nach dem
//!    Auflösen symbolischer Verweise unter der Wurzel des Workspace liegen.
//!    Pfadabhängigkeiten haben keinen `source`-Eintrag; eine außerhalb des
//!    Workspace fällt erst hier auf.
//!
//! Aufruf: `cargo run -p deps-guard [-- <Pfad zu Cargo.lock>]`. Das Manifest
//! des Workspace liegt neben `Cargo.lock`.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use mb_json::Value;

/// Ein Eintrag aus `Cargo.lock`, der von außerhalb des Workspace stammt.
#[derive(Debug, PartialEq, Eq)]
struct External {
    name: String,
    source: String,
}

/// Ergebnis der Prüfung einer `Cargo.lock`.
#[derive(Debug, PartialEq, Eq)]
struct Report {
    entries: usize,
    external: Vec<External>,
}

#[derive(Default)]
struct Entry {
    name: Option<String>,
    source: Option<String>,
}

/// Liest `Cargo.lock` zeilenweise. Geprüft werden `[[package]]` und
/// `[[patch.unused]]`, denn beide können eine externe Quelle haben.
fn check(lock: &str) -> Report {
    let mut report = Report {
        entries: 0,
        external: Vec::new(),
    };
    let mut entry: Option<Entry> = None;
    for line in lock.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            finish(entry.take(), &mut report);
            if line == "[[package]]" || line == "[[patch.unused]]" {
                entry = Some(Entry::default());
            }
            continue;
        }
        let Some(current) = entry.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(value.trim()).to_owned();
        match key.trim() {
            "name" => current.name = Some(value),
            "source" => current.source = Some(value),
            _ => {}
        }
    }
    finish(entry, &mut report);
    report
}

fn finish(entry: Option<Entry>, report: &mut Report) {
    let Some(entry) = entry else {
        return;
    };
    report.entries += 1;
    if let Some(source) = entry.source {
        report.external.push(External {
            name: entry.name.unwrap_or_else(|| "<ohne Namen>".to_owned()),
            source,
        });
    }
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
}

/// Warum ein Paket aus den Metadaten nicht zum Workspace gehört.
#[derive(Debug, PartialEq, Eq)]
enum Reason {
    /// Es hat eine Quelle (Registry oder Git).
    Source(String),
    /// Es liegt im Workspace, ist aber kein Mitglied.
    NotMember,
    /// Sein Manifest liegt außerhalb der Wurzel, hier kanonisch.
    Outside(PathBuf),
}

/// Ein Paket aus den Metadaten, das nicht zum Workspace gehört.
#[derive(Debug, PartialEq, Eq)]
struct Outsider {
    name: String,
    reason: Reason,
}

impl std::fmt::Display for Outsider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.reason {
            Reason::Source(source) => write!(formatter, "{} ({source})", self.name),
            Reason::NotMember => write!(formatter, "{} (kein Mitglied des Workspace)", self.name),
            Reason::Outside(path) => write!(
                formatter,
                "{} (außerhalb des Workspace: {})",
                self.name,
                path.display()
            ),
        }
    }
}

/// Prüft die Ausgabe von `cargo metadata --format-version 1`. Liefert die
/// Zahl der Pakete und die, die nicht zum Workspace gehören; `Err`, wenn
/// die Metadaten nicht die erwartete Form haben.
fn check_metadata(metadata: &Value) -> Result<(usize, Vec<Outsider>), String> {
    let text = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("Metadaten ohne `{key}`"))
    };
    let root = text(metadata, "workspace_root")?;
    let root = std::fs::canonicalize(&root).map_err(|err| format!("{root}: {err}"))?;
    let members: BTreeSet<&str> = metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .and_then(|members| members.iter().map(Value::as_str).collect())
        .ok_or("Metadaten ohne `workspace_members`")?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("Metadaten ohne `packages`")?;
    let mut outsiders = Vec::new();
    for package in packages {
        let name = text(package, "name")?;
        let id = text(package, "id")?;
        let reason = match package.get("source") {
            Some(Value::String(source)) => Some(Reason::Source(source.clone())),
            Some(Value::Null) => {
                let manifest = text(package, "manifest_path")?;
                // Lässt sich das Manifest nicht auflösen, gilt es als draußen.
                let canonical =
                    std::fs::canonicalize(&manifest).unwrap_or_else(|_| PathBuf::from(&manifest));
                if !canonical.starts_with(&root) {
                    Some(Reason::Outside(canonical))
                } else if !members.contains(id.as_str()) {
                    Some(Reason::NotMember)
                } else {
                    None
                }
            }
            _ => return Err(format!("{name}: Metadaten ohne `source`")),
        };
        if let Some(reason) = reason {
            outsiders.push(Outsider { name, reason });
        }
    }
    Ok((packages.len(), outsiders))
}

/// Die Metadaten des Workspace mit diesem Manifest, ohne Netz und ohne
/// `Cargo.lock` zu ändern. Mit allen Features, sonst fehlen optionale
/// Abhängigkeiten, die kein Standard-Feature einschaltet.
fn metadata(manifest: &Path) -> Result<Value, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--all-features"])
        .args(["--locked", "--offline"])
        .arg("--manifest-path")
        .arg(manifest)
        .output()
        .map_err(|err| format!("cargo metadata: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata ist gescheitert:\n{}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "cargo metadata: Ausgabe ist kein UTF-8".to_owned())?;
    mb_json::parse(&text).map_err(|err| format!("cargo metadata: {err}"))
}

/// Beide Prüfungen für die `Cargo.lock` unter `path`. `Ok` mit der Meldung
/// für die Standardausgabe, `Err` mit der für die Fehlerausgabe.
fn guard(path: &Path) -> Result<String, String> {
    let lock = std::fs::read_to_string(path)
        .map_err(|err| format!("{} lässt sich nicht lesen: {err}", path.display()))?;
    let report = check(&lock);
    if report.entries == 0 {
        return Err(format!(
            "{} enthält keine Pakete. Ist das die richtige Datei?",
            path.display()
        ));
    }
    let rule = "Erlaubt sind nur Crates aus diesem Workspace (docs/architektur.md, Abschnitt 1.1)";
    if !report.external.is_empty() {
        let mut message = format!(
            "{} externe Abhängigkeiten gefunden. {rule}:",
            report.external.len()
        );
        for external in &report.external {
            message.push_str(&format!("\n  {} ({})", external.name, external.source));
        }
        return Err(message);
    }
    let manifest = path.with_file_name("Cargo.toml");
    let (packages, outsiders) = check_metadata(&metadata(&manifest)?)?;
    if !outsiders.is_empty() {
        let mut message = format!(
            "{} Pakete gehören nicht zum Workspace. {rule}:",
            outsiders.len()
        );
        for outsider in &outsiders {
            message.push_str(&format!("\n  {outsider}"));
        }
        return Err(message);
    }
    Ok(format!(
        "{} Pakete in Cargo.lock und {packages} in den Metadaten geprüft, keine externen Abhängigkeiten.",
        report.entries
    ))
}

fn main() -> ExitCode {
    let path = std::env::args_os().nth(1).map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("Cargo.lock")
        },
        PathBuf::from,
    );
    match guard(&path) {
        Ok(message) => {
            println!("deps-guard: {message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("deps-guard: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACE_ONLY: &str = r#"# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 4

[[package]]
name = "mb-base"
version = "0.0.0"

[[package]]
name = "meinbrowser"
version = "0.0.0"
dependencies = [
 "mb-base",
]
"#;

    const FROM_REGISTRY: &str = r#"
[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0000"
"#;

    const FROM_GIT: &str = r#"
[[package]]
name = "foo"
version = "0.1.0"
source = "git+https://example.com/foo.git#0123456"
"#;

    const UNUSED_PATCH: &str = r#"
[[patch.unused]]
name = "bar"
version = "0.2.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    fn external(name: &str, source: &str) -> External {
        External {
            name: name.to_owned(),
            source: source.to_owned(),
        }
    }

    #[test]
    fn workspace_crates_are_allowed() {
        assert_eq!(
            check(WORKSPACE_ONLY),
            Report {
                entries: 2,
                external: Vec::new(),
            }
        );
    }

    #[test]
    fn registry_and_git_sources_are_reported() {
        let lock = format!("{WORKSPACE_ONLY}{FROM_REGISTRY}{FROM_GIT}");
        assert_eq!(
            check(&lock),
            Report {
                entries: 4,
                external: vec![
                    external(
                        "serde",
                        "registry+https://github.com/rust-lang/crates.io-index"
                    ),
                    external("foo", "git+https://example.com/foo.git#0123456"),
                ],
            }
        );
    }

    #[test]
    fn unused_patches_are_reported() {
        let lock = format!("{WORKSPACE_ONLY}{UNUSED_PATCH}");
        assert_eq!(
            check(&lock).external,
            vec![external(
                "bar",
                "registry+https://github.com/rust-lang/crates.io-index"
            )]
        );
    }

    #[test]
    fn source_without_name_is_still_reported() {
        let lock = "[[package]]\nsource = \"registry+https://example.com/index\"\n";
        assert_eq!(
            check(lock).external,
            vec![external(
                "<ohne Namen>",
                "registry+https://example.com/index"
            )]
        );
    }

    #[test]
    fn empty_lock_has_no_entries() {
        assert_eq!(check("version = 4\n").entries, 0);
    }

    #[test]
    fn this_workspace_has_no_external_dependencies() {
        let report = check(include_str!("../../../Cargo.lock"));
        assert!(report.entries > 0);
        assert_eq!(report.external, Vec::new());
    }

    /// Ein eigener Ordner je Test (RS-14).
    fn directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("deps-guard-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("Ordner");
        std::fs::canonicalize(&directory).expect("Ordner")
    }

    /// Ein Paket mit Manifest unter `path`.
    fn package(path: &Path, name: &str, dependencies: &str) {
        std::fs::create_dir_all(path.join("src")).expect("Ordner");
        std::fs::write(
            path.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
                 [dependencies]\n{dependencies}"
            ),
        )
        .expect("schreiben");
        std::fs::write(path.join("src").join("lib.rs"), "").expect("schreiben");
    }

    /// Metadaten in der Form von `cargo metadata`, mit dem Workspace unter
    /// `root` und Paketen als (Name, Quelle, Manifest, Mitglied).
    fn metadata_json(root: &Path, packages: &[(&str, Option<&str>, &Path, bool)]) -> Value {
        let quote = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\"));
        let id = |name: &str| format!("path+file:///{name}#0.0.0");
        let entries: Vec<String> = packages
            .iter()
            .map(|(name, source, manifest, _)| {
                format!(
                    "{{\"name\": {}, \"id\": {}, \"source\": {}, \"manifest_path\": {}}}",
                    quote(name),
                    quote(&id(name)),
                    source.map_or_else(|| "null".to_owned(), quote),
                    quote(&manifest.to_string_lossy())
                )
            })
            .collect();
        let members: Vec<String> = packages
            .iter()
            .filter(|(_, _, _, member)| *member)
            .map(|(name, ..)| quote(&id(name)))
            .collect();
        let text = format!(
            "{{\"packages\": [{}], \"workspace_members\": [{}], \"workspace_root\": {}}}",
            entries.join(", "),
            members.join(", "),
            quote(&root.to_string_lossy())
        );
        mb_json::parse(&text).expect("JSON")
    }

    #[test]
    fn metadata_reject_sources_strangers_and_paths_outside() {
        let base = directory("metadaten");
        let root = base.join("ws");
        package(&root.join("app"), "app", "");
        package(&root.join("vendor"), "vendor", "");
        package(&base.join("foreign"), "foreign", "");
        let manifest = |path: &Path| path.join("Cargo.toml");
        let metadata = metadata_json(
            &root,
            &[
                ("app", None, &manifest(&root.join("app")), true),
                (
                    "serde",
                    Some("registry+https://github.com/rust-lang/crates.io-index"),
                    &manifest(&root.join("app")),
                    false,
                ),
                (
                    "foo",
                    Some("git+https://example.com/foo.git#0123456"),
                    &manifest(&root.join("app")),
                    false,
                ),
                ("vendor", None, &manifest(&root.join("vendor")), false),
                ("foreign", None, &manifest(&base.join("foreign")), true),
            ],
        );
        let (count, outsiders) = check_metadata(&metadata).expect("Metadaten");
        assert_eq!(count, 5);
        let reasons: Vec<(&str, &Reason)> = outsiders
            .iter()
            .map(|outsider| (outsider.name.as_str(), &outsider.reason))
            .collect();
        assert_eq!(
            reasons,
            [
                (
                    "serde",
                    &Reason::Source("registry+https://github.com/rust-lang/crates.io-index".into())
                ),
                (
                    "foo",
                    &Reason::Source("git+https://example.com/foo.git#0123456".into())
                ),
                ("vendor", &Reason::NotMember),
                ("foreign", &Reason::Outside(manifest(&base.join("foreign")))),
            ]
        );
        std::fs::remove_dir_all(&base).expect("aufräumen");
    }

    #[cfg(unix)]
    #[test]
    fn members_linked_from_outside_are_rejected() {
        let base = directory("verweis");
        let root = base.join("ws");
        package(&base.join("foreign"), "linked", "");
        std::fs::create_dir_all(&root).expect("Ordner");
        std::os::unix::fs::symlink(base.join("foreign"), root.join("linked")).expect("Verweis");
        let metadata = metadata_json(
            &root,
            &[(
                "linked",
                None,
                &root.join("linked").join("Cargo.toml"),
                true,
            )],
        );
        let (_, outsiders) = check_metadata(&metadata).expect("Metadaten");
        assert_eq!(
            outsiders,
            [Outsider {
                name: "linked".into(),
                reason: Reason::Outside(base.join("foreign").join("Cargo.toml")),
            }]
        );
        std::fs::remove_dir_all(&base).expect("aufräumen");
    }

    #[test]
    fn path_dependency_outside_the_workspace_is_found() {
        // Wie der Fall aus dem Bericht: Ein Mitglied hängt von einem Paket
        // außerhalb des Workspace ab; Cargo.lock hat dafür keine Quelle.
        let base = directory("pfad");
        let root = base.join("ws");
        package(&base.join("foreign"), "foreign", "");
        package(
            &root.join("app"),
            "app",
            "foreign = { path = \"../../foreign\" }\n",
        );
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\"]\nresolver = \"3\"\n",
        )
        .expect("schreiben");
        std::fs::write(
            root.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.0.0\"\n\
             dependencies = [\n \"foreign\",\n]\n\n\
             [[package]]\nname = \"foreign\"\nversion = \"0.0.0\"\n",
        )
        .expect("schreiben");
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("lesen");
        assert_eq!(check(&lock).external, Vec::new());
        let error = guard(&root.join("Cargo.lock")).expect_err("Pfad nach draußen");
        assert!(
            error.contains("foreign (außerhalb des Workspace"),
            "{error}"
        );
        std::fs::remove_dir_all(&base).expect("aufräumen");
    }

    #[test]
    fn optional_path_dependency_outside_the_workspace_is_found() {
        // Ohne eingeschaltetes Feature fehlt das Paket in den Metadaten.
        let base = directory("optional");
        let root = base.join("ws");
        package(&base.join("foreign"), "foreign", "");
        package(
            &root.join("app"),
            "app",
            "foreign = { path = \"../../foreign\", optional = true }\n",
        );
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\"]\nresolver = \"3\"\n",
        )
        .expect("schreiben");
        std::fs::write(
            root.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.0.0\"\n\
             dependencies = [\n \"foreign\",\n]\n\n\
             [[package]]\nname = \"foreign\"\nversion = \"0.0.0\"\n",
        )
        .expect("schreiben");
        let error = guard(&root.join("Cargo.lock")).expect_err("optionaler Pfad nach draußen");
        assert!(
            error.contains("foreign (außerhalb des Workspace"),
            "{error}"
        );
        std::fs::remove_dir_all(&base).expect("aufräumen");
    }

    #[test]
    fn this_workspace_passes_both_checks() {
        let lock = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("Cargo.lock");
        let message = guard(&lock).expect("eigener Workspace");
        assert!(
            message.ends_with("keine externen Abhängigkeiten."),
            "{message}"
        );
    }
}
