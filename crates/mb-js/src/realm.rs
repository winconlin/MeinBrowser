//! Realms (Abschnitt 9.3) und die globale Umgebung (Abschnitt 9.1.1.4).
//!
//! Von den Intrinsics gibt es bisher die aus [`Intrinsic`]: die Prototypen,
//! die Literale und Zugriffe auf primitive Werte brauchen, was die Harness
//! von Test262 braucht, die Iteratoren für `for`-`of` und die
//! Ausbreitung und die Promises (siehe `builtins`); die übrigen kommen mit
//! den eingebauten Objekten. Das globale Objekt hat die Werteigenschaften
//! aus Abschnitt 19.1, die Funktionen aus 19.2, die Konstruktoren aus
//! 19.3 und die Objekte aus 19.4, die es schon gibt, und die Eigenschaften
//! des Hosts. [[GlobalThisValue]] ist immer das
//! globale Objekt.
//!
//! Die globale Umgebung besteht wie in der Spezifikation aus einem
//! Objekt-Record über dem globalen Objekt und einem deklarativen Record für
//! `let`, `const` und `class`. Dessen Bindungen hält der Realm nach ihrem
//! Namen; vor der Initialisierung ist ihr Wert die Lücke.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::builtin_function_with_prototype;
use crate::builtins::{self, FUNCTION_PROTOTYPE};
use crate::descriptor::PropertyDescriptor;
use crate::function::add_restricted_function_properties;
use crate::heap::{Local, Persistent, Scope};
use crate::key::PropertyKey;
use crate::object::{
    self, array_create, make_prototype_immutable, ordinary_object_create, primitive_object_create,
    string_create,
};
use crate::operations::{define_property_or_throw, get, has_own_property, has_property, set};
use crate::value::Value;

/// Ein Realm des Agents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RealmId(usize);

impl RealmId {
    /// Die Nummer des Realms im Agent, wie sie ein Funktionsobjekt hält.
    pub(crate) fn index(self) -> usize {
        self.0
    }

    pub(crate) fn from_index(index: usize) -> RealmId {
        RealmId(index)
    }
}

/// Eine Bindung des deklarativen Records.
struct Binding {
    /// Der Wert, die Lücke vor der Initialisierung.
    value: Persistent,
    /// Unveränderliche Bindungen stammen von `const` und sind immer
    /// strikt.
    mutable: bool,
}

/// Die Intrinsics (Abschnitt 6.1.7.4), die es schon gibt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Intrinsic {
    /// %Object.prototype% (Abschnitt 20.1.3) mit allen Eigenschaften, auch
    /// denen aus Anhang B.2.2.
    ObjectPrototype,
    /// %Function.prototype% (Abschnitt 20.2.3), selbst eine eingebaute
    /// Funktion, mit allen Eigenschaften, auch `caller` und `arguments` aus
    /// AddRestrictedFunctionProperties (Abschnitt 10.2.4).
    FunctionPrototype,
    /// %ThrowTypeError% (Abschnitt 10.2.4.1): wirft einen TypeError, ist
    /// nicht erweiterbar, `length` und `name` sind nicht konfigurierbar.
    ThrowTypeError,
    /// %Array.prototype% (Abschnitt 23.1.3): ein Array mit der Länge 0.
    ArrayPrototype,
    /// %Boolean.prototype% (Abschnitt 20.3.3), ein Boolean-Objekt mit
    /// *false*.
    BooleanPrototype,
    /// %Number.prototype% (Abschnitt 21.1.3), ein Number-Objekt mit +0.
    NumberPrototype,
    /// %String.prototype% (Abschnitt 22.1.3), ein String-Objekt mit der
    /// leeren Zeichenkette.
    StringPrototype,
    /// %Symbol.prototype% (Abschnitt 20.4.3).
    SymbolPrototype,
    /// %Iterator.prototype% (Abschnitt 27.1.3.3).
    IteratorPrototype,
    /// %IteratorHelperPrototype% (Abschnitt 27.1.2.1).
    IteratorHelperPrototype,
    /// %WrapForValidIteratorPrototype% (Abschnitt 27.1.3.2.2.1).
    WrapForValidIteratorPrototype,
    /// %ArrayIteratorPrototype% (Abschnitt 23.1.5.2).
    ArrayIteratorPrototype,
    /// %StringIteratorPrototype% (Abschnitt 22.1.5.1).
    StringIteratorPrototype,
    /// %AsyncIteratorPrototype% (Abschnitt 27.1.4), bisher nur mit
    /// @@asyncIterator.
    AsyncIteratorPrototype,
    /// %AsyncFromSyncIteratorPrototype% (Abschnitt 27.1.5.2) mit `next`,
    /// `return` und `throw`.
    AsyncFromSyncIteratorPrototype,
    /// %GeneratorFunction.prototype% (Abschnitt 27.6.3), der Prototyp der
    /// Generatorfunktionen, bisher ohne `constructor`.
    GeneratorFunctionPrototype,
    /// %GeneratorPrototype% (Abschnitt 27.8.1) mit `next`, `return` und
    /// `throw`.
    GeneratorPrototype,
    /// %AsyncGeneratorFunction.prototype% (Abschnitt 27.7.3), der Prototyp
    /// der async-Generatorfunktionen, bisher ohne `constructor`.
    AsyncGeneratorFunctionPrototype,
    /// %AsyncGeneratorPrototype% (Abschnitt 27.9.1) mit `next`, `return` und
    /// `throw`.
    AsyncGeneratorPrototype,
    /// %AsyncFunction.prototype% (Abschnitt 27.10.3), der Prototyp der
    /// async-Funktionen, bisher ohne `constructor`.
    AsyncFunctionPrototype,
    /// %Promise.prototype% (Abschnitt 27.5.5) mit `catch`, `finally` und
    /// `then`.
    PromisePrototype,
    /// %Map.prototype% (Abschnitt 24.1.3).
    MapPrototype,
    /// %MapIteratorPrototype% (Abschnitt 24.1.5.2).
    MapIteratorPrototype,
    /// %Set.prototype% (Abschnitt 24.2.4).
    SetPrototype,
    /// %SetIteratorPrototype% (Abschnitt 24.2.6.2).
    SetIteratorPrototype,
    /// %Array.prototype.values% (Abschnitt 23.1.3.38), auch @@iterator von
    /// Arrays und arguments-Objekten.
    ArrayPrototypeValues,
    /// %Error.prototype% (Abschnitt 20.5.3) mit `name`, `message` und
    /// `toString`.
    ErrorPrototype,
    /// %EvalError.prototype% (Abschnitt 20.5.6.3), wie die fünf folgenden
    /// mit `name` und `message` und %Error.prototype% als Prototyp.
    EvalErrorPrototype,
    /// %RangeError.prototype%
    RangeErrorPrototype,
    /// %ReferenceError.prototype%
    ReferenceErrorPrototype,
    /// %SyntaxError.prototype%
    SyntaxErrorPrototype,
    /// %TypeError.prototype%
    TypeErrorPrototype,
    /// %URIError.prototype%
    URIErrorPrototype,
    /// %AggregateError.prototype% (Abschnitt 20.5.7.3), auch mit `name`
    /// und `message` und %Error.prototype% als Prototyp.
    AggregateErrorPrototype,
    /// %Object% (Abschnitt 20.1.1).
    Object,
    /// %Array% (Abschnitt 23.1.1).
    Array,
    /// %Function% (Abschnitt 20.2.1), bisher ohne CreateDynamicFunction:
    /// Aufruf und `new` enden als nicht unterstützt, bis `eval` kommt.
    Function,
    /// %String% (Abschnitt 22.1.1).
    String,
    /// %Symbol% (Abschnitt 20.4.1) mit den bekannten Symbolen und
    /// `prototype`.
    Symbol,
    /// %Error% (Abschnitt 20.5.1).
    Error,
    /// %EvalError% (Abschnitt 20.5.6.1), wie die fünf folgenden mit %Error%
    /// als Prototyp.
    EvalError,
    /// %RangeError%
    RangeError,
    /// %ReferenceError%
    ReferenceError,
    /// %SyntaxError%
    SyntaxError,
    /// %TypeError%
    TypeError,
    /// %URIError%
    URIError,
    /// %AggregateError% (Abschnitt 20.5.7.1), auch mit %Error% als
    /// Prototyp.
    AggregateError,
    /// %Promise% (Abschnitt 27.5.3).
    Promise,
    /// %Boolean% (Abschnitt 20.3.1).
    Boolean,
    /// %Number% (Abschnitt 21.1.1).
    Number,
    /// %Math% (Abschnitt 21.3).
    Math,
    /// %isFinite% (Abschnitt 19.2.2).
    IsFinite,
    /// %isNaN% (Abschnitt 19.2.3).
    IsNaN,
    /// %parseFloat% (Abschnitt 19.2.4), zugleich `Number.parseFloat`.
    ParseFloat,
    /// %parseInt% (Abschnitt 19.2.5), zugleich `Number.parseInt`.
    ParseInt,
    /// %Iterator% (Abschnitt 27.1.3.1).
    Iterator,
    /// %JSON% (Abschnitt 25.5).
    Json,
    /// %Map% (Abschnitt 24.1.1).
    Map,
    /// %Set% (Abschnitt 24.2.2).
    Set,
}

impl Intrinsic {
    /// Der Prototyp der Fehlerobjekte der Art `kind`.
    pub(crate) fn error_prototype(kind: ErrorKind) -> Intrinsic {
        match kind {
            ErrorKind::RangeError => Intrinsic::RangeErrorPrototype,
            ErrorKind::ReferenceError => Intrinsic::ReferenceErrorPrototype,
            ErrorKind::SyntaxError => Intrinsic::SyntaxErrorPrototype,
            ErrorKind::TypeError => Intrinsic::TypeErrorPrototype,
        }
    }
}

pub(crate) struct Realm {
    intrinsics: BTreeMap<Intrinsic, Persistent>,
    /// [[GlobalObject]], zugleich [[GlobalThisValue]].
    global_object: Persistent,
    /// [[DeclarativeRecord]] der globalen Umgebung.
    declarative: BTreeMap<Vec<u16>, Binding>,
    /// Der Zustand von `Math.random`: Jeder Realm hat seine eigene Folge
    /// (Abschnitt 21.3.2.27).
    random: [u64; 2],
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
    /// Ausführungskontext: CreateIntrinsics (Abschnitt 9.3.2) für die
    /// Intrinsics aus [`Intrinsic`], das globale Objekt als gewöhnliches
    /// Objekt mit %Object.prototype% als Prototyp, SetDefaultGlobalBindings
    /// (Abschnitt 9.3.3) für die Eigenschaften, die es schon gibt, und
    /// zuletzt die Eigenschaften des Hosts
    /// ([`Host::define_global_properties`](crate::host::Host::define_global_properties)).
    /// Scheitert das, gibt es den Realm nicht. Im Belastungsmodus des Heaps
    /// entsteht der Realm ohne Sammlungen, eine Sammlung am Ende verschiebt
    /// ihn dann einmal ([`Heap::pause_stress`](crate::heap::Heap::pause_stress)).
    pub fn create_realm(&mut self) -> Completion<RealmId> {
        self.create_realm_with(true)
    }

    /// Wie [`Agent::create_realm`], aber mit Sammlungen vor jeder
    /// Anforderung, wenn der Heap im Belastungsmodus ist.
    #[cfg(test)]
    pub(crate) fn create_realm_under_stress(&mut self) -> Completion<RealmId> {
        self.create_realm_with(false)
    }

    fn create_realm_with(&mut self, pause_stress: bool) -> Completion<RealmId> {
        let realm = RealmId(self.realms.len());
        let global_object = self.scope(|agent, scope| {
            let undefined = agent.heap().local(scope, Value::UNDEFINED);
            agent.heap_mut().persist(undefined)
        });
        self.realms.push(Realm {
            intrinsics: BTreeMap::new(),
            global_object,
            declarative: BTreeMap::new(),
            random: builtins::random_seed(),
        });
        if pause_stress {
            self.heap_mut().pause_stress();
        }
        let result = self.scope(|agent, scope| initialize_realm(agent, scope, realm));
        if pause_stress {
            self.heap_mut().resume_stress();
        }
        if let Err(abrupt) = result {
            self.discard_realm(realm);
            return Err(abrupt);
        }
        if pause_stress && self.heap().stress() {
            self.heap_mut().collect();
        }
        Ok(realm)
    }

    /// Entfernt den Realm, dessen Aufbau scheiterte. Darin lief kein Code,
    /// also verweist nur er selbst auf seine Objekte.
    fn discard_realm(&mut self, realm: RealmId) {
        if realm.0.checked_add(1) != Some(self.realms.len()) {
            return;
        }
        let Some(old) = self.realms.pop() else {
            return;
        };
        let heap = self.heap_mut();
        for (_, handle) in old.intrinsics {
            heap.release(handle);
        }
        for (_, binding) in old.declarative {
            heap.release(binding.value);
        }
        heap.release(old.global_object);
    }

    /// Setzt ein Intrinsic des Realms, während er entsteht.
    pub(crate) fn set_intrinsic(&mut self, realm: RealmId, intrinsic: Intrinsic, value: Local<'_>) {
        let handle = self.heap_mut().persist(value);
        let old = match self.realms.get_mut(realm.0) {
            Some(entry) => entry.intrinsics.insert(intrinsic, handle),
            None => Some(handle),
        };
        if let Some(old) = old {
            self.heap_mut().release(old);
        }
    }

    /// Setzt [[GlobalObject]] des Realms, während er entsteht.
    fn set_global_object(&mut self, realm: RealmId, global: Local<'_>) {
        let (realms, heap) = self.realms_and_heap();
        if let Some(entry) = realms.get(realm.0) {
            heap.set_persistent(&entry.global_object, global);
        }
    }

    fn realm(&self, realm: RealmId) -> Option<&Realm> {
        self.realms.get(realm.0)
    }

    /// Ein Intrinsic des Realms.
    /// Der Zustand von `Math.random` im Realm `realm`.
    pub(crate) fn random_state(&mut self, realm: RealmId) -> Option<&mut [u64; 2]> {
        self.realms
            .get_mut(realm.index())
            .map(|realm| &mut realm.random)
    }

    pub fn intrinsic<'s>(
        &self,
        scope: &'s Scope,
        realm: RealmId,
        intrinsic: Intrinsic,
    ) -> Local<'s> {
        let value = self
            .realm(realm)
            .and_then(|realm| realm.intrinsics.get(&intrinsic))
            .map_or(Value::UNDEFINED, |handle| {
                self.heap().get_persistent(handle)
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

    /// CanDeclareGlobalFunction (Abschnitt 9.1.1.4.15).
    pub(crate) fn can_declare_global_function(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        key: PropertyKey<'_>,
    ) -> Completion<bool> {
        let global = self.global_object(scope, realm);
        let Some(existing) = object::get_own_property(self, scope, global, key)? else {
            return object::is_extensible(self, global);
        };
        if existing.configurable == Some(true) {
            return Ok(true);
        }
        Ok(existing.is_data()
            && existing.writable == Some(true)
            && existing.enumerable == Some(true))
    }

    /// CreateGlobalFunctionBinding (Abschnitt 9.1.1.4.17).
    pub(crate) fn create_global_function_binding(
        &mut self,
        scope: &Scope,
        realm: RealmId,
        key: PropertyKey<'_>,
        function: Local<'_>,
        deletable: bool,
    ) -> Completion<()> {
        let global = self.global_object(scope, realm);
        let existing = object::get_own_property(self, scope, global, key)?;
        let descriptor = if existing.is_none_or(|property| property.configurable == Some(true)) {
            PropertyDescriptor::data(function, true, true, deletable)
        } else {
            PropertyDescriptor {
                value: Some(function),
                ..PropertyDescriptor::default()
            }
        };
        define_property_or_throw(self, scope, global, key, &descriptor)?;
        set(self, scope, global, key, function, false)
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

/// Der Aufbau eines neuen Realms in [`Agent::create_realm`].
fn initialize_realm(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    create_intrinsics(agent, scope, realm)?;
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let global = ordinary_object_create(agent, scope, Some(prototype))?;
    agent.set_global_object(realm, global);
    set_default_global_bindings(agent, scope, realm, global)?;
    if let Some(host) = agent.host() {
        host.define_global_properties(agent, scope, realm)?;
    }
    Ok(())
}

/// CreateIntrinsics (Abschnitt 9.3.2) für die Intrinsics, die es schon gibt:
/// erst %Object.prototype% und %Function.prototype%, auf denen alle anderen
/// aufbauen, dann die Prototypen der Arrays und der primitiven Werte, dann
/// die eingebauten Objekte aus `builtins`, zuletzt `caller` und `arguments`
/// an %Function.prototype% (Schritt 3).
fn create_intrinsics(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = ordinary_object_create(agent, scope, None)?;
    // %Object.prototype% ist ein Objekt mit unveränderlichem Prototyp
    // (Abschnitt 20.1.3).
    make_prototype_immutable(agent, object);
    agent.set_intrinsic(realm, Intrinsic::ObjectPrototype, object);
    let function =
        builtin_function_with_prototype(agent, scope, realm, &FUNCTION_PROTOTYPE, object)?;
    agent.set_intrinsic(realm, Intrinsic::FunctionPrototype, function);
    let array = array_create(agent, scope, 0, Some(object))?;
    agent.set_intrinsic(realm, Intrinsic::ArrayPrototype, array);
    let heap = agent.heap();
    let (no, zero) = (
        heap.local(scope, Value::FALSE),
        heap.local(scope, Value::int32(0)),
    );
    let boolean = primitive_object_create(agent, scope, no, object)?;
    agent.set_intrinsic(realm, Intrinsic::BooleanPrototype, boolean);
    let number = primitive_object_create(agent, scope, zero, object)?;
    agent.set_intrinsic(realm, Intrinsic::NumberPrototype, number);
    let empty = agent.heap_mut().alloc_string(scope, &[])?;
    let string = string_create(agent, scope, empty, object)?;
    agent.set_intrinsic(realm, Intrinsic::StringPrototype, string);
    let symbol = ordinary_object_create(agent, scope, Some(object))?;
    agent.set_intrinsic(realm, Intrinsic::SymbolPrototype, symbol);
    builtins::create_intrinsics(agent, scope, realm)?;
    add_restricted_function_properties(agent, scope, realm, function)
}

/// SetDefaultGlobalBindings (Abschnitt 9.3.3) für die Werteigenschaften aus
/// Abschnitt 19.1, die Funktionen aus Abschnitt 19.2, soweit es sie gibt,
/// die Konstruktoren aus Abschnitt 19.3, die es schon gibt, und %Math%.
fn set_default_global_bindings(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    global: Local<'_>,
) -> Completion<()> {
    let infinity = agent.heap().local(scope, Value::number(f64::INFINITY));
    let nan = agent.heap().local(scope, Value::number(f64::NAN));
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    // globalThis ist schreibbar und konfigurierbar, die übrigen
    // Werteigenschaften sind keines von beiden. Aufzählbar ist keine.
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
    let intrinsics = [
        ("AggregateError", Intrinsic::AggregateError),
        ("Array", Intrinsic::Array),
        ("Boolean", Intrinsic::Boolean),
        ("Error", Intrinsic::Error),
        ("EvalError", Intrinsic::EvalError),
        ("Function", Intrinsic::Function),
        ("Iterator", Intrinsic::Iterator),
        ("Map", Intrinsic::Map),
        ("Number", Intrinsic::Number),
        ("Object", Intrinsic::Object),
        ("Promise", Intrinsic::Promise),
        ("RangeError", Intrinsic::RangeError),
        ("ReferenceError", Intrinsic::ReferenceError),
        ("Set", Intrinsic::Set),
        ("String", Intrinsic::String),
        ("Symbol", Intrinsic::Symbol),
        ("SyntaxError", Intrinsic::SyntaxError),
        ("TypeError", Intrinsic::TypeError),
        ("URIError", Intrinsic::URIError),
        // Die Funktionen aus Abschnitt 19.2 und %JSON% und %Math% aus
        // Abschnitt 19.4 haben dieselben Attribute.
        ("isFinite", Intrinsic::IsFinite),
        ("isNaN", Intrinsic::IsNaN),
        ("parseFloat", Intrinsic::ParseFloat),
        ("parseInt", Intrinsic::ParseInt),
        ("JSON", Intrinsic::Json),
        ("Math", Intrinsic::Math),
    ];
    for (name, intrinsic) in intrinsics {
        let value = agent.intrinsic(scope, realm, intrinsic);
        let key = agent.key_from_units(scope, &units(name))?;
        let descriptor = PropertyDescriptor::data(value, true, false, true);
        define_property_or_throw(agent, scope, global, key, &descriptor)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
