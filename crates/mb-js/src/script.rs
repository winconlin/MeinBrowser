//! Scripts ausführen: ParseScript, ScriptEvaluation und
//! GlobalDeclarationInstantiation (Abschnitte 16.1.5 bis 16.1.7).

#![forbid(unsafe_code)]

use std::rc::Rc;

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::ast::Ast;
use crate::bytecode::FunctionTemplate;
use crate::compiler::{CompileError, Script, compile_script};
use crate::function::instantiate_function;
use crate::heap::{Local, Scope};
use crate::interpreter::execute_script;
use crate::parser::{SyntaxError, parse_script};
use crate::realm::RealmId;

/// Warum ein Script keinen Wert ergab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptError {
    /// Ein früher Fehler: Das Script lief nicht.
    Syntax(SyntaxError),
    /// Etwas, das der Übersetzer noch nicht kann, an dieser Lage im
    /// Quelltext. Steht es im Script selbst, lief es nicht; steht es in
    /// einer Funktion, endete es beim ersten Aufruf der Funktion, denn
    /// Funktionen werden erst dann übersetzt.
    Unsupported(usize),
    /// Das Script endete abrupt. Eine Ausnahme hält der Agent, siehe
    /// [`Agent::take_exception`].
    Abrupt(Abrupt),
}

impl Agent {
    /// Liest, übersetzt und führt ein Script im Realm `realm` aus und gibt
    /// seinen Abschlusswert zurück. Ein Script, das zu viel Bytecode,
    /// Register oder Konstanten bräuchte, wirft einen RangeError.
    pub fn evaluate_script<'s>(
        &mut self,
        scope: &'s Scope,
        realm: RealmId,
        source: &[u16],
    ) -> Result<Local<'s>, ScriptError> {
        let ast = Rc::new(parse_script(source).map_err(ScriptError::Syntax)?);
        let script = match compile_script(&ast) {
            Ok(script) => script,
            Err(CompileError::Unsupported(position)) => {
                return Err(ScriptError::Unsupported(position));
            }
            Err(CompileError::TooLarge) => {
                return Err(ScriptError::Abrupt(self.throw_error(ErrorKind::RangeError)));
            }
        };
        self.script_evaluation(scope, realm, &ast, &script)
            .map_err(|abrupt| match abrupt {
                Abrupt::Unsupported(position) => ScriptError::Unsupported(position),
                abrupt => ScriptError::Abrupt(abrupt),
            })
    }

    /// ScriptEvaluation (Abschnitt 16.1.6). Der Ausführungskontext besteht
    /// bisher nur aus dem Realm und dem Rahmen des Interpreters; `this` ist
    /// das globale Objekt. Ein leerer Abschlusswert ist *undefined*, denn so
    /// beginnt das Register dafür.
    fn script_evaluation<'s>(
        &mut self,
        scope: &'s Scope,
        realm: RealmId,
        unit: &Rc<Ast>,
        script: &Script,
    ) -> Completion<Local<'s>> {
        let flags = self.scope(|agent, inner| {
            global_declaration_instantiation(agent, inner, realm, unit, script)
        })?;
        let this = self.global_object(scope, realm);
        execute_script(self, scope, realm, &script.code, unit, this, &flags)
    }
}

/// GlobalDeclarationInstantiation (Abschnitt 16.1.7) mit den Schritten aus
/// Anhang B.3.2.2. Die Funktionen auf oberster Ebene sehen außer ihren
/// eigenen Bindungen nur die globale Umgebung. Gibt die Register der
/// Funktionen in Blöcken zurück, die die globale Bindung setzen.
fn global_declaration_instantiation(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    unit: &Rc<Ast>,
    script: &Script,
) -> Completion<Vec<u32>> {
    for (name, _) in &script.lexical_names {
        if agent.has_lexical_declaration(realm, name) {
            return Err(agent.throw_error(ErrorKind::SyntaxError));
        }
        let key = agent.key_from_units(scope, name)?;
        if agent.has_restricted_global_property(scope, realm, key)? {
            return Err(agent.throw_error(ErrorKind::SyntaxError));
        }
    }
    let function_names = script.functions.iter().map(|(name, _)| name);
    for name in function_names.chain(&script.var_names) {
        if agent.has_lexical_declaration(realm, name) {
            return Err(agent.throw_error(ErrorKind::SyntaxError));
        }
    }
    // Schritt 10 geht die Funktionen von hinten durch.
    for (name, _) in script.functions.iter().rev() {
        let key = agent.key_from_units(scope, name)?;
        if !agent.can_declare_global_function(scope, realm, key)? {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
    }
    for name in &script.var_names {
        let key = agent.key_from_units(scope, name)?;
        if !agent.can_declare_global_var(scope, realm, key)? {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
    }
    // Anhang B.3.2.2: Eine Funktion in einem Block setzt auch die globale
    // Bindung, wenn es keine lexikalische gibt und eine aus `var` entstehen
    // darf. Neu angelegt wird sie nur, wenn der Name nicht schon aus einer
    // Funktion oder `var` stammt (declaredFunctionOrVarNames).
    let mut declared: Vec<&[u16]> = script
        .functions
        .iter()
        .map(|(name, _)| name.as_slice())
        .chain(script.var_names.iter().map(Vec::as_slice))
        .collect();
    let mut flags = Vec::new();
    for (name, flag) in &script.hoisted_functions {
        if agent.has_lexical_declaration(realm, name) {
            continue;
        }
        let key = agent.key_from_units(scope, name)?;
        if !agent.can_declare_global_var(scope, realm, key)? {
            continue;
        }
        if !declared.contains(&name.as_slice()) {
            agent.create_global_var_binding(scope, realm, key, false)?;
            declared.push(name);
        }
        flags.push(*flag);
    }
    for (name, constant) in &script.lexical_names {
        agent.create_lexical_binding(scope, realm, name, !constant)?;
    }
    for &(ref name, function) in &script.functions {
        agent.scope(|agent, inner| -> Completion<()> {
            let template = FunctionTemplate {
                function,
                binds_name: false,
                outer: Rc::default(),
            };
            let shared = agent.add_shared_function(Rc::clone(unit), template)?;
            let key = agent.key_from_units(inner, name)?;
            let object = instantiate_function(agent, inner, realm, shared, key)?;
            agent.create_global_function_binding(inner, realm, key, object, false)
        })?;
    }
    for name in &script.var_names {
        let key = agent.key_from_units(scope, name)?;
        agent.create_global_var_binding(scope, realm, key, false)?;
    }
    Ok(flags)
}

#[cfg(test)]
mod tests;
