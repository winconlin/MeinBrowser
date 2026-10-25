# CSS-Regeln aus dem Abschnitt „Rendering“ des WHATWG HTML Standard

| | |
|---|---|
| Datensatz | Die CSS-Blöcke aus dem Abschnitt „Rendering“ des HTML Standard (Living Standard): das Stylesheet der Benutzeragenten und die *presentational hints* in CSS |
| Quelle | Repository `whatwg/html`, Datei `source`, Commit `b342939ba3b7e40041655fb926f6e41b54fe47ef` (22.10.2026), SHA-256 der Datei `6bacb94ff9fc3dcf336fef1ca4b48d4a3bd458f58437ff1edb68990443f3609c` |
| Abgerufen | 24.10.2026 mit `git fetch` von https://github.com/whatwg/html |
| Lizenz | © WHATWG (Apple, Google, Mozilla, Microsoft), Creative Commons Attribution 4.0 (SPDX `CC-BY-4.0`); Teile, die in Quelltext übernommen werden, stehen stattdessen unter der BSD-3-Clause-Lizenz (SPDX `BSD-3-Clause`). Text in [LICENSE.txt](LICENSE.txt) |

## Dateien

| Datei | Herkunft | SHA-256 |
|---|---|---|
| `LICENSE.txt` | `LICENSE` aus demselben Commit, unverändert | `85dc6f5ccb57a6fe8c33d158f9fc8fc7ee5655a5d3db2cdd131c6a3d0f48a864` |
| `rendering.json` | aus `source` ausgezogen, siehe unten | `b84ebec5adbc40b0df6fbce6bab8951931893a9bb0a210ab1226d10a35948207` |

`rendering.json` ist eine Liste mit einem Eintrag je CSS-Block, in der Reihenfolge des Standards. Ein Eintrag nennt

- `section`: die Überschrift, unter der der Block steht,
- `intro`: den Absatz direkt vor dem Block, der sagt, wann die Regeln gelten (leer, wenn dort keiner steht),
- `css`: den Block, Zeile für Zeile.

So entstand die Datei: Der Abschnitt reicht in `source` von Zeile 152350 (`<h2 … id="rendering">`) bis Zeile 156134 (`<h2 … id="obsolete">`). Darin ist jedes `<pre><code class="css" …>` ein Block, 38 insgesamt. Aus ihnen sind HTML-Kommentare (`<!-- … -->`) und Tags entfernt und Zeichenreferenzen dekodiert, dazu Leerzeichen am Zeilenende, die nach dem Entfernen der Kommentare übrig blieben. Überschriften und Absätze sind genauso von Tags befreit, ihr Leerraum ist zu einzelnen Leerzeichen zusammengefasst. Sonst ist nichts geändert. Die Absätze mit Regeln in Prosa (etwa die Zuordnung von Attributen zu Eigenschaften) gehören nicht dazu.

Veröffentlicht wird der Standard unter https://html.spec.whatwg.org/multipage/rendering.html; von dort ließ sich beim Abruf nichts laden (whatwg.org war aus der Arbeitsumgebung nicht erreichbar).

## Verwendung

`cargo run -p datagen` erzeugt daraus `crates/mb-style/src/ua/tables.rs`, `cargo run -p datagen -- --check` prüft in der CI, dass die Tabelle zu den Rohdaten passt. Der Generator ordnet jedem Block nach seinem `intro` eine Bedingung zu (immer, im *quirks mode*, als *presentational hint* und so weiter) und bricht bei einem unbekannten Absatz ab.

Für einen neuen Stand die Blöcke aus `source` eines neuen Commits auf dieselbe Weise ausziehen, diese Datei anpassen, den Generator laufen lassen und neue Absätze im Generator zuordnen.
