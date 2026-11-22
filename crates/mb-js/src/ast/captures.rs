//! Was innere Funktionen sehen: für jede Funktion und für das Script die
//! Namen, auf die tiefer verschachtelte Funktionen zugreifen, und ob
//! Pfeilfunktionen darin `this`, `new.target` oder `super` lesen. Bindungen
//! mit diesen Namen legt der Übersetzer in Kontexte statt in Register.
//! Außerdem steht für jede Funktion, ob ihr Code oder der ihrer
//! Pfeilfunktionen den Namen `arguments`, `new.target` oder `super` enthält.
//!
//! Ein direktes `eval` (ein Aufruf des Bezeichners `eval`, auch in
//! Klammern, nicht optional) kann jeden Namen nennen: Die Funktionen um es
//! herum und das Script legen dann alle ihre Bindungen in Kontexte. Sein
//! Code liest `this`, `new.target`, `super` und `arguments` wie eine
//! Pfeilfunktion an seiner Stelle. Die Funktion, deren eigener Code es
//! enthält, merkt es sich für ihren var-Bereich (siehe
//! [`FunctionCaptures::eval`]).
//!
//! Im Rumpf einer `with`-Anweisung kann jeder Name eine Eigenschaft des
//! Objekts sein; nur wenn es sie nicht hat, gilt die Bindung dahinter, und
//! die liest der Code zur Laufzeit aus ihrem Kontext. Ein Name im Rumpf,
//! auch in `var`, zählt darum auch für die Funktion, deren eigener Code ihn
//! enthält, und für das Script.
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
//! berechneter Name läuft aber im umgebenden Code. Der Initialisierer eines
//! Feldes beginnt mit `=` vor seinem Ausdruck; so liegt eine Funktion, die
//! sein Wert ist, in ihm und nicht um ihn herum.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use super::{Ast, Atom, FunctionId, FunctionKind, Node, NodeId, to_usize};

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
    /// Ob der Code der Funktion oder ihrer Pfeilfunktionen, auch
    /// ineinander, den Bezeichner `arguments` enthält. Beim Script bleibt es
    /// falsch.
    pub(crate) arguments: bool,
    /// Ob der Code der Funktion oder ihrer Pfeilfunktionen, auch ineinander,
    /// `new.target` enthält. Beim Script bleibt es falsch.
    pub(crate) new_target: bool,
    /// Ob Pfeilfunktionen darin, auch ineinander, `new.target` lesen.
    pub(crate) arrow_new_target: bool,
    /// Ob der Code der Funktion oder ihrer Pfeilfunktionen, auch
    /// ineinander, `super.x` oder `super[x]` enthält; dann braucht das
    /// Funktionsobjekt [[HomeObject]].
    pub(crate) super_property: bool,
    /// Ob Pfeilfunktionen darin, auch ineinander, `super` nennen; dann
    /// lesen sie das Funktionsobjekt aus einem Platz im Kontext.
    pub(crate) arrow_super: bool,
    /// Ob der Code der Funktion oder des Scripts selbst, ohne innere
    /// Funktionen, ein direktes `eval` enthält. Außerhalb des strikten Modus
    /// kann es Bindungen mit `var` in ihrem var-Bereich anlegen.
    pub(crate) eval: bool,
    /// Ob der Code oder der einer inneren Funktion ein direktes `eval`
    /// enthält: Dann sieht eine innere Funktion jede Bindung.
    pub(crate) all: bool,
}

impl FunctionCaptures {
    /// Ob eine innere Funktion oder ein `eval` die Bindungen namens `atom`
    /// sehen kann; dann liegen sie in Kontexten.
    pub(crate) fn captured(&self, atom: Atom) -> bool {
        self.all || self.names.contains(&atom)
    }
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
    NewTarget,
    /// `super.x` oder `super[x]`.
    SuperProperty,
    /// `super(...)`.
    SuperCall,
    /// Ein direktes `eval`.
    Eval,
    /// Ein Name, den eine Deklaration bindet; zählt nur im Rumpf einer
    /// `with`-Anweisung.
    Binding(Atom),
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
        // Die Rümpfe der `with`-Anweisungen, nach Anfang geordnet.
        let mut withs: Vec<(usize, Reverse<usize>)> = self
            .nodes
            .iter()
            .filter_map(|node| match *node {
                Node::With { body, .. } => self.ranges.get(body.index()),
                _ => None,
            })
            .map(|&(start, end)| (to_usize(start), Reverse(to_usize(end))))
            .collect();
        withs.sort_unstable();
        for (node, &(start, _)) in self.nodes.iter().zip(&self.ranges) {
            match *node {
                Node::Identifier(atom) => uses.push((to_usize(start), Use::Name(atom))),
                Node::BindingIdentifier(atom) if !withs.is_empty() => {
                    uses.push((to_usize(start), Use::Binding(atom)));
                }
                Node::This => uses.push((to_usize(start), Use::This)),
                Node::NewTarget => uses.push((to_usize(start), Use::NewTarget)),
                Node::SuperMember(_) | Node::SuperComputedMember(_) => {
                    uses.push((to_usize(start), Use::SuperProperty));
                }
                Node::SuperCall(_) => uses.push((to_usize(start), Use::SuperCall)),
                Node::Call {
                    callee,
                    optional: false,
                    ..
                } if self.is_eval_reference(callee) => uses.push((to_usize(start), Use::Eval)),
                _ => {}
            }
        }
        uses.sort_by_key(|&(position, _)| position);
        let arguments = self.find_atom(&"arguments".encode_utf16().collect::<Vec<u16>>());
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
        let mut next_with = withs.into_iter().peekable();
        // Die Enden der Rümpfe um die aktuelle Stelle.
        let mut open_withs: Vec<usize> = Vec::new();
        for (position, found) in uses {
            while let Some(&(start, Reverse(end))) = next_with.peek()
                && start <= position
            {
                next_with.next();
                while open_withs.last().is_some_and(|&open| open <= start) {
                    open_withs.pop();
                }
                open_withs.push(end);
            }
            while open_withs.last().is_some_and(|&open| open <= position) {
                open_withs.pop();
            }
            let in_with = !open_withs.is_empty();
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
                Use::Name(atom) => {
                    if Some(atom) == arguments {
                        captures.add_arguments(&stack);
                    }
                    if in_with {
                        captures.add_with_name(&stack, atom);
                    } else {
                        captures.add_name(&stack, atom);
                    }
                }
                Use::Binding(atom) => {
                    if in_with {
                        captures.add_with_name(&stack, atom);
                    }
                }
                Use::This => captures.add_this(&stack),
                Use::NewTarget => captures.add_new_target(&stack),
                Use::SuperProperty => captures.add_super(&stack, false),
                Use::SuperCall => captures.add_super(&stack, true),
                Use::Eval => captures.add_eval(&stack),
            }
        }
        captures
    }

    /// Ob `callee` der Bezeichner `eval` ist, auch in Klammern: Dann ist ein
    /// Aufruf ohne `?.` ein direktes `eval`, wenn der Wert %eval% ist
    /// (Abschnitt 13.3.6.1, Schritt 6).
    pub(crate) fn is_eval_reference(&self, callee: NodeId) -> bool {
        let mut node = callee;
        // Klammern begrenzt der Parser mit seiner Tiefe.
        loop {
            match self.nodes.get(node.index()) {
                Some(&Node::Paren(inner)) => node = inner,
                Some(&Node::Identifier(atom)) => {
                    return self.atom(atom).iter().copied().eq("eval".encode_utf16());
                }
                _ => return false,
            }
        }
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

    /// Ein Name im Rumpf einer `with`-Anweisung in der innersten Funktion
    /// auf `stack`: Sie selbst, alle Funktionen um sie herum und das Script
    /// legen Bindungen mit dem Namen in Kontexte.
    fn add_with_name(&mut self, stack: &[Open], atom: Atom) {
        for open in stack {
            if let Some(captures) = self.functions.get_mut(open.index) {
                captures.names.insert(atom);
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

    /// Der Bezeichner `arguments` in der innersten Funktion auf `stack`: Er
    /// gehört ihr oder, wenn sie eine Pfeilfunktion ist, der nächsten
    /// Funktion außen, die keine ist.
    fn add_arguments(&mut self, stack: &[Open]) {
        if let Some(captures) = stack
            .iter()
            .rev()
            .find(|open| !open.arrow)
            .and_then(|open| self.functions.get_mut(open.index))
        {
            captures.arguments = true;
        }
    }

    /// `new.target` in der innersten Funktion auf `stack`: Es gehört ihr
    /// oder, wenn sie eine Pfeilfunktion ist, der nächsten Funktion außen,
    /// die keine ist; dann liest die Pfeilfunktion es aus deren Kontext.
    /// Außerhalb aller Funktionen lässt der Parser es nicht zu.
    fn add_new_target(&mut self, stack: &[Open]) {
        let arrow = stack.last().is_some_and(|open| open.arrow);
        if let Some(captures) = stack
            .iter()
            .rev()
            .find(|open| !open.arrow)
            .and_then(|open| self.functions.get_mut(open.index))
        {
            captures.new_target = true;
            captures.arrow_new_target |= arrow;
        }
    }

    /// Ein direktes `eval` in der innersten Funktion auf `stack`: Es kann
    /// jeden Namen der Funktionen um sie herum und des Scripts nennen und
    /// liest `this`, `new.target`, `super` und `arguments` wie eine
    /// Pfeilfunktion an seiner Stelle aus dem Kontext der nächsten Funktion
    /// außen, die keine Pfeilfunktion ist, oder des Scripts.
    fn add_eval(&mut self, stack: &[Open]) {
        match stack.last() {
            Some(open) => {
                if let Some(captures) = self.functions.get_mut(open.index) {
                    captures.eval = true;
                }
            }
            None => self.script.eval = true,
        }
        for open in stack {
            if let Some(captures) = self.functions.get_mut(open.index) {
                captures.all = true;
            }
        }
        self.script.all = true;
        let owner = stack
            .iter()
            .rev()
            .find(|open| !open.arrow)
            .and_then(|open| self.functions.get_mut(open.index));
        match owner {
            Some(captures) => {
                captures.this = true;
                captures.arguments = true;
                captures.new_target = true;
                captures.arrow_new_target = true;
                captures.super_property = true;
                captures.arrow_super = true;
            }
            None => self.script.this = true,
        }
    }

    /// `super` in der innersten Funktion auf `stack`, mit `call` für
    /// `super(...)`: Es gehört ihr oder, wenn sie eine Pfeilfunktion ist, der
    /// nächsten Funktion außen, die keine ist. Eine Referenz braucht deren
    /// [[HomeObject]], ein Aufruf ihr new.target. Eine Pfeilfunktion liest
    /// dazu das Funktionsobjekt und `this` aus deren Kontext, ein Aufruf
    /// auch new.target, und bindet dort `this`. Außerhalb aller Funktionen
    /// lässt der Parser es nicht zu.
    fn add_super(&mut self, stack: &[Open], call: bool) {
        let arrow = stack.last().is_some_and(|open| open.arrow);
        let Some(captures) = stack
            .iter()
            .rev()
            .find(|open| !open.arrow)
            .and_then(|open| self.functions.get_mut(open.index))
        else {
            return;
        };
        if call {
            captures.new_target = true;
        } else {
            captures.super_property = true;
        }
        if arrow {
            captures.this = true;
            captures.arrow_super = true;
            captures.arrow_new_target |= call;
        }
    }
}
