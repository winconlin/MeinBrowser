//! Speicherzählung und Verdrängung gelten für Dateien, nicht für Schnitte.

use super::*;

type Sources = Arc<Mutex<BTreeMap<PathBuf, Vec<u8>>>>;

fn ahem() -> Vec<u8> {
    testdata::read_bytes("wpt", "fonts/Ahem.ttf")
}

fn with_fonts(budget: u64, files: Vec<(&str, Vec<u8>)>) -> (FontLibrary, Sources) {
    let sources: Sources = Arc::new(Mutex::new(
        files
            .into_iter()
            .map(|(name, bytes)| (PathBuf::from(name), bytes))
            .collect(),
    ));
    let reader = Arc::clone(&sources);
    let mut library = FontLibrary::with_cache_budget(
        Box::new(move |path| {
            reader
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(path)
                .cloned()
        }),
        budget,
    );
    let paths: Vec<PathBuf> = sources
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect();
    for path in paths {
        assert!(library.add_file(path) > 0, "gültige Prüfschrift");
    }
    (library, sources)
}

fn ids(library: &FontLibrary) -> Vec<FaceId> {
    library.family("Ahem").to_vec()
}

fn assert_accounting(stats: &FontCacheStats) {
    assert_eq!(
        stats.resident_bytes,
        stats.active_bytes + stats.unused_bytes
    );
}

fn fallback_query() -> FontQuery<'static> {
    FontQuery {
        families: &[],
        weight: 400.0,
        width: 100.0,
        slope: Slope::Normal,
    }
}

fn cached_fallback(library: &FontLibrary, characters: &[char]) -> Option<Option<FaceId>> {
    let key = FallbackKey::new(&fallback_query(), characters).expect("kleine Anfrage");
    library
        .cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fallback
        .get(&key)
}

#[test]
fn an_empty_cache_reports_its_budget_without_loading_anything() {
    let library = FontLibrary::new(Box::new(|_| panic!("keine Datei registriert")));
    let stats = library.cache_stats();
    assert_eq!(stats.budget_bytes, 64 << 20);
    assert_eq!(
        (stats.resident_bytes, stats.active_bytes, stats.unused_bytes),
        (0, 0, 0)
    );
    assert_eq!((stats.resident_files, stats.resident_faces), (0, 0));
    assert_eq!((stats.loads, stats.evictions), (0, 0));
    let trimmed = library.trim_cache();
    assert_eq!((trimmed.loads, trimmed.evictions), (0, 0));
    assert_accounting(&trimmed);
}

#[test]
fn active_faces_are_reused_and_release_their_bytes_after_the_last_drop() {
    let font = ahem();
    let bytes = font.len() as u64;
    let (library, _) = with_fonts(0, vec![("a.ttf", font)]);
    let id = ids(&library)[0];
    let initial = library.cache_stats();
    assert_eq!(
        (initial.loads, initial.resident_bytes),
        (0, 0),
        "Index ist kein Daten-Cache"
    );
    let first = library.face(id).expect("Schrift");
    let second = library.face(id).expect("dieselbe Schrift");
    assert!(Arc::ptr_eq(&first, &second));
    for _ in 0..3 {
        let stats = library.cache_stats();
        assert_eq!(
            (stats.resident_bytes, stats.active_bytes, stats.unused_bytes),
            (bytes, bytes, 0)
        );
        assert_eq!((stats.loads, stats.evictions), (1, 0));
        assert_eq!((stats.resident_files, stats.resident_faces), (1, 1));
        assert_accounting(&stats);
    }
    let trimmed = library.trim_cache();
    assert_eq!((trimmed.resident_bytes, trimmed.evictions), (bytes, 0));
    assert!(first.glyph_index('X').is_some());
    drop(first);
    drop(second);
    let unused = library.cache_stats();
    assert_eq!((unused.active_bytes, unused.unused_bytes), (0, bytes));
    let trimmed = library.trim_cache();
    assert_eq!(
        (
            trimmed.resident_bytes,
            trimmed.resident_files,
            trimmed.resident_faces
        ),
        (0, 0, 0)
    );
    assert_eq!((trimmed.loads, trimmed.evictions), (1, 1));
    let reloaded = library.face(id).expect("erneut geladen");
    assert!(reloaded.glyph_index('X').is_some());
    assert_eq!(library.cache_stats().loads, 2);
    drop(reloaded);
    assert_eq!(library.trim_cache().resident_bytes, 0);
}

#[test]
fn an_owned_face_clone_keeps_the_shared_file_active() {
    let font = ahem();
    let bytes = font.len() as u64;
    let (library, _) = with_fonts(0, vec![("a.ttf", font)]);
    let id = ids(&library)[0];
    let face = library.face(id).expect("Schrift");
    // Ein eigener Face hat ein eigenes FontData-Arc. Er erhöht nicht den
    // strong_count des äußeren Arc<Face> im Cache.
    let owned: Face = face.as_ref().clone();
    drop(face);
    let stats = library.trim_cache();
    assert_eq!((stats.active_bytes, stats.unused_bytes), (bytes, 0));
    assert_eq!(stats.evictions, 0);
    assert!(owned.glyph_index('X').is_some());
    let again = library.face(id).expect("Cachetreffer");
    assert_eq!(library.cache_stats().loads, 1);
    drop(again);
    drop(owned);
    assert_eq!(library.trim_cache().resident_bytes, 0);
}

#[test]
fn weak_face_handles_pin_the_file_until_they_are_dropped() {
    let font = ahem();
    let bytes = font.len() as u64;
    let (library, _) = with_fonts(0, vec![("a.ttf", font)]);
    let id = ids(&library)[0];
    let face = library.face(id).expect("Schrift");
    let weak = Arc::downgrade(&face);
    drop(face);
    let stats = library.trim_cache();
    assert_eq!((stats.active_bytes, stats.unused_bytes), (bytes, 0));
    let recovered = weak.upgrade().expect("schwaches Handle bleibt nutzbar");
    let from_cache = library.face(id).expect("Cachetreffer");
    assert!(Arc::ptr_eq(&recovered, &from_cache));
    assert_eq!(library.cache_stats().loads, 1);
    drop(recovered);
    drop(from_cache);
    drop(weak);
    assert_eq!(library.trim_cache().resident_bytes, 0);
}

fn two_face_collection(font: &[u8]) -> Vec<u8> {
    // Beide Indizes zeigen auf denselben gültigen sfnt-Schnitt. Die
    // Sammlung muss trotz zweier Face-Objekte nur einmal gezählt werden.
    let header = 20_u32;
    let mut collection = b"ttcf".to_vec();
    collection.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    collection.extend_from_slice(&2_u32.to_be_bytes());
    collection.extend_from_slice(&header.to_be_bytes());
    collection.extend_from_slice(&header.to_be_bytes());
    collection.extend_from_slice(font);
    let tables = usize::from(u16::from_be_bytes([font[4], font[5]]));
    for table in 0..tables {
        let field = header as usize + 12 + table * 16 + 8;
        let offset = u32::from_be_bytes(collection[field..field + 4].try_into().expect("4 Bytes"));
        collection[field..field + 4].copy_from_slice(&(offset + header).to_be_bytes());
    }
    collection
}

#[test]
fn collection_bytes_are_counted_once_and_all_faces_share_their_lifetime() {
    let collection = two_face_collection(&ahem());
    let bytes = collection.len() as u64;
    let (library, _) = with_fonts(0, vec![("collection.ttc", collection)]);
    let ids = ids(&library);
    assert_eq!(ids.len(), 2);
    let first = library.face(ids[0]).expect("erster Schnitt");
    let second = library.face(ids[1]).expect("zweiter Schnitt");
    let stats = library.cache_stats();
    assert_eq!(
        (stats.resident_bytes, stats.active_bytes, stats.unused_bytes),
        (bytes, bytes, 0)
    );
    assert_eq!(
        (stats.resident_files, stats.resident_faces, stats.loads),
        (1, 2, 1)
    );
    drop(first);
    assert_eq!(
        library.trim_cache().resident_bytes,
        bytes,
        "zweiter Schnitt lebt noch"
    );
    assert!(second.glyph_index('X').is_some());
    drop(second);
    let stats = library.trim_cache();
    assert_eq!(
        (
            stats.resident_bytes,
            stats.resident_files,
            stats.resident_faces
        ),
        (0, 0, 0)
    );
    assert_eq!(stats.evictions, 1);
}

#[test]
fn the_least_recently_used_inactive_file_is_evicted() {
    let font = ahem();
    let bytes = font.len() as u64;
    let (library, _) = with_fonts(
        2 * bytes,
        vec![
            ("a.ttf", font.clone()),
            ("b.ttf", font.clone()),
            ("c.ttf", font),
        ],
    );
    let ids = ids(&library);
    assert_eq!(ids.len(), 3);
    drop(library.face(ids[0]).expect("A"));
    drop(library.face(ids[1]).expect("B"));
    drop(library.face(ids[0]).expect("A erneut benutzt"));
    drop(library.face(ids[2]).expect("C"));
    let stats = library.trim_cache();
    assert_eq!(
        (stats.resident_bytes, stats.unused_bytes),
        (2 * bytes, 2 * bytes)
    );
    assert_eq!((stats.loads, stats.evictions), (3, 1));
    drop(library.face(ids[0]).expect("A blieb im Cache"));
    drop(library.face(ids[2]).expect("C blieb im Cache"));
    assert_eq!(library.cache_stats().loads, 3);
    drop(library.face(ids[1]).expect("B wird nachgeladen"));
    assert_eq!(library.cache_stats().loads, 4);
    assert_accounting(&library.cache_stats());
}

#[test]
fn a_single_font_larger_than_the_budget_is_retained_only_while_active() {
    let font = ahem();
    let bytes = font.len() as u64;
    let (library, _) = with_fonts(bytes - 1, vec![("a.ttf", font)]);
    let id = ids(&library)[0];
    let first = library.face(id).expect("auch über dem Budget nutzbar");
    let second = library.face(id).expect("kein ständiges Nachladen");
    assert!(Arc::ptr_eq(&first, &second));
    let stats = library.trim_cache();
    assert_eq!(
        (stats.resident_bytes, stats.active_bytes, stats.loads),
        (bytes, bytes, 1)
    );
    drop(first);
    drop(second);
    let stats = library.trim_cache();
    assert_eq!((stats.resident_bytes, stats.evictions), (0, 1));
}

#[test]
fn a_failed_reload_leaves_no_resident_buffer_or_repeated_io() {
    let font = ahem();
    let (library, sources) = with_fonts(0, vec![("a.ttf", font)]);
    let id = ids(&library)[0];
    drop(library.face(id).expect("erster Abruf"));
    assert_eq!(library.trim_cache().resident_bytes, 0);
    sources
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(Path::new("a.ttf"));
    assert!(library.face(id).is_none());
    assert!(library.face(id).is_none());
    let stats = library.trim_cache();
    assert_eq!(
        (
            stats.resident_bytes,
            stats.resident_files,
            stats.resident_faces
        ),
        (0, 0, 0)
    );
    assert_eq!(stats.loads, 2);
    assert_accounting(&stats);
}

#[test]
fn alternating_known_missing_characters_do_not_reload_trimmed_files() {
    let (library, _) = with_fonts(0, vec![("a.ttf", ahem())]);
    let query = FontQuery {
        families: &[],
        weight: 400.0,
        width: 100.0,
        slope: Slope::Normal,
    };
    let missing = ['\u{10ffff}', '\u{10fffe}'];
    for character in missing {
        assert_eq!(library.fallback(&query, &[character]), None);
        assert_eq!(library.trim_cache().resident_bytes, 0);
    }
    let loads = library.cache_stats().loads;
    for _ in 0..10 {
        for character in missing {
            assert_eq!(library.fallback(&query, &[character]), None);
        }
    }
    assert_eq!(
        library.cache_stats().loads,
        loads,
        "Suchcache verhindert Datei-Pingpong"
    );
}

#[test]
fn a_malformed_reload_is_not_kept_or_retried() {
    let (library, sources) = with_fonts(0, vec![("a.ttf", ahem())]);
    let id = ids(&library)[0];
    drop(library.face(id).expect("gültige Schrift vor Bereinigung"));
    assert_eq!(library.trim_cache().resident_bytes, 0);
    sources
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(PathBuf::from("a.ttf"), b"keine Schrift".to_vec());
    assert!(library.face(id).is_none());
    let stats = library.trim_cache();
    assert_eq!(
        (
            stats.resident_bytes,
            stats.resident_files,
            stats.resident_faces
        ),
        (0, 0, 0)
    );
    assert_eq!(stats.loads, 2);
    assert!(library.face(id).is_none());
    assert_eq!(library.cache_stats().loads, 2, "Fehler wird gemerkt");
}

#[test]
fn a_cached_positive_match_becomes_negative_when_reload_fails() {
    for replacement in [None, Some(b"keine Schrift".to_vec())] {
        let (library, sources) = with_fonts(0, vec![("a.ttf", ahem())]);
        let query = fallback_query();
        let old = library.fallback(&query, &['A']).expect("alter Treffer");
        assert_eq!(cached_fallback(&library, &['A']), Some(Some(old)));
        assert_eq!(library.trim_cache().resident_bytes, 0);
        {
            let mut sources = sources.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(bytes) = replacement {
                sources.insert(PathBuf::from("a.ttf"), bytes);
            } else {
                sources.remove(Path::new("a.ttf"));
            }
        }
        assert_eq!(library.fallback(&query, &['A']), None);
        assert_eq!(cached_fallback(&library, &['A']), Some(None));
        assert_eq!(library.cache_stats().loads, 2);
        assert_eq!(library.fallback(&query, &['A']), None);
        assert_eq!(library.cache_stats().loads, 2, "kein erneuter Dateiversuch");
        assert_eq!(library.trim_cache().resident_bytes, 0);
    }
}

#[test]
fn a_failed_cached_positive_match_does_not_hide_another_font() {
    for replacement in [None, Some(b"keine Schrift".to_vec())] {
        let basic = testdata::read_bytes("wpt", "fonts/CSSTest/csstest-basic-regular.ttf");
        let (library, sources) = with_fonts(0, vec![("a.ttf", ahem()), ("b.ttf", basic)]);
        let query = fallback_query();
        let old = library.fallback(&query, &['A']).expect("Ahem zuerst");
        assert_eq!(old, library.family("Ahem")[0]);
        assert_eq!(library.trim_cache().resident_bytes, 0);
        {
            let mut sources = sources.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(bytes) = replacement {
                sources.insert(PathBuf::from("a.ttf"), bytes);
            } else {
                sources.remove(Path::new("a.ttf"));
            }
        }
        let alternative = library
            .fallback(&query, &['A'])
            .expect("CSSTest Basic übernimmt");
        assert_ne!(alternative, old);
        assert_eq!(alternative, library.family("CSSTest Basic")[0]);
        assert_eq!(cached_fallback(&library, &['A']), Some(Some(alternative)));
        assert!(
            library
                .face(alternative)
                .expect("neuer Treffer")
                .glyph_index('A')
                .is_some()
        );
        assert_eq!(library.cache_stats().loads, 3);
        assert_eq!(library.fallback(&query, &['A']), Some(alternative));
        assert_eq!(library.cache_stats().loads, 3, "kein erneuter Fehlversuch");
    }
}

#[test]
fn a_valid_reloaded_font_must_still_cover_the_cached_characters() {
    let (library, sources) = with_fonts(0, vec![("a.ttf", ahem())]);
    let query = fallback_query();
    let old = library
        .fallback(&query, &['一'])
        .expect("Ahem hat das Zeichen");
    assert_eq!(cached_fallback(&library, &['一']), Some(Some(old)));
    assert_eq!(library.trim_cache().resident_bytes, 0);
    let basic = testdata::read_bytes("wpt", "fonts/CSSTest/csstest-basic-regular.ttf");
    sources
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(PathBuf::from("a.ttf"), basic);
    // Das Nachladen ist erfolgreich; die nun geladene Schrift hat dieses
    // Zeichen aber nicht. Ein alter FaceId-Treffer genügt deshalb nicht.
    assert_eq!(library.fallback(&query, &['一']), None);
    assert_eq!(cached_fallback(&library, &['一']), Some(None));
    assert_eq!(library.cache_stats().loads, 2);
}
