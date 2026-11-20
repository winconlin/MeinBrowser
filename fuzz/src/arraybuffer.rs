//! `ArrayBuffer` und `DataView` über JavaScript, gegen ein Modell der Bytes
//! im Script.
//!
//! Die ersten drei Bytes legen den ersten ArrayBuffer fest: seine Länge
//! (Byte 0 modulo 33), was sein Maximum darüber liegt (Byte 1 modulo 33)
//! und ob er veränderlich ist (Bit 0 von Byte 2). Danach beschreiben je elf
//! Bytes einen Schritt, höchstens [`MAX_STEPS`]: die Art des Schritts
//! (Byte 0 modulo 8, die Bits 3 und 4 wählen die Art einer Number), die Art
//! der Elemente (Byte 1 modulo 11, Bit 7 für little-endian), einen Abstand
//! (Byte 2) und acht Bytes für den Wert (niedriges Byte zuerst); das erste
//! davon dient auch als zweiter Abstand und als Wert eines Bytes. Ein BigInt
//! ist die ganze Zahl aus den acht Bytes, um bis zu 63 Bits verschoben;
//! eine Number ist eine ganze Zahl bis 2⁵⁹, die Bits einer
//! Gleitkommazahl, ein Viertel einer kleinen ganzen Zahl oder ein Randwert
//! wie NaN, -0 oder ein Gleichstand beim Runden auf binary16.
//!
//! Die Schritte schreiben und lesen jede Art von Elementen über einen
//! `DataView` auf dem ganzen ArrayBuffer und über einen zweiten auf einem
//! Teil, ändern die Größe, übertragen mit `transfer` und
//! `transferToFixedLength`, schneiden mit `slice` aus, legen Sichten und
//! neue ArrayBuffer an. Das Script führt die Bytes als Array mit und prüft:
//!
//! - Jeder Schritt wirft genau dann, wenn das Modell es verlangt, mit
//!   TypeError oder RangeError, und lässt dann alles unverändert.
//! - Nach jedem Schritt stimmen Länge, Maximum, `resizable`, `detached` und
//!   jedes Byte mit dem Modell überein, ebenso `byteLength` und
//!   `byteOffset` beider Sichten, auch wenn sie außerhalb liegen.
//! - Ein geschriebener Wert kommt so zurück, wie ihn die Operatoren der
//!   Sprache umwandeln: `<<` und `>>` für die kleinen ganzen Zahlen, `|`
//!   und `>>>` für 32 Bit, `BigInt.asIntN` und `BigInt.asUintN` für 64 Bit,
//!   `Math.fround` und `Math.f16round` für binary32 und binary16.
//!   Schreiben ändert nur seine Bytes; ganze Zahlen ergeben sich aus den
//!   Bytes in der gewählten Reihenfolge, und Lesen in der anderen
//!   Reihenfolge liest die umgedrehten Bytes.
//!
//! Erlaubt sind ein bestandenes Script und das Ende einer Quote. Wie bei
//! den Zielen `bigint` und `date` läuft jede achte Eingabe (nach der Summe
//! ihrer Bytes) außerdem im Belastungsmodus des Collectors, und beide Läufe
//! müssen dasselbe ergeben, außer wenn einer den Speicher erschöpft.

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
/// Höchstens so viele Schritte liest das Ziel aus einer Eingabe.
const MAX_STEPS: usize = 48;
/// Die Bytes eines Schritts.
const STEP_BYTES: usize = 11;
/// Die Bytes vor den Schritten.
const HEADER_BYTES: usize = 3;
/// So viele Randwerte kennt [`special`].
const SPECIALS: u8 = 16;

/// Ein Randwert für Numbers, auch Gleichstände beim Runden auf binary16 und
/// binary32 und Grenzen der ganzen Zahlen.
fn special(index: u8) -> f64 {
    match index % SPECIALS {
        0 => f64::NAN,
        1 => -0.0,
        2 => f64::INFINITY,
        3 => f64::NEG_INFINITY,
        4 => 65504.0,
        5 => 65520.0,
        6 => 65519.99,
        7 => 2f64.powi(-24),
        8 => 2f64.powi(-25),
        9 => 1.0 + 2f64.powi(-11),
        10 => 1.0 + 2f64.powi(-11) + 2f64.powi(-40),
        11 => 2f64.powi(24) + 1.0,
        12 => 2f64.powi(31),
        13 => -(2f64.powi(31)) - 1.0,
        14 => 2f64.powi(32) + 0.5,
        _ => 2f64.powi(53) + 2.0,
    }
}

const CHECKS: &str = r#"
var TYPES = ["Int8", "Uint8", "Int16", "Uint16", "Int32", "Uint32", "BigInt64", "BigUint64",
  "Float16", "Float32", "Float64"];
var SIZES = [1, 1, 2, 2, 4, 4, 8, 8, 2, 4, 8];
function fail(what) { throw "Fehler: " + what; }
function same(a, b) {
  if (typeof a === "bigint" || typeof b === "bigint") return a === b;
  return Object.is(a, b) || (a !== a && b !== b);
}
function throws(kind, f, what) {
  try { f(); } catch (e) { if (e instanceof kind) return; throw e; }
  fail("kein " + kind.name + ": " + what);
}
function expected(t, x) {
  switch (TYPES[t]) {
    case "Int8": return (x << 24) >> 24;
    case "Uint8": return x & 255;
    case "Int16": return (x << 16) >> 16;
    case "Uint16": return x & 65535;
    case "Int32": return x | 0;
    case "Uint32": return x >>> 0;
    case "BigInt64": return BigInt.asIntN(64, x);
    case "BigUint64": return BigInt.asUintN(64, x);
    case "Float16": return Math.f16round(x);
    case "Float32": return Math.fround(x);
  }
  return x;
}
var B, M, MAX, V, VOFF, VLEN, W, WOFF, WLEN;
function zeros(n) { var a = []; for (var i = 0; i < n; i++) a.push(0); return a; }
function fresh(length, max) {
  B = max === null ? new ArrayBuffer(length) : new ArrayBuffer(length, { maxByteLength: max });
  M = zeros(length);
  MAX = max;
  V = new DataView(B);
  VOFF = 0;
  VLEN = max === null ? length : undefined;
  W = V; WOFF = VOFF; WLEN = VLEN;
}
function viewLength(off, len) {
  if (M === null || off > M.length) return -1;
  if (len === undefined) return M.length - off;
  return off + len > M.length ? -1 : len;
}
function compose(o, size, le) {
  var v = size === 8 ? 0n : 0;
  for (var i = 0; i < size; i++) {
    var b = M[o + (le ? size - 1 - i : i)];
    v = size === 8 ? v * 256n + BigInt(b) : v * 256 + b;
  }
  return v;
}
function fromBytes(t, o, le) {
  var size = SIZES[t], u = compose(o, size, le);
  switch (TYPES[t]) {
    case "Int8": return (u << 24) >> 24;
    case "Int16": return (u << 16) >> 16;
    case "Int32": return u | 0;
    case "BigInt64": return BigInt.asIntN(64, u);
    case "Uint8": case "Uint16": case "Uint32": case "BigUint64": return u;
  }
  var s = new DataView(new ArrayBuffer(8));
  for (var i = 0; i < size; i++) s.setUint8(i, M[o + i]);
  return s["get" + TYPES[t]](0, le);
}
function checkView(view, off, len, where) {
  var n = viewLength(off, len);
  if (view.buffer !== B) fail("buffer der Sicht " + where);
  if (n < 0) {
    if (view.byteLength !== 0 || view.byteOffset !== 0) fail("Sicht außerhalb " + where);
    throws(TypeError, function () { view.getInt8(0); }, "Sicht außerhalb " + where);
    return;
  }
  if (view.byteLength !== n || view.byteOffset !== off) fail("Sicht " + where);
  for (var i = 0; i < n; i++) if (view.getUint8(i) !== M[off + i]) fail("Byte der Sicht " + where);
  throws(RangeError, function () { view.getUint8(n); }, "hinter der Sicht " + where);
}
function check(where) {
  if (B.resizable !== (MAX !== null)) fail("resizable " + where);
  if (M === null) {
    if (!B.detached || B.byteLength !== 0 || B.maxByteLength !== 0) fail("abgelöst " + where);
    throws(TypeError, function () { B.slice(0); }, "slice abgelöst " + where);
  } else {
    if (B.detached || B.byteLength !== M.length) fail("Länge " + where);
    if (B.maxByteLength !== (MAX === null ? M.length : MAX)) fail("Maximum " + where);
    var all = new DataView(B);
    for (var i = 0; i < M.length; i++) if (all.getUint8(i) !== M[i]) fail("Byte " + i + " " + where);
  }
  checkView(V, VOFF, VLEN, where);
  checkView(W, WOFF, WLEN, where);
}
function set(t, r, x, le) {
  var size = SIZES[t], n = viewLength(VOFF, VLEN), o = r % (n < 0 ? 4 : n + 4);
  var name = TYPES[t];
  if (n < 0) { throws(TypeError, function () { V["set" + name](o, x, le); }, "set außerhalb"); return; }
  if (o + size > n) { throws(RangeError, function () { V["set" + name](o, x, le); }, "set " + o); return; }
  V["set" + name](o, x, le);
  var all = new DataView(B), start = VOFF + o;
  for (var i = 0; i < M.length; i++) {
    var b = all.getUint8(i);
    if (i < start || i >= start + size) { if (b !== M[i]) fail("Nachbarbyte " + i); }
    else M[i] = b;
  }
  var want = expected(t, x);
  if (!same(V["get" + name](o, le), want)) fail("Rundreise " + name + " " + x);
  if (!same(fromBytes(t, start, le), want)) fail("Bytes " + name + " " + x);
  if (!same(V["get" + name](o, !le), fromBytes(t, start, !le))) fail("andere Reihenfolge " + name);
}
function get(t, r, le) {
  var size = SIZES[t], n = viewLength(WOFF, WLEN), o = r % (n < 0 ? 4 : n + 4);
  var name = "get" + TYPES[t];
  if (n < 0) { throws(TypeError, function () { W[name](o, le); }, "get außerhalb"); return; }
  if (o + size > n) { throws(RangeError, function () { W[name](o, le); }, "get " + o); return; }
  if (!same(W[name](o, le), fromBytes(t, WOFF + o, le))) fail(name + " " + o);
}
function resize(r) {
  var m = r % ((MAX === null ? 32 : MAX) + 4);
  if (MAX === null || M === null) { throws(TypeError, function () { B.resize(m); }, "resize"); return; }
  if (m > MAX) { throws(RangeError, function () { B.resize(m); }, "resize " + m); return; }
  if (B.resize(m) !== undefined) fail("resize gibt etwas zurück");
  if (m < M.length) M.length = m; else while (M.length < m) M.push(0);
}
function transfer(t, r, s, fixed) {
  var length = t % 4 === 0 ? undefined : r % 40;
  var f = function () { return fixed ? B.transferToFixedLength(length) : B.transfer(length); };
  if (M === null) { throws(TypeError, f, "transfer abgelöst"); return; }
  var n = length === undefined ? M.length : length, max = fixed ? null : MAX;
  if (max !== null && n > max) { throws(RangeError, f, "transfer " + n); return; }
  var old = B, next = f();
  if (!old.detached || old.byteLength !== 0) fail("transfer löst nicht ab");
  if (next === old || next.byteLength !== n) fail("transfer Länge");
  // Mit dem abgelösten ArrayBuffer und seinen Sichten weiter.
  if (s & 1) { M = null; return; }
  B = next;
  M = M.slice(0, n);
  while (M.length < n) M.push(0);
  MAX = max;
  V = new DataView(B); VOFF = 0; VLEN = max === null ? n : undefined;
  W = V; WOFF = VOFF; WLEN = VLEN;
}
function cut(t, r, s) {
  var start = (r % 41) - 20, end = t % 3 === 0 ? undefined : (s % 41) - 20;
  if (M === null) { throws(TypeError, function () { B.slice(start, end); }, "slice abgelöst"); return; }
  var n = M.length;
  var first = start < 0 ? Math.max(n + start, 0) : Math.min(start, n);
  var last = end === undefined ? n : end < 0 ? Math.max(n + end, 0) : Math.min(end, n);
  var piece = B.slice(start, end), want = M.slice(first, Math.max(last, first));
  if (piece === B || piece.resizable || piece.byteLength !== want.length) fail("slice Länge");
  var view = new DataView(piece);
  for (var i = 0; i < want.length; i++) if (view.getUint8(i) !== want[i]) fail("slice Byte " + i);
}
function view(t, r, s) {
  var n = M === null ? 0 : M.length, off = r % (n + 3);
  var length = t % 3 === 0 ? undefined : (s >> 2) % (n + 3);
  var f = function () { return new DataView(B, off, length); };
  if (M === null) { throws(TypeError, f, "Sicht abgelöst"); return; }
  if (off > n || (length !== undefined && off + length > n)) { throws(RangeError, f, "Sicht " + off); return; }
  W = f(); WOFF = off;
  WLEN = length !== undefined ? length : MAX === null ? n - off : undefined;
}
function byte(r, value) {
  var n = viewLength(WOFF, WLEN), o = r % (n < 0 ? 4 : n + 4);
  if (n < 0) { throws(TypeError, function () { W.setUint8(o, value); }, "Byte außerhalb"); return; }
  if (o >= n) { throws(RangeError, function () { W.setUint8(o, value); }, "Byte " + o); return; }
  W.setUint8(o, value);
  M[WOFF + o] = value;
}
function replace(r, s, resizable) {
  var length = r % 33, max = length + (s % 35) - 2;
  if (!resizable) { fresh(length, null); return; }
  if (max < length) {
    throws(RangeError, function () { new ArrayBuffer(length, { maxByteLength: max }); }, "Maximum zu klein");
    return;
  }
  fresh(length, max);
}
function main() {
  fresh(START[0], START[1]);
  check("am Anfang");
  for (var i = 0; i < STEPS.length; i++) {
    var step = STEPS[i], op = step[0], t = step[1], r = step[2], le = step[3], x = step[4], s = step[5];
    switch (op) {
      case 0: set(t, r, x, le); break;
      case 1: get(t, r, le); break;
      case 2: resize(r); break;
      case 3: transfer(t, r, s, !le); break;
      case 4: cut(t, r, s); break;
      case 5: view(t, r, s); break;
      case 6: byte(r, s); break;
      default: replace(r, s, le);
    }
    check("nach Schritt " + i);
  }
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

pub fn arraybuffer(data: &[u8]) {
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

/// Eine Number als Literal, das JavaScript zum selben Wert liest.
fn literal(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else if value == 0.0 && value.is_sign_negative() {
        "-0".to_owned()
    } else {
        // Rust schreibt die kürzeste Ziffernfolge, die zum Wert zurückführt.
        format!("{value}")
    }
}

/// Der Wert eines Schritts als Literal: ein BigInt für die Arten mit 64
/// Bit, sonst eine Number nach den Bits 3 und 4 der Art des Schritts.
fn value(kind: u8, control: u8, shift: u8, bytes: [u8; 8]) -> String {
    let integer = i64::from_le_bytes(bytes);
    if matches!(kind, 6 | 7) {
        let wide = i128::from(integer) << (shift % 64);
        return format!("{wide}n");
    }
    let number = match (control >> 3) & 3 {
        // Bis 2⁵⁹ ist die ganze Zahl als Number genau.
        0 => (integer >> 4) as f64,
        1 => f64::from_bits(u64::from_le_bytes(bytes)),
        2 => f64::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 4.0,
        _ => special(bytes[0]),
    };
    literal(number)
}

fn script(data: &[u8]) -> String {
    let mut header = [0u8; HEADER_BYTES];
    let length = data.len().min(HEADER_BYTES);
    header[..length].copy_from_slice(&data[..length]);
    let start = u32::from(header[0] % 33);
    let max = if header[2] & 1 == 1 {
        (start + u32::from(header[1] % 33)).to_string()
    } else {
        "null".to_owned()
    };
    let mut steps = String::new();
    for record in data[length..].chunks(STEP_BYTES).take(MAX_STEPS) {
        let mut step = [0u8; STEP_BYTES];
        step[..record.len()].copy_from_slice(record);
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&step[3..]);
        let kind = step[1] % 11;
        let _ = write!(
            steps,
            "[{}, {kind}, {}, {}, {}, {}],",
            step[0] % 8,
            step[2],
            step[1] >= 128,
            value(kind, step[0], step[1] >> 1, bytes),
            step[3],
        );
    }
    format!("var START = [{start}, {max}];\nvar STEPS = [{steps}];\n{CHECKS}")
}

fn run(source: &str, stress: bool) -> Outcome {
    let source: Vec<u16> = source.encode_utf16().collect();
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress,
        verify: true,
    })
    .expect("Agent für ArrayBuffer");
    let realm = agent.create_realm().expect("Realm für ArrayBuffer");
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

    /// Ein Schritt aus Art, Art der Elemente, Abstand und Wert.
    fn step(op: u8, kind: u8, offset: u8, value: [u8; 8]) -> Vec<u8> {
        let mut bytes = vec![op, kind, offset];
        bytes.extend_from_slice(&value);
        bytes
    }

    fn outcome(data: &[u8]) -> Outcome {
        let source = script(data);
        let normal = run(&source, false);
        assert_eq!(run(&source, true), normal);
        normal
    }

    #[test]
    fn every_step_reaches_the_engine_and_its_checks() {
        assert_eq!(outcome(&[]), Outcome::Passed);
        let pi = std::f64::consts::PI.to_bits().to_le_bytes();
        let minus = (-5i64 << 4).to_le_bytes();
        let mut data = vec![16, 8, 1];
        for kind in 0..11 {
            // Jede Art als ganze Zahl, als Bits einer Gleitkommazahl und
            // als Randwert, in beiden Reihenfolgen, und Lesen über die
            // zweite Sicht.
            data.extend(step(0, kind, 3, minus));
            data.extend(step(1 << 3, kind | 128, 5, pi));
            data.extend(step(3 << 3, kind + 11, 1, [9; 8]));
            data.extend(step(1, kind | 128, 2, [0; 8]));
        }
        assert_eq!(outcome(&data), Outcome::Passed);
        let mut data = vec![4, 20, 1];
        for (op, kind, offset, second) in [
            (5, 1, 2, 0x41),    // Sicht ab 2 mit Länge 16
            (5, 0, 2, 0x41),    // Sicht ab 2, die mitwächst
            (2, 0, 22, 0x41),   // resize auf 22
            (6, 0, 3, 0x41),    // ein Byte über die Sicht
            (4, 1, 22, 0x41),   // slice von 2 bis 45 % 41 - 20
            (2, 0, 1, 0x41),    // kleiner, die Sicht liegt außerhalb
            (3, 128, 30, 0x40), // transfer(30) über das Maximum: RangeError
            (3, 129, 9, 0x40),  // transfer(9)
            (3, 1, 30, 0x40),   // transferToFixedLength(30)
            (2, 0, 3, 0x41),    // resize ohne Maximum: TypeError
            (3, 0, 0, 0x41),    // transferToFixedLength() und weiter mit dem alten
            (0, 0, 0, 0x41),    // set auf dem abgelösten: TypeError
            (4, 0, 0, 0x41),    // slice auf dem abgelösten: TypeError
            (5, 0, 0, 0x41),    // Sicht auf dem abgelösten: TypeError
            (7, 128, 7, 0x41),  // neuer veränderlicher ArrayBuffer
            (7, 128, 0, 0),     // Maximum zu klein: RangeError
            (7, 0, 40, 0x41),   // neuer fester ArrayBuffer
            (5, 0, 200, 0x41),  // Sicht außerhalb
        ] {
            data.extend(step(op, kind, offset, [second; 8]));
        }
        assert_eq!(outcome(&data), Outcome::Passed);
    }

    #[test]
    fn values_are_literals_of_their_kind() {
        assert_eq!(value(6, 0, 1, (-1i64).to_le_bytes()), "-2n");
        assert_eq!(value(7, 0, 63, 1i64.to_le_bytes()), "9223372036854775808n");
        assert_eq!(value(0, 0, 0, (-16i64).to_le_bytes()), "-1");
        assert_eq!(value(8, 1 << 3, 0, (-0.0f64).to_bits().to_le_bytes()), "-0");
        assert_eq!(value(9, 2 << 3, 0, [6, 0, 0, 0, 0, 0, 0, 0]), "1.5");
        assert_eq!(value(10, 3 << 3, 0, [0; 8]), "NaN");
        let source = script(&[1, 2, 1]);
        assert!(source.contains("var START = [1, 3];"), "{source}");
        let source = script(&[40, 2, 0, 9]);
        assert!(source.contains("var START = [7, null];"), "{source}");
        assert!(source.contains("[1, 0, 0, false, 0, 0],"), "{source}");
    }
}
