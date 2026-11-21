//! `WeakMap`, `WeakSet`, `WeakRef` und `FinalizationRegistry` über
//! JavaScript, gegen ein Modell der Erreichbarkeit.
//!
//! Je vier Bytes beschreiben einen Schritt, höchstens [`MAX_STEPS`]: die
//! Art (Byte 0 modulo 12), einen Platz (Byte 1), einen zweiten Platz oder
//! eine WeakRef (Byte 2) und Schalter (Byte 3). Das Script hält acht Plätze
//! `S`, zwei WeakMaps, ein WeakSet, acht Plätze für WeakRefs und zwei
//! FinalizationRegistries, alle global. Die Schritte legen Objekte und
//! Symbole an, geben Plätze frei, setzen und löschen Kanten zwischen
//! Objekten (Eigenschaften `e0` bis `e3`), setzen und löschen Einträge der
//! WeakMaps und des WeakSet, legen WeakRefs an, holen ihr Ziel zurück auf
//! einen Platz, melden Ziele mit gehaltenem Wert und Abmeldemarke an und
//! melden sie ab. Die Art 11 und das Ende der Eingabe beenden eine Runde,
//! höchstens [`MAX_ROUNDS`]: Rust lässt die Jobs laufen (also
//! ClearKeptObjects), sammelt, lässt die Aufräum-Jobs laufen und prüft mit
//! einem zweiten Script.
//!
//! Das Modell kennt die Plätze, die Kanten, die Einträge und [[KeptAlive]]
//! und rechnet nach jedem Sammeln aus, was erreichbar ist; ein Eintrag
//! einer WeakMap hält seinen Wert nur, solange sein Schlüssel erreichbar
//! ist. Geprüft wird:
//!
//! - Jeder Schritt gibt zurück oder wirft, was das Modell verlangt, etwa
//!   `delete`, `unregister` und ein TypeError, wenn Ziel und gehaltener
//!   Wert gleich sind.
//! - Nach dem Sammeln liefert jede WeakRef genau dann ihr Ziel, wenn es
//!   erreichbar war, `get` jeder WeakMap und `has` des WeakSet stimmen für
//!   jeden belegten Platz, und die Callbacks haben genau die gehaltenen
//!   Werte der Records bekommen, deren Ziel nicht mehr erreichbar war.
//! - Ein Ziel, das eine WeakRef zurückholt, ist erreichbar, in
//!   [[KeptAlive]] oder schon gesammelt; ein Ziel, das seit dem letzten
//!   Sammeln unerreichbar geworden ist, kann je nach Collector noch leben,
//!   also lässt das Modell diesen Schritt aus.
//!
//! Erlaubt sind bestandene Scripts und das Ende einer Quote. Jede achte
//! Eingabe (nach der Summe ihrer Bytes) läuft außerdem mit ihren ersten
//! [`STRESS_STEPS`] Schritten im Belastungsmodus des Collectors, der vor
//! jeder Anforderung sammelt. Dort leert er Ziele auch mitten in einer
//! Runde, und die Aufräum-Jobs können früher laufen; geprüft wird nach dem
//! Sammeln am Ende einer Runde dasselbe. Die Zahl der Arbeitsschritte hängt
//! dabei vom Collector ab, denn jedes Sammeln, das ein Ziel leert, stellt
//! einen Aufräum-Job ein; darum vergleicht das Ziel die beiden Läufe nicht,
//! jeder prüft sich selbst.

use std::fmt::Write as _;
use std::rc::Rc;

use mb_js::agent::{Abrupt, Agent, Budget, Exception, Exhausted};
use mb_js::heap::Options;
use mb_js::script::ScriptError;
use mb_js::value::Kind;

const SCRIPT_STEPS: u64 = 200_000;
const HEAP_BYTES: usize = 4 << 20;
/// Jede so vielte Eingabe läuft auch im Belastungsmodus.
const STRESS_EVERY: u32 = 8;
/// So viele Schritte laufen im Belastungsmodus.
const STRESS_STEPS: usize = 16;
/// Höchstens so viele Schritte liest das Ziel aus einer Eingabe.
const MAX_STEPS: usize = 64;
/// Höchstens so viele Runden, jede mit Sammeln und Prüfen.
const MAX_ROUNDS: usize = 6;
const STEP_BYTES: usize = 4;
const SLOTS: usize = 8;
const REFS: usize = 8;
const MAPS: usize = 2;
const REGISTRIES: usize = 2;
const EDGES: usize = 4;

const PRELUDE: &str = r#"
var S = [], R = [], L = [];
var M = [new WeakMap(), new WeakMap()], W = new WeakSet();
function fail(what) { throw "Fehler: " + what; }
function name(v) {
  if (v === undefined) return "u";
  if (typeof v === "object") return "o" + v.id;
  if (typeof v === "symbol") return "s" + v.description;
  return "n" + v;
}
function record(h) { L.push(name(h)); }
var F = [new FinalizationRegistry(record), new FinalizationRegistry(record)];
function throwsType(f, what) {
  try { f(); } catch (e) { if (e instanceof TypeError) return; throw e; }
  fail("kein TypeError: " + what);
}
true
"#;

/// Was ein Platz, ein Wert einer WeakMap oder ein gehaltener Wert sein kann.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Node(usize),
    Number(usize),
}

/// Ein Objekt oder ein Symbol des Scripts.
struct Node {
    /// Die Zahl im Namen, `o<id>` oder `s<id>`.
    id: usize,
    symbol: bool,
    edges: [Option<usize>; EDGES],
    /// Gesammelt; das bleibt so.
    dead: bool,
}

/// Ein Record einer FinalizationRegistry.
struct Record {
    target: usize,
    held: Held,
    token: Option<usize>,
}

#[derive(Default)]
struct Model {
    nodes: Vec<Node>,
    slots: [Option<usize>; SLOTS],
    maps: [Vec<(usize, Held)>; MAPS],
    set: Vec<usize>,
    refs: [Option<usize>; REFS],
    registries: [Vec<Record>; REGISTRIES],
    /// [[KeptAlive]] seit dem letzten ClearKeptObjects.
    kept: Vec<usize>,
    /// Die Namen, die die Callbacks seit der letzten Prüfung bekommen.
    log: Vec<String>,
}

impl Model {
    fn name(&self, held: Held) -> String {
        match held {
            Held::Node(node) => {
                let node = &self.nodes[node];
                format!("{}{}", if node.symbol { 's' } else { 'o' }, node.id)
            }
            Held::Number(number) => format!("n{number}"),
        }
    }

    fn slot(&self, index: u8) -> Option<usize> {
        self.slots[usize::from(index) % SLOTS]
    }

    /// Was von den Plätzen, den gehaltenen Werten und, wenn `kept`, aus
    /// [[KeptAlive]] erreichbar ist, über Kanten und Einträge der WeakMaps.
    fn reachable(&self, kept: bool) -> Vec<bool> {
        let mut live = vec![false; self.nodes.len()];
        let mut stack: Vec<usize> = self.slots.iter().flatten().copied().collect();
        if kept {
            stack.extend(&self.kept);
        }
        for registry in &self.registries {
            for record in registry {
                if let Held::Node(node) = record.held {
                    stack.push(node);
                }
            }
        }
        loop {
            while let Some(node) = stack.pop() {
                if live[node] {
                    continue;
                }
                live[node] = true;
                stack.extend(self.nodes[node].edges.iter().flatten());
            }
            for map in &self.maps {
                for &(key, value) in map {
                    if let (true, Held::Node(value)) = (live[key], value)
                        && !live[value]
                    {
                        stack.push(value);
                    }
                }
            }
            if stack.is_empty() {
                return live;
            }
        }
    }

    /// Das Sammeln nach ClearKeptObjects mit den Aufräum-Jobs danach.
    fn collect(&mut self) {
        self.kept.clear();
        let live = self.reachable(false);
        for (node, live) in self.nodes.iter_mut().zip(&live) {
            if !live {
                node.dead = true;
            }
        }
        for map in &mut self.maps {
            map.retain(|&(key, _)| live[key]);
        }
        self.set.retain(|&node| live[node]);
        for index in 0..REGISTRIES {
            let records = std::mem::take(&mut self.registries[index]);
            for mut record in records {
                if !live[record.target] {
                    let name = self.name(record.held);
                    self.log.push(name);
                    continue;
                }
                if record.token.is_some_and(|token| !live[token]) {
                    record.token = None;
                }
                self.registries[index].push(record);
            }
        }
    }

    fn add(&mut self, symbol: bool, id: usize) -> usize {
        self.nodes.push(Node {
            id,
            symbol,
            edges: [None; EDGES],
            dead: false,
        });
        self.nodes.len() - 1
    }

    /// Ein Schritt als JavaScript, leer, wenn er ausfällt.
    fn step(&mut self, index: usize, step: [u8; 4]) -> String {
        let [kind, a, b, c] = step;
        let slot = usize::from(a) % SLOTS;
        let other = usize::from(b) % SLOTS;
        let target = self.slot(a);
        let source = self.slot(b);
        let held = match source {
            Some(node) if c & 8 != 0 => Held::Node(node),
            _ => Held::Number(index),
        };
        let held_text = match held {
            Held::Node(_) => format!("S[{other}]"),
            Held::Number(number) => number.to_string(),
        };
        let at = format!("Schritt {index}");
        match kind % 12 {
            0 | 1 => {
                let symbol = kind % 12 == 1;
                let node = self.add(symbol, index);
                self.slots[slot] = Some(node);
                if symbol {
                    format!("S[{slot}] = Symbol(\"{index}\");")
                } else {
                    format!("S[{slot}] = {{ id: {index} }};")
                }
            }
            2 => {
                self.slots[slot] = None;
                format!("S[{slot}] = undefined;")
            }
            3 => {
                let Some(node) = target.filter(|&node| !self.nodes[node].symbol) else {
                    return String::new();
                };
                let edge = usize::from(c >> 1) % EDGES;
                self.nodes[node].edges[edge] = source;
                format!("S[{slot}].e{edge} = S[{other}];")
            }
            4 => {
                let Some(key) = target else {
                    return format!(
                        "throwsType(function () {{ M[{}].set(undefined, 1); }}, \"{at}\");",
                        c % 2
                    );
                };
                let map = &mut self.maps[usize::from(c) % MAPS];
                match map.iter_mut().find(|(existing, _)| *existing == key) {
                    Some(entry) => entry.1 = held,
                    None => map.push((key, held)),
                }
                format!(
                    "if (M[{}].set(S[{slot}], {held_text}) !== M[{}]) fail(\"{at}\");",
                    c % 2,
                    c % 2
                )
            }
            5 => {
                let map = &mut self.maps[usize::from(c) % MAPS];
                let before = map.len();
                if let Some(key) = target {
                    map.retain(|&(existing, _)| existing != key);
                }
                let deleted = map.len() != before;
                format!(
                    "if (M[{}].delete(S[{slot}]) !== {deleted}) fail(\"{at}\");",
                    c % 2
                )
            }
            6 => {
                let Some(node) = target else {
                    return format!("if (W.has(S[{slot}]) !== false) fail(\"{at}\");");
                };
                let present = self.set.contains(&node);
                if c % 2 == 0 {
                    if !present {
                        self.set.push(node);
                    }
                    format!("if (W.add(S[{slot}]) !== W) fail(\"{at}\");")
                } else {
                    self.set.retain(|&existing| existing != node);
                    format!("if (W.delete(S[{slot}]) !== {present}) fail(\"{at}\");")
                }
            }
            7 => {
                let reference = usize::from(b) % REFS;
                let Some(node) = target else {
                    return format!(
                        "throwsType(function () {{ new WeakRef(S[{slot}]); }}, \"{at}\");"
                    );
                };
                self.refs[reference] = Some(node);
                self.kept.push(node);
                format!("R[{reference}] = new WeakRef(S[{slot}]);")
            }
            8 => {
                let reference = usize::from(b) % REFS;
                let result = match self.refs[reference] {
                    None => None,
                    Some(node) if self.nodes[node].dead => None,
                    Some(node) => {
                        if !self.reachable(true)[node] {
                            // Kann leben oder schon gesammelt sein.
                            return String::new();
                        }
                        self.kept.push(node);
                        Some(node)
                    }
                };
                self.slots[slot] = result;
                let expected =
                    result.map_or_else(|| "u".to_owned(), |node| self.name(Held::Node(node)));
                format!(
                    "S[{slot}] = R[{reference}] === undefined ? undefined : R[{reference}].deref(); \
                     if (name(S[{slot}]) !== \"{expected}\") fail(\"{at}\");"
                )
            }
            9 => {
                let registry = usize::from(c) % REGISTRIES;
                let Some(node) = target else {
                    return format!(
                        "throwsType(function () {{ F[{registry}].register(S[{slot}], 1); }}, \"{at}\");"
                    );
                };
                if held == Held::Node(node) {
                    return format!(
                        "throwsType(function () {{ F[{registry}].register(S[{slot}], S[{slot}]); }}, \"{at}\");"
                    );
                }
                let token_slot = usize::from(b >> 3) % SLOTS;
                let token = if c & 16 != 0 {
                    self.slots[token_slot]
                } else {
                    None
                };
                self.registries[registry].push(Record {
                    target: node,
                    held,
                    token,
                });
                let token_text = match token {
                    Some(_) => format!("S[{token_slot}]"),
                    None => "undefined".to_owned(),
                };
                format!(
                    "if (F[{registry}].register(S[{slot}], {held_text}, {token_text}) !== undefined) \
                     fail(\"{at}\");"
                )
            }
            10 => {
                let registry = usize::from(c) % REGISTRIES;
                let Some(node) = target else {
                    return format!(
                        "throwsType(function () {{ F[{registry}].unregister(S[{slot}]); }}, \"{at}\");"
                    );
                };
                let records = &mut self.registries[registry];
                let before = records.len();
                records.retain(|record| record.token != Some(node));
                let removed = records.len() != before;
                format!("if (F[{registry}].unregister(S[{slot}]) !== {removed}) fail(\"{at}\");")
            }
            _ => String::new(),
        }
    }

    /// Das Script, das nach dem Sammeln prüft. Es holt jedes lebende Ziel
    /// zurück; die Jobs danach leeren [[KeptAlive]] wieder.
    fn check(&mut self, round: usize) -> String {
        let quoted = |names: Vec<String>| {
            let names: Vec<String> = names.iter().map(|name| format!("\"{name}\"")).collect();
            format!("[{}]", names.join(", "))
        };
        let refs: Vec<String> = self
            .refs
            .iter()
            .map(|reference| match reference {
                Some(node) if !self.nodes[*node].dead => self.name(Held::Node(*node)),
                _ => "u".to_owned(),
            })
            .collect();
        let maps: Vec<String> = self
            .maps
            .iter()
            .map(|map| {
                quoted(
                    self.slots
                        .iter()
                        .map(|slot| {
                            slot.and_then(|node| map.iter().find(|(key, _)| *key == node))
                                .map_or_else(|| "u".to_owned(), |&(_, value)| self.name(value))
                        })
                        .collect(),
                )
            })
            .collect();
        let set: Vec<String> = self
            .slots
            .iter()
            .map(|slot| {
                slot.is_some_and(|node| self.set.contains(&node))
                    .to_string()
            })
            .collect();
        let mut log = std::mem::take(&mut self.log);
        log.sort();
        self.kept.clear();
        let mut text = String::new();
        let _ = write!(
            text,
            "(function () {{\n\
             var refs = {refs}, maps = [{maps}], set = [{set}];\n\
             for (var i = 0; i < {REFS}; i++) {{\n\
               var r = R[i] === undefined ? undefined : R[i].deref();\n\
               if (name(r) !== refs[i]) fail(\"Runde {round}: WeakRef \" + i + \" ist \" + name(r));\n\
             }}\n\
             for (var i = 0; i < {SLOTS}; i++) {{\n\
               if (S[i] === undefined) continue;\n\
               for (var m = 0; m < {MAPS}; m++) {{\n\
                 if (name(M[m].get(S[i])) !== maps[m][i]) fail(\"Runde {round}: WeakMap \" + m + \" an \" + i);\n\
                 if (M[m].has(S[i]) !== (maps[m][i] !== \"u\")) fail(\"Runde {round}: has \" + m + \" an \" + i);\n\
               }}\n\
               if (W.has(S[i]) !== set[i]) fail(\"Runde {round}: WeakSet an \" + i);\n\
             }}\n\
             var log = L.splice(0, L.length).sort().join();\n\
             if (log !== \"{log}\") fail(\"Runde {round}: Callbacks \" + log);\n\
             return true;\n\
             }})()",
            refs = quoted(refs),
            maps = maps.join(", "),
            set = set.join(", "),
            log = log.join(","),
        );
        text
    }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Passed,
    Exhausted(Exhausted),
    OutOfMemory,
}

/// Die Scripts einer Eingabe: das Vorspiel, dann je Runde die Schritte und
/// die Prüfung. Zwischen beiden sammelt [`run`].
fn scripts(data: &[u8], limit: usize) -> Vec<(String, String)> {
    let mut model = Model::default();
    let mut scripts = Vec::new();
    let mut steps = String::from("(function () {\n");
    let mut round = 0;
    let (steps_data, _) = data.as_chunks::<STEP_BYTES>();
    for (index, &step) in steps_data.iter().take(limit).enumerate() {
        if step[0] % 12 == 11 {
            steps.push_str("return true;\n})()");
            model.collect();
            scripts.push((std::mem::take(&mut steps), model.check(round)));
            round += 1;
            if round == MAX_ROUNDS {
                return scripts;
            }
            steps.push_str("(function () {\n");
            continue;
        }
        let text = model.step(index, step);
        if !text.is_empty() {
            steps.push_str(&text);
            steps.push('\n');
        }
    }
    steps.push_str("return true;\n})()");
    model.collect();
    scripts.push((steps, model.check(round)));
    scripts
}

/// Führt ein Script aus. `None`, wenn es bestand, sonst das Ergebnis.
fn evaluate(
    agent: &mut Agent,
    realm: mb_js::realm::RealmId,
    source: &str,
    budget: &Rc<Budget>,
) -> Option<Outcome> {
    let source: Vec<u16> = source.encode_utf16().collect();
    agent.scope(|agent, scope| {
        let result = agent.evaluate_script_within(scope, realm, &source, Rc::clone(budget));
        let outcome = match result {
            Ok(value) => match agent.heap().get(value).kind() {
                Kind::Boolean(true) => None,
                kind => panic!("unerwartetes Ergebnis: {kind:?}"),
            },
            Err(ScriptError::Abrupt(abrupt)) => Some(abrupt_outcome(agent, scope, abrupt, budget)),
            Err(error) => panic!("das Script selbst ist gültig: {error:?}"),
        };
        assert!(agent.take_exception(scope).is_none(), "keine alte Ausnahme");
        outcome
    })
}

fn abrupt_outcome(
    agent: &mut Agent,
    scope: &mb_js::heap::Scope,
    abrupt: Abrupt,
    budget: &Budget,
) -> Outcome {
    match abrupt {
        Abrupt::Exhausted(reason) => {
            assert_eq!(budget.exhausted(), Some(reason), "Quote des Hosts");
            Outcome::Exhausted(reason)
        }
        Abrupt::OutOfMemory => Outcome::OutOfMemory,
        Abrupt::Throw => match agent.take_exception(scope) {
            Some(Exception::Value(value)) => match agent.heap().get(value).kind() {
                Kind::String(string) => panic!(
                    "{}",
                    String::from_utf16_lossy(&agent.heap().string_units(string))
                ),
                kind => panic!("geworfen: {kind:?}"),
            },
            Some(Exception::Error(kind)) => panic!("Fehler der Engine: {kind:?}"),
            None => panic!("geworfen ohne Ausnahme"),
        },
        abrupt => panic!("unerwarteter Abbruch: {abrupt:?}"),
    }
}

/// Lässt die Jobs laufen. `None`, wenn alle bestanden.
fn jobs(agent: &mut Agent, budget: &Rc<Budget>) -> Option<Outcome> {
    let result = agent.with_execution_budget(Rc::clone(budget), Agent::run_jobs);
    let outcome = agent.scope(|agent, scope| match result {
        Ok(()) => None,
        Err(abrupt) => Some(abrupt_outcome(agent, scope, abrupt, budget)),
    });
    assert!(!agent.has_pending_jobs() || outcome.is_some(), "Jobs übrig");
    outcome
}

fn run(scripts: &[(String, String)], stress: bool) -> Outcome {
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress,
        verify: true,
    })
    .expect("Agent für schwache Sammlungen");
    let realm = agent.create_realm().expect("Realm für schwache Sammlungen");
    let budget = Rc::new(Budget::new(SCRIPT_STEPS));
    let outcome = (|| {
        if let Some(outcome) = evaluate(&mut agent, realm, PRELUDE, &budget) {
            return outcome;
        }
        for (steps, check) in scripts {
            if let Some(outcome) =
                evaluate(&mut agent, realm, steps, &budget).or_else(|| jobs(&mut agent, &budget))
            {
                return outcome;
            }
            agent.heap_mut().collect();
            if let Some(outcome) = jobs(&mut agent, &budget)
                .or_else(|| evaluate(&mut agent, realm, check, &budget))
                .or_else(|| jobs(&mut agent, &budget))
            {
                return outcome;
            }
        }
        Outcome::Passed
    })();
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
    outcome
}

pub fn weak(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let _ = run(&scripts(data, MAX_STEPS), false);
    let sum = data
        .iter()
        .fold(0u32, |sum, &byte| sum.wrapping_add(u32::from(byte)));
    if sum % STRESS_EVERY == 0 {
        let _ = run(&scripts(data, STRESS_STEPS), true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(steps: &[[u8; 4]]) -> Vec<u8> {
        steps.iter().flatten().copied().collect()
    }

    #[test]
    fn rounds_reach_the_engine_and_its_checks() {
        // Objekte 0 und 1 mit einer Kante 0 → 1, 1 als Schlüssel mit 0 als
        // Wert, WeakRefs auf beide, 1 angemeldet mit 0 als Abmeldemarke;
        // dann beide Plätze frei.
        let data = steps(&[
            [0, 0, 0, 0],
            [0, 1, 0, 0],
            [3, 0, 1, 0],
            [4, 1, 0, 8],
            [7, 0, 0, 0],
            [7, 1, 1, 0],
            [9, 1, 2, 16 | 8],
            [11, 0, 0, 0],
            [2, 0, 0, 0],
            [2, 1, 0, 0],
            [11, 0, 0, 0],
            [8, 2, 0, 0],
            [8, 3, 1, 0],
        ]);
        let scripts = scripts(&data, MAX_STEPS);
        assert_eq!(scripts.len(), 3);
        assert_eq!(run(&scripts, false), Outcome::Passed);
        assert_eq!(run(&scripts, true), Outcome::Passed);
        let check = &scripts[0].1;
        assert!(
            check.contains("var refs = [\"o0\", \"o1\", \"u\""),
            "{check}"
        );
        let check = &scripts[1].1;
        assert!(check.contains("var refs = [\"u\", \"u\""), "{check}");
        assert!(check.contains("if (log !== \"n6\")"), "{check}");
    }

    #[test]
    fn ephemeron_chains_and_symbols_follow_the_model() {
        // Eine Kette über beide WeakMaps: Schlüssel 0 → Wert 1, Schlüssel 1
        // → Wert 2, ein Symbol. Nur Platz 0 hält die Kette; danach holen
        // WeakRefs 1 und das Symbol zurück, und Platz 0 wird frei.
        let data = steps(&[
            [0, 0, 0, 0],
            [0, 1, 0, 0],
            [1, 2, 0, 0],
            [4, 0, 1, 8],
            [4, 1, 2, 9],
            [7, 1, 0, 0],
            [7, 2, 1, 0],
            [9, 2, 0, 1],
            [2, 1, 0, 0],
            [2, 2, 0, 0],
            [11, 0, 0, 0],
            [8, 3, 0, 0],
            [8, 4, 1, 0],
            [2, 0, 0, 0],
            [11, 0, 0, 0],
            [8, 5, 0, 0],
            [2, 3, 0, 0],
            [2, 4, 0, 0],
            [2, 5, 0, 0],
        ]);
        let scripts = scripts(&data, MAX_STEPS);
        assert_eq!(scripts.len(), 3);
        for stress in [false, true] {
            assert_eq!(run(&scripts, stress), Outcome::Passed);
        }
        let check = &scripts[0].1;
        assert!(check.contains("var refs = [\"o1\", \"s2\""), "{check}");
        let check = &scripts[1].1;
        assert!(check.contains("var refs = [\"o1\", \"s2\""), "{check}");
        assert!(
            check.contains("[\"u\", \"u\", \"u\", \"s2\", \"u\""),
            "{check}"
        );
        let check = &scripts[2].1;
        assert!(check.contains("var refs = [\"u\", \"u\""), "{check}");
        assert!(check.contains("if (log !== \"n7\")"), "{check}");
    }

    #[test]
    fn errors_and_empty_inputs_pass() {
        weak(&[]);
        let data = steps(&[
            [4, 0, 0, 0],
            [7, 0, 0, 0],
            [9, 0, 0, 0],
            [10, 0, 0, 0],
            [0, 0, 0, 0],
            [9, 0, 0, 8],
            [6, 0, 0, 1],
            [5, 0, 0, 0],
        ]);
        assert_eq!(run(&scripts(&data, MAX_STEPS), false), Outcome::Passed);
    }
}
