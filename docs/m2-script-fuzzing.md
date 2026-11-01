# M2: Compiler- und Script-Fuzzing zur Prüfung

Das Ziel `js-execute` ergänzt die M2-Prüfung aus [Architektur, Abschnitt 8.3](architektur.md#83-sicherheit-in-jedem-meilenstein) für den nach PR #71 und #72 vorhandenen Bytecode-Compiler und Interpreter, erweitert um die Funktionsstufe aus PR #75. Es liegt im separaten Entwicklungsworkspace `fuzz/`. Der Interpreter verwendet die in [m2-ausfuehrungsbudget.md](m2-ausfuehrungsbudget.md) beschriebenen Hostquoten und weiterhin ausschließlich eigene Workspace-Crates. Die Modelle aus [m2-runtime-fuzzing.md](m2-runtime-fuzzing.md) prüfen die Laufzeitbausteine weiterhin direkt. Der Compilerteil berücksichtigt auch die Kontext- und Parameterformate aus PR #77 und #79.

## Beliebige Eingaben übersetzen

Jede Eingabe wird einmal als UTF-8 mit Ersetzungszeichen und einmal als UTF-16LE gelesen; ein letztes einzelnes Byte der UTF-16-Fassung bleibt unbenutzt. Beide Fassungen durchlaufen den Script-Parser und bei Erfolg den Compiler. Syntaxfehler, noch nicht unterstützte Konstrukte und eine Überschreitung der Compilergrenzen sind normale Ergebnisse. Fehlerpositionen müssen innerhalb der jeweiligen UTF-16-Quelle liegen.

Ein unabhängiger Checker prüft erfolgreich erzeugten Bytecode:

- Alle Bytes sind durch vollständig dekodierbare Befehle abgedeckt; das Script endet mit `Return`.
- Operanden entsprechen ihrer Little-Endian-Kodierung und Vorzeichenerweiterung, einschließlich `Wide` und `ExtraWide`.
- Register und Konstanten liegen innerhalb des Rahmens beziehungsweise Konstantenpools. Parameterregister müssen in den Rahmen passen; das Registerfenster eines Aufrufs muss Funktion, `this` und alle Argumente enthalten.
- Sprünge bleiben im Code und treffen Befehlsanfänge. Ein `JumpLoop` darf bei einer leeren Schleife auf sich selbst zeigen.
- Vorhandene Quelltextpositionen liegen innerhalb der Quelle. Eine fehlende Position, etwa im Prolog eines leeren Scripts, ist zulässig.

Eine iterative, nach Funktions-ID, Namensbindung und struktureller Gleichheit des vollständigen äußeren Kontexts deduplizierte Warteschlange folgt zusätzlich erreichbaren Funktionsvorlagen: globalen Funktionsdeklarationen und `Constant::Function` im erzeugten Code. Die öffentliche `compile_function`-API übersetzt ihre Rümpfe ohne Aufruf; der Checker prüft den erzeugten Code ebenso. Noch nicht unterstützte Rümpfe, etwa mit Generatoren, Async-Funktionen oder `try`/`catch`, sind normale Compilerergebnisse mit geprüfter Fehlerposition. Unterstützte Closures, Pfeilfunktionen, `arguments`, Vorgabe- und Restparameter werden auch im Compilerteil geprüft; Restparameter zählen nicht zu den positionalen Parameterregistern. Nicht unterstützte übergeordnete Konstrukte können verhindern, dass innere Vorlagen überhaupt sichtbar werden.

Dieser Compilerteil führt den Eingabetext nicht aus. Auch Quelltexte mit nicht endenden Schleifen werden hier ausschließlich übersetzt. Der zusätzliche budgetierte Ausführungsteil ist unten beschrieben. Geprüft wird vom Compiler erzeugter Bytecode, kein Loader für beliebige gespeicherte Bytecode-Dateien.

## Begrenzte Vorlagen ausführen

Zusätzlich erzeugen die ersten vier Eingabebytes einen Fall aus 32 festen Programmfamilien. Byte 0 modulo 32 wählt die Familie; fehlende Bytes sind null. Zahlenparameter sind als `i8` begrenzt. Schleifengrenzen entstehen aus Byte 3 modulo neun. Zeichenkettenparameter werden ausschließlich als UTF-16-Hexescapes eingesetzt; roher Eingabetext wird nie in ausführbaren Quelltext eingebaut.

| Familien | Geprüfter Teilumfang |
| --- | --- |
| 0 | Ganzzahlarithmetik und Abschlusswert, einschließlich des Vorzeichens von Null |
| 1–2 | Summenschleifen mit null bis acht Durchläufen sowie `break` und `continue` |
| 3–5 | Blockbindungen, Schreibschutz von `const` und TDZ-Fehler |
| 6 | Optional Chaining und Nullish Coalescing mit beobachtbarer Auswertungsreihenfolge |
| 7–9 | Objekt- und Arrayliterale, Property-Zugriffe, zusammengesetzte Zuweisung, `delete`, `in` und Arraylänge |
| 10–12 | UTF-16-Zeichenketten und explizit geworfene Zahlen oder Zeichenketten |
| 13–15 | Strikte fehlerhafte Schreibzugriffe, Seiteneffekte vor einem Fehler und mehrere Scripts im selben Realm |
| 16–19 | Wiederaufrufe mit gecachtem Funktionscode, fehlende/zusätzliche/doppelte Parameter und globale beziehungsweise lokale Bindungen |
| 20–24 | Methoden- und abgetrennte Aufrufe, striktes `this`, optionale Aufrufe, benannte Funktionsausdrücke und ausdrücklich gebundenes `arguments` |
| 25 | Rekursive Summe mit absteigendem Parameter 0 bis 8 und veränderlichem globalem Wert |
| 26–28 | UTF-16-Rückgaben und geworfene Werte, Getter/Setter, Funktionsmetadaten und `instanceof` ohne `new` |
| 29–31 | Argumentauswertung vor TypeError, Folgeaufrufe nach Fehlern, lokale Funktionsdeklarationen und zurückgegebene Funktionen ohne eingefangene Bindungen |

Jeder Fall läuft in einem frischen Agent und Realm, einmal mit normalen und einmal mit erzwungenen GC-Zeitpunkten (`stress: true`), jeweils mit aktivierter Heapprüfung. Die erwarteten primitiven Ergebnisse und Fehler stammen aus kleinen mathematischen Orakeln beziehungsweise den festen Vorlagen. Geworfene Werte und normale Ergebnisse werden nach einer Sammlung aus gültigen Handles gelesen. Folge-Scripts prüfen unter anderem den erhaltenen Zustand nach einer gescheiterten Zuweisung oder Deklaration. Ausnahmen müssen genau einmal entnommen werden, der Heap muss nach jeder Ausführung gültig sein.

## Grenzen und Annahmen

Die Eingabe bleibt auf 4 KiB begrenzt. Pro Fall entstehen höchstens vier Scripts mit jeweils höchstens 2.048 UTF-16-Codeeinheiten. Jede erzeugte Schleife hat nach ihrer Vorlagensemantik höchstens acht Durchläufe. Die einzige rekursive Vorlage startet mit `n = Byte3 % 9` und vermindert `n` bis null: höchstens acht rekursive Abstiege beziehungsweise neun gleichzeitig aktive Funktionsaufrufe, zuzüglich Script-Rahmen. Die Vorlagen bleiben ein eigener Teil mit bekannten Ergebnissen. Zusätzlich werden beliebige Eingabequellen unter einer eigenen strikten Hostquote ausgewertet. Der vorhandene Läufer beendet außerdem langsame Eingaben und begrenzt die RSS wie in [fuzzing.md](fuzzing.md) beschrieben. Ein fehlerhafter Interpreter kann auch bei einer begrenzten Vorlage hängen; das ist ein fehlgeschlagener Fuzzing-Lauf.

Für die Ausführung gilt ein logisches Heapbudget von 1 MiB. Parser- und Compilerstrukturen sowie Prozess-RAM sind damit nicht begrenzt. Die Registerstapeltiefe ist außerhalb von `mb-js` nicht öffentlich abfragbar: Der Harness prüft Folge-Scripts und Heapgültigkeit, während die vorhandenen internen Engine-Tests die direkte Registerstapelprüfung behalten.

Die Vorlagen sind keine vollständige ECMAScript-Implementierung und erfassen nicht alle schon unterstützten Konstrukte. Echte Closures, das implizite `arguments`-Objekt, `new`, Arrow-/Async-/Generatorfunktionen, Default-/Rest-/Destrukturierungsparameter, Jobs, Modulevaluation, DOM-Bindings gehören nicht zu diesem Vorlagenmodell. Der Interpreter besitzt inzwischen Standard- und Hostquoten; deren Vertrag und Grenzen stehen in [m2-ausfuehrungsbudget.md](m2-ausfuehrungsbudget.md). `arguments` wird ausschließlich als ausdrücklich benannter Parameter oder lexikalische Bindung verwendet. Die `this`-Orakel betreffen Objekt-Receiver, den globalen Receiver eines abgetrennten nicht strikten Aufrufs sowie striktes `undefined`; das noch unvollständige Wrapping primitiver Receiver wird nicht als spezifikationsgetreu vorausgesetzt. Test262, Engine-Regressionstests und die weiteren M2-Aufgaben bleiben erforderlich.

## Beliebige Quellen unter Quote ausführen

`bounded.rs` wertet jede UTF-8-/UTF-16-Fassung in einem frischen Agent/Realm mit 512 Schritten und 1 MiB logischem Heapbudget aus, normal und mit Stress-GC. Der Harness prüft den tatsächlich übergebenen Budgetzustand bei Host-Abbrüchen, entnimmt Ausnahmen genau einmal und prüft nach GC den Heap. Primitive Werte und Fehler werden semantisch verglichen; Objekte, Symbole und BigInts bleiben passive Typklassen. Falls ein Modus OOM liefert, wird wegen unterschiedlicher GC-/Cachekosten keine Ergebnisgleichheit verlangt; beide Ausführungen bleiben auf gültige Heaps geprüft. Die Beobachtung führt keine Getter aus. Parser/Compiler und einzelne native Operationen benötigen weiterhin die äußeren Zeit-/RSS-Limits.

## Lokal prüfen

Nach dem Einrichten der festgelegten Werkzeuge:

```sh
cargo fmt --manifest-path fuzz/Cargo.toml --all --check
cargo fetch --locked --manifest-path fuzz/Cargo.toml
cargo test --locked --offline --manifest-path fuzz/Cargo.toml --lib
cargo clippy --locked --offline --manifest-path fuzz/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s fuzz -p test_run.py
CARGO_NET_OFFLINE=true cargo +nightly-2026-10-27 fuzz build js-execute
python3 fuzz/run.py js-execute --seconds 30 --seed 1
```

`python3 fuzz/seed_corpus.py` erzeugt die eigenen Startkorpora neu. Das Ziel nimmt an den vorhandenen kurzen PR-Läufen und nächtlichen Läufen teil. Rohprotokolle und Fehlerdateien bleiben gemäß [SECURITY.md](../SECURITY.md) vertraulich.

## Prüfpunkte für Claude

1. Passen die Compilerinvarianten in `fuzz/src/execution/compiler.rs` zum Bytecodeformat, insbesondere Präfixe, signierte Sprungweiten und das Ziel einer leeren Schleife?
2. Verwendet jede Auswertung beliebiger Eingabe in `bounded.rs` eine explizite Hostquote, und ruft die Ergebnisbeobachtung weder Getter noch Konversionen auf? Die Vorlagen in `programs.rs` behalten ihre festen Quelltextgrenzen.
3. Terminieren die Vorlagen für alle Byteparameter nach ihrer Sprachsemantik, und werden sämtliche Quelltextparameter sicher als Zahlen oder Hexescapes eingesetzt?
4. Stimmen Orakel, Fehlerarten, Auswertungsreihenfolge und Zustand zwischen Folge-Scripts mit dem unterstützten Sprachumfang überein?
5. Bleiben normale Ergebnisse und geworfene Werte über Sammlungen hinweg verwurzelt, und werden Ausnahmen vollständig entnommen?
6. Folgen Call-Fenster und verzögerte Funktionsübersetzung dem aktuellen Format? Sind Parameter, Namensbindungen und ausdrücklich gebundenes `arguments` korrekt behandelt, und endet die Rekursion für jeden Byteparameter?
7. Müssen bei der nächsten M2-Stufe Vorlagen, bislang nicht unterstützte Compilerfälle oder Bytecodeannahmen aktualisiert werden? Neue Konstrukte gezielt ergänzen; Abweichungen untersuchen und bestehende Zusicherungen beibehalten.
