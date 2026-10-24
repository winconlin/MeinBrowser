//! Arenen, typisierte Indizes, Atome, Zeichenketten, Bitmengen und Speicherzähler.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.1. Bisher gibt es die
//! Zeichenketten für DOM und JavaScript ([`DomString`]).

#![forbid(unsafe_code)]

pub mod string;

pub use string::{CodePoint, DomString};
