//! Werte der Engine (`docs/architektur.md`, Abschnitt 4.9).
//!
//! Ein [`Value`] ist ein 64-Bit-Wort mit NaN-Boxing. Jede Zahl steht als
//! Double unmittelbar im Wort, alle übrigen Arten tragen ein Kennzeichen im
//! Bereich der negativen stillen NaNs und höchstens 32 Bit Nutzlast. Damit
//! kein Double in diesen Bereich fällt, bekommt jedes NaN beim Einpacken ein
//! festes Bitmuster. Das ist erlaubt: Für ECMAScript-Code sind alle NaN-Werte
//! ununterscheidbar (Abschnitt 6.1.6.1), und NumericToRawBytes darf für NaN
//! jede Kodierung schreiben (Abschnitt 25.1.3.17).
//!
//! Ganze Zahlen im Bereich von `i32` stehen mit eigenem Kennzeichen im Wort,
//! damit der Interpreter ohne Gleitkomma rechnen kann. [`Value::number`]
//! wählt für sie immer diese Form, -0 bleibt ein Double. Jede Zahl hat damit
//! genau eine Darstellung.
//!
//! Verweise in den Heap sind 32-Bit-Abstände ([`Ref`]). Ein Wert mit Verweis
//! leiht sich den [`Heap`], aus dem er gelesen wurde. Solange er lebt, kann
//! niemand Speicher anfordern, also lässt keine Sammlung den Verweis
//! veralten. Über eine Anforderung hinweg hält Rust-Code Werte in einem
//! [`Local`](crate::heap::Local).

#![forbid(unsafe_code)]

use std::fmt;
use std::marker::PhantomData;

use crate::heap::Heap;

/// Die Bits 63 bis 51 aller Werte mit Kennzeichen: Vorzeichen, Exponent und
/// das Bit für stille NaNs.
const BOXED: u64 = 0xFFF8_0000_0000_0000;
/// Das Kennzeichen steht in den Bits 50 bis 48, die Nutzlast darunter.
const TAG_SHIFT: u32 = 48;
const TAG_MASK: u64 = 0x7;
const PAYLOAD_MASK: u64 = 0xFFFF_FFFF;
/// Das feste Bitmuster jedes NaN.
const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;

const TAG_INT32: u64 = 0;
const TAG_SPECIAL: u64 = 1;
const TAG_STRING: u64 = 2;
const TAG_SYMBOL: u64 = 3;
const TAG_BIGINT: u64 = 4;
const TAG_OBJECT: u64 = 5;
const TAG_INTERNAL: u64 = 6;

const UNDEFINED: u64 = 0;
const NULL: u64 = 1;
const FALSE: u64 = 2;
const TRUE: u64 = 3;
const HOLE: u64 = 4;

const fn boxed(tag: u64, payload: u64) -> u64 {
    BOXED | (tag << TAG_SHIFT) | (payload & PAYLOAD_MASK)
}

/// Ein Wert. Werte ohne Verweis haben die Lebensdauer `'static`, Werte mit
/// Verweis leihen sich den Heap.
#[derive(Clone, Copy)]
pub struct Value<'h> {
    bits: u64,
    heap: PhantomData<&'h Heap>,
}

/// Ein Verweis in den Heap, solange der Heap geliehen ist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ref<'h> {
    offset: u32,
    heap: PhantomData<&'h Heap>,
}

impl Ref<'_> {
    pub(crate) fn new(offset: u32) -> Self {
        Ref {
            offset,
            heap: PhantomData,
        }
    }

    pub(crate) fn offset(self) -> u32 {
        self.offset
    }
}

/// Worauf ein Verweis zeigt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefKind {
    String,
    Symbol,
    BigInt,
    Object,
    Internal,
}

impl RefKind {
    fn tag(self) -> u64 {
        match self {
            RefKind::String => TAG_STRING,
            RefKind::Symbol => TAG_SYMBOL,
            RefKind::BigInt => TAG_BIGINT,
            RefKind::Object => TAG_OBJECT,
            RefKind::Internal => TAG_INTERNAL,
        }
    }
}

/// Die Art eines Werts (Abschnitt 6.1), dazu zwei Arten, die Skripte nie
/// sehen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind<'h> {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    String(Ref<'h>),
    Symbol(Ref<'h>),
    BigInt(Ref<'h>),
    Object(Ref<'h>),
    /// Die Marke für Bindungen vor ihrer Initialisierung (TDZ) und für
    /// Lücken in Arrays.
    Hole,
    /// Ein Verweis auf eine interne Zelle, etwa Felder aus
    /// [`Heap::alloc_slots`].
    Internal(Ref<'h>),
}

impl Value<'static> {
    pub const UNDEFINED: Value<'static> = Value::special(UNDEFINED);
    pub const NULL: Value<'static> = Value::special(NULL);
    pub const TRUE: Value<'static> = Value::special(TRUE);
    pub const FALSE: Value<'static> = Value::special(FALSE);
    /// Siehe [`Kind::Hole`].
    pub const HOLE: Value<'static> = Value::special(HOLE);

    const fn special(payload: u64) -> Value<'static> {
        Value {
            bits: boxed(TAG_SPECIAL, payload),
            heap: PhantomData,
        }
    }

    pub fn boolean(value: bool) -> Value<'static> {
        if value { Value::TRUE } else { Value::FALSE }
    }

    pub fn int32(value: i32) -> Value<'static> {
        let payload = u32::from_ne_bytes(value.to_ne_bytes());
        Value {
            bits: boxed(TAG_INT32, u64::from(payload)),
            heap: PhantomData,
        }
    }

    /// Eine Zahl. Ganze Zahlen im Bereich von `i32` außer -0 werden Int32,
    /// jedes NaN bekommt das feste Bitmuster.
    pub fn number(value: f64) -> Value<'static> {
        if value.is_nan() {
            return Value {
                bits: CANONICAL_NAN,
                heap: PhantomData,
            };
        }
        let negative_zero = value.to_bits() == (-0.0f64).to_bits();
        if value.fract() == 0.0
            && (-2_147_483_648.0..=2_147_483_647.0).contains(&value)
            && !negative_zero
        {
            // Schneidet nicht ab: Bereich und Ganzzahligkeit sind geprüft.
            return Value::int32(value as i32);
        }
        Value {
            bits: value.to_bits(),
            heap: PhantomData,
        }
    }
}

impl<'h> Value<'h> {
    /// Ein Wert aus Bits, die aus einem anderen Wert stammen.
    pub(crate) const fn from_bits(bits: u64) -> Value<'h> {
        Value {
            bits,
            heap: PhantomData,
        }
    }

    pub(crate) const fn bits(self) -> u64 {
        self.bits
    }

    pub(crate) fn reference(kind: RefKind, offset: u32) -> Value<'h> {
        Value::from_bits(boxed(kind.tag(), u64::from(offset)))
    }

    /// Ein Verweis auf ein Objekt.
    pub(crate) fn object(object: Ref<'h>) -> Value<'h> {
        Value::reference(RefKind::Object, object.offset())
    }

    /// Ein Verweis auf eine interne Zelle.
    pub(crate) fn internal(cell: Ref<'h>) -> Value<'h> {
        Value::reference(RefKind::Internal, cell.offset())
    }

    /// Art und Abstand, wenn der Wert ein Verweis ist.
    pub(crate) fn reference_parts(self) -> Option<(RefKind, u32)> {
        let kind = match self.tag()? {
            TAG_STRING => RefKind::String,
            TAG_SYMBOL => RefKind::Symbol,
            TAG_BIGINT => RefKind::BigInt,
            TAG_OBJECT => RefKind::Object,
            TAG_INTERNAL => RefKind::Internal,
            _ => return None,
        };
        Some((kind, self.payload()))
    }

    /// Das Kennzeichen, `None` für ein Double.
    fn tag(self) -> Option<u64> {
        (self.bits & BOXED == BOXED).then_some((self.bits >> TAG_SHIFT) & TAG_MASK)
    }

    fn payload(self) -> u32 {
        // Die Maske lässt nur 32 Bit übrig.
        u32::try_from(self.bits & PAYLOAD_MASK).unwrap_or(0)
    }

    pub fn kind(self) -> Kind<'h> {
        match self.tag() {
            None => Kind::Number(f64::from_bits(self.bits)),
            Some(TAG_INT32) => Kind::Number(f64::from(self.int32_payload())),
            Some(TAG_SPECIAL) => match u64::from(self.payload()) {
                NULL => Kind::Null,
                FALSE => Kind::Boolean(false),
                TRUE => Kind::Boolean(true),
                HOLE => Kind::Hole,
                // Andere Muster entstehen nicht, `from_bits` ist intern.
                _ => Kind::Undefined,
            },
            Some(TAG_STRING) => Kind::String(Ref::new(self.payload())),
            Some(TAG_SYMBOL) => Kind::Symbol(Ref::new(self.payload())),
            Some(TAG_BIGINT) => Kind::BigInt(Ref::new(self.payload())),
            Some(TAG_OBJECT) => Kind::Object(Ref::new(self.payload())),
            Some(TAG_INTERNAL) => Kind::Internal(Ref::new(self.payload())),
            Some(_) => Kind::Undefined,
        }
    }

    fn int32_payload(self) -> i32 {
        i32::from_ne_bytes(self.payload().to_ne_bytes())
    }

    /// Die Zahl, wenn der Wert eine ist.
    pub fn as_number(self) -> Option<f64> {
        match self.tag() {
            None => Some(f64::from_bits(self.bits)),
            Some(TAG_INT32) => Some(f64::from(self.int32_payload())),
            Some(_) => None,
        }
    }

    /// Die Zahl, wenn der Wert eine ganze Zahl im Bereich von `i32` und
    /// nicht -0 ist.
    pub fn as_int32(self) -> Option<i32> {
        (self.tag() == Some(TAG_INT32)).then(|| self.int32_payload())
    }

    pub fn is_undefined(self) -> bool {
        self.bits == Value::UNDEFINED.bits
    }

    pub fn is_null(self) -> bool {
        self.bits == Value::NULL.bits
    }

    pub fn is_hole(self) -> bool {
        self.bits == Value::HOLE.bits
    }
}

impl fmt::Debug for Value<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Value({:?})", self.kind())
    }
}

#[cfg(test)]
mod tests;
