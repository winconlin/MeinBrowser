//! Übersetzter Bytecode von Scripts zum Speichern und Wiederladen.
//!
//! Siehe `docs/architektur.md`, Abschnitt 4.9: Wer ein Script ein zweites
//! Mal ausführt, soll es nicht noch einmal parsen und übersetzen müssen.
//! Dafür gibt es einen Speicher ([`CodeStore`]) am Agent
//! ([`Agent::set_code_store`](crate::agent::Agent::set_code_store)). Bei
//! jedem Script fragt der Agent ihn mit dem Schlüssel des Quelltexts
//! ([`key`]). Hat er einen Eintrag, lädt der Agent ihn ([`decode`]) und
//! führt ihn ohne Syntaxbaum aus. Sonst parst und übersetzt er das Script
//! wie immer, übersetzt dazu gleich alle Funktionen darin ([`prepare`]),
//! legt das Ergebnis im Speicher ab ([`encode`]) und führt es aus.
//!
//! # Was ein Eintrag enthält
//!
//! Den Bytecode des Scripts mit den Namen für
//! GlobalDeclarationInstantiation, den Bytecode jeder seiner Funktionen und
//! je Funktion, was die Laufzeit sonst aus dem Syntaxbaum liest: die Art,
//! ob Generator oder `async`, `length` und wo ihr Quelltext für `toString`
//! liegt. Kann der Übersetzer eine Funktion nicht, steht dort der Fehler;
//! er kommt wie ohne Speicher erst beim ersten Aufruf. Den Quelltext selbst
//! enthält der Eintrag nicht, ihn hat der Agent ohnehin.
//!
//! Nicht gespeichert werden Module, Code aus `eval` und `Function` und
//! Scripts, die der Übersetzer nicht ganz kann. Sie laufen wie bisher.
//!
//! # Prüfung beim Laden
//!
//! Ein Eintrag kann beschädigt oder untergeschoben sein, spätestens wenn er
//! auf der Platte liegt. Der Interpreter verlässt sich aber darauf, dass
//! Bytecode vom Übersetzer stammt. [`decode`] nimmt darum nur Einträge an,
//! deren Kopf zu Format und Quelltext passt, die genau bis zum Ende
//! reichen, und deren Bytecode alle Regeln erfüllt, die der Übersetzer
//! einhält (siehe `verify.rs`): Befehle lesbar, Register im Rahmen,
//! Konstanten der passenden Art, Sprünge auf Befehlsanfänge und Lagen im
//! Quelltext. Alles andere ist ein Fehlschlag, und der Agent übersetzt das
//! Script neu. Das Laden rekursiert nicht und legt höchstens linear zur
//! Länge des Eintrags Speicher an.

#![forbid(unsafe_code)]

mod format;
mod store;
mod verify;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;
use std::rc::Rc;

use mb_crypto::{Builtin, Digest, HashAlgorithm, Provider};

use crate::ast::{Ast, ClassId, FunctionId};
use crate::bytecode::{Code, Constant, FunctionTemplate};
use crate::compiler::{CompileError, Script, compile_function, compile_script};
use crate::parser::parse_script;
use crate::unit::{Shape, function_text};

#[cfg(test)]
pub(crate) use store::DiskStore;
pub use store::{CodeStore, MemoryStore};

/// Die Fassung des Formats. Sie steht im Kopf jedes Eintrags und geht in
/// den Schlüssel ein; ändert sich das Format, muss sie steigen.
pub const FORMAT: u32 = 2;

/// Die Kennung dieses Baus der Engine: SHA-256 über die Quellen von `mb-js`
/// und den Crates, von denen es abhängt (`build.rs`). Sie steht wie
/// [`FORMAT`] im Kopf und geht in den Schlüssel ein, also findet eine
/// andere Engine die Einträge dieser nie, auch nicht auf der Platte, und
/// ein geänderter Übersetzer braucht keine neue Fassung.
pub const ENGINE: [u8; 32] = include!(concat!(env!("OUT_DIR"), "/engine.rs"));

/// Wie lang der Kopf eines Eintrags ist: die Kennung `MBJSCODE`, die
/// Fassung, die Kennung der Engine und der Schlüssel.
pub const HEADER: usize = 8 + 4 + 32 + 32;

/// Wie viele Stufen die Kette einer Funktion im Eintrag höchstens hat. Der
/// Übersetzer legt je umgebende Funktion höchstens eine an, dazu eine für
/// das Script, und der Parser verschachtelt Funktionen höchstens
/// [`MAX_DEPTH`](crate::parser::MAX_DEPTH) tief. Tiefere Ketten lehnt der
/// Leser ab, bevor er sie aufbaut.
const MAX_LEVELS: usize = crate::parser::MAX_DEPTH as usize + 1;

/// Was vor dem Quelltext in den Schlüssel eingeht.
const DOMAIN: &[u8] = b"MeinBrowser mb-js Bytecode\0";

/// Ein Script mit allen Funktionen übersetzt, ohne Syntaxbaum.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledScript {
    pub(crate) source: Vec<u16>,
    pub(crate) script: Script,
    /// Die Funktionen nach ihrer Nummer aufsteigend, jede einmal.
    pub(crate) functions: Vec<(FunctionId, CompiledFunction)>,
    /// Die Klassen mit Standardkonstruktor nach ihrer Nummer aufsteigend,
    /// mit Anfang und Ende ihres Quelltexts.
    pub(crate) classes: Vec<(ClassId, u32, u32)>,
}

/// Eine übersetzte Funktion mit dem, was die Laufzeit über sie wissen muss.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompiledFunction {
    pub(crate) shape: Shape,
    /// Anfang und Ende des Quelltexts für `toString`.
    pub(crate) text: (u32, u32),
    pub(crate) code: Result<Rc<Code>, CompileError>,
}

impl CompiledFunction {
    pub(crate) fn text(&self) -> Range<usize> {
        let (start, end) = self.text;
        to_usize(start)..to_usize(end)
    }
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Warum ein Eintrag abgelehnt wurde.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejected {
    reason: &'static str,
}

impl Rejected {
    pub(crate) const fn new(reason: &'static str) -> Rejected {
        Rejected { reason }
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl fmt::Display for Rejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Eintrag abgelehnt: {}", self.reason)
    }
}

/// Warum sich ein Script nicht speichern lässt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Uncacheable {
    /// Es hat einen Syntaxfehler.
    Syntax,
    /// Der Übersetzer kann das Script selbst nicht.
    Compile(CompileError),
    /// Etwas darin lässt sich nicht speichern, etwa eine Funktion, die an
    /// zwei Stellen verschieden übersetzt wird.
    Unsupported,
}

impl CompiledScript {
    pub(crate) fn source(&self) -> &[u16] {
        &self.source
    }

    pub(crate) fn script(&self) -> &Script {
        &self.script
    }

    pub(crate) fn function(&self, id: FunctionId) -> Option<&CompiledFunction> {
        let index = self
            .functions
            .binary_search_by_key(&id.to_raw(), |(function, _)| function.to_raw())
            .ok()?;
        self.functions.get(index).map(|(_, function)| function)
    }

    pub(crate) fn class_text(&self, id: ClassId) -> Option<Range<usize>> {
        let index = self
            .classes
            .binary_search_by_key(&id.to_raw(), |&(class, _, _)| class.to_raw())
            .ok()?;
        let &(_, start, end) = self.classes.get(index)?;
        Some(to_usize(start)..to_usize(end))
    }

    /// Übersetzt alle Funktionen des Scripts `script` aus dem Syntaxbaum
    /// `ast`, ohne Rekursion über eine Liste offener Stellen. Lässt sich
    /// das Ergebnis nicht speichern, kommt das Script zurück.
    pub(crate) fn from_script(ast: &Ast, script: Script) -> Result<CompiledScript, Box<Script>> {
        match collect(ast, &script) {
            Some((functions, classes)) => {
                let compiled = CompiledScript {
                    source: ast.source().to_vec(),
                    script,
                    functions,
                    classes,
                };
                // Was die Prüfung beim Laden ablehnte, würde nie getroffen.
                match verify::verify(&compiled) {
                    Ok(()) => Ok(compiled),
                    Err(_) => Err(Box::new(compiled.script)),
                }
            }
            None => Err(Box::new(script)),
        }
    }
}

type Collected = (
    Vec<(FunctionId, CompiledFunction)>,
    Vec<(ClassId, u32, u32)>,
);

/// Die Funktionen und Standardkonstruktoren, die der Code des Scripts und
/// seiner Funktionen nennt, übersetzt. `None`, wenn eine Funktion an zwei
/// Stellen verschieden übersetzt würde oder eine Lage nicht in 32 Bit
/// passt.
fn collect(ast: &Ast, script: &Script) -> Option<Collected> {
    let mut functions: BTreeMap<u32, (FunctionTemplate, CompiledFunction)> = BTreeMap::new();
    let mut classes: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut open: Vec<FunctionTemplate> = script
        .functions
        .iter()
        .map(|&(_, function)| FunctionTemplate {
            function,
            binds_name: false,
            outer: Rc::default(),
        })
        .collect();
    let mut push_constants = |code: &Code, open: &mut Vec<FunctionTemplate>| -> Option<()> {
        for constant in code.constants() {
            match constant {
                Constant::Function(template) => open.push(template.clone()),
                Constant::DefaultConstructor { class, .. } => {
                    let range = ast.class(*class)?.range.clone();
                    let text = (
                        u32::try_from(range.start).ok()?,
                        u32::try_from(range.end).ok()?,
                    );
                    classes.insert(class.to_raw(), text);
                }
                _ => {}
            }
        }
        Some(())
    };
    push_constants(&script.code, &mut open)?;
    while let Some(template) = open.pop() {
        let id = template.function.to_raw();
        if let Some((known, compiled)) = functions.get(&id) {
            if *known == template {
                continue;
            }
            // Dieselbe Funktion an einer anderen Stelle: Gespeichert wird
            // ein Code je Funktion, also muss er gleich sein.
            let same = match (&compiled.code, compile_function(ast, &template)) {
                (Ok(code), Ok(other)) => **code == other,
                (Err(error), Err(other)) => *error == other,
                _ => false,
            };
            if !same {
                return None;
            }
            continue;
        }
        let function = ast.function(template.function)?;
        let range = function_text(ast, template.function)?;
        let text = (
            u32::try_from(range.start).ok()?,
            u32::try_from(range.end).ok()?,
        );
        let code = compile_function(ast, &template).map(Rc::new);
        if let Ok(code) = &code {
            push_constants(code, &mut open)?;
        }
        let compiled = CompiledFunction {
            shape: Shape::of(ast, function),
            text,
            code,
        };
        functions.insert(id, (template, compiled));
    }
    let functions = functions
        .into_iter()
        .map(|(id, (_, compiled))| (FunctionId::from_raw(id), compiled))
        .collect();
    let classes = classes
        .into_iter()
        .map(|(id, (start, end))| (ClassId::from_raw(id), start, end))
        .collect();
    Some((functions, classes))
}

/// Parst und übersetzt `source` als Script mit allen Funktionen, so wie der
/// Agent es bei einem Fehlschlag im Speicher tut.
pub fn prepare(source: &[u16]) -> Result<CompiledScript, Uncacheable> {
    let ast = parse_script(source).map_err(|_| Uncacheable::Syntax)?;
    let script = compile_script(&ast).map_err(Uncacheable::Compile)?;
    CompiledScript::from_script(&ast, script).map_err(|_| Uncacheable::Unsupported)
}

/// Der Schlüssel für `source`: SHA-256 über eine feste Kennung, die Fassung
/// des Formats, die Kennung der Engine und den Quelltext in UTF-16LE.
pub fn key(source: &[u16]) -> Digest {
    let mut context = Builtin.hash(HashAlgorithm::Sha256);
    context.update(DOMAIN);
    context.update(&FORMAT.to_le_bytes());
    context.update(&ENGINE);
    let mut buffer = Vec::with_capacity(8192);
    for chunk in source.chunks(4096) {
        buffer.clear();
        buffer.extend(chunk.iter().flat_map(|unit| unit.to_le_bytes()));
        context.update(&buffer);
    }
    context.finish()
}

/// Der Eintrag für `script`.
pub fn encode(script: &CompiledScript) -> Vec<u8> {
    format::encode(script, &key(&script.source))
}

/// Lädt den Eintrag `bytes` für den Quelltext `source` und prüft ihn.
pub fn decode(bytes: &[u8], source: &[u16]) -> Result<CompiledScript, Rejected> {
    decode_with_key(bytes, source, &key(source))
}

/// Dasselbe mit dem schon berechneten Schlüssel von `source`.
pub(crate) fn decode_with_key(
    bytes: &[u8],
    source: &[u16],
    key: &Digest,
) -> Result<CompiledScript, Rejected> {
    let compiled = format::decode(bytes, source, key)?;
    verify::verify(&compiled)?;
    Ok(compiled)
}

/// Dasselbe wie [`encode`] mit dem schon berechneten Schlüssel.
pub(crate) fn encode_with_key(script: &CompiledScript, key: &Digest) -> Vec<u8> {
    format::encode(script, key)
}
