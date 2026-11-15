//! WebIDL-Leser und Generator für die Bindings. Tokenizer und Syntaxparser
//! sowie ein quellenübergreifender Namensindex und eine strukturierte
//! Member- und Typbeschreibung mit ersten Prüfungen sind vorhanden.
//! Vollständige semantische Prüfung und Binding-Generator folgen.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.10.

#![forbid(unsafe_code)]

pub mod index;
pub mod lexer;
pub mod members;
pub mod parser;
pub mod types;
