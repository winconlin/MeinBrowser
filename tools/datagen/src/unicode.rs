//! Die Unicode-Tabellen für `mb-unicode`: allgemeine Kategorie, kanonische
//! Kombinationsklasse, kanonische Zerlegungen und Kompatibilitätszerlegungen
//! aus `UnicodeData.txt`, Ausnahmen der Komposition aus
//! `CompositionExclusions.txt`, Bidi-Klasse und Joining-Type aus den
//! abgeleiteten Dateien, Zeilenumbruchklasse aus `LineBreak.txt`,
//! East_Asian_Width aus `EastAsianWidth.txt`, Extended_Pictographic aus
//! `emoji-data.txt`, ID_Start, ID_Continue, Cased und Case_Ignorable aus
//! `DerivedCoreProperties.txt`, die Faltung der Groß- und Kleinschreibung
//! aus `CaseFolding.txt`, die Abbildungen auf Klein- und Großbuchstaben aus
//! `UnicodeData.txt` und `SpecialCasing.txt`, Namen der Werte aus
//! `PropertyValueAliases.txt`, dort auch alle Namen der Werte von
//! General_Category und Script.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::ucd::{self, MAX_CODE_POINT, PropertyLine, ValueAlias, Version};

/// Die Rohdaten, relativ zum Workspace.
pub const DATA: &str = "data/unicode";

/// Die erzeugte Datei, relativ zum Workspace.
pub const OUTPUT: &str = "crates/mb-unicode/src/tables.rs";

/// Wie tief kanonische Zerlegungen höchstens verschachtelt sein dürfen.
const MAX_DECOMPOSITION_DEPTH: usize = 8;

/// Ein Wert einer Eigenschaft, etwa `Lu` mit dem Langnamen `Uppercase_Letter`.
struct PropertyValue {
    short: String,
    long: String,
}

/// Eine Gruppe von Kategorien, etwa `L` (`Letter`) mit `Ll | Lm | Lo | Lt | Lu`.
struct Group {
    short: String,
    long: String,
    members: Vec<usize>,
}

/// Die Inhalte der Rohdaten.
pub struct Sources<'a> {
    pub unicode_data: &'a str,
    pub value_aliases: &'a str,
    pub composition_exclusions: &'a str,
    pub bidi_classes: &'a str,
    pub joining_types: &'a str,
    pub line_breaks: &'a str,
    pub east_asian_widths: &'a str,
    pub emoji_data: &'a str,
    pub derived_core_properties: &'a str,
    pub case_folding: &'a str,
    pub special_casing: &'a str,
}

/// Eine Eigenschaft mit Aufzählungswerten, wie sie in `tables.rs` landet.
struct Enumeration<'a> {
    name: &'a str,
    alias: &'a str,
    doc: &'a str,
    example: (&'a str, &'a str),
    values: &'a [PropertyValue],
}

/// Erzeugt den Quelltext von `tables.rs`. `read` liefert den Inhalt einer
/// Datei aus [`DATA`].
pub fn run(read: &dyn Fn(&str) -> Result<String, String>) -> Result<String, String> {
    generate(&Sources {
        unicode_data: &read("UnicodeData.txt")?,
        value_aliases: &read("PropertyValueAliases.txt")?,
        composition_exclusions: &read("CompositionExclusions.txt")?,
        bidi_classes: &read("DerivedBidiClass.txt")?,
        joining_types: &read("DerivedJoiningType.txt")?,
        line_breaks: &read("LineBreak.txt")?,
        east_asian_widths: &read("EastAsianWidth.txt")?,
        emoji_data: &read("emoji-data.txt")?,
        derived_core_properties: &read("DerivedCoreProperties.txt")?,
        case_folding: &read("CaseFolding.txt")?,
        special_casing: &read("SpecialCasing.txt")?,
    })
}

/// Erzeugt den Quelltext von `tables.rs` aus den Rohdaten.
pub fn generate(sources: &Sources<'_>) -> Result<String, String> {
    let entries = ucd::parse_unicode_data(sources.unicode_data)?;
    let (version, aliases) = ucd::parse_property_value_aliases(sources.value_aliases)?;
    let (categories, groups) = general_categories(&aliases)?;
    let bidi_values = property_values(&aliases, "bc")?;
    let joining_values = property_values(&aliases, "jt")?;
    let line_break_values = property_values(&aliases, "lb")?;
    let east_asian_values = property_values(&aliases, "ea")?;
    let category_names = value_names(&aliases, "gc")?;
    let script_names = value_names(&aliases, "sc")?;
    let unassigned = find(&categories, "Cn")?;
    let count = index(MAX_CODE_POINT)? + 1;
    let mut category_of = vec![unassigned; count];
    let mut class_of = vec![0u8; count];
    let mut decompositions = BTreeMap::new();
    let mut compatibility = BTreeMap::new();
    let mut simple_lowercase = BTreeMap::new();
    let mut simple_uppercase = BTreeMap::new();
    for entry in &entries {
        let category = find(&categories, &entry.general_category)?;
        let range = index(entry.first)?..=index(entry.last)?;
        category_of
            .get_mut(range.clone())
            .ok_or("Bereich außerhalb des Coderaums")?
            .fill(category);
        class_of
            .get_mut(range)
            .ok_or("Bereich außerhalb des Coderaums")?
            .fill(entry.canonical_combining_class);
        if !entry.canonical_decomposition.is_empty() {
            decompositions.insert(entry.first, entry.canonical_decomposition.clone());
        }
        if !entry.compatibility_decomposition.is_empty() {
            compatibility.insert(entry.first, entry.compatibility_decomposition.clone());
        }
        if let Some(lowercase) = entry.simple_lowercase {
            simple_lowercase.insert(entry.first, lowercase);
        }
        if let Some(uppercase) = entry.simple_uppercase {
            simple_uppercase.insert(entry.first, uppercase);
        }
    }
    let (_, exclusions) =
        ucd::parse_property_file("CompositionExclusions.txt", sources.composition_exclusions)?;
    let compositions = compositions(&decompositions, &exclusions, &class_of)?;
    let compatibility = compatibility_decompositions(&decompositions, &compatibility)?;
    let decompositions = full_decompositions(&decompositions)?;
    let (lowercase, uppercase) =
        case_mappings(&simple_lowercase, &simple_uppercase, sources.special_casing)?;
    let bidi_of = fill(
        "DerivedBidiClass.txt",
        sources.bidi_classes,
        &bidi_values,
        count,
    )?;
    let joining_of = fill(
        "DerivedJoiningType.txt",
        sources.joining_types,
        &joining_values,
        count,
    )?;
    let line_break_of = fill(
        "LineBreak.txt",
        sources.line_breaks,
        &line_break_values,
        count,
    )?;
    let east_asian_of = fill(
        "EastAsianWidth.txt",
        sources.east_asian_widths,
        &east_asian_values,
        count,
    )?;
    let pictographic_of = flags(
        "emoji-data.txt",
        sources.emoji_data,
        "Extended_Pictographic",
        count,
    )?;
    let id_start_of = flags(
        "DerivedCoreProperties.txt",
        sources.derived_core_properties,
        "ID_Start",
        count,
    )?;
    let id_continue_of = flags(
        "DerivedCoreProperties.txt",
        sources.derived_core_properties,
        "ID_Continue",
        count,
    )?;
    let cased_of = flags(
        "DerivedCoreProperties.txt",
        sources.derived_core_properties,
        "Cased",
        count,
    )?;
    let case_ignorable_of = flags(
        "DerivedCoreProperties.txt",
        sources.derived_core_properties,
        "Case_Ignorable",
        count,
    )?;
    let tables = Tables {
        general: runs(&category_of),
        classes: runs(&class_of),
        bidi: runs(&bidi_of),
        joining: runs(&joining_of),
        line_break: runs(&line_break_of),
        east_asian: runs(&east_asian_of),
        pictographic: runs(&pictographic_of),
        id_start: runs(&id_start_of),
        id_continue: runs(&id_continue_of),
        cased: runs(&cased_of),
        case_ignorable: runs(&case_ignorable_of),
        decompositions,
        compatibility,
        compositions,
        case_folding: case_folding(sources.case_folding)?,
        lowercase,
        uppercase,
        category_names,
        script_names,
    };
    emit(
        version,
        [
            Enumeration {
                name: "GeneralCategory",
                alias: "G",
                doc: "Allgemeine Kategorie eines Codepunkts (General_Category, UAX #44).",
                example: ("Lu", "Uppercase_Letter"),
                values: &categories,
            },
            Enumeration {
                name: "BidiClass",
                alias: "B",
                doc: "Bidi-Klasse eines Codepunkts (Bidi_Class, UAX #9).",
                example: ("AL", "Arabic_Letter"),
                values: &bidi_values,
            },
            Enumeration {
                name: "JoiningType",
                alias: "J",
                doc: "Wie ein Zeichen in Schriften wie Arabisch verbunden wird (Joining_Type).",
                example: ("D", "Dual_Joining"),
                values: &joining_values,
            },
            Enumeration {
                name: "LineBreak",
                alias: "L",
                doc: "Zeilenumbruchklasse eines Codepunkts (Line_Break, UAX #14).",
                example: ("AL", "Alphabetic"),
                values: &line_break_values,
            },
            Enumeration {
                name: "EastAsianWidth",
                alias: "E",
                doc: "Breite eines Codepunkts in ostasiatischen Texten (East_Asian_Width, UAX #11).",
                example: ("W", "Wide"),
                values: &east_asian_values,
            },
        ],
        &groups,
        &tables,
    )
    .map_err(|_| "Fehler beim Schreiben des Quelltexts".to_owned())
}

/// Die Werte einer Eigenschaft in der Reihenfolge der Datei.
fn property_values(aliases: &[ValueAlias], property: &str) -> Result<Vec<PropertyValue>, String> {
    let mut values = Vec::new();
    for alias in aliases.iter().filter(|alias| alias.property == property) {
        let [short, long, ..] = alias.fields.as_slice() else {
            return Err(format!(
                "{property}-Eintrag ohne Langnamen: {:?}",
                alias.fields
            ));
        };
        values.push(PropertyValue {
            short: short.clone(),
            long: long.clone(),
        });
    }
    if values.is_empty() {
        return Err(format!(
            "PropertyValueAliases.txt enthält keine Werte für {property}"
        ));
    }
    Ok(values)
}

/// Alle Namen der Werte einer Eigenschaft (Kurzname, Langname und weitere
/// Aliasse), nach Namen sortiert, mit dem Kurznamen ihres Werts.
fn value_names(aliases: &[ValueAlias], property: &str) -> Result<BTreeMap<String, String>, String> {
    let mut names = BTreeMap::new();
    for alias in aliases.iter().filter(|alias| alias.property == property) {
        let Some(short) = alias.fields.first() else {
            continue;
        };
        for name in &alias.fields {
            if let Some(other) = names.insert(name.clone(), short.clone())
                && other != *short
            {
                return Err(format!(
                    "{property}: der Name {name:?} gehört zu {other} und zu {short}"
                ));
            }
        }
    }
    if names.is_empty() {
        return Err(format!(
            "PropertyValueAliases.txt enthält keine Werte für {property}"
        ));
    }
    Ok(names)
}

/// Die Kategorien und Gruppen in der Reihenfolge der Datei. Gruppen erkennt
/// man am Kommentar mit ihren Mitgliedern.
fn general_categories(aliases: &[ValueAlias]) -> Result<(Vec<PropertyValue>, Vec<Group>), String> {
    let mut categories = Vec::new();
    let mut members = Vec::new();
    for alias in aliases.iter().filter(|alias| alias.property == "gc") {
        let [short, long, ..] = alias.fields.as_slice() else {
            return Err(format!("gc-Eintrag ohne Langnamen: {:?}", alias.fields));
        };
        match alias.comment.as_deref().and_then(group_members) {
            Some(list) => members.push((short, long, list)),
            None => categories.push(PropertyValue {
                short: short.clone(),
                long: long.clone(),
            }),
        }
    }
    if categories.is_empty() {
        return Err("PropertyValueAliases.txt enthält keine allgemeinen Kategorien".into());
    }
    let mut groups = Vec::new();
    for (short, long, list) in members {
        let members = list
            .into_iter()
            .map(|member| find(&categories, member))
            .collect::<Result<Vec<_>, _>>()?;
        groups.push(Group {
            short: short.clone(),
            long: long.clone(),
            members,
        });
    }
    Ok((categories, groups))
}

/// Die Mitglieder einer Gruppe aus dem Kommentar, etwa `Ll | Lm | Lo | Lt | Lu`.
/// Andere Kommentare ergeben `None`.
fn group_members(comment: &str) -> Option<Vec<&str>> {
    let members: Vec<&str> = comment.split('|').map(str::trim).collect();
    members
        .iter()
        .all(|member| {
            matches!(member.as_bytes(), [first, second]
                if first.is_ascii_uppercase() && second.is_ascii_lowercase())
        })
        .then_some(members)
}

/// Sucht einen Wert über seinen Kurz- oder Langnamen.
fn find(values: &[PropertyValue], name: &str) -> Result<usize, String> {
    values
        .iter()
        .position(|value| value.short == name || value.long == name)
        .ok_or_else(|| format!("unbekannter Eigenschaftswert {name:?}"))
}

fn index(code_point: u32) -> Result<usize, String> {
    usize::try_from(code_point).map_err(|_| format!("Codepunkt {code_point:#X} passt nicht"))
}

/// Liest eine abgeleitete Eigenschaftsdatei: erst die Vorgaben aus den
/// `@missing`-Zeilen (spätere überschreiben frühere), dann die Daten. Jeder
/// Codepunkt muss danach einen Wert haben.
fn fill(
    file: &str,
    text: &str,
    values: &[PropertyValue],
    count: usize,
) -> Result<Vec<usize>, String> {
    let (missing, data) = ucd::parse_property_file(file, text)?;
    let mut value_of: Vec<Option<usize>> = vec![None; count];
    for line in missing.iter().chain(&data) {
        let value = find(values, line.value()).map_err(|message| format!("{file}: {message}"))?;
        value_of
            .get_mut(index(line.first)?..=index(line.last)?)
            .ok_or_else(|| format!("{file}: Bereich außerhalb des Coderaums"))?
            .fill(Some(value));
    }
    value_of
        .into_iter()
        .zip(0u32..)
        .map(|(value, code_point)| {
            value.ok_or_else(|| format!("{file}: kein Wert für U+{code_point:04X}"))
        })
        .collect()
}

/// Liest eine binäre Eigenschaft aus einer Datei, die mehrere Eigenschaften
/// nennt, wie `emoji-data.txt`. Nicht genannte Codepunkte haben sie nicht.
fn flags(file: &str, text: &str, property: &str, count: usize) -> Result<Vec<bool>, String> {
    let (_, data) = ucd::parse_property_file(file, text)?;
    let mut flag_of = vec![false; count];
    let mut found = false;
    for line in data.iter().filter(|line| line.value() == property) {
        flag_of
            .get_mut(index(line.first)?..=index(line.last)?)
            .ok_or_else(|| format!("{file}: Bereich außerhalb des Coderaums"))?
            .fill(true);
        found = true;
    }
    if !found {
        return Err(format!("{file} nennt {property} nicht"));
    }
    Ok(flag_of)
}

/// Die vollständige Faltung aus `CaseFolding.txt`: die Einträge mit Status
/// `C` und `F`. Die einfachen (`S`) und die türkischen (`T`) bleiben weg.
fn case_folding(text: &str) -> Result<BTreeMap<u32, Vec<u32>>, String> {
    let file = "CaseFolding.txt";
    let (_, data) = ucd::parse_property_file(file, text)?;
    let mut folding = BTreeMap::new();
    for line in &data {
        let code_point = line.first;
        let [status, mapping, ..] = line.fields.as_slice() else {
            return Err(format!("{file}: U+{code_point:04X} ohne Abbildung"));
        };
        if line.last != code_point {
            return Err(format!(
                "{file}: unerwarteter Bereich ab U+{code_point:04X}"
            ));
        }
        match status.as_str() {
            "C" | "F" => {}
            "S" | "T" => continue,
            _ => return Err(format!("{file}: unbekannter Status {status:?}")),
        }
        let mapping = mapping
            .split_whitespace()
            .map(ucd::parse_code_point)
            .collect::<Result<Vec<u32>, String>>()
            .map_err(|message| format!("{file}: {message}"))?;
        if mapping.is_empty() {
            return Err(format!("{file}: leere Abbildung für U+{code_point:04X}"));
        }
        if folding.insert(code_point, mapping).is_some() {
            return Err(format!("{file}: U+{code_point:04X} doppelt"));
        }
    }
    if folding.is_empty() {
        return Err(format!("{file} enthält keine Faltung"));
    }
    Ok(folding)
}

/// Die Abbildungen auf Klein- und Großbuchstaben für die Default Case
/// Conversion (Unicode-Standard, Abschnitt 3.13): die Abbildungen ohne
/// Bedingung aus `SpecialCasing.txt`, sonst Simple_Lowercase_Mapping und
/// Simple_Uppercase_Mapping aus `UnicodeData.txt`. Abbildungen auf das
/// Zeichen selbst bleiben weg. Die Zeilen mit einer Sprache gelten nur dort
/// und bleiben weg; die einzige Bedingung ohne Sprache ist Final_Sigma für
/// U+03A3, die `mb-unicode` selbst prüft.
fn case_mappings(
    simple_lowercase: &BTreeMap<u32, u32>,
    simple_uppercase: &BTreeMap<u32, u32>,
    special_casing: &str,
) -> Result<(CaseMapping, CaseMapping), String> {
    let file = "SpecialCasing.txt";
    let (_, data) = ucd::parse_property_file(file, special_casing)?;
    let single = |simple: &BTreeMap<u32, u32>| -> CaseMapping {
        simple
            .iter()
            .map(|(&code_point, &mapping)| (code_point, vec![mapping]))
            .collect()
    };
    let mut lowercase = single(simple_lowercase);
    let mut uppercase = single(simple_uppercase);
    let mut final_sigma = false;
    let mut unconditional = BTreeSet::new();
    for line in &data {
        let code_point = line.first;
        let [lower, _, upper, rest @ ..] = line.fields.as_slice() else {
            return Err(format!("{file}: U+{code_point:04X} ohne Abbildungen"));
        };
        if line.last != code_point {
            return Err(format!(
                "{file}: unerwarteter Bereich ab U+{code_point:04X}"
            ));
        }
        let parse = |mapping: &str| {
            mapping
                .split_whitespace()
                .map(ucd::parse_code_point)
                .collect::<Result<Vec<u32>, String>>()
                .map_err(|message| format!("{file}: {message}"))
        };
        let condition = rest.first().map_or("", String::as_str);
        let language = condition
            .split_whitespace()
            .next()
            .is_some_and(|first| first.bytes().all(|byte| byte.is_ascii_lowercase()));
        if language {
            continue;
        }
        if condition == "Final_Sigma" {
            // `mb-unicode` kennt genau diese Zeile.
            if code_point != 0x03A3 || parse(lower)? != [0x03C2] || final_sigma {
                return Err(format!(
                    "{file}: unerwartete Zeile mit Final_Sigma für U+{code_point:04X}"
                ));
            }
            final_sigma = true;
            continue;
        }
        if !condition.is_empty() {
            return Err(format!("{file}: unbekannte Bedingung {condition:?}"));
        }
        if !unconditional.insert(code_point) {
            return Err(format!("{file}: U+{code_point:04X} doppelt"));
        }
        for (mapping, target) in [(lower, &mut lowercase), (upper, &mut uppercase)] {
            let mapping = parse(mapping)?;
            if mapping.is_empty() {
                return Err(format!("{file}: leere Abbildung für U+{code_point:04X}"));
            }
            if mapping == [code_point] {
                target.remove(&code_point);
            } else {
                target.insert(code_point, mapping);
            }
        }
    }
    if !final_sigma {
        return Err(format!("{file} nennt Final_Sigma nicht"));
    }
    for mapping in [&mut lowercase, &mut uppercase] {
        mapping.retain(|&code_point, mapping| *mapping != [code_point]);
    }
    Ok((lowercase, uppercase))
}

/// Die vollständigen Kompatibilitätszerlegungen für NFKD: Jede Zerlegung,
/// kanonisch oder nicht, wird so lange weiter zerlegt, bis kein Teil mehr
/// eine hat. Die Tabelle nennt nur die Zeichen, bei denen das Ergebnis von
/// der vollständigen kanonischen Zerlegung abweicht.
fn compatibility_decompositions(
    canonical: &BTreeMap<u32, Vec<u32>>,
    compatibility: &BTreeMap<u32, Vec<u32>>,
) -> Result<BTreeMap<u32, Vec<u32>>, String> {
    if let Some(code_point) = compatibility
        .keys()
        .find(|code_point| canonical.contains_key(code_point))
    {
        return Err(format!("U+{code_point:04X} hat zwei Zerlegungen"));
    }
    let both: BTreeMap<u32, Vec<u32>> = canonical
        .iter()
        .chain(compatibility)
        .map(|(&code_point, decomposition)| (code_point, decomposition.clone()))
        .collect();
    let with_compatibility = full_decompositions(&both)?;
    let without = full_decompositions(canonical)?;
    Ok(with_compatibility
        .into_iter()
        .filter(|(code_point, decomposition)| without.get(code_point) != Some(decomposition))
        .collect())
}

/// Die vollständigen kanonischen Zerlegungen: Jede Zerlegung wird so lange
/// weiter zerlegt, bis kein Teil mehr eine Zerlegung hat.
fn full_decompositions(
    decompositions: &BTreeMap<u32, Vec<u32>>,
) -> Result<BTreeMap<u32, Vec<u32>>, String> {
    let mut full = BTreeMap::new();
    for (&code_point, decomposition) in decompositions {
        let mut current = decomposition.clone();
        let mut depth = 0;
        while current.iter().any(|part| decompositions.contains_key(part)) {
            depth += 1;
            if depth > MAX_DECOMPOSITION_DEPTH {
                return Err(format!("Zerlegung von U+{code_point:04X} ist zu tief"));
            }
            current = current
                .iter()
                .flat_map(|part| {
                    decompositions
                        .get(part)
                        .cloned()
                        .unwrap_or_else(|| vec![*part])
                })
                .collect();
        }
        full.insert(code_point, current);
    }
    Ok(full)
}

/// Die primären Komposita (UAX #15): alle Zerlegungen in genau zwei
/// Codepunkte, außer denen mit Full_Composition_Exclusion. Das sind die
/// Einträge in `CompositionExclusions.txt`, Zerlegungen in nur einen
/// Codepunkt und solche, die mit einem Zeichen der Klasse ungleich 0 beginnen
/// oder deren Zeichen selbst eine Klasse ungleich 0 hat.
fn compositions(
    decompositions: &BTreeMap<u32, Vec<u32>>,
    exclusions: &[PropertyLine],
    class_of: &[u8],
) -> Result<Vec<(u32, u32, u32)>, String> {
    let class = |code_point: u32| -> Result<u8, String> {
        class_of
            .get(index(code_point)?)
            .copied()
            .ok_or_else(|| format!("U+{code_point:04X} außerhalb des Coderaums"))
    };
    let mut excluded = BTreeSet::new();
    for line in exclusions {
        excluded.extend(line.first..=line.last);
    }
    let mut pairs = BTreeMap::new();
    for (&code_point, decomposition) in decompositions {
        let &[first, second] = decomposition.as_slice() else {
            continue;
        };
        if excluded.contains(&code_point) || class(code_point)? != 0 || class(first)? != 0 {
            continue;
        }
        if pairs.insert((first, second), code_point).is_some() {
            return Err(format!(
                "U+{first:04X} U+{second:04X} ergeben mehr als ein Kompositum"
            ));
        }
    }
    Ok(pairs
        .into_iter()
        .map(|((first, second), composite)| (first, second, composite))
        .collect())
}

/// Fasst gleiche Werte zu Abschnitten zusammen: Anfang und Wert.
fn runs<T: Copy + PartialEq>(values: &[T]) -> Vec<(u32, T)> {
    let mut runs: Vec<(u32, T)> = Vec::new();
    for (code_point, &value) in (0u32..).zip(values) {
        if runs.last().is_none_or(|&(_, previous)| previous != value) {
            runs.push((code_point, value));
        }
    }
    runs
}

/// `Uppercase_Letter` wird `UppercaseLetter`.
fn variant(long: &str) -> String {
    long.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect()
}

/// `Cased_Letter` wird `is_cased_letter`.
fn predicate(long: &str) -> String {
    format!("is_{}", long.to_lowercase())
}

/// Eine Abbildung von Codepunkten auf Folgen von Codepunkten.
type CaseMapping = BTreeMap<u32, Vec<u32>>;

/// Die Tabellen, die `emit` schreibt.
struct Tables {
    general: Vec<(u32, usize)>,
    classes: Vec<(u32, u8)>,
    bidi: Vec<(u32, usize)>,
    joining: Vec<(u32, usize)>,
    line_break: Vec<(u32, usize)>,
    east_asian: Vec<(u32, usize)>,
    pictographic: Vec<(u32, bool)>,
    id_start: Vec<(u32, bool)>,
    id_continue: Vec<(u32, bool)>,
    cased: Vec<(u32, bool)>,
    case_ignorable: Vec<(u32, bool)>,
    decompositions: BTreeMap<u32, Vec<u32>>,
    compatibility: BTreeMap<u32, Vec<u32>>,
    compositions: Vec<(u32, u32, u32)>,
    case_folding: BTreeMap<u32, Vec<u32>>,
    lowercase: CaseMapping,
    uppercase: CaseMapping,
    category_names: BTreeMap<String, String>,
    script_names: BTreeMap<String, String>,
}

fn emit(
    version: Version,
    enumerations: [Enumeration<'_>; 5],
    groups: &[Group],
    tables: &Tables,
) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    writeln!(
        out,
        "// Erzeugt mit `cargo run -p datagen` aus der Unicode-Zeichendatenbank {version}"
    )?;
    writeln!(out, "// ({DATA}). Nicht von Hand ändern.")?;
    writeln!(out, "//")?;
    writeln!(
        out,
        "// Daten: © Unicode, Inc., Unicode License V3, siehe {DATA}/LICENSE.txt."
    )?;
    writeln!(out)?;
    for enumeration in &enumerations {
        writeln!(out, "use {} as {};", enumeration.name, enumeration.alias)?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "/// Version der Unicode-Zeichendatenbank, aus der die Tabellen stammen."
    )?;
    writeln!(
        out,
        "pub const UNICODE_VERSION: (u8, u8, u8) = ({}, {}, {});",
        version.major, version.minor, version.update
    )?;
    let [general, bidi, joining, line_break, east_asian] = &enumerations;
    emit_enumeration(&mut out, general)?;
    let categories = general.values;
    for group in groups {
        let members: Vec<&PropertyValue> = group
            .members
            .iter()
            .filter_map(|&member| categories.get(member))
            .collect();
        let shorts: Vec<&str> = members.iter().map(|c| c.short.as_str()).collect();
        let variants: Vec<String> = members
            .iter()
            .map(|c| format!("G::{}", variant(&c.long)))
            .collect();
        writeln!(out)?;
        writeln!(
            out,
            "    /// Gruppe `{}` ({}): {}.",
            group.short,
            group.long,
            shorts.join(", ")
        )?;
        writeln!(
            out,
            "    pub const fn {}(self) -> bool {{",
            predicate(&group.long)
        )?;
        writeln!(out, "        matches!(self, {})", variants.join(" | "))?;
        writeln!(out, "    }}")?;
    }
    writeln!(out, "}}")?;
    emit_enumeration(&mut out, bidi)?;
    writeln!(out, "}}")?;
    for enumeration in [joining, line_break, east_asian] {
        emit_enumeration(&mut out, enumeration)?;
        writeln!(out, "}}")?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "/// Allgemeine Kategorie: Anfang jedes Abschnitts und seine Kategorie. Ein"
    )?;
    writeln!(
        out,
        "/// Abschnitt reicht bis vor den nächsten, der letzte bis U+10FFFF."
    )?;
    emit_enumeration_runs(&mut out, "GENERAL_CATEGORY", general, &tables.general)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Kanonische Kombinationsklasse (Canonical_Combining_Class): Anfang jedes"
    )?;
    writeln!(out, "/// Abschnitts und seine Klasse, aufgebaut wie oben.")?;
    writeln!(
        out,
        "pub static CANONICAL_COMBINING_CLASS: [(u32, u8); {}] = [",
        tables.classes.len()
    )?;
    for &(start, class) in &tables.classes {
        writeln!(out, "    (0x{start:04X}, {class}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Bidi-Klasse mit den Vorgaben für nicht vergebene Codepunkte, aufgebaut"
    )?;
    writeln!(out, "/// wie oben.")?;
    emit_enumeration_runs(&mut out, "BIDI_CLASS", bidi, &tables.bidi)?;
    writeln!(out)?;
    writeln!(out, "/// Joining-Type, aufgebaut wie oben.")?;
    emit_enumeration_runs(&mut out, "JOINING_TYPE", joining, &tables.joining)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Zeilenumbruchklasse mit den Vorgaben für nicht vergebene Codepunkte,"
    )?;
    writeln!(out, "/// aufgebaut wie oben.")?;
    emit_enumeration_runs(&mut out, "LINE_BREAK", line_break, &tables.line_break)?;
    writeln!(out)?;
    writeln!(out, "/// East_Asian_Width, aufgebaut wie oben.")?;
    emit_enumeration_runs(&mut out, "EAST_ASIAN_WIDTH", east_asian, &tables.east_asian)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Alle Namen der Werte von General_Category aus `PropertyValueAliases.txt`,"
    )?;
    writeln!(
        out,
        "/// auch der Gruppen und der weiteren Aliasse, nach Namen sortiert, mit dem"
    )?;
    writeln!(out, "/// Kurznamen ihres Werts.")?;
    emit_names(&mut out, "GENERAL_CATEGORY_NAMES", &tables.category_names)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Alle Namen der Werte von Script, aufgebaut wie oben."
    )?;
    emit_names(&mut out, "SCRIPT_NAMES", &tables.script_names)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Extended_Pictographic (UTS #51): Anfang jedes Abschnitts und ob seine"
    )?;
    writeln!(out, "/// Codepunkte die Eigenschaft haben.")?;
    writeln!(
        out,
        "pub static EXTENDED_PICTOGRAPHIC: [(u32, bool); {}] = [",
        tables.pictographic.len()
    )?;
    for &(start, flag) in &tables.pictographic {
        writeln!(out, "    (0x{start:04X}, {flag}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// ID_Start (UAX #31): Anfang jedes Abschnitts und ob seine Codepunkte"
    )?;
    writeln!(out, "/// die Eigenschaft haben.")?;
    emit_flags(&mut out, "ID_START", &tables.id_start)?;
    writeln!(out)?;
    writeln!(out, "/// ID_Continue (UAX #31), aufgebaut wie oben.")?;
    emit_flags(&mut out, "ID_CONTINUE", &tables.id_continue)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Vollständige kanonische Zerlegung jedes zerlegbaren Zeichens, nach"
    )?;
    writeln!(
        out,
        "/// Codepunkt sortiert. Hangul-Silben fehlen, sie werden berechnet."
    )?;
    emit_sequences(&mut out, "CANONICAL_DECOMPOSITION", &tables.decompositions)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Vollständige Kompatibilitätszerlegung (NFKD) jedes Zeichens, bei dem sie"
    )?;
    writeln!(
        out,
        "/// von der vollständigen kanonischen Zerlegung abweicht, nach Codepunkt"
    )?;
    writeln!(out, "/// sortiert.")?;
    emit_sequences(
        &mut out,
        "COMPATIBILITY_DECOMPOSITION",
        &tables.compatibility,
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Primäre Komposita: zwei Codepunkte und das Zeichen, zu dem sie sich"
    )?;
    writeln!(
        out,
        "/// verbinden, nach den beiden Codepunkten sortiert. Hangul fehlt."
    )?;
    writeln!(
        out,
        "pub static CANONICAL_COMPOSITION: [(u32, u32, u32); {}] = [",
        tables.compositions.len()
    )?;
    for &(first, second, composite) in &tables.compositions {
        writeln!(
            out,
            "    (0x{first:04X}, 0x{second:04X}, 0x{composite:04X}),"
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Vollständige Faltung der Groß- und Kleinschreibung (Status C und F in"
    )?;
    writeln!(
        out,
        "/// `CaseFolding.txt`), nach Codepunkt sortiert. Alle anderen Codepunkte"
    )?;
    writeln!(out, "/// bleiben, wie sie sind.")?;
    emit_sequences(&mut out, "CASE_FOLDING", &tables.case_folding)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Abbildung auf Kleinbuchstaben für die Default Case Conversion: die"
    )?;
    writeln!(
        out,
        "/// Abbildungen ohne Bedingung aus `SpecialCasing.txt`, sonst"
    )?;
    writeln!(
        out,
        "/// Simple_Lowercase_Mapping, nach Codepunkt sortiert. Alle anderen"
    )?;
    writeln!(
        out,
        "/// Codepunkte bleiben, wie sie sind. Final_Sigma prüft `case_mapping`."
    )?;
    emit_sequences(&mut out, "LOWERCASE", &tables.lowercase)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Abbildung auf Großbuchstaben, aufgebaut wie oben mit"
    )?;
    writeln!(out, "/// Simple_Uppercase_Mapping.")?;
    emit_sequences(&mut out, "UPPERCASE", &tables.uppercase)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Cased (Unicode-Standard, D135): Anfang jedes Abschnitts und ob seine"
    )?;
    writeln!(out, "/// Codepunkte die Eigenschaft haben.")?;
    emit_flags(&mut out, "CASED", &tables.cased)?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Case_Ignorable (Unicode-Standard, D136), aufgebaut wie oben."
    )?;
    emit_flags(&mut out, "CASE_IGNORABLE", &tables.case_ignorable)?;
    Ok(out)
}

/// Schreibt eine Tabelle von Namen mit den Kurznamen ihrer Werte.
fn emit_names(out: &mut String, table: &str, names: &BTreeMap<String, String>) -> std::fmt::Result {
    writeln!(
        out,
        "pub static {table}: [(&str, &str); {}] = [",
        names.len()
    )?;
    for (name, short) in names {
        writeln!(out, "    ({name:?}, {short:?}),")?;
    }
    writeln!(out, "];")
}

/// Schreibt eine binäre Eigenschaft als Abschnitte.
fn emit_flags(out: &mut String, table: &str, runs: &[(u32, bool)]) -> std::fmt::Result {
    writeln!(out, "pub static {table}: [(u32, bool); {}] = [", runs.len())?;
    for &(start, flag) in runs {
        writeln!(out, "    (0x{start:04X}, {flag}),")?;
    }
    writeln!(out, "];")
}

/// Schreibt eine Tabelle, die Codepunkten Folgen von Codepunkten zuordnet.
fn emit_sequences(
    out: &mut String,
    table: &str,
    sequences: &BTreeMap<u32, Vec<u32>>,
) -> std::fmt::Result {
    writeln!(
        out,
        "pub static {table}: [(u32, &[u32]); {}] = [",
        sequences.len()
    )?;
    for (code_point, sequence) in sequences {
        let parts: Vec<String> = sequence
            .iter()
            .map(|part| format!("0x{part:04X}"))
            .collect();
        writeln!(out, "    (0x{code_point:04X}, &[{}]),", parts.join(", "))?;
    }
    writeln!(out, "];")
}

/// Schreibt das Aufzählungsobjekt mit Kurz- und Langnamen. Der `impl`-Block
/// bleibt offen, damit weitere Funktionen folgen können.
fn emit_enumeration(out: &mut String, enumeration: &Enumeration<'_>) -> std::fmt::Result {
    let Enumeration {
        name,
        alias,
        doc,
        example: (short_example, long_example),
        values,
    } = enumeration;
    writeln!(out)?;
    writeln!(out, "/// {doc}")?;
    writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]")?;
    writeln!(out, "pub enum {name} {{")?;
    for value in *values {
        writeln!(out, "    /// `{}`", value.short)?;
        writeln!(out, "    {},", variant(&value.long))?;
    }
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "impl {name} {{")?;
    writeln!(out, "    /// Der Kurzname, etwa `{short_example}`.")?;
    writeln!(out, "    pub const fn short_name(self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for value in *values {
        writeln!(
            out,
            "            {alias}::{} => \"{}\",",
            variant(&value.long),
            value.short
        )?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;
    writeln!(out, "    /// Der Langname, etwa `{long_example}`.")?;
    writeln!(out, "    pub const fn long_name(self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for value in *values {
        let variant = variant(&value.long);
        writeln!(out, "            {alias}::{variant} => \"{}\",", value.long)?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")
}

fn emit_enumeration_runs(
    out: &mut String,
    table: &str,
    enumeration: &Enumeration<'_>,
    runs: &[(u32, usize)],
) -> std::fmt::Result {
    writeln!(
        out,
        "pub static {table}: [(u32, {}); {}] = [",
        enumeration.name,
        runs.len()
    )?;
    for &(start, value) in runs {
        let name = enumeration
            .values
            .get(value)
            .map_or_else(String::new, |value| variant(&value.long));
        writeln!(out, "    (0x{start:04X}, {}::{name}),", enumeration.alias)?;
    }
    writeln!(out, "];")
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNICODE_DATA: &str = "0041;LATIN CAPITAL LETTER A;Lu;0;L;;;;;N;;;;0061;\n\
                                0042;LATIN CAPITAL LETTER B;Lu;0;L;;;;;N;;;;0062;\n\
                                00C0;LATIN CAPITAL LETTER A WITH GRAVE;Lu;0;L;0041 0300;;;;N;;;;00E0;\n\
                                00C2;A WITH CIRCUMFLEX;Lu;0;L;0041 0302;;;;N;;;;00E2;\n\
                                0300;COMBINING GRAVE ACCENT;Mn;230;NSM;;;;;N;;;;;\n\
                                0340;COMBINING GRAVE TONE MARK;Mn;230;NSM;0300;;;;N;;;;;\n\
                                0958;DEVANAGARI LETTER QA;Lo;0;L;0915 093C;;;;N;;;;;\n\
                                1EA6;A WITH CIRCUMFLEX AND GRAVE;Lu;0;L;00C2 0300;;;;N;;;;1EA7;\n";

    const VALUE_ALIASES: &str = "# PropertyValueAliases-18.0.0.txt\n\
                                 bc ; L  ; Left_To_Right\n\
                                 bc ; NSM ; Nonspacing_Mark\n\
                                 bc ; R  ; Right_To_Left\n\
                                 ea ; N  ; Neutral\n\
                                 ea ; W  ; Wide\n\
                                 gc ; Cn ; Unassigned\n\
                                 gc ; L  ; Letter  # Lu\n\
                                 gc ; Lo ; Other_Letter\n\
                                 gc ; Lu ; Uppercase_Letter\n\
                                 gc ; Mn ; Nonspacing_Mark\n\
                                 jt ; T ; Transparent\n\
                                 jt ; U ; Non_Joining\n\
                                 lb ; AL ; Alphabetic\n\
                                 lb ; CM ; Combining_Mark\n\
                                 lb ; XX ; Unknown\n\
                                 sc ; Latn ; Latin\n\
                                 sc ; Zinh ; Inherited ; Qaai\n";

    const EXCLUSIONS: &str = "0958    #  DEVANAGARI LETTER QA\n";

    const BIDI: &str = "# @missing: 0000..10FFFF; Left_To_Right\n\
                        # @missing: 0590..05FF; Right_To_Left\n\
                        0300 ; NSM\n0340 ; NSM\n";

    const JOINING: &str = "# @missing: 0000..10FFFF; Non_Joining\n\
                           0300..0340 ; T\n";

    const LINE_BREAKS: &str = "# @missing: 0000..10FFFF; XX\n\
                               0041..0042     ; AL # Lu\n\
                               0300           ; CM # Mn\n";

    const EAST_ASIAN: &str = "# @missing: 0000..10FFFF; N\n\
                              1100..115F     ; W  # Lo\n";

    const EMOJI: &str = "0023          ; Emoji                # E0.0   [1] (#)\n\
                         00A9          ; Extended_Pictographic# E0.6   [1] (©)\n\
                         1F000..1F0FF  ; Extended_Pictographic# E0.0 [256]\n";

    const DERIVED_CORE: &str = "0041..0042    ; ID_Start # L&   [2]\n\
                                0030..0039    ; ID_Continue # Nd  [10]\n\
                                0041..0042    ; ID_Continue # L&   [2]\n\
                                0300          ; ID_Continue # Mn\n\
                                0041..0042    ; XID_Start # L&   [2]\n\
                                0041..0042    ; Cased # L&   [2]\n\
                                0300          ; Case_Ignorable # Mn\n\
                                # @missing: 0000..10FFFF; InCB; None\n\
                                094D          ; InCB; Linker # Mn\n";

    const SPECIAL_CASING: &str = "00DF; 00DF; 0053 0073; 0053 0053; # LATIN SMALL LETTER SHARP S\n\
                                  03A3; 03C2; 03A3; 03A3; Final_Sigma; # GREEK CAPITAL LETTER SIGMA\n\
                                  0049; 0131; 0049; 0049; tr Not_Before_Dot; # LATIN CAPITAL LETTER I\n";

    const CASE_FOLDING: &str = "0041; C; 0061; # LATIN CAPITAL LETTER A\n\
                                0049; T; 0131; # LATIN CAPITAL LETTER I\n\
                                00DF; F; 0073 0073; # LATIN SMALL LETTER SHARP S\n\
                                1E9E; F; 0073 0073; # LATIN CAPITAL LETTER SHARP S\n\
                                1E9E; S; 00DF; # LATIN CAPITAL LETTER SHARP S\n";

    fn sources<'a>(unicode_data: &'a str, value_aliases: &'a str) -> Sources<'a> {
        Sources {
            unicode_data,
            value_aliases,
            composition_exclusions: EXCLUSIONS,
            bidi_classes: BIDI,
            joining_types: JOINING,
            line_breaks: LINE_BREAKS,
            east_asian_widths: EAST_ASIAN,
            emoji_data: EMOJI,
            derived_core_properties: DERIVED_CORE,
            case_folding: CASE_FOLDING,
            special_casing: SPECIAL_CASING,
        }
    }

    #[test]
    fn equal_values_become_runs() {
        assert_eq!(runs(&[1, 1, 2, 2, 2, 1]), [(0, 1), (2, 2), (5, 1)]);
        assert_eq!(runs::<u8>(&[]), []);
    }

    #[test]
    fn group_comments() {
        assert_eq!(group_members("Ll | Lt | Lu"), Some(vec!["Ll", "Lt", "Lu"]));
        assert_eq!(group_members("Lu"), Some(vec!["Lu"]));
        assert_eq!(group_members("RESERVED"), None);
        assert_eq!(group_members("Ll | etwas"), None);
    }

    #[test]
    fn names_follow_rust_conventions() {
        assert_eq!(variant("Uppercase_Letter"), "UppercaseLetter");
        assert_eq!(variant("Other"), "Other");
        assert_eq!(predicate("Cased_Letter"), "is_cased_letter");
    }

    #[test]
    fn small_database_gives_tables() {
        let source = generate(&sources(UNICODE_DATA, VALUE_ALIASES)).expect("gültige Daten");
        assert!(source.contains("pub const UNICODE_VERSION: (u8, u8, u8) = (18, 0, 0);"));
        assert!(source.contains("    /// `Lu`\n    UppercaseLetter,\n"));
        assert!(source.contains("    pub const fn is_letter(self) -> bool {\n        matches!(self, G::UppercaseLetter)\n"));
        assert!(source.contains(
            "pub static GENERAL_CATEGORY: [(u32, GeneralCategory); 15] = [\n    \
             (0x0000, G::Unassigned),\n    \
             (0x0041, G::UppercaseLetter),\n    \
             (0x0043, G::Unassigned),\n    \
             (0x00C0, G::UppercaseLetter),\n    \
             (0x00C1, G::Unassigned),\n    \
             (0x00C2, G::UppercaseLetter),\n    \
             (0x00C3, G::Unassigned),\n    \
             (0x0300, G::NonspacingMark),\n    \
             (0x0301, G::Unassigned),\n"
        ));
        assert!(source.contains(
            "pub static CANONICAL_COMBINING_CLASS: [(u32, u8); 5] = [\n    \
             (0x0000, 0),\n    (0x0300, 230),\n    (0x0301, 0),\n    (0x0340, 230),\n    (0x0341, 0),\n];\n"
        ));
        assert!(source.contains(
            "pub static BIDI_CLASS: [(u32, BidiClass); 7] = [\n    \
             (0x0000, B::LeftToRight),\n    (0x0300, B::NonspacingMark),\n    \
             (0x0301, B::LeftToRight),\n    (0x0340, B::NonspacingMark),\n    \
             (0x0341, B::LeftToRight),\n    (0x0590, B::RightToLeft),\n    \
             (0x0600, B::LeftToRight),\n];\n"
        ));
        assert!(source.contains(
            "pub static JOINING_TYPE: [(u32, JoiningType); 3] = [\n    \
             (0x0000, J::NonJoining),\n    (0x0300, J::Transparent),\n    \
             (0x0341, J::NonJoining),\n];\n"
        ));
        // Vollständig zerlegt, auch U+1EA6 über U+00C2.
        assert!(source.contains(
            "pub static CANONICAL_DECOMPOSITION: [(u32, &[u32]); 5] = [\n    \
             (0x00C0, &[0x0041, 0x0300]),\n    (0x00C2, &[0x0041, 0x0302]),\n    \
             (0x0340, &[0x0300]),\n    (0x0958, &[0x0915, 0x093C]),\n    \
             (0x1EA6, &[0x0041, 0x0302, 0x0300]),\n];\n"
        ));
        // Ohne Einzelzerlegung (U+0340) und Ausnahme (U+0958), U+1EA6 mit
        // seiner Zerlegung eine Ebene tief.
        assert!(source.contains(
            "pub static CANONICAL_COMPOSITION: [(u32, u32, u32); 3] = [\n    \
             (0x0041, 0x0300, 0x00C0),\n    (0x0041, 0x0302, 0x00C2),\n    \
             (0x00C2, 0x0300, 0x1EA6),\n];\n"
        ));
        assert!(source.contains(
            "pub static LINE_BREAK: [(u32, LineBreak); 5] = [\n    \
             (0x0000, L::Unknown),\n    (0x0041, L::Alphabetic),\n    \
             (0x0043, L::Unknown),\n    (0x0300, L::CombiningMark),\n    \
             (0x0301, L::Unknown),\n];\n"
        ));
        assert!(source.contains(
            "pub static EAST_ASIAN_WIDTH: [(u32, EastAsianWidth); 3] = [\n    \
             (0x0000, E::Neutral),\n    (0x1100, E::Wide),\n    \
             (0x1160, E::Neutral),\n];\n"
        ));
        assert!(source.contains(
            "pub static EXTENDED_PICTOGRAPHIC: [(u32, bool); 5] = [\n    \
             (0x0000, false),\n    (0x00A9, true),\n    (0x00AA, false),\n    \
             (0x1F000, true),\n    (0x1F100, false),\n];\n"
        ));
        // Nur die genannte Eigenschaft, nicht XID_Start oder InCB.
        assert!(source.contains(
            "pub static ID_START: [(u32, bool); 3] = [\n    \
             (0x0000, false),\n    (0x0041, true),\n    (0x0043, false),\n];\n"
        ));
        assert!(source.contains(
            "pub static ID_CONTINUE: [(u32, bool); 7] = [\n    \
             (0x0000, false),\n    (0x0030, true),\n    (0x003A, false),\n    \
             (0x0041, true),\n    (0x0043, false),\n    (0x0300, true),\n    \
             (0x0301, false),\n];\n"
        ));
        // Alle Namen samt Gruppen und weiteren Aliassen, nach Namen sortiert.
        assert!(source.contains(
            "pub static GENERAL_CATEGORY_NAMES: [(&str, &str); 10] = [\n    \
             (\"Cn\", \"Cn\"),\n    (\"L\", \"L\"),\n    (\"Letter\", \"L\"),\n    \
             (\"Lo\", \"Lo\"),\n    (\"Lu\", \"Lu\"),\n    (\"Mn\", \"Mn\"),\n    \
             (\"Nonspacing_Mark\", \"Mn\"),\n    (\"Other_Letter\", \"Lo\"),\n    \
             (\"Unassigned\", \"Cn\"),\n    (\"Uppercase_Letter\", \"Lu\"),\n];\n"
        ));
        assert!(source.contains(
            "pub static SCRIPT_NAMES: [(&str, &str); 5] = [\n    \
             (\"Inherited\", \"Zinh\"),\n    (\"Latin\", \"Latn\"),\n    \
             (\"Latn\", \"Latn\"),\n    (\"Qaai\", \"Zinh\"),\n    \
             (\"Zinh\", \"Zinh\"),\n];\n"
        ));
        // Ohne die einfache und die türkische Faltung.
        assert!(source.contains(
            "pub static CASE_FOLDING: [(u32, &[u32]); 3] = [\n    \
             (0x0041, &[0x0061]),\n    (0x00DF, &[0x0073, 0x0073]),\n    \
             (0x1E9E, &[0x0073, 0x0073]),\n];\n"
        ));
    }

    #[test]
    fn unknown_values_are_errors() {
        let data = UNICODE_DATA.replace("Mn", "Xx");
        assert!(generate(&sources(&data, VALUE_ALIASES)).is_err());
        let aliases = VALUE_ALIASES.replace("# Lu", "# Lu | Xx");
        assert!(generate(&sources(UNICODE_DATA, &aliases)).is_err());
        let aliases = VALUE_ALIASES.replace("gc ; Cn ; Unassigned\n", "");
        assert!(generate(&sources(UNICODE_DATA, &aliases)).is_err());
        let aliases = VALUE_ALIASES.replace("bc ; R  ; Right_To_Left\n", "");
        assert!(generate(&sources(UNICODE_DATA, &aliases)).is_err());
        let aliases = VALUE_ALIASES.replace("sc ; Latn ; Latin\n", "");
        let aliases = aliases.replace("sc ; Zinh ; Inherited ; Qaai\n", "");
        assert!(
            generate(&sources(UNICODE_DATA, &aliases)).is_err(),
            "ohne Script"
        );
        let aliases = VALUE_ALIASES.replace("; Qaai", "; Latin");
        assert!(
            generate(&sources(UNICODE_DATA, &aliases)).is_err(),
            "Name für zwei Werte"
        );
        let mut broken = sources(UNICODE_DATA, VALUE_ALIASES);
        broken.joining_types = "0300 ; T\n";
        assert!(generate(&broken).is_err(), "Codepunkte ohne Wert");
        let mut broken = sources(UNICODE_DATA, VALUE_ALIASES);
        broken.emoji_data = "0023 ; Emoji\n";
        assert!(generate(&broken).is_err(), "ohne Extended_Pictographic");
        let mut broken = sources(UNICODE_DATA, VALUE_ALIASES);
        broken.derived_core_properties = "0041 ; ID_Start\n";
        assert!(generate(&broken).is_err(), "ohne ID_Continue");
        for case_folding in [
            "0041; X; 0061;\n",
            "0041; C;\n",
            "0041; C; ;\n",
            "0041..0042; C; 0061;\n",
            "0041; C; 0061;\n0041; F; 0061;\n",
            "0041; S; 0061;\n",
        ] {
            let mut broken = sources(UNICODE_DATA, VALUE_ALIASES);
            broken.case_folding = case_folding;
            assert!(generate(&broken).is_err(), "{case_folding:?}");
        }
    }

    #[test]
    fn case_mappings_and_compatibility_decompositions() {
        // U+1EA7 bekommt hier eine erfundene Kompatibilitätszerlegung, die
        // über U+1EA6 und U+00C2 weiter zerlegt wird.
        let data = format!(
            "{UNICODE_DATA}1EA7;A WITH CIRCUMFLEX AND GRAVE;Lo;0;L;<compat> 1EA6 0042;;;;N;;;1EA6;;\n"
        );
        let source = generate(&sources(&data, VALUE_ALIASES)).expect("gültige Daten");
        assert!(source.contains(
            "pub static COMPATIBILITY_DECOMPOSITION: [(u32, &[u32]); 1] = [\n    \
             (0x1EA7, &[0x0041, 0x0302, 0x0300, 0x0042]),\n];\n"
        ));
        // U+00DF bleibt klein, wie es ist, und wird groß zu zwei Zeichen; die
        // Zeile für Türkisch zählt nicht.
        assert!(source.contains(
            "pub static LOWERCASE: [(u32, &[u32]); 5] = [\n    \
             (0x0041, &[0x0061]),\n    (0x0042, &[0x0062]),\n    \
             (0x00C0, &[0x00E0]),\n    (0x00C2, &[0x00E2]),\n    \
             (0x1EA6, &[0x1EA7]),\n];\n"
        ));
        assert!(source.contains(
            "pub static UPPERCASE: [(u32, &[u32]); 2] = [\n    \
             (0x00DF, &[0x0053, 0x0053]),\n    (0x1EA7, &[0x1EA6]),\n];\n"
        ));
        assert!(source.contains(
            "pub static CASED: [(u32, bool); 3] = [\n    \
             (0x0000, false),\n    (0x0041, true),\n    (0x0043, false),\n];\n"
        ));
        assert!(source.contains(
            "pub static CASE_IGNORABLE: [(u32, bool); 3] = [\n    \
             (0x0000, false),\n    (0x0300, true),\n    (0x0301, false),\n];\n"
        ));
        for special_casing in [
            // Ohne Final_Sigma.
            "00DF; 00DF; 0053 0073; 0053 0053;\n",
            // Final_Sigma an einem anderen Zeichen und doppelt.
            "0041; 0061; 0041; 0041; Final_Sigma;\n",
            "03A3; 03C2; 03A3; 03A3; Final_Sigma;\n03A3; 03C2; 03A3; 03A3; Final_Sigma;\n",
            // Eine Bedingung, die `mb-unicode` nicht kennt.
            "03A3; 03C2; 03A3; 03A3; Final_Sigma;\n0041; 0061; 0041; 0041; After_I;\n",
            "03A3; 03C2; 03A3; 03A3; Final_Sigma;\n00DF; 00DF; 0053 0073;\n",
            "03A3; 03C2; 03A3; 03A3; Final_Sigma;\n00DF; ; 0053 0073; 0053 0053;\n",
            "03A3; 03C2; 03A3; 03A3; Final_Sigma;\n00DF..00E0; 00DF; 0053 0073; 0053 0053;\n",
            "03A3; 03C2; 03A3; 03A3; Final_Sigma;\n00DF; 00DF; 0053; 0053;\n00DF; 00DF; 0053; 0053;\n",
        ] {
            let mut broken = sources(UNICODE_DATA, VALUE_ALIASES);
            broken.special_casing = special_casing;
            assert!(generate(&broken).is_err(), "{special_casing:?}");
        }
        let mut broken = sources(UNICODE_DATA, VALUE_ALIASES);
        broken.derived_core_properties = "0041 ; ID_Start\n0041 ; ID_Continue\n0041 ; Cased\n";
        assert!(generate(&broken).is_err(), "ohne Case_Ignorable");
    }

    #[test]
    fn cyclic_decompositions_are_errors() {
        let data = "0041;A;Lu;0;L;0042;;;;N;;;;;\n0042;B;Lu;0;L;0041;;;;N;;;;;\n";
        assert!(generate(&sources(data, VALUE_ALIASES)).is_err());
    }
}
