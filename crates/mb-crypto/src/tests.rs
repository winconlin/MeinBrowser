//! Tests der Anbieter-Schnittstelle.

use super::*;

/// Ein einfacher Zufallsgenerator für die Tests (xorshift64).
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: u64) -> usize {
        usize::try_from(self.next() % limit).expect("klein")
    }
}

#[test]
fn digest_matches_hash_in_any_pieces() {
    // Daten jeder Länge bis über vier Blöcke, in zufällige Stücke geteilt,
    // auch leere; über die Schnittstelle als Box und in einem Aufruf.
    let provider: &dyn Provider = &Builtin;
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    for length in 0..300 {
        let data: Vec<u8> = (0..length)
            .map(|_| random.next().to_le_bytes()[0])
            .collect();
        let whole = provider.digest(HashAlgorithm::Sha256, &data);
        for _ in 0..4 {
            let mut context = provider.hash(HashAlgorithm::Sha256);
            let mut rest = data.as_slice();
            while !rest.is_empty() {
                let (piece, tail) = rest.split_at(random.below(70).min(rest.len()));
                context.update(piece);
                rest = tail;
            }
            context.update(&[]);
            assert_eq!(context.finish(), whole, "Länge {length}");
        }
    }
}

#[test]
fn digest_reports_its_algorithm() {
    let digest = Builtin.digest(HashAlgorithm::Sha256, b"abc");
    assert_eq!(digest.algorithm(), HashAlgorithm::Sha256);
    assert_eq!(digest.as_bytes().len(), HashAlgorithm::Sha256.output_len());
    assert_eq!(digest.as_bytes().first(), Some(&0xBA));
    assert_eq!(
        format!("{digest:?}"),
        "Digest(Sha256, ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad)"
    );
    assert_ne!(digest, Builtin.digest(HashAlgorithm::Sha256, b"abd"));
}

#[test]
fn provider_is_usable_across_threads() {
    fn shared<T: Send + Sync>(_: &T) {}
    shared(&Builtin);
    let digests: Vec<Digest> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4_u8)
            .map(|index| scope.spawn(move || Builtin.digest(HashAlgorithm::Sha256, &[index; 100])))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("Thread"))
            .collect()
    });
    for (index, digest) in (0..4_u8).zip(&digests) {
        assert_eq!(
            *digest,
            Builtin.digest(HashAlgorithm::Sha256, &[index; 100])
        );
    }
}
