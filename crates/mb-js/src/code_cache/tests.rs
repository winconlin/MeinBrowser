use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::agent::{Abrupt, Agent, Exception};
use crate::bytecode::Op;
use crate::heap::{Local, Options};
use crate::number;
use crate::realm::RealmId;
use crate::script::ScriptError;
use crate::value::Kind;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Scripts mit allem, was ein Eintrag halten muss: Funktionen in
/// Funktionen, Closures, Klassen mit und ohne Konstruktor, private Namen,
/// Generatoren, async, direktes `eval`, `with`, BigInt, reguläre Ausdrücke,
/// Funktionen in Blöcken und `toString`.
const SCRIPTS: &[&str] = &[
    "1 + 2",
    "var a = 1; let b = 2; const c = 3; function f(x, y = 1, ...z) { return x + y + z.length; } \
     [a + b + c, f(1), f.length].join()",
    "function outer() { let n = 0; return function inner() { return ++n; }; } \
     var g = outer(); g(); g(); g()",
    "class A { #x = 1; static s = 2; get x() { return this.#x; } static { A.t = 3; } } \
     class B extends A { constructor() { super(); this.y = 4; } } \
     class C extends B {} \
     var c = new C(); [c.x, c.y, A.s, A.t, C.name, String(C)].join()",
    "function* gen() { yield 1; yield* [2, 3]; return 4; } [...gen()].join()",
    "async function f() { await null; return 1; } \
     async function* g() { yield await f(); } \
     var log = []; g().next().then(r => log.push(r.value)); typeof f",
    "var x = 1; function f() { var y = 2; return eval('x + y'); } f()",
    "var o = { p: 5 }; with (o) { p + 1 }",
    "(123n ** 5n).toString() + -0n + 0x1fffffffffffffffffn",
    "/a(b)?c/gu.exec('xabcx')[0] + /x/.flags",
    "{ function inBlock() { return 7; } } inBlock()",
    "(function named(a, b) { return a; }).toString()",
    "var f = (a, b) => a * b; f(6, 7) + f.toString()",
    "label: for (var i = 0; i < 3; i++) { try { if (i == 1) continue label; throw i; } \
     catch (e) { } finally { } } i",
    "Object.keys(this).length >= 0 && [1, 2, 3].map(function (v) { return v * this.k; }, { k: 2 }).join()",
    "\"use strict\"; function tail(n) { return n == 0 ? 'fertig' : tail(n - 1); } tail(10000)",
    "var { a, b: [c, ...d] = [] } = { a: 1, b: [2, 3, 4] }; a + c + d.length",
    "typeof import('x')",
];

fn agent() -> (Agent, RealmId) {
    let mut agent = Agent::with_options(Options {
        verify: true,
        ..Options::default()
    })
    .expect("Speicher");
    let realm = agent.create_realm().expect("Speicher");
    (agent, realm)
}

fn describe(agent: &Agent, value: Local<'_>) -> String {
    let heap = agent.heap();
    match heap.get(value).kind() {
        Kind::Undefined => "undefined".to_owned(),
        Kind::Null => "null".to_owned(),
        Kind::Boolean(boolean) => boolean.to_string(),
        Kind::Number(number) => number::to_string(number),
        Kind::String(string) => {
            let text = String::from_utf16(&heap.string_units(string)).expect("UTF-16");
            format!("{text:?}")
        }
        Kind::Object(_) => "Objekt".to_owned(),
        kind => format!("{kind:?}"),
    }
}

fn run(agent: &mut Agent, realm: RealmId, source: &str) -> String {
    let result =
        agent.scope(
            |agent, scope| match agent.evaluate_script(scope, realm, &units(source)) {
                Ok(value) => describe(agent, value),
                Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
                    Some(Exception::Error(kind)) => format!("{kind:?}"),
                    Some(Exception::Value(value)) => format!("throw {}", describe(agent, value)),
                    None => "throw ohne Ausnahme".to_owned(),
                },
                Err(error) => format!("{error:?}"),
            },
        );
    assert_eq!(agent.heap_mut().push_registers(0), Some(0), "{source}");
    agent.heap().verify().expect("gültiger Heap");
    result
}

/// Ein Speicher, der zählt, wie oft er gefragt wurde und etwas hatte.
#[derive(Default)]
struct Counting {
    inner: MemoryStore,
    loads: AtomicUsize,
    hits: AtomicUsize,
    stores: AtomicUsize,
}

impl CodeStore for Counting {
    fn load(&self, key: &Digest) -> Option<Vec<u8>> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        let bytes = self.inner.load(key);
        if bytes.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        }
        bytes
    }

    fn store(&self, key: &Digest, bytes: &[u8]) {
        self.stores.fetch_add(1, Ordering::Relaxed);
        self.inner.store(key, bytes);
    }
}

#[test]
fn entries_load_back_unchanged() {
    for source in SCRIPTS {
        let source = units(source);
        let prepared = prepare(&source).expect("speicherbar");
        let bytes = encode(&prepared);
        let loaded = decode(&bytes, &source).expect("Eintrag gültig");
        assert_eq!(loaded, prepared);
        assert_eq!(encode(&loaded), bytes, "das Format ist eindeutig");
    }
}

#[test]
fn scripts_run_the_same_from_the_store() {
    for source in SCRIPTS {
        let (mut plain, realm) = agent();
        let expected = run(&mut plain, realm, source);
        let store = Arc::new(Counting::default());
        for round in 0..2 {
            let (mut cached, realm) = agent();
            cached.set_code_store(Some(Arc::<Counting>::clone(&store)));
            assert_eq!(
                run(&mut cached, realm, source),
                expected,
                "{source}, Runde {round}"
            );
        }
        assert_eq!(store.loads.load(Ordering::Relaxed), 2, "{source}");
        assert_eq!(store.stores.load(Ordering::Relaxed), 1, "{source}");
        assert_eq!(store.hits.load(Ordering::Relaxed), 1, "{source}");
    }
}

#[test]
fn what_cannot_be_stored_runs_as_before() {
    let store = Arc::new(Counting::default());
    let (mut agent, realm) = agent();
    agent.set_code_store(Some(Arc::<Counting>::clone(&store)));
    assert!(run(&mut agent, realm, "1 +").starts_with("Syntax("));
    assert_eq!(store.stores.load(Ordering::Relaxed), 0);
    assert_eq!(prepare(&units("1 +")), Err(Uncacheable::Syntax));
}

#[test]
fn a_function_the_compiler_cannot_fails_at_its_first_call() {
    let source = units("function f() { return 1; } var ok = 2; ok");
    let mut prepared = prepare(&source).expect("speicherbar");
    for (_, function) in &mut prepared.functions {
        function.code = Err(CompileError::TooLarge);
    }
    let bytes = encode(&prepared);
    assert_eq!(decode(&bytes, &source), Ok(prepared));
    let store = Arc::new(MemoryStore::default());
    store.store(&key(&source), &bytes);
    let (mut agent, realm) = agent();
    agent.set_code_store(Some(Arc::<MemoryStore>::clone(&store)));
    assert_eq!(
        run(
            &mut agent,
            realm,
            "function f() { return 1; } var ok = 2; ok"
        ),
        "2"
    );
    assert_eq!(run(&mut agent, realm, "f()"), "RangeError");
}

#[test]
fn the_header_must_match() {
    let source = units("var x = 1;");
    let bytes = encode(&prepare(&source).expect("speicherbar"));
    let reason =
        |bytes: &[u8], source: &[u16]| decode(bytes, source).map(|_| ()).map_err(|e| e.reason());
    assert_eq!(
        reason(&bytes, &units("var x = 2;")),
        Err("Schlüssel passt nicht zum Quelltext")
    );
    let mut changed = bytes.clone();
    changed[0] ^= 1;
    assert_eq!(reason(&changed, &source), Err("keine Kennung"));
    let mut changed = bytes.clone();
    changed[8] ^= 1;
    assert_eq!(reason(&changed, &source), Err("andere Fassung des Formats"));
    let mut changed = bytes.clone();
    changed.push(0);
    assert_eq!(reason(&changed, &source), Err("Bytes hinter dem Eintrag"));
    for length in 0..bytes.len() {
        assert!(
            decode(&bytes[..length], &source).is_err(),
            "gekürzt auf {length}"
        );
    }
}

#[test]
fn every_changed_byte_is_rejected_or_harmless() {
    // Jedes Byte jedes Eintrags auf andere Werte gesetzt: Laden darf
    // scheitern, und was es annimmt, muss laufen, ohne den Heap oder den
    // Registerstapel zu beschädigen.
    for source in SCRIPTS.iter().take(3) {
        let units = units(source);
        let bytes = encode(&prepare(&units).expect("speicherbar"));
        let header = MAGIC_LENGTH + 4 + 32;
        for index in header..bytes.len() {
            for value in [0, 0xFF, bytes[index] ^ 0x01] {
                if value == bytes[index] {
                    continue;
                }
                let mut changed = bytes.clone();
                changed[index] = value;
                if let Ok(loaded) = decode(&changed, &units) {
                    let store = Arc::new(MemoryStore::default());
                    store.store(&key(&units), &encode(&loaded));
                    let (mut agent, realm) = agent();
                    agent.set_code_store(Some(Arc::<MemoryStore>::clone(&store)));
                    let budget = std::rc::Rc::new(crate::agent::Budget::new(10_000));
                    agent.scope(|agent, scope| {
                        let result = agent.evaluate_script_within(scope, realm, &units, budget);
                        if result.is_err() {
                            agent.take_exception(scope);
                        }
                    });
                    assert_eq!(agent.heap_mut().push_registers(0), Some(0));
                    agent.heap().verify().expect("gültiger Heap");
                }
            }
        }
    }
}

/// Die Länge der Kennung am Anfang.
const MAGIC_LENGTH: usize = 8;

#[test]
fn the_verifier_rejects_what_the_compiler_never_emits() {
    let source =
        units("var g = function (a) { return a + 1; }; function f(a) { return g(a); } f(1)");
    let prepared = prepare(&source).expect("speicherbar");
    let rejected = |change: &dyn Fn(&mut CompiledScript)| {
        let mut changed = prepared.clone();
        change(&mut changed);
        decode(&encode(&changed), &source)
            .map(|_| ())
            .map_err(|error| error.reason())
    };
    assert_eq!(
        rejected(&|script| script.script.code.registers = 1),
        Err("zu wenige Register")
    );
    assert_eq!(
        rejected(&|script| script.script.code.bytes.push(Op::Return.byte())),
        Ok(()),
        "ein zweites Return am Ende schadet nicht"
    );
    assert_eq!(
        rejected(&|script| {
            script.script.code.bytes.pop();
        }),
        Err("Code endet nicht mit Return")
    );
    assert_eq!(
        rejected(&|script| script.script.code.bytes.insert(0, 0xFF)),
        Err("Befehl nicht lesbar")
    );
    assert_eq!(
        rejected(&|script| script.functions.clear()),
        Err("Konstante ohne Gegenstück im Eintrag")
    );
    assert_eq!(
        rejected(&|script| script
            .script
            .functions
            .push((units("h"), FunctionId::from_raw(99)))),
        Err("Funktionsdeklaration fehlt")
    );
    assert_eq!(
        rejected(&|script| script.script.code.positions.push((u32::MAX, 0))),
        Err("Lage im Quelltext außerhalb")
    );
    assert_eq!(
        rejected(&|script| {
            if let Some((_, function)) = script.functions.first_mut() {
                function.text = (0, u32::MAX);
            }
        }),
        Err("Quelltext einer Funktion außerhalb")
    );
    assert_eq!(
        rejected(&|script| {
            if let Some((_, function)) = script.functions.first_mut() {
                function.shape.generator = true;
            }
        }),
        Err("Code passt nicht zur Art der Funktion")
    );
    assert_eq!(
        rejected(&|script| {
            let code = &mut script.script.code;
            let count = code.constants.len();
            // Ein Name statt der Funktion für CreateClosure.
            code.constants.push(Constant::Name(units("f")));
            for constant in &mut code.constants {
                if matches!(constant, Constant::Function(_)) {
                    *constant = Constant::Name(units("g"));
                }
            }
            assert!(count > 0);
        }),
        Err("Konstante falscher Art")
    );
}

#[test]
fn functions_named_in_constants_must_be_in_the_entry() {
    let source = units("var f = function () { return 1; }; f()");
    let mut prepared = prepare(&source).expect("speicherbar");
    prepared.functions.clear();
    assert_eq!(
        decode(&encode(&prepared), &source)
            .map(|_| ())
            .map_err(|error| error.reason()),
        Err("Konstante ohne Gegenstück im Eintrag")
    );
}

#[test]
fn the_memory_store_forgets_the_least_recently_used() {
    let store = MemoryStore::new(3 * (100 + 64));
    let keys: Vec<Digest> = (0..4_u16).map(|index| key(&[index])).collect();
    for key in &keys[..3] {
        store.store(key, &[0; 100]);
    }
    assert_eq!(store.len(), 3);
    // Der erste wird gebraucht, also fällt der zweite heraus.
    assert!(store.load(&keys[0]).is_some());
    store.store(&keys[3], &[1; 100]);
    assert_eq!(store.len(), 3);
    assert!(store.load(&keys[1]).is_none());
    assert!(store.load(&keys[0]).is_some());
    assert_eq!(store.load(&keys[3]), Some(vec![1; 100]));
    // Ersetzen zählt nicht doppelt, und zu Großes kommt nicht hinein.
    store.store(&keys[3], &[2; 100]);
    assert_eq!(store.bytes(), 3 * (100 + 64));
    store.store(&keys[2], &[0; 1000]);
    assert!(store.load(&keys[2]).is_none());
    assert_eq!(store.len(), 2);
}

#[test]
fn the_key_covers_format_and_every_unit() {
    assert_ne!(key(&units("a")), key(&units("b")));
    assert_ne!(key(&units("ab")), key(&units("ba")));
    assert_ne!(key(&[0x0100]), key(&[0x0001]));
    let long: Vec<u16> = (0..20_000_u32)
        .map(|index| (index % 65_536) as u16)
        .collect();
    let mut changed = long.clone();
    changed[12_345] ^= 1;
    assert_ne!(key(&long), key(&changed));
}

/// Die längste Kette in den Konstanten des Eintrags.
fn deepest_chain(script: &CompiledScript) -> usize {
    let functions = script
        .functions
        .iter()
        .filter_map(|(_, function)| function.code.as_deref().ok());
    std::iter::once(&script.script.code)
        .chain(functions)
        .flat_map(Code::constants)
        .filter_map(|constant| match constant {
            Constant::Function(template) => Some(template.outer.depth()),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn the_deepest_nesting_loads_back() {
    // Je Funktion zwei Ebenen des Parsers, die tiefsten Formen, die es gibt.
    for (open, middle, close) in [("a => ", "1", ""), ("class C { static { ", "1;", " } }")] {
        let deepest = (1..=crate::parser::MAX_DEPTH as usize)
            .rev()
            .map(|depth| {
                units(&format!(
                    "{}{middle}{}",
                    open.repeat(depth),
                    close.repeat(depth)
                ))
            })
            .find(|source| parse_script(source).is_ok())
            .expect("eine Tiefe passt");
        let prepared = prepare(&deepest).expect("speicherbar");
        let depth = deepest_chain(&prepared);
        assert!((100..=MAX_LEVELS).contains(&depth), "{depth} Stufen");
        let loaded = decode(&encode(&prepared), &deepest).expect("lädt");
        assert_eq!(loaded, prepared);
    }
}

#[test]
fn a_deep_chain_is_rejected_before_it_is_built() {
    fn leb(out: &mut Vec<u8>, mut value: usize) {
        while value >= 0x80 {
            out.push(u8::try_from(value & 0x7F).expect("sieben Bits") | 0x80);
            value >>= 7;
        }
        out.push(u8::try_from(value).expect("sieben Bits"));
    }
    let source = units("1");
    // Ein Kopf, dann `levels` leere Stufen, jede in der davor.
    let entry = |levels: usize| {
        let mut bytes = encode(&prepare(&source).expect("speicherbar"));
        bytes.truncate(8 + 4 + 32);
        leb(&mut bytes, levels);
        for level in 0..levels {
            bytes.extend_from_slice(&[0, 0]);
            leb(&mut bytes, level);
        }
        bytes
    };
    let reason = |bytes: &[u8]| decode(bytes, &source).map(|_| ()).map_err(|e| e.reason());
    assert_eq!(reason(&entry(MAX_LEVELS)), Err("Eintrag zu kurz"));
    assert_eq!(
        reason(&entry(MAX_LEVELS + 1)),
        Err("Stufen zu tief verschachtelt")
    );
    assert_eq!(reason(&entry(100_000)), Err("Stufen zu tief verschachtelt"));
}
