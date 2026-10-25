//! Selektor-Abgleich, Kaskade und berechnete Stile.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.6. Bisher gibt es die Selektoren
//! nach Selectors Level 4 mit Parser, Spezifität und Abgleich gegen den
//! Dokumentspeicher, die Werte und Eigenschaften der ersten Ausbaustufe mit
//! Parsen und Serialisieren, Deklarationsblöcke nach CSSOM, Stylesheets mit
//! Verschachtelung, `@layer` und `@namespace`, das Stylesheet der
//! Benutzeragenten aus dem HTML Standard, die Kaskade bis zum kaskadierten
//! Wert und daraus die berechneten Werte mit den Werten für
//! `getComputedStyle()`, soweit sie nicht vom Layout abhängen.

#![forbid(unsafe_code)]

mod cascade;
mod computed;
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
pub use computed::{
    Advance, Ancestors, ComputedValues, Device, Font, FontMetrics, FontMetricsSource,
    MEDIUM_FONT_SIZE, MissingFontMetric, NeedsLayout, NoFonts, flow_mode,
};
pub use declarations::{DeclarationBlock, Declared, parse_declaration_block};
pub use selectors::{
    InvalidSelector, MatchingContext, Namespaces, PseudoElement, Selector, SelectorList,
    Specificity,
};
pub use stylesheet::StyleSheet;
pub use ua::DocumentInfo;
