//! Virtuelle Tastencodes von Windows als Tasten.

#![forbid(unsafe_code)]

use crate::event::{Key, NamedKey};

/// Die Taste zu einem virtuellen Tastencode. `character` ist das Zeichen,
/// das die Taste laut Belegung ohne Umschalttaste erzeugt (aus
/// `MapVirtualKeyW`), `upper` sagt, ob Großbuchstaben gelten (Umschalttaste
/// oder Feststelltaste).
pub fn key_from_virtual_key(vk: u32, character: Option<char>, upper: bool) -> Key {
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
            return match character.filter(|c| !c.is_control()) {
                // MapVirtualKeyW liefert Buchstaben groß.
                Some(c) if c.is_alphabetic() => Key::Character(change_case(c, upper)),
                Some(c) => Key::Character(c),
                None => Key::Unidentified,
            };
        }
    };
    Key::Named(named)
}

fn change_case(c: char, upper: bool) -> char {
    if upper {
        single(c.to_uppercase()).unwrap_or(c)
    } else {
        single(c.to_lowercase()).unwrap_or(c)
    }
}

/// Das einzige Zeichen, falls die Umwandlung genau eines ergibt.
fn single(mut chars: impl Iterator<Item = char>) -> Option<char> {
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys() {
        assert_eq!(
            key_from_virtual_key(0x0D, None, false),
            Key::Named(NamedKey::Enter)
        );
        assert_eq!(
            key_from_virtual_key(0x70, None, false),
            Key::Named(NamedKey::F(1))
        );
        assert_eq!(
            key_from_virtual_key(0x87, None, false),
            Key::Named(NamedKey::F(24))
        );
        assert_eq!(
            key_from_virtual_key(0x5B, None, false),
            Key::Named(NamedKey::Meta)
        );
    }

    #[test]
    fn characters_follow_case() {
        assert_eq!(
            key_from_virtual_key(0x41, Some('A'), false),
            Key::Character('a')
        );
        assert_eq!(
            key_from_virtual_key(0x41, Some('A'), true),
            Key::Character('A')
        );
        assert_eq!(
            key_from_virtual_key(0xDE, Some('Ä'), false),
            Key::Character('ä')
        );
        assert_eq!(
            key_from_virtual_key(0x31, Some('1'), true),
            Key::Character('1')
        );
        assert_eq!(key_from_virtual_key(0x63, None, false), Key::Character('3'));
        assert_eq!(key_from_virtual_key(0xFF, None, false), Key::Unidentified);
    }
}
