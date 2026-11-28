//! Vorgeprüfte Funktionsrümpfe (`docs/architektur.md`, Abschnitt 4.9,
//! Stufe 1).
//!
//! Es gibt keine zweite Grammatik: Der Parser liest einen Rumpf wie immer,
//! mit allen frühen Fehlern. Erst an seinem Ende entscheidet er, ob er die
//! Knoten behält. Verwirft er sie, kürzt er die Tabellen des Baums auf ihre
//! Länge vor `{`; nur die Atome bleiben, damit ein nachgelesener Rumpf
//! dieselben Nummern bekommt wie beim ersten Lesen. Der Übersetzer
//! vergleicht Namen über ihre Atome, auch mit den Ketten der umgebenden
//! Funktionen ([`crate::bytecode::Enclosing`]).
//!
//! Was bleibt, steht in [`LazyBody`]: die Lage von `{`, der Kontext im
//! Rumpf und die Parameter für das Nachlesen und die Analyse für den
//! Übersetzer ([`crate::ast::Captures`]), die der Rumpf den Funktionen außen
//! zuträgt. [`Ast::expand`] liest den Rumpf später ab `{` nach und hängt
//! seine Knoten an denselben Baum an. Seine inneren Funktionen prüft es
//! dabei wieder nur vor.
//!
//! Welche Rümpfe der Parser verwirft, sagt [`PreCheck`]: keine, die kleiner
//! sind als eine Grenze, denn für sie lohnt das Nachlesen nicht, und keine,
//! die tiefer verschachtelt sind als eine andere. Diese zweite Grenze
//! begrenzt den Stapel beim Nachlesen; das geschieht beim ersten Aufruf,
//! und der kann tief in verschachtelten Läufen des Interpreters liegen.
//! Mit [`PreCheck::immediate`] behält er auch Funktionsausdrücke direkt
//! hinter `(` oder `!`, wie `(function () { … })()`: Solche werden meist
//! sofort aufgerufen, und ein Nachlesen läse sie nur ein zweites Mal.
//! Feldinitialisierer, `static`-Blöcke und Pfeilfunktionen mit einem
//! Ausdruck als Rumpf behält er immer, ebenso alles Übrige außerhalb von
//! Funktionsrümpfen.

use std::rc::Rc;

use super::pattern::Names;
use super::scope::{Scope, ScopeKind};
use super::{ErrorKind, FunctionState, MAX_DEPTH, Parser, PrivateScope, Result, SyntaxError};
use crate::ast::{Ast, Atom, FunctionId, FunctionKind, LazyBody, List, NodeId};
use crate::lexer::Lexer;

/// Welche Funktionsrümpfe der Parser nur vorprüft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreCheck {
    /// Rümpfe mit weniger Knoten behält er ganz.
    pub min_nodes: usize,
    /// Rümpfe, deren Inhalt mehr Ebenen tief verschachtelt ist, behält er
    /// ganz ([`MAX_DEPTH`] zählt dieselben Ebenen).
    pub max_depth: u32,
    /// Funktionsausdrücke direkt hinter `(` oder `!` behält er ganz; ihre
    /// inneren Funktionen prüft er trotzdem nur vor.
    pub immediate: bool,
}

impl PreCheck {
    /// Die Vorgabe des Agents ([`crate::agent::Agent::set_pre_check`]).
    ///
    /// Die Grenze für die Tiefe ist gemessen (Debug-Build, Faden mit 1 MiB
    /// Stapel, Test `pre_checked_bodies_are_read_again_on_a_small_stack`):
    /// Mit 128 Ebenen reicht der Stapel für das Nachlesen in der tiefsten
    /// Verschachtelung noch bei einem [`crate::agent::STACK_BUDGET`] von
    /// 640 KiB, mehr als für das Übersetzen eines Rumpfs, den der Parser
    /// gerade noch annimmt; ohne Grenze nur bis 448 KiB.
    pub const DEFAULT: PreCheck = PreCheck {
        min_nodes: 32,
        max_depth: 128,
        immediate: true,
    };

    /// Jeder Rumpf, der es sein kann, wird nur vorgeprüft; für Tests.
    pub const ALL: PreCheck = PreCheck {
        min_nodes: 0,
        max_depth: MAX_DEPTH,
        immediate: false,
    };
}

/// Liest ein Script wie [`super::parse_script`] und prüft die
/// Funktionsrümpfe nach `pre_check` nur vor.
pub fn parse_script_pre_checked(source: &[u16], pre_check: PreCheck) -> Result<Ast> {
    let mut parser = Parser::new(source, false)?;
    parser.pre_check = Some(pre_check);
    parser.program()
}

/// Liest ein Modul wie [`super::parse_module`] und prüft die
/// Funktionsrümpfe nach `pre_check` nur vor.
pub fn parse_module_pre_checked(source: &[u16], pre_check: PreCheck) -> Result<Ast> {
    let mut parser = Parser::new(source, true)?;
    parser.pre_check = Some(pre_check);
    parser.program()
}

impl Parser<'_> {
    /// Der Rumpf ab `{` an der Lage `start`, ganz gelesen; danach verwirft
    /// er die Knoten, wenn `pre_check` es erlaubt.
    pub(super) fn pre_checked_body(
        &mut self,
        pre_check: PreCheck,
        start: usize,
        simple: bool,
        names: &Names,
        name: Option<(Atom, usize)>,
        arrow: bool,
    ) -> Result<(List, Option<Box<LazyBody>>)> {
        let marks = self.ast.marks();
        let context = self.function.context;
        let depth = self.depth;
        let outer_max = std::mem::replace(&mut self.max_depth, depth);
        let result = self.body_statements(start, simple, names, name);
        let body_max = self.max_depth;
        self.max_depth = outer_max.max(body_max);
        let list = result?;
        let nodes = self.ast.next_node().saturating_sub(marks.nodes());
        if nodes < pre_check.min_nodes || body_max.saturating_sub(depth) > pre_check.max_depth {
            return Ok((list, None));
        }
        let in_with = self.with_depth > 0;
        let captures = self.ast.region_captures(marks.region(), arrow, in_with);
        self.ast.truncate(marks);
        self.forget_nodes_from(marks.nodes());
        let lazy = LazyBody {
            start,
            context,
            parameters: names.clone().into_boxed_slice(),
            name,
            in_with,
            beyond: captures.beyond,
            super_property: captures.own.super_property,
            script_this: captures.beyond_this,
        };
        Ok((List::default(), Some(Box::new(lazy))))
    }

    /// Ob das zuletzt gelesene Token `(` oder `!` ist. Kein anderes Token
    /// endet mit einem dieser Zeichen: Zeichenketten, Templates und
    /// reguläre Ausdrücke enden mit ihren eigenen Zeichen oder Flags.
    pub(super) fn after_wrapping_token(&self) -> bool {
        self.previous_end
            .checked_sub(1)
            .and_then(|index| self.source.get(index))
            .is_some_and(|&unit| unit == u16::from(b'(') || unit == u16::from(b'!'))
    }

    /// Vergisst, was der Parser sich über Knoten ab dem Index `index` gemerkt
    /// hat; sie sind verworfen, und neue bekommen ihre Nummern.
    fn forget_nodes_from(&mut self, index: usize) {
        if self
            .arrow_head
            .is_some_and(|head| head.node.index() >= index)
        {
            self.arrow_head = None;
        }
        if let Some(first) = NodeId::from_index(index) {
            drop(self.spread_with_comma.split_off(&first));
        }
    }
}

impl Ast {
    /// Liest den Rumpf der Funktion `id` nach, wenn der Parser ihn nur
    /// vorgeprüft hat, und hängt seine Knoten an den Baum an. Danach lässt
    /// sich die Funktion übersetzen. Seine inneren Funktionen prüft es nach
    /// denselben Regeln wie beim ersten Lesen nur vor.
    ///
    /// Ein Fehler kann nur entstehen, wenn der Baum nicht mehr zum Quelltext
    /// passt; dann bleibt er, wie er war.
    pub fn expand(&mut self, id: FunctionId) -> std::result::Result<(), SyntaxError> {
        let Some(function) = self.function(id) else {
            return Ok(());
        };
        let Some(lazy) = function.lazy.clone() else {
            return Ok(());
        };
        let arrow = function.kind == FunctionKind::Arrow;
        let simple = function.simple_parameters;
        let end = function.range.end;
        let source: Rc<[u16]> = self.shared_source();
        let Some(text) = source.get(..end) else {
            return Err(SyntaxError {
                kind: ErrorKind::UnexpectedToken,
                position: lazy.start,
            });
        };
        let marks = self.marks();
        let module = self.is_module();
        let pre_check = self.pre_check();
        let ast = std::mem::take(self);
        let mut parser = Parser::resume(ast, text, module, lazy.start, pre_check);
        parser.function = FunctionState::new(lazy.context);
        parser.with_depth = u32::from(lazy.in_with);
        parser.scopes.push(Scope::new(ScopeKind::Function));
        for &(parameter, _) in lazy.parameters.iter() {
            parser.declare_parameter(parameter);
        }
        // Die privaten Namen hat das erste Lesen schon geprüft; was der
        // Rumpf nennt, sammelt sich hier.
        parser.classes.push(PrivateScope::default());
        let names: Names = lazy.parameters.to_vec();
        let result = parser
            .body_statements(lazy.start, simple, &names, lazy.name)
            .and_then(|list| {
                if parser.previous_end == end && !parser.ast.overflowed() {
                    Ok(list)
                } else {
                    Err(SyntaxError {
                        kind: ErrorKind::UnexpectedToken,
                        position: parser.previous_end,
                    })
                }
            });
        *self = parser.ast;
        let list = match result {
            Ok(list) => list,
            Err(error) => {
                self.truncate(marks);
                return Err(error);
            }
        };
        if let Some(function) = self.function_mut(id) {
            function.body = list;
            function.lazy = None;
        }
        if self.captures_mut().is_some() {
            let body = self.region_captures(marks.region(), arrow, lazy.in_with);
            let extended = self
                .captures_mut()
                .is_some_and(|captures| captures.extend(id, marks.region(), body));
            if !extended {
                self.reset_captures();
            }
        }
        Ok(())
    }
}

impl<'a> Parser<'a> {
    /// Ein Parser, der im Baum `ast` weiterliest, ab der Lage `start` in
    /// `source`.
    fn resume(
        ast: Ast,
        source: &'a [u16],
        module: bool,
        start: usize,
        pre_check: Option<PreCheck>,
    ) -> Parser<'a> {
        Parser {
            source,
            lexer: Lexer::starting_at(source, module, start),
            current: None,
            at_start: false,
            previous_end: start,
            ast,
            scratch: Vec::new(),
            module,
            function: FunctionState::default(),
            scopes: Vec::new(),
            classes: Vec::new(),
            depth: 0,
            exported_names: std::collections::BTreeSet::new(),
            exported_bindings: Vec::new(),
            arrow_head: None,
            spread_with_comma: std::collections::BTreeSet::new(),
            pre_check,
            with_depth: 0,
            max_depth: 0,
        }
    }
}

#[cfg(test)]
mod measure;
#[cfg(test)]
mod tests;
