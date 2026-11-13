//! %Math% (Abschnitt 21.3): ein gewöhnliches Objekt mit den Konstanten aus
//! Abschnitt 21.3.1 und den Funktionen aus Abschnitt 21.3.2.
//!
//! Die Funktionen, deren Ergebnis die Spezifikation der Implementierung
//! überlässt (*implementation-approximated*), etwa `sin` oder `exp`,
//! rechnen mit den Methoden von `f64` aus der Standardbibliothek. Deren
//! Sonderfälle für NaN, ±0 und ±∞ sind die aus C99 Anhang F und decken
//! sich mit den Schritten der Spezifikation. Genau gerechnet sind `round`,
//! `fround`, `f16round`, `hypot` für die Sonderfälle und `sumPrecise`.

mod sum_precise;

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use super::{define_functions, define_to_string_tag, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall};
use crate::conversion::{to_number, to_uint32};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::iterator::{close_if_abrupt, get_iterator, iterator_step_value};
use crate::number::exponentiate;
use crate::object::ordinary_object_create;
use crate::operations::define_property_or_throw;
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};
use sum_precise::ExactSum;

/// Die Konstanten aus Abschnitt 21.3.1: weder beschreibbar noch aufzählbar
/// noch konfigurierbar.
const CONSTANTS: [(&str, f64); 8] = [
    ("E", std::f64::consts::E),
    ("LN10", std::f64::consts::LN_10),
    ("LN2", std::f64::consts::LN_2),
    ("LOG10E", std::f64::consts::LOG10_E),
    ("LOG2E", std::f64::consts::LOG2_E),
    ("PI", std::f64::consts::PI),
    ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
    ("SQRT2", std::f64::consts::SQRT_2),
];

/// Eine Funktion von %Math% mit einem Argument, das ToNumber umwandelt.
macro_rules! unary {
    ($($function:ident => $operation:expr,)*) => {
        $(
            fn $function<'s>(
                agent: &mut Agent,
                scope: &'s Scope,
                arguments: &BuiltinCall<'_, '_>,
            ) -> Completion<Local<'s>> {
                let operation: fn(f64) -> f64 = $operation;
                let value = arguments.argument(agent, scope, 0);
                let x = to_number(agent, scope, value)?;
                Ok(number(agent, scope, operation(x)))
            }
        )*
    };
}

const fn function(
    name: &'static str,
    length: u32,
    behaviour: crate::builtin::Behaviour,
) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

static FUNCTIONS: [Builtin; 37] = [
    function("abs", 1, abs),
    function("acos", 1, acos),
    function("acosh", 1, acosh),
    function("asin", 1, asin),
    function("asinh", 1, asinh),
    function("atan", 1, atan),
    function("atanh", 1, atanh),
    function("atan2", 2, atan2),
    function("cbrt", 1, cbrt),
    function("ceil", 1, ceil),
    function("clz32", 1, clz32),
    function("cos", 1, cos),
    function("cosh", 1, cosh),
    function("exp", 1, exp),
    function("expm1", 1, expm1),
    function("floor", 1, floor),
    function("fround", 1, fround),
    function("f16round", 1, f16round),
    function("hypot", 2, hypot),
    function("imul", 2, imul),
    function("log", 1, log),
    function("log1p", 1, log1p),
    function("log10", 1, log10),
    function("log2", 1, log2),
    function("max", 2, max),
    function("min", 2, min),
    function("pow", 2, pow),
    function("random", 0, random),
    function("round", 1, round),
    function("sign", 1, sign),
    function("sin", 1, sin),
    function("sinh", 1, sinh),
    function("sqrt", 1, sqrt),
    function("sumPrecise", 1, sum_precise),
    function("tan", 1, tan),
    function("tanh", 1, tanh),
    function("trunc", 1, trunc),
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let math = ordinary_object_create(agent, scope, Some(prototype))?;
    for (name, value) in CONSTANTS {
        let value = number(agent, scope, value);
        let key = agent.key_from_units(scope, &units(name))?;
        let descriptor = PropertyDescriptor::data(value, false, false, false);
        define_property_or_throw(agent, scope, math, key, &descriptor)?;
    }
    define_functions(agent, scope, realm, math, &FUNCTIONS)?;
    // Math [ %Symbol.toStringTag% ] (Abschnitt 21.3.1.9).
    define_to_string_tag(agent, scope, math, "Math")?;
    agent.set_intrinsic(realm, Intrinsic::Math, math);
    Ok(())
}

fn number<'s>(agent: &Agent, scope: &'s Scope, value: f64) -> Local<'s> {
    agent.heap().local(scope, Value::number(value))
}

/// Die Argumente in ihrer Reihenfolge mit ToNumber umgewandelt, wie `hypot`,
/// `max` und `min` es vor allem anderen tun.
fn coerce_all(
    agent: &mut Agent,
    scope: &Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Vec<f64>> {
    let mut coerced = Vec::with_capacity(arguments.arguments.len());
    for &argument in arguments.arguments {
        coerced.push(to_number(agent, scope, argument)?);
    }
    Ok(coerced)
}

unary! {
    abs => f64::abs,
    acos => f64::acos,
    acosh => f64::acosh,
    asin => f64::asin,
    asinh => f64::asinh,
    atan => f64::atan,
    atanh => f64::atanh,
    cbrt => f64::cbrt,
    ceil => f64::ceil,
    cos => f64::cos,
    cosh => f64::cosh,
    exp => f64::exp,
    expm1 => f64::exp_m1,
    floor => f64::floor,
    fround => to_float32,
    f16round => to_float16,
    log => f64::ln,
    log1p => f64::ln_1p,
    log10 => f64::log10,
    log2 => f64::log2,
    round => round_half_up,
    sign => sign_of,
    sin => f64::sin,
    sinh => f64::sinh,
    sqrt => f64::sqrt,
    tan => f64::tan,
    tanh => f64::tanh,
    trunc => f64::trunc,
}

/// Math.fround ( x ) (Abschnitt 21.3.2.17): die nächste Zahl aus binary32,
/// bei Gleichstand die gerade. So rundet Rusts `as` zwischen `f64` und
/// `f32`.
fn to_float32(x: f64) -> f64 {
    f64::from(x as f32)
}

/// 2^`exponent` für `exponent` im Bereich der normalen Zahlen.
fn power_of_two(exponent: i64) -> f64 {
    let biased = u64::try_from(exponent.saturating_add(1023)).unwrap_or(0);
    f64::from_bits(biased.min(0x7FE) << 52)
}

/// Math.f16round ( x ) (Abschnitt 21.3.2.18): die nächste Zahl aus
/// binary16, bei Gleichstand die gerade. Über dem Umweg über binary32
/// könnte zweimal gerundet werden; deshalb rundet die Funktion direkt auf
/// den Abstand benachbarter Zahlen von binary16.
fn to_float16(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let magnitude = x.abs();
    // 2⁻¹⁴ ist die kleinste normale Zahl von binary16; darunter liegen die
    // subnormalen im Abstand 2⁻²⁴, darüber 10 Bit Mantisse.
    let step = if magnitude < power_of_two(-14) {
        power_of_two(-24)
    } else {
        // Der Exponent der normalen Zahl `magnitude` aus ihren Bits.
        let exponent = i64::try_from((magnitude.to_bits() >> 52) & 0x7FF).unwrap_or(0) - 1023;
        power_of_two(exponent.saturating_sub(10))
    };
    // Teilen und Malnehmen mit Zweierpotenzen sind hier genau.
    let rounded = (magnitude / step).round_ties_even() * step;
    // 65504 ist die größte endliche Zahl von binary16; was darüber auf ein
    // Vielfaches rundet, ist schon ab 65520 und damit ∞.
    let result = if rounded > 65504.0 {
        f64::INFINITY
    } else {
        rounded
    };
    result.copysign(x)
}

/// Math.round ( x ) (Abschnitt 21.3.2.28): die nächste ganze Zahl, bei
/// Gleichstand die größere; zwischen -0,5 und -0 ergibt sich -0.
fn round_half_up(x: f64) -> f64 {
    if !x.is_finite() || x.trunc() == x {
        return x;
    }
    if (-0.5..0.0).contains(&x) {
        return -0.0;
    }
    // Für Zahlen mit Nachkommastellen ist x − floor(x) genau.
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// Math.sign ( x ) (Abschnitt 21.3.2.29).
fn sign_of(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        x
    } else {
        x.signum()
    }
}

/// Math.atan2 ( y, x ) (Abschnitt 21.3.2.8).
fn atan2<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let y = arguments.argument(agent, scope, 0);
    let y = to_number(agent, scope, y)?;
    let x = arguments.argument(agent, scope, 1);
    let x = to_number(agent, scope, x)?;
    Ok(number(agent, scope, y.atan2(x)))
}

/// Math.clz32 ( x ) (Abschnitt 21.3.2.11).
fn clz32<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let x = arguments.argument(agent, scope, 0);
    let n = to_uint32(agent, scope, x)?;
    Ok(number(agent, scope, f64::from(n.leading_zeros())))
}

/// Math.hypot ( ...args ) (Abschnitt 21.3.2.19).
fn hypot<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let coerced = coerce_all(agent, scope, arguments)?;
    Ok(number(agent, scope, hypot_of(&coerced)))
}

/// Die Schritte 3 bis 7 von Math.hypot. Die Wurzel der Quadratsumme
/// rechnet die Funktion mit dem größten Betrag als Maßstab, damit kein
/// Quadrat über- oder unterläuft; für zwei Zahlen gilt `f64::hypot`.
fn hypot_of(coerced: &[f64]) -> f64 {
    if coerced.iter().any(|number| number.is_infinite()) {
        return f64::INFINITY;
    }
    if coerced.iter().any(|number| number.is_nan()) {
        return f64::NAN;
    }
    let largest = coerced
        .iter()
        .fold(0.0_f64, |largest, number| largest.max(number.abs()));
    if largest == 0.0 {
        return 0.0;
    }
    if let [a, b] = coerced {
        return a.hypot(*b);
    }
    // Kahan-Summe der skalierten Quadrate.
    let mut sum = 0.0_f64;
    let mut compensation = 0.0_f64;
    for number in coerced {
        let scaled = number / largest;
        let term = scaled * scaled - compensation;
        let next = sum + term;
        compensation = (next - sum) - term;
        sum = next;
    }
    largest * sum.sqrt()
}

/// Math.imul ( x, y ) (Abschnitt 21.3.2.20).
fn imul<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let a = arguments.argument(agent, scope, 0);
    let a = to_uint32(agent, scope, a)?;
    let b = arguments.argument(agent, scope, 1);
    let b = to_uint32(agent, scope, b)?;
    let product = i32::from_ne_bytes(a.wrapping_mul(b).to_ne_bytes());
    Ok(number(agent, scope, f64::from(product)))
}

/// Math.max ( ...args ) (Abschnitt 21.3.2.24).
fn max<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let coerced = coerce_all(agent, scope, arguments)?;
    let mut highest = f64::NEG_INFINITY;
    for value in coerced {
        if value.is_nan() {
            return Ok(number(agent, scope, f64::NAN));
        }
        // +0 gilt als größer als -0.
        if value > highest || (value == 0.0 && highest == 0.0 && value.is_sign_positive()) {
            highest = value;
        }
    }
    Ok(number(agent, scope, highest))
}

/// Math.min ( ...args ) (Abschnitt 21.3.2.25).
fn min<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let coerced = coerce_all(agent, scope, arguments)?;
    let mut lowest = f64::INFINITY;
    for value in coerced {
        if value.is_nan() {
            return Ok(number(agent, scope, f64::NAN));
        }
        // -0 gilt als kleiner als +0.
        if value < lowest || (value == 0.0 && lowest == 0.0 && value.is_sign_negative()) {
            lowest = value;
        }
    }
    Ok(number(agent, scope, lowest))
}

/// Math.pow ( base, exponent ) (Abschnitt 21.3.2.26).
fn pow<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let base = arguments.argument(agent, scope, 0);
    let base = to_number(agent, scope, base)?;
    let exponent = arguments.argument(agent, scope, 1);
    let exponent = to_number(agent, scope, exponent)?;
    Ok(number(agent, scope, exponentiate(base, exponent)))
}

/// Ein neuer Zustand für `Math.random` aus dem Zufall der
/// Standardbibliothek: Jeder [`RandomState`] hat eigene Schlüssel, die
/// beim ersten im Thread vom Betriebssystem kommen.
pub(crate) fn random_seed() -> [u64; 2] {
    let state = RandomState::new();
    let mut seed = [0; 2];
    for (index, slot) in (0u8..).zip(seed.iter_mut()) {
        let mut hasher = state.build_hasher();
        hasher.write_u8(index);
        *slot = hasher.finish();
    }
    // Xorshift braucht einen Zustand, der nicht ganz null ist.
    if seed == [0, 0] { [1, 2] } else { seed }
}

/// Der nächste Wert von xorshift128+ als Zahl in [0, 1) mit 53 zufälligen
/// Bits.
fn next_random(state: &mut [u64; 2]) -> f64 {
    let [mut s1, s0] = *state;
    let result = s0.wrapping_add(s1);
    s1 ^= s1 << 23;
    *state = [s0, s1 ^ s0 ^ (s1 >> 18) ^ (s0 >> 5)];
    // Unter 2⁵³ ist die Umwandlung genau.
    let bits = (result >> 11) as f64;
    bits * power_of_two(-53)
}

/// Math.random ( ) (Abschnitt 21.3.2.27) mit dem Zustand des Realms.
fn random<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = agent.random_state(arguments.realm).map_or(0.0, next_random);
    Ok(number(agent, scope, value))
}

/// Der Zustand von Math.sumPrecise.
enum SumState {
    MinusZero,
    Finite,
    PlusInfinity,
    MinusInfinity,
    NotANumber,
}

/// Math.sumPrecise ( items ) (Abschnitt 21.3.2.34). Jeder Wert kostet einen
/// Arbeitsschritt.
fn sum_precise<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let items = arguments.argument(agent, scope, 0);
    let value = agent.heap().get(items);
    if value.is_undefined() || value.is_null() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let (iterator, next) = get_iterator(agent, scope, arguments.realm, items)?;
    let mut state = SumState::MinusZero;
    let mut sum = ExactSum::new();
    let mut count: u64 = 0;
    loop {
        agent.spend_execution_steps(1)?;
        let item = agent.scope(|agent, inner| -> Completion<Option<f64>> {
            let Some(value) = iterator_step_value(agent, inner, iterator, next)? else {
                return Ok(None);
            };
            count = count.saturating_add(1);
            if count >= 1 << 53 {
                let error = agent.throw_error(ErrorKind::RangeError);
                return close_if_abrupt(agent, inner, iterator, Err(error));
            }
            match agent.heap().get(value).kind() {
                Kind::Number(number) => Ok(Some(number)),
                _ => {
                    let error = agent.throw_error(ErrorKind::TypeError);
                    close_if_abrupt(agent, inner, iterator, Err(error))
                }
            }
        })?;
        let Some(n) = item else {
            break;
        };
        // Nach NaN zählt nur noch, dass jeder Wert eine Zahl ist.
        if matches!(state, SumState::NotANumber) {
            continue;
        }
        if n.is_nan() {
            state = SumState::NotANumber;
        } else if n == f64::INFINITY {
            state = if matches!(state, SumState::MinusInfinity) {
                SumState::NotANumber
            } else {
                SumState::PlusInfinity
            };
        } else if n == f64::NEG_INFINITY {
            state = if matches!(state, SumState::PlusInfinity) {
                SumState::NotANumber
            } else {
                SumState::MinusInfinity
            };
        } else if !(n == 0.0 && n.is_sign_negative())
            && matches!(state, SumState::MinusZero | SumState::Finite)
        {
            state = SumState::Finite;
            sum.add(n);
        }
    }
    let result = match state {
        SumState::NotANumber => f64::NAN,
        SumState::PlusInfinity => f64::INFINITY,
        SumState::MinusInfinity => f64::NEG_INFINITY,
        SumState::MinusZero => -0.0,
        SumState::Finite => sum.round(),
    };
    Ok(number(agent, scope, result))
}
