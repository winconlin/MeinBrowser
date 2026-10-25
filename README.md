# MeinBrowser
A new Browser lightweight, open source, fully compatible to firefox and chrome plugins.

Architekturkonzept (Deutsch): [docs/architektur.md](docs/architektur.md)

Sicherheitslücken bitte vertraulich melden, siehe [SECURITY.md](SECURITY.md).

## Bauen und prüfen

Voraussetzung ist eine aktuelle stabile Rust-Toolchain mit rustfmt und clippy.

```sh
cargo run -p deps-guard                                # keine externen Abhängigkeiten
cargo run -p datagen -- --check                        # generierte Tabellen passen zu data/
cargo fmt --all --check                                # Formatierung
cargo clippy --workspace --all-targets -- -D warnings  # Lints
cargo run -p testdata                                  # externe Testdaten holen (einmal vor den Tests)
cargo test --workspace                                 # Tests
cargo run -p meinbrowser                               # Fenster mit Testbild (Meilenstein M0)
cargo run -p meinbrowser -- --selbsttest               # zeigt das Testbild und prüft es
```

Unter Linux braucht das Fenster einen X-Server (unter Wayland über XWayland). Ohne Bildschirm geht es mit `xvfb-run cargo run -p meinbrowser -- --selbsttest`.

Fremde Crates sind nicht erlaubt, auch nicht als dev- oder build-dependency.

Tabellen aus Rohdaten unter `data/` (zurzeit die Unicode-Zeichendatenbank mit der IDNA-Zuordnungstabelle und die Daten des WHATWG Encoding Standard) erzeugt `cargo run -p datagen`. Die erzeugten Dateien werden eingecheckt.

Externe Testsammlungen (zurzeit Teile der WPT, html5lib-tests, Testdateien von Unicode, css-parsing-tests und webref) liegen nicht im Repository. `cargo run -p testdata` holt sie mit `git` in der Version, die `tools/testdata/src/lib.rs` festlegt, nach `target/testdata/`.
