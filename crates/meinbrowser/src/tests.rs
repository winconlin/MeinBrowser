use mb_platform::{Key, KeyEvent, Modifiers, NamedKey, ScrollDelta};

use super::{Limits, Renderer, View, key_scroll, wheel_scroll};

#[test]
fn renderer_draws_a_local_file() {
    let path =
        std::env::temp_dir().join(format!("meinbrowser-{}-anzeige.html", std::process::id()));
    let html = "<!DOCTYPE html><body style='margin: 0; background: #00ff00'>\
                <div style='width: 10px; height: 10px; background: #0000ff'></div>";
    std::fs::write(&path, html).unwrap();
    let renderer = Renderer::start(&path);
    std::fs::remove_file(&path).unwrap();
    let renderer = renderer.unwrap();
    let canvas = renderer.render(40, 30).unwrap();
    assert_eq!((canvas.width(), canvas.height()), (40, 30));
    assert_eq!(canvas.pixel(5, 5), Some(0xff00_00ff));
    assert_eq!(canvas.pixel(20, 20), Some(0xff00_ff00));
    // Die Seite bleibt geladen und passt sich neuen Größen an.
    let canvas = renderer.render(12, 8).unwrap();
    assert_eq!((canvas.width(), canvas.height()), (12, 8));
    assert_eq!(canvas.pixel(11, 7), Some(0xff00_ff00));
}

#[test]
fn missing_files_are_reported() {
    let path = std::env::temp_dir().join(format!("meinbrowser-{}-fehlt.html", std::process::id()));
    let error = Renderer::start(&path).err().unwrap();
    assert!(error.ends_with("lässt sich nicht laden"), "{error}");
}

#[test]
fn requests_can_be_cancelled_without_waiting() {
    let path =
        std::env::temp_dir().join(format!("meinbrowser-{}-auftraege.html", std::process::id()));
    std::fs::write(&path, "<!DOCTYPE html><body style='background: #00ff00'>").unwrap();
    let renderer = Renderer::start(&path);
    std::fs::remove_file(&path).unwrap();
    let renderer = renderer.unwrap();
    let first = renderer.request(40, 30).unwrap();
    first.cancel.cancel();
    let second = renderer.request(12, 8).unwrap();
    let canvas = loop {
        if let Some(result) = renderer.poll(&second) {
            break result.unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    assert_eq!((canvas.width(), canvas.height()), (12, 8));
    assert_eq!(canvas.pixel(0, 0), Some(0xff00_ff00));
}

#[test]
fn renderer_scrolls_and_scales() {
    let path =
        std::env::temp_dir().join(format!("meinbrowser-{}-scrollen.html", std::process::id()));
    let html = "<!DOCTYPE html><body style='margin: 0; background: #00ff00'>\
                <div style='height: 1000px'></div>\
                <div style='height: 10px; background: #0000ff'></div>";
    std::fs::write(&path, html).unwrap();
    let renderer = Renderer::start(&path);
    std::fs::remove_file(&path).unwrap();
    let renderer = renderer.unwrap();
    // 40 × 30 Gerätepixel bei zwei je CSS-Pixel: ein Viewport von 20 × 15.
    // Gewünscht ist mehr, als geht; es bleibt beim Ende des Dokuments.
    let view = View {
        width: 40,
        height: 30,
        scale: 2.0,
        scroll: (5.0, 2000.0),
    };
    let canvas = renderer.render_view(view).unwrap();
    assert_eq!((canvas.width(), canvas.height()), (40, 30));
    let limits = renderer.limits().unwrap();
    assert_eq!(limits.max, (0.0, 995.0));
    assert_eq!(limits.scroll, (0.0, 995.0));
    // Der blaue Kasten liegt bei 1000 bis 1010, im Bild bei 5 bis 15 CSS-
    // Pixeln, also in den Zeilen 10 bis 29.
    assert_eq!(canvas.pixel(4, 9), Some(0xff00_ff00));
    assert_eq!(canvas.pixel(4, 10), Some(0xff00_00ff));
    assert_eq!(canvas.pixel(4, 29), Some(0xff00_00ff));
    // Rechts liegt der Balken, bei 2 bis 13 CSS-Pixeln mit hellem Rand: Die
    // Bahn ist kürzer als seine kleinste Länge, er füllt sie ganz.
    assert_ne!(canvas.pixel(30, 25), Some(0xff00_00ff));
    assert_ne!(canvas.pixel(30, 2), Some(0xff00_ff00));
    assert_eq!(canvas.pixel(30, 1), Some(0xff00_ff00));
}

fn key(key: Key, shift: bool) -> KeyEvent {
    KeyEvent {
        key,
        pressed: true,
        repeat: false,
        modifiers: Modifiers {
            shift,
            ..Modifiers::default()
        },
    }
}

#[test]
fn keys_scroll_by_lines_and_pages() {
    let limits = Limits {
        scroll: (0.0, 0.0),
        max: (100.0, 1000.0),
    };
    let scroll = |pressed: Key, shift: bool, at: (f64, f64)| {
        key_scroll(&key(pressed, shift), 300.0, limits, at)
    };
    let named = Key::Named;
    assert_eq!(
        scroll(named(NamedKey::ArrowDown), false, (0.0, 0.0)),
        Some((0.0, 40.0))
    );
    assert_eq!(
        scroll(named(NamedKey::ArrowUp), false, (0.0, 10.0)),
        Some((0.0, 0.0))
    );
    assert_eq!(
        scroll(named(NamedKey::ArrowRight), false, (90.0, 0.0)),
        Some((100.0, 0.0))
    );
    // Eine Seite ist die Höhe ohne eine Zeile.
    assert_eq!(
        scroll(named(NamedKey::PageDown), false, (0.0, 0.0)),
        Some((0.0, 260.0))
    );
    assert_eq!(
        scroll(Key::Character(' '), false, (0.0, 0.0)),
        Some((0.0, 260.0))
    );
    assert_eq!(
        scroll(Key::Character(' '), true, (0.0, 300.0)),
        Some((0.0, 40.0))
    );
    assert_eq!(
        scroll(named(NamedKey::End), false, (0.0, 0.0)),
        Some((0.0, 1000.0))
    );
    assert_eq!(
        scroll(named(NamedKey::Home), false, (5.0, 500.0)),
        Some((5.0, 0.0))
    );
    assert_eq!(scroll(Key::Character('a'), false, (0.0, 0.0)), None);
    // Tastenkürzel und losgelassene Tasten scrollen nicht.
    let mut control = key(named(NamedKey::ArrowDown), false);
    control.modifiers.control = true;
    assert_eq!(key_scroll(&control, 300.0, limits, (0.0, 0.0)), None);
    let mut released = key(named(NamedKey::ArrowDown), false);
    released.pressed = false;
    assert_eq!(key_scroll(&released, 300.0, limits, (0.0, 0.0)), None);
}

#[test]
fn wheels_scroll_by_steps_and_pixels() {
    let limits = Limits {
        scroll: (0.0, 0.0),
        max: (0.0, 1000.0),
    };
    // Eine Raste sind drei Zeilen.
    let steps = ScrollDelta::Steps { x: 0.0, y: 1.0 };
    assert_eq!(wheel_scroll(steps, 1.0, limits, (0.0, 0.0)), (0.0, 120.0));
    // Pixel sind Gerätepixel.
    let pixels = ScrollDelta::Pixels { x: 10.0, y: 30.0 };
    assert_eq!(
        wheel_scroll(pixels, 2.0, limits, (0.0, 100.0)),
        (0.0, 115.0)
    );
    let back = ScrollDelta::Steps { x: 0.0, y: -1.0 };
    assert_eq!(wheel_scroll(back, 1.0, limits, (0.0, 50.0)), (0.0, 0.0));
}
