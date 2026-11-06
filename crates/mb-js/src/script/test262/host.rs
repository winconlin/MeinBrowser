//! Der Host für Test262 mit den Funktionen aus INTERPRETING.md
//! („Host-Defined Functions“): `print` und `$262` mit `createRealm`,
//! `evalScript`, `gc` und `global`. Es fehlen `detachArrayBuffer`,
//! `IsHTMLDDA`, `agent` und `AbstractModuleSource`, denn die Engine hat die
//! Teile dafür noch nicht.
//!
//! Lesarten, wo INTERPRETING.md nichts festlegt: Die Eigenschaften von
//! `$262` haben dieselben Attribute wie `print` und `$262` selbst
//! (beschreibbar, konfigurierbar, nicht aufzählbar). `evalScript` und
//! `print` wandeln ihr erstes Argument mit ToString. `length` ist die Zahl
//! der Argumente, die die Funktion liest.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, define_builtin_function};
use crate::conversion::to_string;
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::host::Host;
use crate::object::ordinary_object_create;
use crate::operations::{define_property_or_throw, get};
use crate::realm::{Intrinsic, RealmId};
use crate::script::ScriptError;
use crate::value::{Kind, Value};

/// Der Host eines Agents im Läufer. Er merkt sich, was `print` ausgab.
#[derive(Default)]
pub(super) struct Test262Host {
    printed: RefCell<Vec<Vec<u16>>>,
}

impl Test262Host {
    /// Die Ausgaben von `print` in ihrer Reihenfolge.
    pub(super) fn printed(&self) -> Vec<Vec<u16>> {
        self.printed.borrow().clone()
    }
}

static PRINT: Builtin = Builtin {
    name: "print",
    length: 1,
    constructor: false,
    behaviour: print,
};

static FUNCTIONS: [Builtin; 3] = [
    Builtin {
        name: "createRealm",
        length: 0,
        constructor: false,
        behaviour: create_realm,
    },
    Builtin {
        name: "evalScript",
        length: 1,
        constructor: false,
        behaviour: eval_script,
    },
    Builtin {
        name: "gc",
        length: 0,
        constructor: false,
        behaviour: gc,
    },
];

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl Host for Test262Host {
    fn define_global_properties(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        realm: RealmId,
    ) -> Completion<()> {
        let global = agent.global_object(scope, realm);
        let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
        let object = ordinary_object_create(agent, scope, Some(prototype))?;
        for builtin in &FUNCTIONS {
            define_builtin_function(agent, scope, realm, object, builtin)?;
        }
        define(agent, scope, object, "global", global)?;
        define_builtin_function(agent, scope, realm, global, &PRINT)?;
        define(agent, scope, global, "$262", object)
    }
}

/// Eine beschreibbare, konfigurierbare, nicht aufzählbare Eigenschaft.
fn define(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: &str,
    value: Local<'_>,
) -> Completion<()> {
    let key = agent.key_from_units(scope, &units(name))?;
    let descriptor = PropertyDescriptor::data(value, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// Die Codeeinheiten von ToString(`value`).
fn text(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<Vec<u16>> {
    let string = to_string(agent, scope, value)?;
    let heap = agent.heap();
    Ok(match heap.get(string).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => Vec::new(),
    })
}

/// `print(value)`: hält ToString(`value`) für den Läufer fest.
fn print<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = call.argument(agent, scope, 0);
    let text = text(agent, scope, value)?;
    let host = agent.host().and_then(|host| -> Option<Rc<Test262Host>> {
        let host: Rc<dyn Any> = host;
        host.downcast::<Test262Host>().ok()
    });
    let Some(host) = host else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    // ToString ist schon gelaufen; während der Ausleihe läuft nichts mehr
    // (RS-10).
    host.printed.borrow_mut().push(text);
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// `$262.createRealm()`: ein neuer Realm mit eigenem `$262`, das die
/// Funktion zurückgibt.
fn create_realm<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    _: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = agent.create_realm()?;
    let global = agent.global_object(scope, realm);
    let key = agent.key_from_units(scope, &units("$262"))?;
    get(agent, scope, global, key)
}

/// `$262.evalScript(source)`: ParseScript und ScriptEvaluation im Realm der
/// Funktion, dem aktuellen Realm. Ein früher Fehler wird als SyntaxError
/// dieses Realms geworfen. Kann der Übersetzer etwas nicht, endet der ganze
/// Lauf mit [`Abrupt::Unsupported`]; die Lage bezieht sich dann auf den
/// Quelltext von `evalScript`.
fn eval_script<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = call.argument(agent, scope, 0);
    let source = text(agent, scope, value)?;
    match agent.evaluate_script(scope, call.realm, &source) {
        Ok(result) => Ok(result),
        Err(ScriptError::Syntax(_)) => Err(agent.throw_error(ErrorKind::SyntaxError)),
        Err(ScriptError::Unsupported(position)) => Err(Abrupt::Unsupported(position)),
        Err(ScriptError::Abrupt(abrupt)) => Err(abrupt),
    }
}

/// `$262.gc()`: eine Sammlung des Heaps.
fn gc<'s>(agent: &mut Agent, scope: &'s Scope, _: &BuiltinCall<'_, '_>) -> Completion<Local<'s>> {
    agent.heap_mut().collect();
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}
