//! `SharedArrayBuffer` und `Atomics` über JavaScript, gegen ein Modell der
//! Bytes und der Warter in Rust.
//!
//! Die ersten drei Bytes legen die Puffer fest: die Länge des
//! SharedArrayBuffer `S` und des ArrayBuffer `A`, der nicht geteilt ist
//! (Byte 0 modulo 5, mal 8), was das Maximum von `S` darüber liegt (Byte 1
//! modulo 5, mal 8) und ob `S` wächst (Bit 0 von Byte 2). Auf beiden liegt
//! je ein typisiertes Array jeder Art mit ganzen Zahlen und eines der vier
//! Arten, die `Atomics` ablehnt. Danach beschreiben je zwölf Bytes einen
//! Schritt, höchstens [`MAX_STEPS`]:
//!
//! - Byte 0 modulo 13 ist die Operation: `add`, `sub`, `and`, `or`, `xor`,
//!   `exchange`, `compareExchange`, `load`, `store`, `grow`, `waitAsync`
//!   oder `wait`, `notify` und `slice`.
//! - Byte 1: die Art der Elemente (modulo 8), Bit 6 wählt `A` statt `S`,
//!   Bit 7 eine abgelehnte Art; die Bits 4 und 5 wählen für `waitAsync`
//!   die Wartezeit 0, +∞ oder 10⁹ ms oder statt dessen `wait`.
//! - Byte 2: der Index (modulo 12), mit Bit 7 ein Randwert wie -1, 2⁵³,
//!   `"3"` oder `NaN`; für `grow` die neue Länge (modulo 65), für `slice`
//!   der Anfang (modulo 48, minus 8).
//! - Byte 3: Bit 0 wählt ein BigInt statt einer Number, die Bits 1 und 2
//!   die Form der Number (ganze Zahl, Viertel, Randwert oder die Bits
//!   eines binary64), Bit 3 lässt `compareExchange` und `waitAsync` den
//!   aktuellen Wert erwarten, `notify` ohne Anzahl und `slice` ohne Ende
//!   aufrufen; die Bits 4 bis 7, mal 4, verschieben den Wert.
//! - Bytes 4 bis 11: der Wert, niedriges Byte zuerst; für `slice` ist das
//!   erste davon das Ende (modulo 48, minus 8).
//!
//! Das Script prüft nach jedem Schritt das Ergebnis oder die Art des
//! Fehlers, die Bytes des Puffers und, nach `grow`, Länge und die Längen
//! der Sichten. `waitAsync` hängt an jedes Promise einen Eintrag in ein
//! Protokoll; nach den Jobs prüft ein zweites Script, dass genau die
//! Warter, die `notify` nach dem Modell geweckt hat, in dieser Reihenfolge
//! *"ok"* bekamen, und noch einmal alle Bytes. Das Modell wandelt die Werte
//! selbst um, ohne Code der Engine: ToIntegerOrInfinity, Reste modulo 2⁶⁴
//! für die Bytes, ToIndex für die Randwerte, die Listen der Warter nach dem
//! Index im Block, auch wenn ein Int32Array und ein BigInt64Array
//! dieselben Bytes nennen. [[CanBlock]] ist *false*, also wirft `wait`.
//!
//! Erlaubt sind bestandene Scripts und das Ende der Quote. Jede achte
//! Eingabe, nach der Summe ihrer Bytes, läuft außerdem mit ihren ersten
//! [`STRESS_STEPS`] Schritten im Belastungsmodus des Collectors.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::rc::Rc;

use mb_js::agent::{Abrupt, Agent, Budget, Exception, Exhausted};
use mb_js::heap::Options;
use mb_js::realm::RealmId;
use mb_js::script::ScriptError;
use mb_js::value::Kind;

const SCRIPT_STEPS: u64 = 200_000;
const HEAP_BYTES: usize = 4 << 20;
/// Jede so vielte Eingabe läuft auch im Belastungsmodus.
const STRESS_EVERY: u32 = 8;
/// So viele Schritte laufen im Belastungsmodus.
const STRESS_STEPS: usize = 8;
/// Höchstens so viele Schritte liest das Ziel aus einer Eingabe.
const MAX_STEPS: usize = 48;
/// Die Bytes eines Schritts.
const STEP_BYTES: usize = 12;
/// Die Bytes vor den Schritten.
const HEADER_BYTES: usize = 3;
const TWO_63: f64 = 9_223_372_036_854_775_808.0;
const TWO_64: f64 = 18_446_744_073_709_551_616.0;

/// Die Arten mit ganzen Zahlen in der Reihenfolge von `CTORS`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Type {
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    BigInt64,
    BigUint64,
}

const TYPES: [Type; 8] = [
    Type::Int8,
    Type::Uint8,
    Type::Int16,
    Type::Uint16,
    Type::Int32,
    Type::Uint32,
    Type::BigInt64,
    Type::BigUint64,
];

impl Type {
    fn size(self) -> usize {
        match self {
            Type::Int8 | Type::Uint8 => 1,
            Type::Int16 | Type::Uint16 => 2,
            Type::Int32 | Type::Uint32 => 4,
            Type::BigInt64 | Type::BigUint64 => 8,
        }
    }

    fn bigint(self) -> bool {
        matches!(self, Type::BigInt64 | Type::BigUint64)
    }

    /// Die Bits auf die Größe gekürzt.
    fn mask(self, bits: u64) -> u64 {
        match self.size() {
            8 => bits,
            size => bits & ((1u64 << (size * 8)) - 1),
        }
    }

    /// Das Element mit diesen Bytes als Literal.
    fn literal(self, bits: u64) -> String {
        let bytes = bits.to_le_bytes();
        match self {
            Type::Int8 => i8::from_le_bytes([bytes[0]]).to_string(),
            Type::Uint8 => bytes[0].to_string(),
            Type::Int16 => i16::from_le_bytes([bytes[0], bytes[1]]).to_string(),
            Type::Uint16 => u16::from_le_bytes([bytes[0], bytes[1]]).to_string(),
            Type::Int32 => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).to_string(),
            Type::Uint32 => {
                u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).to_string()
            }
            Type::BigInt64 => format!("{}n", i64::from_le_bytes(bytes)),
            Type::BigUint64 => format!("{bits}n"),
        }
    }
}

/// Ein Wert eines Schritts.
#[derive(Clone, Copy)]
enum Value {
    Number(f64),
    BigInt(i128),
}

impl Value {
    fn literal(self) -> String {
        match self {
            Value::Number(number) => number_literal(number),
            Value::BigInt(value) => format!("{value}n"),
        }
    }
}

/// Eine Number als Literal, das genau diese Number ergibt.
fn number_literal(number: f64) -> String {
    if number.is_nan() {
        "NaN".to_owned()
    } else if number == f64::INFINITY {
        "Infinity".to_owned()
    } else if number == f64::NEG_INFINITY {
        "-Infinity".to_owned()
    } else {
        // Die kürzeste Darstellung, die zurück dieselbe Zahl ergibt, wie
        // in JavaScript; -0 als "-0.0".
        format!("{number:?}")
    }
}

/// ToIntegerOrInfinity (ECMA-262, Abschnitt 7.1.5), nie -0.
fn integer_or_infinity(number: f64) -> f64 {
    if number.is_nan() {
        return 0.0;
    }
    let integer = number.trunc();
    if integer == 0.0 { 0.0 } else { integer }
}

/// Der Rest einer ganzen Zahl modulo 2⁶⁴, 0 für ±∞; exakt, denn `%` auf
/// binary64 rundet nicht.
fn modulo_bits(number: f64) -> u64 {
    let integer = integer_or_infinity(number);
    if !integer.is_finite() {
        return 0;
    }
    let rest = integer % TWO_64;
    if rest >= 0.0 {
        rest as u64
    } else if rest >= -TWO_63 {
        u64::from_le_bytes((rest as i64).to_le_bytes())
    } else {
        // Ein Vielfaches von 2¹¹ zwischen -2⁶⁴ und -2⁶³, die Summe also
        // exakt.
        (rest + TWO_64) as u64
    }
}

/// Die unteren 64 Bit eines BigInt im Zweierkomplement.
fn bigint_bits(value: i128) -> u64 {
    let bytes = value.to_le_bytes();
    let mut low = [0u8; 8];
    low.copy_from_slice(&bytes[..8]);
    u64::from_le_bytes(low)
}

/// Ein Randwert für Numbers.
fn special(index: u8) -> f64 {
    match index % 16 {
        0 => f64::NAN,
        1 => -0.0,
        2 => f64::INFINITY,
        3 => f64::NEG_INFINITY,
        4 => 2f64.powi(53),
        5 => -TWO_63,
        6 => TWO_64 + 4096.0,
        7 => 2f64.powi(31),
        8 => -(2f64.powi(31)) - 1.0,
        9 => 0.5,
        10 => -0.5,
        11 => 1e300,
        12 => 4_294_967_295.0,
        13 => 4_294_967_296.0,
        14 => 65535.75,
        _ => -1.0,
    }
}

/// Ein Index als Literal und was ToIndex daraus macht, `None` für einen
/// RangeError.
fn index(byte: u8) -> (String, Option<u64>) {
    if byte & 0x80 == 0 {
        let value = u64::from(byte % 12);
        return (value.to_string(), Some(value));
    }
    let (text, value) = match byte % 8 {
        0 => ("-1", None),
        1 => ("2 ** 53", None),
        2 => ("\"3\"", Some(3)),
        3 => ("1.9", Some(1)),
        4 => ("NaN", Some(0)),
        5 => ("-0.5", Some(0)),
        6 => ("Infinity", None),
        _ => ("undefined", Some(0)),
    };
    (text.to_owned(), value)
}

/// Ein Fehler, den ein Schritt verlangt.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Error {
    Type,
    Range,
}

impl Error {
    fn name(self) -> &'static str {
        match self {
            Error::Type => "TypeError",
            Error::Range => "RangeError",
        }
    }
}

/// Die beiden Puffer und die Warter.
struct Model {
    shared: Vec<u8>,
    plain: Vec<u8>,
    max: usize,
    growable: bool,
    /// Die Warter nach Index im Block von `S`, ihre Nummern in der
    /// Reihenfolge, in der sie kamen.
    waiters: BTreeMap<usize, Vec<usize>>,
    /// Die Nummern der geweckten Warter in der Reihenfolge, in der `notify`
    /// sie weckte.
    woken: Vec<usize>,
}

/// Ein Schritt, gelesen.
struct Step {
    operation: u8,
    kind: u8,
    position: u8,
    flags: u8,
    raw: [u8; 8],
}

impl Step {
    fn read(bytes: &[u8]) -> Option<Step> {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(bytes.get(4..12)?);
        Some(Step {
            operation: *bytes.first()? % 13,
            kind: *bytes.get(1)?,
            position: *bytes.get(2)?,
            flags: *bytes.get(3)?,
            raw,
        })
    }

    fn on_plain(&self) -> bool {
        self.kind & 0x40 != 0
    }

    fn rejected_type(&self) -> bool {
        self.kind & 0x80 != 0
    }

    fn element_type(&self) -> Type {
        TYPES[usize::from(self.kind % 8)]
    }

    /// Der Ausdruck für die Sicht dieses Schritts.
    fn view(&self) -> String {
        let buffer = if self.on_plain() { "A" } else { "S" };
        if self.rejected_type() {
            format!("W{buffer}[{}]", self.kind % 4)
        } else {
            format!("V{buffer}[{}]", self.kind % 8)
        }
    }

    fn shift(&self) -> u32 {
        u32::from(self.flags >> 4) * 4
    }

    fn value(&self) -> Value {
        let raw = i64::from_le_bytes(self.raw);
        if self.flags & 1 != 0 {
            return Value::BigInt(i128::from(raw) << self.shift());
        }
        Value::Number(match (self.flags >> 1) & 3 {
            0 => (raw >> self.shift()) as f64,
            1 => ((raw >> 40) as f64) / 4.0,
            2 => special(self.raw[0]),
            _ => f64::from_bits(u64::from_le_bytes(self.raw)),
        })
    }
}

impl Model {
    fn buffer(&self, plain: bool) -> &[u8] {
        if plain { &self.plain } else { &self.shared }
    }

    fn buffer_mut(&mut self, plain: bool) -> &mut Vec<u8> {
        if plain {
            &mut self.plain
        } else {
            &mut self.shared
        }
    }

    fn read(&self, plain: bool, offset: usize, size: usize) -> u64 {
        let mut bytes = [0u8; 8];
        if let Some(source) = self.buffer(plain).get(offset..offset + size) {
            bytes[..size].copy_from_slice(source);
        }
        u64::from_le_bytes(bytes)
    }

    fn write(&mut self, plain: bool, offset: usize, size: usize, bits: u64) {
        if let Some(target) = self.buffer_mut(plain).get_mut(offset..offset + size) {
            target.copy_from_slice(&bits.to_le_bytes()[..size]);
        }
    }

    /// Die Bytes eines Puffers als Text, wie `hex` im Script sie schreibt.
    fn hex(&self, plain: bool) -> String {
        self.buffer(plain)
            .iter()
            .fold(String::new(), |mut text, byte| {
                let _ = write!(text, "{byte:02x}");
                text
            })
    }

    /// Die Prüfungen nach einem Schritt auf einem Puffer.
    fn check_buffer(&self, plain: bool, what: &str) -> String {
        let name = if plain { "A" } else { "S" };
        format!(
            "expect(hex({name}), \"{}\", \"{name} {what}\");\n",
            self.hex(plain)
        )
    }

    /// ValidateIntegerTypedArray und ValidateAtomicAccess: der Index des
    /// Elements im Block.
    fn access(&self, step: &Step, waitable: bool) -> Result<usize, Error> {
        if step.rejected_type() {
            return Err(Error::Type);
        }
        let element = step.element_type();
        if waitable && !matches!(element, Type::Int32 | Type::BigInt64) {
            return Err(Error::Type);
        }
        self.index(step, element)
    }

    fn index(&self, step: &Step, element: Type) -> Result<usize, Error> {
        let length = self.buffer(step.on_plain()).len() / element.size();
        let (_, index) = index(step.position);
        match index.and_then(|index| usize::try_from(index).ok()) {
            Some(index) if index < length => Ok(index * element.size()),
            _ => Err(Error::Range),
        }
    }

    /// Die Bytes des Werts für ein Element dieser Art und was `store`
    /// zurückgibt, ein TypeError für die falsche Art von Wert.
    fn convert(element: Type, value: Value) -> Result<(u64, String), Error> {
        match (element.bigint(), value) {
            (true, Value::BigInt(value)) => Ok((bigint_bits(value), format!("{value}n"))),
            (false, Value::Number(number)) => {
                let integer = integer_or_infinity(number);
                Ok((modulo_bits(integer), number_literal(integer)))
            }
            _ => Err(Error::Type),
        }
    }

    fn step(&mut self, number: usize, step: &Step) -> String {
        let what = format!("Schritt {number}");
        let call = |name: &str, arguments: &str| {
            format!(
                "Atomics.{name}({}, {}{arguments})",
                step.view(),
                index(step.position).0
            )
        };
        let mut text = match step.operation {
            0..=8 => self.element_step(step, &what, call),
            9 => self.grow(step, &what),
            10 => self.wait(number, step, &what, call),
            11 => self.notify(step, &what, call),
            _ => self.slice(step, &what),
        };
        text.push_str(&self.check_buffer(step.on_plain(), &what));
        text
    }

    /// `add` bis `store`.
    fn element_step(
        &mut self,
        step: &Step,
        what: &str,
        call: impl Fn(&str, &str) -> String,
    ) -> String {
        const NAMES: [&str; 9] = [
            "add",
            "sub",
            "and",
            "or",
            "xor",
            "exchange",
            "compareExchange",
            "load",
            "store",
        ];
        let name = NAMES[usize::from(step.operation)];
        let value = step.value();
        let element = step.element_type();
        let plain = step.on_plain();
        let current = self
            .access(step, false)
            .map(|offset| self.read(plain, offset, element.size()));
        let expected = match current {
            Ok(old) if step.flags & 8 != 0 => element.literal(old),
            _ => value.literal(),
        };
        let arguments = match step.operation {
            6 => format!(", {expected}, {}", value.literal()),
            7 => String::new(),
            _ => format!(", {}", value.literal()),
        };
        let expression = call(name, &arguments);
        let outcome = self.access(step, false).and_then(|offset| {
            if step.operation == 7 {
                return Ok((offset, 0, String::new()));
            }
            let (bits, stored) = Model::convert(element, value)?;
            Ok((offset, bits, stored))
        });
        let (offset, bits, stored) = match outcome {
            Ok(outcome) => outcome,
            Err(error) => return throws(error, &expression, what),
        };
        let old = self.read(plain, offset, element.size());
        let new = match step.operation {
            0 => Some(old.wrapping_add(bits)),
            1 => Some(old.wrapping_sub(bits)),
            2 => Some(old & bits),
            3 => Some(old | bits),
            4 => Some(old ^ bits),
            5 | 8 => Some(bits),
            6 => {
                // Der erwartete Wert ist entweder das Element selbst oder
                // der Wert des Schritts.
                let expected = if step.flags & 8 != 0 { old } else { bits };
                (element.mask(expected) == old).then_some(bits)
            }
            _ => None,
        };
        if let Some(new) = new {
            self.write(plain, offset, element.size(), element.mask(new));
        }
        let result = if step.operation == 8 {
            stored
        } else {
            element.literal(old)
        };
        format!("expect({expression}, {result}, \"{what}\");\n")
    }

    /// `grow` auf `S`, mit Bit 6 aus Byte 1 auf `A`.
    fn grow(&mut self, step: &Step, what: &str) -> String {
        let length = usize::from(step.position % 65);
        if step.on_plain() {
            let expression = format!("SharedArrayBuffer.prototype.grow.call(A, {length})");
            return throws(Error::Type, &expression, what);
        }
        let expression = format!("S.grow({length})");
        if !self.growable {
            return throws(Error::Type, &expression, what);
        }
        if length < self.shared.len() || length > self.max {
            return throws(Error::Range, &expression, what);
        }
        self.shared.resize(length, 0);
        let lengths: Vec<String> = TYPES
            .iter()
            .map(|element| (length / element.size()).to_string())
            .collect();
        format!(
            "expect({expression}, undefined, \"{what}\");\n\
             expect(S.byteLength, {length}, \"Länge {what}\");\n\
             expect(lengths(VS), \"{}\", \"Sichten {what}\");\n",
            lengths.join(",")
        )
    }

    /// `waitAsync` mit drei Wartezeiten oder `wait`.
    fn wait(
        &mut self,
        number: usize,
        step: &Step,
        what: &str,
        call: impl Fn(&str, &str) -> String,
    ) -> String {
        let element = step.element_type();
        let plain = step.on_plain();
        let (name, timeout) = match (step.kind >> 4) & 3 {
            0 => ("waitAsync", "0"),
            1 => ("waitAsync", "Infinity"),
            2 => ("waitAsync", "1e9"),
            _ => ("wait", "0"),
        };
        let value = step.value();
        let current = self
            .access(step, true)
            .map(|offset| self.read(plain, offset, element.size()));
        let expected = match current {
            Ok(old) if step.flags & 8 != 0 => element.literal(old),
            _ => value.literal(),
        };
        let expression = call(name, &format!(", {expected}, {timeout}"));
        let offset = match self.access(step, true) {
            // Ein ArrayBuffer, der nicht geteilt ist, kommt vor dem Index.
            Ok(_) | Err(Error::Range) if plain => return throws(Error::Type, &expression, what),
            Ok(offset) => offset,
            Err(error) => return throws(error, &expression, what),
        };
        let old = self.read(plain, offset, element.size());
        let wanted = if step.flags & 8 != 0 {
            old
        } else {
            match (element, value) {
                (Type::Int32, Value::Number(number)) => element.mask(modulo_bits(number)),
                (Type::BigInt64, Value::BigInt(value)) => bigint_bits(value),
                _ => return throws(Error::Type, &expression, what),
            }
        };
        // [[CanBlock]] ist *false*.
        if name == "wait" {
            return throws(Error::Type, &expression, what);
        }
        if wanted != old {
            return format!(
                "var r = {expression};\n\
                 expect(r.async, false, \"{what}\"); expect(r.value, \"not-equal\", \"{what}\");\n"
            );
        }
        if timeout == "0" {
            return format!(
                "var r = {expression};\n\
                 expect(r.async, false, \"{what}\"); expect(r.value, \"timed-out\", \"{what}\");\n"
            );
        }
        self.waiters.entry(offset).or_default().push(number);
        format!(
            "var r = {expression};\n\
             expect(r.async, true, \"{what}\");\n\
             r.value.then(function (v) {{ log.push(\"{number}:\" + v); }});\n"
        )
    }

    /// `notify` mit bis zu drei, keinem oder -1 als Anzahl.
    fn notify(&mut self, step: &Step, what: &str, call: impl Fn(&str, &str) -> String) -> String {
        let (argument, count) = if step.flags & 8 != 0 {
            (String::new(), usize::MAX)
        } else {
            match (step.flags >> 4) % 5 {
                4 => (", -1".to_owned(), 0),
                count => (format!(", {count}"), usize::from(count)),
            }
        };
        let expression = call("notify", &argument);
        let offset = match self.access(step, true) {
            Ok(offset) => offset,
            Err(error) => return throws(error, &expression, what),
        };
        if step.on_plain() {
            return format!("expect({expression}, 0, \"{what}\");\n");
        }
        let list = self.waiters.entry(offset).or_default();
        let woken: Vec<usize> = list.drain(..count.min(list.len())).collect();
        let result = woken.len();
        self.woken.extend(woken);
        format!("expect({expression}, {result}, \"{what}\");\n")
    }

    /// `slice` auf `S`, mit Bit 6 aus Byte 1 auf `A`.
    fn slice(&mut self, step: &Step, what: &str) -> String {
        let start = i64::from(step.position % 48) - 8;
        let end = (step.flags & 8 == 0).then(|| i64::from(step.raw[0] % 48) - 8);
        let arguments = match end {
            Some(end) => format!("{start}, {end}"),
            None => start.to_string(),
        };
        if step.on_plain() {
            let expression = format!("SharedArrayBuffer.prototype.slice.call(A, {arguments})");
            return throws(Error::Type, &expression, what);
        }
        let length = i64::try_from(self.shared.len()).unwrap_or(i64::MAX);
        let clamp = |relative: i64| {
            if relative < 0 {
                (length + relative).max(0)
            } else {
                relative.min(length)
            }
        };
        let first = clamp(start);
        let last = end.map_or(length, clamp);
        let bytes = match (usize::try_from(first), usize::try_from(last.max(first))) {
            (Ok(first), Ok(last)) => self.shared.get(first..last).unwrap_or(&[]),
            _ => &[],
        };
        let hex = bytes.iter().fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        });
        format!(
            "var s = S.slice({arguments});\n\
             expect(s instanceof SharedArrayBuffer && s !== S, true, \"{what}\");\n\
             expect(hex(s), \"{hex}\", \"{what}\");\n"
        )
    }
}

/// Die Prüfung, dass ein Ausdruck mit diesem Fehler wirft.
fn throws(error: Error, expression: &str, what: &str) -> String {
    format!(
        "throws({}, function () {{ {expression}; }}, \"{what}\");\n",
        error.name()
    )
}

const PRELUDE: &str = r#"
function fail(what) { throw "Fehler: " + what; }
function same(a, b) { return typeof a === typeof b && Object.is(a, b); }
function expect(value, wanted, what) {
  if (!same(value, wanted)) fail(what + ": " + String(value) + " statt " + String(wanted));
}
function throws(kind, f, what) {
  try { f(); } catch (e) { if (e instanceof kind) return; throw e; }
  fail("kein " + kind.name + ": " + what);
}
function hex(buffer) {
  var bytes = new Uint8Array(buffer), text = "";
  for (var i = 0; i < bytes.length; i++) text += (bytes[i] < 16 ? "0" : "") + bytes[i].toString(16);
  return text;
}
function lengths(views) {
  var text = [];
  for (var i = 0; i < views.length; i++) text.push(views[i].length);
  return text.join();
}
var CTORS = [Int8Array, Uint8Array, Int16Array, Uint16Array, Int32Array, Uint32Array,
  BigInt64Array, BigUint64Array];
var REJECTED = [Uint8ClampedArray, Float16Array, Float32Array, Float64Array];
function views(list, buffer) {
  var result = [];
  for (var i = 0; i < list.length; i++) result.push(new list[i](buffer));
  return result;
}
var log = [];
"#;

/// Die beiden Scripts: die Schritte und die Prüfung nach den Jobs.
fn scripts(data: &[u8], max_steps: usize) -> (String, String) {
    let header = data.get(..HEADER_BYTES).unwrap_or(&[0, 0, 0]);
    let length = usize::from(header[0] % 5) * 8;
    let max = length + usize::from(header[1] % 5) * 8;
    let growable = header[2] & 1 != 0;
    let mut model = Model {
        shared: vec![0; length],
        plain: vec![0; length],
        max,
        growable,
        waiters: BTreeMap::new(),
        woken: Vec::new(),
    };
    let options = if growable {
        format!(", {{ maxByteLength: {max} }}")
    } else {
        String::new()
    };
    let mut steps = format!(
        "{PRELUDE}var S = new SharedArrayBuffer({length}{options}), A = new ArrayBuffer({length});\n\
         var VS = views(CTORS, S), VA = views(CTORS, A), WS = views(REJECTED, S), \
         WA = views(REJECTED, A);\n(function () {{\n"
    );
    let rest = data.get(HEADER_BYTES..).unwrap_or(&[]);
    let (chunks, _) = rest.as_chunks::<STEP_BYTES>();
    for (number, bytes) in chunks.iter().take(max_steps).enumerate() {
        if let Some(step) = Step::read(bytes) {
            steps.push_str(&model.step(number, &step));
        }
    }
    steps.push_str("return true;\n})()");
    let log: Vec<String> = model
        .woken
        .iter()
        .map(|number| format!("{number}:ok"))
        .collect();
    let check = format!(
        "(function () {{\nexpect(log.join(), \"{}\", \"Protokoll\");\n{}{}return true;\n}})()",
        log.join(","),
        model.check_buffer(false, "am Ende"),
        model.check_buffer(true, "am Ende"),
    );
    (steps, check)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Exhausted(Exhausted),
    OutOfMemory,
}

/// Führt ein Script aus. `None`, wenn es bestand, sonst das Ergebnis.
fn evaluate(
    agent: &mut Agent,
    realm: RealmId,
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

fn run(scripts: &(String, String), stress: bool) -> Outcome {
    let mut agent = Agent::with_options(Options {
        limit: HEAP_BYTES,
        stress,
        verify: true,
    })
    .expect("Agent für Atomics");
    let realm = agent.create_realm().expect("Realm für Atomics");
    let budget = Rc::new(Budget::new(SCRIPT_STEPS));
    let (steps, check) = scripts;
    let outcome = evaluate(&mut agent, realm, steps, &budget)
        .or_else(|| jobs(&mut agent, &budget))
        .or_else(|| {
            agent.heap_mut().collect();
            evaluate(&mut agent, realm, check, &budget)
        })
        .unwrap_or(Outcome::Passed);
    agent.heap_mut().collect();
    agent.heap().verify().expect("gültiger Heap");
    outcome
}

pub fn atomics(data: &[u8]) {
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

    /// Ein Schritt aus Operation, Art, Index, Schaltern und Wert.
    fn step(operation: u8, kind: u8, position: u8, flags: u8, value: i64) -> Vec<u8> {
        let mut bytes = vec![operation, kind, position, flags];
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes
    }

    fn input(header: [u8; 3], steps: &[Vec<u8>]) -> Vec<u8> {
        let mut data = header.to_vec();
        for step in steps {
            data.extend_from_slice(step);
        }
        data
    }

    #[test]
    fn numbers_convert_like_the_specification() {
        assert_eq!(modulo_bits(-1.0), u64::MAX);
        assert_eq!(modulo_bits(-3.0), u64::MAX - 2);
        assert_eq!(modulo_bits(f64::INFINITY), 0);
        assert_eq!(modulo_bits(f64::NAN), 0);
        assert_eq!(modulo_bits(TWO_64 + 4096.0), 4096);
        assert_eq!(modulo_bits(-TWO_63), 1 << 63);
        assert_eq!(modulo_bits(-TWO_64 + 4096.0), 4096);
        assert_eq!(modulo_bits(-(TWO_63 + 4096.0)), (1 << 63) - 4096);
        assert_eq!(modulo_bits(4.75), 4);
        assert_eq!(modulo_bits(-4.75), u64::MAX - 3);
        assert_eq!(number_literal(-0.0), "-0.0");
        assert_eq!(number_literal(integer_or_infinity(-0.5)), "0.0");
        assert_eq!(bigint_bits(-1), u64::MAX);
        assert_eq!(bigint_bits(1 << 64), 0);
    }

    #[test]
    fn steps_reach_the_engine_and_its_checks() {
        let data = input(
            [3, 2, 1],
            &[
                // add 5 auf Int32Array[1] von S, dann sub, and, or, xor,
                // exchange, compareExchange mit dem aktuellen Wert, load
                // und store auf A.
                step(0, 4, 1, 0, 5),
                step(1, 4, 1, 0, 2),
                step(2, 4, 1, 0, 1),
                step(3, 4, 1, 0, 6),
                step(4, 4, 1, 0, 3),
                step(5, 4, 1, 0, 9),
                step(6, 4, 1, 8, 7),
                step(7, 4, 1, 0, 0),
                step(8, 0x40, 2, 0, 300),
                // BigInt64 auf S, mit BigInt und mit falscher Art von Wert.
                step(0, 6, 1, 1, -5),
                step(0, 6, 1, 0, -5),
                // Abgelehnte Art, Index außerhalb und Randwert als Index.
                step(0, 0x80, 0, 0, 1),
                step(0, 4, 11, 0, 1),
                step(7, 4, 0x82, 0, 0),
                // Wachsen um 8 Bytes, über das Maximum hinaus.
                step(9, 0, 32, 0, 0),
                step(9, 0, 64, 0, 0),
                // Zwei Warter bei Byte 8, über Int32Array[2] und
                // BigInt64Array[1], einer gleich beantwortet; notify über
                // Int32Array[2] weckt beide in ihrer Reihenfolge.
                step(10, 4 | 0x10, 2, 8, 0),
                step(10, 6 | 0x20, 1, 8, 0),
                step(10, 4, 2, 8, 0),
                step(10, 4 | 0x30, 2, 8, 0),
                step(10, 4 | 0x10, 3, 0, 1),
                step(11, 4, 2, 0x20, 0),
                step(11, 4, 2, 8, 0),
                step(11, 0x44, 2, 8, 0),
                // slice mit negativem Anfang.
                step(12, 0, 4, 0, 30),
                step(12, 0x40, 4, 8, 0),
            ],
        );
        let scripts = scripts(&data, MAX_STEPS);
        assert!(scripts.0.contains("maxByteLength: 40"), "{}", scripts.0);
        assert!(
            scripts.1.contains("expect(log.join(), \"16:ok,17:ok\""),
            "{}",
            scripts.1
        );
        assert_eq!(run(&scripts, false), Outcome::Passed);
        assert_eq!(run(&scripts, true), Outcome::Passed);
    }

    #[test]
    fn fixed_buffers_and_edge_values_pass() {
        // Alle Arten mit allen Formen von Werten, Verschiebungen und
        // Operationen, die schreiben.
        let steps: Vec<Vec<u8>> = (0u8..48)
            .map(|index| {
                let flags = [0, 2, 4, 6, 0x11, 0xF0][usize::from(index % 6)];
                let operation = [0, 1, 5, 6, 8][usize::from(index % 5)];
                step(operation, index / 6, 3, flags, -0x1234_5678_9ABC_DEF1)
            })
            .collect();
        let data = input([4, 0, 0], &steps);
        assert_eq!(run(&scripts(&data, MAX_STEPS), false), Outcome::Passed);
        let data = input([1, 0, 0], &[step(9, 0, 8, 0, 0), step(10, 0x44, 0, 0, 0)]);
        assert_eq!(run(&scripts(&data, MAX_STEPS), false), Outcome::Passed);
    }

    #[test]
    fn empty_and_short_inputs_pass() {
        atomics(&[]);
        atomics(&[1]);
        atomics(&[0, 0, 1, 9]);
    }
}
