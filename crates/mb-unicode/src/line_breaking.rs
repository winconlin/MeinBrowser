//! Zeilenumbruch nach UAX #14 (Unicode Line Breaking Algorithm) ohne
//! Anpassungen: die Regeln LB2 bis LB31 so, wie die Unicode-Werkzeuge sie für
//! `LineBreakTest.txt` umsetzen (`Segmenter-18.0.0.txt` in
//! unicode-org/unicodetools).
//!
//! LB1 löst wie dort AI, SG, XX und SA ohne Mn oder Mc zu AL auf, SA mit Mn
//! oder Mc zu CM und CJ zu NS. Wörterbücher für SA (Thai, Lao, Khmer,
//! Myanmar) und die Anpassungen aus CSS Text (`line-break`, `word-break`)
//! fehlen noch. Codepunkte sind `u32`, einzelne Surrogate gelten als SG
//! (RS-9).

use crate::{EastAsianWidth, GeneralCategory, LineBreak, east_asian_width, general_category};

use LineBreak as L;

/// Art einer Umbruchstelle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Break {
    /// Nach einem harten Zeilenumbruch (BK, CR, LF, NL; CR LF zählt als
    /// einer): Die Zeile muss hier enden.
    Mandatory,
    /// Hier darf die Zeile enden.
    Allowed,
}

/// Die Umbruchstellen in `code_points`: der Index des Codepunkts, vor dem
/// umbrochen wird, und die Art. Am Anfang wird nie umbrochen (LB2), das Ende
/// ist immer eine Umbruchstelle (LB3) mit dem Index `code_points.len()`,
/// zwingend nur nach einem harten Zeilenumbruch. Ein leerer Text hat keine.
pub fn line_breaks(code_points: &[u32]) -> Vec<(usize, Break)> {
    let items = items(code_points);
    let context = Context::new(&items);
    let mut breaks = Vec::new();
    for (position, item) in items.iter().enumerate().skip(1) {
        if let Some(kind) = context.decide(position) {
            breaks.push((item.start, kind));
        }
    }
    if let Some(last) = items.last() {
        let kind = if HARD_BREAKS.contains(&last.class) {
            Break::Mandatory
        } else {
            Break::Allowed
        };
        breaks.push((code_points.len(), kind));
    }
    breaks
}

/// Die Klassen der harten Zeilenumbrüche.
const HARD_BREAKS: [LineBreak; 4] = [
    L::MandatoryBreak,
    L::CarriageReturn,
    L::LineFeed,
    L::NextLine,
];

/// Ein Zeichen nach LB9 und LB10: ein Codepunkt mit den CM und ZWJ, die an
/// ihm hängen. Innerhalb eines Zeichens wird nie umbrochen.
#[derive(Clone, Copy)]
struct Item {
    /// Index des ersten Codepunkts.
    start: usize,
    /// Die Klasse nach LB1, LB9 und LB10.
    class: LineBreak,
    /// East_Asian_Width F, W oder H.
    east_asian: bool,
    /// Allgemeine Kategorie Pi (öffnendes Anführungszeichen).
    initial_quote: bool,
    /// Allgemeine Kategorie Pf (schließendes Anführungszeichen).
    final_quote: bool,
    /// U+25CC DOTTED CIRCLE (LB28a).
    dotted_circle: bool,
    /// Extended_Pictographic und nicht vergeben (LB30b).
    unassigned_pictographic: bool,
    /// Ob der letzte Codepunkt ZWJ ist (LB8a).
    ends_with_zwj: bool,
}

/// LB1.
fn resolve(code_point: u32) -> LineBreak {
    match crate::line_break(code_point) {
        L::Ambiguous | L::Surrogate | L::Unknown => L::Alphabetic,
        L::ComplexContext => match general_category(code_point) {
            GeneralCategory::NonspacingMark | GeneralCategory::SpacingMark => L::CombiningMark,
            _ => L::Alphabetic,
        },
        L::ConditionalJapaneseStarter => L::Nonstarter,
        class => class,
    }
}

/// Fasst die Codepunkte nach LB9 und LB10 zu Zeichen zusammen.
fn items(code_points: &[u32]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::with_capacity(code_points.len());
    for (start, &code_point) in code_points.iter().enumerate() {
        let class = resolve(code_point);
        let attached = matches!(class, L::CombiningMark | L::ZWJ);
        if attached
            && let Some(base) = items.last_mut()
            && !HARD_BREAKS.contains(&base.class)
            && !matches!(base.class, L::Space | L::ZWSpace)
        {
            // LB9: Das Zeichen behält die Eigenschaften seiner Basis.
            base.ends_with_zwj = class == L::ZWJ;
            continue;
        }
        let item = if attached {
            // LB10: wie der Buchstabe A.
            Item {
                start,
                class: L::Alphabetic,
                east_asian: false,
                initial_quote: false,
                final_quote: false,
                dotted_circle: false,
                unassigned_pictographic: false,
                ends_with_zwj: class == L::ZWJ,
            }
        } else {
            let category = general_category(code_point);
            Item {
                start,
                class,
                east_asian: matches!(
                    east_asian_width(code_point),
                    EastAsianWidth::Fullwidth | EastAsianWidth::Wide | EastAsianWidth::Halfwidth
                ),
                initial_quote: category == GeneralCategory::InitialPunctuation,
                final_quote: category == GeneralCategory::FinalPunctuation,
                dotted_circle: code_point == 0x25CC,
                unassigned_pictographic: category == GeneralCategory::Unassigned
                    && crate::is_extended_pictographic(code_point),
                ends_with_zwj: false,
            }
        };
        items.push(item);
    }
    items
}

/// Die Zeichen und was die Regeln vor einer Stelle nachschlagen, im Voraus
/// berechnet, damit lange Folgen von Leerzeichen nicht jedes Mal neu
/// durchlaufen werden.
struct Context<'a> {
    items: &'a [Item],
    /// Für jedes Zeichen das letzte an oder vor ihm, das kein SP ist.
    without_spaces: Vec<Option<usize>>,
    /// Ebenso ohne SY und IS (LB25).
    without_separators: Vec<Option<usize>>,
    /// Wie viele RI unmittelbar an jedem Zeichen enden (LB30a).
    regional_run: Vec<usize>,
}

impl<'a> Context<'a> {
    fn new(items: &'a [Item]) -> Self {
        let mut without_spaces = Vec::with_capacity(items.len());
        let mut without_separators = Vec::with_capacity(items.len());
        let mut regional_run = Vec::with_capacity(items.len());
        let (mut last_non_space, mut last_non_separator, mut run) = (None, None, 0_usize);
        for (index, item) in items.iter().enumerate() {
            if item.class != L::Space {
                last_non_space = Some(index);
            }
            if !matches!(item.class, L::BreakSymbols | L::InfixNumeric) {
                last_non_separator = Some(index);
            }
            run = if item.class == L::RegionalIndicator {
                run.saturating_add(1)
            } else {
                0
            };
            without_spaces.push(last_non_space);
            without_separators.push(last_non_separator);
            regional_run.push(run);
        }
        Context {
            items,
            without_spaces,
            without_separators,
            regional_run,
        }
    }

    /// Das Zeichen `offset` Stellen vor der Stelle, 1 für das unmittelbar
    /// davor.
    fn before(&self, position: usize, offset: usize) -> Option<&Item> {
        self.items.get(position.checked_sub(offset)?)
    }

    /// Das Zeichen `offset` Stellen nach dem unmittelbar folgenden.
    fn after(&self, position: usize, offset: usize) -> Option<&Item> {
        self.items.get(position.checked_add(offset)?)
    }

    /// Der Index des letzten Zeichens an oder vor `index`, das `skipped`
    /// nicht überspringt.
    fn skipping(skipped: &[Option<usize>], index: Option<usize>) -> Option<usize> {
        *skipped.get(index?)?
    }

    /// Entscheidet über die Stelle vor dem Zeichen `position` (mindestens 1):
    /// `None`, wenn dort nicht umbrochen wird.
    fn decide(&self, position: usize) -> Option<Break> {
        let previous = self.before(position, 1)?;
        let next = self.items.get(position)?;
        let after_next = self.after(position, 1);
        let before_previous = self.before(position, 2);
        let (p, n) = (previous.class, next.class);
        let class_is = |item: Option<&Item>, classes: &[LineBreak]| {
            item.is_some_and(|item| classes.contains(&item.class))
        };
        // Das letzte Zeichen vor der Stelle, das kein SP ist.
        let before_spaces_index = Self::skipping(&self.without_spaces, position.checked_sub(1));
        let before_spaces = before_spaces_index.and_then(|index| self.items.get(index));

        // LB4, LB5
        if p == L::CarriageReturn && n == L::LineFeed {
            return None;
        }
        if HARD_BREAKS.contains(&p) {
            return Some(Break::Mandatory);
        }
        // LB6, LB7
        if HARD_BREAKS.contains(&n) || matches!(n, L::Space | L::ZWSpace) {
            return None;
        }
        // LB8
        if class_is(before_spaces, &[L::ZWSpace]) {
            return Some(Break::Allowed);
        }
        // LB8a
        if previous.ends_with_zwj {
            return None;
        }
        // LB11, LB12
        if p == L::WordJoiner || n == L::WordJoiner || p == L::Glue {
            return None;
        }
        // LB12a
        if n == L::Glue && !matches!(p, L::Space | L::Hyphen | L::UnambiguousHyphen) {
            return None;
        }
        // LB13
        if matches!(
            n,
            L::Exclamation | L::ClosePunctuation | L::CloseParenthesis | L::BreakSymbols
        ) {
            return None;
        }
        // LB14
        if class_is(before_spaces, &[L::OpenPunctuation]) {
            return None;
        }
        // LB15a
        if let Some(index) = before_spaces_index
            && let Some(quote) = self.items.get(index)
            && quote.class == L::Quotation
            && quote.initial_quote
        {
            let opener = self.before(index, 1);
            if opener.is_none()
                || class_is(
                    opener,
                    &[
                        L::MandatoryBreak,
                        L::CarriageReturn,
                        L::LineFeed,
                        L::NextLine,
                        L::OpenPunctuation,
                        L::Quotation,
                        L::Glue,
                        L::Space,
                        L::ZWSpace,
                    ],
                )
            {
                return None;
            }
        }
        // LB15b
        if n == L::Quotation
            && next.final_quote
            && (after_next.is_none()
                || class_is(
                    after_next,
                    &[
                        L::Space,
                        L::Glue,
                        L::WordJoiner,
                        L::ClosePunctuation,
                        L::Quotation,
                        L::CloseParenthesis,
                        L::Exclamation,
                        L::InfixNumeric,
                        L::BreakSymbols,
                        L::MandatoryBreak,
                        L::CarriageReturn,
                        L::LineFeed,
                        L::NextLine,
                        L::ZWSpace,
                    ],
                ))
        {
            return None;
        }
        // LB15c
        if p == L::Space && n == L::InfixNumeric && class_is(after_next, &[L::Numeric]) {
            return Some(Break::Allowed);
        }
        // LB15d
        if n == L::InfixNumeric {
            return None;
        }
        // LB16
        if n == L::Nonstarter
            && class_is(before_spaces, &[L::ClosePunctuation, L::CloseParenthesis])
        {
            return None;
        }
        // LB17
        if n == L::BreakBoth && class_is(before_spaces, &[L::BreakBoth]) {
            return None;
        }
        // LB18
        if p == L::Space {
            return Some(Break::Allowed);
        }
        // LB19
        if (n == L::Quotation && !next.initial_quote)
            || (p == L::Quotation && !previous.final_quote)
        {
            return None;
        }
        // LB19a
        if n == L::Quotation
            && (!previous.east_asian || after_next.is_none_or(|item| !item.east_asian))
        {
            return None;
        }
        if p == L::Quotation
            && (!next.east_asian || before_previous.is_none_or(|item| !item.east_asian))
        {
            return None;
        }
        // LB20
        if p == L::ContingentBreak || n == L::ContingentBreak {
            return Some(Break::Allowed);
        }
        let hyphen = matches!(p, L::Hyphen | L::UnambiguousHyphen);
        let letter = |class: LineBreak| matches!(class, L::Alphabetic | L::HebrewLetter);
        // LB20a
        if hyphen
            && letter(n)
            && (before_previous.is_none()
                || class_is(
                    before_previous,
                    &[
                        L::MandatoryBreak,
                        L::CarriageReturn,
                        L::LineFeed,
                        L::NextLine,
                        L::Space,
                        L::ZWSpace,
                        L::ContingentBreak,
                        L::Glue,
                    ],
                ))
        {
            return None;
        }
        // LB21
        if matches!(
            n,
            L::BreakAfter | L::UnambiguousHyphen | L::Hyphen | L::Nonstarter
        ) || p == L::BreakBefore
        {
            return None;
        }
        // LB21a
        if hyphen && n != L::HebrewLetter && class_is(before_previous, &[L::HebrewLetter]) {
            return None;
        }
        // LB21b
        if p == L::BreakSymbols && n == L::HebrewLetter {
            return None;
        }
        // LB22
        if n == L::Inseparable {
            return None;
        }
        // LB23
        if (letter(p) && n == L::Numeric) || (p == L::Numeric && letter(n)) {
            return None;
        }
        // LB23a
        let ideographic =
            |class: LineBreak| matches!(class, L::Ideographic | L::EBase | L::EModifier);
        if (p == L::PrefixNumeric && ideographic(n)) || (ideographic(p) && n == L::PostfixNumeric) {
            return None;
        }
        // LB24
        let affix = |class: LineBreak| matches!(class, L::PrefixNumeric | L::PostfixNumeric);
        if (affix(p) && letter(n)) || (letter(p) && affix(n)) {
            return None;
        }
        // LB25: Eine Zahl, auf die nur noch SY und IS folgen, endet an
        // `index` oder davor.
        let number_until = |index: Option<usize>| {
            let found = Self::skipping(&self.without_separators, index);
            class_is(found.and_then(|found| self.items.get(found)), &[L::Numeric])
        };
        if affix(n)
            && (number_until(position.checked_sub(1))
                || (matches!(p, L::ClosePunctuation | L::CloseParenthesis)
                    && number_until(position.checked_sub(2))))
        {
            return None;
        }
        if affix(p)
            && n == L::OpenPunctuation
            && (class_is(after_next, &[L::Numeric])
                || (class_is(after_next, &[L::InfixNumeric])
                    && class_is(self.after(position, 2), &[L::Numeric])))
        {
            return None;
        }
        if n == L::Numeric
            && (affix(p)
                || matches!(p, L::Hyphen | L::InfixNumeric)
                || number_until(position.checked_sub(1)))
        {
            return None;
        }
        // LB26
        if (p == L::JL && matches!(n, L::JL | L::JV | L::H2 | L::H3))
            || (matches!(p, L::JV | L::H2) && matches!(n, L::JV | L::JT))
            || (matches!(p, L::JT | L::H3) && n == L::JT)
        {
            return None;
        }
        // LB27
        let korean = |class: LineBreak| matches!(class, L::JL | L::JV | L::JT | L::H2 | L::H3);
        if (korean(p) && n == L::PostfixNumeric) || (p == L::PrefixNumeric && korean(n)) {
            return None;
        }
        // LB28
        if letter(p) && letter(n) {
            return None;
        }
        // LB28a
        let aksara =
            |item: &Item| matches!(item.class, L::Aksara | L::AksaraStart) || item.dotted_circle;
        if (p == L::AksaraPrebase && aksara(next))
            || (aksara(previous) && matches!(n, L::ViramaFinal | L::Virama))
            || (p == L::Virama
                && before_previous.is_some_and(aksara)
                && (n == L::Aksara || next.dotted_circle))
            || (aksara(previous) && aksara(next) && class_is(after_next, &[L::ViramaFinal]))
        {
            return None;
        }
        // LB29
        if p == L::InfixNumeric && letter(n) {
            return None;
        }
        // LB30
        let operand = |class: LineBreak| letter(class) || class == L::Numeric;
        if (operand(p) && n == L::OpenPunctuation && !next.east_asian)
            || (p == L::CloseParenthesis && !previous.east_asian && operand(n))
        {
            return None;
        }
        // LB30a: nur nach einer geraden Zahl von RI umbrechen.
        if p == L::RegionalIndicator && n == L::RegionalIndicator {
            let run = self
                .regional_run
                .get(position.checked_sub(1)?)
                .copied()
                .unwrap_or_default();
            return (run % 2 == 0).then_some(Break::Allowed);
        }
        // LB30b
        if n == L::EModifier && (p == L::EBase || previous.unassigned_pictographic) {
            return None;
        }
        // LB31
        Some(Break::Allowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn breaks(text: &str) -> Vec<(usize, Break)> {
        let code_points: Vec<u32> = text.chars().map(u32::from).collect();
        line_breaks(&code_points)
    }

    #[test]
    fn kinds_of_breaks() {
        use Break::{Allowed, Mandatory};
        assert_eq!(breaks(""), []);
        assert_eq!(breaks("a"), [(1, Allowed)]);
        assert_eq!(breaks("ab cd"), [(3, Allowed), (5, Allowed)]);
        assert_eq!(breaks("a\r\nb"), [(3, Mandatory), (4, Allowed)]);
        assert_eq!(breaks("a\rb\n"), [(2, Mandatory), (4, Mandatory)]);
        assert_eq!(breaks("a\u{2028}b"), [(2, Mandatory), (3, Allowed)]);
        // Kein Umbruch vor Satzzeichen. Nach einem Bindestrich darf
        // umbrochen werden, nicht aber vor einer Ziffer (LB25).
        assert_eq!(breaks("Ja, gut!"), [(4, Allowed), (8, Allowed)]);
        assert_eq!(breaks("Hans-Peter"), [(5, Allowed), (10, Allowed)]);
        assert_eq!(breaks("-5"), [(2, Allowed)]);
        // Ideogramme lassen sich überall umbrechen.
        assert_eq!(breaks("漢字"), [(1, Allowed), (2, Allowed)]);
        // Kombinierende Zeichen hängen an ihrer Basis (LB9).
        assert_eq!(breaks("a\u{308} b"), [(3, Allowed), (4, Allowed)]);
    }

    #[test]
    fn long_runs_of_spaces() {
        let mut code_points = vec![u32::from('(')];
        code_points.extend(std::iter::repeat_n(0x20, 100_000));
        code_points.push(u32::from('a'));
        // LB14: nach der öffnenden Klammer auch über alle Leerzeichen hinweg
        // kein Umbruch.
        assert_eq!(line_breaks(&code_points), [(100_002, Break::Allowed)]);
    }

    /// `LineBreakTest.txt` aus den Unicode-Werkzeugen: `÷` markiert eine
    /// Umbruchstelle, `×` eine Stelle ohne.
    #[test]
    fn line_break_test_suite() {
        let text = testdata::read(
            "unicodetools",
            "unicodetools/data/ucd/18.0.0/auxiliary/LineBreakTest.txt",
        );
        let mut cases = 0;
        for line in text.lines() {
            let data = line.split('#').next().unwrap_or_default();
            if data.trim().is_empty() {
                continue;
            }
            let mut tokens = data.split_whitespace();
            assert_eq!(tokens.next(), Some("×"), "{line}");
            let mut code_points = Vec::new();
            let mut expected = Vec::new();
            while let (Some(code_point), Some(mark)) = (tokens.next(), tokens.next()) {
                code_points.push(u32::from_str_radix(code_point, 16).expect("Codepunkt"));
                match mark {
                    "÷" => expected.push(code_points.len()),
                    "×" => {}
                    _ => panic!("unbekannte Markierung {mark:?}: {line}"),
                }
            }
            let actual: Vec<usize> = line_breaks(&code_points)
                .into_iter()
                .map(|(index, _)| index)
                .collect();
            assert_eq!(actual, expected, "{line}");
            cases += 1;
        }
        assert!(cases > 19_000, "nur {cases} Fälle gelesen");
    }
}
