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

#[test]
fn css_text() {
    let mut block = block(&[("color", "red")]);
    // Die wichtige Deklaration gilt auch vor einer späteren ohne.
    block.set_css_text(
        "margin-top: 1px !important; background: url(a.png) top left; margin-top: 2px",
        ParseContext::default(),
    );
    assert_eq!(
        block.css_text(),
        "margin-top: 1px !important; background: url(\"a.png\") left top;"
    );
    // Nur `background-image` mit zwei Ebenen: `background` passt nicht mehr.
    assert!(block.set_property(
        "background-image",
        "none, none",
        "",
        ParseContext::default()
    ));
    assert_eq!(block.get_property_value("background"), "");
    assert_eq!(block.get_property_value("background-position"), "left top");
}

#[test]
fn edge_offset_next_to_bare_offset() {
    // Ein Abstand ohne Kante gilt von links oder oben; neben einem Abstand
    // mit Kante wird die Kante ausgeschrieben, denn `right 10px 20px` ist
    // ungültig.
    let block = block(&[
        ("background", "none"),
        ("background-position-x", "right 10px"),
        ("background-position-y", "20px"),
    ]);
    assert_eq!(
        block.get_property_value("background-position"),
        "right 10px top 20px"
    );
    assert_eq!(
        block.get_property_value("background"),
        "right 10px top 20px"
    );
    let block = block_with_position("10px, center", "bottom 5%, top 1em");
    assert_eq!(
        block.get_property_value("background-position"),
        "left 10px bottom 5%, center top 1em"
    );
}

fn block_with_position(x: &str, y: &str) -> DeclarationBlock {
    block(&[("background-position-x", x), ("background-position-y", y)])
}

#[test]
fn outline_with_auto() {
    // `auto` passt zu Stil und Farbe und füllt, was die übrigen Angaben frei
    // lassen, den Stil zuerst.
    let longhands = |value: &str| {
        let outline = block(&[("outline", value)]);
        ["outline-width", "outline-style", "outline-color"]
            .map(|longhand| outline.get_property_value(longhand))
    };
    assert_eq!(longhands("auto solid"), ["medium", "solid", "auto"]);
    assert_eq!(longhands("2px auto dashed"), ["2px", "dashed", "auto"]);
    assert_eq!(longhands("auto 2px red"), ["2px", "auto", "red"]);
    assert_eq!(longhands("2px auto"), ["2px", "auto", "auto"]);
    assert_eq!(longhands("auto auto"), ["medium", "auto", "auto"]);
    for invalid in ["solid red auto", "auto auto auto", "auto solid auto red"] {
        let mut outline = DeclarationBlock::new();
        assert!(
            !outline.set_property("outline", invalid, "", ParseContext::default()),
            "{invalid}"
        );
    }
}
