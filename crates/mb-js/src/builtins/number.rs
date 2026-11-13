//! %Number% (Abschnitt 21.1.1) mit seinen Eigenschaften (Abschnitt 21.1.2)
//! und %Number.prototype% (Abschnitt 21.1.3), selbst ein Number-Objekt mit
//! +0. Die Zahlformate rechnet `crate::number`.

use super::{define_functions, link_constructor, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::{to_integer_or_infinity, to_numeric};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::number;
use crate::object::{get_prototype_from_constructor, primitive_object_create};
use crate::operations::define_property_or_throw;
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "Number",
    length: 1,
    constructor: true,
    behaviour: number_constructor,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 4] = [
    Builtin {
        name: "isFinite",
        length: 1,
        constructor: false,
        behaviour: is_finite,
    },
    Builtin {
        name: "isInteger",
        length: 1,
        constructor: false,
        behaviour: is_integer,
    },
    Builtin {
        name: "isNaN",
        length: 1,
        constructor: false,
        behaviour: is_nan,
    },
    Builtin {
        name: "isSafeInteger",
        length: 1,
        constructor: false,
        behaviour: is_safe_integer,
    },
];

static PROTOTYPE_FUNCTIONS: [Builtin; 6] = [
    Builtin {
        name: "toExponential",
        length: 1,
        constructor: false,
        behaviour: to_exponential,
    },
    Builtin {
        name: "toFixed",
        length: 1,
        constructor: false,
        behaviour: to_fixed,
    },
    Builtin {
        name: "toLocaleString",
        length: 0,
        constructor: false,
        behaviour: to_locale_string,
    },
    Builtin {
        name: "toPrecision",
        length: 1,
        constructor: false,
        behaviour: to_precision,
    },
    Builtin {
        name: "toString",
        length: 1,
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

/// 2⁵³ − 1, die größte ganze Zahl, deren Nachbarn noch Zahlen sind.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Die Werteigenschaften von %Number% (Abschnitte 21.1.2.1 bis 21.1.2.11
/// ohne die Funktionen): weder beschreibbar noch aufzählbar noch
/// konfigurierbar.
const CONSTANTS: [(&str, f64); 8] = [
    ("EPSILON", f64::EPSILON),
    ("MAX_SAFE_INTEGER", MAX_SAFE_INTEGER),
    ("MAX_VALUE", f64::MAX),
    ("MIN_SAFE_INTEGER", -MAX_SAFE_INTEGER),
    // 5 × 10⁻³²⁴, die kleinste Zahl über null.
    ("MIN_VALUE", 5e-324),
    ("NaN", f64::NAN),
    ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
    ("POSITIVE_INFINITY", f64::INFINITY),
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::NumberPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    for (name, value) in CONSTANTS {
        let value = agent.heap().local(scope, Value::number(value));
        let key = agent.key_from_units(scope, &units(name))?;
        let descriptor = PropertyDescriptor::data(value, false, false, false);
        define_property_or_throw(agent, scope, constructor, key, &descriptor)?;
    }
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    // Number.parseFloat und Number.parseInt (Abschnitte 21.1.2.12 und
    // 21.1.2.13) sind %parseFloat% und %parseInt%.
    for (name, intrinsic) in [
        ("parseFloat", Intrinsic::ParseFloat),
        ("parseInt", Intrinsic::ParseInt),
    ] {
        let function = agent.intrinsic(scope, realm, intrinsic);
        let key = agent.key_from_units(scope, &units(name))?;
        let descriptor = PropertyDescriptor::data(function, true, false, true);
        define_property_or_throw(agent, scope, constructor, key, &descriptor)?;
    }
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    agent.set_intrinsic(realm, Intrinsic::Number, constructor);
    Ok(())
}

fn number_value<'s>(agent: &Agent, scope: &'s Scope, value: f64) -> Local<'s> {
    agent.heap().local(scope, Value::number(value))
}

fn boolean<'s>(agent: &Agent, scope: &'s Scope, value: bool) -> Local<'s> {
    agent.heap().local(scope, Value::boolean(value))
}

fn string<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> Completion<Local<'s>> {
    Ok(agent.heap_mut().alloc_string(scope, &units(text))?)
}

/// Number ( value ) (Abschnitt 21.1.1.1). BigInt-Werte gibt es noch
/// nicht; ToNumeric ergibt also immer eine Zahl.
fn number_constructor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let n = match arguments.arguments.first() {
        Some(&value) => to_numeric(agent, scope, value)?,
        None => 0.0,
    };
    let n = number_value(agent, scope, n);
    let Some(new_target) = arguments.new_target else {
        return Ok(n);
    };
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::NumberPrototype,
        arguments.realm,
    )?;
    Ok(primitive_object_create(agent, scope, n, prototype)?)
}

/// Die Zahl im ersten Argument, `None` für alle anderen Werte.
fn number_argument(agent: &Agent, scope: &Scope, arguments: &BuiltinCall<'_, '_>) -> Option<f64> {
    let value = arguments.argument(agent, scope, 0);
    match agent.heap().get(value).kind() {
        Kind::Number(number) => Some(number),
        _ => None,
    }
}

/// Number.isFinite ( number ) (Abschnitt 21.1.2.2).
fn is_finite<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let finite = number_argument(agent, scope, arguments).is_some_and(f64::is_finite);
    Ok(boolean(agent, scope, finite))
}

/// IsIntegralNumber (Abschnitt 7.2.6) für Zahlen.
fn is_integral(number: f64) -> bool {
    number.is_finite() && number.trunc() == number
}

/// Number.isInteger ( number ) (Abschnitt 21.1.2.3).
fn is_integer<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let integral = number_argument(agent, scope, arguments).is_some_and(is_integral);
    Ok(boolean(agent, scope, integral))
}

/// Number.isNaN ( number ) (Abschnitt 21.1.2.4).
fn is_nan<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let nan = number_argument(agent, scope, arguments).is_some_and(f64::is_nan);
    Ok(boolean(agent, scope, nan))
}

/// Number.isSafeInteger ( number ) (Abschnitt 21.1.2.5).
fn is_safe_integer<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let safe = number_argument(agent, scope, arguments)
        .is_some_and(|number| is_integral(number) && number.abs() <= MAX_SAFE_INTEGER);
    Ok(boolean(agent, scope, safe))
}

/// ThisNumberValue (Abschnitt 21.1.3.7.1).
fn this_number_value(agent: &mut Agent, value: Local<'_>) -> Completion<f64> {
    let heap = agent.heap();
    let found = match heap.get(value).kind() {
        Kind::Number(number) => Some(number),
        Kind::Object(object) => match heap.primitive_data(object).map(Value::kind) {
            Some(Kind::Number(number)) => Some(number),
            _ => None,
        },
        _ => None,
    };
    found.ok_or_else(|| agent.throw_error(ErrorKind::TypeError))
}

/// Eine ganze Zahl aus ToIntegerOrInfinity im Bereich `low..=high` als
/// `usize`, sonst ein RangeError.
fn digits_in_range(agent: &mut Agent, count: f64, low: f64, high: f64) -> Completion<usize> {
    if !(low..=high).contains(&count) {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    // Ganz und höchstens 100: Die Umwandlung ist genau.
    Ok(count as usize)
}

/// Number.prototype.toExponential ( fractionDigits ) (Abschnitt
/// 21.1.3.2).
fn to_exponential<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let x = this_number_value(agent, arguments.this)?;
    let fraction_digits = arguments.argument(agent, scope, 0);
    let undefined = agent.heap().get(fraction_digits).is_undefined();
    let f = to_integer_or_infinity(agent, scope, fraction_digits)?;
    if !x.is_finite() {
        return string(agent, scope, &number::to_string(x));
    }
    let f = digits_in_range(agent, f, 0.0, 100.0)?;
    let text = number::to_exponential(x, (!undefined).then_some(f));
    string(agent, scope, &text)
}

/// Number.prototype.toFixed ( fractionDigits ) (Abschnitt 21.1.3.3).
fn to_fixed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let x = this_number_value(agent, arguments.this)?;
    let fraction_digits = arguments.argument(agent, scope, 0);
    let f = to_integer_or_infinity(agent, scope, fraction_digits)?;
    let f = digits_in_range(agent, f, 0.0, 100.0)?;
    // Ab 10²¹ gilt ToString wie für nicht endliche Zahlen.
    if !x.is_finite() || x.abs() >= 1e21 {
        return string(agent, scope, &number::to_string(x));
    }
    string(agent, scope, &number::to_fixed(x, f))
}

/// Number.prototype.toLocaleString ( [ reserved1 [ , reserved2 ] ] )
/// (Abschnitt 21.1.3.4). Ohne ECMA-402 legt die Implementierung das
/// Format fest; hier ist es das von `toString`.
fn to_locale_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let x = this_number_value(agent, arguments.this)?;
    string(agent, scope, &number::to_string(x))
}

/// Number.prototype.toPrecision ( precision ) (Abschnitt 21.1.3.5).
fn to_precision<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let x = this_number_value(agent, arguments.this)?;
    let precision = arguments.argument(agent, scope, 0);
    if agent.heap().get(precision).is_undefined() {
        return string(agent, scope, &number::to_string(x));
    }
    let p = to_integer_or_infinity(agent, scope, precision)?;
    if !x.is_finite() {
        return string(agent, scope, &number::to_string(x));
    }
    let p = digits_in_range(agent, p, 1.0, 100.0)?;
    string(agent, scope, &number::to_precision(x, p))
}

/// Number.prototype.toString ( [ radix ] ) (Abschnitt 21.1.3.6).
fn to_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let x = this_number_value(agent, arguments.this)?;
    let radix = arguments.argument(agent, scope, 0);
    let radix = if agent.heap().get(radix).is_undefined() {
        10
    } else {
        let radix = to_integer_or_infinity(agent, scope, radix)?;
        digits_in_range(agent, radix, 2.0, 36.0)?
    };
    let text = match u32::try_from(radix) {
        Ok(radix) if radix != 10 => number::to_radix_string(x, radix),
        _ => number::to_string(x),
    };
    string(agent, scope, &text)
}

/// Number.prototype.valueOf ( ) (Abschnitt 21.1.3.7).
fn value_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let x = this_number_value(agent, arguments.this)?;
    Ok(number_value(agent, scope, x))
}
