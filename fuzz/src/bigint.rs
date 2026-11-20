//! BigInt-Arithmetik über JavaScript, mit kleinen Quoten.
//!
//! Das erste Byte trägt die Vorzeichen, Bit 0 für `a` und Bit 1 für `b`,
//! und in den Bits 2 bis 7 eine Weite `k` von 0 bis 63. Die nächsten zwei
//! Bytes, eine Zahl mit dem niedrigen Byte zuerst, teilen den Rest: Die
//! ersten so vielen Bytes, genommen modulo der Länge des Rests plus eins,
//! sind der Betrag von `a`, der Rest der von `b`, beide als Hexziffern mit
//! dem höchsten Byte zuerst. Beide Werte stehen als Literale
//! im Script und werden auch mit `BigInt` aus Text gelesen. Das Script prüft
//! Gleichungen, die für alle ganzen Zahlen gelten:
//!
//! - Summe, Differenz und Produkt passen zusammen, und das Produkt geteilt
//!   durch einen Faktor ergibt den anderen.
//! - Für `b` ungleich 0 gilt `a == (a / b) * b + a % b`, der Rest ist
//!   betragsmäßig kleiner als `b` und hat das Vorzeichen von `a`; durch 0
//!   zu teilen ist ein RangeError.
//! - Verschieben um `k` Bit ist Multiplizieren mit `2 ** k`, nach rechts
//!   rundet es ab, und eine negative Weite verschiebt in die andere
//!   Richtung.
//! - Die bitweisen Operatoren rechnen im unendlichen Zweierkomplement:
//!   `(a & b) + (a | b) == a + b`, `a ^ b == (a | b) - (a & b)` und
//!   `~a == -a - 1`.
//! - `BigInt.asUintN` und `asIntN` mit `3 * k` Bit ergeben den Rest modulo
//!   `2 ** (3 * k)` im passenden Bereich.
//! - Die Texte zu den Basen 10, 16, 8 und 2 ergeben wieder den Wert.
//! - Der Vergleich mit `Number(a)` sagt genau eines von kleiner, gleich und
//!   größer, und für eine endliche Zahl dasselbe wie der Vergleich mit
//!   `BigInt(Number(a))`.
//! - `a ** 3n` ist `a * a * a`, und `Map` und `Set` finden den Wert unter
//!   einem gleichen BigInt.
//!
//! Erlaubt sind ein bestandenes Script und das Ende einer Quote. Wie beim
//! Ziel `regexp` läuft jede achte Eingabe (nach der Summe ihrer Bytes)
//! außerdem im Belastungsmodus des Collectors, und beide Läufe müssen
//! dasselbe ergeben, außer wenn einer den Speicher erschöpft.

use std::rc::Rc;

use mb_js::agent::{Abrupt, Agent, Budget, Exception, Exhausted};
use mb_js::heap::Options;
use mb_js::script::ScriptError;
use mb_js::value::Kind;

const SCRIPT_STEPS: u64 = 50_000;
const HEAP_BYTES: usize = 4 << 20;
/// Jede so vielte Eingabe läuft auch im Belastungsmodus.
const STRESS_EVERY: u32 = 8;

const CHECKS: &str = r#"
function fail(what) { throw "Fehler: " + what; }
function abs(x) { return x < 0n ? -x : x; }
function main() {
  if ((SA ? -BigInt(AH) : BigInt(AH)) !== a) fail("Text von a");
  if ((SB ? -BigInt(BH) : BigInt(BH)) !== b) fail("Text von b");
  if ((a + b) - b !== a || a - b !== -(b - a)) fail("Summe");
  if (a * b !== b * a || a * -1n !== -a) fail("Produkt");
  if (b !== 0n) {
    var q = a / b, r = a % b;
    if (q * b + r !== a) fail("Division");
    if (abs(r) >= abs(b)) fail("Rest zu groß");
    if (r !== 0n && (r < 0n) !== (a < 0n)) fail("Vorzeichen des Rests");
    if (a * b / b !== a) fail("Produkt durch Faktor");
  } else {
    try { a / b; fail("Division durch 0"); } catch (e) { if (!(e instanceof RangeError)) throw e; }
    try { a % b; fail("Rest durch 0"); } catch (e) { if (!(e instanceof RangeError)) throw e; }
  }
  var p = 1n << K;
  if (p !== 2n ** K) fail("Zweierpotenz");
  if (a << K !== a * p || (a << K) >> K !== a) fail("Verschiebung nach links");
  var f = a >> K;
  if (f * p > a || (f + 1n) * p <= a) fail("Verschiebung rundet ab");
  if (a >> -K !== a << K || a << -K !== f) fail("negative Weite");
  if ((a & b) + (a | b) !== a + b) fail("und, oder");
  if ((a ^ b) !== (a | b) - (a & b)) fail("xor");
  if (~a !== -a - 1n || (a & ~b) !== a - (a & b)) fail("nicht");
  var n = 3n * K, m = 1n << n;
  var u = ((a % m) + m) % m;
  if (BigInt.asUintN(Number(n), a) !== u) fail("asUintN");
  if (BigInt.asIntN(Number(n), a) !== (n === 0n ? 0n : u >= m / 2n ? u - m : u)) fail("asIntN");
  var x = abs(a);
  if (BigInt(a.toString()) !== a || String(a) !== a.toString()) fail("Basis 10");
  if (BigInt("0x" + x.toString(16)) !== x || BigInt("0o" + x.toString(8)) !== x) fail("Basen 16 und 8");
  if (BigInt("0b" + x.toString(2)) !== x) fail("Basis 2");
  if (a.toString(36) !== (a < 0n ? "-" : "") + x.toString(36)) fail("Vorzeichen im Text");
  var d = Number(a);
  if ((a < d) + (a == d) + (a > d) !== 1) fail("Vergleich mit Number");
  if (Number.isFinite(d)) {
    var e = BigInt(d);
    if ((a < d) !== (a < e) || (a == d) !== (a === e) || (d < a) !== (e < a)) fail("Number als BigInt");
  } else if (!(a < 0n ? a > d : a < d)) {
    fail("Unendlich");
  }
  if (a ** 3n !== a * a * a) fail("Potenz");
  var map = new Map([[a, 1]]);
  if (map.get(BigInt(a.toString())) !== 1 || !new Set([a]).has(a + 0n)) fail("Map und Set");
  return true;
}
main();
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Exhausted(Exhausted),
    OutOfMemory,
}

pub fn bigint(data: &[u8]) {
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

/// Das Script zu einer Eingabe: `a` und `b` als Literale, ihre Beträge als
/// Text, die Vorzeichen, die Weite `K`, dann die Prüfungen.
fn script(data: &[u8]) -> String {
    let (control, rest) = data
        .split_first()
        .map_or((0, data), |(&first, rest)| (first, rest));
    let (split, rest) = match rest {
        [low, high, rest @ ..] => (usize::from(u16::from_le_bytes([*low, *high])), rest),
        _ => (0, &[][..]),
    };
    let (a, b) = rest.split_at(split % (rest.len() + 1));
    let mut text = String::new();
    for (name, negative, bytes) in [("a", control & 1 != 0, a), ("b", control & 2 != 0, b)] {
        let mut hex = String::from("0x");
        for byte in bytes {
            hex.push_str(&format!("{byte:02x}"));
        }
        if bytes.is_empty() {
            hex.push('0');
        }
        let sign = if negative { "-" } else { "" };
        let upper = name.to_uppercase();
        text.push_str(&format!(
            "var {name} = {sign}{hex}n;\nvar {upper}H = \"{hex}\";\nvar S{upper} = {negative};\n"
        ));
    }
    text.push_str(&format!("var K = {}n;\n", control >> 2));
    text.push_str(CHECKS);
    text
}

fn run(source: &str, stress: bool) -> Outcome {
    let source: Vec<u16> = source.encode_utf16().collect();
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress,
        verify: true,
    })
    .expect("Agent für BigInt");
    let realm = agent.create_realm().expect("Realm für BigInt");
    let budget = Rc::new(Budget::new(SCRIPT_STEPS));
    let outcome = agent.scope(|agent, scope| {
        let result = agent.evaluate_script_within(scope, realm, &source, Rc::clone(&budget));
        let outcome = match result {
            Ok(value) => match agent.heap().get(value).kind() {
                Kind::Boolean(true) => Outcome::Passed,
                kind => panic!("unerwartetes Ergebnis: {kind:?}"),
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(control: u8, a: &[u8], b: &[u8]) -> Outcome {
        let length = u16::try_from(a.len()).expect("kurz").to_le_bytes();
        let mut data = vec![control, length[0], length[1]];
        data.extend_from_slice(a);
        data.extend_from_slice(b);
        let source = script(&data);
        let normal = run(&source, false);
        assert_eq!(run(&source, true), normal);
        normal
    }

    #[test]
    fn values_reach_the_engine_and_its_checks() {
        assert_eq!(outcome(0, &[], &[]), Outcome::Passed);
        assert_eq!(outcome(1 | 5 << 2, &[0x12, 0x34], &[0x05]), Outcome::Passed);
        assert_eq!(
            outcome(3 | 63 << 2, &[0xff; 17], &[0x80; 9]),
            Outcome::Passed
        );
        // Ein Divisor mit gesetztem obersten Bit und einer, bei dem die
        // Schätzung des Quotienten in Knuths Algorithmus D korrigiert wird.
        assert_eq!(
            outcome(
                2 | 1 << 2,
                &[0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                &[0xff; 9]
            ),
            Outcome::Passed
        );
        assert_eq!(
            outcome(
                0,
                &[
                    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0
                ],
                &[0x80, 0, 0, 0, 0, 0, 0, 1]
            ),
            Outcome::Passed
        );
        let source = script(&[0, 0, 0]);
        assert!(source.contains("var a = 0x0n;"), "{source}");
    }

    /// Werte bis 1 KiB kommen durch alle Prüfungen, zwei Werte mit je
    /// 2 KiB enden an der Quote.
    #[test]
    fn long_values_pass_or_end_at_the_quota() {
        let short = [0xa5; 255];
        let long = vec![0x5a; 1024];
        assert_eq!(outcome(3 | 63 << 2, &short, &long), Outcome::Passed);
        assert_eq!(outcome(1 | 7 << 2, &long, &short), Outcome::Passed);
        let half = vec![0x5a; (crate::MAX_INPUT - 3) / 2];
        assert_eq!(
            outcome(2, &half, &half),
            Outcome::Exhausted(Exhausted::Steps)
        );
    }
}
