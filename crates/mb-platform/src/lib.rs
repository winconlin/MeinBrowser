//! Die einzige Stelle, die mit Betriebssystem und Grafiktreibern spricht: Fenster, Eingabe, Ein-/Ausgabe, GPU-Oberfläche, Sandbox. Die FFI-Deklarationen sind selbst geschrieben.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.2.
//! `unsafe` ist hier für die FFI erlaubt, siehe Abschnitt 4.14.
