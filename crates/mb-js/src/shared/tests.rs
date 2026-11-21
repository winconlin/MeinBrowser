use std::sync::Arc;
use std::time::Duration;

use super::*;

fn block(length: usize) -> SharedBlock {
    SharedBlock::new(length, None).expect("Block")
}

#[test]
fn blocks_start_with_zeros_and_keep_their_length() {
    let fixed = block(13);
    assert_eq!((fixed.len(), fixed.capacity()), (13, 13));
    let mut out = [0xAA; 13];
    assert!(fixed.read(0, &mut out));
    assert_eq!(out, [0; 13]);
    let empty = block(0);
    assert!(empty.read(0, &mut []));
    assert!(!empty.read(0, &mut [0]));
    assert!(SharedBlock::new(9, Some(8)).is_none());
    let growable = SharedBlock::new(3, Some(20)).expect("Block");
    assert_eq!((growable.len(), growable.capacity()), (3, 20));
}

#[test]
fn reads_and_writes_cross_word_boundaries() {
    let shared = block(21);
    let data: Vec<u8> = (1..=13).collect();
    assert!(shared.write(5, &data));
    // Nichts außerhalb des Blocks, und dann auch nichts davor.
    assert!(!shared.write(9, &data));
    assert!(!shared.read(20, &mut [0; 2]));
    assert!(!shared.write(usize::MAX, &[1]));
    assert!(shared.read(21, &mut []));
    let mut out = [0; 21];
    assert!(shared.read(0, &mut out));
    let mut expected = [0u8; 21];
    expected[5..18].copy_from_slice(&data);
    assert_eq!(out, expected);
}

#[test]
fn elements_lie_in_one_word_little_endian() {
    let shared = block(16);
    assert!(shared.write(0, &[1, 2, 3, 4, 5, 6, 7, 8]));
    assert_eq!(shared.load(0, 8), Some(0x0807_0605_0403_0201));
    assert_eq!(shared.load(2, 2), Some(0x0403));
    assert_eq!(shared.load(4, 4), Some(0x0807_0605));
    assert_eq!(shared.load(7, 1), Some(8));
    // Nur ausgerichtet, nur in der Länge und nur mit den Größen der
    // Elemente.
    assert_eq!(shared.load(1, 2), None);
    assert_eq!(shared.load(12, 8), None);
    assert_eq!(shared.load(16, 1), None);
    assert_eq!(shared.load(0, 3), None);
    let set = |bits: u64| move |_| Some(bits);
    assert_eq!(shared.update(10, 2, set(0xFFFF_ABCD)), Some(0));
    let mut out = [0; 4];
    assert!(shared.read(9, &mut out));
    assert_eq!(out, [0, 0xCD, 0xAB, 0]);
    assert_eq!(shared.update(11, 2, set(1)), None);
    assert_eq!(shared.update(8, 8, set(u64::MAX)), Some(0xABCD_0000));
    assert_eq!(shared.load(8, 8), Some(u64::MAX));
    // Die anderen Bytes des Worts bleiben.
    assert_eq!(shared.load(0, 8), Some(0x0807_0605_0403_0201));
}

#[test]
fn updates_return_the_old_value() {
    let shared = block(8);
    assert!(shared.write(4, &[0xFF; 4]));
    let add = |amount: u64| move |old: u64| Some(old.wrapping_add(amount));
    // Die Summe wird auf vier Bytes gekürzt, das Byte danach bleibt 0.
    assert_eq!(shared.update(4, 4, add(2)), Some(0xFFFF_FFFF));
    assert_eq!(shared.load(4, 4), Some(1));
    assert_eq!(shared.load(0, 4), Some(0));
    // Vergleichen und Tauschen.
    let exchange =
        |expected: u64, replacement: u64| move |old: u64| (old == expected).then_some(replacement);
    assert_eq!(shared.update(4, 4, exchange(7, 9)), Some(1));
    assert_eq!(shared.load(4, 4), Some(1));
    assert_eq!(shared.update(4, 4, exchange(1, 9)), Some(1));
    assert_eq!(shared.load(4, 4), Some(9));
    assert_eq!(shared.update(3, 2, add(1)), None);
}

#[test]
fn only_growable_blocks_grow_and_never_shrink() {
    let fixed = block(4);
    assert!(fixed.grow(4));
    assert!(!fixed.grow(5));
    let growable = SharedBlock::new(4, Some(12)).expect("Block");
    assert!(!growable.write(4, &[1]));
    assert!(growable.grow(10));
    assert_eq!(growable.len(), 10);
    assert!(growable.write(9, &[7]));
    assert_eq!(growable.load(9, 1), Some(7));
    assert!(growable.grow(10));
    assert!(!growable.grow(9));
    assert!(!growable.grow(13));
    assert!(growable.grow(12));
    assert_eq!(growable.len(), 12);
}

#[test]
fn copies_overlap_like_memmove() {
    // Mehr als ein Stück des Zwischenpuffers.
    let len = 10_000;
    let data: Vec<u8> = (0..len).map(|index| (index % 251) as u8).collect();
    for (from, to, count) in [
        (0, 3, 9_000),
        (3, 0, 9_000),
        (100, 5_000, 4_100),
        (5, 5, 50),
    ] {
        let shared = block(len);
        assert!(shared.write(0, &data));
        assert!(shared.copy_to(&shared, to, from, count));
        let mut expected = data.clone();
        expected.copy_within(from..from + count, to);
        let mut out = vec![0; len];
        assert!(shared.read(0, &mut out));
        assert_eq!(out, expected, "{from} → {to}");
    }
    let small = block(4);
    let other = block(8);
    assert!(other.write(0, &[1, 2, 3, 4, 5, 6, 7, 8]));
    assert!(other.copy_to(&small, 1, 4, 3));
    assert!(!other.copy_to(&small, 2, 0, 3));
    assert!(!other.copy_to(&small, 0, 6, 3));
    let mut out = [0; 4];
    assert!(small.read(0, &mut out));
    assert_eq!(out, [0, 5, 6, 7]);
}

#[test]
fn parallel_writers_lose_no_bytes_of_a_word() {
    // Zwei Fäden zählen atomar in benachbarten Bytes desselben Worts, ein
    // dritter schreibt nicht atomar in ein anderes Byte. Keiner überschreibt
    // die Bytes der anderen.
    let shared = Arc::new(block(8));
    std::thread::scope(|threads| {
        for index in [0, 1] {
            let shared = Arc::clone(&shared);
            threads.spawn(move || {
                for _ in 0..1000 {
                    shared.update(index, 1, |old| Some(old.wrapping_add(1)));
                }
            });
        }
        let shared = Arc::clone(&shared);
        threads.spawn(move || {
            for round in 0..1000u32 {
                let byte = u8::try_from(round % 200).unwrap_or(0) + 1;
                assert!(shared.write(4, &[byte, byte]));
            }
        });
    });
    // 1000 modulo 256.
    assert_eq!(shared.load(0, 1), Some(232));
    assert_eq!(shared.load(1, 1), Some(232));
    assert_eq!(shared.load(4, 2), Some(0xC8C8));
}

#[test]
fn waiter_lists_keep_their_order() {
    let shared = block(8);
    let mailbox = Arc::new(Mailbox::default());
    let mut lists = shared.critical_section();
    let wake = |ticket| Wake::Async {
        ticket,
        mailbox: Arc::clone(&mailbox),
    };
    let first = lists.add(4, wake(10));
    let second = lists.add(4, wake(11));
    let third = lists.add(4, wake(12));
    let elsewhere = lists.add(0, wake(13));
    assert!(lists.contains(4, second) && !lists.contains(0, second));
    assert!(lists.remove(4, second));
    assert!(!lists.remove(4, second));
    let removed = lists.remove_first(4, u64::MAX);
    let tickets: Vec<u64> = removed
        .iter()
        .map(|waiter| match waiter.wake {
            Wake::Async { ticket, .. } => ticket,
            Wake::Blocking(_) => 0,
        })
        .collect();
    assert_eq!(tickets, [10, 12]);
    assert!(!lists.contains(4, first) && !lists.contains(4, third));
    assert!(lists.remove_first(4, 1).is_empty());
    assert_eq!(lists.remove_first(0, 0).len(), 0);
    assert!(lists.contains(0, elsewhere));
    assert_eq!(lists.remove_first(0, 1).len(), 1);
}

#[test]
fn a_blocked_waiter_wakes_when_it_leaves_the_list() {
    let shared = Arc::new(block(8));
    let condition = Arc::new(Condvar::new());
    let id = shared
        .critical_section()
        .add(0, Wake::Blocking(Arc::clone(&condition)));
    std::thread::scope(|threads| {
        let waiter = {
            let shared = Arc::clone(&shared);
            threads.spawn(move || {
                let mut lists = shared.critical_section();
                // Ohne Wecken wartet der Faden höchstens zehn Sekunden.
                for _ in 0..1000 {
                    if !lists.contains(0, id) {
                        return true;
                    }
                    lists = condition
                        .wait_timeout(lists, Duration::from_millis(10))
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
                false
            })
        };
        let woken = shared.critical_section().remove_first(0, 1);
        for waiter in woken {
            if let Wake::Blocking(condition) = waiter.wake {
                condition.notify_one();
            }
        }
        assert!(waiter.join().expect("Faden"));
    });
}

#[test]
fn mailboxes_hand_over_tickets_once() {
    let mailbox = Mailbox::default();
    assert!(mailbox.is_empty());
    mailbox.push(3);
    mailbox.push(1);
    assert!(!mailbox.is_empty());
    assert_eq!(mailbox.take(), [3, 1]);
    assert!(mailbox.take().is_empty());
}
