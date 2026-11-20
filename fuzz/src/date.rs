//! `Date` über JavaScript, mit einem Host, dessen Uhr und Zeitzone aus der
//! Eingabe kommen.
//!
//! Das erste Byte wählt in den Bits 0 und 1 die Zeitzone: UTC, ein
//! Vielfaches einer Viertelstunde, eine Zahl von Minuten oder den Text der
//! Eingabe als Bezeichner, den `TimeZone::from_identifier` liest (ohne
//! Treffer gilt UTC). Bit 2 nimmt die acht Bytes des Zeitwerts als Bits
//! einer Gleitkommazahl, sonst als ganze Zahl knapp über den Bereich von
//! TimeClip hinaus; Bit 3 ebenso für die Uhr des Hosts. Dann folgen zwei
//! Bytes für die Zeitzone (niedriges Byte zuerst), acht für den Zeitwert,
//! acht für die Uhr und als Rest ein Text in Latin-1. Das Script prüft:
//!
//! - `Date.now()`, `new Date()` und `Date()` zeigen die Uhr des Hosts nach
//!   TimeClip.
//! - Für den Zeitwert und für das, was `Date.parse` aus dem Text liest:
//!   Ein Ergebnis von `Date.parse` ist NaN oder ganz und im Bereich, und
//!   `new Date(text)` ergibt dasselbe.
//! - Ein ungültiges Datum hat nur NaN in seinen Feldern, `Invalid Date` als
//!   Text, und `toISOString` wirft einen RangeError.
//! - Bei einem gültigen ergeben die UTC-Felder und die Ortsfelder, mit
//!   `setUTCFullYear` und `setUTCHours` wieder zusammengesetzt, den Wert
//!   und den Wert plus den Abstand der Zeitzone; der Wochentag folgt aus
//!   der Zahl der Tage, und `getTimezoneOffset` ist der Abstand.
//! - Setter mit den eigenen Feldern ändern den Wert nicht, `Date.UTC` und
//!   der Konstruktor mit Feldern ergeben ihn wieder.
//! - `toISOString` hat die Form des Date Time String Format und wird von
//!   `Date.parse` genau gelesen, `toUTCString` und bei Abständen in ganzen
//!   Minuten `toString` auf die Sekunde; die übrigen Texte passen zu ihnen.
//!
//! Erlaubt sind ein bestandenes Script und das Ende einer Quote. Wie beim
//! Ziel `bigint` läuft jede achte Eingabe (nach der Summe ihrer Bytes)
//! außerdem im Belastungsmodus des Collectors, und beide Läufe müssen
//! dasselbe ergeben, außer wenn einer den Speicher erschöpft.

use std::rc::Rc;

use mb_js::agent::{Abrupt, Agent, Budget, Exception, Exhausted};
use mb_js::date::TimeZone;
use mb_js::heap::Options;
use mb_js::host::Host;
use mb_js::script::ScriptError;
use mb_js::value::Kind;

const SCRIPT_STEPS: u64 = 50_000;
const HEAP_BYTES: usize = 4 << 20;
/// Jede so vielte Eingabe läuft auch im Belastungsmodus.
const STRESS_EVERY: u32 = 8;
/// Der größte Betrag eines Zeitwerts (Abschnitt 21.4.1.31), als ganze Zahl.
const MAX_TIME: i64 = 8_640_000_000_000_000;
/// Minuten eines Tages minus eins: der größte Abstand in ganzen Minuten.
const MAX_OFFSET_MINUTES: i64 = 24 * 60 - 1;

const CHECKS: &str = r#"
function fail(what) { throw "Fehler: " + what; }
function clip(t) { return Math.abs(t) <= 8.64e15 ? Math.trunc(t) + 0 : NaN; }
function weekday(t) { return ((Math.floor(t / 864e5) + 4) % 7 + 7) % 7; }
var DAYS = "(Sun|Mon|Tue|Wed|Thu|Fri|Sat)";
var MONTHS = "(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)";
var ISO = /^([+-]\d{6}|\d{4})-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/;
var TEXT = new RegExp("^" + DAYS + " " + MONTHS + " \\d\\d -?\\d{4,6} \\d\\d:\\d\\d:\\d\\d GMT[+-]\\d{4}( \\(.+\\))?$");
var UTC_TEXT = new RegExp("^" + DAYS + ", \\d\\d " + MONTHS + " -?\\d{4,6} \\d\\d:\\d\\d:\\d\\d GMT$");
function invalid(d) {
  var fields = [d.getFullYear(), d.getMonth(), d.getDate(), d.getDay(), d.getHours(),
    d.getMinutes(), d.getSeconds(), d.getMilliseconds(), d.getUTCFullYear(), d.getUTCMonth(),
    d.getUTCDate(), d.getUTCDay(), d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds(),
    d.getUTCMilliseconds(), d.getTimezoneOffset(), d.getYear()];
  for (var i = 0; i < fields.length; i++) if (fields[i] === fields[i]) fail("Feld " + i + " ungültig");
  if (d.toString() !== "Invalid Date" || d.toUTCString() !== "Invalid Date") fail("Text ungültig");
  if (d.toDateString() !== "Invalid Date" || d.toLocaleString() !== "Invalid Date") fail("Teiltext ungültig");
  if (JSON.stringify(d) !== "null" || d.toJSON() !== null) fail("JSON ungültig");
  try { d.toISOString(); fail("toISOString ungültig"); } catch (e) { if (!(e instanceof RangeError)) throw e; }
  if (d.setMilliseconds(0) === d.setMilliseconds(0)) fail("Setter ungültig");
}
function check(d, t) {
  if (!Object.is(d.getTime(), t) || !Object.is(+d, t) || !Object.is(d.valueOf(), t)) fail("Zeitwert");
  if (t !== t) { invalid(d); return; }
  var u = new Date(0);
  u.setUTCFullYear(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate());
  if (u.setUTCHours(d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds(), d.getUTCMilliseconds()) !== t) fail("UTC-Felder");
  if (d.getUTCDay() !== weekday(t)) fail("Wochentag UTC");
  if (d.getTimezoneOffset() !== -OFF / 60000) fail("getTimezoneOffset");
  var l = t + OFF, inside = Math.abs(l) <= 8.64e15;
  var v = new Date(0);
  v.setUTCFullYear(d.getFullYear(), d.getMonth(), d.getDate());
  var local = v.setUTCHours(d.getHours(), d.getMinutes(), d.getSeconds(), d.getMilliseconds());
  if (inside ? local !== l : local === local) fail("Ortsfelder");
  if (d.getDay() !== weekday(l)) fail("Wochentag");
  if (d.getYear() !== d.getFullYear() - 1900) fail("getYear");
  var s = new Date(t);
  if (s.setMilliseconds(d.getMilliseconds()) !== t || s.setSeconds(d.getSeconds()) !== t) fail("setMilliseconds");
  if (s.setFullYear(d.getFullYear(), d.getMonth(), d.getDate()) !== t) fail("setFullYear");
  if (s.setHours(d.getHours(), d.getMinutes(), d.getSeconds(), d.getMilliseconds()) !== t) fail("setHours");
  if (s.setUTCMonth(d.getUTCMonth(), d.getUTCDate()) !== t || s.setUTCMinutes(d.getUTCMinutes()) !== t) fail("setUTCMonth");
  if (s.setTime(t) !== t || s.getTime() !== t) fail("setTime");
  var y = d.getUTCFullYear(), w = d.getFullYear();
  if ((y < 0 || y > 99) && Date.UTC(y, d.getUTCMonth(), d.getUTCDate(), d.getUTCHours(),
      d.getUTCMinutes(), d.getUTCSeconds(), d.getUTCMilliseconds()) !== t) fail("Date.UTC");
  if ((w < 0 || w > 99) && new Date(w, d.getMonth(), d.getDate(), d.getHours(), d.getMinutes(),
      d.getSeconds(), d.getMilliseconds()).getTime() !== t) fail("Konstruktor mit Feldern");
  var iso = d.toISOString();
  if (!ISO.test(iso) || iso.slice(0, 7) === "-000000") fail("Form von toISOString: " + iso);
  if (Date.parse(iso) !== t) fail("toISOString zurück: " + iso);
  if (JSON.stringify(d) !== '"' + iso + '"' || d.toJSON() !== iso) fail("toJSON");
  var second = t - ((t % 1000) + 1000) % 1000;
  var utc = d.toUTCString();
  if (!UTC_TEXT.test(utc) || d.toGMTString() !== utc) fail("Form von toUTCString: " + utc);
  if (Date.parse(utc) !== second) fail("toUTCString zurück: " + utc);
  var text = d.toString();
  if (!TEXT.test(text)) fail("Form von toString: " + text);
  if (text !== d.toDateString() + " " + d.toTimeString()) fail("toString in Teilen");
  if (d.toLocaleString() !== d.toLocaleDateString() + ", " + d.toLocaleTimeString()) fail("toLocaleString");
  if (d[Symbol.toPrimitive]("default") !== text || d + "" !== text || d[Symbol.toPrimitive]("number") !== t) fail("toPrimitive");
  if (OFF % 60000 === 0) {
    var back = Date.parse(text);
    if (back !== second) fail("toString zurück: " + text + " ergibt " + back);
  }
}
function main() {
  var c = clip(C);
  if (!Object.is(Date.now(), c) || !Object.is(new Date().getTime(), c)) fail("Uhr");
  if (Date() !== new Date(c).toString()) fail("Date()");
  check(new Date(T), clip(T));
  var p = Date.parse(S);
  if (p === p && (p % 1 !== 0 || Math.abs(p) > 8.64e15)) fail("Date.parse außerhalb: " + p);
  if (!Object.is(new Date(S).getTime(), p)) fail("new Date(Text)");
  check(new Date(p), p);
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

/// Ein Host mit fester Uhr und Zeitzone.
struct Fixed {
    time: f64,
    zone: TimeZone,
}

impl Host for Fixed {
    fn current_time(&self) -> f64 {
        self.time
    }

    fn time_zone(&self) -> TimeZone {
        self.zone
    }
}

/// Was eine Eingabe festlegt: Zeitzone, Zeitwert, Uhr und Text.
struct Input {
    zone: TimeZone,
    time: f64,
    clock: f64,
    text: String,
}

pub fn date(data: &[u8]) {
    if data.len() > crate::MAX_INPUT {
        return;
    }
    let input = input(data);
    let source = script(&input);
    let normal = run(&source, &input, false);
    let sum = data
        .iter()
        .fold(0u32, |sum, &byte| sum.wrapping_add(u32::from(byte)));
    if sum % STRESS_EVERY != 0 {
        return;
    }
    let stress = run(&source, &input, true);
    if normal != Outcome::OutOfMemory && stress != Outcome::OutOfMemory {
        assert_eq!(normal, stress, "Ergebnis hängt vom Collector ab");
    }
}

fn input(data: &[u8]) -> Input {
    let mut header = [0u8; 19];
    let length = data.len().min(header.len());
    header[..length].copy_from_slice(&data[..length]);
    let rest = &data[length..];
    let control = header[0];
    let parameter = u16::from_le_bytes([header[1], header[2]]);
    let mut time = [0u8; 8];
    time.copy_from_slice(&header[3..11]);
    let mut clock = [0u8; 8];
    clock.copy_from_slice(&header[11..19]);
    let text: String = rest.iter().map(|&byte| char::from(byte)).collect();
    let minutes = |minutes: i64| TimeZone::Offset(minutes * 60_000_000_000);
    let zone = match control & 3 {
        0 => TimeZone::Utc,
        1 => minutes((i64::from(parameter) % 191 - 95) * 15),
        2 => minutes(i64::from(parameter) % (2 * MAX_OFFSET_MINUTES + 1) - MAX_OFFSET_MINUTES),
        _ => TimeZone::from_identifier(&text).unwrap_or(TimeZone::Utc),
    };
    Input {
        zone,
        time: number(time, control & 4 != 0),
        clock: number(clock, control & 8 != 0),
        text,
    }
}

/// Die Bits einer Gleitkommazahl oder eine ganze Zahl, die TimeClip knapp
/// über den Rand hinaus prüft.
fn number(bytes: [u8; 8], raw: bool) -> f64 {
    if raw {
        return f64::from_bits(u64::from_le_bytes(bytes));
    }
    let bound = MAX_TIME + 1;
    let value = i64::from_le_bytes(bytes).rem_euclid(2 * bound + 1) - bound;
    // Unter 2⁵³ ist die ganze Zahl als Number genau.
    value as f64
}

/// Eine Zahl als Literal, das JavaScript zum selben Wert liest.
fn literal(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else {
        // Rust schreibt die kürzeste Ziffernfolge, die zum Wert zurückführt,
        // auch für -0.
        format!("{value}")
    }
}

/// Der Abstand in Millisekunden, wie LocalTime (Abschnitt 21.4.1.25) ihn
/// aus den Nanosekunden abschneidet.
fn offset_milliseconds(zone: TimeZone) -> i64 {
    match zone {
        TimeZone::Utc => 0,
        TimeZone::Offset(nanoseconds) => nanoseconds / 1_000_000,
    }
}

fn script(input: &Input) -> String {
    let mut text = String::from("\"");
    for character in input.text.chars() {
        if character.is_ascii_alphanumeric() || " :+-.,/()".contains(character) {
            text.push(character);
        } else {
            text.push_str(&format!("\\u{:04x}", u32::from(character)));
        }
    }
    text.push('"');
    format!(
        "var T = {};\nvar C = {};\nvar OFF = {};\nvar S = {text};\n{CHECKS}",
        literal(input.time),
        literal(input.clock),
        offset_milliseconds(input.zone),
    )
}

fn run(source: &str, input: &Input, stress: bool) -> Outcome {
    let source: Vec<u16> = source.encode_utf16().collect();
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress,
        verify: true,
    })
    .expect("Agent für Date");
    agent.set_host(Rc::new(Fixed {
        time: input.clock,
        zone: input.zone,
    }));
    let realm = agent.create_realm().expect("Realm für Date");
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

    fn data(control: u8, parameter: u16, time: i64, clock: i64, text: &str) -> Vec<u8> {
        let mut data = vec![control];
        data.extend_from_slice(&parameter.to_le_bytes());
        data.extend_from_slice(&time.to_le_bytes());
        data.extend_from_slice(&clock.to_le_bytes());
        data.extend_from_slice(text.as_bytes());
        data
    }

    fn outcome(data: &[u8]) -> Outcome {
        let input = input(data);
        let source = script(&input);
        let normal = run(&source, &input, false);
        assert_eq!(run(&source, &input, true), normal);
        normal
    }

    #[test]
    fn values_reach_the_engine_and_its_checks() {
        assert_eq!(outcome(&[]), Outcome::Passed);
        let now = 1_577_977_445_123;
        for (control, parameter, time, text) in [
            (0, 0, 0, "1970-01-01T00:00:00Z"),
            (1, 117, now, "Thu Jan 02 2020 20:34:05 GMT+0530"),
            (2, 2 * 1439, -1, "1/2/2020, 3:04:05 PM"),
            (2, 0, MAX_TIME, "-271821-04-20T00:00:00.000Z"),
            (1, 0, -MAX_TIME, "+275760-09-13T00:00:00.000Z"),
            (0, 0, MAX_TIME + 1, "Sat, 13 Sep 275760 00:00:00 GMT"),
            (3, 0, -62_198_755_200_000, "+05:30:15.5"),
            (3, 0, 253_402_300_799_999, "-23:59:59.999999999"),
            (0, 0, 0, "not a date (but a comment)"),
        ] {
            assert_eq!(
                outcome(&data(control, parameter, time, now, text)),
                Outcome::Passed,
                "{text}"
            );
        }
        let zone = input(&data(1, 117, 0, 0, "")).zone;
        assert_eq!(offset_milliseconds(zone), 19_800_000);
        let zone = input(&data(3, 0, 0, 0, "-00:00:00.0019")).zone;
        assert_eq!(offset_milliseconds(zone), -1);
    }

    #[test]
    fn raw_numbers_and_clocks_reach_time_clip() {
        for bits in [
            f64::NAN.to_bits(),
            f64::INFINITY.to_bits(),
            (-0.0f64).to_bits(),
            0.5f64.to_bits(),
            (-8.64e15 - 0.5f64).to_bits(),
            1e300f64.to_bits(),
        ] {
            let raw = i64::from_le_bytes(bits.to_le_bytes());
            assert_eq!(outcome(&data(4 | 8, 0, raw, raw, "")), Outcome::Passed);
        }
        let source = script(&input(&data(4, 0, (-0.0f64).to_bits() as i64, 0, "\"\\")));
        assert!(source.contains("var T = -0;"), "{source}");
        assert!(source.contains("var S = \"\\u0022\\u005c\";"), "{source}");
    }
}
