//! Das Programm MeinBrowser. Für Meilenstein M0 öffnet es ein Fenster mit einem
//! selbst gezeichneten Testbild, siehe `docs/architektur.md`, Abschnitt 12.
//!
//! - `meinbrowser` öffnet das Fenster. Mausklicks setzen Punkte ins Bild,
//!   Tasten und Texteingaben erscheinen auf der Standardausgabe.
//! - `meinbrowser --selbsttest [bild.ppm]` zeigt das Testbild, liest es aus
//!   dem Fenster zurück und prüft es an festen Messpunkten. Auf Wunsch wird das
//!   zurückgelesene Bild als PPM-Datei gespeichert.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mb_platform::{Capture, Event, KeyEvent, Window, WindowOptions};
use mb_raster::{Canvas, Color, PROBES, probe_position, test_image};

const USAGE: &str = "Aufruf: meinbrowser [--selbsttest [bild.ppm] | --version | --help]";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
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
