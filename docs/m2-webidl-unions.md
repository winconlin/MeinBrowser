# M2: Unterscheidbarkeit abgeflachter WebIDL-Unionglieder

Die [Typstufe](m2-webidl-types.md) prüft jetzt zusätzlich, ob alle abgeflachten
Glieder einer Union paarweise unterscheidbar sind. Typedefs, Annotations- und
Nullable-Hüllen sowie verschachtelte Unions werden dabei aufgelöst. Das ist
weiterhin eigener Rust-Code ohne neue Abhängigkeit oder Kopplung an `mb-js`.

## Standard und Umfang

Grundlage ist der gelesene WHATWG-WebIDL-Stand
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs),
insbesondere Union types, die Definition von distinguishable samt Tabelle und
Bedingungen (a) bis (d), die Interface-Implementierungsregeln, String types,
Buffer source types und `LegacyTreatNonObjectAsNull`.

Die zwölf Tabellenkategorien sind `undefined`, Boolean, numerisch, BigInt,
String, Object, Symbol, interface-like, Callback-Funktion, dictionary-like,
AsyncSequence und sequence-like. Enums gehören zu String; Callback-Interfaces
und Records zu dictionary-like; FrozenArray zu sequence-like. Die Typen der
Containerparameter verändern diese Kategorie nicht. Unions innerhalb eines
Parameters werden dennoch eigenständig geprüft.

Beispiele:

| Deklaration | Ergebnis dieser Union-Prüfung |
| --- | --- |
| `(long or DOMString)` | unterscheidbar |
| `(long or double)` | nicht unterscheidbar: beide numerisch |
| `(DOMString or USVString)` | nicht unterscheidbar: beide String |
| `(sequence<long> or FrozenArray<DOMString>)` | nicht unterscheidbar: beide sequence-like |
| `(ArrayBuffer or DataView or Uint8Array)` | verschiedene native Buffertypen |
| `(bigint or double)` | unterscheidbar; Overload-Sonderregeln sind eine andere Prüfung |

Gleiche Interface-IDs und Vorfahren-/Nachfahrenpaare sind nicht unterscheidbar.
Verschiedene Interfaces in getrennten Teilbäumen sind unterscheidbar, auch wenn
sie einen Vorfahren oder ein Mixin teilen: Nach dem Standard implementiert ein
Plattformobjekt genau ein Interface und dessen Vorfahren. Die 15 eingebauten
Buffertypen besitzen jeweils eine eigene native Identität. Verschiedene solche
Typen sowie ein gewöhnliches Interface und ein eingebauter Buffertyp sind
unterscheidbar. Ein escaptes `_ArrayBuffer` bleibt ein gewöhnlicher benannter Typ.

Eine Callback-Funktion ist gegenüber dictionary-like nur ohne das
Definitionsattribut `[LegacyTreatNonObjectAsNull]` unterscheidbar. Es wird der
exakte Name eines obersten Attributs gelesen, nicht eine Textteilfolge in einem
fremden Attributwert. Kommentare ändern das Ergebnis nicht. Der bekannte Name
muss in der parameterlosen NoArgs-Form stehen; andere Formen ergeben
`ErrorKind::InvalidCallbackAttribute`. Ausgewertet werden nur Callback-
Definitionen, die als Unionglied benötigt werden. Andere Attribute bleiben
Syntaxdaten; dies ist keine allgemeine Extended-Attribute-Validierung.

Bedingung (b) der Tabelle erlaubt numerisch/BigInt für Unions; die zusätzliche
Einschränkung für Overloads bleibt offen. Auch String/AsyncSequence ist nach
Bedingung (d) unterscheidbar. Die dort vorgeschriebene Sonderbehandlung von
Stringobjekten bei der späteren ECMAScript-Konvertierung gehört zu Bindings
und ist hier noch nicht implementiert.

`any` und Promise behalten ihren vorherigen `UnionType`-Fehler. ObservableArray
erscheint nicht in der Tabelle und ist deshalb gegenüber keinem Unionpartner
unterscheidbar. Ein ObservableArray außerhalb von Unions wird durch diese
Prüfung nicht pauschal verboten; weitere Verwendungsregeln bleiben offen.
Die bisherigen Nullable-/Dictionary-Vorbedingungen laufen vor dieser Prüfung.

`ErrorKind::IndistinguishableUnion` zeigt auf die betroffene Unionproduktion,
auch über Quelldateien und Vorwärtsverweise hinweg. Das ist keine Diagnose
mit beiden kleinsten kollidierenden Teilgliedern. `webidl-check` nennt Datei,
Zeile und Spalte und gibt keinen teilweise geprüften Typgraphen aus.

## Algorithmus und begrenzte Arbeit

Die Prüfung benutzt den bestehenden flachen Typ-/Typedef-Graphen. Transparente
Elternkanten bestimmen die Wurzeln der Union-Prüfungen; Containerparameter sind
eigene Wurzeln. Ein iterativer Arbeitsstapel ersetzt Rekursion. Erfolgreich
geprüfte äußere Unions decken ihre enthaltenen Unions mit ab. Ein pro Wurzel
markierter Typknoten darf nicht zweimal erreicht werden: Ein geteilter Knoten
auf zwei transparenten Pfaden würde dasselbe Unionglied erneut einführen.
Damit werden auch exponentiell expandierbare Alias-DAGs früh abgelehnt.

Eine kleine Kategorienmaske und eine Buffermaske ersetzen den Vergleich jedes
Gliedes mit jedem anderen. Für gewöhnliche Interfaces wird bei Bedarf einmal
ein Intervallindex des Vererbungswaldes aufgebaut. Die Intervalle erlauben
Vorfahren-/Nachfahrenerkennung ohne erneutes Ablaufen langer Vorfahrenketten.
Eine geordnete Menge hält die Intervalle der bereits gesehenen, disjunkten
Interface-Teilbäume. Suchen und Einfügen benötigen logarithmische Map-Zugriffe.
Für reine skalare oder Buffer-Unions wird kein Interface-Index benötigt.

Verschiedene äußere Unions können denselben gültigen Untergraphen erneut
besuchen. Eine universell lineare Laufzeit über sämtliche Wurzeln wird deshalb
nicht behauptet. Alle Auflösungs- und Abflachungsbesuche zählen gegen ein
kumulatives Limit; keine Wurzel erhält eine neue Quote.

| Neues Feld in `types::Limits` | Standard | Fehler bei Überschreitung |
| --- | ---: | --- |
| `union_visits` | 1.000.000 | `Limit::UnionVisits` |
| `union_interfaces` | 200.000 | `Limit::UnionInterfaces` |

Bei vorhandenen Unions zählen auch transparente Wurzelauflösungen ohne Union
gegen `union_visits`. `union_interfaces` begrenzt alle gewöhnlichen Interfaces
im bedarfsweise angelegten Index, nicht nur die direkten Unionglieder. Die
Grenzen werden vor weiterem Besuch beziehungsweise Einfügen geprüft.

Die zusätzliche Prüfung verwendet den Rest desselben `Budget` samt Abbruchsignal.
Vorbereitende Vektoren, Graphbesuche, Attributknoten und der Interface-Index
verbrauchen Arbeitsschritte. Die Typstufe bleibt standardmäßig bei zehn Millionen,
das Prüfwerkzeug bei 45 Millionen gemeinsamen Schritten für alle Stufen.
Die Grenzen sind logische Quoten, keine vollständige RAM-/Zeitmessung; einzelne
Allokationen und Map-Operationen haben keine inneren Abbruchpunkte. Die vorherigen
Typ-, Kanten-, Parser- und Dictionary-Grenzen gelten weiterhin.

## Regressionen und unabhängiges Fuzz-Modell

Gezielte Tests prüfen alle 144 Paarungen der zwölf Kategorien und alle 225
Paarungen der 15 Buffertypen, Alias-/Annotations-/Nullable-Hüllen, Enums,
Callback-Interfaces, Legacy-Callbacks, Vorfahren und Geschwister, Mixins,
Quellpositionen über Dateien, exakte Hostgrenzen und das gemeinsame Budget.

2.000 verschachtelte Unionglieder aus verschiedenen Interfaces passen auf einen
Rust-Faden mit 1 MiB Stapel unter 300.000 Arbeitsschritten. Ein weiterer Test
prüft 600 Geschwister unter einer 1.000-stufigen gemeinsamen Vorfahrenkette mit
150.000 Schritten und ebenfalls 1 MiB Stapel. Aufbau, Prüfung und Freigabe
verwenden keine mit der IDL-Tiefe wachsende Rust-Rekursion.

Das bestehende Fuzzing-Ziel `webidl` enthält ein zusätzliches unabhängiges Modell.
Es erzeugt zwei Quellen mit zwei bis sechs Uniongliedern und transparenten
Alias-/Annotationshüllen. Ein kleines paarweises Orakel verwendet direkt
abgeschriebene Tabellenzeilen und explizite nominale Beziehungen statt der
Masken und Interface-Intervalle der Implementierung. Es erwartet exakt Annahme
oder `IndistinguishableUnion`, einschließlich Legacy-Callback- und ObservableArray-
Fällen. Die Modelltests variieren alle Kategorien und Interface-/Callback-Varianten.

Parser, Namensindex, Member und Typstufe teilen 100.000 Schritte. Erlaubt sind
8 KiB Quelltext, 8.192 Syntaxknoten, 1.024 Datensätze je Vorstufe, 4.096
Memberbesuche, 1.024 Typknoten, 2.048 Typkanten, 16 Dictionary-Knoten, 512
Dictionary-Kanten, 8.192 Unionbesuche und 16 Interfaces. Für beliebige
Fuzz-Quellen und die bisherigen Modelle werden 8.192 Unionbesuche und 1.024
Interfaces verwendet. Das sind logische Hostgrenzen, keine Laufzeitgarantie.

Acht eigene synthetische Seeds liegen unter `fuzz/seeds/webidl/union-*`.
Ein separater Generator reproduziert nur diese Dateien; der gemeinsame
Seed-Generator wird nicht geändert:

```sh
python3 fuzz/seed_webidl_union.py
cargo test --locked -p mb-webidl -p webidl-check
cargo test --locked --offline --manifest-path fuzz/Cargo.toml --lib
python3 fuzz/run.py webidl --seconds 30 --seed 1
```

Die Seed-Reproduktion wird auch aus einem leeren Verzeichnis geprüft. Gemäß
[SECURITY.md](../SECURITY.md) bleiben Rohprotokolle, mutierte Korpora und Funde
privat. Die neuen Seeds sind eigene Konformitätsbeispiele.

## Anschluss für Claude

1. Die Kategorien und Bedingungen (a) bis (d) gegen den gepinnten Standard prüfen,
   insbesondere Interfaces versus Callback-Interfaces und native Bufferidentitäten.
2. Transparente Wurzeln, Duplikaterkennung, Interface-Intervalle und kumulative
   Grenzen prüfen; geteilte Typedef-Untergraphen weiterhin kompakt lassen.
3. Die inzwischen implementierte [Default-/Konstantenprüfung](m2-webidl-values.md)
   prüfen. Weitere Member-Typregeln, allgemeine Extended Attributes und effektive
   Overload-Sets ergänzen. Uniongültigkeit ersetzt
   insbesondere nicht die Unterscheidbarkeit effektiver Overloads.
4. Erst nach den übrigen semantischen Prüfungen Bindings an die dann aktuelle
   JavaScript-API anbinden und gegen DOM-/WebIDL-WPT prüfen. M2 bleibt offen.
