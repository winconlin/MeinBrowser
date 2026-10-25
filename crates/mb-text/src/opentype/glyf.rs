//! Umrisse aus den Tabellen `glyf` und `loca` (TrueType).
//!
//! Die Hinweise (*instructions*) für das Raster werden übergangen; die
//! Umrisse sind die unveränderten Entwürfe. Bei zusammengesetzten Glyphen
//! wird der Versatz einer Komponente nicht skaliert, außer das Flag
//! SCALED_COMPONENT_OFFSET ist gesetzt. So hält es Microsoft; Apple skaliert
//! ohne Flag, und fontTools folgt Microsoft.

use super::FontError;
use super::reader::{Reader, offset, u16_at, u32_at};
use crate::outline::{Outline, PathCommand, Point};

/// Wie tief zusammengesetzte Glyphen geschachtelt sein dürfen; Apple nennt
/// 16 als größten gültigen Wert von `maxComponentDepth`.
const MAX_DEPTH: usize = 16;
/// Wie viele Punkte eine Glyphe insgesamt haben darf; `maxp` speichert die
/// Zahlen der Punkte mit 16 Bits.
const MAX_POINTS: usize = 0xFFFF;
/// Wie viele Komponenten beim Laden einer Glyphe insgesamt besucht werden
/// dürfen, auch leere. Ohne Grenze wüchse die Arbeit mit tiefer
/// Schachtelung exponentiell.
const MAX_COMPONENTS: usize = 0xFFFF;

// Die Flags der Punkte einer einfachen Glyphe.
const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME_OR_POSITIVE: u8 = 0x10;
const Y_SAME_OR_POSITIVE: u8 = 0x20;

// Die Flags einer Komponente.
const ARGS_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const HAVE_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const HAVE_X_AND_Y_SCALE: u16 = 0x0040;
const HAVE_TWO_BY_TWO: u16 = 0x0080;
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;

/// Die Tabellen einer Schrift mit TrueType-Umrissen.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Glyphs<'a> {
    pub(crate) glyf: &'a [u8],
    pub(crate) loca: &'a [u8],
    pub(crate) long_offsets: bool,
    pub(crate) glyph_count: u16,
}

/// Ein Punkt mit der Angabe, ob er auf der Kurve liegt.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GlyphPoint {
    point: Point,
    on_curve: bool,
}

/// Die Punkte einer Glyphe und das Ende jeder Kontur (Index des letzten
/// Punkts).
#[derive(Debug, Default)]
struct Points {
    points: Vec<GlyphPoint>,
    ends: Vec<usize>,
}

/// Die Punkte als `(x, y, auf der Kurve)` und die Enden der Konturen.
#[cfg(test)]
pub(crate) type RawPoints = (Vec<(f32, f32, bool)>, Vec<usize>);

/// Was beim Laden einer Glyphe noch erlaubt ist.
struct Budget {
    points: usize,
    components: usize,
}

impl Glyphs<'_> {
    /// Der Umriss einer Glyphe.
    pub(crate) fn outline(&self, glyph: u16) -> Result<Outline, FontError> {
        let mut budget = Budget {
            points: MAX_POINTS,
            components: MAX_COMPONENTS,
        };
        let points = self.points(glyph, 0, &mut budget)?;
        Ok(to_outline(&points))
    }

    /// Die Punkte einer Glyphe mit der Angabe, ob sie auf der Kurve liegen,
    /// und die Enden der Konturen, zum Vergleich mit fontTools.
    #[cfg(test)]
    pub(crate) fn raw_points(&self, glyph: u16) -> Result<RawPoints, FontError> {
        let mut budget = Budget {
            points: MAX_POINTS,
            components: MAX_COMPONENTS,
        };
        let points = self.points(glyph, 0, &mut budget)?;
        let list = points
            .points
            .iter()
            .map(|point| (point.point.x, point.point.y, point.on_curve))
            .collect();
        Ok((list, points.ends))
    }

    /// Die Daten einer Glyphe in `glyf`; leer, wenn sie keinen Umriss hat.
    fn data(&self, glyph: u16) -> Result<&[u8], FontError> {
        if glyph >= self.glyph_count {
            return Err(FontError::Malformed("Glyphe außerhalb der Schrift"));
        }
        let index = usize::from(glyph);
        let (start, end) = if self.long_offsets {
            let start = u32_at(self.loca, offset(0, index, 4)?)?;
            let end = u32_at(self.loca, offset(0, index.saturating_add(1), 4)?)?;
            (
                usize::try_from(start).map_err(|_| FontError::Truncated)?,
                usize::try_from(end).map_err(|_| FontError::Truncated)?,
            )
        } else {
            // Die kurze Form speichert die Hälfte des Versatzes.
            let start = usize::from(u16_at(self.loca, offset(0, index, 2)?)?);
            let end = usize::from(u16_at(self.loca, offset(0, index.saturating_add(1), 2)?)?);
            (offset(0, start, 2)?, offset(0, end, 2)?)
        };
        if start > end {
            return Err(FontError::Malformed("loca: Ende vor dem Anfang"));
        }
        self.glyf.get(start..end).ok_or(FontError::Truncated)
    }

    fn points(&self, glyph: u16, depth: usize, budget: &mut Budget) -> Result<Points, FontError> {
        let data = self.data(glyph)?;
        if data.is_empty() {
            return Ok(Points::default());
        }
        let mut reader = Reader::at(data, 0)?;
        let contours = reader.i16()?;
        // Der Rahmen (xMin, yMin, xMax, yMax) wird aus den Punkten berechnet.
        reader.skip(8)?;
        match u16::try_from(contours) {
            Ok(contours) => simple(reader, contours, budget),
            Err(_) => self.composite(reader, depth, budget),
        }
    }

    fn composite(
        &self,
        mut reader: Reader<'_>,
        depth: usize,
        budget: &mut Budget,
    ) -> Result<Points, FontError> {
        if depth >= MAX_DEPTH {
            return Err(FontError::Malformed("Glyphen zu tief geschachtelt"));
        }
        let mut result = Points::default();
        loop {
            budget.components = budget
                .components
                .checked_sub(1)
                .ok_or(FontError::Malformed("zu viele Komponenten"))?;
            let flags = reader.u16()?;
            let glyph = reader.u16()?;
            // Versatz oder die Nummern zweier Punkte, die aufeinander fallen.
            let arguments = match (flags & ARGS_ARE_WORDS != 0, flags & ARGS_ARE_XY_VALUES != 0) {
                (true, true) => Arguments::Offset(reader.i16()?.into(), reader.i16()?.into()),
                (false, true) => Arguments::Offset(reader.i8()?.into(), reader.i8()?.into()),
                (true, false) => Arguments::Points(reader.u16()?.into(), reader.u16()?.into()),
                (false, false) => Arguments::Points(reader.u8()?.into(), reader.u8()?.into()),
            };
            // [a, b, c, d]: x' = a·x + c·y, y' = b·x + d·y.
            let matrix = if flags & HAVE_SCALE != 0 {
                let scale = reader.f2dot14()?;
                Some([scale, 0.0, 0.0, scale])
            } else if flags & HAVE_X_AND_Y_SCALE != 0 {
                let x = reader.f2dot14()?;
                let y = reader.f2dot14()?;
                Some([x, 0.0, 0.0, y])
            } else if flags & HAVE_TWO_BY_TWO != 0 {
                Some([
                    reader.f2dot14()?,
                    reader.f2dot14()?,
                    reader.f2dot14()?,
                    reader.f2dot14()?,
                ])
            } else {
                None
            };
            let mut component = self.points(glyph, depth.saturating_add(1), budget)?;
            let transform = |point: Point| match matrix {
                Some([a, b, c, d]) => {
                    Point::new(a * point.x + c * point.y, b * point.x + d * point.y)
                }
                None => point,
            };
            let shift = match arguments {
                Arguments::Offset(x, y) => {
                    let shift = Point::new(x, y);
                    if flags & SCALED_COMPONENT_OFFSET != 0 {
                        transform(shift)
                    } else {
                        shift
                    }
                }
                Arguments::Points(parent, child) => {
                    let parent = result
                        .points
                        .get(parent)
                        .ok_or(FontError::Malformed("Punkt der Glyphe fehlt"))?
                        .point;
                    let child = component
                        .points
                        .get(child)
                        .ok_or(FontError::Malformed("Punkt der Komponente fehlt"))?
                        .point;
                    let child = transform(child);
                    Point::new(parent.x - child.x, parent.y - child.y)
                }
            };
            for glyph_point in &mut component.points {
                let point = transform(glyph_point.point);
                glyph_point.point = Point::new(point.x + shift.x, point.y + shift.y);
            }
            let base = result.points.len();
            for end in component.ends {
                result
                    .ends
                    .push(end.checked_add(base).ok_or(FontError::Truncated)?);
            }
            result.points.append(&mut component.points);
            if flags & MORE_COMPONENTS == 0 {
                // Was folgt, sind Hinweise für die ganze Glyphe.
                return Ok(result);
            }
        }
    }
}

/// Die Argumente einer Komponente.
enum Arguments {
    Offset(f32, f32),
    /// Ein Punkt der bisherigen Glyphe und einer der Komponente.
    Points(usize, usize),
}

/// Eine einfache Glyphe mit `contours` Konturen.
fn simple(mut reader: Reader<'_>, contours: u16, budget: &mut Budget) -> Result<Points, FontError> {
    let mut ends = Vec::with_capacity(usize::from(contours));
    for _ in 0..contours {
        let end = usize::from(reader.u16()?);
        if ends.last().is_some_and(|&last| end <= last) {
            return Err(FontError::Malformed("Konturen nicht aufsteigend"));
        }
        ends.push(end);
    }
    let count = match ends.last() {
        Some(&last) => last.checked_add(1).ok_or(FontError::Truncated)?,
        None => 0,
    };
    budget.points = budget
        .points
        .checked_sub(count)
        .ok_or(FontError::Malformed("zu viele Punkte"))?;
    let instructions = usize::from(reader.u16()?);
    reader.skip(instructions)?;
    let mut flags = Vec::with_capacity(count);
    while flags.len() < count {
        let flag = reader.u8()?;
        let repeat = if flag & REPEAT != 0 {
            usize::from(reader.u8()?)
        } else {
            0
        };
        let total = flags.len().saturating_add(repeat).saturating_add(1);
        if total > count {
            return Err(FontError::Malformed("mehr Flags als Punkte"));
        }
        flags.resize(total, flag);
    }
    let xs = coordinates(&mut reader, &flags, X_SHORT, X_SAME_OR_POSITIVE)?;
    let ys = coordinates(&mut reader, &flags, Y_SHORT, Y_SAME_OR_POSITIVE)?;
    let points = flags
        .iter()
        .zip(xs.iter().zip(&ys))
        .map(|(flag, (&x, &y))| GlyphPoint {
            // Genau bis 2^24, und so große Koordinaten haben Schriften nicht.
            point: Point::new(x as f32, y as f32),
            on_curve: flag & ON_CURVE != 0,
        })
        .collect();
    Ok(Points { points, ends })
}

/// Die Koordinaten einer Achse: Differenzen zum vorigen Punkt, mit einem
/// Byte und Vorzeichen im Flag, gleich dem vorigen oder mit 16 Bits.
fn coordinates(
    reader: &mut Reader<'_>,
    flags: &[u8],
    short: u8,
    same_or_positive: u8,
) -> Result<Vec<i32>, FontError> {
    let mut values = Vec::with_capacity(flags.len());
    let mut current: i32 = 0;
    for &flag in flags {
        let delta = if flag & short != 0 {
            let magnitude = i32::from(reader.u8()?);
            if flag & same_or_positive != 0 {
                magnitude
            } else {
                -magnitude
            }
        } else if flag & same_or_positive != 0 {
            0
        } else {
            i32::from(reader.i16()?)
        };
        current = current.checked_add(delta).ok_or(FontError::Truncated)?;
        values.push(current);
    }
    Ok(values)
}

/// Die Konturen als Pfad. Zwischen zwei Punkten neben der Kurve liegt ein
/// gedachter Punkt auf der Kurve in ihrer Mitte.
fn to_outline(points: &Points) -> Outline {
    let mut commands = Vec::new();
    let mut start = 0;
    for &end in &points.ends {
        let contour = points.points.get(start..=end).unwrap_or_default();
        start = end.saturating_add(1);
        contour_commands(contour, &mut commands);
    }
    Outline { commands }
}

fn contour_commands(contour: &[GlyphPoint], commands: &mut Vec<PathCommand>) {
    let (Some(first), Some(last)) = (contour.first(), contour.last()) else {
        return;
    };
    // Der Anfang muss auf der Kurve liegen: der erste Punkt, sonst der
    // letzte, sonst die Mitte zwischen beiden.
    let (begin, rest) = if first.on_curve {
        (first.point, contour.get(1..).unwrap_or_default())
    } else if last.on_curve {
        (
            last.point,
            contour.get(..contour.len() - 1).unwrap_or_default(),
        )
    } else {
        (first.point.midpoint(last.point), contour)
    };
    commands.push(PathCommand::MoveTo(begin));
    let mut control: Option<Point> = None;
    for glyph_point in rest {
        let point = glyph_point.point;
        match (glyph_point.on_curve, control) {
            (true, None) => commands.push(PathCommand::LineTo(point)),
            (true, Some(previous)) => {
                commands.push(PathCommand::QuadTo(previous, point));
                control = None;
            }
            (false, None) => control = Some(point),
            (false, Some(previous)) => {
                commands.push(PathCommand::QuadTo(previous, previous.midpoint(point)));
                control = Some(point);
            }
        }
    }
    if let Some(previous) = control {
        commands.push(PathCommand::QuadTo(previous, begin));
    }
    commands.push(PathCommand::Close);
}
