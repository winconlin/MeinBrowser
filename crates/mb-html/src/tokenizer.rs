//! Der Tokenizer (HTML Standard, Abschnitt „Tokenization“).
//!
//! Von Hand nach dem Wortlaut des Standards: Jeder Zustand ist eine Methode,
//! deren Kommentar den Abschnitt nennt, und die Fälle stehen in derselben
//! Reihenfolge wie dort. Die Eingabe liegt vollständig vor; Nachladen und
//! `document.write()` kommen mit der Netzwerk- und Skriptanbindung.

use std::collections::VecDeque;

use mb_base::{CodePoint, DomString};

use crate::entities;
use crate::error::{ErrorCode, ParseError};

/// Ein Attribut eines Tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: DomString,
    pub value: DomString,
}

/// Ein Start- oder End-Tag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tag {
    pub name: DomString,
    pub self_closing: bool,
    pub attributes: Vec<Attribute>,
}

/// Ein DOCTYPE. `None` steht für „fehlt“, was etwas anderes ist als die
/// leere Zeichenkette.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doctype {
    pub name: Option<DomString>,
    pub public_id: Option<DomString>,
    pub system_id: Option<DomString>,
    pub force_quirks: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Doctype(Doctype),
    StartTag(Tag),
    EndTag(Tag),
    Comment(DomString),
    ProcessingInstruction { target: DomString, data: DomString },
    Character(CodePoint),
    Eof,
}

/// Die Zustände, in die der Baumaufbau den Tokenizer schalten kann, dazu der
/// CDATA-Zustand als Anfangszustand für Tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenizerState {
    Data,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
    CdataSection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Data,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
    TagOpen,
    EndTagOpen,
    TagName,
    RcdataLessThanSign,
    RcdataEndTagOpen,
    RcdataEndTagName,
    RawtextLessThanSign,
    RawtextEndTagOpen,
    RawtextEndTagName,
    ScriptDataLessThanSign,
    ScriptDataEndTagOpen,
    ScriptDataEndTagName,
    ScriptDataEscapeStart,
    ScriptDataEscapeStartDash,
    ScriptDataEscaped,
    ScriptDataEscapedDash,
    ScriptDataEscapedDashDash,
    ScriptDataEscapedLessThanSign,
    ScriptDataEscapedEndTagOpen,
    ScriptDataEscapedEndTagName,
    ScriptDataDoubleEscapeStart,
    ScriptDataDoubleEscaped,
    ScriptDataDoubleEscapedDash,
    ScriptDataDoubleEscapedDashDash,
    ScriptDataDoubleEscapedLessThanSign,
    ScriptDataDoubleEscapeEnd,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThanSign,
    CommentLessThanSignBang,
    CommentLessThanSignBangDash,
    CommentLessThanSignBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    AfterDoctypePublicKeyword,
    BeforeDoctypePublicIdentifier,
    DoctypePublicIdentifierDoubleQuoted,
    DoctypePublicIdentifierSingleQuoted,
    AfterDoctypePublicIdentifier,
    BetweenDoctypePublicAndSystemIdentifiers,
    AfterDoctypeSystemKeyword,
    BeforeDoctypeSystemIdentifier,
    DoctypeSystemIdentifierDoubleQuoted,
    DoctypeSystemIdentifierSingleQuoted,
    AfterDoctypeSystemIdentifier,
    BogusDoctype,
    CdataSection,
    CdataSectionBracket,
    CdataSectionEnd,
    ProcessingInstructionOpen,
    ProcessingInstructionTarget,
    AfterProcessingInstructionTarget,
    ProcessingInstructionData,
    ProcessingInstructionQuestionable,
    CharacterReference,
    NamedCharacterReference,
    AmbiguousAmpersand,
    NumericCharacterReference,
    HexadecimalCharacterReferenceStart,
    HexadecimalCharacterReference,
    DecimalCharacterReference,
    NumericCharacterReferenceEnd,
}

/// Das aktuelle Eingabezeichen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Char(char),
    /// Ein Surrogat; es kommt nur über Skript-Schnittstellen wie
    /// `document.write()` in den Eingabestrom und fällt überall unter
    /// „anything else“.
    Surrogate(CodePoint),
    Eof,
}

/// Ersetzungen für C1-Steuerzeichen aus dem Abschnitt „Numeric character
/// reference end state“.
const C1_REPLACEMENTS: [(u32, char); 27] = [
    (0x80, '\u{20AC}'),
    (0x82, '\u{201A}'),
    (0x83, '\u{0192}'),
    (0x84, '\u{201E}'),
    (0x85, '\u{2026}'),
    (0x86, '\u{2020}'),
    (0x87, '\u{2021}'),
    (0x88, '\u{02C6}'),
    (0x89, '\u{2030}'),
    (0x8A, '\u{0160}'),
    (0x8B, '\u{2039}'),
    (0x8C, '\u{0152}'),
    (0x8E, '\u{017D}'),
    (0x91, '\u{2018}'),
    (0x92, '\u{2019}'),
    (0x93, '\u{201C}'),
    (0x94, '\u{201D}'),
    (0x95, '\u{2022}'),
    (0x96, '\u{2013}'),
    (0x97, '\u{2014}'),
    (0x98, '\u{02DC}'),
    (0x99, '\u{2122}'),
    (0x9A, '\u{0161}'),
    (0x9B, '\u{203A}'),
    (0x9C, '\u{0153}'),
    (0x9E, '\u{017E}'),
    (0x9F, '\u{0178}'),
];

/// *noncharacter* (Infra): U+FDD0 bis U+FDEF und die letzten beiden
/// Codepunkte jeder Ebene.
fn is_noncharacter(value: u32) -> bool {
    (0xFDD0..=0xFDEF).contains(&value) || ((value & 0xFFFE) == 0xFFFE && value <= 0x10_FFFF)
}

/// *control* (Infra): C0-Steuerzeichen und U+007F bis U+009F.
fn is_control(value: u32) -> bool {
    value <= 0x1F || (0x7F..=0x9F).contains(&value)
}

/// *ASCII whitespace* (Infra).
fn is_ascii_whitespace(value: u32) -> bool {
    matches!(value, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

fn push_next(string: &mut DomString, next: Next) {
    match next {
        Next::Char(c) => string.push(c),
        Next::Surrogate(code_point) => string.push_code_point(code_point),
        Next::Eof => {}
    }
}

/// Der HTML-Tokenizer. [`Tokenizer::next_token`] liefert ein Token nach dem
/// anderen; der Baumaufbau kann dazwischen den Zustand umschalten.
pub struct Tokenizer {
    input: Vec<CodePoint>,
    /// Anfang jeder Zeile in `input`, für die Stellen der Fehler.
    line_starts: Vec<usize>,
    /// Die Stellen der Codepunkte über U+FFFF; sie zählen in den Spalten
    /// doppelt.
    astral: Vec<usize>,
    /// Das nächste Eingabezeichen. Nach dem Dateiende steht `pos` eins
    /// dahinter, damit `pos - 1` immer das aktuelle Eingabezeichen ist.
    pos: usize,
    /// Bis hier sind die Fehler aus „Preprocessing the input stream“ gemeldet.
    checked: usize,
    state: State,
    return_state: State,
    temporary_buffer: String,
    tag: Tag,
    end_tag: bool,
    attribute: Option<Attribute>,
    /// Das aktuelle Attribut hat denselben Namen wie ein früheres und wird
    /// verworfen.
    attribute_duplicate: bool,
    comment: DomString,
    doctype: Doctype,
    processing_instruction: (DomString, DomString),
    /// Wo das Token der Verarbeitungsanweisung entstand.
    processing_instruction_start: usize,
    character_reference_code: u32,
    last_start_tag: Option<DomString>,
    /// Der angepasste aktuelle Knoten ist ein Element außerhalb des
    /// HTML-Namensraums; der Baumaufbau hält das aktuell.
    foreign_content: bool,
    pending: VecDeque<Token>,
    finished: bool,
    errors: Option<Vec<ParseError>>,
}

impl Tokenizer {
    pub fn new(input: &str) -> Tokenizer {
        Tokenizer::from_code_points(input.chars().map(CodePoint::from))
    }

    /// Ein Tokenizer für einen Eingabestrom, der auch Surrogate enthalten
    /// darf. Zeilenumbrüche werden dabei vereinheitlicht (*normalize
    /// newlines*): CR LF und einzelnes CR werden zu LF.
    pub fn from_code_points(input: impl IntoIterator<Item = CodePoint>) -> Tokenizer {
        const CR: CodePoint = CodePoint::from_char('\r');
        const LF: CodePoint = CodePoint::from_char('\n');
        let mut normalized = Vec::new();
        let mut line_starts = vec![0];
        let mut astral = Vec::new();
        let mut after_cr = false;
        for code_point in input {
            if code_point == LF && after_cr {
                after_cr = false;
                continue;
            }
            after_cr = code_point == CR;
            let code_point = if after_cr { LF } else { code_point };
            if code_point.value() > 0xFFFF {
                astral.push(normalized.len());
            }
            normalized.push(code_point);
            if code_point == LF {
                line_starts.push(normalized.len());
            }
        }
        Tokenizer {
            input: normalized,
            line_starts,
            astral,
            pos: 0,
            checked: 0,
            state: State::Data,
            return_state: State::Data,
            temporary_buffer: String::new(),
            tag: Tag::default(),
            end_tag: false,
            attribute: None,
            attribute_duplicate: false,
            comment: DomString::new(),
            doctype: Doctype::default(),
            processing_instruction: (DomString::new(), DomString::new()),
            processing_instruction_start: 0,
            character_reference_code: 0,
            last_start_tag: None,
            foreign_content: false,
            pending: VecDeque::new(),
            finished: false,
            errors: None,
        }
    }

    /// Ab jetzt Parse-Fehler sammeln.
    pub fn collect_errors(&mut self) {
        self.errors.get_or_insert_with(Vec::new);
    }

    /// Die bisher gesammelten Fehler.
    pub fn take_errors(&mut self) -> Vec<ParseError> {
        self.errors.as_mut().map(std::mem::take).unwrap_or_default()
    }

    pub fn set_state(&mut self, state: TokenizerState) {
        self.state = match state {
            TokenizerState::Data => State::Data,
            TokenizerState::Rcdata => State::Rcdata,
            TokenizerState::Rawtext => State::Rawtext,
            TokenizerState::ScriptData => State::ScriptData,
            TokenizerState::Plaintext => State::Plaintext,
            TokenizerState::CdataSection => State::CdataSection,
        };
    }

    /// Setzt den Namen des letzten Start-Tags, das dieser Tokenizer
    /// ausgegeben hat (für *appropriate end tag token*).
    pub fn set_last_start_tag(&mut self, name: Option<DomString>) {
        self.last_start_tag = name;
    }

    /// Ob es einen angepassten aktuellen Knoten gibt und er kein Element im
    /// HTML-Namensraum ist; davon hängt `<![CDATA[` ab.
    pub fn set_foreign_content(&mut self, foreign: bool) {
        self.foreign_content = foreign;
    }

    /// Meldet einen Fehler an der Stelle des aktuellen Eingabezeichens, zum
    /// Beispiel einen Fehler des Baumaufbaus.
    pub fn report(&mut self, code: Option<ErrorCode>) {
        self.report_at(code, self.pos.saturating_sub(1));
    }

    /// Das nächste Token. Nach dem Dateiende immer wieder [`Token::Eof`].
    pub fn next_token(&mut self) -> Token {
        loop {
            if let Some(token) = self.pending.pop_front() {
                return token;
            }
            if self.finished {
                return Token::Eof;
            }
            self.step();
        }
    }

    // Eingabe und Ausgabe

    fn report_at(&mut self, code: Option<ErrorCode>, index: usize) {
        if self.errors.is_none() {
            return;
        }
        let index = index.min(self.input.len());
        let line = self.line_starts.partition_point(|&start| start <= index);
        let start = line
            .checked_sub(1)
            .and_then(|line| self.line_starts.get(line))
            .copied()
            .unwrap_or(0);
        // Die Spalte zählt UTF-16-Codeeinheiten, wie die html5lib-Tests.
        let wide = self
            .astral
            .partition_point(|&at| at < index)
            .saturating_sub(self.astral.partition_point(|&at| at < start));
        let error = ParseError {
            code,
            line,
            column: index
                .saturating_sub(start)
                .saturating_add(wide)
                .saturating_add(1),
        };
        if let Some(errors) = &mut self.errors {
            errors.push(error);
        }
    }

    /// Ein Fehler am aktuellen Eingabezeichen.
    fn error(&mut self, code: ErrorCode) {
        self.report(Some(code));
    }

    /// Ein Fehler am nächsten Eingabezeichen, für Zustände, die nur
    /// vorausschauen.
    fn error_ahead(&mut self, code: ErrorCode) {
        self.report_at(Some(code), self.pos);
    }

    /// Prüft ein Zeichen beim ersten Verbrauch nach „Preprocessing the input
    /// stream“.
    fn check_input(&mut self, index: usize) {
        if index < self.checked {
            return;
        }
        self.checked = index.saturating_add(1);
        let Some(&code_point) = self.input.get(index) else {
            return;
        };
        let value = code_point.value();
        let code = if code_point.is_surrogate() {
            ErrorCode::SurrogateInInputStream
        } else if is_noncharacter(value) {
            ErrorCode::NoncharacterInInputStream
        } else if is_control(value) && !is_ascii_whitespace(value) && value != 0 {
            ErrorCode::ControlCharacterInInputStream
        } else {
            return;
        };
        self.report_at(Some(code), index);
    }

    fn consume(&mut self) -> Next {
        let index = self.pos;
        match self.input.get(index).copied() {
            Some(code_point) => {
                self.pos = index.saturating_add(1);
                self.check_input(index);
                match code_point.to_char() {
                    Some(c) => Next::Char(c),
                    None => Next::Surrogate(code_point),
                }
            }
            None => {
                self.pos = self.input.len().saturating_add(1);
                Next::Eof
            }
        }
    }

    /// Verbraucht `count` Zeichen, die schon vorausschauend geprüft wurden.
    fn advance(&mut self, count: usize) {
        for _ in 0..count {
            self.consume();
        }
    }

    /// Ob die nächsten Eingabezeichen `expected` sind, bei
    /// `ignore_case` ohne Unterschied zwischen ASCII-Groß- und -Kleinbuchstaben.
    fn next_is(&self, from: usize, expected: &str, ignore_case: bool) -> bool {
        let mut index = from;
        for c in expected.chars() {
            let Some(found) = self.input.get(index).and_then(|cp| cp.to_char()) else {
                return false;
            };
            let same = if ignore_case {
                found.eq_ignore_ascii_case(&c)
            } else {
                found == c
            };
            if !same {
                return false;
            }
            index = index.saturating_add(1);
        }
        true
    }

    /// *Reconsume in* `state`.
    fn reconsume(&mut self, state: State) {
        self.pos = self.pos.saturating_sub(1);
        self.state = state;
    }

    fn emit_char(&mut self, c: char) {
        self.pending.push_back(Token::Character(CodePoint::from(c)));
    }

    fn emit_str(&mut self, text: &str) {
        for c in text.chars() {
            self.emit_char(c);
        }
    }

    /// *Emit the current input character as a character token.*
    fn emit_next(&mut self, next: Next) {
        match next {
            Next::Char(c) => self.emit_char(c),
            Next::Surrogate(code_point) => self.pending.push_back(Token::Character(code_point)),
            Next::Eof => {}
        }
    }

    fn emit_eof(&mut self) {
        self.pending.push_back(Token::Eof);
        self.finished = true;
    }

    fn emit_comment(&mut self) {
        let data = std::mem::take(&mut self.comment);
        self.pending.push_back(Token::Comment(data));
    }

    fn emit_doctype(&mut self) {
        let doctype = std::mem::take(&mut self.doctype);
        self.pending.push_back(Token::Doctype(doctype));
    }

    fn emit_processing_instruction(&mut self) {
        let (target, data) = std::mem::take(&mut self.processing_instruction);
        self.pending
            .push_back(Token::ProcessingInstruction { target, data });
    }

    fn emit_tag(&mut self) {
        self.finish_attribute();
        let tag = std::mem::take(&mut self.tag);
        if self.end_tag {
            if !tag.attributes.is_empty() {
                self.error(ErrorCode::EndTagWithAttributes);
            }
            if tag.self_closing {
                self.error(ErrorCode::EndTagWithTrailingSolidus);
            }
            self.pending.push_back(Token::EndTag(tag));
        } else {
            self.last_start_tag = Some(tag.name.clone());
            self.pending.push_back(Token::StartTag(tag));
        }
    }

    // Tags und Attribute

    fn new_tag(&mut self, end_tag: bool) {
        self.tag = Tag::default();
        self.end_tag = end_tag;
        self.attribute = None;
    }

    /// *Start a new attribute in the current tag token.*
    fn start_attribute(&mut self, name: DomString) {
        self.finish_attribute();
        self.attribute = Some(Attribute {
            name,
            value: DomString::new(),
        });
        self.attribute_duplicate = false;
    }

    /// Beim Verlassen des Zustands „Attribute name state“: Ein Name, den der
    /// Tag schon hat, ist ein Fehler, und das Attribut wird verworfen.
    fn leave_attribute_name(&mut self) {
        let duplicate = self.attribute.as_ref().is_some_and(|attribute| {
            self.tag
                .attributes
                .iter()
                .any(|earlier| earlier.name == attribute.name)
        });
        if duplicate {
            self.error(ErrorCode::DuplicateAttribute);
            self.attribute_duplicate = true;
        }
    }

    fn finish_attribute(&mut self) {
        if let Some(attribute) = self.attribute.take()
            && !self.attribute_duplicate
        {
            self.tag.attributes.push(attribute);
        }
        self.attribute_duplicate = false;
    }

    fn attribute_name(&mut self) -> Option<&mut DomString> {
        self.attribute.as_mut().map(|attribute| &mut attribute.name)
    }

    fn attribute_value(&mut self) -> Option<&mut DomString> {
        self.attribute
            .as_mut()
            .map(|attribute| &mut attribute.value)
    }

    /// *An appropriate end tag token.*
    fn is_appropriate_end_tag(&self) -> bool {
        self.end_tag && self.last_start_tag.as_ref() == Some(&self.tag.name)
    }

    // Zeichenreferenzen

    /// *Consumed as part of an attribute.*
    fn in_attribute(&self) -> bool {
        matches!(
            self.return_state,
            State::AttributeValueDoubleQuoted
                | State::AttributeValueSingleQuoted
                | State::AttributeValueUnquoted
        )
    }

    /// *Flush code points consumed as a character reference.*
    fn flush_character_reference(&mut self) {
        let buffer = std::mem::take(&mut self.temporary_buffer);
        if self.in_attribute() {
            if let Some(value) = self.attribute_value() {
                value.push_str(&buffer);
            }
        } else {
            self.emit_str(&buffer);
        }
        self.temporary_buffer = buffer;
    }

    /// *Convert the temporary buffer to a comment.*
    fn temporary_buffer_to_comment(&mut self) {
        self.comment = DomString::from("?");
        self.comment.push_str(&self.temporary_buffer);
    }

    fn step(&mut self) {
        match self.state {
            State::Data => self.data(),
            State::Rcdata => self.rcdata(),
            State::Rawtext => self.rawtext(),
            State::ScriptData => self.script_data(),
            State::Plaintext => self.plaintext(),
            State::TagOpen => self.tag_open(),
            State::EndTagOpen => self.end_tag_open(),
            State::TagName => self.tag_name(),
            State::RcdataLessThanSign => {
                self.text_less_than_sign(State::Rcdata, State::RcdataEndTagOpen)
            }
            State::RcdataEndTagOpen => {
                self.text_end_tag_open(State::Rcdata, State::RcdataEndTagName)
            }
            State::RcdataEndTagName => self.text_end_tag_name(State::Rcdata),
            State::RawtextLessThanSign => {
                self.text_less_than_sign(State::Rawtext, State::RawtextEndTagOpen)
            }
            State::RawtextEndTagOpen => {
                self.text_end_tag_open(State::Rawtext, State::RawtextEndTagName)
            }
            State::RawtextEndTagName => self.text_end_tag_name(State::Rawtext),
            State::ScriptDataLessThanSign => self.script_data_less_than_sign(),
            State::ScriptDataEndTagOpen => {
                self.text_end_tag_open(State::ScriptData, State::ScriptDataEndTagName)
            }
            State::ScriptDataEndTagName => self.text_end_tag_name(State::ScriptData),
            State::ScriptDataEscapeStart => self.script_data_escape_start(),
            State::ScriptDataEscapeStartDash => self.script_data_escape_start_dash(),
            State::ScriptDataEscaped => self.script_data_escaped(),
            State::ScriptDataEscapedDash => self.script_data_escaped_dash(),
            State::ScriptDataEscapedDashDash => self.script_data_escaped_dash_dash(),
            State::ScriptDataEscapedLessThanSign => self.script_data_escaped_less_than_sign(),
            State::ScriptDataEscapedEndTagOpen => {
                self.text_end_tag_open(State::ScriptDataEscaped, State::ScriptDataEscapedEndTagName)
            }
            State::ScriptDataEscapedEndTagName => self.text_end_tag_name(State::ScriptDataEscaped),
            State::ScriptDataDoubleEscapeStart => self.script_data_double_escape_boundary(
                State::ScriptDataDoubleEscaped,
                State::ScriptDataEscaped,
            ),
            State::ScriptDataDoubleEscaped => self.script_data_double_escaped(),
            State::ScriptDataDoubleEscapedDash => self.script_data_double_escaped_dash(),
            State::ScriptDataDoubleEscapedDashDash => self.script_data_double_escaped_dash_dash(),
            State::ScriptDataDoubleEscapedLessThanSign => {
                self.script_data_double_escaped_less_than_sign()
            }
            State::ScriptDataDoubleEscapeEnd => self.script_data_double_escape_boundary(
                State::ScriptDataEscaped,
                State::ScriptDataDoubleEscaped,
            ),
            State::BeforeAttributeName => self.before_attribute_name(),
            State::AttributeName => self.attribute_name_state(),
            State::AfterAttributeName => self.after_attribute_name(),
            State::BeforeAttributeValue => self.before_attribute_value(),
            State::AttributeValueDoubleQuoted => {
                self.attribute_value_quoted('"', State::AttributeValueDoubleQuoted)
            }
            State::AttributeValueSingleQuoted => {
                self.attribute_value_quoted('\'', State::AttributeValueSingleQuoted)
            }
            State::AttributeValueUnquoted => self.attribute_value_unquoted(),
            State::AfterAttributeValueQuoted => self.after_attribute_value_quoted(),
            State::SelfClosingStartTag => self.self_closing_start_tag(),
            State::BogusComment => self.bogus_comment(),
            State::MarkupDeclarationOpen => self.markup_declaration_open(),
            State::CommentStart => self.comment_start(),
            State::CommentStartDash => self.comment_start_dash(),
            State::Comment => self.comment_state(),
            State::CommentLessThanSign => self.comment_less_than_sign(),
            State::CommentLessThanSignBang => self.comment_less_than_sign_bang(),
            State::CommentLessThanSignBangDash => self.comment_less_than_sign_bang_dash(),
            State::CommentLessThanSignBangDashDash => self.comment_less_than_sign_bang_dash_dash(),
            State::CommentEndDash => self.comment_end_dash(),
            State::CommentEnd => self.comment_end(),
            State::CommentEndBang => self.comment_end_bang(),
            State::Doctype => self.doctype_state(),
            State::BeforeDoctypeName => self.before_doctype_name(),
            State::DoctypeName => self.doctype_name(),
            State::AfterDoctypeName => self.after_doctype_name(),
            State::AfterDoctypePublicKeyword => self.after_doctype_keyword(Identifier::Public),
            State::BeforeDoctypePublicIdentifier => {
                self.before_doctype_identifier(Identifier::Public)
            }
            State::DoctypePublicIdentifierDoubleQuoted => {
                self.doctype_identifier_quoted(Identifier::Public, '"')
            }
            State::DoctypePublicIdentifierSingleQuoted => {
                self.doctype_identifier_quoted(Identifier::Public, '\'')
            }
            State::AfterDoctypePublicIdentifier => self.after_doctype_public_identifier(),
            State::BetweenDoctypePublicAndSystemIdentifiers => {
                self.between_doctype_public_and_system_identifiers()
            }
            State::AfterDoctypeSystemKeyword => self.after_doctype_keyword(Identifier::System),
            State::BeforeDoctypeSystemIdentifier => {
                self.before_doctype_identifier(Identifier::System)
            }
            State::DoctypeSystemIdentifierDoubleQuoted => {
                self.doctype_identifier_quoted(Identifier::System, '"')
            }
            State::DoctypeSystemIdentifierSingleQuoted => {
                self.doctype_identifier_quoted(Identifier::System, '\'')
            }
            State::AfterDoctypeSystemIdentifier => self.after_doctype_system_identifier(),
            State::BogusDoctype => self.bogus_doctype(),
            State::CdataSection => self.cdata_section(),
            State::CdataSectionBracket => self.cdata_section_bracket(),
            State::CdataSectionEnd => self.cdata_section_end(),
            State::ProcessingInstructionOpen => self.processing_instruction_open(),
            State::ProcessingInstructionTarget => self.processing_instruction_target(),
            State::AfterProcessingInstructionTarget => self.after_processing_instruction_target(),
            State::ProcessingInstructionData => self.processing_instruction_data(),
            State::ProcessingInstructionQuestionable => self.processing_instruction_questionable(),
            State::CharacterReference => self.character_reference(),
            State::NamedCharacterReference => self.named_character_reference(),
            State::AmbiguousAmpersand => self.ambiguous_ampersand(),
            State::NumericCharacterReference => self.numeric_character_reference(),
            State::HexadecimalCharacterReferenceStart => {
                self.hexadecimal_character_reference_start()
            }
            State::HexadecimalCharacterReference => self.hexadecimal_character_reference(),
            State::DecimalCharacterReference => self.decimal_character_reference(),
            State::NumericCharacterReferenceEnd => self.numeric_character_reference_end(),
        }
    }

    // Text

    /// „Data state“
    fn data(&mut self) {
        match self.consume() {
            Next::Char('&') => {
                self.return_state = State::Data;
                self.state = State::CharacterReference;
            }
            Next::Char('<') => self.state = State::TagOpen,
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.emit_char('\0');
            }
            Next::Eof => self.emit_eof(),
            next => self.emit_next(next),
        }
    }

    /// „RCDATA state“
    fn rcdata(&mut self) {
        match self.consume() {
            Next::Char('&') => {
                self.return_state = State::Rcdata;
                self.state = State::CharacterReference;
            }
            Next::Char('<') => self.state = State::RcdataLessThanSign,
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => self.emit_eof(),
            next => self.emit_next(next),
        }
    }

    /// „RAWTEXT state“
    fn rawtext(&mut self) {
        match self.consume() {
            Next::Char('<') => self.state = State::RawtextLessThanSign,
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => self.emit_eof(),
            next => self.emit_next(next),
        }
    }

    /// „Script data state“
    fn script_data(&mut self) {
        match self.consume() {
            Next::Char('<') => self.state = State::ScriptDataLessThanSign,
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => self.emit_eof(),
            next => self.emit_next(next),
        }
    }

    /// „PLAINTEXT state“
    fn plaintext(&mut self) {
        match self.consume() {
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => self.emit_eof(),
            next => self.emit_next(next),
        }
    }

    // Tags

    /// „Tag open state“
    fn tag_open(&mut self) {
        match self.consume() {
            Next::Char('!') => self.state = State::MarkupDeclarationOpen,
            Next::Char('/') => self.state = State::EndTagOpen,
            Next::Char(c) if c.is_ascii_alphabetic() => {
                self.new_tag(false);
                self.reconsume(State::TagName);
            }
            Next::Char('?') => {
                self.temporary_buffer.clear();
                self.state = State::ProcessingInstructionOpen;
            }
            Next::Eof => {
                self.error(ErrorCode::EofBeforeTagName);
                self.emit_char('<');
                self.emit_eof();
            }
            _ => {
                self.error(ErrorCode::InvalidFirstCharacterOfTagName);
                self.emit_char('<');
                self.reconsume(State::Data);
            }
        }
    }

    /// „End tag open state“
    fn end_tag_open(&mut self) {
        match self.consume() {
            Next::Char(c) if c.is_ascii_alphabetic() => {
                self.new_tag(true);
                self.reconsume(State::TagName);
            }
            Next::Char('>') => {
                self.error(ErrorCode::MissingEndTagName);
                self.state = State::Data;
            }
            Next::Eof => {
                self.error(ErrorCode::EofBeforeTagName);
                self.emit_str("</");
                self.emit_eof();
            }
            _ => {
                self.error(ErrorCode::InvalidFirstCharacterOfTagName);
                self.comment = DomString::new();
                self.reconsume(State::BogusComment);
            }
        }
    }

    /// „Tag name state“
    fn tag_name(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => self.state = State::BeforeAttributeName,
            Next::Char('/') => self.state = State::SelfClosingStartTag,
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_tag();
            }
            Next::Char(c) if c.is_ascii_uppercase() => self.tag.name.push(c.to_ascii_lowercase()),
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.tag.name.push('\u{FFFD}');
            }
            Next::Eof => {
                self.error(ErrorCode::EofInTag);
                self.emit_eof();
            }
            next => push_next(&mut self.tag.name, next),
        }
    }

    /// „RCDATA less-than sign state“ und „RAWTEXT less-than sign state“
    fn text_less_than_sign(&mut self, text: State, end_tag_open: State) {
        match self.consume() {
            Next::Char('/') => {
                self.temporary_buffer.clear();
                self.state = end_tag_open;
            }
            _ => {
                self.emit_char('<');
                self.reconsume(text);
            }
        }
    }

    /// „RCDATA end tag open state“ und die entsprechenden Zustände für
    /// RAWTEXT und Skriptdaten
    fn text_end_tag_open(&mut self, text: State, end_tag_name: State) {
        match self.consume() {
            Next::Char(c) if c.is_ascii_alphabetic() => {
                self.new_tag(true);
                self.reconsume(end_tag_name);
            }
            _ => {
                self.emit_str("</");
                self.reconsume(text);
            }
        }
    }

    /// „RCDATA end tag name state“ und die entsprechenden Zustände für
    /// RAWTEXT und Skriptdaten
    fn text_end_tag_name(&mut self, text: State) {
        let next = self.consume();
        match next {
            Next::Char('\t' | '\n' | '\x0C' | ' ') if self.is_appropriate_end_tag() => {
                self.state = State::BeforeAttributeName;
                return;
            }
            Next::Char('/') if self.is_appropriate_end_tag() => {
                self.state = State::SelfClosingStartTag;
                return;
            }
            Next::Char('>') if self.is_appropriate_end_tag() => {
                self.state = State::Data;
                self.emit_tag();
                return;
            }
            Next::Char(c) if c.is_ascii_alphabetic() => {
                self.tag.name.push(c.to_ascii_lowercase());
                self.temporary_buffer.push(c);
                return;
            }
            _ => {}
        }
        self.emit_str("</");
        let buffer = std::mem::take(&mut self.temporary_buffer);
        self.emit_str(&buffer);
        self.temporary_buffer = buffer;
        self.reconsume(text);
    }

    // Skriptdaten

    /// „Script data less-than sign state“
    fn script_data_less_than_sign(&mut self) {
        match self.consume() {
            Next::Char('/') => {
                self.temporary_buffer.clear();
                self.state = State::ScriptDataEndTagOpen;
            }
            Next::Char('!') => {
                self.state = State::ScriptDataEscapeStart;
                self.emit_str("<!");
            }
            _ => {
                self.emit_char('<');
                self.reconsume(State::ScriptData);
            }
        }
    }

    /// „Script data escape start state“
    fn script_data_escape_start(&mut self) {
        match self.consume() {
            Next::Char('-') => {
                self.state = State::ScriptDataEscapeStartDash;
                self.emit_char('-');
            }
            _ => self.reconsume(State::ScriptData),
        }
    }

    /// „Script data escape start dash state“
    fn script_data_escape_start_dash(&mut self) {
        match self.consume() {
            Next::Char('-') => {
                self.state = State::ScriptDataEscapedDashDash;
                self.emit_char('-');
            }
            _ => self.reconsume(State::ScriptData),
        }
    }

    /// „Script data escaped state“
    fn script_data_escaped(&mut self) {
        match self.consume() {
            Next::Char('-') => {
                self.state = State::ScriptDataEscapedDash;
                self.emit_char('-');
            }
            Next::Char('<') => self.state = State::ScriptDataEscapedLessThanSign,
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => {
                self.error(ErrorCode::EofInScriptHtmlCommentLikeText);
                self.emit_eof();
            }
            next => self.emit_next(next),
        }
    }

    /// „Script data escaped dash state“
    fn script_data_escaped_dash(&mut self) {
        match self.consume() {
            Next::Char('-') => {
                self.state = State::ScriptDataEscapedDashDash;
                self.emit_char('-');
            }
            Next::Char('<') => self.state = State::ScriptDataEscapedLessThanSign,
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.state = State::ScriptDataEscaped;
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => {
                self.error(ErrorCode::EofInScriptHtmlCommentLikeText);
                self.emit_eof();
            }
            next => {
                self.state = State::ScriptDataEscaped;
                self.emit_next(next);
            }
        }
    }

    /// „Script data escaped dash dash state“
    fn script_data_escaped_dash_dash(&mut self) {
        match self.consume() {
            Next::Char('-') => self.emit_char('-'),
            Next::Char('<') => self.state = State::ScriptDataEscapedLessThanSign,
            Next::Char('>') => {
                self.state = State::ScriptData;
                self.emit_char('>');
            }
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.state = State::ScriptDataEscaped;
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => {
                self.error(ErrorCode::EofInScriptHtmlCommentLikeText);
                self.emit_eof();
            }
            next => {
                self.state = State::ScriptDataEscaped;
                self.emit_next(next);
            }
        }
    }

    /// „Script data escaped less-than sign state“
    fn script_data_escaped_less_than_sign(&mut self) {
        match self.consume() {
            Next::Char('/') => {
                self.temporary_buffer.clear();
                self.state = State::ScriptDataEscapedEndTagOpen;
            }
            Next::Char(c) if c.is_ascii_alphabetic() => {
                self.temporary_buffer.clear();
                self.emit_char('<');
                self.reconsume(State::ScriptDataDoubleEscapeStart);
            }
            _ => {
                self.emit_char('<');
                self.reconsume(State::ScriptDataEscaped);
            }
        }
    }

    /// „Script data double escape start state“ (`script` führt nach
    /// `matched`) und „Script data double escape end state“ (umgekehrt)
    fn script_data_double_escape_boundary(&mut self, matched: State, otherwise: State) {
        match self.consume() {
            Next::Char(c @ ('\t' | '\n' | '\x0C' | ' ' | '/' | '>')) => {
                self.state = if self.temporary_buffer == "script" {
                    matched
                } else {
                    otherwise
                };
                self.emit_char(c);
            }
            Next::Char(c) if c.is_ascii_alphabetic() => {
                self.temporary_buffer.push(c.to_ascii_lowercase());
                self.emit_char(c);
            }
            // Die Rückkehr geht beim Anfang ins Escape, beim Ende ins
            // doppelte Escape, also in den Zustand, aus dem der Tokenizer kam.
            _ => self.reconsume(otherwise),
        }
    }

    /// „Script data double escaped state“
    fn script_data_double_escaped(&mut self) {
        match self.consume() {
            Next::Char('-') => {
                self.state = State::ScriptDataDoubleEscapedDash;
                self.emit_char('-');
            }
            Next::Char('<') => {
                self.state = State::ScriptDataDoubleEscapedLessThanSign;
                self.emit_char('<');
            }
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => {
                self.error(ErrorCode::EofInScriptHtmlCommentLikeText);
                self.emit_eof();
            }
            next => self.emit_next(next),
        }
    }

    /// „Script data double escaped dash state“
    fn script_data_double_escaped_dash(&mut self) {
        match self.consume() {
            Next::Char('-') => {
                self.state = State::ScriptDataDoubleEscapedDashDash;
                self.emit_char('-');
            }
            Next::Char('<') => {
                self.state = State::ScriptDataDoubleEscapedLessThanSign;
                self.emit_char('<');
            }
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.state = State::ScriptDataDoubleEscaped;
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => {
                self.error(ErrorCode::EofInScriptHtmlCommentLikeText);
                self.emit_eof();
            }
            next => {
                self.state = State::ScriptDataDoubleEscaped;
                self.emit_next(next);
            }
        }
    }

    /// „Script data double escaped dash dash state“
    fn script_data_double_escaped_dash_dash(&mut self) {
        match self.consume() {
            Next::Char('-') => self.emit_char('-'),
            Next::Char('<') => {
                self.state = State::ScriptDataDoubleEscapedLessThanSign;
                self.emit_char('<');
            }
            Next::Char('>') => {
                self.state = State::ScriptData;
                self.emit_char('>');
            }
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.state = State::ScriptDataDoubleEscaped;
                self.emit_char('\u{FFFD}');
            }
            Next::Eof => {
                self.error(ErrorCode::EofInScriptHtmlCommentLikeText);
                self.emit_eof();
            }
            next => {
                self.state = State::ScriptDataDoubleEscaped;
                self.emit_next(next);
            }
        }
    }

    /// „Script data double escaped less-than sign state“
    fn script_data_double_escaped_less_than_sign(&mut self) {
        match self.consume() {
            Next::Char('/') => {
                self.temporary_buffer.clear();
                self.state = State::ScriptDataDoubleEscapeEnd;
                self.emit_char('/');
            }
            _ => self.reconsume(State::ScriptDataDoubleEscaped),
        }
    }

    // Attribute

    /// „Before attribute name state“
    fn before_attribute_name(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => {}
            Next::Char('/' | '>') | Next::Eof => self.reconsume(State::AfterAttributeName),
            Next::Char('=') => {
                self.error(ErrorCode::UnexpectedEqualsSignBeforeAttributeName);
                self.start_attribute(DomString::from('='));
                self.state = State::AttributeName;
            }
            _ => {
                self.start_attribute(DomString::new());
                self.reconsume(State::AttributeName);
            }
        }
    }

    /// „Attribute name state“
    fn attribute_name_state(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ' | '/' | '>') | Next::Eof => {
                self.leave_attribute_name();
                self.reconsume(State::AfterAttributeName);
            }
            Next::Char('=') => {
                self.leave_attribute_name();
                self.state = State::BeforeAttributeValue;
            }
            Next::Char(c) if c.is_ascii_uppercase() => {
                if let Some(name) = self.attribute_name() {
                    name.push(c.to_ascii_lowercase());
                }
            }
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                if let Some(name) = self.attribute_name() {
                    name.push('\u{FFFD}');
                }
            }
            next => {
                if let Next::Char('"' | '\'' | '<') = next {
                    self.error(ErrorCode::UnexpectedCharacterInAttributeName);
                }
                if let Some(name) = self.attribute_name() {
                    push_next(name, next);
                }
            }
        }
    }

    /// „After attribute name state“
    fn after_attribute_name(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => {}
            Next::Char('/') => self.state = State::SelfClosingStartTag,
            Next::Char('=') => self.state = State::BeforeAttributeValue,
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_tag();
            }
            Next::Eof => {
                self.error(ErrorCode::EofInTag);
                self.emit_eof();
            }
            _ => {
                self.start_attribute(DomString::new());
                self.reconsume(State::AttributeName);
            }
        }
    }

    /// „Before attribute value state“
    fn before_attribute_value(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => {}
            Next::Char('"') => self.state = State::AttributeValueDoubleQuoted,
            Next::Char('\'') => self.state = State::AttributeValueSingleQuoted,
            Next::Char('>') => {
                self.error(ErrorCode::MissingAttributeValue);
                self.state = State::Data;
                self.emit_tag();
            }
            _ => self.reconsume(State::AttributeValueUnquoted),
        }
    }

    /// „Attribute value (double-quoted) state“ und „Attribute value
    /// (single-quoted) state“
    fn attribute_value_quoted(&mut self, quote: char, state: State) {
        match self.consume() {
            Next::Char(c) if c == quote => self.state = State::AfterAttributeValueQuoted,
            Next::Char('&') => {
                self.return_state = state;
                self.state = State::CharacterReference;
            }
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                if let Some(value) = self.attribute_value() {
                    value.push('\u{FFFD}');
                }
            }
            Next::Eof => {
                self.error(ErrorCode::EofInTag);
                self.emit_eof();
            }
            next => {
                if let Some(value) = self.attribute_value() {
                    push_next(value, next);
                }
            }
        }
    }

    /// „Attribute value (unquoted) state“
    fn attribute_value_unquoted(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => self.state = State::BeforeAttributeName,
            Next::Char('&') => {
                self.return_state = State::AttributeValueUnquoted;
                self.state = State::CharacterReference;
            }
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_tag();
            }
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                if let Some(value) = self.attribute_value() {
                    value.push('\u{FFFD}');
                }
            }
            Next::Eof => {
                self.error(ErrorCode::EofInTag);
                self.emit_eof();
            }
            next => {
                if let Next::Char('"' | '\'' | '<' | '=' | '`') = next {
                    self.error(ErrorCode::UnexpectedCharacterInUnquotedAttributeValue);
                }
                if let Some(value) = self.attribute_value() {
                    push_next(value, next);
                }
            }
        }
    }

    /// „After attribute value (quoted) state“
    fn after_attribute_value_quoted(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => self.state = State::BeforeAttributeName,
            Next::Char('/') => self.state = State::SelfClosingStartTag,
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_tag();
            }
            Next::Eof => {
                self.error(ErrorCode::EofInTag);
                self.emit_eof();
            }
            _ => {
                self.error(ErrorCode::MissingWhitespaceBetweenAttributes);
                self.reconsume(State::BeforeAttributeName);
            }
        }
    }

    /// „Self-closing start tag state“
    fn self_closing_start_tag(&mut self) {
        match self.consume() {
            Next::Char('>') => {
                self.tag.self_closing = true;
                self.state = State::Data;
                self.emit_tag();
            }
            Next::Eof => {
                self.error(ErrorCode::EofInTag);
                self.emit_eof();
            }
            _ => {
                self.error(ErrorCode::UnexpectedSolidusInTag);
                self.reconsume(State::BeforeAttributeName);
            }
        }
    }

    // Kommentare

    /// „Bogus comment state“
    fn bogus_comment(&mut self) {
        match self.consume() {
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_comment();
            }
            Next::Eof => {
                self.emit_comment();
                self.emit_eof();
            }
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.comment.push('\u{FFFD}');
            }
            next => push_next(&mut self.comment, next),
        }
    }

    /// „Markup declaration open state“
    fn markup_declaration_open(&mut self) {
        if self.next_is(self.pos, "--", false) {
            self.advance(2);
            self.comment = DomString::new();
            self.state = State::CommentStart;
        } else if self.next_is(self.pos, "DOCTYPE", true) {
            self.advance(7);
            self.state = State::Doctype;
        } else if self.next_is(self.pos, "[CDATA[", false) {
            self.advance(7);
            if self.foreign_content {
                self.state = State::CdataSection;
            } else {
                self.error(ErrorCode::CdataInHtmlContent);
                self.comment = DomString::from("[CDATA[");
                self.state = State::BogusComment;
            }
        } else {
            // In den html5lib-Tests steht ein Fehler des Eingabestroms für
            // das nächste Zeichen vor diesem.
            self.check_input(self.pos);
            self.error_ahead(ErrorCode::IncorrectlyOpenedComment);
            self.comment = DomString::new();
            self.state = State::BogusComment;
        }
    }

    /// „Comment start state“
    fn comment_start(&mut self) {
        match self.consume() {
            Next::Char('-') => self.state = State::CommentStartDash,
            Next::Char('>') => {
                self.error(ErrorCode::AbruptClosingOfEmptyComment);
                self.state = State::Data;
                self.emit_comment();
            }
            _ => self.reconsume(State::Comment),
        }
    }

    /// „Comment start dash state“
    fn comment_start_dash(&mut self) {
        match self.consume() {
            Next::Char('-') => self.state = State::CommentEnd,
            Next::Char('>') => {
                self.error(ErrorCode::AbruptClosingOfEmptyComment);
                self.state = State::Data;
                self.emit_comment();
            }
            Next::Eof => {
                self.error(ErrorCode::EofInComment);
                self.emit_comment();
                self.emit_eof();
            }
            _ => {
                self.comment.push('-');
                self.reconsume(State::Comment);
            }
        }
    }

    /// „Comment state“
    fn comment_state(&mut self) {
        match self.consume() {
            Next::Char('<') => {
                self.comment.push('<');
                self.state = State::CommentLessThanSign;
            }
            Next::Char('-') => self.state = State::CommentEndDash,
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.comment.push('\u{FFFD}');
            }
            Next::Eof => {
                self.error(ErrorCode::EofInComment);
                self.emit_comment();
                self.emit_eof();
            }
            next => push_next(&mut self.comment, next),
        }
    }

    /// „Comment less-than sign state“
    fn comment_less_than_sign(&mut self) {
        match self.consume() {
            Next::Char('!') => {
                self.comment.push('!');
                self.state = State::CommentLessThanSignBang;
            }
            Next::Char('<') => self.comment.push('<'),
            _ => self.reconsume(State::Comment),
        }
    }

    /// „Comment less-than sign bang state“
    fn comment_less_than_sign_bang(&mut self) {
        match self.consume() {
            Next::Char('-') => self.state = State::CommentLessThanSignBangDash,
            _ => self.reconsume(State::Comment),
        }
    }

    /// „Comment less-than sign bang dash state“
    fn comment_less_than_sign_bang_dash(&mut self) {
        match self.consume() {
            Next::Char('-') => self.state = State::CommentLessThanSignBangDashDash,
            _ => self.reconsume(State::CommentEndDash),
        }
    }

    /// „Comment less-than sign bang dash dash state“
    fn comment_less_than_sign_bang_dash_dash(&mut self) {
        match self.consume() {
            Next::Char('>') | Next::Eof => self.reconsume(State::CommentEnd),
            _ => {
                self.error(ErrorCode::NestedComment);
                self.reconsume(State::CommentEnd);
            }
        }
    }

    /// „Comment end dash state“
    fn comment_end_dash(&mut self) {
        match self.consume() {
            Next::Char('-') => self.state = State::CommentEnd,
            Next::Eof => {
                self.error(ErrorCode::EofInComment);
                self.emit_comment();
                self.emit_eof();
            }
            _ => {
                self.comment.push('-');
                self.reconsume(State::Comment);
            }
        }
    }

    /// „Comment end state“
    fn comment_end(&mut self) {
        match self.consume() {
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_comment();
            }
            Next::Char('!') => self.state = State::CommentEndBang,
            Next::Char('-') => self.comment.push('-'),
            Next::Eof => {
                self.error(ErrorCode::EofInComment);
                self.emit_comment();
                self.emit_eof();
            }
            _ => {
                self.comment.push_str("--");
                self.reconsume(State::Comment);
            }
        }
    }

    /// „Comment end bang state“
    fn comment_end_bang(&mut self) {
        match self.consume() {
            Next::Char('-') => {
                self.comment.push_str("--!");
                self.state = State::CommentEndDash;
            }
            Next::Char('>') => {
                self.error(ErrorCode::IncorrectlyClosedComment);
                self.state = State::Data;
                self.emit_comment();
            }
            Next::Eof => {
                self.error(ErrorCode::EofInComment);
                self.emit_comment();
                self.emit_eof();
            }
            _ => {
                self.comment.push_str("--!");
                self.reconsume(State::Comment);
            }
        }
    }

    // DOCTYPE

    /// Der Fall „EOF“ in den meisten DOCTYPE-Zuständen.
    fn eof_in_doctype(&mut self) {
        self.error(ErrorCode::EofInDoctype);
        self.doctype.force_quirks = true;
        self.emit_doctype();
        self.emit_eof();
    }

    /// „DOCTYPE state“
    fn doctype_state(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => self.state = State::BeforeDoctypeName,
            Next::Char('>') => self.reconsume(State::BeforeDoctypeName),
            Next::Eof => {
                self.doctype = Doctype::default();
                self.eof_in_doctype();
            }
            _ => {
                self.error(ErrorCode::MissingWhitespaceBeforeDoctypeName);
                self.reconsume(State::BeforeDoctypeName);
            }
        }
    }

    /// „Before DOCTYPE name state“
    fn before_doctype_name(&mut self) {
        let next = self.consume();
        let name = match next {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => return,
            Next::Char(c) if c.is_ascii_uppercase() => DomString::from(c.to_ascii_lowercase()),
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                DomString::from('\u{FFFD}')
            }
            Next::Char('>') => {
                self.error(ErrorCode::MissingDoctypeName);
                self.doctype = Doctype {
                    force_quirks: true,
                    ..Doctype::default()
                };
                self.state = State::Data;
                self.emit_doctype();
                return;
            }
            Next::Eof => {
                self.doctype = Doctype::default();
                self.eof_in_doctype();
                return;
            }
            next => {
                let mut name = DomString::new();
                push_next(&mut name, next);
                name
            }
        };
        self.doctype = Doctype {
            name: Some(name),
            ..Doctype::default()
        };
        self.state = State::DoctypeName;
    }

    fn doctype_name_mut(&mut self) -> &mut DomString {
        self.doctype.name.get_or_insert_with(DomString::new)
    }

    /// „DOCTYPE name state“
    fn doctype_name(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => self.state = State::AfterDoctypeName,
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Next::Char(c) if c.is_ascii_uppercase() => {
                self.doctype_name_mut().push(c.to_ascii_lowercase());
            }
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                self.doctype_name_mut().push('\u{FFFD}');
            }
            Next::Eof => self.eof_in_doctype(),
            next => push_next(self.doctype_name_mut(), next),
        }
    }

    /// „After DOCTYPE name state“
    fn after_doctype_name(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => {}
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Next::Eof => self.eof_in_doctype(),
            _ => {
                // Die sechs Zeichen ab dem aktuellen Eingabezeichen.
                let current = self.pos.saturating_sub(1);
                if self.next_is(current, "PUBLIC", true) {
                    self.advance(5);
                    self.state = State::AfterDoctypePublicKeyword;
                } else if self.next_is(current, "SYSTEM", true) {
                    self.advance(5);
                    self.state = State::AfterDoctypeSystemKeyword;
                } else {
                    self.error(ErrorCode::InvalidCharacterSequenceAfterDoctypeName);
                    self.doctype.force_quirks = true;
                    self.reconsume(State::BogusDoctype);
                }
            }
        }
    }

    /// Setzt den öffentlichen oder den Systembezeichner auf die leere
    /// Zeichenkette und wechselt in den Zustand für `quote`.
    fn start_doctype_identifier(&mut self, identifier: Identifier, quote: char) {
        let (field, state) = match (identifier, quote) {
            (Identifier::Public, '"') => (
                &mut self.doctype.public_id,
                State::DoctypePublicIdentifierDoubleQuoted,
            ),
            (Identifier::Public, _) => (
                &mut self.doctype.public_id,
                State::DoctypePublicIdentifierSingleQuoted,
            ),
            (Identifier::System, '"') => (
                &mut self.doctype.system_id,
                State::DoctypeSystemIdentifierDoubleQuoted,
            ),
            (Identifier::System, _) => (
                &mut self.doctype.system_id,
                State::DoctypeSystemIdentifierSingleQuoted,
            ),
        };
        *field = Some(DomString::new());
        self.state = state;
    }

    /// „After DOCTYPE public keyword state“ und „After DOCTYPE system keyword
    /// state“
    fn after_doctype_keyword(&mut self, identifier: Identifier) {
        let (before, missing_whitespace, missing, missing_quote) = match identifier {
            Identifier::Public => (
                State::BeforeDoctypePublicIdentifier,
                ErrorCode::MissingWhitespaceAfterDoctypePublicKeyword,
                ErrorCode::MissingDoctypePublicIdentifier,
                ErrorCode::MissingQuoteBeforeDoctypePublicIdentifier,
            ),
            Identifier::System => (
                State::BeforeDoctypeSystemIdentifier,
                ErrorCode::MissingWhitespaceAfterDoctypeSystemKeyword,
                ErrorCode::MissingDoctypeSystemIdentifier,
                ErrorCode::MissingQuoteBeforeDoctypeSystemIdentifier,
            ),
        };
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => self.state = before,
            Next::Char(quote @ ('"' | '\'')) => {
                self.error(missing_whitespace);
                self.start_doctype_identifier(identifier, quote);
            }
            Next::Char('>') => {
                self.error(missing);
                self.doctype.force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            Next::Eof => self.eof_in_doctype(),
            _ => {
                self.error(missing_quote);
                self.doctype.force_quirks = true;
                self.reconsume(State::BogusDoctype);
            }
        }
    }

    /// „Before DOCTYPE public identifier state“ und „Before DOCTYPE system
    /// identifier state“
    fn before_doctype_identifier(&mut self, identifier: Identifier) {
        let (missing, missing_quote) = match identifier {
            Identifier::Public => (
                ErrorCode::MissingDoctypePublicIdentifier,
                ErrorCode::MissingQuoteBeforeDoctypePublicIdentifier,
            ),
            Identifier::System => (
                ErrorCode::MissingDoctypeSystemIdentifier,
                ErrorCode::MissingQuoteBeforeDoctypeSystemIdentifier,
            ),
        };
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => {}
            Next::Char(quote @ ('"' | '\'')) => self.start_doctype_identifier(identifier, quote),
            Next::Char('>') => {
                self.error(missing);
                self.doctype.force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            Next::Eof => self.eof_in_doctype(),
            _ => {
                self.error(missing_quote);
                self.doctype.force_quirks = true;
                self.reconsume(State::BogusDoctype);
            }
        }
    }

    /// Die vier Zustände „DOCTYPE public/system identifier
    /// (double-/single-quoted) state“
    fn doctype_identifier_quoted(&mut self, identifier: Identifier, quote: char) {
        let (after, abrupt) = match identifier {
            Identifier::Public => (
                State::AfterDoctypePublicIdentifier,
                ErrorCode::AbruptDoctypePublicIdentifier,
            ),
            Identifier::System => (
                State::AfterDoctypeSystemIdentifier,
                ErrorCode::AbruptDoctypeSystemIdentifier,
            ),
        };
        match self.consume() {
            Next::Char(c) if c == quote => self.state = after,
            Next::Char('\0') => {
                self.error(ErrorCode::UnexpectedNullCharacter);
                identifier.field(&mut self.doctype).push('\u{FFFD}');
            }
            Next::Char('>') => {
                self.error(abrupt);
                self.doctype.force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            Next::Eof => self.eof_in_doctype(),
            next => push_next(identifier.field(&mut self.doctype), next),
        }
    }

    /// „After DOCTYPE public identifier state“
    fn after_doctype_public_identifier(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => {
                self.state = State::BetweenDoctypePublicAndSystemIdentifiers;
            }
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Next::Char(quote @ ('"' | '\'')) => {
                self.error(ErrorCode::MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers);
                self.start_doctype_identifier(Identifier::System, quote);
            }
            Next::Eof => self.eof_in_doctype(),
            _ => {
                self.error(ErrorCode::MissingQuoteBeforeDoctypeSystemIdentifier);
                self.doctype.force_quirks = true;
                self.reconsume(State::BogusDoctype);
            }
        }
    }

    /// „Between DOCTYPE public and system identifiers state“
    fn between_doctype_public_and_system_identifiers(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => {}
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Next::Char(quote @ ('"' | '\'')) => {
                self.start_doctype_identifier(Identifier::System, quote);
            }
            Next::Eof => self.eof_in_doctype(),
            _ => {
                self.error(ErrorCode::MissingQuoteBeforeDoctypeSystemIdentifier);
                self.doctype.force_quirks = true;
                self.reconsume(State::BogusDoctype);
            }
        }
    }

    /// „After DOCTYPE system identifier state“
    fn after_doctype_system_identifier(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => {}
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Next::Eof => self.eof_in_doctype(),
            _ => {
                // Setzt den force-quirks-Schalter ausdrücklich nicht.
                self.error(ErrorCode::UnexpectedCharacterAfterDoctypeSystemIdentifier);
                self.reconsume(State::BogusDoctype);
            }
        }
    }

    /// „Bogus DOCTYPE state“
    fn bogus_doctype(&mut self) {
        match self.consume() {
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Next::Char('\0') => self.error(ErrorCode::UnexpectedNullCharacter),
            Next::Eof => {
                self.emit_doctype();
                self.emit_eof();
            }
            _ => {}
        }
    }

    // CDATA

    /// „CDATA section state“
    fn cdata_section(&mut self) {
        match self.consume() {
            Next::Char(']') => self.state = State::CdataSectionBracket,
            Next::Eof => {
                self.error(ErrorCode::EofInCdata);
                self.emit_eof();
            }
            next => self.emit_next(next),
        }
    }

    /// „CDATA section bracket state“
    fn cdata_section_bracket(&mut self) {
        match self.consume() {
            Next::Char(']') => self.state = State::CdataSectionEnd,
            _ => {
                self.emit_char(']');
                self.reconsume(State::CdataSection);
            }
        }
    }

    /// „CDATA section end state“
    fn cdata_section_end(&mut self) {
        match self.consume() {
            Next::Char(']') => self.emit_char(']'),
            Next::Char('>') => self.state = State::Data,
            _ => {
                self.emit_str("]]");
                self.reconsume(State::CdataSection);
            }
        }
    }

    // Verarbeitungsanweisungen

    /// „Processing instruction open state“. Jeder Fall verbraucht das
    /// Zeichen erneut; hier wird es deshalb nur angesehen. So stehen die
    /// Fehler des Eingabestroms für dieses Zeichen hinter denen des Zustands,
    /// wie in den html5lib-Tests.
    fn processing_instruction_open(&mut self) {
        match self.input.get(self.pos).map(|cp| cp.to_char()) {
            Some(Some(c)) if c.is_ascii_alphabetic() || c == '_' => {
                self.state = State::ProcessingInstructionTarget;
            }
            None => {
                self.error_ahead(ErrorCode::EofInProcessingInstruction);
                self.consume();
                self.emit_eof();
            }
            Some(_) => {
                self.error_ahead(ErrorCode::InvalidFirstCharacterOfProcessingInstructionTarget);
                self.temporary_buffer_to_comment();
                self.state = State::BogusComment;
            }
        }
    }

    /// „Processing instruction target state“
    fn processing_instruction_target(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ' | '?' | '>') => {
                let target = &self.temporary_buffer;
                if target.eq_ignore_ascii_case("xml")
                    || target.eq_ignore_ascii_case("xml-stylesheet")
                {
                    self.error(ErrorCode::DisallowedProcessingInstructionTarget);
                    self.temporary_buffer_to_comment();
                    self.reconsume(State::BogusComment);
                } else {
                    self.processing_instruction =
                        (DomString::from(target.as_str()), DomString::new());
                    self.processing_instruction_start = self.pos.saturating_sub(1);
                    self.reconsume(State::AfterProcessingInstructionTarget);
                }
            }
            Next::Char(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_' => {
                self.temporary_buffer.push(c);
            }
            Next::Eof => {
                self.error(ErrorCode::EofInProcessingInstruction);
                self.emit_eof();
            }
            _ => {
                self.error(ErrorCode::InvalidProcessingInstructionTarget);
                self.temporary_buffer_to_comment();
                self.reconsume(State::BogusComment);
            }
        }
    }

    /// „After processing instruction target state“
    fn after_processing_instruction_target(&mut self) {
        match self.consume() {
            Next::Char('\t' | '\n' | '\x0C' | ' ') => {}
            _ => self.reconsume(State::ProcessingInstructionData),
        }
    }

    /// Der Fall „EOF“, nachdem das Token entstanden ist. Die html5lib-Tests
    /// erwarten den Fehler an der Stelle, an der das Token entstand.
    fn eof_in_processing_instruction(&mut self) {
        self.report_at(
            Some(ErrorCode::EofInProcessingInstruction),
            self.processing_instruction_start,
        );
        self.emit_eof();
    }

    /// „Processing instruction data state“
    fn processing_instruction_data(&mut self) {
        match self.consume() {
            Next::Char('?') => self.state = State::ProcessingInstructionQuestionable,
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_processing_instruction();
            }
            Next::Eof => self.eof_in_processing_instruction(),
            next => push_next(&mut self.processing_instruction.1, next),
        }
    }

    /// „Processing instruction questionable state“
    fn processing_instruction_questionable(&mut self) {
        match self.consume() {
            Next::Char('>') => {
                self.state = State::Data;
                self.emit_processing_instruction();
            }
            Next::Eof => self.eof_in_processing_instruction(),
            _ => {
                self.processing_instruction.1.push('?');
                self.reconsume(State::ProcessingInstructionData);
            }
        }
    }

    // Zeichenreferenzen

    /// „Character reference state“
    fn character_reference(&mut self) {
        self.temporary_buffer.clear();
        self.temporary_buffer.push('&');
        match self.consume() {
            Next::Char(c) if c.is_ascii_alphanumeric() => {
                self.reconsume(State::NamedCharacterReference);
            }
            Next::Char('#') => {
                self.temporary_buffer.push('#');
                self.state = State::NumericCharacterReference;
            }
            _ => {
                self.flush_character_reference();
                let state = self.return_state;
                self.reconsume(state);
            }
        }
    }

    /// „Named character reference state“
    fn named_character_reference(&mut self) {
        let rest = self.input.get(self.pos..).unwrap_or_default();
        let Some((length, characters)) = entities::longest_match(rest) else {
            self.flush_character_reference();
            self.state = State::AmbiguousAmpersand;
            return;
        };
        for code_point in rest.iter().take(length) {
            if let Some(c) = code_point.to_char() {
                self.temporary_buffer.push(c);
            }
        }
        self.advance(length);
        let last_is_semicolon = self.temporary_buffer.ends_with(';');
        let next = self.input.get(self.pos).and_then(|cp| cp.to_char());
        let historical = self.in_attribute()
            && !last_is_semicolon
            && next.is_some_and(|c| c == '=' || c.is_ascii_alphanumeric());
        if !historical {
            if !last_is_semicolon {
                self.error_ahead(ErrorCode::MissingSemicolonAfterCharacterReference);
            }
            self.temporary_buffer.clear();
            self.temporary_buffer.push_str(characters);
        }
        self.flush_character_reference();
        self.state = self.return_state;
    }

    /// „Ambiguous ampersand state“
    fn ambiguous_ampersand(&mut self) {
        match self.consume() {
            Next::Char(c) if c.is_ascii_alphanumeric() => {
                if self.in_attribute() {
                    if let Some(value) = self.attribute_value() {
                        value.push(c);
                    }
                } else {
                    self.emit_char(c);
                }
            }
            Next::Char(';') => {
                self.error(ErrorCode::UnknownNamedCharacterReference);
                let state = self.return_state;
                self.reconsume(state);
            }
            _ => {
                let state = self.return_state;
                self.reconsume(state);
            }
        }
    }

    /// „Numeric character reference state“
    fn numeric_character_reference(&mut self) {
        self.character_reference_code = 0;
        match self.consume() {
            Next::Char(c @ ('x' | 'X')) => {
                self.temporary_buffer.push(c);
                self.state = State::HexadecimalCharacterReferenceStart;
            }
            Next::Char(c) if c.is_ascii_digit() => {
                self.reconsume(State::DecimalCharacterReference);
            }
            _ => {
                self.error(ErrorCode::AbsenceOfDigitsInNumericCharacterReference);
                self.flush_character_reference();
                let state = self.return_state;
                self.reconsume(state);
            }
        }
    }

    /// „Hexadecimal character reference start state“
    fn hexadecimal_character_reference_start(&mut self) {
        match self.consume() {
            Next::Char(c) if c.is_ascii_hexdigit() => {
                self.reconsume(State::HexadecimalCharacterReference);
            }
            _ => {
                self.error(ErrorCode::AbsenceOfDigitsInNumericCharacterReference);
                self.flush_character_reference();
                let state = self.return_state;
                self.reconsume(state);
            }
        }
    }

    /// „Hexadecimal character reference state“. Der Wert sättigt; alles über
    /// 0x10FFFF wird ohnehin gleich behandelt.
    fn hexadecimal_character_reference(&mut self) {
        match self.consume() {
            Next::Char(c) if c.is_ascii_hexdigit() => {
                let digit = c.to_digit(16).unwrap_or(0);
                self.character_reference_code = self
                    .character_reference_code
                    .saturating_mul(16)
                    .saturating_add(digit);
            }
            Next::Char(';') => self.state = State::NumericCharacterReferenceEnd,
            _ => {
                self.error(ErrorCode::MissingSemicolonAfterCharacterReference);
                self.reconsume(State::NumericCharacterReferenceEnd);
            }
        }
    }

    /// „Decimal character reference state“
    fn decimal_character_reference(&mut self) {
        match self.consume() {
            Next::Char(c) if c.is_ascii_digit() => {
                let digit = c.to_digit(10).unwrap_or(0);
                self.character_reference_code = self
                    .character_reference_code
                    .saturating_mul(10)
                    .saturating_add(digit);
            }
            Next::Char(';') => self.state = State::NumericCharacterReferenceEnd,
            _ => {
                self.error(ErrorCode::MissingSemicolonAfterCharacterReference);
                self.reconsume(State::NumericCharacterReferenceEnd);
            }
        }
    }

    /// „Numeric character reference end state“. Der Zustand verbraucht
    /// nichts; die Fehler stehen am nächsten Eingabezeichen.
    fn numeric_character_reference_end(&mut self) {
        let mut code = self.character_reference_code;
        if code == 0 {
            self.error_ahead(ErrorCode::NullCharacterReference);
            code = 0xFFFD;
        } else if code > 0x10_FFFF {
            self.error_ahead(ErrorCode::CharacterReferenceOutsideUnicodeRange);
            code = 0xFFFD;
        } else if (0xD800..=0xDFFF).contains(&code) {
            self.error_ahead(ErrorCode::SurrogateCharacterReference);
            code = 0xFFFD;
        }
        if is_noncharacter(code) {
            self.error_ahead(ErrorCode::NoncharacterCharacterReference);
        }
        if code == 0x0D || (is_control(code) && !is_ascii_whitespace(code)) {
            self.error_ahead(ErrorCode::ControlCharacterReference);
            if let Some(&(_, replacement)) = C1_REPLACEMENTS.iter().find(|&&(from, _)| from == code)
            {
                code = u32::from(replacement);
            }
        }
        self.temporary_buffer.clear();
        self.temporary_buffer
            .push(char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER));
        self.flush_character_reference();
        self.state = self.return_state;
    }
}

/// Welcher Bezeichner eines DOCTYPE gemeint ist.
#[derive(Clone, Copy)]
enum Identifier {
    Public,
    System,
}

impl Identifier {
    fn field(self, doctype: &mut Doctype) -> &mut DomString {
        match self {
            Identifier::Public => doctype.public_id.get_or_insert_with(DomString::new),
            Identifier::System => doctype.system_id.get_or_insert_with(DomString::new),
        }
    }
}

#[cfg(test)]
mod tests;
