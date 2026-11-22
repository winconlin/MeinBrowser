//! Module mit `await` auf oberster Ebene: ExecuteAsyncModule,
//! GatherAvailableAncestors, AsyncModuleExecutionFulfilled und
//! AsyncModuleExecutionRejected (Abschnitte 16.2.1.6.1.3.2 bis
//! 16.2.1.6.1.3.5).
//!
//! Ein solches Modul läuft wie der Rumpf einer async-Funktion
//! (`crate::async_function`). Sein Zustand nennt statt eines
//! Funktionsobjekts die Nummer des Moduls; ExecuteAsyncModule legt ihn an
//! und hängt an sein Promise zwei eingebaute Funktionen, die mit der Nummer
//! des Moduls AsyncModuleExecutionFulfilled oder
//! AsyncModuleExecutionRejected ausführen. GatherAvailableAncestors und das
//! Weitergeben einer Ablehnung an [[AsyncParentModules]] sind Schleifen mit
//! eigenem Stapel statt Rekursion (RS-8), in derselben Reihenfolge.
//!
//! Endet ein solcher Lauf anders abrupt als mit einer Ausnahme, etwa am
//! Ende der Quote des Hosts, gilt das wie in Evaluate als
//! [[EvaluationError]]: Das Modul und alle, die über [[AsyncParentModules]]
//! auf es warten, sind danach mit diesem Abbruch ausgewertet, ihre
//! Promises aus [[TopLevelCapability]] bleiben offen, und ein späteres
//! Evaluate meldet den Abbruch wieder.

use std::collections::BTreeSet;

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::async_function::{async_start, perform_then_with, promise, settle};
use crate::builtin::{Builtin, BuiltinCall};
use crate::heap::{Local, Scope};
use crate::promise::settle_pending;
use crate::value::Value;

use super::{AsyncOrder, EvaluationError, ModuleId, ModuleStatus};

/// *onFulfilled* aus ExecuteAsyncModule.
static FULFILLED: Builtin = Builtin {
    name: "",
    length: 0,
    constructor: false,
    behaviour: fulfilled,
};

/// *onRejected* aus ExecuteAsyncModule.
static REJECTED: Builtin = Builtin {
    name: "",
    length: 0,
    constructor: false,
    behaviour: rejected,
};

impl Agent {
    /// ExecuteAsyncModule (Abschnitt 16.2.1.6.1.3.2): der Zustand mit
    /// NewPromiseCapability(%Promise%) im Realm des Moduls, die beiden
    /// Funktionen an seinem Promise, dann ExecuteModule bis zum ersten
    /// `await` oder zum Ende. Kann der Rahmen nicht starten, weil der
    /// Registerstapel nicht reicht, lehnt der RangeError das Promise ab wie
    /// eine Ausnahme im Rumpf.
    pub(super) fn execute_async_module(&mut self, module: ModuleId) -> Completion<()> {
        let Some(realm) = self.module_realm(module) else {
            return Err(self.throw_error(ErrorKind::TypeError));
        };
        let index = i32::try_from(module.index()).map_err(|_| Abrupt::OutOfMemory)?;
        self.scope(|agent, scope| {
            let marker = agent.heap().local(scope, Value::int32(index));
            let state = async_start(agent, scope, realm, marker)?;
            let heap = agent.heap();
            let capability = heap.local(scope, promise(heap, heap.get(state)));
            perform_then_with(
                agent, scope, realm, capability, marker, &FULFILLED, &REJECTED,
            )?;
            match agent.execute_module(module, Some(state)) {
                Err(Abrupt::Throw) => {
                    let error = agent.take_thrown_value(scope, realm)?;
                    settle(agent, scope, state, error, true)?;
                    Ok(())
                }
                result => result,
            }
        })
    }

    /// GatherAvailableAncestors (Abschnitt 16.2.1.6.1.3.3) für `module`:
    /// die Module aus [[AsyncParentModules]], die danach auf nichts mehr
    /// warten, und ohne eigenes Top-Level-`await` deren Vorfahren ebenso.
    fn gather_available_ancestors(&mut self, module: ModuleId) -> Vec<ModuleId> {
        let mut list = Vec::new();
        let mut listed = BTreeSet::new();
        let mut frames = vec![(module, 0usize)];
        while let Some(frame) = frames.last_mut() {
            let (current, next) = *frame;
            let parent = self
                .module_record(current)
                .and_then(|record| record.async_parent_modules.get(next).copied());
            let Some(ancestor) = parent else {
                frames.pop();
                continue;
            };
            frame.1 = next.saturating_add(1);
            if listed.contains(&ancestor) {
                continue;
            }
            let Some(record) = self.module_record(ancestor) else {
                continue;
            };
            // Ein Modul, das auf dem Stapel von Evaluate einen Fehler bekam,
            // hat keine Wurzel; dann gilt sein eigener.
            let root = record.cycle_root.unwrap_or(ancestor);
            if self
                .module_record(root)
                .is_some_and(|root| root.evaluation_error.is_some())
            {
                continue;
            }
            let Some(record) = self.module_record_mut(ancestor) else {
                continue;
            };
            record.pending_async_dependencies = record.pending_async_dependencies.saturating_sub(1);
            if record.pending_async_dependencies == 0 {
                list.push(ancestor);
                listed.insert(ancestor);
                if !record.has_tla {
                    frames.push((ancestor, 0));
                }
            }
        }
        list
    }

    /// AsyncModuleExecutionFulfilled (Abschnitt 16.2.1.6.1.3.4): Das Modul
    /// ist ausgewertet, und die Vorfahren, die nur noch auf es warteten,
    /// laufen nach [[AsyncEvaluationOrder]], mit Top-Level-`await` über
    /// ExecuteAsyncModule, sonst sofort. Bricht einer davon ohne Ausnahme
    /// ab, gilt der Abbruch für ihn und alle, die noch nicht liefen.
    fn async_module_execution_fulfilled(
        &mut self,
        scope: &Scope,
        module: ModuleId,
    ) -> Completion<()> {
        let Some(record) = self.module_record_mut(module) else {
            return Ok(());
        };
        if record.status == ModuleStatus::Evaluated {
            return Ok(());
        }
        record.async_evaluation_order = AsyncOrder::Done;
        record.status = ModuleStatus::Evaluated;
        self.settle_top_level(scope, module, None)?;
        let mut list = self.gather_available_ancestors(module);
        list.sort_by_key(|&ancestor| match self.module_record(ancestor) {
            Some(record) => match record.async_evaluation_order {
                AsyncOrder::Order(order) => order,
                AsyncOrder::Unset | AsyncOrder::Done => u64::MAX,
            },
            None => u64::MAX,
        });
        for (position, &ancestor) in list.iter().enumerate() {
            let Some(record) = self.module_record(ancestor) else {
                continue;
            };
            if record.status == ModuleStatus::Evaluated {
                continue;
            }
            let realm = record.realm;
            let result = if record.has_tla {
                self.execute_async_module(ancestor)
            } else {
                match self.execute_module(ancestor, None) {
                    Ok(()) => {
                        if let Some(record) = self.module_record_mut(ancestor) {
                            record.async_evaluation_order = AsyncOrder::Done;
                            record.status = ModuleStatus::Evaluated;
                        }
                        self.settle_top_level(scope, ancestor, None)
                    }
                    Err(Abrupt::Throw) => {
                        let error = self.take_thrown_value(scope, realm)?;
                        self.async_module_execution_rejected(scope, ancestor, error)
                    }
                    Err(abrupt) => Err(abrupt),
                }
            };
            if let Err(abrupt) = result {
                for &rest in list.get(position..).unwrap_or_default() {
                    self.abort_async_module(rest, abrupt);
                }
                return Err(abrupt);
            }
        }
        Ok(())
    }

    /// AsyncModuleExecutionRejected (Abschnitt 16.2.1.6.1.3.5): Das Modul
    /// und alle, die über [[AsyncParentModules]] auf es warten, sind mit
    /// `error` ausgewertet, in der Reihenfolge der Rekursion.
    fn async_module_execution_rejected(
        &mut self,
        scope: &Scope,
        module: ModuleId,
        error: Local<'_>,
    ) -> Completion<()> {
        let mut work = vec![module];
        while let Some(current) = work.pop() {
            if self.module_status(current) != Some(ModuleStatus::EvaluatingAsync) {
                continue;
            }
            let handle = self.heap_mut().persist(error);
            let Some(record) = self.module_record_mut(current) else {
                self.heap_mut().release(handle);
                continue;
            };
            record.evaluation_error = Some(EvaluationError::Throw(handle));
            record.status = ModuleStatus::Evaluated;
            record.async_evaluation_order = AsyncOrder::Done;
            work.extend(record.async_parent_modules.iter().rev().copied());
            self.settle_top_level(scope, current, Some(error))?;
        }
        Ok(())
    }

    /// Ein Lauf des Moduls endete ohne Ausnahme abrupt: Das Modul und alle,
    /// die über [[AsyncParentModules]] auf es warten, sind mit diesem
    /// Abbruch ausgewertet. Ihre Promises bleiben offen; der Agent gibt sie
    /// frei.
    pub(crate) fn abort_async_module(&mut self, module: ModuleId, abrupt: Abrupt) {
        let mut work = vec![module];
        while let Some(current) = work.pop() {
            let Some(record) = self.module_record_mut(current) else {
                continue;
            };
            if record.status == ModuleStatus::Evaluated {
                continue;
            }
            record.evaluation_error = Some(EvaluationError::Abrupt(abrupt));
            record.status = ModuleStatus::Evaluated;
            record.async_evaluation_order = AsyncOrder::Done;
            work.extend(record.async_parent_modules.iter().rev().copied());
            if let Some(capability) = record.top_level_capability.take() {
                self.heap_mut().release(capability);
            }
        }
    }

    /// Call von [[Resolve]] mit *undefined* oder, mit einem Grund, von
    /// [[Reject]] der [[TopLevelCapability]] des Moduls, wenn es eine hat.
    fn settle_top_level(
        &mut self,
        scope: &Scope,
        module: ModuleId,
        error: Option<Local<'_>>,
    ) -> Completion<()> {
        let Some(record) = self.module_record(module) else {
            return Ok(());
        };
        let realm = record.realm;
        let Some(capability) = &record.top_level_capability else {
            return Ok(());
        };
        let heap = self.heap();
        let promise = heap.local(scope, heap.get_persistent(capability));
        match error {
            Some(error) => settle_pending(self, scope, realm, promise, error, false),
            None => {
                let undefined = heap.local(scope, Value::UNDEFINED);
                settle_pending(self, scope, realm, promise, undefined, true)
            }
        }
    }
}

/// Die Nummer des Moduls, die eine der beiden Funktionen einfängt.
fn captured_module(agent: &Agent, scope: &Scope, call: &BuiltinCall<'_, '_>) -> Option<ModuleId> {
    let captures = call.captures(agent, scope);
    let index = agent.heap().get(captures).as_int32()?;
    usize::try_from(index).ok().map(ModuleId::from_index)
}

/// *onFulfilled*: AsyncModuleExecutionFulfilled.
fn fulfilled<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if let Some(module) = captured_module(agent, scope, call) {
        agent.async_module_execution_fulfilled(scope, module)?;
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// *onRejected*: AsyncModuleExecutionRejected mit dem Grund.
fn rejected<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if let Some(module) = captured_module(agent, scope, call) {
        let error = call.argument(agent, scope, 0);
        agent.async_module_execution_rejected(scope, module, error)?;
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}
