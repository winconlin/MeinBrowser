//! Dateipuffer zählen und an abgeschlossenen Darstellungsgrenzen bereinigen.
//!
//! Ein TTC-/OTC-Puffer zählt einmal, auch wenn mehrere Schnitte ihn teilen.
//! Gezählt werden die kodierten Dateibytes, keine Parserstrukturen,
//! Allokatorverwaltung oder Prozess-RSS. Externe starke oder schwache
//! Face-Verweise und eigenständige Face-Klone schützen die ganze Datei.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{Face, FaceEntry, FaceId, FallbackCache, FontData};

/// Vorgabe für ungenutzte Schriftdateipuffer: 64 MiB.
pub const DEFAULT_FONT_CACHE_BYTES: u64 = 64 << 20;

/// Momentaufnahme der vom Cache gehaltenen kodierten Schriftdateien.
/// Externe Referenzen können sich währenddessen ändern. Aktive Daten sind
/// vom Budget ausgenommen; dies ist keine Obergrenze für den Prozess-RAM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontCacheStats {
    /// Budget für ungenutzte Puffer bei der nächsten Bereinigung.
    pub budget_bytes: u64,
    /// Vom Cache gehaltene Dateipuffer, einschließlich aktiver Dateien.
    pub resident_bytes: u64,
    /// Durch externe Face-Referenzen oder -Klone gebundene Dateibytes.
    /// Schwache Face-Handles zählen vorsorglich ebenfalls als aktiv.
    pub active_bytes: u64,
    /// Der aktuell nicht gebundene Anteil von `resident_bytes`.
    pub unused_bytes: u64,
    /// Gehaltene Dateipuffer; fehlgeschlagene Abrufe zählen nicht.
    pub resident_files: usize,
    /// Gehaltene, erfolgreich geparste Schnitte.
    pub resident_faces: usize,
    /// Nachladeversuche, einschließlich Fehlern; ohne die Indexaufnahme.
    pub loads: u64,
    /// Verdrängte Dateipuffer, einschließlich später erneut geladener Dateien.
    pub evictions: u64,
}

pub(super) struct LoadedFile {
    pub(super) data: FontData,
    used: u64,
}

pub(super) struct Cache {
    pub(super) files: BTreeMap<usize, Option<LoadedFile>>,
    pub(super) faces: BTreeMap<FaceId, Option<Arc<Face>>>,
    pub(super) fallback: FallbackCache,
    budget: u64,
    loads: u64,
    evictions: u64,
    clock: u64,
}

impl Cache {
    pub(super) fn new(budget: u64) -> Self {
        Self {
            files: BTreeMap::new(),
            faces: BTreeMap::new(),
            fallback: FallbackCache::default(),
            budget,
            loads: 0,
            evictions: 0,
            clock: 0,
        }
    }

    fn next_stamp(&mut self) -> u64 {
        if self.clock == u64::MAX {
            let mut order: Vec<_> = self
                .files
                .iter()
                .filter_map(|(&id, file)| file.as_ref().map(|file| (file.used, id)))
                .collect();
            order.sort_unstable();
            for (rank, &(_, id)) in order.iter().enumerate() {
                self.files
                    .get_mut(&id)
                    .and_then(Option::as_mut)
                    .expect("Datei")
                    .used = rank as u64;
            }
            self.clock = order.len() as u64;
        }
        self.clock += 1;
        self.clock
    }

    pub(super) fn touch(&mut self, id: usize) {
        let stamp = self.next_stamp();
        if let Some(Some(file)) = self.files.get_mut(&id) {
            file.used = stamp;
        }
    }

    pub(super) fn record_load(&mut self, id: usize, data: Option<FontData>) {
        self.loads = self.loads.saturating_add(1);
        let used = self.next_stamp();
        self.files
            .insert(id, data.map(|data| LoadedFile { data, used }));
    }

    fn groups(&self, entries: &[FaceEntry]) -> BTreeMap<usize, Vec<FaceId>> {
        let mut groups: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
        for (&id, face) in &self.faces {
            if face.is_some() {
                let entry = entries.get(id.0).expect("Schnitt im Index");
                groups.entry(entry.file).or_default().push(id);
            }
        }
        groups
    }

    fn active(&self, file: &LoadedFile, faces: &[FaceId]) -> bool {
        faces.iter().any(|id| {
            self.faces
                .get(id)
                .and_then(Option::as_ref)
                .is_some_and(|face| Arc::strong_count(face) > 1 || Arc::weak_count(face) > 0)
        }) || Arc::strong_count(&file.data) > faces.len().saturating_add(1)
    }

    pub(super) fn stats(&self, entries: &[FaceEntry]) -> FontCacheStats {
        let groups = self.groups(entries);
        let mut stats = FontCacheStats {
            budget_bytes: self.budget,
            resident_faces: self.faces.values().filter(|face| face.is_some()).count(),
            loads: self.loads,
            evictions: self.evictions,
            ..FontCacheStats::default()
        };
        for (&id, file) in &self.files {
            let Some(file) = file else { continue };
            let bytes = u64::try_from(file.data.len()).unwrap_or(u64::MAX);
            stats.resident_files += 1;
            stats.resident_bytes = stats.resident_bytes.saturating_add(bytes);
            let faces = groups.get(&id).map_or(&[][..], Vec::as_slice);
            if self.active(file, faces) {
                stats.active_bytes = stats.active_bytes.saturating_add(bytes);
            } else {
                stats.unused_bytes = stats.unused_bytes.saturating_add(bytes);
            }
        }
        stats
    }

    /// Für die Verdrängung genügen die getrennten Zählerabfragen nicht:
    /// get_mut prüft starke UND schwache Face-Verweise atomar. Ist jeder
    /// Schnitt eindeutig besessen, kann während des gehaltenen Cache-Locks
    /// kein externer Besitzer einen neuen Face-/Datenverweis gewinnen.
    fn removable(&mut self, id: usize, faces: &[FaceId]) -> bool {
        for face in faces {
            let Some(Some(face)) = self.faces.get_mut(face) else {
                return false;
            };
            if Arc::get_mut(face).is_none() {
                return false;
            }
        }
        self.files
            .get(&id)
            .and_then(Option::as_ref)
            .is_some_and(|file| Arc::strong_count(&file.data) == faces.len().saturating_add(1))
    }

    pub(super) fn trim(&mut self, entries: &[FaceEntry]) -> FontCacheStats {
        let before = self.stats(entries);
        if before.unused_bytes <= self.budget {
            return before;
        }
        let groups = self.groups(entries);
        let mut candidates: Vec<_> = self
            .files
            .iter()
            .filter_map(|(&id, file)| {
                let file = file.as_ref()?;
                let faces = groups.get(&id).map_or(&[][..], Vec::as_slice);
                (!self.active(file, faces)).then_some((
                    file.used,
                    id,
                    u64::try_from(file.data.len()).unwrap_or(u64::MAX),
                ))
            })
            .collect();
        candidates.sort_unstable();
        let mut unused = before.unused_bytes;
        for (_, id, bytes) in candidates {
            if unused <= self.budget {
                break;
            }
            let faces = groups.get(&id).map_or(&[][..], Vec::as_slice);
            if self.removable(id, faces) {
                for face in faces {
                    self.faces.remove(face);
                }
                self.files.remove(&id);
                self.evictions = self.evictions.saturating_add(1);
            }
            // Zwischenzeitlich gepinnte Dateien zählen nicht mehr zum
            // bereinigbaren Anteil. Spätere Drops brauchen einen neuen Sweep.
            unused = unused.saturating_sub(bytes);
        }
        self.stats(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_wrap_keeps_the_least_recently_used_order() {
        let mut cache = Cache::new(2);
        for id in 0..3 {
            cache.record_load(id, Some(FontData::from(vec![0_u8])));
        }
        cache.touch(0);
        cache.clock = u64::MAX;
        cache.touch(2);
        let stats = cache.trim(&[]);
        assert_eq!(stats.resident_bytes, 2);
        assert_eq!(stats.evictions, 1);
        assert!(!cache.files.contains_key(&1), "älteste Datei verdrängt");
        assert!(cache.files.contains_key(&0));
        assert!(cache.files.contains_key(&2));
    }
}
