# M2: WebIDL-Konstanten, Defaultwerte und Dictionary-Argumente

Die Typstufe prüft nach den [Grundregeln](m2-webidl-types.md),
[Union-Unterscheidbarkeit](m2-webidl-unions.md) und
[Dictionary-Zyklen](m2-webidl-dictionary-types.md) nun auch Literalwerte und
zusätzliche Argumentregeln. `types::build` und `build_within` liefern weiterhin
nur vollständig innerhalb des implementierten Umfangs geprüfte Typgraphen.
Die Prüfung besteht aus eigenem Rust-Code ohne neue Crates und verwendet
keine JavaScript-Laufzeit oder fremde Browserimplementierung.

## Standard und Verhalten

Grundlage ist der gelesene WHATWG-WebIDL-Stand
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs),
insbesondere Constants, Operations/optional arguments, Dictionaries,
Primitive types, String types, Nullable types und Union types.

| Beispiel | Ergebnis |
| --- | --- |
| `const unsigned long long n = 18446744073709551615;` | gültig |
| `const unsigned long long n = 18446744073709551616;` | `ConstantValue` |
| `const boolean n = 1;` | `ConstantValue`, keine JS-Umwandlung nach Boolean |
| `const float n = 3.4028236e38;` | `ConstantValue`, gerundet nicht endlich |
| `const unrestricted double n = NaN;` | gültig |
| `enum E { "on" };` mit Default `E x = "off"` | `DefaultValue` |
| Default `ByteString x = "😀"` | `DefaultValue`, Skalar über U+00FF |
| Default `sequence<long> x = []` | gültig |
| Default `FrozenArray<long> x = []` | `DefaultValue`, die Regel nennt Sequence |

Konstantentypen bleiben auf primitive Typen beschränkt. Boolesche Literale
passen nur zu Boolean. Integer passen zu den Integer-Typen, BigInt oder
passenden numerischen Typen, Decimal-Literale zu den Gleitkommatypen.
Integer-Schreibweisen in Basis 8, 10 und 16 werden nach der lexikalischen
Grammatik behandelt. Die acht festen Integer-Typen prüfen ihre Grenzen exakt,
insbesondere ohne Umweg über `f64`. `-0` ist ein Integer-Nullwert und daher
auch für vorzeichenlose Typen zulässig. BigInt akzeptiert beliebig lange
syntaktisch gültige Integer innerhalb der Hostquoten, ohne einen BigInt-Wert
anzulegen; Decimal- und Boolean-Literale sind keine BigInt-Konstanten.

Integer-Literale für Gleitkommatypen werden vor einer etwaigen Rundung gegen
die exakten endlichen IEEE-Maxima geprüft. Für Dezimalschreibweise dienen
exakte Maximalwert-Zeichenketten, für Hex/Oktal die Bitgruppen. Es wird kein
beliebig großer Integer aufgebaut. Decimal-Literale werden direkt mit der
Rust-Standardbibliothek in die Zielpräzision `f32` oder `f64` gelesen; ein
Umweg von `f64` nach `f32` entfällt. Unterlauf auf Null ist erlaubt. Überlauf
auf Infinity ist nur für unrestricted-Typen erlaubt. Die speziellen Tokens
`Infinity`, `-Infinity` und `NaN` passen ebenfalls nur zu unrestricted-Typen.
Die Prüfung speichert keine normalisierten Laufzeitwerte oder NaN-Bitmuster.

## Defaults und Signaturen

Defaults werden an Dictionary-Feldern einschließlich Partials und in allen
bereits erfassten Signaturarten geprüft: Operationen, Konstruktoren,
Callback-Funktionen, Callback-Interfaces, Namespaces und Async-Iterable-Argumenten.
Typedefs, explizite Annotationen, Nullable und verschachtelte Unions verbergen
keine Typbedingung. Containerparameter werden dabei nicht zu Uniongliedern.

- `null` benötigt Nullable oder `any`. Ein nullable Interface- oder Callback-
  Typ kann deshalb `null` als Default haben, ein nicht-nullable Typ nicht.
- Stringwerte behalten den exakten Inhalt zwischen den Anführungszeichen.
  WebIDL-Strings haben keine Backslash-Escapes. ByteString prüft die Skalare;
  Enum-Defaults müssen exakt einem deklarierten Wert entsprechen, auch über
  Typedefs und Quellen hinweg. Es gibt keine Normalisierung oder JS-Umwandlung.
- `[]` benötigt Sequence, nullable Sequence oder eine passende Union. Es
  ist kein allgemeiner Default für FrozenArray, AsyncSequence oder `any`.
- `{}` benötigt ein gewöhnliches Dictionary oder eine passende Union.
  Record, Callback-Interface, Object und `any` erfüllen diese ausdrückliche
  Typbedingung nicht. Die Prüfung führt keine spätere Dictionary-Instanziierung
  aus und erfindet deshalb keine zusätzliche Verweigerung wegen Required-Feldern.
- Skalare Defaults von `any` einschließlich `null` und `undefined` werden
  akzeptiert. Der konkrete Laufzeit- und Konvertierungswert entsteht erst
  in den späteren Bindings.

`undefined` darf weder Argument- noch Dictionary-Feldtyp sein, auch nicht
hinter Typedefs, Nullable oder in abgeflachten Unions. Rückgabewerte und
Containerparameter werden dadurch nicht pauschal verboten. `any` ist nicht
der Typ `undefined`; `optional any x = undefined` bleibt möglich.

Wenn ein Argument ein Dictionary oder eine Union mit Dictionary enthält,
und dieses Dictionary samt Vorfahren keine Required-Felder besitzt, dann
muss es explizit optional sein und einen Default besitzen, sofern nur noch
optionale Argumente folgen oder es das letzte Argument ist. Für eine Union
kann der Default ein anderes passendes Unionglied auswählen. Required-Felder
in Partials und Vorfahren zählen mit; Felder eines Geschwister-Dictionarys nicht.
Eine rückwärts gelesene Signatur bestimmt den optionalen Suffix ohne wiederholte
Nachfolgersuchen. Variadische Dictionary-Argumente können die vorgeschriebene
Default-Bedingung nicht erfüllen, weil ihre Grammatik keinen Default erlaubt.

## API, Diagnosen und begrenzte Arbeit

Vier zusätzliche `types::ErrorKind`-Varianten sind verfügbar:

| Fehler | Fundstelle |
| --- | --- |
| `ConstantValue` | tatsächliches Konstantenliteral |
| `DefaultValue` | tatsächliche DefaultValue-Produktion |
| `UndefinedType` | betroffene Argument-/Feld-Typverwendung |
| `DictionaryArgumentDefault` | Name des betroffenen Arguments |

`webidl-check` meldet jeweils Quelldatei, Zeile und Spalte. Ein Alias aus einer
anderen Quelle verschiebt die Literaldiagnose nicht in die Typedef-Deklaration.
Die `Types`-API und ihre Limits erhalten keine weiteren Felder. Die temporären
Wertzusammenfassungen und Metadaten werden nach der Prüfung freigegeben.

Pro Typknoten wird einmal eine kleine Zusammenfassung der zulässigen
Literalarten berechnet. Geteilte Alias-Untergraphen bleiben geteilt; eine
Zusammenfassung kopiert nur feste Slots, keine abgeflachten Typbäume.
Dictionary-Required-Eigenschaften werden über den Vererbungsgraphen einmal
nachgeführt. Enums werden einmal über geliehene Strings indiziert; Defaults
kopieren nicht alle Enumwerte. Aufbau, Prüfung und Freigabe benutzen flache
Vektoren und iterative Arbeitsstapel ohne Rust-Rekursion nach IDL-Tiefe.

Alle Stufen verwenden den Rest desselben `Budget` einschließlich Abbruchsignal.
Graphaktionen, Definitions-/Member-/Argumentbesuche, Enum-Inhalte und Literal-
Tokenbytes verbrauchen Arbeit. Lange Zahlen/Strings bezahlen ihre Bytes vor
der Auswertung. Die Typstufe bleibt standardmäßig bei zehn Millionen Schritten,
das Werkzeug bei insgesamt 45 Millionen. Es gibt kein neues Budget pro Default.
Die bestehenden Grenzen für Typ-/Quellknoten, Kanten und Index-/Memberdatensätze
begrenzen auch die zusätzlichen Vektoren und Metadaten. Dies sind logische
Quoten, keine vollständige RAM-/Zeitmessung. Allokationen, Standardbibliotheks-
Zahlkonvertierungen und einzelne Map-Zugriffe haben keine inneren Abbruchpunkte.

## Regressionen und Fuzzing

Gezielte Tests prüfen alle acht Integer-Breiten mit beiden Grenzen und Nachbarn
in drei Basen, alle Literalarten, IEEE-Maxima samt Nachbarn und beiden Vorzeichen,
Decimal-Über-/Unterlauf, Enum-/ByteString-Werte, exakten Backslash-Inhalt,
Nullable/Union/Alias-Hüllen, alle Signaturarten, Partials und Dateipositionen.
Exakte Budgetgrenzen und Abbruch bleiben Hostfehler. Ein Test akzeptiert einen
50.000-stelligen BigInt, lehnt denselben Wert als `unsigned long long` ab und
bricht unter einer zu kleinen Arbeitsquote ab, ohne BigInt-Werte zu materialisieren.
2.000 Dictionarys in einer langen Vererbungskette und 2.000 Argumentverwendungen
werden auf einem Rust-Faden mit 1 MiB Stapel unter 500.000 Typ-Schritten geprüft.

Das Fuzzing-Ziel `webidl` erhält drei zusätzliche unabhängige Modelle:
Integer-Grenzen mit breiterer `i128`-Arithmetik, eine explizite Tabelle mit 224
Typ-/Default-Kombinationen sowie Dictionary-Argumente mit vorab bekannten
Required-Eigenschaften und optionalen Suffixen. Sie teilen jeweils 100.000
Schritte über Parser, Index, Member und Typen. Es gelten je Modell zwei Quellen,
8 KiB Quelltext, 8.192 Syntaxknoten, 1.024 Datensätze je Vorstufe, 4.096
Memberbesuche, 1.024 Typknoten, 2.048 Typkanten, 16 Dictionary-Knoten, 512
Dictionary-Kanten, 8.192 Unionbesuche und 16 Interfaces.

Zehn eigene synthetische Seeds sind separat reproduzierbar:

```sh
python3 fuzz/seed_webidl_values.py
cargo test --locked -p mb-webidl -p webidl-check
cargo test --locked --offline --manifest-path fuzz/Cargo.toml --lib idl::
python3 fuzz/run.py webidl --seconds 30 --seed 1
```

Der Generator schreibt nur `fuzz/seeds/webidl/value-*`; alle bisherigen Seeds
bleiben bytegleich. Reproduktion aus einem leeren Verzeichnis wird ebenfalls
geprüft. Rohe Protokolle, mutierte Korpora und Funde bleiben gemäß
[SECURITY.md](../SECURITY.md) privat.

## Anschluss für Claude und verbleibendes M2

Der untersuchte main-Stand ist `bd66b5f5d7ee7891fd25671d2ef61324aa116a92`
nach PR #125: Schritt 8 und 9c-1 sind bereits umgesetzt; 9c-2 betrifft
`import()` und Top-Level-Await. Dieses Paket arbeitet unabhängig davon an der
WebIDL-Vorbereitung für DOM-Bindings. Die gemeinsame `architektur.md`,
JavaScript-/Unicode-Dateien, allgemeine Fuzzing-Dateien und Generatoren werden
zur Vermeidung von Überschneidungen nicht bearbeitet.

1. Literaltyp-Kompatibilität gegen den gepinnten Standard prüfen, besonders
   exakte Integer-Grenzen, die Trennung von Integer- und Decimal-Literalen,
   unrestricted-Typen sowie die Behandlung von `any` und leeren Containern.
2. Die Required-Eigenschaften aus Partials/Vorfahren und die Suffixregel prüfen.
   Temporäre Zusammenfassungen müssen weiter unabhängig von der Zahl ihrer
   Verwendungsorte bleiben; keine wiederholten Vorfahrenlisten kopieren.
3. Die inzwischen ergänzten [Spezialoperationen/Stringifier](m2-webidl-special-operations.md)
   und [Collection-Deklarationen](m2-webidl-collections.md) prüfen. Allgemeine
   Extended Attributes, übrige Array-/`toJSON`-Regeln und effektive Overload-Sets
   ergänzen. Der Binding-Generator benötigt anschließend
   Argumentauswahl, Konvertierung, Ausnahmen, Prototypen und GC-/Host-Verknüpfung.
4. DOM-Bindings, Ereignisschleife, Skripte im Dokument und den WPT-Läufer für
   `testharness.js` verbinden; tägliche DOM-Messung einrichten. Die JavaScript-
   Schritte 9c-2 und 10 bis 12 verbleiben gemäß Architekturplan auf Claudes Spur.

Damit ist ein weiteres WebIDL-Arbeitspaket erledigt, M2 jedoch noch nicht
abgeschlossen. Die genaue Zielquote für „Test262 ohne Intl zu einem hohen Anteil“
ist im Architekturplan weiterhin offen.
