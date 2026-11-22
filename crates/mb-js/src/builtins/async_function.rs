//! %AsyncFunction.prototype% (Abschnitt 27.10.3), der Prototyp der
//! async-Funktionen, und der Konstruktor %AsyncFunction% (Abschnitt
//! 27.10.1) mit CreateDynamicFunction (Abschnitt 20.2.1.1.1) wie
//! `Function`.

use super::{create_dynamic_constructor, define_to_string_tag};
use crate::agent::{Agent, Completion};
use crate::builtin::{Builtin, BuiltinCall};
use crate::eval::create_dynamic_function;
use crate::heap::{Local, Scope};
use crate::object::ordinary_object_create;
use crate::parser::DynamicFunctionKind;
use crate::realm::{Intrinsic, RealmId};

static CONSTRUCTOR: Builtin = Builtin {
    name: "AsyncFunction",
    length: 1,
    constructor: true,
    behaviour: async_function,
};

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let function = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(function))?;
    // AsyncFunction.prototype [ %Symbol.toStringTag% ] (Abschnitt 27.10.3.2).
    define_to_string_tag(agent, scope, prototype, "AsyncFunction")?;
    agent.set_intrinsic(realm, Intrinsic::AsyncFunctionPrototype, prototype);
    create_dynamic_constructor(
        agent,
        scope,
        realm,
        &CONSTRUCTOR,
        prototype,
        Intrinsic::AsyncFunction,
    )
}

/// AsyncFunction ( ...parameterArgs, bodyArg ) (Abschnitt 27.10.1.1).
fn async_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_dynamic_function(agent, scope, arguments, DynamicFunctionKind::Async)
}
