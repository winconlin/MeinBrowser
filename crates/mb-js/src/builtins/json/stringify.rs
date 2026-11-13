//! JSON.stringify ( value [ , replacer [ , space ] ] ) (Abschnitt 25.5.4)
//! mit SerializeJSONProperty, QuoteJSONString, SerializeJSONObject und
//! SerializeJSONArray (Abschnitte 25.5.4.2 bis 25.5.4.6).
//!
//! Die Spezifikation beschreibt das Serialisieren rekursiv; hier läuft es
//! mit einem Stapel offener Objekte und Arrays, der zugleich [[Stack]] für
//! die Erkennung von Zyklen ist. Jede Eigenschaft kostet einen
//! Arbeitsschritt und hat einen eigenen Bereich für Handles. Die Ausgabe
//! wächst in Rust-Puffern; wird sie länger als das Speicherbudget des
//! Heaps in Bytes, passte sie auch als Latin-1-Zeichenkette nicht hinein,
//! und das Serialisieren endet mit [`Abrupt::OutOfMemory`].

use super::super::units;
use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::builtin::BuiltinCall;
use crate::compare::same_value;
use crate::conversion::{to_integer_or_infinity, to_number, to_string};
use crate::heap::{Local, Scope};
use crate::key::PropertyKey;
use crate::number;
use crate::object::ordinary_object_create;
use crate::operations::{
    EnumerableKind, call, create_data_property_or_throw, enumerable_own_properties, get, is_array,
    is_callable, length_of_array_like,
};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Was SerializeJSONProperty für eine Eigenschaft ergab.
enum Serialized<'s> {
    /// Nichts: *undefined*.
    Nothing,
    /// Ein fertiger Text.
    Text(Vec<u16>),
    /// Ein Array oder Objekt, das noch zu serialisieren ist.
    Array(Local<'s>),
    Object(Local<'s>),
}

/// Ein offenes Array oder Objekt.
struct Open<'s> {
    /// Das Handle der Tiefe, in der es offen ist (siehe [`slot`]).
    value: Local<'s>,
    /// Die Schlüssel von SerializeJSONObject; für ein Array leer.
    keys: Vec<Vec<u16>>,
    /// Die Länge eines Arrays.
    length: f64,
    is_array: bool,
    /// Der nächste Index oder Schlüssel.
    next: usize,
    /// Die fertigen Teile, *partial*.
    partial: Vec<Vec<u16>>,
    /// Der Schlüssel, unter dem das Objekt im umgebenden Objekt steht.
    key: Vec<u16>,
}

/// Der JSON Serialization Record (Abschnitt 25.5.4.1) ohne [[Stack]].
struct State<'s> {
    replacer: Option<Local<'s>>,
    property_list: Option<Vec<Vec<u16>>>,
    gap: Vec<u16>,
    indent: Vec<u16>,
    realm: RealmId,
    /// Die Grenze für die Länge der Ausgabe in Codeeinheiten.
    limit: usize,
}

pub(super) fn stringify<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let value = arguments.argument(agent, scope, 0);
    let replacer = arguments.argument(agent, scope, 1);
    let mut space = arguments.argument(agent, scope, 2);
    let mut replacer_function = None;
    let mut property_list = None;
    if is_object(agent, replacer) {
        if is_callable(agent.heap(), agent.heap().get(replacer)) {
            replacer_function = Some(replacer);
        } else if is_array(agent, replacer)? {
            property_list = Some(replacer_property_list(agent, scope, replacer)?);
        }
    }
    match primitive_data(agent, space).map(Value::kind) {
        Some(Kind::Number(_)) => {
            let number = to_number(agent, scope, space)?;
            space = agent.heap().local(scope, Value::number(number));
        }
        Some(Kind::String(_)) => space = to_string(agent, scope, space)?,
        _ => {}
    }
    let gap = match agent.heap().get(space).kind() {
        Kind::Number(_) => {
            let count = to_integer_or_infinity(agent, scope, space)?.min(10.0);
            // Zwischen 1 und 10 ist die Zahl ganz; `as` schneidet nichts ab.
            let count = if count < 1.0 { 0 } else { count as usize };
            vec![0x20; count]
        }
        Kind::String(string) => {
            let mut units = agent.heap().string_units(string);
            units.truncate(10);
            units
        }
        _ => Vec::new(),
    };
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let wrapper = ordinary_object_create(agent, scope, Some(prototype))?;
    let key = agent.key_from_units(scope, &[])?;
    create_data_property_or_throw(agent, scope, wrapper, key, value)?;
    let mut state = State {
        replacer: replacer_function,
        property_list,
        gap,
        indent: Vec::new(),
        realm,
        limit: agent.heap().limit(),
    };
    match serialize(agent, scope, &mut state, wrapper)? {
        Some(text) => Ok(agent.heap_mut().alloc_string(scope, &text)?),
        None => Ok(agent.heap().local(scope, Value::UNDEFINED)),
    }
}

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

/// Der primitive Wert eines Boolean-, Number-, String- oder
/// Symbol-Objekts, sonst `None`.
fn primitive_data<'h>(agent: &'h Agent, value: Local<'_>) -> Option<Value<'h>> {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::Object(object) => heap.primitive_data(object),
        _ => None,
    }
}

/// Schritt 5.b.ii von JSON.stringify: die Schlüssel aus einem Array als
/// *replacer*, ohne Doppelte.
fn replacer_property_list(
    agent: &mut Agent,
    scope: &Scope,
    replacer: Local<'_>,
) -> Completion<Vec<Vec<u16>>> {
    let length = length_of_array_like(agent, scope, replacer)?;
    let mut list: Vec<Vec<u16>> = Vec::new();
    let mut index = 0.0;
    while index < length {
        agent.spend_execution_steps(1)?;
        let item = agent.scope(|agent, inner| -> Completion<Option<Vec<u16>>> {
            let key = index_key(agent, inner, index)?;
            let value = get(agent, inner, replacer, key)?;
            let heap = agent.heap();
            let item = match heap.get(value).kind() {
                Kind::String(_) | Kind::Number(_) => Some(to_string(agent, inner, value)?),
                Kind::Object(_) => match primitive_data(agent, value).map(Value::kind) {
                    Some(Kind::String(_) | Kind::Number(_)) => {
                        Some(to_string(agent, inner, value)?)
                    }
                    _ => None,
                },
                _ => None,
            };
            Ok(item.map(|item| string_units(agent, item)))
        })?;
        if let Some(item) = item
            && !list.contains(&item)
        {
            list.push(item);
        }
        index += 1.0;
    }
    Ok(list)
}

fn string_units(agent: &Agent, value: Local<'_>) -> Vec<u16> {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => Vec::new(),
    }
}

/// Der Schlüssel zum Index `index`, einer ganzen Zahl unter 2⁵³.
fn index_key<'s>(agent: &mut Agent, scope: &'s Scope, index: f64) -> Completion<PropertyKey<'s>> {
    let text = units(&number::to_string(index));
    Ok(agent.key_from_units(scope, &text)?)
}

/// Das Handle für das Array oder Objekt, das in der Tiefe `depth` offen
/// ist. Jede Tiefe hat ein eigenes, das jedes neue Array oder Objekt dort
/// überschreibt; so hält `stringify` nur die offenen fest und nicht alle,
/// die es schon serialisiert hat.
fn slot<'s>(
    agent: &Agent,
    scope: &'s Scope,
    slots: &mut Vec<Local<'s>>,
    depth: usize,
) -> Local<'s> {
    if let Some(&slot) = slots.get(depth) {
        return slot;
    }
    let slot = agent.heap().local(scope, Value::UNDEFINED);
    slots.push(slot);
    slot
}

/// SerializeJSONProperty(state, "", wrapper) ohne Rekursion.
fn serialize<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    state: &mut State<'_>,
    wrapper: Local<'_>,
) -> Completion<Option<Vec<u16>>> {
    let mut stack: Vec<Open<'s>> = Vec::new();
    let mut slots: Vec<Local<'s>> = Vec::new();
    let root = slot(agent, scope, &mut slots, 0);
    let first = property(agent, state, wrapper, &[], root)?;
    // Was ein fertiges Array oder Objekt dem umgebenden übergibt.
    let mut finished: Option<(Vec<u16>, Option<Vec<u16>>)> = None;
    match first {
        Serialized::Nothing => return Ok(None),
        Serialized::Text(text) => return Ok(Some(text)),
        Serialized::Array(value) | Serialized::Object(value) => {
            let is_array = matches!(first, Serialized::Array(_));
            open(agent, state, &mut stack, value, is_array, Vec::new())?;
        }
    }
    loop {
        if let Some((key, text)) = finished.take() {
            let Some(top) = stack.last_mut() else {
                return Ok(text);
            };
            add(state, top, &key, text)?;
        }
        let Some(top) = stack.last_mut() else {
            return Ok(None);
        };
        let key = if top.is_array {
            // Indizes unter 2⁵³ sind als f64 genau.
            let index = top.next as f64;
            if index >= top.length {
                None
            } else {
                Some(units(&number::to_string(index)))
            }
        } else {
            top.keys.get(top.next).cloned()
        };
        let Some(key) = key else {
            let Some(done) = stack.pop() else {
                return Ok(None);
            };
            agent.heap().set(done.value, Value::UNDEFINED);
            let text = close(state, &done)?;
            finished = Some((done.key, Some(text)));
            continue;
        };
        top.next = top.next.saturating_add(1);
        let holder = top.value;
        agent.spend_execution_steps(1)?;
        let child = slot(agent, scope, &mut slots, stack.len());
        let result = property(agent, state, holder, &key, child)?;
        match result {
            Serialized::Nothing => finished = Some((key, None)),
            Serialized::Text(text) => finished = Some((key, Some(text))),
            Serialized::Array(value) => open(agent, state, &mut stack, value, true, key)?,
            Serialized::Object(value) => {
                open(agent, state, &mut stack, value, false, key)?;
            }
        }
    }
}

/// Fügt das Ergebnis für `key` zu *partial* des offenen Arrays oder
/// Objekts hinzu.
fn add(
    state: &State<'_>,
    top: &mut Open<'_>,
    key: &[u16],
    text: Option<Vec<u16>>,
) -> Completion<()> {
    if top.is_array {
        top.partial.push(text.unwrap_or_else(|| units("null")));
        return Ok(());
    }
    let Some(text) = text else {
        return Ok(());
    };
    let mut member = quote(key);
    member.push(0x3A);
    if !state.gap.is_empty() {
        member.push(0x20);
    }
    member.extend_from_slice(&text);
    if member.len() > state.limit {
        return Err(Abrupt::OutOfMemory);
    }
    top.partial.push(member);
    Ok(())
}

/// Die ersten Schritte von SerializeJSONObject und SerializeJSONArray:
/// Zyklen erkennen, die Einrückung vertiefen und die Schlüssel oder die
/// Länge holen.
fn open<'s>(
    agent: &mut Agent,
    state: &mut State<'_>,
    stack: &mut Vec<Open<'s>>,
    value: Local<'s>,
    is_array: bool,
    key: Vec<u16>,
) -> Completion<()> {
    let heap = agent.heap();
    if stack
        .iter()
        .any(|open| same_value(heap, heap.get(open.value), heap.get(value)))
    {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    state.indent.extend_from_slice(&state.gap);
    let mut entry = Open {
        value,
        keys: Vec::new(),
        length: 0.0,
        is_array,
        next: 0,
        partial: Vec::new(),
        key,
    };
    if is_array {
        entry.length = agent.scope(|agent, inner| length_of_array_like(agent, inner, value))?;
    } else if let Some(list) = &state.property_list {
        entry.keys = list.clone();
    } else {
        entry.keys = agent.scope(|agent, inner| -> Completion<Vec<Vec<u16>>> {
            let keys =
                enumerable_own_properties(agent, inner, state.realm, value, EnumerableKind::Key)?;
            Ok(keys
                .into_iter()
                .map(|key| string_units(agent, key))
                .collect())
        })?;
    }
    stack.push(entry);
    Ok(())
}

/// Die letzten Schritte von SerializeJSONObject und SerializeJSONArray: die
/// Teile zusammensetzen und die Einrückung zurücknehmen.
fn close(state: &mut State<'_>, done: &Open<'_>) -> Completion<Vec<u16>> {
    let (open, close) = if done.is_array {
        (0x5B, 0x5D)
    } else {
        (0x7B, 0x7D)
    };
    let step_back = state.indent.len().saturating_sub(state.gap.len());
    let mut text = vec![open];
    if !done.partial.is_empty() {
        let mut separator = vec![0x2C];
        if !state.gap.is_empty() {
            separator.push(0x0A);
            separator.extend_from_slice(&state.indent);
            text.push(0x0A);
            text.extend_from_slice(&state.indent);
        }
        for (index, part) in done.partial.iter().enumerate() {
            if index > 0 {
                text.extend_from_slice(&separator);
            }
            text.extend_from_slice(part);
            if text.len() > state.limit {
                return Err(Abrupt::OutOfMemory);
            }
        }
        if !state.gap.is_empty() {
            text.push(0x0A);
            text.extend_from_slice(state.indent.get(..step_back).unwrap_or_default());
        }
    }
    text.push(close);
    state.indent.truncate(step_back);
    if text.len() > state.limit {
        return Err(Abrupt::OutOfMemory);
    }
    Ok(text)
}

/// SerializeJSONProperty (Abschnitt 25.5.4.2) bis auf das Serialisieren
/// von Arrays und Objekten, das der Aufrufer übernimmt. Ein Array oder
/// Objekt kommt in `slot`.
fn property<'s>(
    agent: &mut Agent,
    state: &State<'_>,
    holder: Local<'_>,
    key: &[u16],
    slot: Local<'s>,
) -> Completion<Serialized<'s>> {
    agent.scope(|agent, inner| -> Completion<Serialized<'s>> {
        let property_key = agent.key_from_units(inner, key)?;
        let mut value = get(agent, inner, holder, property_key)?;
        // BigInt gibt es noch nicht; nur Objekte haben `toJSON`.
        if is_object(agent, value) {
            let to_json = agent.key_from_units(inner, &units("toJSON"))?;
            let method = get(agent, inner, value, to_json)?;
            if is_callable(agent.heap(), agent.heap().get(method)) {
                let name = agent.heap_mut().alloc_string(inner, key)?;
                value = call(agent, inner, method, value, &[name])?;
            }
        }
        if let Some(replacer) = state.replacer {
            let name = agent.heap_mut().alloc_string(inner, key)?;
            value = call(agent, inner, replacer, holder, &[name, value])?;
        }
        if let Kind::Object(object) = agent.heap().get(value).kind() {
            if agent.heap().object_is_raw_json(object) {
                let raw = agent.key_from_units(inner, &units("rawJSON"))?;
                let text = get(agent, inner, value, raw)?;
                return Ok(Serialized::Text(string_units(agent, text)));
            }
            match primitive_data(agent, value).map(Value::kind) {
                Some(Kind::Number(_)) => {
                    let number = to_number(agent, inner, value)?;
                    value = agent.heap().local(inner, Value::number(number));
                }
                Some(Kind::String(_)) => value = to_string(agent, inner, value)?,
                Some(Kind::Boolean(boolean)) => {
                    value = agent.heap().local(inner, Value::boolean(boolean));
                }
                _ => {}
            }
        }
        let array = is_object(agent, value)
            && !is_callable(agent.heap(), agent.heap().get(value))
            && is_array(agent, value)?;
        let heap = agent.heap();
        Ok(match heap.get(value).kind() {
            Kind::Null => Serialized::Text(units("null")),
            Kind::Boolean(true) => Serialized::Text(units("true")),
            Kind::Boolean(false) => Serialized::Text(units("false")),
            Kind::String(string) => {
                let text = quote(&heap.string_units(string));
                if text.len() > state.limit {
                    return Err(Abrupt::OutOfMemory);
                }
                Serialized::Text(text)
            }
            Kind::Number(number) if number.is_finite() => {
                Serialized::Text(units(&number::to_string(number)))
            }
            Kind::Number(_) => Serialized::Text(units("null")),
            Kind::Object(_) if !is_callable(heap, heap.get(value)) => {
                heap.set(slot, heap.get(value));
                if array {
                    Serialized::Array(slot)
                } else {
                    Serialized::Object(slot)
                }
            }
            _ => Serialized::Nothing,
        })
    })
}

/// QuoteJSONString (Abschnitt 25.5.4.3) mit UnicodeEscape (Abschnitt
/// 25.5.4.4): Steuerzeichen und einzelne Surrogate als `\uXXXX` mit
/// kleinen Hexziffern.
pub(super) fn quote(text: &[u16]) -> Vec<u16> {
    let mut product = Vec::with_capacity(text.len().saturating_add(2));
    product.push(0x22);
    let mut index = 0;
    while let Some(&unit) = text.get(index) {
        index += 1;
        let short = match unit {
            0x08 => Some(0x62),
            0x09 => Some(0x74),
            0x0A => Some(0x6E),
            0x0C => Some(0x66),
            0x0D => Some(0x72),
            0x22 => Some(0x22),
            0x5C => Some(0x5C),
            _ => None,
        };
        if let Some(letter) = short {
            product.extend_from_slice(&[0x5C, letter]);
            continue;
        }
        let paired = match unit {
            0xD800..=0xDBFF => {
                if let Some(&next) = text.get(index)
                    && (0xDC00..=0xDFFF).contains(&next)
                {
                    product.extend_from_slice(&[unit, next]);
                    index += 1;
                    continue;
                }
                false
            }
            0xDC00..=0xDFFF => false,
            0x00..=0x1F => false,
            _ => true,
        };
        if paired {
            product.push(unit);
        } else {
            product.extend(units(&format!("\\u{unit:04x}")));
        }
    }
    product.push(0x22);
    product
}
