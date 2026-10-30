//! Der gemeinsame Schriftlader muss die Datei bei jedem Lesen neu prüfen.

use super::*;

fn directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "mb-platform-font-loader-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("Verzeichnis anlegen");
    path
}

#[test]
fn discovered_font_bytes_reach_the_loader_unchanged() {
    let directory = directory("normal");
    let path = directory.join("normal.ttf");
    // Den Inhalt prüft danach mb-text; hier geht es um den Dateizugriff.
    let bytes = b"Schriftdaten\0\xff";
    std::fs::write(&path, bytes).expect("Datei anlegen");
    #[cfg(unix)]
    {
        let link = directory.join("verweis.otf");
        std::os::unix::fs::symlink(&path, &link).expect("Verweis anlegen");
        assert_eq!(read_font(&link), Some(bytes.to_vec()));
    }
    let files = list(std::slice::from_ref(&directory));
    assert_eq!(
        files,
        [std::fs::canonicalize(&path).expect("kanonischer Pfad")]
    );
    // Unter Windows liefert die Suche üblicherweise einen Verbatim-Pfad.
    assert_eq!(read_font(&files[0]), Some(bytes.to_vec()));
    std::fs::remove_dir_all(directory).expect("aufräumen");
}

#[test]
fn a_font_that_grows_after_discovery_is_rejected_on_reload() {
    let directory = directory("groesse");
    let path = directory.join("sammlung.ttc");
    std::fs::write(&path, b"klein").expect("Datei anlegen");
    let files = list(std::slice::from_ref(&directory));
    assert_eq!(files.len(), 1);
    assert_eq!(read_font(&files[0]), Some(b"klein".to_vec()));
    // Eine dünn belegte Datei vermeidet einen großen Testpuffer. Die
    // Dateischnittstelle muss sie schon anhand der Größe ablehnen.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("Datei öffnen")
        .set_len(MAX_FONT_BYTES + 1)
        .expect("Dateilänge setzen");
    assert!(read_font(&files[0]).is_none(), "zu große Schriftdatei");
    std::fs::remove_dir_all(directory).expect("aufräumen");
}

#[cfg(unix)]
#[test]
fn a_discovered_font_replaced_by_a_pipe_or_device_is_not_read() {
    let directory = directory("austausch");
    let path = directory.join("austausch.ttf");
    std::fs::write(&path, b"normal").expect("Datei anlegen");
    let files = list(std::slice::from_ref(&directory));
    assert_eq!(files.len(), 1);
    std::fs::remove_file(&path).expect("Datei entfernen");
    let status = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .expect("mkfifo ausführen");
    assert!(status.success());
    // Ein Schreiber sorgt dafür, dass auch ein versehentlich wieder
    // verwendetes fs::read endet und der Test fehlschlägt statt zu hängen.
    let mut writer = std::process::Command::new("sh")
        .args(["-c", "printf x > \"$1\"", "font-fifo-writer"])
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("Pipe-Schreiber starten");
    let bytes = read_font(&files[0]);
    let _ = writer.kill();
    writer.wait().expect("Pipe-Schreiber aufräumen");
    assert!(bytes.is_none(), "nach der Suche ersetzte Pipe");

    std::fs::remove_file(&path).expect("Pipe entfernen");
    // /dev/null ist auch bei einem Rückfall auf fs::read endlich; ein
    // solcher Rückfall lieferte Some([]) statt der geforderten Ablehnung.
    std::os::unix::fs::symlink("/dev/null", &path).expect("Geräteverweis anlegen");
    assert!(
        read_font(&files[0]).is_none(),
        "nach der Suche ersetztes Gerät"
    );
    std::fs::remove_dir_all(directory).expect("aufräumen");
}

#[cfg(windows)]
#[test]
fn the_font_loader_rejects_the_windows_null_device() {
    // Das wäre unter Windows trotz der Erweiterung ein Gerät. Ein
    // gewöhnliches fs::read könnte dafür erfolgreich leere Bytes liefern.
    assert_eq!(read_font(Path::new("NUL.ttf")), None);
}
