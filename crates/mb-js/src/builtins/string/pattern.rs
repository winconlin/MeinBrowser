//! Die Methoden von %String.prototype%, die einem Muster folgen: `match`,
//! `matchAll`, `replace`, `replaceAll`, `search` und `split` (Abschnitte
//! 22.1.3.13, 22.1.3.14, 22.1.3.19 bis 22.1.3.21 und 22.1.3.23), dazu
//! GetSubstitution (Abschnitt 22.1.3.19.1).
//!
//! Ist das Muster ein Objekt mit der passenden Methode unter @@match,
//! @@matchAll, @@replace, @@search oder @@split, ruft die Methode sie auf.
//! Sonst suchen `replace`, `replaceAll` und `split` nach einer
//! Zeichenkette. `match`, `matchAll` und `search` bräuchten dafür
//! RegExpCreate; bis die Engine für reguläre Ausdrücke da ist, enden sie
//! dort als nicht unterstützt ([`Abrupt::Unsupported`]), nicht mit einem
//! Fehler, den ein Script fangen könnte.
//!
//! `replaceAll` sammelt die Fundstellen nicht erst in einer Liste, sondern
//! sucht die nächste, nachdem die vorige ersetzt ist. Die Zeichenketten
//! ändern sich dabei nicht, von außen ist das also dasselbe. Jede
//! Fundstelle und jedes Element von `split` kostet einen Arbeitsschritt.

use super::search::index_of;
use super::{
    is_object, is_regexp, is_undefined, new_string, number, push_units, require_object_coercible,
    undefined,
};
use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::builtin::BuiltinCall;
use crate::builtins::array::{step, units_of};
use crate::builtins::units;
use crate::conversion::{to_string, to_uint32};
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::operations::{
    call, create_array_from_list, create_data_property_or_throw, get, get_method, is_callable,
};

const DOLLAR: u16 = 0x24;
const AMPERSAND: u16 = 0x26;
const APOSTROPHE: u16 = 0x27;
const LESS_THAN: u16 = 0x3C;
const GREATER_THAN: u16 = 0x3E;
const GRAVE_ACCENT: u16 = 0x60;

/// Der gemeinsame Anfang der sechs Methoden: Ist `pattern` ein Objekt mit
/// einer Methode unter `symbol`, ihr Ergebnis für `arguments`, sonst
/// `None`.
fn dispatch<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    pattern: Local<'_>,
    symbol: WellKnownSymbol,
    arguments: &[Local<'_>],
) -> Completion<Option<Local<'s>>> {
    if !is_object(agent, pattern) {
        return Ok(None);
    }
    let key = PropertyKey::Symbol(agent.well_known_symbol(scope, symbol));
    let Some(method) = get_method(agent, scope, pattern, key)? else {
        return Ok(None);
    };
    call(agent, scope, method, pattern, arguments).map(Some)
}

/// Die Prüfung von `matchAll` und `replaceAll`: Ein regulärer Ausdruck
/// nach IsRegExp muss in `flags` ein `g` haben.
fn require_global(agent: &mut Agent, pattern: Local<'_>) -> Completion<()> {
    if !is_regexp(agent, pattern)? {
        return Ok(());
    }
    agent.scope(|agent, scope| {
        let key = agent.key_from_units(scope, &units("flags"))?;
        let flags = get(agent, scope, pattern, key)?;
        require_object_coercible(agent, flags)?;
        let flags = to_string(agent, scope, flags)?;
        if !units_of(agent, flags).contains(&u16::from(b'g')) {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        Ok(())
    })
}

/// Der Rest von `match`, `matchAll` und `search` für ein Muster ohne
/// eigene Methode: ToString(*this*), dann RegExpCreate, das noch fehlt.
fn regexp_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    to_string(agent, scope, arguments.this)?;
    Err(Abrupt::Unsupported(0))
}

/// String.prototype.match ( regexp ) (Abschnitt 22.1.3.13).
pub(super) fn match_pattern<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_object_coercible(agent, arguments.this)?;
    let regexp = arguments.argument(agent, scope, 0);
    let this = [arguments.this];
    if let Some(result) = dispatch(agent, scope, regexp, WellKnownSymbol::Match, &this)? {
        return Ok(result);
    }
    regexp_create(agent, scope, arguments)
}

/// String.prototype.matchAll ( regexp ) (Abschnitt 22.1.3.14).
pub(super) fn match_all<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_object_coercible(agent, arguments.this)?;
    let regexp = arguments.argument(agent, scope, 0);
    require_global(agent, regexp)?;
    let this = [arguments.this];
    if let Some(result) = dispatch(agent, scope, regexp, WellKnownSymbol::MatchAll, &this)? {
        return Ok(result);
    }
    regexp_create(agent, scope, arguments)
}

/// String.prototype.search ( regexp ) (Abschnitt 22.1.3.21).
pub(super) fn search<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_object_coercible(agent, arguments.this)?;
    let regexp = arguments.argument(agent, scope, 0);
    let this = [arguments.this];
    if let Some(result) = dispatch(agent, scope, regexp, WellKnownSymbol::Search, &this)? {
        return Ok(result);
    }
    regexp_create(agent, scope, arguments)
}

/// Was GetSubstitution über einen Fund weiß.
pub(crate) struct Match<'a, 'l> {
    /// Der gefundene Teil.
    pub matched: &'a [u16],
    /// Die ganze durchsuchte Zeichenkette.
    pub string: &'a [u16],
    /// Die Stelle des Funds in ihr.
    pub position: usize,
    /// Die Gruppen des Musters ab der ersten, `None` für *undefined*.
    pub captures: &'a [Option<Vec<u16>>],
    /// Das Objekt mit den benannten Gruppen.
    pub named_captures: Option<Local<'l>>,
}

/// Der Wert einer Dezimalziffer.
fn digit(unit: Option<&u16>) -> Option<usize> {
    let unit = *unit?;
    (0x30..=0x39)
        .contains(&unit)
        .then(|| usize::from(unit - 0x30))
}

/// GetSubstitution (Abschnitt 22.1.3.19.1): hängt den Ersatz für `found`
/// nach der Vorlage `template` an `result` an.
pub(crate) fn get_substitution(
    agent: &mut Agent,
    found: &Match<'_, '_>,
    template: &[u16],
    result: &mut Vec<u16>,
) -> Completion<()> {
    let mut rest = template;
    while !rest.is_empty() {
        // Alles bis zum nächsten Dollarzeichen bleibt, wie es ist.
        let plain = rest
            .iter()
            .position(|&unit| unit == DOLLAR)
            .unwrap_or(rest.len());
        if plain > 0 {
            push_units(agent, result, rest.get(..plain).unwrap_or_default())?;
            rest = rest.get(plain..).unwrap_or_default();
            continue;
        }
        // `rest` beginnt mit einem Dollarzeichen; `consumed` zählt, wie
        // viele Codeeinheiten der Verweis belegt.
        let consumed = match rest.get(1) {
            Some(&DOLLAR) => {
                push_units(agent, result, &[DOLLAR])?;
                2
            }
            Some(&GRAVE_ACCENT) => {
                let preceding = found.string.get(..found.position).unwrap_or_default();
                push_units(agent, result, preceding)?;
                2
            }
            Some(&AMPERSAND) => {
                push_units(agent, result, found.matched)?;
                2
            }
            Some(&APOSTROPHE) => {
                // Die Stelle hinter dem Fund kann nur hinter dem Ende
                // liegen, wenn ein fremdes `exec` den Fund geliefert hat.
                let tail = found
                    .position
                    .saturating_add(found.matched.len())
                    .min(found.string.len());
                push_units(agent, result, found.string.get(tail..).unwrap_or_default())?;
                2
            }
            Some(&LESS_THAN) => named_reference(agent, found, rest, result)?,
            first => match digit(first) {
                Some(first) => numbered_reference(agent, found, rest, first, result)?,
                None => {
                    push_units(agent, result, &[DOLLAR])?;
                    1
                }
            },
        };
        rest = rest.get(consumed..).unwrap_or_default();
    }
    Ok(())
}

/// Ein Verweis `$n` oder `$nn` am Anfang von `rest`, dessen erste Ziffer
/// den Wert `first` hat. Gibt zurück, wie viele Codeeinheiten er belegt.
fn numbered_reference(
    agent: &mut Agent,
    found: &Match<'_, '_>,
    rest: &[u16],
    first: usize,
    result: &mut Vec<u16>,
) -> Completion<usize> {
    let count = found.captures.len();
    // Zwei Ziffern gelten nur, wenn es so viele Gruppen gibt; sonst ist
    // die zweite Ziffer Text.
    let (index, digits) = match digit(rest.get(2)) {
        Some(second) if first * 10 + second <= count => (first * 10 + second, 2),
        _ => (first, 1),
    };
    let reference = rest.get(..1 + digits).unwrap_or_default();
    match index.checked_sub(1).and_then(|at| found.captures.get(at)) {
        Some(Some(capture)) => push_units(agent, result, capture)?,
        Some(None) => {}
        // Keine solche Gruppe: Der Verweis bleibt stehen.
        None => push_units(agent, result, reference)?,
    }
    Ok(reference.len())
}

/// Ein Verweis `$<name>` am Anfang von `rest`. Gibt zurück, wie viele
/// Codeeinheiten er belegt.
fn named_reference(
    agent: &mut Agent,
    found: &Match<'_, '_>,
    rest: &[u16],
    result: &mut Vec<u16>,
) -> Completion<usize> {
    let end = rest.iter().position(|&unit| unit == GREATER_THAN);
    let (Some(end), Some(named_captures)) = (end, found.named_captures) else {
        push_units(agent, result, &[DOLLAR, LESS_THAN])?;
        return Ok(2);
    };
    let group_name = rest.get(2..end).unwrap_or_default();
    agent.scope(|agent, scope| -> Completion<()> {
        let key = agent.key_from_units(scope, group_name)?;
        let capture = get(agent, scope, named_captures, key)?;
        if is_undefined(agent, capture) {
            return Ok(());
        }
        let capture = to_string(agent, scope, capture)?;
        push_units(agent, result, &units_of(agent, capture))
    })?;
    Ok(end.saturating_add(1))
}

/// Was `replace` und `replaceAll` für jeden Fund brauchen.
struct Replacement<'a, 'l> {
    string: Local<'l>,
    string_units: &'a [u16],
    search_string: Local<'l>,
    search: &'a [u16],
    /// Die Funktion oder die Vorlage als Zeichenkette.
    replace_value: Local<'l>,
    /// Die Vorlage, `None` für eine Funktion.
    template: Option<Vec<u16>>,
}

impl Replacement<'_, '_> {
    /// Die nächste Fundstelle ab `from`.
    fn find(&self, from: usize) -> Option<usize> {
        index_of(
            |index| self.string_units.get(index).copied(),
            self.string_units.len(),
            self.search,
            from,
        )
    }

    /// Hängt den Ersatz für den Fund an der Stelle `position` an.
    fn push(&self, agent: &mut Agent, position: usize, result: &mut Vec<u16>) -> Completion<()> {
        let Some(template) = &self.template else {
            return agent.scope(|agent, scope| {
                let position = number(agent, scope, super::count(position));
                let this = undefined(agent, scope);
                let arguments = [self.search_string, position, self.string];
                let replacement = call(agent, scope, self.replace_value, this, &arguments)?;
                let replacement = to_string(agent, scope, replacement)?;
                push_units(agent, result, &units_of(agent, replacement))
            });
        };
        let found = Match {
            matched: self.search,
            string: self.string_units,
            position,
            captures: &[],
            named_captures: None,
        };
        get_substitution(agent, &found, template, result)
    }
}

/// Die gemeinsamen Schritte von `replace` und `replaceAll` nach dem Aufruf
/// von @@replace: die Umwandlungen und die Schleife über die Funde, bei
/// `replace` nur über den ersten.
fn replace_string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    all: bool,
) -> Completion<Local<'s>> {
    let string = to_string(agent, scope, arguments.this)?;
    let search_value = arguments.argument(agent, scope, 0);
    let search_string = to_string(agent, scope, search_value)?;
    let mut replace_value = arguments.argument(agent, scope, 1);
    let functional = is_callable(agent.heap(), agent.heap().get(replace_value));
    if !functional {
        replace_value = to_string(agent, scope, replace_value)?;
    }
    let (string_units, search) = (units_of(agent, string), units_of(agent, search_string));
    let replacement = Replacement {
        string,
        string_units: &string_units,
        search_string,
        search: &search,
        replace_value,
        template: (!functional).then(|| units_of(agent, replace_value)),
    };
    let Some(first) = replacement.find(0) else {
        return Ok(string);
    };
    let advance_by = search.len().max(1);
    let mut end_of_last_match = 0;
    let mut result = Vec::new();
    let mut position = Some(first);
    while let Some(found) = position {
        step(agent)?;
        let preserved = string_units
            .get(end_of_last_match..found)
            .unwrap_or_default();
        push_units(agent, &mut result, preserved)?;
        replacement.push(agent, found, &mut result)?;
        end_of_last_match = found.saturating_add(search.len());
        position = match all {
            true => replacement.find(found.saturating_add(advance_by)),
            false => None,
        };
    }
    let following = string_units.get(end_of_last_match..).unwrap_or_default();
    push_units(agent, &mut result, following)?;
    new_string(agent, scope, &result)
}

/// String.prototype.replace ( searchValue, replaceValue ) (Abschnitt
/// 22.1.3.19).
pub(super) fn replace<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_object_coercible(agent, arguments.this)?;
    let search_value = arguments.argument(agent, scope, 0);
    let replace_value = arguments.argument(agent, scope, 1);
    let passed = [arguments.this, replace_value];
    let replaced = dispatch(
        agent,
        scope,
        search_value,
        WellKnownSymbol::Replace,
        &passed,
    )?;
    if let Some(result) = replaced {
        return Ok(result);
    }
    replace_string(agent, scope, arguments, false)
}

/// String.prototype.replaceAll ( searchValue, replaceValue ) (Abschnitt
/// 22.1.3.20).
pub(super) fn replace_all<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_object_coercible(agent, arguments.this)?;
    let search_value = arguments.argument(agent, scope, 0);
    let replace_value = arguments.argument(agent, scope, 1);
    require_global(agent, search_value)?;
    let passed = [arguments.this, replace_value];
    let replaced = dispatch(
        agent,
        scope,
        search_value,
        WellKnownSymbol::Replace,
        &passed,
    )?;
    if let Some(result) = replaced {
        return Ok(result);
    }
    replace_string(agent, scope, arguments, true)
}

/// Legt `units` als Zeichenkette unter dem Index `index` in `array` ab.
fn push_element(
    agent: &mut Agent,
    array: Local<'_>,
    index: usize,
    units: &[u16],
) -> Completion<()> {
    // Mehr Elemente als Indizes bis 2³² − 2 kann kein Array halten.
    let Some(index) = u32::try_from(index).ok().filter(|&index| index < u32::MAX) else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    agent.scope(|agent, scope| {
        let element = new_string(agent, scope, units)?;
        create_data_property_or_throw(agent, scope, array, PropertyKey::Index(index), element)
    })
}

/// String.prototype.split ( separator, limit ) (Abschnitt 22.1.3.23).
pub(super) fn split<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_object_coercible(agent, arguments.this)?;
    let separator = arguments.argument(agent, scope, 0);
    let limit = arguments.argument(agent, scope, 1);
    let passed = [arguments.this, limit];
    if let Some(result) = dispatch(agent, scope, separator, WellKnownSymbol::Split, &passed)? {
        return Ok(result);
    }
    let string = to_string(agent, scope, arguments.this)?;
    let limit = if is_undefined(agent, limit) {
        u32::MAX
    } else {
        to_uint32(agent, scope, limit)?
    };
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let separator_string = to_string(agent, scope, separator)?;
    let array = create_array_from_list(agent, scope, arguments.realm, &[])?;
    if limit == 0 {
        return Ok(array);
    }
    let units = units_of(agent, string);
    if is_undefined(agent, separator) {
        push_element(agent, array, 0, &units)?;
        return Ok(array);
    }
    let separator = units_of(agent, separator_string);
    if separator.is_empty() {
        // Jede Codeeinheit für sich, höchstens `limit` davon.
        for (index, &unit) in units.iter().take(limit).enumerate() {
            step(agent)?;
            push_element(agent, array, index, &[unit])?;
        }
        return Ok(array);
    }
    if units.is_empty() {
        push_element(agent, array, 0, &units)?;
        return Ok(array);
    }
    let find = |from: usize| {
        index_of(
            |index| units.get(index).copied(),
            units.len(),
            &separator,
            from,
        )
    };
    let mut count = 0;
    let mut search_start = 0;
    let mut match_index = find(0);
    while let Some(found) = match_index {
        step(agent)?;
        let part = units.get(search_start..found).unwrap_or_default();
        push_element(agent, array, count, part)?;
        count += 1;
        if count == limit {
            return Ok(array);
        }
        search_start = found.saturating_add(separator.len());
        match_index = find(search_start);
    }
    let part = units.get(search_start..).unwrap_or_default();
    push_element(agent, array, count, part)?;
    Ok(array)
}
