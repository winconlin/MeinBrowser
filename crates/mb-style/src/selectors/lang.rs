//! Sprachbereiche für `:lang()`: Wohlgeformtheit nach RFC 5646 (BCP 47) und
//! erweiterte Filterung nach RFC 4647, Abschnitt 3.3.2.
//!
//! Selectors Level 4 verlangt außerdem, Sprache und Bereich vorher nach
//! RFC 5646, Abschnitt 4.5, zu kanonisieren und nur *gültige* Tags passen zu
//! lassen. Beides braucht die IANA-Registry der Subtags und fehlt noch;
//! geprüft wird nur, ob Tags wohlgeformt sind.

use mb_base::DomString;

/// Die Tags, die RFC 5646 aus RFC 3066 übernimmt (*grandfathered*), klein
/// geschrieben. Die *irregular* passen nicht zur übrigen Grammatik.
const GRANDFATHERED: [&str; 26] = [
    "en-gb-oed",
    "i-ami",
    "i-bnn",
    "i-default",
    "i-enochian",
    "i-hak",
    "i-klingon",
    "i-lux",
    "i-mingo",
    "i-navajo",
    "i-pwn",
    "i-tao",
    "i-tay",
    "i-tsu",
    "sgn-be-fr",
    "sgn-be-nl",
    "sgn-ch-de",
    "art-lojban",
    "cel-gaulish",
    "no-bok",
    "no-nyn",
    "zh-guoyu",
    "zh-hakka",
    "zh-min",
    "zh-min-nan",
    "zh-xiang",
];

/// Ob die Sprache eines Elements zu einem Sprachbereich aus `:lang()` passt.
/// Die leere Sprache steht für „nicht ausgezeichnet“; zu ihr passt nur der
/// leere Bereich, und der leere Bereich passt zu nichts anderem.
pub(super) fn matches(language: &DomString, range: &DomString) -> bool {
    let (Some(language), Some(range)) = (language.as_ascii(), range.as_ascii()) else {
        return false;
    };
    if language.is_empty() || range.is_empty() {
        return language.is_empty() && range.is_empty();
    }
    // Der Bereich muss ein wohlgeformtes Tag sein, wenn man ein `*` am
    // Anfang durch ein gültiges Subtag ersetzt. Ein `*` an anderer Stelle
    // erlaubt Selectors Level 4 nicht.
    let tag = match range.strip_prefix('*') {
        Some(rest) => format!("und{rest}"),
        None => range.to_owned(),
    };
    is_well_formed(language) && is_well_formed(&tag) && extended_filter(range, language)
}

/// *Extended Filtering* (RFC 4647, Abschnitt 3.3.2), ohne Unterschied
/// zwischen ASCII-Groß- und -Kleinbuchstaben.
fn extended_filter(range: &str, tag: &str) -> bool {
    let mut range = range.split('-');
    let mut tag = tag.split('-');
    let (Some(first_range), Some(first_tag)) = (range.next(), tag.next()) else {
        return false;
    };
    if first_range != "*" && !first_range.eq_ignore_ascii_case(first_tag) {
        return false;
    }
    for subtag in range {
        // Ein Platzhalter passt auf beliebig viele Subtags.
        if subtag == "*" {
            continue;
        }
        loop {
            let Some(current) = tag.next() else {
                return false;
            };
            if current.eq_ignore_ascii_case(subtag) {
                break;
            }
            // Über ein Singleton geht die Suche nicht hinweg.
            if current.len() == 1 {
                return false;
            }
        }
    }
    true
}

/// Geht ein Subtag weiter, wenn es `test` erfüllt.
fn advance(rest: &mut &[&str], test: impl Fn(&str) -> bool) -> bool {
    let current = *rest;
    match current.split_first() {
        Some((first, tail)) if test(first) => {
            *rest = tail;
            true
        }
        _ => false,
    }
}

fn is_alpha(subtag: &str) -> bool {
    subtag.bytes().all(|byte| byte.is_ascii_alphabetic())
}

fn is_digit(subtag: &str) -> bool {
    subtag.bytes().all(|byte| byte.is_ascii_digit())
}

/// Ob das Tag zur Grammatik ⟨Language-Tag⟩ von RFC 5646 passt
/// (*well-formed*).
fn is_well_formed(tag: &str) -> bool {
    let tag = tag.to_ascii_lowercase();
    if GRANDFATHERED.contains(&tag.as_str()) {
        return true;
    }
    let subtags: Vec<&str> = tag.split('-').collect();
    let valid_subtag = |subtag: &&str| {
        (1..=8).contains(&subtag.len()) && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
    };
    if !subtags.iter().all(valid_subtag) {
        return false;
    }
    let mut rest = subtags.as_slice();
    // privateuse = "x" 1*("-" (1*8alphanum))
    if rest.first() == Some(&"x") {
        return rest.len() > 1;
    }
    // language = 2*3ALPHA ["-" extlang] / 4ALPHA / 5*8ALPHA
    let Some(language) = rest.first() else {
        return false;
    };
    if !advance(&mut rest, |subtag| subtag.len() >= 2 && is_alpha(subtag)) {
        return false;
    }
    // extlang = 3ALPHA *2("-" 3ALPHA)
    if language.len() <= 3 {
        for _ in 0..3 {
            if !advance(&mut rest, |subtag| subtag.len() == 3 && is_alpha(subtag)) {
                break;
            }
        }
    }
    // script = 4ALPHA
    advance(&mut rest, |subtag| subtag.len() == 4 && is_alpha(subtag));
    // region = 2ALPHA / 3DIGIT
    advance(&mut rest, |subtag| {
        (subtag.len() == 2 && is_alpha(subtag)) || (subtag.len() == 3 && is_digit(subtag))
    });
    // variant = 5*8alphanum / (DIGIT 3alphanum)
    while advance(&mut rest, |subtag| {
        subtag.len() >= 5 || (subtag.len() == 4 && subtag.starts_with(|c: char| c.is_ascii_digit()))
    }) {}
    // extension = singleton 1*("-" (2*8alphanum))
    while advance(&mut rest, |subtag| subtag.len() == 1 && subtag != "x") {
        if !advance(&mut rest, |subtag| subtag.len() >= 2) {
            return false;
        }
        while advance(&mut rest, |subtag| subtag.len() >= 2) {}
    }
    // ["-" privateuse]
    if rest.first() == Some(&"x") {
        return rest.len() > 1;
    }
    rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(language: &str, range: &str) -> bool {
        matches(&DomString::from(language), &DomString::from(range))
    }

    #[test]
    fn well_formed() {
        for tag in [
            "de",
            "zh-yue-Hant-HK",
            "sr-Latn-RS",
            "de-CH-1996",
            "en-a-bbb-x-a-ccc",
            "x-whatever",
            "i-klingon",
            "qq",
            "xyzzy",
            "es-419",
        ] {
            assert!(is_well_formed(tag), "{tag}");
        }
        for tag in [
            "",
            "en-",
            "a",
            "en--US",
            "åå",
            "de-419-DE-x",
            "en-a",
            "abcdefghi",
            "en_US",
        ] {
            assert!(!is_well_formed(tag), "{tag}");
        }
    }

    #[test]
    fn extended_filtering() {
        // Beispiele aus RFC 4647, Abschnitt 3.3.2, zum Bereich „de-*-DE“.
        for tag in [
            "de-DE",
            "de-de",
            "de-Latn-DE",
            "de-Latf-DE",
            "de-DE-x-goethe",
        ] {
            assert!(extended_filter("de-*-DE", tag), "{tag}");
        }
        for tag in ["de", "de-x-DE", "de-Deva"] {
            assert!(!extended_filter("de-*-DE", tag), "{tag}");
        }
    }

    #[test]
    fn selectors_rules() {
        assert!(lang("en-US", "en"));
        assert!(lang("EN-us", "en-US"));
        assert!(!lang("en", "en-US"));
        assert!(lang("de-CH", "*-CH"));
        assert!(lang("und", "*"));
        assert!(!lang("", "*"));
        assert!(lang("", ""));
        assert!(!lang("en", ""));
        assert!(lang("xyzzy", "xyzzy"));
        assert!(!lang("xyzzy", "abcde"));
        assert!(!lang("en_US", "en"));
        assert!(!lang("de-Latn-DE", "de-*-DE"));
    }
}
