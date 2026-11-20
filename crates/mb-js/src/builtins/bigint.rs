//! %BigInt% (Abschnitt 21.2.1) mit `asIntN` und `asUintN` (Abschnitt
//! 21.2.2) und %BigInt.prototype% (Abschnitt 21.2.3), ein gewöhnliches
//! Objekt.

use super::{define_functions, define_to_string_tag, link_constructor};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::bigint::{self, BigInt};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::{
    PreferredType, number_to_bigint, to_bigint, to_index, to_integer_or_infinity, to_primitive,
};
use crate::heap::{Local, Scope};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "BigInt",
    length: 1,
    constructor: true,
    behaviour: bigint_constructor,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 2] = [
    Builtin {
        name: "asIntN",
        length: 2,
        constructor: false,
        behaviour: as_int_n,
    },
    Builtin {
        name: "asUintN",
        length: 2,
        constructor: false,
        behaviour: as_uint_n,
    },
];

static PROTOTYPE_FUNCTIONS: [Builtin; 3] = [
    Builtin {
        name: "toLocaleString",
        length: 0,
        constructor: false,
        behaviour: to_locale_string,
    },
    Builtin {
        name: "toString",
        length: 0,
        constructor: false,
        behaviour: to_string,
    },
    Builtin {
        name: "valueOf",
        length: 0,
        constructor: false,
        behaviour: value_of,
    },
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::BigIntPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    define_to_string_tag(agent, scope, prototype, "BigInt")?;
    agent.set_intrinsic(realm, Intrinsic::BigInt, constructor);
    Ok(())
}

/// BigInt ( value ) (Abschnitt 21.2.1.1). Kein Konstruktor für `new`,
/// obwohl `prototype` daran hängt.
fn bigint_constructor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if arguments.new_target.is_some() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let value = arguments.argument(agent, scope, 0);
    let primitive = to_primitive(agent, scope, value, Some(PreferredType::Number))?;
    if let Some(number) = agent.heap().get(primitive).as_number() {
        return number_to_bigint(agent, scope, number);
    }
    to_bigint(agent, scope, primitive)
}

/// Die ersten beiden Schritte von asIntN und asUintN: ToIndex(bits) und
/// ToBigInt(bigint).
fn bits_and_value(
    agent: &mut Agent,
    scope: &Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<(u64, BigInt)> {
    let bits = arguments.argument(agent, scope, 0);
    let bits = to_index(agent, scope, bits)?;
    let value = arguments.argument(agent, scope, 1);
    let value = to_bigint(agent, scope, value)?;
    // Für 0 Bit ist das Ergebnis 0n; der Wert muss dafür nicht aus dem Heap.
    let value = if bits == 0 {
        BigInt::zero()
    } else {
        bigint::value_of(agent, value).unwrap_or_default()
    };
    Ok((bits, value))
}

/// BigInt.asIntN ( bits, bigint ) (Abschnitt 21.2.2.1).
fn as_int_n<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (bits, value) = bits_and_value(agent, scope, arguments)?;
    let result = bigint::compute(agent, |budget| value.as_int_n(bits, budget))?;
    bigint::allocate(agent, scope, &result)
}

/// BigInt.asUintN ( bits, bigint ) (Abschnitt 21.2.2.2). Für negative
/// Werte hat das Ergebnis `bits` Bit; über
/// [`MAX_BITS`](crate::bigint::MAX_BITS) ist das ein RangeError.
fn as_uint_n<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let (bits, value) = bits_and_value(agent, scope, arguments)?;
    let result = bigint::compute(agent, |budget| value.as_uint_n(bits, budget))?;
    bigint::allocate(agent, scope, &result)
}

/// ThisBigIntValue (Abschnitt 21.2.3.4.1).
fn this_bigint_value(agent: &mut Agent, value: Local<'_>) -> Completion<BigInt> {
    let heap = agent.heap();
    let found = match heap.get(value).kind() {
        Kind::BigInt(cell) => heap.bigint(cell),
        Kind::Object(object) => match heap.primitive_data(object).map(Value::kind) {
            Some(Kind::BigInt(cell)) => heap.bigint(cell),
            _ => None,
        },
        _ => None,
    };
    found.ok_or_else(|| agent.throw_error(ErrorKind::TypeError))
}

/// BigInt.prototype.toLocaleString ( [ reserved1 [ , reserved2 ] ] )
/// (Abschnitt 21.2.3.2). Ohne ECMA-402 und ohne Gebietsschema schreibt es
/// die Dezimalziffern wie `toString()`.
fn to_locale_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let x = this_bigint_value(agent, arguments.this)?;
    let units = bigint::to_units(agent, &x, 10)?;
    Ok(agent.heap_mut().alloc_string(scope, &units)?)
}

/// BigInt.prototype.toString ( [ radix ] ) (Abschnitt 21.2.3.3).
fn to_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let x = this_bigint_value(agent, arguments.this)?;
    let radix = arguments.argument(agent, scope, 0);
    let radix = if agent.heap().get(radix).is_undefined() {
        10.0
    } else {
        to_integer_or_infinity(agent, scope, radix)?
    };
    if !(2.0..=36.0).contains(&radix) {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    // Ganz und zwischen 2 und 36.
    let units = bigint::to_units(agent, &x, radix as u32)?;
    Ok(agent.heap_mut().alloc_string(scope, &units)?)
}

/// BigInt.prototype.valueOf ( ) (Abschnitt 21.2.3.4).
fn value_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    // ThisBigIntValue, ohne den Wert zu kopieren.
    let heap = agent.heap();
    let value = heap.get(arguments.this);
    let primitive = match value.kind() {
        Kind::BigInt(_) => Some(value),
        Kind::Object(object) => heap
            .primitive_data(object)
            .filter(|data| matches!(data.kind(), Kind::BigInt(_))),
        _ => None,
    };
    match primitive {
        Some(primitive) => Ok(heap.local(scope, primitive)),
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}
