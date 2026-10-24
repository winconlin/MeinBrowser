//! Die Umgebungsvariable `DISPLAY` und die Anmeldedaten aus `~/.Xauthority`.

#![forbid(unsafe_code)]

use std::net::IpAddr;
use std::path::PathBuf;

use crate::Error;

/// Wohin die Verbindung zum X-Server geht.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Address {
    /// Lokaler Socket. Unter Linux gibt es zusätzlich den gleichnamigen
    /// abstrakten Socket, den viele Server bevorzugen.
    Unix(PathBuf),
    Tcp {
        host: String,
        port: u16,
    },
}

/// Eine zerlegte Angabe der Form `[protokoll/][rechner]:anzeige[.bildschirm]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    pub address: Address,
    pub number: u16,
    pub screen: u16,
}

pub fn parse_display(name: &str) -> Result<Display, Error> {
    let invalid = || Error::InvalidArgument(format!("DISPLAY={name:?} ist ungültig"));
    let (protocol, rest) = match name.split_once('/') {
        Some((protocol @ ("unix" | "tcp" | "inet" | "inet6"), rest)) => (Some(protocol), rest),
        _ => (None, name),
    };
    let (host, display) = rest.rsplit_once(':').ok_or_else(invalid)?;
    if host.ends_with(':') {
        return Err(Error::Unsupported(format!(
            "DECnet-Adresse in DISPLAY={name:?}"
        )));
    }
    let (number, screen) = match display.split_once('.') {
        Some((number, screen)) => (number, Some(screen)),
        None => (display, None),
    };
    let number: u16 = number.parse().map_err(|_| invalid())?;
    let screen: u16 = match screen {
        Some(screen) => screen.parse().map_err(|_| invalid())?,
        None => 0,
    };
    let address = if host.starts_with('/') {
        // Vollständiger Pfad zum Socket, wie ihn macOS mit XQuartz setzt.
        Address::Unix(PathBuf::from(host))
    } else if protocol != Some("tcp")
        && protocol != Some("inet")
        && protocol != Some("inet6")
        && (host.is_empty() || host == "unix")
    {
        Address::Unix(PathBuf::from(format!("/tmp/.X11-unix/X{number}")))
    } else {
        let host = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        let host = if host.is_empty() { "localhost" } else { host };
        let port = 6000u16.checked_add(number).ok_or_else(invalid)?;
        Address::Tcp {
            host: host.to_owned(),
            port,
        }
    };
    Ok(Display {
        address,
        number,
        screen,
    })
}

pub const FAMILY_INTERNET: u16 = 0;
pub const FAMILY_INTERNET6: u16 = 6;
pub const FAMILY_LOCAL: u16 = 256;
pub const FAMILY_WILD: u16 = 65_535;

/// Der einzige Anmeldeweg, den dieser Client beherrscht.
pub const MIT_MAGIC_COOKIE: &[u8] = b"MIT-MAGIC-COOKIE-1";

/// Ein Eintrag aus einer Xauthority-Datei.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthEntry {
    pub family: u16,
    pub address: Vec<u8>,
    pub number: Vec<u8>,
    pub name: Vec<u8>,
    pub data: Vec<u8>,
}

/// Liest alle vollständigen Einträge. Die Datei besteht aus Einträgen mit
/// Familie (16 Bit) und vier Feldern, jeweils mit 16-Bit-Länge davor, alles
/// in Netzwerk-Bytereihenfolge.
pub fn parse_xauthority(bytes: &[u8]) -> Vec<AuthEntry> {
    let mut entries = Vec::new();
    let mut rest = bytes;
    while let Some((entry, after)) = take_entry(rest) {
        entries.push(entry);
        rest = after;
    }
    entries
}

fn take_entry(bytes: &[u8]) -> Option<(AuthEntry, &[u8])> {
    let (family, rest) = take_u16(bytes)?;
    let (address, rest) = take_field(rest)?;
    let (number, rest) = take_field(rest)?;
    let (name, rest) = take_field(rest)?;
    let (data, rest) = take_field(rest)?;
    let entry = AuthEntry {
        family,
        address: address.to_vec(),
        number: number.to_vec(),
        name: name.to_vec(),
        data: data.to_vec(),
    };
    Some((entry, rest))
}

fn take_u16(bytes: &[u8]) -> Option<(u16, &[u8])> {
    let value = u16::from_be_bytes(bytes.get(..2)?.try_into().ok()?);
    Some((value, bytes.get(2..)?))
}

fn take_field(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let (length, rest) = take_u16(bytes)?;
    let length = usize::from(length);
    Some((rest.get(..length)?, rest.get(length..)?))
}

/// Eine Adresse, unter der der Client beim Server bekannt ist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthAddress {
    pub family: u16,
    pub address: Vec<u8>,
}

impl AuthAddress {
    pub fn local(hostname: &[u8]) -> AuthAddress {
        AuthAddress {
            family: FAMILY_LOCAL,
            address: hostname.to_vec(),
        }
    }

    pub fn ip(address: IpAddr) -> AuthAddress {
        match address {
            IpAddr::V4(v4) => AuthAddress {
                family: FAMILY_INTERNET,
                address: v4.octets().to_vec(),
            },
            IpAddr::V6(v6) => AuthAddress {
                family: FAMILY_INTERNET6,
                address: v6.octets().to_vec(),
            },
        }
    }
}

/// Sucht wie Xlib den ersten passenden Eintrag mit `MIT-MAGIC-COOKIE-1`:
/// Familie und Adresse stimmen (oder der Eintrag gilt für alle Adressen), und
/// die Anzeigenummer stimmt (oder fehlt im Eintrag).
pub fn find_cookie<'a>(
    entries: &'a [AuthEntry],
    addresses: &[AuthAddress],
    number: u16,
) -> Option<&'a AuthEntry> {
    let number = number.to_string();
    entries.iter().find(|entry| {
        let address_matches = entry.family == FAMILY_WILD
            || addresses
                .iter()
                .any(|a| a.family == entry.family && a.address == entry.address);
        let number_matches = entry.number.is_empty() || entry.number == number.as_bytes();
        address_matches && number_matches && entry.name == MIT_MAGIC_COOKIE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix(path: &str, number: u16, screen: u16) -> Display {
        Display {
            address: Address::Unix(PathBuf::from(path)),
            number,
            screen,
        }
    }

    fn tcp(host: &str, port: u16, number: u16, screen: u16) -> Display {
        Display {
            address: Address::Tcp {
                host: host.to_owned(),
                port,
            },
            number,
            screen,
        }
    }

    #[test]
    fn parses_display_names() {
        assert_eq!(
            parse_display(":0").unwrap(),
            unix("/tmp/.X11-unix/X0", 0, 0)
        );
        assert_eq!(
            parse_display(":99.1").unwrap(),
            unix("/tmp/.X11-unix/X99", 99, 1)
        );
        assert_eq!(
            parse_display("unix:3").unwrap(),
            unix("/tmp/.X11-unix/X3", 3, 0)
        );
        assert_eq!(
            parse_display("unix/:3").unwrap(),
            unix("/tmp/.X11-unix/X3", 3, 0)
        );
        assert_eq!(
            parse_display("localhost:10.0").unwrap(),
            tcp("localhost", 6010, 10, 0)
        );
        assert_eq!(
            parse_display("tcp/:1").unwrap(),
            tcp("localhost", 6001, 1, 0)
        );
        assert_eq!(parse_display("[::1]:2").unwrap(), tcp("::1", 6002, 2, 0));
        assert_eq!(
            parse_display("/private/tmp/com.apple.launchd.x/org.xquartz:0").unwrap(),
            unix("/private/tmp/com.apple.launchd.x/org.xquartz", 0, 0)
        );
    }

    #[test]
    fn rejects_invalid_display_names() {
        for name in ["", "0", ":", ":x", ":0.x", "host::0", "localhost:65535"] {
            assert!(parse_display(name).is_err(), "{name:?}");
        }
    }

    fn entry(family: u16, address: &[u8], number: &[u8], name: &[u8], data: &[u8]) -> Vec<u8> {
        let mut bytes = family.to_be_bytes().to_vec();
        for field in [address, number, name, data] {
            bytes.extend((field.len() as u16).to_be_bytes());
            bytes.extend(field);
        }
        bytes
    }

    #[test]
    fn parses_xauthority_and_finds_cookie() {
        let mut file = entry(FAMILY_LOCAL, b"anderer", b"0", MIT_MAGIC_COOKIE, &[1; 16]);
        file.extend(entry(
            FAMILY_LOCAL,
            b"rechner",
            b"1",
            MIT_MAGIC_COOKIE,
            &[2; 16],
        ));
        file.extend(entry(
            FAMILY_LOCAL,
            b"rechner",
            b"0",
            b"XDM-AUTHORIZATION-1",
            &[3; 8],
        ));
        file.extend(entry(
            FAMILY_LOCAL,
            b"rechner",
            b"0",
            MIT_MAGIC_COOKIE,
            &[4; 16],
        ));
        file.extend(entry(FAMILY_WILD, b"", b"", MIT_MAGIC_COOKIE, &[5; 16]));
        let entries = parse_xauthority(&file);
        assert_eq!(entries.len(), 5);

        let local = [AuthAddress::local(b"rechner")];
        assert_eq!(find_cookie(&entries, &local, 0).unwrap().data, [4; 16]);
        assert_eq!(find_cookie(&entries, &local, 1).unwrap().data, [2; 16]);
        // Andere Anzeige: nur der Eintrag für alle Adressen passt.
        assert_eq!(find_cookie(&entries, &local, 7).unwrap().data, [5; 16]);
        let ip = [AuthAddress::ip(IpAddr::from([10, 0, 0, 1]))];
        assert_eq!(find_cookie(&entries, &ip, 0).unwrap().data, [5; 16]);
    }

    #[test]
    fn truncated_xauthority_keeps_complete_entries() {
        let mut file = entry(FAMILY_LOCAL, b"rechner", b"0", MIT_MAGIC_COOKIE, &[4; 16]);
        let complete = file.len();
        file.extend(entry(FAMILY_WILD, b"", b"", MIT_MAGIC_COOKIE, &[5; 16]));
        for length in 0..file.len() {
            let entries = parse_xauthority(&file[..length]);
            assert_eq!(
                entries.len(),
                usize::from(length >= complete),
                "Länge {length}"
            );
        }
    }
}
