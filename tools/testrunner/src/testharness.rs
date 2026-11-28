//! Tests mit testharness.js in einer JavaScript-Shell (Schritt 14a, siehe
//! `docs/m2-wpt-testharness.md`).
//!
//! Ein Lauf folgt dem Wrapper der WPT für klassische Worker
//! (`ClassicWorkerHandler` in `tools/serve/serve.py`), nur dass
//! `GLOBAL.isWorker()` *false* ergibt: `self.GLOBAL`, testharness.js, an
//! Stelle von testharnessreport.js das eigene `report.js`, dann die Zeilen
//! `script=` und `title=` aus den `META`-Zeilen, die Testdatei und
//! `done()`. Weil es weder `document` noch einen `WorkerGlobalScope` gibt,
//! wählt testharness.js seine `ShellTestEnvironment`. Wirft eines der
//! Skripte, laufen die übrigen nicht, wie bei `importScripts`; danach
//! läuft die Ereignisschleife, bis `report.js` das Ende meldet.

mod host;
mod meta;
#[cfg(test)]
mod tests;

pub use meta::{Include, Meta, parse_meta};

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use mb_js::agent::{Abrupt, Agent, Budget, ErrorKind, Exception};
use mb_js::heap::{Local, Options, Scope};
use mb_js::operations::{call, get, is_callable};
use mb_js::realm::RealmId;
use mb_js::script::ScriptError;
use mb_js::value::Value;

use host::{Event, ShellHost, Timer};

/// Die Arbeitsschritte je Datei, für Laden, Tests und Ereignisschleife
/// zusammen; `timeout=long` bekommt das Sechsfache, wie die WPT ihre
/// Zeitgrenze verlängern.
pub const STEPS: u64 = 50_000_000;
/// Der Speicher des Heaps je Datei.
pub const HEAP_BYTES: usize = 64 << 20;
/// Die Zeitgrenze der virtuellen Uhr in Millisekunden, wie die
/// Zeitgrenzen der WPT: 10 s, mit `timeout=long` 60 s.
const TIME_LIMIT: u64 = 10_000;
const LONG_TIME_LIMIT: u64 = 60_000;

/// Das erste Skript, wie im Wrapper der WPT für Worker.
const PRELUDE: &str = "self.GLOBAL = {
  isWindow: function() { return false; },
  isWorker: function() { return false; },
  isShadowRealm: function() { return false; },
};
";
const REPORT: &str = include_str!("testharness/report.js");

/// Der Status eines Untertests, wie `Test.prototype.PASS` und die
/// folgenden in testharness.js.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    Timeout,
    NotRun,
    PreconditionFailed,
}

const STATUSES: [(Status, &str); 5] = [
    (Status::Pass, "PASS"),
    (Status::Fail, "FAIL"),
    (Status::Timeout, "TIMEOUT"),
    (Status::NotRun, "NOTRUN"),
    (Status::PreconditionFailed, "PRECONDITION_FAILED"),
];

impl Status {
    fn from_number(number: f64) -> Option<Status> {
        STATUSES
            .iter()
            .enumerate()
            .find(|(index, _)| u32::try_from(*index).is_ok_and(|index| f64::from(index) == number))
            .map(|(_, (status, _))| *status)
    }

    /// Der Name, wie ihn die WPT schreiben.
    pub fn name(self) -> &'static str {
        STATUSES
            .iter()
            .find(|(status, _)| *status == self)
            .map_or("", |(_, name)| name)
    }
}

/// Der Status der Harness, wie `TestsStatus.prototype.OK` und die
/// folgenden in testharness.js.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HarnessStatus {
    Ok,
    Error,
    Timeout,
    PreconditionFailed,
}

const HARNESS_STATUSES: [(HarnessStatus, &str); 4] = [
    (HarnessStatus::Ok, "OK"),
    (HarnessStatus::Error, "ERROR"),
    (HarnessStatus::Timeout, "TIMEOUT"),
    (HarnessStatus::PreconditionFailed, "PRECONDITION_FAILED"),
];

impl HarnessStatus {
    fn from_number(number: f64) -> Option<HarnessStatus> {
        HARNESS_STATUSES
            .iter()
            .enumerate()
            .find(|(index, _)| u32::try_from(*index).is_ok_and(|index| f64::from(index) == number))
            .map(|(_, (status, _))| *status)
    }

    /// Der Name, wie ihn die WPT schreiben.
    pub fn name(self) -> &'static str {
        HARNESS_STATUSES
            .iter()
            .find(|(status, _)| *status == self)
            .map_or("", |(_, name)| name)
    }
}

/// Ein Untertest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subtest {
    pub name: String,
    pub status: Status,
    pub message: Option<String>,
}

/// Das Ergebnis einer Datei.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileResult {
    pub harness: HarnessStatus,
    pub message: Option<String>,
    pub subtests: Vec<Subtest>,
}

impl FileResult {
    fn failed(harness: HarnessStatus, message: impl Into<String>, subtests: Vec<Subtest>) -> Self {
        FileResult {
            harness,
            message: Some(message.into()),
            subtests,
        }
    }
}

/// Woher der Läufer die Dateien der WPT liest. Pfade sind relativ zur
/// Wurzel der WPT, ohne `/` am Anfang.
pub trait Files {
    fn read(&self, path: &str) -> Option<String>;
}

/// Die Dateien eines Verzeichnisses, etwa `target/testdata/wpt`.
pub struct Directory {
    pub root: PathBuf,
}

impl Files for Directory {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(path)).ok()
    }
}

/// Löst `reference` aus einer Zeile `script=` der Datei `base` auf: mit
/// `/` am Anfang relativ zur Wurzel, sonst zum Verzeichnis von `base`.
/// Anfrage und Fragment fallen weg. `None`, wenn der Pfad über die Wurzel
/// hinausführt.
pub fn resolve(base: &str, reference: &str) -> Option<String> {
    let reference = reference.split(['?', '#']).next().unwrap_or("");
    let mut segments: Vec<&str> = Vec::new();
    let relative = match reference.strip_prefix('/') {
        Some(rest) => rest,
        None => {
            segments.extend(base.split('/'));
            // Der Name der Datei selbst gehört nicht zum Verzeichnis.
            segments.pop();
            reference
        }
    };
    for segment in relative.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            segment => segments.push(segment),
        }
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

/// Ein Skript des Wrappers: sein Name für Meldungen und sein Quelltext,
/// oder `Err`, wenn es fehlt.
struct Script {
    name: String,
    source: Result<String, String>,
}

/// Die Skripte des Wrappers für die Datei `path` mit dem Quelltext
/// `source`.
fn wrapper(files: &dyn Files, path: &str, meta: &Meta, source: &str) -> Vec<Script> {
    let fixed = |name: &str, source: &str| Script {
        name: name.to_owned(),
        source: Ok(source.to_owned()),
    };
    let read = |name: &str| Script {
        name: format!("/{name}"),
        source: files
            .read(name)
            .ok_or_else(|| format!("Skript fehlt: /{name}")),
    };
    let mut scripts = vec![
        fixed("self.GLOBAL", PRELUDE),
        read("resources/testharness.js"),
        fixed("report.js", REPORT),
    ];
    for include in &meta.includes {
        scripts.push(match include {
            Include::Script(reference) => match resolve(path, reference) {
                Some(resolved) => read(&resolved),
                None => Script {
                    name: reference.clone(),
                    source: Err(format!("Pfad außerhalb der WPT: {reference}")),
                },
            },
            // Wie `_script_replacement` in serve.py.
            Include::Title(title) => {
                let escaped = title.replace('\\', "\\\\").replace('"', "\\\"");
                fixed("title", &format!("self.META_TITLE = \"{escaped}\";"))
            }
        });
    }
    scripts.push(fixed(path, source));
    scripts.push(fixed("done", "done();"));
    scripts
}

/// Warum ein Lauf endet.
#[derive(Clone, Debug, PartialEq, Eq)]
enum End {
    /// `report.js` hat das Ende gemeldet.
    Completed,
    /// Nichts wartet mehr, aber die Harness wurde nicht fertig.
    Idle,
    /// Der nächste Zeitgeber läge hinter der Zeitgrenze.
    TimeLimit(u64),
    Exhausted,
    OutOfMemory,
    Unsupported(usize),
    /// Eine Ausnahme kam an einer Stelle an, an der der Läufer keine
    /// erwartet, etwa beim Anlegen eines Ereignisobjekts.
    Unexpected,
}

impl End {
    /// Warum der Lauf so endete, für die Meldung.
    fn reason(&self) -> String {
        match self {
            End::Completed => "fertig".to_owned(),
            End::Idle => "Die Harness wurde nicht fertig, und nichts wartet mehr".to_owned(),
            End::TimeLimit(limit) => format!("Zeitgrenze von {} s erreicht", limit / 1000),
            End::Exhausted => "Arbeitsschritte erschöpft".to_owned(),
            End::OutOfMemory => "Speicher erschöpft".to_owned(),
            End::Unsupported(position) => format!("Nicht unterstützt (Lage {position})"),
            End::Unexpected => "Unerwartete Ausnahme im Läufer".to_owned(),
        }
    }

    fn from_abrupt(abrupt: Abrupt) -> End {
        match abrupt {
            Abrupt::Throw => End::Unexpected,
            Abrupt::Exhausted(_) => End::Exhausted,
            Abrupt::OutOfMemory => End::OutOfMemory,
            Abrupt::Unsupported(position) => End::Unsupported(position),
        }
    }
}

/// Was ein Skript warf.
enum Thrown {
    /// Eine Ausnahme; der Agent hält sie.
    Exception,
    /// Ein früher Fehler an dieser Lage.
    Syntax(usize),
    /// Das Skript fehlt, mit der Meldung dazu.
    Missing(String),
}

/// Was ein Lauf braucht.
struct Run {
    agent: Agent,
    host: Rc<ShellHost>,
    realm: RealmId,
    budget: Rc<Budget>,
    /// Die Meldungen der Ausnahmen, die kein Skript fing, in ihrer
    /// Reihenfolge.
    uncaught: Vec<String>,
    /// Warum der Läufer `timeout()` rief, siehe [`Run::time_out`].
    timed_out: Option<String>,
}

/// Führt die Datei `path` aus.
pub fn run_file(files: &dyn Files, path: &str) -> FileResult {
    run_limited(files, path, STEPS)
}

/// Wie [`run_file`] mit `steps` Arbeitsschritten, für `timeout=long` dem
/// Sechsfachen.
pub fn run_limited(files: &dyn Files, path: &str, steps: u64) -> FileResult {
    let Some(source) = files.read(path) else {
        return FileResult::failed(HarnessStatus::Error, "Datei fehlt", Vec::new());
    };
    let meta = parse_meta(&source);
    let Ok(mut agent) = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress: false,
        verify: false,
    }) else {
        return FileResult::failed(HarnessStatus::Error, "Agent: Speicher", Vec::new());
    };
    let host = Rc::new(ShellHost::new());
    agent.set_host(Rc::<ShellHost>::clone(&host));
    let Ok(realm) = agent.create_realm() else {
        return FileResult::failed(HarnessStatus::Error, "Realm: Speicher", Vec::new());
    };
    let (steps, limit) = if meta.long_timeout {
        (steps.saturating_mul(6), LONG_TIME_LIMIT)
    } else {
        (steps, TIME_LIMIT)
    };
    let mut run = Run {
        agent,
        host,
        realm,
        budget: Rc::new(Budget::new(steps)),
        uncaught: Vec::new(),
        timed_out: None,
    };
    let end = match run.load(wrapper(files, path, &meta, &source)) {
        Ok(()) => run.event_loop(limit),
        Err(end) => end,
    };
    let end = match end {
        End::Idle | End::TimeLimit(_) | End::Exhausted => run.time_out(end, steps, limit),
        end => end,
    };
    run.finish(end)
}

impl Run {
    /// Führt die Skripte des Wrappers nacheinander aus, ohne Microtasks
    /// dazwischen, bis eines wirft oder fehlt.
    fn load(&mut self, scripts: Vec<Script>) -> Result<(), End> {
        for script in scripts {
            let thrown = match script.source {
                Ok(source) => self.evaluate(&source)?,
                Err(message) => Some(Thrown::Missing(message)),
            };
            if let Some(thrown) = thrown {
                self.report(&script.name, thrown)?;
                return Ok(());
            }
        }
        Ok(())
    }

    /// Ein Skript; was es warf oder `None`.
    fn evaluate(&mut self, source: &str) -> Result<Option<Thrown>, End> {
        let units: Vec<u16> = source.encode_utf16().collect();
        let realm = self.realm;
        let budget = Rc::clone(&self.budget);
        let result = self.agent.scope(|agent, scope| {
            agent
                .evaluate_script_within(scope, realm, &units, budget)
                .map(|_| ())
        });
        match result {
            Ok(()) => Ok(None),
            Err(ScriptError::Syntax(error)) => Ok(Some(Thrown::Syntax(error.position))),
            Err(ScriptError::Unsupported(position)) => Err(End::Unsupported(position)),
            Err(ScriptError::Abrupt(Abrupt::Throw)) => Ok(Some(Thrown::Exception)),
            Err(ScriptError::Abrupt(abrupt)) => Err(End::from_abrupt(abrupt)),
        }
    }

    /// Meldet, was `script` warf, mit einem Ereignis `error`, wie „report
    /// an exception“ in HTML: Das Ereignis trägt den Wert und als Meldung
    /// ToString davon. Ein Fehler der Engine wird dafür zum Fehlerobjekt,
    /// ein früher Fehler zu einem SyntaxError, ein fehlendes Skript zu
    /// einem Ereignis ohne Wert.
    fn report(&mut self, script: &str, thrown: Thrown) -> Result<(), End> {
        let realm = self.realm;
        let host = Rc::clone(&self.host);
        let budget = Rc::clone(&self.budget);
        let (message, result) = self.agent.scope(|agent, scope| {
            let value = match &thrown {
                Thrown::Exception => match agent.take_exception(scope) {
                    Some(Exception::Value(value)) => Ok(Some(value)),
                    Some(Exception::Error(kind)) => {
                        host::engine_error(agent, scope, realm, kind).map(Some)
                    }
                    None => Ok(None),
                },
                Thrown::Syntax(_) => {
                    host::engine_error(agent, scope, realm, ErrorKind::SyntaxError).map(Some)
                }
                Thrown::Missing(_) => Ok(None),
            };
            let value = match value {
                Ok(value) => value,
                Err(abrupt) => return (String::new(), Err(abrupt)),
            };
            let message = match (&thrown, value) {
                (Thrown::Missing(message), _) => message.clone(),
                (Thrown::Syntax(position), _) => {
                    format!("SyntaxError in {script} bei Codeeinheit {position}")
                }
                (Thrown::Exception, Some(value)) => agent
                    .with_execution_budget(Rc::clone(&budget), |agent| {
                        host::text(agent, scope, value)
                    })
                    .unwrap_or_else(|_| {
                        let _ = agent.take_exception(scope);
                        format!("Ausnahme ohne Text in {script}")
                    }),
                (Thrown::Exception, None) => format!("Ausnahme in {script}"),
            };
            let undefined = agent.heap().local(scope, Value::UNDEFINED);
            let result =
                host::error_event(agent, scope, realm, &message, value.unwrap_or(undefined))
                    .and_then(|event| {
                        dispatch(agent, scope, &host, realm, budget, Event::Error, event)
                    });
            (message, result)
        });
        if !message.is_empty() {
            self.uncaught.push(message);
        }
        self.listener_result(result)
    }

    /// Wirft ein Listener, meldet HTML das wieder mit einem Ereignis
    /// `error`. Der Läufer merkt es sich nur, damit ein werfender
    /// Listener keine Schleife auslöst.
    fn listener_result(&mut self, result: Result<(), Abrupt>) -> Result<(), End> {
        match result {
            Ok(()) => Ok(()),
            Err(Abrupt::Throw) => {
                let text = self
                    .agent
                    .scope(|agent, scope| match agent.take_exception(scope) {
                        Some(Exception::Error(kind)) => format!("{kind:?}"),
                        _ => "Ausnahme".to_owned(),
                    });
                self.uncaught.push(format!("in einem Listener: {text}"));
                Ok(())
            }
            Err(abrupt) => Err(End::from_abrupt(abrupt)),
        }
    }

    /// Microtasks, bis keine mehr warten, danach die Ereignisse
    /// `unhandledrejection` für abgelehnte Promises ohne Handler (HTML,
    /// „perform a microtask checkpoint“ mit „notify about rejected
    /// promises“).
    fn checkpoint(&mut self) -> Result<(), End> {
        loop {
            loop {
                let budget = Rc::clone(&self.budget);
                match self.agent.with_execution_budget(budget, Agent::run_jobs) {
                    Ok(()) => break,
                    Err(Abrupt::Throw) => self.report("einem Job", Thrown::Exception)?,
                    Err(abrupt) => return Err(End::from_abrupt(abrupt)),
                }
            }
            let rejected = self.host.take_rejected();
            if rejected.is_empty() {
                return Ok(());
            }
            for promise in rejected {
                let realm = self.realm;
                let host = Rc::clone(&self.host);
                let budget = Rc::clone(&self.budget);
                let result = self.agent.scope(|agent, scope| {
                    let local = agent
                        .heap()
                        .local(scope, agent.heap().get_persistent(&promise));
                    agent.heap_mut().release(promise);
                    let reason = agent
                        .promise_result(scope, local)
                        .unwrap_or_else(|| agent.heap().local(scope, Value::UNDEFINED));
                    let event = host::rejection_event(agent, scope, realm, local, reason)?;
                    dispatch(
                        agent,
                        scope,
                        &host,
                        realm,
                        budget,
                        Event::UnhandledRejection,
                        event,
                    )
                });
                self.listener_result(result)?;
            }
        }
    }

    /// Die Ereignisschleife: Microtasks, dann der nächste Zeitgeber, bis
    /// `report.js` das Ende meldet, nichts mehr wartet oder der nächste
    /// Zeitgeber hinter `limit` läge.
    fn event_loop(&mut self, limit: u64) -> End {
        loop {
            if let Err(end) = self.checkpoint() {
                return end;
            }
            if self.host.is_completed() {
                return End::Completed;
            }
            let Some(timer) = self.host.take_next_timer() else {
                return End::Idle;
            };
            if timer.due > limit {
                host::release(&mut self.agent, timer);
                return End::TimeLimit(limit);
            }
            self.host.advance(timer.due);
            if let Err(end) = self.fire(timer) {
                return end;
            }
        }
    }

    /// Ruft den Handler eines Zeitgebers mit dem globalen Objekt als
    /// `this` und seinen Argumenten.
    fn fire(&mut self, timer: Timer) -> Result<(), End> {
        let realm = self.realm;
        let budget = Rc::clone(&self.budget);
        let result = self.agent.scope(|agent, scope| {
            let heap = agent.heap();
            let callback = heap.local(scope, heap.get_persistent(&timer.callback));
            let arguments: Vec<Local<'_>> = timer
                .arguments
                .iter()
                .map(|argument| heap.local(scope, heap.get_persistent(argument)))
                .collect();
            host::release(agent, timer);
            let global = agent.global_object(scope, realm);
            agent.with_execution_budget(budget, |agent| {
                call(agent, scope, callback, global, &arguments).map(|_| ())
            })
        });
        match result {
            Ok(()) => Ok(()),
            Err(Abrupt::Throw) => self.report("einem Zeitgeber", Thrown::Exception),
            Err(abrupt) => Err(End::from_abrupt(abrupt)),
        }
    }

    /// Wird die Harness nicht von selbst fertig, ruft der Läufer
    /// `timeout()` aus testharness.js, wie ein Läufer der WPT bei seiner
    /// äußeren Zeitgrenze: In der Shell hat testharness.js keine eigene
    /// (`timeout_length` ist *null*), und genau dann setzt `timeout()` den
    /// Status der Harness auf TIMEOUT, wenn er noch keinen hat, und schließt
    /// ab. Dafür gibt es ein neues Zehntel der Arbeitsschritte und noch
    /// einmal die Zeitgrenze. Endet auch das nicht, bleibt es bei `end`,
    /// außer bei einem Ende, das schwerer wiegt, etwa ohne Speicher.
    fn time_out(&mut self, end: End, steps: u64, limit: u64) -> End {
        self.timed_out = Some(end.reason());
        self.budget = Rc::new(Budget::new(steps / 10));
        let realm = self.realm;
        let budget = Rc::clone(&self.budget);
        let result = self.agent.scope(|agent, scope| {
            let global = agent.global_object(scope, realm);
            let units: Vec<u16> = "timeout".encode_utf16().collect();
            let key = agent.key_from_units(scope, &units)?;
            agent.with_execution_budget(budget, |agent| {
                let function = get(agent, scope, global, key)?;
                if !is_callable(agent.heap(), agent.heap().get(function)) {
                    return Ok(false);
                }
                call(agent, scope, function, global, &[]).map(|_| true)
            })
        });
        match result {
            Ok(true) => {}
            Ok(false) => return end,
            Err(Abrupt::Throw) => {
                if let Err(fatal) = self.report("timeout()", Thrown::Exception) {
                    return fatal;
                }
            }
            Err(Abrupt::Exhausted(_)) => return end,
            Err(abrupt) => return End::from_abrupt(abrupt),
        }
        let limit = self.host.now().saturating_add(limit);
        match self.event_loop(limit) {
            End::Idle | End::TimeLimit(_) | End::Exhausted => end,
            other => other,
        }
    }

    fn finish(mut self, end: End) -> FileResult {
        self.host.release_all(&mut self.agent);
        let finished = self.host.take_finished();
        let harness = match end {
            End::Completed => {
                return match self.host.take_completed() {
                    Some(completed) => FileResult {
                        harness: completed.status,
                        message: completed.message.or(self.timed_out),
                        subtests: completed.subtests,
                    },
                    None => {
                        FileResult::failed(HarnessStatus::Error, "Ende ohne Ergebnis", finished)
                    }
                };
            }
            // Warf etwas, bevor die Harness stand, ist das der Grund.
            End::Idle if !self.uncaught.is_empty() => {
                let message = self.uncaught.swap_remove(0);
                return FileResult::failed(HarnessStatus::Error, message, finished);
            }
            End::Idle | End::TimeLimit(_) | End::Exhausted => HarnessStatus::Timeout,
            End::OutOfMemory | End::Unsupported(_) | End::Unexpected => HarnessStatus::Error,
        };
        FileResult::failed(harness, end.reason(), finished)
    }
}

/// Ruft die Listener für `event` mit dem Ereignisobjekt `object`.
fn dispatch(
    agent: &mut Agent,
    scope: &Scope,
    host: &ShellHost,
    realm: RealmId,
    budget: Rc<Budget>,
    event: Event,
    object: Local<'_>,
) -> Result<(), Abrupt> {
    let listeners = host.listeners(agent, scope, event);
    let global = agent.global_object(scope, realm);
    agent.with_execution_budget(budget, |agent| {
        for listener in listeners {
            call(agent, scope, listener, global, &[object])?;
        }
        Ok(())
    })
}

/// Wie eine Datei ausgewählt wird.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// Nur Dateien mit `jsshell` in `global=`.
    Declared,
    /// Alle Dateien `*.any.js`, als eigene Variante in der Shell, auch
    /// wenn die WPT sie dort nicht vorsehen.
    Own,
}

/// Die Verzeichnisse, deren Dateien `*.any.js` laufen, relativ zur Wurzel
/// der WPT.
pub const AREAS: [(&str, Selection); 2] = [
    ("wasm/jsapi/", Selection::Declared),
    ("js/builtins/weakrefs/", Selection::Own),
];

/// Die Dateien aus [`AREAS`] unter `root`, sortiert.
pub fn select(root: &Path) -> Vec<String> {
    let files = Directory {
        root: root.to_owned(),
    };
    let mut selected = Vec::new();
    for (area, selection) in AREAS {
        for path in any_files(root, area) {
            let include = match selection {
                Selection::Own => true,
                Selection::Declared => files
                    .read(&path)
                    .is_some_and(|source| parse_meta(&source).for_shell()),
            };
            if include {
                selected.push(path);
            }
        }
    }
    selected.sort();
    selected
}

/// Alle Dateien `*.any.js` unter dem Verzeichnis `area`, ohne Rekursion.
fn any_files(root: &Path, area: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![area.trim_end_matches('/').to_owned()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&directory)) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let path = format!("{directory}/{name}");
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(path),
                Ok(kind) if kind.is_file() && name.ends_with(".any.js") => found.push(path),
                _ => {}
            }
        }
    }
    found
}

/// Der Stapel je Arbeitsfaden. Die Engine begrenzt ihre Tiefe selbst
/// (`STACK_BUDGET` in mb-js); das hier ist nur reichlich Platz dafür.
const STACK: usize = 64 << 20;

/// Führt `paths` unter `root` aus, auf allen Kernen, und gibt die
/// Ergebnisse in der Reihenfolge von `paths` zurück.
pub fn run_all(root: &Path, paths: &[String]) -> Vec<(String, FileResult)> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, FileResult)>> = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(paths.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let builder = std::thread::Builder::new().stack_size(STACK);
            let spawned = builder.spawn_scoped(scope, || {
                let files = Directory {
                    root: root.to_owned(),
                };
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(index) else {
                        break;
                    };
                    let result = run_file(&files, path);
                    results
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push((index, result));
                }
            });
            if let Err(error) = spawned {
                eprintln!("Faden: {error}");
            }
        }
    });
    let mut results = results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    results.sort_by_key(|(index, _)| *index);
    results
        .into_iter()
        .filter_map(|(index, result)| paths.get(index).map(|path| (path.clone(), result)))
        .collect()
}

/// Der Kopf der Datei der erwarteten Ergebnisse.
const HEADER: &str = "\
# Erwartete Ergebnisse der Tests mit testharness.js in der Shell, Pfade
# relativ zur Wurzel der WPT. Je Datei steht der Status der Harness, wenn
# er nicht OK ist, und jeder Untertest, der nicht besteht, mit seinem
# Status; Tabulatoren trennen die Felder. Unter wasm/jsapi/ laufen die
# Dateien mit global=jsshell, unter js/builtins/weakrefs/ alle als eigene
# Variante. Erzeugt von tools/testrunner (testharness.rs); bei einer
# Abweichung schreibt der Test die neue Fassung nach
# target/testrunner/testharness-erwartet.txt.
";

/// Die erwarteten Ergebnisse für `results`, im Format der Datei
/// `testharness-erwartet.txt`.
pub fn render_expectations(results: &[(String, FileResult)]) -> String {
    let files = results.len();
    let harness_ok = results
        .iter()
        .filter(|(_, result)| result.harness == HarnessStatus::Ok)
        .count();
    let subtests: usize = results
        .iter()
        .map(|(_, result)| result.subtests.len())
        .sum();
    let passed: usize = results
        .iter()
        .map(|(_, result)| {
            result
                .subtests
                .iter()
                .filter(|subtest| subtest.status == Status::Pass)
                .count()
        })
        .sum();
    let mut text = HEADER.to_owned();
    text.push_str(&format!(
        "# {files} Dateien, Harness OK in {harness_ok}; bestanden {passed} von {subtests} Untertests\n"
    ));
    for (path, result) in results {
        if result.harness != HarnessStatus::Ok {
            text.push_str(&format!("{path}\t{}\n", result.harness.name()));
        }
        for subtest in &result.subtests {
            if subtest.status != Status::Pass {
                text.push_str(&format!(
                    "{path}\t{}\t{}\n",
                    subtest.status.name(),
                    escape(&subtest.name)
                ));
            }
        }
    }
    text
}

/// Schützt Tabulatoren, Zeilenwechsel und `\` im Namen eines Untertests.
fn escape(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for character in name.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            character => escaped.push(character),
        }
    }
    escaped
}

/// Die erwarteten Ergebnisse im Repository.
pub const EXPECTATIONS: &str = include_str!("../testharness-erwartet.txt");
