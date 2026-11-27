use std::path::PathBuf;
use std::time::Duration;

use super::*;

/// Ein eigener Ordner je Test (RS-14).
fn directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("mb-platform-ordner-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn names(directory: &Path) -> Vec<OsString> {
    let mut names: Vec<OsString> = regular_files(directory)
        .expect("auflisten")
        .into_iter()
        .map(|file| file.name)
        .collect();
    names.sort();
    names
}

#[test]
fn directories_are_created_with_their_parents() {
    let root = directory("anlegen");
    let nested = root.join("a").join("b");
    create_directory(&nested).expect("anlegen");
    create_directory(&nested).expect("gibt es schon");
    assert!(nested.is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&nested)
            .expect("Angaben")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    let file = root.join("datei");
    std::fs::write(&file, b"x").expect("schreiben");
    let error = create_directory(&file).expect_err("eine Datei");
    assert_eq!(error.kind(), io::ErrorKind::NotADirectory);
    std::fs::remove_dir_all(&root).expect("aufräumen");
}

#[test]
fn files_are_replaced_as_a_whole() {
    let root = directory("ersetzen");
    create_directory(&root).expect("anlegen");
    replace(&root, OsStr::new("eintrag"), b"alt").expect("schreiben");
    replace(&root, OsStr::new("eintrag"), b"neu und l\xc3\xa4nger").expect("ersetzen");
    assert_eq!(
        std::fs::read(root.join("eintrag")).expect("lesen"),
        "neu und länger".as_bytes()
    );
    // Keine Reste der Zwischendateien.
    assert_eq!(names(&root), [OsString::from("eintrag")]);
    let files = regular_files(&root).expect("auflisten");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].len, 15);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(root.join("eintrag"))
            .expect("Angaben")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    remove(&root, OsStr::new("eintrag")).expect("entfernen");
    assert!(names(&root).is_empty());
    let error = remove(&root, OsStr::new("eintrag")).expect_err("schon weg");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    std::fs::remove_dir_all(&root).expect("aufräumen");
}

#[test]
fn names_are_single_components() {
    let root = directory("namen");
    create_directory(&root).expect("anlegen");
    for name in ["", ".", "..", "a/b", "../c", "/d", "e/"] {
        let error = replace(&root, OsStr::new(name), b"x").expect_err(name);
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        let error = remove(&root, OsStr::new(name)).expect_err(name);
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
    }
    assert!(names(&root).is_empty());
    std::fs::remove_dir_all(&root).expect("aufräumen");
}

#[test]
fn only_regular_files_are_listed() {
    let root = directory("auflisten");
    create_directory(&root.join("unterordner")).expect("anlegen");
    std::fs::write(root.join("normal"), b"abc").expect("schreiben");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("normal"), root.join("verweis")).expect("Verweis");
        std::os::unix::fs::symlink("/dev/zero", root.join("geraet")).expect("Verweis");
    }
    assert_eq!(names(&root), [OsString::from("normal")]);
    std::fs::remove_dir_all(&root).expect("aufräumen");
}

#[cfg(unix)]
#[test]
fn replacing_a_link_replaces_the_link() {
    let root = directory("verweis-ersetzen");
    create_directory(&root).expect("anlegen");
    let outside = directory("verweis-ziel");
    create_directory(&outside).expect("anlegen");
    std::fs::write(outside.join("ziel"), b"bleibt").expect("schreiben");
    std::os::unix::fs::symlink(outside.join("ziel"), root.join("eintrag")).expect("Verweis");
    replace(&root, OsStr::new("eintrag"), b"neu").expect("ersetzen");
    assert_eq!(
        std::fs::read(outside.join("ziel")).expect("lesen"),
        b"bleibt"
    );
    assert_eq!(std::fs::read(root.join("eintrag")).expect("lesen"), b"neu");
    assert_eq!(names(&root), [OsString::from("eintrag")]);
    std::fs::remove_dir_all(&root).expect("aufräumen");
    std::fs::remove_dir_all(&outside).expect("aufräumen");
}

#[test]
fn the_time_of_a_file_can_be_set() {
    let root = directory("zeit");
    create_directory(&root).expect("anlegen");
    let file = root.join("eintrag");
    std::fs::write(&file, b"x").expect("schreiben");
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    set_modified(&file, time).expect("Zeit setzen");
    let files = regular_files(&root).expect("auflisten");
    assert_eq!(files[0].modified, time);
    assert_eq!(std::fs::read(&file).expect("lesen"), b"x");
    let error = set_modified(&root, time).expect_err("ein Ordner");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    #[cfg(unix)]
    {
        let error = set_modified(Path::new("/dev/null"), time).expect_err("ein Gerät");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    std::fs::remove_dir_all(&root).expect("aufräumen");
}
