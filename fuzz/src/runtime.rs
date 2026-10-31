//! Zustandsbasierte Prüfungen der bereits vorhandenen Laufzeit ohne Interpreter.
//!
//! Das erste Byte modulo vier wählt Heap-Graphen, gewöhnliche Objekte,
//! Arrays oder primitive Typumwandlungen. Die folgenden Vierergruppen sind
//! Operationen; unvollständige Gruppen bleiben unbenutzt. Jeder Modus prüft
//! denselben Ablauf mit normalen und erzwungenen GC-Zeitpunkten gegen ein
//! unabhängiges Modell beziehungsweise berechenbare Erwartungswerte.

mod arrays;
mod conversions;
mod heap_graph;
mod objects;

pub fn js_runtime(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let (mode, commands) = data.split_first().unwrap_or((&0, &[]));
    match mode % 4 {
        0 => heap_graph::exercise(commands),
        1 => objects::exercise(commands),
        2 => arrays::exercise(commands),
        _ => conversions::exercise(commands),
    }
}
