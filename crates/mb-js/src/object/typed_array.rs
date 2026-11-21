//! TypedArrays (Abschnitt 10.4.5): ihre internen Felder, die abstrakten
//! Operationen der Abschnitte 10.4.5.9 bis 10.4.5.18 und die internen
//! Methoden, die sich von denen gewöhnlicher Objekte unterscheiden.
//!
//! Ein TypedArray ist ein Objekt der Art [`ObjectClass::TypedArray`] mit
//! [`TYPED_ARRAY_FIELDS`](crate::heap::TYPED_ARRAY_FIELDS) internen
//! Feldern. Ein Schlüssel, der eine kanonische numerische Zeichenkette ist
//! (CanonicalNumericIndexString, Abschnitt 7.1.23), nennt ein Element oder
//! nichts; die Prototypkette sieht er nie. Array-Indizes sind immer
//! kanonisch, andere Zeichenketten prüft [`numeric_key`]. Die Schleifen
//! über die Prototypkette in [`super`] halten bei einem TypedArray mit einem
//! solchen Schlüssel an, denn dort enden [[Get]], [[Set]] und
//! [[HasProperty]] nach Abschnitt 10.4.5.
//!
//! Gelesen wird immer mit [[LittleEndian]] *true*, wie ihn der Agent hat
//! (siehe [`crate::buffer`]). Eigene Elemente im Speicher gewöhnlicher
//! Objekte hat ein TypedArray nie: [[DefineOwnProperty]] fängt jeden
//! numerischen Schlüssel ab.

use super::{
    INLINE, object_of, ordinary_define_own_property, ordinary_get_own_property,
    ordinary_prevent_extensions,
};
use crate::agent::{Agent, Completion};
use crate::buffer::{self, ElementType, Numeric, numeric_value, raw_bytes_of_value};
use crate::conversion::{string_to_number, to_bigint, to_number};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Heap, Local, ObjectClass, OutOfMemory, Scope};
use crate::key::PropertyKey;
use crate::number;
use crate::value::{Kind, Ref, Value};

/// Das Feld mit [[ViewedArrayBuffer]].
const VIEWED_BUFFER: usize = 0;
/// Das Feld mit der Art der Elemente als Zahl ([`ElementType::index`]).
const NAME: usize = 1;
/// Das Feld mit [[ArrayLength]], *undefined* für ~auto~.
const ARRAY_LENGTH: usize = 2;
/// Das Feld mit [[ByteOffset]].
const BYTE_OFFSET: usize = 3;

/// Ein TypedArray With Buffer Witness Record (Abschnitt 10.4.5.9) mit den
/// Feldern des TypedArray, die die Operationen darauf lesen.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Record {
    /// Die Art der Elemente, für [[TypedArrayName]] und [[ContentType]].
    pub(crate) kind: ElementType,
    /// [[ArrayLength]], `None` für ~auto~.
    pub(crate) array_length: Option<u64>,
    /// [[ByteOffset]].
    pub(crate) byte_offset: u64,
    /// [[CachedBufferByteLength]], `None` für ~detached~.
    pub(crate) buffer_byte_length: Option<u64>,
}

impl Record {
    /// Die Größe eines Elements als `u64`.
    fn size(&self) -> u64 {
        // Höchstens 8.
        self.kind.size() as u64
    }

    /// IsTypedArrayOutOfBounds (Abschnitt 10.4.5.14).
    pub(crate) fn is_out_of_bounds(&self) -> bool {
        let Some(buffer_length) = self.buffer_byte_length else {
            return true;
        };
        let start = self.byte_offset;
        let end = match self.array_length {
            None => buffer_length,
            Some(length) => match length
                .checked_mul(self.size())
                .and_then(|bytes| start.checked_add(bytes))
            {
                Some(end) => end,
                None => return true,
            },
        };
        start > buffer_length || end > buffer_length
    }

    /// TypedArrayLength (Abschnitt 10.4.5.13). Die Aufrufer haben geprüft,
    /// dass das TypedArray in den Grenzen liegt; sonst ist das Ergebnis 0.
    pub(crate) fn length(&self) -> u64 {
        if let Some(length) = self.array_length {
            return length;
        }
        let buffer_length = self.buffer_byte_length.unwrap_or(0);
        buffer_length.saturating_sub(self.byte_offset) / self.size()
    }

    /// TypedArrayLength, `None`, wenn das TypedArray außerhalb der Grenzen
    /// liegt.
    pub(crate) fn length_in_bounds(&self) -> Option<u64> {
        (!self.is_out_of_bounds()).then(|| self.length())
    }

    /// TypedArrayByteLength (Abschnitt 10.4.5.12), mit denselben
    /// Voraussetzungen wie [`Record::length`].
    pub(crate) fn byte_length(&self) -> u64 {
        self.length().saturating_mul(self.size())
    }

    /// Ob ein Index ein Element nennt: IsValidIntegerIndex (Abschnitt
    /// 10.4.5.16) ohne den ersten Schritt, den [`Record::is_out_of_bounds`]
    /// für einen abgelösten ArrayBuffer schon abdeckt.
    fn element(&self, index: f64) -> Option<u64> {
        if index.fract() != 0.0 || index.is_sign_negative() || !index.is_finite() {
            return None;
        }
        let length = self.length_in_bounds()?;
        // Ganz, nicht negativ und kleiner als eine Länge unter 2⁵³.
        (index < length as f64).then_some(index as u64)
    }

    /// Der Index im Block des ArrayBuffer, an dem das Element `element`
    /// beginnt.
    fn byte_index(&self, element: u64) -> Option<usize> {
        let index = element
            .checked_mul(self.size())?
            .checked_add(self.byte_offset)?;
        usize::try_from(index).ok()
    }
}

/// Ein Feld mit einer Länge oder einem Abstand als ganze Zahl.
fn integer_field(value: Option<Value<'_>>) -> Option<u64> {
    let number = value?.as_number()?;
    // Die Felder halten nur ganze Zahlen unter 2⁵³.
    (number >= 0.0).then_some(number as u64)
}

/// Das TypedArray hinter `value`, `None` für andere Werte.
pub(crate) fn typed_array_ref<'h>(heap: &'h Heap, value: Value<'h>) -> Option<Ref<'h>> {
    match value.kind() {
        Kind::Object(object) if heap.object_class(object) == Some(ObjectClass::TypedArray) => {
            Some(object)
        }
        _ => None,
    }
}

/// Die Art der Elemente des TypedArray `object`.
fn kind_of(heap: &Heap, object: Ref<'_>) -> Option<ElementType> {
    let index = heap.typed_array_field(object, NAME)?.as_int32()?;
    ElementType::from_index(u8::try_from(index).ok()?)
}

/// [[ViewedArrayBuffer]] des TypedArray `object`, `None`, bevor es einen
/// hat.
fn buffer_of<'h>(heap: &'h Heap, object: Ref<'h>) -> Option<Ref<'h>> {
    match heap.typed_array_field(object, VIEWED_BUFFER)?.kind() {
        Kind::Object(buffer) => Some(buffer),
        _ => None,
    }
}

/// MakeTypedArrayWithBufferWitnessRecord (Abschnitt 10.4.5.10) für das
/// TypedArray `object`. Die Länge eines SharedArrayBuffer wird immer mit
/// ~seq-cst~ gelesen; das ist auch für ~unordered~ erlaubt.
pub(crate) fn record(heap: &Heap, object: Ref<'_>) -> Option<Record> {
    let kind = kind_of(heap, object)?;
    let length = heap.typed_array_field(object, ARRAY_LENGTH)?;
    let array_length = if length.is_undefined() {
        None
    } else {
        Some(integer_field(Some(length))?)
    };
    let byte_offset = integer_field(heap.typed_array_field(object, BYTE_OFFSET))?;
    let buffer_byte_length = buffer_of(heap, object)
        .and_then(|buffer| buffer::block_length(heap, buffer))
        .and_then(|length| u64::try_from(length).ok());
    Some(Record {
        kind,
        array_length,
        byte_offset,
        buffer_byte_length,
    })
}

/// [`record`] für einen Wert, `None`, wenn er kein TypedArray ist.
pub(crate) fn record_of(agent: &Agent, value: Local<'_>) -> Option<Record> {
    let heap = agent.heap();
    record(heap, typed_array_ref(heap, heap.get(value))?)
}

/// IsTypedArrayFixedLength (Abschnitt 10.4.5.15). Ein SharedArrayBuffer
/// wird nie kürzer, also hat ein TypedArray mit fester Länge darauf auch
/// dann eine feste Länge, wenn er wächst.
pub(crate) fn is_fixed_length(heap: &Heap, object: Ref<'_>) -> bool {
    let fixed_array = heap
        .typed_array_field(object, ARRAY_LENGTH)
        .is_some_and(|length| !length.is_undefined());
    fixed_array
        && buffer_of(heap, object).is_some_and(|buffer| {
            buffer::is_fixed_length(heap, buffer) || buffer::is_shared(heap, buffer)
        })
}

/// IsValidIntegerIndex (Abschnitt 10.4.5.16).
pub(crate) fn is_valid_integer_index(heap: &Heap, object: Ref<'_>, index: f64) -> bool {
    record(heap, object).is_some_and(|record| record.element(index).is_some())
}

/// Das Element `index` des TypedArray `object` ohne es in den Heap zu
/// legen; `None`, wenn der Index keines nennt.
pub(crate) fn read_element(heap: &Heap, object: Ref<'_>, index: f64) -> Option<Numeric> {
    let record = record(heap, object)?;
    let byte_index = record.byte_index(record.element(index)?)?;
    let buffer = buffer_of(heap, object)?;
    let mut raw = [0u8; 8];
    let bytes = raw.get_mut(..record.kind.size())?;
    buffer::read_block(heap, buffer, byte_index, bytes).then_some(())?;
    Some(buffer::raw_bytes_to_numeric(record.kind, &raw, true))
}

/// Schreibt `value`, schon eine Number oder ein BigInt passend zur Art, in
/// das Element `index`, wenn es eines nennt (Schritte 2 bis 3 von
/// TypedArraySetElement).
pub(crate) fn write_element(heap: &Heap, object: Ref<'_>, index: f64, value: Value<'_>) {
    let Some(record) = record(heap, object) else {
        return;
    };
    let Some(byte_index) = record
        .element(index)
        .and_then(|element| record.byte_index(element))
    else {
        return;
    };
    let raw = raw_bytes_of_value(heap, value, record.kind, true);
    if let (Some(buffer), Some(bytes)) = (buffer_of(heap, object), raw.get(..record.kind.size())) {
        buffer::write_block(heap, buffer, byte_index, bytes);
    }
}

/// TypedArrayCreate (Abschnitt 10.4.5.11) mit den ersten Schritten von
/// AllocateTypedArray (Abschnitt 23.2.5.1.1): ein TypedArray der Art `kind`
/// mit `prototype`, noch ohne ArrayBuffer, Länge und Abstand 0.
pub(crate) fn typed_array_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
    kind: ElementType,
) -> Completion<Local<'s>> {
    let shape = agent.empty_shape(scope);
    let object = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::TypedArray,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    if let Some(reference) = object_of(heap, object) {
        heap.set_typed_array_field(reference, VIEWED_BUFFER, Value::UNDEFINED);
        heap.set_typed_array_field(reference, NAME, Value::int32(i32::from(kind.index())));
        heap.set_typed_array_field(reference, ARRAY_LENGTH, Value::int32(0));
        heap.set_typed_array_field(reference, BYTE_OFFSET, Value::int32(0));
    }
    Ok(object)
}

/// Setzt [[ViewedArrayBuffer]], [[ArrayLength]] (`None` für ~auto~) und
/// [[ByteOffset]] des TypedArray `typed_array`; [[ByteLength]] folgt
/// daraus.
pub(crate) fn initialize(
    agent: &Agent,
    typed_array: Local<'_>,
    buffer: Local<'_>,
    array_length: Option<u64>,
    byte_offset: u64,
) {
    let heap = agent.heap();
    let Some(object) = typed_array_ref(heap, heap.get(typed_array)) else {
        return;
    };
    // Längen und Abstände liegen unter 2⁵³ und sind als Number exakt.
    let length = array_length.map_or(Value::UNDEFINED, |length| Value::number(length as f64));
    heap.set_typed_array_field(object, VIEWED_BUFFER, heap.get(buffer));
    heap.set_typed_array_field(object, ARRAY_LENGTH, length);
    heap.set_typed_array_field(object, BYTE_OFFSET, Value::number(byte_offset as f64));
}

/// [[ViewedArrayBuffer]] des TypedArray `typed_array`, *undefined* für
/// einen anderen Wert.
pub(crate) fn viewed_buffer<'s>(
    agent: &Agent,
    scope: &'s Scope,
    typed_array: Local<'_>,
) -> Local<'s> {
    let heap = agent.heap();
    let buffer = typed_array_ref(heap, heap.get(typed_array))
        .and_then(|object| heap.typed_array_field(object, VIEWED_BUFFER))
        .unwrap_or(Value::UNDEFINED);
    heap.local(scope, buffer)
}

/// TypedArrayGetElement (Abschnitt 10.4.5.17).
pub(crate) fn get_element<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    typed_array: Local<'_>,
    index: f64,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let element = typed_array_ref(heap, heap.get(typed_array))
        .and_then(|object| read_element(heap, object, index));
    match element {
        Some(value) => numeric_value(agent, scope, value),
        None => Ok(heap.local(scope, Value::UNDEFINED)),
    }
}

/// TypedArraySetElement (Abschnitt 10.4.5.18).
pub(crate) fn set_element(
    agent: &mut Agent,
    scope: &Scope,
    typed_array: Local<'_>,
    index: f64,
    value: Local<'_>,
) -> Completion<()> {
    let heap = agent.heap();
    let Some(kind) =
        typed_array_ref(heap, heap.get(typed_array)).and_then(|object| kind_of(heap, object))
    else {
        return Ok(());
    };
    let numeric = if kind.is_bigint() {
        to_bigint(agent, scope, value)?
    } else {
        let number = to_number(agent, scope, value)?;
        agent.heap().local(scope, Value::number(number))
    };
    let heap = agent.heap();
    if let Some(object) = typed_array_ref(heap, heap.get(typed_array)) {
        write_element(heap, object, index, heap.get(numeric));
    }
    Ok(())
}

// Schlüssel

/// CanonicalNumericIndexString (Abschnitt 7.1.23) für die Codeeinheiten
/// einer Zeichenkette. Kanonisch ist nur, was mit einer Ziffer, `-`, `I`
/// (*"Infinity"*) oder `N` (*"NaN"*) beginnt; das prüft der erste Schritt,
/// bevor die Zahl gelesen und wieder geschrieben wird.
pub(crate) fn canonical_numeric_index(units: &[u16]) -> Option<f64> {
    let &first = units.first()?;
    let starts_numeric = matches!(first, 0x30..=0x39 | 0x2D | 0x49 | 0x4E);
    if !starts_numeric {
        return None;
    }
    if units == [0x2D, 0x30] {
        return Some(-0.0);
    }
    let number = string_to_number(units);
    number::to_string(number)
        .encode_utf16()
        .eq(units.iter().copied())
        .then_some(number)
}

/// Der numerische Index, den ein Schlüssel nach CanonicalNumericIndexString
/// nennt, `None` für einen Schlüssel, der kein Element meint.
pub(crate) fn numeric_key(heap: &Heap, key: PropertyKey<'_>) -> Option<f64> {
    match key {
        PropertyKey::Index(index) => Some(f64::from(index)),
        PropertyKey::String(name) => {
            let Kind::String(string) = heap.get(name).kind() else {
                return None;
            };
            let first = heap.string_unit(string, 0)?;
            if !matches!(first, 0x30..=0x39 | 0x2D | 0x49 | 0x4E) {
                return None;
            }
            canonical_numeric_index(&heap.string_units(string))
        }
        PropertyKey::Symbol(_) => None,
    }
}

// Die internen Methoden

/// [[PreventExtensions]] (Abschnitt 10.4.5.1).
pub(super) fn prevent_extensions(heap: &Heap, object: Local<'_>) -> bool {
    let fixed = object_of(heap, object).is_some_and(|reference| is_fixed_length(heap, reference));
    fixed && ordinary_prevent_extensions(heap, object)
}

/// [[GetOwnProperty]] (Abschnitt 10.4.5.2).
pub(super) fn get_own_property<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Option<PropertyDescriptor<'s>>> {
    let heap = agent.heap();
    let Some(index) = numeric_key(heap, key) else {
        return Ok(ordinary_get_own_property(heap, scope, object, key));
    };
    let valid = object_of(heap, object)
        .is_some_and(|reference| is_valid_integer_index(heap, reference, index));
    if !valid {
        return Ok(None);
    }
    let value = get_element(agent, scope, object, index)?;
    Ok(Some(PropertyDescriptor {
        value: Some(value),
        writable: Some(true),
        enumerable: Some(true),
        configurable: Some(true),
        ..PropertyDescriptor::default()
    }))
}

/// [[DefineOwnProperty]] (Abschnitt 10.4.5.4).
pub(super) fn define_own_property(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<bool> {
    let heap = agent.heap();
    let Some(index) = numeric_key(heap, key) else {
        return ordinary_define_own_property(agent, scope, object, key, descriptor);
    };
    let valid = object_of(heap, object)
        .is_some_and(|reference| is_valid_integer_index(heap, reference, index));
    if !valid
        || descriptor.configurable == Some(false)
        || descriptor.enumerable == Some(false)
        || descriptor.is_accessor()
        || descriptor.writable == Some(false)
    {
        return Ok(false);
    }
    if let Some(value) = descriptor.value {
        set_element(agent, scope, object, index, value)?;
    }
    Ok(true)
}

/// [[Delete]] (Abschnitt 10.4.5.7) für einen numerischen Schlüssel; `None`
/// für einen anderen, dann gilt OrdinaryDelete.
pub(super) fn delete(heap: &Heap, object: Local<'_>, key: PropertyKey<'_>) -> Option<bool> {
    let index = numeric_key(heap, key)?;
    let valid = object_of(heap, object)
        .is_some_and(|reference| is_valid_integer_index(heap, reference, index));
    Some(!valid)
}

/// Die Indizes, mit denen [[OwnPropertyKeys]] (Abschnitt 10.4.5.8)
/// beginnt: keine, wenn das TypedArray außerhalb der Grenzen liegt. Die
/// Liste muss so viele Bytes, wie sie belegt, auch im Heap fassen können;
/// sonst ist der Speicher erschöpft. Ein Aufrufer, der sie als Werte
/// braucht, legt ohnehin je Index eine Zeichenkette im Heap an.
pub(super) fn index_keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    keys: &mut Vec<PropertyKey<'s>>,
) -> Completion<()> {
    let heap = agent.heap();
    let count = object_of(heap, object)
        .and_then(|reference| record(heap, reference))
        .and_then(|record| record.length_in_bounds())
        .unwrap_or(0);
    let key_size = u64::try_from(size_of::<PropertyKey<'_>>()).unwrap_or(u64::MAX);
    let fits = count
        .checked_mul(key_size)
        .is_some_and(|bytes| heap.bytes_fit(bytes));
    let Some(length) = usize::try_from(count).ok().filter(|_| fits) else {
        return Err(OutOfMemory.into());
    };
    keys.reserve(length);
    for index in 0..count {
        match u32::try_from(index) {
            Ok(index) if index < u32::MAX => keys.push(PropertyKey::Index(index)),
            _ => {
                // Ganze Zahlen unter 2⁵³ sind als Number exakt.
                let units: Vec<u16> = number::to_string(index as f64).encode_utf16().collect();
                keys.push(agent.key_from_units(scope, &units)?);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
