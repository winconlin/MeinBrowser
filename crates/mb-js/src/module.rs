//! Module (Abschnitt 16.2): Source Text Module Records, Synthetic Module
//! Records für JSON-Module, das Laden über den Host, Link und Evaluate.
//!
//! Der Agent hält alle Module Records in einer Liste, nach ihrer Nummer
//! ([`ModuleId`]); sie leben so lange wie er. Werte des Heaps, die ein
//! Record hält, sind dauerhafte Handles: die Umgebung, das
//! Namensraum-Objekt, `import.meta`, das Promise aus [[TopLevelCapability]]
//! und der Fehler aus [[EvaluationError]].
//!
//! Die Umgebung eines Moduls (Module Environment Record, Abschnitt 9.1.1.5)
//! ist ein Kontext wie der eines Blocks: eine interne Zelle, deren erstes
//! Feld der äußere Kontext ist, hier *undefined* für die globale Umgebung,
//! und deren übrige Felder die Bindungen auf oberster Ebene halten. Sie
//! entsteht schon mit dem Record, alle Bindungen in der Lücke;
//! InitializeEnvironment füllt sie. Ein Zugriff davor, den nur ein
//! Namensraum-Objekt versuchen könnte, wirft wie vorgeschrieben einen
//! ReferenceError. Eine Import-Bindung (CreateImportBinding) hält eine
//! interne Zelle mit dem Kontext und dem Platz der Zielbindung; lesen geht
//! über [`Op::LdaImport`](crate::bytecode::Op::LdaImport). Ein importierter
//! Namensraum steht direkt in ihrem Platz.
//!
//! Laden, Linken und Auswerten gehen den Graphen der Module in der Tiefe
//! durch. Statt Rekursion halten Schleifen eigene Stapel (RS-8): Ein Graph
//! aus vielen Modulen hintereinander belegt keinen Rust-Stapel. Auch
//! ResolveExport und GetExportedNames laufen so.
//!
//! Ein Modul mit `await` auf oberster Ebene ([[HasTLA]]) läuft wie der
//! Rumpf einer async-Funktion; Evaluate führt es und die Module, die auf
//! es warten, mit den Schritten aus `asynchronous` aus. `import()` beginnt
//! in `dynamic` und lädt über denselben Hook des Hosts wie die statischen
//! Importe.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::ast::Ast;
use crate::bytecode::Code;
use crate::compiler::{CompileError, DEFAULT_BINDING, ModuleBinding, ModuleInit, compile_module};
use crate::function::instantiate_closure;
use crate::heap::{Local, Persistent, Scope};
use crate::interpreter::execute_module;
use crate::object::ordinary_object_create;
use crate::operations::create_data_property_or_throw;
use crate::parser::parse_module;
use crate::promise::{promise_create, settle_pending};
use crate::realm::{Intrinsic, RealmId};
use crate::script::ScriptError;
use crate::value::{Kind, Value};

mod asynchronous;
pub(crate) mod dynamic;
mod entries;
pub(crate) mod loading;
pub(crate) mod namespace;

pub(crate) use dynamic::evaluate_import_call;

pub use loading::LoadPayload;

/// Die Nummer eines Module Records im Agenten.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(usize);

impl ModuleId {
    pub fn index(self) -> usize {
        self.0
    }

    pub(crate) fn from_index(index: usize) -> ModuleId {
        ModuleId(index)
    }
}

/// Ein ModuleRequest Record (Abschnitt 16.2.1.3.1): der Bezeichner und die
/// Attribute aus `with`, nach ihrem Schlüssel geordnet wie in
/// WithClauseToAttributes. Ein Schlüssel kommt höchstens einmal vor, das
/// prüft der Parser; so ist ModuleRequestsEqual die Gleichheit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleRequest {
    pub specifier: Vec<u16>,
    pub attributes: Vec<(Vec<u16>, Vec<u16>)>,
}

impl ModuleRequest {
    /// Der Wert des Attributs `key`, wenn es eines gibt.
    pub fn attribute(&self, key: &str) -> Option<&[u16]> {
        self.attributes
            .iter()
            .find(|(name, _)| name.iter().copied().eq(key.encode_utf16()))
            .map(|(_, value)| value.as_slice())
    }
}

/// Wer ein Modul anfordert (*referrer* in HostLoadImportedModule): ein
/// Modul mit seinen Importen oder seinem `import()`, sonst der Realm. Ein
/// Script hat noch keinen Script Record; `import()` aus einem Script oder
/// aus Code von `eval` und `Function` nennt darum den aktuellen Realm, wie
/// EvaluateImportCall es für einen fehlenden ScriptOrModule tut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Referrer {
    Module(ModuleId),
    Realm(RealmId),
}

/// [[Status]] eines Cyclic Module Record (Abschnitt 16.2.1.6). Ein
/// Synthetic Module Record hat keinen; er gilt als ~new~, bis er gelinkt
/// ist, dann als ~linked~ und nach Evaluate als ~evaluated~.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleStatus {
    New,
    Unlinked,
    Linking,
    Linked,
    Evaluating,
    EvaluatingAsync,
    Evaluated,
}

/// [[ImportName]] eines ImportEntry Record oder eines ExportEntry Record
/// mit Modul (Abschnitt 16.2.1.7).
#[derive(Clone, Debug, PartialEq, Eq)]
enum ImportName {
    Name(Vec<u16>),
    /// ~namespace-object~ bei `import * as`, ~all~ bei `export * as`.
    Namespace,
}

/// Ein ImportEntry Record (Tabelle 61): [[ModuleRequest]] als Stelle in
/// [[RequestedModules]].
#[derive(Clone, Debug)]
struct ImportEntry {
    request: usize,
    import_name: ImportName,
    local: Vec<u16>,
}

/// Ein ExportEntry Record aus [[IndirectExportEntries]].
#[derive(Clone, Debug)]
struct IndirectExport {
    export: Vec<u16>,
    request: usize,
    import_name: ImportName,
}

/// [[BindingName]] eines ResolvedBinding Record (Abschnitt 16.2.1.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BindingName {
    Name(Vec<u16>),
    /// ~namespace~
    Namespace,
}

/// Ein ResolvedBinding Record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedBinding {
    pub(crate) module: ModuleId,
    pub(crate) name: BindingName,
}

/// Was ResolveExport findet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// *null*
    None,
    /// ~ambiguous~
    Ambiguous,
    Found(ResolvedBinding),
}

/// [[AsyncEvaluationOrder]] eines Cyclic Module Record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AsyncOrder {
    /// ~unset~
    Unset,
    /// Eine Zahl aus IncrementModuleAsyncEvaluationCount.
    Order(u64),
    /// ~done~
    Done,
}

/// [[EvaluationError]]: ein geworfener Wert oder, wenn die Auswertung
/// anders abrupt endete (Quote, Speicher, nicht Übersetzbares), diese
/// Beendigung. Sie kehrt bei jedem weiteren Evaluate wieder.
enum EvaluationError {
    Throw(Persistent),
    Abrupt(Abrupt),
}

/// Was ein Record außer den gemeinsamen Feldern hat.
enum Body {
    /// Ein Source Text Module Record (Abschnitt 16.2.1.7): Syntaxbaum,
    /// übersetzter Code und für jede Funktionsdeklaration auf oberster Ebene
    /// die Nummer ihrer gemeinsamen Daten, parallel zu den Bindungen.
    SourceText {
        unit: Rc<Ast>,
        code: Rc<Code>,
        bindings: Vec<ModuleBinding>,
        functions: Vec<Option<i32>>,
        /// Die Plätze der Bindungen nach ihrem Namen.
        slots: BTreeMap<Vec<u16>, u32>,
        local_exports: Vec<(Vec<u16>, Vec<u16>)>,
        indirect_exports: Vec<IndirectExport>,
        star_exports: Vec<usize>,
        imports: Vec<ImportEntry>,
    },
    /// Ein Synthetic Module Record (Abschnitt 16.2.1.8) aus
    /// CreateDefaultExportSyntheticModule: [[ExportNames]] ist «
    /// *"default"* », die Bindung liegt in Platz 1. Der Wert kommt bei
    /// Evaluate in die Bindung.
    Synthetic { value: Persistent },
}

/// Ein Module Record (Abschnitt 16.2.1.5) mit den Feldern eines Cyclic
/// Module Record.
pub(crate) struct ModuleRecord {
    realm: RealmId,
    body: Body,
    /// [[Environment]], ein Kontext.
    environment: Persistent,
    /// [[Namespace]]
    namespace: Option<namespace::Namespace>,
    status: ModuleStatus,
    evaluation_error: Option<EvaluationError>,
    dfs_index: usize,
    dfs_ancestor_index: usize,
    /// [[RequestedModules]] ohne Wiederholungen, in der Reihenfolge des
    /// Quelltexts.
    requested: Vec<ModuleRequest>,
    /// [[LoadedModules]]
    loaded: Vec<(ModuleRequest, ModuleId)>,
    cycle_root: Option<ModuleId>,
    /// [[HasTLA]]
    has_tla: bool,
    /// [[AsyncEvaluationOrder]]
    async_evaluation_order: AsyncOrder,
    /// [[PendingAsyncDependencies]]
    pending_async_dependencies: usize,
    /// [[AsyncParentModules]]
    async_parent_modules: Vec<ModuleId>,
    /// Das Promise aus [[TopLevelCapability]].
    top_level_capability: Option<Persistent>,
    /// Das Objekt für `import.meta` ([[ImportMeta]]).
    import_meta: Option<Persistent>,
}

impl ModuleRecord {
    fn is_cyclic(&self) -> bool {
        matches!(self.body, Body::SourceText { .. })
    }

    /// Der Platz der Bindung `name` in der Umgebung.
    pub(crate) fn slot_of(&self, name: &[u16]) -> Option<u32> {
        match &self.body {
            Body::SourceText { slots, .. } => slots.get(name).copied(),
            Body::Synthetic { .. } => name
                .iter()
                .copied()
                .eq("default".encode_utf16())
                .then_some(1),
        }
    }

    pub(crate) fn environment(&self) -> &Persistent {
        &self.environment
    }
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Der Wert einer Bindung einer Modulumgebung: Eine Import-Bindung führt
/// über ihre Zelle zur Zielbindung. `None` für die Lücke, eine Bindung vor
/// ihrer Initialisierung.
pub(crate) fn binding_value<'h>(
    heap: &'h crate::heap::Heap,
    value: Value<'h>,
) -> Option<Value<'h>> {
    let value = match value.kind() {
        Kind::Internal(cell) => {
            let context = heap.slot(cell, 0)?;
            let slot = heap.slot(cell, 1)?.as_int32()?;
            let Kind::Internal(context) = context.kind() else {
                return None;
            };
            heap.slot(context, usize::try_from(slot).ok()?)?
        }
        _ => value,
    };
    (!value.is_hole()).then_some(value)
}

impl Agent {
    fn module(&mut self, module: ModuleId) -> Completion<&ModuleRecord> {
        if self.modules.get(module.0).is_none() {
            return Err(self.throw_error(ErrorKind::TypeError));
        }
        Ok(&self.modules[module.0])
    }

    pub(crate) fn module_record(&self, module: ModuleId) -> Option<&ModuleRecord> {
        self.modules.get(module.0)
    }

    fn module_record_mut(&mut self, module: ModuleId) -> Option<&mut ModuleRecord> {
        self.modules.get_mut(module.0)
    }

    /// Der Realm des Moduls.
    pub fn module_realm(&self, module: ModuleId) -> Option<RealmId> {
        self.modules.get(module.0).map(|record| record.realm)
    }

    /// [[Status]] des Moduls.
    pub fn module_status(&self, module: ModuleId) -> Option<ModuleStatus> {
        self.modules.get(module.0).map(|record| record.status)
    }

    /// Der Code eines Source Text Module Record und sein Realm, um den
    /// Rahmen nach `await` auf oberster Ebene fortzusetzen.
    pub(crate) fn module_code(&self, module: ModuleId) -> Option<(Rc<Code>, RealmId)> {
        let record = self.modules.get(module.0)?;
        match &record.body {
            Body::SourceText { code, .. } => Some((Rc::clone(code), record.realm)),
            Body::Synthetic { .. } => None,
        }
    }

    /// [[RequestedModules]] des Moduls.
    pub fn module_requests(&self, module: ModuleId) -> &[ModuleRequest] {
        self.modules
            .get(module.0)
            .map_or(&[], |record| record.requested.as_slice())
    }

    /// ParseModule (Abschnitt 16.2.1.7.1) im Realm `realm`: liest und
    /// übersetzt das Modul und legt seinen Record an, Status ~new~. Ein
    /// Modul, das zu viel Bytecode, Register oder Konstanten bräuchte, wirft
    /// einen RangeError.
    pub fn parse_module(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        source: &[u16],
    ) -> Result<ModuleId, ScriptError> {
        let ast = Rc::new(parse_module(source).map_err(ScriptError::Syntax)?);
        let compiled = match compile_module(&ast) {
            Ok(compiled) => compiled,
            Err(CompileError::Unsupported(position)) => {
                return Err(ScriptError::Unsupported(position));
            }
            Err(CompileError::TooLarge) => {
                return Err(ScriptError::Abrupt(self.throw_error(ErrorKind::RangeError)));
            }
        };
        let entries = entries::collect(&ast);
        let agent = self;
        let inner = scope;
        (|| -> Completion<ModuleId> {
            let length = usize::try_from(compiled.slots)
                .ok()
                .and_then(|slots| slots.checked_add(1))
                .ok_or(Abrupt::OutOfMemory)?;
            let environment = agent.heap_mut().alloc_slots(inner, length)?;
            let mut functions = Vec::with_capacity(compiled.bindings.len());
            let mut slots = BTreeMap::new();
            for binding in &compiled.bindings {
                set_slot(agent, environment, binding.slot, Value::HOLE);
                slots.insert(binding.name.clone(), binding.slot);
                let shared = match &binding.init {
                    ModuleInit::Function(template) => {
                        Some(agent.add_shared_function(Rc::clone(&ast), template.clone())?)
                    }
                    _ => None,
                };
                functions.push(shared);
            }
            let environment = agent.heap_mut().persist(environment);
            let has_tla = compiled.code.async_state.is_some();
            let record = ModuleRecord {
                realm,
                body: Body::SourceText {
                    unit: Rc::clone(&ast),
                    code: Rc::new(compiled.code),
                    bindings: compiled.bindings,
                    functions,
                    slots,
                    local_exports: entries.local_exports,
                    indirect_exports: entries.indirect_exports,
                    star_exports: entries.star_exports,
                    imports: entries.imports,
                },
                environment,
                namespace: None,
                status: ModuleStatus::New,
                evaluation_error: None,
                dfs_index: 0,
                dfs_ancestor_index: 0,
                requested: entries.requested,
                loaded: Vec::new(),
                cycle_root: None,
                has_tla,
                async_evaluation_order: AsyncOrder::Unset,
                pending_async_dependencies: 0,
                async_parent_modules: Vec::new(),
                top_level_capability: None,
                import_meta: None,
            };
            Ok(agent.add_module(record))
        })()
        .map_err(ScriptError::Abrupt)
    }

    /// CreateDefaultExportSyntheticModule (Abschnitt 16.2.1.8.1) mit
    /// ParseJSONModule (Abschnitt 16.2.1.8.2): JSON.parse ohne Reviver mit
    /// den Prototypen aus `realm`, dann ein Record, dessen einzige Bindung
    /// `default` den Wert bekommt. Ungültiges JSON wirft einen SyntaxError.
    pub fn parse_json_module(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        source: &[u16],
    ) -> Completion<ModuleId> {
        self.with_default_execution_budget(|agent| {
            let value = crate::builtins::json::parse_text(agent, scope, realm, source)?;
            Ok(agent.synthetic_module(scope, realm, value)?)
        })
    }

    /// CreateDefaultExportSyntheticModule (Abschnitt 16.2.1.8.1) mit
    /// `value` als Wert von `default`.
    pub fn create_default_export_synthetic_module(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        value: Local<'_>,
    ) -> Completion<ModuleId> {
        Ok(self.synthetic_module(scope, realm, value)?)
    }

    fn synthetic_module(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        value: Local<'_>,
    ) -> Result<ModuleId, crate::heap::OutOfMemory> {
        let environment = self.heap_mut().alloc_slots(scope, 2)?;
        set_slot(self, environment, 1, Value::HOLE);
        let environment = self.heap_mut().persist(environment);
        let value = self.heap_mut().persist(value);
        let record = ModuleRecord {
            realm,
            body: Body::Synthetic { value },
            environment,
            namespace: None,
            status: ModuleStatus::New,
            evaluation_error: None,
            dfs_index: 0,
            dfs_ancestor_index: 0,
            requested: Vec::new(),
            loaded: Vec::new(),
            cycle_root: None,
            has_tla: false,
            async_evaluation_order: AsyncOrder::Unset,
            pending_async_dependencies: 0,
            async_parent_modules: Vec::new(),
            top_level_capability: None,
            import_meta: None,
        };
        Ok(self.add_module(record))
    }

    fn add_module(&mut self, record: ModuleRecord) -> ModuleId {
        self.modules.push(record);
        ModuleId(self.modules.len().saturating_sub(1))
    }

    /// GetImportedModule (Abschnitt 16.2.1.9) für die Anfrage mit der Stelle
    /// `request` in [[RequestedModules]]; `None`, solange sie nicht geladen
    /// ist.
    fn imported_module(&self, referrer: ModuleId, request: usize) -> Option<ModuleId> {
        let record = self.modules.get(referrer.0)?;
        let request = record.requested.get(request)?;
        record
            .loaded
            .iter()
            .find(|(loaded, _)| loaded == request)
            .map(|&(_, module)| module)
    }

    // Exporte

    /// GetExportedNames (Abschnitte 16.2.1.7.2.1 und 16.2.1.8.4.2): die
    /// eigenen Namen und, über `export *` erreichbar, die der anderen Module
    /// außer *"default"*, ohne Wiederholungen. Jedes Modul zählt einmal;
    /// das ergibt dieselben Namen wie die Rekursion mit *exportStarSet*.
    pub(crate) fn exported_names(&self, module: ModuleId) -> Vec<Vec<u16>> {
        let mut names: Vec<Vec<u16>> = Vec::new();
        let mut known: BTreeSet<Vec<u16>> = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut work = vec![(module, true)];
        let default = units("default");
        while let Some((current, root)) = work.pop() {
            if !visited.insert(current) {
                continue;
            }
            let Some(record) = self.modules.get(current.0) else {
                continue;
            };
            let mut add = |name: &Vec<u16>| {
                if (root || *name != default) && known.insert(name.clone()) {
                    names.push(name.clone());
                }
            };
            match &record.body {
                Body::SourceText {
                    local_exports,
                    indirect_exports,
                    star_exports,
                    ..
                } => {
                    for (export, _) in local_exports {
                        add(export);
                    }
                    for entry in indirect_exports {
                        add(&entry.export);
                    }
                    for &request in star_exports.iter().rev() {
                        if let Some(imported) = self.imported_module(current, request) {
                            work.push((imported, false));
                        }
                    }
                }
                Body::Synthetic { .. } => add(&default),
            }
        }
        names
    }

    /// ResolveExport (Abschnitte 16.2.1.7.2.2 und 16.2.1.8.4.3) mit einer
    /// gemeinsamen *resolveSet*. Eine Weitergabe aus einem anderen Modul
    /// ersetzt die aktuelle Suche; für `export *` wartet die Suche auf dem
    /// Stapel auf die Ergebnisse der Module dahinter.
    pub(crate) fn resolve_export(&self, module: ModuleId, name: &[u16]) -> Resolution {
        struct Frame {
            module: ModuleId,
            name: Vec<u16>,
            next: usize,
            found: Option<ResolvedBinding>,
        }
        let mut resolve_set: BTreeSet<(ModuleId, Vec<u16>)> = BTreeSet::new();
        let mut frames: Vec<Frame> = Vec::new();
        let mut pending = Some((module, name.to_vec()));
        let mut result = None;
        loop {
            if let Some((current, name)) = pending.take() {
                match self.resolve_own(current, &name, &mut resolve_set) {
                    Own::Done(resolution) => result = Some(resolution),
                    Own::Forward(next, next_name) => {
                        pending = Some((next, next_name));
                        continue;
                    }
                    Own::Stars => frames.push(Frame {
                        module: current,
                        name,
                        next: 0,
                        found: None,
                    }),
                }
            }
            let Some(frame) = frames.last_mut() else {
                return result.unwrap_or(Resolution::None);
            };
            match result.take() {
                Some(Resolution::Ambiguous) => {
                    frames.pop();
                    result = Some(Resolution::Ambiguous);
                    continue;
                }
                Some(Resolution::Found(binding)) => match &frame.found {
                    None => frame.found = Some(binding),
                    Some(found) if *found != binding => {
                        frames.pop();
                        result = Some(Resolution::Ambiguous);
                        continue;
                    }
                    Some(_) => {}
                },
                Some(Resolution::None) | None => {}
            }
            let star = match &self.modules.get(frame.module.0).map(|record| &record.body) {
                Some(Body::SourceText { star_exports, .. }) => {
                    star_exports.get(frame.next).copied()
                }
                _ => None,
            };
            match star {
                Some(request) => {
                    frame.next = frame.next.saturating_add(1);
                    match self.imported_module(frame.module, request) {
                        Some(imported) => pending = Some((imported, frame.name.clone())),
                        None => result = Some(Resolution::None),
                    }
                }
                None => {
                    let found = frames.pop().and_then(|frame| frame.found);
                    result = Some(found.map_or(Resolution::None, Resolution::Found));
                }
            }
        }
    }

    /// Die Schritte von ResolveExport bis vor die Schleife über
    /// [[StarExportEntries]].
    fn resolve_own(
        &self,
        module: ModuleId,
        name: &[u16],
        resolve_set: &mut BTreeSet<(ModuleId, Vec<u16>)>,
    ) -> Own {
        let Some(record) = self.modules.get(module.0) else {
            return Own::Done(Resolution::None);
        };
        let (local_exports, indirect_exports) = match &record.body {
            Body::SourceText {
                local_exports,
                indirect_exports,
                ..
            } => (local_exports, indirect_exports),
            Body::Synthetic { .. } => {
                let found = record.slot_of(name).is_some();
                return Own::Done(if found {
                    Resolution::Found(ResolvedBinding {
                        module,
                        name: BindingName::Name(name.to_vec()),
                    })
                } else {
                    Resolution::None
                });
            }
        };
        // Eine zirkuläre Anfrage.
        if !resolve_set.insert((module, name.to_vec())) {
            return Own::Done(Resolution::None);
        }
        if let Some((_, local)) = local_exports.iter().find(|(export, _)| export == name) {
            return Own::Done(Resolution::Found(ResolvedBinding {
                module,
                name: BindingName::Name(local.clone()),
            }));
        }
        if let Some(entry) = indirect_exports.iter().find(|entry| entry.export == name) {
            let Some(imported) = self.imported_module(module, entry.request) else {
                return Own::Done(Resolution::None);
            };
            return match &entry.import_name {
                ImportName::Namespace => Own::Done(Resolution::Found(ResolvedBinding {
                    module: imported,
                    name: BindingName::Namespace,
                })),
                ImportName::Name(import) => Own::Forward(imported, import.clone()),
            };
        }
        // `export *` gibt *"default"* nie weiter.
        if name.iter().copied().eq("default".encode_utf16()) {
            return Own::Done(Resolution::None);
        }
        Own::Stars
    }

    // Link

    /// Link (Abschnitt 16.2.1.6.1.2) des Moduls, nachdem
    /// [`Agent::load_requested_modules`] alle Module des Graphen geladen
    /// hat. Bei einem Fehler wirft es, und die Module, die gerade gelinkt
    /// wurden, sind wieder ~unlinked~.
    pub fn link_module(&mut self, module: ModuleId) -> Completion<()> {
        let record = self.module(module)?;
        if !record.is_cyclic() {
            self.link_synthetic(module);
            return Ok(());
        }
        if matches!(
            record.status,
            ModuleStatus::New | ModuleStatus::Linking | ModuleStatus::Evaluating
        ) {
            return Err(self.throw_error(ErrorKind::TypeError));
        }
        let mut stack = Vec::new();
        let result = self.inner_module_linking(module, &mut stack);
        if result.is_err() {
            for required in stack {
                if let Some(record) = self.module_record_mut(required) {
                    record.status = ModuleStatus::Unlinked;
                }
            }
        }
        result
    }

    /// Link eines Synthetic Module Record (Abschnitt 16.2.1.8.4.4): Die
    /// Umgebung gibt es schon; beim ersten Mal bekommt `default`
    /// *undefined*. Weitere Aufrufe ändern nichts, denn Module, die schon
    /// gelinkt sind, verweisen auf diese Umgebung.
    fn link_synthetic(&mut self, module: ModuleId) {
        let Some(record) = self.modules.get_mut(module.0) else {
            return;
        };
        if record.status != ModuleStatus::New {
            return;
        }
        record.status = ModuleStatus::Linked;
        let heap = self.heap();
        let environment = heap.get_persistent(self.module_environment(module));
        if let Kind::Internal(cell) = environment.kind() {
            heap.set_slot(cell, 1, Value::UNDEFINED);
        }
    }

    /// InnerModuleLinking (Abschnitt 16.2.1.6.1.2.1) als Schleife mit einem
    /// Stapel offener Module.
    fn inner_module_linking(
        &mut self,
        root: ModuleId,
        stack: &mut Vec<ModuleId>,
    ) -> Completion<()> {
        struct Frame {
            module: ModuleId,
            next: usize,
        }
        let mut index: usize = 0;
        let mut frames: Vec<Frame> = Vec::new();
        let mut pending = Some(root);
        loop {
            if let Some(module) = pending.take() {
                let record = self.module(module)?;
                let done = if !record.is_cyclic() {
                    self.link_synthetic(module);
                    true
                } else {
                    match record.status {
                        ModuleStatus::Linking
                        | ModuleStatus::Linked
                        | ModuleStatus::EvaluatingAsync
                        | ModuleStatus::Evaluated => true,
                        ModuleStatus::Unlinked => false,
                        ModuleStatus::New | ModuleStatus::Evaluating => {
                            return Err(self.throw_error(ErrorKind::TypeError));
                        }
                    }
                };
                if done {
                    if let Some(parent) = frames.last() {
                        self.after_linking(parent.module, module);
                    }
                } else {
                    if let Some(record) = self.module_record_mut(module) {
                        record.status = ModuleStatus::Linking;
                        record.dfs_index = index;
                        record.dfs_ancestor_index = index;
                    }
                    index = index.saturating_add(1);
                    stack.push(module);
                    frames.push(Frame { module, next: 0 });
                }
            }
            let Some(frame) = frames.last_mut() else {
                return Ok(());
            };
            let module = frame.module;
            let count = self.module_requests(module).len();
            if frame.next < count {
                let request = frame.next;
                frame.next = request.saturating_add(1);
                match self.imported_module(module, request) {
                    Some(required) => pending = Some(required),
                    None => return Err(self.throw_error(ErrorKind::TypeError)),
                }
                continue;
            }
            frames.pop();
            self.initialize_environment(module)?;
            let (dfs_index, ancestor) = self.module_record(module).map_or((0, 0), |record| {
                (record.dfs_index, record.dfs_ancestor_index)
            });
            if ancestor == dfs_index {
                while let Some(required) = stack.pop() {
                    if let Some(record) = self.module_record_mut(required) {
                        record.status = ModuleStatus::Linked;
                    }
                    if required == module {
                        break;
                    }
                }
            }
            if let Some(parent) = frames.last() {
                self.after_linking(parent.module, module);
            }
        }
    }

    /// Schritt 9.c von InnerModuleLinking und 16.d von
    /// InnerModuleEvaluation: Ist das angeforderte Modul noch auf dem
    /// Stapel, sinkt [[DFSAncestorIndex]].
    fn after_linking(&mut self, module: ModuleId, required: ModuleId) {
        self.lower_ancestor(module, required, ModuleStatus::Linking);
    }

    fn lower_ancestor(&mut self, module: ModuleId, required: ModuleId, open: ModuleStatus) {
        let Some(required) = self.module_record(required) else {
            return;
        };
        if !required.is_cyclic() || required.status != open {
            return;
        }
        let ancestor = required.dfs_ancestor_index;
        if let Some(record) = self.module_record_mut(module) {
            record.dfs_ancestor_index = record.dfs_ancestor_index.min(ancestor);
        }
    }

    /// InitializeEnvironment (Abschnitt 16.2.1.7.3.1): Jede Weitergabe muss
    /// sich auflösen lassen, Import-Bindungen verweisen auf ihr Ziel oder
    /// halten einen Namensraum, `var` beginnt mit *undefined*, lexikalische
    /// Bindungen mit der Lücke, Funktionen mit einem neuen Funktionsobjekt.
    fn initialize_environment(&mut self, module: ModuleId) -> Completion<()> {
        let Some(record) = self.module_record(module) else {
            return Err(self.throw_error(ErrorKind::TypeError));
        };
        let realm = record.realm;
        let Body::SourceText {
            indirect_exports,
            imports,
            bindings,
            functions,
            slots,
            ..
        } = &record.body
        else {
            return Ok(());
        };
        for entry in indirect_exports {
            if !matches!(
                self.resolve_export(module, &entry.export),
                Resolution::Found(_)
            ) {
                return Err(self.throw_error(ErrorKind::SyntaxError));
            }
        }
        let imports = imports.clone();
        let bindings: Vec<(u32, ModuleInit, Option<i32>, bool)> = bindings
            .iter()
            .zip(functions)
            .map(|(binding, &shared)| {
                let default = binding
                    .name
                    .iter()
                    .copied()
                    .eq(DEFAULT_BINDING.encode_utf16());
                (binding.slot, binding.init.clone(), shared, default)
            })
            .collect();
        let local_slots: Vec<Option<u32>> = imports
            .iter()
            .map(|entry| slots.get(&entry.local).copied())
            .collect();
        self.scope(|agent, inner| -> Completion<()> {
            let heap = agent.heap();
            let environment =
                heap.local(inner, heap.get_persistent(agent.module_environment(module)));
            for (entry, slot) in imports.iter().zip(local_slots) {
                let Some(slot) = slot else {
                    return Err(agent.throw_error(ErrorKind::TypeError));
                };
                let Some(imported) = agent.imported_module(module, entry.request) else {
                    return Err(agent.throw_error(ErrorKind::TypeError));
                };
                let value = match &entry.import_name {
                    ImportName::Namespace => agent.module_namespace(inner, imported)?,
                    ImportName::Name(name) => match agent.resolve_export(imported, name) {
                        Resolution::Found(ResolvedBinding {
                            module: target,
                            name: BindingName::Namespace,
                        }) => agent.module_namespace(inner, target)?,
                        Resolution::Found(ResolvedBinding {
                            module: target,
                            name: BindingName::Name(name),
                        }) => agent.import_cell(inner, target, &name)?,
                        Resolution::None | Resolution::Ambiguous => {
                            return Err(agent.throw_error(ErrorKind::SyntaxError));
                        }
                    },
                };
                set_slot(agent, environment, slot, agent.heap().get(value));
            }
            for (slot, init, shared, default) in &bindings {
                match init {
                    ModuleInit::Import => {}
                    ModuleInit::Var => set_slot(agent, environment, *slot, Value::UNDEFINED),
                    ModuleInit::Lexical => set_slot(agent, environment, *slot, Value::HOLE),
                    ModuleInit::Function(_) => {
                        let Some(shared) = *shared else {
                            return Err(agent.throw_error(ErrorKind::TypeError));
                        };
                        agent.scope(|agent, scope| -> Completion<()> {
                            let name = if *default {
                                units("default")
                            } else {
                                agent.module_binding_name(module, *slot)
                            };
                            let key = agent.key_from_units(scope, &name)?;
                            let function =
                                instantiate_closure(agent, scope, realm, shared, key, environment)?;
                            set_slot(agent, environment, *slot, agent.heap().get(function));
                            Ok(())
                        })?;
                    }
                }
            }
            Ok(())
        })
    }

    fn module_environment(&self, module: ModuleId) -> &Persistent {
        &self.modules[module.0].environment
    }

    fn module_binding_name(&self, module: ModuleId, slot: u32) -> Vec<u16> {
        match self.modules.get(module.0).map(|record| &record.body) {
            Some(Body::SourceText { bindings, .. }) => bindings
                .iter()
                .find(|binding| binding.slot == slot)
                .map(|binding| binding.name.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Die Zelle einer Import-Bindung: der Kontext des Zielmoduls und der
    /// Platz der Bindung `name` darin (CreateImportBinding, Abschnitt
    /// 9.1.1.5.5).
    fn import_cell<'s>(
        &mut self,
        scope: &'s Scope,
        target: ModuleId,
        name: &[u16],
    ) -> Completion<Local<'s>> {
        let Some(slot) = self
            .module_record(target)
            .and_then(|record| record.slot_of(name))
        else {
            return Err(self.throw_error(ErrorKind::SyntaxError));
        };
        let slot = i32::try_from(slot).map_err(|_| Abrupt::OutOfMemory)?;
        let cell = self.heap_mut().alloc_slots(scope, 2)?;
        let heap = self.heap();
        let environment = heap.get_persistent(self.module_environment(target));
        if let Kind::Internal(reference) = heap.get(cell).kind() {
            heap.set_slot(reference, 0, environment);
            heap.set_slot(reference, 1, Value::int32(slot));
        }
        Ok(cell)
    }

    // Evaluate

    /// Evaluate (Abschnitt 16.2.1.6.1.3) des gelinkten Moduls: führt es und
    /// die Module, die es braucht, aus und gibt das Promise aus
    /// [[TopLevelCapability]] zurück. Ohne Top-Level-`await` im Graphen ist
    /// es danach erfüllt oder mit dem Fehler abgelehnt; sonst erst, wenn
    /// die Jobs die wartenden Module zu Ende geführt haben. Endet die
    /// Ausführung anders abrupt als mit einer Ausnahme, kehrt das als Fehler
    /// zurück, und die betroffenen Module bleiben so beendet. Ohne
    /// bestehende Hostquote gilt [`crate::agent::DEFAULT_EXECUTION_STEPS`].
    pub fn evaluate_module<'s>(
        &mut self,
        scope: &'s Scope,
        module: ModuleId,
    ) -> Completion<Local<'s>> {
        self.with_default_execution_budget(|agent| agent.evaluate_module_inner(scope, module))
    }

    fn evaluate_module_inner<'s>(
        &mut self,
        scope: &'s Scope,
        module: ModuleId,
    ) -> Completion<Local<'s>> {
        let record = self.module(module)?;
        let realm = record.realm;
        if !record.is_cyclic() {
            if record.status == ModuleStatus::New {
                return Err(self.throw_error(ErrorKind::TypeError));
            }
            self.evaluate_synthetic(module);
            let promise = self.new_module_promise(scope, realm)?;
            let undefined = self.heap().local(scope, Value::UNDEFINED);
            settle_pending(self, scope, realm, promise, undefined, true)?;
            return Ok(promise);
        }
        let mut module = module;
        match record.status {
            ModuleStatus::Linked => {}
            ModuleStatus::EvaluatingAsync | ModuleStatus::Evaluated => {
                if let Some(root) = record.cycle_root {
                    module = root;
                }
            }
            _ => return Err(self.throw_error(ErrorKind::TypeError)),
        }
        // Eine frühere Ausführung endete ohne Ausnahme abrupt, etwa am Ende
        // der Quote: Sie meldet das wieder, statt ein Promise zu geben.
        if let Some(EvaluationError::Abrupt(abrupt)) = self
            .module_record(module)
            .and_then(|record| record.evaluation_error.as_ref())
        {
            return Err(*abrupt);
        }
        if let Some(capability) = self
            .module_record(module)
            .and_then(|record| record.top_level_capability.as_ref())
        {
            let heap = self.heap();
            return Ok(heap.local(scope, heap.get_persistent(capability)));
        }
        let realm = self.module_realm(module).unwrap_or(realm);
        let promise = self.new_module_promise(scope, realm)?;
        let handle = self.heap_mut().persist(promise);
        if let Some(record) = self.module_record_mut(module) {
            record.top_level_capability = Some(handle);
        }
        let mut stack = Vec::new();
        match self.inner_module_evaluation(module, &mut stack) {
            // Ein Modul, das noch ~evaluating-async~ ist, erfüllt oder lehnt
            // das Promise mit AsyncModuleExecutionFulfilled oder
            // AsyncModuleExecutionRejected.
            Ok(()) => {
                if self.module_status(module) == Some(ModuleStatus::Evaluated) {
                    let undefined = self.heap().local(scope, Value::UNDEFINED);
                    settle_pending(self, scope, realm, promise, undefined, true)?;
                }
            }
            Err(Abrupt::Throw) => {
                let error = self.take_thrown_value(scope, realm)?;
                for required in stack {
                    let handle = self.heap_mut().persist(error);
                    if let Some(record) = self.module_record_mut(required) {
                        record.status = ModuleStatus::Evaluated;
                        record.evaluation_error = Some(EvaluationError::Throw(handle));
                    }
                }
                settle_pending(self, scope, realm, promise, error, false)?;
            }
            Err(abrupt) => {
                for required in stack {
                    if let Some(record) = self.module_record_mut(required) {
                        record.status = ModuleStatus::Evaluated;
                        record.evaluation_error = Some(EvaluationError::Abrupt(abrupt));
                    }
                }
                // Das Promise bliebe für immer offen.
                let capability = self
                    .module_record_mut(module)
                    .and_then(|record| record.top_level_capability.take());
                if let Some(capability) = capability {
                    self.heap_mut().release(capability);
                }
                return Err(abrupt);
            }
        }
        Ok(promise)
    }

    fn new_module_promise<'s>(
        &mut self,
        scope: &'s Scope,
        realm: RealmId,
    ) -> Completion<Local<'s>> {
        let prototype = self.intrinsic(scope, realm, Intrinsic::PromisePrototype);
        Ok(promise_create(self, scope, prototype)?)
    }

    /// Evaluate eines Synthetic Module Record (Abschnitt 16.2.1.8.4.5) mit
    /// den Schritten aus CreateDefaultExportSyntheticModule: Der Wert kommt
    /// in die Bindung `default`.
    fn evaluate_synthetic(&mut self, module: ModuleId) {
        let Some(record) = self.modules.get_mut(module.0) else {
            return;
        };
        if !matches!(record.body, Body::Synthetic { .. }) {
            return;
        }
        record.status = ModuleStatus::Evaluated;
        let Some(record) = self.modules.get(module.0) else {
            return;
        };
        let Body::Synthetic { value } = &record.body else {
            return;
        };
        let heap = self.heap();
        if let Kind::Internal(cell) = heap.get_persistent(&record.environment).kind() {
            heap.set_slot(cell, 1, heap.get_persistent(value));
        }
    }

    /// Wirft [[EvaluationError]] des Moduls wieder.
    fn rethrow_evaluation_error(&mut self, module: ModuleId) -> Abrupt {
        let error = match self
            .module_record(module)
            .and_then(|record| record.evaluation_error.as_ref())
        {
            Some(EvaluationError::Throw(handle)) => Ok(self.heap().get_persistent(handle).bits()),
            Some(EvaluationError::Abrupt(abrupt)) => Err(*abrupt),
            None => Err(Abrupt::Throw),
        };
        match error {
            Ok(bits) => self.scope(|agent, scope| {
                let value = agent.heap().local(scope, Value::from_bits(bits));
                agent.throw_value(value)
            }),
            Err(Abrupt::Throw) => self.throw_error(ErrorKind::TypeError),
            Err(abrupt) => abrupt,
        }
    }

    /// InnerModuleEvaluation (Abschnitt 16.2.1.6.1.3.1) als Schleife mit
    /// einem Stapel offener Module. Ein Modul läuft mit ExecuteModule,
    /// sobald die Module, die es braucht, gelaufen sind. Hat es
    /// Top-Level-`await` oder wartet es auf ein Modul, das noch läuft,
    /// bekommt es [[AsyncEvaluationOrder]]; ohne offene Abhängigkeiten
    /// startet es dann mit ExecuteAsyncModule.
    fn inner_module_evaluation(
        &mut self,
        root: ModuleId,
        stack: &mut Vec<ModuleId>,
    ) -> Completion<()> {
        struct Frame {
            module: ModuleId,
            next: usize,
        }
        let mut index: usize = 0;
        let mut frames: Vec<Frame> = Vec::new();
        let mut pending = Some(root);
        loop {
            if let Some(module) = pending.take() {
                let record = self.module(module)?;
                let done = if !record.is_cyclic() {
                    if record.status == ModuleStatus::New {
                        return Err(self.throw_error(ErrorKind::TypeError));
                    }
                    self.evaluate_synthetic(module);
                    true
                } else {
                    match record.status {
                        ModuleStatus::EvaluatingAsync | ModuleStatus::Evaluated => {
                            if record.evaluation_error.is_some() {
                                return Err(self.rethrow_evaluation_error(module));
                            }
                            true
                        }
                        ModuleStatus::Evaluating => true,
                        ModuleStatus::Linked => false,
                        ModuleStatus::New | ModuleStatus::Unlinked | ModuleStatus::Linking => {
                            return Err(self.throw_error(ErrorKind::TypeError));
                        }
                    }
                };
                if done {
                    if let Some(parent) = frames.last() {
                        let parent = parent.module;
                        self.after_evaluation(parent, module)?;
                    }
                } else {
                    if let Some(record) = self.module_record_mut(module) {
                        record.status = ModuleStatus::Evaluating;
                        record.dfs_index = index;
                        record.dfs_ancestor_index = index;
                        record.pending_async_dependencies = 0;
                    }
                    index = index.saturating_add(1);
                    stack.push(module);
                    frames.push(Frame { module, next: 0 });
                }
            }
            let Some(frame) = frames.last_mut() else {
                return Ok(());
            };
            let module = frame.module;
            let count = self.module_requests(module).len();
            if frame.next < count {
                let request = frame.next;
                frame.next = request.saturating_add(1);
                match self.imported_module(module, request) {
                    Some(required) => pending = Some(required),
                    None => return Err(self.throw_error(ErrorKind::TypeError)),
                }
                continue;
            }
            frames.pop();
            let (has_tla, pending) = self.module_record(module).map_or((false, 0), |record| {
                (record.has_tla, record.pending_async_dependencies)
            });
            if has_tla || pending > 0 {
                // IncrementModuleAsyncEvaluationCount (Abschnitt 9.6.3).
                let order = self.module_async_evaluation_count;
                self.module_async_evaluation_count = order.saturating_add(1);
                if let Some(record) = self.module_record_mut(module) {
                    record.async_evaluation_order = AsyncOrder::Order(order);
                }
                if pending == 0 {
                    self.execute_async_module(module)?;
                }
            } else {
                self.execute_module(module, None)?;
            }
            let (dfs_index, ancestor) = self.module_record(module).map_or((0, 0), |record| {
                (record.dfs_index, record.dfs_ancestor_index)
            });
            if ancestor == dfs_index {
                while let Some(required) = stack.pop() {
                    if let Some(record) = self.module_record_mut(required) {
                        record.status = if record.async_evaluation_order == AsyncOrder::Unset {
                            ModuleStatus::Evaluated
                        } else {
                            ModuleStatus::EvaluatingAsync
                        };
                        record.cycle_root = Some(module);
                    }
                    if required == module {
                        break;
                    }
                }
            }
            if let Some(parent) = frames.last() {
                let parent = parent.module;
                self.after_evaluation(parent, module)?;
            }
        }
    }

    /// Schritt 16.d von InnerModuleEvaluation für ein angefordertes Modul,
    /// das fertig ist oder noch auf dem Stapel liegt. Läuft es oder die
    /// Wurzel seines Zyklus noch asynchron, wartet `module` darauf.
    fn after_evaluation(&mut self, module: ModuleId, required: ModuleId) -> Completion<()> {
        let Some(record) = self.module_record(required) else {
            return Ok(());
        };
        if !record.is_cyclic() {
            return Ok(());
        }
        let required = if record.status == ModuleStatus::Evaluating {
            self.lower_ancestor(module, required, ModuleStatus::Evaluating);
            required
        } else {
            let root = record.cycle_root.unwrap_or(required);
            if self
                .module_record(root)
                .is_some_and(|record| record.evaluation_error.is_some())
            {
                return Err(self.rethrow_evaluation_error(root));
            }
            root
        };
        let waiting = self
            .module_record(required)
            .is_some_and(|record| matches!(record.async_evaluation_order, AsyncOrder::Order(_)));
        if waiting {
            if let Some(record) = self.module_record_mut(module) {
                record.pending_async_dependencies =
                    record.pending_async_dependencies.saturating_add(1);
            }
            if let Some(record) = self.module_record_mut(required) {
                record.async_parent_modules.push(module);
            }
        }
        Ok(())
    }

    /// ExecuteModule (Abschnitt 16.2.1.7.3.2): ein Rahmen mit der Umgebung
    /// des Moduls als Kontext, `this` *undefined*. Mit Top-Level-`await`
    /// bekommt er den Zustand `state` aus ExecuteAsyncModule und endet beim
    /// ersten `await`.
    fn execute_module(&mut self, module: ModuleId, state: Option<Local<'_>>) -> Completion<()> {
        let Some(record) = self.module_record(module) else {
            return Err(self.throw_error(ErrorKind::TypeError));
        };
        let realm = record.realm;
        let Body::SourceText { unit, code, .. } = &record.body else {
            return Ok(());
        };
        let unit = Rc::clone(unit);
        let code = Rc::clone(code);
        self.scope(|agent, inner| {
            let heap = agent.heap();
            let environment =
                heap.local(inner, heap.get_persistent(agent.module_environment(module)));
            execute_module(agent, inner, realm, &code, &unit, environment, state).map(|_| ())
        })
    }

    /// Das Modul, zu dessen Syntaxbaum `unit` gehört.
    pub(crate) fn module_of_unit(&self, unit: &Rc<Ast>) -> Option<ModuleId> {
        self.modules
            .iter()
            .position(|record| match &record.body {
                Body::SourceText { unit: own, .. } => Rc::ptr_eq(own, unit),
                Body::Synthetic { .. } => false,
            })
            .map(ModuleId)
    }

    /// `import.meta` (Abschnitt 13.3.12.1): beim ersten Mal ein neues
    /// Objekt ohne Prototyp mit den Eigenschaften aus
    /// HostGetImportMetaProperties, danach HostFinalizeImportMeta.
    pub(crate) fn import_meta<'s>(
        &mut self,
        scope: &'s Scope,
        module: ModuleId,
    ) -> Completion<Local<'s>> {
        self.module(module)?;
        let known = self
            .module_record(module)
            .and_then(|record| record.import_meta.as_ref())
            .map(|meta| self.heap().get_persistent(meta).bits());
        if let Some(bits) = known {
            return Ok(self.heap().local(scope, Value::from_bits(bits)));
        }
        let meta = ordinary_object_create(self, scope, None)?;
        if let Some(host) = self.host() {
            self.scope(|agent, inner| -> Completion<()> {
                let properties = host.import_meta_properties(agent, inner, module)?;
                for (key, value) in properties {
                    create_data_property_or_throw(agent, inner, meta, key, value)?;
                }
                Ok(())
            })?;
            host.finalize_import_meta(self, scope, meta, module)?;
        }
        let handle = self.heap_mut().persist(meta);
        if let Some(record) = self.module_record_mut(module) {
            record.import_meta = Some(handle);
        }
        Ok(meta)
    }
}

/// Was ResolveExport vor `export *` entscheidet.
enum Own {
    Done(Resolution),
    /// Die Suche geht in einem anderen Modul mit einem anderen Namen weiter.
    Forward(ModuleId, Vec<u16>),
    /// Die Antwort hängt an [[StarExportEntries]].
    Stars,
}

fn set_slot(agent: &Agent, context: Local<'_>, slot: u32, value: Value<'_>) {
    let heap = agent.heap();
    if let (Kind::Internal(cell), Ok(slot)) = (heap.get(context).kind(), usize::try_from(slot)) {
        heap.set_slot(cell, slot, value);
    }
}

#[cfg(test)]
mod tests;
