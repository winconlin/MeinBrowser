//! Tests für die einfache Textformung mit Schriften aus den WPT. Glyphen
//! und Breiten sind mit fontTools nachgesehen: Ahem hat 1000 Einheiten je
//! Geviert und jede Glyphe ist ein Geviert breit, Lato Medium hat 2000
//! Einheiten, CSSTest Basic 2048.

use std::path::{Path, PathBuf};

use super::*;
use crate::library::Family;
use crate::matching::Slope;

fn library(files: &[&str]) -> FontLibrary {
    let mut library = FontLibrary::new(Box::new(|path: &Path| std::fs::read(path).ok()));
    for file in files {
        let path: PathBuf = testdata::root().join("wpt").join("fonts").join(file);
        assert!(
            path.is_file(),
            "Externe Testdaten fehlen ({}). Bitte `cargo run -p testdata` ausführen.",
            path.display()
        );
        assert_eq!(library.add_file(path), 1, "{file}");
    }
    library
}

fn names(names: &[&str]) -> Vec<Family> {
    names
        .iter()
        .map(|name| Family::Name((*name).to_owned()))
        .collect()
}

fn shaped(library: &FontLibrary, families: &[Family], size: f32, text: &str) -> Vec<Glyph> {
    let query = FontQuery {
        families,
        weight: 400.0,
        width: 100.0,
        slope: Slope::Normal,
    };
    let text: Vec<u32> = text.chars().map(u32::from).collect();
    shape(library, &query, size, &text)
}

fn family_of(library: &FontLibrary, name: &str) -> FaceId {
    *library.family(name).first().expect("Familie")
}

#[test]
fn glyphs_and_advances() {
    let library = library(&["Ahem.ttf", "Lato-Medium.ttf"]);
    let ahem = family_of(&library, "Ahem");
    let lato = family_of(&library, "Lato");
    let glyphs = shaped(&library, &names(&["Ahem"]), 16.0, "AB ");
    assert_eq!(
        glyphs,
        [
            Glyph {
                face: ahem,
                id: GlyphId(35),
                cluster: 0,
                advance: 16.0
            },
            Glyph {
                face: ahem,
                id: GlyphId(36),
                cluster: 1,
                advance: 16.0
            },
            Glyph {
                face: ahem,
                id: GlyphId(3),
                cluster: 2,
                advance: 16.0
            },
        ]
    );
    let glyphs = shaped(&library, &names(&["Lato"]), 20.0, "AB");
    assert_eq!(
        glyphs,
        [
            Glyph {
                face: lato,
                id: GlyphId(3),
                cluster: 0,
                advance: 1363.0 * 20.0 / 2000.0
            },
            Glyph {
                face: lato,
                id: GlyphId(6),
                cluster: 1,
                advance: 1299.0 * 20.0 / 2000.0
            },
        ]
    );
}

#[test]
fn characters_fall_back_through_the_list() {
    let library = library(&["Ahem.ttf", "Lato-Medium.ttf"]);
    let ahem = family_of(&library, "Ahem");
    let lato = family_of(&library, "Lato");
    // Ahem hat kein ő, Lato schon.
    let glyphs = shaped(&library, &names(&["Ahem", "Lato"]), 20.0, "Aő");
    assert_eq!(
        glyphs,
        [
            Glyph {
                face: ahem,
                id: GlyphId(35),
                cluster: 0,
                advance: 20.0
            },
            Glyph {
                face: lato,
                id: GlyphId(2395),
                cluster: 1,
                advance: 1138.0 * 20.0 / 2000.0
            },
        ]
    );
    // Auch ohne Lato in der Liste: dann als Ersatzschrift.
    let glyphs = shaped(&library, &names(&["Ahem"]), 20.0, "ő");
    assert_eq!(
        glyphs.iter().map(|glyph| glyph.face).collect::<Vec<_>>(),
        [lato]
    );
}

#[test]
fn clusters_stay_in_one_font() {
    let library = library(&[
        "Ahem.ttf",
        "Lato-Medium.ttf",
        "CSSTest/csstest-basic-regular.ttf",
    ]);
    let ahem = family_of(&library, "Ahem");
    let lato = family_of(&library, "Lato");
    let basic = family_of(&library, "CSSTest Basic");
    // Ahem hat U+0301 nicht, aber é: das kanonisch gleiche Zeichen.
    let glyphs = shaped(&library, &names(&["Ahem", "Lato"]), 10.0, "e\u{301}x");
    assert_eq!(
        glyphs,
        [
            Glyph {
                face: ahem,
                id: GlyphId(111),
                cluster: 0,
                advance: 10.0
            },
            Glyph {
                face: ahem,
                id: GlyphId(90),
                cluster: 2,
                advance: 10.0
            },
        ]
    );
    // Für B mit Akut gibt es kein Zeichen. Lato hat beide und kommt in der
    // Liste nach Ahem; der ganze Cluster nimmt Lato.
    let glyphs = shaped(&library, &names(&["Ahem", "Lato"]), 10.0, "B\u{301}");
    assert_eq!(
        glyphs,
        [
            Glyph {
                face: lato,
                id: GlyphId(6),
                cluster: 0,
                advance: 1299.0 * 10.0 / 2000.0
            },
            Glyph {
                face: lato,
                id: GlyphId(1577),
                cluster: 1,
                advance: 0.0
            },
        ]
    );
    // Ohne Lato in der Liste: die Ersatzschrift für den ganzen Cluster,
    // die erste Familie nach Namen mit beiden Zeichen.
    let faces: Vec<FaceId> = shaped(&library, &names(&["Ahem"]), 10.0, "B\u{301}")
        .iter()
        .map(|glyph| glyph.face)
        .collect();
    assert_eq!(faces, [basic, basic]);
}

#[test]
fn longest_supported_prefix_then_single_characters() {
    let library = library(&["Ahem.ttf"]);
    let ahem = family_of(&library, "Ahem");
    let notdef = library
        .face(ahem)
        .and_then(|face| face.advance_width(GlyphId(0)))
        .map(|units| f32::from(units) * 10.0 / 1000.0)
        .expect("Breite von .notdef");
    // Keine Schrift hat U+0301: B aus Ahem, der Akut als *.notdef*.
    let glyphs = shaped(&library, &names(&["Ahem"]), 10.0, "B\u{301}");
    assert_eq!(
        glyphs,
        [
            Glyph {
                face: ahem,
                id: GlyphId(36),
                cluster: 0,
                advance: 10.0
            },
            Glyph {
                face: ahem,
                id: GlyphId(0),
                cluster: 1,
                advance: notdef
            },
        ]
    );
    // Ein Variantenselektor ohne Schrift bleibt ohne Glyphe.
    let glyphs = shaped(&library, &names(&["Ahem"]), 10.0, "A\u{FE0F}B");
    assert_eq!(
        glyphs
            .iter()
            .map(|glyph| (glyph.id, glyph.cluster))
            .collect::<Vec<_>>(),
        [(GlyphId(35), 0), (GlyphId(36), 2)]
    );
    // Ohne passende Familie nimmt die erste verfügbare Schrift *.notdef*,
    // hier die Schrift des Browsers.
    let glyphs = shaped(&library, &names(&["Gibt es nicht"]), 10.0, "あ");
    assert_eq!(
        glyphs,
        [Glyph {
            face: ahem,
            id: GlyphId(0),
            cluster: 0,
            advance: notdef
        }]
    );
    // Ein einzelnes Surrogat gilt als U+FFFD, das Ahem nicht hat.
    let query = FontQuery {
        families: &names(&["Ahem"]),
        weight: 400.0,
        width: 100.0,
        slope: Slope::Normal,
    };
    let glyphs = shape(&library, &query, 10.0, &[0xD800, 0x41]);
    assert_eq!(
        glyphs.iter().map(|glyph| glyph.id).collect::<Vec<_>>(),
        [GlyphId(0), GlyphId(35)]
    );
}

#[test]
fn without_fonts_nothing_is_shaped() {
    let library = FontLibrary::new(Box::new(|_: &Path| None));
    assert!(shaped(&library, &names(&["Ahem"]), 10.0, "abc").is_empty());
    assert!(shaped(&library, &[], 10.0, "").is_empty());
}
