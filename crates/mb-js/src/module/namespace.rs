//! Namensraum-Objekte von Modulen (Module Namespace Exotic Objects,
//! Abschnitt 10.4.6) und GetModuleNamespace (Abschnitt 16.2.1.13).
//!
//! Das Objekt hat als internes Feld die Nummer seines Moduls; [[Exports]]
//! steht im Record des Moduls, nach Codeeinheiten geordnet und mit dem
//! Ergebnis von ResolveExport für jeden Namen. Nach dem Laden ändert sich
//! das nicht mehr, darum muss [[Get]] nicht jedes Mal neu auflösen. Für
//! Symbole gelten die gewöhnlichen internen Methoden; das Objekt hat als
//! einzige eigene Eigenschaft @@toStringTag, keinen Prototyp und ist nicht
//! erweiterbar. Die internen Methoden für andere Schlüssel stehen im Modul
//! `object` und fragen hier nach den Exporten.

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, ObjectClass, Persistent, Scope};
use crate::key::PropertyKey;
use crate::key::WellKnownSymbol;
use crate::object::{INLINE, define_own_property, prevent_extensions};
use crate::value::{Kind, Value};

use super::{BindingName, ModuleId, Resolution, ResolvedBinding, binding_value};

/// [[Namespace]] eines Moduls.
pub(crate) struct Namespace {
    object: Persistent,
    /// [[Exports]] mit ihren Bindungen.
    exports: Vec<(Vec<u16>, ResolvedBinding)>,
}

/// Die Codeeinheiten eines Schlüssels, der kein Symbol ist.
fn key_units(agent: &Agent, key: PropertyKey<'_>) -> Option<Vec<u16>> {
    match key {
        PropertyKey::Index(index) => Some(index.to_string().encode_utf16().collect()),
        PropertyKey::String(string) => match agent.heap().get(string).kind() {
            Kind::String(reference) => Some(agent.heap().string_units(reference)),
            _ => None,
        },
        PropertyKey::Symbol(_) => None,
    }
}

/// Das Modul des Namensraum-Objekts.
fn module_of(agent: &Agent, object: Local<'_>) -> Option<ModuleId> {
    let heap = agent.heap();
    let Kind::Object(reference) = heap.get(object).kind() else {
        return None;
    };
    let index = heap.namespace_field(reference)?.as_int32()?;
    usize::try_from(index).ok().map(ModuleId::from_index)
}

/// Die Bindung des Exports `key`, wenn das Objekt ihn hat.
fn export(agent: &Agent, object: Local<'_>, key: PropertyKey<'_>) -> Option<ResolvedBinding> {
    let name = key_units(agent, key)?;
    let module = module_of(agent, object)?;
    let namespace = agent.module_record(module)?.namespace.as_ref()?;
    let index = namespace
        .exports
        .binary_search_by(|(export, _)| export.as_slice().cmp(&name))
        .ok()?;
    namespace
        .exports
        .get(index)
        .map(|(_, binding)| binding.clone())
}

/// Ob [[Exports]] den Schlüssel enthält ([[HasProperty]], Abschnitt
/// 10.4.6.7, für Schlüssel, die keine Symbole sind).
pub(crate) fn has_export(agent: &Agent, object: Local<'_>, key: PropertyKey<'_>) -> bool {
    export(agent, object, key).is_some()
}

/// [[Get]] (Abschnitt 10.4.6.8) für einen Schlüssel, der kein Symbol ist:
/// *undefined* ohne Export, sonst der Wert der Bindung, auf die er
/// verweist; vor ihrer Initialisierung ein ReferenceError.
pub(crate) fn get<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    key: PropertyKey<'_>,
) -> Completion<Local<'s>> {
    let Some(binding) = export(agent, object, key) else {
        return Ok(agent.heap().local(scope, Value::UNDEFINED));
    };
    let name = match binding.name {
        BindingName::Namespace => return agent.module_namespace(scope, binding.module),
        BindingName::Name(name) => name,
    };
    let Some(record) = agent.module_record(binding.module) else {
        return Err(agent.throw_error(ErrorKind::ReferenceError));
    };
    let slot = record
        .slot_of(&name)
        .and_then(|slot| usize::try_from(slot).ok());
    let heap = agent.heap();
    let value = match (heap.get_persistent(record.environment()).kind(), slot) {
        (Kind::Internal(environment), Some(slot)) => heap
            .slot(environment, slot)
            .and_then(|value| binding_value(heap, value)),
        _ => None,
    };
    match value {
        Some(value) => Ok(heap.local(scope, value)),
        None => Err(agent.throw_error(ErrorKind::ReferenceError)),
    }
}

/// Die Schlüssel aus [[Exports]] in ihrer Reihenfolge ([[OwnPropertyKeys]],
/// Abschnitt 10.4.6.11, vor den Symbolen).
pub(crate) fn export_keys<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    keys: &mut Vec<PropertyKey<'s>>,
) -> Completion<()> {
    let names: Vec<Vec<u16>> = module_of(agent, object)
        .and_then(|module| agent.module_record(module))
        .and_then(|record| record.namespace.as_ref())
        .map(|namespace| {
            namespace
                .exports
                .iter()
                .map(|(name, _)| name.clone())
                .collect()
        })
        .unwrap_or_default();
    for name in names {
        keys.push(agent.key_from_units(scope, &name)?);
    }
    Ok(())
}

impl Agent {
    /// GetModuleNamespace (Abschnitt 16.2.1.13): beim ersten Mal ein neues
    /// Namensraum-Objekt mit den Namen aus GetExportedNames, die
    /// ResolveExport eindeutig auflöst (ModuleNamespaceCreate, Abschnitt
    /// 10.4.6.12).
    pub fn module_namespace<'s>(
        &mut self,
        scope: &'s Scope,
        module: ModuleId,
    ) -> Completion<Local<'s>> {
        self.module(module)?;
        let known = self
            .module_record(module)
            .and_then(|record| record.namespace.as_ref())
            .map(|namespace| self.heap().get_persistent(&namespace.object).bits());
        if let Some(bits) = known {
            return Ok(self.heap().local(scope, Value::from_bits(bits)));
        }
        let mut exports: Vec<(Vec<u16>, ResolvedBinding)> = self
            .exported_names(module)
            .into_iter()
            .filter_map(|name| match self.resolve_export(module, &name) {
                Resolution::Found(binding) => Some((name, binding)),
                Resolution::None | Resolution::Ambiguous => None,
            })
            .collect();
        exports.sort_by(|(a, _), (b, _)| a.cmp(b));
        let index = i32::try_from(module.index()).map_err(|_| Abrupt::OutOfMemory)?;
        let shape = self.empty_shape(scope);
        let object =
            self.heap_mut()
                .alloc_object(scope, ObjectClass::Namespace, INLINE, shape, None)?;
        let heap = self.heap();
        if let Kind::Object(reference) = heap.get(object).kind() {
            heap.set_namespace_field(reference, Value::int32(index));
        }
        // Der Record braucht [[Exports]], bevor die Eigenschaft entsteht.
        let handle = self.heap_mut().persist(object);
        if let Some(record) = self.module_record_mut(module) {
            record.namespace = Some(Namespace {
                object: handle,
                exports,
            });
        }
        let tag = self.well_known_symbol(scope, WellKnownSymbol::ToStringTag);
        let value = self.atom(scope, &"Module".encode_utf16().collect::<Vec<_>>())?;
        let descriptor = PropertyDescriptor::data(value, false, false, false);
        define_own_property(self, scope, object, PropertyKey::Symbol(tag), &descriptor)?;
        prevent_extensions(self, object)?;
        Ok(object)
    }
}
