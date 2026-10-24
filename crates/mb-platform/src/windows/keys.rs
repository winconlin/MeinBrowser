//! Virtuelle Tastencodes von Windows als Tasten.

#![forbid(unsafe_code)]

use crate::event::{Key, NamedKey};

/// Die Taste zu einem virtuellen Tastencode. `character` ist das Zeichen,
/// das die Taste mit der aktuellen Belegung und den gedrückten Umschalttasten
/// erzeugt (aus `ToUnicodeEx`).
pub fn key_from_virtual_key(vk: u32, character: Option<char>) -> Key {
    let named = match vk {
        0x08 => NamedKey::Backspace,
        0x09 => NamedKey::Tab,
        0x0D => NamedKey::Enter,
        0x10 | 0xA0 | 0xA1 => NamedKey::Shift,
        0x11 | 0xA2 | 0xA3 => NamedKey::Control,
        0x12 | 0xA4 | 0xA5 => NamedKey::Alt,
        0x13 => NamedKey::Pause,
        0x14 => NamedKey::CapsLock,
        0x1B => NamedKey::Escape,
        0x21 => NamedKey::PageUp,
        0x22 => NamedKey::PageDown,
        0x23 => NamedKey::End,
        0x24 => NamedKey::Home,
        0x25 => NamedKey::ArrowLeft,
        0x26 => NamedKey::ArrowUp,
        0x27 => NamedKey::ArrowRight,
        0x28 => NamedKey::ArrowDown,
        0x2C => NamedKey::PrintScreen,
        0x2D => NamedKey::Insert,
        0x2E => NamedKey::Delete,
        0x5B | 0x5C => NamedKey::Meta,
        0x5D => NamedKey::ContextMenu,
        0x70..=0x87 => NamedKey::F((vk - 0x70 + 1) as u8),
        0x90 => NamedKey::NumLock,
        0x91 => NamedKey::ScrollLock,
        0x20 => return Key::Character(' '),
        // Ziffernblock
        0x60..=0x69 => {
            return char::from_digit(vk - 0x60, 10).map_or(Key::Unidentified, Key::Character);
        }
        0x6A => return Key::Character('*'),
        0x6B => return Key::Character('+'),
        0x6C => return Key::Character(','),
        0x6D => return Key::Character('-'),
        0x6E => return Key::Character('.'),
        0x6F => return Key::Character('/'),
        _ => {
            return character
                .filter(|c| !c.is_control())
                .map_or(Key::Unidentified, Key::Character);
        }
    };
    Key::Named(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys() {
        assert_eq!(
            key_from_virtual_key(0x0D, None),
            Key::Named(NamedKey::Enter)
        );
        assert_eq!(key_from_virtual_key(0x70, None), Key::Named(NamedKey::F(1)));
        assert_eq!(
            key_from_virtual_key(0x87, None),
            Key::Named(NamedKey::F(24))
        );
        assert_eq!(key_from_virtual_key(0x5B, None), Key::Named(NamedKey::Meta));
    }

    #[test]
    fn characters_come_from_the_layout() {
        assert_eq!(key_from_virtual_key(0x41, Some('a')), Key::Character('a'));
        assert_eq!(key_from_virtual_key(0x41, Some('A')), Key::Character('A'));
        assert_eq!(key_from_virtual_key(0xDE, Some('ä')), Key::Character('ä'));
        // Umschalttaste und 1 auf einer US-Belegung.
        assert_eq!(key_from_virtual_key(0x31, Some('!')), Key::Character('!'));
        assert_eq!(key_from_virtual_key(0x63, None), Key::Character('3'));
        assert_eq!(key_from_virtual_key(0xFF, None), Key::Unidentified);
    }

    #[test]
    fn named_keys_win_over_control_characters() {
        assert_eq!(
            key_from_virtual_key(0x0D, Some('\r')),
            Key::Named(NamedKey::Enter)
        );
        assert_eq!(key_from_virtual_key(0x41, Some('\u{1}')), Key::Unidentified);
    }
}
