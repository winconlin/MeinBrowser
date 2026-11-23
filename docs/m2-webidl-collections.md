# M2-Arbeitsprotokoll: WebIDL-Collection-Deklarationen

Dieses Paket folgt auf [Spezialoperationen/Stringifier](m2-webidl-special-operations.md).
Es prüft die Deklarationen `iterable`, `async_iterable`, `maplike` und `setlike`,
Namenskonflikte und die IDL-Grundtypen eines Value-Iterators. Es implementiert
keine ECMAScript-Iteratoren oder nativen Map-/Set-Laufzeitobjekte.

## 1. Ausgangsstand, Anleitungen und Zuständigkeit prüfen

Basis ist `9ab82679b66224f674ada4a8afa4464d0b5a0f4c`, der Merge von PR #128.
Gelesen wurden README-Bauanleitung, SECURITY.md, die M2-Roadmap und die
WebIDL-Dokumentation zu Membern, Typen und Spezialoperationen. Es gab keine
zusätzliche AGENTS.md, CLAUDE.md oder CONTRIBUTING.md im untersuchten Quellbaum.
Die README verlangt eigenen Browsercode ohne neue externe Crates.

Claudes PR #127 bearbeitet weiter `import()` und Top-Level-Await. Dessen
27 Dateien wurden vor Arbeitsbeginn abgeglichen. JavaScript-/RegExp-/Unicode-
Code, die gemeinsame Architektur- und Fuzzing-Dokumentation, Workflows und
Datengeneratoren bleiben außerhalb dieses Pakets. Vor Veröffentlichung wird
der Abgleich mit allen offenen PRs wiederholt.

Die ältere, bereits veränderte Original-Arbeitskopie bleibt erhalten. Umsetzung
und Tests laufen in einem sauberen Export der Basis; der spätere Commit wird
über einen separaten temporären Git-Index erstellt. Es wird kein fremder
Browsercode oder neues Browser-Crate übernommen.

## 2. Normative Regeln lesen und in Prüffälle übersetzen

Tatsächlich gelesene Grundlage ist derselbe WHATWG-WebIDL-Stand wie bei den
bisherigen Stufen:
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs).
Gelesen wurden Iterable declarations, Asynchronously iterable declarations,
Maplike declarations und Setlike declarations, außerdem die Definition regular
operation, Union/Nullable/Annotated types und die zugehörige Grammatik.

| Normative Bedingung | Ergebnis bei Verstoß |
| --- | --- |
| Je Interface höchstens eine Collection-Deklaration, keine Mischung der Arten | `CollectionConflict` |
| Keine weitere Collection auf einem Vorfahren einer neuen Collection | `CollectionConflict` |
| Sync-Value-Iterator benötigt Indexed Getter | `ValueIteratorGetter` |
| Value-Iterator und Indexed Getter benötigen denselben IDL-Grundtyp | `ValueIteratorType` |
| Sync-Pair-Iterator darf keine Indexed Properties haben | `CollectionIndexedGetter` |
| Maplike/Setlike samt Vorfahren dürfen keinen Indexed Getter haben | `CollectionIndexedGetter` |
| Argumente von Async-Iterables sind optional | `AsyncIteratorArgument` |
| Keine expliziten Member unter den je Art reservierten Namen | `CollectionMemberName` |

`readonly maplike` und `readonly setlike` zählen jeweils zur selben Art wie
ihre schreibbare Form. Auch Deklarationen in Partials und über längere
Vorfahrenketten werden zusammengeführt. Geschwisterinterfaces bleiben getrennt.
Für Async-Iterables gibt es keine entsprechende Indexed-Getter-/Werttypbindung;
die Sync-Regeln werden nicht auf sie übertragen. Variadische letzte Argumente
sind implizit optional und erfüllen deshalb die Async-Optionalitätsbedingung.

Ein benannter, nicht-statischer Getter/Setter/Deleter deklariert laut dem
Operations-Abschnitt zugleich eine reguläre Operation. Er zählt deshalb bei
Namenskonflikten mit. Statische Operationen gehören nicht zu regular operations;
statische Attribute zählen dagegen zum ausdrücklich genannten Attributverbot.

## 3. Namen, Vererbung und erlaubte Überschreibungen trennen

| Collection | Verbotene Attribut-/Konstanten-/reguläre Operationsnamen |
| --- | --- |
| Iterable | entries, forEach, keys, values |
| Async Iterable, auch mit nur einem Werttyp | entries, keys, values |
| Maplike, readonly oder schreibbar | entries, forEach, get, has, keys, size, values |
| Setlike, readonly oder schreibbar | entries, forEach, has, keys, size, values |

Zusätzlich verbieten **schreibbare** Maplikes Attribute/Konstanten namens clear,
delete und set; schreibbare Setlikes solche namens add, clear und delete.
Reguläre Operationen unter diesen Mutatornamen sind ausdrücklich erlaubt.
Ihre tatsächlichen Ein-/Ausgabeerwartungen und Implementierungen sind hier
noch nicht als passend bestätigt. Diese Prüfung erteilt keine Freigabe für
ungeprüfte Mutator-Bindings oder Overloads.

Die Verbote betrachten das deklarierende Interface einschließlich Partials,
Mixins und seiner Vorfahren. Auch ein überschattetes Attribut eines Vorfahren
zählt für dessen ausdrückliches Verbot. Ein neues Interface ohne eigene
Collection wird nicht allein wegen eines vom Vorfahren geerbten generierten
Methodennamens pauschal eingeschränkt; die zitierten Regeln nennen den
Deklarationsinhaber und dessen Vorfahren. Diese Richtung wird gezielt getestet.
Ein Mixin wird pro Host berücksichtigt, ohne dessen vollständige Memberliste
für jeden Host in der neuen Stufe zu kopieren.

## 4. IDL-Grundtypen vergleichen und Arbeit begrenzen

`collections/identity.rs` löst Typedef- und Annotationshüllen einmal pro
Typknoten auf. Nominale Interface-, Enum-, Dictionary- und Callback-IDs bleiben
verschieden. Primitive Typen behalten Breite, Vorzeichen und unrestricted;
Bufferarten und die drei Stringtypen bleiben ebenfalls verschieden. Container-
und Recordparameter werden iterativ verglichen, einschließlich des Record-
Schlüsseltyps. Gleichheit bedeutet hier Grundtypgleichheit, keine Untertyp-
Beziehung und keine ECMAScript-Coercion.

Unions werden nur bei einem tatsächlich benötigten Vergleich abgeflacht.
Mitgliedsreihenfolge und zusätzliche Unionklammern verändern den Grundtyp nicht.
Nullable-Markierungen auf Union oder abgeflachtem Mitglied werden zu einer
Nullmöglichkeit zusammengefasst; innerhalb eines Containers bleibt Nullable
Teil seines Parametertyps. Beispiel: `(long or DOMString)?` und
`(DOMString or long?)` haben in dieser Prüfung dieselben Grundwerte. Unions
wurden zuvor bereits auf Form und Unterscheidbarkeit geprüft. Die Abflachung
baut nicht für jede verschachtelte Union eine Liste sämtlicher Präfixglieder auf.

Vergleichspaare werden symmetrisch normalisiert und gemeinsam zwischengespeichert.
Nur nach einem vollständig erfolgreichen Vergleich werden dessen besuchte Paare
als gleich markiert. Ein Fehlschlag bestätigt keine noch ungeprüften Teilpaare.
Offene Arbeit liegt in einem flachen Stapel; die Quotenprüfung erfolgt bereits
vor dem Vormerken eines neuen Paars, nicht erst nach einer großen Warteschlange.

Neues Feld `types::Limits::collection_pairs`: standardmäßig **200.000** verschiedene
vorgemerkte oder gespeicherte Typknotenpaare, geteilt über alle Collections.
Überschreitung liefert `Limit::CollectionPairs` als Hostgrenze. Physisch identische
Grundtyp-IDs brauchen kein neues Paar; reine Map-/Set-Deklarationen verbrauchen
keine Vergleichspaare. Null ist deshalb ein sinnvoll prüfbarer Grenzwert.

Annotationen werden für diesen Grundtypvergleich aufgelöst. Allgemeine Extended
Attributes, deren Assoziation und Konvertierungsbedingungen bleiben separat
zu prüfen. Die neue Vergleichsfunktion ist keine vollständige Binding-Typ-
Äquivalenzprüfung und wird nicht als öffentliche Runtime-API angeboten.

## 5. Prüfung integrieren und Ressourcen abrechnen

Neue Implementierung: `types/collections.rs`, `types/collections/identity.rs`
und `types/collections/tests.rs`. Die Einbindung in `types::check` erfolgt nach
Spezialoperationen und vor der Wert-/Defaultprüfung. Vorherige Typ-, Union-,
Dictionary- und Getter-/length-Fehler können daher weiterhin früher erscheinen.
Bei mehreren Verstößen ist keine beliebige gewünschte Einzeldiagnose garantiert.

Pro Interface/Mixin genügt eine feste Zusammenfassung: Collection, Indexed
Getter und elf relevante Namen. Geerbte Zusammenfassungen werden einmal
berechnet; es entstehen keine vollständigen Vorfahren-/Memberkopien. Ein
Attribut-/Konstantenbeleg wird gegenüber einem regulären Operationsbeleg
bevorzugt, damit das engere Mutator-Attributverbot nicht durch Shadowing verloren geht.

Ohne Collection beendet sich die neue Stufe nach einem begrenzten Member-Scan.
Ansonsten bezahlen Typvektoren, Graphbesuche, Argumente, Namen, Includes,
Vorfahren, Abflachung und Vergleichspaare den Rest desselben `Budget` samt
Abbruchsignal. Die Typstufe bleibt bei zehn Millionen Standard-Schritten;
`webidl-check` teilt weiterhin 45 Millionen Schritte über alle Stufen.

Die Paarquote gilt auch bei unbegrenztem Schrittbudget. Die bestehenden Quell-,
Typ-, Kanten- und Datensatzgrenzen begrenzen zusätzlich Vektoren und Metadaten.
Map-/Set-Zugriffe sind logarithmisch. Logische Quoten sind keine vollständige
RAM-/Zeitmessung; Allokationen und einzelne Map-Zugriffe haben keine inneren
Abbruchpunkte. Viele verschiedene Vergleiche großer geteilter Untertypen können
trotz Cache erneut Arbeit benötigen; universell lineare Gesamtzeit wird nicht
behauptet. Die frühere Member-Namensprüfung kann große Includes weiterhin
mehrfach prüfen; diese Kosten werden hier nicht beseitigt.

## 6. Regressionen, Vorstufen und Fuzz-Orakel prüfen

Der frühere Mapping-Test deklarierte alle vier Collection-Arten auf demselben
Interface. Das ist nach den jetzt implementierten Regeln ungültig. Er wurde
auf getrennte Interfaces umgestellt; seine vollständigen Mapping-Assertions
für Member, Argumente, Callbacks und Typedefs bleiben erhalten.

Neue Regressionen prüfen sämtliche lokalen und geerbten Collection-Paare,
396 Kombinationen aus Collection, Name und Memberart, Mutator-Ausnahmen,
Mixins, echte Dateipositionen, Async-Optionalität, Getterbeziehungen, Alias-
und nominale Identität, Container, Unions, exakte Paarquoten und gemeinsames
Budget. Ein eigener Test verhindert, dass fehlgeschlagene Vergleiche den
Gleichheitscache mit ungeprüften positiven Aussagen füllen.

Auf einem Rust-Faden mit 1 MiB Stack werden geprüft:

- 2.000 verschachtelte Sequence-Parameter auf beiden Seiten unter 750.000
  Typ-Schritten, einschließlich Freigabe.
- Tiefe Unions über 1.000 verschiedene Interfaces mit umgekehrter Reihenfolge
  unter 600.000 Typ-Schritten.
- 2.000 Vorfahren vor einer Collection, mit gültigem und mit reserviertem
  Vorfahrennamen, unter jeweils 500.000 Typ-Schritten.
- 1.000 Hosts, die einen Mixin mit 1.000 Attributen teilen, unter 300.000
  Typ-Schritten. Die ältere Member-Namensprüfung erhält separat drei Millionen
  Besuche und 20 Millionen Schritte; die gesamte Pipeline wird nicht als
  entsprechend billig behauptet.

Das neue Fuzz-Untermodul `fuzz/src/idl/collections.rs` nutzt vier unabhängige
Orakel: 1.024 Deklarations-/Vererbungs-/Index-/Duplikatvarianten, 1.188 Namens-/
Memberart-/Positionsparameter, 576 Paare vorab gewählter Typäquivalenzklassen
und 64 Async-Argument-/Flagvarianten. Die Orakel arbeiten mit festen Listen,
booleschen Beziehungen und vorher vergebenen Klassen, nicht mit den Maps und
Typvergleichsstapeln der Implementierung. Gemischte Bytefolgen prüfen zusätzlich
alle vier Einstiege gemeinsam.

Mixins erlauben grammatisch weder statische Member noch Spezialoperationen.
Das Namensmodell erzeugt solche Varianten daher am Host; die Zahl 1.188 ist
eine Zahl geprüfter Parametervarianten, nicht unterschiedlicher gültiger Mixin-
Grammatiken. Der Parser wurde für die Modellgenerierung nicht abgeschwächt.

Jedes Modell teilt 100.000 Schritte über Parser, Index, Member und Typen.
Grenzen: zwei Quellen, 8 KiB Quelltext, 8.192 Syntaxknoten, je 1.024 Datensätze,
4.096 Memberbesuche, 1.024 Typknoten, 2.048 Kanten, 16 Dictionarys, 512
Dictionary-Kanten, 8.192 Unionbesuche, 16 Interfaces und 4.096 Vergleichspaare.
Die bisherigen WebIDL-Fuzz-Modelle erhalten ebenfalls 4.096 Vergleichspaare.

Zehn neue eigene Seeds unter `fuzz/seeds/webidl/collection-*` lassen sich mit
`python3 fuzz/seed_webidl_collections.py` reproduzieren. Alle 310 bisherigen
Seeds bleiben bytegleich; insgesamt sind es 320. Zweiter Lauf und Reproduktion
aus einem leeren Verzeichnis wurden bytegleich geprüft. Gemeinsamer Seed-
Generator und Fuzz-Zielliste bleiben unverändert. Rohe Logs, mutierte Korpora
und mögliche Funde bleiben gemäß SECURITY.md privat.

## 7. Prüfungen nach Repository-Anleitung ausführen

Die Testdaten wurden vor den breiten Tests abgeglichen. In der Cloud wurden
vorhandene Caches und `--offline` genutzt; die Befehle folgen der README:

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
| Workspace außerhalb von mb-js | 726 Tests bestanden |
| Darin WebIDL / CLI | 145 / 9 Tests bestanden |
| Neu gegenüber der Basis | 18 WebIDL-Tests und 1 CLI-Test mit 3 Diagnosen |
| Gesamte Fuzz-Library | 103 Tests bestanden, davon 4 neue Modelltests |
| Python-Fuzz-Läufer | 7 Tests bestanden |
| Linux-Clippy, Windows-Cross-Clippy, Fuzz-Clippy | jeweils `-D warnings` bestanden |
| Beide Formatprüfungen | bestanden |
| Generatoren / Abhängigkeitswächter | alle 6 Tabellen aktuell, keine neue externe Abhängigkeit |
| M1-Referenzen | 398/398 bestanden, keine einfarbigen Flächen |
| Fenster-Selbsttest | alle 10 Messpunkte stimmen |
| ASAN/libFuzzer-Build | alle 22 Ziele gebaut; `__asan_init` im WebIDL-Binary geprüft |
| WebIDL, 30 s, Seed 1 | 3.573 Ausführungen, 258 MiB Spitzen-RSS |
| WebIDL, 30 s, Seed 2 | 3.531 Ausführungen, 409 MiB Spitzen-RSS |

Zusammen 7.104 Ausführungen ohne Fund in diesen begrenzten Läufen, ohne
Zusage allgemeiner Fehlerfreiheit. Die vollständige JavaScript-/Test262-
Sammlung wird lokal nicht erneut als bestanden behauptet: Der Workspace-Test
schließt `mb-js` ausdrücklich aus. Dessen Targets wurden mit Clippy kompiliert
und die Fuzz-Harnesses getestet; native Plattformtests einschließlich JavaScript
laufen in GitHub-CI.

Ein tatsächlicher CLI-Lauf mit Maplike im Child und einem geerbten Attribut
`keys` beendet sich mit Exitcode 1 und `base.idl:1:42: CollectionMemberName`.
Die Diagnose zeigt damit auf das wirkliche kollidierende Attribut. Ein weiterer
CLI-Regressionstest prüft Iterator-Typ und Pflichtargument eines Async-Iterators.


## 8. Veröffentlichung ohne Überschneidungen prüfen

Der konkrete Commit wurde vor dem Upload mit `core.autocrlf=true` und
`core.eol=crlf` frisch ausgecheckt. Neue Rust-Dateien enthalten dort CRLF,
alle sechs generierten Tabellen bleiben gemäß `.gitattributes` LF. Tabellencheck,
beide Formatprüfungen und alle 154 WebIDL-/CLI-Tests bestanden. Dieser Linux-
Checkout und Windows-Cross-Clippy ersetzen keinen nativen Windows-Lauf.

Veröffentlicht werden nur die abgegrenzten WebIDL-Dateien, das gezielte Fuzz-
Untermodul, die Ergänzung der Paarquote in bisherigen WebIDL-Modellen, eigene
Seeds und die zugehörigen Dokumente. Die Dateiliste wurde mit Claudes PR #127
und vor dem Upload mit allen offenen PRs abgeglichen. Original-Arbeitskopie und
Index bleiben unverändert. Für jede Datei wird Bytegleichheit zwischen geprüftem
Export und Git-Blob kontrolliert; Remote-Branch und PR-Head müssen dem Commit
entsprechen. Rohe Logs und mutierte Korpora werden nicht hinzugefügt.

Branch: `codex/m2-webidl-collections`. Laufende GitHub-Prüfungen werden als
laufend gemeldet, nicht als bestanden. Der PR wird vor Abschluss der Remote-CI
nicht als gemergt bezeichnet. Das Protokoll dokumentiert fachliche Schritte,
konkrete Befehle und aggregierte Ergebnisse, keine privaten Fuzz-Rohdaten.

## 9. Übergabe und verbleibende M2-Punkte

Claude kann besonders die Richtung der Vorfahrenregeln, benannte Getter als
reguläre Operationen, die Mutatorausnahmen und den auf Grundtypen beschränkten
Iteratorvergleich prüfen. Die späteren Map-/Set-Mutator-Bindings müssen weiterhin
ihre normativen Ein-/Ausgaben erfüllen; die Namensfreigabe allein belegt das nicht.

Die nachfolgende [`toJSON`-/JSON-Typprüfung](m2-webidl-json-types.md) ist
inzwischen implementiert, ebenso die [Array-Verwendungsregeln](m2-webidl-arrays.md).
Allgemeine Extended Attributes einschließlich
Assoziation, effektive Overload-Sets und vollständige Binding-Konvertierungen
bleiben ebenfalls offen. Erst danach folgen der Binding-Generator und die
Verknüpfung mit DOM, Ereignisschleife, Skripten im Dokument und WPT/testharness.js.
Claudes JavaScript-Arbeit wird nicht übernommen; M2 ist damit nicht abgeschlossen.
