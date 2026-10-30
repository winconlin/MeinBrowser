//! Das Programm MeinBrowser, siehe `docs/architektur.md`, Abschnitt 12.
//!
//! - `meinbrowser DATEI` zeigt eine lokale HTML- oder XHTML-Datei in der
//!   Größe des Fensters an (M1), mit so vielen Gerätepixeln je CSS-Pixel,
//!   wie das System für das Fenster vorgibt ([`Window::scale`]). Gescrollt
//!   wird mit Mausrad oder Touchpad, mit den Pfeiltasten, Bild auf und ab,
//!   der Leertaste (mit Umschalttaste zurück), Pos1 und Ende; Balken am
//!   rechten und unteren Rand zeigen die Stelle. Gezeichnet wird auf einem
//!   eigenen Faden, das Fenster bleibt dabei bedienbar: Eine neue Größe
//!   bricht den laufenden Auftrag ab, und für Stil und Layout gilt das
//!   Budget [`mb_web::RENDER_STEPS`]. Zum Scrollen wird nur neu gezeichnet,
//!   nicht neu gesetzt. Ist das Budget verbraucht, bleibt das letzte Bild
//!   stehen, und der Fehler erscheint auf der Standardfehlerausgabe. Die
//!   Seite darf Stylesheets und Bilder aus normalen lokalen Dateien laden
//!   ([`mb_web::read_file`]); das Fenster öffnet sich, während sie lädt.
//! - `meinbrowser` öffnet ein Fenster mit einem selbst gezeichneten Testbild
//!   (M0). Mausklicks setzen Punkte ins Bild, Tasten und Texteingaben
//!   erscheinen auf der Standardausgabe.
//! - `meinbrowser --selbsttest [bild.ppm]` zeigt das Testbild, liest es aus
//!   dem Fenster zurück und prüft es an festen Messpunkten. Auf Wunsch wird das
//!   zurückgelesene Bild als PPM-Datei gespeichert.

#![forbid(unsafe_code)]

use std::cell::Cell;
use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use mb_base::{Budget, Cancel, Exhausted};
use mb_layout::{Fonts, FragmentTree};
use mb_platform::{Capture, Event, Key, KeyEvent, NamedKey, ScrollDelta, Window, WindowOptions};
use mb_raster::{Canvas, Color, PROBES, probe_position, test_image};
use mb_scene::{Rect, Scene};
use mb_text::FontLibrary;
use mb_web::{Fetch, Page, Viewport};

#[cfg(test)]
mod tests;

const USAGE: &str = "Aufruf: meinbrowser [DATEI | --selbsttest [bild.ppm] | --version | --help]";

/// Der Stapel des Fadens, der die Seite lädt und zeichnet. Layout und
/// Zeichnen arbeiten rekursiv, wie im Läufer der Referenztests.
const STACK: usize = 64 << 20;

/// So oft schaut das Fenster nach, ob ein Bild fertig ist, solange eins
/// aussteht. Sonst wartet es nur auf Ereignisse.
const POLL: Duration = Duration::from_millis(50);

/// So weit scrollen die Pfeiltasten, in CSS-Pixeln. Eine Raste des Mausrads
/// scrollt [`WHEEL_LINES`] solche Zeilen, eine Seite ist die Höhe des
/// Viewports ohne eine Zeile, die stehen bleibt, mindestens die halbe Höhe.
/// Festgelegt von diesem Browser.
const LINE: f64 = 40.0;
const WHEEL_LINES: f64 = 3.0;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if let [path] = args.as_slice()
        && !path.to_string_lossy().starts_with('-')
    {
        return view(Path::new(path));
    }
    let flag = args.first().map(|arg| arg.to_str());
    match (flag, args.len()) {
        (None, _) => interactive(),
        (Some(Some("--selbsttest")), 1) => self_test(None),
        (Some(Some("--selbsttest")), 2) => self_test(args.get(1).map(Path::new)),
        (Some(Some("--version")), 1) => {
            println!("MeinBrowser {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        (Some(Some("--help" | "-h")), 1) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Zeichnet das Testbild und die gesetzten Punkte.
fn draw(width: u32, height: u32, points: &[(f64, f64)]) -> Result<Canvas, String> {
    let mut canvas = test_image(width, height).map_err(|err| err.to_string())?;
    for &(x, y) in points {
        let (x, y) = (x as f32, y as f32);
        canvas.fill_circle(x, y, 9.0, Color::WHITE);
        canvas.fill_circle(x, y, 6.0, Color::rgb(0xe0, 0x20, 0x60));
    }
    Ok(canvas)
}

fn show(window: &mut Window, canvas: &Canvas) -> Result<(), String> {
    window
        .present(canvas.width(), canvas.height(), canvas.pixels())
        .map_err(|err| err.to_string())
}

/// Was ein Bild der Seite zeigen soll.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    /// Die Größe der Zeichenfläche in Gerätepixeln.
    width: u32,
    height: u32,
    /// Gerätepixel je CSS-Pixel.
    scale: f64,
    /// Die gewünschte Scrollposition in CSS-Pixeln.
    scroll: (f64, f64),
}

impl View {
    /// Ein Bild in dieser Größe, ein Gerätepixel je CSS-Pixel, ungescrollt.
    #[cfg(test)]
    fn new(width: u32, height: u32) -> View {
        View {
            width,
            height,
            scale: 1.0,
            scroll: (0.0, 0.0),
        }
    }

    /// Der Viewport in CSS-Pixeln.
    fn viewport(&self) -> Viewport {
        Viewport {
            width: f64::from(self.width) / self.scale,
            height: f64::from(self.height) / self.scale,
            device_pixel_ratio: self.scale,
        }
    }
}

/// Ein Auftrag an den Faden der Seite: zeichnen, was `view` zeigen soll.
struct Job {
    id: u64,
    view: View,
    cancel: Cancel,
}

/// Wo ein fertiges Bild gescrollt ist und wie weit es sich scrollen ließe,
/// in CSS-Pixeln.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Limits {
    scroll: (f64, f64),
    max: (f64, f64),
}

/// Ein fertiges Bild der Seite.
struct Frame {
    canvas: Canvas,
    limits: Limits,
}

/// Ein Bild, das der Faden der Seite noch zeichnet.
struct Pending {
    id: u64,
    cancel: Cancel,
}

/// Lädt und zeichnet eine Seite auf einem eigenen Faden mit großem Stapel.
struct Renderer {
    jobs: Sender<Job>,
    frames: Receiver<(u64, Result<Frame, String>)>,
    next_id: Cell<u64>,
    /// Die Scrollgrenzen des zuletzt gelieferten Bildes.
    limits: Cell<Option<Limits>>,
    /// Meldet einmal, ob sich die Seite laden ließ; danach steht es in
    /// `state`.
    loaded: Receiver<bool>,
    state: Cell<Option<bool>>,
    /// Der Pfad für Meldungen.
    name: String,
}

impl Renderer {
    /// Startet den Faden, der die Seite lädt, ohne zu warten.
    fn spawn(path: &Path) -> Result<Renderer, String> {
        let name = path.display().to_string();
        let url = mb_web::file_url(path).ok_or_else(|| format!("{name}: kein gültiger Pfad"))?;
        let (loaded_sender, loaded) = mpsc::channel();
        let (jobs, requests) = mpsc::channel();
        let (results, frames) = mpsc::channel();
        let fetch: Fetch = Box::new(mb_web::read_file_within);
        std::thread::Builder::new()
            .name("Seite".into())
            .stack_size(STACK)
            .spawn(move || render_page(url, fetch, loaded_sender, requests, results))
            .map_err(|err| format!("Faden: {err}"))?;
        Ok(Renderer {
            jobs,
            frames,
            next_id: Cell::new(0),
            limits: Cell::new(None),
            loaded,
            state: Cell::new(None),
            name,
        })
    }

    /// Startet den Faden und wartet, bis die Seite geladen ist. Das Fenster
    /// wartet nicht, es nimmt [`Renderer::spawn`].
    #[cfg(test)]
    fn start(path: &Path) -> Result<Renderer, String> {
        let renderer = Renderer::spawn(path)?;
        let loaded = renderer.loaded.recv().unwrap_or(false);
        renderer.state.set(Some(loaded));
        if loaded {
            Ok(renderer)
        } else {
            Err(renderer.unloadable())
        }
    }

    /// Ob sich die Seite laden ließ; `None`, solange sie lädt.
    fn loaded(&self) -> Option<bool> {
        if self.state.get().is_none() {
            match self.loaded.try_recv() {
                Ok(loaded) => self.state.set(Some(loaded)),
                Err(TryRecvError::Disconnected) => self.state.set(Some(false)),
                Err(TryRecvError::Empty) => {}
            }
        }
        self.state.get()
    }

    fn unloadable(&self) -> String {
        format!("{}: lässt sich nicht laden", self.name)
    }

    /// Zeichnet die Seite in einem Viewport von `width` × `height` Pixeln
    /// und wartet auf das Bild. Das Fenster wartet nicht, es nimmt
    /// [`Renderer::request_view`].
    #[cfg(test)]
    fn render(&self, width: u32, height: u32) -> Result<Canvas, String> {
        self.render_view(View::new(width, height))
    }

    /// Zeichnet, was `view` zeigen soll, und wartet auf das Bild.
    #[cfg(test)]
    fn render_view(&self, view: View) -> Result<Canvas, String> {
        let pending = self.request_view(view)?;
        loop {
            let (id, result) = self.frames.recv().map_err(|_| stopped())?;
            if id == pending.id {
                return self.deliver(result);
            }
        }
    }

    /// Gibt ein Bild in einem Viewport von `width` × `height` Pixeln in
    /// Auftrag, ohne zu warten.
    #[cfg(test)]
    fn request(&self, width: u32, height: u32) -> Result<Pending, String> {
        self.request_view(View::new(width, height))
    }

    /// Gibt ein Bild in Auftrag, ohne zu warten.
    fn request_view(&self, view: View) -> Result<Pending, String> {
        let id = self.next_id.get();
        self.next_id.set(id.wrapping_add(1));
        let cancel = Cancel::new();
        let job = Job {
            id,
            view,
            cancel: cancel.clone(),
        };
        self.jobs.send(job).map_err(|_| stopped())?;
        Ok(Pending { id, cancel })
    }

    /// Das Bild zu `pending`, falls es fertig ist, ohne zu warten. Ältere
    /// Bilder werden verworfen.
    fn poll(&self, pending: &Pending) -> Option<Result<Canvas, String>> {
        loop {
            match self.frames.try_recv() {
                Ok((id, result)) if id == pending.id => return Some(self.deliver(result)),
                Ok(_) => {}
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => return Some(Err(stopped())),
            }
        }
    }

    /// Merkt sich die Scrollgrenzen eines gelieferten Bildes.
    fn deliver(&self, result: Result<Frame, String>) -> Result<Canvas, String> {
        result.map(|frame| {
            self.limits.set(Some(frame.limits));
            frame.canvas
        })
    }

    /// Die Scrollgrenzen des zuletzt gelieferten Bildes; `None` vor dem
    /// ersten.
    fn limits(&self) -> Option<Limits> {
        self.limits.get()
    }
}

fn stopped() -> String {
    "der Faden der Seite ist beendet".to_owned()
}

/// Der Faden eines [`Renderer`]: lädt die Seite mit `fetch`, meldet über
/// `loaded`, ob das ging, und zeichnet sie dann für jeden Auftrag aus
/// `requests`. Von mehreren wartenden Aufträgen zählt nur der neueste; die
/// anderen enden als abgebrochen.
fn render_page(
    url: mb_url::Url,
    fetch: Fetch,
    loaded: Sender<bool>,
    requests: Receiver<Job>,
    results: Sender<(u64, Result<Frame, String>)>,
) {
    let mut library = FontLibrary::new(Box::new(mb_platform::fonts::read_font));
    for file in mb_platform::fonts::font_files() {
        library.add_file(file);
    }
    let fonts = Fonts::new(&library);
    let page = Page::load(url, fetch);
    if loaded.send(page.is_some()).is_err() {
        return;
    }
    let Some(page) = page else {
        return;
    };
    let mut laid_out: Option<LaidOut> = None;
    while let Ok(mut job) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            let skipped = std::mem::replace(&mut job, newer);
            let cancelled = Err(Exhausted::Cancelled.to_string());
            if results.send((skipped.id, cancelled)).is_err() {
                return;
            }
        }
        let frame = draw_page(&page, &fonts, &mut laid_out, &job);
        if results.send((job.id, frame)).is_err() {
            return;
        }
    }
}

/// Das Layout für eine Größe und Skalierung. Ein zu teures bleibt als
/// Fehler gemerkt, damit Scrollen es nicht jedes Mal neu versucht.
struct LaidOut {
    width: u32,
    height: u32,
    scale: f64,
    tree: Result<FragmentTree, Exhausted>,
}

/// Zeichnet ein Bild für `job`; gesetzt wird nur, wenn sich Größe oder
/// Skalierung geändert haben. Die Scrollposition wird auf den möglichen
/// Bereich begrenzt und auf ganze Gerätepixel abgerundet, damit Kanten und
/// Text scharf bleiben.
fn draw_page(
    page: &Page,
    fonts: &Fonts<'_>,
    laid_out: &mut Option<LaidOut>,
    job: &Job,
) -> Result<Frame, String> {
    let view = job.view;
    let viewport = view.viewport();
    let fresh = laid_out.as_ref().is_some_and(|laid_out| {
        (laid_out.width, laid_out.height, laid_out.scale) == (view.width, view.height, view.scale)
    });
    if !fresh {
        let budget = Budget::with_cancel(mb_web::RENDER_STEPS, job.cancel.clone());
        let tree = page.layout_within(fonts, viewport, &budget);
        *laid_out = match tree {
            // Ein abgebrochener Auftrag sagt nichts über den nächsten.
            Err(Exhausted::Cancelled) => None,
            tree => Some(LaidOut {
                width: view.width,
                height: view.height,
                scale: view.scale,
                tree,
            }),
        };
    }
    let tree = match laid_out.as_ref().map(|laid_out| &laid_out.tree) {
        Some(Ok(tree)) => tree,
        Some(Err(err)) => return Err(format!("Stil und Layout: {err}")),
        None => return Err(format!("Stil und Layout: {}", Exhausted::Cancelled)),
    };
    let size = tree.scroll_size();
    let max = (
        (size.0 - viewport.width).max(0.0),
        (size.1 - viewport.height).max(0.0),
    );
    let snap = |value: f64, max: f64| {
        if value.is_finite() {
            (value.clamp(0.0, max) * view.scale).floor() / view.scale
        } else {
            0.0
        }
    };
    let scroll = (snap(view.scroll.0, max.0), snap(view.scroll.1, max.1));
    let mut scene = page.paint(tree, scroll);
    scrollbars(&mut scene, &viewport, size, scroll);
    let canvas = mb_raster::render_scaled(&scene, view.width, view.height, view.scale as f32)
        .map_err(|err| err.to_string())?;
    Ok(Frame {
        canvas,
        limits: Limits { scroll, max },
    })
}

/// Zeichnet Balken über den Inhalt, die zeigen, welcher Teil des Dokuments
/// zu sehen ist: rechts, wenn es höher ist als der Viewport, unten, wenn es
/// breiter ist. Die Seite verliert dadurch keinen Platz, wie bei Leisten,
/// die über dem Inhalt liegen; CSS Overflow 3 überlässt das dem Browser.
/// Der dunkle Balken hat einen hellen Rand, damit er auf jedem Hintergrund
/// zu sehen ist.
fn scrollbars(scene: &mut Scene, viewport: &Viewport, size: (f64, f64), scroll: (f64, f64)) {
    // Dicke, Abstand vom Rand und kleinste Länge in CSS-Pixeln.
    const THICKNESS: f64 = 6.0;
    const EDGE: f64 = 2.0;
    const MIN_LENGTH: f64 = 16.0;
    let vertical = size.1 > viewport.height;
    let horizontal = size.0 > viewport.width;
    // Ohne den anderen Balken reicht die Bahn bis zur Ecke.
    let corner = |other: bool| if other { THICKNESS + EDGE } else { 0.0 };
    // Anfang und Länge eines Balkens auf seiner Bahn.
    let thumb = |visible: f64, total: f64, position: f64, track: f64| {
        let length = (track * visible / total).max(MIN_LENGTH).min(track);
        let max = total - visible;
        let offset = if max > 0.0 {
            (track - length) * (position / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (EDGE + offset, length)
    };
    let mut bars = Vec::new();
    if vertical {
        let track = viewport.height - 2.0 * EDGE - corner(horizontal);
        if track > 0.0 {
            let (y, length) = thumb(viewport.height, size.1, scroll.1, track);
            let x = viewport.width - EDGE - THICKNESS;
            bars.push((x, y, THICKNESS, length));
        }
    }
    if horizontal {
        let track = viewport.width - 2.0 * EDGE - corner(vertical);
        if track > 0.0 {
            let (x, length) = thumb(viewport.width, size.0, scroll.0, track);
            let y = viewport.height - EDGE - THICKNESS;
            bars.push((x, y, length, THICKNESS));
        }
    }
    let rect = |x: f64, y: f64, width: f64, height: f64| {
        Rect::new(x as f32, y as f32, width as f32, height as f32)
    };
    for (x, y, width, height) in bars {
        scene.push_rect(
            rect(x - 1.0, y - 1.0, width + 2.0, height + 2.0),
            mb_scene::Color::rgba(255, 255, 255, 128),
        );
        scene.push_rect(
            rect(x, y, width, height),
            mb_scene::Color::rgba(0, 0, 0, 128),
        );
    }
}

/// Begrenzt eine Scrollposition auf `[0, max]`; was nicht endlich ist, wird
/// null.
fn clamp_scroll(target: (f64, f64), max: (f64, f64)) -> (f64, f64) {
    let clamp = |value: f64, max: f64| {
        if value.is_finite() {
            value.clamp(0.0, max.max(0.0))
        } else {
            0.0
        }
    };
    (clamp(target.0, max.0), clamp(target.1, max.1))
}

/// Die Scrollposition nach Mausrad oder Touchpad; `scale` sind die
/// Gerätepixel je CSS-Pixel.
fn wheel_scroll(delta: ScrollDelta, scale: f64, limits: Limits, scroll: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = match delta {
        ScrollDelta::Steps { x, y } => (x * WHEEL_LINES * LINE, y * WHEEL_LINES * LINE),
        ScrollDelta::Pixels { x, y } => (x / scale, y / scale),
    };
    clamp_scroll((scroll.0 + dx, scroll.1 + dy), limits.max)
}

/// Die Scrollposition nach einem Tastendruck in einem Viewport der Höhe
/// `height`; `None` für Tasten, die nicht scrollen. Mit Strg, Alt oder der
/// Meta-Taste scrollt nichts, das sind Tastenkürzel.
fn key_scroll(
    key: &KeyEvent,
    height: f64,
    limits: Limits,
    scroll: (f64, f64),
) -> Option<(f64, f64)> {
    let modifiers = key.modifiers;
    if !key.pressed || modifiers.control || modifiers.alt || modifiers.meta {
        return None;
    }
    let page = (height - LINE).max(height / 2.0);
    let (x, y) = scroll;
    let target = match key.key {
        Key::Named(NamedKey::ArrowDown) => (x, y + LINE),
        Key::Named(NamedKey::ArrowUp) => (x, y - LINE),
        Key::Named(NamedKey::ArrowRight) => (x + LINE, y),
        Key::Named(NamedKey::ArrowLeft) => (x - LINE, y),
        Key::Named(NamedKey::PageDown) => (x, y + page),
        Key::Named(NamedKey::PageUp) => (x, y - page),
        Key::Character(' ') if modifiers.shift => (x, y - page),
        Key::Character(' ') => (x, y + page),
        Key::Named(NamedKey::Home) => (x, 0.0),
        Key::Named(NamedKey::End) => (x, limits.max.1),
        _ => return None,
    };
    Some(clamp_scroll(target, limits.max))
}

/// Zeigt eine lokale Datei an und zeichnet sie bei jeder Größenänderung und
/// beim Scrollen neu.
fn view(path: &Path) -> ExitCode {
    // Was offensichtlich fehlt, wird gemeldet, bevor das Fenster aufgeht.
    if !std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        eprintln!("meinbrowser: {}: keine lesbare Datei", path.display());
        return ExitCode::FAILURE;
    }
    let renderer = match Renderer::spawn(path) {
        Ok(renderer) => renderer,
        Err(err) => {
            eprintln!("meinbrowser: {err}");
            return ExitCode::FAILURE;
        }
    };
    let name = path
        .file_name()
        .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy());
    let title = format!("{name} – MeinBrowser");
    let options = WindowOptions {
        title: &title,
        width: 800,
        height: 600,
    };
    let mut window = match Window::open(&options) {
        Ok(window) => window,
        Err(err) => {
            eprintln!("meinbrowser: Fenster lässt sich nicht öffnen: {err}");
            return ExitCode::FAILURE;
        }
    };
    let mut frame: Option<Canvas> = None;
    let mut pending: Option<Pending> = None;
    // Die gewünschte Scrollposition in CSS-Pixeln, und ob dafür noch ein
    // Bild in Auftrag zu geben ist. Es steht höchstens eins aus; was sich
    // inzwischen ändert, kommt mit dem nächsten.
    let mut scroll = (0.0, 0.0);
    let mut outdated = false;
    // Derselbe Fehler wird nur einmal gemeldet, nicht bei jedem Bild.
    let mut last_error: Option<String> = None;
    loop {
        // Solange ein Bild aussteht, kommt die Schleife regelmäßig zurück.
        let timeout = pending.as_ref().map(|_| POLL);
        let event = match window.next_event(timeout) {
            Ok(event) => event,
            Err(err) => {
                eprintln!("meinbrowser: {err}");
                return ExitCode::FAILURE;
            }
        };
        if renderer.loaded() == Some(false) {
            eprintln!("meinbrowser: {}", renderer.unloadable());
            return ExitCode::FAILURE;
        }
        let mut redraw = false;
        match event {
            Some(Event::CloseRequested) => {
                if let Some(pending) = &pending {
                    pending.cancel.cancel();
                }
                return ExitCode::SUCCESS;
            }
            Some(Event::Resized { .. }) => {
                // Das Bild in der alten Größe braucht niemand mehr.
                if let Some(pending) = pending.take() {
                    pending.cancel.cancel();
                }
                outdated = true;
            }
            Some(Event::RedrawRequested) => redraw = true,
            Some(Event::Scroll(delta)) => {
                let limits = renderer.limits().unwrap_or_default();
                let next = wheel_scroll(delta, window.scale(), limits, scroll);
                outdated |= next != scroll;
                scroll = next;
            }
            Some(Event::Key(key)) => {
                let limits = renderer.limits().unwrap_or_default();
                let height = f64::from(window.size().1) / window.scale();
                if let Some(next) = key_scroll(&key, height, limits, scroll) {
                    outdated |= next != scroll;
                    scroll = next;
                }
            }
            _ => {}
        }
        if let Some(waiting) = &pending
            && let Some(result) = renderer.poll(waiting)
        {
            pending = None;
            redraw = true;
            match result {
                Ok(canvas) => {
                    frame = Some(canvas);
                    last_error = None;
                    // Nach einer neuen Größe kann der Bereich kleiner sein.
                    if let Some(limits) = renderer.limits() {
                        scroll = clamp_scroll(scroll, limits.max);
                    }
                }
                Err(err) => {
                    if last_error.as_ref() != Some(&err) {
                        eprintln!("meinbrowser: {err}");
                        last_error = Some(err);
                    }
                    // Ohne ein früheres Bild bleibt das Fenster weiß.
                    if frame.is_none() {
                        let (width, height) = window.size();
                        frame = Canvas::new(width, height, Color::WHITE).ok();
                    }
                }
            }
        }
        if outdated && pending.is_none() {
            let (width, height) = window.size();
            let view = View {
                width,
                height,
                scale: window.scale(),
                scroll,
            };
            match renderer.request_view(view) {
                Ok(next) => pending = Some(next),
                Err(err) => {
                    eprintln!("meinbrowser: {err}");
                    return ExitCode::FAILURE;
                }
            }
            outdated = false;
        }
        if redraw
            && let Some(canvas) = &frame
            && let Err(err) = show(&mut window, canvas)
        {
            eprintln!("meinbrowser: {err}");
            return ExitCode::FAILURE;
        }
    }
}

fn interactive() -> ExitCode {
    let options = WindowOptions {
        title: "MeinBrowser",
        width: 960,
        height: 600,
    };
    let mut window = match Window::open(&options) {
        Ok(window) => window,
        Err(err) => {
            eprintln!("meinbrowser: Fenster lässt sich nicht öffnen: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "MeinBrowser {}: Klicks setzen Punkte, Tasten erscheinen hier.",
        env!("CARGO_PKG_VERSION")
    );
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut frame: Option<Canvas> = None;
    loop {
        let event = match window.next_event(None) {
            Ok(Some(event)) => event,
            Ok(None) => continue,
            Err(err) => {
                eprintln!("meinbrowser: {err}");
                return ExitCode::FAILURE;
            }
        };
        let redraw_requested = matches!(event, Event::RedrawRequested);
        let redraw = match event {
            Event::CloseRequested => return ExitCode::SUCCESS,
            Event::Resized { .. } => true,
            Event::RedrawRequested => frame.is_none(),
            Event::PointerButton {
                pressed: true,
                x,
                y,
                ..
            } => {
                if points.len() == 32 {
                    points.remove(0);
                }
                points.push((x, y));
                true
            }
            Event::Key(KeyEvent {
                key,
                pressed,
                repeat,
                modifiers,
            }) => {
                let action = match (pressed, repeat) {
                    (true, false) => "gedrückt",
                    (true, true) => "wiederholt",
                    (false, _) => "losgelassen",
                };
                println!("Taste {action}: {key:?} {modifiers:?}");
                false
            }
            Event::Text(text) => {
                println!("Text: {text:?}");
                false
            }
            Event::Focused(focused) => {
                println!("Fokus: {}", if focused { "ja" } else { "nein" });
                false
            }
            _ => false,
        };
        if redraw {
            let (width, height) = window.size();
            match draw(width, height, &points) {
                Ok(canvas) => frame = Some(canvas),
                Err(err) => {
                    eprintln!("meinbrowser: {err}");
                    return ExitCode::FAILURE;
                }
            }
        }
        if (redraw || redraw_requested)
            && let Some(canvas) = &frame
            && let Err(err) = show(&mut window, canvas)
        {
            eprintln!("meinbrowser: {err}");
            return ExitCode::FAILURE;
        }
    }
}

/// Größte Abweichung je Farbkanal, die noch als Treffer gilt. Systeme dürfen
/// Farben leicht anpassen, etwa durch Farbmanagement.
const TOLERANCE: u32 = 16;

fn self_test(save: Option<&Path>) -> ExitCode {
    match run_self_test(save) {
        Ok(report) => {
            println!("Selbsttest bestanden: {report}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("Selbsttest gescheitert: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_self_test(save: Option<&Path>) -> Result<String, String> {
    let options = WindowOptions {
        title: "MeinBrowser Selbsttest",
        width: 640,
        height: 400,
    };
    let start = Instant::now();
    let mut window = Window::open(&options).map_err(|err| format!("Fenster: {err}"))?;
    // Zeichnen, bis eine halbe Sekunde lang nichts Neues mehr kommt.
    let mut frame: Option<Canvas> = None;
    let mut last_present: Option<Instant> = None;
    loop {
        if last_present.is_some_and(|time| time.elapsed() >= Duration::from_millis(500)) {
            break;
        }
        if start.elapsed() > Duration::from_secs(20) {
            return Err("das Fenster ist nicht rechtzeitig erschienen".into());
        }
        let event = window
            .next_event(Some(Duration::from_millis(50)))
            .map_err(|err| format!("Ereignis: {err}"))?;
        match event {
            Some(Event::Resized { .. }) | Some(Event::RedrawRequested) => {
                let (width, height) = window.size();
                let canvas = draw(width, height, &[])?;
                show(&mut window, &canvas)?;
                frame = Some(canvas);
                last_present = Some(Instant::now());
            }
            Some(Event::CloseRequested) => return Err("Fenster wurde geschlossen".into()),
            _ => {}
        }
    }
    let frame = frame.ok_or("kein Bild gezeichnet")?;
    let capture = window
        .capture()
        .map_err(|err| format!("Zurücklesen: {err}"))?;
    if let Some(path) = save {
        write_ppm(path, &capture).map_err(|err| format!("{}: {err}", path.display()))?;
    }

    let mut failures = Vec::new();
    for probe in PROBES {
        let (x, y) = probe_position(probe, frame.width(), frame.height());
        let expected = frame.pixel(x, y).unwrap_or(0) & 0x00ff_ffff;
        let (cx, cy) = probe_position(probe, capture.width, capture.height);
        let actual = capture.pixel(cx, cy);
        let matches = actual.is_some_and(|actual| {
            [16, 8, 0].iter().all(|shift| {
                let a = (actual >> shift) & 0xff;
                let e = (expected >> shift) & 0xff;
                a.abs_diff(e) <= TOLERANCE
            })
        });
        if !matches {
            failures.push(format!(
                "Messpunkt ({cx}, {cy}): erwartet #{expected:06x}, angezeigt {}",
                actual.map_or("nichts".to_owned(), |a| format!("#{a:06x}"))
            ));
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "{} von {} Messpunkten weichen ab (Bild {}×{}, zurückgelesen {}×{}):\n  {}",
            failures.len(),
            PROBES.len(),
            frame.width(),
            frame.height(),
            capture.width,
            capture.height,
            failures.join("\n  ")
        ));
    }
    Ok(format!(
        "Testbild {}×{} angezeigt, zurückgelesen {}×{}, alle {} Messpunkte stimmen",
        frame.width(),
        frame.height(),
        capture.width,
        capture.height,
        PROBES.len()
    ))
}

/// Speichert ein Bild im PPM-Format (P6), das jedes Bildprogramm öffnet.
fn write_ppm(path: &Path, capture: &Capture) -> std::io::Result<()> {
    let mut bytes = format!("P6\n{} {}\n255\n", capture.width, capture.height).into_bytes();
    for pixel in &capture.pixels {
        bytes.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8]);
    }
    std::fs::write(path, bytes)
}
