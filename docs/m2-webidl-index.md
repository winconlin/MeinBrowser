# M2: WebIDL-Namen und Verweise über mehrere Quellen

`mb_webidl::index` ergänzt den [Syntaxparser](m2-webidl-parser.md) um einen
quellenübergreifenden Namensindex. Er führt Grunddefinitionen und Partials
zusammen und löst benannte Typen, Vererbung und Includes auf. Die vollständigen
Syntaxbäume bleiben erhalten. Darauf baut inzwischen die strukturierte
[Memberbeschreibung mit ersten Prüfungen](m2-webidl-members.md) auf. Eine
normalisierte Typbeschreibung, vollständige semantische Validierung und
Binding-Generierung stehen noch aus.

## Standard und Umfang

Geprüfte Grundlage ist derselbe WebIDL-Quellstand wie bei Lexer und Parser:
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs),
insbesondere die Abschnitte [Names](https://webidl.spec.whatwg.org/#idl-names),
[Interfaces](https://webidl.spec.whatwg.org/#idl-interfaces),
[Interface mixins](https://webidl.spec.whatwg.org/#idl-interface-mixins),
[Dictionaries](https://webidl.spec.whatwg.org/#idl-dictionaries) und
[Typedefs](https://webidl.spec.whatwg.org/#idl-typedefs).
Der Index ist eigener Rust-Code ohne zusätzliche externe Abhängigkeiten.

Folgende Schritte sind implementiert:

- Interfaces, Callback-Interfaces, Namespaces, Dictionaries, Enums,
  Callback-Funktionen, Typedefs und Mixins erfassen; der führende
  Escape-Unterstrich wird für Namen und Verweise entfernt.
- Namenskollisionen unter den benannten Definitionen melden. Mixins sind
  keine Typen und werden getrennt indiziert; doppelte Mixin-Grunddefinitionen
  werden ebenfalls abgelehnt. Reservierte Namen `constructor` und `toString`
  werden bei benannten Definitionen auch nach dem Entschärfen des Escape
  abgelehnt. Weitere Member-Regeln prüft anschließend `mb_webidl::members`.
- Partials mit der passenden Grunddefinition verbinden, auch wenn diese
  später oder in einer anderen Quelle steht. Fehlende Grunddefinitionen und
  abweichende Definitionsarten ergeben Fehler.
- Identifier in allen normalen Grammatik-Typpositionen auflösen, einschließlich
  verschachtelter Typen, Callback-Signaturen, Konstanten, Konstruktoren und
  Collection-Deklarationen. Namespace- und Mixin-Namen sind keine Typen.
- Interface- und Dictionary-Vererbung auf passende Eltern und Zyklen prüfen.
  Callback-Interfaces sind keine Eltern gewöhnlicher Interfaces.
- Includes mit gewöhnlichen Interfaces und Mixins verbinden. Die Reihenfolge
  der Includes bleibt erhalten; Member werden dabei nicht kopiert.
- Unmittelbare Typedef-zu-Typedef-Aliasse nach der speziellen Typedef-Regel
  ablehnen und Zyklen zwischen zusammengesetzten Typedefs erkennen.

Unbekannte Namen werden nicht als implizite externe Typen akzeptiert. Alle
benötigten Quellen müssen gemeinsam übergeben werden. Es gibt keine stille
Fehlerbereinigung oder Rückgabe eines teilweise aufgelösten Index.

Extended Attributes bleiben unverändert im Syntaxbaum. Ihre Bezeichner werden
hier bewusst nicht als normale Typreferenzen behandelt. Attribute wie
`Exposed`, `PutForwards` oder `LegacyFactoryFunction` benötigen später eigene
Auswertung und Validierung; deren Verweise sind durch diesen Index noch
nicht geprüft.

## API und Prüfwerkzeug

```rust
use mb_webidl::index;

let sources = [
    "partial interface Example { attribute Options options; };",
    "interface Example {}; dictionary Options {};",
];
let index = index::build(&sources).expect("names and links");
let id = index.find("Example").expect("normalized name");
let definition = index.definition(id).expect("local definition id");
assert_eq!(definition.fragments.len(), 2);
assert_eq!(index.references().len(), 1);
```

`DefinitionId` und `FragmentId` gehören ausschließlich zu ihrem Index.
`find` und `find_mixin` erwarten bereits normalisierte Namen. Definitionen
stehen in Reihenfolge ihrer Grunddefinitionen. Ihre Fragmente stehen in
Quellreihenfolge, also bei Bedarf mit einem Partial vor der Grunddefinition.
`parent`, `mixins`, `references()` und `includes()` enthalten aufgelöste IDs.

Jedes Fragment bewahrt seine konkrete Grammatikproduktion und die davor
stehende `ExtendedAttributeList`. `document(source)` liefert den zugehörigen
Syntaxbaum samt geliehener Quelle. Eine `Location` enthält Quellindex,
UTF-8-Byte-Range und, wenn bereits ein Syntaxbaum existiert, die lokale
Knoten-ID. Doppelte Grunddefinitionen melden beide Fundstellen.

Das eigene Entwicklungswerkzeug liest mehrere lokale UTF-8-Dateien mit
begrenzter Eingabegröße und gibt Fehler mit Datei, Zeile und Spalte aus:

```sh
cargo run --locked -p webidl-check -- --help
cargo run --locked -p webidl-check -- api.idl dependencies.idl
```

Es meldet die Anzahl aufgelöster Definitionen, Fragmente, Typverweise und
Includes. Anschließend läuft die [Member-Prüfung](m2-webidl-members.md) mit
weiteren Zählern und derselben Arbeitsquote. Ein erfolgreicher Lauf bestätigt
die jeweils dokumentierten Prüfungen; er erzeugt keine Bindings und bestätigt
keine vollständige WebIDL-Gültigkeit.

## Gemeinsame Grenzen und iterative Graphen

`build` verwendet 25 Millionen Arbeitsschritte und folgende Gesamtgrenzen:

| Grenze | Standardwert |
| --- | ---: |
| Quellen | 4.096 |
| Quellbytes insgesamt | 16 MiB |
| Syntaxknoten insgesamt | 1.000.000 |
| Definitionen, Fragmente, Typverweise und Includes zusammen | 200.000 |

Zusätzlich gelten pro Quelle die [Parsergrenzen](m2-webidl-parser.md):
1 MiB Quelltext sowie 200.000 Knoten und ausstehende Aufgaben.
`build_within(sources, limits, &budget)` erlaubt engere Hostgrenzen und teilt
auch mit vorausgehender Arbeit dasselbe Budget und Abbruchsignal.

Die Gesamtquellgröße wird vor dem Parsen mit geprüfter Addition begrenzt.
Die noch verfügbaren Gesamtknoten begrenzen jeden folgenden Parseraufruf.
Neue Indexeinträge werden vor dem Anlegen gezählt. Parser, Sammeln,
Namensauflösung und Graphprüfung verbrauchen dieselbe Quote; Namenssuchen
berechnen zusätzlich die Länge des jeweiligen Namens. Knoten und Kanten der
Graphprüfung werden iterativ besucht, ohne rekursive Typedef-Expansion oder
rekursives Freigeben. Ein Mixin mit vielen Host-Interfaces vervielfacht die
Memberbäume nicht.

Die Grenzen sind logische Quoten, keine vollständige Messung des Prozess-RAMs
oder der Laufzeit. Syntaxbäume aller Quellen bleiben bis zum Freigeben des
Index erhalten. Allokationen, Namensvergleiche und einzelne Lexer-Aufrufe
besitzen keine zusätzlichen Abbruchpunkte innerhalb ihrer Ausführung.

## Tests und Fuzzing

Gezielte Regressionen prüfen alle Definitionsarten, Vorwärtsreferenzen über
mehrere Quellen, Partials, Escape-Namen, Namenskollisionen mit beiden Orten,
alle Grammatik-Typpositionen, unpassende Eltern, Includes-Reihenfolge,
Vererbungs- und Typedef-Zyklen, gemeinsame Quoten und Grenzwerte. Ein Test
baut und verwirft 2.000 verschachtelte Sequenztypen sowie eine Vererbungskette
mit 2.000 Interfaces auf einem Faden mit 1 MiB Rust-Stapel.

Aus dem gepinnten webref-Stand werden alle 350 Dateien gelesen. Die 348
syntaktisch zulässigen Dateien liefern beim Sammeln 3.424 Definitionsfragmente,
8.480 normale benannte Typverweise und 270 Includes. Die beiden bekannten
Konstruktoren in Partial-Interfaces bleiben explizite negative Grammatiktests.
Das Sammeln des Corpus bestätigt keine vollständige Verknüpfung oder
semantische Gültigkeit aller webref-Spezifikationen. Ein unveränderter gemeinsamer
CLI-Lauf über diese 348 Dateien stoppt derzeit am unbekannten Typ `SVGRect`
in `SVG.idl`. `geometry.idl` nennt diesen Namen als `LegacyWindowAlias`,
nicht als benannte Typdefinition. Solche Laufzeit-Aliasse werden nicht
stillschweigend zu zusätzlichen Typdefinitionen gemacht. Weitere Corpus-
Abweichungen müssen bei der vollständigen semantischen Prüfung untersucht
werden; es wird keine vollständig bestandene Verknüpfung behauptet.

Das bestehende Fuzzing-Ziel `webidl` prüft zusätzlich beliebige Quellen mit
dem Index unter 75.000 Schritten, höchstens 12 KiB Gesamtquelltext,
8.192 Gesamtknoten und 1.024 Indexeinträgen. Gültige Ergebnisse müssen lokale
IDs, passende Namenszuordnung, Quellbereiche und azyklische Vererbung haben.
Fehlerpositionen und tatsächliche Budgeterschöpfung werden ebenfalls geprüft.

Ein unabhängiges Modell erzeugt zusätzlich zwei zusammengehörige Dateien mit
bis zu 16 Interfaces, vorangestellten Partials, Vorwärtsreferenzen, zwei
Mixins und variierenden Includes sowie einem azyklischen Vererbungsbaum.
Es vergleicht die aufgelösten Verweise, Eltern und Mixin-Reihenfolgen mit den
vorher berechneten Erwartungen. Eine zusätzliche Kante schließt jeweils einen
bekannten Zyklus, der abgelehnt werden muss. Beide Modellaufrufe besitzen
je 75.000 Schritte, höchstens 8 KiB Text und dieselben Knoten-/Eintragsgrenzen.
Sechs neue eigene Seeds ergänzen das Corpus. Rohprotokolle, Funde und neu
mutierte Korpora bleiben nach [SECURITY.md](../SECURITY.md) und
[fuzzing.md](fuzzing.md) privat.

## Anschluss für Claude

1. Namespaces, Escape-Regeln, Fragmentreihenfolge, Typverweispositionen und
   die besondere Typedef-Regel gegen den angegebenen Standardstand prüfen.
   Die fehlende vollständige Member-Reihenfolge über Dokumentgrenzen ist im
   Standard weiterhin [Issue 432](https://github.com/whatwg/webidl/issues/432).
2. Die inzwischen vorhandene [Memberbeschreibung](m2-webidl-members.md)
   prüfen: Member-Namen und Kollisionen über Partials/Mixins, Dictionary-Felder
   einschließlich Vererbung, Enumduplikate und erste Argument-/Overload-Regeln
   sind implementiert. Eine normalisierte Typbeschreibung, vollständige
   Overload-Auflösung, erlaubte Typkombinationen, Nullable-/Union-Regeln,
   konstante Werte und Defaults sowie Extended Attributes bleiben offen.
3. Erst nach diesen Prüfungen Bindings mit Argumentprüfung, Konversionen,
   Ausnahmen und Prototypen erzeugen. Mixin-Member benötigen pro Host-Interface
   eigene JavaScript-Funktionsobjekte; gemeinsam indizierter Quellcode bedeutet
   keine gemeinsame Funktionsidentität zur Laufzeit.
4. Danach mit der aktuellen `mb-js`-API und DOM-WPT verbinden. Dieser Schritt
   ändert weder die JavaScript-Engine noch DOM-Bindings oder Ereignisschleife;
   M2 bleibt offen.
