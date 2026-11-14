# MeinBrowser – Architekturkonzept

Stand: Entwurf vom 24.10.2026, Ziele und Meilensteine am 11.11.2026 mit dem Repository abgeglichen · Status: zur Diskussion

Dieses Dokument beschreibt, wie MeinBrowser aufgebaut werden soll: welche Bausteine es gibt, wo wir bewusst andere Wege gehen als heutige Browser, wie die eigene JavaScript-Engine aussieht, wie Chrome- und Firefox-Erweiterungen laufen sollen und in welcher Reihenfolge wir bauen. Es ist ein Entwurf. Alles, was noch entschieden werden muss, steht gesammelt in [Abschnitt 14](#14-offene-entscheidungen). Wie weit die Umsetzung ist, zeigen [1.3](#13-stand-der-ziele) je Ziel und [12.1](#121-stand-der-meilensteine) je Meilenstein.

## Inhalt

1. [Ziele und Leitplanken](#1-ziele-und-leitplanken)
2. [Die Grundideen](#2-die-grundideen)
3. [Gesamtbild](#3-gesamtbild)
4. [Die Komponenten im Einzelnen](#4-die-komponenten-im-einzelnen)
5. [Erweiterungen für Chrome und Firefox](#5-erweiterungen-für-chrome-und-firefox)
6. [Modifizierbarkeit und Skins](#6-modifizierbarkeit-und-skins)
7. [Ressourcenschonung](#7-ressourcenschonung)
8. [Sicherheit](#8-sicherheit)
9. [Plattformen](#9-plattformen)
10. [Kompatibilität messen](#10-kompatibilität-messen)
11. [Aufbau des Repositorys](#11-aufbau-des-repositorys)
12. [Roadmap und Meilensteine](#12-roadmap-und-meilensteine)
13. [Der Aufwand, ehrlich betrachtet](#13-der-aufwand-ehrlich-betrachtet)
14. [Offene Entscheidungen](#14-offene-entscheidungen)
15. [Quellen](#15-quellen)

---

## 1. Ziele und Leitplanken

**Ziele**

- **Open Source** unter der MIT-Lizenz (wie im Repository festgelegt).
- **Extrem ressourcenschonend**: wenig Arbeitsspeicher, schneller Start, kaum Energieverbrauch im Leerlauf.
- **Maximal kompatibel**: gemessen an den offiziellen Testsammlungen des Webs (web-platform-tests, Test262 und weitere, siehe [Abschnitt 10](#10-kompatibilität-messen)) und an echten Webseiten.
- **Erweiterungen**: Chrome- und Firefox-Erweiterungen laufen ohne Anpassung.
- **Sicher**: Weder Webseiten noch Erweiterungen noch Angreifer im Netzwerk kommen an Daten anderer Seiten oder an den Rechner (siehe [Abschnitt 8](#8-sicherheit)).
- **Modifizierbar und skinbar**: Aussehen und Verhalten lassen sich ändern, ohne den Browser neu zu übersetzen.
- **Läuft auf jedem System**: Linux, Windows und macOS zuerst, danach Android, BSD und weitere.
- **Eigene Engine und eigener Browsercode**: eine von Grund auf neue Engine ohne fremde Browser-Engines. Als begrenzte Ausnahme kann der Nutzer ein proprietäres Widevine-Modul für DRM aktivieren ([4.13](#413-bilder-audio-und-video)).

Woran wir jedes Ziel messen und wo es heute steht, zeigt [1.3](#13-stand-der-ziele).

### 1.1 Was „komplett eigen“ hier bedeutet

Ganz ohne fremden Code kann kein Programm laufen: Um ein Fenster zu öffnen, ins Netz zu gehen oder die Grafikkarte zu nutzen, muss der Browser mit dem Betriebssystem sprechen. Entschieden am 24.10.2026: Erlaubt sind die Rust-Standardbibliothek sowie die Schnittstellen von Betriebssystem und Grafiktreibern. Den Browsercode darüber bauen wir selbst, auch TLS und Kryptografie. Ergänzend ist ein optionales Widevine-CDM als eng begrenzte Laufzeitausnahme erlaubt; die eigene Engine bleibt davon unabhängig. Im Einzelnen:

| Kategorie | Regel |
|---|---|
| Browserkern und Engine | Alles selbst geschrieben: Parser, DOM, CSS, Layout, Rendering, Schriften, JavaScript, WebAssembly, Netzwerk, TLS, Bilder, Medien, Erweiterungen, Oberfläche. |
| Optionales DRM-Modul | Ein externes, proprietäres Widevine Content Decryption Module (CDM) ist als begrenzte Ausnahme erlaubt. EME-Schnittstelle und CDM-Host schreiben wir selbst; das Modul läuft getrennt und wird nur nach Zustimmung bezogen und aktiviert. Es ist keine Cargo-Abhängigkeit des Browserkerns. Bezug und etwaige Weitergabe setzen entsprechende Rechte voraus; das CDM erhält durch die Browser-Lizenz keine MIT-Lizenz. |
| Rust-Sprache und Standardbibliothek | Erlaubt (`core`, `alloc`, `std`). |
| Schnittstellen des Betriebssystems | Erlaubt: Systemaufrufe, Fenster und Eingabe, Zwischenablage, Barrierefreiheit, Zertifikatsspeicher, Schlüsselbund. Die Anbindung (FFI-Deklarationen) schreiben wir selbst, also keine Crates wie `libc`, `windows-sys` oder `objc`. |
| Grafiktreiber-Schnittstellen | Erlaubt: Vulkan, Metal, Direct3D 12. |
| Werkzeuge, die nicht ausgeliefert werden | Annahme: erlaubt. Gemeint sind Compiler, Cargo, Git, Profiler und Fuzzing-Werkzeuge der Toolchain. Dazu kommt als einzige fremde Crate `libfuzzer-sys` im getrennten Fuzzing-Workspace `fuzz/` (entschieden am 30.10.2026, [14](#14-offene-entscheidungen)). |
| Testsammlungen | Annahme: erlaubt. web-platform-tests, Test262, html5lib-tests und andere sind Prüfmaßstab, kein Bestandteil des Browsers. Bisher holt `tools/testdata` neun Sammlungen in fester Version ([Abschnitt 10](#10-kompatibilität-messen)). |
| Datensätze, die man nicht erfinden kann | Annahme: erlaubt, eingelesen mit eigenen Generatoren. Dazu gehören die Unicode-Zeichendatenbank, CLDR, die IANA-Zeitzonendatenbank, die Public Suffix List, die Liste vertrauenswürdiger Wurzelzertifikate, die HSTS-Preload-Liste, Silbentrennmuster und die IDL-Blöcke aus den Spezifikationen. Die Lizenzen jedes Datensatzes dokumentieren wir im Repository. Bisher liegen unter `data/` fünf Datensätze, jeder mit Quelle, Version und Lizenz in einer `HERKUNFT.md`: die Unicode-Zeichendatenbank, die Daten des WHATWG Encoding Standard, die benannten Zeichenreferenzen und die CSS-Blöcke aus dem HTML Standard sowie die WebIDL-Grammatik. |

**Durchsetzung:** Eine CI-Prüfung (`tools/deps-guard`) lehnt jeden Pull Request ab, dessen `Cargo.lock` ein Paket mit externer Quelle enthält (also mit einem `source`-Eintrag). Weil Pfadabhängigkeiten keinen solchen Eintrag haben, prüft sie zusätzlich die Metadaten von Cargo mit allen Features: Jedes Paket im Abhängigkeitsgraphen, auch optionale Abhängigkeiten, muss ein Mitglied des Workspace sein, und sein Manifest muss nach dem Auflösen symbolischer Verweise im Workspace liegen. Im getrennten Fuzzing-Workspace prüft sie `fuzz/Cargo.lock`: Fremd darf dort nur sein, was `libfuzzer-sys` von crates.io mitbringt ([8.2](#82-schutzschichten), Punkt 14). So bleibt die Regel nicht nur ein Vorsatz. Ein separat geladenes CDM wird von dieser Cargo-Prüfung nicht erfasst und benötigt eine eigene Herkunfts-, Integritäts- und Updateprüfung nach [4.13](#413-bilder-audio-und-video). Die Widevine-Ausnahme erlaubt keine sonstigen fremden Bibliotheken im Browserkern.

### 1.2 Neu, wo es nützt, und exakt, wo das Web es verlangt

Webseiten sehen vom Browser nur sein Verhalten. Dieses Verhalten ist durch Spezifikationen festgelegt (WHATWG, W3C, TC39, Khronos, IETF) und in der Praxis zusätzlich durch das, was Chrome und Firefox tun. Hier dürfen wir nicht abweichen, sonst leidet die Kompatibilität.

Wie wir dieses Verhalten im Inneren herstellen, ist dagegen völlig frei. Dort setzen die Grundideen in [Abschnitt 2](#2-die-grundideen) an. Eine Einschränkung gilt trotzdem: Wo eine bekannte Technik nachweislich am besten funktioniert (zum Beispiel Inline-Caches für JavaScript-Eigenschaftszugriffe), bauen wir sie selbst nach, statt aus Prinzip eine schlechtere zu erfinden. Die Funktionalität soll unter dem Eigenbau nicht leiden.

### 1.3 Stand der Ziele

Stand 14.11.2026, nach PR #109. Die Tabelle nennt zu jedem Ziel den Maßstab und was das Repository davon heute zeigt. Den Stand je Meilenstein enthält [12.1](#121-stand-der-meilensteine).

| Ziel | Maßstab | Stand |
|---|---|---|
| Open Source | MIT-Lizenz | Erfüllt: `LICENSE` und alle Crates stehen unter MIT. Beitragsregeln fehlen noch ([14](#14-offene-entscheidungen), Punkt 6). |
| Extrem ressourcenschonend | Zielwerte in [Abschnitt 7](#7-ressourcenschonung) | Noch nicht messbar: Es gibt weder Tabs noch Speicherzähler oder Messläufe in der CI, gemessen ist nur die Programmgröße. Angelegt sind der Dokumentspeicher mit 32-Bit-Nummern, der JavaScript-Heap mit 32-Bit-Abständen und das Übersetzen von Funktionen erst beim ersten Aufruf. |
| Maximal kompatibel | Testsammlungen aus [Abschnitt 10](#10-kompatibilität-messen), echte Webseiten | Das Kriterium für M1 ist erfüllt ([12.1](#121-stand-der-meilensteine)). Von Test262 bestehen alle Syntaxläufe; ausgeführt bestehen gut 64 Prozent der Testdateien ohne Intl. Echte Webseiten lassen sich noch nicht laden: Es gibt kein Netzwerk, und Seiten führen keine Skripte aus. |
| Erweiterungen | Die Zielerweiterungen laufen ohne Änderung ([5.4](#54-kompatibilität-prüfen)) | Nicht begonnen, `mb-ext` ist leer. Voraussetzungen sind JavaScript mit DOM-Bindings (M2), Netzwerk (M3) sowie Speicherung und Oberfläche (M5). |
| Sicher | Schutzschichten aus [8.2](#82-schutzschichten) | Vorhanden: Abhängigkeitswächter, CI ohne fremde Actions, `SECURITY.md`, `#![forbid(unsafe_code)]` in allen Crates außer `mb-platform`, `mb-ipc` und `mb-js` (auch `mb-js` enthält bisher kein `unsafe`), 16 Fuzz-Ziele bei Pull Requests mit Codeänderungen und nachts, Arbeits- und Bytebudgets für Seiten und ein Schrittbudget für die Ausführung von JavaScript. Es fehlen Prozessisolation, Sandbox und TLS; bis M5 ist der Browser nur für Entwicklung und Tests gedacht ([8.3](#83-sicherheit-in-jedem-meilenstein)). |
| Modifizierbar und skinbar | Skins, Anordnung und Mods aus [Abschnitt 6](#6-modifizierbarkeit-und-skins) | Nicht begonnen, `mb-ui` ist leer. Das Programm zeigt bisher ein Fenster mit einer Seite, ohne Oberfläche. |
| Läuft auf jedem System | Stufen aus [Abschnitt 9](#9-plattformen) | Fenster, Eingabe und CPU-Bildausgabe laufen unter Linux (X11, unter Wayland über XWayland), Windows und macOS. Die CI baut und testet jeden Pull Request auf je einem Läufer dieser drei Systeme, die Referenztests nur unter Linux. Es fehlen der eigene Wayland-Client, die Ausgabe über die GPU und die Stufen 2 und 3. |
| Eigene Engine und eigener Browsercode | Abhängigkeitswächter ([1.1](#11-was-komplett-eigen-hier-bedeutet)) | Erfüllt: `Cargo.lock` enthält kein fremdes Paket, `fuzz/Cargo.lock` nur `libfuzzer-sys` mit dem, was es mitbringt. Die Widevine-Anbindung ist nicht umgesetzt. |

---

## 2. Die Grundideen

Acht Entscheidungen prägen die Architektur. Zu jeder steht, was sie bringt und wo das Risiko liegt.

| Bereich | Häufiger Weg | MeinBrowser |
|---|---|---|
| Dokumentmodell | Objektbaum, jeder Knoten ein eigenes Heap-Objekt mit Zeigern | Spaltenspeicher mit 32-Bit-Indizes |
| Änderungen verfolgen | viele einzelne „dirty“-Markierungen je Subsystem | ein gemeinsames Änderungsjournal für alle |
| Spezifikationstreue | Spezifikationsschritte als Kommentare von Hand | Spezifikation als Quelltext, generiert und rückverfolgbar |
| Speicherfreigabe | ein Garbage Collector pro Prozess | Speicherregionen pro Dokument, beim Schließen sofort frei |
| Start neuer Fenster und Frames | eingebaute Objekte werden je Realm neu erzeugt oder aus einem Snapshot geladen | geteiltes, unveränderliches Abbild, Objekte entstehen erst bei Bedarf |
| Hintergrund-Tabs | verwerfen und beim Zurückkehren neu laden | einfrieren und als Abbild auslagern, ohne Neuladen |
| Grafik | häufig getrennte Rasterwege für Text und übrige Grafik | eine Vektorpipeline für alles, GPU und CPU rechnen gleich |
| Erweiterungen | je Browser ein eigenes API-Modell | ein Kern, `chrome.*` und `browser.*` aus einer Beschreibung erzeugt |

### 2.1 Spaltenspeicher statt Objektbaum

Das Dokument (DOM) ist kein Geflecht aus Objekten, sondern eine Tabelle. Jeder Knoten ist eine Nummer (32 Bit, plus Generationszähler gegen veraltete Verweise). Seine Eigenschaften liegen in Spalten: Knotenart, Elternknoten, erstes Kind, nächstes Geschwister, Name, Attribute, Text, berechneter Stil, Layout-Ergebnis, Markierungen.

- **Vorteile:** Wenig Speicher (32-Bit-Indizes statt 64-Bit-Zeigern, keine Verwaltungsdaten pro Einzelobjekt). Durchläufe sind cachefreundlich und lassen sich leicht auf Threads verteilen. Rust kommt damit natürlich zurecht, ohne `Rc<RefCell<…>>`-Geflechte. Ein ganzes Dokument lässt sich am Stück kopieren, einfrieren oder auf die Festplatte schreiben.
- **Risiko:** Manche DOM-Operationen (große Teilbäume verschieben, Knoten adoptieren) brauchen sorgfältige Freilisten und Kompaktierung. Das klären wir früh mit Messungen.

### 2.2 Ein Änderungsjournal für alles

Jede Änderung am Dokument landet in einem Journal: Knoten eingefügt, Attribut geändert, Text geändert. Alle Interessenten lesen dasselbe Journal: Stil-Invalidierung, Layout, `MutationObserver`, der Barrierefreiheitsbaum, die Entwicklerwerkzeuge und Erweiterungen.

- **Vorteile:** Eine einzige, prüfbare Stelle für „was hat sich geändert“. Fehler durch vergessene Invalidierung, eine häufige Fehlerquelle in Browsern, werden seltener. Neue Interessenten kommen ohne Eingriffe in den Kern hinzu.
- **Risiko:** Das Journal darf bei massenhaften Änderungen nicht zum Flaschenhals werden. Deshalb fasst es Folgen gleichartiger Änderungen zusammen und wird pro Darstellungsschritt geleert.

### 2.3 Spezifikation als Quelltext

Die WHATWG-Standards sind als Algorithmen geschrieben, viele davon als Zustandsautomaten (HTML-Tokenizer, Baumaufbau, URL-Parser, CSS-Syntax). Diese Teile beschreiben wir in einer kleinen eigenen Beschreibungssprache, nah am Wortlaut der Spezifikation. Ein eigener Generator erzeugt daraus Rust-Code, und jeder erzeugte Schritt trägt einen Verweis auf den Abschnitt der Spezifikation.

- **Vorteile:** Hohe Treue zur Spezifikation. Ändert sich ein Standard (die WHATWG-Standards werden laufend fortgeschrieben), meldet ein Werkzeug, welche referenzierten Abschnitte sich geändert haben. Die Abdeckung wird messbar.
- **Risiko:** Nicht alles lässt sich generieren. Layout und Grafik schreiben wir von Hand. Der Generator ist selbst Code, der gepflegt werden muss, deshalb setzen wir ihn nur dort ein, wo die Spezifikation wirklich tabellarisch ist.

### 2.4 Speicherregionen pro Dokument

Der JavaScript-Speicher ist in Regionen aufgeteilt: eine pro Agent (eine Ereignisschleife im Sinne der HTML-Spezifikation) und darin eine Unterregion pro Dokument. Objekte verschiedener Agents können sich laut Spezifikation nicht gegenseitig referenzieren, sie tauschen nur Kopien aus (`postMessage`, strukturiertes Klonen). Innerhalb einer Region arbeitet ein Garbage Collector mit Generationen.

- **Vorteile:** Wird ein Tab geschlossen, wird seine Region auf einen Schlag freigegeben, ohne Garbage-Collection-Lauf. Gibt es keine Verweise zwischen Dokumenten (der Normalfall), gilt das auch beim Wegnavigieren. Regionen lassen sich einzeln messen, begrenzen und einfrieren.
- **Risiko:** Gleichherkunfts-Frames dürfen Objekte teilen. Solche Verweise erfasst eine Schreibbarriere, und die betroffene Unterregion wird dann normal vom Garbage Collector behandelt statt pauschal freigegeben.

### 2.5 Geteilte, unveränderliche Abbilder

Alles, was in jedem Prozess gleich ist, wird beim Übersetzen einmal vorberechnet und als schreibgeschütztes Abbild per Speicherabbildung (`mmap`) von allen Prozessen geteilt: die Vorlagen aller eingebauten JavaScript-Objekte und DOM-Schnittstellen, das vorübersetzte Standard-Stylesheet, die Unicode-Tabellen. Systemschriften werden ebenfalls nur abgebildet, nicht kopiert.

Ein neues Realm (Fenster, Frame, Worker, Erweiterungskontext) erzeugt seine eingebauten Objekte erst, wenn ein Skript sie tatsächlich anfasst. Jedes Realm bekommt dabei weiterhin eigene Objekte, wie die Spezifikation es verlangt (`Array` im Frame ist nicht `Array` im Hauptdokument). Die Vorlagen dafür, also Gestalt, Eigenschaftstabellen und Code, kommen aus dem geteilten Abbild.

- **Vorteile:** Neue Realms starten fast ohne Kosten. Von vielen hundert DOM-Schnittstellen benutzt eine typische Seite nur einen Bruchteil.
- **Risiko:** Die verzögerte Erzeugung muss für Skripte unsichtbar sein. Das prüfen Test262 und die WPT-Tests zu WebIDL.

### 2.6 Einfrieren statt Verwerfen

Weil Dokument (Spaltenspeicher) und JavaScript-Speicher (Regionen) zusammenhängende Blöcke sind, lässt sich ein Hintergrund-Tab komplett einfrieren: anhalten, komprimieren, auf die Festplatte schreiben. Beim Zurückkehren wird er in seinem genauen Zustand wiederhergestellt, inklusive Formulareingaben, Scrollposition und Skriptzustand, ohne Neuladen.

- **Vorteile:** Sehr viele offene Tabs bei geringem Speicherverbrauch, ohne dass Seiten ihren Zustand verlieren.
- **Risiko:** Seiten mit offenen Verbindungen (WebSocket, WebRTC), laufender Wiedergabe oder geteiltem Speicher mit Workern lassen sich nicht ohne Verlust einfrieren. Für sie gelten Regeln ähnlich denen des Back-Forward-Caches der HTML-Spezifikation: Sie bleiben aktiv oder werden gedrosselt.

### 2.7 Eine Grafikpipeline für alles

Seiteninhalt, Text, Oberfläche und Skins laufen durch dieselbe Pipeline: Das Layout erzeugt eine kompakte Szenenliste (Pfade, Bilder, Glyphen, Effekte), die ein eigener, auf Compute-Shadern basierender Vektor-Renderer auf der GPU zeichnet. Ein CPU-Renderer mit denselben Algorithmen ist Rückfallebene und zugleich Referenz.

- **Vorteile:** Ein Codepfad statt mehrerer. Glyphen sind einfach Pfade. Die CPU-Variante macht Tests deterministisch und läuft auf jedem Gerät, auch ohne brauchbaren Grafiktreiber.
- **Risiko:** GPU-Vektorrasterung mit Compute-Shadern ist ein aktives Forschungsgebiet und anspruchsvoll. Für alte GPUs ohne Compute-Unterstützung zeichnet die CPU und lädt nur fertige Kacheln hoch.

### 2.8 Ein Erweiterungskern, zwei Gesichter

Intern gibt es genau ein Modell für Erweiterungen, mit Fähigkeiten (Capabilities) statt verstreuter Berechtigungsprüfungen. Die APIs beschreiben wir einmal in einem eigenen Beschreibungsformat. Ein Generator erzeugt daraus `chrome.*` (Callbacks und Promises) und `browser.*` (Promises, Firefox-Stil), samt Argumentprüfung und Testgerüsten. Details stehen in [Abschnitt 5](#5-erweiterungen-für-chrome-und-firefox).

---

## 3. Gesamtbild

### 3.1 Prozesse und Zellen

Seit den Spectre-Angriffen gilt: Inhalte verschiedener Websites gehören in getrennte Betriebssystemprozesse, weil Seitenkanäle Speicher innerhalb eines Prozesses auslesen können. Wir nennen eine solche Isolationseinheit **Zelle**.

```mermaid
flowchart LR
  subgraph Steuerung [Steuerprozess]
    UI[Oberfläche]
    TABS[Tabs, Fenster, Profile]
    EXTK[Erweiterungskern]
  end
  subgraph Dienste [Dienstprozesse]
    NET[Netzwerk und TLS]
    GPU[GPU und Compositor]
    STO[Speicherdienst]
  end
  subgraph Inhalt [Inhaltszellen, eine pro Website]
    DOC[DOM, Stil, Layout, JavaScript]
  end
  subgraph Erw [Erweiterungszellen]
    BG[Hintergrundkontexte und Erweiterungsseiten]
  end
  UI --> TABS
  TABS <--> DOC
  EXTK <--> BG
  DOC -- Anfragen --> NET
  BG -- Anfragen --> NET
  DOC -- Szenenlisten --> GPU
  UI -- Szenenlisten --> GPU
  DOC <--> STO
  BG <--> STO
```

- **Steuerprozess:** Oberfläche, Tab- und Fensterverwaltung, Profile, Berechtigungen, Erweiterungskern. Er hat die meisten Rechte und führt kein Webseiten-JavaScript aus.
- **Dienstprozesse:** Netzwerk (inklusive TLS und Cookies), GPU und Compositor, Speicherdienst (IndexedDB, Cache, Verlauf). Auf Geräten mit wenig Speicher lassen sie sich zu einem einzigen Dienstprozess zusammenlegen.
- **Inhaltszellen:** eine pro Website (Schema plus registrierbare Domain laut Public Suffix List). Sie sind stark eingeschränkt (Sandbox) und kommen an Dateien, Netz und Grafik nur über Nachrichten an die anderen Prozesse.
- **Erweiterungszellen:** Hintergrundkontexte, Popups und Optionsseiten von Erweiterungen.
- **Prozesskommunikation (IPC):** eigene, typisierte Nachrichten über Ringpuffer in gemeinsamem Speicher. Die Nachrichtentypen werden aus einer Schnittstellenbeschreibung generiert, damit Absender und Empfänger nie auseinanderlaufen.

### 3.2 Der Weg einer Seite

```mermaid
flowchart LR
  A[Bytes aus dem Netz] --> B[Zeichenkodierung erkennen]
  B --> C[HTML-Tokenizer]
  C --> D[Baumaufbau]
  D --> E[(Dokumentspeicher)]
  E --> J[Änderungsjournal]
  J --> F[Stil]
  F --> G[Layout]
  G --> H[Szenenliste]
  H --> I[Compositor]
  I --> K[GPU- oder CPU-Renderer]
  S[JavaScript] <--> E
```

1. Der Netzwerkprozess liefert Bytes. Die Zeichenkodierung wird nach dem Encoding-Standard erkannt.
2. Tokenizer und Baumaufbau (generiert nach [2.3](#23-spezifikation-als-quelltext)) füllen den Dokumentspeicher. Skripte können den Parser laut Spezifikation anhalten und weiterschreiben (`document.write`).
3. Das Änderungsjournal sagt dem Stilsystem, welche Knoten neu zu berechnen sind.
4. Das Layout erzeugt einen unveränderlichen Fragmentbaum und daraus die Szenenliste.
5. Die Szenenliste geht über gemeinsamen Speicher an den Compositor. Er übernimmt Scrollen und einfache Animationen (Verschiebung, Transparenz) selbst, sodass Scrollen flüssig bleibt, auch wenn JavaScript gerade rechnet.

### 3.3 Threads und Planung

- Jede Inhaltszelle hat einen Hauptthread für die Ereignisschleife (wie es die HTML-Spezifikation vorgibt) und einen gemeinsamen Pool von Hilfsthreads für Stil, Layout, Bilddekodierung, Textformung und Garbage Collection.
- Ein eigener Planer verteilt Arbeit nach Priorität: Eingaben vor Animationsbildern, vor Darstellung, vor normalen Aufgaben, vor Leerlaufarbeit.
- Ein-/Ausgabe läuft über eine eigene Ereignisschicht auf den Mechanismen des Systems: `epoll` unter Linux, `kqueue` unter macOS und BSD, IOCP unter Windows.
- Leerlauf heißt Leerlauf: kein Abfragen in Schleifen, gebündelte Timer, keine Darstellung ohne Änderung.

---

## 4. Die Komponenten im Einzelnen

Dieser Abschnitt beschreibt das Ziel. Was davon schon umgesetzt ist, steht im Stand in [Abschnitt 11](#11-aufbau-des-repositorys).

### 4.1 Fundament

Gemeinsame Bausteine für alle anderen Teile: Arenen und typisierte Indizes, Interning von Zeichenketten (Atome für Tag- und Attributnamen), Bitmengen, kompakte Zeichenketten (Latin-1 oder UTF-16), Fehler- und Protokollierungstypen sowie ein Speicherzähler je Subsystem, damit jeder Bereich seinen Verbrauch kennt.

### 4.2 Plattformschicht

Die einzige Stelle, die mit dem Betriebssystem spricht. Sie bietet dem Rest eine kleine, einheitliche Schnittstelle: Fenster, Eingabeereignisse, Zwischenablage, GPU-Oberfläche, Ein-/Ausgabe-Ereignisse, Dateizugriff, Prozesse und Sandbox, Systemschriften, Zertifikatsspeicher, Schlüsselbund, Barrierefreiheit und Eingabemethoden.

| System | Fenster und Eingabe | Grafik |
|---|---|---|
| Linux | eigener Wayland-Client und eigener X11-Client (beides dokumentierte Protokolle über Sockets, ohne `libwayland` oder `libX11`); der X11-Client steht, unter Wayland läuft er bis zum eigenen Wayland-Client über XWayland | Vulkan, sonst CPU über gemeinsamen Speicher |
| Windows | Win32 | Direct3D 12, alternativ Vulkan, sonst CPU |
| macOS | AppKit über die Objective-C-Laufzeit | Metal, sonst CPU |
| Android | NDK-Schnittstellen (Native Activity) | Vulkan, sonst CPU |

Unter Linux sind viele „Systemfunktionen“ in Wirklichkeit Bibliotheken (fontconfig, libdbus, libwayland). Diese ersetzen wir durch eigene Implementierungen der zugrunde liegenden Protokolle und Dateiformate, zum Beispiel einen eigenen D-Bus-Client für Barrierefreiheit und Eingabemethoden.

### 4.3 Text und Unicode

- **Unicode-Tabellen** generieren wir aus der Unicode-Zeichendatenbank: Eigenschaften, Normalisierung (UAX #15), bidirektionaler Text (UAX #9), Zeilenumbruch (UAX #14), Segmentierung (UAX #29), internationalisierte Domainnamen (UTS #46).
- **Schriften:** eigener Leser für TrueType, OpenType (inklusive CFF und CFF2), variable Schriften, Farbschriften für Emoji (COLR, CBDT, sbix), WOFF und WOFF2 (mit eigenem Brotli-Dekoder). Für einige Systemschriften von macOS brauchen wir zusätzlich AAT-Tabellen.
- **Textformung (Shaping):** eigene Umsetzung von OpenType GSUB und GPOS, zuerst für lateinische, kyrillische und griechische Schrift, danach Arabisch, Hebräisch, indische Schriften, Thai und ostasiatische Schriften. Das ist einer der aufwendigsten Teile des ganzen Projekts.
- **Glyphen** sind Pfade und laufen durch die normale Grafikpipeline. Kleine Größen werden in einem Glyphenatlas zwischengespeichert.
- **Schriftauswahl und Ersatzschriften:** eigener Index der Systemschriften (Verzeichnisse und Systemschnittstellen), zwischengespeichert pro Profil.

### 4.4 Parser

| Parser | Grundlage | Hinweis |
|---|---|---|
| Zeichenkodierung | WHATWG Encoding | alle dort gelisteten Kodierungen, Tabellen aus den Daten des Standards generiert |
| URL | WHATWG URL | generierter Zustandsautomat, IDNA aus UTS #46 |
| HTML | WHATWG HTML | Tokenizer und Baumaufbau generiert, inklusive Fehlerbehandlung und Quirks-Modus |
| XML und XHTML, SVG | XML 1.0, Namespaces | eigener, nicht validierender Parser |
| CSS | CSS Syntax Level 3 | Tokenizer und Grundgrammatik generiert, Eigenschaftsgrammatiken aus einer Beschreibungsdatei |
| JSON | ECMA-404 | geteilt mit der JavaScript-Engine |
| JavaScript und WebAssembly | ECMA-262, WebAssembly | siehe [4.9](#49-javascript-und-webassembly) |

### 4.5 Dokumentspeicher (DOM)

- Spaltenspeicher wie in [2.1](#21-spaltenspeicher-statt-objektbaum), Änderungsjournal wie in [2.2](#22-ein-änderungsjournal-für-alles).
- JavaScript-Wrapper sind schlanke Verweise der Form (Realm, Knotennummer, Generation). Darum können mehrere Welten (Seite und Inhalts-Skripte von Erweiterungen) denselben Knoten mit getrennten Wrappern sehen, was die Erweiterungen in [Abschnitt 5](#5-erweiterungen-für-chrome-und-firefox) brauchen.
- Shadow DOM, Slots, Custom Elements, Ranges und Selection und `TreeWalker` setzen auf denselben Spalten auf.
- Darstellung und Skript sehen konsistente Versionen: Der Renderer liest einen Schnappschuss, während das Skript schon weiterarbeitet.

### 4.6 Stil

- Eigener Selektor-Abgleich mit Vorfiltern über die Vorfahrenkette, Kaskade mit Ebenen (`@layer`), Containerabfragen, `:has()` und Verschachtelung.
- **Stil-Interning:** Gleiche berechnete Stile werden nur einmal gespeichert. Die meisten Knoten einer Seite teilen sich wenige Stilvarianten.
- Berechnung parallel über Teilbäume, angestoßen durch das Änderungsjournal.
- Animationen und Übergänge (CSS Animations, Transitions, Web Animations) laufen über eine gemeinsame Zeitleiste. Was der Compositor allein kann, gibt die Zelle an ihn ab.

### 4.7 Layout

- **Layout als reine Funktion:** Eingabe sind ein Knoten und die Randbedingungen (verfügbare Breite und Höhe, Schreibrichtung), Ausgabe ein unveränderliches Fragment. Ergebnisse werden nach (Knoten, Randbedingungen, Stilversion) zwischengespeichert. Erneutes Layout berechnet nur, was sich wirklich geändert hat.
- **Formatierungskontexte** (Block, Inline, Flex, Grid, Tabelle, Mehrspaltig, Positionierung, Floats) sind getrennte Module mit gemeinsamer Schnittstelle. Unabhängige Kontexte werden parallel berechnet.
- **Inline-Layout:** Bidi, Zeilenumbruch, Silbentrennung, vertikale Schrift, Ruby.
- **Später:** Seitenlayout für den Druck (Paged Media).

### 4.8 Grafik

- **Szenenliste:** kompaktes Binärformat mit Pfaden, Bildern, Glyphenfolgen, Verläufen, Masken, Filtern und Mischmodi. Inhaltszellen und Oberfläche erzeugen beide Szenenlisten.
- **GPU-Renderer:** Compute-Shader zerlegen Pfade in Kacheln und rastern sie (siehe [2.7](#27-eine-grafikpipeline-für-alles)).
- **CPU-Renderer:** dieselben Algorithmen in Rust, für Tests, Rückfall und Geräte ohne geeignete GPU.
- **Eigener Shader-Übersetzer:** Wir brauchen ihn ohnehin für WebGL (GLSL ES) und WebGPU (WGSL). Er übersetzt sowohl diese Sprachen als auch unsere eigenen Shader nach SPIR-V (Vulkan), MSL (Metal) und HLSL (Direct3D). Ein Übersetzer, drei Nutzer.
- **Compositor:** Ebenen für Scrollbereiche, Videos, Canvas und Animationen. Auch Frames aus anderen Inhaltszellen werden hier zu einem Bild zusammengesetzt.
- **Canvas 2D, WebGL, WebGPU** bauen auf derselben GPU-Abstraktion auf.

### 4.9 JavaScript und WebAssembly

Ziel ist vollständiges ECMAScript in der jeweils aktuellen Ausgabe einschließlich der Internationalisierungs-API (ECMA-402), gemessen mit Test262.

**Ausführungsstufen**

1. **Parser:** erzeugt einen kompakten Syntaxbaum in einer Arena. Funktionskörper werden zunächst nur vorgeprüft und erst beim ersten Aufruf vollständig übersetzt. Die „early errors“, die die Spezifikation vorschreibt, werden trotzdem sofort gemeldet.
2. **Bytecode:** registerbasiert mit Akkumulator und kompakt. Übersetzter Bytecode wird pro Skript-Hash auf der Festplatte zwischengespeichert, sodass wiederholte Besuche ohne erneutes Parsen auskommen.
3. **Interpreter:** läuft überall und ist die Grundlage für alle Plattformen, auch für die, auf denen kein JIT erlaubt ist. Aufrufe zwischen JavaScript-Funktionen laufen über einen eigenen Stapel von Rahmen statt über den Rust-Stapel, und Aufrufe in Endposition sind echte Endaufrufe (ECMA-262, Abschnitt 15.10).
4. **Basis-JIT:** setzt vorgefertigte Maschinencode-Bausteine je Bytecode zusammen, zuerst für x86_64 und AArch64.
5. **Optimierender JIT (später):** eigene SSA-Zwischendarstellung, die JavaScript und WebAssembly gemeinsam nutzen. Ein Backend, zwei Sprachen.

Ein JIT-freier Modus bleibt immer verfügbar, als Sicherheitsoption und für eingeschränkte Plattformen.

**Laufzeit**

- **Objektmodell:** Objektgestalten (Shapes) mit Inline-Caches, dichte und dünn besetzte Array-Elemente. Das ist bewährte Technik, die wir eigenständig umsetzen.
- **Werte:** 64-Bit-Darstellung mit eingebetteten Zahlen (NaN-Boxing, ganze Zahlen im Bereich von `i32` mit eigenem Kennzeichen). In Objekten gespeicherte Verweise sind innerhalb einer Region 32-Bit-Abstände (Regionen bis 4 GiB), das spart Speicher.
- **Speicher:** Regionen wie in [2.4](#24-speicherregionen-pro-dokument), präziser und verschiebender Garbage Collector mit Generationen. Die erste Ausbaustufe sammelt kopierend nach Cheney und kommt wie der Interpreter ohne `unsafe` aus, das braucht erst der JIT. Verweise aus Rust-Code laufen über typisierte, bereichsgebundene Handles, sodass der Collector jeden Verweis kennt.
- **Zeichenketten:** Latin-1 oder UTF-16 je nach Inhalt, Ropes für zusammengesetzte Ketten, Interning für Eigenschaftsnamen.
- **Reguläre Ausdrücke:** eigene Engine mit der vollen JavaScript-Semantik (Rückverweise, Lookbehind, Unicode-Sets) und einem Automaten-Schnellpfad für einfache Muster.
- **Intl:** Daten aus CLDR und der IANA-Zeitzonendatenbank, mit eigenem Generator in kompakte Tabellen übersetzt.
- **Eingebaute Objekte** kommen aus dem geteilten Abbild ([2.5](#25-geteilte-unveränderliche-abbilder)).

**WebAssembly**

Eigener Validierer, Interpreter und Basis-Übersetzer. Später teilt es sich das optimierende Backend mit JavaScript. Zuerst kommt der Kernstandard, danach Erweiterungen wie Threads, SIMD, Ausnahmen und Garbage Collection.

### 4.10 Web-APIs und Bindings

- Die Schnittstellen der Webplattform sind in den Spezifikationen in WebIDL beschrieben. Ein eigenes Werkzeug zieht die IDL-Blöcke aus den Spezifikationen, und ein eigener Generator erzeugt daraus die Bindings: Typumwandlung, Argumentprüfung, Ausnahmen, Prototypen. Rust-Code implementiert nur noch die eigentliche Logik.
- **Bisher vorhanden:** ein eigener WebIDL-Tokenizer, ein iterativer LL(1)-Syntaxparser und ein quellenübergreifender Namensindex mit Arbeits-/Speichergrenzen. Der Index verbindet Partials, benannte Typverweise, Includes und Vererbung und erkennt Namenskollisionen sowie Vererbungs-/Typedef-Zyklen. Die Tabellen erzeugt der eigene `datagen` aus der normativen Grammatik; Corpus- und Fuzztests prüfen die Bausteine. Vollständige Typ-/Memberprüfung, Extended-Attribute-Auswertung und Binding-Generator folgen; API, Grenzen und Prüfpunkte stehen in [m2-webidl-index.md](m2-webidl-index.md) und [m2-webidl-parser.md](m2-webidl-parser.md).
- Die Ereignisschleife, Timer, Microtasks und `requestAnimationFrame` folgen exakt dem Verarbeitungsmodell der HTML-Spezifikation.
- **Worker, Service Worker und Worklets** sind eigene Agents mit eigener Region.

### 4.11 Netzwerk und Kryptografie

- **Protokolle:** HTTP/1.1 (RFC 9112), HTTP/2 (RFC 9113 mit HPACK), HTTP/3 (RFC 9114 über QUIC nach RFC 9000, mit QPACK), WebSocket, später WebTransport.
- **Fetch:** Der Fetch-Standard wird als ein einziger Zustandsautomat umgesetzt, den Seiten, Worker, Service Worker und Erweiterungen gemeinsam nutzen. Die Eingriffspunkte für Erweiterungen (`webRequest`, `declarativeNetRequest`) liegen an fest definierten Stellen dieses Automaten.
- **Weitere Bausteine:** HTTP-Cache, Cookies (inklusive Partitionierung), HSTS, CORS, CSP, gemischte Inhalte, Proxy-Unterstützung. PAC-Dateien sind JavaScript und laufen in unserer eigenen Engine.
- **DNS:** eigener Resolver, der die Systemkonfiguration liest, plus DNS über HTTPS (RFC 8484).
- **Kompression:** eigene Dekoder für gzip und Deflate, Brotli und zstd.
- **TLS:** eigene Umsetzung von TLS 1.3 (RFC 8446) und TLS 1.2 für ältere Server, Zertifikatsprüfung mit eigenem ASN.1-, DER- und X.509-Code. Die Wurzelzertifikate kommen aus dem Systemspeicher oder aus einer mitgelieferten Liste.
- **Kryptografie:** AES-GCM, ChaCha20-Poly1305, SHA-2, HMAC, HKDF, X25519, ECDSA P-256 und P-384, RSA, Ed25519 sowie die hybride Post-Quanten-Schlüsselvereinbarung X25519MLKEM768, die aktuelle Browser standardmäßig anbieten. Dieselben Bausteine bedienen die Web Crypto API.

**Wichtig zur Kryptografie:** Wir schreiben sie selbst (siehe [1.1](#11-was-komplett-eigen-hier-bedeutet)). Selbst geschriebene Kryptografie ist aber ein ernstes Sicherheitsrisiko. Schon kleine Fehler, etwa Laufzeiten, die von geheimen Daten abhängen, können Schlüssel verraten. Deshalb gilt:

- Die Kryptografie liegt hinter einer schmalen Anbieter-Schnittstelle, damit sie bei Bedarf austauschbar ist.
- Jede Primitive wird gegen offizielle Testvektoren geprüft (NIST, RFCs, Project Wycheproof), gefuzzt und auf konstante Laufzeit untersucht.
- Vor einer öffentlichen Version prüft ein externes Audit diesen Teil.

### 4.12 Speicherung

Ein eigener, absturzsicherer Speicherkern (Schreib-Log plus B-Baum) trägt alles, was dauerhaft gespeichert wird: Cookies, `localStorage`, IndexedDB, Cache API, das Origin Private File System, Verlauf, Lesezeichen, Einstellungen und Erweiterungsspeicher. Passwörter werden zusätzlich mit dem Schlüsselbund des Systems verschlüsselt.

### 4.13 Bilder, Audio und Video

- **Bilder:** PNG, APNG, JPEG (Baseline und progressiv), GIF, WebP (VP8 und VP8L), AVIF (AV1), BMP, ICO, SVG (über die eigene Engine). Bilder werden erst beim Zeichnen in der angezeigten Größe dekodiert und außerhalb des Sichtbereichs wieder verworfen.
- **Audio und Video:** HTML-Medienelemente, Media Source Extensions, Web Audio, später WebRTC. Lizenzfreie Codecs implementieren wir selbst: VP8, VP9, AV1, Opus, Vorbis, FLAC, MP3.
- **Patentbelastete Codecs** (H.264, AAC, HEVC) brauchen sehr viele Webseiten. Hier ist zu entscheiden, ob wir sie über die Medienschnittstellen des Betriebssystems nutzen, die zugleich Hardwaredekodierung bieten. Die Patentlage muss vor einer Veröffentlichung geklärt werden.
- **DRM:** Die EME-Schnittstelle (*Encrypted Media Extensions*), den CDM-Host und die Anbindung an das eigene Mediensystem entwickeln wir selbst. Zusätzlich ist die optionale Nutzung eines proprietären Widevine-CDM vorgesehen. Ein eigenes CDM wäre technisch möglich, benötigt für Inhalte fremder Anbieter aber deren Zusammenarbeit; es ersetzt Widevine bei bestehenden Diensten nicht automatisch.

**Festgelegte Grenzen für die optionale Widevine-Anbindung:**

- **Freiwillig und bedarfsweise:** Widevine ist standardmäßig deaktiviert. Bezug, Installation und Aktivierung erfolgen nur nach informierter Zustimmung. Der CDM-Prozess startet erst für erlaubte DRM-Nutzung; DRM-freie Wiedergabe benötigt ihn nicht. Modul und lokale DRM-Daten müssen entfernbar sein.
- **Eigene Sicherheitsgrenze:** Der selbst geschriebene CDM-Host lädt das Modul in einem eigenen eingeschränkten Prozess, mit schmaler, vollständig geprüfter IPC und Ressourcenlimits. Dateien, Netzwerk und gegebenenfalls geschützte Ausgabepfade werden nur über ausdrücklich freigegebene Schnittstellen vermittelt. Fehlt eine sichere Anbindung, bleibt diese Plattform-/CDM-Kombination nicht unterstützt.
- **Datenschutz:** Zugriffe werden pro Website freigegeben. Dauerhafte DRM-Sitzungen und unterscheidende Kennungen (*distinctive identifiers*) benötigen eine gesonderte Freigabe, soweit sie unterstützt werden. Lokale DRM-Daten werden nach Profil und Herkunft und, wo anwendbar, oberster Website getrennt und sind löschbar. Im Privatmodus wird kein dauerhafter DRM-Zustand angelegt. Verlangt ein Dienst darüber hinausgehende Rechte, kann seine Nutzung eingeschränkt sein.
- **Vertrauenswürdiger Bezug und Updates:** Nur ausdrücklich zugelassene CDM-Versionen aus berechtigten, geprüften Bezugswegen laden; Herkunft und Integrität auch bei Updates prüfen. Eine Website darf keine beliebige native Bibliothek als CDM vorgeben. Lizenzierung und eventuelle Weitergabe des Moduls sind gesondert zu klären.
- **Stand und Grenzen:** Die Anbindung ist noch nicht implementiert. Verfügbarkeit, Anbietervereinbarungen, Zertifizierungen, unterstützte Plattformen und CDM-Schnittstellen, Codecs sowie Ausgabeschutz sind gesondert zu prüfen. Unterstützung bestimmter Streamingdienste und HD-/4K-Wiedergabe sind damit nicht garantiert.

### 4.14 Sicherheit und Sandbox

Das Bedrohungsmodell und alle Schutzschichten im Überblick stehen in [Abschnitt 8](#8-sicherheit). Hier stehen die technischen Bausteine.

- **Rust schützt vor Speicherfehlern**, solange kein `unsafe` im Spiel ist. Darum steht in allen Crates außer wenigen ausgewiesenen (Plattformschicht, JIT, Garbage Collector, IPC) `#![forbid(unsafe_code)]`. Die Ausnahmen werden besonders geprüft und gefuzzt.
- **Sandbox je System:** unter Linux seccomp-bpf, Namespaces und Landlock, unter Windows AppContainer, eingeschränkte Tokens und Job-Objekte, unter macOS die Seatbelt-Sandbox.
- **Fähigkeiten statt Umgebungsrechten:** Eine Komponente bekommt nur Handles auf das, was sie braucht. Eine Inhaltszelle öffnet keine Dateien selbst, sie bekommt sie vom Steuerprozess, wenn der Nutzer sie ausgewählt hat.
- **JIT-Speicher** ist nie gleichzeitig beschreibbar und ausführbar (W^X).
- **Updates** sind signiert, und der Update-Mechanismus ist selbst geschrieben.

### 4.15 Barrierefreiheit und Eingabemethoden

- Der Barrierefreiheitsbaum entsteht aus Dokumentspeicher und Änderungsjournal und wird über die Systemschnittstellen bereitgestellt: UI Automation unter Windows, NSAccessibility unter macOS, AT-SPI über D-Bus unter Linux.
- Eingabemethoden für Chinesisch, Japanisch, Koreanisch und andere Sprachen laufen über TSF unter Windows, `NSTextInputClient` unter macOS sowie IBus oder Fcitx über D-Bus beziehungsweise das Wayland-Protokoll `text-input` unter Linux.
- Beides ist viel Arbeit, darf aber nicht bis zum Schluss warten. Ein Browser ohne Barrierefreiheit ist für viele Menschen nicht benutzbar.

### 4.16 Entwicklerwerkzeuge und Automatisierung

- Eigene Entwicklerwerkzeuge, gebaut mit der eigenen Engine.
- **Protokolle:** WebDriver (klassisch) und WebDriver BiDi, außerdem eine Teilmenge des Chrome DevTools Protocol. Damit lassen sich Werkzeuge wie Selenium und Puppeteer anbinden, und die vollständige Ausführung der web-platform-tests wird möglich.
- **Kopfloser Modus** (ohne Fenster) für Tests und Automatisierung.

---

## 5. Erweiterungen für Chrome und Firefox

### 5.1 Ziel

Erweiterungen aus dem Chrome Web Store und von addons.mozilla.org laufen ohne Änderung. Das betrifft Manifest V3 (Chrome und Firefox) und Manifest V2, das Firefox weiter unterstützt.

### 5.2 Aufbau

```mermaid
flowchart TB
  D[API-Beschreibungen] --> G[Generator]
  G --> C["chrome.* (Callbacks und Promises)"]
  G --> B["browser.* (Promises)"]
  C --> K[Erweiterungskern]
  B --> K
  K --> F[Fähigkeiten und Berechtigungen]
  K --> N[Fetch-Eingriffspunkte]
  K --> T[Tabs, Fenster, Oberfläche]
  K --> S[Speicherkern]
```

- **Ein internes Modell:** Eine Erweiterung besteht aus Manifest, Dateien, Fähigkeiten (aus `permissions` und `host_permissions`) und Kontexten: Hintergrund, Inhalts-Skripte, Erweiterungsseiten (Popup, Optionen, Seitenleiste) und Offscreen-Dokumente.
- **Eine Beschreibung pro API-Namensraum,** erstellt aus der öffentlichen Dokumentation von Chrome und MDN. Abweichungen zwischen den Browsern sind dort ausdrücklich markiert (nur Chrome, nur Firefox, unterschiedliches Verhalten). Der Generator erzeugt beide Gesichter, Argumentprüfung, Dokumentation und Testgerüste.
- **Hintergrund:** Chrome nutzt in Manifest V3 Service Worker, Firefox Hintergrundskripte und Event-Pages. Beides bilden wir auf denselben ereignisgesteuerten Kontext ab, der schlafen gelegt und bei Bedarf geweckt wird. Das spart Speicher.
- **Inhalts-Skripte** laufen in isolierten Welten: ein eigenes globales Objekt pro Erweiterung auf demselben DOM, möglich durch die schlanken Wrapper aus [4.5](#45-dokumentspeicher-dom). Firefox-Besonderheiten wie `wrappedJSObject`, `cloneInto` und `exportFunction` werden ebenfalls unterstützt.
- **Netzwerkeingriffe:** `declarativeNetRequest` wird in einen kompakten Regelautomaten übersetzt, schnell und speichersparend, was Werbeblockern zugutekommt. Blockierendes `webRequest` steht wie in Firefox in allen Manifest-Versionen zur Verfügung, auch für Chrome-Erweiterungen.
- **Pakete:** CRX3 (ZIP mit Protobuf-Kopf und Signaturen) und XPI (ZIP mit Signatur), gelesen mit eigenem ZIP-, Deflate- und Protobuf-Code.
- **Updates** über `update_url` (Chrome) beziehungsweise `browser_specific_settings.gecko.update_url` (Firefox).
- **Themes:** Chrome-Themes (`theme` im Manifest) und Firefox-Themes (`theme`-Schlüssel und Theme-API) werden auf die Design-Token aus [Abschnitt 6](#6-modifizierbarkeit-und-skins) abgebildet.
- **Native Messaging** startet das native Hilfsprogramm über die Prozessschnittstelle des Systems, wie in Chrome und Firefox.

### 5.3 Reihenfolge der APIs

1. `runtime`, `storage`, `i18n`, `permissions`, Inhalts-Skripte und `scripting`
2. `tabs`, `windows`, `action` und `browserAction`, `contextMenus`, `alarms`, `notifications`, `commands`
3. `declarativeNetRequest`, `webRequest`, `webNavigation`, `cookies`
4. `downloads`, `history`, `bookmarks`, `sidePanel` und `sidebarAction`, `offscreen`, `identity`
5. `devtools.*`, sobald die eigenen Entwicklerwerkzeuge stehen

### 5.4 Kompatibilität prüfen

Eine gemeinsame offizielle Testsammlung wie die WPT gibt es für Erweiterungs-APIs nach unserem Kenntnisstand nicht. Die W3C WebExtensions Community Group arbeitet an einer gemeinsamen Grundlage, der wir folgen. Wir bauen zusätzlich eigene API-Tests aus der Dokumentation und eine Liste realer Zielerweiterungen als Abnahmetest, zum Beispiel uBlock Origin, Dark Reader und Bitwarden.

Offen ist, wie die Installation direkt aus den Stores aussieht, denn beide Stores haben eigene Nutzungsbedingungen (siehe [Abschnitt 14](#14-offene-entscheidungen)).

---

## 6. Modifizierbarkeit und Skins

Der Browser soll sich auf vier Ebenen anpassen lassen, von einfach bis tief.

1. **Skins:** Die Oberfläche wird von der eigenen Engine gezeichnet, beschrieben in einer deklarativen Teilmenge von HTML und CSS. Ein Skin ist eine Datei mit Design-Token (Farben, Abstände, Schriften, Radien) plus beliebigem CSS für die Oberfläche. Skins lassen sich im laufenden Betrieb wechseln. Chrome- und Firefox-Themes werden automatisch übersetzt.
2. **Anordnung:** Leisten, Knöpfe und Seitenleisten sind frei anordenbar. Die Anordnung steht als Klartext in einer Datei und lässt sich teilen.
3. **Erweiterungen:** wie in [Abschnitt 5](#5-erweiterungen-für-chrome-und-firefox).
4. **Mods:** WebAssembly-Module mit einer eigenen Fähigkeiten-API für Oberfläche, Tabs, Tastenkürzel, eigene Panels und Netzwerk-Hooks. Mods sind abgeschottet, in jeder Sprache schreibbar, die nach WebAssembly übersetzt, und ohne Neustart nachladbar.

Darunter liegt der offene Quelltext mit klaren Crate-Grenzen. Feature-Schalter ermöglichen schlanke Varianten, etwa ohne WebGPU oder WebRTC für schwache Geräte.

Die Oberfläche selbst führt kein JavaScript aus. Ihre Logik ist in Rust geschrieben, eigene Logik kommt über Mods. Das spart einen ganzen JavaScript-Kontext im Steuerprozess. Popups und Optionsseiten von Erweiterungen sind dagegen normale Webseiten und laufen in Erweiterungszellen.

---

## 7. Ressourcenschonung

**Mechanismen**

1. Kompakte Datenstrukturen: 32-Bit-Indizes, Interning, Stil-Teilung ([2.1](#21-spaltenspeicher-statt-objektbaum), [4.6](#46-stil)).
2. Geteilte Abbilder für alles Unveränderliche ([2.5](#25-geteilte-unveränderliche-abbilder)).
3. Alles verzögert: Funktionskörper, eingebaute Objekte, Bilddekodierung, Schriftdaten.
4. Speicherbudgets mit Druckstufen: Unter Speicherdruck werden zuerst Caches geleert (Bilder, Glyphen, Bytecode), danach Hintergrund-Tabs eingefroren und ausgelagert ([2.6](#26-einfrieren-statt-verwerfen)).
5. Energie: keine Abfrageschleifen, gebündelte Timer, stark gedrosselte Hintergrund-Tabs, Darstellung nur bei Änderung, schlafender GPU-Prozess.
6. Zusammenlegbare Dienstprozesse auf schwachen Geräten ([3.1](#31-prozesse-und-zellen)).
7. Messen statt hoffen: Speicherzähler je Subsystem und Messläufe in der CI, damit Rückschritte sofort auffallen.

**Erste Zielwerte.** Das sind Ziele, keine Messungen. Wir überprüfen sie ab Meilenstein M5 an echter Hardware und passen sie an. Die Spalte „Stand“ zeigt, was sich am 11.11.2026 schon messen lässt.

| Kennzahl | Zielwert | Stand |
|---|---|---|
| Kaltstart bis bedienbares Fenster | unter 300 ms | noch nicht gemessen |
| Zusätzlicher Arbeitsspeicher für einen leeren Tab | unter 10 MB | noch nicht messbar, es gibt keine Tabs |
| Aufwachvorgänge eines Hintergrund-Tabs im Leerlauf | höchstens einer pro Minute | noch nicht messbar, es gibt keine Tabs |
| Größe des Programms | unter 40 MB | 3,9 MB unter Linux auf x86_64 (`cargo build --release -p meinbrowser`, rustc 1.97.0). Das Programm bindet JavaScript-Engine, Netzwerk und Oberfläche noch nicht ein, die Zahl sagt also wenig über den fertigen Browser. |

Von den Mechanismen fehlt der siebte noch ganz: Es gibt weder Speicherzähler je Subsystem noch Messläufe in der CI.

---

## 8. Sicherheit

Ein Browser führt ständig fremden Code aus und ist deshalb eines der wichtigsten Angriffsziele auf jedem Rechner. Viele Entscheidungen weiter oben sind darum zugleich Sicherheitsentscheidungen. Dieser Abschnitt bündelt sie, nennt das Bedrohungsmodell und ergänzt, was in den anderen Abschnitten fehlt.

### 8.1 Bedrohungsmodell

| Angreifer | Ziel | Wichtigster Schutz |
|---|---|---|
| Bösartige Webseite | Daten anderer Seiten lesen, aus der Sandbox ausbrechen, Code auf dem Rechner ausführen | Rust, Zellen pro Website, Sandbox, Fuzzing |
| Angreifer im Netzwerk | Verbindungen mitlesen oder verändern | TLS, Zertifikatsprüfung, HSTS, HTTPS zuerst |
| Bösartige oder übernommene Erweiterung | mehr Daten abgreifen, als ihre Berechtigungen erlauben | Fähigkeiten, Erweiterungszellen, Berechtigungsdialoge, Signaturen |
| Bösartiger Mod oder Skin | wie bei Erweiterungen | WebAssembly-Sandbox mit Fähigkeiten, Skins enthalten keinen Code |
| Täuschung des Nutzers | Phishing, gefälschte Adresszeile, gefälschte Dialoge | Regeln für die Oberfläche, Anzeige internationaler Domainnamen, Schutzlisten |
| Lieferkette | Schadcode über Abhängigkeiten, Build oder Updates | keine fremden Crates, abgesicherte CI, signierte Updates |

**Grundannahme:** Irgendwann wird eine Inhaltszelle trotz allem übernommen. Die Architektur muss dafür sorgen, dass eine übernommene Zelle nicht mehr kann als die Seite, die sie anzeigt. Sie darf weder an Daten anderer Websites noch an Dateien oder Netzwerkzugänge kommen, die der Steuerprozess ihr nicht gegeben hat.

### 8.2 Schutzschichten

Jede Schicht soll auch dann noch schützen, wenn die davor versagt hat.

1. **Speichersicherheit:** Rust, `unsafe` nur in drei Crates (`mb-platform`, `mb-ipc`, `mb-js`), jeder `unsafe`-Block mit Begründung. Die Workspace-Lints erzwingen das schon heute ([4.14](#414-sicherheit-und-sandbox)).
2. **Isolation:** eine Zelle pro Website, auch für Frames fremder Websites ([3.1](#31-prozesse-und-zellen)). Dazu kommen die Gegenmaßnahmen gegen Spectre, die die Webplattform vorsieht: Antworten fremder Herkunft, die eine Seite nicht lesen darf, gelangen gar nicht erst in deren Zelle (Opaque Response Blocking). `SharedArrayBuffer` und genaue Zeitmessung gibt es nur in herkunftsisolierten Seiten (COOP und COEP). Zeitstempel werden so vergröbert, wie es die High-Resolution-Time-Spezifikation vorsieht.
3. **Sandbox:** Die Mechanismen der Betriebssysteme aus [4.14](#414-sicherheit-und-sandbox). Eine Inhaltszelle hat keinen direkten Zugriff auf Dateien, Netz oder GPU.
4. **Misstrauische Prozessgrenzen:** Jede Nachricht aus einer Inhalts- oder Erweiterungszelle gilt als möglicherweise bösartig. Der Empfänger prüft sie vollständig, mit Prüfcode, der aus derselben Schnittstellenbeschreibung generiert wird wie die Nachrichten selbst. Die IPC ist der Hauptweg, um aus einer Sandbox auszubrechen, und wird deshalb besonders gefuzzt.
5. **GPU:** Grafiktreiber sind eine große Angriffsfläche. Darum laufen sie in einem eigenen Prozess, und Shader aus WebGL und WebGPU prüft unser Shader-Übersetzer vollständig, bevor ein Treiber sie sieht.
6. **JIT:** Fehler in JIT-Compilern sind in Browsern eine häufige Ursache schwerer Lücken. Gegenmaßnahmen: W^X, ein JIT-freier Modus als Einstellung, und differentielles Fuzzing, bei dem Interpreter und JIT dieselben Programme ausführen und gleiche Ergebnisse liefern müssen.
7. **Netzwerk:** TLS 1.3 bevorzugt, vollständige Zertifikatsprüfung, HSTS mit Preload-Liste, HTTPS zuerst (unverschlüsselte Verbindungen nur nach Warnung), Blockieren gemischter Inhalte, Prüfung von Certificate Transparency und von widerrufenen Zertifikaten (Verfahren noch offen, siehe [Abschnitt 14](#14-offene-entscheidungen)).
8. **Sicherheitsmechanismen der Webplattform,** exakt nach Spezifikation und mit den WPT getestet: Same-Origin-Policy, CORS, Content Security Policy, Trusted Types, Sandbox-Frames, Permissions Policy, Referrer Policy, SameSite-Cookies und sichere Kontexte für mächtige Funktionen.
9. **Berechtigungen:** Kamera, Mikrofon, Standort, Benachrichtigungen und Ähnliches nur mit Zustimmung, nur in sicheren Kontexten, pro Website widerrufbar.
10. **Oberfläche:** Die Adresszeile zeigt die Herkunft eindeutig. Internationale Domainnamen, die mit verwechselbaren Zeichen arbeiten, werden nach den Regeln von UTS #39 (Unicode Security Mechanisms) als Punycode angezeigt. Die Oberfläche des Browsers liegt immer außerhalb des Bereichs, in den eine Seite zeichnen kann. Vollbild und Mauszeigersperre werden deutlich angekündigt.
11. **Erweiterungen, Mods und Skins:** Berechtigungen werden vor der Installation angezeigt, Zugriff auf Websites lässt sich auch erst bei Bedarf erteilen. Erweiterungen laufen in eigenen Zellen, Inhalts-Skripte in isolierten Welten. Mods laufen in der WebAssembly-Sandbox und bekommen nur die Fähigkeiten, die der Nutzer freigibt. Skins sind reine Daten ohne Code und dürfen nur lokale Ressourcen laden, damit sie weder nachladen noch mitlesen können.
12. **Datenschutz:** Vorschlag: Speicher und Cookies werden standardmäßig nach der obersten Website getrennt (Partitionierung), damit eingebettete Inhalte Nutzer nicht über Websites hinweg verfolgen können.
13. **Lieferkette:** keine fremden Crates (der Abhängigkeitswächter läuft bereits in der CI; die Ausnahme für den Fuzzing-Workspace `fuzz/` steht in Punkt 14), keine fremden GitHub Actions (die CI holt den Quelltext selbst mit git; braucht es doch einmal eine Action, wird sie auf einen festen Commit-Hash festgelegt), reproduzierbare Builds als Ziel, Updates signiert mit einem Schlüssel, der nicht auf dem Build-Server liegt.
14. **Fehler finden, bevor andere es tun:** Fuzzing für jeden Parser ab dem ersten Tag (HTML, CSS, URL, JavaScript, Bilder, Schriften, Kompression, TLS, IPC-Nachrichten), nächtlich in der CI. Dafür gibt es `cargo-fuzz` mit libFuzzer in einem eigenen Cargo-Workspace unter `fuzz/`, mit eigenem Lockfile. `libfuzzer-sys` und die Crates, die es mitbringt, sind dort als einzige Ausnahme von „keine fremden Crates“ zugelassen; kein Crate des Browsers hängt von `fuzz/` ab, und weitere fremde Crates deckt die Ausnahme nicht; das prüft `tools/deps-guard` an `fuzz/Cargo.lock` ([Abschnitt 14](#14-offene-entscheidungen)). Elf Ziele prüfen HTML, CSS, URL, XML, PNG, GIF, Deflate/zlib, Schriften, JSON, Encoding und Unicode, das zwölfte (`page`) eine ganze Seite mit Stylesheets und Bildern durch Kaskade, Layout und Zeichnen, das dreizehnte (`js`) Lexer und Parser der JavaScript-Engine samt dem Parser für die Muster regulärer Ausdrücke und das vierzehnte (`js-runtime`) Operationsfolgen auf den vorhandenen Laufzeitbausteinen mit gewöhnlichen Objekten, Arrays, primitiven numerischen Umwandlungen, Umwandlungen in Text und Eigenschaftsschlüssel, Handles und Garbage Collection. Das Laufzeitziel vergleicht eigene Ausführungen bei unterschiedlichen Sammelzeitpunkten. Das fünfzehnte Ziel (`js-execute`) prüft die inzwischen vorhandene Bytecode-/Interpreterpipeline für Scripts und gewöhnliche Funktionen: Beliebige Quellen und erreichbare Funktionsrümpfe werden übersetzt und ihr Bytecode geprüft; bekannte Script- und Funktionsvorlagen besitzen unabhängige Erwartungen, auch für Closures, `arguments`-Objekte sowie Vorgabe- und Restparameter. Zusätzlich laufen beliebige Quellen unter einer kleinen Hostquote, jeweils mit normalen und erzwungenen GC-Zeitpunkten. Das sechzehnte Ziel (`webidl`) prüft WebIDL-Tokenizer, budgetierten Syntaxparser und Namensindex einschließlich Quelltextabdeckung, UTF-8-Grenzen, Fortschritt, Baumstruktur und quellenübergreifender Verknüpfung; ein eigenes Graphmodell liefert unabhängige Erwartungen für Partials, Typverweise, Includes und Vererbungszyklen. TLS und IPC erhalten Ziele mit ihren Implementierungen, weitere Teile der JavaScript-Engine mit ihren Stufen. Dauerhafte private Aufbewahrung von CI-Rohprotokollen, Fehlerdateien und neuen Korpora ist noch einzurichten. Ziele, Grenzen und der Umgang mit Funden stehen in [docs/fuzzing.md](fuzzing.md).
15. **Begrenzte Arbeit:** Was eine Seite an Rechenarbeit auslöst, bezahlt sie aus einem Budget mit festen Messstellen; ist es verbraucht oder bricht die Oberfläche den Auftrag ab, endet er mit einem Fehler statt weiterzurechnen. Bisher gilt das für Kaskaden-Vorbereitung, Stil und Layout (`mb_base::Budget`) sowie für die Ausführung von JavaScript: Ohne Hostquote gelten 1.000.000 Schritte pro äußerem Aufruf, explizite Quoten und Abbruchsignale bleiben über direkte und native Wiedereintritte geteilt. Ein Hostabbruch ist keine JavaScript-Ausnahme. Parser, Compiler und native Einzeloperationen sind dadurch noch nicht vollständig begrenzt; API, Messstellen und Grenzen stehen in [m2-ausfuehrungsbudget.md](m2-ausfuehrungsbudget.md). Ebenso hat eine Seite ein Bytekontingent für Dokument, Stylesheets, Bilddateien und dekodierte Bilddaten (`mb_web::PAGE_BYTES`). Jeder Abruf erhält das Restbudget als Leselimit; bei null beginnt kein weiterer Abruf. Von lokalen Dateien liest sie nur normale Dateien, keine Geräte oder Pipes, mit der kleineren Grenze aus Restbudget und Dateilimit (`mb_web::read_file_within`). Das Kontingent ist keine Grenze für den gesamten RAM-Verbrauch: zusätzliche Parserdatenstrukturen und feste Tabellen der Dekoder sind davon nicht erfasst. Wiederholte Arbeit wie verschachtelte Selektoren wird innerhalb eines Auftrags gemerkt, damit das Budget nicht schon bei gewöhnlichen Seiten ausgeht.
16. **Umgang mit gemeldeten Lücken:** eine `SECURITY.md` mit vertraulichem Meldeweg (die private Schwachstellenmeldung von GitHub), Sicherheitshinweise zu jeder behobenen Lücke und schnelle Sicherheitsupdates. Vor Version 1.0 prüft ein externes Audit Kryptografie, Sandbox, IPC und JIT.

### 8.3 Sicherheit in jedem Meilenstein

| Meilenstein | Sicherheitsaufgaben |
|---|---|
| M0 | `SECURITY.md` und Meldeweg (erledigt), Abhängigkeitswächter (erledigt), keine fremden GitHub Actions (erledigt) |
| M1 | Arbeits- und Ladebudgets vorhanden; elf Parser-Fuzzing-Ziele, ein Ziel für die ganze Seitenpipeline und nächtlicher CI-Workflow eingerichtet. Dauerhafte private Aufbewahrung von CI-Funden noch offen. |
| M2 | Belastungstests für den Garbage Collector und differentielle Tests der JavaScript-Engine; `js-runtime` prüft vorhandene Laufzeitbausteine bei unterschiedlichen Sammelzeitpunkten. `js-execute` prüft Bytecode, bekannte Script-/Funktionsvorlagen und beliebige Quellen unter einer kleinen Ausführungsquote. Der Interpreter besitzt Standard- und Hostquoten mit externem Abbruchsignal; Parser-/Compiler- und native Einzelkosten bleiben weiter zu begrenzen. Die Vergleiche ersetzen keine Konformitätstests und schließen M2 nicht ab. |
| M3 | TLS gegen Testvektoren, HSTS-Preload, Certificate Transparency und Widerrufsprüfung |
| M5 | Zellen pro Website und Sandbox auf allen Stufe-1-Systemen, Prüfung aller IPC-Nachrichten, Berechtigungsdialoge, Regeln für die Adresszeile |
| M6 | Signaturprüfung und Berechtigungsanzeige für Erweiterungen |
| M8 | Härtung des JIT, differentielles Fuzzing Interpreter gegen JIT |
| M10 | externes Audit, signierte Updates, Ablauf für Sicherheitsupdates |

**Wichtig:** Vor M5 fehlen Isolation und Sandbox. Bis dahin ist der Browser ausdrücklich nur für Entwicklung und Tests gedacht, nicht für beliebige Websites.

### 8.4 Restrisiken, ehrlich benannt

- **Junge Codebasis:** Selbst Browser mit großen Sicherheitsteams schließen jedes Jahr viele Sicherheitslücken. Eine neue Engine wird anfangs mehr Fehler haben. Für Onlinebanking und vergleichbar heikle Dinge eignet sie sich erst nach dem externen Audit.
- **Eigene Kryptografie:** bewusst entschieden, mit den Gegenmaßnahmen aus [4.11](#411-netzwerk-und-kryptografie), aber ein bleibendes Risiko.
- **Optionales Widevine:** proprietärer Binärcode ist nicht im gleichen Umfang prüfbar wie der eigene Browsercode. Prozessisolation und beschränkte Rechte begrenzen seine Wirkung; seine interne Sicherheit und sein Umgang mit Daten bleiben ein zusätzliches Vertrauen in den Anbieter.
- **Außerhalb unserer Kontrolle:** Betriebssystem, Grafiktreiber und Hardware. Seitenkanäle wie Spectre lassen sich nur eindämmen, nicht ganz beseitigen.

---

## 9. Plattformen

| Stufe | Systeme | Bedeutung |
|---|---|---|
| 1 | Linux (Wayland und X11), Windows 10 und 11, macOS; jeweils x86_64 und AArch64 | Jeder Pull Request wird darauf gebaut und getestet. |
| 2 | Android, FreeBSD | Regelmäßige Builds, Fehler werden zeitnah behoben. |
| 3 | iOS und iPadOS, weitere BSDs, RISC-V | Best-Effort. |

Zu iOS: Apple erlaubt eigene Browser-Engines nur in einzelnen Regionen (in der EU seit iOS 17.4) und nur mit besonderer Berechtigung. Wo das nicht gilt, muss ein Browser auf iOS WebKit verwenden, was unserem Grundsatz widerspricht.

Auf Plattformen ohne JIT-Erlaubnis läuft JavaScript über den Interpreter. Die Engine funktioniert dort vollständig, nur langsamer.

---

## 10. Kompatibilität messen

| Testsammlung | Prüft | Ab |
|---|---|---|
| html5lib-tests | HTML-Tokenizer und Baumaufbau | M1 |
| URL-Testdaten der WPT (`url/`, als JSON ohne JavaScript ausführbar) | URL-Parser | M1 |
| Testdateien von Unicode (`NormalizationTest.txt`, `LineBreakTest.txt` und `IdnaTestV2.txt` aus unicodetools) | Normalisierung, Zeilenumbruch nach UAX #14, IDNA nach UTS #46 | M1 |
| css-parsing-tests (nur die Dateien, die zum aktuellen Entwurf von CSS Syntax passen) | CSS-Tokenizer, Komponentenwerte, Dekodieren von Stylesheets | M1 |
| einzelne Tests der WPT, ohne JavaScript nachgebaut oder in einem kleinen Interpreter nur für Tests: `querySelectorAll()` mit `selectors.js`, die Tests der CSS-Eigenschaften in `css/*/parsing/` | Selektoren, Parsen, Serialisieren und berechnete Werte von CSS-Eigenschaften | M1 |
| webref (aus den Editor's Drafts gelesene CSS-Definitionen) | Grammatik, Anfangswert, Vererbung und Langformen der CSS-Eigenschaften | M1 |
| XML-Testsuite des W3C (über xml-conformance-suite, nur XML 1.0 in der fünften Ausgabe) | XML-Parser | M1 |
| Deflate-Testdaten aus Go (`src/compress/flate/testdata`) und die Fälle aus zlibs `infcover.c` | Deflate | M1 |
| PngSuite und weitere Bilder aus image-png, mit dessen Prüfsummen | PNG-Dekoder | M1 |
| Referenztests der WPT aus `css/CSS2/normal-flow/` (Liste in `tools/testrunner/m1-referenztests.txt`) | Block- und Inline-Layout, Text und Zeichnen | M1 |
| übrige web-platform-tests (testharness.js und Referenztests) | DOM, HTML, CSS, Fetch und alle Web-APIs | M2 |
| Test262 | JavaScript | M2 |
| WebAssembly-Spezifikationstests | WebAssembly | M7 |
| Khronos WebGL Conformance Tests | WebGL | M7 |
| eigene API-Tests und Zielerweiterungen | Erweiterungen | M6 |
| Liste meistbesuchter Seiten | Alltagstauglichkeit | M5 |

- **Ein eigener Testläufer** führt die WPT-Dateien im kopflosen Modus direkt aus, schon bevor WebDriver fertig ist. Später kommt die Anbindung an den offiziellen Testläufer der WPT hinzu, damit die Ergebnisse auch auf wpt.fyi erscheinen. Bisher führt `tools/testrunner` nur die Referenztests für M1 aus; die übrigen Sammlungen aus M1 laufen mit `cargo test` in den Crates, die sie prüfen, etwa html5lib-tests in `mb-html`. Test262 läuft ebenso mit `cargo test` in `mb-js`. Neben den 48.517 Syntaxläufen führt eine feste Auswahl 188 Testdateien in 347 Sprachvarianten aus, jeweils mit normaler und erzwungener Garbage Collection (694 Ausführungen), und ein zweiter Lauf alle Dateien unter `test/language/` gegen eine Datei der erwarteten Fehlschläge; ein Rückschritt oder ein unerwarteter Erfolg lässt ihn scheitern. Dort bestehen bisher 39.420 von 43.794 Varianten. Zwei weitere Läufe nehmen ebenso die Teile von `test/built-ins/` und `test/annexB/built-ins/`, deren eingebaute Objekte es schon gibt (die Liste steht im Stand in [Abschnitt 11](#11-aufbau-des-repositorys)); dort bestehen 21.706 von 23.600 und 208 von 224 Varianten. Alle verwenden gemeinsame Arbeits- und logische Heapquoten; Grenzen und Anschlussarbeiten stehen in [m2-test262-ausfuehrung.md](m2-test262-ausfuehrung.md).
- **Jede Nacht** läuft die gesamte Sammlung, die Ergebnisse werden als Zeitreihe je Bereich festgehalten. Jeder Pull Request führt die betroffenen Bereiche aus. Rückschritte blockieren den Merge.
- **Referenztests** werden mit dem CPU-Renderer ausgeführt, damit die Ergebnisse unabhängig von Grafiktreibern sind.
- **Prioritäten** richten sich nach dem, was das Web tatsächlich nutzt. Dafür gibt es öffentliche Nutzungsstatistiken, etwa Chrome Platform Status und das HTTP Archive.
- **Kompatibilität im echten Web:** Quirks-Modus und der WHATWG Compatibility Standard werden umgesetzt. Viele Seiten werten den User-Agent aus, deshalb gibt es wählbare User-Agent-Profile. Seitenspezifische Korrekturen werden, wenn nötig, offen dokumentiert.

---

## 11. Aufbau des Repositorys

Ein Cargo-Workspace, ausschließlich mit eigenen Crates. Vorschlag:

```text
MeinBrowser/
├── Cargo.toml               Workspace
├── crates/
│   ├── mb-base/             Arenen, Indizes, Atome, Zeichenketten, Bitmengen, Speicherzähler
│   ├── mb-platform/         Fenster, Eingabe, Ein-/Ausgabe, GPU-Oberfläche, Sandbox (eigene FFI)
│   ├── mb-ipc/              Prozesskommunikation über gemeinsamen Speicher
│   ├── mb-unicode/          generierte Unicode-Tabellen und Algorithmen
│   ├── mb-encoding/         WHATWG Encoding
│   ├── mb-url/              WHATWG URL
│   ├── mb-html/             Tokenizer und Baumaufbau
│   ├── mb-xml/              XML, XHTML, SVG-Parser
│   ├── mb-json/             JSON-Zerleger, auch für JSON.parse und Manifeste
│   ├── mb-dom/              Dokumentspeicher und Änderungsjournal
│   ├── mb-css/              CSS-Parser und Eigenschaftsgrammatiken
│   ├── mb-style/            Selektoren, Kaskade, berechnete Stile
│   ├── mb-layout/           Formatierungskontexte, Fragmente
│   ├── mb-text/             Schriften, Textformung
│   ├── mb-scene/            Szenenliste
│   ├── mb-raster/           CPU-Renderer
│   ├── mb-gpu/              GPU-Abstraktion, GPU-Renderer, Compositor
│   ├── mb-shader/           Shader-Übersetzer (GLSL ES, WGSL, eigene Shader)
│   ├── mb-js/               JavaScript-Engine
│   ├── mb-wasm/             WebAssembly
│   ├── mb-webidl/           IDL-Leser und Binding-Generator
│   ├── mb-web/              Laden und Darstellen von Seiten, Web-APIs, Ereignisschleife, Worker
│   ├── mb-compress/         Deflate, Brotli, zstd, ZIP
│   ├── mb-crypto/           Kryptografie hinter einer Anbieter-Schnittstelle
│   ├── mb-net/              DNS, HTTP/1.1/2/3, QUIC, TLS, Fetch, Cookies
│   ├── mb-storage/          Speicherkern
│   ├── mb-image/            Bildformate
│   ├── mb-media/            Audio, Video, Codecs
│   ├── mb-ext/              Erweiterungskern und generierte APIs
│   ├── mb-ui/               Browser-Oberfläche, Skins, Mods
│   ├── mb-devtools/         Entwicklerwerkzeuge, WebDriver, CDP
│   └── meinbrowser/         Programm
├── tools/
│   ├── codegen/             Generator für die Spezifikations-Beschreibungen
│   ├── datagen/             Generatoren für Unicode, CLDR, Zeitzonen und weitere Daten
│   ├── deps-guard/          CI-Prüfung: keine externen Abhängigkeiten
│   ├── testdata/            holt externe Testsammlungen in fester Version
│   └── testrunner/          WPT-Läufer (Test262 läuft in mb-js)
├── fuzz/                    Fuzzing mit cargo-fuzz, eigener Workspace
├── data/                    Rohdaten mit Lizenzhinweisen
├── docs/                    Dokumentation (dieses Dokument)
└── tests/                   eigene Tests
```

Große Testsammlungen wie die WPT werden nicht ins Repository kopiert, sondern in der CI in einer festgelegten Version geholt (`cargo run -p testdata`, nur die benötigten Pfade).

**Stand 14.11.2026:** Angelegt sind der Workspace, alle Crates unter `crates/`, `tools/deps-guard`, `tools/datagen`, `tools/testdata` (holt externe Testsammlungen in fester Version nach `target/testdata/`), `tools/testrunner` (die Liste der 398 Referenztests für M1 aus `css/CSS2/normal-flow/` der WPT und ein Läufer, der Test und Referenz mit den Schriften des Systems zeichnet und Pixel für Pixel vergleicht; alle 398 bestehen, die CI prüft das unter Linux), `data/` und die CI für Linux, Windows und macOS. Inhalt haben bisher `mb-platform` (Fenster mit Eingabe und CPU-Bildausgabe: eigener X11-Client, Win32, AppKit, jeweils mit der Skalierung des Systems für hohe Auflösungen; dazu die Verzeichnisse der Systemschriften und das sichere Lesen lokaler Dateien: nur normale Dateien, geprüft vor dem Öffnen und am offenen Handle, mit Grenze für die Bytes; Schriftdateien lesen `meinbrowser` und `tools/testrunner` beim Indexaufbau und beim Nachladen über `mb-platform::fonts::read_font`, nur aus normalen Dateien mit höchstens 64 MiB je Datei; dies ist keine Grenze für den gesamten Schriftcache oder den RAM-Verbrauch des Parsers), `mb-raster` (Zeichenfläche mit Rechtecken auf ganzen Pixeln, geglätteten Kreisen und Pfaden aus Strecken und Bézierkurven mit Kantenglättung, dazu das Testbild und das Zeichnen einer Szene aus `mb-scene` mit einer festen Zahl Gerätepixel je CSS-Pixel), `meinbrowser` (zeigt das Testbild, mit Selbsttest in der CI, und mit `meinbrowser DATEI` eine lokale HTML- oder XHTML-Datei, mit der Skalierung des Systems und mit Scrollen über Mausrad, Touchpad und Tasten; gezeichnet wird auf einem eigenen Faden, das Fenster bleibt bedienbar und bricht überholte Aufträge ab, und zum Scrollen wird nur neu gezeichnet, nicht neu gesetzt), `mb-unicode` (allgemeine Kategorie, kanonische Kombinationsklasse, Bidi-Klasse, Joining-Type, Zeilenumbruchklasse, East Asian Width, Extended_Pictographic sowie ID_Start und ID_Continue für Bezeichner und die Namen der Werte von General_Category und Script, von `tools/datagen` aus der Unicode-Zeichendatenbank 18.0.0 erzeugt, dazu die Normalisierung NFC, NFD, NFKC und NFKD, IDNA nach UTS #46 mit Punycode, Tabelle aus der IDNA-Mappingtabelle 18.0.0, der Zeilenumbruch nach UAX #14 ohne Wörterbücher für südostasiatische Schriften, gegen alle Fälle aus `LineBreakTest.txt` geprüft, die Faltung der Groß- und Kleinschreibung aus `CaseFolding.txt` und die Umwandlung in Klein- und Großbuchstaben nach Abschnitt 3.13 des Unicode-Standards mit den vollständigen Abbildungen aus `SpecialCasing.txt`, ohne die Regeln für einzelne Sprachen), `mb-encoding` (alle 40 Kodierungen des WHATWG Encoding Standard, Tabellen ebenfalls von `tools/datagen`), `mb-json` (ein JSON-Zerleger ohne Rekursion auf UTF-16-Codeeinheiten, den auch `JSON.parse` in `mb-js` nutzt, und darauf ein Leser für Rohdaten und Testdaten), `mb-url` (URL-Parser nach dem WHATWG URL Standard mit den Gettern und Settern der Klasse `URL`, Hosts mit IDNA, Prozentkodierung und `application/x-www-form-urlencoded`; alle URL-Testdaten der WPT bestehen), `mb-base` (`DomString`, Zeichenketten aus UTF-16-Codeeinheiten wie im DOM, und Arbeitsbudgets mit Abbruch, aus denen Stil und Layout fremder Seiten ihre Arbeit bezahlen), `mb-dom` (Dokumentspeicher als Spaltenspeicher mit Änderungsjournal, Einfügen, Entfernen, Ersetzen und Klonen nach dem DOM Standard, dazu Auswahl und `selectedcontent` von `select` und `option` aus dem HTML Standard und für XML beliebige Namensräume und CDATA-Abschnitte), `mb-html` (Tokenizer und Baumaufbau nach dem WHATWG HTML Standard samt Fragment-Parsing und den benannten Zeichenreferenzen, Tabelle von `tools/datagen`; die Tokenizer-Tests aus html5lib-tests bestehen bis auf `xmlViolation.test`, das nur Werkzeuge betrifft, die aus HTML XML machen, die Baumaufbau-Tests aus den WPT bis auf vier Dateien, die laufende Skripte erwarten; dazu die Zeichenkodierung aus den Bytes mit dem Vorab-Lesen von `meta` und dem Wechsel der Kodierung beim Parsen, gegen die Kodierungstests aus html5lib-tests geprüft), `mb-css` (Dekodieren von Stylesheets, Tokenizer und Parser mit allen Einstiegen nach dem Entwurf von CSS Syntax Level 3, dazu die Mikrosyntax An+B; die Dateien aus css-parsing-tests, die zu diesem Entwurf passen, bestehen bis auf drei begründete Abweichungen), `mb-style` (Selektoren nach dem Entwurf von Selectors Level 4 mit Parser, Spezifität und Abgleich gegen den Dokumentspeicher; die Fälle des WPT-Tests zu `querySelectorAll()` bestehen bis auf zwei, die Elemente in fremden Namensräumen brauchen. Dazu die Werte und Eigenschaften der ersten Ausbaustufe einschließlich `background`, `outline` und ihrer Langformen mit Parsen und Serialisieren, deren Tabellen gegen webref geprüft werden, und Deklarationsblöcke nach CSSOM. Außerdem Stylesheets mit Verschachtelung nach CSS Nesting, `@layer`, `@namespace` und `@import`, `@media` mit Media Queries nach dem Entwurf von Media Queries 5 und `@supports` samt `CSS.supports()` nach CSS Conditional 3 bis 5, das Stylesheet der Benutzeragenten aus dem Abschnitt „Rendering“ des HTML Standard, Tabelle von `tools/datagen`, dazu `width` und `height` von Bildern und eingebetteten Inhalten und die Ränder von `body` aus seinen Attributen als *presentational hints*, die Kaskade nach CSS Cascade 5 mit `revert`, `revert-layer`, `revert-rule` und den logischen Eigenschaften und daraus die berechneten Werte und die Werte für `getComputedStyle()`. Von den 15149 WPT-Tests zum Parsen und zu den berechneten Werten dieser Eigenschaften bestehen 8873; 6230 brauchen noch fehlende Funktionen wie `calc()`, `color-mix()` und relative Farben, 35 das Layout, Skriptmittel oder ein nachgeladenes Stylesheet, die der Testläufer nicht hat, und elf weichen begründet ab; von den 57 WPT-Tests zu `CSS.supports()` bestehen 46, zehn brauchen noch Custom Properties, `@font-face` oder `::details-content`, einer lässt sich ohne Custom Properties nicht ausführen), `mb-text` (Leser für Schriftdateien im sfnt-Format, also TrueType und OpenType samt Sammlungen: Kopfdaten, Metriken, Namen, die Zuordnung von Zeichen zu Glyphen und die Umrisse aus `glyf`; Umrisse aus CFF, variable Schriften, Farbschriften und WOFF fehlen noch; dazu die Bibliothek der Systemschriften (ihr Index entsteht bei jedem Start neu, ein zwischengespeicherter fehlt noch) mit der Schriftauswahl nach CSS Fonts 4, den generischen Familien nach den Vorgaben von fontconfig und der Suche nach Ersatzschriften, und eine einfache Textformung mit Glyphen aus `cmap` und Vorschüben aus `hmtx`, noch ohne GSUB und GPOS, bei der die Schrift für ganze Cluster gewählt wird; hat eine Familie keinen kursiven oder schrägen Schnitt, wird der aufrechte künstlich schräg gestellt, die Eigenschaft `font-synthesis-style` fehlt noch), `mb-xml` (nicht validierender Parser nach XML 1.0 in der fünften Ausgabe und Namespaces in XML 1.0, für XHTML mit den Entitäten des HTML Standard; von den 1908 Fällen der XML-Testsuite des W3C, die für einen nicht validierenden Parser ohne externe Entitäten gelten, weicht einer begründet ab), `mb-compress` (Deflate nach RFC 1951 und das zlib-Format nach RFC 1950 mit CRC-32 und Adler-32, geprüft mit den Deflate-Testdaten aus Go und den Fällen aus zlibs `infcover.c`), `mb-image` (PNG nach der vierten Ausgabe der PNG Specification, ohne Animation und Farbmanagement; von den 195 Testbildern aus image-png ergeben 173 dessen Prüfsummen, die übrigen 22 sind fehlerhaft und werden wie erwartet abgelehnt; außerdem das erste Einzelbild von GIF87a und GIF89a mit globalen und lokalen Paletten, Transparenz und Interlacing, ohne Animationsablauf), `mb-scene` (Szene als Liste von Zeichenbefehlen: gefüllte Rechtecke und Vielecke, Glyphenfolgen und Bilder, auch gekachelt; das kompakte Binärformat fehlt noch), `mb-layout` (Kastenbaum aus Dokument und Stilen, Block- und Inline-Formatierungskontexte mit zusammenfallenden Außenabständen, Leerraum, Zeilenumbruch und senkrechter Ausrichtung, Inline-Blöcke, Bilder als ersetzte Elemente, der Bereich, über den der Viewport scrollt, nach CSS Overflow 3 und das Zeichnen des Fragmentbaums an einer Scrollposition, auch mit gepunkteten und gestrichelten Rahmen und Umrissen; Floats, Positionierung, die Eigenschaft `overflow`, Tabellen, Flex, Grid, Listen, generierter Inhalt, Bidi und senkrechte Schreibmodi fehlen noch, und die Rahmenstile `double`, `groove`, `ridge`, `inset` und `outset` werden noch durchgezogen gezeichnet), `mb-web` (bisher nur das Laden und Darstellen einer Seite ohne Skripte: Dokument aus dem HTML- oder XML-Parser, Stylesheets aus `<style>` und `<link rel=stylesheet>` mit dem Attribut `media` und aus `@import`, PNG-Bilder und das erste Einzelbild von GIF87a/GIF89a, jeweils nach Dateisignatur erkannt, Layout und Szene, mit den Regeln, deren Media Queries zum Viewport passen; eine lokale Seite lädt nur normale Dateien, Abrufe erhalten ihr verbleibendes Bytekontingent als Leselimit, und Eingaben sowie dekodierte Bilddaten teilen sich 256 MiB; dies ist keine Grenze für den gesamten RAM-Verbrauch; ohne Angabe der Kodierung gilt UTF-8, die Ersatzkodierung nach Sprache und Region fehlt noch), `mb-webidl` (WebIDL-Tokenizer mit unveränderten Tokens und Byte-Ranges sowie iterativer LL(1)-Syntaxparser mit flachem Arena-Baum und Hostquoten, Tabellen vom eigenen `datagen` aus der normativen Grammatik; Namens-/Typprüfung und Binding-Generator fehlen noch) und `mb-js` (bisher der Lexer nach dem Entwurf von ECMA-262 mit den HTML-ähnlichen Kommentaren aus Anhang B.1.1: Tokens für alle Ziele der lexikalischen Grammatik, Zahlen exakt gerundet, Zeichenketten und Templates mit ihren Werten, Altformen für den strikten Modus markiert; dazu der Parser für Scripts und Module mit dem Syntaxbaum in einer Arena, allen frühen Fehlern der Abschnitte 13 bis 16 und den Erweiterungen aus Anhang B.3 und einer Grenze für die Verschachtelung statt tiefer Rekursion, und der Parser für die Muster regulärer Ausdrücke nach Abschnitt 22.2.1 mit Anhang B.1.2, mit allen frühen Fehlern einschließlich doppelter Gruppennamen, Modifikatoren und der Klassen mit `v`, ohne Rekursion; er liefert den Baum, aus dem die noch fehlende Engine ihre Automaten bauen soll. Dazu kommen die Werte mit NaN-Boxing und der Heap: Seiten mit 32-Bit-Abständen, flache Zeichenketten und interne Felder, Handles in Bereichen und dauerhafte Handles, ein kopierender Garbage Collector nach Cheney ohne Rekursion und ohne `unsafe` sowie ein Belastungs- und ein Prüfmodus. Darauf bauen der Agent mit der geworfenen Ausnahme, Atome als Eigenschaftsschlüssel, Symbole und die gewöhnlichen Objekte nach Abschnitt 10.1 auf: Gestalten als Ketten mit schwachen Übergangstabellen, die der Collector leert, sobald keine Gestalt mehr gebraucht wird, dazu dichte oder dünn besetzte Elemente und eine Prototypkette, die ohne Rekursion durchlaufen wird, außerdem Arrays nach Abschnitt 10.4.2, die bekannten Symbole, die Umwandlungen ToPrimitive, ToBoolean, ToNumeric, ToNumber mit StringToNumber, ToIntegerOrInfinity, ToInt32, ToUint32, ToString mit Number::toString und ToPropertyKey, die Vergleiche für `==`, `===` und `<` sowie die zweistelligen Rechen- und Bitoperatoren nach ApplyStringOrNumericBinaryOperator und `typeof`. Scripts und Funktionen mit Closures laufen schon: Ein Realm mit %Object.prototype%, %Function.prototype%, den Prototypen für Arrays und die primitiven Werte, die selbst Wrapper-Objekte sind, und für die Fehler, und einem globalen Objekt mit den Werteigenschaften aus Abschnitt 19.1, den Funktionen `isFinite`, `isNaN`, `parseFloat` und `parseInt`, den Konstruktoren `Object`, `Function` (bisher nur als Platzhalter, siehe unten), `Array`, `String`, `Symbol`, `Boolean`, `Number`, `Error`, den sechs *NativeError*, `AggregateError`, `Promise`, `Proxy`, `Iterator`, `Map` und `Set`, mit `Math`, `JSON` und `Reflect` und den Eigenschaften, die der Host über das Trait `Host` anlegt, die globale Umgebung mit deklarativem Record und GlobalDeclarationInstantiation, ein Übersetzer in Bytecode für eine Registermaschine mit Akkumulator (Bindungen in Registern, nur die, deren Namen innere Funktionen nennen, in Kontexten im Heap; die TDZ-Prüfung entfällt, wo sie nicht nötig ist, Ketten zweistelliger Operatoren und Ketten von Zugriffen und Aufrufen ohne Rekursion) und ein Interpreter, dessen Register im Heap liegen und dem Collector als Wurzeln dienen; eine Ausnahme findet ihren Handler über eine Tabelle je Code und baut die Rahmen bis dorthin ohne Rekursion ab. Aufrufe zwischen JavaScript-Funktionen brauchen keinen Rust-Stapel; ruft die Engine eine Funktion auf, etwa einen Getter, schachtelt sich ein neuer Lauf, und diese Läufe begrenzt ein Budget für den Rust-Stapel. Ein Generator bewahrt seinen angehaltenen Rahmen als Kopie im Heap auf, und jedes `next` setzt ihn in einem solchen neuen Lauf fort. Ebenso hält eine async-Funktion bei `await` an; fortgesetzt wird sie von einem Promise-Job. Ein async-Generator verbindet beides: `yield` und `await` halten seinen Rahmen an, und seine Anfragen warten in einer Warteschlange, jede mit ihrem eigenen Promise. Übersetzt werden Ausdrücke mit Bezeichnern, Literalen außer BigInt und regulären Ausdrücken, Objekt- und Array-Literalen mit Methoden und Accessoren und mit Ausbreitung, Templates ohne Tag, Eigenschaftszugriffen auch auf primitive Werte und in optionalen Ketten, Aufrufen auch optional, `new` und `new.target`, Aufrufe und `new` auch mit Ausbreitung, Funktionsausdrücken, Pfeilfunktionen und Funktionsdeklarationen, auch in Blöcken mit Anhang B.3.2 und B.3.3, Generatorfunktionen und Generator-Methoden mit `yield` und `yield*` nach Abschnitt 15.5, async-Funktionen, async-Methoden und async-Pfeilfunktionen mit `await` nach den Abschnitten 15.8 und 15.9, async-Generatoren und async-Generator-Methoden mit `yield`, `yield*` und `await` nach Abschnitt 15.6, Klassen nach Abschnitt 15.7 mit Konstruktor, Methoden, Accessoren und statischen Methoden, öffentlichen und privaten Feldern, privaten Methoden und Accessoren, `#x in` und `static`-Blöcken, mit `extends` und mit `super` in Konstruktoren, Methoden, Feldern und Objektliteralen (private Namen sind Symbole mit einem Kennzeichen, private Elemente liegen in der Gestalt des Objekts, und die Aufzählung der Schlüssel überspringt sie; die Felder einer Instanz legt eine Schleife im Bytecode des Konstruktors an; alle Teile einer Klasse laufen strikt, in nicht striktem Code über eine Tabelle strikter Bereiche im Bytecode), jeweils mit Parametern aus Bezeichnern und Mustern, auch mit Vorgabewerten und als Rest-Parameter, und mit dem arguments-Objekt nach Abschnitt 10.4.4, allen Operatoren, Zuweisungen auch mit Operator und mit Mustern (Destrukturierung nach Abschnitt 13.15.5, mit Rest in Arrays und Objekten und mit dem Schließen der Iteratoren), `++` und `--`, dazu `var`, `let`, `const` mit Bezeichnern und Mustern, Blöcke, `if`, `while`, `do`-`while`, `for` mit eigenen Bindungen für jeden Durchlauf, `for`-`of` mit dem Iteratorprotokoll aus Abschnitt 7.4, auch mit Mustern links, und `for await` in async-Funktionen und async-Generatoren, `for`-`in` nach EnumerateObjectProperties (Abschnitt 14.7.5.9), auch mit Initialisierer nach Anhang B.3.5, `switch`, Marken, `break`, `continue`, `return`, `throw` und `try` mit `catch` und `finally`, auch mit Mustern im Parameter von `catch` und mit Anhang B.3.4; alles andere meldet der Übersetzer als nicht unterstützt. Eine Funktion wird erst bei ihrem ersten Aufruf übersetzt; was der Übersetzer dort nicht kann, etwa `with` im Rumpf, beendet das Script dann. Funktionsobjekte haben `length`, `name` und `prototype`; `caller` und `arguments` erben sie von %Function.prototype%, wo beide werfen (%ThrowTypeError%), die Erweiterung aus Abschnitt 17.1 gibt es nicht. Gebundene Funktionen nach Abschnitt 10.4.1 lösen ihre Kette ohne Rekursion auf. Gewöhnliche Funktionen und Klassen haben [[Construct]] nach Abschnitt 10.2.2, Pfeilfunktionen, Methoden, Accessoren, Generatoren, async-Funktionen und async-Generatoren nicht; eine Klasse lässt sich nur mit `new` aufrufen, und ein Konstruktor einer abgeleiteten Klasse bekommt `this` erst von `super()`. Im strikten Modus sind Aufrufe in Endposition echte Endaufrufe nach Abschnitt 15.10, dabei wächst der Registerstapel nicht. Eingebaute Funktionen nach Abschnitt 10.3 laufen im Befehl des Aufrufs, ohne eigenen Rahmen; davon gibt es bisher `Object` mit allen Funktionen aus Abschnitt 20.1.2 und %Object.prototype% mit allen aus Abschnitt 20.1.3 und Anhang B.2.2, `Array` mit allen Funktionen aus Abschnitt 23.1.2 einschließlich `Array.fromAsync` und %Array.prototype% mit allen aus Abschnitt 23.1.3 (`sort` sortiert stabil durch Mischen), %Function.prototype% mit `apply`, `bind`, `call`, `toString` (mit dem Quelltext, den der Syntaxbaum dafür behält) und @@hasInstance, den Konstruktor `Function` nur als Platzhalter, dessen Aufruf als nicht unterstützt endet, bis CreateDynamicFunction mit `eval` kommt, `Error` mit `Error.isError` und `Error.prototype.toString`, die sechs *NativeError* und `AggregateError` nach Abschnitt 20.5, `Symbol` mit den bekannten Symbolen, `Symbol.for`, `Symbol.keyFor` und %Symbol.prototype% nach Abschnitt 20.4, `Boolean` nach Abschnitt 20.3, `Number` mit allen Eigenschaften aus Abschnitt 21.1 (`toFixed`, `toExponential` und `toPrecision` runden den genauen Dezimalwert, `toString` gibt zu anderen Basen so viele Stellen aus, wie die Zahl von ihren Nachbarn unterscheiden), `Math` mit allen Funktionen aus Abschnitt 21.3 einschließlich `sumPrecise` (genaue Summe als Festkommazahl, einmal gerundet) und `f16round`, die globalen Funktionen `isFinite`, `isNaN`, `parseFloat` und `parseInt`, `String` mit `fromCharCode`, `fromCodePoint` und `raw` und %String.prototype% mit allen Methoden aus Abschnitt 22.1.3 und Anhang B.2.2 (die Suche nach einer Zeichenkette läuft in linearer Zeit, Groß- und Kleinschreibung und `normalize` nutzen `mb-unicode`, `localeCompare` vergleicht bis `Intl` kommt die Codepunkte der NFD-Formen, und jede Methode prüft die Länge ihres Ergebnisses gegen 2³² − 1 Codeeinheiten und die Heapquote; `match`, `matchAll` und `search` rufen die Methode ihres Musters auf und enden mit einem Muster ohne solche Methode als nicht unterstützt, bis es die Engine für reguläre Ausdrücke gibt), dazu Boolean-, Number-, String- und Symbol-Objekte mit ToObject, `Iterator` mit allen Eigenschaften aus Abschnitt 27.1.3, also `Iterator.from`, `concat`, `zip` und `zipKeyed` und an %Iterator.prototype% allen Helfern wie `map`, `flatMap`, `chunks` und `windows` und den Methoden wie `reduce` und `join`, dazu `includes` aus dem aktuellen Entwurf (ein Helfer hält den Zustand seiner Closure in einer internen Zelle, nicht als angehaltenen Rahmen), und `JSON` nach Abschnitt 25.5 mit `parse` samt *reviver* und dem Quelltext der Werte, `stringify`, `rawJSON` und `isRawJSON` (mit dem Zerleger aus `mb-json` und einer eigenen Serialisierung ohne Rekursion), `Map` und `Set` nach den Abschnitten 24.1 und 24.2 mit `Map.groupBy`, `getOrInsert`, `getOrInsertComputed` und den Methoden für Vereinigung, Schnitt und Differenz (eine Hashtabelle in Zellen des Heaps, die die Reihenfolge der Einträge hält; Objekte und Symbole bekommen ihren Hashwert aus ihrer Lage im Heap und nach einer Sammlung einen neuen), `Reflect` nach Abschnitt 28.1 und `Proxy` mit `Proxy.revocable` nach Abschnitt 28.2, dazu die Proxy-Objekte nach Abschnitt 10.5 mit allen internen Methoden und den Prüfungen ihrer Invarianten (eine Kette von Proxys ohne Trap durchläuft die Engine ohne Rekursion; jeder Schritt durch einen Proxy zählt wie ein neuer Lauf des Interpreters, kostet also einen Arbeitsschritt und darf den Rust-Stapel nur bis zum Budget belegen; neue Objekte für einen Trap, der Deskriptor für `defineProperty` und das Array der Argumente für `apply` und `construct`, entstehen im Realm des laufenden Codes, den der Agent dafür mitführt). Außerdem gibt es die Iteratoren für Arrays und Zeichenketten, dazu %GeneratorFunction.prototype% und %GeneratorPrototype% mit `next`, `return` und `throw`, noch ohne den Konstruktor %GeneratorFunction%, %AsyncFunction.prototype%, noch ohne den Konstruktor %AsyncFunction%, %AsyncIteratorPrototype% mit @@asyncIterator, %AsyncFromSyncIteratorPrototype% für `for await` über synchrone Iteratoren, %AsyncGeneratorFunction.prototype% und %AsyncGeneratorPrototype% mit `next`, `return` und `throw`, noch ohne den Konstruktor %AsyncGeneratorFunction%, und `Promise` mit allen Eigenschaften nach Abschnitt 27.5 samt `AggregateError`; Jobs hält der Agent in einer Warteschlange, die der Host leert, wenn kein Script läuft, und der Host erfährt über einen Hook von abgelehnten Promises ohne Handler. BigInt-Werte und die übrigen eingebauten Objekte fehlen noch, und ein Fehler der Engine wird erst zum Fehlerobjekt, wenn `catch` ihn fängt. Funktionsrümpfe werden noch vollständig gelesen statt nur vorgeprüft. Alle 48517 Läufe der Syntaxtests aus Test262 bestehen, das sind `test/language`, `test/annexB/language`, `test/built-ins/RegExp` und `test/annexB/built-ins/RegExp` ohne Vorschläge; ausgeführt bestehen von `test/language` 39420 von 43794 Varianten, von den fertigen Teilen von `test/built-ins` (`AggregateError`, `Array`, `Boolean`, `Error`, `Function`, `Iterator`, `JSON`, `Map`, `MapIteratorPrototype`, `Math`, `NativeErrors`, `Number`, `Object`, `Promise`, `Proxy`, `Reflect`, `Set`, `SetIteratorPrototype`, `String`, `StringIteratorPrototype`, `Symbol`, `isFinite`, `isNaN`, `parseFloat`, `parseInt`) zusammen 21706 von 23600 und von `test/annexB/built-ins/Object` und `String` 208 von 224). M0 und M1 sind abgeschlossen, M2 hat begonnen. Die übrigen Crates sind noch leer. `tools/codegen` und `tests/` kommen mit den Meilensteinen, die sie brauchen.

---

## 12. Roadmap und Meilensteine

Die Meilensteine bauen aufeinander auf, viele Teile lassen sich aber parallel bearbeiten. Jeder Meilenstein hat ein prüfbares Abschlusskriterium. Bewusst stehen hier keine Termine, denn die hängen davon ab, wie viele Menschen mitarbeiten (siehe [Abschnitt 13](#13-der-aufwand-ehrlich-betrachtet)). Wie weit jeder Meilenstein ist, steht in [12.1](#121-stand-der-meilensteine), was als Nächstes kommt, in [12.2](#122-nächste-schritte).

| # | Meilenstein | Inhalt | Fertig, wenn |
|---|---|---|---|
| M0 | Fundament | Workspace, CI für Linux, Windows und macOS, Abhängigkeitswächter, Unicode-Generator, Plattformschicht mit Fenster, Eingabe und CPU-Bildausgabe | Ein Fenster zeigt auf allen drei Systemen ein selbst gezeichnetes Testbild, die CI ist grün. |
| M1 | Erstes Dokument | Encoding, URL, HTML-Parser, Dokumentspeicher, CSS-Grundlagen, Block- und Inline-Layout, einfache Textformung, Systemschriften, CPU-Renderer, dazu XML und XHTML sowie PNG mit Deflate, weil die Referenztests sie brauchen (entschieden am 25.10.2026) | html5lib-tests vollständig bestanden, die URL-Testdaten der WPT weitgehend bestanden, eine lokale HTML-Datei wird korrekt angezeigt (gemessen an den Referenztests in `tools/testrunner/m1-referenztests.txt`). |
| M2 | JavaScript | Parser, Bytecode, Interpreter, Garbage Collector, eingebaute Objekte (ohne Intl), WebIDL-Generator, DOM-Bindings, Ereignisschleife, eigener WPT-Läufer | Test262 ohne Intl zu einem hohen Anteil bestanden, testharness.js läuft, WPT-Ergebnisse für DOM-Bereiche werden täglich gemessen. |
| M3 | Netzwerk | DNS, HTTP/1.1, TLS 1.3 und 1.2, X.509, Kompression, Cookies, Fetch, HTTP-Cache, HSTS | Beliebige HTTPS-Seiten laden. WPT-Bereiche zu Fetch und Cookies werden gemessen. |
| M4 | Modernes Layout und GPU | Flex, Grid, Positionierung, Floats, Tabellen, Scrollen, Transformationen, Filter, Animationen, GPU-Renderer, Compositor | CSS-Bereiche der WPT erreichen ihre Zielquoten, CPU- und GPU-Renderer stimmen in den Referenztests überein. |
| M5 | Alltagstauglich | Formulare, Frames, Navigation und Verlauf, Prozessisolation und Sandbox, Speicherung, alle Bildformate, WOFF2, komplexe Schriften, Bidi, HTTP/2, Canvas 2D, SVG, Oberfläche mit Tabs, Lesezeichen und Downloads, Skins, WebDriver | Die Liste meistbesuchter Seiten ist bedienbar, die Entwickler nutzen den Browser im Alltag. |
| M6 | Erweiterungen | Manifest V2 und V3 von Chrome und Firefox, CRX3 und XPI, Inhalts-Skripte, die APIs aus [5.3](#53-reihenfolge-der-apis) Stufe 1 bis 3, Themes | Die Zielerweiterungen laufen ohne Änderung. |
| M7 | Medien und Grafik | Audio, Video, Media Source Extensions, eigene Codecs, Web Audio, WebGL 1 und 2, WebGPU, WebAssembly, Intl | Videoseiten ohne DRM spielen ab, Konformitätstests für WebGL und WebAssembly werden gemessen. |
| M8 | Leistung | Basis-JIT, Einfrieren von Tabs, paralleles Stil und Layout, HTTP/3 und QUIC, später optimierender JIT | Die Zielwerte aus [Abschnitt 7](#7-ressourcenschonung) sind erreicht, Speedometer und JetStream werden regelmäßig gemessen. |
| M9 | Breite | Android, weitere Architekturen, vollständige Barrierefreiheit und Eingabemethoden, Entwicklerwerkzeuge und CDP, WebRTC, Service Worker, Push | Stufe-2-Plattformen laufen, Screenreader-Tests auf allen Stufe-1-Systemen bestehen. |
| M10 | Version 1.0 | Externes Sicherheitsaudit (insbesondere Kryptografie, Sandbox, JIT), signierte Updates, Absturzberichte, Datenschutzprüfung, Doku für Skin- und Mod-Autoren | Das Audit ist abgeschlossen und alle kritischen Befunde sind behoben. |

Barrierefreiheit (M9) wird schon ab M5 mitgedacht und nicht erst am Ende angefangen. Die Sicherheitsaufgaben je Meilenstein stehen in [8.3](#83-sicherheit-in-jedem-meilenstein).

### 12.1 Stand der Meilensteine

Stand 14.11.2026, nach PR #109: M0 und M1 sind abgeschlossen, M2 läuft. Einige Teile späterer Meilensteine sind vorgezogen. Was der Code im Einzelnen kann, steht im Stand in [Abschnitt 11](#11-aufbau-des-repositorys).

| # | Stand | Erreicht | Bis zum Abschlusskriterium fehlt |
|---|---|---|---|
| M0 | abgeschlossen | Workspace, CI für Linux, Windows und macOS, Abhängigkeitswächter, `tools/datagen` für die Unicode-Tabellen, Fenster mit Eingabe und CPU-Bildausgabe über einen eigenen X11-Client, Win32 und AppKit. Die CI prüft das Testbild auf allen drei Systemen mit `meinbrowser --selbsttest`. | Nichts. Der eigene Wayland-Client aus [4.2](#42-plattformschicht) kommt später, bis dahin läuft der X11-Client über XWayland. |
| M1 | abgeschlossen am 26.10.2026 | Encoding, URL, HTML, XML, Dokumentspeicher, CSS bis zur Kaskade und den berechneten Werten, Block- und Inline-Layout, einfache Textformung, Systemschriften, PNG, CPU-Renderer und `meinbrowser DATEI`. Zum Kriterium: Die Tokenizer-Tests aus html5lib-tests bestehen bis auf `xmlViolation.test`, die Baumaufbau-Tests bis auf vier Dateien, die laufende Skripte erwarten. Alle URL-Testdaten der WPT und alle 398 Referenztests bestehen. Bis zum 30.10.2026 kamen Scrollen, hohe Auflösungen, `@media`, `@supports`, `@import` und das erste Einzelbild von GIF dazu. | Nichts. Die vier Baumaufbau-Dateien lassen sich erst mit Skripten im Dokument prüfen (M2). |
| M2 | läuft seit 30.10.2026 | JavaScript: Parser für Scripts, Module und die Muster regulärer Ausdrücke, Werte, Heap und kopierender Garbage Collector, Bytecode und Interpreter. Die Schritte 1 bis 7 aus [12.2](#122-nächste-schritte) und die ersten sieben Wellen von Schritt 8 bis auf die Engine für reguläre Ausdrücke sind umgesetzt, also die Sprache bis zu Klassen, Generatoren, `async` und `await` und `for`-`in`, dazu `Promise`, `Object`, `Array`, `Function.prototype` mit gebundenen Funktionen, die Fehlerobjekte, `Symbol`, `Boolean`, `Number`, `Math`, die globalen Funktionen für Zahlen, `Iterator` mit den Helfern, `JSON`, `Map` und `Set`, `Reflect` und `Proxy` und `String` mit allen Methoden, die ohne reguläre Ausdrücke auskommen. Test262: Alle 48.517 Syntaxläufe bestehen. Ausgeführt werden 35.321 der 48.882 Testdateien außerhalb von `intl402/` und `staging/`, nämlich `test/language/`, die fertigen Teile von `test/built-ins/` (siehe [Abschnitt 11](#11-aufbau-des-repositorys)) sowie `test/annexB/built-ins/Object/` und `String/` ohne Vorschläge; 31.405 davon bestehen, das sind gut 64 Prozent der Sammlung ohne Intl. WebIDL: Tokenizer, Syntaxparser und Namensindex über mehrere Quellen. | Der Rest von Schritt 8 und die Schritte 9 bis 12 aus [12.2](#122-nächste-schritte), vor allem die eingebauten Objekte. Die Typ- und Memberprüfung für WebIDL und der Binding-Generator, DOM-Bindings, die Ereignisschleife und Skripte im Dokument (`mb-web` bindet `mb-js` noch nicht ein). Ein WPT-Läufer für testharness.js und die tägliche Messung der DOM-Bereiche. |
| M3 | Vorarbeiten | Deflate und zlib mit ihren Prüfsummen (`mb-compress`). Seiten holen ihre Dateien über eine Abruffunktion mit Bytekontingent, bisher nur aus lokalen Dateien. | Alles Übrige: DNS, HTTP/1.1, TLS, X.509, Cookies, Fetch, HTTP-Cache und HSTS. `mb-net` und `mb-crypto` sind leer. |
| M4 | Vorarbeiten | Scrollen des Viewports nach CSS Overflow 3, Skalierung für hohe Auflösungen. | Flex, Grid, Positionierung, Floats, Tabellen, die Eigenschaft `overflow`, Transformationen, Filter, Animationen, GPU-Renderer und Compositor. `mb-gpu` und `mb-shader` sind leer. |
| M5 | Vorarbeiten | PNG und das erste Einzelbild von GIF. | Alles Übrige. `mb-ipc`, `mb-storage` und `mb-ui` sind leer. |
| M6 bis M10 | nicht begonnen | – | Alles. `mb-ext`, `mb-media`, `mb-wasm` und `mb-devtools` sind leer. |

### 12.2 Nächste Schritte

Für JavaScript folgt M2 dem am 31.10.2026 angenommenen Plan in zwölf Schritten. Umgesetzt sind die Schritte 1 bis 7: Werte und Heap, Objekte, Bytecode und Interpreter für Scripts, Funktionen, Realm, Host und Test262, dann Klassen, Destrukturierung, Ausbreitung, Iteratoren und Generatoren und zuletzt `Promise`, Jobs, `async` und `await`; dazu von Schritt 8 die ersten sechs Wellen (PR #102, PR #103, PR #104, PR #105, PR #107 und PR #108). Offen sind:

8. Eingebaute Objekte in Wellen, jede mit ihrem Teil von `test/built-ins/`: `Object`, `Function.prototype` und `Array` sind fertig, mit `Array` auch `for`-`in`, dazu `Error`, `Symbol`, `Boolean`, `Number` und `Math` mit `isFinite`, `isNaN`, `parseFloat` und `parseInt`, die Iterator-Helfer, `JSON`, `Map` und `Set`, `Reflect` und `Proxy` und `String` mit allen Methoden, die ohne reguläre Ausdrücke auskommen; als Nächstes die Engine für reguläre Ausdrücke mit `RegExp`, den Literalen und den Wegen von `match`, `matchAll` und `search`, die ein Muster erst anlegen. Danach BigInt, `Date`, `ArrayBuffer` mit den typisierten Arrays und die schwachen Sammlungen, zuletzt `SharedArrayBuffer` und `Atomics`.
9. `eval`, `with`, der `Function`-Konstruktor und Module mit `import()`.
10. Garbage Collector mit Generationen und Schreibbarriere, dazu nächtliche Belastungstests.
11. Zwischenspeicher für Bytecode pro Skript-Hash. Er braucht einen eigenen Hash aus `mb-crypto`.
12. Vorprüfung der Funktionsrümpfe, wie Stufe 1 in [4.9](#49-javascript-und-webassembly) sie vorsieht.

In der Sprache fehlen daneben noch Templates mit Tag, `using` und Funktionsdeklarationen mit Marke (Anhang B.3.1). Literale für BigInt und reguläre Ausdrücke brauchen ihre eingebauten Objekte aus Schritt 8.

Für die übrigen Teile von M2 stehen an:

- **WebIDL:** auf dem Namensindex eine geprüfte Beschreibung der Typen und Member aufbauen (Member-Namen, Dictionaries, Overloads, Extended Attributes), danach der Binding-Generator ([m2-webidl-index.md](m2-webidl-index.md)).
- **Skripte im Dokument:** DOM-Bindings, Ereignisschleife und das Ausführen von `script`-Elementen in `mb-web`.
- **Messung:** ein WPT-Läufer für testharness.js und ein nächtlicher Lauf, der die Ergebnisse je Bereich festhält. Bisher läuft nachts nur das Fuzzing.

Danach folgt M3. Für „Test262 ohne Intl zu einem hohen Anteil bestanden“ ist noch kein Zahlenwert festgelegt.

---

## 13. Der Aufwand, ehrlich betrachtet

Ein Browser, der heutige Webseiten zuverlässig darstellt und die Webtests weitgehend besteht, gehört zu den größten Softwareprojekten überhaupt. Allein JavaScript-Engine, Textformung für alle Schriften, CSS-Layout oder ein Videocodec wie AV1 sind jeweils ein eigenes, jahrelanges Vorhaben.

Ein Vergleich: Ladybird, ein unabhängiger Browser mit eigener Engine, begann 2019 als HTML-Betrachter im Hobbyprojekt SerenityOS. Heute arbeitet ein kleines Vollzeit-Team daran. Die erste Alpha-Version ist für 2026 angekündigt, Beta und stabile Version sind laut Wikipedia für 2027 und 2028 geplant. Selbst Ladybird nutzt für Bild-, Audio- und Videoformate, Verschlüsselung und Grafik inzwischen fremde Bibliotheken. Unser Grundsatz, das alles selbst zu bauen, macht den Weg also noch länger.

Was daraus folgt:

- **Ein alltagstauglicher Stand (M5) ist eine Frage von Jahren, nicht von Monaten,** und das umso mehr, je weniger Menschen mitarbeiten.
- **Klare Prioritäten:** zuerst das, was die meisten Seiten brauchen. Messbarer Fortschritt über die WPT hält das Projekt ehrlich und motivierend.
- **Parallelisierbarkeit:** Die Crate-Grenzen sind so gezogen, dass viele Menschen gleichzeitig an getrennten Teilen arbeiten können. Das ist wichtig, um Mitwirkende zu gewinnen.
- **Früh sichtbare Ergebnisse:** M0 und M1 liefern schnell etwas Vorzeigbares. Das hilft, Mitstreiter zu finden.

---

## 14. Offene Entscheidungen

**Bereits entschieden**

- **Grenze des Eigenbaus:** Rust-Standardbibliothek sowie Betriebssystem- und Grafikschnittstellen sind erlaubt; Browserkern und Engine sind eigen. Die optionale Widevine-Anbindung und die getrennten Fuzzing-Werkzeuge bilden die begrenzten Ausnahmen aus [1.1](#11-was-komplett-eigen-hier-bedeutet).
- **Kryptografie und TLS:** selbst geschrieben, mit Testvektoren, Fuzzing und externem Audit ([4.11](#411-netzwerk-und-kryptografie)).
- **Fuzzing-Werkzeug (30.10.2026):** `cargo-fuzz` mit libFuzzer im getrennten Workspace `fuzz/`, mit `libfuzzer-sys` als einziger Ausnahme von „keine fremden Crates“ ([8.2](#82-schutzschichten), Punkt 14). Zuvor (PR #40) war ein eigener Fuzzer ohne Ausnahme vorgesehen.
- **JavaScript-Engine (31.10.2026):** Werte mit NaN-Boxing und Int32-Schnellweg, Bytecode für eine Registermaschine mit Akkumulator, echte Endaufrufe nach Abschnitt 15.10 von ECMA-262, Heap und Garbage Collector bis zum JIT ohne `unsafe`, Test262 mit `cargo test` in `mb-js`. Bereiche und Handles tragen die Nummer ihres Heaps, eine Verwechslung löst eine Panic aus; Werte tragen keine, eine Marke im Typ wird vor dem JIT erneut geprüft. Differentielle Tests vergleichen vorerst nur eigene Wege (normal, Belastungsmodus, Zwischenspeicher für Bytecode), keine fremde Engine ([4.9](#49-javascript-und-webassembly)).
- **DRM und Widevine:** eigene EME-Schnittstelle und eigener CDM-Host, mit optionalem externen Widevine-CDM; standardmäßig deaktiviert, nur nach Zustimmung und in einem eigenen eingeschränkten Prozess ([4.13](#413-bilder-audio-und-video)).
- **Rust-Edition:** Edition 2024 ist im Workspace eingestellt; die Mindestversion des Compilers bleibt offen.

**Noch offen**

1. **Werkzeuge, Testsammlungen und Datensätze:** Gelten sie als erlaubt, wie in [1.1](#11-was-komplett-eigen-hier-bedeutet) angenommen? Die getrennten Fuzzing-Werkzeuge sind bereits ausdrücklich erlaubt.
2. **Patentbelastete Codecs** (H.264, AAC, HEVC): über die Medienschnittstellen des Systems nutzen oder weglassen?
3. **Widevine-Einbindung:** Welche rechtmäßigen Bezugs- und Weitergabemöglichkeiten, Anbietervereinbarungen und gegebenenfalls Zertifizierungen stehen zur Verfügung? Welche Plattformen, CDM-Schnittstellen, Codecs und Streamingdienste lassen sich nachweislich unterstützen? Die optionale Anbindung als Grundsatz ist entschieden.
4. **Erweiterungen aus den Stores:** Wie sieht die Installation direkt aus dem Chrome Web Store und von addons.mozilla.org aus, mit Blick auf deren Nutzungsbedingungen? Prüfen wir die Signaturen der Stores?
5. **Rust-Version:** eine festgelegte Mindestversion des Compilers (MSRV).
6. **Mitwirkung:** Beitragsregeln (zum Beispiel Developer Certificate of Origin), Code-Review-Regeln, Umgang mit `unsafe`.
7. **Telemetrie:** keine, oder nur freiwillig?
8. **Schutzlisten gegen Phishing und Schadsoftware:** Google Safe Browsing hat eigene Nutzungsbedingungen. Welche Listen nutzen wir, und wie bleibt dabei die Privatsphäre gewahrt?
9. **Widerruf von Zertifikaten und Certificate Transparency:** eigenes Verfahren nach dem Vorbild von CRLite (Firefox) oder CRLSets (Chrome), oder Sperrlisten der Aussteller direkt? Woher kommt die Liste der CT-Logs?
10. **Partitionierung als Standard:** Speicher und Cookies standardmäßig nach oberster Website trennen ([8.2](#82-schutzschichten), Punkt 12)?

---

## 15. Quellen

**Spezifikationen und Testsammlungen**

- WHATWG HTML: <https://html.spec.whatwg.org/>
- WHATWG DOM: <https://dom.spec.whatwg.org/>
- WHATWG URL: <https://url.spec.whatwg.org/>
- WHATWG Encoding: <https://encoding.spec.whatwg.org/>
- WHATWG Fetch: <https://fetch.spec.whatwg.org/>
- WHATWG Web IDL: <https://webidl.spec.whatwg.org/>
- WHATWG Compatibility: <https://compat.spec.whatwg.org/>
- ECMAScript (ECMA-262): <https://tc39.es/ecma262/>
- ECMAScript Internationalization API (ECMA-402): <https://tc39.es/ecma402/>
- CSS-Spezifikationen des W3C: <https://www.w3.org/Style/CSS/>
- WebAssembly: <https://webassembly.github.io/spec/>
- web-platform-tests: <https://web-platform-tests.org/>, Ergebnisse: <https://wpt.fyi/>
- Test262: <https://github.com/tc39/test262>
- html5lib-tests: <https://github.com/html5lib/html5lib-tests>
- css-parsing-tests: <https://github.com/SimonSapin/css-parsing-tests>
- webref: <https://github.com/w3c/webref>
- W3C WebExtensions Community Group: <https://www.w3.org/community/webextensions/>
- Chrome-Erweiterungen: <https://developer.chrome.com/docs/extensions>
- Firefox-Erweiterungen (MDN): <https://developer.mozilla.org/docs/Mozilla/Add-ons/WebExtensions>

**Zum Vergleich mit Ladybird ([Abschnitt 13](#13-der-aufwand-ehrlich-betrachtet))**

- Ladybird, Startseite und FAQ (eigene Engine, kleines Vollzeit-Team, Fremdbibliotheken für Medienformate, Verschlüsselung und Grafik): <https://ladybird.org/>
- Ankündigung vom 1. Juli 2024 (Ursprung als HTML-Betrachter in SerenityOS): <https://ladybird.org/posts/announcement/>
- Wikipedia (geplant: Alpha 2026, Beta 2027, stabile Version 2028): <https://en.wikipedia.org/wiki/Ladybird_(web_browser)>
- Zeitleiste ab Juni 2019 (Dokumentationskopie eines Drittanbieters): <https://mintlify.wiki/LadybirdBrowser/ladybird/faq>
