# M2: Laufzeit-Fuzzing zur Prüfung

Diese Vorarbeit ergänzt die GC-Belastungstests aus [Architektur, Abschnitt 8.3](architektur.md#83-sicherheit-in-jedem-meilenstein) und das Fuzzing weiterer JavaScript-Stufen aus Abschnitt 8.2, Punkt 14. Die ersten Modelle entstanden für Heap, Agent und gewöhnliche Objekte nach PR #66. Arrays sowie primitive numerische Umwandlungen und Umwandlungen in Text und Eigenschaftsschlüssel ergänzen die Prüfung. Die direkten Laufzeitmodelle liegen im Testwerkzeug; das Ausführungsbudget der Engine ist getrennt in [m2-ausfuehrungsbudget.md](m2-ausfuehrungsbudget.md) beschrieben. Das Ziel ist `js-runtime` im separaten Entwicklungsworkspace `fuzz/`.

## Was geprüft wird

Das erste Eingabebyte modulo fünf wählt einen Modus. Der Rest enthält Vierergruppen als Operationen, keine JavaScript-Anweisungen. Jede Folge läuft mit normalen und mit erzwungenen GC-Zeitpunkten (`stress: true`), jeweils mit aktivierter Heap-Prüfung. Beide Ausführungen müssen zum unabhängigen Modell oder zu berechenbaren Erwartungswerten passen. Verglichen werden Inhalte und beobachtbares Verhalten; GC-Abstände zwischen unterschiedlichen Ausführungen sind keine Identitäten.

- **Modus 0: Heap-Graph.** Zeichenketten und Felder bilden einen kleinen Graphen, einschließlich gemeinsamer Ziele und Zyklen. Ein Rust-Modell führt die Knoten und Wurzeln unabhängig vom Heap. Nach einer Sammlung müssen Inhalte, Verweisidentitäten innerhalb des Graphen und die Bytezahl der erreichbaren Zellen stimmen. Nach Freigabe aller Wurzeln und erneuter Sammlung müssen null Zellbytes übrig sein.
- **Modus 1: gewöhnliche Objekte.** Ein Modell führt vollständige Datendeskriptoren, Erweiterbarkeit und Prototypen. Es prüft die Ergebnisse von Definieren, Schreiben, Löschen, Prototypwechsel und `PreventExtensions`, dazu eigene Eigenschaften, vererbtes Lesen, `HasProperty` und die Reihenfolge eigener Schlüssel. Die Schlüssel enthalten kleine und große Array-Indizes, nicht kanonische Zahlzeichenketten, einen einzelnen Surrogaten und zwei unterschiedliche Symbole.
- **Modus 2: Arrays.** Ein eigenes Modell prüft Dateneigenschaften, Indexanlage und Löschen, die Länge, ihren Schreibschutz und das Verhindern neuer Eigenschaften. Beim fehlgeschlagenen Schrumpfen bestimmt der höchste blockierende, nicht konfigurierbare Index den Restzustand: Höhere Elemente sind bereits gelöscht, die Länge bleibt bei diesem Index plus eins. Große Lücken werden über vorhandene Eigenschaften modelliert. `4294967294` ist der letzte Index, `"4294967295"` ein normaler Stringschlüssel. Ungültige Längen, Symbolwerte und die Reihenfolge von Konversion und Schreibschutzprüfung erhalten eigene Fälle; erwartete Ausnahmen werden entnommen und geprüft.
- **Modus 3: primitive numerische Umwandlungen.** Geprüft werden `ToNumber`, `ToNumeric`, `ToBoolean`, `ToIntegerOrInfinity`, `ToUint32` und `ToInt32`. Zahlen enthalten unter anderem negative Null, NaN, Unendlichkeiten und Grenzen von Ganzzahlen. Das Uint32-Orakel zerlegt IEEE-754-Bits, statt den Moduloalgorithmus der Engine nachzubauen. Berechenbare Dezimal-, Binär-, Oktal- und Hexadezimalstrings sowie exakte Binärbrüche, Leerraum, fehlerhafte Syntax und einzelne Surrogate prüfen `StringToNumber`. Bei den numerischen Umwandlungen müssen Symbole einen TypeError liefern; `ToBoolean(Symbol)` ergibt true. Die erwarteten Stringwerte werden ohne den zu prüfenden Parser oder einen zweiten Floatparser berechnet.

- **Modus 4: primitive Text- und Schlüsselumwandlungen.** `ToString` und `ToPropertyKey` werden mit bekannten Erwartungen geprüft. Eine Palette enthält unter anderem -0, NaN, Unendlichkeiten, subnormale Zahlen, Grenzen der Exponentenschreibweise und Rundungsgleichstände. Generierte 24-Bit-Ganzzahlen und exakte Binärbrüche erhalten ihre erwarteten Dezimaltexte ohne Fließkommaformatter. Nicht kanonische Zahlzeichenketten, NUL, einzelne Surrogate und beliebige kurze UTF-16-Folgen bleiben unverändert. Für Array-Indizes prüft das Orakel die kanonische Ganzzahlschreibweise und die Grenze 4294967294, ohne den Indexleser der Engine aufzurufen. Gleichlautende Zeichenkettenschlüssel müssen nach GC dasselbe Atom bezeichnen. Symbole behalten ihre eigene Identität, auch bei gleicher Beschreibung; das abstrakte `ToString(Symbol)` muss einen TypeError liefern, der genau einmal entnommen wird. Die Sonderregel des JavaScript-Builtins `String(symbol)` ist nicht Gegenstand dieses Modells.

Lokale Handles entstehen in kurzen Bereichen. Nur die ausdrücklich vorgesehenen `Persistent`-Wurzeln bleiben über Operationen hinweg bestehen. Dadurch kann der Collector aufgegebene Daten tatsächlich freigeben; ein einziger großer Bereich würde die erzeugten lokalen Handles bis zum Schluss festhalten.

## Grenzen

Es gelten höchstens 4 KiB Eingabe und 1 MiB logisches Heapbudget pro Ausführung. Der Heap-Modus verarbeitet höchstens 128 Operationen mit 16 Wurzelslots, höchstens 128 neuen Knoten, höchstens 16 UTF-16-Codeeinheiten je Zeichenkette und vier Werten je Feldzelle. Die anderen Modi verarbeiten höchstens 64 Operationen: gewöhnliche Objekte mit vier verwurzelten Objekten und 16 festen Schlüsseln, Arrays mit zwei verwurzelten Arrays und elf festen Schlüsseln, numerische Umwandlungen mit Werten aus den vier Eingabebytes und höchstens 32 UTF-16-Codeeinheiten je erzeugter Zeichenkette; Text-/Schlüsselumwandlungen ebenfalls mit höchstens 64 Operationen und 32 UTF-16-Codeeinheiten je erzeugtem Text. Deren Zahlenpalette hat 32 Einträge, die Schlüsselpalette 21; beliebige erzeugte UTF-16-Folgen bleiben auf 16 Codeeinheiten begrenzt. Unvollständige Vierergruppen und weitere Operationen werden ignoriert. Der vorhandene Läufer begrenzt zusätzlich Zeit und RSS; Details stehen in [fuzzing.md](fuzzing.md).

Die Modellfälle sind so klein, dass Speicherfehler unerwartet sind und den Lauf fehlschlagen lassen. Der Lauf prüft weder den gesamten Speicherverbrauch des Prozesses noch die Vollständigkeit aller Out-of-Memory-Pfade.

Das Graphmodell setzt das aktuelle Zellformat und getrennte Zellen für getrennte Zeichenkettenallokationen voraus. Ändern sich das Zellformat oder das Interning in `Heap::alloc_string`, muss diese Annahme zusammen mit der Byteberechnung geprüft werden. Das Objektmodell vergleicht Zeichenketten nach ihrem Inhalt.

Zugriffseigenschaften, Funktionsaufrufe, `ToPrimitive` für Objekte, BigInt-Umwandlungen, Array-Prototypmethoden, Realms, Bytecode und Interpreter gehören nicht zu diesen Laufzeitmodellen. Die Textprüfung deckt feste Zahlenpaletten, kleine ganze Zahlen und exakte Binärbrüche ab, nicht die kürzeste Darstellung sämtlicher binary64-Bitmuster. Das separate Ziel `js-execute` prüft Compiler und begrenzte Vorlagenprogramme; Umfang und Prüfpunkte stehen in der [Übergabe zur Script-Prüfung](m2-script-fuzzing.md). Der Vergleich verschiedener GC-Zeitpunkte derselben Engine ist keine zweite ECMAScript-Implementierung und kein vollständiger Konformitätsnachweis. Test262 und die gezielten Engine-Regressionstests bleiben erforderlich.

## Text- und Schlüsselorakel

Der neue Modus steht in `fuzz/src/runtime/text_keys.rs`. Byte 0 jeder
Vierergruppe modulo zehn wählt Undefined, Null, Boolean, Zahlenpalette,
24-Bit-Ganzzahl, Binärbruch, Schlüsselpalette, kurze UTF-16-Folge, Symbol
oder eine generierte Zahlzeichenkette. Die übrigen drei Bytes bestimmen
Vorzeichen, Werte, Länge oder Schreibweise.

Für Binärbrüche wird eine vorzeichenbehaftete 16-Bit-Ganzzahl durch
`2^(Byte3 % 9)` geteilt. Die erwartete Dezimaldarstellung entsteht aus
Ganzzahldivision mit Rest, mit höchstens acht Nachkommastellen und ohne
überflüssige nachgestellte Nullen. In diesem begrenzten Wertebereich ist sie
auch die kürzeste binary64-Darstellung: Der Abstand einer kürzeren
Dezimalstelle übersteigt das Rundungsintervall deutlich. Weder der
Zahlenformatter der Engine noch ein zweiter Floatformatter berechnet die
Erwartung. Für die übrigen Zahlen gelten feste bekannte Texte oder die
Dezimalschreibweise der exakt darstellbaren kleinen Ganzzahlen.

Das Indexorakel liest den erwarteten Text mit Rusts Ganzzahlparser und
verlangt bytegleiche kanonische Dezimalschreibweise sowie einen Wert bis
4294967294. Dadurch bleiben etwa `"00"`, `"-0"`, `"1e0"`, `"1.0"` und
`"4294967295"` normale Zeichenkettenschlüssel. Es verwendet weder
`key::array_index` noch eine Konversion der Engine als Orakel.

Pro Operation werden Quellwert und Ergebnisse über bewegende Sammlungen
hinweg in lokalen Handles gehalten. Ein zweiter frischer String mit dem
unabhängig erwarteten Text muss denselben Schlüssel und bei einem Atom
dieselbe Identität ergeben. Symbolprüfungen erzeugen einen unabhängigen
zweiten Wert mit gleicher Beschreibung, zusätzlich zu Fällen ohne oder mit
leerer Beschreibung und einem bekannten Symbol. Verglichen werden
Identitäten ausschließlich innerhalb desselben aktuell geliehenen Heaps,
niemals Speicherpositionen zwischen Agenten oder über eine Sammlung hinweg.

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

1. Stimmen die Modelle für den dokumentierten Teilumfang mit ECMA-262 überein, insbesondere nicht konfigurierbare Dateneigenschaften, geerbte Schreibsperren, Prototypzyklen, Schlüsselreihenfolge und teilweise fehlgeschlagenes Schrumpfen von Arrays?
2. Werden alle Werte nach möglichen Sammlungen aus gültigen Handles gelesen und aufgegebene persistente Wurzeln wirklich freigegeben? Prüfen: `fuzz/src/runtime/heap_graph.rs`, `objects.rs`, `arrays.rs` und `conversions.rs`.
3. Erreichen die Startkorpora die beabsichtigten GC- und Objektpfade, und schlagen Modellabweichungen als Fehler bis zu libFuzzer durch?
4. Bleiben API-Aufrufe gültig, wenn die nächste M2-Stufe hinzukommt? Dann das Modell gezielt um deren Semantik erweitern; Zusicherungen bei Funden beibehalten und die Ursache klären.
5. Sind RangeError und TypeError im richtigen Schritt erwartet, einschließlich unterschiedlicher Fehlerpriorität von `DefineOwnProperty` und `Set` bei schreibgeschützter Länge? Prüfen: `arrays.rs`.
6. Sind Vorzeichen von Null, NaN-Vergleich und Ganzzahlgrenzen korrekt, und ist das IEEE-754-Orakel unabhängig von der Implementierung? Prüfen: `conversions.rs`.
7. Stimmen in `text_keys.rs` die begrenzte Langdivision, kanonischen Indexgrenzen und Sonderfälle von `ToString`/`ToPropertyKey`? Bleiben gleiche Atome und verschiedene gleich beschriebene Symbole nach GC richtig unterschieden? Objektkonversionen und weitere Wertebereiche sind mit eigenen Orakeln zu ergänzen.

Dieser Schritt schließt M2 nicht ab. Er liefert ein wiederholbares Prüfgerüst für die bereits vorhandene Laufzeit, auf dem die weiteren Stufen aufbauen können.
