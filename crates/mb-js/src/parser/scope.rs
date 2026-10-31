//! Gültigkeitsbereiche für die frühen Fehler zu Deklarationen: doppelte
//! lexikalische Namen, lexikalische Namen, die auch mit `var` deklariert
//! werden (*LexicallyDeclaredNames* und *VarDeclaredNames*), und Parameter,
//! die der Rumpf lexikalisch neu deklariert.
//!
//! Ein Name aus `var` gilt in allen Bereichen bis zum nächsten
//! Funktionsbereich; jeder davon merkt ihn sich, damit eine spätere
//! lexikalische Deklaration den Konflikt findet. Funktionsdeklarationen
//! zählen auf oberster Ebene von Funktionen und Scripts wie `var`, sonst
//! lexikalisch (Abschnitte 8.2.4 und 8.2.6). In Blöcken außerhalb des
//! strikten Modus dürfen gleichnamige Funktionsdeklarationen mehrfach
//! vorkommen (Abschnitt B.3.2), und `var` darf den einfachen Parameter von
//! `catch` wiederholen (Abschnitt B.3.4).

use std::collections::{BTreeMap, BTreeSet};

use super::{ErrorKind, Parser, Result};
use crate::ast::Atom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScopeKind {
    Script,
    Module,
    /// Der Rumpf einer Funktion mit ihren Parametern, auch eines
    /// `static`-Blocks.
    Function,
    Block,
    /// Der Parameter von `catch`; `simple`, wenn er ein Bezeichner ist.
    Catch {
        simple: bool,
    },
    /// Der Block von `catch`.
    CatchBody,
    /// Der Kopf von `for` mit `let`, `const` oder `using`.
    For,
    /// Der Block von `switch`.
    Switch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Binding {
    Lexical,
    /// Eine Funktionsdeklaration (nicht Generator oder async) in einem
    /// Block außerhalb des strikten Modus.
    SloppyFunction,
    CatchParameter,
}

#[derive(Debug)]
pub(super) struct Scope {
    pub(super) kind: ScopeKind,
    lexical: BTreeMap<Atom, Binding>,
    /// Namen aus `var`, die hier deklariert sind oder hindurchgehen.
    vars: BTreeSet<Atom>,
    /// Die Namen der Parameter eines Funktionsbereichs.
    parameters: BTreeSet<Atom>,
}

impl Scope {
    pub(super) fn new(kind: ScopeKind) -> Scope {
        Scope {
            kind,
            lexical: BTreeMap::new(),
            vars: BTreeSet::new(),
            parameters: BTreeSet::new(),
        }
    }

    /// Ob hier der Bereich der Namen aus `var` endet.
    fn is_var_scope(&self) -> bool {
        matches!(
            self.kind,
            ScopeKind::Script | ScopeKind::Module | ScopeKind::Function
        )
    }

    /// Ob ein Name im Bereich deklariert ist, lexikalisch oder mit `var`.
    pub(super) fn declares(&self, name: Atom) -> bool {
        self.lexical.contains_key(&name) || self.vars.contains(&name)
    }
}

impl Parser<'_> {
    pub(super) fn push_scope(&mut self, kind: ScopeKind) {
        self.scopes.push(Scope::new(kind));
    }

    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Deklariert einen lexikalischen Namen im innersten Bereich.
    pub(super) fn declare_lexical(
        &mut self,
        name: Atom,
        binding: Binding,
        position: usize,
    ) -> Result<()> {
        let count = self.scopes.len();
        let Some(scope) = self.scopes.last_mut() else {
            return Ok(());
        };
        let conflict = match scope.lexical.get(&name) {
            Some(Binding::SloppyFunction) => binding != Binding::SloppyFunction,
            Some(_) => true,
            None => false,
        } || scope.vars.contains(&name)
            || scope.parameters.contains(&name);
        if conflict {
            return Err(self.error_at(ErrorKind::Redeclaration, position));
        }
        scope.lexical.insert(name, binding);
        // Der Block von `catch` darf den Parameter nicht neu deklarieren.
        if scope.kind == ScopeKind::CatchBody
            && let Some(parent) = count
                .checked_sub(2)
                .and_then(|index| self.scopes.get(index))
            && parent.lexical.contains_key(&name)
        {
            return Err(self.error_at(ErrorKind::Redeclaration, position));
        }
        Ok(())
    }

    /// Deklariert einen Namen mit `var`.
    pub(super) fn declare_var(&mut self, name: Atom, position: usize) -> Result<()> {
        for index in (0..self.scopes.len()).rev() {
            let Some(scope) = self.scopes.get_mut(index) else {
                break;
            };
            match (scope.kind, scope.lexical.get(&name)) {
                (_, None) | (ScopeKind::Catch { simple: true }, Some(Binding::CatchParameter)) => {}
                _ => return Err(self.error_at(ErrorKind::Redeclaration, position)),
            }
            scope.vars.insert(name);
            if scope.is_var_scope() {
                break;
            }
        }
        Ok(())
    }

    /// Deklariert den Namen einer Funktionsdeklaration. `plain` ist eine
    /// Funktion ohne `*` und `async`.
    pub(super) fn declare_function(
        &mut self,
        name: Atom,
        plain: bool,
        position: usize,
    ) -> Result<()> {
        let kind = self.scopes.last().map(|scope| scope.kind);
        match kind {
            Some(ScopeKind::Script | ScopeKind::Function) => {
                let Some(scope) = self.scopes.last_mut() else {
                    return Ok(());
                };
                if scope.lexical.contains_key(&name) {
                    return Err(self.error_at(ErrorKind::Redeclaration, position));
                }
                scope.vars.insert(name);
                Ok(())
            }
            Some(ScopeKind::Block | ScopeKind::CatchBody | ScopeKind::Switch)
                if plain && !self.strict() =>
            {
                self.declare_lexical(name, Binding::SloppyFunction, position)
            }
            _ => self.declare_lexical(name, Binding::Lexical, position),
        }
    }

    /// Deklariert einen Parameter im innersten Bereich, einem
    /// Funktionsbereich. Doppelte Namen prüft der Aufrufer.
    pub(super) fn declare_parameter(&mut self, name: Atom) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.parameters.insert(name);
        }
    }

    /// Ob der äußerste Bereich (Script oder Modul) den Namen deklariert.
    pub(super) fn top_level_declares(&self, name: Atom) -> bool {
        self.scopes
            .first()
            .is_some_and(|scope| scope.declares(name))
    }
}
