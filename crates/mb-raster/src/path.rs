//! Pfade und ihre Abdeckung mit Kantenglättung.
//!
//! Kurven werden beim Aufbau in Strecken zerlegt. Die Abdeckung entsteht
//! wie bei font-rs: Jede Strecke trägt in einen Puffer ein, um wie viel sich
//! die Abdeckung von einem Pixel zum nächsten ändert, nämlich um die Fläche,
//! die sie in jedem Pixel nach rechts abschneidet; die Summe entlang einer
//! Zeile ergibt die Abdeckung. Gefüllt wird nach der Regel *nonzero*, der
//! Betrag der Summe wird bei 1 abgeschnitten. Das ist exakt, wo sich
//! Konturen nicht überlagern, und an den Kanten überlagerter Konturen eine
//! Näherung.

/// So weit (in Pixeln) darf eine Strecke höchstens von ihrer Kurve
/// abweichen.
const TOLERANCE: f32 = 0.1;

/// Höchstens so viele Strecken je Kurve.
const MAX_SEGMENTS: u16 = 1000;

/// Eine Strecke von `(x0, y0)` nach `(x1, y1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Line {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Line {
    fn is_finite(&self) -> bool {
        self.x0.is_finite() && self.y0.is_finite() && self.x1.is_finite() && self.y1.is_finite()
    }
}

/// Ein Pfad aus Konturen, die y-Achse nach unten. Beim Füllen gilt jede
/// Kontur als geschlossen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    lines: Vec<Line>,
    /// Anfang der aktuellen Kontur.
    start: (f32, f32),
    current: (f32, f32),
    /// Ob die aktuelle Kontur noch offen ist.
    open: bool,
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    /// Ein Rechteck, im Uhrzeigersinn.
    pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Path {
        let mut path = Path::new();
        path.move_to(x, y);
        path.line_to(x + width, y);
        path.line_to(x + width, y + height);
        path.line_to(x, y + height);
        path.close();
        path
    }

    /// Beginnt eine neue Kontur und schließt die bisherige.
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.start = (x, y);
        self.current = (x, y);
        self.open = true;
    }

    /// Eine Strecke vom aktuellen Punkt. Ohne offene Kontur beginnt eine am
    /// aktuellen Punkt.
    pub fn line_to(&mut self, x: f32, y: f32) {
        if !self.open {
            let (start_x, start_y) = self.current;
            self.move_to(start_x, start_y);
        }
        let (x0, y0) = self.current;
        self.lines.push(Line {
            x0,
            y0,
            x1: x,
            y1: y,
        });
        self.current = (x, y);
    }

    /// Eine quadratische Bézierkurve mit dem Kontrollpunkt `(cx, cy)`.
    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (x0, y0) = self.current;
        // Mit n gleichen Schritten weicht die Kurve in der Mitte jeder
        // Strecke um |p0 − 2c + p1| / (4 n²) ab (die Formel von Wang für
        // Grad 2).
        let deviation = (x0 - 2.0 * cx + x).hypot(y0 - 2.0 * cy + y);
        let count = segments((deviation / (4.0 * TOLERANCE)).sqrt());
        for step in 1..=count {
            let t = f32::from(step) / f32::from(count);
            let u = 1.0 - t;
            self.line_to(
                u * u * x0 + 2.0 * u * t * cx + t * t * x,
                u * u * y0 + 2.0 * u * t * cy + t * t * y,
            );
        }
    }

    /// Eine kubische Bézierkurve mit den Kontrollpunkten `(c1x, c1y)` und
    /// `(c2x, c2y)`.
    pub fn cubic_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let (x0, y0) = self.current;
        // Die Formel von Wang: n = sqrt(3 · 2 / 8 · M / Toleranz) mit M, der
        // größten zweiten Differenz der Kontrollpunkte.
        let first = (x0 - 2.0 * c1x + c2x).hypot(y0 - 2.0 * c1y + c2y);
        let second = (c1x - 2.0 * c2x + x).hypot(c1y - 2.0 * c2y + y);
        let count = segments((0.75 * first.max(second) / TOLERANCE).sqrt());
        for step in 1..=count {
            let t = f32::from(step) / f32::from(count);
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.line_to(
                a * x0 + b * c1x + c * c2x + d * x,
                a * y0 + b * c1y + c * c2y + d * y,
            );
        }
    }

    /// Schließt die aktuelle Kontur mit einer Strecke zu ihrem Anfang.
    pub fn close(&mut self) {
        if !self.open {
            return;
        }
        if self.current != self.start {
            let (x, y) = self.start;
            self.line_to(x, y);
        }
        self.open = false;
    }

    /// Alle Strecken, die offene Kontur geschlossen.
    fn closed_lines(&self) -> impl Iterator<Item = Line> + Clone + '_ {
        let closing = (self.open && self.current != self.start).then_some(Line {
            x0: self.current.0,
            y0: self.current.1,
            x1: self.start.0,
            y1: self.start.1,
        });
        self.lines.iter().copied().chain(closing)
    }
}

/// Die Zahl der Strecken für eine Kurve.
fn segments(estimate: f32) -> u16 {
    if estimate.is_nan() {
        return 1;
    }
    // Der Wert liegt nach dem Abschneiden in 1 bis MAX_SEGMENTS.
    estimate.ceil().clamp(1.0, f32::from(MAX_SEGMENTS)) as u16
}

/// Die Abdeckung eines Pfads in einem Rechteck der Zeichenfläche.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Coverage {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Zeilenweise, je Pixel 0 bis 1.
    pub(crate) values: Vec<f32>,
}

/// Die Abdeckung von `path` auf einer Fläche von `width` mal `height`
/// Pixeln; `None`, wenn der Pfad dort nichts bedeckt oder nicht endliche
/// Koordinaten hat.
pub(crate) fn coverage(path: &Path, width: u32, height: u32) -> Option<Coverage> {
    let lines = path.closed_lines();
    if lines.clone().any(|line| !line.is_finite()) {
        return None;
    }
    let mut points = lines
        .clone()
        .flat_map(|line| [(line.x0, line.y0), (line.x1, line.y1)]);
    let (first_x, first_y) = points.next()?;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (first_x, first_y, first_x, first_y);
    for (x, y) in points {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    let (left, right) = pixel_span(min_x, max_x, width);
    let (top, bottom) = pixel_span(min_y, max_y, height);
    if left >= right || top >= bottom {
        return None;
    }
    let region_width = usize::try_from(right - left).ok()?;
    let region_height = usize::try_from(bottom - top).ok()?;
    // Zwei Spalten mehr für Einträge rechts der Fläche.
    let stride = region_width.checked_add(2)?;
    let mut cells = vec![0.0_f32; stride.checked_mul(region_height)?];
    let (offset_x, offset_y) = (left as f32, top as f32);
    for line in lines {
        let shifted = Line {
            x0: line.x0 - offset_x,
            y0: line.y0 - offset_y,
            x1: line.x1 - offset_x,
            y1: line.y1 - offset_y,
        };
        add_line(&mut cells, stride, region_width, region_height, shifted);
    }
    let mut values = Vec::with_capacity(region_width.checked_mul(region_height)?);
    for row in cells.chunks_exact(stride) {
        let mut sum = 0.0_f32;
        for &cell in row.iter().take(region_width) {
            sum += cell;
            values.push(sum.abs().min(1.0));
        }
    }
    Some(Coverage {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
        values,
    })
}

/// Die Pixel `[begin, end)` von `min` bis `max`, auf `[0, limit]` begrenzt.
fn pixel_span(min: f32, max: f32, limit: u32) -> (u32, u32) {
    let limit_float = limit as f32;
    // Nach dem Abschneiden liegen beide Werte in [0, limit].
    let begin = min.floor().clamp(0.0, limit_float) as u32;
    let end = max.ceil().clamp(0.0, limit_float) as u32;
    (begin.min(limit), end.min(limit))
}

/// Trägt eine Strecke ein. Teile über und unter der Fläche fallen weg,
/// Teile links von 0 und rechts von `width` werden zu senkrechten Strecken
/// auf dem Rand: Links tragen sie ihre ganze Abdeckung in die erste Spalte
/// ein, rechts liegen sie außerhalb.
fn add_line(cells: &mut [f32], stride: usize, width: usize, height: usize, line: Line) {
    let (limit_x, limit_y) = (width as f32, height as f32);
    let (start, end) = ((line.x0, line.y0), (line.x1, line.y1));
    if start.1.max(end.1) <= 0.0 || start.1.min(end.1) >= limit_y {
        return;
    }
    let flip = |(x, y): (f32, f32)| (y, x);
    let clip_y = |(x, y): (f32, f32)| {
        let clipped = y.clamp(0.0, limit_y);
        if clipped == y {
            (x, y)
        } else {
            (crossing(flip(start), flip(end), clipped), clipped)
        }
    };
    let (a, b) = (clip_y(start), clip_y(end));
    // Anfang, die Schnitte mit dem linken und rechten Rand in Richtung der
    // Strecke, Ende.
    let mut points = [a; 4];
    let mut count = 1;
    let borders = if a.0 <= b.0 {
        [0.0, limit_x]
    } else {
        [limit_x, 0.0]
    };
    for border in borders {
        if a.0.min(b.0) < border
            && border < a.0.max(b.0)
            && let Some(slot) = points.get_mut(count)
        {
            let y = crossing(start, end, border).clamp(a.1.min(b.1), a.1.max(b.1));
            *slot = (border, y);
            count += 1;
        }
    }
    if let Some(slot) = points.get_mut(count) {
        *slot = b;
        count += 1;
    }
    for pair in points.get(..count).unwrap_or_default().windows(2) {
        let &[(xa, ya), (xb, yb)] = pair else {
            continue;
        };
        add_segment(
            cells,
            stride,
            height,
            (xa.clamp(0.0, limit_x), ya),
            (xb.clamp(0.0, limit_x), yb),
        );
    }
}

/// Auf der Strecke von `a` nach `b` die zweite Koordinate dort, wo die
/// erste `at` ist; `at` liegt zwischen den ersten Koordinaten von `a` und
/// `b`. Gerechnet wird vom näheren Endpunkt aus und mit `f64`, damit ein
/// weit entfernter Endpunkt nicht die Genauigkeit kostet und die
/// Differenzen nicht überlaufen.
fn crossing(a: (f32, f32), b: (f32, f32), at: f32) -> f32 {
    let (a0, a1) = (f64::from(a.0), f64::from(a.1));
    let (b0, b1) = (f64::from(b.0), f64::from(b.1));
    let at = f64::from(at);
    let slope = (b1 - a1) / (b0 - a0);
    let value = if (at - a0).abs() <= (b0 - at).abs() {
        a1 + (at - a0) * slope
    } else {
        b1 + (at - b0) * slope
    };
    // Auf `f32` gerundet, in den Grenzen der Strecke.
    (value as f32).clamp(a.1.min(b.1), a.1.max(b.1))
}

/// Trägt eine Strecke ein, deren x-Koordinaten in `[0, Breite]` liegen.
fn add_segment(
    cells: &mut [f32],
    stride: usize,
    height: usize,
    (xa, ya): (f32, f32),
    (xb, yb): (f32, f32),
) {
    if ya == yb {
        return;
    }
    // Von oben nach unten; nach oben laufende Strecken zählen negativ.
    let (direction, x0, y0, x1, y1) = if ya < yb {
        (1.0, xa, ya, xb, yb)
    } else {
        (-1.0, xb, yb, xa, ya)
    };
    let dxdy = (x1 - x0) / (y1 - y0);
    let top = y0.max(0.0);
    let bottom = y1.min(height as f32);
    if top >= bottom {
        return;
    }
    let mut add = |index: usize, value: f32| {
        if let Some(cell) = cells.get_mut(index) {
            *cell += value;
        }
    };
    // x an einer Höhe, in den Grenzen der Strecke: Gerundete Werte dürfen
    // nicht in ein Nachbarpixel oder links aus der Fläche rutschen.
    let (low, high) = (x0.min(x1), x0.max(x1));
    let x_at = |y: f32| (x0 + (y - y0) * dxdy).clamp(low, high);
    let mut x = x_at(top);
    // Beide Grenzen liegen in [0, height].
    let first_row = top.floor() as usize;
    let end_row = bottom.ceil() as usize;
    for row in first_row..end_row {
        let row_top = row as f32;
        let row_bottom = (row_top + 1.0).min(bottom);
        let dy = row_bottom - row_top.max(top);
        let x_next = x_at(row_bottom);
        let d = dy * direction;
        let (left, right) = if x < x_next { (x, x_next) } else { (x_next, x) };
        let left_floor = left.floor();
        let right_ceil = right.ceil();
        // Beide liegen in [0, Breite].
        let first = left_floor as usize;
        let last = right_ceil as usize;
        let base = row.saturating_mul(stride);
        if last <= first + 1 {
            // In einem Pixel: rechts von der Kante liegt der Anteil
            // 1 − (mittleres x im Pixel).
            let middle = 0.5 * (x + x_next) - left_floor;
            add(base + first, d - d * middle);
            add(base + first + 1, d * middle);
        } else {
            // Über mehrere Pixel: je Einheit in x der Anteil `slope` der
            // Abdeckung, im ersten und letzten Pixel Dreiecke.
            let slope = 1.0 / (right - left);
            let left_fraction = left - left_floor;
            let first_area = 0.5 * slope * (1.0 - left_fraction) * (1.0 - left_fraction);
            let right_fraction = right - right_ceil + 1.0;
            let last_area = 0.5 * slope * right_fraction * right_fraction;
            add(base + first, d * first_area);
            if last == first + 2 {
                add(base + first + 1, d * (1.0 - first_area - last_area));
            } else {
                let second = slope * (1.5 - left_fraction);
                add(base + first + 1, d * (second - first_area));
                for column in first + 2..last - 1 {
                    add(base + column, d * slope);
                }
                let before_last = second + (last - first - 3) as f32 * slope;
                add(base + last - 1, d * (1.0 - before_last - last_area));
            }
            add(base + last, d * last_area);
        }
        x = x_next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Canvas, Color};

    /// Die Deckkraft jedes Pixels als Bruchteil von 1, gelesen am Blau
    /// von Weiß auf Schwarz.
    fn alphas(path: &Path, width: u32, height: u32) -> Vec<f32> {
        let mut canvas = Canvas::new(width, height, Color::BLACK).expect("Fläche");
        canvas.fill_path(path, Color::WHITE);
        canvas
            .pixels()
            .iter()
            .map(|pixel| (pixel & 0xff) as f32 / 255.0)
            .collect()
    }

    fn area(path: &Path, width: u32, height: u32) -> f32 {
        alphas(path, width, height).iter().sum()
    }

    #[test]
    fn rectangles_on_pixel_edges_match_fill_rect() {
        let mut expected = Canvas::new(6, 5, Color::BLACK).expect("Fläche");
        expected.fill_rect(1, 2, 3, 2, Color::WHITE);
        let mut canvas = Canvas::new(6, 5, Color::BLACK).expect("Fläche");
        canvas.fill_path(&Path::rect(1.0, 2.0, 3.0, 2.0), Color::WHITE);
        assert_eq!(canvas, expected);
        // Gegen den Uhrzeigersinn und ohne `close` dasselbe.
        let mut path = Path::new();
        path.move_to(1.0, 2.0);
        path.line_to(1.0, 4.0);
        path.line_to(4.0, 4.0);
        path.line_to(4.0, 2.0);
        let mut canvas = Canvas::new(6, 5, Color::BLACK).expect("Fläche");
        canvas.fill_path(&path, Color::WHITE);
        assert_eq!(canvas, expected);
    }

    #[test]
    fn partial_pixels_get_their_share() {
        let mut canvas = Canvas::new(2, 1, Color::BLACK).expect("Fläche");
        canvas.fill_path(&Path::rect(0.5, 0.0, 1.0, 1.0), Color::WHITE);
        assert_eq!(canvas.pixels(), [0xff80_8080, 0xff80_8080]);
        // Ein Dreieck mit der Fläche 8 auf 4 × 4 Pixeln.
        let mut triangle = Path::new();
        triangle.move_to(0.0, 0.0);
        triangle.line_to(4.0, 0.0);
        triangle.line_to(0.0, 4.0);
        assert!((area(&triangle, 4, 4) - 8.0).abs() < 0.02);
    }

    #[test]
    fn winding_decides_about_holes() {
        let ring = |inner_clockwise: bool| {
            let mut path = Path::rect(0.0, 0.0, 4.0, 4.0);
            path.move_to(1.0, 1.0);
            if inner_clockwise {
                path.line_to(3.0, 1.0);
                path.line_to(3.0, 3.0);
                path.line_to(1.0, 3.0);
            } else {
                path.line_to(1.0, 3.0);
                path.line_to(3.0, 3.0);
                path.line_to(3.0, 1.0);
            }
            alphas(&path, 4, 4)
        };
        assert!(ring(true).iter().all(|&alpha| alpha == 1.0));
        let hole = ring(false);
        for y in 0..4 {
            for x in 0..4 {
                let inside = (1..3).contains(&x) && (1..3).contains(&y);
                let expected = if inside { 0.0 } else { 1.0 };
                assert_eq!(hole.get(y * 4 + x), Some(&expected), "({x}, {y})");
            }
        }
    }

    #[test]
    fn edges_outside_are_clipped() {
        // Die Kante x = 4 − y geht über den linken Rand: Pixel mit
        // x + y ≤ 2 sind ganz bedeckt, mit x + y = 3 zur Hälfte.
        let mut triangle = Path::new();
        triangle.move_to(-4.0, 0.0);
        triangle.line_to(4.0, 0.0);
        triangle.line_to(-4.0, 8.0);
        let coverage = alphas(&triangle, 4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let expected = match x + y {
                    0..=2 => 1.0,
                    3 => 128.0 / 255.0,
                    _ => 0.0,
                };
                let value = coverage.get(y * 4 + x).copied().unwrap_or(-1.0);
                assert!((value - expected).abs() < 1e-6, "({x}, {y}): {value}");
            }
        }
        // Nur die linke obere Ecke liegt auf der Fläche.
        let corner = alphas(&Path::rect(-2.0, -2.0, 4.0, 4.0), 4, 4);
        for (index, value) in corner.iter().enumerate() {
            let expected = if index % 4 < 2 && index / 4 < 2 {
                1.0
            } else {
                0.0
            };
            assert_eq!(*value, expected, "{index}");
        }
        assert!(
            alphas(&Path::rect(-1e30, -1e30, 2e30, 2e30), 3, 3)
                .iter()
                .all(|&alpha| alpha == 1.0)
        );
        assert_eq!(area(&Path::rect(5.0, 0.0, 2.0, 2.0), 4, 4), 0.0);
    }

    #[test]
    fn far_endpoints_keep_their_precision() {
        // Die Kante von (10⁸, 0) nach (1, 3) liegt fast ganz rechts der
        // Fläche, die Spalten 1 und 2 sind also bis auf weniger als 10⁻⁶
        // bedeckt; an der Diagonale gespiegelt die Zeilen 1 und 2.
        let mut right = Path::new();
        right.move_to(1e8, 0.0);
        right.line_to(1.0, 3.0);
        right.line_to(1.0, 0.0);
        let mut below = Path::new();
        below.move_to(0.0, 1e8);
        below.line_to(3.0, 1.0);
        below.line_to(0.0, 1.0);
        // Beide Koordinaten weit weg: Bedeckt ist alles über der Diagonale.
        let mut diagonal = Path::new();
        diagonal.move_to(-1e8, -1e8);
        diagonal.line_to(3.0, 3.0);
        diagonal.line_to(3.0, -1e8);
        let check = |path: &Path, expected: fn(usize, usize) -> f32| {
            let coverage = alphas(path, 3, 3);
            for y in 0..3 {
                for x in 0..3 {
                    let value = coverage.get(y * 3 + x).copied().unwrap_or(-1.0);
                    let expected = expected(x, y);
                    assert!(
                        (value - expected).abs() <= 0.5 / 255.0 + 1e-6,
                        "{path:?} ({x}, {y}): {value}"
                    );
                }
            }
        };
        check(&right, |x, _| if x > 0 { 1.0 } else { 0.0 });
        check(&below, |_, y| if y > 0 { 1.0 } else { 0.0 });
        check(&diagonal, |x, y| match x.cmp(&y) {
            std::cmp::Ordering::Greater => 1.0,
            std::cmp::Ordering::Equal => 0.5,
            std::cmp::Ordering::Less => 0.0,
        });
    }

    /// Die Fläche der Strecken nach der Gaußschen Trapezformel.
    fn polygon_area(path: &Path) -> f32 {
        let twice: f32 = path
            .closed_lines()
            .map(|line| line.x0 * line.y1 - line.x1 * line.y0)
            .sum();
        twice.abs() / 2.0
    }

    #[test]
    fn curves_are_flattened_closely() {
        // Ein Kreis aus vier kubischen Bögen, Radius 10. Die Punkte liegen
        // auf dem Kreis (bis auf den Fehler der Bögen), die Mitten der
        // Strecken höchstens um die Toleranz innen.
        let k = 0.552_284_8 * 10.0;
        let (cx, cy) = (16.0, 16.0);
        let mut circle = Path::new();
        circle.move_to(cx + 10.0, cy);
        circle.cubic_to(cx + 10.0, cy + k, cx + k, cy + 10.0, cx, cy + 10.0);
        circle.cubic_to(cx - k, cy + 10.0, cx - 10.0, cy + k, cx - 10.0, cy);
        circle.cubic_to(cx - 10.0, cy - k, cx - k, cy - 10.0, cx, cy - 10.0);
        circle.cubic_to(cx + k, cy - 10.0, cx + 10.0, cy - k, cx + 10.0, cy);
        let radius = |x: f32, y: f32| (x - cx).hypot(y - cy);
        for line in circle.closed_lines() {
            assert!((radius(line.x0, line.y0) - 10.0).abs() < 0.01, "{line:?}");
            let middle = radius((line.x0 + line.x1) / 2.0, (line.y0 + line.y1) / 2.0);
            assert!(10.0 - middle <= TOLERANCE, "{line:?}");
        }
        assert!((area(&circle, 32, 32) - polygon_area(&circle)).abs() < 0.3);

        // Ein Parabelsegment, y = 8 − 2x + x²/4 von (0, 8) bis (8, 8). Die
        // Abweichung zeigt hier senkrecht nach oben.
        let mut segment = Path::new();
        segment.move_to(0.0, 8.0);
        segment.quad_to(4.0, 0.0, 8.0, 8.0);
        let parabola = |x: f32| 8.0 - 2.0 * x + x * x / 4.0;
        for line in segment.closed_lines().filter(|line| line.x1 > line.x0) {
            assert!((parabola(line.x0) - line.y0).abs() < 1e-4, "{line:?}");
            let middle = (line.x0 + line.x1) / 2.0;
            assert!(
                (line.y0 + line.y1) / 2.0 - parabola(middle) <= TOLERANCE,
                "{line:?}"
            );
        }
        assert!((area(&segment, 8, 8) - polygon_area(&segment)).abs() < 0.3);
        // Das Segment ist spiegelsymmetrisch; früher rutschte am linken Rand
        // ein gerundetes x unter 0 und vertauschte die Anteile eines Pixels.
        let coverage = alphas(&segment, 8, 8);
        for row in coverage.as_chunks::<8>().0 {
            for (left, right) in row.iter().zip(row.iter().rev()) {
                assert!((left - right).abs() <= 1.0 / 255.0, "{row:?}");
            }
        }
    }

    #[test]
    fn invalid_paths_draw_nothing() {
        let mut nan = Path::new();
        nan.move_to(0.0, 0.0);
        nan.line_to(f32::NAN, 2.0);
        nan.line_to(0.0, 2.0);
        assert_eq!(area(&nan, 4, 4), 0.0);
        let mut infinite = Path::rect(0.0, 0.0, 2.0, 2.0);
        infinite.quad_to(f32::INFINITY, 0.0, 1.0, 1.0);
        assert_eq!(area(&infinite, 4, 4), 0.0);
        assert_eq!(area(&Path::new(), 4, 4), 0.0);
        assert_eq!(area(&Path::rect(1.0, 1.0, 0.0, 2.0), 4, 4), 0.0);
    }
}
