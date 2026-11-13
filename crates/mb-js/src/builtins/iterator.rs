//! %Iterator% und %Iterator.prototype% (Abschnitt 27.1.3) mit allen
//! Helfern, %IteratorHelperPrototype% (Abschnitt 27.1.2.1) und
//! %WrapForValidIteratorPrototype% (Abschnitt 27.1.3.2.2.1), dazu die
//! Iteratoren für Arrays (Abschnitt 23.1.5) und Zeichenketten (Abschnitt
//! 22.1.5) mit ihren Prototypen. Den Konstruktor und seine Funktionen
//! enthält [`constructor`], die Methoden des Prototyps [`prototype`], die
//! Objekte, die `map` und die anderen Helfer anlegen, [`helper`].
//!
//! Alle diese Iteratoren sind Objekte der Art [`ObjectClass::Iterator`] mit
//! drei internen Feldern; was sie bedeuten, hängt von der Art im zweiten
//! Feld ab. Für Arrays sind es [[IteratedArrayLike]],
//! [[ArrayLikeIterationKind]] und [[ArrayLikeNextIndex]]. Den Iterator für
//! Zeichenketten beschreibt die Spezifikation als Generator aus einer
//! Closure (CreateIteratorFromClosure, Abschnitt 27.8.3.8); er hält
//! stattdessen die Zeichenkette und die Position, die die Closure
//! festhält. Die Art steht für [[GeneratorBrand]], ~completed~ für eine
//! Zeichenkette *undefined*, und ~executing~ wird er nie, denn die Closure
//! ruft keinen Code der Sprache auf (GeneratorValidate, Abschnitt
//! 27.8.3.2). Ein Helfer hält im ersten Feld eine interne Zelle mit dem
//! Zustand seiner Closure, die Hülle aus `Iterator.from` [[Iterated]]. Die
//! Iteratoren über Maps und Sets legt [`super::collection`] an.

mod constructor;
mod helper;
mod prototype;

use super::{define_functions, define_symbol_function, define_to_string_tag};
use crate::agent::{Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall, create_builtin_function};
use crate::conversion::{to_length, to_property_key};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, ObjectClass, OutOfMemory, Scope};
use crate::iterator::create_iterator_result_object;
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::number;
use crate::object::{INLINE, array_create, ordinary_object_create};
use crate::operations::{create_data_property_or_throw, define_property_or_throw, get};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};

/// Was der Iterator durchläuft: ein Objekt, eine Zeichenkette oder
/// *undefined*, wenn er fertig ist; für einen Helfer die Zelle mit seinem
/// Zustand, für eine Hülle [[Iterated]].[[Iterator]], für eine Map oder ein
/// Set deren Tabelle.
pub(super) const TARGET: usize = 0;
/// Die Art des Iterators, eine der Zahlen unten.
pub(super) const KIND: usize = 1;
/// Der nächste Index; für eine Hülle [[Iterated]].[[NextMethod]], für eine
/// Map oder ein Set die Zelle mit der Stelle in der Tabelle.
pub(super) const INDEX: usize = 2;

/// ~key~: die Indizes eines Arrays.
pub(super) const KEYS: i32 = 0;
/// ~value~: die Elemente.
pub(super) const VALUES: i32 = 1;
/// ~key+value~: Paare aus Index und Element.
pub(super) const ENTRIES: i32 = 2;
/// Die Codepunkte einer Zeichenkette.
const STRING: i32 = 3;
/// Ein Iterator Helper (Abschnitt 27.1.2).
const HELPER: i32 = 4;
/// Eine Hülle aus `Iterator.from` mit [[Iterated]].
const WRAP: i32 = 5;
/// Ein Iterator über eine Map (Abschnitt 24.1.5) mit ~key~, ~value~ oder
/// ~key+value~.
pub(super) const MAP_KEYS: i32 = 6;
pub(super) const MAP_VALUES: i32 = 7;
pub(super) const MAP_ENTRIES: i32 = 8;
/// Ein Iterator über ein Set (Abschnitt 24.2.6) mit ~value~ oder
/// ~key+value~.
pub(super) const SET_VALUES: i32 = 9;
pub(super) const SET_ENTRIES: i32 = 10;

static ITERATOR_PROTOTYPE_ITERATOR: Builtin = Builtin {
    name: "[Symbol.iterator]",
    length: 0,
    constructor: false,
    behaviour: iterator_prototype_iterator,
};

static ARRAY_ITERATOR_FUNCTIONS: [Builtin; 1] = [Builtin {
    name: "next",
    length: 0,
    constructor: false,
    behaviour: array_iterator_next,
}];

static STRING_ITERATOR_FUNCTIONS: [Builtin; 1] = [Builtin {
    name: "next",
    length: 0,
    constructor: false,
    behaviour: string_iterator_next,
}];

/// String.prototype [ %Symbol.iterator% ] (Abschnitt 22.1.3.36), für
/// `string`.
pub(super) static STRING_ITERATOR: Builtin = Builtin {
    name: "[Symbol.iterator]",
    length: 0,
    constructor: false,
    behaviour: string_prototype_iterator,
};

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(object))?;
    define_symbol_function(
        agent,
        scope,
        realm,
        prototype,
        WellKnownSymbol::Iterator,
        &ITERATOR_PROTOTYPE_ITERATOR,
    )?;
    define_functions(agent, scope, realm, prototype, &prototype::FUNCTIONS)?;
    define_symbol_function(
        agent,
        scope,
        realm,
        prototype,
        WellKnownSymbol::Dispose,
        &prototype::DISPOSE,
    )?;
    agent.set_intrinsic(realm, Intrinsic::IteratorPrototype, prototype);
    // `constructor` und @@toStringTag sind Accessoren, nicht aufzählbar
    // und konfigurierbar (Abschnitte 27.1.3.3.2 und 27.1.3.3.18).
    let accessors = [
        (
            agent.key_from_units(scope, &units("constructor"))?,
            &prototype::CONSTRUCTOR_GETTER,
            &prototype::CONSTRUCTOR_SETTER,
        ),
        (
            PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::ToStringTag)),
            &prototype::TAG_GETTER,
            &prototype::TAG_SETTER,
        ),
    ];
    for (key, getter, setter) in accessors {
        let getter = create_builtin_function(agent, scope, realm, getter)?;
        let setter = create_builtin_function(agent, scope, realm, setter)?;
        let descriptor = PropertyDescriptor::accessor(getter, setter, false, true);
        define_property_or_throw(agent, scope, prototype, key, &descriptor)?;
    }
    // Iterator.prototype ist weder beschreibbar noch aufzählbar noch
    // konfigurierbar (Abschnitt 27.1.3.2.3).
    let constructor = create_builtin_function(agent, scope, realm, &constructor::CONSTRUCTOR)?;
    let key = agent.key_from_units(scope, &units("prototype"))?;
    let descriptor = PropertyDescriptor::data(prototype, false, false, false);
    define_property_or_throw(agent, scope, constructor, key, &descriptor)?;
    define_functions(agent, scope, realm, constructor, &constructor::FUNCTIONS)?;
    agent.set_intrinsic(realm, Intrinsic::Iterator, constructor);
    let helper = ordinary_object_create(agent, scope, Some(prototype))?;
    define_functions(agent, scope, realm, helper, &helper::FUNCTIONS)?;
    define_to_string_tag(agent, scope, helper, "Iterator Helper")?;
    agent.set_intrinsic(realm, Intrinsic::IteratorHelperPrototype, helper);
    let wrap = ordinary_object_create(agent, scope, Some(prototype))?;
    define_functions(agent, scope, realm, wrap, &constructor::WRAP_FUNCTIONS)?;
    agent.set_intrinsic(realm, Intrinsic::WrapForValidIteratorPrototype, wrap);
    let families: [(Intrinsic, &'static [Builtin], &str); 2] = [
        (
            Intrinsic::ArrayIteratorPrototype,
            &ARRAY_ITERATOR_FUNCTIONS,
            "Array Iterator",
        ),
        (
            Intrinsic::StringIteratorPrototype,
            &STRING_ITERATOR_FUNCTIONS,
            "String Iterator",
        ),
    ];
    for (intrinsic, functions, tag) in families {
        let object = ordinary_object_create(agent, scope, Some(prototype))?;
        define_functions(agent, scope, realm, object, functions)?;
        define_to_string_tag(agent, scope, object, tag)?;
        agent.set_intrinsic(realm, intrinsic, object);
    }
    Ok(())
}

/// Ein neuer Iterator der Art `kind` über `target` mit `prototype`, beim
/// Index 0. Für Arrays ist das CreateArrayIterator (Abschnitt 23.1.5.1).
pub(super) fn iterator_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
    target: Local<'_>,
    kind: i32,
) -> Result<Local<'s>, OutOfMemory> {
    let shape = agent.empty_shape(scope);
    let iterator = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::Iterator,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    if let Kind::Object(object) = heap.get(iterator).kind() {
        heap.set_iterator_field(object, TARGET, heap.get(target));
        heap.set_iterator_field(object, KIND, Value::int32(kind));
        heap.set_iterator_field(object, INDEX, Value::int32(0));
    }
    Ok(iterator)
}

/// Das interne Feld `index` des Iterators `this`, `None`, wenn `this` kein
/// Iterator ist. Nach jedem Aufruf, der Speicher anfordern kann, ist es neu
/// zu lesen.
fn field<'s>(agent: &Agent, scope: &'s Scope, this: Local<'_>, index: usize) -> Option<Local<'s>> {
    let heap = agent.heap();
    let Kind::Object(object) = heap.get(this).kind() else {
        return None;
    };
    let value = heap.iterator_field(object, index)?;
    Some(heap.local(scope, value))
}

fn set_field(agent: &Agent, this: Local<'_>, index: usize, value: Value<'_>) {
    let heap = agent.heap();
    if let Kind::Object(object) = heap.get(this).kind() {
        heap.set_iterator_field(object, index, value);
    }
}

/// Die Art des Iterators `this`, `None` für andere Werte.
fn kind_of(agent: &Agent, scope: &Scope, this: Local<'_>) -> Option<i32> {
    let kind = field(agent, scope, this, KIND)?;
    agent.heap().get(kind).as_int32()
}

fn done<'s>(agent: &mut Agent, scope: &'s Scope, realm: RealmId) -> Completion<Local<'s>> {
    let undefined = agent.heap().local(scope, Value::UNDEFINED);
    create_iterator_result_object(agent, scope, realm, undefined, true)
}

/// Iterator.prototype [ %Symbol.iterator% ] (Abschnitt 27.1.3.3.17).
fn iterator_prototype_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    Ok(heap.local(scope, heap.get(arguments.this)))
}

/// %ArrayIteratorPrototype%.next ( ) (Abschnitt 23.1.5.2.1). Arrays mit
/// [[TypedArrayName]] gibt es noch nicht.
fn array_iterator_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    let realm = arguments.realm;
    let kind = match kind_of(agent, scope, this) {
        Some(kind @ (KEYS | VALUES | ENTRIES)) => kind,
        _ => return Err(agent.throw_error(ErrorKind::TypeError)),
    };
    let Some(array) = field(agent, scope, this, TARGET) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    if agent.heap().get(array).is_undefined() {
        return done(agent, scope, realm);
    }
    let index = field(agent, scope, this, INDEX)
        .and_then(|index| agent.heap().get(index).as_number())
        .unwrap_or(0.0);
    // LengthOfArrayLike (Abschnitt 7.3.18).
    let key = agent.key_from_units(scope, &units("length"))?;
    let length = get(agent, scope, array, key)?;
    let length = to_length(agent, scope, length)?;
    if index >= length {
        set_field(agent, this, TARGET, Value::UNDEFINED);
        return done(agent, scope, realm);
    }
    // Der Index bleibt unter 2⁵³ − 1 und damit exakt.
    set_field(agent, this, INDEX, Value::number(index + 1.0));
    let number = agent.heap().local(scope, Value::number(index));
    if kind == KEYS {
        return create_iterator_result_object(agent, scope, realm, number, false);
    }
    let key = to_property_key(agent, scope, number)?;
    let element = get(agent, scope, array, key)?;
    if kind == VALUES {
        return create_iterator_result_object(agent, scope, realm, element, false);
    }
    // CreateArrayFromList (Abschnitt 7.3.17).
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    let pair = array_create(agent, scope, 0, Some(prototype))?;
    create_data_property_or_throw(agent, scope, pair, PropertyKey::Index(0), number)?;
    create_data_property_or_throw(agent, scope, pair, PropertyKey::Index(1), element)?;
    create_iterator_result_object(agent, scope, realm, pair, false)
}

/// String.prototype [ %Symbol.iterator% ] ( ) (Abschnitt 22.1.3.36).
fn string_prototype_iterator<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    // RequireObjectCoercible (Abschnitt 7.2.1).
    let this = agent.heap().get(arguments.this);
    if this.is_undefined() || this.is_null() {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let string = crate::conversion::to_string(agent, scope, arguments.this)?;
    let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::StringIteratorPrototype);
    Ok(iterator_create(agent, scope, prototype, string, STRING)?)
}

/// %StringIteratorPrototype%.next ( ) (Abschnitt 22.1.5.1.1): GeneratorResume
/// (Abschnitt 27.8.3.3) für die Closure aus `String.prototype[@@iterator]`,
/// mit CodePointAt (Abschnitt 11.1.4).
fn string_iterator_next<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let this = arguments.this;
    let realm = arguments.realm;
    if kind_of(agent, scope, this) != Some(STRING) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let heap = agent.heap();
    let target = field(agent, scope, this, TARGET).map(|target| heap.get(target));
    let Some(Kind::String(string)) = target.map(Value::kind) else {
        return done(agent, scope, realm);
    };
    // Die Position ist eine ganze Zahl bis zur Länge, höchstens `u32::MAX`;
    // ToUint32 gibt sie unverändert zurück.
    let position = field(agent, scope, this, INDEX)
        .and_then(|index| heap.get(index).as_number())
        .map(number::to_uint32)
        .and_then(|index| usize::try_from(index).ok())
        .unwrap_or(0);
    let Some(first) = heap.string_unit(string, position) else {
        set_field(agent, this, TARGET, Value::UNDEFINED);
        return done(agent, scope, realm);
    };
    let mut units = vec![first];
    if (0xD800..=0xDBFF).contains(&first)
        && let Some(second) = position
            .checked_add(1)
            .and_then(|next| heap.string_unit(string, next))
        && (0xDC00..=0xDFFF).contains(&second)
    {
        units.push(second);
    }
    // Zeichenketten haben höchstens `u32::MAX` Codeeinheiten, also auch die
    // Position dahinter.
    let next = position
        .checked_add(units.len())
        .and_then(|next| u32::try_from(next).ok())
        .ok_or(OutOfMemory)?;
    set_field(agent, this, INDEX, Value::number(f64::from(next)));
    let result = agent.heap_mut().alloc_string(scope, &units)?;
    create_iterator_result_object(agent, scope, realm, result, false)
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
