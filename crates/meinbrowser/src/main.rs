//! Das Programm MeinBrowser, siehe `docs/architektur.md`, Abschnitt 12.
//!
//! - `meinbrowser DATEI` zeigt eine lokale HTML- oder XHTML-Datei in der
//!   Größe des Fensters an (M1), ein Gerätepixel je CSS-Pixel. Scrollen gibt
//!   es noch nicht. Gezeichnet wird auf einem eigenen Faden, das Fenster
//!   bleibt dabei bedienbar: Eine neue Größe bricht den laufenden Auftrag
//!   ab, und für Stil und Layout gilt das Budget [`mb_web::RENDER_STEPS`].
//!   Ist es verbraucht, bleibt das letzte Bild stehen, und der Fehler
//!   erscheint auf der Standardfehlerausgabe. Die Seite darf Stylesheets und
//!   Bilder aus normalen lokalen Dateien laden ([`mb_web::read_file`]); das
//!   Fenster öffnet sich, während sie lädt.
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
use mb_layout::Fonts;
use mb_platform::{Capture, Event, KeyEvent, Window, WindowOptions};
use mb_raster::{Canvas, Color, PROBES, probe_position, test_image};
use mb_text::FontLibrary;
use mb_web::{Fetch, Page};

#[cfg(test)]
mod tests;

const USAGE: &str = "Aufruf: meinbrowser [DATEI | --selbsttest [bild.ppm] | --version | --help]";

/// Der Stapel des Fadens, der die Seite lädt und zeichnet. Layout und
/// Zeichnen arbeiten rekursiv, wie im Läufer der Referenztests.
const STACK: usize = 64 << 20;

/// So oft schaut das Fenster nach, ob ein Bild fertig ist, solange eins
/// aussteht. Sonst wartet es nur auf Ereignisse.
const POLL: Duration = Duration::from_millis(50);

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

/// Ein Auftrag an den Faden der Seite: zeichnen in dieser Größe.
struct Job {
    id: u64,
    width: u32,
    height: u32,
    cancel: Cancel,
}

/// Ein Bild, das der Faden der Seite noch zeichnet.
struct Pending {
    id: u64,
    cancel: Cancel,
}

/// Lädt und zeichnet eine Seite auf einem eigenen Faden mit großem Stapel.
struct Renderer {
    jobs: Sender<Job>,
    frames: Receiver<(u64, Result<Canvas, String>)>,
    next_id: Cell<u64>,
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
        let fetch: Fetch = Box::new(mb_web::read_file);
        std::thread::Builder::new()
            .name("Seite".into())
            .stack_size(STACK)
            .spawn(move || render_page(url, fetch, loaded_sender, requests, results))
            .map_err(|err| format!("Faden: {err}"))?;
        Ok(Renderer {
            jobs,
            frames,
            next_id: Cell::new(0),
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
    /// [`Renderer::request`].
    #[cfg(test)]
    fn render(&self, width: u32, height: u32) -> Result<Canvas, String> {
        let pending = self.request(width, height)?;
        loop {
            let (id, result) = self.frames.recv().map_err(|_| stopped())?;
            if id == pending.id {
                return result;
            }
        }
    }

    /// Gibt ein Bild in Auftrag, ohne zu warten.
    fn request(&self, width: u32, height: u32) -> Result<Pending, String> {
        let id = self.next_id.get();
        self.next_id.set(id.wrapping_add(1));
        let cancel = Cancel::new();
        let job = Job {
            id,
            width,
            height,
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
                Ok((id, result)) if id == pending.id => return Some(result),
                Ok(_) => {}
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => return Some(Err(stopped())),
            }
        }
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
    results: Sender<(u64, Result<Canvas, String>)>,
) {
    let mut library = FontLibrary::new(Box::new(|path| std::fs::read(path).ok()));
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
    while let Ok(mut job) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            let skipped = std::mem::replace(&mut job, newer);
            let cancelled = Err(Exhausted::Cancelled.to_string());
            if results.send((skipped.id, cancelled)).is_err() {
                return;
            }
        }
        let budget = Budget::with_cancel(mb_web::RENDER_STEPS, job.cancel);
        let canvas = page
            .render_within(&fonts, f64::from(job.width), f64::from(job.height), &budget)
            .map_err(|err| format!("Stil und Layout: {err}"))
            .and_then(|scene| mb_raster::render(&scene).map_err(|err| err.to_string()));
        if results.send((job.id, canvas)).is_err() {
            return;
        }
    }
}

/// Zeigt eine lokale Datei an und zeichnet sie bei jeder Größenänderung neu.
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
                if let Some(pending) = pending.take() {
                    pending.cancel.cancel();
                }
                let (width, height) = window.size();
                match renderer.request(width, height) {
                    Ok(next) => pending = Some(next),
                    Err(err) => {
                        eprintln!("meinbrowser: {err}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            Some(Event::RedrawRequested) => redraw = true,
            _ => {}
        }
        if let Some(waiting) = &pending
            && let Some(result) = renderer.poll(waiting)
        {
            pending = None;
            redraw = true;
            match result {
                Ok(canvas) => frame = Some(canvas),
                Err(err) => {
                    eprintln!("meinbrowser: {err}");
                    // Ohne ein früheres Bild bleibt das Fenster weiß.
                    if frame.is_none() {
                        let (width, height) = window.size();
                        frame = Canvas::new(width, height, Color::WHITE).ok();
                    }
                }
            }
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
