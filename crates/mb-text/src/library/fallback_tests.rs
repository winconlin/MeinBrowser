//! Der Suchcache darf weder neue Schriften verstecken noch die Wahl ändern.

use super::*;

fn path(file: &str) -> PathBuf {
    testdata::root().join("wpt").join("fonts").join(file)
}

fn library(files: &[&str]) -> FontLibrary {
    let mut library = FontLibrary::new(Box::new(|path| std::fs::read(path).ok()));
    for file in files {
        assert_eq!(library.add_file(path(file)), 1, "{file}");
    }
    library
}

fn query(weight: f32, slope: Slope) -> FontQuery<'static> {
    FontQuery {
        families: &[],
        weight,
        width: 100.0,
        slope,
    }
}

fn cached(library: &FontLibrary, query: &FontQuery<'_>, chars: &[char]) -> Option<Option<FaceId>> {
    let key = FallbackKey::new(query, chars)?;
    library
        .cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fallback
        .get(&key)
}

fn postscript(library: &FontLibrary, id: FaceId) -> String {
    library
        .face(id)
        .expect("Schrift")
        .names()
        .iter()
        .find(|name| name.id == 6)
        .expect("PostScript-Name")
        .value
        .clone()
}

#[test]
fn a_new_earlier_family_invalidates_a_cached_match() {
    let mut library = library(&["CSSTest/csstest-basic-regular.ttf"]);
    let query = query(400.0, Slope::Normal);
    let old = library.fallback(&query, &['A']).expect("CSSTest Basic");
    assert_eq!(postscript(&library, old), "CSSTestBasic");
    assert_eq!(cached(&library, &query, &['A']), Some(Some(old)));
    assert_eq!(library.add_file(path("Ahem.ttf")), 1);
    assert_eq!(cached(&library, &query, &['A']), None);
    let new = library.fallback(&query, &['A']).expect("Ahem");
    assert_ne!(new, old);
    assert_eq!(postscript(&library, new), "Ahem");
}

#[test]
fn a_new_font_invalidates_a_cached_missing_character() {
    let mut library = library(&["Ahem.ttf"]);
    let query = query(400.0, Slope::Normal);
    assert_eq!(library.fallback(&query, &['ő']), None);
    assert_eq!(cached(&library, &query, &['ő']), Some(None));
    assert_eq!(
        library.add_file(path("CSSTest/csstest-basic-regular.ttf")),
        1
    );
    assert_eq!(cached(&library, &query, &['ő']), None);
    let found = library
        .fallback(&query, &['ő'])
        .expect("neue Ersatzschrift");
    assert_eq!(postscript(&library, found), "CSSTestBasic");
}

#[test]
fn failed_additions_do_not_invalidate_existing_results() {
    let mut library = library(&["Ahem.ttf"]);
    let query = query(400.0, Slope::Normal);
    assert_eq!(library.fallback(&query, &['ő']), None);
    assert_eq!(library.add_file(path("missing-fallback-font.ttf")), 0);
    assert_eq!(cached(&library, &query, &['ő']), Some(None));
}

#[test]
fn cached_results_keep_weights_and_slopes_separate() {
    let library = library(&[
        "CSSTest/csstest-basic-regular.ttf",
        "CSSTest/csstest-basic-bold.ttf",
        "CSSTest/csstest-basic-italic.ttf",
    ]);
    let cases = [
        (400.0, Slope::Normal, "CSSTestBasic"),
        (700.0, Slope::Normal, "CSSTestBasic-Bold"),
        (400.0, Slope::Italic, "CSSTestBasic-Italic"),
    ];
    for _ in 0..2 {
        for (weight, slope, expected) in cases {
            let query = query(weight, slope);
            let found = library.fallback(&query, &['ő']).expect("Ersatzschrift");
            assert_eq!(postscript(&library, found), expected);
            assert_eq!(cached(&library, &query, &['ő']), Some(Some(found)));
        }
    }
}

#[test]
fn long_clusters_and_empty_requests_keep_their_results() {
    let library = library(&["Ahem.ttf"]);
    let query = query(400.0, Slope::Normal);
    let expected = library.fallback(&query, &['A']).expect("Ahem");
    assert_eq!(library.fallback(&query, &['A'; 64]), Some(expected));
    assert_eq!(cached(&library, &query, &['A'; 64]), Some(Some(expected)));
    assert_eq!(library.fallback(&query, &['A'; 65]), Some(expected));
    assert!(FallbackKey::new(&query, &['A'; 65]).is_none());
    assert_eq!(library.fallback(&query, &[]), None);
}

#[test]
fn shaping_reuses_the_same_fallback_and_notdef_glyphs() {
    let library = library(&["Ahem.ttf", "CSSTest/csstest-basic-regular.ttf"]);
    let families = [Family::Name("Ahem".to_owned())];
    let query = FontQuery {
        families: &families,
        ..query(400.0, Slope::Normal)
    };
    let text: Vec<u32> = "Aő\u{10ffff}Aő\u{10ffff}".chars().map(u32::from).collect();
    let first = crate::shape(&library, &query, 16.0, &text);
    assert_eq!(first.len(), text.len());
    assert_eq!(postscript(&library, first[1].face), "CSSTestBasic");
    assert_eq!(first[2].id.0, 0, "fehlendes Zeichen bleibt .notdef");
    assert_eq!(crate::shape(&library, &query, 16.0, &text), first);
}
