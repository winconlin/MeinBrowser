//! Eine Szene aus `mb-scene` zeichnen, mit einer festen Zahl Gerätepixel je
//! CSS-Pixel ([`render_scaled`]), etwa zwei auf Bildschirmen mit hoher
//! Auflösung.
//!
//! Kanten von Rechtecken und Bildern fallen auf ganze Gerätepixel: Beide Enden
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

/// Zeichnet `scene` auf eine neue Zeichenfläche ihrer Größe, aufgerundet,
/// ein Gerätepixel je CSS-Pixel.
pub fn render(scene: &Scene) -> Result<Canvas, TooLarge> {
    let size = |value: f32| {
        // Nach dem Begrenzen passt der Wert in u32.
        if value.is_finite() {
            value.ceil().clamp(0.0, u32::MAX as f32) as u32
        } else {
            0
        }
    };
    render_scaled(scene, size(scene.width), size(scene.height), 1.0)
}

/// Zeichnet `scene` mit `scale` Gerätepixeln je CSS-Pixel auf eine neue
/// Zeichenfläche von `width` × `height` Gerätepixeln. Ohne gültigen
/// Maßstab (endlich und größer als null) bleibt es beim Hintergrund.
pub fn render_scaled(
    scene: &Scene,
    width: u32,
    height: u32,
    scale: f32,
) -> Result<Canvas, TooLarge> {
    let mut canvas = Canvas::new(width, height, color(scene.background))?;
    if !(scale.is_finite() && scale > 0.0) {
        return Ok(canvas);
    }
    for item in &scene.items {
        match item {
            Item::Rect { rect, color: fill } => {
                fill_rect(&mut canvas, &scaled(rect, scale), color(*fill));
            }
            Item::Glyphs(run) => draw_glyphs(&mut canvas, run, scale),
            Item::Image {
                rect,
                clip,
                image,
                repeat_x,
                repeat_y,
            } => draw_image(
                &mut canvas,
                &scaled(rect, scale),
                &scaled(clip, scale),
                image,
                (*repeat_x, *repeat_y),
            ),
            Item::Polygon {
                points,
                color: fill,
            } => fill_polygon(&mut canvas, points, scale, color(*fill)),
        }
    }
    Ok(canvas)
}

/// Ein Rechteck in Gerätepixeln.
fn scaled(rect: &Rect, scale: f32) -> Rect {
    Rect::new(
        rect.x * scale,
        rect.y * scale,
        rect.width * scale,
        rect.height * scale,
    )
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

fn draw_glyphs(canvas: &mut Canvas, run: &GlyphRun, device_scale: f32) {
    let units_per_em = f32::from(run.face.metrics().units_per_em);
    let size = run.size * device_scale;
    if !(units_per_em > 0.0 && size.is_finite() && size > 0.0) {
        return;
    }
    let scale = size / units_per_em;
    let fill = color(run.color);
    // Künstlich schräg: x' = x + y · tan(Winkel) um die Grundlinie, nach
    // oben positiv. Höchstens 89 Grad, damit die Scherung endlich bleibt;
    // CSS erlaubt bis 90.
    let skew = if run.oblique.is_finite() {
        f64::from(run.oblique).clamp(-89.0, 89.0).to_radians().tan() as f32
    } else {
        0.0
    };
    // Das Rechteck um alle Glyphen der Schrift, nach oben positiv, mit
    // einem Pixel Rand für die Kantenglättung; geschert reicht es oben und
    // unten weiter zur Seite.
    let (x_min, y_min, x_max, y_max) = run.face.metrics().bounds;
    let (lean_low, lean_high) = {
        let (low, high) = (
            skew * f32::from(y_min) * scale,
            skew * f32::from(y_max) * scale,
        );
        (low.min(high), low.max(high))
    };
    let (left, right) = (
        f32::from(x_min) * scale - 1.0 + lean_low,
        f32::from(x_max) * scale + 1.0 + lean_high,
    );
    let (below, above) = (
        f32::from(y_min) * scale - 1.0,
        f32::from(y_max) * scale + 1.0,
    );
    let (width, height) = (canvas.width() as f32, canvas.height() as f32);
    for glyph in &run.glyphs {
        let (Some(x), Some(y)) = (snap(glyph.x * device_scale), snap(glyph.y * device_scale))
        else {
            continue;
        };
        // Glyphen ganz außerhalb der Fläche werden gar nicht erst gelesen.
        let (x_f, y_f) = (x as f32, y as f32);
        if x_f + right < 0.0 || x_f + left > width || y_f - below < 0.0 || y_f - above > height {
            continue;
        }
        let Ok(outline) = run.face.outline(glyph.id) else {
            continue;
        };
        if outline.is_empty() {
            continue;
        }
        // Ganze Pixel bis 10⁹ sind als f32 nur auf Vielfache von 64 genau;
        // so weit draußen zeichnet ohnehin nichts.
        let (origin_x, origin_y) = (x as f32, y as f32);
        let point = |p: mb_text::Point| {
            (
                origin_x + (p.x + skew * p.y) * scale,
                origin_y - p.y * scale,
            )
        };
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

fn fill_polygon(canvas: &mut Canvas, points: &[(f32, f32)], scale: f32, fill: Color) {
    let mut corners = points.iter().map(|&(x, y)| (x * scale, y * scale));
    let Some((x, y)) = corners.next() else {
        return;
    };
    if !points.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
        return;
    }
    let mut path = Path::new();
    path.move_to(x, y);
    for (x, y) in corners {
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
            oblique: 0.0,
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
    fn scaled_scenes_cover_device_pixels() {
        // Zwei Gerätepixel je CSS-Pixel: Das Rechteck bei x = 1 bedeckt die
        // Spalten 2 und 3 in beiden Zeilen, das Vieleck die Spalten 0 und 1.
        let mut scene = Scene::new(2.0, 1.0, mb_scene::Color::WHITE);
        scene.push_rect(Rect::new(1.0, 0.0, 1.0, 1.0), RED);
        scene.items.push(Item::Polygon {
            points: vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
            color: mb_scene::Color::rgb(0, 0, 255),
        });
        let canvas = render_scaled(&scene, 4, 2, 2.0).expect("Fläche");
        let row = [0xff00_00ff, 0xff00_00ff, 0xffff_0000, 0xffff_0000];
        assert_eq!(canvas.pixels(), [row, row].concat());
        // Ohne gültigen Maßstab nur der Hintergrund.
        let canvas = render_scaled(&scene, 4, 2, f32::NAN).expect("Fläche");
        assert!(canvas.pixels().iter().all(|&pixel| pixel == 0xffff_ffff));
    }

    #[test]
    fn oblique_glyphs_are_sheared_about_the_baseline() {
        // Ahem bei 10 px: ein Quadrat von 8 px über bis 2 px unter der
        // Grundlinie. Um 45 Grad geschert rückt jede Zeile um so viel nach
        // rechts, wie sie über der Grundlinie liegt.
        let data = testdata::read_bytes("wpt", "fonts/Ahem.ttf");
        let face = Arc::new(mb_text::Face::parse(data.into(), 0).expect("Ahem"));
        let id = face.glyph_index('X').expect("Glyphe");
        let mut scene = Scene::new(30.0, 12.0, mb_scene::Color::WHITE);
        scene.items.push(Item::Glyphs(GlyphRun {
            face,
            size: 10.0,
            color: mb_scene::Color::rgb(0, 0, 0),
            glyphs: vec![PositionedGlyph {
                id,
                x: 5.0,
                y: 10.0,
            }],
            oblique: 45.0,
        }));
        let canvas = render(&scene).expect("Fläche");
        let black = Some(0xff00_0000);
        let white = Some(0xffff_ffff);
        // Zeile 2 liegt 7 bis 8 px über der Grundlinie: von 12 bis 23 px.
        for x in 13..22 {
            assert_eq!(canvas.pixel(x, 2), black, "({x}, 2)");
        }
        assert_eq!(canvas.pixel(11, 2), white);
        assert_eq!(canvas.pixel(23, 2), white);
        // Zeile 11 liegt 1 bis 2 px unter ihr: von 3 bis 14 px.
        for x in 4..13 {
            assert_eq!(canvas.pixel(x, 11), black, "({x}, 11)");
        }
        assert_eq!(canvas.pixel(2, 11), white);
        assert_eq!(canvas.pixel(14, 11), white);
    }

    #[test]
    fn scaled_glyphs_start_on_whole_device_pixels() {
        // Wie `glyphs_start_on_whole_pixels` mit halber Größe und doppeltem
        // Maßstab: (0,7, 3,8) wird zu (1,4, 7,6) und auf (1, 8) gerundet.
        let data = testdata::read_bytes("wpt", "fonts/Ahem.ttf");
        let face = Arc::new(mb_text::Face::parse(data.into(), 0).expect("Ahem"));
        let id = face.glyph_index('X').expect("Glyphe");
        let mut scene = Scene::new(6.0, 5.0, mb_scene::Color::WHITE);
        scene.items.push(Item::Glyphs(GlyphRun {
            face,
            size: 5.0,
            color: mb_scene::Color::rgb(0, 0, 0),
            glyphs: vec![PositionedGlyph { id, x: 0.7, y: 3.8 }],
            oblique: 0.0,
        }));
        let canvas = render_scaled(&scene, 12, 10, 2.0).expect("Fläche");
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
    fn glyphs_partly_outside_are_drawn() {
        // Links und unten über den Rand hinaus: Zu sehen ist der Rest, ganz
        // außerhalb liegende Glyphen fehlen.
        let data = testdata::read_bytes("wpt", "fonts/Ahem.ttf");
        let face = Arc::new(mb_text::Face::parse(data.into(), 0).expect("Ahem"));
        let id = face.glyph_index('X').expect("Glyphe");
        let mut scene = Scene::new(10.0, 10.0, mb_scene::Color::WHITE);
        scene.items.push(Item::Glyphs(GlyphRun {
            face,
            size: 10.0,
            color: mb_scene::Color::rgb(0, 0, 0),
            glyphs: vec![
                PositionedGlyph {
                    id,
                    x: -5.0,
                    y: 13.0,
                },
                PositionedGlyph {
                    id,
                    x: -20.0,
                    y: 5.0,
                },
                PositionedGlyph {
                    id,
                    x: 0.0,
                    y: 25.0,
                },
            ],
            oblique: 0.0,
        }));
        let canvas = render(&scene).expect("Fläche");
        for y in 0..10 {
            for x in 0..10 {
                let expected = if x < 5 && y >= 5 {
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
