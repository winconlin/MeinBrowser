//! Deklarationsblöcke nach CSSOM („CSS Declaration Blocks“): die Methoden
//! von `CSSStyleDeclaration` ohne die Anbindung an JavaScript und ohne
//! *owner node* und *updating flag*.
//!
//! Namen und Werte kommen als Text. Ein DOMString mit einzelnen Surrogaten
//! wird vorher mit U+FFFD umgewandelt; dasselbe täte das Filtern der
//! Codepunkte beim Parsen (CSS Syntax 3). Custom Properties fehlen noch.

use mb_css::{Source, parse_component_value_list};

use crate::properties::{
    DeclaredValue, Longhand, PropertyDeclaration, PropertyId, Shorthand, serialize_shorthand,
};
use crate::values::{ParseContext, ToCss};

/// Eine Deklaration im Block mit ihrem *important flag*.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    declaration: PropertyDeclaration,
    important: bool,
}

/// Ein *CSS declaration block*: geordnete Deklarationen, je Langform
/// höchstens eine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeclarationBlock {
    entries: Vec<Entry>,
}

impl DeclarationBlock {
    pub fn new() -> DeclarationBlock {
        DeclarationBlock::default()
    }

    /// `length`
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `item()`: der Name der Eigenschaft an einer Stelle.
    pub fn item(&self, index: usize) -> Option<&'static str> {
        self.entries
            .get(index)
            .map(|entry| entry.declaration.longhand.name())
    }

    /// Die Deklarationen in ihrer Reihenfolge, mit dem *important flag*.
    pub fn declarations(&self) -> impl Iterator<Item = (&PropertyDeclaration, bool)> {
        self.entries
            .iter()
            .map(|entry| (&entry.declaration, entry.important))
    }

    fn entry(&self, longhand: Longhand) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.declaration.longhand == longhand)
    }

    /// `getPropertyValue()`
    pub fn get_property_value(&self, property: &str) -> String {
        PropertyId::from_name(property)
            .map(|property| self.value(property))
            .unwrap_or_default()
    }

    fn value(&self, property: PropertyId) -> String {
        match property {
            PropertyId::Longhand(longhand) => self
                .entry(longhand)
                .map(|entry| entry.declaration.value.to_css_string())
                .unwrap_or_default(),
            PropertyId::Shorthand(shorthand) => {
                let Some(list) = shorthand
                    .longhands()
                    .iter()
                    .map(|&longhand| self.entry(longhand))
                    .collect::<Option<Vec<_>>>()
                else {
                    return String::new();
                };
                let important = list.first().is_some_and(|entry| entry.important);
                if list.iter().any(|entry| entry.important != important) {
                    return String::new();
                }
                let values: Vec<&DeclaredValue> =
                    list.iter().map(|entry| &entry.declaration.value).collect();
                serialize_shorthand(shorthand, &values).unwrap_or_default()
            }
        }
    }

    /// `getPropertyPriority()`: `"important"` oder leer.
    pub fn get_property_priority(&self, property: &str) -> &'static str {
        let important = match PropertyId::from_name(property) {
            Some(PropertyId::Longhand(longhand)) => {
                self.entry(longhand).is_some_and(|entry| entry.important)
            }
            Some(PropertyId::Shorthand(shorthand)) => shorthand
                .longhands()
                .iter()
                .all(|&longhand| self.entry(longhand).is_some_and(|entry| entry.important)),
            None => false,
        };
        if important { "important" } else { "" }
    }

    /// `setProperty()`. Liefert, ob sich der Block geändert hat (dann wäre
    /// das Attribut `style` zu aktualisieren).
    pub fn set_property(
        &mut self,
        property: &str,
        value: &str,
        priority: &str,
        context: ParseContext<'_>,
    ) -> bool {
        let Some(property) = PropertyId::from_name(property) else {
            return false;
        };
        if value.is_empty() {
            return self.remove(property);
        }
        let important = if priority.is_empty() {
            false
        } else if priority.eq_ignore_ascii_case("important") {
            true
        } else {
            return false;
        };
        let values = parse_component_value_list(&Source::new(value));
        let Some(declarations) = property.parse(values.values(), context) else {
            return false;
        };
        let mut updated = false;
        for declaration in declarations {
            updated |= self.set_declaration(declaration, important);
        }
        updated
    }

    /// *Set a CSS declaration* nach dem vorgeschlagenen Algorithmus: Die
    /// Deklaration bleibt an ihrer Stelle, außer danach folgt eine aus
    /// derselben *logical property group* mit anderer *mapping logic*; dann
    /// kommt sie ans Ende.
    pub fn set_declaration(&mut self, declaration: PropertyDeclaration, important: bool) -> bool {
        let longhand = declaration.longhand;
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.declaration.longhand == longhand)
        {
            let needs_append = self
                .entries
                .get(index.saturating_add(1)..)
                .unwrap_or_default()
                .iter()
                .any(|entry| longhand.conflicts_with(entry.declaration.longhand));
            if !needs_append {
                let Some(entry) = self.entries.get_mut(index) else {
                    return false;
                };
                if entry.declaration == declaration && entry.important == important {
                    return false;
                }
                entry.declaration = declaration;
                entry.important = important;
                return true;
            }
            self.entries.remove(index);
        }
        self.entries.push(Entry {
            declaration,
            important,
        });
        true
    }

    /// `removeProperty()`: entfernt die Eigenschaft und liefert ihren
    /// bisherigen Wert.
    pub fn remove_property(&mut self, property: &str) -> String {
        let Some(property) = PropertyId::from_name(property) else {
            return String::new();
        };
        let value = self.value(property);
        self.remove(property);
        value
    }

    fn remove(&mut self, property: PropertyId) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| !property.contains(entry.declaration.longhand));
        self.entries.len() != before
    }

    /// `cssText` beim Lesen: *serialize a CSS declaration block*. Langformen
    /// werden, wo es geht, zu Kurzschreibweisen zusammengefasst.
    pub fn css_text(&self) -> String {
        let mut list = Vec::new();
        let mut serialized: Vec<Longhand> = Vec::new();
        'declarations: for entry in &self.entries {
            let property = entry.declaration.longhand;
            if serialized.contains(&property) {
                continue;
            }
            let shorthands = Shorthand::containing(property);
            for &shorthand in &shorthands {
                let longhands: Vec<&Entry> = self
                    .entries
                    .iter()
                    .filter(|entry| {
                        let longhand = entry.declaration.longhand;
                        !serialized.contains(&longhand)
                            && shorthands
                                .iter()
                                .any(|shorthand| shorthand.longhands().contains(&longhand))
                    })
                    .collect();
                let mut current = Vec::new();
                for &longhand in shorthand.longhands() {
                    match longhands
                        .iter()
                        .find(|entry| entry.declaration.longhand == longhand)
                    {
                        Some(entry) => current.push(*entry),
                        None => continue,
                    }
                }
                if current.len() != shorthand.longhands().len() {
                    continue;
                }
                let important = current.first().is_some_and(|entry| entry.important);
                if current.iter().any(|entry| entry.important != important) {
                    continue;
                }
                if self.interleaved(&current) {
                    continue;
                }
                let values: Vec<&DeclaredValue> = current
                    .iter()
                    .map(|entry| &entry.declaration.value)
                    .collect();
                let Some(value) = serialize_shorthand(shorthand, &values) else {
                    continue;
                };
                if value.is_empty() {
                    continue;
                }
                list.push(serialize_declaration(shorthand.name(), &value, important));
                serialized.extend(current.iter().map(|entry| entry.declaration.longhand));
                continue 'declarations;
            }
            let value = entry.declaration.value.to_css_string();
            list.push(serialize_declaration(
                property.name(),
                &value,
                entry.important,
            ));
            serialized.push(property);
        }
        list.join(" ")
    }

    /// Ob zwischen der ersten und der letzten der Deklarationen eine andere
    /// steht, die zu einer von ihnen in derselben *logical property group*
    /// gehört, aber eine andere *mapping logic* hat.
    fn interleaved(&self, current: &[&Entry]) -> bool {
        let positions: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| current.contains(entry))
            .map(|(index, _)| index)
            .collect();
        let (Some(&first), Some(&last)) = (positions.first(), positions.last()) else {
            return false;
        };
        self.entries
            .get(first..=last)
            .unwrap_or_default()
            .iter()
            .filter(|entry| !current.contains(entry))
            .any(|entry| {
                current.iter().any(|own| {
                    own.declaration
                        .longhand
                        .conflicts_with(entry.declaration.longhand)
                })
            })
    }
}

/// *Serialize a CSS declaration*
fn serialize_declaration(property: &str, value: &str, important: bool) -> String {
    let mut out = format!("{property}: {value}");
    if important {
        out.push_str(" !important");
    }
    out.push(';');
    out
}
