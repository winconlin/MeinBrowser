# MeinBrowser
Ein unabhängiger, ressourcenschonender Open-Source-Browser mit eigener Engine. Ziel ist die Unterstützung von Chrome- und Firefox-Erweiterungen; implementiert sind bisher die Grundlagen zur Darstellung lokaler HTML- und XHTML-Dateien (M0 und M1).

Architekturkonzept (Deutsch): [docs/architektur.md](docs/architektur.md)

Grundsatz ist eigener Browsercode. Geplant ist eine optionale Widevine-Anbindung für DRM: standardmäßig deaktiviert, nur nach Zustimmung und in einem eigenen eingeschränkten Prozess. Sie ist noch nicht implementiert; Lizenzierung, Plattformen und Verfügbarkeit bei Streamingdiensten sind gesondert zu klären. Die Entscheidung und Schutzgrenzen stehen in [Abschnitt 4.13 der Architektur](docs/architektur.md#413-bilder-audio-und-video).

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
cargo run --release -p testrunner                      # Referenztests für M1 (WPT, normal-flow)
cargo run -p meinbrowser                               # Fenster mit Testbild (Meilenstein M0)
cargo run -p meinbrowser -- --selbsttest               # zeigt das Testbild und prüft es
cargo run -p meinbrowser -- seite.html                 # zeigt eine lokale HTML- oder XHTML-Datei (M1)
```

Unter Linux braucht das Fenster einen X-Server (unter Wayland über XWayland). Ohne Bildschirm geht es mit `xvfb-run cargo run -p meinbrowser -- --selbsttest`.

Fremde Crates sind nicht erlaubt, auch nicht als dev- oder build-dependency.

Einzige Ausnahme ist das Fuzzing unter `fuzz/`: ein eigener Cargo-Workspace mit `cargo-fuzz` und `libfuzzer-sys` (entschieden am 30.10.2026). Es wird nicht ausgeliefert, und kein Crate des Browsers hängt davon ab. Anleitung: [docs/fuzzing.md](docs/fuzzing.md).

Tabellen aus Rohdaten unter `data/` (zurzeit die Unicode-Zeichendatenbank mit der IDNA-Zuordnungstabelle, die Daten des WHATWG Encoding Standard, die benannten Zeichenreferenzen und die CSS-Blöcke aus dem Abschnitt „Rendering“ des HTML Standard) erzeugt `cargo run -p datagen`. Die erzeugten Dateien werden eingecheckt.

Externe Testsammlungen (zurzeit Teile der WPT, html5lib-tests, Testdateien von Unicode, css-parsing-tests, webref, die XML-Testsuite des W3C, Deflate-Testdaten aus Go und die PNG-Testbilder von image-png) liegen nicht im Repository. `cargo run -p testdata` holt sie mit `git` in der Version, die `tools/testdata/src/lib.rs` festlegt, nach `target/testdata/`.
