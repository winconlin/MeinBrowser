//! Web-APIs, Ereignisschleife und Worker.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.10. Bisher gibt es nur das
//! Laden und Darstellen einer Seite ohne Skripte: Dokument, Stylesheets,
//! Bilder, Layout und Szene ([`Page`]).

#![forbid(unsafe_code)]

mod page;

#[cfg(test)]
mod tests;

pub use page::{Fetch, Page, file_url, read_file};
