# M2-Arbeitsprotokoll: toJSON und WebIDL-JSON-Typen

Dieses Paket folgt auf [Collection-Deklarationen](m2-webidl-collections.md).
Es prüft die reservierte reguläre Operation `toJSON` und stellt die JSON-
Typzugehörigkeit über `Types::is_json` bereit. Das ist eine deklarative WebIDL-
Eigenschaft, keine neue JSON-Laufzeit oder Implementierung von JSON.stringify.

## 1. Ausgangsstand, Repository-Anleitungen und Arbeitsbereich prüfen

Basis ist `8c176ffb67e76976e6ce1d975bdc4ef0a630554a`, der Merge von PR #129.
Darin ist inzwischen auch Claudes PR #127 für `import()` und Top-Level-Await
enthalten. Beim Abgleich zu Arbeitsbeginn waren keine weiteren PRs offen.
Vor Veröffentlichung wird dieser Stand erneut geprüft.

Gelesen wurden README-Bauanleitung, SECURITY.md, die M2-Roadmap und die
WebIDL-Arbeitsprotokolle zu Typen, Werten, Spezialoperationen und Collections.
Es gab im untersuchten Quellbaum keine zusätzliche AGENTS.md, CLAUDE.md oder
CONTRIBUTING.md. Fremde Browser-Crates bleiben gemäß README ausgeschlossen.
Die Cloud-Shell und die konfigurierte GitHub-Verbindung wurden vor dem
Repository-Zugriff geprüft; ein fehlender temporärer Sandbox-Pfad wurde als
Umgebungsproblem behoben, ohne Repository-Dateien zu ändern.

Die ältere veränderte Original-Arbeitskopie und ihr Index bleiben erhalten.
Umsetzung und Tests erfolgen in einem sauberen Export der genannten Basis.
JavaScript-/RegExp-/Unicode-Code, gemeinsame Architektur-/Fuzzing-Dokumentation,
Workflows und Datengeneratoren liegen außerhalb dieses Pakets. Der spätere
Commit verwendet wieder einen separaten temporären Git-Index.

## 2. Normative Anforderungen und Typklassen einzeln lesen

Grundlage ist der tatsächlich gelesene WHATWG-WebIDL-Quellstand
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs),
insbesondere toJSON/JSON types, die Definition regular operation, die Listen
numeric/primitive/string types und die bereits verwendeten Dictionary-,
Nullable-, Union- und Typedef-Regeln.

| Typgruppe | JSON-Typbedingung |
| --- | --- |
| Numerische Typen einschließlich unrestricted | ja |
| Boolean, DOMString, ByteString, USVString, Enums | ja |
| object | ja |
| Nullable, Annotation, Typedef | innerer Typ ist JSON-Typ |
| Union | alle Mitgliedstypen sind JSON-Typen |
| Sequence und FrozenArray | Parameter ist JSON-Typ |
| Record | Wertparameter ist JSON-Typ |
| Dictionary | alle eigenen, partiellen und geerbten Felder haben JSON-Typen |
| Gewöhnliches Interface | reguläre toJSON-Operation auf sich oder einem Vorfahren |
| BigInt, any, undefined, Symbol und Buffertypen | nein |
| Promise, AsyncSequence und ObservableArray | nein |
| Callback-Funktion und Callback-Interface als Werttyp | nein |

BigInt gehört laut Standard zwar zu primitive types, aber nicht zu numeric
types. Es wird deshalb nicht versehentlich zusammen mit den numerischen Typen
freigegeben. Umgekehrt bleibt unrestricted double ein JSON-Typ; der Umgang
mit konkreten Infinity-/NaN-Werten gehört zu den späteren Bindings.
Ein leeres Dictionary erfüllt die All-Felder-Bedingung. `object` kann ein
JSON-Typ sein, obwohl konkrete Objektwerte zur Laufzeit problematisch sein können.

## 3. Reguläre toJSON-Operationen und nominale Interface-Präsenz trennen

Die reservierte reguläre Operation muss **null deklarierte Argumente** besitzen
und einen JSON-Typ zurückgeben. Auch optionale Argumente, Argumente mit Defaults
und variadische Argumente verletzen die Null-Argument-Regel. Ein Name mit
Escape-Unterstrich wird wie bei der Memberstufe normalisiert.

Ein benannter, nicht-statischer Getter/Setter/Deleter ist zugleich eine reguläre
Operation. Ein ansonsten gültiger benannter Getter `toJSON(DOMString key)`
verletzt daher die Null-Argument-Regel. Statische Operationen sind keine regulären
Operationen und werden von diesen reservierten Regeln ausgenommen. Ein Attribut
namens `toJSON` ist ebenfalls kein Operationsbeleg.

Die Signatur-/Rückgaberegeln gelten für die erfassten regulären Operationen in
Interfaces, Mixins, Namespaces und Callback-Interfaces. Für die nominale JSON-
Typzugehörigkeit zählen dagegen nur gewöhnliche Interface-Typen. Ein Callback-
Interface wird durch eine Methode namens `toJSON` nicht zum JSON-Interface-Typ.

Pro Interface/Mixin wird die eigene Präsenz einmal erfasst. Includes liefern
den Beleg für den jeweiligen Host; Vorfahren reichen ihn an Nachfahren weiter.
Geschwister und verschiedene Mixin-Hosts bleiben getrennt. Ein geerbter Beleg
wird durch ein gleichnamiges statisches Member oder ein Attribut nicht aus der
normativen Präsenzbedingung entfernt. Das ist keine spätere Property-Dispatch-
Entscheidung und kein Beleg, dass ein konkreter JS-Propertyzugriff aufrufbar ist.

Interfaces sind bei der JSON-Typklassifikation nominale Blätter. `I toJSON()`
auf I und gegenseitig A/B zurückgebende Interfaces benötigen deshalb keine
rekursive Auswertung ihrer Methoden. Ihre Signaturen und Rückgabetypen werden
anschließend trotzdem geprüft; ein fehlerhafter Beleg liefert keinen gültigen
Typgraphen zurück. Prose, native Algorithmen und `[Default]`-Verhalten werden
hier nicht implementiert.

## 4. Gemeinsamen Typ-/Dictionary-Graphen aufbauen

Neue Teile sind `types/json.rs`, `types/json/interfaces.rs` und
`types/json/tests.rs`. Die Prüfung läuft in `types::check` nach den bisherigen
Wert-/Defaultregeln. Bereits vorhandene Typ-, Dictionary-, Spezialoperations-
oder Collection-Fehler können deshalb früher erscheinen.

Der Graph verwendet die vorhandenen Typknoten und einen dichten Index der
Dictionary-IDs. Kanten führen von Typedef/Nullable/Annotation zu ihrem inneren
Typ, von Unions zu Mitgliedern, von Sequence/FrozenArray/Record zum relevanten
Parameter sowie von Dictionary-Typen zu ihrer Definition. Dictionaries führen
zu Vorfahren und eigenen Feldtypen; Partials sind in der Memberbeschreibung
bereits zusammengeführt. Felder oder Vorfahrenlisten werden nicht pro
Verwendungsort kopiert.

Eine iterative Tiefensuche mit Farben und booleschem All-Dependencies-Ergebnis
besucht jeden Knoten einmal. Ein abgeschlossener Untergraph liefert seinen
Memo-Wert; er wird nicht erneut expandiert. Typ- und Dictionary-Farben/Werte
liegen in getrennten flachen Vektoren, der Arbeitsstapel enthält flache Frames.
Die bisherige Dictionary-Zyklenprüfung läuft vorher. Eine defensive Erkennung
aktiver Rückkanten bleibt zusätzlich erhalten.

JSON-Kanten unterscheiden sich von Dictionary-Einschlusskanten: Promise und
AsyncSequence sind nicht-JSON-Blätter. Etwa `dictionary D { Promise<D> future; }`
bleibt deshalb kein JSON-Typ, ohne die Promise-Kante als neue rekursive JSON-
Abhängigkeit zu expandieren. Interface-Rückgabemethoden erzeugen ebenfalls
keine Datenkanten. Die Klassifikation bestätigt keine Serialisierbarkeit
beliebiger konkreter oder zyklischer Laufzeitobjekte.

## 5. API, Diagnosen und gemeinsame Quoten integrieren

`Types::is_json(id) -> Option<bool>` liefert die WebIDL-JSON-Typzugehörigkeit.
IDs gehören wie bisher ausschließlich zum zugehörigen Typgraphen. Alle
zurückgegebenen Types besitzen die vollständig berechneten Fakten, auch wenn
sie keine toJSON-Methode verwenden. Das Ergebnis liegt als zusätzliches boolesches
Faktum in der bestehenden Typbeschreibung, nicht als neue Kopie des Graphen.

```rust
let sources = ["dictionary Data { long count; }; interface I { Data toJSON(); };"];
let index = mb_webidl::index::build(&sources).unwrap();
let members = mb_webidl::members::build(&index).unwrap();
let types = mb_webidl::types::build(&index, &members).unwrap();
let method = &members.members(index.find("I").unwrap()).unwrap()[0];
let id = types.at(method.location.source, method.types[0].unwrap()).unwrap();
assert_eq!(types.is_json(id), Some(true));
```

Zwei zusätzliche Fehler sind verfügbar: `ToJSONArguments` zeigt auf den Namen
des ersten Arguments, `ToJSONType` auf die tatsächliche Rückgabetyp-Verwendung.
Ein Alias oder ein unpassendes Dictionary in einer anderen Datei verschiebt
die Rückgabediagnose nicht dorthin. Eine vollständige Begründungskette bis zum
innersten nicht-JSON-Feld wird noch nicht ausgegeben.

Neue Limits-Felder oder externe Abhängigkeiten sind nicht nötig. Typzahl,
Dictionary-Knoten und Datensätze sind bereits begrenzt; die zusätzlichen
Vektoren und ID-Maps bleiben dadurch beschränkt. Jeder Graphbesuch, Feld-/
Vorfahrenübergang, Methoden- und Include-Check sowie die Übernahme der Fakten
verbraucht den Rest desselben `Budget` samt Abbruchsignal. Die Typstufe bleibt
bei zehn Millionen Standard-Schritten, das Werkzeug bei 45 Millionen gemeinsamen
Schritten. Es gibt kein neues Budget pro Dictionary oder Methode.

Die Klassifikation ist linear in den tatsächlich vorhandenen Knoten und
impliziten Kanten, zusätzlich zu logarithmischen ID-Map-Zugriffen. Die Graphen
werden nicht als expandierte Bäume materialisiert. Logische Quoten sind keine
vollständige RAM-/Zeitmessung; Allokationen und einzelne Map-Operationen haben
keine inneren Abbruchpunkte. Die früheren Memberprüfungen können große Includes
weiterhin mehrfach prüfen. Fehler oder erschöpfte Arbeit liefern keinen
teilweise geprüften Typgraphen aus.

## 6. Regressionen und unabhängige Fuzz-Orakel ergänzen

Neue gezielte Tests decken sämtliche Grundklassen, Nullable/Annotation/Aliasse,
Unions, Container, Dictionary-Parents/Partials, reguläre und statische Operationen,
benannte Getter, Namespaces/Callbacks, Selbst-/gegenseitige Interface-Rückgaben,
Mixin-Hosts, Geschwister, Dateipositionen, exakte Budgets und externen Abbruch ab.
Die bestehenden Tests bleiben erhalten. Der allgemeine WebIDL-Fuzz-Graphcheck
prüft zusätzlich, dass für jede lokale TypeId eine JSON-Antwort vorhanden ist.

Auf einem Faden mit 1 MiB Rust-Stapel werden Aufbau, Prüfung und Freigabe von
2.000 gemeinsamen Dictionary-Schichten mit je zwei Feldern zum Vorgänger
geprüft, sowohl mit gültigem als auch nicht-JSON-Blatt, unter 500.000 Typ-
Schritten. Vollständiges Kopieren hätte exponentielle Größe. Eine rückwärts
deklarierte 2.000-stufige Dictionary-Vererbung passt ebenfalls unter 500.000
Schritte; 2.000 Interface-Vorfahren mit geerbtem Operationsbeleg unter 250.000.

Das neue Fuzz-Untermodul `fuzz/src/idl/json.rs` nutzt vier unabhängige Orakel:
128 Blatt-/Containerparameter, 448 Dictionary-Graphparameter, 128 Interface-
Präsenzparameter und 16 Methodensignaturen. Dictionary- und Interface-Erwartungen
werden mit kleinen booleschen Erreichbarkeitsmatrizen berechnet; die Implementierung
verwendet stattdessen gemeinsame Graph-Memos und iterative Vorfahrenauflösung.
Weitere gemischte Bytefolgen prüfen alle Einstiege gemeinsam.

Die Modelle halten Vorstufen gültig: Feldnamen sind pro Dictionary-Schicht
verschieden und Abhängigkeiten gehen zu niedrigeren IDs. Mixin-Includes werden
nur erzeugt, wenn sie nicht bereits durch vorhandene Membernamen ungültig sind.
Die Namen-/Typregeln wurden für Fuzzing nicht abgeschwächt.

Je Modell teilen Parser, Index, Member und Typen 100.000 Schritte. Grenzen:
zwei Quellen, 8 KiB Quelltext, 8.192 Syntaxknoten, je 1.024 Datensätze,
4.096 Memberbesuche, 1.024 Typknoten, 2.048 Kanten, 16 Dictionarys, 512
Dictionary-Kanten, 8.192 Unionbesuche, 16 Interfaces und 4.096 Vergleichspaare.

Zehn neue eigene Seeds unter `fuzz/seeds/webidl/json-*` sind mit
`python3 fuzz/seed_webidl_json.py` reproduzierbar. Alle 322 bisherigen Seeds
bleiben bytegleich; insgesamt sind es danach 332. Wiederholung und Reproduktion
aus einem leeren Verzeichnis wurden bytegleich geprüft. Gemeinsamer Generator,
Zielliste und Browser-Abhängigkeiten bleiben unverändert. Rohe Logs, mutierte
Korpora und mögliche Funde bleiben gemäß SECURITY.md privat.

## 7. Prüfungen nach Repository-Anleitung ausführen

Die Testdaten wurden vor den breiten Tests abgeglichen. Vorhandene Caches und
`--offline` wurden genutzt; die Befehle folgen der README:

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

Die aktuelle lokale Sandbox verweigerte dem unveränderten X11-Test
`expired_deadline_still_reads_waiting_events` das Schreiben auf sein lokales
Unix-Socketpaar mit EPERM. Der Offline-Workspace-Lauf und die Fensterprüfung
wurden über den vorgesehenen Tool-Freigabeweg außerhalb dieser Socket-Sperre
wiederholt. Auch der erste begrenzte ASAN-Lauf konnte seine abschließende
LeakSanitizer-Prozessinspektion wegen der Sandbox-ptrace-Sperre nicht abschließen.
Beide Läufe wurden mit aktiven Sanitizern über denselben Tool-Freigabeweg
außerhalb der Sperre wiederholt. Es wurden weder Sanitizer noch Kernel-Schutz-
funktionen abgeschaltet; kein Test wurde entfernt und kein Repository-Code
für diese Umgebungsgrenzen verändert.

| Prüfung | Lokales Ergebnis |
| --- | --- |
| Workspace außerhalb von mb-js | 745 Tests bestanden |
| Darin WebIDL / CLI | 163 / 10 Tests bestanden |
| Neu gegenüber der Basis | 18 WebIDL-Tests und 1 CLI-Test mit 2 Diagnosen |
| Fuzz-Library | 107 Tests bestanden, davon 4 neue Modelltests |
| Python-Fuzz-Läufer | 7 Tests bestanden |
| Linux-Clippy, Windows-Cross-Clippy, Fuzz-Clippy | jeweils `-D warnings` bestanden |
| Beide Formatprüfungen | bestanden |
| Generatoren / Abhängigkeitswächter | alle 6 Tabellen aktuell, keine neue externe Abhängigkeit |
| M1-Referenzen | 398/398 bestanden, keine einfarbigen Flächen |
| Fenster-Selbsttest | alle 10 Messpunkte stimmen |
| ASAN/libFuzzer-Build | alle 22 Ziele gebaut; `__asan_init` im WebIDL-Binary geprüft |
| WebIDL, 30 s, Seed 1 | 2.903 Ausführungen, 421 MiB Spitzen-RSS |
| WebIDL, 30 s, Seed 2 | 2.955 Ausführungen, 428 MiB Spitzen-RSS |

Zusammen 5.858 Ausführungen ohne Fund in diesen begrenzten Läufen, ohne
Zusage allgemeiner Fehlerfreiheit. Die vollständige JavaScript-/Test262-
Sammlung wird lokal nicht erneut als bestanden behauptet: Der Workspace-Test
schließt `mb-js` ausdrücklich aus. Dessen Targets wurden mit Clippy kompiliert
und die Fuzz-Harnesses getestet; native Plattformtests einschließlich JS laufen
in GitHub-CI.

Ein tatsächlicher CLI-Lauf mit `Data toJSON()` und `dictionary Data { any value; }`
endet mit Exitcode 1 und `api.idl:1:15: ToJSONType`. Die Diagnose zeigt auf die
Rückgabetyp-Verwendung. Der CLI-Regressionstest prüft zusätzlich den Argumentnamen
in einer anderen Quelldatei.


## 8. Veröffentlichung und Überschneidungen kontrollieren

Der konkrete Commit wurde vor dem Upload mit `core.autocrlf=true` und
`core.eol=crlf` frisch ausgecheckt. Neue Rust-Dateien enthalten dort CRLF,
alle sechs generierten Tabellen bleiben gemäß `.gitattributes` LF. Tabellencheck,
beide Formatprüfungen und alle 173 WebIDL-/CLI-Tests bestanden. Dieser Linux-
Checkout und Windows-Cross-Clippy ersetzen keinen nativen Windows-Lauf.

Veröffentlicht werden nur die abgegrenzten WebIDL-Dateien, das gezielte Fuzz-
Untermodul, der API-Faktencheck, die zehn eigenen Seeds und zugehörige Dokumente.
Die Dateiliste wird vor dem Upload mit allen dann offenen PRs abgeglichen.
Für jede Datei wird Bytegleichheit zwischen geprüftem Export und Git-Blob
kontrolliert. Remote-Branch und PR-Head müssen dem Commit entsprechen.
Original-Arbeitskopie und Index bleiben unverändert; rohe Logs und mutierte
Korpora werden nicht hinzugefügt.

Branch: `codex/m2-webidl-json-types`. Laufende GitHub-Prüfungen werden als
laufend gemeldet, nicht als bestanden. Der PR wird vor Abschluss der Remote-CI
nicht als gemergt bezeichnet. Dieses Protokoll hält fachliche Schritte,
Befehle und aggregierte Ergebnisse fest, keine privaten Fuzz-Rohdaten.

## 9. Übergabe und verbleibende M2-Punkte

Claude kann besonders die Unterscheidung numeric/primitive (BigInt), die
regulären Operationen trotz Spezialschlüsselwort, Callback-Interface versus
gewöhnliches Interface, Includes/Vorfahren und die unterschiedlichen Dictionary-
und JSON-Abhängigkeiten prüfen. Das neue API belegt Typzugehörigkeit, weder
konkrete JSON-Daten noch sichere native oder JavaScript-Laufzeitalgorithmen.

Die [Array-Verwendungsregeln](m2-webidl-arrays.md) sind inzwischen implementiert.
Allgemeine Extended Attributes einschließlich Assoziation, effektive Overload-Sets und
vollständige Binding-Konvertierungen bleiben ebenfalls offen. Der Binding-
Generator benötigt danach Argumentauswahl, Konvertierung, Ausnahmen, Prototypen
und GC-/Host-Verknüpfung. DOM-Bindings, Ereignisschleife, Skripte im Dokument und
WPT/testharness.js bleiben weitere M2-Aufgaben. Claudes JavaScript-Arbeit wird
nicht übernommen; M2 ist damit nicht abgeschlossen.
