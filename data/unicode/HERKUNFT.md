# Unicode-Zeichendatenbank

| | |
|---|---|
| Datensatz | Unicode Character Database (UCD), beschrieben in UAX #44 |
| Version | 18.0.0 |
| Quelle | Repository `unicode-org/unicodetools`, in dem Unicode, Inc. die Dateien der UCD pflegt, Ordner `unicodetools/data/ucd/18.0.0/`, Commit `578f6c0516f5f268292ebda50b51848407a06478` |
| Abgerufen | 24.10.2026 über raw.githubusercontent.com |
| Lizenz | Unicode License V3 (SPDX `Unicode-3.0`), Text in [LICENSE.txt](LICENSE.txt) |

## Dateien

| Datei | URL | SHA-256 |
|---|---|---|
| `UnicodeData.txt` | https://raw.githubusercontent.com/unicode-org/unicodetools/578f6c0516f5f268292ebda50b51848407a06478/unicodetools/data/ucd/18.0.0/UnicodeData.txt | `0736451de439ae7baf1425136617da495e09ee5afbe6e394374db7009ea08950` |
| `PropertyValueAliases.txt` | https://raw.githubusercontent.com/unicode-org/unicodetools/578f6c0516f5f268292ebda50b51848407a06478/unicodetools/data/ucd/18.0.0/PropertyValueAliases.txt | `06c4c8eaf7b0bf34abe73b113da1215bd784ac254d4c223600b90267caa4bbbd` |
| `LICENSE.txt` | https://raw.githubusercontent.com/unicode-org/unicodetools/578f6c0516f5f268292ebda50b51848407a06478/LICENSE | `fe5c62b543e287981db198f2acfa0ca732d12591a1024536dc8fd85dacd77104` |

Die Dateien sind unverändert. `LICENSE.txt` ist die Lizenzdatei des Repositorys, derselbe Text der Unicode License V3, auf den die Kopfzeilen der Datendateien verweisen (https://www.unicode.org/terms_of_use.html).

Offiziell veröffentlicht wird die UCD unter https://www.unicode.org/Public/18.0.0/ucd/. Von dort ließ sich beim Abruf nichts laden (unicode.org war aus der Arbeitsumgebung nicht erreichbar), ein Abgleich mit diesen Dateien steht noch aus.

## Verwendung

`cargo run -p datagen` erzeugt daraus `crates/mb-unicode/src/tables.rs`, `cargo run -p datagen -- --check` prüft in der CI, dass die Tabelle zu den Rohdaten passt.

Für eine neue Version alle Dateien aus demselben Stand ersetzen, diese Datei anpassen und den Generator laufen lassen. Die Version liest der Generator aus der Kopfzeile von `PropertyValueAliases.txt`.
