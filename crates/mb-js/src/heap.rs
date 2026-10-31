//! Der Heap eines Agents (`docs/architektur.md`, Abschnitte 2.4 und 4.9).
//!
//! Zellen liegen in Seiten zu 256 KiB, Zellen über 256 KiB in eigenen
//! Seiten. Ein Verweis ist ein 32-Bit-Abstand: oben 14 Bit für die Nummer der
//! Seite, unten 18 Bit für die Lage der Zelle in Bytes. Weil Verweise keine
//! Zeiger sind, kommt der Heap ohne `unsafe` aus, gelesen wird mit `get()`
//! (RS-7). Jede Zelle beginnt mit einem Kopfwort: Art in den Bits 0 bis 7,
//! Kennzeichen in den Bits 8 bis 31, Länge in den Bits 32 bis 63.
//!
//! Die meisten Zellen bestehen nach dem Kopfwort aus Werten: Felder,
//! Symbole, Gestalten, schwache Felder und dünn besetzte Elemente. Objekte
//! beginnen mit zwei Wörtern, die je zwei 32-Bit-Verweise packen (Gestalt
//! und Prototyp, Elemente und Zusatzfeld), dann folgen ihre Werte. Die
//! Felder schwacher Zellen halten ihr Ziel nicht am Leben; nach dem Sammeln
//! zeigen sie auf den neuen Ort oder sind *undefined*.
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
//! [`Persistent`] für langlebige Verweise, dazu der Registerstapel des
//! Interpreters. Jeder offene Bereich hat eine
//! eigene Liste von Handles. Ein Handle gehört dem Bereich, für den es
//! angelegt wurde, auch wenn das in einem inneren Bereich geschieht, und
//! endet erst mit ihm. Gelesene Werte ([`Value`]) leihen
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
//! Bereiche und Handles tragen die Nummer ihres Heaps. Wer sie mit einem
//! anderen Heap verwendet, löst eine Panic aus. Werte tragen keine Nummer,
//! dafür ist beim NaN-Boxing kein Platz. Ein Wert gehört dem Heap, aus dem
//! er gelesen wurde, und nur an ihn darf er zurückgehen.
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
use std::sync::atomic::{AtomicU64, Ordering};

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
/// Höchstens so viele Register hat der Registerstapel, zusammen 8 MiB.
pub const MAX_REGISTERS: usize = 1 << 20;

/// Die Nummer des nächsten Heaps. Mit 64 Bit läuft sie praktisch nie über.
static NEXT_HEAP: AtomicU64 = AtomicU64::new(0);

const KIND_FORWARD: u64 = 1;
const KIND_LATIN1: u64 = 2;
const KIND_UTF16: u64 = 3;
const KIND_SLOTS: u64 = 4;
const KIND_SYMBOL: u64 = 5;
const KIND_SHAPE: u64 = 6;
const KIND_WEAK: u64 = 7;
const KIND_SPARSE: u64 = 8;
const KIND_OBJECT: u64 = 9;
const KIND_ARRAY: u64 = 10;

/// Die Kennzeichen stehen in den Bits 8 bis 31 des Kopfworts.
const FLAG_SHIFT: u32 = 8;
const FLAG_MASK: u64 = 0xFF_FFFF;

/// Wörter mit gepackten Verweisen am Anfang eines Objekts.
const OBJECT_REFS: usize = 2;
/// Ein gepackter Verweis auf nichts. Kein Abstand hat diesen Wert, denn
/// Zellen beginnen an Wortgrenzen.
const NONE: u32 = u32::MAX;

fn header(kind: u64, length: u32) -> u64 {
    kind | (u64::from(length) << 32)
}

fn header_kind(header: u64) -> u64 {
    header & 0xFF
}

fn header_flags(header: u64) -> u32 {
    // Die Maske lässt 24 Bit übrig.
    u32::try_from((header >> FLAG_SHIFT) & FLAG_MASK).unwrap_or(0)
}

fn with_flags(header: u64, flags: u32) -> u64 {
    (header & !(FLAG_MASK << FLAG_SHIFT)) | ((u64::from(flags) & FLAG_MASK) << FLAG_SHIFT)
}

/// Zellen, die nach dem Kopfwort nur aus Werten bestehen.
fn is_value_cell(kind: u64) -> bool {
    matches!(
        kind,
        KIND_SLOTS | KIND_SYMBOL | KIND_SHAPE | KIND_WEAK | KIND_SPARSE
    )
}

/// Die Arten der Objektzellen.
const OBJECT_KINDS: [u64; 2] = [KIND_OBJECT, KIND_ARRAY];

fn is_object(kind: u64) -> bool {
    OBJECT_KINDS.contains(&kind)
}

fn packed(low: u32, high: u32) -> u64 {
    u64::from(low) | (u64::from(high) << 32)
}

fn packed_low(word: u64) -> u32 {
    u32::try_from(word & 0xFFFF_FFFF).unwrap_or(NONE)
}

fn packed_high(word: u64) -> u32 {
    u32::try_from(word >> 32).unwrap_or(NONE)
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
        kind if is_value_cell(kind) => length,
        kind if is_object(kind) => length.checked_add(OBJECT_REFS)?,
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
    heap: u64,
    /// Die Nummer des Bereichs, 0 für den äußersten. Innere Bereiche enden
    /// vor den äußeren, also ist die Nummer eindeutig, solange er offen ist.
    depth: usize,
}

/// Ein Handle in einem Bereich. Der Garbage Collector führt es beim
/// Verschieben nach, und es gilt, bis sein Bereich endet.
#[derive(Clone, Copy, Debug)]
pub struct Local<'s> {
    heap: u64,
    depth: usize,
    index: usize,
    scope: PhantomData<&'s Scope>,
}

/// Ein Handle ohne Bereich, bis [`Heap::release`] es freigibt.
#[derive(Debug)]
pub struct Persistent {
    heap: u64,
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
    /// Die Nummer, an der Bereiche und Handles ihren Heap erkennen.
    id: u64,
    pages: Vec<Option<Page>>,
    free_pages: Vec<usize>,
    /// Seiten zu 256 KiB in der Reihenfolge, in der sie angelegt wurden; in
    /// die letzte wird angefordert.
    small: Vec<usize>,
    /// Seiten mit einer großen Zelle, ebenfalls in Reihenfolge.
    large: Vec<usize>,
    /// Die Handles der offenen Bereiche, eine Liste je Bereich, der innerste
    /// zuletzt.
    roots: RefCell<Vec<Vec<u64>>>,
    persistent: Vec<u64>,
    free_persistent: Vec<usize>,
    /// Der Registerstapel des Interpreters: die Register aller Rahmen
    /// hintereinander, der jüngste Rahmen zuletzt.
    registers: RefCell<Vec<u64>>,
    /// Bytes in Zellen.
    used: usize,
    /// Ab hier wird vor der nächsten Anforderung gesammelt.
    trigger: usize,
    options: Options,
    collections: u64,
    /// Während einer Sammlung die schon kopierten schwachen Zellen.
    weak: Vec<u32>,
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
            id: NEXT_HEAP.fetch_add(1, Ordering::Relaxed),
            pages: Vec::new(),
            free_pages: Vec::new(),
            small: Vec::new(),
            large: Vec::new(),
            roots: RefCell::new(Vec::new()),
            persistent: Vec::new(),
            free_persistent: Vec::new(),
            registers: RefCell::new(Vec::new()),
            used: 0,
            trigger: MIN_TRIGGER.min(options.limit),
            options,
            collections: 0,
            weak: Vec::new(),
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

    /// Führt `f` in einem neuen Bereich aus. Handles für diesen Bereich
    /// gelten bis zum Ende von `f`; die Typen verhindern, dass sie
    /// entkommen. Bereiche lassen sich schachteln, und in einem inneren
    /// Bereich lassen sich Handles für einen äußeren anlegen.
    pub fn scope<R, F: FnOnce(&mut Heap, &Scope) -> R>(&mut self, f: F) -> R {
        let scope = self.enter_scope();
        let result = f(self, &scope);
        self.leave_scope(&scope);
        result
    }

    /// Öffnet einen Bereich. Wer das aufruft, schließt ihn mit
    /// [`Heap::leave_scope`] und gibt ihn nur als Referenz weiter, so wie
    /// [`Heap::scope`].
    pub(crate) fn enter_scope(&mut self) -> Scope {
        let roots = self.roots.get_mut();
        let depth = roots.len();
        roots.push(Vec::new());
        Scope {
            heap: self.id,
            depth,
        }
    }

    pub(crate) fn leave_scope(&mut self, scope: &Scope) {
        // Alle inneren Bereiche sind schon beendet.
        self.roots.get_mut().truncate(scope.depth);
    }

    /// Hält `value` in einem Handle des Bereichs `scope`.
    ///
    /// # Panics
    ///
    /// Wenn `scope` zu einem anderen Heap gehört.
    pub fn local<'s>(&self, scope: &'s Scope, value: Value<'_>) -> Local<'s> {
        self.check_owner(scope.heap, "Bereich");
        let mut roots = self.roots.borrow_mut();
        // Ein offener Bereich dieses Heaps hat immer eine Liste.
        let index = roots.get_mut(scope.depth).map_or(0, |list| {
            let index = list.len();
            list.push(value.bits());
            index
        });
        Local {
            heap: self.id,
            depth: scope.depth,
            index,
            scope: PhantomData,
        }
    }

    /// # Panics
    ///
    /// Wenn `local` zu einem anderen Heap gehört.
    pub fn get(&self, local: Local<'_>) -> Value<'_> {
        self.check_owner(local.heap, "Handle");
        let roots = self.roots.borrow();
        let bits = roots
            .get(local.depth)
            .and_then(|list| list.get(local.index))
            .copied();
        Value::from_bits(bits.unwrap_or(Value::UNDEFINED.bits()))
    }

    /// # Panics
    ///
    /// Wenn `local` zu einem anderen Heap gehört.
    pub fn set(&self, local: Local<'_>, value: Value<'_>) {
        self.check_owner(local.heap, "Handle");
        let mut roots = self.roots.borrow_mut();
        if let Some(slot) = roots
            .get_mut(local.depth)
            .and_then(|list| list.get_mut(local.index))
        {
            *slot = value.bits();
        }
    }

    /// Hält den Wert von `local` über alle Bereiche hinweg.
    ///
    /// # Panics
    ///
    /// Wenn `local` zu einem anderen Heap gehört.
    pub fn persist(&mut self, local: Local<'_>) -> Persistent {
        let bits = self.get(local).bits();
        if let Some(index) = self.free_persistent.pop()
            && let Some(slot) = self.persistent.get_mut(index)
        {
            *slot = bits;
            return Persistent {
                heap: self.id,
                index,
            };
        }
        self.persistent.push(bits);
        Persistent {
            heap: self.id,
            index: self.persistent.len() - 1,
        }
    }

    /// # Panics
    ///
    /// Wenn `handle` zu einem anderen Heap gehört.
    pub fn get_persistent(&self, handle: &Persistent) -> Value<'_> {
        self.check_owner(handle.heap, "dauerhaftes Handle");
        let bits = self.persistent.get(handle.index).copied();
        Value::from_bits(bits.unwrap_or(Value::UNDEFINED.bits()))
    }

    /// # Panics
    ///
    /// Wenn `handle` zu einem anderen Heap gehört.
    pub fn release(&mut self, handle: Persistent) {
        self.check_owner(handle.heap, "dauerhaftes Handle");
        if let Some(slot) = self.persistent.get_mut(handle.index) {
            *slot = Value::UNDEFINED.bits();
            self.free_persistent.push(handle.index);
        }
    }

    /// Ersetzt den Wert von `handle` durch den von `local`.
    ///
    /// # Panics
    ///
    /// Wenn eines der Handles zu einem anderen Heap gehört.
    pub fn set_persistent(&mut self, handle: &Persistent, local: Local<'_>) {
        self.check_owner(handle.heap, "dauerhaftes Handle");
        let bits = self.get(local).bits();
        if let Some(slot) = self.persistent.get_mut(handle.index) {
            *slot = bits;
        }
    }

    // Register

    /// Legt `count` Register mit *undefined* oben auf den Stapel und gibt
    /// die Nummer des ersten zurück; `None`, wenn der Stapel dann mehr als
    /// [`MAX_REGISTERS`] hätte.
    pub(crate) fn push_registers(&mut self, count: usize) -> Option<usize> {
        let registers = self.registers.get_mut();
        let base = registers.len();
        let end = base
            .checked_add(count)
            .filter(|&end| end <= MAX_REGISTERS)?;
        registers.resize(end, Value::UNDEFINED.bits());
        Some(base)
    }

    /// Nimmt alle Register ab `base` vom Stapel.
    pub(crate) fn truncate_registers(&mut self, base: usize) {
        self.registers.get_mut().truncate(base);
    }

    /// Der Wert im Register `index`, *undefined* hinter dem Stapel.
    pub(crate) fn register(&self, index: usize) -> Value<'_> {
        let bits = self.registers.borrow().get(index).copied();
        Value::from_bits(bits.unwrap_or(Value::UNDEFINED.bits()))
    }

    pub(crate) fn set_register(&self, index: usize, value: Value<'_>) {
        if let Some(slot) = self.registers.borrow_mut().get_mut(index) {
            *slot = value.bits();
        }
    }

    fn check_owner(&self, heap: u64, what: &str) {
        assert!(heap == self.id, "{what} eines anderen Heaps");
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
        self.alloc_values(scope, KIND_SLOTS, len)
    }

    /// Legt eine Zelle der Art `kind` aus `len` Werten an, alle
    /// *undefined*. Der Verweis hat die Art Symbol oder Internal.
    fn alloc_values<'s>(
        &mut self,
        scope: &'s Scope,
        kind: u64,
        len: usize,
    ) -> Result<Local<'s>, OutOfMemory> {
        let length = u32::try_from(len).map_err(|_| OutOfMemory)?;
        let head = header(kind, length);
        let words = cell_words(head).ok_or(OutOfMemory)?;
        let offset = self.allocate(words)?;
        self.init(offset, 0, head);
        for index in 1..words {
            self.init(offset, index, Value::UNDEFINED.bits());
        }
        let reference = if kind == KIND_SYMBOL {
            RefKind::Symbol
        } else {
            RefKind::Internal
        };
        Ok(self.local(scope, Value::reference(reference, offset)))
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

        let mut roots = std::mem::take(self.roots.get_mut());
        for slot in roots.iter_mut().flatten() {
            *slot = self.evacuate(*slot);
        }
        *self.roots.get_mut() = roots;
        let mut persistent = std::mem::take(&mut self.persistent);
        for slot in &mut persistent {
            *slot = self.evacuate(*slot);
        }
        self.persistent = persistent;
        let mut registers = std::mem::take(self.registers.get_mut());
        for slot in &mut registers {
            *slot = self.evacuate(*slot);
        }
        *self.registers.get_mut() = registers;

        self.scan();
        self.clear_weak();

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
        match header_kind(head) {
            KIND_WEAK => self.weak.push(offset),
            kind if is_value_cell(kind) => self.trace_values(offset, 1, words),
            kind if is_object(kind) => {
                self.trace_packed(offset, 1, RefKind::Internal, RefKind::Object);
                self.trace_packed(offset, 2, RefKind::Internal, RefKind::Internal);
                self.trace_values(offset, 1 + OBJECT_REFS, words);
            }
            _ => {}
        }
        Some(words)
    }

    /// Führt die Werte in den Wörtern `from..to` der Zelle nach.
    fn trace_values(&mut self, offset: u32, from: usize, to: usize) {
        for slot in from..to {
            if let Some(bits) = self.word(offset, slot) {
                let moved = self.evacuate(bits);
                self.write(offset, slot, moved);
            }
        }
    }

    /// Führt die beiden gepackten Verweise im Wort `index` nach.
    fn trace_packed(&mut self, offset: u32, index: usize, low: RefKind, high: RefKind) {
        let Some(word) = self.word(offset, index) else {
            return;
        };
        let low = self.evacuate_packed(low, packed_low(word));
        let high = self.evacuate_packed(high, packed_high(word));
        self.write(offset, index, packed(low, high));
    }

    fn evacuate_packed(&mut self, kind: RefKind, offset: u32) -> u32 {
        if offset == NONE {
            return NONE;
        }
        let moved = self.evacuate(Value::reference(kind, offset).bits());
        Value::from_bits(moved)
            .reference_parts()
            .map_or(offset, |(_, moved)| moved)
    }

    /// Führt nach dem Lesezeiger die Felder der schwachen Zellen nach: Ein
    /// Ziel, das verschoben wurde, bekommt seinen neuen Ort, eines im alten
    /// Raum wird *undefined*.
    fn clear_weak(&mut self) {
        for offset in std::mem::take(&mut self.weak) {
            let count = self
                .word(offset, 0)
                .map_or(0, |head| to_usize(header_length(head)));
            for slot in 1..=count {
                let Some(bits) = self.word(offset, slot) else {
                    continue;
                };
                let Some((kind, target)) = Value::from_bits(bits).reference_parts() else {
                    continue;
                };
                if !self.page(target).is_some_and(|page| page.old) {
                    continue;
                }
                let moved = match self.word(target, 0) {
                    Some(head) if header_kind(head) == KIND_FORWARD => {
                        Value::reference(kind, header_length(head)).bits()
                    }
                    _ => Value::UNDEFINED.bits(),
                };
                self.write(offset, slot, moved);
            }
        }
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
    /// jeder Verweis in Handles, Feldern und Objekten zeigt auf den Anfang
    /// einer passenden Zelle.
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
        let fits = |kind: RefKind, cell: u64| match kind {
            RefKind::String => matches!(cell, KIND_LATIN1 | KIND_UTF16),
            RefKind::Symbol => cell == KIND_SYMBOL,
            RefKind::Object => is_object(cell),
            RefKind::Internal => {
                matches!(cell, KIND_SLOTS | KIND_SHAPE | KIND_WEAK | KIND_SPARSE)
            }
            RefKind::BigInt => false,
        };
        let check = |bits: u64, place: &str| -> Result<(), String> {
            let Some((kind, offset)) = Value::from_bits(bits).reference_parts() else {
                return Ok(());
            };
            if starts.get(&offset).is_some_and(|&cell| fits(kind, cell)) {
                return Ok(());
            }
            Err(format!(
                "{place}: Verweis {offset:#x} ({kind:?}) zeigt auf keine passende Zelle"
            ))
        };
        let check_packed = |offset: u32, allowed: &[u64], place: &str| -> Result<(), String> {
            if offset == NONE
                || starts
                    .get(&offset)
                    .is_some_and(|cell| allowed.contains(cell))
            {
                return Ok(());
            }
            Err(format!(
                "{place}: Verweis {offset:#x} zeigt auf keine passende Zelle"
            ))
        };
        for &bits in self.roots.borrow().iter().flatten() {
            check(bits, "Handle")?;
        }
        for &bits in &self.persistent {
            check(bits, "dauerhaftes Handle")?;
        }
        for &bits in self.registers.borrow().iter() {
            check(bits, "Register")?;
        }
        for (&offset, &kind) in &starts {
            let count = self
                .word(offset, 0)
                .map_or(0, |head| to_usize(header_length(head)));
            if is_value_cell(kind) {
                for slot in 1..=count {
                    check(self.word(offset, slot).unwrap_or(0), "Feld")?;
                }
            } else if is_object(kind) {
                let refs = self.word(offset, 1).unwrap_or(0);
                if packed_low(refs) == NONE {
                    return Err(format!("Objekt {offset:#x} ohne Gestalt"));
                }
                check_packed(packed_low(refs), &[KIND_SHAPE], "Gestalt")?;
                check_packed(packed_high(refs), &OBJECT_KINDS, "Prototyp")?;
                let more = self.word(offset, 2).unwrap_or(0);
                check_packed(packed_low(more), &[KIND_SLOTS, KIND_SPARSE], "Elemente")?;
                check_packed(packed_high(more), &[KIND_SLOTS], "Zusatzfeld")?;
                for slot in 0..count {
                    let bits = self.word(offset, 1 + OBJECT_REFS + slot).unwrap_or(0);
                    check(bits, "Eigenschaft")?;
                }
            }
        }
        Ok(())
    }
}

mod cells;

pub(crate) use cells::{CellKind, ObjectClass};

#[cfg(test)]
mod tests;
