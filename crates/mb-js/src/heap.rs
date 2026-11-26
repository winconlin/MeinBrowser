//! Der Heap eines Agents (`docs/architektur.md`, Abschnitte 2.4 und 4.9).
//!
//! Zellen liegen in Seiten zu 256 KiB, Zellen über 256 KiB in eigenen
//! Seiten. Ein Verweis ist ein 32-Bit-Abstand: oben 14 Bit für die Nummer der
//! Seite, unten 18 Bit für die Lage der Zelle in Bytes. Weil Verweise keine
//! Zeiger sind, kommt der Heap ohne `unsafe` aus, gelesen wird mit `get()`
//! (RS-7). Jede Zelle beginnt mit einem Kopfwort: Art in den Bits 0 bis 7,
//! Kennzeichen in den Bits 8 bis 31, Länge in den Bits 32 bis 63.
//!
//! Zeichenketten, BigInts und die Daten von ArrayBuffern bestehen nach dem
//! Kopfwort aus Rohdaten: den Codeeinheiten, den Wörtern des Betrags mit dem
//! Vorzeichen als Kennzeichen oder den Bytes, acht je Wort, das erste im
//! niedrigsten. Die meisten anderen Zellen bestehen nach dem Kopfwort aus
//! Werten: Felder, Symbole, Gestalten, schwache Felder und dünn besetzte
//! Elemente. Objekte beginnen mit zwei Wörtern, die je zwei 32-Bit-Verweise
//! packen (Gestalt und Prototyp, Elemente und Zusatzfeld), dann folgen ihre
//! Werte:
//! Funktionsobjekte, eingebaute Funktionen, arguments-Objekte und
//! Wrapper-Objekte haben zuerst interne Felder,
//! danach kommen bei allen Objekten die eingebetteten Plätze. Die Felder
//! schwacher Zellen halten ihr Ziel nicht am Leben; nach dem Sammeln zeigen
//! sie auf den neuen Ort oder sind *undefined*. Eine schwache Zelle mit dem
//! Kennzeichen für Finalisierung hält im ersten Feld ihre
//! FinalizationRegistry: Leert das Sammeln eines der übrigen Felder, kommt
//! die Registry in die Liste, aus der der Agent Aufräum-Jobs macht
//! ([`Heap::pop_cleanup`]).
//!
//! Ephemeron-Zellen tragen die Einträge von WeakMaps und WeakSets
//! (Abschnitt 9.9.3): Einträge gleicher Breite, die Breite steht in den
//! Kennzeichen. Das erste Feld eines Eintrags ist sein Schlüssel. Die
//! übrigen Felder hält der Collector nur, solange der Schlüssel auf anderem
//! Weg erreichbar ist; ist er es nicht, wird der Schlüssel zur Lücke, und
//! Verweise in den übrigen Feldern werden *undefined*, Zahlen bleiben.
//!
//! Die Bytes eines SharedArrayBuffer liegen nicht im Heap, denn andere
//! Agents sollen sie lesen können. Eine Zelle der Art für geteilte Blöcke
//! hält nur die Nummer eines Eintrags in einer Tabelle des Heaps, und der
//! Eintrag hält den Block. Das Sammeln gibt Einträge frei, deren Zelle es
//! nicht erreicht hat. Die Bytes der Blöcke zählen zum Speicherbudget.
//!
//! Gesammelt wird präzise und kopierend nach Cheney, in zwei Generationen.
//! Neue Zellen liegen in jungen Seiten. Eine junge Sammlung
//! ([`Heap::collect_young`]) verurteilt nur die jungen Seiten: Die
//! erreichbaren jungen Zellen wandern in alte Seiten (jede Zelle, die eine
//! Sammlung übersteht, ist danach alt), und ein Lesezeiger über die eben
//! kopierten Zellen führt deren Verweise nach. Wurzeln sind dabei außer den
//! Handles die alten Zellen der gemerkten Menge. Eine volle Sammlung
//! ([`Heap::collect`]) verurteilt alle Seiten und kopiert alles Erreichbare
//! in neue alte Seiten. Beides kommt ohne Rekursion und ohne Markierstapel
//! aus (RS-8).
//!
//! Die Schreibbarriere sitzt in der einzigen Funktion, über die ein Wert in
//! eine bestehende Zelle gespeichert wird: Zeigt er aus einer alten Zelle auf
//! eine junge, kommt die alte Zelle in die gemerkte Menge. Eine Zelle, die
//! gerade angelegt wird, ist jung und braucht keine Barriere. Nach jeder
//! Sammlung gibt es keine jungen Zellen mehr, also ist die Menge danach
//! leer. Der Prüfmodus kontrolliert, dass jeder Verweis aus einer alten in
//! eine junge Zelle in der Menge steht.
//!
//! Gesammelt wird jung, sobald die jungen Zellen [`NURSERY`] Bytes
//! überschreiten würden, und voll, sobald alle Zellen die Schwelle aus der
//! letzten vollen Sammlung überschreiten würden. Der Belastungsmodus
//! sammelt vor jeder Anforderung, abwechselnd jung und voll.
//!
//! Erreicht der Lesezeiger eine Ephemeron-Zelle, ist jeder ihrer Einträge
//! bereit, wenn sein Schlüssel schon kopiert ist, sonst wartet er in einer
//! Tabelle nach dem alten Ort des Schlüssels. Kopiert der Collector später
//! eine Zelle, die dort steht, werden ihre Einträge bereit. Bereite Einträge
//! führt er nach, sobald der Lesezeiger fertig ist, und liest danach weiter,
//! bis beides nichts mehr findet. So kommt jeder Eintrag höchstens einmal
//! dran, auch wenn Werte die Schlüssel anderer Einträge erreichen, und die
//! Arbeit bleibt linear. Was danach noch wartet, ist nicht erreichbar.
//!
//! Wurzeln sind die Handles: [`Local`] in einem Bereich ([`Heap::scope`]) und
//! [`Persistent`] für langlebige Verweise, dazu der Registerstapel des
//! Interpreters, die [[KeptAlive]]-Liste des Agents (Abschnitt 9.11) und
//! die Registries, deren Aufräum-Jobs warten. Jeder offene Bereich hat eine
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
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bigint::BigInt;
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
/// Ab so vielen Bytes in jungen Zellen wird jung gesammelt.
const NURSERY: usize = 2 << 20;
/// So viele frei gewordene kleine Seiten behält der Heap für die nächsten
/// Anforderungen, statt sie neu anzulegen: so viele, wie junge Zellen
/// füllen, bevor jung gesammelt wird. Im Belastungsmodus wird vor jeder
/// Anforderung gesammelt; ohne Vorrat kostete jede Sammlung eine neue Seite
/// zu 256 KiB samt Nullen.
const SPARE_PAGES: usize = NURSERY / (PAGE_WORDS * 8);
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
const KIND_FUNCTION: u64 = 11;
const KIND_ARGUMENTS: u64 = 12;
const KIND_ERROR: u64 = 13;
const KIND_BUILTIN: u64 = 14;
const KIND_PRIMITIVE: u64 = 15;
const KIND_ITERATOR: u64 = 16;
const KIND_GENERATOR: u64 = 17;
const KIND_PROMISE: u64 = 18;
const KIND_ASYNC_FROM_SYNC: u64 = 19;
const KIND_ASYNC_GENERATOR: u64 = 20;
const KIND_BOUND_FUNCTION: u64 = 21;
const KIND_COLLECTION: u64 = 22;
const KIND_PROXY: u64 = 23;
const KIND_REGEXP: u64 = 24;
const KIND_BIGINT: u64 = 25;
const KIND_DATE: u64 = 26;
const KIND_BYTES: u64 = 27;
const KIND_ARRAY_BUFFER: u64 = 28;
const KIND_DATA_VIEW: u64 = 29;
const KIND_TYPED_ARRAY: u64 = 30;
const KIND_EPHEMERON: u64 = 31;
const KIND_WEAK_REF: u64 = 32;
const KIND_FINALIZATION_REGISTRY: u64 = 33;
const KIND_SHARED: u64 = 34;
const KIND_SHARED_ARRAY_BUFFER: u64 = 35;
const KIND_NAMESPACE: u64 = 36;

/// Die Kennzeichen stehen in den Bits 8 bis 31 des Kopfworts.
const FLAG_SHIFT: u32 = 8;
const FLAG_MASK: u64 = 0xFF_FFFF;
/// Kennzeichen eines negativen BigInt.
const NEGATIVE_FLAG: u32 = 1;
/// Kennzeichen einer schwachen Zelle, deren erstes Feld eine
/// FinalizationRegistry hält.
const FINALIZER_FLAG: u32 = 1;

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
        KIND_SLOTS | KIND_SYMBOL | KIND_SHAPE | KIND_WEAK | KIND_SPARSE | KIND_EPHEMERON
    )
}

/// Die Breite der Einträge einer Ephemeron-Zelle, mindestens 1.
fn ephemeron_width(header: u64) -> usize {
    to_usize(header_flags(header)).max(1)
}

/// Die Arten der Objektzellen.
const OBJECT_KINDS: [u64; 24] = [
    KIND_OBJECT,
    KIND_ARRAY,
    KIND_FUNCTION,
    KIND_ARGUMENTS,
    KIND_ERROR,
    KIND_BUILTIN,
    KIND_PRIMITIVE,
    KIND_ITERATOR,
    KIND_GENERATOR,
    KIND_PROMISE,
    KIND_ASYNC_FROM_SYNC,
    KIND_ASYNC_GENERATOR,
    KIND_BOUND_FUNCTION,
    KIND_COLLECTION,
    KIND_PROXY,
    KIND_REGEXP,
    KIND_DATE,
    KIND_ARRAY_BUFFER,
    KIND_DATA_VIEW,
    KIND_TYPED_ARRAY,
    KIND_WEAK_REF,
    KIND_FINALIZATION_REGISTRY,
    KIND_SHARED_ARRAY_BUFFER,
    KIND_NAMESPACE,
];

/// Die internen Felder eines Funktionsobjekts, Werte vor den eingebetteten
/// Plätzen.
pub(crate) const FUNCTION_FIELDS: usize = 6;
/// Das interne Feld eines arguments-Objekts: [[ParameterMap]].
pub(crate) const ARGUMENTS_FIELDS: usize = 1;
/// Die internen Felder einer eingebauten Funktion: ihr Eintrag, ihr Realm
/// und was sie als Closure einfängt.
pub(crate) const BUILTIN_FIELDS: usize = 3;
/// Das interne Feld eines Wrapper-Objekts: der primitive Wert.
pub(crate) const PRIMITIVE_FIELDS: usize = 1;
/// Die internen Felder eines Iterators für Arrays oder Zeichenketten.
pub(crate) const ITERATOR_FIELDS: usize = 3;
/// Die internen Felder eines Generators.
pub(crate) const GENERATOR_FIELDS: usize = 5;
/// Die internen Felder eines Promise.
pub(crate) const PROMISE_FIELDS: usize = 4;
/// Die internen Felder eines Async-from-Sync Iterator: [[Iterator]] und
/// [[NextMethod]] seines [[SyncIteratorRecord]].
pub(crate) const ASYNC_FROM_SYNC_FIELDS: usize = 2;
/// Die internen Felder eines async-Generators.
pub(crate) const ASYNC_GENERATOR_FIELDS: usize = 8;
/// Die internen Felder einer gebundenen Funktion: [[BoundTargetFunction]],
/// [[BoundThis]] und [[BoundArguments]].
pub(crate) const BOUND_FUNCTION_FIELDS: usize = 3;
/// Die internen Felder einer Map oder eines Set: die Art und die Tabelle
/// mit [[MapData]] beziehungsweise [[SetData]].
pub(crate) const COLLECTION_FIELDS: usize = 2;
/// Die internen Felder eines Proxys: [[ProxyTarget]], [[ProxyHandler]] und
/// ob er [[Call]] und [[Construct]] hat.
pub(crate) const PROXY_FIELDS: usize = 3;
/// Die internen Felder eines regulären Ausdrucks: [[OriginalSource]],
/// [[OriginalFlags]] und die Nummer seines Programms, die für
/// [[RegExpRecord]] und [[RegExpMatcher]] steht.
pub(crate) const REGEXP_FIELDS: usize = 3;
/// Das interne Feld eines Date-Objekts: [[DateValue]], eine Number.
pub(crate) const DATE_FIELDS: usize = 1;
/// Die internen Felder eines ArrayBuffer: [[ArrayBufferData]], eine Zelle
/// aus Bytes oder *null*, wenn er abgelöst ist, und
/// [[ArrayBufferMaxByteLength]], eine Number oder *undefined* für einen
/// ArrayBuffer fester Länge. [[ArrayBufferByteLength]] ist die Länge der
/// Zelle. Ein SharedArrayBuffer hat dieselben Felder; [[ArrayBufferData]]
/// ist bei ihm eine Zelle für einen geteilten Block, der auch
/// [[ArrayBufferByteLengthData]] hält.
pub(crate) const ARRAY_BUFFER_FIELDS: usize = 2;
/// Die internen Felder eines DataView: [[ViewedArrayBuffer]],
/// [[ByteLength]] als Number oder *undefined* für ~auto~ und
/// [[ByteOffset]] als Number. [[DataView]] hat keinen Wert, die Art sagt
/// alles.
pub(crate) const DATA_VIEW_FIELDS: usize = 3;
/// Die internen Felder eines TypedArray: [[ViewedArrayBuffer]], die Art der
/// Elemente als Zahl (sie steht für [[TypedArrayName]] und
/// [[ContentType]]), [[ArrayLength]] als Number oder *undefined* für ~auto~
/// und [[ByteOffset]] als Number. [[ByteLength]] ist [[ArrayLength]] mal
/// die Größe eines Elements, ~auto~ mit [[ArrayLength]].
pub(crate) const TYPED_ARRAY_FIELDS: usize = 4;
/// Das interne Feld einer WeakRef: eine schwache Zelle mit
/// [[WeakRefTarget]] im einzigen Feld.
pub(crate) const WEAK_REF_FIELDS: usize = 1;
/// Die internen Felder einer FinalizationRegistry: [[Realm]] als Zahl,
/// [[CleanupCallback]], die schwache Zelle mit Registry, Zielen und
/// Abmeldemarken ihrer [[Cells]], die Zelle mit den gehaltenen Werten und
/// wie viele Einträge belegt sind.
pub(crate) const FINALIZATION_REGISTRY_FIELDS: usize = 5;
/// Das interne Feld eines Namensraum-Objekts: die Nummer seines Moduls,
/// das für [[Module]] und [[Exports]] steht.
pub(crate) const NAMESPACE_FIELDS: usize = 1;

fn is_object(kind: u64) -> bool {
    OBJECT_KINDS.contains(&kind)
}

/// Wie viele interne Felder ein Objekt dieser Art vor seinen eingebetteten
/// Plätzen hat.
fn object_fields(kind: u64) -> usize {
    match kind {
        KIND_FUNCTION => FUNCTION_FIELDS,
        KIND_ARGUMENTS => ARGUMENTS_FIELDS,
        KIND_BUILTIN => BUILTIN_FIELDS,
        KIND_PRIMITIVE => PRIMITIVE_FIELDS,
        KIND_ITERATOR => ITERATOR_FIELDS,
        KIND_GENERATOR => GENERATOR_FIELDS,
        KIND_PROMISE => PROMISE_FIELDS,
        KIND_ASYNC_FROM_SYNC => ASYNC_FROM_SYNC_FIELDS,
        KIND_ASYNC_GENERATOR => ASYNC_GENERATOR_FIELDS,
        KIND_BOUND_FUNCTION => BOUND_FUNCTION_FIELDS,
        KIND_COLLECTION => COLLECTION_FIELDS,
        KIND_PROXY => PROXY_FIELDS,
        KIND_REGEXP => REGEXP_FIELDS,
        KIND_DATE => DATE_FIELDS,
        KIND_ARRAY_BUFFER | KIND_SHARED_ARRAY_BUFFER => ARRAY_BUFFER_FIELDS,
        KIND_DATA_VIEW => DATA_VIEW_FIELDS,
        KIND_TYPED_ARRAY => TYPED_ARRAY_FIELDS,
        KIND_WEAK_REF => WEAK_REF_FIELDS,
        KIND_FINALIZATION_REGISTRY => FINALIZATION_REGISTRY_FIELDS,
        KIND_NAMESPACE => NAMESPACE_FIELDS,
        _ => 0,
    }
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
        KIND_BIGINT => length,
        KIND_BYTES => length.div_ceil(8),
        KIND_SHARED => length,
        kind if is_value_cell(kind) => length,
        kind if is_object(kind) => length
            .checked_add(OBJECT_REFS)?
            .checked_add(object_fields(kind))?,
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
    /// Sammeln vor jeder Anforderung, abwechselnd jung und voll
    /// (Belastungsmodus). Während ein Realm entsteht, setzt er aus und eine
    /// volle Sammlung am Ende verschiebt den fertigen Realm; siehe
    /// [`Heap::pause_stress`].
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
    /// Gehört während einer Sammlung zum Raum, der gesammelt wird.
    condemned: bool,
    /// Gehört zur jungen Generation.
    young: bool,
}

/// Die Seiten einer Generation, jede Liste in der Reihenfolge, in der die
/// Seiten angelegt wurden.
#[derive(Default)]
struct Space {
    /// Seiten zu 256 KiB; in die letzte wird angefordert.
    small: Vec<usize>,
    /// Seiten mit einer großen Zelle.
    large: Vec<usize>,
}

/// Der Heap eines Agents.
pub struct Heap {
    /// Die Nummer, an der Bereiche und Handles ihren Heap erkennen.
    id: u64,
    pages: Vec<Option<Page>>,
    free_pages: Vec<usize>,
    /// Vorrat an Wortfeldern kleiner Seiten, bis zur früheren Belegung
    /// genullt; siehe [`SPARE_PAGES`].
    spare: Vec<Box<[Cell<u64>]>>,
    /// Die Seiten der jungen Zellen; hierhin fordert der Heap an.
    young: Space,
    /// Die Seiten der alten Zellen; hierhin kopiert das Sammeln.
    old: Space,
    /// Die alten Zellen, die seit der letzten Sammlung einen Verweis auf
    /// eine junge bekommen haben, nach ihrem Ort geordnet.
    remembered: RefCell<BTreeSet<u32>>,
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
    /// Davon Bytes in jungen Zellen.
    young_used: usize,
    /// Ab hier wird vor der nächsten Anforderung voll gesammelt.
    trigger: usize,
    options: Options,
    /// Solange größer als null, setzt der Belastungsmodus aus.
    paused: u32,
    /// Ob der Belastungsmodus als Nächstes voll sammelt.
    stress_full: bool,
    collections: u64,
    young_collections: u64,
    /// Während einer Sammlung die schon kopierten schwachen Zellen.
    weak: Vec<u32>,
    /// Während einer Sammlung die schon kopierten Ephemeron-Zellen.
    ephemerons: Vec<u32>,
    /// Während einer Sammlung die Einträge der Ephemeron-Zellen, deren
    /// Schlüssel noch nicht kopiert ist, nach dem alten Ort des Schlüssels:
    /// je Eintrag der neue Ort der Zelle und seine Nummer.
    waiting: HashMap<u32, Vec<(u32, u32)>>,
    /// Während einer Sammlung die Einträge, deren Schlüssel kopiert ist und
    /// deren übrige Felder noch nachgeführt werden.
    ready: Vec<(u32, u32)>,
    /// Die [[KeptAlive]]-Liste des Agents (Abschnitt 9.11), jede Zelle
    /// einmal, und die Orte ihrer Zellen.
    kept: Vec<u64>,
    kept_cells: HashSet<u32>,
    /// FinalizationRegistries, deren Ziele das Sammeln geleert hat, bis der
    /// Agent ihre Aufräum-Jobs ausführt.
    cleanup: VecDeque<u64>,
    /// Die geteilten Data Blocks, die Zellen dieses Heaps nennen, und die
    /// freien Plätze der Tabelle.
    shared: Vec<Option<shared::Entry>>,
    free_shared: Vec<usize>,
    /// Bytes in diesen Blöcken; sie zählen zum Budget.
    external: usize,
    /// Ein Eintrag, dessen Zelle gerade angelegt wird, siehe
    /// [`Heap::adopt_shared`].
    pinned_shared: Option<usize>,
}

/// Wo der Lesezeiger in den alten Seiten steht: die Nummer der kleinen
/// Seite in der Liste, das Wort darin und die Nummer der großen Seite.
#[derive(Default)]
struct Position {
    small: usize,
    word: usize,
    large: usize,
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
            spare: Vec::new(),
            young: Space::default(),
            old: Space::default(),
            remembered: RefCell::new(BTreeSet::new()),
            roots: RefCell::new(Vec::new()),
            persistent: Vec::new(),
            free_persistent: Vec::new(),
            registers: RefCell::new(Vec::new()),
            used: 0,
            young_used: 0,
            trigger: MIN_TRIGGER.min(options.limit),
            options,
            paused: 0,
            stress_full: false,
            collections: 0,
            young_collections: 0,
            weak: Vec::new(),
            ephemerons: Vec::new(),
            waiting: HashMap::new(),
            ready: Vec::new(),
            kept: Vec::new(),
            kept_cells: HashSet::new(),
            cleanup: VecDeque::new(),
            shared: Vec::new(),
            free_shared: Vec::new(),
            external: 0,
            pinned_shared: None,
        }
    }

    /// Bytes in Zellen, nach einer vollen Sammlung nur noch erreichbare.
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// Bytes in jungen Zellen, nach jeder Sammlung 0.
    pub fn young_bytes(&self) -> usize {
        self.young_used
    }

    /// Das Speicherbudget in Bytes.
    pub fn limit(&self) -> usize {
        self.options.limit
    }

    /// Wie oft bisher gesammelt wurde, jung oder voll.
    pub fn collections(&self) -> u64 {
        self.collections
    }

    /// Wie oft davon jung gesammelt wurde.
    pub fn young_collections(&self) -> u64 {
        self.young_collections
    }

    /// Ob der Heap im Belastungsmodus ist.
    pub fn stress(&self) -> bool {
        self.options.stress
    }

    /// Setzt den Belastungsmodus aus, bis [`Heap::resume_stress`] ebenso
    /// oft gerufen wurde. Der Aufbau eines Realms läuft so: Er ist für jeden
    /// Realm gleich und kostete sonst eine Sammlung je Anforderung, im
    /// Fuzzer mit seinen frischen Realms je Eingabe den größten Teil der
    /// Zeit. Dass er auch mit Sammlungen vor jeder Anforderung besteht,
    /// prüft ein Test in `realm`.
    pub(crate) fn pause_stress(&mut self) {
        self.paused = self.paused.saturating_add(1);
    }

    pub(crate) fn resume_stress(&mut self) {
        self.paused = self.paused.saturating_sub(1);
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

    // BigInts

    /// Legt ein BigInt an. Wie Zeichenketten sind BigInts unveränderlich.
    pub(crate) fn alloc_bigint<'s>(
        &mut self,
        scope: &'s Scope,
        value: &BigInt,
    ) -> Result<Local<'s>, OutOfMemory> {
        let magnitude = value.magnitude();
        let length = u32::try_from(magnitude.len()).map_err(|_| OutOfMemory)?;
        let flags = if value.is_negative() {
            NEGATIVE_FLAG
        } else {
            0
        };
        let head = with_flags(header(KIND_BIGINT, length), flags);
        let words = cell_words(head).ok_or(OutOfMemory)?;
        let offset = self.allocate(words)?;
        self.init(offset, 0, head);
        for (index, &word) in magnitude.iter().enumerate() {
            self.init(offset, index + 1, word);
        }
        Ok(self.local(scope, Value::reference(RefKind::BigInt, offset)))
    }

    /// Der Wert eines BigInt, `None` für einen Verweis auf etwas anderes.
    pub(crate) fn bigint(&self, bigint: Ref<'_>) -> Option<BigInt> {
        let head = self.bigint_header(bigint)?;
        let magnitude = (1..=to_usize(header_length(head)))
            .map(|index| self.word(bigint.offset(), index))
            .collect::<Option<Vec<u64>>>()?;
        BigInt::from_parts(header_flags(head) & NEGATIVE_FLAG != 0, magnitude).ok()
    }

    /// BigInt::equal (Abschnitt 6.1.6.2.13) zweier BigInt-Zellen, ohne die
    /// Beträge zu kopieren. Die Form ist kanonisch, also sind gleiche Werte
    /// Wort für Wort gleich.
    pub(crate) fn bigints_equal(&self, x: Ref<'_>, y: Ref<'_>) -> bool {
        let (Some(first), Some(second)) = (self.bigint_header(x), self.bigint_header(y)) else {
            return false;
        };
        first == second
            && (1..=to_usize(header_length(first)))
                .all(|index| self.word(x.offset(), index) == self.word(y.offset(), index))
    }

    /// Ob das BigInt 0 ist; ohne den Betrag zu kopieren.
    pub(crate) fn bigint_is_zero(&self, bigint: Ref<'_>) -> bool {
        self.bigint_header(bigint)
            .is_some_and(|head| header_length(head) == 0)
    }

    /// Das Vorzeichen und das niedrigste Wort des Betrags, ohne den Betrag
    /// zu kopieren; 0 für den Wert 0.
    pub(crate) fn bigint_low_word(&self, bigint: Ref<'_>) -> Option<(bool, u64)> {
        let head = self.bigint_header(bigint)?;
        let negative = header_flags(head) & NEGATIVE_FLAG != 0;
        if header_length(head) == 0 {
            return Some((negative, 0));
        }
        Some((negative, self.word(bigint.offset(), 1)?))
    }

    fn bigint_header(&self, bigint: Ref<'_>) -> Option<u64> {
        let head = self.word(bigint.offset(), 0)?;
        (header_kind(head) == KIND_BIGINT).then_some(head)
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

    /// Legt eine Ephemeron-Zelle für `records` Einträge zu je `width`
    /// Werten an, alle *undefined*. Das erste Feld eines Eintrags ist sein
    /// Schlüssel; die übrigen hält die Zelle nur, solange der Schlüssel
    /// anders erreichbar ist (siehe oben).
    pub(crate) fn alloc_ephemerons<'s>(
        &mut self,
        scope: &'s Scope,
        records: usize,
        width: usize,
    ) -> Result<Local<'s>, OutOfMemory> {
        let flags = u32::try_from(width)
            .ok()
            .filter(|&flags| flags >= 1 && u64::from(flags) <= FLAG_MASK)
            .ok_or(OutOfMemory)?;
        let len = records.checked_mul(width).ok_or(OutOfMemory)?;
        let cell = self.alloc_values(scope, KIND_EPHEMERON, len)?;
        if let Some((_, offset)) = self.get(cell).reference_parts()
            && let Some(head) = self.word(offset, 0)
        {
            self.init(offset, 0, with_flags(head, flags));
        }
        Ok(cell)
    }

    /// Legt eine schwache Zelle mit `len` Feldern an, alle *undefined*,
    /// deren erstes Feld eine FinalizationRegistry halten soll: Leert das
    /// Sammeln eines der anderen Felder, kommt die Registry in die Liste der
    /// Aufräum-Jobs.
    pub(crate) fn alloc_finalizer_cell<'s>(
        &mut self,
        scope: &'s Scope,
        len: usize,
    ) -> Result<Local<'s>, OutOfMemory> {
        let cell = self.alloc_values(scope, KIND_WEAK, len)?;
        if let Some((_, offset)) = self.get(cell).reference_parts()
            && let Some(head) = self.word(offset, 0)
        {
            self.init(offset, 0, with_flags(head, FINALIZER_FLAG));
        }
        Ok(cell)
    }

    /// AddToKeptObjects (Abschnitt 9.11). Jede Zelle steht höchstens einmal
    /// in der Liste.
    pub(crate) fn add_to_kept_objects(&mut self, value: Local<'_>) {
        let bits = self.get(value).bits();
        if let Some((_, offset)) = Value::from_bits(bits).reference_parts()
            && self.kept_cells.insert(offset)
        {
            self.kept.push(bits);
        }
    }

    /// ClearKeptObjects (Abschnitt 9.10).
    pub(crate) fn clear_kept_objects(&mut self) {
        self.kept.clear();
        self.kept_cells.clear();
    }

    /// Nimmt die nächste FinalizationRegistry aus der Liste, deren Ziele das
    /// Sammeln geleert hat, in ein Handle des Bereichs `scope`. Wurde eine
    /// Registry mehrmals eingetragen, kommt sie mehrmals.
    pub(crate) fn pop_cleanup<'s>(&mut self, scope: &'s Scope) -> Option<Local<'s>> {
        let bits = self.cleanup.pop_front()?;
        Some(self.local(scope, Value::from_bits(bits)))
    }

    /// Ob eine FinalizationRegistry auf ihren Aufräum-Job wartet.
    pub(crate) fn has_pending_cleanup(&self) -> bool {
        !self.cleanup.is_empty()
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

    /// Speichert einen Wert in eine bestehende Zelle, mit der
    /// Schreibbarriere. Jedes solche Speichern läuft hierüber oder über
    /// [`Heap::store_packed`].
    fn store(&self, offset: u32, index: usize, bits: u64) -> bool {
        if !self.write(offset, index, bits) {
            return false;
        }
        if let Some((_, target)) = Value::from_bits(bits).reference_parts() {
            self.barrier(offset, target);
        }
        true
    }

    /// Speichert zwei gepackte Verweise in eine bestehende Zelle, mit der
    /// Schreibbarriere für beide.
    fn store_packed(&self, offset: u32, index: usize, low: u32, high: u32) -> bool {
        if !self.write(offset, index, packed(low, high)) {
            return false;
        }
        for target in [low, high] {
            if target != NONE {
                self.barrier(offset, target);
            }
        }
        true
    }

    /// Die Schreibbarriere: Die Zelle bei `cell` hat eben einen Verweis auf
    /// `target` bekommen. Ist sie alt und das Ziel jung, merkt sich der Heap
    /// die Zelle bis zur nächsten Sammlung.
    fn barrier(&self, cell: u32, target: u32) {
        let young = |offset: u32| self.page(offset).is_some_and(|page| page.young);
        if young(target) && !young(cell) {
            self.remembered.borrow_mut().insert(cell);
        }
    }

    // Anfordern

    /// Nimmt `words` Wörter für eine neue, junge Zelle. Vorher wird jung
    /// gesammelt, wenn die jungen Zellen sonst [`NURSERY`] überschritten,
    /// und voll, wenn alle Zellen sonst die Schwelle überschritten; im
    /// Belastungsmodus immer, abwechselnd jung und voll. Die Schwelle liegt
    /// nie über dem Budget, also wird voll gesammelt, bevor eine
    /// Anforderung am Budget scheitert.
    fn allocate(&mut self, words: usize) -> Result<u32, OutOfMemory> {
        let bytes = words.checked_mul(8).ok_or(OutOfMemory)?;
        let total = |heap: &Heap| {
            heap.used
                .saturating_add(heap.external)
                .saturating_add(bytes)
        };
        let mut full = false;
        if self.options.stress && self.paused == 0 {
            full = self.stress_full;
            self.stress_full = !full;
            if full {
                self.collect();
            } else {
                self.collect_young();
            }
        } else if self.young_used.saturating_add(bytes) > NURSERY {
            self.collect_young();
        }
        if !full && total(self) > self.trigger {
            self.collect();
        }
        if total(self) > self.options.limit {
            return Err(OutOfMemory);
        }
        let offset = self.bump(words, true).ok_or(OutOfMemory)?;
        self.used = self.used.saturating_add(bytes);
        self.young_used = self.young_used.saturating_add(bytes);
        Ok(offset)
    }

    /// Nimmt `words` Wörter aus der letzten Seite der jungen oder alten
    /// Generation oder aus einer neuen.
    fn bump(&mut self, words: usize, young: bool) -> Option<u32> {
        if words > PAGE_WORDS {
            let index = self.new_page(words, young)?;
            self.space(young).large.push(index);
            if let Some(Some(page)) = self.pages.get_mut(index) {
                page.used = words;
            }
            return make_offset(index, 0);
        }
        let last = self.space(young).small.last().copied();
        if let Some(offset) = last.and_then(|index| self.bump_in(index, words)) {
            return Some(offset);
        }
        let index = self.new_page(PAGE_WORDS, young)?;
        self.space(young).small.push(index);
        self.bump_in(index, words)
    }

    fn space(&mut self, young: bool) -> &mut Space {
        if young {
            &mut self.young
        } else {
            &mut self.old
        }
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

    fn new_page(&mut self, words: usize, young: bool) -> Option<usize> {
        let index = match self.free_pages.pop() {
            Some(index) => index,
            None if self.pages.len() < MAX_PAGES => {
                self.pages.push(None);
                self.pages.len() - 1
            }
            None => return None,
        };
        let spare = if words == PAGE_WORDS {
            self.spare.pop()
        } else {
            None
        };
        let slot = self.pages.get_mut(index)?;
        *slot = Some(Page {
            words: spare.unwrap_or_else(|| vec![Cell::new(0); words].into_boxed_slice()),
            used: 0,
            condemned: false,
            young,
        });
        Some(index)
    }

    /// Gibt eine gesammelte Seite frei. Das Wortfeld einer kleinen Seite
    /// kommt genullt in den Vorrat, solange dort Platz ist.
    fn release_page(&mut self, index: usize) {
        let Some(slot) = self.pages.get_mut(index) else {
            return;
        };
        if let Some(page) = slot.take()
            && page.words.len() == PAGE_WORDS
            && self.spare.len() < SPARE_PAGES
        {
            for cell in page.words.iter().take(page.used) {
                cell.set(0);
            }
            self.spare.push(page.words);
        }
        self.free_pages.push(index);
    }

    // Sammeln

    /// Sammelt den ganzen Heap: Was von keinem Handle aus erreichbar ist,
    /// wird frei, alles andere verschoben und alt.
    pub fn collect(&mut self) {
        for page in self.pages.iter_mut().flatten() {
            page.condemned = true;
        }
        let young = std::mem::take(&mut self.young);
        let old = std::mem::take(&mut self.old);
        self.used = 0;
        self.young_used = 0;
        self.remembered.get_mut().clear();

        self.evacuate_roots();
        self.trace(Position::default());
        self.sweep_shared(false);
        for index in [young, old]
            .into_iter()
            .flat_map(|space| space.small.into_iter().chain(space.large))
        {
            self.release_page(index);
        }
        self.trigger = self
            .used
            .saturating_add(self.external)
            .saturating_mul(2)
            .max(MIN_TRIGGER)
            .min(self.options.limit);
        self.finish_collection();
    }

    /// Sammelt nur die jungen Zellen: Was weder von einem Handle noch von
    /// einer alten Zelle der gemerkten Menge aus erreichbar ist, wird frei,
    /// alles andere kommt in alte Seiten. Alte Zellen bleiben, wo sie sind,
    /// auch wenn sie nicht mehr erreichbar sind; sie räumt erst die nächste
    /// volle Sammlung ab.
    pub fn collect_young(&mut self) {
        let young = std::mem::take(&mut self.young);
        for &index in young.small.iter().chain(&young.large) {
            if let Some(Some(page)) = self.pages.get_mut(index) {
                page.condemned = true;
            }
        }
        self.used = self.used.saturating_sub(self.young_used);
        self.young_used = 0;
        // Der Lesezeiger beginnt hinter den alten Zellen, die es schon gibt.
        let start = Position {
            small: self.old.small.len().saturating_sub(1),
            word: self.old.small.last().map_or(0, |&index| {
                self.pages
                    .get(index)
                    .and_then(Option::as_ref)
                    .map_or(0, |page| page.used)
            }),
            large: self.old.large.len(),
        };

        self.evacuate_roots();
        for offset in std::mem::take(self.remembered.get_mut()) {
            self.scan_offset(offset);
        }
        self.trace(start);
        self.sweep_shared(true);
        for index in young.small.into_iter().chain(young.large) {
            self.release_page(index);
        }
        self.young_collections = self.young_collections.saturating_add(1);
        self.finish_collection();
    }

    /// Kopiert, worauf die Handles und die übrigen Wurzeln zeigen, und
    /// führt sie nach.
    fn evacuate_roots(&mut self) {
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
        let mut kept = std::mem::take(&mut self.kept);
        for slot in &mut kept {
            *slot = self.evacuate(*slot);
        }
        self.kept_cells = kept
            .iter()
            .filter_map(|&bits| Value::from_bits(bits).reference_parts())
            .map(|(_, offset)| offset)
            .collect();
        self.kept = kept;
        let mut cleanup = std::mem::take(&mut self.cleanup);
        for slot in &mut cleanup {
            *slot = self.evacuate(*slot);
        }
        self.cleanup = cleanup;
    }

    /// Liest ab `position` alle kopierten Zellen, bis der Lesezeiger und die
    /// bereiten Ephemeron-Einträge nichts mehr finden, und leert danach
    /// die Einträge und schwachen Felder, deren Ziel nicht erreicht wurde.
    fn trace(&mut self, mut position: Position) {
        loop {
            self.scan(&mut position);
            if self.ready.is_empty() {
                break;
            }
            self.trace_ready();
        }
        self.clear_ephemerons();
        self.clear_weak();
    }

    fn finish_collection(&mut self) {
        self.collections = self.collections.saturating_add(1);
        if self.options.verify
            && let Err(error) = self.verify()
        {
            panic!("Prüfmodus des Heaps: {error}");
        }
    }

    /// Der Lesezeiger nach Cheney über die alten Seiten. Kleine Seiten
    /// werden der Reihe nach gelesen; kopierte Zellen landen immer in der
    /// letzten. Große Seiten haben je eine Zelle und eine eigene Liste. Der
    /// Lesezeiger steht in `position` und liest beim nächsten Aufruf dort
    /// weiter.
    fn scan(&mut self, position: &mut Position) {
        let Position { small, word, large } = position;
        loop {
            let mut progress = false;
            while let Some(&index) = self.old.small.get(*small) {
                let used = self
                    .pages
                    .get(index)
                    .and_then(Option::as_ref)
                    .map_or(0, |page| page.used);
                if *word < used {
                    *word = self
                        .scan_cell(index, *word)
                        .map_or(used, |words| word.saturating_add(words));
                    progress = true;
                } else if *small + 1 < self.old.small.len() {
                    *small += 1;
                    *word = 0;
                } else {
                    break;
                }
            }
            while let Some(&index) = self.old.large.get(*large) {
                self.scan_cell(index, 0);
                *large += 1;
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
        self.scan_offset(make_offset(index, word)?)
    }

    /// Führt die Verweise der Zelle bei `offset` nach und gibt ihre Größe in
    /// Wörtern zurück.
    fn scan_offset(&mut self, offset: u32) -> Option<usize> {
        let head = self.word(offset, 0)?;
        let words = cell_words(head)?;
        match header_kind(head) {
            KIND_WEAK => self.weak.push(offset),
            KIND_EPHEMERON => self.index_ephemerons(offset, head),
            KIND_SHARED => self.reach_shared(offset),
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

    /// Nimmt die Einträge der Ephemeron-Zelle bei `offset` auf, die der
    /// Lesezeiger eben erreicht hat: Ein Eintrag, dessen Schlüssel schon
    /// kopiert oder kein Verweis in den alten Raum ist, ist bereit, die
    /// übrigen warten auf ihren Schlüssel.
    fn index_ephemerons(&mut self, offset: u32, head: u64) {
        self.ephemerons.push(offset);
        let width = ephemeron_width(head);
        let records = to_usize(header_length(head)) / width;
        for record in 0..records {
            let Some(key) = self.word(offset, 1 + record * width) else {
                continue;
            };
            // Die Länge einer Zelle passt in 32 Bit, also auch die Nummer.
            let entry = (offset, u32::try_from(record).unwrap_or(u32::MAX));
            match self.waiting_key(key) {
                Some(old) => self.waiting.entry(old).or_default().push(entry),
                None => self.ready.push(entry),
            }
        }
    }

    /// Der alte Ort eines Schlüssels, der noch nicht kopiert ist; `None`,
    /// wenn er es ist oder nicht im Raum liegt, der gesammelt wird.
    fn waiting_key(&self, bits: u64) -> Option<u32> {
        let (_, offset) = Value::from_bits(bits).reference_parts()?;
        if !self.condemned(offset) {
            return None;
        }
        let head = self.word(offset, 0)?;
        (header_kind(head) != KIND_FORWARD).then_some(offset)
    }

    /// Führt die bereiten Einträge nach, Schlüssel und übrige Felder. Was
    /// dabei kopiert wird, kann weitere Einträge bereit machen.
    fn trace_ready(&mut self) {
        while let Some((offset, record)) = self.ready.pop() {
            let Some(head) = self.word(offset, 0) else {
                continue;
            };
            let width = ephemeron_width(head);
            let base = to_usize(record).saturating_mul(width).saturating_add(1);
            self.trace_values(offset, base, base.saturating_add(width));
        }
    }

    /// Leert nach dem Lesezeiger die Einträge, deren Schlüssel nicht kopiert
    /// wurde: Der Schlüssel wird zur Lücke, Verweise in den übrigen Feldern
    /// werden *undefined*. Die anderen hat [`Heap::trace_ready`] schon
    /// nachgeführt.
    fn clear_ephemerons(&mut self) {
        for offset in std::mem::take(&mut self.ephemerons) {
            let Some(head) = self.word(offset, 0) else {
                continue;
            };
            let width = ephemeron_width(head);
            let records = to_usize(header_length(head)) / width;
            for record in 0..records {
                let base = 1 + record * width;
                let dead = self
                    .word(offset, base)
                    .and_then(|key| Value::from_bits(key).reference_parts())
                    .is_some_and(|(_, key)| self.condemned(key));
                if !dead {
                    continue;
                }
                self.write(offset, base, Value::HOLE.bits());
                for slot in base + 1..base + width {
                    let reference = self
                        .word(offset, slot)
                        .is_some_and(|bits| Value::from_bits(bits).reference_parts().is_some());
                    if reference {
                        self.write(offset, slot, Value::UNDEFINED.bits());
                    }
                }
            }
        }
        self.waiting.clear();
        self.ready.clear();
    }

    /// Führt nach dem Lesezeiger die Felder der schwachen Zellen nach: Ein
    /// Ziel, das verschoben wurde, bekommt seinen neuen Ort, eines, das im
    /// gesammelten Raum blieb, wird *undefined*. Leert das ein Feld hinter dem ersten einer
    /// Zelle mit [`FINALIZER_FLAG`], kommt die Registry im ersten Feld in die
    /// Liste der Aufräum-Jobs.
    fn clear_weak(&mut self) {
        for offset in std::mem::take(&mut self.weak) {
            let Some(head) = self.word(offset, 0) else {
                continue;
            };
            let count = to_usize(header_length(head));
            let mut emptied = false;
            for slot in 1..=count {
                let Some(bits) = self.word(offset, slot) else {
                    continue;
                };
                let Some((kind, target)) = Value::from_bits(bits).reference_parts() else {
                    continue;
                };
                if !self.condemned(target) {
                    continue;
                }
                let moved = match self.word(target, 0) {
                    Some(head) if header_kind(head) == KIND_FORWARD => {
                        Value::reference(kind, header_length(head)).bits()
                    }
                    _ => {
                        emptied |= slot > 1;
                        Value::UNDEFINED.bits()
                    }
                };
                self.write(offset, slot, moved);
            }
            if emptied
                && header_flags(head) & FINALIZER_FLAG != 0
                && let Some(registry) = self.word(offset, 1)
                && Value::from_bits(registry).reference_parts().is_some()
            {
                self.cleanup.push_back(registry);
            }
        }
    }

    /// Ob die Zelle bei `offset` im Raum liegt, der gerade gesammelt wird.
    fn condemned(&self, offset: u32) -> bool {
        self.page(offset).is_some_and(|page| page.condemned)
    }

    /// Kopiert die Zelle, auf die `bits` zeigt, in eine alte Seite, wenn sie
    /// im gesammelten Raum liegt und das noch nicht geschehen ist, und gibt
    /// den nachgeführten Wert zurück.
    fn evacuate(&mut self, bits: u64) -> u64 {
        let Some((kind, offset)) = Value::from_bits(bits).reference_parts() else {
            return bits;
        };
        if !self.condemned(offset) {
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
        // Die alten Seiten finden immer Platz: Das Budget ist höchstens ein
        // Viertel dessen, was die Abstände fassen.
        let Some(moved) = self.bump(words, false) else {
            return bits;
        };
        for index in 0..words {
            if let Some(word) = self.word(offset, index) {
                self.write(moved, index, word);
            }
        }
        self.write(offset, 0, header(KIND_FORWARD, moved));
        self.used = self.used.saturating_add(words.saturating_mul(8));
        if !self.waiting.is_empty()
            && let Some(entries) = self.waiting.remove(&offset)
        {
            self.ready.extend(entries);
        }
        Value::reference(kind, moved).bits()
    }

    /// Prüft den Heap: Jede Seite gehört zu genau einer Generation und
    /// besteht lückenlos aus gültigen Zellen, die Zähler der Bytes stimmen,
    /// jeder Verweis in Handles, Feldern und Objekten zeigt auf den Anfang
    /// einer passenden Zelle, und jede alte Zelle mit einem Verweis auf eine
    /// junge steht in der gemerkten Menge.
    pub fn verify(&self) -> Result<(), String> {
        self.verify_spaces()?;
        // Ein Bit je Wort jeder Seite: Beginnt dort eine Zelle? Abstände
        // nummerieren die Wörter einer Seite mit 15 Bit, auch in großen
        // Seiten, deren einzige Zelle bei Wort 0 liegt.
        const STRIDE: usize = PAGE_WORDS / 64;
        let mut starts = vec![0_u64; self.pages.len().saturating_mul(STRIDE)];
        let mut cells = Vec::new();
        for (index, page) in self.pages.iter().enumerate() {
            let Some(page) = page else {
                continue;
            };
            if page.condemned {
                return Err(format!("Seite {index} gehört noch zum gesammelten Raum"));
            }
            let mut word = 0;
            while word < page.used {
                let offset = make_offset(index, word)
                    .ok_or_else(|| format!("Seite {index}: Zelle bei Wort {word} ohne Abstand"))?;
                let head = page.words.get(word).map_or(0, Cell::get);
                let words = cell_words(head)
                    .ok_or_else(|| format!("ungültiges Kopfwort {head:#x} bei {offset:#x}"))?;
                if let Some(bits) = starts.get_mut(index.saturating_mul(STRIDE) + word / 64) {
                    *bits |= 1 << (word % 64);
                }
                cells.push((offset, header_kind(head)));
                word = word.saturating_add(words);
            }
            if word != page.used {
                return Err(format!("Seite {index}: Zellen überschreiten die Belegung"));
            }
        }
        let kind_at = |offset: u32| -> Option<u64> {
            let page = to_usize(offset >> PAGE_SHIFT);
            let word = to_usize((offset & IN_PAGE_MASK) >> 3);
            let bits = starts.get(page.checked_mul(STRIDE)?.checked_add(word / 64)?)?;
            if bits >> (word % 64) & 1 == 0 {
                return None;
            }
            self.word(offset, 0).map(header_kind)
        };
        let fits = |kind: RefKind, cell: u64| match kind {
            RefKind::String => matches!(cell, KIND_LATIN1 | KIND_UTF16),
            RefKind::Symbol => cell == KIND_SYMBOL,
            RefKind::Object => is_object(cell),
            RefKind::Internal => matches!(
                cell,
                KIND_SLOTS
                    | KIND_SHAPE
                    | KIND_WEAK
                    | KIND_SPARSE
                    | KIND_BYTES
                    | KIND_EPHEMERON
                    | KIND_SHARED
            ),
            RefKind::BigInt => cell == KIND_BIGINT,
        };
        let check = |bits: u64, place: &str| -> Result<(), String> {
            let Some((kind, offset)) = Value::from_bits(bits).reference_parts() else {
                return Ok(());
            };
            if kind_at(offset).is_some_and(|cell| fits(kind, cell)) {
                return Ok(());
            }
            Err(format!(
                "{place}: Verweis {offset:#x} ({kind:?}) zeigt auf keine passende Zelle"
            ))
        };
        let check_packed = |offset: u32, allowed: &[u64], place: &str| -> Result<(), String> {
            if offset == NONE || kind_at(offset).is_some_and(|cell| allowed.contains(&cell)) {
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
        for &bits in &self.kept {
            check(bits, "[[KeptAlive]]")?;
        }
        for &bits in &self.cleanup {
            check(bits, "Aufräum-Job")?;
        }
        let young = |offset: u32| self.page(offset).is_some_and(|page| page.young);
        let remembered = self.remembered.borrow();
        // Ein Verweis aus einer alten in eine junge Zelle braucht die Zelle
        // in der gemerkten Menge.
        let barrier = |cell: u32, target: Option<u32>, place: &str| -> Result<(), String> {
            match target {
                Some(target) if young(target) && !young(cell) && !remembered.contains(&cell) => {
                    Err(format!(
                        "{place}: alte Zelle {cell:#x} verweist auf junge {target:#x}, steht aber nicht in der gemerkten Menge"
                    ))
                }
                _ => Ok(()),
            }
        };
        let reference = |bits: u64| {
            Value::from_bits(bits)
                .reference_parts()
                .map(|(_, offset)| offset)
        };
        let reference_packed = |offset: u32| (offset != NONE).then_some(offset);
        for &cell in remembered.iter() {
            if young(cell)
                || !kind_at(cell).is_some_and(|kind| is_value_cell(kind) || is_object(kind))
            {
                return Err(format!(
                    "gemerkte Menge: {cell:#x} ist keine alte Zelle mit Verweisen"
                ));
            }
        }
        for &(offset, kind) in &cells {
            let count = self
                .word(offset, 0)
                .map_or(0, |head| to_usize(header_length(head)));
            if is_value_cell(kind) {
                for slot in 1..=count {
                    let bits = self.word(offset, slot).unwrap_or(0);
                    check(bits, "Feld")?;
                    barrier(offset, reference(bits), "Feld")?;
                }
            } else if kind == KIND_SHARED {
                if !self.shared_cell_is_valid(offset) {
                    return Err(format!(
                        "Zelle {offset:#x} nennt keinen geteilten Data Block"
                    ));
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
                for target in [refs, more]
                    .into_iter()
                    .flat_map(|word| [packed_low(word), packed_high(word)])
                {
                    barrier(offset, reference_packed(target), "gepackter Verweis")?;
                }
                // Interne Felder und eingebettete Plätze sind Werte.
                for slot in 0..count.saturating_add(object_fields(kind)) {
                    let bits = self.word(offset, 1 + OBJECT_REFS + slot).unwrap_or(0);
                    check(bits, "Eigenschaft")?;
                    barrier(offset, reference(bits), "Eigenschaft")?;
                }
            }
        }
        Ok(())
    }

    /// Für [`Heap::verify`]: Jede Seite steht in genau einer Liste ihrer
    /// Generation, und die Zähler der Bytes stimmen mit den Seiten überein.
    fn verify_spaces(&self) -> Result<(), String> {
        let mut listed = vec![false; self.pages.len()];
        let mut bytes = [0_usize; 2];
        for (young, space) in [(true, &self.young), (false, &self.old)] {
            for (large, list) in [(false, &space.small), (true, &space.large)] {
                for &index in list {
                    let Some(Some(page)) = self.pages.get(index) else {
                        return Err(format!(
                            "Seite {index} steht in einer Liste, gibt es aber nicht"
                        ));
                    };
                    if std::mem::replace(listed.get_mut(index).unwrap_or(&mut true), true) {
                        return Err(format!("Seite {index} steht in mehr als einer Liste"));
                    }
                    if page.young != young || (page.words.len() > PAGE_WORDS) != large {
                        return Err(format!("Seite {index} steht in der falschen Liste"));
                    }
                    let slot = usize::from(young);
                    if let Some(sum) = bytes.get_mut(slot) {
                        *sum = sum.saturating_add(page.used.saturating_mul(8));
                    }
                }
            }
        }
        for (index, page) in self.pages.iter().enumerate() {
            if page.is_some() && !listed.get(index).copied().unwrap_or(false) {
                return Err(format!("Seite {index} steht in keiner Liste"));
            }
        }
        let [old, young] = bytes;
        if young != self.young_used || old.saturating_add(young) != self.used {
            return Err(format!(
                "Bytes: {young} jung und {old} alt in den Seiten, gezählt {} jung und {} insgesamt",
                self.young_used, self.used
            ));
        }
        Ok(())
    }
}

mod bytes;
mod cells;
mod shared;

pub(crate) use cells::{CellKind, ObjectClass};

#[cfg(test)]
mod tests;
