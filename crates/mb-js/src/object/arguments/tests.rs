use super::*;
use crate::heap::Options;
use crate::object::{
    define_own_property, delete, get, get_own_property, ordinary_object_create, own_property_keys,
    set,
};

fn agent_with(stress: bool) -> Agent {
    Agent::with_options(Options {
        stress,
        verify: true,
        ..Options::default()
    })
    .expect("Speicher")
}

/// Eine interne Zelle mit den Zahlen.
fn cell<'s>(agent: &mut Agent, scope: &'s Scope, values: &[f64]) -> Local<'s> {
    let cell = agent
        .heap_mut()
        .alloc_slots(scope, values.len())
        .expect("Speicher");
    let heap = agent.heap();
    let Kind::Internal(target) = heap.get(cell).kind() else {
        panic!("keine interne Zelle");
    };
    for (index, &value) in values.iter().enumerate() {
        heap.set_slot(target, index, Value::number(value));
    }
    cell
}

fn slot_of(agent: &Agent, cell: Local<'_>, index: usize) -> Option<f64> {
    let heap = agent.heap();
    match heap.get(cell).kind() {
        Kind::Internal(target) => heap.slot(target, index).and_then(|value| value.as_number()),
        _ => None,
    }
}

fn set_slot_of(agent: &Agent, cell: Local<'_>, index: usize, value: f64) {
    let heap = agent.heap();
    if let Kind::Internal(target) = heap.get(cell).kind() {
        heap.set_slot(target, index, Value::number(value));
    }
}

fn name<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> PropertyKey<'s> {
    agent.key_from_units(scope, &units(text)).expect("Speicher")
}

fn read(agent: &mut Agent, scope: &Scope, object: Local<'_>, index: u32) -> Option<f64> {
    let value = get(agent, scope, object, PropertyKey::Index(index), object).expect("kein Fehler");
    agent.heap().get(value).as_number()
}

fn write(agent: &mut Agent, scope: &Scope, object: Local<'_>, index: u32, value: f64) -> bool {
    let value = agent.heap().local(scope, Value::number(value));
    set(
        agent,
        scope,
        object,
        PropertyKey::Index(index),
        value,
        object,
    )
    .expect("kein Fehler")
}

fn own<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> PropertyDescriptor<'s> {
    get_own_property(agent, scope, object, key)
        .expect("kein Fehler")
        .expect("eigene Eigenschaft")
}

/// Die Attribute einer Dateneigenschaft: Wert, writable, enumerable,
/// configurable.
fn data(agent: &Agent, descriptor: &PropertyDescriptor<'_>) -> (Option<f64>, bool, bool, bool) {
    let value = descriptor
        .value
        .and_then(|value| agent.heap().get(value).as_number());
    (
        value,
        descriptor.writable == Some(true),
        descriptor.enumerable == Some(true),
        descriptor.configurable == Some(true),
    )
}

/// Die eigenen Schlüssel als Text, Indizes als Zahl.
fn keys(agent: &mut Agent, scope: &Scope, object: Local<'_>) -> Vec<String> {
    own_property_keys(agent, scope, object)
        .expect("kein Fehler")
        .into_iter()
        .map(|key| match key {
            PropertyKey::Index(index) => index.to_string(),
            PropertyKey::String(string) => match agent.heap().get(string).kind() {
                Kind::String(text) => {
                    String::from_utf16(&agent.heap().string_units(text)).expect("UTF-16")
                }
                kind => format!("{kind:?}"),
            },
            PropertyKey::Symbol(_) => "Symbol".to_owned(),
        })
        .collect()
}

/// Ein verknüpftes Objekt für zwei Parameter, deren Bindungen in den
/// Plätzen 1 und 2 von `context` liegen, und drei Argumente.
fn mapped<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    context: Local<'_>,
    map: &[Option<u32>],
) -> Local<'s> {
    let prototype = ordinary_object_create(agent, scope, None).expect("Speicher");
    let function = ordinary_object_create(agent, scope, None).expect("Speicher");
    let arguments = cell(agent, scope, &[1.0, 2.0, 3.0]);
    create_mapped_arguments_object(agent, scope, prototype, function, arguments, context, map)
        .expect("kein Fehler")
}

#[test]
fn unmapped_objects_have_length_indices_and_callee() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let prototype = ordinary_object_create(agent, scope, None).expect("Speicher");
        let arguments = cell(agent, scope, &[1.0, 2.0]);
        let object = create_unmapped_arguments_object(agent, scope, prototype, arguments)
            .expect("kein Fehler");
        let heap = agent.heap();
        let class = object_of(heap, object).and_then(|target| heap.object_class(target));
        assert_eq!(class, Some(ObjectClass::Arguments));
        assert_eq!(keys(agent, scope, object), ["0", "1", "length", "callee"]);
        let length = name(agent, scope, "length");
        let descriptor = own(agent, scope, object, length);
        assert_eq!(data(agent, &descriptor), (Some(2.0), true, false, true));
        let descriptor = own(agent, scope, object, PropertyKey::Index(1));
        assert_eq!(data(agent, &descriptor), (Some(2.0), true, true, true));
        // Abweichung: *undefined* statt %ThrowTypeError%.
        let callee = name(agent, scope, "callee");
        let descriptor = own(agent, scope, object, callee);
        assert!(descriptor.is_accessor());
        let undefined = |local: Option<Local<'_>>| {
            local.is_some_and(|local| agent.heap().get(local).is_undefined())
        };
        assert!(undefined(descriptor.get) && undefined(descriptor.set));
        assert_eq!(
            (descriptor.enumerable, descriptor.configurable),
            (Some(false), Some(false))
        );
        // Ohne Verknüpfung bleibt ein geschriebener Wert, wo er ist.
        assert!(write(agent, scope, object, 0, 5.0));
        assert_eq!(read(agent, scope, object, 0), Some(5.0));
        assert_eq!(slot_of(agent, arguments, 0), Some(1.0));
    });
}

#[test]
fn mapped_indices_read_and_write_the_bindings() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let context = cell(agent, scope, &[0.0, 10.0, 20.0]);
        let object = mapped(agent, scope, context, &[Some(1), Some(2)]);
        assert_eq!(
            keys(agent, scope, object),
            ["0", "1", "2", "length", "callee"]
        );
        assert_eq!(read(agent, scope, object, 0), Some(10.0));
        assert_eq!(read(agent, scope, object, 1), Some(20.0));
        // Ein Argument ohne Parameter ist nicht verknüpft.
        assert_eq!(read(agent, scope, object, 2), Some(3.0));
        let descriptor = own(agent, scope, object, PropertyKey::Index(0));
        assert_eq!(data(agent, &descriptor), (Some(10.0), true, true, true));
        assert!(write(agent, scope, object, 0, 5.0));
        assert_eq!(slot_of(agent, context, 1), Some(5.0));
        set_slot_of(agent, context, 2, 7.0);
        assert_eq!(read(agent, scope, object, 1), Some(7.0));
        let callee = name(agent, scope, "callee");
        let descriptor = own(agent, scope, object, callee);
        assert!(descriptor.is_data());
        assert_eq!(
            (
                descriptor.writable,
                descriptor.enumerable,
                descriptor.configurable
            ),
            (Some(true), Some(false), Some(true))
        );
    });
}

#[test]
fn a_repeated_parameter_name_maps_only_the_last() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let context = cell(agent, scope, &[0.0, 10.0]);
        let object = mapped(agent, scope, context, &[None, Some(1)]);
        assert_eq!(read(agent, scope, object, 0), Some(1.0));
        assert_eq!(read(agent, scope, object, 1), Some(10.0));
        assert!(write(agent, scope, object, 0, 4.0));
        assert_eq!(slot_of(agent, context, 1), Some(10.0));
    });
}

#[test]
fn defining_and_deleting_end_the_mapping() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let context = cell(agent, scope, &[0.0, 10.0, 20.0]);
        let object = mapped(agent, scope, context, &[Some(1), Some(2)]);
        // Mit einem Wert bleibt die Verknüpfung und die Bindung bekommt ihn.
        let value = agent.heap().local(scope, Value::number(8.0));
        let descriptor = PropertyDescriptor {
            value: Some(value),
            ..PropertyDescriptor::default()
        };
        let key = PropertyKey::Index(0);
        assert_eq!(
            define_own_property(agent, scope, object, key, &descriptor),
            Ok(true)
        );
        assert_eq!(slot_of(agent, context, 1), Some(8.0));
        set_slot_of(agent, context, 1, 9.0);
        assert_eq!(read(agent, scope, object, 0), Some(9.0));
        // Schreibgeschützt behält der Index den Wert der Bindung.
        let descriptor = PropertyDescriptor {
            writable: Some(false),
            ..PropertyDescriptor::default()
        };
        assert_eq!(
            define_own_property(agent, scope, object, key, &descriptor),
            Ok(true)
        );
        set_slot_of(agent, context, 1, 11.0);
        let descriptor = own(agent, scope, object, key);
        assert_eq!(data(agent, &descriptor), (Some(9.0), false, true, true));
        // Eine Zugriffseigenschaft endet die Verknüpfung.
        let undefined = agent.heap().local(scope, Value::UNDEFINED);
        let descriptor = PropertyDescriptor {
            get: Some(undefined),
            ..PropertyDescriptor::default()
        };
        let key = PropertyKey::Index(1);
        assert_eq!(
            define_own_property(agent, scope, object, key, &descriptor),
            Ok(true)
        );
        set_slot_of(agent, context, 2, 21.0);
        assert!(own(agent, scope, object, key).is_accessor());
        assert_eq!(slot_of(agent, context, 2), Some(21.0));
        // Gelöscht und neu angelegt ist ein Index nicht mehr verknüpft.
        let context = cell(agent, scope, &[0.0, 10.0, 20.0]);
        let object = mapped(agent, scope, context, &[Some(1), Some(2)]);
        assert_eq!(
            delete(agent, scope, object, PropertyKey::Index(1)),
            Ok(true)
        );
        assert_eq!(read(agent, scope, object, 1), None);
        assert!(write(agent, scope, object, 1, 6.0));
        assert_eq!(slot_of(agent, context, 2), Some(20.0));
        set_slot_of(agent, context, 2, 22.0);
        assert_eq!(read(agent, scope, object, 1), Some(6.0));
    });
}

/// Schreibgeschützt mit einem Wert: Die Bindung bekommt ihn noch, danach
/// endet die Verknüpfung.
#[test]
fn a_value_and_read_only_together_write_once() {
    let mut agent = agent_with(false);
    agent.scope(|agent, scope| {
        let context = cell(agent, scope, &[0.0, 10.0, 20.0]);
        let object = mapped(agent, scope, context, &[Some(1), Some(2)]);
        let value = agent.heap().local(scope, Value::number(6.0));
        let descriptor = PropertyDescriptor {
            value: Some(value),
            writable: Some(false),
            ..PropertyDescriptor::default()
        };
        let key = PropertyKey::Index(0);
        assert_eq!(
            define_own_property(agent, scope, object, key, &descriptor),
            Ok(true)
        );
        assert_eq!(slot_of(agent, context, 1), Some(6.0));
        set_slot_of(agent, context, 1, 7.0);
        assert_eq!(read(agent, scope, object, 0), Some(6.0));
        assert!(!write(agent, scope, object, 0, 8.0));
        assert_eq!(slot_of(agent, context, 1), Some(7.0));
    });
}

#[test]
fn arguments_objects_survive_collections() {
    let mut agent = agent_with(true);
    agent.scope(|agent, scope| {
        let context = cell(agent, scope, &[0.0, 10.0, 20.0]);
        let object = mapped(agent, scope, context, &[Some(1), Some(2)]);
        let prototype = ordinary_object_create(agent, scope, None).expect("Speicher");
        let arguments = cell(agent, scope, &[1.0, 2.0]);
        let unmapped = create_unmapped_arguments_object(agent, scope, prototype, arguments)
            .expect("kein Fehler");
        for round in 0..4 {
            let value = f64::from(round);
            assert!(write(agent, scope, object, 1, value));
            assert_eq!(slot_of(agent, context, 2), Some(value));
            assert_eq!(read(agent, scope, unmapped, 1), Some(2.0));
            assert_eq!(read(agent, scope, object, 2), Some(3.0));
        }
    });
}
