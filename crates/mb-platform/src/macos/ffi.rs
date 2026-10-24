//! Selbst geschriebene Deklarationen für die Objective-C-Laufzeit, Core
//! Foundation, Core Graphics und die Frameworks AppKit und QuartzCore.
//!
//! Nachrichten an Objective-C-Objekte gehen über `objc_msgSend`, das vor jedem
//! Aufruf auf die genaue Signatur der Methode umgedeutet werden muss. Unter
//! AArch64 ist das zwingend, weil variadische Funktionen dort eine andere
//! Aufrufkonvention haben.

#![allow(non_upper_case_globals, non_snake_case, clippy::upper_case_acronyms)]

use core::ffi::{CStr, c_char, c_void};

pub type Id = *mut c_void;
pub type Sel = *const c_void;
pub type Class = *mut c_void;
pub type NSUInteger = usize;
pub type NSInteger = isize;
pub type CGFloat = f64;

/// `BOOL` ist unter AArch64 ein C-`bool`, unter x86_64 ein `signed char`.
#[cfg(target_arch = "aarch64")]
pub type BOOL = bool;
#[cfg(not(target_arch = "aarch64"))]
pub type BOOL = i8;

#[cfg(target_arch = "aarch64")]
pub const YES: BOOL = true;
#[cfg(target_arch = "aarch64")]
pub const NO: BOOL = false;
#[cfg(not(target_arch = "aarch64"))]
pub const YES: BOOL = 1;
#[cfg(not(target_arch = "aarch64"))]
pub const NO: BOOL = 0;

/// Typkodierung von `BOOL` für `class_addMethod`.
#[cfg(target_arch = "aarch64")]
pub const BOOL_ENCODING: &CStr = c"B@:@";
#[cfg(not(target_arch = "aarch64"))]
pub const BOOL_ENCODING: &CStr = c"c@:@";

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NSPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NSSize {
    pub width: CGFloat,
    pub height: CGFloat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}

pub const NS_WINDOW_STYLE_TITLED: NSUInteger = 1 << 0;
pub const NS_WINDOW_STYLE_CLOSABLE: NSUInteger = 1 << 1;
pub const NS_WINDOW_STYLE_MINIATURIZABLE: NSUInteger = 1 << 2;
pub const NS_WINDOW_STYLE_RESIZABLE: NSUInteger = 1 << 3;
pub const NS_BACKING_STORE_BUFFERED: NSUInteger = 2;
pub const NS_APPLICATION_ACTIVATION_POLICY_REGULAR: NSInteger = 0;
pub const NS_UTF8_STRING_ENCODING: NSUInteger = 4;
pub const NS_EVENT_MASK_ANY: NSUInteger = NSUInteger::MAX;

pub const NS_EVENT_LEFT_MOUSE_DOWN: NSUInteger = 1;
pub const NS_EVENT_LEFT_MOUSE_UP: NSUInteger = 2;
pub const NS_EVENT_RIGHT_MOUSE_DOWN: NSUInteger = 3;
pub const NS_EVENT_RIGHT_MOUSE_UP: NSUInteger = 4;
pub const NS_EVENT_MOUSE_MOVED: NSUInteger = 5;
pub const NS_EVENT_LEFT_MOUSE_DRAGGED: NSUInteger = 6;
pub const NS_EVENT_RIGHT_MOUSE_DRAGGED: NSUInteger = 7;
pub const NS_EVENT_KEY_DOWN: NSUInteger = 10;
pub const NS_EVENT_KEY_UP: NSUInteger = 11;
pub const NS_EVENT_FLAGS_CHANGED: NSUInteger = 12;
pub const NS_EVENT_SCROLL_WHEEL: NSUInteger = 22;
pub const NS_EVENT_OTHER_MOUSE_DOWN: NSUInteger = 25;
pub const NS_EVENT_OTHER_MOUSE_UP: NSUInteger = 26;
pub const NS_EVENT_OTHER_MOUSE_DRAGGED: NSUInteger = 27;

pub const NS_EVENT_MODIFIER_CAPS_LOCK: NSUInteger = 1 << 16;
pub const NS_EVENT_MODIFIER_SHIFT: NSUInteger = 1 << 17;
pub const NS_EVENT_MODIFIER_CONTROL: NSUInteger = 1 << 18;
pub const NS_EVENT_MODIFIER_OPTION: NSUInteger = 1 << 19;
pub const NS_EVENT_MODIFIER_COMMAND: NSUInteger = 1 << 20;

pub const kCGImageAlphaNoneSkipFirst: u32 = 6;
pub const kCGBitmapByteOrder32Little: u32 = 2 << 12;
pub const kCGBitmapByteOrder32Big: u32 = 4 << 12;
pub const kCGRenderingIntentDefault: i32 = 0;

#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    pub fn objc_getClass(name: *const c_char) -> Class;
    pub fn sel_registerName(name: *const c_char) -> Sel;
    /// Wird vor dem Aufruf auf die Signatur der Methode umgedeutet.
    pub fn objc_msgSend();
    /// Unter x86_64 für Methoden, die große Strukturen zurückgeben.
    #[cfg(target_arch = "x86_64")]
    pub fn objc_msgSend_stret();
    pub fn objc_allocateClassPair(
        superclass: Class,
        name: *const c_char,
        extra_bytes: usize,
    ) -> Class;
    pub fn objc_registerClassPair(class: Class);
    pub fn class_addMethod(
        class: Class,
        name: Sel,
        imp: unsafe extern "C" fn(),
        types: *const c_char,
    ) -> BOOL;
    pub fn objc_autoreleasePoolPush() -> *mut c_void;
    pub fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    pub static kCAGravityTopLeft: Id;
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    pub static NSDefaultRunLoopMode: Id;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: isize)
    -> *const c_void;
    pub fn CFRelease(object: *const c_void);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    pub static kCGColorSpaceSRGB: *const c_void;
    pub fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
    pub fn CGColorSpaceRelease(space: *mut c_void);
    pub fn CGDataProviderCreateWithCFData(data: *const c_void) -> *mut c_void;
    pub fn CGDataProviderRelease(provider: *mut c_void);
    pub fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
        provider: *mut c_void,
        decode: *const CGFloat,
        should_interpolate: bool,
        intent: i32,
    ) -> *mut c_void;
    pub fn CGImageRelease(image: *mut c_void);
    pub fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    pub fn CGBitmapContextGetData(context: *mut c_void) -> *mut c_void;
    pub fn CGContextRelease(context: *mut c_void);
    pub fn CGContextScaleCTM(context: *mut c_void, sx: CGFloat, sy: CGFloat);
}

/// Pixel `0x00RRGGBB` in eigener Bytereihenfolge, für Core Graphics.
pub const PIXEL_FORMAT: u32 = kCGImageAlphaNoneSkipFirst
    | if cfg!(target_endian = "little") {
        kCGBitmapByteOrder32Little
    } else {
        kCGBitmapByteOrder32Big
    };

pub fn class(name: &CStr) -> Class {
    // SAFETY: `name` ist nullterminiert. Unbekannte Klassen ergeben Null.
    unsafe { objc_getClass(name.as_ptr()) }
}

pub fn selector(name: &CStr) -> Sel {
    // SAFETY: `name` ist nullterminiert. Selektoren werden nie freigegeben.
    unsafe { sel_registerName(name.as_ptr()) }
}

/// Argumente einer Objective-C-Nachricht als Tupel.
pub trait Arguments {
    /// # Safety
    ///
    /// Die Methode muss genau diese Argument- und Rückgabetypen haben.
    unsafe fn send<R>(self, receiver: Id, selector: Sel) -> R;
}

macro_rules! arguments {
    ($($name:ident: $ty:ident),*) => {
        impl<$($ty),*> Arguments for ($($ty,)*) {
            unsafe fn send<R>(self, receiver: Id, selector: Sel) -> R {
                let ($($name,)*) = self;
                // SAFETY: Der Aufrufer sichert zu, dass die Signatur passt.
                // Funktionszeiger sind unabhängig von der Signatur gleich groß.
                unsafe {
                    let function: unsafe extern "C" fn(Id, Sel $(, $ty)*) -> R =
                        std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
                    function(receiver, selector $(, $name)*)
                }
            }
        }
    };
}

arguments!();
arguments!(a: A);
arguments!(a: A, b: B);
arguments!(a: A, b: B, c: C);
arguments!(a: A, b: B, c: C, d: D);

/// Sendet eine Nachricht.
///
/// # Safety
///
/// `receiver` muss ein gültiges Objekt oder Null sein, und die Methode muss
/// genau die Typen von `arguments` und `R` haben.
pub unsafe fn send<A: Arguments, R>(receiver: Id, name: &CStr, arguments: A) -> R {
    // SAFETY: Vom Aufrufer zugesichert.
    unsafe { arguments.send(receiver, selector(name)) }
}

/// Sendet eine Nachricht ohne Argumente, die ein `NSRect` zurückgibt. Unter
/// x86_64 gehen so große Strukturen über `objc_msgSend_stret`.
///
/// # Safety
///
/// Wie bei [`send`].
pub unsafe fn send_rect(receiver: Id, name: &CStr) -> NSRect {
    #[cfg(target_arch = "x86_64")]
    let entry = objc_msgSend_stret as unsafe extern "C" fn();
    #[cfg(not(target_arch = "x86_64"))]
    let entry = objc_msgSend as unsafe extern "C" fn();
    // SAFETY: Vom Aufrufer zugesichert; siehe oben zur Wahl des Einsprungs.
    unsafe {
        let function: unsafe extern "C" fn(Id, Sel) -> NSRect = std::mem::transmute(entry);
        function(receiver, selector(name))
    }
}

/// Ein `NSString` mit Besitz (+1), freizugeben mit `release`.
///
/// # Safety
///
/// Nur auf dem Hauptthread aufrufen.
pub unsafe fn ns_string(text: &str) -> Id {
    // SAFETY: Bytes und Länge stammen aus einem gültigen &str in UTF-8.
    unsafe {
        let string: Id = send(class(c"NSString"), c"alloc", ());
        send(
            string,
            c"initWithBytes:length:encoding:",
            (
                text.as_ptr().cast::<c_void>(),
                text.len() as NSUInteger,
                NS_UTF8_STRING_ENCODING,
            ),
        )
    }
}

/// Liest einen `NSString` als Rust-Zeichenkette.
///
/// # Safety
///
/// `string` muss ein gültiger `NSString` oder Null sein, und ein
/// Autorelease-Pool muss bestehen.
pub unsafe fn rust_string(string: Id) -> String {
    if string.is_null() {
        return String::new();
    }
    // SAFETY: UTF8String liefert einen nullterminierten Puffer, der bis zum
    // Leeren des Autorelease-Pools gültig ist.
    unsafe {
        let bytes: *const c_char = send(string, c"UTF8String", ());
        if bytes.is_null() {
            return String::new();
        }
        CStr::from_ptr(bytes).to_string_lossy().into_owned()
    }
}
