//! Der Interpreter: eine Schleife, die die Befehle eines Rahmens liest und
//! mit `match` verzweigt. Bisher gibt es nur den Rahmen eines Scripts.
//!
//! Die Register des Rahmens liegen im Registerstapel des Heaps, der
//! Akkumulator in einem Handle des Laufs, die Konstanten in einem internen
//! Feld. Jeder Befehl läuft in einem eigenen Bereich für Handles, damit
//! Zwischenwerte nicht über die ganze Ausführung anwachsen.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::bytecode::{Code, Constant, Instruction, Op};
use crate::compare::{is_less_than, is_loosely_equal, is_strictly_equal};
use crate::conversion::{to_boolean, to_number, to_numeric, to_property_key, to_string};
use crate::heap::{Local, Scope};
use crate::key::{PropertyKey, array_index};
use crate::number;
use crate::object::{array_create, ordinary_object_create, set_prototype_of};
use crate::operations::{create_data_property_or_throw, get, has_property, set};
use crate::operators::{BinaryOp, apply_string_or_numeric_binary_operator, type_of};
use crate::realm::{GlobalReference, Intrinsic, RealmId};
use crate::reference::{
    copy_data_properties, delete_property, get_value, instanceof_operator, put_value,
    to_object_base,
};
use crate::value::{Kind, Value};

/// Was nach einem Befehl geschieht.
enum Flow {
    Next,
    Jump(usize),
    Return,
}

struct Frame<'c, 's> {
    code: &'c Code,
    realm: RealmId,
    /// Das erste Register des Rahmens im Registerstapel.
    base: usize,
    /// Die Konstanten als Werte.
    pool: Local<'s>,
    accumulator: Local<'s>,
}

/// Führt `code` als Rahmen mit `this` aus und gibt den Akkumulator bei
/// [`Op::Return`] zurück. Reicht der Registerstapel nicht, wirft das einen
/// RangeError.
pub(crate) fn execute<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    code: &Code,
    this: Local<'_>,
) -> Completion<Local<'s>> {
    let Some(base) = agent.heap_mut().push_registers(code.register_count()) else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    let result = run(agent, scope, realm, code, base, this);
    agent.heap_mut().truncate_registers(base);
    result
}

fn run<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    code: &Code,
    base: usize,
    this: Local<'_>,
) -> Completion<Local<'s>> {
    let pool = constant_pool(agent, scope, code)?;
    let accumulator = agent.heap().local(scope, Value::UNDEFINED);
    let heap = agent.heap();
    heap.set_register(base, heap.get(this));
    let frame = Frame {
        code,
        realm,
        base,
        pool,
        accumulator,
    };
    let mut offset = 0;
    loop {
        let Some(instruction) = code.decode(offset) else {
            // Kann nicht vorkommen: Der Übersetzer schreibt nur gültige
            // Befehle und endet mit Return.
            return Err(agent.throw_error(ErrorKind::RangeError));
        };
        let flow = agent.scope(|agent, inner| frame.step(agent, inner, &instruction))?;
        offset = match flow {
            Flow::Next => instruction.end,
            Flow::Jump(target) => target,
            Flow::Return => return Ok(accumulator),
        };
    }
}

/// Legt die Konstanten im Heap an: Zahlen als Werte, Zeichenketten als
/// neue Zeichenketten, Namen als Atome.
fn constant_pool<'s>(agent: &mut Agent, scope: &'s Scope, code: &Code) -> Completion<Local<'s>> {
    let pool = agent
        .heap_mut()
        .alloc_slots(scope, code.constants().len())?;
    for (index, constant) in code.constants().iter().enumerate() {
        agent.scope(|agent, inner| -> Completion<()> {
            let value = match constant {
                Constant::Number(number) => agent.heap().local(inner, Value::number(*number)),
                Constant::String(units) => agent.heap_mut().alloc_string(inner, units)?,
                Constant::Name(units) => agent.atom(inner, units)?,
            };
            let heap = agent.heap();
            if let Kind::Internal(cell) = heap.get(pool).kind() {
                heap.set_slot(cell, index, heap.get(value));
            }
            Ok(())
        })?;
    }
    Ok(pool)
}

fn binary_op(op: Op) -> Option<BinaryOp> {
    Some(match op {
        Op::Add => BinaryOp::Add,
        Op::Sub => BinaryOp::Subtract,
        Op::Mul => BinaryOp::Multiply,
        Op::Div => BinaryOp::Divide,
        Op::Mod => BinaryOp::Remainder,
        Op::Exp => BinaryOp::Exponentiate,
        Op::BitwiseAnd => BinaryOp::BitwiseAnd,
        Op::BitwiseOr => BinaryOp::BitwiseOr,
        Op::BitwiseXor => BinaryOp::BitwiseXor,
        Op::ShiftLeft => BinaryOp::LeftShift,
        Op::ShiftRight => BinaryOp::SignedRightShift,
        Op::ShiftRightLogical => BinaryOp::UnsignedRightShift,
        _ => return None,
    })
}

/// Die Zeichenkette `text` als Atom.
fn string<'s>(agent: &mut Agent, scope: &'s Scope, text: &str) -> Completion<Local<'s>> {
    let units: Vec<u16> = text.encode_utf16().collect();
    Ok(agent.atom(scope, &units)?)
}

/// Ein Schlüssel als Wert, wie ihn ToPropertyKey liefert. Array-Indizes
/// bleiben Zahlen; ToPropertyKey macht daraus wieder denselben Index.
fn key_value<'s>(agent: &Agent, scope: &'s Scope, key: PropertyKey<'_>) -> Local<'s> {
    let heap = agent.heap();
    match key {
        PropertyKey::Index(index) => heap.local(scope, Value::number(f64::from(index))),
        PropertyKey::String(local) | PropertyKey::Symbol(local) => {
            heap.local(scope, heap.get(local))
        }
    }
}

fn length_key<'s>(agent: &mut Agent, scope: &'s Scope) -> Completion<PropertyKey<'s>> {
    let units: Vec<u16> = "length".encode_utf16().collect();
    Ok(agent.key_from_units(scope, &units)?)
}

impl<'c> Frame<'c, '_> {
    /// Der Platz eines Registers im Registerstapel. Register stammen vom
    /// Übersetzer und liegen im Rahmen; ein ungültiges liest *undefined*.
    fn slot(&self, operand: i64) -> usize {
        usize::try_from(operand)
            .ok()
            .and_then(|register| self.base.checked_add(register))
            .unwrap_or(usize::MAX)
    }

    fn register<'s>(&self, agent: &Agent, scope: &'s Scope, operand: i64) -> Local<'s> {
        let heap = agent.heap();
        heap.local(scope, heap.register(self.slot(operand)))
    }

    fn set_register(&self, agent: &Agent, operand: i64, value: Local<'_>) {
        let heap = agent.heap();
        heap.set_register(self.slot(operand), heap.get(value));
    }

    fn set_accumulator(&self, agent: &Agent, value: Local<'_>) {
        let heap = agent.heap();
        heap.set(self.accumulator, heap.get(value));
    }

    fn set_accumulator_value(&self, agent: &Agent, value: Value<'static>) {
        agent.heap().set(self.accumulator, value);
    }

    /// Die Konstante als Wert.
    fn constant<'s>(&self, agent: &Agent, scope: &'s Scope, operand: i64) -> Local<'s> {
        let heap = agent.heap();
        let value = match (heap.get(self.pool).kind(), usize::try_from(operand)) {
            (Kind::Internal(cell), Ok(index)) => heap.slot(cell, index),
            _ => None,
        };
        heap.local(scope, value.unwrap_or(Value::UNDEFINED))
    }

    /// Der Name in der Konstante `operand` und der Schlüssel dazu. Die
    /// Konstante ist schon ein Atom; nur Namen aus Literalen wie `{0: x}`
    /// können ein Array-Index sein.
    fn name<'t>(
        &self,
        agent: &Agent,
        scope: &'t Scope,
        operand: i64,
    ) -> (&'c [u16], PropertyKey<'t>) {
        let units = match self.code.constant(operand) {
            Some(Constant::Name(units)) => units.as_slice(),
            _ => &[],
        };
        let key = match array_index(units) {
            Some(index) => PropertyKey::Index(index),
            None => PropertyKey::String(self.constant(agent, scope, operand)),
        };
        (units, key)
    }

    /// Eine Kopie des Akkumulators, die ihn überdauert, wenn ein Befehl ihn
    /// überschreibt.
    fn accumulator_copy<'t>(&self, agent: &Agent, scope: &'t Scope) -> Local<'t> {
        let heap = agent.heap();
        heap.local(scope, heap.get(self.accumulator))
    }

    /// Die Befehle für Objekte und Eigenschaften.
    fn property(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        instruction: &Instruction,
    ) -> Completion<()> {
        let first = instruction.operand(0);
        let second = instruction.operand(1);
        let strict = self.code.is_strict();
        match instruction.op {
            Op::CreateObject | Op::CreateArray => {
                let object = if instruction.op == Op::CreateObject {
                    let prototype = agent.intrinsic(scope, self.realm, Intrinsic::ObjectPrototype);
                    ordinary_object_create(agent, scope, Some(prototype))?
                } else {
                    let prototype = agent.intrinsic(scope, self.realm, Intrinsic::ArrayPrototype);
                    array_create(agent, scope, 0, Some(prototype))?
                };
                self.set_accumulator(agent, object);
            }
            Op::DefineNamed | Op::DefineKeyed => {
                let object = self.register(agent, scope, first);
                let key = if instruction.op == Op::DefineNamed {
                    self.name(agent, scope, second).1
                } else {
                    let key = self.register(agent, scope, second);
                    to_property_key(agent, scope, key)?
                };
                create_data_property_or_throw(agent, scope, object, key, self.accumulator)?;
            }
            Op::SetPrototypeLiteral => {
                let object = self.register(agent, scope, first);
                let prototype = match agent.heap().get(self.accumulator).kind() {
                    Kind::Object(_) => Some(self.accumulator),
                    Kind::Null => None,
                    _ => return Ok(()),
                };
                // Das Objekt ist neu, gewöhnlich und erweiterbar; das gelingt
                // immer.
                set_prototype_of(agent, object, prototype)?;
            }
            Op::CopyDataProperties => {
                let object = self.register(agent, scope, first);
                copy_data_properties(agent, scope, object, self.accumulator)?;
            }
            Op::ArrayPush | Op::ArrayHole => {
                // Im Literal ist die Länge stets der nächste Index
                // (ArrayAccumulation, Abschnitt 13.2.4.1): Elemente
                // verlängern das Array, Lücken setzen die Länge. Eine Folge
                // von Lücken setzt sie hier einmal je Lücke statt einmal am
                // Ende; bei einem neuen Array ist das nicht zu unterscheiden.
                let array = self.register(agent, scope, first);
                let length = length_key(agent, scope)?;
                let current = get(agent, scope, array, length)?;
                if instruction.op == Op::ArrayPush {
                    let key = to_property_key(agent, scope, current)?;
                    create_data_property_or_throw(agent, scope, array, key, self.accumulator)?;
                } else {
                    let next = agent.heap().get(current).as_number().unwrap_or(0.0) + 1.0;
                    let next = agent.heap().local(scope, Value::number(next));
                    set(agent, scope, array, length, next, true)?;
                }
            }
            Op::ToPropertyKey => {
                let key = to_property_key(agent, scope, self.accumulator)?;
                let value = key_value(agent, scope, key);
                self.set_accumulator(agent, value);
            }
            Op::LdaNamed | Op::LdaKeyed | Op::LdaKeyedRef => {
                let this = if instruction.op == Op::LdaNamed {
                    self.accumulator_copy(agent, scope)
                } else {
                    self.register(agent, scope, first)
                };
                let base = to_object_base(agent, scope, self.realm, this)?;
                let key = match instruction.op {
                    Op::LdaNamed => self.name(agent, scope, first).1,
                    Op::LdaKeyed => to_property_key(agent, scope, self.accumulator)?,
                    _ => {
                        let key = self.register(agent, scope, second);
                        let key = to_property_key(agent, scope, key)?;
                        let stored = key_value(agent, scope, key);
                        self.set_register(agent, second, stored);
                        key
                    }
                };
                let value = get_value(agent, scope, base, this, key)?;
                self.set_accumulator(agent, value);
            }
            Op::StaNamed | Op::StaKeyed => {
                let this = self.register(agent, scope, first);
                let base = to_object_base(agent, scope, self.realm, this)?;
                let key = if instruction.op == Op::StaNamed {
                    self.name(agent, scope, second).1
                } else {
                    let key = self.register(agent, scope, second);
                    to_property_key(agent, scope, key)?
                };
                put_value(agent, scope, base, this, key, self.accumulator, strict)?;
            }
            Op::DeleteNamed | Op::DeleteKeyed => {
                let this = if instruction.op == Op::DeleteNamed {
                    self.accumulator_copy(agent, scope)
                } else {
                    self.register(agent, scope, first)
                };
                let base = to_object_base(agent, scope, self.realm, this)?;
                let key = if instruction.op == Op::DeleteNamed {
                    self.name(agent, scope, first).1
                } else {
                    to_property_key(agent, scope, self.accumulator)?
                };
                let deleted = delete_property(agent, scope, base, this, key, strict)?;
                self.set_accumulator_value(agent, Value::boolean(deleted));
            }
            Op::TestIn => {
                // Abschnitt 13.10.1: erst die Art des Objekts, dann
                // ToPropertyKey.
                let left = self.register(agent, scope, first);
                let right = self.accumulator_copy(agent, scope);
                if !matches!(agent.heap().get(right).kind(), Kind::Object(_)) {
                    return Err(agent.throw_error(ErrorKind::TypeError));
                }
                let key = to_property_key(agent, scope, left)?;
                let found = has_property(agent, right, key)?;
                self.set_accumulator_value(agent, Value::boolean(found));
            }
            Op::TestInstanceOf => {
                let left = self.register(agent, scope, first);
                let right = self.accumulator_copy(agent, scope);
                let found = instanceof_operator(agent, scope, left, right)?;
                self.set_accumulator_value(agent, Value::boolean(found));
            }
            // Die übrigen Befehle führt `step` aus.
            _ => return Err(agent.throw_error(ErrorKind::RangeError)),
        }
        Ok(())
    }

    fn global_reference(&self, agent: &Agent, operand: i64) -> GlobalReference {
        let value = agent.heap().register(self.slot(operand));
        GlobalReference::from_number(value.as_number().unwrap_or(0.0))
    }

    fn step(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        instruction: &Instruction,
    ) -> Completion<Flow> {
        let first = instruction.operand(0);
        let second = instruction.operand(1);
        let strict = self.code.is_strict();
        match instruction.op {
            Op::LdaUndefined => self.set_accumulator_value(agent, Value::UNDEFINED),
            Op::LdaNull => self.set_accumulator_value(agent, Value::NULL),
            Op::LdaTrue => self.set_accumulator_value(agent, Value::TRUE),
            Op::LdaFalse => self.set_accumulator_value(agent, Value::FALSE),
            Op::LdaHole => self.set_accumulator_value(agent, Value::HOLE),
            Op::LdaSmi => {
                let small = i32::try_from(first).unwrap_or(0);
                self.set_accumulator_value(agent, Value::int32(small));
            }
            Op::LdaConstant => {
                let value = self.constant(agent, scope, first);
                self.set_accumulator(agent, value);
            }
            Op::Ldar => {
                let value = self.register(agent, scope, first);
                self.set_accumulator(agent, value);
            }
            Op::Star => self.set_register(agent, first, self.accumulator),
            Op::Mov => {
                let value = self.register(agent, scope, first);
                self.set_register(agent, second, value);
            }
            Op::CheckHole => {
                if agent.heap().register(self.slot(first)).is_hole() {
                    return Err(agent.throw_error(ErrorKind::ReferenceError));
                }
            }
            Op::ThrowConstAssignment => return Err(agent.throw_error(ErrorKind::TypeError)),
            Op::ResolveGlobal => {
                let (name, key) = self.name(agent, scope, first);
                let reference = agent.resolve_global(scope, self.realm, name, key)?;
                let number = agent.heap().local(scope, Value::int32(reference.number()));
                self.set_register(agent, second, number);
            }
            Op::LdaGlobal | Op::LdaGlobalRef => {
                let (name, key) = self.name(agent, scope, first);
                let reference = if instruction.op == Op::LdaGlobal {
                    agent.resolve_global(scope, self.realm, name, key)?
                } else {
                    self.global_reference(agent, second)
                };
                let value = agent.get_global(scope, self.realm, reference, name, key, strict)?;
                self.set_accumulator(agent, value);
            }
            Op::StaGlobal => {
                let (name, key) = self.name(agent, scope, first);
                let reference = self.global_reference(agent, second);
                agent.put_global(
                    scope,
                    self.realm,
                    reference,
                    name,
                    key,
                    self.accumulator,
                    strict,
                )?;
            }
            Op::InitGlobal => {
                let (name, _) = self.name(agent, scope, first);
                agent.initialize_lexical_binding(self.realm, name, self.accumulator);
            }
            Op::TypeofGlobal => {
                let (name, key) = self.name(agent, scope, first);
                let reference = agent.resolve_global(scope, self.realm, name, key)?;
                let text = if reference == GlobalReference::Unresolvable {
                    "undefined"
                } else {
                    let value =
                        agent.get_global(scope, self.realm, reference, name, key, strict)?;
                    type_of(agent.heap(), agent.heap().get(value))
                };
                let result = string(agent, scope, text)?;
                self.set_accumulator(agent, result);
            }
            Op::DeleteGlobal => {
                let (name, key) = self.name(agent, scope, first);
                let reference = agent.resolve_global(scope, self.realm, name, key)?;
                let deleted = reference == GlobalReference::Unresolvable
                    || agent.delete_global(scope, self.realm, name, key)?;
                self.set_accumulator_value(agent, Value::boolean(deleted));
            }
            Op::Add
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Mod
            | Op::Exp
            | Op::BitwiseAnd
            | Op::BitwiseOr
            | Op::BitwiseXor
            | Op::ShiftLeft
            | Op::ShiftRight
            | Op::ShiftRightLogical => {
                let left = self.register(agent, scope, first);
                let op = binary_op(instruction.op).unwrap_or(BinaryOp::Add);
                let result = apply_string_or_numeric_binary_operator(
                    agent,
                    scope,
                    left,
                    op,
                    self.accumulator,
                )?;
                self.set_accumulator(agent, result);
            }
            Op::TestEqual => {
                let left = self.register(agent, scope, first);
                let equal = is_loosely_equal(agent, scope, left, self.accumulator)?;
                self.set_accumulator_value(agent, Value::boolean(equal));
            }
            Op::TestEqualStrict => {
                let left = self.register(agent, scope, first);
                let heap = agent.heap();
                let equal = is_strictly_equal(heap, heap.get(left), heap.get(self.accumulator));
                self.set_accumulator_value(agent, Value::boolean(equal));
            }
            Op::TestLessThan
            | Op::TestGreaterThan
            | Op::TestLessThanOrEqual
            | Op::TestGreaterThanOrEqual => {
                // Abschnitt 13.10.1: `>` und `<=` vergleichen umgekehrt und
                // werten dabei weiter von links nach rechts aus; `<=` und
                // `>=` sind falsch, wenn der Vergleich *true* oder
                // *undefined* ergibt.
                let left = self.register(agent, scope, first);
                let right = self.accumulator;
                let result = match instruction.op {
                    Op::TestLessThan => {
                        is_less_than(agent, scope, left, right, true)? == Some(true)
                    }
                    Op::TestGreaterThan => {
                        is_less_than(agent, scope, right, left, false)? == Some(true)
                    }
                    Op::TestLessThanOrEqual => {
                        is_less_than(agent, scope, right, left, false)? == Some(false)
                    }
                    _ => is_less_than(agent, scope, left, right, true)? == Some(false),
                };
                self.set_accumulator_value(agent, Value::boolean(result));
            }
            Op::LogicalNot => {
                let heap = agent.heap();
                let value = !to_boolean(heap, heap.get(self.accumulator));
                self.set_accumulator_value(agent, Value::boolean(value));
            }
            Op::TypeOf => {
                let heap = agent.heap();
                let text = type_of(heap, heap.get(self.accumulator));
                let result = string(agent, scope, text)?;
                self.set_accumulator(agent, result);
            }
            Op::Negate | Op::BitwiseNot | Op::ToNumeric | Op::Inc | Op::Dec => {
                // Bis es BigInt-Werte gibt, ergibt ToNumeric eine Zahl.
                let numeric = to_numeric(agent, scope, self.accumulator)?;
                let result = match instruction.op {
                    Op::Negate => number::unary_minus(numeric),
                    Op::BitwiseNot => number::bitwise_not(numeric),
                    Op::Inc => numeric + 1.0,
                    Op::Dec => numeric - 1.0,
                    _ => numeric,
                };
                self.set_accumulator_value(agent, Value::number(result));
            }
            Op::ToNumber => {
                let result = to_number(agent, scope, self.accumulator)?;
                self.set_accumulator_value(agent, Value::number(result));
            }
            Op::ToString => {
                let result = to_string(agent, scope, self.accumulator)?;
                self.set_accumulator(agent, result);
            }
            Op::Jump | Op::JumpLoop => return Ok(jump(instruction)),
            Op::JumpIfTrue | Op::JumpIfFalse => {
                let heap = agent.heap();
                let value = to_boolean(heap, heap.get(self.accumulator));
                if value == (instruction.op == Op::JumpIfTrue) {
                    return Ok(jump(instruction));
                }
            }
            Op::JumpIfNullish | Op::JumpIfNotNullish => {
                let value = agent.heap().get(self.accumulator);
                let nullish = value.is_undefined() || value.is_null();
                if nullish == (instruction.op == Op::JumpIfNullish) {
                    return Ok(jump(instruction));
                }
            }
            Op::CreateObject
            | Op::CreateArray
            | Op::DefineNamed
            | Op::DefineKeyed
            | Op::SetPrototypeLiteral
            | Op::CopyDataProperties
            | Op::ArrayPush
            | Op::ArrayHole
            | Op::ToPropertyKey
            | Op::LdaNamed
            | Op::LdaKeyed
            | Op::LdaKeyedRef
            | Op::StaNamed
            | Op::StaKeyed
            | Op::DeleteNamed
            | Op::DeleteKeyed
            | Op::TestIn
            | Op::TestInstanceOf => self.property(agent, scope, instruction)?,
            Op::Throw => return Err(agent.throw_value(self.accumulator)),
            Op::Return => return Ok(Flow::Return),
            // Präfixe liest `Code::decode` mit dem Befehl danach.
            Op::Wide | Op::ExtraWide => return Err(agent.throw_error(ErrorKind::RangeError)),
        }
        Ok(Flow::Next)
    }
}

fn jump(instruction: &Instruction) -> Flow {
    // Ein Sprung ohne gültiges Ziel endet beim nächsten Lesen.
    Flow::Jump(instruction.target().unwrap_or(usize::MAX))
}

#[cfg(test)]
mod tests;
