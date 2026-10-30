//! Windows-Dateipfade prüfen, bevor Metadaten oder Dateien geöffnet werden.
//!
//! Nur Laufwerkspfade: keine UNC-, Geräte- oder sonstigen NT-Namensräume
//! und keine alternativen Datenströme. Für lange lokale Pfade ist die
//! verbatim Form `\\?\C:\…` erlaubt. DOS-Gerätenamen gelten auch mit
//! Erweiterungen und in Unterordnern; `CONIN$` und `CONOUT$` sind Konsolen,
//! `CLOCK$` wird als historischer Gerätename vorsorglich abgewiesen.
//!
//! Grundlagen: [Naming Files, Paths, and Namespaces][names] und
//! [Console Handles][console]. Die Prüfung ist lexikalisch. Verweise und Reparse
//! Points benötigen weiterhin die Prüfung des tatsächlich geöffneten Handles.
//! Ein Laufwerksbuchstabe kann auch ein verbundenes Netzlaufwerk bezeichnen;
//! diese Prüfung ist keine vollständige Zugriffskontrolle.
//!
//! [names]: https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file
//! [console]: https://learn.microsoft.com/en-us/windows/console/console-handles

#[cfg(test)]
mod tests;

const MAX_UNITS: usize = 32_767;
const BACKSLASH: u16 = b'\\' as u16;
const COLON: u16 = b':' as u16;
const QUESTION: u16 = b'?' as u16;

fn separator(unit: u16) -> bool {
    unit == BACKSLASH || unit == u16::from(b'/')
}

fn drive(unit: u16) -> bool {
    matches!(unit, 0x41..=0x5a | 0x61..=0x7a)
}

/// Ohne verlustbehaftete Umwandlung nach UTF-8: Windows-Dateinamen können
/// auch einzelne UTF-16-Surrogate enthalten.
pub(super) fn allowed(path: &[u16]) -> bool {
    if path.is_empty() || path.len() > MAX_UNITS {
        return false;
    }
    let rest = if path.starts_with(&[BACKSLASH, BACKSLASH, QUESTION, BACKSLASH]) {
        match &path[4..] {
            [letter, COLON, BACKSLASH, rest @ ..] if drive(*letter) => rest,
            _ => return false,
        }
    } else if path.len() >= 2 && separator(path[0]) && separator(path[1]) {
        // Auch UNC-Pfade können zu Pipes führen. Die Dateischnittstelle
        // ist auf Laufwerkspfade beschränkt.
        return false;
    } else if path.len() >= 2 && drive(path[0]) && path[1] == COLON {
        &path[2..]
    } else {
        path
    };
    if rest
        .iter()
        .any(|&unit| unit < 32 || unit == COLON || unit == QUESTION)
    {
        return false;
    }
    !rest.split(|&unit| separator(unit)).any(device)
}

fn equal_ascii(name: &[u16], ascii: &[u8]) -> bool {
    name.len() == ascii.len()
        && name.iter().zip(ascii).all(|(&unit, &expected)| {
            let upper = if (0x61..=0x7a).contains(&unit) {
                unit - 0x20
            } else {
                unit
            };
            upper == u16::from(expected)
        })
}

fn device(component: &[u16]) -> bool {
    let base = component
        .split(|&unit| unit == u16::from(b'.'))
        .next()
        .unwrap_or_default();
    let end = base
        .iter()
        .rposition(|&unit| unit != u16::from(b' '))
        .map_or(0, |index| index + 1);
    let base = &base[..end];
    if [
        b"CON".as_slice(),
        b"PRN",
        b"AUX",
        b"NUL",
        b"CONIN$",
        b"CONOUT$",
        b"CLOCK$",
    ]
    .iter()
    .any(|name| equal_ascii(base, name))
    {
        return true;
    }
    base.len() == 4
        && (equal_ascii(&base[..3], b"COM") || equal_ascii(&base[..3], b"LPT"))
        && matches!(base[3], 0x31..=0x39 | 0x00b9 | 0x00b2 | 0x00b3)
}

/// `absolute` verwendet unter Windows GetFullPathNameW, ohne eine Datei
/// zu öffnen. Weitere von Win32 erkannte Geräte werden dadurch in den
/// Gerätenamensraum aufgelöst und anschließend ebenfalls abgewiesen.
/// Das Ergebnis wird für alle späteren Dateizugriffe wiederverwendet.
#[cfg(windows)]
pub(super) fn resolve(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};

    let check = |path: &std::path::Path| {
        let units: Vec<u16> = path.as_os_str().encode_wide().take(MAX_UNITS + 1).collect();
        allowed(&units).then_some(()).ok_or_else(super::not_regular)
    };
    check(path)?;
    let absolute = std::path::absolute(path)?;
    check(&absolute)?;
    let local = absolute.has_root()
        && matches!(absolute.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)));
    if local {
        Ok(absolute)
    } else {
        Err(super::not_regular())
    }
}
