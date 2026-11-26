//! GC-lastige Programme für den Garbage Collector mit Generationen. Jedes
//! Programm baut Daten auf, die Sammlungen überstehen müssen, vor allem
//! Verweise aus alten Zellen auf junge (Felder, Eigenschaften, Prototypen,
//! Elemente, private Felder, Einträge von Map, Set und WeakMap, Rahmen von
//! Generatoren, Reaktionen von Promises), prüft sie selbst und setzt die
//! globale Variable `result` auf `"ok"` oder beschreibt den Fehler. Die
//! Größe gibt die Variable `N` vor.
//!
//! Jeder Lauf hat einen frischen Agent im Prüfmodus, der den Heap nach jeder
//! Sammlung kontrolliert. Im normalen Modus muss jedes Programm mindestens
//! eine junge Sammlung auslösen. Die Läufe mit großem `N` und im
//! Belastungsmodus mit mittlerem `N` brauchen ein Vielfaches der Zeit, sind
//! mit `#[ignore]` markiert und laufen nachts im Release-Build
//! (`.github/workflows/belastung.yml`).

use std::rc::Rc;

use crate::agent::{Agent, Budget};
use crate::heap::Options;
use crate::value::Kind;

/// Die Arbeitsschritte eines Laufs, für das Programm und alle Jobs.
const STEPS: u64 = 400_000_000;
/// Das Speicherbudget eines Laufs.
const HEAP_BYTES: usize = 256 << 20;

/// Vor jedem Programm.
const PRELUDE: &str = "var result = 'läuft';
function check(condition, message) {
    if (!condition) {
        throw new Error(message);
    }
}
";

/// Die Programme mit ihren Namen.
const PROGRAMS: &[(&str, &str)] = &[
    (
        "Feld mit jungen Objekten",
        "var keep = [];
        for (var i = 0; i < N; i++) {
            keep.push({ index: i, text: 'Eintrag ' + i, inner: [i, i * 2] });
            if (i % 97 === 0) {
                var j = (i * 31) % keep.length;
                check(keep[j].index === j && keep[j].inner[1] === 2 * j, 'Eintrag ' + j);
            }
        }
        var total = 0;
        var expected = 0;
        for (var i = 0; i < N; i++) {
            total += keep[i].inner[0] + keep[i].text.length;
            expected += i + ('Eintrag ' + i).length;
        }
        check(total === expected, 'Summe');
        result = 'ok';",
    ),
    (
        "verkettete Listen",
        "var head = { value: -1, next: null, prev: null };
        var tail = head;
        for (var i = 0; i < N; i++) {
            var node = { value: i, next: null, prev: tail, label: 'n' + i };
            tail.next = node;
            tail = node;
        }
        var count = 0;
        var sum = 0;
        for (var node = head.next; node !== null; node = node.next) {
            check(node.prev.next === node && node.label === 'n' + node.value, 'Knoten ' + node.value);
            count++;
            sum += node.value;
        }
        check(count === N && sum === N * (N - 1) / 2, 'vorwärts');
        var back = 0;
        for (var node = tail; node !== head; node = node.prev) {
            back++;
        }
        check(back === N, 'rückwärts');
        result = 'ok';",
    ),
    (
        "Map und Set",
        "var keys = [];
        var map = new Map();
        var set = new Set();
        for (var i = 0; i < N; i++) {
            var key = { id: i };
            keys.push(key);
            map.set(key, 'Wert ' + i);
            set.add(key);
            if (i % 3 === 0) {
                map.set('s' + i, [i]);
            }
            if (i % 5 === 4) {
                set.delete(keys[i - 2]);
                map.delete(keys[i - 4]);
            }
        }
        for (var k = 0; k < N; k++) {
            var inMap = !(k % 5 === 0 && k + 4 < N);
            check(map.has(keys[k]) === inMap, 'Map ' + k);
            check(!inMap || map.get(keys[k]) === 'Wert ' + k, 'Wert ' + k);
            var inSet = !(k % 5 === 2 && k + 2 < N);
            check(set.has(keys[k]) === inSet, 'Set ' + k);
            check(k % 3 !== 0 || map.get('s' + k)[0] === k, 'Zeichenkette ' + k);
        }
        var last = -1;
        for (var entry of set) {
            check(entry.id > last, 'Reihenfolge ' + entry.id);
            last = entry.id;
        }
        check(!map.has({ id: 0 }), 'fremder Schlüssel');
        result = 'ok';",
    ),
    (
        "schwache Sammlungen",
        "var weak = new WeakMap();
        var holders = [];
        var refs = [];
        for (var i = 0; i < N; i++) {
            var key = { k: i };
            holders.push(key);
            weak.set(key, { v: i, data: 'x' + i });
            if (i % 10 === 0) {
                refs.push(new WeakRef(key));
            }
            weak.set({ lost: i }, { values: [i, i, i] });
            new WeakRef({ lost: i });
        }
        for (var i = 0; i < N; i++) {
            var value = weak.get(holders[i]);
            check(value !== undefined && value.v === i && value.data === 'x' + i, 'WeakMap ' + i);
        }
        for (var r = 0; r < refs.length; r++) {
            check(refs[r].deref() === holders[r * 10], 'WeakRef ' + r);
        }
        var weakSet = new WeakSet(holders.slice(0, 100));
        check(weakSet.has(holders[0]) && !weakSet.has({}), 'WeakSet');
        var registry = new FinalizationRegistry(function () {});
        for (var i = 0; i < N; i++) {
            registry.register({ target: i }, 'gehalten ' + i);
        }
        result = 'ok';",
    ),
    (
        "Closures und Generatoren",
        "var closures = [];
        for (let i = 0; i < N; i++) {
            let box = { value: i };
            closures.push(function () { return box.value + i; });
        }
        function* producer() {
            var made = [];
            for (var i = 0; i < N; i++) {
                made.push({ i: i });
                yield made[made.length - 1];
            }
            return made;
        }
        var generator = producer();
        var seen = 0;
        var churn = [];
        var step = generator.next();
        while (!step.done) {
            check(step.value.i === seen, 'Generator ' + seen);
            seen++;
            churn.push([seen, 'c' + seen]);
            if (churn.length > 50) {
                churn = [];
            }
            step = generator.next();
        }
        check(seen === N && step.value.length === N && step.value[N - 1].i === N - 1, 'Anzahl');
        var sum = 0;
        for (var i = 0; i < N; i++) {
            sum += closures[i]();
        }
        check(sum === N * (N - 1), 'Closures');
        result = 'ok';",
    ),
    (
        "Promise-Ketten und async",
        "var chain = Promise.resolve({ count: 0, items: [] });
        for (var i = 0; i < N; i++) {
            chain = chain.then(function (state) {
                state.items.push({ n: state.count, text: 't' + state.count });
                state.count++;
                return state;
            });
        }
        async function collect(count) {
            var gathered = [];
            for (var i = 0; i < count; i++) {
                gathered.push(await Promise.resolve({ i: i, label: 'a' + i }));
            }
            return gathered;
        }
        Promise.all([chain, collect(N)]).then(function (both) {
            var state = both[0];
            var gathered = both[1];
            check(state.count === N && state.items.length === N, 'Anzahl');
            for (var i = 0; i < N; i++) {
                check(state.items[i].n === i && state.items[i].text === 't' + i, 'Kette ' + i);
                check(gathered[i].i === i && gathered[i].label === 'a' + i, 'await ' + i);
            }
            result = 'ok';
        }).catch(function (error) {
            result = 'Fehler: ' + error;
        });",
    ),
    (
        "Prototypen und Gestalten",
        "var targets = [];
        var shadows = [];
        var prototypes = [];
        for (var t = 0; t < 64; t++) {
            targets.push({ base: t });
            shadows.push(new Map());
            prototypes.push(-1);
        }
        for (var i = 0; i < N; i++) {
            var t = i % 64;
            var key = 'p' + (i % 300);
            targets[t][key] = { v: i };
            shadows[t].set(key, i);
            if (i % 7 === 0) {
                Object.setPrototypeOf(targets[t], { proto: i, parent: { deep: i } });
                prototypes[t] = i;
            }
            if (i % 11 === 0) {
                var gone = 'p' + ((i + 150) % 300);
                delete targets[t][gone];
                shadows[t].delete(gone);
            }
        }
        for (var t = 0; t < 64; t++) {
            var target = targets[t];
            check(target.base === t, 'Basis ' + t);
            var names = Object.keys(target);
            check(names.length === shadows[t].size + 1 && names[0] === 'base', 'Schlüssel ' + t);
            var position = 1;
            for (var entry of shadows[t]) {
                check(names[position] === entry[0] && target[entry[0]].v === entry[1], 'Eigenschaft ' + t + ' ' + entry[0]);
                position++;
            }
            var prototype = Object.getPrototypeOf(target);
            if (prototypes[t] < 0) {
                check(prototype === Object.prototype, 'Prototyp ' + t);
            } else {
                check(prototype.proto === prototypes[t] && prototype.parent.deep === prototypes[t], 'Prototyp ' + t);
            }
        }
        result = 'ok';",
    ),
    (
        "Puffer und Zeichenketten",
        "var buffers = [];
        var text = '';
        for (var i = 0; i < N; i++) {
            var bytes = new Uint8Array(16 + (i % 32));
            bytes[0] = i & 255;
            bytes[bytes.length - 1] = (i * 7) & 255;
            if (i % 4 === 0) {
                buffers.push(bytes);
            }
            var view = new DataView(new ArrayBuffer(8));
            view.setFloat64(0, i + 0.5);
            check(view.getFloat64(0) === i + 0.5, 'DataView ' + i);
            text += String.fromCharCode(65 + (i % 26));
            if (text.length > 1000) {
                text = text.slice(500);
            }
        }
        for (var b = 0; b < buffers.length; b++) {
            var i = b * 4;
            var bytes = buffers[b];
            check(bytes.length === 16 + (i % 32) && bytes[0] === (i & 255) && bytes[bytes.length - 1] === ((i * 7) & 255), 'Puffer ' + b);
        }
        check(text.charCodeAt(text.length - 1) === 65 + ((N - 1) % 26), 'Text');
        var numbers = new Float64Array(1024);
        for (var i = 0; i < 1024; i++) {
            numbers[i] = i;
        }
        var copy = numbers.slice().reverse();
        check(copy[0] === 1023 && copy[1023] === 0 && numbers[0] === 0, 'Kopie');
        result = 'ok';",
    ),
    (
        "Klassen mit privaten Feldern",
        "class Holder {
            #payload;
            static #count = 0;
            static made;
            static {
                Holder.made = 0;
            }
            constructor(i) {
                this.#payload = { i: i, list: [i] };
                Holder.#count++;
                Holder.made++;
            }
            get i() {
                return this.#payload.i;
            }
            get list() {
                return this.#payload.list;
            }
            replace(i) {
                this.#payload = { i: i, list: [i, i] };
            }
            static count() {
                return Holder.#count;
            }
        }
        var holders = [];
        for (var i = 0; i < N; i++) {
            holders.push(new Holder(i));
            if (i % 3 === 0) {
                holders[i >> 1].replace(i >> 1);
            }
        }
        for (var k = 0; k < N; k++) {
            check(holders[k].i === k && holders[k].list[0] === k, 'Instanz ' + k);
        }
        check(Holder.count() === N && Holder.made === N, 'Zähler');
        result = 'ok';",
    ),
    (
        "JSON, Ausbreitung und Destrukturierung",
        "var records = [];
        for (var i = 0; i < N; i++) {
            var record = { id: i, tags: ['a' + (i % 5), 'b'], nested: { flag: i % 2 === 0 } };
            var copy = { ...record, tags: [...record.tags] };
            var { id, tags: [first], nested: { flag } } = copy;
            check(id === i && first === 'a' + (i % 5) && flag === (i % 2 === 0), 'Destrukturierung ' + i);
            records.push(copy);
            if (i % 100 === 99) {
                var back = JSON.parse(JSON.stringify(records.slice(-100)));
                check(back.length === 100 && back[99].id === i && back[0].nested.flag === ((i - 99) % 2 === 0), 'JSON ' + i);
            }
        }
        check(records.length === N && records[N - 1].tags[1] === 'b', 'Anzahl');
        result = 'ok';",
    ),
];

/// Was ein Lauf ergab.
struct Run {
    /// Der Wert von `result` nach dem Programm und allen Jobs.
    result: String,
    /// Junge und volle Sammlungen.
    young: u64,
    full: u64,
}

/// Führt das Programm `source` mit `N = n` in einem frischen Agent aus.
fn run(source: &str, n: u32, stress: bool) -> Run {
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress,
        verify: true,
    })
    .expect("Agent");
    let realm = agent.create_realm().expect("Realm");
    let script = format!(
        "var N = {n};\n{PRELUDE}try {{\n{source}\n}} catch (error) {{\n    result = 'Fehler: ' + error;\n}}"
    );
    let units: Vec<u16> = script.encode_utf16().collect();
    let budget = Rc::new(Budget::new(STEPS));
    let result = agent.with_execution_budget(budget, |agent| {
        let evaluated = agent.scope(|agent, scope| {
            agent
                .evaluate_script(scope, realm, &units)
                .map(|_| ())
                .map_err(|error| format!("{error:?}"))
        });
        evaluated?;
        agent
            .run_jobs()
            .map_err(|error| format!("Jobs: {error:?}"))?;
        let name: Vec<u16> = "result".encode_utf16().collect();
        agent.scope(|agent, scope| {
            let value = agent
                .evaluate_script(scope, realm, &name)
                .map_err(|error| format!("{error:?}"))?;
            let heap = agent.heap();
            Ok(match heap.get(value).kind() {
                Kind::String(string) => String::from_utf16_lossy(&heap.string_units(string)),
                kind => format!("{kind:?}"),
            })
        })
    });
    assert!(!agent.has_pending_jobs(), "offene Jobs");
    assert_eq!(
        agent.heap_mut().push_registers(0),
        Some(0),
        "gesicherte Rahmen"
    );
    let young = agent.heap().young_collections();
    let full = agent.heap().collections().saturating_sub(young);
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
    Run {
        result: result.unwrap_or_else(|error: String| error),
        young,
        full,
    }
}

/// Führt alle Programme aus, jedes in einem eigenen Faden, und gibt die
/// Läufe mit ihren Namen zurück; scheitert, wenn eines nicht `"ok"` ergibt.
fn run_all(n: u32, stress: bool) -> Vec<(&'static str, Run)> {
    let runs: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = PROGRAMS
            .iter()
            .map(|&(name, source)| (name, scope.spawn(move || run(source, n, stress))))
            .collect();
        workers
            .into_iter()
            .map(|(name, worker)| (name, worker.join().expect("Faden des Programms")))
            .collect()
    });
    let failures: Vec<String> = runs
        .iter()
        .filter(|(_, run)| run.result != "ok")
        .map(|(name, run)| format!("{name}: {}", run.result))
        .collect();
    assert!(
        failures.is_empty(),
        "N = {n}, Belastungsmodus {stress}:\n{}",
        failures.join("\n")
    );
    for (name, run) in &runs {
        println!(
            "GC-Programm {name} (N = {n}, Belastungsmodus {stress}): {} junge und {} volle Sammlungen",
            run.young, run.full
        );
    }
    runs
}

#[test]
fn gc_programs_survive_young_collections() {
    for (name, run) in run_all(20_000, false) {
        assert!(run.young >= 1, "{name}: keine junge Sammlung");
    }
}

#[test]
fn gc_programs_survive_stress_collections() {
    for (name, run) in run_all(8, true) {
        assert!(
            run.young >= 1 && run.full >= 1,
            "{name}: zu wenige Sammlungen"
        );
    }
}

#[test]
#[ignore = "nächtlicher Belastungstest, siehe .github/workflows/belastung.yml"]
fn gc_programs_at_scale() {
    for (name, run) in run_all(300_000, false) {
        assert!(run.young >= 10, "{name}: zu wenige junge Sammlungen");
    }
}

#[test]
#[ignore = "nächtlicher Belastungstest, siehe .github/workflows/belastung.yml"]
fn gc_programs_under_stress() {
    run_all(400, true);
}
