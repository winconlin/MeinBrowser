//! %TypedArray% (Abschnitte 23.2.1 und 23.2.2), die zwölf Konstruktoren
//! für TypedArrays mit ihren Prototypen (Abschnitte 23.2.5 bis 23.2.7)
//! und die abstrakten Operationen aus Abschnitt 23.2.4.
//! %TypedArray.prototype% (Abschnitt 23.2.3) steht in `prototype`, die
//! Zusätze von %Uint8Array% für base64 und hex (Abschnitt 23.3) in
//! `base64`.
//!
//! Die TypedArrays selbst, ihre Felder und ihre internen Methoden stehen in
//! [`crate::object`] (Modul `typed_array`). Gelesen und geschrieben wird
//! immer mit [[LittleEndian]] *true* wie dort.
//!
//! Jedes Element, das eine Schleife umwandelt, liest oder an eine Funktion
//! gibt, kostet einen Arbeitsschritt wie bei den Methoden von Arrays.
//! Kopien von Bytes, die die Bitmuster erhalten, bezahlen dagegen wie
//! `ArrayBuffer` 256 Bytes je Schritt (siehe [`crate::buffer`]).

mod base64;
mod prototype;

use super::array::{index_key, step};
use super::{define_functions, define_species, link_constructor, units};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::buffer::{self, ElementType};
use crate::builtin::{
    Builtin, BuiltinCall, builtin_function_with_prototype, create_builtin_function,
};
use crate::conversion::{to_index, to_integer_or_infinity, to_object};
use crate::descriptor::PropertyDescriptor;
use crate::function::is_constructor;
use crate::heap::{Local, Scope};
use crate::iterator::{get_iterator_from_method, get_method_of_value, iterator_step_value};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{
    TypedArrayRecord, get_prototype_from_constructor, initialize_typed_array,
    ordinary_object_create, typed_array_create, typed_array_record_of, typed_array_set_element,
    viewed_buffer,
};
use crate::operations::{
    call, construct, define_property_or_throw, get, get_method, is_callable, length_of_array_like,
    species_constructor,
};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static TYPED_ARRAY: Builtin = Builtin {
    name: "TypedArray",
    length: 0,
    constructor: true,
    behaviour: typed_array_abstract,
};

static TYPED_ARRAY_FUNCTIONS: [Builtin; 2] = [
    Builtin {
        name: "from",
        length: 1,
        constructor: false,
        behaviour: typed_array_from,
    },
    Builtin {
        name: "of",
        length: 0,
        constructor: false,
        behaviour: typed_array_of,
    },
];

/// Ein Konstruktor aus der Tabelle in Abschnitt 23.2: Länge 3 (Abschnitt
/// 23.2.6) und sein Name.
const fn constructor(name: &'static str, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name,
        length: 3,
        constructor: true,
        behaviour,
    }
}

/// Die Konstruktoren in der Reihenfolge von [`ElementType::ALL`].
static CONSTRUCTORS: [Builtin; 12] = [
    constructor("Int8Array", int8_array),
    constructor("Uint8Array", uint8_array),
    constructor("Uint8ClampedArray", uint8_clamped_array),
    constructor("Int16Array", int16_array),
    constructor("Uint16Array", uint16_array),
    constructor("Int32Array", int32_array),
    constructor("Uint32Array", uint32_array),
    constructor("BigInt64Array", big_int64_array),
    constructor("BigUint64Array", big_uint64_array),
    constructor("Float16Array", float16_array),
    constructor("Float32Array", float32_array),
    constructor("Float64Array", float64_array),
];

/// Der Konstruktor zu einer Art von Elementen als Intrinsic.
pub(crate) fn constructor_intrinsic(kind: ElementType) -> Intrinsic {
    match kind {
        ElementType::Int8 => Intrinsic::Int8Array,
        ElementType::Uint8 => Intrinsic::Uint8Array,
        ElementType::Uint8Clamped => Intrinsic::Uint8ClampedArray,
        ElementType::Int16 => Intrinsic::Int16Array,
        ElementType::Uint16 => Intrinsic::Uint16Array,
        ElementType::Int32 => Intrinsic::Int32Array,
        ElementType::Uint32 => Intrinsic::Uint32Array,
        ElementType::BigInt64 => Intrinsic::BigInt64Array,
        ElementType::BigUint64 => Intrinsic::BigUint64Array,
        ElementType::Float16 => Intrinsic::Float16Array,
        ElementType::Float32 => Intrinsic::Float32Array,
        ElementType::Float64 => Intrinsic::Float64Array,
    }
}

/// Der Prototyp zu einer Art von Elementen als Intrinsic.
fn prototype_intrinsic(kind: ElementType) -> Intrinsic {
    match kind {
        ElementType::Int8 => Intrinsic::Int8ArrayPrototype,
        ElementType::Uint8 => Intrinsic::Uint8ArrayPrototype,
        ElementType::Uint8Clamped => Intrinsic::Uint8ClampedArrayPrototype,
        ElementType::Int16 => Intrinsic::Int16ArrayPrototype,
        ElementType::Uint16 => Intrinsic::Uint16ArrayPrototype,
        ElementType::Int32 => Intrinsic::Int32ArrayPrototype,
        ElementType::Uint32 => Intrinsic::Uint32ArrayPrototype,
        ElementType::BigInt64 => Intrinsic::BigInt64ArrayPrototype,
        ElementType::BigUint64 => Intrinsic::BigUint64ArrayPrototype,
        ElementType::Float16 => Intrinsic::Float16ArrayPrototype,
        ElementType::Float32 => Intrinsic::Float32ArrayPrototype,
        ElementType::Float64 => Intrinsic::Float64ArrayPrototype,
    }
}

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object_prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(object_prototype))?;
    let typed_array = create_builtin_function(agent, scope, realm, &TYPED_ARRAY)?;
    link_constructor(agent, scope, typed_array, prototype)?;
    define_functions(agent, scope, realm, typed_array, &TYPED_ARRAY_FUNCTIONS)?;
    define_species(agent, scope, realm, typed_array)?;
    prototype::define(agent, scope, realm, prototype)?;
    agent.set_intrinsic(realm, Intrinsic::TypedArray, typed_array);
    agent.set_intrinsic(realm, Intrinsic::TypedArrayPrototype, prototype);
    for (kind, builtin) in ElementType::ALL.into_iter().zip(&CONSTRUCTORS) {
        let constructor =
            builtin_function_with_prototype(agent, scope, realm, builtin, typed_array)?;
        let own_prototype = ordinary_object_create(agent, scope, Some(prototype))?;
        link_constructor(agent, scope, constructor, own_prototype)?;
        // BYTES_PER_ELEMENT (Abschnitte 23.2.6.1 und 23.2.7.1): weder
        // beschreibbar noch aufzählbar noch konfigurierbar.
        let size = number(agent, scope, kind.size() as f64);
        let descriptor = PropertyDescriptor::data(size, false, false, false);
        for object in [constructor, own_prototype] {
            let key = agent.key_from_units(scope, &units("BYTES_PER_ELEMENT"))?;
            define_property_or_throw(agent, scope, object, key, &descriptor)?;
        }
        if kind == ElementType::Uint8 {
            base64::define(agent, scope, realm, constructor, own_prototype)?;
        }
        agent.set_intrinsic(realm, constructor_intrinsic(kind), constructor);
        agent.set_intrinsic(realm, prototype_intrinsic(kind), own_prototype);
    }
    Ok(())
}

// Helfer

fn number<'s>(agent: &Agent, scope: &'s Scope, value: f64) -> Local<'s> {
    agent.heap().local(scope, Value::number(value))
}

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

/// Eine Länge oder ein Index als Number; alle liegen unter 2⁵³.
fn to_f64(value: u64) -> f64 {
    value as f64
}

/// ValidateTypedArray (Abschnitt 23.2.4.4) mit ValidateTypedArrayBounds
/// (Abschnitt 23.2.4.5): ein TypeError für einen anderen Wert und für ein
/// TypedArray außerhalb der Grenzen seines ArrayBuffer.
pub(super) fn validate_typed_array(
    agent: &mut Agent,
    value: Local<'_>,
) -> Completion<TypedArrayRecord> {
    match typed_array_record_of(agent, value) {
        Some(record) if !record.is_out_of_bounds() => Ok(record),
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// RequireInternalSlot(`value`, [[TypedArrayName]]) (Abschnitt 10.1.15)
/// und MakeTypedArrayWithBufferWitnessRecord.
pub(super) fn require_typed_array(
    agent: &mut Agent,
    value: Local<'_>,
) -> Completion<TypedArrayRecord> {
    match typed_array_record_of(agent, value) {
        Some(record) => Ok(record),
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// ToAbsoluteIndex (Abschnitt 7.1.25) für eine Länge unter 2⁵³.
pub(super) fn to_absolute_index(
    agent: &mut Agent,
    scope: &Scope,
    value: Local<'_>,
    length: u64,
) -> Completion<f64> {
    let index = to_integer_or_infinity(agent, scope, value)?;
    Ok(if index.is_finite() && index < 0.0 {
        to_f64(length) + index
    } else {
        index
    })
}

/// ToClampedIndex (Abschnitt 7.1.26): eine ganze Zahl von 0 bis `length`.
pub(super) fn to_clamped_index(
    agent: &mut Agent,
    scope: &Scope,
    value: Local<'_>,
    length: u64,
) -> Completion<u64> {
    let index = to_absolute_index(agent, scope, value, length)?;
    // Ganz und zwischen 0 und einer Länge unter 2⁵³.
    Ok(index.clamp(0.0, to_f64(length)) as u64)
}

/// CloneArrayBuffer (Abschnitt 25.1.3.6) für einen nicht abgelösten
/// ArrayBuffer: ein neuer mit `length` Bytes ab `offset`. `current` ist
/// der aktuelle Realm für %ArrayBuffer%.
pub(super) fn clone_array_buffer<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    current: RealmId,
    source: Local<'_>,
    offset: u64,
    length: u64,
) -> Completion<Local<'s>> {
    let constructor = agent.intrinsic(scope, current, Intrinsic::ArrayBuffer);
    let target = buffer::allocate_array_buffer(agent, scope, constructor, length, None, current)?;
    copy_buffer_bytes(agent, target, 0, source, offset, length)?;
    Ok(target)
}

/// Kopiert `count` Bytes zwischen den Blöcken zweier ArrayBuffer wie
/// `memmove`, bezahlt wie [`buffer::copy_block_bytes`]. Ist einer
/// abgelöst oder liegt ein Bereich nicht im Block, geschieht nichts.
pub(super) fn copy_buffer_bytes(
    agent: &mut Agent,
    to: Local<'_>,
    to_index: u64,
    from: Local<'_>,
    from_index: u64,
    count: u64,
) -> Completion<()> {
    let (Ok(to_index), Ok(from_index), Ok(count)) = (
        usize::try_from(to_index),
        usize::try_from(from_index),
        usize::try_from(count),
    ) else {
        return Ok(());
    };
    agent.scope(|agent, inner| {
        let (Some(to), Some(from)) = (
            buffer::data_block_local(agent, inner, to),
            buffer::data_block_local(agent, inner, from),
        ) else {
            return Ok(());
        };
        buffer::copy_block_bytes(agent, to, to_index, from, from_index, count)
    })
}

/// Liest das Element an `byte_index` der Art `kind` aus dem ArrayBuffer
/// `source` und schreibt es als Art `target_kind` an `target_index` in
/// `target` (GetValueFromBuffer und SetValueInBuffer ohne Umweg über den
/// Heap). Beide Arten haben Numbers oder beide BigInts.
pub(super) fn convert_element(
    agent: &Agent,
    target: Local<'_>,
    target_index: u64,
    target_kind: ElementType,
    source: Local<'_>,
    source_index: u64,
    source_kind: ElementType,
) {
    let heap = agent.heap();
    let (Some(target), Some(source)) = (object_ref(agent, target), object_ref(agent, source))
    else {
        return;
    };
    let (Ok(target_index), Ok(source_index)) =
        (usize::try_from(target_index), usize::try_from(source_index))
    else {
        return;
    };
    let mut raw = [0u8; 8];
    let Some(bytes) = raw.get_mut(..source_kind.size()) else {
        return;
    };
    if !buffer::read_block(heap, source, source_index, bytes) {
        return;
    }
    let value = buffer::raw_bytes_to_numeric(source_kind, &raw, true);
    let converted = buffer::numeric_to_raw_bytes(target_kind, value, true);
    if let Some(bytes) = converted.get(..target_kind.size()) {
        buffer::write_block(heap, target, target_index, bytes);
    }
}

/// Das Objekt hinter einem Handle.
fn object_ref<'h>(agent: &'h Agent, value: Local<'_>) -> Option<crate::value::Ref<'h>> {
    match agent.heap().get(value).kind() {
        Kind::Object(object) => Some(object),
        _ => None,
    }
}

/// IteratorToList (Abschnitt 7.4.12) mit GetIteratorFromMethod: Jeder
/// Wert kostet einen Arbeitsschritt.
fn iterator_to_list<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
    method: Local<'_>,
) -> Completion<Vec<Local<'s>>> {
    let (iterator, next) = get_iterator_from_method(agent, scope, value, method)?;
    let mut values = Vec::new();
    loop {
        step(agent)?;
        match iterator_step_value(agent, scope, iterator, next)? {
            Some(value) => values.push(value),
            None => return Ok(values),
        }
    }
}

/// TypedArraySetElement für den Index `index` als ganze Zahl.
pub(super) fn set_element(
    agent: &mut Agent,
    scope: &Scope,
    typed_array: Local<'_>,
    index: u64,
    value: Local<'_>,
) -> Completion<()> {
    typed_array_set_element(agent, scope, typed_array, to_f64(index), value)
}

// Abstrakte Operationen (Abschnitt 23.2.4)

/// TypedArrayCreateFromConstructor (Abschnitt 23.2.4.1). `current` ist
/// der aktuelle Realm für Construct.
pub(super) fn typed_array_create_from_constructor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    current: RealmId,
    constructor: Local<'_>,
    arguments: &[Local<'_>],
) -> Completion<(Local<'s>, TypedArrayRecord)> {
    let typed_array = construct(agent, scope, current, constructor, arguments, None)?;
    // ValidateTypedArray prüft auch die Grenzen (Schritt 4.a).
    let record = validate_typed_array(agent, typed_array)?;
    if let &[length] = arguments
        && let Some(length) = agent.heap().get(length).as_number()
        && to_f64(record.length()) < length
    {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok((typed_array, record))
}

/// TypedArrayCreateSameType (Abschnitt 23.2.4.2) mit dem Konstruktor der
/// Art `kind` aus dem aktuellen Realm `current`.
pub(super) fn typed_array_create_same_type<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    current: RealmId,
    kind: ElementType,
    length: u64,
) -> Completion<Local<'s>> {
    let constructor = agent.intrinsic(scope, current, constructor_intrinsic(kind));
    let length = number(agent, scope, to_f64(length));
    Ok(typed_array_create_from_constructor(agent, scope, current, constructor, &[length])?.0)
}

/// TypedArraySpeciesCreate (Abschnitt 23.2.4.3) für das TypedArray
/// `exemplar` der Art `kind`; der Vorgabekonstruktor kommt aus dem
/// aktuellen Realm `current`.
pub(super) fn typed_array_species_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    current: RealmId,
    exemplar: Local<'_>,
    kind: ElementType,
    arguments: &[Local<'_>],
) -> Completion<(Local<'s>, TypedArrayRecord)> {
    let default = agent.intrinsic(scope, current, constructor_intrinsic(kind));
    let constructor = species_constructor(agent, scope, exemplar, default)?;
    let (result, record) =
        typed_array_create_from_constructor(agent, scope, current, constructor, arguments)?;
    if record.kind.is_bigint() != kind.is_bigint() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok((result, record))
}

// %TypedArray% und seine Funktionen

/// %TypedArray% ( ) (Abschnitt 23.2.1.1).
fn typed_array_abstract<'s>(
    agent: &mut Agent,
    _scope: &'s Scope,
    _arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Err(agent.throw_error(ErrorKind::TypeError))
}

/// Das Argument 1 von `from`, wenn es eine Funktion ist; `None` für
/// *undefined*.
fn mapper<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Option<Local<'s>>> {
    let mapper = arguments.argument(agent, scope, 1);
    let value = agent.heap().get(mapper);
    if value.is_undefined() {
        return Ok(None);
    }
    if !is_callable(agent.heap(), value) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(Some(mapper))
}

/// %TypedArray%.from ( source [ , mapper [ , thisArg ] ] ) (Abschnitt
/// 23.2.2.1).
fn typed_array_from<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let constructor = arguments.this;
    if !is_constructor(agent, agent.heap().get(constructor)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let mapper = mapper(agent, scope, arguments)?;
    let this_arg = arguments.argument(agent, scope, 2);
    let source = arguments.argument(agent, scope, 0);
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::Iterator);
    let using_iterator =
        get_method_of_value(agent, scope, realm, source, PropertyKey::Symbol(symbol))?;
    // Die Werte: aus dem Iterator oder aus dem Objekt, das wie ein Array
    // aussieht.
    let (values, array_like, length) = if let Some(method) = using_iterator {
        let values = iterator_to_list(agent, scope, source, method)?;
        let length = u64::try_from(values.len()).unwrap_or(u64::MAX);
        (values, None, length)
    } else {
        let array_like = to_object(agent, scope, realm, source)?;
        let length = length_of_array_like(agent, scope, array_like)?;
        // LengthOfArrayLike liefert eine ganze Zahl bis 2⁵³ − 1.
        (Vec::new(), Some(array_like), length as u64)
    };
    let count = number(agent, scope, to_f64(length));
    let (target, _) =
        typed_array_create_from_constructor(agent, scope, realm, constructor, &[count])?;
    for k in 0..length {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let value = match array_like {
                Some(array_like) => {
                    let key = index_key(agent, inner, to_f64(k))?;
                    get(agent, inner, array_like, key)?
                }
                None => {
                    let found = usize::try_from(k).ok().and_then(|k| values.get(k)).copied();
                    let heap = agent.heap();
                    heap.local(
                        inner,
                        found.map_or(Value::UNDEFINED, |value| heap.get(value)),
                    )
                }
            };
            let mapped = match mapper {
                Some(mapper) => {
                    let index = number(agent, inner, to_f64(k));
                    call(agent, inner, mapper, this_arg, &[value, index])?
                }
                None => value,
            };
            set_element(agent, inner, target, k, mapped)
        })?;
    }
    Ok(target)
}

/// %TypedArray%.of ( ...items ) (Abschnitt 23.2.2.2).
fn typed_array_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let constructor = arguments.this;
    if !is_constructor(agent, agent.heap().get(constructor)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let items = arguments.arguments;
    let length = u64::try_from(items.len()).unwrap_or(u64::MAX);
    let count = number(agent, scope, to_f64(length));
    let (target, _) =
        typed_array_create_from_constructor(agent, scope, arguments.realm, constructor, &[count])?;
    for (k, &item) in (0..length).zip(items) {
        step(agent)?;
        set_element(agent, scope, target, k, item)?;
    }
    Ok(target)
}

// Die Konstruktoren (Abschnitt 23.2.5)

macro_rules! constructors {
    ($($name:ident => $kind:ident,)*) => {
        $(
            fn $name<'s>(
                agent: &mut Agent,
                scope: &'s Scope,
                arguments: &BuiltinCall<'_, '_>,
            ) -> Completion<Local<'s>> {
                typed_array_constructor(agent, scope, arguments, ElementType::$kind)
            }
        )*
    };
}

constructors! {
    int8_array => Int8,
    uint8_array => Uint8,
    uint8_clamped_array => Uint8Clamped,
    int16_array => Int16,
    uint16_array => Uint16,
    int32_array => Int32,
    uint32_array => Uint32,
    big_int64_array => BigInt64,
    big_uint64_array => BigUint64,
    float16_array => Float16,
    float32_array => Float32,
    float64_array => Float64,
}

/// _TypedArray_ ( ...args ) (Abschnitt 23.2.5.1) für die Art `kind`.
fn typed_array_constructor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    kind: ElementType,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let realm = arguments.realm;
    let Some(&first) = arguments.arguments.first() else {
        return allocate_typed_array(agent, scope, realm, kind, new_target, Some(0));
    };
    if !is_object(agent, first) {
        let length = to_index(agent, scope, first)?;
        return allocate_typed_array(agent, scope, realm, kind, new_target, Some(length));
    }
    let object = allocate_typed_array(agent, scope, realm, kind, new_target, None)?;
    if typed_array_record_of(agent, first).is_some() {
        initialize_from_typed_array(agent, scope, realm, object, kind, first)?;
    } else if buffer::has_array_buffer_data(agent, first) {
        let byte_offset = arguments.argument(agent, scope, 1);
        let length = arguments.argument(agent, scope, 2);
        initialize_from_array_buffer(agent, scope, object, kind, first, byte_offset, length)?;
    } else {
        initialize_from_elements(agent, scope, realm, object, kind, first)?;
    }
    Ok(object)
}

/// AllocateTypedArray (Abschnitt 23.2.5.1.1); `current` ist der aktuelle
/// Realm.
pub(super) fn allocate_typed_array<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    current: RealmId,
    kind: ElementType,
    new_target: Local<'_>,
    length: Option<u64>,
) -> Completion<Local<'s>> {
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        prototype_intrinsic(kind),
        current,
    )?;
    let object = typed_array_create(agent, scope, prototype, kind)?;
    if let Some(length) = length {
        allocate_typed_array_buffer(agent, scope, current, object, kind, length)?;
    }
    Ok(object)
}

/// AllocateTypedArrayBuffer (Abschnitt 23.2.5.1.5). Eine Länge, deren
/// Bytes es nicht geben kann, ist wie in CreateByteDataBlock ein
/// RangeError.
fn allocate_typed_array_buffer(
    agent: &mut Agent,
    scope: &Scope,
    current: RealmId,
    object: Local<'_>,
    kind: ElementType,
    length: u64,
) -> Completion<()> {
    let size = u64::try_from(kind.size()).unwrap_or(u64::MAX);
    let Some(byte_length) = length.checked_mul(size) else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    let constructor = agent.intrinsic(scope, current, Intrinsic::ArrayBuffer);
    let data =
        buffer::allocate_array_buffer(agent, scope, constructor, byte_length, None, current)?;
    initialize_typed_array(agent, object, data, Some(length), 0);
    Ok(())
}

/// InitializeTypedArrayFromTypedArray (Abschnitt 23.2.5.1.2).
fn initialize_from_typed_array(
    agent: &mut Agent,
    scope: &Scope,
    current: RealmId,
    object: Local<'_>,
    kind: ElementType,
    source: Local<'_>,
) -> Completion<()> {
    let source_data = viewed_buffer(agent, scope, source);
    let source_record = validate_typed_array(agent, source)?;
    let source_kind = source_record.kind;
    let element_length = source_record.length();
    let size = u64::try_from(kind.size()).unwrap_or(u64::MAX);
    let Some(byte_length) = element_length.checked_mul(size) else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    let source_offset = source_record.byte_offset;
    let data = if kind == source_kind {
        clone_array_buffer(
            agent,
            scope,
            current,
            source_data,
            source_offset,
            byte_length,
        )?
    } else {
        let constructor = agent.intrinsic(scope, current, Intrinsic::ArrayBuffer);
        let data =
            buffer::allocate_array_buffer(agent, scope, constructor, byte_length, None, current)?;
        if source_kind.is_bigint() != kind.is_bigint() {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        let source_size = u64::try_from(source_kind.size()).unwrap_or(u64::MAX);
        for k in 0..element_length {
            step(agent)?;
            let source_index = source_offset.saturating_add(k.saturating_mul(source_size));
            let target_index = k.saturating_mul(size);
            convert_element(
                agent,
                data,
                target_index,
                kind,
                source_data,
                source_index,
                source_kind,
            );
        }
        data
    };
    initialize_typed_array(agent, object, data, Some(element_length), 0);
    Ok(())
}

/// InitializeTypedArrayFromArrayBuffer (Abschnitt 23.2.5.1.3).
fn initialize_from_array_buffer(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    kind: ElementType,
    buffer: Local<'_>,
    byte_offset: Local<'_>,
    length: Local<'_>,
) -> Completion<()> {
    let size = u64::try_from(kind.size()).unwrap_or(u64::MAX);
    let offset = to_index(agent, scope, byte_offset)?;
    if offset % size != 0 {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let fixed_length = buffer::max_byte_length(agent, buffer).is_none();
    let new_length = if agent.heap().get(length).is_undefined() {
        None
    } else {
        Some(to_index(agent, scope, length)?)
    };
    if buffer::is_detached(agent, buffer) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let buffer_byte_length = u64::try_from(buffer::byte_length(agent, buffer)).unwrap_or(u64::MAX);
    let array_length = match new_length {
        None if !fixed_length => {
            if offset > buffer_byte_length {
                return Err(agent.throw_error(ErrorKind::RangeError));
            }
            None
        }
        None => {
            if buffer_byte_length % size != 0 || offset > buffer_byte_length {
                return Err(agent.throw_error(ErrorKind::RangeError));
            }
            Some((buffer_byte_length - offset) / size)
        }
        Some(new_length) => {
            let fits = new_length
                .checked_mul(size)
                .and_then(|bytes| offset.checked_add(bytes))
                .is_some_and(|end| end <= buffer_byte_length);
            if !fits {
                return Err(agent.throw_error(ErrorKind::RangeError));
            }
            Some(new_length)
        }
    };
    initialize_typed_array(agent, object, buffer, array_length, offset);
    Ok(())
}

/// InitializeTypedArrayFromElements (Abschnitt 23.2.5.1.4).
fn initialize_from_elements(
    agent: &mut Agent,
    scope: &Scope,
    current: RealmId,
    object: Local<'_>,
    kind: ElementType,
    source: Local<'_>,
) -> Completion<()> {
    let symbol = agent.well_known_symbol(scope, WellKnownSymbol::Iterator);
    let using_iterator = get_method(agent, scope, source, PropertyKey::Symbol(symbol))?;
    if let Some(method) = using_iterator {
        let values = iterator_to_list(agent, scope, source, method)?;
        let length = u64::try_from(values.len()).unwrap_or(u64::MAX);
        allocate_typed_array_buffer(agent, scope, current, object, kind, length)?;
        for (k, &value) in (0..length).zip(&values) {
            step(agent)?;
            set_element(agent, scope, object, k, value)?;
        }
        return Ok(());
    }
    // LengthOfArrayLike liefert eine ganze Zahl bis 2⁵³ − 1.
    let length = length_of_array_like(agent, scope, source)? as u64;
    allocate_typed_array_buffer(agent, scope, current, object, kind, length)?;
    for k in 0..length {
        step(agent)?;
        agent.scope(|agent, inner| -> Completion<()> {
            let key = index_key(agent, inner, to_f64(k))?;
            let value = get(agent, inner, source, key)?;
            set_element(agent, inner, object, k, value)
        })?;
    }
    Ok(())
}
