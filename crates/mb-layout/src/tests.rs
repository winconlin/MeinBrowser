//! Tests des Layouts in einem Viewport von 800 × 600 mit Ahem aus den WPT
//! als einziger Schrift: Jede Glyphe außer dem Leerzeichen ist ein Quadrat
//! von 1 em, 0,8 em über und 0,2 em unter der Grundlinie, jedes Zeichen
//! 1 em breit.

use std::sync::{Arc, OnceLock};

use mb_base::CodePoint;
use mb_dom::{Document, NodeId};
use mb_html::{Options, ScriptingMode, parse_document};
use mb_scene::{Color, Item};
use mb_style::values::ParseContext;
use mb_style::values::color::{Rgba, SystemColor};
use mb_style::{DocumentInfo, Origin, StyleSheet, StylistBuilder};
use mb_text::FontLibrary;

use super::*;
use crate::fragment::{Fragment, Rect};

fn library() -> &'static FontLibrary {
    static LIBRARY: OnceLock<FontLibrary> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let mut library = FontLibrary::new(Box::new(|path| std::fs::read(path).ok()));
        let path = testdata::root().join("wpt").join("fonts").join("Ahem.ttf");
        assert_eq!(
            library.add_file(path),
            1,
            "Ahem fehlt. Bitte `cargo run -p testdata` ausführen."
        );
        library
    })
}

fn system_color(_: SystemColor) -> Rgba {
    Rgba {
        red: 0,
        green: 0,
        blue: 0,
        alpha: 255,
    }
}

/// Jedes Bild ist 20 × 10 Pixel groß und blau.
fn image(_: &Url) -> Option<Arc<Image>> {
    Image::new(20, 10, [0, 0, 255, 255].repeat(200)).map(Arc::new)
}

struct Page {
    document: Document,
    tree: FragmentTree,
}

fn load(html: &str, css: &str) -> Page {
    load_scaled(html, css, 1.0)
}

/// Wie [`load`] mit `scale` Gerätepixeln je CSS-Pixel.
fn load_scaled(html: &str, css: &str, scale: f64) -> Page {
    let mut document = Document::new();
    parse_document(
        &mut document,
        format!("<!DOCTYPE html>{html}")
            .chars()
            .map(CodePoint::from),
        Options {
            scripting: ScriptingMode::Disabled,
            collect_errors: false,
        },
    );
    document.perform_microtask_checkpoint();
    let base = Url::parse("https://example.com/", None).expect("Basis");
    let mut builder = StylistBuilder::new();
    builder.add_user_agent_sheets(DocumentInfo {
        quirks_mode: document.quirks_mode(),
        html_document: true,
        iso_8859_8: false,
    });
    let context = ParseContext {
        base_url: Some(&base),
        quirks_mode: false,
    };
    builder.add(
        Origin::Author,
        &StyleSheet::parse(&format!("* {{ font-family: Ahem }} {css}"), context),
    );
    let stylist = builder.build();
    let device = Device {
        viewport_width: 800.0,
        viewport_height: 600.0,
        device_pixel_ratio: scale,
        system_color,
    };
    let fonts = Fonts::new(library());
    let tree = layout(&LayoutInput {
        document: &document,
        stylist: &stylist,
        device: &device,
        fonts: &fonts,
        base: &base,
        images: &image,
    });
    Page { document, tree }
}

/// Ein Fragment mit dem Ursprung seines Koordinatensystems.
struct Placed<'a> {
    fragment: &'a Fragment,
    origin: (f64, f64),
}

impl Page {
    fn node(&self, id: &str) -> NodeId {
        let document = &self.document;
        let mut stack = vec![document.root()];
        while let Some(node) = stack.pop() {
            if document
                .element(node)
                .and_then(|element| element.attribute("id"))
                .is_some_and(|value| value.to_string_lossy() == id)
            {
                return node;
            }
            let children: Vec<NodeId> = document.children(node).collect();
            stack.extend(children.into_iter().rev());
        }
        panic!("kein Element #{id}");
    }

    /// Alle Fragmente in Baumreihenfolge.
    fn fragments(&self) -> Vec<Placed<'_>> {
        let Some(root) = &self.tree.root else {
            return Vec::new();
        };
        let mut result = Vec::new();
        let mut stack: Vec<(&[Fragment], (f64, f64))> = Vec::new();
        let content = root.content_rect();
        stack.push((&root.children, (content.x, content.y)));
        while let Some((children, origin)) = stack.pop() {
            let Some((first, rest)) = children.split_first() else {
                continue;
            };
            stack.push((rest, origin));
            result.push(Placed {
                fragment: first,
                origin,
            });
            match first {
                Fragment::Box(child) => {
                    let content = child.content_rect();
                    stack.push((
                        &child.children,
                        (origin.0 + content.x, origin.1 + content.y),
                    ));
                }
                Fragment::Inline(inline) => stack.push((&inline.children, origin)),
                Fragment::Text(_) | Fragment::Image(_) => {}
            }
        }
        result
    }

    /// Der Rahmenbereich eines Elements auf der Seite.
    fn rect(&self, id: &str) -> Rect {
        let node = self.node(id);
        if self
            .tree
            .root
            .as_ref()
            .is_some_and(|root| root.node == Some(node))
        {
            return self
                .tree
                .root
                .as_ref()
                .map(|root| root.rect)
                .unwrap_or_default();
        }
        self.fragments()
            .iter()
            .find_map(|placed| match placed.fragment {
                Fragment::Box(child) if child.node == Some(node) => {
                    Some(child.rect.translate(placed.origin.0, placed.origin.1))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("kein Kasten für #{id}"))
    }

    /// Die Textfragmente: Grundlinie und Anfänge der Glyphen auf der Seite.
    fn texts(&self) -> Vec<(f64, Vec<f64>)> {
        self.fragments()
            .iter()
            .filter_map(|placed| match placed.fragment {
                Fragment::Text(text) => Some((
                    placed.origin.1 + text.baseline,
                    text.runs
                        .iter()
                        .flat_map(|run| run.glyphs.iter().map(|&(_, x)| placed.origin.0 + x))
                        .collect(),
                )),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn margins_collapse() {
    // b fällt in sich zusammen, und seine Abstände fallen mit denen von
    // body, a und c zusammen: 30 px. Der untere Abstand von c geht durch a
    // und fällt mit dem oberen von d zusammen: 20 px und −5 px.
    let page = load(
        "<body id=body><div id=a><div id=b></div><div id=c></div></div><div id=d></div>",
        "body { margin: 0 } #a { margin-top: 10px } #b { margin: 30px 0 }
         #c { margin: 5px 0 20px; height: 10px } #d { margin-top: -5px; height: 1px }",
    );
    assert_eq!(page.rect("a"), Rect::new(0.0, 30.0, 800.0, 10.0));
    assert_eq!(page.rect("b"), Rect::new(0.0, 30.0, 800.0, 0.0));
    assert_eq!(page.rect("c"), Rect::new(0.0, 30.0, 800.0, 10.0));
    assert_eq!(page.rect("d"), Rect::new(0.0, 55.0, 800.0, 1.0));
    assert_eq!(page.rect("body"), Rect::new(0.0, 30.0, 800.0, 26.0));
}

#[test]
fn borders_and_padding_separate_margins() {
    let page = load(
        "<div id=a><div id=b></div></div><div id=c></div>",
        "body { margin: 0 } #a { padding-top: 1px; border-bottom: 2px solid; margin-bottom: 10px }
         #b { margin: 10px 0; height: 5px } #c { margin-top: 20px; min-height: 3px }",
    );
    // Der obere Abstand von b bleibt in a, der untere auch.
    assert_eq!(page.rect("a"), Rect::new(0.0, 0.0, 800.0, 28.0));
    assert_eq!(page.rect("b"), Rect::new(0.0, 11.0, 800.0, 5.0));
    assert_eq!(page.rect("c"), Rect::new(0.0, 48.0, 800.0, 3.0));
}

#[test]
fn widths() {
    let page = load(
        "<div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div>",
        "body { margin: 0 } div { height: 1px }
         #a { width: 200px; margin: 0 auto; padding: 0 10px; border: 5px solid }
         #b { margin-left: 50px; margin-right: 30px }
         #c { width: 100px; min-width: 150px; margin-left: auto }
         #d { width: 50%; max-width: 300px; box-sizing: border-box; padding-left: 20px }
         #e { width: 900px; margin: 0 auto }",
    );
    assert_eq!(page.rect("a"), Rect::new(285.0, 0.0, 230.0, 11.0));
    assert_eq!(page.rect("b"), Rect::new(50.0, 11.0, 720.0, 1.0));
    assert_eq!(page.rect("c"), Rect::new(650.0, 12.0, 150.0, 1.0));
    assert_eq!(page.rect("d"), Rect::new(0.0, 13.0, 300.0, 1.0));
    // Zu breit: `auto` gilt als null, der rechte Abstand weicht.
    assert_eq!(page.rect("e"), Rect::new(0.0, 14.0, 900.0, 1.0));
}

#[test]
fn heights_with_percentages() {
    let page = load(
        "<div id=a><div id=b></div></div><div id=c><div id=d></div></div>",
        "html, body { margin: 0; height: 100% } #a { height: 50% } #b { height: 50% }
         #c { min-height: 10px } #d { height: 50% }",
    );
    assert_eq!(page.rect("a").height, 300.0);
    assert_eq!(page.rect("b").height, 150.0);
    // Ohne feste Höhe des Blocks gilt `auto`.
    assert_eq!(page.rect("d").height, 0.0);
    assert_eq!(page.rect("c").height, 10.0);
}

#[test]
fn lines_wrap_and_align() {
    let page = load(
        "<div id=a>aaa bbb\n ccc</div><div id=b>aa<br>b</div>",
        "body { margin: 0 } div { font-size: 20px; line-height: 1; width: 100px }
         #b { text-align: right }",
    );
    assert_eq!(page.rect("a"), Rect::new(0.0, 0.0, 100.0, 60.0));
    assert_eq!(page.rect("b"), Rect::new(0.0, 60.0, 100.0, 40.0));
    assert_eq!(
        page.texts(),
        [
            (16.0, vec![0.0, 20.0, 40.0]),
            (36.0, vec![0.0, 20.0, 40.0]),
            (56.0, vec![0.0, 20.0, 40.0]),
            (76.0, vec![60.0, 80.0]),
            (96.0, vec![80.0]),
        ]
    );
}

#[test]
fn white_space_and_nowrap() {
    let page = load(
        "<div id=a>a  \n\t b</div><div id=b>aa bb</div><pre id=c>a  b\nc</pre>",
        "body { margin: 0 } div, pre { font-size: 10px; line-height: 1; width: 30px; margin: 0 }
         #b { white-space: nowrap }",
    );
    // Der Leerraum fällt zu einem Leerzeichen zusammen.
    assert_eq!(page.texts().first(), Some(&(8.0, vec![0.0, 10.0, 20.0])));
    // Ohne Umbruch läuft die Zeile über.
    assert_eq!(page.rect("b").height, 10.0);
    // In `pre` bleiben Leerzeichen und Zeilenumbrüche.
    assert_eq!(page.rect("c").height, 20.0);
    let texts = page.texts();
    assert_eq!(texts.get(2), Some(&(28.0, vec![0.0, 10.0, 20.0, 30.0])));
    assert_eq!(texts.get(3), Some(&(38.0, vec![0.0])));
}

#[test]
fn inline_boxes_take_space_and_line_height() {
    let page = load(
        "<div id=a>a<span id=s>b</span>c</div><div id=b>a<span id=t>b</span></div>",
        "body { margin: 0 } div { font-size: 10px; line-height: 1 }
         #s { padding: 0 5px; border-left: 2px solid; margin-right: 3px }
         #t { font-size: 20px; line-height: 30px }",
    );
    // Der Inhaltsbereich ist 10 px hoch, Rand und Innenabstand kommen dazu.
    assert_eq!(page.rect("a").height, 10.0);
    let texts = page.texts();
    assert_eq!(texts.first(), Some(&(8.0, vec![0.0])));
    assert_eq!(texts.get(1), Some(&(8.0, vec![17.0])));
    assert_eq!(texts.get(2), Some(&(8.0, vec![35.0])));
    // b: 30 px Zeilenhöhe um 16 + 4 px, also 5 px Durchschuss oben und
    // unten; die Grundlinie liegt bei 21 px.
    assert_eq!(page.rect("b").height, 30.0);
    assert_eq!(texts.get(3), Some(&(31.0, vec![0.0])));
    assert_eq!(texts.get(4), Some(&(31.0, vec![10.0])));
}

#[test]
fn inline_blocks_shrink_and_sit_on_the_baseline() {
    let page = load(
        "<div id=a>x<span id=b>yy yy</span></div>\
         <div id=c>x<span id=d>y</span><span id=e>z</span></div>",
        "body { margin: 0 } div { font-size: 10px; line-height: 1; width: 50px }
         span { display: inline-block }
         #d { padding-bottom: 6px } #e { vertical-align: top; height: 25px }",
    );
    // Neben x passt der Inline-Block nicht; er kommt in die zweite Zeile.
    assert_eq!(page.rect("b"), Rect::new(0.0, 10.0, 50.0, 10.0));
    // d steht auf der Grundlinie (8 px), mit 6 px darunter; e oben, und
    // die Zeile wächst nach unten auf 25 px.
    assert_eq!(page.rect("d"), Rect::new(10.0, 20.0, 10.0, 16.0));
    assert_eq!(page.rect("e"), Rect::new(20.0, 20.0, 10.0, 25.0));
    assert_eq!(page.rect("c").height, 25.0);
}

#[test]
fn images_keep_their_ratio() {
    let page = load(
        "<div id=a><img id=b src=x><img id=c src=x style='width: 40px'></div>\
         <img id=d src=x style='display: block; max-width: 10px; margin: 0 auto'>",
        "body { margin: 0 } div { font-size: 10px; line-height: 1 }",
    );
    // Auf der Grundlinie, 8 px unter der Oberkante der Zeile; die Zeile
    // reicht bis 2 px unter das höchste Bild.
    assert_eq!(page.rect("b"), Rect::new(0.0, 10.0, 20.0, 10.0));
    assert_eq!(page.rect("c"), Rect::new(20.0, 0.0, 40.0, 20.0));
    assert_eq!(page.rect("a").height, 22.0);
    assert_eq!(page.rect("d"), Rect::new(395.0, 22.0, 10.0, 5.0));
}

#[test]
fn body_background_paints_the_canvas() {
    let page = load(
        "<div id=a></div>",
        "body { background: lime; margin: 10px } #a { height: 10px; background: blue }",
    );
    let scene = page.tree.paint(&image);
    let lime = Color::rgb(0, 255, 0);
    let rects: Vec<(mb_scene::Rect, Color)> = scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect { rect, color } => Some((*rect, *color)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rects,
        [
            (mb_scene::Rect::new(0.0, 0.0, 800.0, 600.0), lime),
            (
                mb_scene::Rect::new(10.0, 10.0, 780.0, 10.0),
                Color::rgb(0, 0, 255)
            ),
        ]
    );
}

#[test]
fn deep_nesting_fits_on_the_stack() {
    let depth = 400;
    let html = format!(
        "{}x{}",
        "<div><span style='display: inline-block'>".repeat(depth),
        "</span></div>".repeat(depth)
    );
    let page = load(&html, "");
    assert!(page.tree.root.is_some());
}

#[test]
fn overflowing_lines_are_start_aligned() {
    // Eine zu lange Zeile beginnt am Zeilenanfang und läuft über das Ende
    // hinaus, wie `text-align` auch steht (CSS Text 3, Abschnitt 6.1).
    let page = load(
        "<div>xxxxx</div><div style='direction: rtl'>xxxxx</div>",
        "body { margin: 0 } div { width: 30px; font-size: 10px; text-align: center }",
    );
    let starts: Vec<Option<f64>> = page
        .texts()
        .iter()
        .map(|(_, glyphs)| glyphs.first().copied())
        .collect();
    assert_eq!(starts, [Some(0.0), Some(-20.0)]);
}

#[test]
fn vertical_align_percentages_of_atomic_inlines() {
    // Prozente beziehen sich auf die eigene `line-height`, nicht auf die
    // Höhe des Kastens: 50 % von 20 px sind 10 px.
    let page = load(
        "<div id=a><img id=b src=x style='vertical-align: 50%'></div>\
         <div id=c><img id=d src=x style='vertical-align: 10px'></div>",
        "body { margin: 0 } div { font-size: 10px; line-height: 20px }",
    );
    let (a, b, c, d) = (
        page.rect("a"),
        page.rect("b"),
        page.rect("c"),
        page.rect("d"),
    );
    assert_eq!(b.y - a.y, d.y - c.y);
    assert_eq!(a.height, c.height);
}

#[test]
fn transparent_sides_keep_slanted_corners() {
    // Neben einer durchsichtigen Seite endet ein Rahmen schräg in der Ecke:
    // Hier wird der untere Rahmen ein Dreieck.
    let page = load(
        "<div></div>",
        "body { margin: 0 } div { width: 0; height: 0; border: 10px solid transparent; \
         border-bottom-color: red }",
    );
    let scene = page.tree.paint(&image);
    let red = Color::rgb(255, 0, 0);
    let painted: Vec<Option<&[(f32, f32)]>> = scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect { color, .. } if *color == red => Some(None),
            Item::Polygon { points, color } if *color == red => Some(Some(points.as_slice())),
            _ => None,
        })
        .collect();
    assert_eq!(
        painted,
        [Some(
            [(20.0, 20.0), (0.0, 20.0), (10.0, 10.0), (10.0, 10.0)].as_slice()
        )]
    );
}

/// Die Vielecke in `color` als umgebende Rechtecke: links, oben, rechts,
/// unten.
fn polygon_bounds(scene: &mb_scene::Scene, color: Color) -> Vec<(f32, f32, f32, f32)> {
    let bounds = |points: &[(f32, f32)]| {
        points.iter().fold(
            (
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ),
            |(left, top, right, bottom), &(x, y)| {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            },
        )
    };
    scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Polygon { points, color: c } if *c == color => Some(bounds(points)),
            _ => None,
        })
        .collect()
}

fn has_rect(scene: &mb_scene::Scene, color: Color) -> bool {
    scene
        .items
        .iter()
        .any(|item| matches!(item, Item::Rect { color: c, .. } if *c == color))
}

#[test]
fn dashed_borders_are_drawn_as_dashes() {
    // 54 px mit Strichen von 6 px: fünf Striche mit Lücken von 6 px, der
    // erste und der letzte an den Enden.
    let page = load(
        "<div></div>",
        "body { margin: 0 } div { width: 54px; border-top: 2px dashed red }",
    );
    let red = Color::rgb(255, 0, 0);
    let scene = page.tree.paint(&image);
    assert!(!has_rect(&scene, red));
    assert_eq!(
        polygon_bounds(&scene, red),
        [
            (0.0, 0.0, 6.0, 2.0),
            (12.0, 0.0, 18.0, 2.0),
            (24.0, 0.0, 30.0, 2.0),
            (36.0, 0.0, 42.0, 2.0),
            (48.0, 0.0, 54.0, 2.0),
        ]
    );
}

#[test]
fn dotted_borders_are_drawn_as_round_dots() {
    // Rahmen von 4 px um 20 × 20 px: Punkte mit 4 px Durchmesser, je Seite
    // bei 2, 10, 18 und 26 px. Die äußeren sitzen in der Mitte der Ecken;
    // die gleichen Seiten treffen sich gerade, die Punkte in den Ecken
    // zeichnen oben und unten ganz.
    let page = load(
        "<div></div>",
        "body { margin: 0 } div { width: 20px; height: 20px; border: 4px dotted red }",
    );
    let red = Color::rgb(255, 0, 0);
    let scene = page.tree.paint(&image);
    assert!(!has_rect(&scene, red));
    let bounds = polygon_bounds(&scene, red);
    assert_eq!(bounds.len(), 12);
    for dot in [
        (0.0, 0.0, 4.0, 4.0),
        (24.0, 24.0, 28.0, 28.0),
        (8.0, 0.0, 12.0, 4.0),
        (16.0, 0.0, 20.0, 4.0),
        (24.0, 8.0, 28.0, 12.0),
        (8.0, 24.0, 12.0, 28.0),
        (0.0, 16.0, 4.0, 20.0),
    ] {
        assert!(bounds.contains(&dot), "{dot:?} fehlt in {bounds:?}");
    }
    // Alles liegt im Rahmen, nichts im Kasten darin.
    for item in &scene.items {
        if let Item::Polygon { points, color } = item
            && *color == red
        {
            for &(x, y) in points {
                assert!((0.0..=28.0).contains(&x) && (0.0..=28.0).contains(&y));
                assert!(
                    !(x > 4.001 && x < 23.999 && y > 4.001 && y < 23.999),
                    "({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn short_sides_get_a_single_dot() {
    // 5 px mit Punkten von 4 px: Für zwei ist kein Platz.
    let page = load(
        "<div></div>",
        "body { margin: 0 } div { width: 5px; border-top: 4px dotted red }",
    );
    let red = Color::rgb(255, 0, 0);
    let scene = page.tree.paint(&image);
    assert_eq!(polygon_bounds(&scene, red), [(0.5, 0.0, 4.5, 4.0)]);
}

#[test]
fn thin_dots_are_squares_on_whole_pixels() {
    let page = load(
        "<div></div>",
        "body { margin: 0 } div { width: 9px; border-top: 1px dotted red }",
    );
    let red = Color::rgb(255, 0, 0);
    let scene = page.tree.paint(&image);
    assert_eq!(
        polygon_bounds(&scene, red),
        [
            (0.0, 0.0, 1.0, 1.0),
            (2.0, 0.0, 3.0, 1.0),
            (4.0, 0.0, 5.0, 1.0),
            (6.0, 0.0, 7.0, 1.0),
            (8.0, 0.0, 9.0, 1.0),
        ]
    );
}

#[test]
fn dashed_outlines_bend_around_the_corners() {
    // Der Umriss liegt bei 8 bis 62 px; die Striche in den Ecken enden an
    // der Diagonale.
    let page = load(
        "<div></div>",
        "body { margin: 0 } div { width: 50px; height: 50px; margin: 10px;          outline: 2px dashed red }",
    );
    let red = Color::rgb(255, 0, 0);
    let scene = page.tree.paint(&image);
    assert!(!has_rect(&scene, red));
    let bounds = polygon_bounds(&scene, red);
    assert_eq!(bounds.len(), 20);
    assert_eq!(
        bounds.get(..5),
        Some(
            [
                (8.0, 8.0, 14.0, 10.0),
                (20.0, 8.0, 26.0, 10.0),
                (32.0, 8.0, 38.0, 10.0),
                (44.0, 8.0, 50.0, 10.0),
                (56.0, 8.0, 62.0, 10.0),
            ]
            .as_slice()
        )
    );
}

#[test]
fn long_dashed_sides_are_drawn_where_visible() {
    // Eine Million Pixel breit; gezeichnet wird nur, was im Viewport liegt.
    let page = load(
        "<div></div>",
        "body { margin: 0 } div { width: 1000000px; border-top: 1px dashed red }",
    );
    let red = Color::rgb(255, 0, 0);
    let bounds = polygon_bounds(&page.tree.paint(&image), red);
    assert!((100..200).contains(&bounds.len()), "{}", bounds.len());
    assert_eq!(bounds.first(), Some(&(0.0, 0.0, 3.0, 1.0)));
    let scrolled = polygon_bounds(&page.tree.paint_scrolled(&image, (500000.0, 0.0)), red);
    assert!((100..200).contains(&scrolled.len()), "{}", scrolled.len());
}

#[test]
fn equal_sides_meet_straight() {
    // Oben und rechts sind gleich: Ihre Ecke gehört ganz der Seite oben.
    // Unten und links sind gleich, aber anders als die beiden: schräg.
    let page = load(
        "<div></div>",
        "body { margin: 0 } div { width: 10px; height: 10px; border: 2px solid; \
         border-color: red red lime lime }",
    );
    let red = Color::rgb(255, 0, 0);
    let scene = page.tree.paint(&image);
    let painted: Vec<&[(f32, f32)]> = scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Polygon { points, color } if *color == red => Some(points.as_slice()),
            _ => None,
        })
        .collect();
    assert_eq!(
        painted,
        [
            [(0.0, 0.0), (14.0, 0.0), (14.0, 2.0), (2.0, 2.0)].as_slice(),
            [(14.0, 2.0), (14.0, 14.0), (12.0, 12.0), (12.0, 2.0)].as_slice(),
        ]
    );
}

#[test]
fn slanted_corners_fall_on_device_pixels() {
    // Bei zwei Gerätepixeln je CSS-Pixel ist der Rahmen 0,5 px breit, und
    // die Ecken liegen auf halben CSS-Pixeln.
    let page = load_scaled(
        "<div></div>",
        "body { margin: 0 } div { width: 10px; height: 10px; border: 0.5px solid;          border-color: red lime lime lime }",
        2.0,
    );
    let red = Color::rgb(255, 0, 0);
    let scene = page.tree.paint(&image);
    let top: Vec<&[(f32, f32)]> = scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Polygon { points, color } if *color == red => Some(points.as_slice()),
            _ => None,
        })
        .collect();
    assert_eq!(
        top,
        [[(0.0, 0.0), (11.0, 0.0), (10.5, 0.5), (0.5, 0.5)].as_slice()]
    );
}

#[test]
fn italic_without_italic_face_is_sheared() {
    // Ahem hat nur einen aufrechten Schnitt.
    let page = load(
        "<p>X</p><p style='font-style: italic'>X</p><p style='font-style: oblique -20deg'>X</p>",
        "",
    );
    let obliques: Vec<f32> = page
        .tree
        .paint(&image)
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Glyphs(run) => Some(run.oblique),
            _ => None,
        })
        .collect();
    assert_eq!(obliques, [0.0, 14.0, -20.0]);
}

#[test]
fn scroll_size_covers_boxes_and_lines() {
    // #a ist breiter als der Viewport. Die Zeilen von #b reichen unter den
    // Kasten, und das zu lange erste Wort ragt rechts über ihn hinaus.
    let page = load(
        "<div id=a></div><div id=b>XXXXXXXXXXXXXXXXXXXX XXXXX</div>",
        "body { margin: 0 } #a { width: 850px; height: 590px } \
         #b { margin-left: 700px; width: 100px; height: 10px; font-size: 10px; \
         line-height: 20px }",
    );
    assert_eq!(page.rect("b"), Rect::new(700.0, 590.0, 100.0, 10.0));
    assert_eq!(page.tree.scroll_size(), (900.0, 630.0));
}

#[test]
fn lines_reach_as_far_as_their_content() {
    // Der negative Außenabstand führt zurück; die Zeile ist danach 10px
    // breit, ihr Text reicht aber bis 1000px.
    let page = load(
        "<div>XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX\
         XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX\
         <span id=a>X</span></div>",
        "body { margin: 0 } div { white-space: nowrap; font-size: 10px; line-height: 10px } \
         #a { margin-left: -1000px }",
    );
    let starts: Vec<f64> = page.texts().into_iter().flat_map(|(_, x)| x).collect();
    assert_eq!(starts.len(), 101);
    assert_eq!(starts.get(99), Some(&990.0));
    assert_eq!(starts.last(), Some(&0.0));
    assert_eq!(page.tree.scroll_size(), (1000.0, 600.0));
}

#[test]
fn unreachable_overflow_does_not_count() {
    // Ganz links vom Ursprung: nicht erreichbar, auch nicht seine Höhe.
    let page = load(
        "<div style='height: 0'><div id=a></div></div>",
        "body { margin: 0 } #a { margin-left: -2000px; width: 1000px; height: 5000px }",
    );
    assert_eq!(page.rect("a"), Rect::new(-2000.0, 0.0, 1000.0, 5000.0));
    assert_eq!(page.tree.scroll_size(), (800.0, 600.0));
}

#[test]
fn painting_follows_the_scroll_position() {
    // Kästen verschieben sich mit dem Viewport, die Leinwand nicht.
    let page = load(
        "<div id=a></div>",
        "body { background: lime; margin: 0 } \
         #a { margin-top: 100px; height: 10px; background: blue }",
    );
    let scene = page.tree.paint_scrolled(&image, (5.0, 50.0));
    let rects: Vec<(mb_scene::Rect, Color)> = scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect { rect, color } => Some((*rect, *color)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rects,
        [
            (
                mb_scene::Rect::new(0.0, 0.0, 800.0, 600.0),
                Color::rgb(0, 255, 0)
            ),
            (
                mb_scene::Rect::new(-5.0, 50.0, 800.0, 10.0),
                Color::rgb(0, 0, 255)
            ),
        ]
    );
}
