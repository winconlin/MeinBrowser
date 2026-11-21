//! Zellen aus Bytes: die Data Blocks (Abschnitt 6.2.9) der ArrayBuffer.
//! Das Kopfwort hält die Länge in Bytes, danach folgen die Bytes, acht je
//! Wort, das erste im niedrigsten. Die Zelle enthält keine Verweise; beim
//! Sammeln wird sie nur kopiert.

use super::{
    Heap, KIND_BYTES, Local, OutOfMemory, Scope, cell_words, header, header_kind, header_length,
    to_usize,
};
use crate::value::{Ref, RefKind, Value};

/// So viele Bytes trägt das Kopieren auf einmal durch einen Zwischenpuffer.
const CHUNK: usize = 4096;

impl Heap {
    /// Legt einen Data Block aus `len` Bytes an, alle 0
    /// (CreateByteDataBlock, Abschnitt 6.2.9.1, Schritte 2 und 3). Der
    /// Verweis hat die Art Internal.
    pub(crate) fn alloc_bytes<'s>(
        &mut self,
        scope: &'s Scope,
        len: usize,
    ) -> Result<Local<'s>, OutOfMemory> {
        let length = u32::try_from(len).map_err(|_| OutOfMemory)?;
        let head = header(KIND_BYTES, length);
        let words = cell_words(head).ok_or(OutOfMemory)?;
        let offset = self.allocate(words)?;
        self.init(offset, 0, head);
        // Neue Seiten sind genullt, aber darauf verlässt sich das nicht.
        for index in 1..words {
            self.init(offset, index, 0);
        }
        Ok(self.local(scope, Value::reference(RefKind::Internal, offset)))
    }

    /// Ob dieser Heap überhaupt einen Data Block aus `len` Bytes fassen
    /// kann, wenn sonst nichts belegt ist.
    pub(crate) fn bytes_fit(&self, len: u64) -> bool {
        let Ok(length) = u32::try_from(len) else {
            return false;
        };
        cell_words(header(KIND_BYTES, length))
            .and_then(|words| words.checked_mul(8))
            .is_some_and(|bytes| bytes <= self.limit())
    }

    /// Die Länge in Bytes, `None` für andere Zellen.
    pub(crate) fn byte_len(&self, cell: Ref<'_>) -> Option<usize> {
        self.bytes_header(cell)
            .map(|head| to_usize(header_length(head)))
    }

    fn bytes_header(&self, cell: Ref<'_>) -> Option<u64> {
        let head = self.word(cell.offset(), 0)?;
        (header_kind(head) == KIND_BYTES).then_some(head)
    }

    /// Ob `count` Bytes ab `start` in der Zelle liegen.
    fn bytes_in_range(&self, cell: Ref<'_>, start: usize, count: usize) -> bool {
        self.byte_len(cell)
            .zip(start.checked_add(count))
            .is_some_and(|(len, end)| end <= len)
    }

    /// Liest `out.len()` Bytes ab `start`. `false` und nichts gelesen, wenn
    /// sie nicht alle in der Zelle liegen.
    pub(crate) fn read_bytes(&self, cell: Ref<'_>, start: usize, out: &mut [u8]) -> bool {
        if !self.bytes_in_range(cell, start, out.len()) {
            return false;
        }
        let mut done = 0;
        while done < out.len() {
            let position = start + done;
            let within = position % 8;
            let count = (8 - within).min(out.len() - done);
            let Some(word) = self.word(cell.offset(), 1 + position / 8) else {
                return false;
            };
            let bytes = word.to_le_bytes();
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

    /// Schreibt `data` ab `start`. `false` und nichts geschrieben, wenn die
    /// Bytes nicht alle in der Zelle liegen.
    pub(crate) fn write_bytes(&self, cell: Ref<'_>, start: usize, data: &[u8]) -> bool {
        if !self.bytes_in_range(cell, start, data.len()) {
            return false;
        }
        let mut done = 0;
        while done < data.len() {
            let position = start + done;
            let within = position % 8;
            let count = (8 - within).min(data.len() - done);
            let index = 1 + position / 8;
            let Some(word) = self.word(cell.offset(), index) else {
                return false;
            };
            let mut bytes = word.to_le_bytes();
            let (Some(target), Some(source)) = (
                bytes.get_mut(within..within + count),
                data.get(done..done + count),
            ) else {
                return false;
            };
            target.copy_from_slice(source);
            // Bytes sind keine Verweise, die Schreibbarriere braucht es
            // dafür nicht.
            self.write(cell.offset(), index, u64::from_le_bytes(bytes));
            done += count;
        }
        true
    }

    /// CopyDataBlockBytes (Abschnitt 6.2.9.3): `count` Bytes von `from` ab
    /// `from_start` nach `to` ab `to_start`. Überlappen sich die Bereiche in
    /// derselben Zelle, ist das Ergebnis, als wären die Bytes erst
    /// vollständig gelesen worden. `false` und nichts kopiert, wenn einer
    /// der Bereiche nicht in seiner Zelle liegt.
    pub(crate) fn copy_bytes(
        &self,
        to: Ref<'_>,
        to_start: usize,
        from: Ref<'_>,
        from_start: usize,
        count: usize,
    ) -> bool {
        if !self.bytes_in_range(to, to_start, count)
            || !self.bytes_in_range(from, from_start, count)
        {
            return false;
        }
        let mut buffer = vec![0u8; count.min(CHUNK)];
        // Liegt das Ziel in derselben Zelle hinter der Quelle, geht es von
        // hinten nach vorn, damit kein Stück überschrieben wird, bevor es
        // gelesen ist.
        let backward = to.offset() == from.offset() && to_start > from_start;
        let mut done = 0;
        while done < count {
            let size = buffer.len().min(count - done);
            let at = if backward { count - done - size } else { done };
            let Some(chunk) = buffer.get_mut(..size) else {
                return false;
            };
            if !self.read_bytes(from, from_start + at, chunk)
                || !self.write_bytes(to, to_start + at, chunk)
            {
                return false;
            }
            done += size;
        }
        true
    }
}
