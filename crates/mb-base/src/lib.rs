//! Arenen, typisierte Indizes, Atome, Zeichenketten, Bitmengen und Speicherzähler.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.1. Bisher gibt es die
//! Zeichenketten für DOM und JavaScript ([`DomString`]) und Arbeitsbudgets
//! für Aufträge, die fremde Inhalte verarbeiten ([`Budget`]).

#![forbid(unsafe_code)]

pub mod budget;
pub mod string;

pub use budget::{Budget, Cancel, Exhausted};
pub use string::{CodePoint, DomString};
