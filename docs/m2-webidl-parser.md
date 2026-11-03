# M2: iterativer WebIDL-Syntaxparser

Auf den [Tokenizer](m2-webidl-lexer.md) baut nun
`mb_webidl::parser` auf. Er prüft die normative WebIDL-Grammatik vollständig
und liefert einen flachen Arena-Syntaxbaum mit Produktionsnamen, Tokenarten
und UTF-8-Byte-Ranges. Die unveränderte Quelle bleibt geliehen. Namens- und
Typprüfung, Auswertung der Extended Attributes und Binding-Generierung sind
weiter eigene, noch fehlende Schritte.

## Grammatik und eigene Tabellen

Die 117 Produktionen mit 312 Alternativen stammen aus demselben gelesenen
WebIDL-Quellstand wie der Tokenizer:
`296213a1d9c8293850295c6e77d3b13dce8681a9`. Sie stehen als Daten in
`data/webidl/grammar.txt`; Herkunft, Transformation, Prüfsummen und Lizenz
stehen in [HERKUNFT.md](../data/webidl/HERKUNFT.md).

Der eigene Rust-Generator in `tools/datagen/src/webidl.rs` berechnet
FIRST-/FOLLOW-Mengen und eine LL(1)-Tabelle mit 1.880 Einträgen. Er lehnt
unbekannte Symbole, nicht produktive Produktionen, reservierte interne
Terminalnamen und LL(1)-Konflikte ab. Es wurde kein fremder Parsercode
übernommen. Die abgeleiteten Grammatiktabellen tragen die Lizenz und
Herkunft des Standards; Parser und Generator sind eigener Code.

```sh
cargo run --locked -p datagen
cargo run --locked -p datagen -- --check
```

Die Tabelle wird eingecheckt, nicht zur Browserlaufzeit erzeugt oder aus
einer Datei geladen. Der bestehende Tabellencheck und die CI prüfen nun
sechs Generatorausgaben. Literalterminals werden zusätzlich gegen die
Tokenisierung geprüft.

## API und Baum

```rust
use mb_webidl::parser::{parse, Kind};

let source = "interface Example { attribute DOMString name; };";
let document = parse(source).expect("IDL syntax");
assert_eq!(document.source(), source);
for child in document.children(document.root()) {
    let node = document.node(child).expect("local node id");
    if let Kind::Production(name) = node.kind {
        // name bezeichnet die normative Grammatikproduktion.
        let original = document.text(child).expect("source range");
        let _ = (name, original);
    }
}
```

`NodeId` bezeichnet nur einen Index im jeweiligen Dokument. Kinder liegen
in zusammenhängenden Arena-Bereichen; Knoten besitzen keine rekursiven
Kindobjekte. Parsen, Traversierung in den Prüfläufen und das Freigeben eines
tiefen Baums benötigen daher keinen entsprechend tiefen Rust-Stapel.

Nur signifikante Tokens erhalten Blätter. Trivia bleibt unverändert in der
Quelle. Produktions-Ranges reichen bis zum Beginn des nächsten
signifikanten Tokens und können folgende Trivia enthalten; der Wurzelbereich
ist immer `0..source.len()`. Token-Ranges enthalten exakt ihren Lexemtext.
Der Parser normalisiert Bezeichner nicht und berechnet keine Zahlenwerte.

`ErrorKind::Syntax` nennt das erwartete Terminal oder die Produktion ohne
passende Alternative. `Error.span` zeigt auf den aktuellen Token oder eine
leere gültige Range, etwa am EOF. Der Parser muss die ganze Quelle lesen;
angehängte ungültige Definitionen werden nicht still ignoriert.

## Arbeits- und Speichergrenzen

`parse` verwendet zwei Millionen Schritte und folgende Standardgrenzen:
1 MiB Quelltext, 200.000 Baumknoten und 200.000 ausstehende Parseraufgaben.
`parse_within(source, limits, &budget)` erlaubt eine eigene Quote und eigene
Grenzen mit den vorhandenen `mb_base::{Budget, Cancel, Exhausted}`.

Die ganze Quelllänge wird vorab in Bytes vom Budget abgezogen, auch bei
einem frühen Syntaxfehler. Jede Parseraktion kostet einen weiteren Schritt.
Abbruch wird am Einstieg, an jeder Aktion und vor jedem Lexer-Aufruf
einschließlich Trivia geprüft. Innerhalb eines einzelnen Tokens und während
einer Allokation gibt es keinen zusätzlichen Kontrollpunkt.

Quell-, Knoten- und Aufgabenlimits ergeben `ErrorKind::Limit`, erschöpfte
oder abgebrochene Arbeit `ErrorKind::Exhausted`. Das sind keine Syntaxfehler;
ein Teilergebnis wird nicht zurückgegeben. Mehrere Aufrufe mit demselben
Budget teilen dessen Rest. Die Grenzen sind logische Grenzen und keine
Messung oder vollständige Begrenzung des Prozess-RAMs oder der Wandzeit.

## Corpus und Fuzzing

Alle 350 IDL-Dateien des bereits gepinnten webref-Commits werden geprüft:
348 werden gelesen und ihre Bäume auf gültige Ranges, eindeutige
Erreichbarkeit, Eltern-/Kindbereiche und vollständige Tokenabdeckung geprüft.
Zwei Dateien deklarieren `constructor()` in einem `partial interface`, was
die normative Grammatik dort nicht zulässt:

- `mediacapture-surface-control.idl`
- `webrtc-ice.idl`

Beide müssen genau am `constructor` in `PartialInterfaceMembers` mit einem
Syntaxfehler enden. Es gibt keinen allgemeinen Ausschluss fehlgeschlagener
Dateien und keine dafür aufgeweichte Grammatik. Die vorherige lexikalische
Prüfung aller 350 Dateien bleibt bestehen.

Weitere Regressionen prüfen verschiedene Definitionen und Typen,
Extended Attributes, genaue Fehler-Ranges, angehängte Eingaben, Quoten und
Limits sowie 2.000 verschachtelte Sequenztypen auf einem Faden mit 1 MiB
Rust-Stapel. Semantisch ungültige, aber syntaktisch gültige Definitionen wie
doppelte Namen oder unbekannte Typen werden bewusst erst später geprüft.

Das vorhandene Fuzzing-Ziel `webidl` prüft zusätzlich den Parser mit
50.000 Schritten, höchstens 12 KiB UTF-8-Quelltext, 8.192 Knoten und 8.192
ausstehenden Aufgaben. Syntax- und Hostfehler sind normale Ergebnisse mit
geprüften Fehler-Ranges; bei erschöpfter Arbeit muss auch das tatsächliche
Budget diesen Grund zeigen. Erfolgreiche Bäume werden ohne Rekursion auf
Zyklen, Erreichbarkeit, Ranges und exakt dieselben signifikanten Tokens wie
der Lexer geprüft. Es gelten die äußeren Grenzen aus [fuzzing.md](fuzzing.md).

## Nächste Schritte für Claude

1. Extrahierte Grammatik und FIRST-/FOLLOW-Berechnung gegen den angegebenen
   Standardstand prüfen. Generierte Tabellen nur über `datagen` ändern.
2. Baum und Quoten prüfen, insbesondere leere Produktionen, Trivia-Ranges,
   Grenzen vor dem Anlegen neuer Knoten und gemeinsam verwendete Budgets.
3. Eine semantische IR aufbauen: Definitionen zusammenführen, Partials,
   Mixins/Includes und Vererbung auflösen, Bezeichner normalisieren, Typen,
   Defaultwerte und Extended Attributes validieren. Der Syntaxbaum allein
   garantiert keine gültige oder implementierte Web-API.
4. Darauf den eigenen Binding-Generator mit Argumentprüfung, Konversionen,
   Ausnahmen und Prototypen implementieren und gegen WebIDL-/DOM-WPT prüfen.
   DOM-Bindings und Ereignisschleife fehlen weiter; M2 ist nicht abgeschlossen.
