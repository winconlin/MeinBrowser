//! CPU-Renderer, zugleich Referenz für den GPU-Renderer.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.8. Bisher gibt es eine
//! Zeichenfläche mit Rechtecken, geglätteten Kreisen und Pfaden sowie das
//! Testbild, das die Plattformschicht in einem Fenster zeigt.

#![forbid(unsafe_code)]

mod canvas;
mod path;
mod test_image;

pub use canvas::{Canvas, Color, TooLarge};
pub use path::Path;
pub use test_image::{PROBES, Probe, probe_position, test_image};
