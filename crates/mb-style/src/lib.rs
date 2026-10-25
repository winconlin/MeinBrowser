//! Selektor-Abgleich, Kaskade und berechnete Stile.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.6. Bisher gibt es die Selektoren
//! nach Selectors Level 4 mit Parser, Spezifität und Abgleich gegen den
//! Dokumentspeicher. Es fehlen noch die Kaskade und die berechneten Werte.

#![forbid(unsafe_code)]

mod selectors;

pub use selectors::{
    InvalidSelector, MatchingContext, Namespaces, PseudoElement, Selector, SelectorList,
    Specificity,
};
