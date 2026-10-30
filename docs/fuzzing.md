# Parser mit libFuzzer prüfen

Die Parser werden mit `cargo-fuzz` und libFuzzer geprüft. `libfuzzer-sys` ist dafür als einzige Ausnahme von der Regel „keine fremden Crates“ zugelassen (entschieden am 30.10.2026, siehe `docs/architektur.md`, Abschnitt 14). Die Ausnahme gilt nur für dieses Entwicklungswerkzeug; weitere fremde Crates deckt sie nicht.

Das Werkzeug liegt in einem eigenen Cargo-Workspace unter `fuzz/`, mit eigenem Lockfile. Die Browser-Crates und ihr Lockfile bleiben frei von fremden Abhängigkeiten; der normale Abhängigkeitswächter bleibt aktiv. Er prüft auch `fuzz/Cargo.lock`: Fremd darf dort nur sein, was `libfuzzer-sys` von crates.io über die Abhängigkeiten in dieser Datei mitbringt.

libFuzzer verändert Eingaben anhand der gemessenen Codeabdeckung. Die Ziele prüfen die echten Parser und zusätzliche Eigenschaften, beispielsweise begrenzte Dekoderausgaben. Abstürze, verletzte Zusicherungen und überschrittene Ressourcenlimits lassen den Lauf fehlschlagen. Ein erfolgreicher begrenzter Lauf beweist keine Fehlerfreiheit.

## Einrichten und starten

Unter Linux werden Rust/rustup, ein C++-Compiler (`g++` oder `clang++`) und Python 3 benötigt. Die CI verwendet Linux. Die Fuzzing-Toolchain ist getrennt von der stabilen Browser-Toolchain:

```sh
rustup toolchain install nightly-2026-10-27 --profile minimal
cargo install cargo-fuzz --version 0.13.2 --locked
```

Alle folgenden Befehle werden im Wurzelverzeichnis des Repositories ausgeführt:

```sh
python3 fuzz/run.py --list
cargo fetch --locked --manifest-path fuzz/Cargo.toml
CARGO_NET_OFFLINE=true cargo +nightly-2026-10-27 fuzz build
python3 fuzz/run.py html --seconds 30
```

Das Fuzzing-Lockfile legt die vollständigen Entwicklungsabhängigkeiten fest, darunter `libfuzzer-sys` 0.4.13. `cargo-fuzz` 0.13.2 hat keinen `--locked`-Schalter: Die Abhängigkeiten werden deshalb zuerst mit `cargo fetch --locked` geholt, anschließend wird offline gebaut. Der Läufer prüft das Lockfile ebenfalls und führt cargo-fuzz offline aus. Toolchain, Werkzeug und Lockfile gemeinsam prüfen, wenn sie aktualisiert werden. Der Build instrumentiert die Parser für libFuzzer und AddressSanitizer und aktiviert Debug-Zusicherungen sowie Überlaufprüfungen; dafür wird kein Browserfenster oder X-Server benötigt.

## Ziele und Grenzen

| Ziel | Geprüfter Einstieg |
| --- | --- |
| `html` | HTML-Tokenizer und Dokumentaufbau ohne Skriptausführung, mit der Kodierung aus den Bytes (Byte-Order-Mark, Vorab-Lesen, `meta`) |
| `css` | CSS-Syntax, Stylesheets, Selektoren und Eigenschaften |
| `url` | URL-Parser und Serialisierung |
| `xml` | XML-/XHTML-Parser, einschließlich interner Entitäten |
| `png` | PNG-Dekoder und Umwandlung nach RGBA8 |
| `gif` | Erstes GIF-Einzelbild mit LZW, Paletten und Interlacing |
| `deflate` | Deflate- und zlib-Dekoder |
| `font` | sfnt-/OpenType-Leser und begrenzte Glyphenabfragen |
| `json` | JSON-Parser |
| `encoding` | Zeichenkodierungen und Streaming-Dekodierung |
| `unicode` | Normalisierung und weitere vorhandene Unicode-Algorithmen |
| `page` | Eine Seite mit Stylesheets und Bildern laden, Kaskade, Layout und Zeichnen in zwei Viewports |

Die eingecheckten Startkorpora liegen unter `fuzz/seeds/ZIEL/`. Der Läufer kopiert sie in das veränderliche, ignorierte Arbeitskorpus `fuzz/corpus/ZIEL/`. Nach dem Einrichten der Werkzeuge und Abhängigkeiten brauchen die Testläufe keine externen Testsammlungen oder Netzverbindungen. Generierte Korpora, Rohprotokolle, Buildausgaben und Fehlerdateien bleiben in ignorierten lokalen Verzeichnissen.

Der Läufer begrenzt die Eingaben auf 4 KiB, beim Schriftziel auf 64 KiB. Deflate/zlib darf höchstens 64 KiB Ausgabe erzeugen; PNG und GIF werden mit höchstens 4.096 Pixeln dekodiert. Pro Eingabe gelten zehn Sekunden und für den Fuzzer eine RSS-Grenze von 512 MiB. Das RSS-Limit wird von libFuzzer überwacht und ist keine Betriebssystem-Sandbox. Der festgelegte Zufallsseed erleichtert den Vergleich von Läufen; Laufzeiten und die erzeugten Korpora können trotzdem variieren.

Beim Ziel `page` ist die Eingabe eine kleine Website. Je zwei Bytes 0x1F trennen ihre Teile: Teil 0 ist das Dokument, und jede URL unter `https://example.test/`, deren Pfad mit einer Zahl N beginnt, liefert Teil N, etwa `1.css` oder `2.png`. Beginnt das Dokument mit `<?xml`, wird es als XHTML gelesen, sonst als HTML. Die Seite darf zusammen 1 MiB laden und dekodieren. Stil und Layout dürfen je Viewport eine Million Arbeitsschritte kosten; die Viewports sind 800 × 600 CSS-Pixel und 320 × 480 mit zwei Gerätepixeln je CSS-Pixel, gezeichnet wird beim zweiten am Ende des Scrollbereichs. Die einzige Schrift ist die synthetische aus dem Startkorpus von `font`. Mit AddressSanitizer brauchte die langsamste von mehreren gemessenen 4-KiB-Eingaben (eine Tabelle mit 400 Zeilen) zwei Sekunden.

Die Grenzen halten kurze Prüfläufe praktikabel. Sie decken keine beliebig großen Dokumente oder alle möglichen Ressourcenangriffe ab. Die Ziele laden keine Ressourcen aus Dateien oder dem Netz; `page` holt Stylesheets und Bilder aus seiner Eingabe. JavaScript, TLS und IPC erhalten Fuzzing-Ziele, sobald ihre Implementierungen vorhanden sind. Konformitätstests und Regressionstests bleiben zusätzlich erforderlich. Die eigenen Startkorpora lassen sich mit `python3 fuzz/seed_corpus.py` neu erzeugen; sie enthalten keine fremden Schriften.

## Automatisierte Läufe

`.github/workflows/fuzz.yml` startet täglich, manuell und für relevante Pull Requests. Pull Requests erhalten kurze Läufe von 30 Sekunden pro Ziel, geplante und manuelle Läufe 600 Sekunden pro Ziel. Jedes Ziel besitzt einen eigenen Job; ein fehlschlagendes Ziel bleibt ein Fehler im Workflow. Die CI holt den Quelltext mit Git und benötigt keine fremden GitHub Actions.

Pull Requests verwenden Seed 1; geplante und manuelle Läufe einen aus der Run-ID abgeleiteten, wechselnden Seed. Der Läufer protokolliert den verwendeten Wert. Lokal kann er mit `--seed ZAHL` gewählt werden. Zusätzlich zur libFuzzer-Zeitgrenze beendet ein äußerer Watchdog den Prozessverbund nach der Laufzeit plus 120 Sekunden Reserve. Neue Rohlogs und Fehlerdateien werden mit privaten Zugriffsrechten angelegt; Core-Dumps sind abgeschaltet.

Die Fehlerbehandlung und Vertraulichkeit des Läufers werden mit `python3 -m unittest discover -s fuzz -p test_run.py` geprüft, auch in der CI. `cargo test --locked --offline --manifest-path fuzz/Cargo.toml --lib` prüft zusätzlich, dass strukturierte Binär-Seeds tatsächlich dekodiert werden und alle Startkorpora die Harnesses erreichen.

Die öffentliche Ausgabe enthält nur freigegebene Statusdaten und Zähler. libFuzzer kann Eingaben und Reproduktionsdaten in seinen normalen Ausgaben zeigen; deshalb werden seine vollständigen Ausgaben lokal aufgezeichnet und nicht direkt in das CI-Protokoll geschrieben.

## Funde vertraulich bearbeiten

Rohprotokolle liegen lokal unter `fuzz/logs/`, Reproduktionsdateien unter `fuzz/artifacts/`. Nicht in öffentliche Issues, Pull Requests, CI-Ausgaben oder öffentliche Artefakte kopieren. Für Sicherheitsfehler gilt der vertrauliche Meldeweg aus [SECURITY.md](../SECURITY.md).

Auf einem CI-Runner sind diese lokalen Dateien nur bis zum Aufräumen des Jobs vorhanden. Es gibt absichtlich keinen öffentlichen Upload; dauerhafte private Aufbewahrung muss gesondert eingerichtet werden. Einen fehlgeschlagenen CI-Lauf zunächst mit demselben Commit, Ziel, Werkzeugstand und Seed in einer privaten lokalen Umgebung wiederholen. Ein Fund kann von der genauen Laufdauer und dem entstandenen Korpus abhängen; ein Seed allein garantiert keine exakte Wiederholung.

Zum Bearbeiten eines lokalen Fundes die betroffene Datei in einer privaten Umgebung gezielt erneut ausführen; `ZIEL` und `DATEI` durch das tatsächliche Ziel und die lokale Reproduktionsdatei ersetzen:

```sh
CARGO_NET_OFFLINE=true cargo +nightly-2026-10-27 fuzz run ZIEL DATEI -- -runs=1 -timeout=10 -rss_limit_mb=512
```

Dieser direkte lokale Aufruf zeigt ungekürzte Ausgaben und gehört deshalb nicht in öffentliche CI-Protokolle. Die Ursache prüfen und anschließend eine geeignete minimierte Regression ergänzen. Vertrauliche Reproduktionsdaten erst nach Abstimmung über den Sicherheitsmeldeweg veröffentlichen. Parserfehler nicht durch das Verwerfen einer bekannten problematischen Eingabe oder das Abschalten einer Zusicherung verdecken.
