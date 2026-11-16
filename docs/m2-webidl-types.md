# M2: flache WebIDL-Typbeschreibung und erste Typregeln

`mb_webidl::types` ergänzt [Namensindex](m2-webidl-index.md) und
[Memberbeschreibung](m2-webidl-members.md) um eine eigene normalisierte
Typdarstellung. Typedefs bleiben Verweise auf einen Graphen. Zusätzlich werden
erste Form- und Verwendungsvorschriften sowie inzwischen
[Dictionary-Typzyklen](m2-webidl-dictionary-types.md) und
[Union-Unterscheidbarkeit](m2-webidl-unions.md) geprüft. Es wird keine fremde
Browserimplementierung oder zusätzliche externe Crate verwendet.

## Standard und geprüfte Regeln

Grundlage ist derselbe gelesene WebIDL-Quellstand wie bei den bisherigen
Stufen:
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs),
insbesondere Constants, Attributes, Operations, Dictionaries, Typedefs,
Types, Nullable types, Promise types, Union types und Annotated types.
Die Prüfung löst Typedefs und explizite Annotationshüllen bei der Einordnung auf:

- Nullable-Typen dürfen kein `any`, Promise, ObservableArray, weiterer
  Nullable-Typ oder Union mit nullable Gliedern beziehungsweise einem
  abgeflachten Dictionary-Glied als inneren Typ besitzen.
- Unions dürfen kein `any` oder Promise als abgeflachtes Glied enthalten.
  Die Zahl nullable Glieder darf höchstens eins sein; bei einem nullable
  Glied darf kein abgeflachtes Dictionary-Glied vorkommen. Verschachtelte
  Unions und Typedef-Verweise werden dabei berücksichtigt.
- Konstanten benötigen primitive Typen, auch bei Verwendung eines Typedefs.
- Attribute dürfen keine nullable oder nicht-nullable Sequenz, Async-Sequenz,
  Dictionary oder Record als Typ haben. Bei Unions werden die im gelesenen
  Attributes-Abschnitt ausdrücklich genannten abgeflachten Sequenz-, Dictionary-
  und Record-Glieder geprüft. Containerparameter werden dabei nicht wie
  Unionglieder behandelt: etwa `FrozenArray<Dictionary>` ist kein Dictionary-Typ.
- Promise-Attribute müssen `readonly` sein. Weitere Einschränkungen ihrer
  Extended Attributes stehen noch aus.
- Operation-/Callback-/Konstruktor-/Async-Iterable-Argumente und Dictionary-
  Felder dürfen nach dem Auflösen kein Nullable mit Dictionary als innerem
  Typ sein. Nullable Dictionary-Rückgabewerte sind davon nicht ausgeschlossen.

Die abgeflachten Unionglieder werden inzwischen zusätzlich nach der
[normativen Unterscheidbarkeitstabelle](m2-webidl-unions.md) geprüft.
Das ist noch keine vollständige semantische Validierung. Insbesondere fehlen
weitere Regeln für ObservableArrays, Collection-/Getter-/Setter-/Stringifier-
und `toJSON`-Typen, Wertebereiche und Defaults, Typattribut-Auswertung sowie
effektive Overload-Sets. Auch ein erfolgreicher Aufbau erlaubt noch keine
ungeprüfte Binding-Generierung.

## API und Darstellung

```rust
use mb_webidl::{index, members, types};

let sources = [
    "typedef unsigned long Count;",
    "interface Example { readonly attribute Count count; };",
];
let index = index::build(&sources).expect("names and references");
let members = members::build(&index).expect("supported member rules");
let types = types::build(&index, &members).expect("supported type rules");
let member = &members.members(index.find("Example").unwrap()).unwrap()[0];
let id = types.at(member.location.source, member.types[0].unwrap()).unwrap();
assert_eq!(types.category(id), Some(types::Category::Primitive));
```

`Types` besitzt einen flachen Knotenvektor und einen separaten Vektor von
Kind-IDs. `TypeId` ist eine lokale 32-Bit-ID; auch `Option<TypeId>` braucht
vier Bytes. `get`, `iter` und `children` liefern Form, Quellort und Kindtypen.
IDs, Definitionen und Quellknoten gehören ausschließlich zu den zusammengehörigen
Index-, Member- und Typbeschreibungen.

Die Formen unterscheiden primitive Typen einschließlich Breite, Vorzeichen
und `unrestricted`, die drei Stringtypen, alle 15 Buffertypen, `any`,
`object`, `symbol`, `undefined`, benannte Typen, Nullable, Sequence,
AsyncSequence, FrozenArray, ObservableArray, Promise, Record, Union und
explizit annotierte Typen. Multiwort-Typen werden aus ihren Grammatik-Tokens
normalisiert; Kommentare und Leerraum verändern ihre Identität nicht.
Escapte Bezeichner wie `_DOMString` bleiben benannte Definitionen und werden
nicht mit dem eingebauten `DOMString`-Token verwechselt.

`at(source, node)` verbindet die Typknoten der Memberbeschreibung mit ihrer
normalisierten Form. Grammatikhüllen ohne eigene Bedeutung teilen dieselbe ID.
Nullable-/Annotationshüllen besitzen eigene Typknoten. Mehrere abgeleitete
Knoten dürfen auf dieselbe Quellproduktion zeigen; `at` liefert deren äußere
Form. Record-Knoten enthalten den kanonischen String-Schlüsseltyp und ein
Kind für ihren Werttyp. Union-Kinder bleiben die direkten Glieder; verschachtelte
Unions werden nicht als große kopierte Listen materialisiert.

Ein benannter Typ bleibt `Kind::Named(DefinitionId)`. Bei Typedefs führt
`typedef(definition)` zum eigentlichen Typknoten. Diese Kante kann auch in
eine spätere Quelldatei zeigen. `category(id)` liefert die nach Typedefs und
Annotationshüllen berechnete Kategorie; es verändert weder `kind` noch
`children`. Zum Lesen der Parameter eines Alias muss dessen Typedef-Kante
weiterverfolgt werden. Der Graph wird nicht in jeden Verwendungsort kopiert.

Explizite Typattribute bleiben als `Kind::Annotated { attributes }` mit einem
lokalen Syntaxknoten erhalten. Ihre Bedeutung, Anwendbarkeit und Weitergabe
nach den Regeln der zugeordneten Extended Attributes werden noch nicht
interpretiert. Member-, Argument- und Definitionsattribute bleiben zusätzlich
in den vorherigen Stufen zugänglich. Es gibt keine stillschweigende Freigabe
unbekannter Attribute.

## Aufbau, gemeinsame Quoten und Speicher

`build` verwendet zehn Millionen Schritte. `build_within` verwendet das
übergebene Restbudget einschließlich Abbruchsignal. Die Standardgrenzen sind:

| Grenze | Wert |
| --- | ---: |
| Syntaxknoten aller Quelldokumente für die Zuordnung | 1.000.000 |
| Normalisierte Typknoten | 200.000 |
| Gespeicherte Kindkanten | 400.000 |
| Dictionary-Definitionen im Einschlussgraphen | 200.000 |
| Besuchte Dictionary-Einschlusskanten | 1.000.000 |
| Besuche bei der Union-Auflösung und -Abflachung | 1.000.000 |
| Interfaces im bedarfsweise aufgebauten Union-Vererbungsindex | 200.000 |

Die Gesamtzahl der Syntaxknoten wird vor dem Anlegen der Zuordnungstabellen
mit geprüfter Addition begrenzt. Typ- und Kantenlimits werden vor dem Einfügen
neuer Datensätze geprüft. Auch temporäre Unionlisten werden begrenzt. Parser,
Namensindex und Memberstufe behalten ihre jeweiligen zusätzlichen Grenzen.

Da Arena-Kinder des Syntaxparsers stets größere IDs haben, liest der Typaufbau
die Arena einmal rückwärts. Kindformen sind dadurch vor der jeweiligen
Elternform bekannt; ein zusätzlicher rekursiver Baumdurchlauf entfällt.
Grammatiklisten großer Unions werden pro Union einmal gelesen.

Die Kategorie- und Formprüfung besucht den Typ-/Typedef-Graphen iterativ und
berechnet kleine Zusammenfassungen pro Knoten. Nullable-Anzahlen werden bei
zwei gesättigt; ein größerer genauer Wert wäre für die Fehlerprüfung nicht
nötig. Geteilte Typedef-Untergraphen werden einmal ausgewertet. Eine Kette von
Typedef-Unions, die beim Kopieren exponentiell groß würde, bleibt so klein.
Ein defensiver Zyklentest bleibt zusätzlich zur Zyklenerkennung des Namensindex
vorhanden. Traversierung und Freigabe benötigen keinen mit der Tiefe wachsenden
Rust-Stapel.

Quellzuordnungen, Knoten, Kanten, Graphbesuche und Kontextprüfungen verbrauchen
dieselbe Quote. Die Grenzen sind logische Quoten, keine vollständige Messung
von Prozess-RAM oder Wandzeit. Allokationen und einzelne kleine Hilfsoperationen
besitzen keine Abbruchpunkte innerhalb ihrer Ausführung. Bei Fehlern oder
Hostgrenzen wird kein teilweise geprüfter Typgraph zurückgegeben.

## Prüfwerkzeug und Tests

```sh
cargo run --locked -p webidl-check -- api.idl dependencies.idl
cargo test --locked -p mb-webidl -p webidl-check
```

`webidl-check` führt nun Namensindex, Member- und Typstufe unter einer gemeinsamen
Quote von 45 Millionen Schritten aus. Es meldet zusätzlich Typknoten und Kanten.
Typfehler nennen Datei, Zeile und Spalte; ein unzulässiger Alias als Attributtyp
wird an seiner Verwendung gemeldet. Das Werkzeug erzeugt keine Bindings.

Regressionen prüfen kanonische Typen, alle Generics, Records, verschachtelte
Unions, Annotations- und Nullable-Hüllen, Vorwärtsverweise über mehrere Quellen,
Escape-Namen, sämtliche Member-/Argument-Typpositionen, die oben genannten
Regeln nach Typedef-Auflösung, exakte Grenzen und gemeinsam verbrauchte Budgets.
2.000 verschachtelte Sequenztypen werden auf einem Faden mit 1 MiB Rust-Stapel
aufgebaut und freigegeben. Ein geteilter Typedef-Graph wird unter einer kleinen
Quote abgelehnt, obwohl sein vollständiges Aufklappen Milliarden Glieder erzeugen
würde: Das Beispiel enthält gleiche Unionglieder und ist damit semantisch
ungültig. Geteilte gültige Untergraphen bleiben weiterhin kompakt gespeichert.

Im bestehenden Fuzzing-Ziel `webidl` erhält die Typstufe den Rest derselben
75.000-Schritt-Quote nach Index und Memberprüfung. Es gelten 8.192 Syntaxknoten,
1.024 Typknoten und 2.048 Kindkanten. Erfolgreiche Graphen müssen gültige lokale
IDs, Quellorte, Formen, Kindanzahlen, Kategorien und Zuordnungen für alle
Member- und Signaturtypen besitzen. Hostfehler müssen zum tatsächlichen Budget
passen.

Ein eigenes zusätzliches Modell erzeugt zwei Quellen mit einem primitiven
Typedef, einem Dictionary, einem Interface und einem mit null bis sieben
Sequenzen verschachtelten FrozenArray-Typ. Die erwartete normalisierte Form
und die Auflösung zum ursprünglichen Integer werden unabhängig geprüft.
Sieben Varianten erzeugen jeweils einen bekannten Fehler: Sequenz-Alias als
Attribut, doppelt nullable über Alias, zwei nullable Unionglieder, nichtprimitive
Konstante, nullable Dictionary-Argument, schreibbares Promise-Attribut oder
`any` über Alias als Unionglied. Die drei Vorstufen und Typstufe teilen
100.000 Schritte, 8 KiB Quellen, 8.192 Syntaxknoten, je 1.024 Datensätze und
2.048 Typkanten; Memberbesuche bleiben auf 4.096 begrenzt.
Acht zusätzliche eigene Seeds sind reproduzierbar.

Die bekannten webref-Abweichungen aus [m2-webidl-index.md](m2-webidl-index.md)
bleiben unverändert; ein vollständig semantisch bestandener Corpus wird nicht
behauptet. Rohprotokolle, Funde und mutierte Korpora bleiben gemäß
[SECURITY.md](../SECURITY.md) und [fuzzing.md](fuzzing.md) privat.

## Anschluss für Claude

1. Typformen, Zuordnung der Grammatikproduktionen, Typedef-Kanten und die
   genauen Regeln nach dem angegebenen Standardstand prüfen.
2. Die inzwischen implementierte [Dictionary-Zyklenprüfung](m2-webidl-dictionary-types.md)
   auf dem impliziten Einschlussgraphen sowie die
   [Union-Unterscheidbarkeit](m2-webidl-unions.md) mit Interface-Intervallen und
   gemeinsamem Besuchslimit prüfen.
3. Die verbleibenden Member-Typregeln, Werte/Defaults und Extended Attributes
   einschließlich ihrer Anwendbarkeit und Assoziation implementieren.
4. Effektive Overload-Sets und Binding-Reihenfolgen ergänzen. Erst danach
   Bindings mit der dann aktuellen JavaScript-API erzeugen und gegen DOM-/
   WebIDL-WPT prüfen. M2 bleibt offen; die RegExp-Engine wird hier nicht geändert.
