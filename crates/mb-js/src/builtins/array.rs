//! Die Methoden von %Array.prototype% (Abschnitt 23.1.3), die Iteratoren
//! liefern: `values`, das auch @@iterator ist, `keys` und `entries`. Den
//! Konstruktor %Array% und die übrigen Methoden bringt die Welle für
//! Arrays.

use super::define_symbol_property;
use super::iterator::{ENTRIES, KEYS, VALUES, iterator_create};
use crate::agent::{Agent, Completion};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function, define_builtin_function};
use crate::conversion::to_object;
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::key::WellKnownSymbol;
use crate::operations::define_property_or_throw;
use crate::realm::{Intrinsic, RealmId};

static VALUES_FUNCTION: Builtin = Builtin {
    name: "values",
    length: 0,
    constructor: false,
    behaviour: array_prototype_values,
};

static PROTOTYPE_FUNCTIONS: [Builtin; 2] = [
    Builtin {
        name: "entries",
        length: 0,
        constructor: false,
        behaviour: array_prototype_entries,
    },
    Builtin {
        name: "keys",
        length: 0,
        constructor: false,
        behaviour: array_prototype_keys,
    },
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    for builtin in &PROTOTYPE_FUNCTIONS {
        define_builtin_function(agent, scope, realm, prototype, builtin)?;
    }
    // Array.prototype [ %Symbol.iterator% ] (Abschnitt 23.1.3.40) ist
    // dasselbe Funktionsobjekt wie `values`.
    let values = create_builtin_function(agent, scope, realm, &VALUES_FUNCTION)?;
    let key = agent.key_from_units(scope, &"values".encode_utf16().collect::<Vec<_>>())?;
    let descriptor = PropertyDescriptor::data(values, true, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    define_symbol_property(agent, scope, prototype, WellKnownSymbol::Iterator, values)?;
    agent.set_intrinsic(realm, Intrinsic::ArrayPrototypeValues, values);
    Ok(())
}

/// CreateArrayIterator (Abschnitt 23.1.5.1) für ToObject(*this*).
fn array_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    kind: i32,
) -> Completion<Local<'s>> {
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::ArrayIteratorPrototype);
    Ok(iterator_create(agent, scope, prototype, object, kind)?)
}

/// Array.prototype.entries ( ) (Abschnitt 23.1.3.5).
fn array_prototype_entries<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    array_iterator(agent, scope, arguments, ENTRIES)
}

/// Array.prototype.keys ( ) (Abschnitt 23.1.3.19).
fn array_prototype_keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    array_iterator(agent, scope, arguments, KEYS)
}

/// Array.prototype.values ( ) (Abschnitt 23.1.3.38).
fn array_prototype_values<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    array_iterator(agent, scope, arguments, VALUES)
}
