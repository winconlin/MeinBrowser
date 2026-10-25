//! Internationalisierte Domainnamen nach UTS #46 (Unicode IDNA Compatibility
//! Processing): ToASCII und ToUnicode, wie der URL-Standard sie aufruft.
//!
//! Der Text von UTS #46 ließ sich bei der Umsetzung nicht laden (unicode.org
//! war aus der Arbeitsumgebung nicht erreichbar). Die Schritte folgen daher
//! der Referenzimplementierung von Unicode, Inc., mit der auch
//! `IdnaTestV2.txt` erzeugt wird (`org.unicode.idna.Uts46` im Repository
//! `unicode-org/unicodetools`, Commit `578f6c0516f5f268292ebda50b51848407a06478`),
//! und werden mit diesen Tests geprüft. Die Fehler tragen die Kürzel aus
//! `IdnaTestV2.txt` (siehe [`Error::code`]).
//!
//! Den Schalter IgnoreInvalidPunycode gibt es nicht, er ist immer aus, so wie
//! der URL-Standard ihn setzt.

pub mod punycode;
#[rustfmt::skip]
mod tables;

use crate::normalization::{is_nfc, nfc};
use crate::{
    BidiClass, JoiningType, bidi_class, canonical_combining_class, general_category, joining_type,
};

/// Status eines Codepunkts in der IDNA-Zuordnungstabelle (UTS #46,
/// Abschnitt 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Valid,
    Ignored,
    Mapped,
    Deviation,
    Disallowed,
}

/// Die Schalter von UTS #46.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub check_hyphens: bool,
    pub check_bidi: bool,
    pub check_joiners: bool,
    pub use_std3_ascii_rules: bool,
    pub transitional_processing: bool,
    pub verify_dns_length: bool,
}

/// Ein Fehler bei der Verarbeitung, mit dem Kürzel aus `IdnaTestV2.txt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// P4: Ein Label mit `xn--` ist kein gültiges Punycode, oder es ergibt
    /// ein leeres Label oder nur ASCII.
    InvalidPunycode,
    /// V1: Das Label ist nicht in NFC.
    NotNfc,
    /// V2: Bindestriche an dritter und vierter Stelle (mit CheckHyphens).
    HyphensAtThirdAndFourth,
    /// V3: Bindestrich am Anfang oder Ende (mit CheckHyphens).
    LeadingOrTrailingHyphen,
    /// V4: Das Label beginnt mit `xn--` (ohne CheckHyphens).
    AcePrefix,
    /// V5: Das Label enthält einen Punkt.
    FullStop,
    /// V6: Das Label beginnt mit einem Kombinationszeichen
    /// (General_Category=Mark).
    LeadingMark,
    /// V7: Ein Codepunkt hat einen Status, der im Label nicht erlaubt ist.
    InvalidCodePoint,
    /// U1: Mit UseSTD3ASCIIRules sind von ASCII nur Kleinbuchstaben, Ziffern
    /// und der Bindestrich erlaubt.
    Std3,
    /// B1: Das erste Zeichen hat nicht die Bidi-Klasse L, R oder AL
    /// (RFC 5893, Abschnitt 2).
    BidiFirstCharacter,
    /// B2: Ein Zeichen, das in einem Label von rechts nach links nicht
    /// erlaubt ist.
    BidiRtlCharacter,
    /// B3: Falsches Ende eines Labels von rechts nach links.
    BidiRtlEnd,
    /// B4: Europäische und arabische Ziffern in einem Label von rechts nach
    /// links.
    BidiNumbers,
    /// B5: Ein Zeichen, das in einem Label von links nach rechts nicht
    /// erlaubt ist.
    BidiLtrCharacter,
    /// B6: Falsches Ende eines Labels von links nach rechts.
    BidiLtrEnd,
    /// C1: U+200C ZERO WIDTH NON-JOINER außerhalb seines Kontexts
    /// (RFC 5892, Anhang A.1).
    ZeroWidthNonJoiner,
    /// C2: U+200D ZERO WIDTH JOINER ohne Virama davor (RFC 5892, Anhang A.2).
    ZeroWidthJoiner,
    /// A3: Die Punycode-Kodierung ist gescheitert.
    PunycodeEncoding,
    /// A4_1: Der Domainname ist leer oder länger als 253 Zeichen (mit
    /// VerifyDnsLength).
    DomainLength,
    /// A4_2: Ein Label ist leer oder länger als 63 Zeichen (mit
    /// VerifyDnsLength).
    LabelLength,
    /// X4_2: Ein leeres Label, außer dem Wurzellabel am Ende (nur ToUnicode).
    EmptyLabel,
}

impl Error {
    /// Alle Fehler in der Reihenfolge ihrer Kürzel in `IdnaTestV2.txt`.
    pub const ALL: [Error; 21] = [
        Error::InvalidPunycode,
        Error::NotNfc,
        Error::HyphensAtThirdAndFourth,
        Error::LeadingOrTrailingHyphen,
        Error::AcePrefix,
        Error::FullStop,
        Error::LeadingMark,
        Error::InvalidCodePoint,
        Error::Std3,
        Error::BidiFirstCharacter,
        Error::BidiRtlCharacter,
        Error::BidiRtlEnd,
        Error::BidiNumbers,
        Error::BidiLtrCharacter,
        Error::BidiLtrEnd,
        Error::ZeroWidthNonJoiner,
        Error::ZeroWidthJoiner,
        Error::PunycodeEncoding,
        Error::DomainLength,
        Error::LabelLength,
        Error::EmptyLabel,
    ];

    /// Das Kürzel aus `IdnaTestV2.txt`, etwa `V6`.
    pub const fn code(self) -> &'static str {
        match self {
            Error::InvalidPunycode => "P4",
            Error::NotNfc => "V1",
            Error::HyphensAtThirdAndFourth => "V2",
            Error::LeadingOrTrailingHyphen => "V3",
            Error::AcePrefix => "V4",
            Error::FullStop => "V5",
            Error::LeadingMark => "V6",
            Error::InvalidCodePoint => "V7",
            Error::Std3 => "U1",
            Error::BidiFirstCharacter => "B1",
            Error::BidiRtlCharacter => "B2",
            Error::BidiRtlEnd => "B3",
            Error::BidiNumbers => "B4",
            Error::BidiLtrCharacter => "B5",
            Error::BidiLtrEnd => "B6",
            Error::ZeroWidthNonJoiner => "C1",
            Error::ZeroWidthJoiner => "C2",
            Error::PunycodeEncoding => "A3",
            Error::DomainLength => "A4_1",
            Error::LabelLength => "A4_2",
            Error::EmptyLabel => "X4_2",
        }
    }

    const fn bit(self) -> u32 {
        // Ein Bit je Variante; es sind weniger als 32.
        1 << self as u32
    }
}

/// Die Menge der Fehler einer Verarbeitung.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Errors(u32);

impl Errors {
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, error: Error) -> bool {
        self.0 & error.bit() != 0
    }

    fn insert(&mut self, error: Error) {
        self.0 |= error.bit();
    }

    pub fn iter(self) -> impl Iterator<Item = Error> {
        Error::ALL
            .into_iter()
            .filter(move |&error| self.contains(error))
    }
}

/// Die Kürzel in eckigen Klammern wie in `IdnaTestV2.txt`, etwa `[B5, B6]`.
impl std::fmt::Display for Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let codes: Vec<&str> = self.iter().map(Error::code).collect();
        write!(f, "[{}]", codes.join(", "))
    }
}

impl std::fmt::Debug for Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

/// ToASCII (UTS #46, Abschnitt 4.2): der Domainname in ASCII, Labels mit
/// anderen Zeichen als Punycode mit `xn--`. Jeder Fehler lässt die Umwandlung
/// scheitern.
pub fn to_ascii(domain: &str, options: Options) -> Result<String, Errors> {
    let mut errors = Errors::default();
    // Leere Labels findet hier die Längenprüfung.
    let labels = process(domain, options, None, &mut errors);
    let mut output = String::with_capacity(domain.len());
    for (index, label) in labels.iter().enumerate() {
        if index > 0 {
            output.push('.');
        }
        let ascii = if label.is_ascii() {
            label.clone()
        } else if let Some(encoded) = punycode::encode(label) {
            format!("xn--{encoded}")
        } else {
            errors.insert(Error::PunycodeEncoding);
            label.clone()
        };
        // Auch das leere Wurzellabel am Ende zählt als zu kurz.
        if options.verify_dns_length && !(1..=63).contains(&ascii.chars().count()) {
            errors.insert(Error::LabelLength);
        }
        output.push_str(&ascii);
    }
    // Die Länge ohne das Wurzellabel und seinen Punkt.
    let length = output.strip_suffix('.').unwrap_or(&output).chars().count();
    if options.verify_dns_length && !(1..=253).contains(&length) {
        errors.insert(Error::DomainLength);
    }
    if errors.is_empty() {
        Ok(output)
    } else {
        Err(errors)
    }
}

/// ToUnicode (UTS #46, Abschnitt 4.3): der verarbeitete Domainname mit
/// dekodierten Labels und die Fehler. Das Ergebnis gibt es auch bei Fehlern.
pub fn to_unicode(domain: &str, options: Options) -> (String, Errors) {
    let mut errors = Errors::default();
    let labels = process(domain, options, Some(Error::EmptyLabel), &mut errors);
    (labels.join("."), errors)
}

/// Die Verarbeitung (UTS #46, Abschnitt 4): abbilden, normalisieren, in
/// Labels teilen, Punycode dekodieren und prüfen. `empty_label` ist der
/// Fehler für ein leeres Label vor dem Ende.
fn process(
    domain: &str,
    options: Options,
    empty_label: Option<Error>,
    errors: &mut Errors,
) -> Vec<String> {
    let transitional = options.transitional_processing;
    let mut mapped = String::with_capacity(domain.len());
    for c in domain.chars() {
        let (status, mapping) = lookup(c);
        match status {
            // Unzulässige Zeichen bleiben und fallen bei der Prüfung auf.
            Status::Valid | Status::Disallowed => mapped.push(c),
            Status::Ignored => {}
            // ẞ wird ß, das bei der Übergangsverarbeitung aber zu ss wird.
            Status::Mapped if transitional && c == '\u{1E9E}' => mapped.push_str("ss"),
            Status::Mapped => mapped.extend(mapping),
            Status::Deviation if transitional => mapped.extend(mapping),
            Status::Deviation => mapped.push(c),
        }
    }
    let labels: Vec<String> = nfc(&mapped)
        .split('.')
        .map(|label| convert(label, options, errors))
        .collect();
    check_bidi_and_joiners(&labels, options, empty_label, errors);
    labels
}

/// Dekodiert ein Label mit `xn--` und prüft es. Scheitert die Dekodierung,
/// bleibt das Label, wie es ist.
fn convert(label: &str, options: Options, errors: &mut Errors) -> String {
    let Some(encoded) = label.strip_prefix("xn--") else {
        validate(label, options, options.transitional_processing, errors);
        return label.to_owned();
    };
    let Some(decoded) = punycode::decode(encoded) else {
        errors.insert(Error::InvalidPunycode);
        return label.to_owned();
    };
    if decoded.is_empty() || decoded.is_ascii() {
        errors.insert(Error::InvalidPunycode);
    }
    // Dekodierte Labels werden immer ohne Übergangsverarbeitung geprüft.
    validate(&decoded, options, false, errors);
    decoded
}

/// Die Gültigkeitskriterien (UTS #46, Abschnitt 4.1) außer Bidi und ContextJ,
/// die für den ganzen Domainnamen geprüft werden.
fn validate(label: &str, options: Options, transitional: bool, errors: &mut Errors) {
    if !is_nfc(label) {
        errors.insert(Error::NotNfc);
    }
    if options.check_hyphens {
        let mut chars = label.chars().skip(2);
        if chars.next() == Some('-') && chars.next() == Some('-') {
            errors.insert(Error::HyphensAtThirdAndFourth);
        }
        if label.starts_with('-') || label.ends_with('-') {
            errors.insert(Error::LeadingOrTrailingHyphen);
        }
    } else if label.starts_with("xn--") {
        errors.insert(Error::AcePrefix);
    }
    if label.contains('.') {
        errors.insert(Error::FullStop);
    }
    if label
        .chars()
        .next()
        .is_some_and(|first| general_category(u32::from(first)).is_mark())
    {
        errors.insert(Error::LeadingMark);
    }
    for c in label.chars() {
        match lookup(c).0 {
            Status::Valid => {
                let ldh = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
                if options.use_std3_ascii_rules && c.is_ascii() && !ldh {
                    errors.insert(Error::Std3);
                }
            }
            Status::Deviation if !transitional => {}
            _ => errors.insert(Error::InvalidCodePoint),
        }
    }
}

/// Bidi-Regel (CheckBidi) und ContextJ (CheckJoiners) für jedes Label. Ein
/// leeres Wurzellabel am Ende zählt nicht mit.
fn check_bidi_and_joiners(
    labels: &[String],
    options: Options,
    empty_label: Option<Error>,
    errors: &mut Errors,
) {
    let labels = match labels.split_last() {
        Some((last, rest)) if last.is_empty() && !rest.is_empty() => rest,
        _ => labels,
    };
    // Ein Bidi-Domainname enthält ein Zeichen der Klasse R, AL oder AN.
    let bidi = labels.iter().flat_map(|label| label.chars()).any(|c| {
        matches!(
            bidi_class(u32::from(c)),
            BidiClass::RightToLeft | BidiClass::ArabicLetter | BidiClass::ArabicNumber
        )
    });
    for label in labels {
        if label.is_empty() {
            if let Some(error) = empty_label {
                errors.insert(error);
            }
            continue;
        }
        if options.check_bidi && bidi {
            check_bidi(label, errors);
        }
        if options.check_joiners {
            check_joiners(label, errors);
        }
    }
}

/// Die sechs Bedingungen aus RFC 5893, Abschnitt 2, für ein nicht leeres
/// Label.
fn check_bidi(label: &str, errors: &mut Errors) {
    use BidiClass::{
        ArabicLetter as AL, ArabicNumber as AN, BoundaryNeutral as BN, CommonSeparator as CS,
        EuropeanNumber as EN, EuropeanSeparator as ES, EuropeanTerminator as ET, LeftToRight as L,
        NonspacingMark as NSM, OtherNeutral as ON, RightToLeft as R,
    };
    let classes: Vec<BidiClass> = label.chars().map(|c| bidi_class(u32::from(c))).collect();
    let rtl = match classes.first() {
        Some(R | AL) => true,
        Some(L) => false,
        _ => {
            // Ohne gültigen Anfang lassen sich die übrigen Regeln nicht anwenden.
            errors.insert(Error::BidiFirstCharacter);
            return;
        }
    };
    // Das letzte Zeichen ohne folgende NSM. Das erste ist keins, also gibt
    // es eins.
    let last = classes.iter().rev().find(|&&class| class != NSM).copied();
    if rtl {
        let allowed = |class| matches!(class, R | AL | AN | EN | ES | CS | ET | ON | BN | NSM);
        if !classes.iter().copied().all(allowed) {
            errors.insert(Error::BidiRtlCharacter);
        }
        if !matches!(last, Some(R | AL | EN | AN)) {
            errors.insert(Error::BidiRtlEnd);
        }
        if classes.contains(&EN) && classes.contains(&AN) {
            errors.insert(Error::BidiNumbers);
        }
    } else {
        let allowed = |class| matches!(class, L | EN | ES | CS | ET | ON | BN | NSM);
        if !classes.iter().copied().all(allowed) {
            errors.insert(Error::BidiLtrCharacter);
        }
        if !matches!(last, Some(L | EN)) {
            errors.insert(Error::BidiLtrEnd);
        }
    }
}

/// Die ContextJ-Regeln aus RFC 5892, Anhang A.1 und A.2.
fn check_joiners(label: &str, errors: &mut Errors) {
    const ZWNJ: char = '\u{200C}';
    const ZWJ: char = '\u{200D}';
    const VIRAMA: u8 = 9;
    let chars: Vec<char> = label.chars().collect();
    for (index, &c) in chars.iter().enumerate() {
        if c != ZWNJ && c != ZWJ {
            continue;
        }
        let before = chars.get(..index).unwrap_or_default();
        if before
            .last()
            .is_some_and(|&b| canonical_combining_class(u32::from(b)) == VIRAMA)
        {
            continue;
        }
        if c == ZWJ {
            errors.insert(Error::ZeroWidthJoiner);
            continue;
        }
        // (Joining_Type:{L,D})(Joining_Type:T)*‌(Joining_Type:T)*(Joining_Type:{R,D})
        let after = chars.get(index + 1..).unwrap_or_default();
        let not_transparent = |c: &&char| joining_type(u32::from(**c)) != JoiningType::Transparent;
        let left = before.iter().rev().find(not_transparent);
        let right = after.iter().find(not_transparent);
        let joins_left = left.is_some_and(|&c| {
            matches!(
                joining_type(u32::from(c)),
                JoiningType::LeftJoining | JoiningType::DualJoining
            )
        });
        let joins_right = right.is_some_and(|&c| {
            matches!(
                joining_type(u32::from(c)),
                JoiningType::RightJoining | JoiningType::DualJoining
            )
        });
        if !(joins_left && joins_right) {
            errors.insert(Error::ZeroWidthNonJoiner);
        }
    }
}

/// Status und Abbildung eines Zeichens.
fn lookup(c: char) -> (Status, &'static [char]) {
    let code_point = u32::from(c);
    let after = tables::MAPPING.partition_point(|&(start, ..)| start <= code_point);
    let Some(&(_, status, offset, length)) = after
        .checked_sub(1)
        .and_then(|index| tables::MAPPING.get(index))
    else {
        return (Status::Disallowed, &[]);
    };
    let start = usize::from(offset);
    let mapping = tables::MAPPING_DATA
        .get(start..start + usize::from(length))
        .unwrap_or_default();
    (status, mapping)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRICT: Options = Options {
        check_hyphens: true,
        check_bidi: true,
        check_joiners: true,
        use_std3_ascii_rules: true,
        transitional_processing: false,
        verify_dns_length: true,
    };

    /// So setzt der URL-Standard die Schalter, wenn er nicht streng prüft.
    const URL: Options = Options {
        check_hyphens: false,
        check_bidi: true,
        check_joiners: true,
        use_std3_ascii_rules: false,
        transitional_processing: false,
        verify_dns_length: false,
    };

    const LENIENT: Options = Options {
        check_hyphens: false,
        check_bidi: false,
        check_joiners: false,
        use_std3_ascii_rules: false,
        transitional_processing: false,
        verify_dns_length: false,
    };

    const TRANSITIONAL: Options = Options {
        transitional_processing: true,
        ..STRICT
    };

    #[test]
    fn examples() {
        // Zeilen aus IdnaTestV2.txt.
        assert_eq!(
            to_ascii("Bücher.de", STRICT).as_deref(),
            Ok("xn--bcher-kva.de")
        );
        assert_eq!(to_ascii("FAẞ.de", STRICT).as_deref(), Ok("xn--fa-hia.de"));
        assert_eq!(to_ascii("FAẞ.de", TRANSITIONAL).as_deref(), Ok("fass.de"));
        assert_eq!(to_ascii("A.B．C。D｡", URL).as_deref(), Ok("a.b.c.d."));
        assert_eq!(
            to_unicode("xn--fa-hia.de", STRICT),
            ("faß.de".to_owned(), Errors::default())
        );
        let (output, errors) = to_unicode("xn--a-Ä.pt", STRICT);
        assert_eq!(output, "xn--a-ä.pt");
        assert_eq!(errors.to_string(), "[P4]");
        assert_eq!(to_unicode("a..b", STRICT).1.to_string(), "[X4_2]");
        assert_eq!(
            to_ascii("a..b", STRICT).map_err(|e| e.to_string()),
            Err("[A4_2]".to_owned())
        );
        assert_eq!(to_ascii("a..b", URL).as_deref(), Ok("a..b"));
        assert_eq!(
            to_ascii("\u{200D}", STRICT).map_err(|e| e.to_string()),
            Err("[C2]".to_owned())
        );
        assert_eq!(to_ascii("\u{200D}", LENIENT).as_deref(), Ok("xn--1ug"));
        // Bidi: Label von links nach rechts mit arabischem Zeichen.
        let errors = to_unicode("xn--a-ccba213a", STRICT).1;
        assert!(errors.contains(Error::BidiLtrCharacter) && errors.contains(Error::BidiLtrEnd));
        assert!(to_unicode("xn--a-ccba213a", LENIENT).1.is_empty());
    }

    /// Ignorierte Kürzel, wenn ein Schalter aus ist (Kopf von
    /// `IdnaTestV2.txt`).
    fn ignored(options: Options) -> Vec<&'static str> {
        let mut ignored = Vec::new();
        if !options.verify_dns_length {
            ignored.extend(["A4_1", "A4_2"]);
        }
        if !options.check_hyphens {
            ignored.extend(["V2", "V3"]);
        }
        if !options.check_joiners {
            ignored.extend(["C1", "C2"]);
        }
        if !options.check_bidi {
            ignored.extend(["B1", "B2", "B3", "B4", "B5", "B6"]);
        }
        if !options.use_std3_ascii_rules {
            ignored.push("U1");
        }
        ignored
    }

    /// Eine Zeichenkette aus der Testdatei mit `\uXXXX`-Escapes. `None`, wenn
    /// ein Escape ein Surrogat ist.
    fn unescape(text: &str) -> Option<String> {
        if text == "\"\"" {
            return Some(String::new());
        }
        let mut output = String::new();
        let mut rest = text;
        while let Some(position) = rest.find("\\u") {
            output.push_str(&rest[..position]);
            let hex = &rest[position + 2..position + 6];
            let code_point = u32::from_str_radix(hex, 16).expect("vier Hexziffern");
            output.push(char::from_u32(code_point)?);
            rest = &rest[position + 6..];
        }
        output.push_str(rest);
        Some(output)
    }

    /// Eine Statusspalte wie `[B5, B6]`, `None`, wenn sie leer ist.
    fn codes(text: &str) -> Option<Vec<String>> {
        if text.is_empty() {
            return None;
        }
        let inner = text
            .strip_prefix('[')
            .and_then(|text| text.strip_suffix(']'))
            .expect("Status in eckigen Klammern");
        Some(
            inner
                .split(',')
                .map(str::trim)
                .filter(|code| !code.is_empty())
                .map(str::to_owned)
                .collect(),
        )
    }

    #[test]
    fn idna_test_suite() {
        let text = testdata::read(
            "unicodetools",
            "unicodetools/data/idna/18.0.0/IdnaTestV2.txt",
        );
        let mut cases = 0;
        let mut skipped = 0;
        for line in text.lines() {
            let data = line.split('#').next().unwrap_or_default();
            if data.trim().is_empty() {
                continue;
            }
            let columns: Vec<&str> = data.split(';').map(str::trim).collect();
            let &[
                source,
                unicode,
                unicode_status,
                ascii_n,
                ascii_n_status,
                ascii_t,
                ascii_t_status,
            ] = columns.as_slice()
            else {
                panic!("Zeile ohne sieben Spalten: {line}");
            };
            // Einzelne Surrogate lassen sich in einem &str nicht darstellen.
            let Some(source) = unescape(source) else {
                skipped += 1;
                continue;
            };
            // Leere Spalten übernehmen den Wert einer früheren (Kopf der Datei).
            let or = |text: &str, fallback: &String| {
                if text.is_empty() {
                    fallback.clone()
                } else {
                    unescape(text).expect("Ergebnis ohne Surrogat")
                }
            };
            let unicode = or(unicode, &source);
            let ascii_n = or(ascii_n, &unicode);
            let ascii_t = or(ascii_t, &ascii_n);
            let unicode_status = codes(unicode_status).unwrap_or_default();
            let ascii_n_status = codes(ascii_n_status).unwrap_or_else(|| unicode_status.clone());
            let ascii_t_status = codes(ascii_t_status).unwrap_or_else(|| ascii_n_status.clone());
            for options in [STRICT, URL, LENIENT] {
                let ignored = ignored(options);
                let fails =
                    |codes: &[String]| codes.iter().any(|code| !ignored.contains(&code.as_str()));
                let (output, errors) = to_unicode(&source, options);
                assert_eq!(output, unicode, "ToUnicode {options:?}: {line}");
                assert_eq!(
                    !errors.is_empty(),
                    fails(&unicode_status),
                    "ToUnicode {options:?} {errors}: {line}"
                );
                for (transitional, expected, status) in [
                    (false, &ascii_n, &ascii_n_status),
                    (true, &ascii_t, &ascii_t_status),
                ] {
                    let options = Options {
                        transitional_processing: transitional,
                        ..options
                    };
                    let result = to_ascii(&source, options);
                    if fails(status) {
                        assert!(result.is_err(), "ToASCII {options:?} {result:?}: {line}");
                    } else {
                        assert_eq!(result.as_ref(), Ok(expected), "ToASCII {options:?}: {line}");
                    }
                }
            }
            cases += 1;
        }
        assert!(cases > 6000, "nur {cases} Fälle gelesen");
        assert!(skipped < 10, "{skipped} Fälle übersprungen");
    }
}
