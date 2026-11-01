//! Was innere Funktionen sehen: für jede Funktion und für das Script die
//! Namen, auf die tiefer verschachtelte Funktionen zugreifen, und ob
//! Pfeilfunktionen darin `this` lesen. Bindungen mit diesen Namen legt der
//! Übersetzer in Kontexte statt in Register.
//!
//! Gezählt wird nach Namen, nicht nach Bindungen: Greift eine innere
//! Funktion auf `x` zu, liegen alle Bindungen namens `x` der umgebenden
//! Funktionen in Kontexten, auch solche, die sie gar nicht erreicht. Das
//! sind höchstens mehr als nötig, nie zu wenige. Ebenso zählen Knoten, die
//! der Parser beim Umdeuten einer Cover-Grammatik zurückließ.
//!
//! Zu welcher Funktion ein Bezeichner gehört, folgt aus den Lagen im
//! Quelltext: Die Funktionen liegen ineinander oder nebeneinander, und ein
//! Durchlauf in der Reihenfolge des Quelltexts hält die Kette der
//! Funktionen um die aktuelle Stelle auf einem Stapel. Eine Methode beginnt
//! dabei erst hinter ihrem Namen: Ihr Quelltext enthält ihn, ein
//! berechneter Name läuft aber im umgebenden Code.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use super::{Ast, Atom, FunctionId, FunctionKind, Node, to_usize};

/// Was innere Funktionen sehen, für alle Funktionen eines Baums.
#[derive(Clone, Debug, Default)]
pub(crate) struct Captures {
    script: FunctionCaptures,
    functions: Vec<FunctionCaptures>,
}

/// Was die inneren Funktionen einer Funktion oder des Scripts sehen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FunctionCaptures {
    /// Die Namen, auf die innere Funktionen zugreifen, auch tiefer
    /// verschachtelte.
    pub(crate) names: BTreeSet<Atom>,
    /// Ob Pfeilfunktionen darin, auch ineinander, `this` lesen.
    pub(crate) this: bool,
}

impl Captures {
    /// Für den Code des Scripts oder Moduls außerhalb aller Funktionen.
    pub(crate) fn script(&self) -> &FunctionCaptures {
        &self.script
    }

    pub(crate) fn function(&self, id: FunctionId) -> Option<&FunctionCaptures> {
        self.functions.get(id.index())
    }
}

/// Ein Knoten, der etwas aus der Umgebung liest.
enum Use {
    Name(Atom),
    This,
}

/// Eine Funktion auf dem Stapel um die aktuelle Stelle.
struct Open {
    end: usize,
    index: usize,
    arrow: bool,
}

impl Ast {
    pub(super) fn compute_captures(&self) -> Captures {
        let mut captures = Captures {
            script: FunctionCaptures::default(),
            functions: vec![FunctionCaptures::default(); self.functions.len()],
        };
        let mut uses: Vec<(usize, Use)> = Vec::new();
        for (node, &(start, _)) in self.nodes.iter().zip(&self.ranges) {
            match *node {
                Node::Identifier(atom) => uses.push((to_usize(start), Use::Name(atom))),
                Node::This => uses.push((to_usize(start), Use::This)),
                _ => {}
            }
        }
        uses.sort_by_key(|&(position, _)| position);
        let mut starts: Vec<usize> = self.functions.iter().map(|f| f.range.start).collect();
        for node in &self.nodes {
            let (key, function) = match *node {
                Node::Property { key, value, .. } => match self.nodes.get(value.index()) {
                    Some(&Node::Function(function)) => (key, function),
                    _ => continue,
                },
                Node::ClassMethod { key, function, .. } => (key, function),
                _ => continue,
            };
            if let Some(start) = starts.get_mut(function.index())
                && let Some(&(_, end)) = self.ranges.get(key.index())
            {
                *start = (*start).max(to_usize(end));
            }
        }
        // Äußere Funktionen vor den inneren, die an derselben Stelle beginnen.
        let mut order: Vec<(usize, Reverse<usize>, usize)> = self
            .functions
            .iter()
            .zip(&starts)
            .enumerate()
            .map(|(index, (function, &start))| (start, Reverse(function.range.end), index))
            .collect();
        order.sort_unstable();
        let mut next = order.into_iter().peekable();
        let mut stack: Vec<Open> = Vec::new();
        for (position, found) in uses {
            while let Some(&(start, Reverse(end), index)) = next.peek()
                && start <= position
            {
                next.next();
                while stack.last().is_some_and(|open| open.end <= start) {
                    stack.pop();
                }
                let arrow = self
                    .functions
                    .get(index)
                    .is_some_and(|function| function.kind == FunctionKind::Arrow);
                stack.push(Open { end, index, arrow });
            }
            while stack.last().is_some_and(|open| open.end <= position) {
                stack.pop();
            }
            match found {
                Use::Name(atom) => captures.add_name(&stack, atom),
                Use::This => captures.add_this(&stack),
            }
        }
        captures
    }
}

impl Captures {
    /// Ein Bezeichner in der innersten Funktion auf `stack`: Alle Funktionen
    /// um sie herum und das Script sehen den Namen in einer inneren. Steht er
    /// schon bei einer, steht er auch bei allen weiter außen.
    fn add_name(&mut self, stack: &[Open], atom: Atom) {
        let Some((_, outer)) = stack.split_last() else {
            return;
        };
        for open in outer.iter().rev() {
            if let Some(captures) = self.functions.get_mut(open.index)
                && !captures.names.insert(atom)
            {
                return;
            }
        }
        self.script.names.insert(atom);
    }

    /// `this` in der innersten Funktion auf `stack`: Ist sie eine
    /// Pfeilfunktion, gehört es der nächsten Funktion außen, die keine ist,
    /// oder dem Script.
    fn add_this(&mut self, stack: &[Open]) {
        if !stack.last().is_some_and(|open| open.arrow) {
            return;
        }
        let owner = stack
            .iter()
            .rev()
            .find(|open| !open.arrow)
            .and_then(|open| self.functions.get_mut(open.index));
        match owner {
            Some(captures) => captures.this = true,
            None => self.script.this = true,
        }
    }
}
