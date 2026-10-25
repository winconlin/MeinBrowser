//! Selektor-Abgleich, Kaskade und berechnete Stile.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.6. Bisher gibt es die Selektoren
//! nach Selectors Level 4 mit Parser, Spezifität und Abgleich gegen den
//! Dokumentspeicher, die Werte und Eigenschaften der ersten Ausbaustufe mit
//! Parsen und Serialisieren und Deklarationsblöcke nach CSSOM. Es fehlen
//! noch die Kaskade und die berechneten Werte.

#![forbid(unsafe_code)]

mod declarations;
pub mod properties;
mod selectors;
pub mod values;
#[cfg(test)]
mod wpt_js;

pub use declarations::DeclarationBlock;

pub use selectors::{
    InvalidSelector, MatchingContext, Namespaces, PseudoElement, Selector, SelectorList,
    Specificity,
};
