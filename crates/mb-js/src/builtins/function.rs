//! %Function.prototype% (Abschnitt 20.2.3) mit `call`. Den Konstruktor
//! %Function% gibt es erst mit `eval`, denn er übersetzt Quelltext zur
//! Laufzeit (CreateDynamicFunction, Abschnitt 20.2.1.1.1).

use super::define_functions;
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall};
use crate::heap::{Local, Scope};
use crate::operations::{call, is_callable};
use crate::realm::{Intrinsic, RealmId};
use crate::value::Value;

/// %Function.prototype% selbst, eine eingebaute Funktion ohne [[Construct]]
/// mit `length` 0 und dem leeren Namen.
pub(crate) static PROTOTYPE: Builtin = Builtin {
    name: "",
    length: 0,
    constructor: false,
    behaviour: function_prototype,
};

static PROTOTYPE_FUNCTIONS: [Builtin; 1] = [Builtin {
    name: "call",
    length: 1,
    constructor: false,
    behaviour: function_prototype_call,
}];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::FunctionPrototype);
    define_functions(agent, scope, realm, prototype, &PROTOTYPE_FUNCTIONS)
}

/// %Function.prototype% nimmt beliebige Argumente und gibt *undefined*
/// zurück (Abschnitt 20.2.3).
fn function_prototype<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    _: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Function.prototype.call (Abschnitt 20.2.3.3).
///
/// Abweichung: PrepareForTailCall gibt den Rahmen des Aufrufers nicht frei.
/// Steht `f.call(…)` in Endposition, baut erst das Return nach dem Aufruf
/// den Rahmen ab, und der Aufruf von `f` beginnt einen neuen Lauf. Sichtbar
/// wird das nur an einem RangeError bei großer Tiefe.
fn function_prototype_call<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let function = arguments.this;
    if !is_callable(agent.heap(), agent.heap().get(function)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let this = arguments.argument(agent, scope, 0);
    let rest = arguments.arguments.get(1..).unwrap_or_default();
    call(agent, scope, function, this, rest)
}
