# MeinBrowser
A new Browser lightweight, open source, fully compatible to firefox and chrome plugins.

Architekturkonzept (Deutsch): [docs/architektur.md](docs/architektur.md)

Sicherheitslücken bitte vertraulich melden, siehe [SECURITY.md](SECURITY.md).

## Bauen und prüfen

Voraussetzung ist eine aktuelle stabile Rust-Toolchain mit rustfmt und clippy.

```sh
cargo run -p deps-guard                                # keine externen Abhängigkeiten
cargo fmt --all --check                                # Formatierung
cargo clippy --workspace --all-targets -- -D warnings  # Lints
cargo test --workspace                                 # Tests
cargo run -p meinbrowser                               # Programm (noch ein Platzhalter)
```

Fremde Crates sind nicht erlaubt, auch nicht als dev- oder build-dependency.
