//! Die `META`-Zeilen am Anfang einer Datei `*.any.js` der WPT, wie sie
//! `tools/manifest/sourcefile.py` der WPT liest: Zeilen der Form
//! `// META: name=wert`, mit beliebigem Leerraum nach `//` und nach
//! `META:`. Die erste andere Zeile beendet sie.

/// Was die `META`-Zeilen einer Datei festlegen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// Die Umgebungen aus der ersten Zeile mit `global=`, ausgeschrieben
    /// wie in `parse_variants` der WPT: `worker` steht für
    /// `dedicatedworker`, `sharedworker` und `serviceworker`,
    /// `shadowrealm` für alle `shadowrealm-in-…`, unbekannte Namen für
    /// nichts. Ohne `global=` oder mit leerem Wert gelten `window` und
    /// `dedicatedworker`.
    pub globals: Vec<String>,
    /// Die Zeilen mit `script=` und `title=` in ihrer Reihenfolge. Der
    /// Wrapper der WPT für Worker setzt sie in dieser Reihenfolge nach
    /// testharness.js ein (`ClassicWorkerHandler` in
    /// `tools/serve/serve.py`).
    pub includes: Vec<Include>,
    /// `timeout=long`.
    pub long_timeout: bool,
    /// Die Werte aus `variant=`, etwa `?1-10`.
    pub variants: Vec<String>,
}

/// Eine Zeile, die der Wrapper vor der Testdatei einsetzt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Include {
    /// `script=`: ein Skript, relativ zur Datei oder, mit `/` am Anfang,
    /// zur Wurzel der WPT, so wie es dasteht.
    Script(String),
    /// `title=`: wird zu `self.META_TITLE = "…";`.
    Title(String),
}

impl Meta {
    /// Ob die Datei für eine JavaScript-Shell gedacht ist (`jsshell`).
    pub fn for_shell(&self) -> bool {
        self.globals.iter().any(|global| global == "jsshell")
    }
}

/// Liest die `META`-Zeilen vom Anfang von `source`.
pub fn parse_meta(source: &str) -> Meta {
    let mut meta = Meta::default();
    let mut global: Option<String> = None;
    for line in source.lines() {
        let Some((name, value)) = meta_line(line) else {
            break;
        };
        match name {
            // Wie in sourcefile.py gilt die erste Zeile mit `global=`.
            "global" => {
                global.get_or_insert_with(|| value.to_owned());
            }
            "script" => meta.includes.push(Include::Script(value.to_owned())),
            "timeout" => meta.long_timeout |= value == "long",
            "title" => meta.includes.push(Include::Title(value.to_owned())),
            "variant" => meta.variants.push(value.to_owned()),
            _ => {}
        }
    }
    meta.globals = globals(global.as_deref().unwrap_or(""));
    meta
}

/// Name und Wert einer Zeile `// META: name=wert`; der Name besteht aus
/// Buchstaben, Ziffern und `_` (`\w*` in sourcefile.py).
fn meta_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("//")?.trim_start();
    let rest = rest.strip_prefix("META:")?.trim_start();
    let (name, value) = rest.split_once('=')?;
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    Some((name, value.trim_end_matches('\r')))
}

/// Die Umgebungen eines Werts von `global=`, in der Reihenfolge ihres
/// ersten Auftretens.
fn globals(value: &str) -> Vec<String> {
    if value.is_empty() {
        return vec!["window".to_owned(), "dedicatedworker".to_owned()];
    }
    let mut globals: Vec<String> = Vec::new();
    for item in value.split(',') {
        for name in expand(item.trim()) {
            if !globals.iter().any(|found| found == name) {
                globals.push((*name).to_owned());
            }
        }
    }
    globals
}

/// Die Umgebungen, für die ein Name steht (`_any_variants` der WPT).
fn expand(name: &str) -> &'static [&'static str] {
    static SIMPLE: [&str; 15] = [
        "window",
        "window-module",
        "serviceworker",
        "serviceworker-module",
        "sharedworker",
        "sharedworker-module",
        "dedicatedworker",
        "dedicatedworker-module",
        "worker-module",
        "shadowrealm-in-window",
        "shadowrealm-in-shadowrealm",
        "shadowrealm-in-dedicatedworker",
        "shadowrealm-in-sharedworker",
        "shadowrealm-in-serviceworker",
        "shadowrealm-in-audioworklet",
    ];
    match name {
        "worker" => &["dedicatedworker", "sharedworker", "serviceworker"],
        "shadowrealm" => &[
            "shadowrealm-in-window",
            "shadowrealm-in-shadowrealm",
            "shadowrealm-in-dedicatedworker",
            "shadowrealm-in-sharedworker",
            "shadowrealm-in-serviceworker",
            "shadowrealm-in-audioworklet",
        ],
        "jsshell" => &["jsshell"],
        name => match SIMPLE.iter().position(|found| *found == name) {
            Some(index) => &SIMPLE[index..=index],
            None => &[],
        },
    }
}
