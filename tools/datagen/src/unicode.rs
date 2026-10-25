//! Die Unicode-Tabellen für `mb-unicode`: allgemeine Kategorie, kanonische
//! Kombinationsklasse und kanonische Zerlegungen aus `UnicodeData.txt`,
//! Ausnahmen der Komposition aus `CompositionExclusions.txt`, Bidi-Klasse und
//! Joining-Type aus den abgeleiteten Dateien, Namen der Werte aus
//! `PropertyValueAliases.txt`.

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
    })
}

/// Erzeugt den Quelltext von `tables.rs` aus den Rohdaten.
pub fn generate(sources: &Sources<'_>) -> Result<String, String> {
    let entries = ucd::parse_unicode_data(sources.unicode_data)?;
    let (version, aliases) = ucd::parse_property_value_aliases(sources.value_aliases)?;
    let (categories, groups) = general_categories(&aliases)?;
    let bidi_values = property_values(&aliases, "bc")?;
    let joining_values = property_values(&aliases, "jt")?;
    let unassigned = find(&categories, "Cn")?;
    let count = index(MAX_CODE_POINT)? + 1;
    let mut category_of = vec![unassigned; count];
    let mut class_of = vec![0u8; count];
    let mut decompositions = BTreeMap::new();
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
    }
    let (_, exclusions) =
        ucd::parse_property_file("CompositionExclusions.txt", sources.composition_exclusions)?;
    let compositions = compositions(&decompositions, &exclusions, &class_of)?;
    let decompositions = full_decompositions(&decompositions)?;
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
    let tables = Tables {
        general: runs(&category_of),
        classes: runs(&class_of),
        bidi: runs(&bidi_of),
        joining: runs(&joining_of),
        decompositions,
        compositions,
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
        let value = find(values, &line.value).map_err(|message| format!("{file}: {message}"))?;
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

/// Die Tabellen, die `emit` schreibt.
struct Tables {
    general: Vec<(u32, usize)>,
    classes: Vec<(u32, u8)>,
    bidi: Vec<(u32, usize)>,
    joining: Vec<(u32, usize)>,
    decompositions: BTreeMap<u32, Vec<u32>>,
    compositions: Vec<(u32, u32, u32)>,
}

fn emit(
    version: Version,
    enumerations: [Enumeration<'_>; 3],
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
    let [general, bidi, joining] = &enumerations;
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
    emit_enumeration(&mut out, joining)?;
    writeln!(out, "}}")?;
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
        "/// Vollständige kanonische Zerlegung jedes zerlegbaren Zeichens, nach"
    )?;
    writeln!(
        out,
        "/// Codepunkt sortiert. Hangul-Silben fehlen, sie werden berechnet."
    )?;
    writeln!(
        out,
        "pub static CANONICAL_DECOMPOSITION: [(u32, &[u32]); {}] = [",
        tables.decompositions.len()
    )?;
    for (code_point, decomposition) in &tables.decompositions {
        let parts: Vec<String> = decomposition
            .iter()
            .map(|part| format!("0x{part:04X}"))
            .collect();
        writeln!(out, "    (0x{code_point:04X}, &[{}]),", parts.join(", "))?;
    }
    writeln!(out, "];")?;
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
    Ok(out)
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
                                 gc ; Cn ; Unassigned\n\
                                 gc ; L  ; Letter  # Lu\n\
                                 gc ; Lo ; Other_Letter\n\
                                 gc ; Lu ; Uppercase_Letter\n\
                                 gc ; Mn ; Nonspacing_Mark\n\
                                 jt ; T ; Transparent\n\
                                 jt ; U ; Non_Joining\n";

    const EXCLUSIONS: &str = "0958    #  DEVANAGARI LETTER QA\n";

    const BIDI: &str = "# @missing: 0000..10FFFF; Left_To_Right\n\
                        # @missing: 0590..05FF; Right_To_Left\n\
                        0300 ; NSM\n0340 ; NSM\n";

    const JOINING: &str = "# @missing: 0000..10FFFF; Non_Joining\n\
                           0300..0340 ; T\n";

    fn sources<'a>(unicode_data: &'a str, value_aliases: &'a str) -> Sources<'a> {
        Sources {
            unicode_data,
            value_aliases,
            composition_exclusions: EXCLUSIONS,
            bidi_classes: BIDI,
            joining_types: JOINING,
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
        let mut broken = sources(UNICODE_DATA, VALUE_ALIASES);
        broken.joining_types = "0300 ; T\n";
        assert!(generate(&broken).is_err(), "Codepunkte ohne Wert");
    }

    #[test]
    fn cyclic_decompositions_are_errors() {
        let data = "0041;A;Lu;0;L;0042;;;;N;;;;;\n0042;B;Lu;0;L;0041;;;;N;;;;;\n";
        assert!(generate(&sources(data, VALUE_ALIASES)).is_err());
    }
}
