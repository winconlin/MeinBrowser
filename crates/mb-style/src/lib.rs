//! Selektor-Abgleich, Kaskade und berechnete Stile.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.6. Bisher gibt es die Selektoren
//! nach Selectors Level 4 mit Parser, Spezifität und Abgleich gegen den
//! Dokumentspeicher, die Werte und Eigenschaften der ersten Ausbaustufe mit
//! Parsen und Serialisieren, Deklarationsblöcke nach CSSOM, Stylesheets mit
//! Verschachtelung, `@layer` und `@namespace`, das Stylesheet der
//! Benutzeragenten aus dem HTML Standard und die Kaskade bis zum
//! kaskadierten Wert. Es fehlen noch die berechneten Werte.

#![forbid(unsafe_code)]

mod cascade;
mod declarations;
pub mod properties;
mod selectors;
mod stylesheet;
mod ua;
pub mod values;
#[cfg(test)]
mod wpt_js;

pub use cascade::{
    Cascaded, CascadedValue, CascadedValues, MatchedDeclarations, Origin, Stylist, StylistBuilder,
};
pub use declarations::{DeclarationBlock, Declared, parse_declaration_block};
pub use selectors::{
    InvalidSelector, MatchingContext, Namespaces, PseudoElement, Selector, SelectorList,
    Specificity,
};
pub use stylesheet::StyleSheet;
pub use ua::DocumentInfo;
