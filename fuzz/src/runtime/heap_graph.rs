//! A separate graph model for moving GC, handles, strings and slot cells.
//!
//! Commands are four bytes `[opcode, a, b, c]`, with opcode modulo eight:
//! 0 string->root a; 1 slots->root a; 2 root a slot b<-root c;
//! 3 root a slot b<-signed c; 4 root a<-root b; 5 release root a;
//! 6 collect; 7 string->root a through nested scopes.

use mb_js::heap::{Heap, Options, Persistent};
use mb_js::value::{Kind, Ref, Value};

const MAX_COMMANDS: usize = 128;
const ROOTS: usize = 16;
const LIMIT: usize = 1 << 20;

#[derive(Clone, Copy, Debug)]
enum ModelValue {
    Undefined,
    Integer(i32),
    Node(usize),
}

enum Node {
    String(Vec<u16>),
    Slots(Vec<ModelValue>),
}

struct Root {
    node: usize,
    handle: Persistent,
}

struct Model {
    nodes: Vec<Node>,
    roots: [Option<Root>; ROOTS],
}

impl Model {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            roots: std::array::from_fn(|_| None),
        }
    }

    fn root_node(&self, root: u8) -> Option<usize> {
        self.roots[usize::from(root) % ROOTS]
            .as_ref()
            .map(|root| root.node)
    }

    fn replace(&mut self, heap: &mut Heap, root: u8, replacement: Option<Root>) {
        if let Some(previous) =
            std::mem::replace(&mut self.roots[usize::from(root) % ROOTS], replacement)
        {
            heap.release(previous.handle);
        }
    }

    fn value<'h>(&self, heap: &'h Heap, root: u8) -> Value<'h> {
        self.roots[usize::from(root) % ROOTS]
            .as_ref()
            .map_or(Value::UNDEFINED, |root| heap.get_persistent(&root.handle))
    }

    /// Check contents and aliasing by traversing actual and modeled edges
    /// together. Reference offsets are only compared within this immutable
    /// borrow of the heap; they are never restored after a collection.
    fn check(&self, heap: &Heap, collected: bool) {
        heap.verify().expect("valid heap graph");
        let mut seen: Vec<Option<Ref<'_>>> = vec![None; self.nodes.len()];
        let mut pending: Vec<_> = self
            .roots
            .iter()
            .flatten()
            .map(|root| {
                (
                    ModelValue::Node(root.node),
                    heap.get_persistent(&root.handle),
                )
            })
            .collect();
        let mut expected_bytes = 0;
        while let Some((expected, actual)) = pending.pop() {
            let node = match expected {
                ModelValue::Undefined => {
                    assert!(actual.is_undefined(), "undefined slot value");
                    continue;
                }
                ModelValue::Integer(integer) => {
                    assert_eq!(actual.as_int32(), Some(integer), "integer slot value");
                    continue;
                }
                ModelValue::Node(node) => node,
            };
            let reference = match actual.kind() {
                Kind::String(reference) | Kind::Internal(reference) => reference,
                kind => panic!("graph node has unexpected kind: {kind:?}"),
            };
            if let Some(previous) = seen[node] {
                assert_eq!(reference, previous, "one node keeps its identity");
                continue;
            }
            assert!(
                !seen.iter().flatten().any(|&other| other == reference),
                "distinct graph nodes must not alias"
            );
            seen[node] = Some(reference);
            match &self.nodes[node] {
                Node::String(units) => {
                    assert!(matches!(actual.kind(), Kind::String(_)), "string kind");
                    assert_eq!(heap.string_len(reference), units.len());
                    assert_eq!(heap.string_units(reference), *units);
                    for (index, &unit) in units.iter().enumerate() {
                        assert_eq!(heap.string_unit(reference, index), Some(unit));
                    }
                    assert_eq!(heap.string_unit(reference, units.len()), None);
                    let latin1 = units.iter().all(|&unit| unit <= 0xff);
                    assert_eq!(heap.is_latin1(reference), latin1);
                    let per_word = if latin1 { 8 } else { 4 };
                    expected_bytes += 8 + units.len().div_ceil(per_word) * 8;
                }
                Node::Slots(slots) => {
                    assert!(matches!(actual.kind(), Kind::Internal(_)), "slot-cell kind");
                    assert_eq!(heap.slot_count(reference), slots.len());
                    for (index, &value) in slots.iter().enumerate() {
                        pending.push((value, heap.slot(reference, index).expect("existing slot")));
                    }
                    assert!(heap.slot(reference, slots.len()).is_none());
                    expected_bytes += 8 + slots.len() * 8;
                }
            }
        }
        if collected {
            assert_eq!(
                heap.used_bytes(),
                expected_bytes,
                "all and only modeled reachable cells survive"
            );
        }
    }
}

fn string_units(length: u8, value: u8) -> Vec<u16> {
    let base = if length & 0x80 == 0 {
        u16::from(value)
    } else {
        u16::from_be_bytes([length, value])
    };
    (0..length % 17)
        .map(|index| base.wrapping_add(u16::from(index)))
        .collect()
}

fn new_string(heap: &mut Heap, units: &[u16], nested: bool) -> Persistent {
    heap.scope(|heap, outer| {
        let held = if nested {
            let held = heap.scope(|heap, inner| {
                heap.alloc_string(inner, &[0x42; 16])
                    .expect("bounded temporary string");
                // A handle made in the inner call for the outer scope
                // must remain a root after the inner scope is gone.
                heap.alloc_string(outer, units)
                    .expect("bounded outer-scope string")
            });
            heap.collect();
            let Kind::String(reference) = heap.get(held).kind() else {
                panic!("outer handle keeps its string");
            };
            assert_eq!(heap.string_units(reference), units);
            held
        } else {
            heap.alloc_string(outer, units).expect("bounded string")
        };
        heap.persist(held)
    })
}

fn run(data: &[u8], stress: bool, mut observe: impl FnMut(&Heap, &Model)) {
    let mut heap = Heap::with_options(Options {
        limit: LIMIT,
        stress,
        verify: true,
    });
    let mut model = Model::new();
    for &[opcode, a, b, c] in data.as_chunks::<4>().0.iter().take(MAX_COMMANDS) {
        let mut collected = false;
        match opcode % 8 {
            0 | 7 => {
                let units = string_units(b, c);
                let handle = new_string(&mut heap, &units, opcode % 8 == 7);
                let node = model.nodes.len();
                model.nodes.push(Node::String(units));
                model.replace(&mut heap, a, Some(Root { node, handle }));
            }
            1 => {
                let length = usize::from(b % 5);
                let handle = heap.scope(|heap, scope| {
                    let local = heap.alloc_slots(scope, length).expect("bounded slot cell");
                    heap.persist(local)
                });
                let node = model.nodes.len();
                model
                    .nodes
                    .push(Node::Slots(vec![ModelValue::Undefined; length]));
                model.replace(&mut heap, a, Some(Root { node, handle }));
            }
            2 | 3 => {
                if let Some(node) = model.root_node(a)
                    && let Node::Slots(slots) = &model.nodes[node]
                {
                    let index = usize::from(b % 5);
                    let (expected, actual) = if opcode % 8 == 2 {
                        (
                            model
                                .root_node(c)
                                .map_or(ModelValue::Undefined, ModelValue::Node),
                            model.value(&heap, c),
                        )
                    } else {
                        let integer = i32::from(i8::from_ne_bytes([c]));
                        (ModelValue::Integer(integer), Value::int32(integer))
                    };
                    let Kind::Internal(target) = model.value(&heap, a).kind() else {
                        panic!("modeled slot cell");
                    };
                    let fits = index < slots.len();
                    assert_eq!(heap.set_slot(target, index, actual), fits);
                    if fits && let Node::Slots(slots) = &mut model.nodes[node] {
                        slots[index] = expected;
                    }
                }
            }
            4 => {
                // Persist the replacement before releasing the old root,
                // including when source and destination are the same slot.
                let replacement = model.root_node(b).map(|node| {
                    let handle = heap.scope(|heap, scope| {
                        let local = heap.local(scope, model.value(heap, b));
                        heap.persist(local)
                    });
                    Root { node, handle }
                });
                model.replace(&mut heap, a, replacement);
            }
            5 => model.replace(&mut heap, a, None),
            6 => {
                heap.collect();
                collected = true;
            }
            _ => unreachable!("opcode modulo eight"),
        }
        assert!(model.nodes.len() <= MAX_COMMANDS);
        model.check(&heap, collected);
        observe(&heap, &model);
    }
    heap.collect();
    model.check(&heap, true);
    observe(&heap, &model);
    for root in &mut model.roots {
        if let Some(root) = root.take() {
            heap.release(root.handle);
        }
    }
    heap.collect();
    model.check(&heap, true);
    assert_eq!(heap.used_bytes(), 0, "released roots leave no cells behind");
    observe(&heap, &model);
}

/// Both collection schedules must satisfy the independent graph model.
pub(super) fn exercise(data: &[u8]) {
    run(data, false, |_, _| {});
    run(data, true, |_, _| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(input: &[[u8; 4]]) -> Vec<u8> {
        input.iter().flatten().copied().collect()
    }

    #[test]
    fn aliases_and_a_cycle_survive_until_the_last_root_is_released() {
        let input = commands(&[
            [1, 0, 2, 0],
            [1, 1, 1, 0],
            [2, 0, 0, 1],
            [2, 1, 0, 0],
            [4, 2, 0, 0],
            [5, 1, 0, 0],
            [6, 0, 0, 0],
            [5, 0, 0, 0],
            [5, 2, 0, 0],
            [6, 0, 0, 0],
        ]);
        let mut states = Vec::new();
        run(&input, false, |heap, model| {
            states.push((heap.used_bytes(), model.roots.iter().flatten().count()));
        });
        assert_eq!(states[6], (40, 2), "alias roots keep both cyclic cells");
        assert_eq!(states[7], (40, 1), "one root still reaches the whole cycle");
        assert_eq!(states[9], (0, 0), "unreachable cycle is collected");
        exercise(&input);
    }

    #[test]
    fn slots_keep_latin1_and_lone_surrogate_strings_without_direct_roots() {
        let input = commands(&[
            [0, 0, 3, b'A'],
            [4, 1, 0, 0],
            [5, 0, 0, 0],
            [6, 0, 0, 0],
            [0, 2, 0xd8, 0],
            [1, 3, 2, 0],
            [2, 3, 0, 1],
            [2, 3, 1, 2],
            [5, 1, 0, 0],
            [5, 2, 0, 0],
            [6, 0, 0, 0],
        ]);
        let mut states = Vec::new();
        run(&input, false, |heap, model| {
            states.push((heap.used_bytes(), model.roots.iter().flatten().count()));
        });
        assert_eq!(states[3], (16, 1), "three Latin-1 units use one data word");
        assert_eq!(
            states[10],
            (72, 1),
            "one slot cell retains both differently encoded strings"
        );
        assert_eq!(string_units(0xd8, 0).first(), Some(&0xd800));
        exercise(&input);
    }

    #[test]
    fn nested_outer_handles_integer_slots_and_reused_root_indices_are_checked() {
        let input = commands(&[
            [7, 0, 3, b'A'],
            [1, 1, 1, 0],
            [3, 1, 0, 0xff],
            [3, 1, 4, 3],
            [4, 2, 2, 0],
            [4, 0, 0, 0],
            [5, 0, 0, 0],
            [0, 0, 2, b'B'],
            [6, 0, 0, 0],
        ]);
        let mut normal_collections = 0;
        let mut stress_collections = 0;
        run(&input, false, |heap, _| {
            normal_collections = heap.collections()
        });
        run(&input, true, |heap, _| {
            stress_collections = heap.collections()
        });
        assert!(stress_collections > normal_collections);
        assert!(
            normal_collections >= 4,
            "nested, explicit and cleanup collections"
        );
        exercise(&input);
    }

    #[test]
    fn empty_partial_and_overlong_command_streams_have_fixed_bounds() {
        for input in [Vec::new(), vec![0; 3], vec![0; 1027]] {
            let mut maximum_nodes = 0;
            run(&input, false, |_, model| {
                maximum_nodes = maximum_nodes.max(model.nodes.len());
            });
            assert!(maximum_nodes <= MAX_COMMANDS);
            if input.len() >= 4 * MAX_COMMANDS {
                assert_eq!(maximum_nodes, MAX_COMMANDS);
            }
            exercise(&input);
        }
    }
}
