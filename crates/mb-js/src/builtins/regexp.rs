//! %RegExp% (Abschnitt 22.2.4) mit `RegExp.escape` und @@species,
//! %RegExp.prototype% (Abschnitt 22.2.6) mit `compile` aus Anhang B.2.4 und
//! %RegExpStringIteratorPrototype% (Abschnitt 22.2.9). Die Methoden unter
//! @@match, @@matchAll, @@replace, @@search und @@split und der Iterator
//! stehen in `symbols`.
//!
//! Ein regulärer Ausdruck ist ein Objekt der Art [`ObjectClass::RegExp`].
//! Seine Felder halten [[OriginalSource]], [[OriginalFlags]] und die Nummer
//! seines Programms im Agent ([`crate::regexp::cache`]); das Programm steht
//! für [[RegExpRecord]] und [[RegExpMatcher]]. Ist es aus dem Speicher der
//! Programme gefallen, wird es aus Quelltext und Flags neu übersetzt.
//!
//! Übersetzen und Abgleich zählen ihre Arbeit in Einheiten der Engine; je
//! [`WORK_PER_STEP`] Einheiten kosten einen Arbeitsschritt des Agents. Ein
//! Abgleich, dessen Stapel über das freie Speicherbudget wüchse, endet als
//! [`Abrupt::OutOfMemory`]. Nennt ein Muster eine Unicode-Eigenschaft,
//! deren Tabellen noch fehlen, endet das Anlegen als nicht unterstützt.

mod symbols;

use std::rc::Rc;

use super::array::units_of;
use super::string::{count, is_object, is_undefined, new_string, number, undefined, unit_at};
use super::{
    define_functions, define_getter, define_species, define_symbol_function, define_to_string_tag,
    link_constructor, units,
};
use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::builtin::{Behaviour, Builtin, BuiltinCall, create_builtin_function};
use crate::compare::same_value;
use crate::conversion::{to_boolean, to_length, to_string};
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Heap, Local, ObjectClass, Scope};
use crate::key::{PropertyKey, WellKnownSymbol};
use crate::object::{INLINE, array_create, get_prototype_from_constructor, ordinary_object_create};
use crate::operations::{
    call, create_data_property_or_throw, define_property_or_throw, get, is_callable, set,
};
use crate::realm::{Intrinsic, RealmId};
use crate::regexp::cache;
use crate::regexp::compile::{CompileError, compile};
use crate::regexp::matcher::{self, Input, Span, Stop};
use crate::regexp::program::{Budget, Program};
use crate::regexp::{Flags, parse};
use crate::value::{Kind, Ref, Value};

/// [[OriginalSource]]
const SOURCE: usize = 0;
/// [[OriginalFlags]]
const FLAGS: usize = 1;
/// Die Nummer des Programms; *undefined*, solange RegExpInitialize nicht
/// fertig ist.
const PROGRAM: usize = 2;

/// So viele Einheiten Arbeit der Engine kosten einen Arbeitsschritt. Ein
/// Befehl der Engine ist weniger Arbeit als einer des Interpreters. Die
/// Zahl ist gewählt und danach einmal gemessen (Release, ein Kern eines
/// Xeon mit 2,1 GHz, 14.11.2026): Ein Schritt des Interpreters dauerte
/// 94 bis 181 ns, ein so bezahlter Schritt der Engine 42 bis 150 ns, mit
/// Ergebnisobjekten bei vielen `exec` 353 ns.
pub(crate) const WORK_PER_STEP: u64 = 8;

static CONSTRUCTOR: Builtin = Builtin {
    name: "RegExp",
    length: 2,
    constructor: true,
    behaviour: regexp,
};

static CONSTRUCTOR_FUNCTIONS: [Builtin; 1] = [function("escape", 1, escape)];

static EXEC: Builtin = function("exec", 1, exec);

/// Die Accessoren in der Reihenfolge der Spezifikation, jeweils mit dem
/// Namen der Eigenschaft.
static GETTERS: [(&str, Builtin); 10] = [
    ("dotAll", function("get dotAll", 0, dot_all)),
    ("flags", function("get flags", 0, flags)),
    ("global", function("get global", 0, global)),
    ("hasIndices", function("get hasIndices", 0, has_indices)),
    ("ignoreCase", function("get ignoreCase", 0, ignore_case)),
    ("multiline", function("get multiline", 0, multiline)),
    ("source", function("get source", 0, source)),
    ("sticky", function("get sticky", 0, sticky)),
    ("unicode", function("get unicode", 0, unicode)),
    ("unicodeSets", function("get unicodeSets", 0, unicode_sets)),
];

static METHODS: [Builtin; 3] = [
    function("test", 1, test),
    function("toString", 0, to_string_method),
    // Anhang B.2.4.1
    function("compile", 2, compile_method),
];

const fn function(name: &'static str, length: u32, behaviour: Behaviour) -> Builtin {
    Builtin {
        name,
        length,
        constructor: false,
        behaviour,
    }
}

pub(super) fn create(agent: &mut Agent, scope: &Scope, realm: RealmId) -> Completion<()> {
    let object = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
    let prototype = ordinary_object_create(agent, scope, Some(object))?;
    let constructor = create_builtin_function(agent, scope, realm, &CONSTRUCTOR)?;
    link_constructor(agent, scope, constructor, prototype)?;
    define_functions(agent, scope, realm, constructor, &CONSTRUCTOR_FUNCTIONS)?;
    define_species(agent, scope, realm, constructor)?;
    // constructor, exec, dotAll, flags, global, hasIndices, ignoreCase,
    // @@match, @@matchAll, multiline, @@replace, @@search, source, @@split,
    // sticky, test, toString, unicode, unicodeSets (22.2.6.1 bis
    // 22.2.6.19), dann compile.
    let symbol = |agent: &mut Agent, symbol, builtin| {
        define_symbol_function(agent, scope, realm, prototype, symbol, builtin).map(|_| ())
    };
    define_functions(agent, scope, realm, prototype, std::slice::from_ref(&EXEC))?;
    let [
        dot_all,
        flags,
        global,
        has_indices,
        ignore_case,
        multiline,
        source,
        sticky,
        rest @ ..,
    ] = &GETTERS;
    for (name, getter) in [dot_all, flags, global, has_indices, ignore_case] {
        define_getter(agent, scope, realm, prototype, name, getter)?;
    }
    symbol(agent, WellKnownSymbol::Match, &symbols::MATCH)?;
    symbol(agent, WellKnownSymbol::MatchAll, &symbols::MATCH_ALL)?;
    define_getter(agent, scope, realm, prototype, multiline.0, &multiline.1)?;
    symbol(agent, WellKnownSymbol::Replace, &symbols::REPLACE)?;
    symbol(agent, WellKnownSymbol::Search, &symbols::SEARCH)?;
    define_getter(agent, scope, realm, prototype, source.0, &source.1)?;
    symbol(agent, WellKnownSymbol::Split, &symbols::SPLIT)?;
    define_getter(agent, scope, realm, prototype, sticky.0, &sticky.1)?;
    let (methods, legacy) = METHODS.split_at(2);
    define_functions(agent, scope, realm, prototype, methods)?;
    for (name, getter) in rest {
        define_getter(agent, scope, realm, prototype, name, getter)?;
    }
    define_functions(agent, scope, realm, prototype, legacy)?;
    agent.set_intrinsic(realm, Intrinsic::RegExp, constructor);
    agent.set_intrinsic(realm, Intrinsic::RegExpPrototype, prototype);
    let iterator_prototype = agent.intrinsic(scope, realm, Intrinsic::IteratorPrototype);
    let iterator = ordinary_object_create(agent, scope, Some(iterator_prototype))?;
    define_functions(agent, scope, realm, iterator, &symbols::ITERATOR_FUNCTIONS)?;
    define_to_string_tag(agent, scope, iterator, "RegExp String Iterator")?;
    agent.set_intrinsic(realm, Intrinsic::RegExpStringIteratorPrototype, iterator);
    Ok(())
}

// Felder

fn object_of<'h>(heap: &'h Heap, value: Local<'_>) -> Option<Ref<'h>> {
    match heap.get(value).kind() {
        Kind::Object(object) => {
            (heap.object_class(object) == Some(ObjectClass::RegExp)).then_some(object)
        }
        _ => None,
    }
}

fn field<'s>(
    agent: &Agent,
    scope: &'s Scope,
    regexp: Local<'_>,
    index: usize,
) -> Option<Local<'s>> {
    let heap = agent.heap();
    let value = heap.regexp_field(object_of(heap, regexp)?, index)?;
    Some(heap.local(scope, value))
}

fn set_field(agent: &Agent, regexp: Local<'_>, index: usize, value: Value<'_>) {
    let heap = agent.heap();
    if let Some(object) = object_of(heap, regexp) {
        heap.set_regexp_field(object, index, value);
    }
}

/// Ob `value` ein Objekt mit [[RegExpMatcher]] ist.
pub(crate) fn has_matcher(agent: &Agent, value: Local<'_>) -> bool {
    object_of(agent.heap(), value).is_some()
}

/// [[OriginalSource]] und [[OriginalFlags]] eines fertigen regulären
/// Ausdrucks.
fn original<'s>(
    agent: &Agent,
    scope: &'s Scope,
    regexp: Local<'_>,
) -> Option<(Local<'s>, Local<'s>)> {
    let source = field(agent, scope, regexp, SOURCE)?;
    let flags = field(agent, scope, regexp, FLAGS)?;
    let ready = matches!(agent.heap().get(source).kind(), Kind::String(_));
    ready.then_some((source, flags))
}

/// Die Flags aus [[OriginalFlags]]; sie sind beim Anlegen geprüft.
fn original_flags(agent: &Agent, scope: &Scope, regexp: Local<'_>) -> Flags {
    field(agent, scope, regexp, FLAGS)
        .and_then(|flags| Flags::parse(&units_of(agent, flags)))
        .unwrap_or_default()
}

/// Die Flags, von denen das Programm abhängt, als Bits.
fn program_key(flags: Flags) -> u8 {
    [
        flags.ignore_case,
        flags.multiline,
        flags.dot_all,
        flags.unicode,
        flags.unicode_sets,
    ]
    .iter()
    .enumerate()
    .fold(0, |bits, (index, &flag)| bits | (u8::from(flag) << index))
}

// Übersetzen

/// Rechnet die Arbeit der Engine in Arbeitsschritte um und merkt sich,
/// warum es nicht weitergeht.
struct Meter<'a> {
    agent: &'a Agent,
    carry: u64,
    failure: Option<Abrupt>,
}

impl<'a> Meter<'a> {
    fn new(agent: &'a Agent) -> Meter<'a> {
        Meter {
            agent,
            carry: 0,
            failure: None,
        }
    }

    fn spend(&mut self, work: u64) -> bool {
        let total = self.carry.saturating_add(work);
        self.carry = total % WORK_PER_STEP;
        let steps = total / WORK_PER_STEP;
        if steps == 0 {
            return true;
        }
        match self.agent.spend_execution_steps(steps) {
            Ok(()) => true,
            Err(abrupt) => {
                self.failure = Some(abrupt);
                false
            }
        }
    }

    fn failure(self) -> Abrupt {
        self.failure.unwrap_or(Abrupt::OutOfMemory)
    }
}

/// Wie viele Bytes Rust außerhalb des Heaps belegen darf: was vom
/// Speicherbudget frei ist.
fn free_bytes(agent: &Agent) -> usize {
    let heap = agent.heap();
    heap.limit().saturating_sub(heap.used_bytes())
}

/// Liest und übersetzt ein Muster; `None` für einen SyntaxError.
fn translate(agent: &Agent, source: &[u16], flags: Flags) -> Completion<Option<Program>> {
    let Ok(pattern) = parse(source, flags) else {
        return Ok(None);
    };
    let mut meter = Meter::new(agent);
    let mut spend = |work| meter.spend(work);
    let budget = Budget {
        bytes: free_bytes(agent).min(cache::LIMIT),
        spend: &mut spend,
    };
    match compile(&pattern, flags, budget) {
        Ok(program) => Ok(Some(program)),
        Err(CompileError::Unsupported) => Err(Abrupt::Unsupported(0)),
        Err(CompileError::TooLarge) => Err(Abrupt::OutOfMemory),
        Err(CompileError::Stopped) => Err(meter.failure()),
    }
}

/// Das Programm eines fertigen regulären Ausdrucks, wenn nötig neu
/// übersetzt.
fn program_of(agent: &mut Agent, scope: &Scope, regexp: Local<'_>) -> Completion<Rc<Program>> {
    let id = field(agent, scope, regexp, PROGRAM)
        .and_then(|id| agent.heap().get(id).as_number())
        .filter(|id| *id >= 0.0);
    let Some(id) = id else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    // Die Nummern zählen von 0 hoch und bleiben unter 2⁵³.
    if let Some(program) = agent.regexps.get(id as u64) {
        return Ok(program);
    }
    let Some((source, _)) = original(agent, scope, regexp) else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    let source = units_of(agent, source);
    let flags = original_flags(agent, scope, regexp);
    // Das Muster ließ sich schon einmal lesen.
    let Some(program) = translate(agent, &source, flags)? else {
        return Err(agent.throw_error(ErrorKind::SyntaxError));
    };
    let id = agent.regexps.insert(&source, program_key(flags), program);
    set_field(agent, regexp, PROGRAM, Value::number(id as f64));
    agent.regexps.get(id).ok_or(Abrupt::OutOfMemory)
}

// Anlegen

/// RegExpAlloc (Abschnitt 22.2.3.2); `current` ist der aktuelle Realm.
fn regexp_alloc<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    new_target: Local<'_>,
    current: RealmId,
) -> Completion<Local<'s>> {
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        new_target,
        Intrinsic::RegExpPrototype,
        current,
    )?;
    allocate(agent, scope, prototype)
}

fn allocate<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    prototype: Local<'_>,
) -> Completion<Local<'s>> {
    let shape = agent.empty_shape(scope);
    let object = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::RegExp,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let key = agent.key_from_units(scope, &units("lastIndex"))?;
    let value = undefined(agent, scope);
    let descriptor = PropertyDescriptor::data(value, true, false, false);
    define_property_or_throw(agent, scope, object, key, &descriptor)?;
    Ok(object)
}

/// RegExpInitialize (Abschnitt 22.2.3.3).
fn regexp_initialize<'s>(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'s>,
    pattern: Local<'_>,
    flags: Local<'_>,
) -> Completion<Local<'s>> {
    let pattern = if is_undefined(agent, pattern) {
        new_string(agent, scope, &[])?
    } else {
        to_string(agent, scope, pattern)?
    };
    let flags = if is_undefined(agent, flags) {
        new_string(agent, scope, &[])?
    } else {
        to_string(agent, scope, flags)?
    };
    let Some(parsed) = Flags::parse(&units_of(agent, flags)) else {
        return Err(agent.throw_error(ErrorKind::SyntaxError));
    };
    let source = units_of(agent, pattern);
    let key = program_key(parsed);
    let id = match agent.regexps.find(&source, key) {
        Some(id) => id,
        None => {
            let Some(program) = translate(agent, &source, parsed)? else {
                return Err(agent.throw_error(ErrorKind::SyntaxError));
            };
            agent.regexps.insert(&source, key, program)
        }
    };
    let heap = agent.heap();
    set_field(agent, object, SOURCE, heap.get(pattern));
    set_field(agent, object, FLAGS, heap.get(flags));
    set_field(agent, object, PROGRAM, Value::number(id as f64));
    let key = agent.key_from_units(scope, &units("lastIndex"))?;
    let zero = number(agent, scope, 0.0);
    set(agent, scope, object, key, zero, true)?;
    Ok(object)
}

/// RegExpCreate (Abschnitt 22.2.3.1) mit %RegExp% aus `realm`, dem
/// aktuellen Realm; `flags` darf *undefined* sein.
pub(crate) fn regexp_create<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    pattern: Local<'_>,
    flags: Local<'_>,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::RegExpPrototype);
    let object = allocate(agent, scope, prototype)?;
    regexp_initialize(agent, scope, object, pattern, flags)
}

/// IsRegExp (Abschnitt 7.2.6).
pub(crate) fn is_regexp(agent: &mut Agent, value: Local<'_>) -> Completion<bool> {
    if !is_object(agent, value) {
        return Ok(false);
    }
    agent.scope(|agent, scope| {
        let key = PropertyKey::Symbol(agent.well_known_symbol(scope, WellKnownSymbol::Match));
        let matcher = get(agent, scope, value, key)?;
        let heap = agent.heap();
        let found = heap.get(matcher);
        if !found.is_undefined() {
            return Ok(to_boolean(heap, found));
        }
        Ok(has_matcher(agent, value))
    })
}

/// RegExp ( pattern, flags ) (Abschnitt 22.2.4.1).
fn regexp<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let pattern = arguments.argument(agent, scope, 0);
    let mut flags = arguments.argument(agent, scope, 1);
    let pattern_is_regexp = is_regexp(agent, pattern)?;
    let new_target = match arguments.new_target {
        Some(new_target) => new_target,
        None => {
            let new_target = arguments.function;
            if pattern_is_regexp && is_undefined(agent, flags) {
                let key = agent.key_from_units(scope, &units("constructor"))?;
                let constructor = get(agent, scope, pattern, key)?;
                let heap = agent.heap();
                if same_value(heap, heap.get(new_target), heap.get(constructor)) {
                    return Ok(heap.local(scope, heap.get(pattern)));
                }
            }
            new_target
        }
    };
    let source = if let Some((source, original_flags)) = original(agent, scope, pattern) {
        if is_undefined(agent, flags) {
            flags = original_flags;
        }
        source
    } else if pattern_is_regexp {
        let key = agent.key_from_units(scope, &units("source"))?;
        let source = get(agent, scope, pattern, key)?;
        if is_undefined(agent, flags) {
            let key = agent.key_from_units(scope, &units("flags"))?;
            flags = get(agent, scope, pattern, key)?;
        }
        source
    } else {
        pattern
    };
    let object = regexp_alloc(agent, scope, new_target, arguments.realm)?;
    regexp_initialize(agent, scope, object, source, flags)
}

/// RegExp.escape ( string ) (Abschnitt 22.2.5.1) mit
/// EncodeForRegExpEscape (Abschnitt 22.2.5.1.1).
fn escape<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let string = arguments.argument(agent, scope, 0);
    if !matches!(agent.heap().get(string).kind(), Kind::String(_)) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let input = units_of(agent, string);
    let space = crate::regexp::charset::white_space();
    let mut escaped: Vec<u16> = Vec::new();
    let mut position = 0;
    while let Some((code_point, width)) =
        super::string::code_point_at(|index| input.get(index).copied(), position)
    {
        position = position.saturating_add(width);
        super::array::step(agent)?;
        let hex = |escaped: &mut Vec<u16>, value: u32, digits: usize| {
            escaped.extend(units(&format!("{value:0digits$x}")));
        };
        let ascii = u8::try_from(code_point).ok();
        if escaped.is_empty() && ascii.is_some_and(|c| c.is_ascii_alphanumeric()) {
            escaped.extend(units("\\x"));
            hex(&mut escaped, code_point, 2);
            continue;
        }
        let control = match ascii {
            Some(b'\t') => Some(b't'),
            Some(b'\n') => Some(b'n'),
            Some(0x0B) => Some(b'v'),
            Some(0x0C) => Some(b'f'),
            Some(b'\r') => Some(b'r'),
            _ => None,
        };
        if ascii.is_some_and(|c| b"^$\\.*+?()[]{}|/".contains(&c)) {
            escaped.push(0x5C);
            escaped.push(u16::from(ascii.unwrap_or(0)));
        } else if let Some(control) = control {
            escaped.push(0x5C);
            escaped.push(u16::from(control));
        } else if ascii.is_some_and(|c| b",-=<>#&!%:;@~'`\"".contains(&c))
            || space.contains(code_point)
            || (0xD800..=0xDFFF).contains(&code_point)
        {
            if code_point <= 0xFF {
                escaped.extend(units("\\x"));
                hex(&mut escaped, code_point, 2);
            } else {
                let mut encoded = Vec::new();
                super::string::push_code_point(&mut encoded, code_point);
                for unit in encoded {
                    escaped.extend(units("\\u"));
                    hex(&mut escaped, u32::from(unit), 4);
                }
            }
        } else {
            super::string::push_code_point(&mut escaped, code_point);
        }
        // Die Kopie liegt außerhalb des Heaps und darf nicht größer werden
        // als das, was vom Speicherbudget frei ist.
        if escaped.len().saturating_mul(2) > free_bytes(agent) {
            return Err(Abrupt::OutOfMemory);
        }
    }
    new_string(agent, scope, &escaped)
}

// Abgleich

/// Eine Zeichenkette im Heap als Eingabe für den Abgleich, ohne Kopie.
struct HeapInput<'h> {
    heap: &'h Heap,
    string: Ref<'h>,
}

impl Input for HeapInput<'_> {
    fn len(&self) -> usize {
        self.heap.string_len(self.string)
    }

    fn unit(&self, index: usize) -> Option<u16> {
        self.heap.string_unit(self.string, index)
    }
}

/// Der Abgleich ab `last_index`: die Bereiche aller Gruppen oder `None`.
fn run(
    agent: &Agent,
    program: &Program,
    string: Local<'_>,
    last_index: usize,
    sticky: bool,
) -> Completion<Option<Vec<Option<Span>>>> {
    let heap = agent.heap();
    let Kind::String(string) = heap.get(string).kind() else {
        return Ok(None);
    };
    let input = HeapInput { heap, string };
    let mut meter = Meter::new(agent);
    let mut spend = |work| meter.spend(work);
    let budget = Budget {
        bytes: free_bytes(agent),
        spend: &mut spend,
    };
    match matcher::execute(program, &input, last_index, sticky, budget) {
        Ok(found) => Ok(found),
        Err(Stop::Memory) => Err(Abrupt::OutOfMemory),
        Err(Stop::Stopped) => Err(meter.failure()),
    }
}

/// Der Teil der Zeichenkette in `span`.
fn slice<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    string: Local<'_>,
    span: Span,
) -> Completion<Local<'s>> {
    let (start, end) = (span.0 as usize, span.1 as usize);
    super::string::substring(agent, scope, string, start, end)
}

/// RegExpBuiltinExec (Abschnitt 22.2.7.2); `realm` ist der aktuelle Realm.
fn regexp_builtin_exec<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    regexp: Local<'_>,
    string: Local<'_>,
) -> Completion<Local<'s>> {
    let length = super::string::length_of(agent, string);
    let key = agent.key_from_units(scope, &units("lastIndex"))?;
    let value = get(agent, scope, regexp, key)?;
    let mut last_index = to_length(agent, scope, value)?;
    let flags = original_flags(agent, scope, regexp);
    let (global, sticky) = (flags.global, flags.sticky);
    if !global && !sticky {
        last_index = 0.0;
    }
    let program = program_of(agent, scope, regexp)?;
    // ToLength liefert eine ganze Zahl bis 2⁵³ − 1; über der Länge passt
    // nichts.
    let found = if last_index > count(length) {
        None
    } else {
        run(agent, &program, string, last_index as usize, sticky)?
    };
    let Some(captures) = found else {
        if global || sticky {
            let key = agent.key_from_units(scope, &units("lastIndex"))?;
            let zero = number(agent, scope, 0.0);
            set(agent, scope, regexp, key, zero, true)?;
        }
        return Ok(agent.heap().local(scope, Value::NULL));
    };
    let Some(&Some((start, end))) = captures.first() else {
        return Ok(agent.heap().local(scope, Value::NULL));
    };
    if global || sticky {
        let key = agent.key_from_units(scope, &units("lastIndex"))?;
        let end = number(agent, scope, f64::from(end));
        set(agent, scope, regexp, key, end, true)?;
    }
    build_result(
        agent,
        scope,
        realm,
        &program,
        string,
        &captures,
        (start, end),
        flags.has_indices,
    )
}

/// Das Array, das RegExpBuiltinExec für einen Treffer liefert, mit
/// MakeMatchIndicesIndexPairArray (Abschnitt 22.2.7.8) für `d`.
#[allow(clippy::too_many_arguments)]
fn build_result<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    program: &Program,
    string: Local<'_>,
    captures: &[Option<Span>],
    whole: Span,
    has_indices: bool,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, realm, Intrinsic::ArrayPrototype);
    let count = u64::try_from(captures.len()).unwrap_or(u64::MAX);
    let array = array_create(agent, scope, count, Some(prototype))?;
    let index = number(agent, scope, f64::from(whole.0));
    let key = agent.key_from_units(scope, &units("index"))?;
    create_data_property_or_throw(agent, scope, array, key, index)?;
    let key = agent.key_from_units(scope, &units("input"))?;
    create_data_property_or_throw(agent, scope, array, key, string)?;
    let matched = slice(agent, scope, string, whole)?;
    create_data_property_or_throw(agent, scope, array, PropertyKey::Index(0), matched)?;
    let has_groups = program.names.iter().any(Option::is_some);
    let groups = if has_groups {
        ordinary_object_create(agent, scope, None)?
    } else {
        undefined(agent, scope)
    };
    let key = agent.key_from_units(scope, &units("groups"))?;
    create_data_property_or_throw(agent, scope, array, key, groups)?;
    // Der Name jeder Gruppe, unter dem sie in `groups` steht; bei gleichen
    // Namen nur die erste, die passt, sonst die letzte.
    let mut group_names: Vec<Option<&[u16]>> = Vec::new();
    let mut matched_names: Vec<&[u16]> = Vec::new();
    for (number, capture) in captures.iter().enumerate().skip(1) {
        super::array::step(agent)?;
        let value = match capture {
            Some(span) => slice(agent, scope, string, *span)?,
            None => undefined(agent, scope),
        };
        let index = u32::try_from(number).map_err(|_| Abrupt::OutOfMemory)?;
        create_data_property_or_throw(agent, scope, array, PropertyKey::Index(index), value)?;
        let name = program
            .names
            .get(number.saturating_sub(1))
            .and_then(Option::as_deref);
        let Some(name) = name else {
            group_names.push(None);
            continue;
        };
        if matched_names.contains(&name) {
            group_names.push(None);
            continue;
        }
        if capture.is_some() {
            matched_names.push(name);
        }
        let key = agent.key_from_units(scope, name)?;
        create_data_property_or_throw(agent, scope, groups, key, value)?;
        group_names.push(Some(name));
    }
    if !has_indices {
        return Ok(array);
    }
    let indices = array_create(agent, scope, count, Some(prototype))?;
    let index_groups = if has_groups {
        ordinary_object_create(agent, scope, None)?
    } else {
        undefined(agent, scope)
    };
    let key = agent.key_from_units(scope, &units("groups"))?;
    create_data_property_or_throw(agent, scope, indices, key, index_groups)?;
    for (number, capture) in captures.iter().enumerate() {
        super::array::step(agent)?;
        let pair = match capture {
            // GetMatchIndexPair (Abschnitt 22.2.7.7)
            Some((start, end)) => {
                let start = number_local(agent, scope, *start);
                let end = number_local(agent, scope, *end);
                crate::operations::create_array_from_list(agent, scope, realm, &[start, end])?
            }
            None => undefined(agent, scope),
        };
        let index = u32::try_from(number).map_err(|_| Abrupt::OutOfMemory)?;
        create_data_property_or_throw(agent, scope, indices, PropertyKey::Index(index), pair)?;
        let name = number
            .checked_sub(1)
            .and_then(|at| group_names.get(at))
            .copied()
            .flatten();
        if let Some(name) = name {
            let key = agent.key_from_units(scope, name)?;
            create_data_property_or_throw(agent, scope, index_groups, key, pair)?;
        }
    }
    let key = agent.key_from_units(scope, &units("indices"))?;
    create_data_property_or_throw(agent, scope, array, key, indices)?;
    Ok(array)
}

fn number_local<'s>(agent: &Agent, scope: &'s Scope, value: u32) -> Local<'s> {
    number(agent, scope, f64::from(value))
}

/// RegExpExec (Abschnitt 22.2.7.1); `realm` ist der aktuelle Realm.
pub(crate) fn regexp_exec<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    regexp: Local<'_>,
    string: Local<'_>,
) -> Completion<Local<'s>> {
    let key = agent.key_from_units(scope, &units("exec"))?;
    let exec = get(agent, scope, regexp, key)?;
    if is_callable(agent.heap(), agent.heap().get(exec)) {
        let result = call(agent, scope, exec, regexp, &[string])?;
        let value = agent.heap().get(result);
        if !value.is_null() && !matches!(value.kind(), Kind::Object(_)) {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        return Ok(result);
    }
    if !has_matcher(agent, regexp) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    regexp_builtin_exec(agent, scope, realm, regexp, string)
}

// %RegExp.prototype%

/// RegExp.prototype.exec ( string ) (Abschnitt 22.2.6.2).
fn exec<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let regexp = arguments.this;
    if !has_matcher(agent, regexp) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let string = arguments.argument(agent, scope, 0);
    let string = to_string(agent, scope, string)?;
    regexp_builtin_exec(agent, scope, arguments.realm, regexp, string)
}

/// RegExpHasFlag (Abschnitt 22.2.6.4.1).
fn has_flag<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    flag: u8,
) -> Completion<Local<'s>> {
    let regexp = arguments.this;
    if !is_object(agent, regexp) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let Some((_, flags)) = original(agent, scope, regexp) else {
        let undefined = undefined(agent, scope);
        return prototype_or_throw(agent, scope, arguments, undefined);
    };
    let found = units_of(agent, flags).contains(&u16::from(flag));
    Ok(agent.heap().local(scope, Value::boolean(found)))
}

/// Was die Accessoren für %RegExp.prototype% selbst liefern; für andere
/// Objekte ohne die Felder ein TypeError.
fn prototype_or_throw<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
    value: Local<'s>,
) -> Completion<Local<'s>> {
    let prototype = agent.intrinsic(scope, arguments.realm, Intrinsic::RegExpPrototype);
    let heap = agent.heap();
    if same_value(heap, heap.get(arguments.this), heap.get(prototype)) {
        return Ok(value);
    }
    Err(agent.throw_error(ErrorKind::TypeError))
}

macro_rules! flag_getters {
    ($($name:ident $flag:literal,)*) => {
        $(
            fn $name<'s>(
                agent: &mut Agent,
                scope: &'s Scope,
                arguments: &BuiltinCall<'_, '_>,
            ) -> Completion<Local<'s>> {
                has_flag(agent, scope, arguments, $flag)
            }
        )*
    };
}

flag_getters! {
    dot_all b's',
    global b'g',
    has_indices b'd',
    ignore_case b'i',
    multiline b'm',
    sticky b'y',
    unicode b'u',
    unicode_sets b'v',
}

/// get RegExp.prototype.flags (Abschnitt 22.2.6.4).
fn flags<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let regexp = arguments.this;
    if !is_object(agent, regexp) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let mut result = Vec::new();
    for (name, flag) in [
        ("hasIndices", b'd'),
        ("global", b'g'),
        ("ignoreCase", b'i'),
        ("multiline", b'm'),
        ("dotAll", b's'),
        ("unicode", b'u'),
        ("unicodeSets", b'v'),
        ("sticky", b'y'),
    ] {
        let key = agent.key_from_units(scope, &units(name))?;
        let value = get(agent, scope, regexp, key)?;
        if to_boolean(agent.heap(), agent.heap().get(value)) {
            result.push(u16::from(flag));
        }
    }
    new_string(agent, scope, &result)
}

/// get RegExp.prototype.source (Abschnitt 22.2.6.13).
fn source<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let regexp = arguments.this;
    if !is_object(agent, regexp) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let Some((source, _)) = original(agent, scope, regexp) else {
        let empty = new_string(agent, scope, &units("(?:)"))?;
        return prototype_or_throw(agent, scope, arguments, empty);
    };
    let escaped = escape_pattern(&units_of(agent, source));
    new_string(agent, scope, &escaped)
}

/// EscapeRegExpPattern (Abschnitt 22.2.6.13.1). Die leere Zeichenkette wird
/// `(?:)`. Ein `/`, das der Lexer als Ende des Literals läse, und jedes
/// Zeilenende werden zu Escapes; der Lexer kennt Klassen dabei nur ohne
/// Verschachtelung (*RegularExpressionClass*, Abschnitt 12.9.5), so wird
/// auch hier gezählt. Alles andere bleibt, wie es ist.
fn escape_pattern(source: &[u16]) -> Vec<u16> {
    if source.is_empty() {
        return units("(?:)");
    }
    let mut result = Vec::with_capacity(source.len());
    let mut class = false;
    let mut escaped = false;
    for &unit in source {
        let line_terminator = match unit {
            0x0A => Some("n"),
            0x0D => Some("r"),
            0x2028 => Some("u2028"),
            0x2029 => Some("u2029"),
            _ => None,
        };
        if let Some(name) = line_terminator {
            if !escaped {
                result.push(0x5C);
            }
            result.extend(units(name));
            escaped = false;
            continue;
        }
        if escaped {
            result.push(unit);
            escaped = false;
            continue;
        }
        match unit {
            0x5C => escaped = true,
            0x5B => class = true,
            0x5D => class = false,
            0x2F if !class => result.push(0x5C),
            _ => {}
        }
        result.push(unit);
    }
    result
}

/// RegExp.prototype.test ( S ) (Abschnitt 22.2.6.16).
fn test<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let regexp = arguments.this;
    if !is_object(agent, regexp) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let string = arguments.argument(agent, scope, 0);
    let string = to_string(agent, scope, string)?;
    let found = regexp_exec(agent, scope, arguments.realm, regexp, string)?;
    let found = !agent.heap().get(found).is_null();
    Ok(agent.heap().local(scope, Value::boolean(found)))
}

/// RegExp.prototype.toString ( ) (Abschnitt 22.2.6.17).
fn to_string_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let regexp = arguments.this;
    if !is_object(agent, regexp) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let mut result = vec![0x2F];
    for (name, slash) in [("source", true), ("flags", false)] {
        let key = agent.key_from_units(scope, &units(name))?;
        let value = get(agent, scope, regexp, key)?;
        let value = to_string(agent, scope, value)?;
        super::string::push_units(agent, &mut result, &units_of(agent, value))?;
        if slash {
            result.push(0x2F);
        }
    }
    new_string(agent, scope, &result)
}

/// RegExp.prototype.compile ( pattern, flags ) (Anhang B.2.4.1).
fn compile_method<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let regexp = arguments.this;
    if !has_matcher(agent, regexp) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    let mut pattern = arguments.argument(agent, scope, 0);
    let mut flags = arguments.argument(agent, scope, 1);
    if has_matcher(agent, pattern) {
        if !is_undefined(agent, flags) {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
        if let Some((source, original_flags)) = original(agent, scope, pattern) {
            pattern = source;
            flags = original_flags;
        }
    }
    let heap = agent.heap();
    let regexp = heap.local(scope, heap.get(regexp));
    regexp_initialize(agent, scope, regexp, pattern, flags)
}

/// AdvanceStringIndex (Abschnitt 22.2.7.3) für `index` in `string`.
fn advance_string_index(agent: &Agent, string: Local<'_>, index: f64, unicode: bool) -> f64 {
    if !unicode || index >= count(super::string::length_of(agent, string)) {
        return index + 1.0;
    }
    // Unter der Länge ist `index` eine Stelle der Zeichenkette.
    let width = super::string::code_point_at(|at| unit_at(agent, string, at), index as usize)
        .map_or(1, |(_, width)| width);
    index + width as f64
}

/// Ob `units` `u` oder `v` enthält.
fn full_unicode(units: &[u16]) -> bool {
    units.contains(&u16::from(b'u')) || units.contains(&u16::from(b'v'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_patterns_lex_as_literals() {
        let check = |source: &str, expected: &str| {
            assert_eq!(
                String::from_utf16_lossy(&escape_pattern(&units(source))),
                expected,
                "{source:?}"
            );
        };
        check("", "(?:)");
        check("a/b", "a\\/b");
        check("a\\/b", "a\\/b");
        check("[/]", "[/]");
        check("[\\]/]/", "[\\]/]\\/");
        check("[[a]/]", "[[a]\\/]");
        check("a\nb\\\nc", "a\\nb\\nc");
        check("\r\u{2028}\u{2029}", "\\r\\u2028\\u2029");
        check("\\\\/", "\\\\\\/");
    }
}
