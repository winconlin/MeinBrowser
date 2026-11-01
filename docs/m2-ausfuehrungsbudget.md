# M2: JavaScript-Ausführung begrenzen

Dieses Paket setzt einen weiteren Teil der begrenzten Arbeit aus [Architektur, Abschnitt 8.2, Punkt 15](architektur.md#82-schutzschichten) für `mb-js` um. Es baut auf `main` nach PR #78 und #79 auf. Claude kann anhand der Prüfpunkte am Ende die Implementierung und ihre Anschlussstellen prüfen.

## API und Kosten

`Agent::evaluate_script` erhält ohne aktive Hostquote ein endliches Standardbudget von `DEFAULT_EXECUTION_STEPS`, derzeit 1.000.000 Schritten. Auch ein aus Rust gestarteter Funktionsaufruf erhält eine Standardquote, wenn noch keine aktiv ist.

Eine eigene Quote lässt sich über `evaluate_script_within` oder `with_execution_budget` setzen. Die Typen `Budget`, `Cancel` und `Exhausted` werden aus dem vorhandenen eigenen Crate `mb-base` über `mb_js::agent` verfügbar gemacht. Es kommt keine fremde Abhängigkeit hinzu.

```rust
use std::rc::Rc;
use mb_js::agent::{Agent, Budget, Cancel};

let mut agent = Agent::new().expect("Agent");
let realm = agent.create_realm().expect("Realm");
let cancel = Cancel::new();
let budget = Rc::new(Budget::with_cancel(100_000, cancel.clone()));
let source: Vec<u16> = "let n = 0; while (n < 10) n++; n;"
    .encode_utf16()
    .collect();
agent.scope(|agent, scope| {
    let result = agent.evaluate_script_within(
        scope, realm, &source, Rc::clone(&budget),
    );
    // Ergebnis innerhalb dieses Handle-Bereichs auswerten.
    assert!(result.is_ok());
});
assert!(budget.remaining() < 100_000);
```

Eine Kopie von `Cancel` kann ein anderer Thread setzen. `Rc<Budget>` bleibt auf dem Ausführungsthread. Wird dieselbe `Rc` für mehrere Scripts wiederverwendet, teilen sie sich den verbleibenden Vorrat. `with_execution_budget` erlaubt ebenso eine gemeinsame Quote um mehrere Scripts oder native Aufrufe.

Ein Schritt wird an diesen Stellen verbraucht:

1. Vor jeder Script-Auswertung, noch vor Parse und globalen Deklarationen.
2. Beim Einstieg in einen Interpreterlauf.
3. Vor jedem Bytecodebefehl, einschließlich Sprüngen und `Return`.

Nach erfolgreichem Übersetzen wird das Abbruchsignal vor der Deklarationsphase nochmals ohne Schrittverbrauch geprüft. Das letzte erlaubte Kommando darf bei anschließend null verbleibenden Schritten noch enden; erst ein weiterer abgelehnter Verbrauch setzt den dauerhaften Erschöpfungsgrund.

## Gemeinsame Quoten und Abbruch

Direkte JavaScript-Aufrufe bleiben im selben Budget. Getter, Setter und Umwandlungen wie `valueOf` oder `toString`, die aus Rust erneut in den Interpreter eintreten, verwenden die aktive Quote weiter. Verschachtelte explizite Hostquoten belasten alle Elternquoten; eine großzügigere Kindquote kann ein Elternlimit nicht ersetzen. Dieselbe `Rc` wird nicht doppelt belastet.

Bei mehreren aktiven Quoten kann eine frühere den Versuch bereits bezahlt haben, wenn eine spätere ihn ablehnt. Danach wird der Bytecodebefehl nicht ausgeführt. Die Quoten werden bei normaler oder `Result`-Fehlerrückkehr aus dem Hostbereich entfernt, ihr verbrauchter Vorrat bleibt im übergebenen Budget erhalten.

Erschöpfung oder ein Abbruchsignal liefern `Abrupt::Exhausted(Exhausted::Steps)` beziehungsweise `Abrupt::Exhausted(Exhausted::Cancelled)`. Das ist eine Hostbeendigung und kein JavaScript-`Throw`; es entsteht dadurch kein neuer geworfener Wert. Register und Interpretertiefe werden über die vorhandenen Fehlerpfade aufgeräumt. Ein frisches Budget erlaubt anschließend einen weiteren Aufruf im selben Agent.

Ein Abbruch ist kein Rollback. Vorherige Zuweisungen und angelegte Bindungen bleiben erhalten. Ein anfängliches Nullbudget oder ein bereits gesetztes Abbruchsignal stoppt vor dem Anlegen globaler Bindungen. Eine kleine positive Quote kann dagegen bereits die Deklarationsphase erreichen und erst beim VM-Einstieg enden. Eine dauernd aktive Hostaufgabe muss ihre Quote über mehrere Aufrufe teilen; getrennte Aufrufe ohne Hostquote erhalten jeweils eine neue Standardquote.

`Budget::unlimited()` ist eine ausdrückliche Möglichkeit für vertrauenswürdige Mess- und Stresstests. Der vorhandene Test für sehr tiefe Aufrufe auf kleinem Rust-Stapel verwendet diese Ausnahme gezielt, damit er weiterhin den Register- und Stapelgrenzwert statt die neue Standardquote prüft.

## Grenzen

Schritte sind keine Laufzeit- oder RAM-Messung. Parser, Compiler, globale Deklarationsarbeit, Garbage Collection und die vollständigen Kosten einer einzelnen nativen Operation werden damit nicht umfassend budgetiert. Ein großes String- oder Objektkommando kann mehr Arbeit als ein einfacher Sprung kosten. Abbruchsignale wirken an den Kontrollpunkten; das Paket liefert keine feste maximale Reaktionszeit für jede native Operation.

Das bestehende Rust-Stapelbudget bleibt erhalten. Die Quotenverwaltung verspricht wie `Agent::scope` keine Wiederverwendung nach einem abgefangenen Rust-Panic. Prozessisolation, Sandbox, Ereignisschleife und die weiteren M2-/M5-Schutzschichten bleiben eigenständige Aufgaben.

## Fuzzing und Prüfung

`js-execute` führt zusätzlich zu Compilerprüfung und bekannten Vorlagen jetzt beliebige Eingaben als UTF-8 und UTF-16LE unter einer expliziten Quote von 512 Schritten aus. Jede Fassung läuft in einem frischen Agent/Realm mit normalem und erzwungenem GC, Heapprüfung und 1 MiB logischem Heapbudget.

Primitive Ergebnisse, Sprachfehler, geworfene Werte und Host-Abbrüche werden verglichen. Objekte, Symbole und BigInts bleiben opaque Typklassen; die Beobachtung ruft keine Getter oder Konversionen auf. Logische Speichererschöpfung ist ein erwartetes Ergebnis. Wenn ein GC-Modus OOM erreicht, verlangt der Harness keine identischen Ergebnisse beider Modi; beide Heaps werden dennoch geprüft und Panics weiterhin als Fehler behandelt. Dies ist kein vollständiger Sprach- oder Speicherkonformitätsnachweis.

Regressionen prüfen unter anderem Nullquote, externes Abbruchsignal, Endlosschleifen, Rekursion, Wiedereintritte, Parameterinitialisierung, gemeinsam genutzte und verschachtelte Quoten, das Ende an einer exakten Schrittgrenze sowie Aufräumen und Folgeaufrufe.

## Prüfpunkte für Claude

1. Erreichen alle VM-Einstiege die Standard- oder Hostquote? Prüfen: `agent.rs`, `script.rs`, `interpreter.rs`.
2. Bleibt die Elternquote bei direkten Aufrufen und nativen Getter-/Umwandlungspfaden erhalten? Können weder gleiche `Rc` noch neue Kindbudgets Schritte umgehen oder doppelt berechnen?
3. Wird `Exhausted` durch zukünftige Sprach-Fehlerbehandlung als Hostabbruch weitergereicht, während Register und Interpretertiefe aufgeräumt werden?
4. Setzt die spätere Ereignisschleife pro Aufgabe eine gemeinsam verwendete Quote, statt sie bei jedem Unteraufruf neu anzulegen?
5. Welche längeren Parser-, Compiler-, Deklarations- oder nativen Algorithmen brauchen eigene Arbeitsmessstellen und Abbruchprüfungen?
6. Sind die Defaultquote und die gesonderten Heaplimits anhand echter Ausführungsmessungen passend? Die Vorgabe ist eine erste Schutzgrenze, kein gemessener Hardware-Zielwert.
