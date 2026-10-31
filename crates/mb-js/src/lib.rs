//! JavaScript-Engine. `unsafe` ist hier für JIT und Garbage Collector erlaubt.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.9.

pub mod agent;
pub mod ast;
pub mod compare;
pub mod descriptor;
pub mod heap;
pub mod key;
pub mod lexer;
pub mod object;
pub mod operations;
pub mod parser;
pub mod regexp;
pub mod value;
