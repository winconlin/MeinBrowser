//! Die zusätzlichen Eigenschaften von %String.prototype% aus Anhang B.2.2:
//! `substr` und die Methoden, die die Zeichenkette in ein HTML-Element
//! setzen. `trimLeft` und `trimRight` legt `create` an, als dieselben
//! Funktionsobjekte wie `trimStart` und `trimEnd`.

use super::prototype::relative_position;
use super::{
    argument_string, clamp_position, count, function, is_undefined, length_of, new_string,
    push_units, substring, this_string,
};
use crate::agent::{Agent, Completion};
use crate::builtin::{Builtin, BuiltinCall};
use crate::builtins::array::units_of;
use crate::builtins::units;
use crate::conversion::to_integer_or_infinity;
use crate::heap::{Local, Scope};

/// Die Methoden in der Reihenfolge von Anhang B.2.2.
pub(super) static FUNCTIONS: [Builtin; 14] = [
    function("substr", 2, substr),
    function("anchor", 1, anchor),
    function("big", 0, big),
    function("blink", 0, blink),
    function("bold", 0, bold),
    function("fixed", 0, fixed),
    function("fontcolor", 1, fontcolor),
    function("fontsize", 1, fontsize),
    function("italics", 0, italics),
    function("link", 1, link),
    function("small", 0, small),
    function("strike", 0, strike),
    function("sub", 0, sub),
    function("sup", 0, sup),
];

/// String.prototype.substr ( start, length ) (Abschnitt B.2.2.1).
fn substr<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let size = length_of(agent, string);
    let start = arguments.argument(agent, scope, 0);
    let start = to_integer_or_infinity(agent, scope, start)?;
    let start = relative_position(start, size);
    let length = arguments.argument(agent, scope, 1);
    let length = if is_undefined(agent, length) {
        count(size)
    } else {
        to_integer_or_infinity(agent, scope, length)?
    };
    let length = clamp_position(length, size);
    let end = start.saturating_add(length).min(size);
    if start >= end {
        return new_string(agent, scope, &[]);
    }
    substring(agent, scope, string, start, end)
}

/// CreateHTML (Abschnitt B.2.2.2.1) mit *this* als Inhalt: `<tag>` oder
/// `<tag attribute="…">` um die Zeichenkette, im Wert des Attributs aus
/// dem ersten Argument jedes Anführungszeichen als `&quot;`.
fn create_html<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    tag: &str,
    attribute: Option<&str>,
) -> Completion<Local<'s>> {
    let string = this_string(agent, scope, arguments)?;
    let mut result = units("<");
    result.extend(units(tag));
    if let Some(attribute) = attribute {
        let value = argument_string(agent, scope, arguments, 0)?;
        result.extend(units(" "));
        result.extend(units(attribute));
        result.extend(units("=\""));
        let quote = units("&quot;");
        for part in units_of(agent, value).split_inclusive(|&unit| unit == 0x22) {
            match part.split_last() {
                Some((&0x22, before)) => {
                    push_units(agent, &mut result, before)?;
                    push_units(agent, &mut result, &quote)?;
                }
                _ => push_units(agent, &mut result, part)?,
            }
        }
        result.extend(units("\""));
    }
    result.extend(units(">"));
    push_units(agent, &mut result, &units_of(agent, string))?;
    result.extend(units("</"));
    result.extend(units(tag));
    result.extend(units(">"));
    new_string(agent, scope, &result)
}

/// String.prototype.anchor ( name ) (Abschnitt B.2.2.2).
fn anchor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "a", Some("name"))
}

/// String.prototype.big ( ) (Abschnitt B.2.2.3).
fn big<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "big", None)
}

/// String.prototype.blink ( ) (Abschnitt B.2.2.4).
fn blink<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "blink", None)
}

/// String.prototype.bold ( ) (Abschnitt B.2.2.5).
fn bold<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "b", None)
}

/// String.prototype.fixed ( ) (Abschnitt B.2.2.6).
fn fixed<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "tt", None)
}

/// String.prototype.fontcolor ( colour ) (Abschnitt B.2.2.7).
fn fontcolor<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "font", Some("color"))
}

/// String.prototype.fontsize ( size ) (Abschnitt B.2.2.8).
fn fontsize<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "font", Some("size"))
}

/// String.prototype.italics ( ) (Abschnitt B.2.2.9).
fn italics<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "i", None)
}

/// String.prototype.link ( url ) (Abschnitt B.2.2.10).
fn link<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "a", Some("href"))
}

/// String.prototype.small ( ) (Abschnitt B.2.2.11).
fn small<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "small", None)
}

/// String.prototype.strike ( ) (Abschnitt B.2.2.12).
fn strike<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "strike", None)
}

/// String.prototype.sub ( ) (Abschnitt B.2.2.13).
fn sub<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "sub", None)
}

/// String.prototype.sup ( ) (Abschnitt B.2.2.14).
fn sup<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    create_html(agent, scope, arguments, "sup", None)
}
