//! Eine Seite laden und darstellen.
//!
//! - Was die Seite lädt, holt eine Funktion des Aufrufers ([`Fetch`]); für
//!   lokale Dateien gibt es [`crate::read_file`]. Netzwerk, Fetch und
//!   MIME-Typen kommen mit M3. Bis dahin entscheidet die Endung: `.xht`,
//!   `.xhtml` und `.xml` liest der XML-Parser, alles andere der HTML-Parser.
//! - Alles, was die Seite lädt und dekodiert, bezahlt sie aus
//!   [`PAGE_BYTES`]: Dokument, Stylesheets, Bilddateien und die Pixel der
//!   Bilder. Ein Bild wird nur dekodiert, wenn das Budget auch für die
//!   Zwischenpuffer des Dekoders reicht; was nicht mehr passt, fehlt.
//! - Die Kodierung von HTML-Bytes bestimmt [`mb_html::parse_bytes`] aus
//!   der Byte-Order-Mark, dem Vorab-Lesen (*prescan*) und einem `meta`
//!   beim Parsen. Ohne Angabe gilt UTF-8; die Ersatzkodierung nach Sprache
//!   und Region, die der HTML Standard vorschlägt, fehlt noch.
//! - Stylesheets kommen in Baumreihenfolge aus `<style>` und aus
//!   `<link rel=stylesheet>` (ohne `alternate`, `disabled` und andere Typen
//!   als `text/css`), jeweils mit dem Attribut `media`, und aus `@import`
//!   darin. Ein `@import` wird nicht geladen, wenn es zu einem Stylesheet
//!   zurückführt, das es schon einbindet, tiefer als
//!   [`mb_style::MAX_IMPORT_DEPTH`] steht oder mehr als [`MAX_IMPORTS`]
//!   Stylesheets aus `@import` lädt. Es fehlt `<?xml-stylesheet?>`.
//! - Bilder sind bisher nur PNG.
//! - Kaskaden-Vorbereitung, Stil und Layout fremder Seiten bezahlt [`Page::render_within`] aus
//!   einem Budget ([`RENDER_STEPS`]); ist es verbraucht, gibt es kein Bild,
//!   sondern einen Fehler.
//! - Zum Scrollen wird einmal gesetzt ([`Page::layout_within`]) und an jeder
//!   Scrollposition neu gezeichnet ([`Page::paint`]). Der Viewport kennt die
//!   Gerätepixel je CSS-Pixel ([`Viewport`]) für Media Queries und für die
//!   Breite von Rahmen, die auf ganze Gerätepixel fällt. Die Regeln für einen
//!   Viewport werden neu zusammengestellt, wenn eine Media Query dort anders
//!   ausfällt als für den vorigen.
//! - Die Systemfarben sind die eines hellen Farbschemas, festgelegt von
//!   diesem Browser (CSS Color 4 überlässt sie dem Benutzeragenten), bis es
//!   Farbschemata und die Werte der Plattformen gibt.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use mb_base::{Budget, Exhausted};
use mb_dom::{Document, NodeId, NodeKind, QuirksMode};
use mb_encoding::Encoding;
use mb_html::{Options, ScriptingMode, parse_bytes};
use mb_layout::{Fonts, FragmentTree, LayoutInput};
use mb_scene::{Image, Scene};
use mb_style::values::ParseContext;
use mb_style::values::color::{Rgba, SystemColor};
use mb_style::{
    Device, DocumentInfo, MAX_IMPORT_DEPTH, MediaQueryList, Origin, StyleSheet, Stylist,
    StylistBuilder,
};
use mb_url::Url;

/// Ein Viewport: seine Größe in CSS-Pixeln und wie viele Gerätepixel auf
/// einen CSS-Pixel kommen (`devicePixelRatio`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
    pub device_pixel_ratio: f64,
}

/// Holt die Bytes hinter einer URL; `None`, wenn das nicht geht.
pub type Fetch = Box<dyn Fn(&Url) -> Option<Vec<u8>> + Send + Sync>;

/// So viele Pixel darf ein Bild haben; größere werden nicht dekodiert.
const MAX_IMAGE_PIXELS: u64 = 1 << 26;

/// So viele Stylesheets darf eine Seite mit `@import` laden. Festgelegt von
/// diesem Browser: Ohne Grenze könnte ein Stylesheet, das sich zweimal unter
/// verschiedenen URLs einbindet, bis zur Tiefe
/// [`mb_style::MAX_IMPORT_DEPTH`] exponentiell viele Abrufe auslösen.
pub const MAX_IMPORTS: usize = 1024;

/// So viele Bytes darf eine Seite laden und dekodieren, zusammen: das
/// Dokument, die Stylesheets, die Bilddateien und die Pixel der Bilder mit
/// vier Bytes je Pixel. Festgelegt von diesem Browser: genug für Seiten mit
/// vielen Bildern, aber auch für Geräte mit wenig Speicher.
pub const PAGE_BYTES: u64 = 256 << 20;

/// So viele Arbeitsschritte dürfen Kaskaden-Vorbereitung, Stil und Layout
/// einer Seite zusammen kosten (siehe `mb_base::budget`). Festgelegt von
/// diesem Browser. Auch Cacheprüfung, Media Queries und Selektorindexierung
/// zählen mit. Schritte sind ungefähre Arbeitseinheiten, keine feste
/// Zeitgrenze; Abbruch wird kooperativ an den budgetierten Stellen geprüft.
pub const RENDER_STEPS: u64 = 100_000_000;

/// Die `file:`-URL eines Pfads, relativ zum Arbeitsverzeichnis, wenn er
/// nicht absolut ist.
pub fn file_url(path: &Path) -> Option<Url> {
    let path = std::path::absolute(path).ok()?;
    let text = path.to_str()?;
    // Unter Windows trennt `\` die Teile des Pfads, sonst gehört es zum Namen.
    let mut text = if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.to_owned()
    };
    if !text.starts_with('/') {
        text.insert(0, '/');
    }
    // Was der URL-Parser anders läse oder entfernte, wird vorher kodiert:
    // `%` und `\` selbst, dann die Menge für Pfade, mit `#`, `?` und den
    // Steuerzeichen wie Tabulator und Zeilenumbruch.
    let text = text.replace('%', "%25").replace('\\', "%5C");
    let text = mb_url::percent::utf8_percent_encode(&text, mb_url::percent::PATH);
    Url::parse(&format!("file://{text}"), None).ok()
}

/// Eine geladene Seite.
pub struct Page {
    document: Document,
    /// Die Basis-URL des Dokuments.
    base: Url,
    /// Was die Stylesheets der Benutzeragenten auswählt.
    info: DocumentInfo,
    /// Die Stylesheets der Seite in Baumreihenfolge, mit den Stylesheets
    /// aus `@import`.
    sheets: Vec<StyleSheet>,
    /// Die Regeln für den zuletzt verwendeten Viewport.
    stylist: Mutex<Option<Arc<Stylist>>>,
    fetch: Fetch,
    /// Die Bilder nach URL ohne Fragment, `None`, wo das Laden scheiterte.
    images: Mutex<BTreeMap<String, Option<Arc<Image>>>>,
    /// Was von [`PAGE_BYTES`] übrig ist.
    bytes: Bytes,
}

/// Ein Budget an Bytes, das mehrere Fäden teilen können.
struct Bytes(AtomicU64);

impl Bytes {
    /// Bezahlt `count` Bytes; `false`, wenn nicht mehr so viele übrig sind.
    fn spend(&self, count: u64) -> bool {
        self.0
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(count)
            })
            .is_ok()
    }

    fn left(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Die Länge eines Puffers als Zahl für das Budget.
fn length(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
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
        Page::load_within(url, fetch, PAGE_BYTES)
    }

    /// Wie [`Page::load`], aber mit `bytes` statt [`PAGE_BYTES`] für alles,
    /// was die Seite lädt und dekodiert.
    pub fn load_within(url: Url, fetch: Fetch, bytes: u64) -> Option<Page> {
        let budget = Bytes(AtomicU64::new(bytes));
        let bytes = fetch(&url).filter(|bytes| budget.spend(length(bytes)))?;
        let pathname = url.pathname().to_ascii_lowercase();
        let xml = [".xht", ".xhtml", ".xml"]
            .iter()
            .any(|extension| pathname.ends_with(extension));
        let (document, encoding) = if xml {
            let parsed = mb_xml::parse(&bytes, &mb_xml::Options::default());
            (parsed.document, parsed.encoding.unwrap_or(Encoding::Utf8))
        } else {
            let options = Options {
                scripting: ScriptingMode::Disabled,
                collect_errors: false,
            };
            let parsed = parse_bytes(&bytes, Encoding::Utf8, options);
            (parsed.document, parsed.encoding)
        };
        let mut document = document;
        document.perform_microtask_checkpoint();
        let base = document_base(&document, &url);
        let quirks = document.quirks_mode() == QuirksMode::Quirks;
        let info = DocumentInfo {
            quirks_mode: document.quirks_mode(),
            html_document: document.is_html_document(),
            iso_8859_8: encoding == Encoding::Iso8859_8,
        };
        let mut imports = Imports {
            fetch: &fetch,
            budget: &budget,
            quirks,
            left: MAX_IMPORTS,
        };
        let mut sheets = Vec::new();
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
                let mut sheet = StyleSheet::parse(&child_text(&document, node), context);
                imports.load(&mut sheet, encoding, &mut Vec::new(), 0);
                if let Some(media) = attribute("media") {
                    sheet.set_media(MediaQueryList::parse(&media));
                }
                sheets.push(sheet);
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
                let Some(bytes) = fetch(&sheet_url).filter(|bytes| budget.spend(length(bytes)))
                else {
                    continue;
                };
                let (text, sheet_encoding) =
                    mb_css::decode_stylesheet(&bytes, None, Some(encoding));
                let context = ParseContext {
                    base_url: Some(&sheet_url),
                    quirks_mode: quirks,
                };
                let mut sheet = StyleSheet::parse(&text, context);
                let mut chain = vec![sheet_url.serialize(true)];
                imports.load(&mut sheet, sheet_encoding, &mut chain, 0);
                if let Some(media) = attribute("media") {
                    sheet.set_media(MediaQueryList::parse(&media));
                }
                sheets.push(sheet);
            }
        }
        Some(Page {
            document,
            base,
            info,
            sheets,
            stylist: Mutex::new(None),
            fetch,
            images: Mutex::new(BTreeMap::new()),
            bytes: budget,
        })
    }

    /// Die Regeln für ein Gerät: die vorigen, wenn jede Media Query dort
    /// gleich ausfällt, sonst neu zusammengestellt.
    fn stylist(
        &self,
        device: &Device,
        fonts: &Fonts<'_>,
        budget: &Budget,
    ) -> Result<Arc<Stylist>, Exhausted> {
        budget.spend(1);
        budget.check()?;
        // Nur den Arc unter dem Lock kopieren. Prüfen und Bauen dürfen
        // weder andere Viewports blockieren noch einen Teilstand publizieren.
        let cached = self
            .stylist
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(stylist) = cached
            && stylist.is_valid_for_within(device, fonts, budget)?
        {
            return Ok(stylist);
        }
        let mut builder = StylistBuilder::for_device(device, fonts);
        builder.add_user_agent_sheets_within(self.info, budget)?;
        for sheet in &self.sheets {
            builder.add_within(Origin::Author, sheet, budget)?;
        }
        let stylist = Arc::new(builder.build_within(budget)?);
        budget.spend(0);
        budget.check()?;
        let previous = self
            .stylist
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(Arc::clone(&stylist));
        drop(previous);
        Ok(stylist)
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
        let image = (self.fetch)(url)
            .filter(|bytes| self.bytes.spend(length(bytes)))
            .and_then(|bytes| {
                // Nur so viele Pixel, wie das Budget mit den Puffern des
                // Dekoders und seiner Kopie der Bilddaten erlaubt; bezahlt
                // werden danach die Pixel des Ergebnisses.
                let left = self.bytes.left().saturating_sub(length(&bytes));
                let pixels = MAX_IMAGE_PIXELS.min(left / mb_image::png::DECODE_BYTES_PER_PIXEL);
                decode_image(&bytes, pixels)
            })
            .filter(|image| {
                let pixels = u64::from(image.width()).saturating_mul(u64::from(image.height()));
                self.bytes.spend(pixels.saturating_mul(4))
            });
        images.insert(key, image.clone());
        image
    }

    /// Layout und Zeichnen in einem Viewport von `width` × `height` CSS-
    /// Pixeln, ein Gerätepixel je CSS-Pixel, ohne Grenze für die Arbeit.
    /// Für fremde Seiten gibt es [`Page::render_within`].
    pub fn render(&self, fonts: &Fonts<'_>, width: f64, height: f64) -> Scene {
        let viewport = Viewport {
            width,
            height,
            device_pixel_ratio: 1.0,
        };
        let device = device(viewport);
        let stylist = self
            .stylist(&device, fonts, &Budget::unlimited())
            .expect("unbegrenztes Budget");
        let images = |url: &Url| self.image(url);
        let tree = mb_layout::layout(&LayoutInput {
            document: &self.document,
            stylist: &stylist,
            device: &device,
            fonts,
            base: &self.base,
            images: &images,
        });
        tree.paint(&images)
    }

    /// Wie [`Page::render`], aber Kaskaden-Vorbereitung, Stil und Layout bezahlt aus `budget`;
    /// `Err`, wenn es verbraucht ist oder der Auftrag abgebrochen wurde.
    pub fn render_within(
        &self,
        fonts: &Fonts<'_>,
        width: f64,
        height: f64,
        budget: &Budget,
    ) -> Result<Scene, Exhausted> {
        let viewport = Viewport {
            width,
            height,
            device_pixel_ratio: 1.0,
        };
        let tree = self.layout_within(fonts, viewport, budget)?;
        Ok(self.paint(&tree, (0.0, 0.0)))
    }

    /// Kaskaden-Vorbereitung, Stil und Layout in `viewport`, bezahlt aus `budget`; `Err`, wenn es
    /// verbraucht ist oder der Auftrag abgebrochen wurde. Wie weit sich
    /// scrollen lässt, sagt [`FragmentTree::scroll_size`].
    pub fn layout_within(
        &self,
        fonts: &Fonts<'_>,
        viewport: Viewport,
        budget: &Budget,
    ) -> Result<FragmentTree, Exhausted> {
        let device = device(viewport);
        let stylist = self.stylist(&device, fonts, budget)?;
        let images = |url: &Url| self.image(url);
        let input = LayoutInput {
            document: &self.document,
            stylist: &stylist,
            device: &device,
            fonts,
            base: &self.base,
            images: &images,
        };
        mb_layout::layout_within(&input, budget)
    }

    /// Zeichnet, was der Viewport von `tree` zeigt, wenn er um `scroll`
    /// CSS-Pixel nach rechts und unten gescrollt ist.
    pub fn paint(&self, tree: &FragmentTree, scroll: (f64, f64)) -> Scene {
        tree.paint_scrolled(&|url: &Url| self.image(url), scroll)
    }
}

/// Lädt die Stylesheets hinter `@import`, aus dem Budget der Seite.
struct Imports<'a> {
    fetch: &'a Fetch,
    budget: &'a Bytes,
    quirks: bool,
    /// Wie viele Stylesheets noch geladen werden dürfen, siehe
    /// [`MAX_IMPORTS`].
    left: usize,
}

impl Imports<'_> {
    /// Lädt die Stylesheets der `@import`-Regeln von `sheet` und darin
    /// wieder. `encoding` ist die Kodierung von `sheet`, die Ersatzkodierung
    /// der importierten (CSS Cascade 5), `chain` die URLs der Stylesheets bis
    /// hierher ohne Fragment und `depth` deren Tiefe.
    fn load(
        &mut self,
        sheet: &mut StyleSheet,
        encoding: Encoding,
        chain: &mut Vec<String>,
        depth: usize,
    ) {
        if depth >= MAX_IMPORT_DEPTH {
            return;
        }
        for pending in sheet.pending_imports() {
            let key = pending.url().serialize(true);
            if chain.contains(&key) || self.left == 0 {
                continue;
            }
            self.left = self.left.saturating_sub(1);
            let Some(bytes) =
                (self.fetch)(pending.url()).filter(|bytes| self.budget.spend(length(bytes)))
            else {
                continue;
            };
            let (text, imported_encoding) = mb_css::decode_stylesheet(&bytes, None, Some(encoding));
            let context = ParseContext {
                base_url: Some(pending.url()),
                quirks_mode: self.quirks,
            };
            let mut imported = StyleSheet::parse(&text, context);
            chain.push(key);
            self.load(
                &mut imported,
                imported_encoding,
                chain,
                depth.saturating_add(1),
            );
            chain.pop();
            pending.load(imported);
        }
    }
}

fn device(viewport: Viewport) -> Device {
    Device {
        viewport_width: viewport.width,
        viewport_height: viewport.height,
        device_pixel_ratio: viewport.device_pixel_ratio,
        system_color,
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

fn decode_image(bytes: &[u8], max_pixels: u64) -> Option<Arc<Image>> {
    let png = mb_image::png::decode(bytes, max_pixels).ok()?;
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
