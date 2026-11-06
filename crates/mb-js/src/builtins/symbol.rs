//! %Symbol% (Abschnitt 20.4.1) mit den bekannten Symbolen als
//! Eigenschaften und `prototype` (Abschnitt 20.4.2). `Symbol.for`,
//! `Symbol.keyFor` und die Methoden von %Symbol.prototype% kommen mit der
//! Welle für Symbole.

use super::link_constructor;
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::to_string;
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::key::WellKnownSymbol;
use crate::operations::define_property_or_throw;
use crate::realm::{Intrinsic, RealmId};

static CONSTRUCTOR: Builtin = Builtin {
    name: "Symbol",
    length: 0,
    constructor: true,
    behaviour: symbol,
};

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::SymbolPrototype);
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    // Die bekannten Symbole von Symbol.asyncDispose bis Symbol.unscopables
    // (in den Abschnitten 20.4.2.1 bis 20.4.2.18): weder beschreibbar noch
    // aufzählbar noch konfigurierbar.
    for symbol in WellKnownSymbol::ALL {
        let description = symbol.description();
        let name = description.strip_prefix("Symbol.").unwrap_or(description);
        let key = agent.key_from_units(scope, &name.encode_utf16().collect::<Vec<_>>())?;
        let value = agent.well_known_symbol(scope, symbol);
        let descriptor = PropertyDescriptor::data(value, false, false, false);
        define_property_or_throw(agent, scope, constructor, key, &descriptor)?;
    }
    agent.set_intrinsic(realm, Intrinsic::Symbol, constructor);
    Ok(())
}

/// Symbol ( [ description ] ) (Abschnitt 20.4.1.1).
fn symbol<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    if arguments.new_target.is_some() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let description = arguments.argument(agent, scope, 0);
    let description = if agent.heap().get(description).is_undefined() {
        None
    } else {
        Some(to_string(agent, scope, description)?)
    };
    Ok(agent.new_symbol(scope, description)?)
}
