//! %Proxy% (Abschnitt 28.2) mit `Proxy.revocable`. Die Objekte selbst und
//! ihre internen Methoden stehen in `object::proxy`.

use super::{define_functions, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{
    Builtin, BuiltinCall, create_builtin_closure, create_builtin_function, set_captures,
};
use crate::heap::{Local, Scope};
use crate::object::{ordinary_object_create, proxy_create, revoke};
use crate::operations::create_data_property_or_throw;
use crate::realm::{Intrinsic, RealmId};
use crate::value::Value;

static CONSTRUCTOR: Builtin = Builtin {
    name: "Proxy",
    length: 2,
    constructor: true,
    behaviour: proxy,
};

static FUNCTIONS: [Builtin; 1] = [Builtin {
    name: "revocable",
    length: 2,
    constructor: false,
    behaviour: revocable,
}];

/// Die Funktion, die einen Proxy aus Proxy.revocable widerruft; ihr
/// [[RevocableProxy]] ist, was sie einfängt.
static REVOKER: Builtin = Builtin {
    name: "",
    length: 0,
    constructor: false,
    behaviour: revoker,
};

/// %Proxy% hat keine Eigenschaft `prototype` (Abschnitt 28.2.2).
pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    define_functions(agent, scope, realm, constructor, &FUNCTIONS)?;
    agent.set_intrinsic(realm, Intrinsic::Proxy, constructor);
    Ok(())
}

/// Proxy ( target, handler ) (Abschnitt 28.2.1.1).
fn proxy<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if arguments.new_target.is_none() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let target = arguments.argument(agent, scope, 0);
    let handler = arguments.argument(agent, scope, 1);
    proxy_create(agent, scope, target, handler)
}

/// Proxy.revocable ( target, handler ) (Abschnitt 28.2.2.1).
fn revocable<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = arguments.argument(agent, scope, 0);
    let handler = arguments.argument(agent, scope, 1);
    let proxy = proxy_create(agent, scope, target, handler)?;
    let revoker = create_builtin_closure(agent, scope, arguments.realm, &REVOKER, proxy)?;
    let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::ObjectPrototype);
    let result = ordinary_object_create(agent, scope, Some(prototype))?;
    let key = agent.key_from_units(scope, &units("proxy"))?;
    create_data_property_or_throw(agent, scope, result, key, proxy)?;
    let key = agent.key_from_units(scope, &units("revoke"))?;
    create_data_property_or_throw(agent, scope, result, key, revoker)?;
    Ok(result)
}

/// Die Funktion zum Widerrufen (Abschnitt 28.2.2.1, Schritt 3): beim
/// ersten Aufruf widerruft sie den Proxy und vergisst ihn, danach tut sie
/// nichts mehr.
fn revoker<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let proxy = arguments.captures(agent, scope);
    let heap = agent.heap();
    let value = heap.get(proxy);
    if !value.is_null() && !value.is_undefined() {
        set_captures(heap, arguments.function, Value::NULL);
        revoke(heap, value);
    }
    Ok(heap.local(scope, Value::UNDEFINED))
}
