# M2: Dictionary-Typzyklen nach der WebIDL-Einschlussregel

Die [Typprüfung](m2-webidl-types.md) lehnt jetzt Dictionary-Felder ab, deren
Typ ihr eigenes Dictionary einschließt. Das betrifft auch indirekte Pfade
über Typedefs, Container, Partials und Vererbung. `types::build` und
`types::build_within` führen die Prüfung automatisch aus; dadurch gilt sie
auch für `webidl-check`.

## Standard und Beispiele

Grundlage ist der bisher verwendete WebIDL-Quellstand
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs#L4762),
Abschnitt Dictionaries, Regel „The type of a dictionary member must not include
the dictionary it appears on“.

Danach schließt ein Typ ein Dictionary D ein, wenn er D selbst oder ein von D
abgeleitetes Dictionary ist, wenn ein einschließender Typ in einem Nullable,
einer Sequence, FrozenArray, Union oder einem Record-Wert liegt oder wenn
ein Dictionary-Feld einschließlich geerbter Felder einen solchen Typ besitzt.
Typedefs und explizite Annotationshüllen werden entsprechend aufgelöst.

```webidl
dictionary Base { Child child; };
dictionary Child : Base {};
```

Dieses Beispiel ist ungültig, obwohl Child kein eigenes Feld hat: Der Feldtyp
Child schließt seinen Vorfahren Base ein. Dagegen ist folgendes zulässig:

```webidl
dictionary Base { long number; };
dictionary Child : Base { Base ancestor; };
```

Der Vorfahrentyp besitzt hier keinen Pfad zurück zu Child. Ebenso werden
`D` in `sequence<D>`, `FrozenArray<D>`, `record<DOMString,D>` und entsprechenden
verschachtelten Formen berücksichtigt. Promise-, AsyncSequence- und
ObservableArray-Parameter gehören nicht zu den in dieser Einschlussregel
aufgeführten Kanten. Etwa `sequence<Promise<D>>` erzeugt deshalb über den
Promise-Parameter keine Rückkante. Andere Typregeln, insbesondere für
ObservableArrays, bleiben zusätzlich zu beachten.

`ErrorKind::DictionaryCycle` zeigt auf den Namen eines beteiligten Feldes.
Die Fundstelle kann in einer anderen Quelldatei oder einem Partial liegen.
Sie muss weder das erste deklarierte Feld noch der kürzeste mögliche Zyklus
sein. Das Prüfwerkzeug meldet Datei, Zeile und Spalte dieser echten Fundstelle.
Die bereits vorhandenen Prüfungen für reine Vererbungs- und Typedef-Zyklen
laufen weiterhin vorher.

## Impliziter gemeinsamer Graph

Die Prüfung betrachtet Dictionary-Definitionen und die vorhandenen Typknoten
als Knoten eines gerichteten Graphen. Kanten sind:

- vom Dictionary zu seinem direkten Vorfahren und den Typen seiner eigenen
  Felder, einschließlich Partials;
- vom benannten Dictionary-Typ zu seiner Dictionary-Definition;
- vom Typedef-Verweis zum bereits aufgebauten Typedef-Typ;
- von Nullable, expliziter Annotation, Sequence, FrozenArray und Union zu den
  jeweiligen Kindtypen sowie von Record zu seinem Werttyp.

Alle Einschlusswege des angegebenen Standards sind damit erreichbar. Die
Vorfahrenkante erfasst zugleich die geerbten Felder. Ein Zyklus bedeutet,
dass mindestens ein Feld sein eigenes Dictionary wieder erreicht. Reine
Typedef- oder Vererbungszyklen wurden vorher ausgeschlossen; deshalb enthält
jeder hier gefundene Zyklus eine Feldkante. Die letzte aktive Feldkante im
betroffenen Pfad liefert die Fundstelle.

Eine iterative Tiefensuche mit Farben besucht jeden erreichbaren Knoten und
jede Einschlusskante einmal. Bereits abgeschlossene Untergraphen bleiben für
weitere Dictionary-Wurzeln markiert. Definitionen und Typedef-Ziele werden
über geordnete ID-Indizes gesucht; diese Zugriffe sind logarithmisch. Der
Graph wird nicht als kopierter Baum oder als komplette Erreichbarkeitsmatrix
materialisiert. Dictionary-Felder und Vorfahrenlisten bleiben in den bestehenden
Stufen und werden nicht für jeden Nachfahren kopiert.

Die zusätzlichen Speicherstrukturen enthalten nur Dictionary-IDs, Eltern-IDs,
Farben und einen flachen Arbeitsstapel. Sie werden nach der Prüfung freigegeben.
Die Feld- und Typknoten bleiben unverändert. Traversierung und Freigabe benötigen
keinen mit der IDL-Verschachtelung wachsenden Rust-Stapel.

## Grenzen und gemeinsame Arbeitsquote

Die neuen Felder von `types::Limits` besitzen folgende Standardwerte:

| Grenze | Standard |
| --- | ---: |
| `dictionary_nodes`: Dictionary-Definitionen im Einschlussgraphen | 200.000 |
| `dictionary_edges`: besuchte Einschlusskanten | 1.000.000 |

Die Kantenquote umfasst auch die impliziten Feld-, Vorfahren- und Typedef-Kanten;
sie ist getrennt von `edges`, der Grenze für gespeicherte Typ-Kindkanten.
Die Dictionary-Grenze wird vor dem Einfügen neuer Dictionary-Datensätze geprüft.
Jede verfolgte Kante wird vor der weiteren Verarbeitung gezählt.
`Limit::DictionaryNodes` und `Limit::DictionaryEdges` sind Hostgrenzen und
keine semantischen Typfehler.

Die Prüfung verwendet den Rest derselben Typ-Arbeitsquote. Es wird kein neues
Budget pro Dictionary oder Feld angelegt. Aufbau der Dictionary-Indizes,
Farben, Wurzelbesuche und Graphaktionen verbrauchen Arbeitsschritte und prüfen
Abbruch. Der Standardaufruf der Typstufe bleibt bei zehn Millionen Schritten;
`webidl-check` teilt weiterhin 45 Millionen Schritte über alle Stufen.

Die Quoten sind logische Grenzen und keine vollständige Messung des RAMs oder
der Laufzeit. Einzelne Allokationen und ID-Map-Zugriffe besitzen keine zusätzlichen
Abbruchpunkte innerhalb ihrer Ausführung. Fehler und erschöpfte Arbeit liefern
keinen teilweise geprüften Typgraphen zurück.

## Regressionen und unabhängiges Fuzz-Modell

Gezielte Tests prüfen direkte und gegenseitige Zyklen, Partials, spätere Quellen,
Typedefs und alle genannten Container. Weitere Fälle trennen Vorfahren- und
Nachfahrentypen, folgen geerbten Feldern und prüfen sichere Untergraphen vor
einem späteren Zyklus. Feldpositionen über Dateigrenzen, leere Dictionaries,
exakte Dictionary-/Kantenlimits und das gemeinsam verwendete Budget werden geprüft.

Zwei Tests bauen 2.000 Dictionaries auf einem Faden mit 1 MiB Rust-Stapel:
eine lange Feldkette und eine rückwärts deklarierte Vererbungskette mit gemeinsamem
Typedef. Kleine Besuchs- und Arbeitsgrenzen reichen dafür aus. Ein gültiger
Dictionary-Graph mit je zwei Feldern auf den gemeinsamen Vorgänger bleibt unter
einer kleinen Quote, obwohl vollständiges Aufklappen mehr als 34 Milliarden
Blätter erzeugen würde. Eine zusätzliche Rückkante muss abgelehnt werden.

Das bestehende Fuzzing-Ziel `webidl` besitzt ein weiteres unabhängiges Modell:
Es erzeugt bis zu acht Dictionaries in zwei Quellen, mit vorangestellten
Partials, Typedefs, verschiedenen Containern und optionaler Vererbung. Aus den
vorab gewählten Beziehungen wird eine kleine boolesche Erreichbarkeitsmatrix
berechnet. Die Matrix sagt unabhängig von der Tiefensuche voraus, ob ein Zyklus
existiert. Die Implementierung muss genau dieses Ergebnis liefern und bei
Zyklen auf eines der tatsächlich erzeugten Felder zeigen.

Das Modell teilt 100.000 Schritte über Parser, Namen, Member und Typprüfung.
Es erlaubt 8 KiB Quellen, 8.192 Syntaxknoten, 1.024 Datensätze pro bisheriger
Stufe, 2.048 gespeicherte Typkanten, 16 Dictionary-Knoten und 512 Einschlusskanten.
Memberbesuche bleiben auf 4.096 begrenzt. Die Prüfung beliebiger Quellen und das
bisherige Typmodell verwenden zusätzlich 1.024 Dictionary-Knoten und 4.096
Einschlusskanten bei ihren bestehenden gemeinsamen Arbeitsquoten.

Acht eigene synthetische Seeds liegen unter `fuzz/seeds/webidl/dictionary-*`.
Sie lassen sich mit dem separaten Generator wiederherstellen:

```sh
python3 fuzz/seed_webidl_dictionary.py
cargo test --locked -p mb-webidl -p webidl-check
cargo test --locked --offline --manifest-path fuzz/Cargo.toml --lib
python3 fuzz/run.py webidl --seconds 30 --seed 1
```

Der Generator schreibt nur diese acht Seeds; eine Reproduktion aus einem leeren
Ausgabeverzeichnis wird ebenfalls geprüft. Der allgemeine Seed-Generator und
die anderen Fuzz-Ziele werden nicht verändert. Rohprotokolle, mutierte Korpora
und Fuzzing-Funde bleiben gemäß [SECURITY.md](../SECURITY.md) privat.

## Anschluss für Claude

1. Die Einschlusskanten und ihre Richtung gegen den genannten Standardstand
   prüfen, besonders Nachfahrentypen in Vorfahrenfeldern und geerbte Felder.
2. Globale Farben, Typedef-Verweise, tatsächliche Feldpositionen und die beiden
   zusätzlichen Grenzen prüfen. Weiterhin keine geteilten Untergraphen expandieren.
3. Als nächsten Typprüfungsschritt die vollständige Unterscheidbarkeit aller
   Unionglieder ergänzen. Werte/Defaults, weitere Member-Typregeln, Extended
   Attributes und effektive Overload-Sets bleiben ebenfalls offen.
4. Erst nach den übrigen semantischen Prüfungen Bindings erzeugen und gegen
   DOM-/WebIDL-WPT prüfen. M2 ist weiterhin nicht abgeschlossen.
