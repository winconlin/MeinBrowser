# M2: erste Ausführungstests aus Test262

Die 48.517 bisherigen Test262-Läufe in `parser/test262.rs` prüfen die Syntax.
`script/test262.rs` führt zusätzlich eine feste Auswahl unveränderter externer
Tests durch Parser, Compiler und Interpreter aus. Das ergänzt die M2-Arbeit
aus [Architektur, Abschnitt 10](architektur.md#10-kompatibilität-messen), ohne
die Implementierung der Engine zu ändern.

## Auswahl und Aufruf

```sh
cargo run --locked -p testdata
cargo test --locked -p mb-js script::test262 -- --nocapture
```

Die Auswahl steht in `crates/mb-js/src/script/test262/runtime.txt`: 103 Dateien,
davon 95 positive und acht mit erwartetem Laufzeitfehler. Sie ergeben 195
Sprachvarianten und mit zwei GC-Modi 390 Ausführungen. Geprüft werden unter
anderem Exponentiation, `??`, logische Zuweisungen, Pfeilfunktionen, Closures,
Vorgabewerte, `arguments`, Methoden, Schleifenbindungen und TDZ-Fehler. Die
Sammlung bleibt im bereits festgelegten Commit
`91c70759363c46557b694792bf34a2551a7af5ba`; `tools/testdata` holt nun auch
`harness/` und `INTERPRETING.md`. Kein externer Testcode wird ausgeliefert.

Jede Sprachvariante läuft in einem frischen Agent und Realm, einmal mit
normaler und einmal mit erzwungener Garbage Collection. Standardmäßig gibt
es eine nicht strikte und eine strikt ausgeführte Variante; `onlyStrict`,
`noStrict` und `raw` folgen den Testmetadaten. `raw` bleibt unverändert und
lädt keinen Harness. Harness-Scripts bleiben nicht strikt; der strikten
Testquelle wird ausschließlich `"use strict";` mit Zeilenumbruch vorangestellt.
`includes` werden in ihrer Reihenfolge vor dem Test ausgewertet.

## Harness und heutige Grenzen

Die originalen Dateien `assert.js` und `sta.js` werden unverändert im Realm
ausgewertet. Der derzeitige Compiler unterstützt noch kein `try` und `new`.
Deshalb ersetzt die eigene Testdatei `same-value.js` vorübergehend nur
`assert.sameValue` und `assert.notSameValue`. Test262s `INTERPRETING.md`
erlaubt das Ersetzen von Harness-Funktionen ausdrücklich. Beide verwenden
den originalen `_isSameValue`-Algorithmus einschließlich NaN und vorzeichenbehafteter
Null und werfen bei Abweichungen einen String. Diese Strings
können keinen erwarteten Engine-Fehlertyp erfüllen. Die übrigen Funktionen
des originalen Harness bleiben erhalten; nicht unterstützte Aufrufe führen
zum Fehlschlag.

Diese Auswahl benötigt weder `print` noch die Host-API `$262`. Beide sind
noch nicht eingerichtet. Module, asynchrone Tests, Agenten, `locale` und
weitere Flags werden als nicht unterstützt abgelehnt. Vorschlags-Features
aus `features.txt` sind nicht zugelassen. Der Metadatenleser unterstützt nur
den hier benötigten Teil des Test262-Formats, keinen allgemeinen YAML-Parser;
nicht unterstützte Formen der ausgewerteten Felder werden abgelehnt.

Die Auswahl ist eine Regressionstest-Grundlage für den vorhandenen
Sprachumfang. Sie liefert keine Bestehensquote der gesamten Test262-Sammlung
und keinen vollständigen ECMAScript-Konformitätsnachweis. Die Syntaxzahlen
werden getrennt geführt; Syntax-Negativtests dürfen nicht in diese Auswahl.

## Fehler und Ressourcen

Harness und Test teilen 100.000 Arbeitsschritte. Der logische Heap ist auf
8 MiB begrenzt. Wie bei der [Ausführungsquote](m2-ausfuehrungsbudget.md) sind
dies keine vollständigen Grenzen für Prozess-RAM, Parser-/Compilerarbeit,
native Einzelkosten oder Wandzeit.

Ein positiver Test muss normal enden. Bei einem negativen Laufzeittest
müssen sowohl Phase als auch der genaue `ErrorKind` passen. Die Engine
meldet ein Größenlimit des äußeren Compilers derzeit ebenfalls als
`RangeError`; deshalb werden Parse und Übersetzen bei negativen
Laufzeittests vorab separat geprüft. Ein Compilerlimit kann so keinen
erwarteten Laufzeitfehler erfüllen. Die Engine
besitzt noch keine eingebauten Fehlerobjekte: Nur ihre ausdrücklich
gespeicherten Fehlerarten werden akzeptiert, keine Strings oder Objekte
mit einer passenden `name`-Eigenschaft. Bei der Beobachtung werden keine
JavaScript-Getter oder Konversionen aufgerufen.

Harness-Fehler, nicht unterstützter Code, fehlende Testdaten, Speicher- und
Arbeitserschöpfung sind Fehlschläge. Sie werden nicht übersprungen und können
keinen erwarteten JavaScript-Fehler erfüllen. Nach jeder Auswertung werden
Register, ausstehende Ausnahme und Heap geprüft, nach dem Test zusätzlich
eine Sammlung mit Heapprüfung durchgeführt. Eigene Tests des Läufers prüfen
unter anderem absichtlich falsche Assertions, NaN und +/-0, falsche Fehlerart
und Phase, Harness-Fehler, Realm-Isolation, gemeinsame Quoten und geworfene
Objekte mit nicht aufzurufenden Gettern.

## Anschluss für Claude

1. Sobald `try`, `new` und Fehlerobjekte vorhanden sind, die beiden
   SameValue-Anpassungen entfernen und mit dem vollständigen originalen
   Harness prüfen. Erst dann Tests mit `assert.throws` oder abgefangenen
   Assertion-Fehlern aufnehmen.
2. Für breitere Tests die vollständige Host-API aus `INTERPRETING.md`, später
   Module und asynchrone Abschlüsse implementieren. Neue Anforderungen im
   Metadatenleser ausdrücklich ergänzen.
3. Neue Fälle mit Pfad in die feste Liste aufnehmen. Es gibt weder eine
   automatische Auswahl bestandener Tests noch eine Ausnahmeliste für
   fehlgeschlagene ausgewählte Fälle.

Vier bei der Vorbereitung untersuchte Dateien benötigen noch fehlende
Standardbibliothek und sind deshalb nicht Teil dieser ersten Auswahl:

| Test unter `test/language/` | Voraussetzung |
| --- | --- |
| `expressions/arrow-function/cannot-override-this-with-thisArg.js` | `Array.prototype.forEach` |
| `expressions/exponentiation/applying-the-exp-operator_A23.js` | `Math.PI`, `Math.E` |
| `expressions/exponentiation/exp-operator-evaluation-order.js` | `Array.prototype.push` |
| `expressions/exponentiation/exp-operator-precedence-update-expression-semantics.js` | `Math.pow` |

Sie sind konkrete Kandidaten für die nächste Erweiterung der Liste. Weitere
Sprachfunktionen wie Generatoren, BigInt und Destrukturierung bleiben mit
ihren Implementierungen zu ergänzen; diese Auswahl behauptet nicht, sie
abzudecken.
