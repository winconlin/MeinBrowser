//! Bytecode aus dem Zwischenspeicher von `mb-js` (`mb_js::code_cache`).
//!
//! Bit 0 des ersten Bytes wählt die Art, der Rest ist ein Script in UTF-8:
//!
//! - Vergleich (Bit 0 frei): Lässt der Übersetzer das Script zu, muss es
//!   sich speichern lassen, und sein Eintrag kommt unverändert zurück. Dann
//!   läuft es in zwei frischen Agents unter derselben Quote, ohne Speicher
//!   und aus einem Speicher mit dem Eintrag, und beide müssen gleich enden.
//!   Bei einem Fehlschlag im Speicher führt der Agent das übersetzte Script
//!   aus, das dem geladenen gleicht; das deckt der Lauf mit ab.
//! - Verändert (Bit 0 gesetzt): Bis zum ersten Nullbyte steht das Script,
//!   dahinter Änderungen am Eintrag, je drei Bytes: die Stelle hinter dem
//!   Kopf mit dem niedrigsten Byte zuerst, dann der neue Wert. Den Kopf
//!   lassen sie stehen, damit der Schlüssel passt. Laden darf scheitern;
//!   was es annimmt, muss unter der Quote laufen, ohne Panik und mit
//!   gültigem Heap, und lässt sich wieder speichern.
//!
//! Damit die Läufe vergleichbar sind, gibt der Host eine feste Zeit, und
//! `Math.random` liefert vor dem Script immer dieselbe Zahl.

use std::rc::Rc;
use std::sync::Arc;

use mb_js::agent::{Abrupt, Agent, Budget, ErrorKind, Exception, Exhausted};
use mb_js::code_cache::{self, CodeStore, HEADER, MemoryStore, Uncacheable};
use mb_js::heap::{Local, Options};
use mb_js::host::Host;
use mb_js::realm::RealmId;
use mb_js::script::ScriptError;
use mb_js::value::Kind;

const SCRIPT_STEPS: u64 = 2048;
const HEAP_BYTES: usize = 1 << 20;
/// Höchstens so viele Änderungen je Eingabe.
const MAX_CHANGES: usize = 64;

/// Macht `Math.random` vorhersagbar, mit denselben Attributen wie zuvor.
const PRELUDE: &str = "Object.defineProperty(Math, 'random', { value: function random() \
                       { return 0.5; }, writable: true, enumerable: false, configurable: true });";

/// Ein Host mit fester Zeit.
struct FixedTime;

impl Host for FixedTime {
    fn current_time(&self) -> f64 {
        1_800_000_000_000.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Undefined,
    Null,
    Boolean(bool),
    Number(u64),
    String(Vec<u16>),
    Object,
    Symbol,
    BigInt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Value(Value),
    Thrown(Value),
    Error(ErrorKind),
    Syntax(usize),
    Unsupported(usize),
    Exhausted(Exhausted),
    OutOfMemory,
}

fn snapshot(agent: &Agent, local: Local<'_>) -> Value {
    let heap = agent.heap();
    match heap.get(local).kind() {
        Kind::Undefined => Value::Undefined,
        Kind::Null => Value::Null,
        Kind::Boolean(value) => Value::Boolean(value),
        Kind::Number(value) if value.is_nan() => Value::Number(f64::NAN.to_bits()),
        Kind::Number(value) => Value::Number(value.to_bits()),
        Kind::String(value) => Value::String(heap.string_units(value)),
        Kind::Object(_) => Value::Object,
        Kind::Symbol(_) => Value::Symbol,
        Kind::BigInt(_) => Value::BigInt,
        kind => panic!("internal value escaped script evaluation: {kind:?}"),
    }
}

fn agent(store: Option<&Arc<MemoryStore>>) -> (Agent, RealmId) {
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress: false,
        verify: true,
    })
    .expect("code cache agent");
    agent.set_host(Rc::new(FixedTime));
    let realm = agent.create_realm().expect("code cache realm");
    let prelude: Vec<u16> = PRELUDE.encode_utf16().collect();
    agent.scope(|agent, scope| {
        agent
            .evaluate_script(scope, realm, &prelude)
            .expect("prelude runs");
    });
    if let Some(store) = store {
        agent.set_code_store(Some(Arc::<MemoryStore>::clone(store) as Arc<dyn CodeStore>));
    }
    (agent, realm)
}

/// Führt `source` unter der Quote aus und prüft danach den Heap.
fn run(source: &[u16], store: Option<&Arc<MemoryStore>>) -> Outcome {
    let (mut agent, realm) = agent(store);
    let budget = Rc::new(Budget::new(SCRIPT_STEPS));
    let outcome = agent.scope(|agent, scope| {
        let result = agent.evaluate_script_within(scope, realm, source, Rc::clone(&budget));
        agent.heap_mut().collect();
        let outcome = match result {
            Ok(value) => Outcome::Value(snapshot(agent, value)),
            Err(ScriptError::Syntax(error)) => Outcome::Syntax(error.position),
            Err(ScriptError::Unsupported(position))
            | Err(ScriptError::Abrupt(Abrupt::Unsupported(position))) => {
                assert!(position <= source.len());
                Outcome::Unsupported(position)
            }
            Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
                Some(Exception::Error(kind)) => Outcome::Error(kind),
                Some(Exception::Value(value)) => Outcome::Thrown(snapshot(agent, value)),
                None => panic!("script threw without an exception"),
            },
            Err(ScriptError::Abrupt(Abrupt::Exhausted(reason))) => Outcome::Exhausted(reason),
            Err(ScriptError::Abrupt(Abrupt::OutOfMemory)) => Outcome::OutOfMemory,
        };
        assert!(agent.take_exception(scope).is_none(), "no stale exception");
        outcome
    });
    agent.heap_mut().collect();
    agent
        .heap()
        .verify()
        .expect("valid heap after cached script");
    outcome
}

pub fn code_cache(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let Some((&control, rest)) = data.split_first() else {
        return;
    };
    if control & 1 == 0 {
        compare(rest);
    } else {
        changed(rest);
    }
}

fn units(bytes: &[u8]) -> Vec<u16> {
    String::from_utf8_lossy(bytes).encode_utf16().collect()
}

fn compare(data: &[u8]) {
    let source = units(data);
    let prepared = match code_cache::prepare(&source) {
        Ok(prepared) => prepared,
        // Dann läuft das Script mit Speicher wie ohne.
        Err(Uncacheable::Syntax | Uncacheable::Compile(_)) => return,
        Err(Uncacheable::Unsupported) => {
            panic!("compiled script cannot be stored: verifier and compiler disagree")
        }
    };
    let bytes = code_cache::encode(&prepared);
    let loaded = code_cache::decode(&bytes, &source).expect("own entry loads");
    assert_eq!(loaded, prepared, "entry loads back unchanged");
    assert_eq!(
        code_cache::encode(&loaded),
        bytes,
        "the format is canonical"
    );
    // Was der Agent bei einem Fehlschlag ausführt, ist `prepared`, also
    // dasselbe wie `loaded`; ein Lauf aus dem Speicher deckt beide ab.
    let plain = run(&source, None);
    let store = Arc::new(MemoryStore::default());
    store.store(&code_cache::key(&source), &bytes);
    assert_eq!(
        run(&source, Some(&store)),
        plain,
        "loading does not change the outcome"
    );
}

fn changed(data: &[u8]) {
    let (script, changes) = match data.iter().position(|&byte| byte == 0) {
        Some(index) => (&data[..index], &data[index + 1..]),
        None => (data, &[][..]),
    };
    let source = units(script);
    let Ok(prepared) = code_cache::prepare(&source) else {
        return;
    };
    let mut bytes = code_cache::encode(&prepared);
    let body = bytes.len() - HEADER;
    for change in changes.chunks_exact(3).take(MAX_CHANGES) {
        let offset = usize::from(u16::from_le_bytes([change[0], change[1]]));
        bytes[HEADER + offset % body] = change[2];
    }
    let Ok(loaded) = code_cache::decode(&bytes, &source) else {
        return;
    };
    let again = code_cache::encode(&loaded);
    assert_eq!(
        code_cache::decode(&again, &source).as_ref(),
        Ok(&loaded),
        "an accepted entry stores again"
    );
    let store = Arc::new(MemoryStore::default());
    store.store(&code_cache::key(&source), &bytes);
    run(&source, Some(&store));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(control: u8, source: &str, changes: &[u8]) -> Vec<u8> {
        let mut data = vec![control];
        data.extend_from_slice(source.as_bytes());
        if !changes.is_empty() {
            data.push(0);
            data.extend_from_slice(changes);
        }
        data
    }

    #[test]
    fn stored_and_loaded_scripts_end_alike() {
        for source in [
            "1 + 2",
            "function f(a, b = 2) { return a + b; } f(1)",
            "class A { #x = 1; get x() { return this.#x; } } new A().x",
            "function* g() { yield 1; } [...g()].length",
            "var x = 1; function f() { return eval('x + 1'); } f()",
            "with ({ y: 2 }) y",
            "Math.random() + Date.now()",
            "for (;;) {}",
            "throw new TypeError('x')",
            "1 +",
        ] {
            code_cache(&input(0, source, &[]));
        }
    }

    #[test]
    fn changed_entries_are_rejected_or_harmless() {
        let source = "var g = function (a) { return [a, typeof a]; }; class C extends Array {} \
                      g(new C().length)";
        for offset in 0..600_u16 {
            for value in [0, 1, 0x7F, 0xFF] {
                let [low, high] = offset.to_le_bytes();
                code_cache(&input(1, source, &[low, high, value]));
            }
        }
        code_cache(&input(1, "1", &[]));
        code_cache(&input(1, "1 +", &[0, 0, 0]));
        code_cache(&[]);
        code_cache(&[1]);
    }
}
