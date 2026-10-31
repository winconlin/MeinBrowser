//! Der Heap eines Agents (`docs/architektur.md`, Abschnitte 2.4 und 4.9).
//!
//! Zellen liegen in Seiten zu 256 KiB, Zellen über 256 KiB in eigenen
//! Seiten. Ein Verweis ist ein 32-Bit-Abstand: oben 14 Bit für die Nummer der
//! Seite, unten 18 Bit für die Lage der Zelle in Bytes. Weil Verweise keine
//! Zeiger sind, kommt der Heap ohne `unsafe` aus, gelesen wird mit `get()`
//! (RS-7). Jede Zelle beginnt mit einem Kopfwort: Art in den Bits 0 bis 7,
//! Länge in den Bits 32 bis 63.
//!
//! Gesammelt wird in der ersten Ausbaustufe präzise und kopierend nach
//! Cheney. Alle Seiten werden zum alten Raum, die erreichbaren Zellen wandern
//! in neue Seiten, und ein Lesezeiger über die neuen Zellen führt deren
//! Verweise nach. Das kommt ohne Rekursion und ohne Markierstapel aus (RS-8).
//! Generationen folgen später. Darum läuft jedes Speichern eines Verweises in
//! eine Zelle schon jetzt über eine einzige Funktion, an der dann die
//! Schreibbarriere ansetzt.
//!
//! Wurzeln sind die Handles: [`Local`] in einem Bereich ([`Heap::scope`]) und
//! [`Persistent`] für langlebige Verweise. Gelesene Werte ([`Value`]) leihen
//! sich den Heap, jede Anforderung braucht `&mut Heap`. So verhindert der
//! Borrow-Checker, dass Rust-Code einen Verweis über eine Sammlung hinweg
//! hält:
//!
//! ```compile_fail,E0502
//! use mb_js::heap::Heap;
//!
//! let mut heap = Heap::new();
//! heap.scope(|heap, scope| {
//!     let a = heap.alloc_string(scope, &[0x61]).expect("Speicher");
//!     let value = heap.get(a);
//!     // Fehler: `value` leiht sich den Heap, die Anforderung braucht ihn
//!     // veränderlich.
//!     let _ = heap.alloc_string(scope, &[0x62]);
//!     println!("{value:?}");
//! });
//! ```
//!
//! Richtig ist, den Wert erst nach der Anforderung aus seinem Handle zu
//! lesen:
//!
//! ```
//! use mb_js::heap::Heap;
//! use mb_js::value::Kind;
//!
//! let mut heap = Heap::new();
//! heap.scope(|heap, scope| {
//!     let a = heap.alloc_string(scope, &[0x61]).expect("Speicher");
//!     let _ = heap.alloc_string(scope, &[0x62]).expect("Speicher");
//!     heap.collect();
//!     let Kind::String(a) = heap.get(a).kind() else { panic!("Zeichenkette") };
//!     assert_eq!(heap.string_units(a), [0x61]);
//! });
//! ```

#![forbid(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use crate::value::{Ref, RefKind, Value};

/// Wörter einer Seite: 256 KiB.
const PAGE_WORDS: usize = 1 << 15;
/// Die Lage in der Seite steht in den unteren 18 Bit eines Abstands.
const PAGE_SHIFT: u32 = 18;
const IN_PAGE_MASK: u32 = (1 << PAGE_SHIFT) - 1;
/// So viele Seiten lassen sich mit 14 Bit nummerieren.
const MAX_PAGES: usize = 1 << 14;

/// Das größte Speicherbudget. Die Abstände reichen für 4 GiB. Beim Sammeln
/// liegen alter und neuer Raum gleichzeitig vor, und Zellen in eigenen
/// Seiten lassen Seiten angebrochen; mit einem Viertel bleibt genug Platz.
pub const MAX_LIMIT: usize = 1 << 30;
/// Ab dieser Menge wird frühestens gesammelt.
const MIN_TRIGGER: usize = 4 << 20;

const KIND_FORWARD: u64 = 1;
const KIND_LATIN1: u64 = 2;
const KIND_UTF16: u64 = 3;
const KIND_SLOTS: u64 = 4;

fn header(kind: u64, length: u32) -> u64 {
    kind | (u64::from(length) << 32)
}

fn header_kind(header: u64) -> u64 {
    header & 0xFF
}

fn header_length(header: u64) -> u32 {
    // Nach der Verschiebung bleiben 32 Bit.
    u32::try_from(header >> 32).unwrap_or(0)
}

/// Wie viele Wörter eine Zelle mit diesem Kopfwort belegt.
fn cell_words(header: u64) -> Option<usize> {
    let length = to_usize(header_length(header));
    let body = match header_kind(header) {
        KIND_LATIN1 => length.div_ceil(8),
        KIND_UTF16 => length.div_ceil(4),
        KIND_SLOTS => length,
        _ => return None,
    };
    body.checked_add(1)
}

fn make_offset(page: usize, word: usize) -> Option<u32> {
    let page = u32::try_from(page)
        .ok()
        .filter(|&page| to_usize(page) < MAX_PAGES)?;
    let byte = u32::try_from(word.checked_mul(8)?)
        .ok()
        .filter(|&byte| byte <= IN_PAGE_MASK)?;
    Some((page << PAGE_SHIFT) | byte)
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Einstellungen eines Heaps.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Das Speicherbudget in Bytes, höchstens [`MAX_LIMIT`].
    pub limit: usize,
    /// Sammeln vor jeder Anforderung (Belastungsmodus).
    pub stress: bool,
    /// Nach jeder Sammlung alle Verweise und Kopfwörter prüfen
    /// (Prüfmodus, nur für Tests: ein Fehler löst eine Panic aus).
    pub verify: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            limit: MAX_LIMIT,
            stress: false,
            verify: false,
        }
    }
}

/// Das Speicherbudget ist auch nach einer Sammlung verbraucht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfMemory;

impl fmt::Display for OutOfMemory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("das Speicherbudget des Heaps ist verbraucht")
    }
}

impl std::error::Error for OutOfMemory {}

/// Ein Bereich für Handles, siehe [`Heap::scope`].
pub struct Scope {
    _private: (),
}

/// Ein Handle in einem Bereich. Der Garbage Collector führt es beim
/// Verschieben nach, und es gilt, bis sein Bereich endet.
#[derive(Clone, Copy, Debug)]
pub struct Local<'s> {
    index: usize,
    scope: PhantomData<&'s Scope>,
}

/// Ein Handle ohne Bereich, bis [`Heap::release`] es freigibt.
#[derive(Debug)]
pub struct Persistent {
    index: usize,
}

struct Page {
    words: Box<[Cell<u64>]>,
    /// Die belegten Wörter.
    used: usize,
    /// Gehört während einer Sammlung zum alten Raum.
    old: bool,
}

/// Der Heap eines Agents.
pub struct Heap {
    pages: Vec<Option<Page>>,
    free_pages: Vec<usize>,
    /// Seiten zu 256 KiB in der Reihenfolge, in der sie angelegt wurden; in
    /// die letzte wird angefordert.
    small: Vec<usize>,
    /// Seiten mit einer großen Zelle, ebenfalls in Reihenfolge.
    large: Vec<usize>,
    /// Handles aller offenen Bereiche.
    stack: RefCell<Vec<u64>>,
    persistent: Vec<u64>,
    free_persistent: Vec<usize>,
    /// Bytes in Zellen.
    used: usize,
    /// Ab hier wird vor der nächsten Anforderung gesammelt.
    trigger: usize,
    options: Options,
    collections: u64,
}

impl Default for Heap {
    fn default() -> Heap {
        Heap::new()
    }
}

impl Heap {
    pub fn new() -> Heap {
        Heap::with_options(Options::default())
    }

    pub fn with_options(options: Options) -> Heap {
        let options = Options {
            limit: options.limit.min(MAX_LIMIT),
            ..options
        };
        Heap {
            pages: Vec::new(),
            free_pages: Vec::new(),
            small: Vec::new(),
            large: Vec::new(),
            stack: RefCell::new(Vec::new()),
            persistent: Vec::new(),
            free_persistent: Vec::new(),
            used: 0,
            trigger: MIN_TRIGGER.min(options.limit),
            options,
            collections: 0,
        }
    }

    /// Bytes in Zellen, nach einer Sammlung nur noch erreichbare.
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// Wie oft bisher gesammelt wurde.
    pub fn collections(&self) -> u64 {
        self.collections
    }

    // Handles

    /// Führt `f` in einem neuen Bereich aus. Handles, die `f` darin anlegt,
    /// gelten bis zum Ende von `f`; die Typen verhindern, dass sie
    /// entkommen. Bereiche lassen sich schachteln.
    pub fn scope<R, F: FnOnce(&mut Heap, &Scope) -> R>(&mut self, f: F) -> R {
        let mark = self.stack.get_mut().len();
        let result = f(self, &Scope { _private: () });
        self.stack.get_mut().truncate(mark);
        result
    }

    /// Hält `value` in einem Handle des Bereichs `scope`.
    pub fn local<'s>(&self, _scope: &'s Scope, value: Value<'_>) -> Local<'s> {
        let mut stack = self.stack.borrow_mut();
        let index = stack.len();
        stack.push(value.bits());
        Local {
            index,
            scope: PhantomData,
        }
    }

    pub fn get(&self, local: Local<'_>) -> Value<'_> {
        let bits = self.stack.borrow().get(local.index).copied();
        Value::from_bits(bits.unwrap_or(Value::UNDEFINED.bits()))
    }

    pub fn set(&self, local: Local<'_>, value: Value<'_>) {
        if let Some(slot) = self.stack.borrow_mut().get_mut(local.index) {
            *slot = value.bits();
        }
    }

    /// Hält den Wert von `local` über alle Bereiche hinweg.
    pub fn persist(&mut self, local: Local<'_>) -> Persistent {
        let bits = self.get(local).bits();
        if let Some(index) = self.free_persistent.pop()
            && let Some(slot) = self.persistent.get_mut(index)
        {
            *slot = bits;
            return Persistent { index };
        }
        self.persistent.push(bits);
        Persistent {
            index: self.persistent.len() - 1,
        }
    }

    pub fn get_persistent(&self, handle: &Persistent) -> Value<'_> {
        let bits = self.persistent.get(handle.index).copied();
        Value::from_bits(bits.unwrap_or(Value::UNDEFINED.bits()))
    }

    pub fn release(&mut self, handle: Persistent) {
        if let Some(slot) = self.persistent.get_mut(handle.index) {
            *slot = Value::UNDEFINED.bits();
            self.free_persistent.push(handle.index);
        }
    }

    // Zeichenketten

    /// Legt eine flache Zeichenkette an, als Latin-1, wenn jede Codeeinheit
    /// unter 0x100 liegt, sonst als UTF-16. Einzelne Surrogate bleiben
    /// erhalten (RS-9).
    pub fn alloc_string<'s>(
        &mut self,
        scope: &'s Scope,
        units: &[u16],
    ) -> Result<Local<'s>, OutOfMemory> {
        let length = u32::try_from(units.len()).map_err(|_| OutOfMemory)?;
        let latin1 = units.iter().all(|&unit| unit < 0x100);
        let kind = if latin1 { KIND_LATIN1 } else { KIND_UTF16 };
        let head = header(kind, length);
        let words = cell_words(head).ok_or(OutOfMemory)?;
        let offset = self.allocate(words)?;
        self.init(offset, 0, head);
        let (per_word, bits) = if latin1 { (8, 8) } else { (4, 16) };
        for (index, chunk) in units.chunks(per_word).enumerate() {
            let word = chunk
                .iter()
                .enumerate()
                .fold(0u64, |word, (position, &unit)| {
                    word | (u64::from(unit) << (position * bits))
                });
            self.init(offset, index + 1, word);
        }
        Ok(self.local(scope, Value::reference(RefKind::String, offset)))
    }

    /// Die Länge in Codeeinheiten, 0 für einen Verweis auf etwas anderes.
    pub fn string_len(&self, string: Ref<'_>) -> usize {
        match self.string_header(string) {
            Some(head) => to_usize(header_length(head)),
            None => 0,
        }
    }

    /// Ob die Zeichenkette als Latin-1 gespeichert ist.
    pub fn is_latin1(&self, string: Ref<'_>) -> bool {
        self.string_header(string)
            .is_some_and(|head| header_kind(head) == KIND_LATIN1)
    }

    pub fn string_unit(&self, string: Ref<'_>, index: usize) -> Option<u16> {
        let head = self.string_header(string)?;
        if index >= to_usize(header_length(head)) {
            return None;
        }
        let (per_word, bits, mask) = if header_kind(head) == KIND_LATIN1 {
            (8, 8, 0xFF)
        } else {
            (4, 16, 0xFFFF)
        };
        let word = self.word(string.offset(), index / per_word + 1)?;
        u16::try_from((word >> ((index % per_word) * bits)) & mask).ok()
    }

    pub fn string_units(&self, string: Ref<'_>) -> Vec<u16> {
        (0..self.string_len(string))
            .map_while(|index| self.string_unit(string, index))
            .collect()
    }

    fn string_header(&self, string: Ref<'_>) -> Option<u64> {
        let head = self.word(string.offset(), 0)?;
        matches!(header_kind(head), KIND_LATIN1 | KIND_UTF16).then_some(head)
    }

    // Felder

    /// Legt eine interne Zelle mit `len` Feldern an, alle *undefined*. Aus
    /// solchen Zellen entstehen später Elemente, Kontexte und Speicher für
    /// Eigenschaften. Der Verweis darauf hat die Art
    /// [`Kind::Internal`](crate::value::Kind::Internal).
    pub fn alloc_slots<'s>(
        &mut self,
        scope: &'s Scope,
        len: usize,
    ) -> Result<Local<'s>, OutOfMemory> {
        let length = u32::try_from(len).map_err(|_| OutOfMemory)?;
        let head = header(KIND_SLOTS, length);
        let words = cell_words(head).ok_or(OutOfMemory)?;
        let offset = self.allocate(words)?;
        self.init(offset, 0, head);
        for index in 1..words {
            self.init(offset, index, Value::UNDEFINED.bits());
        }
        Ok(self.local(scope, Value::reference(RefKind::Internal, offset)))
    }

    /// Die Zahl der Felder, 0 für einen Verweis auf etwas anderes.
    pub fn slot_count(&self, cell: Ref<'_>) -> usize {
        match self.slots_header(cell) {
            Some(head) => to_usize(header_length(head)),
            None => 0,
        }
    }

    pub fn slot(&self, cell: Ref<'_>, index: usize) -> Option<Value<'_>> {
        let head = self.slots_header(cell)?;
        if index >= to_usize(header_length(head)) {
            return None;
        }
        self.word(cell.offset(), index.checked_add(1)?)
            .map(Value::from_bits)
    }

    /// Schreibt ein Feld. `false`, wenn es das Feld nicht gibt.
    pub fn set_slot(&self, cell: Ref<'_>, index: usize, value: Value<'_>) -> bool {
        let Some(head) = self.slots_header(cell) else {
            return false;
        };
        if index >= to_usize(header_length(head)) {
            return false;
        }
        index
            .checked_add(1)
            .is_some_and(|word| self.store(cell.offset(), word, value.bits()))
    }

    fn slots_header(&self, cell: Ref<'_>) -> Option<u64> {
        let head = self.word(cell.offset(), 0)?;
        (header_kind(head) == KIND_SLOTS).then_some(head)
    }

    // Wörter

    fn page(&self, offset: u32) -> Option<&Page> {
        self.pages.get(to_usize(offset >> PAGE_SHIFT))?.as_ref()
    }

    fn word(&self, offset: u32, index: usize) -> Option<u64> {
        let start = to_usize((offset & IN_PAGE_MASK) >> 3);
        let page = self.page(offset)?;
        page.words.get(start.checked_add(index)?).map(Cell::get)
    }

    fn write(&self, offset: u32, index: usize, bits: u64) -> bool {
        let start = to_usize((offset & IN_PAGE_MASK) >> 3);
        let Some(cell) = self
            .page(offset)
            .and_then(|page| page.words.get(start.checked_add(index)?))
        else {
            return false;
        };
        cell.set(bits);
        true
    }

    /// Füllt eine eben angelegte Zelle. Sie ist neu, eine Schreibbarriere
    /// braucht es dafür nicht.
    fn init(&self, offset: u32, index: usize, bits: u64) {
        self.write(offset, index, bits);
    }

    /// Speichert einen Wert in eine bestehende Zelle. Jedes solche Speichern
    /// läuft hierüber; mit den Generationen kommt hier die Schreibbarriere
    /// hinzu.
    fn store(&self, offset: u32, index: usize, bits: u64) -> bool {
        self.write(offset, index, bits)
    }

    // Anfordern

    fn allocate(&mut self, words: usize) -> Result<u32, OutOfMemory> {
        let bytes = words.checked_mul(8).ok_or(OutOfMemory)?;
        if self.options.stress || self.used.saturating_add(bytes) > self.trigger {
            self.collect();
        }
        if self.used.saturating_add(bytes) > self.options.limit {
            return Err(OutOfMemory);
        }
        let offset = self.bump(words).ok_or(OutOfMemory)?;
        self.used = self.used.saturating_add(bytes);
        Ok(offset)
    }

    /// Nimmt `words` Wörter aus der letzten Seite oder einer neuen.
    fn bump(&mut self, words: usize) -> Option<u32> {
        if words > PAGE_WORDS {
            let index = self.new_page(words)?;
            self.large.push(index);
            if let Some(Some(page)) = self.pages.get_mut(index) {
                page.used = words;
            }
            return make_offset(index, 0);
        }
        let last = self.small.last().copied();
        if let Some(offset) = last.and_then(|index| self.bump_in(index, words)) {
            return Some(offset);
        }
        let index = self.new_page(PAGE_WORDS)?;
        self.small.push(index);
        self.bump_in(index, words)
    }

    fn bump_in(&mut self, index: usize, words: usize) -> Option<u32> {
        let page = self.pages.get_mut(index)?.as_mut()?;
        let start = page.used;
        let end = start.checked_add(words)?;
        if end > page.words.len() {
            return None;
        }
        page.used = end;
        make_offset(index, start)
    }

    fn new_page(&mut self, words: usize) -> Option<usize> {
        let index = match self.free_pages.pop() {
            Some(index) => index,
            None if self.pages.len() < MAX_PAGES => {
                self.pages.push(None);
                self.pages.len() - 1
            }
            None => return None,
        };
        let slot = self.pages.get_mut(index)?;
        *slot = Some(Page {
            words: vec![Cell::new(0); words].into_boxed_slice(),
            used: 0,
            old: false,
        });
        Some(index)
    }

    // Sammeln

    /// Sammelt den ganzen Heap: Was von keinem Handle aus erreichbar ist,
    /// wird frei, alles andere verschoben.
    pub fn collect(&mut self) {
        for page in self.pages.iter_mut().flatten() {
            page.old = true;
        }
        let old_small = std::mem::take(&mut self.small);
        let old_large = std::mem::take(&mut self.large);
        self.used = 0;

        let mut stack = std::mem::take(self.stack.get_mut());
        for slot in &mut stack {
            *slot = self.evacuate(*slot);
        }
        *self.stack.get_mut() = stack;
        let mut persistent = std::mem::take(&mut self.persistent);
        for slot in &mut persistent {
            *slot = self.evacuate(*slot);
        }
        self.persistent = persistent;

        self.scan();

        for index in old_small.into_iter().chain(old_large) {
            if let Some(slot) = self.pages.get_mut(index) {
                *slot = None;
                self.free_pages.push(index);
            }
        }
        self.trigger = self
            .used
            .saturating_mul(2)
            .max(MIN_TRIGGER)
            .min(self.options.limit);
        self.collections = self.collections.saturating_add(1);
        if self.options.verify
            && let Err(error) = self.verify()
        {
            panic!("Prüfmodus des Heaps: {error}");
        }
    }

    /// Der Lesezeiger nach Cheney. Kleine Seiten werden der Reihe nach
    /// gelesen; neue Zellen landen immer in der letzten. Große Seiten haben
    /// je eine Zelle und eine eigene Liste.
    fn scan(&mut self) {
        let (mut small, mut word, mut large) = (0, 0, 0);
        loop {
            let mut progress = false;
            while let Some(&index) = self.small.get(small) {
                let used = self
                    .pages
                    .get(index)
                    .and_then(Option::as_ref)
                    .map_or(0, |page| page.used);
                if word < used {
                    word = self
                        .scan_cell(index, word)
                        .map_or(used, |words| word.saturating_add(words));
                    progress = true;
                } else if small + 1 < self.small.len() {
                    small += 1;
                    word = 0;
                } else {
                    break;
                }
            }
            while let Some(&index) = self.large.get(large) {
                self.scan_cell(index, 0);
                large += 1;
                progress = true;
            }
            if !progress {
                break;
            }
        }
    }

    /// Führt die Verweise der Zelle bei `word` in Seite `index` nach und
    /// gibt ihre Größe in Wörtern zurück.
    fn scan_cell(&mut self, index: usize, word: usize) -> Option<usize> {
        let offset = make_offset(index, word)?;
        let head = self.word(offset, 0)?;
        let words = cell_words(head)?;
        if header_kind(head) == KIND_SLOTS {
            for slot in 1..words {
                if let Some(bits) = self.word(offset, slot) {
                    let moved = self.evacuate(bits);
                    self.write(offset, slot, moved);
                }
            }
        }
        Some(words)
    }

    /// Kopiert die Zelle, auf die `bits` zeigt, in den neuen Raum, wenn das
    /// noch nicht geschehen ist, und gibt den nachgeführten Wert zurück.
    fn evacuate(&mut self, bits: u64) -> u64 {
        let Some((kind, offset)) = Value::from_bits(bits).reference_parts() else {
            return bits;
        };
        if !self.page(offset).is_some_and(|page| page.old) {
            return bits;
        }
        let Some(head) = self.word(offset, 0) else {
            return bits;
        };
        if header_kind(head) == KIND_FORWARD {
            return Value::reference(kind, header_length(head)).bits();
        }
        let Some(words) = cell_words(head) else {
            return bits;
        };
        // Der neue Raum findet immer Platz: Das Budget ist höchstens ein
        // Viertel dessen, was die Abstände fassen.
        let Some(moved) = self.bump(words) else {
            return bits;
        };
        for index in 0..words {
            if let Some(word) = self.word(offset, index) {
                self.write(moved, index, word);
            }
        }
        self.write(offset, 0, header(KIND_FORWARD, moved));
        self.used = self.used.saturating_add(words.saturating_mul(8));
        Value::reference(kind, moved).bits()
    }

    /// Prüft den Heap: Jede Seite besteht lückenlos aus gültigen Zellen, und
    /// jeder Verweis in Handles und Feldern zeigt auf den Anfang einer
    /// passenden Zelle.
    pub fn verify(&self) -> Result<(), String> {
        let mut starts = BTreeMap::new();
        for (index, page) in self.pages.iter().enumerate() {
            let Some(page) = page else {
                continue;
            };
            if page.old {
                return Err(format!("Seite {index} gehört noch zum alten Raum"));
            }
            let mut word = 0;
            while word < page.used {
                let offset = make_offset(index, word)
                    .ok_or_else(|| format!("Seite {index}: Zelle bei Wort {word} ohne Abstand"))?;
                let head = page.words.get(word).map_or(0, Cell::get);
                let words = cell_words(head)
                    .ok_or_else(|| format!("ungültiges Kopfwort {head:#x} bei {offset:#x}"))?;
                starts.insert(offset, header_kind(head));
                word = word.saturating_add(words);
            }
            if word != page.used {
                return Err(format!("Seite {index}: Zellen überschreiten die Belegung"));
            }
        }
        let check = |bits: u64, place: &str| -> Result<(), String> {
            let Some((kind, offset)) = Value::from_bits(bits).reference_parts() else {
                return Ok(());
            };
            match (kind, starts.get(&offset)) {
                (RefKind::String, Some(&(KIND_LATIN1 | KIND_UTF16)))
                | (RefKind::Internal, Some(&KIND_SLOTS)) => Ok(()),
                _ => Err(format!(
                    "{place}: Verweis {offset:#x} ({kind:?}) zeigt auf keine passende Zelle"
                )),
            }
        };
        for &bits in self.stack.borrow().iter() {
            check(bits, "Handle")?;
        }
        for &bits in &self.persistent {
            check(bits, "dauerhaftes Handle")?;
        }
        for (&offset, &kind) in &starts {
            if kind != KIND_SLOTS {
                continue;
            }
            let count = self
                .word(offset, 0)
                .map_or(0, |head| to_usize(header_length(head)));
            for slot in 1..=count {
                check(self.word(offset, slot).unwrap_or(0), "Feld")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
