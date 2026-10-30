//! CPU-Renderer, zugleich Referenz für den GPU-Renderer.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.8. Bisher gibt es eine
//! Zeichenfläche mit Rechtecken, geglätteten Kreisen und Pfaden, das
//! Zeichnen einer Szene aus `mb-scene` ([`render`], mit mehreren
//! Gerätepixeln je CSS-Pixel [`render_scaled`]) sowie das Testbild, das die
//! Plattformschicht in einem Fenster zeigt.

#![forbid(unsafe_code)]

mod canvas;
mod path;
mod scene;
mod test_image;

pub use canvas::{Canvas, Color, TooLarge};
pub use path::Path;
pub use scene::{render, render_scaled};
pub use test_image::{PROBES, Probe, probe_position, test_image};
