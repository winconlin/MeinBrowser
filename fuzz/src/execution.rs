//! Compilerprüfung beliebiger Eingaben und Ausführung begrenzter Vorlagen.
//!
//! Beliebiger Eingabetext wird nur kompiliert. Der Interpreter erhält
//! Quelltext aus festen Vorlagen mit begrenzten Zahlenparametern.

mod compiler;
mod programs;

pub fn js_execute(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    compiler::exercise(data);
    programs::exercise(data);
}
