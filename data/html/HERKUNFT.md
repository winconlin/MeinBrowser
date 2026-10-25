# Benannte Zeichenreferenzen aus dem WHATWG HTML Standard

| | |
|---|---|
| Datensatz | Tabelle der benannten Zeichenreferenzen (*named character references*) des HTML Standard (Living Standard) |
| Quelle | Repository `whatwg/html-build`, Commit `283a3531a61106d07d9a7d9fb3e6f3b9bfd33d70` (12.11.2025), aus dem die Tabelle im Standard erzeugt wird |
| Abgerufen | 24.10.2026 mit `git fetch` von https://github.com/whatwg/html-build |
| Lizenz | © WHATWG (Apple, Google, Mozilla, Microsoft), Creative Commons Attribution 4.0 (SPDX `CC-BY-4.0`); Teile, die in Quelltext übernommen werden, stehen stattdessen unter der BSD-3-Clause-Lizenz (SPDX `BSD-3-Clause`). Text in [LICENSE.txt](LICENSE.txt) |

## Dateien

Jede Datei stammt unverändert aus dem genannten Commit.

| Datei | Pfad im Repository | SHA-256 |
|---|---|---|
| `LICENSE.txt` | `LICENSE` | `85dc6f5ccb57a6fe8c33d158f9fc8fc7ee5655a5d3db2cdd131c6a3d0f48a864` |
| `entities.json` | `entities/out/entities.json` | `d741d877ac77c4194c4ad526b5b4a19aef8dfe411ab840a466891cdbb9f362e6` |

`entities.json` enthält alle 2231 Namen mit dem führenden `&`, je mit den Codepunkten und den Zeichen, für die sie stehen. Laut `entities/README.md` im Repository wird die Datei aus `unicode.xml` der W3C erzeugt und eingecheckt, weil sie sich kaum noch ändert. Veröffentlicht wird sie auch unter https://html.spec.whatwg.org/entities.json; von dort ließ sich beim Abruf nichts laden (whatwg.org war aus der Arbeitsumgebung nicht erreichbar).

Die Algorithmen in `crates/mb-html` folgen der Datei `source` aus `whatwg/html`, Commit `b342939ba3b7e40041655fb926f6e41b54fe47ef` (22.10.2026). Diese Datei liegt nicht im Repository.

## Verwendung

`cargo run -p datagen` erzeugt daraus `crates/mb-html/src/entities/tables.rs`, `cargo run -p datagen -- --check` prüft in der CI, dass die Tabelle zu den Rohdaten passt.

Für einen neuen Stand beide Dateien aus demselben Commit ersetzen, diese Datei anpassen und den Generator laufen lassen.
