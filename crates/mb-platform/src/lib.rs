//! Die einzige Stelle, die mit Betriebssystem und Grafiktreibern spricht: Fenster, Eingabe, Ein-/Ausgabe, GPU-Oberfläche, Sandbox. Die FFI-Deklarationen sind selbst geschrieben.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.2.
//! `unsafe` ist hier für die FFI erlaubt, siehe Abschnitt 4.14.
//!
//! Für M0 gibt es Fenster mit Eingabe und CPU-Bildausgabe:
//!
//! | System | Umsetzung |
//! |---|---|
//! | Linux und BSD | eigener X11-Client über das dokumentierte Protokoll (ohne `libX11` und `libxcb`) |
//! | Windows | Win32 und GDI |
//! | macOS | AppKit über die Objective-C-Laufzeit, Bildausgabe über Core Animation |
//!
//! Ein Fenster liefert seine Ereignisse über [`Window::next_event`]. Das erste
//! Ereignis ist immer [`Event::Resized`] mit der tatsächlichen Größe.
//!
//! Dazu kommen die Verzeichnisse der Systemschriften ([`fonts`]).

mod event;
pub mod fonts;

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
mod unix;
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
mod x11;
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
use x11 as backend;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as backend;

#[cfg(not(any(
    windows,
    target_os = "macos",
    all(unix, not(any(target_os = "ios", target_os = "android")))
)))]
mod unsupported;
#[cfg(not(any(
    windows,
    target_os = "macos",
    all(unix, not(any(target_os = "ios", target_os = "android")))
)))]
use unsupported as backend;

use std::fmt;
use std::time::Duration;

pub use event::{Event, Key, KeyEvent, Modifiers, NamedKey, PointerButton, ScrollDelta};

/// Fehler der Plattformschicht.
#[derive(Debug)]
pub enum Error {
    /// Auf diesem System oder mit dieser Konfiguration nicht verfügbar.
    Unsupported(String),
    /// Ein- oder Ausgabefehler, zum Beispiel beim Verbinden mit dem X-Server.
    Io(std::io::Error),
    /// Die Gegenseite hat etwas Unerwartetes gesendet oder einen Fehler gemeldet.
    Protocol(String),
    /// Ein Systemaufruf ist gescheitert.
    System { function: &'static str, code: i64 },
    /// Ungültige Argumente, zum Beispiel ein Bild mit falscher Pixelzahl.
    InvalidArgument(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(what) => write!(f, "nicht unterstützt: {what}"),
            Error::Io(err) => write!(f, "Ein-/Ausgabefehler: {err}"),
            Error::Protocol(what) => write!(f, "Protokollfehler: {what}"),
            Error::System { function, code } => {
                write!(f, "Systemaufruf {function} gescheitert (Fehlercode {code})")
            }
            Error::InvalidArgument(what) => write!(f, "ungültiges Argument: {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::Io(err)
    }
}

/// Einstellungen für ein neues Fenster.
#[derive(Clone, Debug)]
pub struct WindowOptions<'a> {
    pub title: &'a str,
    /// Gewünschte Breite der Zeichenfläche in Gerätepixeln.
    pub width: u32,
    /// Gewünschte Höhe der Zeichenfläche in Gerätepixeln.
    pub height: u32,
}

/// Ein Bild, das mit [`Window::capture`] aus einem Fenster zurückgelesen wurde.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub width: u32,
    pub height: u32,
    /// Pixel im Format `0x00RRGGBB`, zeilenweise von oben nach unten.
    pub pixels: Vec<u32>,
}

impl Capture {
    /// Das Pixel an `(x, y)` als `0x00RRGGBB`.
    pub fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = (y as usize)
            .checked_mul(self.width as usize)?
            .checked_add(x as usize)?;
        self.pixels.get(index).copied()
    }
}

/// Ein Fenster mit einer Zeichenfläche, die vom Programm selbst gefüllt wird.
pub struct Window {
    inner: backend::Window,
}

impl fmt::Debug for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (width, height) = self.size();
        f.debug_struct("Window")
            .field("width", &width)
            .field("height", &height)
            .finish_non_exhaustive()
    }
}

impl Window {
    /// Öffnet ein Fenster und zeigt es an.
    pub fn open(options: &WindowOptions<'_>) -> Result<Window, Error> {
        if options.width == 0 || options.height == 0 {
            return Err(Error::InvalidArgument(format!(
                "Fenstergröße {}×{}",
                options.width, options.height
            )));
        }
        Ok(Window {
            inner: backend::Window::open(options)?,
        })
    }

    /// Größe der Zeichenfläche in Gerätepixeln.
    pub fn size(&self) -> (u32, u32) {
        self.inner.size()
    }

    /// Zeigt ein Bild in der linken oberen Ecke der Zeichenfläche an.
    ///
    /// `pixels` enthält `width * height` Pixel im Format `0x00RRGGBB` (das
    /// oberste Byte wird ignoriert), zeilenweise von oben nach unten.
    pub fn present(&mut self, width: u32, height: u32, pixels: &[u32]) -> Result<(), Error> {
        let expected = u64::from(width) * u64::from(height);
        if u64::try_from(pixels.len()).ok() != Some(expected) {
            return Err(Error::InvalidArgument(format!(
                "Bild {width}×{height} braucht {expected} Pixel, übergeben wurden {}",
                pixels.len()
            )));
        }
        if width == 0 || height == 0 {
            return Ok(());
        }
        self.inner.present(width, height, pixels)
    }

    /// Wartet auf das nächste Ereignis.
    ///
    /// Mit `timeout = None` wartet die Funktion unbegrenzt, sonst höchstens die
    /// angegebene Zeit und liefert dann `Ok(None)`. Mit `Duration::ZERO` holt
    /// sie nur ab, was schon vorliegt.
    pub fn next_event(&mut self, timeout: Option<Duration>) -> Result<Option<Event>, Error> {
        self.inner.next_event(timeout)
    }

    /// Liest den angezeigten Inhalt der Zeichenfläche zurück. Gedacht für
    /// Selbsttests: Damit lässt sich prüfen, ob ein Bild wirklich angekommen ist.
    pub fn capture(&mut self) -> Result<Capture, Error> {
        self.inner.capture()
    }
}
