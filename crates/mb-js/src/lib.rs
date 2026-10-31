//! JavaScript-Engine. `unsafe` ist hier für JIT und Garbage Collector erlaubt.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.9.

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod regexp;
