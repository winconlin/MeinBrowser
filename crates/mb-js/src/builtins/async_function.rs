//! %AsyncFunction.prototype% (Abschnitt 27.10.3), der Prototyp der
//! async-Funktionen.
//!
//! Den Konstruktor %AsyncFunction% (Abschnitt 27.10.1) gibt es noch nicht,
//! denn er braucht CreateDynamicFunction (Abschnitt 20.2.1.1.1) wie
//! `Function`; darum fehlt auch `constructor` an %AsyncFunction.prototype%
//! (Abschnitt 27.10.3.1).

use super::define_to_string_tag;
use crate::agent::{Agent, Completion};
use crate::heap::Scope;
use crate::object::ordinary_object_create;
use crate::realm::{Intrinsic, RealmId};

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let function = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(function))?;
    // AsyncFunction.prototype [ %Symbol.toStringTag% ] (Abschnitt 27.10.3.2).
    define_to_string_tag(agent, scope, prototype, "AsyncFunction")?;
    agent.set_intrinsic(realm, Intrinsic::AsyncFunctionPrototype, prototype);
    Ok(())
}
