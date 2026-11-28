//! Tests für DisposableStack und AsyncDisposableStack (Schritt 13d), mit
//! den Hilfen aus [`super::using_tests`].

use super::using_tests::{check, check_async, check_with};

#[test]
fn stacks_dispose_their_resources_in_reverse_order() {
    let cases = [
        (
            "var s = new DisposableStack(); s.use(res('a')); s.defer(() => log.push('b')); \
             s.adopt('c', v => log.push(v)); s.dispose(); log.join()",
            "c,b,a",
        ),
        // `use` ruft mit dem Wert als `this`, `adopt` mit *undefined* und
        // dem Wert, `defer` ohne Argumente.
        (
            "var r = { [Symbol.dispose]() { log.push(this === r, arguments.length); } }; \
             var s = new DisposableStack(); s.use(r); \
             s.adopt(1, function (v) { 'use strict'; log.push(typeof this, v); }); \
             s.defer(function () { log.push(arguments.length); }); s.dispose(); log.join()",
            "0,undefined,1,true,0",
        ),
        (
            "var s = new DisposableStack(); var r = res('a'); \
             [s.use(r) === r, s.adopt(r, () => {}) === r, s.defer(() => {})].join()",
            "true,true,",
        ),
        // *null* und *undefined* kommen bei ~sync-dispose~ nicht hinzu.
        (
            "var s = new DisposableStack(); [s.use(null), s.use(undefined)].join(); s.dispose(); \
             log.length",
            "0",
        ),
        // Mehr Ressourcen, als die erste Zelle fasst.
        (
            "var s = new DisposableStack(); for (let i = 0; i < 10; i++) s.defer(() => log.push(i)); \
             s.dispose(); log.join()",
            "9,8,7,6,5,4,3,2,1,0",
        ),
        // Ein zweites `dispose` tut nichts.
        (
            "var s = new DisposableStack(); s.use(res('a')); var d = [s.disposed]; \
             s.dispose(); d.push(s.disposed, s.dispose()); s.dispose(); d.join() + ' ' + log.join()",
            "false,true, a",
        ),
        // @@dispose ist `dispose`, darum geht der Stapel mit `using`.
        (
            "{ using s = new DisposableStack(); s.defer(() => log.push('x')); log.push('body'); } \
             [log.join(), DisposableStack.prototype[Symbol.dispose] === DisposableStack.prototype.dispose, \
              Object.prototype.toString.call(new DisposableStack())].join()",
            "body,x,true,[object DisposableStack]",
        ),
    ];
    check(&cases);
    check_with(&cases, false, true);
}

#[test]
fn stacks_check_their_receivers_values_and_state() {
    check(&[
        (
            "try { DisposableStack(); } catch (e) { e.name }",
            "TypeError",
        ),
        ("new DisposableStack().use(1)", "TypeError"),
        ("new DisposableStack().use({})", "TypeError"),
        (
            "new DisposableStack().use({ [Symbol.dispose]: 1 })",
            "TypeError",
        ),
        // Ohne @@dispose hilft @@asyncDispose bei ~sync-dispose~ nicht.
        (
            "new DisposableStack().use({ [Symbol.asyncDispose]() {} })",
            "TypeError",
        ),
        ("new DisposableStack().adopt(1, 2)", "TypeError"),
        ("new DisposableStack().defer({})", "TypeError"),
        (
            "var s = new DisposableStack(); s.dispose(); \
             [() => s.use(res('a')), () => s.adopt(1, () => {}), () => s.defer(() => {}), () => s.move()] \
             .map(f => { try { f(); } catch (e) { return e.name; } }).join()",
            "ReferenceError,ReferenceError,ReferenceError,ReferenceError",
        ),
        // Während des Freigebens ist der Stapel schon ~disposed~.
        (
            "var s = new DisposableStack(); \
             s.defer(() => { try { s.defer(() => {}); } catch (e) { log.push(e.name, s.disposed); } }); \
             s.dispose(); log.join()",
            "ReferenceError,true",
        ),
        // Die Methoden verlangen den richtigen Stapel.
        ("DisposableStack.prototype.dispose.call({})", "TypeError"),
        (
            "DisposableStack.prototype.dispose.call(new AsyncDisposableStack())",
            "TypeError",
        ),
        (
            "Object.getOwnPropertyDescriptor(DisposableStack.prototype, 'disposed').get \
             .call(new AsyncDisposableStack())",
            "TypeError",
        ),
        (
            "AsyncDisposableStack.prototype.use.call(new DisposableStack(), null)",
            "TypeError",
        ),
    ]);
}

#[test]
fn errors_from_stacks_are_joined_with_suppressed_error() {
    check(&[
        // Ein einzelner Fehler bleibt, wie er ist.
        (
            "var s = new DisposableStack(); s.defer(() => { throw 1; }); s.use(res('a')); \
             try { s.dispose(); } catch (e) { e + ' ' + log.join() }",
            "1 a",
        ),
        (
            "var s = new DisposableStack(); s.defer(() => { throw 1; }); s.defer(() => { throw 2; }); \
             try { s.dispose(); } catch (e) { [e instanceof SuppressedError, e.error, e.suppressed].join() }",
            "true,1,2",
        ),
        // Auch Fehler der Engine.
        (
            "var s = new DisposableStack(); s.defer(() => null.x); s.defer(() => { throw 2; }); \
             try { s.dispose(); } catch (e) { [e.error instanceof TypeError, e.suppressed].join() }",
            "true,2",
        ),
        (
            "var s = new DisposableStack(); s.defer(() => { throw 2; }); s.defer(() => null.x); \
             try { s.dispose(); } catch (e) { [e.error, e.suppressed instanceof TypeError].join() }",
            "2,true",
        ),
        (
            "var s = new DisposableStack(); s.defer(() => null.x); \
             try { s.dispose(); } catch (e) { e instanceof TypeError }",
            "true",
        ),
    ]);
}

#[test]
fn move_hands_the_resources_to_a_new_stack() {
    check(&[
        (
            "var s = new DisposableStack(); s.defer(() => log.push('a')); var t = s.move(); \
             log.push(s.disposed, t.disposed); s.dispose(); log.push('between'); t.dispose(); log.join()",
            "true,false,between,a",
        ),
        // Der neue Stapel kommt aus %DisposableStack%, auch bei Unterklassen.
        (
            "class M extends DisposableStack {} var m = new M(); var t = m.move(); \
             [m instanceof M, t instanceof M, t instanceof DisposableStack].join()",
            "true,false,true",
        ),
        (
            "class M extends AsyncDisposableStack {} var t = new M().move(); \
             [t instanceof M, Object.getPrototypeOf(t) === AsyncDisposableStack.prototype].join()",
            "false,true",
        ),
        // Der alte Stapel ist ~disposed~, der neue nimmt weitere Ressourcen.
        (
            "var s = new DisposableStack(); s.defer(() => log.push('a')); var t = s.move(); \
             t.defer(() => log.push('b')); t.dispose(); log.join()",
            "b,a",
        ),
        // `use` hängt die Ressource an die [[DisposeCapability]], die es vor
        // dem Getter der Methode gelesen hat: Zieht der Getter sie mit `move`
        // zu einem neuen Stapel, gibt dieser sie frei.
        (
            "var s = new DisposableStack(); var t; s.defer(() => log.push('a')); \
             s.use({ get [Symbol.dispose]() { t = s.move(); return () => log.push('r'); } }); \
             s.dispose(); log.push(s.disposed); t.dispose(); log.join()",
            "true,r,a",
        ),
        (
            "var s = new AsyncDisposableStack(); var t; \
             s.use({ get [Symbol.asyncDispose]() { t = s.move(); return () => log.push('r'); } }); \
             t.disposeAsync(); log.join()",
            "r",
        ),
        // Gibt der Getter den Stapel selbst frei, geht die Ressource wie in
        // der Spezifikation verloren.
        (
            "var s = new DisposableStack(); \
             s.use({ get [Symbol.dispose]() { s.dispose(); return () => log.push('r'); } }); \
             s.dispose(); log.length",
            "0",
        ),
    ]);
}

#[test]
fn async_stacks_await_each_disposal() {
    check_async(&[
        (
            "var s = new AsyncDisposableStack(); \
             s.use({ async [Symbol.asyncDispose]() { log.push('a'); } }); \
             s.defer(async () => log.push('b')); \
             s.disposeAsync().then(v => log.push('done', v)); log.push('sync');",
            "b,sync,a,done,",
        ),
        // Auf das Ergebnis jeder Methode wird gewartet.
        (
            "var s = new AsyncDisposableStack(); s.defer(() => { log.push('second'); }); \
             s.defer(() => Promise.resolve().then(() => log.push('first'))); \
             s.disposeAsync().then(() => log.push('done'));",
            "first,second,done",
        ),
        // `adopt` ruft mit dem Wert; @@dispose wird nicht abgewartet.
        (
            "var s = new AsyncDisposableStack(); s.adopt('v', v => log.push(v)); \
             s.use({ [Symbol.dispose]() { log.push('d'); return new Promise(() => {}); } }); \
             s.disposeAsync().then(() => log.push('done'));",
            "d,v,done",
        ),
        // Eine Ressource aus *null* verlangt ein Await, ein leerer Stapel
        // keins.
        (
            "var s = new AsyncDisposableStack(); s.use(null); \
             Promise.all([Promise.resolve().then(() => 0).then(() => log.push('job 1')), \
                 s.disposeAsync().then(() => log.push('dispose')), \
                 Promise.resolve().then(() => 0).then(() => log.push('job 2'))]);",
            "job 1,dispose,job 2",
        ),
        (
            "var s = new AsyncDisposableStack(); Promise.resolve().then(() => log.push('job 1')); \
             s.disposeAsync().then(() => log.push('dispose')); \
             Promise.resolve().then(() => log.push('job 2'));",
            "job 1,dispose,job 2",
        ),
        // Ein zweites `disposeAsync` gibt nichts mehr frei.
        (
            "var s = new AsyncDisposableStack(); s.defer(() => log.push('a')); \
             Promise.all([s.disposeAsync(), s.disposeAsync()]).then(() => log.push(s.disposed));",
            "a,true",
        ),
        // Fehler und abgelehnte Promises, dazu ein falscher Empfänger:
        // Das Promise wird abgelehnt, nichts wirft.
        (
            "var s = new AsyncDisposableStack(); s.defer(() => Promise.reject(1)); \
             s.defer(() => { throw 2; }); s.disposeAsync().catch(e => log.push(e.error, e.suppressed));",
            "1,2",
        ),
        (
            "var s = new AsyncDisposableStack(); s.defer(() => Promise.reject(1)); \
             s.disposeAsync().catch(e => log.push(e));",
            "1",
        ),
        (
            "AsyncDisposableStack.prototype.disposeAsync.call(new DisposableStack()) \
             .catch(e => log.push(e.name));",
            "TypeError",
        ),
        // Mit `await using` und mehr Ressourcen, als die erste Zelle fasst.
        (
            "async function f() { await using s = new AsyncDisposableStack(); \
                 for (let i = 0; i < 6; i++) s.defer(async () => log.push(i)); log.push('body'); } \
             f().then(() => log.push('after'));",
            "body,5,4,3,2,1,0,after",
        ),
    ]);
}
