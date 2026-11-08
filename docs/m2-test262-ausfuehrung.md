# M2: erste Ausführungstests aus Test262

Die 48.517 bisherigen Test262-Läufe in `parser/test262.rs` prüfen die Syntax.
`script/test262.rs` führt zusätzlich eine feste Auswahl unveränderter externer
Tests durch Parser, Compiler und Interpreter aus, und `script/test262/language.rs`
alle Dateien unter `test/language/` gegen eine Datei der erwarteten
Fehlschläge (siehe [unten](#ganz-language-mit-erwarteten-ergebnissen)) und die
unter `test/built-ins/Promise/` gegen eine zweite (siehe
[unten](#built-inspromise-mit-erwarteten-ergebnissen)). Das ergänzt die M2-Arbeit
aus [Architektur, Abschnitt 10](architektur.md#10-kompatibilität-messen), ohne
die Implementierung der Engine zu ändern.

## Auswahl und Aufruf

```sh
cargo run --locked -p testdata
cargo test --locked -p mb-js script::test262 -- --nocapture
```

Die Auswahl steht in `crates/mb-js/src/script/test262/runtime.txt`: 180 Dateien,
davon 165 positive und 15 mit erwartetem Laufzeitfehler. Sie ergeben 331
Sprachvarianten und mit zwei GC-Modi 662 Ausführungen. Geprüft werden unter
anderem Exponentiation, `??`, logische Zuweisungen, Pfeilfunktionen, Closures,
Vorgabewerte, `arguments`, Methoden, Schleifenbindungen, TDZ-Fehler, `try`
mit `catch` und `finally`, `new` mit [[Construct]] und `new.target`,
`assert.throws` und Kommentare mit Zeilenenden, deren Tests einen
`Test262Error` erwarten. Die
Sammlung bleibt im bereits festgelegten Commit
`91c70759363c46557b694792bf34a2551a7af5ba`; `tools/testdata` holt nun auch
`harness/`, `INTERPRETING.md` und `test/built-ins/Promise/`. Kein externer
Testcode wird ausgeliefert.

Jede Sprachvariante läuft in einem frischen Agent und Realm, einmal mit
normaler und einmal mit erzwungener Garbage Collection. Standardmäßig gibt
es eine nicht strikte und eine strikt ausgeführte Variante; `onlyStrict`,
`noStrict` und `raw` folgen den Testmetadaten. `raw` bleibt unverändert und
lädt keinen Harness. Harness-Scripts bleiben nicht strikt; der strikten
Testquelle wird ausschließlich `"use strict";` mit Zeilenumbruch vorangestellt.
`includes` werden in ihrer Reihenfolge vor dem Test ausgewertet.

## Harness und heutige Grenzen

Die originalen Dateien `assert.js` und `sta.js` werden unverändert im Realm
ausgewertet, ohne eigene Anpassung. Sie brauchen `Function.prototype.call`,
`Object.prototype.toString`, `String` und Fehlerkonstruktoren; diese
eingebauten Funktionen hat die Engine jetzt.

Jeder Agent bekommt vor seinem Realm den Host aus
`script/test262/host.rs`. Er legt in jedem neuen Realm, auch in denen von
`$262.createRealm()`, die Eigenschaften `print` und `$262` des globalen
Objekts an, beschreibbar, konfigurierbar und nicht aufzählbar, wie
`INTERPRETING.md` es verlangt. `$262` hat `createRealm`, `evalScript`, `gc`
und `global` mit denselben Attributen; das ist eine Lesart, denn
`INTERPRETING.md` nennt für diese keine. `evalScript` und `print` wandeln ihr
Argument mit ToString. `evalScript` wertet im Realm seines `$262` aus und
wirft bei einem frühen Fehler einen SyntaxError dieses Realms. `print` hält
die Ausgaben für den Läufer fest; ausgewertet werden sie für Tests mit dem
Flag `async` (siehe [unten](#jobs-und-asynchrone-tests)). Es fehlen `detachArrayBuffer`,
`IsHTMLDDA`, `agent` und `AbstractModuleSource`, bis die Engine die Teile
dafür hat; Tests, die sie brauchen, schlagen fehl.

Module, Agenten, `locale` und weitere Flags werden als nicht unterstützt
abgelehnt, ebenso `async` zusammen mit `raw`, denn ohne Harness gäbe es kein
`$DONE`, das das Ergebnis meldet. Vorschlags-Features
aus `features.txt` sind nicht zugelassen. Der Metadatenleser unterstützt nur
den hier benötigten Teil des Test262-Formats, keinen allgemeinen YAML-Parser;
nicht unterstützte Formen der ausgewerteten Felder werden abgelehnt.

Die Auswahl ist eine Regressionstest-Grundlage für den vorhandenen
Sprachumfang, auch im Belastungsmodus. Die Bestehensquoten für
`test/language/` und `test/built-ins/Promise/` liefern die Läufe weiter
unten; für das übrige `built-ins/` und `annexB/` gibt es noch keine, und keine der Zahlen ist ein vollständiger
ECMAScript-Konformitätsnachweis. Die Syntaxzahlen werden getrennt geführt;
Syntax-Negativtests dürfen nicht in diese Auswahl.

## Jobs und asynchrone Tests

Nach jedem Script, das normal endet, auch nach jeder Datei der Harness, leert
der Läufer mit `Agent::run_jobs` die Warteschlange der Jobs, mit derselben
Quote wie Harness und Test. So hält es ein Browser nach jedem Script, und so
sieht es der Plan für Schritt 7 vor. Wirft ein Job, zählt das wie eine
Ausnahme des Scripts; erschöpft er die Quote, ist das ein Hostabbruch.

Für Tests mit dem Flag `async` wird `harness/doneprintHandle.js` nach
`assert.js` und `sta.js` und vor den `includes` ausgewertet, wie es
`INTERPRETING.md` verlangt. Endet der Test normal, entscheiden danach die
Ausgaben von `print`: Er besteht, wenn er `Test262:AsyncTestComplete`
ausgab und nie etwas, das mit `Test262:AsyncTestFailure:` beginnt; sonst ist
das Ergebnis `AsyncFailure`. Gab er keines von beiden aus, ist er nicht
fertig geworden (`AsyncIncomplete`). `INTERPRETING.md` lässt den Läufer eine
Weile auf `print` warten; das braucht es hier nicht, denn nach dem Leeren der
Warteschlange geschieht nichts mehr, Zeitgeber gibt es nicht. Dass auch eine
Meldung eines Fehlschlags nach `Test262:AsyncTestComplete` den Test scheitern
lässt, ist eine Lesart: `INTERPRETING.md` sagt nur, dass ein Test mit einer
solchen Meldung als gescheitert gilt.

## Ganz `language/` mit erwarteten Ergebnissen

```sh
cargo test --locked -p mb-js script::test262::language -- --nocapture
```

Der Lauf nimmt alle 23.828 Dateien unter `test/language/`, außer den 532 mit
Vorschlägen aus `features.txt`. Jede Variante läuft wie in der Auswahl in
einem frischen Agent mit dem Host für Test262 und denselben Quoten, aber nur
mit normaler Garbage Collection; den Belastungsmodus prüft weiter die
Auswahl. Die Dateien verteilen sich auf so viele Fäden, wie das System
anbietet. Auf vier Kernen dauert der Lauf im Debug-Build knapp vier Minuten.

Die Datei `crates/mb-js/src/script/test262/erwartete-fehlschlaege.txt` nennt
jede Variante, die noch nicht besteht: eine Zeile je Datei, wenn alle ihre
Varianten scheitern, sonst eine Zeile je Variante mit `sloppy` oder `strict`.
Ihre Kopfzeile nennt die Zahlen. Der Test vergleicht den ganzen Text, also
lassen ein Rückschritt, ein unerwarteter Erfolg und geänderte Zahlen ihn
scheitern. Er nennt dann jede abweichende Variante und schreibt die neue
Fassung nach `target/test262/erwartete-fehlschlaege.txt`. Die Datei wird
nicht von Hand gepflegt: Wer die Abweichungen geprüft hat, übernimmt die neue
Fassung in den PR. So zeigt jeder PR, welche Tests er besteht oder verliert.

Dateien, deren Metadaten der Leser ablehnt, scheitern als Ganzes, bisher die
mit `module`. Eine Panik der Engine lässt den Test immer scheitern,
sie kann nicht erwartet werden. Jede Datei der festen Auswahl muss auch hier
bestehen.

Mit diesem Stand bestehen 12.695 von 23.296 ausgeführten Dateien und 24.481
von 43.794 Varianten. Seit Teil 7b gibt es async-Funktionen und `await`, seit
Teil 7c-1 `for await` in async-Funktionen. Die meisten Dateien mit dem Flag
`async`, die noch scheitern, brauchen async-Generatoren (Teil 7c-2) und
scheitern als `Unsupported`, oder ihnen fehlen eingebaute Objekte, etwa
`Object.prototype.hasOwnProperty`, das `asyncTest` aus `asyncHelpers.js`
gleich zu Beginn aufruft, `Object.defineProperty` oder
`Array.prototype.push`. Dateien mit `async` und `module` scheitern am Flag
`module`. Die Gründe der Fehlschläge gibt der Test mit `--nocapture` aus:

| Varianten | Grund |
| ---: | --- |
| 8.726 | Der Übersetzer kann ein Konstrukt noch nicht (`Unsupported`) |
| 3.631 | Eine Datei der Harness endet mit einem ReferenceError |
| 3.277 | ReferenceError im Test |
| 2.031 | TypeError im Test |
| 772 | `Test262Error`: Eine Assertion schlug fehl |
| 698 | Flag `module` |
| 146 | Ein Test mit `async` meldete einen Fehlschlag (`AsyncFailure`) |
| 30 | Die Arbeitsschritte sind erschöpft |
| 2 | Eine Datei der Harness endet mit einem TypeError |

## `built-ins/Promise/` mit erwarteten Ergebnissen

```sh
cargo test --locked -p mb-js script::test262::language::test262_built_ins -- --nocapture
```

Derselbe Läufer nimmt die Teile von `test/built-ins/`, deren eingebaute
Objekte es schon mit allen Eigenschaften gibt, bisher nur `Promise/`. Ihre
erwarteten Fehlschläge stehen in
`crates/mb-js/src/script/test262/erwartete-fehlschlaege-built-ins.txt`, mit
Pfaden relativ zu `test/built-ins/`, sonst wie oben. Dass es eine eigene
Datei ist und nicht die für `language/`, ist eine Lesart von „in denselben
Lauf aufnehmen“ aus dem Anschluss unten: Beide bleiben für sich lesbar, und
ein neuer Teil von `built-ins/` ändert nur die zweite.

Von 732 Dateien laufen 643, 89 brauchen Vorschläge aus `features.txt`. Es
bestehen 328 Dateien und 650 von 1.280 Varianten. Jede gescheiterte Datei
nutzt im Quelltext oder in ihren `includes` mindestens eines der eingebauten
Objekte, die noch fehlen, am häufigsten `Object.defineProperty` und
`Object.getOwnPropertyDescriptor` (auch über `propertyHelper.js`),
`Array.prototype.push` (auch über `promiseHelper.js`), `Set`, `Function`,
`Function.prototype.bind` und `apply`, `Array`, `Reflect` und `eval`. Das ist
eine Suche nach den Namen; dass es jeweils der Grund des Fehlschlags ist,
ist nicht für jede Datei einzeln geprüft. Die Gründe:

| Varianten | Grund |
| ---: | --- |
| 336 | TypeError im Test, meist der Aufruf einer Funktion, die fehlt |
| 106 | Eine Datei der Harness endet mit einem ReferenceError |
| 98 | `AsyncFailure`: Der Test meldete einen Fehlschlag |
| 40 | ReferenceError im Test |
| 40 | `Test262Error`: Eine Assertion schlug fehl |
| 8 | Der Übersetzer kann ein Konstrukt noch nicht (`Unsupported`) |
| 2 | `AsyncIncomplete`: Der Test meldete nichts |

## Fehler und Ressourcen

Harness und Test teilen 100.000 Arbeitsschritte. Der logische Heap ist auf
8 MiB begrenzt. Wie bei der [Ausführungsquote](m2-ausfuehrungsbudget.md) sind
dies keine vollständigen Grenzen für Prozess-RAM, Parser-/Compilerarbeit,
native Einzelkosten oder Wandzeit.

Ein positiver Test muss normal enden. Bei einem negativen Laufzeittest
müssen sowohl Phase als auch die genaue Fehlerart passen. Die Engine
meldet ein Größenlimit des äußeren Compilers derzeit ebenfalls als
`RangeError`; deshalb werden Parse und Übersetzen bei negativen
Laufzeittests vorab separat geprüft. Ein Compilerlimit kann so keinen
erwarteten Laufzeitfehler erfüllen.

`INTERPRETING.md` vergleicht den Namen des Konstruktors der geworfenen
Ausnahme. Der Läufer liest dafür weder `constructor` noch `name`, sondern
ordnet ohne JavaScript ein: Ein Fehler der Engine, den kein `catch` fing,
zählt nach seiner Art. Ein Fehlerobjekt zählt als RangeError,
ReferenceError, SyntaxError oder TypeError, wenn sein direkter Prototyp der
Prototyp dieser Art in einem Realm des Agents ist. Ein Objekt, dessen
direkter Prototyp `Test262Error.prototype` ist, gelesen als eigene
Dateneigenschaften des globalen Objekts und des Konstruktors, zählt als
`Test262Error`. Strings, Objekte mit passendem `name` oder `constructor` und
Objekte, die nur weiter oben in ihrer Prototypkette einen dieser Prototypen
haben, zählen nicht; das ist strenger als der Vergleich der Namen. Als
erwartete Art liest der Metadatenleser bisher nur diese fünf; EvalError,
URIError und Error lehnt er ab, in `test/language/` erwartet sie kein Test.
Bei der Beobachtung werden keine JavaScript-Getter oder Konversionen
aufgerufen.

Harness-Fehler, nicht unterstützter Code, fehlende Testdaten, Speicher- und
Arbeitserschöpfung sind Fehlschläge. Sie werden nicht übersprungen und können
keinen erwarteten JavaScript-Fehler erfüllen. Nach jeder Auswertung werden
Register, ausstehende Ausnahme und Heap geprüft, nach dem Test zusätzlich
eine Sammlung mit Heapprüfung durchgeführt. Eigene Tests des Läufers prüfen
unter anderem absichtlich falsche Assertions und ihre Meldungen,
`assert.throws`, NaN und +/-0, falsche Fehlerart und Phase, `Test262Error`
als erwartete Art, Harness-Fehler, Realm-Isolation, `print` und `$262`
samt den Attributen, gemeinsame Quoten und geworfene Objekte mit nicht
aufzurufenden Gettern.

## Anschluss für Claude

1. Die übrigen Teile der Host-API aus `INTERPRETING.md` mit den Teilen der
   Engine ergänzen, die sie brauchen, später Module. Neue Anforderungen im
   Metadatenleser ausdrücklich ergänzen.
2. Neue Fälle mit Pfad in die feste Liste aufnehmen, wenn sie auch im
   Belastungsmodus laufen sollen. Für die feste Liste gibt es keine
   Ausnahmeliste; die erwarteten Fehlschläge gelten nur für den Lauf über
   `language/`.
3. `annexB/language/`, das `tools/testdata` schon holt, und mit Schritt 8
   weitere Teile von `built-ins/` in den Lauf mit erwarteten Ergebnissen
   aufnehmen: je Teil ein Eintrag in `BUILT_INS` in `language.rs` und sein
   Pfad in `tools/testdata`.

Elf bei der Vorbereitung untersuchte Dateien benötigen noch fehlende
Standardbibliothek und sind deshalb nicht Teil dieser Auswahl:

| Test unter `test/language/` | Voraussetzung |
| --- | --- |
| `expressions/arrow-function/cannot-override-this-with-thisArg.js` | `Array.prototype.forEach` |
| `expressions/call/11.2.3-3_4.js`, `11.2.3-3_6.js`, `11.2.3-3_7.js` | `Object.defineProperty` |
| `expressions/call/11.2.3-3_5.js` | `eval` |
| `expressions/exponentiation/applying-the-exp-operator_A23.js` | `Math.PI`, `Math.E` |
| `expressions/exponentiation/exp-operator-evaluation-order.js` | `Array.prototype.push` |
| `expressions/exponentiation/exp-operator-precedence-update-expression-semantics.js` | `Math.pow` |
| `expressions/instanceof/primitive-prototype-with-object.js` | `Function` |
| `expressions/instanceof/prototype-getter-with-object-throws.js` | `Object.defineProperty`, `Function` |
| `expressions/new/ctorExpr-isCtor-after-args-eval.js` | `Array` |

Sie sind konkrete Kandidaten für die nächste Erweiterung der Liste.

Die 34 Dateien mit dem Feature `tail-call-optimization` sind ebenfalls nicht
in der Auswahl. Mit `$MAX_ITERATIONS` aus `tcoHelper.js` brauchen sie etwa
1,5 bis 2,6 Millionen Arbeitsschritte, gemessen an den 30, die mit größerem
Budget bestehen; das ist mehr als die 100.000 der Auswahl. Von den übrigen
vier brauchen zwei `eval`, eine davon auch `with`, und zwei Templates mit
Tag. Ihre Endpositionen prüfen auch eigene Tests in `compiler/tests.rs`, die
Rekursion ohne wachsenden Registerstapel `script/tests.rs`. Weitere
Sprachfunktionen wie BigInt bleiben mit ihren Implementierungen zu
ergänzen. Iteratoren, Destrukturierung, Generatoren, Klassen mit Feldern
und privaten Namen, async-Funktionen und `for await` gibt es schon, Dateien dazu stehen aber noch nicht in der
Auswahl; sie laufen bisher nur im Lauf über `test/language/` ohne
erzwungene Sammlungen.
Diese Auswahl behauptet nicht, sie abzudecken.
