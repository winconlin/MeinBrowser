# Sicherheit

## Sicherheitslücken melden

Bitte melde Sicherheitslücken vertraulich und nicht in öffentlichen Issues, Pull Requests oder Diskussionen.

Der Meldeweg ist die private Schwachstellenmeldung von GitHub: Im Repository unter **Security** auf **Report a vulnerability** klicken oder direkt <https://github.com/winconlin/MeinBrowser/security/advisories/new> öffnen. Die Meldung sehen nur die Betreuer des Projekts.

Hilfreich sind:

- die betroffene Komponente (Crate) und der Commit oder die Version,
- Schritte oder eine Datei, mit denen sich der Fehler nachstellen lässt,
- was ein Angreifer damit erreichen kann, soweit bekannt.

## Was danach passiert

Wir bestätigen den Eingang, klären die Meldung gemeinsam mit dir und beheben die Lücke. Danach veröffentlichen wir einen Sicherheitshinweis (GitHub Security Advisory) und nennen dich darin, wenn du das möchtest.

## Unterstützte Versionen

Es gibt noch keine veröffentlichte Version. Bis Meilenstein M5, der Prozessisolation und Sandbox bringt, ist MeinBrowser ausdrücklich nur für Entwicklung und Tests gedacht, nicht für beliebige Websites (siehe [docs/architektur.md](docs/architektur.md), Abschnitt 8.3). Meldungen zum aktuellen Stand von `main` nehmen wir trotzdem gern entgegen.

## Geltungsbereich

Alles in diesem Repository, besonders:

- Parser für Daten aus dem Netz (HTML, CSS, URL, JavaScript, Bilder, Schriften, Kompression, TLS),
- Prozesskommunikation, Sandbox und Kryptografie,
- die Lieferkette: Abhängigkeitswächter, CI und Build.

Das Bedrohungsmodell und die geplanten Schutzschichten stehen in [docs/architektur.md](docs/architektur.md), Abschnitt 8.
