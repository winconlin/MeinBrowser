//! Bewusst nur der benötigte Ausschnitt der Test262-Metadaten, kein YAML-
//! Parser. Nicht unterstützte Ausführungsanforderungen sind Fehler.

use std::collections::BTreeSet;

use crate::agent::ErrorKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Sloppy,
    Strict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Negative {
    Parse,
    Runtime(ErrorKind),
}

#[derive(Debug, Default)]
pub(super) struct Metadata {
    pub(super) flags: Vec<String>,
    pub(super) includes: Vec<String>,
    pub(super) features: Vec<String>,
    pub(super) negative: Option<Negative>,
}

/// Plattformunabhängige relative Pfade, auch auf Windows ohne Laufwerk,
/// Backslashes, Elternpfade oder leere Komponenten.
pub(super) fn relative_path(text: &str) -> bool {
    !text.is_empty()
        && text.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
}

fn token(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._/-".contains(&byte))
    {
        return Err(format!("nicht unterstützter Metadatenwert {value:?}"));
    }
    Ok(value.to_owned())
}

fn inline_list(value: &str) -> Result<Vec<String>, String> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let items = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| format!("keine einzeilige Liste: {value}"))?;
    if items.trim().is_empty() {
        return Ok(Vec::new());
    }
    items.split(',').map(token).collect()
}

impl Metadata {
    pub(super) fn read(source: &str) -> Result<Self, String> {
        let (_, rest) = source
            .split_once("/*---")
            .ok_or("Test262-Metadaten fehlen")?;
        let (yaml, _) = rest.split_once("---*/").ok_or("Metadaten nicht beendet")?;
        let mut result = Self::default();
        let mut keys = BTreeSet::new();
        let mut current = "";
        let mut phase = None;
        let mut error_type = None;
        for line in yaml.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(item) = trimmed.strip_prefix("- ") {
                let list = match current {
                    "flags" => Some(&mut result.flags),
                    "includes" => Some(&mut result.includes),
                    "features" => Some(&mut result.features),
                    _ => None,
                };
                if let Some(list) = list {
                    list.push(token(item)?);
                }
            } else if !line.starts_with([' ', '\t']) {
                let Some((key, value)) = trimmed.split_once(':') else {
                    return Err(format!("Metadatenzeile nicht unterstützt: {trimmed}"));
                };
                current = key;
                if !matches!(key, "flags" | "includes" | "features" | "negative") {
                    if key == "locale" {
                        return Err("locale wird noch nicht ausgeführt".to_owned());
                    }
                    continue;
                }
                if !keys.insert(key) {
                    return Err(format!("doppelter Metadatenschlüssel: {key}"));
                }
                let value = value.trim();
                match key {
                    "flags" => result.flags = inline_list(value)?,
                    "includes" => result.includes = inline_list(value)?,
                    "features" => result.features = inline_list(value)?,
                    "negative" if !value.is_empty() => {
                        return Err("negative braucht phase und type als Block".to_owned());
                    }
                    _ => {}
                }
            } else if current == "negative" {
                let (key, value) = trimmed
                    .split_once(':')
                    .ok_or("negative-Eintrag ohne Doppelpunkt")?;
                let field = match key {
                    "phase" => &mut phase,
                    "type" => &mut error_type,
                    _ => return Err(format!("unbekannter negative-Schlüssel: {key}")),
                };
                if field.replace(token(value)?).is_some() {
                    return Err(format!("doppelter negative-Schlüssel: {key}"));
                }
            } else if matches!(current, "flags" | "includes" | "features") {
                return Err(format!("Listenfortsetzung nicht unterstützt: {trimmed}"));
            }
        }
        let mut flags = BTreeSet::new();
        for flag in &result.flags {
            if !matches!(
                flag.as_str(),
                "raw" | "noStrict" | "onlyStrict" | "generated"
            ) {
                return Err(format!("Flag wird noch nicht ausgeführt: {flag}"));
            }
            if !flags.insert(flag) {
                return Err(format!("doppeltes Flag: {flag}"));
            }
        }
        if (result.raw() && (result.flag("noStrict") || result.flag("onlyStrict")))
            || (result.flag("noStrict") && result.flag("onlyStrict"))
        {
            return Err("widersprüchliche Modusflags".to_owned());
        }
        if result
            .includes
            .iter()
            .any(|name| !relative_path(name) || !name.ends_with(".js") || name.contains("_FIXTURE"))
        {
            return Err("ungültiger Harness-Pfad".to_owned());
        }
        if result.raw() && !result.includes.is_empty() {
            return Err("raw mit includes wird nicht unterstützt".to_owned());
        }
        if keys.contains("negative") {
            result.negative = Some(match (phase.as_deref(), error_type.as_deref()) {
                (Some("parse"), Some("SyntaxError")) => Negative::Parse,
                (Some("runtime"), Some(name)) => Negative::Runtime(match name {
                    "RangeError" => ErrorKind::RangeError,
                    "ReferenceError" => ErrorKind::ReferenceError,
                    "SyntaxError" => ErrorKind::SyntaxError,
                    "TypeError" => ErrorKind::TypeError,
                    _ => return Err(format!("Fehlerart wird noch nicht geprüft: {name}")),
                }),
                _ => {
                    return Err(
                        "negative-Phase oder Fehlerart fehlt/ist nicht unterstützt".to_owned()
                    );
                }
            });
        }
        Ok(result)
    }

    fn flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|value| value == flag)
    }

    pub(super) fn raw(&self) -> bool {
        self.flag("raw")
    }

    pub(super) fn modes(&self) -> Vec<Mode> {
        if self.flag("onlyStrict") {
            vec![Mode::Strict]
        } else if self.raw() || self.flag("noStrict") {
            vec![Mode::Sloppy]
        } else {
            vec![Mode::Sloppy, Mode::Strict]
        }
    }
}
