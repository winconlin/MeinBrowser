//! Unicode-Eigenschaften in `\p{…}` und `\P{…}`: die Namen aus den Tabellen
//! der Spezifikation zu UnicodeMatchProperty, die Werte von
//! General_Category und Script aus `mb-unicode`.

use super::{Error, ErrorKind, Node, Parser, Result};

/// Eine Eigenschaft mit ihrem Wert, jeweils mit dem kanonischen Namen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    /// Ein Wert von General_Category, auch eine Gruppe, als Kurzname.
    GeneralCategory(&'static str),
    /// Ein Wert von Script als Kurzname.
    Script(&'static str),
    /// Ein Wert von Script_Extensions als Kurzname.
    ScriptExtensions(&'static str),
    /// Eine binäre Eigenschaft.
    Binary(&'static str),
    /// Eine binäre Eigenschaft von Zeichenketten, nur mit `v`.
    Strings(&'static str),
}

/// Die binären Eigenschaften mit ihren Aliassen, nach Namen sortiert, und
/// ihr kanonischer Name.
const BINARY: [(&str, &str); 98] = [
    ("AHex", "ASCII_Hex_Digit"),
    ("ASCII", "ASCII"),
    ("ASCII_Hex_Digit", "ASCII_Hex_Digit"),
    ("Alpha", "Alphabetic"),
    ("Alphabetic", "Alphabetic"),
    ("Any", "Any"),
    ("Assigned", "Assigned"),
    ("Bidi_C", "Bidi_Control"),
    ("Bidi_Control", "Bidi_Control"),
    ("Bidi_M", "Bidi_Mirrored"),
    ("Bidi_Mirrored", "Bidi_Mirrored"),
    ("CI", "Case_Ignorable"),
    ("CWCF", "Changes_When_Casefolded"),
    ("CWCM", "Changes_When_Casemapped"),
    ("CWKCF", "Changes_When_NFKC_Casefolded"),
    ("CWL", "Changes_When_Lowercased"),
    ("CWT", "Changes_When_Titlecased"),
    ("CWU", "Changes_When_Uppercased"),
    ("Case_Ignorable", "Case_Ignorable"),
    ("Cased", "Cased"),
    ("Changes_When_Casefolded", "Changes_When_Casefolded"),
    ("Changes_When_Casemapped", "Changes_When_Casemapped"),
    ("Changes_When_Lowercased", "Changes_When_Lowercased"),
    (
        "Changes_When_NFKC_Casefolded",
        "Changes_When_NFKC_Casefolded",
    ),
    ("Changes_When_Titlecased", "Changes_When_Titlecased"),
    ("Changes_When_Uppercased", "Changes_When_Uppercased"),
    ("DI", "Default_Ignorable_Code_Point"),
    ("Dash", "Dash"),
    (
        "Default_Ignorable_Code_Point",
        "Default_Ignorable_Code_Point",
    ),
    ("Dep", "Deprecated"),
    ("Deprecated", "Deprecated"),
    ("Dia", "Diacritic"),
    ("Diacritic", "Diacritic"),
    ("EBase", "Emoji_Modifier_Base"),
    ("EComp", "Emoji_Component"),
    ("EMod", "Emoji_Modifier"),
    ("EPres", "Emoji_Presentation"),
    ("Emoji", "Emoji"),
    ("Emoji_Component", "Emoji_Component"),
    ("Emoji_Modifier", "Emoji_Modifier"),
    ("Emoji_Modifier_Base", "Emoji_Modifier_Base"),
    ("Emoji_Presentation", "Emoji_Presentation"),
    ("Ext", "Extender"),
    ("ExtPict", "Extended_Pictographic"),
    ("Extended_Pictographic", "Extended_Pictographic"),
    ("Extender", "Extender"),
    ("Gr_Base", "Grapheme_Base"),
    ("Gr_Ext", "Grapheme_Extend"),
    ("Grapheme_Base", "Grapheme_Base"),
    ("Grapheme_Extend", "Grapheme_Extend"),
    ("Hex", "Hex_Digit"),
    ("Hex_Digit", "Hex_Digit"),
    ("IDC", "ID_Continue"),
    ("IDS", "ID_Start"),
    ("IDSB", "IDS_Binary_Operator"),
    ("IDST", "IDS_Trinary_Operator"),
    ("IDS_Binary_Operator", "IDS_Binary_Operator"),
    ("IDS_Trinary_Operator", "IDS_Trinary_Operator"),
    ("ID_Continue", "ID_Continue"),
    ("ID_Start", "ID_Start"),
    ("Ideo", "Ideographic"),
    ("Ideographic", "Ideographic"),
    ("Join_C", "Join_Control"),
    ("Join_Control", "Join_Control"),
    ("LOE", "Logical_Order_Exception"),
    ("Logical_Order_Exception", "Logical_Order_Exception"),
    ("Lower", "Lowercase"),
    ("Lowercase", "Lowercase"),
    ("Math", "Math"),
    ("NChar", "Noncharacter_Code_Point"),
    ("Noncharacter_Code_Point", "Noncharacter_Code_Point"),
    ("Pat_Syn", "Pattern_Syntax"),
    ("Pat_WS", "Pattern_White_Space"),
    ("Pattern_Syntax", "Pattern_Syntax"),
    ("Pattern_White_Space", "Pattern_White_Space"),
    ("QMark", "Quotation_Mark"),
    ("Quotation_Mark", "Quotation_Mark"),
    ("RI", "Regional_Indicator"),
    ("Radical", "Radical"),
    ("Regional_Indicator", "Regional_Indicator"),
    ("SD", "Soft_Dotted"),
    ("STerm", "Sentence_Terminal"),
    ("Sentence_Terminal", "Sentence_Terminal"),
    ("Soft_Dotted", "Soft_Dotted"),
    ("Term", "Terminal_Punctuation"),
    ("Terminal_Punctuation", "Terminal_Punctuation"),
    ("UIdeo", "Unified_Ideograph"),
    ("Unified_Ideograph", "Unified_Ideograph"),
    ("Upper", "Uppercase"),
    ("Uppercase", "Uppercase"),
    ("VS", "Variation_Selector"),
    ("Variation_Selector", "Variation_Selector"),
    ("White_Space", "White_Space"),
    ("XIDC", "XID_Continue"),
    ("XIDS", "XID_Start"),
    ("XID_Continue", "XID_Continue"),
    ("XID_Start", "XID_Start"),
    ("space", "White_Space"),
];

/// Die binären Eigenschaften von Zeichenketten.
const STRINGS: [&str; 7] = [
    "Basic_Emoji",
    "Emoji_Keycap_Sequence",
    "RGI_Emoji_Modifier_Sequence",
    "RGI_Emoji_Flag_Sequence",
    "RGI_Emoji_Tag_Sequence",
    "RGI_Emoji_ZWJ_Sequence",
    "RGI_Emoji",
];

impl Parser<'_> {
    /// `p{…}` oder `P{…}` ab dem `p`; `start` ist die Lage des `\`. Sagt
    /// auch, ob die Eigenschaft Zeichenketten enthalten kann
    /// (MayContainStrings).
    pub(super) fn property(&mut self, start: usize) -> Result<(Node, bool)> {
        let invalid = Error {
            kind: ErrorKind::InvalidProperty,
            position: start,
        };
        let negated = self.unit(0) == Some(b'P');
        self.advance(1);
        if !self.eat(b'{') {
            return Err(invalid);
        }
        let name = self.property_characters();
        let value = self.eat(b'=').then(|| self.property_characters());
        if !self.eat(b'}') {
            return Err(invalid);
        }
        let property = match value {
            Some(value) => {
                let value = |find: fn(&str) -> Option<&'static str>| find(&value).ok_or(invalid);
                match name.as_str() {
                    "General_Category" | "gc" => {
                        Property::GeneralCategory(value(mb_unicode::general_category_value)?)
                    }
                    "Script" | "sc" => Property::Script(value(mb_unicode::script_value)?),
                    "Script_Extensions" | "scx" => {
                        Property::ScriptExtensions(value(mb_unicode::script_value)?)
                    }
                    _ => return Err(invalid),
                }
            }
            None => lone(&name).ok_or(invalid)?,
        };
        let strings = matches!(property, Property::Strings(_));
        if strings {
            if !self.sets {
                return Err(Error {
                    kind: ErrorKind::PropertyOfStringsWithoutV,
                    position: start,
                });
            }
            if negated {
                return Err(Error {
                    kind: ErrorKind::NegatedStrings,
                    position: start,
                });
            }
        }
        Ok((Node::Property { property, negated }, strings))
    }

    /// Buchstaben, Ziffern und `_`, wie sie in Namen und Werten stehen.
    fn property_characters(&mut self) -> String {
        let mut text = String::new();
        while let Some(unit) = self
            .unit(0)
            .filter(|unit| unit.is_ascii_alphanumeric() || *unit == b'_')
        {
            text.push(char::from(unit));
            self.advance(1);
        }
        text
    }
}

/// *LoneUnicodePropertyNameOrValue*: ein Wert von General_Category, eine
/// binäre Eigenschaft oder eine von Zeichenketten.
fn lone(name: &str) -> Option<Property> {
    if let Some(value) = mb_unicode::general_category_value(name) {
        return Some(Property::GeneralCategory(value));
    }
    if let Ok(index) = BINARY.binary_search_by(|&(entry, _)| entry.cmp(name)) {
        return BINARY
            .get(index)
            .map(|&(_, canonical)| Property::Binary(canonical));
    }
    STRINGS
        .iter()
        .find(|&&entry| entry == name)
        .map(|&entry| Property::Strings(entry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_table_is_sorted_and_canonical() {
        assert!(BINARY.windows(2).all(|pair| pair[0].0 < pair[1].0));
        // Jeder kanonische Name steht selbst in der Tabelle.
        for (_, canonical) in BINARY {
            assert!(BINARY.contains(&(canonical, canonical)), "{canonical}");
        }
        let canonical: std::collections::BTreeSet<_> =
            BINARY.iter().map(|&(_, name)| name).collect();
        assert_eq!(canonical.len(), 53);
    }

    #[test]
    fn lone_names() {
        assert_eq!(lone("Lu"), Some(Property::GeneralCategory("Lu")));
        assert_eq!(lone("punct"), Some(Property::GeneralCategory("P")));
        assert_eq!(lone("Alpha"), Some(Property::Binary("Alphabetic")));
        assert_eq!(lone("space"), Some(Property::Binary("White_Space")));
        assert_eq!(lone("RGI_Emoji"), Some(Property::Strings("RGI_Emoji")));
        for name in ["Latin", "Script", "ascii", "In_Latin", "Block", ""] {
            assert_eq!(lone(name), None, "{name}");
        }
    }
}
