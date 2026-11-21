//! %Atomics% (Abschnitt 25.4): ein gewöhnliches Objekt mit den Funktionen
//! aus Abschnitt 25.4.4. Auf einem SharedArrayBuffer sind die Zugriffe
//! atomar (siehe `crate::shared`), auf einem ArrayBuffer, der nicht geteilt
//! ist, gewöhnliche Zugriffe, denn ihn sieht nur dieser Agent. Das Warten
//! steht in `crate::waiting`.
//!
//! Festlegungen:
//! - [[IsLockFree1]], [[IsLockFree2]] und [[IsLockFree8]] sind *true*: Jede
//!   Größe geht auf ein Wort aus [`AtomicU64`](std::sync::atomic::AtomicU64).
//! - `Atomics.pause` gibt für jeden Aufruf einmal
//!   [`std::hint::spin_loop`], mit einer ganzen Zahl N höchstens
//!   [`MAX_PAUSES`] Mal; die Spezifikation überlässt die Zahl der
//!   Implementierung.

use super::typed_array::validate_typed_array;
use super::{define_functions, define_to_string_tag, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::buffer::{self, ElementType, number_to_raw_bytes, raw_bytes_of, raw_bytes_to_numeric};
use crate::builtin::{Builtin, BuiltinCall};
use crate::conversion::{to_bigint, to_index, to_int32, to_integer_or_infinity, to_number};
use crate::heap::{Local, Scope};
use crate::object::{
    TypedArrayRecord, ordinary_object_create, typed_array_record_of, viewed_buffer,
};
use crate::operations::create_data_property_or_throw;
use crate::promise::promise_create;
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};
use crate::waiting::{deadline, notify as notify_waiters};

/// So oft gibt `Atomics.pause` höchstens das Signal fürs Warten.
const MAX_PAUSES: u32 = 64;

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

static FUNCTIONS: [Builtin; 14] = [
    function("add", 3, add),
    function("and", 3, and),
    function("compareExchange", 4, compare_exchange),
    function("exchange", 3, exchange),
    function("isLockFree", 1, is_lock_free),
    function("load", 2, load),
    function("or", 3, or),
    function("store", 3, store),
    function("sub", 3, sub),
    function("wait", 4, wait),
    function("waitAsync", 4, wait_async),
    function("notify", 3, notify),
    function("xor", 3, xor),
    function("pause", 0, pause),
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let atomics = ordinary_object_create(agent, scope, Some(prototype))?;
    define_functions(agent, scope, realm, atomics, &FUNCTIONS)?;
    define_to_string_tag(agent, scope, atomics, "Atomics")?;
    agent.set_intrinsic(realm, Intrinsic::Atomics, atomics);
    Ok(())
}

// Die abstrakten Operationen

/// ValidateIntegerTypedArray (Abschnitt 25.4.3.1). Mit `waitable` nur
/// Int32Array und BigInt64Array, sonst die Arten mit ganzen Zahlen außer
/// Uint8ClampedArray.
fn validate_integer_typed_array(
    agent: &mut Agent,
    typed_array: Local<'_>,
    waitable: bool,
) -> Completion<TypedArrayRecord> {
    let record = validate_typed_array(agent, typed_array)?;
    let allowed = if waitable {
        matches!(record.kind, ElementType::Int32 | ElementType::BigInt64)
    } else {
        !matches!(
            record.kind,
            ElementType::Uint8Clamped
                | ElementType::Float16
                | ElementType::Float32
                | ElementType::Float64
        )
    };
    if !allowed {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(record)
}

/// ValidateAtomicAccess (Abschnitt 25.4.3.2): der Index des Elements im
/// Block.
fn validate_atomic_access(
    agent: &mut Agent,
    scope: &Scope,
    record: &TypedArrayRecord,
    request_index: Local<'_>,
) -> Completion<usize> {
    let length = record.length();
    let access_index = to_index(agent, scope, request_index)?;
    if access_index >= length {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    // Das Element liegt im Puffer, also passt sein Index in `usize`.
    let size = u64::try_from(record.kind.size()).unwrap_or(u64::MAX);
    access_index
        .checked_mul(size)
        .and_then(|index| index.checked_add(record.byte_offset))
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| agent.throw_error(ErrorKind::RangeError))
}

/// ValidateAtomicAccessOnIntegerTypedArray (Abschnitt 25.4.3.3).
fn validate_atomic_access_on_integer_typed_array(
    agent: &mut Agent,
    scope: &Scope,
    typed_array: Local<'_>,
    request_index: Local<'_>,
) -> Completion<(usize, ElementType)> {
    let record = validate_integer_typed_array(agent, typed_array, false)?;
    let index = validate_atomic_access(agent, scope, &record, request_index)?;
    Ok((index, record.kind))
}

/// RevalidateAtomicAccess (Abschnitt 25.4.3.4): Die Umwandlungen der
/// Argumente können den Puffer abgelöst oder verkleinert haben. Das ganze
/// Element von `kind` ab `byte_index` muss noch im Puffer liegen, auch
/// sein letztes Byte.
fn revalidate_atomic_access(
    agent: &mut Agent,
    typed_array: Local<'_>,
    byte_index: usize,
    kind: ElementType,
) -> Completion<()> {
    let record = typed_array_record_of(agent, typed_array);
    let Some(record) = record.filter(|record| !record.is_out_of_bounds()) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let end = byte_index
        .checked_add(kind.size())
        .and_then(|end| u64::try_from(end).ok());
    let in_buffer = record
        .buffer_byte_length
        .zip(end)
        .is_some_and(|(length, end)| end <= length);
    if !in_buffer {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    Ok(())
}

/// Ein Wert für ein Element der Art `kind` als Bytes, das erste im
/// niedrigsten: ToBigInt für die Arten mit BigInts, sonst
/// ToIntegerOrInfinity. Dazu der Wert selbst, wie `Atomics.store` ihn
/// zurückgibt.
fn element_bits<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    kind: ElementType,
    value: Local<'_>,
) -> Completion<(u64, Local<'s>)> {
    if kind.is_bigint() {
        let bigint = to_bigint(agent, scope, value)?;
        let bits = u64::from_le_bytes(raw_bytes_of(agent, bigint, kind, true));
        Ok((bits, bigint))
    } else {
        let integer = to_integer_or_infinity(agent, scope, value)?;
        let bits = u64::from_le_bytes(number_to_raw_bytes(kind, integer, true));
        Ok((bits, agent.heap().local(scope, Value::number(integer))))
    }
}

/// Das Element mit den Bytes `bits` als Number oder BigInt.
fn element_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    kind: ElementType,
    bits: u64,
) -> Completion<Local<'s>> {
    let numeric = raw_bytes_to_numeric(kind, &bits.to_le_bytes(), true);
    buffer::numeric_value(agent, scope, numeric)
}

/// Liest und ersetzt das Element bei `byte_index` im Puffer von
/// `typed_array` (siehe [`buffer::update_element`]).
fn update(
    agent: &Agent,
    scope: &Scope,
    typed_array: Local<'_>,
    byte_index: usize,
    kind: ElementType,
    operation: impl Fn(u64) -> Option<u64>,
) -> u64 {
    let buffer = viewed_buffer(agent, scope, typed_array);
    let heap = agent.heap();
    // RevalidateAtomicAccess hat geprüft, dass das Element im Puffer
    // liegt.
    match heap.get(buffer).kind() {
        Kind::Object(buffer) => {
            buffer::update_element(heap, buffer, byte_index, kind.size(), operation).unwrap_or(0)
        }
        _ => 0,
    }
}

/// AtomicReadModifyWrite (Abschnitt 25.4.3.17) mit der Operation `op` auf
/// den Bytes als Zahl.
fn read_modify_write<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    op: fn(u64, u64) -> u64,
) -> Completion<Local<'s>> {
    let typed_array = arguments.argument(agent, scope, 0);
    let index = arguments.argument(agent, scope, 1);
    let (byte_index, kind) =
        validate_atomic_access_on_integer_typed_array(agent, scope, typed_array, index)?;
    let value = arguments.argument(agent, scope, 2);
    let (bits, _) = element_bits(agent, scope, kind, value)?;
    revalidate_atomic_access(agent, typed_array, byte_index, kind)?;
    let old = update(agent, scope, typed_array, byte_index, kind, |old| {
        Some(op(old, bits))
    });
    element_value(agent, scope, kind, old)
}

// Die Funktionen

/// Atomics.add ( typedArray, index, value ) (Abschnitt 25.4.4.1). Die
/// Summe modulo 2^(8 · Größe) hat in beiden Darstellungen dieselben
/// Bytes.
fn add<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    read_modify_write(agent, scope, arguments, u64::wrapping_add)
}

/// Atomics.and ( typedArray, index, value ) (Abschnitt 25.4.4.2).
fn and<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    read_modify_write(agent, scope, arguments, |old, value| old & value)
}

/// Atomics.exchange ( typedArray, index, value ) (Abschnitt 25.4.4.5).
fn exchange<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    read_modify_write(agent, scope, arguments, |_, value| value)
}

/// Atomics.or ( typedArray, index, value ) (Abschnitt 25.4.4.9).
fn or<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    read_modify_write(agent, scope, arguments, |old, value| old | value)
}

/// Atomics.sub ( typedArray, index, value ) (Abschnitt 25.4.4.12).
fn sub<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    read_modify_write(agent, scope, arguments, u64::wrapping_sub)
}

/// Atomics.xor ( typedArray, index, value ) (Abschnitt 25.4.4.16).
fn xor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    read_modify_write(agent, scope, arguments, |old, value| old ^ value)
}

/// Atomics.compareExchange ( typedArray, index, expectedValue,
/// replacementValue ) (Abschnitt 25.4.4.4). Verglichen werden die Bytes.
fn compare_exchange<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let typed_array = arguments.argument(agent, scope, 0);
    let index = arguments.argument(agent, scope, 1);
    let (byte_index, kind) =
        validate_atomic_access_on_integer_typed_array(agent, scope, typed_array, index)?;
    let expected = arguments.argument(agent, scope, 2);
    let (expected, _) = element_bits(agent, scope, kind, expected)?;
    let replacement = arguments.argument(agent, scope, 3);
    let (replacement, _) = element_bits(agent, scope, kind, replacement)?;
    revalidate_atomic_access(agent, typed_array, byte_index, kind)?;
    // Die Bytes sind auf die Größe gekürzt, wie die gelesenen.
    let mask = match kind.size() {
        8 => u64::MAX,
        size => (1u64 << (size * 8)) - 1,
    };
    let old = update(agent, scope, typed_array, byte_index, kind, |old| {
        (old == expected & mask).then_some(replacement)
    });
    element_value(agent, scope, kind, old)
}

/// Atomics.isLockFree ( size ) (Abschnitt 25.4.4.6).
fn is_lock_free<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let size = arguments.argument(agent, scope, 0);
    let n = to_integer_or_infinity(agent, scope, size)?;
    let lock_free = n == 1.0 || n == 2.0 || n == 4.0 || n == 8.0;
    Ok(agent.heap().local(scope, Value::boolean(lock_free)))
}

/// Atomics.load ( typedArray, index ) (Abschnitt 25.4.4.7).
fn load<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let typed_array = arguments.argument(agent, scope, 0);
    let index = arguments.argument(agent, scope, 1);
    let (byte_index, kind) =
        validate_atomic_access_on_integer_typed_array(agent, scope, typed_array, index)?;
    revalidate_atomic_access(agent, typed_array, byte_index, kind)?;
    let bits = update(agent, scope, typed_array, byte_index, kind, |_| None);
    element_value(agent, scope, kind, bits)
}

/// Atomics.store ( typedArray, index, value ) (Abschnitt 25.4.4.11). Das
/// Ergebnis ist der umgewandelte Wert, nicht der geschriebene.
fn store<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let typed_array = arguments.argument(agent, scope, 0);
    let index = arguments.argument(agent, scope, 1);
    let (byte_index, kind) =
        validate_atomic_access_on_integer_typed_array(agent, scope, typed_array, index)?;
    let value = arguments.argument(agent, scope, 2);
    let (bits, value) = element_bits(agent, scope, kind, value)?;
    revalidate_atomic_access(agent, typed_array, byte_index, kind)?;
    update(agent, scope, typed_array, byte_index, kind, |_| Some(bits));
    Ok(value)
}

/// Atomics.wait ( typedArray, index, value, timeout ) (Abschnitt
/// 25.4.4.13).
fn wait<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    do_wait(agent, scope, arguments, false)
}

/// Atomics.waitAsync ( typedArray, index, value, timeout ) (Abschnitt
/// 25.4.4.14).
fn wait_async<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    do_wait(agent, scope, arguments, true)
}

/// DoWait ( mode, typedArray, index, value, timeout ) (Abschnitt
/// 25.4.3.14); `asynchronous` steht für ~async~.
fn do_wait<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    asynchronous: bool,
) -> Completion<Local<'s>> {
    let typed_array = arguments.argument(agent, scope, 0);
    let record = validate_integer_typed_array(agent, typed_array, true)?;
    let buffer = viewed_buffer(agent, scope, typed_array);
    let Some(block) = buffer::shared_block(agent, buffer) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let index = arguments.argument(agent, scope, 1);
    let byte_index = validate_atomic_access(agent, scope, &record, index)?;
    let value = arguments.argument(agent, scope, 2);
    let expected = if record.kind == ElementType::BigInt64 {
        // ToBigInt64: die unteren 64 Bit des Zweierkomplements.
        let bigint = to_bigint(agent, scope, value)?;
        u64::from_le_bytes(raw_bytes_of(agent, bigint, ElementType::BigInt64, true))
    } else {
        u64::from(u32::from_le_bytes(
            to_int32(agent, scope, value)?.to_le_bytes(),
        ))
    };
    let timeout = arguments.argument(agent, scope, 3);
    let q = to_number(agent, scope, timeout)?;
    let t = if q.is_nan() {
        f64::INFINITY
    } else {
        q.max(0.0)
    };
    if !asynchronous && !agent.can_suspend() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let size = record.kind.size();
    if !asynchronous {
        let result = agent.wait_blocking(&block, byte_index, size, expected, deadline(t))?;
        return string(agent, scope, result.text());
    }
    let realm = arguments.realm;
    let prototype = agent.intrinsic(scope, realm, Intrinsic::PromisePrototype);
    let promise = promise_create(agent, scope, prototype)?;
    let object_prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let result = ordinary_object_create(agent, scope, Some(object_prototype))?;
    let started = agent.wait_async(&block, byte_index, size, expected, t, promise, realm);
    let (is_async, value) = match started {
        Some(early) => (false, string(agent, scope, early.text())?),
        None => (true, promise),
    };
    let key = agent.key_from_units(scope, &units("async"))?;
    let flag = agent.heap().local(scope, Value::boolean(is_async));
    create_data_property_or_throw(agent, scope, result, key, flag)?;
    let key = agent.key_from_units(scope, &units("value"))?;
    create_data_property_or_throw(agent, scope, result, key, value)?;
    Ok(result)
}

fn string<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> Completion<Local<'s>> {
    Ok(agent.heap_mut().alloc_string(scope, &units(text))?)
}

/// Atomics.notify ( typedArray, index, count ) (Abschnitt 25.4.4.8).
fn notify<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let typed_array = arguments.argument(agent, scope, 0);
    let record = validate_integer_typed_array(agent, typed_array, true)?;
    let index = arguments.argument(agent, scope, 1);
    let byte_index = validate_atomic_access(agent, scope, &record, index)?;
    let count = arguments.argument(agent, scope, 2);
    let count = if agent.heap().get(count).is_undefined() {
        u64::MAX
    } else {
        // Ganz oder ±∞; `as` begrenzt auf 0 bis u64::MAX.
        to_integer_or_infinity(agent, scope, count)?.max(0.0) as u64
    };
    let buffer = viewed_buffer(agent, scope, typed_array);
    let woken = match buffer::shared_block(agent, buffer) {
        Some(block) => notify_waiters(&block, byte_index, count),
        None => 0,
    };
    // Höchstens so viele, wie es Warter geben kann.
    Ok(agent.heap().local(scope, Value::number(woken as f64)))
}

/// Atomics.pause ( [ N ] ) (Abschnitt 25.4.4.10).
fn pause<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let n = arguments.argument(agent, scope, 0);
    let pauses = match agent.heap().get(n).kind() {
        Kind::Undefined => 1,
        Kind::Number(number) if number.is_finite() && number.fract() == 0.0 => {
            // Ganz und endlich; `as` begrenzt auf den Bereich von u32.
            (number as u32).min(MAX_PAUSES)
        }
        _ => return Err(agent.throw_error(ErrorKind::TypeError)),
    };
    for _ in 0..pauses {
        std::hint::spin_loop();
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}
