# M2: Laufzeit-Fuzzing zur Prüfung

Diese Vorarbeit ergänzt die GC-Belastungstests aus [Architektur, Abschnitt 8.3](architektur.md#83-sicherheit-in-jedem-meilenstein) und das Fuzzing weiterer JavaScript-Stufen aus Abschnitt 8.2, Punkt 14. Grundlage sind Heap, Agent und gewöhnliche Objekte auf `main` nach PR #66; die Engine-Dateien bleiben unverändert. Das Ziel ist `js-runtime` im separaten Entwicklungsworkspace `fuzz/`.

## Was geprüft wird

Das erste Eingabebyte wählt einen von zwei Modi. Der Rest enthält Vierergruppen als Operationen, keine JavaScript-Anweisungen. Jede Folge läuft mit normalen und mit erzwungenen GC-Zeitpunkten (`stress: true`), jeweils mit aktivierter Heap-Prüfung. Beide Ausführungen müssen zu ihrem unabhängigen Modell passen. Verglichen werden Inhalte und beobachtbares Verhalten; GC-Abstände zwischen unterschiedlichen Ausführungen sind keine Identitäten.

- **Gerades erstes Byte: Heap-Graph.** Zeichenketten und Felder bilden einen kleinen Graphen, einschließlich gemeinsamer Ziele und Zyklen. Ein Rust-Modell führt die Knoten und Wurzeln unabhängig vom Heap. Nach einer Sammlung müssen Inhalte, Verweisidentitäten innerhalb des Graphen und die Bytezahl der erreichbaren Zellen stimmen. Nach Freigabe aller Wurzeln und erneuter Sammlung müssen null Zellbytes übrig sein.
- **Ungerades erstes Byte: gewöhnliche Objekte.** Ein Modell führt vollständige Datendeskriptoren, Erweiterbarkeit und Prototypen. Es prüft die Ergebnisse von Definieren, Schreiben, Löschen, Prototypwechsel und `PreventExtensions`, dazu eigene Eigenschaften, vererbtes Lesen, `HasProperty` und die Reihenfolge eigener Schlüssel. Die Schlüssel enthalten kleine und große Array-Indizes, nicht kanonische Zahlzeichenketten, einen einzelnen Surrogaten und zwei unterschiedliche Symbole.

Lokale Handles entstehen in kurzen Bereichen. Nur die ausdrücklich vorgesehenen `Persistent`-Wurzeln bleiben über Operationen hinweg bestehen. Dadurch kann der Collector aufgegebene Daten tatsächlich freigeben; ein einziger großer Bereich würde die erzeugten lokalen Handles bis zum Schluss festhalten.

## Grenzen

Es gelten höchstens 4 KiB Eingabe und 1 MiB logisches Heapbudget pro Ausführung. Der Heap-Modus verarbeitet höchstens 128 Operationen mit 16 Wurzelslots, höchstens 128 neuen Knoten, höchstens 16 UTF-16-Codeeinheiten je Zeichenkette und vier Werten je Feldzelle. Der Objektmodus verarbeitet höchstens 64 Operationen mit vier verwurzelten Objekten und 16 festen Schlüsseln. Unvollständige Vierergruppen und weitere Operationen werden ignoriert. Der vorhandene Läufer begrenzt zusätzlich Zeit und RSS; Details stehen in [fuzzing.md](fuzzing.md).

Die Modellfälle sind so klein, dass Speicherfehler unerwartet sind und den Lauf fehlschlagen lassen. Der Lauf prüft weder den gesamten Speicherverbrauch des Prozesses noch die Vollständigkeit aller Out-of-Memory-Pfade.

Das Graphmodell setzt das aktuelle Zellformat und getrennte Zellen für getrennte Zeichenkettenallokationen voraus. Ändern sich das Zellformat oder das Interning in `Heap::alloc_string`, muss diese Annahme zusammen mit der Byteberechnung geprüft werden. Das Objektmodell vergleicht Zeichenketten nach ihrem Inhalt.

Zugriffseigenschaften, Funktionsaufrufe, Arrays als exotische Objekte, Realms, Bytecode und Interpreter gehören noch nicht zum Modell. Der Vergleich verschiedener GC-Zeitpunkte derselben Engine ist keine zweite ECMAScript-Implementierung und kein vollständiger Konformitätsnachweis. Test262 und die gezielten Engine-Regressionstests bleiben erforderlich.

## Lokal prüfen

Nach dem Einrichten der festgelegten Fuzzing-Werkzeuge:

```sh
cargo fmt --manifest-path fuzz/Cargo.toml --all --check
cargo fetch --locked --manifest-path fuzz/Cargo.toml
cargo test --locked --offline --manifest-path fuzz/Cargo.toml --lib
cargo clippy --locked --offline --manifest-path fuzz/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s fuzz -p test_run.py
CARGO_NET_OFFLINE=true cargo +nightly-2026-10-27 fuzz build js-runtime
python3 fuzz/run.py js-runtime --seconds 30 --seed 1
```

Die selbst erstellten Startkorpora lassen sich mit `python3 fuzz/seed_corpus.py` neu erzeugen. Die CI nimmt das neue Ziel in dieselben kurzen PR-Läufe und nächtlichen Läufe auf. Rohprotokolle und Fehlerdateien bleiben gemäß [SECURITY.md](../SECURITY.md) vertraulich.

## Prüfpunkte für Claude

1. Stimmen die Modelle für den dokumentierten Teilumfang mit ECMA-262 überein, insbesondere nicht konfigurierbare Dateneigenschaften, geerbte Schreibsperren, Prototypzyklen und Schlüsselreihenfolge?
2. Werden alle Werte nach möglichen Sammlungen aus gültigen Handles gelesen und aufgegebene persistente Wurzeln wirklich freigegeben? Prüfen: `fuzz/src/runtime/heap_graph.rs` und `objects.rs`.
3. Erreichen die Startkorpora die beabsichtigten GC- und Objektpfade, und schlagen Modellabweichungen als Fehler bis zu libFuzzer durch?
4. Bleiben API-Aufrufe gültig, wenn die nächste M2-Stufe hinzukommt? Dann das Modell gezielt um deren Semantik erweitern; Zusicherungen bei Funden beibehalten und die Ursache klären.

Dieser Schritt schließt M2 nicht ab. Er liefert ein wiederholbares Prüfgerüst für die bereits vorhandene Laufzeit, auf dem die weiteren Stufen aufbauen können.
