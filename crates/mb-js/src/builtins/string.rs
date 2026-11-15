//! %String% (Abschnitt 22.1.1) mit `String.fromCharCode`,
//! `String.fromCodePoint` und `String.raw` (Abschnitt 22.1.2) und
//! %String.prototype% (Abschnitt 22.1.3), selbst ein String-Objekt mit der
//! leeren Zeichenkette.
//!
//! Die Methoden des Prototyps stehen in `prototype`, die Methoden, die
//! einem Muster folgen (`match`, `replace`, `split` und ihre Verwandten),
//! in `pattern`, die Suche nach einer Teilfolge in `search` und die
//! Methoden aus Anhang B in `html`. `String.prototype[@@iterator]` liegt
//! bei den Iteratoren.
//!
//! Zeichenketten sind Folgen von Codeeinheiten und behalten einzelne
//! Surrogate (RS-9). Eine Zeichenkette hat höchstens 2³² − 1
//! Codeeinheiten; was länger würde, wirft wie die Verkettung einen
//! RangeError, und was nicht ins Speicherbudget passt, endet als
//! [`OutOfMemory`]. Steht die Länge vorher fest (`repeat`, `padStart`,
//! `padEnd`), entscheidet sich das, bevor Rust einen Puffer anlegt;
//! Ergebnisse, die Stück für Stück wachsen, prüfen bei jedem Anhängen.
//! Groß- und Kleinschreibung und Normalisierung rechnen erst und prüfen
//! dann: Ihr Ergebnis hat höchstens dreimal beziehungsweise achtzehnmal so
//! viele Codepunkte wie die Zeichenkette, die schon im Budget liegt.

mod html;
mod pattern;
mod prototype;
mod search;

use super::array::{append, index_key, step, units_of};
use super::symbol::symbol_descriptive_string;
use super::{
    define_functions, define_symbol_function, iterator::STRING_ITERATOR, link_constructor, units,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Behaviour, Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::{to_boolean, to_number, to_object, to_string, to_uint32};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, OutOfMemory, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{get_prototype_from_constructor, string_create};
use crate::operations::{define_property_or_throw, get, length_of_array_like};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "String",
    length: 1,
    constructor: true,
    behaviour: string,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 3] = [
    function("fromCharCode", 1, from_char_code),
    function("fromCodePoint", 1, from_code_point),
    function("raw", 1, raw),
];

const fn function(name: &'static str, length: u32, behaviour: Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::StringPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    define_functions(agent, scope, realm, prototype, &prototype::FUNCTIONS)?;
    define_symbol_function(
        agent,
        scope,
        realm,
        prototype,
        WellKnownSymbol::Iterator,
        &STRING_ITERATOR,
    )?;
    define_functions(agent, scope, realm, prototype, &html::FUNCTIONS)?;
    // `trimLeft` und `trimRight` sind dieselben Funktionsobjekte wie
    // `trimStart` und `trimEnd` (Abschnitte B.2.2.15 und B.2.2.16).
    for (name, alias) in [("trimStart", "trimLeft"), ("trimEnd", "trimRight")] {
        let key = agent.key_from_units(scope, &units(name))?;
        let function = get(agent, scope, prototype, key)?;
        let key = agent.key_from_units(scope, &units(alias))?;
        let descriptor = PropertyDescriptor::data(function, true, false, true);
        define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    }
    agent.set_intrinsic(realm, Intrinsic::String, constructor);
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

fn is_undefined(agent: &Agent, value: Local<'_>) -> bool {
    agent.heap().get(value).is_undefined()
}

/// Eine Länge oder Stelle als Zahl. Zeichenketten haben höchstens 2³² − 1
/// Codeeinheiten, die Umwandlung ist also exakt.
fn count(length: usize) -> f64 {
    u32::try_from(length).map_or(f64::INFINITY, f64::from)
}

/// RequireObjectCoercible (Abschnitt 7.2.1).
fn require_object_coercible(agent: &mut Agent, value: Local<'_>) -> Completion<()> {
    let found = agent.heap().get(value);
    if found.is_undefined() || found.is_null() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(())
}

/// RequireObjectCoercible(*this*) und ToString, die ersten Schritte fast
/// jeder Methode.
fn this_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_object_coercible(agent, arguments.this)?;
    to_string(agent, scope, arguments.this)
}

/// ToString des Arguments `index`.
fn argument_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    index: usize,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, index);
    to_string(agent, scope, value)
}

/// Die Länge einer Zeichenkette in Codeeinheiten.
fn length_of(agent: &Agent, string: Local<'_>) -> usize {
    let heap = agent.heap();
    match heap.get(string).kind() {
        Kind::String(string) => heap.string_len(string),
        _ => 0,
    }
}

/// Die Codeeinheit an der Stelle `index`, ohne die Zeichenkette zu
/// kopieren.
fn unit_at(agent: &Agent, string: Local<'_>, index: usize) -> Option<u16> {
    let heap = agent.heap();
    match heap.get(string).kind() {
        Kind::String(string) => heap.string_unit(string, index),
        _ => None,
    }
}

/// Die Codeeinheiten von `from` bis vor `to`.
fn units_between(agent: &Agent, string: Local<'_>, from: usize, to: usize) -> Vec<u16> {
    (from..to)
        .map_while(|index| unit_at(agent, string, index))
        .collect()
}

/// Der Teil von `from` bis vor `to` als Zeichenkette; der ganze Bereich
/// ist die Zeichenkette selbst.
fn substring<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    string: Local<'_>,
    from: usize,
    to: usize,
) -> Completion<Local<'s>> {
    if from == 0 && to >= length_of(agent, string) {
        let heap = agent.heap();
        return Ok(heap.local(scope, heap.get(string)));
    }
    let units = units_between(agent, string, from, to);
    Ok(agent.heap_mut().alloc_string(scope, &units)?)
}

/// Eine neue Zeichenkette aus `units`.
fn new_string<'s>(agent: &mut Agent, scope: &'s Scope, units: &[u16]) -> Completion<Local<'s>> {
    check_length(agent, units.len())?;
    Ok(agent.heap_mut().alloc_string(scope, units)?)
}

/// Ob eine Zeichenkette mit `length` Codeeinheiten entstehen darf: über
/// 2³² − 1 ein RangeError wie bei der Verkettung, über dem Speicherbudget
/// [`OutOfMemory`]. Gerechnet wird wie in [`append`] mit zwei Bytes je
/// Codeeinheit, so viel braucht der Puffer in Rust.
fn check_length(agent: &mut Agent, length: usize) -> Completion<()> {
    if u32::try_from(length).is_err() {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let heap = agent.heap();
    let bytes = length.saturating_mul(2).saturating_add(heap.used_bytes());
    if bytes > heap.limit() {
        return Err(OutOfMemory.into());
    }
    Ok(())
}

/// Hängt `units` an eine wachsende Zeichenkette an: ein RangeError über
/// 2³² − 1 Codeeinheiten, sonst wie [`append`].
fn push_units(agent: &mut Agent, text: &mut Vec<u16>, units: &[u16]) -> Completion<()> {
    let fits = text
        .len()
        .checked_add(units.len())
        .is_some_and(|length| u32::try_from(length).is_ok());
    if !fits {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    append(agent, text, units)
}

/// Eine ganze Zahl oder ±∞ zwischen 0 und `length` eingeklemmt, als
/// Stelle.
fn clamp_position(position: f64, length: usize) -> usize {
    if position >= count(length) {
        length
    } else if position > 0.0 {
        // Zwischen 0 und der Länge und ganzzahlig, schneidet nicht ab.
        position as usize
    } else {
        0
    }
}

/// CodePointAt (Abschnitt 11.1.4) auf Codeeinheiten: der Codepunkt an
/// `position` und wie viele Codeeinheiten er belegt. Ein einzelnes
/// Surrogat ist sein eigener Codepunkt.
fn code_point_at(unit: impl Fn(usize) -> Option<u16>, position: usize) -> Option<(u32, usize)> {
    let first = unit(position)?;
    if (0xD800..=0xDBFF).contains(&first)
        && let Some(second) = position.checked_add(1).and_then(&unit)
        && (0xDC00..=0xDFFF).contains(&second)
    {
        let code_point =
            0x1_0000 + ((u32::from(first) - 0xD800) << 10) + (u32::from(second) - 0xDC00);
        return Some((code_point, 2));
    }
    Some((u32::from(first), 1))
}

/// Die Codepunkte einer Folge von Codeeinheiten.
fn code_points(units: &[u16]) -> Vec<u32> {
    let mut code_points = Vec::with_capacity(units.len());
    let mut position = 0;
    while let Some((code_point, width)) = code_point_at(|index| units.get(index).copied(), position)
    {
        code_points.push(code_point);
        position += width;
    }
    code_points
}

/// UTF16EncodeCodePoint (Abschnitt 11.1.1); ein einzelnes Surrogat bleibt
/// eine Codeeinheit.
fn push_code_point(units: &mut Vec<u16>, code_point: u32) {
    let mut buffer = [0; 2];
    match char::from_u32(code_point) {
        Some(c) => units.extend_from_slice(c.encode_utf16(&mut buffer)),
        None => units.extend(u16::try_from(code_point).ok()),
    }
}

/// IsRegExp (Abschnitt 7.2.6). Objekte mit [[RegExpMatcher]] gibt es noch
/// nicht; bis dahin entscheidet allein @@match.
fn is_regexp(agent: &mut Agent, value: Local<'_>) -> Completion<bool> {
    if !is_object(agent, value) {
        return Ok(false);
    }
    agent.scope(|agent, scope| {
        let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::Match));
        let matcher = get(agent, scope, value, key)?;
        let heap = agent.heap();
        let found = heap.get(matcher);
        Ok(!found.is_undefined() && to_boolean(heap, found))
    })
}

// Der Konstruktor und seine Funktionen

/// String ( value ) (Abschnitt 22.1.1.1).
fn string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let text = match arguments.arguments.first() {
        None => agent.heap_mut().alloc_string(scope, &[])?,
        Some(&value) => {
            if arguments.new_target.is_none()
                && matches!(agent.heap().get(value).kind(), Kind::Symbol(_))
            {
                return symbol_descriptive_string(agent, scope, value);
            }
            to_string(agent, scope, value)?
        }
    };
    let Some(new_target) = arguments.new_target else {
        return Ok(text);
    };
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::StringPrototype,
        arguments.realm,
    )?;
    string_create(agent, scope, text, prototype)
}

/// String.fromCharCode ( ...codeUnits ) (Abschnitt 22.1.2.1).
fn from_char_code<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let mut result = Vec::with_capacity(arguments.arguments.len());
    for &next in arguments.arguments {
        // ToUint16 (Abschnitt 7.1.11) sind die unteren 16 Bit von ToUint32.
        let unit = to_uint32(agent, scope, next)? & 0xFFFF;
        result.extend(u16::try_from(unit).ok());
    }
    new_string(agent, scope, &result)
}

/// String.fromCodePoint ( ...codePoints ) (Abschnitt 22.1.2.2).
fn from_code_point<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let mut result = Vec::with_capacity(arguments.arguments.len());
    for &next in arguments.arguments {
        let code_point = to_number(agent, scope, next)?;
        let valid = code_point.fract() == 0.0 && (0.0..=1_114_111.0).contains(&code_point);
        if !valid {
            return Err(agent.throw_error(ErrorKind::RangeError));
        }
        // Ganzzahlig und im Bereich der Codepunkte, schneidet nicht ab.
        push_code_point(&mut result, code_point as u32);
    }
    new_string(agent, scope, &result)
}

/// String.raw ( template, ...substitutions ) (Abschnitt 22.1.2.4). Jedes
/// Literal kostet einen Arbeitsschritt, damit eine riesige `length` am
/// Budget endet.
fn raw<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let substitutions = arguments.arguments.get(1..).unwrap_or_default();
    let template = arguments.argument(agent, scope, 0);
    let cooked = to_object(agent, scope, arguments.realm, template)?;
    let key = agent.key_from_units(scope, &units("raw"))?;
    let literals = get(agent, scope, cooked, key)?;
    let literals = to_object(agent, scope, arguments.realm, literals)?;
    let literal_count = length_of_array_like(agent, scope, literals)?;
    let mut result = Vec::new();
    let mut next_index = 0.0;
    let mut substitutions = substitutions.iter();
    while next_index < literal_count {
        step(agent)?;
        let last = next_index + 1.0 == literal_count;
        let substitution = substitutions.next();
        agent.scope(|agent, inner| -> Completion<()> {
            let key = index_key(agent, inner, next_index)?;
            let literal = get(agent, inner, literals, key)?;
            let literal = to_string(agent, inner, literal)?;
            push_units(agent, &mut result, &units_of(agent, literal))?;
            if let (false, Some(&substitution)) = (last, substitution) {
                let substitution = to_string(agent, inner, substitution)?;
                push_units(agent, &mut result, &units_of(agent, substitution))?;
            }
            Ok(())
        })?;
        next_index += 1.0;
    }
    new_string(agent, scope, &result)
}
