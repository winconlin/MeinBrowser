//! Tests für die Bibliothek mit Schriften aus den WPT (`fonts/`), vor allem
//! den CSSTest-Schriften, die für Tests der Schriftauswahl gebaut sind. Was
//! welche Datei enthält, ist mit fontTools nachgesehen.

use super::*;

fn path(file: &str) -> PathBuf {
    let path = testdata::root().join("wpt").join("fonts").join(file);
    assert!(
        path.is_file(),
        "Externe Testdaten fehlen ({}). Bitte `cargo run -p testdata` ausführen.",
        path.display()
    );
    path
}

fn library(files: &[&str]) -> FontLibrary {
    let mut library = FontLibrary::new(Box::new(|path: &Path| std::fs::read(path).ok()));
    for file in files {
        assert_eq!(library.add_file(path(file)), 1, "{file}");
    }
    library
}

fn query(families: &[Family], weight: f32, slope: Slope) -> FontQuery<'_> {
    FontQuery {
        families,
        weight,
        width: 100.0,
        slope,
    }
}

fn name(name: &str) -> Family {
    Family::Name(name.to_owned())
}

/// Der PostScript-Name eines Schnitts.
fn postscript(library: &FontLibrary, id: FaceId) -> String {
    let face = library.face(id).expect("Schnitt lässt sich laden");
    face.names()
        .iter()
        .find(|name| name.id == 6)
        .map(|name| name.value.clone())
        .expect("PostScript-Name")
}

fn selected(library: &FontLibrary, query: &FontQuery<'_>) -> Vec<String> {
    library
        .select(query)
        .into_iter()
        .map(|id| postscript(library, id))
        .collect()
}

const BASIC: [&str; 4] = [
    "CSSTest/csstest-basic-regular.ttf",
    "CSSTest/csstest-basic-bold.ttf",
    "CSSTest/csstest-basic-italic.ttf",
    "CSSTest/csstest-basic-bolditalic.ttf",
];

#[test]
fn localized_names_match_caselessly() {
    let library = library(&[
        "CSSTest/csstest-familyname.ttf",
        "CSSTest/csstest-familyname-bold.ttf",
    ]);
    assert_eq!(library.face_count(), 2);
    assert_eq!(library.family("CSSTest FamilyName").len(), 2);
    assert_eq!(library.family("csstest FAMILYNAME").len(), 2);
    // Der japanische Name, auch mit kleinen Vollbreitenbuchstaben.
    assert_eq!(library.family("ＣＳＳテスト\u{3000}フォント名").len(), 2);
    assert_eq!(library.family("ｃｓｓテスト\u{3000}フォント名").len(), 2);
    assert!(library.family("CSSTest FamilyName Bold").is_empty());
    assert!(library.family("CSSTest").is_empty());
    let families = [name("ｃｓｓテスト\u{3000}フォント名")];
    let bold = library.select(&query(&families, 700.0, Slope::Normal));
    let [bold] = bold.as_slice() else {
        panic!("ein Schnitt erwartet: {bold:?}");
    };
    assert_eq!(library.face(*bold).expect("geladen").style().weight, 700);
}

#[test]
fn weights_follow_the_search_order() {
    let library = library(&[
        "CSSTest/csstest-weights-1479-w1.ttf",
        "CSSTest/csstest-weights-1479-w4.ttf",
        "CSSTest/csstest-weights-1479-w7.ttf",
        "CSSTest/csstest-weights-1479-w9.ttf",
    ]);
    let families = [name("CSSTest Weights W1479")];
    let chosen: Vec<u16> = (1..=9_u8)
        .map(|step| {
            let query = query(&families, f32::from(step) * 100.0, Slope::Normal);
            let id = *library.select(&query).first().expect("ein Schnitt");
            library.face(id).expect("geladen").style().weight
        })
        .collect();
    assert_eq!(chosen, [100, 100, 100, 400, 400, 700, 700, 900, 900]);
    // Der Familienname für alte Programme (Nummer 1) enthält nur einen
    // Schnitt.
    let families = [name("CSSTest Weights W1479 W7")];
    assert_eq!(
        selected(&library, &query(&families, 100.0, Slope::Normal)),
        ["CSSTestWeightsW1479-W7"]
    );
}

#[test]
fn styles_and_weights_in_one_family() {
    let library = library(&BASIC);
    let families = [name("CSSTest Basic")];
    let cases = [
        (400.0, Slope::Normal, "CSSTestBasic"),
        (700.0, Slope::Normal, "CSSTestBasic-Bold"),
        (400.0, Slope::Italic, "CSSTestBasic-Italic"),
        (900.0, Slope::Italic, "CSSTestBasic-BoldItalic"),
        // Ohne schrägen Schnitt kursiv.
        (400.0, Slope::Oblique(20.0), "CSSTestBasic-Italic"),
        // Unter 11deg wird nach dem aufrechten Schnitt gesucht.
        (400.0, Slope::Oblique(5.0), "CSSTestBasic"),
    ];
    for (weight, slope, expected) in cases {
        assert_eq!(
            selected(&library, &query(&families, weight, slope)),
            [expected],
            "{weight} {slope:?}"
        );
    }
}

#[test]
fn font_list_in_order_without_missing_families() {
    let library = library(&["Lato-Bold.ttf", "Lato-Medium.ttf", "Ahem.ttf"]);
    let families = [
        name("Gibt es nicht"),
        name("Lato"),
        Family::Generic(GenericFamily::Serif),
        name("lato"),
        name("Ahem"),
    ];
    assert_eq!(
        selected(&library, &query(&families, 400.0, Slope::Normal)),
        ["Lato-Medium", "Ahem"]
    );
    assert_eq!(
        selected(&library, &query(&families, 600.0, Slope::Normal)),
        ["Lato-Bold", "Ahem"]
    );
    let families = [name("Lato Medium")];
    assert_eq!(
        selected(&library, &query(&families, 700.0, Slope::Normal)),
        ["Lato-Medium"]
    );
    // Ohne Familie aus `serif`, `sans-serif` und `monospace` ist die Schrift
    // des Browsers die erste nach gefaltetem Namen.
    let none = [Family::Generic(GenericFamily::Monospace)];
    let query = query(&none, 400.0, Slope::Normal);
    assert!(library.select(&query).is_empty());
    let default = library.default_face(&query).expect("eine Schrift");
    assert_eq!(postscript(&library, default), "Ahem");
}

#[test]
fn generic_families_take_the_first_installed() {
    assert_eq!(GenericFamily::Serif.families().first(), Some(&"Noto Serif"));
    assert!(GenericFamily::Math.families().is_empty());
    // Ohne installierte Familie passt `serif` auf nichts. Der Name `serif`
    // (in Anführungszeichen) ist keine generische Familie und passt auch
    // nicht.
    let library = library(&["Ahem.ttf"]);
    let families = [Family::Generic(GenericFamily::Serif), name("serif")];
    assert!(
        library
            .select(&query(&families, 400.0, Slope::Normal))
            .is_empty()
    );
}

#[test]
fn fallback_searches_families_by_name() {
    let mut files = vec!["Ahem.ttf", "Lato-Bold.ttf", "Lato-Medium.ttf"];
    files.extend(BASIC);
    let library = library(&files);
    let fallback = |weight: f32, character: char| {
        let query = query(&[], weight, Slope::Normal);
        library
            .fallback(&query, character)
            .map(|id| postscript(&library, id))
    };
    // Die Familien nach gefaltetem Namen: ahem, csstest basic, lato,
    // lato medium.
    assert_eq!(fallback(400.0, '一').as_deref(), Some("Ahem"));
    assert_eq!(fallback(400.0, 'ő').as_deref(), Some("CSSTestBasic"));
    assert_eq!(fallback(700.0, 'ő').as_deref(), Some("CSSTestBasic-Bold"));
    // U+0122 hat nur Lato Medium. Bei 700 ist in `lato` der fette Schnitt
    // der passende; Glyphen anderer Schnitte der Familie zählen nicht, also
    // folgt `lato medium`.
    assert_eq!(fallback(400.0, '\u{122}').as_deref(), Some("Lato-Medium"));
    assert_eq!(fallback(700.0, '\u{122}').as_deref(), Some("Lato-Medium"));
    assert_eq!(fallback(400.0, 'あ'), None);
}

#[test]
fn faces_are_loaded_once() {
    let library = library(&["Ahem.ttf"]);
    let id = library.family("Ahem")[0];
    let first = library.face(id).expect("geladen");
    let second = library.face(id).expect("geladen");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.glyph_count(), 278);
    assert!(library.face(FaceId(5)).is_none());
}

#[test]
fn unreadable_files_add_nothing() {
    let mut library = FontLibrary::new(Box::new(|path: &Path| match path.to_str() {
        Some("kaputt.ttf") => Some(b"keine Schrift".to_vec()),
        Some("leer.ttf") => Some(Vec::new()),
        _ => None,
    }));
    assert_eq!(library.add_file(PathBuf::from("fehlt.ttf")), 0);
    assert_eq!(library.add_file(PathBuf::from("kaputt.ttf")), 0);
    assert_eq!(library.add_file(PathBuf::from("leer.ttf")), 0);
    assert_eq!(library.face_count(), 0);
    let query = query(&[], 400.0, Slope::Normal);
    assert!(library.default_face(&query).is_none());
    assert!(library.fallback(&query, 'a').is_none());
}

/// Eine Sammlung, deren Kopf mehr Schnitte verspricht, als hineinpassen,
/// liest nur so viele, wie Versätze darin Platz haben.
#[test]
fn collections_with_too_many_faces() {
    let mut header = b"ttcf".to_vec();
    header.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    header.extend_from_slice(&u32::MAX.to_be_bytes());
    header.extend_from_slice(&[0; 8]);
    let mut library = FontLibrary::new(Box::new(move |_: &Path| Some(header.clone())));
    assert_eq!(library.add_file(PathBuf::from("sammlung.ttc")), 0);
}
