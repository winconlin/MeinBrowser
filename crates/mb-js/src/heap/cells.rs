//! Zellen für Symbole, Gestalten, schwache Felder, Elemente und Objekte.
//! Das Modul `object` setzt daraus die Objekte nach Abschnitt 10.1
//! zusammen; hier steht nur, wie die Zellen im Heap liegen.

use super::{
    Heap, KIND_ARGUMENTS, KIND_ARRAY, KIND_FUNCTION, KIND_OBJECT, KIND_SHAPE, KIND_SLOTS,
    KIND_SPARSE, KIND_SYMBOL, KIND_WEAK, Local, NONE, OBJECT_REFS, OutOfMemory, Scope, cell_words,
    header, header_flags, header_kind, header_length, is_object, is_value_cell, object_fields,
    packed, packed_high, packed_low, to_usize, with_flags,
};
use crate::value::{Ref, RefKind, Value};

/// Kennzeichen einer Zeichenkette, die ein Atom ist.
const ATOM_FLAG: u32 = 1;
/// Kennzeichen eines Objekts, dessen [[Extensible]] *true* ist.
const EXTENSIBLE_FLAG: u32 = 1;

/// Die Arten der Zellen aus Werten.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellKind {
    /// Interne Felder, etwa dichte Elemente, das Zusatzfeld eines Objekts
    /// und Paare aus Getter und Setter.
    Slots,
    /// Ein Symbol, im einzigen Feld seine Beschreibung.
    Symbol,
    Shape,
    /// Felder, die ihr Ziel nicht am Leben halten.
    Weak,
    /// Dünn besetzte Elemente.
    Sparse,
}

impl CellKind {
    fn code(self) -> u64 {
        match self {
            CellKind::Slots => KIND_SLOTS,
            CellKind::Symbol => KIND_SYMBOL,
            CellKind::Shape => KIND_SHAPE,
            CellKind::Weak => KIND_WEAK,
            CellKind::Sparse => KIND_SPARSE,
        }
    }

    fn from_code(code: u64) -> Option<CellKind> {
        Some(match code {
            KIND_SLOTS => CellKind::Slots,
            KIND_SYMBOL => CellKind::Symbol,
            KIND_SHAPE => CellKind::Shape,
            KIND_WEAK => CellKind::Weak,
            KIND_SPARSE => CellKind::Sparse,
            _ => return None,
        })
    }
}

/// Die Arten von Objekten. Jede hat eine eigene Art im Kopfwort.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectClass {
    Ordinary,
    /// Ein Array (Abschnitt 10.4.2).
    Array,
    /// Ein ECMAScript-Funktionsobjekt (Abschnitt 10.2) mit
    /// [`FUNCTION_FIELDS`](super::FUNCTION_FIELDS) internen Feldern.
    Function,
    /// Ein arguments-Objekt (Abschnitt 10.4.4) mit dem internen Feld
    /// [[ParameterMap]].
    Arguments,
}

impl ObjectClass {
    fn code(self) -> u64 {
        match self {
            ObjectClass::Ordinary => KIND_OBJECT,
            ObjectClass::Array => KIND_ARRAY,
            ObjectClass::Function => KIND_FUNCTION,
            ObjectClass::Arguments => KIND_ARGUMENTS,
        }
    }

    fn from_code(code: u64) -> Option<ObjectClass> {
        match code {
            KIND_OBJECT => Some(ObjectClass::Ordinary),
            KIND_ARRAY => Some(ObjectClass::Array),
            KIND_FUNCTION => Some(ObjectClass::Function),
            KIND_ARGUMENTS => Some(ObjectClass::Arguments),
            _ => None,
        }
    }
}

/// Wort mit Gestalt (unten) und Prototyp (oben).
const SHAPE_AND_PROTOTYPE: usize = 1;
/// Wort mit Elementen (unten) und Zusatzfeld (oben).
const ELEMENTS_AND_OVERFLOW: usize = 2;

impl Heap {
    // Zellen aus Werten

    /// Legt eine Zelle aus `len` Werten an, alle *undefined*. Ein Symbol
    /// bekommt einen Verweis der Art Symbol, alle anderen der Art Internal.
    pub(crate) fn alloc_cell<'s>(
        &mut self,
        scope: &'s Scope,
        kind: CellKind,
        len: usize,
    ) -> Result<Local<'s>, OutOfMemory> {
        self.alloc_values(scope, kind.code(), len)
    }

    pub(crate) fn cell_kind(&self, cell: Ref<'_>) -> Option<CellKind> {
        CellKind::from_code(header_kind(self.word(cell.offset(), 0)?))
    }

    /// Die Zahl der Werte einer Zelle aus Werten, 0 für andere Zellen.
    pub(crate) fn field_count(&self, cell: Ref<'_>) -> usize {
        self.value_header(cell)
            .map_or(0, |head| to_usize(header_length(head)))
    }

    pub(crate) fn field(&self, cell: Ref<'_>, index: usize) -> Option<Value<'_>> {
        let head = self.value_header(cell)?;
        if index >= to_usize(header_length(head)) {
            return None;
        }
        self.word(cell.offset(), index.checked_add(1)?)
            .map(Value::from_bits)
    }

    /// Schreibt einen Wert. `false`, wenn es ihn nicht gibt.
    pub(crate) fn set_field(&self, cell: Ref<'_>, index: usize, value: Value<'_>) -> bool {
        let Some(head) = self.value_header(cell) else {
            return false;
        };
        if index >= to_usize(header_length(head)) {
            return false;
        }
        index
            .checked_add(1)
            .is_some_and(|word| self.store(cell.offset(), word, value.bits()))
    }

    fn value_header(&self, cell: Ref<'_>) -> Option<u64> {
        let head = self.word(cell.offset(), 0)?;
        is_value_cell(header_kind(head)).then_some(head)
    }

    // Atome

    pub(crate) fn is_atom(&self, string: Ref<'_>) -> bool {
        self.string_header(string)
            .is_some_and(|head| header_flags(head) & ATOM_FLAG != 0)
    }

    /// Kennzeichnet eine Zeichenkette als Atom. Zeichenketten sind
    /// unveränderlich, also darf jede eines werden.
    pub(crate) fn mark_atom(&self, string: Ref<'_>) {
        if let Some(head) = self.string_header(string) {
            let head = with_flags(head, header_flags(head) | ATOM_FLAG);
            self.write(string.offset(), 0, head);
        }
    }

    /// Ob zwei Zeichenketten dieselben Codeeinheiten an denselben Stellen
    /// haben.
    pub(crate) fn strings_equal(&self, a: Ref<'_>, b: Ref<'_>) -> bool {
        if self.string_header(a).is_none() || self.string_header(b).is_none() {
            return false;
        }
        let length = self.string_len(a);
        length == self.string_len(b)
            && (0..length).all(|index| self.string_unit(a, index) == self.string_unit(b, index))
    }

    // Objekte

    /// Legt ein erweiterbares Objekt ohne Elemente an, mit `inline`
    /// eingebetteten Plätzen für Eigenschaften, alle *undefined*, und den
    /// internen Feldern seiner Art, ebenfalls *undefined*.
    pub(crate) fn alloc_object<'s>(
        &mut self,
        scope: &'s Scope,
        class: ObjectClass,
        inline: usize,
        shape: Local<'_>,
        prototype: Option<Local<'_>>,
    ) -> Result<Local<'s>, OutOfMemory> {
        let length = u32::try_from(inline).map_err(|_| OutOfMemory)?;
        let head = with_flags(header(class.code(), length), EXTENSIBLE_FLAG);
        let words = cell_words(head).ok_or(OutOfMemory)?;
        let offset = self.allocate(words)?;
        // Erst nach der Anforderung lesen, sie kann verschoben haben.
        let shape = self.target(shape);
        let prototype = prototype.map_or(NONE, |prototype| self.target(prototype));
        self.init(offset, 0, head);
        self.init(offset, SHAPE_AND_PROTOTYPE, packed(shape, prototype));
        self.init(offset, ELEMENTS_AND_OVERFLOW, packed(NONE, NONE));
        for index in 1 + OBJECT_REFS..words {
            self.init(offset, index, Value::UNDEFINED.bits());
        }
        Ok(self.local(scope, Value::reference(RefKind::Object, offset)))
    }

    /// Der Abstand, auf den ein Handle verweist, sonst [`NONE`].
    fn target(&self, local: Local<'_>) -> u32 {
        self.get(local)
            .reference_parts()
            .map_or(NONE, |(_, offset)| offset)
    }

    fn object_header(&self, object: Ref<'_>) -> Option<u64> {
        let head = self.word(object.offset(), 0)?;
        is_object(header_kind(head)).then_some(head)
    }

    pub(crate) fn object_class(&self, object: Ref<'_>) -> Option<ObjectClass> {
        ObjectClass::from_code(header_kind(self.object_header(object)?))
    }

    fn packed_ref(&self, object: Ref<'_>, index: usize, high: bool) -> Option<Ref<'_>> {
        self.object_header(object)?;
        let word = self.word(object.offset(), index)?;
        let offset = if high {
            packed_high(word)
        } else {
            packed_low(word)
        };
        (offset != NONE).then(|| Ref::new(offset))
    }

    fn set_packed_ref(
        &self,
        object: Ref<'_>,
        index: usize,
        high: bool,
        target: Option<Ref<'_>>,
    ) -> bool {
        if self.object_header(object).is_none() {
            return false;
        }
        let Some(word) = self.word(object.offset(), index) else {
            return false;
        };
        let offset = target.map_or(NONE, Ref::offset);
        let word = if high {
            packed(packed_low(word), offset)
        } else {
            packed(offset, packed_high(word))
        };
        self.store(object.offset(), index, word)
    }

    pub(crate) fn object_shape(&self, object: Ref<'_>) -> Option<Ref<'_>> {
        self.packed_ref(object, SHAPE_AND_PROTOTYPE, false)
    }

    pub(crate) fn set_object_shape(&self, object: Ref<'_>, shape: Ref<'_>) -> bool {
        self.set_packed_ref(object, SHAPE_AND_PROTOTYPE, false, Some(shape))
    }

    /// [[Prototype]], `None` für *null*.
    pub(crate) fn object_prototype(&self, object: Ref<'_>) -> Option<Ref<'_>> {
        self.packed_ref(object, SHAPE_AND_PROTOTYPE, true)
    }

    pub(crate) fn set_object_prototype(&self, object: Ref<'_>, prototype: Option<Ref<'_>>) -> bool {
        self.set_packed_ref(object, SHAPE_AND_PROTOTYPE, true, prototype)
    }

    pub(crate) fn object_elements(&self, object: Ref<'_>) -> Option<Ref<'_>> {
        self.packed_ref(object, ELEMENTS_AND_OVERFLOW, false)
    }

    pub(crate) fn set_object_elements(&self, object: Ref<'_>, elements: Option<Ref<'_>>) -> bool {
        self.set_packed_ref(object, ELEMENTS_AND_OVERFLOW, false, elements)
    }

    pub(crate) fn object_overflow(&self, object: Ref<'_>) -> Option<Ref<'_>> {
        self.packed_ref(object, ELEMENTS_AND_OVERFLOW, true)
    }

    pub(crate) fn set_object_overflow(&self, object: Ref<'_>, overflow: Option<Ref<'_>>) -> bool {
        self.set_packed_ref(object, ELEMENTS_AND_OVERFLOW, true, overflow)
    }

    /// [[Extensible]].
    pub(crate) fn object_extensible(&self, object: Ref<'_>) -> bool {
        self.object_header(object)
            .is_some_and(|head| header_flags(head) & EXTENSIBLE_FLAG != 0)
    }

    pub(crate) fn prevent_object_extensions(&self, object: Ref<'_>) {
        if let Some(head) = self.object_header(object) {
            let head = with_flags(head, header_flags(head) & !EXTENSIBLE_FLAG);
            self.write(object.offset(), 0, head);
        }
    }

    /// Die Zahl der eingebetteten Plätze.
    pub(crate) fn inline_count(&self, object: Ref<'_>) -> usize {
        self.object_header(object)
            .map_or(0, |head| to_usize(header_length(head)))
    }

    /// Das Wort, in dem der eingebettete Platz `index` liegt.
    fn inline_word(&self, object: Ref<'_>, index: usize) -> Option<usize> {
        let head = self.object_header(object)?;
        if index >= to_usize(header_length(head)) {
            return None;
        }
        index
            .checked_add(1 + OBJECT_REFS)?
            .checked_add(object_fields(header_kind(head)))
    }

    pub(crate) fn inline_slot(&self, object: Ref<'_>, index: usize) -> Option<Value<'_>> {
        let word = self.inline_word(object, index)?;
        self.word(object.offset(), word).map(Value::from_bits)
    }

    pub(crate) fn set_inline_slot(&self, object: Ref<'_>, index: usize, value: Value<'_>) -> bool {
        self.inline_word(object, index)
            .is_some_and(|word| self.store(object.offset(), word, value.bits()))
    }

    /// Das Wort, in dem das interne Feld `index` eines Objekts der Art
    /// `kind` liegt.
    fn field_word(&self, object: Ref<'_>, kind: u64, index: usize) -> Option<usize> {
        let head = self.object_header(object)?;
        (header_kind(head) == kind && index < object_fields(kind))
            .then_some(1 + OBJECT_REFS + index)
    }

    /// Ein internes Feld eines Funktionsobjekts, `None` für andere Zellen.
    pub(crate) fn function_field(&self, object: Ref<'_>, index: usize) -> Option<Value<'_>> {
        let word = self.field_word(object, KIND_FUNCTION, index)?;
        self.word(object.offset(), word).map(Value::from_bits)
    }

    /// Schreibt ein internes Feld. `false`, wenn es das Feld nicht gibt.
    pub(crate) fn set_function_field(
        &self,
        object: Ref<'_>,
        index: usize,
        value: Value<'_>,
    ) -> bool {
        self.field_word(object, KIND_FUNCTION, index)
            .is_some_and(|word| self.store(object.offset(), word, value.bits()))
    }

    /// [[ParameterMap]] eines arguments-Objekts, `None` für andere Zellen.
    pub(crate) fn arguments_map(&self, object: Ref<'_>) -> Option<Value<'_>> {
        let word = self.field_word(object, KIND_ARGUMENTS, 0)?;
        self.word(object.offset(), word).map(Value::from_bits)
    }

    /// Schreibt [[ParameterMap]]. `false` für andere Zellen.
    pub(crate) fn set_arguments_map(&self, object: Ref<'_>, map: Value<'_>) -> bool {
        self.field_word(object, KIND_ARGUMENTS, 0)
            .is_some_and(|word| self.store(object.offset(), word, map.bits()))
    }
}
