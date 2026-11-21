//! %WeakRef% mit %WeakRef.prototype% (Abschnitt 26.1) und
//! %FinalizationRegistry% mit %FinalizationRegistry.prototype% (Abschnitt
//! 26.2), dazu CleanupFinalizationRegistry (Abschnitt 9.12) für die
//! Aufräum-Jobs, die der Agent ausführt.
//!
//! Eine WeakRef hält [[WeakRefTarget]] in einer schwachen Zelle mit einem
//! Feld; leert der Collector es, ist das Ziel ~empty~. Die Records in
//! [[Cells]] einer FinalizationRegistry liegen in zwei Zellen. Die schwache
//! hält im ersten Feld die Registry selbst und danach je Record
//! [[WeakRefTarget]] und [[UnregisterToken]], beide schwach; die zweite hält
//! je Record [[HeldValue]], stark, solange der Record besteht. Ein Ziel, das
//! der Collector leert, wird *undefined*, und die Registry kommt in die
//! Liste der Aufräum-Jobs des Heaps. Ein entfernter Record hat die Lücke
//! als Ziel. Eine fehlende Abmeldemarke (~empty~) ist *undefined*, so wie
//! eine, die der Collector geleert hat: Beide kann `unregister` nicht mehr
//! treffen, denn es nimmt nur Werte, die CanBeHeldWeakly erfüllen. Sind
//! alle Plätze belegt, ziehen die Records ohne die entfernten in neue
//! Zellen um, die doppelt so viele fassen.
//!
//! `unregister` und die Aufräum-Jobs gehen wie die Spezifikation über alle
//! Records und bezahlen dafür einen Arbeitsschritt je [`RECORDS_PER_STEP`]
//! Records.

use super::{define_functions, define_to_string_tag, link_constructor};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::compare::same_value;
use crate::heap::{CellKind, Local, ObjectClass, OutOfMemory, Scope};
use crate::object::{INLINE, get_prototype_from_constructor, ordinary_object_create};
use crate::operations::{call, can_be_held_weakly, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Ref, Value};

/// So viele Records bezahlt ein Arbeitsschritt, wenn `unregister` oder ein
/// Aufräum-Job über [[Cells]] geht.
const RECORDS_PER_STEP: usize = 16;

/// Das interne Feld einer WeakRef: die schwache Zelle mit dem Ziel.
const TARGET: usize = 0;

/// Die internen Felder einer FinalizationRegistry. [[Realm]] bewahrt sie
/// nur auf: Der Aufräum-Job ruft den Callback in dessen eigenem Realm, und
/// erst ein Host, der Fehler aus dem Job meldet, braucht ihn.
const REALM: usize = 0;
const CALLBACK: usize = 1;
/// Die schwache Zelle: im Feld 0 die Registry, im Feld `1 + 2 * i` das
/// Ziel des Records `i`, im Feld `2 + 2 * i` seine Abmeldemarke.
const TARGETS: usize = 2;
/// Die Zelle mit dem gehaltenen Wert des Records `i` im Feld `i`.
const HELD: usize = 3;
/// Wie viele Records belegt sind, entfernte mitgezählt.
const USED: usize = 4;

/// So viele Records fassen die Zellen einer Registry mindestens.
const MIN_CAPACITY: usize = 4;

static WEAK_REF_CONSTRUCTOR: Builtin = Builtin {
    name: "WeakRef",
    length: 1,
    constructor: true,
    behaviour: weak_ref,
};

static WEAK_REF_FUNCTIONS: [Builtin; 1] = [method("deref", 0, deref)];

static REGISTRY_CONSTRUCTOR: Builtin = Builtin {
    name: "FinalizationRegistry",
    length: 1,
    constructor: true,
    behaviour: finalization_registry,
};

static REGISTRY_FUNCTIONS: [Builtin; 2] = [
    method("register", 2, register),
    method("unregister", 1, unregister),
];

const fn method(name: &'static str, length: u32, behaviour: crate::builtin::Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    for (constructor, functions, tag, intrinsics) in [
        (
            &WEAK_REF_CONSTRUCTOR,
            &WEAK_REF_FUNCTIONS[..],
            "WeakRef",
            (Intrinsic::WeakRef, Intrinsic::WeakRefPrototype),
        ),
        (
            &REGISTRY_CONSTRUCTOR,
            &REGISTRY_FUNCTIONS[..],
            "FinalizationRegistry",
            (
                Intrinsic::FinalizationRegistry,
                Intrinsic::FinalizationRegistryPrototype,
            ),
        ),
    ] {
        let prototype = ordinary_object_create(agent, scope, Some(object))?;
        let function = create_builtin_function(agent, scope, realm, constructor)?;
        link_constructor(agent, scope, function, prototype)?;
        define_functions(agent, scope, realm, prototype, functions)?;
        define_to_string_tag(agent, scope, prototype, tag)?;
        agent.set_intrinsic(realm, intrinsics.0, function);
        agent.set_intrinsic(realm, intrinsics.1, prototype);
    }
    Ok(())
}

fn internal<'h>(value: Option<Value<'h>>) -> Option<Ref<'h>> {
    match value?.kind() {
        Kind::Internal(cell) => Some(cell),
        _ => None,
    }
}

fn index_of(value: Option<Value<'_>>) -> usize {
    value
        .and_then(Value::as_int32)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

/// Eine Zahl als Wert. Ein Record braucht mehr als ein Wort, also bleibt
/// jede Zahl von Records unter `i32::MAX`.
fn int(value: usize) -> Value<'static> {
    Value::int32(i32::try_from(value).unwrap_or(i32::MAX))
}

/// Ein Objekt der Art `class` mit dem Prototyp aus NewTarget, für
/// OrdinaryCreateFromConstructor.
fn create_object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    new_target: Local<'_>,
    fallback: Intrinsic,
    class: ObjectClass,
) -> Completion<Local<'s>> {
    let prototype =
        get_prototype_from_constructor(agent, scope, new_target, fallback, arguments.realm)?;
    let shape = agent.empty_shape(scope);
    Ok(agent
        .heap_mut()
        .alloc_object(scope, class, INLINE, shape, Some(prototype))?)
}

// WeakRef

/// WeakRef ( target ) (Abschnitt 26.1.1.1).
fn weak_ref<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let target = arguments.argument(agent, scope, 0);
    if !can_be_held_weakly(agent.heap(), agent.heap().get(target)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let object = create_object(
        agent,
        scope,
        arguments,
        new_target,
        Intrinsic::WeakRefPrototype,
        ObjectClass::WeakRef,
    )?;
    let cell = agent.heap_mut().alloc_cell(scope, CellKind::Weak, 1)?;
    agent.heap_mut().add_to_kept_objects(target);
    let heap = agent.heap();
    if let (Kind::Object(reference), Some(weak)) =
        (heap.get(object).kind(), internal(Some(heap.get(cell))))
    {
        heap.set_field(weak, 0, heap.get(target));
        heap.set_weak_ref_field(reference, TARGET, heap.get(cell));
    }
    Ok(object)
}

/// WeakRef.prototype.deref ( ) (Abschnitt 26.1.3.2) mit RequireInternalSlot
/// und WeakRefDeref (Abschnitt 26.1.4.1).
fn deref<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let target = match heap.get(arguments.this).kind() {
        Kind::Object(object) if heap.object_class(object) == Some(ObjectClass::WeakRef) => {
            internal(heap.weak_ref_field(object, TARGET))
                .and_then(|weak| heap.field(weak, 0))
                .unwrap_or(Value::UNDEFINED)
        }
        _ => return Err(agent.throw_error(ErrorKind::TypeError)),
    };
    let target = heap.local(scope, target);
    if !agent.heap().get(target).is_undefined() {
        agent.heap_mut().add_to_kept_objects(target);
    }
    Ok(target)
}

// FinalizationRegistry

/// FinalizationRegistry ( cleanupCallback ) (Abschnitt 26.2.1.1).
/// HostMakeJobCallback gibt die Funktion selbst zurück.
fn finalization_registry<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let Some(new_target) = arguments.new_target else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let callback = arguments.argument(agent, scope, 0);
    if !is_callable(agent.heap(), agent.heap().get(callback)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let object = create_object(
        agent,
        scope,
        arguments,
        new_target,
        Intrinsic::FinalizationRegistryPrototype,
        ObjectClass::FinalizationRegistry,
    )?;
    // So viele Realms passen nicht in den Speicher.
    let realm = i32::try_from(arguments.realm.index())
        .map_err(|_| agent.throw_error(ErrorKind::RangeError))?;
    let heap = agent.heap();
    if let Kind::Object(reference) = heap.get(object).kind() {
        heap.set_registry_field(reference, REALM, Value::int32(realm));
        heap.set_registry_field(reference, CALLBACK, heap.get(callback));
        heap.set_registry_field(reference, USED, Value::int32(0));
    }
    Ok(object)
}

/// RequireInternalSlot(*this*, [[Cells]]): die Registry.
fn this_registry<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    match heap.get(arguments.this).kind() {
        Kind::Object(object)
            if heap.object_class(object) == Some(ObjectClass::FinalizationRegistry) =>
        {
            Ok(heap.local(scope, heap.get(arguments.this)))
        }
        _ => Err(agent.throw_error(ErrorKind::TypeError)),
    }
}

fn registry_ref<'h>(agent: &'h Agent, registry: Local<'_>) -> Option<Ref<'h>> {
    match agent.heap().get(registry).kind() {
        Kind::Object(object) => Some(object),
        _ => None,
    }
}

/// Die beiden Zellen der Records und wie viele belegt sind.
fn records<'h>(agent: &'h Agent, registry: Local<'_>) -> Option<(Ref<'h>, Ref<'h>, usize)> {
    let heap = agent.heap();
    let object = registry_ref(agent, registry)?;
    let targets = internal(heap.registry_field(object, TARGETS))?;
    let held = internal(heap.registry_field(object, HELD))?;
    Some((targets, held, index_of(heap.registry_field(object, USED))))
}

/// FinalizationRegistry.prototype.register ( target, heldValue [ ,
/// unregisterToken ] ) (Abschnitt 26.2.3.2).
fn register<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let registry = this_registry(agent, scope, arguments)?;
    let target = arguments.argument(agent, scope, 0);
    let held = arguments.argument(agent, scope, 1);
    let token = arguments.argument(agent, scope, 2);
    let heap = agent.heap();
    if !can_be_held_weakly(heap, heap.get(target)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if same_value(heap, heap.get(target), heap.get(held)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    if !can_be_held_weakly(heap, heap.get(token)) && !heap.get(token).is_undefined() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let capacity = records(agent, registry).map_or(0, |(targets, _, _)| {
        agent.heap().field_count(targets).saturating_sub(1) / 2
    });
    let used = records(agent, registry).map_or(0, |(_, _, used)| used);
    if used >= capacity {
        grow(agent, scope, registry)?;
    }
    let Some((targets, cells, used)) = records(agent, registry) else {
        return Err(OutOfMemory.into());
    };
    let heap = agent.heap();
    let base = 1 + used * 2;
    heap.set_field(targets, base, heap.get(target));
    heap.set_field(targets, base + 1, heap.get(token));
    heap.set_field(cells, used, heap.get(held));
    if let Some(object) = registry_ref(agent, registry) {
        heap.set_registry_field(object, USED, int(used + 1));
    }
    Ok(heap.local(scope, Value::UNDEFINED))
}

/// Zieht die Records ohne die entfernten in neue Zellen um, die doppelt so
/// viele fassen, wie danach belegt sind, mindestens [`MIN_CAPACITY`].
fn grow(agent: &mut Agent, scope: &Scope, registry: Local<'_>) -> Completion<()> {
    let live = records(agent, registry).map_or(0, |(targets, _, used)| {
        let heap = agent.heap();
        (0..used)
            .filter(|&index| {
                heap.field(targets, 1 + index * 2)
                    .is_some_and(|target| !target.is_hole())
            })
            .count()
    });
    let capacity = live
        .checked_add(1)
        .and_then(|records| records.checked_mul(2))
        .ok_or(OutOfMemory)?
        .max(MIN_CAPACITY);
    let length = capacity
        .checked_mul(2)
        .and_then(|length| length.checked_add(1))
        .filter(|&length| i32::try_from(length).is_ok())
        .ok_or(OutOfMemory)?;
    let new_targets = agent.heap_mut().alloc_finalizer_cell(scope, length)?;
    let new_held = agent.heap_mut().alloc_slots(scope, capacity)?;
    // Erst nach den Anforderungen lesen: Der Collector kann verschoben und
    // Ziele geleert haben. Geleerte Records ziehen mit um, ihr Aufräum-Job
    // steht schon in der Liste.
    let heap = agent.heap();
    let (Some(object), Some(to_targets), Some(to_held)) = (
        registry_ref(agent, registry),
        internal(Some(heap.get(new_targets))),
        internal(Some(heap.get(new_held))),
    ) else {
        return Err(OutOfMemory.into());
    };
    heap.set_field(to_targets, 0, heap.get(registry));
    let mut count = 0;
    if let Some((targets, held, used)) = records(agent, registry) {
        for index in 0..used {
            let target = heap.field(targets, 1 + index * 2).unwrap_or(Value::HOLE);
            if target.is_hole() {
                continue;
            }
            let token = heap
                .field(targets, 2 + index * 2)
                .unwrap_or(Value::UNDEFINED);
            let value = heap.field(held, index).unwrap_or(Value::UNDEFINED);
            heap.set_field(to_targets, 1 + count * 2, target);
            heap.set_field(to_targets, 2 + count * 2, token);
            heap.set_field(to_held, count, value);
            count += 1;
        }
    }
    heap.set_registry_field(object, TARGETS, heap.get(new_targets));
    heap.set_registry_field(object, HELD, heap.get(new_held));
    heap.set_registry_field(object, USED, int(count));
    Ok(())
}

/// Bezahlt das Durchgehen von `records` Records.
fn pay(agent: &Agent, records: usize) -> Completion<()> {
    let steps = records.div_ceil(RECORDS_PER_STEP).max(1);
    agent.spend_execution_steps(u64::try_from(steps).unwrap_or(u64::MAX))
}

/// FinalizationRegistry.prototype.unregister ( unregisterToken ) (Abschnitt
/// 26.2.3.3).
fn unregister<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let registry = this_registry(agent, scope, arguments)?;
    let token = arguments.argument(agent, scope, 0);
    if !can_be_held_weakly(agent.heap(), agent.heap().get(token)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let used = records(agent, registry).map_or(0, |(_, _, used)| used);
    pay(agent, used)?;
    let mut removed = false;
    if let Some((targets, held, used)) = records(agent, registry) {
        let heap = agent.heap();
        let token = heap.get(token);
        for index in 0..used {
            let base = 1 + index * 2;
            let Some(stored) = heap.field(targets, base + 1) else {
                break;
            };
            // Auch ein Record, dessen Ziel geleert ist, geht.
            let present = heap
                .field(targets, base)
                .is_some_and(|target| !target.is_hole());
            if present && !stored.is_undefined() && same_value(heap, stored, token) {
                heap.set_field(targets, base, Value::HOLE);
                heap.set_field(targets, base + 1, Value::UNDEFINED);
                heap.set_field(held, index, Value::UNDEFINED);
                removed = true;
            }
        }
    }
    Ok(agent.heap().local(scope, Value::boolean(removed)))
}

/// CleanupFinalizationRegistry (Abschnitt 9.12) mit HostCallJobCallback:
/// Ruft den Callback für jeden Record, dessen Ziel der Collector geleert
/// hat, mit dessen gehaltenem Wert und entfernt den Record vorher. Ein
/// Aufruf kann Records anmelden und abmelden, die Zellen ersetzen und
/// sammeln lassen; siehe [`take_emptied`].
pub(crate) fn cleanup_finalization_registry(
    agent: &mut Agent,
    scope: &Scope,
    registry: Local<'_>,
) -> Completion<()> {
    let heap = agent.heap();
    let Some(object) = registry_ref(agent, registry) else {
        return Ok(());
    };
    if heap.object_class(object) != Some(ObjectClass::FinalizationRegistry) {
        return Ok(());
    }
    let field = |index| {
        heap.registry_field(object, index)
            .unwrap_or(Value::UNDEFINED)
    };
    let callback = heap.local(scope, field(CALLBACK));
    let cells = heap.local(scope, field(TARGETS));
    let mut search = Search {
        next: 0,
        collections: heap.collections(),
    };
    loop {
        let finished = agent.scope(|agent, inner| -> Completion<bool> {
            let Some(value) = take_emptied(agent, inner, registry, cells, &mut search)? else {
                return Ok(true);
            };
            let undefined = agent.heap().local(inner, Value::UNDEFINED);
            call(agent, inner, callback, undefined, &[value])?;
            Ok(false)
        })?;
        if finished {
            return Ok(());
        }
    }
}

/// Wie weit ein Aufräum-Job die Records durchgegangen ist.
struct Search {
    /// Der nächste Record.
    next: usize,
    /// Wie oft der Heap beim Beginn des Durchgangs gesammelt hatte.
    collections: u64,
}

/// Sucht ab dem nächsten Record einen, dessen Ziel der Collector geleert
/// hat, entfernt ihn und gibt seinen gehaltenen Wert. Hat die Registry neue
/// Zellen, beginnt die Suche vorn. Hat der Heap während des Durchgangs
/// gesammelt, beginnt am Ende ein neuer: Die Spezifikation ruft den
/// Callback, solange es solche Records gibt.
fn take_emptied<'s>(
    agent: &Agent,
    scope: &'s Scope,
    registry: Local<'_>,
    cells: Local<'_>,
    search: &mut Search,
) -> Completion<Option<Local<'s>>> {
    let heap = agent.heap();
    let Some((targets, held, used)) = records(agent, registry) else {
        return Ok(None);
    };
    let current = Value::internal(targets);
    if heap.get(cells).bits() != current.bits() {
        heap.set(cells, current);
        search.next = 0;
    }
    let mut scanned = 0;
    loop {
        while search.next < used {
            let index = search.next;
            search.next += 1;
            scanned += 1;
            let base = 1 + index * 2;
            if heap.field(targets, base).is_some_and(Value::is_undefined) {
                let value = heap.field(held, index).unwrap_or(Value::UNDEFINED);
                heap.set_field(targets, base, Value::HOLE);
                heap.set_field(targets, base + 1, Value::UNDEFINED);
                heap.set_field(held, index, Value::UNDEFINED);
                pay(agent, scanned)?;
                return Ok(Some(heap.local(scope, value)));
            }
        }
        if heap.collections() == search.collections {
            break;
        }
        search.collections = heap.collections();
        search.next = 0;
    }
    pay(agent, scanned)?;
    Ok(None)
}
