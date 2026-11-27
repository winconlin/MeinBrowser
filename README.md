# MeinBrowser
Ein unabhängiger, ressourcenschonender Open-Source-Browser mit eigener Engine. Ziel ist die Unterstützung von Chrome- und Firefox-Erweiterungen; implementiert sind bisher die Grundlagen zur Darstellung lokaler HTML- und XHTML-Dateien (M0 und M1, abgeschlossen) und große Teile der JavaScript-Engine (M2, läuft); Skripte in Seiten laufen aber noch nicht. Netzwerk, Oberfläche und Erweiterungen fehlen noch.

Architekturkonzept (Deutsch): [docs/architektur.md](docs/architektur.md). Wo die Ziele und die Meilensteine heute stehen, zeigen dort die Abschnitte [1.3](docs/architektur.md#13-stand-der-ziele) und [12.1](docs/architektur.md#121-stand-der-meilensteine).

Grundsatz ist eigener Browsercode. Geplant ist eine optionale Widevine-Anbindung für DRM: standardmäßig deaktiviert, nur nach Zustimmung und in einem eigenen eingeschränkten Prozess. Sie ist noch nicht implementiert; Lizenzierung, Plattformen und Verfügbarkeit bei Streamingdiensten sind gesondert zu klären. Die Entscheidung und Schutzgrenzen stehen in [Abschnitt 4.13 der Architektur](docs/architektur.md#413-bilder-audio-und-video).

Sicherheitslücken bitte vertraulich melden, siehe [SECURITY.md](SECURITY.md).

## Entwicklungsstand

Stand 27.11.2026, nach PR #141. Bis M5 ist der Browser nur für Entwicklung und Tests gedacht: Es gibt noch kein Netzwerk, keine Sandbox und keine Oberfläche. Ausführlich stehen der Stand je Ziel in [1.3](docs/architektur.md#13-stand-der-ziele), je Meilenstein in [12.1](docs/architektur.md#121-stand-der-meilensteine) und die nächsten Schritte in [12.2](docs/architektur.md#122-nächste-schritte) der Architektur.

| Meilenstein | Stand | Erreicht |
|---|---|---|
| M0 Fundament | abgeschlossen | Workspace, CI für Linux, Windows und macOS, Abhängigkeitswächter, Fenster mit Eingabe und CPU-Bildausgabe über einen eigenen X11-Client, Win32 und AppKit |
| M1 Erstes Dokument | abgeschlossen am 26.10.2026 | Lokale HTML- und XHTML-Dateien werden angezeigt, mit Scrollen und hohen Auflösungen. Alle 398 Referenztests aus `css/CSS2/normal-flow/` der WPT bestehen. |
| M2 JavaScript | läuft seit 30.10.2026 | Die Engine `mb-js` mit den Schritten 1 bis 11 des Plans, also bis zum Zwischenspeicher für Bytecode im Arbeitsspeicher und auf der Platte, Test262 siehe unten. WebIDL-Parser mit Namensindex und ersten Typ- und Memberprüfungen. |
| M3 Netzwerk | Vorarbeiten | Deflate und zlib (`mb-compress`), SHA-256 mit den Testvektoren des NIST (`mb-crypto`) |
| M4 Modernes Layout und GPU | Vorarbeiten | Scrollen des Viewports, Skalierung für hohe Auflösungen |
| M5 Alltagstauglich | Vorarbeiten | PNG und das erste Einzelbild von GIF, in `mb-storage` ein Zwischenspeicher auf der Platte (bisher für Bytecode) |
| M6 bis M10 | nicht begonnen | |

### Was schon geht

- **Seiten ohne Skripte:** `meinbrowser seite.html` zeigt eine lokale Datei mit Encoding (alle 40 Kodierungen des WHATWG Encoding Standard), URL, HTML-Parser, XML und XHTML, Dokumentspeicher, CSS mit Selektoren, Kaskade, `@media`, `@supports`, `@import` und berechneten Werten, Block- und Inline-Layout, Systemschriften (TrueType und OpenType) mit einfacher Textformung, PNG und GIF sowie dem CPU-Renderer. Die Tokenizer-Tests aus html5lib-tests bestehen bis auf `xmlViolation.test`, die Baumaufbau-Tests bis auf vier Dateien, die laufende Skripte erwarten; alle URL-Testdaten der WPT bestehen.
- **JavaScript (`mb-js`):** eigener Lexer und Parser für Scripts, Module und reguläre Ausdrücke, Bytecode für eine Registermaschine mit Interpreter und ein kopierender Garbage Collector mit zwei Generationen und Schreibbarriere. Die Sprache reicht bis zu Klassen mit privaten Feldern, Generatoren, `async` und `await`, async-Generatoren, Destrukturierung, `eval`, `with` und Modulen mit `import()` und `await` auf oberster Ebene. Eingebaute Objekte ohne Intl: `Object`, `Function`, `Array`, `String`, `RegExp` mit eigener Engine, `Number`, `BigInt`, `Math`, `Date`, `JSON`, `Symbol`, `Boolean`, `Promise`, `Iterator` mit den Helfern, `Map`, `Set`, `WeakMap`, `WeakSet`, `WeakRef`, `FinalizationRegistry`, `ArrayBuffer`, `SharedArrayBuffer`, `DataView`, die typisierten Arrays, `Atomics`, `Reflect`, `Proxy` und die Fehlerobjekte. Übersetzter Bytecode von Scripts lässt sich speichern und nach strenger Prüfung ohne Syntaxbaum wieder ausführen, mit einem Zwischenspeicher im Arbeitsspeicher oder auf der Platte (`mb-storage`, mit einer Grenze für die Bytes, die den am längsten ungenutzten Eintrag verdrängt). Kopf und Schlüssel enthalten eine Kennung des Baus der Engine, Einträge eines anderen Baus passen also nicht. Module und Code aus `eval` und `Function` werden noch nicht gespeichert, und das Programm `meinbrowser` nutzt den Speicher noch nicht, weil Seiten noch keine Skripte ausführen.
- **Kryptografie (`mb-crypto`):** SHA-256 nach FIPS 180-4 hinter einer schmalen Anbieter-Schnittstelle, geprüft mit den Testvektoren des NIST aus CAVP und ACVP. Weitere Verfahren und TLS fehlen noch.
- **WebIDL (`mb-webidl`):** Tokenizer, Syntaxparser, Namensindex über mehrere Quellen und erste semantische Prüfungen für Typen und Member, aufrufbar mit `webidl-check`.
- **Absicherung:** Abhängigkeitswächter, 24 Fuzz-Ziele bei Pull Requests mit Codeänderungen und nachts, Arbeits- und Bytebudgets für Seiten, ein Schrittbudget für JavaScript und nachts Belastungstests für den Garbage Collector, Test262 über die Zwischenspeicher für Bytecode und große Nachrichten bei SHA-256.

### Test262

Alle 48.517 Syntaxläufe bestehen. Ausgeführt werden die Teile, deren eingebaute Objekte es schon gibt:

| Bereich | Dateien bestanden | Varianten bestanden |
|---|---|---|
| `test/language/` | 23.089 von 23.296 | 43.431 von 43.794 |
| `test/built-ins/`, fertige Teile | 17.543 von 18.357 | 34.824 von 36.431 |
| `test/annexB/built-ins/` (`Date`, `Function`, `Object`, `RegExp`, `String`) | 168 von 178 | 336 von 356 |
| zusammen | 40.800 von 41.831 | 78.591 von 80.581 |

Das sind gut 83 Prozent der 48.882 Testdateien ohne `intl402/` und `staging/`. Die Zahlen stehen in den Kopfzeilen der Listen erwarteter Fehlschläge unter `crates/mb-js/src/script/test262/`; welche Teile von `test/built-ins/` fertig sind, steht in `language.rs` daneben. Nachts laufen dieselben Dateien auch über den Zwischenspeicher für Bytecode im Arbeitsspeicher, `test/language/` zusätzlich über den auf der Platte, gegen dieselben erwarteten Fehlschläge.

### Was noch fehlt

- **Skripte in Seiten:** `mb-web` bindet `mb-js` noch nicht ein. Es fehlen der Binding-Generator für WebIDL, DOM-Bindings, die Ereignisschleife und ein WPT-Läufer für testharness.js.
- **In `mb-js`:** Schritt 12 (Vorprüfung der Funktionsrümpfe), Templates mit Tag, `using`, Funktionsdeklarationen mit Marke sowie unter anderem die Funktionen für URIs, `DisposableStack`, `Temporal` und `Intl`.
- **Alles ab M3:** Netzwerk und TLS, modernes Layout wie Flex, Grid, Floats, Positionierung und Tabellen, Ausgabe über die GPU, Prozessisolation und Sandbox, Speicherung, Oberfläche, Erweiterungen, Medien und WebAssembly. Die Crates dafür (`mb-net`, `mb-gpu`, `mb-shader`, `mb-ipc`, `mb-ui`, `mb-ext`, `mb-media`, `mb-wasm`, `mb-devtools`) sind noch leer, `mb-crypto` hat bisher nur SHA-256 und `mb-storage` nur den Zwischenspeicher auf der Platte.

Als Nächstes kommen Schritt 12 für `mb-js`, der Binding-Generator für WebIDL und Skripte im Dokument. Danach folgt M3.

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

Die Belastungstests laufen nachts über `.github/workflows/belastung.yml` im Release-Build, von Hand zum Beispiel mit `cargo test --locked --release -p mb-js --lib -- --ignored script::gc_tests::` für den Garbage Collector und `cargo test --locked --release -p mb-crypto --lib -- --ignored acvp_large_data` für die großen Nachrichten des ACVP bei SHA-256 (1 bis 8 GiB). Die Aufrufe für Test262 im Belastungsmodus und über die Zwischenspeicher für Bytecode stehen in [docs/m2-test262-ausfuehrung.md](docs/m2-test262-ausfuehrung.md#nächtliche-belastungstests).

WebIDL-Dateien gemeinsam prüfen: `cargo run --locked -p webidl-check -- api.idl dependencies.idl`. Geprüft werden Syntax, Namen und Verweise und erste semantische Regeln für Typen und Member; Bindings erzeugt das Werkzeug nicht. Umfang und noch fehlende Prüfungen stehen in [docs/m2-webidl-index.md](docs/m2-webidl-index.md).

Tabellen aus Rohdaten unter `data/` (zurzeit die Unicode-Zeichendatenbank mit der IDNA-Zuordnungstabelle, die Daten des WHATWG Encoding Standard, die benannten Zeichenreferenzen, die CSS-Blöcke aus dem Abschnitt „Rendering“ des HTML Standard und die normative WebIDL-Grammatik) erzeugt `cargo run -p datagen`. Die erzeugten Dateien werden eingecheckt.

Externe Testsammlungen (zurzeit Teile der WPT, html5lib-tests, Testdateien von Unicode, css-parsing-tests, webref, die XML-Testsuite des W3C, Deflate-Testdaten aus Go, die PNG-Testbilder von image-png, Teile von Test262 und die Testvektoren des NIST für SHA-256 aus ACVP-Server und pyca/cryptography) liegen nicht im Repository. `cargo run -p testdata` holt sie mit `git` in der Version, die `tools/testdata/src/lib.rs` festlegt, nach `target/testdata/`.
