//! Ein Anbieter außerhalb des Crates: Er rechnet selbst und gibt seine
//! Hashwerte über `Digest::from_bytes` zurück.

use mb_crypto::{Builtin, Digest, HashAlgorithm, HashContext, Provider};

/// Ein Anbieter, der die Daten sammelt und am Ende selbst hasht, hier mit
/// dem eigenen Anbieter als Rechenwerk, aber nur über dessen Bytes.
struct Collecting;

struct Collected {
    algorithm: HashAlgorithm,
    data: Vec<u8>,
}

impl HashContext for Collected {
    fn update(&mut self, data: &[u8]) {
        self.data.extend_from_slice(data);
    }

    fn finish(self: Box<Self>) -> Digest {
        let bytes = Builtin
            .digest(self.algorithm, &self.data)
            .as_bytes()
            .to_vec();
        Digest::from_bytes(self.algorithm, &bytes).expect("passende Länge")
    }
}

impl Provider for Collecting {
    fn hash(&self, algorithm: HashAlgorithm) -> Box<dyn HashContext> {
        Box::new(Collected {
            algorithm,
            data: Vec::new(),
        })
    }
}

#[test]
fn another_provider_returns_its_own_digests() {
    let providers: [&dyn Provider; 2] = [&Builtin, &Collecting];
    let [ours, theirs] = providers.map(|provider| {
        let mut context = provider.hash(HashAlgorithm::Sha256);
        context.update(b"ab");
        context.update(b"c");
        context.finish()
    });
    assert_eq!(ours, theirs);
    assert_eq!(theirs, Collecting.digest(HashAlgorithm::Sha256, b"abc"));
}

#[test]
fn digest_from_bytes_checks_the_length() {
    let bytes = [0xAB_u8; 32];
    let digest = Digest::from_bytes(HashAlgorithm::Sha256, &bytes).expect("32 Bytes");
    assert_eq!(digest.as_bytes(), bytes);
    assert_eq!(digest.algorithm(), HashAlgorithm::Sha256);
    assert_eq!(
        Digest::from_bytes(HashAlgorithm::Sha256, &bytes[..31]),
        None
    );
    assert_eq!(Digest::from_bytes(HashAlgorithm::Sha256, &[0; 33]), None);
    assert_eq!(Digest::from_bytes(HashAlgorithm::Sha256, &[]), None);
}
