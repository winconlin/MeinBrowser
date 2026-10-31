# Parser und JavaScript-Laufzeit mit libFuzzer prüfen

Parser und vorhandene Teile der JavaScript-Laufzeit werden mit `cargo-fuzz` und libFuzzer geprüft. `libfuzzer-sys` ist dafür als einzige Ausnahme von der Regel „keine fremden Crates“ zugelassen (entschieden am 30.10.2026, siehe `docs/architektur.md`, Abschnitt 14). Die Ausnahme gilt nur für dieses Entwicklungswerkzeug; weitere fremde Crates deckt sie nicht.

Das Werkzeug liegt in einem eigenen Cargo-Workspace unter `fuzz/`, mit eigenem Lockfile. Die Browser-Crates und ihr Lockfile bleiben frei von fremden Abhängigkeiten; der normale Abhängigkeitswächter bleibt aktiv. Er prüft auch `fuzz/Cargo.lock`: Fremd darf dort nur sein, was `libfuzzer-sys` von crates.io über die Abhängigkeiten in dieser Datei mitbringt.

libFuzzer verändert Eingaben anhand der gemessenen Codeabdeckung. Die Ziele prüfen die echten Parser und zusätzliche Eigenschaften, beispielsweise begrenzte Dekoderausgaben. Das Ziel `js-runtime` prüft außerdem Operationsfolgen auf der vorhandenen JavaScript-Laufzeit, `js-execute` die neue Compiler-/Interpreterpipeline. Abstürze, verletzte Zusicherungen und überschrittene Ressourcenlimits lassen den Lauf fehlschlagen. Ein erfolgreicher begrenzter Lauf beweist keine Fehlerfreiheit.

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

Das Fuzzing-Lockfile legt die vollständigen Entwicklungsabhängigkeiten fest, darunter `libfuzzer-sys` 0.4.13. `cargo-fuzz` 0.13.2 hat keinen `--locked`-Schalter: Die Abhängigkeiten werden deshalb zuerst mit `cargo fetch --locked` geholt, anschließend wird offline gebaut. Der Läufer prüft das Lockfile ebenfalls und führt cargo-fuzz offline aus. Toolchain, Werkzeug und Lockfile gemeinsam prüfen, wenn sie aktualisiert werden. Der Build instrumentiert die geprüften Komponenten für libFuzzer und AddressSanitizer und aktiviert Debug-Zusicherungen sowie Überlaufprüfungen; dafür wird kein Browserfenster oder X-Server benötigt.

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
| `js` | JavaScript-Lexer und -Parser für Scripts und Module und der Parser für Muster regulärer Ausdrücke, mit der Eingabe als UTF-8 und als UTF-16LE |
| `js-runtime` | Heap-Graphen, gewöhnliche Objekte, Arrays und primitive numerische Umwandlungen; unabhängige Modelle mit normalen und erzwungenen GC-Zeitpunkten |
| `js-execute` | Beliebige Quellen nur parsen und übersetzen; ausschließlich begrenzte Vorlagenprogramme mit normalen und erzwungenen GC-Zeitpunkten ausführen |

Die eingecheckten Startkorpora liegen unter `fuzz/seeds/ZIEL/`. Der Läufer kopiert sie in das veränderliche, ignorierte Arbeitskorpus `fuzz/corpus/ZIEL/`. Nach dem Einrichten der Werkzeuge und Abhängigkeiten brauchen die Testläufe keine externen Testsammlungen oder Netzverbindungen. Generierte Korpora, Rohprotokolle, Buildausgaben und Fehlerdateien bleiben in ignorierten lokalen Verzeichnissen.

Der Läufer begrenzt die Eingaben auf 4 KiB, beim Schriftziel auf 64 KiB. Deflate/zlib darf höchstens 64 KiB Ausgabe erzeugen; PNG und GIF werden mit höchstens 4.096 Pixeln dekodiert. Pro Eingabe gelten zehn Sekunden und für den Fuzzer eine RSS-Grenze von 512 MiB. Das RSS-Limit wird von libFuzzer überwacht und ist keine Betriebssystem-Sandbox. Der festgelegte Zufallsseed erleichtert den Vergleich von Läufen; Laufzeiten und die erzeugten Korpora können trotzdem variieren.

Beim Ziel `page` ist die Eingabe eine kleine Website. Je zwei Bytes 0x1F trennen ihre Teile: Teil 0 ist das Dokument, und jede URL unter `https://example.test/`, deren Pfad mit einer Zahl N beginnt, liefert Teil N, etwa `1.css` oder `2.png`. Beginnt das Dokument mit `<?xml`, wird es als XHTML gelesen, sonst als HTML. Die Seite darf zusammen 1 MiB laden und dekodieren. Stil und Layout dürfen je Viewport eine Million Arbeitsschritte kosten; die Viewports sind 800 × 600 CSS-Pixel und 320 × 480 mit zwei Gerätepixeln je CSS-Pixel, gezeichnet wird beim zweiten am Ende des Scrollbereichs. Die einzige Schrift ist die synthetische aus dem Startkorpus von `font`. Mit AddressSanitizer brauchte die langsamste von mehreren gemessenen 4-KiB-Eingaben (eine Tabelle mit 400 Zeilen) zwei Sekunden.

Beim Ziel `js` wird die Eingabe zweimal gelesen: als UTF-8-Text und als Folge von UTF-16-Codeeinheiten in Little Endian, damit auch einzelne Surrogate den Lexer erreichen. Jede Fassung durchläuft den Lexer mit vier festen Zielen, als Script und als Modul, und den Parser als Script und als Modul. Geprüft wird, dass Tokens und Anweisungen in der Reihenfolge des Quelltexts und innerhalb seiner Grenzen liegen und dass jedes Token außer dem Ende Eingabe verbraucht. Läuft der Stapel bei tiefer Verschachtelung über, schlägt der Lauf fehl; der Parser muss dann `TooDeep` melden. Außerdem liest der Parser für Muster regulärer Ausdrücke jede Fassung als Muster ohne Flags, mit `u` und mit `v`; ein Muster muss eine Disjunktion ergeben, ein Fehler innerhalb der Eingabe liegen. Gruppen und Klassen liest dieser Parser ohne Rekursion, tiefe Verschachtelung darf ihn also nicht scheitern lassen.

Beim Ziel `js-runtime` wählt das erste Byte modulo vier den Modus: 0 prüft einen Heap-Graphen, 1 gewöhnliche Objekte, 2 Arrays und 3 primitive numerische Umwandlungen. Je vier weitere Bytes beschreiben eine Operation; ein unvollständiger Rest bleibt unbenutzt. Die gesamte Eingabe ist wie bei den Parserzielen auf 4 KiB begrenzt. Jeder Ablauf wird mit normaler Sammlung und im GC-Belastungsmodus ausgeführt; der Heap-Prüfmodus ist in beiden eingeschaltet. Pro Ausführung gilt ein Heapbudget von 1 MiB. Dieses Budget erfasst den logischen Heap, nicht den gesamten Prozessspeicher; zusätzlich gelten die oben genannten Laufzeit- und RSS-Grenzen.

Der Graphmodus verarbeitet höchstens 128 Operationen mit bis zu 16 dauerhaften Wurzeln und 128 Modellknoten. Zeichenketten enthalten höchstens 16 UTF-16-Codeeinheiten, Feldzellen höchstens vier Plätze. Ein unabhängiges Graphmodell prüft Inhalte und gemeinsam referenzierte Zellen. Nach expliziter Sammlung werden die erreichbaren Zellbytes gegen das Modell geprüft; nach Freigabe aller Wurzeln muss der Heap leer sein. Lokale Handles entstehen in kurzen Bereichen, damit nicht versehentlich alle Zwischenwerte bis zum Ende verwurzelt bleiben.

Der Objektmodus verarbeitet höchstens 64 Operationen auf vier dauerhaft verwurzelten gewöhnlichen Objekten und 16 festen Index-, Zeichenketten- und Symbolschlüsseln. Geprüft werden Dateneigenschaften mit ihren Attributen, Schreiben, Löschen, das Verhindern neuer Eigenschaften und Änderungen der Prototypkette einschließlich der Ablehnung von Zyklen. Ein eigenes Modell prüft Ergebnisse, Deskriptoren, Schlüsselreihenfolge sowie Lesen und Existenzprüfung entlang der Prototypkette. Zugriffseigenschaften und Funktionsaufrufe sind nicht Teil dieses Ziels.

Der Arraymodus verarbeitet höchstens 64 Operationen auf zwei verwurzelten Arrays mit elf festen Schlüsseln. Ein eigenes Modell prüft Indexanlage, Datendeskriptoren, Löschen, Erweiterbarkeit und die Eigenschaft `length` einschließlich ihres Schreibschutzes. Beim fehlgeschlagenen Schrumpfen werden bereits gelöschte höhere Elemente, der blockierende nicht konfigurierbare Index und die verbleibende Länge geprüft. Große Indizes werden als vorhandene Eigenschaften betrachtet, ohne Schleifen über Lücken. 29 Längeneingaben enthalten numerische Grenzen, negative Null, NaN, Unendlichkeiten, Strings, Null, Undefined, Booleans und Symbole; erwartete RangeError- und TypeError-Ausnahmen werden geprüft und entnommen.

Der Konversionsmodus verarbeitet höchstens 64 Operationen mit primitiven Werten und höchstens 32 UTF-16-Codeeinheiten je erzeugter Zeichenkette. Er prüft `ToNumber`, `ToNumeric`, `ToBoolean`, `ToIntegerOrInfinity`, `ToUint32`, `ToInt32` und bekannte `StringToNumber`-Fälle. Zahlenpaletten und veränderliche IEEE-754-Bitmuster erreichen unter anderem NaN, beide Nullen, Unendlichkeiten und Ganzzahlgrenzen. Das unabhängige Uint32-Orakel arbeitet mit den IEEE-Bits. Generierte kleine Integer und exakte Binärbrüche liefern berechenbare Erwartungen für Dezimal- und Radixstrings, ergänzt um Leerraum und bekannte fehlerhafte Syntax einschließlich Surrogaten. Der Modus verwendet keinen zweiten Floatparser als Orakel. Objekt-ToPrimitive, ToString und ToPropertyKey sind nicht abgedeckt. Die [Übergabe für die M2-Prüfung](m2-runtime-fuzzing.md) beschreibt die Abgrenzung und den weiteren Ausbau.

Beim Ziel `js-execute` bleibt die Eingabe auf 4 KiB begrenzt. Der Compilerteil liest sie als UTF-8 mit Ersetzungszeichen und als UTF-16LE, parst Scripts und übersetzt die unterstützten Konstrukte. Syntaxfehler und noch nicht unterstützte Konstrukte sind normale Ergebnisse. Erfolgreich erzeugter Bytecode wird vollständig dekodiert; Register- und Konstantenoperanden, Sprungziele auf Befehlsanfänge sowie vorhandene Quelltextpositionen werden geprüft. Beliebige Eingabequellen werden dabei niemals ausgeführt.

Der Ausführungsteil erzeugt aus den Eingabebytes ausschließlich feste, begrenzte Programmvorlagen mit bekannten Ergebnissen. Die Bytes werden nicht als JavaScript-Quelltext eingefügt. Erzeugte Schleifen haben höchstens acht Durchläufe. Jeder Fall läuft mit normalen und erzwungenen GC-Zeitpunkten, aktiviertem Heap-Prüfmodus und einem logischen Heapbudget von 1 MiB. Ergebnisse und erwartete Ausnahmen werden gegen unabhängige Erwartungen geprüft; Folge-Scripts prüfen erhaltenen Zustand nach Fehlern, und anschließend muss der Heap gültig sein. Die Registerstapeltiefe ist außerhalb von `mb-js` nicht öffentlich abfragbar; ihre direkte Prüfung bleibt bei den internen Engine-Tests. Das Heapbudget ist keine Grenze für den Prozess-RAM. Der Interpreter besitzt noch kein eigenes Schrittbudget, deshalb bleiben beliebige Quellen auf den Compilerteil beschränkt. Die [Übergabe zur Script-Prüfung](m2-script-fuzzing.md) beschreibt Vorlagen, Grenzen und Prüfpunkte.

Die Grenzen halten kurze Prüfläufe praktikabel. Sie decken keine beliebig großen Dokumente oder alle möglichen Ressourcenangriffe ab. Die Ziele laden keine Ressourcen aus Dateien oder dem Netz; `page` holt Stylesheets und Bilder aus seiner Eingabe. TLS und IPC erhalten Fuzzing-Ziele, sobald ihre Implementierungen vorhanden sind; von der JavaScript-Engine prüft `js` Lexer, Parser und die Muster regulärer Ausdrücke, `js-runtime` die vorhandenen Laufzeitbausteine und `js-execute` Bytecode-Compiler und Interpreter für den beschriebenen begrenzten Umfang. Funktionsrümpfe übersetzt der Compiler erst bei ihrem ersten Aufruf; der Compilerteil prüft deshalb nur den Code der Scripts, und die Vorlagen enthalten keine Funktionen. Die generierten Vorlagen sind kein vollständiger Test aller unterstützten Sprachkonstrukte. Die Vergleiche eigener Ausführungswege sind kein Nachweis der ECMAScript-Konformität. Konformitätstests und Regressionstests bleiben zusätzlich erforderlich. Die eigenen Startkorpora lassen sich mit `python3 fuzz/seed_corpus.py` neu erzeugen; sie enthalten keine fremden Schriften.

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
