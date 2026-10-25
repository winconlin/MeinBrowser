use super::{Longhand, PropertyId, Shorthand};
use crate::declarations::DeclarationBlock;
use crate::values::ParseContext;

fn block(declarations: &[(&str, &str)]) -> DeclarationBlock {
    let mut block = DeclarationBlock::new();
    for (property, value) in declarations {
        assert!(
            block.set_property(property, value, "", ParseContext::default()),
            "{property}: {value}"
        );
    }
    block
}

#[test]
fn initial_values_parse() {
    for &longhand in Longhand::ALL {
        // „depends on user agent“
        let expected = longhand != Longhand::FontFamily;
        assert_eq!(
            longhand.initial_value().is_some(),
            expected,
            "{}",
            longhand.name()
        );
    }
}

#[test]
fn names() {
    assert_eq!(
        PropertyId::from_name("MARGIN-top"),
        Some(PropertyId::Longhand(Longhand::MarginTop))
    );
    assert_eq!(
        PropertyId::from_name("font-stretch"),
        Some(PropertyId::Longhand(Longhand::FontWidth))
    );
    assert_eq!(
        PropertyId::from_name("border"),
        Some(PropertyId::Shorthand(Shorthand::Border))
    );
    assert_eq!(PropertyId::from_name("float"), None);
    // Bevorzugte Reihenfolge: nach Namen, dann mehr Langformen zuerst;
    // `all` umfasst fast alle Langformen.
    assert_eq!(
        Shorthand::containing(Longhand::BorderTopWidth),
        [
            Shorthand::All,
            Shorthand::Border,
            Shorthand::BorderWidth,
            Shorthand::BorderTop
        ]
    );
}

#[test]
fn shorthands_and_priorities() {
    let mut margin = block(&[("margin", "1px 2px")]);
    assert_eq!(margin.len(), 4);
    assert_eq!(margin.get_property_value("margin"), "1px 2px");
    assert_eq!(margin.css_text(), "margin: 1px 2px;");
    assert!(margin.set_property("margin-top", "3px", "IMPORTANT", ParseContext::default()));
    assert_eq!(margin.get_property_priority("margin-top"), "important");
    assert_eq!(margin.get_property_value("margin"), "");
    assert_eq!(
        margin.css_text(),
        "margin-top: 3px !important; margin-right: 2px; margin-bottom: 1px; margin-left: 2px;"
    );
    assert!(!margin.set_property("margin-top", "4px", "wichtig", ParseContext::default()));
    assert!(!margin.set_property("margin-top", "3px", "important", ParseContext::default()));

    let text_align = block(&[("text-align", "match-parent")]);
    assert_eq!(text_align.get_property_value("text-align"), "match-parent");
    let text_align = block(&[
        ("text-align-all", "match-parent"),
        ("text-align-last", "auto"),
    ]);
    assert_eq!(text_align.get_property_value("text-align"), "");
    assert_eq!(
        text_align.css_text(),
        "text-align-all: match-parent; text-align-last: auto;"
    );

    let mut border = block(&[("border", "1px solid red")]);
    assert_eq!(border.css_text(), "border: 1px solid red;");
    assert_eq!(border.remove_property("border-top"), "1px solid red");
    assert_eq!(border.len(), 9);
}

#[test]
fn logical_and_physical_declarations() {
    let mut margins = block(&[
        ("margin-top", "1px"),
        ("margin-block-start", "2px"),
        ("margin-right", "1px"),
        ("margin-bottom", "1px"),
        ("margin-left", "1px"),
    ]);
    // margin-block-start steht zwischen den Langformen von margin.
    assert_eq!(
        margins.css_text(),
        "margin-top: 1px; margin-block-start: 2px; margin-right: 1px; margin-bottom: 1px; \
         margin-left: 1px;"
    );
    // Nach einer Deklaration derselben Gruppe mit anderer mapping logic
    // rückt eine geänderte Deklaration ans Ende.
    assert!(margins.set_property("margin-top", "3px", "", ParseContext::default()));
    assert_eq!(margins.item(0), Some("margin-block-start"));
    assert_eq!(margins.item(4), Some("margin-top"));
}
