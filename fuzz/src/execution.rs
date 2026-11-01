//! Compilerprüfung, begrenzte Vorlagen und Ausführung unter einer Hostquote.
//!
//! Beliebiger Eingabetext wird zusätzlich mit kleinem Arbeits- und
//! Heapbudget ausgeführt; die Vorlagen behalten ihre unabhängigen Orakel.

mod bounded;
mod compiler;
mod programs;

pub fn js_execute(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    compiler::exercise(data);
    programs::exercise(data);
    bounded::exercise(data);
}
