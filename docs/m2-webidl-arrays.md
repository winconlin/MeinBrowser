# M2-Arbeitsprotokoll: FrozenArray- und ObservableArray-Verwendungen

Dieses Paket folgt auf [toJSON und JSON-Typen](m2-webidl-json-types.md).
Die vorhandene Typstufe prüft jetzt die Verwendungsorte referenzbasierter
Arrays und die Elementklassen von ObservableArray. Die Syntax und das öffentliche
Types-API bleiben erhalten; es werden noch keine Array-Bindings erzeugt.

## 1. Merge, Anleitungen und Arbeitsbereich prüfen

Basis ist `6e2eabecfd04db88f1f313b29cd92abfec365f97`, der bestätigte Merge
von PR #130. Beim ersten Abgleich waren keine weiteren PRs offen. Main und
Dateiüberschneidungen werden vor dem Push erneut geprüft. Der Nutzer hat
Vorarbeit auch für weitere Projektteile erlaubt; dieses Paket setzt die
bereits dokumentierte Reihenfolge der WebIDL-Typregeln fort.

Gelesen wurden README, SECURITY.md, die M2-Roadmap sowie die Protokolle für
Typen, Member, Werte, Spezialoperationen, Collections und JSON-Typen. Im
geprüften Quellbaum gab es keine zusätzliche AGENTS.md, CLAUDE.md oder
CONTRIBUTING.md. Fremde Browser-Crates bleiben nach README ausgeschlossen.

Die ältere veränderte Original-Arbeitskopie und ihr Index bleiben erhalten.
Ein sauberer Archivexport der Basis enthält Umsetzung und Tests; ein separater
Git-Index dient später dem Commit. JavaScript-/RegExp-/Unicode-Code, gemeinsame
Architektur-/Fuzzing-Dokumentation, Workflows und Datengeneratoren werden nicht
geändert. Claudes Arbeit an der JavaScript-Engine bleibt dadurch unabhängig.

## 2. Standardregeln und zulässige Kontexte lesen

Grundlage ist der tatsächlich gelesene WHATWG-WebIDL-Quellstand
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs),
insbesondere [Frozen array types](https://webidl.spec.whatwg.org/#idl-frozen-array),
[Observable array types](https://webidl.spec.whatwg.org/#idl-observable-array),
[Attributes](https://webidl.spec.whatwg.org/#idl-attributes),
[Interface mixins](https://webidl.spec.whatwg.org/#idl-interface-mixins),
[Nullable types](https://webidl.spec.whatwg.org/#idl-nullable-type),
[Union types](https://webidl.spec.whatwg.org/#idl-union) und Typedefs.

Die maßgeblichen Regeln lauten im gelesenen Stand:

> Frozen array types must only be used as the type of regular attributes or
> static attributes defined on an interface.

> Observable array types must only be used as the type of regular attributes
> defined on an interface.

> The parameterized type T must not be a dictionary type, sequence type,
> record type, or observable array type. However, T may be nullable.

Die letzte Regel betrifft ObservableArray. Sie spricht vom Parametertyp,
nicht von sämtlichen abgeflachten Uniongliedern. Die Prüfung erweitert diese
Liste deshalb nicht stillschweigend auf andere Klassen.

| Verwendungsort | FrozenArray | ObservableArray |
| --- | --- | --- |
| Reguläres Interface-Attribut, auch readonly/inherit/partial | erlaubt | erlaubt |
| Statisches Interface-Attribut | erlaubt | verboten |
| Mixin-Attribut, auch aus Partials | erlaubt | erlaubt |
| Namespace-Attribut | verboten | verboten |
| Operation-/Callback-Rückgabe und Argument | verboten | verboten |
| Konstruktorargument und Async-Iterable-Argument | verboten | verboten |
| Dictionary-Feld, Collection-Schlüssel/-Wert | verboten | verboten |
| Containerparameter, etwa Sequence, Record oder Promise | verboten | verboten |
| Eigenständige Typedef-Deklaration ohne Verwendung | Typbeschreibung erlaubt | Typbeschreibung erlaubt |

Mixins sind laut Standard redaktionelle Vorlagen für die Interfaces, die sie
inkludieren. Ihre Attribute werden daher wie Interface-Attribute behandelt,
auch solange ein Fragment noch keinen Host enthält. Includes und Vererbung
müssen keine Array-Typbäume oder Memberlisten kopieren.

Typedefs und Annotationen verdecken eine Verwendung nicht. Nullable- und
Union-Hüllen behalten deren Kontext; etwa ein nullable FrozenArray oder ein
Unionglied FrozenArray ist in einem zulässigen Attributkontext erlaubt.
Die bereits vorhandenen Regeln verbieten nullable ObservableArray und
ObservableArray in nicht unterscheidbaren Unions weiterhin vorher.
Ein Containerparameter ist dagegen ein eigener Verwendungsort: Auch ein
FrozenArray-Attribut erlaubt kein darin verschachteltes weiteres FrozenArray.
Dictionaries werden hier nominal behandelt; ihre Felder haben eigene Kontexte.

## 3. Vorhandene Typ-Fakten um zwei interne Eigenschaften ergänzen

Die iterative topologische Zusammenfassung in `types/check.rs` berechnet
zusätzlich eine Zweibitmaske `array_use` und das boolesche Merkmal
`observable_element_forbidden`. Named-Typedefs und Annotationen übernehmen
die inneren Fakten. Nullable erhält die Maske und die Elementklasse; Unions
vereinigen nur die Verwendungsmasken. Die Union selbst bleibt eine eigene
Parametertypklasse, statt fälschlich zum Dictionary oder zur Sequence zu werden.

FrozenArray und ObservableArray setzen je ihr eigenes Verwendungsbit.
Ein anderer Container übernimmt die Bits seiner Parameter nicht in seinen
äußeren Typ. Seine Parameter werden anschließend separat geprüft. So wird
beispielsweise `Promise<Values>` an `Values` beanstandet, statt Promise zum
Array-Typ umzudefinieren. JSON-Eigenschaften und bestehende Kategorien bleiben
davon unabhängig.

Das neue Modul `types/arrays.rs` besucht einmal die Typknoten und einmal
Definitionen, Member und Argumente. Es braucht keine zusätzliche Typauflösung,
Rekursion, Graphsuche, Memos oder Kopien von Typedef-Bäumen. Die zwei zusätzlichen
Fakten liegen im vorhandenen dichten Facts-Vektor. Es gibt keine neuen externen
Abhängigkeiten, Limits-Felder oder öffentlichen Typmethoden.

## 4. Parameter, Member und Diagnosen prüfen

Für Generic- und Record-Knoten wird jeder Parameter als eigener Kontext
geprüft. Bei ObservableArray wird zuvor die verbotene Elementklasse geprüft.
Dann werden beide Typslots der Member, alle Signaturargumente sowie Callback-
Rückgaben und -Argumente geprüft. Die Callback-Quelldatei stammt vom zugehörigen
Definitionsfragment. Ein Attribut erhält die erlaubte Maske anhand seiner
Definitionsart und des static-Flags, nicht anhand seines Namens oder readonly.

| Diagnose | Auslöser und Position |
| --- | --- |
| FrozenArrayUse | FrozenArray an einem verbotenen Ort; tatsächliche Typverwendung |
| ObservableArrayUse | ObservableArray an einem verbotenen Ort; tatsächliche Typverwendung |
| ObservableArrayElement | Verbotene direkte/potenziell nullable Elementklasse; Parameterverwendung |

Beispiel: `typedef FrozenArray<long> Values;` in `base.idl` und
`interface I { Values run(); };` in `api.idl` ergeben
`api.idl:1:15: FrozenArrayUse`. Die Diagnose springt nicht zur Aliasdefinition.
`ObservableArray<Item>` mit `typedef sequence<long> Item;` zeigt auf den
Parameternamen `Item` in der aktuellen Datei, mit Spalte 41 im getesteten Beispiel.

Die neue Stufe läuft nach den vorhandenen Typ-, Union-, Dictionary-,
Spezialoperations-, Collection-, Wert- und JSON-Regeln. Bereits spezifische
Fehler behalten dadurch ihre Diagnose, etwa NullableType, DictionaryCycle,
ValueIteratorType oder ToJSONType. Kein fehlerhaftes Types-Teilergebnis wird
zurückgegeben, auch wenn vorher bereits JSON-Fakten berechnet wurden.
Das CLI nennt Array-Verwendungen im Hilfetext; der Erfolgsfall behauptet
weiterhin keine vollständige WebIDL-Gültigkeit.

## 5. Budgets, Grenzen und Regressionen nachweisen

Die Berechnung der beiden Fakten verwendet die bereits budgetierten
Typ-/Kantenbesuche. Der neue Prüflauf belastet jede Definition, jeden Member,
Typknoten, Parameter und Argument-/Typslot mit dem bestehenden gemeinsamen
Restbudget. Die vorhandenen SourceNodes-/Types-/Edges-/Records-Grenzen begrenzen
den Speicher. Es entsteht keine pro Verwendung expandierte Aliasstruktur.

17 neue WebIDL-Tests prüfen zulässige Attribute, Mixins/Partials/Vererbung,
Typedef-/Nullable-/Unionhüllen, Container, Callbacks, Konstruktoren, Collections,
Elementklassen, frühere Diagnosen und quellenübergreifende Fehlerpositionen.
Ein Budgettest ermittelt die tatsächlichen Kosten, besteht mit exakt dieser
Quote und scheitert einen Schritt darunter; externes Cancel bleibt ein
Hostabbruch. Ein separater Thread mit 1 MiB Stack prüft 3.000 Sequence-Schichten
innerhalb eines FrozenArray und 2.000 gemeinsame Attribut-Verwendungen unter
1.000.000 Typschritten.

Ein neuer CLI-Test prüft drei genaue Dateinamen-/Zeilen-/Spalten-Diagnosen.
Ein tatsächlicher CLI-Aufruf wird zusätzlich vor Veröffentlichung geprüft.

Ältere positive Testvorlagen lagen teilweise außerhalb der nun geprüften
Array-Kontexte. Die JSON-Klassifikation von direkten FrozenArray-/ObservableArray-
Typen wird jetzt an gültigen Attributen abgefragt. Eine rein strukturelle
Record-Vorlage verwendet Sequence statt eines darin verbotenen FrozenArray.
Ein früher gültiger FrozenArray-Iteratorvergleich erwartet nach erfolgreichem
Identitätsvergleich die spätere FrozenArrayUse-Diagnose; der Vergleich mit
Sequence erwartet weiterhin ValueIteratorType. Die bisherigen Dictionary-
und JSON-Graphmodelle behalten ihre FrozenArray-Kanten und berechnen die
zusätzliche Verwendungserwartung unabhängig. Keine Tests werden gelöscht
oder mit einer allgemeinen Fehler-Akzeptanz abgeschwächt.

## 6. Unabhängige Fuzz-Erwartungen und eigene Seeds ergänzen

`fuzz/src/idl/arrays.rs` ergänzt das bestehende Ziel, ohne ein neues Binary oder
eine Änderung am gemeinsamen Fuzz-Läufer. Eine vorab festgelegte Berechtigungsmatrix
prüft 16 Verwendungsorte × 2 Array-Arten × 3 Hüllen, insgesamt 96 Fälle.
Die ObservableArray-Elementtabelle prüft 16 vorab gewählte Klassen mit und ohne
Nullable-Hülle, insgesamt 32 Fälle. Ein dritter Test speist beide Modelle mit
64 gemischten Bytefolgen. Die Orakel laufen bei jedem WebIDL-Fuzz-Aufruf;
sie übernehmen keine Fakten aus dem geprüften Types-Ergebnis.

Je Modell teilen Parser, Index, Member und Typstufe 75.000 Schritte. Grenzen:
2 Quellen, 8 KiB Text, 8.192 Syntaxknoten, 1.024 Index-/Memberdatensätze,
4.096 Memberbesuche, 1.024 Typknoten, 2.048 Typkanten, 16 Dictionary-Knoten,
512 Dictionary-Kanten, 8.192 Unionbesuche, 16 Interfaces und 4.096 Vergleichspaare.

Die zwölf neuen eigenen synthetischen Seeds entstehen durch
`python3 fuzz/seed_webidl_arrays.py`. Zwei Läufe und ein leerer Zielordner
liefern dieselben Bytes. Alle 332 bisherigen Seeds des gesamten Fuzz-Workspaces
bleiben bytegleich; danach gibt es 344 insgesamt, davon 102 im WebIDL-Verzeichnis.
Rohe Logs, mutierte Korpora und etwaige Sicherheitsfunde bleiben gemäß
[SECURITY.md](../SECURITY.md) und [fuzzing.md](fuzzing.md) privat.

## 7. Lokale Prüfungen vollständig ausführen

```sh
cargo run --locked -p testdata
cargo run --locked -p datagen -- --check
cargo run --locked -p deps-guard
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --exclude mb-js
cargo clippy --locked --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
cargo fmt --manifest-path fuzz/Cargo.toml --all --check
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
| Workspace außerhalb mb-js | 763 Tests bestanden |
| Darin WebIDL / CLI | 180 / 11 Tests bestanden |
| Neu gegenüber Basis | 17 WebIDL-Tests und 1 CLI-Test mit 3 Diagnosen |
| Fuzz-Library | 110 Tests bestanden, darunter 3 neue Modelltests |
| WebIDL-Modelltests nach der letzten Lint-Anpassung | 24 bestanden |
| Python-Fuzz-Läufer | 7 bestanden |
| Linux-Clippy, Windows-Cross-Clippy, Fuzz-Clippy | jeweils mit -D warnings bestanden |
| Beide Formatprüfungen | bestanden |
| Generatoren / Abhängigkeitswächter | alle 6 Tabellen aktuell, keine neue externe Abhängigkeit |
| M1-Referenzen / Fenster-Selbsttest | 398/398, keine einfarbigen Flächen; alle 10 Messpunkte stimmen |
| ASAN/libFuzzer-Build | alle 22 Ziele gebaut; __asan_init im WebIDL-Binary geprüft |
| WebIDL, 30 s, Seed 1 | 2.559 Ausführungen, 344 MiB Spitzen-RSS |
| WebIDL, 30 s, Seed 2 | 2.601 Ausführungen, 420 MiB Spitzen-RSS |

Zusammen 5.160 Ausführungen ohne Fund in diesen beiden begrenzten Läufen,
ohne Zusage allgemeiner Fehlerfreiheit. Der tatsächliche CLI-Lauf mit den
oben genannten Dateien endet mit Exitcode 1 und der erwarteten Diagnose
`api.idl:1:15: FrozenArrayUse`.

Die Offline-Workspace- und Fensterprüfungen benötigen lokale Unix-Sockets.
Wie beim vorigen Paket blockiert die normale Cloud-Sandbox deren Nutzung;
diese Prüfungen erfolgen deshalb über den vorgesehenen Tool-Freigabeweg
außerhalb dieser Socket-Sperre. Auch die Sanitizer-Läufe verwenden diesen Weg,
damit LeakSanitizer seine abschließende Prozessinspektion durchführen kann.
Es werden keine Sanitizer, Tests oder Kernel-Schutzfunktionen abgeschaltet.

Die vollständige JavaScript-/Test262-Sammlung wird lokal nicht als bestanden
behauptet: Der Workspace-Test schließt mb-js aus. Dessen Targets werden mit
Clippy kompiliert und die Fuzz-Harnesses getestet; vollständige native Plattform-
und JavaScript-Tests laufen in GitHub-CI. Die lokalen Windows-GNU-Crossprüfungen
und der CRLF-Checkout laufen unter Linux und ersetzen keinen nativen Windows-Test.

## 8. Commit, Windows-Zeilenenden und Veröffentlichung kontrollieren

Der endgültige Commit wurde mit `core.autocrlf=true` und `core.eol=crlf`
frisch ausgecheckt. Neue Rust-Dateien enthalten dort CRLF; alle sechs generierten
Tabellen bleiben gemäß .gitattributes LF. Tabellencheck, beide Formatprüfungen
und alle 191 WebIDL-/CLI-Tests bestanden.

Die abgegrenzte Dateiliste wird vor dem Upload mit allen dann offenen PRs
abgeglichen. Jede Datei muss zwischen geprüftem Export und Git-Blob bytegleich
sein. Remote-Branch und PR-Head müssen dem Commit entsprechen. Original-
Arbeitskopie und echter Git-Index bleiben unverändert. Der spätere PR enthält
dieses fachliche Protokoll und aggregierte Ergebnisse, keine privaten Fuzz-Daten.

Branch: `codex/m2-webidl-array-uses`. Laufende GitHub-CI wird als laufend
angegeben. Dieses Paket wird vor Abschluss der Remote-Prüfungen nicht als
gemergt bezeichnet und erklärt M2 nicht für abgeschlossen.

## 9. Übergabe und nächste dokumentierte Aufgaben

Claude kann besonders Kontextmasken gegenüber Elementklassen, reguläre gegenüber
statischen Attributen, Mixin-Vorlagen, Alias-/Nullable-/Unionhüllen sowie die
separaten Container-Verwendungen prüfen. Die ObservableArray-Liste wird auf den
Parametertyp angewandt; zulässige Unions sind keine verbotene direkte
Dictionary-/Sequence-Klasse. Allgemeine Extended Attributes werden hier nicht
interpretiert und gewähren weiterhin keine Freigabe für Binding-Codegenerierung.

Die [fünf standardisierten Typattribute](m2-webidl-type-attributes.md) werden
inzwischen samt Assoziation geprüft. Weitere Extended Attributes und deren
gezielte Attributvalidierung sowie effektive Overload-Sets bleiben offen.
Der Binding-Generator benötigt danach Argumentauswahl, Konvertierungen, Ausnahmen,
Prototypen und die GC-/Host-Verknüpfung. FrozenArray muss später echte unveränderliche
Array-Referenzen erhalten; ObservableArray benötigt Backing Lists, exotische
Objekte und die spezifizierten Set-/Delete-Schritte. Die Typregeln implementieren
diese Laufzeitsemantik noch nicht. DOM-Bindings, Ereignisschleife, Skripte im
Dokument und WPT/testharness.js bleiben weitere M2-Aufgaben.
