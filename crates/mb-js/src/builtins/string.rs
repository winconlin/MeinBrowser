//! %String% (Abschnitt 22.1.1). Die Methoden von %String.prototype% kommen
//! mit der Welle für Zeichenketten.

use super::{link_constructor, units};
use crate::agent::{Agent, Completion};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::to_string;
use crate::heap::{Local, Scope};
use crate::key::symbol_description;
use crate::object::{get_prototype_from_constructor, string_create};
use crate::realm::{Intrinsic, RealmId};
use crate::value::Kind;

static CONSTRUCTOR: Builtin = Builtin {
    name: "String",
    length: 1,
    constructor: true,
    behaviour: string,
};

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::StringPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    agent.set_intrinsic(realm, Intrinsic::String, constructor);
    Ok(())
}

/// String ( value ) (Abschnitt 22.1.1.1).
fn string<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let text = match arguments.arguments.first() {
        None => agent.heap_mut().alloc_string(scope, &[])?,
        Some(&value) => {
            if arguments.new_target.is_none() {
                let heap = agent.heap();
                if let Kind::Symbol(symbol) = heap.get(value).kind() {
                    // SymbolDescriptiveString (Abschnitt 20.4.3.3.1).
                    let mut text = units("Symbol(");
                    if let Kind::String(description) = symbol_description(heap, symbol).kind() {
                        text.extend(heap.string_units(description));
                    }
                    text.push(u16::from(b')'));
                    return Ok(agent.heap_mut().alloc_string(scope, &text)?);
                }
            }
            to_string(agent, scope, value)?
        }
    };
    let Some(new_target) = arguments.new_target else {
        return Ok(text);
    };
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::StringPrototype,
        arguments.realm,
    )?;
    string_create(agent, scope, text, prototype)
}
