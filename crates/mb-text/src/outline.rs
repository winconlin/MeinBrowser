//! Umrisse von Glyphen als Pfade, in Einheiten des Geviertquadrats der
//! Schrift (*font units*), die y-Achse nach oben.

/// Ein Punkt eines Umrisses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    /// Die Mitte zwischen zwei Punkten.
    pub fn midpoint(self, other: Point) -> Point {
        Point {
            x: (self.x + other.x) / 2.0,
            y: (self.y + other.y) / 2.0,
        }
    }
}

/// Ein Schritt eines Pfads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    /// Beginnt eine Kontur.
    MoveTo(Point),
    LineTo(Point),
    /// Quadratische Bézierkurve: Kontrollpunkt, Endpunkt.
    QuadTo(Point, Point),
    /// Kubische Bézierkurve: zwei Kontrollpunkte, Endpunkt.
    CubicTo(Point, Point, Point),
    /// Schließt die Kontur mit einer Linie zu ihrem Anfang.
    Close,
}

/// Der Umriss einer Glyphe. Konturen sind immer geschlossen; gefüllt wird
/// nach der Regel *nonzero*.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub commands: Vec<PathCommand>,
}

impl Outline {
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Das kleinste Rechteck um alle Punkte, Kontrollpunkte eingeschlossen:
    /// unten links und oben rechts. `None` bei einem leeren Umriss.
    pub fn control_bounds(&self) -> Option<(Point, Point)> {
        let mut points = self.commands.iter().flat_map(|command| {
            let points: &[Point] = match command {
                PathCommand::MoveTo(point) | PathCommand::LineTo(point) => {
                    std::slice::from_ref(point)
                }
                PathCommand::QuadTo(control, end) => &[*control, *end],
                PathCommand::CubicTo(first, second, end) => &[*first, *second, *end],
                PathCommand::Close => &[],
            };
            points.to_vec()
        });
        let first = points.next()?;
        let (mut min, mut max) = (first, first);
        for point in points {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
        }
        Some((min, max))
    }
}
