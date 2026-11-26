use super::*;
use crate::value::Kind;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn checked() -> Heap {
    Heap::with_options(Options {
        verify: true,
        ..Options::default()
    })
}

fn string(value: Value<'_>) -> Ref<'_> {
    match value.kind() {
        Kind::String(string) => string,
        kind => panic!("keine Zeichenkette: {kind:?}"),
    }
}

fn cell(value: Value<'_>) -> Ref<'_> {
    match value.kind() {
        Kind::Internal(cell) => cell,
        kind => panic!("keine interne Zelle: {kind:?}"),
    }
}

#[test]
fn strings_choose_latin1_or_utf16() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let empty = heap.alloc_string(scope, &[]).expect("Speicher");
        let latin1 = heap
            .alloc_string(scope, &units("Grüße, Welt!"))
            .expect("Speicher");
        let wide = heap
            .alloc_string(scope, &units("€ und 😀"))
            .expect("Speicher");
        let lone = heap
            .alloc_string(scope, &[0x61, 0xD800, 0x62])
            .expect("Speicher");
        heap.collect();
        heap.collect();

        let empty = string(heap.get(empty));
        assert_eq!(heap.string_len(empty), 0);
        assert_eq!(heap.string_unit(empty, 0), None);

        let latin1 = string(heap.get(latin1));
        assert!(heap.is_latin1(latin1));
        assert_eq!(heap.string_units(latin1), units("Grüße, Welt!"));
        assert_eq!(heap.string_unit(latin1, 12), None);

        let wide = string(heap.get(wide));
        assert!(!heap.is_latin1(wide));
        assert_eq!(heap.string_len(wide), 8);
        assert_eq!(heap.string_units(wide), units("€ und 😀"));

        let lone = string(heap.get(lone));
        assert_eq!(heap.string_units(lone), [0x61, 0xD800, 0x62]);
    });
}

#[test]
fn slots_hold_values_and_survive_collection() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let cells = heap.alloc_slots(scope, 4).expect("Speicher");
        let text = heap.alloc_string(scope, &units("Feld")).expect("Speicher");
        {
            let target = cell(heap.get(cells));
            assert_eq!(heap.slot_count(target), 4);
            assert!(heap.slot(target, 0).is_some_and(Value::is_undefined));
            assert!(heap.set_slot(target, 0, Value::number(2.5)));
            assert!(heap.set_slot(target, 1, heap.get(text)));
            assert!(heap.set_slot(target, 2, heap.get(cells)));
            assert!(!heap.set_slot(target, 4, Value::NULL));
            assert_eq!(heap.slot(target, 4).map(Value::kind), None);
        }
        heap.collect();

        let target = cell(heap.get(cells));
        let slot = |index| heap.slot(target, index).map(Value::kind);
        assert_eq!(slot(0), Some(Kind::Number(2.5)));
        let moved = string(heap.slot(target, 1).expect("Feld"));
        assert_eq!(heap.string_units(moved), units("Feld"));
        assert_eq!(moved, string(heap.get(text)));
        // Der Verweis auf sich selbst zeigt auf den neuen Ort.
        assert_eq!(slot(2), Some(Kind::Internal(target)));
        assert_eq!(slot(3), Some(Kind::Undefined));
    });
}

#[test]
fn unreachable_cells_are_freed() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let kept = heap
            .alloc_string(scope, &units("bleibt"))
            .expect("Speicher");
        heap.scope(|heap, scope| {
            for _ in 0..1000 {
                heap.alloc_slots(scope, 8).expect("Speicher");
            }
        });
        let before = heap.used_bytes();
        heap.collect();
        assert!(heap.used_bytes() < before);
        // Kopfwort und ein Wort mit sechs Latin-1-Zeichen.
        assert_eq!(heap.used_bytes(), 16);
        assert_eq!(heap.string_units(string(heap.get(kept))), units("bleibt"));
    });
    heap.collect();
    assert_eq!(heap.used_bytes(), 0);
}

#[test]
fn cycles_survive_while_reachable() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let a = heap.alloc_slots(scope, 1).expect("Speicher");
        let b = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.set_slot(cell(heap.get(a)), 0, heap.get(b));
        heap.set_slot(cell(heap.get(b)), 0, heap.get(a));
        // Nur noch `a` ist ein Handle.
        heap.set(b, Value::UNDEFINED);
        heap.collect();
        let a_cell = cell(heap.get(a));
        let b_cell = cell(heap.slot(a_cell, 0).expect("Feld"));
        assert_eq!(
            heap.slot(b_cell, 0).map(Value::kind),
            Some(Kind::Internal(a_cell))
        );
        assert_eq!(heap.used_bytes(), 32);

        heap.set(a, Value::UNDEFINED);
        heap.collect();
        assert_eq!(heap.used_bytes(), 0);
    });
}

#[test]
fn weak_fields_forget_unreachable_cells() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let weak = heap.alloc_cell(scope, CellKind::Weak, 3).expect("Speicher");
        let kept = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.scope(|heap, scope| {
            let lost = heap.alloc_slots(scope, 1).expect("Speicher");
            let table = cell(heap.get(weak));
            heap.set_field(table, 0, heap.get(kept));
            heap.set_field(table, 1, heap.get(lost));
            heap.set_field(table, 2, Value::int32(7));
        });
        heap.collect();
        let table = cell(heap.get(weak));
        assert_eq!(
            heap.field(table, 0).map(Value::kind),
            Some(Kind::Internal(cell(heap.get(kept))))
        );
        assert!(heap.field(table, 1).is_some_and(Value::is_undefined));
        assert_eq!(heap.field(table, 2).and_then(Value::as_int32), Some(7));
        // Die schwache Zelle mit drei Feldern und die gehaltene mit einem.
        assert_eq!(heap.used_bytes(), 32 + 16);
    });
}

#[test]
fn ephemeron_values_live_only_with_their_keys() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let table = heap.alloc_ephemerons(scope, 3, 3).expect("Speicher");
        let key = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.scope(|heap, scope| {
            let kept = heap.alloc_slots(scope, 1).expect("Speicher");
            let lost = heap.alloc_slots(scope, 1).expect("Speicher");
            let lost_value = heap.alloc_slots(scope, 2).expect("Speicher");
            let cells = cell(heap.get(table));
            heap.set_field(cells, 0, heap.get(key));
            heap.set_field(cells, 1, heap.get(kept));
            heap.set_field(cells, 2, Value::int32(5));
            heap.set_field(cells, 3, heap.get(lost));
            heap.set_field(cells, 4, heap.get(lost_value));
            heap.set_field(cells, 5, Value::int32(6));
            // Ein Eintrag ohne Verweis als Schlüssel hält seine Felder.
            heap.set_field(cells, 6, Value::HOLE);
            heap.set_field(cells, 7, heap.get(key));
        });
        heap.collect();
        let cells = cell(heap.get(table));
        let key = cell(heap.get(key));
        assert_eq!(
            heap.field(cells, 0).map(Value::kind),
            Some(Kind::Internal(key))
        );
        let kept = heap.field(cells, 1).map(Value::kind);
        assert!(matches!(kept, Some(Kind::Internal(value)) if heap.field_count(value) == 1));
        assert_eq!(heap.field(cells, 2).and_then(Value::as_int32), Some(5));
        assert!(heap.field(cells, 3).is_some_and(Value::is_hole));
        assert!(heap.field(cells, 4).is_some_and(Value::is_undefined));
        assert_eq!(heap.field(cells, 5).and_then(Value::as_int32), Some(6));
        assert!(heap.field(cells, 6).is_some_and(Value::is_hole));
        assert_eq!(
            heap.field(cells, 7).map(Value::kind),
            Some(Kind::Internal(key))
        );
        // Die Zelle mit neun Feldern, der Schlüssel und der gehaltene Wert.
        assert_eq!(heap.used_bytes(), 80 + 16 + 16);
    });
}

#[test]
fn ephemeron_chains_are_followed_in_any_order() {
    const LENGTH: usize = 300;
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let table = heap
            .alloc_ephemerons(scope, LENGTH + 2, 2)
            .expect("Speicher");
        let first = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.scope(|heap, scope| {
            let mut keys = vec![first];
            for _ in 1..LENGTH {
                keys.push(heap.alloc_slots(scope, 1).expect("Speicher"));
            }
            // Zwei Schlüssel, die sich gegenseitig als Wert halten, aber
            // sonst niemand.
            let a = heap.alloc_slots(scope, 1).expect("Speicher");
            let b = heap.alloc_slots(scope, 1).expect("Speicher");
            // Jeder Schlüssel hält über seinen Wert den nächsten; die
            // Einträge stehen rückwärts, der erste Schlüssel ganz hinten.
            let cells = cell(heap.get(table));
            for (record, pair) in keys.windows(2).rev().enumerate() {
                heap.set_field(cells, record * 2, heap.get(pair[0]));
                heap.set_field(cells, record * 2 + 1, heap.get(pair[1]));
            }
            let base = (LENGTH - 1) * 2;
            heap.set_field(cells, base, heap.get(a));
            heap.set_field(cells, base + 1, heap.get(b));
            heap.set_field(cells, base + 2, heap.get(b));
            heap.set_field(cells, base + 3, heap.get(a));
            // Der letzte Wert hält den ersten Schlüssel; das ändert nichts.
            let last = cell(heap.get(keys[LENGTH - 1]));
            heap.set_field(last, 0, heap.get(first));
        });
        heap.collect();
        let cells = cell(heap.get(table));
        for record in 0..LENGTH - 1 {
            assert!(
                matches!(
                    heap.field(cells, record * 2).map(Value::kind),
                    Some(Kind::Internal(_))
                ),
                "Eintrag {record}"
            );
        }
        let base = (LENGTH - 1) * 2;
        for field in base..base + 4 {
            assert!(!matches!(
                heap.field(cells, field).map(Value::kind),
                Some(Kind::Internal(_))
            ));
        }
        // Die Zelle und alle Schlüssel der Kette, die beiden anderen nicht.
        let table_bytes = 8 * (1 + (LENGTH + 2) * 2);
        assert_eq!(heap.used_bytes(), table_bytes + LENGTH * 16);
    });
}

#[test]
fn a_value_that_holds_its_own_key_does_not_keep_it() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let table = heap.alloc_ephemerons(scope, 1, 2).expect("Speicher");
        let weak = heap.alloc_cell(scope, CellKind::Weak, 1).expect("Speicher");
        heap.scope(|heap, scope| {
            let key = heap.alloc_slots(scope, 1).expect("Speicher");
            let value = heap.alloc_slots(scope, 1).expect("Speicher");
            heap.set_field(cell(heap.get(value)), 0, heap.get(key));
            let cells = cell(heap.get(table));
            heap.set_field(cells, 0, heap.get(key));
            heap.set_field(cells, 1, heap.get(value));
            // Eine schwache Zelle hält den Wert nicht.
            heap.set_field(cell(heap.get(weak)), 0, heap.get(value));
        });
        heap.collect();
        let cells = cell(heap.get(table));
        assert!(heap.field(cells, 0).is_some_and(Value::is_hole));
        assert!(heap.field(cells, 1).is_some_and(Value::is_undefined));
        assert!(
            heap.field(cell(heap.get(weak)), 0)
                .is_some_and(Value::is_undefined)
        );
        assert_eq!(heap.used_bytes(), 24 + 16);
    });
}

#[test]
fn ephemeron_values_keep_what_weak_cells_name() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let table = heap.alloc_ephemerons(scope, 1, 2).expect("Speicher");
        let weak = heap.alloc_cell(scope, CellKind::Weak, 1).expect("Speicher");
        let key = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.scope(|heap, scope| {
            let value = heap.alloc_slots(scope, 1).expect("Speicher");
            let cells = cell(heap.get(table));
            heap.set_field(cells, 0, heap.get(key));
            heap.set_field(cells, 1, heap.get(value));
            heap.set_field(cell(heap.get(weak)), 0, heap.get(value));
        });
        heap.collect();
        let value = heap.field(cell(heap.get(table)), 1).map(Value::kind);
        assert!(matches!(value, Some(Kind::Internal(_))));
        assert_eq!(heap.field(cell(heap.get(weak)), 0).map(Value::kind), value);
    });
}

#[test]
fn emptied_finalizer_cells_report_their_registry() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let registry = heap.alloc_slots(scope, 1).expect("Speicher");
        let cells = heap.alloc_finalizer_cell(scope, 3).expect("Speicher");
        let token = heap.alloc_slots(scope, 1).expect("Speicher");
        let target = heap.alloc_slots(scope, 1).expect("Speicher");
        {
            let weak = cell(heap.get(cells));
            heap.set_field(weak, 0, heap.get(registry));
            heap.set_field(weak, 1, heap.get(target));
            heap.set_field(weak, 2, heap.get(token));
        }
        // Solange das Ziel lebt, gibt es nichts aufzuräumen.
        heap.collect();
        assert!(heap.pop_cleanup(scope).is_none());
        heap.set(target, Value::UNDEFINED);
        heap.collect();
        let weak = cell(heap.get(cells));
        assert!(heap.field(weak, 1).is_some_and(Value::is_undefined));
        assert_eq!(
            heap.field(weak, 2).map(Value::kind),
            Some(Kind::Internal(cell(heap.get(token))))
        );
        let pending = heap.pop_cleanup(scope).expect("Aufräum-Job");
        assert_eq!(
            heap.get(pending).kind(),
            Kind::Internal(cell(heap.get(registry)))
        );
        assert!(heap.pop_cleanup(scope).is_none());
        // Ein leeres Feld leert das Sammeln nicht noch einmal.
        heap.collect();
        assert!(heap.pop_cleanup(scope).is_none());
    });
}

#[test]
fn pending_registries_stay_alive_until_they_are_taken() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let cells = heap.alloc_finalizer_cell(scope, 2).expect("Speicher");
        heap.scope(|heap, scope| {
            let registry = heap.alloc_slots(scope, 3).expect("Speicher");
            let target = heap.alloc_slots(scope, 1).expect("Speicher");
            let weak = cell(heap.get(cells));
            heap.set_field(weak, 0, heap.get(registry));
            heap.set_field(weak, 1, heap.get(target));
            // Ohne die Registry kein Aufräum-Job: Hier lebt sie, weil der
            // Bereich sie bei der Sammlung hält.
            heap.set(target, Value::UNDEFINED);
            heap.collect();
        });
        // Jetzt hält nur die Liste der Aufräum-Jobs die Registry.
        heap.collect();
        heap.collect();
        let pending = heap.pop_cleanup(scope).expect("Aufräum-Job");
        assert!(matches!(heap.get(pending).kind(), Kind::Internal(registry) if heap.field_count(registry) == 3));
    });
    heap.collect();
    assert_eq!(heap.used_bytes(), 0);
    heap.scope(|heap, scope| {
        let cells = heap.alloc_finalizer_cell(scope, 2).expect("Speicher");
        heap.scope(|heap, scope| {
            let registry = heap.alloc_slots(scope, 3).expect("Speicher");
            let target = heap.alloc_slots(scope, 1).expect("Speicher");
            let weak = cell(heap.get(cells));
            heap.set_field(weak, 0, heap.get(registry));
            heap.set_field(weak, 1, heap.get(target));
        });
        // Registry und Ziel sterben zusammen: kein Aufräum-Job.
        heap.collect();
        assert!(heap.pop_cleanup(scope).is_none());
    });
}

#[test]
fn kept_objects_live_until_they_are_cleared() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let weak = heap.alloc_cell(scope, CellKind::Weak, 1).expect("Speicher");
        heap.scope(|heap, scope| {
            let target = heap.alloc_slots(scope, 1).expect("Speicher");
            heap.set_field(cell(heap.get(weak)), 0, heap.get(target));
            heap.add_to_kept_objects(target);
            heap.add_to_kept_objects(target);
        });
        heap.collect();
        heap.collect();
        let kept = heap
            .field(cell(heap.get(weak)), 0)
            .unwrap_or(Value::UNDEFINED);
        assert!(matches!(kept.kind(), Kind::Internal(_)));
        // Nach dem Verschieben kommt dieselbe Zelle nicht noch einmal dazu.
        let kept = heap.local(scope, kept);
        heap.add_to_kept_objects(kept);
        assert_eq!(heap.kept.len(), 1);
        heap.set(kept, Value::UNDEFINED);
        heap.clear_kept_objects();
        heap.collect();
        assert!(
            heap.field(cell(heap.get(weak)), 0)
                .is_some_and(Value::is_undefined)
        );
        assert_eq!(heap.used_bytes(), 16);
    });
}

#[test]
fn handles_belong_to_the_scope_they_were_made_for() {
    let mut heap = checked();
    heap.scope(|heap, outer| {
        let early = heap.alloc_string(outer, &units("früh")).expect("Speicher");
        let handle = heap.scope(|heap, inner| {
            heap.alloc_string(inner, &units("innen")).expect("Speicher");
            heap.alloc_string(outer, &units("außen")).expect("Speicher")
        });
        // Ein späteres Handle darf den Platz des äußeren nicht übernehmen.
        let late = heap.alloc_string(outer, &units("spät")).expect("Speicher");
        heap.collect();
        assert_eq!(heap.string_units(string(heap.get(handle))), units("außen"));
        assert_eq!(heap.string_units(string(heap.get(early))), units("früh"));
        assert_eq!(heap.string_units(string(heap.get(late))), units("spät"));
        // Nur die Zeichenkette des inneren Bereichs ist frei geworden.
        assert_eq!(heap.used_bytes(), 48);
    });
}

#[test]
#[should_panic(expected = "Bereich eines anderen Heaps")]
fn scopes_of_another_heap_are_rejected() {
    let mut source = Heap::new();
    let destination = Heap::new();
    source.scope(|_, scope| {
        destination.local(scope, Value::NULL);
    });
}

#[test]
#[should_panic(expected = "Handle eines anderen Heaps")]
fn handles_of_another_heap_are_rejected() {
    let mut source = Heap::new();
    let destination = Heap::new();
    source.scope(|source, scope| {
        let text = source
            .alloc_string(scope, &units("fremd"))
            .expect("Speicher");
        destination.get(text);
    });
}

#[test]
#[should_panic(expected = "dauerhaftes Handle eines anderen Heaps")]
fn persistent_handles_of_another_heap_are_rejected() {
    let mut source = Heap::new();
    let destination = Heap::new();
    let handle = source.scope(|source, scope| {
        let text = source
            .alloc_string(scope, &units("fremd"))
            .expect("Speicher");
        source.persist(text)
    });
    destination.get_persistent(&handle);
}

#[test]
fn persistent_handles_outlive_scopes() {
    let mut heap = checked();
    let handle = heap.scope(|heap, scope| {
        let text = heap
            .alloc_string(scope, &units("dauerhaft"))
            .expect("Speicher");
        heap.persist(text)
    });
    heap.collect();
    let text = string(heap.get_persistent(&handle));
    assert_eq!(heap.string_units(text), units("dauerhaft"));
    heap.release(handle);
    heap.collect();
    assert_eq!(heap.used_bytes(), 0);

    // Ein freigegebener Platz wird wiederverwendet.
    let again = heap.scope(|heap, scope| {
        let cells = heap.alloc_slots(scope, 2).expect("Speicher");
        heap.persist(cells)
    });
    assert_eq!(heap.persistent.len(), 1);
    assert_eq!(heap.slot_count(cell(heap.get_persistent(&again))), 2);
}

#[test]
fn large_cells_get_their_own_pages() {
    let mut heap = checked();
    // 300 000 Codeeinheiten in UTF-16 sind etwa 600 KB, mehr als eine Seite.
    let text: Vec<u16> = (0..300_000u32)
        .map(|index| u16::try_from(0x100 + index % 0x7000).unwrap_or(0))
        .collect();
    heap.scope(|heap, scope| {
        let small = heap.alloc_string(scope, &units("klein")).expect("Speicher");
        let large = heap.alloc_string(scope, &text).expect("Speicher");
        let cells = heap.alloc_slots(scope, 70_000).expect("Speicher");
        heap.set_slot(cell(heap.get(cells)), 69_999, heap.get(large));
        heap.set(large, Value::UNDEFINED);
        assert_eq!((heap.young.large.len(), heap.old.large.len()), (2, 0));
        heap.collect();
        assert_eq!((heap.young.large.len(), heap.old.large.len()), (0, 2));
        let cells = cell(heap.get(cells));
        let large = string(heap.slot(cells, 69_999).expect("Feld"));
        assert_eq!(heap.string_units(large), text);
        assert_eq!(heap.string_units(string(heap.get(small))), units("klein"));
    });
}

#[test]
fn long_chains_need_no_deep_stack() {
    const LENGTH: u32 = 100_000;
    let worker = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut heap = checked();
            heap.scope(|heap, scope| {
                let head = heap.alloc_slots(scope, 2).expect("Speicher");
                heap.set_slot(cell(heap.get(head)), 1, Value::int32(0));
                heap.scope(|heap, scope| {
                    let tail = heap.local(scope, heap.get(head));
                    for index in 1..LENGTH {
                        let next = heap.alloc_slots(scope, 2).expect("Speicher");
                        heap.set_slot(cell(heap.get(next)), 1, Value::number(f64::from(index)));
                        heap.set_slot(cell(heap.get(tail)), 0, heap.get(next));
                        heap.set(tail, heap.get(next));
                    }
                });
                heap.collect();
                let mut count = 0;
                let mut current = heap.get(head);
                while let Kind::Internal(link) = current.kind() {
                    let number = heap.slot(link, 1).and_then(Value::as_number);
                    assert_eq!(number, Some(f64::from(count)));
                    count += 1;
                    current = heap.slot(link, 0).unwrap_or(Value::UNDEFINED);
                }
                assert_eq!(count, LENGTH);
            });
        })
        .expect("Faden startet");
    assert!(worker.join().is_ok());
}

#[test]
fn stress_mode_collects_before_every_allocation() {
    let mut heap = Heap::with_options(Options {
        stress: true,
        verify: true,
        ..Options::default()
    });
    heap.scope(|heap, scope| {
        let list = heap.alloc_slots(scope, 100).expect("Speicher");
        for index in 0..100 {
            let text = heap
                .alloc_string(scope, &units(&format!("Eintrag {index}")))
                .expect("Speicher");
            heap.set_slot(cell(heap.get(list)), index, heap.get(text));
        }
        assert_eq!(heap.collections(), 101);
        // Junge und volle Sammlungen wechseln sich ab, die erste ist jung.
        assert_eq!(heap.young_collections(), 51);
        let list = cell(heap.get(list));
        for index in 0..100 {
            let text = string(heap.slot(list, index).expect("Feld"));
            assert_eq!(heap.string_units(text), units(&format!("Eintrag {index}")));
        }
    });
}

#[test]
fn the_limit_holds_and_garbage_makes_room() {
    let mut heap = Heap::with_options(Options {
        limit: 64 * 1024,
        verify: true,
        ..Options::default()
    });
    let filled = heap.scope(|heap, scope| {
        let mut count = 0;
        while heap.alloc_slots(scope, 127).is_ok() {
            count += 1;
        }
        assert_eq!(
            heap.alloc_string(scope, &units("x")).err(),
            Some(OutOfMemory)
        );
        count
    });
    // 64 KiB fassen 64 Zellen zu 1 KiB.
    assert_eq!(filled, 64);
    heap.scope(|heap, scope| {
        heap.alloc_slots(scope, 127)
            .expect("nach der Sammlung ist wieder Platz");
    });
    assert!(heap.verify().is_ok());
}

#[test]
fn the_limit_is_capped() {
    let heap = Heap::with_options(Options {
        limit: usize::MAX,
        ..Options::default()
    });
    assert_eq!(heap.options.limit, MAX_LIMIT);
}

#[test]
fn registers_are_roots() {
    let mut heap = checked();
    let base = heap.push_registers(3).expect("Platz");
    assert_eq!(base, 0);
    heap.scope(|heap, scope| {
        let text = heap
            .alloc_string(scope, &units("im Register"))
            .expect("Speicher");
        heap.set_register(1, heap.get(text));
    });
    heap.collect();
    let text = string(heap.register(1));
    assert_eq!(heap.string_units(text), units("im Register"));
    assert!(heap.register(0).is_undefined());
    // Hinter dem Stapel liest man *undefined*, und Schreiben wirkt nicht.
    heap.set_register(3, Value::TRUE);
    assert!(heap.register(3).is_undefined());

    assert_eq!(heap.push_registers(2), Some(3));
    heap.truncate_registers(base);
    assert_eq!(heap.push_registers(0), Some(0));
    assert_eq!(heap.push_registers(MAX_REGISTERS + 1), None);
    assert_eq!(heap.push_registers(MAX_REGISTERS), Some(0));
    assert_eq!(heap.push_registers(1), None);
    heap.truncate_registers(0);
}

#[test]
fn persistent_handles_can_be_replaced() {
    let mut heap = checked();
    let handle = heap.scope(|heap, scope| {
        let first = heap.alloc_string(scope, &units("eins")).expect("Speicher");
        heap.persist(first)
    });
    heap.scope(|heap, scope| {
        let second = heap.alloc_string(scope, &units("zwei")).expect("Speicher");
        heap.set_persistent(&handle, second);
    });
    heap.collect();
    let text = string(heap.get_persistent(&handle));
    assert_eq!(heap.string_units(text), units("zwei"));
    heap.release(handle);
}

#[test]
fn byte_cells_read_write_and_survive_collection() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let empty = heap.alloc_bytes(scope, 0).expect("Speicher");
        let bytes = heap.alloc_bytes(scope, 21).expect("Speicher");
        // Groß genug für eine eigene Seite.
        let large = heap.alloc_bytes(scope, 300_000).expect("Speicher");
        assert_eq!(heap.byte_len(cell(heap.get(empty))), Some(0));
        let block = cell(heap.get(bytes));
        assert_eq!(heap.byte_len(block), Some(21));
        let mut out = [0xAA; 21];
        assert!(heap.read_bytes(block, 0, &mut out));
        assert_eq!(out, [0; 21]);
        let data: Vec<u8> = (1..=13).collect();
        assert!(heap.write_bytes(block, 5, &data));
        // Nichts außerhalb der Zelle.
        assert!(!heap.write_bytes(block, 9, &data));
        assert!(!heap.read_bytes(block, 20, &mut [0; 2]));
        assert!(heap.read_bytes(block, 21, &mut []));
        assert!(heap.write_bytes(cell(heap.get(large)), 299_999, &[7]));
        heap.collect();
        let block = cell(heap.get(bytes));
        assert!(heap.read_bytes(block, 0, &mut out));
        let mut expected = [0u8; 21];
        expected[5..18].copy_from_slice(&data);
        assert_eq!(out, expected);
        let mut last = [0];
        assert!(heap.read_bytes(cell(heap.get(large)), 299_999, &mut last));
        assert_eq!(last, [7]);
        // Kein Verweis: ein Wort aus Bytes, das wie ein Verweis aussieht,
        // bleibt beim Sammeln unverändert.
        let pattern = Value::reference(RefKind::Object, 8).bits().to_le_bytes();
        assert!(heap.write_bytes(block, 8, &pattern));
        heap.collect();
        let block = cell(heap.get(bytes));
        let mut word = [0; 8];
        assert!(heap.read_bytes(block, 8, &mut word));
        assert_eq!(word, pattern);
    });
}

#[test]
fn byte_copies_overlap_like_memmove() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        // Mehr als ein Stück des Zwischenpuffers.
        let len = 10_000;
        let data: Vec<u8> = (0..len).map(|index| (index % 251) as u8).collect();
        for (from, to, count) in [
            (0, 3, 9_000),
            (3, 0, 9_000),
            (100, 5_000, 4_100),
            (5, 5, 50),
        ] {
            let bytes = heap.alloc_bytes(scope, len).expect("Speicher");
            let block = cell(heap.get(bytes));
            assert!(heap.write_bytes(block, 0, &data));
            assert!(heap.copy_bytes(block, to, block, from, count));
            let mut expected = data.clone();
            expected.copy_within(from..from + count, to);
            let mut out = vec![0; len];
            assert!(heap.read_bytes(block, 0, &mut out));
            assert_eq!(out, expected, "{from} → {to}");
        }
        let small = heap.alloc_bytes(scope, 4).expect("Speicher");
        let other = heap.alloc_bytes(scope, 8).expect("Speicher");
        let (small, other) = (cell(heap.get(small)), cell(heap.get(other)));
        assert!(heap.write_bytes(other, 0, &[1, 2, 3, 4, 5, 6, 7, 8]));
        assert!(heap.copy_bytes(small, 1, other, 4, 3));
        assert!(!heap.copy_bytes(small, 2, other, 0, 3));
        assert!(!heap.copy_bytes(small, 0, other, 6, 3));
        let mut out = [0; 4];
        assert!(heap.read_bytes(small, 0, &mut out));
        assert_eq!(out, [0, 5, 6, 7]);
    });
}

#[test]
fn byte_cells_count_against_the_limit() {
    let mut heap = Heap::with_options(Options {
        limit: MIN_TRIGGER,
        ..Options::default()
    });
    assert!(heap.bytes_fit(1 << 20));
    assert!(!heap.bytes_fit(u64::try_from(MIN_TRIGGER).expect("Größe")));
    assert!(!heap.bytes_fit(u64::MAX));
    heap.scope(|heap, scope| {
        let first = heap.alloc_bytes(scope, 3 << 20).expect("Speicher");
        assert!(heap.alloc_bytes(scope, 2 << 20).is_err());
        heap.set(first, Value::UNDEFINED);
        assert!(heap.alloc_bytes(scope, 2 << 20).is_ok());
    });
}

#[test]
fn shared_blocks_live_while_a_cell_names_them() {
    let mut heap = checked();
    let block = heap.scope(|heap, scope| {
        let first = heap.alloc_shared(scope, 100, None).expect("Speicher");
        let second = heap.alloc_shared(scope, 10, Some(1000)).expect("Speicher");
        assert_eq!(heap.external_bytes(), 1100);
        let block = std::sync::Arc::clone(heap.shared_block(cell(heap.get(first))).expect("Block"));
        assert_eq!((block.len(), block.capacity()), (100, 100));
        assert!(block.write(99, &[7]));
        heap.set(second, Value::UNDEFINED);
        heap.collect();
        // Die Zelle ist verschoben und nennt noch denselben Block.
        assert_eq!(heap.external_bytes(), 100);
        let moved = heap.shared_block(cell(heap.get(first))).expect("Block");
        assert!(std::sync::Arc::ptr_eq(moved, &block));
        // Nur Zellen für geteilte Blöcke haben einen.
        let bytes = heap.alloc_bytes(scope, 8).expect("Speicher");
        assert!(heap.shared_block(cell(heap.get(bytes))).is_none());
        block
    });
    heap.collect();
    assert_eq!(heap.external_bytes(), 0);
    // Wer den Block sonst hält, behält ihn, etwa ein anderer Agent.
    let mut out = [0];
    assert!(block.read(99, &mut out));
    assert_eq!(out, [7]);
    heap.scope(|heap, scope| {
        let adopted = heap
            .adopt_shared(scope, std::sync::Arc::clone(&block))
            .expect("Speicher");
        assert_eq!(heap.external_bytes(), 100);
        heap.collect();
        let moved = heap.shared_block(cell(heap.get(adopted))).expect("Block");
        assert!(std::sync::Arc::ptr_eq(moved, &block));
    });
    heap.collect();
    assert_eq!(heap.external_bytes(), 0);
}

#[test]
fn shared_blocks_survive_stress_collections() {
    let mut heap = Heap::with_options(Options {
        stress: true,
        verify: true,
        ..Options::default()
    });
    heap.scope(|heap, scope| {
        let cells: Vec<_> = (0..20)
            .map(|index| heap.alloc_shared(scope, index, Some(32)).expect("Speicher"))
            .collect();
        heap.alloc_slots(scope, 4).expect("Speicher");
        for (index, local) in cells.iter().enumerate() {
            let block = heap.shared_block(cell(heap.get(*local))).expect("Block");
            assert_eq!(block.len(), index);
        }
        assert_eq!(heap.external_bytes(), 20 * 32);
    });
}

#[test]
fn shared_blocks_count_against_the_limit() {
    let mut heap = Heap::with_options(Options {
        limit: MIN_TRIGGER,
        ..Options::default()
    });
    heap.scope(|heap, scope| {
        let first = heap.alloc_shared(scope, 3 << 20, None).expect("Speicher");
        assert!(heap.alloc_shared(scope, 2 << 20, None).is_err());
        assert!(heap.alloc_bytes(scope, 2 << 20).is_err());
        // Ein wachsender Block zählt mit seinem Maximum.
        assert!(heap.alloc_shared(scope, 0, Some(2 << 20)).is_err());
        heap.set(first, Value::UNDEFINED);
        assert!(heap.alloc_shared(scope, 2 << 20, None).is_ok());
    });
}

/// Der Abstand der Zelle hinter einem Handle.
fn offset_of(heap: &Heap, local: Local<'_>) -> u32 {
    heap.get(local)
        .reference_parts()
        .map(|(_, offset)| offset)
        .expect("Verweis")
}

#[test]
fn young_collections_promote_survivors_and_leave_old_cells_in_place() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let old = heap.alloc_slots(scope, 2).expect("Speicher");
        heap.collect();
        let old_offset = offset_of(heap, old);
        assert_eq!(heap.young_bytes(), 0);
        let young = heap.alloc_string(scope, &units("jung")).expect("Speicher");
        heap.scope(|heap, scope| {
            for _ in 0..100 {
                heap.alloc_slots(scope, 8).expect("Speicher");
            }
        });
        let young_offset = offset_of(heap, young);
        assert!(heap.young_bytes() > 16 * 100);
        heap.collect_young();
        assert_eq!((heap.collections(), heap.young_collections()), (2, 1));
        // Die alte Zelle bleibt, wo sie war, die junge zieht in eine alte
        // Seite um, der Müll ist fort.
        assert_eq!(offset_of(heap, old), old_offset);
        assert_ne!(offset_of(heap, young), young_offset);
        assert_eq!(heap.young_bytes(), 0);
        assert_eq!(heap.used_bytes(), 24 + 16);
        assert_eq!(heap.string_units(string(heap.get(young))), units("jung"));
        assert!(heap.young.small.is_empty() && heap.young.large.is_empty());
    });
}

#[test]
fn the_barrier_remembers_old_cells_that_name_young_ones() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let old = heap.alloc_slots(scope, 3).expect("Speicher");
        let other = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.collect();
        // Ein Verweis auf eine alte Zelle braucht keinen Eintrag.
        heap.set_slot(cell(heap.get(old)), 2, heap.get(other));
        assert!(heap.remembered.borrow().is_empty());
        heap.scope(|heap, scope| {
            let first = heap.alloc_string(scope, &units("eins")).expect("Speicher");
            let second = heap.alloc_string(scope, &units("zwei")).expect("Speicher");
            let target = cell(heap.get(old));
            heap.set_slot(target, 0, heap.get(first));
            heap.set_slot(target, 1, heap.get(second));
            // Eine junge Zelle, die auf eine junge zeigt, merkt sich niemand.
            let young = heap.alloc_slots(scope, 1).expect("Speicher");
            heap.set_slot(cell(heap.get(young)), 0, heap.get(first));
        });
        // Jede alte Zelle steht höchstens einmal in der Menge.
        assert_eq!(
            heap.remembered.borrow().iter().copied().collect::<Vec<_>>(),
            [offset_of(heap, old)]
        );
        heap.collect_young();
        assert!(heap.remembered.borrow().is_empty());
        let target = cell(heap.get(old));
        let text = |index| heap.string_units(string(heap.slot(target, index).expect("Feld")));
        assert_eq!(text(0), units("eins"));
        assert_eq!(text(1), units("zwei"));
        // Die alte Zelle, die andere und die beiden Zeichenketten.
        assert_eq!(heap.used_bytes(), 32 + 16 + 16 + 16);
    });
}

#[test]
fn packed_references_pass_the_barrier() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let shape = heap
            .alloc_cell(scope, CellKind::Shape, 1)
            .expect("Speicher");
        let object = heap
            .alloc_object(scope, ObjectClass::Ordinary, 1, shape, None)
            .expect("Speicher");
        heap.collect();
        heap.scope(|heap, scope| {
            let shape = heap
                .alloc_cell(scope, CellKind::Shape, 2)
                .expect("Speicher");
            let first = heap
                .alloc_cell(scope, CellKind::Shape, 1)
                .expect("Speicher");
            let prototype = heap
                .alloc_object(scope, ObjectClass::Ordinary, 0, first, None)
                .expect("Speicher");
            let elements = heap.alloc_slots(scope, 5).expect("Speicher");
            let overflow = heap.alloc_slots(scope, 6).expect("Speicher");
            let target = Ref::new(offset_of(heap, object));
            assert!(heap.set_object_shape(target, cell(heap.get(shape))));
            assert!(heap.set_object_prototype(target, Some(Ref::new(offset_of(heap, prototype)))));
            assert!(heap.set_object_elements(target, Some(cell(heap.get(elements)))));
            assert!(heap.set_object_overflow(target, Some(cell(heap.get(overflow)))));
        });
        assert_eq!(heap.remembered.borrow().len(), 1);
        heap.collect_young();
        let target = Ref::new(offset_of(heap, object));
        let shape = heap.object_shape(target).expect("Gestalt");
        assert_eq!(heap.field_count(shape), 2);
        let prototype = heap.object_prototype(target).expect("Prototyp");
        assert_eq!(heap.object_class(prototype), Some(ObjectClass::Ordinary));
        let elements = heap.object_elements(target).expect("Elemente");
        assert_eq!(heap.slot_count(elements), 5);
        let overflow = heap.object_overflow(target).expect("Zusatzfeld");
        assert_eq!(heap.slot_count(overflow), 6);
    });
}

#[test]
fn old_weak_cells_forget_young_cells_that_die() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let weak = heap.alloc_cell(scope, CellKind::Weak, 3).expect("Speicher");
        let old = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.collect();
        let kept = heap.alloc_slots(scope, 2).expect("Speicher");
        heap.scope(|heap, scope| {
            let lost = heap.alloc_slots(scope, 1).expect("Speicher");
            let table = cell(heap.get(weak));
            heap.set_field(table, 0, heap.get(kept));
            heap.set_field(table, 1, heap.get(lost));
            heap.set_field(table, 2, heap.get(old));
        });
        heap.collect_young();
        let table = cell(heap.get(weak));
        assert_eq!(
            heap.field(table, 0).map(Value::kind),
            Some(Kind::Internal(cell(heap.get(kept))))
        );
        assert!(heap.field(table, 1).is_some_and(Value::is_undefined));
        assert_eq!(
            heap.field(table, 2).map(Value::kind),
            Some(Kind::Internal(cell(heap.get(old))))
        );
        assert_eq!(heap.used_bytes(), 32 + 16 + 24);
    });
}

#[test]
fn old_finalizer_cells_report_young_targets_that_die() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let registry = heap.alloc_slots(scope, 1).expect("Speicher");
        let cells = heap.alloc_finalizer_cell(scope, 2).expect("Speicher");
        heap.set_field(cell(heap.get(cells)), 0, heap.get(registry));
        heap.collect();
        heap.scope(|heap, scope| {
            let target = heap.alloc_slots(scope, 1).expect("Speicher");
            heap.set_field(cell(heap.get(cells)), 1, heap.get(target));
            heap.collect_young();
            assert!(heap.pop_cleanup(scope).is_none());
        });
        // Das Ziel ist jetzt alt; eine junge Sammlung sieht seinen Tod nicht.
        heap.collect_young();
        assert!(heap.pop_cleanup(scope).is_none());
        heap.collect();
        assert!(heap.pop_cleanup(scope).is_some());
        heap.scope(|heap, scope| {
            let target = heap.alloc_slots(scope, 1).expect("Speicher");
            heap.set_field(cell(heap.get(cells)), 1, heap.get(target));
        });
        heap.collect_young();
        assert!(
            heap.field(cell(heap.get(cells)), 1)
                .is_some_and(Value::is_undefined)
        );
        let pending = heap.pop_cleanup(scope).expect("Aufräum-Job");
        assert_eq!(
            heap.get(pending).kind(),
            Kind::Internal(cell(heap.get(registry)))
        );
    });
}

#[test]
fn old_ephemerons_follow_young_keys_and_values() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let table = heap.alloc_ephemerons(scope, 4, 2).expect("Speicher");
        let old_key = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.collect();
        let young_key = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.scope(|heap, scope| {
            let lost_key = heap.alloc_slots(scope, 1).expect("Speicher");
            let values: Vec<_> = (0..4)
                .map(|_| heap.alloc_slots(scope, 2).expect("Speicher"))
                .collect();
            let chained = heap.alloc_slots(scope, 1).expect("Speicher");
            let cells = cell(heap.get(table));
            // Ein alter Schlüssel lebt in einer jungen Sammlung immer.
            heap.set_field(cells, 0, heap.get(old_key));
            heap.set_field(cells, 1, heap.get(values[0]));
            heap.set_field(cells, 2, heap.get(young_key));
            heap.set_field(cells, 3, heap.get(values[1]));
            heap.set_field(cells, 4, heap.get(lost_key));
            heap.set_field(cells, 5, heap.get(values[2]));
            // Der Wert eines lebenden Eintrags hält den Schlüssel eines
            // anderen, der weiter hinten steht.
            heap.set_field(cells, 6, heap.get(chained));
            heap.set_field(cells, 7, heap.get(values[3]));
            heap.set_field(cell(heap.get(values[1])), 0, heap.get(chained));
        });
        heap.collect_young();
        let cells = cell(heap.get(table));
        let internal = |index| {
            matches!(
                heap.field(cells, index).map(Value::kind),
                Some(Kind::Internal(_))
            )
        };
        assert!((0..4).all(internal));
        assert!(heap.field(cells, 4).is_some_and(Value::is_hole));
        assert!(heap.field(cells, 5).is_some_and(Value::is_undefined));
        assert!(internal(6) && internal(7));
        // Tabelle, alter und junger Schlüssel, drei Werte, der verkettete
        // Schlüssel.
        assert_eq!(heap.used_bytes(), 72 + 16 + 16 + 3 * 24 + 16);
    });
}

#[test]
fn young_collections_free_only_young_shared_blocks() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let old = heap.alloc_shared(scope, 10, None).expect("Speicher");
        heap.collect();
        let kept = heap.alloc_shared(scope, 20, None).expect("Speicher");
        heap.scope(|heap, scope| {
            heap.alloc_shared(scope, 40, None).expect("Speicher");
        });
        assert_eq!(heap.external_bytes(), 70);
        heap.collect_young();
        assert_eq!(heap.external_bytes(), 30);
        assert_eq!(
            heap.shared_block(cell(heap.get(old)))
                .map(|block| block.len()),
            Some(10)
        );
        assert_eq!(
            heap.shared_block(cell(heap.get(kept)))
                .map(|block| block.len()),
            Some(20)
        );
        // Der umgezogene Eintrag ist jetzt alt, eine junge Sammlung lässt
        // ihn stehen, auch ohne Handle; erst die volle gibt ihn frei.
        heap.set(kept, Value::UNDEFINED);
        heap.collect_young();
        assert_eq!(heap.external_bytes(), 30);
        heap.collect();
        assert_eq!(heap.external_bytes(), 10);
    });
}

#[test]
fn a_full_nursery_starts_a_young_collection() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let kept = heap
            .alloc_string(scope, &units("bleibt"))
            .expect("Speicher");
        // Etwa 3 MiB Müll in Zellen zu 72 Bytes.
        for _ in 0..45_000 {
            heap.scope(|heap, scope| {
                heap.alloc_slots(scope, 8).expect("Speicher");
            });
        }
        assert_eq!((heap.collections(), heap.young_collections()), (1, 1));
        assert!(heap.young_bytes() <= NURSERY);
        heap.collect_young();
        assert_eq!(heap.used_bytes(), 16);
        assert_eq!(heap.string_units(string(heap.get(kept))), units("bleibt"));
    });
}

#[test]
fn verify_finds_a_store_past_the_barrier() {
    let mut heap = checked();
    heap.scope(|heap, scope| {
        let old = heap.alloc_slots(scope, 1).expect("Speicher");
        heap.collect();
        let young = heap.alloc_slots(scope, 1).expect("Speicher");
        let offset = offset_of(heap, old);
        // Ein Schreiben ohne Barriere, wie es kein Aufrufer tun darf.
        assert!(heap.write(offset, 1, heap.get(young).bits()));
        let error = heap.verify().expect_err("Barriere fehlt");
        assert!(error.contains("gemerkten Menge"), "{error}");
        heap.set_slot(cell(heap.get(old)), 0, heap.get(young));
        assert_eq!(heap.verify(), Ok(()));
    });
}
