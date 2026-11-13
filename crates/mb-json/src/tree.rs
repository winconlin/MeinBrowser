//! Der gemeinsame Zerleger für JSON nach ECMA-404 und RFC 8259: Er prüft
//! einen Text aus UTF-16-Codeeinheiten und zerlegt ihn in einen Baum.
//! [`crate::parse`] baut daraus [`crate::Value`], die JavaScript-Engine
//! daraus die Werte von `JSON.parse` (ParseJSON, ECMA-262 Abschnitt
//! 25.5.2.1).
//!
//! Für gültiges JSON ergibt der Baum dieselben Werte wie die Auswertung
//! als Script in ParseJSON: Literale, Zahlen nach StringToNumber,
//! Zeichenketten mit ihren Escapes, Arrays und Objekte. Doppelte Schlüssel
//! bleiben mit ihrer Reihenfolge erhalten; was daraus wird, entscheidet,
//! wer den Baum liest.
//!
//! Der Zerleger arbeitet ohne Rekursion mit einem Stapel offener Arrays
//! und Objekte. Er arbeitet auf den Codeeinheiten: Außerhalb von
//! Zeichenketten sind nur ASCII-Zeichen gültig, und in Zeichenketten gehen
//! Codeeinheiten unverändert durch, auch einzelne Surrogate.
//!
//! Wie tief der Baum werden und wie viele Knoten er haben darf, bestimmt
//! der Aufrufer mit [`Limits`].

/// Ein Wert des Baums. Kinder sind Indizes in [`Tree::nodes`] und stehen
/// immer hinter ihrem Elternknoten.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Null,
    Boolean(bool),
    Number(f64),
    String(Vec<u16>),
    /// Die Elemente in ihrer Reihenfolge.
    Array(Vec<usize>),
    /// Die Paare aus Schlüssel und Wert in ihrer Reihenfolge, auch
    /// doppelte Schlüssel.
    Object(Vec<(Vec<u16>, usize)>),
}

/// Der Baum eines JSON-Texts: der Wurzelknoten hat den Index 0. Zu jedem
/// Knoten gehört der Bereich seiner Codeeinheiten im Text, für Zahlen mit
/// dem Minus davor.
#[derive(Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub ranges: Vec<(usize, usize)>,
}

/// Grenzen für den Baum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Die größte Zahl der Knoten.
    pub nodes: usize,
    /// Die größte Tiefe eines Knotens; die Wurzel hat die Tiefe 0, die
    /// Elemente und Werte eines Arrays oder Objekts der Tiefe n die Tiefe
    /// n + 1.
    pub depth: usize,
}

impl Limits {
    /// Keine Grenzen außer dem Speicher.
    pub const NONE: Limits = Limits {
        nodes: usize::MAX,
        depth: usize::MAX,
    };
}

/// Warum der Zerleger aufgab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// Der Text ist kein gültiges JSON. `position` ist die Stelle in
    /// Codeeinheiten, an der das auffiel.
    Invalid {
        position: usize,
        message: &'static str,
    },
    /// Ein Knoten an der Stelle `position` läge tiefer als erlaubt.
    TooDeep { position: usize },
    /// Der Baum hätte mehr Knoten, als erlaubt sind.
    TooLarge,
}

/// Was als Nächstes kommen muss.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Ein Wert.
    Value,
    /// Nach einem Wert: `,`, eine schließende Klammer oder das Ende.
    AfterValue,
}

const TAB: u16 = 0x09;
const LINE_FEED: u16 = 0x0A;
const CARRIAGE_RETURN: u16 = 0x0D;
const SPACE: u16 = 0x20;
const QUOTE: u16 = 0x22;
const COMMA: u16 = 0x2C;
const MINUS: u16 = 0x2D;
const COLON: u16 = 0x3A;
const BACKSLASH: u16 = 0x5C;
/// Die Meldungen, die mehrere Stellen geben.
const UNEXPECTED: &str = "unerwartetes Zeichen";
const DIGIT: &str = "Ziffer erwartet";

const OPEN_BRACKET: u16 = 0x5B;
const CLOSE_BRACKET: u16 = 0x5D;
const OPEN_BRACE: u16 = 0x7B;
const CLOSE_BRACE: u16 = 0x7D;

/// Zerlegt `text` in einen Baum innerhalb von `limits`.
pub fn parse(text: &[u16], limits: Limits) -> Result<Tree, Rejected> {
    let mut parser = Parser {
        text,
        position: 0,
        limits,
        parsed: Tree::default(),
        open: Vec::new(),
        key: None,
    };
    parser.run()?;
    Ok(parser.parsed)
}

struct Parser<'t> {
    text: &'t [u16],
    position: usize,
    limits: Limits,
    parsed: Tree,
    /// Die offenen Arrays und Objekte, innerstes zuletzt.
    open: Vec<usize>,
    /// Der Schlüssel, zu dem der nächste Wert gehört.
    key: Option<Vec<u16>>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u16> {
        self.text.get(self.position).copied()
    }

    fn invalid<T>(&self, message: &'static str) -> Result<T, Rejected> {
        Err(Rejected::Invalid {
            position: self.position,
            message,
        })
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(TAB | LINE_FEED | CARRIAGE_RETURN | SPACE)) {
            self.position = self.position.saturating_add(1);
        }
    }

    fn advance(&mut self) {
        self.position = self.position.saturating_add(1);
    }

    /// Erwartet `unit` an der aktuellen Stelle.
    fn expect(&mut self, unit: u16) -> Result<(), Rejected> {
        if self.peek() != Some(unit) {
            return self.invalid(UNEXPECTED);
        }
        self.advance();
        Ok(())
    }

    fn run(&mut self) -> Result<(), Rejected> {
        let mut expect = Expect::Value;
        loop {
            self.skip_whitespace();
            match expect {
                Expect::Value => expect = self.value()?,
                Expect::AfterValue => {
                    let Some(&top) = self.open.last() else {
                        // Nach dem Wert der Wurzel darf nur Leerraum kommen.
                        return if self.position == self.text.len() {
                            Ok(())
                        } else {
                            self.invalid("Zeichen nach dem Ende des Werts")
                        };
                    };
                    let is_object = matches!(self.parsed.nodes.get(top), Some(Node::Object(_)));
                    match self.peek() {
                        Some(COMMA) => {
                            self.advance();
                            if is_object {
                                self.skip_whitespace();
                                self.member_key()?;
                            }
                            expect = Expect::Value;
                        }
                        Some(CLOSE_BRACKET) if !is_object => self.close(top),
                        Some(CLOSE_BRACE) if is_object => self.close(top),
                        _ if is_object => return self.invalid("`,` oder `}` erwartet"),
                        _ => return self.invalid("`,` oder `]` erwartet"),
                    }
                }
            }
        }
    }

    /// Schließt das innerste Array oder Objekt.
    fn close(&mut self, node: usize) {
        self.advance();
        if let Some(range) = self.parsed.ranges.get_mut(node) {
            range.1 = self.position;
        }
        self.open.pop();
    }

    /// Ein Schlüssel mit dem Doppelpunkt danach.
    fn member_key(&mut self) -> Result<(), Rejected> {
        if self.peek() != Some(QUOTE) {
            return self.invalid("Schlüssel erwartet");
        }
        let key = self.string()?;
        self.skip_whitespace();
        self.expect(COLON)?;
        self.key = Some(key);
        Ok(())
    }

    /// Ein Wert; für ein Array oder Objekt nur seine öffnende Klammer und,
    /// wenn es nicht gleich endet, der erste Schlüssel.
    fn value(&mut self) -> Result<Expect, Rejected> {
        let start = self.position;
        let node = match self.peek() {
            Some(OPEN_BRACKET) => {
                self.advance();
                let index = self.add(Node::Array(Vec::new()), start)?;
                self.open.push(index);
                self.skip_whitespace();
                if self.peek() == Some(CLOSE_BRACKET) {
                    self.close(index);
                    return Ok(Expect::AfterValue);
                }
                return Ok(Expect::Value);
            }
            Some(OPEN_BRACE) => {
                self.advance();
                let index = self.add(Node::Object(Vec::new()), start)?;
                self.open.push(index);
                self.skip_whitespace();
                if self.peek() == Some(CLOSE_BRACE) {
                    self.close(index);
                    return Ok(Expect::AfterValue);
                }
                self.member_key()?;
                return Ok(Expect::Value);
            }
            Some(QUOTE) => Node::String(self.string()?),
            Some(MINUS | 0x30..=0x39) => Node::Number(self.number()?),
            Some(0x74) => {
                self.literal("true")?;
                Node::Boolean(true)
            }
            Some(0x66) => {
                self.literal("false")?;
                Node::Boolean(false)
            }
            Some(0x6E) => {
                self.literal("null")?;
                Node::Null
            }
            Some(_) => return self.invalid(UNEXPECTED),
            None => return self.invalid("Text endet zu früh"),
        };
        let index = self.add(node, start)?;
        if let Some(range) = self.parsed.ranges.get_mut(index) {
            range.1 = self.position;
        }
        Ok(Expect::AfterValue)
    }

    /// Hängt einen Knoten an das innerste offene Array oder Objekt.
    fn add(&mut self, node: Node, start: usize) -> Result<usize, Rejected> {
        let index = self.parsed.nodes.len();
        if index >= self.limits.nodes {
            return Err(Rejected::TooLarge);
        }
        if self.open.len() > self.limits.depth {
            return Err(Rejected::TooDeep { position: start });
        }
        self.parsed.nodes.push(node);
        self.parsed.ranges.push((start, start));
        if let Some(&parent) = self.open.last() {
            match self.parsed.nodes.get_mut(parent) {
                Some(Node::Array(elements)) => elements.push(index),
                Some(Node::Object(members)) => {
                    let Some(key) = self.key.take() else {
                        return self.invalid(UNEXPECTED);
                    };
                    members.push((key, index));
                }
                _ => return self.invalid(UNEXPECTED),
            }
        }
        Ok(index)
    }

    fn literal(&mut self, word: &str) -> Result<(), Rejected> {
        for unit in word.encode_utf16() {
            self.expect(unit)?;
        }
        Ok(())
    }

    fn digits(&mut self) -> usize {
        let start = self.position;
        while matches!(self.peek(), Some(0x30..=0x39)) {
            self.advance();
        }
        self.position.saturating_sub(start)
    }

    /// `-`? (`0` | [1-9][0-9]*) (`.` [0-9]+)? ([eE] [+-]? [0-9]+)?, der
    /// Wert nach StringToNumber.
    fn number(&mut self) -> Result<f64, Rejected> {
        let start = self.position;
        if self.peek() == Some(MINUS) {
            self.advance();
        }
        match self.peek() {
            Some(0x30) => self.advance(),
            Some(0x31..=0x39) => {
                self.digits();
            }
            _ => return self.invalid(DIGIT),
        }
        if self.peek() == Some(0x2E) {
            self.advance();
            if self.digits() == 0 {
                return self.invalid(DIGIT);
            }
        }
        if matches!(self.peek(), Some(0x45 | 0x65)) {
            self.advance();
            if matches!(self.peek(), Some(0x2B | MINUS)) {
                self.advance();
            }
            if self.digits() == 0 {
                return self.invalid(DIGIT);
            }
        }
        // Nur ASCII-Ziffern und Zeichen, die die std ebenso liest; sie
        // rundet korrekt wie StringToNumber, zu groß wird ±∞.
        let digits = self.text.get(start..self.position).unwrap_or_default();
        let ascii: String = digits
            .iter()
            .filter_map(|&unit| char::from_u32(u32::from(unit)))
            .collect();
        ascii.parse::<f64>().or_else(|_| self.invalid(DIGIT))
    }

    /// Eine Zeichenkette ab ihrem `"`: ihr Inhalt mit aufgelösten Escapes.
    fn string(&mut self) -> Result<Vec<u16>, Rejected> {
        self.expect(QUOTE)?;
        let mut units = Vec::new();
        loop {
            let Some(unit) = self.peek() else {
                return self.invalid("Zeichenkette ohne Ende");
            };
            match unit {
                QUOTE => {
                    self.advance();
                    return Ok(units);
                }
                BACKSLASH => {
                    self.advance();
                    let escaped = match self.peek() {
                        Some(QUOTE) => QUOTE,
                        Some(BACKSLASH) => BACKSLASH,
                        Some(0x2F) => 0x2F,
                        Some(0x62) => 0x08,
                        Some(0x66) => 0x0C,
                        Some(0x6E) => LINE_FEED,
                        Some(0x72) => CARRIAGE_RETURN,
                        Some(0x74) => TAB,
                        Some(0x75) => {
                            self.advance();
                            let mut value: u16 = 0;
                            for _ in 0..4 {
                                let Some(digit) = self
                                    .peek()
                                    .and_then(|unit| char::from_u32(u32::from(unit)))
                                    .and_then(|c| c.to_digit(16))
                                    .and_then(|digit| u16::try_from(digit).ok())
                                else {
                                    return self.invalid("vier Hexziffern erwartet");
                                };
                                // Vier Hexziffern passen in 16 Bit.
                                value = (value << 4) | digit;
                                self.advance();
                            }
                            units.push(value);
                            continue;
                        }
                        _ => return self.invalid("ungültige Escape-Sequenz"),
                    };
                    self.advance();
                    units.push(escaped);
                }
                // Steuerzeichen müssen escapet sein.
                0x00..=0x1F => return self.invalid("Steuerzeichen in Zeichenkette"),
                _ => {
                    self.advance();
                    units.push(unit);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn nodes(text: &str) -> Vec<Node> {
        parse(&units(text), Limits::NONE).expect("gültig").nodes
    }

    #[test]
    fn literals_numbers_and_strings() {
        assert_eq!(nodes(" null "), vec![Node::Null]);
        assert_eq!(nodes("true"), vec![Node::Boolean(true)]);
        assert_eq!(nodes("false"), vec![Node::Boolean(false)]);
        assert_eq!(nodes("-0"), vec![Node::Number(-0.0)]);
        assert_eq!(nodes("1.5e3"), vec![Node::Number(1500.0)]);
        assert_eq!(nodes("1E+400"), vec![Node::Number(f64::INFINITY)]);
        assert_eq!(
            nodes(r#""a\u00e4\n\/\"""#),
            vec![Node::String(units("aä\n/\""))]
        );
        // Ein einzelnes Surrogat geht unverändert durch.
        let parsed = parse(&[QUOTE, 0xD800, QUOTE], Limits::NONE).expect("gültig");
        assert_eq!(parsed.nodes, vec![Node::String(vec![0xD800])]);
    }

    #[test]
    fn containers_keep_order_and_duplicates() {
        assert_eq!(
            nodes(r#"[1, [], {"a": 2, "a": 3}]"#),
            vec![
                Node::Array(vec![1, 2, 3]),
                Node::Number(1.0),
                Node::Array(vec![]),
                Node::Object(vec![(units("a"), 4), (units("a"), 5)]),
                Node::Number(2.0),
                Node::Number(3.0),
            ]
        );
    }

    #[test]
    fn ranges_cover_the_source_text() {
        let parsed = parse(&units(r#"{"x": -1.5, "y": "s"}"#), Limits::NONE).expect("gültig");
        assert_eq!(parsed.ranges, vec![(0, 21), (6, 10), (17, 20)]);
    }

    #[test]
    fn rejects_what_ecma_404_does_not_allow() {
        for text in [
            "",
            " ",
            "01",
            "1.",
            ".1",
            "+1",
            "1e",
            "-",
            "[1,]",
            "[,1]",
            "{\"a\":1,}",
            "{a:1}",
            "'a'",
            "\"\t\"",
            "\"\\x41\"",
            "\"\\u12\"",
            "nul",
            "[1] 2",
            "\u{a0}1",
            "[",
            "{\"a\"}",
            "NaN",
            "Infinity",
            "\"a",
        ] {
            assert!(
                matches!(
                    parse(&units(text), Limits::NONE),
                    Err(Rejected::Invalid { .. })
                ),
                "{text:?}"
            );
        }
    }

    #[test]
    fn deep_nesting_needs_no_recursion() {
        let mut text = "[".repeat(100_000);
        text.push_str(&"]".repeat(100_000));
        let parsed = parse(&units(&text), Limits::NONE).expect("gültig");
        assert_eq!(parsed.nodes.len(), 100_000);
    }

    #[test]
    fn the_tree_stays_within_its_depth_limit() {
        let depth = |depth| Limits {
            nodes: usize::MAX,
            depth,
        };
        let text = units("[[1]]");
        assert!(parse(&text, depth(2)).is_ok());
        assert_eq!(
            parse(&text, depth(1)).map(|_| ()),
            Err(Rejected::TooDeep { position: 2 })
        );
    }

    #[test]
    fn failures_name_the_place_and_the_reason() {
        let failure = |text: &str| match parse(&units(text), Limits::NONE) {
            Err(Rejected::Invalid { position, message }) => Some((position, message)),
            _ => None,
        };
        assert_eq!(failure("[1,]"), Some((3, "unerwartetes Zeichen")));
        assert_eq!(failure("[1 2]"), Some((3, "`,` oder `]` erwartet")));
        assert_eq!(failure("{\"a\": 1 2}"), Some((8, "`,` oder `}` erwartet")));
        assert_eq!(failure("{1: 2}"), Some((1, "Schlüssel erwartet")));
        assert_eq!(
            failure("[1] 2"),
            Some((4, "Zeichen nach dem Ende des Werts"))
        );
        assert_eq!(failure("["), Some((1, "Text endet zu früh")));
        assert_eq!(failure("-a"), Some((1, "Ziffer erwartet")));
        assert_eq!(failure("\"a"), Some((2, "Zeichenkette ohne Ende")));
        assert_eq!(failure("\"\\x\""), Some((2, "ungültige Escape-Sequenz")));
        assert_eq!(failure("\"\\u12\""), Some((5, "vier Hexziffern erwartet")));
        assert_eq!(
            failure("\"\t\""),
            Some((1, "Steuerzeichen in Zeichenkette"))
        );
    }

    #[test]
    fn the_tree_stays_within_its_node_limit() {
        let nodes = |nodes| Limits {
            nodes,
            depth: usize::MAX,
        };
        let text = units(r#"[1, {"a": [2]}]"#);
        assert_eq!(parse(&text, nodes(5)).expect("gültig").nodes.len(), 5);
        assert_eq!(parse(&text, nodes(4)).map(|_| ()), Err(Rejected::TooLarge));
        assert_eq!(
            parse(&units("1"), nodes(0)).map(|_| ()),
            Err(Rejected::TooLarge)
        );
    }
}
