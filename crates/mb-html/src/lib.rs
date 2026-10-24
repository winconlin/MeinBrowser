//! HTML-Tokenizer und Baumaufbau nach dem WHATWG HTML Standard.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.4. Grundlage ist die Datei
//! `source` aus `whatwg/html`, Commit
//! `b342939ba3b7e40041655fb926f6e41b54fe47ef` (22.10.2026), Kapitel
//! „Parsing HTML documents“.

#![forbid(unsafe_code)]

mod entities;
pub mod error;
pub mod tokenizer;
pub mod tree_builder;

pub use error::{ErrorCode, ParseError};
pub use tokenizer::{Attribute, Doctype, Tag, Token, Tokenizer, TokenizerState};
pub use tree_builder::{Options, ScriptingMode, parse_document, parse_fragment};
