//! Web-APIs, Ereignisschleife und Worker.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.10. Bisher gibt es nur das
//! Laden und Darstellen einer Seite ohne Skripte: Dokument, Stylesheets,
//! Bilder, Layout und Szene an einer Scrollposition ([`Page`]), dazu das
//! Lesen der lokalen Dateien, die eine Seite laden darf ([`read_file`]).

#![forbid(unsafe_code)]

mod files;
mod page;

#[cfg(test)]
mod tests;

pub use files::{MAX_FILE_BYTES, read_file};
pub use page::{Fetch, PAGE_BYTES, Page, RENDER_STEPS, Viewport, file_url};
