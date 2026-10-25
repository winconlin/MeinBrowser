//! Leser für die Dateien der Unicode-Zeichendatenbank (UAX #44, Abschnitt 4.2).
//!
//! Die Dateien sind Rohdaten von außen. Fehler werden mit Datei und Zeile
//! gemeldet, nie mit einer Panic (RS-7).

/// Der größte Codepunkt.
pub const MAX_CODE_POINT: u32 = 0x10_FFFF;

/// Eine Zeile aus `UnicodeData.txt`. Bereiche (`<…, First>` und `<…, Last>`)
/// werden zu einem Eintrag zusammengefasst.
#[derive(Debug, PartialEq, Eq)]
pub struct UnicodeDataEntry {
    pub first: u32,
    pub last: u32,
    /// Kurzname der allgemeinen Kategorie, etwa `Lu`.
    pub general_category: String,
    pub canonical_combining_class: u8,
    /// Die kanonische Zerlegung eine Ebene tief, leer ohne Zerlegung oder bei
    /// einer Kompatibilitätszerlegung (`<compat> …`).
    pub canonical_decomposition: Vec<u32>,
}

/// Liest `UnicodeData.txt`. Die Einträge müssen aufsteigend sortiert sein und
/// dürfen sich nicht überlappen.
pub fn parse_unicode_data(text: &str) -> Result<Vec<UnicodeDataEntry>, String> {
    let mut entries: Vec<UnicodeDataEntry> = Vec::new();
    // Name und Zeile eines offenen `<…, First>`-Eintrags.
    let mut open_range: Option<(String, usize, UnicodeDataEntry)> = None;
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let error = |message: &str| format!("UnicodeData.txt, Zeile {number}: {message}");
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(';').collect();
        let [
            code_point,
            name,
            general_category,
            combining_class,
            _,
            decomposition,
            ..,
        ] = fields.as_slice()
        else {
            return Err(error("zu wenige Felder"));
        };
        if fields.len() != 15 {
            return Err(error(&format!("{} statt 15 Felder", fields.len())));
        }
        let code_point = parse_code_point(code_point).map_err(|message| error(&message))?;
        let canonical_combining_class = combining_class
            .parse::<u8>()
            .map_err(|_| error(&format!("ungültige Combining Class {combining_class:?}")))?;
        if general_category.is_empty() {
            return Err(error("allgemeine Kategorie fehlt"));
        }
        let canonical_decomposition = if decomposition.starts_with('<') {
            Vec::new()
        } else {
            decomposition
                .split_whitespace()
                .map(parse_code_point)
                .collect::<Result<Vec<u32>, String>>()
                .map_err(|message| error(&message))?
        };
        let mut entry = UnicodeDataEntry {
            first: code_point,
            last: code_point,
            general_category: (*general_category).to_owned(),
            canonical_combining_class,
            canonical_decomposition,
        };
        if let Some(range) = name.strip_suffix(", Last>") {
            let Some((first_name, first_line, first)) = open_range.take() else {
                return Err(error("Bereichsende ohne Anfang"));
            };
            if first_name != range {
                return Err(error(&format!(
                    "Bereichsende passt nicht zum Anfang in Zeile {first_line}"
                )));
            }
            if first.general_category != entry.general_category
                || first.canonical_combining_class != entry.canonical_combining_class
                || !first.canonical_decomposition.is_empty()
                || !entry.canonical_decomposition.is_empty()
            {
                return Err(error("Anfang und Ende des Bereichs haben andere Werte"));
            }
            entry.first = first.first;
        } else if open_range.is_some() {
            return Err(error("Bereich ohne Ende"));
        } else if let Some(range) = name.strip_suffix(", First>") {
            open_range = Some((range.to_owned(), number, entry));
            continue;
        }
        if entry.last < entry.first {
            return Err(error("Bereichsende liegt vor dem Anfang"));
        }
        if let Some(previous) = entries.last()
            && entry.first <= previous.last
        {
            return Err(error("Einträge nicht aufsteigend sortiert"));
        }
        entries.push(entry);
    }
    if let Some((_, line, _)) = open_range {
        return Err(format!("UnicodeData.txt, Zeile {line}: Bereich ohne Ende"));
    }
    Ok(entries)
}

/// Eine Datenzeile einer Eigenschaftsdatei wie `DerivedBidiClass.txt`: ein
/// Codepunkt oder Bereich und die übrigen Felder.
#[derive(Debug, PartialEq, Eq)]
pub struct PropertyLine {
    pub first: u32,
    pub last: u32,
    pub fields: Vec<String>,
}

impl PropertyLine {
    /// Der Wert im zweiten Feld, leer, wenn die Zeile nur Codepunkte nennt
    /// wie `CompositionExclusions.txt`.
    pub fn value(&self) -> &str {
        self.fields.first().map_or("", String::as_str)
    }
}

/// Die Zeilen einer Eigenschaftsdatei (UAX #44, Abschnitt 4.2): zuerst die
/// Vorgaben aus `# @missing:`-Zeilen in ihrer Reihenfolge, dann die Daten.
pub type PropertyFile = (Vec<PropertyLine>, Vec<PropertyLine>);

/// Liest eine Eigenschaftsdatei. `file` erscheint in Fehlermeldungen.
pub fn parse_property_file(file: &str, text: &str) -> Result<PropertyFile, String> {
    let mut missing = Vec::new();
    let mut data = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let error = |message: &str| format!("{file}, Zeile {}: {message}", index + 1);
        let (fields, target) = match line.trim().strip_prefix("# @missing:") {
            Some(rest) => (rest, &mut missing),
            None => (line.split('#').next().unwrap_or_default(), &mut data),
        };
        if fields.trim().is_empty() {
            continue;
        }
        let mut fields = fields.split(';').map(str::trim);
        let range = fields.next().unwrap_or_default();
        let (first, last) = match range.split_once("..") {
            Some((first, last)) => (first, last),
            None => (range, range),
        };
        let first = parse_code_point(first).map_err(|message| error(&message))?;
        let last = parse_code_point(last).map_err(|message| error(&message))?;
        if last < first {
            return Err(error("Bereichsende liegt vor dem Anfang"));
        }
        target.push(PropertyLine {
            first,
            last,
            fields: fields.map(str::to_owned).collect(),
        });
    }
    Ok((missing, data))
}

fn parse_code_point(text: &str) -> Result<u32, String> {
    let valid = (4..=6).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit());
    let value = if valid {
        u32::from_str_radix(text, 16).ok()
    } else {
        None
    };
    match value {
        Some(value) if value <= MAX_CODE_POINT => Ok(value),
        _ => Err(format!("ungültiger Codepunkt {text:?}")),
    }
}

/// Eine Zeile aus `PropertyValueAliases.txt`: Eigenschaft, Felder und der
/// Kommentar am Zeilenende (bei Gruppen wie `L` stehen dort die Mitglieder).
#[derive(Debug, PartialEq, Eq)]
pub struct ValueAlias {
    pub property: String,
    pub fields: Vec<String>,
    pub comment: Option<String>,
}

/// Liest `PropertyValueAliases.txt` und die Version aus der Kopfzeile.
pub fn parse_property_value_aliases(text: &str) -> Result<(Version, Vec<ValueAlias>), String> {
    let version = text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("# PropertyValueAliases-"))
        .and_then(|rest| rest.strip_suffix(".txt"))
        .and_then(Version::parse)
        .ok_or("PropertyValueAliases.txt: Version in der ersten Zeile fehlt")?;
    let mut aliases = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let (data, comment) = match line.split_once('#') {
            Some((data, comment)) => (data, Some(comment.trim())),
            None => (line, None),
        };
        if data.trim().is_empty() {
            continue;
        }
        let mut fields = data.split(';').map(|field| field.trim().to_owned());
        let property = fields.next().unwrap_or_default();
        let fields: Vec<String> = fields.collect();
        if property.is_empty() || fields.len() < 2 || fields.iter().any(String::is_empty) {
            return Err(format!(
                "PropertyValueAliases.txt, Zeile {}: unvollständiger Eintrag",
                index + 1
            ));
        }
        aliases.push(ValueAlias {
            property,
            fields,
            comment: comment.filter(|c| !c.is_empty()).map(str::to_owned),
        });
    }
    Ok((version, aliases))
}

/// Eine Unicode-Version wie 18.0.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub update: u8,
}

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.').map(|part| part.parse::<u8>().ok());
        let version = Version {
            major: parts.next()??,
            minor: parts.next()??,
            update: parts.next()??,
        };
        parts.next().is_none().then_some(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(code_point: &str, name: &str, gc: &str, ccc: &str) -> String {
        format!("{code_point};{name};{gc};{ccc};L;;;;;N;;;;;\n")
    }

    #[test]
    fn reads_canonical_decompositions() {
        let text = "00A8;DIAERESIS;Sk;0;ON;<compat> 0020 0308;;;;N;;;;;\n\
                    00C0;A WITH GRAVE;Lu;0;L;0041 0300;;;;N;;;;00E0;\n\
                    212B;ANGSTROM SIGN;Lu;0;L;00C5;;;;N;;;;00E5;\n";
        let entries = parse_unicode_data(text).expect("gültige Daten");
        let decompositions: Vec<&[u32]> = entries
            .iter()
            .map(|entry| entry.canonical_decomposition.as_slice())
            .collect();
        assert_eq!(decompositions, [&[][..], &[0x41, 0x300], &[0xC5]]);
        let broken = "00C0;A WITH GRAVE;Lu;0;L;0041 XYZ;;;;N;;;;00E0;\n";
        assert!(parse_unicode_data(broken).is_err());
    }

    #[test]
    fn reads_property_files() {
        let text = "# @missing: 0000..10FFFF; Left_To_Right\n\
                    # Kommentar\n\
                    \n\
                    0041..005A    ; L # Lu  [26] A..Z\n\
                    05D0 ; R\n\
                    0958    #  DEVANAGARI LETTER QA\n";
        let (missing, data) = parse_property_file("Test.txt", text).expect("gültige Daten");
        assert_eq!(
            missing,
            [PropertyLine {
                first: 0,
                last: 0x10_FFFF,
                fields: vec!["Left_To_Right".into()]
            }]
        );
        let lines: Vec<(u32, u32, &str)> = data
            .iter()
            .map(|line| (line.first, line.last, line.value()))
            .collect();
        assert_eq!(
            lines,
            [(0x41, 0x5A, "L"), (0x5D0, 0x5D0, "R"), (0x958, 0x958, "")]
        );
        for broken in ["005A..0041 ; L\n", "XYZ ; L\n", "110000 ; L\n"] {
            assert!(
                parse_property_file("Test.txt", broken).is_err(),
                "{broken:?}"
            );
        }
    }

    #[test]
    fn reads_single_code_points_and_ranges() {
        let text = [
            line("0041", "LATIN CAPITAL LETTER A", "Lu", "0"),
            line("0300", "COMBINING GRAVE ACCENT", "Mn", "230"),
            line("3400", "<CJK Ideograph Extension A, First>", "Lo", "0"),
            line("4DBF", "<CJK Ideograph Extension A, Last>", "Lo", "0"),
            line("100000", "<Plane 16 Private Use, First>", "Co", "0"),
            line("10FFFD", "<Plane 16 Private Use, Last>", "Co", "0"),
        ]
        .concat();
        let entries = parse_unicode_data(&text).expect("gültige Daten");
        let ranges: Vec<_> = entries
            .iter()
            .map(|e| {
                (
                    e.first,
                    e.last,
                    e.general_category.as_str(),
                    e.canonical_combining_class,
                )
            })
            .collect();
        assert_eq!(
            ranges,
            [
                (0x41, 0x41, "Lu", 0),
                (0x300, 0x300, "Mn", 230),
                (0x3400, 0x4DBF, "Lo", 0),
                (0x10_0000, 0x10_FFFD, "Co", 0),
            ]
        );
    }

    #[test]
    fn accepts_windows_line_endings() {
        let text = line("0041", "A", "Lu", "0").replace('\n', "\r\n");
        assert_eq!(parse_unicode_data(&text).map(|e| e.len()), Ok(1));
    }

    #[test]
    fn rejects_broken_unicode_data() {
        let broken = [
            line("XYZ", "A", "Lu", "0"),
            line("110000", "A", "Lu", "0"),
            line("0041", "A", "Lu", "300"),
            line("0041", "A", "", "0"),
            "0041;A;Lu\n".to_owned(),
            "0041;A;Lu;0;L;;;;;N;;;;;;\n".to_owned(),
            line("0042", "B", "Lu", "0") + &line("0041", "A", "Lu", "0"),
            line("0041", "A", "Lu", "0") + &line("0041", "A", "Lu", "0"),
            line("3400", "<X, First>", "Lo", "0"),
            line("3400", "<X, Last>", "Lo", "0"),
            line("3400", "<X, First>", "Lo", "0") + &line("4DBF", "<Y, Last>", "Lo", "0"),
            line("3400", "<X, First>", "Lo", "0") + &line("4DBF", "<X, Last>", "Lu", "0"),
            line("3400", "<X, First>", "Lo", "0") + &line("3500", "A", "Lo", "0"),
            line("4DBF", "<X, First>", "Lo", "0") + &line("3400", "<X, Last>", "Lo", "0"),
        ];
        for text in broken {
            assert!(parse_unicode_data(&text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn reads_value_aliases_with_groups() {
        let text = "# PropertyValueAliases-18.0.0.txt\n\
                    # Kommentar\n\
                    \n\
                    gc ; Cc ; Control ; cntrl\n\
                    gc ; L  ; Letter  # Ll | Lm | Lo | Lt | Lu\n\
                    ccc;   0; NR ; Not_Reordered\n";
        let (version, aliases) = parse_property_value_aliases(text).expect("gültige Daten");
        assert_eq!(
            version,
            Version {
                major: 18,
                minor: 0,
                update: 0
            }
        );
        assert_eq!(version.to_string(), "18.0.0");
        assert_eq!(aliases.len(), 3);
        assert_eq!(aliases[0].fields, ["Cc", "Control", "cntrl"]);
        assert_eq!(aliases[0].comment, None);
        assert_eq!(aliases[1].fields, ["L", "Letter"]);
        assert_eq!(
            aliases[1].comment.as_deref(),
            Some("Ll | Lm | Lo | Lt | Lu")
        );
        assert_eq!(aliases[2].property, "ccc");
        assert_eq!(aliases[2].fields, ["0", "NR", "Not_Reordered"]);
    }

    #[test]
    fn rejects_broken_value_aliases() {
        for text in [
            "gc ; Cc ; Control\n",
            "# PropertyValueAliases-18.0.txt\ngc ; Cc ; Control\n",
            "# PropertyValueAliases-18.0.0.txt\ngc ; Cc\n",
            "# PropertyValueAliases-18.0.0.txt\ngc ; ; Control\n",
        ] {
            assert!(parse_property_value_aliases(text).is_err(), "{text:?}");
        }
    }
}
