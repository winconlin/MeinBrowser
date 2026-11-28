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
//!
//! Von einem Rumpf, den der Parser nur vorgeprüft hat ([`LazyBody`]), sind
//! keine Knoten da. Bevor er sie verwirft, läuft dieselbe Analyse über den
//! Rumpf allein, mit der Funktion unten auf dem Stapel und darunter einer
//! gedachten Funktion, die keine Pfeilfunktion ist. Was bei ihr ankommt,
//! ist, was der Rumpf dem Code um die Funktion zuträgt; das steht in
//! [`LazyBody::beyond`] und zählt in der Analyse des ganzen Baums an der
//! Stelle von `{`. Liest [`Ast::expand`] den Rumpf nach, läuft die Analyse
//! über die neuen Knoten noch einmal und ergänzt die Einträge der Funktion
//! und ihrer inneren Funktionen. Weil jede Wirkung eines Knotens nur von
//! den Funktionen zwischen ihm und dem Eintrag abhängt und Einträge nur
//! wachsen, ergibt das dieselben Einträge wie ein Durchlauf über den ganz
//! gelesenen Baum.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use super::{Ast, Atom, FunctionId, FunctionKind, LazyBody, Node, NodeId, to_usize};

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
    /// Ob der Code der Funktion oder ihrer Pfeilfunktionen, auch ineinander,
    /// den Bezeichner `arguments` enthält. Beim Script bleibt es falsch.
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

    /// Nimmt alles aus `other` dazu.
    fn merge(&mut self, other: FunctionCaptures) {
        self.names.extend(other.names);
        self.this |= other.this;
        self.arguments |= other.arguments;
        self.new_target |= other.new_target;
        self.arrow_new_target |= other.arrow_new_target;
        self.super_property |= other.super_property;
        self.arrow_super |= other.arrow_super;
        self.eval |= other.eval;
        self.all |= other.all;
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

    /// Ergänzt die Einträge um einen nachgelesenen Rumpf der Funktion
    /// `function`, dessen Knoten und Funktionen ab `region` im Baum stehen.
    /// Gibt `false` zurück, wenn die Einträge nicht zum Baum vor dem
    /// Nachlesen passen; dann muss die Analyse neu laufen.
    pub(crate) fn extend(
        &mut self,
        function: FunctionId,
        region: Region,
        body: RegionCaptures,
    ) -> bool {
        if self.functions.len() != region.functions {
            return false;
        }
        let Some(entry) = self.functions.get_mut(function.index()) else {
            return false;
        };
        entry.merge(body.own);
        self.functions.extend(body.functions);
        true
    }
}

/// Wo eine Analyse beginnt: bei diesen Indizes der Knoten und Funktionen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub(crate) nodes: usize,
    pub(crate) functions: usize,
}

/// Die Analyse für einen Rumpf allein ([`Ast::region_captures`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct RegionCaptures {
    /// Für die Funktionen im Rumpf, in der Reihenfolge des Baums.
    pub(crate) functions: Vec<FunctionCaptures>,
    /// Was der Rumpf dem Eintrag seiner Funktion zuträgt.
    pub(crate) own: FunctionCaptures,
    /// Was er den Funktionen außen zuträgt ([`LazyBody::beyond`]).
    pub(crate) beyond: FunctionCaptures,
    /// Ob `this` oder ein direktes `eval` im Rumpf `this` außen liest
    /// ([`LazyBody::script_this`]).
    pub(crate) beyond_this: bool,
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
    /// Der vorgeprüfte Rumpf der Funktion mit diesem Index, an der Stelle
    /// von `{`.
    Lazy(usize),
}

/// Eine Funktion auf dem Stapel um die aktuelle Stelle.
#[derive(Clone, Copy)]
struct Open {
    end: usize,
    index: usize,
    arrow: bool,
}

/// Die Einträge einer Analyse: für die Funktionen ab `offset`, in ihrer
/// Reihenfolge, und für das Script.
struct Sink {
    offset: usize,
    functions: Vec<FunctionCaptures>,
    script: FunctionCaptures,
    /// Die gedachte Funktion unter einem Rumpf allein.
    beyond: Option<usize>,
    /// Ob `this` oder `eval` bei ihr ankam, nicht nur `super`.
    beyond_this: bool,
}

impl Sink {
    fn get_mut(&mut self, index: usize) -> Option<&mut FunctionCaptures> {
        self.functions.get_mut(index.checked_sub(self.offset)?)
    }
}

impl Ast {
    pub(super) fn compute_captures(&self) -> Captures {
        let mut sink = Sink {
            offset: 0,
            functions: vec![FunctionCaptures::default(); self.functions.len()],
            script: FunctionCaptures::default(),
            beyond: None,
            beyond_this: false,
        };
        let region = Region {
            nodes: 0,
            functions: 0,
        };
        self.walk(region, &[], false, &mut sink);
        Captures {
            script: sink.script,
            functions: sink.functions,
        }
    }

    /// Die Analyse für den Rumpf einer Funktion allein, dessen Knoten und
    /// innere Funktionen ab `region` im Baum stehen; `arrow` sagt, ob die
    /// Funktion eine Pfeilfunktion ist, `in_with`, ob sie im Rumpf einer
    /// `with`-Anweisung steht.
    pub(crate) fn region_captures(
        &self,
        region: Region,
        arrow: bool,
        in_with: bool,
    ) -> RegionCaptures {
        let count = self.functions.len().saturating_sub(region.functions);
        let own = region.functions.saturating_add(count);
        let beyond = own.saturating_add(1);
        let mut sink = Sink {
            offset: region.functions,
            functions: vec![FunctionCaptures::default(); count.saturating_add(2)],
            script: FunctionCaptures::default(),
            beyond: Some(beyond),
            beyond_this: false,
        };
        let base = [
            Open {
                end: usize::MAX,
                index: beyond,
                arrow: false,
            },
            Open {
                end: usize::MAX,
                index: own,
                arrow,
            },
        ];
        self.walk(region, &base, in_with, &mut sink);
        let beyond = sink.functions.pop().unwrap_or_default();
        let own = sink.functions.pop().unwrap_or_default();
        RegionCaptures {
            functions: sink.functions,
            own,
            beyond,
            beyond_this: sink.beyond_this,
        }
    }

    /// Der Durchlauf über die Knoten und Funktionen ab `region`, mit `base`
    /// unten auf dem Stapel; mit `in_with` steht alles im Rumpf einer
    /// `with`-Anweisung.
    fn walk(&self, region: Region, base: &[Open], in_with: bool, sink: &mut Sink) {
        let nodes = self.nodes.get(region.nodes..).unwrap_or_default();
        let ranges = self.ranges.get(region.nodes..).unwrap_or_default();
        let functions = self.functions.get(region.functions..).unwrap_or_default();
        let mut uses: Vec<(usize, Use)> = Vec::new();
        // Die Rümpfe der `with`-Anweisungen, nach Anfang geordnet.
        let mut withs: Vec<(usize, Reverse<usize>)> = nodes
            .iter()
            .filter_map(|node| match *node {
                Node::With { body, .. } => self.ranges.get(body.index()),
                _ => None,
            })
            .map(|&(start, end)| (to_usize(start), Reverse(to_usize(end))))
            .collect();
        withs.sort_unstable();
        let bindings = in_with || !withs.is_empty();
        for (node, &(start, _)) in nodes.iter().zip(ranges) {
            match *node {
                Node::Identifier(atom) => uses.push((to_usize(start), Use::Name(atom))),
                Node::BindingIdentifier(atom) if bindings => {
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
        for (offset, function) in functions.iter().enumerate() {
            if let Some(lazy) = &function.lazy {
                let index = region.functions.saturating_add(offset);
                uses.push((lazy.start, Use::Lazy(index)));
            }
        }
        uses.sort_by_key(|&(position, _)| position);
        let arguments = self.find_atom(&"arguments".encode_utf16().collect::<Vec<u16>>());
        let mut starts: Vec<usize> = functions.iter().map(|f| f.range.start).collect();
        for node in nodes {
            let (key, function) = match *node {
                Node::Property { key, value, .. } => match self.nodes.get(value.index()) {
                    Some(&Node::Function(function)) => (key, function),
                    _ => continue,
                },
                Node::ClassMethod { key, function, .. } => (key, function),
                _ => continue,
            };
            if let Some(offset) = function.index().checked_sub(region.functions)
                && let Some(start) = starts.get_mut(offset)
                && let Some(&(_, end)) = self.ranges.get(key.index())
            {
                *start = (*start).max(to_usize(end));
            }
        }
        // Äußere Funktionen vor den inneren, die an derselben Stelle beginnen.
        let mut order: Vec<(usize, Reverse<usize>, usize)> = functions
            .iter()
            .zip(&starts)
            .enumerate()
            .map(|(offset, (function, &start))| {
                (
                    start,
                    Reverse(function.range.end),
                    region.functions.saturating_add(offset),
                )
            })
            .collect();
        order.sort_unstable();
        let mut next = order.into_iter().peekable();
        let mut stack: Vec<Open> = base.to_vec();
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
            let here_in_with = in_with || !open_withs.is_empty();
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
                        sink.add_arguments(&stack);
                    }
                    if here_in_with {
                        sink.add_with_name(&stack, atom);
                    } else {
                        sink.add_name(&stack, atom);
                    }
                }
                Use::Binding(atom) => {
                    if here_in_with {
                        sink.add_with_name(&stack, atom);
                    }
                }
                Use::This => sink.add_this(&stack),
                Use::NewTarget => sink.add_new_target(&stack),
                Use::SuperProperty => sink.add_super(&stack, false),
                Use::SuperCall => sink.add_super(&stack, true),
                Use::Eval => sink.add_eval(&stack),
                Use::Lazy(index) => {
                    let Some(lazy) = self
                        .functions
                        .get(index)
                        .and_then(|function| function.lazy.as_deref())
                    else {
                        continue;
                    };
                    // Die Funktion ist hier die innerste; was der Rumpf
                    // außen bewirkt, gilt für die Funktionen unter ihr.
                    let below = stack
                        .iter()
                        .rposition(|open| open.index == index)
                        .map_or(&stack[..], |at| stack.get(..at).unwrap_or_default());
                    sink.add_lazy(below, index, lazy);
                }
            }
        }
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

impl Sink {
    /// Ein Bezeichner in der innersten Funktion auf `stack`: Alle Funktionen
    /// um sie herum und das Script sehen den Namen in einer inneren.
    fn add_name(&mut self, stack: &[Open], atom: Atom) {
        let Some((_, outer)) = stack.split_last() else {
            return;
        };
        self.add_name_to(outer, atom);
    }

    /// `atom` für die Funktionen `functions` und das Script. Steht er schon
    /// bei einer, steht er auch bei allen weiter außen.
    fn add_name_to(&mut self, functions: &[Open], atom: Atom) {
        for open in functions.iter().rev() {
            if let Some(captures) = self.get_mut(open.index)
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
            if let Some(captures) = self.get_mut(open.index) {
                captures.names.insert(atom);
            }
        }
        self.script.names.insert(atom);
    }

    /// Die nächste Funktion von innen auf `stack`, die keine Pfeilfunktion
    /// ist.
    fn owner(stack: &[Open]) -> Option<usize> {
        stack
            .iter()
            .rev()
            .find(|open| !open.arrow)
            .map(|open| open.index)
    }

    /// `this` in der innersten Funktion auf `stack`: Ist sie eine
    /// Pfeilfunktion, gehört es der nächsten Funktion außen, die keine ist,
    /// oder dem Script.
    fn add_this(&mut self, stack: &[Open]) {
        if !stack.last().is_some_and(|open| open.arrow) {
            return;
        }
        let owner = Self::owner(stack);
        self.beyond_this |= owner.is_some() && owner == self.beyond;
        match owner.and_then(|index| self.get_mut(index)) {
            Some(captures) => captures.this = true,
            None => self.script.this = true,
        }
    }

    /// Der Bezeichner `arguments` in der innersten Funktion auf `stack`: Er
    /// gehört ihr oder, wenn sie eine Pfeilfunktion ist, der nächsten
    /// Funktion außen, die keine ist.
    fn add_arguments(&mut self, stack: &[Open]) {
        if let Some(captures) = Self::owner(stack).and_then(|index| self.get_mut(index)) {
            captures.arguments = true;
        }
    }

    /// `new.target` in der innersten Funktion auf `stack`: Es gehört ihr
    /// oder, wenn sie eine Pfeilfunktion ist, der nächsten Funktion außen,
    /// die keine ist; dann liest die Pfeilfunktion es aus deren Kontext.
    /// Außerhalb aller Funktionen lässt der Parser es nicht zu.
    fn add_new_target(&mut self, stack: &[Open]) {
        let arrow = stack.last().is_some_and(|open| open.arrow);
        if let Some(captures) = Self::owner(stack).and_then(|index| self.get_mut(index)) {
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
                if let Some(captures) = self.get_mut(open.index) {
                    captures.eval = true;
                }
            }
            None => self.script.eval = true,
        }
        for open in stack {
            if let Some(captures) = self.get_mut(open.index) {
                captures.all = true;
            }
        }
        self.script.all = true;
        let owner = Self::owner(stack);
        self.beyond_this |= owner.is_some() && owner == self.beyond;
        match owner.and_then(|index| self.get_mut(index)) {
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
        let Some(captures) = Self::owner(stack).and_then(|index| self.get_mut(index)) else {
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

    /// Der vorgeprüfte Rumpf `lazy` der Funktion `index`, die über den
    /// Funktionen `below` liegt: Er wirkt auf sie und das Script, wie seine
    /// Knoten es täten.
    fn add_lazy(&mut self, below: &[Open], index: usize, lazy: &LazyBody) {
        if let Some(own) = self.get_mut(index) {
            own.super_property |= lazy.super_property;
        }
        let beyond = &lazy.beyond;
        for &atom in &beyond.names {
            self.add_name_to(below, atom);
        }
        if beyond.all {
            for open in below {
                if let Some(captures) = self.get_mut(open.index) {
                    captures.all = true;
                }
            }
            self.script.all = true;
        }
        let owner = Self::owner(below);
        if owner.is_some() && owner == self.beyond {
            self.beyond_this |= lazy.script_this;
        }
        match owner.and_then(|index| self.get_mut(index)) {
            Some(captures) => {
                captures.this |= beyond.this;
                captures.arguments |= beyond.arguments;
                captures.new_target |= beyond.new_target;
                captures.arrow_new_target |= beyond.arrow_new_target;
                captures.super_property |= beyond.super_property;
                captures.arrow_super |= beyond.arrow_super;
            }
            None => self.script.this |= lazy.script_this,
        }
    }
}
