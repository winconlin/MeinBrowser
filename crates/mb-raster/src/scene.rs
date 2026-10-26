//! Eine Szene aus `mb-scene` zeichnen, ein Gerätepixel je CSS-Pixel.
//!
//! Kanten von Rechtecken und Bildern fallen auf ganze Pixel: Beide Enden
//! werden gerundet, sodass aneinanderstoßende Rechtecke weder Lücken noch
//! Überlappungen haben. Glyphen beginnen auf ganzen Pixeln, gleicher Text
//! sieht also an jeder Stelle gleich aus; ihre Umrisse werden wie Vielecke
//! mit Kantenglättung gefüllt. Bilder werden nach dem nächsten Nachbarn
//! abgetastet, gekachelte ab der Kachel in `rect`.

use mb_scene::{GlyphRun, Image, Item, Rect, Scene};
use mb_text::PathCommand;

use crate::canvas::{Canvas, Color, TooLarge};
use crate::path::Path;

fn color(color: mb_scene::Color) -> Color {
    Color::rgba(color.r, color.g, color.b, color.a)
}

/// Eine Koordinate auf ganze Pixel gerundet; `None`, wenn sie nicht endlich
/// ist.
fn snap(value: f32) -> Option<i64> {
    // Nach dem Begrenzen passt der Wert in i64; so weit draußen liegt nie
    // ein Pixel der Fläche.
    value
        .is_finite()
        .then(|| value.round().clamp(-1.0e9, 1.0e9) as i64)
}

/// Die Pixel `[begin, end)`, die ein Rechteck in einer Richtung bedeckt,
/// auf `[0, limit)` begrenzt.
fn span(start: f32, end: f32, limit: u32) -> Option<(u32, u32)> {
    let limit = i64::from(limit);
    let begin = snap(start)?.clamp(0, limit);
    let end = snap(end)?.clamp(0, limit);
    Some((
        u32::try_from(begin).ok()?,
        u32::try_from(end.max(begin)).ok()?,
    ))
}

/// Die Pixel eines Rechtecks: Spalten und Zeilen.
fn pixels(rect: &Rect, canvas: &Canvas) -> Option<((u32, u32), (u32, u32))> {
    Some((
        span(rect.x, rect.right(), canvas.width())?,
        span(rect.y, rect.bottom(), canvas.height())?,
    ))
}

/// Zeichnet `scene` auf eine neue Zeichenfläche ihrer Größe, aufgerundet.
pub fn render(scene: &Scene) -> Result<Canvas, TooLarge> {
    let size = |value: f32| {
        // Nach dem Begrenzen passt der Wert in u32.
        if value.is_finite() {
            value.ceil().clamp(0.0, u32::MAX as f32) as u32
        } else {
            0
        }
    };
    let mut canvas = Canvas::new(
        size(scene.width),
        size(scene.height),
        color(scene.background),
    )?;
    for item in &scene.items {
        match item {
            Item::Rect { rect, color: fill } => fill_rect(&mut canvas, rect, color(*fill)),
            Item::Glyphs(run) => draw_glyphs(&mut canvas, run),
            Item::Image {
                rect,
                clip,
                image,
                repeat_x,
                repeat_y,
            } => draw_image(&mut canvas, rect, clip, image, (*repeat_x, *repeat_y)),
            Item::Polygon {
                points,
                color: fill,
            } => fill_polygon(&mut canvas, points, color(*fill)),
        }
    }
    Ok(canvas)
}

fn fill_rect(canvas: &mut Canvas, rect: &Rect, fill: Color) {
    let Some(((x0, x1), (y0, y1))) = pixels(rect, canvas) else {
        return;
    };
    for y in y0..y1 {
        for x in x0..x1 {
            canvas.blend(x, y, fill, 255);
        }
    }
}

fn draw_glyphs(canvas: &mut Canvas, run: &GlyphRun) {
    let units_per_em = f32::from(run.face.metrics().units_per_em);
    if !(units_per_em > 0.0 && run.size.is_finite() && run.size > 0.0) {
        return;
    }
    let scale = run.size / units_per_em;
    let fill = color(run.color);
    for glyph in &run.glyphs {
        let (Some(x), Some(y)) = (snap(glyph.x), snap(glyph.y)) else {
            continue;
        };
        let Ok(outline) = run.face.outline(glyph.id) else {
            continue;
        };
        if outline.is_empty() {
            continue;
        }
        // Ganze Pixel bis 10⁹ sind als f32 nur auf Vielfache von 64 genau;
        // so weit draußen zeichnet ohnehin nichts.
        let (origin_x, origin_y) = (x as f32, y as f32);
        let point = |p: mb_text::Point| (origin_x + p.x * scale, origin_y - p.y * scale);
        let mut path = Path::new();
        for command in &outline.commands {
            match *command {
                PathCommand::MoveTo(p) => {
                    let (x, y) = point(p);
                    path.move_to(x, y);
                }
                PathCommand::LineTo(p) => {
                    let (x, y) = point(p);
                    path.line_to(x, y);
                }
                PathCommand::QuadTo(c, p) => {
                    let (cx, cy) = point(c);
                    let (x, y) = point(p);
                    path.quad_to(cx, cy, x, y);
                }
                PathCommand::CubicTo(c1, c2, p) => {
                    let (c1x, c1y) = point(c1);
                    let (c2x, c2y) = point(c2);
                    let (x, y) = point(p);
                    path.cubic_to(c1x, c1y, c2x, c2y, x, y);
                }
                PathCommand::Close => path.close(),
            }
        }
        canvas.fill_path(&path, fill);
    }
}

fn fill_polygon(canvas: &mut Canvas, points: &[(f32, f32)], fill: Color) {
    let mut corners = points.iter();
    let Some(&(x, y)) = corners.next() else {
        return;
    };
    if !points.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
        return;
    }
    let mut path = Path::new();
    path.move_to(x, y);
    for &(x, y) in corners {
        path.line_to(x, y);
    }
    path.close();
    canvas.fill_path(&path, fill);
}

fn draw_image(canvas: &mut Canvas, rect: &Rect, clip: &Rect, image: &Image, repeat: (bool, bool)) {
    if rect.is_empty() || image.width() == 0 || image.height() == 0 {
        return;
    }
    let (Some(((x0, x1), (y0, y1))), Some(((cx0, cx1), (cy0, cy1)))) =
        (pixels(rect, canvas), pixels(clip, canvas))
    else {
        return;
    };
    // Gekachelt bedeckt das Bild in der Richtung die ganze Fläche.
    let (x0, x1) = if repeat.0 {
        (0, canvas.width())
    } else {
        (x0, x1)
    };
    let (y0, y1) = if repeat.1 {
        (0, canvas.height())
    } else {
        (y0, y1)
    };
    let scale_x = image.width() as f32 / rect.width;
    let scale_y = image.height() as f32 / rect.height;
    // Die Pixelmitte im Bild, bei Kacheln modulo der Bildgröße; nach dem
    // Begrenzen passt der Wert in u32.
    let source = |pixel: u32, start: f32, scale: f32, limit: u32, repeat: bool| {
        let offset = ((pixel as f32 + 0.5 - start) * scale).floor();
        let offset = if repeat {
            offset.rem_euclid(limit as f32)
        } else {
            offset
        };
        offset.clamp(0.0, (limit - 1) as f32) as u32
    };
    for y in y0.max(cy0)..y1.min(cy1) {
        let sy = source(y, rect.y, scale_y, image.height(), repeat.1);
        for x in x0.max(cx0)..x1.min(cx1) {
            let sx = source(x, rect.x, scale_x, image.width(), repeat.0);
            if let Some(pixel) = image.pixel(sx, sy) {
                canvas.blend(x, y, color(pixel), 255);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mb_scene::PositionedGlyph;

    use super::*;

    const RED: mb_scene::Color = mb_scene::Color::rgb(255, 0, 0);

    #[test]
    fn rectangles_snap_to_pixels() {
        let mut scene = Scene::new(4.0, 1.0, mb_scene::Color::WHITE);
        // 0,4 bis 1,6 wird zu 0 bis 2 gerundet und bedeckt die Pixel 0 und
        // 1; 1,6 bis 2,5 das Pixel 2 (2,5 wird zu 3). Aneinander stoßen sie
        // lückenlos.
        scene.push_rect(Rect::new(0.4, 0.0, 1.2, 1.0), RED);
        scene.push_rect(
            Rect::new(1.6, 0.0, 0.9, 1.0),
            mb_scene::Color::rgb(0, 0, 255),
        );
        let canvas = render(&scene).expect("Fläche");
        assert_eq!(
            canvas.pixels(),
            [0xffff_0000, 0xffff_0000, 0xff00_00ff, 0xffff_ffff]
        );
    }

    #[test]
    fn images_are_sampled_and_clipped() {
        // 2 × 1 Pixel: rot, blau, auf 4 × 2 gestreckt und rechts beschnitten.
        let image = Image::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).expect("Bild");
        let mut scene = Scene::new(4.0, 2.0, mb_scene::Color::WHITE);
        scene.items.push(Item::Image {
            rect: Rect::new(0.0, 0.0, 4.0, 2.0),
            clip: Rect::new(0.0, 0.0, 3.0, 2.0),
            image: Arc::new(image),
            repeat_x: false,
            repeat_y: false,
        });
        let canvas = render(&scene).expect("Fläche");
        let row = [0xffff_0000, 0xffff_0000, 0xff00_00ff, 0xffff_ffff];
        assert_eq!(canvas.pixels(), [row, row].concat());
    }

    #[test]
    fn images_repeat() {
        // Die Kachel beginnt bei x = 1; links davon geht es mit Blau weiter.
        let image = Image::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).expect("Bild");
        let mut scene = Scene::new(5.0, 2.0, mb_scene::Color::WHITE);
        scene.items.push(Item::Image {
            rect: Rect::new(1.0, 0.0, 2.0, 1.0),
            clip: Rect::new(0.0, 0.0, 5.0, 2.0),
            image: Arc::new(image),
            repeat_x: true,
            repeat_y: false,
        });
        let canvas = render(&scene).expect("Fläche");
        let white = [0xffff_ffff; 5];
        let row = [
            0xff00_00ff,
            0xffff_0000,
            0xff00_00ff,
            0xffff_0000,
            0xff00_00ff,
        ];
        assert_eq!(canvas.pixels(), [row, white].concat());
    }

    #[test]
    fn polygons_fill_whole_pixels() {
        let mut scene = Scene::new(3.0, 2.0, mb_scene::Color::WHITE);
        scene.items.push(Item::Polygon {
            points: vec![(1.0, 0.0), (3.0, 0.0), (3.0, 2.0), (1.0, 2.0)],
            color: RED,
        });
        let canvas = render(&scene).expect("Fläche");
        let row = [0xffff_ffff, 0xffff_0000, 0xffff_0000];
        assert_eq!(canvas.pixels(), [row, row].concat());
    }

    #[test]
    fn glyphs_start_on_whole_pixels() {
        // In Ahem ist jede Glyphe außer dem Leerzeichen ein Quadrat über die
        // ganze Vorschubbreite, 0,8 em über und 0,2 em unter der Grundlinie.
        let data = testdata::read_bytes("wpt", "fonts/Ahem.ttf");
        let face = Arc::new(mb_text::Face::parse(data.into(), 0).expect("Ahem"));
        let id = face.glyph_index('X').expect("Glyphe");
        let mut scene = Scene::new(12.0, 10.0, mb_scene::Color::WHITE);
        scene.items.push(Item::Glyphs(GlyphRun {
            face,
            size: 10.0,
            color: mb_scene::Color::rgb(0, 0, 0),
            // Wird zu (1, 8) gerundet.
            glyphs: vec![PositionedGlyph { id, x: 1.4, y: 7.6 }],
        }));
        let canvas = render(&scene).expect("Fläche");
        for y in 0..10 {
            for x in 0..12 {
                let expected = if (1..11).contains(&x) {
                    0xff00_0000
                } else {
                    0xffff_ffff
                };
                assert_eq!(canvas.pixel(x, y), Some(expected), "({x}, {y})");
            }
        }
    }

    #[test]
    fn invalid_geometry_draws_nothing() {
        let mut scene = Scene::new(2.5, 1.0, mb_scene::Color::WHITE);
        scene.push_rect(Rect::new(f32::NAN, 0.0, 1.0, 1.0), RED);
        scene.push_rect(Rect::new(-1e30, -1e30, 1e30, 1e30), RED);
        let canvas = render(&scene).expect("Fläche");
        assert_eq!(canvas.width(), 3);
        assert!(canvas.pixels().iter().all(|&pixel| pixel == 0xffff_ffff));
    }
}
