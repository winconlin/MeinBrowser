//! Eigenschaftsbeschreibungen (Abschnitt 6.2.6).

#![forbid(unsafe_code)]

use crate::heap::Local;

/// Ein Property Descriptor: jedes Feld ist vorhanden oder fehlt. Werte,
/// Getter und Setter stehen in Handles, denn die Operationen, die eine
/// Beschreibung lesen, fordern Speicher an.
#[derive(Clone, Copy, Debug, Default)]
pub struct PropertyDescriptor<'s> {
    pub value: Option<Local<'s>>,
    pub writable: Option<bool>,
    /// [[Get]]: eine Funktion oder *undefined*.
    pub get: Option<Local<'s>>,
    /// [[Set]]: eine Funktion oder *undefined*.
    pub set: Option<Local<'s>>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}

impl<'s> PropertyDescriptor<'s> {
    /// Eine vollständige Beschreibung einer Dateneigenschaft.
    pub fn data(value: Local<'s>, writable: bool, enumerable: bool, configurable: bool) -> Self {
        PropertyDescriptor {
            value: Some(value),
            writable: Some(writable),
            enumerable: Some(enumerable),
            configurable: Some(configurable),
            ..PropertyDescriptor::default()
        }
    }

    /// Eine vollständige Beschreibung einer Zugriffseigenschaft.
    pub fn accessor(get: Local<'s>, set: Local<'s>, enumerable: bool, configurable: bool) -> Self {
        PropertyDescriptor {
            get: Some(get),
            set: Some(set),
            enumerable: Some(enumerable),
            configurable: Some(configurable),
            ..PropertyDescriptor::default()
        }
    }

    /// IsAccessorDescriptor (Abschnitt 6.2.6.1).
    pub fn is_accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    /// IsDataDescriptor (Abschnitt 6.2.6.2).
    pub fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    /// IsGenericDescriptor (Abschnitt 6.2.6.3).
    pub fn is_generic(&self) -> bool {
        !self.is_accessor() && !self.is_data()
    }

    /// Ob die Beschreibung kein Feld hat.
    pub fn is_empty(&self) -> bool {
        self.is_generic() && self.enumerable.is_none() && self.configurable.is_none()
    }
}
