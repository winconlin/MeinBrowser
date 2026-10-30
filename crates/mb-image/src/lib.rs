//! Bildformate.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.13. Bisher gibt es PNG ohne
//! Animation ([`png`]) und das erste Einzelbild eines GIF ([`gif`]).

#![forbid(unsafe_code)]

pub mod gif;
pub mod png;
