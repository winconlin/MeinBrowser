use std::sync::Arc;
use std::time::Duration;

use super::*;

/// Ein eigener Ordner je Test (RS-14).
fn directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("mb-storage-cache-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn key(byte: u8) -> Key {
    let mut key = [byte; 32];
    key[31] = 0xA5;
    key
}

fn name(byte: u8) -> String {
    file_name(&key(byte)).into_string().expect("ASCII")
}

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("auflisten")
        .map(|entry| {
            entry
                .expect("Eintrag")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .collect();
    names.sort();
    names
}

#[test]
fn keys_and_file_names_belong_together() {
    let name = file_name(&key(0x3C));
    assert_eq!(
        name,
        "3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3ca5"
    );
    assert_eq!(key_of(&name), Some(key(0x3C)));
    for other in [
        "",
        "3c",
        "3C3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3ca5",
        "3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3ca5.tmp",
        "g3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3ca5",
    ] {
        assert_eq!(key_of(OsStr::new(other)), None, "{other}");
    }
    let leftover = format!(".{}.17.3.tmp", name.to_str().expect("ASCII"));
    assert!(is_leftover(OsStr::new(&leftover)));
    assert!(!is_leftover(&name));
    assert!(!is_leftover(OsStr::new(".fremd.tmp")));
}

#[test]
fn entries_survive_a_restart() {
    let root = directory("neustart");
    let cache = DiskCache::open(&root, 1 << 20).expect("öffnen");
    assert!(cache.is_empty());
    assert_eq!(cache.load(&key(1)), None);
    cache.store(&key(1), b"eins");
    cache.store(&key(2), b"zwei!");
    cache.store(&key(1), b"eins, neu");
    assert_eq!(cache.load(&key(1)).as_deref(), Some(&b"eins, neu"[..]));
    assert_eq!((cache.len(), cache.bytes()), (2, 14));
    drop(cache);
    let cache = DiskCache::open(&root, 1 << 20).expect("wieder öffnen");
    assert_eq!((cache.len(), cache.bytes()), (2, 14));
    assert_eq!(cache.load(&key(2)).as_deref(), Some(&b"zwei!"[..]));
    assert_eq!(cache.directory(), root);
    assert_eq!(cache.limit(), 1 << 20);
    std::fs::remove_dir_all(&root).expect("aufräumen");
}

#[test]
fn the_least_recently_used_falls_out() {
    let root = directory("grenze");
    let cache = DiskCache::open(&root, 10).expect("öffnen");
    cache.store(&key(1), b"aaaa");
    cache.store(&key(2), b"bbbb");
    assert!(cache.load(&key(1)).is_some());
    cache.store(&key(3), b"cccc");
    assert_eq!(cache.load(&key(2)), None, "am längsten nicht gebraucht");
    assert!(cache.load(&key(1)).is_some());
    assert!(cache.load(&key(3)).is_some());
    assert_eq!(cache.bytes(), 8);
    // Zu groß für die Grenze: Auch der alte Eintrag fällt heraus.
    cache.store(&key(1), b"elf Zeichen");
    assert_eq!(cache.load(&key(1)), None);
    assert_eq!(names(&root), [name(3)]);
    std::fs::remove_dir_all(&root).expect("aufräumen");
}

#[test]
fn the_order_survives_a_restart() {
    let root = directory("reihenfolge");
    let cache = DiskCache::open(&root, 1 << 20).expect("öffnen");
    for byte in 1..=4 {
        cache.store(&key(byte), &[byte; 4]);
    }
    drop(cache);
    // Die Zeiten der Dateien bestimmen die Reihenfolge: 3, 1, 4, 2.
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for (byte, seconds) in [(3, 0), (1, 10), (4, 20), (2, 30)] {
        let time = base + Duration::from_secs(seconds);
        files::set_modified(&root.join(name(byte)), time).expect("Zeit setzen");
    }
    // Mit einer kleineren Grenze fallen gleich die beiden ältesten heraus.
    let cache = DiskCache::open(&root, 8).expect("öffnen");
    assert_eq!((cache.len(), cache.bytes()), (2, 8));
    assert_eq!(cache.load(&key(3)), None);
    assert_eq!(cache.load(&key(1)), None);
    let mut expected = vec![name(4), name(2)];
    expected.sort();
    assert_eq!(names(&root), expected);
    // Laden setzt die Zeit, also übersteht auch das einen Neustart.
    assert!(cache.load(&key(4)).is_some());
    drop(cache);
    let cache = DiskCache::open(&root, 4).expect("öffnen");
    assert!(cache.load(&key(4)).is_some());
    assert_eq!(cache.load(&key(2)), None);
    std::fs::remove_dir_all(&root).expect("aufräumen");
}

#[test]
fn foreign_files_stay_and_leftovers_go() {
    let root = directory("fremde");
    std::fs::create_dir_all(root.join("unterordner")).expect("anlegen");
    std::fs::write(root.join("liesmich.txt"), b"fremd").expect("schreiben");
    let leftover = format!(".{}.1.2.tmp", name(7));
    std::fs::write(root.join(&leftover), b"halb").expect("schreiben");
    std::fs::write(root.join(".fremd.tmp"), b"fremd").expect("schreiben");
    let cache = DiskCache::open(&root, 1 << 20).expect("öffnen");
    assert!(cache.is_empty());
    assert_eq!(names(&root), [".fremd.tmp", "liesmich.txt", "unterordner"]);
    std::fs::remove_dir_all(&root).expect("aufräumen");
}

#[test]
fn vanished_and_changed_files_are_handled() {
    let root = directory("verschwunden");
    let cache = DiskCache::open(&root, 16).expect("öffnen");
    cache.store(&key(1), b"eins");
    cache.store(&key(2), b"zwei");
    std::fs::remove_file(root.join(name(1))).expect("entfernen");
    assert_eq!(cache.load(&key(1)), None);
    assert_eq!((cache.len(), cache.bytes()), (1, 4));
    // Von außen verändert: Der Speicher liefert, was dort steht, bis zur
    // Grenze; prüfen muss, wer liest.
    std::fs::write(root.join(name(2)), b"anders").expect("schreiben");
    assert_eq!(cache.load(&key(2)).as_deref(), Some(&b"anders"[..]));
    std::fs::write(root.join(name(2)), [0; 17]).expect("schreiben");
    assert_eq!(cache.load(&key(2)), None, "größer als die Grenze");
    assert!(cache.is_empty());
    std::fs::remove_dir_all(&root).expect("aufräumen");
}

#[cfg(unix)]
#[test]
fn links_in_the_directory_are_not_entries() {
    let root = directory("verweise");
    std::fs::create_dir_all(&root).expect("anlegen");
    let outside = directory("verweise-ziel");
    std::fs::create_dir_all(&outside).expect("anlegen");
    std::fs::write(outside.join("geheim"), b"geheim").expect("schreiben");
    std::os::unix::fs::symlink(outside.join("geheim"), root.join(name(9))).expect("Verweis");
    let cache = DiskCache::open(&root, 1 << 20).expect("öffnen");
    assert!(cache.is_empty());
    assert_eq!(cache.load(&key(9)), None);
    // Ablegen ersetzt den Verweis, nicht sein Ziel.
    cache.store(&key(9), b"eigen");
    assert_eq!(cache.load(&key(9)).as_deref(), Some(&b"eigen"[..]));
    assert_eq!(
        std::fs::read(outside.join("geheim")).expect("lesen"),
        b"geheim"
    );
    std::fs::remove_dir_all(&root).expect("aufräumen");
    std::fs::remove_dir_all(&outside).expect("aufräumen");
}

#[test]
fn threads_share_a_cache() {
    let root = directory("faeden");
    let cache = Arc::new(DiskCache::open(&root, 64 * 100).expect("öffnen"));
    let threads: Vec<_> = (0..4_u8)
        .map(|thread| {
            let cache = Arc::clone(&cache);
            std::thread::spawn(move || {
                for round in 0..50_u8 {
                    let key = key(thread * 50 + round);
                    let bytes = [round; 64];
                    cache.store(&key, &bytes);
                    if let Some(loaded) = cache.load(&key) {
                        assert_eq!(loaded, bytes);
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("Faden");
    }
    assert_eq!((cache.len(), cache.bytes()), (100, 6400));
    let on_disk = files::regular_files(&root).expect("auflisten");
    assert_eq!(on_disk.len(), 100);
    assert!(on_disk.iter().all(|file| key_of(&file.name).is_some()));
    std::fs::remove_dir_all(&root).expect("aufräumen");
}
