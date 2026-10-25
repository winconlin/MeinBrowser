//! Prüft die Tabellen der Eigenschaften gegen webref (`ed/css/*.json`):
//! Datei, Grammatik, Anfangswert, Vererbung und *logical property group*
//! der Langformen, Grammatik und Langformen der Kurzschreibweisen und die
//! Altnamen.
//!
//! Gilt eine Eigenschaft in mehreren Spezifikationen, zählt die Definition
//! mit Grammatik aus der höchsten Stufe; die Stufe steht am Ende der URL
//! der Spezifikation, ohne Angabe ist es Stufe 1.

use mb_json::Value as Json;

use super::{ALIASES, Longhand, Mapping, PropertyId, Shorthand};

/// Kurzschreibweisen, deren Langformen von webref abweichen, mit Grund.
const LONGHAND_EXCEPTIONS: &[(&str, &str)] = &[
    (
        "font",
        "Von font-variant gibt es bisher nur font-variant-caps; die übrigen \
         Langformen von font-variant fehlen noch.",
    ),
    (
        "white-space",
        "Die Grammatik von Text 4 enthält <'white-space-trim'>, die Liste der \
         Langformen in webref aber nicht.",
    ),
    ("all", "webref nennt für all keine Langformen."),
];

/// Eine Spezifikation aus webref.
struct Spec {
    /// Der Dateiname ohne `.json`
    name: String,
    level: u32,
    data: Json,
}

fn load() -> Vec<Spec> {
    let mut specs = Vec::new();
    for file in testdata::files("webref", "ed/css", "json") {
        let text = testdata::read("webref", &format!("ed/css/{file}"));
        let data = mb_json::parse(&text).unwrap_or_else(|error| panic!("{file}: {error}"));
        let url = data
            .get("spec")
            .and_then(|spec| spec.get("url"))
            .and_then(Json::as_str)
            .unwrap_or_default();
        let level = url
            .trim_end_matches('/')
            .rsplit('-')
            .next()
            .and_then(|suffix| suffix.parse().ok())
            .unwrap_or(1);
        let name = file.strip_suffix(".json").unwrap_or(&file).to_owned();
        specs.push(Spec { name, level, data });
    }
    specs
}

fn properties(spec: &Spec) -> &[Json] {
    spec.data
        .get("properties")
        .and_then(Json::as_array)
        .unwrap_or_default()
}

/// Die geltende Definition einer Eigenschaft mit ihrer Spezifikation.
fn definition<'a>(specs: &'a [Spec], name: &str) -> Result<(&'a Spec, &'a Json), String> {
    let mut candidates: Vec<(&Spec, &Json)> = specs
        .iter()
        .flat_map(|spec| {
            properties(spec)
                .iter()
                .map(move |property| (spec, property))
        })
        .filter(|(_, property)| {
            property.get("name").and_then(Json::as_str) == Some(name)
                && property.get("value").is_some()
        })
        .collect();
    candidates.sort_by_key(|(spec, _)| std::cmp::Reverse(spec.level));
    match candidates.as_slice() {
        [] => Err(format!("{name}: fehlt in webref")),
        [(first, _), (second, _), ..] if first.level == second.level => Err(format!(
            "{name}: mehrdeutig zwischen {} und {}",
            first.name, second.name
        )),
        [best, ..] => Ok(*best),
    }
}

fn field<'a>(property: &'a Json, key: &str) -> Option<&'a str> {
    property.get(key).and_then(Json::as_str)
}

/// Die Langformen einer Kurzschreibweise nach webref, mit aufgelösten
/// Kurzschreibweisen darin und Altnamen. Die Tiefe ist begrenzt.
fn webref_longhands(specs: &[Spec], name: &str, depth: usize) -> Result<Vec<String>, String> {
    if depth > 4 {
        return Err(format!("{name}: Langformen zu tief verschachtelt"));
    }
    let (_, property) = definition(specs, name)?;
    let Some(list) = property.get("longhands").and_then(Json::as_array) else {
        return Ok(vec![name.to_owned()]);
    };
    let mut longhands = Vec::new();
    for entry in list {
        let entry = entry
            .as_str()
            .ok_or_else(|| format!("{name}: Langform ohne Namen"))?;
        let alias = specs
            .iter()
            .flat_map(properties)
            .find(|property| field(property, "name") == Some(entry))
            .and_then(|property| field(property, "legacyAliasOf"));
        match alias {
            Some(target) => longhands.push(target.to_owned()),
            None => longhands.extend(webref_longhands(specs, entry, depth.saturating_add(1))?),
        }
    }
    Ok(longhands)
}

fn check_longhand(specs: &[Spec], longhand: Longhand) -> Result<(), String> {
    let info = longhand.info();
    let (spec, property) = definition(specs, info.name)?;
    let name = info.name;
    if spec.name != info.spec {
        return Err(format!(
            "{name}: gilt aus {}, nicht {}",
            spec.name, info.spec
        ));
    }
    if field(property, "value") != Some(info.syntax) {
        return Err(format!("{name}: Grammatik {:?}", field(property, "value")));
    }
    if field(property, "initial") != Some(info.initial) {
        return Err(format!(
            "{name}: Anfangswert {:?}",
            field(property, "initial")
        ));
    }
    let inherited = field(property, "inherited").unwrap_or_default();
    let inherited = if inherited.starts_with("yes") {
        true
    } else if inherited.starts_with("no") {
        false
    } else {
        return Err(format!("{name}: Vererbung {inherited:?}"));
    };
    if inherited != info.inherited {
        return Err(format!("{name}: vererbt {inherited}"));
    }
    let group = info.group.map(|(group, _)| group.name());
    if field(property, "logicalPropertyGroup") != group {
        return Err(format!(
            "{name}: Gruppe {:?}",
            field(property, "logicalPropertyGroup")
        ));
    }
    // webref nennt die *mapping logic* nicht; logisch sind die Eigenschaften
    // mit „block“ oder „inline“ im Namen.
    if let Some((_, mapping)) = info.group {
        let logical = name.contains("block") || name.contains("inline");
        if (mapping == Mapping::Logical) != logical {
            return Err(format!("{name}: falsche mapping logic"));
        }
    }
    Ok(())
}

fn check_shorthand(specs: &[Spec], shorthand: Shorthand) -> Result<(), String> {
    let info = shorthand.info();
    let name = info.name;
    let (spec, property) = definition(specs, name)?;
    if spec.name != info.spec {
        return Err(format!(
            "{name}: gilt aus {}, nicht {}",
            spec.name, info.spec
        ));
    }
    if field(property, "value") != Some(info.syntax) {
        return Err(format!("{name}: Grammatik {:?}", field(property, "value")));
    }
    if LONGHAND_EXCEPTIONS
        .iter()
        .any(|(exception, _)| *exception == name)
    {
        return Ok(());
    }
    let mut expected = webref_longhands(specs, name, 0)?;
    expected.sort();
    let mut own: Vec<&str> = shorthand
        .longhands()
        .iter()
        .map(|longhand| longhand.name())
        .collect();
    own.sort_unstable();
    if expected != own {
        return Err(format!("{name}: Langformen {expected:?}"));
    }
    Ok(())
}

#[test]
fn tables_match_webref() {
    let specs = load();
    let mut failures = Vec::new();
    for &longhand in Longhand::ALL {
        if let Err(failure) = check_longhand(&specs, longhand) {
            failures.push(failure);
        }
    }
    for &shorthand in Shorthand::ALL {
        if let Err(failure) = check_shorthand(&specs, shorthand) {
            failures.push(failure);
        }
    }
    for (alias, longhand) in ALIASES {
        let target = specs
            .iter()
            .flat_map(properties)
            .find(|property| field(property, "name") == Some(alias))
            .and_then(|property| field(property, "legacyAliasOf"));
        if target != Some(longhand.name()) {
            failures.push(format!("{alias}: Altname für {target:?}"));
        }
        if PropertyId::from_name(alias) != Some(PropertyId::Longhand(longhand)) {
            failures.push(format!("{alias}: nicht als Altname bekannt"));
        }
    }
    // Die Ausnahmen müssen noch gelten.
    for (name, _) in LONGHAND_EXCEPTIONS {
        let Some(PropertyId::Shorthand(shorthand)) = PropertyId::from_name(name) else {
            failures.push(format!("{name}: keine Kurzschreibweise"));
            continue;
        };
        let mut own: Vec<&str> = shorthand
            .longhands()
            .iter()
            .map(|longhand| longhand.name())
            .collect();
        own.sort_unstable();
        if let Ok(mut expected) = webref_longhands(&specs, name, 0) {
            expected.sort();
            if expected == own {
                failures.push(format!("{name}: Ausnahme nicht mehr nötig"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} Abweichungen von webref:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
