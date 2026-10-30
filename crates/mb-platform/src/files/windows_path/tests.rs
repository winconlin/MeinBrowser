//! Die Windows-Pfadregeln werden auch auf anderen Systemen geprüft.

use super::allowed;

fn accepts(path: &str) -> bool {
    allowed(&path.encode_utf16().collect::<Vec<_>>())
}

#[test]
fn ordinary_relative_and_drive_paths_remain_allowed() {
    for path in [
        "style.css",
        "assets/style.css",
        r"assets\image.gif",
        r"..\assets\.\style.css",
        "./style.css",
        r"\root\style.css",
        "/root/style.css",
        r"C:\assets\style.css",
        "c:/assets/style.css",
        "C:style.css",
        r"D:assets\style.css",
        r"\\?\C:\assets\style.css",
        r"\\?\d:\assets\style.css",
        "Schriften/Ägyptisch-漢字-😀.ttf",
    ] {
        assert!(accepts(path), "normaler Dateipfad: {path:?}");
    }
}

#[test]
fn reserved_devices_are_refused_with_case_and_filename_suffixes() {
    let mut devices: Vec<String> = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    for prefix in ["COM", "LPT"] {
        for number in '1'..='9' {
            devices.push(format!("{prefix}{number}"));
        }
        for number in ['¹', '²', '³'] {
            devices.push(format!("{prefix}{number}"));
        }
    }
    for device in devices {
        let lowercase = device.to_ascii_lowercase();
        for name in [&device, &lowercase] {
            for prefix in ["", r"C:\assets\", "C:", "assets/", r"\\?\C:\assets\"] {
                for suffix in ["", ".txt", ".html.backup", "   .txt", ".", "   "] {
                    let path = format!("{prefix}{name}{suffix}");
                    assert!(!accepts(&path), "Gerätename: {path:?}");
                }
            }
        }
    }
}

#[test]
fn reserved_devices_are_also_refused_in_parent_components() {
    for path in [
        "NUL/style.css",
        r"C:\CON\style.css",
        "C:/assets/COM1/icon.gif",
        r"assets\LPT².txt/child.css",
        r"C:\assets\NUL\..\style.css",
        r"\\?\C:\assets\AUX\style.css",
    ] {
        assert!(!accepts(path), "Gerät in einem Pfadteil: {path:?}");
    }
}

#[test]
fn similar_regular_names_are_not_device_aliases() {
    for name in [
        ".NUL",
        " NUL.txt",
        "COM0",
        "COM10",
        "COM１",
        "COM١",
        "COM¹x",
        "LPT0",
        "LPT10",
        "LPT⁴",
        "LPT-1",
        "CONSOLE",
        "AUXILIARY",
        "CONIN",
        "CONOUT",
        "CONIN$x",
        "NUL-file",
        "normalNUL.txt",
        "normal.NUL",
        "normal.COM1",
    ] {
        let path = format!(r"C:\assets\{name}");
        assert!(accepts(&path), "gewöhnlicher ähnlicher Name: {path:?}");
    }
}

#[test]
fn namespaces_and_network_paths_are_refused() {
    for path in [
        r"\\.\NUL",
        r"\\.\C:\style.css",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1\style.css",
        r"\\?\UNC\server\share\style.css",
        r"\\?\Volume{1234}\style.css",
        r"\\?\C:style.css",
        r"\\?\C:/style.css",
        r"//?/C:/style.css",
        r"\/?\C:\style.css",
        r"/\?\C:\style.css",
        r"\??\C:\style.css",
        r"\\??\C:\style.css",
        r"\\server\share\style.css",
        "//server/share/style.css",
        r"/\server\share\style.css",
        r"\/server/share/style.css",
    ] {
        assert!(!accepts(path), "kein erlaubter Datenträgerpfad: {path:?}");
    }
}

#[test]
fn alternate_streams_are_refused_after_any_drive_prefix() {
    for path in [
        "style.css:stream",
        r"C:\assets\style.css:stream",
        "C:style.css:stream",
        "C::stream",
        "/assets/style.css:stream",
        r"\\?\C:\assets\style.css::$DATA",
        r"C:\assets\NUL:stream",
    ] {
        assert!(!accepts(path), "zusätzlicher Doppelpunkt: {path:?}");
    }
}

#[test]
fn control_characters_and_question_marks_are_refused() {
    for control in 0..=31 {
        let mut path: Vec<u16> = "assets/".encode_utf16().collect();
        path.push(control);
        path.extend("style.css".encode_utf16());
        assert!(!allowed(&path), "Steuerzeichen U+{control:04X}");
    }
    for path in ["what?.css", "assets/?/style.css", r"C:\assets\style?.css"] {
        assert!(!accepts(path), "Fragezeichen: {path:?}");
    }
    // Das Fragezeichen der einzigen erlaubten Verbatim-Präfixform ist
    // Teil des Präfixes, kein Zeichen eines Dateinamens.
    assert!(accepts(r"\\?\C:\assets\style.css"));
}

#[test]
fn raw_utf16_is_checked_without_lossy_unicode_conversion() {
    let mut ordinary: Vec<u16> = "assets/".encode_utf16().collect();
    ordinary.push(0xD800);
    ordinary.extend(".css".encode_utf16());
    assert!(
        allowed(&ordinary),
        "ein ungepaartes Surrogat ist kein Gerät"
    );

    let mut device = ordinary;
    device.extend("/NUL.txt".encode_utf16());
    assert!(
        !allowed(&device),
        "folgende Gerätekomponente bleibt sichtbar"
    );
}

#[test]
fn empty_and_overlong_paths_are_refused_in_utf16_units() {
    assert!(!allowed(&[]));
    let mut path = vec![u16::from(b'a'); 32_767];
    assert!(allowed(&path), "oberste zugelassene UTF-16-Länge");
    path.push(u16::from(b'a'));
    assert!(!allowed(&path), "ein UTF-16-Codeelement über der Grenze");
}
