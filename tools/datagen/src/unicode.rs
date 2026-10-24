//! Die Unicode-Tabellen für `mb-unicode`: allgemeine Kategorie und
//! kanonische Kombinationsklasse aus `UnicodeData.txt`, Namen der Kategorien
//! aus `PropertyValueAliases.txt`.

use std::fmt::Write;

use crate::ucd::{self, MAX_CODE_POINT, ValueAlias, Version};

/// Die Rohdaten, relativ zum Workspace.
pub const DATA: &str = "data/unicode";

/// Die erzeugte Datei, relativ zum Workspace.
pub const OUTPUT: &str = "crates/mb-unicode/src/tables.rs";

/// Eine allgemeine Kategorie, etwa `Lu` mit dem Langnamen `Uppercase_Letter`.
struct Category {
    short: String,
    long: String,
}

/// Eine Gruppe von Kategorien, etwa `L` (`Letter`) mit `Ll | Lm | Lo | Lt | Lu`.
struct Group {
    short: String,
    long: String,
    members: Vec<usize>,
}

/// Erzeugt den Quelltext von `tables.rs` aus den Inhalten von
/// `UnicodeData.txt` und `PropertyValueAliases.txt`.
pub fn generate(unicode_data: &str, value_aliases: &str) -> Result<String, String> {
    let entries = ucd::parse_unicode_data(unicode_data)?;
    let (version, aliases) = ucd::parse_property_value_aliases(value_aliases)?;
    let (categories, groups) = general_categories(&aliases)?;
    let unassigned = find(&categories, "Cn")?;
    let count = index(MAX_CODE_POINT)? + 1;
    let mut category_of = vec![unassigned; count];
    let mut class_of = vec![0u8; count];
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
    }
    emit(
        version,
        &categories,
        &groups,
        &runs(&category_of),
        &runs(&class_of),
    )
    .map_err(|_| "Fehler beim Schreiben des Quelltexts".to_owned())
}

/// Die Kategorien und Gruppen in der Reihenfolge der Datei. Gruppen erkennt
/// man am Kommentar mit ihren Mitgliedern.
fn general_categories(aliases: &[ValueAlias]) -> Result<(Vec<Category>, Vec<Group>), String> {
    let mut categories = Vec::new();
    let mut members = Vec::new();
    for alias in aliases.iter().filter(|alias| alias.property == "gc") {
        let [short, long, ..] = alias.fields.as_slice() else {
            return Err(format!("gc-Eintrag ohne Langnamen: {:?}", alias.fields));
        };
        match alias.comment.as_deref().and_then(group_members) {
            Some(list) => members.push((short, long, list)),
            None => categories.push(Category {
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

fn find(categories: &[Category], short: &str) -> Result<usize, String> {
    categories
        .iter()
        .position(|category| category.short == short)
        .ok_or_else(|| format!("unbekannte allgemeine Kategorie {short:?}"))
}

fn index(code_point: u32) -> Result<usize, String> {
    usize::try_from(code_point).map_err(|_| format!("Codepunkt {code_point:#X} passt nicht"))
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

fn emit(
    version: Version,
    categories: &[Category],
    groups: &[Group],
    general: &[(u32, usize)],
    classes: &[(u32, u8)],
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
    writeln!(out, "use GeneralCategory as G;")?;
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
    writeln!(out)?;
    writeln!(
        out,
        "/// Allgemeine Kategorie eines Codepunkts (General_Category, UAX #44)."
    )?;
    writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]")?;
    writeln!(out, "pub enum GeneralCategory {{")?;
    for category in categories {
        writeln!(out, "    /// `{}`", category.short)?;
        writeln!(out, "    {},", variant(&category.long))?;
    }
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "impl GeneralCategory {{")?;
    writeln!(out, "    /// Der Kurzname, etwa `Lu`.")?;
    writeln!(out, "    pub const fn short_name(self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for category in categories {
        writeln!(
            out,
            "            G::{} => \"{}\",",
            variant(&category.long),
            category.short
        )?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out)?;
    writeln!(out, "    /// Der Langname, etwa `Uppercase_Letter`.")?;
    writeln!(out, "    pub const fn long_name(self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for category in categories {
        let name = variant(&category.long);
        writeln!(out, "            G::{name} => \"{}\",", category.long)?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    for group in groups {
        let members: Vec<&Category> = group
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
    writeln!(out)?;
    writeln!(
        out,
        "/// Allgemeine Kategorie: Anfang jedes Abschnitts und seine Kategorie. Ein"
    )?;
    writeln!(
        out,
        "/// Abschnitt reicht bis vor den nächsten, der letzte bis U+10FFFF."
    )?;
    writeln!(
        out,
        "pub static GENERAL_CATEGORY: [(u32, GeneralCategory); {}] = [",
        general.len()
    )?;
    for &(start, category) in general {
        let name = categories
            .get(category)
            .map_or_else(String::new, |category| variant(&category.long));
        writeln!(out, "    (0x{start:04X}, G::{name}),")?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Kanonische Kombinationsklasse (Canonical_Combining_Class): Anfang jedes"
    )?;
    writeln!(out, "/// Abschnitts und seine Klasse, aufgebaut wie oben.")?;
    writeln!(
        out,
        "pub static CANONICAL_COMBINING_CLASS: [(u32, u8); {}] = [",
        classes.len()
    )?;
    for &(start, class) in classes {
        writeln!(out, "    (0x{start:04X}, {class}),")?;
    }
    writeln!(out, "];")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNICODE_DATA: &str = "0041;LATIN CAPITAL LETTER A;Lu;0;L;;;;;N;;;;0061;\n\
                                0042;LATIN CAPITAL LETTER B;Lu;0;L;;;;;N;;;;0062;\n\
                                0300;COMBINING GRAVE ACCENT;Mn;230;NSM;;;;;N;;;;;\n";

    const VALUE_ALIASES: &str = "# PropertyValueAliases-18.0.0.txt\n\
                                 gc ; Cn ; Unassigned\n\
                                 gc ; L  ; Letter  # Lu\n\
                                 gc ; Lu ; Uppercase_Letter\n\
                                 gc ; Mn ; Nonspacing_Mark\n";

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
        let source = generate(UNICODE_DATA, VALUE_ALIASES).expect("gültige Daten");
        assert!(source.contains("pub const UNICODE_VERSION: (u8, u8, u8) = (18, 0, 0);"));
        assert!(source.contains("    /// `Lu`\n    UppercaseLetter,\n"));
        assert!(source.contains("    pub const fn is_letter(self) -> bool {\n        matches!(self, G::UppercaseLetter)\n"));
        assert!(source.contains(
            "pub static GENERAL_CATEGORY: [(u32, GeneralCategory); 5] = [\n    \
             (0x0000, G::Unassigned),\n    \
             (0x0041, G::UppercaseLetter),\n    \
             (0x0043, G::Unassigned),\n    \
             (0x0300, G::NonspacingMark),\n    \
             (0x0301, G::Unassigned),\n];\n"
        ));
        assert!(source.contains(
            "pub static CANONICAL_COMBINING_CLASS: [(u32, u8); 3] = [\n    \
             (0x0000, 0),\n    (0x0300, 230),\n    (0x0301, 0),\n];\n"
        ));
    }

    #[test]
    fn unknown_categories_are_errors() {
        let data = UNICODE_DATA.replace("Mn", "Xx");
        assert!(generate(&data, VALUE_ALIASES).is_err());
        let aliases = VALUE_ALIASES.replace("# Lu", "# Lu | Xx");
        assert!(generate(UNICODE_DATA, &aliases).is_err());
        let aliases = VALUE_ALIASES.replace("gc ; Cn ; Unassigned\n", "");
        assert!(generate(UNICODE_DATA, &aliases).is_err());
    }

    #[test]
    fn checked_in_tables_are_current() {
        let generated = generate(
            include_str!("../../../data/unicode/UnicodeData.txt"),
            include_str!("../../../data/unicode/PropertyValueAliases.txt"),
        )
        .expect("Rohdaten lesbar");
        assert!(
            generated == include_str!("../../../crates/mb-unicode/src/tables.rs"),
            "{OUTPUT} ist veraltet, bitte `cargo run -p datagen` ausführen"
        );
    }
}
