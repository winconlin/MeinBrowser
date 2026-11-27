//! Das Format eines Eintrags, Schreiben und Lesen.
//!
//! Ein Eintrag beginnt mit dem Kopf: `MBJSCODE`, der Fassung [`FORMAT`] als
//! vier Bytes mit dem niedrigsten zuerst und den 32 Bytes des Schlüssels.
//! Dann folgen die Tabelle der [`Enclosing`], das Script, die Funktionen
//! und die Klassen. Zahlen stehen als LEB128 ohne Vorzeichen in kürzester
//! Form, Codeeinheiten und Gleitkommazahlen mit festen Bytes, das niedrigste
//! zuerst, Wahrheitswerte als 0 oder 1. Eine Liste beginnt mit ihrer Länge.
//!
//! Die [`Enclosing`] teilen sich ihre äußeren Stufen über `Rc`. Damit das
//! geteilt bleibt, stehen sie einmal in der Tabelle, jede nach ihrer
//! äußeren; der Rest nennt sie mit ihrer Stelle dort. Alle leeren teilen
//! sich einen Eintrag.
//!
//! Der Leser prüft nur die Form: dass jede Zahl passt, jede Art bekannt ist,
//! Namen in Tabellen aufsteigen und nichts übrig bleibt. Ob der Bytecode
//! stimmt, prüft danach `verify.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use mb_crypto::Digest;

use super::{CompiledFunction, CompiledScript, FORMAT, Rejected};
use crate::ast::{Atom, ClassId, FunctionId, FunctionKind};
use crate::bigint::BigInt;
use crate::bytecode::{
    BindingKind, Code, Constant, ContextBinding, ContextLayout, DynamicName, Enclosing, EvalSite,
    Extension, Fallback, FunctionScope, FunctionTemplate, Handler, HandlerKind,
};
use crate::compiler::{CompileError, Script};
use crate::unit::Shape;

const MAGIC: &[u8; 8] = b"MBJSCODE";

/// Die Arten der Konstanten.
const NUMBER: u8 = 0;
const BIGINT: u8 = 1;
const STRING: u8 = 2;
const NAME: u8 = 3;
const FUNCTION: u8 = 4;
const DEFAULT_CONSTRUCTOR: u8 = 5;
const DYNAMIC: u8 = 6;
const EVAL_SITE: u8 = 7;
const MODULE: u8 = 8;
/// Für die Konstanten aus dem Code eines `eval`. Sie kommen in einem Script
/// nicht vor; der Leser lehnt sie ab.
const UNSUPPORTED: u8 = 0xFF;

fn reject<T>(reason: &'static str) -> Result<T, Rejected> {
    Err(Rejected::new(reason))
}

// Schreiben

pub(super) fn encode(script: &CompiledScript, key: &Digest) -> Vec<u8> {
    let mut body = Writer::default();
    body.script(&script.script);
    body.count(script.functions.len());
    for (id, function) in &script.functions {
        body.u32(id.to_raw());
        body.function(function);
    }
    body.count(script.classes.len());
    for &(id, start, end) in &script.classes {
        body.u32(id.to_raw());
        body.u32(start);
        body.u32(end);
    }
    let mut out = Writer::default();
    out.bytes.extend_from_slice(MAGIC);
    out.bytes.extend_from_slice(&FORMAT.to_le_bytes());
    out.bytes.extend_from_slice(key.as_bytes());
    out.count(body.table.len());
    for enclosing in &body.table {
        out.count(enclosing.contexts.len());
        for layout in &enclosing.contexts {
            out.layout(layout);
        }
        out.count(enclosing.registers.len());
        for atom in &enclosing.registers {
            out.u32(atom.to_raw());
        }
        // Jede äußere Stufe steht schon davor in der Tabelle.
        let parent = enclosing.parent.as_ref().map_or(0, |parent| {
            body.indices
                .get(&address(parent))
                .map_or(u64::MAX, |index| index.saturating_add(1))
        });
        out.u64(parent);
    }
    out.bytes.extend_from_slice(&body.bytes);
    out.bytes
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    /// Die Tabelle der [`Enclosing`], jede nach ihrer äußeren.
    table: Vec<Rc<Enclosing>>,
    /// Ihre Stellen dort nach ihrer Adresse; die leeren unter der Adresse
    /// null.
    indices: HashMap<*const Enclosing, u64>,
}

fn is_empty(enclosing: &Enclosing) -> bool {
    enclosing.contexts.is_empty() && enclosing.registers.is_empty() && enclosing.parent.is_none()
}

fn address(enclosing: &Rc<Enclosing>) -> *const Enclosing {
    if is_empty(enclosing) {
        std::ptr::null()
    } else {
        Rc::as_ptr(enclosing)
    }
}

impl Writer {
    fn byte(&mut self, byte: u8) {
        self.bytes.push(byte);
    }

    fn bool(&mut self, value: bool) {
        self.byte(u8::from(value));
    }

    fn u64(&mut self, mut value: u64) {
        loop {
            let low = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                self.byte(low);
                return;
            }
            self.byte(low | 0x80);
        }
    }

    fn u32(&mut self, value: u32) {
        self.u64(u64::from(value));
    }

    fn count(&mut self, count: usize) {
        self.u64(u64::try_from(count).unwrap_or(u64::MAX));
    }

    /// `None` als 0, sonst die Zahl plus eins.
    fn option(&mut self, value: Option<u32>) {
        self.u64(value.map_or(0, |value| u64::from(value).saturating_add(1)));
    }

    fn units(&mut self, units: &[u16]) {
        self.count(units.len());
        for unit in units {
            self.bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }

    /// Die Stelle von `enclosing` in der Tabelle, mit allen äußeren Stufen,
    /// die dort noch fehlen.
    fn index(&mut self, enclosing: &Rc<Enclosing>) -> u64 {
        if let Some(&index) = self.indices.get(&address(enclosing)) {
            return index;
        }
        let mut missing = Vec::new();
        let mut current = Some(enclosing);
        while let Some(level) = current {
            if self.indices.contains_key(&address(level)) {
                break;
            }
            missing.push(Rc::clone(level));
            current = level.parent.as_ref();
        }
        for level in missing.into_iter().rev() {
            let index = u64::try_from(self.table.len()).unwrap_or(u64::MAX);
            self.indices.insert(address(&level), index);
            self.table.push(level);
        }
        self.indices
            .get(&address(enclosing))
            .copied()
            .unwrap_or(u64::MAX)
    }

    fn enclosing(&mut self, enclosing: &Rc<Enclosing>) {
        let index = self.index(enclosing);
        self.u64(index);
    }

    fn function_scope(&mut self, scope: Option<FunctionScope>) {
        let value = scope.map_or(0, |scope| {
            1 | u8::from(scope.method) << 1
                | u8::from(scope.derived) << 2
                | u8::from(scope.field_initializer) << 3
        });
        self.byte(value);
    }

    fn layout(&mut self, layout: &ContextLayout) {
        self.count(layout.bindings.len());
        for (atom, binding) in &layout.bindings {
            self.u32(atom.to_raw());
            self.u32(binding.slot);
            self.byte(binding_kind(binding.kind));
            self.bool(binding.lexical);
        }
        self.option(layout.this);
        self.option(layout.new_target);
        self.option(layout.function);
        self.bool(layout.derived);
        self.count(layout.privates.len());
        for (atom, &slot) in &layout.privates {
            self.u32(atom.to_raw());
            self.u32(slot);
        }
        self.option(layout.extension);
        self.bool(layout.with);
        self.bool(layout.var_scope);
        self.bool(layout.catch);
        self.function_scope(layout.function_scope);
    }

    fn script(&mut self, script: &Script) {
        self.code(&script.code);
        self.count(script.var_names.len());
        for name in &script.var_names {
            self.units(name);
        }
        self.count(script.lexical_names.len());
        for (name, constant) in &script.lexical_names {
            self.units(name);
            self.bool(*constant);
        }
        self.count(script.functions.len());
        for (name, function) in &script.functions {
            self.units(name);
            self.u32(function.to_raw());
        }
        self.count(script.hoisted_functions.len());
        for (name, register) in &script.hoisted_functions {
            self.units(name);
            self.u32(*register);
        }
    }

    fn function(&mut self, function: &CompiledFunction) {
        self.byte(function_kind(function.shape.kind));
        self.bool(function.shape.generator);
        self.bool(function.shape.is_async);
        self.u32(function.shape.length);
        self.u32(function.text.0);
        self.u32(function.text.1);
        match &function.code {
            Ok(code) => {
                self.byte(0);
                self.code(code);
            }
            Err(CompileError::Unsupported(position)) => {
                self.byte(1);
                self.u64(u64::try_from(*position).unwrap_or(u64::MAX));
            }
            Err(CompileError::TooLarge) => self.byte(2),
        }
    }

    fn code(&mut self, code: &Code) {
        self.count(code.bytes.len());
        self.bytes.extend_from_slice(&code.bytes);
        self.count(code.constants.len());
        for constant in &code.constants {
            self.constant(constant);
        }
        self.u32(code.registers);
        self.u32(code.parameters);
        self.option(code.arguments);
        self.count(code.parameter_map.len());
        for &slot in &code.parameter_map {
            self.option(slot);
        }
        self.option(code.new_target);
        self.option(code.generator);
        self.option(code.async_state);
        self.bool(code.async_generator);
        self.bool(code.strict);
        self.count(code.strict_ranges.len());
        for &(start, end) in &code.strict_ranges {
            self.u32(start);
            self.u32(end);
        }
        self.bool(code.derived);
        self.count(code.positions.len());
        for &(offset, position) in &code.positions {
            self.u32(offset);
            self.u32(position);
        }
        self.count(code.handlers.len());
        for handler in &code.handlers {
            for value in [handler.start, handler.end, handler.target] {
                self.u64(u64::try_from(value).unwrap_or(u64::MAX));
            }
            self.byte(match handler.kind {
                HandlerKind::Catch => 0,
                HandlerKind::Finally => 1,
            });
        }
    }

    fn constant(&mut self, constant: &Constant) {
        match constant {
            Constant::Number(number) => {
                self.byte(NUMBER);
                self.bytes
                    .extend_from_slice(&number.to_bits().to_le_bytes());
            }
            Constant::BigInt(value) => {
                self.byte(BIGINT);
                self.bool(value.is_negative());
                self.count(value.magnitude().len());
                for limb in value.magnitude() {
                    self.bytes.extend_from_slice(&limb.to_le_bytes());
                }
            }
            Constant::String(units) => {
                self.byte(STRING);
                self.units(units);
            }
            Constant::Name(units) => {
                self.byte(NAME);
                self.units(units);
            }
            Constant::Function(template) => {
                self.byte(FUNCTION);
                self.u32(template.function.to_raw());
                self.bool(template.binds_name);
                self.enclosing(&template.outer);
            }
            Constant::DefaultConstructor { derived, class } => {
                self.byte(DEFAULT_CONSTRUCTOR);
                self.bool(*derived);
                self.u32(class.to_raw());
            }
            Constant::Dynamic(dynamic) => {
                self.byte(DYNAMIC);
                self.units(&dynamic.name);
                self.count(dynamic.extensions.len());
                for extension in &dynamic.extensions {
                    self.u32(extension.depth);
                    self.u32(extension.slot);
                    self.bool(extension.with);
                }
                match dynamic.fallback {
                    Fallback::Global => self.byte(0),
                    Fallback::Context {
                        depth,
                        slot,
                        kind,
                        lexical,
                    } => {
                        self.byte(1);
                        self.u32(depth);
                        self.u32(slot);
                        self.byte(binding_kind(kind));
                        self.bool(lexical);
                    }
                }
            }
            Constant::EvalSite(site) => {
                self.byte(EVAL_SITE);
                self.u32(site.arguments);
                self.bool(site.spread);
                self.bool(site.strict);
                self.bool(site.tail);
                self.function_scope(site.function);
                self.enclosing(&site.outer);
                self.count(site.names.len());
                for (atom, name) in &site.names {
                    self.u32(atom.to_raw());
                    self.units(name);
                }
            }
            Constant::Module => self.byte(MODULE),
            Constant::EvalInstantiation(_) | Constant::EvalBinding(_) => self.byte(UNSUPPORTED),
        }
    }
}

fn binding_kind(kind: BindingKind) -> u8 {
    match kind {
        BindingKind::Mutable => 0,
        BindingKind::Const => 1,
        BindingKind::FunctionName => 2,
        BindingKind::Import => 3,
    }
}

fn function_kind(kind: FunctionKind) -> u8 {
    match kind {
        FunctionKind::Normal => 0,
        FunctionKind::Arrow => 1,
        FunctionKind::Method => 2,
        FunctionKind::Getter => 3,
        FunctionKind::Setter => 4,
        FunctionKind::Constructor => 5,
        FunctionKind::DerivedConstructor => 6,
        FunctionKind::StaticBlock => 7,
        FunctionKind::FieldInitializer => 8,
    }
}

// Lesen

pub(super) fn decode(
    bytes: &[u8],
    source: &[u16],
    key: &Digest,
) -> Result<CompiledScript, Rejected> {
    let mut reader = Reader { bytes, at: 0 };
    if reader.take(MAGIC.len())? != MAGIC {
        return reject("keine Kennung");
    }
    if reader.take(4)? != FORMAT.to_le_bytes() {
        return reject("andere Fassung des Formats");
    }
    if reader.take(key.as_bytes().len())? != key.as_bytes() {
        return reject("Schlüssel passt nicht zum Quelltext");
    }
    let count = reader.count()?;
    let mut table: Vec<Rc<Enclosing>> = Vec::new();
    for _ in 0..count {
        let enclosing = reader.enclosing_entry(&table)?;
        table.push(Rc::new(enclosing));
    }
    let script = reader.script(&table)?;
    let count = reader.count()?;
    let mut functions: Vec<(FunctionId, CompiledFunction)> = Vec::new();
    for _ in 0..count {
        let id = reader.u32()?;
        if functions
            .last()
            .is_some_and(|(last, _)| last.to_raw() >= id)
        {
            return reject("Funktionen nicht aufsteigend");
        }
        let function = reader.function(&table)?;
        functions.push((FunctionId::from_raw(id), function));
    }
    let count = reader.count()?;
    let mut classes: Vec<(ClassId, u32, u32)> = Vec::new();
    for _ in 0..count {
        let id = reader.u32()?;
        if classes
            .last()
            .is_some_and(|(last, _, _)| last.to_raw() >= id)
        {
            return reject("Klassen nicht aufsteigend");
        }
        let start = reader.u32()?;
        let end = reader.u32()?;
        classes.push((ClassId::from_raw(id), start, end));
    }
    if reader.at != bytes.len() {
        return reject("Bytes hinter dem Eintrag");
    }
    Ok(CompiledScript {
        source: source.to_vec(),
        script,
        functions,
        classes,
    })
}

struct Reader<'b> {
    bytes: &'b [u8],
    at: usize,
}

impl<'b> Reader<'b> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    fn take(&mut self, length: usize) -> Result<&'b [u8], Rejected> {
        let end = self
            .at
            .checked_add(length)
            .filter(|&end| end <= self.bytes.len());
        let Some(slice) = end.and_then(|end| self.bytes.get(self.at..end)) else {
            return reject("Eintrag zu kurz");
        };
        self.at = self.at.saturating_add(length);
        Ok(slice)
    }

    fn byte(&mut self) -> Result<u8, Rejected> {
        match self.take(1)? {
            &[byte] => Ok(byte),
            _ => reject("Eintrag zu kurz"),
        }
    }

    fn bool(&mut self) -> Result<bool, Rejected> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => reject("kein Wahrheitswert"),
        }
    }

    fn u64(&mut self) -> Result<u64, Rejected> {
        let mut value = 0_u64;
        for index in 0..10_u32 {
            let byte = self.byte()?;
            let low = u64::from(byte & 0x7F);
            let shift = index * 7;
            // Das zehnte Byte trägt nur noch ein Bit.
            if index == 9 && low > 1 {
                return reject("Zahl zu groß");
            }
            value |= low << shift;
            if byte & 0x80 == 0 {
                if byte == 0 && index > 0 {
                    return reject("Zahl nicht in kürzester Form");
                }
                return Ok(value);
            }
        }
        reject("Zahl zu lang")
    }

    fn u32(&mut self) -> Result<u32, Rejected> {
        u32::try_from(self.u64()?).or_else(|_| reject("Zahl zu groß"))
    }

    fn usize(&mut self) -> Result<usize, Rejected> {
        usize::try_from(self.u64()?).or_else(|_| reject("Zahl zu groß"))
    }

    /// Die Länge einer Liste, deren Elemente je mindestens `width` Bytes
    /// brauchen. Mehr, als übrig ist, kann sie nicht haben.
    fn count_of(&mut self, width: usize) -> Result<usize, Rejected> {
        let count = self.usize()?;
        if count
            .checked_mul(width)
            .is_none_or(|bytes| bytes > self.remaining())
        {
            return reject("Liste länger als der Eintrag");
        }
        Ok(count)
    }

    fn count(&mut self) -> Result<usize, Rejected> {
        self.count_of(1)
    }

    fn option(&mut self) -> Result<Option<u32>, Rejected> {
        match self.u64()? {
            0 => Ok(None),
            value => u32::try_from(value - 1)
                .map(Some)
                .or_else(|_| reject("Zahl zu groß")),
        }
    }

    fn units(&mut self) -> Result<Vec<u16>, Rejected> {
        let count = self.count_of(2)?;
        let bytes = self.take(count.saturating_mul(2))?;
        let (pairs, _) = bytes.as_chunks::<2>();
        Ok(pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect())
    }

    fn atom(&mut self) -> Result<Atom, Rejected> {
        Atom::from_raw(self.u32()?).ok_or(Rejected::new("Atom 0"))
    }

    fn enclosing(&mut self, table: &[Rc<Enclosing>]) -> Result<Rc<Enclosing>, Rejected> {
        let index = self.usize()?;
        match table.get(index) {
            Some(enclosing) => Ok(Rc::clone(enclosing)),
            None => reject("Stufe nicht in der Tabelle"),
        }
    }

    /// Ein Eintrag der Tabelle; seine äußere Stufe steht schon in `table`.
    fn enclosing_entry(&mut self, table: &[Rc<Enclosing>]) -> Result<Enclosing, Rejected> {
        let count = self.count()?;
        let mut contexts = Vec::new();
        for _ in 0..count {
            contexts.push(self.layout()?);
        }
        let count = self.count()?;
        let mut registers = BTreeSet::new();
        let mut last = None;
        for _ in 0..count {
            let atom = self.atom()?;
            if last.is_some_and(|last| last >= atom) {
                return reject("Namen nicht aufsteigend");
            }
            last = Some(atom);
            registers.insert(atom);
        }
        let parent = match self.usize()? {
            0 => None,
            index => match table.get(index - 1) {
                Some(parent) => Some(Rc::clone(parent)),
                None => return reject("äußere Stufe nicht davor"),
            },
        };
        Ok(Enclosing {
            contexts,
            registers,
            parent,
        })
    }

    fn function_scope(&mut self) -> Result<Option<FunctionScope>, Rejected> {
        let byte = self.byte()?;
        match byte {
            0 => Ok(None),
            _ if byte & 1 == 1 && byte >> 4 == 0 => Ok(Some(FunctionScope {
                method: byte & 2 != 0,
                derived: byte & 4 != 0,
                field_initializer: byte & 8 != 0,
            })),
            _ => reject("keine Angaben zur Funktion"),
        }
    }

    fn binding_kind(&mut self) -> Result<BindingKind, Rejected> {
        Ok(match self.byte()? {
            0 => BindingKind::Mutable,
            1 => BindingKind::Const,
            2 => BindingKind::FunctionName,
            3 => BindingKind::Import,
            _ => return reject("unbekannte Art der Bindung"),
        })
    }

    fn layout(&mut self) -> Result<ContextLayout, Rejected> {
        let count = self.count()?;
        let mut bindings = BTreeMap::new();
        let mut last = None;
        for _ in 0..count {
            let atom = self.atom()?;
            if last.is_some_and(|last| last >= atom) {
                return reject("Namen nicht aufsteigend");
            }
            last = Some(atom);
            let slot = self.u32()?;
            let kind = self.binding_kind()?;
            let lexical = self.bool()?;
            bindings.insert(
                atom,
                ContextBinding {
                    slot,
                    kind,
                    lexical,
                },
            );
        }
        let this = self.option()?;
        let new_target = self.option()?;
        let function = self.option()?;
        let derived = self.bool()?;
        let count = self.count()?;
        let mut privates = BTreeMap::new();
        let mut last = None;
        for _ in 0..count {
            let atom = self.atom()?;
            if last.is_some_and(|last| last >= atom) {
                return reject("Namen nicht aufsteigend");
            }
            last = Some(atom);
            privates.insert(atom, self.u32()?);
        }
        Ok(ContextLayout {
            bindings,
            this,
            new_target,
            function,
            derived,
            privates,
            extension: self.option()?,
            with: self.bool()?,
            var_scope: self.bool()?,
            catch: self.bool()?,
            function_scope: self.function_scope()?,
        })
    }

    fn script(&mut self, table: &[Rc<Enclosing>]) -> Result<Script, Rejected> {
        let code = self.code(table)?;
        let count = self.count()?;
        let mut var_names = Vec::new();
        for _ in 0..count {
            var_names.push(self.units()?);
        }
        let count = self.count()?;
        let mut lexical_names = Vec::new();
        for _ in 0..count {
            lexical_names.push((self.units()?, self.bool()?));
        }
        let count = self.count()?;
        let mut functions = Vec::new();
        for _ in 0..count {
            functions.push((self.units()?, FunctionId::from_raw(self.u32()?)));
        }
        let count = self.count()?;
        let mut hoisted_functions = Vec::new();
        for _ in 0..count {
            hoisted_functions.push((self.units()?, self.u32()?));
        }
        Ok(Script {
            code,
            var_names,
            lexical_names,
            functions,
            hoisted_functions,
        })
    }

    fn function(&mut self, table: &[Rc<Enclosing>]) -> Result<CompiledFunction, Rejected> {
        let kind = match self.byte()? {
            0 => FunctionKind::Normal,
            1 => FunctionKind::Arrow,
            2 => FunctionKind::Method,
            3 => FunctionKind::Getter,
            4 => FunctionKind::Setter,
            5 => FunctionKind::Constructor,
            6 => FunctionKind::DerivedConstructor,
            7 => FunctionKind::StaticBlock,
            8 => FunctionKind::FieldInitializer,
            _ => return reject("unbekannte Art der Funktion"),
        };
        let shape = Shape {
            kind,
            generator: self.bool()?,
            is_async: self.bool()?,
            length: self.u32()?,
        };
        let text = (self.u32()?, self.u32()?);
        let code = match self.byte()? {
            0 => Ok(Rc::new(self.code(table)?)),
            1 => Err(CompileError::Unsupported(self.usize()?)),
            2 => Err(CompileError::TooLarge),
            _ => return reject("unbekanntes Ergebnis des Übersetzers"),
        };
        Ok(CompiledFunction { shape, text, code })
    }

    fn code(&mut self, table: &[Rc<Enclosing>]) -> Result<Code, Rejected> {
        let count = self.count()?;
        let bytes = self.take(count)?.to_vec();
        let count = self.count()?;
        let mut constants = Vec::new();
        for _ in 0..count {
            constants.push(self.constant(table)?);
        }
        let registers = self.u32()?;
        let parameters = self.u32()?;
        let arguments = self.option()?;
        let count = self.count()?;
        let mut parameter_map = Vec::new();
        for _ in 0..count {
            parameter_map.push(self.option()?);
        }
        let new_target = self.option()?;
        let generator = self.option()?;
        let async_state = self.option()?;
        let async_generator = self.bool()?;
        let strict = self.bool()?;
        let count = self.count_of(2)?;
        let mut strict_ranges = Vec::new();
        for _ in 0..count {
            strict_ranges.push((self.u32()?, self.u32()?));
        }
        let derived = self.bool()?;
        let count = self.count_of(2)?;
        let mut positions = Vec::new();
        for _ in 0..count {
            positions.push((self.u32()?, self.u32()?));
        }
        let count = self.count_of(4)?;
        let mut handlers = Vec::new();
        for _ in 0..count {
            let start = self.usize()?;
            let end = self.usize()?;
            let target = self.usize()?;
            let kind = match self.byte()? {
                0 => HandlerKind::Catch,
                1 => HandlerKind::Finally,
                _ => return reject("unbekannte Art des Handlers"),
            };
            handlers.push(Handler {
                start,
                end,
                target,
                kind,
            });
        }
        Ok(Code {
            bytes,
            constants,
            registers,
            parameters,
            arguments,
            parameter_map,
            new_target,
            generator,
            async_state,
            async_generator,
            strict,
            strict_ranges,
            derived,
            positions,
            handlers,
        })
    }

    fn constant(&mut self, table: &[Rc<Enclosing>]) -> Result<Constant, Rejected> {
        Ok(match self.byte()? {
            NUMBER => {
                let bytes = self.take(8)?;
                let mut bits = [0; 8];
                bits.copy_from_slice(bytes);
                Constant::Number(f64::from_bits(u64::from_le_bytes(bits)))
            }
            BIGINT => {
                let negative = self.bool()?;
                let count = self.count_of(8)?;
                let bytes = self.take(count.saturating_mul(8))?;
                let (limbs, _) = bytes.as_chunks::<8>();
                let magnitude: Vec<u64> =
                    limbs.iter().map(|&limb| u64::from_le_bytes(limb)).collect();
                // Nur die kürzeste Form: kein führendes Null-Glied, keine
                // negative Null.
                if magnitude.last() == Some(&0) || (negative && magnitude.is_empty()) {
                    return reject("BigInt nicht in kürzester Form");
                }
                match BigInt::from_parts(negative, magnitude) {
                    Ok(value) => Constant::BigInt(value),
                    Err(_) => return reject("BigInt zu groß"),
                }
            }
            STRING => Constant::String(self.units()?),
            NAME => Constant::Name(self.units()?),
            FUNCTION => Constant::Function(FunctionTemplate {
                function: FunctionId::from_raw(self.u32()?),
                binds_name: self.bool()?,
                outer: self.enclosing(table)?,
            }),
            DEFAULT_CONSTRUCTOR => Constant::DefaultConstructor {
                derived: self.bool()?,
                class: ClassId::from_raw(self.u32()?),
            },
            DYNAMIC => {
                let name = self.units()?;
                let count = self.count_of(3)?;
                let mut extensions = Vec::new();
                for _ in 0..count {
                    extensions.push(Extension {
                        depth: self.u32()?,
                        slot: self.u32()?,
                        with: self.bool()?,
                    });
                }
                let fallback = match self.byte()? {
                    0 => Fallback::Global,
                    1 => Fallback::Context {
                        depth: self.u32()?,
                        slot: self.u32()?,
                        kind: self.binding_kind()?,
                        lexical: self.bool()?,
                    },
                    _ => return reject("unbekannte Art der Bindung dahinter"),
                };
                Constant::Dynamic(DynamicName {
                    name,
                    extensions,
                    fallback,
                })
            }
            EVAL_SITE => {
                let arguments = self.u32()?;
                let spread = self.bool()?;
                let strict = self.bool()?;
                let tail = self.bool()?;
                let function = self.function_scope()?;
                let outer = self.enclosing(table)?;
                let count = self.count_of(2)?;
                let mut names = BTreeMap::new();
                let mut last = None;
                for _ in 0..count {
                    let atom = self.atom()?;
                    if last.is_some_and(|last| last >= atom) {
                        return reject("Namen nicht aufsteigend");
                    }
                    last = Some(atom);
                    names.insert(atom, self.units()?);
                }
                Constant::EvalSite(Rc::new(EvalSite {
                    arguments,
                    spread,
                    strict,
                    tail,
                    function,
                    outer,
                    names,
                }))
            }
            MODULE => Constant::Module,
            _ => return reject("unbekannte Art der Konstante"),
        })
    }
}
