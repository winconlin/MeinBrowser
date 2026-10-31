//! Eigenschaftsschlüssel (Abschnitt 6.1.7) und Symbole (Abschnitt 6.1.5).
//!
//! Ein Schlüssel ist eine Zeichenkette oder ein Symbol. Array-Indizes, also
//! die Zeichenketten "0" bis "4294967294" in der Form, die ToString für
//! diese Zahlen liefert, stehen als Zahl im Schlüssel; Objekte halten sie
//! als Elemente getrennt von den übrigen Eigenschaften. Jede andere
//! Zeichenkette ist im Schlüssel ein Atom.

#![forbid(unsafe_code)]

use crate::agent::Agent;
use crate::heap::{CellKind, Heap, Local, OutOfMemory, Scope};
use crate::value::{Kind, Ref, Value};

/// Der größte Array-Index, 2³² − 2.
pub const MAX_ARRAY_INDEX: u32 = u32::MAX - 1;

/// Die bekannten Symbole (Abschnitt 6.1.5.1). Alle Realms eines Agents
/// teilen sie.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WellKnownSymbol {
    AsyncDispose,
    AsyncIterator,
    Dispose,
    HasInstance,
    IsConcatSpreadable,
    Iterator,
    Match,
    MatchAll,
    Replace,
    Search,
    Species,
    Split,
    ToPrimitive,
    ToStringTag,
    Unscopables,
}

impl WellKnownSymbol {
    pub const ALL: [WellKnownSymbol; 15] = [
        WellKnownSymbol::AsyncDispose,
        WellKnownSymbol::AsyncIterator,
        WellKnownSymbol::Dispose,
        WellKnownSymbol::HasInstance,
        WellKnownSymbol::IsConcatSpreadable,
        WellKnownSymbol::Iterator,
        WellKnownSymbol::Match,
        WellKnownSymbol::MatchAll,
        WellKnownSymbol::Replace,
        WellKnownSymbol::Search,
        WellKnownSymbol::Species,
        WellKnownSymbol::Split,
        WellKnownSymbol::ToPrimitive,
        WellKnownSymbol::ToStringTag,
        WellKnownSymbol::Unscopables,
    ];

    /// [[Description]] des Symbols.
    pub fn description(self) -> &'static str {
        match self {
            WellKnownSymbol::AsyncDispose => "Symbol.asyncDispose",
            WellKnownSymbol::AsyncIterator => "Symbol.asyncIterator",
            WellKnownSymbol::Dispose => "Symbol.dispose",
            WellKnownSymbol::HasInstance => "Symbol.hasInstance",
            WellKnownSymbol::IsConcatSpreadable => "Symbol.isConcatSpreadable",
            WellKnownSymbol::Iterator => "Symbol.iterator",
            WellKnownSymbol::Match => "Symbol.match",
            WellKnownSymbol::MatchAll => "Symbol.matchAll",
            WellKnownSymbol::Replace => "Symbol.replace",
            WellKnownSymbol::Search => "Symbol.search",
            WellKnownSymbol::Species => "Symbol.species",
            WellKnownSymbol::Split => "Symbol.split",
            WellKnownSymbol::ToPrimitive => "Symbol.toPrimitive",
            WellKnownSymbol::ToStringTag => "Symbol.toStringTag",
            WellKnownSymbol::Unscopables => "Symbol.unscopables",
        }
    }
}

/// Ein Eigenschaftsschlüssel.
#[derive(Clone, Copy, Debug)]
pub enum PropertyKey<'s> {
    /// Ein Array-Index.
    Index(u32),
    /// Eine Zeichenkette, die kein Array-Index ist, als Atom.
    String(Local<'s>),
    Symbol(Local<'s>),
}

/// Der Array-Index, den die Codeeinheiten darstellen: "0" oder eine
/// Ziffernfolge ohne führende Null mit einem Wert bis 2³² − 2. Genau diese
/// Zeichenketten ergeben mit CanonicalNumericIndexString (Abschnitt 7.1.23)
/// eine ganze Zahl in diesem Bereich, denn ToString schreibt ganze Zahlen
/// unter 10²¹ als Dezimalziffern ohne Exponent.
pub fn array_index(units: &[u16]) -> Option<u32> {
    let (&first, _) = units.split_first()?;
    if first == u16::from(b'0') {
        return (units.len() == 1).then_some(0);
    }
    let mut value: u32 = 0;
    for &unit in units {
        let digit = unit
            .checked_sub(u16::from(b'0'))
            .filter(|&digit| digit < 10)?;
        value = value.checked_mul(10)?.checked_add(u32::from(digit))?;
    }
    (value <= MAX_ARRAY_INDEX).then_some(value)
}

impl Agent {
    /// Der Schlüssel aus den Codeeinheiten einer Zeichenkette.
    pub fn key_from_units<'s>(
        &mut self,
        scope: &'s Scope,
        units: &[u16],
    ) -> Result<PropertyKey<'s>, OutOfMemory> {
        if let Some(index) = array_index(units) {
            return Ok(PropertyKey::Index(index));
        }
        Ok(PropertyKey::String(self.atom(scope, units)?))
    }

    /// Der Schlüssel zu einer Zeichenkette oder einem Symbol, `None` für
    /// andere Werte. Das ist ToPropertyKey (Abschnitt 7.1.21) ohne den
    /// Schritt ToPrimitive.
    pub fn key_from_value<'s>(
        &mut self,
        scope: &'s Scope,
        value: Local<'s>,
    ) -> Option<PropertyKey<'s>> {
        match self.heap().get(value).kind() {
            Kind::Symbol(_) => Some(PropertyKey::Symbol(value)),
            Kind::String(string) => {
                let units = self.heap().string_units(string);
                Some(match array_index(&units) {
                    Some(index) => PropertyKey::Index(index),
                    None => PropertyKey::String(self.atomize(scope, value)),
                })
            }
            _ => None,
        }
    }

    /// Ein neues Symbol mit der Beschreibung `description`, einer
    /// Zeichenkette oder *undefined* für `None`.
    pub fn new_symbol<'s>(
        &mut self,
        scope: &'s Scope,
        description: Option<Local<'_>>,
    ) -> Result<Local<'s>, OutOfMemory> {
        let heap = self.heap_mut();
        let symbol = heap.alloc_cell(scope, CellKind::Symbol, 1)?;
        if let (Some(description), Kind::Symbol(cell)) = (description, heap.get(symbol).kind()) {
            heap.set_field(cell, 0, heap.get(description));
        }
        Ok(symbol)
    }
}

/// [[Description]] eines Symbols: eine Zeichenkette oder *undefined*.
pub fn symbol_description<'h>(heap: &'h Heap, symbol: Ref<'h>) -> Value<'h> {
    heap.field(symbol, 0).unwrap_or(Value::UNDEFINED)
}

/// Ob zwei Schlüssel gleich sind: Atome und Symbole über ihre Identität.
pub fn same_key(heap: &Heap, a: PropertyKey<'_>, b: PropertyKey<'_>) -> bool {
    match (a, b) {
        (PropertyKey::Index(a), PropertyKey::Index(b)) => a == b,
        (PropertyKey::String(a), PropertyKey::String(b))
        | (PropertyKey::Symbol(a), PropertyKey::Symbol(b)) => {
            heap.get(a).bits() == heap.get(b).bits()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
