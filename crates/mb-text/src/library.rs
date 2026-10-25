//! Die installierten Schriften und die Schriftauswahl nach CSS Fonts 4,
//! Abschnitt „Matching font styles“, für Schriften ohne `@font-face`.
//!
//! Die Bibliothek merkt sich von jedem Schnitt nur Datei, Nummer, Familien
//! und Einordnung; die Daten lädt sie erst, wenn ein Schnitt gebraucht wird,
//! und behält sie dann. Familien sind alle Namen mit der Nummer 16
//! (typografische Familie) und 1 (Familie) in jeder Sprache, verglichen nach
//! Default Caseless Matching (Abschnitt „Localized name matching“). So
//! findet `Lato` alle Schnitte der Familie und `Lato Medium` den einen, der
//! diesen Namen für alte Programme trägt.
//!
//! Noch nicht: Schriftaliase wie bei fontconfig (`Arial` für `Liberation
//! Sans`), künstlicher Fettdruck und künstliche Schrägstellung, eine
//! Übersicht der Zeichen jedes Schnitts im Index (die Ersatzschriftsuche lädt
//! dafür Schnitte).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use mb_unicode::case_folding;

use crate::matching::{self, Candidate, Slope};
use crate::opentype::{Face, FontData, face_count};

/// Liest den Inhalt einer Schriftdatei; `None`, wenn das nicht geht.
pub type Loader = Box<dyn Fn(&Path) -> Option<Vec<u8>> + Send + Sync>;

/// Ein Schnitt in der Bibliothek.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceId(usize);

/// Eine generische Familie aus `font-family`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenericFamily {
    Serif,
    SansSerif,
    Monospace,
    Cursive,
    Fantasy,
    SystemUi,
    Math,
    UiSerif,
    UiSansSerif,
    UiMonospace,
    UiRounded,
    Fangsong,
    Kai,
    KhmerMul,
    Nastaliq,
}

impl GenericFamily {
    /// Die Familien, für die der Name steht, in der Reihenfolge, in der sie
    /// gesucht werden: für `serif`, `sans-serif`, `monospace`, `fantasy`,
    /// `cursive` und `system-ui` die Listen aus `60-latin.conf` von
    /// fontconfig 2.15.0. Für die übrigen gibt es noch keine; sie passen auf
    /// keine Familie.
    pub fn families(self) -> &'static [&'static str] {
        match self {
            GenericFamily::Serif => &[
                "Noto Serif",
                "DejaVu Serif",
                "Times New Roman",
                "Thorndale AMT",
                "Luxi Serif",
                "Nimbus Roman No9 L",
                "Nimbus Roman",
                "Times",
            ],
            GenericFamily::SansSerif => &[
                "Noto Sans",
                "DejaVu Sans",
                "Verdana",
                "Arial",
                "Albany AMT",
                "Luxi Sans",
                "Nimbus Sans L",
                "Nimbus Sans",
                "Helvetica",
                "Lucida Sans Unicode",
                "BPG Glaho International",
                "Tahoma",
            ],
            GenericFamily::Monospace => &[
                "DejaVu Sans Mono",
                "Noto Mono",
                "Noto Sans Mono",
                "Inconsolata",
                "Andale Mono",
                "Courier New",
                "Cumberland AMT",
                "Luxi Mono",
                "Nimbus Mono L",
                "Nimbus Mono",
                "Nimbus Mono PS",
                "Courier",
            ],
            GenericFamily::Fantasy => &[
                "Impact",
                "Copperplate Gothic Std",
                "Cooper Std",
                "Bauhaus Std",
            ],
            GenericFamily::Cursive => &["ITC Zapf Chancery Std", "Zapfino", "Comic Sans MS"],
            GenericFamily::SystemUi => &[
                "Cantarell",
                "Noto Sans UI",
                "Segoe UI",
                "Segoe UI Historic",
                "Segoe UI Symbol",
            ],
            GenericFamily::Math
            | GenericFamily::UiSerif
            | GenericFamily::UiSansSerif
            | GenericFamily::UiMonospace
            | GenericFamily::UiRounded
            | GenericFamily::Fangsong
            | GenericFamily::Kai
            | GenericFamily::KhmerMul
            | GenericFamily::Nastaliq => &[],
        }
    }
}

/// Ein Eintrag aus `font-family`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Name(String),
    Generic(GenericFamily),
}

/// Was für ein Element gesucht wird: die berechneten Werte von
/// `font-family`, `font-weight`, `font-width` (in Prozent) und `font-style`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontQuery<'a> {
    pub families: &'a [Family],
    pub weight: f32,
    pub width: f32,
    pub slope: Slope,
}

struct FaceEntry {
    file: usize,
    index: u32,
    candidate: Candidate,
}

/// Was schon geladen ist: Dateien und Schnitte, `None`, wo das Laden
/// scheiterte.
#[derive(Default)]
struct Cache {
    files: BTreeMap<usize, Option<FontData>>,
    faces: BTreeMap<FaceId, Option<Arc<Face>>>,
}

/// Die installierten Schriften.
pub struct FontLibrary {
    loader: Loader,
    files: Vec<PathBuf>,
    faces: Vec<FaceEntry>,
    /// Die gefalteten Familiennamen mit ihren Schnitten in der Reihenfolge,
    /// in der sie dazukamen.
    families: BTreeMap<String, Vec<FaceId>>,
    cache: Mutex<Cache>,
}

impl fmt::Debug for FontLibrary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FontLibrary")
            .field("files", &self.files)
            .field("faces", &self.faces.len())
            .finish_non_exhaustive()
    }
}

impl FontLibrary {
    /// Eine leere Bibliothek, die Dateien mit `loader` liest.
    pub fn new(loader: Loader) -> Self {
        FontLibrary {
            loader,
            files: Vec::new(),
            faces: Vec::new(),
            families: BTreeMap::new(),
            cache: Mutex::new(Cache::default()),
        }
    }

    /// Nimmt die Schnitte einer Datei auf und gibt zurück, wie viele es
    /// waren. Dateien und Schnitte, die sich nicht lesen lassen, fallen weg.
    pub fn add_file(&mut self, path: PathBuf) -> usize {
        let Some(bytes) = (self.loader)(&path) else {
            return 0;
        };
        let data = FontData::from(bytes);
        let Ok(count) = face_count(&data) else {
            return 0;
        };
        // Jeder Schnitt einer Sammlung braucht vier Bytes im Kopf; mehr
        // angebliche Schnitte gibt es nicht.
        let possible = u32::try_from(data.len().saturating_sub(12) / 4).unwrap_or(u32::MAX);
        let file = self.files.len();
        let mut added = 0;
        for index in 0..count.min(possible.max(1)) {
            let Ok(face) = Face::parse(data.clone(), index) else {
                continue;
            };
            let id = FaceId(self.faces.len());
            self.faces.push(FaceEntry {
                file,
                index,
                candidate: Candidate::from_style(face.style()),
            });
            let names: BTreeSet<String> = face
                .names()
                .iter()
                .filter(|name| matches!(name.id, 1 | 16) && !name.value.is_empty())
                .map(|name| case_folding::fold(&name.value))
                .collect();
            for name in names {
                self.families.entry(name).or_default().push(id);
            }
            added += 1;
        }
        if added > 0 {
            self.files.push(path);
        }
        added
    }

    /// Wie viele Schnitte die Bibliothek kennt.
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Die Schnitte einer Familie.
    pub fn family(&self, name: &str) -> &[FaceId] {
        self.families
            .get(&case_folding::fold(name))
            .map_or(&[], Vec::as_slice)
    }

    /// Schritt 4 für eine Familie: der passende Schnitt.
    fn best_in(&self, family: &[FaceId], query: &FontQuery<'_>) -> Option<FaceId> {
        let candidates: Vec<Candidate> = family
            .iter()
            .filter_map(|id| self.faces.get(id.0))
            .map(|entry| entry.candidate)
            .collect();
        let index = matching::best(&candidates, query.width, query.slope, query.weight)?;
        family.get(index).copied()
    }

    /// Die erste Familie aus der Liste, die es gibt.
    fn first_family(&self, names: &[&str]) -> Option<&[FaceId]> {
        names
            .iter()
            .map(|name| self.family(name))
            .find(|family| !family.is_empty())
    }

    /// Die Schriftliste für eine Anfrage (Schritte 1 bis 4): für jede Familie
    /// aus `font-family`, die es gibt, der passende Schnitt, in der
    /// Reihenfolge der Liste und ohne Wiederholungen. Eine generische
    /// Familie steht für die erste ihrer Familien, die es gibt.
    pub fn select(&self, query: &FontQuery<'_>) -> Vec<FaceId> {
        let mut selected = Vec::new();
        for family in query.families {
            let faces = match family {
                Family::Name(name) => Some(self.family(name)),
                Family::Generic(generic) => self.first_family(generic.families()),
            };
            if let Some(id) = faces.and_then(|faces| self.best_in(faces, query))
                && !selected.contains(&id)
            {
                selected.push(id);
            }
        }
        selected
    }

    /// Die Schrift des Browsers, wenn keine Familie der Anfrage passt: der
    /// passende Schnitt der ersten Familie von `serif`, `sans-serif` und
    /// `monospace`, die es gibt, sonst der ersten Familie überhaupt.
    pub fn default_face(&self, query: &FontQuery<'_>) -> Option<FaceId> {
        let family = [
            GenericFamily::Serif,
            GenericFamily::SansSerif,
            GenericFamily::Monospace,
        ]
        .into_iter()
        .find_map(|generic| self.first_family(generic.families()))
        .or_else(|| self.families.values().next().map(Vec::as_slice))?;
        self.best_in(family, query)
    }

    /// Die Ersatzschrift für Zeichen, die keine Schrift der Liste hat
    /// (Schritt 7, *installed font fallback*): der passende Schnitt der
    /// ersten Familie nach gefaltetem Namen, der für jedes der Zeichen eine
    /// Glyphe hat. Ohne Zeichen `None`.
    pub fn fallback(&self, query: &FontQuery<'_>, characters: &[char]) -> Option<FaceId> {
        if characters.is_empty() {
            return None;
        }
        self.families.values().find_map(|family| {
            let id = self.best_in(family, query)?;
            let face = self.face(id)?;
            characters
                .iter()
                .all(|&character| face.glyph_index(character).is_some())
                .then_some(id)
        })
    }

    /// Der Schnitt mit seinen Daten, beim ersten Mal geladen.
    pub fn face(&self, id: FaceId) -> Option<Arc<Face>> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(face) = cache.faces.get(&id) {
            return face.clone();
        }
        let entry = self.faces.get(id.0)?;
        let data = match cache.files.get(&entry.file) {
            Some(data) => data.clone(),
            None => {
                let data = self
                    .files
                    .get(entry.file)
                    .and_then(|path| (self.loader)(path))
                    .map(FontData::from);
                cache.files.insert(entry.file, data.clone());
                data
            }
        };
        let face = data
            .and_then(|data| Face::parse(data, entry.index).ok())
            .map(Arc::new);
        cache.faces.insert(id, face.clone());
        face
    }
}

#[cfg(test)]
mod tests;
