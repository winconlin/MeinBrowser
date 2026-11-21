//! Die Zusätze von %Uint8Array% für base64 und hex (Abschnitt 23.3):
//! `fromBase64` und `fromHex` am Konstruktor, `setFromBase64`,
//! `setFromHex`, `toBase64` und `toHex` am Prototyp, dazu FromBase64 und
//! FromHex (Abschnitte 23.3.3.7 und 23.3.3.8) über den Codeeinheiten der
//! Zeichenkette.
//!
//! Dekodieren und Kodieren bezahlen wie Kopien von Bytes 256 Codeeinheiten
//! oder Bytes je Arbeitsschritt ([`buffer::pay`]). Eine Ausgabe, die nicht
//! in den freien Teil des Heaps passt, ist ein erschöpfter Speicher, bevor
//! sie entsteht.

use super::{allocate_typed_array, validate_typed_array};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::buffer::{self, ElementType};
use crate::builtin::{Builtin, BuiltinCall};
use crate::builtins::{define_functions, units};
use crate::conversion::to_boolean;
use crate::heap::{Local, OutOfMemory, Scope};
use crate::object::{TypedArrayRecord, ordinary_object_create, viewed_buffer};
use crate::operations::{create_data_property_or_throw, get};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Ein Eintrag der Tabelle der Methoden.
const fn method(name: &'static str, length: u32, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

static CONSTRUCTOR_FUNCTIONS: [Builtin; 2] = [
    method("fromBase64", 1, from_base64),
    method("fromHex", 1, from_hex),
];

static PROTOTYPE_FUNCTIONS: [Builtin; 4] = [
    method("setFromBase64", 1, set_from_base64),
    method("setFromHex", 1, set_from_hex),
    method("toBase64", 0, to_base64),
    method("toHex", 0, to_hex),
];

/// Die Funktionen an %Uint8Array% und %Uint8Array.prototype%.
pub(super) fn define(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    constructor: Local<'_>,
    prototype: Local<'_>,
) -> Completion<()> {
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)
}

/// Das Alphabet von RFC 4648, Abschnitt 4.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Das Alphabet von RFC 4648, Abschnitt 5.
const URL_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// `lastChunkHandling`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LastChunk {
    Loose,
    Strict,
    StopBeforePartial,
}

/// Was FromBase64 und FromHex liefern: [[Read]], [[Bytes]] und ob
/// [[Error]] ein SyntaxError ist.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Decoded {
    pub(crate) read: usize,
    pub(crate) bytes: Vec<u8>,
    pub(crate) error: bool,
}

impl Decoded {
    fn done(read: usize, bytes: Vec<u8>) -> Decoded {
        Decoded {
            read,
            bytes,
            error: false,
        }
    }

    fn failed(read: usize, bytes: Vec<u8>) -> Decoded {
        Decoded {
            read,
            bytes,
            error: true,
        }
    }
}

/// SkipAsciiWhitespace (Abschnitt 23.3.3.4).
fn skip_ascii_whitespace(units: &[u16], mut index: usize) -> usize {
    while let Some(&unit) = units.get(index) {
        if !matches!(unit, 0x09 | 0x0A | 0x0C | 0x0D | 0x20) {
            return index;
        }
        index += 1;
    }
    index
}

/// DecodeFullLengthBase64Chunk (Abschnitt 23.3.3.6) für vier Werte von 0
/// bis 63.
fn decode_full_chunk(chunk: [u8; 4]) -> [u8; 3] {
    let [a, b, c, d] = chunk.map(u32::from);
    let bits = (a << 18) | (b << 12) | (c << 6) | d;
    let [_, high, middle, low] = bits.to_be_bytes();
    [high, middle, low]
}

/// DecodeFinalBase64Chunk (Abschnitt 23.3.3.5) für die ersten `length`
/// Werte (2 oder 3); `None` für den SyntaxError bei übrigen Bits.
fn decode_final_chunk(chunk: [u8; 4], length: usize, throw_on_extra_bits: bool) -> Option<Vec<u8>> {
    let mut padded = chunk;
    for value in padded.iter_mut().skip(length) {
        *value = 0;
    }
    let [first, second, third] = decode_full_chunk(padded);
    if length == 2 {
        if throw_on_extra_bits && second != 0 {
            return None;
        }
        return Some(vec![first]);
    }
    if throw_on_extra_bits && third != 0 {
        return None;
    }
    Some(vec![first, second])
}

/// Der Wert eines Zeichens im Standardalphabet.
fn alphabet_value(unit: u16) -> Option<u8> {
    let position = ALPHABET
        .iter()
        .position(|&letter| u16::from(letter) == unit)?;
    u8::try_from(position).ok()
}

/// FromBase64 (Abschnitt 23.3.3.7). `max_length` `None` steht für
/// 2⁵³ − 1, das keine Zeichenkette erreicht.
pub(crate) fn decode_base64(
    units: &[u16],
    url: bool,
    last_chunk: LastChunk,
    max_length: Option<usize>,
) -> Decoded {
    let max_length = max_length.unwrap_or(usize::MAX);
    if max_length == 0 {
        return Decoded::done(0, Vec::new());
    }
    let length = units.len();
    let mut read = 0;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4];
    let mut chunk_length = 0;
    let mut index = 0;
    loop {
        index = skip_ascii_whitespace(units, index);
        let Some(&unit) = units.get(index) else {
            if chunk_length > 0 {
                match last_chunk {
                    LastChunk::StopBeforePartial => return Decoded::done(read, bytes),
                    LastChunk::Strict => return Decoded::failed(read, bytes),
                    LastChunk::Loose => {}
                }
                if chunk_length == 1 {
                    return Decoded::failed(read, bytes);
                }
                // Ohne Prüfung der übrigen Bits gelingt das immer.
                if let Some(last) = decode_final_chunk(chunk, chunk_length, false) {
                    bytes.extend(last);
                }
            }
            return Decoded::done(length, bytes);
        };
        index += 1;
        if unit == u16::from(b'=') {
            if chunk_length < 2 {
                return Decoded::failed(read, bytes);
            }
            index = skip_ascii_whitespace(units, index);
            if chunk_length == 2 {
                let Some(&next) = units.get(index) else {
                    if last_chunk == LastChunk::StopBeforePartial {
                        return Decoded::done(read, bytes);
                    }
                    return Decoded::failed(read, bytes);
                };
                if next == u16::from(b'=') {
                    index = skip_ascii_whitespace(units, index + 1);
                }
            }
            if index < length {
                return Decoded::failed(read, bytes);
            }
            let throw_on_extra_bits = last_chunk == LastChunk::Strict;
            let Some(last) = decode_final_chunk(chunk, chunk_length, throw_on_extra_bits) else {
                return Decoded::failed(read, bytes);
            };
            bytes.extend(last);
            return Decoded::done(length, bytes);
        }
        let mut unit = unit;
        if url {
            if unit == u16::from(b'+') || unit == u16::from(b'/') {
                return Decoded::failed(read, bytes);
            }
            if unit == u16::from(b'-') {
                unit = u16::from(b'+');
            } else if unit == u16::from(b'_') {
                unit = u16::from(b'/');
            }
        }
        let Some(value) = alphabet_value(unit) else {
            return Decoded::failed(read, bytes);
        };
        let remaining = max_length - bytes.len();
        if (remaining == 1 && chunk_length == 2) || (remaining == 2 && chunk_length == 3) {
            return Decoded::done(read, bytes);
        }
        if let Some(slot) = chunk.get_mut(chunk_length) {
            *slot = value;
        }
        chunk_length += 1;
        if chunk_length == 4 {
            bytes.extend(decode_full_chunk(chunk));
            chunk_length = 0;
            read = index;
            if bytes.len() == max_length {
                return Decoded::done(read, bytes);
            }
        }
    }
}

/// Der Wert einer Hexadezimalziffer.
fn hex_value(unit: u16) -> Option<u8> {
    let digit = char::from_u32(u32::from(unit))?.to_digit(16)?;
    u8::try_from(digit).ok()
}

/// FromHex (Abschnitt 23.3.3.8). `max_length` `None` steht für 2⁵³ − 1.
pub(crate) fn decode_hex(units: &[u16], max_length: Option<usize>) -> Decoded {
    let max_length = max_length.unwrap_or(usize::MAX);
    let mut bytes = Vec::new();
    let mut read = 0;
    if !units.len().is_multiple_of(2) {
        return Decoded::failed(read, bytes);
    }
    while bytes.len() < max_length {
        let Some(&[high, low]) = units
            .get(read..read + 2)
            .and_then(|pair| pair.first_chunk())
        else {
            break;
        };
        let (Some(high), Some(low)) = (hex_value(high), hex_value(low)) else {
            return Decoded::failed(read, bytes);
        };
        read += 2;
        bytes.push((high << 4) | low);
    }
    Decoded::done(read, bytes)
}

/// Die Kodierung von `bytes` nach RFC 4648, Abschnitt 4 oder 5, mit
/// Auffüllen durch `=`, wenn `padding`.
pub(crate) fn encode_base64(bytes: &[u8], url: bool, padding: bool) -> Vec<u16> {
    let alphabet = if url { URL_ALPHABET } else { ALPHABET };
    let letter = |bits: u32| {
        let index = usize::try_from(bits & 0x3F).unwrap_or(0);
        u16::from(alphabet.get(index).copied().unwrap_or(b'A'))
    };
    let mut out = Vec::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for group in bytes.chunks(3) {
        let [a, b, c] = [0, 1, 2].map(|i| u32::from(group.get(i).copied().unwrap_or(0)));
        let bits = (a << 16) | (b << 8) | c;
        out.push(letter(bits >> 18));
        out.push(letter(bits >> 12));
        if group.len() > 1 {
            out.push(letter(bits >> 6));
        } else if padding {
            out.push(u16::from(b'='));
        }
        if group.len() > 2 {
            out.push(letter(bits));
        } else if padding {
            out.push(u16::from(b'='));
        }
    }
    out
}

// Helfer

/// Die Codeeinheiten des Arguments 0, das eine Zeichenkette sein muss.
fn string_argument(agent: &mut Agent, arguments: &BuiltinCall<'_, '_>) -> Completion<Vec<u16>> {
    let heap = agent.heap();
    let value = arguments
        .arguments
        .first()
        .map_or(Value::UNDEFINED, |&argument| heap.get(argument));
    match value.kind() {
        Kind::String(string) => Ok(heap.string_units(string)),
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// GetOptionsObject (Abschnitt 7.3.36): *undefined* wird
/// ein Objekt ohne Prototyp, ein Objekt bleibt, anderes ist ein TypeError.
fn options_object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    options: Local<'s>,
) -> Completion<Local<'s>> {
    match agent.heap().get(options).kind() {
        Kind::Undefined => Ok(ordinary_object_create(agent, scope, None)?),
        Kind::Object(_) => Ok(options),
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// Ob `value` genau die Zeichenkette `text` ist.
fn is_string(agent: &Agent, value: Local<'_>, text: &str) -> bool {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::String(string) => heap.string_units(string) == units(text),
        _ => false,
    }
}

/// Die Eigenschaft `alphabet` der Optionen: `true` für *"base64url"*.
fn alphabet_option(agent: &mut Agent, scope: &Scope, options: Local<'_>) -> Completion<bool> {
    let key = agent.key_from_units(scope, &units("alphabet"))?;
    let alphabet = get(agent, scope, options, key)?;
    if agent.heap().get(alphabet).is_undefined() || is_string(agent, alphabet, "base64") {
        return Ok(false);
    }
    if is_string(agent, alphabet, "base64url") {
        return Ok(true);
    }
    Err(agent.throw_error(ErrorKind::TypeError))
}

/// Die Eigenschaft `lastChunkHandling` der Optionen.
fn last_chunk_option(
    agent: &mut Agent,
    scope: &Scope,
    options: Local<'_>,
) -> Completion<LastChunk> {
    let key = agent.key_from_units(scope, &units("lastChunkHandling"))?;
    let handling = get(agent, scope, options, key)?;
    if agent.heap().get(handling).is_undefined() || is_string(agent, handling, "loose") {
        return Ok(LastChunk::Loose);
    }
    if is_string(agent, handling, "strict") {
        return Ok(LastChunk::Strict);
    }
    if is_string(agent, handling, "stop-before-partial") {
        return Ok(LastChunk::StopBeforePartial);
    }
    Err(agent.throw_error(ErrorKind::TypeError))
}

/// ValidateUint8Array (Abschnitt 23.3.3.1).
fn validate_uint8_array(agent: &mut Agent, value: Local<'_>) -> Completion<()> {
    match crate::object::typed_array_record_of(agent, value) {
        Some(record) if record.kind == ElementType::Uint8 => Ok(()),
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// Bezahlt die Arbeit an `count` Codeeinheiten oder Bytes.
fn pay(agent: &Agent, count: usize) -> Completion<()> {
    buffer::pay(agent, count)
}

/// SetUint8ArrayBytes (Abschnitt 23.3.3.3) in ein TypedArray innerhalb
/// seiner Grenzen.
fn write_bytes(
    agent: &Agent,
    scope: &Scope,
    into: Local<'_>,
    record: &TypedArrayRecord,
    bytes: &[u8],
) {
    let buffer = viewed_buffer(agent, scope, into);
    let heap = agent.heap();
    if let (Kind::Object(buffer), Ok(offset)) =
        (heap.get(buffer).kind(), usize::try_from(record.byte_offset))
    {
        buffer::write_block(heap, buffer, offset, bytes);
    }
}

/// GetUint8ArrayBytes (Abschnitt 23.3.3.2).
fn read_bytes(agent: &mut Agent, scope: &Scope, object: Local<'_>) -> Completion<Vec<u8>> {
    let record = validate_typed_array(agent, object)?;
    let length = usize::try_from(record.length()).map_err(|_| OutOfMemory)?;
    pay(agent, length)?;
    let buffer = viewed_buffer(agent, scope, object);
    let mut bytes = vec![0; length];
    let heap = agent.heap();
    if let (Kind::Object(buffer), Ok(offset)) =
        (heap.get(buffer).kind(), usize::try_from(record.byte_offset))
    {
        buffer::read_block(heap, buffer, offset, &mut bytes);
    }
    Ok(bytes)
}

/// Eine neue Zeichenkette aus `text`, wenn sie in den freien Teil des
/// Heaps passt.
fn string_of<'s>(agent: &mut Agent, scope: &'s Scope, text: &[u16]) -> Completion<Local<'s>> {
    Ok(agent.heap_mut().alloc_string(scope, text)?)
}

/// Ob eine Ausgabe aus `count` Codeeinheiten in den freien Teil des Heaps
/// passt.
fn output_fits(agent: &Agent, count: usize) -> Completion<()> {
    let heap = agent.heap();
    let bytes = count.saturating_mul(2).saturating_add(heap.used_bytes());
    if bytes > heap.limit() {
        return Err(OutOfMemory.into());
    }
    Ok(())
}

/// Ein neues Uint8Array mit `bytes` (Uint8Array.fromBase64 und fromHex ab
/// Schritt 12 beziehungsweise 5).
fn uint8_array_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    bytes: &[u8],
) -> Completion<Local<'s>> {
    let constructor = agent.intrinsic(scope, realm, Intrinsic::Uint8Array);
    let length = u64::try_from(bytes.len()).map_err(|_| OutOfMemory)?;
    let typed_array = allocate_typed_array(
        agent,
        scope,
        realm,
        ElementType::Uint8,
        constructor,
        Some(length),
    )?;
    let record = validate_typed_array(agent, typed_array)?;
    write_bytes(agent, scope, typed_array, &record, bytes);
    Ok(typed_array)
}

/// Das Ergebnis von `setFromBase64` und `setFromHex`: ein Objekt mit
/// `read` und `written`.
fn read_and_written<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    read: usize,
    written: usize,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let result = ordinary_object_create(agent, scope, Some(prototype))?;
    for (name, count) in [("read", read), ("written", written)] {
        // Längen von Zeichenketten liegen unter 2⁵³.
        let value = agent.heap().local(scope, Value::number(count as f64));
        let key = agent.key_from_units(scope, &units(name))?;
        create_data_property_or_throw(agent, scope, result, key, value)?;
    }
    Ok(result)
}

// Die Funktionen

/// Uint8Array.fromBase64 ( string [ , options ] ) (Abschnitt 23.3.1.1).
fn from_base64<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let text = string_argument(agent, arguments)?;
    let options = arguments.argument(agent, scope, 1);
    let options = options_object(agent, scope, options)?;
    let url = alphabet_option(agent, scope, options)?;
    let last_chunk = last_chunk_option(agent, scope, options)?;
    pay(agent, text.len())?;
    let result = decode_base64(&text, url, last_chunk, None);
    if result.error {
        return Err(agent.throw_error(ErrorKind::SyntaxError));
    }
    uint8_array_of(agent, scope, arguments.realm, &result.bytes)
}

/// Uint8Array.fromHex ( string ) (Abschnitt 23.3.1.2).
fn from_hex<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let text = string_argument(agent, arguments)?;
    pay(agent, text.len())?;
    let result = decode_hex(&text, None);
    if result.error {
        return Err(agent.throw_error(ErrorKind::SyntaxError));
    }
    uint8_array_of(agent, scope, arguments.realm, &result.bytes)
}

/// Uint8Array.prototype.setFromBase64 ( string [ , options ] ) (Abschnitt
/// 23.3.2.1).
fn set_from_base64<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let into = arguments.this;
    validate_uint8_array(agent, into)?;
    let text = string_argument(agent, arguments)?;
    let options = arguments.argument(agent, scope, 1);
    let options = options_object(agent, scope, options)?;
    let url = alphabet_option(agent, scope, options)?;
    let last_chunk = last_chunk_option(agent, scope, options)?;
    let record = validate_typed_array(agent, into)?;
    let byte_length = usize::try_from(record.length()).map_err(|_| OutOfMemory)?;
    pay(agent, text.len())?;
    let result = decode_base64(&text, url, last_chunk, Some(byte_length));
    write_bytes(agent, scope, into, &record, &result.bytes);
    if result.error {
        return Err(agent.throw_error(ErrorKind::SyntaxError));
    }
    read_and_written(
        agent,
        scope,
        arguments.realm,
        result.read,
        result.bytes.len(),
    )
}

/// Uint8Array.prototype.setFromHex ( string ) (Abschnitt 23.3.2.2).
fn set_from_hex<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let into = arguments.this;
    validate_uint8_array(agent, into)?;
    let text = string_argument(agent, arguments)?;
    let record = validate_typed_array(agent, into)?;
    let byte_length = usize::try_from(record.length()).map_err(|_| OutOfMemory)?;
    pay(agent, text.len())?;
    let result = decode_hex(&text, Some(byte_length));
    write_bytes(agent, scope, into, &record, &result.bytes);
    if result.error {
        return Err(agent.throw_error(ErrorKind::SyntaxError));
    }
    read_and_written(
        agent,
        scope,
        arguments.realm,
        result.read,
        result.bytes.len(),
    )
}

/// Uint8Array.prototype.toBase64 ( [ options ] ) (Abschnitt 23.3.2.3).
fn to_base64<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    validate_uint8_array(agent, object)?;
    let options = arguments.argument(agent, scope, 0);
    let options = options_object(agent, scope, options)?;
    let url = alphabet_option(agent, scope, options)?;
    let key = agent.key_from_units(scope, &units("omitPadding"))?;
    let omit_padding = get(agent, scope, options, key)?;
    let omit_padding = {
        let heap = agent.heap();
        to_boolean(heap, heap.get(omit_padding))
    };
    let bytes = read_bytes(agent, scope, object)?;
    output_fits(agent, bytes.len().div_ceil(3).saturating_mul(4))?;
    let text = encode_base64(&bytes, url, !omit_padding);
    string_of(agent, scope, &text)
}

/// Uint8Array.prototype.toHex ( ) (Abschnitt 23.3.2.4).
fn to_hex<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let object = arguments.this;
    validate_uint8_array(agent, object)?;
    let bytes = read_bytes(agent, scope, object)?;
    output_fits(agent, bytes.len().saturating_mul(2))?;
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let digit = |nibble: u8| u16::from(DIGITS.get(usize::from(nibble)).copied().unwrap_or(b'0'));
    let mut text = Vec::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        text.push(digit(byte >> 4));
        text.push(digit(byte & 0x0F));
    }
    string_of(agent, scope, &text)
}
