# Normative WebIDL-Grammatik

Quelle: WHATWG Web IDL, Repository [whatwg/webidl](https://github.com/whatwg/webidl), Commit `296213a1d9c8293850295c6e77d3b13dce8681a9`, Datei `index.bs`. Der Standardtext wurde am 2.11.2026 gelesen, die Lizenz am 3.11.2026 aus demselben Commit abgerufen.

Lizenz: Copyright © WHATWG (Apple, Google, Mozilla, Microsoft), `CC-BY-4.0`; in Quelltext übernommene Teile stehen stattdessen unter `BSD-3-Clause`. Die vollständige unveränderte Lizenz steht in [LICENSE.txt](LICENSE.txt). Die eigene Generator- und Parserimplementierung ist davon unabhängig und folgt der Repository-Lizenz; die abgeleiteten Tabellen tragen die BSD-3-Clause-Kennzeichnung und Herkunft.

`grammar.txt` enthält die 117 normativen Produktionen aus den Blöcken `<pre class="grammar" id="prod-…">` in `index.bs`: HTML-Markup entfernt, Zeichenreferenzen dekodiert, umgebender Leerraum vereinheitlicht. Die Reihenfolge der Produktionen, Alternativen und Symbole blieb erhalten. `ε` bezeichnet eine leere Alternative. Es wurde kein Parsercode übernommen und keine Grammatikregel erweitert.

| Datei | SHA-256 |
| --- | --- |
| ursprüngliche `index.bs` (nicht eingecheckt) | `5ca85e0bd132d39e98aa9b63b02bc61c257d182785639471e00c545dcd5e6a66` |
| extrahierte `grammar.txt` | `3049083114e81c8a35c4b2f59c957b9e8e8252352cdf9c3769667409f61e2151` |
| unveränderte `LICENSE.txt` | `85dc6f5ccb57a6fe8c33d158f9fc8fc7ee5655a5d3db2cdd131c6a3d0f48a864` |

`cargo run -p datagen` erzeugt daraus FIRST-/FOLLOW-Mengen und eine konfliktfreie LL(1)-Tabelle für `crates/mb-webidl/src/parser/tables.rs`. `cargo run -p datagen -- --check` prüft die Reproduzierbarkeit. Unbekannte Symbole, nicht produktive Produktionen und LL(1)-Konflikte sind Fehler; für einen neuen Stand sind Quelle, Lizenz, diese Herkunft und die Tabellen gemeinsam zu prüfen.
