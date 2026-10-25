//! Die *presentational hints*, die der HTML Standard im Abschnitt
//! „Rendering“ in Prosa beschreibt statt in CSS. Bisher die Attribute
//! `width` und `height`, die bei `img`, `embed`, `iframe`, `object` und
//! `video` auf die gleichnamigen Eigenschaften abgebildet werden (*maps to
//! the dimension property*). Es fehlen die übrigen Zuordnungen,
//! `aspect-ratio` (die Eigenschaft fehlt noch) und `input` im Zustand
//! *Image Button*, für den bekannt sein muss, ob es ein Bild darstellt. Die
//! *dimension attribute source* eines `img` ist hier immer das `img`
//! selbst, denn `source` in `picture` wird noch nicht ausgewertet.

use mb_dom::Element;

use crate::declarations::Declared;
use crate::properties::{
    BoxSize, DeclaredValue, Longhand, PropertyDeclaration, Size, SpecifiedValue,
};
use crate::values::{Length, LengthPercentage, clamp};

/// Die Elemente, deren `width` und `height` auf die Eigenschaften abgebildet
/// werden.
const DIMENSION_ELEMENTS: [&str; 5] = ["img", "embed", "iframe", "object", "video"];

/// Das Ergebnis der *rules for parsing dimension values*.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dimension {
    Length(f64),
    /// Die Zahl vor dem `%`.
    Percentage(f64),
}

fn is_ascii_whitespace(unit: u16) -> bool {
    matches!(unit, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

fn digit(unit: u16) -> Option<f64> {
    (0x30..=0x39)
        .contains(&unit)
        .then(|| f64::from(unit.saturating_sub(0x30)))
}

/// *Rules for parsing dimension values*. Alles nach der Zahl außer einem
/// `%` gleich dahinter wird übergangen.
fn parse_dimension(input: &[u16]) -> Option<Dimension> {
    let mut rest = input;
    while let Some((&unit, tail)) = rest.split_first()
        && is_ascii_whitespace(unit)
    {
        rest = tail;
    }
    rest.first().copied().and_then(digit)?;
    let mut value = 0.0;
    while let Some((&unit, tail)) = rest.split_first()
        && let Some(digit) = digit(unit)
    {
        value = value * 10.0 + digit;
        rest = tail;
    }
    if let Some((&0x2E, tail)) = rest.split_first() {
        rest = tail;
        let mut divisor = 1.0;
        while let Some((&unit, tail)) = rest.split_first()
            && let Some(digit) = digit(unit)
        {
            divisor *= 10.0;
            value += digit / divisor;
            rest = tail;
        }
    }
    let value = clamp(value);
    Some(match rest.first() {
        Some(0x25) => Dimension::Percentage(value),
        _ => Dimension::Length(value),
    })
}

fn dimension_hint(longhand: Longhand, dimension: Dimension) -> Declared {
    let value = match dimension {
        Dimension::Length(px) => LengthPercentage::Length(Length::px(px)),
        Dimension::Percentage(percentage) => LengthPercentage::Percentage(percentage),
    };
    Declared {
        declaration: PropertyDeclaration {
            longhand,
            value: DeclaredValue::Value(SpecifiedValue::Size(Size::Box(
                BoxSize::LengthPercentage(value),
            ))),
        },
        important: false,
    }
}

/// Die *presentational hints* eines Elements aus seinen Attributen, soweit
/// sie nicht als CSS im Stylesheet der Benutzeragenten stehen.
pub(crate) fn presentational_hints(element: &Element) -> Vec<Declared> {
    let mut hints = Vec::new();
    if DIMENSION_ELEMENTS.iter().any(|name| element.is_html(name)) {
        for (attribute, longhand) in [("width", Longhand::Width), ("height", Longhand::Height)] {
            let units: Option<Vec<u16>> = element
                .attribute(attribute)
                .map(|value| value.code_units().collect());
            if let Some(dimension) = units.as_deref().and_then(parse_dimension) {
                hints.push(dimension_hint(longhand, dimension));
            }
        }
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Option<Dimension> {
        let units: Vec<u16> = text.encode_utf16().collect();
        parse_dimension(&units)
    }

    #[test]
    fn dimension_values() {
        assert_eq!(parse("120"), Some(Dimension::Length(120.0)));
        assert_eq!(parse(" \t50%"), Some(Dimension::Percentage(50.0)));
        assert_eq!(parse("2.5px"), Some(Dimension::Length(2.5)));
        assert_eq!(parse("2.5%"), Some(Dimension::Percentage(2.5)));
        // Ein Punkt ohne Ziffer danach endet die Zahl; das `%` gilt noch.
        assert_eq!(parse("3.%"), Some(Dimension::Percentage(3.0)));
        assert_eq!(parse("3. %"), Some(Dimension::Length(3.0)));
        assert_eq!(parse("0"), Some(Dimension::Length(0.0)));
        assert_eq!(parse(""), None);
        assert_eq!(parse("-5"), None);
        assert_eq!(parse(".5"), None);
        assert_eq!(parse("+5"), None);
        // Ein Zeichen nach Unicode als Leerraum zählt hier nicht.
        assert_eq!(parse("\u{A0}5"), None);
        assert_eq!(
            parse(&"9".repeat(400)),
            Some(Dimension::Length(f64::from(f32::MAX)))
        );
    }
}
