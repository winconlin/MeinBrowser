//! Wo das System Schriften ablegt.
//!
//! [`font_files`] findet die Dateien; den Index baut `mb-text`. Die Bytes
//! liest [`read_font`] mit einer Grenze je Datei, auch beim späteren
//! Nachladen eines Schnitts. Die Erkennung von Schriftformaten bleibt
//! Aufgabe von `mb-text`.
//! Unter Linux und BSD sind es die Vorgaben aus der `fonts.conf` von
//! fontconfig (Debian und Ubuntu, fontconfig 2.15.0); weitere Verzeichnisse
//! aus den Konfigurationsdateien von fontconfig werden noch nicht gelesen.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Höchstgröße einer Schriftdatei, einschließlich Sammlungen (TTC/OTC).
/// Festgelegt von diesem Browser: 64 MiB. Größere Dateien werden weder
/// indexiert noch nachgeladen. Dies begrenzt nicht den gesamten Schriftcache
/// oder den RAM-Verbrauch beim Parsen.
pub const MAX_FONT_BYTES: u64 = 64 << 20;

/// Liest eine Schriftdatei, wenn sie eine normale Datei mit höchstens
/// [`MAX_FONT_BYTES`] ist. Bei Spezialdateien, Überschreitung oder Lesefehlern
/// `None`. Symbolische Verweise zählen mit ihrem Ziel; geprüft wird auch am
/// offenen Handle (siehe [`crate::files::read_regular`]).
///
/// Derselbe Leser wird für den Indexaufbau und für spätere Abrufe verwendet:
/// Die Verzeichnissuche allein schützt nicht davor, dass eine Datei danach
/// ausgetauscht wird. Die bestehenden Grenzen der Pfad-/Handleprüfung gelten
/// weiterhin; die Suche selbst ist damit nicht gegen alle Dateisystemrennen
/// abgesichert.
pub fn read_font(path: &Path) -> Option<Vec<u8>> {
    crate::files::read_regular(path, MAX_FONT_BYTES).ok()
}

/// Die Verzeichnisse mit installierten Schriften, ob es sie gibt oder nicht.
pub fn font_directories() -> Vec<PathBuf> {
    system_directories()
}

/// Alle Schriftdateien in [`font_directories`] und deren Unterverzeichnissen:
/// Dateien mit den Endungen `ttf`, `otf`, `ttc` und `otc`, ohne Rücksicht
/// auf Groß- und Kleinschreibung. Die Pfade sind kanonisch, sortiert und
/// kommen je nur einmal vor, auch wenn symbolische Verweise auf dieselbe
/// Datei zeigen.
pub fn font_files() -> Vec<PathBuf> {
    list(&font_directories())
}

/// Durchsucht die Verzeichnisse ohne Rekursion (RS-8); jedes Verzeichnis
/// nur einmal, damit Schleifen aus symbolischen Verweisen enden.
fn list(directories: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut pending: Vec<PathBuf> = directories.iter().rev().cloned().collect();
    while let Some(directory) = pending.pop() {
        let Ok(directory) = std::fs::canonicalize(&directory) else {
            continue;
        };
        if !visited.insert(directory.clone()) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `metadata` folgt symbolischen Verweisen.
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file()
                && is_font_file(&path)
                && let Ok(path) = std::fs::canonicalize(&path)
            {
                files.insert(path);
            }
        }
    }
    files.into_iter().collect()
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["ttf", "otf", "ttc", "otc"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

/// Eine Umgebungsvariable mit einem absoluten Pfad; leere und relative Werte
/// gelten wie in der XDG Base Directory Specification als nicht gesetzt.
#[cfg(any(windows, unix))]
fn absolute_variable(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// `/usr/share/fonts`, `/usr/local/share/fonts`, `fonts` unter
/// `$XDG_DATA_HOME` (sonst `~/.local/share`) und das veraltete `~/.fonts`.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn system_directories() -> Vec<PathBuf> {
    let home = absolute_variable("HOME");
    let mut directories = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    let data = absolute_variable("XDG_DATA_HOME")
        .or_else(|| home.as_ref().map(|home| home.join(".local").join("share")));
    directories.extend(data.map(|data| data.join("fonts")));
    directories.extend(home.map(|home| home.join(".fonts")));
    directories
}

/// Die Schriften des Systems unter `%WINDIR%\Fonts` und die nur für den
/// Benutzer installierten unter `%LOCALAPPDATA%\Microsoft\Windows\Fonts`.
#[cfg(windows)]
fn system_directories() -> Vec<PathBuf> {
    let windows = absolute_variable("WINDIR").or_else(|| absolute_variable("SystemRoot"));
    let local = absolute_variable("LOCALAPPDATA");
    let mut directories = Vec::new();
    directories.extend(windows.map(|windows| windows.join("Fonts")));
    directories.extend(local.map(|local| local.join("Microsoft").join("Windows").join("Fonts")));
    directories
}

/// Die Schriften des Systems, die für alle Benutzer, die im Netzwerk und
/// die des Benutzers.
#[cfg(target_os = "macos")]
fn system_directories() -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/Network/Library/Fonts"),
    ];
    directories.extend(absolute_variable("HOME").map(|home| home.join("Library").join("Fonts")));
    directories
}

#[cfg(not(any(
    windows,
    target_os = "macos",
    all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )
)))]
fn system_directories() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn font_files_by_extension() {
        assert!(is_font_file(Path::new("/a/DejaVuSans.ttf")));
        assert!(is_font_file(Path::new("/a/Inter.OTF")));
        assert!(is_font_file(Path::new("/a/wqy-zenhei.ttc")));
        assert!(is_font_file(Path::new("a.otc")));
        assert!(!is_font_file(Path::new("/a/c0419bt_.pfb")));
        assert!(!is_font_file(Path::new("/a/crox1c.pcf.gz")));
        assert!(!is_font_file(Path::new("/a/ttf")));
        assert!(!is_font_file(Path::new("/a/fonts.dir")));
    }

    /// Baut einen kleinen Baum mit Unterverzeichnis, doppeltem Eintrag und
    /// (unter Unix) einer Schleife aus symbolischen Verweisen.
    #[test]
    fn lists_directories_once() {
        let root = std::env::temp_dir().join(format!(
            "mb-platform-fonts-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let nested = root.join("truetype").join("dejavu");
        std::fs::create_dir_all(&nested).expect("Verzeichnis anlegen");
        for (directory, name) in [
            (&root, "b.otf"),
            (&root, "README"),
            (&nested, "a.TTF"),
            (&nested, "c.pfb"),
        ] {
            std::fs::write(directory.join(name), b"").expect("Datei anlegen");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, nested.join("loop")).expect("Verweis anlegen");
        let missing = root.join("fehlt");
        let found = list(&[root.clone(), nested.clone(), missing]);
        let root = std::fs::canonicalize(&root).expect("kanonischer Pfad");
        let nested = root.join("truetype").join("dejavu");
        assert_eq!(found, [root.join("b.otf"), nested.join("a.TTF")]);
        std::fs::remove_dir_all(&root).expect("aufräumen");
    }
}

#[cfg(test)]
mod tests;
