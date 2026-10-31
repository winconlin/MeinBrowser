//! Bytecode: eine Registermaschine mit Akkumulator (`docs/architektur.md`,
//! Abschnitt 4.9, und der Entwurf „Bytecode und Interpreter für mb-js“).
//!
//! Ein Befehl ist ein Byte für den Opcode und seine Operanden. Ohne Präfix
//! ist jeder Operand ein Byte breit, nach [`Op::Wide`] zwei und nach
//! [`Op::ExtraWide`] vier Bytes, jeweils in der Reihenfolge Little Endian.
//! Register und Konstanten zählen ohne Vorzeichen, Zahlen im Befehl und
//! Sprungweiten mit Vorzeichen. Ein Sprung zählt ab dem ersten Byte seines
//! Befehls, ein Präfix eingeschlossen. Sprünge nach vorn haben immer vier
//! Bytes, damit der Übersetzer sie nachtragen kann; nur [`Op::JumpLoop`]
//! springt zurück.
//!
//! Register sind Plätze im Rahmen: erst `this`, dann bei Scripts der
//! Abschlusswert und bei Funktionen das Funktionsobjekt selbst und die
//! Parameter, dann Bindungen und Zwischenwerte. Der Akkumulator ist
//! implizit. Ein zweistelliger Befehl wie [`Op::Add`] rechnet mit dem
//! Register als linkem und dem Akkumulator als rechtem Operanden und legt
//! das Ergebnis in den Akkumulator.
//!
//! Im Bytecode steht keine Adresse im Heap. Zahlen, Zeichenketten und die
//! Funktionen, die der Code anlegt, liegen als [`Constant`] bei ihm; der
//! Interpreter legt sie im Heap an, für ein Script zu Beginn des Laufs, für
//! eine Funktion einmal bei ihrem ersten Aufruf.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt;
use std::rc::Rc;

use crate::ast::{Atom, FunctionId};

/// Die Art eines Operanden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    Register,
    Constant,
    /// Eine ganze Zahl mit Vorzeichen.
    Immediate,
    /// Eine Sprungweite mit Vorzeichen.
    Jump,
}

macro_rules! ops {
    ($($(#[$doc:meta])* $name:ident [$($operand:ident),*],)*) => {
        /// Ein Opcode.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Op {
            $($(#[$doc])* $name,)*
        }

        impl Op {
            /// Alle Opcodes in der Reihenfolge ihrer Bytes.
            pub const ALL: &[Op] = &[$(Op::$name,)*];

            /// Die Operanden des Befehls.
            pub fn operands(self) -> &'static [Operand] {
                match self {
                    $(Op::$name => &[$(Operand::$operand),*],)*
                }
            }
        }
    };
}

ops! {
    /// Präfix: Die Operanden des nächsten Befehls sind zwei Bytes breit.
    Wide [],
    /// Präfix: Die Operanden des nächsten Befehls sind vier Bytes breit.
    ExtraWide [],

    LdaUndefined [],
    LdaNull [],
    LdaTrue [],
    LdaFalse [],
    /// Die Lücke, die Marke für Bindungen vor ihrer Initialisierung.
    LdaHole [],
    /// Eine ganze Zahl aus dem Befehl.
    LdaSmi [Immediate],
    LdaConstant [Constant],
    /// Akkumulator := Register.
    Ldar [Register],
    /// Register := Akkumulator.
    Star [Register],
    /// Zweites Register := erstes Register.
    Mov [Register, Register],

    /// Wirft den ReferenceError der TDZ, wenn das Register die Lücke hält.
    /// Die Konstante nennt den Namen.
    CheckHole [Register, Constant],
    /// Wirft den TypeError für eine Zuweisung an eine Konstante.
    ThrowConstAssignment [Constant],

    /// GetIdentifierReference (Abschnitt 9.1.2.1) in der globalen Umgebung:
    /// legt ab, wo der Name gefunden wurde, für [`Op::LdaGlobalRef`] und
    /// [`Op::StaGlobal`].
    ResolveGlobal [Constant, Register],
    /// Löst den Namen auf und liest ihn (GetValue).
    LdaGlobal [Constant],
    /// GetValue für einen Namen, den [`Op::ResolveGlobal`] aufgelöst hat.
    LdaGlobalRef [Constant, Register],
    /// PutValue mit dem Akkumulator für einen aufgelösten Namen.
    StaGlobal [Constant, Register],
    /// InitializeBinding einer lexikalischen Bindung der globalen Umgebung.
    InitGlobal [Constant],
    /// `typeof` für einen Namen der globalen Umgebung, auch einen
    /// unbekannten.
    TypeofGlobal [Constant],
    /// `delete` für einen Namen der globalen Umgebung.
    DeleteGlobal [Constant],

    /// Ein neues gewöhnliches Objekt mit %Object.prototype%.
    CreateObject [],
    /// Ein neues Array mit der Länge 0 und %Array.prototype%.
    CreateArray [],
    /// CreateDataPropertyOrThrow für das Objekt im Register mit dem Namen
    /// und dem Akkumulator als Wert.
    DefineNamed [Register, Constant],
    /// Dasselbe mit dem Schlüssel aus dem zweiten Register.
    DefineKeyed [Register, Register],
    /// `__proto__: Wert` in einem Objektliteral, der Wert im Akkumulator.
    SetPrototypeLiteral [Register],
    /// `...Wert` in einem Objektliteral (CopyDataProperties).
    CopyDataProperties [Register],
    /// Ein Getter im Objekt im ersten Register für den Schlüssel im
    /// zweiten, die Funktion im Akkumulator.
    DefineGetter [Register, Register],
    /// Dasselbe für einen Setter.
    DefineSetter [Register, Register],
    /// Ein Element eines Array-Literals: der Akkumulator an der Stelle der
    /// Länge.
    ArrayPush [Register],
    /// Eine Lücke in einem Array-Literal: die Länge um eins größer.
    ArrayHole [Register],
    /// Akkumulator := ToPropertyKey(Akkumulator).
    ToPropertyKey [],

    /// GetValue für die Eigenschaft mit dem Namen auf dem Akkumulator.
    LdaNamed [Constant],
    /// GetValue für die Eigenschaft mit dem Schlüssel im Akkumulator auf dem
    /// Register.
    LdaKeyed [Register],
    /// GetValue für die Eigenschaft mit dem Schlüssel im zweiten Register auf
    /// dem ersten. Danach hält das zweite den Schlüssel nach ToPropertyKey,
    /// wie die Referenz in GetValue.
    LdaKeyedRef [Register, Register],
    /// PutValue mit dem Akkumulator für die Eigenschaft mit dem Namen auf dem
    /// Register.
    StaNamed [Register, Constant],
    /// PutValue mit dem Akkumulator für die Eigenschaft mit dem Schlüssel im
    /// zweiten Register auf dem ersten.
    StaKeyed [Register, Register],
    /// `delete` für die Eigenschaft mit dem Namen auf dem Akkumulator.
    DeleteNamed [Constant],
    /// `delete` für die Eigenschaft mit dem Schlüssel im Akkumulator auf dem
    /// Register.
    DeleteKeyed [Register],

    Add [Register],
    Sub [Register],
    Mul [Register],
    Div [Register],
    Mod [Register],
    Exp [Register],
    BitwiseAnd [Register],
    BitwiseOr [Register],
    BitwiseXor [Register],
    ShiftLeft [Register],
    ShiftRight [Register],
    ShiftRightLogical [Register],

    /// `==`
    TestEqual [Register],
    /// `===`
    TestEqualStrict [Register],
    /// `<`
    TestLessThan [Register],
    /// `>`
    TestGreaterThan [Register],
    /// `<=`
    TestLessThanOrEqual [Register],
    /// `>=`
    TestGreaterThanOrEqual [Register],
    /// `in`: der Schlüssel im Register, das Objekt im Akkumulator.
    TestIn [Register],
    /// `instanceof`: der Wert im Register, das Ziel im Akkumulator.
    TestInstanceOf [Register],

    /// `!`
    LogicalNot [],
    TypeOf [],
    /// Einstelliges `-`.
    Negate [],
    /// `~`
    BitwiseNot [],
    /// Einstelliges `+`.
    ToNumber [],
    ToNumeric [],
    ToString [],
    /// Der Akkumulator nach ToNumeric plus eins.
    Inc [],
    /// Der Akkumulator nach ToNumeric minus eins.
    Dec [],

    Jump [Jump],
    /// Springt, wenn ToBoolean des Akkumulators *true* ist.
    JumpIfTrue [Jump],
    /// Springt, wenn ToBoolean des Akkumulators *false* ist.
    JumpIfFalse [Jump],
    /// Springt, wenn der Akkumulator *undefined* oder *null* ist.
    JumpIfNullish [Jump],
    JumpIfNotNullish [Jump],
    /// Springt zurück an den Anfang einer Schleife.
    JumpLoop [Jump],

    /// Ein neues Funktionsobjekt für die Funktion in der ersten Konstante,
    /// mit dem Namen aus der zweiten (SetFunctionName).
    CreateClosure [Constant, Constant],
    /// Dasselbe mit dem Schlüssel im Register als Namen.
    CreateClosureKeyed [Constant, Register],
    /// Ruft die Funktion im Register auf (Call, Abschnitt 7.3.13): `this`
    /// steht im Register danach, dann folgen so viele Argumente, wie die
    /// Zahl sagt. Das Ergebnis kommt in den Akkumulator. Ist der Wert nicht
    /// aufrufbar, wirft das einen TypeError, erst nach den Argumenten
    /// (EvaluateCall, Abschnitt 13.3.6.2).
    Call [Register, Immediate],

    /// Wirft den Akkumulator.
    Throw [],
    /// Beendet den Rahmen mit dem Akkumulator.
    Return [],
}

impl Op {
    pub fn byte(self) -> u8 {
        // `ALL` zählt jeden Opcode genau einmal auf, und es sind weniger
        // als 256.
        Op::ALL
            .iter()
            .position(|&op| op == self)
            .and_then(|index| u8::try_from(index).ok())
            .unwrap_or(u8::MAX)
    }

    pub fn from_byte(byte: u8) -> Option<Op> {
        Op::ALL.get(usize::from(byte)).copied()
    }

    fn is_prefix(self) -> bool {
        matches!(self, Op::Wide | Op::ExtraWide)
    }
}

/// Eine Konstante einer Funktion.
#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Number(f64),
    /// Eine Zeichenkette als Wert.
    String(Vec<u16>),
    /// Ein Name: ein Bezeichner oder Eigenschaftsname. Er wird zum Atom.
    Name(Vec<u16>),
    /// Eine Funktion aus dem Quelltext, aus der [`Op::CreateClosure`]
    /// Funktionsobjekte macht.
    Function(FunctionTemplate),
}

/// Eine Stelle im Quelltext, an der Funktionsobjekte entstehen. Übersetzt
/// wird die Funktion erst bei ihrem ersten Aufruf, mit dem Syntaxbaum, aus
/// dem der Code stammt.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionTemplate {
    pub function: FunctionId,
    /// Ob der Rumpf den eigenen Namen bindet, wie ein benannter
    /// Funktionsausdruck (Abschnitt 15.2.5).
    pub binds_name: bool,
    /// Die Namen, die der umgebende Code in Registern hält. Ein Zugriff des
    /// Rumpfs darauf bräuchte eine Closure.
    pub outer: Rc<BTreeSet<Atom>>,
}

/// Ein gelesener Befehl.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub op: Op,
    /// Die Operanden in der Reihenfolge von [`Op::operands`].
    pub operands: [i64; 2],
    /// Wo der Befehl beginnt, mit Präfix.
    pub start: usize,
    /// Wo der nächste beginnt.
    pub end: usize,
}

impl Instruction {
    pub fn operand(&self, index: usize) -> i64 {
        self.operands.get(index).copied().unwrap_or(0)
    }

    /// Das Ziel eines Sprungs mit der Weite im ersten Operanden.
    pub fn target(&self) -> Option<usize> {
        let start = i64::try_from(self.start).ok()?;
        usize::try_from(start.checked_add(self.operand(0))?).ok()
    }
}

/// Der Bytecode einer Funktion oder eines Scripts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Code {
    pub(crate) bytes: Vec<u8>,
    pub(crate) constants: Vec<Constant>,
    pub(crate) registers: u32,
    /// Die Zahl der Parameter einer Funktion. Ihre Register folgen auf
    /// `this` und das Funktionsobjekt.
    pub(crate) parameters: u32,
    pub(crate) strict: bool,
    /// Lagen im Quelltext ab einem Befehl: Paare aus Abstand im Bytecode
    /// und Abstand im Quelltext, nach dem ersten aufsteigend.
    pub(crate) positions: Vec<(u32, u32)>,
}

impl Code {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn constants(&self) -> &[Constant] {
        &self.constants
    }

    pub fn constant(&self, index: i64) -> Option<&Constant> {
        self.constants.get(usize::try_from(index).ok()?)
    }

    /// Die Zahl der Register eines Rahmens.
    pub fn register_count(&self) -> usize {
        usize::try_from(self.registers).unwrap_or(usize::MAX)
    }

    /// Die Zahl der Parameter, 0 bei einem Script.
    pub fn parameter_count(&self) -> usize {
        usize::try_from(self.parameters).unwrap_or(usize::MAX)
    }

    pub fn is_strict(&self) -> bool {
        self.strict
    }

    /// Die Lage im Quelltext, zu der der Befehl bei `offset` gehört.
    pub fn position(&self, offset: usize) -> Option<usize> {
        let offset = u32::try_from(offset).ok()?;
        let after = self
            .positions
            .partition_point(|&(start, _)| start <= offset);
        let (_, position) = self.positions.get(after.checked_sub(1)?)?;
        usize::try_from(*position).ok()
    }

    /// Liest den Befehl, der bei `offset` beginnt.
    pub fn decode(&self, offset: usize) -> Option<Instruction> {
        let mut at = offset;
        let mut op = Op::from_byte(*self.bytes.get(at)?)?;
        let mut width = 1;
        if op.is_prefix() {
            width = if op == Op::Wide { 2 } else { 4 };
            at = at.checked_add(1)?;
            op = Op::from_byte(*self.bytes.get(at)?)?;
            if op.is_prefix() {
                return None;
            }
        }
        at = at.checked_add(1)?;
        let mut operands = [0; 2];
        for (index, kind) in op.operands().iter().enumerate() {
            let end = at.checked_add(width)?;
            let bytes = self.bytes.get(at..end)?;
            let signed = matches!(kind, Operand::Immediate | Operand::Jump);
            let value = read(bytes, signed)?;
            *operands.get_mut(index)? = value;
            at = end;
        }
        Some(Instruction {
            op,
            operands,
            start: offset,
            end: at,
        })
    }
}

/// Ein Operand aus ein, zwei oder vier Bytes.
fn read(bytes: &[u8], signed: bool) -> Option<i64> {
    Some(match (bytes, signed) {
        (&[a], false) => i64::from(a),
        (&[a], true) => i64::from(i8::from_le_bytes([a])),
        (&[a, b], false) => i64::from(u16::from_le_bytes([a, b])),
        (&[a, b], true) => i64::from(i16::from_le_bytes([a, b])),
        (&[a, b, c, d], false) => i64::from(u32::from_le_bytes([a, b, c, d])),
        (&[a, b, c, d], true) => i64::from(i32::from_le_bytes([a, b, c, d])),
        _ => return None,
    })
}

/// Das Ende des Bytecodes ist erreicht: mehr als `i32::MAX` Bytes, die kein
/// Sprung mehr überbrückt, oder ein Operand, der nicht in vier Bytes passt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLarge;

/// Ein Sprung nach vorn, dessen Ziel noch fehlt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct Label {
    start: usize,
    operand: usize,
}

/// Schreibt Befehle.
#[derive(Debug, Default)]
pub struct Emitter {
    bytes: Vec<u8>,
    positions: Vec<(u32, u32)>,
}

impl Emitter {
    pub fn offset(&self) -> usize {
        self.bytes.len()
    }

    /// Merkt sich, dass die folgenden Befehle zur Lage `position` im
    /// Quelltext gehören.
    pub fn set_position(&mut self, position: usize) -> Result<(), TooLarge> {
        let offset = u32::try_from(self.bytes.len()).map_err(|_| TooLarge)?;
        let position = u32::try_from(position).map_err(|_| TooLarge)?;
        match self.positions.last_mut() {
            Some((_, last)) if *last == position => {}
            Some((start, last)) if *start == offset => *last = position,
            _ => self.positions.push((offset, position)),
        }
        Ok(())
    }

    /// Schreibt einen Befehl mit der kleinsten Breite, in die alle
    /// Operanden passen.
    pub fn emit(&mut self, op: Op, operands: &[i64]) -> Result<(), TooLarge> {
        let width = operands
            .iter()
            .zip(op.operands())
            .map(|(&value, &kind)| width(value, kind))
            .try_fold(1, |widest, width| width.map(|width| widest.max(width)))
            .ok_or(TooLarge)?;
        self.emit_with_width(op, operands, width)
    }

    fn emit_with_width(&mut self, op: Op, operands: &[i64], width: usize) -> Result<(), TooLarge> {
        let fits = operands
            .iter()
            .zip(op.operands())
            .all(|(&value, &kind)| self::width(value, kind).is_some_and(|needed| needed <= width));
        if operands.len() != op.operands().len() || !fits {
            return Err(TooLarge);
        }
        match width {
            2 => self.bytes.push(Op::Wide.byte()),
            4 => self.bytes.push(Op::ExtraWide.byte()),
            _ => {}
        }
        self.bytes.push(op.byte());
        for &value in operands {
            write(&mut self.bytes, value, width)?;
        }
        self.check_size()
    }

    /// Schreibt einen Sprung nach vorn, dessen Ziel [`Emitter::bind`]
    /// nachträgt.
    pub fn jump(&mut self, op: Op) -> Result<Label, TooLarge> {
        let start = self.bytes.len();
        self.emit_with_width(op, &[0], 4)?;
        Ok(Label {
            start,
            operand: start.checked_add(2).ok_or(TooLarge)?,
        })
    }

    /// Lässt den Sprung `label` hierher zeigen.
    pub fn bind(&mut self, label: Label) -> Result<(), TooLarge> {
        let distance = self
            .bytes
            .len()
            .checked_sub(label.start)
            .and_then(|distance| i32::try_from(distance).ok())
            .ok_or(TooLarge)?;
        let end = label.operand.checked_add(4).ok_or(TooLarge)?;
        let slot = self.bytes.get_mut(label.operand..end).ok_or(TooLarge)?;
        slot.copy_from_slice(&distance.to_le_bytes());
        Ok(())
    }

    /// Schreibt [`Op::JumpLoop`] zurück zu `target`.
    pub fn jump_loop(&mut self, target: usize) -> Result<(), TooLarge> {
        let start = i64::try_from(self.bytes.len()).map_err(|_| TooLarge)?;
        let target = i64::try_from(target).map_err(|_| TooLarge)?;
        let distance = target.checked_sub(start).ok_or(TooLarge)?;
        // Mit Präfix beginnt der Befehl am selben Byte, die Weite bleibt.
        self.emit(Op::JumpLoop, &[distance])
    }

    fn check_size(&self) -> Result<(), TooLarge> {
        if i32::try_from(self.bytes.len()).is_err() {
            return Err(TooLarge);
        }
        Ok(())
    }

    /// Der fertige Bytecode.
    pub fn finish(self, constants: Vec<Constant>, registers: u32, strict: bool) -> Code {
        Code {
            bytes: self.bytes,
            constants,
            registers,
            parameters: 0,
            strict,
            positions: self.positions,
        }
    }
}

/// Die kleinste Breite in Bytes, in die `value` als Operand der Art `kind`
/// passt.
fn width(value: i64, kind: Operand) -> Option<usize> {
    let ranges: [(usize, i64, i64); 3] = match kind {
        Operand::Immediate | Operand::Jump => [
            (1, i8::MIN.into(), i8::MAX.into()),
            (2, i16::MIN.into(), i16::MAX.into()),
            (4, i32::MIN.into(), i32::MAX.into()),
        ],
        Operand::Register | Operand::Constant => [
            (1, 0, u8::MAX.into()),
            (2, 0, u16::MAX.into()),
            (4, 0, u32::MAX.into()),
        ],
    };
    ranges
        .into_iter()
        .find(|&(_, low, high)| (low..=high).contains(&value))
        .map(|(width, _, _)| width)
}

/// Schreibt `value` in `width` Bytes. Im Zweierkomplement stellen die
/// unteren Bytes jeden Wert dar, der in die Breite passt; das prüft der
/// Aufrufer.
fn write(bytes: &mut Vec<u8>, value: i64, width: usize) -> Result<(), TooLarge> {
    let all = value.to_le_bytes();
    bytes.extend_from_slice(all.get(..width).ok_or(TooLarge)?);
    Ok(())
}

/// Codeeinheiten zum Anzeigen, einzelne Surrogate als `\u{…}`.
fn display_units(formatter: &mut fmt::Formatter<'_>, units: &[u16]) -> fmt::Result {
    for unit in char::decode_utf16(units.iter().copied()) {
        match unit {
            Ok(character) => write!(formatter, "{}", character.escape_debug())?,
            Err(error) => write!(formatter, "\\u{{{:x}}}", error.unpaired_surrogate())?,
        }
    }
    Ok(())
}

/// Eine Zeile je Befehl: Abstand, Opcode und Operanden. Register stehen als
/// `r0`, Konstanten mit ihrem Inhalt, Sprünge mit ihrem Ziel.
impl fmt::Display for Code {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut offset = 0;
        while offset < self.bytes.len() {
            let Some(instruction) = self.decode(offset) else {
                return writeln!(formatter, "{offset:>4}: ?");
            };
            write!(formatter, "{offset:>4}: {:?}", instruction.op)?;
            for (index, kind) in instruction.op.operands().iter().enumerate() {
                let value = instruction.operand(index);
                match kind {
                    Operand::Register => write!(formatter, " r{value}")?,
                    Operand::Immediate => write!(formatter, " {value}")?,
                    Operand::Jump => match instruction.target() {
                        Some(target) => write!(formatter, " @{target}")?,
                        None => write!(formatter, " @?")?,
                    },
                    Operand::Constant => match self.constant(value) {
                        Some(Constant::Number(number)) => write!(formatter, " {number:?}")?,
                        Some(Constant::String(units)) => {
                            formatter.write_str(" \"")?;
                            display_units(formatter, units)?;
                            formatter.write_str("\"")?;
                        }
                        Some(Constant::Name(units)) => {
                            formatter.write_str(" ")?;
                            display_units(formatter, units)?;
                        }
                        Some(Constant::Function(template)) => {
                            write!(formatter, " fn{}", template.function.index())?;
                        }
                        None => write!(formatter, " c{value}?")?,
                    },
                }
            }
            writeln!(formatter)?;
            offset = instruction.end;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
