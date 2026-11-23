# M2-Arbeitsprotokoll: Spezialoperationen und Stringifier

Dieses Protokoll hält Auswahl, Grundlagen, Umsetzung, Tests und Grenzen des
nächsten Arbeitspakets nach [Konstanten und Defaultwerten](m2-webidl-values.md)
fest. Es beschreibt die tatsächlich implementierte Prüfung, keine vollständige
WebIDL-Validierung oder fertige DOM-Bindings.

## 1. Ausgangsstand und Arbeitsaufteilung prüfen

Geprüfte Basis ist `7feff286b8db4debd4d3c8d4167abc00a7772794`, der Merge
von PR #126. Gelesen wurden `README.md`, `SECURITY.md`, die M2-Roadmap und die
WebIDL-Dokumente zu Namen, Membern, Typen und Werten. Im untersuchten Quellbaum
gab es keine zusätzliche `AGENTS.md`, `CLAUDE.md` oder `CONTRIBUTING.md`.
Die README verlangt eigenen Browsercode ohne fremde Crates und nennt die
Build-, Generator-, Format-, Clippy-, Testdaten- und Testbefehle.

Claudes offener PR #127 bearbeitet JavaScript-Schritt 9c-2, insbesondere
`import()` und Top-Level-Await. Seine Dateiliste wurde vor der Umsetzung
abgeglichen: Die JavaScript-Engine, die gemeinsame Architektur- und Fuzzing-
Dokumentation sowie `fuzz/src/execution/compiler.rs` liegen außerhalb dieses
Pakets. Auch Unicode, Datengeneratoren, Workflows und gemeinsame Fuzzing-
Generatoren bleiben unverändert. Vor Veröffentlichung wird der Abgleich mit
allen dann offenen PRs wiederholt.

Die ursprüngliche Arbeitskopie enthält ältere lokale Änderungen. Sie wird
weder zurückgesetzt noch ausgecheckt. Umsetzung und Tests erfolgen in einem
sauberen Export der Basis. Ein separater temporärer Git-Index dient später
der Veröffentlichung; Arbeitskopie und tatsächlicher Index bleiben erhalten.

**Ergebnis:** Das nächste unabhängige Paket sind die noch offenen Regeln für
Getter, Setter, Deleter und Stringifier aus der Typdokumentation. Collection-,
Array-, `toJSON`-, Overload- und allgemeine Extended-Attribute-Regeln werden
nicht mit einer abgeschlossenen Prüfung dieser Spezialoperationen gleichgesetzt.

## 2. Normative Anforderungen einzeln lesen und zuordnen

Grundlage ist der tatsächlich gelesene WHATWG-WebIDL-Quellstand
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs).
Gelesen wurden Stringifiers, Special operations, Indexed properties und Named
properties samt zugehöriger Grammatik. Die Zeilenbereiche des Quellstands
sind 2503–2567 sowie 2719–2780 und 2938–2953. Die Typkategorien stammen aus
demselben Standardstand wie die bisherigen Typregeln.

| Anforderung | Prüfung | Fehler |
| --- | --- | --- |
| Getter hat genau ein Argument | Signaturlänge | `SpecialOperationSignature` |
| Setter hat genau zwei Argumente | Signaturlänge | `SpecialOperationSignature` |
| Deleter hat genau ein Argument | Signaturlänge | `SpecialOperationSignature` |
| Keine optionalen/variadischen Spezialargumente | Flags aller Argumente | `SpecialOperationSignature` |
| Named Getter/Setter: Schlüssel exakt DOMString | aufgelöster erster Typ | `SpecialOperationKey` |
| Indexed Getter/Setter: Schlüssel exakt unsigned long | aufgelöster erster Typ | `SpecialOperationKey` |
| Deleter: Schlüssel ausschließlich DOMString | aufgelöster erster Typ | `SpecialOperationKey` |
| Höchstens eine eigene Deklaration je Spezialvariante | fünf feste Slots | `DuplicateSpecialOperation` |
| Setter benötigt Getter derselben Variante | Deklarationszusammenfassung | `MissingSpecialGetter` |
| Named Deleter benötigt Named Getter | Deklarationszusammenfassung | `MissingSpecialGetter` |
| Indexed Properties benötigen Integer-Attribut length | reguläres Instanzattribut | `IndexedLength` |
| Stringifier-Attribut ist DOMString oder USVString | exakter aufgelöster Typ | `StringifierType` |
| Höchstens ein eigener Stringifier inklusive Mixins | sechster Slot | `DuplicateStringifier` |

„Exakt“ bedeutet: Typedef- und Annotationshüllen werden aufgelöst, Nullable,
Union und Container bleiben eigene Formen. USVString/ByteString ersetzen keinen
DOMString-Schlüssel; unsigned short/long long ersetzen keinen unsigned-long-
Schlüssel. Ein benanntes `_DOMString` wird nicht zum eingebauten DOMString.
Allgemeine Extended Attributes werden dabei weiterhin nicht validiert.

Es wird keine zusätzliche Vorgabe für Rückgabetypen von Settern oder Deletern
erfunden: Die gelesenen Abschnitte geben hierfür kein ausschließliches
`undefined` beziehungsweise Boolean vor. Gewöhnliche Operationen werden durch
die neuen Signaturregeln nicht eingeschränkt.

## 3. Vererbung und Mixins ausdrücklich abgrenzen

Eigene Deklarationen umfassen Grunddefinition und Partials. Zusätzlich werden
die Stringifier und das relevante `length` aus eingeschlossenen Mixins
berücksichtigt. Zwei eigene Named Getter sind auch mit verschiedenen Namen
oder Rückgabetypen doppelt; ein Named und ein Indexed Getter sind verschiedene
Varianten. Zwei verschiedene Host-Interfaces dürfen denselben Mixin-Stringifier
verwenden. Ein Mixin mit mehreren eigenen Stringifiern wird schon als ungültige
Vorlage abgelehnt.

Vorfahren liefern fehlende Getter und das reguläre `length`-Attribut. Eine
lokale Deklaration desselben Slots ersetzt für diese Bedingungsprüfung den
geerbten Slot; sie wird nicht als zweite eigene Deklaration gezählt. Ein lokal
neu deklariertes nicht-integer `length` kann daher eine geerbte Indexed-Getter-
Bedingung verletzen. `length` muss ein gewöhnliches Instanzattribut sein;
eine statische Eigenschaft oder eine gewöhnliche Operation namens `length`
liefert kein Instanzattribut. Alle acht festen Integer-Typen sind zulässig,
Boolean, Gleitkomma, BigInt, Nullable und Unions nicht.

Das ist eine deklarative Bedingungsprüfung. Sie legt keine Aufrufreihenfolge
oder Implementierung von geerbten Spezialoperationen fest. Der Standard weist
selbst auf die offene Wahl hin, wenn ein Objekt mehrere Interfaces mit derselben
Spezialoperation implementiert. Die spätere Binding-/Dispatch-Implementierung
muss diese Stelle gesondert prüfen. Auch die für Spezialoperationen geforderten
Prose-Algorithmen und Listen unterstützter Namen/Indizes werden hier nicht aus
IDL generiert oder als vorhanden bestätigt.

## 4. Umsetzung und gemeinsame Quoten

Neue Implementierungsteile:

- `crates/mb-webidl/src/types/special.rs`: Signaturen, sechs Slots, Includes,
  Vererbungsbedingungen und Diagnosen.
- `crates/mb-webidl/src/types/special/terminals.rs`: iterative Auflösung von
  Alias-/Annotationshüllen mit einmaliger Zusammenfassung pro Typknoten.
- `crates/mb-webidl/src/types/special/tests.rs`: gezielte Regressionen und
  Belastungsfälle.

Die Einbindung erfolgt in `types::check` nach den bisherigen Form-, Kontext-,
Union- und Dictionary-Zyklenprüfungen und vor der Wert-/Defaultprüfung. Damit
haben vorherige grundsätzliche Typfehler weiter Vorrang. Innerhalb der neuen
Stufe werden zunächst Signaturen und lokale Kardinalitäten, danach Beziehungen
zwischen Deklarationen geprüft. Ein Ausdruck mit mehreren Verstößen kann also
zunächst eine andere passende Diagnose liefern als ein einzeln isolierter Fall.

Aliasse werden nicht pro Getter neu expandiert. Jede Typ-Zusammenfassung wird
in einem flachen Vektor gespeichert und nach der Prüfung freigegeben. Ein Mixin
wird einmal beschrieben; pro Include werden nur sechs feste Slots übertragen.
Auch gemeinsame Vorfahren werden einmal aufgelöst, keine vollständigen
Memberlisten für jeden Nachfahren kopiert. Geordnete ID-Maps benötigen
logarithmische Zugriffe. Arbeitsstapel, Typvektoren und Summaries bleiben durch
die vorhandenen Typ-/Index-/Membergrenzen begrenzt.

Alle Aktionen verwenden den Rest desselben `Budget` einschließlich Abbruchsignal:
Vektorvorbereitung, Aliasbesuche, Argumentprüfungen, Definitions-/Memberbesuche,
Includes und Vererbung bezahlen Arbeitsschritte. Es gibt kein neues Budget pro
Interface oder Include. Die Typstufe bleibt bei standardmäßig zehn Millionen
Schritten, das CLI teilt weiter 45 Millionen über alle Stufen. Neue `Limits`-
Felder oder Abhängigkeiten sind nicht nötig. Logische Quoten sind keine komplette
RAM-/Zeitmessung; Allokationen und einzelne Map-Zugriffe besitzen keine inneren
Abbruchpunkte. Die vorherige Memberstufe kann große Includes weiterhin mehrfach
auf Namen prüfen; diese Kosten werden durch das neue Paket nicht beseitigt.

## 5. Diagnosen und Regressionen prüfen

`types::ErrorKind` erhält die sieben oben genannten Varianten. Die bestehende
Fehlerstruktur bleibt gleich und enthält einen Quellort, keinen zweiten
verwandten Quellort. Ungültige Schlüssel-/Stringifier-/length-Typen zeigen auf
die tatsächliche Verwendung, auch wenn der Alias in einer anderen Datei liegt.
Eine Typ-Produktion kann dabei nachfolgende Trivia enthalten; sie ist nicht
zwangsläufig nur die Bezeichner-Tokenrange. Kardinalitätsfehler zeigen auf die
zweite Deklaration, gegebenenfalls im Mixin statt auf das Include. Ein fehlender
Getter zeigt auf Setter/Deleter, ein fehlendes `length` auf den Indexed Getter.

Gezielte Tests trennen alle fünf Spezialvarianten, Argumentzahlen und
Optional-/Variadic-Flags, primitive und escapte Schlüsseltypen, Aliasse,
Annotationen, Partials, Quellen, Mixins, Vererbung, `length`-Formen und beide
Stringifier-Formen. Rückgabetypen werden ausdrücklich auf die nicht zusätzlich
beschränkte Behandlung geprüft. Ein CLI-Test bestätigt die Spalten 31 bei
`SpecialOperationKey` und 37 bei `StringifierType` über zwei Dateien.

Bei der Testkonstruktion wurden zwei Vorstufen-/Positionsdetails berücksichtigt:
Typ-Ranges können Trivia enthalten; eine Konstante namens `length` scheitert
bereits an den bestehenden reservierten Konstantennamen. Für den isolierten
neuen `IndexedLength`-Fall wird daher stattdessen eine normale Operation namens
`length` benutzt. Die bestehenden Regeln wurden dafür nicht abgeschwächt.

Zwei Tests laufen auf einem Rust-Faden mit 1 MiB Stapel:

1. Eine Kette aus 2.000 Interfaces mit geerbtem Indexed Getter, Integer-`length`
   und lokalen Settern passt unter 500.000 Typ-Arbeitsschritte.
2. Ein Mixin mit 1.000 Attributen und einem Stringifier wird von 1.000 Hosts
   eingebunden. Die neue Typprüfung passt unter 200.000 Schritte. Die vorherige
   Memberprüfung erhält für ihre bestehenden Namensprüfungen separat drei
   Millionen Besuche und 20 Millionen Arbeitsschritte. Der Test behauptet keine
   solche Laufzeitersparnis für die gesamte Pipeline.

Exakte Budgetgrenzen, Schritteerschöpfung und externes Abbruchsignal werden
geprüft. Traversierung und Freigabe benötigen keinen mit der IDL-Tiefe wachsenden
Rust-Aufrufstapel.

## 6. Unabhängige Fuzz-Modelle und reproduzierbare Seeds ergänzen

`fuzz/src/idl/special.rs` wird im bestehenden `webidl`-Ziel aufgerufen. Drei
kleine Modelle erzeugen Erwartungen direkt aus Tabellen und booleschen
Beziehungen statt aus den Summary-Maps und Aliasvektoren der Implementierung:

- 324 Kombinationen: drei Operationsarten, neun Schlüsseltypen, vier
  Argumentzahlen und drei Modifierzustände. Ein gültiger Vorfahr stellt die
  Getter/`length`-Bedingungen bereit, damit Signaturfehler isoliert bleiben.
- 1.792 Kombinationen: 64 Mengen der sechs Deklarationsarten, sieben
  Duplikatvarianten und vier `length`-Formen. Das Orakel trennt Duplikate,
  fehlende passende Getter und fehlendes/falsches `length`.
- 56 Kombinationen: sieben Stringifier-Typen und acht Kombinationen aus eigenem
  Stringifier und zwei Includes; ein geerbter Stringifier ist zusätzlich vorhanden.
  Weitere gemischte Bytefolgen prüfen den gesamten Modell-Einstieg.

Je Modell werden Parser, Namensindex, Member und Typen unter einer gemeinsamen
100.000-Schritt-Quote ausgeführt. Die Grenzen sind zwei Quellen, 8 KiB Quelltext,
8.192 Syntaxknoten, je 1.024 Datensätze pro Vorstufe, 4.096 Memberbesuche,
1.024 Typknoten, 2.048 Kanten, 16 Dictionarys, 512 Dictionary-Kanten,
8.192 Unionbesuche und 16 Interfaces. Es wird kein eigener Rust-Fuzz-Binärname
oder ausgelieferter Browsercode mit einer Fuzzing-Abhängigkeit hinzugefügt.

Zehn neue eigene Konformitäts-Seeds liegen unter `fuzz/seeds/webidl/special-*`.
`python3 fuzz/seed_webidl_special.py` reproduziert nur diese Dateien. Alle 300
bisherigen Seeds bleiben bytegleich; insgesamt sind es danach 310. Zweiter Lauf
und Reproduktion aus einem leeren Verzeichnis wurden bytegleich geprüft.
Der gemeinsame Generator wird nicht geändert. Gemäß `SECURITY.md` bleiben
Rohprotokolle, mutierte Korpora und mögliche Funde privat.

## 7. Prüfungen nach der Repository-Anleitung ausführen

Die Build-/Prüfreihenfolge folgt der README; vor den breiten Tests wurde der
Testdaten-Loader ausgeführt. In der Cloud wurden vorhandene Caches mit `--offline`
und wiederverwendete Target-Verzeichnisse verwendet. Maßgebliche Befehle:

```sh
cargo run --locked -p testdata
cargo run --locked -p deps-guard
cargo run --locked -p datagen -- --check
cargo fmt --all --check
cargo fmt --manifest-path fuzz/Cargo.toml --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --exclude mb-js
cargo clippy --locked --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
cargo test --locked --manifest-path fuzz/Cargo.toml --lib
cargo clippy --locked --manifest-path fuzz/Cargo.toml --lib --bins -- -D warnings
python3 -m unittest discover -s fuzz -p 'test_*.py'
cargo run --locked --release -p testrunner
xvfb-run -a cargo run --locked -p meinbrowser -- --selbsttest
CARGO_NET_OFFLINE=true cargo +nightly-2026-10-27 fuzz build
python3 fuzz/run.py webidl --seconds 30 --seed 1
python3 fuzz/run.py webidl --seconds 30 --seed 2
```

| Prüfung | Lokales Ergebnis |
| --- | --- |
| Workspace außerhalb der unveränderten JS-Engine | 707 Tests bestanden |
| Darin WebIDL / CLI | 127 / 8 Tests bestanden |
| Neu gegenüber der Basis | 15 WebIDL- und 1 CLI-Test |
| Gesamte Fuzz-Library | 99 Tests bestanden, davon 3 neue Modelltests |
| Python-Fuzz-Läufer | 7 Tests bestanden |
| Linux-Clippy, Windows-Cross-Clippy, Fuzz-Clippy | jeweils mit `-D warnings` bestanden |
| Beide Formatprüfungen | bestanden |
| Generatoren / Abhängigkeitswächter | alle 6 Tabellen aktuell, keine neue Fremdabhängigkeit |
| M1-Referenzen | 398/398 bestanden, keine einfarbigen Flächen |
| Fenster-Selbsttest | alle 10 Messpunkte stimmen |
| ASAN/libFuzzer-Build | alle 22 Ziele gebaut; `__asan_init` im WebIDL-Binary geprüft |
| WebIDL, 30 s, Seed 1 | 4.304 Ausführungen, 278 MiB Spitzen-RSS |
| WebIDL, 30 s, Seed 2 | 4.399 Ausführungen, 405 MiB Spitzen-RSS |

Zusammen 8.703 Fuzz-Ausführungen ohne Fund in
diesen begrenzten Läufen; das ist keine Zusage allgemeiner Fehlerfreiheit.
Die vollständige JavaScript-/Test262-Sammlung wird lokal nicht erneut als
bestanden behauptet: `mb-js` wurde beim Workspace-Test ausdrücklich ausgeschlossen,
seine Targets aber mit Clippy kompiliert und die Fuzz-Harnesses getestet.
Native Linux-/Windows-/macOS-Tests einschließlich JS laufen in GitHub-CI.

Ein manueller CLI-Lauf mit `typedef USVString Key;` in `types.idl` und
`interface I { getter long get(Key key); };` in `api.idl` endet wie erwartet
mit Exitcode 1 und `api.idl:1:31: SpecialOperationKey`. Auch das tatsächliche
Werkzeug zeigt damit auf die Verwendung, nicht auf die spätere Aliasdefinition.


## 8. Veröffentlichung und Überschneidungen prüfen

Der konkrete Commit wurde vor dem Upload in einem frischen Checkout mit
`core.autocrlf=true` und `core.eol=crlf` geprüft. Gewöhnliche neue Rust-Dateien
enthalten dort CRLF; die sechs generierten Tabellen bleiben gemäß
`.gitattributes` LF. Tabellencheck, beide Formatprüfungen und alle 135
WebIDL-/CLI-Tests bestanden. Dieser Check läuft unter
Linux und ersetzt keinen nativen Windows-Test.

Die Veröffentlichung verwendet ausschließlich die erlaubten WebIDL-Dateien,
die zugehörigen Dokumente, das gezielte Fuzz-Untermodul und die zehn eigenen
Seeds. Rohe Logs und mutierte Korpora werden nicht hinzugefügt. Die Dateiliste
wird mit Claudes aktuellem PR und sämtlichen weiteren offenen PRs abgeglichen.
Für jede veröffentlichte Datei wird Bytegleichheit zwischen geprüftem Export
und Git-Blob kontrolliert. Der Remote-Branch und später der PR-Head werden
gegen den lokalen Commit geprüft; ursprüngliche Arbeitskopie und Index müssen
unverändert bleiben.

Der Branch heißt `codex/m2-webidl-special-operations`. Laufende GitHub-Prüfungen
werden als laufend gemeldet und nicht als bestanden ausgegeben. Der neue PR
wird vor Abschluss der Remote-CI nicht als gemergt bezeichnet. Dieses Protokoll
enthält die fachlichen Schritte und aggregierte Prüfergebnisse; private rohe
Fuzzing-Protokolle werden gemäß SECURITY.md nicht veröffentlicht.

## 9. Übergabe und verbleibende Arbeiten

Claude kann insbesondere die Trennung von eigenen und geerbten Deklarationen,
Mixins pro Host, die Instanz-`length`-Bedingung und die normativen Grenzen der
Stringifier-Typen prüfen. Laufzeit-Dispatch und Prose-Algorithmen bleiben dabei
ausdrücklich getrennte Aufgaben.

Die nachfolgenden [Collection-Deklarationen einschließlich Vererbungs-/Namensregeln](m2-webidl-collections.md)
sind inzwischen implementiert, ebenso [`toJSON`/JSON-Typen](m2-webidl-json-types.md).
Die [Array-Verwendungsregeln](m2-webidl-arrays.md) sind ebenfalls implementiert.
Offen bleiben allgemeine Extended Attributes und effektive Overload-Sets. Danach
benötigt der Binding-Generator Argumentauswahl, Konvertierung, Ausnahmen,
Prototypen und die GC-/Host-Anbindung. DOM-Bindings, Ereignisschleife, Skripte
im Dokument und WPT/testharness.js bleiben weitere M2-Aufgaben. Claudes
JavaScript-Arbeit wird durch dieses Paket nicht ersetzt; M2 bleibt offen.
