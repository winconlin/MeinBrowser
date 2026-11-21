//! %DataView% (Abschnitte 25.3.2 und 25.3.3) und %DataView.prototype%
//! (Abschnitt 25.3.4), ein gewöhnliches Objekt ohne [[DataView]].
//!
//! Ein DataView ist ein Objekt der Art [`ObjectClass::DataView`] mit den
//! internen Feldern [[ViewedArrayBuffer]], [[ByteLength]] (eine Number oder
//! *undefined* für ~auto~) und [[ByteOffset]]. Die Bytes liest und
//! schreibt [`crate::buffer`].

use super::{define_functions, define_getter, define_to_string_tag, link_constructor};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::buffer::{self, ElementType};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::{to_bigint, to_boolean, to_index, to_number};
use crate::heap::{Local, ObjectClass, Scope};
use crate::object::{INLINE, get_prototype_from_constructor, ordinary_object_create};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Das Feld mit [[ViewedArrayBuffer]].
const VIEWED_BUFFER: usize = 0;
/// Das Feld mit [[ByteLength]].
const BYTE_LENGTH: usize = 1;
/// Das Feld mit [[ByteOffset]].
const BYTE_OFFSET: usize = 2;

static CONSTRUCTOR: Builtin = Builtin {
    name: "DataView",
    length: 1,
    constructor: true,
    behaviour: data_view_constructor,
};

/// Ein Eintrag der Tabelle der Methoden.
const fn method(name: &'static str, length: u32, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

static GETTERS: [(&str, Builtin); 3] = [
    ("buffer", method("get buffer", 0, buffer_getter)),
    (
        "byteLength",
        method("get byteLength", 0, byte_length_getter),
    ),
    (
        "byteOffset",
        method("get byteOffset", 0, byte_offset_getter),
    ),
];

static PROTOTYPE_FUNCTIONS: [Builtin; 22] = [
    method("getBigInt64", 1, get_big_int64),
    method("getBigUint64", 1, get_big_uint64),
    method("getFloat16", 1, get_float16),
    method("getFloat32", 1, get_float32),
    method("getFloat64", 1, get_float64),
    method("getInt8", 1, get_int8),
    method("getInt16", 1, get_int16),
    method("getInt32", 1, get_int32),
    method("getUint8", 1, get_uint8),
    method("getUint16", 1, get_uint16),
    method("getUint32", 1, get_uint32),
    method("setBigInt64", 2, set_big_int64),
    method("setBigUint64", 2, set_big_uint64),
    method("setFloat16", 2, set_float16),
    method("setFloat32", 2, set_float32),
    method("setFloat64", 2, set_float64),
    method("setInt8", 2, set_int8),
    method("setInt16", 2, set_int16),
    method("setInt32", 2, set_int32),
    method("setUint8", 2, set_uint8),
    method("setUint16", 2, set_uint16),
    method("setUint32", 2, set_uint32),
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(object))?;
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    for (name, getter) in &GETTERS {
        define_getter(agent, scope, realm, prototype, name, getter)?;
    }
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    define_to_string_tag(agent, scope, prototype, "DataView")?;
    agent.set_intrinsic(realm, Intrinsic::DataView, constructor);
    agent.set_intrinsic(realm, Intrinsic::DataViewPrototype, prototype);
    Ok(())
}

// Helfer

/// Die internen Felder eines DataView.
struct View<'s> {
    /// [[ViewedArrayBuffer]].
    buffer: Local<'s>,
    /// [[ByteLength]], `None` für ~auto~.
    byte_length: Option<usize>,
    /// [[ByteOffset]].
    byte_offset: usize,
}

/// Eine Länge oder ein Abstand aus einem internen Feld. Die Felder halten
/// nur ganze Zahlen bis zur Größe des Heaps.
fn field_length(value: Option<Value<'_>>) -> Option<usize> {
    value
        .and_then(Value::as_number)
        .map(|number| number as usize)
}

/// RequireInternalSlot(`value`, [[DataView]]) (Abschnitt 10.1.15) und die
/// internen Felder.
fn require_data_view<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
) -> Completion<View<'s>> {
    let heap = agent.heap();
    let view = match heap.get(value).kind() {
        Kind::Object(object) if heap.object_class(object) == Some(ObjectClass::DataView) => {
            let buffer = heap
                .data_view_field(object, VIEWED_BUFFER)
                .unwrap_or(Value::UNDEFINED);
            Some(View {
                buffer: heap.local(scope, buffer),
                byte_length: field_length(heap.data_view_field(object, BYTE_LENGTH)),
                byte_offset: field_length(heap.data_view_field(object, BYTE_OFFSET)).unwrap_or(0),
            })
        }
        _ => None,
    };
    view.ok_or_else(|| agent.throw_error(ErrorKind::TypeError))
}

/// MakeDataViewWithBufferWitnessRecord (Abschnitt 25.3.1.2), dann
/// IsViewOutOfBounds und GetViewByteLength (Abschnitte 25.3.1.4 und
/// 25.3.1.3): die Länge der Sicht, `None`, wenn sie außerhalb liegt.
fn view_byte_length(agent: &Agent, view: &View<'_>) -> Option<usize> {
    if buffer::is_detached(agent, view.buffer) {
        return None;
    }
    let buffer_length = buffer::byte_length(agent, view.buffer);
    let end = match view.byte_length {
        Some(length) => view.byte_offset.checked_add(length)?,
        None => buffer_length,
    };
    if view.byte_offset > buffer_length || end > buffer_length {
        return None;
    }
    Some(end - view.byte_offset)
}

fn number<'s>(agent: &Agent, scope: &'s Scope, value: usize) -> Local<'s> {
    // Längen von Blöcken liegen unter 2⁵³ und sind als Number exakt.
    agent.heap().local(scope, Value::number(value as f64))
}

// Der Konstruktor

/// DataView ( buffer [ , byteOffset [ , byteLength ] ] ) (Abschnitt
/// 25.3.2.1).
fn data_view_constructor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let buffer = arguments.argument(agent, scope, 0);
    if !buffer::has_array_buffer_data(agent, buffer) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let byte_offset = arguments.argument(agent, scope, 1);
    let offset = to_index(agent, scope, byte_offset)?;
    if buffer::is_detached(agent, buffer) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let buffer_length = u64::try_from(buffer::byte_length(agent, buffer)).unwrap_or(u64::MAX);
    if offset > buffer_length {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let fixed_length = buffer::max_byte_length(agent, buffer).is_none();
    let byte_length = arguments.argument(agent, scope, 2);
    let length_given = !agent.heap().get(byte_length).is_undefined();
    // `None` steht für ~auto~.
    let view_byte_length = if length_given {
        let length = to_index(agent, scope, byte_length)?;
        if offset.saturating_add(length) > buffer_length {
            return Err(agent.throw_error(ErrorKind::RangeError));
        }
        Some(length)
    } else if fixed_length {
        Some(buffer_length - offset)
    } else {
        None
    };
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::DataViewPrototype,
        arguments.realm,
    )?;
    // GetPrototypeFromConstructor kann den ArrayBuffer abgelöst oder
    // verkleinert haben.
    if buffer::is_detached(agent, buffer) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let buffer_length = u64::try_from(buffer::byte_length(agent, buffer)).unwrap_or(u64::MAX);
    if offset > buffer_length {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    if length_given
        && view_byte_length.is_some_and(|length| offset.saturating_add(length) > buffer_length)
    {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let shape = agent.empty_shape(scope);
    let object = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::DataView,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    if let Kind::Object(reference) = heap.get(object).kind() {
        heap.set_data_view_field(reference, VIEWED_BUFFER, heap.get(buffer));
        // Beide liegen unter der Länge des Blocks und sind als Number
        // exakt.
        let length =
            view_byte_length.map_or(Value::UNDEFINED, |length| Value::number(length as f64));
        heap.set_data_view_field(reference, BYTE_LENGTH, length);
        heap.set_data_view_field(reference, BYTE_OFFSET, Value::number(offset as f64));
    }
    Ok(object)
}

// Die Getter

/// get DataView.prototype.buffer (Abschnitt 25.3.4.1).
fn buffer_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(require_data_view(agent, scope, arguments.this)?.buffer)
}

/// get DataView.prototype.byteLength (Abschnitt 25.3.4.2).
fn byte_length_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let view = require_data_view(agent, scope, arguments.this)?;
    let size = view_byte_length(agent, &view).unwrap_or(0);
    Ok(number(agent, scope, size))
}

/// get DataView.prototype.byteOffset (Abschnitt 25.3.4.3).
fn byte_offset_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let view = require_data_view(agent, scope, arguments.this)?;
    let offset = if view_byte_length(agent, &view).is_some() {
        view.byte_offset
    } else {
        0
    };
    Ok(number(agent, scope, offset))
}

// Lesen und Schreiben

/// Prüft, dass `size` Bytes ab `index` in der Sicht liegen (GetViewValue
/// und SetViewValue, ab Schritt 5), und gibt den Index im ArrayBuffer
/// zurück.
fn buffer_index(agent: &mut Agent, view: &View<'_>, index: u64, size: usize) -> Completion<usize> {
    let Some(view_size) = view_byte_length(agent, view) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let end = usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(size));
    match end {
        Some(end) if end <= view_size => Ok(end - size + view.byte_offset),
        _ => Err(agent.throw_error(ErrorKind::RangeError)),
    }
}

/// GetViewValue (Abschnitt 25.3.1.5). Das Argument 1 ist `littleEndian`;
/// fehlt es, ist es *undefined* und damit *false*.
fn get_view_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    kind: ElementType,
) -> Completion<Local<'s>> {
    let view = require_data_view(agent, scope, arguments.this)?;
    let request_index = arguments.argument(agent, scope, 0);
    let index = to_index(agent, scope, request_index)?;
    let little_endian = arguments.argument(agent, scope, 1);
    let little = to_boolean(agent.heap(), agent.heap().get(little_endian));
    let index = buffer_index(agent, &view, index, kind.size())?;
    buffer::get_value_from_buffer(agent, scope, view.buffer, index, kind, little)
}

/// SetViewValue (Abschnitt 25.3.1.6). Das Argument 1 ist der Wert, das
/// Argument 2 `littleEndian`.
fn set_view_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    kind: ElementType,
) -> Completion<Local<'s>> {
    let view = require_data_view(agent, scope, arguments.this)?;
    let request_index = arguments.argument(agent, scope, 0);
    let index = to_index(agent, scope, request_index)?;
    let value = arguments.argument(agent, scope, 1);
    let value = if kind.is_bigint() {
        to_bigint(agent, scope, value)?
    } else {
        let number = to_number(agent, scope, value)?;
        agent.heap().local(scope, Value::number(number))
    };
    let little_endian = arguments.argument(agent, scope, 2);
    let little = to_boolean(agent.heap(), agent.heap().get(little_endian));
    let index = buffer_index(agent, &view, index, kind.size())?;
    buffer::set_value_in_buffer(agent, view.buffer, index, kind, value, little);
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Die Methoden von %DataView.prototype% für eine Art von Elementen.
macro_rules! view_methods {
    ($($get:ident, $set:ident => $kind:expr,)*) => {
        $(
            fn $get<'s>(
                agent: &mut Agent,
                scope: &'s Scope,
                arguments: &BuiltinCall<'_, '_>,
            ) -> Completion<Local<'s>> {
                get_view_value(agent, scope, arguments, $kind)
            }

            fn $set<'s>(
                agent: &mut Agent,
                scope: &'s Scope,
                arguments: &BuiltinCall<'_, '_>,
            ) -> Completion<Local<'s>> {
                set_view_value(agent, scope, arguments, $kind)
            }
        )*
    };
}

// Abschnitte 25.3.4.5 bis 25.3.4.26.
view_methods! {
    get_big_int64, set_big_int64 => ElementType::BigInt64,
    get_big_uint64, set_big_uint64 => ElementType::BigUint64,
    get_float16, set_float16 => ElementType::Float16,
    get_float32, set_float32 => ElementType::Float32,
    get_float64, set_float64 => ElementType::Float64,
    get_int8, set_int8 => ElementType::Int8,
    get_int16, set_int16 => ElementType::Int16,
    get_int32, set_int32 => ElementType::Int32,
    get_uint8, set_uint8 => ElementType::Uint8,
    get_uint16, set_uint16 => ElementType::Uint16,
    get_uint32, set_uint32 => ElementType::Uint32,
}
