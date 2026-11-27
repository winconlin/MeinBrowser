//! Ordner, deren Dateien das Programm selbst anlegt, etwa für
//! Zwischenspeicher: anlegen, auflisten, Dateien ersetzen und entfernen und
//! die Zeit einer Datei setzen.
//!
//! Wer den Ordner sonst noch beschreiben kann, kann dort auch Verweise,
//! Unterordner und Spezialdateien ablegen. Darum folgt das Auflisten keinen
//! Verweisen und nennt nur normale Dateien, und ein Dateiname ist immer ein
//! einzelner Bestandteil ohne Trenner. Eine neue Datei entsteht unter einem
//! frischen Namen mit `create_new`, das auch an einem Verweis scheitert,
//! und `rename` setzt sie dann an ihren Platz. Das ersetzt den Eintrag im
//! Ordner selbst, nicht das Ziel eines Verweises, und wer liest, sieht die
//! alte Datei oder die neue ganz, nie eine halb geschriebene. Ohne `fsync`
//! kann ein Absturz des Systems trotzdem eine kürzere Datei hinterlassen;
//! wer die Dateien liest, muss sie also ohnehin prüfen.
//!
//! Unter Unix sind neue Ordner nur für den eigenen Benutzer zugänglich
//! (`0o700`) und neue Dateien nur für ihn lesbar und schreibbar (`0o600`).
//! Unter Windows gelten für jeden Pfad dieselben Regeln wie beim Lesen
//! ([`super::read_regular`]).

use std::ffi::{OsStr, OsString};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use super::{disk_file, regular};

/// Eine normale Datei in einem Ordner, wie [`regular_files`] sie findet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: OsString,
    pub len: u64,
    /// Die Zeit der letzten Änderung, ohne Angabe des Systems der Beginn
    /// der Unix-Zeit.
    pub modified: SystemTime,
}

/// So enden die Namen der Dateien, die [`replace`] beim Schreiben anlegt:
/// `.<Name>.<Prozess>.<Zähler>.tmp`, siehe [`replaced_name`].
const TEMPORARY_SUFFIX: &str = ".tmp";

/// Zählt die Dateien, die dieser Prozess beim Ersetzen anlegt.
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Unter Windows der geprüfte absolute Pfad, sonst `path` selbst.
fn checked(path: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        super::windows_path::resolve(path)
    }
    #[cfg(not(windows))]
    {
        Ok(path.to_path_buf())
    }
}

fn not_directory() -> io::Error {
    io::Error::new(io::ErrorKind::NotADirectory, "kein Ordner")
}

/// `name`, wenn es genau ein gewöhnlicher Bestandteil eines Pfads ist.
fn single(name: &OsStr) -> io::Result<&OsStr> {
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(part)), None) if part == name => Ok(name),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "kein einzelner Dateiname",
        )),
    }
}

/// Legt den Ordner `path` an, wenn es ihn noch nicht gibt, mit allen
/// fehlenden Ordnern darüber. Ein Verweis auf einen Ordner genügt auch.
///
/// Fehler: [`io::ErrorKind::NotADirectory`], wenn dort etwas anderes
/// steht, sonst die Fehler des Systems.
pub fn create_directory(path: &Path) -> io::Result<()> {
    let path = checked(path)?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(not_directory()),
        Err(error) => return Err(error),
    }
    if std::fs::metadata(&path)?.is_dir() {
        Ok(())
    } else {
        Err(not_directory())
    }
}

/// Die normalen Dateien im Ordner `directory`, in keiner bestimmten
/// Reihenfolge. Verweise, Ordner und Spezialdateien fehlen, ebenso
/// Einträge, deren Angaben sich nicht lesen lassen.
pub fn regular_files(directory: &Path) -> io::Result<Vec<FileEntry>> {
    let directory = checked(directory)?;
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&directory)? {
        let Ok(entry) = entry else {
            continue;
        };
        // Die Angaben des Eintrags selbst, ohne einem Verweis zu folgen.
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        files.push(FileEntry {
            name: entry.file_name(),
            len: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    Ok(files)
}

/// Wenn `name` eine Datei ist, die [`replace`] beim Schreiben anlegt, der
/// Name der Datei, die sie ersetzen sollte. Bleibt eine solche Datei nach
/// einem Absturz liegen, darf, wer den Ordner verwaltet, sie entfernen.
/// Andere Namen, auch ähnliche, ergeben `None`.
pub fn replaced_name(name: &OsStr) -> Option<&OsStr> {
    let number = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let rest = name
        .to_str()?
        .strip_prefix('.')?
        .strip_suffix(TEMPORARY_SUFFIX)?;
    let (rest, counter) = rest.rsplit_once('.')?;
    let (target, process) = rest.rsplit_once('.')?;
    if !number(counter) || !number(process) {
        return None;
    }
    single(OsStr::new(target)).ok()
}

/// Schreibt `bytes` als Datei `name` in den Ordner `directory` und ersetzt
/// eine alte Datei dieses Namens als Ganzes.
///
/// Fehler: [`io::ErrorKind::InvalidInput`], wenn `name` kein einzelner
/// Dateiname ist, sonst die Fehler des Systems. Dann bleibt die alte Datei,
/// wie sie war, und keine neue liegen.
pub fn replace(directory: &Path, name: &OsStr, bytes: &[u8]) -> io::Result<()> {
    let name = single(name)?;
    let target = checked(&directory.join(name))?;
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(
        ".{}.{}{TEMPORARY_SUFFIX}",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = target.with_file_name(temporary);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Scheitert schon das Anlegen, gehört dort nichts uns.
    let mut file = options.open(&temporary)?;
    let written = file.write_all(bytes);
    drop(file);
    let written = written.and_then(|()| std::fs::rename(&temporary, &target));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// Entfernt die Datei `name` aus dem Ordner `directory`. Ist sie ein
/// Verweis, verschwindet der Verweis, nicht sein Ziel.
///
/// Fehler: [`io::ErrorKind::InvalidInput`], wenn `name` kein einzelner
/// Dateiname ist, sonst die Fehler des Systems.
pub fn remove(directory: &Path, name: &OsStr) -> io::Result<()> {
    let name = single(name)?;
    std::fs::remove_file(checked(&directory.join(name))?)
}

/// Setzt die Zeit der letzten Änderung der normalen Datei `path`, etwa
/// damit ein Zwischenspeicher auch nach einem Neustart weiß, welche Datei
/// zuletzt gebraucht wurde. Geprüft wird wie beim Lesen vor dem Öffnen und
/// am offenen Handle.
///
/// Fehler: [`io::ErrorKind::InvalidInput`] für Spezialdateien wie Geräte,
/// Pipes und Ordner, sonst die Fehler des Systems.
pub fn set_modified(path: &Path, time: SystemTime) -> io::Result<()> {
    let path = checked(path)?;
    regular(&std::fs::metadata(&path)?)?;
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(super::OPEN_FLAGS);
    }
    let file = options.open(&path)?;
    regular(&file.metadata()?)?;
    disk_file(&file)?;
    file.set_modified(time)
}

#[cfg(test)]
mod tests;
