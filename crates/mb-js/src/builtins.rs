//! Die eingebauten Objekte (Abschnitte 19 bis 28), soweit es sie gibt. Jede
//! Familie hat ein Modul mit ihrer Tabelle aus [`Builtin`]-Einträgen und
//! einer Funktion, die sie in einem neuen Realm anlegt.
//!
//! Bisher sind es die Teile, die die Harness von Test262 braucht:
//! %Function.prototype% mit `call`, %Object% mit
//! `Object.prototype.toString`, %String% und die Fehlerkonstruktoren mit
//! `Error.prototype.toString`. Die übrigen Eigenschaften dieser Objekte und
//! die anderen eingebauten Objekte kommen in Wellen (Schritt 8 des Plans).
//!
//! Wo nichts anderes steht, sind Eigenschaften eingebauter Objekte
//! beschreibbar und konfigurierbar, aber nicht aufzählbar (Abschnitt 18).

#![forbid(unsafe_code)]

mod error;
mod function;
mod object;
mod string;

use crate::agent::{Agent, Completion};
use crate::builtin::Builtin;
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
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
    string::create(agent, scope, realm)?;
    error::create(agent, scope, realm)
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
