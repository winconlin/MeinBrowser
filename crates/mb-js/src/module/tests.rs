//! Tests für Module (Schritt 9c-1): Laden über einen Host, Link, Evaluate,
//! Import-Bindungen, Namensräume, `import.meta` und JSON-Module.

use std::cell::RefCell;
use std::collections::BTreeMap;

use super::*;
use crate::PromiseState;
use crate::agent::Exception;
use crate::heap::Options;
use crate::host::Host;
use crate::key::PropertyKey;
use crate::number;
use crate::operations::create_data_property_or_throw;

/// Ein Host mit festen Quelltexten nach Bezeichner; `.json` mit
/// `type: "json"` wird ein JSON-Modul. Jeder Bezeichner gibt einmal ein
/// Modul.
struct Sources {
    sources: BTreeMap<String, String>,
    loaded: RefCell<BTreeMap<String, ModuleId>>,
    loads: RefCell<Vec<String>>,
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl Host for Sources {
    fn load_imported_module(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        referrer: Referrer,
        request: &ModuleRequest,
        payload: LoadPayload,
    ) -> Completion<()> {
        let name = String::from_utf16_lossy(&request.specifier);
        self.loads.borrow_mut().push(name.clone());
        let Referrer::Module(referrer_module) = referrer;
        let realm = agent.module_realm(referrer_module).expect("Realm");
        let known = self.loaded.borrow().get(&name).copied();
        let result = match (known, self.sources.get(&name)) {
            (Some(module), _) => Ok(module),
            (None, None) => Err(agent.throw_error(ErrorKind::TypeError)),
            (None, Some(source)) => {
                let json = request.attribute("type") == Some(&units("json")[..]);
                let parsed = if json {
                    agent.parse_json_module(scope, realm, &units(source))
                } else {
                    match agent.parse_module(scope, realm, &units(source)) {
                        Ok(module) => Ok(module),
                        Err(ScriptError::Syntax(_)) => {
                            Err(agent.throw_error(ErrorKind::SyntaxError))
                        }
                        Err(ScriptError::Abrupt(abrupt)) => Err(abrupt),
                        Err(ScriptError::Unsupported(_)) => {
                            Err(agent.throw_error(ErrorKind::RangeError))
                        }
                    }
                };
                if let Ok(module) = parsed {
                    self.loaded.borrow_mut().insert(name, module);
                }
                parsed
            }
        };
        agent.finish_loading_imported_module(scope, referrer, request, payload, result)
    }

    fn supports_import_attribute(&self, key: &[u16]) -> bool {
        key == units("type")
    }

    fn import_meta_properties<'s>(
        &self,
        agent: &mut Agent,
        scope: &'s Scope,
        module: ModuleId,
    ) -> Completion<Vec<(PropertyKey<'s>, Local<'s>)>> {
        let key = agent.key_from_units(scope, &units("id"))?;
        let value = agent.heap().local(
            scope,
            Value::int32(i32::try_from(module.index()).unwrap_or(-1)),
        );
        Ok(vec![(key, value)])
    }
}

fn describe(agent: &mut Agent, scope: &Scope, realm: RealmId, value: Local<'_>) -> String {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::Undefined => "undefined".to_owned(),
        Kind::Boolean(boolean) => boolean.to_string(),
        Kind::Number(number) => number::to_string(number),
        Kind::String(string) => String::from_utf16(&heap.string_units(string)).expect("UTF-16"),
        Kind::Object(_) => {
            // Der Name eines Fehlers.
            let global = agent.global_object(scope, realm);
            let key = agent
                .key_from_units(scope, &units("reason"))
                .expect("Speicher");
            create_data_property_or_throw(agent, scope, global, key, value).expect("Eigenschaft");
            let source = units("reason instanceof Error ? reason.name : String(reason)");
            match agent.evaluate_script(scope, realm, &source) {
                Ok(name) => describe(agent, scope, realm, name),
                Err(error) => format!("{error:?}"),
            }
        }
        kind => format!("{kind:?}"),
    }
}

fn exception(agent: &mut Agent, scope: &Scope, realm: RealmId) -> String {
    match agent.take_exception(scope) {
        Some(Exception::Error(kind)) => format!("{kind:?}"),
        Some(Exception::Value(value)) => describe(agent, scope, realm, value),
        None => "keine Ausnahme".to_owned(),
    }
}

/// Lädt, linkt und wertet das Modul `main` aus und gibt `out` zurück oder
/// den Schritt, der scheiterte, mit dem Fehler.
fn run(sources: &[(&str, &str)]) -> (String, Vec<String>) {
    let host = Rc::new(Sources {
        sources: sources
            .iter()
            .map(|&(name, source)| (name.to_owned(), source.to_owned()))
            .collect(),
        loaded: RefCell::new(BTreeMap::new()),
        loads: RefCell::new(Vec::new()),
    });
    let mut agent = Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    agent.set_host(host.clone());
    let realm = agent.create_realm().expect("Speicher");
    let outcome = agent.scope(|agent, scope| {
        let main = host.sources.get("main").expect("main");
        let module = match agent.parse_module(scope, realm, &units(main)) {
            Ok(module) => module,
            Err(error) => return format!("Parse: {error:?}"),
        };
        host.loaded.borrow_mut().insert("main".to_owned(), module);
        let promise = match agent.load_requested_modules(scope, module) {
            Ok(promise) => promise,
            Err(_) => return format!("Laden: {}", exception(agent, scope, realm)),
        };
        match agent.promise_state(promise) {
            Some(PromiseState::Fulfilled) => {}
            Some(PromiseState::Rejected) => {
                let reason = agent.promise_result(scope, promise).expect("Grund");
                return format!("Laden: {}", describe(agent, scope, realm, reason));
            }
            state => return format!("Laden: {state:?}"),
        }
        if agent.link_module(module).is_err() {
            return format!("Link: {}", exception(agent, scope, realm));
        }
        let promise = match agent.evaluate_module(scope, module) {
            Ok(promise) => promise,
            Err(abrupt) => {
                return format!("Evaluate: {abrupt:?} {}", exception(agent, scope, realm));
            }
        };
        agent.run_jobs().expect("Jobs");
        match agent.promise_state(promise) {
            Some(PromiseState::Fulfilled) => {}
            Some(PromiseState::Rejected) => {
                let reason = agent.promise_result(scope, promise).expect("Grund");
                return format!("Evaluate: {}", describe(agent, scope, realm, reason));
            }
            state => return format!("Evaluate: {state:?}"),
        }
        match agent.evaluate_script(scope, realm, &units("globalThis.out")) {
            Ok(value) => describe(agent, scope, realm, value),
            Err(_) => format!("out: {}", exception(agent, scope, realm)),
        }
    });
    assert_eq!(agent.heap_mut().push_registers(0), Some(0));
    let loads = host.loads.borrow().clone();
    (outcome, loads)
}

fn check(cases: &[(&[(&str, &str)], &str)]) {
    let mut failures = Vec::new();
    for &(sources, expected) in cases {
        let (actual, _) = run(sources);
        if actual != expected {
            failures.push(format!(
                "{sources:?}\n  erwartet {expected}, bekommen {actual}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn imports_see_live_bindings() {
    check(&[
        (&[("main", "globalThis.out = 'ok';")], "ok"),
        (
            &[
                (
                    "main",
                    "import { x, inc } from 'a'; inc(); globalThis.out = x;",
                ),
                ("a", "export let x = 1; export function inc() { x++; }"),
            ],
            "2",
        ),
        (
            &[
                (
                    "main",
                    "import d, { y as z } from 'a'; globalThis.out = d + z;",
                ),
                ("a", "export default 40; var y = 2; export { y };"),
            ],
            "42",
        ),
        (
            &[
                ("main", "import f from 'a'; globalThis.out = f.name + f();"),
                ("a", "export default function () { return 1; }"),
            ],
            "default1",
        ),
        (
            &[
                ("main", "import C from 'a'; globalThis.out = C.name;"),
                ("a", "export default class {}"),
            ],
            "default",
        ),
        (
            &[
                ("main", "import g from 'a'; globalThis.out = g.name;"),
                ("a", "export default (function () {});"),
            ],
            "default",
        ),
        (
            &[
                ("main", "import { x } from 'b'; globalThis.out = x;"),
                ("b", "export { x } from 'a';"),
                ("a", "export const x = 'weiter';"),
            ],
            "weiter",
        ),
        (
            &[
                ("main", "import { x } from 'b'; globalThis.out = x;"),
                ("b", "import { y } from 'a'; export { y as x };"),
                ("a", "export const y = 'umbenannt';"),
            ],
            "umbenannt",
        ),
        (
            &[
                ("main", "import { a, b } from 'c'; globalThis.out = a + b;"),
                ("c", "export * from 'a'; export * from 'b';"),
                ("a", "export const a = 'A';"),
                ("b", "export const b = 'B';"),
            ],
            "AB",
        ),
    ]);
}

#[test]
fn import_bindings_are_immutable_and_checked() {
    check(&[
        (
            &[
                (
                    "main",
                    "import { x } from 'a'; try { x = 2; } catch (e) { globalThis.out = e.name; }",
                ),
                ("a", "export let x = 1;"),
            ],
            "TypeError",
        ),
        (
            &[
                (
                    "main",
                    "import { x } from 'a'; export function f() { return x; }",
                ),
                (
                    "a",
                    "import { f } from 'main'; try { f(); } catch (e) { globalThis.out = e.name; } export let x = 1;",
                ),
            ],
            "ReferenceError",
        ),
        (
            &[
                ("main", "import { g } from 'a'; globalThis.out = g();"),
                (
                    "a",
                    "import { f } from 'main'; export function g() { return f(); }",
                ),
            ],
            "Link: SyntaxError",
        ),
        (
            &[
                (
                    "main",
                    "import { f } from 'a'; export function h() { return 'früh'; }",
                ),
                (
                    "a",
                    "import { h } from 'main'; globalThis.out = h(); export function f() {}",
                ),
            ],
            "früh",
        ),
        (
            &[
                ("main", "import { x } from 'a';"),
                ("a", "export const y = 1;"),
            ],
            "Link: SyntaxError",
        ),
        (
            &[
                ("main", "import { x } from 'c';"),
                ("c", "export * from 'a'; export * from 'b';"),
                ("a", "export const x = 1;"),
                ("b", "export const x = 2;"),
            ],
            "Link: SyntaxError",
        ),
        (
            &[("main", "import { x } from 'fehlt';")],
            "Laden: TypeError",
        ),
        (
            &[("main", "import 'a';"), ("a", "let let = 1;")],
            "Laden: SyntaxError",
        ),
        (
            &[("main", "import x from 'a' with { art: 'x' };"), ("a", "")],
            "Laden: SyntaxError",
        ),
    ]);
}

#[test]
fn evaluation_runs_dependencies_first_and_remembers_errors() {
    check(&[
        (
            &[
                ("main", "import 'a'; import 'b'; globalThis.out += 'm';"),
                ("a", "import 'c'; globalThis.out += 'a';"),
                ("b", "import 'c'; globalThis.out += 'b';"),
                ("c", "globalThis.out = 'c';"),
            ],
            "cabm",
        ),
        (
            &[
                ("main", "import 'a'; globalThis.out += 'm';"),
                ("a", "import 'main'; globalThis.out = 'a';"),
            ],
            "am",
        ),
        (
            &[
                ("main", "import 'a'; globalThis.out = 'nie';"),
                ("a", "throw new RangeError();"),
            ],
            "Evaluate: RangeError",
        ),
        (
            &[("main", "globalThis.out = this === undefined && typeof x;")],
            "undefined",
        ),
    ]);
}

#[test]
fn namespaces_are_sorted_and_frozen() {
    check(&[
        (
            &[
                (
                    "main",
                    "import * as ns from 'a'; globalThis.out = Object.keys(ns).join() + ns[Symbol.toStringTag] + Object.getPrototypeOf(ns) + Object.isExtensible(ns);",
                ),
                (
                    "a",
                    "export var b = 1, a = 2; export { a as c }; export default 3;",
                ),
            ],
            "a,b,c,defaultModulenullfalse",
        ),
        (
            &[
                (
                    "main",
                    "import * as ns from 'a'; 'use strict'; let r = []; try { ns.a = 1; } catch (e) { r.push(e.name); } r.push(delete ns.zz, Reflect.deleteProperty(ns, 'a'), Reflect.set(ns, 'a', 1), 'a' in ns, Reflect.defineProperty(ns, 'a', { value: 2 })); globalThis.out = r.join();",
                ),
                ("a", "export let a = 2;"),
            ],
            "TypeError,true,false,false,true,true",
        ),
        (
            &[
                (
                    "main",
                    "import * as ns from 'a'; import { ns as same } from 'b'; globalThis.out = (ns === same) + ',' + ns.x;",
                ),
                ("a", "export const x = 'x';"),
                ("b", "export * as ns from 'a';"),
            ],
            "true,x",
        ),
        (
            &[
                (
                    "main",
                    "import * as ns from 'a'; let d = Object.getOwnPropertyDescriptor(ns, 'x'); globalThis.out = [d.value, d.writable, d.enumerable, d.configurable].join();",
                ),
                ("a", "export let x = 5;"),
            ],
            "5,true,true,false",
        ),
        (
            &[
                (
                    "main",
                    "import * as ns from 'a'; let o = Object.create(ns); globalThis.out = o.x + ',' + ('x' in o);",
                ),
                ("a", "export let x = 'geerbt';"),
            ],
            "geerbt,true",
        ),
        (
            &[
                ("main", "import * as ns from 'a'; export let x = 1;"),
                (
                    "a",
                    "import * as main from 'main'; try { main.x; } catch (e) { globalThis.out = e.name; }",
                ),
            ],
            "ReferenceError",
        ),
    ]);
}

#[test]
fn import_meta_is_one_object_per_module() {
    check(&[
        (
            &[
                (
                    "main",
                    "import { m } from 'a'; globalThis.out = (import.meta !== m) + ',' + (import.meta === import.meta) + ',' + Object.getPrototypeOf(m) + ',' + typeof import.meta.id;",
                ),
                ("a", "export const m = import.meta;"),
            ],
            "true,true,null,number",
        ),
        (
            &[(
                "main",
                "function f() { return import.meta; } globalThis.out = f() === import.meta;",
            )],
            "true",
        ),
    ]);
}

#[test]
fn json_modules_export_their_value() {
    check(&[
        (
            &[
                (
                    "main",
                    "import data from 'd.json' with { type: 'json' }; globalThis.out = data.a + data.b.length;",
                ),
                ("d.json", "{ \"a\": 40, \"b\": [1, 2] }"),
            ],
            "42",
        ),
        (
            &[
                (
                    "main",
                    "import * as ns from 'd.json' with { type: 'json' }; globalThis.out = Object.keys(ns).join() + ns.default;",
                ),
                ("d.json", "7"),
            ],
            "default7",
        ),
        (
            &[
                ("main", "import data from 'd.json' with { type: 'json' };"),
                ("d.json", "{ a: 1 }"),
            ],
            "Laden: SyntaxError",
        ),
    ]);
}

#[test]
fn the_host_loads_each_request_once_in_source_order() {
    let (outcome, loads) = run(&[
        (
            "main",
            "import 'a'; import 'b'; import 'a'; globalThis.out = 'ok';",
        ),
        ("a", "import 'c';"),
        ("b", "import 'c';"),
        ("c", ""),
    ]);
    assert_eq!(outcome, "ok");
    // Jedes Modul fordert jeden Bezeichner einmal an, in der Tiefe zuerst.
    assert_eq!(loads, ["a", "c", "b", "c"]);
}

#[test]
fn long_chains_need_no_rust_stack() {
    let count = 3000;
    let mut sources: Vec<(String, String)> = (0..count)
        .map(|index| {
            let next = index + 1;
            let source = if next < count {
                format!("import {{ v }} from 'm{next}'; export {{ v }}; export * from 'm{next}';")
            } else {
                "export const v = 'Ende'; export const w = 1;".to_owned()
            };
            (format!("m{index}"), source)
        })
        .collect();
    sources.push((
        "main".to_owned(),
        "import { v, w } from 'm0'; globalThis.out = v + w;".to_owned(),
    ));
    let borrowed: Vec<(&str, &str)> = sources
        .iter()
        .map(|(name, source)| (name.as_str(), source.as_str()))
        .collect();
    let (outcome, _) = run(&borrowed);
    assert_eq!(outcome, "Ende1");
}

#[test]
fn an_evaluation_cut_short_by_the_host_reports_it_again() {
    use crate::agent::{Budget, Exhausted};
    let host = Rc::new(Sources {
        sources: [("a", "while (true) {}")]
            .into_iter()
            .map(|(name, source)| (name.to_owned(), source.to_owned()))
            .collect(),
        loaded: RefCell::new(BTreeMap::new()),
        loads: RefCell::new(Vec::new()),
    });
    let mut agent = Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    agent.set_host(host.clone());
    let realm = agent.create_realm().expect("Speicher");
    agent.scope(|agent, scope| {
        let main = agent
            .parse_module(scope, realm, &units("import 'a'; globalThis.out = 1;"))
            .expect("Modul");
        let other = agent
            .parse_module(scope, realm, &units("import 'a';"))
            .expect("Modul");
        for module in [main, other] {
            agent.load_requested_modules(scope, module).expect("Laden");
            agent.link_module(module).expect("Link");
        }
        let steps = Err(Abrupt::Exhausted(Exhausted::Steps));
        for module in [main, main, other] {
            let result = agent.with_execution_budget(Rc::new(Budget::new(1_000)), |agent| {
                agent.evaluate_module(scope, module).map(|_| ())
            });
            // Nie ein Promise, das offen bleibt.
            assert_eq!(result, steps);
            assert_eq!(agent.module_status(module), Some(ModuleStatus::Evaluated));
        }
        let a = host.loaded.borrow().get("a").copied().expect("a");
        assert_eq!(agent.module_status(a), Some(ModuleStatus::Evaluated));
        assert!(agent.take_exception(scope).is_none());
    });
    assert_eq!(agent.heap_mut().push_registers(0), Some(0));
    agent.heap_mut().collect();
    agent.heap().verify().expect("Heap");
}
