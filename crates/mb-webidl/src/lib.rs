//! WebIDL-Leser und Generator für die Bindings. Tokenizer und Syntaxparser
//! sowie ein quellenübergreifender Namensindex und eine strukturierte
//! Memberbeschreibung mit ersten Prüfungen sind vorhanden. Vollständige
//! Typprüfung und Binding-Generator folgen.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.10.

#![forbid(unsafe_code)]

pub mod index;
pub mod lexer;
pub mod members;
pub mod parser;
