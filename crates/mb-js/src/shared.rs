//! Geteilte Data Blocks (Shared Data Block, Abschnitt 6.2.9) der
//! SharedArrayBuffer und ihre Warteschlangen für `Atomics.wait` und
//! `Atomics.notify` (WaiterList, Abschnitt 25.4.3).
//!
//! Die Bytes eines geteilten Blocks liegen nicht in Zellen des Heaps,
//! sondern in einem [`SharedBlock`] dahinter, den ein [`Arc`] hält. Worker
//! sind eigene Agents mit eigenem Heap (`docs/architektur.md`, Abschnitt
//! 4); damit mehrere von ihnen denselben Block gleichzeitig lesen und
//! schreiben können, darf er keinem Heap gehören. Der Heap eines Agents
//! hält für jeden Block, den er sieht, einen Eintrag in einer Tabelle
//! (siehe `heap::shared`).
//!
//! Festlegungen:
//! - Die Bytes stehen in Wörtern aus [`AtomicU64`], acht je Wort, das erste
//!   im niedrigsten, wie in den Zellen aus Bytes. Ein Element von 1, 2, 4
//!   oder 8 Bytes, das an seiner Größe ausgerichtet ist, liegt so immer in
//!   einem Wort. Atomare Zugriffe (~seq-cst~) gehen auf dieses Wort mit
//!   [`Ordering::SeqCst`]; ist das Element kleiner als das Wort, ersetzt
//!   eine Schleife mit `compare_exchange` nur seine Bytes. Alle Größen sind
//!   damit frei von Sperren, auch 1, 2 und 8 Bytes ([[IsLockFree1]],
//!   [[IsLockFree2]] und [[IsLockFree8]] sind *true*).
//! - Nicht atomare Zugriffe (~unordered~) lesen und schreiben die Wörter mit
//!   [`Ordering::Relaxed`]. Ein Schreiben, das nur einen Teil eines Worts
//!   trifft, ersetzt ebenfalls nur seine Bytes, so geht kein Schreiben eines
//!   anderen Agents auf die übrigen Bytes verloren.
//! - Ein wachsender Block (growable SharedArrayBuffer) belegt von Anfang an
//!   [[ArrayBufferMaxByteLength]] Bytes, wie es AllocateSharedArrayBuffer
//!   (Abschnitt 25.2.2.1) beschreibt; `grow` setzt nur die Länge. Die Länge
//!   steht für [[ArrayBufferByteLengthData]] in einem eigenen Wort.
//! - Die kritische Sektion einer WaiterList (EnterCriticalSection) ist eine
//!   Sperre je Block für alle seine Listen. Ein Agent, der in `Atomics.wait`
//!   wartet, schläft an einer eigenen [`Condvar`] mit dieser Sperre.

#![forbid(unsafe_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// So viele Bytes kopiert [`SharedBlock::copy_to`] auf einmal durch einen
/// Zwischenpuffer.
const CHUNK: usize = 4096;

/// Ein geteilter Data Block.
pub(crate) struct SharedBlock {
    words: Box<[AtomicU64]>,
    /// Die Länge in Bytes, für einen wachsenden Block
    /// [[ArrayBufferByteLengthData]].
    length: AtomicUsize,
    /// Wie viele Bytes der Block fasst: die Länge, bei einem wachsenden
    /// Block [[ArrayBufferMaxByteLength]].
    capacity: usize,
    growable: bool,
    /// Die WaiterLists nach Index im Block, hinter der Sperre der
    /// kritischen Sektion.
    waiters: Mutex<WaiterLists>,
}

impl SharedBlock {
    /// Ein Block aus `length` Bytes, alle 0 (CreateSharedByteDataBlock,
    /// Abschnitt 6.2.9.2). Mit `max` wächst er bis zu `max` Bytes, die er
    /// gleich belegt. `None`, wenn `length` größer als `max` ist.
    pub(crate) fn new(length: usize, max: Option<usize>) -> Option<SharedBlock> {
        let capacity = max.unwrap_or(length);
        if length > capacity {
            return None;
        }
        let words = std::iter::repeat_with(|| AtomicU64::new(0))
            .take(capacity.div_ceil(8))
            .collect();
        Some(SharedBlock {
            words,
            length: AtomicUsize::new(length),
            capacity,
            growable: max.is_some(),
            waiters: Mutex::new(WaiterLists::default()),
        })
    }

    /// Die Länge in Bytes (ArrayBufferByteLength mit ~seq-cst~).
    pub(crate) fn len(&self) -> usize {
        self.length.load(Ordering::SeqCst)
    }

    /// Wie viele Bytes der Block fasst.
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// Setzt die Länge auf `new_length` (die Schleife aus
    /// SharedArrayBuffer.prototype.grow, Abschnitt 25.2.5.3, Schritte 9 bis
    /// 11). `false`, wenn die Länge kleiner wäre als die aktuelle oder größer
    /// als das Maximum; ist sie gleich, gelingt das immer.
    pub(crate) fn grow(&self, new_length: usize) -> bool {
        let mut current = self.len();
        loop {
            if new_length == current {
                return true;
            }
            if !self.growable || new_length < current || new_length > self.capacity {
                return false;
            }
            match self.length.compare_exchange(
                current,
                new_length,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(read) => current = read,
            }
        }
    }

    /// Ob `count` Bytes ab `start` in der aktuellen Länge liegen.
    fn in_range(&self, start: usize, count: usize) -> bool {
        start
            .checked_add(count)
            .is_some_and(|end| end <= self.len())
    }

    /// Liest `out.len()` Bytes ab `start`, nicht atomar (~unordered~).
    /// `false` und nichts gelesen, wenn sie nicht alle im Block liegen.
    pub(crate) fn read(&self, start: usize, out: &mut [u8]) -> bool {
        if !self.in_range(start, out.len()) {
            return false;
        }
        let mut done = 0;
        while done < out.len() {
            let position = start + done;
            let within = position % 8;
            let count = (8 - within).min(out.len() - done);
            let Some(word) = self.words.get(position / 8) else {
                return false;
            };
            let bytes = word.load(Ordering::Relaxed).to_le_bytes();
            let (Some(target), Some(source)) = (
                out.get_mut(done..done + count),
                bytes.get(within..within + count),
            ) else {
                return false;
            };
            target.copy_from_slice(source);
            done += count;
        }
        true
    }

    /// Schreibt `data` ab `start`, nicht atomar (~unordered~). `false` und
    /// nichts geschrieben, wenn die Bytes nicht alle im Block liegen.
    pub(crate) fn write(&self, start: usize, data: &[u8]) -> bool {
        if !self.in_range(start, data.len()) {
            return false;
        }
        let mut done = 0;
        while done < data.len() {
            let position = start + done;
            let within = position % 8;
            let count = (8 - within).min(data.len() - done);
            let (Some(word), Some(source)) =
                (self.words.get(position / 8), data.get(done..done + count))
            else {
                return false;
            };
            let mut bytes = [0u8; 8];
            if let Some(target) = bytes.get_mut(within..within + count) {
                target.copy_from_slice(source);
            }
            let shift = within * 8;
            replace_bits(
                word,
                lane_mask(count) << shift,
                u64::from_le_bytes(bytes),
                Ordering::Relaxed,
            );
            done += count;
        }
        true
    }

    /// CopyDataBlockBytes (Abschnitt 6.2.9.3) nach `to` ab `to_start` aus
    /// diesem Block ab `from_start`, nicht atomar. Ist `to` derselbe Block
    /// und überlappen sich die Bereiche, ist das Ergebnis, als wären die
    /// Bytes erst vollständig gelesen worden. `false` und nichts kopiert,
    /// wenn ein Bereich nicht in seinem Block liegt.
    pub(crate) fn copy_to(
        &self,
        to: &SharedBlock,
        to_start: usize,
        from_start: usize,
        count: usize,
    ) -> bool {
        if !to.in_range(to_start, count) || !self.in_range(from_start, count) {
            return false;
        }
        let backward = std::ptr::eq(self, to) && to_start > from_start;
        let mut buffer = vec![0u8; count.min(CHUNK)];
        let mut done = 0;
        while done < count {
            let size = buffer.len().min(count - done);
            let at = if backward { count - done - size } else { done };
            let Some(chunk) = buffer.get_mut(..size) else {
                return false;
            };
            if !self.read(from_start + at, chunk) || !to.write(to_start + at, chunk) {
                return false;
            }
            done += size;
        }
        true
    }

    /// Das Wort und die Lage eines Elements von `size` Bytes bei `index`:
    /// `None`, wenn es nicht im Block liegt, nicht an seiner Größe
    /// ausgerichtet ist oder `size` nicht 1, 2, 4 oder 8 ist.
    fn lane(&self, index: usize, size: usize) -> Option<(&AtomicU64, u32, u64)> {
        if !matches!(size, 1 | 2 | 4 | 8)
            || !index.is_multiple_of(size)
            || !self.in_range(index, size)
        {
            return None;
        }
        let word = self.words.get(index / 8)?;
        let shift = u32::try_from((index % 8) * 8).ok()?;
        Some((word, shift, lane_mask(size)))
    }

    /// Liest das Element von `size` Bytes bei `index` atomar (~seq-cst~),
    /// die Bytes als Zahl, das erste im niedrigsten Byte.
    pub(crate) fn load(&self, index: usize, size: usize) -> Option<u64> {
        let (word, shift, mask) = self.lane(index, size)?;
        Some((word.load(Ordering::SeqCst) >> shift) & mask)
    }

    /// Liest das Element von `size` Bytes bei `index` und ersetzt es durch
    /// `operation(alter Wert)`, wenn das `Some` ist, in einem atomaren
    /// Schritt. Das ist das Read-Modify-Write-Ereignis aus
    /// GetModifySetValueInBuffer (Abschnitt 25.1.3.19) und, mit einer
    /// Operation, die nur bei Gleichheit ersetzt,
    /// AtomicCompareExchangeInSharedBlock (Abschnitt 25.4.3.18). Das
    /// Ergebnis ist der alte Wert; das von `operation` wird auf `size`
    /// Bytes gekürzt.
    pub(crate) fn update(
        &self,
        index: usize,
        size: usize,
        operation: impl Fn(u64) -> Option<u64>,
    ) -> Option<u64> {
        let (word, shift, mask) = self.lane(index, size)?;
        let read = match word.try_update(Ordering::SeqCst, Ordering::SeqCst, |bits| {
            let new = operation((bits >> shift) & mask)? & mask;
            Some((bits & !(mask << shift)) | (new << shift))
        }) {
            Ok(bits) | Err(bits) => bits,
        };
        Some((read >> shift) & mask)
    }

    /// EnterCriticalSection (Abschnitt 25.4.3.5) für alle WaiterLists des
    /// Blocks. Die Sperre ist vergiftet, wenn ein Faden sie bei einer Panik
    /// hielt; die Listen ändern sich nur in Schritten, die nicht abbrechen,
    /// also gelten sie auch dann noch, und der Wächter wird übernommen
    /// (RS-15).
    pub(crate) fn critical_section(&self) -> MutexGuard<'_, WaiterLists> {
        self.waiters.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Eine Maske für die unteren `count` Bytes eines Worts.
fn lane_mask(count: usize) -> u64 {
    match count {
        8.. => u64::MAX,
        _ => (1u64 << (count * 8)) - 1,
    }
}

/// Ersetzt in `word` die Bits unter `mask` durch die von `bits`.
fn replace_bits(word: &AtomicU64, mask: u64, bits: u64, order: Ordering) {
    if mask == u64::MAX {
        word.store(bits, order);
        return;
    }
    let load = match order {
        Ordering::SeqCst => Ordering::SeqCst,
        _ => Ordering::Relaxed,
    };
    // `try_update` gibt bei einer Schließung, die immer `Some` liefert,
    // nie `Err` zurück.
    let _ = word.try_update(order, load, |old| Some((old & !mask) | (bits & mask)));
}

/// Die Nachricht an einen Agent, dass Warter von `Atomics.waitAsync` aus
/// einem anderen Agent geweckt wurden: die Nummern ihrer Einträge in seiner
/// Tabelle. Der Agent liest sie, wenn er seine Jobs laufen lässt
/// (EnqueueResolveInAgentJob, Abschnitt 25.4.3.15, für einen anderen
/// Agent).
#[derive(Default)]
pub(crate) struct Mailbox {
    tickets: Mutex<Vec<u64>>,
}

impl Mailbox {
    pub(crate) fn push(&self, ticket: u64) {
        // Wie bei der kritischen Sektion: Die Liste ändert sich nur in
        // einem Schritt.
        self.tickets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ticket);
    }

    pub(crate) fn take(&self) -> Vec<u64> {
        std::mem::take(&mut *self.tickets.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tickets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }
}

/// Wie ein Warter geweckt wird.
pub(crate) enum Wake {
    /// Ein Agent, der in `Atomics.wait` schläft ([[PromiseCapability]] ist
    /// ~blocking~), an dieser Bedingung.
    Blocking(Arc<Condvar>),
    /// Ein Warter von `Atomics.waitAsync`: die Nummer seines Eintrags im
    /// Agent und dessen Briefkasten.
    Async { ticket: u64, mailbox: Arc<Mailbox> },
}

/// Ein Waiter Record (Abschnitt 25.4.3.2) ohne [[TimeoutTime]], die der
/// wartende Agent selbst kennt, und ohne [[AgentSignifier]]: Wie er
/// geweckt wird, nennt schon seinen Agent.
pub(crate) struct Waiter {
    /// Die Nummer des Warters im Block.
    id: u64,
    pub(crate) wake: Wake,
}

/// Die WaiterLists eines Blocks (Abschnitt 25.4.3.1), je Index im Block
/// eine Liste in der Reihenfolge, in der die Warter kamen.
#[derive(Default)]
pub(crate) struct WaiterLists {
    next: u64,
    lists: HashMap<usize, VecDeque<Waiter>>,
}

impl WaiterLists {
    /// AddWaiter (Abschnitt 25.4.3.6): hängt einen Warter an die Liste von
    /// `index` und gibt seine Nummer zurück.
    pub(crate) fn add(&mut self, index: usize, wake: Wake) -> u64 {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        self.lists
            .entry(index)
            .or_default()
            .push_back(Waiter { id, wake });
        id
    }

    /// Ob der Warter `id` noch in der Liste von `index` steht.
    pub(crate) fn contains(&self, index: usize, id: u64) -> bool {
        self.lists
            .get(&index)
            .is_some_and(|list| list.iter().any(|waiter| waiter.id == id))
    }

    /// RemoveWaiter (Abschnitt 25.4.3.7). `false`, wenn er nicht mehr in
    /// der Liste steht.
    pub(crate) fn remove(&mut self, index: usize, id: u64) -> bool {
        let Some(list) = self.lists.get_mut(&index) else {
            return false;
        };
        let before = list.len();
        list.retain(|waiter| waiter.id != id);
        let removed = list.len() != before;
        if list.is_empty() {
            self.lists.remove(&index);
        }
        removed
    }

    /// RemoveWaiters (Abschnitt 25.4.3.8): nimmt die ersten `count` Warter
    /// der Liste von `index` heraus.
    pub(crate) fn remove_first(&mut self, index: usize, count: u64) -> Vec<Waiter> {
        let Some(list) = self.lists.get_mut(&index) else {
            return Vec::new();
        };
        let take = usize::try_from(count).unwrap_or(usize::MAX).min(list.len());
        let removed: Vec<Waiter> = list.drain(..take).collect();
        if list.is_empty() {
            self.lists.remove(&index);
        }
        removed
    }
}

#[cfg(test)]
mod tests;
