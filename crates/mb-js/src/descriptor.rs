//! Eigenschaftsbeschreibungen (Abschnitt 6.2.6).

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::conversion::to_boolean;
use crate::heap::{Local, Scope};
use crate::object::ordinary_object_create;
use crate::operations::{create_data_property_or_throw, get, has_property, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Ein Property Descriptor: jedes Feld ist vorhanden oder fehlt. Werte,
/// Getter und Setter stehen in Handles, denn die Operationen, die eine
/// Beschreibung lesen, fordern Speicher an.
#[derive(Clone, Copy, Debug, Default)]
pub struct PropertyDescriptor<'s> {
    pub value: Option<Local<'s>>,
    pub writable: Option<bool>,
    /// [[Get]]: eine Funktion oder *undefined*.
    pub get: Option<Local<'s>>,
    /// [[Set]]: eine Funktion oder *undefined*.
    pub set: Option<Local<'s>>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}

impl<'s> PropertyDescriptor<'s> {
    /// Eine vollständige Beschreibung einer Dateneigenschaft.
    pub fn data(value: Local<'s>, writable: bool, enumerable: bool, configurable: bool) -> Self {
        PropertyDescriptor {
            value: Some(value),
            writable: Some(writable),
            enumerable: Some(enumerable),
            configurable: Some(configurable),
            ..PropertyDescriptor::default()
        }
    }

    /// Eine vollständige Beschreibung einer Zugriffseigenschaft.
    pub fn accessor(get: Local<'s>, set: Local<'s>, enumerable: bool, configurable: bool) -> Self {
        PropertyDescriptor {
            get: Some(get),
            set: Some(set),
            enumerable: Some(enumerable),
            configurable: Some(configurable),
            ..PropertyDescriptor::default()
        }
    }

    /// IsAccessorDescriptor (Abschnitt 6.2.6.1).
    pub fn is_accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    /// IsDataDescriptor (Abschnitt 6.2.6.2).
    pub fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    /// IsGenericDescriptor (Abschnitt 6.2.6.3).
    pub fn is_generic(&self) -> bool {
        !self.is_accessor() && !self.is_data()
    }

    /// Ob die Beschreibung kein Feld hat.
    pub fn is_empty(&self) -> bool {
        self.is_generic() && self.enumerable.is_none() && self.configurable.is_none()
    }
}

/// Die Felder einer Beschreibung in der Reihenfolge von ToPropertyDescriptor.
#[derive(Clone, Copy)]
enum Field {
    Enumerable,
    Configurable,
    Value,
    Writable,
    Get,
    Set,
}

impl Field {
    const ALL: [Field; 6] = [
        Field::Enumerable,
        Field::Configurable,
        Field::Value,
        Field::Writable,
        Field::Get,
        Field::Set,
    ];

    fn name(self) -> &'static str {
        match self {
            Field::Enumerable => "enumerable",
            Field::Configurable => "configurable",
            Field::Value => "value",
            Field::Writable => "writable",
            Field::Get => "get",
            Field::Set => "set",
        }
    }
}

/// ToPropertyDescriptor (Abschnitt 6.2.6.5).
pub fn to_property_descriptor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Local<'_>,
) -> Completion<PropertyDescriptor<'s>> {
    if !matches!(agent.heap().get(value).kind(), Kind::Object(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let mut descriptor = PropertyDescriptor::default();
    for field in Field::ALL {
        let key = agent.key_from_units(scope, &units(field.name()))?;
        if !has_property(agent, value, key)? {
            continue;
        }
        let found = get(agent, scope, value, key)?;
        let heap = agent.heap();
        let flag = to_boolean(heap, heap.get(found));
        match field {
            Field::Enumerable => descriptor.enumerable = Some(flag),
            Field::Configurable => descriptor.configurable = Some(flag),
            Field::Value => descriptor.value = Some(found),
            Field::Writable => descriptor.writable = Some(flag),
            Field::Get | Field::Set => {
                let function = heap.get(found);
                if !is_callable(heap, function) && !function.is_undefined() {
                    return Err(agent.throw_error(ErrorKind::TypeError));
                }
                if matches!(field, Field::Get) {
                    descriptor.get = Some(found);
                } else {
                    descriptor.set = Some(found);
                }
            }
        }
    }
    if descriptor.is_accessor() && descriptor.is_data() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    Ok(descriptor)
}

/// FromPropertyDescriptor (Abschnitt 6.2.6.4) mit %Object.prototype% aus
/// `realm`, dem aktuellen Realm; *undefined* für `None`.
pub fn from_property_descriptor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    descriptor: Option<&PropertyDescriptor<'_>>,
) -> Completion<Local<'s>> {
    let Some(descriptor) = descriptor else {
        return Ok(agent.heap().local(scope, Value::UNDEFINED));
    };
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let object = ordinary_object_create(agent, scope, Some(prototype))?;
    let flag = |agent: &Agent, flag: Option<bool>| {
        flag.map(|flag| agent.heap().local(scope, Value::boolean(flag)))
    };
    let fields = [
        ("value", descriptor.value),
        ("writable", flag(agent, descriptor.writable)),
        ("get", descriptor.get),
        ("set", descriptor.set),
        ("enumerable", flag(agent, descriptor.enumerable)),
        ("configurable", flag(agent, descriptor.configurable)),
    ];
    for (name, value) in fields {
        let Some(value) = value else {
            continue;
        };
        let key = agent.key_from_units(scope, &units(name))?;
        create_data_property_or_throw(agent, scope, object, key, value)?;
    }
    Ok(object)
}
