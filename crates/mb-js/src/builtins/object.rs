//! %Object% (Abschnitt 20.1.1) mit allen Funktionen des Konstruktors
//! (Abschnitt 20.1.2) und %Object.prototype% (Abschnitt 20.1.3) mit allen
//! Methoden, dazu `__proto__` und die vier Altmethoden aus Anhang B.2.2.

use super::{define_functions, link_constructor, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_closure, create_builtin_function};
use crate::compare::same_value;
use crate::conversion::{to_object, to_property_key};
use crate::descriptor::{PropertyDescriptor, from_property_descriptor, to_property_descriptor};
use crate::heap::{Local, ObjectClass, Scope};
use crate::iterator::{add_entries_from_iterable, group_by};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{
    get_own_property, get_prototype_of, is_extensible, ordinary_create_from_constructor,
    ordinary_object_create, own_property_keys, prevent_extensions, set_prototype_of,
};
use crate::operations::{
    EnumerableKind, IntegrityLevel, create_array_from_list, create_data_property_or_throw,
    define_property_or_throw, enumerable_own_properties, get, has_own_property, invoke, is_array,
    is_callable, set, set_integrity_level, test_integrity_level,
};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "Object",
    length: 1,
    constructor: true,
    behaviour: object,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 23] = [
    Builtin {
        name: "assign",
        length: 2,
        constructor: false,
        behaviour: object_assign,
    },
    Builtin {
        name: "create",
        length: 2,
        constructor: false,
        behaviour: object_create,
    },
    Builtin {
        name: "defineProperties",
        length: 2,
        constructor: false,
        behaviour: object_define_properties,
    },
    Builtin {
        name: "defineProperty",
        length: 3,
        constructor: false,
        behaviour: object_define_property,
    },
    Builtin {
        name: "entries",
        length: 1,
        constructor: false,
        behaviour: object_entries,
    },
    Builtin {
        name: "freeze",
        length: 1,
        constructor: false,
        behaviour: object_freeze,
    },
    Builtin {
        name: "fromEntries",
        length: 1,
        constructor: false,
        behaviour: object_from_entries,
    },
    Builtin {
        name: "getOwnPropertyDescriptor",
        length: 2,
        constructor: false,
        behaviour: object_get_own_property_descriptor,
    },
    Builtin {
        name: "getOwnPropertyDescriptors",
        length: 1,
        constructor: false,
        behaviour: object_get_own_property_descriptors,
    },
    Builtin {
        name: "getOwnPropertyNames",
        length: 1,
        constructor: false,
        behaviour: object_get_own_property_names,
    },
    Builtin {
        name: "getOwnPropertySymbols",
        length: 1,
        constructor: false,
        behaviour: object_get_own_property_symbols,
    },
    Builtin {
        name: "getPrototypeOf",
        length: 1,
        constructor: false,
        behaviour: object_get_prototype_of,
    },
    Builtin {
        name: "groupBy",
        length: 2,
        constructor: false,
        behaviour: object_group_by,
    },
    Builtin {
        name: "hasOwn",
        length: 2,
        constructor: false,
        behaviour: object_has_own,
    },
    Builtin {
        name: "is",
        length: 2,
        constructor: false,
        behaviour: object_is,
    },
    Builtin {
        name: "isExtensible",
        length: 1,
        constructor: false,
        behaviour: object_is_extensible,
    },
    Builtin {
        name: "isFrozen",
        length: 1,
        constructor: false,
        behaviour: object_is_frozen,
    },
    Builtin {
        name: "isSealed",
        length: 1,
        constructor: false,
        behaviour: object_is_sealed,
    },
    Builtin {
        name: "keys",
        length: 1,
        constructor: false,
        behaviour: object_keys,
    },
    Builtin {
        name: "preventExtensions",
        length: 1,
        constructor: false,
        behaviour: object_prevent_extensions,
    },
    Builtin {
        name: "seal",
        length: 1,
        constructor: false,
        behaviour: object_seal,
    },
    Builtin {
        name: "setPrototypeOf",
        length: 2,
        constructor: false,
        behaviour: object_set_prototype_of,
    },
    Builtin {
        name: "values",
        length: 1,
        constructor: false,
        behaviour: object_values,
    },
];

static PROTOTYPE_FUNCTIONS: [Builtin; 10] = [
    Builtin {
        name: "hasOwnProperty",
        length: 1,
        constructor: false,
        behaviour: object_prototype_has_own_property,
    },
    Builtin {
        name: "isPrototypeOf",
        length: 1,
        constructor: false,
        behaviour: object_prototype_is_prototype_of,
    },
    Builtin {
        name: "propertyIsEnumerable",
        length: 1,
        constructor: false,
        behaviour: object_prototype_property_is_enumerable,
    },
    Builtin {
        name: "toLocaleString",
        length: 0,
        constructor: false,
        behaviour: object_prototype_to_locale_string,
    },
    Builtin {
        name: "toString",
        length: 0,
        constructor: false,
        behaviour: object_prototype_to_string,
    },
    Builtin {
        name: "valueOf",
        length: 0,
        constructor: false,
        behaviour: object_prototype_value_of,
    },
    Builtin {
        name: "__defineGetter__",
        length: 2,
        constructor: false,
        behaviour: object_prototype_define_getter,
    },
    Builtin {
        name: "__defineSetter__",
        length: 2,
        constructor: false,
        behaviour: object_prototype_define_setter,
    },
    Builtin {
        name: "__lookupGetter__",
        length: 1,
        constructor: false,
        behaviour: object_prototype_lookup_getter,
    },
    Builtin {
        name: "__lookupSetter__",
        length: 1,
        constructor: false,
        behaviour: object_prototype_lookup_setter,
    },
];

/// get Object.prototype.__proto__ (Anhang B.2.2.1.1).
static PROTO_GETTER: Builtin = Builtin {
    name: "get __proto__",
    length: 0,
    constructor: false,
    behaviour: object_prototype_get_proto,
};

/// set Object.prototype.__proto__ (Anhang B.2.2.1.2).
static PROTO_SETTER: Builtin = Builtin {
    name: "set __proto__",
    length: 1,
    constructor: false,
    behaviour: object_prototype_set_proto,
};

/// Das Abstract Closure *adder* aus Object.fromEntries.
static FROM_ENTRIES_ADDER: Builtin = Builtin {
    name: "",
    length: 2,
    constructor: false,
    behaviour: from_entries_adder,
};

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    // Object.prototype.__proto__ (Anhang B.2.2.1): ein Accessor,
    // konfigurierbar und nicht aufzählbar.
    let getter = create_builtin_function(agent, scope, realm, &PROTO_GETTER)?;
    let setter = create_builtin_function(agent, scope, realm, &PROTO_SETTER)?;
    let key = agent.key_from_units(scope, &units("__proto__"))?;
    let descriptor = PropertyDescriptor::accessor(getter, setter, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    agent.set_intrinsic(realm, Intrinsic::Object, constructor);
    Ok(())
}

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

/// RequireObjectCoercible (Abschnitt 7.2.1).
fn require_object_coercible(agent: &mut Agent, value: Local<'_>) -> Completion<()> {
    let found = agent.heap().get(value);
    if found.is_undefined() || found.is_null() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(())
}

/// Ein Objekt oder *null* als [[Prototype]], sonst ein TypeError.
fn prototype_argument<'s>(agent: &mut Agent, value: Local<'s>) -> Completion<Option<Local<'s>>> {
    match agent.heap().get(value).kind() {
        Kind::Object(_) => Ok(Some(value)),
        Kind::Null => Ok(None),
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

fn boolean<'s>(agent: &Agent, scope: &'s Scope, value: bool) -> Local<'s> {
    agent.heap().local(scope, Value::boolean(value))
}

fn null_or<'s>(agent: &Agent, scope: &'s Scope, value: Option<Local<'s>>) -> Local<'s> {
    value.unwrap_or_else(|| agent.heap().local(scope, Value::NULL))
}

/// Object ( [ value ] ) (Abschnitt 20.1.1.1).
fn object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if let Some(new_target) = arguments.new_target {
        let heap = agent.heap();
        if !same_value(heap, heap.get(new_target), heap.get(arguments.function)) {
            return ordinary_create_from_constructor(
                agent,
                scope,
                new_target,
                Intrinsic::ObjectPrototype,
                arguments.realm,
            );
        }
    }
    let value = arguments.argument(agent, scope, 0);
    let found = agent.heap().get(value);
    if found.is_undefined() || found.is_null() {
        let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::ObjectPrototype);
        return Ok(ordinary_object_create(agent, scope, Some(prototype))?);
    }
    to_object(agent, scope, arguments.realm, value)
}

/// Object.assign ( target, ...sources ) (Abschnitt 20.1.2.1).
fn object_assign<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let target = arguments.argument(agent, scope, 0);
    let target = to_object(agent, scope, arguments.realm, target)?;
    for &source in arguments.arguments.get(1..).unwrap_or_default() {
        let found = agent.heap().get(source);
        if found.is_undefined() || found.is_null() {
            continue;
        }
        let from = to_object(agent, scope, arguments.realm, source)?;
        let keys = own_property_keys(agent, scope, from)?;
        for key in keys {
            agent.scope(|agent, inner| -> Completion<()> {
                let Some(descriptor) = get_own_property(agent, inner, from, key)? else {
                    return Ok(());
                };
                if descriptor.enumerable != Some(true) {
                    return Ok(());
                }
                let value = get(agent, inner, from, key)?;
                set(agent, inner, target, key, value, true)
            })?;
        }
    }
    Ok(target)
}

/// Object.create ( O, Properties ) (Abschnitt 20.1.2.2).
fn object_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let prototype = arguments.argument(agent, scope, 0);
    let prototype = prototype_argument(agent, prototype)?;
    let object = ordinary_object_create(agent, scope, prototype)?;
    let properties = arguments.argument(agent, scope, 1);
    if agent.heap().get(properties).is_undefined() {
        return Ok(object);
    }
    define_properties(agent, scope, arguments.realm, object, properties)
}

/// ObjectDefineProperties (Abschnitt 20.1.2.3.1).
fn define_properties<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    object: Local<'s>,
    properties: Local<'_>,
) -> Completion<Local<'s>> {
    let properties = to_object(agent, scope, realm, properties)?;
    let keys = own_property_keys(agent, scope, properties)?;
    let mut descriptors: Vec<(PropertyKey<'s>, PropertyDescriptor<'s>)> = Vec::new();
    for key in keys {
        let Some(current) = get_own_property(agent, scope, properties, key)? else {
            continue;
        };
        if current.enumerable != Some(true) {
            continue;
        }
        let value = get(agent, scope, properties, key)?;
        let descriptor = to_property_descriptor(agent, scope, value)?;
        descriptors.push((key, descriptor));
    }
    for (key, descriptor) in &descriptors {
        define_property_or_throw(agent, scope, object, *key, descriptor)?;
    }
    Ok(object)
}

/// Object.defineProperties ( O, Properties ) (Abschnitt 20.1.2.3).
fn object_define_properties<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    if !is_object(agent, object) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let properties = arguments.argument(agent, scope, 1);
    define_properties(agent, scope, arguments.realm, object, properties)
}

/// Object.defineProperty ( O, P, Attributes ) (Abschnitt 20.1.2.4).
fn object_define_property<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    if !is_object(agent, object) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let key = arguments.argument(agent, scope, 1);
    let key = to_property_key(agent, scope, key)?;
    let attributes = arguments.argument(agent, scope, 2);
    let descriptor = to_property_descriptor(agent, scope, attributes)?;
    define_property_or_throw(agent, scope, object, key, &descriptor)?;
    Ok(object)
}

/// Object.entries, Object.keys und Object.values (Abschnitte 20.1.2.5,
/// 20.1.2.19 und 20.1.2.24).
fn enumerable_list<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    kind: EnumerableKind,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    let object = to_object(agent, scope, arguments.realm, object)?;
    let list = enumerable_own_properties(agent, scope, arguments.realm, object, kind)?;
    create_array_from_list(agent, scope, arguments.realm, &list)
}

/// Object.entries ( O ) (Abschnitt 20.1.2.5).
fn object_entries<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    enumerable_list(agent, scope, arguments, EnumerableKind::KeyValue)
}

/// Object.keys ( O ) (Abschnitt 20.1.2.19).
fn object_keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    enumerable_list(agent, scope, arguments, EnumerableKind::Key)
}

/// Object.values ( O ) (Abschnitt 20.1.2.24).
fn object_values<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    enumerable_list(agent, scope, arguments, EnumerableKind::Value)
}

/// Object.freeze und Object.seal (Abschnitte 20.1.2.6 und 20.1.2.22).
fn set_level<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    level: IntegrityLevel,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    if !is_object(agent, object) {
        return Ok(object);
    }
    if !set_integrity_level(agent, scope, object, level)? {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(object)
}

/// Object.freeze ( O ) (Abschnitt 20.1.2.6).
fn object_freeze<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_level(agent, scope, arguments, IntegrityLevel::Frozen)
}

/// Object.seal ( O ) (Abschnitt 20.1.2.22).
fn object_seal<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_level(agent, scope, arguments, IntegrityLevel::Sealed)
}

/// Object.isFrozen und Object.isSealed (Abschnitte 20.1.2.17 und
/// 20.1.2.18).
fn test_level<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    level: IntegrityLevel,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    if !is_object(agent, object) {
        return Ok(boolean(agent, scope, true));
    }
    let fixed = test_integrity_level(agent, scope, object, level)?;
    Ok(boolean(agent, scope, fixed))
}

/// Object.isFrozen ( O ) (Abschnitt 20.1.2.17).
fn object_is_frozen<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    test_level(agent, scope, arguments, IntegrityLevel::Frozen)
}

/// Object.isSealed ( O ) (Abschnitt 20.1.2.18).
fn object_is_sealed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    test_level(agent, scope, arguments, IntegrityLevel::Sealed)
}

/// Object.fromEntries ( iterable ) (Abschnitt 20.1.2.7).
fn object_from_entries<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let iterable = arguments.argument(agent, scope, 0);
    require_object_coercible(agent, iterable)?;
    let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::ObjectPrototype);
    let object = ordinary_object_create(agent, scope, Some(prototype))?;
    let adder = create_builtin_closure(agent, scope, arguments.realm, &FROM_ENTRIES_ADDER, object)?;
    add_entries_from_iterable(agent, scope, arguments.realm, object, iterable, adder)
}

/// Das Abstract Closure aus Object.fromEntries: legt `key` mit `value` am
/// eingefangenen Objekt an.
fn from_entries_adder<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.captures(agent, scope);
    let key = arguments.argument(agent, scope, 0);
    let key = to_property_key(agent, scope, key)?;
    let value = arguments.argument(agent, scope, 1);
    create_data_property_or_throw(agent, scope, object, key, value)?;
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Object.getOwnPropertyDescriptor ( O, P ) (Abschnitt 20.1.2.8).
fn object_get_own_property_descriptor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    let object = to_object(agent, scope, arguments.realm, object)?;
    let key = arguments.argument(agent, scope, 1);
    let key = to_property_key(agent, scope, key)?;
    let descriptor = get_own_property(agent, scope, object, key)?;
    from_property_descriptor(agent, scope, arguments.realm, descriptor.as_ref())
}

/// Object.getOwnPropertyDescriptors ( O ) (Abschnitt 20.1.2.9).
fn object_get_own_property_descriptors<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    let object = to_object(agent, scope, arguments.realm, object)?;
    let keys = own_property_keys(agent, scope, object)?;
    let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::ObjectPrototype);
    let descriptors = ordinary_object_create(agent, scope, Some(prototype))?;
    for key in keys {
        agent.scope(|agent, inner| -> Completion<()> {
            let Some(descriptor) = get_own_property(agent, inner, object, key)? else {
                return Ok(());
            };
            let value = from_property_descriptor(agent, inner, arguments.realm, Some(&descriptor))?;
            create_data_property_or_throw(agent, inner, descriptors, key, value)
        })?;
    }
    Ok(descriptors)
}

/// GetOwnPropertyKeys (Abschnitt 20.1.2.11.1) als Array.
fn get_own_property_keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    symbols: bool,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    let object = to_object(agent, scope, arguments.realm, object)?;
    let keys = own_property_keys(agent, scope, object)?;
    let mut names = Vec::new();
    for key in keys {
        if matches!(key, PropertyKey::Symbol(_)) == symbols {
            names.push(agent.key_as_value(scope, key)?);
        }
    }
    create_array_from_list(agent, scope, arguments.realm, &names)
}

/// Object.getOwnPropertyNames ( O ) (Abschnitt 20.1.2.10).
fn object_get_own_property_names<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_own_property_keys(agent, scope, arguments, false)
}

/// Object.getOwnPropertySymbols ( O ) (Abschnitt 20.1.2.11).
fn object_get_own_property_symbols<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_own_property_keys(agent, scope, arguments, true)
}

/// Object.getPrototypeOf ( O ) (Abschnitt 20.1.2.12).
fn object_get_prototype_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    let object = to_object(agent, scope, arguments.realm, object)?;
    let prototype = get_prototype_of(agent, scope, object)?;
    Ok(null_or(agent, scope, prototype))
}

/// Object.groupBy ( items, callback ) (Abschnitt 20.1.2.13).
fn object_group_by<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let items = arguments.argument(agent, scope, 0);
    let callback = arguments.argument(agent, scope, 1);
    let groups = group_by(agent, scope, arguments.realm, items, callback)?;
    let object = ordinary_object_create(agent, scope, None)?;
    for (key, elements) in groups {
        let elements = create_array_from_list(agent, scope, arguments.realm, &elements)?;
        // Die Schlüssel kamen aus ToPropertyKey.
        let Some(key) = agent.key_from_value(scope, key) else {
            return Err(agent.throw_error(ErrorKind::TypeError));
        };
        create_data_property_or_throw(agent, scope, object, key, elements)?;
    }
    Ok(object)
}

/// Object.hasOwn ( O, P ) (Abschnitt 20.1.2.14).
fn object_has_own<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    let object = to_object(agent, scope, arguments.realm, object)?;
    let key = arguments.argument(agent, scope, 1);
    let key = to_property_key(agent, scope, key)?;
    let found = has_own_property(agent, scope, object, key)?;
    Ok(boolean(agent, scope, found))
}

/// Object.is ( value1, value2 ) (Abschnitt 20.1.2.15).
fn object_is<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let left = arguments.argument(agent, scope, 0);
    let right = arguments.argument(agent, scope, 1);
    let heap = agent.heap();
    let same = same_value(heap, heap.get(left), heap.get(right));
    Ok(boolean(agent, scope, same))
}

/// Object.isExtensible ( O ) (Abschnitt 20.1.2.16).
fn object_is_extensible<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    if !is_object(agent, object) {
        return Ok(boolean(agent, scope, false));
    }
    let extensible = is_extensible(agent, object)?;
    Ok(boolean(agent, scope, extensible))
}

/// Object.preventExtensions ( O ) (Abschnitt 20.1.2.20).
fn object_prevent_extensions<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    if !is_object(agent, object) {
        return Ok(object);
    }
    if !prevent_extensions(agent, object)? {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(object)
}

/// Object.setPrototypeOf ( O, proto ) (Abschnitt 20.1.2.23).
fn object_set_prototype_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.argument(agent, scope, 0);
    require_object_coercible(agent, object)?;
    let prototype = arguments.argument(agent, scope, 1);
    let prototype = prototype_argument(agent, prototype)?;
    if !is_object(agent, object) {
        return Ok(object);
    }
    if !set_prototype_of(agent, object, prototype)? {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(object)
}

/// Object.prototype.hasOwnProperty ( V ) (Abschnitt 20.1.3.2).
fn object_prototype_has_own_property<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let key = arguments.argument(agent, scope, 0);
    let key = to_property_key(agent, scope, key)?;
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    let found = has_own_property(agent, scope, object, key)?;
    Ok(boolean(agent, scope, found))
}

/// Object.prototype.isPrototypeOf ( V ) (Abschnitt 20.1.3.3). Die
/// Prototypkette wird ohne Rekursion und mit einem Handle durchlaufen
/// (RS-8).
fn object_prototype_is_prototype_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    if !is_object(agent, value) {
        return Ok(boolean(agent, scope, false));
    }
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    loop {
        let found = agent.scope(|agent, inner| -> Completion<Option<bool>> {
            let Some(next) = get_prototype_of(agent, inner, value)? else {
                return Ok(Some(false));
            };
            let heap = agent.heap();
            if same_value(heap, heap.get(object), heap.get(next)) {
                return Ok(Some(true));
            }
            heap.set(value, heap.get(next));
            Ok(None)
        })?;
        if let Some(found) = found {
            return Ok(boolean(agent, scope, found));
        }
    }
}

/// Object.prototype.propertyIsEnumerable ( V ) (Abschnitt 20.1.3.4).
fn object_prototype_property_is_enumerable<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let key = arguments.argument(agent, scope, 0);
    let key = to_property_key(agent, scope, key)?;
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    let descriptor = get_own_property(agent, scope, object, key)?;
    let enumerable = descriptor.is_some_and(|descriptor| descriptor.enumerable == Some(true));
    Ok(boolean(agent, scope, enumerable))
}

/// Object.prototype.toLocaleString ( [ reserved1 [ , reserved2 ] ] )
/// (Abschnitt 20.1.3.5).
fn object_prototype_to_locale_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let key = agent.key_from_units(scope, &units("toString"))?;
    invoke(agent, scope, arguments.realm, arguments.this, key, &[])
}

/// Der Teil *builtinTag* aus Object.prototype.toString.
fn builtin_tag(agent: &mut Agent, object: Local<'_>) -> Completion<&'static str> {
    if is_array(agent, object)? {
        return Ok("Array");
    }
    let heap = agent.heap();
    let value = heap.get(object);
    let Kind::Object(reference) = value.kind() else {
        return Ok("Object");
    };
    let class = heap.object_class(reference);
    if class == Some(ObjectClass::Arguments) {
        return Ok("Arguments");
    }
    if is_callable(heap, value) {
        return Ok("Function");
    }
    Ok(match class {
        Some(ObjectClass::Error) => "Error",
        Some(ObjectClass::Primitive) => match heap.primitive_data(reference).map(Value::kind) {
            Some(Kind::Boolean(_)) => "Boolean",
            Some(Kind::Number(_)) => "Number",
            Some(Kind::String(_)) => "String",
            // Symbol-Objekte haben [[SymbolData]], das hier nicht zählt.
            _ => "Object",
        },
        _ => "Object",
    })
}

/// Object.prototype.toString ( ) (Abschnitt 20.1.3.6).
pub(super) fn object_prototype_to_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = agent.heap().get(arguments.this);
    if this.is_undefined() {
        return Ok(agent
            .heap_mut()
            .alloc_string(scope, &units("[object Undefined]"))?);
    }
    if this.is_null() {
        return Ok(agent
            .heap_mut()
            .alloc_string(scope, &units("[object Null]"))?);
    }
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    let builtin = builtin_tag(agent, object)?;
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::ToStringTag);
    let tag = get(agent, scope, object, PropertyKey::Symbol(symbol))?;
    let mut text = units("[object ");
    let heap = agent.heap();
    match heap.get(tag).kind() {
        Kind::String(tag) => text.extend(heap.string_units(tag)),
        _ => text.extend(units(builtin)),
    }
    text.push(u16::from(b']'));
    Ok(agent.heap_mut().alloc_string(scope, &text)?)
}

/// Object.prototype.valueOf ( ) (Abschnitt 20.1.3.7).
fn object_prototype_value_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    to_object(agent, scope, arguments.realm, arguments.this)
}

/// get Object.prototype.__proto__ (Anhang B.2.2.1.1).
fn object_prototype_get_proto<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    let prototype = get_prototype_of(agent, scope, object)?;
    Ok(null_or(agent, scope, prototype))
}

/// set Object.prototype.__proto__ (Anhang B.2.2.1.2).
fn object_prototype_set_proto<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_object_coercible(agent, arguments.this)?;
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let prototype = arguments.argument(agent, scope, 0);
    let prototype = match agent.heap().get(prototype).kind() {
        Kind::Object(_) => Some(prototype),
        Kind::Null => None,
        _ => return Ok(undefined),
    };
    if !is_object(agent, arguments.this) {
        return Ok(undefined);
    }
    if !set_prototype_of(agent, arguments.this, prototype)? {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(undefined)
}

/// Object.prototype.__defineGetter__ und __defineSetter__ (Anhang B.2.2.2
/// und B.2.2.3).
fn define_accessor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    getter: bool,
) -> Completion<Local<'s>> {
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    let function = arguments.argument(agent, scope, 1);
    if !is_callable(agent.heap(), agent.heap().get(function)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let descriptor = PropertyDescriptor {
        get: getter.then_some(function),
        set: (!getter).then_some(function),
        enumerable: Some(true),
        configurable: Some(true),
        ..PropertyDescriptor::default()
    };
    let key = arguments.argument(agent, scope, 0);
    let key = to_property_key(agent, scope, key)?;
    define_property_or_throw(agent, scope, object, key, &descriptor)?;
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Object.prototype.__defineGetter__ ( P, getter ) (Anhang B.2.2.2).
fn object_prototype_define_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    define_accessor(agent, scope, arguments, true)
}

/// Object.prototype.__defineSetter__ ( P, setter ) (Anhang B.2.2.3).
fn object_prototype_define_setter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    define_accessor(agent, scope, arguments, false)
}

/// Object.prototype.__lookupGetter__ und __lookupSetter__ (Anhang B.2.2.4
/// und B.2.2.5): die Prototypkette ohne Rekursion und mit einem Handle
/// (RS-8).
fn lookup_accessor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    getter: bool,
) -> Completion<Local<'s>> {
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    let key = arguments.argument(agent, scope, 0);
    let key = to_property_key(agent, scope, key)?;
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    loop {
        let found = agent.scope(|agent, inner| -> Completion<Option<Local<'s>>> {
            if let Some(descriptor) = get_own_property(agent, inner, object, key)? {
                let function = if getter {
                    descriptor.get
                } else {
                    descriptor.set
                };
                let function = function.filter(|_| descriptor.is_accessor());
                return Ok(Some(match function {
                    Some(function) => {
                        let heap = agent.heap();
                        heap.local(scope, heap.get(function))
                    }
                    None => undefined,
                }));
            }
            let Some(next) = get_prototype_of(agent, inner, object)? else {
                return Ok(Some(undefined));
            };
            let heap = agent.heap();
            heap.set(object, heap.get(next));
            Ok(None)
        })?;
        if let Some(found) = found {
            return Ok(found);
        }
    }
}

/// Object.prototype.__lookupGetter__ ( P ) (Anhang B.2.2.4).
fn object_prototype_lookup_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    lookup_accessor(agent, scope, arguments, true)
}

/// Object.prototype.__lookupSetter__ ( P ) (Anhang B.2.2.5).
fn object_prototype_lookup_setter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    lookup_accessor(agent, scope, arguments, false)
}
