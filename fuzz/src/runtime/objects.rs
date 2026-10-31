//! Ordinary objects against a small, independent data-property model.
//!
//! Four-byte commands: opcode in byte 0's low three bits, descriptor flags
//! writable/enumerable/configurable in bits 3/4/5, object in byte 1 modulo
//! four, key in byte 2 modulo sixteen, and signed numeric payload in byte 3.
//! SetPrototype uses byte 2 modulo five, with four meaning null. String
//! payloads select zero, one or two UTF-16 units, including lone surrogates.

use mb_js::agent::Agent;
use mb_js::compare::same_value;
use mb_js::descriptor::PropertyDescriptor;
use mb_js::heap::{Local, Options, Persistent, Scope};
use mb_js::key::{PropertyKey, same_key};
use mb_js::object;
use mb_js::value::{Kind, Value};

const OBJECTS: usize = 4;
const KEYS: usize = 16;
const INDICES: [u32; 7] = [0, 1, 2, 15, 64, 1024, u32::MAX - 1];
const NAMES: [&[u16]; 7] = [
    &[0x61],
    &[0x62],
    &[0x30, 0x30],
    &[0x30, 0x31],
    &[0x34, 0x32, 0x39, 0x34, 0x39, 0x36, 0x37, 0x32, 0x39, 0x35],
    &[],
    &[0xd800],
];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Data {
    Undefined,
    Number(i32),
    String(Vec<u16>),
}

impl Data {
    fn read(agent: &Agent, value: Local<'_>) -> Self {
        match agent.heap().get(value).kind() {
            Kind::Undefined => Self::Undefined,
            Kind::Number(value) => {
                assert!(value.is_finite() && value.fract() == 0.0);
                assert!((-128.0..=127.0).contains(&value));
                Self::Number(value as i32)
            }
            Kind::String(string) => Self::String(agent.heap().string_units(string)),
            value => panic!("unexpected data-property value: {value:?}"),
        }
    }

    fn local<'s>(&self, agent: &mut Agent, scope: &'s Scope) -> Local<'s> {
        match self {
            Self::Undefined => agent.heap().local(scope, Value::UNDEFINED),
            Self::Number(value) => agent.heap().local(scope, Value::int32(*value)),
            Self::String(units) => agent
                .heap_mut()
                .alloc_string(scope, units)
                .expect("bounded object-string allocation"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Property {
    value: Data,
    writable: bool,
    enumerable: bool,
    configurable: bool,
    created: usize,
}

impl Property {
    fn command(value: Data, flags: u8) -> Self {
        Self {
            value,
            writable: flags & 8 != 0,
            enumerable: flags & 16 != 0,
            configurable: flags & 32 != 0,
            created: 0,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ModelObject {
    own: [Option<Property>; KEYS],
    prototype: Option<usize>,
    extensible: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Model {
    objects: [ModelObject; OBJECTS],
    clock: usize,
}

impl Model {
    fn new() -> Self {
        Self {
            objects: std::array::from_fn(|_| ModelObject {
                own: std::array::from_fn(|_| None),
                prototype: None,
                extensible: true,
            }),
            clock: 0,
        }
    }

    fn find(&self, mut id: usize, key: usize) -> Option<&Property> {
        loop {
            let object = &self.objects[id];
            if let Some(property) = &object.own[key] {
                return Some(property);
            }
            id = object.prototype?;
        }
    }

    fn define(&mut self, id: usize, key: usize, mut property: Property) -> bool {
        let object = &mut self.objects[id];
        if let Some(current) = &object.own[key] {
            if !current.configurable
                && (property.configurable
                    || property.enumerable != current.enumerable
                    || (!current.writable
                        && (property.writable || property.value != current.value)))
            {
                return false;
            }
            property.created = current.created;
        } else {
            if !object.extensible {
                return false;
            }
            self.clock += 1;
            property.created = self.clock;
        }
        object.own[key] = Some(property);
        true
    }

    fn set(&mut self, id: usize, key: usize, value: Data) -> bool {
        if self
            .find(id, key)
            .is_some_and(|property| !property.writable)
        {
            return false;
        }
        if let Some(property) = &mut self.objects[id].own[key] {
            property.value = value;
            true
        } else {
            self.define(id, key, Property::command(value, 8 | 16 | 32))
        }
    }

    fn delete(&mut self, id: usize, key: usize) -> bool {
        let property = &mut self.objects[id].own[key];
        if property
            .as_ref()
            .is_some_and(|property| !property.configurable)
        {
            return false;
        }
        *property = None;
        true
    }

    fn prototype(&mut self, id: usize, prototype: Option<usize>) -> bool {
        if self.objects[id].prototype == prototype {
            return true;
        }
        if !self.objects[id].extensible {
            return false;
        }
        let mut ancestor = prototype;
        while let Some(current) = ancestor {
            if current == id {
                return false;
            }
            ancestor = self.objects[current].prototype;
        }
        self.objects[id].prototype = prototype;
        true
    }

    fn keys(&self, id: usize) -> Vec<usize> {
        let own = &self.objects[id].own;
        let mut keys: Vec<_> = (0..KEYS).filter(|&key| own[key].is_some()).collect();
        keys.sort_by_key(|&key| match key {
            0..7 => (0, INDICES[key] as usize),
            7..14 => (1, own[key].as_ref().expect("property").created),
            _ => (2, own[key].as_ref().expect("property").created),
        });
        keys
    }
}

struct Roots {
    objects: [Persistent; OBJECTS],
    symbols: [Persistent; 2],
}

fn restored<'s>(agent: &Agent, scope: &'s Scope, root: &Persistent) -> Local<'s> {
    let heap = agent.heap();
    heap.local(scope, heap.get_persistent(root))
}

fn key<'s>(agent: &mut Agent, scope: &'s Scope, roots: &Roots, id: usize) -> PropertyKey<'s> {
    match id {
        0..7 => PropertyKey::Index(INDICES[id]),
        7..14 => agent
            .key_from_units(scope, NAMES[id - 7])
            .expect("bounded property-key allocation"),
        _ => PropertyKey::Symbol(restored(agent, scope, &roots.symbols[id - 14])),
    }
}

fn check(agent: &mut Agent, roots: &Roots, model: &Model) {
    agent.scope(|agent, scope| {
        for id in 0..OBJECTS {
            let actual = restored(agent, scope, &roots.objects[id]);
            let expected = &model.objects[id];
            assert_eq!(
                object::is_extensible(agent, actual),
                Ok(expected.extensible)
            );
            let prototype = object::get_prototype_of(agent, scope, actual).expect("prototype");
            match (prototype, expected.prototype) {
                (Some(prototype), Some(id)) => {
                    let expected = restored(agent, scope, &roots.objects[id]);
                    let heap = agent.heap();
                    assert!(same_value(heap, heap.get(prototype), heap.get(expected)));
                }
                (None, None) => {}
                _ => panic!("prototype differs from model"),
            }
            let keys = object::own_property_keys(agent, scope, actual).expect("own keys");
            let ordered = model.keys(id);
            assert_eq!(keys.len(), ordered.len());
            for (&actual, expected) in keys.iter().zip(ordered) {
                let expected = key(agent, scope, roots, expected);
                assert!(same_key(agent.heap(), actual, expected));
            }
            for index in 0..KEYS {
                let key = key(agent, scope, roots, index);
                let descriptor =
                    object::get_own_property(agent, scope, actual, key).expect("own property");
                match (descriptor, &expected.own[index]) {
                    (Some(actual), Some(expected)) => {
                        assert!(!actual.is_accessor());
                        assert_eq!(actual.writable, Some(expected.writable));
                        assert_eq!(actual.enumerable, Some(expected.enumerable));
                        assert_eq!(actual.configurable, Some(expected.configurable));
                        assert_eq!(
                            Data::read(agent, actual.value.expect("data value")),
                            expected.value
                        );
                    }
                    (None, None) => {}
                    _ => panic!("own property differs from model"),
                }
                let expected = model.find(id, index);
                assert_eq!(
                    object::has_property(agent, actual, key),
                    Ok(expected.is_some())
                );
                let value = object::get(agent, scope, actual, key, actual).expect("get");
                assert_eq!(
                    Data::read(agent, value),
                    expected.map_or(Data::Undefined, |p| p.value.clone())
                );
            }
        }
    });
}

fn string_payload(payload: u8) -> Data {
    let unit = u16::from(payload);
    Data::String(match payload & 3 {
        0 => vec![],
        1 => vec![unit],
        2 => vec![0xd800 | unit],
        _ => vec![unit, 0xdc00 | unit],
    })
}

fn run(data: &[u8], stress: bool) -> Model {
    let mut agent = Agent::with_options(Options {
        limit: 1 << 20,
        stress,
        verify: true,
    })
    .expect("bounded object agent");
    let roots = Roots {
        objects: std::array::from_fn(|_| {
            agent.scope(|agent, scope| {
                let object = object::ordinary_object_create(agent, scope, None).expect("object");
                agent.heap_mut().persist(object)
            })
        }),
        symbols: std::array::from_fn(|_| {
            agent.scope(|agent, scope| {
                let symbol = agent.new_symbol(scope, None).expect("symbol");
                agent.heap_mut().persist(symbol)
            })
        }),
    };
    let mut model = Model::new();
    check(&mut agent, &roots, &model);
    for command in data.as_chunks::<4>().0.iter().take(64) {
        agent.scope(|agent, scope| {
            let id = usize::from(command[1]) % OBJECTS;
            let index = usize::from(command[2]) % KEYS;
            let actual = restored(agent, scope, &roots.objects[id]);
            let key = key(agent, scope, &roots, index);
            let value = Data::Number(i32::from(command[3] as i8));
            match command[0] & 7 {
                0 | 6 => {
                    let value = if command[0] & 7 == 6 {
                        string_payload(command[3])
                    } else {
                        value
                    };
                    let property = Property::command(value, command[0]);
                    let value = property.value.local(agent, scope);
                    let descriptor = PropertyDescriptor::data(
                        value,
                        property.writable,
                        property.enumerable,
                        property.configurable,
                    );
                    assert_eq!(
                        object::define_own_property(agent, scope, actual, key, &descriptor),
                        Ok(model.define(id, index, property))
                    );
                }
                1 => {
                    let local = value.local(agent, scope);
                    assert_eq!(
                        object::set(agent, scope, actual, key, local, actual),
                        Ok(model.set(id, index, value))
                    );
                }
                2 => assert_eq!(
                    object::delete(agent, scope, actual, key),
                    Ok(model.delete(id, index))
                ),
                3 => {
                    let prototype = usize::from(command[2]) % 5;
                    let prototype = (prototype < OBJECTS).then_some(prototype);
                    let local = prototype.map(|id| restored(agent, scope, &roots.objects[id]));
                    assert_eq!(
                        object::set_prototype_of(agent, actual, local),
                        Ok(model.prototype(id, prototype))
                    );
                }
                4 => {
                    assert_eq!(object::prevent_extensions(agent, actual), Ok(true));
                    model.objects[id].extensible = false;
                }
                5 => agent.heap_mut().collect(),
                _ => {}
            }
        });
        check(&mut agent, &roots, &model);
    }
    agent.heap_mut().collect();
    check(&mut agent, &roots, &model);
    for root in roots.objects.into_iter().chain(roots.symbols) {
        agent.heap_mut().release(root);
    }
    agent.heap_mut().collect();
    agent.heap().verify().expect("heap after releasing roots");
    model
}

pub(super) fn exercise(data: &[u8]) {
    run(data, false);
    run(data, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both(commands: &[[u8; 4]]) -> Model {
        let bytes: Vec<_> = commands.iter().flatten().copied().collect();
        let normal = run(&bytes, false);
        assert_eq!(normal, run(&bytes, true));
        normal
    }

    #[test]
    fn deletion_reinsertion_preserves_the_three_key_order_groups() {
        let mut commands: Vec<_> = (0..16).rev().map(|key| [56, 0, key, key]).collect();
        commands.extend([[2, 0, 8, 0], [56, 0, 8, 42], [5, 0, 0, 0]]);
        let model = both(&commands);
        assert_eq!(
            model.keys(0),
            [0, 1, 2, 3, 4, 5, 6, 13, 12, 11, 10, 9, 7, 8, 15, 14]
        );
        assert_eq!(
            model.objects[0].own[8].as_ref().expect("reinserted").value,
            Data::Number(42)
        );
    }

    #[test]
    fn fixed_descriptors_reject_changes_but_writable_values_can_change() {
        let model = both(&[
            [0, 0, 7, 1],
            [0, 0, 7, 1],
            [0, 0, 7, 2],
            [8, 0, 7, 1],
            [16, 0, 7, 1],
            [32, 0, 7, 1],
            [1, 0, 7, 3],
            [2, 0, 7, 0],
            [8, 0, 8, 4],
            [1, 0, 8, 5],
            [0, 0, 8, 6],
            [1, 0, 8, 7],
        ]);
        assert_eq!(
            model.objects[0].own[7].as_ref().expect("fixed").value,
            Data::Number(1)
        );
        let changed = model.objects[0].own[8].as_ref().expect("write then freeze");
        assert_eq!(changed.value, Data::Number(6));
        assert!(!changed.writable);
    }

    #[test]
    fn inherited_writes_cycles_and_nonextensible_objects_follow_the_model() {
        let model = both(&[
            [56, 0, 7, 1],
            [0, 0, 8, 2],
            [3, 1, 0, 0],
            [1, 1, 7, 3],
            [1, 1, 8, 4],
            [3, 0, 1, 0],
            [4, 1, 0, 0],
            [1, 1, 9, 5],
            [3, 1, 4, 0],
            [3, 1, 0, 0],
            [1, 1, 7, 6],
            [5, 0, 0, 0],
        ]);
        assert_eq!(model.objects[0].prototype, None);
        assert_eq!(model.objects[1].prototype, Some(0));
        assert!(model.objects[1].own[8].is_none());
        assert!(model.objects[1].own[9].is_none());
        assert_eq!(
            model.objects[1].own[7]
                .as_ref()
                .expect("own writable")
                .value,
            Data::Number(6)
        );
        assert_eq!(
            model.objects[0].own[7]
                .as_ref()
                .expect("parent unchanged")
                .value,
            Data::Number(1)
        );
    }

    #[test]
    fn utf16_data_and_signed_numbers_survive_collection() {
        let model = both(&[
            [62, 0, 7, 0],
            [62, 0, 8, 1],
            [62, 0, 9, 2],
            [62, 0, 10, 3],
            [56, 0, 11, 128],
            [56, 0, 12, 255],
            [5, 0, 0, 0],
            [7, 0, 0, 0],
        ]);
        assert_eq!(
            model.objects[0].own[9].as_ref().expect("surrogate").value,
            Data::String(vec![0xd802])
        );
        assert_eq!(
            model.objects[0].own[10].as_ref().expect("two units").value,
            Data::String(vec![3, 0xdc03])
        );
        assert_eq!(
            model.objects[0].own[11].as_ref().expect("negative").value,
            Data::Number(-128)
        );
        assert_eq!(
            model.objects[0].own[12].as_ref().expect("negative").value,
            Data::Number(-1)
        );
    }
}
