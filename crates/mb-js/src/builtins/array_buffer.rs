//! %ArrayBuffer% (Abschnitte 25.1.4 und 25.1.5) und %ArrayBuffer.prototype%
//! (Abschnitt 25.1.6), ein gewöhnliches Objekt ohne [[ArrayBufferData]].
//! Die abstrakten Operationen und die Festlegungen stehen in
//! [`crate::buffer`].

use super::{
    define_functions, define_getter, define_species, define_to_string_tag, link_constructor, units,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::buffer::{self, allocate_array_buffer};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::compare::same_value;
use crate::conversion::{to_index, to_integer_or_infinity};
use crate::heap::{Local, ObjectClass, Scope};
use crate::object::ordinary_object_create;
use crate::operations::{construct, get, species_constructor};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "ArrayBuffer",
    length: 1,
    constructor: true,
    behaviour: array_buffer_constructor,
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

static CONSTRUCTOR_FUNCTIONS: [Builtin; 1] = [method("isView", 1, is_view)];

static PROTOTYPE_FUNCTIONS: [Builtin; 4] = [
    method("resize", 1, resize),
    method("slice", 2, slice),
    method("transfer", 0, transfer),
    method("transferToFixedLength", 0, transfer_to_fixed_length),
];

static GETTERS: [(&str, Builtin); 4] = [
    (
        "byteLength",
        method("get byteLength", 0, byte_length_getter),
    ),
    ("detached", method("get detached", 0, detached_getter)),
    (
        "maxByteLength",
        method("get maxByteLength", 0, max_byte_length_getter),
    ),
    ("resizable", method("get resizable", 0, resizable_getter)),
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(object))?;
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    define_species(agent, scope, realm, constructor)?;
    for (name, getter) in &GETTERS {
        define_getter(agent, scope, realm, prototype, name, getter)?;
    }
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    define_to_string_tag(agent, scope, prototype, "ArrayBuffer")?;
    agent.set_intrinsic(realm, Intrinsic::ArrayBuffer, constructor);
    agent.set_intrinsic(realm, Intrinsic::ArrayBufferPrototype, prototype);
    Ok(())
}

// Helfer

/// RequireInternalSlot(*this*, [[ArrayBufferData]]) mit dem Schritt
/// danach, der einen SharedArrayBuffer abweist; bis es die gibt, ist das
/// dieselbe Prüfung.
fn require_array_buffer(agent: &mut Agent, value: Local<'_>) -> Completion<()> {
    if buffer::is_array_buffer(agent, value) {
        Ok(())
    } else {
        Err(agent.throw_error(ErrorKind::TypeError))
    }
}

fn number<'s>(agent: &Agent, scope: &'s Scope, value: usize) -> Local<'s> {
    // Längen von Blöcken liegen unter 2⁵³ und sind als Number exakt.
    agent.heap().local(scope, Value::number(value as f64))
}

fn boolean<'s>(agent: &Agent, scope: &'s Scope, value: bool) -> Local<'s> {
    agent.heap().local(scope, Value::boolean(value))
}

/// GetArrayBufferMaxByteLengthOption (Abschnitt 25.1.3.7).
fn max_byte_length_option(
    agent: &mut Agent,
    scope: &Scope,
    options: Local<'_>,
) -> Completion<Option<u64>> {
    if !matches!(agent.heap().get(options).kind(), Kind::Object(_)) {
        return Ok(None);
    }
    let key = agent.key_from_units(scope, &units("maxByteLength"))?;
    let value = get(agent, scope, options, key)?;
    if agent.heap().get(value).is_undefined() {
        return Ok(None);
    }
    Ok(Some(to_index(agent, scope, value)?))
}

/// ToClampedIndex (Abschnitt 7.1.26) mit ToAbsoluteIndex (Abschnitt
/// 7.1.25).
fn to_clamped_index(
    agent: &mut Agent,
    scope: &Scope,
    value: Local<'_>,
    length: usize,
) -> Completion<usize> {
    let relative = to_integer_or_infinity(agent, scope, value)?;
    // Längen liegen unter 2⁵³ und sind als Number exakt.
    let length_number = length as f64;
    let index = if relative < 0.0 {
        (length_number + relative).max(0.0)
    } else {
        relative.min(length_number)
    };
    // Ganz und zwischen 0 und `length`.
    Ok(index as usize)
}

// Der Konstruktor

/// ArrayBuffer ( length [ , options ] ) (Abschnitt 25.1.4.1).
fn array_buffer_constructor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let length = arguments.argument(agent, scope, 0);
    let byte_length = to_index(agent, scope, length)?;
    let options = arguments.argument(agent, scope, 1);
    let max_byte_length = max_byte_length_option(agent, scope, options)?;
    allocate_array_buffer(
        agent,
        scope,
        new_target,
        byte_length,
        max_byte_length,
        arguments.realm,
    )
}

/// ArrayBuffer.isView ( arg ) (Abschnitt 25.1.5.1): ob `arg`
/// [[ViewedArrayBuffer]] hat, also ein DataView oder ein TypedArray ist.
fn is_view<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let heap = agent.heap();
    let view = match heap.get(value).kind() {
        Kind::Object(object) => matches!(
            heap.object_class(object),
            Some(ObjectClass::DataView | ObjectClass::TypedArray)
        ),
        _ => false,
    };
    Ok(boolean(agent, scope, view))
}

// Die Getter

/// get ArrayBuffer.prototype.byteLength (Abschnitt 25.1.6.1).
fn byte_length_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_array_buffer(agent, arguments.this)?;
    let length = buffer::byte_length(agent, arguments.this);
    Ok(number(agent, scope, length))
}

/// get ArrayBuffer.prototype.detached (Abschnitt 25.1.6.3).
fn detached_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_array_buffer(agent, arguments.this)?;
    let detached = buffer::is_detached(agent, arguments.this);
    Ok(boolean(agent, scope, detached))
}

/// get ArrayBuffer.prototype.maxByteLength (Abschnitt 25.1.6.4).
fn max_byte_length_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_array_buffer(agent, arguments.this)?;
    let this = arguments.this;
    if buffer::is_detached(agent, this) {
        return Ok(number(agent, scope, 0));
    }
    let length = match buffer::max_byte_length(agent, this) {
        // Das Maximum passt in den Heap und damit in `usize`.
        Some(max) => usize::try_from(max).unwrap_or(usize::MAX),
        None => buffer::byte_length(agent, this),
    };
    Ok(number(agent, scope, length))
}

/// get ArrayBuffer.prototype.resizable (Abschnitt 25.1.6.5).
fn resizable_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_array_buffer(agent, arguments.this)?;
    let resizable = buffer::max_byte_length(agent, arguments.this).is_some();
    Ok(boolean(agent, scope, resizable))
}

// Die Methoden

/// ArrayBuffer.prototype.resize ( newLength ) (Abschnitt 25.1.6.6).
fn resize<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    // RequireInternalSlot(*this*, [[ArrayBufferMaxByteLength]]): nur ein
    // veränderlicher ArrayBuffer hat es.
    let Some(max) = buffer::max_byte_length(agent, this) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let new_length = arguments.argument(agent, scope, 0);
    let new_byte_length = to_index(agent, scope, new_length)?;
    if buffer::is_detached(agent, this) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if new_byte_length > max {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    // HostResizeArrayBuffer gibt ~unhandled~.
    let new_block = buffer::create_byte_data_block(agent, scope, new_byte_length)?;
    let old_length = buffer::byte_length(agent, this);
    // Höchstens das Maximum, und das passt in den Heap.
    let copy_length = usize::try_from(new_byte_length)
        .unwrap_or(usize::MAX)
        .min(old_length);
    if let Some(old_block) = buffer::data_block_local(agent, scope, this) {
        buffer::copy_block_bytes(agent, new_block, 0, old_block, 0, copy_length)?;
    }
    buffer::set_data_block(agent, this, new_block);
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// ArrayBuffer.prototype.slice ( start, end ) (Abschnitt 25.1.6.7).
fn slice<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    require_array_buffer(agent, this)?;
    if buffer::is_detached(agent, this) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let length = buffer::byte_length(agent, this);
    let start = arguments.argument(agent, scope, 0);
    let first = to_clamped_index(agent, scope, start, length)?;
    let end = arguments.argument(agent, scope, 1);
    let last = if agent.heap().get(end).is_undefined() {
        length
    } else {
        to_clamped_index(agent, scope, end, length)?
    };
    let new_length = last.saturating_sub(first);
    let default = agent.intrinsic(scope, arguments.realm, Intrinsic::ArrayBuffer);
    let constructor = species_constructor(agent, scope, this, default)?;
    let count = number(agent, scope, new_length);
    let new = construct(agent, scope, arguments.realm, constructor, &[count], None)?;
    require_array_buffer(agent, new)?;
    if buffer::is_detached(agent, new) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if same_value(agent.heap(), agent.heap().get(new), agent.heap().get(this)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if buffer::byte_length(agent, new) < new_length {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    // Die Schritte davor können *this* abgelöst oder verkleinert haben.
    if buffer::is_detached(agent, this) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let current_length = buffer::byte_length(agent, this);
    if current_length > first {
        let count = new_length.min(current_length - first);
        if let (Some(from), Some(to)) = (
            buffer::data_block_local(agent, scope, this),
            buffer::data_block_local(agent, scope, new),
        ) {
            buffer::copy_block_bytes(agent, to, 0, from, first, count)?;
        }
    }
    Ok(new)
}

/// ArrayBuffer.prototype.transfer ( [ newLength ] ) (Abschnitt 25.1.6.8).
fn transfer<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let new_length = arguments.argument(agent, scope, 0);
    copy_and_detach(agent, scope, arguments, new_length, true)
}

/// ArrayBuffer.prototype.transferToFixedLength ( [ newLength ] )
/// (Abschnitt 25.1.6.9).
fn transfer_to_fixed_length<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let new_length = arguments.argument(agent, scope, 0);
    copy_and_detach(agent, scope, arguments, new_length, false)
}

/// ArrayBufferCopyAndDetach (Abschnitt 25.1.3.3) mit *this*;
/// `preserve_resizability` steht für ~preserve-resizability~.
fn copy_and_detach<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    new_length: Local<'_>,
    preserve_resizability: bool,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    require_array_buffer(agent, this)?;
    let new_byte_length = if agent.heap().get(new_length).is_undefined() {
        u64::try_from(buffer::byte_length(agent, this)).unwrap_or(u64::MAX)
    } else {
        to_index(agent, scope, new_length)?
    };
    if buffer::is_detached(agent, this) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let new_max = if preserve_resizability {
        buffer::max_byte_length(agent, this)
    } else {
        None
    };
    // [[ArrayBufferDetachKey]] ist immer ~empty~.
    let constructor = agent.intrinsic(scope, arguments.realm, Intrinsic::ArrayBuffer);
    let new = allocate_array_buffer(
        agent,
        scope,
        constructor,
        new_byte_length,
        new_max,
        arguments.realm,
    )?;
    let copy_length = buffer::byte_length(agent, new).min(buffer::byte_length(agent, this));
    if let (Some(from), Some(to)) = (
        buffer::data_block_local(agent, scope, this),
        buffer::data_block_local(agent, scope, new),
    ) {
        buffer::copy_block_bytes(agent, to, 0, from, 0, copy_length)?;
    }
    buffer::detach_array_buffer(agent, this);
    Ok(new)
}
