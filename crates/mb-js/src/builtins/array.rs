//! %Array% (Abschnitt 23.1.1) mit `from`, `fromAsync`, `isArray`, `of` und
//! @@species sowie %Array.prototype% (Abschnitt 23.1.3) mit allen Methoden
//! und @@unscopables. Die Sortierung liegt in [`sort`], `fromAsync` in
//! [`from_async`].
//!
//! Alle Methoden sind generisch: Sie arbeiten mit ToObject(*this*) und
//! LengthOfArrayLike; ein Index über 2³² − 2 ist ein Zeichenkettenschlüssel.
//! Jedes Element kostet einen Arbeitsschritt, damit eine riesige `length`
//! am Budget scheitert, statt den Agent zu blockieren. Eine Schleife hält
//! die Werte eines Durchlaufs in einem inneren Bereich, damit die Handles
//! nicht mit der Länge wachsen; was einen Durchlauf überdauert, steht in
//! einem Handle des äußeren Bereichs, das der Durchlauf überschreibt.

mod from_async;
mod sort;

use self::sort::{Holes, sort_indexed_properties};
use super::iterator::{ENTRIES, KEYS, VALUES, iterator_create};
use super::object::object_prototype_to_string;
use super::{define_functions, define_symbol_property, link_constructor, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::compare::{is_strictly_equal, same_value, same_value_zero};
use crate::conversion::{to_boolean, to_integer_or_infinity, to_object, to_string, to_uint32};
use crate::descriptor::PropertyDescriptor;
use crate::function::{get_function_realm, is_constructor};
use crate::heap::{Local, OutOfMemory, Scope};
use crate::iterator::{
    close_if_abrupt, get_iterator_from_method, get_method_of_value, iterator_step_value,
};
use crate::key::{MAX_ARRAY_INDEX, PropertyKey, WellKnownSymbol};
use crate::number;
use crate::object::{array_create, get_prototype_from_constructor, ordinary_object_create};
use crate::operations::{
    call, construct, create_data_property_or_throw, define_property_or_throw,
    delete_property_or_throw, get, has_property, invoke, is_array, is_callable,
    length_of_array_like, set,
};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// 2⁵³ − 1, die größte Länge eines Array-ähnlichen Objekts.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

static CONSTRUCTOR: Builtin = Builtin {
    name: "Array",
    length: 1,
    constructor: true,
    behaviour: array,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 4] = [
    Builtin {
        name: "from",
        length: 1,
        constructor: false,
        behaviour: array_from,
    },
    Builtin {
        name: "fromAsync",
        length: 1,
        constructor: false,
        behaviour: from_async::array_from_async,
    },
    Builtin {
        name: "isArray",
        length: 1,
        constructor: false,
        behaviour: array_is_array,
    },
    Builtin {
        name: "of",
        length: 0,
        constructor: false,
        behaviour: array_of,
    },
];

static SPECIES: Builtin = Builtin {
    name: "get [Symbol.species]",
    length: 0,
    constructor: false,
    behaviour: species,
};

static VALUES_FUNCTION: Builtin = Builtin {
    name: "values",
    length: 0,
    constructor: false,
    behaviour: array_prototype_values,
};

static PROTOTYPE_FUNCTIONS: [Builtin; 37] = [
    Builtin {
        name: "at",
        length: 1,
        constructor: false,
        behaviour: array_prototype_at,
    },
    Builtin {
        name: "concat",
        length: 1,
        constructor: false,
        behaviour: array_prototype_concat,
    },
    Builtin {
        name: "copyWithin",
        length: 2,
        constructor: false,
        behaviour: array_prototype_copy_within,
    },
    Builtin {
        name: "entries",
        length: 0,
        constructor: false,
        behaviour: array_prototype_entries,
    },
    Builtin {
        name: "every",
        length: 1,
        constructor: false,
        behaviour: array_prototype_every,
    },
    Builtin {
        name: "fill",
        length: 1,
        constructor: false,
        behaviour: array_prototype_fill,
    },
    Builtin {
        name: "filter",
        length: 1,
        constructor: false,
        behaviour: array_prototype_filter,
    },
    Builtin {
        name: "find",
        length: 1,
        constructor: false,
        behaviour: array_prototype_find,
    },
    Builtin {
        name: "findIndex",
        length: 1,
        constructor: false,
        behaviour: array_prototype_find_index,
    },
    Builtin {
        name: "findLast",
        length: 1,
        constructor: false,
        behaviour: array_prototype_find_last,
    },
    Builtin {
        name: "findLastIndex",
        length: 1,
        constructor: false,
        behaviour: array_prototype_find_last_index,
    },
    Builtin {
        name: "flat",
        length: 0,
        constructor: false,
        behaviour: array_prototype_flat,
    },
    Builtin {
        name: "flatMap",
        length: 1,
        constructor: false,
        behaviour: array_prototype_flat_map,
    },
    Builtin {
        name: "forEach",
        length: 1,
        constructor: false,
        behaviour: array_prototype_for_each,
    },
    Builtin {
        name: "includes",
        length: 1,
        constructor: false,
        behaviour: array_prototype_includes,
    },
    Builtin {
        name: "indexOf",
        length: 1,
        constructor: false,
        behaviour: array_prototype_index_of,
    },
    Builtin {
        name: "join",
        length: 1,
        constructor: false,
        behaviour: array_prototype_join,
    },
    Builtin {
        name: "keys",
        length: 0,
        constructor: false,
        behaviour: array_prototype_keys,
    },
    Builtin {
        name: "lastIndexOf",
        length: 1,
        constructor: false,
        behaviour: array_prototype_last_index_of,
    },
    Builtin {
        name: "map",
        length: 1,
        constructor: false,
        behaviour: array_prototype_map,
    },
    Builtin {
        name: "pop",
        length: 0,
        constructor: false,
        behaviour: array_prototype_pop,
    },
    Builtin {
        name: "push",
        length: 1,
        constructor: false,
        behaviour: array_prototype_push,
    },
    Builtin {
        name: "reduce",
        length: 1,
        constructor: false,
        behaviour: array_prototype_reduce,
    },
    Builtin {
        name: "reduceRight",
        length: 1,
        constructor: false,
        behaviour: array_prototype_reduce_right,
    },
    Builtin {
        name: "reverse",
        length: 0,
        constructor: false,
        behaviour: array_prototype_reverse,
    },
    Builtin {
        name: "shift",
        length: 0,
        constructor: false,
        behaviour: array_prototype_shift,
    },
    Builtin {
        name: "slice",
        length: 2,
        constructor: false,
        behaviour: array_prototype_slice,
    },
    Builtin {
        name: "some",
        length: 1,
        constructor: false,
        behaviour: array_prototype_some,
    },
    Builtin {
        name: "sort",
        length: 1,
        constructor: false,
        behaviour: array_prototype_sort,
    },
    Builtin {
        name: "splice",
        length: 2,
        constructor: false,
        behaviour: array_prototype_splice,
    },
    Builtin {
        name: "toLocaleString",
        length: 0,
        constructor: false,
        behaviour: array_prototype_to_locale_string,
    },
    Builtin {
        name: "toReversed",
        length: 0,
        constructor: false,
        behaviour: array_prototype_to_reversed,
    },
    Builtin {
        name: "toSorted",
        length: 1,
        constructor: false,
        behaviour: array_prototype_to_sorted,
    },
    Builtin {
        name: "toSpliced",
        length: 2,
        constructor: false,
        behaviour: array_prototype_to_spliced,
    },
    Builtin {
        name: "toString",
        length: 0,
        constructor: false,
        behaviour: array_prototype_to_string,
    },
    Builtin {
        name: "unshift",
        length: 1,
        constructor: false,
        behaviour: array_prototype_unshift,
    },
    Builtin {
        name: "with",
        length: 2,
        constructor: false,
        behaviour: array_prototype_with,
    },
];

/// Die Namen in Array.prototype [ %Symbol.unscopables% ] (Abschnitt
/// 23.1.3.41).
const UNSCOPABLES: [&str; 16] = [
    "at",
    "copyWithin",
    "entries",
    "fill",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "flat",
    "flatMap",
    "includes",
    "keys",
    "toReversed",
    "toSorted",
    "toSpliced",
    "values",
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    // get Array [ %Symbol.species% ] (Abschnitt 23.1.2.5): ein Accessor
    // ohne Setter, konfigurierbar und nicht aufzählbar.
    let getter = create_builtin_function(agent, scope, realm, &SPECIES)?;
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::Species));
    let descriptor = PropertyDescriptor::accessor(getter, undefined, false, true);
    define_property_or_throw(agent, scope, constructor, key, &descriptor)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    // Array.prototype [ %Symbol.iterator% ] (Abschnitt 23.1.3.40) ist
    // dasselbe Funktionsobjekt wie `values`.
    let values = create_builtin_function(agent, scope, realm, &VALUES_FUNCTION)?;
    let key = agent.key_from_units(scope, &units("values"))?;
    let descriptor = PropertyDescriptor::data(values, true, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    define_symbol_property(agent, scope, prototype, WellKnownSymbol::Iterator, values)?;
    agent.set_intrinsic(realm, Intrinsic::ArrayPrototypeValues, values);
    // Array.prototype [ %Symbol.unscopables% ] (Abschnitt 23.1.3.41): ein
    // Objekt ohne Prototyp, nur konfigurierbar.
    let unscopables = ordinary_object_create(agent, scope, None)?;
    let truth = agent.heap().local(scope, Value::TRUE);
    for name in UNSCOPABLES {
        let key = agent.key_from_units(scope, &units(name))?;
        create_data_property_or_throw(agent, scope, unscopables, key, truth)?;
    }
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::Unscopables));
    let descriptor = PropertyDescriptor::data(unscopables, false, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    agent.set_intrinsic(realm, Intrinsic::Array, constructor);
    Ok(())
}

// Helfer

fn number<'s>(agent: &Agent, scope: &'s Scope, value: f64) -> Local<'s> {
    agent.heap().local(scope, Value::number(value))
}

fn boolean<'s>(agent: &Agent, scope: &'s Scope, value: bool) -> Local<'s> {
    agent.heap().local(scope, Value::boolean(value))
}

fn undefined<'s>(agent: &Agent, scope: &'s Scope) -> Local<'s> {
    agent.heap().local(scope, Value::UNDEFINED)
}

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

/// Ein Arbeitsschritt je Element, damit riesige Längen am Budget enden.
pub(super) fn step(agent: &Agent) -> Completion<()> {
    agent.spend_execution_steps(1)
}

/// ToObject(*this*), der erste Schritt fast jeder Methode.
fn this_object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    to_object(agent, scope, arguments.realm, arguments.this)
}

/// Der Schlüssel zum Index `index` (ToString eines Index): bis 2³² − 2 ein
/// Index, darüber die Zeichenkette seiner Ziffern.
pub(super) fn index_key<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    index: f64,
) -> Completion<PropertyKey<'s>> {
    if index >= 0.0 && index <= f64::from(MAX_ARRAY_INDEX) && index.fract() == 0.0 {
        // Bereich und Ganzzahligkeit sind geprüft.
        return Ok(PropertyKey::Index(index as u32));
    }
    let text: Vec<u16> = number::to_string(index).encode_utf16().collect();
    Ok(agent.key_from_units(scope, &text)?)
}

/// Set(O, "length", length, true).
fn set_length(agent: &mut Agent, scope: &Scope, object: Local<'_>, length: f64) -> Completion<()> {
    let key = agent.key_from_units(scope, &units("length"))?;
    let value = number(agent, scope, length);
    set(agent, scope, object, key, value, true)
}

/// Das Argument `index`, das eine Funktion sein muss.
fn callable<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    index: usize,
) -> Completion<Local<'s>> {
    let function = arguments.argument(agent, scope, index);
    if !is_callable(agent.heap(), agent.heap().get(function)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(function)
}

/// Ein relativer Index wie bei `slice`: negative zählen vom Ende, das
/// Ergebnis liegt zwischen 0 und `length`.
fn clamp_relative(relative: f64, length: f64) -> f64 {
    if relative < 0.0 {
        (length + relative).max(0.0)
    } else {
        relative.min(length)
    }
}

/// ToIntegerOrInfinity des Arguments als relativer Index; *undefined*
/// ergibt `default`.
fn relative_index(
    agent: &mut Agent,
    scope: &Scope,
    value: Local<'_>,
    length: f64,
    default: f64,
) -> Completion<f64> {
    let relative = if agent.heap().get(value).is_undefined() {
        default
    } else {
        to_integer_or_infinity(agent, scope, value)?
    };
    Ok(clamp_relative(relative, length))
}

/// Die Zahl der Argumente als Länge.
fn count_of(count: usize) -> f64 {
    u32::try_from(count).map_or(f64::INFINITY, f64::from)
}

/// ArrayCreate (Abschnitt 10.4.2.2) mit %Array.prototype% aus `realm`; eine
/// Länge über 2³² − 1 ist ein RangeError.
fn array_create_in<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    length: f64,
) -> Completion<Local<'s>> {
    if !(0.0..=f64::from(u32::MAX)).contains(&length) {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    // Bereich geprüft; Längen sind ganz.
    array_create(agent, scope, length as u64, Some(prototype))
}

/// ArraySpeciesCreate (Abschnitt 10.4.2.3).
fn array_species_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    original: Local<'_>,
    length: f64,
) -> Completion<Local<'s>> {
    if !is_array(agent.heap(), agent.heap().get(original)) {
        return array_create_in(agent, scope, realm, length);
    }
    let key = agent.key_from_units(scope, &units("constructor"))?;
    let mut constructor = get(agent, scope, original, key)?;
    if is_constructor(agent, agent.heap().get(constructor)) {
        let other = get_function_realm(agent.heap(), agent.heap().get(constructor), realm);
        if other != realm {
            // %Array% eines anderen Realms zählt wie der eigene.
            let foreign = agent.intrinsic(scope, other, Intrinsic::Array);
            let heap = agent.heap();
            if same_value(heap, heap.get(constructor), heap.get(foreign)) {
                constructor = undefined(agent, scope);
            }
        }
    }
    if is_object(agent, constructor) {
        let species = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::Species));
        constructor = get(agent, scope, constructor, species)?;
        if agent.heap().get(constructor).is_null() {
            constructor = undefined(agent, scope);
        }
    }
    if agent.heap().get(constructor).is_undefined() {
        return array_create_in(agent, scope, realm, length);
    }
    if !is_constructor(agent, agent.heap().get(constructor)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let length = number(agent, scope, length);
    construct(agent, scope, realm, constructor, &[length], None)
}

/// IsConcatSpreadable (Abschnitt 23.1.3.2.1).
fn is_concat_spreadable(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<bool> {
    if !is_object(agent, value) {
        return Ok(false);
    }
    let key =
        PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::IsConcatSpreadable));
    let spreadable = get(agent, scope, value, key)?;
    let heap = agent.heap();
    let found = heap.get(spreadable);
    if !found.is_undefined() {
        return Ok(to_boolean(heap, found));
    }
    Ok(is_array(heap, heap.get(value)))
}

/// Die Codeeinheiten einer Zeichenkette.
fn units_of(agent: &Agent, value: Local<'_>) -> Vec<u16> {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => Vec::new(),
    }
}

/// Hängt `units` an eine wachsende Zeichenkette an, solange sie ins
/// Speicherbudget passt.
fn append(agent: &Agent, text: &mut Vec<u16>, units: &[u16]) -> Completion<()> {
    let heap = agent.heap();
    let bytes = text
        .len()
        .saturating_add(units.len())
        .saturating_mul(2)
        .saturating_add(heap.used_bytes());
    if bytes > heap.limit() {
        return Err(OutOfMemory.into());
    }
    text.extend_from_slice(units);
    Ok(())
}

/// Die Schleife von `every`, `some`, `forEach`, `map` und `filter`: ruft
/// `callback` mit `this_arg` für jedes vorhandene Element mit Element,
/// Index und Objekt auf und gibt `visit` das Element, den Index und das
/// Ergebnis; `visit` sagt, ob die Schleife weitergeht.
fn each_present(
    agent: &mut Agent,
    object: Local<'_>,
    length: f64,
    callback: Local<'_>,
    this_arg: Local<'_>,
    mut visit: impl FnMut(&mut Agent, &Scope, Local<'_>, f64, Local<'_>) -> Completion<bool>,
) -> Completion<()> {
    let mut k = 0.0;
    while k < length {
        step(agent)?;
        let go_on = agent.scope(|agent, inner| -> Completion<bool> {
            let key = index_key(agent, inner, k)?;
            if !has_property(agent, object, key)? {
                return Ok(true);
            }
            let value = get(agent, inner, object, key)?;
            let index = number(agent, inner, k);
            let result = call(agent, inner, callback, this_arg, &[value, index, object])?;
            visit(agent, inner, value, k, result)
        })?;
        if !go_on {
            break;
        }
        k += 1.0;
    }
    Ok(())
}

// Der Konstruktor

/// Array ( ...values ) (Abschnitt 23.1.1.1).
fn array<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let new_target = arguments.new_target.unwrap_or(arguments.function);
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::ArrayPrototype,
        arguments.realm,
    )?;
    match arguments.arguments {
        [] => array_create(agent, scope, 0, Some(prototype)),
        &[length] => {
            let array = array_create(agent, scope, 0, Some(prototype))?;
            let int_len = if let Kind::Number(number) = agent.heap().get(length).kind() {
                let int_len = to_uint32(agent, scope, length)?;
                // SameValueZero(intLen, len): NaN und Brüche scheitern,
                // -0 gilt als 0.
                if f64::from(int_len) != number {
                    return Err(agent.throw_error(ErrorKind::RangeError));
                }
                f64::from(int_len)
            } else {
                create_data_property_or_throw(agent, scope, array, PropertyKey::Index(0), length)?;
                1.0
            };
            set_length(agent, scope, array, int_len)?;
            Ok(array)
        }
        items => {
            let count = u64::try_from(items.len()).map_err(|_| OutOfMemory)?;
            let array = array_create(agent, scope, count, Some(prototype))?;
            for (index, &item) in items.iter().enumerate() {
                let key = index_key(agent, scope, count_of(index))?;
                create_data_property_or_throw(agent, scope, array, key, item)?;
            }
            Ok(array)
        }
    }
}

/// Array.from ( items [ , mapper [ , thisArg ] ] ) (Abschnitt 23.1.2.1).
fn array_from<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let constructor = arguments.this;
    let items = arguments.argument(agent, scope, 0);
    let mapper = arguments.argument(agent, scope, 1);
    let this_arg = arguments.argument(agent, scope, 2);
    let mapping = !agent.heap().get(mapper).is_undefined();
    if mapping && !is_callable(agent.heap(), agent.heap().get(mapper)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::Iterator);
    let using_iterator =
        get_method_of_value(agent, scope, realm, items, PropertyKey::Symbol(symbol))?;
    if let Some(method) = using_iterator {
        let array = if is_constructor(agent, agent.heap().get(constructor)) {
            construct(agent, scope, realm, constructor, &[], None)?
        } else {
            array_create_in(agent, scope, realm, 0.0)?
        };
        let (iterator, next) = get_iterator_from_method(agent, scope, items, method)?;
        let mut k = 0.0;
        loop {
            step(agent)?;
            if k >= MAX_SAFE_INTEGER {
                let error = agent.throw_error(ErrorKind::TypeError);
                return close_if_abrupt(agent, scope, iterator, Err(error));
            }
            let done = agent.scope(|agent, inner| -> Completion<bool> {
                let Some(value) = iterator_step_value(agent, inner, iterator, next)? else {
                    return Ok(true);
                };
                let mapped = if mapping {
                    let index = number(agent, inner, k);
                    let result = call(agent, inner, mapper, this_arg, &[value, index]);
                    close_if_abrupt(agent, inner, iterator, result)?
                } else {
                    value
                };
                let key = index_key(agent, inner, k)?;
                let defined = create_data_property_or_throw(agent, inner, array, key, mapped);
                close_if_abrupt(agent, inner, iterator, defined)?;
                Ok(false)
            })?;
            if done {
                break;
            }
            k += 1.0;
        }
        set_length(agent, scope, array, k)?;
        return Ok(array);
    }
    let array_like = to_object(agent, scope, realm, items)?;
    let length = length_of_array_like(agent, scope, array_like)?;
    let array = if is_constructor(agent, agent.heap().get(constructor)) {
        let length = number(agent, scope, length);
        construct(agent, scope, realm, constructor, &[length], None)?
    } else {
        array_create_in(agent, scope, realm, length)?
    };
    let mut k = 0.0;
    while k < length {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let key = index_key(agent, inner, k)?;
            let value = get(agent, inner, array_like, key)?;
            let mapped = if mapping {
                let index = number(agent, inner, k);
                call(agent, inner, mapper, this_arg, &[value, index])?
            } else {
                value
            };
            create_data_property_or_throw(agent, inner, array, key, mapped)
        })?;
        k += 1.0;
    }
    set_length(agent, scope, array, length)?;
    Ok(array)
}

/// Array.isArray ( arg ) (Abschnitt 23.1.2.3).
fn array_is_array<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let found = is_array(agent.heap(), agent.heap().get(value));
    Ok(boolean(agent, scope, found))
}

/// Array.of ( ...items ) (Abschnitt 23.1.2.4).
fn array_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let constructor = arguments.this;
    let length = count_of(arguments.arguments.len());
    let array = if is_constructor(agent, agent.heap().get(constructor)) {
        let length = number(agent, scope, length);
        construct(agent, scope, realm, constructor, &[length], None)?
    } else {
        array_create_in(agent, scope, realm, length)?
    };
    for (index, &item) in arguments.arguments.iter().enumerate() {
        step(agent)?;
        let key = index_key(agent, scope, count_of(index))?;
        create_data_property_or_throw(agent, scope, array, key, item)?;
    }
    set_length(agent, scope, array, length)?;
    Ok(array)
}

/// get Array [ %Symbol.species% ] (Abschnitt 23.1.2.5): *this*.
fn species<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    Ok(heap.local(scope, heap.get(arguments.this)))
}

// Die Iteratoren

/// CreateArrayIterator (Abschnitt 23.1.5.1) für ToObject(*this*).
fn array_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    kind: i32,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
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

// Lesende Methoden

/// Array.prototype.at ( index ) (Abschnitt 23.1.3.1).
fn array_prototype_at<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let index = arguments.argument(agent, scope, 0);
    let relative = to_integer_or_infinity(agent, scope, index)?;
    let k = if relative >= 0.0 {
        relative
    } else {
        length + relative
    };
    if k < 0.0 || k >= length {
        return Ok(undefined(agent, scope));
    }
    let key = index_key(agent, scope, k)?;
    get(agent, scope, object, key)
}

/// Array.prototype.concat ( ...items ) (Abschnitt 23.1.3.2).
fn array_prototype_concat<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let object = this_object(agent, scope, arguments)?;
    let array = array_species_create(agent, scope, realm, object, 0.0)?;
    let mut n = 0.0;
    concat_item(agent, scope, array, object, &mut n)?;
    for &item in arguments.arguments {
        concat_item(agent, scope, array, item, &mut n)?;
    }
    set_length(agent, scope, array, n)?;
    Ok(array)
}

/// Ein Element von `concat`: ausgebreitet, wenn IsConcatSpreadable es
/// sagt, sonst als Ganzes.
fn concat_item(
    agent: &mut Agent,
    scope: &Scope,
    array: Local<'_>,
    item: Local<'_>,
    n: &mut f64,
) -> Completion<()> {
    if !is_concat_spreadable(agent, scope, item)? {
        if *n >= MAX_SAFE_INTEGER {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        let key = index_key(agent, scope, *n)?;
        create_data_property_or_throw(agent, scope, array, key, item)?;
        *n += 1.0;
        return Ok(());
    }
    let length = length_of_array_like(agent, scope, item)?;
    if *n + length > MAX_SAFE_INTEGER {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let mut k = 0.0;
    while k < length {
        step(agent)?;
        let target = *n;
        agent.scope(|agent, inner| -> Completion<()> {
            let key = index_key(agent, inner, k)?;
            if has_property(agent, item, key)? {
                let value = get(agent, inner, item, key)?;
                let key = index_key(agent, inner, target)?;
                create_data_property_or_throw(agent, inner, array, key, value)?;
            }
            Ok(())
        })?;
        *n += 1.0;
        k += 1.0;
    }
    Ok(())
}

/// Array.prototype.every ( callback [ , thisArg ] ) (Abschnitt 23.1.3.6).
fn array_prototype_every<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let callback = callable(agent, scope, arguments, 0)?;
    let this_arg = arguments.argument(agent, scope, 1);
    let mut all = true;
    each_present(
        agent,
        object,
        length,
        callback,
        this_arg,
        |agent, _, _, _, result| {
            all = to_boolean(agent.heap(), agent.heap().get(result));
            Ok(all)
        },
    )?;
    Ok(boolean(agent, scope, all))
}

/// Array.prototype.filter ( callback [ , thisArg ] ) (Abschnitt 23.1.3.9).
fn array_prototype_filter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let callback = callable(agent, scope, arguments, 0)?;
    let this_arg = arguments.argument(agent, scope, 1);
    let array = array_species_create(agent, scope, arguments.realm, object, 0.0)?;
    let mut to = 0.0;
    each_present(
        agent,
        object,
        length,
        callback,
        this_arg,
        |agent, inner, value, _, result| {
            if to_boolean(agent.heap(), agent.heap().get(result)) {
                let key = index_key(agent, inner, to)?;
                create_data_property_or_throw(agent, inner, array, key, value)?;
                to += 1.0;
            }
            Ok(true)
        },
    )?;
    Ok(array)
}

/// FindViaPredicate (Abschnitt 23.1.3.12.1) für `find`, `findIndex`,
/// `findLast` und `findLastIndex`: der Index und der Wert des ersten
/// Treffers, sonst −1 und *undefined*.
fn find_via_predicate<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    from_end: bool,
) -> Completion<(f64, Local<'s>)> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let predicate = callable(agent, scope, arguments, 0)?;
    let this_arg = arguments.argument(agent, scope, 1);
    let found = undefined(agent, scope);
    let mut k = if from_end { length - 1.0 } else { 0.0 };
    while if from_end { k >= 0.0 } else { k < length } {
        step(agent)?;
        let hit = agent.scope(|agent, inner| -> Completion<bool> {
            let key = index_key(agent, inner, k)?;
            let value = get(agent, inner, object, key)?;
            let index = number(agent, inner, k);
            let result = call(agent, inner, predicate, this_arg, &[value, index, object])?;
            let heap = agent.heap();
            if to_boolean(heap, heap.get(result)) {
                heap.set(found, heap.get(value));
                return Ok(true);
            }
            Ok(false)
        })?;
        if hit {
            return Ok((k, found));
        }
        k = if from_end { k - 1.0 } else { k + 1.0 };
    }
    Ok((-1.0, found))
}

/// Array.prototype.find ( predicate [ , thisArg ] ) (Abschnitt 23.1.3.10).
fn array_prototype_find<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(find_via_predicate(agent, scope, arguments, false)?.1)
}

/// Array.prototype.findIndex ( predicate [ , thisArg ] ) (Abschnitt
/// 23.1.3.11).
fn array_prototype_find_index<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (index, _) = find_via_predicate(agent, scope, arguments, false)?;
    Ok(number(agent, scope, index))
}

/// Array.prototype.findLast ( predicate [ , thisArg ] ) (Abschnitt
/// 23.1.3.12).
fn array_prototype_find_last<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(find_via_predicate(agent, scope, arguments, true)?.1)
}

/// Array.prototype.findLastIndex ( predicate [ , thisArg ] ) (Abschnitt
/// 23.1.3.13).
fn array_prototype_find_last_index<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (index, _) = find_via_predicate(agent, scope, arguments, true)?;
    Ok(number(agent, scope, index))
}

/// Was FlattenIntoArray mit einem Element getan hat.
enum Flatten {
    /// Es fehlt.
    Skipped,
    /// Es steht nun im Ziel.
    Written,
    /// Es ist selbst ein Array mit dieser Länge und wird ausgebreitet.
    Descend(f64),
}

/// Ein Array, das FlattenIntoArray gerade ausbreitet.
struct Source<'s> {
    array: Local<'s>,
    length: f64,
    index: f64,
    depth: f64,
}

/// FlattenIntoArray (Abschnitt 23.1.3.14.1) ohne Rekursion: Ein Stapel hält
/// die Arrays, die gerade ausgebreitet werden. `mapper` mit seinem `this`
/// gilt nur für die Elemente von `source` selbst, wie bei `flatMap`. Das
/// Ziel ist leer, `start` also 0; die Funktion gibt den nächsten freien
/// Index des Ziels zurück.
fn flatten_into_array(
    agent: &mut Agent,
    scope: &Scope,
    target: Local<'_>,
    source: Local<'_>,
    source_length: f64,
    depth: f64,
    mapper: Option<(Local<'_>, Local<'_>)>,
) -> Completion<f64> {
    let mut target_index = 0.0;
    let root = agent.heap().local(scope, agent.heap().get(source));
    let mut stack = vec![Source {
        array: root,
        length: source_length,
        index: 0.0,
        depth,
    }];
    while let Some(top) = stack.last_mut() {
        if top.index >= top.length {
            stack.pop();
            continue;
        }
        step(agent)?;
        let (array, source_index, depth) = (top.array, top.index, top.depth);
        top.index += 1.0;
        let is_root = stack.len() == 1;
        // Ein Array zum Ausbreiten überdauert den Durchlauf in diesem Handle.
        let next = undefined(agent, scope);
        let outcome = agent.scope(|agent, inner| -> Completion<Flatten> {
            let key = index_key(agent, inner, source_index)?;
            if !has_property(agent, array, key)? {
                return Ok(Flatten::Skipped);
            }
            let mut element = get(agent, inner, array, key)?;
            if let (true, Some((mapper, this_arg))) = (is_root, mapper) {
                let index = number(agent, inner, source_index);
                element = call(agent, inner, mapper, this_arg, &[element, index, array])?;
            }
            if depth > 0.0 && is_array(agent.heap(), agent.heap().get(element)) {
                let heap = agent.heap();
                heap.set(next, heap.get(element));
                let length = length_of_array_like(agent, inner, element)?;
                return Ok(Flatten::Descend(length));
            }
            if target_index >= MAX_SAFE_INTEGER {
                return Err(agent.throw_error(ErrorKind::TypeError));
            }
            let key = index_key(agent, inner, target_index)?;
            create_data_property_or_throw(agent, inner, target, key, element)?;
            Ok(Flatten::Written)
        })?;
        match outcome {
            Flatten::Skipped => {}
            Flatten::Written => target_index += 1.0,
            Flatten::Descend(length) => stack.push(Source {
                array: next,
                length,
                index: 0.0,
                depth: if depth == f64::INFINITY {
                    depth
                } else {
                    depth - 1.0
                },
            }),
        }
    }
    Ok(target_index)
}

/// Array.prototype.flat ( [ depth ] ) (Abschnitt 23.1.3.14).
fn array_prototype_flat<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let depth = arguments.argument(agent, scope, 0);
    let depth = if agent.heap().get(depth).is_undefined() {
        1.0
    } else {
        to_integer_or_infinity(agent, scope, depth)?.max(0.0)
    };
    let array = array_species_create(agent, scope, arguments.realm, object, 0.0)?;
    flatten_into_array(agent, scope, array, object, length, depth, None)?;
    Ok(array)
}

/// Array.prototype.flatMap ( mapper [ , thisArg ] ) (Abschnitt 23.1.3.15).
fn array_prototype_flat_map<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let mapper = callable(agent, scope, arguments, 0)?;
    let this_arg = arguments.argument(agent, scope, 1);
    let array = array_species_create(agent, scope, arguments.realm, object, 0.0)?;
    flatten_into_array(
        agent,
        scope,
        array,
        object,
        length,
        1.0,
        Some((mapper, this_arg)),
    )?;
    Ok(array)
}

/// Array.prototype.forEach ( callback [ , thisArg ] ) (Abschnitt 23.1.3.16).
fn array_prototype_for_each<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let callback = callable(agent, scope, arguments, 0)?;
    let this_arg = arguments.argument(agent, scope, 1);
    each_present(
        agent,
        object,
        length,
        callback,
        this_arg,
        |_, _, _, _, _| Ok(true),
    )?;
    Ok(undefined(agent, scope))
}

/// Der Anfang der Suche von `indexOf` und `includes` aus `fromIndex`:
/// `None`, wenn nichts zu durchsuchen ist.
fn search_start(
    agent: &mut Agent,
    scope: &Scope,
    from_index: Local<'_>,
    length: f64,
) -> Completion<Option<f64>> {
    let n = to_integer_or_infinity(agent, scope, from_index)?;
    if n == f64::INFINITY {
        return Ok(None);
    }
    let n = if n == f64::NEG_INFINITY { 0.0 } else { n };
    Ok(Some(if n >= 0.0 { n } else { (length + n).max(0.0) }))
}

/// Array.prototype.includes ( searchElement [ , fromIndex ] ) (Abschnitt
/// 23.1.3.17).
fn array_prototype_includes<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    if length == 0.0 {
        return Ok(boolean(agent, scope, false));
    }
    let search = arguments.argument(agent, scope, 0);
    let from_index = arguments.argument(agent, scope, 1);
    let Some(mut k) = search_start(agent, scope, from_index, length)? else {
        return Ok(boolean(agent, scope, false));
    };
    while k < length {
        step(agent)?;
        let found = agent.scope(|agent, inner| -> Completion<bool> {
            let key = index_key(agent, inner, k)?;
            let element = get(agent, inner, object, key)?;
            let heap = agent.heap();
            Ok(same_value_zero(heap, heap.get(search), heap.get(element)))
        })?;
        if found {
            return Ok(boolean(agent, scope, true));
        }
        k += 1.0;
    }
    Ok(boolean(agent, scope, false))
}

/// Array.prototype.indexOf ( searchElement [ , fromIndex ] ) (Abschnitt
/// 23.1.3.18).
fn array_prototype_index_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    if length == 0.0 {
        return Ok(number(agent, scope, -1.0));
    }
    let search = arguments.argument(agent, scope, 0);
    let from_index = arguments.argument(agent, scope, 1);
    let Some(mut k) = search_start(agent, scope, from_index, length)? else {
        return Ok(number(agent, scope, -1.0));
    };
    while k < length {
        step(agent)?;
        let found = agent.scope(|agent, inner| -> Completion<bool> {
            let key = index_key(agent, inner, k)?;
            if !has_property(agent, object, key)? {
                return Ok(false);
            }
            let element = get(agent, inner, object, key)?;
            let heap = agent.heap();
            Ok(is_strictly_equal(heap, heap.get(search), heap.get(element)))
        })?;
        if found {
            return Ok(number(agent, scope, k));
        }
        k += 1.0;
    }
    Ok(number(agent, scope, -1.0))
}

/// Array.prototype.lastIndexOf ( searchElement [ , fromIndex ] ) (Abschnitt
/// 23.1.3.20).
fn array_prototype_last_index_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    if length == 0.0 {
        return Ok(number(agent, scope, -1.0));
    }
    let search = arguments.argument(agent, scope, 0);
    let n = if arguments.arguments.len() > 1 {
        let from_index = arguments.argument(agent, scope, 1);
        to_integer_or_infinity(agent, scope, from_index)?
    } else {
        length - 1.0
    };
    if n == f64::NEG_INFINITY {
        return Ok(number(agent, scope, -1.0));
    }
    let mut k = if n >= 0.0 {
        n.min(length - 1.0)
    } else {
        length + n
    };
    while k >= 0.0 {
        step(agent)?;
        let found = agent.scope(|agent, inner| -> Completion<bool> {
            let key = index_key(agent, inner, k)?;
            if !has_property(agent, object, key)? {
                return Ok(false);
            }
            let element = get(agent, inner, object, key)?;
            let heap = agent.heap();
            Ok(is_strictly_equal(heap, heap.get(search), heap.get(element)))
        })?;
        if found {
            return Ok(number(agent, scope, k));
        }
        k -= 1.0;
    }
    Ok(number(agent, scope, -1.0))
}

/// Array.prototype.join ( separator ) (Abschnitt 23.1.3.18).
fn array_prototype_join<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let separator = arguments.argument(agent, scope, 0);
    let separator = if agent.heap().get(separator).is_undefined() {
        units(",")
    } else {
        let string = to_string(agent, scope, separator)?;
        units_of(agent, string)
    };
    let mut text = Vec::new();
    let mut k = 0.0;
    while k < length {
        step(agent)?;
        if k > 0.0 {
            append(agent, &mut text, &separator)?;
        }
        agent.scope(|agent, inner| -> Completion<()> {
            let key = index_key(agent, inner, k)?;
            let element = get(agent, inner, object, key)?;
            let found = agent.heap().get(element);
            if found.is_undefined() || found.is_null() {
                return Ok(());
            }
            let string = to_string(agent, inner, element)?;
            append(agent, &mut text, &units_of(agent, string))
        })?;
        k += 1.0;
    }
    Ok(agent.heap_mut().alloc_string(scope, &text)?)
}

/// Array.prototype.map ( callback [ , thisArg ] ) (Abschnitt 23.1.3.21).
fn array_prototype_map<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let callback = callable(agent, scope, arguments, 0)?;
    let this_arg = arguments.argument(agent, scope, 1);
    let array = array_species_create(agent, scope, arguments.realm, object, length)?;
    each_present(
        agent,
        object,
        length,
        callback,
        this_arg,
        |agent, inner, _, k, result| {
            let key = index_key(agent, inner, k)?;
            create_data_property_or_throw(agent, inner, array, key, result)?;
            Ok(true)
        },
    )?;
    Ok(array)
}

/// Array.prototype.pop ( ) (Abschnitt 23.1.3.22).
fn array_prototype_pop<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    if length == 0.0 {
        set_length(agent, scope, object, 0.0)?;
        return Ok(undefined(agent, scope));
    }
    let new_length = length - 1.0;
    let key = index_key(agent, scope, new_length)?;
    let element = get(agent, scope, object, key)?;
    delete_property_or_throw(agent, scope, object, key)?;
    set_length(agent, scope, object, new_length)?;
    Ok(element)
}

/// Array.prototype.push ( ...items ) (Abschnitt 23.1.3.23).
fn array_prototype_push<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let mut length = length_of_array_like(agent, scope, object)?;
    let count = count_of(arguments.arguments.len());
    if length + count > MAX_SAFE_INTEGER {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    for &item in arguments.arguments {
        step(agent)?;
        let key = index_key(agent, scope, length)?;
        set(agent, scope, object, key, item, true)?;
        length += 1.0;
    }
    set_length(agent, scope, object, length)?;
    Ok(number(agent, scope, length))
}

/// Array.prototype.reduce und reduceRight (Abschnitte 23.1.3.24 und
/// 23.1.3.25): `from_end` wählt die Richtung.
fn reduce<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    from_end: bool,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let callback = callable(agent, scope, arguments, 0)?;
    let has_initial = arguments.arguments.len() > 1;
    if length == 0.0 && !has_initial {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let in_range = |k: f64| if from_end { k >= 0.0 } else { k < length };
    let advance = |k: f64| if from_end { k - 1.0 } else { k + 1.0 };
    let accumulator = undefined(agent, scope);
    let mut k = if from_end { length - 1.0 } else { 0.0 };
    if has_initial {
        let initial = arguments.argument(agent, scope, 1);
        let heap = agent.heap();
        heap.set(accumulator, heap.get(initial));
    } else {
        let mut present = false;
        while !present && in_range(k) {
            step(agent)?;
            present = agent.scope(|agent, inner| -> Completion<bool> {
                let key = index_key(agent, inner, k)?;
                if !has_property(agent, object, key)? {
                    return Ok(false);
                }
                let value = get(agent, inner, object, key)?;
                let heap = agent.heap();
                heap.set(accumulator, heap.get(value));
                Ok(true)
            })?;
            k = advance(k);
        }
        if !present {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
    }
    while in_range(k) {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let key = index_key(agent, inner, k)?;
            if !has_property(agent, object, key)? {
                return Ok(());
            }
            let value = get(agent, inner, object, key)?;
            let index = number(agent, inner, k);
            let this = undefined(agent, inner);
            let result = call(
                agent,
                inner,
                callback,
                this,
                &[accumulator, value, index, object],
            )?;
            let heap = agent.heap();
            heap.set(accumulator, heap.get(result));
            Ok(())
        })?;
        k = advance(k);
    }
    Ok(accumulator)
}

/// Array.prototype.reduce ( callback [ , initialValue ] ) (Abschnitt
/// 23.1.3.24).
fn array_prototype_reduce<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    reduce(agent, scope, arguments, false)
}

/// Array.prototype.reduceRight ( callback [ , initialValue ] ) (Abschnitt
/// 23.1.3.25).
fn array_prototype_reduce_right<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    reduce(agent, scope, arguments, true)
}

/// Array.prototype.some ( callback [ , thisArg ] ) (Abschnitt 23.1.3.29).
fn array_prototype_some<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let callback = callable(agent, scope, arguments, 0)?;
    let this_arg = arguments.argument(agent, scope, 1);
    let mut any = false;
    each_present(
        agent,
        object,
        length,
        callback,
        this_arg,
        |agent, _, _, _, result| {
            any = to_boolean(agent.heap(), agent.heap().get(result));
            Ok(!any)
        },
    )?;
    Ok(boolean(agent, scope, any))
}

/// Array.prototype.toLocaleString ( [ reserved1 [ , reserved2 ] ] )
/// (Abschnitt 23.1.3.32, ohne ECMA-402): Jedes Element außer *undefined*
/// und *null* liefert seine `toLocaleString`, das Trennzeichen ist das
/// Komma.
fn array_prototype_to_locale_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let separator = units(",");
    let mut text = Vec::new();
    let mut k = 0.0;
    while k < length {
        step(agent)?;
        if k > 0.0 {
            append(agent, &mut text, &separator)?;
        }
        agent.scope(|agent, inner| -> Completion<()> {
            let key = index_key(agent, inner, k)?;
            let element = get(agent, inner, object, key)?;
            let found = agent.heap().get(element);
            if found.is_undefined() || found.is_null() {
                return Ok(());
            }
            let key = agent.key_from_units(inner, &units("toLocaleString"))?;
            let result = invoke(agent, inner, realm, element, key, &[])?;
            let string = to_string(agent, inner, result)?;
            append(agent, &mut text, &units_of(agent, string))
        })?;
        k += 1.0;
    }
    Ok(agent.heap_mut().alloc_string(scope, &text)?)
}

/// Array.prototype.toString ( ) (Abschnitt 23.1.3.36): `join`, wenn das
/// eine Funktion ist, sonst %Object.prototype.toString%.
fn array_prototype_to_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let array = this_object(agent, scope, arguments)?;
    let key = agent.key_from_units(scope, &units("join"))?;
    let join = get(agent, scope, array, key)?;
    if is_callable(agent.heap(), agent.heap().get(join)) {
        return call(agent, scope, join, array, &[]);
    }
    let heap = agent.heap();
    let function = heap.local(scope, heap.get(arguments.function));
    let fallback = BuiltinCall {
        function,
        realm: arguments.realm,
        this: array,
        arguments: &[],
        new_target: None,
    };
    object_prototype_to_string(agent, scope, &fallback)
}

// Methoden, die ihr Objekt ändern

/// Array.prototype.copyWithin ( target, start [ , end ] ) (Abschnitt
/// 23.1.3.4).
fn array_prototype_copy_within<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let target = arguments.argument(agent, scope, 0);
    let mut to = relative_index(agent, scope, target, length, 0.0)?;
    let start = arguments.argument(agent, scope, 1);
    let mut from = relative_index(agent, scope, start, length, 0.0)?;
    let end = arguments.argument(agent, scope, 2);
    let final_index = relative_index(agent, scope, end, length, length)?;
    let mut count = (final_index - from).min(length - to);
    let direction = if from < to && to < from + count {
        from += count - 1.0;
        to += count - 1.0;
        -1.0
    } else {
        1.0
    };
    while count > 0.0 {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let from_key = index_key(agent, inner, from)?;
            let to_key = index_key(agent, inner, to)?;
            if has_property(agent, object, from_key)? {
                let value = get(agent, inner, object, from_key)?;
                set(agent, inner, object, to_key, value, true)
            } else {
                delete_property_or_throw(agent, inner, object, to_key)
            }
        })?;
        from += direction;
        to += direction;
        count -= 1.0;
    }
    Ok(object)
}

/// Array.prototype.fill ( value [ , start [ , end ] ] ) (Abschnitt
/// 23.1.3.7).
fn array_prototype_fill<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let value = arguments.argument(agent, scope, 0);
    let start = arguments.argument(agent, scope, 1);
    let mut k = relative_index(agent, scope, start, length, 0.0)?;
    let end = arguments.argument(agent, scope, 2);
    let final_index = relative_index(agent, scope, end, length, length)?;
    while k < final_index {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let key = index_key(agent, inner, k)?;
            set(agent, inner, object, key, value, true)
        })?;
        k += 1.0;
    }
    Ok(object)
}

/// Array.prototype.reverse ( ) (Abschnitt 23.1.3.26).
fn array_prototype_reverse<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let middle = (length / 2.0).floor();
    let mut lower = 0.0;
    while lower != middle {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let upper = length - lower - 1.0;
            let lower_key = index_key(agent, inner, lower)?;
            let upper_key = index_key(agent, inner, upper)?;
            let lower_exists = has_property(agent, object, lower_key)?;
            let lower_value = if lower_exists {
                Some(get(agent, inner, object, lower_key)?)
            } else {
                None
            };
            let upper_exists = has_property(agent, object, upper_key)?;
            let upper_value = if upper_exists {
                Some(get(agent, inner, object, upper_key)?)
            } else {
                None
            };
            match (lower_value, upper_value) {
                (Some(lower_value), Some(upper_value)) => {
                    set(agent, inner, object, lower_key, upper_value, true)?;
                    set(agent, inner, object, upper_key, lower_value, true)
                }
                (None, Some(upper_value)) => {
                    set(agent, inner, object, lower_key, upper_value, true)?;
                    delete_property_or_throw(agent, inner, object, upper_key)
                }
                (Some(lower_value), None) => {
                    delete_property_or_throw(agent, inner, object, lower_key)?;
                    set(agent, inner, object, upper_key, lower_value, true)
                }
                (None, None) => Ok(()),
            }
        })?;
        lower += 1.0;
    }
    Ok(object)
}

/// Array.prototype.shift ( ) (Abschnitt 23.1.3.27).
fn array_prototype_shift<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    if length == 0.0 {
        set_length(agent, scope, object, 0.0)?;
        return Ok(undefined(agent, scope));
    }
    let first = get(agent, scope, object, PropertyKey::Index(0))?;
    let mut k = 1.0;
    while k < length {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let from = index_key(agent, inner, k)?;
            let to = index_key(agent, inner, k - 1.0)?;
            if has_property(agent, object, from)? {
                let value = get(agent, inner, object, from)?;
                set(agent, inner, object, to, value, true)
            } else {
                delete_property_or_throw(agent, inner, object, to)
            }
        })?;
        k += 1.0;
    }
    let key = index_key(agent, scope, length - 1.0)?;
    delete_property_or_throw(agent, scope, object, key)?;
    set_length(agent, scope, object, length - 1.0)?;
    Ok(first)
}

/// Array.prototype.sort ( comparator ) (Abschnitt 23.1.3.30).
fn array_prototype_sort<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let comparator = arguments.argument(agent, scope, 0);
    let found = agent.heap().get(comparator);
    if !found.is_undefined() && !is_callable(agent.heap(), found) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let items = sort_indexed_properties(agent, scope, object, length, comparator, Holes::Skip)?;
    let mut j = 0.0;
    for &item in &items {
        step(agent)?;
        let key = index_key(agent, scope, j)?;
        set(agent, scope, object, key, item, true)?;
        j += 1.0;
    }
    while j < length {
        step(agent)?;
        let key = index_key(agent, scope, j)?;
        delete_property_or_throw(agent, scope, object, key)?;
        j += 1.0;
    }
    Ok(object)
}

/// Array.prototype.splice ( start, deleteCount, ...items ) (Abschnitt
/// 23.1.3.31).
fn array_prototype_splice<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let start = arguments.argument(agent, scope, 0);
    let actual_start = relative_index(agent, scope, start, length, 0.0)?;
    let items = arguments.arguments.get(2..).unwrap_or_default();
    let item_count = count_of(items.len());
    let delete_count = match arguments.arguments.len() {
        0 => 0.0,
        1 => length - actual_start,
        _ => {
            let delete_count = arguments.argument(agent, scope, 1);
            let delete_count = to_integer_or_infinity(agent, scope, delete_count)?;
            delete_count.clamp(0.0, length - actual_start)
        }
    };
    if length + item_count - delete_count > MAX_SAFE_INTEGER {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let array = array_species_create(agent, scope, arguments.realm, object, delete_count)?;
    let mut k = 0.0;
    while k < delete_count {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let from = index_key(agent, inner, actual_start + k)?;
            if has_property(agent, object, from)? {
                let value = get(agent, inner, object, from)?;
                let to = index_key(agent, inner, k)?;
                create_data_property_or_throw(agent, inner, array, to, value)?;
            }
            Ok(())
        })?;
        k += 1.0;
    }
    set_length(agent, scope, array, delete_count)?;
    if item_count < delete_count {
        let mut k = actual_start;
        while k < length - delete_count {
            step(agent)?;
            move_element(agent, object, k + delete_count, k + item_count)?;
            k += 1.0;
        }
        let mut k = length;
        while k > length - delete_count + item_count {
            step(agent)?;
            let key = index_key(agent, scope, k - 1.0)?;
            delete_property_or_throw(agent, scope, object, key)?;
            k -= 1.0;
        }
    } else if item_count > delete_count {
        let mut k = length - delete_count;
        while k > actual_start {
            step(agent)?;
            move_element(agent, object, k + delete_count - 1.0, k + item_count - 1.0)?;
            k -= 1.0;
        }
    }
    let mut k = actual_start;
    for &item in items {
        step(agent)?;
        let key = index_key(agent, scope, k)?;
        set(agent, scope, object, key, item, true)?;
        k += 1.0;
    }
    set_length(agent, scope, object, length - delete_count + item_count)?;
    Ok(array)
}

/// Verschiebt das Element von `from` nach `to`: Fehlt es, fällt `to` weg.
/// So verschieben `splice` und `unshift` ihre Elemente.
fn move_element(agent: &mut Agent, object: Local<'_>, from: f64, to: f64) -> Completion<()> {
    agent.scope(|agent, inner| -> Completion<()> {
        let from = index_key(agent, inner, from)?;
        let to = index_key(agent, inner, to)?;
        if has_property(agent, object, from)? {
            let value = get(agent, inner, object, from)?;
            set(agent, inner, object, to, value, true)
        } else {
            delete_property_or_throw(agent, inner, object, to)
        }
    })
}

/// Array.prototype.unshift ( ...items ) (Abschnitt 23.1.3.37).
fn array_prototype_unshift<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let count = count_of(arguments.arguments.len());
    if count > 0.0 {
        if length + count > MAX_SAFE_INTEGER {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        let mut k = length;
        while k > 0.0 {
            step(agent)?;
            move_element(agent, object, k - 1.0, k + count - 1.0)?;
            k -= 1.0;
        }
        for (j, &item) in arguments.arguments.iter().enumerate() {
            step(agent)?;
            let key = index_key(agent, scope, count_of(j))?;
            set(agent, scope, object, key, item, true)?;
        }
    }
    set_length(agent, scope, object, length + count)?;
    Ok(number(agent, scope, length + count))
}

// Methoden, die ein neues Array liefern

/// Array.prototype.slice ( start, end ) (Abschnitt 23.1.3.28).
fn array_prototype_slice<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let start = arguments.argument(agent, scope, 0);
    let mut k = relative_index(agent, scope, start, length, 0.0)?;
    let end = arguments.argument(agent, scope, 1);
    let final_index = relative_index(agent, scope, end, length, length)?;
    let count = (final_index - k).max(0.0);
    let array = array_species_create(agent, scope, arguments.realm, object, count)?;
    let mut n = 0.0;
    while k < final_index {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let from = index_key(agent, inner, k)?;
            if has_property(agent, object, from)? {
                let value = get(agent, inner, object, from)?;
                let to = index_key(agent, inner, n)?;
                create_data_property_or_throw(agent, inner, array, to, value)?;
            }
            Ok(())
        })?;
        k += 1.0;
        n += 1.0;
    }
    set_length(agent, scope, array, n)?;
    Ok(array)
}

/// Array.prototype.toReversed ( ) (Abschnitt 23.1.3.33).
fn array_prototype_to_reversed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let array = array_create_in(agent, scope, arguments.realm, length)?;
    let mut k = 0.0;
    while k < length {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let from = index_key(agent, inner, length - k - 1.0)?;
            let value = get(agent, inner, object, from)?;
            let to = index_key(agent, inner, k)?;
            create_data_property_or_throw(agent, inner, array, to, value)
        })?;
        k += 1.0;
    }
    Ok(array)
}

/// Array.prototype.toSorted ( comparator ) (Abschnitt 23.1.3.34).
fn array_prototype_to_sorted<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let comparator = arguments.argument(agent, scope, 0);
    let found = agent.heap().get(comparator);
    if !found.is_undefined() && !is_callable(agent.heap(), found) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let array = array_create_in(agent, scope, arguments.realm, length)?;
    let items =
        sort_indexed_properties(agent, scope, object, length, comparator, Holes::ReadThrough)?;
    let mut j = 0.0;
    for &item in &items {
        step(agent)?;
        let key = index_key(agent, scope, j)?;
        create_data_property_or_throw(agent, scope, array, key, item)?;
        j += 1.0;
    }
    Ok(array)
}

/// Array.prototype.toSpliced ( start, skipCount, ...items ) (Abschnitt
/// 23.1.3.35).
fn array_prototype_to_spliced<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let start = arguments.argument(agent, scope, 0);
    let actual_start = relative_index(agent, scope, start, length, 0.0)?;
    let items = arguments.arguments.get(2..).unwrap_or_default();
    let insert_count = count_of(items.len());
    let skip_count = match arguments.arguments.len() {
        0 => 0.0,
        1 => length - actual_start,
        _ => {
            let skip_count = arguments.argument(agent, scope, 1);
            let skip_count = to_integer_or_infinity(agent, scope, skip_count)?;
            skip_count.clamp(0.0, length - actual_start)
        }
    };
    let new_length = length + insert_count - skip_count;
    if new_length > MAX_SAFE_INTEGER {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let array = array_create_in(agent, scope, arguments.realm, new_length)?;
    let mut i = 0.0;
    while i < actual_start {
        step(agent)?;
        copy_element(agent, object, array, i, i)?;
        i += 1.0;
    }
    for &item in items {
        step(agent)?;
        let key = index_key(agent, scope, i)?;
        create_data_property_or_throw(agent, scope, array, key, item)?;
        i += 1.0;
    }
    let mut r = actual_start + skip_count;
    while i < new_length {
        step(agent)?;
        copy_element(agent, object, array, r, i)?;
        i += 1.0;
        r += 1.0;
    }
    Ok(array)
}

/// Liest das Element `from` von `source` und legt es als `to` in `target`
/// an, wie `toSpliced` und `with` es tun.
fn copy_element(
    agent: &mut Agent,
    source: Local<'_>,
    target: Local<'_>,
    from: f64,
    to: f64,
) -> Completion<()> {
    agent.scope(|agent, inner| -> Completion<()> {
        let from = index_key(agent, inner, from)?;
        let value = get(agent, inner, source, from)?;
        let to = index_key(agent, inner, to)?;
        create_data_property_or_throw(agent, inner, target, to, value)
    })
}

/// Array.prototype.with ( index, value ) (Abschnitt 23.1.3.39).
fn array_prototype_with<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = this_object(agent, scope, arguments)?;
    let length = length_of_array_like(agent, scope, object)?;
    let index = arguments.argument(agent, scope, 0);
    let relative = to_integer_or_infinity(agent, scope, index)?;
    let actual = if relative >= 0.0 {
        relative
    } else {
        length + relative
    };
    if actual >= length || actual < 0.0 {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let value = arguments.argument(agent, scope, 1);
    let array = array_create_in(agent, scope, arguments.realm, length)?;
    let mut k = 0.0;
    while k < length {
        step(agent)?;
        if k == actual {
            let key = index_key(agent, scope, k)?;
            create_data_property_or_throw(agent, scope, array, key, value)?;
        } else {
            copy_element(agent, object, array, k, k)?;
        }
        k += 1.0;
    }
    Ok(array)
}
