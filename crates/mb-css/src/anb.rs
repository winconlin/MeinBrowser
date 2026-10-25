//! Das An+B-Format („The An+B microsyntax“), etwa für `:nth-child()`.

use crate::{ComponentValue, Token};

/// Ein Wert im An+B-Format: alle Indizes A·n + B für n ≥ 0. Der erste
/// Eintrag einer Liste hat den Index 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnPlusB {
    pub a: i32,
    pub b: i32,
}

impl AnPlusB {
    /// Ob es ein n ≥ 0 gibt mit A·n + B = `index`.
    pub fn contains(self, index: i64) -> bool {
        let Some(offset) = index.checked_sub(i64::from(self.b)) else {
            return false;
        };
        let a = i64::from(self.a);
        if a == 0 {
            return offset == 0;
        }
        offset.checked_rem(a) == Some(0) && offset.checked_div(a).is_some_and(|n| n >= 0)
    }
}

/// Liest ⟨an+b⟩ aus einer Liste von Komponentenwerten, die nichts anderes
/// enthält. Leerraum davor und danach gehört nicht dazu.
pub fn parse_an_plus_b(values: &[ComponentValue<'_>]) -> Option<AnPlusB> {
    // Die Tokens ohne Leerraum, jeweils mit der Angabe, ob Leerraum davor
    // stand. Funktionen und Blöcke kommen in der Grammatik nicht vor.
    let mut tokens = Vec::new();
    let mut space = false;
    for value in values {
        match value {
            ComponentValue::Token(Token::Whitespace) => space = true,
            ComponentValue::Token(token) => {
                tokens.push((*token, space));
                space = false;
            }
            _ => return None,
        }
    }
    let rest = tokens.get(1..).unwrap_or_default();
    match tokens.first()?.0 {
        Token::Ident(ident) => {
            let ident = ident.to_ascii_lowercase();
            match ident.as_str() {
                "odd" if rest.is_empty() => Some(AnPlusB { a: 2, b: 1 }),
                "even" if rest.is_empty() => Some(AnPlusB { a: 2, b: 0 }),
                _ => match ident.strip_prefix('-') {
                    Some(unit) => after_n(-1, unit, rest),
                    None => after_n(1, &ident, rest),
                },
            }
        }
        // „'+'? n …“: Zwischen „+“ und dem Ident darf kein Leerraum stehen.
        Token::Delim('+') => match rest.split_first()? {
            ((Token::Ident(ident), false), rest) => {
                let ident = ident.to_ascii_lowercase();
                // „+-n“ steht nicht in der Grammatik.
                if ident.starts_with('-') {
                    return None;
                }
                after_n(1, &ident, rest)
            }
            _ => None,
        },
        Token::Number {
            value,
            integer: true,
            ..
        } if rest.is_empty() => Some(AnPlusB {
            a: 0,
            b: to_integer(*value),
        }),
        Token::Dimension {
            value,
            integer: true,
            unit,
            ..
        } => after_n(to_integer(*value), &unit.to_ascii_lowercase(), rest),
        _ => None,
    }
}

/// Der Teil ab dem „n“: `unit` ist der Rest des Idents oder die Einheit
/// (klein geschrieben) und beginnt mit „n“, `rest` sind die übrigen Tokens.
fn after_n(a: i32, unit: &str, rest: &[(&Token, bool)]) -> Option<AnPlusB> {
    let suffix = unit.strip_prefix('n')?;
    if suffix.is_empty() {
        // „n“, danach nichts, ⟨signed-integer⟩ oder ['+' | '-'] ⟨signless-integer⟩
        let b = match rest {
            [] => 0,
            [
                (
                    Token::Number {
                        value,
                        integer: true,
                        sign: Some(_),
                    },
                    _,
                ),
            ] => to_integer(*value),
            [(Token::Delim(sign @ ('+' | '-')), _), (token, _)] => {
                let value = signless_integer(token)?;
                if *sign == '-' {
                    value.saturating_neg()
                } else {
                    value
                }
            }
            _ => return None,
        };
        return Some(AnPlusB { a, b });
    }
    let digits = suffix.strip_prefix('-')?;
    if digits.is_empty() {
        // „n-“ ⟨signless-integer⟩
        let [(token, _)] = rest else {
            return None;
        };
        let b = signless_integer(token)?.saturating_neg();
        return Some(AnPlusB { a, b });
    }
    // „n-“ und Ziffern in einem Token; B ist negativ.
    if !rest.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = digits.bytes().fold(0_i32, |value, digit| {
        value
            .saturating_mul(10)
            .saturating_add(i32::from(digit.saturating_sub(b'0')))
    });
    Some(AnPlusB {
        a,
        b: value.saturating_neg(),
    })
}

/// ⟨signless-integer⟩: eine ganze Zahl ohne Vorzeichen.
fn signless_integer(token: &Token) -> Option<i32> {
    match token {
        Token::Number {
            value,
            integer: true,
            sign: None,
        } => Some(to_integer(*value)),
        _ => None,
    }
}

/// Der Wert eines ganzzahligen Tokens. Werte außerhalb von `i32` werden zum
/// nächsten darstellbaren Wert (CSS Values 4); `as` sättigt genau so.
fn to_integer(value: f64) -> i32 {
    value as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sign, Source, parse_component_value_list};

    fn parse(text: &str) -> Option<(i32, i32)> {
        let values = parse_component_value_list(&Source::new(text));
        let values: Vec<_> = values.iter().collect();
        parse_an_plus_b(&values).map(|value| (value.a, value.b))
    }

    #[test]
    fn sign_rules() {
        assert_eq!(parse("+n-1"), Some((1, -1)));
        assert_eq!(parse("-n- 3"), Some((-1, -3)));
        assert_eq!(parse("3n - -1"), None);
        assert_eq!(parse("+-n"), None);
        assert_eq!(parse("n-1-"), None);
        assert_eq!(parse("99999999999n+1"), Some((i32::MAX, 1)));
        assert_eq!(parse("n-99999999999"), Some((1, -i32::MAX)));
    }

    #[test]
    fn contains_indices() {
        let odd = AnPlusB { a: 2, b: 1 };
        assert!(odd.contains(1) && odd.contains(3) && !odd.contains(2));
        let first_three = AnPlusB { a: -1, b: 3 };
        assert!(first_three.contains(3) && first_three.contains(1));
        assert!(!first_three.contains(4));
        assert!(AnPlusB { a: 0, b: 5 }.contains(5));
        assert!(!AnPlusB { a: 0, b: 0 }.contains(1));
        assert!(!AnPlusB { a: -1, b: 0 }.contains(i64::MIN));
    }

    #[test]
    fn sign_of_number_matters() {
        // „+5“ ist ein ⟨signed-integer⟩, nach „n“ also erlaubt; „5“ nicht.
        assert_eq!(parse("n+5"), Some((1, 5)));
        assert_eq!(parse("n 5"), None);
        let number = Token::Number {
            value: 5.0,
            integer: true,
            sign: Some(Sign::Plus),
        };
        assert_eq!(signless_integer(&number), None);
    }
}
