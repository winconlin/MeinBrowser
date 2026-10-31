//! Begrenzter Cache für die Maße einer Schriftanfrage bei einem Pixel.
//!
//! Map und LRU teilen sich den Schlüssel, damit lange Familiennamen nicht
//! doppelt gehalten werden. Gezählt werden Schlüssel, Werte, die Kapazität
//! der Familienliste und ihrer Zeichenketten. Verwaltungsstrukturen der
//! Container und des Allokators sind nicht Teil dieses Bytezählers. Die
//! Eintragsgrenze beschränkt zusätzlich die Containerverwaltung. Die Grenzen
//! gelten nicht für vorübergehende Anfragen oder für die Schriftbibliothek.

use std::collections::{BTreeMap, VecDeque};
use std::mem::size_of;
use std::sync::Arc;

use super::{FamilyKey, Metrics, QueryKey};

#[cfg(test)]
mod tests;

const MAX_ENTRIES: usize = 256;
const MAX_BYTES: usize = 256 << 10;

struct Entry {
    value: Option<Metrics>,
    bytes: usize,
}

pub(super) struct MetricsCache {
    entries: BTreeMap<Arc<QueryKey>, Entry>,
    order: VecDeque<Arc<QueryKey>>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl MetricsCache {
    pub(super) fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            max_entries: MAX_ENTRIES,
            max_bytes: MAX_BYTES,
        }
    }

    #[cfg(test)]
    pub(super) fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries,
            max_bytes,
            ..Self::new()
        }
    }

    /// Auch `Some(None)` ist ein Treffer: Es gibt keine passende Schrift.
    pub(super) fn get(&mut self, key: &QueryKey) -> Option<Option<Metrics>> {
        let (stored, entry) = self.entries.get_key_value(key)?;
        let stored = Arc::clone(stored);
        let value = entry.value;
        if !self
            .order
            .back()
            .is_some_and(|last| Arc::ptr_eq(last, &stored))
        {
            let position = self
                .order
                .iter()
                .position(|item| Arc::ptr_eq(item, &stored))
                .expect("jeder Cache-Eintrag hat einen LRU-Platz");
            self.order.remove(position);
            self.order.push_back(stored);
        }
        Some(value)
    }

    pub(super) fn insert(&mut self, key: QueryKey, value: Option<Metrics>) {
        if let Some(entry) = self.entries.get_mut(&key) {
            // Der vorhandene Schlüssel mit seiner Kapazität bleibt liegen.
            entry.value = value;
            let _ = self.get(&key);
            return;
        }
        let Some(bytes) = key_bytes(&key) else { return };
        // Eine einzelne zu große Anfrage darf den nützlichen Cache nicht
        // leeren. Solche Werte werden berechnet, aber nicht aufbewahrt.
        if self.max_entries == 0 || bytes > self.max_bytes {
            return;
        }
        while self.entries.len() >= self.max_entries || bytes > self.max_bytes - self.bytes {
            let oldest = self
                .order
                .pop_front()
                .expect("zu verdrängender Cache-Eintrag");
            let removed = self
                .entries
                .remove(oldest.as_ref())
                .expect("LRU-Eintrag im Cache");
            self.bytes -= removed.bytes;
        }
        let key = Arc::new(key);
        self.entries
            .insert(Arc::clone(&key), Entry { value, bytes });
        self.order.push_back(key);
        self.bytes += bytes;
    }
}

fn key_bytes(key: &QueryKey) -> Option<usize> {
    let fixed = size_of::<QueryKey>()
        .checked_add(size_of::<Entry>())?
        .checked_add(2 * size_of::<Arc<QueryKey>>())?
        .checked_add(2 * size_of::<usize>())?;
    let families = key
        .families
        .capacity()
        .checked_mul(size_of::<FamilyKey>())?;
    key.families
        .iter()
        .try_fold(fixed.checked_add(families)?, |total, family| {
            let bytes = match family {
                FamilyKey::Name(name) => name.capacity(),
                FamilyKey::Generic(_) => 0,
            };
            total.checked_add(bytes)
        })
}
