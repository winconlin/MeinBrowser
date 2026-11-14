//! Zeichenmengen (CharSet, Abschnitt 22.2.2.1) für die Engine: Bereiche
//! einzelner Zeichen und, nur mit `v`, Zeichenketten. Dazu Canonicalize
//! (Abschnitt 22.2.2.7.3) und die Mengen, die CompileToCharSet (Abschnitt
//! 22.2.2.9) aus Tabellen holt.
//!
//! Ein Zeichen ist wie im Musterbaum mit `u` oder `v` ein Codepunkt, sonst
//! eine Codeeinheit. Einzelne Surrogate sind Zeichen wie alle anderen
//! (RS-9).

use std::collections::BTreeSet;

use mb_unicode::GeneralCategory;
use mb_unicode::case_folding::{simple_fold, simple_foldings};
use mb_unicode::case_mapping::{uppercase_mapping, uppercase_mappings};

use super::{ClassEscape, Property};

/// Der größte Codepunkt.
pub(crate) const MAX_CODE_POINT: u32 = 0x10_FFFF;
/// Die größte Codeeinheit.
pub(crate) const MAX_CODE_UNIT: u32 = 0xFFFF;

/// Ein Bereich von Zeichen, beide Enden gehören dazu.
pub(crate) type Range = (u32, u32);

/// Eine Menge von Zeichen und Zeichenketten.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CharSet {
    /// Aufsteigend, ohne Überlappung und ohne Nachbarn, die sich berühren.
    ranges: Vec<Range>,
    /// Zeichenketten mit keinem oder mindestens zwei Zeichen; eine mit
    /// genau einem steht als Zeichen in `ranges`.
    strings: BTreeSet<Vec<u32>>,
}

impl CharSet {
    pub(crate) fn new() -> CharSet {
        CharSet::default()
    }

    /// Die Menge aus beliebigen Bereichen, auch ungeordnet und überlappend.
    pub(crate) fn from_ranges(ranges: impl IntoIterator<Item = Range>) -> CharSet {
        CharSet {
            ranges: normalize(ranges.into_iter().collect()),
            strings: BTreeSet::new(),
        }
    }

    pub(crate) fn single(character: u32) -> CharSet {
        CharSet::from_ranges([(character, character)])
    }

    pub(crate) fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    pub(crate) fn strings(&self) -> &BTreeSet<Vec<u32>> {
        &self.strings
    }

    /// Nimmt eine Zeichenkette auf; eine mit einem Zeichen als Zeichen.
    pub(crate) fn insert_string(&mut self, string: Vec<u32>) {
        match string.as_slice() {
            [character] => self.ranges = normalize_with(&self.ranges, (*character, *character)),
            _ => {
                self.strings.insert(string);
            }
        }
    }

    pub(crate) fn contains(&self, character: u32) -> bool {
        contains(&self.ranges, character)
    }

    /// Die Vereinigung beliebig vieler Mengen in einem Durchgang.
    pub(crate) fn union_all(sets: &[CharSet]) -> CharSet {
        let ranges = sets
            .iter()
            .flat_map(|set| set.ranges.iter().copied())
            .collect();
        CharSet {
            ranges: normalize(ranges),
            strings: sets
                .iter()
                .flat_map(|set| set.strings.iter().cloned())
                .collect(),
        }
    }

    pub(crate) fn intersection(&self, other: &CharSet) -> CharSet {
        CharSet {
            ranges: intersect(&self.ranges, &other.ranges),
            strings: self.strings.intersection(&other.strings).cloned().collect(),
        }
    }

    /// Die Elemente dieser Menge, die nicht in `other` sind.
    pub(crate) fn difference(&self, other: &CharSet) -> CharSet {
        CharSet {
            ranges: subtract(&self.ranges, &other.ranges),
            strings: self.strings.difference(&other.strings).cloned().collect(),
        }
    }
}

/// Ob `character` in einem der geordneten Bereiche liegt.
pub(crate) fn contains(ranges: &[Range], character: u32) -> bool {
    let after = ranges.partition_point(|&(first, _)| first <= character);
    after
        .checked_sub(1)
        .and_then(|index| ranges.get(index))
        .is_some_and(|&(_, last)| character <= last)
}

/// Ordnet Bereiche und fasst zusammen, was sich überlappt oder berührt.
fn normalize(mut ranges: Vec<Range>) -> Vec<Range> {
    ranges.retain(|&(first, last)| first <= last);
    ranges.sort_unstable();
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for (first, last) in ranges {
        match merged.last_mut() {
            Some(previous) if first <= previous.1.saturating_add(1) => {
                previous.1 = previous.1.max(last);
            }
            _ => merged.push((first, last)),
        }
    }
    merged
}

fn normalize_with(ranges: &[Range], range: Range) -> Vec<Range> {
    normalize(ranges.iter().copied().chain([range]).collect())
}

/// Der Schnitt zweier geordneter Listen von Bereichen.
fn intersect(a: &[Range], b: &[Range]) -> Vec<Range> {
    let mut result = Vec::new();
    let (mut i, mut j) = (0_usize, 0_usize);
    while let (Some(&(a_first, a_last)), Some(&(b_first, b_last))) = (a.get(i), b.get(j)) {
        let first = a_first.max(b_first);
        let last = a_last.min(b_last);
        if first <= last {
            result.push((first, last));
        }
        if a_last < b_last {
            i = i.saturating_add(1);
        } else {
            j = j.saturating_add(1);
        }
    }
    result
}

/// Die Bereiche von `a` ohne die von `b`, beide geordnet.
fn subtract(a: &[Range], b: &[Range]) -> Vec<Range> {
    let mut result = Vec::new();
    let mut skipped = 0_usize;
    for &(first, last) in a {
        // Bereiche von `b`, die ganz vor diesem liegen, auch vor allen
        // späteren.
        while b.get(skipped).is_some_and(|&(_, b_last)| b_last < first) {
            skipped = skipped.saturating_add(1);
        }
        let mut start = first;
        let mut rest = true;
        let mut index = skipped;
        while let Some(&(b_first, b_last)) = b.get(index) {
            if b_first > last {
                break;
            }
            if b_first > start {
                result.push((start, b_first.saturating_sub(1)));
            }
            if b_last >= last {
                rest = false;
                break;
            }
            start = b_last.saturating_add(1);
            index = index.saturating_add(1);
        }
        if rest {
            result.push((start, last));
        }
    }
    result
}

/// Wie Canonicalize (Abschnitt 22.2.2.7.3) Zeichen abbildet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Folding {
    /// Ohne `i` bleibt jedes Zeichen.
    None,
    /// `i` ohne `u` und `v`: toUppercase für eine Codeeinheit, wenn das
    /// Ergebnis wieder eine Codeeinheit ist und den ASCII-Bereich nicht
    /// betritt.
    Upper,
    /// `i` mit `u` oder `v`: die einfache Faltung (scf).
    Simple,
}

impl Folding {
    pub(crate) fn canonicalize(self, character: u32) -> u32 {
        match self {
            Folding::None => character,
            Folding::Upper => upper(character),
            Folding::Simple => match character {
                0x41..=0x5A => character.saturating_add(0x20),
                0..=0x7F => character,
                _ => simple_fold(character),
            },
        }
    }

    /// Die Bilder aller Zeichen von `ranges` unter Canonicalize. Ein
    /// Zeichen ist genau dann darin, wenn es das Bild eines Zeichens der
    /// Menge ist; damit vergleicht CharacterSetMatcher (Abschnitt
    /// 22.2.2.7.1).
    pub(crate) fn canonical_ranges(self, ranges: &[Range]) -> Vec<Range> {
        // Nur die Zeichen aus den Tabellen ändern sich. Wer davon in der
        // Menge ist, geht hinaus, und sein Bild kommt hinein.
        let mut removed = Vec::new();
        let mut added = Vec::new();
        let mut visit = |source: u32, target: u32| {
            if source != target && contains(ranges, source) {
                removed.push((source, source));
                added.push((target, target));
            }
        };
        match self {
            Folding::None => return ranges.to_vec(),
            Folding::Upper => {
                for (source, _) in uppercase_mappings() {
                    visit(source, upper(source));
                }
            }
            Folding::Simple => {
                for (source, target) in simple_foldings() {
                    visit(source, target);
                }
            }
        }
        let mut result = subtract(ranges, &normalize(removed));
        result.extend(added);
        normalize(result)
    }
}

/// Canonicalize ohne `u` und `v`.
fn upper(character: u32) -> u32 {
    match character {
        0x61..=0x7A => character.saturating_sub(0x20),
        0..=0x7F => character,
        _ if character > MAX_CODE_UNIT => character,
        _ => match uppercase_mapping(character) {
            Some(&[mapped]) if (0x80..=MAX_CODE_UNIT).contains(&mapped) => mapped,
            _ => character,
        },
    }
}

/// Die Flags, von denen die Mengen abhängen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mode {
    /// `u` oder `v`: Zeichen sind Codepunkte.
    pub(crate) unicode: bool,
    /// `v`
    pub(crate) sets: bool,
    /// `i`, auch aus einer Gruppe wie `(?i:…)`.
    pub(crate) ignore_case: bool,
}

impl Mode {
    pub(crate) fn folding(self) -> Folding {
        match (self.ignore_case, self.unicode) {
            (false, _) => Folding::None,
            (true, false) => Folding::Upper,
            (true, true) => Folding::Simple,
        }
    }

    /// AllCharacters (Abschnitt 22.2.2.9.4).
    pub(crate) fn all_characters(self) -> CharSet {
        if self.sets && self.ignore_case {
            let changed = simple_foldings().map(|(source, _)| (source, source));
            return CharSet::from_ranges([(0, MAX_CODE_POINT)])
                .difference(&CharSet::from_ranges(changed));
        }
        let last = if self.unicode {
            MAX_CODE_POINT
        } else {
            MAX_CODE_UNIT
        };
        CharSet::from_ranges([(0, last)])
    }

    /// CharacterComplement (Abschnitt 22.2.2.9.6).
    pub(crate) fn complement(self, set: &CharSet) -> CharSet {
        self.all_characters().difference(set)
    }

    /// MaybeSimpleCaseFolding (Abschnitt 22.2.2.9.5).
    pub(crate) fn maybe_simple_case_folding(self, set: CharSet) -> CharSet {
        if !(self.sets && self.ignore_case) {
            return set;
        }
        CharSet {
            ranges: Folding::Simple.canonical_ranges(&set.ranges),
            strings: set
                .strings
                .iter()
                .map(|string| string.iter().map(|&c| simple_fold(c)).collect())
                .collect(),
        }
    }

    /// WordCharacters (Abschnitt 22.2.2.9.3).
    pub(crate) fn word_characters(self) -> CharSet {
        let mut ranges = WORD.to_vec();
        if self.unicode && self.ignore_case {
            ranges.extend(FOLDED_WORD.map(|c| (c, c)));
        }
        CharSet::from_ranges(ranges)
    }

    /// CompileToCharSet für *CharacterClassEscape* ohne `\p` (Abschnitt
    /// 22.2.2.9).
    pub(crate) fn class_escape(self, escape: ClassEscape) -> CharSet {
        let digits = || CharSet::from_ranges([(0x30, 0x39)]);
        let word = || self.maybe_simple_case_folding(self.word_characters());
        match escape {
            ClassEscape::Digit => digits(),
            ClassEscape::NotDigit => self.complement(&digits()),
            ClassEscape::Space => white_space(),
            ClassEscape::NotSpace => self.complement(&white_space()),
            ClassEscape::Word => word(),
            ClassEscape::NotWord => self.complement(&word()),
        }
    }
}

/// Die Wortzeichen aus ASCII.
const WORD: [Range; 4] = [(0x30, 0x39), (0x41, 0x5A), (0x5F, 0x5F), (0x61, 0x7A)];

/// Die Zeichen außerhalb von ASCII, die die einfache Faltung auf ein
/// Wortzeichen abbildet: U+017F (ſ) und U+212A (Kelvin). Ein Test gleicht
/// sie mit den Tabellen ab.
pub(crate) const FOLDED_WORD: [u32; 2] = [0x017F, 0x212A];

/// Ob eine Codeeinheit ein Wortzeichen aus ASCII ist.
pub(crate) fn is_ascii_word(unit: u32) -> bool {
    contains(&WORD, unit)
}

/// Die Zeilenenden (*LineTerminator*, Abschnitt 12.3).
pub(crate) fn is_line_terminator(character: u32) -> bool {
    matches!(character, 0x0A | 0x0D | 0x2028 | 0x2029)
}

/// *WhiteSpace* und *LineTerminator* (Abschnitte 12.2 und 12.3): die
/// Zeichen für `\s`.
pub(crate) fn white_space() -> CharSet {
    let fixed = [(0x09, 0x0D), (0x2028, 0x2029), (0xFEFF, 0xFEFF)];
    let separators = mb_unicode::general_category_sections()
        .filter(|&(_, _, category)| category == GeneralCategory::SpaceSeparator)
        .map(|(first, last, _)| (first, last));
    CharSet::from_ranges(fixed.into_iter().chain(separators))
}

/// Die Codepunkte mit einer Eigenschaft (CompileToCharSet für
/// *UnicodePropertyValueExpression*, vor MaybeSimpleCaseFolding). `None`
/// für die Eigenschaften, deren Tabellen `mb-unicode` noch nicht hat.
pub(crate) fn property(property: Property) -> Option<CharSet> {
    match property {
        Property::GeneralCategory(value) | Property::NamedGeneralCategory(value) => {
            Some(general_category(value))
        }
        Property::Binary("Any") => Some(CharSet::from_ranges([(0, MAX_CODE_POINT)])),
        Property::Binary("ASCII") => Some(CharSet::from_ranges([(0, 0x7F)])),
        Property::Binary("Assigned") => {
            let unassigned = general_category("Cn");
            Some(CharSet::from_ranges([(0, MAX_CODE_POINT)]).difference(&unassigned))
        }
        Property::Binary(_)
        | Property::Script(_)
        | Property::ScriptExtensions(_)
        | Property::Strings(_) => None,
    }
}

/// Die Codepunkte mit einem Wert von General_Category; `value` ist der
/// Kurzname eines Werts oder einer Gruppe, wie der Parser ihn liefert.
fn general_category(value: &str) -> CharSet {
    type G = GeneralCategory;
    let group: Option<fn(G) -> bool> = match value {
        "C" => Some(G::is_other),
        "L" => Some(G::is_letter),
        "LC" => Some(G::is_cased_letter),
        "M" => Some(G::is_mark),
        "N" => Some(G::is_number),
        "P" => Some(G::is_punctuation),
        "S" => Some(G::is_symbol),
        "Z" => Some(G::is_separator),
        _ => None,
    };
    let sections = mb_unicode::general_category_sections()
        .filter(|&(_, _, category)| match group {
            Some(group) => group(category),
            None => category.short_name() == value,
        })
        .map(|(first, last, _)| (first, last));
    CharSet::from_ranges(sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ranges: &[Range]) -> CharSet {
        CharSet::from_ranges(ranges.iter().copied())
    }

    #[test]
    fn ranges_are_ordered_and_merged() {
        let mixed = set(&[(10, 20), (0, 3), (4, 5), (15, 30), (40, 40), (9, 2)]);
        assert_eq!(mixed.ranges(), [(0, 5), (10, 30), (40, 40)]);
        assert!(mixed.contains(0) && mixed.contains(5) && mixed.contains(30));
        assert!(!mixed.contains(6) && !mixed.contains(39) && !mixed.contains(41));
        assert!(!CharSet::new().contains(0));
        let top = set(&[(u32::MAX - 1, u32::MAX), (5, u32::MAX)]);
        assert_eq!(top.ranges(), [(5, u32::MAX)]);
    }

    #[test]
    fn set_operations_agree_with_membership() {
        // Zufällige Mengen über einem kleinen Bereich, verglichen Zeichen
        // für Zeichen.
        let mut state = 0x9E37_79B9_u32;
        let mut next = |bound: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state % bound
        };
        for _ in 0..200 {
            let mut random = || {
                let count = next(6);
                let ranges: Vec<Range> = (0..count)
                    .map(|_| {
                        let first = next(60);
                        (first, first + next(8))
                    })
                    .collect();
                CharSet::from_ranges(ranges)
            };
            let (a, b) = (random(), random());
            let union = CharSet::union_all(&[a.clone(), b.clone()]);
            let (both, only) = (a.intersection(&b), a.difference(&b));
            for c in 0..80 {
                assert_eq!(
                    union.contains(c),
                    a.contains(c) || b.contains(c),
                    "{a:?} {b:?}"
                );
                assert_eq!(
                    both.contains(c),
                    a.contains(c) && b.contains(c),
                    "{a:?} {b:?}"
                );
                assert_eq!(
                    only.contains(c),
                    a.contains(c) && !b.contains(c),
                    "{a:?} {b:?}"
                );
            }
            for result in [&union, &both, &only] {
                let ranges = result.ranges();
                assert!(ranges.iter().all(|&(first, last)| first <= last));
                assert!(ranges.windows(2).all(|pair| pair[0].1 + 1 < pair[1].0));
            }
        }
    }

    #[test]
    fn strings_take_part_in_the_operations() {
        let mut a = set(&[(0x61, 0x63)]);
        a.insert_string(vec![0x61, 0x62]);
        a.insert_string(vec![]);
        a.insert_string(vec![0x7A]);
        assert!(a.contains(0x7A));
        assert_eq!(a.strings().len(), 2);
        let mut b = set(&[(0x62, 0x62)]);
        b.insert_string(vec![0x61, 0x62]);
        assert_eq!(a.intersection(&b).strings().len(), 1);
        assert_eq!(a.intersection(&b).ranges(), [(0x62, 0x62)]);
        let only = a.difference(&b);
        assert_eq!(only.ranges(), [(0x61, 0x61), (0x63, 0x63), (0x7A, 0x7A)]);
        assert!(only.strings().contains(&Vec::new()));
        assert_eq!(only.strings().len(), 1);
        assert_eq!(CharSet::union_all(&[a.clone(), b]).strings().len(), 2);
    }

    #[test]
    fn canonicalize_follows_the_flags() {
        let (none, upper, simple) = (Folding::None, Folding::Upper, Folding::Simple);
        assert_eq!(none.canonicalize(0x61), 0x61);
        assert_eq!(upper.canonicalize(0x61), 0x41);
        assert_eq!(simple.canonicalize(0x41), 0x61);
        // ß wird groß zu zwei Zeichen und bleibt deshalb.
        assert_eq!(upper.canonicalize(0xDF), 0xDF);
        // ſ und das Kelvin-Zeichen würden in den ASCII-Bereich fallen: ohne
        // `u` bleiben sie, mit `u` nicht.
        assert_eq!(upper.canonicalize(0x17F), 0x17F);
        assert_eq!(upper.canonicalize(0x212A), 0x212A);
        assert_eq!(simple.canonicalize(0x17F), 0x73);
        assert_eq!(simple.canonicalize(0x212A), 0x6B);
        // Das Ohm-Zeichen: toUppercase lässt es, die Faltung nicht.
        assert_eq!(upper.canonicalize(0x2126), 0x2126);
        assert_eq!(simple.canonicalize(0x2126), 0x3C9);
        assert_eq!(upper.canonicalize(0x3C9), 0x3A9);
        // Einzelne Surrogate und Zeichen ohne Abbildung bleiben.
        for folding in [none, upper, simple] {
            assert_eq!(folding.canonicalize(0xD800), 0xD800);
            assert_eq!(folding.canonicalize(0x4E00), 0x4E00);
        }
        assert_eq!(upper.canonicalize(0x1_0428), 0x1_0428);
        assert_eq!(simple.canonicalize(0x1_0400), 0x1_0428);
    }

    #[test]
    fn canonical_ranges_hold_exactly_the_images() {
        let all = Mode {
            unicode: true,
            sets: false,
            ignore_case: true,
        };
        for folding in [Folding::Upper, Folding::Simple] {
            for ranges in [
                vec![(0x41, 0x5A)],
                vec![(0x61, 0x7A)],
                vec![(0x45, 0x66)],
                vec![(0x100, 0x24F)],
                vec![(0x3A3, 0x3A3), (0x3C2, 0x3C3)],
                vec![(0x212A, 0x212A)],
                vec![(0, 0x7F), (0x400, 0x4FF)],
            ] {
                let canonical = folding.canonical_ranges(&ranges);
                let mut expected = BTreeSet::new();
                for &(first, last) in &ranges {
                    expected.extend((first..=last).map(|c| folding.canonicalize(c)));
                }
                let found: BTreeSet<u32> = canonical
                    .iter()
                    .flat_map(|&(first, last)| first..=last)
                    .collect();
                assert_eq!(found, expected, "{folding:?} {ranges:?}");
            }
        }
        assert_eq!(Folding::None.canonical_ranges(&[(1, 2)]), [(1, 2)]);
        // Die Faltung ist idempotent, also ist die Menge aller gefalteten
        // Zeichen ihr eigenes Bild.
        let folded = Mode { sets: true, ..all }.all_characters();
        assert_eq!(
            Folding::Simple.canonical_ranges(folded.ranges()),
            folded.ranges()
        );
        assert!(folded.contains(0x61) && !folded.contains(0x41));
    }

    #[test]
    fn word_characters_beyond_ascii_match_the_tables() {
        let extra: Vec<u32> = simple_foldings()
            .filter(|&(source, target)| is_ascii_word(target) && !is_ascii_word(source))
            .map(|(source, _)| source)
            .collect();
        assert_eq!(extra, FOLDED_WORD);
        let mode = Mode {
            unicode: true,
            sets: false,
            ignore_case: true,
        };
        assert!(mode.word_characters().contains(0x17F));
        let plain = Mode {
            ignore_case: false,
            ..mode
        };
        assert!(!plain.word_characters().contains(0x17F));
        // Ohne `u` ändert `i` an den Wortzeichen nichts.
        let legacy = Mode {
            unicode: false,
            ..mode
        };
        assert_eq!(legacy.word_characters(), plain.word_characters());
    }

    #[test]
    fn class_escapes_and_complements() {
        let legacy = Mode {
            unicode: false,
            sets: false,
            ignore_case: false,
        };
        let unicode = Mode {
            unicode: true,
            ..legacy
        };
        assert_eq!(
            legacy.class_escape(ClassEscape::Digit).ranges(),
            [(0x30, 0x39)]
        );
        assert_eq!(
            legacy.class_escape(ClassEscape::NotDigit).ranges(),
            [(0, 0x2F), (0x3A, 0xFFFF)]
        );
        assert_eq!(
            unicode.class_escape(ClassEscape::NotDigit).ranges(),
            [(0, 0x2F), (0x3A, 0x10_FFFF)]
        );
        let space = legacy.class_escape(ClassEscape::Space);
        for c in [
            0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0xA0, 0x1680, 0x2000, 0x200A,
        ] {
            assert!(space.contains(c), "U+{c:04X}");
        }
        for c in [0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF] {
            assert!(space.contains(c), "U+{c:04X}");
        }
        for c in [0x08, 0x0E, 0x180E, 0x200B, 0x85] {
            assert!(!space.contains(c), "U+{c:04X}");
        }
        // Mit `v` und `i` stehen die Mengen in gefalteter Form: `\w` ohne
        // Großbuchstaben, `\W` ohne alles, was auf ein Wortzeichen faltet.
        let folded = Mode {
            unicode: true,
            sets: true,
            ignore_case: true,
        };
        let word = folded.class_escape(ClassEscape::Word);
        assert!(word.contains(0x61) && !word.contains(0x41) && !word.contains(0x17F));
        let not_word = folded.class_escape(ClassEscape::NotWord);
        for c in [0x61, 0x41, 0x17F, 0x212A, 0x73, 0x6B] {
            assert!(!not_word.contains(c), "U+{c:04X}");
        }
        assert!(not_word.contains(0x2D));
    }

    #[test]
    fn properties_from_the_general_category() {
        let letters = property(Property::GeneralCategory("L")).expect("Buchstaben");
        assert!(letters.contains(0x41) && letters.contains(0x4E00) && !letters.contains(0x30));
        let upper = property(Property::NamedGeneralCategory("Lu")).expect("Großbuchstaben");
        assert!(upper.contains(0x41) && !upper.contains(0x61));
        let surrogates = property(Property::GeneralCategory("Cs")).expect("Surrogate");
        assert_eq!(surrogates.ranges(), [(0xD800, 0xDFFF)]);
        let any = property(Property::Binary("Any")).expect("Any");
        assert_eq!(any.ranges(), [(0, MAX_CODE_POINT)]);
        let assigned = property(Property::Binary("Assigned")).expect("Assigned");
        assert!(assigned.contains(0x41) && !assigned.contains(0x378));
        assert_eq!(
            property(Property::Binary("ASCII")).expect("ASCII").ranges(),
            [(0, 0x7F)]
        );
        // Jeder Codepunkt hat genau eine Kategorie.
        let mut total = 0_u64;
        for value in [
            "Cc", "Cf", "Cn", "Co", "Cs", "Ll", "Lm", "Lo", "Lt", "Lu", "Mc", "Me", "Mn", "Nd",
            "Nl", "No", "Pc", "Pd", "Pe", "Pf", "Pi", "Po", "Ps", "Sc", "Sk", "Sm", "So", "Zl",
            "Zp", "Zs",
        ] {
            let set = property(Property::GeneralCategory(value)).expect(value);
            assert!(!set.ranges().is_empty(), "{value}");
            total += set
                .ranges()
                .iter()
                .map(|&(first, last)| u64::from(last - first) + 1)
                .sum::<u64>();
        }
        assert_eq!(total, u64::from(MAX_CODE_POINT) + 1);
        assert!(property(Property::Script("Latn")).is_none());
        assert!(property(Property::Binary("Alphabetic")).is_none());
    }
}
