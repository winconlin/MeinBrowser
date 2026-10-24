//! Virtuelle Tastencodes von macOS (`kVK_…` aus `HIToolbox/Events.h`) als
//! Tasten.

#![forbid(unsafe_code)]

use crate::event::{Key, NamedKey};

/// Die Taste zu einem Tastencode. `character` ist das erste Zeichen von
/// `charactersIgnoringModifiers`.
pub fn key_from_keycode(keycode: u16, character: Option<char>) -> Key {
    let named = match keycode {
        0x24 | 0x4C => NamedKey::Enter,
        0x30 => NamedKey::Tab,
        0x31 => return Key::Character(' '),
        0x33 => NamedKey::Backspace,
        0x35 => NamedKey::Escape,
        0x36 | 0x37 => NamedKey::Meta,
        0x38 | 0x3C => NamedKey::Shift,
        0x39 => NamedKey::CapsLock,
        0x3A | 0x3D => NamedKey::Alt,
        0x3B | 0x3E => NamedKey::Control,
        0x6E => NamedKey::ContextMenu,
        0x72 => NamedKey::Insert,
        0x73 => NamedKey::Home,
        0x74 => NamedKey::PageUp,
        0x75 => NamedKey::Delete,
        0x77 => NamedKey::End,
        0x79 => NamedKey::PageDown,
        0x7B => NamedKey::ArrowLeft,
        0x7C => NamedKey::ArrowRight,
        0x7D => NamedKey::ArrowDown,
        0x7E => NamedKey::ArrowUp,
        0x7A => NamedKey::F(1),
        0x78 => NamedKey::F(2),
        0x63 => NamedKey::F(3),
        0x76 => NamedKey::F(4),
        0x60 => NamedKey::F(5),
        0x61 => NamedKey::F(6),
        0x62 => NamedKey::F(7),
        0x64 => NamedKey::F(8),
        0x65 => NamedKey::F(9),
        0x6D => NamedKey::F(10),
        0x67 => NamedKey::F(11),
        0x6F => NamedKey::F(12),
        0x69 => NamedKey::F(13),
        0x6B => NamedKey::F(14),
        0x71 => NamedKey::F(15),
        0x6A => NamedKey::F(16),
        0x40 => NamedKey::F(17),
        0x4F => NamedKey::F(18),
        0x50 => NamedKey::F(19),
        0x5A => NamedKey::F(20),
        _ => {
            return match character.filter(|&c| is_text(c)) {
                Some(c) => Key::Character(c),
                None => Key::Unidentified,
            };
        }
    };
    Key::Named(named)
}

/// Ob ein Zeichen aus `NSEvent.characters` echter Text ist. Funktionstasten
/// liefern Zeichen aus dem privaten Bereich U+F700 bis U+F8FF.
pub fn is_text(c: char) -> bool {
    !c.is_control() && !('\u{F700}'..='\u{F8FF}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_and_character_keys() {
        assert_eq!(
            key_from_keycode(0x24, Some('\r')),
            Key::Named(NamedKey::Enter)
        );
        assert_eq!(
            key_from_keycode(0x7A, Some('\u{F704}')),
            Key::Named(NamedKey::F(1))
        );
        assert_eq!(key_from_keycode(0x00, Some('a')), Key::Character('a'));
        assert_eq!(key_from_keycode(0x29, Some('ö')), Key::Character('ö'));
        assert_eq!(key_from_keycode(0x47, Some('\u{F739}')), Key::Unidentified);
    }
}
