# M2-Arbeitsprotokoll: Objekt- und Setter-Memberattribute

Dieses Paket folgt auf [fünf Typattribute](m2-webidl-type-attributes.md).
Die Typstufe prüft zusätzlich SameObject, NewObject, Replaceable und
LegacyLenientSetter. Es handelt sich um Deklarationsbedingungen; Objektidentität,
neue Objektallokation und die entsprechenden Setter werden noch nicht erzeugt.

## 1. Merge, Repository-Anleitungen und Arbeitsbereich prüfen

Basis ist `4db9c4026056a56b84d1d2e92326162511755eb5`, der bestätigte Merge
von PR #132. Zu Arbeitsbeginn waren keine weiteren PRs offen. Main und die
Dateien offener PRs werden vor Veröffentlichung erneut abgeglichen.
Der Nutzer hat weitere Vorarbeit erlaubt, solange Claudes parallele Arbeit
und das Gesamtprojekt nicht beeinträchtigt werden.

Gelesen wurden README, SECURITY.md, die M2-Roadmap und die WebIDL-Protokolle.
Im geprüften Quellbaum gab es keine zusätzliche AGENTS.md, CLAUDE.md oder
CONTRIBUTING.md. Keine fremde Browser-Crate wird eingeführt. Der angeforderte
GitHub-Connector war nicht als Tool verfügbar; die bestehende GitHub-Verbindung
wird über Git und API genutzt. Ein fehlender temporärer Sandbox-Pfad wurde im
freigegebenen Bereich wiederhergestellt. Der normale Sandbox-HTTP-Test konnte
den Proxy nicht erreichen; der vorgesehene Tool-Freigabeweg funktioniert.

Die ältere veränderte Original-Arbeitskopie und ihr Index bleiben erhalten.
Umsetzung und Tests verwenden einen sauberen Archivexport der Basis; der Commit
entsteht mit einem getrennten temporären Index. JavaScript-/RegExp-/Unicode-Code,
gemeinsame Architektur-/Fuzzing-Dokumentation, Workflows, Rohdaten und
Datengeneratoren bleiben unverändert.

## 2. Normative Attribute und Typklassen lesen

Grundlage ist der tatsächlich gelesene WHATWG-WebIDL-Quellstand
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs),
insbesondere [SameObject](https://webidl.spec.whatwg.org/#SameObject),
[NewObject](https://webidl.spec.whatwg.org/#NewObject),
[Replaceable](https://webidl.spec.whatwg.org/#Replaceable),
[LegacyLenientSetter](https://webidl.spec.whatwg.org/#LegacyLenientSetter),
[PutForwards](https://webidl.spec.whatwg.org/#PutForwards),
[Attributes](https://webidl.spec.whatwg.org/#idl-attributes) und die Definition
regular operation im Operations-Abschnitt.

| Flag | Erlaubte Deklaration | Zieltyp / weitere Bedingungen |
| --- | --- | --- |
| SameObject | readonly-Attribut | nicht-nullable gewöhnliches Interface oder object |
| NewObject | reguläre oder statische Operation | nicht-nullable gewöhnliches Interface oder Promise als Rückgabe |
| Replaceable | reguläres readonly-Attribut, außerhalb von Namespaces | weder LegacyLenientSetter noch PutForwards |
| LegacyLenientSetter | reguläres readonly-Attribut, außerhalb von Namespaces | weder Replaceable noch PutForwards |

Alle vier Flags verlangen die argumentlose Form. Ein gleichnamiges Flag mit
`=Value`, `()` oder einer Liste wird nicht als gültige Kurzschreibweise akzeptiert.
Kommentare und wiederholte identische Flags ändern die Bedeutung nicht.

SameObject verbietet statische Attribute nicht und schließt Namespaces nicht
pauschal aus. NewObject gilt auch für passende Namespace-/Callback-Interface-
Operationen. Ein benannter Spezialgetter ist zugleich eine reguläre Operation;
ein anonymer Getter ist es nicht. Callback-Interface-Werttypen sind keine
gewöhnlichen Interface-Typen. Nullable Interface-/Object-Typen, FrozenArray,
Buffertypen und Unions erfüllen SameObject nicht. NewObject erlaubt ausdrücklich
Promise-Rückgaben, ohne deren Parameter auf Interfaces zu beschränken.

Promise-Attribute müssen bereits nach der vorhandenen Typregel readonly sein.
Zusätzlich dürfen sie keine der Annotationen SameObject, Replaceable,
LegacyLenientSetter oder PutForwards tragen. Dieser Ausschluss gilt für den
äußeren aufgelösten Promise-Typ, nicht für beliebige Containerparameter.
Ein Replaceable-FrozenArray mit Promise-Elementen ist dadurch nicht selbst ein
Promise-Attribut.

## 3. Scope von PutForwards und unbekannten Attributen begrenzen

Der gelesene Standard normiert Konflikte von Replaceable/LegacyLenientSetter
mit PutForwards sowie dessen Ausschluss auf Promise-Attributen. Dafür erkennt
diese Stufe ausschließlich die Präsenz des exakten Namens PutForwards.

Die vollständige Argumentform, dessen readonly-/Interface-Anforderungen,
Zielnamensauflösung auf dem Interface, Vererbung/Includes bei der Auflösung und
Zyklen von Weiterleitungsketten werden hier noch nicht validiert. Ein allein
verwendetes PutForwards bestätigt daher keine gültige oder sichere Weiterleitung.
Die Scope-Regression hält diese Grenze ausdrücklich fest. Seine späteren Setter-
Algorithmen bleiben ebenfalls offen. Das ist eine bewusst noch unvollständige
Semantikstufe, keine Freigabe für Binding-Codegenerierung.

Andere Attributnamen und ihre verschachtelten Inhalte bleiben opaque und
ungeprüft. `Custom=(SameObject,NewObject)` enthält keine eigenständigen Flags.
Groß-/Kleinschreibung und führende Unterstriche werden nicht stillschweigend
zu einem der vier exakten Standardnamen normalisiert.

## 4. Listen, Memberkontext und Typverwendungen zusammenführen

Das neue Modul `types/member_attributes.rs` verwendet die bereits im Lowering
vorgemerkten nichtleeren echten ExtendedAttributeList-Produktionen. Der vollständige
Syntaxbaum wird nicht noch einmal durchsucht. `member_attributes/syntax.rs`
liest relevante Listen iterativ und erzeugt höchstens einen temporären Eintrag
pro Liste mit einer Bitmaske und einer ursprünglichen Quellposition.

Die vier vollständigen Flags werden nur aus der vorderen Member-Attributliste
entnommen. An Definitionen, Typannotationen, Argumenten, Dictionary-Feldern oder
Includes sind sie falsch platziert. Reste solcher Listen führen zu einer
Placement-Diagnose. PutForwards allein wird dabei nicht als vollständig geprüftes
Memberflag umgedeutet.

Der Prüfer besucht einmal Definitionen und deren eigene Member. Mixin-Member
werden nicht für jeden Host kopiert oder erneut geprüft. Statisch/regulär,
readonly, Namespace und Operationsname stammen aus dem vorhandenen Membermodell.
Die bereits berechnete Typkategorie löst Typedefs und Annotationen auf, ohne
die Nullable-Kategorie zu verlieren. Es gibt keine neue Typgraphsuche oder
verwendungsabhängige Mutation gemeinsamer Aliasse.

## 5. Diagnosen, Priorität und Ressourcen prüfen

| Diagnose | Bedeutung / Quellposition |
| --- | --- |
| MemberAttributeForm | einer der vier Namen mit unzulässigen Zusätzen; Attribut selbst |
| MemberAttributePlacement | falscher Member-/Definitions-/Typkontext oder verbotene static/namespace-Kombination |
| MemberAttributeReadonly | readonly fehlt; tatsächlicher Attributname |
| MemberAttributeConflict | Replaceable/LegacyLenientSetter/PutForwards-Konflikt; erste relevante Annotation |
| MemberAttributePromise | verbotenes Flag auf Promise-Attribut; tatsächliche Typverwendung |
| MemberAttributeType | SameObject-/NewObject-Zielklasse unzulässig; tatsächliche Typverwendung |

Die bisherigen Typ-, Dictionary-, Union-, Spezialoperations-, Collection-, Wert-,
JSON-, Array- und Typattributprüfungen laufen zuerst. Ihre spezifischen Fehler
bleiben erhalten. Innerhalb dieser Stufe folgen auf die Flag-Form zunächst
Kontext/readonly, dann Konflikte, Promise-Ausschlüsse und Zieltypen. Die CLI-
Regression prüft drei genaue Dateinamen-/Zeilen-/Spalten-Diagnosen, darunter
Typedef-Verwendung, Membername und fehlerhafte Attributform.

Listen-, Token-, Definitions-, Member- und Restbesuche belasten das vorhandene
geteilte Restbudget. Die eigentlichen Bedingungen bestehen aus einer festen
Zahl von Bit-/Kategorieprüfungen pro Member. Die bisherigen Syntaxknoten-/
Datensatzgrenzen beschränken Listen und Cacheeinträge. Es gibt keine zusätzlichen
Limits-Felder, Fremd-Crates, Rekursion, Aliasbaumkopien oder expandierten
Vorfahren-/Mixinlisten. Fehlen die relevanten Namen, endet die Stufe nach dem
Listenabgleich ohne Memberdurchlauf.

21 neue WebIDL-Tests prüfen alle vier Flagklassen, Formen, Callback-/Namespace-
und statische Fälle, benannte/anonyme Getter, Promise-Ausschlüsse, Kombinationen,
Dateigrenzen, Budget und Abbruch. Ein Stresstest prüft ein Mixin mit 1.000
annotierten Attributen und 2.000 Hosts auf 1 MiB Stack unter 1.000.000
Typschritten. Exakte Kosten, ein Schritt darunter und externes Cancel werden
zusätzlich geprüft. Alte Tests und deren Budgets werden nicht abgeschwächt.
Ein neuer CLI-Test ergänzt drei konkrete Positionsdiagnosen.

## 6. Unabhängige Modelle und eigene Seeds ergänzen

`fuzz/src/idl/member_attributes.rs` ergänzt das vorhandene WebIDL-Ziel um vier
Orakeltests: 64 Typ-/Flagpaare, 48 Kontext-/Flagpaare, 18 Konflikt-/Promise-Paare
sowie 24 Formen und 64 gemischte Bytefolgen. Tabellen und erwartete Kategorien
werden vor dem Rendern der Quellen festgelegt. Die Modelle übernehmen keine
Bedingungen oder Bitmasken aus dem geprüften Typmodell. Frühere Fehler wie
FrozenArrayUse werden in den Erwartungen ausdrücklich berücksichtigt.

Je Modell teilen Parser, Index, Member und Typstufe 100.000 Schritte. Grenzen:
2 Quellen, 8 KiB Text, 8.192 Syntaxknoten, 1.024 Index-/Memberdatensätze,
4.096 Memberbesuche, 1.024 Typknoten, 2.048 Kanten, 16 Dictionary-Knoten,
512 Dictionary-Kanten, 8.192 Unionbesuche, 16 Interfaces und 4.096 Vergleichspaare.
Die gemeinsame Fuzz-Zielliste und der Läufer bleiben unverändert.

Zwölf eigene synthetische Seeds entstehen mit
`python3 fuzz/seed_webidl_member_attributes.py`. Zwei Läufe und ein leerer
Zielordner liefern identische Bytes. Alle 356 bisherigen Seeds bleiben
bytegleich; anschließend gibt es 368 insgesamt, davon 126 für WebIDL.
Rohe Logs, mutierte Korpora und mögliche Sicherheitsfunde bleiben nach
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
| Workspace außerhalb mb-js | 808 Tests bestanden |
| Darin WebIDL / CLI | 223 / 13 Tests bestanden |
| Neu gegenüber Basis | 21 WebIDL-Tests und 1 CLI-Test mit 3 Diagnosen |
| Fuzz-Library | 118 Tests bestanden, darunter 4 neue Modelltests |
| WebIDL-Fuzz-Modelltests | 32 bestanden |
| Python-Fuzz-Läufer | 7 bestanden |
| Linux-Clippy, Windows-Cross-Clippy, Fuzz-Clippy | jeweils mit -D warnings bestanden |
| Beide Formatprüfungen | bestanden |
| Generatoren / Abhängigkeitswächter | alle 6 Tabellen aktuell, keine neue externe Abhängigkeit |
| M1-Referenzen / Fenster-Selbsttest | 398/398, keine einfarbigen Flächen; alle 10 Messpunkte stimmen |
| ASAN/libFuzzer-Build | alle 22 Ziele gebaut; __asan_init im WebIDL-Binary geprüft |
| WebIDL, 30 s, Seed 1 | 2.141 Ausführungen, 338 MiB Spitzen-RSS |
| WebIDL, 30 s, Seed 2 | 2.124 Ausführungen, 422 MiB Spitzen-RSS |

Zusammen 4.265 Ausführungen ohne Fund in diesen beiden begrenzten Läufen,
ohne Zusage allgemeiner Fehlerfreiheit. Der tatsächliche CLI-Lauf mit den
oben genannten Dateien endet mit Exitcode 1 und der erwarteten Diagnose
`api.idl:1:47: MemberAttributeType`.

Die Offline-Workspace-/Fensterprüfungen verwenden den vorgesehenen Tool-
Freigabeweg für lokale Unix-Sockets außerhalb der Cloud-Sandbox-Sperre.
Dasselbe gilt für die ASAN-Läufe, damit LeakSanitizer seine abschließende
Prozessinspektion durchführen kann. Keine Tests, Sanitizer oder Kernel-
Schutzfunktionen wurden abgeschaltet. Vollständige native Plattformtests und
JavaScript-/Test262 laufen in GitHub-CI; lokal wird dafür kein bestandener
Gesamtlauf behauptet. Windows-Crossprüfungen und CRLF unter Linux ersetzen
keinen nativen Windows-Test.

## 8. Exakten Commit und Veröffentlichung kontrollieren

Der endgültige Commit wurde mit `core.autocrlf=true` und `core.eol=crlf`
frisch ausgecheckt. Neue Rust-Dateien enthalten dort CRLF; alle sechs generierten
Tabellen bleiben gemäß .gitattributes LF. Tabellencheck, beide Formatprüfungen
und alle 236 WebIDL-/CLI-Tests bestanden.

Main und die Dateiliste aller offenen PRs werden vor Upload erneut verglichen.
Jede Datei muss zwischen geprüftem Export und Git-Blob bytegleich sein.
Remote-Branch und PR-Head müssen dem Commit entsprechen. Die ursprüngliche
veränderte Arbeitskopie und ihr echter Index bleiben erhalten; der Commit
verwendet einen separaten temporären Index.

Branch: `codex/m2-webidl-member-attributes`. Laufende GitHub-Prüfungen werden
als laufend gemeldet. Der PR wird vor Abschluss der Remote-CI nicht als gemergt
bezeichnet. Fachliche Schritte, Befehle und aggregierte Ergebnisse werden
veröffentlicht; private Fuzz-Daten bleiben ausgeschlossen.

## 9. Übergabe und weitere M2-Aufgaben

Claude kann besonders Interface versus Callback-Interface, Nullable-Grenzen,
benannte Spezialgetter als reguläre Operationen, erlaubte Promise-Rückgaben und
die Trennung zu Promise-Attributen prüfen. SameObject darf mit jeweils einem
der Setterflags koexistieren; Replaceable und LegacyLenientSetter untereinander
oder mit PutForwards nicht. Dieses Paket erzeugt weder neue Objekte noch
identische Getterwerte, Schatteneigenschaften oder No-op-Setter.

Als nächster eigenständiger Schritt bleibt die vollständige PutForwards-Prüfung
mit Identifierform, Kontext-/Zielbedingungen, Namensauflösung und Kettenzyklen.
Weitere Aufgaben sind Exposed/Global/SecureContext, übrige Legacy-Attribute,
ein vollständiges Metadatenmodell und effektive Overload-Sets. Danach benötigt
der Binding-Generator Argumentauswahl, Konvertierungen, Ausnahmen, Prototypen
und GC-/Host-Verknüpfung. DOM-Bindings, Ereignisschleife, Skripte im Dokument und
WPT/testharness.js bleiben weitere M2-Aufgaben; M2 ist damit nicht abgeschlossen.
