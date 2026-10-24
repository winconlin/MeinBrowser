//! Fenster unter Windows über Win32, Bildausgabe über GDI.
//!
//! Das Programm erklärt sich für hohe Auflösungen zuständig (pro Monitor),
//! damit Windows das Bild nicht unscharf hochskaliert. Alle Größen sind daher
//! echte Gerätepixel.

mod ffi;
mod keys;

use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem::size_of;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use core::ffi::{c_int, c_void};

use crate::event::{Event, KeyEvent, Modifiers, PointerButton, ScrollDelta};
use crate::{Capture, Error, WindowOptions};

use ffi::*;

const CLASS_NAME: &str = "MeinBrowserFenster";

/// Ein Mausrad meldet je Raste 120 Einheiten (`WHEEL_DELTA`).
const WHEEL_DELTA: f64 = 120.0;

thread_local! {
    /// Eine Panic in der Fensterprozedur, die nach `DispatchMessageW`
    /// weitergereicht wird. Über die Systemgrenze darf sie nicht laufen.
    static PANIC: RefCell<Option<Box<dyn Any + Send>>> = const { RefCell::new(None) };
}

fn resume_panic() {
    if let Some(payload) = PANIC.with(|slot| slot.borrow_mut().take()) {
        panic::resume_unwind(payload);
    }
}

fn last_error(function: &'static str) -> Error {
    // SAFETY: GetLastError hat keine Vorbedingungen.
    let code = unsafe { GetLastError() };
    Error::System {
        function,
        code: i64::from(code),
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn struct_size<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
}

fn loword(value: usize) -> u16 {
    (value & 0xffff) as u16
}

fn hiword(value: usize) -> u16 {
    ((value >> 16) & 0xffff) as u16
}

/// Registriert die Fensterklasse einmal pro Prozess und schaltet die
/// DPI-Behandlung pro Monitor ein.
fn register_class() -> Result<HINSTANCE, Error> {
    static REGISTERED: OnceLock<Result<usize, i64>> = OnceLock::new();
    let result = REGISTERED.get_or_init(|| {
        // SAFETY: Das Pseudo-Handle ist ein dokumentierter Wert. Scheitert der
        // Aufruf (etwa weil ein Manifest die Einstellung schon setzt), bleibt
        // es bei der bisherigen Einstellung.
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 as HANDLE);
        }
        // SAFETY: Ein Nullzeiger liefert das Modul des laufenden Programms.
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        let class_name = wide(CLASS_NAME);
        let class = WNDCLASSEXW {
            cbSize: struct_size::<WNDCLASSEXW>(),
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: ptr::null_mut(),
            // SAFETY: Systemcursor werden mit Null als Modul und der
            // Ressourcennummer anstelle eines Namens geladen.
            hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW as *const u16) },
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: ptr::null_mut(),
        };
        // SAFETY: `class` ist vollständig belegt, der Klassenname lebt bis
        // nach dem Aufruf (Windows kopiert ihn).
        if unsafe { RegisterClassExW(&class) } == 0 {
            // SAFETY: GetLastError hat keine Vorbedingungen.
            return Err(i64::from(unsafe { GetLastError() }));
        }
        Ok(instance as usize)
    });
    match result {
        Ok(instance) => Ok(*instance as HINSTANCE),
        Err(code) => Err(Error::System {
            function: "RegisterClassExW",
            code: *code,
        }),
    }
}

/// Zustand eines Fensters, den die Fensterprozedur über `GWLP_USERDATA`
/// erreicht. Ein Borrow wird nie über einen Aufruf gehalten, der Nachrichten
/// auslösen kann (RS-10).
struct State {
    events: VecDeque<Event>,
    width: u32,
    height: u32,
    /// Das zuletzt gezeigte Bild, für `WM_PAINT`.
    image: Vec<u32>,
    image_width: u32,
    image_height: u32,
    high_surrogate: Option<u16>,
    tracking_leave: bool,
}

pub struct Window {
    hwnd: HWND,
    state: Box<RefCell<State>>,
}

impl Window {
    pub fn open(options: &WindowOptions<'_>) -> Result<Window, Error> {
        let instance = register_class()?;
        let too_large = || {
            Error::InvalidArgument(format!(
                "Fenstergröße {}×{} ist zu groß",
                options.width, options.height
            ))
        };
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: i32::try_from(options.width).map_err(|_| too_large())?,
            bottom: i32::try_from(options.height).map_err(|_| too_large())?,
        };
        // SAFETY: `rect` ist ein gültiger Zeiger auf ein RECT.
        unsafe { AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, FALSE, 0) };
        let state = Box::new(RefCell::new(State {
            events: VecDeque::new(),
            width: options.width,
            height: options.height,
            image: Vec::new(),
            image_width: 0,
            image_height: 0,
            high_surrogate: None,
            tracking_leave: false,
        }));
        let class_name = wide(CLASS_NAME);
        let title = wide(options.title);
        let state_pointer: *const RefCell<State> = &*state;
        // SAFETY: Klassenname und Titel sind nullterminiert und leben bis nach
        // dem Aufruf. Der Zustand liegt auf dem Heap und lebt länger als das
        // Fenster: `Drop` löst ihn vor `DestroyWindow` vom Fenster.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right.saturating_sub(rect.left),
                rect.bottom.saturating_sub(rect.top),
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                state_pointer.cast_mut().cast::<c_void>(),
            )
        };
        resume_panic();
        if hwnd.is_null() {
            return Err(last_error("CreateWindowExW"));
        }
        let window = Window { hwnd, state };
        // SAFETY: `hwnd` ist das eben erzeugte, gültige Fenster.
        unsafe {
            ShowWindow(hwnd, SW_SHOWNORMAL);
            UpdateWindow(hwnd);
        }
        resume_panic();
        let (width, height) = window.client_size();
        {
            let mut state = window.state.borrow_mut();
            state.width = width;
            state.height = height;
            state.events.push_front(Event::Resized { width, height });
        }
        Ok(window)
    }

    fn client_size(&self) -> (u32, u32) {
        let mut rect = RECT::default();
        // SAFETY: Gültiges Fenster und gültiger Zeiger auf ein RECT.
        if unsafe { GetClientRect(self.hwnd, &mut rect) } == 0 {
            let state = self.state.borrow();
            return (state.width, state.height);
        }
        let width = u32::try_from(rect.right.saturating_sub(rect.left)).unwrap_or(0);
        let height = u32::try_from(rect.bottom.saturating_sub(rect.top)).unwrap_or(0);
        (width, height)
    }

    pub fn size(&self) -> (u32, u32) {
        let state = self.state.borrow();
        (state.width, state.height)
    }

    pub fn present(&mut self, width: u32, height: u32, pixels: &[u32]) -> Result<(), Error> {
        {
            let mut state = self.state.borrow_mut();
            state.image.clear();
            state.image.extend_from_slice(pixels);
            state.image_width = width;
            state.image_height = height;
        }
        // SAFETY: Gültiges Fenster.
        let hdc = unsafe { GetDC(self.hwnd) };
        if hdc.is_null() {
            return Err(last_error("GetDC"));
        }
        let drawn = blit(hdc, pixels, width, height);
        // SAFETY: `hdc` stammt von GetDC für dieses Fenster.
        unsafe { ReleaseDC(self.hwnd, hdc) };
        if drawn {
            Ok(())
        } else {
            Err(last_error("StretchDIBits"))
        }
    }

    pub fn next_event(&mut self, timeout: Option<Duration>) -> Result<Option<Event>, Error> {
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        loop {
            if let Some(event) = self.state.borrow_mut().events.pop_front() {
                return Ok(Some(event));
            }
            let mut message = MSG {
                hwnd: ptr::null_mut(),
                message: 0,
                wParam: 0,
                lParam: 0,
                time: 0,
                pt: POINT::default(),
            };
            // SAFETY: `message` ist ein gültiger Zeiger auf ein MSG.
            while unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                // SAFETY: `message` stammt von PeekMessageW. Während der
                // Verteilung hält niemand einen Borrow auf den Zustand.
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                resume_panic();
                if !self.state.borrow().events.is_empty() {
                    break;
                }
            }
            if !self.state.borrow().events.is_empty() {
                continue;
            }
            let wait = match deadline {
                None => INFINITE,
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Ok(None);
                    }
                    let millis = (deadline - now).as_micros().div_ceil(1000);
                    u32::try_from(millis)
                        .unwrap_or(INFINITE - 1)
                        .min(INFINITE - 1)
                }
            };
            // SAFETY: Keine Handles, nur die Nachrichtenwarteschlange.
            let result =
                unsafe { MsgWaitForMultipleObjects(0, ptr::null(), FALSE, wait, QS_ALLINPUT) };
            if result == WAIT_FAILED {
                return Err(last_error("MsgWaitForMultipleObjects"));
            }
        }
    }

    pub fn capture(&mut self) -> Result<Capture, Error> {
        let (width, height) = self.size();
        let (w, h) = (
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(height).unwrap_or(i32::MAX),
        );
        let count = (width as usize).saturating_mul(height as usize);
        let info = bitmap_info(width, height);
        // SAFETY: Alle Handles werden geprüft und in umgekehrter Reihenfolge
        // freigegeben. CreateDIBSection liefert einen Puffer für
        // `width * height` Pixel mit 32 Bit, der bis DeleteObject gültig ist.
        unsafe {
            let window_dc = GetDC(self.hwnd);
            if window_dc.is_null() {
                return Err(last_error("GetDC"));
            }
            let memory_dc = CreateCompatibleDC(window_dc);
            let mut bits: *mut c_void = ptr::null_mut();
            let bitmap = CreateDIBSection(
                memory_dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                ptr::null_mut(),
                0,
            );
            let mut result = Err(last_error("CreateDIBSection"));
            if !memory_dc.is_null() && !bitmap.is_null() && !bits.is_null() {
                let previous = SelectObject(memory_dc, bitmap);
                if BitBlt(memory_dc, 0, 0, w, h, window_dc, 0, 0, SRCCOPY) == 0 {
                    result = Err(last_error("BitBlt"));
                } else {
                    GdiFlush();
                    let pixels = std::slice::from_raw_parts(bits.cast::<u32>(), count);
                    result = Ok(Capture {
                        width,
                        height,
                        pixels: pixels.iter().map(|pixel| pixel & 0x00ff_ffff).collect(),
                    });
                }
                SelectObject(memory_dc, previous);
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !memory_dc.is_null() {
                DeleteDC(memory_dc);
            }
            ReleaseDC(self.hwnd, window_dc);
            result
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: Erst wird der Zustand vom Fenster gelöst, dann das Fenster
        // zerstört. Nachrichten beim Zerstören landen bei DefWindowProcW.
        unsafe {
            set_window_long_ptr(self.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.hwnd);
        }
    }
}

fn bitmap_info(width: u32, height: u32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: struct_size::<BITMAPINFOHEADER>(),
            biWidth: i32::try_from(width).unwrap_or(i32::MAX),
            // Negative Höhe: Zeilen von oben nach unten.
            biHeight: -i32::try_from(height).unwrap_or(i32::MAX),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [RGBQUAD {
            rgbBlue: 0,
            rgbGreen: 0,
            rgbRed: 0,
            rgbReserved: 0,
        }],
    }
}

/// Kopiert ein Bild 1:1 in einen Gerätekontext.
fn blit(hdc: HDC, pixels: &[u32], width: u32, height: u32) -> bool {
    let info = bitmap_info(width, height);
    let (w, h) = (
        i32::try_from(width).unwrap_or(i32::MAX),
        i32::try_from(height).unwrap_or(i32::MAX),
    );
    // SAFETY: `pixels` enthält `width * height` Pixel mit 32 Bit, wie in
    // `info` beschrieben (von `Window::present` geprüft).
    let lines = unsafe {
        StretchDIBits(
            hdc,
            0,
            0,
            w,
            h,
            0,
            0,
            w,
            h,
            pixels.as_ptr().cast::<c_void>(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
    lines != 0
}

fn key_down(key: c_int) -> bool {
    // SAFETY: GetKeyState hat keine Vorbedingungen.
    unsafe { GetKeyState(key) < 0 }
}

fn current_modifiers() -> Modifiers {
    Modifiers {
        shift: key_down(VK_SHIFT),
        control: key_down(VK_CONTROL),
        alt: key_down(VK_MENU),
        meta: key_down(VK_LWIN) || key_down(VK_RWIN),
    }
}

fn caps_lock_on() -> bool {
    // SAFETY: GetKeyState hat keine Vorbedingungen.
    unsafe { GetKeyState(VK_CAPITAL) & 1 != 0 }
}

/// Die Fensterprozedur. Panics werden aufgefangen und nach der Rückkehr aus
/// `DispatchMessageW` weitergereicht (RS-5).
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Windows ruft die Prozedur mit einem gültigen Fenster dieser
        // Klasse auf.
        unsafe { handle_message(hwnd, message, wparam, lparam) }
    }));
    match result {
        Ok(result) => result,
        Err(payload) => {
            PANIC.with(|slot| *slot.borrow_mut() = Some(payload));
            // SAFETY: Wie oben.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

/// # Safety
///
/// `hwnd` muss ein gültiges Fenster dieser Klasse sein, `lparam` muss zur
/// Nachricht passen.
unsafe fn handle_message(hwnd: HWND, message: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = lparam as *const CREATESTRUCTW;
        // SAFETY: Bei WM_NCCREATE zeigt lparam auf ein CREATESTRUCTW, dessen
        // lpCreateParams der Zustand aus `Window::open` ist.
        unsafe {
            set_window_long_ptr(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
    }
    // SAFETY: Gültiges Fenster (vom Aufrufer zugesichert).
    let pointer = unsafe { get_window_long_ptr(hwnd, GWLP_USERDATA) } as *const RefCell<State>;
    if pointer.is_null() {
        // SAFETY: Wie oben.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    // SAFETY: Der Zeiger stammt aus `Window::open` und bleibt gültig, bis
    // `Drop` ihn vor dem Zerstören des Fensters entfernt.
    let state = unsafe { &*pointer };
    let push = |event: Event| state.borrow_mut().events.push_back(event);
    let position = || {
        let x = f64::from(loword(lparam as usize) as i16);
        let y = f64::from(hiword(lparam as usize) as i16);
        (x, y)
    };
    match message {
        WM_CLOSE => {
            push(Event::CloseRequested);
            0
        }
        WM_SIZE => {
            if wparam != SIZE_MINIMIZED {
                let width = u32::from(loword(lparam as usize));
                let height = u32::from(hiword(lparam as usize));
                let mut state = state.borrow_mut();
                if (width, height) != (state.width, state.height) {
                    state.width = width;
                    state.height = height;
                    state.events.push_back(Event::Resized { width, height });
                }
            }
            0
        }
        WM_PAINT => {
            // SAFETY: PAINTSTRUCT besteht nur aus Zahlen und Zeigern, Nullen
            // sind gültig.
            let mut paint: PAINTSTRUCT = unsafe { std::mem::zeroed() };
            // SAFETY: Gültiges Fenster; EndPaint folgt unten.
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            let has_image = {
                let state = state.borrow();
                if !hdc.is_null() && state.image_width > 0 {
                    blit(hdc, &state.image, state.image_width, state.image_height);
                }
                state.image_width > 0
            };
            // SAFETY: Gehört zu BeginPaint oben.
            unsafe { EndPaint(hwnd, &paint) };
            if !has_image {
                push(Event::RedrawRequested);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_SETFOCUS | WM_KILLFOCUS => {
            push(Event::Focused(message == WM_SETFOCUS));
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let pressed = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
            let vk = u32::try_from(wparam & 0xffff).unwrap_or(0);
            // SAFETY: MapVirtualKeyW hat keine Vorbedingungen.
            let mapped = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR) };
            // Das oberste Bit kennzeichnet Tottasten.
            let character = if mapped & 0x8000_0000 == 0 {
                char::from_u32(mapped & 0xffff).filter(|&c| c != '\0')
            } else {
                None
            };
            let modifiers = current_modifiers();
            push(Event::Key(KeyEvent {
                key: keys::key_from_virtual_key(vk, character, modifiers.shift != caps_lock_on()),
                pressed,
                repeat: pressed && (lparam >> 30) & 1 != 0,
                modifiers,
            }));
            if message == WM_SYSKEYDOWN || message == WM_SYSKEYUP {
                // Alt+F4 und das Systemmenü erledigt Windows.
                // SAFETY: Wie oben.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            } else {
                0
            }
        }
        WM_CHAR => {
            let unit = (wparam & 0xffff) as u16;
            let text = {
                let mut state = state.borrow_mut();
                match unit {
                    0xD800..=0xDBFF => {
                        state.high_surrogate = Some(unit);
                        None
                    }
                    0xDC00..=0xDFFF => state
                        .high_surrogate
                        .take()
                        .and_then(|high| char::decode_utf16([high, unit]).next())
                        .and_then(Result::ok),
                    _ => {
                        state.high_surrogate = None;
                        char::from_u32(u32::from(unit))
                    }
                }
            };
            if let Some(c) = text.filter(|c| !c.is_control()) {
                push(Event::Text(c.to_string()));
            }
            0
        }
        WM_MOUSEMOVE => {
            let start_tracking = !std::mem::replace(&mut state.borrow_mut().tracking_leave, true);
            if start_tracking {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: struct_size::<TRACKMOUSEEVENT>(),
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: Gültige Struktur für ein gültiges Fenster.
                unsafe { TrackMouseEvent(&mut track) };
            }
            let (x, y) = position();
            push(Event::PointerMoved { x, y });
            0
        }
        WM_MOUSELEAVE => {
            state.borrow_mut().tracking_leave = false;
            push(Event::PointerLeft);
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN
        | WM_MBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let (button, pressed) = match message {
                WM_LBUTTONDOWN => (PointerButton::Left, true),
                WM_LBUTTONUP => (PointerButton::Left, false),
                WM_RBUTTONDOWN => (PointerButton::Right, true),
                WM_RBUTTONUP => (PointerButton::Right, false),
                WM_MBUTTONDOWN => (PointerButton::Middle, true),
                WM_MBUTTONUP => (PointerButton::Middle, false),
                _ => {
                    let button = match hiword(wparam) {
                        1 => PointerButton::Back,
                        2 => PointerButton::Forward,
                        other => PointerButton::Other(other),
                    };
                    (button, message == WM_XBUTTONDOWN)
                }
            };
            // Damit das Loslassen auch außerhalb des Fensters ankommt.
            // SAFETY: Gültiges Fenster.
            unsafe {
                if pressed {
                    SetCapture(hwnd);
                } else {
                    ReleaseCapture();
                }
            }
            let (x, y) = position();
            push(Event::PointerButton {
                button,
                pressed,
                x,
                y,
            });
            // Für die X-Tasten verlangt die Dokumentation TRUE als Ergebnis.
            LRESULT::from(message == WM_XBUTTONDOWN || message == WM_XBUTTONUP)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = f64::from(hiword(wparam) as i16) / WHEEL_DELTA;
            // Windows zählt das Vorwärtsdrehen positiv, gemeint ist „nach oben“.
            let scroll = if message == WM_MOUSEWHEEL {
                ScrollDelta::Steps { x: 0.0, y: -delta }
            } else {
                ScrollDelta::Steps { x: delta, y: 0.0 }
            };
            push(Event::Scroll(scroll));
            0
        }
        WM_DPICHANGED => {
            let suggested = lparam as *const RECT;
            // SAFETY: Bei WM_DPICHANGED zeigt lparam auf das vorgeschlagene
            // Fensterrechteck.
            unsafe {
                let rect = *suggested;
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    rect.left,
                    rect.top,
                    rect.right.saturating_sub(rect.left),
                    rect.bottom.saturating_sub(rect.top),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            0
        }
        // SAFETY: Wie oben.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
