//! Die Zuordnung der flussrelativen Eigenschaften zu den physischen
//! („Abstract-to-Physical Mappings“ in CSS Writing Modes 4). In einer
//! *logical property group* teilen sich eine logische und die zugeordnete
//! physische Eigenschaft den berechneten Wert (CSS Logical 1).

use super::{Direction, Longhand, TextOrientation, WritingMode};

/// Was die Zuordnung braucht: der berechnete Schreibmodus und die
/// verwendete Richtung.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlowMode {
    pub writing_mode: WritingMode,
    /// Die verwendete Richtung (*used value* von `direction`).
    pub direction: Direction,
}

impl Default for FlowMode {
    fn default() -> FlowMode {
        FlowMode {
            writing_mode: WritingMode::HorizontalTb,
            direction: Direction::Ltr,
        }
    }
}

/// Eine Seite eines Kastens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// Eine flussrelative Seite oder Größe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    BlockStart,
    BlockEnd,
    InlineStart,
    InlineEnd,
    BlockSize,
    InlineSize,
}

impl FlowMode {
    /// Aus den berechneten Werten von `writing-mode`, `direction` und
    /// `text-orientation`. `upright` macht die verwendete Richtung `ltr`,
    /// wirkt aber nur im vertikalen *typographic mode*, also bei
    /// `vertical-rl` und `vertical-lr`.
    pub fn new(
        writing_mode: WritingMode,
        direction: Direction,
        text_orientation: TextOrientation,
    ) -> FlowMode {
        let vertical_typography = matches!(
            writing_mode,
            WritingMode::VerticalRl | WritingMode::VerticalLr
        );
        let direction = if vertical_typography && text_orientation == TextOrientation::Upright {
            Direction::Ltr
        } else {
            direction
        };
        FlowMode {
            writing_mode,
            direction,
        }
    }

    fn is_horizontal(self) -> bool {
        self.writing_mode == WritingMode::HorizontalTb
    }

    /// Die physische Seite zu einer flussrelativen.
    fn side(self, flow: Flow) -> Option<Side> {
        let ltr = self.direction == Direction::Ltr;
        let block_start = match self.writing_mode {
            WritingMode::HorizontalTb => Side::Top,
            WritingMode::VerticalRl | WritingMode::SidewaysRl => Side::Right,
            WritingMode::VerticalLr | WritingMode::SidewaysLr => Side::Left,
        };
        let inline_start = match (self.writing_mode, ltr) {
            (WritingMode::HorizontalTb, true) => Side::Left,
            (WritingMode::HorizontalTb, false) => Side::Right,
            (WritingMode::SidewaysLr, true) => Side::Bottom,
            (WritingMode::SidewaysLr, false) => Side::Top,
            (_, true) => Side::Top,
            (_, false) => Side::Bottom,
        };
        match flow {
            Flow::BlockStart => Some(block_start),
            Flow::BlockEnd => Some(block_start.opposite()),
            Flow::InlineStart => Some(inline_start),
            Flow::InlineEnd => Some(inline_start.opposite()),
            Flow::BlockSize | Flow::InlineSize => None,
        }
    }
}

impl Side {
    fn opposite(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Right => Side::Left,
            Side::Bottom => Side::Top,
            Side::Left => Side::Right,
        }
    }
}

/// Die physischen Langformen einer Gruppe mit Seiten: oben, rechts, unten,
/// links.
type Sides = [Longhand; 4];

/// Die physischen Langformen einer Gruppe mit Größen: Breite, Höhe.
type Sizes = [Longhand; 2];

/// Zu einer logischen Langform die Richtung und die physischen Langformen
/// ihrer Gruppe.
fn logical(longhand: Longhand) -> Option<(Flow, Result<Sides, Sizes>)> {
    use Longhand as L;
    const MARGIN: Sides = [L::MarginTop, L::MarginRight, L::MarginBottom, L::MarginLeft];
    const PADDING: Sides = [
        L::PaddingTop,
        L::PaddingRight,
        L::PaddingBottom,
        L::PaddingLeft,
    ];
    const BORDER_WIDTH: Sides = [
        L::BorderTopWidth,
        L::BorderRightWidth,
        L::BorderBottomWidth,
        L::BorderLeftWidth,
    ];
    const BORDER_STYLE: Sides = [
        L::BorderTopStyle,
        L::BorderRightStyle,
        L::BorderBottomStyle,
        L::BorderLeftStyle,
    ];
    const BORDER_COLOR: Sides = [
        L::BorderTopColor,
        L::BorderRightColor,
        L::BorderBottomColor,
        L::BorderLeftColor,
    ];
    const SIZE: Sizes = [L::Width, L::Height];
    const MIN_SIZE: Sizes = [L::MinWidth, L::MinHeight];
    const MAX_SIZE: Sizes = [L::MaxWidth, L::MaxHeight];
    let entry = match longhand {
        L::MarginBlockStart => (Flow::BlockStart, Ok(MARGIN)),
        L::MarginBlockEnd => (Flow::BlockEnd, Ok(MARGIN)),
        L::MarginInlineStart => (Flow::InlineStart, Ok(MARGIN)),
        L::MarginInlineEnd => (Flow::InlineEnd, Ok(MARGIN)),
        L::PaddingBlockStart => (Flow::BlockStart, Ok(PADDING)),
        L::PaddingBlockEnd => (Flow::BlockEnd, Ok(PADDING)),
        L::PaddingInlineStart => (Flow::InlineStart, Ok(PADDING)),
        L::PaddingInlineEnd => (Flow::InlineEnd, Ok(PADDING)),
        L::BorderBlockStartWidth => (Flow::BlockStart, Ok(BORDER_WIDTH)),
        L::BorderBlockEndWidth => (Flow::BlockEnd, Ok(BORDER_WIDTH)),
        L::BorderInlineStartWidth => (Flow::InlineStart, Ok(BORDER_WIDTH)),
        L::BorderInlineEndWidth => (Flow::InlineEnd, Ok(BORDER_WIDTH)),
        L::BorderBlockStartStyle => (Flow::BlockStart, Ok(BORDER_STYLE)),
        L::BorderBlockEndStyle => (Flow::BlockEnd, Ok(BORDER_STYLE)),
        L::BorderInlineStartStyle => (Flow::InlineStart, Ok(BORDER_STYLE)),
        L::BorderInlineEndStyle => (Flow::InlineEnd, Ok(BORDER_STYLE)),
        L::BorderBlockStartColor => (Flow::BlockStart, Ok(BORDER_COLOR)),
        L::BorderBlockEndColor => (Flow::BlockEnd, Ok(BORDER_COLOR)),
        L::BorderInlineStartColor => (Flow::InlineStart, Ok(BORDER_COLOR)),
        L::BorderInlineEndColor => (Flow::InlineEnd, Ok(BORDER_COLOR)),
        L::InlineSize => (Flow::InlineSize, Err(SIZE)),
        L::BlockSize => (Flow::BlockSize, Err(SIZE)),
        L::MinInlineSize => (Flow::InlineSize, Err(MIN_SIZE)),
        L::MinBlockSize => (Flow::BlockSize, Err(MIN_SIZE)),
        L::MaxInlineSize => (Flow::InlineSize, Err(MAX_SIZE)),
        L::MaxBlockSize => (Flow::BlockSize, Err(MAX_SIZE)),
        _ => return None,
    };
    Some(entry)
}

impl Longhand {
    /// Die physische Langform, die sich mit dieser den berechneten Wert
    /// teilt; physische und Langformen ohne Gruppe bleiben, wie sie sind.
    pub fn physical(self, flow: FlowMode) -> Longhand {
        let Some((direction, group)) = logical(self) else {
            return self;
        };
        let [top, right, bottom, left] = match group {
            Ok(sides) => sides,
            Err([width, height]) => {
                let inline = direction == Flow::InlineSize;
                return if inline == flow.is_horizontal() {
                    width
                } else {
                    height
                };
            }
        };
        match flow.side(direction) {
            Some(Side::Top) => top,
            Some(Side::Right) => right,
            Some(Side::Bottom) => bottom,
            Some(Side::Left) | None => left,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abstract_to_physical() {
        use Longhand as L;
        let flow = |writing_mode, direction| FlowMode {
            writing_mode,
            direction,
        };
        // Die Spalten der Tabelle in Writing Modes 4: block-start,
        // inline-start, inline-size.
        for (mode, expected) in [
            (
                flow(WritingMode::HorizontalTb, Direction::Ltr),
                [L::MarginTop, L::MarginLeft, L::Width],
            ),
            (
                flow(WritingMode::HorizontalTb, Direction::Rtl),
                [L::MarginTop, L::MarginRight, L::Width],
            ),
            (
                flow(WritingMode::VerticalRl, Direction::Ltr),
                [L::MarginRight, L::MarginTop, L::Height],
            ),
            (
                flow(WritingMode::SidewaysRl, Direction::Rtl),
                [L::MarginRight, L::MarginBottom, L::Height],
            ),
            (
                flow(WritingMode::VerticalLr, Direction::Rtl),
                [L::MarginLeft, L::MarginBottom, L::Height],
            ),
            (
                flow(WritingMode::SidewaysLr, Direction::Ltr),
                [L::MarginLeft, L::MarginBottom, L::Height],
            ),
            (
                flow(WritingMode::SidewaysLr, Direction::Rtl),
                [L::MarginLeft, L::MarginTop, L::Height],
            ),
        ] {
            let actual = [
                L::MarginBlockStart.physical(mode),
                L::MarginInlineStart.physical(mode),
                L::InlineSize.physical(mode),
            ];
            assert_eq!(actual, expected, "{mode:?}");
        }
        let vertical = flow(WritingMode::VerticalLr, Direction::Ltr);
        assert_eq!(
            L::BorderBlockEndColor.physical(vertical),
            L::BorderRightColor
        );
        assert_eq!(L::MinBlockSize.physical(vertical), L::MinWidth);
        assert_eq!(L::MarginTop.physical(vertical), L::MarginTop);
        assert_eq!(L::Color.physical(vertical), L::Color);
    }

    #[test]
    fn table_matches_groups() {
        use crate::properties::Mapping;
        let modes = [
            WritingMode::HorizontalTb,
            WritingMode::VerticalRl,
            WritingMode::VerticalLr,
            WritingMode::SidewaysRl,
            WritingMode::SidewaysLr,
        ];
        for &longhand in Longhand::ALL {
            let group = longhand.info().group;
            let is_logical = group.is_some_and(|(_, mapping)| mapping == Mapping::Logical);
            assert_eq!(
                logical(longhand).is_some(),
                is_logical,
                "{}",
                longhand.name()
            );
            for writing_mode in modes {
                for direction in [Direction::Ltr, Direction::Rtl] {
                    let flow = FlowMode {
                        writing_mode,
                        direction,
                    };
                    let physical = longhand.physical(flow).info().group;
                    assert_eq!(
                        physical.map(|(group, _)| group),
                        group.map(|(group, _)| group),
                        "{}",
                        longhand.name()
                    );
                    assert!(physical.is_none_or(|(_, mapping)| mapping == Mapping::Physical));
                }
            }
        }
    }

    #[test]
    fn upright_text_forces_ltr() {
        for (writing_mode, expected) in [
            (WritingMode::VerticalRl, Direction::Ltr),
            (WritingMode::VerticalLr, Direction::Ltr),
            (WritingMode::SidewaysRl, Direction::Rtl),
            (WritingMode::HorizontalTb, Direction::Rtl),
        ] {
            let flow = FlowMode::new(writing_mode, Direction::Rtl, TextOrientation::Upright);
            assert_eq!(flow.direction, expected, "{writing_mode:?}");
        }
    }
}
