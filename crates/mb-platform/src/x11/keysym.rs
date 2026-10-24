//! Tastaturbelegung: von Tastencodes über Keysyms zu Tasten und Zeichen.
//!
//! Die Auswahl des Keysyms folgt Abschnitt 5 („Keyboards“) der
//! Protokollspezifikation. Zusätzlich wird die dritte Ebene (AltGr)
//! ausgewertet, die XKB-Server für Kernprotokoll-Clients an den Positionen 5
//! und 6 der Keysym-Liste ablegen.
//!
//! Noch nicht unterstützt: Tottasten und Compose, die alten Keysym-Bereiche
//! außerhalb von Latin-1 (etwa Kyrillisch oder Griechisch mit eigenen Keysyms)
//! und Eingabemethoden. Unicode-Keysyms (`0x01000000 + Codepunkt`) werden
//! dagegen vollständig umgesetzt.

#![forbid(unsafe_code)]

use crate::event::{Key, Modifiers, NamedKey};

pub const NO_SYMBOL: u32 = 0;

pub const MASK_SHIFT: u16 = 0x01;
pub const MASK_LOCK: u16 = 0x02;
pub const MASK_CONTROL: u16 = 0x04;

const KEYSYM_NUM_LOCK: u32 = 0xff7f;
const KEYSYM_MODE_SWITCH: u32 = 0xff7e;
const KEYSYM_ISO_LEVEL3_SHIFT: u32 = 0xfe03;
const KEYSYM_CAPS_LOCK: u32 = 0xffe5;
const KEYSYM_SHIFT_LOCK: u32 = 0xffe6;

/// Welche Umschalttaste auf welchem Modifikator-Bit liegt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModifierRoles {
    pub num_lock: u16,
    pub mode_switch: u16,
    pub level3: u16,
    pub alt: u16,
    pub meta: u16,
    pub lock_is_caps_lock: bool,
    pub lock_is_shift_lock: bool,
}

impl Default for ModifierRoles {
    /// Die übliche Belegung, solange die tatsächliche nicht bekannt ist:
    /// Mod1 = Alt, Mod2 = NumLock, Mod4 = Super, Mod5 = AltGr.
    fn default() -> ModifierRoles {
        ModifierRoles {
            num_lock: 0x10,
            mode_switch: 0,
            level3: 0x80,
            alt: 0x08,
            meta: 0x40,
            lock_is_caps_lock: true,
            lock_is_shift_lock: false,
        }
    }
}

/// Die Tastaturbelegung des Servers.
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    min_keycode: u8,
    per_keycode: usize,
    keysyms: Vec<u32>,
    roles: ModifierRoles,
}

impl Keymap {
    pub fn new(min_keycode: u8, per_keycode: usize, keysyms: Vec<u32>) -> Keymap {
        Keymap {
            min_keycode,
            per_keycode,
            keysyms,
            roles: ModifierRoles::default(),
        }
    }

    /// Alle Keysyms eines Tastencodes ohne abschließende `NoSymbol`.
    pub fn keysyms(&self, keycode: u8) -> &[u32] {
        let Some(offset) = keycode.checked_sub(self.min_keycode) else {
            return &[];
        };
        let start = usize::from(offset).saturating_mul(self.per_keycode);
        let end = start.saturating_add(self.per_keycode);
        let list = self.keysyms.get(start..end).unwrap_or(&[]);
        let used = list
            .iter()
            .rposition(|&k| k != NO_SYMBOL)
            .map_or(0, |i| i + 1);
        list.get(..used).unwrap_or(&[])
    }

    /// Wertet die Antwort auf `GetModifierMapping` aus: acht Gruppen (Shift,
    /// Lock, Control, Mod1 bis Mod5) mit je `per_modifier` Tastencodes.
    pub fn set_modifier_mapping(&mut self, per_modifier: usize, keycodes: &[u8]) {
        let mut roles = ModifierRoles {
            num_lock: 0,
            mode_switch: 0,
            level3: 0,
            alt: 0,
            meta: 0,
            lock_is_caps_lock: false,
            lock_is_shift_lock: false,
        };
        if per_modifier == 0 {
            self.roles = roles;
            return;
        }
        for (index, group) in keycodes.chunks(per_modifier).take(8).enumerate() {
            let mask = 1u16 << index;
            for &keycode in group.iter().filter(|&&k| k != 0) {
                for &keysym in self.keysyms(keycode) {
                    match (index, keysym) {
                        (1, KEYSYM_CAPS_LOCK) => roles.lock_is_caps_lock = true,
                        (1, KEYSYM_SHIFT_LOCK) => roles.lock_is_shift_lock = true,
                        (3.., KEYSYM_NUM_LOCK) => roles.num_lock |= mask,
                        (3.., KEYSYM_MODE_SWITCH) => roles.mode_switch |= mask,
                        (3.., KEYSYM_ISO_LEVEL3_SHIFT) => roles.level3 |= mask,
                        (3.., 0xffe7..=0xffea) => roles.alt |= mask,
                        (3.., 0xffeb..=0xffee) => roles.meta |= mask,
                        _ => {}
                    }
                }
            }
        }
        self.roles = roles;
    }

    pub fn modifiers(&self, state: u16) -> Modifiers {
        Modifiers {
            shift: state & MASK_SHIFT != 0,
            control: state & MASK_CONTROL != 0,
            alt: state & self.roles.alt != 0,
            meta: state & self.roles.meta != 0,
        }
    }

    /// Das Keysym für einen Tastencode bei gegebenem Modifikatorzustand.
    pub fn lookup(&self, keycode: u8, state: u16) -> u32 {
        let list = self.keysyms(keycode);
        let get = |i: usize| list.get(i).copied().unwrap_or(NO_SYMBOL);
        // Kürzere Listen werden laut Spezifikation aufgefüllt.
        let (group1, group2) = match list.len() {
            0 => return NO_SYMBOL,
            1 => ((get(0), NO_SYMBOL), (get(0), NO_SYMBOL)),
            2 => ((get(0), get(1)), (get(0), get(1))),
            3 => ((get(0), get(1)), (get(2), NO_SYMBOL)),
            _ => ((get(0), get(1)), (get(2), get(3))),
        };
        let mut group = if state & self.roles.mode_switch != 0 {
            group2
        } else {
            group1
        };
        if state & self.roles.level3 != 0 && get(4) != NO_SYMBOL {
            group = (get(4), get(5));
        }
        let (first, second) = match group {
            (first, NO_SYMBOL) => match case_pair(first) {
                Some((lower, upper)) => (lower, upper),
                None => (first, first),
            },
            pair => pair,
        };

        let shift = state & MASK_SHIFT != 0;
        let lock = state & MASK_LOCK != 0;
        let caps_lock = lock && self.roles.lock_is_caps_lock;
        let shift_lock = lock && !self.roles.lock_is_caps_lock && self.roles.lock_is_shift_lock;
        if state & self.roles.num_lock != 0 && is_keypad(second) {
            if shift || shift_lock { first } else { second }
        } else if !shift && !caps_lock && !shift_lock {
            first
        } else if !shift && caps_lock {
            to_upper(first)
        } else if shift && caps_lock {
            to_upper(second)
        } else {
            second
        }
    }
}

fn is_keypad(keysym: u32) -> bool {
    (0xff80..=0xffbd).contains(&keysym)
}

/// Klein- und Großbuchstabe zu einem Keysym, falls es beide gibt.
fn case_pair(keysym: u32) -> Option<(u32, u32)> {
    match keysym {
        0x41..=0x5a => Some((keysym + 0x20, keysym)),
        0x61..=0x7a => Some((keysym, keysym - 0x20)),
        0xc0..=0xde if keysym != 0xd7 => Some((keysym + 0x20, keysym)),
        0xe0..=0xfe if keysym != 0xf7 => Some((keysym, keysym - 0x20)),
        0x0100_0100..=0x0110_ffff => {
            let c = char::from_u32(keysym - 0x0100_0000)?;
            let lower = single(c.to_lowercase())?;
            let upper = single(c.to_uppercase())?;
            (lower != upper).then(|| (unicode_keysym(lower), unicode_keysym(upper)))
        }
        _ => None,
    }
}

fn single(mut chars: impl Iterator<Item = char>) -> Option<char> {
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

fn unicode_keysym(c: char) -> u32 {
    let code = u32::from(c);
    // Latin-1 hat eigene Keysyms mit gleichem Wert.
    if (0x20..=0x7e).contains(&code) || (0xa0..=0xff).contains(&code) {
        code
    } else {
        0x0100_0000 + code
    }
}

fn to_upper(keysym: u32) -> u32 {
    match case_pair(keysym) {
        Some((lower, upper)) if lower == keysym => upper,
        _ => keysym,
    }
}

/// Das Zeichen, das ein Keysym erzeugt, ohne Steuerzeichen.
pub fn char_from_keysym(keysym: u32) -> Option<char> {
    let code = match keysym {
        0x20..=0x7e | 0xa0..=0xff => keysym,
        0x0100_00a0..=0x0110_ffff => keysym - 0x0100_0000,
        // Ziffernblock
        0xff80 => 0x20,
        0xffaa => u32::from('*'),
        0xffab => u32::from('+'),
        0xffac => u32::from(','),
        0xffad => u32::from('-'),
        0xffae => u32::from('.'),
        0xffaf => u32::from('/'),
        0xffb0..=0xffb9 => keysym - 0xffb0 + u32::from('0'),
        0xffbd => u32::from('='),
        // Eurozeichen hat ein eigenes, älteres Keysym.
        0x20ac => 0x20ac,
        _ => return None,
    };
    char::from_u32(code).filter(|c| !c.is_control())
}

/// Die Taste zu einem Keysym.
pub fn key_from_keysym(keysym: u32) -> Key {
    let named = match keysym {
        0xff08 => NamedKey::Backspace,
        0xff09 | 0xfe20 => NamedKey::Tab,
        0xff0d | 0xff8d => NamedKey::Enter,
        0xff13 => NamedKey::Pause,
        0xff14 => NamedKey::ScrollLock,
        0xff1b => NamedKey::Escape,
        0xffff | 0xff9f => NamedKey::Delete,
        0xff50 | 0xff95 => NamedKey::Home,
        0xff51 | 0xff96 => NamedKey::ArrowLeft,
        0xff52 | 0xff97 => NamedKey::ArrowUp,
        0xff53 | 0xff98 => NamedKey::ArrowRight,
        0xff54 | 0xff99 => NamedKey::ArrowDown,
        0xff55 | 0xff9a => NamedKey::PageUp,
        0xff56 | 0xff9b => NamedKey::PageDown,
        0xff57 | 0xff9c => NamedKey::End,
        0xff61 => NamedKey::PrintScreen,
        0xff63 | 0xff9e => NamedKey::Insert,
        0xff67 => NamedKey::ContextMenu,
        KEYSYM_NUM_LOCK => NamedKey::NumLock,
        0xffbe..=0xffd5 => NamedKey::F((keysym - 0xffbe + 1) as u8),
        0xffe1 | 0xffe2 => NamedKey::Shift,
        0xffe3 | 0xffe4 => NamedKey::Control,
        KEYSYM_CAPS_LOCK => NamedKey::CapsLock,
        0xffe7 | 0xffe8 | 0xffeb | 0xffec => NamedKey::Meta,
        0xffe9 | 0xffea => NamedKey::Alt,
        KEYSYM_ISO_LEVEL3_SHIFT | KEYSYM_MODE_SWITCH => NamedKey::AltGraph,
        _ => {
            return match char_from_keysym(keysym) {
                Some(c) => Key::Character(c),
                None => Key::Unidentified,
            };
        }
    };
    Key::Named(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ein kleiner Ausschnitt einer deutschen Belegung, wie Xvfb ihn liefert.
    fn german() -> Keymap {
        let per_keycode = 7;
        let mut keysyms = vec![NO_SYMBOL; per_keycode * 5];
        // Tastencode 8: q Q q Q @ Ω
        keysyms[..6].copy_from_slice(&[0x71, 0x51, 0x71, 0x51, 0x40, 0x07d9]);
        // Tastencode 9: nur „a“, Großbuchstabe ergibt sich aus der Regel
        keysyms[per_keycode] = 0x61;
        // Tastencode 10: 1 !
        keysyms[2 * per_keycode..2 * per_keycode + 2].copy_from_slice(&[0x31, 0x21]);
        // Tastencode 11: KP_End KP_1
        keysyms[3 * per_keycode..3 * per_keycode + 2].copy_from_slice(&[0xff9c, 0xffb1]);
        // Tastencode 12: nur das Unicode-Keysym ā, der Großbuchstabe ergibt sich aus der Regel
        keysyms[4 * per_keycode] = 0x0100_0101;
        Keymap::new(8, per_keycode, keysyms)
    }

    #[test]
    fn trailing_no_symbols_are_ignored() {
        let keymap = german();
        assert_eq!(keymap.keysyms(9), &[0x61]);
        assert_eq!(keymap.keysyms(7), &[] as &[u32]);
        assert_eq!(keymap.keysyms(200), &[] as &[u32]);
    }

    #[test]
    fn shift_caps_lock_and_alt_gr() {
        let keymap = german();
        assert_eq!(keymap.lookup(8, 0), 0x71);
        assert_eq!(keymap.lookup(8, MASK_SHIFT), 0x51);
        assert_eq!(keymap.lookup(8, MASK_LOCK), 0x51);
        assert_eq!(keymap.lookup(8, MASK_SHIFT | MASK_LOCK), 0x51);
        assert_eq!(keymap.lookup(8, 0x80), 0x40);
        assert_eq!(keymap.lookup(9, 0), 0x61);
        assert_eq!(keymap.lookup(9, MASK_SHIFT), 0x41);
        assert_eq!(keymap.lookup(10, MASK_LOCK), 0x31);
        assert_eq!(keymap.lookup(10, MASK_SHIFT), 0x21);
        assert_eq!(keymap.lookup(12, 0), 0x0100_0101);
        assert_eq!(keymap.lookup(12, MASK_SHIFT), 0x0100_0100);
    }

    #[test]
    fn num_lock_switches_keypad() {
        let keymap = german();
        assert_eq!(keymap.lookup(11, 0), 0xff9c);
        assert_eq!(keymap.lookup(11, 0x10), 0xffb1);
        assert_eq!(keymap.lookup(11, 0x10 | MASK_SHIFT), 0xff9c);
    }

    #[test]
    fn modifier_mapping_finds_roles() {
        let mut keymap = Keymap::new(
            8,
            1,
            vec![0xffe1, 0xffe5, 0xffe3, 0xffe9, 0xff7f, 0xffeb, 0xfe03],
        );
        // Zwei Tastencodes je Modifikator: Shift, Lock, Control, Mod1 … Mod5
        let map = [8, 0, 9, 0, 10, 0, 11, 0, 12, 0, 0, 0, 13, 0, 14, 0];
        keymap.set_modifier_mapping(2, &map);
        assert_eq!(
            keymap.roles,
            ModifierRoles {
                num_lock: 0x10,
                mode_switch: 0,
                level3: 0x80,
                alt: 0x08,
                meta: 0x40,
                lock_is_caps_lock: true,
                lock_is_shift_lock: false,
            }
        );
        assert_eq!(
            keymap.modifiers(0x08 | MASK_CONTROL),
            Modifiers {
                shift: false,
                control: true,
                alt: true,
                meta: false,
            }
        );
    }

    #[test]
    fn keysyms_to_characters() {
        assert_eq!(char_from_keysym(0x61), Some('a'));
        assert_eq!(char_from_keysym(0xe4), Some('ä'));
        assert_eq!(char_from_keysym(0xdf), Some('ß'));
        assert_eq!(char_from_keysym(0x20ac), Some('€'));
        assert_eq!(char_from_keysym(0x0100_20ac), Some('€'));
        assert_eq!(char_from_keysym(0x0101_f600), Some('😀'));
        assert_eq!(char_from_keysym(0xffb7), Some('7'));
        assert_eq!(char_from_keysym(0xff0d), None);
        assert_eq!(char_from_keysym(0x0100_0007), None);
        assert_eq!(char_from_keysym(0x0100_d800), None);
    }

    #[test]
    fn keysyms_to_keys() {
        assert_eq!(key_from_keysym(0xff0d), Key::Named(NamedKey::Enter));
        assert_eq!(key_from_keysym(0xffbe), Key::Named(NamedKey::F(1)));
        assert_eq!(key_from_keysym(0xffc9), Key::Named(NamedKey::F(12)));
        assert_eq!(key_from_keysym(0xfe03), Key::Named(NamedKey::AltGraph));
        assert_eq!(key_from_keysym(0x51), Key::Character('Q'));
        assert_eq!(key_from_keysym(0xfe51), Key::Unidentified);
    }
}
