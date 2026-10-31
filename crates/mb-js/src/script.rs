//! Scripts ausführen: ParseScript, ScriptEvaluation und
//! GlobalDeclarationInstantiation (Abschnitte 16.1.5 bis 16.1.7).

#![forbid(unsafe_code)]

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::compiler::{CompileError, Script, compile_script};
use crate::heap::{Local, Scope};
use crate::interpreter::execute;
use crate::parser::{SyntaxError, parse_script};
use crate::realm::RealmId;

/// Warum ein Script keinen Wert ergab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptError {
    /// Ein früher Fehler: Das Script lief nicht.
    Syntax(SyntaxError),
    /// Etwas, das der Übersetzer noch nicht kann, an dieser Lage im
    /// Quelltext. Das Script lief nicht.
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
        let ast = parse_script(source).map_err(ScriptError::Syntax)?;
        let script = match compile_script(&ast) {
            Ok(script) => script,
            Err(CompileError::Unsupported(position)) => {
                return Err(ScriptError::Unsupported(position));
            }
            Err(CompileError::TooLarge) => {
                return Err(ScriptError::Abrupt(self.throw_error(ErrorKind::RangeError)));
            }
        };
        self.script_evaluation(scope, realm, &script)
            .map_err(ScriptError::Abrupt)
    }

    /// ScriptEvaluation (Abschnitt 16.1.6). Der Ausführungskontext besteht
    /// bisher nur aus dem Realm und dem Rahmen des Interpreters; `this` ist
    /// das globale Objekt. Ein leerer Abschlusswert ist *undefined*, denn so
    /// beginnt das Register dafür.
    fn script_evaluation<'s>(
        &mut self,
        scope: &'s Scope,
        realm: RealmId,
        script: &Script,
    ) -> Completion<Local<'s>> {
        self.scope(|agent, inner| global_declaration_instantiation(agent, inner, realm, script))?;
        let this = self.global_object(scope, realm);
        execute(self, scope, realm, &script.code, this)
    }
}

/// GlobalDeclarationInstantiation (Abschnitt 16.1.7) ohne Funktionen, die
/// der Übersetzer noch ablehnt.
fn global_declaration_instantiation(
    agent: &mut Agent,
    scope: &Scope,
    realm: RealmId,
    script: &Script,
) -> Completion<()> {
    for (name, _) in &script.lexical_names {
        if agent.has_lexical_declaration(realm, name) {
            return Err(agent.throw_error(ErrorKind::SyntaxError));
        }
        let key = agent.key_from_units(scope, name)?;
        if agent.has_restricted_global_property(scope, realm, key)? {
            return Err(agent.throw_error(ErrorKind::SyntaxError));
        }
    }
    for name in &script.var_names {
        if agent.has_lexical_declaration(realm, name) {
            return Err(agent.throw_error(ErrorKind::SyntaxError));
        }
    }
    for name in &script.var_names {
        let key = agent.key_from_units(scope, name)?;
        if !agent.can_declare_global_var(scope, realm, key)? {
            return Err(agent.throw_error(ErrorKind::TypeError));
        }
    }
    for (name, constant) in &script.lexical_names {
        agent.create_lexical_binding(scope, realm, name, !constant)?;
    }
    for name in &script.var_names {
        let key = agent.key_from_units(scope, name)?;
        agent.create_global_var_binding(scope, realm, key, false)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
