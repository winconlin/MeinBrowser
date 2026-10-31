//! Verdrängung und Speichergrenzen des Schriftmaß-Caches.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::super::{FamilyKey, Metrics, QueryKey};
use super::{MetricsCache, key_bytes};

fn key(name: &str) -> QueryKey {
    QueryKey {
        families: vec![FamilyKey::Name(name.to_owned())],
        weight: u64::from(400.0_f32.to_bits()),
        width: u64::from(100.0_f32.to_bits()),
        slope: (0, 0),
    }
}

fn metrics(ascent: f64) -> Metrics {
    Metrics {
        ascent,
        descent: 0.2,
        line_gap: 0.1,
        x_height: Some(0.5),
        cap_height: Some(0.7),
        zero: Some(0.6),
        water: None,
    }
}

fn assert_bounds(cache: &MetricsCache) {
    assert!(cache.entries.len() <= cache.max_entries);
    assert!(cache.bytes <= cache.max_bytes);
    assert_eq!(cache.order.len(), cache.entries.len());
    assert_eq!(
        cache
            .entries
            .values()
            .map(|entry| entry.bytes)
            .sum::<usize>(),
        cache.bytes
    );
    let ordered: BTreeSet<&QueryKey> = cache.order.iter().map(Arc::as_ref).collect();
    assert_eq!(ordered.len(), cache.entries.len());
    for stored in cache.entries.keys() {
        // Beide Verwaltungsstrukturen teilen die gehaltenen Zeichenketten.
        assert!(cache.order.iter().any(|item| Arc::ptr_eq(item, stored)));
    }
}

#[test]
fn a_hit_keeps_the_recently_used_entry_when_the_count_limit_is_reached() {
    let mut cache = MetricsCache::with_limits(2, usize::MAX);
    cache.insert(key("first"), Some(metrics(0.8)));
    cache.insert(key("second"), Some(metrics(0.9)));
    // Ein neu aufgebauter, gleicher Schlüssel muss den Eintrag finden.
    assert_eq!(cache.get(&key("first")), Some(Some(metrics(0.8))));
    cache.insert(key("third"), Some(metrics(1.0)));
    assert_eq!(cache.get(&key("second")), None);
    assert_eq!(cache.get(&key("first")), Some(Some(metrics(0.8))));
    assert_eq!(cache.get(&key("third")), Some(Some(metrics(1.0))));
    assert_bounds(&cache);
}

#[test]
fn a_cached_missing_font_is_a_hit_and_refreshes_recency() {
    let mut cache = MetricsCache::with_limits(2, usize::MAX);
    cache.insert(key("missing"), None);
    cache.insert(key("other"), Some(metrics(0.8)));
    assert_eq!(cache.get(&key("missing")), Some(None));
    cache.insert(key("new"), Some(metrics(0.9)));
    assert_eq!(cache.get(&key("other")), None);
    assert_eq!(cache.get(&key("missing")), Some(None));
    assert_bounds(&cache);
}

#[test]
fn the_byte_limit_evicts_before_the_entry_count_limit() {
    let first = key("first");
    let second = key("other");
    let first_bytes = key_bytes(&first).expect("zählbar");
    let second_bytes = key_bytes(&second).expect("zählbar");
    let mut cache = MetricsCache::with_limits(8, first_bytes + second_bytes - 1);
    cache.insert(first, Some(metrics(0.8)));
    cache.insert(second, Some(metrics(0.9)));
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(cache.get(&key("first")), None);
    assert_eq!(cache.get(&key("other")), Some(Some(metrics(0.9))));
    assert_bounds(&cache);
}

#[test]
fn an_entry_fits_at_its_exact_byte_limit() {
    let request = key("exact");
    let bytes = key_bytes(&request).expect("zählbar");
    let mut exact = MetricsCache::with_limits(1, bytes);
    exact.insert(request.clone(), Some(metrics(0.8)));
    assert_eq!(exact.get(&request), Some(Some(metrics(0.8))));
    assert_eq!(exact.bytes, bytes);
    assert_bounds(&exact);

    let mut too_small = MetricsCache::with_limits(1, bytes - 1);
    too_small.insert(request.clone(), Some(metrics(0.8)));
    assert_eq!(too_small.get(&request), None);
    assert_bounds(&too_small);
}

#[test]
fn oversized_names_and_family_lists_do_not_evict_useful_entries() {
    let mut cache = MetricsCache::with_limits(4, 1024);
    cache.insert(key("useful"), Some(metrics(0.8)));
    let retained_bytes = cache.bytes;
    cache.insert(key(&"x".repeat(4096)), Some(metrics(0.9)));
    let mut many_families = key("many");
    many_families.families = vec![FamilyKey::Generic(0); 1024];
    cache.insert(many_families, None);
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(cache.bytes, retained_bytes);
    assert_eq!(cache.get(&key("useful")), Some(Some(metrics(0.8))));
    assert_bounds(&cache);
}

#[test]
fn reserved_string_and_vector_capacity_counts_toward_the_byte_limit() {
    let mut cache = MetricsCache::with_limits(4, 1024);
    cache.insert(key("useful"), Some(metrics(0.8)));
    let mut large_name = String::with_capacity(4096);
    large_name.push_str("short");
    let mut request = key("short");
    request.families = vec![FamilyKey::Name(large_name)];
    cache.insert(request, None);
    assert_eq!(cache.get(&key("short")), None);

    let mut large_list = Vec::with_capacity(1024);
    large_list.push(FamilyKey::Name("small".to_owned()));
    let mut request = key("small");
    request.families = large_list;
    cache.insert(request, None);
    assert_eq!(cache.get(&key("small")), None);
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(cache.get(&key("useful")), Some(Some(metrics(0.8))));
    assert_bounds(&cache);
}

#[test]
fn updating_an_existing_entry_neither_duplicates_nor_recharges_its_key() {
    let mut cache = MetricsCache::with_limits(2, 1024);
    cache.insert(key("first"), Some(metrics(0.8)));
    cache.insert(key("other"), Some(metrics(0.9)));
    let retained_bytes = cache.bytes;
    let mut large_replacement = String::with_capacity(4096);
    large_replacement.push_str("first");
    let mut replacement = key("first");
    replacement.families = vec![FamilyKey::Name(large_replacement)];
    cache.insert(replacement, Some(metrics(1.0)));
    assert_eq!(cache.bytes, retained_bytes);
    assert_eq!(cache.entries.len(), 2);
    assert_bounds(&cache);

    // Auch eine Aktualisierung zählt als Nutzung des Eintrags.
    cache.insert(key("third"), None);
    assert_eq!(cache.get(&key("other")), None);
    assert_eq!(cache.get(&key("first")), Some(Some(metrics(1.0))));
    assert_bounds(&cache);
}

#[test]
fn zero_entry_or_byte_limits_disable_retention() {
    for (entries, bytes) in [(0, usize::MAX), (4, 0), (0, 0)] {
        let mut cache = MetricsCache::with_limits(entries, bytes);
        cache.insert(key("present"), Some(metrics(0.8)));
        cache.insert(key("missing"), None);
        assert_eq!(cache.get(&key("present")), None);
        assert_eq!(cache.get(&key("missing")), None);
        assert_eq!(cache.bytes, 0);
        assert_bounds(&cache);
    }
}

#[test]
fn many_distinct_requests_stay_within_both_limits() {
    let mut cache = MetricsCache::with_limits(17, 8 << 10);
    for index in 0..1024 {
        let name = format!("family-{index}-{}", "x".repeat(index % 600));
        let request = key(&name);
        cache.insert(request.clone(), Some(metrics(0.8)));
        assert_eq!(cache.get(&request), Some(Some(metrics(0.8))));
        assert_bounds(&cache);
    }
    assert_eq!(cache.get(&key("family-0-")), None);

    let mut default = MetricsCache::new();
    assert_eq!((default.max_entries, default.max_bytes), (256, 256 << 10));
    for index in 0..1024 {
        default.insert(key(&format!("family-{index}")), None);
        assert_bounds(&default);
    }
}

#[test]
fn family_weight_width_and_slope_remain_distinct() {
    let base = key("same");
    let mut weight = base.clone();
    weight.weight = u64::from(700.0_f32.to_bits());
    let mut width = base.clone();
    width.width = u64::from(80.0_f32.to_bits());
    let mut slope = base.clone();
    slope.slope = (2, u64::from(14.0_f32.to_bits()));
    let mut generic = base.clone();
    generic.families = vec![FamilyKey::Generic(0)];
    let mut ordered = base.clone();
    ordered
        .families
        .push(FamilyKey::Name("fallback".to_owned()));
    let mut reversed = ordered.clone();
    reversed.families.reverse();
    let requests = [base, weight, width, slope, generic, ordered, reversed];
    let mut cache = MetricsCache::with_limits(requests.len(), usize::MAX);
    for (index, request) in requests.iter().enumerate() {
        cache.insert(request.clone(), Some(metrics(index as f64)));
    }
    for (index, request) in requests.iter().enumerate() {
        assert_eq!(cache.get(request), Some(Some(metrics(index as f64))));
    }
    assert_bounds(&cache);
}
