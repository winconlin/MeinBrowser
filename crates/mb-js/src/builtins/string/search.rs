//! StringIndexOf und StringLastIndexOf (Abschnitte 6.1.4.1 und 6.1.4.2).
//!
//! Die Spezifikation vergleicht an jeder Stelle die ganze gesuchte Folge;
//! das kostet im schlechtesten Fall Länge mal Länge. Hier sucht ab
//! [`NAIVE_LIMIT`] Codeeinheiten das Verfahren von Knuth, Morris und Pratt
//! mit einer Tabelle der längsten Ränder, das jede Codeeinheit der
//! Zeichenkette höchstens zweimal ansieht. Das Ergebnis ist dasselbe.
//!
//! Die Zeichenkette kommt als Funktion von der Stelle zur Codeeinheit, damit
//! Aufrufer wie `indexOf` sie nicht erst kopieren müssen.

/// Bis zu dieser Länge der gesuchten Folge wird Stelle für Stelle
/// verglichen; der schlechteste Fall kostet dann höchstens so viele
/// Vergleiche je Stelle.
const NAIVE_LIMIT: usize = 16;

/// StringIndexOf: die erste Stelle ab `from`, an der `needle` in der
/// Zeichenkette mit `length` Codeeinheiten steht.
pub(super) fn index_of(
    unit: impl Fn(usize) -> Option<u16>,
    length: usize,
    needle: &[u16],
    from: usize,
) -> Option<usize> {
    if from > length {
        return None;
    }
    let Some(&first) = needle.first() else {
        return Some(from);
    };
    // Die letzte Stelle, an der die Folge noch ganz in die Zeichenkette
    // passt.
    let last = length.checked_sub(needle.len())?;
    if from > last {
        return None;
    }
    if needle.len() == 1 {
        return (from..=last).find(|&index| unit(index) == Some(first));
    }
    if needle.len() <= NAIVE_LIMIT {
        return (from..=last).find(|&index| matches_at(&unit, needle, index));
    }
    knuth_morris_pratt(&unit, length, needle, from)
}

/// StringLastIndexOf: die letzte Stelle bis `from`, an der `needle` steht.
/// Der Aufrufer sorgt dafür, dass die Folge an der Stelle `from` noch in
/// die Zeichenkette passt.
pub(super) fn last_index_of(
    unit: impl Fn(usize) -> Option<u16>,
    length: usize,
    needle: &[u16],
    from: usize,
) -> Option<usize> {
    let end = from.checked_add(needle.len())?;
    if end > length {
        return None;
    }
    // Rückwärts gelesen ist die letzte Stelle die erste: Die Stelle `i` der
    // umgedrehten Zeichenkette ist `length - 1 - i`.
    let reversed: Vec<u16> = needle.iter().rev().copied().collect();
    let backwards = |index: usize| unit(length.checked_sub(index)?.checked_sub(1)?);
    let found = index_of(backwards, length, &reversed, length - end)?;
    // Der Fund reicht umgedreht von `found` bis vor `found + needle.len()`.
    length.checked_sub(found)?.checked_sub(needle.len())
}

/// Ob `needle` an der Stelle `index` steht.
pub(super) fn matches_at(
    unit: impl Fn(usize) -> Option<u16>,
    needle: &[u16],
    index: usize,
) -> bool {
    needle
        .iter()
        .enumerate()
        .all(|(offset, &expected)| index.checked_add(offset).and_then(&unit) == Some(expected))
}

/// Die Suche nach Knuth, Morris und Pratt ab der Stelle `from`.
fn knuth_morris_pratt(
    unit: impl Fn(usize) -> Option<u16>,
    length: usize,
    needle: &[u16],
    from: usize,
) -> Option<usize> {
    let borders = borders(needle);
    // So viele Codeeinheiten von `needle` stimmen vor `index` überein.
    let mut matched = 0;
    for index in from..length {
        let current = unit(index)?;
        while matched > 0 && needle.get(matched) != Some(&current) {
            matched = border(&borders, matched);
        }
        if needle.get(matched) == Some(&current) {
            matched += 1;
        }
        if matched == needle.len() {
            return index.checked_add(1)?.checked_sub(matched);
        }
    }
    None
}

/// Zu jeder Länge eines Anfangs von `needle` die Länge seines längsten
/// echten Rands, also des längsten Anfangs, der auch sein Ende ist. Der
/// Eintrag `i` gilt für den Anfang mit `i + 1` Codeeinheiten. Die Längen
/// passen in 32 Bit, weil keine Zeichenkette länger ist.
fn borders(needle: &[u16]) -> Vec<u32> {
    let mut borders: Vec<u32> = Vec::with_capacity(needle.len());
    let mut matched = 0;
    for (index, unit) in needle.iter().enumerate() {
        if index == 0 {
            borders.push(0);
            continue;
        }
        while matched > 0 && needle.get(matched) != Some(unit) {
            matched = border(&borders, matched);
        }
        if needle.get(matched) == Some(unit) {
            matched += 1;
        }
        borders.push(u32::try_from(matched).unwrap_or(u32::MAX));
    }
    borders
}

/// Der längste echte Rand des Anfangs mit `matched` Codeeinheiten.
fn border(borders: &[u32], matched: usize) -> usize {
    matched
        .checked_sub(1)
        .and_then(|index| borders.get(index))
        .and_then(|&border| usize::try_from(border).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn find(text: &str, needle: &str, from: usize) -> Option<usize> {
        let text = units(text);
        index_of(
            |index| text.get(index).copied(),
            text.len(),
            &units(needle),
            from,
        )
    }

    fn find_last(text: &str, needle: &str, from: usize) -> Option<usize> {
        let text = units(text);
        last_index_of(
            |index| text.get(index).copied(),
            text.len(),
            &units(needle),
            from,
        )
    }

    /// Die Schritte der Spezifikation, Stelle für Stelle.
    fn reference(text: &[u16], needle: &[u16], from: usize) -> Option<usize> {
        if needle.is_empty() && from <= text.len() {
            return Some(from);
        }
        let last = text.len().checked_sub(needle.len())?;
        (from..=last).find(|&index| text.get(index..index + needle.len()) == Some(needle))
    }

    fn reference_last(text: &[u16], needle: &[u16], from: usize) -> Option<usize> {
        (0..=from)
            .rev()
            .find(|&index| text.get(index..index + needle.len()) == Some(needle))
    }

    #[test]
    fn empty_and_short_needles() {
        assert_eq!(find("abc", "", 0), Some(0));
        assert_eq!(find("abc", "", 3), Some(3));
        assert_eq!(find("abc", "", 4), None);
        assert_eq!(find("", "", 0), Some(0));
        assert_eq!(find("", "a", 0), None);
        assert_eq!(find("abcabc", "c", 0), Some(2));
        assert_eq!(find("abcabc", "c", 3), Some(5));
        assert_eq!(find("abcabc", "bc", 2), Some(4));
        assert_eq!(find("abcabc", "abcabcd", 0), None);
        assert_eq!(find("abcabc", "cb", 0), None);
    }

    #[test]
    fn last_index() {
        assert_eq!(find_last("abcabc", "bc", 4), Some(4));
        assert_eq!(find_last("abcabc", "bc", 3), Some(1));
        assert_eq!(find_last("abcabc", "bc", 0), None);
        assert_eq!(find_last("abcabc", "", 6), Some(6));
        assert_eq!(find_last("abcabc", "", 2), Some(2));
        assert_eq!(find_last("abcabc", "abcabc", 0), Some(0));
        // Die Folge passt an der Stelle nicht mehr in die Zeichenkette.
        assert_eq!(find_last("abc", "bc", 2), None);
    }

    #[test]
    fn long_needles_agree_with_the_specification() {
        // Ein kleiner Zeichenvorrat gibt viele Ränder und Teilfunde.
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = |bound: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % bound
        };
        for _ in 0..200 {
            let length = usize::try_from(next(300)).unwrap_or(0);
            let text: Vec<u16> = (0..length)
                .map(|_| u16::try_from(next(2)).unwrap_or(0))
                .collect();
            let needle_length = usize::try_from(next(40)).unwrap_or(0);
            let needle: Vec<u16> = if next(2) == 0 && needle_length <= length {
                // Ein Stück der Zeichenkette selbst kommt sicher vor.
                let start = usize::try_from(next(64)).unwrap_or(0) % (length - needle_length + 1);
                text.get(start..start + needle_length)
                    .unwrap_or_default()
                    .to_vec()
            } else {
                (0..needle_length)
                    .map(|_| u16::try_from(next(2)).unwrap_or(0))
                    .collect()
            };
            let unit = |index: usize| text.get(index).copied();
            for from in 0..=length + 1 {
                assert_eq!(
                    index_of(unit, length, &needle, from),
                    reference(&text, &needle, from),
                    "{text:?} {needle:?} {from}"
                );
            }
            if let Some(last) = length.checked_sub(needle.len()) {
                for from in 0..=last {
                    assert_eq!(
                        last_index_of(unit, length, &needle, from),
                        reference_last(&text, &needle, from),
                        "{text:?} {needle:?} {from}"
                    );
                }
            }
        }
    }

    #[test]
    fn repeated_prefixes_stay_linear() {
        // aaa…ab in aaa…a: Stelle für Stelle wären das 10¹⁰ Vergleiche.
        let text = vec![u16::from(b'a'); 200_000];
        let mut needle = vec![u16::from(b'a'); 50_000];
        needle.push(u16::from(b'b'));
        let unit = |index: usize| text.get(index).copied();
        assert_eq!(index_of(unit, text.len(), &needle, 0), None);
        needle.pop();
        assert_eq!(index_of(unit, text.len(), &needle, 7), Some(7));
        assert_eq!(
            last_index_of(unit, text.len(), &needle, 150_000),
            Some(150_000)
        );
    }
}
