//! Das Programm, zu dem [`super::compile`] ein Muster übersetzt und das
//! [`super::matcher`] ausführt: eine Folge von Befehlen für eine Maschine
//! mit Rückverfolgung.
//!
//! Die Maschine hält eine Stelle in der Eingabe, die Gruppen und eine Reihe
//! von Registern. Ein Register merkt sich eine Stelle (den Anfang einer
//! Gruppe oder eines Durchlaufs) oder zählt die Durchläufe eines Quantors.
//! Jeder Befehl passt und macht beim nächsten weiter, oder er scheitert;
//! dann geht es bei der jüngsten offenen Wahl aus [`Instruction::Split`]
//! weiter, und alles, was seither an Gruppen und Registern geändert wurde,
//! gilt wieder wie vorher.

use super::charset::{Folding, Range};

/// Steht für „kein Register“ und für ein Sprungziel, das noch fehlt.
pub(crate) const NONE: u32 = u32::MAX;

/// Was das Übersetzen oder ein Abgleich verbrauchen darf.
pub(crate) struct Budget<'a> {
    /// Wie viele Bytes das Programm oder der Stapel der Rückverfolgung
    /// belegen darf.
    pub(crate) bytes: usize,
    /// Nimmt die Arbeit entgegen, die seit dem letzten Aufruf anfiel: beim
    /// Übersetzen je Knoten und je Bereich einer Menge eine Einheit, beim
    /// Abgleich je Befehl und je verworfenem Eintrag des Stapels. Mit
    /// `false` bricht der Aufrufer ab.
    pub(crate) spend: &'a mut dyn FnMut(u64) -> bool,
}

/// So viel Arbeit sammelt sich an, bevor [`Budget::spend`] gefragt wird.
pub(crate) const WORK_BATCH: u64 = 1024;

/// Wie ein Befehl Zeichen liest und vergleicht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    /// In einem Lookbehind liest die Maschine rückwärts.
    pub(crate) backward: bool,
    pub(crate) folding: Folding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Instruction {
    /// Ein Zeichen, dessen kanonische Form `code` ist.
    Character {
        code: u32,
        step: Step,
    },
    /// Ein Zeichen, dessen kanonische Form in der Menge `set` liegt, mit
    /// `invert` eines, für das das nicht gilt.
    Set {
        set: u32,
        invert: bool,
        step: Step,
    },
    /// Ein beliebiges Zeichen, ohne `line_terminators` kein Zeilenende.
    Any {
        line_terminators: bool,
        backward: bool,
    },
    /// Macht bei `first` weiter und merkt `second` als Wahl vor.
    Split {
        first: u32,
        second: u32,
    },
    Jump {
        target: u32,
    },
    /// Merkt die Stelle im Register.
    Position {
        register: u32,
    },
    /// Setzt die Gruppe auf den Bereich zwischen der Stelle im Register
    /// und der jetzigen.
    Capture {
        group: u32,
        register: u32,
    },
    /// Macht `count` Gruppen ab `first` wieder undefiniert.
    ResetCaptures {
        first: u32,
        count: u32,
    },
    /// Setzt einen Zähler auf 0.
    SetCounter {
        register: u32,
    },
    Increment {
        register: u32,
    },
    /// Springt, solange der Zähler unter `value` liegt.
    JumpIfBelow {
        register: u32,
        value: u32,
        target: u32,
    },
    /// Springt, sobald der Zähler `value` erreicht hat.
    JumpIfReached {
        register: u32,
        value: u32,
        target: u32,
    },
    /// Scheitert, wenn ein Durchlauf nichts verbraucht hat, obwohl er nicht
    /// mehr nötig war (RepeatMatcher, Abschnitt 22.2.2.3.1): Die Stelle ist
    /// die aus `position`, und der Zähler hat `min` erreicht. Ohne Zähler
    /// ([`NONE`]) war kein Durchlauf nötig.
    EmptyCheck {
        position: u32,
        counter: u32,
        min: u32,
    },
    /// `^`
    LineStart {
        multiline: bool,
    },
    /// `$`
    LineEnd {
        multiline: bool,
    },
    /// `\b` und mit `negated` `\B`. Mit `folded` zählen auch die Zeichen,
    /// die die einfache Faltung auf ein Wortzeichen abbildet.
    WordBoundary {
        negated: bool,
        folded: bool,
    },
    /// Was die Gruppe zuletzt erfasst hat; nichts, wenn sie undefiniert
    /// ist.
    BackReference {
        group: u32,
        step: Step,
    },
    /// Wie [`Instruction::BackReference`] für die eine definierte Gruppe
    /// aus der Liste `list`: `\k<name>`, wenn mehrere Gruppen so heißen.
    NamedBackReference {
        list: u32,
        step: Step,
    },
    /// Der Anfang eines Lookaround. Sein Rumpf endet mit
    /// [`Instruction::LookEnd`], `end` ist der Befehl danach.
    LookStart {
        negative: bool,
        end: u32,
    },
    LookEnd {
        negative: bool,
    },
    /// Das Muster passt.
    Match,
}

/// Ein übersetztes Muster.
#[derive(Debug)]
pub(crate) struct Program {
    pub(crate) instructions: Vec<Instruction>,
    /// Die Mengen für [`Instruction::Set`], schon in kanonischer Form.
    pub(crate) sets: Vec<Vec<Range>>,
    /// Die Listen für [`Instruction::NamedBackReference`].
    pub(crate) group_lists: Vec<Vec<u32>>,
    /// Die Zahl der Gruppen ohne die nullte für den ganzen Treffer.
    pub(crate) groups: u32,
    /// Der Name jeder Gruppe, Gruppe 1 an der Stelle 0.
    pub(crate) names: Vec<Option<Vec<u16>>>,
    pub(crate) registers: u32,
    /// `u` oder `v`: Zeichen sind Codepunkte.
    pub(crate) unicode: bool,
}

impl Program {
    /// Wie viele Bytes das Programm ungefähr belegt.
    pub(crate) fn bytes(&self) -> usize {
        let instructions = self
            .instructions
            .len()
            .saturating_mul(size_of::<Instruction>());
        let sets: usize = self
            .sets
            .iter()
            .map(|set| set.len().saturating_mul(size_of::<Range>()))
            .sum();
        let lists: usize = self
            .group_lists
            .iter()
            .map(|list| list.len().saturating_mul(size_of::<u32>()))
            .sum();
        let names: usize = self
            .names
            .iter()
            .map(|name| name.as_ref().map_or(0, |name| name.len().saturating_mul(2)))
            .sum();
        instructions
            .saturating_add(sets)
            .saturating_add(lists)
            .saturating_add(names)
    }
}
