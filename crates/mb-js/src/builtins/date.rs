//! %Date% (Abschnitte 21.4.2 und 21.4.3) und %Date.prototype% (Abschnitt
//! 21.4.4), ein gewöhnliches Objekt ohne [[DateValue]], dazu `getYear`,
//! `setYear` und `toGMTString` aus Anhang B.2.3.
//!
//! Ein Date-Objekt ist ein Objekt der Art [`ObjectClass::Date`] mit
//! [[DateValue]] als internem Feld. Die Rechnung mit Zeitwerten steht in
//! [`crate::date`]; Uhr und Zeitzone kommen vom Host
//! ([`crate::host::Host::current_time`] und
//! [`crate::host::Host::time_zone`]).

use super::string::{new_string, number};
use super::{define_functions, define_method, link_constructor, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::{
    PreferredType, ordinary_to_primitive, to_number, to_object, to_primitive, to_string,
};
use crate::date::{self, LocaleParts};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, ObjectClass, Scope};
use crate::host::{current_time, time_zone};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{INLINE, get_prototype_from_constructor};
use crate::operations::{define_property_or_throw, invoke};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "Date",
    length: 7,
    constructor: true,
    behaviour: date_constructor,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 3] = [
    Builtin {
        name: "now",
        length: 0,
        constructor: false,
        behaviour: now,
    },
    Builtin {
        name: "parse",
        length: 1,
        constructor: false,
        behaviour: parse,
    },
    Builtin {
        name: "UTC",
        length: 7,
        constructor: false,
        behaviour: utc,
    },
];

/// Ein Eintrag der Tabelle der Methoden.
const fn method(name: &'static str, length: u32, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

static PROTOTYPE_FUNCTIONS: [Builtin; 44] = [
    method("getDate", 0, get_date),
    method("getDay", 0, get_day),
    method("getFullYear", 0, get_full_year),
    method("getHours", 0, get_hours),
    method("getMilliseconds", 0, get_milliseconds),
    method("getMinutes", 0, get_minutes),
    method("getMonth", 0, get_month),
    method("getSeconds", 0, get_seconds),
    method("getTime", 0, get_time),
    method("getTimezoneOffset", 0, get_timezone_offset),
    method("getUTCDate", 0, get_utc_date),
    method("getUTCDay", 0, get_utc_day),
    method("getUTCFullYear", 0, get_utc_full_year),
    method("getUTCHours", 0, get_utc_hours),
    method("getUTCMilliseconds", 0, get_utc_milliseconds),
    method("getUTCMinutes", 0, get_utc_minutes),
    method("getUTCMonth", 0, get_utc_month),
    method("getUTCSeconds", 0, get_utc_seconds),
    method("setDate", 1, set_date),
    method("setFullYear", 3, set_full_year),
    method("setHours", 4, set_hours),
    method("setMilliseconds", 1, set_milliseconds),
    method("setMinutes", 3, set_minutes),
    method("setMonth", 2, set_month),
    method("setSeconds", 2, set_seconds),
    method("setTime", 1, set_time),
    method("setUTCDate", 1, set_utc_date),
    method("setUTCFullYear", 3, set_utc_full_year),
    method("setUTCHours", 4, set_utc_hours),
    method("setUTCMilliseconds", 1, set_utc_milliseconds),
    method("setUTCMinutes", 3, set_utc_minutes),
    method("setUTCMonth", 2, set_utc_month),
    method("setUTCSeconds", 2, set_utc_seconds),
    method("toDateString", 0, to_date_string),
    method("toISOString", 0, to_iso_string),
    method("toJSON", 1, to_json),
    method("toLocaleDateString", 0, to_locale_date_string),
    method("toLocaleString", 0, to_locale_string),
    method("toLocaleTimeString", 0, to_locale_time_string),
    method("toString", 0, to_string_method),
    method("toTimeString", 0, to_time_string),
    method("valueOf", 0, get_time),
    // Anhang B.2.3.
    method("getYear", 0, get_year),
    method("setYear", 1, set_year),
];

static TO_UTC_STRING: Builtin = method("toUTCString", 0, to_utc_string);

static TO_PRIMITIVE: Builtin = method("[Symbol.toPrimitive]", 1, to_primitive_method);

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::DatePrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    // Date.prototype.toGMTString ist dieselbe Funktion wie toUTCString
    // (Anhang B.2.3.3).
    let to_utc = define_method(agent, scope, realm, prototype, &TO_UTC_STRING)?;
    let key = agent.key_from_units(scope, &units("toGMTString"))?;
    let descriptor = PropertyDescriptor::data(to_utc, true, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    // Date.prototype [ %Symbol.toPrimitive% ] (Abschnitt 21.4.4.45): nur
    // konfigurierbar.
    let function = create_builtin_function(agent, scope, realm, &TO_PRIMITIVE)?;
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::ToPrimitive));
    let descriptor = PropertyDescriptor::data(function, false, false, true);
    define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    agent.set_intrinsic(realm, Intrinsic::Date, constructor);
    Ok(())
}

// Helfer

/// thisTimeValue: RequireInternalSlot(*this*, [[DateValue]]) und der Wert.
fn this_time_value(agent: &mut Agent, value: Local<'_>) -> Completion<f64> {
    let heap = agent.heap();
    let found = match heap.get(value).kind() {
        Kind::Object(object) => heap.date_value(object),
        _ => None,
    };
    found.ok_or_else(|| agent.throw_error(ErrorKind::TypeError))
}

/// Setzt [[DateValue]] eines Date-Objekts.
fn set_time_value(agent: &Agent, object: Local<'_>, value: f64) {
    let heap = agent.heap();
    if let Kind::Object(reference) = heap.get(object).kind() {
        heap.set_date_value(reference, value);
    }
}

fn text<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> Completion<Local<'s>> {
    new_string(agent, scope, &units(text))
}

/// ToNumber für Argument `index`, *undefined*, wenn es fehlt.
fn number_argument(
    agent: &mut Agent,
    scope: &Scope,
    arguments: &BuiltinCall<'_, '_>,
    index: usize,
) -> Completion<f64> {
    let value = arguments.argument(agent, scope, index);
    to_number(agent, scope, value)
}

// Der Konstruktor

/// Date ( ...values ) (Abschnitt 21.4.2.1).
fn date_constructor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        let now = current_time(agent);
        let zone = time_zone(agent);
        return text(agent, scope, &date::to_date_string(now, zone));
    };
    let value = match arguments.arguments {
        [] => current_time(agent),
        [value] => {
            let found = match agent.heap().get(*value).kind() {
                Kind::Object(object) => agent.heap().date_value(object),
                _ => None,
            };
            let tv = match found {
                Some(tv) => tv,
                None => {
                    let primitive = to_primitive(agent, scope, *value, None)?;
                    match agent.heap().get(primitive).kind() {
                        Kind::String(string) => {
                            let text = agent.heap().string_units(string);
                            date::parse(&text, time_zone(agent))
                        }
                        _ => to_number(agent, scope, primitive)?,
                    }
                }
            };
            date::time_clip(tv)
        }
        _ => {
            let fields = date_fields(agent, scope, arguments)?;
            date::time_clip(date::utc(fields, time_zone(agent)))
        }
    };
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::DatePrototype,
        arguments.realm,
    )?;
    let shape = agent.empty_shape(scope);
    let object =
        agent
            .heap_mut()
            .alloc_object(scope, ObjectClass::Date, INLINE, shape, Some(prototype))?;
    set_time_value(agent, object, value);
    Ok(object)
}

/// Die Schritte von Date ( ...values ) mit mindestens zwei Argumenten und
/// von Date.UTC bis vor UTC und TimeClip: Jahr, Monat und die übrigen
/// Felder mit ToNumber, die fehlenden mit ihren Vorgaben, MakeFullYear,
/// dann MakeDate.
fn date_fields(
    agent: &mut Agent,
    scope: &Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<f64> {
    // Vorgaben für Monat, Tag, Stunde, Minute, Sekunde und Millisekunde.
    let mut values = [f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    for (index, slot) in values.iter_mut().enumerate() {
        if index == 0 || index < arguments.arguments.len() {
            *slot = number_argument(agent, scope, arguments, index)?;
        }
    }
    let [year, month, day, hour, minute, second, millisecond] = values;
    Ok(date::make_date(
        date::make_day(date::make_full_year(year), month, day),
        date::make_time(hour, minute, second, millisecond),
    ))
}

/// Date.now ( ) (Abschnitt 21.4.3.1).
fn now<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    _arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = current_time(agent);
    Ok(number(agent, scope, value))
}

/// Date.parse ( string ) (Abschnitt 21.4.3.2).
fn parse<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let string = to_string(agent, scope, value)?;
    let text = match agent.heap().get(string).kind() {
        Kind::String(string) => agent.heap().string_units(string),
        _ => Vec::new(),
    };
    let value = date::parse(&text, time_zone(agent));
    Ok(number(agent, scope, value))
}

/// Date.UTC ( year [ , month [ , … ] ] ) (Abschnitt 21.4.3.4).
fn utc<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = date::time_clip(date_fields(agent, scope, arguments)?);
    Ok(number(agent, scope, value))
}

// Lesen

/// Ein Feld von LocalTime(tv) oder tv, NaN für NaN.
fn get_field<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    local: bool,
    field: fn(&date::Fields) -> u32,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    if tv.is_nan() {
        return Ok(number(agent, scope, f64::NAN));
    }
    let t = if local {
        date::local_time(tv, time_zone(agent))
    } else {
        tv
    };
    let value = f64::from(field(&date::fields(t)));
    Ok(number(agent, scope, value))
}

/// YearFromTime von LocalTime(tv) oder tv, minus `base`.
fn get_year_field<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    local: bool,
    base: f64,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    if tv.is_nan() {
        return Ok(number(agent, scope, f64::NAN));
    }
    let t = if local {
        date::local_time(tv, time_zone(agent))
    } else {
        tv
    };
    // Jahre eines Zeitwerts liegen unter 300.000 im Betrag.
    let year = date::fields(t).year as f64;
    Ok(number(agent, scope, year - base))
}

/// Date.prototype.getDate ( ) (Abschnitt 21.4.4.2).
fn get_date<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, true, |fields| fields.date)
}

/// Date.prototype.getDay ( ) (Abschnitt 21.4.4.3).
fn get_day<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, true, |fields| fields.weekday)
}

/// Date.prototype.getFullYear ( ) (Abschnitt 21.4.4.4).
fn get_full_year<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_year_field(agent, scope, arguments, true, 0.0)
}

/// Date.prototype.getHours ( ) (Abschnitt 21.4.4.5).
fn get_hours<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, true, |fields| fields.hour)
}

/// Date.prototype.getMilliseconds ( ) (Abschnitt 21.4.4.6).
fn get_milliseconds<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, true, |fields| fields.millisecond)
}

/// Date.prototype.getMinutes ( ) (Abschnitt 21.4.4.7).
fn get_minutes<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, true, |fields| fields.minute)
}

/// Date.prototype.getMonth ( ) (Abschnitt 21.4.4.8).
fn get_month<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, true, |fields| fields.month)
}

/// Date.prototype.getSeconds ( ) (Abschnitt 21.4.4.9).
fn get_seconds<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, true, |fields| fields.second)
}

/// Date.prototype.getTime ( ) (Abschnitt 21.4.4.10), auch
/// Date.prototype.valueOf ( ) (Abschnitt 21.4.4.44).
fn get_time<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    Ok(number(agent, scope, tv))
}

/// Date.prototype.getTimezoneOffset ( ) (Abschnitt 21.4.4.11).
fn get_timezone_offset<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    if tv.is_nan() {
        return Ok(number(agent, scope, f64::NAN));
    }
    let local = date::local_time(tv, time_zone(agent));
    Ok(number(agent, scope, (tv - local) / 60_000.0))
}

/// Date.prototype.getUTCDate ( ) (Abschnitt 21.4.4.12).
fn get_utc_date<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, false, |fields| fields.date)
}

/// Date.prototype.getUTCDay ( ) (Abschnitt 21.4.4.13).
fn get_utc_day<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, false, |fields| fields.weekday)
}

/// Date.prototype.getUTCFullYear ( ) (Abschnitt 21.4.4.14).
fn get_utc_full_year<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_year_field(agent, scope, arguments, false, 0.0)
}

/// Date.prototype.getUTCHours ( ) (Abschnitt 21.4.4.15).
fn get_utc_hours<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, false, |fields| fields.hour)
}

/// Date.prototype.getUTCMilliseconds ( ) (Abschnitt 21.4.4.16).
fn get_utc_milliseconds<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, false, |fields| fields.millisecond)
}

/// Date.prototype.getUTCMinutes ( ) (Abschnitt 21.4.4.17).
fn get_utc_minutes<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, false, |fields| fields.minute)
}

/// Date.prototype.getUTCMonth ( ) (Abschnitt 21.4.4.18).
fn get_utc_month<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, false, |fields| fields.month)
}

/// Date.prototype.getUTCSeconds ( ) (Abschnitt 21.4.4.19).
fn get_utc_seconds<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_field(agent, scope, arguments, false, |fields| fields.second)
}

/// Date.prototype.getYear ( ) (Anhang B.2.3.1).
fn get_year<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    get_year_field(agent, scope, arguments, true, 1900.0)
}

// Schreiben

/// Die Felder in der Reihenfolge der Setter: Jahr, Monat, Tag, Stunde,
/// Minute, Sekunde, Millisekunde.
const YEAR: usize = 0;
const DATE: usize = 2;
const HOUR: usize = 3;
const MINUTE: usize = 4;
const SECOND: usize = 5;
const MILLISECOND: usize = 6;

/// Die Setter von setDate bis setUTCSeconds (Abschnitte 21.4.4.20 bis
/// 21.4.4.34) ohne setTime. Jeder setzt die Felder ab `first`, höchstens
/// `count`: das erste immer mit ToNumber, auch wenn das Argument fehlt,
/// die weiteren nur, wenn ihr Argument da ist; die übrigen kommen aus
/// LocalTime(tv) oder tv. Alle ToNumber laufen, bevor ein Zeitwert NaN das
/// Ergebnis NaN macht; nur die Setter für das Jahr rechnen dann von +0 aus.
///
/// Die Spezifikation nimmt für die Setter des Datums TimeWithinDay(t) und
/// für die der Uhrzeit Day(t), statt MakeTime und MakeDay der alten Felder;
/// beides ergibt dieselben Numbers, denn die Felder sind kleine ganze
/// Zahlen.
fn set_fields<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    local: bool,
    first: usize,
    count: usize,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    let mut given = [None; 7];
    for offset in 0..count {
        if offset == 0 || offset < arguments.arguments.len() {
            let value = number_argument(agent, scope, arguments, offset)?;
            if let Some(slot) = given.get_mut(first + offset) {
                *slot = Some(value);
            }
        }
    }
    let zone = time_zone(agent);
    let t = if tv.is_nan() {
        if first != YEAR {
            return Ok(number(agent, scope, f64::NAN));
        }
        0.0
    } else if local {
        date::local_time(tv, zone)
    } else {
        tv
    };
    let fields = date::fields(t);
    // Jahre eines Zeitwerts liegen unter 300.000 im Betrag.
    let old = [
        fields.year as f64,
        f64::from(fields.month),
        f64::from(fields.date),
        f64::from(fields.hour),
        f64::from(fields.minute),
        f64::from(fields.second),
        f64::from(fields.millisecond),
    ];
    let mut values = old;
    for (value, given) in values.iter_mut().zip(given) {
        if let Some(given) = given {
            *value = given;
        }
    }
    let [year, month, day, hour, minute, second, millisecond] = values;
    let new_date = date::make_date(
        date::make_day(year, month, day),
        date::make_time(hour, minute, second, millisecond),
    );
    let value = date::time_clip(if local {
        date::utc(new_date, zone)
    } else {
        new_date
    });
    set_time_value(agent, arguments.this, value);
    Ok(number(agent, scope, value))
}

/// Date.prototype.setDate ( day ) (Abschnitt 21.4.4.20).
fn set_date<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, true, DATE, 1)
}

/// Date.prototype.setFullYear ( year [ , month [ , day ] ] ) (Abschnitt
/// 21.4.4.21).
fn set_full_year<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, true, YEAR, 3)
}

/// Date.prototype.setHours ( hour [ , min [ , sec [ , ms ] ] ] )
/// (Abschnitt 21.4.4.22).
fn set_hours<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, true, HOUR, 4)
}

/// Date.prototype.setMilliseconds ( ms ) (Abschnitt 21.4.4.23).
fn set_milliseconds<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, true, MILLISECOND, 1)
}

/// Date.prototype.setMinutes ( min [ , sec [ , ms ] ] ) (Abschnitt
/// 21.4.4.24).
fn set_minutes<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, true, MINUTE, 3)
}

/// Date.prototype.setMonth ( month [ , date ] ) (Abschnitt 21.4.4.25).
fn set_month<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, true, YEAR + 1, 2)
}

/// Date.prototype.setSeconds ( sec [ , ms ] ) (Abschnitt 21.4.4.26).
fn set_seconds<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, true, SECOND, 2)
}

/// Date.prototype.setTime ( time ) (Abschnitt 21.4.4.27).
fn set_time<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    this_time_value(agent, arguments.this)?;
    let time = number_argument(agent, scope, arguments, 0)?;
    let value = date::time_clip(time);
    set_time_value(agent, arguments.this, value);
    Ok(number(agent, scope, value))
}

/// Date.prototype.setUTCDate ( date ) (Abschnitt 21.4.4.28).
fn set_utc_date<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, false, DATE, 1)
}

/// Date.prototype.setUTCFullYear ( year [ , month [ , date ] ] )
/// (Abschnitt 21.4.4.29).
fn set_utc_full_year<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, false, YEAR, 3)
}

/// Date.prototype.setUTCHours ( hour [ , min [ , sec [ , ms ] ] ] )
/// (Abschnitt 21.4.4.30).
fn set_utc_hours<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, false, HOUR, 4)
}

/// Date.prototype.setUTCMilliseconds ( ms ) (Abschnitt 21.4.4.31).
fn set_utc_milliseconds<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, false, MILLISECOND, 1)
}

/// Date.prototype.setUTCMinutes ( min [ , sec [ , ms ] ] ) (Abschnitt
/// 21.4.4.32).
fn set_utc_minutes<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, false, MINUTE, 3)
}

/// Date.prototype.setUTCMonth ( month [ , date ] ) (Abschnitt 21.4.4.33).
fn set_utc_month<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, false, YEAR + 1, 2)
}

/// Date.prototype.setUTCSeconds ( sec [ , ms ] ) (Abschnitt 21.4.4.34).
fn set_utc_seconds<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    set_fields(agent, scope, arguments, false, SECOND, 2)
}

/// Date.prototype.setYear ( year ) (Anhang B.2.3.2).
fn set_year<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    let zone = time_zone(agent);
    let t = if tv.is_nan() {
        0.0
    } else {
        date::local_time(tv, zone)
    };
    let year = number_argument(agent, scope, arguments, 0)?;
    let full_year = date::make_full_year(year);
    let fields = date::fields(t);
    let day = date::make_day(full_year, f64::from(fields.month), f64::from(fields.date));
    let new_date = date::make_date(day, date::time_within_day(t));
    let value = date::time_clip(date::utc(new_date, zone));
    set_time_value(agent, arguments.this, value);
    Ok(number(agent, scope, value))
}

// Texte

/// Date.prototype.toDateString ( ) (Abschnitt 21.4.4.35).
fn to_date_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    let zone = time_zone(agent);
    text(agent, scope, &date::to_date_only_string(tv, zone))
}

/// Date.prototype.toISOString ( ) (Abschnitt 21.4.4.36). Jedes Jahr eines
/// Zeitwerts passt in das Format; nur NaN wirft.
fn to_iso_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    if tv.is_nan() {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    text(agent, scope, &date::to_iso_string(tv))
}

/// Date.prototype.toJSON ( key ) (Abschnitt 21.4.4.37).
fn to_json<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = to_object(agent, scope, arguments.realm, arguments.this)?;
    let tv = to_primitive(agent, scope, object, Some(PreferredType::Number))?;
    if agent
        .heap()
        .get(tv)
        .as_number()
        .is_some_and(|tv| !tv.is_finite())
    {
        return Ok(agent.heap().local(scope, Value::NULL));
    }
    let key = agent.key_from_units(scope, &units("toISOString"))?;
    invoke(agent, scope, arguments.realm, object, key, &[])
}

/// Date.prototype.toLocaleDateString ( ) (Abschnitt 21.4.4.38) ohne
/// ECMA-402.
fn to_locale_date_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    locale_string(agent, scope, arguments, LocaleParts::Date)
}

/// Date.prototype.toLocaleString ( ) (Abschnitt 21.4.4.39) ohne ECMA-402.
fn to_locale_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    locale_string(agent, scope, arguments, LocaleParts::DateAndTime)
}

/// Date.prototype.toLocaleTimeString ( ) (Abschnitt 21.4.4.40) ohne
/// ECMA-402.
fn to_locale_time_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    locale_string(agent, scope, arguments, LocaleParts::Time)
}

fn locale_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    parts: LocaleParts,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    let zone = time_zone(agent);
    text(agent, scope, &date::to_locale_string(tv, zone, parts))
}

/// Date.prototype.toString ( ) (Abschnitt 21.4.4.41).
fn to_string_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    let zone = time_zone(agent);
    text(agent, scope, &date::to_date_string(tv, zone))
}

/// Date.prototype.toTimeString ( ) (Abschnitt 21.4.4.42).
fn to_time_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    let zone = time_zone(agent);
    text(agent, scope, &date::to_time_only_string(tv, zone))
}

/// Date.prototype.toUTCString ( ) (Abschnitt 21.4.4.43), auch
/// Date.prototype.toGMTString (Anhang B.2.3.3).
fn to_utc_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let tv = this_time_value(agent, arguments.this)?;
    text(agent, scope, &date::to_utc_string(tv))
}

/// Date.prototype [ %Symbol.toPrimitive% ] ( hint ) (Abschnitt 21.4.4.45).
fn to_primitive_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    if !matches!(heap.get(arguments.this).kind(), Kind::Object(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let hint = arguments.argument(agent, scope, 0);
    let hint = match agent.heap().get(hint).kind() {
        Kind::String(string) => agent.heap().string_units(string),
        _ => Vec::new(),
    };
    let try_first = if hint == units("string") || hint == units("default") {
        PreferredType::String
    } else if hint == units("number") {
        PreferredType::Number
    } else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    ordinary_to_primitive(agent, scope, arguments.this, try_first)
}
