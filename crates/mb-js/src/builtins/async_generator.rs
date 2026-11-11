//! %AsyncGeneratorFunction.prototype% (Abschnitt 27.7.3), der Prototyp der
//! async-Generatorfunktionen, und %AsyncGeneratorPrototype% (Abschnitt
//! 27.9.1), der Prototyp ihrer `prototype`-Objekte, mit `next`, `return`
//! und `throw`.
//!
//! Den Konstruktor %AsyncGeneratorFunction% (Abschnitt 27.7.1) gibt es noch
//! nicht, denn er braucht CreateDynamicFunction (Abschnitt 20.2.1.1.1) wie
//! `Function`; darum fehlt auch `constructor` an
//! %AsyncGeneratorFunction.prototype%.

use super::promise::if_abrupt_reject_promise;
use super::{define_functions, define_to_string_tag, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::async_generator::{async_generator_request, is_async_generator};
use crate::builtin::{Builtin, BuiltinCall};
use crate::descriptor::PropertyDescriptor;
use crate::generator::Resumption;
use crate::heap::{Local, Scope};
use crate::object::ordinary_object_create;
use crate::operations::define_property_or_throw;
use crate::promise::new_promise_capability;
use crate::realm::{Intrinsic, RealmId};

static ASYNC_GENERATOR_FUNCTIONS: [Builtin; 3] = [
    Builtin {
        name: "next",
        length: 1,
        constructor: false,
        behaviour: async_generator_next,
    },
    Builtin {
        name: "return",
        length: 1,
        constructor: false,
        behaviour: async_generator_return,
    },
    Builtin {
        name: "throw",
        length: 1,
        constructor: false,
        behaviour: async_generator_throw,
    },
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let function = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
    let async_generator_function = ordinary_object_create(agent, scope, Some(function))?;
    let iterator = agent.intrinsic(scope, realm, Intrinsic::AsyncIteratorPrototype);
    let async_generator = ordinary_object_create(agent, scope, Some(iterator))?;
    // AsyncGeneratorFunction.prototype.prototype (Abschnitt 27.7.3.2) und
    // %AsyncGeneratorPrototype%.constructor (Abschnitt 27.9.1.1): nur
    // konfigurierbar.
    let key = agent.key_from_units(scope, &units("prototype"))?;
    let descriptor = PropertyDescriptor::data(async_generator, false, false, true);
    define_property_or_throw(agent, scope, async_generator_function, key, &descriptor)?;
    define_to_string_tag(
        agent,
        scope,
        async_generator_function,
        "AsyncGeneratorFunction",
    )?;
    let key = agent.key_from_units(scope, &units("constructor"))?;
    let descriptor = PropertyDescriptor::data(async_generator_function, false, false, true);
    define_property_or_throw(agent, scope, async_generator, key, &descriptor)?;
    define_functions(
        agent,
        scope,
        realm,
        async_generator,
        &ASYNC_GENERATOR_FUNCTIONS,
    )?;
    define_to_string_tag(agent, scope, async_generator, "AsyncGenerator")?;
    agent.set_intrinsic(
        realm,
        Intrinsic::AsyncGeneratorFunctionPrototype,
        async_generator_function,
    );
    agent.set_intrinsic(realm, Intrinsic::AsyncGeneratorPrototype, async_generator);
    Ok(())
}

/// %AsyncGeneratorPrototype%.next ( value ) (Abschnitt 27.9.1.2).
fn async_generator_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    request(agent, scope, arguments, Resumption::Next)
}

/// %AsyncGeneratorPrototype%.return ( value ) (Abschnitt 27.9.1.3).
fn async_generator_return<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    request(agent, scope, arguments, Resumption::Return)
}

/// %AsyncGeneratorPrototype%.throw ( exception ) (Abschnitt 27.9.1.4).
fn async_generator_throw<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    request(agent, scope, arguments, Resumption::Throw)
}

/// Die Schritte 1 bis 4 der drei Methoden: NewPromiseCapability(%Promise%)
/// im aktuellen Realm, dann AsyncGeneratorValidate mit ~empty~; ist `this`
/// kein async-Generator, wird das Promise mit einem TypeError abgelehnt.
/// Den Rest erledigt [`async_generator_request`] mit dem ersten Argument.
fn request<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    resumption: Resumption,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Promise);
    let capability = new_promise_capability(agent, scope, realm, constructor)?;
    let generator = arguments.this;
    if !is_async_generator(agent.heap(), agent.heap().get(generator)) {
        let abrupt = agent.throw_error(ErrorKind::TypeError);
        return if_abrupt_reject_promise(agent, scope, realm, abrupt, &capability);
    }
    let value = arguments.argument(agent, scope, 0);
    async_generator_request(
        agent,
        scope,
        realm,
        generator,
        resumption,
        value,
        &capability,
    )?;
    Ok(capability.promise)
}
