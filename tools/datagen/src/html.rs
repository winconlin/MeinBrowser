//! Die Tabelle der benannten Zeichenreferenzen für `mb-html` aus
//! `entities.json` (HTML Standard, Abschnitt „Named character references“).

use std::fmt::Write;

use mb_json::Value;

/// Die Rohdaten, relativ zum Workspace.
pub const DATA: &str = "data/html";

/// Die erzeugte Datei, relativ zum Workspace.
pub const OUTPUT: &str = "crates/mb-html/src/entities/tables.rs";

/// Eine Zeichenreferenz: Name ohne das führende `&` und die Zeichen.
type Entity = (String, String);

/// Erzeugt den Quelltext von `entities/tables.rs`. `read` liefert den Inhalt
/// einer Datei aus [`DATA`].
pub fn run(read: &dyn Fn(&str) -> Result<String, String>) -> Result<String, String> {
    let entities = parse_entities(&read("entities.json")?)?;
    emit(&entities).map_err(|_| "Fehler beim Schreiben des Quelltexts".to_owned())
}

/// Liest `entities.json`: ein Objekt, dessen Schlüssel die Namen mit `&` sind
/// und dessen Werte die Codepunkte (`codepoints`) und die Zeichen
/// (`characters`) nennen. Beides muss zusammenpassen.
fn parse_entities(text: &str) -> Result<Vec<Entity>, String> {
    let root = mb_json::parse(text).map_err(|error| format!("entities.json: {error}"))?;
    let Value::Object(members) = root else {
        return Err("entities.json: Objekt erwartet".into());
    };
    let mut entities = Vec::new();
    for (key, value) in &members {
        let error = |message: &str| format!("entities.json, {key}: {message}");
        let name = key.strip_prefix('&').ok_or_else(|| error("ohne &"))?;
        let letters = name.strip_suffix(';').unwrap_or(name);
        if letters.is_empty() || !letters.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(error("ungültiger Name"));
        }
        let characters = value
            .get("characters")
            .and_then(Value::as_str)
            .ok_or_else(|| error("ohne characters"))?;
        let code_points = value
            .get("codepoints")
            .and_then(Value::as_array)
            .ok_or_else(|| error("ohne codepoints"))?
            .iter()
            .map(|number| match number {
                Value::Number(digits) => digits.parse::<u32>().ok(),
                _ => None,
            })
            .collect::<Option<Vec<u32>>>()
            .ok_or_else(|| error("ungültige codepoints"))?;
        let expected: Vec<u32> = characters.chars().map(u32::from).collect();
        if code_points != expected || !(1..=2).contains(&code_points.len()) {
            return Err(error("codepoints und characters passen nicht"));
        }
        entities.push((name.to_owned(), characters.to_owned()));
    }
    entities.sort();
    if entities.is_empty() {
        return Err("entities.json ist leer".into());
    }
    if entities
        .windows(2)
        .any(|pair| pair.first().map(|e| &e.0) == pair.get(1).map(|e| &e.0))
    {
        return Err("entities.json: ein Name kommt doppelt vor".into());
    }
    Ok(entities)
}

fn emit(entities: &[Entity]) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    writeln!(
        out,
        "// Erzeugt mit `cargo run -p datagen` aus der Tabelle der benannten"
    )?;
    writeln!(
        out,
        "// Zeichenreferenzen des WHATWG HTML Standard ({DATA}). Nicht von Hand ändern."
    )?;
    writeln!(out, "//")?;
    writeln!(
        out,
        "// Daten: © WHATWG (Apple, Google, Mozilla, Microsoft), in Quelltext unter der"
    )?;
    writeln!(out, "// BSD-3-Clause-Lizenz, siehe {DATA}/LICENSE.txt.")?;
    writeln!(out)?;
    let longest = entities
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    writeln!(out, "/// Die Länge des längsten Namens in Zeichen.")?;
    writeln!(out, "pub const LONGEST_NAME: usize = {longest};")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Name ohne das führende `&` und die Zeichen, nach Bytes sortiert."
    )?;
    writeln!(
        out,
        "pub static ENTITIES: [(&str, &str); {}] = [",
        entities.len()
    )?;
    for (name, characters) in entities {
        write!(out, "    (\"{name}\", \"")?;
        for c in characters.chars() {
            write!(out, "\\u{{{:X}}}", u32::from(c))?;
        }
        writeln!(out, "\"),")?;
    }
    writeln!(out, "];")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_entities_json() {
        let text = r#"{
            "&amp;": { "codepoints": [38], "characters": "&" },
            "&AMP": { "codepoints": [38], "characters": "&" },
            "&NotEqualTilde;": { "codepoints": [8770, 824], "characters": "≂̸" }
        }"#;
        let entities = parse_entities(text).expect("gültig");
        let names: Vec<&str> = entities.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["AMP", "NotEqualTilde;", "amp;"]);
        let source = emit(&entities).expect("schreibbar");
        assert!(source.contains("pub const LONGEST_NAME: usize = 14;"));
        assert!(source.contains("(\"NotEqualTilde;\", \"\\u{2242}\\u{338}\"),"));
        for broken in [
            "[]",
            "{}",
            r#"{"amp;": {"codepoints": [38], "characters": "&"}}"#,
            r#"{"&a-b;": {"codepoints": [38], "characters": "&"}}"#,
            r#"{"&;": {"codepoints": [38], "characters": "&"}}"#,
            r#"{"&amp;": {"codepoints": [39], "characters": "&"}}"#,
            r#"{"&amp;": {"codepoints": [], "characters": ""}}"#,
            r#"{"&amp;": {"characters": "&"}}"#,
            r#"{"&amp;": {"codepoints": [38], "characters": "&"}, "&amp;": {"codepoints": [38], "characters": "&"}}"#,
        ] {
            assert!(parse_entities(broken).is_err(), "{broken}");
        }
    }
}
