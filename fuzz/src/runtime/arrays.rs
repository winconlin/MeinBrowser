//! Arrays against a bounded descriptor/length model, independent of the GC.
//!
//! Commands `[opflags, array, key, payload]` have an opcode in bits 0..2.
//! 0 defines W/E/C data (bits 3/4/5), 1 sets, 2 deletes, 3 defines length
//! with optional writability ((opflags >> 3) % 3: absent/true/false),
//! 4 freezes length, 5 prevents extensions, 6 collects, 7 only checks.
//! Payloads for length select LENGTH_INPUTS; other property values are i8.

use mb_js::agent::{Abrupt, Agent, Completion, ErrorKind, Exception};
use mb_js::descriptor::PropertyDescriptor;
use mb_js::heap::{Local, Options, Persistent, Scope};
use mb_js::key::{PropertyKey, same_key};
use mb_js::object;
use mb_js::value::{Kind, Value};

const ARRAYS: usize = 2;
const MAX_COMMANDS: usize = 64;
const INDICES: [u32; 7] = [0, 1, 2, 7, 64, 1024, u32::MAX - 1];
const NAMES: [&[u16]; 4] = [
    &[0x34, 0x32, 0x39, 0x34, 0x39, 0x36, 0x37, 0x32, 0x39, 0x35],
    &[0x30, 0x31],
    &[0x6e, 0x61, 0x6d, 0x65],
    &[0x6c, 0x65, 0x6e, 0x67, 0x74, 0x68],
];
const KEYS: usize = INDICES.len() + NAMES.len();
const LENGTH: usize = KEYS - 1;

#[derive(Clone, Copy)]
enum Input {
    Number(f64),
    String(&'static [u16]),
    Null,
    Undefined,
    Boolean(bool),
    Symbol,
}

/// Fixed expected conversions avoid sharing the engine's number parser or
/// ToUint32 implementation with the oracle. Array length must be an exact
/// integer in 0..=2^32-1; -0 is accepted and stored as +0.
const LENGTH_INPUTS: [(Input, Result<u32, ErrorKind>); 29] = [
    (Input::Number(0.0), Ok(0)),
    (Input::Number(1.0), Ok(1)),
    (Input::Number(2.0), Ok(2)),
    (Input::Number(3.0), Ok(3)),
    (Input::Number(7.0), Ok(7)),
    (Input::Number(8.0), Ok(8)),
    (Input::Number(64.0), Ok(64)),
    (Input::Number(1025.0), Ok(1025)),
    (Input::Number(4_294_967_294.0), Ok(u32::MAX - 1)),
    (Input::Number(4_294_967_295.0), Ok(u32::MAX)),
    (Input::Number(-0.0), Ok(0)),
    (Input::Number(-1.0), Err(ErrorKind::RangeError)),
    (Input::Number(1.5), Err(ErrorKind::RangeError)),
    (Input::Number(f64::NAN), Err(ErrorKind::RangeError)),
    (Input::Number(f64::INFINITY), Err(ErrorKind::RangeError)),
    (Input::Number(f64::NEG_INFINITY), Err(ErrorKind::RangeError)),
    (Input::String(&[0x32]), Ok(2)),
    (Input::String(&[0x20, 0x33, 0x20]), Ok(3)),
    (Input::String(&[]), Ok(0)),
    (
        Input::String(&[0x34, 0x32, 0x39, 0x34, 0x39, 0x36, 0x37, 0x32, 0x39, 0x35]),
        Ok(u32::MAX),
    ),
    (
        Input::String(&[0x34, 0x32, 0x39, 0x34, 0x39, 0x36, 0x37, 0x32, 0x39, 0x36]),
        Err(ErrorKind::RangeError),
    ),
    (
        Input::String(&[0x31, 0x2e, 0x35]),
        Err(ErrorKind::RangeError),
    ),
    (
        Input::String(&[
            0x6e, 0x6f, 0x74, 0x20, 0x61, 0x20, 0x6e, 0x75, 0x6d, 0x62, 0x65, 0x72,
        ]),
        Err(ErrorKind::RangeError),
    ),
    (Input::String(&[0xd800]), Err(ErrorKind::RangeError)),
    (Input::Null, Ok(0)),
    (Input::Undefined, Err(ErrorKind::RangeError)),
    (Input::Boolean(true), Ok(1)),
    (Input::Boolean(false), Ok(0)),
    (Input::Symbol, Err(ErrorKind::TypeError)),
];

impl Input {
    fn local<'s>(self, agent: &mut Agent, scope: &'s Scope) -> Local<'s> {
        match self {
            Self::Number(value) => agent.heap().local(scope, Value::number(value)),
            Self::String(units) => agent
                .heap_mut()
                .alloc_string(scope, units)
                .expect("bounded array-length string"),
            Self::Null => agent.heap().local(scope, Value::NULL),
            Self::Undefined => agent.heap().local(scope, Value::UNDEFINED),
            Self::Boolean(value) => agent.heap().local(scope, Value::boolean(value)),
            Self::Symbol => agent.new_symbol(scope, None).expect("bounded symbol"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Property {
    value: i64,
    writable: bool,
    enumerable: bool,
    configurable: bool,
    created: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct Model {
    own: [Option<Property>; KEYS],
    extensible: bool,
    clock: usize,
}

impl Model {
    fn new() -> Self {
        let mut own = std::array::from_fn(|_| None);
        own[LENGTH] = Some(Property {
            value: 0,
            writable: true,
            enumerable: false,
            configurable: false,
            created: 0,
        });
        Self {
            own,
            extensible: true,
            clock: 0,
        }
    }

    fn length(&self) -> &Property {
        self.own[LENGTH].as_ref().expect("length always exists")
    }

    fn ordinary_define(&mut self, key: usize, mut property: Property) -> bool {
        if let Some(current) = &self.own[key] {
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
            if !self.extensible {
                return false;
            }
            self.clock += 1;
            property.created = self.clock;
        }
        self.own[key] = Some(property);
        true
    }

    fn define_element(&mut self, key: usize, property: Property) -> bool {
        assert_ne!(key, LENGTH);
        let index = INDICES.get(key).copied();
        if index.is_some_and(|index| i64::from(index) >= self.length().value)
            && !self.length().writable
        {
            return false;
        }
        if !self.ordinary_define(key, property) {
            return false;
        }
        if let Some(index) = index {
            let length = self.own[LENGTH].as_mut().expect("length");
            length.value = length.value.max(i64::from(index) + 1);
        }
        true
    }

    fn define_length(
        &mut self,
        converted: Result<u32, ErrorKind>,
        writable: Option<bool>,
        enumerable: Option<bool>,
        configurable: Option<bool>,
    ) -> Result<bool, ErrorKind> {
        // A direct Define converts its supplied value before inspecting
        // the existing descriptor, including a read-only length.
        let wanted = i64::from(converted?);
        let current = self.length();
        if enumerable == Some(true)
            || configurable == Some(true)
            || (!current.writable && (writable == Some(true) || wanted != current.value))
        {
            return Ok(false);
        }
        let shrinking = wanted < current.value;
        let frozen = writable == Some(false) || !current.writable;
        let blocker = if shrinking {
            // A single mathematical boundary determines the partial
            // result. Never iterate all indices of a sparse array.
            INDICES
                .iter()
                .enumerate()
                .filter(|(key, index)| {
                    i64::from(**index) >= wanted
                        && self.own[*key]
                            .as_ref()
                            .is_some_and(|property| !property.configurable)
                })
                .map(|(_, &index)| index)
                .max()
        } else {
            None
        };
        if shrinking {
            for (key, &index) in INDICES.iter().enumerate() {
                let erase = blocker.map_or(i64::from(index) >= wanted, |stop| index > stop);
                if erase {
                    self.own[key] = None;
                }
            }
        }
        let length = self.own[LENGTH].as_mut().expect("length");
        length.value = blocker.map_or(wanted, |index| i64::from(index) + 1);
        length.writable = !frozen;
        Ok(blocker.is_none())
    }

    fn freeze_length(&mut self) -> bool {
        self.own[LENGTH].as_mut().expect("length").writable = false;
        true
    }

    fn set_element(&mut self, key: usize, value: i64) -> bool {
        let property = if let Some(current) = &self.own[key] {
            if !current.writable {
                return false;
            }
            Property {
                value,
                ..current.clone()
            }
        } else {
            Property {
                value,
                writable: true,
                enumerable: true,
                configurable: true,
                created: 0,
            }
        };
        self.define_element(key, property)
    }

    fn delete(&mut self, key: usize) -> bool {
        if self.own[key]
            .as_ref()
            .is_some_and(|property| !property.configurable)
        {
            return false;
        }
        self.own[key] = None;
        true
    }

    fn keys(&self) -> Vec<usize> {
        let mut keys: Vec<_> = (0..KEYS).filter(|&key| self.own[key].is_some()).collect();
        keys.sort_by_key(|&key| match INDICES.get(key) {
            Some(&index) => (0, u64::from(index)),
            None => (
                1,
                self.own[key].as_ref().expect("present property").created as u64,
            ),
        });
        keys
    }
}

fn restored<'s>(agent: &Agent, scope: &'s Scope, root: &Persistent) -> Local<'s> {
    let heap = agent.heap();
    heap.local(scope, heap.get_persistent(root))
}

fn key<'s>(agent: &mut Agent, scope: &'s Scope, id: usize) -> PropertyKey<'s> {
    if let Some(&index) = INDICES.get(id) {
        PropertyKey::Index(index)
    } else {
        agent
            .key_from_units(scope, NAMES[id - INDICES.len()])
            .expect("bounded array key")
    }
}

fn result(
    agent: &mut Agent,
    scope: &Scope,
    actual: Completion<bool>,
    expected: Result<bool, ErrorKind>,
) {
    match expected {
        Ok(expected) => {
            assert_eq!(actual, Ok(expected), "array operation result");
            assert!(agent.take_exception(scope).is_none(), "no stale exception");
        }
        Err(expected) => {
            assert_eq!(actual, Err(Abrupt::Throw));
            assert!(matches!(
                agent.take_exception(scope),
                Some(Exception::Error(actual)) if actual == expected
            ));
            assert!(
                agent.take_exception(scope).is_none(),
                "exception consumed once"
            );
        }
    }
}

fn check(agent: &mut Agent, roots: &[Persistent; ARRAYS], models: &[Model; ARRAYS]) {
    agent.scope(|agent, scope| {
        for (root, model) in roots.iter().zip(models) {
            let array = restored(agent, scope, root);
            assert_eq!(object::is_extensible(agent, array), Ok(model.extensible));
            assert!(
                object::get_prototype_of(agent, scope, array)
                    .expect("array prototype")
                    .is_none()
            );
            let keys = object::own_property_keys(agent, scope, array).expect("array keys");
            let expected = model.keys();
            assert_eq!(keys.len(), expected.len());
            for (&actual, expected) in keys.iter().zip(expected) {
                let expected = key(agent, scope, expected);
                assert!(same_key(agent.heap(), actual, expected), "array key order");
            }
            for id in 0..KEYS {
                let key = key(agent, scope, id);
                let descriptor =
                    object::get_own_property(agent, scope, array, key).expect("array own property");
                match (descriptor, &model.own[id]) {
                    (Some(actual), Some(expected)) => {
                        assert!(actual.is_data() && !actual.is_accessor());
                        assert_eq!(actual.writable, Some(expected.writable));
                        assert_eq!(actual.enumerable, Some(expected.enumerable));
                        assert_eq!(actual.configurable, Some(expected.configurable));
                        let value = agent.heap().get(actual.value.expect("data value"));
                        assert_eq!(value.as_number(), Some(expected.value as f64));
                        if id == LENGTH && expected.value == 0 {
                            assert_eq!(value.as_number().expect("length").to_bits(), 0);
                        }
                    }
                    (None, None) => {}
                    _ => panic!("array property differs from model"),
                }
                assert_eq!(
                    object::has_property(agent, array, key),
                    Ok(model.own[id].is_some())
                );
                let actual = object::get(agent, scope, array, key, array).expect("array get");
                let value = agent.heap().get(actual);
                if let Some(expected) = &model.own[id] {
                    assert_eq!(value.as_number(), Some(expected.value as f64));
                } else {
                    assert!(matches!(value.kind(), Kind::Undefined));
                }
            }
        }
    });
    agent.heap().verify().expect("verified array heap");
}

fn run(data: &[u8], stress: bool) -> [Model; ARRAYS] {
    let mut agent = Agent::with_options(Options {
        limit: 1 << 20,
        stress,
        verify: true,
    })
    .expect("bounded array agent");
    let roots = std::array::from_fn(|_| {
        agent.scope(|agent, scope| {
            let array = object::array_create(agent, scope, 0, None).expect("array create");
            agent.heap_mut().persist(array)
        })
    });
    let mut models = std::array::from_fn(|_| Model::new());
    check(&mut agent, &roots, &models);
    for command in data.as_chunks::<4>().0.iter().take(MAX_COMMANDS) {
        agent.scope(|agent, scope| {
            let [flags, target, selected, payload] = *command;
            let model = &mut models[usize::from(target) % ARRAYS];
            let array = restored(agent, scope, &roots[usize::from(target) % ARRAYS]);
            let id = usize::from(selected) % KEYS;
            let key = key(agent, scope, id);
            let (input, converted) = LENGTH_INPUTS[usize::from(payload) % LENGTH_INPUTS.len()];
            let number = i64::from(i8::from_ne_bytes([payload]));
            let actual;
            let expected;
            match flags & 7 {
                0 => {
                    let writable = flags & 8 != 0;
                    let enumerable = flags & 16 != 0;
                    let configurable = flags & 32 != 0;
                    let value = if id == LENGTH {
                        input.local(agent, scope)
                    } else {
                        agent.heap().local(scope, Value::int32(number as i32))
                    };
                    let descriptor =
                        PropertyDescriptor::data(value, writable, enumerable, configurable);
                    actual = object::define_own_property(agent, scope, array, key, &descriptor);
                    expected = if id == LENGTH {
                        model.define_length(
                            converted,
                            Some(writable),
                            Some(enumerable),
                            Some(configurable),
                        )
                    } else {
                        Ok(model.define_element(
                            id,
                            Property {
                                value: number,
                                writable,
                                enumerable,
                                configurable,
                                created: 0,
                            },
                        ))
                    };
                }
                1 => {
                    let value = if id == LENGTH {
                        input.local(agent, scope)
                    } else {
                        agent.heap().local(scope, Value::int32(number as i32))
                    };
                    actual = object::set(agent, scope, array, key, value, array);
                    expected = if id == LENGTH {
                        if model.length().writable {
                            model.define_length(converted, None, None, None)
                        } else {
                            Ok(false)
                        }
                    } else {
                        Ok(model.set_element(id, number))
                    };
                }
                2 => {
                    actual = object::delete(agent, scope, array, key);
                    expected = Ok(model.delete(id));
                }
                3 => {
                    let writable = match (flags >> 3) % 3 {
                        0 => None,
                        1 => Some(true),
                        _ => Some(false),
                    };
                    let value = input.local(agent, scope);
                    let descriptor = PropertyDescriptor {
                        value: Some(value),
                        writable,
                        ..PropertyDescriptor::default()
                    };
                    let key = self::key(agent, scope, LENGTH);
                    actual = object::define_own_property(agent, scope, array, key, &descriptor);
                    expected = model.define_length(converted, writable, None, None);
                }
                4 => {
                    let descriptor = PropertyDescriptor {
                        writable: Some(false),
                        ..PropertyDescriptor::default()
                    };
                    let key = self::key(agent, scope, LENGTH);
                    actual = object::define_own_property(agent, scope, array, key, &descriptor);
                    expected = Ok(model.freeze_length());
                }
                5 => {
                    actual = object::prevent_extensions(agent, array);
                    model.extensible = false;
                    expected = Ok(true);
                }
                6 => {
                    agent.heap_mut().collect();
                    return;
                }
                _ => return,
            }
            result(agent, scope, actual, expected);
        });
        check(&mut agent, &roots, &models);
    }
    agent.heap_mut().collect();
    check(&mut agent, &roots, &models);
    for root in roots {
        agent.heap_mut().release(root);
    }
    agent.heap_mut().collect();
    agent
        .heap()
        .verify()
        .expect("array heap after root release");
    models
}

pub(super) fn exercise(data: &[u8]) {
    let normal = run(data, false);
    assert_eq!(normal, run(data, true));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both(commands: &[[u8; 4]]) -> [Model; ARRAYS] {
        let bytes: Vec<_> = commands.iter().flatten().copied().collect();
        let normal = run(&bytes, false);
        assert_eq!(normal, run(&bytes, true));
        normal
    }

    #[test]
    fn sparse_indices_and_numeric_string_keys_keep_length_separate() {
        let models = both(&[
            [56, 0, 6, 12],
            [56, 0, 7, 13],
            [56, 0, 8, 14],
            [56, 0, 9, 15],
            [2, 0, 6, 0],
            [6, 0, 0, 0],
        ]);
        assert_eq!(models[0].length().value, i64::from(u32::MAX));
        assert_eq!(models[0].keys(), [LENGTH, 7, 8, 9]);
        assert!(models[0].own[6].is_none());
        assert_eq!(models[0].own[7].as_ref().expect("named").value, 13);
    }

    #[test]
    fn shrinking_stops_at_the_highest_fixed_element_and_still_freezes_length() {
        let models = both(&[
            [56, 0, 0, 1],
            [24, 0, 2, 2],
            [56, 0, 3, 3],
            [56, 0, 4, 4],
            [19, 0, 0, 1],
            [1, 0, 1, 5],
            [1, 0, 3, 6],
            [6, 0, 0, 0],
        ]);
        let model = &models[0];
        assert_eq!(model.length().value, 3);
        assert!(!model.length().writable);
        assert!(model.own[3].is_none() && model.own[4].is_none());
        assert_eq!(model.own[1].as_ref().expect("below length").value, 5);
        assert!(model.own[2].is_some(), "blocking element remains");
    }

    #[test]
    fn nonextensible_arrays_allow_length_growth_and_existing_value_changes() {
        let models = both(&[
            [56, 0, 0, 1],
            [5, 0, 0, 0],
            [3, 0, 0, 7],
            [1, 0, 0, 2],
            [1, 0, 1, 3],
            [1, 0, 9, 4],
            [6, 0, 0, 0],
        ]);
        let model = &models[0];
        assert_eq!(model.length().value, 1025);
        assert!(!model.extensible);
        assert_eq!(model.own[0].as_ref().expect("existing index").value, 2);
        assert!(model.own[1].is_none() && model.own[9].is_none());
    }

    #[test]
    fn direct_length_conversion_errors_precede_descriptor_checks_but_set_checks_readonly_first() {
        let mut commands = vec![[3, 0, 0, 2], [4, 0, 0, 0]];
        for payload in [11, 12, 13, 14, 15, 20, 21, 22, 23, 25, 28] {
            commands.push([56, 0, 10, payload]);
            commands.push([1, 0, 10, payload]);
        }
        commands.extend([[11, 0, 0, 2], [2, 0, 10, 0]]);
        let models = both(&commands);
        assert_eq!(models[0].length().value, 2);
        assert!(!models[0].length().writable);
    }

    #[test]
    fn all_length_input_classes_and_negative_zero_are_checked_in_both_gc_modes() {
        let commands: Vec<_> = (0..LENGTH_INPUTS.len() as u8)
            .map(|payload| [3, 0, 0, payload])
            .collect();
        let models = both(&commands);
        assert_eq!(models[0].length().value, 0);
        assert!(models[0].length().writable);
        assert_eq!(models[1].length().value, 0);
    }

    #[test]
    fn empty_partial_and_extra_commands_are_bounded() {
        both(&[]);
        exercise(&[3, 0, 0]);
        let mut commands = vec![[7, 0, 0, 0]; MAX_COMMANDS];
        commands.push([56, 0, 6, 1]);
        let models = both(&commands);
        assert_eq!(models[0].length().value, 0);
        assert!(models[0].own[6].is_none());
    }
}
