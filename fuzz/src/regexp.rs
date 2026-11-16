//! Die Engine für reguläre Ausdrücke über `RegExp`, mit kleinen Quoten.
//!
//! Das erste Byte wählt die Flags, ein Bit je Flag in der Reihenfolge
//! `dgimsuvy`. Der Rest ist das Muster und, nach dem ersten Nullbyte, die
//! Eingabe, beide als UTF-8 gelesen. Ein Script legt den Ausdruck an, ruft
//! `exec`, `test`, `replace`, `matchAll`, `split`, `search` und `match` auf
//! und prüft dabei Eigenschaften, die für jedes Muster gelten:
//!
//! - Ein Treffer liegt in der Eingabe, und `index` und `indices` zeigen auf
//!   genau die Teile, die er und seine Gruppen erfasst haben.
//! - `test` sagt dasselbe wie `exec`.
//! - Jeden Treffer durch sich selbst zu ersetzen (`$&`) ergibt die Eingabe.
//! - `flags` nennt die Flags in der festen Reihenfolge, `source` ergibt
//!   wieder dieselbe Quelle, und `toString` setzt beide zusammen.
//! - Das Ergebnis von `RegExp.escape` passt genau auf die Eingabe, ohne
//!   Flags und mit `v`.
//!
//! Erlaubt sind ein bestandenes Script, ein SyntaxError für ein ungültiges
//! Muster, eine nicht unterstützte Unicode-Eigenschaft und das Ende einer
//! Quote. Jede achte Eingabe (nach der Summe ihrer Bytes, also für dieselbe
//! Eingabe immer gleich) läuft außerdem im Belastungsmodus des Collectors,
//! der vor jeder Anforderung sammelt; beide Läufe müssen dasselbe ergeben,
//! außer wenn einer den Speicher erschöpft, denn das hängt vom Zeitpunkt
//! der Sammlungen ab. Im Belastungsmodus kostet jede Anforderung eine Kopie
//! des ganzen Realms; in einer Messung dauerte ein Lauf so 30 bis 50 ms
//! statt 2 ms, und mit jeder Eingabe käme das Ziel kaum voran.

use std::rc::Rc;

use mb_js::agent::{Abrupt, Agent, Budget, Exception, Exhausted};
use mb_js::heap::Options;
use mb_js::script::ScriptError;
use mb_js::value::Kind;

const SCRIPT_STEPS: u64 = 10_000;
const HEAP_BYTES: usize = 4 << 20;
/// Jede so vielte Eingabe läuft auch im Belastungsmodus.
const STRESS_EVERY: u32 = 8;
const FLAGS: &str = "dgimsuvy";

const CHECKS: &str = r#"
function fail(what) { throw "Fehler: " + what; }
function main() {
  var r;
  try { r = new RegExp(P, F); } catch (e) {
    if (e instanceof SyntaxError) return "syntax";
    throw e;
  }
  var canonical = "";
  for (var c of "dgimsuvy") if (F.indexOf(c) >= 0) canonical += c;
  if (r.flags !== canonical) fail("flags");
  if (r.toString() !== "/" + r.source + "/" + r.flags) fail("toString");
  if (new RegExp(r.source, r.flags).source !== r.source) fail("source");
  function check(m) {
    if (m === null) return;
    var end = m.index + m[0].length;
    if (!(m.index >= 0 && end <= S.length && S.slice(m.index, end) === m[0])) fail("Treffer");
    if (m.input !== S) fail("input");
    if (!r.hasIndices) return;
    for (var k = 0; k < m.length; k++) {
      var span = m.indices[k];
      if (m[k] === undefined) {
        if (span !== undefined) fail("indices " + k);
      } else if (span === undefined || S.slice(span[0], span[1]) !== m[k]) {
        fail("indices " + k);
      }
    }
  }
  var m = r.exec(S);
  check(m);
  r.lastIndex = 0;
  if (r.test(S) !== (m !== null)) fail("test");
  var g = new RegExp(P, r.global ? r.flags : r.flags + "g");
  if (S.replace(g, "$&") !== S) fail("replace");
  for (var x of S.matchAll(g)) check(x);
  S.split(r);
  S.search(r);
  S.match(g);
  var escaped = "^(?:" + RegExp.escape(S) + ")$";
  if (!new RegExp(escaped, "").test(S)) fail("escape");
  if (!new RegExp(escaped, "v").test(S)) fail("escape v");
  return true;
}
main();
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Syntax,
    Unsupported,
    Exhausted(Exhausted),
    OutOfMemory,
}

pub fn regexp(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let source = script(data);
    let normal = run(&source, false);
    let sum = data
        .iter()
        .fold(0u32, |sum, &byte| sum.wrapping_add(u32::from(byte)));
    if sum % STRESS_EVERY != 0 {
        return;
    }
    let stress = run(&source, true);
    if normal != Outcome::OutOfMemory && stress != Outcome::OutOfMemory {
        assert_eq!(normal, stress, "Ergebnis hängt vom Collector ab");
    }
}

/// Das Script zu einer Eingabe: Muster, Flags und Eingabe als
/// Zeichenketten, jede Codeeinheit als `\uXXXX`, dann die Prüfungen.
fn script(data: &[u8]) -> Vec<u16> {
    let (flags, rest) = data
        .split_first()
        .map_or((0, data), |(&first, rest)| (first, rest));
    let (pattern, subject) = match rest.iter().position(|&byte| byte == 0) {
        Some(split) => (&rest[..split], &rest[split + 1..]),
        None => (rest, &[][..]),
    };
    let flags: String = FLAGS
        .chars()
        .enumerate()
        .filter(|&(bit, _)| flags & (1 << bit) != 0)
        .map(|(_, flag)| flag)
        .collect();
    let mut text = String::new();
    for (name, value) in [
        ("P", String::from_utf8_lossy(pattern)),
        ("F", flags.into()),
        ("S", String::from_utf8_lossy(subject)),
    ] {
        text.push_str("var ");
        text.push_str(name);
        text.push_str(" = \"");
        for unit in value.encode_utf16() {
            text.push_str(&format!("\\u{unit:04x}"));
        }
        text.push_str("\";\n");
    }
    text.push_str(CHECKS);
    text.encode_utf16().collect()
}

fn run(source: &[u16], stress: bool) -> Outcome {
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress,
        verify: true,
    })
    .expect("Agent für reguläre Ausdrücke");
    let realm = agent.create_realm().expect("Realm für reguläre Ausdrücke");
    let budget = Rc::new(Budget::new(SCRIPT_STEPS));
    let outcome = agent.scope(|agent, scope| {
        let result = agent.evaluate_script_within(scope, realm, source, Rc::clone(&budget));
        let outcome = match result {
            Ok(value) => match agent.heap().get(value).kind() {
                Kind::Boolean(true) => Outcome::Passed,
                Kind::String(string) if agent.heap().string_units(string) == units("syntax") => {
                    Outcome::Syntax
                }
                kind => panic!("unerwartetes Ergebnis: {kind:?}"),
            },
            // Die Prüfungen selbst übersetzt der Übersetzer; das zeigen die
            // Tests unten. Was bleibt, ist eine Unicode-Eigenschaft ohne Daten.
            Err(ScriptError::Unsupported(_) | ScriptError::Abrupt(Abrupt::Unsupported(_))) => {
                Outcome::Unsupported
            }
            Err(ScriptError::Abrupt(Abrupt::Exhausted(reason))) => {
                assert_eq!(budget.exhausted(), Some(reason), "Quote des Hosts");
                Outcome::Exhausted(reason)
            }
            Err(ScriptError::Abrupt(Abrupt::OutOfMemory)) => Outcome::OutOfMemory,
            Err(ScriptError::Abrupt(Abrupt::Throw)) => match agent.take_exception(scope) {
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
            Err(error) => panic!("das Script selbst ist gültig: {error:?}"),
        };
        assert!(agent.take_exception(scope).is_none(), "keine alte Ausnahme");
        outcome
    });
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
    outcome
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(flags: &str, pattern: &str, subject: &str) -> Outcome {
        let bits = flags
            .chars()
            .map(|flag| 1u8 << FLAGS.find(flag).expect("Flag"))
            .fold(0, |bits, bit| bits | bit);
        let mut data = vec![bits];
        data.extend_from_slice(pattern.as_bytes());
        data.push(0);
        data.extend_from_slice(subject.as_bytes());
        let source = script(&data);
        let normal = run(&source, false);
        assert_eq!(run(&source, true), normal);
        normal
    }

    #[test]
    fn patterns_reach_the_engine_and_its_checks() {
        assert_eq!(outcome("dg", "(?<x>a)|(b)", "xaby"), Outcome::Passed);
        assert_eq!(
            outcome("iu", "\\u{1F600}|[^a]", "A\u{1F600}"),
            Outcome::Passed
        );
        assert_eq!(outcome("v", "[\\p{L}--[a-z]]+", "abcÄÖ"), Outcome::Passed);
        assert_eq!(outcome("y", "(?<=a)b\\1(c)?", "abc"), Outcome::Passed);
        assert_eq!(outcome("", "(", "x"), Outcome::Syntax);
        assert_eq!(outcome("uv", "a", "a"), Outcome::Syntax);
        assert_eq!(outcome("u", "\\p{Script=Greek}", "α"), Outcome::Unsupported);
        assert_eq!(
            outcome("", "(a*)*b", &"a".repeat(64)),
            Outcome::Exhausted(Exhausted::Steps)
        );
    }
}
