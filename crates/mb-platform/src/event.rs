//! Ereignisse eines Fensters, unabhängig vom System.
//!
//! Die Namen der Tasten folgen den Werten von `KeyboardEvent.key` aus der
//! UI-Events-Spezifikation des W3C, damit sie sich später ohne Umweg an
//! Webseiten weitergeben lassen.

/// Ein Ereignis eines Fensters.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Der Nutzer will das Fenster schließen. Es bleibt offen, bis das
    /// Programm das [`Window`](crate::Window) verwirft.
    CloseRequested,
    /// Die Zeichenfläche hat eine neue Größe in Gerätepixeln oder eine neue
    /// Skalierung ([`Window::scale`](crate::Window::scale)).
    Resized { width: u32, height: u32 },
    /// Der Inhalt muss neu gezeichnet werden, zum Beispiel nach dem Aufdecken.
    RedrawRequested,
    /// Das Fenster hat den Tastaturfokus bekommen (`true`) oder verloren.
    Focused(bool),
    /// Eine Taste wurde gedrückt oder losgelassen.
    Key(KeyEvent),
    /// Eingegebener Text nach Tastaturbelegung, ohne Steuerzeichen.
    Text(String),
    /// Der Mauszeiger wurde bewegt. Koordinaten in Gerätepixeln, gemessen von
    /// der linken oberen Ecke der Zeichenfläche.
    PointerMoved { x: f64, y: f64 },
    /// Eine Maustaste wurde gedrückt oder losgelassen.
    PointerButton {
        button: PointerButton,
        pressed: bool,
        x: f64,
        y: f64,
    },
    /// Der Mauszeiger hat die Zeichenfläche verlassen.
    PointerLeft,
    /// Mausrad oder Touchpad. Positive Werte scrollen nach unten bzw. rechts.
    Scroll(ScrollDelta),
}

/// Eine Taste mit Zustand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub pressed: bool,
    /// Wiederholung durch gedrückt gehaltene Taste.
    pub repeat: bool,
    pub modifiers: Modifiers,
}

/// Welche Taste es war, wie `KeyboardEvent.key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// Eine Taste, die ein Zeichen erzeugt, nach Tastaturbelegung.
    Character(char),
    Named(NamedKey),
    /// Unbekannte Taste.
    Unidentified,
}

/// Tasten ohne eigenes Zeichen. Die Namen entsprechen den Werten von
/// `KeyboardEvent.key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Tab,
    Backspace,
    Escape,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Shift,
    Control,
    Alt,
    AltGraph,
    Meta,
    CapsLock,
    NumLock,
    ScrollLock,
    ContextMenu,
    Pause,
    PrintScreen,
    /// Funktionstaste F1 bis F24.
    F(u8),
}

/// Gedrückte Umschalttasten.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    /// Windows-Taste, Super-Taste oder Befehlstaste (macOS).
    pub meta: bool,
}

/// Eine Maustaste.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
    Other(u16),
}

/// Wie weit gescrollt wurde.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    /// In Rasten eines Mausrads.
    Steps { x: f64, y: f64 },
    /// In Gerätepixeln, zum Beispiel von einem Touchpad.
    Pixels { x: f64, y: f64 },
}
