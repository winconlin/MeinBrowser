//! Führt ein [`Program`] auf einer Eingabe aus: eine Maschine mit
//! Rückverfolgung, die dieselben Treffer findet wie die Abschlüsse aus
//! Abschnitt 22.2.2.
//!
//! - Die Maschine ruft sich nie selbst auf (RS-8). Offene Wahlen und alles,
//!   was beim Scheitern zurückzunehmen ist, liegen auf einem eigenen Stapel,
//!   den [`Budget::bytes`] begrenzt.
//! - Stellen zählen in Codeeinheiten. Mit `u` oder `v` liest die Maschine
//!   Codepunkte und steht deshalb nie zwischen den Hälften eines
//!   Ersatzpaars; einzelne Surrogate sind Zeichen wie alle anderen (RS-9).
//! - Jeder Befehl und jeder verworfene Eintrag des Stapels ist eine Einheit
//!   Arbeit für [`Budget::spend`]. Ein Muster wie `(a*)*b` läuft damit nicht
//!   endlos, sondern in die Arbeitsquote.

use super::charset::{self, FOLDED_WORD, is_ascii_word, is_line_terminator};
use super::program::{Budget, Instruction, NONE, Program, Step, WORK_BATCH};
use super::to_usize;

/// Die Eingabe eines Abgleichs: UTF-16-Codeeinheiten.
pub(crate) trait Input {
    fn len(&self) -> usize;
    fn unit(&self, index: usize) -> Option<u16>;
}

impl Input for [u16] {
    fn len(&self) -> usize {
        <[u16]>::len(self)
    }

    fn unit(&self, index: usize) -> Option<u16> {
        self.get(index).copied()
    }
}

/// Warum ein Abgleich ohne Ergebnis endete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// Der Stapel der Rückverfolgung wäre größer als erlaubt, oder die
    /// Eingabe ist zu lang für Stellen in 32 Bit.
    Memory,
    /// [`Budget::spend`] hat abgebrochen.
    Stopped,
}

/// Ein Bereich der Eingabe in Codeeinheiten: Anfang und Ende.
pub(crate) type Span = (u32, u32);

/// Sucht den ersten Treffer ab `last_index` (die Schleife aus
/// RegExpBuiltinExec, Abschnitt 22.2.7.2), mit `sticky` nur genau dort.
/// Liefert die Bereiche aller Gruppen; die nullte ist der ganze Treffer.
///
/// Mit `u` oder `v` beginnt ein Versuch am Anfang des Zeichens, in dem
/// `last_index` liegt. Der Treffer beginnt trotzdem bei `last_index`, so
/// steht es in RegExpBuiltinExec: Die Stelle für das Ergebnis ist
/// `lastIndex`, nicht der Anfang des Zeichens.
pub(crate) fn execute<I: Input + ?Sized>(
    program: &Program,
    input: &I,
    last_index: usize,
    sticky: bool,
    budget: Budget<'_>,
) -> Result<Option<Vec<Option<Span>>>, Stop> {
    let length = u32::try_from(input.len()).map_err(|_| Stop::Memory)?;
    if length == NONE {
        return Err(Stop::Memory);
    }
    let Ok(mut last_index) = u32::try_from(last_index) else {
        return Ok(None);
    };
    let groups = to_usize(program.groups).saturating_add(1);
    let mut machine = Machine {
        program,
        input,
        length,
        registers: vec![0; to_usize(program.registers)],
        captures: vec![None; groups],
        stack: Vec::new(),
        kept: Vec::new(),
        // Ein Vec verdoppelt seinen Platz, wenn er wächst.
        frames: budget.bytes / size_of::<Frame>() / 2,
        pending: 0,
        budget,
    };
    let found = loop {
        if last_index > length {
            break false;
        }
        if machine.attempt(machine.character_start(last_index))? {
            // Der Treffer beginnt für RegExpBuiltinExec bei `lastIndex`,
            // auch wenn der Versuch davor begann.
            if let Some(Some(whole)) = machine.captures.first_mut() {
                whole.0 = last_index;
            }
            break true;
        }
        if sticky {
            break false;
        }
        last_index = machine.advance(last_index);
    };
    if machine.pending > 0 && !(machine.budget.spend)(machine.pending) {
        return Err(Stop::Stopped);
    }
    Ok(found.then_some(machine.captures))
}

/// Ein Eintrag auf dem Stapel der Rückverfolgung.
#[derive(Clone, Copy)]
enum Frame {
    /// Eine offene Wahl: Scheitert der Weg davor, geht es hier weiter.
    Choice { pc: u32, position: u32 },
    /// Der alte Wert eines Registers.
    Register { register: u32, value: u32 },
    /// Der alte Wert einer Gruppe.
    Capture { group: u32, value: Option<Span> },
    /// Der Anfang eines Lookaround: die Stelle davor und der Befehl
    /// dahinter. Wahlen über diesem Eintrag gehören zum Rumpf.
    Look {
        negative: bool,
        pc: u32,
        position: u32,
    },
}

struct Machine<'a, 'b, I: Input + ?Sized> {
    program: &'a Program,
    input: &'a I,
    length: u32,
    registers: Vec<u32>,
    captures: Vec<Option<Span>>,
    stack: Vec<Frame>,
    /// Zwischenablage für [`Machine::look_end`].
    kept: Vec<Frame>,
    /// So viele Einträge darf der Stapel haben.
    frames: usize,
    /// Arbeit, die [`Budget::spend`] noch nicht gesehen hat.
    pending: u64,
    budget: Budget<'b>,
}

impl<I: Input + ?Sized> Machine<'_, '_, I> {
    fn work(&mut self, work: u64) -> Result<(), Stop> {
        self.pending = self.pending.saturating_add(work);
        if self.pending >= WORK_BATCH {
            let pending = std::mem::take(&mut self.pending);
            if !(self.budget.spend)(pending) {
                return Err(Stop::Stopped);
            }
        }
        Ok(())
    }

    fn unit(&self, index: u32) -> Option<u16> {
        self.input.unit(to_usize(index))
    }

    /// Das Zeichen ab `position` oder, rückwärts, das davor, und die Stelle
    /// dahinter. Nichts am Rand der Eingabe.
    fn read(&self, position: u32, backward: bool) -> Option<(u32, u32)> {
        if backward {
            let index = position.checked_sub(1)?;
            let unit = self.unit(index)?;
            if self.program.unicode && is_low_surrogate(unit) {
                let before = index.checked_sub(1).and_then(|before| {
                    let high = self.unit(before)?;
                    is_high_surrogate(high).then(|| (code_point(high, unit), before))
                });
                if before.is_some() {
                    return before;
                }
            }
            return Some((u32::from(unit), index));
        }
        let unit = self.unit(position)?;
        let next = position.saturating_add(1);
        if self.program.unicode
            && is_high_surrogate(unit)
            && let Some(low) = self.unit(next).filter(|&low| is_low_surrogate(low))
        {
            return Some((code_point(unit, low), next.saturating_add(1)));
        }
        Some((u32::from(unit), next))
    }

    /// Der Anfang des Zeichens, in dem `index` liegt.
    fn character_start(&self, index: u32) -> u32 {
        let inside = self.program.unicode
            && self.unit(index).is_some_and(is_low_surrogate)
            && index
                .checked_sub(1)
                .and_then(|before| self.unit(before))
                .is_some_and(is_high_surrogate);
        if inside {
            index.saturating_sub(1)
        } else {
            index
        }
    }

    /// AdvanceStringIndex (Abschnitt 22.2.7.3)
    fn advance(&self, index: u32) -> u32 {
        // Ohne `u` und `v` liest `read` einzelne Codeeinheiten.
        self.read(index, false)
            .map_or(index.saturating_add(1), |(_, next)| next)
    }

    fn push(&mut self, frame: Frame) -> Result<(), Stop> {
        if self.stack.len() >= self.frames {
            return Err(Stop::Memory);
        }
        self.stack.push(frame);
        Ok(())
    }

    /// Setzt ein Register. Den alten Wert braucht nur, wer zu einer Wahl
    /// zurückkehren kann.
    fn set_register(&mut self, register: u32, new: u32) -> Result<(), Stop> {
        let Some(&value) = self.registers.get(to_usize(register)) else {
            return Ok(());
        };
        if !self.stack.is_empty() {
            self.push(Frame::Register { register, value })?;
        }
        if let Some(slot) = self.registers.get_mut(to_usize(register)) {
            *slot = new;
        }
        Ok(())
    }

    fn register(&self, register: u32) -> u32 {
        self.registers.get(to_usize(register)).copied().unwrap_or(0)
    }

    fn set_capture(&mut self, group: u32, new: Option<Span>) -> Result<(), Stop> {
        let Some(&value) = self.captures.get(to_usize(group)) else {
            return Ok(());
        };
        if value == new {
            return Ok(());
        }
        if !self.stack.is_empty() {
            self.push(Frame::Capture { group, value })?;
        }
        if let Some(slot) = self.captures.get_mut(to_usize(group)) {
            *slot = new;
        }
        Ok(())
    }

    fn capture(&self, group: u32) -> Option<Span> {
        self.captures.get(to_usize(group)).copied().flatten()
    }

    /// Nimmt einen Eintrag vom Stapel zurück, der keine Wahl ist.
    fn undo(&mut self, frame: Frame) {
        match frame {
            Frame::Register { register, value } => {
                if let Some(slot) = self.registers.get_mut(to_usize(register)) {
                    *slot = value;
                }
            }
            Frame::Capture { group, value } => {
                if let Some(slot) = self.captures.get_mut(to_usize(group)) {
                    *slot = value;
                }
            }
            Frame::Choice { .. } | Frame::Look { .. } => {}
        }
    }

    /// Der Weg ist gescheitert: zurück zur jüngsten Wahl. Liefert, wo es
    /// weitergeht, oder nichts, wenn keine Wahl mehr offen ist.
    fn fail(&mut self) -> Result<Option<(u32, u32)>, Stop> {
        while let Some(frame) = self.stack.pop() {
            self.work(1)?;
            match frame {
                Frame::Choice { pc, position }
                // Der Rumpf eines verneinten Lookaround passt nirgends: Der
                // Lookaround selbst passt, und es bleibt nichts von ihm.
                | Frame::Look {
                    negative: true,
                    pc,
                    position,
                } => return Ok(Some((pc, position))),
                // Der Rumpf passt nirgends, also scheitert auch der Weg
                // davor.
                Frame::Look {
                    negative: false, ..
                } => {}
                undo => self.undo(undo),
            }
        }
        Ok(None)
    }

    /// Der Rumpf eines Lookaround hat gepasst. Verneint scheitert damit der
    /// Lookaround, und alles aus dem Rumpf wird zurückgenommen. Sonst geht
    /// es an der Stelle vor dem Lookaround weiter: Die Wahlen aus dem Rumpf
    /// verfallen, denn in einen Lookaround kehrt die Rückverfolgung nicht
    /// zurück; seine Gruppen bleiben, bis der Weg davor scheitert.
    fn look_end(&mut self, negative: bool) -> Result<Option<u32>, Stop> {
        if negative {
            while let Some(frame) = self.stack.pop() {
                self.work(1)?;
                if matches!(frame, Frame::Look { .. }) {
                    break;
                }
                self.undo(frame);
            }
            return Ok(None);
        }
        let is_look = |frame: &Frame| matches!(frame, Frame::Look { .. });
        let Some(start) = self.stack.iter().rposition(is_look) else {
            return Ok(None);
        };
        let Some(&Frame::Look { position, .. }) = self.stack.get(start) else {
            return Ok(None);
        };
        let above = self.stack.len().saturating_sub(start);
        self.work(u64::try_from(above).unwrap_or(u64::MAX))?;
        let undo = |frame: &Frame| matches!(frame, Frame::Register { .. } | Frame::Capture { .. });
        self.kept.extend(self.stack.drain(start..).filter(undo));
        self.stack.append(&mut self.kept);
        Ok(Some(position))
    }

    /// BackreferenceMatcher (Abschnitt 22.2.2.8): Vergleicht, was die
    /// Gruppe erfasst hat, Zeichen für Zeichen mit der Eingabe ab
    /// `position`. Liefert die Stelle dahinter.
    fn back_reference(
        &mut self,
        span: Option<Span>,
        position: u32,
        step: Step,
    ) -> Result<Option<u32>, Stop> {
        let Some((start, end)) = span else {
            return Ok(Some(position));
        };
        self.work(u64::from(end.saturating_sub(start)))?;
        // Rückwärts läuft der Vergleich vom Ende der Gruppe zum Anfang.
        let (mut index, stop) = if step.backward {
            (end, start)
        } else {
            (start, end)
        };
        let mut at = position;
        while index != stop && (index < stop) != step.backward {
            let (Some((expected, next_index)), Some((found, next_at))) = (
                self.read(index, step.backward),
                self.read(at, step.backward),
            ) else {
                return Ok(None);
            };
            if step.folding.canonicalize(expected) != step.folding.canonicalize(found) {
                return Ok(None);
            }
            index = next_index;
            at = next_at;
        }
        Ok(Some(at))
    }

    /// Ob an `index` ein Wortzeichen steht (IsWordChar, Abschnitt
    /// 22.2.2.4.2).
    fn is_word(&self, index: Option<u32>, folded: bool) -> bool {
        index
            .and_then(|index| self.unit(index))
            .map(u32::from)
            .is_some_and(|unit| is_ascii_word(unit) || (folded && FOLDED_WORD.contains(&unit)))
    }

    fn is_line_terminator(&self, index: Option<u32>) -> bool {
        index
            .and_then(|index| self.unit(index))
            .is_some_and(|unit| is_line_terminator(u32::from(unit)))
    }

    /// Ein Versuch ab `start`. Bei einem Treffer stehen die Gruppen in
    /// `captures`.
    fn attempt(&mut self, start: u32) -> Result<bool, Stop> {
        self.stack.clear();
        self.captures.fill(None);
        self.registers.fill(0);
        let program = self.program;
        let mut pc = 0_u32;
        let mut position = start;
        loop {
            self.work(1)?;
            let Some(&instruction) = program.instructions.get(to_usize(pc)) else {
                return Ok(false);
            };
            // Wo es weitergeht, wenn der Befehl passt; nichts, wenn er
            // scheitert.
            let mut next = Some(pc.saturating_add(1));
            match instruction {
                Instruction::Character { code, step } => match self.read(position, step.backward) {
                    Some((character, after)) if step.folding.canonicalize(character) == code => {
                        position = after;
                    }
                    _ => next = None,
                },
                Instruction::Set { set, invert, step } => {
                    let ranges = program
                        .sets
                        .get(to_usize(set))
                        .map_or(&[][..], Vec::as_slice);
                    match self.read(position, step.backward) {
                        Some((character, after))
                            if charset::contains(ranges, step.folding.canonicalize(character))
                                != invert =>
                        {
                            position = after;
                        }
                        _ => next = None,
                    }
                }
                Instruction::Any {
                    line_terminators,
                    backward,
                } => match self.read(position, backward) {
                    Some((character, after))
                        if line_terminators || !is_line_terminator(character) =>
                    {
                        position = after;
                    }
                    _ => next = None,
                },
                Instruction::Split { first, second } => {
                    self.push(Frame::Choice {
                        pc: second,
                        position,
                    })?;
                    next = Some(first);
                }
                Instruction::Jump { target } => next = Some(target),
                Instruction::Position { register } => self.set_register(register, position)?,
                Instruction::Capture { group, register } => {
                    let other = self.register(register);
                    let span = (other.min(position), other.max(position));
                    self.set_capture(group, Some(span))?;
                }
                Instruction::ResetCaptures { first, count } => {
                    for group in first..first.saturating_add(count) {
                        self.set_capture(group, None)?;
                    }
                    self.work(u64::from(count))?;
                }
                Instruction::SetCounter { register } => self.set_register(register, 0)?,
                Instruction::Increment { register } => {
                    let value = self.register(register).saturating_add(1);
                    self.set_register(register, value)?;
                }
                Instruction::JumpIfBelow {
                    register,
                    value,
                    target,
                } => {
                    if self.register(register) < value {
                        next = Some(target);
                    }
                }
                Instruction::JumpIfReached {
                    register,
                    value,
                    target,
                } => {
                    if self.register(register) >= value {
                        next = Some(target);
                    }
                }
                Instruction::EmptyCheck {
                    position: register,
                    counter,
                    min,
                } => {
                    let needed = counter != NONE && self.register(counter) < min;
                    if self.register(register) == position && !needed {
                        next = None;
                    }
                }
                Instruction::LineStart { multiline } => {
                    let before = position.checked_sub(1);
                    if !(position == 0 || (multiline && self.is_line_terminator(before))) {
                        next = None;
                    }
                }
                Instruction::LineEnd { multiline } => {
                    let here = Some(position);
                    if !(position == self.length || (multiline && self.is_line_terminator(here))) {
                        next = None;
                    }
                }
                Instruction::WordBoundary { negated, folded } => {
                    let before = self.is_word(position.checked_sub(1), folded);
                    let after = self.is_word(Some(position), folded);
                    if (before != after) == negated {
                        next = None;
                    }
                }
                Instruction::BackReference { group, step } => {
                    let span = self.capture(group);
                    match self.back_reference(span, position, step)? {
                        Some(after) => position = after,
                        None => next = None,
                    }
                }
                Instruction::NamedBackReference { list, step } => {
                    // Von Gruppen mit gleichem Namen ist höchstens eine
                    // definiert.
                    let groups = program.group_lists.get(to_usize(list));
                    let span = groups
                        .into_iter()
                        .flatten()
                        .find_map(|&group| self.capture(group));
                    match self.back_reference(span, position, step)? {
                        Some(after) => position = after,
                        None => next = None,
                    }
                }
                Instruction::LookStart { negative, end } => self.push(Frame::Look {
                    negative,
                    pc: end,
                    position,
                })?,
                Instruction::LookEnd { negative } => match self.look_end(negative)? {
                    Some(before) => position = before,
                    None => next = None,
                },
                Instruction::Match => {
                    if let Some(whole) = self.captures.first_mut() {
                        *whole = Some((start, position));
                    }
                    return Ok(true);
                }
            }
            match next {
                Some(next) => pc = next,
                None => match self.fail()? {
                    Some((resume, at)) => {
                        pc = resume;
                        position = at;
                    }
                    None => return Ok(false),
                },
            }
        }
    }
}

fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

/// Der Codepunkt eines Ersatzpaars.
fn code_point(high: u16, low: u16) -> u32 {
    let high = u32::from(high).saturating_sub(0xD800);
    let low = u32::from(low).saturating_sub(0xDC00);
    (high << 10).saturating_add(low).saturating_add(0x1_0000)
}

#[cfg(test)]
mod tests;
