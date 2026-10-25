//! Tokenizer nach CSS Syntax Level 3 („Tokenization“).
//!
//! Die Eingabe sind schon gefilterte Codepunkte ([`crate::filter_code_points`]).
//! Jedes Token kommt mit seiner Lage in der Eingabe, gezählt in Codepunkten,
//! damit der Parser den Originaltext von Deklarationen liefern kann. Die
//! Funktionen folgen der Reihenfolge der Spezifikation.

use std::ops::Range;

/// Ein Token (Abschnitt 4). Das ⟨EOF-token⟩ ist kein Wert, sondern das Ende
/// der Liste. Werte sind `String`, denn gefilterte Eingaben enthalten keine
/// Surrogate.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// ⟨ident-token⟩
    Ident(String),
    /// ⟨function-token⟩, der Name vor „(“.
    Function(String),
    /// ⟨at-keyword-token⟩, der Name nach „@“.
    AtKeyword(String),
    /// ⟨hash-token⟩. `id` ist das Typ-Flag „id“, sonst „unrestricted“.
    Hash { value: String, id: bool },
    /// ⟨string-token⟩
    String(String),
    /// ⟨bad-string-token⟩
    BadString,
    /// ⟨url-token⟩
    Url(String),
    /// ⟨bad-url-token⟩
    BadUrl,
    /// ⟨delim-token⟩
    Delim(char),
    /// ⟨number-token⟩. `integer` ist das Typ-Flag „integer“, sonst „number“.
    Number {
        value: f64,
        integer: bool,
        sign: Option<Sign>,
    },
    /// ⟨percentage-token⟩
    Percentage { value: f64, sign: Option<Sign> },
    /// ⟨dimension-token⟩
    Dimension {
        value: f64,
        integer: bool,
        sign: Option<Sign>,
        unit: String,
    },
    /// ⟨unicode-range-token⟩, nur im Wert des Deskriptors `unicode-range`.
    UnicodeRange { start: u32, end: u32 },
    /// ⟨whitespace-token⟩
    Whitespace,
    /// ⟨CDO-token⟩ `<!--`
    Cdo,
    /// ⟨CDC-token⟩ `-->`
    Cdc,
    /// ⟨colon-token⟩
    Colon,
    /// ⟨semicolon-token⟩
    Semicolon,
    /// ⟨comma-token⟩
    Comma,
    /// <[-token>
    OpenBracket,
    /// <]-token>
    CloseBracket,
    /// <(-token>
    OpenParenthesis,
    /// <)-token>
    CloseParenthesis,
    /// <{-token>
    OpenBrace,
    /// <}-token>
    CloseBrace,
}

/// Das Vorzeichen, mit dem eine Zahl geschrieben war.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Plus,
    Minus,
}

/// *Tokenize*: alle Tokens der Eingabe mit ihrer Lage.
pub(crate) fn tokenize(input: &[char], unicode_ranges_allowed: bool) -> Vec<(Token, Range<usize>)> {
    let mut tokenizer = Tokenizer { input, position: 0 };
    let mut tokens = Vec::new();
    while let Some(token) = tokenizer.consume_token(unicode_ranges_allowed) {
        tokens.push(token);
    }
    tokens
}

struct Tokenizer<'a> {
    input: &'a [char],
    position: usize,
}

impl Tokenizer<'_> {
    /// Der Codepunkt `offset` Stellen nach der Position, `None` für EOF.
    /// Statt „consume“ und „reconsume“ schaut der Tokenizer voraus und rückt
    /// erst vor, wenn klar ist, was folgt.
    fn peek(&self, offset: usize) -> Option<char> {
        self.input
            .get(self.position.saturating_add(offset))
            .copied()
    }

    fn advance(&mut self, count: usize) {
        self.position = self.position.saturating_add(count).min(self.input.len());
    }

    /// *Consume a token*. `None` steht für das ⟨EOF-token⟩.
    fn consume_token(&mut self, unicode_ranges_allowed: bool) -> Option<(Token, Range<usize>)> {
        self.consume_comments();
        let start = self.position;
        let current = self.peek(0)?;
        let token = match current {
            '\n' | '\t' | ' ' => {
                self.consume_whitespace();
                Token::Whitespace
            }
            '"' | '\'' => {
                self.advance(1);
                self.consume_string(current)
            }
            '#' => {
                if is_ident_code_point(self.peek(1)) || is_valid_escape(self.peek(1), self.peek(2))
                {
                    let id = would_start_ident(self.peek(1), self.peek(2), self.peek(3));
                    self.advance(1);
                    Token::Hash {
                        value: self.consume_ident_sequence(),
                        id,
                    }
                } else {
                    self.delim(current)
                }
            }
            '(' => self.single(Token::OpenParenthesis),
            ')' => self.single(Token::CloseParenthesis),
            '+' | '.' => {
                if self.starts_number() {
                    self.consume_numeric()
                } else {
                    self.delim(current)
                }
            }
            ',' => self.single(Token::Comma),
            '-' => {
                if self.starts_number() {
                    self.consume_numeric()
                } else if self.peek(1) == Some('-') && self.peek(2) == Some('>') {
                    self.advance(3);
                    Token::Cdc
                } else if would_start_ident(Some(current), self.peek(1), self.peek(2)) {
                    self.consume_ident_like()
                } else {
                    self.delim(current)
                }
            }
            ':' => self.single(Token::Colon),
            ';' => self.single(Token::Semicolon),
            '<' => {
                if self.peek(1) == Some('!')
                    && self.peek(2) == Some('-')
                    && self.peek(3) == Some('-')
                {
                    self.advance(4);
                    Token::Cdo
                } else {
                    self.delim(current)
                }
            }
            '@' => {
                if would_start_ident(self.peek(1), self.peek(2), self.peek(3)) {
                    self.advance(1);
                    Token::AtKeyword(self.consume_ident_sequence())
                } else {
                    self.delim(current)
                }
            }
            '[' => self.single(Token::OpenBracket),
            '\\' => {
                if is_valid_escape(Some(current), self.peek(1)) {
                    self.consume_ident_like()
                } else {
                    // Ein Parse-Fehler.
                    self.delim(current)
                }
            }
            ']' => self.single(Token::CloseBracket),
            '{' => self.single(Token::OpenBrace),
            '}' => self.single(Token::CloseBrace),
            '0'..='9' => self.consume_numeric(),
            'U' | 'u'
                if unicode_ranges_allowed
                    && would_start_unicode_range(Some(current), self.peek(1), self.peek(2)) =>
            {
                self.consume_unicode_range()
            }
            _ if is_ident_start(Some(current)) => self.consume_ident_like(),
            _ => self.delim(current),
        };
        Some((token, start..self.position))
    }

    fn single(&mut self, token: Token) -> Token {
        self.advance(1);
        token
    }

    fn delim(&mut self, current: char) -> Token {
        self.single(Token::Delim(current))
    }

    fn consume_whitespace(&mut self) {
        while is_whitespace(self.peek(0)) {
            self.advance(1);
        }
    }

    /// *The input stream starts with a number*, ab dem aktuellen Codepunkt.
    fn starts_number(&self) -> bool {
        would_start_number(self.peek(0), self.peek(1), self.peek(2))
    }

    /// *Consume comments*. Ein Kommentar ohne Ende ist ein Parse-Fehler.
    fn consume_comments(&mut self) {
        while self.peek(0) == Some('/') && self.peek(1) == Some('*') {
            self.advance(2);
            loop {
                match self.peek(0) {
                    None => return,
                    Some('*') if self.peek(1) == Some('/') => {
                        self.advance(2);
                        break;
                    }
                    Some(_) => self.advance(1),
                }
            }
        }
    }

    /// *Consume a numeric token*.
    fn consume_numeric(&mut self) -> Token {
        let (value, integer, sign) = self.consume_number();
        if would_start_ident(self.peek(0), self.peek(1), self.peek(2)) {
            Token::Dimension {
                value,
                integer,
                sign,
                unit: self.consume_ident_sequence(),
            }
        } else if self.peek(0) == Some('%') {
            self.advance(1);
            Token::Percentage { value, sign }
        } else {
            Token::Number {
                value,
                integer,
                sign,
            }
        }
    }

    /// *Consume an ident-like token*.
    fn consume_ident_like(&mut self) -> Token {
        let string = self.consume_ident_sequence();
        if string.eq_ignore_ascii_case("url") && self.peek(0) == Some('(') {
            self.advance(1);
            while is_whitespace(self.peek(0)) && is_whitespace(self.peek(1)) {
                self.advance(1);
            }
            let quote = |c: Option<char>| matches!(c, Some('"' | '\''));
            if quote(self.peek(0)) || (is_whitespace(self.peek(0)) && quote(self.peek(1))) {
                Token::Function(string)
            } else {
                self.consume_url()
            }
        } else if self.peek(0) == Some('(') {
            self.advance(1);
            Token::Function(string)
        } else {
            Token::Ident(string)
        }
    }

    /// *Consume a string token*, nach dem öffnenden `ending`.
    fn consume_string(&mut self, ending: char) -> Token {
        let mut value = String::new();
        loop {
            match self.peek(0) {
                // Ein Parse-Fehler.
                None => return Token::String(value),
                Some(c) if c == ending => {
                    self.advance(1);
                    return Token::String(value);
                }
                // Ein Parse-Fehler; der Zeilenumbruch bleibt in der Eingabe.
                Some('\n') => return Token::BadString,
                Some('\\') => {
                    self.advance(1);
                    match self.peek(0) {
                        None => {}
                        Some('\n') => self.advance(1),
                        Some(_) => value.push(self.consume_escaped()),
                    }
                }
                Some(c) => {
                    self.advance(1);
                    value.push(c);
                }
            }
        }
    }

    /// *Consume a url token*, nach `url(` und Leerraum.
    fn consume_url(&mut self) -> Token {
        let mut value = String::new();
        self.consume_whitespace();
        loop {
            // EOF ist ein Parse-Fehler.
            let Some(current) = self.peek(0) else {
                return Token::Url(value);
            };
            self.advance(1);
            match current {
                ')' => return Token::Url(value),
                '\n' | '\t' | ' ' => {
                    self.consume_whitespace();
                    return match self.peek(0) {
                        Some(')') => {
                            self.advance(1);
                            Token::Url(value)
                        }
                        // Ein Parse-Fehler.
                        None => Token::Url(value),
                        Some(_) => {
                            self.consume_bad_url_remnants();
                            Token::BadUrl
                        }
                    };
                }
                // Parse-Fehler.
                '"' | '\'' | '(' => {
                    self.consume_bad_url_remnants();
                    return Token::BadUrl;
                }
                _ if is_non_printable(current) => {
                    self.consume_bad_url_remnants();
                    return Token::BadUrl;
                }
                '\\' => {
                    if is_valid_escape(Some(current), self.peek(0)) {
                        value.push(self.consume_escaped());
                    } else {
                        // Ein Parse-Fehler.
                        self.consume_bad_url_remnants();
                        return Token::BadUrl;
                    }
                }
                _ => value.push(current),
            }
        }
    }

    /// *Consume an escaped code point*, nach dem `\`.
    fn consume_escaped(&mut self) -> char {
        // EOF ist ein Parse-Fehler.
        let Some(current) = self.peek(0) else {
            return '\u{FFFD}';
        };
        self.advance(1);
        let Some(mut value) = current.to_digit(16) else {
            return current;
        };
        let mut digits = 1;
        while digits < 6
            && let Some(digit) = self.peek(0).and_then(|c| c.to_digit(16))
        {
            value = value.saturating_mul(16).saturating_add(digit);
            digits += 1;
            self.advance(1);
        }
        if is_whitespace(self.peek(0)) {
            self.advance(1);
        }
        if value == 0 || (0xD800..=0xDFFF).contains(&value) {
            return '\u{FFFD}';
        }
        // Über U+10FFFF liefert `from_u32` nichts.
        char::from_u32(value).unwrap_or('\u{FFFD}')
    }

    /// *Consume an ident sequence*.
    fn consume_ident_sequence(&mut self) -> String {
        let mut result = String::new();
        loop {
            match self.peek(0) {
                Some(c) if is_ident_code_point(Some(c)) => {
                    self.advance(1);
                    result.push(c);
                }
                Some('\\') if is_valid_escape(Some('\\'), self.peek(1)) => {
                    self.advance(1);
                    result.push(self.consume_escaped());
                }
                _ => return result,
            }
        }
    }

    /// *Consume a number*: Wert, Typ-Flag „integer“ und Vorzeichen.
    fn consume_number(&mut self) -> (f64, bool, Option<Sign>) {
        let mut integer = true;
        let mut number = String::new();
        let mut exponent = String::new();
        let mut sign = None;
        if let Some(c @ ('+' | '-')) = self.peek(0) {
            self.advance(1);
            number.push(c);
            sign = Some(if c == '+' { Sign::Plus } else { Sign::Minus });
        }
        self.consume_digits(&mut number);
        if self.peek(0) == Some('.') && is_digit(self.peek(1)) {
            self.advance(1);
            number.push('.');
            self.consume_digits(&mut number);
            integer = false;
        }
        let signed = matches!(self.peek(1), Some('+' | '-'));
        if matches!(self.peek(0), Some('E' | 'e'))
            && (is_digit(self.peek(1)) || (signed && is_digit(self.peek(2))))
        {
            self.advance(1);
            if let Some(c @ ('+' | '-')) = self.peek(0) {
                self.advance(1);
                exponent.push(c);
            }
            self.consume_digits(&mut exponent);
            integer = false;
        }
        (number_value(&number, &exponent), integer, sign)
    }

    fn consume_digits(&mut self, into: &mut String) {
        while let Some(c) = self.peek(0).filter(char::is_ascii_digit) {
            self.advance(1);
            into.push(c);
        }
    }

    /// *Consume a unicode-range token*.
    fn consume_unicode_range(&mut self) -> Token {
        self.advance(2);
        let mut first = String::new();
        while first.len() < 6
            && let Some(c) = self.peek(0).filter(char::is_ascii_hexdigit)
        {
            self.advance(1);
            first.push(c);
        }
        while first.len() < 6 && self.peek(0) == Some('?') {
            self.advance(1);
            first.push('?');
        }
        if first.contains('?') {
            return Token::UnicodeRange {
                start: hex(&first.replace('?', "0")),
                end: hex(&first.replace('?', "F")),
            };
        }
        let start = hex(&first);
        if self.peek(0) == Some('-') && self.peek(1).is_some_and(|c| c.is_ascii_hexdigit()) {
            self.advance(1);
            let mut second = String::new();
            while second.len() < 6
                && let Some(c) = self.peek(0).filter(char::is_ascii_hexdigit)
            {
                self.advance(1);
                second.push(c);
            }
            return Token::UnicodeRange {
                start,
                end: hex(&second),
            };
        }
        Token::UnicodeRange { start, end: start }
    }

    /// *Consume the remnants of a bad url*.
    fn consume_bad_url_remnants(&mut self) {
        loop {
            let Some(current) = self.peek(0) else {
                return;
            };
            self.advance(1);
            if current == ')' {
                return;
            }
            // So beendet ein `\)` das ⟨bad-url-token⟩ nicht.
            if is_valid_escape(Some(current), self.peek(0)) {
                self.consume_escaped();
            }
        }
    }
}

/// Der Wert aus *consume a number*: Zahlteil mal zehn hoch Exponent. Rusts
/// Umwandlung aus Dezimaltext rechnet das exakt und rundet einmal. Was `f64`
/// nicht fasst, wird nach CSS Values 4 („Numeric Data Types“) zum nächsten
/// darstellbaren Wert: unendlich zur größten endlichen Zahl, zu Kleines zu 0.
fn number_value(number: &str, exponent: &str) -> f64 {
    let text = if exponent.is_empty() {
        number.to_owned()
    } else {
        format!("{number}e{exponent}")
    };
    // Der Text passt immer zur Zahlensyntax von Rust; 0 ist nur Rückfall.
    let value = text.parse::<f64>().unwrap_or(0.0);
    if value.is_infinite() {
        f64::MAX.copysign(value)
    } else {
        value
    }
}

fn hex(digits: &str) -> u32 {
    // Höchstens sechs Hexziffern, das passt immer.
    u32::from_str_radix(digits, 16).unwrap_or(0)
}

// Definitionen („Definitions“). EOF ist `None`.

fn is_digit(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_digit())
}

/// *non-ASCII ident code point*
fn is_non_ascii_ident(c: char) -> bool {
    matches!(c,
        '\u{B7}'
        | '\u{C0}'..='\u{D6}'
        | '\u{D8}'..='\u{F6}'
        | '\u{F8}'..='\u{37D}'
        | '\u{37F}'..='\u{1FFF}'
        | '\u{200C}'
        | '\u{200D}'
        | '\u{203F}'
        | '\u{2040}'
        | '\u{2070}'..='\u{218F}'
        | '\u{2C00}'..='\u{2FEF}'
        | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}'
        | '\u{FDF0}'..='\u{FFFD}'
        | '\u{10000}'..='\u{10FFFF}')
}

/// *ident-start code point*
fn is_ident_start(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || is_non_ascii_ident(c))
}

/// *ident code point*
fn is_ident_code_point(c: Option<char>) -> bool {
    is_ident_start(c) || is_digit(c) || c == Some('-')
}

/// *non-printable code point*
fn is_non_printable(c: char) -> bool {
    matches!(c, '\0'..='\u{8}' | '\u{B}' | '\u{E}'..='\u{1F}' | '\u{7F}')
}

/// *whitespace*; CR und FF hat das Filtern schon ersetzt.
fn is_whitespace(c: Option<char>) -> bool {
    matches!(c, Some('\n' | '\t' | ' '))
}

/// *Check if two code points are a valid escape*.
fn is_valid_escape(first: Option<char>, second: Option<char>) -> bool {
    first == Some('\\') && second != Some('\n')
}

/// *Check if three code points would start an ident sequence*.
fn would_start_ident(first: Option<char>, second: Option<char>, third: Option<char>) -> bool {
    match first {
        Some('-') => {
            is_ident_start(second) || second == Some('-') || is_valid_escape(second, third)
        }
        Some('\\') => is_valid_escape(first, second),
        _ => is_ident_start(first),
    }
}

/// *Check if three code points would start a number*.
fn would_start_number(first: Option<char>, second: Option<char>, third: Option<char>) -> bool {
    match first {
        Some('+' | '-') => is_digit(second) || (second == Some('.') && is_digit(third)),
        Some('.') => is_digit(second),
        _ => is_digit(first),
    }
}

/// *Check if three code points would start a unicode-range*.
fn would_start_unicode_range(
    first: Option<char>,
    second: Option<char>,
    third: Option<char>,
) -> bool {
    matches!(first, Some('U' | 'u'))
        && second == Some('+')
        && third.is_some_and(|c| c == '?' || c.is_ascii_hexdigit())
}
