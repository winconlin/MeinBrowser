//! WebIDL-Leser und Generator für die Bindings. Tokenizer und Syntaxparser
//! sind vorhanden; semantische Prüfung und Binding-Generator folgen.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.10.

#![forbid(unsafe_code)]

pub mod lexer;
pub mod parser;
