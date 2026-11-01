//! Der Interpreter: eine Schleife, die die Befehle des obersten Rahmens
//! liest und mit `match` verzweigt.
//!
//! Die Register eines Rahmens liegen im Registerstapel des Heaps, davor zwei
//! Kopfregister: die Konstanten als Werte in einer internen Zelle und der
//! aktuelle Kontext, für ein Script zu Beginn *undefined*, für eine
//! Funktion ihr [[Environment]]. Der Akkumulator ist ein Handle des Laufs,
//! das sich alle seine Rahmen teilen; darin kommt auch der Rückgabewert
//! eines Aufrufs an. Jeder Befehl läuft in einem eigenen Bereich für
//! Handles, damit Zwischenwerte nicht über die ganze Ausführung anwachsen.
//!
//! Ruft JavaScript eine Funktion auf, legt der Lauf einen Rahmen auf seinen
//! eigenen Stapel, ohne Rekursion in Rust. Ruft die Engine eine Funktion
//! auf, etwa einen Getter, beginnt ein neuer Lauf ([`call_function`]). Wie
//! viel vom Stapel der Maschine solche verschachtelten Läufe brauchen
//! dürfen, begrenzt [`crate::agent::STACK_BUDGET`] (RS-8); wie viele Rahmen
//! es gibt, begrenzt der Registerstapel.

#![forbid(unsafe_code)]

use std::rc::Rc;

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::ast::Ast;
use crate::bytecode::{Code, Constant, Instruction, Op};
use crate::compare::{is_less_than, is_loosely_equal, is_strictly_equal};
use crate::compiler::{CompileError, compile_function};
use crate::conversion::{to_boolean, to_number, to_numeric, to_property_key, to_string};
use crate::descriptor::PropertyDescriptor;
use crate::function::{Compiled, function_environment, function_parts, instantiate_closure};
use crate::heap::{Heap, Local, Scope};
use crate::key::{PropertyKey, array_index};
use crate::number;
use crate::object::{array_create, ordinary_object_create, set_prototype_of};
use crate::operations::{
    create_data_property_or_throw, define_property_or_throw, get, has_property, is_callable, set,
};
use crate::operators::{BinaryOp, apply_string_or_numeric_binary_operator, type_of};
use crate::realm::{GlobalReference, Intrinsic, RealmId};
use crate::reference::{
    copy_data_properties, delete_property, get_value, instanceof_operator, put_value,
    to_object_base,
};
use crate::value::{Kind, Ref, Value};

/// Die Register vor dem ersten eines Rahmens: die Konstanten und der
/// aktuelle Kontext.
const HEADER: usize = 2;
/// Wie weit das Register mit den Konstanten vor dem ersten des Rahmens
/// liegt.
const POOL: usize = 2;
/// Wie weit das Register mit dem aktuellen Kontext davor liegt.
const CONTEXT: usize = 1;

/// Was nach einem Befehl geschieht.
enum Flow {
    Next,
    Jump(usize),
    Return,
    /// Ruft die Funktion im Platz `callee` des Registerstapels auf, `this`
    /// liegt dahinter und dann `count` Argumente.
    Call {
        callee: usize,
        count: usize,
    },
}

/// Der Bytecode eines Rahmens: geliehen für ein Script, geteilt für
/// Funktionen.
enum Source<'c> {
    Borrowed(&'c Code),
    Shared(Rc<Code>),
}

impl Source<'_> {
    fn code(&self) -> &Code {
        match self {
            Source::Borrowed(code) => code,
            Source::Shared(code) => code,
        }
    }
}

/// Ein Rahmen auf dem Stapel eines Laufs.
struct Activation<'c> {
    source: Source<'c>,
    realm: RealmId,
    /// Das erste Register des Rahmens im Registerstapel, hinter dem Kopf.
    base: usize,
    /// Der nächste Befehl.
    offset: usize,
}

/// Wo die Argumente eines Aufrufs liegen.
enum Arguments<'a, 'l> {
    /// In Plätzen des Registerstapels.
    Registers {
        first: usize,
        count: usize,
    },
    Locals(&'a [Local<'l>]),
}

/// Der oberste Rahmen, während er einen Befehl ausführt.
struct Frame<'c, 's> {
    code: &'c Code,
    realm: RealmId,
    /// Das erste Register des Rahmens im Registerstapel.
    base: usize,
    accumulator: Local<'s>,
}

/// Der Platz `offset` hinter `base` im Registerstapel; einer, den es nicht
/// gibt, liest *undefined* und schreibt nichts.
fn place(base: usize, offset: usize) -> usize {
    base.saturating_add(offset)
}

/// Führt `code` als Rahmen mit `this` aus und gibt den Akkumulator bei
/// [`Op::Return`] zurück. Reicht der Registerstapel nicht, wirft das einen
/// RangeError. Funktionen in den Konstanten braucht [`execute_script`];
/// ohne sie führen die Tests von Hand gebauten Code aus.
#[cfg(test)]
pub(crate) fn execute<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    code: &Code,
    this: Local<'_>,
) -> Completion<Local<'s>> {
    start(agent, scope, realm, code, None, this, &[])
}

/// Dasselbe für den Code eines Scripts aus dem Syntaxbaum `unit`. Die
/// Register in `flags` beginnen mit *true*.
pub(crate) fn execute_script<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    code: &Code,
    unit: &Rc<Ast>,
    this: Local<'_>,
    flags: &[u32],
) -> Completion<Local<'s>> {
    start(agent, scope, realm, code, Some(unit), this, flags)
}

fn start<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    code: &Code,
    unit: Option<&Rc<Ast>>,
    this: Local<'_>,
    flags: &[u32],
) -> Completion<Local<'s>> {
    agent.enter_interpreter()?;
    let result = start_frame(agent, scope, realm, code, unit, this, flags);
    agent.leave_interpreter();
    result
}

fn start_frame<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    code: &Code,
    unit: Option<&Rc<Ast>>,
    this: Local<'_>,
    flags: &[u32],
) -> Completion<Local<'s>> {
    let pool = constant_pool(agent, scope, code, unit)?;
    let count = HEADER.checked_add(code.register_count());
    let Some(header) = count.and_then(|count| agent.heap_mut().push_registers(count)) else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    let base = place(header, HEADER);
    let heap = agent.heap();
    heap.set_register(header, heap.get(pool));
    heap.set_register(base, heap.get(this));
    for &flag in flags {
        if let Ok(flag) = usize::try_from(flag) {
            heap.set_register(place(base, flag), Value::TRUE);
        }
    }
    let accumulator = heap.local(scope, Value::UNDEFINED);
    let activation = Activation {
        source: Source::Borrowed(code),
        realm,
        base,
        offset: 0,
    };
    let result = run(agent, activation, accumulator);
    agent.heap_mut().truncate_registers(header);
    result.map(|()| accumulator)
}

/// [[Call]] eines Funktionsobjekts (Abschnitt 10.2.1), wenn die Engine es
/// aufruft: ein neuer Lauf, der mit der Rückkehr der Funktion endet.
pub(crate) fn call_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    function: Local<'_>,
    this: Local<'_>,
    arguments: &[Local<'_>],
) -> Completion<Local<'s>> {
    agent.enter_interpreter()?;
    let result = call_frame(agent, scope, function, this, arguments);
    agent.leave_interpreter();
    result
}

fn call_frame<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    function: Local<'_>,
    this: Local<'_>,
    arguments: &[Local<'_>],
) -> Completion<Local<'s>> {
    let activation = enter_function(agent, scope, function, this, Arguments::Locals(arguments))?;
    let header = activation.base.saturating_sub(HEADER);
    let accumulator = agent.heap().local(scope, Value::UNDEFINED);
    let result = run(agent, activation, accumulator);
    agent.heap_mut().truncate_registers(header);
    result.map(|()| accumulator)
}

/// Führt Befehle aus, bis `activation` zurückkehrt. Der Rückgabewert steht
/// dann im Akkumulator. Die Register der Rahmen räumt bei einer abrupten
/// Rückkehr der Aufrufer ab.
fn run(agent: &mut Agent, activation: Activation<'_>, accumulator: Local<'_>) -> Completion<()> {
    let mut stack = vec![activation];
    loop {
        let Some(top) = stack.last_mut() else {
            return Ok(());
        };
        let code = top.source.code();
        let Some(instruction) = code.decode(top.offset) else {
            // Kann nicht vorkommen: Der Übersetzer schreibt nur gültige
            // Befehle und endet mit Return.
            return Err(agent.throw_error(ErrorKind::RangeError));
        };
        let frame = Frame {
            code,
            realm: top.realm,
            base: top.base,
            accumulator,
        };
        let flow = agent.scope(|agent, inner| frame.step(agent, inner, &instruction))?;
        match flow {
            Flow::Next => top.offset = instruction.end,
            Flow::Jump(target) => top.offset = target,
            Flow::Return => {
                let header = top.base.saturating_sub(HEADER);
                stack.pop();
                if stack.is_empty() {
                    return Ok(());
                }
                agent.heap_mut().truncate_registers(header);
            }
            Flow::Call { callee, count } => {
                top.offset = instruction.end;
                let next = agent.scope(|agent, inner| {
                    let heap = agent.heap();
                    let function = heap.local(inner, heap.register(callee));
                    let this = heap.local(inner, heap.register(place(callee, 1)));
                    let first = place(callee, 2);
                    let arguments = Arguments::Registers { first, count };
                    enter_function(agent, inner, function, this, arguments)
                })?;
                stack.push(next);
            }
        }
    }
}

/// PrepareForOrdinaryCall, OrdinaryCallBindThis und die Parameter aus
/// FunctionDeclarationInstantiation (Abschnitte 10.2.1.1, 10.2.1.2 und
/// 10.2.11): ein neuer Rahmen für das Funktionsobjekt `function` mit seinem
/// [[Environment]] als aktuellem Kontext. Fehlende Argumente bleiben
/// *undefined*, überzählige fallen weg.
///
/// Abweichung: Im nicht strikten Modus bleibt ein primitiver Wert als
/// `this` primitiv, denn Wrapper-Objekte gibt es noch nicht.
fn enter_function(
    agent: &mut Agent,
    scope: &Scope,
    function: Local<'_>,
    this: Local<'_>,
    arguments: Arguments<'_, '_>,
) -> Completion<Activation<'static>> {
    let Some((shared, realm)) = function_parts(agent.heap(), agent.heap().get(function)) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let (code, pool) = compiled(agent, scope, shared)?;
    let count = HEADER.checked_add(code.register_count());
    let Some(header) = count.and_then(|count| agent.heap_mut().push_registers(count)) else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    let value = agent.heap().get(this);
    let this = if !code.is_strict() && (value.is_undefined() || value.is_null()) {
        agent.global_object(scope, realm)
    } else {
        this
    };
    let base = place(header, HEADER);
    let heap = agent.heap();
    heap.set_register(header, heap.get(pool));
    let environment = function_environment(heap, heap.get(function)).unwrap_or(Value::UNDEFINED);
    heap.set_register(place(header, HEADER - CONTEXT), environment);
    heap.set_register(base, heap.get(this));
    heap.set_register(place(base, 1), heap.get(function));
    let parameters = code.parameter_count();
    let first_parameter = place(base, 2);
    match arguments {
        Arguments::Registers { first, count } => {
            for index in 0..count.min(parameters) {
                let value = heap.register(place(first, index));
                heap.set_register(place(first_parameter, index), value);
            }
        }
        Arguments::Locals(locals) => {
            for (index, &local) in locals.iter().take(parameters).enumerate() {
                heap.set_register(place(first_parameter, index), heap.get(local));
            }
        }
    }
    Ok(Activation {
        source: Source::Shared(code),
        realm,
        base,
        offset: 0,
    })
}

/// Bytecode und Konstanten der gemeinsamen Daten `shared`, beim ersten
/// Aufruf übersetzt. Was der Übersetzer nicht kann, endet mit
/// [`Abrupt::Unsupported`].
fn compiled<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    shared: usize,
) -> Completion<(Rc<Code>, Local<'s>)> {
    let Some(entry) = agent.functions.get(shared) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    if let Some(compiled) = &entry.compiled {
        let heap = agent.heap();
        let pool = heap.local(scope, heap.get_persistent(&compiled.pool));
        return Ok((Rc::clone(&compiled.code), pool));
    }
    let unit = Rc::clone(&entry.unit);
    let code = match compile_function(&unit, &entry.template) {
        Ok(code) => Rc::new(code),
        Err(CompileError::Unsupported(position)) => return Err(Abrupt::Unsupported(position)),
        Err(CompileError::TooLarge) => return Err(agent.throw_error(ErrorKind::RangeError)),
    };
    let pool = constant_pool(agent, scope, &code, Some(&unit))?;
    let handle = agent.heap_mut().persist(pool);
    if let Some(entry) = agent.functions.get_mut(shared) {
        entry.compiled = Some(Compiled {
            code: Rc::clone(&code),
            pool: handle,
        });
    }
    Ok((code, pool))
}

/// Legt die Konstanten im Heap an: Zahlen als Werte, Zeichenketten als
/// neue Zeichenketten, Namen als Atome und Funktionen als neue gemeinsame
/// Daten mit ihrer Nummer als Wert. Funktionen braucht es den Syntaxbaum
/// `unit`, aus dem der Code stammt.
fn constant_pool<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    code: &Code,
    unit: Option<&Rc<Ast>>,
) -> Completion<Local<'s>> {
    let pool = agent
        .heap_mut()
        .alloc_slots(scope, code.constants().len())?;
    for (index, constant) in code.constants().iter().enumerate() {
        agent.scope(|agent, inner| -> Completion<()> {
            let value = match constant {
                Constant::Number(number) => agent.heap().local(inner, Value::number(*number)),
                Constant::String(units) => agent.heap_mut().alloc_string(inner, units)?,
                Constant::Name(units) => agent.atom(inner, units)?,
                Constant::Function(template) => {
                    let Some(unit) = unit else {
                        return Err(agent.throw_error(ErrorKind::TypeError));
                    };
                    let shared = agent.add_shared_function(Rc::clone(unit), template.clone())?;
                    agent.heap().local(inner, Value::int32(shared))
                }
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

    /// Die Konstante als Wert, aus dem Kopf des Rahmens.
    fn constant<'s>(&self, agent: &Agent, scope: &'s Scope, operand: i64) -> Local<'s> {
        let heap = agent.heap();
        let pool = heap.register(self.base.checked_sub(POOL).unwrap_or(usize::MAX));
        let value = match (pool.kind(), usize::try_from(operand)) {
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
            Op::DefineGetter | Op::DefineSetter => {
                // MethodDefinitionEvaluation (Abschnitt 15.4.4): Die
                // Beschreibung hat nur [[Get]] oder [[Set]], ein vorhandener
                // Accessor behält das andere.
                let object = self.register(agent, scope, first);
                let key = self.register(agent, scope, second);
                let key = to_property_key(agent, scope, key)?;
                let mut descriptor = PropertyDescriptor {
                    enumerable: Some(true),
                    configurable: Some(true),
                    ..PropertyDescriptor::default()
                };
                if instruction.op == Op::DefineGetter {
                    descriptor.get = Some(self.accumulator);
                } else {
                    descriptor.set = Some(self.accumulator);
                }
                define_property_or_throw(agent, scope, object, key, &descriptor)?;
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

    /// Der Platz des aktuellen Kontexts im Registerstapel.
    fn context_slot(&self) -> usize {
        self.base.checked_sub(CONTEXT).unwrap_or(usize::MAX)
    }

    /// Der Kontext `depth` Stufen außerhalb des aktuellen. Der Übersetzer
    /// zählt nie über die Kette hinaus.
    fn context<'h>(&self, heap: &'h Heap, depth: i64) -> Option<Ref<'h>> {
        let mut value = heap.register(self.context_slot());
        for _ in 0..depth {
            let Kind::Internal(cell) = value.kind() else {
                return None;
            };
            value = heap.slot(cell, 0)?;
        }
        match value.kind() {
            Kind::Internal(cell) => Some(cell),
            _ => None,
        }
    }

    /// Die Befehle für Kontexte.
    fn context_operation(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        instruction: &Instruction,
    ) -> Completion<()> {
        let first = instruction.operand(0);
        let second = instruction.operand(1);
        let current = self.context_slot();
        match instruction.op {
            Op::PushContext => {
                let length = usize::try_from(first).unwrap_or(0).saturating_add(1);
                let context = agent.heap_mut().alloc_slots(scope, length)?;
                let heap = agent.heap();
                if let Kind::Internal(cell) = heap.get(context).kind() {
                    heap.set_slot(cell, 0, heap.register(current));
                }
                heap.set_register(current, heap.get(context));
            }
            Op::CloneContext => {
                let heap = agent.heap();
                let old = heap.local(scope, heap.register(current));
                let length = match heap.get(old).kind() {
                    Kind::Internal(cell) => heap.slot_count(cell),
                    _ => 0,
                };
                let context = agent.heap_mut().alloc_slots(scope, length)?;
                let heap = agent.heap();
                if let (Kind::Internal(old), Kind::Internal(new)) =
                    (heap.get(old).kind(), heap.get(context).kind())
                {
                    for index in 0..length {
                        if let Some(value) = heap.slot(old, index) {
                            heap.set_slot(new, index, value);
                        }
                    }
                }
                heap.set_register(current, heap.get(context));
            }
            Op::PopContext => {
                let heap = agent.heap();
                let outer = self
                    .context(heap, 0)
                    .and_then(|cell| heap.slot(cell, 0))
                    .unwrap_or(Value::UNDEFINED);
                heap.set_register(current, outer);
            }
            Op::LdaContext => {
                let heap = agent.heap();
                let value = usize::try_from(second)
                    .ok()
                    .and_then(|slot| heap.slot(self.context(heap, first)?, slot))
                    .unwrap_or(Value::UNDEFINED);
                heap.set(self.accumulator, value);
            }
            Op::StaContext => {
                let heap = agent.heap();
                if let (Some(cell), Ok(slot)) = (self.context(heap, first), usize::try_from(second))
                {
                    heap.set_slot(cell, slot, heap.get(self.accumulator));
                }
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
            Op::ThrowIfHole => {
                if agent.heap().get(self.accumulator).is_hole() {
                    return Err(agent.throw_error(ErrorKind::ReferenceError));
                }
            }
            Op::PushContext
            | Op::PopContext
            | Op::CloneContext
            | Op::LdaContext
            | Op::StaContext => self.context_operation(agent, scope, instruction)?,
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
            | Op::DefineGetter
            | Op::DefineSetter
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
            Op::CreateClosure | Op::CreateClosureKeyed => {
                let shared = self.constant(agent, scope, first);
                // Die Konstante ist die Nummer der gemeinsamen Daten.
                let shared = agent.heap().get(shared).as_int32().unwrap_or(-1);
                let name = if instruction.op == Op::CreateClosure {
                    self.name(agent, scope, second).1
                } else {
                    let key = self.register(agent, scope, second);
                    to_property_key(agent, scope, key)?
                };
                let environment = {
                    let heap = agent.heap();
                    heap.local(scope, heap.register(self.context_slot()))
                };
                let function =
                    instantiate_closure(agent, scope, self.realm, shared, name, environment)?;
                self.set_accumulator(agent, function);
            }
            Op::Call => {
                // EvaluateCall (Abschnitt 13.3.6.2): IsCallable erst nach
                // den Argumenten.
                let callee = self.register(agent, scope, first);
                if !is_callable(agent.heap(), agent.heap().get(callee)) {
                    return Err(agent.throw_error(ErrorKind::TypeError));
                }
                let count = usize::try_from(second).unwrap_or(0);
                return Ok(Flow::Call {
                    callee: self.slot(first),
                    count,
                });
            }
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
