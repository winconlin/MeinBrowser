//! Die CSS-Blöcke aus dem Abschnitt „Rendering“ des HTML Standard für
//! `mb-style` aus `rendering.json`: das Stylesheet der Benutzeragenten und
//! die *presentational hints* in CSS, je mit der Bedingung, unter der sie
//! gelten.

use std::fmt::Write;

use mb_json::Value;

/// Die Rohdaten, relativ zum Workspace.
pub const DATA: &str = "data/html-rendering";

/// Die erzeugte Datei, relativ zum Workspace.
pub const OUTPUT: &str = "crates/mb-style/src/ua/tables.rs";

/// Die Absätze vor den Blöcken und die Bedingung, die sie nennen, als Name
/// einer Variante von `Condition` in `crates/mb-style/src/ua.rs`.
const CONDITIONS: [(&str, &str); 16] = [
    ("", "Always"),
    ("The following CSS rules are expected to apply:", "Always"),
    // Ein Hinweis der Herausgeber direkt vor dem allgemeinen Block für
    // `select`; die Bedingungen stehen erst vor den folgenden Blöcken.
    (
        "Need to detail the native appearance and primitive appearance.",
        "Always",
    ),
    (
        "The following rules are also expected to apply, as presentational hints:",
        "PresentationalHint",
    ),
    (
        "The following CSS rules are expected to apply as presentational hints:",
        "PresentationalHint",
    ),
    (
        "In quirks mode, the following rules are also expected to apply:",
        "Quirks",
    ),
    (
        "The following CSS rules are expected to apply when the Document is in quirks mode:",
        "Quirks",
    ),
    (
        "In HTML documents, the following rules are also expected to apply:",
        "HtmlDocument",
    ),
    (
        "When the document's character encoding is ISO-8859-8, the following rules are \
         additionally expected to apply, following those above: ENCODING",
        "Iso8859_8",
    ),
    (
        "The third child element is either a link or style element with the following \
         styles for the default summary:",
        "DetailsShadowTree",
    ),
    (
        "The following styles are expected to apply to select elements when they are being \
         rendered as a drop-down box:",
        "SelectDropDown",
    ),
    (
        "The following styles are expected to apply to select elements when they are being \
         rendered with native appearance or primitive appearance:",
        "SelectNative",
    ),
    (
        "The following styles are expected to apply to select elements when they are being \
         rendered as a drop-down box with native appearance or primitive appearance:",
        "SelectDropDownNative",
    ),
    (
        "The following styles are expected to apply to select elements when they are being \
         rendered with base appearance:",
        "SelectBase",
    ),
    (
        "The following styles are expected to apply to select elements when they are being \
         rendered as a drop-down box with base appearance:",
        "SelectDropDownBase",
    ),
    (
        "The following styles are expected to apply to select elements when they are being \
         rendered as a list box with base appearance:",
        "SelectListBoxBase",
    ),
];

/// Ein Block: Überschrift, Bedingung und CSS.
struct Block {
    section: String,
    condition: &'static str,
    css: String,
}

/// Erzeugt den Quelltext von `ua/tables.rs`. `read` liefert den Inhalt
/// einer Datei aus [`DATA`].
pub fn run(read: &dyn Fn(&str) -> Result<String, String>) -> Result<String, String> {
    let blocks = parse_blocks(&read("rendering.json")?)?;
    emit(&blocks).map_err(|_| "Fehler beim Schreiben des Quelltexts".to_owned())
}

fn parse_blocks(text: &str) -> Result<Vec<Block>, String> {
    let root = mb_json::parse(text).map_err(|error| format!("rendering.json: {error}"))?;
    let entries = root.as_array().ok_or("rendering.json: Liste erwartet")?;
    let mut blocks = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let error = |message: &str| format!("rendering.json, Block {}: {message}", index + 1);
        let field = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| error(&format!("ohne {key}")))
        };
        let section = field("section")?;
        // Die Überschrift steht als Kommentar im Quelltext.
        if section.is_empty() || section.contains(['\n', '\r']) {
            return Err(error("Überschrift leer oder mehrzeilig"));
        }
        let intro = field("intro")?;
        let condition = CONDITIONS
            .iter()
            .find(|(text, _)| *text == intro)
            .map(|(_, condition)| *condition)
            .ok_or_else(|| error(&format!("unbekannter Absatz {intro:?}")))?;
        let lines = entry
            .get("css")
            .and_then(Value::as_array)
            .ok_or_else(|| error("ohne css"))?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<&str>>>()
            .ok_or_else(|| error("css enthält etwas anderes als Zeilen"))?;
        let css = lines.join("\n");
        if css.trim().is_empty() {
            return Err(error("leerer Block"));
        }
        // Der Block steht im Quelltext als `r##"…"##`.
        if css.contains("\"##") {
            return Err(error("enthält \"##"));
        }
        blocks.push(Block {
            section: section.to_owned(),
            condition,
            css,
        });
    }
    if blocks.is_empty() {
        return Err("rendering.json ist leer".into());
    }
    Ok(blocks)
}

fn emit(blocks: &[Block]) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    writeln!(
        out,
        "// Erzeugt mit `cargo run -p datagen` aus den CSS-Blöcken des Abschnitts"
    )?;
    writeln!(
        out,
        "// „Rendering“ des WHATWG HTML Standard ({DATA}). Nicht von Hand ändern."
    )?;
    writeln!(out, "//")?;
    writeln!(
        out,
        "// Daten: © WHATWG (Apple, Google, Mozilla, Microsoft), in Quelltext unter der"
    )?;
    writeln!(out, "// BSD-3-Clause-Lizenz, siehe {DATA}/LICENSE.txt.")?;
    writeln!(out)?;
    writeln!(out, "use super::{{Block, Condition}};")?;
    writeln!(out)?;
    writeln!(out, "/// Die Blöcke in der Reihenfolge des Standards.")?;
    writeln!(
        out,
        "pub(super) static BLOCKS: [Block; {}] = [",
        blocks.len()
    )?;
    for block in blocks {
        writeln!(out, "    // {}", block.section)?;
        writeln!(out, "    Block {{")?;
        writeln!(out, "        condition: Condition::{},", block.condition)?;
        writeln!(out, "        css: r##\"{}\"##,", block.css)?;
        writeln!(out, "    }},")?;
    }
    writeln!(out, "];")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rendering_json() {
        let text = r#"[
            {"section": "The page", "intro": "", "css": ["html, body { display: block; }"]},
            {
                "section": "Flow content",
                "intro": "In quirks mode, the following rules are also expected to apply:",
                "css": ["form {", "  margin-block-end: 1em;", "}"]
            }
        ]"#;
        let blocks = parse_blocks(text).expect("gültig");
        let source = emit(&blocks).expect("schreibbar");
        assert!(source.contains("pub(super) static BLOCKS: [Block; 2] = ["));
        assert!(
            source.contains(
                "    // Flow content\n    Block {\n        condition: Condition::Quirks,\n"
            )
        );
        assert!(source.contains("        css: r##\"form {\n  margin-block-end: 1em;\n}\"##,\n"));
        for broken in [
            "{}",
            "[]",
            r#"[{"section": "x", "intro": "Irgendwann:", "css": ["a {}"]}]"#,
            r#"[{"section": "x", "intro": "", "css": []}]"#,
            r#"[{"section": "x", "intro": "", "css": [1]}]"#,
            r###"[{"section": "x", "intro": "", "css": ["a[x=\"##\"] {}"]}]"###,
            r#"[{"intro": "", "css": ["a {}"]}]"#,
            r#"[{"section": "a\nb", "intro": "", "css": ["a {}"]}]"#,
        ] {
            assert!(parse_blocks(broken).is_err(), "{broken}");
        }
    }
}
