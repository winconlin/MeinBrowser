//! Bilder durch die wirkliche Seitenpipeline, mit eigenen kleinen GIFs.

use std::collections::BTreeMap;
use std::sync::Arc;

use mb_layout::Fonts;
use mb_scene::{Color, Image, Item, Rect};
use mb_text::FontLibrary;
use mb_url::Url;

use crate::Page;

/// Ein schwarzes Pixel, LZW: Clear 4, Literal 0, Ende 5, je drei Bits.
const BLACK: &[u8] =
    b"GIF87a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff,\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0;";

/// Eine 3 × 2 Leinwand mit zwei Pixeln an (1, 1): rot und transparent.
/// Der transparente Index ist 1. LZW: Clear 4, 0, 1, Ende 5.
const TRANSPARENT_OFFSET: &[u8] = b"GIF89a\x03\0\x02\0\x80\0\0\xff\0\0\0\xff\0\
      \x21\xf9\x04\x01\0\0\x01\0\
      ,\x01\0\x01\0\x02\0\x01\0\0\x02\x02\x44\x0a\0;";

fn page(html: &[u8], files: &[(&str, &[u8])], budget: u64) -> Page {
    let mut files: BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(name, bytes)| (format!("file:///images/{name}"), bytes.to_vec()))
        .collect();
    files.insert("file:///images/a.html".to_owned(), html.to_vec());
    Page::load_within(
        Url::parse("file:///images/a.html", None).expect("URL"),
        Box::new(move |url: &Url, limit| {
            files
                .get(&url.serialize(true))
                .filter(|bytes| u64::try_from(bytes.len()).expect("Länge") <= limit)
                .cloned()
        }),
        budget,
    )
    .expect("Seite")
}

fn images(page: &Page) -> Vec<(Rect, Arc<Image>, bool, bool)> {
    let library = FontLibrary::new(Box::new(|_| None));
    let fonts = Fonts::new(&library);
    page.render(&fonts, 80.0, 60.0)
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Image {
                rect,
                image,
                repeat_x,
                repeat_y,
                ..
            } => Some((rect, image, repeat_x, repeat_y)),
            _ => None,
        })
        .collect()
}

#[test]
fn gif_signatures_decide_the_format_instead_of_the_extension() {
    let html = b"<!DOCTYPE html><style>body { margin: 0 } img { display: block }</style>\
                 <img src=first.png><img src=second.dat>";
    let mut newer = BLACK.to_vec();
    newer[..6].copy_from_slice(b"GIF89a");
    let page = page(html, &[("first.png", BLACK), ("second.dat", &newer)], 4096);
    let images = images(&page);
    assert_eq!(images.len(), 2);
    for (index, (rect, image, repeat_x, repeat_y)) in images.iter().enumerate() {
        assert_eq!(*rect, Rect::new(0.0, index as f32, 1.0, 1.0));
        assert_eq!((image.width(), image.height()), (1, 1));
        assert_eq!(image.pixel(0, 0), Some(Color::rgb(0, 0, 0)));
        assert!(!repeat_x && !repeat_y);
    }
}

#[test]
fn gif_canvas_offsets_and_transparency_survive_the_page_pipeline() {
    let html = b"<!DOCTYPE html><style>body { margin: 0 } img { display: block }</style>\
                 <img src=offset.gif>";
    let page = page(html, &[("offset.gif", TRANSPARENT_OFFSET)], 4096);
    let images = images(&page);
    assert_eq!(images.len(), 1);
    let (rect, image, _, _) = &images[0];
    assert_eq!(*rect, Rect::new(0.0, 0.0, 3.0, 2.0));
    assert_eq!((image.width(), image.height()), (3, 2));
    for y in 0..2 {
        for x in 0..3 {
            let expected = if (x, y) == (1, 1) {
                Color::rgb(255, 0, 0)
            } else {
                Color::TRANSPARENT
            };
            assert_eq!(image.pixel(x, y), Some(expected), "Pixel ({x}, {y})");
        }
    }
}

#[test]
fn gif_backgrounds_use_the_same_decoder_and_repeat() {
    let html = b"<!DOCTYPE html><style>body { margin: 0 }\
                 div { width: 8px; height: 4px; background: url(tile.gif) repeat }\
                 </style><div></div>";
    let page = page(html, &[("tile.gif", BLACK)], 4096);
    let images = images(&page);
    assert_eq!(images.len(), 1);
    let (rect, image, repeat_x, repeat_y) = &images[0];
    assert_eq!(*rect, Rect::new(0.0, 0.0, 1.0, 1.0));
    assert_eq!(image.pixel(0, 0), Some(Color::rgb(0, 0, 0)));
    assert!(*repeat_x && *repeat_y);
}

#[test]
fn truncated_or_invalid_gifs_do_not_produce_scene_images() {
    let html = b"<!DOCTYPE html><img src=broken.gif><img src=signature.gif><img src=good.gif>";
    // Auch der erste Bildblock ist unvollständig, nicht nur der optionale
    // Trailer nach dem vom Dekoder ausgewerteten Einzelbild.
    let truncated = &BLACK[..BLACK.len() - 2];
    let page = page(
        html,
        &[
            ("broken.gif", truncated),
            ("signature.gif", b"GIF89a"),
            ("good.gif", BLACK),
        ],
        4096,
    );
    let images = images(&page);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].1.pixel(0, 0), Some(Color::rgb(0, 0, 0)));
}

#[test]
fn the_whole_gif_canvas_must_fit_the_remaining_pixel_budget() {
    let html = b"<!DOCTYPE html><img src=offset.gif>";
    // Sechs Leinwandpixel, obwohl der Bildausschnitt nur zwei Pixel hat.
    // GIF schreibt direkt nach RGBA8; es braucht keinen zweiten Bildpuffer.
    let encoded = u64::try_from(html.len() + TRANSPARENT_OFFSET.len()).expect("Länge");
    let files = [("offset.gif", TRANSPARENT_OFFSET)];
    assert!(images(&page(html, &files, encoded + 23)).is_empty());
    let images = images(&page(html, &files, encoded + 24));
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].1.width(), images[0].1.height()), (3, 2));
}
