//! %SharedArrayBuffer% (Abschnitte 25.2.3 und 25.2.4) und
//! %SharedArrayBuffer.prototype% (Abschnitt 25.2.5), ein gewöhnliches
//! Objekt ohne [[ArrayBufferData]]. Die Bytes liegen in einem geteilten
//! Block außerhalb des Heaps (siehe `crate::shared`), die abstrakten
//! Operationen stehen in [`crate::buffer`].
//!
//! Festlegungen:
//! - HostGrowSharedArrayBuffer (Abschnitt 25.2.2.3) gibt immer
//!   ~unhandled~. Weil der Block gleich mit [[ArrayBufferMaxByteLength]]
//!   Bytes angelegt wird, gelingt `grow` bis dahin immer.

use super::array_buffer::{max_byte_length_option, to_clamped_index};
use super::{
    define_functions, define_getter, define_species, define_to_string_tag, link_constructor,
};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::buffer::{self, allocate_shared_array_buffer};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::to_index;
use crate::heap::{Local, Scope};
use crate::object::ordinary_object_create;
use crate::operations::{construct, species_constructor};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static CONSTRUCTOR: Builtin = Builtin {
    name: "SharedArrayBuffer",
    length: 1,
    constructor: true,
    behaviour: shared_array_buffer_constructor,
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

static PROTOTYPE_FUNCTIONS: [Builtin; 2] = [method("grow", 1, grow), method("slice", 2, slice)];

static GETTERS: [(&str, Builtin); 3] = [
    (
        "byteLength",
        method("get byteLength", 0, byte_length_getter),
    ),
    ("growable", method("get growable", 0, growable_getter)),
    (
        "maxByteLength",
        method("get maxByteLength", 0, max_byte_length_getter),
    ),
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(object))?;
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_species(agent, scope, realm, constructor)?;
    for (name, getter) in &GETTERS {
        define_getter(agent, scope, realm, prototype, name, getter)?;
    }
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)?;
    define_to_string_tag(agent, scope, prototype, "SharedArrayBuffer")?;
    agent.set_intrinsic(realm, Intrinsic::SharedArrayBuffer, constructor);
    agent.set_intrinsic(realm, Intrinsic::SharedArrayBufferPrototype, prototype);
    Ok(())
}

// Helfer

/// RequireInternalSlot(*this*, [[ArrayBufferData]]) mit dem Schritt
/// danach, der einen ArrayBuffer abweist, der nicht geteilt ist.
fn require_shared_array_buffer(agent: &mut Agent, value: Local<'_>) -> Completion<()> {
    if buffer::is_shared_array_buffer(agent, value) {
        Ok(())
    } else {
        Err(agent.throw_error(ErrorKind::TypeError))
    }
}

fn number<'s>(agent: &Agent, scope: &'s Scope, value: usize) -> Local<'s> {
    // Längen von Blöcken liegen unter 2⁵³ und sind als Number exakt.
    agent.heap().local(scope, Value::number(value as f64))
}

// Der Konstruktor

/// SharedArrayBuffer ( length [ , options ] ) (Abschnitt 25.2.3.1).
fn shared_array_buffer_constructor<'s>(
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
    allocate_shared_array_buffer(
        agent,
        scope,
        new_target,
        byte_length,
        max_byte_length,
        arguments.realm,
    )
}

// Die Getter

/// get SharedArrayBuffer.prototype.byteLength (Abschnitt 25.2.5.2).
fn byte_length_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_shared_array_buffer(agent, arguments.this)?;
    let length = buffer::byte_length(agent, arguments.this);
    Ok(number(agent, scope, length))
}

/// get SharedArrayBuffer.prototype.growable (Abschnitt 25.2.5.4).
fn growable_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    require_shared_array_buffer(agent, arguments.this)?;
    let growable = buffer::max_byte_length(agent, arguments.this).is_some();
    Ok(agent.heap().local(scope, Value::boolean(growable)))
}

/// get SharedArrayBuffer.prototype.maxByteLength (Abschnitt 25.2.5.5).
fn max_byte_length_getter<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    require_shared_array_buffer(agent, this)?;
    let length = match buffer::max_byte_length(agent, this) {
        // Das Maximum passt in den Heap und damit in `usize`.
        Some(max) => usize::try_from(max).unwrap_or(usize::MAX),
        None => buffer::byte_length(agent, this),
    };
    Ok(number(agent, scope, length))
}

// Die Methoden

/// SharedArrayBuffer.prototype.grow ( newLength ) (Abschnitt 25.2.5.3).
/// Die Schleife mit AtomicCompareExchangeInSharedBlock steht in
/// [`crate::shared::SharedBlock::grow`].
fn grow<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    // RequireInternalSlot(*this*, [[ArrayBufferMaxByteLength]]): nur ein
    // wachsender SharedArrayBuffer hat es, ein ArrayBuffer ist nicht
    // geteilt.
    if !buffer::is_shared_array_buffer(agent, this)
        || buffer::max_byte_length(agent, this).is_none()
    {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let new_length = arguments.argument(agent, scope, 0);
    let new_byte_length = to_index(agent, scope, new_length)?;
    let grown = match (
        buffer::shared_block(agent, this),
        usize::try_from(new_byte_length),
    ) {
        (Some(block), Ok(length)) => block.grow(length),
        _ => false,
    };
    if !grown {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// SharedArrayBuffer.prototype.slice ( start, end ) (Abschnitt 25.2.5.6).
/// Ein SharedArrayBuffer wird nie kürzer, also liegen die Bytes nach den
/// Aufrufen der Zwischenschritte noch im Block; die Länge wird nicht neu
/// gelesen.
fn slice<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    require_shared_array_buffer(agent, this)?;
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
    let default = agent.intrinsic(scope, arguments.realm, Intrinsic::SharedArrayBuffer);
    let constructor = species_constructor(agent, scope, this, default)?;
    let count = number(agent, scope, new_length);
    let new = construct(agent, scope, arguments.realm, constructor, &[count], None)?;
    require_shared_array_buffer(agent, new)?;
    let heap = agent.heap();
    if let (Kind::Object(left), Kind::Object(right)) = (heap.get(new).kind(), heap.get(this).kind())
        && buffer::same_shared_block(heap, left, right)
    {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if buffer::byte_length(agent, new) < new_length {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if let (Some(from), Some(to)) = (
        buffer::data_block_local(agent, scope, this),
        buffer::data_block_local(agent, scope, new),
    ) {
        buffer::copy_block_bytes(agent, to, 0, from, first, new_length)?;
    }
    Ok(new)
}
