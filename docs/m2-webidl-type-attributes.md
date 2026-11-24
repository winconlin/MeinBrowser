# M2-Arbeitsprotokoll: fünf standardisierte WebIDL-Typattribute

Dieses Paket folgt auf [Array-Verwendungen](m2-webidl-arrays.md).
Es ergänzt die Typstufe um die fünf im Standard als applicable to types
geführten Attribute: Clamp, EnforceRange, LegacyNullToEmptyString, AllowShared
und AllowResizable. Die Prüfung validiert Deklarationen; die entsprechenden
JavaScript-Konvertierungen und der Binding-Generator folgen weiterhin später.

## 1. Merge, Anleitungen und parallele Arbeit prüfen

Basis ist `10d1451f78392dc9107c916de6e6ae46f49add7a`, der bestätigte Merge
von PR #131. Beim ersten Abgleich waren keine weiteren PRs offen. Main und
Überschneidungen mit offenen PRs werden vor Veröffentlichung erneut geprüft.
Der Nutzer hat weitere Vorarbeit erlaubt, solange das Gesamtprojekt und
Claudes parallele Arbeit nicht beeinträchtigt werden.

Gelesen wurden README, SECURITY.md, die M2-Roadmap und die WebIDL-Protokolle
für Typen, Member, Werte, Spezialoperationen, Collections, JSON und Arrays.
Im geprüften Quellbaum gab es keine zusätzliche AGENTS.md, CLAUDE.md oder
CONTRIBUTING.md. Fremde Browser-Crates bleiben nach README ausgeschlossen.

Die ältere veränderte Original-Arbeitskopie samt Index bleibt erhalten.
Umsetzung und Tests verwenden einen sauberen Archivexport der Basis; der
Commit entsteht über einen separaten temporären Git-Index. JavaScript-/RegExp-/
Unicode-Dateien, gemeinsame Architektur-/Fuzzing-Dokumentation, Workflows,
Rohdaten und Datengeneratoren bleiben unverändert.

## 2. Normative Zielklassen und Bindungsgrenzen lesen

Grundlage ist der tatsächlich gelesene WHATWG-WebIDL-Quellstand
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs),
insbesondere [Annotated types](https://webidl.spec.whatwg.org/#idl-annotated-types),
[Extended attributes](https://webidl.spec.whatwg.org/#idl-extended-attributes),
[Clamp](https://webidl.spec.whatwg.org/#Clamp),
[EnforceRange](https://webidl.spec.whatwg.org/#EnforceRange),
[LegacyNullToEmptyString](https://webidl.spec.whatwg.org/#LegacyNullToEmptyString),
[AllowShared](https://webidl.spec.whatwg.org/#AllowShared) und
[AllowResizable](https://webidl.spec.whatwg.org/#AllowResizable).
Die Definitionen von integer types, buffer types und buffer view types wurden
zusätzlich gegen die Listen des Standardstands abgeglichen.

| Typattribut | Zulässiger Zieltyp | Weitere Regel |
| --- | --- | --- |
| Clamp | potenziell nullable Ganzzahl fester Breite | weder mit EnforceRange noch in readonly-Attributen |
| EnforceRange | potenziell nullable Ganzzahl fester Breite | weder mit Clamp noch in readonly-Attributen |
| LegacyNullToEmptyString | nicht-nullable DOMString oder USVString | ByteString und nullable Strings ausgeschlossen |
| AllowShared | potenziell nullable Buffer View | ArrayBuffer und SharedArrayBuffer sind keine Views |
| AllowResizable | potenziell nullable Buffer Source | erlaubt Buffer und Views, auch zusammen mit AllowShared auf Views |

Boolean, BigInt und die Float-/Double-Typen sind keine integer types dieser
Regeln. Buffer Views umfassen DataView und alle im Standardstand vorhandenen
typisierten Arrays, einschließlich Float16Array. Eine Union aus passenden
Blättern wird nicht selbst zur Ganzzahl oder zum Buffer View. Die Regeln
werden daher am assoziierten Typ geprüft und nicht auf beliebige passende
Nachfahren umgedeutet.

Die fünf erkannten Namen werden in der bloßen Flag-Form unterstützt. Zusätze
wie `Clamp=Value`, `Clamp()` oder `AllowShared=(A,B)` ergeben eine Formdiagnose.
Kommentare und Trivia ändern die Form nicht. Wiederholte identische Flags
sind idempotent; Clamp plus EnforceRange ist ein echter Konflikt.
Andere Namen und ihre verschachtelten Inhalte bleiben opaque und ungeprüft,
wie zuvor. Dieses Paket ist kein allgemeiner Extended-Attribute-Validator und
keine Freigabe unbekannter Attribute zur Binding-Codegenerierung.

## 3. Echte Attributlisten einmal erfassen und zuordnen

Der bereits budgetierte Lowering-Durchlauf merkt die Positionen nichtleerer
ExtendedAttributeList-Produktionen vor. Er interpretiert dabei keine Inhalte
und verändert die Grammatik nicht. Die Liste ist nach source_nodes begrenzt
und wird als temporärer Teil des Lowering-Ergebnisses weitergegeben; sie ist
kein neues öffentliches Feld von Types.

`attributes/syntax.rs` liest nur diese echten Listen. Ein iterativer Tokenbesuch
unterscheidet einen der fünf exakten Namen von einem undurchsichtigen Attribut
und prüft bei erkannten Namen die Flag-Form. Namen in `Custom=(Clamp)` werden
nicht als eigenständige Clamp-Annotation behandelt. Eine spätere Suchrunde
über den vollständigen Syntaxbaum entfällt.

| Schreibweise | Zuordnung |
| --- | --- |
| `typedef [Clamp] long Count;` | annotierter Typ |
| `sequence<[Clamp] long>` | Parametertyp, nicht die Sequence |
| `f([Clamp] long n)` | Typ des nicht optional deklarierten Arguments |
| `f([Clamp] long... n)` | Typ des variadischen Arguments |
| `f(optional [Clamp] long n)` | explizit annotierter Argumenttyp |
| `f([Clamp] optional long n)` | falscher Anbringungsort vor optional |
| Dictionary: `[Clamp] long n;` | Typ des nicht erforderlichen Feldes |
| Dictionary: `required [Clamp] long n;` | explizit annotierter Feldtyp |
| Dictionary: `[Clamp] required long n;` | falscher Anbringungsort vor required |
| `[Clamp] attribute long n;` oder `[Clamp] typedef long Count;` | falscher Member-/Definitionskontext |

Die Memberbeschreibung behandelt Variadics als implizit optional. Für die
Assoziation zählt jedoch das ausdrücklich geschriebene optional-Schlüsselwort;
Variadics übernehmen deshalb weiterhin die vorderen Typflags. Callbacks,
Konstruktoren und Async-Iterable-Signaturen verwenden denselben Zuordnungsweg.
Übrig gebliebene erkannte Listen, etwa an Definitionen oder Includes, ergeben
eine Placement-Diagnose. Unbekannte Attributteile bleiben im Originalbaum.

## 4. Typziele und verwendungsabhängige Assoziationen prüfen

Die vorhandene topologische Faktenberechnung erhält eine interne Vierbit-
Zielklassifikation für Ganzzahlen, nicht-nullable DOM-/USV-Strings, Buffer und
Views. Typedefs und Annotationen übernehmen die Zielklasse. Nullable erhält
Ganzzahl-/Buffer-Eigenschaften, entfernt aber die nicht-nullable Stringklasse.
Container und Unions behalten ihre eigenen Klassen.

Lokale Flags liegen in einem dichten Bytevektor. Der iterative Prüfer verwendet
Zustände aus Typ-ID, eingehender Fünfbitmaske und readonly-Kontext. Typedef-,
Annotations-, Nullable- und Unionkanten führen die eingehende Maske weiter.
Containerparameter beginnen mit einer frischen Typmaske; ein umgebendes readonly
bleibt für darin vorkommende Typannotation bestehen. Nominale Dictionary-Felder
sind unabhängige Verwendungen und werden hier nicht pro Attribut erneut expandiert.

Wichtig für gemeinsam genutzte Aliasse: Eingehende Flags werden niemals in die
Typfakten oder in den lokalen Maskenvektor des Aliasziels geschrieben. Ein
`[Clamp] Count` und ein `[EnforceRange] Count` dürfen denselben einfachen Count-
Typ verwenden, ohne gegenseitigen Konflikt oder Verunreinigung einer dritten
readonly-Verwendung von Count. Ein bereits selbst mit Clamp annotierter Alias
plus EnforceRange an seiner Verwendung wird dagegen korrekt abgelehnt.

Ein u64 pro Typ-ID merkt höchstens 32 Masken × 2 readonly-Zustände. Zustände
werden vor dem Einreihen markiert; wiederholte Wege über geteilte Teilgraphen
erzeugen keine exponentielle Traversierung. Es gibt keine Rust-Rekursion und
keine Kopie von Aliasbäumen. Auch die Herkunft einer konkreten Verwendung wird
getrennt von ihrem Aliasziel gehalten.

## 5. Diagnosen und bestehende Regeln erhalten

| Diagnose | Bedeutung und Position |
| --- | --- |
| TypeAttributeForm | erkannter Name mit unzulässigem Zusatz; Attribut selbst |
| TypeAttributePlacement | Typflag an Member/Definition oder vor optional/required; Attribut selbst |
| TypeAttributeTarget | Flag passt nicht zur Typklasse; tatsächliche Annotation-/Argumentverwendung |
| TypeAttributeConflict | Clamp und EnforceRange auf derselben Typverwendung |
| TypeAttributeReadonly | Bereichsflag kommt in einem readonly-Attribut vor; readonly-Typverwendung |

Beispiel: `[AllowShared] Value` als Argumenttyp und `typedef ArrayBuffer Value;`
in einer zweiten Datei zeigen auf die Verwendung in der API-Datei. Ein
readonly-Attribut des gültigen Alias `typedef [Clamp] long Value;` zeigt
ebenfalls auf die readonly-Verwendung statt auf die Aliasdefinition. Die CLI-
Regression prüft drei genaue Dateinamen-/Zeilen-/Spalten-Diagnosen, darunter eine
fehlerhafte Form.

Die neue Prüfung läuft nach den vorhandenen Typ-, Union-, Dictionary-,
Spezialoperations-, Collection-, Wert-, JSON- und Array-Regeln. Deren frühere
spezifische Fehler behalten Vorrang. Innerhalb der neuen Stufe werden Formen
und Anbringungsorte vor den Assoziationszuständen geprüft. Innerhalb eines
Zustands gehen Konflikte unpassenden Zielklassen vor. Ein erfolgreicher Teilaufbau wird bei einem späteren
Fehler oder Hostabbruch nicht zurückgegeben.

Das öffentliche Types-API und seine Knotendarstellung bleiben bestehen. Die
internen fünf Flag-Bedeutungen ersetzen noch kein vollständiges öffentliches
Binding-Metadatenmodell für alle Extended Attributes. Das CLI nennt den
zusätzlichen Umfang und behauptet weiter keine vollständige semantische Gültigkeit.

## 6. Budget, Stresstests und unabhängige Fuzz-Modelle prüfen

Die vorgemerkten Listen sind durch die vorhandene Syntaxknotengrenze beschränkt.
Listen-, Token-, Member-, Argument-, Zustands- und Kantenbesuche belasten das
bestehende gemeinsame Restbudget. Fehlt ein erkannter Typattributname, werden
weder lokale Masken noch der Zustandsgraph angelegt. Die vorhandenen kleinen
Budget-Stresstests bleiben mit ihren bisherigen Quoten erhalten; sie wurden
nicht abgeschwächt oder erhöht. Zusätzliche externe Abhängigkeiten und neue
Limits-Felder sind nicht nötig.

22 neue WebIDL-Tests decken alle Zielklassen, nullable Formen, Flag-Syntax,
Zuordnungspositionen, Konflikte, getrennte Aliasverwendungen, readonly, Dateigrenzen,
exakte Budgetkosten und externen Abbruch ab. Drei Stresstests beziehungsweise
Kontextprüfungen sind besonders relevant:

- 3.000 Container-Schichten und 2.000 Aliasverwendungen auf 1 MiB Stack,
  unter 2.000.000 Typschritten;
- 1.500 Ebenen geteilter Sequence-/Record-Diamanten auf 1 MiB Stack,
  unter 1.500.000 Typschritten;
- Dictionary-Felder als unabhängige Kontexte in readonly FrozenArray-Attributen.

`fuzz/src/idl/attributes.rs` ergänzt das bestehende WebIDL-Ziel um vier
unabhängige Orakeltests: 160 Zieltyp-/Flag-/Nullable-Kombinationen, 80 Kontext-/
Flagkombinationen, 16 unabhängige Alias-Maskenpaare sowie 25 Flag-/Formfälle und
64 gemischte Bytefolgen. Die Tabellen werden vor dem Rendern der Quellen
gewählt; sie lesen keine Attribute-Fakten aus der Implementierung.
Je Modell teilen Parser, Index, Member und Typen 100.000 Schritte. Grenzen:
2 Quellen, 8 KiB Text, 8.192 Syntaxknoten, 1.024 Datensätze,
4.096 Memberbesuche, 1.024 Typknoten, 2.048 Typkanten, 16 Dictionary-Knoten,
512 Dictionary-Kanten, 8.192 Unionbesuche, 16 Interfaces, 4.096 Vergleichspaare.

Zwölf eigene synthetische Seeds entstehen durch
`python3 fuzz/seed_webidl_type_attributes.py`. Zwei Läufe und ein leerer
Zielordner liefern identische Bytes. Alle 344 bisherigen Seeds bleiben
bytegleich; anschließend gibt es 356 insgesamt, davon 114 für WebIDL.
Rohlogs, mutierte Korpora und mögliche Sicherheitsfunde bleiben gemäß
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
| Workspace außerhalb mb-js | 786 Tests bestanden |
| Darin WebIDL / CLI | 202 / 12 Tests bestanden |
| Neu gegenüber Basis | 22 WebIDL-Tests und 1 CLI-Test mit 3 Diagnosen |
| Fuzz-Library | 114 Tests bestanden, darunter 4 neue Modelltests |
| WebIDL-Fuzz-Modelltests | 28 bestanden |
| Python-Fuzz-Läufer | 7 bestanden |
| Linux-Clippy, Windows-Cross-Clippy, Fuzz-Clippy | jeweils mit -D warnings bestanden |
| Beide Formatprüfungen | bestanden |
| Generatoren / Abhängigkeitswächter | alle 6 Tabellen aktuell, keine neue externe Abhängigkeit |
| M1-Referenzen / Fenster-Selbsttest | 398/398, keine einfarbigen Flächen; alle 10 Messpunkte stimmen |
| ASAN/libFuzzer-Build | alle 22 Ziele gebaut; __asan_init im WebIDL-Binary geprüft |
| WebIDL, 30 s, Seed 1 | 2.405 Ausführungen, 270 MiB Spitzen-RSS |
| WebIDL, 30 s, Seed 2 | 2.296 Ausführungen, 419 MiB Spitzen-RSS |

Zusammen 4.701 Ausführungen ohne Fund in diesen beiden begrenzten Läufen,
ohne Zusage allgemeiner Fehlerfreiheit. Der tatsächliche CLI-Lauf mit den
oben genannten Dateien endet mit Exitcode 1 und der erwarteten Diagnose
`api.idl:1:43: TypeAttributeTarget`.

Die Offline-Workspace- und Fensterprüfungen verwenden den vorgesehenen
Tool-Freigabeweg für lokale Unix-Sockets außerhalb der Cloud-Sandbox-Sperre.
Dasselbe gilt für die ASAN-Läufe, damit LeakSanitizer seine abschließende
Prozessinspektion durchführen kann. Weder Tests noch Sanitizer oder Kernel-
Schutzfunktionen wurden abgeschaltet. Vollständige native Plattformtests und
die JavaScript-/Test262-Sammlung laufen in GitHub-CI; lokal wird dafür kein
bestandener Gesamtlauf behauptet. Windows-GNU-Crossprüfungen und CRLF-Checkout
unter Linux ersetzen keinen nativen Windows-Lauf.

## 8. Exakten Commit und Veröffentlichung kontrollieren

Der endgültige Commit wurde mit `core.autocrlf=true` und `core.eol=crlf`
frisch ausgecheckt. Neue Rust-Dateien enthalten dort CRLF; alle sechs generierten
Tabellen bleiben gemäß .gitattributes LF. Tabellencheck, beide Formatprüfungen
und alle 214 WebIDL-/CLI-Tests bestanden.

Die Dateiübersicht wird vor dem Upload mit allen offenen PRs verglichen.
Export und Git-Blobs müssen für jede geänderte Datei bytegleich sein. Remote-
Branch und PR-Head müssen auf den geprüften Commit zeigen. Die ursprüngliche
veränderte Arbeitskopie und ihr echter Index bleiben unverändert; der Commit
verwendet einen getrennten temporären Index.

Branch: `codex/m2-webidl-type-attributes`. Laufende GitHub-Prüfungen werden als
laufend gemeldet, nicht als bestanden. Der PR wird vor Abschluss der Remote-CI
nicht als gemergt bezeichnet. Das Protokoll veröffentlicht aggregierte Ergebnisse
und eigene synthetische Konformitätsfälle; private Fuzz-Daten werden nicht hinzugefügt.

## 9. Übergabe und verbleibende M2-Aufgaben

Claude kann besonders den Unterschied zwischen Buffer und View, BigInt und
Ganzzahl, explizit optional und variadisch, die Richtung von Nullable-Assoziationen,
Alias-Isolation und readonly-Kontexte prüfen. Unbekannte Attribute sind weiter
ungeprüfte Syntax. Weitere Attributformen, Namensverweise und Anwendbarkeit
müssen vor ihrer Verwendung im Binding-Generator gezielt validiert werden.

Offen bleiben insbesondere Exposed/Global/SecureContext, PutForwards/SameObject,
die übrigen Legacy-Attribute, ein vollständiges Metadatenmodell und effektive
Overload-Sets. Danach benötigt der Binding-Generator Argumentauswahl,
Konvertierungen, Ausnahmen, Prototypen und GC-/Host-Verknüpfung. Clamp und
EnforceRange müssen später die normativen Zahlenkonvertierungen ausführen;
AllowShared/AllowResizable müssen echte Buffer-Eigenschaften prüfen. Diese
Deklarationsprüfung implementiert jene Laufzeitsemantik nicht. DOM-Bindings,
Ereignisschleife, Skripte im Dokument und WPT/testharness.js bleiben weitere
M2-Aufgaben; M2 wird durch dieses Paket nicht als abgeschlossen erklärt.
