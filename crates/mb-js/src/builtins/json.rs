//! %JSON% (Abschnitt 25.5): `JSON.parse` mit *reviver* und dem Kontext
//! mit `source` (JSON.parse source text access), `JSON.stringify`,
//! `JSON.rawJSON` und `JSON.isRawJSON`.
//!
//! Den Text prüft und zerlegt der Zerleger aus [`mb_json::tree`], den
//! auch `mb-json` nutzt, ohne den Parser der Sprache. Aus
//! dem Baum entstehen die Werte von oben nach unten mit einem Stapel; mit
//! *reviver* merkt sich jeder Knoten für Arrays und Objekte das Objekt, das
//! aus ihm entstand. Das ist der JSON Parse Record (Abschnitt 25.5.2.2):
//! [[Value]] eines Knotens ist sein Objekt oder sein primitiver Wert im
//! Baum, [[Elements]] und [[Entries]] sind seine Kinder, und von doppelten
//! Schlüsseln gilt der letzte, wie in CreateJSONParseRecord. Auch
//! InternalizeJSONProperty läuft ohne Rekursion über einen Stapel.
//!
//! [[IsRawJSON]] ist ein Kennzeichen im Kopfwort des Objekts.

mod stringify;

use std::collections::BTreeMap;

use super::{define_functions, define_to_string_tag, units};
use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::builtin::{Builtin, BuiltinCall};
use crate::compare::same_value;
use crate::conversion::to_string;
use crate::heap::{Local, Scope};
use crate::key::PropertyKey;
use crate::number;
use crate::object::{self, array_create, ordinary_object_create};
use crate::operations::{
    EnumerableKind, IntegrityLevel, call, create_data_property, create_data_property_or_throw,
    enumerable_own_properties, get, is_array, is_callable, length_of_array_like,
    set_integrity_level,
};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Value};
use mb_json::tree::{self, Limits, Node, Rejected, Tree};

/// Die Bytes eines Werts im Heap: ein Wort aus 64 Bit.
const VALUE_BYTES: usize = 8;

static FUNCTIONS: [Builtin; 4] = [
    Builtin {
        name: "isRawJSON",
        length: 1,
        constructor: false,
        behaviour: is_raw_json,
    },
    Builtin {
        name: "parse",
        length: 2,
        constructor: false,
        behaviour: parse,
    },
    Builtin {
        name: "rawJSON",
        length: 1,
        constructor: false,
        behaviour: raw_json,
    },
    Builtin {
        name: "stringify",
        length: 3,
        constructor: false,
        behaviour: stringify::stringify,
    },
];

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let json = ordinary_object_create(agent, scope, Some(prototype))?;
    define_functions(agent, scope, realm, json, &FUNCTIONS)?;
    define_to_string_tag(agent, scope, json, "JSON")?;
    agent.set_intrinsic(realm, Intrinsic::Json, json);
    Ok(())
}

fn is_object(agent: &Agent, value: Local<'_>) -> bool {
    matches!(agent.heap().get(value).kind(), Kind::Object(_))
}

fn string_units(agent: &Agent, value: Local<'_>) -> Vec<u16> {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => Vec::new(),
    }
}

/// ParseJSON (Abschnitt 25.5.2.1) bis auf das Erzeugen der Werte: ein
/// SyntaxError, wenn der Text kein JSON ist.
///
/// Jeder Knoten außer der Wurzel wird ein Element oder eine Eigenschaft
/// seines Arrays oder Objekts, also mindestens ein Wert von 8 Bytes im
/// Heap. Ein Baum mit mehr Knoten passt nie ins Speicherbudget; schon der
/// Zerleger hört dann auf, und das Parsen endet mit
/// [`Abrupt::OutOfMemory`], auch wenn der Text weiter hinten ungültig
/// wäre.
fn parse_json(agent: &mut Agent, text: &[u16]) -> Completion<Tree> {
    let limits = Limits {
        nodes: (agent.heap().limit() / VALUE_BYTES).saturating_add(1),
        depth: usize::MAX,
    };
    match tree::parse(text, limits) {
        Ok(parsed) => Ok(parsed),
        Err(Rejected::Invalid { .. } | Rejected::TooDeep { .. }) => {
            Err(agent.throw_error(ErrorKind::SyntaxError))
        }
        Err(Rejected::TooLarge) => Err(Abrupt::OutOfMemory),
    }
}

/// JSON.isRawJSON ( O ) (Abschnitt 25.5.1).
fn is_raw_json<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    let heap = agent.heap();
    let raw = match heap.get(value).kind() {
        Kind::Object(object) => heap.object_is_raw_json(object),
        _ => false,
    };
    Ok(heap.local(scope, Value::boolean(raw)))
}

/// JSON.rawJSON ( text ) (Abschnitt 25.5.3).
fn raw_json<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let text = arguments.argument(agent, scope, 0);
    let string = to_string(agent, scope, text)?;
    let units = string_units(agent, string);
    let first_allowed = |unit: u16| matches!(unit, 0x61..=0x7A | 0x30..=0x39 | 0x22 | 0x2D);
    let last_allowed = |unit: u16| matches!(unit, 0x61..=0x7A | 0x30..=0x39 | 0x22);
    let valid = match (units.first(), units.last()) {
        (Some(&first), Some(&last)) => first_allowed(first) && last_allowed(last),
        _ => false,
    };
    if !valid {
        return Err(agent.throw_error(ErrorKind::SyntaxError));
    }
    // Mit diesen ersten und letzten Zeichen kann der Text weder ein Array
    // noch ein Objekt sein.
    parse_json(agent, &units)?;
    let object = ordinary_object_create(agent, scope, None)?;
    if let Kind::Object(cell) = agent.heap().get(object).kind() {
        agent.heap().mark_raw_json(cell);
    }
    let key = agent.key_from_units(scope, &self::units("rawJSON"))?;
    create_data_property_or_throw(agent, scope, object, key, string)?;
    set_integrity_level(agent, scope, object, IntegrityLevel::Frozen)?;
    Ok(object)
}

/// JSON.parse ( text [ , reviver ] ) (Abschnitt 25.5.2).
fn parse<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = arguments.realm;
    let text = arguments.argument(agent, scope, 0);
    let string = to_string(agent, scope, text)?;
    let source = string_units(agent, string);
    let parsed = parse_json(agent, &source)?;
    let reviver = arguments.argument(agent, scope, 1);
    if !is_callable(agent.heap(), agent.heap().get(reviver)) {
        let (value, _) = materialize(agent, scope, realm, &parsed, false)?;
        return Ok(value);
    }
    let (unfiltered, objects) = materialize(agent, scope, realm, &parsed, true)?;
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let root = ordinary_object_create(agent, scope, Some(prototype))?;
    let key = agent.key_from_units(scope, &[])?;
    create_data_property_or_throw(agent, scope, root, key, unfiltered)?;
    let records = Records {
        parsed: &parsed,
        source: &source,
        objects: &objects,
    };
    internalize(agent, scope, realm, &records, root, reviver)
}

/// Ein Array oder Objekt, das gerade entsteht.
struct Building<'s> {
    node: usize,
    object: Local<'s>,
    next: usize,
}

/// Erzeugt die Werte des Baums mit %Object.prototype% und
/// %Array.prototype% aus `realm`. Mit `keep` liefert es zu jedem Knoten
/// das Objekt, das aus ihm entstand (für Arrays und Objekte).
fn materialize<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    parsed: &Tree,
    keep: bool,
) -> Completion<(Local<'s>, BTreeMap<usize, Local<'s>>)> {
    let mut objects = BTreeMap::new();
    let root = match create_node(agent, scope, realm, parsed, 0)? {
        Created::Primitive(value) => return Ok((value, objects)),
        Created::Container(object) => object,
    };
    if keep {
        objects.insert(0, root);
    }
    let mut stack = vec![Building {
        node: 0,
        object: root,
        next: 0,
    }];
    while let Some(top) = stack.last_mut() {
        let (node, holder, position) = (top.node, top.object, top.next);
        let child = match parsed.nodes.get(node) {
            Some(Node::Array(elements)) => elements.get(position).map(|&child| (None, child)),
            Some(Node::Object(members)) => members
                .get(position)
                .map(|(key, child)| (Some(key.as_slice()), *child)),
            _ => None,
        };
        let Some((key, child)) = child else {
            stack.pop();
            continue;
        };
        top.next = position.saturating_add(1);
        agent.spend_execution_steps(1)?;
        let created = agent.scope(|agent, inner| -> Completion<Option<Local<'s>>> {
            let property = match key {
                Some(key) => agent.key_from_units(inner, key)?,
                None => {
                    let index = u32::try_from(position)
                        .ok()
                        .filter(|&index| index < u32::MAX)
                        .ok_or_else(|| agent.throw_error(ErrorKind::RangeError))?;
                    PropertyKey::Index(index)
                }
            };
            let (value, container) = match create_node(agent, inner, realm, parsed, child)? {
                Created::Primitive(value) => (value, None),
                Created::Container(object) => {
                    let heap = agent.heap();
                    (object, Some(heap.local(scope, heap.get(object))))
                }
            };
            create_data_property_or_throw(agent, inner, holder, property, value)?;
            Ok(container)
        })?;
        if let Some(object) = created {
            if keep {
                objects.insert(child, object);
            }
            stack.push(Building {
                node: child,
                object,
                next: 0,
            });
        }
    }
    Ok((root, objects))
}

/// Was aus einem Knoten entsteht.
enum Created<'s> {
    Primitive(Local<'s>),
    /// Ein leeres Array oder Objekt; die Kinder kommen danach.
    Container(Local<'s>),
}

fn create_node<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    parsed: &Tree,
    node: usize,
) -> Completion<Created<'s>> {
    let heap = agent.heap();
    Ok(match parsed.nodes.get(node) {
        Some(Node::Null) | None => Created::Primitive(heap.local(scope, Value::NULL)),
        Some(Node::Boolean(boolean)) => {
            Created::Primitive(heap.local(scope, Value::boolean(*boolean)))
        }
        Some(Node::Number(number)) => Created::Primitive(heap.local(scope, Value::number(*number))),
        Some(Node::String(units)) => {
            Created::Primitive(agent.heap_mut().alloc_string(scope, units)?)
        }
        Some(Node::Array(_)) => {
            let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
            Created::Container(array_create(agent, scope, 0, Some(prototype))?)
        }
        Some(Node::Object(_)) => {
            let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
            Created::Container(ordinary_object_create(agent, scope, Some(prototype))?)
        }
    })
}

/// Die JSON Parse Records: der Baum, der Text und die Objekte der Knoten.
struct Records<'r, 's> {
    parsed: &'r Tree,
    source: &'r [u16],
    objects: &'r BTreeMap<usize, Local<'s>>,
}

impl Records<'_, '_> {
    /// Ob [[Value]] des Knotens `node` SameValue mit `value` ist.
    fn same_value(&self, agent: &Agent, node: usize, value: Local<'_>) -> bool {
        let heap = agent.heap();
        let value = heap.get(value);
        match (self.parsed.nodes.get(node), value.kind()) {
            (Some(Node::Null), Kind::Null) => true,
            (Some(Node::Boolean(expected)), Kind::Boolean(found)) => *expected == found,
            (Some(Node::Number(expected)), Kind::Number(found)) => {
                same_value(heap, Value::number(*expected), Value::number(found))
            }
            (Some(Node::String(expected)), Kind::String(found)) => {
                heap.string_units(found) == *expected
            }
            (Some(Node::Array(_) | Node::Object(_)), Kind::Object(_)) => self
                .objects
                .get(&node)
                .is_some_and(|object| same_value(heap, heap.get(*object), value)),
            _ => false,
        }
    }

    /// Der Text eines primitiven Knotens.
    fn source(&self, node: usize) -> &[u16] {
        let (start, end) = self.parsed.ranges.get(node).copied().unwrap_or_default();
        self.source.get(start..end).unwrap_or_default()
    }

    /// [[Elements]] eines Arrays.
    fn elements(&self, node: usize) -> &[usize] {
        match self.parsed.nodes.get(node) {
            Some(Node::Array(elements)) => elements,
            _ => &[],
        }
    }

    /// [[Entries]] eines Objekts: zu jedem Schlüssel der letzte Knoten.
    fn entries(&self, node: usize) -> BTreeMap<Vec<u16>, usize> {
        match self.parsed.nodes.get(node) {
            Some(Node::Object(members)) => members
                .iter()
                .map(|(key, child)| (key.clone(), *child))
                .collect(),
            _ => BTreeMap::new(),
        }
    }
}

/// Ein Aufruf von InternalizeJSONProperty, der gerade die Eigenschaften
/// seines Werts durchläuft. `holder`, `value` und `context` sind die
/// Handles seiner Tiefe (siehe [`frame_slots`]).
struct Internalizing<'s> {
    holder: Local<'s>,
    name: Vec<u16>,
    value: Local<'s>,
    context: Local<'s>,
    /// Die Schlüssel eines Objekts, für ein Array leer.
    keys: Vec<Vec<u16>>,
    /// Die Länge eines Arrays.
    length: f64,
    is_array: bool,
    next: usize,
    /// [[Elements]] oder [[Entries]], wenn der Record noch zum Wert passt.
    elements: Vec<usize>,
    entries: BTreeMap<Vec<u16>, usize>,
}

/// Was der Anfang von InternalizeJSONProperty ergab.
enum Started<'i, 's> {
    /// Der Wert ist kein Objekt: das Ergebnis von *reviver*.
    Done(Local<'i>),
    /// Seine Eigenschaften sind noch zu durchlaufen.
    Open(Internalizing<'s>),
}

/// Die Handles für `holder`, `value` und `context` des Aufrufs in der Tiefe
/// `depth`. Jede Tiefe hat eigene, die jeder neue Aufruf dort überschreibt
/// und ein fertiger leert; so hält der Durchlauf nur fest, was gerade
/// offen ist.
fn frame_slots<'s>(
    agent: &Agent,
    scope: &'s Scope,
    slots: &mut Vec<[Local<'s>; 3]>,
    depth: usize,
) -> [Local<'s>; 3] {
    if let Some(&found) = slots.get(depth) {
        return found;
    }
    let heap = agent.heap();
    let created = [
        heap.local(scope, Value::UNDEFINED),
        heap.local(scope, Value::UNDEFINED),
        heap.local(scope, Value::UNDEFINED),
    ];
    slots.push(created);
    created
}

/// InternalizeJSONProperty(root, "", reviver, snapshot) (Abschnitt
/// 25.5.2.4) ohne Rekursion.
fn internalize<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    records: &Records<'_, '_>,
    root: Local<'s>,
    reviver: Local<'s>,
) -> Completion<Local<'s>> {
    let mut stack: Vec<Internalizing<'s>> = Vec::new();
    let mut slots: Vec<[Local<'s>; 3]> = Vec::new();
    let first = frame_slots(agent, scope, &mut slots, 0);
    match start(
        agent,
        scope,
        realm,
        records,
        root,
        &[],
        Some(0),
        reviver,
        first,
    )? {
        Started::Done(result) => return Ok(result),
        Started::Open(frame) => stack.push(frame),
    }
    while let Some(top) = stack.last_mut() {
        let key = if top.is_array {
            // Indizes unter 2⁵³ sind als f64 genau.
            let index = top.next as f64;
            (index < top.length).then(|| units(&number::to_string(index)))
        } else {
            top.keys.get(top.next).cloned()
        };
        let Some(key) = key else {
            // Alle Eigenschaften durchlaufen: *reviver* für den Wert selbst,
            // dann ins umgebende Objekt eintragen.
            let Some(done) = stack.pop() else {
                break;
            };
            let last = stack.is_empty();
            let returned = agent.scope(|agent, inner| -> Completion<Option<Local<'s>>> {
                let name = agent.heap_mut().alloc_string(inner, &done.name)?;
                let result = call(
                    agent,
                    inner,
                    reviver,
                    done.holder,
                    &[name, done.value, done.context],
                )?;
                if last {
                    let heap = agent.heap();
                    return Ok(Some(heap.local(scope, heap.get(result))));
                }
                apply(agent, inner, done.holder, &done.name, result)?;
                Ok(None)
            })?;
            let heap = agent.heap();
            for slot in [done.holder, done.value, done.context] {
                heap.set(slot, Value::UNDEFINED);
            }
            if let Some(result) = returned {
                return Ok(result);
            }
            continue;
        };
        let record = if top.is_array {
            top.elements.get(top.next).copied()
        } else {
            top.entries.get(&key).copied()
        };
        top.next = top.next.saturating_add(1);
        let holder = top.value;
        agent.spend_execution_steps(1)?;
        let child = frame_slots(agent, scope, &mut slots, stack.len());
        let opened = agent.scope(|agent, inner| -> Completion<Option<Internalizing<'s>>> {
            match start(
                agent, inner, realm, records, holder, &key, record, reviver, child,
            )? {
                Started::Done(result) => {
                    apply(agent, inner, holder, &key, result)?;
                    Ok(None)
                }
                Started::Open(frame) => Ok(Some(frame)),
            }
        })?;
        if let Some(frame) = opened {
            stack.push(frame);
        }
    }
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// Die Schritte 1 bis 4 von InternalizeJSONProperty für `holder` und
/// `name`; für einen Wert, der kein Objekt ist, auch Schritt 5. Ein Wert,
/// der ein Objekt ist, kommt mit `holder` und `context` in `slots`.
///
/// Danach braucht kein Aufruf mehr das Objekt aus dem Knoten `record`;
/// sein Handle wird geleert, damit es nicht länger lebt als ohne Records.
#[allow(clippy::too_many_arguments)]
fn start<'i, 's>(
    agent: &mut Agent,
    scope: &'i Scope,
    realm: RealmId,
    records: &Records<'_, '_>,
    holder: Local<'_>,
    name: &[u16],
    record: Option<usize>,
    reviver: Local<'_>,
    slots: [Local<'s>; 3],
) -> Completion<Started<'i, 's>> {
    let key = agent.key_from_units(scope, name)?;
    let value = get(agent, scope, holder, key)?;
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let context = ordinary_object_create(agent, scope, Some(prototype))?;
    let matched = record.filter(|&node| records.same_value(agent, node, value));
    if let Some(object) = record.and_then(|node| records.objects.get(&node)) {
        agent.heap().set(*object, Value::UNDEFINED);
    }
    if let Some(node) = matched
        && !is_object(agent, value)
    {
        let text = agent.heap_mut().alloc_string(scope, records.source(node))?;
        let source = agent.key_from_units(scope, &units("source"))?;
        create_data_property_or_throw(agent, scope, context, source, text)?;
    }
    if !is_object(agent, value) {
        let name = agent.heap_mut().alloc_string(scope, name)?;
        let result = call(agent, scope, reviver, holder, &[name, value, context])?;
        return Ok(Started::Done(result));
    }
    let is_array = is_array(agent, value)?;
    let heap = agent.heap();
    let [holder_slot, value_slot, context_slot] = slots;
    heap.set(holder_slot, heap.get(holder));
    heap.set(value_slot, heap.get(value));
    heap.set(context_slot, heap.get(context));
    let mut frame = Internalizing {
        holder: holder_slot,
        name: name.to_vec(),
        value: value_slot,
        context: context_slot,
        keys: Vec::new(),
        length: 0.0,
        is_array,
        next: 0,
        elements: Vec::new(),
        entries: BTreeMap::new(),
    };
    if is_array {
        frame.length = length_of_array_like(agent, scope, value)?;
        if let Some(node) = matched {
            frame.elements = records.elements(node).to_vec();
        }
    } else {
        let keys = enumerable_own_properties(agent, scope, realm, value, EnumerableKind::Key)?;
        frame.keys = keys
            .into_iter()
            .map(|key| string_units(agent, key))
            .collect();
        if let Some(node) = matched {
            frame.entries = records.entries(node);
        }
    }
    Ok(Started::Open(frame))
}

/// Schritte 4.b.iv.3 bis 4 und 4.c.ii.2 bis 3: das Ergebnis von *reviver*
/// eintragen oder bei *undefined* die Eigenschaft löschen. Scheitern
/// [[Delete]] oder CreateDataProperty, bleibt das ohne Fehler.
fn apply(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: &[u16],
    result: Local<'_>,
) -> Completion<()> {
    let key = agent.key_from_units(scope, name)?;
    if agent.heap().get(result).is_undefined() {
        object::delete(agent, scope, object, key)?;
    } else {
        create_data_property(agent, scope, object, key, result)?;
    }
    Ok(())
}
