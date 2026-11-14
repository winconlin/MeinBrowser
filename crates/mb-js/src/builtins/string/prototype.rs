//! Die Methoden von %String.prototype% (Abschnitt 22.1.3). Die Methoden,
//! die einem Muster folgen, stehen in `pattern`; die Tabelle hier nennt sie
//! mit, damit die Eigenschaften in der Reihenfolge der Spezifikation
//! entstehen.
//!
//! Lesarten:
//!
//! - `localeCompare` vergleicht ohne ECMA-402 die kanonischen Zerlegungen
//!   (NFD) beider Zeichenketten Codepunkt für Codepunkt. Das ist eine
//!   Ordnung aller Zeichenketten, und kanonisch gleichwertige Zeichenketten
//!   ergeben 0, wie Abschnitt 22.1.3.12 es verlangt. Eine Sortierung nach
//!   den Gewohnheiten einer Sprache (UTS #10) ist das nicht.
//! - `toLocaleLowerCase` und `toLocaleUpperCase` rechnen ohne ECMA-402 wie
//!   `toLowerCase` und `toUpperCase`: mit den Abbildungen aus der
//!   Unicode-Zeichendatenbank, die nicht von der Sprache abhängen
//!   (Abschnitte 22.1.3.26 und 22.1.3.28).

use super::pattern;
use super::search::{index_of, last_index_of, matches_at};
use super::{
    argument_string, boolean, check_length, clamp_position, code_point_at, code_points, count,
    function, is_regexp, is_undefined, length_of, new_string, number, push_code_point, push_units,
    substring, this_string, undefined, unit_at,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall};
use crate::builtins::array::units_of;
use crate::conversion::{is_trimmed, to_integer_or_infinity, to_length, to_number, to_string};
use crate::heap::{Local, Scope};
use crate::number::integer_or_infinity;
use crate::value::Kind;
use mb_unicode::case_mapping::{to_lowercase, to_uppercase};
use mb_unicode::normalization::{compose, compose_compatible, decompose, decompose_compatible};
use std::cmp::Ordering;

/// Die Methoden in der Reihenfolge der Spezifikation (Abschnitte 22.1.3.1
/// bis 22.1.3.35).
pub(super) static FUNCTIONS: [Builtin; 34] = [
    function("at", 1, at),
    function("charAt", 1, char_at),
    function("charCodeAt", 1, char_code_at),
    function("codePointAt", 1, code_point_at_position),
    function("concat", 1, concat),
    function("endsWith", 1, ends_with),
    function("includes", 1, includes),
    function("indexOf", 1, index_of_string),
    function("isWellFormed", 0, is_well_formed),
    function("lastIndexOf", 1, last_index_of_string),
    function("localeCompare", 1, locale_compare),
    function("match", 1, pattern::match_pattern),
    function("matchAll", 1, pattern::match_all),
    function("normalize", 0, normalize),
    function("padEnd", 1, pad_end),
    function("padStart", 1, pad_start),
    function("repeat", 1, repeat),
    function("replace", 2, pattern::replace),
    function("replaceAll", 2, pattern::replace_all),
    function("search", 1, pattern::search),
    function("slice", 2, slice),
    function("split", 2, pattern::split),
    function("startsWith", 1, starts_with),
    function("substring", 2, substring_between),
    function("toLocaleLowerCase", 0, to_lower_case),
    function("toLocaleUpperCase", 0, to_upper_case),
    function("toLowerCase", 0, to_lower_case),
    function("toString", 0, to_string_value),
    function("toUpperCase", 0, to_upper_case),
    function("toWellFormed", 0, to_well_formed),
    function("trim", 0, trim),
    function("trimEnd", 0, trim_end),
    function("trimStart", 0, trim_start),
    function("valueOf", 0, to_string_value),
];

/// Ein relativer Index wie bei `slice`: negative zählen vom Ende, das
/// Ergebnis liegt zwischen 0 und `length`.
pub(super) fn relative_position(relative: f64, length: usize) -> usize {
    if relative < 0.0 {
        clamp_position(count(length) + relative, length)
    } else {
        clamp_position(relative, length)
    }
}

/// ToIntegerOrInfinity des Arguments `index`, zwischen 0 und `length`
/// eingeklemmt.
fn position_argument(
    agent: &mut Agent,
    scope: &Scope,
    arguments: &BuiltinCall<'_, '_>,
    index: usize,
    length: usize,
) -> Completion<usize> {
    let position = arguments.argument(agent, scope, index);
    let position = to_integer_or_infinity(agent, scope, position)?;
    Ok(clamp_position(position, length))
}

/// ToIntegerOrInfinity des ersten Arguments als Stelle in der Zeichenkette,
/// `None` außerhalb von ihr.
fn index_argument(
    agent: &mut Agent,
    scope: &Scope,
    arguments: &BuiltinCall<'_, '_>,
    length: usize,
) -> Completion<Option<usize>> {
    let position = arguments.argument(agent, scope, 0);
    let position = to_integer_or_infinity(agent, scope, position)?;
    let inside = position >= 0.0 && position < count(length);
    Ok(inside.then(|| clamp_position(position, length)))
}

/// Das erste Argument als gesuchte Zeichenkette für `endsWith`, `includes`
/// und `startsWith`: ein TypeError, wenn es nach IsRegExp ein regulärer
/// Ausdruck ist.
fn search_string(
    agent: &mut Agent,
    scope: &Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Vec<u16>> {
    let search = arguments.argument(agent, scope, 0);
    if is_regexp(agent, search)? {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let search = to_string(agent, scope, search)?;
    Ok(units_of(agent, search))
}

/// Eine Stelle oder -1 als Zahl.
fn found<'s>(agent: &Agent, scope: &'s Scope, index: Option<usize>) -> Local<'s> {
    number(agent, scope, index.map_or(-1.0, count))
}

// Einzelne Codeeinheiten

/// String.prototype.at ( index ) (Abschnitt 22.1.3.1).
fn at<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let index = arguments.argument(agent, scope, 0);
    let relative = to_integer_or_infinity(agent, scope, index)?;
    let k = if relative >= 0.0 {
        relative
    } else {
        count(length) + relative
    };
    if k < 0.0 || k >= count(length) {
        return Ok(undefined(agent, scope));
    }
    let k = clamp_position(k, length);
    substring(agent, scope, string, k, k.saturating_add(1))
}

/// String.prototype.charAt ( pos ) (Abschnitt 22.1.3.2).
fn char_at<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    match index_argument(agent, scope, arguments, length)? {
        Some(position) => substring(agent, scope, string, position, position.saturating_add(1)),
        None => new_string(agent, scope, &[]),
    }
}

/// String.prototype.charCodeAt ( pos ) (Abschnitt 22.1.3.3).
fn char_code_at<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let unit = index_argument(agent, scope, arguments, length)?
        .and_then(|position| unit_at(agent, string, position));
    Ok(number(agent, scope, unit.map_or(f64::NAN, f64::from)))
}

/// String.prototype.codePointAt ( pos ) (Abschnitt 22.1.3.4).
fn code_point_at_position<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let code_point = index_argument(agent, scope, arguments, length)?
        .and_then(|position| code_point_at(|index| unit_at(agent, string, index), position));
    Ok(match code_point {
        Some((code_point, _)) => number(agent, scope, f64::from(code_point)),
        None => undefined(agent, scope),
    })
}

// Verketten und Auffüllen

/// String.prototype.concat ( ...args ) (Abschnitt 22.1.3.5).
fn concat<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    if arguments.arguments.is_empty() {
        return Ok(string);
    }
    let mut result = units_of(agent, string);
    for &next in arguments.arguments {
        agent.scope(|agent, inner| -> Completion<()> {
            let next = to_string(agent, inner, next)?;
            push_units(agent, &mut result, &units_of(agent, next))
        })?;
    }
    new_string(agent, scope, &result)
}

/// Eine ganze Zahl von 0 an als Länge einer Zeichenkette: ein RangeError
/// über 2³² − 1.
fn string_length(agent: &mut Agent, length: f64) -> Completion<usize> {
    if length > f64::from(u32::MAX) {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    // Ganzzahlig und zwischen 0 und 2³² − 1, schneidet nicht ab.
    Ok(length as usize)
}

/// StringPaddingBuiltinsImpl (Abschnitt 22.1.3.17.1) mit StringPad
/// (Abschnitt 22.1.3.17.2); `at_start` steht für ~start~.
fn pad<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    at_start: bool,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let max_length = arguments.argument(agent, scope, 0);
    let max_length = to_length(agent, scope, max_length)?;
    let length = length_of(agent, string);
    if max_length <= count(length) {
        return Ok(string);
    }
    let fill = arguments.argument(agent, scope, 1);
    let filler = if is_undefined(agent, fill) {
        vec![0x20]
    } else {
        let fill = to_string(agent, scope, fill)?;
        units_of(agent, fill)
    };
    if filler.is_empty() {
        return Ok(string);
    }
    let total = string_length(agent, max_length)?;
    check_length(agent, total)?;
    let fill_length = total.saturating_sub(length);
    let padding = filler.iter().copied().cycle().take(fill_length);
    let mut result = Vec::with_capacity(total);
    if at_start {
        result.extend(padding);
        result.extend(units_of(agent, string));
    } else {
        result.extend(units_of(agent, string));
        result.extend(padding);
    }
    new_string(agent, scope, &result)
}

/// String.prototype.padEnd ( maxLength [ , fillString ] ) (Abschnitt
/// 22.1.3.16).
fn pad_end<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    pad(agent, scope, arguments, false)
}

/// String.prototype.padStart ( maxLength [ , fillString ] ) (Abschnitt
/// 22.1.3.17).
fn pad_start<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    pad(agent, scope, arguments, true)
}

/// String.prototype.repeat ( count ) (Abschnitt 22.1.3.18).
fn repeat<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let n = arguments.argument(agent, scope, 0);
    let n = to_integer_or_infinity(agent, scope, n)?;
    if n < 0.0 || n == f64::INFINITY {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let length = length_of(agent, string);
    if n == 0.0 || length == 0 {
        return new_string(agent, scope, &[]);
    }
    if n == 1.0 {
        return Ok(string);
    }
    // Das Produkt zweier ganzer Zahlen; über 2⁵³ ist es ungenau, aber
    // dann längst über der Grenze.
    let total = string_length(agent, count(length) * n)?;
    check_length(agent, total)?;
    let units = units_of(agent, string);
    let result = units.repeat(total / length);
    new_string(agent, scope, &result)
}

// Suchen

/// String.prototype.endsWith ( searchString [ , endPosition ] ) (Abschnitt
/// 22.1.3.7).
fn ends_with<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let search = search_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let end_position = arguments.argument(agent, scope, 1);
    let end = if is_undefined(agent, end_position) {
        length
    } else {
        position_argument(agent, scope, arguments, 1, length)?
    };
    let result = end
        .checked_sub(search.len())
        .is_some_and(|start| matches_at(|index| unit_at(agent, string, index), &search, start));
    Ok(boolean(agent, scope, result))
}

/// String.prototype.includes ( searchString [ , position ] ) (Abschnitt
/// 22.1.3.8).
fn includes<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let search = search_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let start = position_argument(agent, scope, arguments, 1, length)?;
    let index = index_of(
        |index| unit_at(agent, string, index),
        length,
        &search,
        start,
    );
    Ok(boolean(agent, scope, index.is_some()))
}

/// String.prototype.indexOf ( searchString [ , position ] ) (Abschnitt
/// 22.1.3.9).
fn index_of_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let search = argument_string(agent, scope, arguments, 0)?;
    let search = units_of(agent, search);
    let length = length_of(agent, string);
    let start = position_argument(agent, scope, arguments, 1, length)?;
    let index = index_of(
        |index| unit_at(agent, string, index),
        length,
        &search,
        start,
    );
    Ok(found(agent, scope, index))
}

/// String.prototype.lastIndexOf ( searchString [ , position ] ) (Abschnitt
/// 22.1.3.11).
fn last_index_of_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let search = argument_string(agent, scope, arguments, 0)?;
    let search = units_of(agent, search);
    let position = arguments.argument(agent, scope, 1);
    let position = to_number(agent, scope, position)?;
    let length = length_of(agent, string);
    let Some(max_start) = length.checked_sub(search.len()) else {
        return Ok(found(agent, scope, None));
    };
    let start = if position.is_nan() {
        max_start
    } else {
        clamp_position(integer_or_infinity(position), max_start)
    };
    let index = last_index_of(
        |index| unit_at(agent, string, index),
        length,
        &search,
        start,
    );
    Ok(found(agent, scope, index))
}

/// String.prototype.startsWith ( searchString [ , position ] ) (Abschnitt
/// 22.1.3.24).
fn starts_with<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let search = search_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let start = position_argument(agent, scope, arguments, 1, length)?;
    let fits = start
        .checked_add(search.len())
        .is_some_and(|end| end <= length);
    let result = fits && matches_at(|index| unit_at(agent, string, index), &search, start);
    Ok(boolean(agent, scope, result))
}

// Teile

/// String.prototype.slice ( start, end ) (Abschnitt 22.1.3.22).
fn slice<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let start = arguments.argument(agent, scope, 0);
    let start = to_integer_or_infinity(agent, scope, start)?;
    let from = relative_position(start, length);
    let end = arguments.argument(agent, scope, 1);
    let to = if is_undefined(agent, end) {
        length
    } else {
        let end = to_integer_or_infinity(agent, scope, end)?;
        relative_position(end, length)
    };
    if from >= to {
        return new_string(agent, scope, &[]);
    }
    substring(agent, scope, string, from, to)
}

/// String.prototype.substring ( start, end ) (Abschnitt 22.1.3.25).
fn substring_between<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let final_start = position_argument(agent, scope, arguments, 0, length)?;
    let end = arguments.argument(agent, scope, 1);
    let final_end = if is_undefined(agent, end) {
        length
    } else {
        position_argument(agent, scope, arguments, 1, length)?
    };
    let (from, to) = (final_start.min(final_end), final_start.max(final_end));
    substring(agent, scope, string, from, to)
}

/// Die Stelle des ersten Zeichens, das kein Leerraum ist, und die nach dem
/// letzten. Für nur Leerraum sind es die Länge und 0, damit an jedem Ende
/// für sich alles wegfällt.
fn trimmed_range(agent: &Agent, string: Local<'_>) -> (usize, usize) {
    let length = length_of(agent, string);
    let space = |index: &usize| unit_at(agent, string, *index).is_some_and(is_trimmed);
    let start = (0..length).find(|index| !space(index)).unwrap_or(length);
    let end = (start..length)
        .rev()
        .find(|index| !space(index))
        .map_or(0, |last| last.saturating_add(1));
    (start, end)
}

/// TrimString (Abschnitt 22.1.3.32.1): `start` und `end` sagen, an welchem
/// Ende der Leerraum wegfällt.
fn trim_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    start: bool,
    end: bool,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let (first, last) = trimmed_range(agent, string);
    let from = if start { first } else { 0 };
    let to = if end { last } else { length_of(agent, string) };
    substring(agent, scope, string, from, to.max(from))
}

/// String.prototype.trim ( ) (Abschnitt 22.1.3.32).
fn trim<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    trim_string(agent, scope, arguments, true, true)
}

/// String.prototype.trimEnd ( ) (Abschnitt 22.1.3.33).
fn trim_end<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    trim_string(agent, scope, arguments, false, true)
}

/// String.prototype.trimStart ( ) (Abschnitt 22.1.3.34).
fn trim_start<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    trim_string(agent, scope, arguments, true, false)
}

// Unicode

/// Ob jede Codeeinheit zu ASCII gehört.
fn is_ascii(units: &[u16]) -> bool {
    units.iter().all(|&unit| unit < 0x80)
}

/// Die Codeeinheiten zu einer Folge von Codepunkten.
fn encode(code_points: &[u32]) -> Vec<u16> {
    let mut units = Vec::with_capacity(code_points.len());
    for &code_point in code_points {
        push_code_point(&mut units, code_point);
    }
    units
}

/// String.prototype.isWellFormed ( ) (Abschnitt 22.1.3.10).
fn is_well_formed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let length = length_of(agent, string);
    let mut position = 0;
    let mut well_formed = true;
    while position < length {
        let Some((code_point, width)) =
            code_point_at(|index| unit_at(agent, string, index), position)
        else {
            break;
        };
        if (0xD800..=0xDFFF).contains(&code_point) {
            well_formed = false;
            break;
        }
        position = position.saturating_add(width);
    }
    Ok(boolean(agent, scope, well_formed))
}

/// String.prototype.toWellFormed ( ) (Abschnitt 22.1.3.31).
fn to_well_formed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let units = units_of(agent, string);
    let code_points = code_points(&units);
    if !code_points
        .iter()
        .any(|code_point| (0xD800..=0xDFFF).contains(code_point))
    {
        return Ok(string);
    }
    let replaced: Vec<u32> = code_points
        .into_iter()
        .map(|code_point| match code_point {
            0xD800..=0xDFFF => 0xFFFD,
            other => other,
        })
        .collect();
    new_string(agent, scope, &encode(&replaced))
}

/// String.prototype.localeCompare ( that [ , reserved1 [ , reserved2 ] ] )
/// (Abschnitt 22.1.3.12), siehe die Lesart oben.
fn locale_compare<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let that = argument_string(agent, scope, arguments, 0)?;
    let (string, that) = (units_of(agent, string), units_of(agent, that));
    // Unter U+00C0 hat kein Zeichen eine Zerlegung oder eine
    // Kombinationsklasse; dort ordnen die Codeeinheiten wie die
    // Codepunkte.
    let plain = |units: &[u16]| units.iter().all(|&unit| unit < 0xC0);
    let order = if plain(&string) && plain(&that) {
        string.cmp(&that)
    } else {
        decompose(code_points(&string)).cmp(&decompose(code_points(&that)))
    };
    let order = match order {
        Ordering::Less => -1.0,
        Ordering::Equal => 0.0,
        Ordering::Greater => 1.0,
    };
    Ok(number(agent, scope, order))
}

/// String.prototype.normalize ( [ form ] ) (Abschnitt 22.1.3.15).
fn normalize<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let form = arguments.argument(agent, scope, 0);
    let form = if is_undefined(agent, form) {
        "NFC".to_owned()
    } else {
        let form = to_string(agent, scope, form)?;
        // Nur die vier Namen gelten; ein einzelnes Surrogat ist keiner.
        String::from_utf16(&units_of(agent, form)).unwrap_or_default()
    };
    let normal: fn(Vec<u32>) -> Vec<u32> = match form.as_str() {
        "NFC" => |input| compose(input),
        "NFD" => |input| decompose(input),
        "NFKC" => |input| compose_compatible(input),
        "NFKD" => |input| decompose_compatible(input),
        _ => return Err(agent.throw_error(ErrorKind::RangeError)),
    };
    let units = units_of(agent, string);
    // ASCII ist in allen vier Formen normalisiert.
    if is_ascii(&units) {
        return Ok(string);
    }
    let normalized = encode(&normal(code_points(&units)));
    new_string(agent, scope, &normalized)
}

/// String.prototype.toLowerCase ( ) (Abschnitt 22.1.3.28) und
/// String.prototype.toLocaleLowerCase (Abschnitt 22.1.3.26).
fn to_lower_case<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let units = units_of(agent, string);
    let lower = if is_ascii(&units) {
        units
            .iter()
            .map(|&unit| match unit {
                0x41..=0x5A => unit + 0x20,
                _ => unit,
            })
            .collect()
    } else {
        encode(&to_lowercase(&code_points(&units)))
    };
    new_string(agent, scope, &lower)
}

/// String.prototype.toUpperCase ( ) (Abschnitt 22.1.3.30) und
/// String.prototype.toLocaleUpperCase (Abschnitt 22.1.3.27).
fn to_upper_case<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let units = units_of(agent, string);
    let upper = if is_ascii(&units) {
        units
            .iter()
            .map(|&unit| match unit {
                0x61..=0x7A => unit - 0x20,
                _ => unit,
            })
            .collect()
    } else {
        encode(&to_uppercase(&code_points(&units)))
    };
    new_string(agent, scope, &upper)
}

// Der Wert selbst

/// String.prototype.toString ( ) (Abschnitt 22.1.3.29) und
/// String.prototype.valueOf ( ) (Abschnitt 22.1.3.35): ThisStringValue
/// (Abschnitt 22.1.3.35.1).
fn to_string_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let this = heap.get(arguments.this);
    let string = match this.kind() {
        Kind::String(_) => Some(this),
        Kind::Object(object) => heap
            .primitive_data(object)
            .filter(|data| matches!(data.kind(), Kind::String(_))),
        _ => None,
    };
    match string {
        Some(string) => Ok(heap.local(scope, string)),
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}
