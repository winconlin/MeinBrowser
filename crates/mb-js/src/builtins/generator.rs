//! %GeneratorFunction.prototype% (Abschnitt 27.6.3), der Prototyp der
//! Generatorfunktionen, und %GeneratorPrototype% (Abschnitt 27.8.1), der
//! Prototyp ihrer `prototype`-Objekte, mit `next`, `return` und `throw`.
//!
//! Den Konstruktor %GeneratorFunction% (Abschnitt 27.6.1) gibt es noch
//! nicht, denn er braucht CreateDynamicFunction (Abschnitt 20.2.1.1.1) wie
//! `Function`; darum fehlt auch `constructor` an %GeneratorFunction.prototype%.

use super::{define_functions, define_to_string_tag, units};
use crate::agent::{Agent, Completion};
use crate::builtin::{Builtin, BuiltinCall};
use crate::descriptor::PropertyDescriptor;
use crate::generator::{Resumption, resume_generator};
use crate::heap::{Local, Scope};
use crate::object::ordinary_object_create;
use crate::operations::define_property_or_throw;
use crate::realm::{Intrinsic, RealmId};

static GENERATOR_FUNCTIONS: [Builtin; 3] = [
    Builtin {
        name: "next",
        length: 1,
        constructor: false,
        behaviour: generator_next,
    },
    Builtin {
        name: "return",
        length: 1,
        constructor: false,
        behaviour: generator_return,
    },
    Builtin {
        name: "throw",
        length: 1,
        constructor: false,
        behaviour: generator_throw,
    },
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let function = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
    let generator_function = ordinary_object_create(agent, scope, Some(function))?;
    let iterator = agent.intrinsic(scope, realm, Intrinsic::IteratorPrototype);
    let generator = ordinary_object_create(agent, scope, Some(iterator))?;
    // GeneratorFunction.prototype.prototype (Abschnitt 27.6.3.2) und
    // %GeneratorPrototype%.constructor (Abschnitt 27.8.1.1): nur
    // konfigurierbar.
    let key = agent.key_from_units(scope, &units("prototype"))?;
    let descriptor = PropertyDescriptor::data(generator, false, false, true);
    define_property_or_throw(agent, scope, generator_function, key, &descriptor)?;
    define_to_string_tag(agent, scope, generator_function, "GeneratorFunction")?;
    let key = agent.key_from_units(scope, &units("constructor"))?;
    let descriptor = PropertyDescriptor::data(generator_function, false, false, true);
    define_property_or_throw(agent, scope, generator, key, &descriptor)?;
    define_functions(agent, scope, realm, generator, &GENERATOR_FUNCTIONS)?;
    define_to_string_tag(agent, scope, generator, "Generator")?;
    agent.set_intrinsic(
        realm,
        Intrinsic::GeneratorFunctionPrototype,
        generator_function,
    );
    agent.set_intrinsic(realm, Intrinsic::GeneratorPrototype, generator);
    Ok(())
}

/// %GeneratorPrototype%.next ( value ) (Abschnitt 27.8.1.2).
fn generator_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume(agent, scope, arguments, Resumption::Next)
}

/// %GeneratorPrototype%.return ( value ) (Abschnitt 27.8.1.3).
fn generator_return<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume(agent, scope, arguments, Resumption::Return)
}

/// %GeneratorPrototype%.throw ( exception ) (Abschnitt 27.8.1.4).
fn generator_throw<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    resume(agent, scope, arguments, Resumption::Throw)
}

/// Setzt den Generator `this` mit dem ersten Argument fort.
fn resume<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    resumption: Resumption,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    resume_generator(
        agent,
        scope,
        arguments.realm,
        arguments.this,
        resumption,
        value,
    )
}
