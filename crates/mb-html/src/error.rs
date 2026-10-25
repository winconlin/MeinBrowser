//! Parse-Fehler (HTML Standard, Abschnitt „Parse errors“).
//!
//! Fehler ändern das Ergebnis nicht, der Parser behandelt jeden nach festen
//! Regeln. Gesammelt werden sie nur auf Wunsch, etwa für Tests und
//! Entwicklerwerkzeuge.

use std::fmt;

/// Die Fehlercodes aus der Tabelle des Standards, in deren Reihenfolge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorCode {
    /// `abrupt-closing-of-empty-comment`
    AbruptClosingOfEmptyComment,
    /// `abrupt-doctype-public-identifier`
    AbruptDoctypePublicIdentifier,
    /// `abrupt-doctype-system-identifier`
    AbruptDoctypeSystemIdentifier,
    /// `absence-of-digits-in-numeric-character-reference`
    AbsenceOfDigitsInNumericCharacterReference,
    /// `cdata-in-html-content`
    CdataInHtmlContent,
    /// `character-reference-outside-unicode-range`
    CharacterReferenceOutsideUnicodeRange,
    /// `control-character-in-input-stream`
    ControlCharacterInInputStream,
    /// `control-character-reference`
    ControlCharacterReference,
    /// `disallowed-processing-instruction-target`
    DisallowedProcessingInstructionTarget,
    /// `duplicate-attribute`
    DuplicateAttribute,
    /// `end-tag-with-attributes`
    EndTagWithAttributes,
    /// `end-tag-with-trailing-solidus`
    EndTagWithTrailingSolidus,
    /// `eof-before-tag-name`
    EofBeforeTagName,
    /// `eof-in-cdata`
    EofInCdata,
    /// `eof-in-comment`
    EofInComment,
    /// `eof-in-doctype`
    EofInDoctype,
    /// `eof-in-processing-instruction`
    EofInProcessingInstruction,
    /// `eof-in-script-html-comment-like-text`
    EofInScriptHtmlCommentLikeText,
    /// `eof-in-tag`
    EofInTag,
    /// `incorrectly-closed-comment`
    IncorrectlyClosedComment,
    /// `incorrectly-opened-comment`
    IncorrectlyOpenedComment,
    /// `invalid-character-sequence-after-doctype-name`
    InvalidCharacterSequenceAfterDoctypeName,
    /// `invalid-first-character-of-processing-instruction-target`
    InvalidFirstCharacterOfProcessingInstructionTarget,
    /// `invalid-first-character-of-tag-name`
    InvalidFirstCharacterOfTagName,
    /// `invalid-processing-instruction-target`
    InvalidProcessingInstructionTarget,
    /// `missing-attribute-value`
    MissingAttributeValue,
    /// `missing-doctype-name`
    MissingDoctypeName,
    /// `missing-doctype-public-identifier`
    MissingDoctypePublicIdentifier,
    /// `missing-doctype-system-identifier`
    MissingDoctypeSystemIdentifier,
    /// `missing-end-tag-name`
    MissingEndTagName,
    /// `missing-quote-before-doctype-public-identifier`
    MissingQuoteBeforeDoctypePublicIdentifier,
    /// `missing-quote-before-doctype-system-identifier`
    MissingQuoteBeforeDoctypeSystemIdentifier,
    /// `missing-semicolon-after-character-reference`
    MissingSemicolonAfterCharacterReference,
    /// `missing-whitespace-after-doctype-public-keyword`
    MissingWhitespaceAfterDoctypePublicKeyword,
    /// `missing-whitespace-after-doctype-system-keyword`
    MissingWhitespaceAfterDoctypeSystemKeyword,
    /// `missing-whitespace-before-doctype-name`
    MissingWhitespaceBeforeDoctypeName,
    /// `missing-whitespace-between-attributes`
    MissingWhitespaceBetweenAttributes,
    /// `missing-whitespace-between-doctype-public-and-system-identifiers`
    MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers,
    /// `nested-comment`
    NestedComment,
    /// `noncharacter-character-reference`
    NoncharacterCharacterReference,
    /// `noncharacter-in-input-stream`
    NoncharacterInInputStream,
    /// `non-void-html-element-start-tag-with-trailing-solidus`
    NonVoidHtmlElementStartTagWithTrailingSolidus,
    /// `null-character-reference`
    NullCharacterReference,
    /// `surrogate-character-reference`
    SurrogateCharacterReference,
    /// `surrogate-in-input-stream`
    SurrogateInInputStream,
    /// `unexpected-character-after-doctype-system-identifier`
    UnexpectedCharacterAfterDoctypeSystemIdentifier,
    /// `unexpected-character-in-attribute-name`
    UnexpectedCharacterInAttributeName,
    /// `unexpected-character-in-unquoted-attribute-value`
    UnexpectedCharacterInUnquotedAttributeValue,
    /// `unexpected-equals-sign-before-attribute-name`
    UnexpectedEqualsSignBeforeAttributeName,
    /// `unexpected-null-character`
    UnexpectedNullCharacter,
    /// `unexpected-solidus-in-tag`
    UnexpectedSolidusInTag,
    /// `unknown-named-character-reference`
    UnknownNamedCharacterReference,
}

impl ErrorCode {
    /// Alle Codes in der Reihenfolge der Tabelle.
    pub const ALL: [ErrorCode; 52] = [
        ErrorCode::AbruptClosingOfEmptyComment,
        ErrorCode::AbruptDoctypePublicIdentifier,
        ErrorCode::AbruptDoctypeSystemIdentifier,
        ErrorCode::AbsenceOfDigitsInNumericCharacterReference,
        ErrorCode::CdataInHtmlContent,
        ErrorCode::CharacterReferenceOutsideUnicodeRange,
        ErrorCode::ControlCharacterInInputStream,
        ErrorCode::ControlCharacterReference,
        ErrorCode::DisallowedProcessingInstructionTarget,
        ErrorCode::DuplicateAttribute,
        ErrorCode::EndTagWithAttributes,
        ErrorCode::EndTagWithTrailingSolidus,
        ErrorCode::EofBeforeTagName,
        ErrorCode::EofInCdata,
        ErrorCode::EofInComment,
        ErrorCode::EofInDoctype,
        ErrorCode::EofInProcessingInstruction,
        ErrorCode::EofInScriptHtmlCommentLikeText,
        ErrorCode::EofInTag,
        ErrorCode::IncorrectlyClosedComment,
        ErrorCode::IncorrectlyOpenedComment,
        ErrorCode::InvalidCharacterSequenceAfterDoctypeName,
        ErrorCode::InvalidFirstCharacterOfProcessingInstructionTarget,
        ErrorCode::InvalidFirstCharacterOfTagName,
        ErrorCode::InvalidProcessingInstructionTarget,
        ErrorCode::MissingAttributeValue,
        ErrorCode::MissingDoctypeName,
        ErrorCode::MissingDoctypePublicIdentifier,
        ErrorCode::MissingDoctypeSystemIdentifier,
        ErrorCode::MissingEndTagName,
        ErrorCode::MissingQuoteBeforeDoctypePublicIdentifier,
        ErrorCode::MissingQuoteBeforeDoctypeSystemIdentifier,
        ErrorCode::MissingSemicolonAfterCharacterReference,
        ErrorCode::MissingWhitespaceAfterDoctypePublicKeyword,
        ErrorCode::MissingWhitespaceAfterDoctypeSystemKeyword,
        ErrorCode::MissingWhitespaceBeforeDoctypeName,
        ErrorCode::MissingWhitespaceBetweenAttributes,
        ErrorCode::MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers,
        ErrorCode::NestedComment,
        ErrorCode::NoncharacterCharacterReference,
        ErrorCode::NoncharacterInInputStream,
        ErrorCode::NonVoidHtmlElementStartTagWithTrailingSolidus,
        ErrorCode::NullCharacterReference,
        ErrorCode::SurrogateCharacterReference,
        ErrorCode::SurrogateInInputStream,
        ErrorCode::UnexpectedCharacterAfterDoctypeSystemIdentifier,
        ErrorCode::UnexpectedCharacterInAttributeName,
        ErrorCode::UnexpectedCharacterInUnquotedAttributeValue,
        ErrorCode::UnexpectedEqualsSignBeforeAttributeName,
        ErrorCode::UnexpectedNullCharacter,
        ErrorCode::UnexpectedSolidusInTag,
        ErrorCode::UnknownNamedCharacterReference,
    ];

    /// Der Code, wie er im Standard steht, etwa `eof-in-tag`.
    pub const fn name(self) -> &'static str {
        match self {
            ErrorCode::AbruptClosingOfEmptyComment => "abrupt-closing-of-empty-comment",
            ErrorCode::AbruptDoctypePublicIdentifier => "abrupt-doctype-public-identifier",
            ErrorCode::AbruptDoctypeSystemIdentifier => "abrupt-doctype-system-identifier",
            ErrorCode::AbsenceOfDigitsInNumericCharacterReference => {
                "absence-of-digits-in-numeric-character-reference"
            }
            ErrorCode::CdataInHtmlContent => "cdata-in-html-content",
            ErrorCode::CharacterReferenceOutsideUnicodeRange => {
                "character-reference-outside-unicode-range"
            }
            ErrorCode::ControlCharacterInInputStream => "control-character-in-input-stream",
            ErrorCode::ControlCharacterReference => "control-character-reference",
            ErrorCode::DisallowedProcessingInstructionTarget => {
                "disallowed-processing-instruction-target"
            }
            ErrorCode::DuplicateAttribute => "duplicate-attribute",
            ErrorCode::EndTagWithAttributes => "end-tag-with-attributes",
            ErrorCode::EndTagWithTrailingSolidus => "end-tag-with-trailing-solidus",
            ErrorCode::EofBeforeTagName => "eof-before-tag-name",
            ErrorCode::EofInCdata => "eof-in-cdata",
            ErrorCode::EofInComment => "eof-in-comment",
            ErrorCode::EofInDoctype => "eof-in-doctype",
            ErrorCode::EofInProcessingInstruction => "eof-in-processing-instruction",
            ErrorCode::EofInScriptHtmlCommentLikeText => "eof-in-script-html-comment-like-text",
            ErrorCode::EofInTag => "eof-in-tag",
            ErrorCode::IncorrectlyClosedComment => "incorrectly-closed-comment",
            ErrorCode::IncorrectlyOpenedComment => "incorrectly-opened-comment",
            ErrorCode::InvalidCharacterSequenceAfterDoctypeName => {
                "invalid-character-sequence-after-doctype-name"
            }
            ErrorCode::InvalidFirstCharacterOfProcessingInstructionTarget => {
                "invalid-first-character-of-processing-instruction-target"
            }
            ErrorCode::InvalidFirstCharacterOfTagName => "invalid-first-character-of-tag-name",
            ErrorCode::InvalidProcessingInstructionTarget => {
                "invalid-processing-instruction-target"
            }
            ErrorCode::MissingAttributeValue => "missing-attribute-value",
            ErrorCode::MissingDoctypeName => "missing-doctype-name",
            ErrorCode::MissingDoctypePublicIdentifier => "missing-doctype-public-identifier",
            ErrorCode::MissingDoctypeSystemIdentifier => "missing-doctype-system-identifier",
            ErrorCode::MissingEndTagName => "missing-end-tag-name",
            ErrorCode::MissingQuoteBeforeDoctypePublicIdentifier => {
                "missing-quote-before-doctype-public-identifier"
            }
            ErrorCode::MissingQuoteBeforeDoctypeSystemIdentifier => {
                "missing-quote-before-doctype-system-identifier"
            }
            ErrorCode::MissingSemicolonAfterCharacterReference => {
                "missing-semicolon-after-character-reference"
            }
            ErrorCode::MissingWhitespaceAfterDoctypePublicKeyword => {
                "missing-whitespace-after-doctype-public-keyword"
            }
            ErrorCode::MissingWhitespaceAfterDoctypeSystemKeyword => {
                "missing-whitespace-after-doctype-system-keyword"
            }
            ErrorCode::MissingWhitespaceBeforeDoctypeName => {
                "missing-whitespace-before-doctype-name"
            }
            ErrorCode::MissingWhitespaceBetweenAttributes => {
                "missing-whitespace-between-attributes"
            }
            ErrorCode::MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers => {
                "missing-whitespace-between-doctype-public-and-system-identifiers"
            }
            ErrorCode::NestedComment => "nested-comment",
            ErrorCode::NoncharacterCharacterReference => "noncharacter-character-reference",
            ErrorCode::NoncharacterInInputStream => "noncharacter-in-input-stream",
            ErrorCode::NonVoidHtmlElementStartTagWithTrailingSolidus => {
                "non-void-html-element-start-tag-with-trailing-solidus"
            }
            ErrorCode::NullCharacterReference => "null-character-reference",
            ErrorCode::SurrogateCharacterReference => "surrogate-character-reference",
            ErrorCode::SurrogateInInputStream => "surrogate-in-input-stream",
            ErrorCode::UnexpectedCharacterAfterDoctypeSystemIdentifier => {
                "unexpected-character-after-doctype-system-identifier"
            }
            ErrorCode::UnexpectedCharacterInAttributeName => {
                "unexpected-character-in-attribute-name"
            }
            ErrorCode::UnexpectedCharacterInUnquotedAttributeValue => {
                "unexpected-character-in-unquoted-attribute-value"
            }
            ErrorCode::UnexpectedEqualsSignBeforeAttributeName => {
                "unexpected-equals-sign-before-attribute-name"
            }
            ErrorCode::UnexpectedNullCharacter => "unexpected-null-character",
            ErrorCode::UnexpectedSolidusInTag => "unexpected-solidus-in-tag",
            ErrorCode::UnknownNamedCharacterReference => "unknown-named-character-reference",
        }
    }

    /// Der Code zu einem Namen aus dem Standard.
    pub fn from_name(name: &str) -> Option<ErrorCode> {
        ErrorCode::ALL.into_iter().find(|code| code.name() == name)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Ein Parse-Fehler mit der Stelle im Eingabestrom (nach dem Vereinheitlichen
/// der Zeilenumbrüche). Zeile und Spalte zählen ab 1, die Spalte in
/// UTF-16-Codeeinheiten; das Dateiende steht hinter dem letzten Zeichen.
///
/// Stellen und Reihenfolge legt der Standard nicht fest. Sie folgen den
/// Festlegungen der html5lib-Tests. Fehler beim Baumaufbau haben im Standard
/// keinen Code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub code: Option<ErrorCode>,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(code) => write!(f, "{code}")?,
            None => f.write_str("Fehler beim Baumaufbau")?,
        }
        write!(f, " in Zeile {}, Spalte {}", self.line, self.column)
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for code in ErrorCode::ALL {
            assert_eq!(ErrorCode::from_name(code.name()), Some(code));
        }
        assert_eq!(
            ErrorCode::from_name("eof-in-tag"),
            Some(ErrorCode::EofInTag)
        );
        assert_eq!(ErrorCode::from_name("unbekannt"), None);
    }
}
