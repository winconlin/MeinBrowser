//! Bounded entry points for the separate, development-only fuzz workspace.
//! Invalid inputs are normal; panics and invariant failures must reach libFuzzer.

#![forbid(unsafe_code)]

use std::hint::black_box;

pub const MAX_INPUT: usize = 4096;
pub const MAX_FONT_INPUT: usize = 65536;
const MAX_PIXELS: u64 = 4096;
const MAX_INFLATED: usize = 65536;
/// Everything one page may load and decode, including image pixels.
const MAX_PAGE_BYTES: u64 = 1 << 20;
/// Style and layout work per viewport; see `mb_web::RENDER_STEPS`. Measured on
/// 30.10.2026 with AddressSanitizer: the slowest of several 4 KiB inputs, a table
/// with 400 rows, took 2 s for both viewports.
const MAX_RENDER_STEPS: u64 = 1_000_000;
/// Separates the parts of a `page` input: the document, then its resources.
const PART_SEPARATOR: [u8; 2] = [0x1f, 0x1f];
/// The origin of a `page` input's document and resources.
const ORIGIN: &str = "https://example.test/";
/// The only font of the `page` target, so text layout reaches real glyphs.
const FONT: &[u8] = include_bytes!("../seeds/font/synthetic-sfnt");

pub fn html(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let mut parsed = mb_html::parse_bytes(
        data,
        mb_encoding::Encoding::Utf8,
        mb_html::Options {
            scripting: mb_html::ScriptingMode::Disabled,
            collect_errors: false,
        },
    );
    parsed.document.perform_microtask_checkpoint();
    black_box(parsed);
}

pub fn css(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let (text, _) = mb_css::decode_stylesheet(data, None, None);
    let source = mb_css::Source::new(&text);
    black_box(mb_css::parse_stylesheet(&source, &mb_css::Syntax));
    black_box(mb_css::parse_declaration_list(&source, &mb_css::Syntax));
    let values = mb_css::parse_component_value_list(&source);
    black_box(mb_css::parse_an_plus_b(&values.iter().collect::<Vec<_>>()));
    let _ = black_box(mb_style::SelectorList::parse(
        &source,
        &mb_style::Namespaces::default(),
    ));
    black_box(mb_style::StyleSheet::parse(
        &text,
        mb_style::values::ParseContext::default(),
    ));
}

pub fn url(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let text = String::from_utf8_lossy(data);
    let base = mb_url::Url::parse("https://example.test/a/b", None).expect("fixed base URL");
    for base in [None, Some(&base)] {
        if let Ok(url) = mb_url::Url::parse(&text, base) {
            let serialized = url.href();
            let reparsed = mb_url::Url::parse(&serialized, None).expect("serialized URL");
            assert_eq!(serialized, reparsed.href());
        }
    }
    let pairs = mb_url::form_urlencoded::parse(data);
    let serialized = mb_url::form_urlencoded::serialize(
        pairs
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        mb_encoding::Encoding::Utf8,
    );
    assert_eq!(pairs, mb_url::form_urlencoded::parse(serialized.as_bytes()));
}

pub fn xml(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    black_box(mb_xml::parse(data, &mb_xml::Options::default()));
}

pub fn png(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    if let Ok(image) = mb_image::png::decode(data, MAX_PIXELS) {
        let pixels = u64::from(image.width()) * u64::from(image.height());
        assert!(pixels <= MAX_PIXELS);
        assert_eq!(image.to_rgba8().len() as u64, pixels * 4);
    }
}

pub fn gif(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    if let Ok(image) = mb_image::gif::decode(data, MAX_PIXELS) {
        let pixels = u64::from(image.width()) * u64::from(image.height());
        assert!(pixels <= MAX_PIXELS);
        assert_eq!(image.pixels().len() as u64, pixels * 4);
    }
}

pub fn deflate(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    if let Ok(result) = mb_compress::inflate(data, MAX_INFLATED) {
        assert!(result.data.len() <= MAX_INFLATED);
        assert!(result.consumed <= data.len());
    }
    if let Ok(result) = mb_compress::zlib_decompress(data, MAX_INFLATED) {
        assert!(result.len() <= MAX_INFLATED);
    }
}

pub fn font(data: &[u8]) {
    if data.len() > MAX_FONT_INPUT {
        return;
    }
    let Ok(count) = mb_text::face_count(data) else {
        return;
    };
    let data: mb_text::FontData = data.into();
    for index in 0..count.min(2) {
        let Ok(face) = mb_text::Face::parse(data.clone(), index) else {
            continue;
        };
        let selected = u16::from_be_bytes([
            data.first().copied().unwrap_or(0),
            data.last().copied().unwrap_or(0),
        ]);
        for glyph in [0, selected, face.glyph_count().saturating_sub(1)] {
            let glyph = mb_text::GlyphId(glyph);
            black_box(face.advance_width(glyph));
            black_box(face.left_side_bearing(glyph));
            let _ = black_box(face.outline(glyph));
        }
        black_box(face.glyph_index('A'));
        black_box(face.metrics());
    }
}

pub fn json(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let text = String::from_utf8_lossy(data);
    if let Err(error) = mb_json::parse(&text) {
        assert!(error.position <= text.len());
    }
}

pub fn encoding(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let Some((&selector, bytes)) = data.split_first() else {
        return;
    };
    let encoding =
        mb_encoding::Encoding::ALL[usize::from(selector) % mb_encoding::Encoding::ALL.len()];
    let split = bytes.len() / 2;
    let mut whole = String::new();
    encoding.new_decoder().decode(bytes, true, &mut whole);
    let mut chunked = String::new();
    let mut decoder = encoding.new_decoder();
    decoder.decode(&bytes[..split], false, &mut chunked);
    decoder.decode(&bytes[split..], true, &mut chunked);
    assert_eq!(whole, chunked);
    black_box(mb_encoding::decode(bytes, encoding));
    black_box(mb_encoding::encode(&whole, encoding));
}

pub fn unicode(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let text = String::from_utf8_lossy(data);
    let nfc = mb_unicode::normalization::nfc(&text);
    let nfd = mb_unicode::normalization::nfd(&text);
    assert_eq!(nfc, mb_unicode::normalization::nfc(&nfc));
    assert_eq!(nfd, mb_unicode::normalization::nfd(&nfd));
    assert_eq!(nfd, mb_unicode::normalization::nfd(&nfc));
    assert!(mb_unicode::normalization::is_nfc(&nfc));
    black_box(mb_unicode::case_folding::fold(&text));
    let points: Vec<u32> = text.chars().map(u32::from).collect();
    let breaks = mb_unicode::line_breaking::line_breaks(&points);
    assert!(breaks.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(
        breaks
            .iter()
            .all(|&(index, _)| index > 0 && index <= points.len())
    );
    assert_eq!(
        breaks.last().map(|entry| entry.0),
        (!points.is_empty()).then_some(points.len())
    );
    let options = mb_unicode::idna::Options {
        check_hyphens: true,
        check_bidi: true,
        check_joiners: true,
        use_std3_ascii_rules: true,
        transitional_processing: false,
        verify_dns_length: true,
    };
    let _ = black_box(mb_unicode::idna::to_ascii(&text, options));
    black_box(mb_unicode::idna::to_unicode(&text, options));
}

/// Splits a `page` input at each [`PART_SEPARATOR`].
fn parts(data: &[u8]) -> Vec<Vec<u8>> {
    let mut parts = Vec::new();
    let mut rest = data;
    while let Some(position) = rest
        .windows(PART_SEPARATOR.len())
        .position(|window| window == PART_SEPARATOR)
    {
        parts.push(rest.get(..position).unwrap_or_default().to_vec());
        rest = rest
            .get(position.saturating_add(PART_SEPARATOR.len())..)
            .unwrap_or_default();
    }
    parts.push(rest.to_vec());
    parts
}

/// Part `N` for each URL below [`ORIGIN`] whose path starts with the number `N`.
fn resource(parts: &[Vec<u8>], url: &mb_url::Url) -> Option<Vec<u8>> {
    let href = url.href();
    let path = href.strip_prefix(ORIGIN)?;
    let digits = path.bytes().take_while(u8::is_ascii_digit).count();
    let index: usize = path.get(..digits)?.parse().ok()?;
    parts.get(index).cloned()
}

/// Loads part 0 as `0.xhtml` if it starts with `<?xml`, otherwise as `0.html`.
fn load_page(data: &[u8]) -> mb_web::Page {
    let parts = parts(data);
    let name = if data.starts_with(b"<?xml") {
        "0.xhtml"
    } else {
        "0.html"
    };
    let url = mb_url::Url::parse(&format!("{ORIGIN}{name}"), None).expect("fixed document URL");
    mb_web::Page::load_within(
        url,
        Box::new(move |url: &mb_url::Url| resource(&parts, url)),
        MAX_PAGE_BYTES,
    )
    .expect("document within the byte budget")
}

fn font_library() -> mb_text::FontLibrary {
    let mut library = mb_text::FontLibrary::new(Box::new(|_| Some(FONT.to_vec())));
    library.add_file(std::path::PathBuf::from("synthetic-sfnt"));
    library
}

/// Loading, cascade, layout and painting of a page with its stylesheets and
/// images, in two viewports.
pub fn page(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let page = load_page(data);
    let library = font_library();
    let fonts = mb_layout::Fonts::new(&library);
    let budget = mb_base::Budget::new(MAX_RENDER_STEPS);
    match page.render_within(&fonts, 800.0, 600.0, &budget) {
        Ok(scene) => {
            black_box(scene);
        }
        Err(error) => assert_eq!(error, mb_base::Exhausted::Steps),
    }
    let viewport = mb_web::Viewport {
        width: 320.0,
        height: 480.0,
        device_pixel_ratio: 2.0,
    };
    let budget = mb_base::Budget::new(MAX_RENDER_STEPS);
    match page.layout_within(&fonts, viewport, &budget) {
        Ok(tree) => {
            let (width, height) = tree.scroll_size();
            assert!(width >= viewport.width && height >= viewport.height);
            let scroll = (width - viewport.width, height - viewport.height);
            black_box(page.paint(&tree, scroll));
        }
        Err(error) => assert_eq!(error, mb_base::Exhausted::Steps),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_seeds_reach_decoding_paths() {
        let png = mb_image::png::decode(include_bytes!("../seeds/png/red-pixel"), MAX_PIXELS)
            .expect("valid first-party PNG seed");
        assert_eq!(png.to_rgba8(), [255, 0, 0, 255]);
        let gif = mb_image::gif::decode(include_bytes!("../seeds/gif/black-pixel"), MAX_PIXELS)
            .expect("valid first-party GIF seed");
        assert_eq!(gif.pixels(), [0, 0, 0, 255]);
        let raw = mb_compress::inflate(include_bytes!("../seeds/deflate/raw"), MAX_INFLATED)
            .expect("valid first-party Deflate seed");
        assert_eq!(raw.data, b"abc".repeat(20));
        let data: mb_text::FontData = include_bytes!("../seeds/font/synthetic-sfnt")
            .as_slice()
            .into();
        let face = mb_text::Face::parse(data, 0).expect("synthetic font accepted by reader");
        let outline = face.outline(mb_text::GlyphId(0)).expect("seed glyph");
        assert!(!outline.is_empty());
    }

    #[test]
    fn page_seeds_reach_resources_and_glyphs() {
        assert_eq!(font_library().face_count(), 1);
        let face = mb_text::Face::parse(FONT.into(), 0).expect("synthetic font");
        for character in [' ', 'A', 'T', 'x', '~'] {
            assert_eq!(face.glyph_index(character), Some(mb_text::GlyphId(1)));
        }
        assert_eq!(face.glyph_index('\u{e9}'), None);
        let data = include_bytes!("../seeds/page/resources");
        assert_eq!(parts(data).len(), 4);
        let page = load_page(data);
        let library = font_library();
        let fonts = mb_layout::Fonts::new(&library);
        let budget = mb_base::Budget::new(MAX_RENDER_STEPS);
        let scene = page
            .render_within(&fonts, 800.0, 600.0, &budget)
            .expect("seed within the step budget");
        // 1.css imports 3.css, which colors the paragraph.
        let green = mb_scene::Color::rgb(0, 128, 0);
        assert!(scene.items.iter().any(|item| matches!(
            item,
            mb_scene::Item::Rect { color, .. } if *color == green
        )));
        assert!(
            scene
                .items
                .iter()
                .any(|item| matches!(item, mb_scene::Item::Image { .. }))
        );
        // Text glyphs from the font, not only the missing glyph 0.
        assert!(scene.items.iter().any(|item| matches!(
            item,
            mb_scene::Item::Glyphs(run)
                if run.glyphs.iter().any(|glyph| glyph.id == mb_text::GlyphId(1))
        )));
    }

    #[test]
    fn start_corpora_and_empty_inputs_exercise_all_harnesses() {
        type Harness = fn(&[u8]);
        let targets: [(&str, Harness); 12] = [
            ("html", html),
            ("css", css),
            ("url", url),
            ("xml", xml),
            ("png", png),
            ("gif", gif),
            ("deflate", deflate),
            ("font", font),
            ("json", json),
            ("encoding", encoding),
            ("unicode", unicode),
            ("page", page),
        ];
        for (name, check) in targets {
            check(&[]);
            let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("seeds")
                .join(name);
            let seeds = std::fs::read_dir(directory).expect("tracked start corpus");
            let mut count = 0;
            for seed in seeds {
                let bytes = std::fs::read(seed.expect("seed entry").path()).expect("seed bytes");
                check(&bytes);
                count += 1;
            }
            assert!(count > 0, "empty start corpus for {name}");
        }
    }
}
