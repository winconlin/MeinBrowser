//! Realms (Abschnitt 9.3) und die globale Umgebung (Abschnitt 9.1.1.4).
//!
//! Von den Intrinsics gibt es bisher nur %Object.prototype%, die übrigen
//! kommen mit den eingebauten Objekten. Das globale Objekt hat die
//! Werteigenschaften aus Abschnitt 19.1, und [[GlobalThisValue]] ist immer
//! das globale Objekt.
//!
//! Die globale Umgebung besteht wie in der Spezifikation aus einem
//! Objekt-Record über dem globalen Objekt und einem deklarativen Record für
//! `let`, `const` und `class`. Dessen Bindungen hält der Realm nach ihrem
//! Namen; vor der Initialisierung ist ihr Wert die Lücke.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use crate::agent::{Agent, Completion, ErrorKind};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Persistent, Scope};
use crate::key::PropertyKey;
use crate::object::{self, ordinary_object_create};
use crate::operations::{define_property_or_throw, get, has_own_property, has_property, set};
use crate::value::Value;

/// Ein Realm des Agents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RealmId(usize);

/// Eine Bindung des deklarativen Records.
struct Binding {
    /// Der Wert, die Lücke vor der Initialisierung.
    value: Persistent,
    /// Unveränderliche Bindungen stammen von `const` und sind immer
    /// strikt.
    mutable: bool,
}

pub(crate) struct Realm {
    /// %Object.prototype%
    object_prototype: Persistent,
    /// [[GlobalObject]], zugleich [[GlobalThisValue]].
    global_object: Persistent,
    /// [[DeclarativeRecord]] der globalen Umgebung.
    declarative: BTreeMap<Vec<u16>, Binding>,
}

/// Was GetIdentifierReference (Abschnitt 9.1.2.1) in der globalen Umgebung
/// findet: eine nicht auflösbare Referenz oder eine mit der globalen
/// Umgebung als [[Base]]. Ob dann der deklarative Record oder der
/// Objekt-Record gilt, entscheiden GetBindingValue und SetMutableBinding
/// (Abschnitte 9.1.1.4.6 und 9.1.1.4.5) erst beim Zugriff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GlobalReference {
    Unresolvable,
    Global,
}

impl GlobalReference {
    /// Die Zahl, die der Interpreter im Register ablegt.
    pub(crate) fn number(self) -> i32 {
        match self {
            GlobalReference::Unresolvable => 0,
            GlobalReference::Global => 1,
        }
    }

    pub(crate) fn from_number(number: f64) -> GlobalReference {
        if number == 1.0 {
            GlobalReference::Global
        } else {
            GlobalReference::Unresolvable
        }
    }
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl Agent {
    /// InitializeHostDefinedRealm (Abschnitt 9.3.1) ohne den
    /// Ausführungskontext: %Object.prototype%, das globale Objekt als
    /// gewöhnliches Objekt mit diesem Prototyp und SetDefaultGlobalBindings
    /// (Abschnitt 9.3.3) für die Eigenschaften, die es schon gibt.
    pub fn create_realm(&mut self) -> Completion<RealmId> {
        let realm = self.scope(|agent, scope| -> Completion<Realm> {
            let prototype = ordinary_object_create(agent, scope, None)?;
            let global = ordinary_object_create(agent, scope, Some(prototype))?;
            set_default_global_bindings(agent, scope, global)?;
            let heap = agent.heap_mut();
            Ok(Realm {
                object_prototype: heap.persist(prototype),
                global_object: heap.persist(global),
                declarative: BTreeMap::new(),
            })
        })?;
        self.realms.push(realm);
        Ok(RealmId(self.realms.len().saturating_sub(1)))
    }

    fn realm(&self, realm: RealmId) -> Option<&Realm> {
        self.realms.get(realm.0)
    }

    /// %Object.prototype% des Realms.
    pub fn object_prototype<'s>(&self, scope: &'s Scope, realm: RealmId) -> Local<'s> {
        let value = self.realm(realm).map_or(Value::UNDEFINED, |realm| {
            self.heap().get_persistent(&realm.object_prototype)
        });
        self.heap().local(scope, value)
    }

    /// Das globale Objekt des Realms.
    pub fn global_object<'s>(&self, scope: &'s Scope, realm: RealmId) -> Local<'s> {
        let value = self.realm(realm).map_or(Value::UNDEFINED, |realm| {
            self.heap().get_persistent(&realm.global_object)
        });
        self.heap().local(scope, value)
    }

    /// Der Wert einer Bindung des deklarativen Records, `None` ohne
    /// Bindung.
    fn declarative_value<'s>(
        &self,
        scope: &'s Scope,
        realm: RealmId,
        name: &[u16],
    ) -> Option<(Local<'s>, bool)> {
        let binding = self.realm(realm)?.declarative.get(name)?;
        let value = self.heap().get_persistent(&binding.value);
        Some((self.heap().local(scope, value), binding.mutable))
    }

    // Die globale Umgebung

    /// HasLexicalDeclaration (Abschnitt 9.1.1.4.12).
    pub(crate) fn has_lexical_declaration(&self, realm: RealmId, name: &[u16]) -> bool {
        self.realm(realm)
            .is_some_and(|realm| realm.declarative.contains_key(name))
    }

    /// HasRestrictedGlobalProperty (Abschnitt 9.1.1.4.13).
    pub(crate) fn has_restricted_global_property(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        key: PropertyKey<'_>,
    ) -> Completion<bool> {
        let global = self.global_object(scope, realm);
        let existing = object::get_own_property(self, scope, global, key)?;
        Ok(existing.is_some_and(|property| property.configurable == Some(false)))
    }

    /// CanDeclareGlobalVar (Abschnitt 9.1.1.4.14).
    pub(crate) fn can_declare_global_var(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        key: PropertyKey<'_>,
    ) -> Completion<bool> {
        let global = self.global_object(scope, realm);
        if has_own_property(self, scope, global, key)? {
            return Ok(true);
        }
        object::is_extensible(self, global)
    }

    /// CreateGlobalVarBinding (Abschnitt 9.1.1.4.16).
    pub(crate) fn create_global_var_binding(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        key: PropertyKey<'_>,
        deletable: bool,
    ) -> Completion<()> {
        let global = self.global_object(scope, realm);
        let has_property = has_own_property(self, scope, global, key)?;
        let extensible = object::is_extensible(self, global)?;
        if !has_property && extensible {
            // CreateMutableBinding und InitializeBinding des Objekt-Records
            // (Abschnitte 9.1.1.2.2 und 9.1.1.2.4).
            let undefined = self.heap().local(scope, Value::UNDEFINED);
            let descriptor = PropertyDescriptor::data(undefined, true, true, deletable);
            define_property_or_throw(self, scope, global, key, &descriptor)?;
            self.object_set_mutable_binding(scope, global, key, undefined, false)?;
        }
        Ok(())
    }

    /// CreateMutableBinding oder CreateImmutableBinding der globalen
    /// Umgebung (Abschnitte 9.1.1.4.2 und 9.1.1.4.3) für `let` und `const`.
    pub(crate) fn create_lexical_binding(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        name: &[u16],
        mutable: bool,
    ) -> Completion<()> {
        if self.has_lexical_declaration(realm, name) {
            return Err(self.throw_error(ErrorKind::TypeError));
        }
        let hole = self.heap().local(scope, Value::HOLE);
        let value = self.heap_mut().persist(hole);
        if let Some(realm) = self.realms.get_mut(realm.0) {
            realm
                .declarative
                .insert(name.to_vec(), Binding { value, mutable });
        }
        Ok(())
    }

    /// InitializeBinding der globalen Umgebung (Abschnitt 9.1.1.4.4) für
    /// eine Bindung des deklarativen Records.
    pub(crate) fn initialize_lexical_binding(
        &mut self,
        realm: RealmId,
        name: &[u16],
        value: Local<'_>,
    ) {
        let (realms, heap) = self.realms_and_heap();
        if let Some(binding) = realms
            .get(realm.0)
            .and_then(|realm| realm.declarative.get(name))
        {
            heap.set_persistent(&binding.value, value);
        }
    }

    /// GetIdentifierReference (Abschnitt 9.1.2.1) mit HasBinding der
    /// globalen Umgebung (Abschnitt 9.1.1.4.1).
    pub(crate) fn resolve_global(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        name: &[u16],
        key: PropertyKey<'_>,
    ) -> Completion<GlobalReference> {
        if self.has_lexical_declaration(realm, name) {
            return Ok(GlobalReference::Global);
        }
        let global = self.global_object(scope, realm);
        Ok(if has_property(self, global, key)? {
            GlobalReference::Global
        } else {
            GlobalReference::Unresolvable
        })
    }

    /// GetValue (Abschnitt 6.2.5.5) für eine Referenz in die globale
    /// Umgebung, mit GetBindingValue (Abschnitte 9.1.1.4.6, 9.1.1.1.6 und
    /// 9.1.1.2.6).
    pub(crate) fn get_global<'s>(
        &mut self,
        scope: &'s Scope,
        realm: RealmId,
        reference: GlobalReference,
        name: &[u16],
        key: PropertyKey<'_>,
        strict: bool,
    ) -> Completion<Local<'s>> {
        if reference == GlobalReference::Unresolvable {
            return Err(self.throw_error(ErrorKind::ReferenceError));
        }
        if let Some((value, _)) = self.declarative_value(scope, realm, name) {
            if self.heap().get(value).is_hole() {
                return Err(self.throw_error(ErrorKind::ReferenceError));
            }
            return Ok(value);
        }
        let global = self.global_object(scope, realm);
        if !has_property(self, global, key)? {
            if !strict {
                return Ok(self.heap().local(scope, Value::UNDEFINED));
            }
            return Err(self.throw_error(ErrorKind::ReferenceError));
        }
        get(self, scope, global, key)
    }

    /// PutValue (Abschnitt 6.2.5.6) für eine Referenz in die globale
    /// Umgebung, mit SetMutableBinding (Abschnitte 9.1.1.4.5, 9.1.1.1.5
    /// und 9.1.1.2.5).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn put_global(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        reference: GlobalReference,
        name: &[u16],
        key: PropertyKey<'_>,
        value: Local<'_>,
        strict: bool,
    ) -> Completion<()> {
        let global = self.global_object(scope, realm);
        if reference == GlobalReference::Unresolvable {
            if strict {
                return Err(self.throw_error(ErrorKind::ReferenceError));
            }
            return set(self, scope, global, key, value, false);
        }
        let Some((current, mutable)) = self.declarative_value(scope, realm, name) else {
            return self.object_set_mutable_binding(scope, global, key, value, strict);
        };
        if self.heap().get(current).is_hole() {
            return Err(self.throw_error(ErrorKind::ReferenceError));
        }
        if !mutable {
            // Unveränderliche Bindungen sind strikt.
            return Err(self.throw_error(ErrorKind::TypeError));
        }
        self.initialize_lexical_binding(realm, name, value);
        Ok(())
    }

    /// SetMutableBinding des Objekt-Records (Abschnitt 9.1.1.2.5).
    fn object_set_mutable_binding(
        &mut self,
        scope: &Scope,
        object: Local<'_>,
        key: PropertyKey<'_>,
        value: Local<'_>,
        strict: bool,
    ) -> Completion<()> {
        let still_exists = has_property(self, object, key)?;
        if !still_exists && strict {
            return Err(self.throw_error(ErrorKind::ReferenceError));
        }
        set(self, scope, object, key, value, strict)
    }

    /// DeleteBinding der globalen Umgebung (Abschnitt 9.1.1.4.7). Die
    /// Bindungen des deklarativen Records stammen alle aus Deklarationen
    /// und lassen sich nicht löschen.
    pub(crate) fn delete_global(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        name: &[u16],
        key: PropertyKey<'_>,
    ) -> Completion<bool> {
        if self.has_lexical_declaration(realm, name) {
            return Ok(false);
        }
        let global = self.global_object(scope, realm);
        if has_own_property(self, scope, global, key)? {
            return object::delete(self, scope, global, key);
        }
        Ok(true)
    }
}

/// SetDefaultGlobalBindings (Abschnitt 9.3.3) für die Werteigenschaften aus
/// Abschnitt 19.1.
fn set_default_global_bindings(
    agent: &mut Agent,
    scope: &Scope,
    global: Local<'_>,
) -> Completion<()> {
    let infinity = agent.heap().local(scope, Value::number(f64::INFINITY));
    let nan = agent.heap().local(scope, Value::number(f64::NAN));
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    // globalThis ist schreibbar und konfigurierbar, die übrigen sind
    // keines von beiden. Aufzählbar ist keine.
    let properties = [
        ("globalThis", global, true),
        ("Infinity", infinity, false),
        ("NaN", nan, false),
        ("undefined", undefined, false),
    ];
    for (name, value, writable) in properties {
        let key = agent.key_from_units(scope, &units(name))?;
        let descriptor = PropertyDescriptor::data(value, writable, false, writable);
        define_property_or_throw(agent, scope, global, key, &descriptor)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
