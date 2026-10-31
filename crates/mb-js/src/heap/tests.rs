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
        assert_eq!(heap.large.len(), 2);
        heap.collect();
        assert_eq!(heap.large.len(), 2);
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
