//! Tests für die Listen der Referenztests, auch gegen die WPT.

use crate::{ListError, M1_REFTESTS, RefTest, Relation, parse_list};

#[test]
fn layout_fonts_survive_trimming_until_the_fragment_tree_is_dropped() {
    let mut library =
        mb_text::FontLibrary::with_cache_budget(Box::new(mb_platform::fonts::read_font), 0);
    let font = testdata::root().join("wpt").join("fonts").join("Ahem.ttf");
    assert_eq!(library.add_file(font), 1, "Ahem fehlt");
    let fonts = mb_layout::Fonts::new(&library);
    let path = std::env::temp_dir().join("mb-font-cache-layout.html");
    let url = mb_web::file_url(&path).expect("URL");
    let html =
        b"<!DOCTYPE html><style>body { margin: 0; font: 16px Ahem; color: green }</style>AAA";
    let page = mb_web::Page::load(
        url,
        Box::new(move |_, limit| (html.len() as u64 <= limit).then(|| html.to_vec())),
    )
    .expect("Seite");
    let viewport = mb_web::Viewport {
        width: 100.0,
        height: 40.0,
        device_pixel_ratio: 1.0,
    };
    let tree = page
        .layout_within(
            &fonts,
            viewport,
            &mb_base::Budget::new(mb_web::RENDER_STEPS),
        )
        .expect("Layout");
    let before = mb_raster::render(&page.paint(&tree, (0.0, 0.0))).expect("Bild");
    assert!(before.pixels().windows(2).any(|pair| pair[0] != pair[1]));
    let held = library.trim_cache();
    assert!(
        held.active_bytes > 0,
        "der Fragmentbaum hält die Schrift fest"
    );
    assert_eq!(held.unused_bytes, 0);
    let after = mb_raster::render(&page.paint(&tree, (0.0, 0.0))).expect("erneutes Bild");
    assert_eq!(before.pixels(), after.pixels());
    assert_eq!(library.cache_stats().loads, held.loads, "kein Nachladen");
    drop(tree);
    let released = library.trim_cache();
    assert_eq!(
        released.resident_bytes, 0,
        "kein unbemerkter Schriftverweis"
    );
    assert_eq!(released.active_bytes, 0);
}

#[test]
fn list_format() {
    let list = "# Kommentar\n\n== a/test.html a/ref.html\n  != b.xht   c.xht  \n";
    assert_eq!(
        parse_list(list),
        Ok(vec![
            RefTest {
                relation: Relation::Match,
                test: "a/test.html".into(),
                reference: "a/ref.html".into(),
            },
            RefTest {
                relation: Relation::Mismatch,
                test: "b.xht".into(),
                reference: "c.xht".into(),
            },
        ])
    );
    let error = |line, message| Err(ListError { line, message });
    assert_eq!(
        parse_list("== a.html\n"),
        error(1, "Test und Referenz erwartet")
    );
    assert_eq!(
        parse_list("\n== a.html b.html c.html\n"),
        error(2, "Test und Referenz erwartet")
    );
    assert_eq!(
        parse_list("a.html b.html\n"),
        error(1, "`==` oder `!=` erwartet")
    );
}

/// Der Pfad, auf den `href` aus der Datei `base` zeigt, relativ zur Wurzel
/// der WPT; `None`, wenn er aus der Wurzel hinausführt.
fn resolve(base: &str, href: &str) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    if !href.starts_with('/') {
        segments.extend(base.split('/'));
        segments.pop();
    }
    for segment in href.trim_start_matches('/').split('/') {
        match segment {
            "." => {}
            ".." => {
                segments.pop()?;
            }
            _ => segments.push(segment),
        }
    }
    Some(segments.join("/"))
}

/// Der Wert eines Attributs in einem Tag wie `<link rel="match" href="…">`,
/// mit oder ohne Anführungszeichen; der Name ohne Rücksicht auf Groß- und
/// Kleinschreibung.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower.get(from..)?.find(name) {
        let start = from.checked_add(found)?;
        let after = start.checked_add(name.len())?;
        from = after;
        // Nur ein ganzes Attribut, nicht das Ende eines anderen Namens.
        if !lower
            .get(..start)?
            .ends_with(|character: char| character.is_ascii_whitespace())
        {
            continue;
        }
        let rest = tag.get(after..)?.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        return match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => {
                let value = rest.get(1..)?;
                value.find(quote).and_then(|end| value.get(..end))
            }
            _ => rest
                .split(|character: char| character.is_ascii_whitespace() || character == '>')
                .next(),
        };
    }
    None
}

/// Die Referenzen, die eine Datei mit `<link rel="match">` und
/// `<link rel="mismatch">` nennt.
fn references(path: &str, text: &str) -> Vec<(Relation, String)> {
    let mut found = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(start) = lower.get(from..).and_then(|rest| rest.find("<link")) {
        let start = from.saturating_add(start);
        let end = lower
            .get(start..)
            .and_then(|rest| rest.find('>'))
            .map_or(text.len(), |end| start.saturating_add(end));
        from = end;
        let Some(tag) = text.get(start..end) else {
            continue;
        };
        let relation = match attribute(tag, "rel")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("match") => Relation::Match,
            Some("mismatch") => Relation::Mismatch,
            _ => continue,
        };
        if let Some(reference) = attribute(tag, "href").and_then(|href| resolve(path, href)) {
            found.push((relation, reference));
        }
    }
    found
}

#[test]
fn helpers() {
    assert_eq!(
        resolve("css/CSS2/normal-flow/a.xht", "../reference/b.xht").as_deref(),
        Some("css/CSS2/reference/b.xht")
    );
    assert_eq!(
        resolve("css/CSS2/normal-flow/a.xht", "/css/reference/b.html").as_deref(),
        Some("css/reference/b.html")
    );
    assert_eq!(
        resolve("css/CSS2/normal-flow/a.xht", "./b-ref.xht").as_deref(),
        Some("css/CSS2/normal-flow/b-ref.xht")
    );
    assert_eq!(resolve("a.xht", "../../b.xht"), None);
    let text = r#"<LINK rel="help" href="x"/><link href='r.xht' REL="match" /><link rel=match href="s.xht">"#;
    assert_eq!(
        references("d/t.xht", text),
        [
            (Relation::Match, "d/r.xht".to_owned()),
            (Relation::Match, "d/s.xht".to_owned())
        ]
    );
    assert_eq!(
        attribute(r#"<link data-rel="x" rel="match">"#, "rel"),
        Some("match")
    );
}

/// Die Liste für M1 ist sortiert, ohne doppelte Tests, und stimmt mit den
/// WPT überein: Jeder Test nennt genau die Referenzen aus der Liste, und alle
/// Dateien sind da.
#[test]
fn m1_list() {
    let list = parse_list(M1_REFTESTS).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(list.len(), 398);
    for pair in list.windows(2) {
        if let [first, second] = pair {
            assert!(
                first.test < second.test,
                "{} vor {}",
                first.test,
                second.test
            );
        }
    }
    let mut failures = Vec::new();
    for entry in &list {
        if !entry.test.starts_with("css/CSS2/normal-flow/") {
            failures.push(format!("{}: nicht aus css/CSS2/normal-flow/", entry.test));
        }
        // Nur die Tags `<link>` zählen, und die sind ASCII.
        let bytes = testdata::read_bytes("wpt", &entry.test);
        let text = String::from_utf8_lossy(&bytes);
        let expected = vec![(entry.relation, entry.reference.clone())];
        let actual = references(&entry.test, &text);
        if actual != expected {
            failures.push(format!(
                "{}: Referenzen {actual:?} statt {expected:?}",
                entry.test
            ));
        }
        testdata::read_bytes("wpt", &entry.reference);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
