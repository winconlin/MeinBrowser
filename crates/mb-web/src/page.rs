//! Eine Seite laden und darstellen.
//!
//! - Was die Seite lädt, holt eine Funktion des Aufrufers ([`Fetch`]); für
//!   lokale Dateien gibt es [`read_file`]. Netzwerk, Fetch und MIME-Typen
//!   kommen mit M3. Bis dahin entscheidet die Endung: `.xht`, `.xhtml` und
//!   `.xml` liest der XML-Parser, alles andere der HTML-Parser.
//! - HTML-Bytes werden nach einer BOM dekodiert, sonst als UTF-8. Das ist
//!   eine Lücke: Es fehlen das Vorab-Lesen von `<meta charset>`
//!   (*prescan*) und die Ersatzkodierung nach Sprache und Region, die der
//!   HTML Standard vorsieht.
//! - Stylesheets kommen in Baumreihenfolge aus `<style>` und aus
//!   `<link rel=stylesheet>` (ohne `alternate`, `disabled` und andere Typen
//!   als `text/css`). Das Attribut `media` wird noch nicht ausgewertet,
//!   denn Media Queries fehlen; ebenso `<?xml-stylesheet?>` und `@import`.
//! - Bilder sind bisher nur PNG.
//! - Die Systemfarben sind die eines hellen Farbschemas, festgelegt von
//!   diesem Browser (CSS Color 4 überlässt sie dem Benutzeragenten), bis es
//!   Farbschemata und die Werte der Plattformen gibt.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use mb_base::CodePoint;
use mb_dom::{Document, NodeId, NodeKind, QuirksMode};
use mb_encoding::Encoding;
use mb_html::{Options, ScriptingMode, parse_document};
use mb_layout::{Fonts, LayoutInput};
use mb_scene::{Image, Scene};
use mb_style::values::ParseContext;
use mb_style::values::color::{Rgba, SystemColor};
use mb_style::{Device, DocumentInfo, Origin, StyleSheet, Stylist, StylistBuilder};
use mb_url::Url;

/// Holt die Bytes hinter einer URL; `None`, wenn das nicht geht.
pub type Fetch = Box<dyn Fn(&Url) -> Option<Vec<u8>> + Send + Sync>;

/// So viele Pixel darf ein Bild haben; größere werden nicht dekodiert.
const MAX_IMAGE_PIXELS: u64 = 1 << 26;

/// Liest eine `file:`-URL aus dem Dateisystem. Hosts in `file:`-URLs (UNC)
/// gibt es noch nicht.
pub fn read_file(url: &Url) -> Option<Vec<u8>> {
    if url.scheme() != "file" || !url.hostname().is_empty() {
        return None;
    }
    let path =
        String::from_utf8(mb_url::percent::percent_decode(url.pathname().as_bytes())).ok()?;
    // Unter Windows beginnt der Pfad mit dem Laufwerk: `/C:/…`.
    let path = if cfg!(windows) && path.get(2..3) == Some(":") && path.starts_with('/') {
        path.get(1..).unwrap_or_default().to_owned()
    } else {
        path
    };
    std::fs::read(path).ok()
}

/// Die `file:`-URL eines Pfads, relativ zum Arbeitsverzeichnis, wenn er
/// nicht absolut ist.
pub fn file_url(path: &Path) -> Option<Url> {
    let path = std::path::absolute(path).ok()?;
    let mut text = path.to_str()?.replace('\\', "/");
    if !text.starts_with('/') {
        text.insert(0, '/');
    }
    // Was der URL-Parser anders läse, wird vorher kodiert.
    let text = text
        .replace('%', "%25")
        .replace('#', "%23")
        .replace('?', "%3F");
    Url::parse(&format!("file://{text}"), None).ok()
}

/// Eine geladene Seite.
pub struct Page {
    document: Document,
    /// Die Basis-URL des Dokuments.
    base: Url,
    stylist: Stylist,
    fetch: Fetch,
    /// Die Bilder nach URL ohne Fragment, `None`, wo das Laden scheiterte.
    images: Mutex<BTreeMap<String, Option<Arc<Image>>>>,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Page")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

/// Ob ein Attribut fehlt, leer ist oder `text/css` lautet.
fn is_css_type(value: Option<String>) -> bool {
    value.is_none_or(|value| value.is_empty() || value.eq_ignore_ascii_case("text/css"))
}

/// Die Knoten in Baumreihenfolge.
fn tree_order(document: &Document) -> Vec<NodeId> {
    let mut order = Vec::new();
    let mut stack = vec![document.root()];
    while let Some(node) = stack.pop() {
        order.push(node);
        let children: Vec<NodeId> = document.children(node).collect();
        stack.extend(children.into_iter().rev());
    }
    order
}

/// *child text content*
fn child_text(document: &Document, node: NodeId) -> String {
    document
        .children(node)
        .filter(|&child| {
            matches!(
                document.kind(child),
                Some(NodeKind::Text | NodeKind::CdataSection)
            )
        })
        .filter_map(|child| document.data(child))
        .map(|data| data.to_string_lossy())
        .collect()
}

impl Page {
    /// Lädt das Dokument hinter `url` mit seinen Stylesheets; `None`, wenn
    /// es sich nicht holen lässt.
    pub fn load(url: Url, fetch: Fetch) -> Option<Page> {
        let bytes = fetch(&url)?;
        let pathname = url.pathname().to_ascii_lowercase();
        let xml = [".xht", ".xhtml", ".xml"]
            .iter()
            .any(|extension| pathname.ends_with(extension));
        let (document, encoding) = if xml {
            let parsed = mb_xml::parse(&bytes, &mb_xml::Options::default());
            (parsed.document, parsed.encoding.unwrap_or(Encoding::Utf8))
        } else {
            let (text, encoding) = mb_encoding::decode(&bytes, Encoding::Utf8);
            let mut document = Document::new();
            parse_document(
                &mut document,
                text.chars().map(CodePoint::from),
                Options {
                    scripting: ScriptingMode::Disabled,
                    collect_errors: false,
                },
            );
            (document, encoding)
        };
        let mut document = document;
        document.perform_microtask_checkpoint();
        let base = document_base(&document, &url);
        let quirks = document.quirks_mode() == QuirksMode::Quirks;
        let mut builder = StylistBuilder::new();
        builder.add_user_agent_sheets(DocumentInfo {
            quirks_mode: document.quirks_mode(),
            html_document: document.is_html_document(),
            iso_8859_8: encoding == Encoding::Iso8859_8,
        });
        for node in tree_order(&document) {
            let Some(element) = document.element(node) else {
                continue;
            };
            let attribute =
                |name: &str| element.attribute(name).map(|value| value.to_string_lossy());
            if element.is_html("style") {
                if !is_css_type(attribute("type")) {
                    continue;
                }
                let context = ParseContext {
                    base_url: Some(&base),
                    quirks_mode: quirks,
                };
                builder.add(
                    Origin::Author,
                    &StyleSheet::parse(&child_text(&document, node), context),
                );
            } else if element.is_html("link") {
                let rel = attribute("rel").unwrap_or_default().to_ascii_lowercase();
                let tokens: Vec<&str> = rel.split_ascii_whitespace().collect();
                if !tokens.contains(&"stylesheet")
                    || tokens.contains(&"alternate")
                    || attribute("disabled").is_some()
                    || !is_css_type(attribute("type"))
                {
                    continue;
                }
                let Some(href) = attribute("href").filter(|href| !href.is_empty()) else {
                    continue;
                };
                let Ok(sheet_url) = Url::parse(&href, Some(&base)) else {
                    continue;
                };
                let Some(bytes) = fetch(&sheet_url) else {
                    continue;
                };
                let (text, _) = mb_css::decode_stylesheet(&bytes, None, Some(encoding));
                let context = ParseContext {
                    base_url: Some(&sheet_url),
                    quirks_mode: quirks,
                };
                builder.add(Origin::Author, &StyleSheet::parse(&text, context));
            }
        }
        Some(Page {
            document,
            base,
            stylist: builder.build(),
            fetch,
            images: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Ein Bild, beim ersten Mal geladen.
    fn image(&self, url: &Url) -> Option<Arc<Image>> {
        let key = url.serialize(true);
        let mut images = self.images.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(image) = images.get(&key) {
            return image.clone();
        }
        let image = (self.fetch)(url).and_then(|bytes| decode_image(&bytes));
        images.insert(key, image.clone());
        image
    }

    /// Layout und Zeichnen in einem Viewport von `width` × `height` CSS-
    /// Pixeln, ein Gerätepixel je CSS-Pixel.
    pub fn render(&self, fonts: &Fonts<'_>, width: f64, height: f64) -> Scene {
        let device = Device {
            viewport_width: width,
            viewport_height: height,
            device_pixel_ratio: 1.0,
            system_color,
        };
        let images = |url: &Url| self.image(url);
        let tree = mb_layout::layout(&LayoutInput {
            document: &self.document,
            stylist: &self.stylist,
            device: &device,
            fonts,
            base: &self.base,
            images: &images,
        });
        tree.paint(&images)
    }
}

/// Die Basis-URL: `href` des ersten `<base>` mit `href`, sonst die URL des
/// Dokuments (HTML, *document base URL*).
fn document_base(document: &Document, url: &Url) -> Url {
    tree_order(document)
        .into_iter()
        .filter_map(|node| document.element(node))
        .find(|element| element.is_html("base") && element.attribute("href").is_some())
        .and_then(|element| element.attribute("href"))
        .and_then(|href| Url::parse(&href.to_string_lossy(), Some(url)).ok())
        .unwrap_or_else(|| url.clone())
}

fn decode_image(bytes: &[u8]) -> Option<Arc<Image>> {
    let png = mb_image::png::decode(bytes, MAX_IMAGE_PIXELS).ok()?;
    Image::new(png.width(), png.height(), png.to_rgba8()).map(Arc::new)
}

/// Die Systemfarben eines hellen Farbschemas. Links, besuchte und aktive
/// Links haben die Farben, die der HTML Standard früher fest vorgab.
fn system_color(color: SystemColor) -> Rgba {
    let rgb = |red, green, blue| Rgba {
        red,
        green,
        blue,
        alpha: 255,
    };
    match color {
        SystemColor::Canvas
        | SystemColor::Field
        | SystemColor::HighlightText
        | SystemColor::SelectedItemText
        | SystemColor::AccentColorText => rgb(0xFF, 0xFF, 0xFF),
        SystemColor::ButtonFace => rgb(0xF0, 0xF0, 0xF0),
        SystemColor::ButtonBorder | SystemColor::GrayText => rgb(0x80, 0x80, 0x80),
        SystemColor::LinkText => rgb(0x00, 0x00, 0xEE),
        SystemColor::VisitedText => rgb(0x55, 0x1A, 0x8B),
        SystemColor::ActiveText => rgb(0xFF, 0x00, 0x00),
        SystemColor::Mark => rgb(0xFF, 0xFF, 0x00),
        SystemColor::AccentColor | SystemColor::Highlight | SystemColor::SelectedItem => {
            rgb(0x00, 0x66, 0xCC)
        }
        _ => rgb(0x00, 0x00, 0x00),
    }
}
