//! Grenzen und Treffer des Ergebnis-Caches für die Ersatzschriftsuche.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::super::{FaceId, Family, FontQuery, GenericFamily};
use super::{FallbackCache, FallbackKey};
use crate::matching::Slope;

fn query() -> FontQuery<'static> {
    FontQuery {
        families: &[],
        weight: 400.0,
        width: 100.0,
        slope: Slope::Normal,
    }
}

fn key(characters: &[char]) -> FallbackKey {
    FallbackKey::new(&query(), characters).expect("kleine Anfrage")
}

fn assert_bounds(cache: &FallbackCache) {
    assert!(cache.entries.len() <= cache.capacity);
    assert_eq!(cache.order.len(), cache.entries.len());
    let ordered: BTreeSet<&FallbackKey> = cache.order.iter().map(Arc::as_ref).collect();
    assert_eq!(ordered.len(), cache.entries.len());
    for stored in cache.entries.keys() {
        // Map und Reihenfolge dürfen die Zeichenfolge nicht doppelt halten.
        assert!(cache.order.iter().any(|item| Arc::ptr_eq(item, stored)));
    }
}

#[test]
fn a_hit_keeps_the_recently_used_result() {
    let mut cache = FallbackCache::with_capacity(2);
    cache.insert(key(&['a']), Some(FaceId(1)));
    cache.insert(key(&['b']), Some(FaceId(2)));
    assert_eq!(cache.get(&key(&['a'])), Some(Some(FaceId(1))));
    cache.insert(key(&['c']), Some(FaceId(3)));
    assert_eq!(cache.get(&key(&['b'])), None);
    assert_eq!(cache.get(&key(&['a'])), Some(Some(FaceId(1))));
    assert_eq!(cache.get(&key(&['c'])), Some(Some(FaceId(3))));
    assert_bounds(&cache);
}

#[test]
fn a_missing_font_is_a_cached_hit_and_refreshes_recency() {
    let mut cache = FallbackCache::with_capacity(2);
    cache.insert(key(&['a']), None);
    cache.insert(key(&['b']), Some(FaceId(1)));
    assert_eq!(cache.get(&key(&['a'])), Some(None));
    cache.insert(key(&['c']), None);
    assert_eq!(cache.get(&key(&['b'])), None);
    assert_eq!(cache.get(&key(&['a'])), Some(None));
    assert_bounds(&cache);
}

#[test]
fn updating_a_result_does_not_duplicate_its_key_or_lru_slot() {
    let mut cache = FallbackCache::with_capacity(2);
    cache.insert(key(&['a']), Some(FaceId(1)));
    cache.insert(key(&['b']), Some(FaceId(2)));
    cache.insert(key(&['a']), None);
    assert_eq!(cache.entries.len(), 2);
    assert_eq!(cache.get(&key(&['a'])), Some(None));
    assert_bounds(&cache);

    // Das Ersetzen selbst zählt als Nutzung, auch ohne weiteren Treffer.
    cache.insert(key(&['b']), Some(FaceId(3)));
    cache.insert(key(&['c']), Some(FaceId(4)));
    assert_eq!(cache.get(&key(&['a'])), None);
    assert_eq!(cache.get(&key(&['b'])), Some(Some(FaceId(3))));
    assert_bounds(&cache);
}

#[test]
fn clearing_the_cache_removes_positive_and_negative_results() {
    let mut cache = FallbackCache::with_capacity(2);
    cache.insert(key(&['a']), Some(FaceId(1)));
    cache.insert(key(&['b']), None);
    cache.clear();
    assert!(cache.entries.is_empty());
    assert!(cache.order.is_empty());
    assert_eq!(cache.get(&key(&['a'])), None);
    assert_eq!(cache.get(&key(&['b'])), None);
    cache.insert(key(&['c']), Some(FaceId(2)));
    assert_eq!(cache.get(&key(&['c'])), Some(Some(FaceId(2))));
    assert_bounds(&cache);
}

#[test]
fn zero_capacity_keeps_no_results() {
    let mut cache = FallbackCache::with_capacity(0);
    cache.insert(key(&['a']), Some(FaceId(1)));
    cache.insert(key(&['b']), None);
    assert_eq!(cache.get(&key(&['a'])), None);
    assert_eq!(cache.get(&key(&['b'])), None);
    assert_bounds(&cache);
}

#[test]
fn many_distinct_requests_stay_bounded() {
    let mut cache = FallbackCache::with_capacity(17);
    for index in 0..1024 {
        let character = char::from_u32(0x100 + index).expect("Skalarwert");
        cache.insert(key(&[character]), Some(FaceId(index as usize)));
        assert_eq!(
            cache.get(&key(&[character])),
            Some(Some(FaceId(index as usize)))
        );
        assert_bounds(&cache);
    }
    assert_eq!(cache.get(&key(&['\u{100}'])), None);

    let mut default = FallbackCache::default();
    assert_eq!(default.capacity, 256);
    for index in 0..1024 {
        let character = char::from_u32(0x100 + index).expect("Skalarwert");
        default.insert(key(&[character]), None);
        assert_bounds(&default);
    }
}

fn variant(index: usize) -> FallbackKey {
    let mut request = query();
    let characters = match index {
        0 => vec!['a'],
        1 => {
            request.weight = 700.0;
            vec!['a']
        }
        2 => {
            request.width = 80.0;
            vec!['a']
        }
        3 => {
            request.slope = Slope::Italic;
            vec!['a']
        }
        4 => {
            request.slope = Slope::Oblique(14.0);
            vec!['a']
        }
        5 => {
            request.slope = Slope::Oblique(20.0);
            vec!['a']
        }
        6 => vec!['b'],
        7 => vec!['a', '\u{301}'],
        8 => vec!['\u{301}', 'a'],
        9 => vec!['a', 'a'],
        _ => unreachable!(),
    };
    FallbackKey::new(&request, &characters).expect("kleine Anfrage")
}

#[test]
fn style_and_character_sequences_remain_distinct() {
    let mut cache = FallbackCache::with_capacity(10);
    for index in 0..10 {
        cache.insert(variant(index), Some(FaceId(index)));
    }
    for index in 0..10 {
        assert_eq!(cache.get(&variant(index)), Some(Some(FaceId(index))));
    }
    assert_bounds(&cache);
}

#[test]
fn query_family_lists_do_not_change_the_installed_font_fallback_key() {
    let names = [Family::Name("explicit family".to_owned())];
    let generic = [Family::Generic(GenericFamily::Monospace)];
    let first = FontQuery {
        families: &names,
        ..query()
    };
    let second = FontQuery {
        families: &generic,
        ..query()
    };
    let mut cache = FallbackCache::with_capacity(1);
    cache.insert(
        FallbackKey::new(&first, &['a']).expect("kleine Anfrage"),
        Some(FaceId(3)),
    );
    let key = FallbackKey::new(&second, &['a']).expect("kleine Anfrage");
    assert_eq!(cache.get(&key), Some(Some(FaceId(3))));
    assert_bounds(&cache);
}

#[test]
fn empty_or_overlong_sequences_are_not_retained() {
    assert!(FallbackKey::new(&query(), &[]).is_none());
    let mut characters = vec!['🧭'; 64];
    assert!(FallbackKey::new(&query(), &characters).is_some());
    characters.push('\u{301}');
    assert!(FallbackKey::new(&query(), &characters).is_none());
    let mut cache = FallbackCache::with_capacity(1);
    cache.insert(key(&['a']), Some(FaceId(1)));
    // Eine nicht speicherbare Anfrage erreicht insert gar nicht und darf
    // somit auch keine bereits brauchbaren Treffer verdrängen.
    if let Some(key) = FallbackKey::new(&query(), &characters) {
        cache.insert(key, None);
    }
    assert_eq!(cache.get(&key(&['a'])), Some(Some(FaceId(1))));
    assert_bounds(&cache);
}
