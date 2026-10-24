//! Die IDNA-Zuordnungstabelle für `mb-unicode` (UTS #46, Abschnitt 5) aus
//! `IdnaMappingTable.txt`.
//!
//! Jede Zeile nennt für einen Codepunkt oder Bereich den Status und, wo
//! nötig, die Abbildung. Das vierte Feld (der Status nach IDNA2008, `NV8`
//! oder `XV8`) spielt für die Verarbeitung nach UTS #46 keine Rolle.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::ucd::{self, MAX_CODE_POINT, Version};

/// Die Rohdaten, relativ zum Workspace.
pub const DATA: &str = "data/unicode";

/// Die erzeugte Datei, relativ zum Workspace.
pub const OUTPUT: &str = "crates/mb-unicode/src/idna/tables.rs";

/// Die Status der Tabelle und ihre Namen in `mb-unicode`.
const STATUSES: [(&str, &str); 5] = [
    ("valid", "Valid"),
    ("ignored", "Ignored"),
    ("mapped", "Mapped"),
    ("deviation", "Deviation"),
    ("disallowed", "Disallowed"),
];

/// Ein Abschnitt der Tabelle: Anfang, Index in [`STATUSES`] und Abbildung.
type Run = (u32, usize, Vec<u32>);

/// Erzeugt den Quelltext von `idna/tables.rs`. `read` liefert den Inhalt
/// einer Datei aus [`DATA`].
pub fn run(read: &dyn Fn(&str) -> Result<String, String>) -> Result<String, String> {
    let (version, _) = ucd::parse_property_value_aliases(&read("PropertyValueAliases.txt")?)?;
    generate(version, &read("IdnaMappingTable.txt")?)
}

/// Erzeugt den Quelltext aus der Tabelle. Sie muss zur Version der
/// Zeichendatenbank passen, deren Normalisierung und Eigenschaften
/// `mb-unicode` zusammen mit ihr verwendet.
pub fn generate(version: Version, table: &str) -> Result<String, String> {
    const FILE: &str = "IdnaMappingTable.txt";
    let table_version = table
        .lines()
        .find_map(|line| line.strip_prefix("# Version: "))
        .and_then(|text| Version::parse(text.trim()))
        .ok_or(format!("{FILE}: Versionszeile fehlt"))?;
    if table_version != version {
        return Err(format!(
            "{FILE} hat Version {table_version}, die Zeichendatenbank {version}"
        ));
    }
    let (missing, lines) = ucd::parse_property_file(FILE, table)?;
    if !missing.is_empty() {
        return Err(format!("{FILE}: unerwartete @missing-Zeilen"));
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut next = 0;
    for line in &lines {
        let range = format!("{:04X}..{:04X}", line.first, line.last);
        let error = |message: &str| format!("{FILE}, {range}: {message}");
        if line.first != next {
            return Err(error("Bereich schließt nicht lückenlos an"));
        }
        next = line
            .last
            .checked_add(1)
            .ok_or_else(|| error("Bereich zu groß"))?;
        let status = STATUSES
            .iter()
            .position(|&(name, _)| name == line.value())
            .ok_or_else(|| error(&format!("unbekannter Status {:?}", line.value())))?;
        let (mapping, idna2008) = match line.fields.as_slice() {
            [_] => ("", ""),
            [_, mapping] => (mapping.as_str(), ""),
            [_, mapping, idna2008] => (mapping.as_str(), idna2008.as_str()),
            _ => return Err(error("zu viele Felder")),
        };
        if !matches!(idna2008, "" | "NV8" | "XV8") {
            return Err(error(&format!("unbekannter IDNA2008-Status {idna2008:?}")));
        }
        let mapping = parse_mapping(mapping).map_err(|message| error(&message))?;
        let needs_mapping = match line.value() {
            "mapped" => Some(true),
            "deviation" => None,
            _ => Some(false),
        };
        if needs_mapping.is_some_and(|needed| needed == mapping.is_empty()) {
            return Err(error("Abbildung passt nicht zum Status"));
        }
        match runs.last() {
            Some((_, previous, previous_mapping))
                if *previous == status && *previous_mapping == mapping => {}
            _ => runs.push((line.first, status, mapping)),
        }
    }
    if next != MAX_CODE_POINT + 1 {
        return Err(format!("{FILE}: Tabelle endet vor U+10FFFF"));
    }
    emit(version, &runs).map_err(|_| "Fehler beim Schreiben des Quelltexts".to_owned())
}

/// Die Abbildung, Codepunkte durch Leerzeichen getrennt. Es müssen
/// Skalarwerte sein, damit `mb-unicode` sie als `char` ablegen kann.
fn parse_mapping(text: &str) -> Result<Vec<u32>, String> {
    text.split_whitespace()
        .map(|part| {
            u32::from_str_radix(part, 16)
                .ok()
                .filter(|&code_point| char::from_u32(code_point).is_some())
                .ok_or_else(|| format!("ungültiger Codepunkt {part:?} in der Abbildung"))
        })
        .collect()
}

fn emit(version: Version, runs: &[Run]) -> Result<String, std::fmt::Error> {
    // Gleiche Abbildungen stehen nur einmal in den Daten.
    let mut data: Vec<u32> = Vec::new();
    let mut offsets: BTreeMap<&[u32], usize> = BTreeMap::new();
    let mut entries = Vec::new();
    for (start, status, mapping) in runs {
        let offset = *offsets.entry(mapping).or_insert_with(|| {
            let offset = data.len();
            data.extend_from_slice(mapping);
            offset
        });
        let offset = u16::try_from(offset).map_err(|_| std::fmt::Error)?;
        let length = u8::try_from(mapping.len()).map_err(|_| std::fmt::Error)?;
        let name = STATUSES.get(*status).map_or("", |&(_, name)| name);
        entries.push((*start, name, offset, length));
    }
    let mut out = String::new();
    writeln!(
        out,
        "// Erzeugt mit `cargo run -p datagen` aus der IDNA-Zuordnungstabelle {version}"
    )?;
    writeln!(
        out,
        "// ({DATA}/IdnaMappingTable.txt). Nicht von Hand ändern."
    )?;
    writeln!(out, "//")?;
    writeln!(
        out,
        "// Daten: © Unicode, Inc., Unicode License V3, siehe {DATA}/LICENSE.txt."
    )?;
    writeln!(out)?;
    writeln!(out, "use super::Status;")?;
    writeln!(out, "use super::Status as S;")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Status und Abbildung jedes Codepunkts: Anfang des Abschnitts, Status,"
    )?;
    writeln!(
        out,
        "/// Anfang und Länge der Abbildung in [`MAPPING_DATA`]. Ein Abschnitt reicht"
    )?;
    writeln!(out, "/// bis vor den nächsten, der letzte bis U+10FFFF.")?;
    writeln!(
        out,
        "pub(super) static MAPPING: [(u32, Status, u16, u8); {}] = [",
        entries.len()
    )?;
    for (start, name, offset, length) in entries {
        writeln!(out, "    (0x{start:04X}, S::{name}, {offset}, {length}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(out, "/// Die Abbildungen hintereinander.")?;
    writeln!(
        out,
        "pub(super) static MAPPING_DATA: [char; {}] = [",
        data.len()
    )?;
    for chunk in data.chunks(8) {
        let chars: Vec<String> = chunk
            .iter()
            .map(|code_point| format!("'\\u{{{code_point:04X}}}'"))
            .collect();
        writeln!(out, "    {},", chars.join(", "))?;
    }
    writeln!(out, "];")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: Version = Version {
        major: 18,
        minor: 0,
        update: 0,
    };

    const TABLE: &str = "# IdnaMappingTable.txt\n\
                         # Version: 18.0.0\n\
                         0000..002C    ; valid      ;      ; NV8    # 1.1  <control-0000>..COMMA\n\
                         002D..002E    ; valid                      # 1.1  HYPHEN-MINUS..FULL STOP\n\
                         002F..0040    ; valid      ;      ; NV8    # 1.1  SOLIDUS..COMMERCIAL AT\n\
                         0041          ; mapped     ; 0061          # 1.1  LATIN CAPITAL LETTER A\n\
                         0042          ; mapped     ; 0062          # 1.1  LATIN CAPITAL LETTER B\n\
                         0043..00AC    ; valid                      # erfunden\n\
                         00AD          ; ignored                    # 1.1  SOFT HYPHEN\n\
                         00AE..00DE    ; disallowed                 # erfunden\n\
                         00DF          ; deviation  ; 0073 0073     # 1.1  LATIN SMALL LETTER SHARP S\n\
                         00E0..0131    ; valid                      # erfunden\n\
                         0132..0133    ; mapped     ; 0069 006A     # 1.1  LATIN CAPITAL LIGATURE IJ..LATIN SMALL LIGATURE IJ\n\
                         0134..200B    ; disallowed                 # erfunden\n\
                         200C..200D    ; deviation  ;               # 1.1  ZERO WIDTH NON-JOINER..ZERO WIDTH JOINER\n\
                         200E..10FFFF  ; disallowed                 # erfunden\n";

    #[test]
    fn small_table_gives_tables() {
        let source = generate(VERSION, TABLE).expect("gültige Daten");
        assert!(source.starts_with(
            "// Erzeugt mit `cargo run -p datagen` aus der IDNA-Zuordnungstabelle 18.0.0\n"
        ));
        // Die drei gültigen Bereiche am Anfang werden ein Abschnitt, die
        // Abbildung der Ligatur gilt für den ganzen Bereich.
        assert!(source.contains(
            "pub(super) static MAPPING: [(u32, Status, u16, u8); 12] = [\n    \
             (0x0000, S::Valid, 0, 0),\n    \
             (0x0041, S::Mapped, 0, 1),\n    \
             (0x0042, S::Mapped, 1, 1),\n    \
             (0x0043, S::Valid, 0, 0),\n    \
             (0x00AD, S::Ignored, 0, 0),\n    \
             (0x00AE, S::Disallowed, 0, 0),\n    \
             (0x00DF, S::Deviation, 2, 2),\n    \
             (0x00E0, S::Valid, 0, 0),\n    \
             (0x0132, S::Mapped, 4, 2),\n    \
             (0x0134, S::Disallowed, 0, 0),\n    \
             (0x200C, S::Deviation, 0, 0),\n    \
             (0x200E, S::Disallowed, 0, 0),\n];\n"
        ));
        assert!(source.contains(
            "pub(super) static MAPPING_DATA: [char; 6] = [\n    \
             '\\u{0061}', '\\u{0062}', '\\u{0073}', '\\u{0073}', '\\u{0069}', '\\u{006A}',\n];\n"
        ));
    }

    #[test]
    fn equal_mappings_are_stored_once() {
        let table = TABLE.replace(
            "0042          ; mapped     ; 0062",
            "0042          ; mapped     ; 0069 006A",
        );
        let source = generate(VERSION, &table).expect("gültige Daten");
        assert!(source.contains("(0x0042, S::Mapped, 1, 2),"));
        assert!(source.contains("(0x0132, S::Mapped, 1, 2),"));
        assert!(source.contains("[char; 5]"));
        // Benachbarte Zeilen mit gleicher Abbildung werden ein Abschnitt.
        let table = TABLE.replace(
            "0042          ; mapped     ; 0062",
            "0042          ; mapped     ; 0061",
        );
        let source = generate(VERSION, &table).expect("gültige Daten");
        assert!(source.contains("(0x0041, S::Mapped, 0, 1),\n    (0x0043, S::Valid, 0, 0),"));
    }

    #[test]
    fn broken_tables_are_errors() {
        let broken = [
            // Andere Version als die Zeichendatenbank.
            TABLE.replace("# Version: 18.0.0", "# Version: 17.0.0"),
            TABLE.replace("# Version: 18.0.0\n", ""),
            // Lücke, Überlappung, unvollständig.
            TABLE.replace("0043..00AC", "0044..00AC"),
            TABLE.replace("0043..00AC", "0042..00AC"),
            TABLE.replace("200E..10FFFF", "200E..10FFFE"),
            // Status und Abbildung passen nicht zusammen.
            TABLE.replace("; mapped     ; 0061", "; mapped     ;"),
            TABLE.replace(
                "00AD          ; ignored   ",
                "00AD          ; ignored ; 0061",
            ),
            TABLE.replace("; valid      ;      ; NV8", "; valid      ; 0061 ; NV8"),
            TABLE.replace("ignored", "unknown"),
            TABLE.replace("NV8", "XYZ"),
            TABLE.replace("; 0069 006A", "; 0069 D800"),
            TABLE.replace("; NV8    # 1.1  SOLIDUS", "; NV8 ; X # 1.1  SOLIDUS"),
            format!("# @missing: 0000..10FFFF; disallowed\n{TABLE}"),
        ];
        for table in broken {
            assert!(generate(VERSION, &table).is_err(), "{table}");
        }
    }
}
