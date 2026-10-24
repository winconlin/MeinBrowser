//! Selbst geschriebene Deklarationen für Win32 und GDI.
//!
//! Typen und Werte stammen aus der Dokumentation von Microsoft (Win32 API,
//! `winuser.h`, `wingdi.h`). Funktionen werden mit `extern "system"`
//! deklariert, Zeichenketten sind UTF-16 (die `…W`-Varianten).

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use core::ffi::{c_int, c_long, c_short, c_uint, c_ulong, c_ushort, c_void};

pub type BOOL = c_int;
pub type UINT = c_uint;
pub type DWORD = c_ulong;
pub type LONG = c_long;
pub type WORD = c_ushort;
pub type SHORT = c_short;
pub type ATOM = WORD;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;
pub type HANDLE = *mut c_void;
pub type HWND = HANDLE;
pub type HINSTANCE = HANDLE;
pub type HICON = HANDLE;
pub type HCURSOR = HANDLE;
pub type HBRUSH = HANDLE;
pub type HMENU = HANDLE;
pub type HDC = HANDLE;
pub type HBITMAP = HANDLE;
pub type HGDIOBJ = HANDLE;
pub type HKL = HANDLE;
pub type WNDPROC = Option<unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT>;

pub const FALSE: BOOL = 0;

pub const CS_VREDRAW: UINT = 0x0001;
pub const CS_HREDRAW: UINT = 0x0002;
pub const WS_OVERLAPPEDWINDOW: DWORD = 0x00CF_0000;
pub const CW_USEDEFAULT: c_int = c_int::MIN;
pub const SW_SHOWNORMAL: c_int = 1;
pub const GWLP_USERDATA: c_int = -21;
pub const IDC_ARROW: usize = 32512;
pub const PM_REMOVE: UINT = 0x0001;
pub const QS_ALLINPUT: DWORD = 0x04FF;
pub const INFINITE: DWORD = 0xFFFF_FFFF;
pub const WAIT_FAILED: DWORD = 0xFFFF_FFFF;
pub const SIZE_MINIMIZED: WPARAM = 1;
pub const TME_LEAVE: DWORD = 0x0000_0002;
pub const SWP_NOZORDER: UINT = 0x0004;
pub const SWP_NOACTIVATE: UINT = 0x0010;
/// `ToUnicodeEx`: Bit 2 von `wFlags` lässt den Tastaturzustand unverändert
/// (ab Windows 10, Version 1607).
pub const TO_UNICODE_KEEP_STATE: UINT = 0x0004;
pub const DIB_RGB_COLORS: UINT = 0;
pub const BI_RGB: DWORD = 0;
pub const SRCCOPY: DWORD = 0x00CC_0020;
/// `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`, ein Pseudo-Handle mit dem Wert −4.
pub const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

pub const WM_SIZE: UINT = 0x0005;
pub const WM_SETFOCUS: UINT = 0x0007;
pub const WM_KILLFOCUS: UINT = 0x0008;
pub const WM_PAINT: UINT = 0x000F;
pub const WM_CLOSE: UINT = 0x0010;
pub const WM_ERASEBKGND: UINT = 0x0014;
pub const WM_NCCREATE: UINT = 0x0081;
pub const WM_KEYDOWN: UINT = 0x0100;
pub const WM_KEYUP: UINT = 0x0101;
pub const WM_CHAR: UINT = 0x0102;
pub const WM_SYSKEYDOWN: UINT = 0x0104;
pub const WM_SYSKEYUP: UINT = 0x0105;
pub const WM_MOUSEMOVE: UINT = 0x0200;
pub const WM_LBUTTONDOWN: UINT = 0x0201;
pub const WM_LBUTTONUP: UINT = 0x0202;
pub const WM_RBUTTONDOWN: UINT = 0x0204;
pub const WM_RBUTTONUP: UINT = 0x0205;
pub const WM_MBUTTONDOWN: UINT = 0x0207;
pub const WM_MBUTTONUP: UINT = 0x0208;
pub const WM_MOUSEWHEEL: UINT = 0x020A;
pub const WM_XBUTTONDOWN: UINT = 0x020B;
pub const WM_XBUTTONUP: UINT = 0x020C;
pub const WM_MOUSEHWHEEL: UINT = 0x020E;
pub const WM_MOUSELEAVE: UINT = 0x02A3;
pub const WM_DPICHANGED: UINT = 0x02E0;

pub const VK_SHIFT: c_int = 0x10;
pub const VK_CONTROL: c_int = 0x11;
pub const VK_MENU: c_int = 0x12;
pub const VK_LWIN: c_int = 0x5B;
pub const VK_RWIN: c_int = 0x5C;
pub const VK_LCONTROL: c_int = 0xA2;
pub const VK_RCONTROL: c_int = 0xA3;
pub const VK_LMENU: c_int = 0xA4;
pub const VK_RMENU: c_int = 0xA5;

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cbSize: UINT,
    pub style: UINT,
    pub lpfnWndProc: WNDPROC,
    pub cbClsExtra: c_int,
    pub cbWndExtra: c_int,
    pub hInstance: HINSTANCE,
    pub hIcon: HICON,
    pub hCursor: HCURSOR,
    pub hbrBackground: HBRUSH,
    pub lpszMenuName: *const u16,
    pub lpszClassName: *const u16,
    pub hIconSm: HICON,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct POINT {
    pub x: LONG,
    pub y: LONG,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RECT {
    pub left: LONG,
    pub top: LONG,
    pub right: LONG,
    pub bottom: LONG,
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: UINT,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: DWORD,
    pub pt: POINT,
}

#[repr(C)]
pub struct PAINTSTRUCT {
    pub hdc: HDC,
    pub fErase: BOOL,
    pub rcPaint: RECT,
    pub fRestore: BOOL,
    pub fIncUpdate: BOOL,
    pub rgbReserved: [u8; 32],
}

#[repr(C)]
pub struct CREATESTRUCTW {
    pub lpCreateParams: *mut c_void,
    pub hInstance: HINSTANCE,
    pub hMenu: HMENU,
    pub hwndParent: HWND,
    pub cy: c_int,
    pub cx: c_int,
    pub y: c_int,
    pub x: c_int,
    pub style: LONG,
    pub lpszName: *const u16,
    pub lpszClass: *const u16,
    pub dwExStyle: DWORD,
}

#[repr(C)]
pub struct TRACKMOUSEEVENT {
    pub cbSize: DWORD,
    pub dwFlags: DWORD,
    pub hwndTrack: HWND,
    pub dwHoverTime: DWORD,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BITMAPINFOHEADER {
    pub biSize: DWORD,
    pub biWidth: LONG,
    pub biHeight: LONG,
    pub biPlanes: WORD,
    pub biBitCount: WORD,
    pub biCompression: DWORD,
    pub biSizeImage: DWORD,
    pub biXPelsPerMeter: LONG,
    pub biYPelsPerMeter: LONG,
    pub biClrUsed: DWORD,
    pub biClrImportant: DWORD,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RGBQUAD {
    pub rgbBlue: u8,
    pub rgbGreen: u8,
    pub rgbRed: u8,
    pub rgbReserved: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BITMAPINFO {
    pub bmiHeader: BITMAPINFOHEADER,
    pub bmiColors: [RGBQUAD; 1],
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn GetModuleHandleW(lpModuleName: *const u16) -> HINSTANCE;
    pub fn GetLastError() -> DWORD;
}

#[link(name = "user32")]
unsafe extern "system" {
    pub fn RegisterClassExW(lpWndClass: *const WNDCLASSEXW) -> ATOM;
    pub fn CreateWindowExW(
        dwExStyle: DWORD,
        lpClassName: *const u16,
        lpWindowName: *const u16,
        dwStyle: DWORD,
        X: c_int,
        Y: c_int,
        nWidth: c_int,
        nHeight: c_int,
        hWndParent: HWND,
        hMenu: HMENU,
        hInstance: HINSTANCE,
        lpParam: *mut c_void,
    ) -> HWND;
    pub fn DestroyWindow(hWnd: HWND) -> BOOL;
    pub fn ShowWindow(hWnd: HWND, nCmdShow: c_int) -> BOOL;
    pub fn UpdateWindow(hWnd: HWND) -> BOOL;
    pub fn DefWindowProcW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    pub fn PeekMessageW(
        lpMsg: *mut MSG,
        hWnd: HWND,
        wMsgFilterMin: UINT,
        wMsgFilterMax: UINT,
        wRemoveMsg: UINT,
    ) -> BOOL;
    pub fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    pub fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;
    pub fn MsgWaitForMultipleObjects(
        nCount: DWORD,
        pHandles: *const HANDLE,
        fWaitAll: BOOL,
        dwMilliseconds: DWORD,
        dwWakeMask: DWORD,
    ) -> DWORD;
    pub fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
    pub fn AdjustWindowRectEx(
        lpRect: *mut RECT,
        dwStyle: DWORD,
        bMenu: BOOL,
        dwExStyle: DWORD,
    ) -> BOOL;
    pub fn SetWindowPos(
        hWnd: HWND,
        hWndInsertAfter: HWND,
        X: c_int,
        Y: c_int,
        cx: c_int,
        cy: c_int,
        uFlags: UINT,
    ) -> BOOL;
    pub fn GetDC(hWnd: HWND) -> HDC;
    pub fn ReleaseDC(hWnd: HWND, hDC: HDC) -> c_int;
    pub fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    pub fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> BOOL;
    pub fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
    pub fn SetCapture(hWnd: HWND) -> HWND;
    pub fn ReleaseCapture() -> BOOL;
    pub fn TrackMouseEvent(lpEventTrack: *mut TRACKMOUSEEVENT) -> BOOL;
    pub fn GetKeyState(nVirtKey: c_int) -> SHORT;
    pub fn GetKeyboardState(lpKeyState: *mut u8) -> BOOL;
    pub fn GetKeyboardLayout(idThread: DWORD) -> HKL;
    pub fn ToUnicodeEx(
        wVirtKey: UINT,
        wScanCode: UINT,
        lpKeyState: *const u8,
        pwszBuff: *mut u16,
        cchBuff: c_int,
        wFlags: UINT,
        dwhkl: HKL,
    ) -> c_int;
    pub fn SetProcessDpiAwarenessContext(value: HANDLE) -> BOOL;
    #[cfg(target_pointer_width = "64")]
    pub fn GetWindowLongPtrW(hWnd: HWND, nIndex: c_int) -> isize;
    #[cfg(target_pointer_width = "64")]
    pub fn SetWindowLongPtrW(hWnd: HWND, nIndex: c_int, dwNewLong: isize) -> isize;
    #[cfg(target_pointer_width = "32")]
    pub fn GetWindowLongW(hWnd: HWND, nIndex: c_int) -> LONG;
    #[cfg(target_pointer_width = "32")]
    pub fn SetWindowLongW(hWnd: HWND, nIndex: c_int, dwNewLong: LONG) -> LONG;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    pub fn StretchDIBits(
        hdc: HDC,
        xDest: c_int,
        yDest: c_int,
        DestWidth: c_int,
        DestHeight: c_int,
        xSrc: c_int,
        ySrc: c_int,
        SrcWidth: c_int,
        SrcHeight: c_int,
        lpBits: *const c_void,
        lpbmi: *const BITMAPINFO,
        iUsage: UINT,
        rop: DWORD,
    ) -> c_int;
    pub fn CreateCompatibleDC(hdc: HDC) -> HDC;
    pub fn DeleteDC(hdc: HDC) -> BOOL;
    pub fn CreateDIBSection(
        hdc: HDC,
        pbmi: *const BITMAPINFO,
        usage: UINT,
        ppvBits: *mut *mut c_void,
        hSection: HANDLE,
        offset: DWORD,
    ) -> HBITMAP;
    pub fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    pub fn DeleteObject(ho: HGDIOBJ) -> BOOL;
    pub fn BitBlt(
        hdc: HDC,
        x: c_int,
        y: c_int,
        cx: c_int,
        cy: c_int,
        hdcSrc: HDC,
        x1: c_int,
        y1: c_int,
        rop: DWORD,
    ) -> BOOL;
    pub fn GdiFlush() -> BOOL;
}

/// `GetWindowLongPtrW`, das es unter 32-Bit-Windows nur als Makro gibt.
///
/// # Safety
///
/// `hwnd` muss ein gültiges Fenster sein.
pub unsafe fn get_window_long_ptr(hwnd: HWND, index: c_int) -> isize {
    #[cfg(target_pointer_width = "64")]
    // SAFETY: Vom Aufrufer zugesichert.
    let value = unsafe { GetWindowLongPtrW(hwnd, index) };
    #[cfg(target_pointer_width = "32")]
    // SAFETY: Vom Aufrufer zugesichert.
    let value = unsafe { GetWindowLongW(hwnd, index) } as isize;
    value
}

/// `SetWindowLongPtrW`, das es unter 32-Bit-Windows nur als Makro gibt.
///
/// # Safety
///
/// `hwnd` muss ein gültiges Fenster sein.
pub unsafe fn set_window_long_ptr(hwnd: HWND, index: c_int, value: isize) -> isize {
    #[cfg(target_pointer_width = "64")]
    // SAFETY: Vom Aufrufer zugesichert.
    let old = unsafe { SetWindowLongPtrW(hwnd, index, value) };
    #[cfg(target_pointer_width = "32")]
    // SAFETY: Vom Aufrufer zugesichert. Unter 32 Bit passt jeder Zeiger in LONG.
    let old = unsafe { SetWindowLongW(hwnd, index, value as LONG) } as isize;
    old
}
