# M2: erste Ausführungstests aus Test262

Die 48.517 bisherigen Test262-Läufe in `parser/test262.rs` prüfen die Syntax.
`script/test262.rs` führt zusätzlich eine feste Auswahl unveränderter externer
Tests durch Parser, Compiler und Interpreter aus, und `script/test262/language.rs`
alle Dateien unter `test/language/` gegen eine Datei der erwarteten
Fehlschläge (siehe [unten](#ganz-language-mit-erwarteten-ergebnissen)), die
unter den fertigen Teilen von `test/built-ins/` gegen eine zweite
und die unter den fertigen Teilen von `test/annexB/built-ins/` gegen eine
dritte (siehe
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
und `test/annexB/built-ins/Date/`, `Object/`, `RegExp/` und `String/`. Kein externer Testcode
wird ausgeliefert.

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
`test/language/` und die fertigen Teile von `test/built-ins/` und
`test/annexB/built-ins/` liefern die Läufe weiter unten; für das
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

Mit diesem Stand bestehen 20.615 von 23.296 ausgeführten Dateien und 39.921
von 43.794 Varianten. Seit Schritt 7 gibt es async-Funktionen, `for await`
und async-Generatoren, seit Schritt 8a `Object` mit allen Funktionen,
`Object.prototype` und `Function.prototype` vollständig und gebundene
Funktionen, seit Schritt 8b `Array` vollständig und `for`-`in`, seit Schritt
8c die Fehlerobjekte, `Symbol`, `Boolean`, `Number`, `Math` und die globalen
Funktionen für Zahlen, seit Schritt 8d `Iterator` mit den Helfern und
`JSON`, seit Schritt 8e `Map` und `Set`, seit Schritt 8f `Reflect` und
`Proxy`, seit Schritt 8g `String` und `RegExp` mit der Engine für reguläre
Ausdrücke und den Literalen, seit Schritt 8h BigInt mit den Literalen, seit Schritt 8i `Date`. Seit
Schritt 8c endet keine Datei
der Harness mehr an `Math.pow`. Den meisten Dateien, die noch scheitern, fehlt `eval`: Es gibt
noch keine globale Funktion `eval`, ein Aufruf endet also mit einem
ReferenceError. Der Name steht im Quelltext von 520 der 581 Dateien, die mit
einem ReferenceError scheitern, und von allen 388 mit `Test262Error`. Das
sind Suchen nach den Namen, nicht für jede Datei einzeln geprüfte Gründe.
Vor Schritt 8g scheiterten weitere 130 Dateien mit `Test262Error`, in denen
String-Objekte oder Methoden von `String` stehen: Ohne
`String.prototype.valueOf` und `toString` wurde ein String-Objekt mit
`Object.prototype.toString` zu `"[object String]"`. Mit dem Schritt bestehen
157 Dateien mehr, darunter diese 130. Mit `RegExp` bestehen 72 Dateien
mehr, die an einem regulären Ausdruck als Literal scheiterten: 29 unter
`literals/`, 25 unter `white-space/` (`after-regular-expression-literal-*.js`),
acht unter `statementList/` und je fünf unter `expressions/` und
`statements/`. Mit BigInt bestehen 170 Dateien mehr, die an einem
BigInt-Literal scheiterten: 148 unter `expressions/`, meist zu den
Operatoren für Rechnen, Vergleichen und Verschieben, 20 unter
`literals/bigint/` und zwei unter `statements/class/elements/`
(`privatefieldget-primitive-receiver.js` und
`privatefieldput-primitive-receiver.js`). Von den Dateien, die noch
scheitern und BigInt nennen, brauchen sechs die typisierten Arrays mit
veränderlichem `ArrayBuffer` aus `resizableArrayBufferUtils.js`, zwei
`BigInt64Array` als Unterklasse und drei `import()`. Mit `Date` bestehen
neun Dateien mehr: vier mit Unterklassen von `Date`, drei davon unter
`statements/class/` und eine unter `expressions/class/subclass-builtins/`,
je zwei unter `expressions/property-accessors/` und `expressions/typeof/`
und `expressions/addition/S11.6.1_A2.2_T2.js`.
Sonst kann der Übersetzer ein Konstrukt noch nicht, etwa `import()`, `with`
oder Templates mit Tag, oder `Function(...)` endet bis
Schritt 9 als `Unsupported`. Dateien mit `async` und `module` scheitern am
Flag `module`.
Die beiden Dateien mit dem Feature `caller`
(`arguments-object/10.6-13-a-2.js` und `-3.js`) lesen `caller` einer nicht
strikten Funktion; diese Erweiterung aus Abschnitt 17.1 gibt es nicht, und
`caller` an %Function.prototype% wirft (Abschnitt 10.2.4). Die Gründe der
Fehlschläge gibt der Test mit `--nocapture` aus:

| Varianten | Grund |
| ---: | --- |
| 1.426 | Der Übersetzer kann ein Konstrukt noch nicht (`Unsupported`) |
| 1.004 | ReferenceError im Test |
| 698 | Flag `module` |
| 572 | `Test262Error`: Eine Assertion schlug fehl |
| 59 | Ein Test mit `async` meldete einen Fehlschlag (`AsyncFailure`) |
| 58 | Eine Datei der Harness endet als `Unsupported`, etwa `fnGlobalObject.js`, das `Function` aufruft |
| 30 | Die Arbeitsschritte sind erschöpft |
| 22 | TypeError im Test |
| 4 | Eine Datei der Harness endet mit einem ReferenceError |

## `built-ins/` und `annexB/built-ins/` mit erwarteten Ergebnissen

```sh
cargo test --locked -p mb-js script::test262::language::test262_built_ins -- --nocapture
cargo test --locked -p mb-js script::test262::language::test262_annex_b_built_ins -- --nocapture
```

Derselbe Läufer nimmt die Teile von `test/built-ins/`, deren eingebaute
Objekte es schon mit allen Eigenschaften gibt: `AggregateError/`, `Array/`,
`BigInt/`, `Boolean/`, `Date/`, `Error/`, `Function/`, `Iterator/`, `JSON/`, `Map/`,
`MapIteratorPrototype/`, `Math/`, `NativeErrors/`, `Number/`, `Object/`,
`Promise/`, `Proxy/`, `Reflect/`, `RegExp/`, `RegExpStringIteratorPrototype/`,
`Set/`, `SetIteratorPrototype/`, `String/`, `StringIteratorPrototype/`,
`Symbol/` und die globalen Funktionen `isFinite/`,
`isNaN/`, `parseFloat/` und `parseInt/`. Ihre erwarteten Fehlschläge stehen in
`crates/mb-js/src/script/test262/erwartete-fehlschlaege-built-ins.txt`, mit
Pfaden relativ zu `test/built-ins/`, sonst wie oben. Dass es eine eigene
Datei ist und nicht die für `language/`, ist eine Lesart von „in denselben
Lauf aufnehmen“ aus dem Anschluss unten: Beide bleiben für sich lesbar, und
ein neuer Teil von `built-ins/` ändert nur die zweite. Ein dritter Lauf
nimmt ebenso die Teile von `test/annexB/built-ins/`, bisher `Date/`,
`Object/`, `RegExp/` und `String/`, gegen `erwartete-fehlschlaege-annexb-built-ins.txt`.

Von 14.622 Dateien laufen 14.480. Übersprungen werden 142 mit Vorschlägen aus
`features.txt`: 89 unter `Promise/`, 35 unter `Error/` mit
`error-stack-accessor` und 18 unter `Iterator/prototype/join/` mit
`Iterator.prototype.join`. Es bestehen 13.489 Dateien und 26.804 von 28.732
Varianten:

| Teil | Dateien | Varianten |
| --- | ---: | ---: |
| `AggregateError/` | 23 von 25 | 46 von 50 |
| `Array/` | 2.975 von 3.083 | 5.903 von 6.119 |
| `BigInt/` | 77 von 77 | 154 von 154 |
| `Boolean/` | 48 von 51 | 95 von 101 |
| `Date/` | 583 von 594 | 1.166 von 1.188 |
| `Error/` | 56 von 58 | 112 von 116 |
| `Function/` | 323 von 509 | 564 von 893 |
| `Iterator/` | 657 von 666 | 1.314 von 1.332 |
| `JSON/` | 166 von 166 | 332 von 332 |
| `Map/` | 189 von 204 | 375 von 405 |
| `MapIteratorPrototype/` | 11 von 11 | 22 von 22 |
| `Math/` | 324 von 327 | 648 von 654 |
| `NativeErrors/` | 88 von 94 | 176 von 188 |
| `Number/` | 339 von 340 | 678 von 680 |
| `Object/` | 3.368 von 3.411 | 6.716 von 6.802 |
| `Promise/` | 634 von 643 | 1.262 von 1.280 |
| `Proxy/` | 294 von 311 | 583 von 607 |
| `Reflect/` | 152 von 153 | 304 von 306 |
| `RegExp/` | 1.345 von 1.879 | 2.688 von 3.756 |
| `RegExpStringIteratorPrototype/` | 17 von 17 | 34 von 34 |
| `Set/` | 374 von 383 | 746 von 764 |
| `SetIteratorPrototype/` | 11 von 11 | 22 von 22 |
| `String/` | 1.195 von 1.223 | 2.388 von 2.443 |
| `StringIteratorPrototype/` | 7 von 7 | 14 von 14 |
| `Symbol/` | 98 von 98 | 192 von 192 |
| `isFinite/` | 15 von 15 | 30 von 30 |
| `isNaN/` | 15 von 15 | 30 von 30 |
| `parseFloat/` | 53 von 54 | 106 von 108 |
| `parseInt/` | 52 von 55 | 104 von 110 |

Unter `annexB/built-ins/` bestehen 162 von 172 Dateien und 324 von 344
Varianten. Die 24 Dateien unter `Date/` zu `getYear`, `setYear` und
`toGMTString` aus Anhang B.2.3 bestehen alle. Unter `RegExp/` laufen 36 der 62 Dateien, die übrigen 26 haben
das Feature `legacy-regexp`, das `features.txt` unter den Vorschlägen führt;
von den 36 rufen `RegExp-leading-escape-BMP.js` und
`RegExp-trailing-escape-BMP.js` `eval` auf und scheitern. Die eine Datei
unter `Object/` und sechs der 111 unter `String/`
(`custom-*-emulates-undefined.js` zu `match`, `matchAll`, `replace`,
`replaceAll`, `search` und `split`) brauchen `$262.IsHTMLDDA`, das der Host
noch nicht hat, und scheitern;
`String/prototype/substr/start-and-length-as-numbers.js` erschöpft die
Arbeitsschritte.

Seit Welle 8c endet `propertyHelper.js` nicht mehr an `Math.pow`; vorher
scheiterten daran 1.077 Dateien. Die meisten der 991 Dateien, die jetzt noch
scheitern, nutzen im Quelltext oder in ihren `includes` ein eingebautes
Objekt, das noch fehlt, oder ein Konstrukt, das der Übersetzer noch nicht
kann. Von den 63 Dateien mit einem ReferenceError im Test nennen außerhalb
von Kommentaren 34 `eval`, die übrigen 29 bis auf
`Function/prototype/toString/line-terminator-normalisation-CR.js`
`ArrayBuffer`, `DataView`, typisierte Arrays, die schwachen Sammlungen,
`WeakRef`, `FinalizationRegistry` oder `SharedArrayBuffer`. Vor Welle 8i
nannten von damals 206 Dateien 141 `Date`. Vor Teil 2 von Welle 8g nannten von damals 315 Dateien 120
`RegExp`. Seit Welle 8d gibt es `JSON`, daran scheiterten
vorher 68 dieser Dateien, seit Welle 8f `Proxy` und `Reflect`: Vor der Welle
nannten von damals 419 Dateien mit einem ReferenceError 125 `Proxy` und 34
`Reflect`. Unter `Function/` rufen
140 der 188 gescheiterten Dateien `Function` auf, das CreateDynamicFunction
braucht, und es scheitern die 21 Dateien mit dem Feature `caller`, die die
Erweiterung aus Abschnitt 17.1 voraussetzen. Das sind Suchen nach den
Namen; dass es jeweils der Grund des Fehlschlags ist, ist nur für die Teile
aus den Wellen 8c bis 8g für jede Datei einzeln geprüft. Mit Welle 8g
bestehen in den älteren Teilen 67 Dateien mehr, die an einem Teil von
`String` scheiterten: 39 unter `Object/`, 16 unter `Array/`, fünf unter
`Iterator/`, drei unter `JSON/`, zwei unter `parseInt/` und je eine unter
`Number/` und `parseFloat/`. Mit `RegExp` bestehen dort weitere 135
Dateien: 69 unter `Object/`, 36 unter `Array/`, 16 unter `Function/`, sieben
unter `Proxy/`, drei unter `Iterator/`, zwei unter `Symbol/` und je eine
unter `Error/` und `JSON/`. Keine Datei scheitert neu.

Mit Welle 8h bestand `BigInt/` bis auf
`wrapper-object-ordinary-toprimitive.js`, das am Ende `new Date` aufruft
und seit Welle 8i besteht.
In den älteren Teilen bestehen 95 Dateien mehr, die an BigInt scheiterten:
22 unter `Set/`, 18 unter `Iterator/`, elf unter `String/`, zehn unter
`RegExp/`, neun unter `Array/`, acht unter `Object/`, sieben unter `JSON/`,
vier unter `Promise/`, je zwei unter `Map/` und `Number/` und je eine unter
`Error/` und `Math/`. Neu scheiterte nur die Datei unter `BigInt/`; drei
Dateien, die an einem BigInt-Literal scheiterten, kommen jetzt bis an ein
fehlendes Objekt: `Array/prototype/toString/non-callable-join-string-tag.js`
an `Date`, `Map/valid-keys.js` und `Set/valid-values.js` an den typisierten
Arrays.

Mit Welle 8i besteht `Date/` bis auf elf Dateien: Die drei
`proto-from-ctor-realm-*.js` holen den Konstruktor `Function` aus
`$262.createRealm()`, die acht unter `prototype/toTemporalInstant/`
brauchen Temporal, das `features.txt` nicht mehr unter den Vorschlägen
führt. In den älteren Teilen bestehen 149 Dateien mehr, die an `Date`
scheiterten, zwei davon unter `RegExp/match-indices/` über die Harness
`deepEqual.js`. Es sind 115 unter `Object/`, die meisten zu den Funktionen
für Eigenschaften und Deskriptoren, 21 unter `Array/`, drei unter `RegExp/`, je zwei unter
`Boolean/`, `Function/`, `Number/` und `Reflect/` und je eine unter
`BigInt/` und `Symbol/`. Keine Datei außerhalb von `Date/` scheitert neu.

Unter `Array/` ist jede der 108 gescheiterten Dateien nach dem Grund
zugeordnet, den der Läufer meldet:

| Dateien | Grund unter `Array/` |
| ---: | --- |
| 71 | `Unsupported` in der Harness: `resizableArrayBufferUtils.js` ruft `Function` auf |
| 11 | ReferenceError im Test an einem fehlenden Objekt: `eval` (6) und `ArrayBuffer` oder die typisierten Arrays (5) |
| 5 | `Unsupported` im Test: der Konstruktor `Function` |
| 11 | ReferenceError in der Harness: `testTypedArray.js` an den typisierten Arrays |
| 9 | Die Arbeitsschritte sind erschöpft, siehe unten |
| 1 | `AsyncFailure`: `Array.fromAsync` über `ArrayBuffer` |

In den Teilen aus Welle 8c scheitern 21 Dateien. Hier ist jede Datei
einzeln gelesen und dem Konstrukt zugeordnet, an dem sie zuerst scheitert:

| Dateien | Grund in den Teilen aus Welle 8c |
| ---: | --- |
| 11 | `proto-from-ctor-realm.js` der zehn Konstruktoren und `Error/isError/non-error-objects-other-realm.js`: der Konstruktor `Function` aus `$262.createRealm()` |
| 2 | `eval` in `Boolean/S9.2_A1_T1.js` und der Konstruktor `Function` in `Boolean/S9.2_A6_T1.js` |
| 7 | Die Arbeitsschritte sind erschöpft: Schleifen über 2.000 Werte in `Math.ceil`, `floor` und `round`, über alle Basen in `parseInt` und über alle 65.536 Codeeinheiten in `parseFloat/S15.1.2.3_A6.js` und `parseInt/S15.1.2.2_A8.js` |
| 1 | Der Konstruktor `Function` in `AggregateError/newtarget-proto-fallback.js` |

Unter `Iterator/` und `JSON/` aus Welle 8d scheitern neun Dateien, alle
unter `Iterator/`, ebenso einzeln zugeordnet:

| Dateien | Grund in den Teilen aus Welle 8d |
| ---: | --- |
| 6 | Die Arbeitsschritte sind erschöpft: `basic-longest.js`, `basic-shortest.js` und `basic-strict.js` unter `Iterator/zip/` und `zipKeyed/`, die mit `forEachSequenceCombination` aus `iteratorZipUtils.js` viele Kombinationen von Eingaben durchgehen |
| 2 | Die Harness: `wellKnownIntrinsicObjects.js`, das `Function` aufruft |
| 1 | `Iterator/proto-from-ctor-realm.js`: der Konstruktor `Function` aus `$262.createRealm()` |

Unter `Map/` und `Set/` aus Welle 8e scheitern 24 Dateien, die Teile für
ihre Iteratoren bestehen ganz. Auch hier ist jede Datei einzeln zugeordnet:

| Dateien | Grund in den Teilen aus Welle 8e |
| ---: | --- |
| 19 | `WeakMap` und `WeakSet`: Die Methoden müssen sie als falsches *this* ablehnen (zwölf unter `Map/`, sieben unter `Set/`) |
| 3 | Der Konstruktor `Function`: `getOrInsertComputed/different-types-function-callbackfn-does-not-throw.js` und `proto-from-ctor-realm.js` beider Konstruktoren, die ihn aus `$262.createRealm()` holen |
| 2 | Die typisierten Arrays `BigInt64Array` und `BigUint64Array` in `Map/valid-keys.js` und `Set/valid-values.js` |

Unter `Proxy/` und `Reflect/` aus Welle 8f scheitern 18 Dateien, ebenso
einzeln zugeordnet:

| Dateien | Grund in den Teilen aus Welle 8f |
| ---: | --- |
| 9 | `with` unter `Proxy/has/` (die Dateien laufen nur ohne strikten Modus) |
| 4 | Der Konstruktor `Function` aus `$262.createRealm()`: `Proxy/get-fn-realm.js`, `get-fn-realm-recursive.js` und die beiden `trap-is-undefined-proto-from-*-realm.js` unter `Proxy/construct/` |
| 3 | `eval`: `Proxy/create-target-is-not-a-constructor.js` und `arguments-realm.js` unter `Proxy/apply/` und `Proxy/construct/`, die `eval` aus einem anderen Realm aufrufen |
| 1 | Flag `module`: `Proxy/preventExtensions/trap-is-undefined-target-is-proxy.js` |
| 1 | `new Function()` in `Reflect/apply/arguments-list-is-not-array-like-but-still-valid.js` |

Unter `String/` aus Welle 8g scheitern 28 Dateien, der Teil
`StringIteratorPrototype/` besteht ganz. Vor Teil 2 der Welle waren es 204,
die meisten an regulären Ausdrücken. Jede Datei ist dem Konstrukt
zugeordnet, an dem sie zuerst scheitert; bei denen mit einem ReferenceError
ist es der einzige fehlende Name, den sie nennen:

| Dateien | Grund unter `String/` |
| ---: | --- |
| 12 | ReferenceError an `eval`; `split/separator-regexp-limit-string-via-eval.js` scheiterte vorher an `RegExp` |
| 5 | Der Konstruktor `Function`: `proto-from-ctor-realm.js` holt ihn aus `$262.createRealm()`, je zwei Dateien unter `replace/` und je eine unter `slice/` und `substring/` rufen ihn auf |
| 5 | Eine Unicode-Eigenschaft ohne Daten in den Dateien `regexp-prototype-*-v-u-flag.js` und `regexp-prototype-search-v-flag.js` unter `match/`, `matchAll/`, `replace/` und `search/` |
| 4 | Templates mit Tag unter `String/raw/` |
| 1 | `with` in `S15.5.5.1_A4_T1.js` |
| 1 | Die Arbeitsschritte sind erschöpft: `repeat/repeat-string-n-times.js` baut in einer Schleife eine Zeichenkette aus 10.000 Punkten |

Unter `RegExp/` aus Welle 8g scheitern 534 der 1.879 Dateien, der Teil
`RegExpStringIteratorPrototype/` besteht ganz. Auch hier ist jede Datei
zugeordnet; die Gruppen nennen die Verzeichnisse und Namen, nach denen
sortiert wurde, und für die Arbeitsschritte hat ein Lauf mit größeren
Quoten nachgeprüft:

| Dateien | Grund unter `RegExp/` |
| ---: | --- |
| 453 | Eine Unicode-Eigenschaft ohne Daten (`Unsupported`): 386 unter `property-escapes/`, 66 unter `unicodeSets/generated/` und `prototype/exec/regexp-builtin-exec-v-u-flag.js` mit `\p{Script=Han}` |
| 72 | Die Arbeitsschritte sind erschöpft: 64 unter `property-escapes/generated/` und sechs unter `CharacterClassEscapes/`, die mit `buildString` aus `regExpUtils.js` Zeichenketten aus allen Codepunkten eines Bereichs bauen, dazu `character-class-escape-non-whitespace.js` und `S15.10.2_A1_T1.js` |
| 7 | `eval`: die fünf `prototype/source/value*.js`, `S15.10.4.1_A8_T11.js` und `S15.10.4.1_A5_T3.js`, die den ReferenceError fängt und dann einen SyntaxError erwartet |
| 2 | Der Konstruktor `Function` aus `$262.createRealm()`: `proto-from-ctor-realm.js` und `prototype/Symbol.split/splitter-proto-from-ctor-realm.js` |

Von den Unicode-Eigenschaften gibt es bisher nur General_Category, `Any`,
`ASCII` und `Assigned`; Script, Script_Extensions, die übrigen binären
Eigenschaften und die Eigenschaften von Zeichenketten mit `v` brauchen
Daten aus der Unicode-Zeichendatenbank, die `tools/datagen` noch nicht
erzeugt.

Für die 72 Dateien an der Quote lief einmalig ein Lauf über `built-ins/` mit
20.000.000 Arbeitsschritten. Dabei bestanden `S15.10.2_A1_T1.js` und
`character-class-escape-non-whitespace.js`. Von den 64 unter
`property-escapes/` kamen 21 bis zu einer Eigenschaft ohne Daten, 42 und die
sechs unter `CharacterClassEscapes/` erschöpften weiter die Schritte, und
`Assigned.js` erschöpfte die 8 MiB des Heaps. Stichproben mit 400.000.000
Schritten und 512 MiB Heap: `character-class-digit-class-escape-negative-cases.js`
und `Any.js` bestanden, `General_Category_-_Cased_Letter.js` und
`Assigned.js` scheiterten mit `Test262Error`. Diese Dateien sind für Unicode
17.0.0 erzeugt, die Tabellen von `mb-unicode` stammen aus Unicode 18.0.0,
und der Vergleich mit `UnicodeData.txt` ergibt genau die Unterschiede der
Versionen: 103 Codepunkte sind in 18.0.0 neu in Lu, Ll oder Lt, 13.007 neu
vergeben, und keiner fehlt. Die beiden scheitern also auch mit mehr Quote
und mehr Heap, bis die Sammlung auf Unicode 18 umgestellt ist; die übrigen
40 sind darauf nicht einzeln geprüft.

Die 18 Dateien unter `Iterator/prototype/join/` überspringt der Läufer, weil
`features.txt` `Iterator.prototype.join` noch unter den Vorschlägen führt,
obwohl die Methode im Entwurf der Spezifikation steht und umgesetzt ist. In
einem einmaligen Lauf ohne diese Ausnahme bestanden vor Welle 8h 17 davon;
die achtzehnte, `receiver-not-object.js`, enthält ein BigInt-Literal und ist
seitdem nicht wieder geprüft.

Von den 95 Dateien, die die 100.000 Arbeitsschritte erschöpfen, gehören 72
zu `RegExp/` (siehe oben). Von den übrigen 23 laufen sieben unter `Array/`
mit `every`, `filter`, `forEach`, `indexOf`,
`lastIndexOf`, `map` und `some` über dünn besetzte Arrays mit einer Länge
bis 1.000.000. Abschnitt 23.1.3 verlangt HasProperty für jeden Index, und
jeder Index kostet einen Arbeitsschritt; Lücken zu überspringen, wenn weder
das Objekt noch seine Prototypen Index-Eigenschaften haben oder exotisch
sind, wäre ein späterer Schnellweg. Zwei weitere vergleichen mit
`compareArray.js` 4.000 Elemente und sortieren 2.048 Elemente mit einer
Vergleichsfunktion in JavaScript. Die übrigen 14 sind Schleifen in
JavaScript selbst: drei unter `Math/`, drei unter `parseInt/`, eine unter
`parseFloat/`, sechs unter `Iterator/zip/` und `zipKeyed/` und eine unter
`String/`. Neun davon scheiterten vor Welle 8g an einer fehlenden Methode
von `String` und kommen jetzt bis an die Quote. In einem einmaligen Lauf mit
20.000.000 Arbeitsschritten bestanden alle 23 und die Datei aus Anhang B.

Die Gründe aller Varianten:

| Varianten | Grund |
| ---: | --- |
| 1.319 | Der Übersetzer kann ein Konstrukt noch nicht, `Function` wurde aufgerufen, oder ein Muster nennt eine Unicode-Eigenschaft ohne Daten (`Unsupported`) |
| 190 | Die Arbeitsschritte sind erschöpft |
| 164 | Eine Datei der Harness endet als `Unsupported` |
| 120 | ReferenceError im Test |
| 70 | `Test262Error`: Eine Assertion schlug fehl |
| 40 | TypeError im Test, meist der Aufruf einer Funktion, die fehlt |
| 22 | Eine Datei der Harness endet mit einem ReferenceError |
| 2 | `AsyncFailure`: Der Test meldete einen Fehlschlag |
| 1 | Flag `module` |

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
Sprachfunktionen bleiben mit ihren Implementierungen zu ergänzen. Iteratoren, Destrukturierung, Generatoren, Klassen mit Feldern
und privaten Namen, async-Funktionen, `for await` und async-Generatoren gibt es schon, Dateien dazu stehen aber noch nicht in der
Auswahl; sie laufen bisher nur im Lauf über `test/language/` ohne
erzwungene Sammlungen.
Diese Auswahl behauptet nicht, sie abzudecken.
