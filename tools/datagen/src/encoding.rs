//! Die Tabellen für `mb-encoding` aus dem WHATWG Encoding Standard: alle
//! Kodierungen mit ihren Bezeichnungen (`encodings.json`) und die Indizes
//! (`index-*.txt`, Abschnitt „Indexes“ des Standards).

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::json::{self, Value};

/// Die Rohdaten, relativ zum Workspace.
pub const DATA: &str = "data/encoding";

/// Die erzeugte Datei, relativ zum Workspace.
pub const OUTPUT: &str = "crates/mb-encoding/src/tables.rs";

/// Die Indizes der Mehrbyte-Kodierungen laut Tabelle im Abschnitt „Indexes“.
/// `index-gb18030-ranges.txt` ist anders aufgebaut und kommt extra.
const MULTI_BYTE_INDEXES: [&str; 6] = [
    "big5",
    "euc-kr",
    "gb18030",
    "jis0208",
    "jis0212",
    "iso-2022-jp-katakana",
];

/// Die Überschrift der Gruppe in `encodings.json`, deren Kodierungen den
/// gemeinsamen Einzelbyte-Dekodierer nutzen.
const SINGLE_BYTE_HEADING: &str = "Legacy single-byte encodings";

/// Eine Kodierung mit Namen und Bezeichnungen.
struct Encoding {
    name: String,
    labels: Vec<String>,
    single_byte: bool,
}

/// Erzeugt den Quelltext von `tables.rs`. `read` liefert den Inhalt einer
/// Datei aus [`DATA`].
pub fn run(read: &dyn Fn(&str) -> Result<String, String>) -> Result<String, String> {
    let encodings = parse_encodings(&read("encodings.json")?)?;
    let mut single_byte = BTreeMap::new();
    for encoding in encodings.iter().filter(|encoding| encoding.single_byte) {
        let index = single_byte_index_name(&encoding.name);
        // ISO-8859-8 und ISO-8859-8-I teilen sich einen Index.
        if let std::collections::btree_map::Entry::Vacant(entry) = single_byte.entry(index) {
            let index = entry.key();
            let entries = parse_index(index, &read(&format!("index-{index}.txt"))?)?;
            if entries
                .iter()
                .any(|&(pointer, code_point)| pointer >= 128 || code_point > 0xFFFF)
            {
                return Err(format!(
                    "index-{index}.txt: Zeiger über 127 oder Codepunkt über U+FFFF"
                ));
            }
            entry.insert(entries);
        }
    }
    let mut multi_byte = Vec::new();
    for index in MULTI_BYTE_INDEXES {
        multi_byte.push((
            index,
            parse_index(index, &read(&format!("index-{index}.txt"))?)?,
        ));
    }
    let ranges = parse_index("gb18030-ranges", &read("index-gb18030-ranges.txt")?)?;
    emit(&encodings, &single_byte, &multi_byte, &ranges)
        .map_err(|_| "Fehler beim Schreiben des Quelltexts".to_owned())
}

fn parse_encodings(text: &str) -> Result<Vec<Encoding>, String> {
    let groups = json::parse(text)?;
    let groups = groups
        .as_array()
        .ok_or("encodings.json: Liste von Gruppen erwartet")?;
    let mut encodings = Vec::new();
    for group in groups {
        let heading = group
            .get("heading")
            .and_then(Value::as_str)
            .ok_or("encodings.json: Gruppe ohne Überschrift")?;
        let members = group
            .get("encodings")
            .and_then(Value::as_array)
            .ok_or("encodings.json: Gruppe ohne Kodierungen")?;
        for member in members {
            let name = member
                .get("name")
                .and_then(Value::as_str)
                .ok_or("encodings.json: Kodierung ohne Namen")?;
            let labels = member
                .get("labels")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("encodings.json: {name} ohne Bezeichnungen"))?
                .iter()
                .map(|label| {
                    label
                        .as_str()
                        .filter(|label| {
                            !label.is_empty()
                                && label
                                    .bytes()
                                    .all(|b| b.is_ascii_graphic() && !b.is_ascii_uppercase())
                        })
                        .map(str::to_owned)
                        .ok_or_else(|| format!("encodings.json: ungültige Bezeichnung bei {name}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            encodings.push(Encoding {
                name: name.to_owned(),
                labels,
                single_byte: heading == SINGLE_BYTE_HEADING,
            });
        }
    }
    let mut variants: Vec<String> = encodings.iter().map(|e| variant(&e.name)).collect();
    variants.sort();
    if variants.windows(2).any(|pair| pair.first() == pair.get(1)) || variants.is_empty() {
        return Err("encodings.json: Namen ergeben doppelte oder keine Varianten".into());
    }
    let mut labels: Vec<&str> = encodings
        .iter()
        .flat_map(|encoding| encoding.labels.iter().map(String::as_str))
        .collect();
    labels.sort_unstable();
    if labels.windows(2).any(|pair| pair.first() == pair.get(1)) {
        return Err("encodings.json: eine Bezeichnung gehört zu zwei Kodierungen".into());
    }
    Ok(encodings)
}

/// ISO-8859-8 und ISO-8859-8-I teilen sich einen Index (Tabelle im Abschnitt
/// „Legacy single-byte encodings“), alle anderen heißen wie die Kodierung.
fn single_byte_index_name(name: &str) -> String {
    match name {
        "ISO-8859-8-I" => "iso-8859-8".to_owned(),
        _ => name.to_ascii_lowercase(),
    }
}

/// Liest eine Indexdatei nach den Regeln im Abschnitt „Indexes“: Zeilen, die
/// leer sind oder mit `#` beginnen, entfallen; sonst ist das erste Feld (durch
/// Tabulatoren getrennt) der Zeiger dezimal, das zweite der Codepunkt
/// hexadezimal. Die Zeiger müssen aufsteigen.
fn parse_index(name: &str, text: &str) -> Result<Vec<(u32, u32)>, String> {
    let mut entries: Vec<(u32, u32)> = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let error = |message: &str| format!("index-{name}.txt, Zeile {}: {message}", index + 1);
        let mut fields = line.split('\t');
        let pointer = fields
            .next()
            .and_then(|field| field.trim_start_matches(' ').parse::<u32>().ok())
            .ok_or_else(|| error("ungültiger Zeiger"))?;
        let code_point = fields
            .next()
            .and_then(|field| field.strip_prefix("0x"))
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .filter(|&value| char::from_u32(value).is_some_and(|c| c != '\0'))
            .ok_or_else(|| error("ungültiger Codepunkt"))?;
        if entries
            .last()
            .is_some_and(|&(previous, _)| previous >= pointer)
        {
            return Err(error("Zeiger steigen nicht auf"));
        }
        entries.push((pointer, code_point));
    }
    if entries.is_empty() {
        return Err(format!("index-{name}.txt ist leer"));
    }
    Ok(entries)
}

/// `ISO-8859-2` wird `Iso8859_2`, `UTF-16BE` wird `Utf16Be`, `Shift_JIS`
/// wird `ShiftJis`.
fn variant(name: &str) -> String {
    let mut out = String::new();
    for token in name.split(['-', '_']) {
        let mut chars = token.chars().peekable();
        while let Some(&first) = chars.peek() {
            let digits = first.is_ascii_digit();
            let mut run = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() != digits {
                    break;
                }
                run.push(c);
                chars.next();
            }
            if digits {
                if out.ends_with(|c: char| c.is_ascii_digit()) {
                    out.push('_');
                }
                out.push_str(&run);
            } else {
                let mut letters = run.chars();
                if let Some(head) = letters.next() {
                    out.push(head.to_ascii_uppercase());
                    out.extend(letters.map(|c| c.to_ascii_lowercase()));
                }
            }
        }
    }
    out
}

/// `iso-2022-jp-katakana` wird `ISO_2022_JP_KATAKANA`.
fn constant(index: &str) -> String {
    index.to_ascii_uppercase().replace('-', "_")
}

/// Schreibt Zahlen zu sechzehn je Zeile; 0 steht für „kein Eintrag“.
fn values(out: &mut String, values: &[u32]) -> std::fmt::Result {
    for row in values.chunks(16) {
        out.push_str("   ");
        for &value in row {
            if value == 0 {
                write!(out, " 0,")?;
            } else {
                write!(out, " 0x{value:X},")?;
            }
        }
        out.push('\n');
    }
    Ok(())
}

/// Die Einträge als Liste vom Zeiger 0 bis zum größten Zeiger.
fn dense(entries: &[(u32, u32)]) -> Vec<u32> {
    let slot = |pointer: u32| usize::try_from(pointer).unwrap_or(usize::MAX);
    let length = entries
        .last()
        .map_or(0, |&(pointer, _)| slot(pointer).saturating_add(1));
    let mut dense = vec![0; length];
    for &(pointer, code_point) in entries {
        if let Some(value) = dense.get_mut(slot(pointer)) {
            *value = code_point;
        }
    }
    dense
}

fn emit(
    encodings: &[Encoding],
    single_byte: &BTreeMap<String, Vec<(u32, u32)>>,
    multi_byte: &[(&str, Vec<(u32, u32)>)],
    ranges: &[(u32, u32)],
) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    writeln!(
        out,
        "// Erzeugt mit `cargo run -p datagen` aus dem WHATWG Encoding Standard"
    )?;
    writeln!(out, "// ({DATA}). Nicht von Hand ändern.")?;
    writeln!(out, "//")?;
    writeln!(
        out,
        "// Daten: © WHATWG (Apple, Google, Mozilla, Microsoft), in Quelltext unter der"
    )?;
    writeln!(out, "// BSD-3-Clause-Lizenz, siehe {DATA}/LICENSE.txt.")?;
    writeln!(out)?;
    writeln!(out, "/// Eine Zeichenkodierung aus dem Encoding Standard.")?;
    writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]")?;
    writeln!(out, "pub enum Encoding {{")?;
    for encoding in encodings {
        writeln!(out, "    /// `{}`", encoding.name)?;
        writeln!(out, "    {},", variant(&encoding.name))?;
    }
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "impl Encoding {{")?;
    writeln!(
        out,
        "    /// Alle Kodierungen in der Reihenfolge des Standards."
    )?;
    writeln!(
        out,
        "    pub const ALL: [Encoding; {}] = [",
        encodings.len()
    )?;
    for encoding in encodings {
        writeln!(out, "        Encoding::{},", variant(&encoding.name))?;
    }
    writeln!(out, "    ];")?;
    writeln!(out)?;
    writeln!(out, "    /// Der Name, etwa `windows-1252`.")?;
    writeln!(out, "    pub const fn name(self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for encoding in encodings {
        writeln!(
            out,
            "            Encoding::{} => \"{}\",",
            variant(&encoding.name),
            encoding.name
        )?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    let mut labels: Vec<(&str, String)> = encodings
        .iter()
        .flat_map(|encoding| {
            encoding
                .labels
                .iter()
                .map(|label| (label.as_str(), variant(&encoding.name)))
        })
        .collect();
    labels.sort();
    writeln!(
        out,
        "/// Alle Bezeichnungen (labels) in Kleinbuchstaben, nach Bytes sortiert."
    )?;
    writeln!(
        out,
        "pub static LABELS: [(&str, Encoding); {}] = [",
        labels.len()
    )?;
    for (label, variant) in &labels {
        writeln!(out, "    (\"{label}\", Encoding::{variant}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Der Index einer Einzelbyte-Kodierung: Codepunkte der Bytes 0x80 bis 0xFF,"
    )?;
    writeln!(out, "/// 0 für ein Byte ohne Zeichen.")?;
    writeln!(
        out,
        "pub const fn single_byte_index(encoding: Encoding) -> Option<&'static [u16; 128]> {{"
    )?;
    writeln!(out, "    match encoding {{")?;
    for encoding in encodings.iter().filter(|encoding| encoding.single_byte) {
        writeln!(
            out,
            "        Encoding::{} => Some(&SINGLE_BYTE_{}),",
            variant(&encoding.name),
            constant(&single_byte_index_name(&encoding.name))
        )?;
    }
    writeln!(out, "        _ => None,")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    for (index, entries) in single_byte {
        let mut table = dense(entries);
        table.resize(128, 0);
        writeln!(out)?;
        writeln!(out, "/// `index-{index}.txt`")?;
        writeln!(
            out,
            "pub static SINGLE_BYTE_{}: [u16; 128] = [",
            constant(index)
        )?;
        values(&mut out, &table)?;
        writeln!(out, "];")?;
    }
    for (index, entries) in multi_byte {
        let table = dense(entries);
        let kind = if table.iter().all(|&code_point| code_point <= 0xFFFF) {
            "u16"
        } else {
            "u32"
        };
        writeln!(out)?;
        writeln!(
            out,
            "/// `index-{index}.txt`: Codepunkt je Zeiger, 0 für keinen."
        )?;
        writeln!(
            out,
            "pub static INDEX_{}: [{kind}; {}] = [",
            constant(index),
            table.len()
        )?;
        values(&mut out, &table)?;
        writeln!(out, "];")?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "/// `index-gb18030-ranges.txt`: Anfang jedes Bereichs als (Zeiger, Codepunkt)."
    )?;
    writeln!(
        out,
        "pub static INDEX_GB18030_RANGES: [(u32, u32); {}] = [",
        ranges.len()
    )?;
    for &(pointer, code_point) in ranges {
        writeln!(out, "    ({pointer}, 0x{code_point:04X}),")?;
    }
    writeln!(out, "];")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_names() {
        let cases = [
            ("UTF-8", "Utf8"),
            ("ISO-8859-2", "Iso8859_2"),
            ("ISO-8859-8-I", "Iso8859_8I"),
            ("ISO-8859-10", "Iso8859_10"),
            ("UTF-16BE", "Utf16Be"),
            ("gb18030", "Gb18030"),
            ("KOI8-R", "Koi8R"),
            ("Shift_JIS", "ShiftJis"),
            ("x-mac-cyrillic", "XMacCyrillic"),
            ("windows-1252", "Windows1252"),
            ("IBM866", "Ibm866"),
        ];
        for (name, expected) in cases {
            assert_eq!(variant(name), expected);
        }
        assert_eq!(constant("iso-2022-jp-katakana"), "ISO_2022_JP_KATAKANA");
        assert_eq!(single_byte_index_name("ISO-8859-8-I"), "iso-8859-8");
        assert_eq!(single_byte_index_name("windows-1252"), "windows-1252");
    }

    #[test]
    fn reads_index_files() {
        let text = "# Identifier: x\n\n  0\t0x20AC\t€ (EURO SIGN)\n  2\t0x201A\t‚\r\n";
        assert_eq!(
            parse_index("test", text),
            Ok(vec![(0, 0x20AC), (2, 0x201A)])
        );
        assert_eq!(dense(&[(0, 0x20AC), (2, 0x201A)]), [0x20AC, 0, 0x201A]);
        for broken in [
            "",
            "# nur Kommentar\n",
            "x\t0x20AC\n",
            "0\t20AC\n",
            "0\t0xD800\n",
            "0\t0x0\n",
            "0\t0x110000\n",
            "1\t0x41\n0\t0x42\n",
            "1\t0x41\n1\t0x42\n",
        ] {
            assert!(parse_index("test", broken).is_err(), "{broken:?}");
        }
    }

    #[test]
    fn reads_encodings_json() {
        let text = r#"[
            {"heading": "The Encoding", "encodings": [{"name": "UTF-8", "labels": ["utf-8", "utf8"]}]},
            {"heading": "Legacy single-byte encodings", "encodings": [{"name": "IBM866", "labels": ["866"]}]}
        ]"#;
        let encodings = parse_encodings(text).expect("gültig");
        assert_eq!(encodings.len(), 2);
        assert_eq!(encodings[0].labels, ["utf-8", "utf8"]);
        assert!(!encodings[0].single_byte && encodings[1].single_byte);
        for broken in [
            "{}",
            "[]",
            r#"[{"heading": "x", "encodings": [{"name": "A", "labels": ["UPPER"]}]}]"#,
            r#"[{"heading": "x", "encodings": [{"name": "A", "labels": [""]}]}]"#,
            r#"[{"heading": "x", "encodings": [{"name": "A-1", "labels": ["a"]}, {"name": "a_1", "labels": ["b"]}]}]"#,
            r#"[{"heading": "x", "encodings": [{"name": "A", "labels": ["a"]}, {"name": "B", "labels": ["a"]}]}]"#,
        ] {
            assert!(parse_encodings(broken).is_err(), "{broken}");
        }
    }
}
