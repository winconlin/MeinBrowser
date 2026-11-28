# M2: Tests mit testharness.js in der Shell (Schritt 14a)

`tools/testrunner/src/testharness.rs` führt Tests der WPT mit
testharness.js in einer JavaScript-Shell aus, ohne DOM. Das ist der erste
Teil des WPT-Läufers für M2 aus
[Architektur, Abschnitt 10](architektur.md#10-kompatibilität-messen) und
Schritt 14a aus [12.2](architektur.md#122-nächste-schritte). Der Läufer im
Dokument folgt, wenn es die Ereignisschleife und Skripte im Dokument in
`mb-web` gibt.

## Aufruf

```sh
cargo run --locked -p testdata
cargo test --locked -p testrunner testharness
cargo run --locked --release -p testrunner -- --testharness [--wpt VERZEICHNIS] [--erwartet DATEI] [MUSTER …]
```

Der Test `selected_files_match_the_expectations` vergleicht alle
ausgewählten Dateien mit `tools/testrunner/testharness-erwartet.txt`; er
läuft in der CI mit `cargo test --workspace` auf allen drei Systemen. Das
Programm gibt jede Datei aus, deren Harness nicht OK ist, und jeden
Untertest, der nicht besteht, mit Meldung; mit `--erwartet` schreibt es
die erwarteten Ergebnisse in eine Datei.

## Auswahl

Die WPT sehen eine Shell nur für Dateien `*.any.js` vor, deren Zeile
`// META: global=` den Namen `jsshell` enthält. Beim festen Commit der WPT
sind das 78 von 1.783 Dateien `*.any.js`, alle unter `wasm/`. Sie laufen
alle (Bereich `wasm/jsapi/`) und scheitern, bis es WebAssembly gibt
(M7).

Damit der Läufer schon jetzt JavaScript prüft, nimmt er zusätzlich alle 9
Dateien `*.any.js` unter `js/builtins/weakrefs/` als eigene Variante in der
Shell, obwohl die WPT sie nur für `window` und `dedicatedworker` vorsehen.
Sie brauchen nur testharness.js, `/common/gc.js` und
`resources/maybe-garbage-collect.js`. Die Bereiche stehen in `AREAS`.
`tools/testdata` holt dafür `/resources/testharness.js`, `/common/gc.js`,
`/wasm/jsapi/` und `/js/builtins/weakrefs/`.

## Ablauf je Datei

Jede Datei bekommt einen neuen Agent mit 64 MiB Heap und 50 Millionen
Arbeitsschritten, mit `timeout=long` dem Sechsfachen. Die Skripte folgen
dem Wrapper der WPT für klassische Worker (`ClassicWorkerHandler` in
`tools/serve/serve.py`):

1. `self.GLOBAL` mit `isWindow`, `isWorker` und `isShadowRealm`, alle
   *false*,
2. `/resources/testharness.js`,
3. `report.js` an Stelle von `/resources/testharnessreport.js`: Es meldet
   jeden Untertest mit `add_result_callback` und am Ende alle mit dem
   Status der Harness mit `add_completion_callback` an `mbWpt`,
4. die Zeilen `script=` und `title=` aus den `META`-Zeilen in ihrer
   Reihenfolge, `title=` als `self.META_TITLE = "…";`,
5. die Testdatei und
6. `done();`.

Weil es weder `document` noch einen `WorkerGlobalScope` gibt und
`GLOBAL.isShadowRealm()` *false* ergibt, wählt testharness.js seine
`ShellTestEnvironment`. Zwischen den Skripten laufen keine Microtasks.
Wirft eines, laufen die übrigen nicht, wie bei `importScripts`; ein früher
Fehler wird dabei zu einem SyntaxError, ein fehlendes Skript zu einem
Ereignis ohne Wert.

Die `META`-Zeilen liest `meta.rs` wie `tools/manifest/sourcefile.py` der
WPT: Zeilen `// META: name=wert` am Anfang, mit beliebigem Leerraum nach
`//` und `META:`; die erste andere Zeile beendet sie. Es gilt die erste
Zeile mit `global=`; `worker` steht für die drei Arten von Workern,
`shadowrealm` für die sechs `shadowrealm-in-…`, unbekannte Namen für
nichts. `variant=` wird gelesen, aber noch nicht ausgeführt; keine der
ausgewählten Dateien hat eine.

## Host

`host.rs` legt auf dem globalen Objekt an:

- `self`, das globale Objekt selbst,
- `setTimeout(handler, timeout, ...arguments)` und `clearTimeout(id)` nach
  den „timer initialization steps“ von HTML für eine Funktion als Handler.
  `timeout` wird wie der WebIDL-Typ `long` umgewandelt, negative Werte
  zählen als 0. Eine Zeichenkette als Handler wirft einen TypeError.
- `gc()`, eine volle Sammlung, wie in den Shells von V8 und SpiderMonkey.
  `/common/gc.js` nimmt sie, wenn es kein `TestUtils.gc()` gibt.
- `addEventListener(type, callback)` nur für `error` und
  `unhandledrejection`. testharness.js hört darauf und setzt dann den
  Status der Harness auf ERROR, außer bei `allow_uncaught_exception`.
- `mbWpt` mit `result`, `last` und `complete` für `report.js`.

Eine Ausnahme, die kein Skript fängt, kommt wie bei „report an exception“
in HTML als Ereignis `error` mit `message`, `error`, `filename` (leer),
`lineno` und `colno` (0) an. Ein Fehler der Engine wird dafür zum
Fehlerobjekt seines Konstruktors. Abgelehnte Promises ohne Handler merkt
sich `HostPromiseRejectionTracker`; nach den Microtasks gehen sie, wenn
sie noch keinen Handler bekamen, als Ereignis `unhandledrejection` mit
`promise` und `reason` an die Listener. Wirft ein Listener, merkt sich der
Läufer das nur, damit keine Schleife entsteht.

## Ereignisschleife und Zeit

Nach dem Laden laufen die Microtasks, bis keine mehr warten, dann die
Ereignisse `unhandledrejection`, dann der Zeitgeber, der als nächster
abläuft (bei gleicher Zeit der ältere), und so fort. Die Uhr ist
virtuell: Sie steht, solange JavaScript läuft, und springt zum nächsten
Zeitgeber. `Date.now()` folgt ihr. So ist jeder Lauf gleich, und Warten
kostet keine Zeit. Verschachtelte Zeitgeber werden nicht auf 4 ms
angehoben.

## Ende eines Laufs

Ein Lauf endet, wenn `report.js` das Ende meldet. Wird die Harness nicht
von selbst fertig, weil nichts mehr wartet, der nächste Zeitgeber hinter
der Zeitgrenze von 10 s (mit `timeout=long` 60 s) läge oder die
Arbeitsschritte erschöpft sind, ruft der Läufer `timeout()` aus
testharness.js, wie ein Läufer der WPT bei seiner äußeren Zeitgrenze. In
der Shell hat testharness.js keine eigene Zeitgrenze, und genau dann setzt
`timeout()` den Status der Harness auf TIMEOUT, wenn er noch keinen hat,
und schließt ab; offene Tests werden TIMEOUT mit „Test timed out“. Dafür
gibt es ein Zehntel der Arbeitsschritte neu und noch einmal die
Zeitgrenze. Die Meldung der Harness nennt dann den Grund, etwa
„Zeitgrenze von 10 s erreicht“.

Kommt auch so kein Ende, gilt TIMEOUT mit den bis dahin gemeldeten
Untertests; warf vorher etwas, ERROR mit dieser Meldung. Kein Speicher,
nicht unterstützter Code und eine Ausnahme an unerwarteter Stelle ergeben
ERROR.

## Erwartete Ergebnisse

`testharness-erwartet.txt` hat je Datei eine Zeile `pfad<TAB>STATUS`, wenn
die Harness nicht OK ist, und je Untertest, der nicht besteht,
`pfad<TAB>STATUS<TAB>name`; im Namen stehen `\`, Tabulator und
Zeilenwechsel als `\\`, `\t`, `\n` und `\r`. Meldungen gehören nicht dazu,
damit die Datei auf allen Systemen gleich bleibt. Der Kopf zählt Dateien,
Harness OK und bestandene Untertests. Weicht ein Lauf ab, schreibt der
Test die neue Fassung nach `target/testrunner/testharness-erwartet.txt`
und nennt jede Zeile als Rückschritt oder unerwarteten Erfolg.

## Stand am 28.11.2026

87 Dateien, Harness OK in 76, bestanden 1 von 720 Untertests
(`AbstractModuleSource is not a global` aus
`wasm/jsapi/module/moduleSource.tentative.any.js`). In Release laufen alle
in unter einer Sekunde, in Debug mit den übrigen Tests des Läufers in
etwa 13 s, beides auf einer Maschine mit vier Kernen.

- `wasm/jsapi/`: 78 Dateien, 703 Untertests. Ohne `WebAssembly` werfen 8
  Dateien schon beim Laden einen ReferenceError, 3 unter `esm-integration/`
  scheitern am Phasen-Import `import source`, den es in Scripts nicht gibt.
- `js/builtins/weakrefs/`: 9 Dateien, Harness überall OK, 0 von 17
  Untertests. 16 brauchen `FinalizationRegistry.prototype.cleanupSome`, das
  nicht in ECMA-262 steht, sondern in einem eigenen Vorschlag. Einer
  wartet auf 1.000 Aufräum-Callbacks, die nicht kommen (siehe Befunde).

Gebraucht werden höchstens knapp 200.000 Arbeitsschritte je Datei
(gemessen: mit 150.000 weicht eine Datei ab, mit 200.000 keine); die 50
Millionen sind für schwerere Dateien gedacht und kosten in Release etwa
eine Sekunde, wenn eine Datei sie ganz braucht.

## Befunde

- **Fehler der Engine haben keine Meldung.** Ein TypeError aus `null.x`
  kommt als Fehlerobjekt ohne `message` an; testharness.js meldet dann
  eine leere Meldung oder nur den Namen. Das betrifft die Meldungen, nicht
  die Status.
- **Register halten Objekte am Leben.** In
  `finalizationregistry-cleanupCallback-gets-a-microtask` legt eine
  `async`-Funktion in einem Block ein Objekt an, registriert es und setzt
  die Variable auf *null*; nach `gc()` wird es nicht gesammelt, auch nicht
  in einer gewöhnlichen Funktion, solange ihr Rahmen läuft. Ein Register
  mit dem Zwischenwert hält es. Die Spezifikation erlaubt das, der Test
  erwartet aber, dass es gesammelt wird. Ist der Rahmen beendet, läuft das
  Aufräumen (Test
  `gc_runs_the_cleanup_of_finalization_registries`).

## Grenzen

- Kein `queueMicrotask`, kein `console`, kein `removeEventListener`, keine
  Zeichenketten als Handler für `setTimeout`, kein `setInterval`.
- Das Ereignis `error` kennt Datei und Lage nicht.
- `variant=` läuft noch nicht; jede Datei läuft einmal.
- Nur die beiden Bereiche aus `AREAS`. Weitere Dateien ohne DOM kommen
  dazu, wenn ihre Abhängigkeiten in der Shell da sind.

## Anschluss

14b: ein nächtlicher Lauf mit den Ergebnissen je Bereich. Danach der
Läufer im Dokument, sobald `mb-web` Skripte ausführt.
