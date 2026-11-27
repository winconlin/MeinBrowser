//! Ein Zwischenspeicher auf der Platte: Einträge aus Bytes unter einem
//! Schlüssel aus 32 Bytes, je eine Datei in einem eigenen Ordner, mit einer
//! Grenze für die Bytes aller Einträge. Wird es mehr, fällt der Eintrag
//! heraus, der am längsten nicht gebraucht wurde.
//!
//! Was ein Eintrag bedeutet, weiß der Speicher nicht. Er darf Einträge
//! vergessen, und was er liefert, kann beschädigt oder untergeschoben sein,
//! denn jeder, der den Ordner beschreiben kann, kann die Dateien ändern.
//! Wer liest, prüft also jeden Eintrag, so wie `mb-js` seinen Bytecode.
//!
//! Die Datei eines Eintrags heißt wie sein Schlüssel in 64 kleinen
//! Hexadezimalziffern. Andere Dateien im Ordner zählen nicht und bleiben
//! liegen, nur Reste halb geschriebener Einträge räumt [`DiskCache::open`]
//! weg. Den Dateizugriff übernimmt `mb-platform`
//! ([`mb_platform::files`]): Es liest nur normale Dateien bis zur Grenze,
//! ersetzt Dateien als Ganzes und folgt beim Auflisten keinen Verweisen.
//!
//! Welche Einträge zuletzt gebraucht wurden, merkt sich der Speicher im
//! Arbeitsspeicher und auf der Platte in der Zeit der letzten Änderung
//! jeder Datei; [`DiskCache::open`] liest die Reihenfolge von dort. Den
//! Ordner sollte nur ein Prozess zugleich verwalten. Teilen sich zwei
//! einen, rechnet jeder nur mit seinen eigenen Einträgen und denen, die er
//! beim Öffnen fand.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use mb_platform::files;

/// Der Schlüssel eines Eintrags.
pub type Key = [u8; 32];

/// Ein Zwischenspeicher in einem Ordner auf der Platte. Mehrere Fäden
/// dürfen ihn sich teilen.
#[derive(Debug)]
pub struct DiskCache {
    directory: PathBuf,
    limit: u64,
    entries: Mutex<Entries>,
}

#[derive(Debug, Default)]
struct Entries {
    /// Nach dem Schlüssel: wann der Eintrag zuletzt gebraucht wurde und
    /// wie viele Bytes er hat.
    by_key: BTreeMap<Key, (u64, u64)>,
    /// Die Schlüssel nach dem Zeitpunkt, also der älteste zuerst.
    by_use: BTreeMap<u64, Key>,
    next: u64,
    bytes: u64,
}

impl Entries {
    fn touch(&mut self) -> u64 {
        let now = self.next;
        self.next = self.next.saturating_add(1);
        now
    }

    /// Nimmt `key` mit `len` Bytes als zuletzt gebraucht auf.
    fn insert(&mut self, key: Key, len: u64) {
        self.remove(&key);
        let now = self.touch();
        self.by_key.insert(key, (now, len));
        self.by_use.insert(now, key);
        self.bytes = self.bytes.saturating_add(len);
    }

    fn remove(&mut self, key: &Key) -> bool {
        let Some((used, len)) = self.by_key.remove(key) else {
            return false;
        };
        self.by_use.remove(&used);
        self.bytes = self.bytes.saturating_sub(len);
        true
    }

    /// Der Eintrag, der am längsten nicht gebraucht wurde.
    fn oldest(&self) -> Option<Key> {
        self.by_use.first_key_value().map(|(_, key)| *key)
    }
}

/// Der Dateiname zum Schlüssel.
fn file_name(key: &Key) -> OsString {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(64);
    for byte in key {
        for digit in [byte >> 4, byte & 0xF] {
            name.push(char::from(DIGITS[usize::from(digit)]));
        }
    }
    OsString::from(name)
}

/// Der Schlüssel zum Dateinamen, wenn er einer ist.
fn key_of(name: &OsStr) -> Option<Key> {
    let name = name.to_str()?.as_bytes();
    if name.len() != 64 {
        return None;
    }
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let mut key = [0; 32];
    for (slot, pair) in key.iter_mut().zip(name.chunks_exact(2)) {
        let [high, low] = pair else {
            return None;
        };
        *slot = digit(*high)? << 4 | digit(*low)?;
    }
    Some(key)
}

/// Ob die Datei der Rest eines Eintrags ist, den `mb_platform` beim
/// Ersetzen noch nicht an seinen Platz gesetzt hatte.
fn is_leftover(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.strip_prefix('.')
            .and_then(|rest| rest.strip_suffix(files::TEMPORARY_SUFFIX))
            .and_then(|rest| rest.get(..64))
            .is_some_and(|key| key_of(OsStr::new(key)).is_some())
    })
}

impl DiskCache {
    /// Öffnet den Speicher im Ordner `directory` und legt ihn an, wenn es
    /// ihn noch nicht gibt. Er hält höchstens `limit` Bytes; hält der
    /// Ordner mehr, fallen die ältesten Einträge gleich heraus.
    pub fn open(directory: &Path, limit: u64) -> io::Result<DiskCache> {
        files::create_directory(directory)?;
        let mut found = Vec::new();
        for file in files::regular_files(directory)? {
            if let Some(key) = key_of(&file.name) {
                found.push((file.modified, key, file.len));
            } else if is_leftover(&file.name) {
                // Kann auch einem anderen Prozess gehören, der gerade
                // schreibt; dann scheitert nur dessen Eintrag.
                let _ = files::remove(directory, &file.name);
            }
        }
        // Der älteste zuerst, bei gleicher Zeit nach dem Schlüssel (RS-11).
        found.sort_unstable();
        let mut entries = Entries::default();
        for (_, key, len) in found {
            entries.insert(key, len);
        }
        let cache = DiskCache {
            directory: directory.to_path_buf(),
            limit,
            entries: Mutex::new(entries),
        };
        {
            let mut entries = cache.lock();
            cache.shrink(&mut entries, 0);
        }
        Ok(cache)
    }

    /// Der Ordner des Speichers.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Die Grenze in Bytes.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Wie viele Einträge er gerade kennt.
    pub fn len(&self) -> usize {
        self.lock().by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Wie viele Bytes seine Einträge zusammen haben. Auf der Platte
    /// belegen sie etwas mehr, je nach Dateisystem.
    pub fn bytes(&self) -> u64 {
        self.lock().bytes
    }

    fn lock(&self) -> MutexGuard<'_, Entries> {
        // Ein Faden, der abbrach, hinterlässt die Tabellen stimmig: Jede
        // Änderung ist vollständig, bevor etwas scheitern kann. Die Dateien
        // stimmen dann vielleicht nicht mehr mit ihnen überein, aber das
        // darf ein Zwischenspeicher.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn path(&self, key: &Key) -> PathBuf {
        self.directory.join(file_name(key))
    }

    /// Entfernt die ältesten Einträge, bis noch `room` Bytes frei sind.
    fn shrink(&self, entries: &mut Entries, room: u64) {
        while entries.bytes.saturating_add(room) > self.limit {
            let Some(oldest) = entries.oldest() else {
                break;
            };
            entries.remove(&oldest);
            let _ = files::remove(&self.directory, &file_name(&oldest));
        }
    }

    /// Der Eintrag zum Schlüssel, wenn es einen gibt.
    pub fn load(&self, key: &Key) -> Option<Vec<u8>> {
        if !self.lock().by_key.contains_key(key) {
            return None;
        }
        // Gelesen wird ohne Sperre; ersetzt ein anderer Faden den Eintrag
        // gerade, liefert das den alten oder den neuen ganz.
        let path = self.path(key);
        match files::read_regular(&path, self.limit) {
            Ok(bytes) => {
                let mut entries = self.lock();
                if let Some(&(_, len)) = entries.by_key.get(key) {
                    entries.insert(*key, len);
                }
                drop(entries);
                // Damit die Reihenfolge einen Neustart übersteht. Gelingt
                // es nicht, gilt nach dem Neustart die alte Zeit.
                let _ = files::set_modified(&path, SystemTime::now());
                Some(bytes)
            }
            Err(_) => {
                self.lock().remove(key);
                None
            }
        }
    }

    /// Legt einen Eintrag ab und ersetzt einen alten zum selben Schlüssel.
    /// Hat er mehr Bytes als die Grenze oder scheitert das Schreiben, fehlt
    /// danach auch der alte.
    pub fn store(&self, key: &Key, bytes: &[u8]) {
        let mut entries = self.lock();
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if len > self.limit {
            if entries.remove(key) {
                let _ = files::remove(&self.directory, &file_name(key));
            }
            return;
        }
        // Unter der Sperre, damit Tabelle und Dateien zusammenpassen.
        entries.remove(key);
        self.shrink(&mut entries, len);
        match files::replace(&self.directory, &file_name(key), bytes) {
            Ok(()) => entries.insert(*key, len),
            Err(_) => {
                let _ = files::remove(&self.directory, &file_name(key));
            }
        }
    }
}

#[cfg(test)]
mod tests;
