//! Objekte und ihre internen Methoden (Abschnitte 6.1.7.2 und 10.1).
//!
//! Ein Objekt ist eine Zelle im Heap mit Gestalt, Prototyp, Elementen und
//! Plätzen für benannte Eigenschaften (siehe `heap`). Die Gestalt nennt die
//! benannten Eigenschaften, Array-Indizes liegen als Elemente daneben.
//!
//! Jede interne Methode ist eine Funktion, die nach der Art des Objekts
//! verzweigt. Bisher gibt es gewöhnliche Objekte, Funktionsobjekte, die
//! dieselben internen Methoden haben und dazu [[Call]] (`function`), und
//! Arrays, die nur [[DefineOwnProperty]] anders umsetzen (`array`). Alle
//! Arten, die
//! [[GetOwnProperty]], [[GetPrototypeOf]], [[Get]], [[Set]] und
//! [[HasProperty]] gewöhnlich umsetzen, durchlaufen die Prototypkette in
//! einer Schleife statt rekursiv (RS-8). Kommt eine Art mit eigenen
//! Fassungen dazu, etwa Proxy, verlässt die Schleife die Kette dort.

#![forbid(unsafe_code)]

mod array;
pub(crate) mod shape;
mod storage;

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::compare::same_value;
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Heap, Local, ObjectClass, OutOfMemory, Scope};
use crate::key::PropertyKey;
use crate::operations::{call, create_data_property};
use crate::value::{Kind, Ref, Value};
use storage::{ACCESSOR, CONFIGURABLE, ENUMERABLE, WRITABLE, object_of};

pub use array::array_create;

/// Eingebettete Plätze eines gewöhnlichen Objekts und eines
/// Funktionsobjekts.
pub(crate) const INLINE: usize = 4;

/// Ein Schlüssel, wie er im Heap steht: Index oder Atom beziehungsweise
/// Symbol.
#[derive(Clone, Copy)]
enum Key<'h> {
    Index(u32),
    Named(Value<'h>),
}

fn key_in<'h>(heap: &'h Heap, key: PropertyKey<'_>) -> Key<'h> {
    match key {
        PropertyKey::Index(index) => Key::Index(index),
        PropertyKey::String(name) | PropertyKey::Symbol(name) => Key::Named(heap.get(name)),
    }
}

/// Eine eigene Eigenschaft, wie sie im Heap steht.
#[derive(Clone, Copy)]
enum Own<'h> {
    Data {
        value: Value<'h>,
        attributes: u32,
    },
    Accessor {
        getter: Value<'h>,
        setter: Value<'h>,
        attributes: u32,
    },
}

fn find_own<'h>(heap: &'h Heap, object: Ref<'h>, key: Key<'_>) -> Option<Own<'h>> {
    let (stored, attributes) = match key {
        Key::Index(index) => storage::element(heap, object, index)?,
        Key::Named(name) => {
            let (position, attributes) = shape::find(heap, heap.object_shape(object)?, name)?;
            (storage::slot(heap, object, position)?, attributes)
        }
    };
    Some(if attributes & ACCESSOR != 0 {
        let (getter, setter) = storage::pair(heap, stored);
        Own::Accessor {
            getter,
            setter,
            attributes,
        }
    } else {
        Own::Data {
            value: stored,
            attributes,
        }
    })
}

/// Die Art des Objekts in `local`. Ein anderer Wert ist ein Fehler im
/// Aufrufer; nach Abschnitt 6.1.7.2 wirft eine fehlende interne Methode
/// einen TypeError.
fn class(agent: &mut Agent, local: Local<'_>) -> Completion<ObjectClass> {
    let heap = agent.heap();
    match object_of(heap, local).and_then(|object| heap.object_class(object)) {
        Some(class) => Ok(class),
        None => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

/// OrdinaryObjectCreate (Abschnitt 10.1.12) mit MakeBasicObject (Abschnitt
/// 7.3.1): ein erweiterbares gewöhnliches Objekt ohne Eigenschaften.
pub fn ordinary_object_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Option<Local<'_>>,
) -> Result<Local<'s>, OutOfMemory> {
    let shape = agent.empty_shape(scope);
    agent
        .heap_mut()
        .alloc_object(scope, ObjectClass::Ordinary, INLINE, shape, prototype)
}

/// [[GetPrototypeOf]] (Abschnitt 10.1.1): *null* ist `None`.
pub fn get_prototype_of<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
) -> Completion<Option<Local<'s>>> {
    match class(agent, object)? {
        ObjectClass::Ordinary | ObjectClass::Array | ObjectClass::Function => {
            let heap = agent.heap();
            let prototype =
                object_of(heap, object).and_then(|object| heap.object_prototype(object));
            Ok(prototype.map(|prototype| heap.local(scope, Value::object(prototype))))
        }
    }
}

/// [[SetPrototypeOf]] (Abschnitt 10.1.2, OrdinarySetPrototypeOf 10.1.2.1).
pub fn set_prototype_of(
    agent: &mut Agent,
    object: Local<'_>,
    prototype: Option<Local<'_>>,
) -> Completion<bool> {
    match class(agent, object)? {
        ObjectClass::Ordinary | ObjectClass::Array | ObjectClass::Function => {}
    }
    if let Some(prototype) = prototype {
        // Der Prototyp muss ein Objekt sein.
        class(agent, prototype)?;
    }
    let heap = agent.heap();
    let (Some(object), prototype) = (
        object_of(heap, object),
        prototype.and_then(|prototype| object_of(heap, prototype)),
    ) else {
        return Ok(false);
    };
    if heap.object_prototype(object) == prototype {
        return Ok(true);
    }
    if !heap.object_extensible(object) {
        return Ok(false);
    }
    let mut cursor = prototype;
    while let Some(current) = cursor {
        if current == object {
            return Ok(false);
        }
        // Alle Arten nutzen bisher das gewöhnliche [[GetPrototypeOf]].
        cursor = heap.object_prototype(current);
    }
    heap.set_object_prototype(object, prototype);
    Ok(true)
}

/// [[IsExtensible]] (Abschnitt 10.1.3).
pub fn is_extensible(agent: &mut Agent, object: Local<'_>) -> Completion<bool> {
    match class(agent, object)? {
        ObjectClass::Ordinary | ObjectClass::Array | ObjectClass::Function => {
            let heap = agent.heap();
            Ok(object_of(heap, object).is_some_and(|object| heap.object_extensible(object)))
        }
    }
}

/// [[PreventExtensions]] (Abschnitt 10.1.4).
pub fn prevent_extensions(agent: &mut Agent, object: Local<'_>) -> Completion<bool> {
    match class(agent, object)? {
        ObjectClass::Ordinary | ObjectClass::Array | ObjectClass::Function => {
            let heap = agent.heap();
            if let Some(object) = object_of(heap, object) {
                heap.prevent_object_extensions(object);
            }
            Ok(true)
        }
    }
}

/// [[GetOwnProperty]] (Abschnitt 10.1.5, OrdinaryGetOwnProperty 10.1.5.1).
pub fn get_own_property<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Option<PropertyDescriptor<'s>>> {
    match class(agent, object)? {
        ObjectClass::Ordinary | ObjectClass::Array | ObjectClass::Function => {
            Ok(ordinary_get_own_property(agent.heap(), scope, object, key))
        }
    }
}

fn ordinary_get_own_property<'s>(
    heap: &Heap,
    scope: &'s Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Option<PropertyDescriptor<'s>> {
    let own = find_own(heap, object_of(heap, object)?, key_in(heap, key))?;
    let flag = |attributes: u32, flag: u32| Some(attributes & flag != 0);
    Some(match own {
        Own::Data { value, attributes } => PropertyDescriptor {
            value: Some(heap.local(scope, value)),
            writable: flag(attributes, WRITABLE),
            enumerable: flag(attributes, ENUMERABLE),
            configurable: flag(attributes, CONFIGURABLE),
            ..PropertyDescriptor::default()
        },
        Own::Accessor {
            getter,
            setter,
            attributes,
        } => PropertyDescriptor {
            get: Some(heap.local(scope, getter)),
            set: Some(heap.local(scope, setter)),
            enumerable: flag(attributes, ENUMERABLE),
            configurable: flag(attributes, CONFIGURABLE),
            ..PropertyDescriptor::default()
        },
    })
}

/// [[DefineOwnProperty]] (Abschnitt 10.1.6).
pub fn define_own_property(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<bool> {
    match class(agent, object)? {
        ObjectClass::Ordinary | ObjectClass::Function => {
            ordinary_define_own_property(agent, scope, object, key, descriptor)
        }
        ObjectClass::Array => array::define_own_property(agent, scope, object, key, descriptor),
    }
}

/// OrdinaryDefineOwnProperty (Abschnitt 10.1.6.1).
fn ordinary_define_own_property(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    descriptor: &PropertyDescriptor<'_>,
) -> Completion<bool> {
    let current = get_own_property(agent, scope, object, key)?;
    let extensible = is_extensible(agent, object)?;
    validate_and_apply_property_descriptor(
        agent,
        scope,
        Some(object),
        key,
        extensible,
        descriptor,
        current.as_ref(),
    )
    .map_err(Abrupt::from)
}

/// IsCompatiblePropertyDescriptor (Abschnitt 10.1.6.2).
pub fn is_compatible_property_descriptor(
    agent: &mut Agent,
    scope: &Scope,
    extensible: bool,
    descriptor: &PropertyDescriptor<'_>,
    current: Option<&PropertyDescriptor<'_>>,
) -> Result<bool, OutOfMemory> {
    // Ohne Objekt ist der Schlüssel gleichgültig.
    validate_and_apply_property_descriptor(
        agent,
        scope,
        None,
        PropertyKey::Index(0),
        extensible,
        descriptor,
        current,
    )
}

/// Eine vollständige Eigenschaft, wie sie geschrieben wird.
enum Property<'s> {
    Data {
        value: Local<'s>,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    },
    Accessor {
        getter: Local<'s>,
        setter: Local<'s>,
        enumerable: bool,
        configurable: bool,
    },
}

/// ValidateAndApplyPropertyDescriptor (Abschnitt 10.1.6.3). `current` ist
/// vollständig, wenn vorhanden.
fn validate_and_apply_property_descriptor<'a>(
    agent: &mut Agent,
    scope: &'a Scope,
    object: Option<Local<'_>>,
    key: PropertyKey<'_>,
    extensible: bool,
    descriptor: &PropertyDescriptor<'a>,
    current: Option<&PropertyDescriptor<'a>>,
) -> Result<bool, OutOfMemory> {
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let Some(current) = current else {
        if !extensible {
            return Ok(false);
        }
        let Some(object) = object else {
            return Ok(true);
        };
        let enumerable = descriptor.enumerable.unwrap_or(false);
        let configurable = descriptor.configurable.unwrap_or(false);
        let property = if descriptor.is_accessor() {
            Property::Accessor {
                getter: descriptor.get.unwrap_or(undefined),
                setter: descriptor.set.unwrap_or(undefined),
                enumerable,
                configurable,
            }
        } else {
            Property::Data {
                value: descriptor.value.unwrap_or(undefined),
                writable: descriptor.writable.unwrap_or(false),
                enumerable,
                configurable,
            }
        };
        write_property(agent, scope, object, key, &property)?;
        return Ok(true);
    };
    if descriptor.is_empty() {
        return Ok(true);
    }
    let heap = agent.heap();
    let same = |a: Local<'_>, b: Option<Local<'_>>| {
        same_value(heap, heap.get(a), heap.get(b.unwrap_or(undefined)))
    };
    if current.configurable == Some(false) {
        if descriptor.configurable == Some(true) {
            return Ok(false);
        }
        if descriptor
            .enumerable
            .is_some_and(|enumerable| Some(enumerable) != current.enumerable)
        {
            return Ok(false);
        }
        if !descriptor.is_generic() && descriptor.is_accessor() != current.is_accessor() {
            return Ok(false);
        }
        if current.is_accessor() {
            if descriptor
                .get
                .is_some_and(|getter| !same(getter, current.get))
            {
                return Ok(false);
            }
            if descriptor
                .set
                .is_some_and(|setter| !same(setter, current.set))
            {
                return Ok(false);
            }
        } else if current.writable == Some(false) {
            if descriptor.writable == Some(true) {
                return Ok(false);
            }
            if let Some(value) = descriptor.value {
                return Ok(same(value, current.value));
            }
        }
    }
    let Some(object) = object else {
        return Ok(true);
    };
    let enumerable = descriptor
        .enumerable
        .or(current.enumerable)
        .unwrap_or(false);
    let configurable = descriptor
        .configurable
        .or(current.configurable)
        .unwrap_or(false);
    let property = if current.is_data() && descriptor.is_accessor() {
        Property::Accessor {
            getter: descriptor.get.unwrap_or(undefined),
            setter: descriptor.set.unwrap_or(undefined),
            enumerable,
            configurable,
        }
    } else if current.is_accessor() && descriptor.is_data() {
        Property::Data {
            value: descriptor.value.unwrap_or(undefined),
            writable: descriptor.writable.unwrap_or(false),
            enumerable,
            configurable,
        }
    } else if current.is_accessor() {
        Property::Accessor {
            getter: descriptor.get.or(current.get).unwrap_or(undefined),
            setter: descriptor.set.or(current.set).unwrap_or(undefined),
            enumerable,
            configurable,
        }
    } else {
        Property::Data {
            value: descriptor.value.or(current.value).unwrap_or(undefined),
            writable: descriptor.writable.or(current.writable).unwrap_or(false),
            enumerable,
            configurable,
        }
    };
    write_property(agent, scope, object, key, &property)?;
    Ok(true)
}

/// Schreibt eine Eigenschaft: legt sie an oder ersetzt sie mit allen
/// Attributen.
fn write_property(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    property: &Property<'_>,
) -> Result<(), OutOfMemory> {
    let flags = |enumerable: bool, configurable: bool| {
        (if enumerable { ENUMERABLE } else { 0 }) | (if configurable { CONFIGURABLE } else { 0 })
    };
    let heap = agent.heap_mut();
    let (stored, attributes) = match *property {
        Property::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => (
            value,
            flags(enumerable, configurable) | if writable { WRITABLE } else { 0 },
        ),
        Property::Accessor {
            getter,
            setter,
            enumerable,
            configurable,
        } => (
            storage::alloc_pair(heap, scope, getter, setter)?,
            flags(enumerable, configurable) | ACCESSOR,
        ),
    };
    let name = match key {
        PropertyKey::Index(index) => {
            return storage::put_element(heap, scope, object, index, stored, attributes);
        }
        PropertyKey::String(name) | PropertyKey::Symbol(name) => name,
    };
    let (shape, found, count) = {
        let object_ref = object_of(heap, object).ok_or(OutOfMemory)?;
        let shape_ref = heap.object_shape(object_ref).ok_or(OutOfMemory)?;
        let found = shape::find(heap, shape_ref, heap.get(name));
        let count = shape::count(heap, shape_ref);
        (heap.local(scope, Value::internal(shape_ref)), found, count)
    };
    let (shape, position) = match found {
        Some((position, old)) if old == attributes => (None, position),
        Some((position, _)) => (
            Some(shape::rebuild(
                heap,
                scope,
                shape,
                position,
                Some(attributes),
            )?),
            position,
        ),
        None => {
            let next = shape::transition(heap, scope, shape, name, attributes)?;
            storage::reserve(
                heap,
                scope,
                object,
                count.checked_add(1).ok_or(OutOfMemory)?,
            )?;
            (Some(next), count)
        }
    };
    let object = object_of(heap, object).ok_or(OutOfMemory)?;
    storage::set_slot(heap, object, position, heap.get(stored));
    if let Some(shape) = shape.and_then(|shape| shape::internal(heap.get(shape))) {
        heap.set_object_shape(object, shape);
    }
    Ok(())
}

/// [[HasProperty]] (Abschnitt 10.1.7, OrdinaryHasProperty 10.1.7.1).
pub fn has_property(
    agent: &mut Agent,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<bool> {
    class(agent, object)?;
    let heap = agent.heap();
    let key = key_in(heap, key);
    let Some(mut current) = object_of(heap, object) else {
        return Ok(false);
    };
    loop {
        if find_own(heap, current, key).is_some() {
            return Ok(true);
        }
        match heap.object_prototype(current) {
            Some(parent) => current = parent,
            None => return Ok(false),
        }
    }
}

/// [[Get]] (Abschnitt 10.1.8, OrdinaryGet 10.1.8.1).
pub fn get<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    receiver: Local<'_>,
) -> Completion<Local<'s>> {
    class(agent, object)?;
    let getter = {
        let heap = agent.heap();
        let key = key_in(heap, key);
        let Some(mut current) = object_of(heap, object) else {
            return Ok(heap.local(scope, Value::UNDEFINED));
        };
        loop {
            match find_own(heap, current, key) {
                Some(Own::Data { value, .. }) => return Ok(heap.local(scope, value)),
                Some(Own::Accessor { getter, .. }) => break heap.local(scope, getter),
                None => match heap.object_prototype(current) {
                    Some(parent) => current = parent,
                    None => return Ok(heap.local(scope, Value::UNDEFINED)),
                },
            }
        }
    };
    if agent.heap().get(getter).is_undefined() {
        return Ok(getter);
    }
    call(agent, scope, getter, receiver, &[])
}

/// Was OrdinarySet auf der Prototypkette findet.
enum Holder<'s> {
    /// Keine Eigenschaft oder eine beschreibbare Dateneigenschaft.
    Writable,
    ReadOnly,
    Setter(Local<'s>),
}

/// [[Set]] (Abschnitt 10.1.9, OrdinarySet 10.1.9.1 mit
/// OrdinarySetWithOwnDescriptor 10.1.9.2).
pub fn set(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
    value: Local<'_>,
    receiver: Local<'_>,
) -> Completion<bool> {
    class(agent, object)?;
    let holder = {
        let heap = agent.heap();
        let key = key_in(heap, key);
        let Some(mut current) = object_of(heap, object) else {
            return Ok(false);
        };
        loop {
            match find_own(heap, current, key) {
                Some(Own::Data { attributes, .. }) if attributes & WRITABLE == 0 => {
                    break Holder::ReadOnly;
                }
                Some(Own::Data { .. }) => break Holder::Writable,
                Some(Own::Accessor { setter, .. }) => {
                    break Holder::Setter(heap.local(scope, setter));
                }
                None => match heap.object_prototype(current) {
                    Some(parent) => current = parent,
                    // Ohne Eigenschaft gilt eine beschreibbare
                    // Dateneigenschaft als gefunden.
                    None => break Holder::Writable,
                },
            }
        }
    };
    match holder {
        Holder::ReadOnly => Ok(false),
        Holder::Setter(setter) => {
            if agent.heap().get(setter).is_undefined() {
                return Ok(false);
            }
            call(agent, scope, setter, receiver, &[value])?;
            Ok(true)
        }
        Holder::Writable => {
            if object_of(agent.heap(), receiver).is_none() {
                return Ok(false);
            }
            match get_own_property(agent, scope, receiver, key)? {
                None => create_data_property(agent, scope, receiver, key, value),
                Some(existing) if existing.is_accessor() => Ok(false),
                Some(existing) if existing.writable == Some(false) => Ok(false),
                Some(_) => {
                    let descriptor = PropertyDescriptor {
                        value: Some(value),
                        ..PropertyDescriptor::default()
                    };
                    define_own_property(agent, scope, receiver, key, &descriptor)
                }
            }
        }
    }
}

/// [[Delete]] (Abschnitt 10.1.10, OrdinaryDelete 10.1.10.1).
pub fn delete(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<bool> {
    let Some(descriptor) = get_own_property(agent, scope, object, key)? else {
        return Ok(true);
    };
    if descriptor.configurable != Some(true) {
        return Ok(false);
    }
    remove_property(agent.heap_mut(), scope, object, key)?;
    Ok(true)
}

fn remove_property(
    heap: &mut Heap,
    scope: &Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Result<(), OutOfMemory> {
    let name = match key {
        PropertyKey::Index(index) => {
            if let Some(object) = object_of(heap, object) {
                storage::remove_element(heap, object, index);
            }
            return Ok(());
        }
        PropertyKey::String(name) | PropertyKey::Symbol(name) => name,
    };
    let (shape, position, count) = {
        let object_ref = object_of(heap, object).ok_or(OutOfMemory)?;
        let shape_ref = heap.object_shape(object_ref).ok_or(OutOfMemory)?;
        let Some((position, _)) = shape::find(heap, shape_ref, heap.get(name)) else {
            return Ok(());
        };
        let count = shape::count(heap, shape_ref);
        (
            heap.local(scope, Value::internal(shape_ref)),
            position,
            count,
        )
    };
    let shape = shape::rebuild(heap, scope, shape, position, None)?;
    let object = object_of(heap, object).ok_or(OutOfMemory)?;
    storage::remove_slot(heap, object, position, count);
    if let Some(shape) = shape::internal(heap.get(shape)) {
        heap.set_object_shape(object, shape);
    }
    Ok(())
}

/// [[OwnPropertyKeys]] (Abschnitt 10.1.11, OrdinaryOwnPropertyKeys
/// 10.1.11.1): Array-Indizes aufsteigend, dann Zeichenketten und danach
/// Symbole, jeweils in der Reihenfolge, in der sie entstanden sind.
pub fn own_property_keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
) -> Completion<Vec<PropertyKey<'s>>> {
    match class(agent, object)? {
        ObjectClass::Ordinary | ObjectClass::Array | ObjectClass::Function => {}
    }
    let heap = agent.heap();
    let Some(object) = object_of(heap, object) else {
        return Ok(Vec::new());
    };
    let mut keys: Vec<PropertyKey<'s>> = storage::indices(heap, object)
        .into_iter()
        .map(PropertyKey::Index)
        .collect();
    let entries = heap
        .object_shape(object)
        .map(|shape| shape::entries(heap, shape))
        .unwrap_or_default();
    for (key, _) in &entries {
        if let Kind::String(_) = key.kind() {
            keys.push(PropertyKey::String(heap.local(scope, *key)));
        }
    }
    for (key, _) in &entries {
        if let Kind::Symbol(_) = key.kind() {
            keys.push(PropertyKey::Symbol(heap.local(scope, *key)));
        }
    }
    Ok(keys)
}

#[cfg(test)]
mod tests;
