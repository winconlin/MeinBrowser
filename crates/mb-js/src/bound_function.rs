//! Gebundene Funktionen (Abschnitt 10.4.1): Exoten, die eine andere
//! Funktion mit festem `this` und vorangestellten Argumenten aufrufen.
//!
//! Eine gebundene Funktion ist ein Objekt der Art
//! [`ObjectClass::BoundFunction`] mit drei internen Feldern:
//! [[BoundTargetFunction]], [[BoundThis]] und [[BoundArguments]] als
//! interne Zelle, *undefined* ohne Argumente. Ihre übrigen internen
//! Methoden sind die gewöhnlichen. [[Call]] und [[Construct]] stehen im
//! Interpreter: Er löst eine Kette gebundener Funktionen mit
//! [`unwrap_call`] und [`unwrap_construct`] in einer Schleife bis zur
//! Zielfunktion auf (RS-8) und ruft die dann wie jede andere Funktion auf.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion};
use crate::compare::same_value;
use crate::heap::{Heap, Local, ObjectClass, Scope};
use crate::object::{INLINE, get_prototype_of};
use crate::value::{Kind, Value};

/// Das interne Feld mit [[BoundTargetFunction]].
const TARGET: usize = 0;
/// Das interne Feld mit [[BoundThis]].
const THIS: usize = 1;
/// Das interne Feld mit [[BoundArguments]].
const ARGUMENTS: usize = 2;

/// Ob `value` eine gebundene Funktion ist.
pub(crate) fn is_bound_function(heap: &Heap, value: Value<'_>) -> bool {
    match value.kind() {
        Kind::Object(object) => heap.object_class(object) == Some(ObjectClass::BoundFunction),
        _ => false,
    }
}

/// [[BoundTargetFunction]] der gebundenen Funktion `value`, `None` für
/// andere Werte.
pub(crate) fn bound_target<'h>(heap: &'h Heap, value: Value<'_>) -> Option<Value<'h>> {
    let Kind::Object(object) = value.kind() else {
        return None;
    };
    heap.bound_function_field(object, TARGET)
}

/// Die Funktion am Ende einer Kette gebundener Funktionen, die mit `value`
/// beginnt; `None`, wenn `value` keine gebundene Funktion ist.
pub(crate) fn innermost_target<'h>(heap: &'h Heap, value: Value<'_>) -> Option<Value<'h>> {
    let mut current = bound_target(heap, value)?;
    while let Some(next) = bound_target(heap, current) {
        current = next;
    }
    Some(current)
}

/// BoundFunctionCreate (Abschnitt 10.4.1.3): eine gebundene Funktion um
/// `target` mit dem Prototyp von `target`. Ob sie [[Construct]] hat,
/// entscheidet `target` bei jedem Aufruf von IsConstructor.
pub(crate) fn bound_function_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    target: Local<'_>,
    this: Local<'_>,
    arguments: &[Local<'_>],
) -> Completion<Local<'s>> {
    let prototype = get_prototype_of(agent, scope, target)?;
    let shape = agent.empty_shape(scope);
    let function = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::BoundFunction,
        INLINE,
        shape,
        prototype,
    )?;
    let bound_arguments = if arguments.is_empty() {
        None
    } else {
        let cell = agent.heap_mut().alloc_slots(scope, arguments.len())?;
        let heap = agent.heap();
        if let Kind::Internal(slots) = heap.get(cell).kind() {
            for (index, &argument) in arguments.iter().enumerate() {
                heap.set_slot(slots, index, heap.get(argument));
            }
        }
        Some(cell)
    };
    let heap = agent.heap();
    if let Kind::Object(object) = heap.get(function).kind() {
        heap.set_bound_function_field(object, TARGET, heap.get(target));
        heap.set_bound_function_field(object, THIS, heap.get(this));
        if let Some(cell) = bound_arguments {
            heap.set_bound_function_field(object, ARGUMENTS, heap.get(cell));
        }
    }
    Ok(function)
}

/// Die Glieder der Kette gebundener Funktionen ab `function`, außen
/// beginnend, und die Zielfunktion am Ende.
fn chain<'s>(agent: &Agent, scope: &'s Scope, function: Local<'_>) -> (Vec<Local<'s>>, Local<'s>) {
    let heap = agent.heap();
    let mut links = Vec::new();
    let mut current = heap.get(function);
    while let Some(target) = bound_target(heap, current) {
        links.push(heap.local(scope, current));
        current = target;
    }
    (links, heap.local(scope, current))
}

/// Hängt die [[BoundArguments]] von `link` an `list`.
fn push_bound_arguments<'s>(
    agent: &Agent,
    scope: &'s Scope,
    link: Local<'_>,
    list: &mut Vec<Local<'s>>,
) {
    let heap = agent.heap();
    let Kind::Object(object) = heap.get(link).kind() else {
        return;
    };
    let Some(Kind::Internal(cell)) = heap
        .bound_function_field(object, ARGUMENTS)
        .map(Value::kind)
    else {
        return;
    };
    for index in 0..heap.slot_count(cell) {
        let value = heap.slot(cell, index).unwrap_or(Value::UNDEFINED);
        list.push(heap.local(scope, value));
    }
}

/// [[Call]] einer gebundenen Funktion (Abschnitt 10.4.1.1) für die ganze
/// Kette: die Zielfunktion, ihr `this` aus dem innersten Glied und die
/// Argumente, die [[BoundArguments]] von innen nach außen vor `arguments`.
pub(crate) fn unwrap_call<'s>(
    agent: &Agent,
    scope: &'s Scope,
    function: Local<'_>,
    this: Local<'_>,
    arguments: &[Local<'_>],
) -> (Local<'s>, Local<'s>, Vec<Local<'s>>) {
    let (links, target) = chain(agent, scope, function);
    let heap = agent.heap();
    let this = match links.last() {
        Some(&inner) => match heap.get(inner).kind() {
            Kind::Object(object) => heap.local(
                scope,
                heap.bound_function_field(object, THIS)
                    .unwrap_or(Value::UNDEFINED),
            ),
            _ => heap.local(scope, Value::UNDEFINED),
        },
        None => heap.local(scope, heap.get(this)),
    };
    let mut list = Vec::new();
    for &link in links.iter().rev() {
        push_bound_arguments(agent, scope, link, &mut list);
    }
    list.extend(
        arguments
            .iter()
            .map(|&argument| heap.local(scope, heap.get(argument))),
    );
    (target, this, list)
}

/// [[Construct]] einer gebundenen Funktion (Abschnitt 10.4.1.2) für die
/// ganze Kette: die Zielfunktion, die Argumente wie bei [`unwrap_call`] und
/// NewTarget, das für jedes Glied, das es selbst ist, zu dessen Ziel wird.
pub(crate) fn unwrap_construct<'s>(
    agent: &Agent,
    scope: &'s Scope,
    constructor: Local<'_>,
    arguments: &[Local<'_>],
    new_target: Local<'_>,
) -> (Local<'s>, Vec<Local<'s>>, Local<'s>) {
    let (links, target) = chain(agent, scope, constructor);
    let heap = agent.heap();
    let new_target = heap.local(scope, heap.get(new_target));
    for &link in &links {
        if same_value(heap, heap.get(link), heap.get(new_target)) {
            let next = bound_target(heap, heap.get(link)).unwrap_or(Value::UNDEFINED);
            heap.set(new_target, next);
        }
    }
    let mut list = Vec::new();
    for &link in links.iter().rev() {
        push_bound_arguments(agent, scope, link, &mut list);
    }
    list.extend(
        arguments
            .iter()
            .map(|&argument| heap.local(scope, heap.get(argument))),
    );
    (target, list, new_target)
}
