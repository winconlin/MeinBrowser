//! Welche lokalen Dateien eine Seite lesen darf.
//!
//! Entschieden am 30.10.2026: Eine lokale Datei, die der Nutzer öffnet,
//! darf Stylesheets und Bilder aus jeder lokalen Datei nachladen, aber nur
//! aus normalen Dateien, keine Geräte, Pipes oder Ordner, und je Datei
//! höchstens [`MAX_FILE_BYTES`]; symbolische Verweise zählen mit ihrem Ziel
//! (siehe `mb_platform::files`). Gelesen werden nur `file:`-URLs ohne Host,
//! unter Windows nur Pfade mit Laufwerk, also keine Freigaben im Netz und
//! keine Gerätepfade wie `//./pipe/…`.
//!
//! Seiten aus dem Netz bekommen diesen Zugriff nie: Ihr [`crate::Fetch`]
//! wird keine `file:`-URLs kennen (ab M3).

use std::path::PathBuf;

use mb_platform::files::read_regular;
use mb_url::Url;

/// So groß darf eine lokale Datei höchstens sein, die eine Seite liest.
/// Festgelegt von diesem Browser, wie das Budget der Seite
/// ([`crate::PAGE_BYTES`]).
pub const MAX_FILE_BYTES: u64 = 64 << 20;

/// Liest eine `file:`-URL aus dem Dateisystem, wenn sie eine normale Datei
/// mit höchstens [`MAX_FILE_BYTES`] bezeichnet.
pub fn read_file(url: &Url) -> Option<Vec<u8>> {
    read_file_within(url, MAX_FILE_BYTES)
}

/// Wie [`read_file`], aber mit der kleineren Grenze aus `limit` und
/// [`MAX_FILE_BYTES`]. Bei null werden weder Pfad noch Datei abgefragt.
/// Beim Lesen einer wachsenden Datei ist ein zusätzliches Byte nötig, um
/// die Überschreitung zu erkennen; der Puffer wird dann verworfen.
pub fn read_file_within(url: &Url, limit: u64) -> Option<Vec<u8>> {
    if limit == 0 {
        return None;
    }
    let path = file_path(url)?;
    read_regular(&path, limit.min(MAX_FILE_BYTES)).ok()
}

/// Der Pfad einer `file:`-URL. Hosts in `file:`-URLs (UNC) gibt es noch
/// nicht.
fn file_path(url: &Url) -> Option<PathBuf> {
    if url.scheme() != "file" || !url.hostname().is_empty() {
        return None;
    }
    let path =
        String::from_utf8(mb_url::percent::percent_decode(url.pathname().as_bytes())).ok()?;
    if cfg!(windows) {
        // Unter Windows beginnt der Pfad mit dem Laufwerk: `/C:/…`. Alles
        // andere wären Freigaben (`//Rechner/…`) oder Geräte (`//./…`).
        let drive = matches!(path.as_bytes(), [b'/', letter, b':', b'/', ..] if letter.is_ascii_alphabetic());
        return drive.then(|| PathBuf::from(path.get(1..).unwrap_or_default()));
    }
    Some(PathBuf::from(path))
}
