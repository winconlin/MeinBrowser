# M2: WebIDL-Memberbeschreibung und erste Member-Prüfungen

`mb_webidl::members` baut auf dem [Namensindex](m2-webidl-index.md) auf.
Der Baustein erfasst Member, Argumente und Enumwerte strukturiert und prüft
weitere Regeln vor einer späteren Binding-Generierung. Er bleibt unabhängig
von der JavaScript-Engine und enthält keine fremde Browserimplementierung.

## Geprüfter Standardumfang

Grundlage ist der bereits verwendete WebIDL-Quellstand
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs),
insbesondere die Abschnitte Names, Constants, Attributes, Operations,
Overloading, Dictionaries und Enumerations. Implementiert sind:

- Member-Namen nach Entfernung des führenden Escape-Unterstrichs prüfen:
  `constructor`, `toString` und mit `_` beginnende normalisierte Namen sind
  reserviert. Konstanten dürfen außerdem nicht `length`, `name` oder
  `prototype` heißen; statische Attribute und Operationen nicht `prototype`.
  Die echte Konstruktor-Deklaration bleibt erlaubt.
- Kollisionen von Attributen und Konstanten untereinander und mit benannten
  Operationen erkennen. Dabei werden Partials und alle direkt eingebundenen
  Mixins berücksichtigt. Statische und gewöhnliche Member werden für diese
  Kollisionen gemeinsam betrachtet.
- Operationen desselben Namens innerhalb derselben Definition als mögliche
  Overloads erhalten. Statische und gewöhnliche Operationen bilden getrennte
  Gruppen. Overloads über Interface-, Partial-Interface-, Mixin- oder
  Partial-Mixin-Grenzen werden nach der ausdrücklichen Overloading-Regel
  abgelehnt. Ob Overloads nach ihren Typen unterscheidbar sind, bleibt offen.
- Bei Dictionaries doppelte Felder innerhalb der Definition, über Partials
  und entlang der gesamten Vererbung erkennen. Interface-Member dürfen
  hingegen Member eines Vorfahren überschreiben; diese verschiedenen Regeln
  werden nicht gleichgesetzt.
- Argumentnamen innerhalb einer Signatur nach dem Unescaping auf Duplikate
  prüfen, auch in Callbacks, Konstruktoren und Async-Iterable-Signaturen.
  Die für Argumente erlaubten Schlüsselwörter und reservierten Member-Namen
  bleiben dort zulässig. Ein `...`-Argument muss das letzte Argument sein und
  wird als implizit optional erfasst.
- Gewöhnliche und statische Operationen ohne Namen ablehnen. Namenlose
  Getter, Setter, Deleter und Stringifier bleiben erhalten.
- Enumwerte anhand ihres exakten Inhalts auf Duplikate prüfen. Anführungszeichen
  werden entfernt, aber weder Backslash-Escapes ausgewertet noch Groß-/
  Kleinschreibung oder Unicode-Normalformen vereinheitlicht.

Diese Prüfungen ersetzen noch keinen vollständigen WebIDL-Validator. Insbesondere
bleiben `toJSON`-Sonderregeln, weitere Getter-/Setter-/Stringifier- und
Collection-Regeln, Typ-/Union-/Nullable-Prüfung, Typedef-Auswertung,
Dictionary-Typzyklen, Pflicht-/Defaultwert-Regeln, Wertebereiche, Extended
Attributes, effektive Overload-Sets und Sprachbindungen offen. Ein Erfolg ist
keine Freigabe zur ungeprüften Codegenerierung.

## Flache Memberbeschreibung

```rust
use mb_webidl::{index, members};

let sources = [
    "partial interface Example { undefined run(optional long count = 0); };",
    "interface Example {};",
];
let index = index::build(&sources).expect("names and links");
let model = members::build(&index).expect("supported member rules");
let id = index.find("Example").expect("definition");
let member = &model.members(id).expect("own members")[0];
let args = model.arguments(member.signature.as_ref().expect("signature"))
    .expect("local arguments");
assert_eq!(args[0].identifier.name, "count");
assert!(args[0].optional);
assert!(args[0].default.is_some());
```

Die Beschreibung enthält Konstanten, Attribute, Operationen mit ihrer
Spezialart, Konstruktoren, Dictionary-Felder, synchrone und asynchrone
Iterables, Maplikes, Setlikes und Stringifier. Sie bewahrt den Herkunfts-
`FragmentId`, die Quellposition, den normalisierten optionalen Namen und
Flags für `static`, `readonly`, `inherit`, `stringifier` und `required`.
Die Typen bleiben bis zu zwei lokale Syntaxknoten; sie sind noch keine
normalisierten oder vollständig geprüften Typobjekte.

Signaturen verweisen in einen flachen Argumentvektor. Callback-Signaturen
sind über `callback(id)` zugänglich. Argumente bewahren ihren Typknoten,
Attribute, `optional`, `variadic` und einen optionalen Defaultwertknoten.
Konstantenwerte und Dictionary-Defaults bleiben ebenfalls Quellknoten.
Extended Attributes an Definitionen, Membern, Argumenten und Typen werden
nicht verworfen oder still als verstanden vorausgesetzt.

`members(id)` liefert ausschließlich die eigenen Deklarationen einschließlich
Partials, in der vom Namensindex bewahrten Fragment- und Quellreihenfolge.
Mixin-Member und geerbte Dictionary-Felder werden für die Prüfung besucht,
aber nicht in jeden Host kopiert. `len()` zählt daher Member-Deklarationen,
keine Zahl von JavaScript-Eigenschaften. Dictionary-Bindings müssen später
geerbte Felder vor lokalen Feldern und lokale Felder nach Unicode-Codepoints
sortieren; die Quellreihenfolge dieser Beschreibung ist nicht die Binding-Reihenfolge.
Für Mixin-Funktionen bleibt die Laufzeitidentität pro Host-Interface getrennt.

Quellknoten gehören zu `index.document(location.source)`. Die Beschreibung
leiht Bezeichner und Enumtexte direkt aus den Quellen. Sie gehört zu genau
dem Index, auf dem sie aufgebaut wurde; dessen IDs und Knoten dürfen nicht
mit denen eines anderen Index vermischt werden. Fehler bewahren beide
Fundstellen einer Namenskollision, auch über Dateigrenzen.

## Grenzen und Arbeitsquote

`members::build` verwendet zehn Millionen Schritte. Standardgrenzen sind:

| Grenze | Wert |
| --- | ---: |
| Definitionen, Member, Argumente und Enumwerte zusammen | 200.000 |
| Besuche bei der Prüfung wirksamer Member und Dictionary-Vererbung | 1.000.000 |

`build_within(&index, limits, &budget)` erlaubt eigene Grenzen und setzt ein
bereits verwendetes Budget fort. Neue Datensätze werden vor dem Anlegen gezählt.
Sammeln, Namensvergleiche und die Prüfung der wirksamen Member verbrauchen
Schritte. Namensvergleiche berechnen zusätzlich die Länge des Namens.
Auch leere Definitionen und Includes zählen bei der Prüfung als Besuche.
Grenzen und Abbruch ergeben eigene Hostfehler, kein semantisch ungültiges
Teilergebnis.

Dictionaries werden als Wald iterativ durchlaufen. Ein aktiver Namensindex
enthält die Felder der Vorfahren und wird beim Verlassen einer Definition um
deren eigene Felder zurückgenommen. Sibling-Dictionaries teilen dadurch keine
lokalen Namen. Vorfahrenlisten werden nicht für jeden Nachfahren kopiert oder
neu durchlaufen. Die Prüfarbeit bleibt bei diesem Durchlauf in der Anzahl der
Definitionen und Felder linear, zuzüglich der geordneten Namensvergleiche.

Bei Includes hängt die Arbeit von der Zahl der wirksamen Member pro Host ab.
Ein eigenes Besuchslimit begrenzt diese wiederholte Arbeit. Der Memberbaum eines
Mixins wird trotzdem nur einmal gespeichert. Datenstrukturen, Traversierung
und Freigabe erfordern keinen mit der IDL-Tiefe wachsenden Rust-Stapel.

Die Grenzen sind logische Quoten, keine vollständige Messung des RAMs oder der
Wandzeit. Parser und Namensindex besitzen weiterhin ihre eigenen Grenzen.
Allokationen und einzelne Namensvergleiche haben keine Abbruchpunkte mitten
in ihrer Ausführung.

## Prüfwerkzeug und Fuzzing

```sh
cargo run --locked -p webidl-check -- api.idl dependencies.idl
cargo test --locked -p mb-webidl -p webidl-check
```

`webidl-check` führt jetzt Namensindex und Member-Prüfung mit einer gemeinsamen
Quote von 35 Millionen Schritten aus. Es meldet Fehler mit Datei, Zeile und
Spalte sowie gegebenenfalls der anderen Deklaration. Bei Erfolg werden zusätzlich
Member-, Argument- und Enumwertzahlen und die noch offenen Prüfschritte ausgegeben.
Es erzeugt keine Bindings.

Regressionen prüfen alle Memberformen und Flags, genaue Quellverweise,
Kollisionen in beiden Reihenfolgen, Partials, mehrere Mixins, erlaubte und
unzulässige Overloads, Dictionary-Vererbung und Wiederherstellung des
Namensbereichs bei Geschwistern, Callback-/Konstruktor-/Iterable-Argumente,
Escape-Namen, Variadics, Enums, gemeinsame Quoten und Grenzwerte.
2.000 Dictionaries mit je einem Feld werden auf einem Faden mit 1 MiB
Rust-Stapel unter einer kleinen Arbeits-/Besuchsquote aufgebaut und freigegeben.

Das bestehende Fuzzing-Ziel `webidl` prüft Member erfolgreicher Namensindizes
mit derselben verbleibenden 75.000-Schritt-Quote, höchstens 1.024 Datensätzen
und 4.096 Besuchen. Quellbereiche, lokale Knoten, Memberzuordnung,
Signaturen, Flags und vollständige Zählung müssen konsistent sein.

Ein zusätzliches unabhängiges Modell erzeugt zwei kleine Quellen mit zwei bis
acht Dictionaries, zwei Mixins, einem Interface samt Partial und einer Enum.
Es vergleicht die Member-, Argument- und Enumwertdaten mit vorab berechneten
Erwartungen. Sieben Varianten fügen jeweils genau einen bekannten Fehler ein:
ein geerbtes doppeltes Feld, einen doppelten Argumentnamen, einen doppelten
Enumwert, ein verbotenes Overload über Fragmente, eine Mixin-Kollision,
einen reservierten Member-Namen oder ein nicht abschließendes Variadic.
Parser, Index und Member-Prüfung teilen dabei 100.000 Schritte, 8 KiB Quellen,
8.192 Syntaxknoten, 1.024 Datensätze je Stufe und 4.096 Memberbesuche.
Acht eigene reproduzierbare Seeds ergänzen das Corpus.

Der bereits dokumentierte webref-Sammeltest bestätigt weiterhin keine
vollständige semantische Verknüpfung aller Spezifikationen. Die dortigen
Syntax- und Namensabweichungen werden nicht durch zusätzliche implizite
Typdefinitionen oder ausgesparte Memberprüfungen verdeckt. Rohprotokolle,
Fuzzing-Funde und mutierte Korpora bleiben nach [SECURITY.md](../SECURITY.md)
und [fuzzing.md](fuzzing.md) privat.

## Anschluss für Claude

1. Die implementierten Regeln gegen den angegebenen Standardstand prüfen,
   besonders static/regular bei Namenskollisionen und Overloads, Partials und
   Mixins sowie die andere Vererbungsregel bei Dictionary-Feldern.
2. Typknoten in eine begrenzte normalisierte Typbeschreibung überführen und
   danach erlaubte Typkombinationen, Dictionary-Typzyklen, Default-/Konstanten-
   werte und die typabhängigen Memberregeln validieren.
3. Extended Attributes gezielt auswerten und validieren; ihre ungeprüften
   Syntaxknoten dürfen keine Erlaubnis für die Codegenerierung sein.
4. Effektive Overload-Sets, Unterscheidbarkeit und Binding-Reihenfolgen umsetzen.
   Danach den eigenen Binding-Generator an die aktuelle JavaScript-Engine
   anbinden und mit DOM-/WebIDL-WPT prüfen. M2 bleibt offen.
