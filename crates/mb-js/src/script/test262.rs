//! Erste feste Test262-Auswahl für die Ausführung, nicht nur die Syntax.
//! Die Harness läuft unverändert, mit dem Host aus `host`. Nach jedem Script
//! leert der Läufer die Warteschlange der Jobs wie ein Browser; ein Test mit
//! dem Flag `async` besteht, wenn er danach mit `print` sein Ende gemeldet
//! hat ([`async_outcome`]). Ein Test mit dem Flag `module` läuft als Modul
//! ([`evaluate_module`]). Grenzen stehen in
//! `docs/m2-test262-ausfuehrung.md`.

use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::agent::{Abrupt, Agent, Budget, ErrorKind, Exception};
use crate::code_cache::{self, CodeStore, Uncacheable};
use crate::compare::same_value;
use crate::compiler::{CompileError, compile_script};
use crate::heap::{Local, ObjectClass, Options, Scope};
use crate::object::{get_own_property, get_prototype_of};
use crate::parser::parse_script;
use crate::promise::PromiseState;
use crate::realm::{Intrinsic, RealmId};
use crate::script::ScriptError;
use crate::value::Kind;

mod host;
mod language;
mod metadata;
#[cfg(test)]
mod tests;

use host::Test262Host;
use metadata::{Metadata, Mode, Negative};

const STEPS: u64 = 100_000;
const HEAP_BYTES: usize = 8 << 20;
/// So lange wartet der Läufer nach einem Script höchstens auf Zeitgeber
/// von `Atomics.waitAsync`.
const TIMER_WAIT: Duration = Duration::from_secs(1);
const SELECTION: &str = include_str!("test262/runtime.txt");

/// Ergebnisse bleiben ohne JavaScript-Aufrufe beobachtbar. Insbesondere
/// wird kein `constructor` oder `name` eines geworfenen Objekts gelesen;
/// es wird über seinen Prototyp eingeordnet ([`classify`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Normal,
    Syntax,
    Error(ErrorKind),
    /// Ein Fehler beim Laden oder Linken eines Moduls.
    Resolution(ErrorKind),
    Test262Error,
    ThrownValue,
    /// Ein Test mit `async` meldete einen Fehlschlag.
    AsyncFailure,
    /// Ein Test mit `async` meldete weder Erfolg noch Fehlschlag.
    AsyncIncomplete,
    Unsupported,
    CompileLimit,
    Host(Abrupt),
}

const ERROR_KINDS: [ErrorKind; 4] = [
    ErrorKind::RangeError,
    ErrorKind::ReferenceError,
    ErrorKind::SyntaxError,
    ErrorKind::TypeError,
];

/// Ordnet einen geworfenen Wert ein, wie INTERPRETING.md es für `negative`
/// verlangt („the name of the thrown exception's constructor“), ohne
/// JavaScript auszuführen: Ein Fehlerobjekt, dessen Prototyp der Prototyp
/// einer Fehlerart in einem Realm des Agents ist, zählt wie ein Fehler der
/// Engine dieser Art; ein Objekt mit `Test262Error.prototype` eines Realms
/// als Prototyp zählt als Test262Error. Gelesen werden nur Prototypen und
/// eigene Dateneigenschaften.
fn classify(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Outcome {
    let error = {
        let heap = agent.heap();
        let Kind::Object(object) = heap.get(value).kind() else {
            return Outcome::ThrownValue;
        };
        heap.object_class(object) == Some(ObjectClass::Error)
    };
    let Ok(Some(prototype)) = get_prototype_of(agent, scope, value) else {
        return Outcome::ThrownValue;
    };
    for index in 0..agent.realms.len() {
        let realm = RealmId::from_index(index);
        if error {
            for kind in ERROR_KINDS {
                let expected = agent.intrinsic(scope, realm, Intrinsic::error_prototype(kind));
                if same(agent, prototype, expected) {
                    return Outcome::Error(kind);
                }
            }
        }
        let global = agent.global_object(scope, realm);
        let expected = own_data(agent, scope, global, "Test262Error")
            .and_then(|constructor| own_data(agent, scope, constructor, "prototype"));
        if expected.is_some_and(|expected| same(agent, prototype, expected)) {
            return Outcome::Test262Error;
        }
    }
    Outcome::ThrownValue
}

fn same(agent: &Agent, left: Local<'_>, right: Local<'_>) -> bool {
    let heap = agent.heap();
    same_value(heap, heap.get(left), heap.get(right))
}

/// Der Wert der eigenen Dateneigenschaft `name`, ohne Getter.
fn own_data<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    object: Local<'_>,
    name: &str,
) -> Option<Local<'s>> {
    if !matches!(agent.heap().get(object).kind(), Kind::Object(_)) {
        return None;
    }
    let units: Vec<u16> = name.encode_utf16().collect();
    let key = agent.key_from_units(scope, &units).ok()?;
    get_own_property(agent, scope, object, key).ok()??.value
}

/// Ordnet die Ausnahme ein, die nach [`Abrupt::Throw`] aussteht.
fn thrown(agent: &mut Agent, scope: &Scope) -> Outcome {
    match agent.take_exception(scope) {
        Some(Exception::Error(kind)) => Outcome::Error(kind),
        Some(Exception::Value(value)) => classify(agent, scope, value),
        None => panic!("Throw ohne Ausnahme"),
    }
}

/// Ein Script und danach alle Jobs, die es und die Jobs selbst anstoßen,
/// mit derselben Quote, auch die der Zeitgeber von `Atomics.waitAsync`,
/// die in [`TIMER_WAIT`] ablaufen. Wirft ein Job, zählt das wie eine
/// Ausnahme des Scripts.
fn evaluate(agent: &mut Agent, realm: RealmId, source: &str, budget: &Rc<Budget>) -> Outcome {
    let source: Vec<u16> = source.encode_utf16().collect();
    let outcome = agent.scope(|agent, scope| {
        match agent.evaluate_script_within(scope, realm, &source, Rc::clone(budget)) {
            Ok(_) => match run_jobs_and_timers(agent, budget) {
                Ok(()) => Outcome::Normal,
                Err(Abrupt::Throw) => thrown(agent, scope),
                Err(Abrupt::Unsupported(_)) => Outcome::Unsupported,
                Err(abrupt) => Outcome::Host(abrupt),
            },
            Err(ScriptError::Syntax(_)) => Outcome::Syntax,
            Err(ScriptError::Unsupported(_)) => Outcome::Unsupported,
            Err(ScriptError::Abrupt(Abrupt::Throw)) => thrown(agent, scope),
            Err(ScriptError::Abrupt(abrupt)) => Outcome::Host(abrupt),
        }
    });
    assert_eq!(agent.heap_mut().push_registers(0), Some(0), "Register frei");
    agent.scope(|agent, scope| {
        assert!(agent.take_exception(scope).is_none(), "keine alte Ausnahme");
    });
    agent.heap().verify().expect("gültiger Heap nach Script");
    outcome
}

/// Ein Test mit dem Flag `module` aus der Datei `file`: ParseModule,
/// LoadRequestedModules, Link und Evaluate mit der Quote, danach die Jobs
/// und Zeitgeber wie bei einem Script. Ein Fehler beim Laden oder Linken
/// gehört zur Phase `resolution` ([`Outcome::Resolution`]), einer beim
/// Auswerten zur Phase `runtime`.
fn evaluate_module(
    agent: &mut Agent,
    host: &Test262Host,
    realm: RealmId,
    file: Option<&Path>,
    source: &str,
    budget: &Rc<Budget>,
) -> Outcome {
    let source: Vec<u16> = source.encode_utf16().collect();
    let outcome = agent.scope(|agent, scope| {
        agent.with_execution_budget(Rc::clone(budget), |agent| {
            module_steps(agent, scope, host, realm, file, &source, budget)
        })
    });
    assert_eq!(agent.heap_mut().push_registers(0), Some(0), "Register frei");
    agent.scope(|agent, scope| {
        assert!(agent.take_exception(scope).is_none(), "keine alte Ausnahme");
    });
    agent.heap().verify().expect("gültiger Heap nach Modul");
    outcome
}

fn module_steps(
    agent: &mut Agent,
    scope: &Scope,
    host: &Test262Host,
    realm: RealmId,
    file: Option<&Path>,
    source: &[u16],
    budget: &Rc<Budget>,
) -> Outcome {
    let module = match agent.parse_module(scope, realm, source) {
        Ok(module) => module,
        Err(ScriptError::Syntax(_)) => return Outcome::Syntax,
        Err(ScriptError::Unsupported(_)) => return Outcome::Unsupported,
        // Der einzige Wurf aus ParseModule: das Größenlimit des Übersetzers.
        Err(ScriptError::Abrupt(Abrupt::Throw)) => {
            agent.take_exception(scope);
            return Outcome::CompileLimit;
        }
        Err(ScriptError::Abrupt(abrupt)) => return Outcome::Host(abrupt),
    };
    if let Some(file) = file {
        host.set_test_module(file, module);
    }
    let resolution = |outcome| match outcome {
        Outcome::Error(kind) => Outcome::Resolution(kind),
        outcome => outcome,
    };
    let loaded = match agent.load_requested_modules(scope, module) {
        Ok(promise) => promise,
        Err(abrupt) => return resolution(abrupt_outcome(agent, scope, abrupt)),
    };
    if host.module_unsupported() {
        return Outcome::Unsupported;
    }
    if let Err(outcome) = settled(agent, scope, loaded) {
        return resolution(outcome);
    }
    if let Err(abrupt) = agent.link_module(module) {
        return resolution(abrupt_outcome(agent, scope, abrupt));
    }
    let evaluated = match agent.evaluate_module(scope, module) {
        Ok(promise) => promise,
        Err(abrupt) => return abrupt_outcome(agent, scope, abrupt),
    };
    if let Err(abrupt) = run_jobs_and_timers(agent, budget) {
        return abrupt_outcome(agent, scope, abrupt);
    }
    match settled(agent, scope, evaluated) {
        Ok(()) => Outcome::Normal,
        Err(outcome) => outcome,
    }
}

/// Das Ergebnis einer abrupten Beendigung.
fn abrupt_outcome(agent: &mut Agent, scope: &Scope, abrupt: Abrupt) -> Outcome {
    match abrupt {
        Abrupt::Throw => thrown(agent, scope),
        Abrupt::Unsupported(_) => Outcome::Unsupported,
        abrupt => Outcome::Host(abrupt),
    }
}

/// Ob das Promise erfüllt ist, sonst der Grund der Ablehnung eingeordnet;
/// ein noch offenes Promise ist nicht fertig geworden.
fn settled(agent: &mut Agent, scope: &Scope, promise: Local<'_>) -> Result<(), Outcome> {
    match agent.promise_state(promise) {
        Some(PromiseState::Fulfilled) => Ok(()),
        Some(PromiseState::Rejected) => match agent.promise_result(scope, promise) {
            Some(reason) => Err(classify(agent, scope, reason)),
            None => Err(Outcome::ThrownValue),
        },
        Some(PromiseState::Pending) | None => Err(Outcome::AsyncIncomplete),
    }
}

/// Lässt die Jobs laufen und wartet danach auf den nächsten Zeitgeber, bis
/// keiner mehr wartet oder der nächste erst nach [`TIMER_WAIT`] abliefe.
fn run_jobs_and_timers(agent: &mut Agent, budget: &Rc<Budget>) -> Result<(), Abrupt> {
    let end = Instant::now().checked_add(TIMER_WAIT);
    loop {
        agent.with_execution_budget(Rc::clone(budget), Agent::run_jobs)?;
        match (agent.next_timer(), end) {
            (Some(next), Some(end)) if next <= end => {
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
            }
            _ => return Ok(()),
        }
    }
}

/// Das Ergebnis eines Tests mit `async`, der normal endete, nach den
/// Ausgaben von `print` (INTERPRETING.md, Flag `async`): Er besteht, wenn er
/// `Test262:AsyncTestComplete` ausgab und nie etwas mit
/// `Test262:AsyncTestFailure:` am Anfang. Ohne beides ist er nicht fertig
/// geworden; länger zu warten hilft nicht, denn die Warteschlange ist leer,
/// und Zeitgeber gibt es nur für `Atomics.waitAsync`, auf die
/// [`run_jobs_and_timers`] schon gewartet hat.
fn async_outcome(printed: &[Vec<u16>]) -> Outcome {
    let failure: Vec<u16> = "Test262:AsyncTestFailure:".encode_utf16().collect();
    let complete: Vec<u16> = "Test262:AsyncTestComplete".encode_utf16().collect();
    if printed.iter().any(|line| line.starts_with(&failure)) {
        Outcome::AsyncFailure
    } else if printed.contains(&complete) {
        Outcome::Normal
    } else {
        Outcome::AsyncIncomplete
    }
}

fn expected(negative: Option<Negative>) -> Outcome {
    match negative {
        None => Outcome::Normal,
        Some(Negative::Parse) => Outcome::Syntax,
        Some(Negative::Runtime(kind)) => Outcome::Error(kind),
        Some(Negative::Test262Error) => Outcome::Test262Error,
        Some(Negative::Resolution(kind)) => Outcome::Resolution(kind),
    }
}

/// Ein Agent mit dem Host für Test262 und ein Realm je Variante; Harness und
/// Test teilen eine Quote. Setupfehler können niemals einen negativen Test
/// erfüllen.
fn run(
    source: &str,
    metadata: &Metadata,
    mode: Mode,
    harness: &[(String, String)],
    stress: bool,
) -> Result<Outcome, String> {
    run_file(None, source, metadata, mode, harness, stress)
}

/// Wie ein Test läuft: im Belastungsmodus des Heaps (`stress`) und mit
/// einem Speicher für übersetzte Scripts (`store`, siehe
/// [`crate::code_cache`]).
#[derive(Clone, Default)]
struct Setup {
    stress: bool,
    store: Option<Arc<dyn CodeStore>>,
}

/// Wie [`run`] für den Test in der Datei `file`: Ein Modul lädt seine
/// Hilfsdateien aus ihrem Verzeichnis.
fn run_file(
    file: Option<&Path>,
    source: &str,
    metadata: &Metadata,
    mode: Mode,
    harness: &[(String, String)],
    stress: bool,
) -> Result<Outcome, String> {
    let setup = Setup {
        stress,
        store: None,
    };
    run_file_with(file, source, metadata, mode, harness, &setup)
}

/// Die Hilfsdateien, deren Eintrag schon geprüft ist.
static CHECKED_HARNESS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Prüft für den Lauf mit Speicher, dass sich das Script `text` speichern
/// lässt, wenn der Übersetzer es kann, und dass sein Eintrag unverändert
/// zurückkommt. Sonst liefe es unbemerkt ohne Speicher.
fn check_entry(text: &str) -> Result<(), String> {
    let units: Vec<u16> = text.encode_utf16().collect();
    match code_cache::prepare(&units) {
        Ok(prepared) => {
            let loaded = code_cache::decode(&code_cache::encode(&prepared), &units)
                .map_err(|rejected| format!("Zwischenspeicher: {rejected}"))?;
            if loaded != prepared {
                return Err("Zwischenspeicher: geladen anders als gespeichert".to_owned());
            }
            Ok(())
        }
        Err(Uncacheable::Syntax | Uncacheable::Compile(_)) => Ok(()),
        Err(Uncacheable::Unsupported) => Err("Zwischenspeicher: nicht speicherbar".to_owned()),
    }
}

fn run_file_with(
    file: Option<&Path>,
    source: &str,
    metadata: &Metadata,
    mode: Mode,
    harness: &[(String, String)],
    setup: &Setup,
) -> Result<Outcome, String> {
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress: setup.stress,
        verify: true,
    })
    .map_err(|error| format!("Agent: {error:?}"))?;
    if let Some(store) = &setup.store {
        agent.set_code_store(Some(Arc::clone(store)));
        for (name, text) in harness {
            let fresh = CHECKED_HARNESS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(name.clone());
            if fresh {
                check_entry(text).map_err(|reason| format!("Harness {name}: {reason}"))?;
            }
        }
    }
    let host = Rc::new(Test262Host::default());
    agent.set_host(Rc::<Test262Host>::clone(&host));
    agent.set_can_block(metadata.can_block());
    let realm = agent
        .create_realm()
        .map_err(|error| format!("Realm: {error:?}"))?;
    if let Some(file) = file {
        host.set_test_file(file);
    }
    let budget = Rc::new(Budget::new(STEPS));
    if !metadata.raw() {
        for (name, text) in harness {
            let result = evaluate(&mut agent, realm, text, &budget);
            if result != Outcome::Normal {
                return Err(format!("Harness {name}: {result:?}"));
            }
        }
    }
    let text = match mode {
        Mode::Sloppy => source.to_owned(),
        Mode::Strict => format!("\"use strict\";\n{source}"),
        Mode::Module => {
            let result = evaluate_module(&mut agent, &host, realm, file, source, &budget);
            return Ok(finish(&mut agent, &host, metadata, result));
        }
    };
    if setup.store.is_some() {
        check_entry(&text)?;
    }
    // evaluate_script meldet ein Größenlimit des äußeren Compilers als
    // RangeError. Es darf keinen erwarteten *Laufzeit*-RangeError erfüllen.
    // Deshalb bei negativen Laufzeittests die frühe Phase separat prüfen.
    if matches!(
        metadata.negative,
        Some(Negative::Runtime(_) | Negative::Test262Error)
    ) {
        let units: Vec<u16> = text.encode_utf16().collect();
        let ast = match parse_script(&units) {
            Ok(ast) => ast,
            Err(_) => return Ok(Outcome::Syntax),
        };
        match compile_script(&ast) {
            Ok(_) => {}
            Err(CompileError::Unsupported(_)) => return Ok(Outcome::Unsupported),
            Err(CompileError::TooLarge) => return Ok(Outcome::CompileLimit),
        }
    }
    let result = evaluate(&mut agent, realm, &text, &budget);
    Ok(finish(&mut agent, &host, metadata, result))
}

/// Sammelt den Heap und ordnet einen Test mit `async` nach seinen Ausgaben
/// ein. Konnte der Übersetzer eine Hilfsdatei nicht, die erst `import()`
/// lud, ist der Test nicht unterstützt.
fn finish(agent: &mut Agent, host: &Test262Host, metadata: &Metadata, result: Outcome) -> Outcome {
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap nach Sammeln");
    if host.module_unsupported() {
        return Outcome::Unsupported;
    }
    if metadata.is_async() && result == Outcome::Normal {
        return async_outcome(&host.printed());
    }
    result
}

fn harness(metadata: &Metadata) -> Vec<(String, String)> {
    if metadata.raw() {
        return Vec::new();
    }
    let mut files: Vec<_> = ["assert.js", "sta.js"]
        .into_iter()
        .chain(metadata.is_async().then_some("doneprintHandle.js"))
        .map(|name| {
            (
                name.to_owned(),
                testdata::read("test262", &format!("harness/{name}")),
            )
        })
        .collect();
    for name in &metadata.includes {
        files.push((
            name.clone(),
            testdata::read("test262", &format!("harness/{name}")),
        ));
    }
    files
}

/// Keine implizite Erweiterung durch Verzeichnisabfragen: Jeder neue Fall
/// wird mit Pfad und Anforderungen in der eingecheckten Liste geprüft.
fn selection() -> Vec<&'static str> {
    let files: Vec<_> = SELECTION
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let mut seen = BTreeSet::new();
    for file in &files {
        assert!(
            file.starts_with("test/language/")
                && file.ends_with(".js")
                && !file.contains("_FIXTURE")
                && metadata::relative_path(file),
            "Testpfad: {file}"
        );
        assert!(seen.insert(*file), "doppelter Testpfad: {file}");
    }
    assert!(!files.is_empty(), "keine Laufzeittests gewählt");
    files
}

fn proposals() -> BTreeSet<String> {
    let text = testdata::read("test262", "features.txt");
    let mut proposed = false;
    let mut names = BTreeSet::new();
    for line in text.lines().map(str::trim) {
        match line {
            "## Proposed language features" => proposed = true,
            "## Standard language features" => proposed = false,
            _ => {}
        }
        let name = line.split('#').next().unwrap_or_default().trim();
        if proposed && !name.is_empty() {
            names.insert(name.to_owned());
        }
    }
    names
}

#[test]
fn test262_execution() {
    let mut failures = Vec::new();
    let mut runs = 0;
    let mut runtime_negative = 0;
    let files = selection();
    let proposals = proposals();
    for file in &files {
        let source = testdata::read("test262", file);
        let metadata = Metadata::read(&source).unwrap_or_else(|error| panic!("{file}: {error}"));
        assert!(
            metadata
                .features
                .iter()
                .all(|feature| !proposals.contains(feature)),
            "Vorschläge gehören nicht in diese M2-Auswahl: {file}"
        );
        // Syntax-Negativtests zählen bereits in parser/test262.rs. Diese
        // Auswahl soll Ausführung messen, keine Parserzahlen aufaddieren.
        assert_ne!(metadata.negative, Some(Negative::Parse), "{file}");
        let harness = harness(&metadata);
        runtime_negative += usize::from(metadata.negative.is_some());
        for mode in metadata.modes() {
            for stress in [false, true] {
                runs += 1;
                let path = testdata::root().join("test262").join(file);
                let result = run_file(Some(&path), &source, &metadata, mode, &harness, stress);
                if result != Ok(expected(metadata.negative)) {
                    failures.push(format!("{file} ({mode:?}, stress={stress}): {result:?}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} Fehlschläge in {runs} Läufen:\n{}",
        failures.len(),
        failures.join("\n")
    );
    println!(
        "Test262-Ausführung: {} Dateien, {runs} Läufe mit zwei GC-Modi, \
         {runtime_negative} Dateien mit erwartetem Laufzeitfehler",
        files.len()
    );
}
