# M2: erste Ausführungstests aus Test262

Die 48.517 bisherigen Test262-Läufe in `parser/test262.rs` prüfen die Syntax.
`script/test262.rs` führt zusätzlich eine feste Auswahl unveränderter externer
Tests durch Parser, Compiler und Interpreter aus, und `script/test262/language.rs`
alle Dateien unter `test/language/` gegen eine Datei der erwarteten
Fehlschläge (siehe [unten](#ganz-language-mit-erwarteten-ergebnissen)), die
unter den fertigen Teilen von `test/built-ins/` gegen eine zweite
und die unter `test/annexB/built-ins/Object/` gegen eine dritte (siehe
[unten](#built-ins-und-annexbbuilt-ins-mit-erwarteten-ergebnissen)). Das ergänzt die M2-Arbeit
aus [Architektur, Abschnitt 10](architektur.md#10-kompatibilität-messen), ohne
die Implementierung der Engine zu ändern.

## Auswahl und Aufruf

```sh
cargo run --locked -p testdata
cargo test --locked -p mb-js script::test262 -- --nocapture
```

Die Auswahl steht in `crates/mb-js/src/script/test262/runtime.txt`: 188 Dateien,
davon 173 positive und 15 mit erwartetem Laufzeitfehler. Sie ergeben 347
Sprachvarianten und mit zwei GC-Modi 694 Ausführungen. Geprüft werden unter
anderem Exponentiation, `??`, logische Zuweisungen, Pfeilfunktionen, Closures,
Vorgabewerte, `arguments`, Methoden, Schleifenbindungen, TDZ-Fehler, `try`
mit `catch` und `finally`, `new` mit [[Construct]] und `new.target`,
`assert.throws` und Kommentare mit Zeilenenden, deren Tests einen
`Test262Error` erwarten. Die
Sammlung bleibt im bereits festgelegten Commit
`91c70759363c46557b694792bf34a2551a7af5ba`; `tools/testdata` holt nun auch
`harness/`, `INTERPRETING.md`, die Teile von `test/built-ins/` aus dem
Abschnitt [unten](#built-ins-und-annexbbuilt-ins-mit-erwarteten-ergebnissen)
und `test/annexB/built-ins/Object/`. Kein externer Testcode wird
ausgeliefert.

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
`test/language/`, die fertigen Teile von `test/built-ins/` und
`test/annexB/built-ins/Object/` liefern die Läufe weiter unten; für das
übrige `built-ins/` und `annexB/` gibt es noch keine, und keine der Zahlen
ist ein vollständiger ECMAScript-Konformitätsnachweis. Die Syntaxzahlen werden getrennt geführt;
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

Mit diesem Stand bestehen 20.168 von 23.296 ausgeführten Dateien und 39.030
von 43.794 Varianten. Seit Schritt 7 gibt es async-Funktionen, `for await`
und async-Generatoren, seit Schritt 8a `Object` mit allen Funktionen,
`Object.prototype` und `Function.prototype` vollständig und gebundene
Funktionen, seit Schritt 8b `Array` vollständig und `for`-`in`, seit Schritt
8c die Fehlerobjekte, `Symbol`, `Boolean`, `Number`, `Math` und die globalen
Funktionen für Zahlen, seit Schritt 8d `Iterator` mit den Helfern und
`JSON`, seit Schritt 8e `Map` und `Set`. Seit Schritt 8c endet keine Datei
der Harness mehr an `Math.pow`. Den meisten Dateien, die noch scheitern, fehlt `eval`: Es gibt
noch keine globale Funktion `eval`, ein Aufruf endet also mit einem
ReferenceError. Der Name steht im Quelltext von 517 der 633 Dateien, die mit
einem ReferenceError scheitern, und von 388 der 522 mit `Test262Error`. In
weiteren 133 Dateien mit `Test262Error` stehen String-Objekte oder Methoden
von `String`; ohne `String.prototype.valueOf` und `toString` wird ein
String-Objekt mit `Object.prototype.toString` zu `"[object String]"`. Das
sind Suchen nach den Namen, nicht für jede Datei einzeln geprüfte Gründe.
Sonst kann der Übersetzer ein Konstrukt noch nicht, etwa `import()`, `with`,
Templates mit Tag oder reguläre Ausdrücke, oder `Function(...)` endet bis
Schritt 9 als `Unsupported`. Dateien mit `async` und `module` scheitern am
Flag `module`.
Die beiden Dateien mit dem Feature `caller`
(`arguments-object/10.6-13-a-2.js` und `-3.js`) lesen `caller` einer nicht
strikten Funktion; diese Erweiterung aus Abschnitt 17.1 gibt es nicht, und
`caller` an %Function.prototype% wirft (Abschnitt 10.2.4). Die Gründe der
Fehlschläge gibt der Test mit `--nocapture` aus:

| Varianten | Grund |
| ---: | --- |
| 1.889 | Der Übersetzer kann ein Konstrukt noch nicht (`Unsupported`) |
| 1.106 | ReferenceError im Test |
| 840 | `Test262Error`: Eine Assertion schlug fehl |
| 698 | Flag `module` |
| 80 | TypeError im Test |
| 59 | Ein Test mit `async` meldete einen Fehlschlag (`AsyncFailure`) |
| 58 | Eine Datei der Harness endet als `Unsupported`, etwa `fnGlobalObject.js`, das `Function` aufruft |
| 30 | Die Arbeitsschritte sind erschöpft |
| 4 | Eine Datei der Harness endet mit einem ReferenceError |

## `built-ins/` und `annexB/built-ins/` mit erwarteten Ergebnissen

```sh
cargo test --locked -p mb-js script::test262::language::test262_built_ins -- --nocapture
cargo test --locked -p mb-js script::test262::language::test262_annex_b_built_ins -- --nocapture
```

Derselbe Läufer nimmt die Teile von `test/built-ins/`, deren eingebaute
Objekte es schon mit allen Eigenschaften gibt: `AggregateError/`, `Array/`,
`Boolean/`, `Error/`, `Function/`, `Iterator/`, `JSON/`, `Map/`,
`MapIteratorPrototype/`, `Math/`, `NativeErrors/`, `Number/`, `Object/`,
`Promise/`, `Set/`, `SetIteratorPrototype/`, `Symbol/` und die globalen Funktionen `isFinite/`,
`isNaN/`, `parseFloat/` und `parseInt/`. Ihre erwarteten Fehlschläge stehen in
`crates/mb-js/src/script/test262/erwartete-fehlschlaege-built-ins.txt`, mit
Pfaden relativ zu `test/built-ins/`, sonst wie oben. Dass es eine eigene
Datei ist und nicht die für `language/`, ist eine Lesart von „in denselben
Lauf aufnehmen“ aus dem Anschluss unten: Beide bleiben für sich lesbar, und
ein neuer Teil von `built-ins/` ändert nur die zweite. Ein dritter Lauf
nimmt ebenso die Teile von `test/annexB/built-ins/`, bisher `Object/`,
gegen `erwartete-fehlschlaege-annexb-built-ins.txt`.

Von 10.361 Dateien laufen 10.219. Übersprungen werden 142 mit Vorschlägen aus
`features.txt`: 89 unter `Promise/`, 35 unter `Error/` mit
`error-stack-accessor` und 18 unter `Iterator/prototype/join/` mit
`Iterator.prototype.join`. Es bestehen 9.253 Dateien und 18.345 von 20.230
Varianten:

| Teil | Dateien | Varianten |
| --- | ---: | ---: |
| `AggregateError/` | 21 von 25 | 42 von 50 |
| `Array/` | 2.857 von 3.083 | 5.671 von 6.119 |
| `Boolean/` | 45 von 51 | 89 von 101 |
| `Error/` | 52 von 58 | 104 von 116 |
| `Function/` | 298 von 509 | 514 von 893 |
| `Iterator/` | 624 von 666 | 1.248 von 1.332 |
| `JSON/` | 136 von 166 | 272 von 332 |
| `Map/` | 186 von 204 | 369 von 405 |
| `MapIteratorPrototype/` | 11 von 11 | 22 von 22 |
| `Math/` | 323 von 327 | 646 von 654 |
| `NativeErrors/` | 82 von 94 | 164 von 188 |
| `Number/` | 333 von 340 | 666 von 680 |
| `Object/` | 3.072 von 3.411 | 6.124 von 6.802 |
| `Promise/` | 626 von 643 | 1.246 von 1.280 |
| `Set/` | 351 von 383 | 700 von 764 |
| `SetIteratorPrototype/` | 11 von 11 | 22 von 22 |
| `Symbol/` | 93 von 98 | 182 von 192 |
| `isFinite/` | 15 von 15 | 30 von 30 |
| `isNaN/` | 15 von 15 | 30 von 30 |
| `parseFloat/` | 52 von 54 | 104 von 108 |
| `parseInt/` | 50 von 55 | 100 von 110 |

Die eine Datei unter `annexB/built-ins/Object/` braucht `$262.IsHTMLDDA`,
das der Host noch nicht hat, und scheitert.

Seit Welle 8c endet `propertyHelper.js` nicht mehr an `Math.pow`; vorher
scheiterten daran 1.077 Dateien. Die meisten der 966 Dateien, die jetzt noch
scheitern, nutzen im Quelltext oder in ihren `includes` ein eingebautes
Objekt, das noch fehlt, oder ein Konstrukt, das der Übersetzer noch nicht
kann. Von den 419 Dateien mit einem ReferenceError im Test nennen außerhalb
von Kommentaren 137 `Date`, 125 `Proxy`, 90 `RegExp` und 34 `Reflect`; eine
Datei kann mehrere Namen enthalten. Seit Welle 8d gibt es `JSON`, daran
scheiterten vorher 68 dieser Dateien. Unter `Function/` rufen
143 der 211 gescheiterten Dateien `Function` auf, das CreateDynamicFunction
braucht, und es scheitern die 21 Dateien mit dem Feature `caller`, die die
Erweiterung aus Abschnitt 17.1 voraussetzen. Das sind Suchen nach den
Namen; dass es jeweils der Grund des Fehlschlags ist, ist nur für die Teile
aus den Wellen 8c bis 8e für jede Datei einzeln geprüft.

Unter `Array/` ist jede der 226 gescheiterten Dateien nach dem Grund
zugeordnet, den der Läufer meldet:

| Dateien | Grund unter `Array/` |
| ---: | --- |
| 85 | ReferenceError im Test an einem fehlenden Objekt; die Dateien nennen `Proxy` (30), `RegExp` (21), `Date` (20), `Reflect` (8), `eval` (6), die typisierten Arrays (5) und `Set` (1), manche mehrere davon |
| 80 | `Unsupported` in der Harness: `resizableArrayBufferUtils.js` ruft `Function` auf, `temporalHelpers.js` enthält BigInt-Literale |
| 21 | `Unsupported` im Test: BigInt-Literale, reguläre Ausdrücke und der Konstruktor `Function` |
| 16 | `Test262Error`: 14 Dateien rechnen mit String-Objekten oder mit `valueOf`, `toString` und `toLocaleString` von `String.prototype`, eine braucht `Reflect.construct` in `isConstructor.js`, eine `Proxy` |
| 11 | ReferenceError in der Harness: `testTypedArray.js` an den typisierten Arrays |
| 9 | Die Arbeitsschritte sind erschöpft, siehe unten |
| 2 | `AsyncFailure`: `Array.fromAsync` über `ArrayBuffer` und `Proxy` |
| 1 | RangeError: `length` wird ein String-Objekt zugewiesen, das ohne `String.prototype.valueOf` NaN ergibt |
| 1 | TypeError: `String.prototype.slice` fehlt |

In den Teilen aus Welle 8c scheitern 51 Dateien. Hier ist jede Datei
einzeln gelesen und dem Konstrukt zugeordnet, an dem sie zuerst scheitert:

| Dateien | Grund in den Teilen aus Welle 8c |
| ---: | --- |
| 11 | `is-a-constructor.js` der zehn Konstruktoren und `Symbol/is-constructor.js`: `isConstructor.js` braucht `Reflect.construct` |
| 11 | `proto-from-ctor-realm.js` der zehn Konstruktoren und `Error/isError/non-error-objects-other-realm.js`: der Konstruktor `Function` aus `$262.createRealm()` |
| 7 | Teile von `String`: String-Objekte, `String.fromCharCode` und `"".concat` |
| 6 | `Date` oder `eval`, eine davon scheitert vorher an `Function` |
| 5 | Die Arbeitsschritte sind erschöpft: Schleifen über 2.000 Werte in `Math.ceil`, `floor` und `round` und über alle Basen in `parseInt` |
| 4 | `Proxy` oder `Reflect`, eine davon scheitert vorher an `Function` |
| 4 | BigInt-Literale |
| 2 | `Map`, `Set` und `RegExp` unter `Symbol/species/` |
| 1 | Ein regulärer Ausdruck als Literal in `Error/isError/non-error-objects.js` |

Unter `Iterator/` und `JSON/` aus Welle 8d scheitern 72 Dateien, ebenso
einzeln zugeordnet:

| Dateien | Grund in den Teilen aus Welle 8d |
| ---: | --- |
| 25 | BigInt: 18 unter `Iterator/` und fünf unter `JSON/` mit Literalen, `JSON/stringify/value-bigint-order.js` mit dem Konstruktor `BigInt` und `value-bigint-cross-realm.js` mit `BigInt` aus einem anderen Realm |
| 32 | `Proxy` oder `Reflect`: 13 unter `Iterator/zip/` und `zipKeyed/`, sechs davon über `Reflect.ownKeys` in `iteratorZipUtils.js`, und 19 unter `JSON/`, eine davon mit `Proxy` aus einem anderen Realm |
| 8 | Teile von `String`: String-Objekte, die ohne `String.prototype.toString` und `valueOf` zu `"[object String]"` werden, `String.prototype.slice` und `String.prototype.charCodeAt` in `value-string-escape-ascii.js` |
| 5 | Die Harness: `temporalHelpers.js` mit BigInt-Literalen (drei unter `Iterator/from/`) und `wellKnownIntrinsicObjects.js`, das `Function` aufruft (zwei) |
| 1 | `Iterator/proto-from-ctor-realm.js`: der Konstruktor `Function` aus `$262.createRealm()` |
| 1 | Ein regulärer Ausdruck als Literal in `JSON/stringify/value-tojson-not-function.js` |

Unter `Map/` und `Set/` aus Welle 8e scheitern 50 Dateien, die Teile für
ihre Iteratoren bestehen ganz. Auch hier ist jede Datei einzeln zugeordnet:

| Dateien | Grund in den Teilen aus Welle 8e |
| ---: | --- |
| 26 | BigInt-Literale: drei unter `Map/` und 23 unter `Set/`, davon 21 zu den Methoden für Vereinigung, Schnitt und Differenz |
| 19 | `WeakMap` und `WeakSet`: Die Methoden müssen sie als falsches *this* ablehnen (zwölf unter `Map/`, sieben unter `Set/`) |
| 3 | Der Konstruktor `Function`: `getOrInsertComputed/different-types-function-callbackfn-does-not-throw.js` und `proto-from-ctor-realm.js` beider Konstruktoren, die ihn aus `$262.createRealm()` holen und auch `Reflect.construct` brauchen |
| 2 | `is-a-constructor.js` beider Konstruktoren: `isConstructor.js` braucht `Reflect.construct` |

Die 18 Dateien unter `Iterator/prototype/join/` überspringt der Läufer, weil
`features.txt` `Iterator.prototype.join` noch unter den Vorschlägen führt,
obwohl die Methode im Entwurf der Spezifikation steht und umgesetzt ist. In
einem einmaligen Lauf ohne diese Ausnahme bestanden 17 davon; die achtzehnte,
`receiver-not-object.js`, enthält ein BigInt-Literal.

Von den 14 Dateien, die die 100.000 Arbeitsschritte erschöpfen, laufen
sieben unter `Array/` mit `every`, `filter`, `forEach`, `indexOf`,
`lastIndexOf`, `map` und `some` über dünn besetzte Arrays mit einer Länge
bis 1.000.000. Abschnitt 23.1.3 verlangt HasProperty für jeden Index, und
jeder Index kostet einen Arbeitsschritt; Lücken zu überspringen, wenn weder
das Objekt noch seine Prototypen Index-Eigenschaften haben oder exotisch
sind, wäre ein späterer Schnellweg. Zwei weitere vergleichen mit
`compareArray.js` 4.000 Elemente und sortieren 2.048 Elemente mit einer
Vergleichsfunktion in JavaScript. Die übrigen fünf sind Schleifen in
JavaScript selbst, drei unter `Math/` und zwei unter `parseInt/`.

Die Gründe aller Varianten:

| Varianten | Grund |
| ---: | --- |
| 832 | ReferenceError im Test |
| 531 | Der Übersetzer kann ein Konstrukt noch nicht oder `Function` wurde aufgerufen (`Unsupported`) |
| 188 | Eine Datei der Harness endet als `Unsupported` |
| 184 | `Test262Error`: Eine Assertion schlug fehl |
| 94 | TypeError im Test, meist der Aufruf einer Funktion, die fehlt |
| 28 | Die Arbeitsschritte sind erschöpft |
| 22 | Eine Datei der Harness endet mit einem ReferenceError |
| 4 | `AsyncFailure`: Der Test meldete einen Fehlschlag |
| 2 | RangeError im Test |

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
3. `annexB/language/`, das `tools/testdata` schon holt, in den Lauf mit
   erwarteten Ergebnissen aufnehmen, und mit jeder Welle von Schritt 8
   weitere Teile von `built-ins/` und `annexB/built-ins/`: je Teil ein
   Eintrag in `BUILT_INS` oder `ANNEX_B_BUILT_INS` in `language.rs` und sein
   Pfad in `tools/testdata`.

Von elf bei der Vorbereitung untersuchten Dateien sind fünf seit Schritt 8a
in der Auswahl (`expressions/call/11.2.3-3_4.js`, `11.2.3-3_6.js`,
`11.2.3-3_7.js` und die beiden unter `expressions/instanceof/`) und drei
seit Schritt 8b (`expressions/arrow-function/cannot-override-this-with-thisArg.js`,
`expressions/exponentiation/exp-operator-evaluation-order.js` und
`expressions/new/ctorExpr-isCtor-after-args-eval.js`, die `Array` mit
`forEach` und `push` brauchen). Drei benötigen noch fehlende
Standardbibliothek und sind deshalb nicht Teil dieser Auswahl:

| Test unter `test/language/` | Voraussetzung |
| --- | --- |
| `expressions/call/11.2.3-3_5.js` | `eval` |
| `expressions/exponentiation/applying-the-exp-operator_A23.js` | `Math.PI`, `Math.E` |
| `expressions/exponentiation/exp-operator-precedence-update-expression-semantics.js` | `Math.pow` |

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
und privaten Namen, async-Funktionen, `for await` und async-Generatoren gibt es schon, Dateien dazu stehen aber noch nicht in der
Auswahl; sie laufen bisher nur im Lauf über `test/language/` ohne
erzwungene Sammlungen.
Diese Auswahl behauptet nicht, sie abzudecken.
