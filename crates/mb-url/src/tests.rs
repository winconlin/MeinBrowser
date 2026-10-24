//! Tests mit den URL-Testdaten der WPT (`url/resources/`), so wie die
//! JavaScript-Tests daneben sie benutzen, und mit den Beispielen des
//! Standards.

use mb_encoding::Encoding;
use mb_json::Value;

use crate::{Url, ValidationError, form_urlencoded};

/// Liest eine JSON-Datei aus `url/resources/` der WPT.
fn wpt(name: &str) -> Value {
    let text = testdata::read("wpt", &format!("url/resources/{name}"));
    mb_json::parse(&text).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn string<'a>(test: &'a Value, key: &str) -> &'a str {
    test.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("Schlüssel {key} fehlt: {test:?}"))
}

/// Die Testfälle einer Datei, ohne die Kommentare dazwischen.
fn cases(data: &Value) -> impl Iterator<Item = &Value> {
    data.as_array()
        .expect("eine Liste")
        .iter()
        .filter(|item| item.as_str().is_none())
}

/// Ein Getter der Klasse `URL` nach Namen.
fn get(url: &Url, attribute: &str) -> String {
    match attribute {
        "href" => url.href(),
        "origin" => url.origin().to_string(),
        "protocol" => url.protocol(),
        "username" => url.username().to_owned(),
        "password" => url.password().to_owned(),
        "host" => url.host_string(),
        "hostname" => url.hostname(),
        "port" => url.port_string(),
        "pathname" => url.pathname(),
        "search" => url.search(),
        "hash" => url.hash(),
        "searchParams" => form_urlencoded::serialize(
            form_urlencoded::parse_str(url.query().unwrap_or_default())
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
            Encoding::Utf8,
        ),
        _ => panic!("unbekanntes Attribut {attribute}"),
    }
}

/// Ein Setter der Klasse `URL` nach Namen.
fn set(url: &mut Url, attribute: &str, value: &str) {
    match attribute {
        // Ein Fehlschlag wirft in JavaScript; die URL bleibt dann gleich.
        "href" => {
            let _ = url.set_href(value);
        }
        "protocol" => url.set_protocol(value),
        "username" => url.set_username(value),
        "password" => url.set_password(value),
        "host" => url.set_host(value),
        "hostname" => url.set_hostname(value),
        "port" => url.set_port(value),
        "pathname" => url.set_pathname(value),
        "search" => url.set_search(value),
        "hash" => url.set_hash(value),
        _ => panic!("unbekanntes Attribut {attribute}"),
    }
}

/// Endet mit allen Abweichungen auf einmal, damit ein Lauf alles zeigt.
fn assert_no_failures(name: &str, total: usize, failures: &[String]) {
    assert!(total > 0, "{name}: keine Testfälle");
    assert!(
        failures.is_empty(),
        "{name}: {} von {total} Fällen weichen ab:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// `url-constructor.any.js` und `url-origin.any.js`
#[test]
fn urltestdata() {
    for name in ["urltestdata.json", "urltestdata-javascript-only.json"] {
        let data = wpt(name);
        let mut failures = Vec::new();
        let mut total = 0;
        for test in cases(&data) {
            total += 1;
            let input = string(test, "input");
            let base = test.get("base").and_then(Value::as_str);
            let result = Url::api_parse(input, base);
            if test.get("failure").and_then(Value::as_bool) == Some(true) {
                if let Ok(url) = result {
                    failures.push(format!("<{input}> gegen {base:?}: {url} statt Fehlschlag"));
                }
                continue;
            }
            let url = match result {
                Ok(url) => url,
                Err(error) => {
                    failures.push(format!("<{input}> gegen {base:?}: {error}"));
                    continue;
                }
            };
            for attribute in [
                "href",
                "origin",
                "protocol",
                "username",
                "password",
                "host",
                "hostname",
                "port",
                "pathname",
                "search",
                "searchParams",
                "hash",
            ] {
                // `origin` und `searchParams` fehlen in manchen Fällen.
                let Some(expected) = test.get(attribute).and_then(Value::as_str) else {
                    continue;
                };
                let actual = get(&url, attribute);
                if actual != expected {
                    failures.push(format!(
                        "<{input}> gegen {base:?}: {attribute} {actual:?} statt {expected:?}"
                    ));
                }
            }
        }
        assert_no_failures(name, total, &failures);
    }
}

/// `url-setters.any.js`
#[test]
fn setters() {
    let data = wpt("setters_tests.json");
    let Value::Object(attributes) = &data else {
        panic!("setters_tests.json ist kein Objekt");
    };
    let mut failures = Vec::new();
    let mut total = 0;
    for (attribute, tests) in attributes {
        if attribute == "comment" {
            continue;
        }
        for test in tests.as_array().expect("eine Liste") {
            total += 1;
            let href = string(test, "href");
            let new_value = string(test, "new_value");
            let mut url = Url::parse(href, None).unwrap_or_else(|error| panic!("{href}: {error}"));
            set(&mut url, attribute, new_value);
            let Some(Value::Object(expected)) = test.get("expected") else {
                panic!("expected fehlt: {test:?}");
            };
            for (key, value) in expected {
                let expected = value.as_str().expect("eine Zeichenkette");
                let actual = get(&url, key);
                if actual != expected {
                    failures.push(format!(
                        "<{href}>.{attribute} = {new_value:?}: {key} {actual:?} statt {expected:?}"
                    ));
                }
            }
        }
    }
    assert_no_failures("setters_tests.json", total, &failures);
}

/// Prüft einen Host in `https://…/x` wie `toascii.window.js` und
/// `IdnaTestV2.any.js`.
fn check_host(input: &str, output: Option<&str>, failures: &mut Vec<String>) {
    let result = Url::parse(&format!("https://{input}/x"), None);
    match (result, output) {
        (Ok(url), None) => failures.push(format!("{input:?}: {url} statt Fehlschlag")),
        (Err(error), Some(output)) => failures.push(format!("{input:?}: {error} statt {output:?}")),
        (Ok(url), Some(output)) => {
            let href = format!("https://{output}/x");
            let actual = [
                url.host_string(),
                url.hostname(),
                url.pathname(),
                url.href(),
            ];
            if actual != [output, output, "/x", href.as_str()] {
                failures.push(format!("{input:?}: {actual:?} statt {output:?}"));
            }
        }
        (Err(_), None) => {}
    }
}

/// `toascii.window.js`, nur mit der Klasse `URL`.
#[test]
fn toascii() {
    let data = wpt("toascii.json");
    let mut failures = Vec::new();
    let mut total = 0;
    for test in cases(&data) {
        total += 1;
        let input = string(test, "input");
        let output = test.get("output").and_then(Value::as_str);
        check_host(input, output, &mut failures);
        for attribute in ["host", "hostname"] {
            let mut url = Url::parse("https://x/x", None).expect("gültig");
            set(&mut url, attribute, input);
            let actual = get(&url, attribute);
            let expected = output.unwrap_or("x");
            if actual != expected {
                failures.push(format!(
                    "{input:?} per {attribute}: {actual:?} statt {expected:?}"
                ));
            }
        }
    }
    assert_no_failures("toascii.json", total, &failures);
}

/// `IdnaTestV2.any.js` und `IdnaTestV2-removed.any.js`
#[test]
fn idna_test_v2() {
    for name in ["IdnaTestV2.json", "IdnaTestV2-removed.json"] {
        let data = wpt(name);
        let mut failures = Vec::new();
        let mut total = 0;
        for test in cases(&data) {
            let input = string(test, "input");
            // Die leere Eingabe lässt sich über `new URL()` nicht prüfen.
            if name == "IdnaTestV2.json" && input.is_empty() {
                continue;
            }
            total += 1;
            check_host(
                input,
                test.get("output").and_then(Value::as_str),
                &mut failures,
            );
        }
        assert_no_failures(name, total, &failures);
    }
}

/// `percent-encoding.window.js`: `<a href="https://doesnotmatter.invalid/?…#…">`
/// in einem Dokument mit der jeweiligen Kodierung. Die Query folgt der
/// Kodierung, das Fragment ist immer UTF-8.
#[test]
fn percent_encoding() {
    let data = wpt("percent-encoding.json");
    let mut failures = Vec::new();
    let mut total = 0;
    for test in cases(&data) {
        let input = string(test, "input");
        let Some(Value::Object(outputs)) = test.get("output") else {
            panic!("output fehlt: {test:?}");
        };
        let fragment = test
            .get("output")
            .and_then(|output| output.get("utf-8"))
            .and_then(Value::as_str)
            .expect("Ausgabe für UTF-8");
        for (label, query) in outputs {
            total += 1;
            let query = query.as_str().expect("eine Zeichenkette");
            let encoding = Encoding::for_label(label.as_bytes()).expect("bekannte Kodierung");
            let href = format!("https://doesnotmatter.invalid/?{input}#{input}");
            let url = Url::parse_with_encoding(&href, None, encoding).expect("gültig");
            let actual = [url.search(), url.hash()];
            if actual != [format!("?{query}"), format!("#{fragment}")] {
                failures.push(format!("{input:?} mit {label}: {actual:?}"));
            }
        }
    }
    assert_no_failures("percent-encoding.json", total, &failures);
}

/// Die Beispiele aus der Tabelle der Validierungsfehler im Standard
/// (Abschnitt 1.1). Jedes muss seinen Fehler melden, und die mit „Failure“
/// müssen scheitern.
#[test]
fn validation_error_examples() {
    use ValidationError as E;
    let examples: [(&str, Option<&str>, E, bool); 38] = [
        ("https://exa%23mple.org", None, E::DomainToAscii, true),
        (
            "https://exam%70le.org",
            None,
            E::DomainPercentEncoded,
            false,
        ),
        ("foo://exa[mple.org", None, E::HostInvalidCodePoint, true),
        ("https://127.0.0.1./", None, E::Ipv4EmptyPart, false),
        ("https://1.2.3/", None, E::Ipv4TooFewParts, false),
        ("https://1.2.3.4.5/", None, E::Ipv4TooManyParts, true),
        ("https://test.42", None, E::Ipv4NonNumericPart, true),
        ("https://127.0.0x0.1", None, E::Ipv4NonDecimalPart, false),
        ("https://255.255.4000.1", None, E::Ipv4OutOfRangePart, true),
        ("https://①.②.③.④", None, E::Ipv4NonAsciiInput, false),
        ("https://[::1", None, E::Ipv6Unclosed, true),
        ("https://[:1]", None, E::Ipv6InvalidCompression, true),
        (
            "https://[1:2:3:4:5:6:7:8:9]",
            None,
            E::Ipv6TooManyPieces,
            true,
        ),
        ("https://[1::1::1]", None, E::Ipv6MultipleCompression, true),
        ("https://[1:2:3!:4]", None, E::Ipv6InvalidCodePoint, true),
        ("https://[1:2:3:]", None, E::Ipv6InvalidCodePoint, true),
        ("https://[1:2:3]", None, E::Ipv6TooFewPieces, true),
        ("https://[::01]", None, E::Ipv6PieceLeadingZero, false),
        (
            "https://[1:1:1:1:1:1:1:127.0.0.1]",
            None,
            E::Ipv4InIpv6TooManyPieces,
            true,
        ),
        (
            "https://[ffff::.0.0.1]",
            None,
            E::Ipv4InIpv6InvalidCodePoint,
            true,
        ),
        (
            "https://[ffff::127.0.xyz.1]",
            None,
            E::Ipv4InIpv6InvalidCodePoint,
            true,
        ),
        (
            "https://[ffff::127.0xyz]",
            None,
            E::Ipv4InIpv6InvalidCodePoint,
            true,
        ),
        (
            "https://[ffff::127.00.0.1]",
            None,
            E::Ipv4InIpv6InvalidCodePoint,
            true,
        ),
        (
            "https://[ffff::127.0.0.1.2]",
            None,
            E::Ipv4InIpv6InvalidCodePoint,
            true,
        ),
        (
            "https://[ffff::127.0.0.4000]",
            None,
            E::Ipv4InIpv6OutOfRangePart,
            true,
        ),
        (
            "https://[ffff::127.0.0]",
            None,
            E::Ipv4InIpv6TooFewParts,
            true,
        ),
        ("https://example.org/>", None, E::InvalidUrlUnit, false),
        (" https://example.org ", None, E::InvalidUrlUnit, false),
        ("ht\ntps://example.org", None, E::InvalidUrlUnit, false),
        ("https://example.org/%s", None, E::InvalidUrlUnit, false),
        (
            "file:c:/my-secret-folder",
            None,
            E::SpecialSchemeMissingFollowingSolidus,
            false,
        ),
        (
            "https:example.org",
            None,
            E::SpecialSchemeMissingFollowingSolidus,
            false,
        ),
        ("💩", None, E::MissingSchemeNonRelativeUrl, true),
        (
            "💩",
            Some("mailto:user@example.org"),
            E::MissingSchemeNonRelativeUrl,
            true,
        ),
        (
            "https://example.org\\path\\to\\file",
            None,
            E::InvalidReverseSolidus,
            false,
        ),
        (
            "https://user@example.org",
            None,
            E::InvalidCredentials,
            false,
        ),
        ("https://#fragment", None, E::HostMissing, true),
        ("https://example.org:70000", None, E::PortOutOfRange, true),
    ];
    // Das Beispiel des Standards für file-invalid-Windows-drive-letter,
    // `/c:/path/to/file` gegen `file:///c:/`, erreicht den Schritt nicht, der
    // den Fehler meldet: Nach dem `/` geht es über den file slash state, und
    // der meldet nichts. Der Algorithmus gilt, das Beispiel ist nur erläuternd.
    let mut errors = Vec::new();
    let base = Url::parse("file:///c:/", None).expect("gültige Basis");
    let url = Url::parse_with_errors("/c:/path/to/file", Some(&base), Encoding::Utf8, &mut errors);
    assert_eq!(
        url.map(|url| url.href()),
        Ok("file:///c:/path/to/file".into())
    );
    assert!(errors.is_empty(), "{errors:?}");
    let more: [(&str, Option<&str>, E, bool); 7] = [
        (
            "c|/path/to/file",
            Some("file:///c:/"),
            E::FileInvalidWindowsDriveLetter,
            false,
        ),
        (
            "https:foo.html",
            Some("https://example.org/"),
            E::SpecialSchemeMissingFollowingSolidus,
            false,
        ),
        ("ssh://user@example.org", None, E::InvalidCredentials, false),
        ("https://:443", None, E::HostMissing, true),
        ("https://user:pass@", None, E::HostMissing, true),
        ("https://example.org:7z", None, E::PortInvalid, true),
        (
            "file://c:",
            None,
            E::FileInvalidWindowsDriveLetterHost,
            false,
        ),
    ];
    for (input, base, expected, fails) in examples.into_iter().chain(more) {
        let base = base.map(|base| Url::parse(base, None).expect("gültige Basis"));
        let mut errors = Vec::new();
        let result = Url::parse_with_errors(input, base.as_ref(), Encoding::Utf8, &mut errors);
        assert!(
            errors.contains(&expected),
            "{input:?}: {errors:?} ohne {expected}"
        );
        match result {
            Ok(url) => assert!(!fails, "{input:?}: {url} statt Fehlschlag"),
            Err(error) => {
                assert!(fails, "{input:?}: {error}");
                assert_eq!(error.cause(), Some(expected), "{input:?}");
            }
        }
    }
    // Ohne Fehler bleibt die Liste leer.
    let mut errors = Vec::new();
    let url = Url::parse_with_errors(
        "https://example.org/a?b#c",
        None,
        Encoding::Utf8,
        &mut errors,
    );
    assert!(url.is_ok() && errors.is_empty(), "{errors:?}");
}

/// Die Beispiele zur Darstellung von URLs im Standard (Abschnitt 4.1).
#[test]
fn url_record_examples() {
    use crate::{Host, Path};
    let url = Url::parse("https://localhost:8000/search?q=text#hello", None).expect("gültig");
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host(), Some(&Host::Domain("localhost".into())));
    assert_eq!(url.port(), Some(8000));
    assert_eq!(url.path(), &Path::Segments(vec!["search".into()]));
    assert_eq!(
        (url.query(), url.fragment()),
        (Some("q=text"), Some("hello"))
    );
    let url = Url::parse("urn:isbn:9780307476463", None).expect("gültig");
    assert_eq!(url.path(), &Path::Opaque("isbn:9780307476463".into()));
    assert_eq!(url.host(), None);
    let url = Url::parse("file:///ada/Analytical%20Engine/README.md", None).expect("gültig");
    assert_eq!(url.host(), Some(&Host::Empty));
    assert_eq!(
        url.path(),
        &Path::Segments(vec![
            "ada".into(),
            "Analytical%20Engine".into(),
            "README.md".into()
        ])
    );
    // Zwei opake Ursprünge sind verschieden, auch bei derselben URL.
    let data = Url::parse("data:,", None).expect("gültig");
    assert_ne!(data.origin(), data.origin());
    let https = Url::parse("https://example.org:443/", None).expect("gültig");
    assert_eq!(https.origin(), https.origin());
    assert_eq!(https.origin().to_string(), "https://example.org");
}
