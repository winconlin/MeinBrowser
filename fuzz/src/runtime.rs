//! Zustandsbasierte Prüfungen der bereits vorhandenen Laufzeit ohne Interpreter.
//!
//! Das erste Byte wählt einen Heap-Graphen (gerade) oder gewöhnliche Objekte
//! (ungerade). Die folgenden Vierergruppen sind Operationen; unvollständige
//! Gruppen bleiben unbenutzt. Beide Modi prüfen denselben Ablauf mit normalen
//! und erzwungenen GC-Zeitpunkten gegen ein unabhängiges Inhaltsmodell.

mod heap_graph;
mod objects;

pub fn js_runtime(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let (mode, commands) = data.split_first().unwrap_or((&0, &[]));
    if mode & 1 == 0 {
        heap_graph::exercise(commands);
    } else {
        objects::exercise(commands);
    }
}
