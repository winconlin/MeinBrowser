//! Der Inline-Formatierungskontext: Leerraum, Formen, Zeilenumbruch und
//! Zeilenkästen nach CSS Text 4 (Abschnitte 4 und 5), CSS 2 (Abschnitte
//! 10.6.1 und 10.8) und CSS Inline 3.
//!
//! - Leerraum, Phase I, über den ganzen Kontext und über die Grenzen von
//!   Inline-Kästen hinweg. Ein Zeilenumbruch im Quelltext (*segment break*)
//!   wird zum Leerzeichen; neben U+200B fällt er weg. Die Regel für
//!   ostasiatische Zeichen fehlt (dafür fehlt die Eigenschaft Script).
//! - Umbruchstellen nach UAX #14, um Inline-Kästen auf Atomebene nach CSS
//!   Text 3 (Abschnitt 5.1): davor und danach erlaubt, außer neben GL, WJ
//!   und ZWJ, wobei U+00A0 nicht zählt. Ob an einer Stelle umbrochen werden
//!   darf, entscheidet `text-wrap-mode` des Zeichens davor, wenn es ein
//!   Leerzeichen ist, sonst des nächsten gemeinsamen Vorfahren. Zwingend
//!   sind nur Umbrüche nach `<br>` und nach erhaltenen Zeilenumbrüchen.
//! - Zeilen werden gierig gefüllt. Leerzeichen, die zusammenfallen, entfallen
//!   am Zeilenanfang und am Zeilenende; erhaltene Leerzeichen am Zeilenende
//!   hängen immer (auch vor einem erzwungenen Umbruch, wo sie nur bedingt
//!   hängen sollten). Markierungen von Inline-Kästen an einer Umbruchstelle:
//!   Enden bleiben auf der Zeile davor, Anfänge kommen auf die nächste.
//! - Zeilenhöhe nach CSS 2, Abschnitt 10.8: Jeder Inline-Kasten trägt seine
//!   `line-height` mit dem halben Durchschuss bei, gemessen an der ersten
//!   verfügbaren Schrift; Kästen auf Atomebene ihren Außenabstandsbereich.
//!   `top` und `bottom` werden wie in WebKit eingeordnet: Ist ein so
//!   ausgerichteter Teilbaum höher als die übrige Zeile, wächst sie nach
//!   unten (`top`) oder oben (`bottom`). Zeilen ohne Text, ohne Kästen auf
//!   Atomebene, ohne erzwungenen Umbruch und ohne Inline-Kasten, der auf
//!   der Zeile links oder rechts Rand, Innen- oder Außenabstand hat, gibt es
//!   nicht (*phantom line boxes*, CSS Inline 3; CSS 2, Abschnitt 9.4.2,
//!   zählt auch oben und unten, die WPT folgen CSS Inline 3).
//! - Fehlt noch: Bidi, `text-transform`, Silbentrennung, `overflow-wrap`,
//!   `word-break`, `line-break`, Tabstopps (ein erhaltener Tabulator ist acht
//!   Leerzeichen breit), `letter-spacing` am Zeilenende wird nicht entfernt.

use mb_style::properties::{LineHeight, Longhand, WhiteSpaceCollapse};
use mb_text::Glyph;
use mb_unicode::LineBreak;
use mb_unicode::line_breaking::{Break, line_breaks};

use crate::LayoutContext;
use crate::block::{AtomicLayout, layout_atomic};
use crate::boxes::{InlineContent, InlineItem, StyleRef};
use crate::fonts::{Metrics, Query};
use crate::fragment::{Fragment, GlyphRun, InlineFragment, Rect, TextFragment};
use crate::style::{Align, Sides, Style, VerticalAlign, resolve};

const LINE_FEED: u32 = 0x0A;
const TAB: u32 = 0x09;
const SPACE: u32 = 0x20;
const ZERO_WIDTH_SPACE: u32 = 0x200B;
const NO_BREAK_SPACE: u32 = 0xA0;
const OBJECT_REPLACEMENT: u32 = 0xFFFC;

/// Der Blockcontainer eines Inline-Formatierungskontexts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Container<'a> {
    pub(crate) style: &'a StyleRef,
    /// Die Breite des Inhaltsbereichs für Prozente; bei den Größen aus dem
    /// Inhalt null.
    pub(crate) width: f64,
    /// Die Höhe des Inhaltsbereichs, wenn sie feststeht.
    pub(crate) height: Option<f64>,
    /// Ob die erste Zeile die erste formatierte Zeile eines Elements ist;
    /// nur dann gilt `text-indent` für sie. Die erste Zeile eines anonymen
    /// Blocks ist es nur, wenn er das erste Kind ist (CSS 2, Abschnitt
    /// 16.1).
    pub(crate) first_formatted_line: bool,
}

/// Wofür die Zeilen gebrochen werden.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Sizing {
    /// Für eine verfügbare Breite.
    Available(f64),
    /// Für die kleinste Breite ohne Überlauf außer an unteilbaren Stellen.
    MinContent,
    /// Für die Breite ohne weichen Umbruch.
    MaxContent,
}

impl Sizing {
    fn width(self) -> f64 {
        match self {
            Sizing::Available(width) => width,
            Sizing::MinContent => 0.0,
            Sizing::MaxContent => f64::INFINITY,
        }
    }
}

/// Das Ergebnis des Inline-Layouts.
#[derive(Debug, Default)]
pub(crate) struct InlineLayout {
    /// Relativ zum Inhaltsbereich des Blockcontainers.
    pub(crate) fragments: Vec<Fragment>,
    pub(crate) height: f64,
    /// Ob es Zeilen gibt (*in-flow line boxes*).
    pub(crate) has_lines: bool,
    pub(crate) first_baseline: Option<f64>,
    pub(crate) last_baseline: Option<f64>,
    /// Die breiteste Zeile, für die Größen aus dem Inhalt.
    pub(crate) width: f64,
    /// Die Zeilen mit ihrem Inhalt, zusammengefasst; `None` ohne Zeilen.
    pub(crate) lines: Option<Rect>,
}

/// Ein Zeichen nach Phase I.
#[derive(Debug)]
struct Char {
    code_point: u32,
    /// Der Eintrag in `InlineContent::items`, zu dem es gehört.
    item: usize,
    mode: WhiteSpaceCollapse,
    /// Ein Leerzeichen, das zusammenfällt.
    collapsible: bool,
    /// Ein erzwungener Umbruch danach.
    forced: bool,
    advance: f64,
    glyphs: Vec<Glyph>,
}

impl Char {
    /// Ein erhaltenes Leerzeichen, das am Zeilenende hängt.
    fn hangs(&self) -> bool {
        self.code_point == SPACE
            && matches!(
                self.mode,
                WhiteSpaceCollapse::Preserve | WhiteSpaceCollapse::PreserveSpaces
            )
    }

    fn is_word_separator(&self) -> bool {
        matches!(
            self.code_point,
            0x20 | 0xA0 | 0x1361 | 0x1_0100 | 0x1_0101 | 0x1_039F | 0x1_091F
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Piece {
    Char(usize),
    Start(usize),
    End(usize),
}

/// Ein Zeichen vor Phase I.
struct Raw {
    code_point: u32,
    item: usize,
    mode: WhiteSpaceCollapse,
    /// Ein Zeilenumbruch aus dem Text, kein `<br>`.
    segment_break: bool,
    removed: bool,
}

impl Raw {
    fn collapsible(&self) -> bool {
        matches!(
            self.mode,
            WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::PreserveBreaks
        )
    }

    fn is_collapsible_space_or_tab(&self) -> bool {
        self.collapsible() && matches!(self.code_point, SPACE | TAB)
    }
}

/// Der vorbereitete Inhalt: Zeichen, Markierungen, Breiten und
/// Umbruchstellen.
struct Prepared<'a> {
    content: &'a InlineContent,
    container: &'a StyleRef,
    chars: Vec<Char>,
    sequence: Vec<Piece>,
    /// Die Stelle jedes Zeichens in `sequence`.
    positions: Vec<usize>,
    /// Die Breite jedes Teils von `sequence`.
    widths: Vec<f64>,
    /// Die Kästen auf Atomebene, nach dem Eintrag.
    atomics: Vec<(usize, AtomicLayout)>,
    /// Rand, Innen- und Außenabstand der Inline-Kästen am Anfang und Ende.
    edges: Vec<BoxEdges>,
}

/// Die waagrechten Maße eines Inline-Kastens.
#[derive(Clone, Copy, Debug, Default)]
struct BoxEdges {
    margin: Sides,
    border: Sides,
    padding: Sides,
}

impl BoxEdges {
    fn start(&self) -> f64 {
        self.margin.left + self.border.left + self.padding.left
    }

    fn end(&self) -> f64 {
        self.margin.right + self.border.right + self.padding.right
    }

    /// Ob am Anfang Rand, Innen- oder Außenabstand nicht null ist.
    fn starts_nonzero(&self) -> bool {
        self.margin.left != 0.0 || self.border.left != 0.0 || self.padding.left != 0.0
    }

    /// Ob am Ende Rand, Innen- oder Außenabstand nicht null ist.
    fn ends_nonzero(&self) -> bool {
        self.margin.right != 0.0 || self.border.right != 0.0 || self.padding.right != 0.0
    }
}

impl<'a> Prepared<'a> {
    /// Der Stil, in dem ein Eintrag steht.
    fn parent_style(&self, parent: Option<usize>) -> &'a StyleRef {
        match parent.and_then(|index| self.content.boxes.get(index)) {
            Some(data) => &data.style,
            None => self.container,
        }
    }

    fn item_parent(&self, item: usize) -> Option<usize> {
        match self.content.items.get(item) {
            Some(
                InlineItem::Text { parent, .. }
                | InlineItem::Atomic { parent, .. }
                | InlineItem::ForcedBreak { parent },
            ) => *parent,
            _ => None,
        }
    }

    /// Die Kette der Inline-Kästen um einen Eintrag, von innen nach außen.
    fn ancestors(&self, item: usize) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut current = self.item_parent(item);
        while let Some(index) = current {
            if chain.contains(&index) {
                break;
            }
            chain.push(index);
            current = self.content.boxes.get(index).and_then(|data| data.parent);
        }
        chain
    }

    fn new(
        context: &LayoutContext<'_>,
        content: &'a InlineContent,
        container: Container<'a>,
        sizing: Sizing,
        depth: usize,
    ) -> Prepared<'a> {
        let containing_width = container.width;
        let mut prepared = Prepared {
            content,
            container: container.style,
            chars: Vec::new(),
            sequence: Vec::new(),
            positions: Vec::new(),
            widths: Vec::new(),
            atomics: Vec::new(),
            edges: Vec::new(),
        };
        prepared.edges = content
            .boxes
            .iter()
            .map(|data| BoxEdges {
                margin: data.style.margins_or_zero(containing_width),
                border: data.style.border(),
                padding: data.style.padding(containing_width),
            })
            .collect();
        prepared.collapse_white_space();
        prepared.shape(context);
        for (index, item) in content.items.iter().enumerate() {
            if let InlineItem::Atomic { atomic, .. } = item {
                let layout = layout_atomic(
                    context,
                    atomic,
                    containing_width,
                    container.height,
                    sizing,
                    depth,
                );
                prepared.atomics.push((index, layout));
            }
        }
        prepared.measure();
        prepared
    }

    /// Phase I der Leerraumverarbeitung.
    fn collapse_white_space(&mut self) {
        let mut raw: Vec<Raw> = Vec::new();
        let mut sequence: Vec<Option<Piece>> = Vec::new();
        for (index, item) in self.content.items.iter().enumerate() {
            match item {
                InlineItem::Start(data) => sequence.push(Some(Piece::Start(*data))),
                InlineItem::End(data) => sequence.push(Some(Piece::End(*data))),
                InlineItem::Text { text, parent } => {
                    let mode = self.parent_style(*parent).white_space_collapse();
                    for &code_point in text {
                        // CR LF ist hier schon zu LF geworden; ein CR allein
                        // gilt wie in HTML als Zeilenumbruch.
                        let code_point = if code_point == 0x0D {
                            LINE_FEED
                        } else {
                            code_point
                        };
                        sequence.push(None);
                        raw.push(Raw {
                            code_point,
                            item: index,
                            mode,
                            segment_break: code_point == LINE_FEED,
                            removed: false,
                        });
                    }
                }
                InlineItem::Atomic { .. } => {
                    sequence.push(None);
                    raw.push(Raw {
                        code_point: OBJECT_REPLACEMENT,
                        item: index,
                        mode: WhiteSpaceCollapse::Preserve,
                        segment_break: false,
                        removed: false,
                    });
                }
                InlineItem::ForcedBreak { .. } => {
                    sequence.push(None);
                    raw.push(Raw {
                        code_point: LINE_FEED,
                        item: index,
                        mode: WhiteSpaceCollapse::Preserve,
                        segment_break: false,
                        removed: false,
                    });
                }
            }
        }

        // Leerzeichen und Tabulatoren vor und nach einem Zeilenumbruch.
        let count = raw.len();
        for index in 0..count {
            if !raw.get(index).is_some_and(|r| r.segment_break) {
                continue;
            }
            let mut before = index;
            while let Some(previous) = before.checked_sub(1)
                && let Some(r) = raw.get_mut(previous)
                && r.is_collapsible_space_or_tab()
            {
                r.removed = true;
                before = previous;
            }
            let mut after = index.saturating_add(1);
            while let Some(r) = raw.get_mut(after)
                && r.is_collapsible_space_or_tab()
            {
                r.removed = true;
                after = after.saturating_add(1);
            }
        }
        // Zeilenumbrüche, die zusammenfallen: ein Folgeumbruch fällt weg,
        // die übrigen werden zum Leerzeichen, neben U+200B fallen sie weg.
        let mut previous: Option<usize> = None;
        for index in 0..count {
            let Some(r) = raw.get(index) else {
                continue;
            };
            if r.removed {
                continue;
            }
            if r.segment_break && r.mode == WhiteSpaceCollapse::Collapse {
                let after_break = previous
                    .and_then(|p| raw.get(p))
                    .is_some_and(|p| p.segment_break && p.mode == WhiteSpaceCollapse::Collapse);
                let before_zwsp = previous
                    .and_then(|p| raw.get(p))
                    .is_some_and(|p| p.code_point == ZERO_WIDTH_SPACE);
                let after_zwsp = raw
                    .iter()
                    .skip(index.saturating_add(1))
                    .find(|r| !r.removed)
                    .is_some_and(|r| r.code_point == ZERO_WIDTH_SPACE);
                if let Some(r) = raw.get_mut(index) {
                    if after_break || before_zwsp || after_zwsp {
                        r.removed = true;
                        // Der Umbruch davor bleibt der vorige.
                        continue;
                    }
                    r.code_point = SPACE;
                }
            }
            previous = Some(index);
        }
        // Für die Fortsetzung: Ein entfernter Folgeumbruch soll den
        // vorigen nicht verdecken, deshalb merkt sich die Schleife oben den
        // letzten verbliebenen. Nun Tabulatoren und doppelte Leerzeichen.
        let mut last_collapsible_space = false;
        for r in raw.iter_mut() {
            if r.removed {
                continue;
            }
            match r.mode {
                WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::PreserveBreaks => {
                    if r.code_point == TAB {
                        r.code_point = SPACE;
                    }
                    if r.code_point == SPACE {
                        if last_collapsible_space {
                            r.removed = true;
                            continue;
                        }
                        last_collapsible_space = true;
                        continue;
                    }
                }
                WhiteSpaceCollapse::PreserveSpaces => {
                    // Tabulatoren und Zeilenumbrüche aus dem Text werden
                    // Leerzeichen.
                    if r.code_point == TAB || (r.code_point == LINE_FEED && r.segment_break) {
                        r.code_point = SPACE;
                    }
                }
                WhiteSpaceCollapse::Discard => {
                    // `white-space-collapse: discard` entfernt allen
                    // Leerraum (CSS Text 4).
                    if matches!(r.code_point, SPACE | TAB | LINE_FEED) && r.item_is_text() {
                        r.removed = true;
                        continue;
                    }
                }
                WhiteSpaceCollapse::Preserve | WhiteSpaceCollapse::BreakSpaces => {}
            }
            last_collapsible_space = false;
        }

        let mut raw_iter = raw.into_iter();
        for piece in sequence {
            match piece {
                Some(piece) => self.sequence.push(piece),
                None => {
                    let Some(r) = raw_iter.next() else {
                        continue;
                    };
                    if r.removed {
                        continue;
                    }
                    let collapsible = r.code_point == SPACE && r.collapsible();
                    let forced = r.code_point == LINE_FEED
                        && (!r.segment_break
                            || !r.collapsible()
                            || r.mode == WhiteSpaceCollapse::PreserveBreaks);
                    let index = self.chars.len();
                    self.chars.push(Char {
                        code_point: r.code_point,
                        item: r.item,
                        mode: r.mode,
                        collapsible,
                        forced,
                        advance: 0.0,
                        glyphs: Vec::new(),
                    });
                    self.positions.push(self.sequence.len());
                    self.sequence.push(Piece::Char(index));
                }
            }
        }
    }

    /// Formt den Text je Eintrag und rechnet die Abstände dazu.
    fn shape(&mut self, context: &LayoutContext<'_>) {
        let mut start = 0;
        while start < self.chars.len() {
            let Some(item) = self.chars.get(start).map(|c| c.item) else {
                break;
            };
            let mut end = start;
            while self.chars.get(end).is_some_and(|c| c.item == item) {
                end = end.saturating_add(1);
            }
            if let Some(InlineItem::Text { parent, .. }) = self.content.items.get(item) {
                let style = self.parent_style(*parent);
                self.shape_run(context, style, start, end);
            }
            start = end;
        }
    }

    fn shape_run(
        &mut self,
        context: &LayoutContext<'_>,
        style: &StyleRef,
        start: usize,
        end: usize,
    ) {
        let font = style.font();
        let query = Query::new(&font);
        let size = font.size;
        let space = context
            .fonts
            .shape(&query, size, &[SPACE])
            .iter()
            .map(|glyph| f64::from(glyph.advance))
            .sum::<f64>();
        let letter_spacing = style.spacing(Longhand::LetterSpacing, size);
        let word_spacing = style.spacing(Longhand::WordSpacing, space);
        // Teilstücke ohne Zeilenumbrüche und Tabulatoren.
        let mut index = start;
        while index < end {
            let code_point = self.chars.get(index).map_or(0, |c| c.code_point);
            if matches!(code_point, LINE_FEED | TAB) {
                if code_point == TAB
                    && let Some(c) = self.chars.get_mut(index)
                {
                    c.advance = 8.0 * space;
                }
                index = index.saturating_add(1);
                continue;
            }
            let mut stop = index;
            while stop < end
                && !matches!(
                    self.chars.get(stop).map(|c| c.code_point),
                    Some(LINE_FEED | TAB) | None
                )
            {
                stop = stop.saturating_add(1);
            }
            let text: Vec<u32> = self
                .chars
                .get(index..stop)
                .unwrap_or_default()
                .iter()
                .map(|c| c.code_point)
                .collect();
            for glyph in context.fonts.shape(&query, size, &text) {
                if let Some(c) = self.chars.get_mut(index.saturating_add(glyph.cluster)) {
                    c.advance += f64::from(glyph.advance);
                    c.glyphs.push(glyph);
                }
            }
            for c in self.chars.get_mut(index..stop).unwrap_or_default() {
                if !c.glyphs.is_empty() {
                    c.advance += letter_spacing;
                }
                if c.is_word_separator() {
                    c.advance += word_spacing;
                }
            }
            index = stop;
        }
    }

    /// Die Breiten der Teile.
    fn measure(&mut self) {
        let mut widths = Vec::with_capacity(self.sequence.len());
        for piece in &self.sequence {
            let width = match *piece {
                Piece::Char(index) => match self.chars.get(index) {
                    Some(c) if c.code_point == OBJECT_REPLACEMENT => self
                        .atomic(c.item)
                        .map_or(0.0, |layout| layout.margin_width()),
                    Some(c) => c.advance,
                    None => 0.0,
                },
                Piece::Start(data) => self.start_edge(data),
                Piece::End(data) => self.end_edge(data),
            };
            widths.push(width);
        }
        self.widths = widths;
    }

    fn atomic(&self, item: usize) -> Option<&AtomicLayout> {
        self.atomics
            .iter()
            .find(|(index, _)| *index == item)
            .map(|(_, layout)| layout)
    }

    fn start_edge(&self, data: usize) -> f64 {
        match (self.content.boxes.get(data), self.edges.get(data)) {
            (Some(info), Some(edges)) if info.start_edge => edges.start(),
            _ => 0.0,
        }
    }

    fn end_edge(&self, data: usize) -> f64 {
        match (self.content.boxes.get(data), self.edges.get(data)) {
            (Some(info), Some(edges)) if info.end_edge => edges.end(),
            _ => 0.0,
        }
    }

    /// Die Umbruchstellen als Stellen in `sequence`, aufsteigend, mit der
    /// Angabe, ob die Zeile dort enden muss. Das Ende fehlt.
    fn opportunities(&self) -> Vec<(usize, bool)> {
        let text: Vec<u32> = self.chars.iter().map(|c| c.code_point).collect();
        let count = text.len();
        let mut allowed = vec![false; count.saturating_add(1)];
        for (index, kind) in line_breaks(&text) {
            if kind == Break::Allowed
                && let Some(slot) = allowed.get_mut(index)
            {
                *slot = true;
            }
        }
        let suppresses = |code_point: u32| {
            code_point != NO_BREAK_SPACE
                && matches!(
                    mb_unicode::line_break(code_point),
                    LineBreak::Glue | LineBreak::WordJoiner | LineBreak::ZWJ
                )
        };
        for index in 1..count {
            let (Some(before), Some(after)) = (text.get(index - 1), text.get(index)) else {
                continue;
            };
            let atomic_before = *before == OBJECT_REPLACEMENT
                && self
                    .chars
                    .get(index - 1)
                    .is_some_and(|c| self.atomic(c.item).is_some());
            let atomic_after = *after == OBJECT_REPLACEMENT
                && self
                    .chars
                    .get(index)
                    .is_some_and(|c| self.atomic(c.item).is_some());
            if atomic_before || atomic_after {
                let blocked =
                    (atomic_before && suppresses(*after)) || (atomic_after && suppresses(*before));
                if let Some(slot) = allowed.get_mut(index) {
                    *slot = !blocked;
                }
            }
            // `break-spaces`: nach jedem Leerzeichen.
            if self.chars.get(index - 1).is_some_and(|c| {
                c.mode == WhiteSpaceCollapse::BreakSpaces && matches!(c.code_point, SPACE | TAB)
            }) && let Some(slot) = allowed.get_mut(index)
            {
                *slot = true;
            }
        }
        let mut result = Vec::new();
        for index in 1..count {
            let forced = self.chars.get(index - 1).is_some_and(|c| c.forced);
            let soft = allowed.get(index).copied().unwrap_or(false) && self.wraps_at(index);
            if !(forced || soft) {
                continue;
            }
            let Some(&position) = self.positions.get(index) else {
                continue;
            };
            result.push((self.split(position), forced));
        }
        result
    }

    /// Ob an der Stelle vor dem Zeichen `index` weich umbrochen werden darf.
    fn wraps_at(&self, index: usize) -> bool {
        let (Some(before), Some(after)) = (
            self.chars.get(index.saturating_sub(1)),
            self.chars.get(index),
        ) else {
            return true;
        };
        if matches!(before.code_point, SPACE | TAB) || before.code_point == 0x3000 {
            let parent = self.item_parent(before.item);
            return self.parent_style(parent).wraps();
        }
        let first = self.ancestors(before.item);
        let second = self.ancestors(after.item);
        let common = first.iter().find(|index| second.contains(index)).copied();
        self.parent_style(common).wraps()
    }

    /// Die Stelle in `sequence`, an der eine Zeile vor `position` endet:
    /// Anfänge von Inline-Kästen direkt davor gehören zur nächsten Zeile.
    fn split(&self, position: usize) -> usize {
        let mut first_marker = position;
        while let Some(previous) = first_marker.checked_sub(1)
            && matches!(
                self.sequence.get(previous),
                Some(Piece::Start(_) | Piece::End(_))
            )
        {
            first_marker = previous;
        }
        (first_marker..position)
            .find(|&index| matches!(self.sequence.get(index), Some(Piece::Start(_))))
            .unwrap_or(position)
    }

    fn char_at(&self, position: usize) -> Option<&Char> {
        match self.sequence.get(position) {
            Some(Piece::Char(index)) => self.chars.get(*index),
            _ => None,
        }
    }

    /// Wie viele Teile am Anfang und am Ende einer Zeile wegfallen: die
    /// Stellen des ersten und hinter dem letzten Teils, die bleiben.
    fn trimmed(&self, start: usize, end: usize) -> (usize, usize) {
        let mut first = start;
        while first < end {
            match self.sequence.get(first) {
                Some(Piece::Char(_)) => {
                    if self.char_at(first).is_some_and(|c| c.collapsible) {
                        first = first.saturating_add(1);
                        continue;
                    }
                    break;
                }
                Some(_) => first = first.saturating_add(1),
                None => break,
            }
        }
        // Die Markierungen vor dem ersten Zeichen bleiben.
        let mut leading = start;
        for position in start..first {
            if !matches!(self.sequence.get(position), Some(Piece::Char(_))) {
                continue;
            }
            leading = position.saturating_add(1);
        }
        let _ = leading;
        let mut last = end;
        while last > first {
            let previous = last - 1;
            match self.sequence.get(previous) {
                Some(Piece::Char(_)) => {
                    if self
                        .char_at(previous)
                        .is_some_and(|c| c.collapsible || c.hangs())
                    {
                        last = previous;
                        continue;
                    }
                    break;
                }
                Some(_) => last = previous,
                None => break,
            }
        }
        (first, last)
    }

    /// Ob ein Teil der Zeile wegfällt.
    fn is_trimmed(&self, position: usize, first: usize, last: usize) -> bool {
        matches!(self.sequence.get(position), Some(Piece::Char(_)))
            && (position < first || position >= last)
    }

    /// Die Breite einer Zeile ohne die Teile, die wegfallen.
    fn line_width(&self, start: usize, end: usize) -> f64 {
        let (first, last) = self.trimmed(start, end);
        (start..end)
            .filter(|&position| !self.is_trimmed(position, first, last))
            .map(|position| self.widths.get(position).copied().unwrap_or(0.0))
            .sum()
    }
}

impl Raw {
    fn item_is_text(&self) -> bool {
        self.segment_break || self.code_point != OBJECT_REPLACEMENT
    }
}

/// Ein Kasten auf der Zeile für die senkrechte Ausrichtung.
#[derive(Clone, Copy, Debug)]
struct Aligned {
    /// Die Gruppe: `None` für die Zeile, sonst der Kasten mit `top` oder
    /// `bottom`, an dem der Teilbaum hängt.
    group: Option<usize>,
    /// Die Grundlinie relativ zur Grundlinie der Gruppe, nach unten
    /// positiv.
    baseline: f64,
    /// Über und unter der Grundlinie, für die Höhe der Zeile.
    above: f64,
    below: f64,
    metrics: Metrics,
}

/// Was die Ausrichtung über einen Kasten wissen muss.
struct Bounds {
    above: f64,
    below: f64,
    metrics: Metrics,
    line_height: f64,
}

fn line_height_px(style: &StyleRef, metrics: &Metrics) -> f64 {
    match style.line_height() {
        LineHeight::Normal => metrics.normal_line_height(),
        LineHeight::Number(number) => number * style.font_size(),
        LineHeight::LengthPercentage(value) => resolve(value, style.font_size()),
    }
}

/// Die Maße eines Inline-Kastens: `line-height` mit dem halben Durchschuss
/// um Aufstieg und Abstieg der ersten verfügbaren Schrift.
fn inline_bounds(context: &LayoutContext<'_>, style: &StyleRef) -> Bounds {
    let font = style.font();
    let metrics = context.fonts.metrics(&Query::new(&font), font.size);
    let line_height = line_height_px(style, &metrics);
    let leading = (line_height - (metrics.ascent + metrics.descent)) / 2.0;
    Bounds {
        above: metrics.ascent + leading,
        below: metrics.descent + leading,
        metrics,
        line_height,
    }
}

/// Was das Layout eines Inline-Formatierungskontexts ungefähr kostet, in
/// Schritten des Budgets: ein fester Teil für Schriftwahl und
/// Umbruchstellen, einer je Eintrag und einer je 16 Zeichen. So kostet ein
/// Schritt hier ungefähr so viel wie beim Abgleich der Selektoren.
fn work(content: &InlineContent) -> u64 {
    let items = content.items.iter().map(|item| match item {
        InlineItem::Text { text, .. } => text.len() / 16 + 1,
        _ => 1,
    });
    let total = items.fold(32_usize, usize::saturating_add);
    u64::try_from(total).unwrap_or(u64::MAX)
}

/// Lays out an inline formatting context.
pub(crate) fn layout(
    context: &LayoutContext<'_>,
    content: &InlineContent,
    container: Container<'_>,
    sizing: Sizing,
    depth: usize,
) -> InlineLayout {
    if !context.budget.spend(work(content)) {
        return InlineLayout::default();
    }
    let prepared = Prepared::new(context, content, container, sizing, depth);
    let opportunities = prepared.opportunities();
    let available = sizing.width();
    let indent = container.style.text_indent();
    let indent_px = indent.map_or(0.0, |indent| resolve(indent.length, container.width));

    let mut result = InlineLayout::default();
    let mut start = 0;
    let mut first_line = container.first_formatted_line;
    let mut after_forced = false;
    let mut open: Vec<usize> = Vec::new();
    let mut y = 0.0;
    let len = prepared.sequence.len();
    let mut candidates = opportunities.iter().peekable();
    while start < len {
        // Einrücken: die erste Zeile, mit `each-line` auch jede nach einem
        // erzwungenen Umbruch, mit `hanging` alle anderen.
        let indented = match indent {
            Some(indent) => {
                let base = first_line || (indent.each_line && after_forced);
                base != indent.hanging
            }
            None => false,
        };
        let line_indent = if indented { indent_px } else { 0.0 };
        let mut chosen: Option<(usize, bool)> = None;
        while let Some(&&(boundary, forced)) = candidates.peek() {
            if boundary <= start {
                candidates.next();
                continue;
            }
            let width = prepared.line_width(start, boundary) + line_indent;
            if width <= available || chosen.is_none() {
                if width > available {
                    // Nichts passt: die erste Stelle, mit Überlauf.
                    chosen = Some((boundary, forced));
                    candidates.next();
                    break;
                }
                chosen = Some((boundary, forced));
                candidates.next();
                if forced {
                    break;
                }
                continue;
            }
            break;
        }
        // Passt der Rest bis zum Ende, endet die Zeile dort.
        let (end, forced) = match chosen {
            Some((boundary, true)) => (boundary, true),
            Some((boundary, false)) => {
                let rest = prepared.line_width(start, len) + line_indent;
                let next_forced = candidates.peek().is_some_and(|&&(_, forced)| forced);
                if rest <= available && !next_forced && candidates.peek().is_none() {
                    (len, false)
                } else {
                    (boundary, false)
                }
            }
            None => (len, false),
        };
        let last = end >= len || forced;
        let line = place_line(
            context,
            &prepared,
            LineInput {
                start,
                end,
                open: &open,
                indent: line_indent,
                available,
                last,
                y,
            },
        );
        // Kästen, die über das Zeilenende hinausgehen.
        for position in start..end {
            match prepared.sequence.get(position) {
                Some(Piece::Start(data)) => open.push(*data),
                Some(Piece::End(data)) => {
                    if let Some(found) = open.iter().rposition(|open| open == data) {
                        open.remove(found);
                    }
                }
                _ => {}
            }
        }
        if let Some(line) = line {
            result.width = result.width.max(line.width);
            if result.first_baseline.is_none() {
                result.first_baseline = Some(line.baseline);
            }
            result.last_baseline = Some(line.baseline);
            result.has_lines = true;
            result.fragments.extend(line.fragments);
            // Die Zeile reicht über den Inhaltsbereich und so weit, wie ihr
            // Inhalt über ihn hinausragt.
            let (left, right) = line.extent;
            let left = left.min(0.0);
            let right = if available.is_finite() {
                right.max(available)
            } else {
                right
            };
            if line.height > 0.0 && right > left {
                let rect = Rect::new(left, y, right - left, line.height);
                result.lines = Some(result.lines.map_or(rect, |lines| lines.union(&rect)));
            }
            y += line.height;
        }
        first_line = false;
        after_forced = forced;
        start = end;
    }
    result.height = y;
    result
}

struct LineInput<'a> {
    start: usize,
    end: usize,
    /// Die Inline-Kästen, die schon auf einer früheren Zeile begannen.
    open: &'a [usize],
    indent: f64,
    available: f64,
    /// Die letzte Zeile oder eine vor einem erzwungenen Umbruch.
    last: bool,
    y: f64,
}

struct Line {
    fragments: Vec<Fragment>,
    height: f64,
    baseline: f64,
    width: f64,
    /// Wo der Inhalt waagrecht beginnt und endet.
    extent: (f64, f64),
}

/// Ein Inline-Kasten auf der Zeile, waagrecht.
struct PlacedBox {
    data: usize,
    left: f64,
    right: f64,
    has_start: bool,
    has_end: bool,
}

/// Baut eine Zeile; `None`, wenn es sie nicht gibt.
fn place_line(
    context: &LayoutContext<'_>,
    prepared: &Prepared<'_>,
    input: LineInput<'_>,
) -> Option<Line> {
    let LineInput {
        start,
        end,
        open,
        indent,
        available,
        last,
        y,
    } = input;
    let (first, last_kept) = prepared.trimmed(start, end);

    // Gibt es die Zeile?
    let has_content = (start..end).any(|position| match prepared.sequence.get(position) {
        Some(Piece::Char(index)) => {
            !prepared.is_trimmed(position, first, last_kept)
                || prepared.chars.get(*index).is_some_and(|c| c.forced)
        }
        Some(Piece::Start(data)) => {
            prepared
                .content
                .boxes
                .get(*data)
                .is_some_and(|b| b.start_edge)
                && prepared
                    .edges
                    .get(*data)
                    .is_some_and(BoxEdges::starts_nonzero)
        }
        Some(Piece::End(data)) => {
            prepared
                .content
                .boxes
                .get(*data)
                .is_some_and(|b| b.end_edge)
                && prepared
                    .edges
                    .get(*data)
                    .is_some_and(BoxEdges::ends_nonzero)
        }
        None => false,
    });
    if !has_content {
        return None;
    }

    // Waagrecht: Breite und Ausrichtung.
    let width: f64 = (start..end)
        .filter(|&position| !prepared.is_trimmed(position, first, last_kept))
        .map(|position| prepared.widths.get(position).copied().unwrap_or(0.0))
        .sum::<f64>()
        + indent;
    let container = prepared.container;
    let align = if last {
        match container.text_align_last() {
            Some(align) => align,
            None => match container.text_align() {
                Align::Justify => {
                    container.align_keyword(mb_style::properties::TextAlignKeyword::Start)
                }
                other => other,
            },
        }
    } else {
        container.text_align()
    };
    let free = available - width;
    let separators = (start..end)
        .filter(|&position| !prepared.is_trimmed(position, first, last_kept))
        .filter(|&position| {
            prepared
                .char_at(position)
                .is_some_and(Char::is_word_separator)
        })
        .count();
    // Am Zeilenanfang ausgerichtet; eine zu lange Zeile läuft so über das
    // Ende hinaus (CSS Text 3, Abschnitt 6.1).
    let start_offset =
        if container.align_keyword(mb_style::properties::TextAlignKeyword::Start) == Align::Right {
            free
        } else {
            0.0
        };
    let (offset, extra) = if !free.is_finite() {
        (0.0, 0.0)
    } else if free <= 0.0 {
        (start_offset, 0.0)
    } else {
        match align {
            Align::Left => (0.0, 0.0),
            Align::Right => (free, 0.0),
            Align::Center => (free / 2.0, 0.0),
            Align::Justify if separators > 0 => (0.0, free / separators as f64),
            Align::Justify => (start_offset, 0.0),
        }
    };

    // Waagrechte Stellen der Teile.
    let mut x = indent + offset;
    // Negative Abstände führen zurück; der Inhalt reicht so weit, wie `x`
    // irgendwo kam, nicht nur bis zum Zeilenende.
    let mut extent = (x, x);
    let mut positions: Vec<f64> = Vec::with_capacity(end.saturating_sub(start));
    let mut placed: Vec<PlacedBox> = open
        .iter()
        .map(|&data| PlacedBox {
            data,
            left: x,
            right: x,
            has_start: false,
            has_end: false,
        })
        .collect();
    for position in start..end {
        positions.push(x);
        if prepared.is_trimmed(position, first, last_kept) {
            continue;
        }
        let width = prepared.widths.get(position).copied().unwrap_or(0.0);
        match prepared.sequence.get(position) {
            Some(Piece::Start(data)) => {
                let edges = prepared.edges.get(*data).copied().unwrap_or_default();
                let has_start = prepared
                    .content
                    .boxes
                    .get(*data)
                    .is_some_and(|b| b.start_edge);
                let left = if has_start { x + edges.margin.left } else { x };
                placed.push(PlacedBox {
                    data: *data,
                    left,
                    right: left,
                    has_start,
                    has_end: false,
                });
                x += width;
            }
            Some(Piece::End(data)) => {
                let edges = prepared.edges.get(*data).copied().unwrap_or_default();
                let has_end = prepared
                    .content
                    .boxes
                    .get(*data)
                    .is_some_and(|b| b.end_edge);
                let right = if has_end {
                    x + width - edges.margin.right
                } else {
                    x
                };
                if let Some(found) = placed.iter_mut().rev().find(|b| b.data == *data) {
                    found.right = right;
                    found.has_end = has_end;
                }
                x += width;
            }
            Some(Piece::Char(_)) => {
                x += width;
                if prepared
                    .char_at(position)
                    .is_some_and(Char::is_word_separator)
                {
                    x += extra;
                }
            }
            None => {}
        }
        extent = (extent.0.min(x), extent.1.max(x));
    }
    let line_end = x;
    for placed_box in placed.iter_mut() {
        if !placed_box.has_end {
            placed_box.right = line_end.max(placed_box.left);
        }
    }

    // Senkrecht.
    let root_bounds = inline_bounds(context, container);
    let mut aligned: Vec<Option<Aligned>> = vec![None; prepared.content.boxes.len()];
    let mut atomic_aligned: Vec<(usize, Aligned)> = Vec::new();
    let root = Aligned {
        group: None,
        baseline: 0.0,
        above: root_bounds.above,
        below: root_bounds.below,
        metrics: root_bounds.metrics,
    };
    let parent_of = |data: usize| prepared.content.boxes.get(data).and_then(|b| b.parent);
    // Ordnet einen Kasten an seinem Elternkasten aus.
    let align_child = |parent: &Aligned,
                       parent_style: &StyleRef,
                       style: &StyleRef,
                       group_if_top: usize,
                       above: f64,
                       below: f64,
                       line_height: f64,
                       metrics: Metrics| {
        let parent_font_size = parent_style.font_size();
        match style.vertical_align(line_height, parent_font_size) {
            VerticalAlign::Shift(shift) => Aligned {
                group: parent.group,
                baseline: parent.baseline - shift,
                above,
                below,
                metrics,
            },
            VerticalAlign::TextTop => Aligned {
                group: parent.group,
                baseline: parent.baseline - parent.metrics.ascent + above,
                above,
                below,
                metrics,
            },
            VerticalAlign::TextBottom => Aligned {
                group: parent.group,
                baseline: parent.baseline + parent.metrics.descent - below,
                above,
                below,
                metrics,
            },
            VerticalAlign::Middle => {
                let x_height = parent.metrics.x_height.unwrap_or(parent_font_size / 2.0);
                let center = parent.baseline - x_height / 2.0;
                Aligned {
                    group: parent.group,
                    baseline: center - (below - above) / 2.0,
                    above,
                    below,
                    metrics,
                }
            }
            VerticalAlign::Top | VerticalAlign::Bottom => Aligned {
                group: Some(group_if_top),
                baseline: 0.0,
                above,
                below,
                metrics,
            },
        }
    };
    // Gruppen: die Kästen mit `top` oder `bottom`, als Einträge der
    // Kästen (Inline-Kästen) oder als `boxes.len() + Eintrag`
    // (Atomebene).
    let atomic_group = |item: usize| prepared.content.boxes.len().saturating_add(item);
    let mut ordered_boxes: Vec<usize> = open.to_vec();
    for position in start..end {
        if let Some(Piece::Start(data)) = prepared.sequence.get(position) {
            ordered_boxes.push(*data);
        }
    }
    for &data in &ordered_boxes {
        let Some(info) = prepared.content.boxes.get(data) else {
            continue;
        };
        let parent = parent_of(data);
        let parent_aligned = parent
            .and_then(|p| aligned.get(p).copied().flatten())
            .unwrap_or(root);
        let parent_style = prepared.parent_style(parent);
        let bounds = inline_bounds(context, &info.style);
        let result = align_child(
            &parent_aligned,
            parent_style,
            &info.style,
            data,
            bounds.above,
            bounds.below,
            bounds.line_height,
            bounds.metrics,
        );
        if let Some(slot) = aligned.get_mut(data) {
            *slot = Some(result);
        }
    }
    for position in start..end {
        let Some(c) = prepared.char_at(position) else {
            continue;
        };
        let Some(layout) = prepared.atomic(c.item) else {
            continue;
        };
        let parent = prepared.item_parent(c.item);
        let parent_aligned = parent
            .and_then(|p| aligned.get(p).copied().flatten())
            .unwrap_or(root);
        let parent_style = prepared.parent_style(parent);
        let style = match prepared.content.items.get(c.item) {
            Some(InlineItem::Atomic { atomic, .. }) => atomic.style().clone(),
            _ => continue,
        };
        let above = layout.baseline;
        let below = layout.margin_height() - layout.baseline;
        let metrics = parent_aligned.metrics;
        // Die Grundlage für Prozente in `vertical-align` (CSS 2, Abschnitt
        // 10.8.1); zur Zeile trägt der Kasten seine Höhe bei.
        let line_height = inline_bounds(context, &style).line_height;
        let result = align_child(
            &parent_aligned,
            parent_style,
            &style,
            atomic_group(c.item),
            above,
            below,
            line_height,
            metrics,
        );
        atomic_aligned.push((c.item, result));
    }
    // Die Ausdehnung jeder Gruppe.
    let mut extents: Vec<(Option<usize>, f64, f64)> = vec![(None, -root.above, root.below)];
    let all = aligned
        .iter()
        .enumerate()
        .filter(|(data, _)| ordered_boxes.contains(data))
        .filter_map(|(data, a)| a.map(|a| (Some(data), a)))
        .chain(
            atomic_aligned
                .iter()
                .map(|(item, a)| (Some(atomic_group(*item)), *a)),
        )
        .collect::<Vec<_>>();
    for (_, a) in &all {
        let top = a.baseline - a.above;
        let bottom = a.baseline + a.below;
        match extents.iter_mut().find(|(group, _, _)| *group == a.group) {
            Some(extent) => {
                extent.1 = extent.1.min(top);
                extent.2 = extent.2.max(bottom);
            }
            None => extents.push((a.group, top, bottom)),
        }
    }
    let (_, mut line_top, mut line_bottom) = extents.first().copied().unwrap_or((None, 0.0, 0.0));
    // Die Gruppen mit `top` und `bottom`, in der Reihenfolge der Zeile.
    let group_kind = |group: usize| -> Option<VerticalAlign> {
        if group < prepared.content.boxes.len() {
            let style = &prepared.content.boxes.get(group)?.style;
            Some(style.vertical_align(0.0, 0.0))
        } else {
            let item = group - prepared.content.boxes.len();
            match prepared.content.items.get(item) {
                Some(InlineItem::Atomic { atomic, .. }) => {
                    Some(atomic.style().vertical_align(0.0, 0.0))
                }
                _ => None,
            }
        }
    };
    for &(group, top, bottom) in extents.iter().skip(1) {
        let Some(group) = group else {
            continue;
        };
        let height = bottom - top;
        if height > line_bottom - line_top {
            match group_kind(group) {
                Some(VerticalAlign::Bottom) => line_top = line_bottom - height,
                _ => line_bottom = line_top + height,
            }
        }
    }
    let line_height = (line_bottom - line_top).max(0.0);
    let root_baseline = y - line_top;
    // Die Grundlinie eines Kastens in den Koordinaten des Blocks.
    let baseline_of = |a: &Aligned| -> f64 {
        match a.group {
            None => root_baseline + a.baseline,
            Some(group) => {
                let (_, top, bottom) = extents
                    .iter()
                    .find(|(g, _, _)| *g == Some(group))
                    .copied()
                    .unwrap_or((None, 0.0, 0.0));
                let group_baseline = match group_kind(group) {
                    Some(VerticalAlign::Bottom) => y + line_height - bottom,
                    _ => y - top,
                };
                group_baseline + a.baseline
            }
        }
    };

    // Die Fragmente, verschachtelt nach den Inline-Kästen.
    let mut stack: Vec<(usize, InlineFragment)> = Vec::new();
    let mut fragments: Vec<Fragment> = Vec::new();
    let make_inline = |placed_box: &PlacedBox| -> Option<InlineFragment> {
        let info = prepared.content.boxes.get(placed_box.data)?;
        let edges = prepared
            .edges
            .get(placed_box.data)
            .copied()
            .unwrap_or_default();
        let a = aligned.get(placed_box.data).copied().flatten()?;
        let baseline = baseline_of(&a);
        let mut border = edges.border;
        let mut padding = edges.padding;
        if !placed_box.has_start {
            border.left = 0.0;
            padding.left = 0.0;
        }
        if !placed_box.has_end {
            border.right = 0.0;
            padding.right = 0.0;
        }
        let content_top = baseline - a.metrics.ascent;
        let content_height = a.metrics.ascent + a.metrics.descent;
        let rect = Rect::new(
            placed_box.left,
            content_top - padding.top - border.top,
            (placed_box.right - placed_box.left).max(0.0),
            content_height + padding.vertical() + border.vertical(),
        );
        Some(InlineFragment {
            style: info.style.clone(),
            rect,
            border,
            padding,
            children: Vec::new(),
        })
    };
    fn push(
        stack: &mut [(usize, InlineFragment)],
        fragments: &mut Vec<Fragment>,
        fragment: Fragment,
    ) {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(fragment),
            None => fragments.push(fragment),
        }
    }
    fn close(stack: &mut Vec<(usize, InlineFragment)>, fragments: &mut Vec<Fragment>) {
        if let Some((_, fragment)) = stack.pop() {
            push(stack, fragments, Fragment::Inline(fragment));
        }
    }
    let mut placed_iter = 0;
    for _ in open {
        if let Some(fragment) = placed.get(placed_iter).and_then(make_inline) {
            stack.push((placed_iter, fragment));
        }
        placed_iter = placed_iter.saturating_add(1);
    }
    let mut text: Option<(usize, TextFragment)> = None;
    let flush_text = |text: &mut Option<(usize, TextFragment)>,
                      stack: &mut Vec<(usize, InlineFragment)>,
                      fragments: &mut Vec<Fragment>| {
        if let Some((_, fragment)) = text.take()
            && !fragment.runs.is_empty()
        {
            push(stack, fragments, Fragment::Text(fragment));
        }
    };
    for (offset_index, position) in (start..end).enumerate() {
        let x = positions.get(offset_index).copied().unwrap_or(0.0);
        match prepared.sequence.get(position) {
            Some(Piece::Start(data)) => {
                flush_text(&mut text, &mut stack, &mut fragments);
                let index = placed
                    .iter()
                    .enumerate()
                    .skip(open.len())
                    .find(|(_, b)| b.data == *data)
                    .map(|(index, _)| index);
                if let Some(index) = index
                    && let Some(fragment) = placed.get(index).and_then(make_inline)
                {
                    stack.push((index, fragment));
                }
            }
            Some(Piece::End(data)) => {
                flush_text(&mut text, &mut stack, &mut fragments);
                if stack
                    .last()
                    .and_then(|(index, _)| placed.get(*index))
                    .is_some_and(|b| b.data == *data)
                {
                    close(&mut stack, &mut fragments);
                }
            }
            Some(Piece::Char(index)) => {
                if prepared.is_trimmed(position, first, last_kept) {
                    continue;
                }
                let Some(c) = prepared.chars.get(*index) else {
                    continue;
                };
                if let Some(layout) = prepared.atomic(c.item) {
                    flush_text(&mut text, &mut stack, &mut fragments);
                    let a = atomic_aligned
                        .iter()
                        .find(|(item, _)| *item == c.item)
                        .map(|(_, a)| *a);
                    if let Some(a) = a {
                        let baseline = baseline_of(&a);
                        let top = baseline - layout.baseline;
                        let fragment = layout.place(x, top);
                        push(&mut stack, &mut fragments, fragment);
                    }
                    continue;
                }
                if c.glyphs.is_empty() {
                    continue;
                }
                let parent = prepared.item_parent(c.item);
                let style = prepared.parent_style(parent);
                let parent_aligned = parent
                    .and_then(|p| aligned.get(p).copied().flatten())
                    .unwrap_or(root);
                let baseline = baseline_of(&parent_aligned);
                if text.as_ref().is_some_and(|(item, _)| *item != c.item) {
                    flush_text(&mut text, &mut stack, &mut fragments);
                }
                let (_, fragment) = text.get_or_insert_with(|| {
                    (
                        c.item,
                        TextFragment {
                            style: style.clone(),
                            baseline,
                            runs: Vec::new(),
                        },
                    )
                });
                let mut glyph_x = x;
                for glyph in &c.glyphs {
                    let same_face = fragment.runs.last().is_some_and(|run| {
                        context
                            .fonts
                            .library()
                            .face(glyph.face)
                            .is_some_and(|face| std::sync::Arc::ptr_eq(&face, &run.face))
                    });
                    if !same_face {
                        if let Some(face) = context.fonts.library().face(glyph.face) {
                            let query = Query::new(&style.font());
                            fragment.runs.push(GlyphRun {
                                face,
                                size: style.font_size(),
                                glyphs: Vec::new(),
                                oblique: context.fonts.synthetic_oblique(&query, glyph.face),
                            });
                        } else {
                            glyph_x += f64::from(glyph.advance);
                            continue;
                        }
                    }
                    if let Some(run) = fragment.runs.last_mut() {
                        run.glyphs.push((glyph.id, glyph_x));
                    }
                    glyph_x += f64::from(glyph.advance);
                }
            }
            None => {}
        }
    }
    flush_text(&mut text, &mut stack, &mut fragments);
    while !stack.is_empty() {
        close(&mut stack, &mut fragments);
    }
    Some(Line {
        fragments,
        height: line_height,
        baseline: root_baseline,
        width,
        extent,
    })
}
