//! Zugriff auf die Indizes (Abschnitt „Indexes“ des Encoding Standard).
//!
//! Die Tabellen gehen vom Zeiger zum Codepunkt. Für die Kodierer wird beim
//! ersten Gebrauch ein sortierter Rückwärtsindex gebaut.

use std::sync::OnceLock;

use crate::tables::{INDEX_BIG5, INDEX_EUC_KR, INDEX_GB18030, INDEX_GB18030_RANGES, INDEX_JIS0208};

/// Der *index code point* für `pointer`, oder `None`. In den Tabellen steht 0
/// für „kein Eintrag“; kein Index bildet auf U+0000 ab.
pub fn code_point<T: Copy + Into<u32>>(index: &[T], pointer: u32) -> Option<u32> {
    let value: u32 = (*index.get(usize::try_from(pointer).ok()?)?).into();
    (value != 0).then_some(value)
}

/// Codepunkt und Zeiger, nach Codepunkt sortiert, je Codepunkt ein Zeiger.
struct Reverse(Vec<(u32, u32)>);

impl Reverse {
    /// Baut den Rückwärtsindex aus allen Zeigern, für die `keep` gilt. Gibt es
    /// mehrere Zeiger für einen Codepunkt, gewinnt der erste, außer `last`
    /// verlangt für diesen Codepunkt den letzten.
    fn build<T: Copy + Into<u32>>(
        index: &[T],
        keep: impl Fn(u32) -> bool,
        last: impl Fn(u32) -> bool,
    ) -> Reverse {
        let mut entries: Vec<(u32, u32)> = (0u32..)
            .zip(index)
            .filter_map(|(pointer, &code_point)| {
                let code_point: u32 = code_point.into();
                (code_point != 0 && keep(pointer)).then_some((code_point, pointer))
            })
            .collect();
        entries.sort_unstable();
        let mut unique: Vec<(u32, u32)> = Vec::with_capacity(entries.len());
        for (code_point, pointer) in entries {
            match unique.last_mut() {
                Some(previous) if previous.0 == code_point => {
                    if last(code_point) {
                        previous.1 = pointer;
                    }
                }
                _ => unique.push((code_point, pointer)),
            }
        }
        Reverse(unique)
    }

    fn pointer(&self, code_point: u32) -> Option<u32> {
        let found = self
            .0
            .binary_search_by_key(&code_point, |&(code_point, _)| code_point)
            .ok()?;
        self.0.get(found).map(|&(_, pointer)| pointer)
    }
}

fn all(_: u32) -> bool {
    true
}

fn none(_: u32) -> bool {
    false
}

/// Der *index pointer* für `code_point` in *index jis0208*.
pub fn jis0208_pointer(code_point: u32) -> Option<u32> {
    static REVERSE: OnceLock<Reverse> = OnceLock::new();
    REVERSE
        .get_or_init(|| Reverse::build(&INDEX_JIS0208, all, none))
        .pointer(code_point)
}

/// Der *index Shift_JIS pointer*: *index jis0208* ohne die Zeiger 8272 bis
/// 8835 (NEC-Dubletten), damit die späteren IBM-Erweiterungen gelten.
pub fn shift_jis_pointer(code_point: u32) -> Option<u32> {
    static REVERSE: OnceLock<Reverse> = OnceLock::new();
    REVERSE
        .get_or_init(|| {
            Reverse::build(
                &INDEX_JIS0208,
                |pointer| !(8272..=8835).contains(&pointer),
                none,
            )
        })
        .pointer(code_point)
}

/// Der *index Big5 pointer*: ohne die Zeiger vor (0xA1 − 0x81) × 157, die die
/// Erweiterungen aus Hongkong enthalten, und für sechs Codepunkte mit dem
/// letzten statt dem ersten Zeiger.
pub fn big5_pointer(code_point: u32) -> Option<u32> {
    static REVERSE: OnceLock<Reverse> = OnceLock::new();
    REVERSE
        .get_or_init(|| {
            Reverse::build(
                &INDEX_BIG5,
                |pointer| pointer >= (0xA1 - 0x81) * 157,
                |code_point| {
                    matches!(
                        code_point,
                        0x2550 | 0x255E | 0x2561 | 0x256A | 0x5341 | 0x5345
                    )
                },
            )
        })
        .pointer(code_point)
}

/// Der *index pointer* für `code_point` in *index EUC-KR*.
pub fn euc_kr_pointer(code_point: u32) -> Option<u32> {
    static REVERSE: OnceLock<Reverse> = OnceLock::new();
    REVERSE
        .get_or_init(|| Reverse::build(&INDEX_EUC_KR, all, none))
        .pointer(code_point)
}

/// Der *index pointer* für `code_point` in *index gb18030*.
pub fn gb18030_pointer(code_point: u32) -> Option<u32> {
    static REVERSE: OnceLock<Reverse> = OnceLock::new();
    REVERSE
        .get_or_init(|| Reverse::build(&INDEX_GB18030, all, none))
        .pointer(code_point)
}

/// Der *index gb18030 ranges code point* für `pointer`.
pub fn gb18030_ranges_code_point(pointer: u32) -> Option<u32> {
    if (pointer > 39419 && pointer < 189000) || pointer > 1237575 {
        return None;
    }
    if pointer == 7457 {
        return Some(0xE7C7);
    }
    let after = INDEX_GB18030_RANGES.partition_point(|&(start, _)| start <= pointer);
    let &(offset, code_point_offset) = INDEX_GB18030_RANGES.get(after.checked_sub(1)?)?;
    code_point_offset.checked_add(pointer - offset)
}

/// Der *index gb18030 ranges pointer* für `code_point`.
pub fn gb18030_ranges_pointer(code_point: u32) -> Option<u32> {
    if code_point == 0xE7C7 {
        return Some(7457);
    }
    let after = INDEX_GB18030_RANGES.partition_point(|&(_, start)| start <= code_point);
    let &(pointer_offset, offset) = INDEX_GB18030_RANGES.get(after.checked_sub(1)?)?;
    pointer_offset.checked_add(code_point - offset)
}

/// Der *index pointer* für `code_point` in einem Einzelbyte-Index.
pub fn single_byte_pointer(index: &[u16; 128], code_point: u32) -> Option<u32> {
    (0u32..)
        .zip(index)
        .find(|&(_, &value)| value != 0 && u32::from(value) == code_point)
        .map(|(pointer, _)| pointer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_resolve_as_specified() {
        // U+2550 steht in index Big5 mehrfach; der Kodierer nimmt den letzten.
        let pointers: Vec<u32> = (0u32..)
            .zip(INDEX_BIG5.iter())
            .filter(|&(pointer, &code_point)| code_point == 0x2550 && pointer >= 5024)
            .map(|(pointer, _)| pointer)
            .collect();
        assert!(pointers.len() > 1);
        assert_eq!(big5_pointer(0x2550), pointers.last().copied());
        // Ohne Sonderregel gilt der erste Zeiger.
        let first = (0u32..)
            .zip(INDEX_BIG5.iter())
            .find(|&(pointer, &code_point)| code_point == 0x5341 && pointer >= 5024);
        assert!(first.is_some());
        assert_ne!(big5_pointer(0x5341), first.map(|(pointer, _)| pointer));
    }

    #[test]
    fn shift_jis_skips_nec_duplicates() {
        // U+2171 (ⅱ) steht bei 8635 (NEC-Auswahl der IBM-Erweiterungen) und
        // bei 10717 (IBM-Erweiterungen).
        assert_eq!(code_point(&INDEX_JIS0208, 8635), Some(0x2171));
        assert_eq!(code_point(&INDEX_JIS0208, 10717), Some(0x2171));
        assert_eq!(jis0208_pointer(0x2171), Some(8635));
        assert_eq!(shift_jis_pointer(0x2171), Some(10717));
    }

    #[test]
    fn gb18030_ranges() {
        assert_eq!(gb18030_ranges_code_point(0), Some(0x80));
        assert_eq!(gb18030_ranges_code_point(7457), Some(0xE7C7));
        assert_eq!(gb18030_ranges_code_point(39420), None);
        assert_eq!(gb18030_ranges_code_point(189000), Some(0x10000));
        assert_eq!(gb18030_ranges_code_point(1237575), Some(0x10_FFFF));
        assert_eq!(gb18030_ranges_code_point(1237576), None);
        assert_eq!(gb18030_ranges_pointer(0x80), Some(0));
        assert_eq!(gb18030_ranges_pointer(0xE7C7), Some(7457));
        assert_eq!(gb18030_ranges_pointer(0x10000), Some(189000));
        assert_eq!(gb18030_ranges_pointer(0x10_FFFF), Some(1237575));
    }
}
