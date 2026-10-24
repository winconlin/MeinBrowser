//! Fenster unter macOS über AppKit, angesprochen über die
//! Objective-C-Laufzeit. Das Bild landet als `CGImage` im Inhalt einer
//! eigenen Core-Animation-Ebene (`CALayer`), die die Inhaltsansicht des
//! Fensters trägt.
//!
//! AppKit muss auf dem Hauptthread laufen. Die Ereignisschleife holt sich die
//! Ereignisse selbst mit `nextEventMatchingMask:…`, statt `[NSApp run]` zu
//! übergeben. Tasten wertet sie selbst aus; Eingabemethoden und Tottasten
//! (`NSTextInputClient`) folgen später.

mod ffi;
mod keys;

use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use core::ffi::c_void;

use crate::event::{Event, KeyEvent, Modifiers, PointerButton, ScrollDelta};
use crate::{Capture, Error, WindowOptions};

use ffi::*;

/// Was der Delegate eines Fensters mitteilt.
#[derive(Clone, Copy, Debug)]
enum Notice {
    Close,
    Resize,
    Focus(bool),
}

thread_local! {
    /// Mitteilungen der Delegates, nach Delegate-Objekt.
    static NOTICES: RefCell<Vec<(usize, Notice)>> = const { RefCell::new(Vec::new()) };
    /// Eine Panic in einer Methode, die AppKit aufgerufen hat. Sie wird nach
    /// der Rückkehr in Rust-Code weitergereicht (RS-5).
    static PANIC: RefCell<Option<Box<dyn Any + Send>>> = const { RefCell::new(None) };
}

fn resume_panic() {
    if let Some(payload) = PANIC.with(|slot| slot.borrow_mut().take()) {
        panic::resume_unwind(payload);
    }
}

fn notify(delegate: Id, notice: Notice) {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        NOTICES.with(|notices| notices.borrow_mut().push((delegate as usize, notice)));
    }));
    if let Err(payload) = result {
        PANIC.with(|slot| *slot.borrow_mut() = Some(payload));
    }
}

extern "C" fn window_should_close(this: Id, _selector: Sel, _sender: Id) -> BOOL {
    notify(this, Notice::Close);
    // Das Programm entscheidet selbst, wann das Fenster geht.
    NO
}

extern "C" fn window_did_resize(this: Id, _selector: Sel, _notification: Id) {
    notify(this, Notice::Resize);
}

extern "C" fn window_did_become_key(this: Id, _selector: Sel, _notification: Id) {
    notify(this, Notice::Focus(true));
}

extern "C" fn window_did_resign_key(this: Id, _selector: Sel, _notification: Id) {
    notify(this, Notice::Focus(false));
}

/// Die Delegate-Klasse `MBFensterDelegate`, einmal pro Prozess angelegt.
fn delegate_class() -> Result<Class, Error> {
    static CLASS: OnceLock<usize> = OnceLock::new();
    let class = *CLASS.get_or_init(|| {
        let name = c"MBFensterDelegate";
        // SAFETY: Die Klasse wird vollständig beschrieben, bevor sie
        // registriert wird. Die Methoden haben die angegebenen Signaturen.
        unsafe {
            let new_class = objc_allocateClassPair(ffi::class(c"NSObject"), name.as_ptr(), 0);
            if new_class.is_null() {
                // Gibt es schon, etwa aus einer zweiten Kopie dieser Bibliothek.
                return ffi::class(name) as usize;
            }
            let should_close: extern "C" fn(Id, Sel, Id) -> BOOL = window_should_close;
            let notification: [(&core::ffi::CStr, extern "C" fn(Id, Sel, Id)); 4] = [
                (c"windowDidResize:", window_did_resize),
                (c"windowDidChangeBackingProperties:", window_did_resize),
                (c"windowDidBecomeKey:", window_did_become_key),
                (c"windowDidResignKey:", window_did_resign_key),
            ];
            class_addMethod(
                new_class,
                selector(c"windowShouldClose:"),
                std::mem::transmute::<extern "C" fn(Id, Sel, Id) -> BOOL, unsafe extern "C" fn()>(
                    should_close,
                ),
                BOOL_ENCODING.as_ptr(),
            );
            for (name, function) in notification {
                class_addMethod(
                    new_class,
                    selector(name),
                    std::mem::transmute::<extern "C" fn(Id, Sel, Id), unsafe extern "C" fn()>(
                        function,
                    ),
                    c"v@:@".as_ptr(),
                );
            }
            objc_registerClassPair(new_class);
            new_class as usize
        }
    });
    if class == 0 {
        return Err(Error::System {
            function: "objc_allocateClassPair",
            code: 0,
        });
    }
    Ok(class as Class)
}

/// Ein Autorelease-Pool für die Dauer eines Blocks.
struct Pool(*mut c_void);

impl Pool {
    fn new() -> Pool {
        // SAFETY: Push und Pop gehören zusammen (siehe Drop).
        Pool(unsafe { objc_autoreleasePoolPush() })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // SAFETY: Der Pool stammt von objc_autoreleasePoolPush.
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}

pub struct Window {
    app: Id,
    window: Id,
    view: Id,
    layer: Id,
    delegate: Id,
    color_space: *mut c_void,
    width: u32,
    height: u32,
    scale: f64,
    pending: VecDeque<Event>,
}

impl Window {
    pub fn open(options: &WindowOptions<'_>) -> Result<Window, Error> {
        let _pool = Pool::new();
        // SAFETY: Alle Nachrichten gehen an AppKit-Klassen und -Objekte mit den
        // dokumentierten Signaturen. Objekte aus alloc/init gehören uns und
        // werden in Drop freigegeben.
        unsafe {
            let is_main: BOOL = send(class(c"NSThread"), c"isMainThread", ());
            if is_main != YES {
                return Err(Error::Unsupported(
                    "Fenster unter macOS nur auf dem Hauptthread".into(),
                ));
            }
            let delegate_class = delegate_class()?;
            let app: Id = send(class(c"NSApplication"), c"sharedApplication", ());
            static LAUNCHED: AtomicBool = AtomicBool::new(false);
            if !LAUNCHED.swap(true, Ordering::SeqCst) {
                let _: BOOL = send(
                    app,
                    c"setActivationPolicy:",
                    (NS_APPLICATION_ACTIVATION_POLICY_REGULAR,),
                );
                let _: () = send(app, c"finishLaunching", ());
            }

            // Die gewünschte Größe ist in Gerätepixeln, AppKit rechnet in Punkten.
            let screen: Id = send(class(c"NSScreen"), c"mainScreen", ());
            let screen_scale: CGFloat = if screen.is_null() {
                1.0
            } else {
                send(screen, c"backingScaleFactor", ())
            };
            let screen_scale = if screen_scale > 0.0 {
                screen_scale
            } else {
                1.0
            };
            let content = NSRect {
                origin: NSPoint { x: 0.0, y: 0.0 },
                size: NSSize {
                    width: f64::from(options.width) / screen_scale,
                    height: f64::from(options.height) / screen_scale,
                },
            };
            let style = NS_WINDOW_STYLE_TITLED
                | NS_WINDOW_STYLE_CLOSABLE
                | NS_WINDOW_STYLE_MINIATURIZABLE
                | NS_WINDOW_STYLE_RESIZABLE;
            let window: Id = send(class(c"NSWindow"), c"alloc", ());
            let window: Id = send(
                window,
                c"initWithContentRect:styleMask:backing:defer:",
                (content, style, NS_BACKING_STORE_BUFFERED, NO),
            );
            if window.is_null() {
                return Err(Error::System {
                    function: "NSWindow initWithContentRect",
                    code: 0,
                });
            }
            let _: () = send(window, c"setReleasedWhenClosed:", (NO,));
            let title = ns_string(options.title);
            let _: () = send(window, c"setTitle:", (title,));
            let _: () = send(title, c"release", ());

            let delegate: Id = send(delegate_class, c"alloc", ());
            let delegate: Id = send(delegate, c"init", ());
            let _: () = send(window, c"setDelegate:", (delegate,));
            let _: () = send(window, c"setAcceptsMouseMovedEvents:", (YES,));

            // Eigene Ebene: Eine Ansicht, die ihre Ebene selbst bekommt
            // („layer-hosting“), überlässt deren Inhalt ganz dem Programm.
            let view: Id = send(window, c"contentView", ());
            let layer: Id = send(class(c"CALayer"), c"alloc", ());
            let layer: Id = send(layer, c"init", ());
            let _: () = send(layer, c"setContentsGravity:", (kCAGravityTopLeft,));
            let _: () = send(view, c"setLayer:", (layer,));
            let _: () = send(view, c"setWantsLayer:", (YES,));

            let _: () = send(window, c"center", ());
            let _: () = send(
                window,
                c"makeKeyAndOrderFront:",
                (ptr::null_mut::<c_void>(),),
            );
            let _: () = send(app, c"activateIgnoringOtherApps:", (YES,));

            let color_space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            let mut this = Window {
                app,
                window,
                view,
                layer,
                delegate,
                color_space,
                width: 0,
                height: 0,
                scale: 1.0,
                pending: VecDeque::new(),
            };
            this.update_size();
            this.pending.push_back(Event::Resized {
                width: this.width,
                height: this.height,
            });
            resume_panic();
            Ok(this)
        }
    }

    /// Liest Größe und Skalierung neu. `true`, wenn sich etwas geändert hat.
    fn update_size(&mut self) -> bool {
        // SAFETY: Gültige Ansicht und gültiges Fenster; `bounds` gibt ein NSRect
        // zurück, `backingScaleFactor` ein CGFloat.
        let (bounds, scale) = unsafe {
            let bounds = send_rect(self.view, c"bounds");
            let scale: CGFloat = send(self.window, c"backingScaleFactor", ());
            (bounds, scale)
        };
        let scale = if scale > 0.0 { scale } else { 1.0 };
        // `as` sättigt hier bewusst: negative oder ungültige Werte werden 0.
        let width = (bounds.size.width * scale).round() as u32;
        let height = (bounds.size.height * scale).round() as u32;
        let changed = (width, height) != (self.width, self.height) || scale != self.scale;
        self.width = width;
        self.height = height;
        self.scale = scale;
        changed
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn present(&mut self, width: u32, height: u32, pixels: &[u32]) -> Result<(), Error> {
        let _pool = Pool::new();
        let byte_count = pixels.len().saturating_mul(4);
        let length = isize::try_from(byte_count)
            .map_err(|_| Error::InvalidArgument("Bild zu groß".into()))?;
        // SAFETY: CFDataCreate kopiert `length` Bytes aus dem Pixelpuffer, der
        // so lang ist. Alle Core-Graphics-Objekte werden wieder freigegeben;
        // die Ebene behält ihr eigenes Exemplar des Bildes.
        unsafe {
            let data = CFDataCreate(ptr::null(), pixels.as_ptr().cast::<u8>(), length);
            if data.is_null() {
                return Err(Error::System {
                    function: "CFDataCreate",
                    code: 0,
                });
            }
            let provider = CGDataProviderCreateWithCFData(data);
            CFRelease(data);
            if provider.is_null() {
                return Err(Error::System {
                    function: "CGDataProviderCreateWithCFData",
                    code: 0,
                });
            }
            let image = CGImageCreate(
                width as usize,
                height as usize,
                8,
                32,
                (width as usize).saturating_mul(4),
                self.color_space,
                PIXEL_FORMAT,
                provider,
                ptr::null(),
                false,
                kCGRenderingIntentDefault,
            );
            CGDataProviderRelease(provider);
            if image.is_null() {
                return Err(Error::System {
                    function: "CGImageCreate",
                    code: 0,
                });
            }
            // Ohne Überblendung: Core Animation würde den Wechsel sonst animieren.
            let transaction = class(c"CATransaction");
            let _: () = send(transaction, c"begin", ());
            let _: () = send(transaction, c"setDisableActions:", (YES,));
            let _: () = send(self.layer, c"setContentsScale:", (self.scale,));
            let _: () = send(self.layer, c"setContents:", (image,));
            let _: () = send(transaction, c"commit", ());
            CGImageRelease(image);
        }
        Ok(())
    }

    pub fn next_event(&mut self, timeout: Option<Duration>) -> Result<Option<Event>, Error> {
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        loop {
            self.drain_notices();
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
            let remaining =
                deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
            if remaining == Some(Duration::ZERO) {
                return Ok(None);
            }
            let pool = Pool::new();
            // SAFETY: Dokumentierte Signaturen; das Ereignis ist bis zum Ende
            // des Pools gültig.
            unsafe {
                let until: Id = match remaining {
                    None => send(class(c"NSDate"), c"distantFuture", ()),
                    Some(remaining) => send(
                        class(c"NSDate"),
                        c"dateWithTimeIntervalSinceNow:",
                        (remaining.as_secs_f64(),),
                    ),
                };
                let event: Id = send(
                    self.app,
                    c"nextEventMatchingMask:untilDate:inMode:dequeue:",
                    (NS_EVENT_MASK_ANY, until, NSDefaultRunLoopMode, YES),
                );
                if !event.is_null() && self.translate(event) {
                    let _: () = send(self.app, c"sendEvent:", (event,));
                }
            }
            drop(pool);
            resume_panic();
        }
    }

    pub fn capture(&mut self) -> Result<Capture, Error> {
        let _pool = Pool::new();
        let (width, height) = (self.width, self.height);
        let count = (width as usize).saturating_mul(height as usize);
        // SAFETY: Der Bitmap-Kontext besitzt seinen Puffer mit
        // `width * height` Pixeln zu 32 Bit, bis er freigegeben wird.
        unsafe {
            let context = CGBitmapContextCreate(
                ptr::null_mut(),
                width as usize,
                height as usize,
                8,
                (width as usize).saturating_mul(4),
                self.color_space,
                PIXEL_FORMAT,
            );
            if context.is_null() {
                return Err(Error::System {
                    function: "CGBitmapContextCreate",
                    code: 0,
                });
            }
            CGContextScaleCTM(context, self.scale, self.scale);
            let _: () = send(self.layer, c"renderInContext:", (context,));
            let data = CGBitmapContextGetData(context);
            let result = if data.is_null() {
                Err(Error::System {
                    function: "CGBitmapContextGetData",
                    code: 0,
                })
            } else {
                let pixels = std::slice::from_raw_parts(data.cast::<u32>(), count);
                Ok(Capture {
                    width,
                    height,
                    pixels: pixels.iter().map(|pixel| pixel & 0x00ff_ffff).collect(),
                })
            };
            CGContextRelease(context);
            result
        }
    }

    fn drain_notices(&mut self) {
        let key = self.delegate as usize;
        let mine: Vec<Notice> = NOTICES.with(|notices| {
            let mut notices = notices.borrow_mut();
            let mut mine = Vec::new();
            notices.retain(|&(delegate, notice)| {
                if delegate == key {
                    mine.push(notice);
                    false
                } else {
                    true
                }
            });
            mine
        });
        for notice in mine {
            match notice {
                Notice::Close => self.pending.push_back(Event::CloseRequested),
                Notice::Focus(focused) => self.pending.push_back(Event::Focused(focused)),
                Notice::Resize => {
                    if self.update_size() {
                        self.pending.push_back(Event::Resized {
                            width: self.width,
                            height: self.height,
                        });
                    }
                }
            }
        }
    }

    /// Wertet ein Ereignis aus. Das Ergebnis sagt, ob es danach an AppKit
    /// weitergeht (für Fensterrahmen, Menüs und Ähnliches).
    ///
    /// # Safety
    ///
    /// `event` muss ein gültiges `NSEvent` sein.
    unsafe fn translate(&mut self, event: Id) -> bool {
        // SAFETY: Dokumentierte Methoden von NSEvent mit passenden Typen.
        unsafe {
            let kind: NSUInteger = send(event, c"type", ());
            let window: Id = send(event, c"window", ());
            if window != self.window {
                return true;
            }
            let flags: NSUInteger = send(event, c"modifierFlags", ());
            let modifiers = Modifiers {
                shift: flags & NS_EVENT_MODIFIER_SHIFT != 0,
                control: flags & NS_EVENT_MODIFIER_CONTROL != 0,
                alt: flags & NS_EVENT_MODIFIER_OPTION != 0,
                meta: flags & NS_EVENT_MODIFIER_COMMAND != 0,
            };
            match kind {
                NS_EVENT_KEY_DOWN | NS_EVENT_KEY_UP => {
                    let pressed = kind == NS_EVENT_KEY_DOWN;
                    let keycode: u16 = send(event, c"keyCode", ());
                    let repeat: BOOL = send(event, c"isARepeat", ());
                    let ignoring = rust_string(send(event, c"charactersIgnoringModifiers", ()));
                    self.pending.push_back(Event::Key(KeyEvent {
                        key: keys::key_from_keycode(keycode, ignoring.chars().next()),
                        pressed,
                        repeat: pressed && repeat == YES,
                        modifiers,
                    }));
                    if pressed && !modifiers.meta && !modifiers.control {
                        let text: String = rust_string(send(event, c"characters", ()))
                            .chars()
                            .filter(|&c| keys::is_text(c))
                            .collect();
                        if !text.is_empty() {
                            self.pending.push_back(Event::Text(text));
                        }
                    }
                    // Tastenkürzel mit der Befehlstaste gehören AppKit, alles
                    // andere würde ohne Empfänger nur einen Warnton auslösen.
                    modifiers.meta
                }
                NS_EVENT_FLAGS_CHANGED => {
                    let keycode: u16 = send(event, c"keyCode", ());
                    let flag = match keycode {
                        0x38 | 0x3C => NS_EVENT_MODIFIER_SHIFT,
                        0x3B | 0x3E => NS_EVENT_MODIFIER_CONTROL,
                        0x3A | 0x3D => NS_EVENT_MODIFIER_OPTION,
                        0x36 | 0x37 => NS_EVENT_MODIFIER_COMMAND,
                        0x39 => NS_EVENT_MODIFIER_CAPS_LOCK,
                        _ => 0,
                    };
                    if flag != 0 {
                        self.pending.push_back(Event::Key(KeyEvent {
                            key: keys::key_from_keycode(keycode, None),
                            pressed: flags & flag != 0,
                            repeat: false,
                            modifiers,
                        }));
                    }
                    true
                }
                NS_EVENT_LEFT_MOUSE_DOWN
                | NS_EVENT_LEFT_MOUSE_UP
                | NS_EVENT_RIGHT_MOUSE_DOWN
                | NS_EVENT_RIGHT_MOUSE_UP
                | NS_EVENT_OTHER_MOUSE_DOWN
                | NS_EVENT_OTHER_MOUSE_UP => {
                    let (x, y) = self.position(event);
                    let pressed = matches!(
                        kind,
                        NS_EVENT_LEFT_MOUSE_DOWN
                            | NS_EVENT_RIGHT_MOUSE_DOWN
                            | NS_EVENT_OTHER_MOUSE_DOWN
                    );
                    let button = match kind {
                        NS_EVENT_LEFT_MOUSE_DOWN | NS_EVENT_LEFT_MOUSE_UP => PointerButton::Left,
                        NS_EVENT_RIGHT_MOUSE_DOWN | NS_EVENT_RIGHT_MOUSE_UP => PointerButton::Right,
                        _ => {
                            let number: NSInteger = send(event, c"buttonNumber", ());
                            match number {
                                2 => PointerButton::Middle,
                                3 => PointerButton::Back,
                                4 => PointerButton::Forward,
                                other => {
                                    PointerButton::Other(u16::try_from(other).unwrap_or(u16::MAX))
                                }
                            }
                        }
                    };
                    // Klicks in die Titelleiste gehören dem Fensterrahmen.
                    if y >= 0.0 {
                        self.pending.push_back(Event::PointerButton {
                            button,
                            pressed,
                            x,
                            y,
                        });
                    }
                    true
                }
                NS_EVENT_MOUSE_MOVED
                | NS_EVENT_LEFT_MOUSE_DRAGGED
                | NS_EVENT_RIGHT_MOUSE_DRAGGED
                | NS_EVENT_OTHER_MOUSE_DRAGGED => {
                    let (x, y) = self.position(event);
                    self.pending.push_back(Event::PointerMoved { x, y });
                    true
                }
                NS_EVENT_SCROLL_WHEEL => {
                    let dx: CGFloat = send(event, c"scrollingDeltaX", ());
                    let dy: CGFloat = send(event, c"scrollingDeltaY", ());
                    let precise: BOOL = send(event, c"hasPreciseScrollingDeltas", ());
                    // AppKit zählt „Inhalt nach unten“ positiv, also umgekehrt.
                    let delta = if precise == YES {
                        ScrollDelta::Pixels {
                            x: -dx * self.scale,
                            y: -dy * self.scale,
                        }
                    } else {
                        ScrollDelta::Steps { x: -dx, y: -dy }
                    };
                    self.pending.push_back(Event::Scroll(delta));
                    true
                }
                _ => true,
            }
        }
    }

    /// Position eines Mausereignisses in Gerätepixeln, von oben links.
    ///
    /// # Safety
    ///
    /// `event` muss ein gültiges Mausereignis dieses Fensters sein.
    unsafe fn position(&self, event: Id) -> (f64, f64) {
        // SAFETY: Vom Aufrufer zugesichert; locationInWindow gibt ein NSPoint
        // zurück, bounds ein NSRect.
        let (location, bounds) = unsafe {
            let location: NSPoint = send(event, c"locationInWindow", ());
            (location, send_rect(self.view, c"bounds"))
        };
        let x = location.x * self.scale;
        let y = (bounds.size.height - location.y) * self.scale;
        (x, y)
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let _pool = Pool::new();
        // SAFETY: Fenster, Delegate und Ebene gehören uns (aus alloc/init).
        unsafe {
            let _: () = send(self.window, c"setDelegate:", (ptr::null_mut::<c_void>(),));
            let _: () = send(self.window, c"orderOut:", (ptr::null_mut::<c_void>(),));
            let _: () = send(self.window, c"close", ());
            let _: () = send(self.window, c"release", ());
            let _: () = send(self.delegate, c"release", ());
            let _: () = send(self.layer, c"release", ());
            if !self.color_space.is_null() {
                CGColorSpaceRelease(self.color_space);
            }
        }
        let key = self.delegate as usize;
        NOTICES.with(|notices| {
            notices
                .borrow_mut()
                .retain(|&(delegate, _)| delegate != key)
        });
    }
}
