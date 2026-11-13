//! Die Funktionen des globalen Objekts aus Abschnitt 19.2, soweit sie zu
//! Zahlen gehören: %isFinite%, %isNaN%, %parseFloat% und %parseInt%.
//! `Number.parseFloat` und `Number.parseInt` sind dieselben
//! Funktionsobjekte (Abschnitte 21.1.2.12 und 21.1.2.13). `eval` kommt mit
//! Schritt 9, die Funktionen für URIs mit der Welle für Zeichenketten.

use crate::agent::{Agent, Completion};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::{parse_float, parse_int, to_int32, to_number, to_string};
use crate::heap::{Local, Scope};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

static FUNCTIONS: [(Builtin, Intrinsic); 4] = [
    (
        Builtin {
            name: "isFinite",
            length: 1,
            constructor: false,
            behaviour: is_finite,
        },
        Intrinsic::IsFinite,
    ),
    (
        Builtin {
            name: "isNaN",
            length: 1,
            constructor: false,
            behaviour: is_nan,
        },
        Intrinsic::IsNaN,
    ),
    (
        Builtin {
            name: "parseFloat",
            length: 1,
            constructor: false,
            behaviour: parse_float_function,
        },
        Intrinsic::ParseFloat,
    ),
    (
        Builtin {
            name: "parseInt",
            length: 2,
            constructor: false,
            behaviour: parse_int_function,
        },
        Intrinsic::ParseInt,
    ),
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    for (builtin, intrinsic) in &FUNCTIONS {
        let function = create_builtin_function(agent, scope, realm, builtin)?;
        agent.set_intrinsic(realm, *intrinsic, function);
    }
    Ok(())
}

fn number<'s>(agent: &Agent, scope: &'s Scope, value: f64) -> Local<'s> {
    agent.heap().local(scope, Value::number(value))
}

fn boolean<'s>(agent: &Agent, scope: &'s Scope, value: bool) -> Local<'s> {
    agent.heap().local(scope, Value::boolean(value))
}

/// Die Codeeinheiten von ToString(`value`).
fn string_units(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<Vec<u16>> {
    let text = to_string(agent, scope, value)?;
    let heap = agent.heap();
    Ok(match heap.get(text).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => Vec::new(),
    })
}

/// isFinite ( number ) (Abschnitt 19.2.2).
fn is_finite<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let number = to_number(agent, scope, value)?;
    Ok(boolean(agent, scope, number.is_finite()))
}

/// isNaN ( number ) (Abschnitt 19.2.3).
fn is_nan<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let number = to_number(agent, scope, value)?;
    Ok(boolean(agent, scope, number.is_nan()))
}

/// parseFloat ( string ) (Abschnitt 19.2.4).
fn parse_float_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let units = string_units(agent, scope, value)?;
    Ok(number(agent, scope, parse_float(&units)))
}

/// parseInt ( string, radix ) (Abschnitt 19.2.5): erst ToString, dann
/// ToInt32 für die Basis.
fn parse_int_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let units = string_units(agent, scope, value)?;
    let radix = arguments.argument(agent, scope, 1);
    let radix = to_int32(agent, scope, radix)?;
    Ok(number(agent, scope, parse_int(&units, radix)))
}
