//! Zellen für geteilte Data Blocks: ein Kopfwort und ein Wort mit der
//! Nummer eines Eintrags in der Tabelle [`Heap::shared`], der den
//! [`SharedBlock`] hält. Die Zelle enthält keine Verweise. Beim Sammeln
//! merkt sich der Lesezeiger die Nummern der Zellen, die er erreicht;
//! Einträge ohne erreichte Zelle gibt der Heap danach frei, und der Block
//! lebt nur weiter, solange ein anderer Agent ihn hält. Eine junge Sammlung
//! erreicht nur junge Zellen und gibt deshalb nur Einträge junger Zellen
//! frei; ein Eintrag wird alt, sobald seine Zelle eine Sammlung übersteht.
//!
//! Die Bytes eines Blocks zählen zum Speicherbudget des Heaps, der ihn
//! hält, mit [`SharedBlock::capacity`] Bytes.

use std::sync::Arc;

use super::{Heap, KIND_SHARED, Local, OutOfMemory, Scope, header, header_kind};
use crate::shared::SharedBlock;
use crate::value::{Ref, RefKind, Value};

/// Ein Eintrag der Tabelle: der Block, ob der Lesezeiger der laufenden
/// Sammlung eine Zelle dafür erreicht hat, und ob die Zelle noch jung ist.
pub(super) struct Entry {
    block: Arc<SharedBlock>,
    reached: bool,
    young: bool,
}

impl Heap {
    /// Legt einen geteilten Data Block aus `length` Bytes an, alle 0, der
    /// mit `max` bis zu `max` Bytes wächst (CreateSharedByteDataBlock,
    /// Abschnitt 6.2.9.2), und eine Zelle dafür. [`OutOfMemory`], wenn er
    /// nicht in den freien Teil des Speicherbudgets passt, auch nach einer
    /// Sammlung. Der Verweis hat die Art Internal.
    pub(crate) fn alloc_shared<'s>(
        &mut self,
        scope: &'s Scope,
        length: usize,
        max: Option<usize>,
    ) -> Result<Local<'s>, OutOfMemory> {
        let capacity = max.unwrap_or(length);
        self.reserve_external(capacity)?;
        let block = SharedBlock::new(length, max).ok_or(OutOfMemory)?;
        self.adopt_shared(scope, Arc::new(block))
    }

    /// Eine Zelle für einen Block, den es schon gibt, etwa aus einem
    /// anderen Agent. Er zählt danach auch zum Budget dieses Heaps.
    pub(crate) fn adopt_shared<'s>(
        &mut self,
        scope: &'s Scope,
        block: Arc<SharedBlock>,
    ) -> Result<Local<'s>, OutOfMemory> {
        let capacity = block.capacity();
        self.reserve_external(capacity)?;
        let index = match self.free_shared.pop() {
            Some(index) => index,
            None => {
                self.shared.push(None);
                self.shared.len() - 1
            }
        };
        if let Some(slot) = self.shared.get_mut(index) {
            *slot = Some(Entry {
                block,
                reached: false,
                young: true,
            });
        }
        self.external = self.external.saturating_add(capacity);
        // Die Zelle kann eine Sammlung auslösen. Der Eintrag ist dann noch
        // von keiner Zelle erreicht; damit er sie übersteht, zählt er bis
        // zum Ende dieser Funktion als erreicht.
        self.pinned_shared = Some(index);
        let cell = self.allocate(2);
        self.pinned_shared = None;
        let offset = match cell {
            Ok(offset) => offset,
            Err(error) => {
                self.release_shared(index);
                return Err(error);
            }
        };
        self.init(offset, 0, header(KIND_SHARED, 1));
        // Die Nummer passt in ein Wort.
        self.init(offset, 1, u64::try_from(index).unwrap_or(u64::MAX));
        Ok(self.local(scope, Value::reference(RefKind::Internal, offset)))
    }

    /// Ob `bytes` Bytes außerhalb der Zellen noch in das Budget passen;
    /// sammelt vorher, wenn die Schwelle dafür erreicht ist.
    fn reserve_external(&mut self, bytes: usize) -> Result<(), OutOfMemory> {
        let total = |heap: &Heap| {
            heap.used
                .saturating_add(heap.external)
                .saturating_add(bytes)
        };
        if total(self) > self.trigger {
            self.collect();
        }
        if total(self) > self.options.limit {
            return Err(OutOfMemory);
        }
        Ok(())
    }

    /// Gibt den Eintrag `index` frei.
    fn release_shared(&mut self, index: usize) {
        if let Some(entry) = self.shared.get_mut(index).and_then(Option::take) {
            self.external = self.external.saturating_sub(entry.block.capacity());
            self.free_shared.push(index);
        }
    }

    /// Der Block hinter einer Zelle für geteilte Data Blocks, `None` für
    /// andere Zellen.
    pub(crate) fn shared_block(&self, cell: Ref<'_>) -> Option<&Arc<SharedBlock>> {
        let index = self.shared_index(cell.offset())?;
        self.shared.get(index)?.as_ref().map(|entry| &entry.block)
    }

    /// Die Nummer des Eintrags der Zelle bei `offset`.
    pub(super) fn shared_index(&self, offset: u32) -> Option<usize> {
        let head = self.word(offset, 0)?;
        if header_kind(head) != KIND_SHARED {
            return None;
        }
        usize::try_from(self.word(offset, 1)?).ok()
    }

    /// Bytes in geteilten Blöcken, die dieser Heap hält.
    pub fn external_bytes(&self) -> usize {
        self.external
    }

    /// Der Lesezeiger hat die Zelle bei `offset` erreicht.
    pub(super) fn reach_shared(&mut self, offset: u32) {
        if let Some(index) = self.shared_index(offset)
            && let Some(Some(entry)) = self.shared.get_mut(index)
        {
            entry.reached = true;
            // Die Zelle liegt jetzt in einer alten Seite.
            entry.young = false;
        }
    }

    /// Nach dem Lesezeiger: Einträge, deren Zelle nicht erreicht wurde,
    /// werden frei, die übrigen warten auf die nächste Sammlung. Mit
    /// `young_only` (junge Sammlung) bleiben alte Einträge stehen: Ihre
    /// Zellen sucht eine junge Sammlung nicht.
    pub(super) fn sweep_shared(&mut self, young_only: bool) {
        for index in 0..self.shared.len() {
            let keep = self.pinned_shared == Some(index);
            let reached = match self.shared.get_mut(index) {
                Some(Some(entry)) => {
                    std::mem::replace(&mut entry.reached, false) || (young_only && !entry.young)
                }
                _ => continue,
            };
            if !reached && !keep {
                self.release_shared(index);
            }
        }
    }

    /// Ob die Zelle bei `offset` einen Eintrag hat; für `verify`.
    pub(super) fn shared_cell_is_valid(&self, offset: u32) -> bool {
        self.shared_index(offset)
            .and_then(|index| self.shared.get(index))
            .is_some_and(Option::is_some)
    }
}
