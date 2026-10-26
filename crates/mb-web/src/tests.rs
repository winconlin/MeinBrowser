//! Tests für das Laden und Darstellen von Seiten, mit Dateien im Speicher.

use std::collections::BTreeMap;
use std::path::Path;

use mb_layout::Fonts;
use mb_scene::{Color, Item, Rect};
use mb_text::FontLibrary;
use mb_url::Url;

use crate::{Page, file_url};

/// Eine Seite aus Dateien unter `file:///t/`.
fn load(name: &str, files: &[(&str, Vec<u8>)]) -> Page {
    let files: BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(name, bytes)| (format!("file:///t/{name}"), bytes.clone()))
        .collect();
    let url = Url::parse(&format!("file:///t/{name}"), None).expect("URL");
    Page::load(
        url,
        Box::new(move |url: &Url| files.get(&url.serialize(true)).cloned()),
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
