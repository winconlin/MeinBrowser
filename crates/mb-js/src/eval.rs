//! Quelltext zur Laufzeit: PerformEval (Abschnitt 19.2.1.1) für direktes
//! und indirektes `eval` und CreateDynamicFunction (Abschnitt 20.2.1.1.1)
//! für `Function` und seine Verwandten.
//!
//! Ein direktes `eval` übersetzt seinen Code mit der Stelle des Aufrufs
//! ([`EvalSite`]): Der Code sieht deren Bindungen über den aktuellen
//! Kontext des Aufrufers, `this` und `new.target` wie eine Pfeilfunktion.
//! Er läuft als eigener Rahmen im Befehl des Aufrufs. Ein indirektes sieht
//! nur die globale Umgebung seines Realms.
//!
//! Lesen und Übersetzen laufen ohne Bytecodeschritte; damit ein Script
//! nicht beliebig viel Arbeit je Schritt anstößt, kostet jeder Quelltext
//! vorab [`BASE_STEPS`] Schritte und [`STEPS_PER_UNIT`] Schritte je
//! Codeeinheit, die gelesen wird.

#![forbid(unsafe_code)]

use std::rc::Rc;

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::ast::Node;
use crate::builtin::BuiltinCall;
use crate::bytecode::{EvalSite, FunctionTemplate};
use crate::compare::same_value;
use crate::compiler::{CompileError, EvalError, compile_eval};
use crate::conversion::to_string;
use crate::function::instantiate_function;
use crate::heap::{Local, Scope};
use crate::interpreter::execute_eval;
use crate::object::{get_prototype_from_constructor, set_prototype_of};
use crate::parser::{DynamicFunctionKind, parse_dynamic_function, parse_eval};
use crate::realm::{Intrinsic, RealmId};
use crate::unit::Unit;
use crate::value::{Kind, Value};

/// So viele Schritte der Arbeitsquote kostet das Lesen und Übersetzen einer
/// Codeeinheit Quelltext.
///
/// [`STEPS_PER_UNIT`] und [`BASE_STEPS`] sind gemessen (Release, ein Kern
/// eines Xeon mit 2,1 GHz, 21.11.2026), mit Schleifen, die eine Quote von
/// 20.000.000 Schritten aufbrauchen: Ein Schritt des Interpreters dauerte
/// 19 ns in einer leeren Schleife und 108 ns mit einer Addition. Mit dem
/// ersten Versuch, einem Schritt je 16 Codeeinheiten, dauerte ein Schritt
/// beim Lesen und Übersetzen von 1.000 Anweisungen, die nicht laufen,
/// 2,8 µs (etwa 175 ns je Codeeinheit) und mit `Function` 5,1 µs, denn
/// Parameter und Rumpf werden zweimal gelesen; ein `eval('1')` kostete ohne
/// Grundbetrag etwa 3 µs. Mit den jetzigen Werten dauerte ein so bezahlter
/// Schritt bei diesen Fällen 76 bis 116 ns, bei `eval('1')` 99 ns, bei 100
/// verschachtelten Arrays 125 ns, wenn sie nicht laufen. Langsamer waren
/// nur 1.000 `var` (415 ns in einer Funktion, 743 ns global): Die Suche in
/// einer Gestalt ist linear, und das Lesen einer Eigenschaft eines Objekts
/// mit 1.000 Eigenschaften kostet auch ohne `eval` 2 µs je Schritt.
pub(crate) const STEPS_PER_UNIT: u64 = 2;

/// So viele Schritte kostet jeder Quelltext zusätzlich, für das Anlegen von
/// Parser, Syntaxbaum, Code und Rahmen (gemessen wie [`STEPS_PER_UNIT`]).
pub(crate) const BASE_STEPS: u64 = 16;

/// Die Stelle eines direkten `eval` und der aktuelle Kontext dort.
pub(crate) struct DirectEval<'a, 'l> {
    pub(crate) site: &'a EvalSite,
    pub(crate) context: Local<'l>,
}

/// PerformEval (Abschnitt 19.2.1.1) im Realm `realm`: für ein direktes
/// `eval` der des Aufrufers, für ein indirektes der von %eval%. Ein
/// Argument, das keine Zeichenkette ist, kommt unverändert zurück. Ein
/// Fehler beim Lesen und ein Konflikt aus EvalDeclarationInstantiation
/// werfen einen SyntaxError.
pub(crate) fn perform_eval<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    argument: Local<'_>,
    direct: Option<DirectEval<'_, '_>>,
) -> Completion<Local<'s>> {
    let heap = agent.heap();
    let source = match heap.get(argument).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => return Ok(heap.local(scope, heap.get(argument))),
    };
    agent.with_default_execution_budget(|agent| {
        // Wie ein Aufruf: Verschachtelte `eval` zählen zum Stapelbudget.
        agent.enter_interpreter()?;
        let result = evaluate(agent, scope, realm, &source, direct.as_ref());
        agent.leave_interpreter();
        result
    })
}

fn evaluate<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    realm: RealmId,
    source: &[u16],
    direct: Option<&DirectEval<'_, '_>>,
) -> Completion<Local<'s>> {
    spend_source_steps(agent, source.len())?;
    let site = direct.map(|direct| direct.site);
    let options = site.map(EvalSite::options).unwrap_or_default();
    let Ok(ast) = parse_eval(source, &options) else {
        return Err(agent.throw_error(ErrorKind::SyntaxError));
    };
    let ast = Rc::new(ast);
    let code = match compile_eval(&ast, site) {
        Ok(code) => code,
        Err(EvalError::Conflict) => return Err(agent.throw_error(ErrorKind::SyntaxError)),
        // Der Code hat keine Lage im Script.
        Err(EvalError::Compile(CompileError::Unsupported(_))) => {
            return Err(Abrupt::Unsupported(0));
        }
        Err(EvalError::Compile(CompileError::TooLarge)) => {
            return Err(agent.throw_error(ErrorKind::RangeError));
        }
    };
    // Ein Abbruchsignal aus der Zeit des Übersetzens wirkt vor der ersten
    // Bindung.
    agent.spend_execution_steps(0)?;
    let (this, context) = match direct {
        // Der Code liest `this` aus dem Kontext.
        Some(direct) => (agent.heap().local(scope, Value::UNDEFINED), direct.context),
        None => (
            agent.global_object(scope, realm),
            agent.heap().local(scope, Value::UNDEFINED),
        ),
    };
    execute_eval(agent, scope, realm, &code, &Unit::Ast(ast), this, context)
}

/// [`BASE_STEPS`] und [`STEPS_PER_UNIT`] Schritte je Codeeinheit.
fn spend_source_steps(agent: &Agent, units: usize) -> Completion<()> {
    let units = u64::try_from(units).unwrap_or(u64::MAX);
    let steps = units
        .saturating_mul(STEPS_PER_UNIT)
        .saturating_add(BASE_STEPS);
    agent.spend_execution_steps(steps)
}

/// CreateDynamicFunction (Abschnitt 20.2.1.1.1) für den Konstruktor `call`
/// der Art `kind`. Die Funktion sieht nur die globale Umgebung des Realms
/// des Konstruktors; ihr Quelltext für `toString` ist der zusammengesetzte.
pub(crate) fn create_dynamic_function<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
    kind: DynamicFunctionKind,
) -> Completion<Local<'s>> {
    let new_target = call.new_target.unwrap_or(call.function);
    let fallback = match kind {
        DynamicFunctionKind::Normal => Intrinsic::FunctionPrototype,
        DynamicFunctionKind::Generator => Intrinsic::GeneratorFunctionPrototype,
        DynamicFunctionKind::Async => Intrinsic::AsyncFunctionPrototype,
        DynamicFunctionKind::AsyncGenerator => Intrinsic::AsyncGeneratorFunctionPrototype,
    };
    // Schritte 6 bis 10: erst die Parameter der Reihe nach, dann der Rumpf.
    let (body, parameters) = match call.arguments.split_last() {
        Some((&body, parameters)) => (Some(body), parameters),
        None => (None, &[][..]),
    };
    let mut parameter_units = Vec::new();
    for (index, &parameter) in parameters.iter().enumerate() {
        if index > 0 {
            parameter_units.push(u16::from(b','));
        }
        parameter_units.extend(string_units(agent, scope, parameter)?);
    }
    let body_units = match body {
        Some(body) => string_units(agent, scope, body)?,
        None => Vec::new(),
    };
    // Schritte 11 bis 13: *bodyParseString* und *sourceString*.
    let line_feed = u16::from(b'\n');
    let mut body_parse = Vec::with_capacity(body_units.len().saturating_add(2));
    body_parse.push(line_feed);
    body_parse.extend_from_slice(&body_units);
    body_parse.push(line_feed);
    let mut source: Vec<u16> = kind.prefix().encode_utf16().collect();
    source.extend(" anonymous(".encode_utf16());
    source.extend_from_slice(&parameter_units);
    source.push(line_feed);
    source.extend(") {".encode_utf16());
    source.extend_from_slice(&body_parse);
    source.push(u16::from(b'}'));
    // Parameter und Rumpf werden erst einzeln, dann im Ganzen gelesen.
    let units = source
        .len()
        .saturating_add(parameter_units.len())
        .saturating_add(body_parse.len());
    spend_source_steps(agent, units)?;
    let Ok(ast) = parse_dynamic_function(kind, &parameter_units, &body_parse, &source) else {
        return Err(agent.throw_error(ErrorKind::SyntaxError));
    };
    let function = match ast.body().first().map(|&item| ast.node(item)) {
        Some(&Node::FunctionDeclaration(function)) => function,
        _ => return Err(agent.throw_error(ErrorKind::SyntaxError)),
    };
    agent.spend_execution_steps(0)?;
    let prototype = get_prototype_from_constructor(agent, scope, new_target, fallback, call.realm)?;
    let template = FunctionTemplate {
        function,
        binds_name: false,
        outer: Rc::default(),
    };
    let shared = agent.add_shared_function(Unit::Ast(Rc::new(ast)), template)?;
    let name: Vec<u16> = "anonymous".encode_utf16().collect();
    let key = agent.key_from_units(scope, &name)?;
    let object = instantiate_function(agent, scope, call.realm, shared, key)?;
    let standard = agent.intrinsic(scope, call.realm, fallback);
    let heap = agent.heap();
    if !same_value(heap, heap.get(prototype), heap.get(standard)) {
        set_prototype_of(agent, object, Some(prototype))?;
    }
    Ok(object)
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
