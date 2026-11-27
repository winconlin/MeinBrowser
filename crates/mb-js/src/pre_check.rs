//! Prüft, dass vorgeprüfte Funktionsrümpfe ([`crate::parser::PreCheck`])
//! nach dem Nachlesen dasselbe ergeben wie ganz gelesener Quelltext: dieselben
//! Fehler, dieselben Atome, dieselben Funktionen mit derselben Analyse für
//! den Übersetzer und derselbe Bytecode für das Script oder Modul und jede
//! Funktion darin. Für Tests und das Fuzz-Ziel `js-pre-check`.
//!
//! Nur die Reihenfolge der Knoten, Funktionen und Klassen im Baum darf sich
//! unterscheiden. Funktionen und Klassen werden darum über ihre Lage im
//! Quelltext zugeordnet, und im Bytecode des vorgeprüften Baums werden ihre
//! Nummern vor dem Vergleich durch die des ganz gelesenen ersetzt.
//!
//! Verglichen wird auf zwei Wegen. Der erste geht wie die Laufzeit vor: Er
//! übersetzt das Script oder Modul, solange die Rümpfe noch vorgeprüft
//! sind, und liest jede Funktion erst nach, bevor er sie übersetzt. Die
//! Analyse entsteht dann vor dem Nachlesen und wird von ihm ergänzt, und
//! der Übersetzer einer Funktion sieht ihre inneren Funktionen noch
//! vorgeprüft. Der zweite liest zuerst alle Rümpfe nach; die Analyse
//! entsteht dann über den ganzen Baum.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::ast::{Ast, Atom, ClassId, FunctionId};
use crate::bytecode::{Code, Constant, FunctionTemplate};
use crate::compiler::{CompileError, ModuleInit, compile_function, compile_module, compile_script};
use crate::parser::{
    PreCheck, SyntaxError, parse_module, parse_module_pre_checked, parse_script,
    parse_script_pre_checked,
};

/// Was beim Vergleich nicht passte. Lagen sind die Anfänge der Funktionen
/// im Quelltext.
#[derive(Clone, Debug, PartialEq)]
pub enum Difference {
    /// Nur einer der beiden Wege fand einen Fehler, oder sie fanden
    /// verschiedene.
    Errors {
        full: Option<SyntaxError>,
        pre_checked: Option<SyntaxError>,
    },
    /// Das Nachlesen schlug fehl.
    Expand(SyntaxError),
    /// Die Atome der Bäume unterscheiden sich.
    Atoms,
    /// Die Funktionen oder Klassen lassen sich nicht über ihre Lagen
    /// zuordnen.
    Functions,
    /// Die Angaben einer Funktion unterscheiden sich.
    Function(usize),
    /// Die Analyse für den Übersetzer unterscheidet sich, beim Script an
    /// der Lage 0.
    Captures(usize),
    /// Der Bytecode des Scripts oder Moduls unterscheidet sich.
    Program,
    /// Der Bytecode einer Funktion unterscheidet sich.
    Code(usize),
}

/// Liest `source` als Script oder, mit `module`, als Modul einmal ganz und
/// einmal mit `pre_check`, liest die vorgeprüften Rümpfe nach und
/// vergleicht.
pub fn compare(source: &[u16], module: bool, pre_check: PreCheck) -> Result<(), Difference> {
    let (full, pre_checked) = if module {
        (
            parse_module(source),
            parse_module_pre_checked(source, pre_check),
        )
    } else {
        (
            parse_script(source),
            parse_script_pre_checked(source, pre_check),
        )
    };
    let (full, pre_checked) = match (full, pre_checked) {
        (Ok(full), Ok(pre_checked)) => (full, pre_checked),
        (Err(full), Err(pre_checked)) if full == pre_checked => return Ok(()),
        (full, pre_checked) => {
            return Err(Difference::Errors {
                full: full.err(),
                pre_checked: pre_checked.err(),
            });
        }
    };
    let mut on_demand = pre_checked.clone();
    compare_code(&full, &mut on_demand)?;
    on_demand.expand_all().map_err(Difference::Expand)?;
    compare_trees(&full, &on_demand)?;
    let mut expanded = pre_checked;
    expanded.expand_all().map_err(Difference::Expand)?;
    compare_trees(&full, &expanded)?;
    compare_code(&full, &mut expanded)
}

/// Die Zuordnung der Funktionen und Klassen von `tree` zu denen von `full`
/// über ihre Lagen.
struct Mapping {
    full_functions: BTreeMap<(usize, usize), FunctionId>,
    full_classes: BTreeMap<(usize, usize), ClassId>,
    /// Nummer in `tree` auf Nummer in `full`.
    functions: BTreeMap<u32, FunctionId>,
    classes: BTreeMap<u32, ClassId>,
    /// Die schon vergebenen Nummern in `full`.
    taken_functions: BTreeSet<u32>,
    taken_classes: BTreeSet<u32>,
}

fn compare_trees(full: &Ast, tree: &Ast) -> Result<(), Difference> {
    if full.atom_count() != tree.atom_count() {
        return Err(Difference::Atoms);
    }
    for raw in 1..=full.atom_count() {
        let atom = u32::try_from(raw)
            .ok()
            .and_then(Atom::from_raw)
            .ok_or(Difference::Atoms)?;
        if full.atom(atom) != tree.atom(atom) {
            return Err(Difference::Atoms);
        }
    }
    if full.captures().script() != tree.captures().script() {
        return Err(Difference::Captures(0));
    }
    let mut mapping = Mapping::new(full)?;
    mapping.update(tree)?;
    if !mapping.complete() {
        return Err(Difference::Functions);
    }
    for (&raw, &id) in &mapping.functions {
        let other = FunctionId::from_raw(raw);
        let (Some(a), Some(b)) = (full.function(id), tree.function(other)) else {
            return Err(Difference::Functions);
        };
        let same = a.kind == b.kind
            && a.generator == b.generator
            && a.is_async == b.is_async
            && a.strict == b.strict
            && a.simple_parameters == b.simple_parameters
            && a.name == b.name
            && a.expression_body == b.expression_body
            && a.range == b.range
            && a.parameters.len() == b.parameters.len()
            && a.body.len() == b.body.len()
            && a.lazy.is_none()
            && b.lazy.is_none();
        if !same {
            return Err(Difference::Function(a.range.start));
        }
        if full.captures().function(id) != tree.captures().function(other) {
            return Err(Difference::Captures(a.range.start));
        }
    }
    Ok(())
}

/// Übersetzt das Script oder Modul und alle Funktionen, die es erreicht,
/// in beiden Bäumen und vergleicht den Bytecode. Eine vorgeprüfte Funktion
/// in `tree` liest es erst nach, bevor es sie übersetzt.
fn compare_code(full: &Ast, tree: &mut Ast) -> Result<(), Difference> {
    let mut mapping = Mapping::new(full)?;
    mapping.update(tree)?;
    let mut open: Vec<(FunctionTemplate, FunctionTemplate)> = Vec::new();
    if full.is_module() {
        let (a, b) = match (compile_module(full), compile_module(tree)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(a), Err(b)) if a == b => return Ok(()),
            _ => return Err(Difference::Program),
        };
        if a.slots != b.slots
            || a.bindings.len() != b.bindings.len()
            || !same_code(&a.code, &b.code, tree, &mapping, &mut open)
        {
            return Err(Difference::Program);
        }
        for (a, b) in a.bindings.iter().zip(&b.bindings) {
            let same = a.name == b.name
                && a.slot == b.slot
                && match (&a.init, &b.init) {
                    (ModuleInit::Function(a), ModuleInit::Function(b)) => {
                        let mapped = mapped_template(b, tree, &mapping);
                        open.push((a.clone(), b.clone()));
                        mapped.as_ref() == Some(a)
                    }
                    (a, b) => a == b,
                };
            if !same {
                return Err(Difference::Program);
            }
        }
    } else {
        let (a, b) = match (compile_script(full), compile_script(tree)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(a), Err(b)) if a == b => return Ok(()),
            _ => return Err(Difference::Program),
        };
        let same = a.var_names == b.var_names
            && a.lexical_names == b.lexical_names
            && a.hoisted_functions == b.hoisted_functions
            && a.functions.len() == b.functions.len()
            && same_code(&a.code, &b.code, tree, &mapping, &mut open);
        if !same {
            return Err(Difference::Program);
        }
        for ((name, a), (other, b)) in a.functions.iter().zip(&b.functions) {
            if name != other || mapping.functions.get(&b.to_raw()) != Some(a) {
                return Err(Difference::Program);
            }
            let template = |function| FunctionTemplate {
                function,
                binds_name: false,
                outer: Rc::default(),
            };
            open.push((template(*a), template(*b)));
        }
    }
    let mut done = BTreeSet::new();
    while let Some((a, b)) = open.pop() {
        if !done.insert(a.function.to_raw()) {
            continue;
        }
        tree.expand(b.function).map_err(Difference::Expand)?;
        // Neue innere Funktionen bekommen Nummern, die zuzuordnen sind.
        mapping.update(tree)?;
        let position = full
            .function(a.function)
            .map_or(0, |function| function.range.start);
        let same = match (compile_function(full, &a), compile_function(tree, &b)) {
            (Ok(a), Ok(b)) => same_code(&a, &b, tree, &mapping, &mut open),
            (Err(a), Err(b)) => same_error(a, b),
            _ => false,
        };
        if !same {
            return Err(Difference::Code(position));
        }
    }
    Ok(())
}

fn same_error(a: CompileError, b: CompileError) -> bool {
    a == b
}

impl Mapping {
    /// Die Lagen der Funktionen und Klassen von `full`; zugeordnet ist
    /// noch nichts.
    fn new(full: &Ast) -> Result<Mapping, Difference> {
        let mut mapping = Mapping {
            full_functions: BTreeMap::new(),
            full_classes: BTreeMap::new(),
            functions: BTreeMap::new(),
            classes: BTreeMap::new(),
            taken_functions: BTreeSet::new(),
            taken_classes: BTreeSet::new(),
        };
        for index in 0..full.function_count() {
            let id = FunctionId::from_raw(raw(index)?);
            let range = full
                .function(id)
                .ok_or(Difference::Functions)?
                .range
                .clone();
            if mapping
                .full_functions
                .insert((range.start, range.end), id)
                .is_some()
            {
                return Err(Difference::Functions);
            }
        }
        for index in 0..full.class_count() {
            let id = ClassId::from_raw(raw(index)?);
            let range = full.class(id).ok_or(Difference::Functions)?.range.clone();
            if mapping
                .full_classes
                .insert((range.start, range.end), id)
                .is_some()
            {
                return Err(Difference::Functions);
            }
        }
        Ok(mapping)
    }

    /// Ordnet die Funktionen und Klassen von `tree` zu, die seit dem
    /// letzten Aufruf dazugekommen sind; das Nachlesen hängt sie nur an.
    fn update(&mut self, tree: &Ast) -> Result<(), Difference> {
        for index in self.functions.len()..tree.function_count() {
            let id = FunctionId::from_raw(raw(index)?);
            let range = &tree.function(id).ok_or(Difference::Functions)?.range;
            let &other = self
                .full_functions
                .get(&(range.start, range.end))
                .ok_or(Difference::Functions)?;
            if !self.taken_functions.insert(other.to_raw()) {
                return Err(Difference::Functions);
            }
            self.functions.insert(id.to_raw(), other);
        }
        for index in self.classes.len()..tree.class_count() {
            let id = ClassId::from_raw(raw(index)?);
            let range = &tree.class(id).ok_or(Difference::Functions)?.range;
            let &other = self
                .full_classes
                .get(&(range.start, range.end))
                .ok_or(Difference::Functions)?;
            if !self.taken_classes.insert(other.to_raw()) {
                return Err(Difference::Functions);
            }
            self.classes.insert(id.to_raw(), other);
        }
        Ok(())
    }

    /// Ob jede Funktion und Klasse von `full` zugeordnet ist.
    fn complete(&self) -> bool {
        self.functions.len() == self.full_functions.len()
            && self.classes.len() == self.full_classes.len()
    }
}

fn raw(index: usize) -> Result<u32, Difference> {
    u32::try_from(index).map_err(|_| Difference::Functions)
}

/// `template` aus `tree` mit der Nummer der Funktion im ganz gelesenen Baum.
fn mapped_template(
    template: &FunctionTemplate,
    _tree: &Ast,
    mapping: &Mapping,
) -> Option<FunctionTemplate> {
    Some(FunctionTemplate {
        function: *mapping.functions.get(&template.function.to_raw())?,
        ..template.clone()
    })
}

/// Ob `b` aus `tree` nach dem Umsetzen der Nummern `a` gleicht. Die
/// Funktionen in den Konstanten kommen nach `open`.
fn same_code(
    a: &Code,
    b: &Code,
    tree: &Ast,
    mapping: &Mapping,
    open: &mut Vec<(FunctionTemplate, FunctionTemplate)>,
) -> bool {
    let mut mapped = b.clone();
    for constant in &mut mapped.constants {
        match constant {
            Constant::Function(template) => {
                let Some(other) = mapped_template(template, tree, mapping) else {
                    return false;
                };
                *template = other;
            }
            Constant::DefaultConstructor { class, .. } => {
                let Some(&other) = mapping.classes.get(&class.to_raw()) else {
                    return false;
                };
                *class = other;
            }
            _ => {}
        }
    }
    if *a != mapped {
        return false;
    }
    for (a, b) in a.constants.iter().zip(&b.constants) {
        if let (Constant::Function(a), Constant::Function(b)) = (a, b) {
            open.push((a.clone(), b.clone()));
        }
    }
    true
}
