//! Der begrenzte Cache darf das Ergebnis der Schriftauswahl nicht ändern.

use super::*;

fn query(weight: f32) -> Query {
    Query {
        families: vec![Family::Name("Ahem".to_owned())],
        weight,
        width: 100.0,
        slope: Slope::Normal,
    }
}

#[test]
fn real_font_metrics_survive_eviction_and_keep_their_scaling() {
    let mut library = FontLibrary::new(Box::new(|path| std::fs::read(path).ok()));
    let path = testdata::root().join("wpt").join("fonts").join("Ahem.ttf");
    assert_eq!(library.add_file(path), 1, "Ahem fehlt");
    let fonts = Fonts::new(&library);
    let original = query(400.0);
    let expected = fonts.metrics(&original, 16.0);
    assert_eq!(expected.ascent, 12.8);
    assert_eq!(expected.descent, 3.2);
    let twice = fonts.metrics(&original, 32.0);
    assert_eq!(twice.ascent, expected.ascent * 2.0);
    assert_eq!(twice.descent, expected.descent * 2.0);
    // Mehr Schlüssel als der Cache behalten darf. Gewicht und Breite
    // sind Teil der Anfrage, die einzige vorhandene Schrift bleibt Ahem.
    for weight in 1..=1024 {
        assert_eq!(fonts.metrics(&query(weight as f32), 16.0), expected);
    }
    let mut cache = fonts.metrics.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(
        cache.get(&original.key()).is_none(),
        "alter Eintrag verdrängt"
    );
    drop(cache);
    assert_eq!(fonts.metrics(&original, 16.0), expected);
}

#[test]
fn missing_fonts_keep_the_same_fallback_after_eviction() {
    let library = FontLibrary::new(Box::new(|_| panic!("keine Schriftdatei registriert")));
    let fonts = Fonts::new(&library);
    let original = query(400.0);
    let expected = Metrics::fallback(20.0);
    assert_eq!(fonts.metrics(&original, 20.0), expected);
    for weight in 1..=1024 {
        assert_eq!(fonts.metrics(&query(weight as f32), 20.0), expected);
    }
    assert_eq!(fonts.metrics(&original, 20.0), expected);
    assert_eq!(fonts.metrics(&original, 40.0), Metrics::fallback(40.0));
}
