//! Wo Einträge liegen: die Schnittstelle und ein Speicher im
//! Arbeitsspeicher.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, PoisonError};

use mb_crypto::Digest;

/// Ein Speicher für Einträge nach ihrem Schlüssel ([`super::key`]).
///
/// Er darf Einträge jederzeit vergessen, und was er liefert, muss nicht
/// stimmen: Der Agent prüft jeden Eintrag beim Laden. Mehrere Agents in
/// verschiedenen Fäden dürfen sich einen Speicher teilen.
pub trait CodeStore: Send + Sync {
    /// Der Eintrag zum Schlüssel, wenn es einen gibt.
    fn load(&self, key: &Digest) -> Option<Vec<u8>>;

    /// Legt einen Eintrag ab und ersetzt einen alten zum selben Schlüssel.
    fn store(&self, key: &Digest, bytes: &[u8]);
}

/// So viel rechnet [`MemoryStore`] je Eintrag zusätzlich zu seinen Bytes.
/// Gewählt, nicht gemessen.
const ENTRY_OVERHEAD: usize = 64;

/// Ein Speicher im Arbeitsspeicher mit einer Grenze in Bytes. Wird es mehr,
/// fällt der Eintrag heraus, der am längsten nicht gebraucht wurde.
#[derive(Debug)]
pub struct MemoryStore {
    limit: usize,
    entries: Mutex<Entries>,
}

#[derive(Debug, Default)]
struct Entries {
    /// Nach dem Schlüssel: wann der Eintrag zuletzt gebraucht wurde und
    /// seine Bytes.
    by_key: HashMap<Digest, (u64, Vec<u8>)>,
    /// Die Schlüssel nach dem Zeitpunkt, also der älteste zuerst.
    by_use: BTreeMap<u64, Digest>,
    next: u64,
    bytes: usize,
}

impl MemoryStore {
    /// Die Grenze, wenn keine angegeben ist: 32 MiB. Gewählt, nicht
    /// gemessen, wie die für die Programme regulärer Ausdrücke.
    pub const DEFAULT_LIMIT: usize = 32 << 20;

    /// Ein leerer Speicher, der höchstens `limit` Bytes hält.
    pub fn new(limit: usize) -> MemoryStore {
        MemoryStore {
            limit,
            entries: Mutex::new(Entries::default()),
        }
    }

    /// Wie viele Einträge er gerade hält.
    pub fn len(&self) -> usize {
        self.lock().by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Wie viele Bytes er gerade rechnet, mit dem Zuschlag je Eintrag.
    pub fn bytes(&self) -> usize {
        self.lock().bytes
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Entries> {
        // Ein Faden, der beim Ablegen abbrach, hinterlässt die Tabellen
        // stimmig: Jede Änderung ist vollständig, bevor etwas scheitern kann.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for MemoryStore {
    fn default() -> MemoryStore {
        MemoryStore::new(MemoryStore::DEFAULT_LIMIT)
    }
}

impl Entries {
    fn touch(&mut self) -> u64 {
        let now = self.next;
        self.next = self.next.saturating_add(1);
        now
    }

    fn remove(&mut self, key: &Digest) {
        if let Some((used, bytes)) = self.by_key.remove(key) {
            self.by_use.remove(&used);
            self.bytes = self
                .bytes
                .saturating_sub(bytes.len().saturating_add(ENTRY_OVERHEAD));
        }
    }
}

impl CodeStore for MemoryStore {
    fn load(&self, key: &Digest) -> Option<Vec<u8>> {
        let mut entries = self.lock();
        let now = entries.touch();
        let (used, bytes) = entries.by_key.get_mut(key)?;
        let before = std::mem::replace(used, now);
        let bytes = bytes.clone();
        entries.by_use.remove(&before);
        entries.by_use.insert(now, *key);
        Some(bytes)
    }

    fn store(&self, key: &Digest, bytes: &[u8]) {
        let size = bytes.len().saturating_add(ENTRY_OVERHEAD);
        let mut entries = self.lock();
        entries.remove(key);
        if size > self.limit {
            return;
        }
        while entries.bytes.saturating_add(size) > self.limit {
            let Some((_, oldest)) = entries.by_use.pop_first() else {
                break;
            };
            if let Some((_, bytes)) = entries.by_key.remove(&oldest) {
                entries.bytes = entries
                    .bytes
                    .saturating_sub(bytes.len().saturating_add(ENTRY_OVERHEAD));
            }
        }
        let now = entries.touch();
        entries.by_key.insert(*key, (now, bytes.to_vec()));
        entries.by_use.insert(now, *key);
        entries.bytes = entries.bytes.saturating_add(size);
    }
}
