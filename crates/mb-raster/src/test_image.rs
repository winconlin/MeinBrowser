//! Das Testbild für Meilenstein M0.
//!
//! Oben acht Farbbalken, darunter ein Grauverlauf und ein Schachbrett, in der
//! Mitte ein geglätteter Ring. Oben und unten unterscheiden sich, damit ein
//! gespiegeltes Bild auffällt. Die Messpunkte in [`PROBES`] liegen in
//! einfarbigen Flächen, sodass sie auch nach leichter Skalierung oder
//! Farbanpassung durch das System noch stimmen.

use crate::canvas::{Canvas, Color, TooLarge};

/// Hintergrund hinter dem Ring.
const BACKGROUND: Color = Color::rgb(0x20, 0x24, 0x2c);

/// Die klassischen Farbbalken in voller Sättigung.
const BARS: [Color; 8] = [
    Color::rgb(255, 255, 255),
    Color::rgb(255, 255, 0),
    Color::rgb(0, 255, 255),
    Color::rgb(0, 255, 0),
    Color::rgb(255, 0, 255),
    Color::rgb(255, 0, 0),
    Color::rgb(0, 0, 255),
    Color::rgb(0, 0, 0),
];

/// Anteil der Höhe, der von oben gezählt an der jeweiligen Zone endet.
const BARS_END: f32 = 0.6;
const GRADIENT_END: f32 = 0.8;

/// Ein Messpunkt, angegeben als Bruchteil von Breite und Höhe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    pub x: f32,
    pub y: f32,
}

/// Messpunkte zum Prüfen eines angezeigten Testbilds: die Mitten der acht
/// Balken im oberen Bereich (außerhalb des Rings) und zwei Punkte im Verlauf.
pub const PROBES: [Probe; 10] = [
    Probe { x: 0.0625, y: 0.1 },
    Probe { x: 0.1875, y: 0.1 },
    Probe { x: 0.3125, y: 0.1 },
    Probe { x: 0.4375, y: 0.1 },
    Probe { x: 0.5625, y: 0.1 },
    Probe { x: 0.6875, y: 0.1 },
    Probe { x: 0.8125, y: 0.1 },
    Probe { x: 0.9375, y: 0.1 },
    Probe { x: 0.25, y: 0.7 },
    Probe { x: 0.75, y: 0.7 },
];

/// Zeichnet das Testbild in der angegebenen Größe.
pub fn test_image(width: u32, height: u32) -> Result<Canvas, TooLarge> {
    let mut canvas = Canvas::new(width, height, BACKGROUND)?;
    let w = width as f32;
    let h = height as f32;
    let bars_end = (h * BARS_END) as u32;
    let gradient_end = (h * GRADIENT_END) as u32;

    // Farbbalken: Balken i reicht von i·w/8 bis (i+1)·w/8.
    for (i, color) in (0u64..).zip(BARS) {
        let x0 = u64::from(width) * i / 8;
        let x1 = u64::from(width) * (i + 1) / 8;
        // x0 und x1 sind höchstens `width` und passen daher in u32 und i32.
        canvas.fill_rect(x0 as i32, 0, (x1 - x0) as u32, bars_end, color);
    }

    // Grauverlauf von Schwarz links nach Weiß rechts.
    let span = u64::from(width.saturating_sub(1).max(1));
    for x in 0..width {
        let value = ((u64::from(x) * 255 + span / 2) / span).min(255) as u8;
        canvas.fill_rect(
            x as i32,
            bars_end as i32,
            1,
            gradient_end - bars_end,
            Color::rgb(value, value, value),
        );
    }

    // Schachbrett aus Quadraten mit einem Zwanzigstel der Höhe.
    let square = (height / 20).max(4);
    let mut row = 0;
    let mut y = gradient_end;
    while y < height {
        let mut column = 0;
        let mut x = 0;
        while x < width {
            let color = if (row + column) % 2 == 0 {
                Color::WHITE
            } else {
                Color::BLACK
            };
            canvas.fill_rect(x as i32, y as i32, square, square, color);
            x = x.saturating_add(square);
            column += 1;
        }
        y = y.saturating_add(square);
        row += 1;
    }

    // Geglätteter Ring auf dunkler Scheibe.
    let radius = 0.18 * w.min(h);
    let (cx, cy) = (w / 2.0, h * 0.35);
    canvas.fill_circle(cx, cy, radius, BACKGROUND);
    canvas.fill_ring(cx, cy, radius * 0.75, radius, Color::WHITE);
    canvas.fill_circle(cx, cy, radius * 0.3, Color::rgb(0xff, 0x80, 0x00));

    Ok(canvas)
}

/// Die Position eines Messpunkts in einem Bild der Größe `width` × `height`.
pub fn probe_position(probe: Probe, width: u32, height: u32) -> (u32, u32) {
    let x = (probe.x * width as f32) as u32;
    let y = (probe.y * height as f32) as u32;
    (
        x.min(width.saturating_sub(1)),
        y.min(height.saturating_sub(1)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_are_where_the_probes_expect_them() {
        let canvas = test_image(640, 400).unwrap();
        for (probe, color) in PROBES.iter().zip(BARS) {
            let (x, y) = probe_position(*probe, 640, 400);
            assert_eq!(canvas.pixel(x, y), Some(color.to_pixel()), "{probe:?}");
        }
    }

    #[test]
    fn gradient_runs_from_dark_to_light() {
        let canvas = test_image(640, 400).unwrap();
        let (x, y) = probe_position(PROBES[8], 640, 400);
        let left = canvas.pixel(x, y).unwrap() & 0xff;
        let (x, y) = probe_position(PROBES[9], 640, 400);
        let right = canvas.pixel(x, y).unwrap() & 0xff;
        assert!((60..70).contains(&left), "links {left}");
        assert!((187..197).contains(&right), "rechts {right}");
    }

    #[test]
    fn probes_avoid_the_ring_for_many_sizes() {
        for (width, height) in [(640, 400), (400, 800), (1920, 1080), (100, 60), (3000, 200)] {
            let canvas = test_image(width, height).unwrap();
            for (probe, color) in PROBES.iter().zip(BARS) {
                let (x, y) = probe_position(*probe, width, height);
                assert_eq!(
                    canvas.pixel(x, y),
                    Some(color.to_pixel()),
                    "{width}×{height} {probe:?}"
                );
            }
        }
    }

    #[test]
    fn is_not_symmetric_top_to_bottom() {
        let canvas = test_image(64, 64).unwrap();
        assert_ne!(canvas.pixel(4, 2), canvas.pixel(4, 61));
    }

    #[test]
    fn tiny_and_empty_sizes_work() {
        for (width, height) in [(0, 0), (1, 1), (0, 10), (10, 0), (3, 2)] {
            let canvas = test_image(width, height).unwrap();
            assert_eq!(canvas.pixels().len(), (width * height) as usize);
        }
    }

    #[test]
    fn is_deterministic() {
        assert_eq!(test_image(321, 123), test_image(321, 123));
    }
}
