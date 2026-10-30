//! Bedingungen für `@supports`, `@import … supports()` und `CSS.supports()`
//! nach CSS Conditional 3 bis 5 (Editor's Drafts, Stand 23.10.2026).
//!
//! Erfüllt sind:
//! - eine Deklaration, die eine Stilregel annähme. Custom Properties fehlen
//!   noch, wie überall, und damit auch `var()`;
//! - `selector()` mit einem gültigen ⟨complex-selector⟩, siehe
//!   [`SelectorList::supports`];
//! - `at-rule()` mit einer At-Regel, die dieser Browser irgendwo annimmt:
//!   `@import`, `@namespace`, `@layer`, `@media` und `@supports`.
//!
//! Nie erfüllt sind `font-tech()` und `font-format()`, denn ohne
//! `@font-face` lassen sich Schriften aus Stylesheets nicht verwenden,
//! `named-feature()`, denn keines der beiden Merkmale gibt es, `env()`,
//! denn es gibt keine Umgebungsvariablen, und benannte Bedingungen, denn
//! `@supports-condition` fehlt.

#[cfg(test)]
mod wpt;

use mb_css::{
    BlockKind, ComponentValue, DeclarationValues, Source, Token, Values, parse_component_value_list,
};

use crate::media::{Cursor, is_any_value, trim};
use crate::properties::PropertyId;
use crate::selectors::{Namespaces, SelectorList};
use crate::values::ParseContext;

/// Wie tief Bedingungen verschachtelt sein dürfen; tiefere werden zu
/// ⟨general-enclosed⟩. Das Lesen steigt rekursiv ab, die Grenze schützt
/// vor einem Stapelüberlauf.
const MAX_DEPTH: usize = 100;

/// Die At-Regeln, die dieser Browser annimmt.
const AT_RULES: &[&str] = &["import", "namespace", "layer", "media", "supports"];

/// Wofür eine Bedingung gelesen wird: die Namensräume für `selector()` und
/// der Kontext für die Werte von Deklarationen.
#[derive(Clone, Copy)]
pub(crate) struct Environment<'a> {
    pub namespaces: &'a Namespaces,
    pub values: ParseContext<'a>,
}

/// Eine ⟨supports-condition⟩: `None`, wenn sie nicht zur Grammatik passt,
/// sonst ob sie erfüllt ist.
pub(crate) fn condition(
    values: &[ComponentValue<'_>],
    environment: Environment<'_>,
) -> Option<bool> {
    condition_at(values, environment, 0)
}

/// Die Bedingung in `@import … supports( [ <supports-condition> |
/// <declaration> ] )`; eine Deklaration steht für sich in Klammern.
pub(crate) fn import_condition(values: Values<'_>, environment: Environment<'_>) -> Option<bool> {
    let list: Vec<ComponentValue<'_>> = values.iter().collect();
    condition(&list, environment).or_else(|| {
        values
            .declaration()
            .map(|declaration| supports_declaration_values(declaration, environment))
    })
}

/// Ob eine Stilregel die Deklaration annähme; `!important` ändert daran
/// nichts.
fn supports_declaration_values(
    declaration: DeclarationValues<'_>,
    environment: Environment<'_>,
) -> bool {
    PropertyId::from_name(declaration.name).is_some_and(|property| {
        property
            .parse(declaration.value, environment.values)
            .is_some()
    })
}

fn condition_at(
    values: &[ComponentValue<'_>],
    environment: Environment<'_>,
    depth: usize,
) -> Option<bool> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut input = Cursor::new(values);
    if input.keyword("not") {
        let inner = in_parens(input.next()?, environment, depth)?;
        return input.at_end().then_some(!inner);
    }
    let mut result = in_parens(input.next()?, environment, depth)?;
    // Ob die Bedingungen mit `or` verbunden sind; mischen geht nicht.
    let mut disjunction = None;
    while !input.at_end() {
        let is_or = if input.keyword("and") {
            false
        } else if input.keyword("or") {
            true
        } else {
            return None;
        };
        if disjunction.is_some_and(|previous| previous != is_or) {
            return None;
        }
        disjunction = Some(is_or);
        // Auch wenn das Ergebnis schon feststeht: Der Rest muss passen.
        let next = in_parens(input.next()?, environment, depth)?;
        result = if is_or {
            result || next
        } else {
            result && next
        };
    }
    Some(result)
}

/// Eine ⟨supports-in-parens⟩. ⟨general-enclosed⟩ gilt erst, wenn nichts
/// anderes passt, und ist nie erfüllt.
fn in_parens(
    value: &ComponentValue<'_>,
    environment: Environment<'_>,
    depth: usize,
) -> Option<bool> {
    match value {
        ComponentValue::Block {
            kind: BlockKind::Parenthesis,
            value,
        } => {
            let values: Vec<ComponentValue<'_>> = value.iter().collect();
            if let Some(result) = condition_at(&values, environment, depth.saturating_add(1)) {
                return Some(result);
            }
            // ⟨supports-decl⟩ mit einer Deklaration oder dem Namen einer
            // benannten Bedingung.
            if let Some(declaration) = value.declaration() {
                return Some(supports_declaration_values(declaration, environment));
            }
            if let [ComponentValue::Token(Token::Ident(_))] = trim(&values) {
                return Some(false);
            }
            is_any_value(*value).then_some(false)
        }
        ComponentValue::Function { name, value } => {
            let is = |function: &str| name.eq_ignore_ascii_case(function);
            let supported = if is("selector") {
                SelectorList::supports(*value, environment.namespaces)
            } else if is("at-rule") {
                supports_at_rule(*value)
            } else {
                false
            };
            if supported {
                Some(true)
            } else {
                is_any_value(*value).then_some(false)
            }
        }
        _ => None,
    }
}

/// `at-rule( <at-keyword-token> )`
fn supports_at_rule(values: Values<'_>) -> bool {
    let values: Vec<ComponentValue<'_>> = values.iter().collect();
    match trim(&values) {
        [ComponentValue::Token(Token::AtKeyword(name))] => {
            AT_RULES.iter().any(|rule| rule.eq_ignore_ascii_case(name))
        }
        _ => false,
    }
}

/// `CSS.supports(property, value)`: ob `property` ohne Unterschied zwischen
/// ASCII-Groß- und -Kleinbuchstaben eine unterstützte Eigenschaft ist und
/// `value` nach ihrer Grammatik gültig. Leerraum im Namen bleibt.
pub fn supports_declaration(property: &str, value: &str) -> bool {
    let Some(property) = PropertyId::from_name(property) else {
        return false;
    };
    let values = parse_component_value_list(&Source::new(value));
    property
        .parse(values.values(), ParseContext::default())
        .is_some()
}

/// `CSS.supports(conditionText)`: ob der Text eine erfüllte
/// ⟨supports-condition⟩ ist, so oder in Klammern. Namensräume gibt es dabei
/// keine.
pub fn supports_condition(text: &str) -> bool {
    let namespaces = Namespaces::default();
    let environment = Environment {
        namespaces: &namespaces,
        values: ParseContext::default(),
    };
    [text.to_owned(), format!("({text})")].iter().any(|text| {
        let values = parse_component_value_list(&Source::new(text));
        let values: Vec<ComponentValue<'_>> = values.iter().collect();
        condition(&values, environment) == Some(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wie in einem Stylesheet: `None` für eine ungültige Bedingung.
    fn rule(text: &str) -> Option<bool> {
        let namespaces = Namespaces::default();
        let values = parse_component_value_list(&Source::new(text));
        let values: Vec<ComponentValue<'_>> = values.iter().collect();
        condition(
            &values,
            Environment {
                namespaces: &namespaces,
                values: ParseContext::default(),
            },
        )
    }

    #[test]
    fn grammar() {
        assert_eq!(rule("(color: red)"), Some(true));
        assert_eq!(rule("( color : red ! important )"), Some(true));
        assert_eq!(rule("(color: red) and (colour: red)"), Some(false));
        assert_eq!(rule("(color: red) or (colour: red)"), Some(true));
        assert_eq!(rule("not (colour: red)"), Some(true));
        assert_eq!(rule("((color: red) and (width: 0))"), Some(true));
        // Ohne Klammern, gemischt oder mit `not` neben `and` ungültig.
        assert_eq!(rule("color: red"), None);
        assert_eq!(rule("(a: b) and (c: d) or (e: f)"), None);
        assert_eq!(rule("not (a: b) and (c: d)"), None);
        assert_eq!(rule("(a: b) or(c: d)"), None);
        assert_eq!(rule("(a: b) and"), None);
        assert_eq!(rule(""), None);
        // ⟨general-enclosed⟩ ist nie erfüllt, aber gültig; `not(` ist eine
        // Funktion.
        assert_eq!(rule("(foo bar)"), Some(false));
        assert_eq!(rule("not (foo bar)"), Some(true));
        assert_eq!(rule("not(color: red)"), Some(false));
        assert_eq!(rule("foo(1) or (color: red)"), Some(true));
        assert_eq!(rule("()"), Some(false));
        // Kein ⟨any-value⟩.
        assert_eq!(rule("(a ])"), None);
        assert_eq!(rule("not (\"a\n)"), None);
        // Eine Deklaration ohne Wert ist gültig, aber nicht erfüllt; ein
        // Semikolon gehört nicht in eine Deklaration.
        assert_eq!(rule("(color:)"), Some(false));
        assert_eq!(rule("(color: red; width: 0)"), Some(false));
        assert_eq!(rule("(color: red !foo)"), Some(false));
        assert_eq!(rule("(color: inherit)"), Some(true));
        // Ein Bezeichner allein ist der Name einer benannten Bedingung.
        assert_eq!(rule("not (foo)"), Some(true));
    }

    #[test]
    fn deep_nesting_needs_no_deep_recursion() {
        let depth = 100_000;
        let text = format!("{}color: red{}", "(".repeat(depth), ")".repeat(depth));
        assert_eq!(rule(&text), Some(false));
    }

    #[test]
    fn functions() {
        assert!(supports_condition("selector(div > p::before)"));
        assert!(!supports_condition("selector(div, p)"));
        assert!(!supports_condition("selector(:is(div, :foo))"));
        assert!(!supports_condition("selector(::-webkit-foo)"));
        assert!(!supports_condition("selector(svg|a)"));
        assert!(supports_condition("at-rule( @MEDIA )"));
        assert!(!supports_condition("at-rule(@charset)"));
        assert!(!supports_condition("at-rule(@media; foo)"));
        assert!(!supports_condition("font-format(opentype)"));
        assert!(!supports_condition("env(safe-area-inset-top)"));
        assert_eq!(rule("selector(a ])"), None);
    }

    #[test]
    fn at_rules() {
        // Die Fälle aus `supports-at-rule.html`; dort erwartet, aber hier
        // noch nicht erfüllt sind `@counter-style`, `@swash` (in
        // `@font-feature-values`) und `@starting-style`.
        let gaps = ["@counter-style)", "@swash)", "@starting-style)"];
        let cases = [
            ("at-rule(@supports)", true),
            ("at-rule( @supports)", true),
            ("at-rule(@supports )", true),
            ("at-rule(@media)", true),
            ("at-rule(@counter-style)", true),
            ("at-rule(@doesnotexist)", false),
            ("at-rule(@import)", true),
            ("at-rule(@swash)", true),
            ("at-rule(@starting-style)", true),
            ("at-rule(@charset)", false),
            ("at-rule(@counter-style; system: fixed)", false),
            ("at-rule(@supports) and (color: red)", true),
            ("at-rule(@media) and (display: flex)", true),
            ("at-rule(@doesnotexist) and (color: red)", false),
            ("at-rule(@supports) and (doesnotexist: red)", false),
            ("at-rule(@supports) or (color: red)", true),
            ("at-rule(@doesnotexist) or (color: red)", true),
            ("at-rule(@supports) or (doesnotexist: red)", true),
            ("at-rule(@doesnotexist) or (doesnotexist: red)", false),
            ("not at-rule(@supports)", false),
            ("not at-rule(@doesnotexist)", true),
            ("(color: red) and at-rule(@supports)", true),
            ("(color: red) and at-rule(@doesnotexist)", false),
            ("(color: red) or at-rule(@doesnotexist)", true),
            ("(doesnotexist: red) or at-rule(@doesnotexist)", false),
        ];
        for (text, expected) in cases {
            let gap = gaps.iter().any(|gap| text.ends_with(gap));
            assert_eq!(supports_condition(text), expected != gap, "{text}");
        }
    }

    #[test]
    fn css_supports() {
        assert!(supports_condition("color: red"));
        assert!(supports_condition("(color: red)"));
        // Eingeklammert wird der Text, wie er ist.
        assert!(supports_condition("color: red) and (width: 0"));
        assert!(supports_declaration("COLOR", "red"));
        assert!(!supports_declaration(" color", "red"));
        assert!(!supports_declaration("color", "red !important"));
        assert!(supports_declaration("color", "inherit"));
    }
}
