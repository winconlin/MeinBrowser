//! Zustandsbasierte Prüfungen der bereits vorhandenen Laufzeit ohne Interpreter.
//!
//! Das erste Byte modulo fünf wählt Heap-Graphen, gewöhnliche Objekte,
//! Arrays, numerische oder Text-/Schlüsselumwandlungen. Vierergruppen sind
//! Operationen; unvollständige Gruppen bleiben unbenutzt. Jeder Modus prüft
//! denselben Ablauf mit normalen und erzwungenen GC-Zeitpunkten gegen ein
//! unabhängiges Modell beziehungsweise berechenbare Erwartungswerte.

mod arrays;
mod conversions;
mod heap_graph;
mod objects;
mod text_keys;

pub fn js_runtime(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let (mode, commands) = data.split_first().unwrap_or((&0, &[]));
    match mode % 5 {
        0 => heap_graph::exercise(commands),
        1 => objects::exercise(commands),
        2 => arrays::exercise(commands),
        3 => conversions::exercise(commands),
        _ => text_keys::exercise(commands),
    }
}
