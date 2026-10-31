//! JavaScript-Engine. `unsafe` ist hier für JIT und Garbage Collector erlaubt.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.9.

pub mod agent;
pub mod ast;
pub mod bytecode;
pub mod compare;
pub mod compiler;
pub mod conversion;
pub mod descriptor;
pub mod function;
pub mod heap;
pub mod interpreter;
pub mod key;
pub mod lexer;
pub mod number;
pub mod object;
pub mod operations;
pub mod operators;
pub mod parser;
pub mod realm;
pub mod reference;
pub mod regexp;
pub mod script;
pub mod value;
