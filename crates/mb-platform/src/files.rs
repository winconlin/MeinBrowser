//! Lokale Dateien sicher lesen: nur normale Dateien, ohne an
//! Spezialdateien hängen zu bleiben, und mit einer Grenze für die Bytes.
//!
//! Ein Pfad allein reicht dafür nicht: Zwischen einer Prüfung und dem
//! Öffnen kann ein anderer Prozess einen Verweis umbiegen. Darum wird der
//! Pfad vor dem Öffnen geprüft, damit Geräte und Pipes gar nicht erst
//! geöffnet werden, und danach noch einmal am offenen Handle; gelesen wird
//! nur aus dem geprüften Handle. Unter Windows zählt am Handle die Art aus
//! `GetFileType`, denn Geräte wie `NUL` gelten dort sonst als Dateien.
//!
//! Unter Unix öffnet `O_NONBLOCK`, damit eine benannte Pipe beim Öffnen
//! nicht auf einen Schreiber wartet, und `O_NOCTTY`, damit ein Terminal
//! nicht zum steuernden Terminal wird. Die Werte stammen aus den
//! Systemköpfen (Linux `asm-generic/fcntl.h`, macOS `bsd/sys/fcntl.h` aus
//! xnu, FreeBSD `sys/sys/fcntl.h`). Auf anderen Unix-Systemen fehlen sie
//! noch; dort schützt nur die Prüfung des Pfads vor dem Öffnen.
//!
//! Unter Windows werden Gerätealiase wie `COM1` vor jedem Dateizugriff
//! abgewiesen, auch vor der Metadatenabfrage. Erlaubt sind
//! Laufwerkspfade, einschließlich langer Pfade mit `\\?\C:\…`, keine
//! UNC- oder Geräte-Namensräume und keine alternativen Datenströme.
//! Relative Pfade werden ohne Dateizugriff aufgelöst und nochmals geprüft.
//! Die Prüfung am Handle bleibt nötig; eine lexikalische Prüfung verhindert
//! keine Rennen mit Verweisen oder Reparse Points im Dateisystem.

#[cfg(any(windows, test))]
mod windows_path;

use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "android"))]
const OPEN_FLAGS: i32 = 0o4000 | 0o400;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const OPEN_FLAGS: i32 = 0x0004 | 0x0002_0000;
#[cfg(target_os = "freebsd")]
const OPEN_FLAGS: i32 = 0x0004 | 0x8000;
#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd"
    ))
))]
const OPEN_FLAGS: i32 = 0;

/// Liest eine normale Datei mit höchstens `limit` Bytes. Symbolische
/// Verweise zählen mit ihrem Ziel.
///
/// Unter Windows nur Laufwerkspfade: keine UNC-/Gerätepfade, reservierten
/// DOS-Gerätenamen oder alternativen Datenströme. Relative Pfade und die
/// lange Form `\\?\C:\…` sind erlaubt. Die Pfadprüfung findet vor der
/// Metadatenabfrage statt; sie ersetzt keine Sandbox.
///
/// Fehler: [`io::ErrorKind::InvalidInput`] für Spezialdateien wie Geräte,
/// Pipes und Ordner, [`io::ErrorKind::FileTooLarge`] über der Grenze, sonst
/// die Fehler des Systems.
pub fn read_regular(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    #[cfg(windows)]
    let absolute = windows_path::resolve(path)?;
    #[cfg(windows)]
    let path = absolute.as_path();

    // Spezialdateien gar nicht erst öffnen; die Prüfung am Handle danach
    // fängt ab, was sich inzwischen geändert hat.
    regular(&std::fs::metadata(path)?)?;
    let file = open(path)?;
    let opened = file.metadata()?;
    regular(&opened)?;
    disk_file(&file)?;
    if opened.len() > limit {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > limit) {
        return Err(too_large());
    }
    Ok(bytes)
}

fn too_large() -> io::Error {
    io::Error::new(io::ErrorKind::FileTooLarge, "zu groß")
}

fn not_regular() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "keine normale Datei")
}

fn regular(metadata: &Metadata) -> io::Result<()> {
    if metadata.is_file() {
        Ok(())
    } else {
        Err(not_regular())
    }
}

#[cfg(unix)]
fn open(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_FLAGS)
        .open(path)
}

#[cfg(not(unix))]
fn open(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Unter Windows: ob das Handle zu einer Datei auf einem Datenträger gehört.
#[cfg(windows)]
fn disk_file(file: &File) -> io::Result<()> {
    if crate::windows::is_disk_file(file) {
        Ok(())
    } else {
        Err(not_regular())
    }
}

/// Anderswo genügt die Art aus den Metadaten des Handles.
#[cfg(not(windows))]
fn disk_file(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Ein eigener Ordner je Test (RS-14).
    fn directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("mb-platform-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("Ordner");
        directory
    }

    #[test]
    fn regular_files_up_to_the_limit() {
        let directory = directory("normal");
        let file = directory.join("a.css");
        std::fs::write(&file, b"p {}").expect("schreiben");
        assert_eq!(read_regular(&file, 4).expect("lesen"), b"p {}");
        let error = read_regular(&file, 3).expect_err("zu groß");
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
        let error = read_regular(&directory, 100).expect_err("Ordner");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        std::fs::remove_dir_all(&directory).expect("aufräumen");
    }

    #[cfg(unix)]
    #[test]
    fn devices_are_refused_and_links_count_with_their_target() {
        let directory = directory("speziell");
        let error = read_regular(Path::new("/dev/zero"), 100).expect_err("Gerät");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let target = directory.join("b.css");
        std::fs::write(&target, b"b {}").expect("schreiben");
        let link = directory.join("verweis.css");
        std::os::unix::fs::symlink(&target, &link).expect("Verweis");
        assert_eq!(read_regular(&link, 100).expect("lesen"), b"b {}");
        let device = directory.join("verweis-geraet");
        std::os::unix::fs::symlink("/dev/zero", &device).expect("Verweis");
        let error = read_regular(&device, 100).expect_err("Verweis auf ein Gerät");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        std::fs::remove_dir_all(&directory).expect("aufräumen");
    }

    #[cfg(unix)]
    #[test]
    fn windows_device_names_are_ordinary_names_on_unix() {
        let directory = directory("windows-namen");
        for name in ["NUL", "COM1.txt", "CONIN$", "COM¹", "name:stream"] {
            let path = directory.join(name);
            std::fs::write(&path, b"normal").expect("schreiben");
            assert_eq!(read_regular(&path, 6).expect("lesen"), b"normal", "{name}");
        }
        std::fs::remove_dir_all(&directory).expect("aufräumen");
    }

    #[cfg(windows)]
    #[test]
    fn windows_special_paths_are_rejected_before_metadata() {
        let directory = directory("windows-pfade");
        let missing = directory.join("nicht-angelegt");
        // Unter dem fehlenden Elternordner müsste metadata NotFound
        // liefern. InvalidInput belegt die vorgezogene Pfadprüfung.
        for name in [
            "NUL",
            "nul.txt",
            "NUL .css",
            "CON",
            "PRN",
            "AUX",
            "COM1",
            "cOm9.html",
            "LPT1",
            "lpt9.txt",
            "COM¹",
            "LPT²",
            "COM³",
            "CONIN$",
            "CONOUT$",
            "CLOCK$",
            "normal.css:stream",
        ] {
            let path = missing.join(name);
            let error = read_regular(&path, 100).expect_err("Spezialpfad");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{path:?}");
        }
        for path in [
            r"\\.\NUL",
            r"\\.\pipe\meinbrowser",
            r"\\?\GLOBALROOT\Device\Null",
            r"\\?\UNC\server\share\file",
            r"\\server\pipe\meinbrowser",
        ] {
            let error = read_regular(Path::new(path), 100).expect_err("Namensraum");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{path}");
        }
        let error = read_regular(&missing.join("normal.css"), 100).expect_err("fehlt");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        std::fs::remove_dir_all(&directory).expect("aufräumen");
    }

    #[cfg(windows)]
    #[test]
    fn windows_verbatim_disk_paths_remain_readable() {
        let directory = directory("windows-lang");
        let path = directory.join("normal.css");
        std::fs::write(&path, b"p {}").expect("schreiben");
        // canonicalize wird nur im Test auf der selbst angelegten Datei
        // verwendet, nicht in der Sicherheitsprüfung fremder Pfade.
        let verbatim = std::fs::canonicalize(&path).expect("langer Laufwerkspfad");
        assert_eq!(read_regular(&verbatim, 4).expect("lesen"), b"p {}");
        std::fs::remove_dir_all(&directory).expect("aufräumen");
    }
}
