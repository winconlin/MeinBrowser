//! Der absturzsichere Speicherkern für alles, was dauerhaft gespeichert wird.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.12. Bisher gibt es nur einen
//! Zwischenspeicher auf der Platte ([`cache`]); der Kern mit Schreib-Log
//! und B-Baum fehlt noch.

#![forbid(unsafe_code)]

pub mod cache;
