//! Benannte Zeichenreferenzen (HTML Standard, Abschnitt „Named character
//! references“). Die Tabelle erzeugt `cargo run -p datagen` aus `data/html`.

#[rustfmt::skip]
mod tables;

use mb_base::CodePoint;

use tables::{ENTITIES, LONGEST_NAME};

/// Der längste Name aus der Tabelle, mit dem `input` beginnt (ohne das
/// führende `&`): seine Länge in Zeichen und die Zeichen, für die er steht.
pub(crate) fn longest_match(input: &[CodePoint]) -> Option<(usize, &'static str)> {
    // Namen bestehen aus ASCII-Buchstaben und -Ziffern, manche enden mit `;`.
    let mut candidate = String::new();
    for c in input.iter().filter_map(|code_point| code_point.to_char()) {
        if candidate.len() == LONGEST_NAME || !(c.is_ascii_alphanumeric() || c == ';') {
            break;
        }
        candidate.push(c);
        if c == ';' {
            break;
        }
    }
    (1..=candidate.len()).rev().find_map(|length| {
        let prefix = candidate.get(..length)?;
        let index = ENTITIES
            .binary_search_by(|&(name, _)| name.cmp(prefix))
            .ok()?;
        ENTITIES
            .get(index)
            .map(|&(_, characters)| (length, characters))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(text: &str) -> Option<(usize, &'static str)> {
        let input: Vec<CodePoint> = text.chars().map(CodePoint::from).collect();
        longest_match(&input)
    }

    #[test]
    fn longest_prefix_wins() {
        assert_eq!(lookup("amp;"), Some((4, "&")));
        assert_eq!(lookup("amp"), Some((3, "&")));
        assert_eq!(lookup("notin;"), Some((6, "\u{2209}")));
        assert_eq!(lookup("notit;"), Some((3, "\u{AC}")));
        assert_eq!(lookup("NotEqualTilde;x"), Some((14, "\u{2242}\u{338}")));
        assert_eq!(
            lookup("CounterClockwiseContourIntegral;"),
            Some((32, "\u{2233}"))
        );
        assert_eq!(lookup("xyz;"), None);
        assert_eq!(lookup(""), None);
        assert_eq!(lookup("am"), None);
    }

    #[test]
    fn table_is_sorted() {
        assert_eq!(ENTITIES.len(), 2231);
        assert!(ENTITIES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(
            ENTITIES.iter().map(|(name, _)| name.len()).max(),
            Some(LONGEST_NAME)
        );
    }
}
