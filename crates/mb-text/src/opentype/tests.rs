//! Tests für den Schriftleser: mit Schriften aus den WPT (`fonts/`), deren
//! erwartete Werte mit fontTools ermittelt sind, und mit kleinen, hier
//! gebauten Schriften für Fälle, die diese Schriften nicht enthalten.

use super::*;
use crate::outline::{PathCommand, Point};

fn font(path: &str) -> FontData {
    testdata::read_bytes("wpt", &format!("fonts/{path}")).into()
}

fn face(path: &str) -> Face {
    Face::parse(font(path), 0).unwrap_or_else(|error| panic!("{path}: {error:?}"))
}

fn glyph(face: &Face, character: char) -> GlyphId {
    face.glyph_index(character)
        .unwrap_or_else(|| panic!("keine Glyphe für {character:?}"))
}

fn point(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

/// Ob ein Punkt bis auf 10^-3 mit dem Wert aus fontTools übereinstimmt,
/// den fontTools mit `f64` berechnet.
fn close_to(actual: (f32, f32, bool), expected: (f64, f64, bool)) -> bool {
    (f64::from(actual.0) - expected.0).abs() < 1e-3
        && (f64::from(actual.1) - expected.1).abs() < 1e-3
        && actual.2 == expected.2
}

#[test]
fn ahem() {
    let data = font("Ahem.ttf");
    assert_eq!(face_count(&data), Ok(1));
    let face = Face::parse(data, 0).expect("Ahem");
    assert_eq!(face.glyph_count(), 278);
    let metrics = face.metrics();
    assert_eq!(metrics.units_per_em, 1000);
    assert_eq!(
        (metrics.ascender, metrics.descender, metrics.line_gap),
        (800, -200, 0)
    );
    assert_eq!(
        (
            metrics.typo_ascender,
            metrics.typo_descender,
            metrics.typo_line_gap
        ),
        (Some(800), Some(-200), Some(0))
    );
    assert_eq!(
        (metrics.win_ascent, metrics.win_descent),
        (Some(800), Some(-200))
    );
    assert_eq!(
        (metrics.x_height, metrics.cap_height),
        (Some(800), Some(800))
    );
    assert_eq!(
        (metrics.underline_position, metrics.underline_thickness),
        (Some(-133), Some(20))
    );
    assert!(!metrics.use_typo_metrics);
    assert_eq!(metrics.italic_angle, 0.0);
    assert_eq!(
        face.style(),
        FaceStyle {
            weight: 400,
            width: 5,
            italic: false,
            oblique: false
        }
    );
    let family: Vec<&str> = face
        .names()
        .iter()
        .filter(|name| name.id == 1)
        .map(|name| name.value.as_str())
        .collect();
    assert_eq!(family, ["Ahem", "Ahem", "Ahem"]);

    // Die vier Glyphen aus der Beschreibung von Ahem in den WPT.
    let x = glyph(&face, 'X');
    assert_eq!(x, GlyphId(58));
    assert_eq!(face.advance_width(x), Some(1000));
    assert_eq!(face.left_side_bearing(x), Some(0));
    assert_eq!(
        face.outline(x).expect("X").commands,
        [
            PathCommand::MoveTo(point(0.0, 800.0)),
            PathCommand::LineTo(point(1000.0, 800.0)),
            PathCommand::LineTo(point(1000.0, -200.0)),
            PathCommand::LineTo(point(0.0, -200.0)),
            PathCommand::Close,
        ]
    );
    let p = glyph(&face, 'p');
    assert_eq!(p, GlyphId(82));
    assert_eq!(
        face.outline(p).expect("p").control_bounds(),
        Some((point(0.0, -200.0), point(1000.0, 0.0)))
    );
    let e = glyph(&face, 'É');
    assert_eq!(e, GlyphId(100));
    assert_eq!(
        face.outline(e).expect("É").control_bounds(),
        Some((point(0.0, 0.0), point(1000.0, 800.0)))
    );
    let space = glyph(&face, ' ');
    assert_eq!(space, GlyphId(3));
    assert_eq!(face.advance_width(space), Some(1000));
    assert!(face.outline(space).expect("Leerzeichen").is_empty());
    assert_eq!(face.glyph_index('\u{10000}'), None);
    assert_eq!(face.advance_width(GlyphId(278)), None);
}

#[test]
fn composite_glyphs() {
    let face = face("Lato-Medium.ttf");
    assert_eq!(face.glyph_count(), 3023);
    assert_eq!(face.style().weight, 500);
    let metrics = face.metrics();
    assert_eq!(metrics.units_per_em, 2000);
    assert_eq!(
        (metrics.x_height, metrics.cap_height),
        (Some(1016), Some(1436))
    );
    let glyphs = face.glyphs().expect("glyf");

    // Á: A und ein Akzent, um 269 verschoben.
    let aacute = glyph(&face, 'Á');
    assert_eq!(aacute, GlyphId(1836));
    assert_eq!(face.advance_width(aacute), Some(1363));
    assert_eq!(face.left_side_bearing(aacute), Some(2));
    let (points, ends) = glyphs.raw_points(aacute.0).expect("Á");
    assert_eq!(ends, [7, 21, 33]);
    for (index, expected) in [
        (0, (940.0, 535.0, true)),
        (2, (704.0, 1149.0, false)),
        (20, (574.0, 1436.0, true)),
        (33, (850.0, 1854.0, true)),
    ] {
        let actual = points.get(index).copied();
        assert!(
            actual.is_some_and(|actual| close_to(actual, expected)),
            "Á, Punkt {index}: {actual:?}"
        );
    }

    // Glyphe 1798: drei Komponenten mit getrennten Faktoren für x und y.
    let (points, ends) = glyphs.raw_points(1798).expect("Glyphe 1798");
    assert_eq!(ends, [31, 51, 71]);
    for (index, expected) in [
        (0, (-276.7347, 1728.2101, true)),
        (1, (-254.6851, 1728.2101, false)),
        (31, (-303.5778, 1728.2101, false)),
        (32, (-287.4604, 1532.8892, true)),
        (52, (-284.4604, 1882.8892, true)),
        (71, (-284.4604, 1903.2791, false)),
    ] {
        let actual = points.get(index).copied();
        assert!(
            actual.is_some_and(|actual| close_to(actual, expected)),
            "Glyphe 1798, Punkt {index}: {actual:?}"
        );
    }
}

#[test]
fn character_maps() {
    // Format 12 in (3, 10) geht der BMP-Tabelle vor.
    let deseret = face("noto/NotoSansDeseret-Regular.ttf");
    assert_eq!(deseret.glyph_index('\u{10400}'), Some(GlyphId(4)));
    assert_eq!(deseret.glyph_index('\u{1044F}'), Some(GlyphId(83)));
    assert_eq!(deseret.glyph_index(' '), Some(GlyphId(3)));
    assert_eq!(deseret.glyph_index('\u{10450}'), None);

    // Format 4 in (3, 1), daneben Format 6 für Macintosh Roman.
    let lato = face("Lato-Medium.ttf");
    assert_eq!(lato.glyph_index('A'), Some(GlyphId(3)));
    assert_eq!(lato.glyph_index('o'), Some(GlyphId(111)));
    assert_eq!(lato.glyph_index('€'), Some(GlyphId(360)));
    let table = lato.table(b"cmap").expect("cmap");
    let mac = cmap::subtables(table)
        .expect("Untertabellen")
        .into_iter()
        .find(|subtable| (subtable.platform, subtable.encoding) == (1, 0))
        .expect("Macintosh");
    assert_eq!(mac.format, 6);
    for (code, expected) in [(0x41, 3), (0xDB, 360), (0x8A, 2204), (0xFF, 176)] {
        assert_eq!(cmap::lookup(table, mac, code), Some(expected), "{code:#x}");
    }
    assert_eq!(cmap::lookup(table, mac, 0x100), None);

    // Format 0.
    let canvas = face("CanvasTest.ttf");
    let table = canvas.table(b"cmap").expect("cmap");
    let mac = cmap::subtables(table)
        .expect("Untertabellen")
        .into_iter()
        .find(|subtable| subtable.format == 0)
        .expect("Format 0");
    for (code, expected) in [
        (0x41, Some(8)),
        (0x20, Some(7)),
        (0x09, Some(3)),
        (0x100, None),
    ] {
        assert_eq!(cmap::lookup(table, mac, code), expected, "{code:#x}");
    }
}

#[test]
fn cff_outlines_are_not_supported_yet() {
    let face = face("noto/cjk-kr/NotoSansCJKkr-Regular-subset.otf");
    assert!(face.has_table(Tag(*b"CFF ")));
    let glyph = GlyphId(1);
    assert_eq!(
        face.outline(glyph),
        Err(FontError::Unsupported("CFF-Umrisse"))
    );
}

#[test]
fn collections() {
    // Ahem als einziger Schnitt einer Sammlung: Kopf davor, die Versätze im
    // Tabellenverzeichnis um dessen Länge verschoben.
    let ahem = font("Ahem.ttf");
    let mut collection = Vec::new();
    collection.extend_from_slice(b"ttcf");
    collection.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    collection.extend_from_slice(&1u32.to_be_bytes());
    collection.extend_from_slice(&16u32.to_be_bytes());
    let header = collection.len();
    collection.extend_from_slice(&ahem);
    let tables = usize::from(u16::from_be_bytes([ahem[4], ahem[5]]));
    for table in 0..tables {
        let field = header + 12 + table * 16 + 8;
        let old = u32::from_be_bytes(collection[field..field + 4].try_into().expect("4 Bytes"));
        collection[field..field + 4].copy_from_slice(&(old + 16).to_be_bytes());
    }
    let collection: FontData = collection.into();
    assert_eq!(face_count(&collection), Ok(1));
    let face = Face::parse(collection.clone(), 0).expect("Schnitt 0");
    assert_eq!(face.glyph_index('X'), Some(GlyphId(58)));
    assert!(Face::parse(collection, 1).is_err());
}

#[test]
fn broken_files() {
    let ahem = font("Ahem.ttf");
    let truncated: FontData = ahem[..100].into();
    assert_eq!(Face::parse(truncated, 0).err(), Some(FontError::Truncated));
    let woff: FontData = b"wOFF\0\0\0\0".as_slice().into();
    assert_eq!(face_count(&woff), Err(FontError::Unsupported("WOFF")));
    let text: FontData = b"<!DOCTYPE html>".as_slice().into();
    assert_eq!(Face::parse(text, 0).err(), Some(FontError::UnknownFormat));
    assert_eq!(face_count(&[]), Err(FontError::Truncated));
}

// ---------------------------------------------------------------------------
// Gebaute Schriften

/// Eine Kontur einer einfachen Glyphe: Punkte mit der Angabe, ob sie auf
/// der Kurve liegen.
type Contour = Vec<(i16, i16, bool)>;

/// Eine einfache Glyphe, alle Koordinaten mit 16 Bits.
fn simple_glyph(contours: &[Contour]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(
        &i16::try_from(contours.len())
            .expect("Konturen")
            .to_be_bytes(),
    );
    data.extend_from_slice(&[0; 8]);
    let mut end: i32 = -1;
    for contour in contours {
        end += i32::try_from(contour.len()).expect("Punkte");
        data.extend_from_slice(&u16::try_from(end).expect("Ende").to_be_bytes());
    }
    data.extend_from_slice(&0u16.to_be_bytes());
    let points: Vec<&(i16, i16, bool)> = contours.iter().flatten().collect();
    for &&(_, _, on_curve) in &points {
        data.push(u8::from(on_curve));
    }
    let mut previous = 0;
    for &&(x, _, _) in &points {
        data.extend_from_slice(&(x - previous).to_be_bytes());
        previous = x;
    }
    previous = 0;
    for &&(_, y, _) in &points {
        data.extend_from_slice(&(y - previous).to_be_bytes());
        previous = y;
    }
    data
}

/// Eine Komponente: Flags, Glyphe, Argumente (mit 16 Bits) und die Werte
/// der Matrix als F2DOT14.
struct Component {
    flags: u16,
    glyph: u16,
    arguments: (i16, i16),
    matrix: Vec<i16>,
}

fn composite_glyph(components: &[Component]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&(-1i16).to_be_bytes());
    data.extend_from_slice(&[0; 8]);
    for (index, component) in components.iter().enumerate() {
        let mut flags = component.flags | ARGS_ARE_WORDS;
        if index + 1 < components.len() {
            flags |= MORE_COMPONENTS;
        }
        data.extend_from_slice(&flags.to_be_bytes());
        data.extend_from_slice(&component.glyph.to_be_bytes());
        data.extend_from_slice(&component.arguments.0.to_be_bytes());
        data.extend_from_slice(&component.arguments.1.to_be_bytes());
        for value in &component.matrix {
            data.extend_from_slice(&value.to_be_bytes());
        }
    }
    data
}

const ARGS_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const HAVE_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;

fn offset_component(glyph: u16, x: i16, y: i16) -> Component {
    Component {
        flags: ARGS_ARE_XY_VALUES,
        glyph,
        arguments: (x, y),
        matrix: Vec::new(),
    }
}

/// Eine Schrift aus Glyphen, mit langen Versätzen in `loca`.
/// `advances` sind die langen Einträge in `hmtx`, `bearings` die Abstände
/// danach.
fn build_font(glyphs: &[Vec<u8>], advances: &[(u16, i16)], bearings: &[i16]) -> FontData {
    let glyph_count = u16::try_from(glyphs.len()).expect("Glyphen");
    let mut head = vec![0; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    let mut hhea = vec![0; 36];
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&u16::try_from(advances.len()).expect("hmtx").to_be_bytes());
    let mut maxp = vec![0, 0, 0x50, 0];
    maxp.extend_from_slice(&glyph_count.to_be_bytes());
    let mut hmtx = Vec::new();
    for &(advance, bearing) in advances {
        hmtx.extend_from_slice(&advance.to_be_bytes());
        hmtx.extend_from_slice(&bearing.to_be_bytes());
    }
    for bearing in bearings {
        hmtx.extend_from_slice(&bearing.to_be_bytes());
    }
    // Eine Untertabelle (3, 1) im Format 4 mit nur dem letzten Abschnitt.
    let mut cmap = Vec::new();
    for value in [0u16, 1, 3, 1, 0, 12] {
        cmap.extend_from_slice(&value.to_be_bytes());
    }
    for value in [4u16, 24, 0, 2, 2, 0, 0, 0xFFFF, 0, 0xFFFF, 1, 0] {
        cmap.extend_from_slice(&value.to_be_bytes());
    }
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for data in glyphs {
        loca.extend_from_slice(&u32::try_from(glyf.len()).expect("glyf").to_be_bytes());
        glyf.extend_from_slice(data);
        if glyf.len() % 2 != 0 {
            glyf.push(0);
        }
    }
    loca.extend_from_slice(&u32::try_from(glyf.len()).expect("glyf").to_be_bytes());
    let tables: [(&[u8; 4], Vec<u8>); 7] = [
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
    ];
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    font.extend_from_slice(&u16::try_from(tables.len()).expect("Tabellen").to_be_bytes());
    font.extend_from_slice(&[0; 6]);
    let mut position = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        font.extend_from_slice(*tag);
        font.extend_from_slice(&0u32.to_be_bytes());
        font.extend_from_slice(&u32::try_from(position).expect("Versatz").to_be_bytes());
        font.extend_from_slice(&u32::try_from(data.len()).expect("Länge").to_be_bytes());
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        position = 12 + 16 * tables.len() + body.len();
    }
    font.extend_from_slice(&body);
    font.into()
}

fn built_face(glyphs: &[Vec<u8>]) -> Face {
    let advances: Vec<(u16, i16)> = glyphs.iter().map(|_| (500, 0)).collect();
    Face::parse(build_font(glyphs, &advances, &[]), 0).expect("gebaute Schrift")
}

#[test]
fn contours_starting_off_the_curve() {
    let face = built_face(&[
        Vec::new(),
        // Nur Punkte neben der Kurve: Anfang in der Mitte zwischen dem
        // ersten und dem letzten.
        simple_glyph(&[vec![
            (0, 0, false),
            (100, 0, false),
            (100, 100, false),
            (0, 100, false),
        ]]),
        // Der letzte Punkt liegt auf der Kurve und wird zum Anfang.
        simple_glyph(&[vec![(0, 0, false), (100, 0, true), (0, 100, true)]]),
    ]);
    assert_eq!(
        face.outline(GlyphId(1)).expect("Glyphe 1").commands,
        [
            PathCommand::MoveTo(point(0.0, 50.0)),
            PathCommand::QuadTo(point(0.0, 0.0), point(50.0, 0.0)),
            PathCommand::QuadTo(point(100.0, 0.0), point(100.0, 50.0)),
            PathCommand::QuadTo(point(100.0, 100.0), point(50.0, 100.0)),
            PathCommand::QuadTo(point(0.0, 100.0), point(0.0, 50.0)),
            PathCommand::Close,
        ]
    );
    assert_eq!(
        face.outline(GlyphId(2)).expect("Glyphe 2").commands,
        [
            PathCommand::MoveTo(point(0.0, 100.0)),
            PathCommand::QuadTo(point(0.0, 0.0), point(100.0, 0.0)),
            PathCommand::Close,
        ]
    );
}

#[test]
fn component_placement() {
    let square = simple_glyph(&[vec![
        (0, 0, true),
        (10, 0, true),
        (10, 10, true),
        (0, 10, true),
    ]]);
    let triangle = simple_glyph(&[vec![(5, 5, true), (6, 5, true), (6, 6, true)]]);
    let half = 8192; // 0,5 als F2DOT14
    let face = built_face(&[
        Vec::new(),
        square,
        triangle,
        // Punkt 0 des Dreiecks fällt auf Punkt 2 des Quadrats.
        composite_glyph(&[
            offset_component(1, 0, 0),
            Component {
                flags: 0,
                glyph: 2,
                arguments: (2, 0),
                matrix: Vec::new(),
            },
        ]),
        // Halb so groß, der Versatz unskaliert und skaliert.
        composite_glyph(&[Component {
            flags: ARGS_ARE_XY_VALUES | HAVE_SCALE,
            glyph: 2,
            arguments: (10, 20),
            matrix: vec![half],
        }]),
        composite_glyph(&[Component {
            flags: ARGS_ARE_XY_VALUES | HAVE_SCALE | SCALED_COMPONENT_OFFSET,
            glyph: 2,
            arguments: (10, 20),
            matrix: vec![half],
        }]),
        // Geschachtelt.
        composite_glyph(&[offset_component(3, 100, 0)]),
    ]);
    let glyphs = face.glyphs().expect("glyf");
    let (points, ends) = glyphs.raw_points(3).expect("Glyphe 3");
    assert_eq!(ends, [3, 6]);
    assert_eq!(
        points.get(4..),
        Some([(10.0, 10.0, true), (11.0, 10.0, true), (11.0, 11.0, true)].as_slice())
    );
    let first = |glyph| glyphs.raw_points(glyph).expect("Glyphe").0.first().copied();
    assert_eq!(first(4), Some((12.5, 22.5, true)));
    assert_eq!(first(5), Some((7.5, 12.5, true)));
    let (points, ends) = glyphs.raw_points(6).expect("Glyphe 6");
    assert_eq!(ends, [3, 6]);
    assert_eq!(points.get(6), Some(&(111.0, 11.0, true)));
}

#[test]
fn composite_limits() {
    // Eine Glyphe, die sich selbst enthält.
    let face = built_face(&[Vec::new(), composite_glyph(&[offset_component(1, 0, 0)])]);
    assert_eq!(
        face.outline(GlyphId(1)),
        Err(FontError::Malformed("Glyphen zu tief geschachtelt"))
    );
    // Eine Kette von Glyphen mit je 100 Komponenten der nächsten; ohne
    // Grenze wären es 100^14 Besuche.
    let mut glyphs = vec![Vec::new()];
    for level in 1..15u16 {
        let components: Vec<Component> = (0..100)
            .map(|_| offset_component(level + 1, 0, 0))
            .collect();
        glyphs.push(composite_glyph(&components));
    }
    glyphs.push(simple_glyph(&[]));
    let face = built_face(&glyphs);
    assert_eq!(
        face.outline(GlyphId(1)),
        Err(FontError::Malformed("zu viele Komponenten"))
    );
    // Eine Komponente, die es nicht gibt.
    let face = built_face(&[Vec::new(), composite_glyph(&[offset_component(9, 0, 0)])]);
    assert_eq!(
        face.outline(GlyphId(1)),
        Err(FontError::Malformed("Glyphe außerhalb der Schrift"))
    );
}

#[test]
fn glyphs_without_contours() {
    // Nur der Kopf mit null Konturen, ohne die Länge der Anweisungen, und
    // eine zusammengesetzte Glyphe daraus.
    let mut header_only = 0i16.to_be_bytes().to_vec();
    header_only.extend_from_slice(&[0; 8]);
    let face = built_face(&[
        Vec::new(),
        header_only,
        composite_glyph(&[offset_component(1, 5, 5)]),
    ]);
    for glyph in [1, 2] {
        let outline = face.outline(GlyphId(glyph));
        assert!(
            outline.as_ref().is_ok_and(Outline::is_empty),
            "Glyphe {glyph}: {outline:?}"
        );
    }
}

#[test]
fn horizontal_metrics_with_trailing_bearings() {
    let glyphs = vec![Vec::new(); 5];
    let data = build_font(&glyphs, &[(600, 10), (700, 20)], &[30, 40, 50]);
    let face = Face::parse(data, 0).expect("gebaute Schrift");
    assert_eq!(face.advance_width(GlyphId(1)), Some(700));
    assert_eq!(face.advance_width(GlyphId(4)), Some(700));
    assert_eq!(face.left_side_bearing(GlyphId(0)), Some(10));
    assert_eq!(face.left_side_bearing(GlyphId(2)), Some(30));
    assert_eq!(face.left_side_bearing(GlyphId(4)), Some(50));
    assert_eq!(face.left_side_bearing(GlyphId(5)), None);
    // Die Untertabelle ordnet keinem Zeichen eine Glyphe zu.
    assert_eq!(face.glyph_index('A'), None);
}
