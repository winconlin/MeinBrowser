//! Die übersetzten Programme der regulären Ausdrücke eines Agents.
//!
//! Ein RegExp-Objekt hält seinen Quelltext, seine Flags und die Nummer
//! seines Programms. Die Programme liegen außerhalb des Heaps, hier, und
//! zusammen nie über [`LIMIT`] Bytes: Wird es mehr, fallen die ältesten
//! heraus. Fehlt einem Objekt sein Programm, übersetzt der Aufrufer es neu
//! und bekommt eine neue Nummer. So braucht der Heap keine Finalizer, und
//! der Speicher bleibt begrenzt, wie viele Objekte es auch gibt.
//!
//! Gleicher Quelltext mit gleichen Flags teilt sich ein Programm. Ein
//! Literal in einer Schleife legt jedes Mal ein neues Objekt an, übersetzt
//! wird es nur beim ersten Mal.

use std::collections::BTreeMap;
use std::rc::Rc;

use super::program::Program;

/// So viele Bytes dürfen alle Programme eines Agents zusammen belegen. Ein
/// einzelnes Programm darf ebenso groß werden. Die Zahl ist gewählt, nicht
/// gemessen.
pub(crate) const LIMIT: usize = 16 << 20;

/// Was ein Programm bestimmt: der Quelltext und die Flags, die beim
/// Übersetzen zählen (`i`, `m`, `s`, `u`, `v` als Bits).
type Key = (Vec<u16>, u8);

struct Entry {
    program: Rc<Program>,
    key: Key,
    bytes: usize,
}

#[derive(Default)]
pub(crate) struct Programs {
    /// Nach der Nummer, also nach dem Alter.
    entries: BTreeMap<u64, Entry>,
    by_key: BTreeMap<Key, u64>,
    next: u64,
    bytes: usize,
}

impl Programs {
    pub(crate) fn get(&self, id: u64) -> Option<Rc<Program>> {
        self.entries.get(&id).map(|entry| Rc::clone(&entry.program))
    }

    /// Die Nummer des Programms zu `source` und `flags`, wenn es schon
    /// übersetzt ist.
    pub(crate) fn find(&self, source: &[u16], flags: u8) -> Option<u64> {
        // Der Schlüssel muss für die Suche einmal kopiert werden; das
        // kostet so viel wie der Vergleich.
        self.by_key.get(&(source.to_vec(), flags)).copied()
    }

    /// Nimmt ein Programm auf und gibt seine Nummer.
    pub(crate) fn insert(&mut self, source: &[u16], flags: u8, program: Program) -> u64 {
        let bytes = program
            .bytes()
            .saturating_add(source.len().saturating_mul(4));
        while self.bytes.saturating_add(bytes) > LIMIT {
            let Some((_, oldest)) = self.entries.pop_first() else {
                break;
            };
            self.by_key.remove(&oldest.key);
            self.bytes = self.bytes.saturating_sub(oldest.bytes);
        }
        let id = self.next;
        self.next = self.next.saturating_add(1);
        let key = (source.to_vec(), flags);
        if let Some(previous) = self.by_key.insert(key.clone(), id)
            && let Some(entry) = self.entries.remove(&previous)
        {
            self.bytes = self.bytes.saturating_sub(entry.bytes);
        }
        self.bytes = self.bytes.saturating_add(bytes);
        let program = Rc::new(program);
        self.entries.insert(
            id,
            Entry {
                program,
                key,
                bytes,
            },
        );
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(instructions: usize) -> Program {
        Program {
            instructions: vec![super::super::program::Instruction::Match; instructions],
            sets: Vec::new(),
            group_lists: Vec::new(),
            groups: 0,
            names: Vec::new(),
            registers: 0,
            unicode: false,
        }
    }

    #[test]
    fn programs_are_shared_and_the_oldest_fall_out() {
        let mut programs = Programs::default();
        let a = programs.insert(&[0x61], 0, program(1));
        assert_eq!(programs.find(&[0x61], 0), Some(a));
        assert_eq!(programs.find(&[0x61], 1), None);
        assert!(programs.get(a).is_some());
        // Ein Programm, das fast die ganze Grenze belegt, verdrängt das
        // ältere.
        let size = size_of::<super::super::program::Instruction>();
        let big = programs.insert(&[0x62], 0, program(LIMIT / size - 1));
        assert!(programs.get(a).is_none());
        assert_eq!(programs.find(&[0x61], 0), None);
        assert!(programs.get(big).is_some());
        assert!(programs.bytes <= LIMIT);
        let c = programs.insert(&[0x63], 0, program(1));
        assert!(programs.get(big).is_none() && programs.get(c).is_some());
        // Derselbe Schlüssel noch einmal ersetzt den alten Eintrag.
        let d = programs.insert(&[0x63], 0, program(2));
        assert!(programs.get(c).is_none());
        assert_eq!(programs.find(&[0x63], 0), Some(d));
        assert_eq!(programs.entries.len(), 1);
    }
}
