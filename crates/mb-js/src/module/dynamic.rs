//! `import()`: EvaluateImportCall und ContinueDynamicImport (Abschnitte
//! 13.3.10.2 und 13.3.10.3).
//!
//! Ein Aufruf legt ein Promise an und fragt den Host mit
//! HostLoadImportedModule nach dem Modul. Das *payload* dafür ist die
//! Nummer eines Eintrags im Agenten, der das Promise hält, bis der Host mit
//! FinishLoadingImportedModule antwortet; danach lädt, linkt und wertet
//! ContinueDynamicImport das Modul aus und erfüllt das Promise mit seinem
//! Namensraum. Die drei Funktionen dafür fangen eine interne Zelle mit dem
//! Promise und der Nummer des Moduls ein.
//!
//! Die PromiseCapability aus NewPromiseCapability(%Promise%) ist ein Promise
//! ohne Executor wie bei async-Funktionen: Ihre Funktionen ruft nur dieser
//! Code, und er löst jedes Promise höchstens einmal auf. Erfüllt wird es
//! mit den Schritten der Funktion *resolve*, denn ein Namensraum kann eine
//! Bindung `then` exportieren.

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_closure};
use crate::heap::{Local, Persistent, Scope};
use crate::key::PropertyKey;
use crate::object::{get_own_property, own_property_keys};
use crate::operations::get;
use crate::promise::{perform_promise_then, promise_create, resolve_promise, settle_pending};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

use super::loading::LoadPayload;
use super::{ModuleId, ModuleRequest, Referrer};

/// Ein Aufruf von `import()`, dessen Modul der Host noch nicht geliefert
/// hat: das Promise aus der PromiseCapability und der Realm des Aufrufs.
pub(crate) struct DynamicImport {
    promise: Persistent,
    realm: RealmId,
}

/// *onRejected* aus ContinueDynamicImport.
static REJECTED: Builtin = Builtin {
    name: "",
    length: 1,
    constructor: false,
    behaviour: rejected,
};

/// *linkAndEvaluate* aus ContinueDynamicImport.
static LINK_AND_EVALUATE: Builtin = Builtin {
    name: "",
    length: 0,
    constructor: false,
    behaviour: link_and_evaluate,
};

/// *onFulfilled* aus *linkAndEvaluate*.
static FULFILLED: Builtin = Builtin {
    name: "",
    length: 0,
    constructor: false,
    behaviour: fulfilled,
};

/// EvaluateImportCall (Abschnitt 13.3.10.2) ab Schritt 8, im aktuellen
/// Realm `realm`, mit dem ausgewerteten Bezeichner und den Optionen: ein
/// neues Promise, das ein Fehler in den Optionen sofort ablehnt; sonst
/// bekommt der Host die Anfrage.
pub(crate) fn evaluate_import_call<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    referrer: Referrer,
    specifier: Local<'_>,
    options: Local<'_>,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::PromisePrototype);
    let promise = promise_create(agent, scope, prototype)?;
    match import_request(agent, scope, specifier, options) {
        Ok(request) => agent.host_load_dynamic(scope, realm, referrer, &request, promise)?,
        Err(Abrupt::Throw) => {
            let error = agent.take_thrown_value(scope, realm)?;
            settle_pending(agent, scope, realm, promise, error, false)?;
        }
        Err(abrupt) => return Err(abrupt),
    }
    Ok(promise)
}

/// Schritte 9 bis 17 von EvaluateImportCall: der ModuleRequest Record aus
/// ToString des Bezeichners und den Attributen in `with` der Optionen. Was
/// IfAbruptRejectPromise ablehnt oder mit einem neuen TypeError abgelehnt
/// wird, wirft hier.
fn import_request(
    agent: &mut Agent,
    scope: &Scope,
    specifier: Local<'_>,
    options: Local<'_>,
) -> Completion<ModuleRequest> {
    let specifier = string_units(agent, scope, specifier)?;
    let mut attributes = Vec::new();
    if !agent.heap().get(options).is_undefined() {
        if !matches!(agent.heap().get(options).kind(), Kind::Object(_)) {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        let with = agent.key_from_units(scope, &"with".encode_utf16().collect::<Vec<_>>())?;
        let object = get(agent, scope, options, with)?;
        if !agent.heap().get(object).is_undefined() {
            if !matches!(agent.heap().get(object).kind(), Kind::Object(_)) {
                return Err(agent.throw_error(ErrorKind::TypeError));
            }
            // EnumerableOwnProperties(…, ~key+value~), ohne die Paare als
            // Arrays, die niemand sieht: erst alle Werte, dann die Prüfung.
            let mut entries = Vec::new();
            for key in own_property_keys(agent, scope, object)? {
                if matches!(key, PropertyKey::Symbol(_)) {
                    continue;
                }
                let Some(descriptor) = get_own_property(agent, scope, object, key)? else {
                    continue;
                };
                if descriptor.enumerable != Some(true) {
                    continue;
                }
                let name = agent.key_as_value(scope, key)?;
                let value = get(agent, scope, object, key)?;
                entries.push((name, value));
            }
            for (name, value) in entries {
                let heap = agent.heap();
                let (Kind::String(name), Kind::String(value)) =
                    (heap.get(name).kind(), heap.get(value).kind())
                else {
                    return Err(agent.throw_error(ErrorKind::TypeError));
                };
                attributes.push((heap.string_units(name), heap.string_units(value)));
            }
        }
        let request = ModuleRequest {
            specifier: Vec::new(),
            attributes,
        };
        if !agent.import_attributes_supported(&request) {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        attributes = request.attributes;
        attributes.sort_by(|left, right| left.0.cmp(&right.0));
    }
    Ok(ModuleRequest {
        specifier,
        attributes,
    })
}

/// Die Codeeinheiten von ToString(`value`).
fn string_units(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<Vec<u16>> {
    let string = crate::conversion::to_string(agent, scope, value)?;
    let heap = agent.heap();
    Ok(match heap.get(string).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => Vec::new(),
    })
}

impl Agent {
    /// Schritt 18 von EvaluateImportCall: HostLoadImportedModule mit einem
    /// neuen Eintrag als *payload*. Ohne Host gibt es kein Modul, und das
    /// Promise wird mit einem TypeError abgelehnt. Endet der Host anders
    /// abrupt als mit einer Ausnahme, gibt der Agent den Eintrag frei.
    fn host_load_dynamic(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        referrer: Referrer,
        request: &ModuleRequest,
        promise: Local<'_>,
    ) -> Completion<()> {
        let id = self.next_module_load;
        self.next_module_load = id.saturating_add(1);
        let handle = self.heap_mut().persist(promise);
        self.dynamic_imports.insert(
            id,
            DynamicImport {
                promise: handle,
                realm,
            },
        );
        let payload = LoadPayload::dynamic(id);
        let result = match self.host() {
            Some(host) => host.load_imported_module(self, scope, referrer, request, payload),
            None => {
                let error = self.throw_error(ErrorKind::TypeError);
                self.finish_loading_imported_module(scope, referrer, request, payload, Err(error))
            }
        };
        match result {
            Ok(()) => Ok(()),
            // Ein Wurf aus dem Host selbst lehnt das Promise ab, wenn er es
            // noch nicht beantwortet hat.
            Err(Abrupt::Throw) => {
                let error = self.take_thrown_value(scope, realm)?;
                self.continue_dynamic_import(scope, id, Err(error))
            }
            Err(abrupt) => {
                if let Some(entry) = self.dynamic_imports.remove(&id) {
                    self.heap_mut().release(entry.promise);
                }
                Err(abrupt)
            }
        }
    }

    /// Der Realm des Aufrufs von `import()` mit der Nummer `id`, solange er
    /// auf den Host wartet.
    pub(super) fn dynamic_import_realm(&self, id: u64) -> Option<RealmId> {
        self.dynamic_imports.get(&id).map(|entry| entry.realm)
    }

    /// Gibt den Eintrag `id` frei, ohne sein Promise aufzulösen.
    pub(super) fn drop_dynamic_import(&mut self, id: u64) {
        if let Some(entry) = self.dynamic_imports.remove(&id) {
            self.heap_mut().release(entry.promise);
        }
    }

    /// ContinueDynamicImport (Abschnitt 13.3.10.3) für den Eintrag `id`
    /// mit dem Modul oder dem Grund aus FinishLoadingImportedModule. Für
    /// einen Eintrag, den es nicht mehr gibt, geschieht nichts.
    pub(super) fn continue_dynamic_import(
        &mut self,
        scope: &Scope,
        id: u64,
        result: Result<ModuleId, Local<'_>>,
    ) -> Completion<()> {
        let Some(entry) = self.dynamic_imports.remove(&id) else {
            return Ok(());
        };
        let realm = entry.realm;
        let heap = self.heap();
        let promise = heap.local(scope, heap.get_persistent(&entry.promise));
        self.heap_mut().release(entry.promise);
        let module = match result {
            Ok(module) => module,
            Err(error) => return settle_pending(self, scope, realm, promise, error, false),
        };
        let index = i32::try_from(module.index()).map_err(|_| Abrupt::OutOfMemory)?;
        let load = self.load_requested_modules(scope, module)?;
        let captures = self.heap_mut().alloc_slots(scope, 2)?;
        let heap = self.heap();
        if let Kind::Internal(cell) = heap.get(captures).kind() {
            heap.set_slot(cell, 0, heap.get(promise));
            heap.set_slot(cell, 1, Value::int32(index));
        }
        let on_rejected = create_builtin_closure(self, scope, realm, &REJECTED, captures)?;
        let link = create_builtin_closure(self, scope, realm, &LINK_AND_EVALUATE, captures)?;
        perform_promise_then(self, scope, realm, load, link, on_rejected, None)?;
        Ok(())
    }
}

/// Das Promise und das Modul, die eine der drei Funktionen einfängt.
fn captured<'s>(
    agent: &Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Option<(Local<'s>, ModuleId)> {
    let captures = call.captures(agent, scope);
    let heap = agent.heap();
    let Kind::Internal(cell) = heap.get(captures).kind() else {
        return None;
    };
    let promise = heap.local(scope, heap.slot(cell, 0)?);
    let index = heap.slot(cell, 1)?.as_int32()?;
    let module = ModuleId::from_index(usize::try_from(index).ok()?);
    Some((promise, module))
}

/// *onRejected*: Call von [[Reject]] mit dem Grund.
fn rejected<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if let Some((promise, _)) = captured(agent, scope, call) {
        let reason = call.argument(agent, scope, 0);
        settle_pending(agent, scope, call.realm, promise, reason, false)?;
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// *linkAndEvaluate*: Link, dann Evaluate; das Promise aus Evaluate
/// bekommt *onFulfilled* und *onRejected*. Ein Fehler beim Linken lehnt
/// ab.
fn link_and_evaluate<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    let Some((promise, module)) = captured(agent, scope, call) else {
        return Ok(undefined);
    };
    let realm = call.realm;
    let evaluated = match agent
        .link_module(module)
        .and_then(|()| agent.evaluate_module(scope, module))
    {
        Ok(evaluated) => evaluated,
        Err(Abrupt::Throw) => {
            let error = agent.take_thrown_value(scope, realm)?;
            settle_pending(agent, scope, realm, promise, error, false)?;
            return Ok(undefined);
        }
        Err(abrupt) => return Err(abrupt),
    };
    let captures = call.captures(agent, scope);
    let on_fulfilled = create_builtin_closure(agent, scope, realm, &FULFILLED, captures)?;
    let on_rejected = create_builtin_closure(agent, scope, realm, &REJECTED, captures)?;
    perform_promise_then(
        agent,
        scope,
        realm,
        evaluated,
        on_fulfilled,
        on_rejected,
        None,
    )?;
    Ok(undefined)
}

/// *onFulfilled*: Call von [[Resolve]] mit dem Namensraum des Moduls.
fn fulfilled<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if let Some((promise, module)) = captured(agent, scope, call) {
        let namespace = agent.module_namespace(scope, module)?;
        resolve_promise(agent, scope, call.realm, promise, namespace)?;
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}
