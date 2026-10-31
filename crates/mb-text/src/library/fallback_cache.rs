//! Begrenzter Ergebnis-Cache für die Ersatzschriftsuche.
//!
//! Höchstens 256 Suchergebnisse, je Schlüssel höchstens 64 Codepunkte.
//! Größere Cluster werden unverändert gesucht, aber nicht gespeichert.
//! Die Familienliste ist kein Schlüsselteil: Die Ersatzschriftsuche geht
//! ohnehin durch alle installierten Familien. Schriftdateien und Schnitte
//! haben ihren eigenen Cache; dessen Größe wird hier nicht begrenzt.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use super::{FaceId, FontQuery, Slope};

#[cfg(test)]
mod tests;

const MAX_ENTRIES: usize = 256;
const MAX_CHARACTERS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct FallbackKey {
    weight: u32,
    width: u32,
    slope: (u8, u32),
    characters: Vec<char>,
}

impl FallbackKey {
    pub(super) fn new(query: &FontQuery<'_>, characters: &[char]) -> Option<Self> {
        if characters.is_empty() || characters.len() > MAX_CHARACTERS {
            return None;
        }
        Some(Self {
            weight: query.weight.to_bits(),
            width: query.width.to_bits(),
            slope: match query.slope {
                Slope::Normal => (0, 0),
                Slope::Italic => (1, 0),
                Slope::Oblique(angle) => (2, angle.to_bits()),
            },
            characters: characters.to_vec(),
        })
    }
}

pub(super) struct FallbackCache {
    entries: BTreeMap<Arc<FallbackKey>, Option<FaceId>>,
    order: VecDeque<Arc<FallbackKey>>,
    capacity: usize,
}

impl Default for FallbackCache {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            capacity: MAX_ENTRIES,
        }
    }
}

impl FallbackCache {
    #[cfg(test)]
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            ..Self::default()
        }
    }

    /// Äußeres `None`: nicht gespeichert; `Some(None)`: keine Ersatzschrift.
    pub(super) fn get(&mut self, key: &FallbackKey) -> Option<Option<FaceId>> {
        let (stored, result) = self.entries.get_key_value(key)?;
        let result = *result;
        if self
            .order
            .back()
            .is_some_and(|last| Arc::ptr_eq(last, stored))
        {
            return Some(result);
        }
        let stored = Arc::clone(stored);
        let position = self
            .order
            .iter()
            .position(|item| Arc::ptr_eq(item, &stored))
            .expect("jeder Suchcache-Eintrag hat einen LRU-Platz");
        self.order.remove(position);
        self.order.push_back(stored);
        Some(result)
    }

    pub(super) fn insert(&mut self, key: FallbackKey, result: Option<FaceId>) {
        if let Some(previous) = self.entries.get_mut(&key) {
            *previous = result;
            let _ = self.get(&key);
            return;
        }
        if self.capacity == 0 {
            return;
        }
        if self.entries.len() == self.capacity {
            let oldest = self
                .order
                .pop_front()
                .expect("zu verdrängender Suchcache-Eintrag");
            self.entries.remove(oldest.as_ref());
        }
        let key = Arc::new(key);
        self.entries.insert(Arc::clone(&key), result);
        self.order.push_back(key);
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    pub(super) fn remove(&mut self, key: &FallbackKey) {
        if let Some((stored, _)) = self.entries.remove_entry(key) {
            let position = self
                .order
                .iter()
                .position(|item| Arc::ptr_eq(item, &stored))
                .expect("Suchcache-Eintrag im LRU");
            self.order.remove(position);
        }
    }
}
