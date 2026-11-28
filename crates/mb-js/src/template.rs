//! Template-Objekte für Templates mit Tag (GetTemplateObject, Abschnitt
//! 13.2.8.4).
//!
//! Die Spezifikation merkt sich jedes Objekt in [[TemplateMap]] des Realms,
//! mit der Stelle im Quelltext als Schlüssel. Hier hält es der Platz seiner
//! Konstante im Pool ([`crate::bytecode::Constant::Template`]). Einen Pool
//! hat jede gemeinsame Funktion einmal, ebenso jeder Rahmen eines Scripts,
//! Moduls oder `eval`; der Code jeder Stelle läuft in genau einem Realm.
//! Ein Platz gehört also zu genau einer Stelle und einem Realm, wie ein
//! Eintrag in [[TemplateMap]]. Wird derselbe Quelltext noch einmal
//! ausgewertet, auch mit Bytecode aus dem Zwischenspeicher, ist das eine
//! neue Stelle mit neuem Pool und neuem Objekt.

use crate::agent::{Agent, Completion, ErrorKind};
use crate::bytecode::TemplateStrings;
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::key::PropertyKey;
use crate::object::array_create;
use crate::operations::{IntegrityLevel, define_property_or_throw, set_integrity_level};
use crate::realm::{Intrinsic, RealmId};
use crate::value::Value;

/// Legt das Template-Objekt für die Teile `strings` in `realm` an
/// (GetTemplateObject, Abschnitt 13.2.8.4, Schritte 3 bis 13): ein Array
/// mit den Werten, eines mit den Rohwerten als `raw`, beide eingefroren.
pub(crate) fn create_template_object<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    strings: &TemplateStrings,
) -> Completion<Local<'s>> {
    let count = strings.raw.len();
    if strings.cooked.len() != count {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let length = u64::try_from(count).map_err(|_| agent.throw_error(ErrorKind::RangeError))?;
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    let template = array_create(agent, scope, length, Some(prototype))?;
    let raw = array_create(agent, scope, length, Some(prototype))?;
    for (index, (cooked, raw_value)) in strings.cooked.iter().zip(&strings.raw).enumerate() {
        let Ok(index) = u32::try_from(index) else {
            return Err(agent.throw_error(ErrorKind::RangeError));
        };
        agent.scope(|agent, inner| -> Completion<()> {
            let value = match cooked {
                Some(units) => agent.heap_mut().alloc_string(inner, units)?,
                None => agent.heap().local(inner, Value::UNDEFINED),
            };
            let descriptor = PropertyDescriptor::data(value, false, true, false);
            define_property_or_throw(
                agent,
                inner,
                template,
                PropertyKey::Index(index),
                &descriptor,
            )?;
            let value = agent.heap_mut().alloc_string(inner, raw_value)?;
            let descriptor = PropertyDescriptor::data(value, false, true, false);
            define_property_or_throw(agent, inner, raw, PropertyKey::Index(index), &descriptor)
        })?;
    }
    set_integrity_level(agent, scope, raw, IntegrityLevel::Frozen)?;
    let key = agent.key_from_units(scope, &"raw".encode_utf16().collect::<Vec<_>>())?;
    let descriptor = PropertyDescriptor::data(raw, false, false, false);
    define_property_or_throw(agent, scope, template, key, &descriptor)?;
    set_integrity_level(agent, scope, template, IntegrityLevel::Frozen)?;
    Ok(template)
}
