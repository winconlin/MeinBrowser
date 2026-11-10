# M2: eigener WebIDL-Tokenizer

`mb-webidl::lexer` liefert den ersten ausführbaren Baustein für den in
[Architektur, Abschnitt 4.10](architektur.md#410-web-apis-und-bindings)
geplanten IDL-Leser und Binding-Generator. Er zerlegt Quelltext nach dem
Anhang [IDL grammar](https://webidl.spec.whatwg.org/#idl-grammar).
Gelesener Quellstand des Standards:
[`296213a1d9c8293850295c6e77d3b13dce8681a9`](https://github.com/whatwg/webidl/blob/296213a1d9c8293850295c6e77d3b13dce8681a9/index.bs#L15426).
Es wird keine fremde Parser- oder Browserimplementierung verwendet. Inzwischen baut darauf der [Syntaxparser](m2-webidl-parser.md) auf; ergänzt wird er durch den [Namensindex](m2-webidl-index.md). Grenzen und nächste Schritte stehen separat.

## API und Verhalten

```rust
use mb_webidl::lexer::{Kind, Lexer};

let source = "interface Example { attribute DOMString name; };";
for token in Lexer::new(source).filter(|token| !token.kind.is_trivia()) {
    assert_eq!(source.get(token.span.clone()), Some(token.text));
    if token.kind == Kind::Identifier {
        // token.text enthält den unveränderten Bezeichner.
    }
}
```

Die Quelle ist ein geliehenes `&str`, also eine Folge gültiger
Unicode-Skalare in UTF-8. Jeder Token enthält Art, unveränderten Quelltext
und eine Byte-Range auf UTF-8-Grenzen. Kommentare und Leerraum bleiben als
Trivia erhalten. Der Iterator lässt EOF weg; `next_token()` liefert am
Ende wiederholt `Kind::End` mit einer leeren Range an der Quelltextlänge.

Erkannt werden Integer, Decimal, Identifier, String, Whitespace, Comment,
feste Grammatikterminals und `other`. Die 70 reservierten Wörter werden
case-sensitive erkannt; ein längerer Bezeichner gewinnt vor einem
Schlüsselwortpräfix. Werte und Bezeichner werden noch nicht normalisiert,
Zahlen nicht in begrenzte Rust-Zahlentypen umgewandelt.

Einige Unterschiede zu JavaScript sind für den späteren Parser wichtig:

- Es gilt der längste lexikalische Treffer. `08` ergibt zwei Integer-Tokens,
  `08.5` einen Decimal-Token. Hexadezimale und oktale Integer folgen den
  WebIDL-Regeln. `1...` beginnt mit dem Decimal-Token `1.`.
- Strings enden am nächsten Anführungszeichen. Backslashes sind keine
  Escape-Einleitung; Zeilenumbrüche im String sind lexikalisch erlaubt.
- Nur Tab, LF, CR und ASCII-Leerzeichen sind Whitespace. Ein BOM oder NBSP
  wird nicht still entfernt. Nicht-ASCII-Zeichen außerhalb eines Strings
  oder Kommentars fallen auf jeweils einen `other`-Skalar zurück.
- Blockkommentare enden am ersten `*/`. Die Line-Comment-Regel des Standards
  verwendet Perl-5-Regex-Syntax: Das dortige `.` schließt LF aus, nicht CR
  oder U+2028.
- Nicht geschlossene Strings und Blockkommentare passen nicht auf ihre
  jeweiligen Regeln. Der Anfang fällt daher auf `other` beziehungsweise
  feste Einzelterminals zurück. Das ist keine Aussage, dass das Fragment
  gültige WebIDL ist: Syntaxfehler muss der spätere Grammatikparser melden.

## Ressourcen und Prüfung

Der Tokenizer arbeitet ohne Rekursion und ohne eigene Heap-Allokationen.
Er hält nur die geliehene Quelle, eine Position und den Zustand einer
erfolglosen Blockkommentarsuche. Der Aufwand ist linear in der Quelllänge:
Insbesondere wird ein Rest ohne `*/` nach der ersten erfolglosen Suche
nicht für jeden weiteren Kommentar-Anfang erneut durchsucht. Die Quelle
selbst und vom Aufrufer gesammelte Tokens sind dadurch nicht größenbegrenzt.

Elf Tests prüfen Keyword-Priorität, Trivia, UTF-8-Grenzen, String-/Kommentarregeln,
EOF und große offene Kommentarfolgen und Zahlen. Ein Test vergleicht
2.800 kurze Eingaben mit einem unabhängigen Orakel für vollständige
Zahlentokens. Alle 350 IDL-Dateien des bereits gepinnten webref-Commits
`85e517c8fb6271560d09c1b550c40bcfc849f580` werden zusätzlich vollständig
tokenisiert: 191.651 Tokens einschließlich Trivia, keine `other`-Zeichen.
Das ist eine lexikalische Prüfung, keine Validierung der Definitionen.

`tools/testdata` holt neben `ed/css/` nun auch `ed/idl/` aus demselben
Commit. Es gibt keine neue externe Abhängigkeit; `testdata` ist eine eigene
Dev-Abhängigkeit. Der separate Fuzzing-Workspace hängt vom eigenen
`mb-webidl` ab und besitzt dafür einen weiteren Lockfile-Eintrag.

Das sechzehnte Fuzzing-Ziel `webidl` prüft Fortschritt, vollständige
Quelltextabdeckung, UTF-8-Grenzen, stabilen EOF und die erneute Tokenisierung
isolierter Tokens. Die Eingabe bleibt auf 4 KiB begrenzt. Ungültiges UTF-8
wird nur im Harness mit Ersetzungszeichen gelesen; dadurch kann dessen
Textquelle bis zu 12 KiB lang werden. Es gelten die vorhandenen äußeren
Zeit- und RSS-Limits aus [fuzzing.md](fuzzing.md). Korpora und Rohprotokolle
werden wie dort und in [SECURITY.md](../SECURITY.md) beschrieben behandelt.

```sh
cargo run --locked -p testdata
cargo test --locked -p mb-webidl
cargo test --locked --offline --manifest-path fuzz/Cargo.toml --lib
python3 -m unittest discover -s fuzz -p test_run.py
python3 fuzz/run.py webidl --seconds 30 --seed 1
```

## Anschluss und Prüfpunkte für Claude

1. Lexikalische Regeln und Terminalmenge gegen den oben genannten
   Standardstand prüfen, besonders die Fallback-Regeln und WebIDLs eigene
   String- und Kommentarregeln. Bei Standardänderungen die Referenz und die
   feste Terminalmenge gemeinsam aktualisieren.
2. Der LL(1)-Syntaxparser ist implementiert und liefert Produktionsbäume
   für Definitionen, Typen, Mitglieder und Extended Attributes. Seine
   Prüfpunkte stehen in [m2-webidl-parser.md](m2-webidl-parser.md). Der
   Namensindex entfernt inzwischen den Escape-Unterstrich; Zahlenwerte
   bleiben semantisch auszuwerten.
3. Den [Namensindex](m2-webidl-index.md) für Definitionen, Partials,
   Typverweise, Includes und Vererbung prüfen. Darauf noch Typ-/Member-IR,
   vollständige Typprüfung und die Bedeutung der Extended Attributes aufbauen.
   Das Lesen der webref-Dateien bestätigt keine vollständige Semantik.
4. Anschließend den Binding-Generator mit Argumentprüfung, Konversionen,
   Ausnahmen und Prototypen an die vorhandene JS-Engine anbinden und gegen
   WebIDL-/DOM-WPT prüfen. DOM-Bindings und Ereignisschleife fehlen weiter;
   dieses Paket schließt M2 nicht ab.
