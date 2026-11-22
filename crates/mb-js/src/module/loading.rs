//! Das Laden des Graphen: LoadRequestedModules, InnerModuleLoading,
//! ContinueModuleLoading und FinishLoadingImportedModule (Abschnitte
//! 16.2.1.6.1.1 und 16.2.1.11).
//!
//! Ein GraphLoadingState Record liegt im Agenten unter seiner Nummer; die
//! Nummer ist das *payload*, das der Host mit HostLoadImportedModule bekommt
//! und an FinishLoadingImportedModule zurückgibt. InnerModuleLoading läuft
//! als Schleife über einen Stapel von Aufgaben statt als Rekursion durch
//! den Host: Beendet der Host das Laden sofort, kommt das geladene Modul
//! oben auf den Stapel und wird bearbeitet, bevor die nächste Anfrage des
//! anfordernden Moduls an der Reihe ist. Das ist die Reihenfolge der
//! Rekursion. Beendet der Host es später, setzt FinishLoadingImportedModule
//! die Schleife fort.

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::heap::{Local, Persistent, Scope};
use crate::promise::settle_pending;
use crate::realm::RealmId;
use crate::value::Value;

use super::{ModuleId, ModuleRequest, ModuleStatus, Referrer};

/// Das *payload* für HostLoadImportedModule: die Nummer des
/// GraphLoadingState Record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LoadPayload(u64);

/// Ein Schritt von InnerModuleLoading.
#[derive(Clone, Copy, Debug)]
enum Task {
    /// InnerModuleLoading für das Modul.
    Visit(ModuleId),
    /// Die Schleife über [[RequestedModules]] ab der Stelle.
    Requests { module: ModuleId, next: usize },
}

/// Ein GraphLoadingState Record (Abschnitt 16.2.1.6.1.1).
pub(crate) struct LoadingState {
    /// [[IsLoading]]
    is_loading: bool,
    /// [[PendingModulesCount]]
    pending: usize,
    /// [[Visited]], in der Reihenfolge des Besuchs.
    visited: Vec<ModuleId>,
    /// Das Promise aus [[PromiseCapability]].
    promise: Persistent,
    realm: RealmId,
    tasks: Vec<Task>,
    /// Ob die Schleife gerade läuft.
    running: bool,
}

impl Agent {
    /// LoadRequestedModules (Abschnitt 16.2.1.6.1.1) des Moduls: lädt über
    /// den Host alle Module, die es direkt oder indirekt braucht, und gibt
    /// ein Promise zurück, das danach erfüllt oder mit dem Fehler beim
    /// Laden abgelehnt ist. Lädt der Host sofort, ist es das schon bei der
    /// Rückkehr.
    pub fn load_requested_modules<'s>(
        &mut self,
        scope: &'s Scope,
        module: ModuleId,
    ) -> Completion<Local<'s>> {
        let realm = self.module(module)?.realm;
        let promise = self.new_module_promise(scope, realm)?;
        let handle = self.heap_mut().persist(promise);
        let id = self.next_module_load;
        self.next_module_load = id.saturating_add(1);
        self.module_loads.insert(
            id,
            LoadingState {
                is_loading: true,
                pending: 1,
                visited: Vec::new(),
                promise: handle,
                realm,
                tasks: vec![Task::Visit(module)],
                running: false,
            },
        );
        self.drive_loading(scope, id)?;
        Ok(promise)
    }

    /// FinishLoadingImportedModule (Abschnitt 16.2.1.11) für den Host:
    /// `result` ist das geladene Modul oder [`Abrupt::Throw`] mit der
    /// Ausnahme im Agenten. Für ein `payload`, dessen Laden schon zu Ende
    /// ist, merkt sich der Agent nur das Modul. Eine andere abrupte
    /// Beendigung als eine Ausnahme beendet das Laden ohne Ergebnis und
    /// kehrt zurück.
    pub fn finish_loading_imported_module(
        &mut self,
        scope: &Scope,
        referrer: Referrer,
        request: &ModuleRequest,
        payload: LoadPayload,
        result: Completion<ModuleId>,
    ) -> Completion<()> {
        let Referrer::Module(referrer) = referrer;
        match result {
            Ok(module) => {
                if self.module_record(module).is_none() {
                    return Err(self.throw_error(ErrorKind::TypeError));
                }
                if let Some(record) = self.module_record_mut(referrer)
                    && !record.loaded.iter().any(|(loaded, _)| loaded == request)
                {
                    record.loaded.push((request.clone(), module));
                }
                self.continue_loading(scope, payload.0, Ok(module))
            }
            Err(Abrupt::Throw) => {
                let realm = self
                    .module_loads
                    .get(&payload.0)
                    .map(|state| state.realm)
                    .or_else(|| self.module_realm(referrer));
                let Some(realm) = realm else {
                    return Err(self.throw_error(ErrorKind::TypeError));
                };
                let error = self.take_thrown_value(scope, realm)?;
                self.continue_loading(scope, payload.0, Err(error))
            }
            Err(abrupt) => {
                self.module_loads.remove(&payload.0);
                Err(abrupt)
            }
        }
    }

    /// ContinueModuleLoading (Abschnitt 16.2.1.6.1.1).
    fn continue_loading(
        &mut self,
        scope: &Scope,
        id: u64,
        result: Result<ModuleId, Local<'_>>,
    ) -> Completion<()> {
        let Some(state) = self.module_loads.get_mut(&id) else {
            return Ok(());
        };
        if !state.is_loading {
            return Ok(());
        }
        match result {
            Ok(module) => {
                state.tasks.push(Task::Visit(module));
                self.drive_loading(scope, id)
            }
            Err(error) => {
                state.is_loading = false;
                let realm = state.realm;
                let running = state.running;
                let promise = self.loading_promise(scope, id);
                if !running {
                    self.drop_loading(id);
                }
                settle_pending(self, scope, realm, promise, error, false)
            }
        }
    }

    /// InnerModuleLoading als Schleife, bis der Stapel leer ist, das Laden
    /// endet oder der Host ein Modul später liefert. Läuft sie schon, legt
    /// ein Aufruf aus dem Host nur das Modul auf den Stapel.
    fn drive_loading(&mut self, scope: &Scope, id: u64) -> Completion<()> {
        match self.module_loads.get_mut(&id) {
            Some(state) if !state.running => state.running = true,
            _ => return Ok(()),
        }
        let result = self.loading_loop(scope, id);
        let finished = match self.module_loads.get_mut(&id) {
            Some(state) => {
                state.running = false;
                !state.is_loading
            }
            None => false,
        };
        if finished || result.is_err() {
            self.drop_loading(id);
        }
        result
    }

    /// Das Promise des GraphLoadingState Record.
    fn loading_promise<'s>(&self, scope: &'s Scope, id: u64) -> Local<'s> {
        let heap = self.heap();
        let value = self
            .module_loads
            .get(&id)
            .map_or(Value::UNDEFINED, |state| {
                heap.get_persistent(&state.promise)
            });
        heap.local(scope, value)
    }

    fn drop_loading(&mut self, id: u64) {
        if let Some(state) = self.module_loads.remove(&id) {
            self.heap_mut().release(state.promise);
        }
    }

    fn loading_loop(&mut self, scope: &Scope, id: u64) -> Completion<()> {
        loop {
            let Some(state) = self.module_loads.get_mut(&id) else {
                return Ok(());
            };
            if !state.is_loading {
                return Ok(());
            }
            let Some(task) = state.tasks.pop() else {
                return Ok(());
            };
            match task {
                Task::Visit(module) => {
                    let Some(record) = self.modules.get(module.index()) else {
                        return Err(self.throw_error(ErrorKind::TypeError));
                    };
                    let state = self.module_loads.get_mut(&id);
                    let Some(state) = state else {
                        return Ok(());
                    };
                    if record.is_cyclic()
                        && record.status == ModuleStatus::New
                        && !state.visited.contains(&module)
                    {
                        state.visited.push(module);
                        state.pending = state.pending.saturating_add(record.requested.len());
                        state.tasks.push(Task::Requests { module, next: 0 });
                    } else {
                        self.finish_visit(scope, id)?;
                    }
                }
                Task::Requests { module, next } => {
                    let request = self.module_requests(module).get(next).cloned();
                    let Some(request) = request else {
                        self.finish_visit(scope, id)?;
                        continue;
                    };
                    if let Some(state) = self.module_loads.get_mut(&id) {
                        state.tasks.push(Task::Requests {
                            module,
                            next: next.saturating_add(1),
                        });
                    }
                    if !self.import_attributes_supported(&request) {
                        // AllImportAttributesSupported ist *false*.
                        let realm = self.module_realm(module).unwrap_or(RealmId::from_index(0));
                        self.throw_error(ErrorKind::SyntaxError);
                        let error = self.take_thrown_value(scope, realm)?;
                        self.continue_loading(scope, id, Err(error))?;
                    } else if let Some(loaded) = self.loaded_module(module, &request) {
                        if let Some(state) = self.module_loads.get_mut(&id) {
                            state.tasks.push(Task::Visit(loaded));
                        }
                    } else {
                        self.host_load_imported_module(scope, module, &request, id)?;
                    }
                }
            }
        }
    }

    /// Die Schritte am Ende von InnerModuleLoading: Ist kein Modul mehr
    /// offen, sind die besuchten ~unlinked~ und das Promise erfüllt.
    fn finish_visit(&mut self, scope: &Scope, id: u64) -> Completion<()> {
        let Some(state) = self.module_loads.get_mut(&id) else {
            return Ok(());
        };
        state.pending = state.pending.saturating_sub(1);
        if state.pending > 0 {
            return Ok(());
        }
        state.is_loading = false;
        let visited = std::mem::take(&mut state.visited);
        let realm = state.realm;
        let promise = self.loading_promise(scope, id);
        for module in visited {
            if let Some(record) = self.module_record_mut(module)
                && record.status == ModuleStatus::New
            {
                record.status = ModuleStatus::Unlinked;
            }
        }
        let undefined = self.heap().local(scope, Value::UNDEFINED);
        settle_pending(self, scope, realm, promise, undefined, true)
    }

    /// AllImportAttributesSupported (Abschnitt 16.2.1.12) mit
    /// HostGetSupportedImportAttributes; ohne Host gibt es keine.
    fn import_attributes_supported(&self, request: &ModuleRequest) -> bool {
        let host = self.host();
        request.attributes.iter().all(|(key, _)| {
            host.as_ref()
                .is_some_and(|host| host.supports_import_attribute(key))
        })
    }

    fn loaded_module(&self, referrer: ModuleId, request: &ModuleRequest) -> Option<ModuleId> {
        self.module_record(referrer)?
            .loaded
            .iter()
            .find(|(loaded, _)| loaded == request)
            .map(|&(_, module)| module)
    }

    /// HostLoadImportedModule (Abschnitt 16.2.1.10). Ohne Host gibt es kein
    /// Modul: Das Laden scheitert mit einem TypeError.
    fn host_load_imported_module(
        &mut self,
        scope: &Scope,
        referrer: ModuleId,
        request: &ModuleRequest,
        id: u64,
    ) -> Completion<()> {
        let referrer = Referrer::Module(referrer);
        let payload = LoadPayload(id);
        match self.host() {
            Some(host) => host.load_imported_module(self, scope, referrer, request, payload),
            None => {
                let error = self.throw_error(ErrorKind::TypeError);
                self.finish_loading_imported_module(scope, referrer, request, payload, Err(error))
            }
        }
    }
}
