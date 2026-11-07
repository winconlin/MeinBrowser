//! Die eingebauten Objekte (Abschnitte 19 bis 28), soweit es sie gibt. Jede
//! Familie hat ein Modul mit ihrer Tabelle aus [`Builtin`]-Einträgen und
//! einer Funktion, die sie in einem neuen Realm anlegt.
//!
//! Bisher sind es die Teile, die die Harness von Test262 braucht:
//! %Function.prototype% mit `call`, %Object% mit
//! `Object.prototype.toString`, %String% und die Fehlerkonstruktoren mit
//! `Error.prototype.toString`. Dazu kommen, was `for`-`of` und die
//! Ausbreitung brauchen: die Iteratoren für Arrays und Zeichenketten mit
//! `Array.prototype.values`, `keys`, `entries` und den Methoden für
//! @@iterator, und %Symbol% mit den bekannten Symbolen, damit Scripts
//! eigene Iteratoren bauen können. Generatoren bringen ihre Prototypen mit
//! `next`, `return` und `throw` mit. %Promise% ist vollständig, mit
//! %AggregateError% für `Promise.any`. Die übrigen Eigenschaften dieser
//! Objekte und die anderen eingebauten Objekte kommen in Wellen (Schritt 8
//! des Plans).
//!
//! Wo nichts anderes steht, sind Eigenschaften eingebauter Objekte
//! beschreibbar und konfigurierbar, aber nicht aufzählbar (Abschnitt 18).

#![forbid(unsafe_code)]

mod array;
mod error;
mod function;
mod generator;
mod iterator;
mod object;
mod promise;
mod string;
mod symbol;

use crate::agent::{Agent, Completion};
use crate::builtin::{Builtin, create_builtin_function};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::operations::define_property_or_throw;
use crate::realm::RealmId;

pub(crate) use function::PROTOTYPE as FUNCTION_PROTOTYPE;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Legt die eingebauten Objekte an, die auf %Object.prototype%,
/// %Function.prototype% und den Prototypen der primitiven Werte aufbauen.
pub(crate) fn create_intrinsics(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
) -> Completion<()> {
    function::create(agent, scope, realm)?;
    object::create(agent, scope, realm)?;
    iterator::create(agent, scope, realm)?;
    generator::create(agent, scope, realm)?;
    array::create(agent, scope, realm)?;
    string::create(agent, scope, realm)?;
    symbol::create(agent, scope, realm)?;
    error::create(agent, scope, realm)?;
    promise::create(agent, scope, realm)
}

/// Verbindet einen Konstruktor mit seinem Prototyp: `prototype` am
/// Konstruktor ist weder beschreibbar noch aufzählbar noch konfigurierbar,
/// `constructor` am Prototyp hat die üblichen Attribute. So steht es bei
/// jedem Konstruktor, etwa in den Abschnitten 20.1.2.21 und 20.1.3.1.
fn link_constructor(
    agent: &mut Agent,
    scope: &Scope,
    constructor: Local<'_>,
    prototype: Local<'_>,
) -> Completion<()> {
    let key = agent.key_from_units(scope, &units("prototype"))?;
    let descriptor = PropertyDescriptor::data(prototype, false, false, false);
    define_property_or_throw(agent, scope, constructor, key, &descriptor)?;
    let key = agent.key_from_units(scope, &units("constructor"))?;
    let descriptor = PropertyDescriptor::data(constructor, true, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)
}

/// Eine eingebaute Funktion als Eigenschaft von `object` mit einem bekannten
/// Symbol als Schlüssel, mit den üblichen Attributen. Ihr Name aus dem
/// Eintrag lautet etwa `"[Symbol.iterator]"`.
fn define_symbol_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    object: Local<'_>,
    symbol: WellKnownSymbol,
    builtin: &'static Builtin,
) -> Completion<Local<'s>> {
    let function = create_builtin_function(agent, scope, realm, builtin)?;
    define_symbol_property(agent, scope, object, symbol, function)?;
    Ok(function)
}

/// `value` als Eigenschaft von `object` mit einem bekannten Symbol als
/// Schlüssel, beschreibbar und konfigurierbar, nicht aufzählbar.
fn define_symbol_property(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    symbol: WellKnownSymbol,
    value: Local<'_>,
) -> Completion<()> {
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, symbol));
    let descriptor = PropertyDescriptor::data(value, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// @@toStringTag mit der Zeichenkette `tag`: nur konfigurierbar, wie bei
/// allen Prototypen, die eine haben (etwa Abschnitt 23.1.5.2.2).
fn define_to_string_tag(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    tag: &str,
) -> Completion<()> {
    let value = agent.heap_mut().alloc_string(scope, &units(tag))?;
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::ToStringTag));
    let descriptor = PropertyDescriptor::data(value, false, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// Die Funktionen einer Tabelle als Eigenschaften von `object`.
fn define_functions(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    object: Local<'_>,
    table: &'static [Builtin],
) -> Completion<()> {
    for builtin in table {
        crate::builtin::define_builtin_function(agent, scope, realm, object, builtin)?;
    }
    Ok(())
}
