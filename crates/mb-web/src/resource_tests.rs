//! Ressourcenabrufe müssen das verbleibende Ladebudget beachten.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use mb_layout::Fonts;
use mb_scene::{Color, Item, Rect, Scene};
use mb_text::FontLibrary;
use mb_url::Url;

use crate::{Fetch, MAX_FILE_BYTES, Page, file_url, read_file_within};

type Calls = Arc<Mutex<Vec<(String, u64)>>>;

fn url(name: &str) -> Url {
    Url::parse(&format!("file:///budget/{name}"), None).expect("URL")
}

fn length(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).expect("Länge")
}

fn fetch(files: &[(&str, Vec<u8>)]) -> (Fetch, Calls) {
    let files: BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(name, bytes)| (url(name).serialize(true), bytes.clone()))
        .collect();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&calls);
    let fetch = Box::new(move |url: &Url, limit: u64| {
        let key = url.serialize(true);
        log.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key, limit));
        files
            .get(&url.serialize(true))
            .filter(|bytes| length(bytes) <= limit)
            .cloned()
    });
    (fetch, calls)
}

fn calls(log: &Calls) -> Vec<(String, u64)> {
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(url, limit)| (url.trim_start_matches("file:///budget/").to_owned(), *limit))
        .collect()
}

fn render(page: &Page) -> Scene {
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    page.render(&fonts, 800.0, 600.0)
}

fn green_rects(page: &Page) -> Vec<Rect> {
    render(page)
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Rect { rect, color } if color == Color::rgb(0, 128, 0) => Some(rect),
            _ => None,
        })
        .collect()
}

#[test]
fn zero_budget_never_starts_the_document_fetch() {
    let (fetch, log) = fetch(&[("a.html", b"<!DOCTYPE html><p>Text".to_vec())]);
    assert!(Page::load_within(url("a.html"), fetch, 0).is_none());
    assert!(calls(&log).is_empty());
}

#[test]
fn an_exhausted_document_budget_never_fetches_subresources() {
    let html = b"<!DOCTYPE html><link rel=stylesheet href=linked.css>\
                 <style>@import 'imported.css'; body { margin: 0 }\
                 div { height: 10px; background-image: url(background.png) }</style>\
                 <div></div><img src=image.png>"
        .to_vec();
    let budget = length(&html);
    let (fetch, log) = fetch(&[("a.html", html)]);
    let page = Page::load_within(url("a.html"), fetch, budget).expect("Dokument");
    // Hintergrundbilder werden beim Zeichnen, img schon beim Layout geladen.
    // Auch weitere Darstellungen dürfen keinen neuen Abruf beginnen.
    for _ in 0..2 {
        assert!(
            render(&page)
                .items
                .iter()
                .all(|item| !matches!(item, Item::Image { .. }))
        );
    }
    assert_eq!(calls(&log), [("a.html".to_owned(), budget)]);
}

#[test]
fn nested_stylesheets_receive_only_the_remaining_bytes() {
    let html = b"<!DOCTYPE html><link rel=stylesheet href=main.css>\
                 <link rel=stylesheet href=unused.css><div></div>"
        .to_vec();
    let main = b"@import 'nested.css'; @import 'never.css';\
                 body { margin: 0 } div { height: 10px }"
        .to_vec();
    let nested = b"div { background: green }".to_vec();
    let document_bytes = length(&html);
    let main_bytes = length(&main);
    let nested_bytes = length(&nested);
    let budget = document_bytes + main_bytes + nested_bytes;
    let (fetch, log) = fetch(&[("a.html", html), ("main.css", main), ("nested.css", nested)]);
    let page = Page::load_within(url("a.html"), fetch, budget).expect("Dokument");
    assert_eq!(green_rects(&page), [Rect::new(0.0, 0.0, 800.0, 10.0)]);
    assert_eq!(
        calls(&log),
        [
            ("a.html".to_owned(), budget),
            ("main.css".to_owned(), main_bytes + nested_bytes),
            ("nested.css".to_owned(), nested_bytes),
        ]
    );
}

#[test]
fn an_oversized_document_from_a_callback_is_rejected() {
    // Auch ein fehlerhafter Fetch, der das übergebene Limit missachtet,
    // darf keinen zu großen Puffer an den Parser weitergeben.
    let page = Page::load_within(
        url("a.html"),
        Box::new(|_, limit| {
            assert_eq!(limit, 1);
            Some(b"<!DOCTYPE html><p>Text".to_vec())
        }),
        1,
    );
    assert!(page.is_none());
}

#[test]
fn a_rejected_oversized_resource_does_not_hide_a_smaller_valid_one() {
    let html = b"<!DOCTYPE html><link rel=stylesheet href=oversized.css>\
                 <link rel=stylesheet href=small.css>\
                 <link rel=stylesheet href=never.css><div></div>"
        .to_vec();
    let css = b"body { margin: 0 } div { height: 10px; background: green }".to_vec();
    let rest = length(&css);
    let budget = length(&html) + rest;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&calls);
    let page = Page::load_within(
        url("a.html"),
        Box::new(move |url, limit| {
            let pathname = url.pathname();
            let name = pathname.trim_start_matches("/budget/");
            log.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((name.to_owned(), limit));
            match name {
                "a.html" => Some(html.clone()),
                "oversized.css" => Some(vec![b'x'; usize::try_from(limit + 1).expect("Länge")]),
                "small.css" => Some(css.clone()),
                _ => panic!("Kein Budget für weiteren Abruf"),
            }
        }),
        budget,
    )
    .expect("Dokument");
    assert_eq!(green_rects(&page), [Rect::new(0.0, 0.0, 800.0, 10.0)]);
    assert_eq!(
        *calls.lock().unwrap_or_else(PoisonError::into_inner),
        [
            ("a.html".to_owned(), budget),
            ("oversized.css".to_owned(), rest),
            ("small.css".to_owned(), rest),
        ]
    );
}

#[test]
fn a_failed_image_is_not_fetched_again_on_repaint() {
    let html = b"<!DOCTYPE html><style>div { height: 10px;\
                 background-image: url(missing.png) }</style>\
                 <div></div><img src=missing.png>"
        .to_vec();
    let budget = length(&html) + 100;
    let (fetch, log) = fetch(&[("a.html", html)]);
    let page = Page::load_within(url("a.html"), fetch, budget).expect("Dokument");
    let _ = render(&page);
    let _ = render(&page);
    assert_eq!(
        calls(&log),
        [
            ("a.html".to_owned(), budget),
            ("missing.png".to_owned(), 100),
        ]
    );
}

#[test]
fn local_reads_observe_both_the_remaining_budget_and_the_file_limit() {
    let directory =
        std::env::temp_dir().join(format!("mb-web-resource-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("Ordner");
    let path = directory.join("bounded.css");
    let bytes = b"12345678";
    std::fs::write(&path, bytes).expect("Datei");
    let url = file_url(&path).expect("URL");
    assert_eq!(read_file_within(&url, 8), Some(bytes.to_vec()));
    assert_eq!(read_file_within(&url, 7), None);
    assert_eq!(read_file_within(&url, 0), None);

    let empty = directory.join("empty.css");
    std::fs::write(&empty, []).expect("leere Datei");
    assert_eq!(read_file_within(&file_url(&empty).expect("URL"), 0), None);

    // Ein großes Restbudget darf die Grenze je Datei nicht aushebeln.
    // set_len vermeidet dafür einen großen Testpuffer.
    let oversized = directory.join("oversized.css");
    std::fs::File::create(&oversized)
        .expect("Datei")
        .set_len(MAX_FILE_BYTES + 1)
        .expect("Dateilänge");
    assert_eq!(
        read_file_within(&file_url(&oversized).expect("URL"), u64::MAX),
        None
    );
    std::fs::remove_dir_all(directory).expect("aufräumen");
}
