//! Binärdaten in der Byte-Reihenfolge der Schriftdateien (*big endian*),
//! jeder Zugriff mit Prüfung der Grenzen.

use super::FontError;

/// Liest der Reihe nach ab einer Stelle.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    /// Beginnt bei `position`, die höchstens am Ende liegen darf.
    pub(crate) fn at(data: &'a [u8], position: usize) -> Result<Reader<'a>, FontError> {
        if position > data.len() {
            return Err(FontError::Truncated);
        }
        Ok(Reader { data, position })
    }

    pub(crate) fn skip(&mut self, count: usize) -> Result<(), FontError> {
        self.bytes(count).map(drop)
    }

    pub(crate) fn bytes(&mut self, count: usize) -> Result<&'a [u8], FontError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(FontError::Truncated)?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or(FontError::Truncated)?;
        self.position = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], FontError> {
        let bytes = self.bytes(N)?;
        <[u8; N]>::try_from(bytes).map_err(|_| FontError::Truncated)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, FontError> {
        self.array::<1>().map(|[byte]| byte)
    }

    pub(crate) fn i8(&mut self) -> Result<i8, FontError> {
        self.array::<1>().map(i8::from_be_bytes)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, FontError> {
        self.array::<2>().map(u16::from_be_bytes)
    }

    pub(crate) fn i16(&mut self) -> Result<i16, FontError> {
        self.array::<2>().map(i16::from_be_bytes)
    }

    pub(crate) fn u32(&mut self) -> Result<u32, FontError> {
        self.array::<4>().map(u32::from_be_bytes)
    }

    pub(crate) fn i32(&mut self) -> Result<i32, FontError> {
        self.array::<4>().map(i32::from_be_bytes)
    }

    /// `F2DOT14`: Festkomma mit 14 Bits hinter dem Komma.
    pub(crate) fn f2dot14(&mut self) -> Result<f32, FontError> {
        self.i16().map(|value| f32::from(value) / 16384.0)
    }
}

/// Ein `uint16` an einer Stelle.
pub(crate) fn u16_at(data: &[u8], position: usize) -> Result<u16, FontError> {
    Reader::at(data, position)?.u16()
}

/// Ein `uint32` an einer Stelle.
pub(crate) fn u32_at(data: &[u8], position: usize) -> Result<u32, FontError> {
    Reader::at(data, position)?.u32()
}

/// `base + index * size`, auf Überlauf geprüft.
pub(crate) fn offset(base: usize, index: usize, size: usize) -> Result<usize, FontError> {
    index
        .checked_mul(size)
        .and_then(|step| base.checked_add(step))
        .ok_or(FontError::Truncated)
}
