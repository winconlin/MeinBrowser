//! Tests für das Laden und Darstellen von Seiten, mit Dateien im Speicher.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use mb_base::{Budget, Cancel, Exhausted};
use mb_layout::Fonts;
use mb_scene::{Color, Item, Rect};
use mb_text::FontLibrary;
use mb_url::Url;

use crate::{Page, RENDER_STEPS, Viewport, file_url, read_file, read_file_within};

/// Eine Seite aus Dateien unter `file:///t/`.
fn load(name: &str, files: &[(&str, Vec<u8>)]) -> Page {
    let files: BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(name, bytes)| (format!("file:///t/{name}"), bytes.clone()))
        .collect();
    let url = Url::parse(&format!("file:///t/{name}"), None).expect("URL");
    Page::load(
        url,
        Box::new(move |url: &Url, limit| {
            files
                .get(&url.serialize(true))
                .filter(|bytes| bytes.len() as u64 <= limit)
                .cloned()
        }),
    )
    .expect("Seite")
}

fn rects(page: &Page) -> Vec<(Rect, Color)> {
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    page.render(&fonts, 800.0, 600.0)
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect { rect, color } => Some((*rect, *color)),
            _ => None,
        })
        .collect()
}

const GREEN: Color = Color::rgb(0, 128, 0);

#[test]
fn stylesheets_in_tree_order() {
    let page = load(
        "a.html",
        &[
            (
                "a.html",
                b"<!DOCTYPE html><link rel=stylesheet href=s.css>\
                  <link rel='alternate stylesheet' href=t.css>\
                  <style type=text/plain>div { background: red }</style>\
                  <style>body { margin: 0 } div { height: 10px }</style><div></div>"
                    .to_vec(),
            ),
            ("s.css", b"div { background: green; height: 20px }".to_vec()),
            ("t.css", b"div { background: red }".to_vec()),
        ],
    );
    assert_eq!(rects(&page), [(Rect::new(0.0, 0.0, 800.0, 10.0), GREEN)]);
}

#[test]
fn meta_charset_decides_the_encoding() {
    // 0xE9 und 0xFC sind in windows-1252 `é` und `ü`, in UTF-8 ungültig.
    // Das Stylesheet ohne `@charset` übernimmt die Kodierung des Dokuments.
    let mut html = b"<!DOCTYPE html><meta charset=windows-1252><link rel=stylesheet href=s.css>\
                     <style>body { margin: 0 } .\\E9 { height: 10px; background: green }</style>\
                     <div class=&uuml;></div><div class="
        .to_vec();
    html.extend_from_slice(b"\xE9></div>");
    let page = load(
        "a.html",
        &[
            ("a.html", html),
            (
                "s.css",
                b".\xFC { height: 20px; background: green }".to_vec(),
            ),
        ],
    );
    assert_eq!(
        rects(&page),
        [
            (Rect::new(0.0, 0.0, 800.0, 20.0), GREEN),
            (Rect::new(0.0, 20.0, 800.0, 10.0), GREEN),
        ]
    );
}

#[test]
fn xhtml_and_images() {
    let png = testdata::read_bytes("image-png", "tests/pngsuite/basn2c08.png");
    let page = load(
        "a.xht",
        &[
            (
                "a.xht",
                b"<html xmlns='http://www.w3.org/1999/xhtml'><head><style>\
                  body { margin: 0; background: green } img { display: block }\
                  </style></head><body><img src='i.png'/></body></html>"
                    .to_vec(),
            ),
            ("i.png", png),
        ],
    );
    assert!(!page.document().is_html_document());
    // Der Hintergrund von body geht auf die Leinwand.
    assert_eq!(rects(&page), [(Rect::new(0.0, 0.0, 800.0, 600.0), GREEN)]);
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    let scene = page.render(&fonts, 800.0, 600.0);
    let image = scene.items.iter().find_map(|item| match item {
        Item::Image { rect, image, .. } => Some((*rect, image.width(), image.height())),
        _ => None,
    });
    assert_eq!(image, Some((Rect::new(0.0, 0.0, 32.0, 32.0), 32, 32)));
}

#[test]
fn file_urls() {
    let url = file_url(Path::new("/a b/c#d.html")).expect("URL");
    if cfg!(windows) {
        assert!(url.href().ends_with("/a%20b/c%23d.html"));
    } else {
        assert_eq!(url.href(), "file:///a%20b/c%23d.html");
    }
}

#[test]
fn file_urls_keep_literal_characters() {
    // Tabulator, Zeilenumbruch, `%` und `?` gehören zum Namen, unter Unix
    // auch `\`; der Pfad der URL ergibt dekodiert wieder den Namen.
    let name = if cfg!(windows) {
        "/a\tb\n%?.html"
    } else {
        "/a\\b\tc\n%?.html"
    };
    let url = file_url(Path::new(name)).expect("URL");
    let path = String::from_utf8(mb_url::percent::percent_decode(url.pathname().as_bytes()))
        .expect("UTF-8");
    if cfg!(windows) {
        assert!(path.ends_with(name), "{path:?}");
    } else {
        assert_eq!(path, name);
    }
}

#[test]
fn nested_nth_child_of_renders_quickly() {
    // Die kleine Datei, mit der sich der Abgleich früher festlief.
    let mut selector = "*".to_owned();
    for _ in 0..20 {
        selector = format!(":nth-child(n of {selector})");
    }
    let html = format!(
        "<!DOCTYPE html><style>body {{ margin: 0 }} div {{ height: 1px }} \
         div{selector} {{ background: green }}</style><body>{}",
        "<div></div>".repeat(30)
    );
    let page = load("a.html", &[("a.html", html.into_bytes())]);
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    let budget = Budget::new(100_000);
    let scene = page
        .render_within(&fonts, 800.0, 600.0, &budget)
        .expect("Szene");
    let green = scene
        .items
        .iter()
        .filter(|item| matches!(item, Item::Rect { color, .. } if *color == GREEN))
        .count();
    assert_eq!(green, 30);
}

#[test]
fn render_within_stops_when_the_budget_runs_out() {
    let page = load(
        "a.html",
        &[(
            "a.html",
            "<!DOCTYPE html><p>Text</p>".repeat(100).into_bytes(),
        )],
    );
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    let budget = Budget::new(1_000);
    assert_eq!(
        page.render_within(&fonts, 800.0, 600.0, &budget).err(),
        Some(Exhausted::Steps)
    );
    let cancel = Cancel::new();
    cancel.cancel();
    let budget = Budget::with_cancel(RENDER_STEPS, cancel);
    assert_eq!(
        page.render_within(&fonts, 800.0, 600.0, &budget).err(),
        Some(Exhausted::Cancelled)
    );
    let budget = Budget::new(RENDER_STEPS);
    assert!(page.render_within(&fonts, 800.0, 600.0, &budget).is_ok());
}

#[test]
fn nested_intrinsic_sizes_are_measured_once() {
    // Jede Ebene misst die darunter mit `width`, `min-width` und `max-width`
    // aus dem Inhalt; ohne gemerkte Größen wüchse die Arbeit mit 3^Tiefe.
    let open = "<span style='display: inline-block; width: min-content; \
                min-width: max-content; max-width: fit-content'>";
    let html = format!(
        "<!DOCTYPE html><body>{}x{}",
        open.repeat(30),
        "</span>".repeat(30)
    );
    let page = load("a.html", &[("a.html", html.into_bytes())]);
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    let budget = Budget::new(100_000);
    assert!(page.render_within(&fonts, 800.0, 600.0, &budget).is_ok());
}

/// Wie [`load`], aber mit einem Budget von `bytes` für die Seite.
fn load_within(name: &str, files: &[(&str, Vec<u8>)], bytes: u64) -> Page {
    let files: BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(name, bytes)| (format!("file:///t/{name}"), bytes.clone()))
        .collect();
    let url = Url::parse(&format!("file:///t/{name}"), None).expect("URL");
    Page::load_within(
        url,
        Box::new(move |url: &Url, limit| {
            files
                .get(&url.serialize(true))
                .filter(|bytes| bytes.len() as u64 <= limit)
                .cloned()
        }),
        bytes,
    )
    .expect("Seite")
}

#[test]
fn the_page_budget_covers_stylesheets_and_pixels() {
    let html = b"<!DOCTYPE html><link rel=stylesheet href=s.css>\
                 <style>body { margin: 0 } div { height: 10px; background: green }</style>\
                 <div></div><img src=i.png>"
        .to_vec();
    let css = format!("div {{ background: red }} /*{}*/", "x".repeat(100)).into_bytes();
    let png = testdata::read_bytes("image-png", "tests/pngsuite/basn2c08.png");
    let files = [
        ("a.html", html.clone()),
        ("s.css", css),
        ("i.png", png.clone()),
    ];
    let images = |page: &Page| {
        let library = FontLibrary::new(Box::new(|_| None));
        let fonts = Fonts::new(&library);
        let scene = page.render(&fonts, 800.0, 600.0);
        scene
            .items
            .iter()
            .filter(|item| matches!(item, Item::Image { .. }))
            .count()
    };
    // Genug für das Dokument und die Bilddatei, aber weder für das
    // Stylesheet noch für die 32 × 32 Pixel des Bildes.
    let html_bytes = u64::try_from(html.len()).expect("Länge");
    let png_bytes = u64::try_from(png.len()).expect("Länge");
    let page = load_within("a.html", &files, html_bytes + png_bytes + 100);
    assert_eq!(rects(&page), [(Rect::new(0.0, 0.0, 800.0, 10.0), GREEN)]);
    assert_eq!(images(&page), 0);
    let page = load_within("a.html", &files, crate::PAGE_BYTES);
    assert_eq!(images(&page), 1);
}

/// Ein eigener Ordner je Test (RS-14).
fn directory(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("mb-web-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join("seite").join("support")).expect("Ordner");
    directory
}

#[test]
fn local_pages_load_normal_files_from_anywhere() {
    let directory = directory("ordner");
    let path = directory.join("seite").join("a.html");
    std::fs::write(
        &path,
        "<!DOCTYPE html><link rel=stylesheet href=support/s.css>\
         <link rel=stylesheet href=../t.css><div></div>",
    )
    .expect("schreiben");
    std::fs::write(
        directory.join("seite").join("support").join("s.css"),
        "body { margin: 0 } div { height: 10px; background: red }",
    )
    .expect("schreiben");
    std::fs::write(directory.join("t.css"), "div { background: green }").expect("schreiben");
    let url = file_url(&path).expect("URL");
    let page = Page::load(url, Box::new(read_file_within)).expect("Seite");
    assert_eq!(rects(&page), [(Rect::new(0.0, 0.0, 800.0, 10.0), GREEN)]);
    std::fs::remove_dir_all(&directory).expect("aufräumen");
}

#[cfg(unix)]
#[test]
fn special_files_are_refused_at_once() {
    let directory = directory("speziell");
    let fifo = directory.join("seite").join("pipe.css");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(made.success());
    let path = directory.join("seite").join("a.html");
    std::fs::write(
        &path,
        "<!DOCTYPE html><link rel=stylesheet href=pipe.css>\
         <link rel=stylesheet href=file:///dev/zero><p>Text",
    )
    .expect("schreiben");
    let url = file_url(&path).expect("URL");
    // Bliebe das Laden an der Pipe hängen, endete der Test nie.
    assert!(Page::load(url, Box::new(read_file_within)).is_some());
    let zero = Url::parse("file:///dev/zero", None).expect("URL");
    assert_eq!(read_file(&zero), None);
    assert_eq!(read_file(&file_url(&fifo).expect("URL")), None);
    std::fs::remove_dir_all(&directory).expect("aufräumen");
}

/// Die Zahl der Bilder in der Szene der Seite.
fn image_count(page: &Page) -> usize {
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    page.render(&fonts, 800.0, 600.0)
        .items
        .iter()
        .filter(|item| matches!(item, Item::Image { .. }))
        .count()
}

#[test]
fn images_are_decoded_only_within_the_budget() {
    let html = b"<!DOCTYPE html><img src=i.png>".to_vec();
    // 32 × 32 Pixel mit 16 Bit RGBA.
    let png = testdata::read_bytes("image-png", "tests/pngsuite/basn6a16.png");
    let files = [("a.html", html.clone()), ("i.png", png.clone())];
    let length = |bytes: &[u8]| u64::try_from(bytes.len()).expect("Länge");
    // Dokument, Bilddatei, die Kopie der Bilddaten im Dekoder und dessen
    // Puffer.
    let needed = length(&html) + 2 * length(&png) + 32 * 32 * mb_image::png::DECODE_BYTES_PER_PIXEL;
    assert_eq!(image_count(&load_within("a.html", &files, needed)), 1);
    assert_eq!(image_count(&load_within("a.html", &files, needed - 1)), 0);
}

#[test]
fn pages_scroll_and_know_the_device_pixel_ratio() {
    let page = load(
        "a.html",
        &[(
            "a.html",
            b"<!DOCTYPE html><style>body { margin: 0 } \
              div { height: 1000px; border-top: 0.5px solid green }</style><div></div>"
                .to_vec(),
        )],
    );
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    let budget = Budget::new(RENDER_STEPS);
    let viewport = Viewport {
        width: 400.0,
        height: 300.0,
        device_pixel_ratio: 2.0,
    };
    let tree = page
        .layout_within(&fonts, viewport, &budget)
        .expect("Layout");
    // Ein halber CSS-Pixel ist hier ein ganzes Gerätepixel und bleibt.
    assert_eq!(tree.scroll_size(), (400.0, 1000.5));
    let scene = page.paint(&tree, (0.0, 100.0));
    assert_eq!((scene.width, scene.height), (400.0, 300.0));
    let rects: Vec<(Rect, Color)> = scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect { rect, color } => Some((*rect, *color)),
            _ => None,
        })
        .collect();
    assert_eq!(rects, [(Rect::new(0.0, -100.0, 400.0, 0.5), GREEN)]);
}

/// Die Rechtecke der Szene in einem Viewport von `width` × 300 CSS-Pixeln.
fn rects_at(page: &Page, width: f64) -> Vec<(Rect, Color)> {
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    page.render(&fonts, width, 300.0)
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect { rect, color } => Some((*rect, *color)),
            _ => None,
        })
        .collect()
}

#[test]
fn media_attributes_and_queries_follow_the_viewport() {
    let page = load(
        "a.html",
        &[
            (
                "a.html",
                b"<!DOCTYPE html><style>body { margin: 0 } div { height: 10px }</style>\
                  <style media='(min-width: 500px)'>div { background: green }</style>\
                  <style media=print>div { background: red }</style>\
                  <link rel=stylesheet href=s.css media='(max-width: 499px)'>\
                  <style>@media (max-width: 499px) { div { height: 20px } }</style><div></div>"
                    .to_vec(),
            ),
            ("s.css", b"div { background: blue }".to_vec()),
        ],
    );
    let blue = Color::rgb(0, 0, 255);
    assert_eq!(
        rects_at(&page, 800.0),
        [(Rect::new(0.0, 0.0, 800.0, 10.0), GREEN)]
    );
    // Ein schmaler Viewport stellt die Regeln neu zusammen, ein breiter
    // wieder zurück.
    assert_eq!(
        rects_at(&page, 400.0),
        [(Rect::new(0.0, 0.0, 400.0, 20.0), blue)]
    );
    assert_eq!(
        rects_at(&page, 600.0),
        [(Rect::new(0.0, 0.0, 600.0, 10.0), GREEN)]
    );
}

#[test]
fn imports_load_once_per_rule_without_cycles() {
    let files: BTreeMap<String, Vec<u8>> = [
        (
            "a.html",
            "<!DOCTYPE html><link rel=stylesheet href=css/main.css>\
             <style>@import 'css/inline.css' layer(x);</style><div class=é></div>"
                .as_bytes()
                .to_vec(),
        ),
        (
            "css/main.css",
            b"@charset \"iso-8859-1\"; @import 'parts/size.css'; \
              @import url(parts/never.css) supports(colour: red); @import 'main.css'; \
              @import 'parts/narrow.css' (max-width: 499px); body { margin: 0 }"
                .to_vec(),
        ),
        (
            "css/parts/size.css",
            b"@import '../main.css'; @import 'deep.css'; div { height: 10px }".to_vec(),
        ),
        // In der Kodierung von `main.css`, die über `size.css` weitergeht.
        (
            "css/parts/deep.css",
            b".\xE9 { background: green }".to_vec(),
        ),
        ("css/parts/narrow.css", b"div { height: 30px }".to_vec()),
        ("css/parts/never.css", b"div { background: red }".to_vec()),
        (
            "css/inline.css",
            b"div { background: red; height: 50px }".to_vec(),
        ),
    ]
    .into_iter()
    .map(|(name, bytes)| (format!("file:///t/{name}"), bytes))
    .collect();
    let fetched = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&fetched);
    let url = Url::parse("file:///t/a.html", None).expect("URL");
    let page = Page::load(
        url,
        Box::new(move |url: &Url, limit| {
            let key = url.serialize(true);
            log.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(key.trim_start_matches("file:///t/").to_owned());
            files
                .get(&key)
                .filter(|bytes| bytes.len() as u64 <= limit)
                .cloned()
        }),
    )
    .expect("Seite");
    let fetched = fetched
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    // Nicht geladen werden `never.css` (`supports()` ist nicht erfüllt) und
    // die Stylesheets, die sich selbst einbinden würden.
    assert_eq!(
        fetched,
        [
            "a.html",
            "css/main.css",
            "css/parts/size.css",
            "css/parts/deep.css",
            "css/parts/narrow.css",
            "css/inline.css",
        ]
    );
    // Die Ebene `x` verliert gegen die Regeln ohne Ebene.
    assert_eq!(
        rects_at(&page, 800.0),
        [(Rect::new(0.0, 0.0, 800.0, 10.0), GREEN)]
    );
    assert_eq!(
        rects_at(&page, 400.0),
        [(Rect::new(0.0, 0.0, 400.0, 30.0), GREEN)]
    );
}

fn rects_within(page: &Page, width: f64, budget: &Budget) -> Result<Vec<(Rect, Color)>, Exhausted> {
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    page.render_within(&fonts, width, 300.0, budget)
        .map(|scene| {
            scene
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Rect { rect, color } => Some((*rect, *color)),
                    _ => None,
                })
                .collect()
        })
}

#[test]
fn failed_preparation_never_caches_partial_rules() {
    let layers = (0..2048)
        .map(|i| format!("layer{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let html = format!(
        "<!DOCTYPE html><style>@layer {layers}; body {{ margin: 0 }}
         div {{ height: 10px; background: green }}</style><div></div>"
    );
    let page = load("a.html", &[("a.html", html.into_bytes())]);
    assert_eq!(
        rects_within(&page, 800.0, &Budget::new(0)),
        Err(Exhausted::Steps)
    );
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(
        rects_within(&page, 800.0, &Budget::with_cancel(RENDER_STEPS, cancel)),
        Err(Exhausted::Cancelled)
    );
    assert_eq!(
        rects_within(&page, 800.0, &Budget::new(128)),
        Err(Exhausted::Steps)
    );
    assert_eq!(
        rects_within(&page, 800.0, &Budget::new(RENDER_STEPS)).expect("vollständige Regeln"),
        [(Rect::new(0.0, 0.0, 800.0, 10.0), GREEN)]
    );
}

#[test]
fn cached_media_validation_stops_when_its_budget_is_spent() {
    // Alle Bedingungen bleiben falsch. Der Cache muss deshalb die ganze
    // Liste auswerten, bevor er seine Gültigkeit bestätigen könnte.
    let media = "@media print { .unused { color: red } }".repeat(10_000);
    let html = format!(
        "<!DOCTYPE html><style>{media} body {{ margin: 0 }}
         div {{ height: 10px; background: green }}</style><div></div>"
    );
    let page = load("a.html", &[("a.html", html.into_bytes())]);
    let expected = [(Rect::new(0.0, 0.0, 800.0, 10.0), GREEN)];
    assert_eq!(
        rects_within(&page, 800.0, &Budget::new(RENDER_STEPS)).expect("Cache aufbauen"),
        expected
    );
    // Für dieses kleine Layout reicht das Budget; nicht aber für alle
    // 10.000 Bedingungen des vorbereiteten Caches.
    let simple = load("a.html", &[("a.html", b"<!DOCTYPE html><style>body { margin: 0 } div { height: 10px; background: green }</style><div></div>".to_vec())]);
    let _ = rects_at(&simple, 800.0);
    assert!(rects_within(&simple, 800.0, &Budget::new(2_000)).is_ok());
    assert_eq!(
        rects_within(&page, 800.0, &Budget::new(2_000)),
        Err(Exhausted::Steps)
    );
    assert_eq!(
        rects_within(&page, 800.0, &Budget::new(RENDER_STEPS)).expect("Cache bleibt nutzbar"),
        expected
    );
}

#[test]
fn failed_viewport_rebuild_preserves_the_previous_stylist() {
    let unused = (0..4096)
        .map(|i| format!(".unused{i} {{ color: red }}"))
        .collect::<String>();
    let html = format!(
        "<!DOCTYPE html><style>{unused} body {{ margin: 0 }}
         div {{ height: 10px; background: green }}
         @media (max-width: 499px) {{ div {{ background: blue }} }}
         </style><div></div>"
    );
    let files = [("a.html", html.into_bytes())];
    let page = load("a.html", &files);
    let green = [(Rect::new(0.0, 0.0, 800.0, 10.0), GREEN)];
    assert_eq!(
        rects_within(&page, 800.0, &Budget::new(RENDER_STEPS)).expect("breiter Cache"),
        green
    );
    // Der alte Cache gilt im schmalen Viewport nicht. Der fehlgeschlagene
    // Neuaufbau darf ihn weder ersetzen noch vorzeitig entfernen.
    assert_eq!(
        rects_within(&page, 400.0, &Budget::new(128)),
        Err(Exhausted::Steps)
    );
    let fresh = load("a.html", &files);
    assert_eq!(
        rects_within(&fresh, 800.0, &Budget::new(2_000)),
        Err(Exhausted::Steps)
    );
    assert_eq!(
        rects_within(&page, 800.0, &Budget::new(2_000)).expect("vorheriger Cache"),
        green
    );
    assert_eq!(
        rects_within(&page, 400.0, &Budget::new(RENDER_STEPS)).expect("neuer schmaler Cache"),
        [(Rect::new(0.0, 0.0, 400.0, 10.0), Color::rgb(0, 0, 255))]
    );
}
