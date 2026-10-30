//! Lexer nach ECMA-262, Abschnitt 12 („ECMAScript Language: Lexical
//! Grammar“), mit den HTML-ähnlichen Kommentaren aus Anhang B.1.1.
//!
//! - Die Eingabe sind UTF-16-Codeeinheiten wie in JavaScript-Zeichenketten.
//!   Ein Surrogatpaar ist ein Codepunkt, ein einzelnes Surrogat ebenfalls
//!   (Abschnitt 11.1). Lagen zählen in Codeeinheiten.
//! - Welche Tokens an einer Stelle möglich sind, hängt vom Syntaxbaum ab,
//!   etwa ob `/` eine Division oder einen regulären Ausdruck beginnt. Der
//!   Parser nennt deshalb bei jedem Aufruf das Ziel der lexikalischen
//!   Grammatik ([`Goal`]).
//! - Was nur der Parser beurteilen kann, gibt der Lexer als Merkmal mit:
//!   Escapes in Namen (reservierte Wörter), Altformen von Zahlen und
//!   Escapes, die im strikten Modus Fehler sind, und Templates mit
//!   ungültigen Escapes, die nur mit Tag erlaubt sind.
//! - Zahlen werden exakt gerundet. Das ist bei mehr als 20 signifikanten
//!   Ziffern immer eine der beiden Wahlen, die RoundMVResult erlaubt.
//! - Den Rumpf regulärer Ausdrücke prüft erst ihr eigener Parser.

#![forbid(unsafe_code)]

use std::ops::Range;

mod number;
#[cfg(test)]
mod tests;

/// Das Ziel der lexikalischen Grammatik (Abschnitt 12, Einleitung).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    /// *InputElementDiv*: `/` ist eine Division, `}` schließt einen Block.
    Div,
    /// *InputElementRegExp*: `/` beginnt einen regulären Ausdruck.
    RegExp,
    /// *InputElementRegExpOrTemplateTail*: wie `RegExp`, aber `}` setzt
    /// ein Template fort.
    RegExpOrTemplateTail,
    /// *InputElementTemplateTail*: `/` ist eine Division, `}` setzt ein
    /// Template fort.
    TemplateTail,
    /// *InputElementHashbangOrRegExp*, am Anfang von Script und Module:
    /// wie `RegExp`, dazu ein Hashbang-Kommentar und in Scripts `-->`.
    HashbangOrRegExp,
}

impl Goal {
    fn regexp(self) -> bool {
        matches!(
            self,
            Goal::RegExp | Goal::RegExpOrTemplateTail | Goal::HashbangOrRegExp
        )
    }

    fn division(self) -> bool {
        matches!(self, Goal::Div | Goal::TemplateTail)
    }

    fn right_brace(self) -> bool {
        matches!(self, Goal::Div | Goal::RegExp)
    }

    fn template_tail(self) -> bool {
        matches!(self, Goal::RegExpOrTemplateTail | Goal::TemplateTail)
    }
}

/// Ein Token mit seiner Lage.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    /// Lage in der Eingabe, in Codeeinheiten.
    pub range: Range<usize>,
    /// Ob vor dem Token ein Zeilenende steht, auch eines in einem
    /// Kommentar. Das brauchen die automatische Semikolon-Einfügung und die
    /// Stellen mit „[no LineTerminator here]“.
    pub newline_before: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    /// *IdentifierName*, auch reservierte Wörter.
    Name(Name),
    /// *PrivateIdentifier*, der Name nach `#`.
    PrivateName(Name),
    Punctuator(Punctuator),
    /// *NumericLiteral* mit einem Number-Wert.
    Number(Number),
    /// *NumericLiteral* mit dem Suffix `n`.
    BigInt(BigInt),
    String(StringLiteral),
    /// *Template* oder *TemplateSubstitutionTail*.
    Template(Template),
    /// *RegularExpressionLiteral*
    RegExp(RegExp),
    /// Das Ende der Eingabe. Danach liefert der Lexer es immer wieder.
    End,
}

/// Ein Name aus Codepunkten (IdentifierCodePoints). Surrogate kommen darin
/// nicht vor, denn sie haben weder ID_Start noch ID_Continue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub value: String,
    /// Ob ein Codepunkt als `\u`-Escape geschrieben war. Dann ist der Name
    /// kein Schlüsselwort (Abschnitt 12.7.2).
    pub escaped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number {
    pub value: f64,
    /// *LegacyOctalIntegerLiteral* oder *NonOctalDecimalIntegerLiteral*,
    /// im strikten Modus ein Fehler (Abschnitt 12.9.3.1).
    pub legacy: bool,
}

/// Eine BigInt-Zahl als Ziffern. Den Wert berechnet erst die Laufzeit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigInt {
    /// 2, 8, 10 oder 16.
    pub radix: u32,
    /// Die Ziffern in ASCII, ohne Präfix, Trennzeichen und Suffix.
    pub digits: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringLiteral {
    /// Der Wert (SV) als UTF-16-Codeeinheiten.
    pub value: Vec<u16>,
    /// Ob ein *LegacyOctalEscapeSequence* oder *NonOctalDecimalEscapeSequence*
    /// vorkommt, im strikten Modus ein Fehler (Abschnitt 12.9.4.1).
    pub legacy_escape: bool,
}

/// Welcher Teil eines Templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplatePart {
    /// *NoSubstitutionTemplate*: `` `…` ``
    NoSubstitution,
    /// *TemplateHead*: `` `…${ ``
    Head,
    /// *TemplateMiddle*: `}…${`
    Middle,
    /// *TemplateTail*: `` }…` ``
    Tail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub part: TemplatePart,
    /// Der Wert (TV); `None` (*undefined*) bei einem *NotEscapeSequence*,
    /// das nur in Templates mit Tag erlaubt ist.
    pub cooked: Option<Vec<u16>>,
    /// Der Rohwert (TRV): der Quelltext mit CR und CR LF als LF.
    pub raw: Vec<u16>,
}

/// Ein regulärer Ausdruck als Quelltext (BodyText und FlagText).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegExp {
    pub body: Vec<u16>,
    pub flags: Vec<u16>,
}

/// *Punctuator*, *DivPunctuator* und *RightBracePunctuator* (Abschnitt 12.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Punctuator {
    LeftBrace,
    RightBrace,
    LeftParenthesis,
    RightParenthesis,
    LeftBracket,
    RightBracket,
    Dot,
    Ellipsis,
    Semicolon,
    Comma,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    Equal,
    NotEqual,
    StrictEqual,
    StrictNotEqual,
    Plus,
    Minus,
    Star,
    Percent,
    StarStar,
    PlusPlus,
    MinusMinus,
    ShiftLeft,
    ShiftRight,
    UnsignedShiftRight,
    Ampersand,
    Bar,
    Caret,
    Bang,
    Tilde,
    AmpersandAmpersand,
    BarBar,
    QuestionQuestion,
    Question,
    QuestionDot,
    Colon,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    PercentAssign,
    StarStarAssign,
    ShiftLeftAssign,
    ShiftRightAssign,
    UnsignedShiftRightAssign,
    AmpersandAssign,
    BarAssign,
    CaretAssign,
    AmpersandAmpersandAssign,
    BarBarAssign,
    QuestionQuestionAssign,
    Arrow,
    Slash,
    SlashAssign,
}

/// Die Zeichen jedes *OtherPunctuator*, längere vor ihren Anfängen, damit
/// der erste Treffer der längste ist. `?.`, `/`, `/=` und `}` behandelt
/// der Lexer selbst.
const PUNCTUATORS: [(&str, Punctuator); 53] = [
    (">>>=", Punctuator::UnsignedShiftRightAssign),
    ("...", Punctuator::Ellipsis),
    ("===", Punctuator::StrictEqual),
    ("!==", Punctuator::StrictNotEqual),
    ("**=", Punctuator::StarStarAssign),
    ("<<=", Punctuator::ShiftLeftAssign),
    (">>=", Punctuator::ShiftRightAssign),
    (">>>", Punctuator::UnsignedShiftRight),
    ("&&=", Punctuator::AmpersandAmpersandAssign),
    ("||=", Punctuator::BarBarAssign),
    ("??=", Punctuator::QuestionQuestionAssign),
    ("<=", Punctuator::LessEqual),
    (">=", Punctuator::GreaterEqual),
    ("==", Punctuator::Equal),
    ("!=", Punctuator::NotEqual),
    ("**", Punctuator::StarStar),
    ("++", Punctuator::PlusPlus),
    ("--", Punctuator::MinusMinus),
    ("<<", Punctuator::ShiftLeft),
    (">>", Punctuator::ShiftRight),
    ("&&", Punctuator::AmpersandAmpersand),
    ("||", Punctuator::BarBar),
    ("??", Punctuator::QuestionQuestion),
    ("+=", Punctuator::PlusAssign),
    ("-=", Punctuator::MinusAssign),
    ("*=", Punctuator::StarAssign),
    ("%=", Punctuator::PercentAssign),
    ("&=", Punctuator::AmpersandAssign),
    ("|=", Punctuator::BarAssign),
    ("^=", Punctuator::CaretAssign),
    ("=>", Punctuator::Arrow),
    ("{", Punctuator::LeftBrace),
    ("(", Punctuator::LeftParenthesis),
    (")", Punctuator::RightParenthesis),
    ("[", Punctuator::LeftBracket),
    ("]", Punctuator::RightBracket),
    (".", Punctuator::Dot),
    (";", Punctuator::Semicolon),
    (",", Punctuator::Comma),
    ("<", Punctuator::Less),
    (">", Punctuator::Greater),
    ("+", Punctuator::Plus),
    ("-", Punctuator::Minus),
    ("*", Punctuator::Star),
    ("%", Punctuator::Percent),
    ("&", Punctuator::Ampersand),
    ("|", Punctuator::Bar),
    ("^", Punctuator::Caret),
    ("!", Punctuator::Bang),
    ("~", Punctuator::Tilde),
    ("?", Punctuator::Question),
    (":", Punctuator::Colon),
    ("=", Punctuator::Assign),
];

/// Ein lexikalischer Fehler, ein früher Fehler (*SyntaxError*) des Scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    /// Wo der Fehler beginnt, in Codeeinheiten.
    pub position: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Ein Codepunkt, mit dem kein Token des Ziels beginnt.
    UnexpectedCharacter,
    UnterminatedComment,
    UnterminatedString,
    UnterminatedTemplate,
    UnterminatedRegExp,
    /// Ein ungültiges Escape in einer Zeichenkette oder einem Namen, oder
    /// eines, das einen im Namen nicht erlaubten Codepunkt ergibt.
    InvalidEscape,
    /// Eine Zahl ohne Ziffern, mit falsch gesetztem `_` oder mit einem
    /// Namen oder einer Ziffer direkt danach (Abschnitt 12.9.3).
    InvalidNumber,
}

const TAB: u32 = 0x09;
const LF: u32 = 0x0A;
const VT: u32 = 0x0B;
const FF: u32 = 0x0C;
const CR: u32 = 0x0D;
const LS: u32 = 0x2028;
const PS: u32 = 0x2029;
const ZWNBSP: u32 = 0xFEFF;
const BACKSLASH: u32 = 0x5C;

/// Der Lexer. Er lässt sich kopieren, damit der Parser an eine frühere
/// Stelle zurückkehren kann.
#[derive(Clone, Debug)]
pub struct Lexer<'a> {
    source: &'a [u16],
    position: usize,
    /// Mit dem Ziel *Module* gibt es keine HTML-ähnlichen Kommentare.
    module: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a [u16], module: bool) -> Lexer<'a> {
        Lexer {
            source,
            position: 0,
            module,
        }
    }

    /// Wo das nächste Token gesucht wird, in Codeeinheiten.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Das nächste Token für das Ziel `goal`. Nach einem Fehler ist die
    /// Position nicht festgelegt.
    pub fn next_token(&mut self, goal: Goal) -> Result<Token, Error> {
        let newline_before = self.skip_trivia(goal)?;
        let start = self.position;
        let kind = match self.code_point(0) {
            None => TokenKind::End,
            Some(c) => self.token(goal, c)?,
        };
        Ok(Token {
            kind,
            range: start..self.position,
            newline_before,
        })
    }

    fn token(&mut self, goal: Goal, c: u32) -> Result<TokenKind, Error> {
        let start = self.position;
        if c == u32::from(b'`') {
            return self.template(false).map(TokenKind::Template);
        }
        if c == u32::from(b'}') {
            if goal.template_tail() {
                return self.template(true).map(TokenKind::Template);
            }
            if goal.right_brace() {
                self.advance(1);
                return Ok(TokenKind::Punctuator(Punctuator::RightBrace));
            }
            return Err(self.error(ErrorKind::UnexpectedCharacter, start));
        }
        if c == u32::from(b'/') {
            if goal.regexp() {
                return self.regexp().map(TokenKind::RegExp);
            }
            if goal.division() {
                let assign = self.is(1, b'=');
                self.advance(if assign { 2 } else { 1 });
                return Ok(TokenKind::Punctuator(if assign {
                    Punctuator::SlashAssign
                } else {
                    Punctuator::Slash
                }));
            }
        }
        if c == u32::from(b'"') || c == u32::from(b'\'') {
            return self.string(c).map(TokenKind::String);
        }
        if is_decimal_digit(Some(c)) || (c == u32::from(b'.') && is_decimal_digit(self.unit(1))) {
            return self.numeric_literal();
        }
        if c == u32::from(b'#') {
            self.advance(1);
            if !self.at_identifier_start() {
                return Err(self.error(ErrorKind::UnexpectedCharacter, start));
            }
            return self.identifier_name().map(TokenKind::PrivateName);
        }
        if self.at_identifier_start() {
            return self.identifier_name().map(TokenKind::Name);
        }
        if c == u32::from(b'?') && self.is(1, b'.') && !is_decimal_digit(self.unit(2)) {
            self.advance(2);
            return Ok(TokenKind::Punctuator(Punctuator::QuestionDot));
        }
        for (text, punctuator) in PUNCTUATORS {
            if self.starts_with(text) {
                self.advance(text.len());
                return Ok(TokenKind::Punctuator(punctuator));
            }
        }
        Err(self.error(ErrorKind::UnexpectedCharacter, start))
    }

    // Eingabe

    /// Die Codeeinheit `offset` Stellen nach der Position.
    fn unit(&self, offset: usize) -> Option<u32> {
        self.source
            .get(self.position.saturating_add(offset))
            .copied()
            .map(u32::from)
    }

    /// Ob an `offset` das ASCII-Zeichen `ascii` steht.
    fn is(&self, offset: usize, ascii: u8) -> bool {
        self.unit(offset) == Some(u32::from(ascii))
    }

    fn starts_with(&self, ascii: &str) -> bool {
        ascii
            .bytes()
            .enumerate()
            .all(|(offset, byte)| self.is(offset, byte))
    }

    /// Der Codepunkt ab `offset` und wie viele Codeeinheiten er belegt.
    fn code_point_with_width(&self, offset: usize) -> Option<(u32, usize)> {
        let first = self.unit(offset)?;
        if (0xD800..0xDC00).contains(&first)
            && let Some(second) = self.unit(offset.saturating_add(1))
            && (0xDC00..0xE000).contains(&second)
        {
            let value = 0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00);
            return Some((value, 2));
        }
        Some((first, 1))
    }

    fn code_point(&self, offset: usize) -> Option<u32> {
        self.code_point_with_width(offset)
            .map(|(code_point, _)| code_point)
    }

    fn advance(&mut self, count: usize) {
        self.position = self.position.saturating_add(count).min(self.source.len());
    }

    /// Rückt um einen Codepunkt vor.
    fn advance_code_point(&mut self) {
        let width = self.code_point_with_width(0).map_or(0, |(_, width)| width);
        self.advance(width);
    }

    fn error(&self, kind: ErrorKind, position: usize) -> Error {
        Error { kind, position }
    }

    // Leerraum und Kommentare (Abschnitte 12.2 bis 12.5, Anhang B.1.1)

    /// Überspringt Leerraum, Zeilenenden und Kommentare und sagt, ob ein
    /// Zeilenende darunter war.
    fn skip_trivia(&mut self, goal: Goal) -> Result<bool, Error> {
        let mut newline = false;
        // Ob `-->` hier einen Kommentar beginnt: nach einem Zeilenende (als
        // *SingleLineHTMLCloseComment* oder nach einem *MultiLineComment*
        // mit Zeilenende) und am Anfang des Scripts, wo das Ziel
        // *HTMLCloseComment* enthält. Leerraum und einzeilige `/* */`
        // dürfen davor stehen.
        let mut line_start = self.position == 0 && goal == Goal::HashbangOrRegExp;
        if line_start && self.starts_with("#!") {
            self.skip_to_line_end();
        }
        while let Some(c) = self.code_point(0) {
            if is_white_space(c) {
                self.advance_code_point();
            } else if is_line_terminator(c) {
                newline = true;
                line_start = true;
                self.advance(1);
            } else if self.starts_with("/*") {
                let start = self.position;
                self.advance(2);
                let mut multiline = false;
                loop {
                    if self.starts_with("*/") {
                        self.advance(2);
                        break;
                    }
                    let Some(c) = self.code_point(0) else {
                        return Err(self.error(ErrorKind::UnterminatedComment, start));
                    };
                    multiline |= is_line_terminator(c);
                    self.advance_code_point();
                }
                if multiline {
                    newline = true;
                    line_start = true;
                }
            } else if self.starts_with("//")
                || (!self.module
                    && (self.starts_with("<!--") || (line_start && self.starts_with("-->"))))
            {
                self.skip_to_line_end();
            } else {
                break;
            }
        }
        Ok(newline)
    }

    /// Überspringt *SingleLineCommentChars*; das Zeilenende bleibt stehen.
    fn skip_to_line_end(&mut self) {
        while let Some(c) = self.code_point(0) {
            if is_line_terminator(c) {
                break;
            }
            self.advance_code_point();
        }
    }

    // Namen (Abschnitt 12.7)

    /// Ob hier ein *IdentifierStart* steht, als Zeichen oder als Escape.
    fn at_identifier_start(&self) -> bool {
        match self.code_point(0) {
            Some(BACKSLASH) => {
                self.is(1, b'u')
                    && self
                        .unicode_escape(2)
                        .is_some_and(|(code_point, _)| is_identifier_start(code_point))
            }
            Some(c) => is_identifier_start(c),
            None => false,
        }
    }

    /// *IdentifierName*. Der Aufrufer hat den Anfang schon geprüft.
    fn identifier_name(&mut self) -> Result<Name, Error> {
        let mut value = String::new();
        let mut escaped = false;
        let mut first = true;
        while let Some((c, width)) = self.code_point_with_width(0) {
            let start = self.position;
            let (code_point, length) = if c == BACKSLASH {
                let escape = if self.is(1, b'u') {
                    self.unicode_escape(2)
                } else {
                    None
                };
                let Some((code_point, length)) = escape else {
                    return Err(self.error(ErrorKind::InvalidEscape, start));
                };
                escaped = true;
                (code_point, length.saturating_add(2))
            } else {
                (c, width)
            };
            let allowed = if first {
                is_identifier_start(code_point)
            } else {
                is_identifier_part(code_point)
            };
            if !allowed {
                if c == BACKSLASH {
                    return Err(self.error(ErrorKind::InvalidEscape, start));
                }
                break;
            }
            let Some(character) = char::from_u32(code_point) else {
                return Err(self.error(ErrorKind::UnexpectedCharacter, start));
            };
            value.push(character);
            self.advance(length);
            first = false;
        }
        Ok(Name { value, escaped })
    }

    /// Der Rest eines *UnicodeEscapeSequence* ab `offset`, also nach `\u`:
    /// der Codepunkt und wie viele Codeeinheiten das Escape nach `\u` belegt.
    fn unicode_escape(&self, offset: usize) -> Option<(u32, usize)> {
        if self.unit(offset) == Some(u32::from(b'{')) {
            let mut value: u32 = 0;
            let mut length = 1;
            loop {
                let unit = self.unit(offset.saturating_add(length));
                if unit == Some(u32::from(b'}')) && length > 1 {
                    return Some((value, length.saturating_add(1)));
                }
                let digit = hex_value(unit)?;
                value = value.checked_mul(16)?.checked_add(digit)?;
                if value > 0x10_FFFF {
                    return None;
                }
                length = length.saturating_add(1);
            }
        }
        let mut value: u32 = 0;
        for index in 0..4 {
            let digit = hex_value(self.unit(offset.saturating_add(index)))?;
            value = value * 16 + digit;
        }
        Some((value, 4))
    }

    // Zahlen (Abschnitt 12.9.3)

    fn numeric_literal(&mut self) -> Result<TokenKind, Error> {
        let start = self.position;
        let radix = if self.is(0, b'0') {
            match self.unit(1).and_then(|unit| u8::try_from(unit).ok()) {
                Some(b'x' | b'X') => Some(16),
                Some(b'o' | b'O') => Some(8),
                Some(b'b' | b'B') => Some(2),
                _ => None,
            }
        } else {
            None
        };
        let kind = if let Some(radix) = radix {
            self.advance(2);
            let mut digits = String::new();
            self.digits(radix, true, &mut digits);
            if digits.is_empty() {
                return Err(self.error(ErrorKind::InvalidNumber, start));
            }
            if self.is(0, b'n') {
                self.advance(1);
                TokenKind::BigInt(BigInt { radix, digits })
            } else {
                TokenKind::Number(Number {
                    value: number::from_power_of_two_radix(&digits, radix),
                    legacy: false,
                })
            }
        } else if self.is(0, b'0') && is_decimal_digit(self.unit(1)) {
            self.legacy_literal()?
        } else {
            self.decimal_literal()?
        };
        // Direkt nach der Zahl darf kein IdentifierStart und keine Ziffer
        // stehen; `\` beginnt hier nie ein gültiges Token.
        let after = self.code_point(0);
        if is_decimal_digit(after)
            || after.is_some_and(|c| c == BACKSLASH || is_identifier_start(c))
        {
            return Err(self.error(ErrorKind::InvalidNumber, start));
        }
        Ok(kind)
    }

    /// *LegacyOctalIntegerLiteral* oder *NonOctalDecimalIntegerLiteral* mit
    /// dem Rest eines *DecimalLiteral*, beide ohne Trennzeichen.
    fn legacy_literal(&mut self) -> Result<TokenKind, Error> {
        let mut digits = String::new();
        self.digits(10, false, &mut digits);
        if digits.bytes().all(|digit| digit < b'8') {
            return Ok(TokenKind::Number(Number {
                value: number::from_power_of_two_radix(&digits, 8),
                legacy: true,
            }));
        }
        let value = self.decimal_rest(digits)?;
        Ok(TokenKind::Number(Number {
            value,
            legacy: true,
        }))
    }

    /// *DecimalLiteral* oder *DecimalBigIntegerLiteral*.
    fn decimal_literal(&mut self) -> Result<TokenKind, Error> {
        let mut integer = String::new();
        if self.is(0, b'0') {
            integer.push('0');
            self.advance(1);
        } else {
            self.digits(10, true, &mut integer);
        }
        if !integer.is_empty() && self.is(0, b'n') {
            self.advance(1);
            return Ok(TokenKind::BigInt(BigInt {
                radix: 10,
                digits: integer,
            }));
        }
        let value = self.decimal_rest(integer)?;
        Ok(TokenKind::Number(Number {
            value,
            legacy: false,
        }))
    }

    /// Nachkommastellen und Exponent nach dem ganzzahligen Teil `integer`
    /// und der Wert der ganzen Zahl.
    fn decimal_rest(&mut self, integer: String) -> Result<f64, Error> {
        let start = self.position;
        let mut text = integer;
        if text.is_empty() {
            text.push('0');
        }
        text.push('.');
        if self.is(0, b'.') {
            self.advance(1);
            self.digits(10, true, &mut text);
        }
        text.push_str("0e");
        if self.is(0, b'e') || self.is(0, b'E') {
            self.advance(1);
            if self.is(0, b'-') {
                text.push('-');
                self.advance(1);
            } else if self.is(0, b'+') {
                self.advance(1);
            }
            let digits = text.len();
            self.digits(10, true, &mut text);
            if text.len() == digits {
                return Err(self.error(ErrorKind::InvalidNumber, start));
            }
        } else {
            text.push('0');
        }
        // Der Text hat immer die Form Ziffern.Ziffern0eZiffern, die Rust
        // exakt rundet; zu große Exponenten ergeben Unendlich.
        text.parse::<f64>()
            .map_err(|_| self.error(ErrorKind::InvalidNumber, start))
    }

    /// Hängt Ziffern zur Basis `radix` an `digits` an. Mit `separators` darf
    /// zwischen zwei Ziffern ein `_` stehen (*NumericLiteralSeparator*).
    /// Ein anderes `_` bleibt stehen, damit es als Fehler auffällt.
    fn digits(&mut self, radix: u32, separators: bool, digits: &mut String) {
        loop {
            let unit = self.unit(0);
            if let Some(digit) = digit_char(unit, radix) {
                digits.push(digit);
                self.advance(1);
            } else if separators
                && unit == Some(u32::from(b'_'))
                && !digits.is_empty()
                && digit_char(self.unit(1), radix).is_some()
                && self.position > 0
                && digit_char(
                    self.source
                        .get(self.position.saturating_sub(1))
                        .copied()
                        .map(u32::from),
                    radix,
                )
                .is_some()
            {
                self.advance(1);
            } else {
                break;
            }
        }
    }

    // Zeichenketten (Abschnitt 12.9.4)

    fn string(&mut self, quote: u32) -> Result<StringLiteral, Error> {
        let start = self.position;
        self.advance(1);
        let mut value = Vec::new();
        let mut legacy_escape = false;
        loop {
            let Some(unit) = self.unit(0) else {
                return Err(self.error(ErrorKind::UnterminatedString, start));
            };
            if unit == quote {
                self.advance(1);
                break;
            }
            if unit == LF || unit == CR {
                return Err(self.error(ErrorKind::UnterminatedString, start));
            }
            if unit == BACKSLASH {
                let escape_start = self.position;
                match self.escape(false) {
                    Escape::Units(units) => value.extend(units),
                    Escape::Legacy(unit) => {
                        value.push(unit);
                        legacy_escape = true;
                    }
                    Escape::LineContinuation => {}
                    Escape::Invalid | Escape::Undefined => {
                        return Err(self.error(ErrorKind::InvalidEscape, escape_start));
                    }
                    Escape::End => {
                        return Err(self.error(ErrorKind::UnterminatedString, start));
                    }
                }
            } else {
                // Codeeinheiten unverändert, auch einzelne Surrogate, das
                // ist UTF16EncodeCodePoint der Codepunkte.
                if let Ok(unit) = u16::try_from(unit) {
                    value.push(unit);
                }
                self.advance(1);
            }
        }
        Ok(StringLiteral {
            value,
            legacy_escape,
        })
    }

    /// Ein Escape ab `\` in einer Zeichenkette oder, mit `template`, in
    /// einem Template (*TemplateEscapeSequence* und *NotEscapeSequence*).
    /// Danach steht die Position hinter dem Escape. Bei *NotEscapeSequence*
    /// rückt sie nur über `\` und den nächsten Codepunkt vor: Was das
    /// Escape sonst noch umfasst, sind gewöhnliche Zeichen des Templates,
    /// und den Rohwert nimmt das Template aus dem Quelltext.
    fn escape(&mut self, template: bool) -> Escape {
        let Some((c, width)) = self.code_point_with_width(1) else {
            return Escape::End;
        };
        let single = |unit: u8| Escape::Units(vec![u16::from(unit)]);
        let escape = match u8::try_from(c).ok() {
            _ if is_line_terminator(c) => {
                let crlf = c == CR && self.unit(2) == Some(LF);
                self.advance(if crlf { 3 } else { 2 });
                return Escape::LineContinuation;
            }
            Some(b'\'') => single(b'\''),
            Some(b'"') => single(b'"'),
            Some(b'\\') => single(b'\\'),
            Some(b'b') => single(0x08),
            Some(b'f') => single(0x0C),
            Some(b'n') => single(0x0A),
            Some(b'r') => single(0x0D),
            Some(b't') => single(0x09),
            Some(b'v') => single(0x0B),
            Some(b'0') if !is_decimal_digit(self.unit(2)) => single(0),
            Some(digit @ b'0'..=b'9') => {
                if template {
                    Escape::Undefined
                } else if digit >= b'8' {
                    // *NonOctalDecimalEscapeSequence*
                    Escape::Legacy(u16::from(digit))
                } else {
                    // *LegacyOctalEscapeSequence*: 0 bis 3 mit bis zu zwei,
                    // 4 bis 7 mit bis zu einer weiteren Oktalziffer.
                    let mut value = u16::from(digit - b'0');
                    let more = if digit <= b'3' { 2 } else { 1 };
                    let mut length = 2;
                    for _ in 0..more {
                        match octal_value(self.unit(length)) {
                            Some(next) => {
                                value = value * 8 + next;
                                length += 1;
                            }
                            None => break,
                        }
                    }
                    self.advance(length);
                    return Escape::Legacy(value);
                }
            }
            Some(b'x') => match (hex_value(self.unit(2)), hex_value(self.unit(3))) {
                (Some(high), Some(low)) => {
                    self.advance(4);
                    return Escape::Units(vec![u16::try_from(high * 16 + low).unwrap_or(0)]);
                }
                _ if template => Escape::Undefined,
                _ => Escape::Invalid,
            },
            Some(b'u') => match self.unicode_escape(2) {
                Some((code_point, length)) => {
                    self.advance(length.saturating_add(2));
                    return Escape::Units(utf16(code_point));
                }
                None if template => Escape::Undefined,
                None => Escape::Invalid,
            },
            // *NonEscapeCharacter*: der Codepunkt selbst.
            _ => {
                let units = (1..=width)
                    .filter_map(|offset| self.unit(offset))
                    .filter_map(|unit| u16::try_from(unit).ok())
                    .collect();
                Escape::Units(units)
            }
        };
        self.advance(width.saturating_add(1));
        escape
    }

    // Templates (Abschnitt 12.9.6)

    /// Ein Template ab `` ` `` oder, mit `continued`, ab `}`.
    fn template(&mut self, continued: bool) -> Result<Template, Error> {
        let start = self.position;
        self.advance(1);
        let characters = self.position;
        let mut cooked = Some(Vec::new());
        let (part, end) = loop {
            let Some(unit) = self.unit(0) else {
                return Err(self.error(ErrorKind::UnterminatedTemplate, start));
            };
            if unit == u32::from(b'`') {
                let end = self.position;
                self.advance(1);
                let part = if continued {
                    TemplatePart::Tail
                } else {
                    TemplatePart::NoSubstitution
                };
                break (part, end);
            }
            if unit == u32::from(b'$') && self.is(1, b'{') {
                let end = self.position;
                self.advance(2);
                let part = if continued {
                    TemplatePart::Middle
                } else {
                    TemplatePart::Head
                };
                break (part, end);
            }
            if unit == BACKSLASH {
                match self.escape(true) {
                    Escape::Units(units) => {
                        if let Some(cooked) = &mut cooked {
                            cooked.extend(units);
                        }
                    }
                    Escape::LineContinuation => {}
                    Escape::Undefined => cooked = None,
                    Escape::Legacy(_) | Escape::Invalid | Escape::End => {
                        return Err(self.error(ErrorKind::UnterminatedTemplate, start));
                    }
                }
                continue;
            }
            let crlf = unit == CR && self.unit(1) == Some(LF);
            let unit = if unit == CR { LF } else { unit };
            if let (Some(cooked), Ok(unit)) = (&mut cooked, u16::try_from(unit)) {
                cooked.push(unit);
            }
            self.advance(if crlf { 2 } else { 1 });
        };
        let raw = normalize_line_ends(self.source.get(characters..end).unwrap_or_default());
        Ok(Template { part, cooked, raw })
    }

    // Reguläre Ausdrücke (Abschnitt 12.9.5)

    /// *RegularExpressionLiteral*. `/*` und `//` sind hier schon als
    /// Kommentare übersprungen.
    fn regexp(&mut self) -> Result<RegExp, Error> {
        let start = self.position;
        self.advance(1);
        let body_start = self.position;
        let mut class = false;
        loop {
            let Some(c) = self.code_point(0) else {
                return Err(self.error(ErrorKind::UnterminatedRegExp, start));
            };
            if is_line_terminator(c) {
                return Err(self.error(ErrorKind::UnterminatedRegExp, start));
            }
            if c == BACKSLASH {
                self.advance(1);
                // *RegularExpressionBackslashSequence*
                match self.code_point(0) {
                    Some(next) if !is_line_terminator(next) => {}
                    _ => return Err(self.error(ErrorKind::UnterminatedRegExp, start)),
                }
            } else if c == u32::from(b'/') && !class {
                break;
            } else if c == u32::from(b'[') {
                class = true;
            } else if c == u32::from(b']') {
                class = false;
            }
            self.advance_code_point();
        }
        let body = self
            .source
            .get(body_start..self.position)
            .unwrap_or_default()
            .to_vec();
        self.advance(1);
        let flags_start = self.position;
        while self.code_point(0).is_some_and(is_identifier_part) {
            self.advance_code_point();
        }
        let flags = self
            .source
            .get(flags_start..self.position)
            .unwrap_or_default()
            .to_vec();
        Ok(RegExp { body, flags })
    }
}

/// Was ein Escape ergibt.
enum Escape {
    Units(Vec<u16>),
    /// Ein Alt-Escape, im strikten Modus ein Fehler.
    Legacy(u16),
    LineContinuation,
    /// Nur in Templates: der Wert ist *undefined*.
    Undefined,
    /// Ein ungültiges Escape in einer Zeichenkette.
    Invalid,
    /// Die Eingabe endet nach `\`.
    End,
}

/// *WhiteSpace* (Abschnitt 12.2).
fn is_white_space(c: u32) -> bool {
    matches!(c, TAB | VT | FF | ZWNBSP)
        || mb_unicode::general_category(c) == mb_unicode::GeneralCategory::SpaceSeparator
}

/// *LineTerminator* (Abschnitt 12.3).
fn is_line_terminator(c: u32) -> bool {
    matches!(c, LF | CR | LS | PS)
}

/// *IdentifierStartChar*
fn is_identifier_start(c: u32) -> bool {
    c == u32::from(b'$') || c == u32::from(b'_') || mb_unicode::is_id_start(c)
}

/// *IdentifierPartChar*
fn is_identifier_part(c: u32) -> bool {
    c == u32::from(b'$') || mb_unicode::is_id_continue(c)
}

fn is_decimal_digit(c: Option<u32>) -> bool {
    c.is_some_and(|c| (u32::from(b'0')..=u32::from(b'9')).contains(&c))
}

/// Die Ziffer zur Basis `radix` als ASCII-Zeichen.
fn digit_char(c: Option<u32>, radix: u32) -> Option<char> {
    let c = char::from_u32(c?)?;
    c.is_digit(radix).then_some(c)
}

fn hex_value(c: Option<u32>) -> Option<u32> {
    char::from_u32(c?)?.to_digit(16)
}

fn octal_value(c: Option<u32>) -> Option<u16> {
    let value = char::from_u32(c?)?.to_digit(8)?;
    u16::try_from(value).ok()
}

/// UTF16EncodeCodePoint
fn utf16(code_point: u32) -> Vec<u16> {
    match char::from_u32(code_point) {
        Some(c) => {
            let mut units = [0; 2];
            c.encode_utf16(&mut units).to_vec()
        }
        // Ein Surrogat bleibt eine Codeeinheit.
        None => u16::try_from(code_point).map_or_else(|_| Vec::new(), |unit| vec![unit]),
    }
}

/// Der Quelltext mit CR LF und CR als LF (TRV von *LineTerminatorSequence*).
fn normalize_line_ends(units: &[u16]) -> Vec<u16> {
    let cr = 0x0D_u16;
    let lf = 0x0A_u16;
    let mut normalized = Vec::with_capacity(units.len());
    let mut previous_cr = false;
    for &unit in units {
        if unit == lf && previous_cr {
            previous_cr = false;
            continue;
        }
        previous_cr = unit == cr;
        normalized.push(if previous_cr { lf } else { unit });
    }
    normalized
}
