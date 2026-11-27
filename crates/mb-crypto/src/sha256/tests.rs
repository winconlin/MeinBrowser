//! Tests für SHA-256: die Konstanten aus den Primzahlen, die Beispiele des
//! NIST, die Antwortdateien des CAVP nach dem SHA Validation System
//! (SHAVS) und die Vektoren des ACVP-Servers (SHA2-256-1.0) mit ihren drei
//! Arten: funktionale Tests, Monte-Carlo-Tests und große Nachrichten.

use super::*;

use mb_json::Value;

/// Die Datei des ACVP-Servers mit Aufgaben und Antworten.
const ACVP: &str = "gen-val/json-files/SHA2-256-1.0/internalProjection.json";

/// Der Ordner der CAVP-Antwortdateien.
const CAVP: &str = "vectors/cryptography_vectors/hashes/SHA2";

fn hex(text: &str) -> Vec<u8> {
    assert!(
        text.len().is_multiple_of(2),
        "ungerade Zahl von Hexziffern: {text}"
    );
    (0..text.len())
        .step_by(2)
        .map(|start| {
            let pair = text.get(start..start + 2).expect("zwei Hexziffern");
            u8::from_str_radix(pair, 16).expect("Hexziffern")
        })
        .collect()
}

/// Der Hash eines Stücks, in einem Aufruf.
fn digest(data: &[u8]) -> Digest {
    let mut context = Sha256::new();
    context.update(data);
    context.finalize()
}

/// Der Hash eines Stücks, in Teilen der Längen aus `lengths`, immer wieder
/// von vorn.
fn digest_in_pieces(data: &[u8], lengths: &[usize]) -> Digest {
    let mut context = Sha256::new();
    let mut rest = data;
    for &length in lengths.iter().cycle() {
        if rest.is_empty() {
            break;
        }
        let (piece, tail) = rest.split_at(length.min(rest.len()));
        context.update(piece);
        rest = tail;
    }
    context.finalize()
}

/// Die größte ganze Zahl r mit rᵏ ≤ n.
fn integer_root(n: u128, k: u32) -> u128 {
    let (mut low, mut high) = (0_u128, 1_u128 << 64);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        match middle.checked_pow(k) {
            Some(power) if power <= n => low = middle,
            _ => high = middle - 1,
        }
    }
    low
}

#[test]
fn constants_come_from_the_primes() {
    let primes: Vec<u128> = (2_u128..)
        .filter(|&n| (2..n).take_while(|d| d * d <= n).all(|d| n % d != 0))
        .take(64)
        .collect();
    // Die ersten 32 Bit der Nachkommastellen von ᵏ√p sind die unteren 32 Bit
    // von ᵏ√(p · 2^(32·k)), abgerundet.
    for (index, (&constant, &prime)) in K.iter().zip(&primes).enumerate() {
        let expected = integer_root(prime << 96, 3) & 0xFFFF_FFFF;
        assert_eq!(u128::from(constant), expected, "K{index} aus {prime}");
    }
    for (index, (&word, &prime)) in H0.iter().zip(&primes).enumerate() {
        let expected = integer_root(prime << 64, 2) & 0xFFFF_FFFF;
        assert_eq!(u128::from(word), expected, "H0 Wort {index} aus {prime}");
    }
}

#[test]
fn examples_from_the_standard() {
    // Die Beispiele des NIST zu FIPS 180-4 („SHA256.pdf“ und
    // „SHA2_Additional.pdf“), dazu die leere Nachricht.
    let cases: [(&[u8], &str); 4] = [
        (b"", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        (b"abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        (
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        (
            b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1",
        ),
    ];
    for (message, expected) in cases {
        assert_eq!(digest(message).to_hex(), expected);
        assert_eq!(digest_in_pieces(message, &[1]).to_hex(), expected);
    }
    // Eine Million mal „a“.
    let million = vec![b'a'; 1_000_000];
    let expected = "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0";
    assert_eq!(digest(&million).to_hex(), expected);
    assert_eq!(
        digest_in_pieces(&million, &[63, 64, 65, 1, 1000]).to_hex(),
        expected
    );
}

/// Die Einträge einer CAVP-Antwortdatei: Gruppen von Zeilen `Name = Wert`,
/// getrennt durch Leerzeilen. Kommentare (`#`) und Abschnittsköpfe (`[`)
/// fallen weg.
fn rsp_entries(text: &str) -> Vec<Vec<(String, String)>> {
    let mut entries = Vec::new();
    let mut entry = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            if !entry.is_empty() {
                entries.push(std::mem::take(&mut entry));
            }
        } else if !line.starts_with('#') && !line.starts_with('[') {
            let (name, value) = line.split_once(" = ").expect("Name = Wert");
            entry.push((name.to_string(), value.to_string()));
        }
    }
    if !entry.is_empty() {
        entries.push(entry);
    }
    entries
}

fn field<'a>(entry: &'a [(String, String)], name: &str) -> &'a str {
    entry
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or_else(|| panic!("Feld {name} fehlt in {entry:?}"))
}

/// Prüft die Nachrichten einer CAVP-Antwortdatei, in einem Aufruf und in
/// Stücken. Liefert die Zahl der Nachrichten.
fn check_messages(file: &str, every_split: bool) -> usize {
    let text = testdata::read("pyca-cryptography", &format!("{CAVP}/{file}"));
    let entries = rsp_entries(&text);
    for entry in &entries {
        let bits: usize = field(entry, "Len").parse().expect("Len");
        assert!(bits.is_multiple_of(8), "{file}: Länge {bits} in Bit");
        // Die leere Nachricht steht als „00“ da.
        let message = if bits == 0 {
            Vec::new()
        } else {
            hex(field(entry, "Msg"))
        };
        assert_eq!(message.len(), bits / 8, "{file}: Länge {bits}");
        let expected = field(entry, "MD");
        assert_eq!(digest(&message).to_hex(), expected, "{file}: Len = {bits}");
        assert_eq!(
            digest_in_pieces(&message, &[1]).to_hex(),
            expected,
            "{file}: Len = {bits}"
        );
        assert_eq!(
            digest_in_pieces(&message, &[7, 64, 0, 130]).to_hex(),
            expected
        );
        if every_split {
            for split in 0..=message.len() {
                let mut context = Sha256::new();
                let (head, tail) = message.split_at(split);
                context.update(head);
                context.update(tail);
                assert_eq!(
                    context.finalize().to_hex(),
                    expected,
                    "{file}: Len = {bits}, geteilt bei {split}"
                );
            }
        }
    }
    entries.len()
}

#[test]
fn cavp_short_messages() {
    // Alle Längen von 0 bis 64 Bytes, also jeder Fall beim Auffüllen.
    assert_eq!(check_messages("SHA256ShortMsg.rsp", true), 65);
}

#[test]
fn cavp_long_messages() {
    assert_eq!(check_messages("SHA256LongMsg.rsp", false), 64);
}

#[test]
fn cavp_monte_carlo() {
    // SHAVS, Abschnitt 6.4: je 1000 Hashes über die letzten drei Hashwerte.
    let text = testdata::read("pyca-cryptography", &format!("{CAVP}/SHA256Monte.rsp"));
    let entries = rsp_entries(&text);
    let (first, checkpoints) = entries.split_first().expect("Seed");
    let mut seed = hex(field(first, "Seed"));
    for (count, entry) in checkpoints.iter().enumerate() {
        assert_eq!(field(entry, "COUNT"), count.to_string());
        let [mut a, mut b, mut c] = [seed.clone(), seed.clone(), seed];
        for _ in 0..1000 {
            let message = [a, b.clone(), c.clone()].concat();
            (a, b, c) = (b, c, digest(&message).as_bytes().to_vec());
        }
        assert_eq!(digest_hex(&c), field(entry, "MD"), "COUNT = {count}");
        seed = c;
    }
    assert_eq!(checkpoints.len(), 100);
}

fn digest_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn number(value: &Value) -> u64 {
    match value {
        Value::Number(text) => text.parse().expect("ganze Zahl"),
        _ => panic!("keine Zahl: {value:?}"),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("Text {key} fehlt"))
}

/// Die Datei des ACVP-Servers, gelesen.
fn acvp() -> Value {
    let root = mb_json::parse(&testdata::read("acvp-server", ACVP)).expect("JSON");
    assert_eq!(
        root.get("algorithm").and_then(Value::as_str),
        Some("SHA2-256")
    );
    root
}

/// Die Testgruppen mit der Art `kind`.
fn groups<'a>(root: &'a Value, kind: &str) -> Vec<&'a Value> {
    let groups = root
        .get("testGroups")
        .and_then(Value::as_array)
        .expect("testGroups");
    groups
        .iter()
        .filter(|group| text(group, "testType") == kind)
        .collect()
}

/// Die Tests einer Gruppe. Hashwerte stehen dort in Großbuchstaben.
fn tests(group: &Value) -> &[Value] {
    group.get("tests").and_then(Value::as_array).expect("tests")
}

#[test]
fn acvp_functional_tests() {
    let root = acvp();
    let tests: Vec<&Value> = groups(&root, "AFT").into_iter().flat_map(tests).collect();
    for test in &tests {
        let id = number(test.get("tcId").expect("tcId"));
        let bits = number(test.get("len").expect("len"));
        assert!(bits.is_multiple_of(8), "tcId {id}: Länge {bits} in Bit");
        let message = hex(text(test, "msg"));
        assert_eq!(
            u64::try_from(message.len()).ok(),
            Some(bits / 8),
            "tcId {id}"
        );
        let expected = text(test, "md").to_ascii_lowercase();
        assert_eq!(digest(&message).to_hex(), expected, "tcId {id}");
        let piece = usize::try_from(id % 97 + 1).expect("klein");
        assert_eq!(
            digest_in_pieces(&message, &[piece]).to_hex(),
            expected,
            "tcId {id}"
        );
    }
    assert_eq!(tests.len(), 512);
}

#[test]
fn acvp_monte_carlo_alternate() {
    // Die „alternate“-Fassung aus dem ACVP für SHA-1 und SHA-2
    // (draft-celi-acvp-sha, Abschnitt „Monte Carlo tests“): Die Nachricht
    // A || B || C wird auf die Länge des ersten Startwerts gekürzt oder mit
    // Nullen aufgefüllt.
    let root = acvp();
    let [group] = groups(&root, "MCT")[..] else {
        panic!("eine Gruppe MCT")
    };
    assert_eq!(text(group, "mctVersion"), "alternate");
    let [test] = tests(group) else {
        panic!("ein Test")
    };
    let mut seed = hex(text(test, "msg"));
    let length = usize::try_from(number(test.get("len").expect("len")) / 8).expect("klein");
    assert_eq!(seed.len(), length);
    let results = test
        .get("resultsArray")
        .and_then(Value::as_array)
        .expect("resultsArray");
    assert_eq!(results.len(), 100);
    for (round, result) in results.iter().enumerate() {
        let [mut a, mut b, mut c] = [seed.clone(), seed.clone(), seed];
        for _ in 0..1000 {
            let mut message = [a, b.clone(), c.clone()].concat();
            message.resize(length, 0);
            (a, b, c) = (b, c, digest(&message).as_bytes().to_vec());
        }
        assert_eq!(
            digest_hex(&c),
            text(result, "md").to_ascii_lowercase(),
            "Runde {round}"
        );
        seed = c;
    }
}

/// Die großen Nachrichten des ACVP (1, 2, 4 und 8 GiB) brauchen im
/// Release-Build zusammen etwa eine Minute und 1 GiB Speicher; sie laufen
/// nachts (`.github/workflows/belastung.yml`).
#[test]
#[ignore = "nächtlicher Belastungstest, siehe .github/workflows/belastung.yml"]
fn acvp_large_data() {
    // Ein Puffer von höchstens 1 GiB mit dem wiederholten Inhalt geht so
    // oft wie nötig in einem Aufruf an `update`. Das ACVP verlangt die ganze
    // Nachricht in einem Aufruf; bei 1 GiB ist das so, darüber zählt die
    // Länge wie verlangt über 2³² Bit und 2³² Bytes hinaus.
    const CHUNK: u64 = 1 << 30;
    let root = acvp();
    let tests: Vec<&Value> = groups(&root, "LDT").into_iter().flat_map(tests).collect();
    assert_eq!(tests.len(), 4);
    for test in &tests {
        let id = number(test.get("tcId").expect("tcId"));
        let large = test.get("largeMsg").expect("largeMsg");
        assert_eq!(text(large, "expansionTechnique"), "repeating");
        let content = hex(text(large, "content"));
        assert_eq!(
            u64::try_from(content.len()).ok().map(|n| n * 8),
            Some(number(large.get("contentLength").expect("contentLength")))
        );
        let full = number(large.get("fullLength").expect("fullLength")) / 8;
        let size = full.min(CHUNK);
        assert!(
            full.is_multiple_of(size) && size.is_multiple_of(8),
            "tcId {id}"
        );
        let buffer: Vec<u8> = content
            .iter()
            .copied()
            .cycle()
            .take(usize::try_from(size).expect("1 GiB"))
            .collect();
        let started = std::time::Instant::now();
        let mut context = Sha256::new();
        for _ in 0..full / size {
            context.update(&buffer);
        }
        let result = context.finalize().to_hex();
        println!(
            "tcId {id}: {} GiB in {:.1} s",
            full >> 30,
            started.elapsed().as_secs_f64()
        );
        assert_eq!(result, text(test, "md").to_ascii_lowercase(), "tcId {id}");
    }
}

#[test]
#[should_panic(expected = "länger als 2^64 - 1 Bit")]
fn message_length_is_limited() {
    let mut context = Sha256::new();
    context.length = MAX_BYTES - 1;
    context.update(b"a");
    // Die größte Länge geht noch, ein Byte mehr nicht.
    assert_eq!(context.length, MAX_BYTES);
    context.update(b"a");
}

#[test]
fn length_field_holds_the_largest_message() {
    // Die Länge in Bit steht am Ende des letzten Blocks; bei der größten
    // Nachricht ist sie 2⁶⁴ − 8. Verglichen wird mit einem von Hand
    // aufgefüllten Block nach 5.1.1 über denselben Zwischenwert.
    let mut context = Sha256::new();
    context.length = MAX_BYTES - 3;
    context.update(b"abc");
    let state = context.state;
    let mut block = [0_u8; BLOCK];
    block[..3].copy_from_slice(b"abc");
    block[3] = 0x80;
    block[56..].copy_from_slice(&(u64::MAX - 7).to_be_bytes());
    let mut expected = state;
    compress(&mut expected, &block);
    let bytes: Vec<u8> = expected
        .iter()
        .flat_map(|word| word.to_be_bytes())
        .collect();
    assert_eq!(context.finalize().as_bytes(), bytes.as_slice());
}
