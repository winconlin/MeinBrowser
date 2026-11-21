//! ArrayBuffer und ihre Data Blocks (Abschnitte 6.2.9 und 25.1): die Arten
//! der Elemente aus der Tabelle der TypedArray-Konstruktoren (Abschnitt
//! 23.2), die Umwandlung zwischen Zahlen und Bytes (RawBytesToNumeric und
//! NumericToRawBytes, Abschnitte 25.1.3.14 und 25.1.3.17) mit binary16, und
//! die abstrakten Operationen auf ArrayBuffern, die DataView und die
//! typisierten Arrays teilen.
//!
//! Ein ArrayBuffer ist ein Objekt der Art [`ObjectClass::ArrayBuffer`]. Sein
//! erstes internes Feld zeigt auf eine Zelle aus Bytes (siehe
//! [`crate::heap`]) oder ist *null*, wenn er abgelöst ist; die Länge der
//! Zelle ist [[ArrayBufferByteLength]]. Das zweite hält
//! [[ArrayBufferMaxByteLength]] als Number, *undefined* bei fester Länge.
//! [[ArrayBufferDetachKey]] ist immer ~empty~: Nur Einbettungen setzen einen
//! Schlüssel (Abschnitt 25.1.7), und diese setzt keinen.
//!
//! Festlegungen:
//! - Ein Data Block lässt sich anlegen (CreateByteDataBlock, Abschnitt
//!   6.2.9.1), wenn er in den freien Teil des Speicherbudgets des Heaps
//!   passt; sonst wirft das einen RangeError, wie es die Spezifikation für
//!   einen Block verlangt, den es nicht geben kann.
//! - [[ArrayBufferMaxByteLength]] ist möglich (AllocateArrayBuffer, Schritt
//!   9.a), wenn ein Block dieser Größe in das ganze Speicherbudget passt.
//!   Ein veränderlicher ArrayBuffer reserviert nichts im Voraus: `resize`
//!   legt einen neuen Block an und kopiert, was die Anmerkung dort erlaubt.
//! - HostResizeArrayBuffer (Abschnitt 25.1.3.8) gibt immer ~unhandled~.
//! - [[LittleEndian]] des Agents ist *true*; so lesen und schreiben die
//!   typisierten Arrays ihre Elemente.
//! - Ein NaN wird als das NaN geschrieben, das auch Rust für `f64::NAN` und
//!   `f32::NAN` nimmt, bei binary16 als 0x7E00 (NumericToRawBytes lässt die
//!   Wahl).
//! - Anlegen, Nullen und Kopieren von Bytes bezahlen aus der Arbeitsquote
//!   des Agents, [`BYTES_PER_STEP`] Bytes je Schritt.

#![forbid(unsafe_code)]

use crate::agent::{Agent, Completion, ErrorKind};
use crate::bigint::{self, BigInt};
use crate::heap::{Heap, Local, ObjectClass, Scope};
use crate::number;
use crate::object::{INLINE, get_prototype_from_constructor};
use crate::realm::{Intrinsic, RealmId};
use crate::value::{Kind, Ref, Value};

/// Das Feld mit [[ArrayBufferData]].
const DATA: usize = 0;
/// Das Feld mit [[ArrayBufferMaxByteLength]].
const MAX_BYTE_LENGTH: usize = 1;

/// So viele Bytes kostet ein Arbeitsschritt beim Anlegen, Nullen und
/// Kopieren.
///
/// Gewählt und danach einmal gemessen (Release, ein Kern eines Xeon mit
/// 2,3 GHz, 20.11.2026), mit Schleifen, die eine Quote von 20.000.000
/// Schritten aufbrauchen: Ein Schritt des Interpreters dauerte 15 ns in
/// einer leeren Schleife und 83 ns mit einer Addition. Ein so bezahlter
/// Schritt dauerte beim Anlegen von ArrayBuffern mit 64 KiB 61 ns, bei
/// `resize` zwischen 0 und 1 MiB 50 ns, bei `transfer()` von 1 MiB 117 ns
/// und bei `slice` von 1 MiB und von 4 KiB 134 und 107 ns. Das liegt im
/// Bereich der Arbeit, die BigInts bezahlen (siehe [`crate::bigint`]).
const BYTES_PER_STEP: u64 = 256;

/// 2⁵³ − 1, die Grenze von CreateByteDataBlock (Schritt 1).
const MAX_SIZE: u64 = (1 << 53) - 1;

/// Die Arten der Elemente (Abschnitt 23.2, Tabelle der
/// TypedArray-Konstruktoren), in der Reihenfolge der Tabelle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ElementType {
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    BigInt64,
    BigUint64,
    Float16,
    Float32,
    Float64,
}

impl ElementType {
    /// Alle Arten in der Reihenfolge der Tabelle.
    pub(crate) const ALL: [ElementType; 12] = [
        ElementType::Int8,
        ElementType::Uint8,
        ElementType::Uint8Clamped,
        ElementType::Int16,
        ElementType::Uint16,
        ElementType::Int32,
        ElementType::Uint32,
        ElementType::BigInt64,
        ElementType::BigUint64,
        ElementType::Float16,
        ElementType::Float32,
        ElementType::Float64,
    ];

    /// Die Zeile in der Tabelle, von 0 an.
    pub(crate) fn index(self) -> u8 {
        match self {
            ElementType::Int8 => 0,
            ElementType::Uint8 => 1,
            ElementType::Uint8Clamped => 2,
            ElementType::Int16 => 3,
            ElementType::Uint16 => 4,
            ElementType::Int32 => 5,
            ElementType::Uint32 => 6,
            ElementType::BigInt64 => 7,
            ElementType::BigUint64 => 8,
            ElementType::Float16 => 9,
            ElementType::Float32 => 10,
            ElementType::Float64 => 11,
        }
    }

    /// Die Art in der Zeile `index` der Tabelle.
    pub(crate) fn from_index(index: u8) -> Option<ElementType> {
        ElementType::ALL.get(usize::from(index)).copied()
    }

    /// Der Constructor Name aus der Tabelle, zugleich [[TypedArrayName]].
    pub(crate) fn constructor_name(self) -> &'static str {
        match self {
            ElementType::Int8 => "Int8Array",
            ElementType::Uint8 => "Uint8Array",
            ElementType::Uint8Clamped => "Uint8ClampedArray",
            ElementType::Int16 => "Int16Array",
            ElementType::Uint16 => "Uint16Array",
            ElementType::Int32 => "Int32Array",
            ElementType::Uint32 => "Uint32Array",
            ElementType::BigInt64 => "BigInt64Array",
            ElementType::BigUint64 => "BigUint64Array",
            ElementType::Float16 => "Float16Array",
            ElementType::Float32 => "Float32Array",
            ElementType::Float64 => "Float64Array",
        }
    }

    /// Element Size: die Zahl der Bytes eines Elements.
    pub(crate) fn size(self) -> usize {
        match self {
            ElementType::Int8 | ElementType::Uint8 | ElementType::Uint8Clamped => 1,
            ElementType::Int16 | ElementType::Uint16 | ElementType::Float16 => 2,
            ElementType::Int32 | ElementType::Uint32 | ElementType::Float32 => 4,
            ElementType::BigInt64 | ElementType::BigUint64 | ElementType::Float64 => 8,
        }
    }

    /// IsBigIntElementType (Abschnitt 25.1.3.12).
    pub(crate) fn is_bigint(self) -> bool {
        matches!(self, ElementType::BigInt64 | ElementType::BigUint64)
    }
}

/// Der Wert eines Elements: eine Number oder ein BigInt, das in 64 Bit
/// passt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Numeric {
    Number(f64),
    BigInt64(i64),
    BigUint64(u64),
}

/// Höchstens acht Bytes eines Elements, die ersten [`ElementType::size`]
/// gelten.
pub(crate) type RawBytes = [u8; 8];

/// NumericToRawBytes (Abschnitt 25.1.3.17) für eine Art mit Numbers. Für
/// die Arten mit BigInts gilt [`bigint_to_raw_bytes`].
pub(crate) fn number_to_raw_bytes(kind: ElementType, value: f64, little: bool) -> RawBytes {
    let mut bytes = [0; 8];
    let size = kind.size();
    match kind {
        ElementType::Float16 => put(&mut bytes, &f16_bits(value).to_le_bytes()),
        ElementType::Float32 => {
            // `as` rundet zur nächsten, bei Gleichstand zur geraden.
            let single = if value.is_nan() {
                f32::NAN
            } else {
                value as f32
            };
            put(&mut bytes, &single.to_le_bytes());
        }
        ElementType::Float64 => {
            let double = if value.is_nan() { f64::NAN } else { value };
            put(&mut bytes, &double.to_le_bytes());
        }
        ElementType::Uint8Clamped => put(&mut bytes, &[to_uint8_clamp(value)]),
        // ToInt8 bis ToUint32 (Abschnitte 7.1.8 bis 7.1.13): Der Rest
        // modulo 2³² hat dieselben unteren Bytes wie der modulo 2⁸ oder
        // 2¹⁶, das Vorzeichen ergibt sich aus dem Zweierkomplement.
        _ => put(&mut bytes, &number::to_uint32(value).to_le_bytes()),
    }
    if !little && let Some(used) = bytes.get_mut(..size) {
        used.reverse();
    }
    bytes
}

/// NumericToRawBytes für ~bigint64~ und ~biguint64~: ToBigInt64 und
/// ToBigUint64 (Abschnitte 7.1.17 und 7.1.18) haben dieselben 64 Bit, das
/// Zweierkomplement des Werts modulo 2⁶⁴. `negative` und `low` sind
/// Vorzeichen und niedrigstes Wort des Betrags.
pub(crate) fn bigint_to_raw_bytes(negative: bool, low: u64, little: bool) -> RawBytes {
    let bits = if negative { low.wrapping_neg() } else { low };
    if little {
        bits.to_le_bytes()
    } else {
        bits.to_be_bytes()
    }
}

/// NumericToRawBytes für einen Wert, wie ihn [`raw_bytes_to_numeric`]
/// liefert, etwa beim Umwandeln zwischen zwei Arten. Eine Art mit Numbers
/// und ein BigInt oder umgekehrt ergeben 0; die Aufrufer haben die
/// [[ContentType]] beider Seiten verglichen.
pub(crate) fn numeric_to_raw_bytes(kind: ElementType, value: Numeric, little: bool) -> RawBytes {
    match (value, kind.is_bigint()) {
        (Numeric::Number(number), false) => number_to_raw_bytes(kind, number, little),
        // Die 64 Bit des Zweierkomplements sind dieselben.
        (Numeric::BigInt64(bits), true) => {
            bigint_to_raw_bytes(false, u64::from_le_bytes(bits.to_le_bytes()), little)
        }
        (Numeric::BigUint64(bits), true) => bigint_to_raw_bytes(false, bits, little),
        _ => [0; 8],
    }
}

/// RawBytesToNumeric (Abschnitt 25.1.3.14). `bytes` hat mindestens
/// [`ElementType::size`] Bytes; fehlende gelten als 0.
pub(crate) fn raw_bytes_to_numeric(kind: ElementType, bytes: &[u8], little: bool) -> Numeric {
    let size = kind.size();
    let mut raw = [0u8; 8];
    for (index, slot) in raw.iter_mut().take(size).enumerate() {
        let source = if little { index } else { size - 1 - index };
        *slot = bytes.get(source).copied().unwrap_or(0);
    }
    let word = u64::from_le_bytes(raw);
    let [b0, b1, b2, b3, ..] = raw;
    match kind {
        ElementType::Int8 => Numeric::Number(f64::from(i8::from_le_bytes([b0]))),
        ElementType::Uint8 | ElementType::Uint8Clamped => Numeric::Number(f64::from(b0)),
        ElementType::Int16 => Numeric::Number(f64::from(i16::from_le_bytes([b0, b1]))),
        ElementType::Uint16 => Numeric::Number(f64::from(u16::from_le_bytes([b0, b1]))),
        ElementType::Int32 => Numeric::Number(f64::from(i32::from_le_bytes([b0, b1, b2, b3]))),
        ElementType::Uint32 => Numeric::Number(f64::from(u32::from_le_bytes([b0, b1, b2, b3]))),
        ElementType::BigInt64 => Numeric::BigInt64(i64::from_le_bytes(raw)),
        ElementType::BigUint64 => Numeric::BigUint64(word),
        ElementType::Float16 => Numeric::Number(f16_value(u16::from_le_bytes([b0, b1]))),
        ElementType::Float32 => Numeric::Number(f64::from(f32::from_le_bytes([b0, b1, b2, b3]))),
        ElementType::Float64 => Numeric::Number(f64::from_le_bytes(raw)),
    }
}

/// ToUint8Clamp (Abschnitt 7.1.12): auf 0 bis 255 begrenzt, zur nächsten
/// ganzen Zahl gerundet, bei Gleichstand zur geraden.
pub(crate) fn to_uint8_clamp(value: f64) -> u8 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= 255.0 {
        return 255;
    }
    // Zwischen 0 und 255, also verlustfrei.
    let floor = value.floor();
    let lower = floor as u8;
    let rest = value - floor;
    if rest > 0.5 || (rest == 0.5 && lower % 2 == 1) {
        lower + 1
    } else {
        lower
    }
}

/// Schreibt `source` an den Anfang von `bytes`.
fn put(bytes: &mut RawBytes, source: &[u8]) {
    for (slot, &byte) in bytes.iter_mut().zip(source) {
        *slot = byte;
    }
}

/// Die Bits einer Zahl im Format binary16 nach IEEE 754-2019, gerundet zur
/// nächsten, bei Gleichstand zur geraden (roundTiesToEven). Gerundet wird
/// einmal, direkt aus binary64; ein Umweg über binary32 könnte doppelt
/// runden.
pub(crate) fn f16_bits(value: f64) -> u16 {
    let bits = value.to_bits();
    let sign = u16::from(bits >> 63 == 1) << 15;
    if value.is_nan() {
        return 0x7E00;
    }
    let magnitude = value.abs();
    if magnitude.is_infinite() {
        return sign | 0x7C00;
    }
    let exponent = i64::try_from((bits >> 52) & 0x7FF).unwrap_or(0);
    let fraction = bits & ((1 << 52) - 1);
    if exponent == 0 {
        // Subnormale Zahlen von binary64 liegen weit unter der kleinsten
        // von binary16 (2⁻²⁴) und werden ±0.
        return sign;
    }
    let unbiased = exponent - 1023;
    let significand = fraction | (1 << 52);
    let result = if unbiased >= -14 {
        // Normal: Exponent und die oberen 10 Bits des Bruchs, dann runden.
        // Ein Übertrag beim Runden wandert in den Exponenten; ab 0x7C00
        // ist das Ergebnis ∞.
        let half_exponent = u64::try_from(unbiased + 15).unwrap_or(u64::MAX);
        if half_exponent >= 31 {
            return sign | 0x7C00;
        }
        let truncated = (half_exponent << 10) | (fraction >> 42);
        round_ties_to_even(truncated, fraction & ((1 << 42) - 1), 42)
    } else {
        // Subnormal in binary16: ein Vielfaches von 2⁻²⁴. Der Signifikand
        // steht für significand · 2^(unbiased − 52), also ist das
        // Vielfache significand · 2^(unbiased + 24 − 52).
        let shift = u32::try_from(28 - unbiased).unwrap_or(u32::MAX);
        if shift >= 64 {
            return sign;
        }
        let truncated = significand >> shift;
        round_ties_to_even(truncated, significand & ((1 << shift) - 1), shift)
    };
    let result = result.min(0x7C00);
    sign | u16::try_from(result).unwrap_or(0x7C00)
}

/// Rundet `truncated` mit dem abgeschnittenen Rest `rest` aus `bits` Bits.
fn round_ties_to_even(truncated: u64, rest: u64, bits: u32) -> u64 {
    let half = 1u64 << (bits - 1);
    if rest > half || (rest == half && truncated & 1 == 1) {
        truncated + 1
    } else {
        truncated
    }
}

/// Der Wert von Bits im Format binary16, exakt als binary64.
pub(crate) fn f16_value(bits: u16) -> f64 {
    let negative = bits & 0x8000 != 0;
    let exponent = i32::from((bits >> 10) & 0x1F);
    let fraction = f64::from(bits & 0x3FF);
    let magnitude = match exponent {
        0 => fraction * 2f64.powi(-24),
        31 if fraction == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1024.0 + fraction) * 2f64.powi(exponent - 25),
    };
    if negative { -magnitude } else { magnitude }
}

// ArrayBuffer

/// Das Objekt hinter einem Handle, wenn es ein ArrayBuffer ist.
fn buffer_ref<'h>(agent: &'h Agent, buffer: Local<'_>) -> Option<Ref<'h>> {
    let heap = agent.heap();
    match heap.get(buffer).kind() {
        Kind::Object(object) if heap.object_class(object) == Some(ObjectClass::ArrayBuffer) => {
            Some(object)
        }
        _ => None,
    }
}

/// Die Länge des Blocks des ArrayBuffer `buffer`, `None`, wenn er
/// abgelöst oder kein ArrayBuffer ist. Für Aufrufer, die nur den Heap
/// haben.
pub(crate) fn block_length(heap: &Heap, buffer: Ref<'_>) -> Option<usize> {
    match heap.array_buffer_field(buffer, DATA)?.kind() {
        Kind::Internal(block) => heap.byte_len(block),
        _ => None,
    }
}

/// IsFixedLengthArrayBuffer (Abschnitt 25.1.3.9) für Aufrufer, die nur den
/// Heap haben.
pub(crate) fn is_fixed_length(heap: &Heap, buffer: Ref<'_>) -> bool {
    heap.array_buffer_field(buffer, MAX_BYTE_LENGTH)
        .is_none_or(|max| max.is_undefined())
}

/// Liest `bytes.len()` Bytes ab `index` aus dem Block des ArrayBuffer
/// `buffer`; `false`, wenn er abgelöst ist.
pub(crate) fn read_block(heap: &Heap, buffer: Ref<'_>, index: usize, bytes: &mut [u8]) -> bool {
    match heap
        .array_buffer_field(buffer, DATA)
        .map(|value| value.kind())
    {
        Some(Kind::Internal(block)) => {
            heap.read_bytes(block, index, bytes);
            true
        }
        _ => false,
    }
}

/// Schreibt `bytes` ab `index` in den Block des ArrayBuffer `buffer`, wenn
/// er nicht abgelöst ist.
pub(crate) fn write_block(heap: &Heap, buffer: Ref<'_>, index: usize, bytes: &[u8]) {
    if let Some(Kind::Internal(block)) = heap
        .array_buffer_field(buffer, DATA)
        .map(|value| value.kind())
    {
        heap.write_bytes(block, index, bytes);
    }
}

/// Ob `value` ein ArrayBuffer ist (RequireInternalSlot mit
/// [[ArrayBufferData]], Abschnitt 10.1.15). SharedArrayBuffer gibt es
/// noch nicht.
pub(crate) fn is_array_buffer(agent: &Agent, value: Local<'_>) -> bool {
    buffer_ref(agent, value).is_some()
}

/// Die Zelle aus Bytes, `None` bei einem abgelösten ArrayBuffer oder
/// einem anderen Wert.
fn data_block<'h>(agent: &'h Agent, buffer: Local<'_>) -> Option<Ref<'h>> {
    let object = buffer_ref(agent, buffer)?;
    match agent.heap().array_buffer_field(object, DATA)?.kind() {
        Kind::Internal(block) => Some(block),
        _ => None,
    }
}

/// IsDetachedBuffer (Abschnitt 25.1.3.4).
pub(crate) fn is_detached(agent: &Agent, buffer: Local<'_>) -> bool {
    data_block(agent, buffer).is_none()
}

/// [[ArrayBufferByteLength]], 0 bei einem abgelösten ArrayBuffer.
pub(crate) fn byte_length(agent: &Agent, buffer: Local<'_>) -> usize {
    data_block(agent, buffer)
        .and_then(|block| agent.heap().byte_len(block))
        .unwrap_or(0)
}

/// [[ArrayBufferMaxByteLength]], `None` bei fester Länge
/// (IsFixedLengthArrayBuffer, Abschnitt 25.1.3.9).
pub(crate) fn max_byte_length(agent: &Agent, buffer: Local<'_>) -> Option<u64> {
    let object = buffer_ref(agent, buffer)?;
    let value = agent.heap().array_buffer_field(object, MAX_BYTE_LENGTH)?;
    // Das Feld hält nur ganze Zahlen bis zur Größe des Heaps.
    value.as_number().map(|length| length as u64)
}

/// Bezahlt `bytes` Bytes Arbeit aus der Arbeitsquote, wenn eine läuft,
/// [`BYTES_PER_STEP`] je Schritt. Den Rest unter einem Schritt bezahlt der
/// Aufruf, der die Arbeit auslöst.
pub(crate) fn pay(agent: &Agent, bytes: usize) -> Completion<()> {
    if !agent.has_execution_budget() {
        return Ok(());
    }
    let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
    agent.spend_execution_steps(bytes / BYTES_PER_STEP)
}

/// CreateByteDataBlock (Abschnitt 6.2.9.1): ein Block aus `size` Bytes,
/// alle 0. Ein RangeError, wenn es ihn nicht geben kann.
pub(crate) fn create_byte_data_block<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    size: u64,
) -> Completion<Local<'s>> {
    if size > MAX_SIZE || !agent.heap().bytes_fit(size) {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let Ok(size) = usize::try_from(size) else {
        return Err(agent.throw_error(ErrorKind::RangeError));
    };
    pay(agent, size)?;
    match agent.heap_mut().alloc_bytes(scope, size) {
        Ok(block) => Ok(block),
        Err(_) => Err(agent.throw_error(ErrorKind::RangeError)),
    }
}

/// AllocateArrayBuffer (Abschnitt 25.1.3.1) mit dem Konstruktor
/// `constructor`; `max_byte_length` ist `Some` für einen veränderlichen
/// ArrayBuffer. `current` ist der aktuelle Realm für
/// GetPrototypeFromConstructor.
pub(crate) fn allocate_array_buffer<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    constructor: Local<'_>,
    byte_length: u64,
    max_byte_length: Option<u64>,
    current: RealmId,
) -> Completion<Local<'s>> {
    if max_byte_length.is_some_and(|max| byte_length > max) {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let prototype = get_prototype_from_constructor(
        agent,
        scope,
        constructor,
        Intrinsic::ArrayBufferPrototype,
        current,
    )?;
    let block = create_byte_data_block(agent, scope, byte_length)?;
    if max_byte_length.is_some_and(|max| !agent.heap().bytes_fit(max)) {
        return Err(agent.throw_error(ErrorKind::RangeError));
    }
    let shape = agent.empty_shape(scope);
    let object = agent.heap_mut().alloc_object(
        scope,
        ObjectClass::ArrayBuffer,
        INLINE,
        shape,
        Some(prototype),
    )?;
    let heap = agent.heap();
    if let Kind::Object(reference) = heap.get(object).kind() {
        heap.set_array_buffer_field(reference, DATA, heap.get(block));
        // Bis 2⁵³ − 1 ist jede ganze Zahl als Number exakt.
        let max = max_byte_length.map_or(Value::UNDEFINED, |max| Value::number(max as f64));
        heap.set_array_buffer_field(reference, MAX_BYTE_LENGTH, max);
    }
    Ok(object)
}

/// Ersetzt [[ArrayBufferData]] durch `block`, etwa nach `resize`.
pub(crate) fn set_data_block(agent: &Agent, buffer: Local<'_>, block: Local<'_>) {
    let heap = agent.heap();
    if let Some(object) = buffer_ref(agent, buffer) {
        heap.set_array_buffer_field(object, DATA, heap.get(block));
    }
}

/// DetachArrayBuffer (Abschnitt 25.1.3.5) ohne Schlüssel. Weil
/// [[ArrayBufferDetachKey]] immer ~empty~ ist, gelingt das immer.
pub(crate) fn detach_array_buffer(agent: &Agent, buffer: Local<'_>) {
    if let Some(object) = buffer_ref(agent, buffer) {
        agent
            .heap()
            .set_array_buffer_field(object, DATA, Value::NULL);
    }
}

/// CopyDataBlockBytes (Abschnitt 6.2.9.3) zwischen den Blöcken zweier
/// nicht abgelöster ArrayBuffer, bezahlt aus der Arbeitsquote. Liegt ein
/// Bereich nicht im Block, wird nichts kopiert; die Aufrufer prüfen die
/// Grenzen vorher, wie es die Zusicherungen dort verlangen.
pub(crate) fn copy_block_bytes(
    agent: &Agent,
    to: Local<'_>,
    to_index: usize,
    from: Local<'_>,
    from_index: usize,
    count: usize,
) -> Completion<()> {
    pay(agent, count)?;
    let heap = agent.heap();
    if let (Kind::Internal(to), Kind::Internal(from)) = (heap.get(to).kind(), heap.get(from).kind())
    {
        heap.copy_bytes(to, to_index, from, from_index, count);
    }
    Ok(())
}

/// Der Block eines nicht abgelösten ArrayBuffer als Handle.
pub(crate) fn data_block_local<'s>(
    agent: &Agent,
    scope: &'s Scope,
    buffer: Local<'_>,
) -> Option<Local<'s>> {
    let object = buffer_ref(agent, buffer)?;
    let heap = agent.heap();
    let value = heap.array_buffer_field(object, DATA)?;
    matches!(value.kind(), Kind::Internal(_)).then(|| heap.local(scope, value))
}

/// GetValueFromBuffer (Abschnitt 25.1.3.16) für einen nicht geteilten
/// ArrayBuffer. Die Aufrufer haben geprüft, dass er nicht abgelöst ist und
/// die Bytes darin liegen; sonst ist das Ergebnis 0.
pub(crate) fn get_value_from_buffer<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    buffer: Local<'_>,
    index: usize,
    kind: ElementType,
    little: bool,
) -> Completion<Local<'s>> {
    let mut raw = [0u8; 8];
    if let (Some(block), Some(slot)) = (data_block(agent, buffer), raw.get_mut(..kind.size())) {
        agent.heap().read_bytes(block, index, slot);
    }
    numeric_value(agent, scope, raw_bytes_to_numeric(kind, &raw, little))
}

/// Eine Number oder ein BigInt im Heap.
pub(crate) fn numeric_value<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    value: Numeric,
) -> Completion<Local<'s>> {
    match value {
        Numeric::Number(number) => Ok(agent.heap().local(scope, Value::number(number))),
        Numeric::BigInt64(value) => bigint::allocate(agent, scope, &BigInt::from_i64(value)),
        Numeric::BigUint64(value) => bigint::allocate(agent, scope, &BigInt::from_u64(value)),
    }
}

/// Die Bytes von NumericToRawBytes für einen Wert, der für `kind` schon
/// eine Number oder ein BigInt ist (ToNumber oder ToBigInt ist geschehen).
pub(crate) fn raw_bytes_of(
    agent: &Agent,
    value: Local<'_>,
    kind: ElementType,
    little: bool,
) -> RawBytes {
    let heap = agent.heap();
    raw_bytes_of_value(heap, heap.get(value), kind, little)
}

/// [`raw_bytes_of`] für einen Wert im Heap.
pub(crate) fn raw_bytes_of_value(
    heap: &Heap,
    value: Value<'_>,
    kind: ElementType,
    little: bool,
) -> RawBytes {
    match value.kind() {
        Kind::BigInt(cell) => {
            let (negative, low) = heap.bigint_low_word(cell).unwrap_or((false, 0));
            bigint_to_raw_bytes(negative, low, little)
        }
        Kind::Number(number) => number_to_raw_bytes(kind, number, little),
        _ => [0; 8],
    }
}

/// SetValueInBuffer (Abschnitt 25.1.3.18) für einen nicht geteilten
/// ArrayBuffer. Die Aufrufer haben geprüft, dass er nicht abgelöst ist und
/// die Bytes darin liegen; sonst geschieht nichts.
pub(crate) fn set_value_in_buffer(
    agent: &Agent,
    buffer: Local<'_>,
    index: usize,
    kind: ElementType,
    value: Local<'_>,
    little: bool,
) {
    let raw = raw_bytes_of(agent, value, kind, little);
    if let (Some(block), Some(bytes)) = (data_block(agent, buffer), raw.get(..kind.size())) {
        agent.heap().write_bytes(block, index, bytes);
    }
}

#[cfg(test)]
mod tests;
